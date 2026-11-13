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
#[path = "steer.rs"]
mod steer;
#[path = "swap.rs"]
mod swap;

pub use refit::{display_changed, found as refit_found, line_up_ended, WM_APP_REFIT};

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
    /// The pointer rested, so zones show as if the show key were held.
    paused: bool,
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
    settings: ZoneSettings,
    /// Lower case.
    exclude: Vec<String>,
    /// The size a free drag to another screen gives the window: Windows' own unless
    /// "Also when I drag a window to another screen" is on and nothing steps aside.
    drag_size: WindowSize,
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
    if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    }
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
    let on_drag = cfg.window_size_on_drag && aside.is_none();
    let drag_size = if on_drag { cfg.window_size_on_move } else { WindowSize::Windows };
    let exclude: Vec<String> = cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    with_engine(|e| {
        // A change that isn't about zones mustn't cut a drag short.
        let same = e.owner == owner.0 as isize
            && e.enabled == cfg.zones.enabled
            && e.aside.as_deref() == aside
            && e.settings == cfg.zones
            && e.exclude == exclude
            && e.drag_size == drag_size
            && e.screens == screens;
        if same {
            return;
        }
        e.cancel_drag();
        e.owner = owner.0 as isize;
        e.enabled = cfg.zones.enabled;
        e.aside = aside.map(String::from);
        e.settings = cfg.zones.clone();
        e.exclude = exclude;
        e.drag_size = drag_size;
        e.screens = screens;
        if !e.settings.refit || e.aside.is_some() {
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
        e.sync_overlay();
        e.sync_dividers();
    });
}

/// The tray is quitting: free the overlay's windows and bitmaps, and the divider handles.
pub fn shutdown() {
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
    with_engine(|e| {
        if e.pending.as_ref().is_some_and(|p| p.timer == id) {
            e.advance();
        } else {
            let _ = KillTimer(None, id);
        }
    });
}

impl Engine {
    /// A free drag to another screen gets its size from Seam (§11.4).
    fn drags_sized(&self) -> bool {
        self.drag_size != WindowSize::Windows
    }

    /// Zones are on, and no other window manager has the windows.
    fn active(&self) -> bool {
        self.enabled && self.aside.is_none()
    }

    /// Have the overlay thread draw ahead of time what a drag will show, or free it all.
    fn sync_overlay(&mut self) {
        let wanted = self.active() && self.screens.iter().any(|s| s.layout.is_some());
        if wanted {
            zone_paint::build(specs(&self.geometry()));
        } else {
            zone_paint::clear();
        }
    }

    fn sync_hook(&mut self) {
        let zoning = self.active() && self.screens.iter().any(|s| s.layout.is_some());
        // A free drag between screens is seen at its end, zones or not.
        let wanted = zoning || self.drags_sized();
        match (wanted, self.hook) {
            (true, None) => unsafe {
                let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                let hook = SetWinEventHook(
                    EVENT_SYSTEM_MOVESIZESTART,
                    EVENT_SYSTEM_MOVESIZEEND,
                    None,
                    Some(on_event),
                    0,
                    0,
                    flags,
                );
                if hook.0.is_null() {
                    log::info!("zone drag hook not installed");
                } else {
                    self.hook = Some(hook);
                }
            },
            (false, Some(hook)) => {
                let _ = unsafe { UnhookWinEvent(hook) };
                self.hook = None;
            }
            _ => {}
        }
    }

