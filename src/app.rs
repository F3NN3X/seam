//! Tray app: owns config, tracks monitor changes, runs the hook and the overlay.

mod session;
mod update;

use crate::api_server::{self, Queued};
use crate::config::{signature, Config};
use crate::desk::{self, has_neighbours, resolve};
use crate::hook;
use crate::layout::{Link, Live};
use crate::live::WM_APP_DRIFT;
use crate::log;
use crate::monitors::{self, Mon};
use crate::snap::{self, WM_APP_REFIT, WM_APP_SNAP_BLOCKED};
use crate::switcher::{
    self, CaptureJob, LiveWorkspace, Screen as SwitchScreen, SwitchJob, WM_APP_CAPTURED, WM_APP_SESSION,
    WM_APP_SWITCHED,
};
use crate::tray::{self, CLASS, WM_APP_API, WM_APP_LINEUP, WM_APP_WINDOWS_CHANGED};
use crate::wm::{Pinned, WM_APP_WS_MOVED};
use crate::{arrange, ease, hotkey, identify, live, overlay, wincontrol, wm};
use seam::api::{self, Client, Command, PinKind, Reply, RpcError, WindowVerb};
use seam::config::{RestoreApps, TrayClick};
use seam::direction::Dir;
use seam::state::{
    self, AlignResult, DeskRef, HotkeyError, Remap, Screen, SessionRef, Snapshot, State, UpdateState, WmState,
    WorkspaceRef,
};
use seam::workspace::{self, Rule, Session, Workspace, SESSION_ID};
use seam::zones;
use seam::{autostart, install, package, process, restart_apps, updates};
use serde_json::{json, Value};
use session::{
    check_restart_apps, end_session, offer_session, pin_closed, query_end_session, reopen_closed, restore_session,
    saved_session, session_captured, start_session_capture, sync_restart_apps, windows_changed,
};
use std::cell::RefCell;
use std::os::windows::process::CommandExt;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use update::{
    arm_auto_install, auto_install, check_for_updates, clean_up_after_update, read_update_state, update_read,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, LASTINPUTINFO,
    VIRTUAL_KEY, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_APP_TRAY: u32 = WM_APP + 1;
/// A worker finished `arrangement.align`.
const WM_APP_ALIGNED: u32 = WM_APP + 11;
/// A worker read the updater's state file.
const WM_APP_UPDATE: u32 = WM_APP + 13;
const TIMER_REFRESH: usize = 1;
/// Windows silently removes low-level hooks that ever time out, so re-arm periodically.
const TIMER_REARM: usize = 2;
/// Retries a reload the app was too busy for (menu open).
const TIMER_RELOAD: usize = 3;
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
/// One hotkey id per workspace on the desk, from here up.
const HOTKEY_WORKSPACE: i32 = 100;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const ID_LINEUP: usize = 1;
const ID_ENABLED: usize = 2;
const ID_RELEASE: usize = 3;
const ID_OPEN: usize = 6;
const ID_QUIT: usize = 9;
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
    /// Restore the last session offered at sign-in (D120).
    RestoreSession,
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
    /// The desk it was started on.
    sig: String,
}

/// An `arrangement.align` running on a worker.
struct Aligning {
    reply: Sender<Reply>,
    result: Option<Result<bool, String>>,
}

struct App {
    hwnd: HWND,
    icon: HICON,
    cfg: Config,
    mons: Vec<Mon>,
    sig: String,
    links: Vec<Link>,
    taskbar_created: u32,
    balloon_action: BalloonAction,
    /// Warn once per run if Windows may stop the pointer before Seam sees it.
    ease_warned: bool,
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
    /// A last-session capture is running, for the desk it started on.
    session_capture: Option<String>,
    /// Windows asked whether the session may end, and the capture for it has started.
    session_ending: bool,
    /// Windows' Restart apps was on when Seam last looked (D128).
    restart_apps: bool,
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

/// Draw the tray icon: two screens of different sizes with a line running straight across.
fn make_icon() -> HICON {
    const N: usize = 32;
    let mut px = vec![0u32; N * N];
    let mut rect = |x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                px[y * N + x] = c;
            }
        }
    };
    let white = 0xFFF2F4F8;
    let amber = 0xFFFFB020;
    // Left (smaller) screen outline.
    rect(1, 10, 13, 11, white);
    rect(1, 22, 13, 23, white);
    rect(1, 10, 2, 23, white);
    rect(12, 10, 13, 23, white);
    // Right (bigger) screen outline.
    rect(16, 5, 30, 6, white);
    rect(16, 26, 30, 27, white);
    rect(16, 5, 17, 27, white);
    rect(29, 5, 30, 27, white);
    // The seam line, straight across both.
    rect(3, 16, 28, 17, amber);
    unsafe {
        let color = CreateBitmap(N as i32, N as i32, 1, 32, Some(px.as_ptr() as *const _));
        let mask_bits = [0u8; N * N / 8];
        let mask = CreateBitmap(N as i32, N as i32, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn tray_data(app: &App) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: app.hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: app.icon,
        ..Default::default()
    };
    let tip = if app.links.is_empty() {
        "Seam — screens not lined up yet".to_string()
    } else if !app.cfg.enabled {
        "Seam — paused".to_string()
    } else {
        format!("Seam — {} border(s) lined up", app.links.len())
    };
    nid.szTip = wide(&tip);
    nid
}

