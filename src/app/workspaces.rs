//! Workspaces on the connected desk: switching, capturing, quick save and the live checks, moved out of `app.rs`.

use super::*;

/// The workspaces saved for the desk that is connected.
pub(super) fn desk_workspaces(app: &App) -> Vec<Workspace> {
    app.cfg.desks.get(&app.sig).map(|d| workspace::list(&d.workspaces)).unwrap_or_default()
}

/// Next and previous zone give up a key the directional moves hold now (D107),
/// in the file too, so the panel shows them as not set.
pub(super) fn migrate_hotkeys(app: &mut App) {
    let dropped = app.cfg.hotkeys.drop_zone_clashes();
    for action in &dropped {
        log::info!("hotkey {action} cleared: a directional key holds it now");
    }
    if !dropped.is_empty() {
        persist(app);
    }
}

/// Register the shortcuts that changed. Windows refuses one another app holds.
pub(super) fn apply_hotkeys(app: &mut App) {
    let keys = &app.cfg.hotkeys;
    let mut wanted = vec![
        (HOTKEY_LINE_UP, "line_up".to_string(), keys.line_up.clone()),
        (HOTKEY_PAUSE, "pause".to_string(), keys.pause.clone()),
        (HOTKEY_NEXT_ZONE, "next_zone".to_string(), keys.next_zone.clone()),
        (HOTKEY_PREV_ZONE, "prev_zone".to_string(), keys.prev_zone.clone()),
        (HOTKEY_CAPTURE, "capture_workspace".to_string(), keys.capture_workspace.clone()),
        (HOTKEY_PIN_TOP, "pin_top".to_string(), keys.pin_top.clone()),
        (HOTKEY_PIN_STICKY, "pin_sticky".to_string(), keys.pin_sticky.clone()),
        (HOTKEY_RELEASE, "release_all".to_string(), keys.release_all.clone()),
        (HOTKEY_SNAP_BACK, "snap_back".to_string(), keys.snap_back.clone()),
        (HOTKEY_QUICK_SAVE, "quick_save".to_string(), keys.quick_save.clone()),
        (HOTKEY_GROW, "grow".to_string(), keys.grow.clone()),
        (HOTKEY_SHRINK, "shrink".to_string(), keys.shrink.clone()),
        (HOTKEY_PRIORITY, "priority".to_string(), keys.priority.clone()),
        (HOTKEY_SWITCH_BACK, "switch_back".to_string(), keys.switch_back.clone()),
        (HOTKEY_TILE_SCREEN, "tile_screen".to_string(), keys.tile_screen.clone()),
        (HOTKEY_FOCUS_ONLY, "focus_only".to_string(), keys.focus_only.clone()),
        (HOTKEY_SPOTLIGHT, "spotlight".to_string(), keys.spotlight.clone()),
    ];
    for (id, (verb, dir, combo)) in (HOTKEY_WINDOW..).zip(keys.directional()) {
        wanted.push((id, format!("{verb}_{dir}"), combo.cloned()));
    }
    for (id, ws) in (HOTKEY_WORKSPACE..).zip(desk_workspaces(app)) {
        wanted.push((id, format!("workspace:{}", ws.id), ws.hotkey));
    }
    if wanted == app.bound {
        return;
    }
    app.hotkey_errors.clear();
    for (id, _, _) in &app.bound {
        unsafe {
            let _ = UnregisterHotKey(Some(app.hwnd), *id);
        }
    }
    for (id, action, combo) in &wanted {
        let Some(combo) = combo else { continue };
        let registered = hotkey::parse(combo)
            .is_some_and(|c| unsafe { RegisterHotKey(Some(app.hwnd), *id, HOT_KEY_MODIFIERS(c.mods), c.vk).is_ok() });
        if !registered {
            log::info!("hotkey {action} not registered: {combo}");
            app.hotkey_errors.push(HotkeyError { action: action.clone(), combo: combo.clone() });
        }
    }
    app.bound = wanted;
}

