//! Refreshing what the tray knows about the screens and publishing it to API clients.

use super::*;

/// Re-read monitors, then relink.
pub(super) fn refresh(app: &mut App) {
    if overlay::is_active() {
        return;
    }
    app.mons = monitors::enumerate();
    spotlight::screens_changed(&app.mons);
    // A display change can make a window cover its screen, or stop it.
    standby_check(app);
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
    if arrived {
        choose_in_use(app);
    }
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

pub(super) fn snapshot(app: &App) -> Snapshot {
    let desk_zones = app.cfg.desks.get(&app.sig).map(|d| &d.zones);
    let screens = app.mons.iter().enumerate().map(|(i, m)| Screen {
        number: i as u32 + 1,
        id: m.id.clone(),
        name: m.name.clone(),
        primary: m.primary,
        rect: ScreenRect { x: m.rect.left, y: m.rect.top, w: m.rect.width(), h: m.rect.height() },
        layout: seam::library::layout_id(desk_zones.and_then(|z| z.get(&m.id)).and_then(zones::layout_of).as_ref()),
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
        spotlight: spotlight::is_on(),
        paused: standby::paused(),
        stepped_aside: standby::reason() == Some(standby::Standby::FullScreen),
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
        windows_cursor_easing: app.cursor_easing,
    };
    Snapshot { state, config: serde_json::to_value(&app.cfg).unwrap_or_default() }
}

/// Tell API clients what the tray is doing now.
pub(super) fn publish(app: &mut App) {
    if api_server::publish(snapshot(app), app.cfg.api.enabled) {
        send_state_event(app);
    }
}

/// At most one `state.changed` per 100 ms. The last change always goes out,
/// because the timer sends whatever the state is by then.
pub(super) fn send_state_event(app: &mut App) {
    if app.last_state_event.is_some_and(|sent| sent.elapsed() < STATE_EVENT_GAP) {
        unsafe { SetTimer(Some(app.hwnd), TIMER_STATE, STATE_EVENT_GAP.as_millis() as u32, None) };
        return;
    }
    app.last_state_event = Some(Instant::now());
    let state = serde_json::to_value(snapshot(app).state).unwrap_or_default();
    api_server::broadcast("state.changed", &state);
}
