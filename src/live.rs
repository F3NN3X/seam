//! The active workspace while it's in use: whether its windows drifted, and
//! putting the windows that open into place (§5, §11.1). One worker does both,
//! one job at a time, so two windows opening together never take the same row.
//! The tray thread only hands it the workspace and reads what it found.

use crate::log;
use crate::switcher::{self, LiveWorkspace};
use seam::api::Reply;
use seam::workspace;
use serde_json::json;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// The worker checked the active workspace for drift. Read it with `take_drift`.
pub const WM_APP_DRIFT: u32 = WM_APP + 27;

/// What the worker found about the active workspace.
pub struct Drift {
    pub id: String,
    pub drifted: bool,
    /// Auto-apply placed a window of it.
    pub placed: bool,
}

enum Job {
    Drift,
    Opened(isize),
}

/// The workspace in use, with what placing its windows needs, and who to tell.
static ACTIVE: Mutex<Option<Arc<LiveWorkspace>>> = Mutex::new(None);
static OWNER: AtomicIsize = AtomicIsize::new(0);
static DRIFT: Mutex<Option<Drift>> = Mutex::new(None);
static WORKER: OnceLock<Option<Sender<Job>>> = OnceLock::new();

fn active() -> Option<Arc<LiveWorkspace>> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The workspace in use now, or none. The seam-wm thread hears whether to watch.
pub fn set_active(owner: isize, live: Option<LiveWorkspace>, auto_apply: bool) {
    OWNER.store(owner, Ordering::SeqCst);
    let active = live.is_some();
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = live.map(Arc::new);
    crate::wm::workspace(active, auto_apply);
}

pub fn take_drift() -> Option<Drift> {
    DRIFT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Look again whether the active workspace drifted.
pub fn check_drift() {
    send(Job::Drift);
}

/// A window opened and has settled. Called from the seam-wm thread, which never waits.
pub fn opened(hwnd: isize) {
    send(Job::Opened(hwnd));
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

/// Jobs that came in together run as one: every opened window, then one drift check.
fn work(jobs: Receiver<Job>) {
    while let Ok(first) = jobs.recv() {
        let batch: Vec<Job> = std::iter::once(first).chain(jobs.try_iter()).collect();
        let Some(live) = active() else { continue };
        let mut placed = false;
        for job in &batch {
            if let Job::Opened(hwnd) = job {
                if let Some(what) = switcher::place_opened(&live, *hwnd) {
                    log::info!("auto-apply {}: {what}", live.workspace.name);
                    placed = true;
                }
            }
        }
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
