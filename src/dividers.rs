//! Divider handles: a thin window in the gap between two snapped windows, along
//! each edge of a screen's zones that has a window on both sides. A handle can't
//! be seen until the pointer is on it, then it shows a slim bar. Dragging one moves
//! that divider, and the windows on both sides follow the way an edge drag's
//! neighbours do (`snap::Member`); letting go saves the layout once, as Custom.
//! Right-click gives shares of the two windows and Take out, and the size menu key
//! gives the same for the focused window, which works with no gap too.
//!
//! Everything lives on a thread of its own. The tray's main thread only hands over
//! what the zones look like (`show`), and the thread builds the handles a moment
//! later, never per tick. A handle sits just above the higher of its two windows, so
//! a window lying over the gap keeps its clicks, and it never takes focus. Only a
//! menu does, while it is open.

use crate::snap::{Member, Zoned};
use crate::wincontrol;
use crate::{dragkeys, log};
use seam::api::{Axis, GroupVerb};
use seam::groups::{self, Edge, Side};
use seam::winutil;
use seam::zones::{self, Dir, Rect, ZoneLayout};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

mod menu;
mod place;

pub use menu::{menu, take_choice};
use place::{gap_between, paint, spots, zones_of};

const CLASS: PCWSTR = w!("SeamDivider");
/// A menu choice is waiting for the tray's main thread.
pub const WM_APP_DIVIDER: u32 = WM_APP + 31;
/// To the thread's own window: the zones changed, a drag began, the size menu key.
const WM_APP_SCENE: u32 = WM_APP + 1;
const WM_APP_HIDE: u32 = WM_APP + 2;
const WM_APP_MENU: u32 = WM_APP + 3;
/// From TrackMouseEvent, defined in a part of the windows crate Seam doesn't build.
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_BUILD: usize = 1;
const TIMER_CHECK: usize = 2;
const TIMER_FOLLOW: usize = 3;
/// The handles are built this long after the last change, once things settle.
const BUILD_AFTER_MS: u32 = 150;
/// How often the handles look whether their windows are still where they were.
const CHECK_MS: u32 = 250;
/// How often the windows beside a dragged handle are sent their latest place.
const FOLLOW_MS: u32 = 16;
/// No handle in a gap narrower than this.
const MIN_GAP_PX: i32 = 4;
/// The bar's thickness at 96 dpi.
const BAR_PX: f32 = 4.0;
/// Muted ink, `#949494`, opaque and premultiplied.
const BAR: u32 = 0xFF94_9494;
/// Alpha 1: can't be seen, but still takes the pointer.
const CLEAR: u32 = 0x0100_0000;
/// The shares the menus offer, in this order.
const SHARES: [(&str, f32); 5] = [("1/2", 0.5), ("1/3", 1.0 / 3.0), ("2/3", 2.0 / 3.0), ("1/4", 0.25), ("3/4", 0.75)];

/// One screen with zones, in pixels.
pub struct Screen {
    pub id: String,
    pub bounds: Rect,
    pub work: Rect,
    pub dpi: u32,
    /// The gap at this screen's scale.
    pub gap: i32,
    pub layout: ZoneLayout,
}

/// The zones as the snapping sees them now.
pub struct Scene {
    /// The tray's window, which hears of a menu choice and saves a dragged layout.
    pub owner: isize,
    pub screens: Vec<Screen>,
    /// The snapped windows as the snapping recorded them.
    pub known: Vec<Zoned>,
    /// Lower-case exe names the zones leave alone.
    pub exclude: Vec<String>,
}

/// What a menu choice asks the tray's main thread to do.
#[derive(Clone, Copy, Debug)]
pub enum Choice {
    /// The window gets a share of itself and its neighbour.
    Share(isize, GroupVerb),
    TakeOut(isize),
}

/// The thread's own window, which the thread puts here once it is up, or 0.
static WINDOW: Mutex<isize> = Mutex::new(0);
/// Where the thread is: not started, started (so it isn't started again while it
/// comes up), or told to stop, which it does as soon as it is up.
static THREAD: AtomicU8 = AtomicU8::new(IDLE);
const IDLE: u8 = 0;
const STARTED: u8 = 1;
const STOPPED: u8 = 2;
/// A handle is being dragged, so the windows beside it move on purpose.
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// The latest zones from the main thread. Some(None) means no handles.
static SCENE: Mutex<Option<Option<Scene>>> = Mutex::new(None);

