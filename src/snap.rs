//! Zone snapping: drag a window with the show key held and it lands in a zone.
//! It runs on the tray's main thread from a WinEvent hook and a 16 ms timer
//! that lives only while a window is being dragged. Nothing here touches the
//! mouse hook (D7).

use crate::tray::WM_APP_WINDOWS_CHANGED;
use crate::{dividers, dragkeys, log, minsize, overlay, wincontrol, zone_paint};
use seam::config::{Config, WindowSize, ZoneSettings};
use seam::desk::{self, Mon};
use seam::geom;
use seam::groups::Side;
use seam::winutil::{self, work_area};
use seam::zones::{self, Rect, ZoneLayout};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[path = "refit.rs"]
mod refit;

#[path = "edges.rs"]
mod edges;

pub(crate) use edges::Member;
#[path = "adhoc.rs"]
mod adhoc;
mod drag;
mod placing;
#[path = "steer.rs"]
mod steer;
#[path = "swap.rs"]
mod swap;
mod sync;

pub use adhoc::{adhoc_area, adhoc_dragged, fit, is_adhoc};
pub use refit::{WM_APP_REFIT, display_changed, found as refit_found, line_up_ended};

/// A window the tray couldn't move, probably because it runs as administrator.
pub const WM_APP_SNAP_BLOCKED: u32 = WM_APP + 14;
/// The app `WM_APP_SNAP_BLOCKED` is about, for the tray to name.
static BLOCKED_APP: Mutex<Option<String>> = Mutex::new(None);

/// The app the last `WM_APP_SNAP_BLOCKED` named, once.
pub fn take_blocked_app() -> Option<String> {
    BLOCKED_APP.lock().unwrap_or_else(|e| e.into_inner()).take()
}
const TICK_MS: u32 = 16;
/// Time for an app to react to a move before the next step.
const SETTLE_MS: u32 = 120;
const MAX_SNAPPED: usize = 256;
/// A window this close to where it was put still counts as snapped.
const CLOSE_ENOUGH: i32 = 4;
/// The gap slider's end. A stored gap past it is cut back when used.
const MAX_GAP: u32 = 32;
/// Times a placement is tried again when the app hadn't finished reacting.
const MAX_RETRIES: u8 = 2;

/// Width, height and monitor DPI of a window before it was snapped.
type Old = (i32, i32, u32);

#[derive(PartialEq)]
struct Screen {
    id: String,
    bounds: Rect,
    layout: Option<ZoneLayout>,
    /// Physical pixels per inch from EDID, 0 when the screen doesn't report its size.
    raw_dpi: f64,
}

/// A screen's zones in pixels, worked out when they're needed so a moved taskbar counts.
struct Placed {
    id: String,
    bounds: Rect,
    work: Rect,
    dpi: u32,
    /// With no gap, so every point of the work area is in a zone.
    cells: Vec<Rect>,
    zones: Vec<Rect>,
}

/// What the overlay shows: a screen, the zones lit on it, and the part of them the
/// drag's keys give the window, on which side.
#[derive(Clone, Copy, PartialEq)]
struct View {
    screen: usize,
    hit: Option<(usize, usize)>,
    cut: Option<(dragkeys::Part, Side)>,
}

/// A drag is only a move once the window has moved without changing size.
/// Until then it could still be an edge resize, which zones leave alone.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Waiting,
    Moving,
    Resizing,
}

