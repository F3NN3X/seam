//! The workspaces in use on this desk: the desk-wide one and each screen's own (D782),
//! remembering them, detecting the desk-wide one and checking their drift.

use super::*;

/// Which screen a workspace action means when it names no workspace (D785).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum OnScreen {
    /// A Line Up number, from the API.
    Number(u32),
    /// The one under the pointer, for a shortcut.
    Pointer,
    /// None named: the tray, or the API without `screen`.
    Unnamed,
}

/// The screen an action means, `None` when none is named.
pub(in crate::app) fn screen_meant(app: &App, on: OnScreen) -> Result<Option<String>, RpcError> {
    match on {
        OnScreen::Number(n) => {
            tiling::screen_of(app, Some(n)).map(Some).ok_or_else(|| RpcError::new(api::NOT_FOUND, "no such screen"))
        }
        OnScreen::Pointer => Ok(tiling::screen_of(app, None)),
        OnScreen::Unnamed => Ok(None),
    }
}

/// The ids of the connected screens, in Line Up order.
pub(in crate::app) fn screen_ids(app: &App) -> Vec<&str> {
    app.mons.iter().map(|m| m.id.as_str()).collect()
}

/// The desk-wide workspace in use on this desk, if there is one.
pub(super) fn active_workspace(app: &App) -> Option<Workspace> {
    let id = app.workspace.as_deref()?;
    desk_workspaces(app).into_iter().find(|w| w.id == id)
}

/// Every workspace in use on a connected screen, each once: the desk-wide one first, then the
/// screens' own in Line Up order.
pub(in crate::app) fn in_use_ids(app: &App) -> Vec<String> {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return Vec::new() };
    let held = workspace::holds(desk, &screen_ids(app));
    let mut ids: Vec<String> = app.workspace.iter().filter(|id| held.values().any(|h| h == *id)).cloned().collect();
    for screen in screen_ids(app) {
        if let Some(id) = held.get(screen).filter(|id| !ids.contains(id)) {
            ids.push(id.clone());
        }
    }
    ids
}

/// The workspace in use on `screen`, or with none named, the one last switched to that is
/// still in use, else the desk-wide one (D785).
pub(in crate::app) fn meant(app: &App, screen: Option<&str>) -> Option<String> {
    let desk = app.cfg.desks.get(&app.sig);
    match screen {
        Some(screen) => desk.and_then(|d| workspace::in_use_on(d, screen)).map(str::to_string),
        None => app.last_in_use.clone().filter(|id| in_use_ids(app).contains(id)).or_else(|| app.workspace.clone()),
    }
}

/// The screens a switch to `ws` leaves alone, with the stamp key of the workspace in use on
/// each, for the windows they hold (D783). `reach` says which screens the switch takes.
pub(in crate::app) fn holders(app: &App, ws: &str, reach: &dyn Fn(&str) -> bool) -> Vec<(zones::Rect, u32)> {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return Vec::new() };
    let rect = |m: &Mon| zones::Rect { x: m.rect.left, y: m.rect.top, w: m.rect.width(), h: m.rect.height() };
    let held = |m: &&Mon| !reach(&m.id);
    let holder = |m: &Mon| {
        let id = workspace::in_use_on(desk, &m.id).filter(|id| *id != ws)?;
        Some((rect(m), workspace::stamp_key(id)))
    };
    app.mons.iter().filter(held).filter_map(holder).collect()
}

/// A workspace with what a status check or auto-apply needs to place its windows: a one-screen
/// workspace has its screen, and a desk-wide one the screens that have no workspace of their own (D786).
pub(in crate::app) fn live_of(app: &App, workspace: Workspace) -> LiveWorkspace {
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let away = match (&workspace.scope, app.cfg.desks.get(&app.sig)) {
        (None, Some(desk)) => workspace::own_screens(desk, &screen_ids(app)),
        _ => Vec::new(),
    };
    let reach = |s: &SwitchScreen| workspace.scope.as_deref().is_none_or(|own| own == s.id) && !away.contains(&s.id);
    let screens = job_screens(app).into_iter().filter(reach).collect();
    let (gap, pins_stay) = (app.cfg.zones.gap, app.cfg.windows.pins_stay_on_switch);
    let (size_mode, places) = (app.cfg.window_size_on_move, app.cfg.workspaces_remember_places);
    LiveWorkspace { workspace, screens, gap, pins_stay, exclude, size_mode, places, away }
}

