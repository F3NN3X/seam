//! Keys and the mouse wheel while a snap drag is moving: the wheel, Tab, 1, 2, 3, 0
//! and the arrows steer where the window lands, and Esc or the other mouse button
//! drops it without a zone. Low-level keyboard and mouse hooks on a thread of their
//! own, put in when a drag first shows zones and taken out when it ends, or after a
//! minute whatever happens (the D204 pattern). They swallow only those keys, the
//! wheel and the other button, only while zones show, and do nothing but post each
//! one to the tray. The snapping works out what it means on the main thread.
//! Every other key, the drag's own button and every move pass through.

use crate::log;
use seam::groups::Side;
use seam::zones::{self, Rect};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A key for the drag, with its virtual-key code in wparam, or 0 when the hook
/// took itself out at the cap, with its generation in lparam.
pub const WM_APP_DRAG_KEY: u32 = WM_APP + 32;
/// The hook takes itself out after this long, even if the drag never ends.
const CAP_MS: u32 = 60_000;

/// The generation of the hook whose drag shows zones, so its keys are the drag's, or 0.
static LIVE: AtomicU64 = AtomicU64::new(0);
/// Ctrl and Alt are the drag's own show or span key, so holding them doesn't hand keys back.
static CTRL_OK: AtomicBool = AtomicBool::new(false);
static ALT_OK: AtomicBool = AtomicBool::new(false);
/// Keys whose press was swallowed, so their release is too, and a repeat posts nothing.
static DOWN: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
/// The left and right mouse buttons, when their press was swallowed.
static BUTTONS: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];

/// What the mouse hook posts in place of a virtual-key code, past every real one.
const WHEEL_NEXT: u32 = 0x1001;
const WHEEL_BACK: u32 = 0x1002;
const OTHER_BUTTON: u32 = 0x1003;

/// The generation of a hook whose drag is over but which still owes a release:
/// it takes itself out once nothing it swallowed the press of is still down.
static ENDING: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The generation of the hook on this thread.
    static MINE: Cell<u64> = const { Cell::new(0) };
}
/// The tray window the keys go to.
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// The id of the hook's thread, or 0.
static OPEN: AtomicU32 = AtomicU32::new(0);
/// Counts starts and stops, so a thread that was stopped before it got going ends.
static WANTED: AtomicU64 = AtomicU64::new(0);

/// How much of the span the dropped window takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Half,
    Third,
    TwoThirds,
}

impl Part {
    pub fn share(self) -> f32 {
        match self {
            Part::Half => 0.5,
            Part::Third => 1.0 / 3.0,
            Part::TwoThirds => 2.0 / 3.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Part::Half => "1/2",
            Part::Third => "1/3",
            Part::TwoThirds => "2/3",
        }
    }
}

/// What a key does to the drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Tab: the next span under the pointer, or the one before with Shift.
    Next,
    /// 1, 2, 3, or 0 for the whole span.
    Part(Option<Part>),
    /// An arrow: the side the part sits on.
    Side(Side),
    /// Esc: this drop doesn't snap.
    Cancel,
    /// The wheel: the next step of whole zone, 1/2, 1/3, 2/3, zone and neighbour,
    /// whole screen, or the one before.
    Scroll(bool),
    /// The other mouse button: this drop doesn't snap.
    Button,
}

/// The keys a drag takes, by virtual-key code, and what the mouse hook posts.
pub fn key_of(vk: u32) -> Option<Key> {
    match vk {
        WHEEL_NEXT => return Some(Key::Scroll(true)),
        WHEEL_BACK => return Some(Key::Scroll(false)),
        OTHER_BUTTON => return Some(Key::Button),
        0x100.. => return None,
        _ => {}
    }
    let vk = VIRTUAL_KEY(vk as u16);
    Some(match vk {
        VK_TAB => Key::Next,
        VK_ESCAPE => Key::Cancel,
        VK_1 | VK_NUMPAD1 => Key::Part(Some(Part::Half)),
        VK_2 | VK_NUMPAD2 => Key::Part(Some(Part::Third)),
        VK_3 | VK_NUMPAD3 => Key::Part(Some(Part::TwoThirds)),
        VK_0 | VK_NUMPAD0 => Key::Part(None),
        VK_LEFT => Key::Side(Side::Left),
        VK_RIGHT => Key::Side(Side::Right),
        VK_UP => Key::Side(Side::Top),
        VK_DOWN => Key::Side(Side::Bottom),
        _ => return None,
    })
}

