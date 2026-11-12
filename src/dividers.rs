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

use crate::log;
use crate::snap::{Member, Zoned};
use crate::wincontrol;
use seam::api::{Axis, GroupVerb};
use seam::groups::{self, Edge, Side};
use seam::winutil;
use seam::zones::{self, Dir, Rect, ZoneLayout};
use std::cell::RefCell;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

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

/// A menu line and what it does, or None for a separator.
type Item = Option<(String, Choice)>;
/// The size menu key's window, its zones, and the zones then.
type Asked = (isize, (String, usize, usize), Scene);

/// The thread's own window, or 0 while the thread isn't running.
static WINDOW: Mutex<isize> = Mutex::new(0);
/// The latest zones from the main thread. Some(None) means no handles.
static SCENE: Mutex<Option<Option<Scene>>> = Mutex::new(None);
/// The size menu key's ask, waiting for the thread.
static ASKED: Mutex<Option<Asked>> = Mutex::new(None);
static CHOICE: Mutex<Option<Choice>> = Mutex::new(None);

/// Hand over the zones as they are now, or None for no handles. The handles are
/// built on their thread a moment later.
pub fn show(scene: Option<Scene>) {
    let start = scene.is_some();
    *SCENE.lock().unwrap_or_else(|e| e.into_inner()) = Some(scene);
    post(start, WM_APP_SCENE);
}

/// A drag began: the handles hide until the zones come back with `show`.
pub fn hide() {
    post(false, WM_APP_HIDE);
}

/// Open the size menu for `hwnd`, in zones `zone`, at its centre.
pub fn menu(hwnd: isize, zone: (String, usize, usize), scene: Scene) {
    *ASKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((hwnd, zone, scene));
    post(true, WM_APP_MENU);
}