struct Drag {
    hwnd: isize,
    phase: Phase,
    /// The window's rect when the drag began.
    start: Rect,
    /// Its visible frame then, for keeping its size (§11.4).
    frame: Option<Rect>,
    was_maximized: bool,
    /// The window carried a zone stamp when the drag began.
    stamped: bool,
    /// The screen and zones it was snapped to when the drag began.
    old_zone: Option<(String, usize, usize)>,
    /// What the window's snapped entry said when the drag began, if it was snapped.
    /// Its old size is given back at the drop, or carried to the next zone.
    was_snapped: Option<Option<Old>>,
    /// The main mouse button, if it was down when the drag began. A drag started
    /// from the keyboard has none, so it can't be told it ended.
    button: Option<VIRTUAL_KEY>,
    geometry: Vec<Placed>,
    /// The zone Span started from.
    anchor: Option<(usize, usize)>,
    shown: Option<View>,
    timer: usize,
    /// The windows that follow its edges, when it's a window in a group of zones.
    edges: Option<edges::Edges>,
    /// What the keys pressed during the drag chose.
    keys: dragkeys::Steer,
    /// Pause to snap: where the pointer came to rest, where it was last tick, and since when.
    rest: Option<(POINT, POINT, Instant)>,
    /// The other windows a drop could swap with, topmost first, and where each was
    /// when the drag began: one look at the windows, for every preview.
    others: Vec<(isize, Rect)>,
    /// `others` was read, the first time zones showed.
    looked: bool,
    /// Moves posted at drag start, and where to, as they may not have landed yet.
    landed: Vec<(isize, Rect)>,
    /// The pointer rested, so zones show as if the show key were held.
    paused: bool,
    /// The window edge the pointer is on, while that is what a drop does.
    beside: Option<adhoc::Beside>,
    /// The last edge looked at, and what a drop there would show, if anything.
    tried: Option<(adhoc::Beside, Option<usize>, Option<Option<View>>)>,
    /// Windows looked at as targets, and whether Seam may move each.
    allowed: Vec<(isize, bool)>,
    /// It began in `altdrag`, from anywhere in the window.
    alt: bool,
}

struct Snapped {
    /// Size and DPI before snapping, for "restore size". None for a maximized window.
    old: Option<Old>,
    screen: String,
    from: usize,
    to: usize,
    at: Rect,
}

enum Step {
    Restore,
    Move,
    Fit,
    Verify,
}

/// A placement waiting on the app between steps.
struct Pending {
    hwnd: isize,
    target: Rect,
    before: Rect,
    steps: VecDeque<Step>,
    retries: u8,
    /// A re-fit after a display change: say nothing if the window doesn't move.
    quiet: bool,
    timer: usize,
}

#[derive(Default)]
struct Engine {
    owner: isize,
    enabled: bool,
    /// Another window manager has the windows, and why Seam steps aside (D110).
    aside: Option<String>,
    /// Seam is paused or a full-screen app is in front (D390): no drag snapping, zones or handles.
    standing_by: bool,
    settings: ZoneSettings,
    /// Lower case.
    exclude: Vec<String>,
    /// The size a free drag to another screen gives the window: Windows' own unless
    /// "Also when I drag a window to another screen" is on and nothing steps aside.
    drag_size: WindowSize,
    /// The key for dragging from anywhere in a window includes Ctrl, so Ctrl is held for the whole drag.
    alt_holds_ctrl: bool,
    screens: Vec<Screen>,
    hook: Option<HWINEVENTHOOK>,
    drag: Option<Drag>,
    snapped: HashMap<isize, Snapped>,
    pending: Option<Pending>,
    /// Exe paths already told about this run.
    warned: HashSet<String>,
    refit: refit::Refit,
    /// Window control's moves, one at a time through the same steps.
    queued: VecDeque<Placement>,
    /// Windows put side by side on screens with no zones, in memory only.
    adhoc: Vec<adhoc::Adhoc>,
    /// The last ad-hoc group's id.
    adhoc_next: u64,
    /// Hooks on closing, minimizing and moving, per thread of a group's windows.
    member_hooks: Vec<(u32, Vec<HWINEVENTHOOK>)>,
    /// Waits for a moved member to stay put.
    adhoc_timer: usize,
}

/// Where window control wants a window, worked out on its worker.
pub struct Placement {
    pub hwnd: isize,
    /// The visible frame's rect.
    pub target: Rect,
    /// The screen and zones it lands in, or None for a free place.
    pub zone: Option<(String, usize, usize)>,
    /// The effective DPI of the screen it lands on.
    pub dpi: u32,
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
}

fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINE.with(|e| e.try_borrow_mut().ok().map(|mut e| f(&mut e)))
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

fn rect_of(r: &geom::Rect) -> Rect {
    Rect { x: r.left, y: r.top, w: r.width(), h: r.height() }
}