    fn geometry(&self) -> Vec<Placed> {
        let gap = |dpi: u32| (self.settings.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        let placed = self.screens.iter().filter_map(|s| {
            let layout = s.layout.as_ref()?;
            let (work, dpi) = work_area(s.bounds)?;
            Some(Placed {
                id: s.id.clone(),
                bounds: s.bounds,
                work,
                dpi,
                cells: zones::rects(&layout.tree, work, 0),
                zones: zones::rects(&layout.tree, work, gap(dpi)),
            })
        });
        placed.collect()
    }

    fn zoned(&self) -> Vec<Zoned> {
        let moving = |key: isize| self.pending.as_ref().is_some_and(|p| p.hwnd == key);
        let entry = |(key, s): (&isize, &Snapped)| Zoned {
            hwnd: *key,
            zone: (s.screen.clone(), s.from, s.to),
            at: s.at,
            moving: moving(*key),
        };
        self.snapped.iter().map(entry).collect()
    }

    /// The snapped entry for a window that is still where it was put.
    fn current(&self, hwnd: HWND) -> Option<&Snapped> {
        let snapped = self.snapped.get(&(hwnd.0 as isize))?;
        let here = winutil::visible_rect(hwnd)?;
        close(here, snapped.at, winutil::is_resizable(hwnd)).then_some(snapped)
    }

    /// What the divider handles and the size menu need to know about the zones now.
    fn scene(&self) -> dividers::Scene {
        let screen = |g: Placed| {
            let layout = self.screens.iter().find(|s| s.id == g.id)?.layout.clone()?;
            let gap = (self.settings.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
            Some(dividers::Screen { id: g.id, bounds: g.bounds, work: g.work, dpi: g.dpi, gap, layout })
        };
        let screens = self.geometry().into_iter().filter_map(screen).collect();
        dividers::Scene { owner: self.owner, screens, known: self.zoned(), exclude: self.exclude.clone() }
    }

    /// Hand the divider handles the zones as they are now, or none with zones off,
    /// stepping aside or the handles turned off. Their thread builds them a moment later.
    fn sync_dividers(&self) {
        // The handles stay hidden until the drag ends, which hands them the zones then.
        if self.drag.is_some() {
            return;
        }
        let on = self.active() && self.settings.divider_handles && self.screens.iter().any(|s| s.layout.is_some());
        dividers::show(on.then(|| self.scene()));
    }

    /// Why a window can't be sized in its zones, or its visible frame.
    fn sizable(&self, hwnd: HWND) -> Result<Rect, String> {
        if let Some(why) = &self.aside {
            return Err(why.clone());
        }
        if !self.enabled {
            return Err("Zones are off".into());
        }
        wincontrol::movable(hwnd, &self.exclude)
    }

    /// The zones a window is in, and its size from before it was snapped: the
    /// snapping's own record, or after a restart of the tray the stamp on the window.
    fn zone_here(&self, hwnd: HWND, frame: Rect) -> Option<(String, usize, usize, Option<Old>)> {
        if let Some(s) = self.current(hwnd) {
            return Some((s.screen.clone(), s.from, s.to, s.old));
        }
        let geometry = self.geometry();
        let g = geometry.iter().find(|g| g.bounds.contains(frame.x + frame.w / 2, frame.y + frame.h / 2))?;
        let (from, to, _) = wincontrol::stamped_in(hwnd, frame, &g.zones)?;
        Some((g.id.clone(), from, to, None))
    }

    fn take_out(&mut self, hwnd: HWND) -> Result<(), String> {
        let frame = self.sizable(hwnd)?;
        let (screen, from, to, old) = self.zone_here(hwnd, frame).ok_or("That window isn't in a zone")?;
        let geometry = self.geometry();
        let g = geometry.iter().find(|g| g.id == screen && to < g.zones.len()).ok_or("That window isn't in a zone")?;
        let zone = zones::span(g.zones[from], g.zones[to]);
        let (w, h) = match old {
            _ if !winutil::is_resizable(hwnd) => (frame.w, frame.h),
            Some((w, h, dpi)) => {
                // The old size is a window rect, so its invisible borders come off.
                let k = winutil::monitor_dpi(hwnd).max(1) as f32 / dpi.max(1) as f32;
                let window = winutil::window_rect(hwnd).unwrap_or(frame);
                (
                    (w as f32 * k).round() as i32 - (window.w - frame.w),
                    (h as f32 * k).round() as i32 - (window.h - frame.h),
                )
            }
            // Maximized before it was snapped, or snapped before the tray started.
            None => (zone.w * 2 / 3, zone.h * 2 / 3),
        };
        let (w, h) = (w.max(1), h.max(1));
        let target = Rect { x: zone.x + (zone.w - w) / 2, y: zone.y + (zone.h - h) / 2, w, h };
        self.place(hwnd, target, false, false);
        self.unsnap(hwnd, winutil::has_zone_stamp(hwnd));
        Ok(())
    }

    fn size_menu(&self, hwnd: HWND) -> Result<(), String> {
        let frame = self.sizable(hwnd)?;
        let (screen, from, to, _) = self.zone_here(hwnd, frame).ok_or("That window isn't in a zone")?;
        dividers::menu(hwnd.0 as isize, (screen, from, to), self.scene());
        Ok(())
    }

    fn drag_start(&mut self, hwnd: HWND) {
        self.cancel_drag();
        // A drag takes over from window control's moves still waiting.
        self.queued.clear();
        let key = hwnd.0 as isize;
        // A placement still waiting would yank the window back under the pointer.
        if self.pending.as_ref().is_some_and(|p| p.hwnd == key) {
            if let Some(p) = self.pending.take() {
                unsafe {
                    let _ = KillTimer(None, p.timer);
                }
            }
        }
        let wanted = self.active() || self.drags_sized();
        if !wanted || !winutil::is_snappable(hwnd, std::process::id(), &self.exclude) {
            return;
        }
        // Windows keeps a lower process from moving this one, so don't offer zones for it (D12).
        if winutil::is_elevated_over_us(hwnd) {
            if self.active() {
                self.blocked(hwnd);
            }
            return;
        }
        let geometry = if self.active() { self.geometry() } else { Vec::new() };
        let Some(start) = winutil::window_rect(hwnd) else { return };
        if geometry.is_empty() && !self.drags_sized() {
            return;
        }
        if !geometry.is_empty() {
            // The work area may have moved since the overlay was last drawn. This only posts a list.
            zone_paint::build(specs(&geometry));
        }
        // A swap, or Undo of a split by the drag's keys, puts windows where this one was.
        let frame = (self.drags_sized() || !geometry.is_empty()).then(|| winutil::visible_rect(hwnd)).flatten();
        let current = self.current(hwnd).map(|s| (s.old, (s.screen.clone(), s.from, s.to)));
        let was_snapped = current.as_ref().map(|(old, _)| *old);
        let stamped = winutil::has_zone_stamp(hwnd);
        let button = Some(main_button()).filter(|b| held(*b));
        let timer = unsafe { SetTimer(None, 0, TICK_MS, Some(on_tick)) };
        let was_maximized = winutil::is_maximized(hwnd);
        let zone = current.map(|(_, zone)| zone);
        let edges = self.edges_start(hwnd, &geometry, (zone.clone(), stamped), was_maximized);
        // The handles stay out of the way of any drag, and come back at its end.
        dividers::hide();
        self.drag = Some(Drag {
            hwnd: key,
            phase: Phase::Waiting,
            start,
            frame,
            was_maximized,
            stamped,
            old_zone: zone,
            was_snapped,
            button,
            geometry,
            anchor: None,
            shown: None,
            timer,
            edges,
            keys: dragkeys::Steer::default(),
            rest: None,
            paused: false,
        });
    }

    /// A snapped window dropped outside any zone gets its old size back, scaled
    /// to the screen it's on, with the pointer at the same place along its width.
    fn restore_size(&mut self, hwnd: HWND, (w, h, dpi): Old) {
        let Some(now) = winutil::window_rect(hwnd) else { return };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() || now.w <= 0 {
            return;
        }
        let k = winutil::monitor_dpi(hwnd).max(1) as f32 / dpi.max(1) as f32;
        let (w, h) = ((w as f32 * k).round() as i32, (h as f32 * k).round() as i32);
        let along = (pt.x - now.x) as f32 / now.w as f32;
        let x = pt.x - (along * w as f32).round() as i32;
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, now.y, w, h, flags);
        }
    }

