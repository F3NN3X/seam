//! Window control: focus, move or swap the focused window in a direction on
//! the desk (D105, D107). The main thread only queues a command. A worker
//! finds the windows and works out where they go in desk millimetres, takes
//! focus itself, and hands moves back to the zone snapping's placement steps.

use crate::hook::SEAM_INPUT_TAG;
use crate::log;
use crate::snap::{self, Placement};
use seam::api::{self, Reply, RpcError, WindowVerb};
use seam::desk::Mon;
use seam::direction::{self, Dir, Found, Screen};
use seam::layout::Link;
use seam::winutil;
use seam::zones::{self, Rect, ZoneLayout};
use serde_json::json;
use std::sync::mpsc::Sender;
use std::sync::{Condvar, Mutex, OnceLock};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The worker has moves for the main thread to start.
pub const WM_APP_WINDOW: u32 = WM_APP + 18;
/// The gap slider's end, as in the snapping.
const MAX_GAP: u32 = 32;
/// A window this close to its zones is still in them.
const CLOSE_ENOUGH: i32 = 4;

/// One window command, with what the worker needs to know about the desk.
pub struct Job {
    pub owner: isize,
    pub verb: WindowVerb,
    pub dir: Dir,
    /// The focused window when the command came.
    pub hwnd: isize,
    /// Where the snapping last put that window, if it's still there.
    pub snapped: Option<(String, usize, usize)>,
    pub mons: Vec<Mon>,
    pub links: Vec<Link>,
    /// Each screen's zones, or None for none or with zones off.
    pub layouts: Vec<Option<ZoneLayout>>,
    pub gap: u32,
    /// Lower-case exe names the zones leave alone.
    pub exclude: Vec<String>,
    /// From the API, which has no right to take focus until it sends input.
    pub from_api: bool,
    pub reply: Option<Sender<Reply>>,
}

/// Moves worked out, waiting for the main thread.
struct Planned {
    list: Vec<Placement>,
    reply: Option<Sender<Reply>>,
}

static SLOT: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static PLANNED: Mutex<Option<Planned>> = Mutex::new(None);

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

/// The worker's moves are ready: start them on the snapping's steps.
pub fn planned() {
    let Some(plan) = PLANNED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    snap::place_all(plan.list);
    answer(plan.reply, Ok(json!({ "ok": true })));
}

