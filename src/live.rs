//! The active workspace while it's in use: whether its windows drifted, and
//! putting the windows that open into place (§5, §11.1). One worker does both,
//! one job at a time, so two windows opening together never take the same row.
//! It also keeps the `Seam_ws` stamps right after a save changes a workspace's
//! rows. The tray thread only hands it the workspace and reads what it found.

use crate::log;
use crate::switcher::{self, AppRules, LiveWorkspace};
use seam::api::Reply;
use seam::{winutil, workspace};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// The worker checked the active workspace for drift. Read it with `take_drift`.
pub const WM_APP_DRIFT: u32 = WM_APP + 27;
/// The worker looked which workspace's windows are all in place. Read it with `take_detected`.
pub const WM_APP_DETECTED: u32 = WM_APP + 28;

/// What the worker found about the active workspace.
pub struct Drift {
    pub id: String,
    pub drifted: bool,
    /// Auto-apply placed a window of it.
    pub placed: bool,
}

/// Which workspace looks like the one in use, for the desk it was asked about.
pub struct Detected {
    pub sig: String,
    /// None unless exactly one qualifies.
    pub id: Option<String>,
}

enum Job {
    Drift,
    /// Find the one workspace of these whose running apps are all in place.
    Detect {
        sig: String,
        workspaces: Vec<LiveWorkspace>,
    },
    Opened(isize),
    /// A switch finished, so the windows that opened during it get their turn.
    SwitchEnded,
    /// These windows now hold these rows of the workspace with this stamp key. With
    /// `whole`, no other window holds one of its rows; otherwise none holds these rows.
    Stamp {
        key: u32,
        windows: Vec<(isize, usize)>,
        whole: bool,
    },
    /// The workspace's rows moved: what was row i is row `moves[i]` now, or gone.
    Restamp {
        key: u32,
        moves: Vec<Option<usize>>,
    },
}

/// The workspace in use, with what placing its windows needs, and who to tell.
static ACTIVE: Mutex<Option<Arc<LiveWorkspace>>> = Mutex::new(None);
static RULES: Mutex<Option<Arc<AppRules>>> = Mutex::new(None);
/// "Put apps in place when they open" is on, so the active workspace places what opens.
/// Windows that showed while a switch ran, which no app rule places.
static SWITCH_OWNED: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static AUTO: AtomicBool = AtomicBool::new(false);
static OWNER: AtomicIsize = AtomicIsize::new(0);
static DRIFT: Mutex<Option<Drift>> = Mutex::new(None);
static DETECTED: Mutex<Option<Detected>> = Mutex::new(None);
static WORKER: OnceLock<Option<Sender<Job>>> = OnceLock::new();

fn rules() -> Option<Arc<AppRules>> {
    RULES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn active() -> Option<Arc<LiveWorkspace>> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The workspace in use now, or none. The seam-wm thread hears whether to watch.
pub fn set_active(owner: isize, live: Option<LiveWorkspace>, auto_apply: bool) {
    OWNER.store(owner, Ordering::SeqCst);
    AUTO.store(auto_apply, Ordering::SeqCst);
    let active = live.is_some();
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = live.map(Arc::new);
    crate::wm::workspace(active, auto_apply);
}

/// The app rules now, or none. The seam-wm thread hears whether to watch.
pub fn set_rules(rules: Option<AppRules>) {
    let any = rules.is_some();
    *RULES.lock().unwrap_or_else(|e| e.into_inner()) = rules.map(Arc::new);
    crate::wm::app_rules(any);
}

pub fn take_drift() -> Option<Drift> {
    DRIFT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Look again whether the active workspace drifted.
pub fn check_drift() {
    send(Job::Drift);
}

/// A window showed while a switch was running. Called from the seam-wm thread.
pub fn owned_by_switch(hwnd: isize) {
    let mut list = SWITCH_OWNED.lock().unwrap_or_else(|e| e.into_inner());
    // Windows that closed or never settled would otherwise stay here for good.
    list.retain(|h| {
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(windows::Win32::Foundation::HWND(*h as *mut _)))
        }
        .as_bool()
    });
    list.push(hwnd);
}

/// A window opened and has settled. Called from the seam-wm thread, which never waits.
pub fn opened(hwnd: isize) {
    send(Job::Opened(hwnd));
}

/// With nothing remembered or chosen at start, find the workspace in use by its windows.
pub fn detect(sig: String, workspaces: Vec<LiveWorkspace>) {
    send(Job::Detect { sig, workspaces });
}

pub fn take_detected() -> Option<Detected> {
    DETECTED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// A switch finished. Called from the tray thread, which never waits.
pub fn switch_ended() {
    send(Job::SwitchEnded);
}

/// Stamp windows a capture saved with their rows, as a switch does. `whole` for a
/// capture of every row, so the workspace's stamps on other windows are stale.
pub fn stamp(key: u32, windows: Vec<(isize, usize)>, whole: bool) {
    send(Job::Stamp { key, windows, whole });
}

/// Move the stamps of a workspace whose rows were reordered, removed or added.
pub fn restamp(key: u32, moves: Vec<Option<usize>>) {
    send(Job::Restamp { key, moves });
}

fn send(job: Job) {
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("seam-live".into()).spawn(move || work(rx));
        spawned.ok().map(|_| tx)
    });
    match worker {
        Some(tx) if tx.send(job).is_ok() => {}
        _ => log::info!("live worker unavailable"),
    }
}

