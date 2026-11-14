//! Window control: focus, move or swap the focused window in a direction on
//! the desk (D105, D107). The main thread only queues a command. A worker
//! finds the windows and works out where they go in desk millimetres, takes
//! focus itself, and hands moves back to the zone snapping's placement steps.
//! Grow, shrink and Priority resize a window's group of zones on the same worker,
//! and so do tiling a screen's windows and saving where they are as a layout.

use crate::hook::SEAM_INPUT_TAG;
use crate::log;
use crate::snap::{self, Placement};
use seam::api::{self, GroupVerb, Reply, RpcError, WindowVerb};
use seam::config::WindowSize;
use seam::desk::{self, Mon};
use seam::direction::{self, Dir, Found, Screen};
use seam::layout::Link;
use seam::winutil;
use seam::zones::{self, Rect, ZoneLayout};
use serde_json::json;
use std::sync::mpsc::Sender;
use std::sync::{Condvar, Mutex, OnceLock};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::*;

#[path = "fromwindows.rs"]
mod fromwindows;
#[path = "regroup.rs"]
mod regroup;
#[path = "tile.rs"]
pub mod tile;

pub use regroup::{group_members, group_mins, screen_members};

/// The worker has moves for the main thread to start.
pub const WM_APP_WINDOW: u32 = WM_APP + 18;
/// The gap slider's end, as in the snapping.
const MAX_GAP: u32 = 32;
/// A window this close to its zones is still in them.
const CLOSE_ENOUGH: i32 = 4;

/// What a window command does.
#[derive(Clone, Debug)]
pub enum Task {
    Window(WindowVerb, Dir),
    Group(GroupVerb),
    Tile(tile::At),
    /// Save from windows.
    FromWindows(tile::At),
    /// A layout picked on the Zones page, which the screen's snapped windows follow.
    Pick(Box<tile::Pick>),
}

/// One window command, with what the worker needs to know about the desk.
pub struct Job {
    pub owner: isize,
    pub task: Task,
    /// The focused window when the command came.
    pub hwnd: isize,
    /// Where the snapping last put that window, if it's still there.
    pub snapped: Option<(String, usize, usize)>,
    /// Every snapped window still in its zones, for a group command. Empty otherwise.
    pub members: Vec<snap::Zoned>,
    pub mons: Vec<Mon>,
    pub links: Vec<Link>,
    /// Each screen's zones, or None for none or with zones off.
    pub layouts: Vec<Option<ZoneLayout>>,
    pub gap: u32,
    /// Lower-case exe names the zones leave alone.
    pub exclude: Vec<String>,
    /// "When a window moves to another screen".
    pub size_mode: WindowSize,
    /// From the API, which has no right to take focus until it sends input.
    pub from_api: bool,
    pub reply: Option<Sender<Reply>>,
}

/// Moves worked out, waiting for the main thread.
struct Planned {
    list: Vec<Placement>,
    /// A screen's layout a group command changed, saved before anything moves.
    layout: Option<(String, ZoneLayout, Option<regroup::Given>)>,
    /// What a tile did.
    tile: Option<tile::Done>,
    /// The id of the library layout Save from windows made, which the layout brings into the library.
    fresh: Option<String>,
    reply: Option<Sender<Reply>>,
}

static SLOT: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static PLANNED: Mutex<Option<Planned>> = Mutex::new(None);
/// Layouts edge and divider drags changed, waiting to be saved in order, with the
/// windows a divider drag moved, which stay snapped at their new places.
static DRAGGED: Mutex<Vec<(String, ZoneLayout, Vec<snap::Zoned>)>> = Mutex::new(Vec::new());
/// What the last split by a drag's keys did, for its notice and Undo once its layout is saved.
static SPLIT: Mutex<Option<tile::Done>> = Mutex::new(None);
/// Why a tile with no API client waiting did nothing. It has its own place, so it can't cancel a move waiting in PLANNED.
static NOTE: Mutex<Option<String>> = Mutex::new(None);

