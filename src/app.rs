//! Tray app: owns config, tracks monitor changes, runs the hook and the overlay.

mod commands;
mod messages;
mod pause;
mod pointer;
mod report;
mod session;
mod share;
mod signin;
mod start;
mod tile;
mod tray_icon;
mod update;
mod workspaces;

use crate::api_server::{self, Queued};
use crate::config::{Config, MAX_EDGE_RESISTANCE_PX, MAX_GLUE_PUSH_PX, signature};
use crate::desk::{self, has_neighbours, resolve};
use crate::hook;
use crate::layout::{Link, Live};
use crate::live::{WM_APP_DETECTED, WM_APP_DRIFT};
use crate::log;
use crate::monitors::{self, Mon};
use crate::snap::{self, WM_APP_REFIT, WM_APP_SNAP_BLOCKED};
use crate::standby::{self, WM_APP_STANDBY};
use crate::switcher::{
    self, AppRules, CaptureJob, LiveWorkspace, Screen as SwitchScreen, SwitchJob, WM_APP_CAPTURED, WM_APP_SESSION,
    WM_APP_SWITCHED,
};
use crate::tray::{CLASS, WM_APP_API, WM_APP_LINEUP, WM_APP_WINDOWS_CHANGED};
use crate::wm::{Pinned, WM_APP_WS_MOVED};
use crate::{
    altdrag, arrange, dividers, ease, focus_only, hotkey, identify, live, overlay, quicklayout, showlayout, spotlight,
    wincontrol, wm,
};
use commands::{align_finished, busy, drain_api, patch_config, refused};
use messages::wndproc;
use pause::{set_paused, standby_check, standby_configure};
use pointer::{lineup_done, set_cursor_easing, set_remap, start_lineup, warn_about_ease};
use report::{publish, refresh, send_state_event, snapshot};
use seam::api::{self, Client, Command, FocusScope, GroupVerb, PinKind, Reply, RpcError, WindowVerb};
use seam::config::{RestoreApps, TrayClick};
use seam::direction::Dir;
use seam::state::{
    self, AlignResult, DeskRef, HotkeyError, Remap, Screen, ScreenRect, SessionRef, Snapshot, State, UpdateState,
    WmState, WorkspaceRef,
};
use seam::workspace::{self, Rule, SESSION_ID, Session, Workspace};
use seam::zones;
use seam::{autostart, install, package, process, restart_apps, updates};
use serde_json::{Value, json};
use session::{
    check_restart_apps, end_session, offer_session, pin_closed, query_end_session, reopen_closed, restore_session,
    saved_session, session_captured, start_session_capture, sync_restart_apps, windows_changed,
};
use signin::{SignInRun, arm, begin, cancel_sign_in, open_offered, sign_in_tick};
pub use start::run;
use std::cell::RefCell;
use std::os::windows::process::CommandExt;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tile::{fit_group, start_tile, tile_finished, undo_tile};
use tray_icon::{add_icon, make_icon, notify, show_menu, tray_data, update_tip};
use update::{
    arm_auto_install, auto_install, check_for_updates, clean_up_after_update, read_update_state, update_read,
};
use wincontrol::tile::At;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, HOT_KEY_MODIFIERS, LASTINPUTINFO, RegisterHotKey, UnregisterHotKey,
    VIRTUAL_KEY, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use workspaces::{
    apply_hotkeys, aside, capture_job, captured, choose_in_use, desk_workspaces, detected_in_use, drift_checked,
    drift_due, forget_deleted, live_of, migrate_hotkeys, other_wm_changed, quick_save, set_default, set_spotlight,
    show_spotlight, snap_back, start_capture, start_focus_only, start_switch, start_switch_job, start_window,
    switch_back, switched, sync_live, undo_quick_save, update_workspace, ws_moved,
};
use workspaces::{start_group, window_planned};

