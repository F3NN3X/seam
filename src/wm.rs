//! The `seam-wm` thread: pins, the focus border, Release all, the check for
//! another window manager (D106, D108, D110, §3.5), and the window events behind
//! workspace drift and auto-apply (§5, §11.1). It has its own message loop,
//! like `seam-hook`, so WinEvent callbacks never run on the tray's thread. The
//! tray only posts commands. Answers go straight back to the API client, as
//! window control's do, and news for the tray comes as `WM_APP_WM`.
//!
//! Zone snapping keeps its own hook on the tray thread for now (v0.7 moves it).

use crate::log;
use crate::tray::WM_APP_WINDOWS_CHANGED;
use crate::wincontrol;
use seam::api::{self, PinKind, Reply, RpcError};
use seam::palette::{self, Colour, FOCUS_DEFAULT, PIN_DEFAULT};
use seam::state::OTHER_WMS;
use seam::{winutil, winver};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DwmSetWindowAttribute};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::*;

mod dispatch;
mod pins;
mod watch;

/// The window manager found running changed. Read it with `detected`.
pub const WM_APP_WM: u32 = WM_APP + 19;
/// A pinned window closed and the user asked to hear about it. Read it with `take_closed`.
pub const WM_APP_PIN_CLOSED: u32 = WM_APP + 24;
/// A window stamped for a workspace was moved, snapped, maximized or minimized: the
/// stamp's workspace key in wParam.
pub const WM_APP_WS_MOVED: u32 = WM_APP + 26;
/// A command for the thread, boxed in lParam.
const WM_APP_COMMAND: u32 = WM_APP + 21;
/// A window event a callback queued: the event in wParam, the window in lParam.
const WM_APP_EVENT: u32 = WM_APP + 22;
/// How often to look for another window manager.
const CHECK_EVERY_MS: u32 = 30_000;
/// Apps such as Chromium and Windows Terminal set their own border colour, often on
/// focus. After a focus change Seam paints again this long after, and once more later.
const REPAINT_FIRST_MS: u32 = 100;
const REPAINT_SECOND_MS: u32 = 500;
/// While a pin exists or the focus border is on, how often to paint them again.
const REFRESH_EVERY_MS: u32 = 2_000;
/// The first Windows 11 build, the first with `DWMWA_BORDER_COLOR`.
const BORDER_BUILD: u32 = 22000;
/// A pinned window that closes this soon after a workspace switch asked it to was closed by Seam.
const ASKED_TO_CLOSE: Duration = Duration::from_secs(30);
/// Chromium, Electron and Office resize themselves right after they show, so
/// auto-apply waits this long before it places a window that opened (§11.1).
const SETTLE: Duration = Duration::from_millis(150);
/// A stamped window's moves come as a burst (a drag, a snap's animation), so they are
/// told once it has been still this long.
const RELOCATED_QUIET_MS: u32 = 100;

/// Who a pinned window was, kept from when it was pinned, because a window that
/// is gone can't be asked (D119).
#[derive(Clone, Debug)]
pub struct Pinned {
    pub exe: String,
    pub aumid: Option<String>,
    pub class: String,
    pub title: String,
    /// `Seam_pin` bits.
    pub bits: usize,
    /// When a workspace switch asked it to close.
    asked_to_close: Option<Instant>,
}

impl Pinned {
    fn of(h: HWND, bits: usize) -> Pinned {
        Pinned {
            exe: winutil::exe_path(h).unwrap_or_default(),
            aumid: winutil::aumid(h),
            class: winutil::class_name(h),
            title: winutil::title(h),
            bits,
            asked_to_close: None,
        }
    }
}

pub enum Command {
    /// Who to tell about another window manager, whether the focus border is on,
    /// the config values of the two border colours, and whether to say when a
    /// pinned window closes.
    Configure {
        owner: isize,
        focus_border: bool,
        focus_color: String,
        pin_color: String,
        notify_close: bool,
    },
    Pin {
        kind: PinKind,
        on: Option<bool>,
        hwnd: isize,
        reply: Option<Sender<Reply>>,
    },
    Release {
        reply: Option<Sender<Reply>>,
    },
    /// Windows' accent colour changed.
    AccentChanged,
    /// A workspace switch is about to ask this window to close.
    Closing {
        hwnd: isize,
    },
    /// Send every top-level window that is shown to this switch, until it stops
    /// listening or a newer one asks.
    WatchShown {
        switch: u64,
        tx: Sender<isize>,
    },
    UnwatchShown {
        switch: u64,
    },
    /// Whether a workspace is in use on this desk, so moves of its windows count as
    /// drift, and whether windows that open are put in place (auto-apply).
    Workspace {
        active: bool,
        auto_apply: bool,
    },
    /// Whether any app rule exists, so windows that open are put in place by it.
    AppRules {
        on: bool,
    },
    /// Spotlight is on, so focus changes and minimizes are told to it.
    Spotlight {
        on: bool,
    },
}

