//! The overview: every window on every screen and virtual desktop, full screen, on
//! a key. Each screen gets a window of its own showing the windows on it, grouped by
//! desktop, with a live preview (a DWM thumbnail) or the app's icon. Typing searches
//! by title or app, the arrows and Enter go to a window, a drag puts one in a zone or
//! on another screen, and Esc closes it.
//!
//! It all lives on a thread of its own, with its own message loop, so the tray's
//! thread only starts it and takes the moves it hands back, which go through the
//! snapping as a placement by hand. Its windows are tool windows of Seam's own
//! process, so they never show in Alt+Tab or in their own list, and the foreground
//! hooks, which skip Seam's process, never count them as a focus change.
//!
//! A window on another virtual desktop can only be gone to, which lets Windows switch
//! desktops. Windows' public desktop interface moves only the calling process's own
//! windows, so the overview offers no move to another desktop (D572).

use crate::geom;
use crate::log;
use crate::showlayout::{Color, fill, text};
use crate::wincontrol;
use seam::overview::{self, Dir, Landing};
use seam::winutil;
use seam::zones::Rect;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

mod draw;
mod list;
mod window;

pub use window::show;

/// Moves are waiting for the tray: `take_moves` has them.
pub const WM_APP_OVERVIEW: u32 = WM_APP + 39;
/// To the overview's own window: read the slower details, app names and icons, after the first paint.
const WM_APP_DETAILS: u32 = WM_APP + 1;
/// Closes the overview if Windows kept the focus elsewhere.
const TIMER_FOCUS: usize = 1;
const FOCUS_GRACE_MS: u32 = 300;
const TIMER_FOCUS_LAST: usize = 2;
const FOCUS_LAST_MS: u32 = 500;

const CLASS: PCWSTR = w!("SeamOverview");

// Sizes at 96 dpi.
const PAD: i32 = 32;
const HEAD: i32 = 40;
const SEARCH_W: i32 = 480;
const FOOT: i32 = 20;
const GAP: i32 = 16;
const LABEL: i32 = 24;
const CAPTION: i32 = 32;
const TILE_MAX: i32 = 360;
const INSET: i32 = 8;
const ICON: i32 = 16;
const BIG_ICON: i32 = 48;

/// The overview's colours: the panel's dark tokens, or Windows' own in high contrast.
#[derive(Clone, Copy)]
struct Colours {
    page: Color,
    card: Color,
    raised: Color,
    hairline: Color,
    text: Color,
    secondary: Color,
    muted: Color,
    /// The zone a drop lands in: the seam amber, or the system highlight in high contrast.
    accent: Color,
    /// That zone's fill under its outline.
    lit: Color,
    /// The keyboard focus ring.
    ring: Color,
}

const DARK: Colours = Colours {
    page: (0x20, 0x20, 0x20),
    card: (0x2A, 0x2A, 0x2A),
    raised: (0x33, 0x33, 0x33),
    hairline: (0x3B, 0x3B, 0x3B),
    text: (0xF3, 0xF3, 0xF3),
    secondary: (0xB0, 0xB0, 0xB0),
    muted: (0x94, 0x94, 0x94),
    accent: (0xFF, 0xB0, 0x20),
    // The accent at 40% over the page, as quick layout's halo.
    lit: (0x79, 0x5A, 0x20),
    ring: (0xF3, 0xF3, 0xF3),
};

fn sys(index: SYS_COLOR_INDEX) -> Color {
    let c = unsafe { GetSysColor(index) };
    ((c & 0xFF) as u8, (c >> 8 & 0xFF) as u8, (c >> 16 & 0xFF) as u8)
}

/// Windows' colours while high contrast is on, else the dark tokens.
fn colours() -> Colours {
    let mut hc = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if read.is_err() || !hc.dwFlags.contains(HCF_HIGHCONTRASTON) {
        return DARK;
    }
    let (window, text) = (sys(COLOR_WINDOW), sys(COLOR_WINDOWTEXT));
    Colours {
        page: window,
        card: window,
        raised: window,
        hairline: text,
        text,
        secondary: text,
        muted: sys(COLOR_GRAYTEXT),
        accent: sys(COLOR_HIGHLIGHT),
        lit: window,
        ring: sys(COLOR_HIGHLIGHT),
    }
}

/// One screen as the tray knows it.
pub struct Screen {
    /// Its Line Up number.
    pub number: u32,
    /// Its device path.
    pub id: String,
    pub rect: geom::Rect,
    /// Its zones with the gap, empty when it has none or zones are off.
    pub zones: Vec<Rect>,
    /// Smart tiling arranges it, so a drop there floats.
    pub tiled: bool,
}

/// What the overview needs to open.
pub struct Spec {
    pub screens: Vec<Screen>,
    /// The tray's window, which hears of each move.
    pub tray: isize,
    /// The window in front when it was asked for.
    pub before: isize,
    /// Opened from the API, which has to earn focus.
    pub from_api: bool,
    /// When the key was pressed, for the log's first-paint time.
    pub pressed: Instant,
}

/// A window the overview dropped, for the snapping: in zone `zone` of the screen
/// `screen`, or at `free` there when it has no zones.
pub struct Move {
    pub hwnd: isize,
    pub screen: String,
    pub zone: Option<usize>,
    pub free: Rect,
}

/// A window in the overview.
struct Win {
    hwnd: HWND,
    title: String,
    /// The app's name: the exe's file name at first, its description once read.
    app: String,
    exe: String,
    /// The screen it is on, by index.
    screen: usize,
    /// Its virtual desktop, by index into `State::desktops`, 0 being the one in front.
    desktop: usize,
    minimized: bool,
    /// Its visible frame, or where it restores to when minimized.
    frame: Rect,
    /// The app's icon, owned by the app, or none.
    icon: Option<HICON>,
    /// Its live preview, 0 for none, and the size it comes in.
    thumb: isize,
    source: (i32, i32),
}

