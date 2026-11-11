//! Focus on this window: minimize the other windows on its screen, or on the
//! desk's screens outside the workspace in use, and bring them back on the next
//! press. The main thread only queues the command. A worker finds the windows,
//! asks for the minimizes and keeps in memory which ones it asked, in z-order.

use crate::switcher;
use crate::wincontrol;
use seam::api::{self, FocusScope, Reply, RpcError};
use seam::geom::Rect;
use seam::winutil;
use seam::workspace::Workspace;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// How long bringing the windows back waits for them to show before it takes focus again.
const RESTORE_WAIT: Duration = Duration::from_millis(500);
const RESTORE_POLL: Duration = Duration::from_millis(25);

/// One press, with what the worker needs to know about the desk.
pub struct Job {
    pub scope: FocusScope,
    /// The focused window when the key came.
    pub hwnd: isize,
    /// The desk's screens, in pixels.
    pub screens: Vec<Rect>,
    /// Lower-case exe names the zones leave alone.
    pub exclude: Vec<String>,
    /// The workspace in use, for the workspace scope.
    pub workspace: Option<Workspace>,
    /// From the API, which has no right to take focus until it sends input.
    pub from_api: bool,
    pub reply: Option<Sender<Reply>>,
}

/// What the last press minimized, topmost first, and the window that had focus.
#[derive(Default)]
struct Held {
    windows: Vec<isize>,
    focused: isize,
    /// What the press was for, so only the same kind of press puts it back.
    scope: Option<FocusScope>,
    screen: Option<Rect>,
}

/// A workspace switch finished: what was held belongs to the old context.
static FORGET: AtomicBool = AtomicBool::new(false);

/// Drop what the last press minimized, so the next press starts fresh.
pub fn forget() {
    FORGET.store(true, Ordering::SeqCst);
}

static SLOT: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn answer(reply: Option<Sender<Reply>>, result: Reply) {
    if let Some(tx) = reply {
        let _ = tx.send(result);
    }
}

/// Hand a press to the worker. One still waiting there is replaced.
pub fn start(job: Job) {
    static RUNNING: OnceLock<bool> = OnceLock::new();
    let running = *RUNNING.get_or_init(|| std::thread::Builder::new().name("seam-focus".into()).spawn(work).is_ok());
    if !running {
        answer(job.reply, Err(RpcError::new(api::BUSY, "Couldn't start the focus command")));
        return;
    }
    let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(earlier) = slot.replace(job) {
        answer(earlier.reply, Err(RpcError::new(api::BUSY, "A newer window command replaced this one")));
    }
    WAKE.notify_one();
}

