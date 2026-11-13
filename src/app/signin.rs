//! "When I sign in": what the sign-in timer does for each mode, and opening a chosen workspace.

use super::*;
use seam::config::{Desk, SignIn, SignInMode};

/// How long the whole sign-in action may wait for a full-screen app, the desk and the
/// shell, counted from the tray's start.
const SIGN_IN_WINDOW: Duration = Duration::from_secs(3 * 60);
/// How long Windows gets to settle once the taskbar exists, before a switch without asking.
const SHELL_SETTLE: Duration = Duration::from_secs(10);

/// The sign-in action of this start, while it's waiting.
pub(super) struct SignInRun {
    started: Instant,
    /// When the taskbar was first seen.
    shell_up: Option<Instant>,
}

/// The tray started at sign-in: begin the clock for what I chose.
pub(super) fn begin(app: &mut App) {
    app.sign_in_run = Some(SignInRun { started: Instant::now(), shell_up: None });
}

/// One tick of the sign-in timer, for the mode of this desk. Returns whether that's
/// settled, done, cancelled or with nothing to do.
pub(super) fn sign_in_tick(app: &mut App) -> bool {
    let Some(run) = &app.sign_in_run else { return true };
    // Screens settle in the first minute, so the desk that is current now is the one that counts.
    let settled = match app.cfg.desks.get(&app.sig).map(Desk::choice) {
        None => waiting(app, "the desk isn't known yet"),
        Some(chosen) => match chosen.mode {
            // Another desk may still take its place, so wait out the time.
            SignInMode::Nothing => run.started.elapsed() >= SIGN_IN_WINDOW,
            SignInMode::LastSession => offer_session(app),
            SignInMode::Workspace => open_workspace(app, &chosen),
        },
    };
    if settled {
        app.sign_in_run = None;
    }
    settled
}

/// Not yet: false to try again at the next tick, or true after logging once that the
/// time is up.
fn waiting(app: &mut App, why: &str) -> bool {
    let over = app.sign_in_run.as_ref().is_none_or(|run| run.started.elapsed() >= SIGN_IN_WINDOW);
    if over {
        log::info!("sign-in workspace: gave up after 3 minutes, {why}");
    }
    over
}

/// Switch to the workspace chosen for sign-in, or ask first. Returns whether it's settled.
fn open_workspace(app: &mut App, chosen: &SignIn) -> bool {
    if standby::paused() {
        log::info!("sign-in workspace: Seam is paused, so nothing opens");
        return true;
    }
    let Some(id) = chosen.workspace.as_deref() else {
        log::info!("sign-in workspace: none is chosen, so nothing opens");
        return true;
    };
    if standby::reason().is_some() {
        return waiting(app, "a full-screen app stayed in front");
    }
    let Some(workspace) = desk_workspaces(app).into_iter().find(|w| w.id == id) else {
        return waiting(app, "the workspace isn't on this desk");
    };
    if chosen.ask {
        app.sign_in_offer = Some((app.sig.clone(), workspace.id.clone()));
        let body = format!("Open {}? Click to open", workspace.name);
        return notify(app, "Workspace", &body, BalloonAction::OpenWorkspace) || waiting(app, "the notice didn't show");
    }
    // The shell is up when its taskbar exists. A switch gets 10 s more to let the desktop settle.
    if unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.is_err() {
        return waiting(app, "the taskbar didn't appear");
    }
    let up = app.sign_in_run.as_mut().map(|run| *run.shell_up.get_or_insert_with(Instant::now));
    if up.is_some_and(|up| up.elapsed() >= SHELL_SETTLE) {
        switch_to(app, &workspace.id);
        return true;
    }
    waiting(app, "Windows was still settling")
}

/// The click on "Open <workspace>?". It switches only on the desk it was offered for,
/// while the workspace is still there.
pub(super) fn open_offered(app: &mut App) {
    let Some((sig, id)) = app.sign_in_offer.take() else { return };
    if standby::paused() {
        log::info!("sign-in workspace: Seam is paused, so nothing opens");
    } else if app.sig != sig {
        log::info!("sign-in workspace: the desk changed, so nothing opens");
    } else if !desk_workspaces(app).iter().any(|w| w.id == id) {
        log::info!("sign-in workspace: the workspace is gone, so nothing opens");
    } else {
        switch_to(app, &id);
    }
}

/// Seam was paused: whatever sign-in is still waiting to open is cancelled at once.
pub(super) fn cancel_sign_in(app: &mut App) {
    let offered = app.sign_in_offer.take().is_some();
    let waiting = app.sign_in_run.is_some()
        && app.cfg.desks.get(&app.sig).is_some_and(|d| d.choice().mode == SignInMode::Workspace);
    if waiting {
        app.sign_in_run = None;
        unsafe {
            let _ = KillTimer(Some(app.hwnd), TIMER_SESSION_OFFER);
        }
    }
    if offered || waiting {
        log::info!("sign-in workspace: Seam was paused, so nothing opens");
    }
}

fn switch_to(app: &mut App, id: &str) {
    if let Err(e) = start_switch(app, Some(id), None, false) {
        log::info!("sign-in workspace: {}", e.message);
    }
}