/// One screen's window and what it shows.
struct Pane {
    hwnd: HWND,
    number: u32,
    id: String,
    /// The whole screen, in screen pixels.
    rect: Rect,
    /// Its work area and DPI.
    work: Rect,
    dpi: u32,
    zones: Vec<Rect>,
    tiled: bool,
    /// Each desktop's label row on this screen, in screen pixels.
    labels: Vec<(usize, Rect)>,
    /// Each window shown on this screen and its tile, in screen pixels.
    tiles: Vec<(usize, Rect)>,
}

impl Pane {
    fn scale(&self) -> f64 {
        self.dpi.max(96) as f64 / 96.0
    }

    fn t(&self, v: i32) -> i32 {
        (v as f64 * self.scale()).round() as i32
    }

    /// A rect in screen pixels as this window's own.
    fn local(&self, r: Rect) -> RECT {
        let (x, y) = (r.x - self.rect.x, r.y - self.rect.y);
        RECT { left: x, top: y, right: x + r.w, bottom: y + r.h }
    }
}

struct Press {
    win: usize,
    at: (i32, i32),
}

struct Drag {
    win: usize,
    at: (i32, i32),
    over: Option<Landing>,
}

struct State {
    panes: Vec<Pane>,
    wins: Vec<Win>,
    /// Each desktop's label, the one in front first.
    desktops: Vec<String>,
    query: String,
    /// The windows the search keeps, best first.
    shown: Vec<usize>,
    selected: Option<usize>,
    before: isize,
    tray: isize,
    colours: Colours,
    press: Option<Press>,
    drag: Option<Drag>,
    /// The last place the pointer was seen, so a window shown under a still pointer doesn't take the selection.
    pointer: Option<(i32, i32)>,
    /// It has been the active app, so losing that closes it.
    active: bool,
    pressed: Instant,
    /// Panes painted so far, for the first-paint log line.
    painted: usize,
    listed_ms: u128,
}

impl State {
    /// Every tile on every screen in order, for the arrows.
    fn tiles(&self) -> Vec<(usize, Rect)> {
        self.panes.iter().flat_map(|p| p.tiles.iter().copied()).collect()
    }

    fn tile_at(&self, at: (i32, i32)) -> Option<usize> {
        self.tiles().into_iter().find(|(_, r)| r.contains(at.0, at.1)).map(|(w, _)| w)
    }

    fn search(&mut self) {
        let keys: Vec<(&str, &str)> = self.wins.iter().map(|w| (w.title.as_str(), w.app.as_str())).collect();
        self.shown = overview::rank(&self.query, &keys);
        draw::lay_out(self);
        let order = self.tiles();
        if !self.selected.is_some_and(|s| order.iter().any(|(w, _)| *w == s)) {
            self.selected = order.first().map(|(w, _)| *w);
        }
        draw::sync_thumbs(self);
    }

    fn arrow(&mut self, dir: Dir) {
        let order = self.tiles();
        let rects: Vec<Rect> = order.iter().map(|(_, r)| *r).collect();
        let Some(from) = order.iter().position(|(w, _)| Some(*w) == self.selected) else {
            self.selected = order.first().map(|(w, _)| *w);
            return;
        };
        if let Some(to) = overview::step(&rects, from, dir) {
            self.selected = Some(order[to].0);
        }
    }

    /// Tab and Shift+Tab, and Home and End: through the tiles in order, wrapping.
    fn advance(&mut self, by: isize) {
        let order = self.tiles();
        if order.is_empty() {
            return;
        }
        let n = order.len() as isize;
        let from = order.iter().position(|(w, _)| Some(*w) == self.selected).map_or(-1, |i| i as isize);
        let to = match by {
            isize::MIN => 0,
            isize::MAX => n - 1,
            by => (from + by).rem_euclid(n),
        };
        self.selected = Some(order[to as usize].0);
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The id of the thread showing the overview, or 0.
static OPEN: AtomicU32 = AtomicU32::new(0);
/// The overview's thread on its way out, which hasn't ended yet, or 0.
static CLOSING: AtomicU32 = AtomicU32::new(0);
/// One thread starts an overview at a time.
static LAUNCH: Mutex<()> = Mutex::new(());
/// Moves for the tray, in order.
static MOVES: Mutex<Vec<Move>> = Mutex::new(Vec::new());

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::SeqCst) != 0
}

/// Close the overview if it's up. Returns whether it was.
pub fn close() -> bool {
    let open = OPEN.swap(0, Ordering::SeqCst);
    if open != 0 {
        unsafe {
            let _ = PostThreadMessageW(open, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    open != 0
}

/// For the key: close the overview if it's up, and say so. One already on its way
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

/// End the overview's message loop.
fn quit() {
    unsafe {
        CLOSING.store(GetCurrentThreadId(), Ordering::SeqCst);
        PostQuitMessage(0);
    }
}

pub fn take_moves() -> Vec<Move> {
    std::mem::take(&mut *locked(&MOVES))
}

/// Moves the tray was too busy for, to try again.
pub fn put_back(moves: Vec<Move>) {
    let mut queue = locked(&MOVES);
    let later = std::mem::take(&mut *queue);
    *queue = moves.into_iter().chain(later).collect();
}

fn rect_of(r: &geom::Rect) -> Rect {
    Rect { x: r.left, y: r.top, w: r.width(), h: r.height() }
}

/// The DPI a window has, at least 96.
fn dpi_of(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96)
}
