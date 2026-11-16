//! Where the pointer goes on the desk: resting it in an outer corner opens the
//! overview, and Win+wheel over a Scrolling screen scrolls its strip. A low-level mouse
//! hook on a thread of its own (the D204 pattern), in only while something wants it.
//! The callback reads atomics, the buttons and the Win keys, keeps the rest's and the
//! wheel's state and posts, and the thread times the rest. It swallows only a wheel
//! turned with Win held over a Scrolling screen. A use that needs the pointer later adds
//! its own want to `configure`, and the hook is in while any is. `src/hook.rs` is left
//! alone. While it is in, the thread wakes for a rest's timer, for a Win+wheel's key tap,
//! and once a minute to look whether Windows took the hook out, and for nothing else.

use crate::log;
use seam::desk::Mon;
use seam::geom::Rect;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

mod rules;

pub use rules::Spot;
use rules::{Corner, Rest, Step};

/// The pointer rested in the corner: open the overview.
pub const WM_APP_CORNER: u32 = WM_APP + 40;
/// Win+wheel over a Scrolling screen, one column's worth: wparam the way and when (`wheel_of`),
/// lparam the point, x low and y high.
pub const WM_APP_WHEEL: u32 = WM_APP + 41;

/// To the hook's thread: look at the rest again in wparam milliseconds.
const MSG_WAIT: u32 = WM_APP + 1;
/// Wakes the hook's thread so it looks whether it is still wanted.
const MSG_WAKE: u32 = WM_APP + 2;
/// The hook's thread looks whether Windows took the hook out.
const MSG_CHECK: u32 = WM_APP + 3;
/// The hook's thread taps a key no app uses, so letting go of Win after a Win+wheel opens no Start.
const MSG_TAP: u32 = WM_APP + 4;
/// One notch of a wheel. A touchpad's smaller steps add up to one.
const NOTCH: i32 = 120;
/// How often the hook's thread looks, while it is in.
const CHECK_MS: u32 = 60_000;
/// Corners kept for the hook, one per screen at most. Past this many screens, the rest are left out.
const MAX_SPOTS: usize = 8;

/// The tray window, told of a rest.
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// The corner's spots, left, top, right and bottom, and how many there are. Written
/// with the count at 0 first, so the callback sees none, the old or the new ones, and
/// at worst a mix for one move during a display change.
static SPOTS: [[AtomicI32; 4]; MAX_SPOTS] = [const { [const { AtomicI32::new(0) }; 4] }; MAX_SPOTS];
static COUNT: AtomicUsize = AtomicUsize::new(0);
/// The Scrolling screens, left, top, right and bottom, and how many, written the same way.
static WHEEL: [[AtomicI32; 4]; MAX_SPOTS] = [const { [const { AtomicI32::new(0) }; 4] }; MAX_SPOTS];
static WHEEL_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Something wants the hook.
static ON: AtomicBool = AtomicBool::new(false);
/// Counts starts and stops: a hook whose generation is gone goes.
static WANTED: AtomicU64 = AtomicU64::new(0);
/// The newest hook's thread, or 0.
static OPEN: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// Where the pointer is with the corner. The callback runs on the hook's thread,
    /// between its messages, so both keep it here.
    static REST: Cell<Rest> = const { Cell::new(Rest::Away) };
    /// Calls of the hook on this thread, to tell whether Windows took it out.
    static CALLS: Cell<u64> = const { Cell::new(0) };
    /// The wheel turned with Win held and not yet a notch, so a touchpad scrolls a column a notch.
    static TURNED: Cell<i32> = const { Cell::new(0) };
}

/// The spots of the chosen corner on this desk: the screens' outer corners only, each
/// a few pixels across at its screen's scale. Called when the screens change.
pub fn corner_spots(mons: &[Mon], corner: &str) -> Vec<Spot> {
    let screens: Vec<(Rect, u32)> = mons.iter().map(|m| (m.rect, dpi_of(m.rect))).collect();
    rules::outer_corners(&screens, Corner::from_name(corner))
}

fn dpi_of(r: Rect) -> u32 {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: r.left, y: r.top }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (0, 0);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 { dx } else { 96 }
    }
}

