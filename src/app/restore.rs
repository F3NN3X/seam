//! "Offer to restore after screens change" (D746-D749): after a display change or a wake, ask
//! whether to put the workspace in use back if its windows got moved.

use super::*;
use seam::config::SignInMode;
use seam::workspace::OfferFacts;

/// The screens changed or the PC woke, and the refresh has run: look once they've settled.
/// Keep windows in their zones goes first, so its end starts the look. A later change arms it again.
pub(super) fn arm_restore_offer(app: &mut App) {
    if !app.cfg.workspaces_offer_restore {
        return;
    }
    app.restore_armed = true;
    let after = if app.cfg.zones.refit { RESTORE_OFFER_FALLBACK_MS } else { RESTORE_OFFER_AFTER_MS };
    unsafe { SetTimer(Some(app.hwnd), TIMER_RESTORE_OFFER, after, None) };
}

/// The screens have settled: have the worker say whether the workspace in use has windows out of place.
pub(super) fn restore_look(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_RESTORE_OFFER);
    }
    if !std::mem::take(&mut app.restore_armed) || app.workspace.is_none() {
        return;
    }
    app.restore_check = Some(Instant::now());
    live::check_drift();
}

/// The worker answered for `id`: ask if everything is in place for it.
pub(super) fn restore_checked(app: &mut App, id: &str, moved: bool) {
    let asked = app.restore_check.take().is_some_and(|at| at.elapsed() < RESTORE_CHECK_FOR);
    if !asked || app.workspace.as_deref() != Some(id) {
        return;
    }
    let sign_in_mode = app.cfg.desks.get(&app.sig).map(|d| d.choice().mode).unwrap_or(SignInMode::Nothing);
    let facts = OfferFacts {
        on: app.cfg.workspaces_offer_restore,
        in_use: true,
        moved,
        busy: switcher::switching()
            || app.capturing.is_some()
            || app.closing.is_some()
            || overlay::is_active()
            || app.sign_in_offer.is_some()
            || (app.sign_in_run.is_some() && sign_in_mode != SignInMode::Nothing),
        standing_by: standby::paused() || standby::reason().is_some(),
        new_desk_balloon: app.new_desk_shown,
    };
    if !workspace::should_offer(facts) {
        return;
    }
    let Some(workspace) = desk_workspaces(app).into_iter().find(|w| w.id == id) else { return };
    app.restore_offer = Some((app.sig.clone(), workspace.id.clone()));
    let body = format!("Restore {}? Click to restore", workspace.name);
    notify(app, "Screens changed", &body, BalloonAction::RestoreAfterScreens);
}

/// The click on "Restore <workspace>?". It snaps back on the desk it was offered for, while the
/// workspace is still there and still in use, and leaves other windows alone.
pub(super) fn restore_offered(app: &mut App) {
    let Some((sig, id)) = app.restore_offer.take() else { return };
    if standby::paused() {
        log::info!("restore after screens change: Seam is paused, so nothing moves");
    } else if app.sig != sig {
        log::info!("restore after screens change: the desk changed, so nothing moves");
    } else if app.workspace.as_deref() != Some(id.as_str()) || !desk_workspaces(app).iter().any(|w| w.id == id) {
        log::info!("restore after screens change: the workspace isn't in use any more, so nothing moves");
    } else if let Err(e) = start_switch(app, Some(&id), None, true, Via::local().snapping()) {
        log::info!("restore after screens change: {}", e.message);
    }
}
