//! Why Seam stands by (D390): paused by hand, or stepped aside while a game or
//! another full-screen app is in front. Every feature asks `reason`, so this is
//! the one place to read it. Pause is runtime state and never saved.
//!
//! The full-screen answer is kept up to date from the tray thread: on a foreground
//! change, when the foreground window moves or resizes (F11 sends no foreground
//! change), and when the full-screen window closes or minimizes. A callback only
//! posts or sets a timer, and the tray calls `check` when it arrives.

use crate::log;
use seam::winutil;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// The foreground changed: look again with `check`.
pub const WM_APP_STANDBY: u32 = WM_APP + 33;
/// Looks again with `check`, after a move, a resize or the full-screen window going.
pub const TIMER_STANDBY: usize = 12;
/// A foreground window that moves or resizes in a burst is looked at once, this long after.
const LOOK_AFTER_MS: u32 = 200;
/// A closed or minimized full-screen window is looked at soon.
const GONE_AFTER_MS: u32 = 50;
/// How far from the screen's edge a window may sit and still cover it.
const EDGE_SLACK: i32 = 3;
/// Windows that come and go over the app in use: the shell's own, menus and toasts.
/// They leave the answer as it was.
const TRANSIENT_CLASSES: [&str; 13] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "MultitaskingViewFrame",
    "XamlExplorerHostIslandWindow",
    "Windows.UI.Core.CoreWindow",
    "ForegroundStaging",
    "TaskSwitcherWnd",
    "TaskSwitcherOverlayWnd",
    "NativeHWNDHost",
    "#32768",
    "NotifyIconOverflowWindow",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standby {
    /// Paused by the user.
    Paused,
    /// A game or other full-screen app is in front.
    FullScreen,
}

/// What the foreground window says.
enum Seen {
    Full(&'static str),
    Not,
    /// A transient window: nothing changed.
    Keep,
}

static PAUSED: AtomicBool = AtomicBool::new(false);
static FULL_SCREEN: AtomicBool = AtomicBool::new(false);
/// "Step aside for games and full-screen apps" is on.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// The tray window, told when the full-screen answer may have changed.
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// The full-screen window being followed, 0 for none.
static WATCHED: AtomicIsize = AtomicIsize::new(0);
/// The foreground window whose moves are followed, 0 for none.
static FOLLOWED: AtomicIsize = AtomicIsize::new(0);
/// The watched window closed or minimized, so it is no longer full screen.
static GONE: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Hooks {
    foreground: Option<HWINEVENTHOOK>,
    /// Moves and resizes of the foreground window, for its thread only.
    followed: Option<HWINEVENTHOOK>,
    /// The close and minimize of the watched window, for its thread only.
    watched: Vec<HWINEVENTHOOK>,
}

thread_local! {
    static HOOKS: RefCell<Hooks> = RefCell::new(Hooks::default());
}

/// Why Seam stands by now, if it does. Paused wins over a full-screen app.
pub fn reason() -> Option<Standby> {
    if PAUSED.load(Ordering::SeqCst) {
        Some(Standby::Paused)
    } else if FULL_SCREEN.load(Ordering::SeqCst) {
        Some(Standby::FullScreen)
    } else {
        None
    }
}

pub fn paused() -> bool {
    reason() == Some(Standby::Paused)
}

pub fn set_paused(on: bool) {
    PAUSED.store(on, Ordering::SeqCst);
}

/// Follow the foreground while "Step aside for games and full-screen apps" is on, and
/// look now. Returns whether the answer changed.
pub fn configure(owner: HWND, on: bool) -> bool {
    OWNER.store(owner.0 as isize, Ordering::SeqCst);
    WATCHING.store(on, Ordering::SeqCst);
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        match (on, hooks.foreground) {
            (true, None) => unsafe {
                let foreground = EVENT_SYSTEM_FOREGROUND;
                let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                let hook = SetWinEventHook(foreground, foreground, None, Some(on_foreground), 0, 0, flags);
                if hook.0.is_null() {
                    log::info!("standby: foreground hook not installed");
                } else {
                    hooks.foreground = Some(hook);
                }
            },
            (false, Some(hook)) => {
                let _ = unsafe { UnhookWinEvent(hook) };
                hooks.foreground = None;
            }
            _ => {}
        }
    });
    if on {
        check()
    } else {
        follow(None);
        leave()
    }
}