/// A key's name for the log.
pub fn name(key: Key) -> &'static str {
    match key {
        Key::Next => "Tab",
        Key::Part(Some(part)) => part.name(),
        Key::Part(None) => "0",
        Key::Side(Side::Left) => "Left",
        Key::Side(Side::Right) => "Right",
        Key::Side(Side::Top) => "Up",
        Key::Side(Side::Bottom) => "Down",
        Key::Cancel => "Esc",
        Key::Scroll(_) => "Wheel",
        Key::Button => "Right-click",
    }
}

/// What the keys chose during one drag.
#[derive(Default)]
pub struct Steer {
    /// Which of the spans under the pointer Tab picked, from 0 for the zone itself.
    pub cycle: usize,
    pub part: Option<Part>,
    /// Picked with an arrow. None is the side of the span nearest the pointer.
    pub side: Option<Side>,
    /// Esc: this drop doesn't snap.
    pub off: bool,
    /// The hook is in for this drag.
    pub hooked: bool,
    /// The generation `start` gave this drag's hook.
    pub generation: u64,
    /// The hook took itself out at the cap, so it isn't put back for this drag.
    pub capped: bool,
    /// The screen and zone Tab's choice is for. Another zone starts again from the zone itself.
    pub cycle_at: Option<(usize, usize)>,
    /// The screen and zone an arrow picked the side in. Another zone follows the pointer again.
    pub side_at: Option<(usize, usize)>,
    /// The keys pressed, each once, for the log.
    pub used: Vec<&'static str>,
}

/// The spans Tab steps through under the pointer: the zone, the zone with a
/// neighbour, and the whole screen, none twice. The neighbour is the one on the
/// side the pointer leans to, else on the other axis it leans along, else any.
/// `cells` are the zones with no gap.
pub fn candidates(cells: &[Rect], zone: usize, pt: (i32, i32)) -> Vec<(usize, usize)> {
    let mut list = vec![(zone, zone)];
    if let Some(n) = cells.get(zone).and_then(|c| neighbour(cells, zone, *c, pt)) {
        list.push((zone.min(n), zone.max(n)));
    }
    list.push((0, cells.len().saturating_sub(1)));
    let mut seen = Vec::new();
    list.retain(|span| {
        let new = !seen.contains(span);
        seen.push(*span);
        new
    });
    list
}

/// The zone across one side of `c`, trying the sides in the order the point leans.
/// Each side is probed just past its edge, level with the point where it can be,
/// else at the side's middle.
fn neighbour(cells: &[Rect], zone: usize, c: Rect, (x, y): (i32, i32)) -> Option<usize> {
    let dx = (x - (c.x + c.w / 2)) as f32 / c.w.max(1) as f32;
    let dy = (y - (c.y + c.h / 2)) as f32 / c.h.max(1) as f32;
    let (h, v) = (if dx >= 0. { Side::Right } else { Side::Left }, if dy >= 0. { Side::Bottom } else { Side::Top });
    let opposite = |s: Side| match s {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
    };
    let order = if dx.abs() >= dy.abs() { [h, v, opposite(v), opposite(h)] } else { [v, h, opposite(h), opposite(v)] };
    let (y, x) = (y.clamp(c.y, c.y + c.h - 1), x.clamp(c.x, c.x + c.w - 1));
    let probes = |side: Side| match side {
        Side::Left => [(c.x - 1, y), (c.x - 1, c.y + c.h / 2)],
        Side::Right => [(c.x + c.w, y), (c.x + c.w, c.y + c.h / 2)],
        Side::Top => [(x, c.y - 1), (c.x + c.w / 2, c.y - 1)],
        Side::Bottom => [(x, c.y + c.h), (c.x + c.w / 2, c.y + c.h)],
    };
    order.into_iter().flat_map(probes).find_map(|(px, py)| zones::zone_at(cells, px, py).filter(|n| *n != zone))
}

