//! The `seam-wm` thread: pins, the focus border, Release all, and the check for
//! another window manager (D106, D108, D110, §3.5). It has its own message loop,
//! like `seam-hook`, so WinEvent callbacks never run on the tray's thread. The
//! tray only posts commands. Answers go straight back to the API client, as
//! window control's do, and news for the tray comes as `WM_APP_WM`.
//!
//! Zone snapping keeps its own hook on the tray thread for now (v0.7 moves it).

use crate::log;
use crate::wincontrol;
use seam::api::{self, PinKind, Reply, RpcError};
use seam::config::ACCENT_RGB;
use seam::state::OTHER_WMS;
use seam::{winutil, winver};
use serde_json::json;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The window manager found running changed. Read it with `detected`.
pub const WM_APP_WM: u32 = WM_APP + 19;
/// A command for the thread, boxed in lParam.
const WM_APP_COMMAND: u32 = WM_APP + 21;
/// A window event a callback queued: the event in wParam, the window in lParam.
const WM_APP_EVENT: u32 = WM_APP + 22;
/// How often to look for another window manager.
const CHECK_EVERY_MS: u32 = 30_000;
/// The first Windows 11 build, the first with `DWMWA_BORDER_COLOR`.
const BORDER_BUILD: u32 = 22000;