/// Put the hook in or take it out. `corner` is where a rest opens the overview, empty
/// while that is off or Seam stands by; `wheel` the screens Win+wheel scrolls, empty
/// while none does. Cheap when nothing changed.
pub fn configure(owner: isize, corner: &[Spot], wheel: &[Rect]) {
    OWNER.store(owner, Ordering::SeqCst);
    COUNT.store(0, Ordering::SeqCst);
    for (slot, spot) in SPOTS.iter().zip(corner) {
        for (cell, value) in slot.iter().zip([spot.left, spot.top, spot.right, spot.bottom]) {
            cell.store(value, Ordering::SeqCst);
        }
    }
    COUNT.store(corner.len().min(MAX_SPOTS), Ordering::SeqCst);
    WHEEL_COUNT.store(0, Ordering::SeqCst);
    for (slot, r) in WHEEL.iter().zip(wheel) {
        for (cell, value) in slot.iter().zip([r.left, r.top, r.right, r.bottom]) {
            cell.store(value, Ordering::SeqCst);
        }
    }
    WHEEL_COUNT.store(wheel.len().min(MAX_SPOTS), Ordering::SeqCst);
    let on = !corner.is_empty() || !wheel.is_empty();
    if ON.swap(on, Ordering::SeqCst) == on {
        // Still on: the hook's thread looks whether Windows took the hook out.
        let open = OPEN.load(Ordering::SeqCst);
        if on && open != 0 {
            unsafe {
                let _ = PostThreadMessageW(open, MSG_CHECK, WPARAM(0), LPARAM(0));
            }
        }
        return;
    }
    if on {
        let generation = WANTED.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(e) = std::thread::Builder::new().name("seam-mousewatch".into()).spawn(move || run(generation)) {
            log::info!("pointer watch: couldn't start: {e}");
            // The next change of settings tries again.
            ON.store(false, Ordering::SeqCst);
        }
    } else {
        stop();
    }
}