/// What a command came to.
enum Outcome {
    Done,
    Place(Vec<Placement>),
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
        match run(&job) {
            Ok(Outcome::Done) => answer(reply, Ok(json!({ "ok": true }))),
            Ok(Outcome::Place(list)) => {
                let earlier = PLANNED.lock().unwrap_or_else(|e| e.into_inner()).replace(Planned { list, reply });
                if let Some(earlier) = earlier {
                    replaced(earlier.reply);
                }
                unsafe {
                    let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_WINDOW, WPARAM(0), LPARAM(0));
                }
            }
            Err(why) => {
                log::info!("window {:?} {}: {why}", job.verb, job.dir.name());
                answer(reply, Err(RpcError::new(api::REFUSED, why)));
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
    bounds: Rect,
    work: Rect,
    dpi: u32,
    zones: Vec<Rect>,
}

fn geometry(job: &Job, screens: &[Screen]) -> Vec<Geo> {
    let geo = |s: &Screen| {
        let (work, dpi) = winutil::work_area(s.px).unwrap_or((s.px, 96));
        let gap = (job.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        let layout = job.layouts.get(s.mm.index).and_then(Option::as_ref);
        let zones = layout.map(|l| zones::rects(&l.tree, work, gap)).unwrap_or_default();
        Geo { id: job.mons[s.mm.index].id.clone(), bounds: s.px, work, dpi, zones }
    };
    screens.iter().map(geo).collect()
}

fn is_seam(h: HWND) -> bool {
    winutil::exe_name(h).is_some_and(|name| name == "seam.exe" || name == "seam-settings.exe")
}

/// A window Seam may move, or why not.
fn movable(h: HWND, exclude: &[String]) -> Result<Rect, &'static str> {
    if h.is_invalid() || !unsafe { IsWindow(Some(h)) }.as_bool() {
        return Err("No window has focus");
    }
    if !winutil::is_snappable(h, std::process::id(), exclude) || is_seam(h) {
        return Err("That window can't be moved");
    }
    if winutil::is_hung(h) {
        return Err("That window isn't responding");
    }
    // Windows keeps a lower process from moving this one (D12).
    if winutil::is_elevated_over_us(h) {
        return Err("That window runs as administrator");
    }
    winutil::visible_rect(h).ok_or("That window can't be moved")
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

fn close(a: Rect, b: Rect, sized: bool) -> bool {
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
    let set = winutil::zone_stamp(h);
    let (x, y) = centre(rect);
    let s = geo.iter().position(|g| g.bounds.contains(x, y))?;
    let g = &geo[s];
    if set.iter().any(|i| *i >= g.zones.len()) {
        return None;
    }
    let span = set.iter().map(|i| g.zones[*i]).reduce(zones::span)?;
    let (from, to) = (*set.iter().min()?, *set.iter().max()?);
    close(rect, span, winutil::is_resizable(h)).then_some((s, from, to, span))
}

/// Where along the free room a window starts, from 0 to 1.
fn fraction(at: i32, room: i32) -> f64 {
    if room <= 0 {
        0.5
    } else {
        (at as f64 / room as f64).clamp(0.0, 1.0)
    }
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

fn into_zone(h: HWND, g: &Geo, from: usize, to: usize, target: Rect) -> Placement {
    Placement { hwnd: h.0 as isize, target, zone: Some((g.id.clone(), from, to)), dpi: g.dpi }
}

fn free(h: HWND, g: &Geo, target: Rect) -> Placement {
    Placement { hwnd: h.0 as isize, target, zone: None, dpi: g.dpi }
}

/// Give a window focus. A caller with no input of its own first sends an
/// empty mouse move, which the hook lets pass untouched (§3.3).
fn focus(h: HWND, nudge: bool) {
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

fn run(job: &Job) -> Result<Outcome, &'static str> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let source = hwnd_of(job.hwnd);
    match job.verb {
        WindowVerb::Focus => {
            let from = winutil::visible_rect(source).filter(|_| !source.is_invalid()).ok_or("No window has focus")?;
            let list = others(source, &screens);
            let rects: Vec<Rect> = list.iter().map(|(_, r)| *r).collect();
            match direction::find(&screens, &rects, from, job.dir) {
                Found::Window(i) => {
                    focus(list[i].0, job.from_api);
                    Ok(Outcome::Done)
                }
                Found::Screen(_) => Err("No window on the screen in that direction"),
                Found::Nothing => Err("Nothing in that direction"),
            }
        }
        WindowVerb::Move => {
            let rect = movable(source, &job.exclude)?;
            let geo = geometry(job, &screens);
            move_one(job, &screens, &geo, source, rect).map(|p| Outcome::Place(vec![p]))
        }
        WindowVerb::Swap => {
            let rect = movable(source, &job.exclude)?;
            let list = others(source, &screens);
            let rects: Vec<Rect> = list.iter().map(|(_, r)| *r).collect();
            let Found::Window(i) = direction::find(&screens, &rects, rect, job.dir) else {
                return Err("No window in that direction");
            };
            let (other, other_rect) = list[i];
            movable(other, &job.exclude)?;
            let geo = geometry(job, &screens);
            // Each takes the other's zones, or failing that its rect.
            let place = |h: HWND, there: HWND, there_rect: Rect, snapped: Option<&(String, usize, usize)>| match zone_of(
                &geo, there, there_rect, snapped,
            ) {
                Some((s, from, to, span)) => Some(into_zone(h, &geo[s], from, to, span)),
                None => Some(free(h, &geo[direction::screen_at(&screens, there_rect)?], there_rect)),
            };
            let first = place(source, other, other_rect, None);
            let second = place(other, source, rect, job.snapped.as_ref());
            match (first, second) {
                (Some(a), Some(b)) => Ok(Outcome::Place(vec![a, b])),
                _ => Err("No window in that direction"),
            }
        }
    }
}

/// The next zone that way, crossing onto the physically next screen at the
/// edge. A window in no zone goes to the next screen, in the same place there.
fn move_one(job: &Job, screens: &[Screen], geo: &[Geo], h: HWND, rect: Rect) -> Result<Placement, &'static str> {
    let zoned = zone_of(geo, h, rect, job.snapped.as_ref());
    let from = zoned.map_or(rect, |(_, _, _, span)| span);
    if let Some((s, _, _, span)) = zoned {
        let g = &geo[s];
        // Zones the window already covers aren't a step.
        let inside =
            |z: &Rect| z.x >= span.x && z.y >= span.y && z.x + z.w <= span.x + span.w && z.y + z.h <= span.y + span.h;
        let steps: Vec<usize> = (0..g.zones.len()).filter(|i| !inside(&g.zones[*i])).collect();
        let rects: Vec<Rect> = steps.iter().map(|i| g.zones[*i]).collect();
        if let Some(k) = direction::pick(screens, &rects, span, job.dir) {
            let zone = steps[k];
            return Ok(into_zone(h, g, zone, zone, g.zones[zone]));
        }
    }
    let next = direction::next_screen(screens, from, job.dir).ok_or("Nothing in that direction")?;
    let to = &geo[next];
    if zoned.is_some() && !to.zones.is_empty() {
        // The zone of the next screen nearest to where the window was.
        let zone = direction::pick(screens, &to.zones, from, job.dir).unwrap_or(0);
        return Ok(into_zone(h, to, zone, zone, to.zones[zone]));
    }
    let here = direction::screen_at(screens, from).ok_or("Nothing in that direction")?;
    Ok(free(h, to, relative(from, &geo[here], to)))
}