const WM_APP_TRAY: u32 = WM_APP + 1;
/// A worker finished `arrangement.align`.
const WM_APP_ALIGNED: u32 = WM_APP + 11;
/// A worker read the updater's state file.
const WM_APP_UPDATE: u32 = WM_APP + 13;
const TIMER_REFRESH: usize = 1;
/// Retries the quick layout overlay's picks the app was too busy for (menu open).
const TIMER_QUICK: usize = 40;
/// Retries a divider menu's choice the app was too busy for (menu open).
const TIMER_DIVIDER: usize = 41;
/// Windows silently removes low-level hooks that ever time out, so re-arm periodically.
const TIMER_REARM: usize = 2;
/// Retries API commands the app was too busy for (menu open).
const TIMER_API: usize = 4;
/// Sends a `state.changed` that came too soon after the last one.
const TIMER_STATE: usize = 5;
const STATE_EVENT_GAP: Duration = Duration::from_millis(100);
/// Starts the update check, 2 minutes after launch and then every 24 hours.
const TIMER_UPDATE: usize = 6;
/// Retries an automatic install until the PC is idle.
const TIMER_AUTO_INSTALL: usize = 7;
/// Retries taking in another window manager's news the app was too busy for (menu open).
const TIMER_OTHER_WM: usize = 8;
/// Saves the last session a while after Seam last pinned, placed or switched (D120).
const TIMER_SESSION: usize = 9;
const SESSION_SAVE_AFTER_MS: u32 = 30_000;
/// Offers the last session once the desktop has settled after sign-in.
const TIMER_SESSION_OFFER: usize = 10;
const SESSION_OFFER_AFTER_MS: u32 = 5_000;
/// Tries to show the offer while the shell isn't ready for notifications yet.
const SESSION_OFFER_TRIES: u8 = 12;
/// Looks whether the active workspace drifted, a moment after its windows last moved
/// or it was switched to, so a drag or a switch is checked once.
const TIMER_DRIFT: usize = 11;
const DRIFT_AFTER_MS: u32 = 400;
/// How long the end of a Windows session waits for the last session's capture.
const SESSION_END_WAIT: Duration = Duration::from_secs(2);
/// How long a restore whose apps Windows reopens waits for their windows (D128).
const ARRANGE_WAIT: Duration = Duration::from_secs(60);
const FIRST_UPDATE_CHECK_MS: u32 = 2 * 60 * 1000;
const UPDATE_CHECK_EVERY_MS: u32 = 24 * 60 * 60 * 1000;
const AUTO_INSTALL_RETRY_MS: u32 = 60 * 1000;
const IDLE_BEFORE_INSTALL_MS: u32 = 2 * 60 * 1000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// How long the Store copy waits for a GitHub copy it asked to quit.
const TAKE_OVER_WAIT_MS: u32 = 25_000;
const HOTKEY_LINE_UP: i32 = 1;
const HOTKEY_PAUSE: i32 = 2;
const HOTKEY_NEXT_ZONE: i32 = 3;
const HOTKEY_PREV_ZONE: i32 = 4;
const HOTKEY_CAPTURE: i32 = 5;
const HOTKEY_PIN_TOP: i32 = 6;
const HOTKEY_PIN_STICKY: i32 = 7;
const HOTKEY_RELEASE: i32 = 8;
const HOTKEY_SNAP_BACK: i32 = 9;
/// The twelve directional shortcuts, in `Hotkeys::directional` order, from here up.
const HOTKEY_WINDOW: i32 = 10;
const WINDOW_KEYS: i32 = 12;
/// After the directional ones.
const HOTKEY_QUICK_SAVE: i32 = 22;
/// Grow, shrink and Priority on the focused window's group of zones.
const HOTKEY_GROW: i32 = 30;
const HOTKEY_SHRINK: i32 = 31;
const HOTKEY_PRIORITY: i32 = 32;
const HOTKEY_SWITCH_BACK: i32 = 23;
const HOTKEY_TILE_SCREEN: i32 = 50;
const HOTKEY_QUICK_LAYOUT: i32 = 55;
const HOTKEY_FOCUS_ONLY: i32 = 40;
const HOTKEY_SPOTLIGHT: i32 = 41;
/// The size menu: share or take out the focused snapped window.
const HOTKEY_DIVIDER_MENU: i32 = 60;
/// Pause or resume everything Seam does.
const HOTKEY_PAUSE_SEAM: i32 = 65;
/// Stretch the focused window's group to fill its screen.
const HOTKEY_FIT_GROUP: i32 = 66;
/// One hotkey id per workspace on the desk, from here up.
const HOTKEY_WORKSPACE: i32 = 100;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const ID_LINEUP: usize = 1;
const ID_ENABLED: usize = 2;
const ID_RELEASE: usize = 3;
const ID_OPEN: usize = 6;
const ID_QUIT: usize = 9;
const ID_SWITCH_BACK: usize = 10;
/// Pause or resume everything Seam does.
const ID_PAUSE_SEAM: usize = 12;
/// Tile with one screen, or with several when the submenu couldn't be made.
const ID_TILE: usize = 11;
/// Menu ids of the screens in the Tile windows submenu, from here up to the workspaces.
const ID_TILE_SCREEN: usize = 20;
/// Menu ids of the workspaces in the submenu, from here up.
const ID_WORKSPACE: usize = 100;

/// What clicking a balloon does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BalloonAction {
    None,
    LineUp,
    OpenPanel(&'static str),
    /// Start the app of the pinned window that closed (D119).
    Reopen,
    /// Open the workspace offered at sign-in.
    OpenWorkspace,
    /// Restore the last session offered at sign-in (D120).
    RestoreSession,
    /// Put back what the last quick save changed.
    UndoQuickSave,
    /// Put back the windows and layout of the last tile.
    UndoTile,
}

