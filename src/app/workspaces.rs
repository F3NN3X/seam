//! Workspaces on the connected desk: switching, capturing, quick save and the live checks, moved out of `app.rs`.

use super::*;

mod add;
mod assist;
mod capture;
mod close;
mod copy;
mod in_use;
mod join;
mod moved;
mod notice;
mod order;
mod review;
mod step;
mod window;

pub(super) use add::{Which, refuse_move, start_add_window, start_move_window};
pub(super) use assist::{create_workspace, edit_workspace};
pub(super) use capture::{captured, quick_save, start_capture, undo_quick_save, update_from_panel, update_workspace};
pub(super) use close::{Ask, Closing, Minimized, closed, start_close, start_close_here, undo_minimize};
pub(super) use copy::copy_from_desk;
pub(super) use in_use::{
    OnScreen, choose_in_use, detected_in_use, drift_checked, drift_due, forget_deleted, in_use_ids, live_of, meant,
    next_put_back, put_back_all, screen_meant, set_default, snap_back, switch_back, switch_back_id, sync_live,
    ws_moved,
};
use in_use::{active_workspace, holders, put_in_use_back, rescoped, screen_ids, set_in_use, took_over};
pub(super) use join::sent as sent_to_join;
pub(super) use notice::{
    LastSwitch, ended, last_switch, open_switched, show_label, skipped_json, started, starting, starting_due,
};
pub(super) use order::move_workspace;
pub(super) use review::{RowKey, save_preview, start_preview};
pub(super) use step::{step, step_add};
pub(super) use window::{
    aside, other_wm_changed, queue_window, set_spotlight, show_spotlight, start_focus_only, start_group, start_window,
    window_planned,
};

/// How a switch was asked for, beyond which workspace. It has no default: each route chooses
/// who is asking, so none can leave out that another app is (D737).
#[derive(Clone, Copy)]
pub(super) struct Via {
    /// Snap back: windows already open are moved whatever the workspace says.
    pub put_back: bool,
    /// The Seam window asked, and shows the result itself.
    pub from_panel: bool,
    /// An API client other than the Seam window asked, so no startup command runs (D737).
    pub other_app: bool,
}

impl Via {
    /// The tray's own menu, a shortcut, sign-in or a restore: Seam itself asks.
    pub fn local() -> Via {
        Via { put_back: false, from_panel: false, other_app: false }
    }

    /// An API request: the Seam window, or else another app.
    pub fn api(from_panel: bool) -> Via {
        Via { put_back: false, from_panel, other_app: !from_panel }
    }

    /// The same, as Snap back.
    pub fn snapping(self) -> Via {
        Via { put_back: true, ..self }
    }
}

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
        (HOTKEY_CLOSE_WORKSPACE, "close_workspace".to_string(), keys.close_workspace.clone()),
        (HOTKEY_MINIMIZE_WORKSPACE, "minimize_workspace".to_string(), keys.minimize_workspace.clone()),
        (HOTKEY_TILE_SCREEN, "tile_screen".to_string(), keys.tile_screen.clone()),
        (HOTKEY_QUICK_LAYOUT, "quick_layout".to_string(), keys.quick_layout.clone()),
        (HOTKEY_FOCUS_ONLY, "focus_only".to_string(), keys.focus_only.clone()),
        (HOTKEY_SPOTLIGHT, "spotlight".to_string(), keys.spotlight.clone()),
        (HOTKEY_DIVIDER_MENU, "divider_menu".to_string(), keys.divider_menu.clone()),
        (HOTKEY_PAUSE_SEAM, "pause_seam".to_string(), keys.pause_seam.clone()),
        (HOTKEY_FIT_GROUP, "fit_group".to_string(), keys.fit_group.clone()),
        (HOTKEY_STACK_NEXT, "stack_next".to_string(), keys.stack_next.clone()),
        (HOTKEY_STACK_PREV, "stack_prev".to_string(), keys.stack_prev.clone()),
        (HOTKEY_FLOAT_TILE, "float_tile".to_string(), keys.float_tile.clone()),
        (HOTKEY_FREE_TILING, "free_tiling".to_string(), keys.free_tiling.clone()),
        (HOTKEY_OVERVIEW, "overview".to_string(), keys.overview.clone()),
        (HOTKEY_ROTATE, "rotate".to_string(), keys.rotate.clone()),
        (HOTKEY_RESET_SIZES, "reset_sizes".to_string(), keys.reset_sizes.clone()),
        (HOTKEY_ROTATE_BACK, "rotate".to_string(), keys.rotate.as_deref().and_then(hotkey::with_shift)),
        (HOTKEY_SCROLL_LEFT, "scroll_left".to_string(), keys.scroll_left.clone()),
        (HOTKEY_SCROLL_RIGHT, "scroll_right".to_string(), keys.scroll_right.clone()),
        (HOTKEY_COLUMN_LEFT, "column_left".to_string(), keys.column_left.clone()),
        (HOTKEY_COLUMN_RIGHT, "column_right".to_string(), keys.column_right.clone()),
        (HOTKEY_COLUMN_WIDTH, "column_width".to_string(), keys.column_width.clone()),
    ];
    wanted.extend([
        (HOTKEY_WORKSPACE_NEXT, "workspace_next".to_string(), keys.workspace_next.clone()),
        (HOTKEY_WORKSPACE_PREV, "workspace_prev".to_string(), keys.workspace_prev.clone()),
        (HOTKEY_ADD_TO_NEXT, "add_to_next".to_string(), keys.add_to_next.clone()),
        (HOTKEY_ADD_TO_PREV, "add_to_prev".to_string(), keys.add_to_prev.clone()),
    ]);
    for (n, (id, combo)) in (HOTKEY_ADD_TO..).zip(keys.add_to_workspace()).enumerate() {
        wanted.push((id, format!("add_to_{}", n + 1), combo.cloned()));
    }
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
    let (mut screens, gap, places) = (job_screens(app), app.cfg.zones.gap, app.cfg.workspaces_remember_places);
    // A tiled screen's tree changes with every open and close, so a zone number from it
    // would point elsewhere later: its windows are saved by their place (D568).
    for s in screens.iter_mut().filter(|s| snap::is_tiled(&s.id)) {
        s.layout = None;
    }
    let removes = app.cfg.closed_apps() != ClosedApps::Keep;
    CaptureJob { owner: app.hwnd.0 as isize, screens, gap, exclude, places, removes }
}