fn answer(reply: Option<Sender<Reply>>, result: Reply) {
    if let Some(tx) = reply {
        let _ = tx.send(result);
    }
}

fn replaced(reply: Option<Sender<Reply>>) {
    answer(reply, Err(RpcError::new(api::BUSY, "A newer window command replaced this one")));
}

/// Hand a command to the worker. A command still waiting there is replaced.
pub fn start(job: Job) {
    static RUNNING: OnceLock<bool> = OnceLock::new();
    let running = *RUNNING.get_or_init(|| std::thread::Builder::new().name("seam-window".into()).spawn(work).is_ok());
    if !running {
        answer(job.reply, Err(RpcError::new(api::BUSY, "Couldn't start the window command")));
        return;
    }
    let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(earlier) = slot.replace(job) {
        replaced(earlier.reply);
    }
    WAKE.notify_one();
}

/// The worker's moves are ready: start them on the snapping's steps. A changed
/// layout goes to `save` first, so the windows' stamps follow the new zones. When
/// it can't be saved nothing moves. Layouts from edge drags are saved before it, so
/// a group command's layout, which is what its windows move to, is the one that stays.
/// A tile answers with what it did, or with why it did nothing when no API client is
/// waiting to be told. `save`'s flag says the layout is new to the library, from Save from windows.
pub fn planned(mut save: impl FnMut(&str, ZoneLayout, bool) -> bool) -> Option<Result<tile::Done, String>> {
    let dragged = std::mem::take(&mut *DRAGGED.lock().unwrap_or_else(|e| e.into_inner()));
    for (screen, layout, moved) in dragged {
        // A divider of an ad-hoc group: the group keeps its tree in memory, and nothing is saved.
        if snap::adhoc_dragged(&screen, &layout.tree) {
            continue;
        }
        // They are where the drag left them either way, as with an edge drag.
        snap::keep_all(moved);
        if save(&screen, layout, false) {
            regroup::saved(&screen, None);
        } else {
            log::info!("zone edges: Seam was busy, so the new layout wasn't saved");
        }
    }
    let note = NOTE.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(plan) = PLANNED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return note.map(Err) };
    if let Some((screen, layout, keep)) = plan.layout {
        if !save(&screen, layout, plan.fresh.is_some()) {
            answer(plan.reply, Err(RpcError::new(api::BUSY, "Seam is busy, try again")));
            return None;
        }
        regroup::saved(&screen, keep);
    }
    // An empty list would clear window control's moves still waiting.
    if !plan.list.is_empty() {
        snap::place_all(plan.list);
    }
    let done = plan.tile;
    let counts = done.as_ref().filter(|d| !d.pick).map(|d| json!({ "ok": true, "tiled": d.tiled, "left": d.left }));
    let made = plan.fresh.map(|id| json!({ "ok": true, "id": id }));
    answer(plan.reply, Ok(counts.or(made).unwrap_or_else(|| json!({ "ok": true }))));
    done.map(Ok)
}

/// What the last split by a drag's keys did, once `planned` has saved its layout.
pub fn take_split() -> Option<tile::Done> {
    SPLIT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// A drag of a window's edge changed a screen's layout: save it the way a group
/// command's is, which also drops what Priority kept for the screen. It waits
/// behind a group command's plan rather than replacing it. Nothing moves here, as
/// the drag has put the windows already.
pub fn save_layout(owner: isize, screen: String, layout: ZoneLayout) {
    save_moved(owner, screen, layout, Vec::new());
}

/// The same for a drag of a divider's handle, from its own thread: `moved` are the
/// windows it put in new places, which the snapping keeps as snapped there.
pub fn save_moved(owner: isize, screen: String, layout: ZoneLayout, moved: Vec<snap::Zoned>) {
    DRAGGED.lock().unwrap_or_else(|e| e.into_inner()).push((screen, layout, moved));
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(owner)), WM_APP_WINDOW, WPARAM(0), LPARAM(0));
    }
}