static DETECTED: Mutex<Option<&'static str>> = Mutex::new(None);
static CLOSED: Mutex<Option<Pinned>> = Mutex::new(None);
static THREAD: OnceLock<u32> = OnceLock::new();

/// The window manager found running at the last check, by its State name.
pub fn detected() -> Option<&'static str> {
    *DETECTED.lock().unwrap_or_else(|e| e.into_inner())
}

/// The pinned window that closed last, once.
pub fn take_closed() -> Option<Pinned> {
    CLOSED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Hand a command to the thread. Only an answer it can't post goes back from here.
fn send(command: Command) {
    let raw = Box::into_raw(Box::new(command));
    let id = wm_thread();
    unsafe {
        if id != 0 && PostThreadMessageW(id, WM_APP_COMMAND, WPARAM(0), LPARAM(raw as isize)).is_ok() {
            return;
        }
        let command = Box::from_raw(raw);
        log::info!("wm thread unavailable");
        if let Command::Pin { reply: Some(tx), .. } | Command::Release { reply: Some(tx) } = *command {
            let _ = tx.send(Err(RpcError::new(api::BUSY, "Couldn't reach the window thread")));
        }
    }
}

pub fn configure(owner: isize, focus_border: bool, focus_color: &str, pin_color: &str, notify_close: bool) {
    let (focus_color, pin_color) = (focus_color.into(), pin_color.into());
    send(Command::Configure { owner, focus_border, focus_color, pin_color, notify_close });
}

/// Seam is about to close this window, so its closing isn't news.
pub fn closing(hwnd: isize) {
    send(Command::Closing { hwnd });
}

pub fn pin(kind: PinKind, on: Option<bool>, hwnd: isize, reply: Option<Sender<Reply>>) {
    send(Command::Pin { kind, on, hwnd, reply });
}

pub fn release(reply: Option<Sender<Reply>>) {
    send(Command::Release { reply });
}

pub fn accent_changed() {
    send(Command::AccentChanged);
}

/// Windows as they are shown, for a switch waiting on the apps it started. The
/// hook for them exists only while a switch listens, so the thread is idle otherwise.
pub fn watch_shown(switch: u64) -> Receiver<isize> {
    let (tx, rx) = channel();
    send(Command::WatchShown { switch, tx });
    rx
}

pub fn unwatch_shown(switch: u64) {
    send(Command::UnwatchShown { switch });
}

/// The hooks for drift and auto-apply exist only while a workspace is in use.
pub fn workspace(active: bool, auto_apply: bool) {
    send(Command::Workspace { active, auto_apply });
}

/// The hooks for opened windows also exist while any app rule does.
pub fn app_rules(on: bool) {
    send(Command::AppRules { on });
}

/// The hooks for spotlight exist only while it is on.
pub fn spotlight(on: bool) {
    send(Command::Spotlight { on });
}

/// Release all on a clean exit. Waits for it, but not for long.
pub fn release_before_exit() {
    let (tx, rx) = channel();
    release(Some(tx));
    let _ = rx.recv_timeout(Duration::from_secs(1));
}

/// The thread's id, starting it on first use. Zero if it couldn't start.
fn wm_thread() -> u32 {
    *THREAD.get_or_init(|| {
        let (ready, wait) = channel();
        let spawned = std::thread::Builder::new().name("seam-wm".into()).spawn(move || unsafe {
            let mut msg = MSG::default();
            // Creates this thread's queue, so posting to it can't fail once `ready` is sent.
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let _ = ready.send(GetCurrentThreadId());
            // A pinned window's AUMID comes from its property store.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut wm = Wm::default();
            wm.adopt();
            wm.check();
            wm.check_timer = SetTimer(None, 0, CHECK_EVERY_MS, None);
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                match msg.message {
                    WM_APP_COMMAND => wm.run(*Box::from_raw(msg.lParam.0 as *mut Command)),
                    WM_APP_EVENT => wm.event(msg.wParam.0 as u32, HWND(msg.lParam.0 as *mut _)),
                    WM_TIMER if msg.wParam.0 == wm.check_timer => {
                        wm.check();
                        wm.write_stats();
                    }
                    WM_TIMER => wm.timer(msg.wParam.0),
                    _ => {}
                }
            }
        });
        spawned.ok().and_then(|_| wait.recv().ok()).unwrap_or(0)
    })
}

/// Runs on the thread that installed the hook. It keeps only whole windows,
/// asks Windows nothing, and queues the event for the loop.
unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    unsafe {
        // Child 0 is the window itself (CHILDID_SELF).
        if object != OBJID_WINDOW.0 || child != 0 {
            return;
        }
        if let Some(&id) = THREAD.get() {
            let _ = PostThreadMessageW(id, WM_APP_EVENT, WPARAM(event as usize), LPARAM(hwnd.0 as isize));
        }
    }
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

