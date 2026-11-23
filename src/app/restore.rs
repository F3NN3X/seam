//! "Offer to restore after screens change" (D746-D749): after a display change or a wake, ask
//! whether to put the workspace in use back if its windows got moved.

use super::*;
use seam::config::SignInMode;
use seam::workspace::OfferFacts;
use std::sync::atomic::{AtomicBool, Ordering};

/// The screens changed and their refresh hasn't run. Set before the app is taken, so it holds
/// even when a tray menu has the app and the cancel can't run.
pub(super) static CHANGE_PENDING: AtomicBool = AtomicBool::new(false);

/// The longest a re-fit that was already going can take to end after a change, which is the
/// debounce a re-fit of that change waits before it starts.
const REFIT_AFTER_CHANGE: Duration = Duration::from_millis(1500);

/// The balloon went away without a click: its click target and the sign-in offer it carried go too.
pub(super) fn balloon_faded(app: &mut App) {
    if app.balloon_action == BalloonAction::OpenWorkspace {
        app.balloon_action = BalloonAction::None;
        app.sign_in_offer = None;
        app.sign_in_offered_at = None;
    }
}

/// The screens changed or the PC woke, and the refresh has run: look once they've settled.
/// Keep windows in their zones goes first, so its end starts the look. A later change arms it again.
pub(super) fn arm_restore_offer(app: &mut App) {
    if !app.cfg.workspaces_offer_restore {
        return;
    }
    app.restore_armed = true;
    // A re-fit that already finished since the change has nothing left to wait for.
    let waits = app.cfg.zones.refit && !app.refit_done;
    let after = if waits { RESTORE_OFFER_FALLBACK_MS } else { RESTORE_OFFER_AFTER_MS };
    unsafe { SetTimer(Some(app.hwnd), TIMER_RESTORE_OFFER, after, None) };
}

/// The screens changed again: whatever was waiting to look is dropped, and the refresh that
/// follows arms it anew (D747, a later change replaces the offer).
pub(super) fn cancel_restore_offer(app: &mut App) {
    app.restore_armed = false;
    app.restore_check = None;
    app.refit_done = false;
    app.screens_changed_at = Some(Instant::now());
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_RESTORE_OFFER);
    }
}

/// The refresh of a change runs: the same cancel again, for when the change's own was skipped
/// because a tray menu held the app. A change that was already noted keeps its time.
pub(super) fn settle_restore_offer(app: &mut App) {
    CHANGE_PENDING.store(false, Ordering::SeqCst);
    let noted = app.screens_changed_at;
    cancel_restore_offer(app);
    if noted.is_some_and(|at| at.elapsed() < REFIT_AFTER_CHANGE) {
        app.screens_changed_at = noted;
    }
}

/// A re-fit run ended. It counts for this change only if it started after it, which is at least
/// the debounce later: a run that was already going ended too soon to be this change's.
/// The look starts now if one was waiting for it.
pub(super) fn refit_ended(app: &mut App) {
    if app.screens_changed_at.is_some_and(|at| at.elapsed() < REFIT_AFTER_CHANGE) {
        return;
    }
    app.refit_done = true;
    if app.restore_armed {
        restore_look(app);
    }
}

/// The screens have settled: have the worker say whether the workspaces in use have windows out of place.
pub(super) fn restore_look(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_RESTORE_OFFER);
    }
    // A change whose refresh hasn't run yet (a tray menu holds the app) isn't settled.
    if !std::mem::take(&mut app.restore_armed) || CHANGE_PENDING.load(Ordering::SeqCst) || in_use_ids(app).is_empty() {
        return;
    }
    // The live worker judges against the screens as they are now.
    sync_live(app);
    app.restore_check = Some(Instant::now());
    live::check_drift();
}

/// The worker answered for every workspace in use, and `moved` have windows out of place: ask
/// whether to put them back (D786).
pub(super) fn restore_checked(app: &mut App, moved: &[String]) {
    let asked = app.restore_check.take().is_some_and(|at| at.elapsed() < RESTORE_CHECK_FOR);
    if !asked {
        return;
    }
    let sign_in_mode = app.cfg.desks.get(&app.sig).map(|d| d.choice().mode).unwrap_or(SignInMode::Nothing);
    let facts = OfferFacts {
        on: app.cfg.workspaces_offer_restore,
        in_use: true,
        moved: !moved.is_empty(),
        busy: switcher::switching()
            || app.capturing.is_some()
            || app.closing.is_some()
            || overlay::is_active()
            || workspace::sign_in_pending(
                app.sign_in_run.is_some() && sign_in_mode != SignInMode::Nothing,
                app.sign_in_offer.is_some().then(|| app.sign_in_offered_at.map(|at| at.elapsed())).flatten(),
                app.balloon_action == BalloonAction::OpenWorkspace,
            ),
        standing_by: standby::paused() || standby::reason().is_some(),
        new_desk_balloon: app.new_desk_shown,
    };
    if !workspace::should_offer(facts) {
        return;
    }
    let list = desk_workspaces(app);
    let named: Vec<&Workspace> = moved.iter().filter_map(|id| list.iter().find(|w| w.id == *id)).collect();
    let body = match named.as_slice() {
        [] => return,
        [one] => format!("Restore {}? Click to restore", one.name),
        several => format!("Restore {} workspaces? Click to restore", several.len()),
    };
    app.restore_offer = Some((app.sig.clone(), named.iter().map(|w| w.id.clone()).collect()));
    notify(app, "Screens changed", &body, BalloonAction::RestoreAfterScreens);
}

/// The click on "Restore <workspace>?". It snaps back on the desk it was offered for each
/// workspace that is still there and still in use, one after another, and leaves other windows alone.
pub(super) fn restore_offered(app: &mut App) {
    let Some((sig, ids)) = app.restore_offer.take() else { return };
    let in_use = in_use_ids(app);
    let still: Vec<String> = ids.into_iter().filter(|id| in_use.contains(id)).collect();
    if standby::paused() {
        log::info!("restore after screens change: Seam is paused, so nothing moves");
    } else if app.sig != sig {
        log::info!("restore after screens change: the desk changed, so nothing moves");
    } else if still.is_empty() {
        log::info!("restore after screens change: the workspace isn't in use any more, so nothing moves");
    } else {
        app.put_back_queue = still.into_iter().map(|id| (id, true)).collect();
        if let Some(Err(e)) = next_put_back(app) {
            log::info!("restore after screens change: {}", e.message);
            notify(app, "Screens changed", &e.message, BalloonAction::None);
        }
    }
}
