//! Quick layout: a small panel on the screen under the pointer for switching that
//! screen's zone layout from the keyboard. It shows the layouts as numbered
//! thumbnails, takes focus, and hands each pick to the tray, which applies it as a
//! click on the Zones page does. It closes on Esc, on a click outside, or when it
//! loses focus.

use crate::geom::Rect;
use crate::log;
use crate::showlayout::{fill, text, Color};
use crate::wincontrol;
use seam::library::Entry;
use seam::zones::{self, Node, Template, ZoneLayout};
use serde_json::Value;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A pick is waiting for the tray, or the tray's finished with one: `take_picks` and `take_done` have them.
pub const WM_APP_QUICK_LAYOUT: u32 = WM_APP + 29;
/// To the overlay's thread: `SETTLED` has what the tray found.
const WM_APP_SETTLED: u32 = WM_APP + 30;
/// Gives up on a request the tray never reported back on.
const TIMER_SETTLE: usize = 1;
const SETTLE_MS: u32 = 3000;
/// Closes the overlay if Windows kept the focus elsewhere.
const TIMER_FOCUS: usize = 2;
const FOCUS_GRACE_MS: u32 = 300;
/// One more try at the focus, then the last look.
const TIMER_FOCUS_LAST: usize = 3;
const FOCUS_LAST_MS: u32 = 500;

const CLASS: PCWSTR = w!("SeamQuickLayout");
/// Thumbnails shown at once, three to a row.
const VISIBLE: usize = 9;
const COLUMNS: usize = 3;

// Sizes at 96 dpi.
const PAD: i32 = 24;
const CELL: (i32, i32) = (168, 128);
const GAP: i32 = 12;
const HEAD: i32 = 32;
const NOTE: i32 = 24;
const FOOT: i32 = 20;
const ART: (i32, i32) = (152, 86);

const PAGE: Color = (0x20, 0x20, 0x20);
const CARD: Color = (0x2A, 0x2A, 0x2A);
const RAISED: Color = (0x33, 0x33, 0x33);
const HAIRLINE: Color = (0x3B, 0x3B, 0x3B);
const SEGMENT: Color = (0x4A, 0x4A, 0x4A);
const TEXT: Color = (0xF3, 0xF3, 0xF3);
const SECONDARY: Color = (0xB0, 0xB0, 0xB0);
const MUTED: Color = (0x94, 0x94, 0x94);
const ACCENT: Color = (0xFF, 0xB0, 0x20);
/// The accent at 40% over the page, for the halo round the layout in use.
const HALO: Color = (0x79, 0x5A, 0x20);

/// One layout to pick, with the tree to draw it by.
pub struct Choice {
    pub title: String,
    /// None is the choice of no zones.
    pub layout: Option<ZoneLayout>,
}

/// What the overlay needs to open.
pub struct Spec {
    /// Line Up number of the screen.
    pub number: u32,
    /// The screen's device path.
    pub screen: String,
    pub rect: Rect,
    /// A line above the thumbnails: why picking won't move windows.
    pub note: Option<String>,
    pub choices: Vec<Choice>,
    /// The layout the screen has now.
    pub now: Option<ZoneLayout>,
    /// The tray's window, which hears of each pick.
    pub tray: isize,
    /// Opened from the API, which has to earn focus.
    pub from_api: bool,
}

struct Overlay {
    number: u32,
    /// The screen's width and height, for the thumbnails' shape.
    shape: (i32, i32),
    note: Option<String>,
    choices: Vec<Choice>,
    now: Option<ZoneLayout>,
    /// The layouts before each pick made here, for Ctrl+Z.
    history: Vec<Option<ZoneLayout>>,
    selected: usize,
    first: usize,
    tray: isize,
    /// It has been the active window, so losing that closes it.
    active: bool,
    /// The screen's device path.
    screen: String,
    /// Which overlay this is, so a reply meant for an earlier one is ignored.
    generation: u32,
    /// The number of the last request sent to the tray.
    seq: u32,
    /// A request is with the tray, which hasn't said what it came to.
    pending: bool,
    /// The layout to keep for Ctrl+Z if the pending request changed the screen.
    record: Option<Option<ZoneLayout>>,
    /// The pending request is an undo, so the cursor follows the layout it restores.
    undoing: bool,
    /// The latest request made while another was pending.
    queued: Option<Request>,
}