pub(super) fn job_screens(app: &App) -> Vec<SwitchScreen> {
    let desk = app.cfg.desks.get(&app.sig);
    let screen = |(i, m): (usize, &Mon)| SwitchScreen {
        id: m.id.clone(),
        number: i + 1,
        bounds: zones::Rect { x: m.rect.left, y: m.rect.top, w: m.rect.width(), h: m.rect.height() },
        layout: desk.and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of),
        raw_dpi: m.raw_dpi,
    };
    app.mons.iter().enumerate().map(screen).collect()
}

/// A capture of this desk. The apps the zones leave alone are never captured either.
pub(super) fn capture_job(app: &App) -> CaptureJob {
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    CaptureJob { owner: app.hwnd.0 as isize, screens: job_screens(app), gap: app.cfg.zones.gap, exclude }
}

/// Start switching to a workspace on this desk. The worker does the rest and the
/// result comes back as an event.
pub(super) fn start_switch(app: &mut App, id: Option<&str>, name: Option<&str>, alongside: bool) -> Reply {
    let found = desk_workspaces(app).into_iter().find(|w| match (id, name) {
        (Some(id), _) => w.id == id,
        (None, Some(name)) => w.name.eq_ignore_ascii_case(name),
        (None, None) => false,
    });
    let Some(workspace) = found else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    start_switch_job(app, workspace, None, alongside)
}

/// Hand a workspace, saved or the last session, to the switch worker.
pub(super) fn start_switch_job(
    app: &mut App,
    workspace: Workspace,
    arrange_for: Option<Duration>,
    alongside: bool,
) -> Reply {
    log::info!("switching to workspace {}", workspace.name);
    // A hotkey gives the tray the right to take focus. For anyone else this is all it can do.
    let _ = unsafe { AllowSetForegroundWindow(std::process::id()) };
    let (owner, screens, gap) = (app.hwnd.0 as isize, job_screens(app), app.cfg.zones.gap);
    let (pins_stay, size_mode) = (app.cfg.windows.pins_stay_on_switch, app.cfg.window_size_on_move);
    let job = SwitchJob { owner, workspace, screens, gap, arrange_for, pins_stay, size_mode, alongside };
    if switcher::switch(job) {
        Ok(json!({ "ok": true }))
    } else {
        refused("Couldn't start the switch")
    }
}

/// Why window placement steps aside for another window manager (D110), if it does.
pub(super) fn aside(app: &App) -> Option<String> {
    match (app.other_wm, app.cfg.windows.other_wm) {
        (Some(name), _) => {
            Some(format!("{} is running, so Seam leaves placing windows to it", state::other_wm_title(name)))
        }
        (None, true) => Some("Another window manager handles the windows, so Seam leaves placing them to it".into()),
        (None, false) => None,
    }
}

/// The seam-wm thread found another window manager start or stop.
pub(super) fn other_wm_changed(app: &mut App) {
    app.other_wm = wm::detected();
    log::info!("other window manager: {}", app.other_wm.unwrap_or("none"));
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    sync_live(app);
    publish(app);
}

/// Queue a window command for the focused window. The worker answers.
pub(super) fn start_window(app: &App, verb: WindowVerb, dir: Dir, reply: Option<Sender<Reply>>) {
    // Focus doesn't place anything, so it works next to another window manager.
    if let Some(why) = aside(app).filter(|_| verb != WindowVerb::Focus) {
        log::info!("window {verb:?} {}: {why}", dir.name());
        if let Some(tx) = reply {
            let _ = tx.send(refused(why));
        }
        return;
    }
    queue_window(app, wincontrol::Task::Window(verb, dir), Vec::new(), reply);
}

/// Queue grow, shrink or Priority for the focused window's group of zones.
pub(super) fn start_group(app: &App, verb: GroupVerb, reply: Option<Sender<Reply>>) {
    if let Some(why) = aside(app) {
        log::info!("window {verb:?}: {why}");
        if let Some(tx) = reply {
            let _ = tx.send(refused(why));
        }
        return;
    }
    queue_window(app, wincontrol::Task::Group(verb), snap::zoned(), reply);
}