fn add_icon(app: &App) {
    unsafe {
        let mut nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

fn update_tip(app: &App) {
    unsafe {
        let nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Show a balloon/toast. Returns whether the shell accepted it.
fn notify(app: &mut App, title: &str, body: &str, click: BalloonAction) -> bool {
    app.balloon_action = click;
    unsafe {
        let mut nid = tray_data(app);
        nid.uFlags |= NIF_INFO;
        nid.szInfoTitle = wide(title);
        nid.szInfo = wide(body);
        nid.dwInfoFlags = NIIF_INFO;
        Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
    }
}

/// (Re)install the hook if it should be running. Unhooking first guarantees a
/// hook Windows dropped behind our back comes back.
fn arm_hook(app: &App) {
    hook::enable(false);
    hook::enable(app.cfg.enabled && !app.links.is_empty());
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
    hook::set_live(Live { rects: app.mons.iter().map(|m| m.rect).collect(), links: app.links.clone() });
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    let windows = &app.cfg.windows;
    let owner = app.hwnd.0 as isize;
    wm::configure(owner, windows.focus_border, &windows.focus_color, &windows.pin_color, windows.notify_pinned_close);
    sync_live(app);
    arm_hook(app);
    update_tip(app);
}

/// Re-read monitors, then relink.
fn refresh(app: &mut App) {
    if overlay::is_active() {
        return;
    }
    app.mons = monitors::enumerate();
    let sig = signature(app.mons.iter().map(|m| m.id.as_str()));
    let arrived = sig != app.sig && !app.mons.is_empty();
    app.sig = sig;
    if arrived {
        desk::note_active(&mut app.cfg, &app.sig, &app.mons, unix_now());
        persist(app);
        // Workspaces and their shortcuts belong to the desk.
        app.workspace = None;
        apply_hotkeys(app);
    }
    relink(app);
    // New set of screens that's never been lined up: ask once. Only mark it
    // seen if the shell actually showed the prompt (it may not be up at login).
    if app.links.is_empty()
        && app.cfg.ask_new_desks
        && has_neighbours(&app.mons)
        && !app.cfg.seen.contains(&app.sig)
        && notify(app, "New screen setup", "Click here to line up your screens.", BalloonAction::LineUp)
    {
        app.cfg.seen.push(app.sig.clone());
        persist(app);
    }
    warn_about_ease(app);
    publish(app);
    if arrived {
        let name = app.cfg.desks.get(&app.sig).map(|d| d.name.as_str()).unwrap_or_default();
        api_server::broadcast("desk.changed", &json!({ "sig": app.sig, "name": name }));
    }
}

fn snapshot(app: &App) -> Snapshot {
    let screens = app.mons.iter().enumerate().map(|(i, m)| Screen {
        number: i as u32 + 1,
        id: m.id.clone(),
        name: m.name.clone(),
        primary: m.primary,
    });
    let saved_name = app.cfg.desks.get(&app.sig).map(|d| d.name.clone()).filter(|n| !n.is_empty());
    let workspace = app.workspace.as_ref().and_then(|id| {
        let saved = app.cfg.desks.get(&app.sig)?;
        workspace::list(&saved.workspaces).into_iter().find(|w| w.id == *id)
    });
    let session = saved_session(app).map(|s| SessionRef { saved_at: s.saved_at, windows: s.windows.len() });
    let state = State {
        workspace: workspace.map(|w| WorkspaceRef { id: w.id, name: w.name, drifted: app.drifted }),
        session,
        seam_version: env!("CARGO_PKG_VERSION").into(),
        packaged: app.packaged,
        update: app.update.clone(),
        remap: Remap::of(app.cfg.enabled, app.links.len()),
        zones: if app.cfg.zones.enabled { state::Zones::On } else { state::Zones::Off },
        wm: WmState {
            other_wm: app.other_wm.map(String::from),
            manual_other_wm: app.cfg.windows.other_wm,
            focus_border: app.cfg.windows.focus_border,
            focus_color: app.cfg.windows.focus_color.clone(),
            pin_color: app.cfg.windows.pin_color.clone(),
        },
        desk: DeskRef { sig: app.sig.clone(), name: saved_name.unwrap_or_else(|| desk::default_name(&app.mons)) },
        screens: screens.collect(),
        borders: desk::border_states(&app.mons, &app.links),
        hotkey_errors: app.hotkey_errors.clone(),
        last_align: app.last_align.clone(),
        clients: app.clients.clone(),
        restart_apps: app.restart_apps,
    };
    Snapshot { state, config: serde_json::to_value(&app.cfg).unwrap_or_default() }
}

/// Tell API clients what the tray is doing now.
fn publish(app: &mut App) {
    if api_server::publish(snapshot(app), app.cfg.api.enabled) {
        send_state_event(app);
    }
}

/// At most one `state.changed` per 100 ms. The last change always goes out,
/// because the timer sends whatever the state is by then.
fn send_state_event(app: &mut App) {
    if app.last_state_event.is_some_and(|sent| sent.elapsed() < STATE_EVENT_GAP) {
        unsafe { SetTimer(Some(app.hwnd), TIMER_STATE, STATE_EVENT_GAP.as_millis() as u32, None) };
        return;
    }
    app.last_state_event = Some(Instant::now());
    let state = serde_json::to_value(snapshot(app).state).unwrap_or_default();
    api_server::broadcast("state.changed", &state);
}

/// The workspaces saved for the desk that is connected.
fn desk_workspaces(app: &App) -> Vec<Workspace> {
    app.cfg.desks.get(&app.sig).map(|d| workspace::list(&d.workspaces)).unwrap_or_default()
}

/// Next and previous zone give up a key the directional moves hold now (D107),
/// in the file too, so the panel shows them as not set.
fn migrate_hotkeys(app: &mut App) {
    let dropped = app.cfg.hotkeys.drop_zone_clashes();
    for action in &dropped {
        log::info!("hotkey {action} cleared: a directional key holds it now");
    }
    if !dropped.is_empty() {
        persist(app);
    }
}

/// Register the shortcuts that changed. Windows refuses one another app holds.
fn apply_hotkeys(app: &mut App) {
    let keys = &app.cfg.hotkeys;
    let mut wanted = vec![
        (HOTKEY_LINE_UP, "line_up".to_string(), keys.line_up.clone()),
        (HOTKEY_PAUSE, "pause".to_string(), keys.pause.clone()),
        (HOTKEY_NEXT_ZONE, "next_zone".to_string(), keys.next_zone.clone()),
        (HOTKEY_PREV_ZONE, "prev_zone".to_string(), keys.prev_zone.clone()),
        (HOTKEY_CAPTURE, "capture_workspace".to_string(), keys.capture_workspace.clone()),
        (HOTKEY_PIN_TOP, "pin_top".to_string(), keys.pin_top.clone()),
        (HOTKEY_PIN_STICKY, "pin_sticky".to_string(), keys.pin_sticky.clone()),
        (HOTKEY_RELEASE, "release_all".to_string(), keys.release_all.clone()),
        (HOTKEY_SNAP_BACK, "snap_back".to_string(), keys.snap_back.clone()),
    ];
    for (id, (verb, dir, combo)) in (HOTKEY_WINDOW..).zip(keys.directional()) {
        wanted.push((id, format!("{verb}_{dir}"), combo.cloned()));
    }
    for (id, ws) in (HOTKEY_WORKSPACE..).zip(desk_workspaces(app)) {
        wanted.push((id, format!("workspace:{}", ws.id), ws.hotkey));
    }
    if wanted == app.bound {
        return;
    }
    app.hotkey_errors.clear();
    for (id, _, _) in &app.bound {
        unsafe {
            let _ = UnregisterHotKey(Some(app.hwnd), *id);
        }
    }
    for (id, action, combo) in &wanted {
        let Some(combo) = combo else { continue };
        let registered = hotkey::parse(combo)
            .is_some_and(|c| unsafe { RegisterHotKey(Some(app.hwnd), *id, HOT_KEY_MODIFIERS(c.mods), c.vk).is_ok() });
        if !registered {
            log::info!("hotkey {action} not registered: {combo}");
            app.hotkey_errors.push(HotkeyError { action: action.clone(), combo: combo.clone() });
        }
    }
    app.bound = wanted;
}

fn job_screens(app: &App) -> Vec<SwitchScreen> {
    let desk = app.cfg.desks.get(&app.sig);
    let screen = |m: &Mon| SwitchScreen {
        id: m.id.clone(),
        bounds: zones::Rect { x: m.rect.left, y: m.rect.top, w: m.rect.width(), h: m.rect.height() },
        layout: desk.and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of),
    };
    app.mons.iter().map(screen).collect()
}

/// A capture of this desk. The apps the zones leave alone are never captured either.
fn capture_job(app: &App) -> CaptureJob {
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    CaptureJob { owner: app.hwnd.0 as isize, screens: job_screens(app), gap: app.cfg.zones.gap, exclude }
}

/// Start switching to a workspace on this desk. The worker does the rest and the
/// result comes back as an event.
fn start_switch(app: &mut App, id: Option<&str>, name: Option<&str>) -> Reply {
    let found = desk_workspaces(app).into_iter().find(|w| match (id, name) {
        (Some(id), _) => w.id == id,
        (None, Some(name)) => w.name.eq_ignore_ascii_case(name),
        (None, None) => false,
    });
    let Some(workspace) = found else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    switch_to(app, workspace)
}

/// Hand a workspace, saved or the last session, to the switch worker.
fn switch_to(app: &mut App, workspace: Workspace) -> Reply {
    start_switch_job(app, workspace, None)
}

fn start_switch_job(app: &mut App, workspace: Workspace, arrange_for: Option<Duration>) -> Reply {
    log::info!("switching to workspace {}", workspace.name);
    // A hotkey gives the tray the right to take focus. For anyone else this is all it can do.
    let _ = unsafe { AllowSetForegroundWindow(std::process::id()) };
    let (owner, screens, gap) = (app.hwnd.0 as isize, job_screens(app), app.cfg.zones.gap);
    let pins_stay = app.cfg.windows.pins_stay_on_switch;
    let job = SwitchJob { owner, workspace, screens, gap, arrange_for, pins_stay };
    if switcher::switch(job) {
        Ok(json!({ "ok": true }))
    } else {
        refused("Couldn't start the switch")
    }
}

/// Why window placement steps aside for another window manager (D110), if it does.
fn aside(app: &App) -> Option<String> {
    match (app.other_wm, app.cfg.windows.other_wm) {
        (Some(name), _) => {
            Some(format!("{} is running, so Seam leaves placing windows to it", state::other_wm_title(name)))
        }
        (None, true) => Some("Another window manager handles the windows, so Seam leaves placing them to it".into()),
        (None, false) => None,
    }
}

/// The seam-wm thread found another window manager start or stop.
fn other_wm_changed(app: &mut App) {
    app.other_wm = wm::detected();
    log::info!("other window manager: {}", app.other_wm.unwrap_or("none"));
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    sync_live(app);
    publish(app);
}

/// Queue a window command for the focused window. The worker answers.
fn start_window(app: &App, verb: WindowVerb, dir: Dir, reply: Option<Sender<Reply>>) {
    // Focus doesn't place anything, so it works next to another window manager.
    if let Some(why) = aside(app).filter(|_| verb != WindowVerb::Focus) {
        log::info!("window {verb:?} {}: {why}", dir.name());
        if let Some(tx) = reply {
            let _ = tx.send(refused(why));
        }
        return;
    }
    let hwnd = unsafe { GetForegroundWindow() };
    let desk = app.cfg.desks.get(&app.sig).filter(|_| app.cfg.zones.enabled);
    let layout = |m: &Mon| desk.and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of);
    wincontrol::start(wincontrol::Job {
        owner: app.hwnd.0 as isize,
        verb,
        dir,
        hwnd: hwnd.0 as isize,
        snapped: snap::zone_of(hwnd),
        mons: app.mons.clone(),
        links: app.links.clone(),
        layouts: app.mons.iter().map(layout).collect(),
        gap: app.cfg.zones.gap,
        exclude: app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect(),
        // A hotkey gives the tray the right to take focus. The API has to earn it.
        from_api: reply.is_some(),
        reply,
    });
}

/// A switch finished: say which workspace is in use, and how it went.
fn switched(app: &mut App) {
    let Some(done) = switcher::take_switched() else { return };
    log::info!(
        "workspace {}: {} launched, {} placed, {} skipped",
        done.name,
        done.launched,
        done.placed,
        done.skipped.len()
    );
    // The last session isn't a workspace anyone can pick, so it never shows as the one in use.
    if done.id != SESSION_ID {
        app.workspace = Some(done.id.clone());
        sync_live(app);
        app.recheck = true;
        unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
    }
    windows_changed(app);
    publish(app);
    let skipped: Vec<Value> =
        done.skipped.iter().map(|(app, reason)| json!({ "app": app, "reason": reason })).collect();
    let data = json!({
        "id": done.id, "name": done.name, "launched": done.launched, "placed": done.placed, "skipped": skipped
    });
    api_server::broadcast("workspace.switched", &data);
}

/// The workspace in use on this desk, if there is one.
fn active_workspace(app: &App) -> Option<Workspace> {
    let id = app.workspace.as_deref()?;
    desk_workspaces(app).into_iter().find(|w| w.id == id)
}

/// A workspace with what a status check or auto-apply needs to place its windows.
fn live_of(app: &App, workspace: Workspace) -> LiveWorkspace {
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let (screens, gap, pins_stay) = (job_screens(app), app.cfg.zones.gap, app.cfg.windows.pins_stay_on_switch);
    LiveWorkspace { workspace, screens, gap, pins_stay, exclude }
}

/// Hand the workspace in use to the live worker, and tell the seam-wm thread what to
/// watch. Auto-apply steps aside for another window manager, as snapping does (D110).
fn sync_live(app: &mut App) {
    let active = active_workspace(app);
    if active.is_none() {
        app.drifted = false;
    }
    let auto_apply = app.cfg.auto_apply && aside(app).is_none();
    let live = active.map(|ws| live_of(app, ws));
    live::set_active(app.hwnd.0 as isize, live, auto_apply);
}

/// Snap back: switch to the workspace in use again, so what drifted goes back.
fn snap_back(app: &mut App) -> Reply {
    match app.workspace.clone() {
        Some(id) => start_switch(app, Some(&id), None),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace is in use on this desk")),
    }
}

/// The seam-wm thread saw a window a workspace placed move or minimize.
fn ws_moved(app: &mut App, key: u32) {
    if !app.moved.contains(&key) {
        app.moved.push(key);
    }
    unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
}

/// Things have been still for a moment: tell the panel which workspaces' windows
/// moved, and have the worker look whether the one in use drifted.
fn drift_due(app: &mut App) {
    let moved = std::mem::take(&mut app.moved);
    for ws in desk_workspaces(app).into_iter().filter(|w| moved.contains(&workspace::stamp_key(&w.id))) {
        api_server::broadcast("workspace.moved", &json!({ "id": ws.id }));
    }
    let active = app.workspace.as_deref().map(workspace::stamp_key);
    if std::mem::take(&mut app.recheck) || active.is_some_and(|key| moved.contains(&key)) {
        live::check_drift();
    }
}

/// The worker looked at the workspace in use.
fn drift_checked(app: &mut App) {
    let Some(drift) = live::take_drift() else { return };
    if drift.placed {
        api_server::broadcast("workspace.moved", &json!({ "id": drift.id }));
        windows_changed(app);
    }
    if app.workspace.as_deref() == Some(drift.id.as_str()) && drift.drifted != app.drifted {
        log::info!("workspace {}: {}", drift.id, if drift.drifted { "drifted" } else { "in place" });
        app.drifted = drift.drifted;
        publish(app);
    }
}

/// Start capturing the windows on screen. Returns an answer only if it can't start.
fn start_capture(
    app: &mut App,
    name: Option<String>,
    into: Option<String>,
    row: Option<usize>,
    reply: Option<Sender<Reply>>,
) -> Option<Reply> {
    if app.capturing.is_some() {
        return Some(busy("Already capturing"));
    }
    let target = into.as_ref().map(|into| desk_workspaces(app).into_iter().find(|w| w.id == *into));
    if matches!(target, Some(None)) {
        return Some(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
    }
    if row.is_some_and(|row| target.flatten().is_none_or(|w| row >= w.windows.len())) {
        return Some(Err(RpcError::new(api::NOT_FOUND, "no such row in that workspace")));
    }
    app.capturing = Some(Capturing { reply, name, into, row, sig: app.sig.clone() });
    if switcher::capture(capture_job(app)) {
        return None;
    }
    app.capturing = None;
    Some(refused("Couldn't start the capture"))
}

/// A capture finished: save it as a new workspace, or over the one it was asked to update.
fn captured(app: &mut App) {
    let (Some(rules), Some(job)) = (switcher::take_captured(), app.capturing.take()) else { return };
    let answer = |r: Reply| {
        if let Some(tx) = &job.reply {
            let _ = tx.send(r);
        }
    };
    let Some(desk) = app.cfg.desks.get_mut(&job.sig) else { return answer(refused("That desk is gone")) };
    let existing = workspace::list(&desk.workspaces);
    let saved = match &job.into {
        Some(into) => {
            let Some(old) = existing.iter().find(|w| w.id == *into) else {
                return answer(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
            };
            let updated = match job.row {
                Some(row) => match workspace::recapture_row(old, row, &rules) {
                    Some(updated) => updated,
                    None => return answer(refused("That app isn't open, so where it goes stays as it was")),
                },
                None => Workspace { windows: workspace::merge_capture(old, rules), ..old.clone() },
            };
            let same = |v: &&mut Value| workspace::parse(v).is_some_and(|w| w.id == *into);
            if let Some(slot) = desk.workspaces.iter_mut().find(same) {
                *slot = workspace::to_value(&updated);
            }
            updated
        }
        None => {
            let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
            let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
            let name = match job.name.as_deref().map(str::trim) {
                Some(name) if !name.is_empty() => workspace::unique_name(name, &names),
                _ => workspace::next_name(&names),
            };
            let created = Workspace { id: workspace::new_id(&ids), name, windows: rules, ..Workspace::default() };
            desk.workspaces.push(workspace::to_value(&created));
            created
        }
    };
    persist(app);
    publish(app);
    if job.reply.is_some() {
        return answer(Ok(json!({ "id": saved.id })));
    }
    let body = format!("{} windows saved as {}", saved.windows.len(), saved.name);
    notify(app, "Workspace captured", &body, BalloonAction::OpenPanel("workspaces"));
}

/// The editor saved a workspace of this desk. It's checked as `checked_update`
/// says, then saved and applied through the same path as `config.patch`.
fn update_workspace(app: &mut App, id: &str, sent: Value) -> Reply {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return refused("This desk has no workspaces") };
    let Some(old) = workspace::list(&desk.workspaces).into_iter().find(|w| w.id == id) else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    let sent: Workspace = match serde_json::from_value(sent) {
        Ok(ws) => ws,
        Err(e) => return refused(format!("That isn't a workspace: {e}")),
    };
    let zones = |monitor: &str| {
        let layout = desk.zones.get(monitor).and_then(zones::layout_of);
        layout.map_or(0, |l| zones::leaf_count(&l.tree))
    };
    let checked = match workspace::checked_update(&old, sent, &zones) {
        Ok(ws) => ws,
        Err(why) => return refused(why),
    };
    let others: Vec<String> =
        workspace::list(&desk.workspaces).into_iter().filter(|w| w.id != id).map(|w| w.name).collect();
    let checked = Workspace { name: workspace::unique_name(&checked.name, &others), ..checked };
    let list: Vec<Value> = desk
        .workspaces
        .iter()
        .map(
            |v| if workspace::parse(v).is_some_and(|w| w.id == id) { workspace::to_value(&checked) } else { v.clone() },
        )
        .collect();
    let patch = json!({ "desks": { app.sig.clone(): { "workspaces": list } } });
    patch_config(app, &patch)
}

fn set_remap(app: &mut App, on: bool) {
    app.cfg.enabled = on;
    persist(app);
    relink(app);
    publish(app);
}

/// The remap needs this Windows setting, so say so if it's off.
fn warn_about_ease(app: &mut App) {
    if app.links.is_empty() || app.ease_warned || ease::is_on() {
        return;
    }
    log::info!("Ease cursor movement is off");
    app.ease_warned = notify(
        app,
        "Ease cursor movement is off",
        "Turn it on in Display settings, or the pointer can stop at screen edges.",
        BalloonAction::OpenPanel("displays"),
    );
}

fn start_lineup(app: &mut App) {
    let mons = monitors::enumerate();
    let sig = signature(mons.iter().map(|m| m.id.as_str()));
    let saved = app.cfg.desks.get(&sig).map(|d| d.borders.clone()).unwrap_or_default();
    if !overlay::start(app.hwnd, mons, &saved) {
        notify(
            app,
            "Nothing to line up",
            "Seam needs two or more screens that touch in the Windows arrangement.",
            BalloonAction::None,
        );
    }
}

/// The panel writes config.json itself; pick up what it changed.
fn reload(app: &mut App) {
    log::info!("config reloaded");
    // The tray keeps what Restart apps was, and the file may not have its last save yet.
    let saved = app.cfg.restore_apps_saved;
    app.cfg = Config::load();
    app.cfg.restore_apps_saved = saved;
    log::apply(app.cfg.debug_log);
    sync_restart_apps(app);
    migrate_hotkeys(app);
    apply_hotkeys(app);
    refresh(app);
}

/// Start the panel, which raises the open one if there is one.
fn open_panel(app: &mut App, page: Option<&str>) -> bool {
    let panel = std::env::current_exe().map(|exe| exe.with_file_name("seam-settings.exe"));
    // Lets an already open panel come to the front.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
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

fn refused(message: impl Into<String>) -> Reply {
    Err(RpcError::new(api::REFUSED, message))
}

fn busy(message: &str) -> Reply {
    Err(RpcError::new(api::BUSY, message))
}

/// Merge a patch into the settings, keep it only if the result is a valid
/// config, and apply it live.
fn patch_config(app: &mut App, patch: &Value) -> Reply {
    let mut next = match app.cfg.patched(patch) {
        Ok(c) => c,
        Err(e) => return refused(format!("That isn't a valid setting: {e}")),
    };
    if next.version != app.cfg.version {
        return refused("That config is from a different version of Seam");
    }
    next.layouts.clear();
    // What Windows' Restart apps was before Seam changed it is the tray's to keep.
    next.restore_apps_saved = app.cfg.restore_apps_saved;
    app.cfg = next;
    sync_restart_apps(app);
    persist(app);
    log::apply(app.cfg.debug_log);
    apply_hotkeys(app);
    relink(app);
    publish(app);
    arm_auto_install(app);
    Ok(json!({ "ok": true }))
}

/// Mode sets can stall input for seconds, so this runs off the main thread with
/// the hook removed. Windows may silently remove a hook that stalls.
fn start_align(app: &mut App, reply: Sender<Reply>) -> Option<Reply> {
    if overlay::is_active() {
        return Some(busy("Line Up is open"));
    }
    let mut slot = ALIGNING.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Some(busy("Already aligning the arrangement"));
    }
    *slot = Some(Aligning { reply, result: None });
    drop(slot);
    hook::enable(false);
    let (mons, links) = (app.mons.clone(), app.links.clone());
    let hwnd = app.hwnd.0 as isize;
    let spawned = std::thread::Builder::new().name("seam-align".into()).spawn(move || {
        let result = arrange::align(&mons, &links);
        if let Some(job) = ALIGNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            job.result = Some(result);
        }
        let hwnd = HWND(hwnd as *mut _);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_ALIGNED, WPARAM(0), LPARAM(0));
        }
    });
    if spawned.is_err() {
        ALIGNING.lock().unwrap_or_else(|e| e.into_inner()).take();
        arm_hook(app);
        return Some(busy("Couldn't start the arrangement change"));
    }
    None
}