/// A drop split a zone with the drag's keys: save the layout as an edge drag's is,
/// then offer Undo with what `done` says.
pub fn save_split(owner: isize, screen: String, layout: ZoneLayout, done: tile::Done) {
    if SPLIT.lock().unwrap_or_else(|e| e.into_inner()).replace(done).is_some() {
        log::info!("zone split: a newer split replaced the earlier one's Undo before its notice");
    }
    save_layout(owner, screen, layout);
}

/// What a command came to.
enum Outcome {
    Done,
    Place(Vec<Placement>),
    /// The moves of a group command, the screen's new layout, and what Priority keeps for that screen once the layout is saved, if anything.
    Regroup(Vec<Placement>, String, ZoneLayout, Option<regroup::Given>),
    /// A tile: the moves, the layout it gave the screen, and what it did.
    Tile(Box<tile::Plan>),
    /// Save from windows: the moves, the screen, and the new library layout it is given.
    Saved(Vec<Placement>, String, ZoneLayout),
}

fn work() {
    loop {
        let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
        while slot.is_none() {
            slot = WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
        }
        let Some(mut job) = slot.take() else { continue };
        drop(slot);
        let reply = job.reply.take();
        let outcome = match &job.task {
            Task::Window(verb, dir) => run(&job, *verb, *dir),
            Task::Group(verb) => regroup::run(&job, *verb),
            Task::Tile(at) => tile::run(&job, *at),
            Task::FromWindows(at) => fromwindows::run(&job, *at),
            Task::Pick(pick) => tile::pick(&job, pick),
        };
        let planned = match outcome {
            Ok(Outcome::Place(list)) => Ok(Some((list, None, None, None))),
            Ok(Outcome::Regroup(list, screen, layout, keep)) => {
                Ok(Some((list, Some((screen, layout, keep)), None, None)))
            }
            Ok(Outcome::Tile(plan)) => {
                let plan = *plan;
                let layout = plan.layout.map(|(screen, layout)| (screen, layout, None));
                Ok(Some((plan.list, layout, Some(plan.done), None)))
            }
            Ok(Outcome::Saved(list, screen, layout)) => {
                let fresh = layout.custom.clone();
                Ok(Some((list, Some((screen, layout, None)), None, fresh)))
            }
            Ok(Outcome::Done) => Ok(None),
            Err(why) => Err(why),
        };
        match planned {
            Ok(None) => answer(reply, Ok(json!({ "ok": true }))),
            Ok(Some((list, layout, tile, fresh))) => {
                let plan = Planned { list, layout, tile, fresh, reply };
                let earlier = PLANNED.lock().unwrap_or_else(|e| e.into_inner()).replace(plan);
                if let Some(earlier) = earlier {
                    replaced(earlier.reply);
                }
                unsafe {
                    let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_WINDOW, WPARAM(0), LPARAM(0));
                }
            }
            Err(why) => {
                match &job.task {
                    Task::Window(verb, dir) => log::info!("window {verb:?} {}: {why}", dir.name()),
                    Task::Group(verb) => log::info!("window {verb:?}: {why}"),
                    Task::Tile(_) => log::info!("tile screen: {why}"),
                    Task::FromWindows(_) => log::info!("save from windows: {why}"),
                    Task::Pick(_) => log::info!("zones pick: {why}"),
                }
                // A tile from a key or the tray has nobody to answer, so the screen is told.
                if matches!(job.task, Task::Tile(_)) && reply.is_none() {
                    *NOTE.lock().unwrap_or_else(|e| e.into_inner()) = Some(why);
                    unsafe {
                        let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_WINDOW, WPARAM(0), LPARAM(0));
                    }
                } else {
                    answer(reply, Err(RpcError::new(api::REFUSED, why)));
                }
            }
        }
    }
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// A screen's work area, scale and zones in pixels, in the order of the desk's screens.
struct Geo {
    id: String,
    /// Line Up number.
    number: usize,
    bounds: Rect,
    work: Rect,
    dpi: u32,
    /// Physical pixels per inch from EDID, 0 when the screen doesn't report its size.
    raw_dpi: f64,
    zones: Vec<Rect>,
}