/// Hand over the zones as they are now, or None for no handles. The handles are
/// built on their thread a moment later.
pub fn show(scene: Option<Scene>) {
    let start = scene.is_some();
    *SCENE.lock().unwrap_or_else(|e| e.into_inner()) = Some(scene);
    post(start, WM_APP_SCENE);
}

/// Whether a handle is being dragged now.
pub fn dragging() -> bool {
    DRAGGING.load(Ordering::SeqCst)
}

/// A drag began: the handles hide until the zones come back with `show`.
pub fn hide() {
    post(false, WM_APP_HIDE);
}

/// Seam is quitting: the handles go with their thread. A thread still coming up
/// sees the flag under the same lock and ends once it is up.
pub fn stop() {
    let mut window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    THREAD.store(STOPPED, Ordering::SeqCst);
    if *window != 0 {
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(*window)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        *window = 0;
    }
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

/// Post to the thread's window, or start the thread when `start` is set and it
/// isn't running. Nobody waits for it: once up, it reads what is waiting itself.
fn post(start: bool, message: u32) {
    let window = *WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if window != 0 {
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(window)), message, WPARAM(0), LPARAM(0));
        }
    } else if start
        && THREAD.compare_exchange(IDLE, STARTED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
        && let Err(e) = std::thread::Builder::new().name("seam-dividers".into()).spawn(run)
    {
        log::info!("divider handles: couldn't start: {e}");
        let _ = THREAD.compare_exchange(STARTED, IDLE, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Where a handle goes: the divider, and the two windows either side of it as
/// (window, first zone, last zone) with the rect of their zones.
struct Spot {
    screen: usize,
    path: Vec<bool>,
    dir: Dir,
    a: (isize, usize, usize),
    b: (isize, usize, usize),
    zones: (Rect, Rect),
    rect: Rect,
    dpi: u32,
}

struct Handle {
    hwnd: HWND,
    spot: Spot,
    hovered: bool,
    /// Its windows moved or closed since it was built, so it stays hidden until the next build.
    gone: bool,
    /// Another window shows through its gap, so it hides while that lasts.
    covered: bool,
}

/// A handle being dragged.
struct Dragging {
    handle: usize,
    /// Where the pointer was along the divider's axis when it began.
    start: i32,
    edge: Edge,
    members: Vec<Member>,
    mins: Vec<(i32, i32)>,
    delta: i32,
    /// The layout for the move so far, and the zones it gives, once it moved.
    solved: Option<(zones::Node, Vec<Rect>)>,
    moves: u32,
    longest: Duration,
}

struct State {
    /// The thread's own window: messages come here, and it owns the menus.
    owner: HWND,
    scene: Option<Scene>,
    /// Zones that came during a handle drag, taken once it ends.
    pending: Option<Option<Scene>>,
    /// The windows in zones the last build found, so focus moving to another one
    /// (on a desktop just switched to, say) builds the handles again.
    seen: HashSet<isize>,
    handles: Vec<Handle>,
    /// A drag of a window is on, so the handles stay out of the way.
    hidden: bool,
    hook: Option<HWINEVENTHOOK>,
    drag: Option<Dragging>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// None while the state is in use further up, as when a window call of its own
/// sends a message back here.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

fn cursor() -> POINT {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    pt
}

/// The pointer's place along the axis a divider in `dir` moves on.
fn along(dir: Dir, pt: POINT) -> i32 {
    match dir {
        Dir::Vertical => pt.x,
        Dir::Horizontal => pt.y,
    }
}

fn is_topmost(h: HWND) -> bool {
    unsafe { GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

/// A window is still in its zones: there, not minimized, on this desktop.
fn still(key: isize, zone: Rect) -> bool {
    let h = hwnd_of(key);
    if !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_minimized(h) || winutil::is_cloaked(h) {
        return false;
    }
    winutil::visible_rect(h).is_some_and(|r| wincontrol::close(r, zone, winutil::is_resizable(h)))
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_state(|s| s.foreground(hwnd));
}

impl State {
    fn handle(&self, hwnd: HWND) -> Option<usize> {
        self.handles.iter().position(|h| h.hwnd == hwnd)
    }

    fn schedule(&self) {
        unsafe {
            SetTimer(Some(self.owner), TIMER_BUILD, BUILD_AFTER_MS, None);
        }
    }

    /// The main thread's latest zones. During a handle drag they wait for its end.
    fn take_scene(&mut self) {
        let Some(scene) = SCENE.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        if self.drag.is_some() {
            self.pending = Some(scene);
        } else {
            self.apply(scene);
        }
    }

    /// New zones. None takes every handle away.
    fn apply(&mut self, scene: Option<Scene>) {
        self.scene = scene;
        self.hidden = false;
        if self.scene.is_none() {
            self.clear();
            return;
        }
        if self.hook.is_none() {
            // Focus moving can lift a window over a handle's gap, or show another desktop.
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let event = EVENT_SYSTEM_FOREGROUND;
            let hook = unsafe { SetWinEventHook(event, event, None, Some(on_foreground), 0, 0, flags) };
            self.hook = (!hook.0.is_null()).then_some(hook);
        }
        self.schedule();
    }

    fn hide(&mut self) {
        self.hidden = true;
        unsafe {
            let _ = KillTimer(Some(self.owner), TIMER_BUILD);
        }
        for h in &self.handles {
            conceal(h.hwnd);
        }
    }

    /// No handles at all: any drag of one goes back, and the windows go.
    fn clear(&mut self) {
        if let Some(drag) = self.drag.take() {
            for m in &drag.members {
                m.back();
            }
            dragkeys::release();
            DRAGGING.store(false, Ordering::SeqCst);
        }
        for h in self.handles.drain(..) {
            unsafe {
                let _ = DestroyWindow(h.hwnd);
            }
        }
        self.seen.clear();
        if let Some(hook) = self.hook.take() {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        unsafe {
            for timer in [TIMER_BUILD, TIMER_CHECK, TIMER_FOLLOW] {
                let _ = KillTimer(Some(self.owner), timer);
            }
        }
    }

    /// Build the handles for the zones now. The windows already made are used
    /// again, so building them again doesn't flicker, unless one was hidden or is on
    /// another desktop: that one is made again on this one.
    fn build(&mut self) {
        if self.hidden || self.drag.is_some() {
            return;
        }
        let started = Instant::now();
        let (spots, seen) = self.scene.as_ref().map(spots).unwrap_or_default();
        self.seen = seen;
        while self.handles.len() > spots.len() {
            if let Some(h) = self.handles.pop() {
                unsafe {
                    let _ = DestroyWindow(h.hwnd);
                }
            }
        }
        for (i, spot) in spots.into_iter().enumerate() {
            // A hidden handle may not report cloaked after a desktop switch, so only one
            // that is shown and not cloaked is used again.
            let shown = |h: &HWND| unsafe { IsWindowVisible(*h) }.as_bool() && !winutil::is_cloaked(*h);
            let reused = self.handles.get(i).map(|h| h.hwnd).filter(shown);
            let Some(hwnd) = reused.or_else(open) else { break };
            let handle = Handle { hwnd, spot, hovered: false, gone: false, covered: false };
            match self.handles.get_mut(i) {
                Some(h) => {
                    if h.hwnd != hwnd {
                        unsafe {
                            let _ = DestroyWindow(h.hwnd);
                        }
                    }
                    *h = handle;
                }
                None => self.handles.push(handle),
            }
            let h = &self.handles[i];
            paint(h.hwnd, h.spot.rect, h.spot.dir, false, h.spot.dpi);
        }
        self.reorder();
        unsafe {
            // Only handles need watching.
            if self.handles.is_empty() {
                let _ = KillTimer(Some(self.owner), TIMER_CHECK);
            } else {
                SetTimer(Some(self.owner), TIMER_CHECK, CHECK_MS, None);
            }
        }
        let ms = started.elapsed().as_secs_f64() * 1e3;
        log::info!("divider handles: {} built in {ms:.1}ms", self.handles.len());
    }

    /// Each handle just above the higher of its two windows that isn't topmost, and
    /// shown. A handle whose windows are both topmost isn't shown, as a handle never is.
    fn reorder(&self) {
        if self.hidden || self.handles.is_empty() {
            return;
        }
        let order = winutil::top_level_windows();
        let ours = |w: &HWND| self.handles.iter().any(|h| h.hwnd == *w);
        for h in self.handles.iter().filter(|h| !h.gone && !h.covered) {
            let (a, b) = (h.spot.a.0, h.spot.b.0);
            let neighbour = |w: &HWND| w.0 as isize == a || w.0 as isize == b;
            if !order.iter().any(neighbour) {
                continue;
            }
            let Some(i) = order.iter().position(|w| neighbour(w) && !is_topmost(*w)) else {
                conceal(h.hwnd);
                continue;
            };
            let above = order[..i].iter().rev().find(|w| !ours(w) && !is_topmost(**w));
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW;
            unsafe {
                let _ = SetWindowPos(h.hwnd, Some(above.copied().unwrap_or(HWND_TOP)), 0, 0, 0, 0, flags);
            }
        }
    }

    /// A handle whose windows closed, were minimized or moved since hides, and the
    /// handles are built again. One with another window lying over its gap, above
    /// the lower of its two windows, hides until that window goes.
    fn check(&mut self) {
        if self.hidden || self.drag.is_some() {
            return;
        }
        let own = std::process::id();
        // One pass, top first, so a window's place in it is its place in the z-order.
        let order = winutil::top_level_windows();
        let z: HashMap<isize, usize> = order.iter().enumerate().map(|(i, w)| (w.0 as isize, i)).collect();
        let others: Vec<(usize, isize, Rect)> = order
            .iter()
            .enumerate()
            .filter(|(_, w)| winutil::is_snappable(**w, own, &[]) && !winutil::is_maximized(**w))
            .filter_map(|(i, w)| Some((i, w.0 as isize, winutil::visible_rect(*w)?)))
            .collect();
        let (mut stale, mut uncovered) = (false, false);
        for h in self.handles.iter_mut().filter(|h| !h.gone) {
            if !still(h.spot.a.0, h.spot.zones.0) || !still(h.spot.b.0, h.spot.zones.1) {
                (stale, h.gone) = (true, true);
                conceal(h.hwnd);
                continue;
            }
            let (a, b, r) = (h.spot.a.0, h.spot.b.0, h.spot.rect);
            // Only a window above the lower of the two lies over the layout. One
            // behind both shows through the gap but is no reason to hide.
            let lower = z.get(&a).zip(z.get(&b)).map(|(x, y)| *x.max(y));
            let over = |i: usize| lower.is_some_and(|l| i < l);
            let covered = others.iter().any(|(i, w, o)| *w != a && *w != b && over(*i) && meets(*o, r));
            if covered != h.covered {
                h.covered = covered;
                if covered {
                    conceal(h.hwnd);
                } else {
                    uncovered = true;
                }
            }
        }
        if uncovered {
            self.reorder();
        }
        if stale {
            self.schedule();
        }
    }

    /// Focus moved. A window in zones the last build didn't see may be on a desktop
    /// just switched to.
    fn foreground(&mut self, fg: HWND) {
        if self.hidden || self.drag.is_some() {
            return;
        }
        if !self.seen.contains(&(fg.0 as isize)) && winutil::has_zone_stamp(fg) {
            self.schedule();
        }
        self.reorder();
    }

    fn timer(&mut self, id: usize) {
        match id {
            TIMER_BUILD => {
                unsafe {
                    let _ = KillTimer(Some(self.owner), TIMER_BUILD);
                }
                self.build();
            }
            TIMER_CHECK => self.check(),
            TIMER_FOLLOW => self.drag_to(),
            _ => {}
        }
    }

    fn set_cursor(&self, hwnd: HWND) -> bool {
        let Some(h) = self.handle(hwnd).and_then(|i| self.handles.get(i)) else { return false };
        let shape = if h.spot.dir == Dir::Vertical { IDC_SIZEWE } else { IDC_SIZENS };
        unsafe {
            if let Ok(c) = LoadCursorW(None, shape) {
                SetCursor(Some(c));
            }
        }
        true
    }

    fn hover(&mut self, hwnd: HWND, on: bool) {
        let dragged = self.drag.as_ref().map(|d| d.handle);
        let Some(i) = self.handle(hwnd).filter(|i| Some(*i) != dragged) else { return };
        let Some(h) = self.handles.get_mut(i) else { return };
        if h.hovered == on {
            return;
        }
        h.hovered = on;
        paint(h.hwnd, h.spot.rect, h.spot.dir, on, h.spot.dpi);
        if on {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
        }
    }

    /// The left button went down on a handle: its divider moves with the pointer.
    fn press(&mut self, hwnd: HWND) {
        if self.drag.is_some() {
            return;
        }
        let Some(i) = self.handle(hwnd) else { return };
        let (Some(scene), Some(handle)) = (self.scene.as_ref(), self.handles.get(i)) else { return };
        let spot = &handle.spot;
        let Some(s) = scene.screens.get(spot.screen) else { return };
        let placed = zones::rects(&s.layout.tree, s.work, s.gap);
        let list = wincontrol::screen_members(&s.id, s.bounds, &placed, &scene.known, &scene.exclude);
        let mins = wincontrol::group_mins(&s.layout.tree, s.work, s.gap, s.dpi, &list);
        let member = |(h, from, to): (HWND, usize, usize)| {
            Member::of(h, from, to, zones::span(*placed.get(from)?, *placed.get(to)?))
        };
        let members = list.into_iter().filter_map(member).collect();
        let start = along(spot.dir, cursor());
        let edge = Edge { path: spot.path.clone() };
        for h in self.handles.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, h)| h) {
            conceal(h.hwnd);
        }
        self.drag = Some(Dragging {
            handle: i,
            start,
            edge,
            members,
            mins,
            delta: 0,
            solved: None,
            moves: 0,
            longest: Duration::ZERO,
        });
        DRAGGING.store(true, Ordering::SeqCst);
        // Esc is the drag's, so it doesn't reach the focused app as well.
        dragkeys::start_esc(self.owner.0 as isize);
        unsafe {
            let _ = SetCapture(hwnd);
            SetTimer(Some(self.owner), TIMER_FOLLOW, FOLLOW_MS, None);
        }
    }

    /// The pointer moved, or it's time to send the windows their latest places. Esc
    /// puts everything back.
    fn drag_to(&mut self) {
        let Some(mut drag) = self.drag.take() else { return };
        let target = self.handles.get(drag.handle).and_then(|h| {
            let s = self.scene.as_ref()?.screens.get(h.spot.screen)?;
            Some((h, s))
        });
        let escape = unsafe { GetAsyncKeyState(i32::from(VK_ESCAPE.0)) } < 0;
        let Some((handle, s)) = target.filter(|_| !escape) else {
            self.drag = Some(drag);
            self.release(false);
            return;
        };
        let started = Instant::now();
        let delta = along(handle.spot.dir, cursor()) - drag.start;
        if delta != drag.delta {
            drag.delta = delta;
            let tree = groups::solve(&s.layout.tree, s.work, &drag.edge, delta, &drag.mins).tree;
            let placed = zones::rects(&tree, s.work, s.gap);
            let (a, b, dir) = (handle.spot.a, handle.spot.b, handle.spot.dir);
            if let Some(r) = zones_of(&placed, a, b).and_then(|pair| gap_between(pair, dir)) {
                paint(handle.hwnd, r, dir, true, handle.spot.dpi);
            }
            drag.solved = Some((tree, placed));
            drag.moves += 1;
        }
        if let Some((_, placed)) = drag.solved.as_ref() {
            for m in &mut drag.members {
                if let (Some(a), Some(b)) = (placed.get(m.from), placed.get(m.to)) {
                    m.follow(zones::span(*a, *b));
                }
            }
        }
        drag.longest = drag.longest.max(started.elapsed());
        self.drag = Some(drag);
    }

    /// The drag is over. Let go, the windows land and the layout is saved once;
    /// cut short or Esc, they go back. Zones that came meanwhile are taken now.
    fn release(&mut self, commit: bool) {
        let Some(drag) = self.drag.take() else { return };
        dragkeys::release();
        unsafe {
            let _ = KillTimer(Some(self.owner), TIMER_FOLLOW);
            let _ = ReleaseCapture();
        }
        let us = drag.longest.as_secs_f64() * 1e6;
        log::info!("divider drag: {} windows, {} moves, max {us:.0}us", drag.members.len(), drag.moves);
        if !(commit && self.land(&drag)) {
            drag.members.iter().for_each(Member::back);
        }
        DRAGGING.store(false, Ordering::SeqCst);
        match self.pending.take() {
            Some(scene) => self.apply(scene),
            None => self.schedule(),
        }
    }

    /// Put the windows of a finished drag in their places and save the layout.
    /// False when the drag changed nothing, or its screen is gone.
    fn land(&mut self, drag: &Dragging) -> bool {
        let Some(i) = self.handles.get(drag.handle).map(|h| h.spot.screen) else { return false };
        let Some(scene) = self.scene.as_mut() else { return false };
        let Some(s) = scene.screens.get_mut(i) else { return false };
        let Some((tree, placed)) = drag.solved.as_ref().filter(|(tree, _)| *tree != s.layout.tree) else {
            return false;
        };
        let mut moved = Vec::new();
        for m in drag.members.iter().filter(|m| m.here(&scene.exclude)) {
            let (Some(a), Some(b)) = (placed.get(m.from), placed.get(m.to)) else { continue };
            let at = zones::span(*a, *b);
            m.land(at);
            // The handles are built from these places until the main thread's zones come.
            match scene.known.iter_mut().find(|k| k.hwnd == m.hwnd) {
                Some(k) => (k.zone, k.at, k.moving) = ((s.id.clone(), m.from, m.to), at, false),
                None => scene.known.push(Zoned { hwnd: m.hwnd, zone: (s.id.clone(), m.from, m.to), at, moving: false }),
            }
            moved.push(Zoned { hwnd: m.hwnd, zone: (s.id.clone(), m.from, m.to), at, moving: false });
        }
        let layout = groups::relabel(s.layout.template, tree.clone());
        s.layout = layout.clone();
        wincontrol::save_moved(scene.owner, s.id.clone(), layout, moved);
        true
    }
}

/// A new handle window: layered, never active, and nothing to see yet.
fn open() -> Option<HWND> {
    let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let class = (CLASS, w!("Seam divider"));
    unsafe { CreateWindowExW(ex, class.0, class.1, WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst.into()), None) }.ok()
}

/// Hide a handle where it is. `reorder` shows it again in its place.
fn conceal(hwnd: HWND) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_HIDEWINDOW;
    unsafe {
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, flags);
    }
}

/// Two rects share some area.
fn meets(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_APP_SCENE => {
                with_state(State::take_scene);
            }
            WM_APP_HIDE => {
                with_state(State::hide);
            }
            WM_APP_MENU => menu::asked_menu(),
            WM_TIMER => {
                with_state(|s| s.timer(wparam.0));
            }
            // A click never makes a handle the active window.
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR if with_state(|s| s.set_cursor(hwnd)) == Some(true) => return LRESULT(1),
            WM_MOUSEMOVE => {
                with_state(|s| match s.drag {
                    Some(_) => s.drag_to(),
                    None => s.hover(hwnd, true),
                });
            }
            WM_MOUSELEAVE => {
                with_state(|s| s.hover(hwnd, false));
            }
            WM_LBUTTONDOWN => {
                with_state(|s| s.press(hwnd));
            }
            WM_LBUTTONUP => {
                with_state(|s| s.release(true));
            }
            // Capture taken away mid-drag. Its own release comes here too, and finds no drag.
            WM_CAPTURECHANGED => {
                with_state(|s| s.release(false));
            }
            // Esc from the drag-keys hook, which swallowed it: the drag goes back.
            dragkeys::WM_APP_DRAG_KEY if wparam.0 == usize::from(VK_ESCAPE.0) => {
                with_state(|s| s.release(false));
            }
            WM_RBUTTONUP => menu::handle_menu(hwnd),
            WM_CLOSE if with_state(|s| s.owner == hwnd) == Some(true) => {
                with_state(State::clear);
                let _ = DestroyWindow(hwnd);
                PostQuitMessage(0);
            }
            _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
        }
        LRESULT(0)
    }
}

fn run() {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        // A hidden window, not message-only, so it can own a menu in front.
        let made = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!("Seam dividers"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        );
        let Ok(owner) = made else {
            log::info!("divider handles: couldn't make the thread's window");
            // A later post may try again, unless Seam is quitting.
            let _ = THREAD.compare_exchange(STARTED, IDLE, Ordering::SeqCst, Ordering::SeqCst);
            return;
        };
        let state = State {
            owner,
            scene: None,
            pending: None,
            seen: HashSet::new(),
            handles: Vec::new(),
            hidden: false,
            hook: None,
            drag: None,
        };
        STATE.with(|s| *s.borrow_mut() = Some(state));
        // From here the main thread posts here, unless Seam began to quit while this
        // came up. What it left before is read now.
        {
            let mut window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            if THREAD.load(Ordering::SeqCst) == STOPPED {
                let _ = DestroyWindow(owner);
                return;
            }
            *window = owner.0 as isize;
        }
        with_state(State::take_scene);
        menu::asked_menu();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