fn set_border_colour(h: HWND, colour: u32) -> bool {
    unsafe { DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR, &colour as *const u32 as _, 4).is_ok() }
}

fn reset_border(h: HWND) {
    if unsafe { IsWindow(Some(h)) }.as_bool() {
        set_border_colour(h, DWMWA_COLOR_DEFAULT);
    }
    winutil::mark_border(h, None);
}

/// Windows 11 has the border colour. Elsewhere the focus border does nothing.
fn borders_supported() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| winver::build().parse::<u32>().is_ok_and(|b| b >= BORDER_BUILD))
}

/// An app's own window, which Seam may pin or colour. Never Seam's, and never
/// one Windows won't let Seam change.
fn ours_to_touch(h: HWND) -> Result<(), &'static str> {
    if h.is_invalid() || !unsafe { IsWindow(Some(h)) }.as_bool() {
        return Err("No window has focus");
    }
    if !winutil::is_capturable(h, std::process::id()) || wincontrol::is_seam(h) {
        return Err("That window can't be pinned");
    }
    if winutil::is_elevated_over_us(h) {
        return Err("That window runs as administrator");
    }
    Ok(())
}

fn is_topmost(h: HWND) -> bool {
    unsafe { GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

/// The only place Seam uses `HWND_TOPMOST`: an explicit pin, and taking it back.
fn set_topmost(h: HWND, on: bool) {
    let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
    unsafe {
        let _ = SetWindowPos(h, Some(after), 0, 0, 0, 0, flags);
    }
}

/// The first window manager from `OTHER_WMS` that is running.
fn find_other_wm() -> Option<&'static str> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more && found.is_none() {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            found = OTHER_WMS.iter().find(|(name, _, _)| *name == exe).map(|(_, id, _)| *id);
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
        found
    }
}

/// Event timing since the last stats line. Only kept while logging.
#[derive(Default)]
struct Stats {
    events: u64,
    total_ns: u64,
    max_ns: u64,
}

#[derive(Default)]
struct Wm {
    owner: isize,
    focus_border: bool,
    /// The window Seam coloured last.
    bordered: Option<isize>,
    /// The focus and pin border colours, as COLORREFs, or None to leave that border
    /// alone. A colour that follows Windows' accent is read again when the setting
    /// changes or the accent does.
    focus_colour: Option<u32>,
    pin_colour: Option<u32>,
    focus_accent: bool,
    pin_accent: bool,
    /// Windows that carry `Seam_pin`.
    pinned: Vec<isize>,
    /// Who each of them is, for the notice when one closes.
    known: HashMap<isize, Pinned>,
    notify_close: bool,
    foreground_hook: Option<HWINEVENTHOOK>,
    destroy_hook: Option<HWINEVENTHOOK>,
    show_hook: Option<HWINEVENTHOOK>,
    name_hook: Option<HWINEVENTHOOK>,
    movesize_hook: Option<HWINEVENTHOOK>,
    minimize_hook: Option<HWINEVENTHOOK>,
    location_hook: Option<HWINEVENTHOOK>,
    /// The switch waiting for windows to show, and where to send them.
    shown_to: Option<(u64, Sender<isize>)>,
    /// A workspace is in use, and windows that open are put in place.
    workspace_active: bool,
    auto_apply: bool,
    /// Any app rule exists, so windows that open are put in place by it.
    app_rules: bool,
    /// Spotlight is on.
    spotlight: bool,
    /// Windows that were there before, or were already looked at, so showing or
    /// renaming one isn't an open. Kept while auto-apply runs.
    seen_before: HashSet<isize>,
    /// Windows that opened, and when they have settled enough to be placed.
    settling: Vec<(isize, Instant)>,
    settle_timer: usize,
    /// Workspace keys of stamped windows that moved, told when the burst is over.
    relocated: Vec<u32>,
    relocated_timer: usize,
    /// The window that had focus at the last foreground event.
    foreground: Option<isize>,
    /// Windows to paint again after a focus change, and which of the two repaints is next.
    repaint: Vec<isize>,
    repaint_second: bool,
    check_timer: usize,
    repaint_timer: usize,
    refresh_timer: usize,
    other_wm: Option<&'static str>,
    stats: Stats,
}

/// Install or remove one narrow hook, for the events `first` to `last`.
fn sync_hook(slot: &mut Option<HWINEVENTHOOK>, first: u32, last: u32, wanted: bool) {
    match (wanted, *slot) {
        (true, None) => unsafe {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let hook = SetWinEventHook(first, last, None, Some(on_event), 0, 0, flags);
            if hook.0.is_null() {
                log::info!("wm hook {first:#x} not installed");
            } else {
                *slot = Some(hook);
            }
        },
        (false, Some(hook)) => {
            let _ = unsafe { UnhookWinEvent(hook) };
            *slot = None;
        }
        _ => {}
    }
}