fn key_code(key: &str) -> Option<VIRTUAL_KEY> {
    match key {
        "shift" => Some(VK_SHIFT),
        "ctrl" => Some(VK_CONTROL),
        "alt" => Some(VK_MENU),
        _ => None,
    }
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk.0)) < 0 }
}

/// Whether two rects match within a few pixels. A window that can't be resized
/// only has to start in the right place.
fn close(a: Rect, b: Rect, sized: bool) -> bool {
    let size = [(a.w, b.w), (a.h, b.h)];
    let sides = [(a.x, b.x), (a.y, b.y)].into_iter().chain(size.into_iter().filter(|_| sized));
    sides.into_iter().all(|(p, q)| (p - q).abs() <= CLOSE_ENOUGH)
}

/// The main mouse button as the keyboard-state calls see it, so a swapped pair counts.
fn main_button() -> VIRTUAL_KEY {
    if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON }
}

/// Tell the engine what the settings, this desk's layouts and the screens are
/// now, and why it steps aside if another window manager has the windows.
pub fn update(owner: HWND, cfg: &Config, sig: &str, mons: &[Mon], aside: Option<&str>) {
    let desk = cfg.desks.get(sig);
    let screens = mons.iter().map(|m| Screen {
        id: m.id.clone(),
        bounds: rect_of(&m.rect),
        layout: desk.and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of),
        raw_dpi: m.raw_dpi,
    });
    let screens: Vec<Screen> = screens.collect();
    let standing_by = crate::standby::reason().is_some();
    let on_drag = cfg.window_size_on_drag && aside.is_none() && !standing_by;
    let drag_size = if on_drag { cfg.window_size_on_move } else { WindowSize::Windows };
    let alt_holds_ctrl = cfg.windows.alt_drag.on && cfg.windows.alt_drag.key == "ctrl_alt";
    let exclude: Vec<String> = cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    with_engine(|e| {
        // A change that isn't about zones mustn't cut a drag short.
        let same = e.owner == owner.0 as isize
            && e.enabled == cfg.zones.enabled
            && e.aside.as_deref() == aside
            && e.standing_by == standing_by
            && e.settings == cfg.zones
            && e.exclude == exclude
            && e.drag_size == drag_size
            && e.alt_holds_ctrl == alt_holds_ctrl
            && e.screens == screens;
        if same {
            return;
        }
        e.cancel_drag();
        e.owner = owner.0 as isize;
        e.enabled = cfg.zones.enabled;
        e.aside = aside.map(String::from);
        e.standing_by = standing_by;
        e.settings = cfg.zones.clone();
        e.exclude = exclude;
        e.drag_size = drag_size;
        e.alt_holds_ctrl = alt_holds_ctrl;
        e.screens = screens;
        // An ad-hoc group is for a screen with no zones, as it was when it was made.
        e.adhoc_screens_changed();
        if !e.settings.refit || e.aside.is_some() || e.standing_by {
            e.refit_stop();
        }
        e.sync_hook();
        e.sync_overlay();
        e.sync_dividers();
    });
}

/// The work area changed (a moved taskbar, a scaling change), so the zones' pixels did.
pub fn work_area_changed() {
    with_engine(|e| {
        e.adhoc_screens_changed();
        e.sync_overlay();
        e.sync_dividers();
    });
}

/// The tray is quitting: free the overlay's windows and bitmaps, and the divider handles.
pub fn shutdown() {
    with_engine(Engine::end_adhoc);
    dragkeys::stop();
    zone_paint::clear();
    dividers::stop();
}

/// Windows a divider drag moved: still in the same zones, at their new places.
pub fn keep_all(list: Vec<Zoned>) {
    if list.is_empty() {
        return;
    }
    with_engine(|e| {
        for z in list {
            let (screen, from, to) = z.zone;
            e.keep_snapped(z.hwnd, &screen, (from, to), z.at);
        }
        e.sync_dividers();
    });
}

