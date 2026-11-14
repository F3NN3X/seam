//! The workspace in use on this desk: remembering it, detecting it and checking its drift.

use super::*;

/// The workspace in use on this desk, if there is one.
pub(super) fn active_workspace(app: &App) -> Option<Workspace> {
    let id = app.workspace.as_deref()?;
    desk_workspaces(app).into_iter().find(|w| w.id == id)
}

/// A workspace with what a status check or auto-apply needs to place its windows.
pub(in crate::app) fn live_of(app: &App, workspace: Workspace) -> LiveWorkspace {
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let (screens, gap, pins_stay) = (job_screens(app), app.cfg.zones.gap, app.cfg.windows.pins_stay_on_switch);
    let (size_mode, places) = (app.cfg.window_size_on_move, app.cfg.workspaces_remember_places);
    LiveWorkspace { workspace, screens, gap, pins_stay, exclude, size_mode, places }
}

/// Hand the workspace in use to the live worker, and tell the seam-wm thread what to
/// watch. Auto-apply steps aside for another window manager, as snapping does (D110).
pub(in crate::app) fn sync_live(app: &mut App) {
    let active = active_workspace(app);
    if active.is_none() {
        app.drifted = false;
    }
    let auto_apply = app.cfg.auto_apply && aside(app).is_none() && !standby::paused();
    let live = active.map(|ws| live_of(app, ws));
    live::set_active(app.hwnd.0 as isize, live, auto_apply);
    live::set_rules(app_rules_of(app), standby::reason().is_some() || aside(app).is_some());
}

/// The app rules this build can use, unless another window manager has the windows.
pub(super) fn app_rules_of(app: &App) -> Option<AppRules> {
    let rules = workspace::app_rules(&app.cfg.app_rules);
    if rules.is_empty() || aside(app).is_some() || standby::paused() {
        return None;
    }
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let (screens, gap, pins_stay) = (job_screens(app), app.cfg.zones.gap, app.cfg.windows.pins_stay_on_switch);
    let (size_mode, places) = (app.cfg.window_size_on_move, app.cfg.workspaces_remember_places);
    Some(AppRules { rules, screens, gap, pins_stay, exclude, size_mode, places })
}

/// The workspace in use changed: remember it for this desk, so a restart shows it again.
pub(super) fn set_in_use(app: &mut App, id: Option<String>) {
    app.workspace = id.clone();
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return };
    if desk.active_workspace != id {
        // Only a change to another workspace leaves one behind; losing it doesn't.
        if id.is_some() && desk.active_workspace.is_some() {
            desk.previous_workspace = desk.active_workspace.take();
        }
        desk.active_workspace = id;
        persist(app);
    }
}

/// A deleted workspace is no longer in use, remembered or the default. Returns
/// whether the config changed.
pub(in crate::app) fn forget_deleted(app: &mut App) -> bool {
    let ids: Vec<String> = desk_workspaces(app).into_iter().map(|w| w.id).collect();
    // A deleted workspace's rows remember no windows (D336). Other desks' keep theirs.
    let everywhere: Vec<String> =
        app.cfg.desks.values().flat_map(|d| workspace::list(&d.workspaces)).map(|w| w.id).collect();
    switcher::keep_bindings(&everywhere);
    let gone = |id: &Option<String>| id.as_ref().is_some_and(|id| !ids.contains(id));
    if gone(&app.workspace) {
        app.workspace = None;
    }
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return false };
    let mut changed = false;
    for slot in [&mut desk.active_workspace, &mut desk.previous_workspace, &mut desk.default_workspace] {
        if gone(slot) {
            *slot = None;
            changed = true;
        }
    }
    changed
}