/// Start switching to a workspace on this desk. The worker does the rest and the
/// result comes back as an event.
pub(super) fn start_switch(app: &mut App, id: Option<&str>, name: Option<&str>, alongside: bool, via: Via) -> Reply {
    let found = desk_workspaces(app).into_iter().find(|w| match (id, name) {
        (Some(id), _) => w.id == id,
        (None, Some(name)) => w.name.eq_ignore_ascii_case(name),
        (None, None) => false,
    });
    let Some(workspace) = found else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    start_switch_job(app, workspace, None, alongside, via)
}

/// Hand a workspace, saved or the last session, to the switch worker.
pub(super) fn start_switch_job(
    app: &mut App,
    workspace: Workspace,
    arrange_for: Option<Duration>,
    alongside: bool,
    via: Via,
) -> Reply {
    if app.closing.is_some() {
        return busy("Windows of a workspace are still being closed or minimized");
    }
    // A one-screen workspace never falls back to all screens (D787).
    if !workspace::scope_connected(&workspace, &screen_ids(app)) {
        log::info!("workspace {}: its screen isn't connected, so it isn't switched", workspace.name);
        return refused(workspace::SCOPE_GONE);
    }
    // What a minimize took belongs to the layout this switch changes. A step and a queue of
    // Snap backs set their own after.
    app.minimized = None;
    app.step_to = None;
    app.put_back_queue.clear();
    log::info!("switching to workspace {}", workspace.name);
    // A hotkey gives the tray the right to take focus. For anyone else this is all it can do.
    let _ = unsafe { AllowSetForegroundWindow(std::process::id()) };
    // A one-screen switch takes its screen only (D783). Snap back of the desk-wide workspace
    // leaves the screens that have their own; any other All-screens switch takes them all (D784).
    let desk_wide_back = via.put_back && workspace.scope.is_none() && app.workspace.as_deref() == Some(&workspace.id);
    let away = match app.cfg.desks.get(&app.sig) {
        Some(desk) if desk_wide_back => workspace::own_screens(desk, &screen_ids(app)),
        _ => Vec::new(),
    };
    let scope = workspace.scope.clone();
    let reach = |s: &str| scope.as_deref().is_none_or(|own| own == s) && !away.iter().any(|a| a == s);
    let held = switcher::held_windows(&holders(app, &workspace.id, &reach));
    // The last session has no layouts, so a restore leaves them alone.
    apply_zones(app, &workspace, &reach);
    let name = workspace.name.clone();
    let screens: Vec<SwitchScreen> = job_screens(app).into_iter().filter(|s| reach(&s.id)).collect();
    let number = scope.as_deref().and_then(|s| screens.iter().find(|m| m.id == s)).map(|m| m.number);
    let (owner, gap) = (app.hwnd.0 as isize, app.cfg.zones.gap);
    let (pins_stay, size_mode) = (app.cfg.windows.pins_stay_on_switch, app.cfg.window_size_on_move);
    // The last session never saves a folder or page, so it never opens one either.
    let places = app.cfg.workspaces_remember_places && workspace.id != SESSION_ID;
    let Via { put_back, from_panel, other_app } = via;
    let job = SwitchJob {
        owner,
        workspace,
        screens,
        gap,
        arrange_for,
        pins_stay,
        size_mode,
        alongside,
        places,
        put_back,
        from_panel,
        other_app,
        away,
        held,
    };
    if switcher::switch(job) {
        Ok(match number {
            Some(screen) => json!({ "ok": true, "screen": screen }),
            None => json!({ "ok": true }),
        })
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
/// the workspace's apps. A screen kept as is, not connected, outside the switch's `reach`, or
/// whose zones already match (`zones_match`, as the editor's status) stays. Saved with zones
/// off or another window manager in charge too.
fn apply_zones(app: &mut App, workspace: &Workspace, reach: &dyn Fn(&str) -> bool) {
    let mut changed = Vec::new();
    for m in app.mons.iter().filter(|m| reach(&m.id)) {
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
    let Some(mut done) = switcher::take_switched() else { return };
    log::info!(
        "workspace {}: {} launched, {} placed, {} skipped",
        done.name,
        done.launched,
        done.placed,
        done.skipped.len()
    );
    // The last session isn't a workspace anyone can pick, so it never shows as the one in use.
    if done.id != SESSION_ID {
        // Snap back of the desk-wide workspace left the screens that have their own: nothing changes hands.
        if !done.left_screens {
            took_over(app, &done.id, done.scope.as_deref());
        }
        app.last_in_use = Some(done.id.clone());
        // Only the end of the switch a step asked for ends its chain: the end of one it replaced may arrive late.
        if app.step_to.as_deref() == Some(done.id.as_str()) {
            app.step_to = None;
        }
        sync_live(app);
        app.recheck = true;
        unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
    }
    // Apps opened by hand during the switch go to their places now.
    live::switch_ended();
    snap::tiling_switch_ended();
    snap::switch_ended();
    snap::rebuild_stacks(std::mem::take(&mut done.stacks));
    focus_only::forget();
    windows_changed(app);
    publish(app);
    let skipped = skipped_json(&done.skipped);
    let problems = skipped.iter().filter(|s| !s["why"].is_null()).count();
    let mut data = json!({
        "id": done.id, "name": done.name, "launched": done.launched, "placed": done.placed, "skipped": skipped,
        "problems": problems, "cancelled": done.cancelled, "from_panel": done.from_panel
    });
    if let Some(n) = done.scope.as_deref().and_then(|s| app.mons.iter().position(|m| m.id == s)) {
        data["screen"] = json!(n + 1);
    }
    api_server::broadcast("workspace.switched", &data);
    ended(app, &done);
    show_label(app, &done);
    // "Put windows back" goes on with the next workspace that moved, now that this one is done,
    // unless the user stopped this one: that stops the rest too (D714).
    app.put_back_queue = left_to_put_back(std::mem::take(&mut app.put_back_queue), done.cancelled);
    if let Some(Err(e)) = next_put_back(app) {
        log::info!("put windows back: {}", e.message);
    }
}

/// What the put-back queue still holds once a switch ended: nothing, when that one was cancelled.
fn left_to_put_back(queue: Vec<(String, bool)>, cancelled: bool) -> Vec<(String, bool)> {
    if cancelled { Vec::new() } else { queue }
}

#[cfg(test)]
mod tests {
    use super::Via;
    use super::left_to_put_back;
    use super::step::step_target;

    #[test]
    fn a_cancel_stops_the_snap_backs_still_queued() {
        let queue = vec![("b".to_string(), false), ("c".to_string(), false)];
        assert_eq!(left_to_put_back(queue.clone(), false), queue);
        assert!(left_to_put_back(queue, true).is_empty());
    }
    use seam::workspace::{Step, Workspace};

    fn list(ids: &[&str]) -> Vec<Workspace> {
        ids.iter().map(|id| Workspace { id: (*id).into(), ..Workspace::default() }).collect()
    }

    #[test]
    fn quick_presses_chain_from_the_last_target() {
        let l = list(&["a", "b", "c", "d"]);
        // Nothing asked yet: from the one in use.
        assert_eq!(step_target(&l, Some("a"), None, Step::Next), Some(1));
        // A switch to b is still running, so the next press goes on from b, then c.
        assert_eq!(step_target(&l, Some("a"), Some("b"), Step::Next), Some(2));
        assert_eq!(step_target(&l, Some("a"), Some("c"), Step::Next), Some(3));
        // Previous after a Next undoes it, whatever is still in use.
        assert_eq!(step_target(&l, Some("a"), Some("b"), Step::Previous), Some(0));
    }

    #[test]
    fn only_the_seam_window_and_seam_itself_may_run_startup_commands() {
        assert!(!Via::local().other_app && !Via::local().from_panel);
        assert!(!Via::api(true).other_app && Via::api(true).from_panel);
        assert!(Via::api(false).other_app && !Via::api(false).from_panel);
        // Snap back keeps who asked.
        assert!(Via::api(false).snapping().other_app && Via::api(false).snapping().put_back);
        assert!(!Via::api(true).snapping().other_app);
        assert!(!Via::local().put_back);
    }
}
