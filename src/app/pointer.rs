//! The pointer remap, Windows' cursor easing and Line Up.

use super::*;

pub(super) fn set_remap(app: &mut App, on: bool) {
    let turned_on = on && !app.cfg.enabled;
    app.cfg.enabled = on;
    persist(app);
    relink(app);
    if turned_on {
        warn_about_ease(app);
    }
    publish(app);
}

/// The remap needs this Windows setting, so say so if it's off. Read again each
/// time, since it has no change event. While the panel is open it asks instead
/// and shows the setting on Displays, so the balloon is only for when it's closed.
pub(super) fn warn_about_ease(app: &mut App) {
    app.cursor_easing = ease::read();
    let panel_open = app.clients.iter().any(|c| c == "panel");
    if app.links.is_empty() || app.ease_warned || app.cursor_easing != Some(false) || panel_open {
        return;
    }
    log::info!("Ease cursor movement is off");
    app.ease_warned = notify(
        app,
        "Ease cursor movement is off",
        "Click to turn it on in Seam, or the pointer can stop at screen edges.",
        BalloonAction::OpenPanel("displays"),
    );
}

/// Turn on Windows' "Ease cursor movement between displays". Seam never turns it off.
pub(super) fn set_cursor_easing(app: &mut App, on: bool) -> Reply {
    if !on {
        return refused("Seam only turns Ease cursor movement on. Turn it off in Display settings");
    }
    let before = ease::read();
    let written = ease::turn_on();
    app.cursor_easing = ease::read();
    log::info!("cursor easing: {before:?} to {:?}, {written:?}", app.cursor_easing);
    if let Err(why) = written {
        publish(app);
        return refused(why);
    }
    // Nothing is left to ask about.
    if !app.cfg.cursor_easing_asked {
        app.cfg.cursor_easing_asked = true;
        persist(app);
    }
    // The broadcast waits on other apps' windows, so it stays off this thread.
    if std::thread::Builder::new().name("seam-easing".into()).spawn(ease::announce).is_err() {
        log::info!("cursor easing: couldn't start the broadcast");
    }
    publish(app);
    Ok(json!({ "on": app.cursor_easing }))
}

pub(super) fn start_lineup(app: &mut App) {
    let mons = monitors::enumerate();
    let sig = signature(mons.iter().map(|m| m.id.as_str()));
    let saved = app.cfg.desks.get(&sig).map(|d| d.borders.clone()).unwrap_or_default();
    if !overlay::start(app.hwnd, mons, &saved, app.cfg.lineup_measure) {
        notify(
            app,
            "Nothing to line up",
            "Seam needs two or more screens that touch in the Windows arrangement.",
            BalloonAction::None,
        );
    }
}

pub(super) fn lineup_done(app: &mut App, accepted: bool) {
    snap::line_up_ended();
    if let (true, Some(res)) = (accepted, overlay::take_result()) {
        let mons = monitors::enumerate();
        let sig = signature(mons.iter().map(|m| m.id.as_str()));
        let links = resolve(&mons, Some(&res));
        desk::note_active(&mut app.cfg, &sig, &mons, unix_now());
        if let Some(desk) = app.cfg.desks.get_mut(&sig) {
            desk.borders = res;
            desk.lined_up_at = Some(unix_now());
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
                Err(e) => format!("Pointer remap is on. Windows arrangement unchanged. {e}."),
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