pub(super) fn queue_window(app: &App, task: wincontrol::Task, members: Vec<snap::Zoned>, reply: Option<Sender<Reply>>) {
    let hwnd = unsafe { GetForegroundWindow() };
    let desk = app.cfg.desks.get(&app.sig).filter(|_| app.cfg.zones.enabled);
    let layout = |m: &Mon| desk.and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of);
    wincontrol::start(wincontrol::Job {
        owner: app.hwnd.0 as isize,
        task,
        hwnd: hwnd.0 as isize,
        snapped: snap::zone_of(hwnd),
        members,
        mons: app.mons.clone(),
        links: app.links.clone(),
        layouts: app.mons.iter().map(layout).collect(),
        gap: app.cfg.zones.gap,
        exclude: app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect(),
        size_mode: app.cfg.window_size_on_move,
        // A hotkey gives the tray the right to take focus. The API has to earn it.
        from_api: reply.is_some(),
        reply,
    });
}

/// Queue "Focus on this window" for the focused window. The worker answers.
pub(super) fn start_focus_only(app: &App, scope: FocusScope, reply: Option<Sender<Reply>>) {
    let refuse = |why: String, reply: Option<Sender<Reply>>| {
        log::info!("focus on this window: {why}");
        if let Some(tx) = reply {
            let _ = tx.send(refused(why));
        }
    };
    if let Some(why) = aside(app) {
        return refuse(why, reply);
    }
    let workspace = app.workspace.as_ref().and_then(|id| desk_workspaces(app).into_iter().find(|w| w.id == *id));
    if scope == FocusScope::Workspace && workspace.is_none() {
        return refuse("No workspace is in use on this desk".into(), reply);
    }
    focus_only::start(focus_only::Job {
        scope,
        hwnd: unsafe { GetForegroundWindow() }.0 as isize,
        screens: app.mons.iter().map(|m| m.rect).collect(),
        exclude: app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect(),
        workspace,
        // A hotkey gives the tray the right to take focus. The API has to earn it.
        from_api: reply.is_some(),
        reply,
    });
}

/// Turn spotlight on or off, or over when `on` is `None`. Returns whether it's on now.
pub(super) fn set_spotlight(app: &mut App, on: Option<bool>) -> bool {
    let now = spotlight::set(on.unwrap_or(!spotlight::is_on()), &app.mons);
    wm::spotlight(now);
    log::info!("spotlight: {}", if now { "on" } else { "off" });
    publish(app);
    now
}

/// Window control's moves are ready. A group command changed a screen's layout,
/// which is saved here once, before the windows move. A layout from Save from
/// windows goes into the library too.
pub(super) fn window_planned(app: &mut App) {
    let tiled = wincontrol::planned(|screen, layout, fresh| set_screen_layout(app, screen, Some(layout), fresh));
    tile_finished(app, tiled);
}

/// Save a screen's layout on this desk, or take it away for no zones. A layout
/// that follows a library layout keeps the link only while it still matches, and
/// a `fresh` one from Save from windows goes into the library. False when this
/// desk isn't saved.
pub(super) fn set_screen_layout(app: &mut App, screen: &str, layout: Option<zones::ZoneLayout>, fresh: bool) -> bool {
    if !app.cfg.desks.contains_key(&app.sig) {
        return false;
    }
    let value = match layout {
        Some(mut layout) => {
            seam::library::link(&mut app.cfg.custom_layouts, &mut layout, fresh.then_some(screen));
            let Ok(value) = serde_json::to_value(&layout) else { return false };
            Some(value)
        }
        None => None,
    };
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return false };
    match value {
        Some(value) => desk.zones.insert(screen.to_string(), value),
        None => desk.zones.remove(screen),
    };
    persist(app);
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    publish(app);
    true
}

/// A switch finished: say which workspace is in use, and how it went.
pub(super) fn switched(app: &mut App) {
    let Some(done) = switcher::take_switched() else { return };
    log::info!(
        "workspace {}: {} launched, {} placed, {} skipped",
        done.name,
        done.launched,
        done.placed,
        done.skipped.len()
    );
    // The last session isn't a workspace anyone can pick, so it never shows as the one in use.
    if done.id != SESSION_ID {
        set_in_use(app, Some(done.id.clone()));
        sync_live(app);
        app.recheck = true;
        unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
    }
    // Apps opened by hand during the switch go to their places now.
    live::switch_ended();
    focus_only::forget();
    windows_changed(app);
    publish(app);
    let skipped: Vec<Value> =
        done.skipped.iter().map(|(app, reason)| json!({ "app": app, "reason": reason })).collect();
    let data = json!({
        "id": done.id, "name": done.name, "launched": done.launched, "placed": done.placed, "skipped": skipped
    });
    api_server::broadcast("workspace.switched", &data);
}