/// Look at what is in front. Returns whether the full-screen answer changed, and
/// logs it once, with the reason and no window titles.
pub fn check() -> bool {
    if !WATCHING.load(Ordering::SeqCst) {
        GONE.store(false, Ordering::SeqCst);
        return false;
    }
    let foreground = unsafe { GetForegroundWindow() };
    // A transient window in front keeps the answer only while the full-screen window is
    // still there and still covers its screen. If it closed, minimized or shrank, it is
    // gone, whatever the foreground says.
    let watched = HWND(WATCHED.load(Ordering::SeqCst) as *mut _);
    let gone = GONE.swap(false, Ordering::SeqCst);
    let mut changed = false;
    if FULL_SCREEN.load(Ordering::SeqCst) && (gone || !still_full(watched)) {
        changed = leave();
    }
    // Between two windows there is none for a moment. That says nothing.
    if foreground.is_invalid() {
        return changed;
    }
    match look(foreground) {
        Seen::Keep => changed,
        Seen::Full(why) => {
            follow(Some(foreground));
            // A window that is already gone has nothing to follow, and isn't full screen.
            if !watch(Some(foreground)) {
                return leave() || changed;
            }
            let entered = !FULL_SCREEN.swap(true, Ordering::SeqCst);
            if entered {
                log::info!("full-screen app in front ({why}): Seam steps aside");
            }
            changed || entered
        }
        Seen::Not => {
            follow(Some(foreground));
            leave() || changed
        }
    }
}

/// The watched window still exists, isn't minimized and still covers its screen.
fn still_full(h: HWND) -> bool {
    !h.0.is_null() && unsafe { IsWindow(Some(h)).as_bool() } && covers_screen(h, false)
}

/// No full-screen app is in front. Returns whether there was one.
fn leave() -> bool {
    watch(None);
    let changed = FULL_SCREEN.swap(false, Ordering::SeqCst);
    if changed {
        log::info!("full-screen app gone: Seam resumes");
    }
    changed
}

/// The window covers the whole of the screen it is on, give or take a few pixels.
fn covers_screen(h: HWND, exact: bool) -> bool {
    let Some(window) = winutil::window_rect(h) else { return false };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let monitor = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() || IsIconic(h).as_bool() {
            return false;
        }
    }
    let screen = info.rcMonitor;
    // Each edge on the screen's or beyond it, give or take a few pixels. Exact also
    // keeps it from reaching further than that: a maximized window overhangs by more.
    let beyond = |edge: i32, at: i32| !exact || (edge - at).abs() <= EDGE_SLACK;
    window.x <= screen.left + EDGE_SLACK
        && window.y <= screen.top + EDGE_SLACK
        && window.x + window.w >= screen.right - EDGE_SLACK
        && window.y + window.h >= screen.bottom - EDGE_SLACK
        && beyond(window.x, screen.left)
        && beyond(window.y, screen.top)
        && beyond(window.x + window.w, screen.right)
        && beyond(window.y + window.h, screen.bottom)
}

/// Whether the foreground window is a full-screen app. Only a window that covers its
/// whole screen counts, so a game on another screen changes nothing. Windows' own word
/// for a Direct3D or presenting app confirms one that has a frame or a maximized look.
fn look(foreground: HWND) -> Seen {
    let transient = unsafe { foreground == GetDesktopWindow() || foreground == GetShellWindow() }
        || winutil::process_id(foreground) == std::process::id()
        || TRANSIENT_CLASSES.contains(&winutil::class_name(foreground).as_str());
    if transient {
        return Seen::Keep;
    }
    // No title bar and not maximized: an F11 page, a borderless, OpenGL or Vulkan game.
    // A maximized app without one (Spotify, VS Code) overhangs its screen, so it needs
    // Windows' word below.
    let style = unsafe { GetWindowLongW(foreground, GWL_STYLE) } as u32;
    let captionless = style & WS_CAPTION.0 != WS_CAPTION.0;
    if captionless && !unsafe { IsZoomed(foreground).as_bool() } && covers_screen(foreground, true) {
        return Seen::Full("window covers its screen");
    }
    if !covers_screen(foreground, false) {
        return Seen::Not;
    }
    match unsafe { SHQueryUserNotificationState() } {
        Ok(QUNS_RUNNING_D3D_FULL_SCREEN) => Seen::Full("Direct3D full screen"),
        Ok(QUNS_PRESENTATION_MODE) => Seen::Full("presentation mode"),
        _ => Seen::Not,
    }
}