pub enum Command {
    /// Who to tell about another window manager, and whether the focus border is on.
    Configure {
        owner: isize,
        focus_border: bool,
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
}

static DETECTED: Mutex<Option<&'static str>> = Mutex::new(None);
static THREAD: OnceLock<u32> = OnceLock::new();

/// The window manager found running at the last check, by its State name.
pub fn detected() -> Option<&'static str> {
    *DETECTED.lock().unwrap_or_else(|e| e.into_inner())
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

pub fn configure(owner: isize, focus_border: bool) {
    send(Command::Configure { owner, focus_border });
}

pub fn pin(kind: PinKind, on: Option<bool>, hwnd: isize, reply: Option<Sender<Reply>>) {
    send(Command::Pin { kind, on, hwnd, reply });
}

pub fn release(reply: Option<Sender<Reply>>) {
    send(Command::Release { reply });
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
            let mut wm = Wm::default();
            wm.adopt();
            wm.check();
            let _ = SetTimer(None, 0, CHECK_EVERY_MS, None);
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                match msg.message {
                    WM_APP_COMMAND => wm.run(*Box::from_raw(msg.lParam.0 as *mut Command)),
                    WM_APP_EVENT => wm.event(msg.wParam.0 as u32, HWND(msg.lParam.0 as *mut _)),
                    WM_TIMER => {
                        wm.check();
                        wm.write_stats();
                    }
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
    // Child 0 is the window itself (CHILDID_SELF).
    if object != OBJID_WINDOW.0 || child != 0 {
        return;
    }
    if let Some(&id) = THREAD.get() {
        let _ = PostThreadMessageW(id, WM_APP_EVENT, WPARAM(event as usize), LPARAM(hwnd.0 as isize));
    }
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

/// 0x00BBGGRR, from the 0xRRGGBB the panel uses.
fn colorref(rgb: u32) -> u32 {
    (rgb >> 16 & 0xFF) | (rgb & 0xFF00) | (rgb & 0xFF) << 16
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
    /// Windows that carry `Seam_pin`.
    pinned: Vec<isize>,
    foreground_hook: Option<HWINEVENTHOOK>,
    destroy_hook: Option<HWINEVENTHOOK>,
    other_wm: Option<&'static str>,
    stats: Stats,
}

/// Install or remove one narrow hook.
fn sync_hook(slot: &mut Option<HWINEVENTHOOK>, event: u32, wanted: bool) {
    match (wanted, *slot) {
        (true, None) => unsafe {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let hook = SetWinEventHook(event, event, None, Some(on_event), 0, 0, flags);
            if hook.0.is_null() {
                log::info!("wm hook {event:#x} not installed");
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

impl Wm {
    /// Only what a feature that is on needs: focus changes for the border,
    /// closed windows while anything is pinned.
    fn sync_hooks(&mut self) {
        sync_hook(&mut self.foreground_hook, EVENT_SYSTEM_FOREGROUND, self.focus_border && borders_supported());
        sync_hook(&mut self.destroy_hook, EVENT_OBJECT_DESTROY, !self.pinned.is_empty());
    }

    /// A tray that starts again keeps the pins it finds, and takes back borders
    /// a tray that stopped left behind.
    fn adopt(&mut self) {
        let mut reset = 0;
        for h in winutil::top_level_windows() {
            if winutil::pin_bits(h) != 0 {
                self.pinned.push(h.0 as isize);
            }
            if winutil::has_border_mark(h) {
                reset_border(h);
                reset += 1;
            }
        }
        if !self.pinned.is_empty() || reset > 0 {
            log::info!("wm start: {} pinned windows kept, {reset} borders reset", self.pinned.len());
        }
        self.sync_hooks();
    }

    fn run(&mut self, command: Command) {
        match command {
            Command::Configure { owner, focus_border } => {
                // A window manager found before the tray first said who it is.
                let first = self.owner == 0;
                self.owner = owner;
                if first && self.other_wm.is_some() {
                    self.tell_tray();
                }
                if focus_border == self.focus_border {
                    return;
                }
                self.focus_border = focus_border;
                self.sync_hooks();
                if focus_border {
                    self.border(unsafe { GetForegroundWindow() });
                } else if let Some(old) = self.bordered.take() {
                    reset_border(hwnd_of(old));
                }
            }
            Command::Pin { kind, on, hwnd, reply } => {
                let result = self.pin(kind, on, hwnd_of(hwnd));
                match (result, reply) {
                    (Ok(on), Some(tx)) => {
                        let _ = tx.send(Ok(json!({ "on": on })));
                    }
                    (Err(why), Some(tx)) => {
                        let _ = tx.send(Err(RpcError::new(api::REFUSED, why)));
                    }
                    (Ok(on), None) => log::info!("pin {kind:?}: {}", if on { "on" } else { "off" }),
                    (Err(why), None) => log::info!("pin {kind:?}: {why}"),
                }
            }
            Command::Release { reply } => {
                let released = self.release();
                log::info!("released {released} pinned windows");
                if let Some(tx) = reply {
                    let _ = tx.send(Ok(json!({ "ok": true, "released": released })));
                }
            }
        }
    }

    fn event(&mut self, event: u32, h: HWND) {
        let started = log::enabled().then(Instant::now);
        match event {
            EVENT_SYSTEM_FOREGROUND if self.focus_border => self.border(h),
            EVENT_OBJECT_DESTROY => {
                let key = h.0 as isize;
                self.pinned.retain(|p| *p != key);
                if self.bordered == Some(key) {
                    self.bordered = None;
                }
                if self.pinned.is_empty() {
                    self.sync_hooks();
                }
            }
            _ => {}
        }
        if let Some(started) = started {
            let ns = started.elapsed().as_nanos() as u64;
            let s = &mut self.stats;
            (s.events, s.total_ns, s.max_ns) = (s.events + 1, s.total_ns + ns, s.max_ns.max(ns));
        }
    }

    fn write_stats(&mut self) {
        let s = std::mem::take(&mut self.stats);
        if s.events > 0 {
            log::info!(
                "wm stats: {} events, avg {:.1}us, max {:.1}us",
                s.events,
                s.total_ns as f64 / s.events as f64 / 1e3,
                s.max_ns as f64 / 1e3
            );
        }
    }

    /// Move the border to the window that has focus now.
    fn border(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.bordered == Some(key) || !borders_supported() {
            return;
        }
        if let Some(old) = self.bordered.take() {
            reset_border(hwnd_of(old));
        }
        let colour = colorref(ACCENT_RGB);
        if ours_to_touch(h).is_ok() && set_border_colour(h, colour) {
            winutil::mark_border(h, Some(colour));
            self.bordered = Some(key);
        }
    }

    /// Turn a pin on, off, or over. Returns whether it is on now.
    fn pin(&mut self, kind: PinKind, on: Option<bool>, h: HWND) -> Result<bool, &'static str> {
        ours_to_touch(h)?;
        let bits = winutil::pin_bits(h);
        let bit = match kind {
            PinKind::Top => winutil::PIN_TOP,
            PinKind::Sticky => winutil::PIN_STICKY,
        };
        let was = bits & bit != 0;
        let want = on.unwrap_or(!was);
        if kind == PinKind::Top && want != was {
            // Seam only ever takes back a topmost it set, so it can't set one the app has.
            if want && is_topmost(h) {
                return Err("That window is already on top");
            }
            set_topmost(h, want);
        }
        let now = if want { bits | bit } else { bits & !bit };
        winutil::set_pin_bits(h, now);
        let key = h.0 as isize;
        self.pinned.retain(|p| *p != key);
        if now != 0 {
            self.pinned.push(key);
        }
        self.sync_hooks();
        Ok(want)
    }

    /// Take back every pin and border Seam set. Returns how many windows were pinned.
    fn release(&mut self) -> usize {
        let mut released = 0;
        for h in winutil::top_level_windows() {
            let bits = winutil::pin_bits(h);
            if bits != 0 {
                if bits & winutil::PIN_TOP != 0 {
                    set_topmost(h, false);
                }
                winutil::set_pin_bits(h, 0);
                released += 1;
            }
            if winutil::has_border_mark(h) {
                reset_border(h);
            }
        }
        self.pinned.clear();
        self.bordered = None;
        self.sync_hooks();
        released
    }

    /// Look for another window manager, and tell the tray when that changes.
    fn check(&mut self) {
        let found = find_other_wm();
        if found == self.other_wm {
            return;
        }
        self.other_wm = found;
        *DETECTED.lock().unwrap_or_else(|e| e.into_inner()) = found;
        self.tell_tray();
    }

    fn tell_tray(&self) {
        if self.owner != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_WM, WPARAM(0), LPARAM(0));
            }
        }
    }
}