/// The menu choice waiting for the main thread, once.
pub fn take_choice() -> Option<Choice> {
    CHOICE.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Seam is quitting: the handles go with their thread.
pub fn stop() {
    let mut window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
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

/// Post to the thread's window, starting the thread first when `start` is set.
fn post(start: bool, message: u32) {
    let mut window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if *window == 0 && start {
        let (ready, wait) = channel();
        match std::thread::Builder::new().name("seam-dividers".into()).spawn(move || run(ready)) {
            Ok(_) => *window = wait.recv().unwrap_or(0),
            Err(e) => log::info!("divider handles: couldn't start: {e}"),
        }
    }
    if *window != 0 {
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(*window)), message, WPARAM(0), LPARAM(0));
        }
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

/// Where a divider's line is, as `zones::rects` draws it.
fn line(d: &zones::Divider) -> i32 {
    match d.dir {
        Dir::Vertical => d.area.x + (d.area.w as f32 * d.ratio).round() as i32,
        Dir::Horizontal => d.area.y + (d.area.h as f32 * d.ratio).round() as i32,
    }
}

/// The zones of `a`, before a divider, and of `b`, after it, in `placed`.
fn zones_of(placed: &[Rect], a: (isize, usize, usize), b: (isize, usize, usize)) -> Option<(Rect, Rect)> {
    let span = |(_, from, to): (isize, usize, usize)| Some(zones::span(*placed.get(from)?, *placed.get(to)?));
    Some((span(a)?, span(b)?))
}

/// The gap between two zones either side of a divider, along the stretch where
/// they lie side by side. None where it's too narrow for a handle.
fn gap_between((za, zb): (Rect, Rect), dir: Dir) -> Option<Rect> {
    let r = match dir {
        Dir::Vertical => {
            let (x, y) = (za.x + za.w, za.y.max(zb.y));
            Rect { x, y, w: zb.x - x, h: (za.y + za.h).min(zb.y + zb.h) - y }
        }
        Dir::Horizontal => {
            let (x, y) = (za.x.max(zb.x), za.y + za.h);
            Rect { x, y, w: (za.x + za.w).min(zb.x + zb.w) - x, h: zb.y - y }
        }
    };
    let thick = if dir == Dir::Vertical { r.w } else { r.h };
    (thick >= MIN_GAP_PX && r.w > 0 && r.h > 0).then_some(r)
}

/// Every handle the zones call for: one per pair of windows either side of a
/// divider, where both lie wholly in that divider's split. Worker only: it reads
/// every window's rect.
fn spots(scene: &Scene) -> Vec<Spot> {
    let mut out = Vec::new();
    for (i, s) in scene.screens.iter().enumerate().filter(|(_, s)| s.gap >= MIN_GAP_PX) {
        let tree = &s.layout.tree;
        let (cells, placed) = (zones::rects(tree, s.work, 0), zones::rects(tree, s.work, s.gap));
        let found = wincontrol::screen_members(&s.id, s.bounds, &placed, &scene.known, &scene.exclude);
        // A window Seam may not move gets no handle.
        let members: Vec<(isize, usize, usize)> = found
            .into_iter()
            .filter(|(h, ..)| wincontrol::movable(*h, &scene.exclude).is_ok())
            .map(|(h, from, to)| (h.0 as isize, from, to))
            .collect();
        for d in zones::dividers(tree, s.work) {
            let at = line(&d);
            let ends = |(_, from, to): &(isize, usize, usize)| {
                let c = zones::span(*cells.get(*from)?, *cells.get(*to)?);
                let inside = c.x >= d.area.x
                    && c.y >= d.area.y
                    && c.x + c.w <= d.area.x + d.area.w
                    && c.y + c.h <= d.area.y + d.area.h;
                let ends = match d.dir {
                    Dir::Vertical => (c.x, c.x + c.w),
                    Dir::Horizontal => (c.y, c.y + c.h),
                };
                inside.then_some(ends)
            };
            let before: Vec<_> = members.iter().filter(|m| ends(m).is_some_and(|(_, end)| end == at)).collect();
            let after: Vec<_> = members.iter().filter(|m| ends(m).is_some_and(|(start, _)| start == at)).collect();
            for (a, b) in before.iter().flat_map(|a| after.iter().map(move |b| (**a, **b))) {
                let Some(pair) = zones_of(&placed, a, b) else { continue };
                if let Some(rect) = gap_between(pair, d.dir) {
                    let (path, dir, dpi) = (d.path.clone(), d.dir, s.dpi);
                    out.push(Spot { screen: i, path, dir, a, b, zones: pair, rect, dpi });
                }
            }
        }
    }
    out
}

/// Put a handle on `r` with the bar along its middle, or with nothing to see.
fn paint(hwnd: HWND, r: Rect, dir: Dir, bar: bool, dpi: u32) {
    let thick = (BAR_PX * dpi as f32 / 96.0).round() as i32;
    let across = if dir == Dir::Vertical { r.w } else { r.h };
    let thick = thick.clamp(1, across.max(1));
    let (lo, hi) = ((across - thick) / 2, (across - thick) / 2 + thick);
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.w,
                biHeight: -r.h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                let old = SelectObject(dc, bitmap.into());
                let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (r.w * r.h) as usize);
                for (i, px) in pixels.iter_mut().enumerate() {
                    let (x, y) = (i as i32 % r.w, i as i32 / r.w);
                    let at = if dir == Dir::Vertical { x } else { y };
                    *px = if bar && at >= lo && at < hi { BAR } else { CLEAR };
                }
                let _ = GdiFlush();
                let (at, size) = (POINT { x: r.x, y: r.y }, SIZE { cx: r.w, cy: r.h });
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    hwnd,
                    Some(screen),
                    Some(&at),
                    Some(&size),
                    Some(dc),
                    Some(&POINT::default()),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(dc, old);
            }
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
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

    /// The main thread's latest zones. None takes every handle away.
    fn take_scene(&mut self) {
        let Some(scene) = SCENE.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
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
            unsafe {
                SetTimer(Some(self.owner), TIMER_CHECK, CHECK_MS, None);
            }
        }
        // A handle being dragged keeps its place; the handles are built when it's let go.
        if self.drag.is_none() {
            self.schedule();
        }
    }

    fn hide(&mut self) {
        self.hidden = true;
        unsafe {
            let _ = KillTimer(Some(self.owner), TIMER_BUILD);
        }
        for h in &self.handles {
            unsafe {
                let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_HIDEWINDOW;
                let _ = SetWindowPos(h.hwnd, None, 0, 0, 0, 0, flags);
            }
        }
    }

    /// No handles at all: any drag of one goes back, and the windows go.
    fn clear(&mut self) {
        if let Some(drag) = self.drag.take() {
            for m in &drag.members {
                m.back();
            }
        }
        for h in self.handles.drain(..) {
            unsafe {
                let _ = DestroyWindow(h.hwnd);
            }
        }
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
    /// again, so building them again doesn't flicker.
    fn build(&mut self) {
        if self.hidden || self.drag.is_some() {
            return;
        }
        let started = Instant::now();
        let spots = self.scene.as_ref().map(spots).unwrap_or_default();
        while self.handles.len() > spots.len() {
            if let Some(h) = self.handles.pop() {
                unsafe {
                    let _ = DestroyWindow(h.hwnd);
                }
            }
        }
        let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        for (i, spot) in spots.into_iter().enumerate() {
            if let Some(h) = self.handles.get_mut(i) {
                (h.spot, h.hovered, h.gone) = (spot, false, false);
            } else {
                let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
                let made = unsafe {
                    CreateWindowExW(
                        ex,
                        CLASS,
                        w!("Seam divider"),
                        WS_POPUP,
                        0,
                        0,
                        1,
                        1,
                        None,
                        None,
                        Some(hinst.into()),
                        None,
                    )
                };
                let Ok(hwnd) = made else { break };
                self.handles.push(Handle { hwnd, spot, hovered: false, gone: false });
            }
            let h = &self.handles[i];
            paint(h.hwnd, h.spot.rect, h.spot.dir, false, h.spot.dpi);
        }
        self.reorder();
        let ms = started.elapsed().as_secs_f64() * 1e3;
        log::info!("divider handles: {} built in {ms:.1}ms", self.handles.len());
    }

    /// Each handle just above the higher of its two windows, and shown. Above a
    /// topmost window only when its own window is one.
    fn reorder(&self) {
        if self.hidden || self.handles.is_empty() {
            return;
        }
        let order = winutil::top_level_windows();
        let ours = |w: &HWND| self.handles.iter().any(|h| h.hwnd == *w);
        for h in self.handles.iter().filter(|h| !h.gone) {
            let (a, b) = (h.spot.a.0, h.spot.b.0);
            let Some(i) = order.iter().position(|w| w.0 as isize == a || w.0 as isize == b) else { continue };
            let above = order[..i].iter().rev().find(|w| !ours(w) && !is_topmost(**w));
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW;
            unsafe {
                let _ = SetWindowPos(h.hwnd, Some(above.copied().unwrap_or(HWND_TOP)), 0, 0, 0, 0, flags);
            }
        }
    }

    /// A handle whose windows closed, were minimized or moved since hides, and the
    /// handles are built again.
    fn check(&mut self) {
        if self.hidden || self.drag.is_some() {
            return;
        }
        let mut stale = false;
        for h in self.handles.iter_mut().filter(|h| !h.gone) {
            if !still(h.spot.a.0, h.spot.zones.0) || !still(h.spot.b.0, h.spot.zones.1) {
                (stale, h.gone) = (true, true);
                unsafe {
                    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_HIDEWINDOW;
                    let _ = SetWindowPos(h.hwnd, None, 0, 0, 0, 0, flags);
                }
            }
        }
        if stale {
            self.schedule();
        }
    }

    /// Focus moved. A snapped window with no handle may be on a desktop just switched to.
    fn foreground(&mut self, fg: HWND) {
        if self.hidden || self.drag.is_some() {
            return;
        }
        let key = fg.0 as isize;
        let known = self.handles.iter().any(|h| h.spot.a.0 == key || h.spot.b.0 == key);
        if !known && winutil::has_zone_stamp(fg) {
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
        let Some(i) = self.handle(hwnd) else { return false };
        let shape = if self.handles[i].spot.dir == Dir::Vertical { IDC_SIZEWE } else { IDC_SIZENS };
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
        let h = &mut self.handles[i];
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
        let (Some(i), Some(scene), None) = (self.handle(hwnd), self.scene.as_ref(), self.drag.as_ref()) else {
            return;
        };
        let spot = &self.handles[i].spot;
        let s = &scene.screens[spot.screen];
        let placed = zones::rects(&s.layout.tree, s.work, s.gap);
        let list = wincontrol::screen_members(&s.id, s.bounds, &placed, &scene.known, &scene.exclude);
        let mins = wincontrol::group_mins(&s.layout.tree, s.work, s.gap, s.dpi, &list);
        let member =
            |(h, from, to): (HWND, usize, usize)| Member::of(h, from, to, zones::span(placed[from], placed[to]));
        let members = list.into_iter().filter_map(member).collect();
        let start = along(spot.dir, cursor());
        let edge = Edge { path: spot.path.clone() };
        for h in self.handles.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, h)| h) {
            unsafe {
                let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_HIDEWINDOW;
                let _ = SetWindowPos(h.hwnd, None, 0, 0, 0, 0, flags);
            }
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
        unsafe {
            let _ = SetCapture(hwnd);
            SetTimer(Some(self.owner), TIMER_FOLLOW, FOLLOW_MS, None);
        }
    }

    /// The pointer moved, or it's time to send the windows their latest places. Esc
    /// puts everything back.
    fn drag_to(&mut self) {
        let Some(mut drag) = self.drag.take() else { return };
        if unsafe { GetAsyncKeyState(i32::from(VK_ESCAPE.0)) } < 0 {
            self.drag = Some(drag);
            self.release(false);
            return;
        }
        let (Some(scene), Some(handle)) = (self.scene.as_ref(), self.handles.get(drag.handle)) else { return };
        let started = Instant::now();
        let s = &scene.screens[handle.spot.screen];
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
                m.follow(zones::span(placed[m.from], placed[m.to]));
            }
        }
        drag.longest = drag.longest.max(started.elapsed());
        self.drag = Some(drag);
    }

    /// The drag is over. Let go, the windows land and the layout is saved once;
    /// cut short or Esc, they go back.
    fn release(&mut self, commit: bool) {
        let Some(drag) = self.drag.take() else { return };
        unsafe {
            let _ = KillTimer(Some(self.owner), TIMER_FOLLOW);
            let _ = ReleaseCapture();
        }
        let us = drag.longest.as_secs_f64() * 1e6;
        log::info!("divider drag: {} windows, {} moves, max {us:.0}us", drag.members.len(), drag.moves);
        let screen = self.handles.get(drag.handle).map(|h| h.spot.screen);
        let target = screen.and_then(|i| Some((self.scene.as_mut()?, i)));
        let Some((scene, i)) = target.filter(|_| commit) else {
            drag.members.iter().for_each(Member::back);
            self.schedule();
            return;
        };
        let s = &mut scene.screens[i];
        let Some((tree, placed)) = drag.solved.filter(|(tree, _)| *tree != s.layout.tree) else {
            drag.members.iter().for_each(Member::back);
            self.schedule();
            return;
        };
        let mut moved = Vec::new();
        for m in drag.members.iter().filter(|m| m.here(&scene.exclude)) {
            let at = zones::span(placed[m.from], placed[m.to]);
            m.land(at);
            moved.push(Zoned { hwnd: m.hwnd, zone: (s.id.clone(), m.from, m.to), at, moving: false });
        }
        let layout = groups::relabel(s.layout.template, tree);
        // The handles are built from the new layout until the main thread's zones come.
        s.layout = layout.clone();
        wincontrol::save_moved(scene.owner, s.id.clone(), layout, moved);
        self.schedule();
    }

    /// A handle's menu: shares of the window before the divider, and Take out for each.
    fn handle_items(&self, hwnd: HWND) -> Option<(Vec<Item>, isize)> {
        let h = &self.handles[self.handle(hwnd).filter(|_| self.drag.is_none())?];
        let scene = self.scene.as_ref()?;
        let (a, b) = (h.spot.a.0, h.spot.b.0);
        let (axis, side, sides) = match h.spot.dir {
            Dir::Vertical => (Axis::Width, Side::Right, ("Left", "right", "left")),
            Dir::Horizontal => (Axis::Height, Side::Bottom, ("Top", "bottom", "top")),
        };
        // An app's name reads better than a side, unless both sides are the same app.
        let names = match (winutil::app_name(hwnd_of(a)), winutil::app_name(hwnd_of(b))) {
            (Some(x), Some(y)) if x != y => (x.clone(), x, y),
            _ => (sides.0.to_string(), format!("the {} window", sides.2), format!("the {} window", sides.1)),
        };
        let share = |(text, fraction): (&str, f32)| {
            let verb = GroupVerb::Share { axis, fraction, side: Some(side) };
            Some((format!("{} gets {text}", names.0), Choice::Share(a, verb)))
        };
        let mut items: Vec<_> = SHARES.into_iter().map(share).collect();
        items.push(None);
        items.push(Some((format!("Take out {}", names.1), Choice::TakeOut(a))));
        items.push(Some((format!("Take out {}", names.2), Choice::TakeOut(b))));
        Some((items, scene.owner))
    }
}