fn align_finished(app: &mut App) {
    let Some(job) = ALIGNING.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let result = job.result.unwrap_or_else(|| Err("the arrangement change stopped".into()));
    log::info!("arrangement result: {result:?}");
    let message = result.as_ref().err().cloned().unwrap_or_default();
    app.last_align = Some(AlignResult { ok: result.is_ok(), message: message.clone() });
    refresh(app);
    let _ = job.reply.send(if result.is_ok() { Ok(json!({ "ok": true })) } else { refused(message) });
}

/// Run one command and answer it, unless a worker will answer it later.
fn run_command(app: &mut App, queued: Queued) {
    let ok = || Ok(json!({ "ok": true }));
    let Queued { command, reply, panel } = queued;
    let result = match command {
        Command::ConfigPatch(patch) => patch_config(app, &patch),
        Command::RemapSet(on) => {
            set_remap(app, on);
            Ok(json!({ "on": on }))
        }
        Command::ZonesSet(on) => {
            app.cfg.zones.enabled = on;
            persist(app);
            snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
            publish(app);
            Ok(json!({ "on": on }))
        }
        Command::LineupStart if overlay::is_active() => busy("Line Up is already open"),
        Command::LineupStart => {
            start_lineup(app);
            ok()
        }
        Command::ArrangementAlign => {
            if let Some(refusal) = start_align(app, reply.clone()) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::ScreenIdentify(number) => match number.checked_sub(1).and_then(|i| app.mons.get(i as usize)) {
            Some(mon) => {
                identify::show(number, mon.rect);
                ok()
            }
            None => Err(RpcError::new(api::NOT_FOUND, "no such screen")),
        },
        Command::PanelOpen(page) if open_panel(app, page.as_deref()) => ok(),
        Command::PanelOpen(_) => refused("seam-settings.exe needs to be next to seam.exe"),
        Command::AppQuit => {
            // The updater and the Store copy ask without the panel's token.
            app.release_on_exit = panel && app.install_started.is_none();
            unsafe {
                let _ = PostMessageW(Some(app.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            ok()
        }
        Command::UpdateReady if app.packaged => refused("The Store updates Seam"),
        Command::UpdateReady => {
            read_update_state(app);
            ok()
        }
        Command::ZoneMove(dir) => match snap::move_foreground(dir == api::Dir::Next) {
            Ok(()) => ok(),
            Err(why) => refused(why),
        },
        Command::Window(verb, dir) => {
            start_window(app, verb, dir, Some(reply));
            return;
        }
        Command::WindowPin { kind, on } => {
            wm::pin(kind, on, unsafe { GetForegroundWindow() }.0 as isize, Some(reply));
            return;
        }
        Command::WmRelease => {
            wm::release(Some(reply));
            return;
        }
        Command::SessionRestore => match saved_session(app) {
            Some(session) => restore_session(app, session.workspace()),
            None => Err(RpcError::new(api::NOT_FOUND, "No last session is saved for this desk")),
        },
        Command::WorkspaceSwitch { id, name } => start_switch(app, id.as_deref(), name.as_deref()),
        Command::WorkspaceUpdate { id, workspace } => update_workspace(app, &id, workspace),
        Command::WorkspaceStatus { id } => match desk_workspaces(app).into_iter().find(|w| w.id == id) {
            Some(ws) => {
                live::answer_status(live_of(app, ws), reply);
                return;
            }
            None => Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")),
        },
        Command::WorkspaceSnapBack => snap_back(app),
        Command::AutoApplySet(on) => {
            app.cfg.auto_apply = on;
            persist(app);
            sync_live(app);
            publish(app);
            Ok(json!({ "on": on }))
        }
        Command::WorkspaceCapture { name, into, row } => {
            if let Some(refusal) = start_capture(app, name, into, row, Some(reply.clone())) {
                let _ = reply.send(refusal);
            }
            return;
        }
        // Answered on the client's own thread.
        _ => Err(RpcError::new(api::METHOD_NOT_FOUND, "not a tray command")),
    };
    let _ = reply.send(result);
}

fn drain_api(app: &mut App) {
    if let Some(names) = api_server::take_client_names() {
        app.clients = names;
        publish(app);
    }
    for queued in api_server::take_commands() {
        run_command(app, queued);
    }
}

fn lineup_done(app: &mut App, accepted: bool) {
    snap::line_up_ended();
    if let (true, Some(res)) = (accepted, overlay::take_result()) {
        let mons = monitors::enumerate();
        let sig = signature(mons.iter().map(|m| m.id.as_str()));
        let links = resolve(&mons, Some(&res));
        desk::note_active(&mut app.cfg, &sig, &mons, unix_now());
        if let Some(desk) = app.cfg.desks.get_mut(&sig) {
            desk.borders = res;
        }
        if !app.cfg.seen.contains(&sig) {
            app.cfg.seen.push(sig);
        }
        app.cfg.enabled = true;
        persist(app);
        // A mode set can stall input for seconds; never let it run under the
        // hook, or Windows may silently remove it.
        hook::enable(false);
        let msg = if app.cfg.align_arrangement {
            let res = arrange::align(&mons, &links);
            log::info!("arrangement result: {res:?}");
            let message = res.as_ref().err().cloned().unwrap_or_default();
            app.last_align = Some(AlignResult { ok: res.is_ok(), message });
            match res {
                Ok(_) => "The pointer now crosses where your screens meet.".to_string(),
                Err(e) => format!("Pointer remap is on. Windows arrangement unchanged: {e}."),
            }
        } else {
            log::info!("arrangement skipped: setting is off");
            "The pointer now crosses where your screens meet.".to_string()
        };
        refresh(app);
        notify(app, "Screens lined up", &msg, BalloonAction::None);
        api_server::broadcast("lineup.finished", &json!({ "saved": true }));
    } else {
        refresh(app);
        api_server::broadcast("lineup.finished", &json!({ "saved": false }));
    }
}

fn show_menu(app: &mut App) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let flag = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
        let status = match (app.mons.len(), app.links.len()) {
            (n, 0) => format!("{n} screen(s), not lined up yet"),
            (n, k) => format!("{n} screens, {k} border(s) lined up"),
        };
        let status_w: Vec<u16> = status.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status_w.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open Seam…"));
        let _ = AppendMenuW(menu, MF_STRING, ID_LINEUP, w!("Line Up Screens…"));
        let workspaces = desk_workspaces(app);
        // Only made when there is something to put in it, so no menu handle is dropped unused.
        let submenu = if workspaces.is_empty() { None } else { CreatePopupMenu().ok() };
        if let Some(submenu) = submenu {
            let active = app.workspace.as_deref();
            for (id, ws) in (ID_WORKSPACE..).zip(&workspaces) {
                // A tab puts the shortcut at the right edge, and && is how a menu spells &.
                let mut label = ws.name.replace('&', "&&");
                if let Some(combo) = &ws.hotkey {
                    label = format!("{label}\t{combo}");
                }
                let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(submenu, flag(active == Some(ws.id.as_str())), id, PCWSTR(label.as_ptr()));
            }
            if AppendMenuW(menu, MF_POPUP, submenu.0 as usize, w!("Workspaces")).is_err() {
                let _ = DestroyMenu(submenu);
            }
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, flag(app.cfg.enabled), ID_ENABLED, w!("Remap pointer at borders"));
        let _ = AppendMenuW(menu, MF_STRING, ID_RELEASE, w!("Release pinned windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit Seam"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(app.hwnd);
        let hwnd = app.hwnd;
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 as usize {
            ID_LINEUP => start_lineup(app),
            ID_OPEN => {
                open_panel(app, None);
            }
            ID_ENABLED => set_remap(app, !app.cfg.enabled),
            ID_RELEASE => wm::release(None),
            id if id >= ID_WORKSPACE => {
                let chosen = workspaces.get(id - ID_WORKSPACE).map(|w| w.id.clone());
                if let Some(Err(e)) = chosen.map(|id| start_switch(app, Some(&id), None)) {
                    log::info!("workspace menu: {}", e.message);
                }
            }
            ID_QUIT => {
                app.release_on_exit = true;
                // Close after this borrow ends so WM_DESTROY can clean up the icon.
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xffff) as u32;
            match event {
                WM_CONTEXTMENU | WM_RBUTTONUP => {
                    // The menu runs a nested loop; take the app out of the cell
                    // only through with_app, and do it after this message returns.
                    let _ = PostMessageW(Some(hwnd), WM_APP + 10, WPARAM(0), LPARAM(0));
                }
                NIN_SELECT | NIN_KEYSELECT => {
                    let _ = PostMessageW(Some(hwnd), WM_APP + 12, WPARAM(0), LPARAM(0));
                }
                NIN_BALLOONUSERCLICK => {
                    let click = with_app(|a| std::mem::replace(&mut a.balloon_action, BalloonAction::None));
                    match click {
                        Some(BalloonAction::LineUp) => {
                            let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
                        }
                        Some(BalloonAction::OpenPanel(page)) => {
                            with_app(|a| open_panel(a, Some(page)));
                        }
                        Some(BalloonAction::Reopen) => {
                            with_app(reopen_closed);
                        }
                        Some(BalloonAction::RestoreSession) => {
                            with_app(|a| {
                                if let Some(Err(e)) = a.session_offer.take().map(|s| restore_session(a, s)) {
                                    log::info!("last session: {}", e.message);
                                }
                            });
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        m if m == WM_APP + 10 => {
            with_app(show_menu);
            LRESULT(0)
        }
        m if m == WM_APP + 12 => {
            with_app(tray_click);
            LRESULT(0)
        }
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_LINE_UP => {
                    with_app(start_lineup);
                }
                HOTKEY_PAUSE => {
                    with_app(|a| set_remap(a, !a.cfg.enabled));
                }
                id @ (HOTKEY_NEXT_ZONE | HOTKEY_PREV_ZONE) => {
                    if let Err(why) = snap::move_foreground(id == HOTKEY_NEXT_ZONE) {
                        log::info!("zone hotkey: {why}");
                    }
                }
                HOTKEY_CAPTURE => {
                    with_app(|a| start_capture(a, None, None, None, None));
                }
                id @ (HOTKEY_PIN_TOP | HOTKEY_PIN_STICKY) => {
                    let kind = if id == HOTKEY_PIN_TOP { PinKind::Top } else { PinKind::Sticky };
                    wm::pin(kind, None, GetForegroundWindow().0 as isize, None);
                }
                HOTKEY_RELEASE => wm::release(None),
                HOTKEY_SNAP_BACK => {
                    if let Some(Err(e)) = with_app(snap_back) {
                        log::info!("snap back hotkey: {}", e.message);
                    }
                }
                id if (HOTKEY_WINDOW..HOTKEY_WINDOW + WINDOW_KEYS).contains(&id) => {
                    with_app(|a| {
                        let (verb, dir, _) = a.cfg.hotkeys.directional()[(id - HOTKEY_WINDOW) as usize];
                        if let (Some(verb), Some(dir)) = (WindowVerb::parse(verb), Dir::parse(dir)) {
                            start_window(a, verb, dir, None);
                        }
                    });
                }
                slot if slot >= HOTKEY_WORKSPACE => {
                    with_app(|a| {
                        let action = a.bound.iter().find(|(id, ..)| *id == slot).map(|(_, action, _)| action.clone());
                        let id = action.as_deref().and_then(|action| action.strip_prefix("workspace:"));
                        if let Some(Err(e)) = id.map(|id| start_switch(a, Some(id), None)) {
                            log::info!("workspace hotkey: {}", e.message);
                        }
                    });
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_SWITCHED => {
            with_app(switched);
            LRESULT(0)
        }
        WM_APP_CAPTURED => {
            with_app(captured);
            LRESULT(0)
        }
        WM_APP_WS_MOVED => {
            with_app(|a| ws_moved(a, wparam.0 as u32));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_DRIFT => {
            if with_app(drift_due).is_some() {
                let _ = KillTimer(Some(hwnd), TIMER_DRIFT);
            }
            LRESULT(0)
        }
        WM_APP_DRIFT => {
            with_app(drift_checked);
            LRESULT(0)
        }
        WM_APP_SESSION => {
            with_app(session_captured);
            LRESULT(0)
        }
        WM_APP_WINDOWS_CHANGED => {
            with_app(|a| windows_changed(a));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_SESSION => {
            if with_app(start_session_capture).is_some() {
                let _ = KillTimer(Some(hwnd), TIMER_SESSION);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_SESSION_OFFER => {
            if with_app(offer_session).unwrap_or(false) {
                let _ = KillTimer(Some(hwnd), TIMER_SESSION_OFFER);
            }
            LRESULT(0)
        }
        wm::WM_APP_PIN_CLOSED => {
            with_app(pin_closed);
            LRESULT(0)
        }
        WM_QUERYENDSESSION => {
            with_app(query_end_session);
            LRESULT(1)
        }
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                with_app(end_session);
            } else {
                // The sign-out was called off.
                with_app(|a| a.session_ending = false);
            }
            LRESULT(0)
        }
        WM_APP_API => {
            if with_app(drain_api).is_none() {
                SetTimer(Some(hwnd), TIMER_API, 250, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_API => {
            if with_app(drain_api).is_some() {
                let _ = KillTimer(Some(hwnd), TIMER_API);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_STATE => {
            let _ = KillTimer(Some(hwnd), TIMER_STATE);
            with_app(send_state_event);
            LRESULT(0)
        }
        WM_APP_ALIGNED => {
            with_app(align_finished);
            LRESULT(0)
        }
        WM_APP_UPDATE => {
            with_app(update_read);
            LRESULT(0)
        }
        WM_APP_REFIT => {
            snap::refit_found();
            LRESULT(0)
        }
        wincontrol::WM_APP_WINDOW => {
            wincontrol::planned();
            LRESULT(0)
        }
        wm::WM_APP_WM => {
            if with_app(other_wm_changed).is_none() {
                SetTimer(Some(hwnd), TIMER_OTHER_WM, 250, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_OTHER_WM => {
            if with_app(other_wm_changed).is_some() {
                let _ = KillTimer(Some(hwnd), TIMER_OTHER_WM);
            }
            LRESULT(0)
        }
        // The work area changed, which is what a scaling change or a moved taskbar does.
        // Windows' accent colour changed, which the focus border follows.
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            wm::accent_changed();
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if wparam.0 == SPI_SETWORKAREA.0 as usize {
                snap::work_area_changed();
                snap::display_changed();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_APP_SNAP_BLOCKED => {
            let body = "Seam can't move windows from apps running as administrator.";
            with_app(|a| notify(a, "Seam", body, BalloonAction::None));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_UPDATE => {
            SetTimer(Some(hwnd), TIMER_UPDATE, UPDATE_CHECK_EVERY_MS, None);
            if with_app(|a| a.cfg.updates.auto_check).unwrap_or(false) {
                check_for_updates(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_AUTO_INSTALL => {
            with_app(auto_install);
            LRESULT(0)
        }
        WM_APP_LINEUP => {
            with_app(start_lineup);
            LRESULT(0)
        }
        overlay::WM_APP_CALIB_DONE => {
            with_app(|a| lineup_done(a, wparam.0 != 0));
            LRESULT(0)
        }
        tray::WM_APP_RELOAD => {
            if with_app(reload).is_none() {
                SetTimer(Some(hwnd), TIMER_RELOAD, 250, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_RELOAD => {
            if with_app(reload).is_some() {
                let _ = KillTimer(Some(hwnd), TIMER_RELOAD);
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            log::info!(
                "display change: {}x{}, overlay active: {}",
                lparam.0 & 0xffff,
                (lparam.0 >> 16) & 0xffff,
                overlay::is_active()
            );
            // Screens changed under us: a half-finished line-up no longer applies.
            if overlay::is_active() {
                overlay::cancel();
            }
            SetTimer(Some(hwnd), TIMER_REFRESH, 800, None);
            snap::display_changed();
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
                log::info!("resumed from sleep");
                SetTimer(Some(hwnd), TIMER_REFRESH, 1500, None);
                snap::display_changed();
            }
            LRESULT(1)
        }
        WM_TIMER if wparam.0 == TIMER_REARM => {
            hook::log_stats();
            if !overlay::is_active() {
                with_app(|a| arm_hook(a));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_REFRESH => {
            // If the app is busy (menu open), keep the timer and try again.
            if with_app(refresh).is_some() {
                let _ = KillTimer(Some(hwnd), TIMER_REFRESH);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            hook::enable(false);
            if with_app(|a| a.release_on_exit).unwrap_or(false) {
                wm::release_before_exit();
            }
            snap::shutdown();
            with_app(|a| {
                let nid = tray_data(a);
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                if let Some(saver) = a.saver.take() {
                    saver.finish();
                }
            });
            PostQuitMessage(0);
            LRESULT(0)
        }
        m if with_app(|a| a.taskbar_created == m).unwrap_or(false) => {
            // Explorer (re)started: put the icon back, and retry a prompt
            // that couldn't be shown earlier.
            with_app(|a| {
                add_icon(a);
                refresh(a);
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Seam is already running, so ask it to do what this launch was for. The one
/// exception is the Store copy meeting the GitHub copy: the Store copy wins, so
/// it asks the other to quit and starts in its place. Returns whether to start.
fn take_over(mut client: Client, line_up: bool) -> bool {
    let info = client.call("app.info", Value::Null).ok();
    let pid = info.as_ref().and_then(|i| i["pid"].as_u64()).and_then(|pid| u32::try_from(pid).ok());
    let github_copy = info.as_ref().is_some_and(|i| i["packaged"] == false);
    if package::is_packaged() && github_copy && client.call("app.quit", Value::Null).is_ok() {
        // It was told to close, so it will, even if slowly. Wait for it, then
        // start either way, so there's never no tray at all.
        if let Some(pid) = pid {
            process::wait_for_exit(pid, TAKE_OVER_WAIT_MS);
        }
        return true;
    }
    // Hand over foreground rights so the window that opens gets the keyboard.
    if let Some(pid) = pid {
        let _ = unsafe { AllowSetForegroundWindow(pid) };
    }
    let method = if line_up { "lineup.start" } else { "panel.open" };
    let _ = client.call(method, json!({}));
    false
}

fn arg_after(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // One instance only. Launching again opens Seam in the running one.
        let has_flag = |flag: &str| std::env::args().any(|a| a == flag);
        let line_up = has_flag("--line-up");
        let updated_from = arg_after("--updated-from");
        let _mutex = CreateMutexW(None, true, w!("Local\\Seam.SingleInstance"));
        let mutex_taken = GetLastError() == ERROR_ALREADY_EXISTS;
        // The pipe name is shared by the Store and GitHub copies, so it also
        // finds a Seam whose mutex this one can't see. A mutex alone means v0.1.
        let carry_on = match Client::connect("seam", None) {
            Ok(client) => take_over(client, line_up),
            Err(_) => !mutex_taken,
        };
        if !carry_on {
            return;
        }

        // A Store copy's first run starts clean and offers the earlier settings in the panel.
        let cfg = if package::first_run() {
            package::stash_earlier_config();
            Config::default()
        } else {
            Config::load()
        };
        // After the instance check: only the running instance owns the log.
        log::apply(cfg.debug_log);
        log::info!("seam {} started", env!("CARGO_PKG_VERSION"));

        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // A hidden top-level window (not message-only) so it receives
        // WM_DISPLAYCHANGE and the TaskbarCreated broadcast.
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!("Seam tray"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            return;
        };

        let app = App {
            hwnd,
            icon: make_icon(),
            cfg,
            mons: Vec::new(),
            sig: String::new(),
            links: Vec::new(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            balloon_action: BalloonAction::None,
            ease_warned: false,
            last_align: None,
            saver: Saver::start(),
            token: api_server::new_token().unwrap_or_default(),
            hotkey_errors: Vec::new(),
            bound: Vec::new(),
            workspace: None,
            drifted: false,
            moved: Vec::new(),
            recheck: false,
            capturing: None,
            clients: Vec::new(),
            last_state_event: None,
            packaged: package::is_packaged(),
            update: None,
            install_started: None,
            update_failed: false,
            other_wm: None,
            release_on_exit: false,
            reopen: None,
            session_offer: None,
            session_offer_tries: 0,
            session_capture: None,
            session_ending: false,
            restart_apps: false,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            add_icon(a);
            migrate_hotkeys(a);
            apply_hotkeys(a);
            check_restart_apps(a);
            refresh(a);
            api_server::start(a.hwnd.0 as isize, a.token.clone(), snapshot(a), a.cfg.api.enabled);
        });
        SetTimer(Some(hwnd), TIMER_REARM, 60_000, None);
        if with_app(|a| a.packaged) == Some(false) {
            SetTimer(Some(hwnd), TIMER_UPDATE, FIRST_UPDATE_CHECK_MS, None);
            package::note_unpackaged_install();
            if let Ok(tray) = std::env::current_exe() {
                autostart::add_flag(&tray);
                install::take_over_autostart(&tray);
            }
            with_app(|a| read_update_state(a));
        }
        if let Some(from) = &updated_from {
            log::info!("updated from {from}");
            let _ = std::thread::Builder::new().name("seam-update-clean".into()).spawn(clean_up_after_update);
            let body = format!("Updated to {}", env!("CARGO_PKG_VERSION"));
            with_app(|a| notify(a, "Seam", &body, BalloonAction::None));
        }
        // A launch from the Start menu or by hand opens the window too. A
        // sign-in launch, a restart after an update and a `--line-up` start don't.
        let login = package::login_launch();
        let quiet = line_up || updated_from.is_some() || has_flag("--tray-only") || login;
        if !quiet {
            with_app(|a| open_panel(a, None));
        }
        // After a sign-in, offer the last session once the desktop has settled (D120).
        if login && with_app(|a| a.cfg.restore_session) == Some(true) {
            SetTimer(Some(hwnd), TIMER_SESSION_OFFER, SESSION_OFFER_AFTER_MS, None);
        }
        if line_up {
            let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        log::info!("seam exiting");
        log::flush();
    }
}
