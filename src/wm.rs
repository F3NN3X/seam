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

impl Wm {
    /// Only what a feature that is on needs: focus changes for the border and for
    /// pins that apps may paint over, closed windows while anything is pinned,
    /// shown windows while a switch waits for them, moves, snaps and minimizes while a
    /// workspace is in use, opened windows while auto-apply runs or an app rule exists, focus changes and
    /// minimizes while spotlight is on, and the refresh timer while the first applies.
    fn sync_hooks(&mut self) {
        let painting = self.painting();
        let auto = self.opening();
        if auto && self.name_hook.is_none() {
            // What is there already never counts as opening.
            self.seen_before = winutil::top_level_windows().into_iter().map(|h| h.0 as isize).collect();
        } else if !auto {
            self.seen_before.clear();
            self.settling.clear();
            self.stop_settle_timer();
        }
        let (show, destroy) = (EVENT_OBJECT_SHOW, EVENT_OBJECT_DESTROY);
        let foreground = EVENT_SYSTEM_FOREGROUND;
        sync_hook(&mut self.foreground_hook, foreground, foreground, painting || self.spotlight);
        sync_hook(&mut self.destroy_hook, destroy, destroy, !self.pinned.is_empty() || auto);
        sync_hook(&mut self.show_hook, show, show, self.shown_to.is_some() || auto);
        sync_hook(&mut self.name_hook, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, auto);
        let active = self.workspace_active;
        sync_hook(
            &mut self.movesize_hook,
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MOVESIZEEND,
            active || self.app_rules,
        );
        let minimize = (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND);
        sync_hook(&mut self.minimize_hook, minimize.0, minimize.1, active || self.spotlight);
        // Win+arrow snaps and the maximize button send no drag or minimize event.
        let location = EVENT_OBJECT_LOCATIONCHANGE;
        sync_hook(&mut self.location_hook, location, location, active);
        if !active {
            self.relocated.clear();
            self.stop_relocated_timer();
        }
        match (painting, self.refresh_timer) {
            (true, 0) => self.refresh_timer = unsafe { SetTimer(None, 0, REFRESH_EVERY_MS, None) },
            (false, id) if id != 0 => {
                let _ = unsafe { KillTimer(None, id) };
                self.refresh_timer = 0;
                self.stop_repaint();
            }
            _ => {}
        }
    }

    /// The focus border or a pin colour is painted, so focus changes matter to it.
    fn painting(&self) -> bool {
        let wanted = (self.focus_border && self.focus_colour.is_some())
            || (!self.pinned.is_empty() && self.pin_colour.is_some());
        wanted && borders_supported()
    }

    fn auto_applying(&self) -> bool {
        self.workspace_active && self.auto_apply
    }

    /// Windows that open are looked at: auto-apply runs or an app rule exists.
    fn opening(&self) -> bool {
        self.auto_applying() || self.app_rules
    }

    fn stop_settle_timer(&mut self) {
        if self.settle_timer != 0 {
            let _ = unsafe { KillTimer(None, self.settle_timer) };
            self.settle_timer = 0;
        }
    }

    /// Wake when the next opened window has settled.
    fn arm_settle_timer(&mut self) {
        let Some(next) = self.settling.iter().map(|(_, at)| *at).min() else { return self.stop_settle_timer() };
        let ms = next.saturating_duration_since(Instant::now()).as_millis().max(1) as u32;
        self.settle_timer = unsafe { SetTimer(None, self.settle_timer, ms, None) };
    }

    /// A window showed or got its title. If it's new, it's placed once it has
    /// settled. A window that was there, a minimized one and one already stamped
    /// aren't opens. One that opens during a switch waits on the worker for it to end.
    fn opened(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.seen_before.contains(&key) {
            return;
        }
        // A window still hidden gets its title first. It counts once it shows.
        if !unsafe { IsWindowVisible(h) }.as_bool() {
            return;
        }
        self.seen_before.insert(key);
        if winutil::is_minimized(h) || winutil::ws_stamp(h).is_some() {
            return;
        }
        // A window that shows while a switch runs is the switch's, even if it settles after.
        if crate::switcher::switching() {
            crate::live::owned_by_switch(key);
        }
        self.settling.push((key, Instant::now() + SETTLE));
        self.arm_settle_timer();
    }