/// Hand the workspaces in use to the live worker, and tell the seam-wm thread what to
/// watch. Auto-apply steps aside for another window manager, as snapping does (D110).
pub(in crate::app) fn sync_live(app: &mut App) {
    let list = desk_workspaces(app);
    let ids = in_use_ids(app);
    let active: Vec<Workspace> = ids.iter().filter_map(|id| list.iter().find(|w| w.id == *id).cloned()).collect();
    app.drifted.retain(|id, _| active.iter().any(|w| w.id == *id));
    let auto_apply = app.cfg.auto_apply && aside(app).is_none() && !standby::paused();
    let lives = active.into_iter().map(|ws| live_of(app, ws)).collect();
    live::set_active(app.hwnd.0 as isize, lives, auto_apply);
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

/// The desk-wide workspace in use changed: remember it for this desk, so a restart shows it
/// again. The screens' own stay as they are.
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

/// A switch to `id`, for `scope`'s screen or all screens, ended: it is in use where it
/// switched, and what was there before is remembered for Switch back (D783, D784).
pub(super) fn took_over(app: &mut App, id: &str, scope: Option<&str>) {
    let ids: Vec<String> = screen_ids(app).into_iter().map(String::from).collect();
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else {
        if scope.is_none() {
            app.workspace = Some(id.to_string());
        }
        return;
    };
    let before = desk.clone();
    workspace::after_switch(desk, id, scope, &ids.iter().map(String::as_str).collect::<Vec<_>>());
    app.workspace = desk.active_workspace.clone();
    if *desk != before {
        persist(app);
    }
}

/// A deleted workspace is no longer in use, remembered or the default, on the desk or on any
/// screen. Returns whether the config changed.
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
    if gone(&app.last_in_use) {
        app.last_in_use = None;
    }
    if gone(&app.step_to) {
        app.step_to = None;
    }
    app.put_back_queue.retain(|(id, _)| ids.contains(id));
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return false };
    // A screen whose workspace went falls back to the desk-wide one.
    workspace::forget(desk, &ids)
}

/// A workspace's scope changed in a save: what is in use follows it (brief J, edge cases).
pub(super) fn rescoped(app: &mut App, id: &str, old: Option<&str>, new: Option<&str>) {
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return };
    let before = desk.clone();
    workspace::rescope(desk, id, old, new);
    if *desk != before {
        app.workspace = desk.active_workspace.clone();
        persist(app);
        sync_live(app);
    }
}