/// The size menu for a window in zones `from` to `to`: a share along each axis
/// where a zone lies beside it, and Take out.
fn key_items(hwnd: isize, (screen, from, to): &(String, usize, usize), scene: &Scene) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(s) = scene.screens.iter().find(|s| s.id == *screen) {
        let cells = zones::rects(&s.layout.tree, s.work, 0);
        if let (Some(a), Some(b)) = (cells.get(*from), cells.get(*to)) {
            let r = zones::span(*a, *b);
            let inside = |c: &Rect| c.x >= r.x && c.y >= r.y && c.x + c.w <= r.x + r.w && c.y + c.h <= r.y + r.h;
            let leaves: Vec<usize> = (0..cells.len()).filter(|i| inside(&cells[*i])).collect();
            let axes = [
                (Axis::Width, [Side::Left, Side::Right], "Width"),
                (Axis::Height, [Side::Top, Side::Bottom], "Height"),
            ];
            for (axis, sides, word) in axes {
                if sides.iter().all(|side| groups::edge_of(&s.layout.tree, s.work, &leaves, *side).is_none()) {
                    continue;
                }
                for (text, fraction) in SHARES {
                    let verb = GroupVerb::Share { axis, fraction, side: None };
                    items.push(Some((format!("{word}: {text}"), Choice::Share(hwnd, verb))));
                }
                items.push(None);
            }
        }
    }
    let name = winutil::app_name(hwnd_of(hwnd)).unwrap_or_else(|| "this window".into());
    items.push(Some((format!("Take out {name}"), Choice::TakeOut(hwnd))));
    items
}