#[derive(Clone, Copy)]
enum Request {
    Pick(usize),
    Undo,
}

thread_local! {
    static STATE: RefCell<Option<Overlay>> = const { RefCell::new(None) };
    static WINDOW: RefCell<HWND> = const { RefCell::new(HWND(std::ptr::null_mut())) };
}

/// The id of the thread showing the overlay, or 0.
static OPEN: AtomicU32 = AtomicU32::new(0);

/// One thread starts an overlay at a time.
static LAUNCH: Mutex<()> = Mutex::new(());

/// The window that had focus before the overlay opened, which a pick treats as the focused one.
static BEFORE: AtomicIsize = AtomicIsize::new(0);

/// Which overlay and which of its requests: the generation and the sequence number.
pub type Stamp = (u32, u32);

/// A pick for the tray: the screen's device path and the layout, None for no zones.
pub struct Pick {
    pub screen: String,
    pub layout: Option<Value>,
    pub stamp: Stamp,
}

/// Counts the overlays opened.
static GENERATION: AtomicU32 = AtomicU32::new(0);

/// The overlay is on its way out, but its thread hasn't ended yet.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Picks for the tray, in order.
static PICKS: Mutex<Vec<Pick>> = Mutex::new(Vec::new());

/// Screens whose last pick the tray has finished with, one way or another.
static DONE: Mutex<Vec<(String, Stamp)>> = Mutex::new(Vec::new());

/// What the tray found on a screen once its pick was finished: the screen's device path and its layout.
static SETTLED: Mutex<Vec<(String, Stamp, Option<ZoneLayout>)>> = Mutex::new(Vec::new());

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the overlay is up, so the tray's own Undo notice can stay quiet.
pub fn is_open() -> bool {
    OPEN.load(Ordering::SeqCst) != 0
}

