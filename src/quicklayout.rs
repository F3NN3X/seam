//! Quick layout: a small panel on the screen under the pointer for switching that
//! screen's zone layout from the keyboard. It shows the layouts as numbered
//! thumbnails, takes focus, and hands each pick to the tray, which applies it as a
//! click on the Zones page does. It closes on Esc, on a click outside, or when it
//! loses focus.

use crate::geom::Rect;
use crate::log;
use crate::showlayout::{Color, fill, text};
use crate::wincontrol;
use seam::library::Entry;
use seam::zones::{self, Node, Template, ZoneLayout};
use serde_json::Value;
use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

mod draw;
mod window;

use draw::{cell, draw, frame, scale_of};
pub use window::show;

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

/// The thread of the overlay on its way out, whose thread hasn't ended yet, or 0.
/// It only counts while it's still `OPEN`, so an old overlay going away late can't
/// mark a newer one as closing.
static CLOSING: AtomicU32 = AtomicU32::new(0);

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
    let open = OPEN.load(Ordering::SeqCst);
    if open == 0 {
        return false;
    }
    if CLOSING.load(Ordering::SeqCst) == open {
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
    unsafe {
        CLOSING.store(GetCurrentThreadId(), Ordering::SeqCst);
        PostQuitMessage(0);
    }
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
        if let Some(before) = self.record.take()
            && before != actual
        {
            self.history.push(before);
        }
        self.now = actual;
        if std::mem::take(&mut self.undoing)
            && let Some(at) = self.current()
        {
            self.selected = at;
            self.reveal();
        }
        if let Some(next) = self.queued.take() {
            self.dispatch(next);
        }
    }
}