/// Show a menu at `(x, y)` and hand its choice to the main thread `main`. The menu
/// takes focus while it is open, as a menu must to close on a click elsewhere, and
/// gives it back.
fn pick(items: &[Item], (x, y): (i32, i32), align: TRACK_POPUP_MENU_FLAGS, main: isize) {
    let Some(owner) = with_state(|s| s.owner) else { return };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for (i, item) in items.iter().enumerate() {
            let _ = match item {
                Some((text, _)) => {
                    let label: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                    AppendMenuW(menu, MF_STRING, i + 1, PCWSTR(label.as_ptr()))
                }
                None => AppendMenuW(menu, MF_SEPARATOR, 0, None),
            };
        }
        let before = GetForegroundWindow();
        let _ = SetForegroundWindow(owner);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | align, x, y, None, owner, None).0 as usize;
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if GetForegroundWindow() == owner && !before.is_invalid() {
            let _ = SetForegroundWindow(before);
        }
        let Some(Some((_, choice))) = id.checked_sub(1).and_then(|i| items.get(i)) else { return };
        *CHOICE.lock().unwrap_or_else(|e| e.into_inner()) = Some(*choice);
        let _ = PostMessageW(Some(hwnd_of(main)), WM_APP_DIVIDER, WPARAM(0), LPARAM(0));
    }
}

