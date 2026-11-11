//! The last session, Windows' Restart apps and the pinned-close reopen: moved out of `app.rs`.

use super::*;

// ---- Last session (D120)

/// The last session saved for the desk that is connected.
pub(super) fn saved_session(app: &App) -> Option<Session> {
    app.cfg.desks.get(&app.sig)?.last_session.as_ref().and_then(Session::parse)
}

/// Seam pinned, placed or switched, so save the last session once things have been
/// quiet for a while. A newer change starts the wait again.
pub(super) fn windows_changed(app: &App) {
    if app.cfg.restore_session {
        unsafe { SetTimer(Some(app.hwnd), TIMER_SESSION, SESSION_SAVE_AFTER_MS, None) };
    }
}

/// Capture the last session on a worker. One at a time.
pub(super) fn start_session_capture(app: &mut App) {
    if !app.cfg.restore_session || app.session_capture.is_some() {
        return;
    }
    if switcher::capture_session(capture_job(app)) {
        app.session_capture = Some(app.sig.clone());
    }
}

/// Keep a finished capture as the desk's last session. An empty one is more likely
/// apps already gone at sign-out than a choice, so the last saved one stays.
pub(super) fn store_session(app: &mut App, rules: Vec<Rule>) {
    let Some(sig) = app.session_capture.take() else { return };
    if rules.is_empty() || !app.cfg.restore_session {
        return;
    }
    let Some(desk) = app.cfg.desks.get_mut(&sig) else { return };
    log::info!("last session saved: {} windows", rules.len());
    desk.last_session = Some(Session { saved_at: unix_now(), windows: rules }.to_value());
    persist(app);
    publish(app);
}

pub(super) fn session_captured(app: &mut App) {
    if let Some(rules) = switcher::take_session() {
        store_session(app, rules);
    }
}

/// Windows asks whether the session may end. Capture now, while the apps are still
/// there, and never hold up the sign-out.
pub(super) fn query_end_session(app: &mut App) {
    app.session_ending = true;
    start_session_capture(app);
}

/// Windows is ending the session: finish the capture started at
/// `WM_QUERYENDSESSION` and write it out before the process goes. A capture that
/// isn't done in time leaves the last saved session as it was.
pub(super) fn end_session(app: &mut App) {
    if !app.cfg.restore_session {
        return;
    }
    // Later than the question, apps may already be closing, so only capture now if it was missed.
    if !app.session_ending {
        start_session_capture(app);
    }
    if app.session_capture.is_some() {
        match switcher::wait_session(SESSION_END_WAIT) {
            Some(rules) => store_session(app, rules),
            None => log::info!("last session not captured in time, the saved one stays"),
        }
    }
    if let Some(saver) = app.saver.take() {
        saver.finish();
    }
}

/// After a sign-in, offer the last session once. Returns whether that's settled,
/// shown or with nothing to offer.
pub(super) fn offer_session(app: &mut App) -> bool {
    let Some(session) = saved_session(app).filter(|_| app.cfg.restore_session) else { return true };
    // The click restores what was offered, even if a save replaces it meanwhile.
    app.session_offer = Some(session.workspace());
    let shown =
        notify(app, "Last session", "Restore your last session? Click to restore", BalloonAction::RestoreSession);
    app.session_offer_tries += 1;
    shown || app.session_offer_tries >= SESSION_OFFER_TRIES
}

/// Run the last session as a workspace: nothing else is touched, and the pins come
/// back by rule. Seam starts the missing apps as captured only when it reopens them;
/// otherwise Windows or the apps do, and Seam places each window as it appears (D128).
pub(super) fn restore_session(app: &mut App, session: Workspace) -> Reply {
    let mode = app.cfg.restore_apps;
    log::info!("restoring the last session: {} windows, apps reopened by {mode:?}", session.windows.len());
    let arrange_for = (mode == RestoreApps::Windows).then_some(ARRANGE_WAIT);
    start_switch_job(app, session, arrange_for, false)
}

// ---- Windows' Restart apps (D128)

/// Seam reopens the apps, so Windows' Restart apps would open them a second time.
pub(super) fn holds_restart_apps(cfg: &Config) -> bool {
    cfg.restore_session && cfg.restore_apps == RestoreApps::Seam
}

/// Turn Windows' Restart apps off while Seam reopens the apps, keeping the value it
/// had the first time, and give that value back when Seam stops. A failed write is
/// logged and the mode stays as chosen.
pub(super) fn sync_restart_apps(app: &mut App) {
    let wanted = holds_restart_apps(&app.cfg);
    match (wanted, app.cfg.restore_apps_saved) {
        (true, None) => match restart_apps::read() {
            Ok(found) => {
                app.cfg.restore_apps_saved = Some(found);
                persist(app);
                match restart_apps::write(0) {
                    Ok(()) => log::info!("Windows' Restart apps turned off (was {found:#x})"),
                    Err(e) => log::info!("Windows' Restart apps not turned off: {e:?}"),
                }
            }
            Err(e) => log::info!("Windows' Restart apps not read: {e:?}"),
        },
        (false, Some(saved)) => {
            match restart_apps::write(saved) {
                Ok(()) => log::info!("Windows' Restart apps given back ({saved:#x})"),
                Err(e) => log::info!("Windows' Restart apps not given back: {e:?}"),
            }
            app.cfg.restore_apps_saved = None;
            persist(app);
        }
        _ => {}
    }
    app.restart_apps = restart_apps::is_on();
}

/// At start: the user may have turned Restart apps back on in Windows. Seam doesn't
/// turn it off again, it says so in the panel.
pub(super) fn check_restart_apps(app: &mut App) {
    if holds_restart_apps(&app.cfg) && app.cfg.restore_apps_saved.is_some() {
        app.restart_apps = restart_apps::is_on();
        if app.restart_apps {
            log::info!("Windows' Restart apps is on again, so apps may open twice");
        }
        return;
    }
    sync_restart_apps(app);
}

// ---- Pinned window closed (D119)

pub(super) fn pin_closed(app: &mut App) {
    let Some(gone) = wm::take_closed() else { return };
    if !app.cfg.windows.notify_pinned_close {
        return;
    }
    let body = format!("{} was pinned and has closed. Click to reopen", switcher::closed_name(&gone));
    // One notice at a time: a second close replaces what the click reopens.
    app.reopen = Some(gone);
    notify(app, "Pinned window closed", &body, BalloonAction::Reopen);
}

pub(super) fn reopen_closed(app: &mut App) {
    if let Some(gone) = app.reopen.take() {
        if !switcher::reopen(gone) {
            log::info!("reopen: couldn't start the worker");
        }
    }
}