/// Jobs that came in together run as one: the stamps first, then every opened
/// window, then one drift check. Windows that open while a switch runs wait for
/// it to end, and then go through auto-apply like any other. The switch owns them,
/// so no app rule places them.
fn work(jobs: Receiver<Job>) {
    let mut later: Vec<isize> = Vec::new();
    while let Ok(first) = jobs.recv() {
        let batch: Vec<Job> = std::iter::once(first).chain(jobs.try_iter()).collect();
        for job in &batch {
            match job {
                Job::Stamp { key, windows, whole } => stamp_windows(*key, windows, *whole),
                Job::Restamp { key, moves } => restamp_windows(*key, moves),
                Job::Detect { sig, workspaces } => detect_in_use(sig, workspaces),
                _ => {}
            }
        }
        for job in &batch {
            if let Job::Opened(hwnd) = job {
                if !later.contains(hwnd) {
                    later.push(*hwnd);
                }
            }
        }
        // The switch stamps what it waited for, so only the others are placed after it.
        let opened = if switcher::switching() { Vec::new() } else { std::mem::take(&mut later) };
        let (live, rules) = (active(), rules());
        let auto = AUTO.load(Ordering::SeqCst);
        let mut placed = false;
        for hwnd in opened {
            let owned = {
                let mut list = SWITCH_OWNED.lock().unwrap_or_else(|e| e.into_inner());
                let held = list.contains(&hwnd);
                list.retain(|h| *h != hwnd);
                held
            };
            // The workspace's row wins; the app rule is for what it has no row for.
            let row = live.as_ref().filter(|_| auto).and_then(|live| switcher::place_opened(live, hwnd));
            if let (Some(what), Some(live)) = (&row, &live) {
                log::info!("auto-apply {}: {what}", live.workspace.name);
                placed = true;
            } else if let Some(set) = rules.as_ref().filter(|_| !owned) {
                if let Some(what) = switcher::place_by_rule(set, live.as_ref().map(|l| &l.workspace), hwnd) {
                    log::info!("app rule {what}");
                }
            }
        }
        let Some(live) = live else { continue };
        if placed || batch.iter().any(|j| matches!(j, Job::Drift)) {
            let drifted = workspace::drifted(&switcher::status(&live));
            *DRIFT.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(Drift { id: live.workspace.id.clone(), drifted, placed });
            let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
            unsafe {
                let _ = PostMessageW(Some(owner), WM_APP_DRIFT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn detect_in_use(sig: &str, workspaces: &[LiveWorkspace]) {
    let fits: Vec<&LiveWorkspace> =
        workspaces.iter().filter(|live| workspace::all_in_place(&switcher::status(live))).collect();
    let id = match fits[..] {
        [one] => Some(one.workspace.id.clone()),
        _ => None,
    };
    log::info!("workspace in use at start: {} of {} have every running app in place", fits.len(), workspaces.len());
    *DETECTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Detected { sig: sig.to_string(), id });
    let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
    unsafe {
        let _ = PostMessageW(Some(owner), WM_APP_DETECTED, WPARAM(0), LPARAM(0));
    }
}

/// Every top-level window with this workspace's stamp, and its row.
fn stamped(key: u32) -> impl Iterator<Item = (HWND, usize)> {
    let stamps = winutil::top_level_windows().into_iter().map(|h| (h, winutil::ws_stamp(h)));
    stamps.filter_map(move |(h, stamp)| stamp.filter(|(k, _)| *k == key).map(|(_, row)| (h, row)))
}

fn stamp_windows(key: u32, windows: &[(isize, usize)], whole: bool) {
    let stale = |(h, row): &(HWND, usize)| {
        let ours = windows.iter().any(|(w, _)| *w == h.0 as isize);
        !ours && (whole || windows.iter().any(|(_, r)| r == row))
    };
    for (h, _) in stamped(key).filter(stale) {
        winutil::clear_ws_stamp(h);
    }
    for (hwnd, row) in windows {
        winutil::set_ws_stamp(HWND(*hwnd as *mut _), key, *row);
    }
}

fn restamp_windows(key: u32, moves: &[Option<usize>]) {
    let (mut moved, mut cleared) = (0, 0);
    for (h, row) in stamped(key).collect::<Vec<_>>() {
        match moves.get(row).copied().flatten() {
            Some(to) if to == row => {}
            Some(to) => {
                winutil::set_ws_stamp(h, key, to);
                moved += 1;
            }
            None => {
                winutil::clear_ws_stamp(h);
                cleared += 1;
            }
        }
    }
    log::info!("workspace rows changed: {moved} stamps moved, {cleared} cleared");
}

/// Answer `workspace.status` from a worker of its own, so a slow app holds up nobody.
pub fn answer_status(live: LiveWorkspace, reply: Sender<Reply>) {
    let spawned = std::thread::Builder::new().name("seam-status".into()).spawn({
        let reply = reply.clone();
        move || {
            let rows = switcher::status(&live);
            let _ = reply.send(Ok(json!(rows)));
        }
    });
    if spawned.is_err() {
        let _ = reply.send(Err(seam::api::RpcError::new(seam::api::BUSY, "Couldn't check the windows")));
    }
}
