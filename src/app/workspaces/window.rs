//! Window commands, spotlight and other-window-manager checks.

use super::*;

/// Why window placement steps aside for another window manager (D110), if it does.
pub(in crate::app) fn aside(app: &App) -> Option<String> {
    match (app.other_wm, app.cfg.windows.other_wm) {
        (Some(name), _) => {
            Some(format!("{} is running, so Seam leaves placing windows to it", state::other_wm_title(name)))
        }
        (None, true) => Some("Another window manager handles the windows, so Seam leaves placing them to it".into()),
        (None, false) => None,
    }
}

/// The seam-wm thread found another window manager start or stop.
pub(in crate::app) fn other_wm_changed(app: &mut App) {
    app.other_wm = wm::detected();
    log::info!("other window manager: {}", app.other_wm.unwrap_or("none"));
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    sync_live(app);
    publish(app);
}

/// Queue a window command for the focused window. The worker answers.
pub(in crate::app) fn start_window(app: &App, verb: WindowVerb, dir: Dir, reply: Option<Sender<Reply>>) {
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
pub(in crate::app) fn start_group(app: &App, verb: GroupVerb, reply: Option<Sender<Reply>>) {
    if let Some(why) = aside(app) {
        log::info!("window {verb:?}: {why}");
        if let Some(tx) = reply {
            let _ = tx.send(refused(why));
        }
        return;
    }
    queue_window(app, wincontrol::Task::Group(verb), snap::zoned(), reply);
}

pub(in crate::app) fn queue_window(
    app: &App,
    task: wincontrol::Task,
    members: Vec<snap::Zoned>,
    reply: Option<Sender<Reply>>,
) {
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
pub(in crate::app) fn start_focus_only(app: &App, scope: FocusScope, reply: Option<Sender<Reply>>) {
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
        places: app.cfg.workspaces_remember_places,
        workspace,
        // A hotkey gives the tray the right to take focus. The API has to earn it.
        from_api: reply.is_some(),
        reply,
    });
}

/// Turn spotlight on or off, or over when `on` is `None`. Returns whether it's on now.
pub(in crate::app) fn set_spotlight(app: &mut App, on: Option<bool>) -> bool {
    let wanted = on.unwrap_or(!(spotlight::is_on() || app.spotlight_resume));
    // Standing by, it waits and comes back with the rest.
    if standby::reason().is_some() {
        app.spotlight_resume = wanted;
        return false;
    }
    show_spotlight(app, wanted)
}

/// Put the dim on or off. Returns whether it's on now.
pub(in crate::app) fn show_spotlight(app: &mut App, on: bool) -> bool {
    let now = spotlight::set(on, &app.mons);
    wm::spotlight(now);
    log::info!("spotlight: {}", if now { "on" } else { "off" });
    publish(app);
    now
}

/// Window control's moves are ready. A group command changed a screen's layout,
/// which is saved here once, before the windows move. A layout from Save from
/// windows goes into the library too.
pub(in crate::app) fn window_planned(app: &mut App) {
    let tiled = wincontrol::planned(|screen, layout, fresh| set_screen_layout(app, screen, Some(layout), fresh));
    tile_finished(app, tiled);
    // A split's layout was saved with the edge drags', so its notice comes after a tile's.
    if let Some(split) = wincontrol::take_split() {
        tile_finished(app, Some(Ok(split)));
    }
}