    /// Hand the windows that have settled to the worker that places them.
    fn settled(&mut self) {
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) = self.settling.drain(..).partition(|(_, at)| *at <= now);
        self.settling = waiting;
        for (key, _) in due {
            crate::live::opened(key);
        }
        self.arm_settle_timer();
    }

    /// A move or minimize of a window a workspace placed: tell the tray which workspace.
    fn moved(&self, h: HWND) {
        if let Some((key, _)) = winutil::ws_stamp(h) {
            self.tell_moved(key);
        }
    }

    fn tell_moved(&self, key: u32) {
        if self.owner != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_WS_MOVED, WPARAM(key as usize), LPARAM(0));
            }
        }
    }

    /// Some window moved or changed size. Only a stamped one counts, which its
    /// property alone says, and its workspace is told once the burst is over.
    fn relocated(&mut self, h: HWND) {
        let Some((key, _)) = winutil::ws_stamp(h) else { return };
        if !self.relocated.contains(&key) {
            self.relocated.push(key);
        }
        // The same id starts the wait again.
        self.relocated_timer = unsafe { SetTimer(None, self.relocated_timer, RELOCATED_QUIET_MS, None) };
    }

    /// The stamped windows that moved have been still a moment.
    fn relocated_due(&mut self) {
        self.stop_relocated_timer();
        for key in std::mem::take(&mut self.relocated) {
            self.tell_moved(key);
        }
    }

    fn stop_relocated_timer(&mut self) {
        if self.relocated_timer != 0 {
            let _ = unsafe { KillTimer(None, self.relocated_timer) };
            self.relocated_timer = 0;
        }
    }

    fn stop_repaint(&mut self) {
        if self.repaint_timer != 0 {
            let _ = unsafe { KillTimer(None, self.repaint_timer) };
            self.repaint_timer = 0;
        }
        self.repaint.clear();
    }

    /// A tray that starts again keeps the pins it finds, and takes back borders
    /// a tray that stopped left behind.
    fn adopt(&mut self) {
        let mut reset = 0;
        for h in winutil::top_level_windows() {
            let bits = winutil::pin_bits(h);
            if bits != 0 {
                self.pinned.push(h.0 as isize);
                self.known.insert(h.0 as isize, Pinned::of(h, bits));
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
            Command::Configure { owner, focus_border, focus_color, pin_color, notify_close } => {
                // A window manager found before the tray first said who it is.
                let first = self.owner == 0;
                self.owner = owner;
                self.notify_close = notify_close;
                if first && self.other_wm.is_some() {
                    self.tell_tray();
                }
                self.focus_accent = palette::colour(&focus_color, FOCUS_DEFAULT) == Colour::Accent;
                self.pin_accent = palette::colour(&pin_color, PIN_DEFAULT) == Colour::Accent;
                let focus_colour = palette::resolve(&focus_color, FOCUS_DEFAULT);
                let pin_colour = palette::resolve(&pin_color, PIN_DEFAULT);
                let (focus_changed, pin_changed) =
                    (focus_colour != self.focus_colour, first || pin_colour != self.pin_colour);
                (self.focus_colour, self.pin_colour) = (focus_colour, pin_colour);
                let was_on = self.focus_border;
                self.focus_border = focus_border;
                self.sync_hooks();
                if pin_changed {
                    // None gives the pinned windows' borders back. Otherwise this is also where
                    // a tray that starts again gives adopted pins their colour.
                    self.paint_pins();
                }
                if !focus_border {
                    if let Some(old) = self.bordered.take() {
                        reset_border(hwnd_of(old));
                    }
                } else if !was_on || focus_changed || pin_changed {
                    match self.focus_colour {
                        Some(_) if self.bordered.is_some() && !pin_changed => self.paint_focus(),
                        Some(_) => self.border(unsafe { GetForegroundWindow() }),
                        None => {
                            if let Some(old) = self.bordered.take() {
                                reset_border(hwnd_of(old));
                            }
                        }
                    }
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
            Command::AccentChanged => {
                if self.focus_accent {
                    self.focus_colour = palette::resolve("accent", FOCUS_DEFAULT);
                    self.paint_focus();
                }
                if self.pin_accent {
                    self.pin_colour = palette::resolve("accent", PIN_DEFAULT);
                    self.paint_pins();
                }
            }
            Command::Release { reply } => {
                let released = self.release();
                log::info!("released {released} pinned windows");
                if let Some(tx) = reply {
                    let _ = tx.send(Ok(json!({ "ok": true, "released": released })));
                }
            }
            Command::Closing { hwnd } => {
                if let Some(known) = self.known.get_mut(&hwnd) {
                    known.asked_to_close = Some(Instant::now());
                }
            }
            Command::WatchShown { switch, tx } => {
                self.shown_to = Some((switch, tx));
                self.sync_hooks();
            }
            Command::UnwatchShown { switch } => {
                if self.shown_to.as_ref().is_some_and(|(s, _)| *s == switch) {
                    self.shown_to = None;
                    self.sync_hooks();
                }
            }
            Command::Workspace { active, auto_apply } => {
                if (active, auto_apply) != (self.workspace_active, self.auto_apply) {
                    (self.workspace_active, self.auto_apply) = (active, auto_apply);
                    self.sync_hooks();
                    log::info!("wm: workspace in use {active}, auto-apply {}", self.auto_applying());
                }
            }
            Command::AppRules { on } => {
                if on != self.app_rules {
                    self.app_rules = on;
                    self.sync_hooks();
                    log::info!("wm: app rules {on}");
                }
            }
            Command::Spotlight { on } => {
                if on != self.spotlight {
                    self.spotlight = on;
                    self.sync_hooks();
                }
            }
        }
    }

    /// Run `work`, and count it in the stats while logging.
    fn timed(&mut self, work: impl FnOnce(&mut Self)) {
        let started = log::enabled().then(Instant::now);
        work(self);
        if let Some(started) = started {
            let ns = started.elapsed().as_nanos() as u64;
            let s = &mut self.stats;
            (s.events, s.total_ns, s.max_ns) = (s.events + 1, s.total_ns + ns, s.max_ns.max(ns));
        }
    }

    /// A repaint or refresh timer fired.
    fn timer(&mut self, id: usize) {
        if id == self.repaint_timer && id != 0 {
            self.timed(|wm| {
                wm.paint_again(&wm.repaint);
                if wm.repaint_second {
                    wm.stop_repaint();
                } else {
                    wm.repaint_second = true;
                    wm.repaint_timer =
                        unsafe { SetTimer(None, wm.repaint_timer, REPAINT_SECOND_MS - REPAINT_FIRST_MS, None) };
                }
            });
        } else if id == self.refresh_timer && id != 0 {
            self.timed(|wm| {
                wm.paint_again(&wm.pinned);
                wm.paint_again(&wm.bordered.into_iter().collect::<Vec<_>>());
            });
        } else if id == self.settle_timer && id != 0 {
            self.timed(Self::settled);
        } else if id == self.relocated_timer && id != 0 {
            self.timed(Self::relocated_due);
        }
    }

    /// Paint the right colour on each of these windows again, if they still exist.
    fn paint_again(&self, keys: &[isize]) {
        for key in keys {
            let h = hwnd_of(*key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                continue;
            }
            if self.pin_owns(*key) {
                self.paint_pin(h);
            } else if self.bordered == Some(*key) {
                self.paint_focus();
            }
        }
    }

    /// Focus moved: paint the new window and the pinned one it left again soon,
    /// in case their app paints its own border colour over ours. A newer focus
    /// change starts over.
    fn focus_moved(&mut self, key: isize) {
        let left = self.foreground.replace(key).filter(|prev| self.pinned.contains(prev));
        self.repaint = std::iter::once(key).chain(left).collect();
        self.repaint_second = false;
        self.repaint_timer = unsafe { SetTimer(None, self.repaint_timer, REPAINT_FIRST_MS, None) };
    }

    fn event(&mut self, event: u32, h: HWND) {
        let started = log::enabled().then(Instant::now);
        match event {
            EVENT_SYSTEM_FOREGROUND => {
                if self.painting() {
                    if self.focus_border {
                        self.border(h);
                    }
                    self.focus_moved(h.0 as isize);
                }
                if self.spotlight {
                    crate::spotlight::reorder();
                }
            }
            EVENT_OBJECT_DESTROY => {
                let key = h.0 as isize;
                self.seen_before.remove(&key);
                self.settling.retain(|(k, _)| *k != key);
                self.pinned.retain(|p| *p != key);
                if self.bordered == Some(key) {
                    self.bordered = None;
                }
                if let Some(gone) = self.known.remove(&key) {
                    self.pin_closed(gone);
                }
                if self.pinned.is_empty() {
                    self.sync_hooks();
                }
            }
            // Only top-level windows. A switch that has stopped listening lets the hook go.
            EVENT_OBJECT_SHOW if unsafe { GetAncestor(h, GA_ROOT) } == h => {
                let gone = self.shown_to.as_ref().is_some_and(|(_, tx)| tx.send(h.0 as isize).is_err());
                if gone {
                    self.shown_to = None;
                    self.sync_hooks();
                }
                if self.opening() {
                    self.opened(h);
                }
            }
            // Some apps never send a show for their window, only its title (§11.1).
            EVENT_OBJECT_NAMECHANGE if self.opening() && unsafe { GetAncestor(h, GA_ROOT) } == h => {
                self.opened(h);
            }
            // Dragging a window that hasn't been placed yet: the user is placing it.
            EVENT_SYSTEM_MOVESIZESTART => {
                let key = h.0 as isize;
                self.settling.retain(|(k, _)| *k != key);
            }
            EVENT_SYSTEM_MOVESIZEEND => self.moved(h),
            EVENT_SYSTEM_MINIMIZESTART | EVENT_SYSTEM_MINIMIZEEND => {
                if self.workspace_active {
                    self.moved(h);
                }
                if self.spotlight {
                    crate::spotlight::reorder();
                }
            }
            EVENT_OBJECT_LOCATIONCHANGE => self.relocated(h),
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
        // A pinned window keeps its pin colour (D117).
        let Some(colour) = self.focus_colour else { return };
        if !self.pin_owns(key) && ours_to_touch(h).is_ok() && set_border_colour(h, colour) {
            winutil::mark_border(h, Some(colour));
            self.bordered = Some(key);
        }
    }

    /// The pin colour paints this window: it is pinned and the pin colour isn't None.
    fn pin_owns(&self, key: isize) -> bool {
        self.pin_colour.is_some() && self.pinned.contains(&key)
    }

    /// Give the window that has the focus border the focus colour again.
    fn paint_focus(&self) {
        if let (Some(h), Some(colour)) = (self.bordered.map(hwnd_of), self.focus_colour)
            && set_border_colour(h, colour)
        {
            winutil::mark_border(h, Some(colour));
        }
    }

    fn paint_pin(&self, h: HWND) {
        if let (true, Some(colour)) = (borders_supported(), self.pin_colour)
            && set_border_colour(h, colour)
        {
            winutil::mark_border(h, Some(colour));
        }
    }

    /// Paint every pinned window in the pin colour. With none, give back the border
    /// Seam painted. A pinned window that had the focus colour now wears the pin's.
    fn paint_pins(&mut self) {
        for key in self.pinned.clone() {
            let h = hwnd_of(key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                continue;
            }
            if self.pin_colour.is_some() {
                if self.bordered == Some(key) {
                    self.bordered = None;
                }
                self.paint_pin(h);
            } else if winutil::has_border_mark(h) && self.bordered != Some(key) {
                reset_border(h);
            }
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
        self.known.remove(&key);
        if now != bits {
            self.tell_tray_of(WM_APP_WINDOWS_CHANGED);
        }
        if now != 0 {
            self.pinned.push(key);
            self.known.insert(key, Pinned::of(h, now));
            // The pin colour takes the border from the focus colour.
            if self.pin_colour.is_some() && self.bordered == Some(key) {
                self.bordered = None;
            }
            self.paint_pin(h);
        } else if bits != 0 {
            // With no pin colour there is nothing of the pin's to take back.
            if self.pin_colour.is_some() {
                reset_border(h);
            }
            // Still the focused window: it gets the focus colour back.
            if self.focus_border && unsafe { GetForegroundWindow() } == h {
                self.border(h);
            }
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
        self.known.clear();
        self.bordered = None;
        self.sync_hooks();
        if released > 0 {
            self.tell_tray_of(WM_APP_WINDOWS_CHANGED);
        }
        released
    }

    /// A pinned window closed. Seam asking it to, Release all and quitting are
    /// not news: the first is marked, and the other two leave nothing pinned.
    fn pin_closed(&self, gone: Pinned) {
        let asked = gone.asked_to_close.is_some_and(|at| at.elapsed() < ASKED_TO_CLOSE);
        if !self.notify_close || asked {
            return;
        }
        log::info!("a pinned window closed: {}", gone.class);
        // A second close before the first notice is used replaces it.
        *CLOSED.lock().unwrap_or_else(|e| e.into_inner()) = Some(gone);
        self.tell_tray_of(WM_APP_PIN_CLOSED);
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
        self.tell_tray_of(WM_APP_WM);
    }

    fn tell_tray_of(&self, message: u32) {
        if self.owner != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), message, WPARAM(0), LPARAM(0));
            }
        }
    }
}