/// The workspace in use on this desk, if there is one.
pub(super) fn active_workspace(app: &App) -> Option<Workspace> {
    let id = app.workspace.as_deref()?;
    desk_workspaces(app).into_iter().find(|w| w.id == id)
}

/// A workspace with what a status check or auto-apply needs to place its windows.
pub(super) fn live_of(app: &App, workspace: Workspace) -> LiveWorkspace {
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let (screens, gap, pins_stay) = (job_screens(app), app.cfg.zones.gap, app.cfg.windows.pins_stay_on_switch);
    LiveWorkspace { workspace, screens, gap, pins_stay, exclude, size_mode: app.cfg.window_size_on_move }
}

/// Hand the workspace in use to the live worker, and tell the seam-wm thread what to
/// watch. Auto-apply steps aside for another window manager, as snapping does (D110).
pub(super) fn sync_live(app: &mut App) {
    let active = active_workspace(app);
    if active.is_none() {
        app.drifted = false;
    }
    let auto_apply = app.cfg.auto_apply && aside(app).is_none();
    let live = active.map(|ws| live_of(app, ws));
    live::set_active(app.hwnd.0 as isize, live, auto_apply);
    live::set_rules(app_rules_of(app));
}

/// The app rules this build can use, unless another window manager has the windows.
fn app_rules_of(app: &App) -> Option<AppRules> {
    let rules = workspace::app_rules(&app.cfg.app_rules);
    if rules.is_empty() || aside(app).is_some() {
        return None;
    }
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let (screens, gap, pins_stay) = (job_screens(app), app.cfg.zones.gap, app.cfg.windows.pins_stay_on_switch);
    Some(AppRules { rules, screens, gap, pins_stay, exclude, size_mode: app.cfg.window_size_on_move })
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
pub(super) fn forget_deleted(app: &mut App) -> bool {
    let ids: Vec<String> = desk_workspaces(app).into_iter().map(|w| w.id).collect();
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
pub(super) fn choose_in_use(app: &mut App) {
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
pub(super) fn detected_in_use(app: &mut App) {
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
pub(super) fn set_default(app: &mut App, id: Option<String>) -> Reply {
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
pub(super) fn snap_back(app: &mut App) -> Reply {
    match app.workspace.clone() {
        Some(id) => start_switch(app, Some(&id), None, false),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace is in use on this desk")),
    }
}

/// Switch back: switch to the workspace that was in use before this one. Switching
/// back twice goes to and fro between the two.
pub(super) fn switch_back(app: &mut App) -> Reply {
    let previous = app.cfg.desks.get(&app.sig).and_then(|d| d.previous_workspace.clone());
    match previous.filter(|id| desk_workspaces(app).iter().any(|w| w.id == *id)) {
        Some(id) => start_switch(app, Some(&id), None, false),
        None => Err(RpcError::new(api::NOT_FOUND, "No workspace to switch back to")),
    }
}

/// The seam-wm thread saw a window a workspace placed move or minimize.
pub(super) fn ws_moved(app: &mut App, key: u32) {
    if !app.moved.contains(&key) {
        app.moved.push(key);
    }
    unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
}

/// Things have been still for a moment: tell the panel which workspaces' windows
/// moved, and have the worker look whether the one in use drifted.
pub(super) fn drift_due(app: &mut App) {
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
pub(super) fn drift_checked(app: &mut App) {
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

/// Start capturing the windows on screen. Returns an answer only if it can't start.
pub(super) fn start_capture(
    app: &mut App,
    name: Option<String>,
    into: Option<String>,
    row: Option<usize>,
    reply: Option<Sender<Reply>>,
) -> Option<Reply> {
    if app.capturing.is_some() {
        return Some(busy("Already capturing"));
    }
    let target = into.as_ref().map(|into| desk_workspaces(app).into_iter().find(|w| w.id == *into));
    if matches!(target, Some(None)) {
        return Some(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
    }
    if row.is_some_and(|row| target.flatten().is_none_or(|w| row >= w.windows.len())) {
        return Some(Err(RpcError::new(api::NOT_FOUND, "no such row in that workspace")));
    }
    app.capturing = Some(Capturing { reply, name, into, row, sig: app.sig.clone(), quick: false });
    if switcher::capture(capture_job(app)) {
        return None;
    }
    app.capturing = None;
    Some(refused("Couldn't start the capture"))
}

/// Quick save (§7): the same capture as "Update from screen", into the workspace in
/// use, or with none in use into a new "Workspace N" that becomes the one in use.
/// Returns an answer only if it can't start.
pub(super) fn quick_save(app: &mut App, reply: Option<Sender<Reply>>) -> Option<Reply> {
    let into = active_workspace(app).map(|w| w.id);
    let refusal = start_capture(app, None, into, None, reply);
    if let Some(job) = app.capturing.as_mut().filter(|_| refusal.is_none()) {
        job.quick = true;
    }
    refusal
}

/// A quick save went through: say so, and keep what Undo puts back. Undo is a
/// click on the notification, because a tray balloon can't carry a button.
pub(super) fn quick_saved(app: &mut App, saved: &Workspace, before: Option<Value>) {
    let created = before.is_none();
    let in_use = app.workspace.clone();
    if created {
        set_in_use(app, Some(saved.id.clone()));
    }
    sync_live(app);
    let until = Instant::now() + QUICK_UNDO_FOR;
    app.quick_undo = Some(QuickUndo { sig: app.sig.clone(), id: saved.id.clone(), before, in_use, until });
    let n = saved.windows.len();
    let body = if created {
        format!("{n} windows saved in a new workspace, now in use. Click to undo")
    } else {
        format!("{n} windows saved. Click to undo")
    };
    log::info!("quick save: {n} windows into {}{}", saved.name, if created { ", created" } else { "" });
    notify(app, &format!("Saved {}", saved.name), &body, BalloonAction::UndoQuickSave);
}

/// The quick save's notification was clicked: put the workspace back as it was, or
/// remove the one it created, while that's still within 8 s.
pub(super) fn undo_quick_save(app: &mut App) {
    let Some(undo) = app.quick_undo.take() else { return };
    if Instant::now() > undo.until {
        log::info!("quick save: too late to undo");
        return;
    }
    let Some(desk) = app.cfg.desks.get_mut(&undo.sig) else { return };
    let same = |v: &Value| workspace::parse(v).is_some_and(|w| w.id == undo.id);
    match undo.before {
        Some(before) => {
            if let Some(slot) = desk.workspaces.iter_mut().find(|v| same(v)) {
                *slot = before;
            }
        }
        None => {
            desk.workspaces.retain(|v| !same(v));
            if app.workspace.as_deref() == Some(undo.id.as_str()) {
                set_in_use(app, undo.in_use);
            }
        }
    }
    log::info!("quick save: undone");
    persist(app);
    sync_live(app);
    publish(app);
}

/// A capture finished: save it as a new workspace, or over the one it was asked to update.
pub(super) fn captured(app: &mut App) {
    let (Some((rules, hwnds)), Some(job)) = (switcher::take_captured(), app.capturing.take()) else { return };
    let answer = |r: Reply| {
        if let Some(tx) = &job.reply {
            let _ = tx.send(r);
        }
    };
    let Some(desk) = app.cfg.desks.get_mut(&job.sig) else { return answer(refused("That desk is gone")) };
    let existing = workspace::list(&desk.workspaces);
    // The workspace as it was, for a quick save's Undo.
    let mut before = None;
    // The row each captured window has now, when the save stamps them.
    let mut rows = None;
    // "Where it is now" stamps its one window.
    let mut stamps = None;
    let saved = match &job.into {
        Some(into) => {
            let Some(old) = existing.iter().find(|w| w.id == *into) else {
                return answer(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
            };
            let updated = match job.row {
                Some(row) => match workspace::recapture_row(old, row, &rules) {
                    Some((updated, hit)) => {
                        stamps = Some(vec![(hwnds[hit], row)]);
                        updated
                    }
                    None => return answer(refused("That app isn't open, so where it goes stays as it was")),
                },
                None => {
                    rows = Some(workspace::capture_rows(old, &rules));
                    Workspace { windows: workspace::merge_capture(old, rules), ..old.clone() }
                }
            };
            let same = |v: &&mut Value| workspace::parse(v).is_some_and(|w| w.id == *into);
            if let Some(slot) = desk.workspaces.iter_mut().find(same) {
                before = Some(std::mem::replace(slot, workspace::to_value(&updated)));
            }
            updated
        }
        None => {
            let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
            let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
            let name = match job.name.as_deref().map(str::trim) {
                Some(name) if !name.is_empty() => workspace::unique_name(name, &names),
                _ => workspace::next_name(&names),
            };
            if job.quick {
                rows = Some((0..rules.len()).collect());
            }
            let created = Workspace { id: workspace::new_id(&ids), name, windows: rules, ..Workspace::default() };
            desk.workspaces.push(workspace::to_value(&created));
            created
        }
    };
    persist(app);
    // The saved windows hold their rows, as after a switch, so drift and auto-apply see them at once.
    let key = workspace::stamp_key(&saved.id);
    if let Some(rows) = rows {
        live::stamp(key, hwnds.into_iter().zip(rows).collect(), true);
    } else if let Some(windows) = stamps {
        // The drift check comes below, once the worker has the updated workspace.
        live::stamp(key, windows, false);
    }
    if job.quick {
        let created = before.is_none();
        quick_saved(app, &saved, before);
        live::check_drift();
        publish(app);
        return answer(Ok(json!({ "id": saved.id, "created": created })));
    }
    if app.workspace.as_deref() == Some(saved.id.as_str()) {
        sync_live(app);
        live::check_drift();
    }
    publish(app);
    if job.reply.is_some() {
        return answer(Ok(json!({ "id": saved.id })));
    }
    let body = format!("{} windows saved as {}", saved.windows.len(), saved.name);
    notify(app, "Workspace captured", &body, BalloonAction::OpenPanel("workspaces"));
}

/// The editor saved a workspace of this desk. It's checked as `checked_update`
/// says, then saved and applied through the same path as `config.patch`.
pub(super) fn update_workspace(app: &mut App, id: &str, sent: Value) -> Reply {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return refused("This desk has no workspaces") };
    let Some(old) = workspace::list(&desk.workspaces).into_iter().find(|w| w.id == id) else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    let sent: Workspace = match serde_json::from_value(sent) {
        Ok(ws) => ws,
        Err(e) => return refused(format!("That isn't a workspace: {e}")),
    };
    let zones = |monitor: &str| {
        let layout = desk.zones.get(monitor).and_then(zones::layout_of);
        layout.map_or(0, |l| zones::leaf_count(&l.tree))
    };
    let checked = match workspace::checked_update(&old, sent, &zones) {
        Ok(ws) => ws,
        Err(why) => return refused(why),
    };
    let others: Vec<String> =
        workspace::list(&desk.workspaces).into_iter().filter(|w| w.id != id).map(|w| w.name).collect();
    let checked = Workspace { name: workspace::unique_name(&checked.name, &others), ..checked };
    let list: Vec<Value> = desk
        .workspaces
        .iter()
        .map(
            |v| if workspace::parse(v).is_some_and(|w| w.id == id) { workspace::to_value(&checked) } else { v.clone() },
        )
        .collect();
    let patch = json!({ "desks": { app.sig.clone(): { "workspaces": list } } });
    let moves = workspace::moved_rows(&old.windows, &checked.windows);
    let reply = patch_config(app, &patch);
    // Windows a switch or auto-apply stamped keep the app they were stamped for. The
    // worker already has the new rows by now, so a window it stamps in between is
    // remapped from the wrong row. That needs a window to open in those microseconds,
    // and the next switch stamps everything again.
    let unmoved = moves.len() == checked.windows.len() && moves.iter().enumerate().all(|(i, m)| *m == Some(i));
    if reply.is_ok() && !unmoved {
        live::restamp(workspace::stamp_key(id), moves);
    }
    reply
}