/// Nothing wants the hook any more, or Seam is quitting.
pub fn stop() {
    ON.store(false, Ordering::SeqCst);
    COUNT.store(0, Ordering::SeqCst);
    WHEEL_COUNT.store(0, Ordering::SeqCst);
    WANTED.fetch_add(1, Ordering::SeqCst);
    let open = OPEN.swap(0, Ordering::SeqCst);
    if open != 0 {
        unsafe {
            let _ = PostThreadMessageW(open, MSG_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

/// A mouse button is down, so a drag or a click is going on.
pub fn button_down() -> bool {
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
        .into_iter()
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}

/// The hook couldn't go in, or was lost: the next change of settings tries again.
fn failed(generation: u64) {
    if WANTED.load(Ordering::SeqCst) == generation {
        ON.store(false, Ordering::SeqCst);
    }
}

fn run(generation: u64) {
    unsafe {
        let id = GetCurrentThreadId();
        let mut msg = MSG::default();
        // Creates this thread's queue, so a stop from now on reaches it.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        OPEN.store(id, Ordering::SeqCst);
        let install = || SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
        if WANTED.load(Ordering::SeqCst) == generation {
            match install() {
                Ok(mut hook) => {
                    log::info!("pointer watch: hook installed");
                    let check = SetTimer(None, 0, CHECK_MS, None);
                    // The rest's timer, or 0 while none runs.
                    let mut wait = 0;
                    let mut seen = (CALLS.get(), cursor());
                    while WANTED.load(Ordering::SeqCst) == generation && GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let timer = msg.message == WM_TIMER && msg.hwnd.is_invalid();
                        if msg.message == MSG_TAP {
                            crate::altdrag::tap();
                        } else if msg.message == MSG_WAIT {
                            // The same id puts the timer back rather than start another.
                            wait = SetTimer(None, wait, msg.wParam.0 as u32, None);
                        } else if timer && wait != 0 && msg.wParam.0 == wait {
                            let _ = KillTimer(None, wait);
                            wait = 0;
                            // Where the pointer is now, not where the hook last saw it. A read
                            // that fails counts as out of the corner, since 0,0 may be in one.
                            let mut pt = POINT::default();
                            let inside = GetCursorPos(&mut pt).is_ok() && in_corner(pt);
                            let (rest, step) = REST.get().tick(inside, button_down(), GetTickCount());
                            REST.set(rest);
                            match step {
                                Step::Wait(ms) => wait = SetTimer(None, 0, ms, None),
                                Step::Open => tell(),
                                Step::Nothing => {}
                            }
                        } else if msg.message == MSG_CHECK || (timer && msg.wParam.0 == check) {
                            // Windows takes out a hook that was too slow, and says nothing. The
                            // pointer moved since the last look, and the hook heard nothing of it.
                            let now = (CALLS.get(), cursor());
                            if now.0 == seen.0 && now.1 != seen.1 {
                                let _ = UnhookWindowsHookEx(hook);
                                match install() {
                                    Ok(again) => {
                                        hook = again;
                                        log::info!("pointer watch: hook re-armed, Windows had taken it out");
                                    }
                                    Err(e) => {
                                        log::info!("pointer watch: hook lost and not put back: {e}");
                                        failed(generation);
                                        break;
                                    }
                                }
                            }
                            seen = (CALLS.get(), cursor());
                        }
                    }
                    let _ = UnhookWindowsHookEx(hook);
                    log::info!("pointer watch: hook uninstalled");
                }
                Err(e) => {
                    log::info!("pointer watch: no mouse hook: {e}");
                    failed(generation);
                }
            }
        }
        let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

fn cursor() -> POINT {
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    pt
}

/// Whether the point is in one of the corner's spots, from the atomics.
fn in_corner(pt: POINT) -> bool {
    let count = COUNT.load(Ordering::Relaxed).min(MAX_SPOTS);
    let spot = |s: &[AtomicI32; 4]| {
        let [left, top, right, bottom] = [0, 1, 2, 3].map(|edge| s[edge].load(Ordering::Relaxed));
        Spot { left, top, right, bottom }
    };
    rules::in_corner(SPOTS[..count].iter().map(spot), (pt.x, pt.y))
}

/// Whether the point is over a Scrolling screen, from the atomics.
fn over_scrolling(pt: POINT) -> bool {
    let count = WHEEL_COUNT.load(Ordering::Relaxed).min(MAX_SPOTS);
    WHEEL[..count].iter().any(|s| {
        let [left, top, right, bottom] = [0, 1, 2, 3].map(|edge| s[edge].load(Ordering::Relaxed));
        pt.x >= left && pt.x < right && pt.y >= top && pt.y < bottom
    })
}

/// Either Win key is down.
fn win_held() -> bool {
    [VK_LWIN, VK_RWIN].into_iter().any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}

/// A post's wparam: the way, and the hook's time of the wheel, for the lag in `--log`.
fn wheel_param(left: bool, time: u32) -> usize {
    ((time as usize) << 1) | usize::from(left)
}

/// The way and how many milliseconds ago the hook heard it, from a `WM_APP_WHEEL`'s wparam.
pub fn wheel_of(wparam: usize) -> (bool, u32) {
    let time = (wparam >> 1) as u32;
    (wparam & 1 == 1, unsafe { GetTickCount() }.wrapping_sub(time))
}

/// A wheel turned with Win held over a Scrolling screen: tell the tray a notch at a time,
/// and have the thread tap a key so letting go of Win opens no Start. Up and left
/// scroll left. Whether it is swallowed.
fn wheel(msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
    let vertical = msg == WM_MOUSEWHEEL;
    if !(vertical || msg == WM_MOUSEHWHEEL) || !win_held() || !over_scrolling(info.pt) {
        TURNED.set(0);
        return false;
    }
    let delta = i32::from((info.mouseData >> 16) as u16 as i16);
    // Up on a wheel and left on a tilt are the same way.
    let delta = if vertical { delta } else { -delta };
    let mut turned = if TURNED.get().signum() == delta.signum() { TURNED.get() + delta } else { delta };
    while turned.abs() >= NOTCH {
        let left = turned > 0;
        turned -= NOTCH * turned.signum();
        let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
        let point = (info.pt.x as u16 as isize) | ((info.pt.y as u16 as isize) << 16);
        unsafe {
            let _ = PostMessageW(Some(owner), WM_APP_WHEEL, WPARAM(wheel_param(left, info.time)), LPARAM(point));
        }
    }
    TURNED.set(turned);
    unsafe {
        let _ = PostThreadMessageW(GetCurrentThreadId(), MSG_TAP, WPARAM(0), LPARAM(0));
    }
    true
}

/// Follow the pointer in and out of the corner, and have the thread time a rest. Only a
/// Win+wheel over a Scrolling screen is swallowed; everything else passes through.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        CALLS.set(CALLS.get() + 1);
        if code >= 0 {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let msg = wparam.0 as u32;
            if wheel(msg, info) {
                return LRESULT(1);
            }
            let press = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN);
            if press || msg == WM_MOUSEMOVE {
                let inside = in_corner(info.pt);
                // The press isn't in the button state yet, so it counts on its own.
                let button = press || (inside && button_down());
                let (rest, step) = REST.get().moved(inside, button, info.time);
                REST.set(rest);
                if let Step::Wait(ms) = step {
                    let _ = PostThreadMessageW(GetCurrentThreadId(), MSG_WAIT, WPARAM(ms as usize), LPARAM(0));
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Tell the tray the pointer rested in the corner. It decides whether the overview opens.
fn tell() {
    let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
    unsafe {
        let _ = PostMessageW(Some(owner), WM_APP_CORNER, WPARAM(0), LPARAM(0));
    }
}