/// Writes config.json on its own thread, so the main thread never does file I/O.
struct Saver {
    queue: Sender<Config>,
    thread: JoinHandle<()>,
}

impl Saver {
    fn start() -> Option<Saver> {
        let (queue, saves) = channel::<Config>();
        let spawned = std::thread::Builder::new().name("seam-save".into()).spawn(move || {
            while let Ok(mut latest) = saves.recv() {
                while let Ok(newer) = saves.try_recv() {
                    latest = newer;
                }
                latest.save();
            }
        });
        Some(Saver { queue, thread: spawned.ok()? })
    }

    /// Let queued saves finish.
    fn finish(self) {
        drop(self.queue);
        let _ = self.thread.join();
    }
}

/// A `workspace.capture` running on a worker.
struct Capturing {
    /// The API client waiting for the id. A hotkey has none.
    reply: Option<Sender<Reply>>,
    name: Option<String>,
    /// The workspace to update, if this is "Update from screen".
    into: Option<String>,
    /// Only this row of `into` takes its window's place ("Where it is now").
    row: Option<usize>,
    /// The window "Add an app" picked: the row only takes its folder or page.
    pick: Option<api::Pick>,
    /// The desk it was started on.
    sig: String,
    /// A quick save (§7): it says so, can be undone, and a new workspace becomes the one in use.
    quick: bool,
}

/// How long a quick save can be undone.
const QUICK_UNDO_FOR: Duration = Duration::from_secs(8);

/// What Undo puts back after a quick save.
struct QuickUndo {
    sig: String,
    id: String,
    /// The workspace as it was, or `None` when the quick save created it.
    before: Option<Value>,
    /// The workspace as the save wrote it. Undo only goes ahead while it is still that.
    after: Value,
    /// For each row now, the row of `before` it was, when an Update from screen removed
    /// rows: the place rows' windows and the live stamps follow it back.
    from: Option<Vec<Option<usize>>>,
    /// The workspace in use before a created one took over.
    in_use: Option<String>,
    until: Instant,
}

/// What Undo puts back after Tile windows.
struct TileUndo {
    sig: String,
    /// The screen's id.
    screen: String,
    /// The layout the tile or a pick gave the screen, which Undo changes back if it is still there.
    changed: Option<wincontrol::tile::Change>,
    /// Each moved window, put back unless it has moved since.
    back: Vec<wincontrol::tile::Back>,
    /// Windows a split renumbered, given their old zone numbers back.
    keep: Vec<snap::Zoned>,
    /// The ad-hoc group Fit group to screen stretched, and its area before.
    group: Option<(u64, zones::Rect)>,
    until: Instant,
}

/// An `arrangement.align` running on a worker.
struct Aligning {
    reply: Sender<Reply>,
    result: Option<Result<bool, String>>,
}

struct App {
    hwnd: HWND,
    icon: HICON,
    /// The icon while Seam stands by.
    quiet_icon: HICON,
    cfg: Config,
    mons: Vec<Mon>,
    sig: String,
    links: Vec<Link>,
    taskbar_created: u32,
    balloon_action: BalloonAction,
    /// Warn once per run if Windows may stop the pointer before Seam sees it.
    ease_warned: bool,
    /// Windows' "Ease cursor movement between displays" as last read, `None` where it doesn't exist.
    cursor_easing: Option<bool>,
    /// What the last arrangement attempt did, for the panel.
    last_align: Option<AlignResult>,
    saver: Option<Saver>,
    /// Lets the panel prove it's the one the tray started.
    token: String,
    hotkey_errors: Vec<HotkeyError>,
    /// The shortcuts registered now (id, action, combo), to skip work when they haven't changed.
    bound: Vec<(i32, String, Option<String>)>,
    /// The workspace last switched to, if it's still on this desk.
    workspace: Option<String>,
    /// A window of that workspace is moved or minimized, so Snap back has work to do.
    drifted: bool,
    /// Keys of workspaces whose stamped windows moved, waiting for the drift timer.
    moved: Vec<u32>,
    /// A switch finished, so the drift timer looks again whatever moved.
    recheck: bool,
    capturing: Option<Capturing>,
    /// The last quick save, while it can still be undone.
    quick_undo: Option<QuickUndo>,
    /// The last tile, while it can still be undone.
    tile_undo: Option<TileUndo>,
    clients: Vec<String>,
    last_state_event: Option<Instant>,
    /// The Store copy: Windows updates it, so the updater stays off.
    packaged: bool,
    /// What the updater left in its state file.
    update: Option<UpdateState>,
    /// The version an automatic install was started for, so it runs once.
    install_started: Option<String>,
    /// The updater's last attempt ended in an error, so nothing installs by itself.
    update_failed: bool,
    /// Another window manager the seam-wm thread found running.
    other_wm: Option<&'static str>,
    /// The user asked to quit, so pins and borders go with the tray. An update
    /// or the Store copy taking over keeps them for the next tray to adopt.
    release_on_exit: bool,
    /// The pinned window that closed last, for the notice's click.
    reopen: Option<Pinned>,
    /// The last session as it was when it was offered, for the notice's click.
    session_offer: Option<Workspace>,
    session_offer_tries: u8,
    /// The workspace offered at sign-in, for the notice's click.
    sign_in_offer: Option<(String, String)>,
    /// The sign-in action of this start, while it waits.
    sign_in_run: Option<SignInRun>,
    /// A last-session capture is running, for the desk it started on.
    session_capture: Option<String>,
    /// Windows asked whether the session may end, and the capture for it has started.
    session_ending: bool,
    /// Windows' Restart apps was on when Seam last looked (D128).
    restart_apps: bool,
    /// Spotlight was on when Seam began to stand by, so it comes back after.
    spotlight_resume: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Hands a finished `arrangement.align` from its worker to the main thread.
static ALIGNING: std::sync::Mutex<Option<Aligning>> = std::sync::Mutex::new(None);

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        out[i] = c;
    }
    out
}