/// Right-click on a handle.
fn handle_menu(hwnd: HWND) {
    let Some(Some((items, main))) = with_state(|s| s.handle_items(hwnd)) else { return };
    let pt = cursor();
    pick(&items, (pt.x, pt.y), TPM_LEFTALIGN, main);
}

/// The size menu key, at the window's centre.
fn asked_menu() {
    let Some((hwnd, zone, scene)) = ASKED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let Some(r) = winutil::visible_rect(hwnd_of(hwnd)) else { return };
    let items = key_items(hwnd, &zone, &scene);
    pick(&items, (r.x + r.w / 2, r.y + r.h / 2), TPM_CENTERALIGN | TPM_VCENTERALIGN, scene.owner);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_SCENE => {
            with_state(State::take_scene);
        }
        WM_APP_HIDE => {
            with_state(State::hide);
        }
        WM_APP_MENU => asked_menu(),
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
        WM_RBUTTONUP => handle_menu(hwnd),
        WM_CLOSE if with_state(|s| s.owner == hwnd) == Some(true) => {
            with_state(State::clear);
            let _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
        }
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    LRESULT(0)
}

fn run(ready: Sender<isize>) {
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
        let owner = CreateWindowExW(
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
        let Ok(owner) = owner else {
            let _ = ready.send(0);
            return;
        };
        STATE.with(|s| {
            *s.borrow_mut() =
                Some(State { owner, scene: None, handles: Vec::new(), hidden: false, hook: None, drag: None })
        });
        let _ = ready.send(owner.0 as isize);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