/// Close the overlay if it's up. Returns whether it was.
pub fn close() -> bool {
    let open = OPEN.swap(0, Ordering::SeqCst);
    if open != 0 {
        unsafe {
            let _ = PostThreadMessageW(open, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    open != 0
}

/// For the key: close the overlay if it's up, and say so. One already on its way
/// out is waited for a moment, and then the key opens a new one.
pub fn close_for_key() -> bool {
    if !is_open() {
        return false;
    }
    if CLOSING.load(Ordering::SeqCst) {
        for _ in 0..20 {
            if !is_open() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        return false;
    }
    close()
}

/// End the overlay's message loop.
fn quit() {
    CLOSING.store(true, Ordering::SeqCst);
    unsafe { PostQuitMessage(0) };
}

pub fn remember_focus(h: HWND) {
    BEFORE.store(h.0 as isize, Ordering::SeqCst);
}

/// The window that had focus when the overlay opened, or 0.
pub fn focus_before() -> isize {
    BEFORE.load(Ordering::SeqCst)
}

pub fn take_picks() -> Vec<Pick> {
    std::mem::take(&mut *locked(&PICKS))
}

/// The tray finished with a pick on this screen. It tells the overlay what the screen has.
pub fn done(screen: String, stamp: Stamp, tray: isize) {
    locked(&DONE).push((screen, stamp));
    unsafe {
        let _ = PostMessageW(Some(HWND(tray as *mut _)), WM_APP_QUICK_LAYOUT, WPARAM(0), LPARAM(0));
    }
}

pub fn take_done() -> Vec<(String, Stamp)> {
    std::mem::take(&mut *locked(&DONE))
}

/// Tell the overlay what a screen's layout is now.
pub fn settled(screen: &str, stamp: Stamp, actual: Option<ZoneLayout>) {
    let open = OPEN.load(Ordering::SeqCst);
    if open != 0 {
        locked(&SETTLED).push((screen.to_string(), stamp, actual));
        unsafe {
            let _ = PostThreadMessageW(open, WM_APP_SETTLED, WPARAM(0), LPARAM(0));
        }
    }
}

/// The layouts offered on a screen: the built-in templates, then the custom ones
/// for it (and the one it uses, wherever that's tied), then None.
pub fn choices(library: &[Entry], device: &str, now: Option<&ZoneLayout>) -> Vec<Choice> {
    let in_use = now.and_then(|l| l.custom.as_deref());
    let built_in = Template::ALL
        .into_iter()
        .filter_map(ZoneLayout::of)
        .map(|layout| Choice { title: layout.template.title().to_string(), layout: Some(layout) });
    let custom = library
        .iter()
        .filter(|e| e.fits(device) || in_use == Some(e.id.as_str()))
        .map(|e| Choice { title: e.name.clone(), layout: Some(e.layout()) });
    let none = Choice { title: Template::None.title().to_string(), layout: None };
    built_in.chain(custom).chain([none]).collect()
}

/// Which choice is this layout, as the Zones page marks it: by library id, else by template.
fn index_of(choices: &[Choice], layout: Option<&ZoneLayout>) -> Option<usize> {
    let Some(layout) = layout else { return choices.iter().position(|c| c.layout.is_none()) };
    let by_id = layout
        .custom
        .as_ref()
        .and_then(|id| choices.iter().position(|c| c.layout.as_ref().is_some_and(|l| l.custom.as_ref() == Some(id))));
    by_id.or_else(|| {
        choices
            .iter()
            .position(|c| c.layout.as_ref().is_some_and(|l| l.custom.is_none() && l.template == layout.template))
    })
}

impl Overlay {
    fn current(&self) -> Option<usize> {
        index_of(&self.choices, self.now.as_ref())
    }

    fn shown(&self) -> usize {
        self.choices.len().min(VISIBLE)
    }

    /// Keep the selected thumbnail on screen, scrolling a row at a time.
    fn reveal(&mut self) {
        let row = self.selected / COLUMNS * COLUMNS;
        if self.selected < self.first {
            self.first = row;
        } else if self.selected >= self.first + VISIBLE {
            self.first = (row + COLUMNS).saturating_sub(VISIBLE);
        }
    }

    fn step(&mut self, by: isize) {
        let last = self.choices.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + by).clamp(0, last) as usize;
        self.reveal();
    }

    /// Give the screen the layout at `at`, unless it has it. While the tray is
    /// still busy with the last one, the latest request waits for it.
    fn pick(&mut self, at: usize) {
        if at >= self.choices.len() {
            return;
        }
        self.selected = at;
        self.reveal();
        self.request(Request::Pick(at));
    }

    /// Put back the layout from before the last pick made here.
    fn undo(&mut self) {
        self.request(Request::Undo);
    }

    fn request(&mut self, request: Request) {
        if self.pending {
            self.queued = Some(request);
        } else {
            self.dispatch(request);
        }
    }

    fn dispatch(&mut self, request: Request) {
        let layout = match request {
            Request::Pick(at) => {
                if self.current() == Some(at) {
                    return;
                }
                self.record = Some(self.now.clone());
                self.choices[at].layout.clone()
            }
            Request::Undo => {
                let Some(before) = self.history.pop() else { return };
                self.record = None;
                self.undoing = true;
                before
            }
        };
        self.pending = true;
        self.seq += 1;
        let value = layout.and_then(|l| serde_json::to_value(l).ok());
        let stamp = (self.generation, self.seq);
        locked(&PICKS).push(Pick { screen: self.screen.clone(), layout: value, stamp });
        unsafe {
            let _ = PostMessageW(Some(HWND(self.tray as *mut _)), WM_APP_QUICK_LAYOUT, WPARAM(0), LPARAM(0));
            let _ = SetTimer(Some(WINDOW.with(|w| *w.borrow())), TIMER_SETTLE, SETTLE_MS, None);
        }
    }

    /// The tray never answered: stop waiting for it. Its answer, if it comes,
    /// is then too late to count.
    fn give_up(&mut self) {
        unsafe {
            let _ = KillTimer(Some(WINDOW.with(|w| *w.borrow())), TIMER_SETTLE);
        }
        self.pending = false;
        self.record = None;
        self.undoing = false;
        if let Some(next) = self.queued.take() {
            self.dispatch(next);
        }
    }

    /// The tray finished with the last request and says what the screen has. That
    /// is what the marker and Ctrl+Z go by, whatever the request came to.
    fn settle(&mut self, (generation, seq): Stamp, actual: Option<ZoneLayout>) {
        if generation != self.generation {
            return;
        }
        if !self.pending {
            // A late answer with nothing newer waiting: the marker may follow it, the history doesn't.
            self.now = actual;
            return;
        }
        if seq != self.seq {
            return;
        }
        unsafe {
            let _ = KillTimer(Some(WINDOW.with(|w| *w.borrow())), TIMER_SETTLE);
        }
        self.pending = false;
        if let Some(before) = self.record.take() {
            if before != actual {
                self.history.push(before);
            }
        }
        self.now = actual;
        if std::mem::take(&mut self.undoing) {
            if let Some(at) = self.current() {
                self.selected = at;
                self.reveal();
            }
        }
        if let Some(next) = self.queued.take() {
            self.dispatch(next);
        }
    }
}

/// The panel's size and the grid's top, in pixels.
fn frame(o: &Overlay, t: &dyn Fn(i32) -> i32) -> (i32, i32, i32) {
    let rows = o.shown().div_ceil(COLUMNS) as i32;
    let note = if o.note.is_some() { NOTE } else { 0 };
    let top = t(PAD + HEAD + note);
    let width = t(PAD * 2 + COLUMNS as i32 * CELL.0 + (COLUMNS as i32 - 1) * GAP);
    let height = top + t(rows * CELL.1 + (rows - 1) * GAP + 12 + FOOT + PAD);
    (width, height, top)
}

/// Where the thumbnail at `slot` (0 to 8) sits.
fn cell(slot: usize, top: i32, t: &dyn Fn(i32) -> i32) -> RECT {
    let (col, row) = ((slot % COLUMNS) as i32, (slot / COLUMNS) as i32);
    let left = t(PAD + col * (CELL.0 + GAP));
    let y = top + t(row * (CELL.1 + GAP));
    RECT { left, top: y, right: left + t(CELL.0), bottom: y + t(CELL.1) }
}

unsafe fn outline(dc: HDC, r: RECT, width: i32, c: Color) {
    fill(dc, RECT { bottom: r.top + width, ..r }, c);
    fill(dc, RECT { top: r.bottom - width, ..r }, c);
    fill(dc, RECT { right: r.left + width, ..r }, c);
    fill(dc, RECT { left: r.right - width, ..r }, c);
}

fn grown(r: RECT, by: i32) -> RECT {
    RECT { left: r.left - by, top: r.top - by, right: r.right + by, bottom: r.bottom + by }
}

/// The zones of a layout inside a box, the box being the screen's shape.
fn art(tree: Option<&Node>, shape: (i32, i32), at: RECT, t: &dyn Fn(i32) -> i32) -> (RECT, Vec<RECT>) {
    let (bw, bh) = (at.right - at.left, at.bottom - at.top);
    let (sw, sh) = (i64::from(shape.0.max(1)), i64::from(shape.1.max(1)));
    // The largest box of the screen's shape that fits.
    let (w, h) = if i64::from(bw) * sh <= i64::from(bh) * sw {
        (bw, (i64::from(bw) * sh / sw) as i32)
    } else {
        ((i64::from(bh) * sw / sh) as i32, bh)
    };
    let (x, y) = (at.left + (bw - w) / 2, at.top + (bh - h) / 2);
    let face = RECT { left: x, top: y, right: x + w, bottom: y + h };
    let pieces = tree
        .map(|tree| {
            zones::rects(tree, zones::Rect { x, y, w, h }, t(2))
                .into_iter()
                .map(|r| RECT { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h })
                .collect()
        })
        .unwrap_or_default();
    (face, pieces)
}

unsafe fn draw(dc: HDC, o: &Overlay, client: RECT, scale: f64) {
    let t = |v: i32| (v as f64 * scale).round() as i32;
    fill(dc, client, PAGE);
    outline(dc, client, 1, HAIRLINE);
    let (width, _, top) = frame(o, &t);
    let inner = |from: i32, to: i32, y: i32, h: i32| RECT { left: from, top: y, right: to, bottom: y + h };
    let (left, right) = (t(PAD), width - t(PAD));
    let title = format!("Layout for screen {}", o.number);
    text(dc, &title, inner(left, right, t(PAD), t(HEAD)), t(16), TEXT, DT_LEFT | DT_VCENTER);
    if o.choices.len() > VISIBLE {
        let last = (o.first + o.shown()).min(o.choices.len());
        let range = format!("{} to {} of {}", o.first + 1, last, o.choices.len());
        text(dc, &range, inner(left, right, t(PAD), t(HEAD)), t(12), MUTED, DT_RIGHT | DT_VCENTER);
    }
    if let Some(note) = &o.note {
        text(
            dc,
            note,
            inner(left, right, t(PAD + HEAD), t(NOTE)),
            t(13),
            SECONDARY,
            DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS,
        );
    }
    let current = o.current();
    for slot in 0..o.shown() {
        let at = o.first + slot;
        let choice = &o.choices[at];
        let r = cell(slot, top, &t);
        if current == Some(at) {
            outline(dc, grown(r, t(2)), t(1), HALO);
        }
        if o.selected == at {
            outline(dc, grown(r, t(5)), t(2), TEXT);
        }
        fill(dc, r, if o.selected == at { RAISED } else { CARD });
        outline(dc, r, t(1), if current == Some(at) { ACCENT } else { HAIRLINE });
        let box_ = RECT { left: r.left + t(8), top: r.top + t(8), right: r.right - t(8), bottom: r.top + t(8 + ART.1) };
        let tree = choice.layout.as_ref().map(|l| &l.tree);
        let (face, pieces) = art(tree, o.shape, box_, &t);
        fill(dc, face, PAGE);
        for piece in pieces {
            fill(dc, piece, SEGMENT);
        }
        let badge = RECT { left: r.left + t(8), top: r.top + t(8), right: r.left + t(28), bottom: r.top + t(28) };
        fill(dc, badge, CARD);
        text(dc, &(slot + 1).to_string(), badge, t(12), TEXT, DT_CENTER | DT_VCENTER);
        let name_top = r.top + t(8 + ART.1 + 8);
        let name = RECT { left: r.left + t(8), top: name_top, right: r.right - t(8), bottom: name_top + t(18) };
        let colour = if current == Some(at) { TEXT } else { SECONDARY };
        if current == Some(at) {
            text(dc, "In use", name, t(12), MUTED, DT_RIGHT | DT_VCENTER);
            let room = RECT { right: name.right - t(52), ..name };
            text(dc, &choice.title, room, t(13), colour, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
        } else {
            text(dc, &choice.title, name, t(13), colour, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
        }
    }
    let hint = "1 to 9 pick, arrows and Enter, Ctrl+Z undo, Esc close";
    let foot_top = client.bottom - t(PAD + FOOT);
    text(dc, hint, inner(left, right, foot_top, t(FOOT)), t(12), MUTED, DT_LEFT | DT_VCENTER);
}

fn scale_of(hwnd: HWND) -> f64 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f64 / 96.0
}

/// The thumbnail under a point in the window's own pixels.
fn slot_at(o: &Overlay, hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let scale = scale_of(hwnd);
    let t = |v: i32| (v as f64 * scale).round() as i32;
    let (_, _, top) = frame(o, &t);
    (0..o.shown()).find(|slot| {
        let r = cell(*slot, top, &t);
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
}

fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn key(o: &mut Overlay, vk: u32) -> bool {
    let digit = |vk: u32| match vk {
        0x31..=0x39 => Some(vk as usize - 0x31),
        0x61..=0x69 => Some(vk as usize - 0x61),
        _ => None,
    };
    match vk {
        v if v == VK_ESCAPE.0 as u32 => quit(),
        v if v == VK_LEFT.0 as u32 => o.step(-1),
        v if v == VK_RIGHT.0 as u32 => o.step(1),
        v if v == VK_UP.0 as u32 => o.step(-(COLUMNS as isize)),
        v if v == VK_DOWN.0 as u32 => o.step(COLUMNS as isize),
        v if v == VK_RETURN.0 as u32 => o.pick(o.selected),
        v if v == u32::from(b'Z') && ctrl_down() => o.undo(),
        v => match digit(v) {
            Some(n) if o.first + n < o.choices.len() => o.pick(o.first + n),
            _ => return false,
        },
    }
    true
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let redraw = |changed: bool| {
        if changed {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    };
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            // Drawn off screen and copied over, so a change doesn't flicker.
            let back = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, client.right, client.bottom);
            let old = SelectObject(back, bitmap.into());
            STATE.with(|s| {
                if let Some(o) = s.borrow().as_ref() {
                    draw(back, o, client, scale_of(hwnd));
                }
            });
            let _ = BitBlt(dc, 0, 0, client.right, client.bottom, Some(back), 0, 0, SRCCOPY);
            SelectObject(back, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(back);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let used = STATE.with(|s| s.borrow_mut().as_mut().is_some_and(|o| key(o, wparam.0 as u32)));
            redraw(used);
            if used {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
            let moved = STATE.with(|s| {
                let mut s = s.borrow_mut();
                let Some(o) = s.as_mut() else { return false };
                match slot_at(o, hwnd, x, y).map(|slot| o.first + slot) {
                    Some(at) if at != o.selected => {
                        o.selected = at;
                        true
                    }
                    _ => false,
                }
            });
            redraw(moved);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
            STATE.with(|s| {
                if let Some(o) = s.borrow_mut().as_mut() {
                    if let Some(slot) = slot_at(o, hwnd, x, y) {
                        o.pick(o.first + slot);
                    }
                }
            });
            redraw(true);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            quit();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_FOCUS => {
            let _ = KillTimer(Some(hwnd), TIMER_FOCUS);
            if GetForegroundWindow() != hwnd {
                let _ = SetForegroundWindow(hwnd);
                let _ = SetTimer(Some(hwnd), TIMER_FOCUS_LAST, FOCUS_LAST_MS, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_FOCUS_LAST => {
            let _ = KillTimer(Some(hwnd), TIMER_FOCUS_LAST);
            if GetForegroundWindow() != hwnd {
                log::info!("quick layout: Windows kept the focus elsewhere, so it closes");
                quit();
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_SETTLE => {
            STATE.with(|s| {
                if let Some(o) = s.borrow_mut().as_mut() {
                    o.give_up();
                }
            });
            redraw(true);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let inactive = (wparam.0 & 0xffff) as u32 == WA_INACTIVE;
            let was_active = STATE.with(|s| {
                let mut s = s.borrow_mut();
                let Some(o) = s.as_mut() else { return false };
                let was = o.active;
                o.active |= !inactive;
                was
            });
            if inactive && was_active {
                quit();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// A click outside the panel closes it, and goes on to what it was for.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
        let at = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
        let hwnd = WINDOW.with(|w| *w.borrow());
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_ok()
            && !(at.x >= r.left && at.x < r.right && at.y >= r.top && at.y < r.bottom)
        {
            quit();
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Clears `OPEN` when the overlay's thread ends, however it ends.
struct Closing(u32);

impl Drop for Closing {
    fn drop(&mut self) {
        let _ = OPEN.compare_exchange(self.0, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// What the tray found: take it into the overlay.
fn take_settled() {
    for (screen, stamp, actual) in std::mem::take(&mut *locked(&SETTLED)) {
        STATE.with(|s| {
            if let Some(o) = s.borrow_mut().as_mut().filter(|o| o.screen == screen) {
                o.settle(stamp, actual);
            }
        });
    }
    let hwnd = WINDOW.with(|w| *w.borrow());
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn run(spec: Spec, generation: u32, ready: std::sync::mpsc::Sender<()>) {
    // Whatever an earlier overlay was left with isn't this one's.
    locked(&SETTLED).clear();
    locked(&DONE).clear();
    unsafe {
        let id = GetCurrentThreadId();
        let _closing = Closing(id);
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let created = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Seam quick layout"),
            WS_POPUP,
            spec.rect.left,
            spec.rect.top,
            1,
            1,
            None,
            None,
            Some(hinst.into()),
            None,
        );
        let Ok(hwnd) = created else {
            log::info!("quick layout: couldn't make the window");
            return;
        };
        let current = index_of(&spec.choices, spec.now.as_ref());
        let selected = current.unwrap_or(0);
        let mut overlay = Overlay {
            number: spec.number,
            shape: (spec.rect.width(), spec.rect.height()),
            note: spec.note,
            choices: spec.choices,
            now: spec.now,
            history: Vec::new(),
            selected,
            first: 0,
            tray: spec.tray,
            active: false,
            screen: spec.screen,
            generation,
            seq: 0,
            pending: false,
            record: None,
            undoing: false,
            queued: None,
        };
        overlay.reveal();
        // Sized after creation, because the window now knows its screen's DPI.
        let scale = scale_of(hwnd);
        let t = |v: i32| (v as f64 * scale).round() as i32;
        let (width, height, _) = frame(&overlay, &t);
        let x = spec.rect.left + (spec.rect.width() - width) / 2;
        let y = spec.rect.top + (spec.rect.height() - height) / 2;
        STATE.with(|s| *s.borrow_mut() = Some(overlay));
        WINDOW.with(|w| *w.borrow_mut() = hwnd);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_SHOWWINDOW);
        // The tray can use the overlay now, so it hears of it before focus is asked for.
        CLOSING.store(false, Ordering::SeqCst);
        OPEN.store(id, Ordering::SeqCst);
        let _ = ready.send(());
        wincontrol::focus(hwnd, spec.from_api);
        let _ = SetTimer(Some(hwnd), TIMER_FOCUS, FOCUS_GRACE_MS, None);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).ok();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_APP_SETTLED {
                take_settled();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(hook) = hook {
            let _ = UnhookWindowsHookEx(hook);
        }
        let _ = DestroyWindow(hwnd);
    }
}

/// Open the overlay, replacing one that's up. It runs on a thread of its own, and
/// this returns once that thread has its window.
pub fn show(spec: Spec) {
    let _launching = locked(&LAUNCH);
    close();
    let (ready, opened) = channel();
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned =
        std::thread::Builder::new().name("seam-quicklayout".into()).spawn(move || run(spec, generation, ready));
    if let Err(e) = spawned {
        log::info!("quick layout: couldn't start: {e}");
        return;
    }
    let _ = opened.recv_timeout(Duration::from_millis(500));
}