fn geometry(job: &Job, screens: &[Screen]) -> Vec<Geo> {
    let geo = |s: &Screen| {
        let (work, dpi) = winutil::work_area(s.px).unwrap_or((s.px, 96));
        let gap = (job.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        let layout = job.layouts.get(s.mm.index).and_then(Option::as_ref);
        let zones = layout.map(|l| zones::rects(&l.tree, work, gap)).unwrap_or_default();
        let mon = &job.mons[s.mm.index];
        Geo { id: mon.id.clone(), number: s.mm.index + 1, bounds: s.px, work, dpi, raw_dpi: mon.raw_dpi, zones }
    };
    screens.iter().map(geo).collect()
}

pub fn is_seam(h: HWND) -> bool {
    winutil::exe_name(h).is_some_and(|name| name == "seam.exe" || name == "seam-settings.exe")
}

/// A window Seam may move, or why not.
pub fn movable(h: HWND, exclude: &[String]) -> Result<Rect, String> {
    if h.is_invalid() || !unsafe { IsWindow(Some(h)) }.as_bool() {
        return Err("No window has focus".into());
    }
    if !winutil::is_snappable(h, std::process::id(), exclude) || is_seam(h) {
        return Err("That window can't be moved".into());
    }
    if winutil::is_hung(h) {
        return Err("That window isn't responding".into());
    }
    // Windows keeps a lower process from moving this one (D12).
    if winutil::is_elevated_over_us(h) {
        return Err(match winutil::app_name(h) {
            Some(name) => format!("{name} runs as administrator"),
            None => "That window runs as administrator".into(),
        });
    }
    winutil::visible_rect(h).ok_or_else(|| "That window can't be moved".into())
}

/// The windows focus and swap can reach, topmost first. Seam's own, hung and
/// minimized windows are never among them.
fn others(source: HWND, screens: &[Screen]) -> Vec<(HWND, Rect)> {
    let own = std::process::id();
    let on_desk = |r: &Rect| {
        let (x, y) = centre(*r);
        r.w > 0 && r.h > 0 && screens.iter().any(|s| s.px.contains(x, y))
    };
    let reachable = winutil::top_level_windows().into_iter().filter(|h| {
        *h != source && winutil::is_capturable(*h, own) && !winutil::is_minimized(*h) && !winutil::is_hung(*h)
    });
    let rects = reachable.filter_map(|h| winutil::visible_rect(h).filter(on_desk).map(|r| (h, r)));
    rects.filter(|(h, _)| !is_seam(*h)).collect()
}

pub fn close(a: Rect, b: Rect, sized: bool) -> bool {
    let size = [(a.w, b.w), (a.h, b.h)];
    let sides = [(a.x, b.x), (a.y, b.y)].into_iter().chain(size.into_iter().filter(|_| sized));
    sides.into_iter().all(|(p, q)| (p - q).abs() <= CLOSE_ENOUGH)
}

/// The zones a window sits in: (screen, first, last, their rect). The snapping's
/// own record counts first, then the stamp on the window if it still fits.
fn zone_of(
    geo: &[Geo],
    h: HWND,
    rect: Rect,
    snapped: Option<&(String, usize, usize)>,
) -> Option<(usize, usize, usize, Rect)> {
    let known = snapped.and_then(|(id, from, to)| {
        let s = geo.iter().position(|g| g.id == *id && *from <= *to && *to < g.zones.len())?;
        Some((s, *from, *to, zones::span(geo[s].zones[*from], geo[s].zones[*to])))
    });
    if known.is_some() {
        return known;
    }
    let (x, y) = centre(rect);
    let s = geo.iter().position(|g| g.bounds.contains(x, y))?;
    let (from, to, span) = stamped_in(h, rect, &geo[s].zones)?;
    Some((s, from, to, span))
}

/// The first and last of `placed`, a screen's zones, that the stamp on a window
/// names, and their rect, while its visible frame `rect` is still on them.
pub fn stamped_in(h: HWND, rect: Rect, placed: &[Rect]) -> Option<(usize, usize, Rect)> {
    let set = winutil::zone_stamp(h);
    if set.iter().any(|i| *i >= placed.len()) {
        return None;
    }
    let span = set.iter().map(|i| placed[*i]).reduce(zones::span)?;
    let (from, to) = (*set.iter().min()?, *set.iter().max()?);
    close(rect, span, winutil::is_resizable(h)).then_some((from, to, span))
}

/// Where along the free room a window starts, from 0 to 1.
fn fraction(at: i32, room: i32) -> f64 {
    if room <= 0 { 0.5 } else { (at as f64 / room as f64).clamp(0.0, 1.0) }
}

/// The same place on another screen: the size follows the scale, and the
/// position keeps its share of the room left in the work area.
fn relative(rect: Rect, from: &Geo, to: &Geo) -> Rect {
    let k = to.dpi as f64 / from.dpi.max(1) as f64;
    let w = ((rect.w as f64 * k).round() as i32).clamp(1, to.work.w.max(1));
    let h = ((rect.h as f64 * k).round() as i32).clamp(1, to.work.h.max(1));
    let fx = fraction(rect.x - from.work.x, from.work.w - rect.w);
    let fy = fraction(rect.y - from.work.y, from.work.h - rect.h);
    let x = to.work.x + (fx * (to.work.w - w) as f64).round() as i32;
    let y = to.work.y + (fy * (to.work.h - h) as f64).round() as i32;
    Rect { x, y, w, h }
}

/// "When a window moves to another screen": a window whose frame was `size` on
/// `from` keeps its pixel size or its real size on `to`, centred where the move put
/// it (§11.4). With Windows' scaling, or a real size where a screen doesn't report
/// its size, it keeps the place the move worked out.
fn resized(job: &Job, h: HWND, size: Rect, from: &Geo, to: &Geo, placed: Rect) -> Rect {
    if job.size_mode == WindowSize::Windows || from.id == to.id {
        return placed;
    }
    let app = winutil::exe_name(h).unwrap_or_default();
    let px = (size.w as f64, size.h as f64);
    let Some(r) = desk::moved_rect(job.size_mode, px, (from.raw_dpi, to.raw_dpi), placed, to.work) else {
        log::info!(
            "window size: {app} scaled by Windows, screen {} or {} doesn't report its size",
            from.number,
            to.number
        );
        return placed;
    };
    let (a, b, mode) = (from.number, to.number, job.size_mode);
    log::info!("window size ({mode:?}): {app} screen {a} → {b}, {}×{} px → {}×{} px", size.w, size.h, r.w, r.h);
    r
}

fn into_zone(h: HWND, g: &Geo, from: usize, to: usize, target: Rect) -> Placement {
    Placement { hwnd: h.0 as isize, target, zone: Some((g.id.clone(), from, to)), dpi: g.dpi, maximize: false }
}

fn free(h: HWND, g: &Geo, target: Rect) -> Placement {
    Placement { hwnd: h.0 as isize, target, zone: None, dpi: g.dpi, maximize: false }
}

/// Give a window focus. A caller with no input of its own first sends an
/// empty mouse move, which the hook lets pass untouched (§3.3). A workspace
/// switch focuses through here too.
pub fn focus(h: HWND, nudge: bool) {
    unsafe {
        if nudge {
            let mi = MOUSEINPUT { dwFlags: MOUSEEVENTF_MOVE, dwExtraInfo: SEAM_INPUT_TAG, ..Default::default() };
            let input = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi } };
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
        if !SetForegroundWindow(h).as_bool() {
            log::info!("window focus: Windows kept the focus where it was");
        }
    }
}