fn work() {
    let mut held = Held::default();
    loop {
        let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
        while slot.is_none() {
            slot = WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
        }
        let Some(mut job) = slot.take() else { continue };
        drop(slot);
        let reply = job.reply.take();
        match run(&job, &mut held) {
            Ok(result) => answer(reply, Ok(result)),
            Err(why) => {
                crate::log::info!("focus on this window: {why}");
                answer(reply, Err(RpcError::new(api::REFUSED, why)));
            }
        }
    }
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

fn exists(key: isize) -> bool {
    key != 0 && unsafe { IsWindow(Some(hwnd_of(key))) }.as_bool()
}

fn centre(r: seam::zones::Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

fn on_screens(h: HWND, screens: &[Rect]) -> bool {
    winutil::visible_rect(h).is_some_and(|r| {
        let (x, y) = centre(r);
        r.w > 0 && r.h > 0 && screens.iter().any(|s| s.contains(x, y))
    })
}

/// The screen the focused window is on, if it is on one.
fn screen_of(h: HWND, screens: &[Rect]) -> Option<Rect> {
    let (x, y) = centre(winutil::visible_rect(h)?);
    screens.iter().find(|s| s.contains(x, y)).copied()
}

fn run(job: &Job, held: &mut Held) -> Result<Value, String> {
    if FORGET.swap(false, Ordering::SeqCst) {
        *held = Held::default();
    }
    let screen = match job.scope {
        FocusScope::Screen => screen_of(hwnd_of(job.hwnd), &job.screens),
        FocusScope::Workspace => None,
    };
    // Pressing again puts back what the last press minimized, if it was the same kind
    // of press. If the user restored all of it meanwhile, there is nothing to put back
    // and this press minimizes.
    let same = held.scope == Some(job.scope) && held.screen == screen;
    if same && held.windows.iter().any(|k| exists(*k)) {
        let restored = bring_back(held, job.from_api);
        if restored > 0 {
            return Ok(json!({ "ok": true, "restored": restored }));
        }
    }
    held.windows.clear();

    let focused = hwnd_of(job.hwnd);
    let screens = match job.scope {
        FocusScope::Screen => {
            if focused.is_invalid() || !unsafe { IsWindow(Some(focused)) }.as_bool() {
                return Err("No window has focus".into());
            }
            if !winutil::is_capturable(focused, std::process::id()) {
                return Err("Focus on this window needs an app window in front".into());
            }
            vec![screen.ok_or("That window isn't on a screen")?]
        }
        FocusScope::Workspace => job.screens.clone(),
    };
    let candidates: Vec<HWND> = match (job.scope, &job.workspace) {
        (FocusScope::Workspace, Some(ws)) => switcher::outside(ws, &job.exclude)?.into_iter().map(hwnd_of).collect(),
        (FocusScope::Workspace, None) => return Err("No workspace is in use on this desk".into()),
        (FocusScope::Screen, _) => {
            let own = std::process::id();
            let stays = winutil::PIN_STICKY | winutil::PIN_TOP;
            let excluded = |h: HWND| winutil::exe_name(h).is_some_and(|name| job.exclude.contains(&name));
            winutil::top_level_windows()
                .into_iter()
                .filter(|h| winutil::is_capturable(*h, own) && winutil::pin_bits(*h) & stays == 0)
                .filter(|h| !winutil::is_elevated_over_us(*h) && !excluded(*h))
                .collect()
        }
    };
    // The window that owns the focused one, or that it belongs to, stays with it.
    let owner = unsafe { GetWindow(focused, GW_OWNER) }.ok();
    let root = unsafe { GetAncestor(focused, GA_ROOTOWNER) };
    let minimize: Vec<isize> = candidates
        .into_iter()
        .filter(|h| {
            *h != focused
                && *h != root
                && Some(*h) != owner
                && !winutil::is_minimized(*h)
                && !winutil::is_hung(*h)
                && on_screens(*h, &screens)
        })
        .map(|h| h.0 as isize)
        .collect();
    if minimize.is_empty() {
        return Err("No other windows to minimize".into());
    }
    for key in &minimize {
        unsafe {
            let _ = ShowWindowAsync(hwnd_of(*key), SW_MINIMIZE);
        }
    }
    let minimized = minimize.len();
    // Minimizing is asynchronous. Wait for it, so a quick second press finds them minimized.
    let until = Instant::now() + RESTORE_WAIT;
    while Instant::now() < until && minimize.iter().any(|k| !winutil::is_minimized(hwnd_of(*k))) {
        std::thread::sleep(RESTORE_POLL);
    }
    *held = Held { windows: minimize, focused: job.hwnd, scope: Some(job.scope), screen };
    Ok(json!({ "ok": true, "minimized": minimized }))
}

/// Restore what is still minimized, bottom of the stack first so the order comes
/// back, then give focus to the window that had it. Returns how many it restored.
fn bring_back(held: &mut Held, nudge: bool) -> usize {
    let Held { windows, focused, .. } = std::mem::take(held);
    let minimized: Vec<HWND> = windows
        .iter()
        .rev()
        .map(|k| hwnd_of(*k))
        .filter(|h| exists(h.0 as isize) && winutil::is_minimized(*h))
        .collect();
    for h in &minimized {
        unsafe {
            let _ = ShowWindowAsync(*h, SW_RESTORE);
        }
    }
    // Restoring activates each window, so focus goes back only once they have all shown.
    let until = Instant::now() + RESTORE_WAIT;
    while Instant::now() < until && minimized.iter().any(|h| winutil::is_minimized(*h)) {
        std::thread::sleep(RESTORE_POLL);
    }
    if !minimized.is_empty() && exists(focused) {
        wincontrol::focus(hwnd_of(focused), nudge);
    }
    minimized.len()
}