/// Take a window out of its zones: it gets its size from before it was snapped,
/// centred on the zones it was in, and the layout and the other windows stay.
pub fn take_out(hwnd: HWND) -> Result<(), String> {
    with_engine(|e| e.take_out(hwnd)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Open the size menu for a window in its zones, at its centre.
pub fn size_menu(hwnd: HWND) -> Result<(), String> {
    with_engine(|e| e.size_menu(hwnd)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

fn specs(geometry: &[Placed]) -> Vec<zone_paint::ScreenSpec> {
    let spec = |g: &Placed| zone_paint::ScreenSpec {
        id: g.id.clone(),
        work: g.work,
        scale: g.dpi as f32 / 96.0,
        zones: g.zones.clone(),
    };
    geometry.iter().map(spec).collect()
}

/// Undo of a split: give these windows their old zone numbers back, without moving them.
pub fn rezone(list: Vec<Zoned>) {
    if !list.is_empty() {
        with_engine(|e| e.rezone(list));
    }
}

/// A key the drag's keyboard hook passed on, by virtual-key code, or 0 with the
/// generation of a hook that took itself out at the cap.
pub fn drag_key(vk: u32, generation: u64) {
    with_engine(|e| e.drag_key(vk, generation));
}

/// A drag from anywhere in a window (`altdrag`) started or ended. It is a drag like
/// any other from here on. Its press never reached Windows, so only its end counts.
pub fn alt_drag(hwnd: HWND, started: bool) {
    with_engine(|e| {
        if !started {
            e.drag_end(hwnd);
            return;
        }
        e.drag_start(hwnd);
        if let Some(drag) = e.drag.as_mut() {
            drag.button = None;
            drag.alt = true;
        }
    });
}

/// Move the foreground window to the next or previous zone, across screens.
pub fn move_foreground(next: bool) -> Result<(), String> {
    with_engine(|e| e.move_foreground(next)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// The screen and zones a window is snapped to, while it's still where it was
/// put. A window still on its way counts as being where it's going.
pub fn zone_of(hwnd: HWND) -> Option<(String, usize, usize)> {
    let key = hwnd.0 as isize;
    with_engine(|e| {
        if let Some(q) = e.queued.iter().rev().find(|q| q.hwnd == key) {
            return q.zone.clone();
        }
        let moving = e.pending.as_ref().is_some_and(|p| p.hwnd == key);
        let s = if moving { e.snapped.get(&key) } else { e.current(hwnd) };
        s.map(|s| (s.screen.clone(), s.from, s.to))
    })
    .flatten()
}

/// A window the snapping put in zones, as it recorded it.
pub struct Zoned {
    pub hwnd: isize,
    /// The screen and zones.
    pub zone: (String, usize, usize),
    /// Where its visible frame was put.
    pub at: Rect,
    /// Still on its way there.
    pub moving: bool,
}

/// Every snapped window as recorded, for finding a group's members. Whether each
/// is still there is for the worker to check: that reads every window's rect.
pub fn zoned() -> Vec<Zoned> {
    with_engine(|e| e.zoned()).unwrap_or_default()
}

/// Put windows where window control worked out, one after another. A newer
/// list replaces what is still waiting.
pub fn place_all(list: Vec<Placement>) {
    with_engine(|e| {
        e.queued = list.into();
        e.window_pump();
    });
}

/// Add moves to the ones still waiting, which `place_all` would replace.
pub fn place_more(list: Vec<Placement>) {
    with_engine(|e| {
        e.queued.extend(list);
        e.window_pump();
    });
}

/// Undo of a pull-out: the window goes back in its zone with the size it had before
/// it was snapped, if it still exists, isn't minimized or maximized and is on that screen.
pub fn put_back(place: Placement, old: Option<Old>) {
    with_engine(|e| e.put_back(place, old));
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object != OBJID_WINDOW.0 {
        return;
    }
    with_engine(|e| match event {
        EVENT_SYSTEM_MOVESIZESTART => e.drag_start(hwnd),
        EVENT_SYSTEM_MOVESIZEEND => e.drag_end(hwnd),
        _ => {}
    });
}

unsafe extern "system" fn on_tick(_: HWND, _: u32, _: usize, _: u32) {
    with_engine(Engine::tick);
}

unsafe extern "system" fn on_settle(_: HWND, _: u32, id: usize, _: u32) {
    unsafe {
        with_engine(|e| {
            if e.pending.as_ref().is_some_and(|p| p.timer == id) {
                e.advance();
            } else {
                let _ = KillTimer(None, id);
            }
        });
    }
}
