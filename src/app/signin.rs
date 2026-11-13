//! "When I sign in": what the sign-in timer does for each mode, and opening a chosen workspace.

use super::*;
use seam::config::{SignIn, SignInMode};

/// One tick of the sign-in timer, for the mode of this desk. Returns whether that's
/// settled, done or with nothing to do.
pub(super) fn sign_in_tick(app: &mut App) -> bool {
    let Some(chosen) = app.cfg.desks.get(&app.sig).map(|d| d.sign_in_or(app.cfg.restore_session)) else {
        // The desk isn't known yet, so wait for it a few times.
        app.session_offer_tries += 1;
        return app.session_offer_tries >= SESSION_OFFER_TRIES;
    };
    match chosen.mode {
        SignInMode::Nothing => true,
        SignInMode::LastSession => offer_session(app),
        SignInMode::Workspace => open_workspace(app, &chosen),
    }
}

/// Switch to the workspace chosen for sign-in, or ask first. Waits while a full-screen
/// app is in front. A pause by hand cancels it.
fn open_workspace(app: &mut App, chosen: &SignIn) -> bool {
    match standby::reason() {
        Some(standby::Standby::Paused) => {
            log::info!("sign-in workspace: Seam is paused, so nothing opens");
            return true;
        }
        Some(standby::Standby::FullScreen) => return false,
        None => {}
    }
    let found = chosen.workspace.as_deref().and_then(|id| desk_workspaces(app).into_iter().find(|w| w.id == id));
    let Some(workspace) = found else {
        log::info!("sign-in workspace: not on this desk any more, so nothing opens");
        return true;
    };
    app.session_offer_tries += 1;
    let gave_up = app.session_offer_tries >= SESSION_OFFER_TRIES;
    if chosen.ask {
        app.sign_in_offer = Some(workspace.id.clone());
        let body = format!("Open {}? Click to open", workspace.name);
        return notify(app, "Workspace", &body, BalloonAction::OpenWorkspace) || gave_up;
    }
    // The shell is up when its taskbar exists, the same moment a notice can be shown.
    if unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.is_ok() {
        switch_to(app, &workspace.id);
        return true;
    }
    gave_up
}

/// The click on "Open <workspace>?". A pause made meanwhile cancels it.
pub(super) fn open_offered(app: &mut App) {
    let Some(id) = app.sign_in_offer.take() else { return };
    if standby::paused() {
        log::info!("sign-in workspace: Seam is paused, so nothing opens");
        return;
    }
    switch_to(app, &id);
}

fn switch_to(app: &mut App, id: &str) {
    if let Err(e) = start_switch(app, Some(id), None, false) {
        log::info!("sign-in workspace: {}", e.message);
    }
}