/// Whether the pointer remap runs: it is on, and Seam isn't paused (D390).
fn remapping(app: &App) -> bool {
    app.cfg.enabled && !standby::paused()
}

/// (Re)install the hook if it should be running. Unhooking first guarantees a
/// hook Windows dropped behind our back comes back. Edge resistance needs it at
/// every border, lined up or not. Paused, the pointer is left to Windows entirely.
fn arm_hook(app: &App) {
    hook::enable(false);
    let resist = app.cfg.pointer.edge_resistance_px > 0 && app.mons.len() > 1;
    hook::enable(remapping(app) && (!app.links.is_empty() || resist));
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn persist(app: &App) {
    if let Some(saver) = &app.saver {
        let _ = saver.queue.send(app.cfg.clone());
    }
}

/// Resolve the saved layout against the monitors we know about, (re)arm the
/// hook. Cheap enough for the main thread to do for a setting change.
fn relink(app: &mut App) {
    app.links = resolve(&app.mons, app.cfg.desks.get(&app.sig).map(|d| d.borders.as_slice()));
    let pointer = &app.cfg.pointer;
    let remapping = remapping(app);
    hook::set_live(Live {
        rects: app.mons.iter().map(|m| m.rect).collect(),
        // Paused, the hook remaps nothing and holds nothing.
        links: if remapping { app.links.clone() } else { Vec::new() },
        edge_resistance: if remapping { pointer.edge_resistance_px.min(MAX_EDGE_RESISTANCE_PX) as i32 } else { 0 },
        glue_push: pointer.glue_push_px.min(MAX_GLUE_PUSH_PX) as i32,
    });
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    let windows = &app.cfg.windows;
    let owner = app.hwnd.0 as isize;
    // The focus border waits with the rest while Seam stands by.
    let border = windows.focus_border && standby::reason().is_none();
    wm::configure(owner, border, &windows.focus_color, &windows.pin_color, windows.notify_pinned_close);
    // The hook is in only while it's wanted, and not while Seam stands by or steps aside.
    let alt_drag = windows.alt_drag.on && standby::reason().is_none() && aside(app).is_none();
    altdrag::configure(owner, alt_drag, &windows.alt_drag.key, &app.cfg.zones.exclude);
    sync_live(app);
    arm_hook(app);
    update_tip(app);
}
/// Start the panel, which raises the open one if there is one.
fn open_panel(app: &mut App, page: Option<&str>) -> bool {
    let panel = std::env::current_exe().map(|exe| exe.with_file_name("seam-settings.exe"));
    // Lets an already open panel come to the front.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    // An open panel switches page itself. One that's starting reads `--page`.
    if let Some(page) = page {
        api_server::broadcast_to_panels("panel.show", &json!({ "page": page }));
    }
    let started = panel.and_then(|p| {
        let mut command = std::process::Command::new(p);
        command.args(["--token", &app.token]);
        if let Some(page) = page {
            command.args(["--page", page]);
        }
        command.spawn()
    });
    if started.is_err() {
        notify(app, "Can't open Seam", "seam-settings.exe needs to be next to seam.exe.", BalloonAction::None);
    }
    started.is_ok()
}

fn tray_click(app: &mut App) {
    match app.cfg.tray_click {
        TrayClick::Open => {
            open_panel(app, None);
        }
        TrayClick::LineUp => start_lineup(app),
    }
}