/// The side of `r` nearest the point.
pub fn nearest_side(r: Rect, (x, y): (i32, i32)) -> Side {
    let sides =
        [(x - r.x, Side::Left), (r.x + r.w - x, Side::Right), (y - r.y, Side::Top), (r.y + r.h - y, Side::Bottom)];
    sides.into_iter().min_by_key(|(d, _)| *d).map_or(Side::Left, |(_, side)| side)
}

/// The strip's text.
pub const HINT: &str = "Scroll to change  Right-click don't snap  Tab/1/2/3/0/Esc";

/// Swallow the drag's keys while zones show, and post each press to the tray.
/// A release is swallowed only when its press was. A press with Ctrl, Alt or Win
/// held passes, unless that key is the drag's own; Shift is Shift+Tab's.
unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let key = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let slot = &DOWN[(key.vkCode & 0xFF) as usize];
        let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
        if up && slot.swap(false, Ordering::Relaxed) {
            released();
            return LRESULT(1);
        }
        let live = LIVE.load(Ordering::Relaxed);
        if !up && live != 0 && live == MINE.get() && key_of(key.vkCode).is_some() && !foreign_modifier() {
            // A held key repeats its press: swallow it, but it steers only once.
            if !slot.swap(true, Ordering::Relaxed) {
                let owner = HWND(OWNER.load(Ordering::Relaxed) as *mut _);
                let _ = PostMessageW(Some(owner), WM_APP_DRAG_KEY, WPARAM(key.vkCode as usize), LPARAM(0));
            }
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Ctrl, Alt or Win is held and isn't the drag's own show or span key, so what
/// comes with it is the app's: Alt+Tab, Ctrl+wheel zoom.
unsafe fn foreign_modifier() -> bool {
    let pressed = |vk: VIRTUAL_KEY| GetAsyncKeyState(i32::from(vk.0)) < 0;
    (pressed(VK_CONTROL) && !CTRL_OK.load(Ordering::Relaxed))
        || (pressed(VK_MENU) && !ALT_OK.load(Ordering::Relaxed))
        || pressed(VK_LWIN)
        || pressed(VK_RWIN)
}

/// A swallowed release came. A hook whose drag is over ends once it owes none.
unsafe fn released() {
    let ending = ENDING.load(Ordering::Relaxed);
    if ending != 0 && ending == MINE.get() && !owes() {
        PostQuitMessage(0);
    }
}

fn owes() -> bool {
    DOWN.iter().chain(&BUTTONS).any(|d| d.load(Ordering::Relaxed))
}

/// Swallow the wheel and the other button's press and release while zones show,
/// and post each turn or press to the tray. The button held for the drag was down
/// before the hook went in, so a press seen now is always the other one. A release
/// is swallowed only when its press was. Moves are let through at once.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let msg = wparam.0 as u32;
    if code >= 0 && msg != WM_MOUSEMOVE {
        let button = match msg {
            WM_LBUTTONDOWN | WM_LBUTTONUP => Some(&BUTTONS[0]),
            WM_RBUTTONDOWN | WM_RBUTTONUP => Some(&BUTTONS[1]),
            _ => None,
        };
        if matches!(msg, WM_LBUTTONUP | WM_RBUTTONUP) && button.is_some_and(|b| b.swap(false, Ordering::Relaxed)) {
            released();
            return LRESULT(1);
        }
        let live = LIVE.load(Ordering::Relaxed);
        if live != 0 && live == MINE.get() {
            let what = match msg {
                WM_MOUSEWHEEL if foreign_modifier() => None,
                WM_MOUSEWHEEL => {
                    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
                    let delta = (info.mouseData >> 16) as u16 as i16;
                    Some(if delta < 0 { WHEEL_NEXT } else { WHEEL_BACK })
                }
                WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                    button.filter(|b| !b.swap(true, Ordering::Relaxed)).map(|_| OTHER_BUTTON)
                }
                _ => None,
            };
            if let Some(what) = what {
                let owner = HWND(OWNER.load(Ordering::Relaxed) as *mut _);
                let _ = PostMessageW(Some(owner), WM_APP_DRAG_KEY, WPARAM(what as usize), LPARAM(0));
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn run(generation: u64) {
    unsafe {
        let id = GetCurrentThreadId();
        let mut msg = MSG::default();
        // Creates this thread's queue, so a stop from now on reaches it.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        OPEN.store(id, Ordering::SeqCst);
        MINE.set(generation);
        // A press swallowed under an earlier hook may never have had its release seen.
        clear_down();
        if WANTED.load(Ordering::SeqCst) == generation {
            match SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0) {
                Ok(hook) => {
                    let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
                    if let Err(e) = &mouse {
                        log::info!("drag keys: no mouse hook: {e}");
                    }
                    SetTimer(None, 0, CAP_MS, None);
                    while WANTED.load(Ordering::SeqCst) == generation && GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                            log::info!("drag keys: the hook took itself out after 60 s");
                            // The drag hears of it, so its strip goes and the hook isn't put back.
                            let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
                            let _ = PostMessageW(Some(owner), WM_APP_DRAG_KEY, WPARAM(0), LPARAM(generation as isize));
                            break;
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                    if let Ok(mouse) = mouse {
                        let _ = UnhookWindowsHookEx(mouse);
                    }
                    clear_down();
                }
                Err(e) => log::info!("drag keys: no keyboard hook: {e}"),
            }
        }
        let _ = LIVE.compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst);
        let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

fn clear_down() {
    DOWN.iter().chain(&BUTTONS).for_each(|d| d.store(false, Ordering::Relaxed));
}

/// Put the hook in for a drag, its keys going to `owner`. `ctrl` and `alt` say
/// whether those are the drag's show or span key. It passes every key through
/// until `set_live` says zones show. Returns its generation, which the cap's message carries.
pub fn start(owner: isize, ctrl: bool, alt: bool) -> u64 {
    stop();
    OWNER.store(owner, Ordering::SeqCst);
    CTRL_OK.store(ctrl, Ordering::SeqCst);
    ALT_OK.store(alt, Ordering::SeqCst);
    let generation = WANTED.load(Ordering::SeqCst);
    if let Err(e) = std::thread::Builder::new().name("seam-dragkeys".into()).spawn(move || run(generation)) {
        log::info!("drag keys: couldn't start: {e}");
    }
    generation
}

/// Whether zones show, so the drag's keys are swallowed. One atomic store.
pub fn set_live(on: bool) {
    LIVE.store(if on { WANTED.load(Ordering::SeqCst) } else { 0 }, Ordering::SeqCst);
}

/// The drag ended, or won't snap: nothing more is swallowed or posted, and the
/// hook stays only until the releases of what it swallowed come, so no app gets a
/// lone release (a right-button release opens a context menu). The cap still holds.
pub fn release() {
    let generation = WANTED.load(Ordering::SeqCst);
    LIVE.store(0, Ordering::SeqCst);
    ENDING.store(generation, Ordering::SeqCst);
    if !owes() {
        stop();
    }
}

/// A new drag, or the tray is quitting: take the hook out now.
pub fn stop() {
    LIVE.store(0, Ordering::SeqCst);
    WANTED.fetch_add(1, Ordering::SeqCst);
    let open = OPEN.swap(0, Ordering::SeqCst);
    if open != 0 {
        unsafe {
            let _ = PostThreadMessageW(open, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}