fn run(job: &Job, verb: WindowVerb, dir: Dir) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let source = hwnd_of(job.hwnd);
    match verb {
        WindowVerb::Focus => {
            let from = winutil::visible_rect(source).filter(|_| !source.is_invalid()).ok_or("No window has focus")?;
            let list = others(source, &screens);
            let rects: Vec<Rect> = list.iter().map(|(_, r)| *r).collect();
            match direction::find(&screens, &rects, from, dir) {
                Found::Window(i) => {
                    focus(list[i].0, job.from_api);
                    Ok(Outcome::Done)
                }
                Found::Screen(_) => Err("No window on the screen in that direction".into()),
                Found::Nothing => Err("Nothing in that direction".into()),
            }
        }
        WindowVerb::Move => {
            let rect = movable(source, &job.exclude)?;
            let geo = geometry(job, &screens);
            move_one(job, dir, &screens, &geo, source, rect).map(|p| Outcome::Place(vec![p])).map_err(String::from)
        }
        WindowVerb::Swap => {
            let rect = movable(source, &job.exclude)?;
            let list = others(source, &screens);
            let rects: Vec<Rect> = list.iter().map(|(_, r)| *r).collect();
            let Found::Window(i) = direction::find(&screens, &rects, rect, dir) else {
                return Err("No window in that direction".into());
            };
            let (other, other_rect) = list[i];
            movable(other, &job.exclude)?;
            let geo = geometry(job, &screens);
            // Each takes the other's zones, or failing that its rect, at its own size in
            // millimetres when that's on another screen.
            let place =
                |h: HWND, own: Rect, there: HWND, there_rect: Rect, snapped: Option<&(String, usize, usize)>| {
                    match zone_of(&geo, there, there_rect, snapped) {
                        Some((s, from, to, span)) => Some(into_zone(h, &geo[s], from, to, span)),
                        None => {
                            let to = &geo[direction::screen_at(&screens, there_rect)?];
                            let target = match direction::screen_at(&screens, own) {
                                Some(here) => resized(job, h, own, &geo[here], to, there_rect),
                                None => there_rect,
                            };
                            Some(free(h, to, target))
                        }
                    }
                };
            let first = place(source, rect, other, other_rect, None);
            let second = place(other, other_rect, source, rect, job.snapped.as_ref());
            match (first, second) {
                (Some(a), Some(b)) => Ok(Outcome::Place(vec![a, b])),
                _ => Err("No window in that direction".into()),
            }
        }
    }
}