    fn tick(&mut self) {
        let Some(mut drag) = self.drag.take() else { return };
        let mut pt = POINT::default();
        if !unsafe { IsWindow(Some(hwnd_of(drag.hwnd))) }.as_bool() {
            self.finish(drag, false);
            return;
        }
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            self.drag = Some(drag);
            return;
        }
        // The end of the drag never arrived, so don't let a drop snap by surprise.
        if drag.button.is_some_and(|b| !held(b)) {
            self.finish(drag, false);
            return;
        }
        if drag.phase == Phase::Waiting {
            let hwnd = hwnd_of(drag.hwnd);
            let Some(now) = winutil::window_rect(hwnd) else {
                self.drag = Some(drag);
                return;
            };
            if now != drag.start {
                let resized = !drag.was_maximized && (now.w != drag.start.w || now.h != drag.start.h);
                drag.phase = if resized { Phase::Resizing } else { Phase::Moving };
                if resized && drag.edges.is_none() {
                    // Zones have nothing to do with a resize, unless the neighbours follow it.
                    unsafe {
                        let _ = KillTimer(None, drag.timer);
                    }
                }
            }
        }
        if drag.phase == Phase::Resizing {
            let (hwnd, start) = (hwnd_of(drag.hwnd), drag.start);
            if let Some(edges) = drag.edges.as_mut() {
                edges.tick(hwnd, start);
            }
        }
        if drag.phase != Phase::Moving {
            self.drag = Some(drag);
            return;
        }
        let show_key = key_code(&self.settings.show_key).unwrap_or(VK_SHIFT);
        let span_key = key_code(&self.settings.span_key).filter(|k| *k != show_key);
        let pressed = held(show_key);
        self.pause(&mut drag, pt, pressed);
        let show = (pressed || drag.paused) && !drag.keys.off;
        let span = show && span_key.is_some_and(held);
        let view =
            show.then(|| drag.geometry.iter().position(|g| g.bounds.contains(pt.x, pt.y))).flatten().map(|screen| {
                let cell = zones::zone_at(&drag.geometry[screen].cells, pt.x, pt.y);
                let hit = cell.map(|zone| {
                    // Tab's choice holds while the pointer stays in the zone it was made in.
                    if drag.keys.cycle_at != Some((screen, zone)) {
                        drag.keys.cycle = 0;
                    }
                    // An arrow's side holds in its zone; elsewhere the side follows the pointer.
                    if drag.keys.side_at != Some((screen, zone)) {
                        drag.keys.side = None;
                    }
                    if drag.keys.cycle > 0 {
                        let list = dragkeys::candidates(&drag.geometry[screen].cells, zone, (pt.x, pt.y));
                        return list[drag.keys.cycle.min(list.len() - 1)];
                    }
                    if !span {
                        drag.anchor = None;
                        return (zone, zone);
                    }
                    match drag.anchor {
                        Some((s, from)) if s == screen => (from, zone),
                        _ => {
                            drag.anchor = Some((screen, zone));
                            (zone, zone)
                        }
                    }
                });
                let g = &drag.geometry[screen];
                let cut = hit.zip(drag.keys.part).map(|((a, b), part)| {
                    let r = zones::span(g.zones[a.min(b)], g.zones[a.max(b)]);
                    (part, drag.keys.side.unwrap_or_else(|| dragkeys::nearest_side(r, (pt.x, pt.y))))
                });
                View { screen, hit, cut }
            });
        // Repaint only when what is lit changes.
        if view != drag.shown {
            self.present(&mut drag, view);
        }
        self.drag = Some(drag);
    }

    fn hide_all(&self) {
        zone_paint::hide_all();
    }

    fn present(&mut self, drag: &mut Drag, view: Option<View>) {
        let started = log::enabled().then(Instant::now);
        if let Some(previous) = drag.shown.filter(|p| view.is_none_or(|v| v.screen != p.screen)) {
            zone_paint::hide(&drag.geometry[previous.screen].id);
        }
        // The keys are the drag's from the first time zones show, until it ends or Esc.
        if view.is_some() && !drag.keys.hooked && !drag.keys.off && !drag.keys.capped {
            let show_key = key_code(&self.settings.show_key).unwrap_or(VK_SHIFT);
            let span_key = key_code(&self.settings.span_key);
            let own = |vk: VIRTUAL_KEY| show_key == vk || span_key == Some(vk);
            drag.keys.generation = dragkeys::start(self.owner, own(VK_CONTROL), own(VK_MENU));
            drag.keys.hooked = true;
        }
        if drag.keys.hooked && !drag.keys.off {
            dragkeys::set_live(view.is_some());
        }
        if let Some(view) = view {
            let g = &drag.geometry[view.screen];
            // The whole span stays lit, and a share of it from the keys is lit again inside it.
            let (lit, part, others) = match view.hit {
                Some((a, b)) => {
                    let out = self.outcome(drag, view.screen, (a, b), view.cut);
                    let part = out.split.is_some().then_some(out.rect);
                    (Some(zones::span(g.zones[a], g.zones[b])), part, out.others())
                }
                None => (None, None, Vec::new()),
            };
            let hint = (drag.keys.hooked && self.settings.drag_hints).then(|| dragkeys::HINT.to_string());
            zone_paint::preview(&g.id, lit, part, others, hint);
        }
        drag.shown = view;
        if let Some(started) = started {
            log::info!("zone overlay: main {:.0}us", started.elapsed().as_secs_f64() * 1e6);
        }
    }

    fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            self.finish(drag, false);
        }
    }

    fn drag_end(&mut self, hwnd: HWND) {
        match self.drag.take() {
            Some(drag) if drag.hwnd == hwnd.0 as isize => self.finish(drag, true),
            other => self.drag = other,
        }
    }

    /// Stop the timer and hide the overlay. If it was showing a zone and the
    /// drag ended normally, put the window there.
    fn finish(&mut self, drag: Drag, dropped: bool) {
        unsafe {
            let _ = KillTimer(None, drag.timer);
        }
        self.hide_all();
        if drag.keys.hooked {
            dragkeys::stop();
        }
        if !drag.keys.used.is_empty() {
            log::info!("drag keys: {}", drag.keys.used.join(", "));
        }
        let hwnd = hwnd_of(drag.hwnd);
        self.finish_drag(&drag, dropped, hwnd);
        self.sync_dividers();
        // A re-fit that had to wait for this drag can go now.
        self.refit_resume();
    }

    fn finish_drag(&mut self, drag: &Drag, dropped: bool, hwnd: HWND) {
        // A window in a group of zones stays snapped, and its neighbours land.
        if drag.phase == Phase::Resizing {
            let ended = dropped || drag.button.is_some_and(|b| !held(b));
            if self.edges_end(drag, hwnd, ended) {
                return;
            }
        }
        // Resized by hand: it's no longer in its zone.
        if dropped && drag.phase == Phase::Resizing {
            self.unsnap(hwnd, drag.stamped);
            return;
        }
        // Maximized by the drag itself is Windows' top-edge snap, or a browser tab
        // merging back into a window. Either way it isn't ours to place.
        if !dropped || drag.phase != Phase::Moving {
            return;
        }
        if winutil::is_maximized(hwnd) && !drag.was_maximized {
            self.unsnap(hwnd, drag.stamped);
            return;
        }
        let lit = drag.shown.and_then(|view| Some((view.screen, view.hit?, view.cut)));
        let Some((screen, (a, b), cut)) = lit else {
            // Dropped outside every zone, so a snapped window gets its old size back.
            let mut restored = false;
            if let (Some(Some(old)), true) = (drag.was_snapped, self.settings.restore_size) {
                if !winutil::is_maximized(hwnd) && winutil::is_resizable(hwnd) {
                    self.restore_size(hwnd, old);
                    restored = true;
                }
            }
            self.unsnap(hwnd, drag.stamped);
            if !restored && !drag.was_maximized {
                self.keep_size(drag, hwnd);
            }
            return;
        };
        let out = self.outcome(drag, screen, (a, b), cut);
        if let Some(split) = out.split {
            self.split_drop(drag, hwnd, screen, split);
            return;
        }
        if out.swap.is_none() {
            if let Err(why) = self.swap_why(drag, screen, (a.min(b), a.max(b))) {
                if why != swap::Skip::Off {
                    log::info!("zone swap: skipped, {why:?}");
                }
            }
        }
        let g = &drag.geometry[screen];
        self.snap_to(hwnd, g, (a.min(b), a.max(b)), out.rect, drag.was_snapped);
        self.swap_start(out.swap);
    }

    /// A free drag that ended on another screen: with "Also when I drag a window to
    /// another screen" on, a worker gives the window the size the chosen mode keeps
    /// (§11.4). Only the screens are looked up here.
    fn keep_size(&self, drag: &Drag, hwnd: HWND) {
        let Some(frame) = drag.frame.filter(|_| self.drags_sized()) else { return };
        if winutil::is_maximized(hwnd) || !winutil::is_resizable(hwnd) {
            return;
        }
        let Some(now) = winutil::window_rect(hwnd) else { return };
        let at = |r: Rect| self.screens.iter().position(|s| s.bounds.contains(r.x + r.w / 2, r.y + r.h / 2));
        let (Some(from), Some(to)) = (at(frame), at(now)) else { return };
        if from == to {
            return;
        }
        let (a, b) = (&self.screens[from], &self.screens[to]);
        let (from, to) = ((from + 1, a.raw_dpi), (to + 1, b.raw_dpi, b.bounds));
        let job = DragJob { hwnd: drag.hwnd, mode: self.drag_size, frame, from, to };
        if std::thread::Builder::new().name("seam-window-size".into()).spawn(move || job.run()).is_err() {
            log::info!("window size: couldn't start the worker");
        }
    }

    /// The window stopped being in a zone, so forget it, including the stamp on the window.
    fn unsnap(&mut self, hwnd: HWND, stamped: bool) {
        self.snapped.remove(&(hwnd.0 as isize));
        if stamped {
            winutil::clear_zone_stamp(hwnd);
        }
        self.sync_dividers();
    }

    fn snap_to(&mut self, hwnd: HWND, g: &Placed, (from, to): (usize, usize), rect: Rect, was: Option<Option<Old>>) {
        let key = hwnd.0 as isize;
        // A window put in a zone a moment ago isn't there yet, but its old size still counts.
        let known = self.current(hwnd).is_some() || self.pending.as_ref().is_some_and(|p| p.hwnd == key);
        let old = match was {
            Some(old) => old,
            None => match self.snapped.get(&key).filter(|_| known) {
                Some(s) => s.old,
                None if winutil::is_maximized(hwnd) => None,
                None => winutil::window_rect(hwnd).map(|r| (r.w, r.h, winutil::monitor_dpi(hwnd))),
            },
        };
        // The screen's DPI, not the window's: a window that ignores DPI needs no extra step.
        let crosses = winutil::monitor_dpi(hwnd) != g.dpi;
        self.place(hwnd, rect, crosses, false);
        self.snapped.insert(key, Snapped { old, screen: g.id.clone(), from, to, at: rect });
        // The zones the window covers, kept on the window so a restart of the tray remembers them.
        winutil::stamp_zones(hwnd, &refit::covered(&g.cells, from, to));
        if self.snapped.len() > MAX_SNAPPED {
            // Windows that closed since. A system-wide destroy hook would cost more than this.
            self.snapped.retain(|k, _| unsafe { IsWindow(Some(hwnd_of(*k))) }.as_bool());
            if self.snapped.len() > MAX_SNAPPED {
                self.snapped.clear();
            }
        }
        self.sync_dividers();
    }

    /// Put the window's visible frame on `target`, a step at a time with the
    /// app's own reactions in between (D8).
    fn place(&mut self, hwnd: HWND, target: Rect, crosses_dpi: bool, quiet: bool) -> bool {
        let Some(before) = winutil::window_rect(hwnd) else { return false };
        if winutil::visible_rect(hwnd) == Some(target) {
            return false;
        }
        let mut steps = VecDeque::new();
        if winutil::is_maximized(hwnd) {
            steps.push_back(Step::Restore);
        }
        if crosses_dpi {
            steps.push_back(Step::Move);
        }
        steps.push_back(Step::Fit);
        steps.push_back(Step::Verify);
        if let Some(earlier) = self.pending.take() {
            unsafe {
                let _ = KillTimer(None, earlier.timer);
                // Another window's placement is cut short, so land it where it was going.
                let other = hwnd_of(earlier.hwnd);
                let unfinished = earlier.steps.iter().any(|s| matches!(s, Step::Fit));
                if earlier.hwnd != hwnd.0 as isize && unfinished && IsWindow(Some(other)).as_bool() {
                    winutil::fit(other, earlier.target);
                }
            }
        }
        self.pending = Some(Pending { hwnd: hwnd.0 as isize, target, before, steps, retries: 0, quiet, timer: 0 });
        self.advance_step();
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_WINDOWS_CHANGED, WPARAM(0), LPARAM(0));
        }
        true
    }

    /// The settle timer fired. When the placement is over, the next re-fit can start.
    fn advance(&mut self) {
        self.advance_step();
        if self.pending.is_none() {
            self.window_pump();
        }
        if self.pending.is_none() {
            self.refit_pump();
        }
    }

    /// Start window control's next move once the one before is over.
    fn window_pump(&mut self) {
        while self.pending.is_none() && self.drag.is_none() {
            let Some(p) = self.queued.pop_front() else { break };
            let hwnd = hwnd_of(p.hwnd);
            if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                continue;
            }
            let geometry = self.geometry();
            match p.zone.and_then(|(id, from, to)| Some((geometry.iter().find(|g| g.id == id)?, from, to))) {
                Some((g, from, to)) if to < g.zones.len() => self.snap_to(hwnd, g, (from, to), p.target, None),
                _ => {
                    // The screen's DPI, as in `snap_to`.
                    self.place(hwnd, p.target, winutil::monitor_dpi(hwnd) != p.dpi, false);
                    self.unsnap(hwnd, winutil::has_zone_stamp(hwnd));
                }
            }
        }
    }

    fn advance_step(&mut self) {
        let Some(mut p) = self.pending.take() else { return };
        if p.timer != 0 {
            unsafe {
                let _ = KillTimer(None, p.timer);
            }
            p.timer = 0;
        }
        let hwnd = hwnd_of(p.hwnd);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return;
        }
        match p.steps.pop_front() {
            Some(Step::Restore) => winutil::restore(hwnd),
            Some(Step::Move) => winutil::move_to(hwnd, p.target),
            Some(Step::Fit) => winutil::fit(hwnd, p.target),
            Some(Step::Verify) => {
                if winutil::window_rect(hwnd) == Some(p.before) {
                    if !p.quiet {
                        self.blocked(hwnd);
                    }
                    return;
                }
                // It moved, but an app that answered a DPI change late can leave it off by its margins.
                let (visible, sized) = (winutil::visible_rect(hwnd), winutil::is_resizable(hwnd));
                let landed = visible.is_some_and(|v| close(v, p.target, sized));
                if landed || p.retries >= MAX_RETRIES {
                    // Still bigger than its place after the retries: that's as small as the app goes.
                    if let (false, true, Some(v)) = (landed, sized, visible) {
                        minsize::observed(hwnd, v, p.target, CLOSE_ENOUGH);
                    }
                    return;
                }
                p.retries += 1;
                p.steps.extend([Step::Fit, Step::Verify]);
            }
            None => return,
        }
        p.timer = unsafe { SetTimer(None, 0, SETTLE_MS, Some(on_settle)) };
        self.pending = Some(p);
    }

    /// Say so once per app per run, because a window that won't move is nearly
    /// always one running as administrator (D12).
    fn blocked(&mut self, hwnd: HWND) {
        log::info!("a window didn't move, probably because it runs as administrator");
        let path = winutil::exe_path_of(winutil::process_id(hwnd)).unwrap_or_default();
        if self.warned.insert(path) {
            *BLOCKED_APP.lock().unwrap_or_else(|e| e.into_inner()) = winutil::app_name(hwnd);
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_SNAP_BLOCKED, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn move_foreground(&mut self, next: bool) -> Result<(), String> {
        if let Some(why) = &self.aside {
            return Err(why.clone());
        }
        if !self.enabled {
            return Err("Zones are off".into());
        }
        let geometry = self.geometry();
        if geometry.is_empty() {
            return Err("No screen has zones yet. Pick a layout on the Zones page".into());
        }
        let hwnd = unsafe { GetForegroundWindow() };
        if !winutil::is_snappable(hwnd, std::process::id(), &self.exclude) {
            return Err("That window can't be moved into a zone".into());
        }
        // Every zone in order, screen by screen, so the last one wraps onto the next screen.
        let flat: Vec<(usize, usize)> =
            geometry.iter().enumerate().flat_map(|(s, g)| (0..g.zones.len()).map(move |z| (s, z))).collect();
        let position = |id: &str, zone: usize| geometry.iter().position(|g| g.id == id).map(|s| (s, zone));
        let snapped = self.current(hwnd).and_then(|s| position(&s.screen, if next { s.to } else { s.from }));
        let centre = winutil::visible_rect(hwnd).map(|r| (r.x + r.w / 2, r.y + r.h / 2));
        let under = centre.and_then(|(x, y)| {
            let s = geometry.iter().position(|g| g.bounds.contains(x, y))?;
            Some((s, zones::zone_at(&geometry[s].cells, x, y)?))
        });
        let n = flat.len();
        let index = match (snapped, under) {
            (Some(at), _) => {
                flat.iter().position(|f| *f == at).map(|i| if next { (i + 1) % n } else { (i + n - 1) % n })
            }
            (None, Some(at)) => flat.iter().position(|f| *f == at),
            (None, None) => Some(if next { 0 } else { n - 1 }),
        };
        let (screen, zone) = flat[index.unwrap_or(0)];
        let g = &geometry[screen];
        self.snap_to(hwnd, g, (zone, zone), g.zones[zone], None);
        Ok(())
    }
}

/// A free drag between screens, for its worker.
struct DragJob {
    hwnd: isize,
    /// Pixels or real size, never Windows'.
    mode: WindowSize,
    /// The visible frame before the drag.
    frame: Rect,
    /// Line Up number and raw density of the screen it left.
    from: (usize, f64),
    /// The same of the screen it was dropped on, and its bounds.
    to: (usize, f64, Rect),
}

impl DragJob {
    /// Once the app has answered the DPI change of the drop (D8), undo Windows' rescale:
    /// fit the frame to the size the mode keeps, centred where it was dropped. Only the
    /// frame is resized.
    fn run(self) {
        std::thread::sleep(std::time::Duration::from_millis(SETTLE_MS.into()));
        let h = hwnd_of(self.hwnd);
        let ((from, src), (to, dst, bounds)) = (self.from, self.to);
        let (Some(now), Some((work, _))) = (winutil::visible_rect(h), work_area(bounds)) else { return };
        let app = winutil::exe_name(h).unwrap_or_default();
        let (w, ht, mode) = (self.frame.w, self.frame.h, self.mode);
        let Some(r) = desk::moved_rect(mode, (w as f64, ht as f64), (src, dst), now, work) else {
            log::info!("window size: {app} scaled by Windows, screen {from} or {to} doesn't report its size");
            return;
        };
        log::info!("window size ({mode:?}): {app} screen {from} → {to}, {w}×{ht} px → {}×{} px", r.w, r.h);
        if !winutil::is_hung(h) {
            winutil::fit(h, r);
        }
    }
}
