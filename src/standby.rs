//! Why Seam stands by (D390): paused by hand, or stepped aside while a game or
//! another full-screen app is in front. Every feature asks `reason`, so this is
//! the one place to read it. Pause is runtime state and never saved.
//!
//! The full-screen answer is kept up to date from the tray thread, on a foreground
//! change (the existing WinEvent kind, no polling), and again when the full-screen
//! window closes or minimizes. A callback only posts `WM_APP_STANDBY`, and the tray
//! calls `check` when it arrives.

use crate::log;
use seam::winutil;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// The foreground changed, or the full-screen window went: look again with `check`.
pub const WM_APP_STANDBY: u32 = WM_APP + 33;
/// The shell's own full-screen windows: the desktop, the taskbar, Task View and Alt+Tab.
const SHELL_CLASSES: [&str; 6] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "MultitaskingViewFrame",
    "XamlExplorerHostIslandWindow",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standby {
    /// Paused by the user.
    Paused,
    /// A game or other full-screen app is in front.
    FullScreen,
}

impl Standby {
    /// The reason in the words a refusal or a status line uses.
    pub fn why(self) -> &'static str {
        match self {
            Standby::Paused => "Seam is paused",
            Standby::FullScreen => "A full-screen app is in front, so Seam steps aside",
        }
    }
}

static PAUSED: AtomicBool = AtomicBool::new(false);
static FULL_SCREEN: AtomicBool = AtomicBool::new(false);
/// "Step aside for games and full-screen apps" is on.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// The tray window, told when the full-screen answer may have changed.
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// The full-screen window being followed, 0 for none.
static WATCHED: AtomicIsize = AtomicIsize::new(0);

#[derive(Default)]
struct Hooks {
    foreground: Option<HWINEVENTHOOK>,
    /// The close and minimize of the watched window, filtered to its process.
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
/// look now. Does nothing again for the same setting. Returns whether the answer changed.
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
    if on { check() } else { leave() }
}

/// Look at what is in front. Returns whether the full-screen answer changed, and
/// logs it once, with the reason and no window titles.
pub fn check() -> bool {
    if !WATCHING.load(Ordering::SeqCst) {
        return false;
    }
    let foreground = unsafe { GetForegroundWindow() };
    // Between two windows there is none for a moment. That says nothing.
    if foreground.is_invalid() {
        return false;
    }
    match look(foreground) {
        Some(why) => {
            watch(Some(foreground));
            let changed = !FULL_SCREEN.swap(true, Ordering::SeqCst);
            if changed {
                log::info!("full-screen app in front ({why}): Seam steps aside");
            }
            changed
        }
        None => leave(),
    }
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

/// What makes the foreground window a full-screen app: the shell's own word for a
/// game, a busy or presenting user, or the window itself covering its whole screen.
fn look(foreground: HWND) -> Option<&'static str> {
    if let Ok(state) = unsafe { SHQueryUserNotificationState() } {
        match state {
            QUNS_RUNNING_D3D_FULL_SCREEN => return Some("Direct3D full screen"),
            QUNS_BUSY => return Some("shell reports a full-screen app"),
            QUNS_PRESENTATION_MODE => return Some("presentation mode"),
            _ => {}
        }
    }
    let shell = unsafe { foreground == GetDesktopWindow() || foreground == GetShellWindow() }
        || SHELL_CLASSES.contains(&winutil::class_name(foreground).as_str());
    let ours = winutil::process_id(foreground) == std::process::id();
    (!shell && !ours && winutil::is_full_screen(foreground)).then_some("window covers its screen")
}

/// Follow this window's close and minimize, or none. The hooks are for its process only.
fn watch(window: Option<HWND>) {
    let key = window.map_or(0, |h| h.0 as isize);
    if WATCHED.swap(key, Ordering::SeqCst) == key {
        return;
    }
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        for hook in hooks.watched.drain(..) {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        let Some(window) = window else { return };
        let pid = winutil::process_id(window);
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for event in [EVENT_OBJECT_DESTROY, EVENT_SYSTEM_MINIMIZESTART] {
            let hook = unsafe { SetWinEventHook(event, event, None, Some(on_watched), pid, 0, flags) };
            if hook.0.is_null() {
                log::info!("standby: window hook {event:#x} not installed");
            } else {
                hooks.watched.push(hook);
            }
        }
    });
}

fn tell_tray() {
    let owner = OWNER.load(Ordering::SeqCst);
    if owner != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(owner as *mut _)), WM_APP_STANDBY, WPARAM(0), LPARAM(0));
        }
    }
}

/// Runs on the tray thread. It only posts, so the work happens in the message loop.
unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, _: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object == OBJID_WINDOW.0 {
        tell_tray();
    }
}

unsafe extern "system" fn on_watched(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object == OBJID_WINDOW.0 && hwnd.0 as isize == WATCHED.load(Ordering::SeqCst) {
        tell_tray();
    }
}
