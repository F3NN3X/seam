//! Workspaces on the connected desk: switching, capturing, quick save and the live checks, moved out of `app.rs`.

use super::*;

mod capture;
mod in_use;
mod window;

pub(super) use capture::{captured, quick_save, start_capture, undo_quick_save, update_workspace};
use in_use::{active_workspace, set_in_use};
pub(super) use in_use::{
    choose_in_use, detected_in_use, drift_checked, drift_due, forget_deleted, live_of, set_default, snap_back,
    switch_back, sync_live, ws_moved,
};
pub(super) use window::{
    aside, other_wm_changed, queue_window, set_spotlight, show_spotlight, start_focus_only, start_group, start_window,
    window_planned,
};

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
        (HOTKEY_QUICK_LAYOUT, "quick_layout".to_string(), keys.quick_layout.clone()),
        (HOTKEY_FOCUS_ONLY, "focus_only".to_string(), keys.focus_only.clone()),
        (HOTKEY_SPOTLIGHT, "spotlight".to_string(), keys.spotlight.clone()),
        (HOTKEY_DIVIDER_MENU, "divider_menu".to_string(), keys.divider_menu.clone()),
        (HOTKEY_PAUSE_SEAM, "pause_seam".to_string(), keys.pause_seam.clone()),
        (HOTKEY_FIT_GROUP, "fit_group".to_string(), keys.fit_group.clone()),
    ];
    for (id, (verb, dir, combo)) in (HOTKEY_WINDOW..).zip(keys.directional()) {
        wanted.push((id, format!("{verb}_{dir}"), combo.cloned()));
    }
    for (id, ws) in (HOTKEY_WORKSPACE..).zip(desk_workspaces(app)) {
        wanted.push((id, format!("workspace:{}", ws.id), ws.hotkey));
    }
    // Standing by, the keys go back to the app in front. A paused Seam keeps the one that resumes it.
    if let Some(why) = standby::reason() {
        wanted.retain(|(id, ..)| *id == HOTKEY_PAUSE_SEAM && why == standby::Standby::Paused);
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
    let (screens, gap, places) = (job_screens(app), app.cfg.zones.gap, app.cfg.workspaces_remember_places);
    let removes = app.cfg.workspaces_update_removes_closed;
    CaptureJob { owner: app.hwnd.0 as isize, screens, gap, exclude, places, removes }
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
    // The last session has no layouts, so a restore leaves them alone.
    apply_zones(app, &workspace);
    let name = workspace.name.clone();
    let (owner, screens, gap) = (app.hwnd.0 as isize, job_screens(app), app.cfg.zones.gap);
    let (pins_stay, size_mode) = (app.cfg.windows.pins_stay_on_switch, app.cfg.window_size_on_move);
    // The last session never saves a folder or page, so it never opens one either.
    let places = app.cfg.workspaces_remember_places && workspace.id != SESSION_ID;
    let job = SwitchJob { owner, workspace, screens, gap, arrange_for, pins_stay, size_mode, alongside, places };
    if switcher::switch(job) {
        Ok(json!({ "ok": true }))
    } else {
        log::info!("workspace {}: the switch didn't start, its screen layouts stay set", name);
        refused("Couldn't start the switch")
    }
}

/// Save a screen's layout on this desk, or take it away for no zones. A layout
/// that follows a library layout keeps the link only while it still matches, and
/// a `fresh` one from Save from windows goes into the library. False when this
/// desk isn't saved.
pub(super) fn set_screen_layout(app: &mut App, screen: &str, layout: Option<zones::ZoneLayout>, fresh: bool) -> bool {
    if !store_screen_layout(app, screen, layout, fresh) {
        return false;
    }
    persist(app);
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    publish(app);
    true
}

/// `set_screen_layout` without saving the file or telling anyone, so several
/// screens can change at once.
fn store_screen_layout(app: &mut App, screen: &str, layout: Option<zones::ZoneLayout>, fresh: bool) -> bool {
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
    true
}

/// Give each connected screen the layout the workspace has for it, before its switch
/// is built, so the switch places apps into the new zones. Saved as a pick on the
/// Zones page is, but the screen's snapped windows aren't moved: the switch places
/// the workspace's apps. A screen kept as is, not connected, or whose zones already
/// match (`zones_match`, as the editor's status) stays. Saved with zones off or
/// another window manager in charge too.
fn apply_zones(app: &mut App, workspace: &Workspace) {
    let mut changed = Vec::new();
    for m in &app.mons {
        let Some(saved) = workspace.zones.get(&m.id) else { continue };
        let before = app.cfg.desks.get(&app.sig).and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of);
        if workspace::zones_match(saved, before.as_ref()) {
            continue;
        }
        let Some(mut layout) = workspace::screen_layout(saved) else { continue };
        if let Some(layout) = layout.as_mut() {
            seam::library::link(&mut app.cfg.custom_layouts, layout, None);
        }
        changed.push((m.id.clone(), layout));
    }
    let mut stored = 0;
    for (screen, layout) in changed {
        stored += usize::from(store_screen_layout(app, &screen, layout, false));
    }
    if stored > 0 {
        log::info!("workspace {}: {stored} screen layouts changed", workspace.name);
        persist(app);
        snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
        publish(app);
    }
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