fn thread_of(h: HWND) -> u32 {
    unsafe { GetWindowThreadProcessId(h, None) }
}

/// Follow the moves and resizes of the foreground window, for its thread only: a
/// browser's F11 changes its size without changing the foreground.
fn follow(window: Option<HWND>) {
    let key = window.map_or(0, |h| h.0 as isize);
    if FOLLOWED.swap(key, Ordering::SeqCst) == key {
        return;
    }
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        if let Some(hook) = hooks.followed.take() {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        let Some(window) = window else { return };
        // A thread id of 0 would hook every thread: a window that is gone has none.
        let thread = thread_of(window);
        if thread == 0 {
            FOLLOWED.store(0, Ordering::SeqCst);
            return;
        }
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let event = EVENT_OBJECT_LOCATIONCHANGE;
        let hook = unsafe { SetWinEventHook(event, event, None, Some(on_followed), 0, thread, flags) };
        if hook.0.is_null() {
            log::info!("standby: window hook {event:#x} not installed");
        } else {
            hooks.followed = Some(hook);
        }
    });
}

/// Follow this window's close and minimize, or none. The hooks are for its thread only.
fn watch(window: Option<HWND>) -> bool {
    let key = window.map_or(0, |h| h.0 as isize);
    if WATCHED.swap(key, Ordering::SeqCst) == key {
        return true;
    }
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        for hook in hooks.watched.drain(..) {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        let Some(window) = window else { return true };
        let thread = thread_of(window);
        if thread == 0 {
            WATCHED.store(0, Ordering::SeqCst);
            return false;
        }
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for event in [EVENT_OBJECT_DESTROY, EVENT_SYSTEM_MINIMIZESTART] {
            let hook = unsafe { SetWinEventHook(event, event, None, Some(on_watched), 0, thread, flags) };
            if hook.0.is_null() {
                log::info!("standby: window hook {event:#x} not installed");
            } else {
                hooks.watched.push(hook);
            }
        }
        true
    })
}

fn owner() -> Option<HWND> {
    let owner = OWNER.load(Ordering::SeqCst);
    (owner != 0).then_some(HWND(owner as *mut _))
}

/// Runs on the tray thread. It only posts, so the work happens in the message loop.
unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, _: HWND, object: i32, _: i32, _: u32, _: u32) {
    if let (true, Some(owner)) = (object == OBJID_WINDOW.0, owner()) {
        unsafe {
            let _ = PostMessageW(Some(owner), WM_APP_STANDBY, WPARAM(0), LPARAM(0));
        }
    }
}

/// The foreground window moved or resized. A burst of them is looked at once.
unsafe extern "system" fn on_followed(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if let (true, Some(owner)) =
        (object == OBJID_WINDOW.0 && hwnd.0 as isize == FOLLOWED.load(Ordering::SeqCst), owner())
    {
        unsafe { SetTimer(Some(owner), TIMER_STANDBY, LOOK_AFTER_MS, None) };
    }
}

/// The full-screen window closed or minimized: it is gone, and that is looked at soon.
unsafe extern "system" fn on_watched(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if let (true, Some(owner)) =
        (object == OBJID_WINDOW.0 && hwnd.0 as isize == WATCHED.load(Ordering::SeqCst), owner())
    {
        GONE.store(true, Ordering::SeqCst);
        unsafe { SetTimer(Some(owner), TIMER_STANDBY, GONE_AFTER_MS, None) };
    }
}