/// At start and on another desk, which workspaces are in use, without moving a window:
/// each screen's own that is still for that screen (D790), and for the desk the one
/// remembered, else its default, else the only one whose running apps are all in place,
/// which the live worker looks for. Detection is for All-screens workspaces only.
pub(in crate::app) fn choose_in_use(app: &mut App) {
    let list = desk_workspaces(app);
    if let Some(desk) = app.cfg.desks.get_mut(&app.sig)
        && workspace::keep_valid(desk, &list)
    {
        persist(app);
    }
    app.last_in_use = None;
    let desk = app.cfg.desks.get(&app.sig);
    let here = |id: Option<&String>| id.filter(|id| list.iter().any(|w| w.id == **id)).cloned();
    let remembered = here(desk.and_then(|d| d.active_workspace.as_ref())).map(|id| (id, "remembered"));
    let chosen = remembered.or_else(|| here(desk.and_then(|d| d.default_workspace.as_ref())).map(|id| (id, "default")));
    let own = desk.map(|d| d.screen_in_use.values().filter(|e| e.workspace.is_some()).count()).unwrap_or(0);
    if own > 0 {
        log::info!("workspaces in use at start: {own} screens have their own");
    }
    match chosen {
        Some((id, why)) => {
            log::info!("workspace in use at start: {id} ({why})");
            set_in_use(app, Some(id));
            sync_live(app);
            live::check_drift();
        }
        None if list.iter().any(|w| w.scope.is_none()) => {
            sync_live(app);
            let lives = list.into_iter().filter(|w| w.scope.is_none()).map(|w| live_of(app, w)).collect();
            live::detect(app.sig.clone(), lives);
        }
        None if list.is_empty() => log::info!("workspace in use at start: none, this desk has no workspaces"),
        None => {
            log::info!("workspace in use at start: none, this desk has no workspace for all screens");
            sync_live(app);
            live::check_drift();
        }
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

/// The workspace taken as in use at start when none is remembered, or none. Only one for all
/// screens can be (D790).
pub(in crate::app) fn set_default(app: &mut App, id: Option<String>) -> Reply {
    let found = id.as_ref().map(|id| desk_workspaces(app).into_iter().find(|w| w.id == *id));
    match found {
        Some(None) => return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")),
        Some(Some(ws)) if ws.scope.is_some() => {
            return refused("Only a workspace for all screens can be the default");
        }
        _ => {}
    }
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return refused("This desk has no workspaces") };
    desk.default_workspace = id.clone();
    persist(app);
    publish(app);
    Ok(json!({ "id": id }))
}

/// Snap back: switch to the workspace in use on the screen meant again, so what drifted goes back.
pub(in crate::app) fn snap_back(app: &mut App, on: OnScreen, alongside: bool, via: Via) -> Reply {
    let screen = screen_meant(app, on)?;
    match meant(app, screen.as_deref()) {
        Some(id) => start_switch(app, Some(&id), None, alongside, via.snapping()),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace is in use on this desk")),
    }
}

/// What Switch back goes to on `screen`: what was in use there before (D785). With no screen
/// named, on the screen of the one-screen workspace meant, else the desk's previous.
pub(in crate::app) fn switch_back_id(app: &App, screen: Option<&str>) -> Option<String> {
    let desk = app.cfg.desks.get(&app.sig)?;
    let list = desk_workspaces(app);
    let own_screen = || meant(app, None).and_then(|id| list.iter().find(|w| w.id == id).and_then(|w| w.scope.clone()));
    let screen = screen.map(str::to_string).or_else(own_screen);
    let previous = match screen.as_deref() {
        Some(screen) => workspace::switch_back_target(desk, screen),
        None => desk.previous_workspace.as_deref(),
    };
    previous.filter(|id| list.iter().any(|w| w.id == *id)).map(str::to_string)
}

/// Switch back: switch to the workspace that was in use before on the screen meant. Switching
/// back twice goes to and fro between the two. A one-screen target switches its screen, an
/// All-screens one the desk.
pub(in crate::app) fn switch_back(app: &mut App, on: OnScreen, alongside: bool, via: Via) -> Reply {
    let screen = screen_meant(app, on)?;
    match switch_back_id(app, screen.as_deref()) {
        Some(id) => start_switch(app, Some(&id), None, alongside, via),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace to switch back to")),
    }
}

/// "Put windows back" from the tray: every workspace in use that moved, one switch after
/// another (D786), or with none moved, the one meant, as before.
pub(in crate::app) fn put_back_all(app: &mut App) -> Reply {
    let moved: Vec<String> = in_use_ids(app).into_iter().filter(|id| app.drifted.get(id) == Some(&true)).collect();
    let queue = if moved.is_empty() { meant(app, None).into_iter().collect() } else { moved };
    if queue.is_empty() {
        return Err(RpcError::new(api::NOT_FOUND, "No workspace is in use on this desk"));
    }
    app.put_back_queue = queue.into_iter().map(|id| (id, false)).collect();
    next_put_back(app).unwrap_or_else(|| Ok(json!({ "ok": true })))
}

/// Start the next Snap back in the queue, if there is one: never two at once, since the next
/// waits for the end of the one before. One that can't start is skipped. `None` with nothing left.
pub(in crate::app) fn next_put_back(app: &mut App) -> Option<Reply> {
    let mut last = None;
    while !app.put_back_queue.is_empty() {
        let (id, alongside) = app.put_back_queue.remove(0);
        // Starting a switch drops the queue, so it is kept aside and given back.
        let rest = std::mem::take(&mut app.put_back_queue);
        let reply = start_switch(app, Some(&id), None, alongside, Via::local().snapping());
        app.put_back_queue = rest;
        match reply {
            Ok(ok) => return Some(Ok(ok)),
            Err(e) => {
                log::info!("put windows back: {id}: {}", e.message);
                last = Some(Err(e));
            }
        }
    }
    last
}

/// The seam-wm thread saw a window a workspace placed move or minimize.
pub(in crate::app) fn ws_moved(app: &mut App, key: u32) {
    if !app.moved.contains(&key) {
        app.moved.push(key);
    }
    unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
}

/// Things have been still for a moment: tell the panel which workspaces' windows
/// moved, and have the worker look whether the ones in use drifted.
pub(in crate::app) fn drift_due(app: &mut App) {
    // Moves wait while paused. Resuming looks again.
    if standby::paused() {
        return;
    }
    let moved = std::mem::take(&mut app.moved);
    for ws in desk_workspaces(app).into_iter().filter(|w| moved.contains(&workspace::stamp_key(&w.id))) {
        api_server::broadcast("workspace.moved", &json!({ "id": ws.id }));
    }
    let active = in_use_ids(app).iter().any(|id| moved.contains(&workspace::stamp_key(id)));
    if std::mem::take(&mut app.recheck) || active {
        live::check_drift();
    }
}

/// The worker looked at the workspaces in use.
pub(in crate::app) fn drift_checked(app: &mut App) {
    let drifts = live::take_drift();
    if drifts.is_empty() {
        return;
    }
    let in_use = in_use_ids(app);
    let mut changed = false;
    for drift in &drifts {
        if drift.placed {
            api_server::broadcast("workspace.moved", &json!({ "id": drift.id }));
            windows_changed(app);
        }
        let was = app.drifted.get(&drift.id).copied().unwrap_or(false);
        if in_use.contains(&drift.id) && drift.drifted != was {
            log::info!("workspace {}: {}", drift.id, if drift.drifted { "drifted" } else { "in place" });
            app.drifted.insert(drift.id.clone(), drift.drifted);
            changed = true;
        }
    }
    if changed {
        publish(app);
    }
    let moved: Vec<String> =
        drifts.iter().filter(|d| d.drifted && in_use.contains(&d.id)).map(|d| d.id.clone()).collect();
    restore_checked(app, &moved);
}