/// The next zone that way, crossing onto the physically next screen at the
/// edge. A window in no zone goes to the next screen, in the same place there.
fn move_one(
    job: &Job,
    dir: Dir,
    screens: &[Screen],
    geo: &[Geo],
    h: HWND,
    rect: Rect,
) -> Result<Placement, &'static str> {
    let zoned = zone_of(geo, h, rect, job.snapped.as_ref());
    let from = zoned.map_or(rect, |(_, _, _, span)| span);
    if let Some((s, _, _, span)) = zoned {
        let g = &geo[s];
        // Zones the window already covers aren't a step.
        let inside =
            |z: &Rect| z.x >= span.x && z.y >= span.y && z.x + z.w <= span.x + span.w && z.y + z.h <= span.y + span.h;
        let steps: Vec<usize> = (0..g.zones.len()).filter(|i| !inside(&g.zones[*i])).collect();
        let rects: Vec<Rect> = steps.iter().map(|i| g.zones[*i]).collect();
        if let Some(k) = direction::pick(screens, &rects, span, dir) {
            let zone = steps[k];
            return Ok(into_zone(h, g, zone, zone, g.zones[zone]));
        }
    }
    let next = direction::next_screen(screens, from, dir).ok_or("Nothing in that direction")?;
    let to = &geo[next];
    if zoned.is_some() && !to.zones.is_empty() {
        // The zone of the next screen nearest to where the window was.
        let zone = direction::pick(screens, &to.zones, from, dir).unwrap_or(0);
        return Ok(into_zone(h, to, zone, zone, to.zones[zone]));
    }
    let here = direction::screen_at(screens, from).ok_or("Nothing in that direction")?;
    let placed = relative(from, &geo[here], to);
    Ok(free(h, to, resized(job, h, from, &geo[here], to, placed)))
}