/// At start and on another desk, which workspace is in use, without moving a window:
/// the one remembered for this desk, else its default, else the only one whose
/// running apps are all in place, which the live worker looks for.
pub(in crate::app) fn choose_in_use(app: &mut App) {
    let list = desk_workspaces(app);
    let desk = app.cfg.desks.get(&app.sig);
    let here = |id: Option<&String>| id.filter(|id| list.iter().any(|w| w.id == **id)).cloned();
    let remembered = here(desk.and_then(|d| d.active_workspace.as_ref())).map(|id| (id, "remembered"));
    let chosen = remembered.or_else(|| here(desk.and_then(|d| d.default_workspace.as_ref())).map(|id| (id, "default")));
    match chosen {
        Some((id, why)) => {
            log::info!("workspace in use at start: {id} ({why})");
            set_in_use(app, Some(id));
            sync_live(app);
            live::check_drift();
        }
        None if !list.is_empty() => {
            let lives = list.into_iter().map(|w| live_of(app, w)).collect();
            live::detect(app.sig.clone(), lives);
        }
        None => log::info!("workspace in use at start: none, this desk has no workspaces"),
    }
}

/// The live worker found which workspace looks like the one in use.
pub(in crate::app) fn detected_in_use(app: &mut App) {
    let Some(found) = live::take_detected() else { return };
    // Another desk since, or a switch got there first.
    if found.sig != app.sig || app.workspace.is_some() {
        return;
    }
    let Some(id) = found.id.filter(|id| desk_workspaces(app).iter().any(|w| w.id == *id)) else {
        log::info!("workspace in use at start: none detected");
        return;
    };
    log::info!("workspace in use at start: {id} (detected)");
    set_in_use(app, Some(id));
    sync_live(app);
    live::check_drift();
    publish(app);
}

/// The workspace taken as in use at start when none is remembered, or none.
pub(in crate::app) fn set_default(app: &mut App, id: Option<String>) -> Reply {
    if id.as_ref().is_some_and(|id| !desk_workspaces(app).iter().any(|w| w.id == *id)) {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    }
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return refused("This desk has no workspaces") };
    desk.default_workspace = id.clone();
    persist(app);
    publish(app);
    Ok(json!({ "id": id }))
}

/// Snap back: switch to the workspace in use again, so what drifted goes back.
pub(in crate::app) fn snap_back(app: &mut App) -> Reply {
    match app.workspace.clone() {
        Some(id) => start_switch(app, Some(&id), None, false),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace is in use on this desk")),
    }
}

/// Switch back: switch to the workspace that was in use before this one. Switching
/// back twice goes to and fro between the two.
pub(in crate::app) fn switch_back(app: &mut App) -> Reply {
    let previous = app.cfg.desks.get(&app.sig).and_then(|d| d.previous_workspace.clone());
    match previous.filter(|id| desk_workspaces(app).iter().any(|w| w.id == *id)) {
        Some(id) => start_switch(app, Some(&id), None, false),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace to switch back to")),
    }
}

/// The seam-wm thread saw a window a workspace placed move or minimize.
pub(in crate::app) fn ws_moved(app: &mut App, key: u32) {
    if !app.moved.contains(&key) {
        app.moved.push(key);
    }
    unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
}

/// Things have been still for a moment: tell the panel which workspaces' windows
/// moved, and have the worker look whether the one in use drifted.
pub(in crate::app) fn drift_due(app: &mut App) {
    // Moves wait while paused. Resuming looks again.
    if standby::paused() {
        return;
    }
    let moved = std::mem::take(&mut app.moved);
    for ws in desk_workspaces(app).into_iter().filter(|w| moved.contains(&workspace::stamp_key(&w.id))) {
        api_server::broadcast("workspace.moved", &json!({ "id": ws.id }));
    }
    let active = app.workspace.as_deref().map(workspace::stamp_key);
    if std::mem::take(&mut app.recheck) || active.is_some_and(|key| moved.contains(&key)) {
        live::check_drift();
    }
}

/// The worker looked at the workspace in use.
pub(in crate::app) fn drift_checked(app: &mut App) {
    let Some(drift) = live::take_drift() else { return };
    if drift.placed {
        api_server::broadcast("workspace.moved", &json!({ "id": drift.id }));
        windows_changed(app);
    }
    if app.workspace.as_deref() == Some(drift.id.as_str()) && drift.drifted != app.drifted {
        log::info!("workspace {}: {}", drift.id, if drift.drifted { "drifted" } else { "in place" });
        app.drifted = drift.drifted;
        publish(app);
    }
}
