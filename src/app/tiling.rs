//! Smart tiling from the tray's side: its settings per screen and virtual desktop,
//! Free all windows from the tray, a key or the API, the float key, and the state.
//! The snapping carries it out (`snap::tiled`).

use super::*;
use seam::state::TilingRef;
use seam::tiling::Family;

/// A `tiling.set` or Free all waiting for the worker to say which desktop is in front.
pub(super) struct TilingWait {
    screen: String,
    on: Option<bool>,
    family: Option<Family>,
    reply: Option<Sender<Reply>>,
}

/// The screen a key, the tray or the API means: its Line Up number, or the one under the pointer.
pub(super) fn screen_of(app: &App, number: Option<u32>) -> Option<String> {
    let at = match number {
        Some(n) => n.checked_sub(1)? as usize,
        None => {
            let At::Point(x, y) = tile::pointer() else { return None };
            app.mons.iter().position(|m| m.rect.contains(x, y)).unwrap_or(0)
        }
    };
    app.mons.get(at).map(|m| m.id.clone())
}

fn answer(reply: Option<Sender<Reply>>, result: Reply) {
    if let Some(tx) = reply {
        let _ = tx.send(result);
    }
}

/// `tiling.set` and the Zones page: tiling on or off on a screen for the desktop in
/// front, or its family. While tiling is off everywhere nothing follows the desktops,
/// so the worker is asked which one is in front first.
pub(super) fn set_tiling(
    app: &mut App,
    number: Option<u32>,
    on: Option<bool>,
    family: Option<Family>,
    reply: Option<Sender<Reply>>,
) {
    let Some(screen) = screen_of(app, number) else {
        return answer(reply, Err(RpcError::new(api::NOT_FOUND, "no such screen")));
    };
    if on == Some(true) && !app.cfg.zones.enabled {
        return answer(reply, refused("Turn zones on first: smart tiling arranges windows in zones"));
    }
    let tracked = app.cfg.desks.get(&app.sig).is_some_and(|d| d.tiling.values().any(|t| !t.desktops.is_empty()));
    let wait = TilingWait { screen, on, family, reply };
    match snap::tiling_desktop().filter(|_| tracked) {
        Some(desktop) => apply(app, wait, &desktop),
        None if crate::tiler::look() => app.tiling_wait.push(wait),
        None => answer(wait.reply, refused("Seam couldn't start smart tiling")),
    }
}

/// The worker's events reach the snapping, and settings that waited to know which
/// desktop is in front are made. False when the snapping was busy.
pub(super) fn tiling_events(app: &mut App) -> bool {
    if !snap::tiling_events() {
        return false;
    }
    // The desktop in front may have changed: Win+wheel follows it.
    mousewatch::set_wheel(&wheel_screens(app));
    if let Some(why) = snap::take_tiling_refused() {
        notify(app, "Float or tile this window", &format!("That window can't be tiled: {why}"), BalloonAction::None);
    }
    if !app.tiling_wait.is_empty() {
        let looked = crate::tiler::take_looked();
        match snap::tiling_desktop() {
            Some(desktop) => {
                for wait in std::mem::take(&mut app.tiling_wait) {
                    apply(app, wait, &desktop);
                }
            }
            // The worker looked and couldn't tell, as on a desktop with no windows yet.
            None if looked => {
                for wait in std::mem::take(&mut app.tiling_wait) {
                    answer(wait.reply, refused(NO_DESKTOP));
                }
            }
            None => {}
        }
    }
    publish(app);
    true
}

/// Why a setting can't be made while the desktop in front isn't known.
const NO_DESKTOP: &str = "Seam can't tell which virtual desktop is in front. Click a window on it and try again";

fn apply(app: &mut App, wait: TilingWait, desktop: &str) {
    // An empty id is only for a Windows with no desktops; it is never stored otherwise.
    if desktop.is_empty() && crate::tiler::has_desktops() {
        return answer(wait.reply, refused(NO_DESKTOP));
    }
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else {
        return answer(wait.reply, refused("This desk isn't saved yet"));
    };
    let entry = desk.tiling.entry(wait.screen.clone()).or_default();
    if let Some(family) = wait.family {
        entry.family = family;
    }
    match wait.on {
        Some(true) if !entry.on(desktop) => entry.desktops.push(desktop.to_string()),
        Some(false) => entry.desktops.retain(|d| d != desktop),
        _ => {}
    }
    let (on, family) = (entry.on(desktop), entry.family);
    if *entry == seam::config::TileScreen::default() {
        desk.tiling.remove(&wait.screen);
    }
    log::info!("tiling: {} on a screen, {}", if on { "on" } else { "off" }, family.title());
    persist(app);
    snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
    // Win+wheel's hook is in while any screen scrolls.
    pointer_watch(app);
    publish(app);
    answer(wait.reply, Ok(json!({ "on": on, "family": family })));
}

/// Free all windows: every window tiling took on the screen goes back to where it was
/// before, and tiling turns off there for the desktop in front.
pub(super) fn free_tiling(app: &mut App, number: Option<u32>, reply: Option<Sender<Reply>>) {
    let Some(screen) = screen_of(app, number) else {
        return answer(reply, Err(RpcError::new(api::NOT_FOUND, "no such screen")));
    };
    let n = app.mons.iter().position(|m| m.id == screen).map_or(1, |i| i + 1);
    if !tiled_screens(app).contains(&n) {
        let why = format!("Smart tiling is off on screen {n}");
        if reply.is_none() {
            notify(app, "Free all windows", &why, BalloonAction::None);
        }
        return answer(reply, refused(why));
    }
    let freed = snap::tiling_free(&screen);
    if reply.is_none() {
        let windows = if freed == 1 { "window" } else { "windows" };
        notify(app, "Free all windows", &format!("{freed} {windows} on screen {n} went back"), BalloonAction::None);
    }
    let (tx, rx) = channel();
    set_tiling(app, Some(n as u32), Some(false), None, Some(tx));
    // The setting answers at once when the desktop is known; otherwise it follows.
    let result = match rx.try_recv() {
        Ok(Err(e)) => Err(e),
        _ => Ok(json!({ "ok": true, "freed": freed })),
    };
    answer(reply, result);
}

/// Scroll left or right on a Scrolling screen, from the keys or `tiling.scroll`: the
/// column next to the focused one takes the focus, and the strip scrolls to show it. The
/// focused window's strip, or else the screen by number or under the pointer.
pub(super) fn scroll(app: &mut App, number: Option<u32>, left: bool, reply: Option<Sender<Reply>>) {
    let Some(screen) = screen_of(app, number) else {
        return answer(reply, Err(RpcError::new(api::NOT_FOUND, "no such screen")));
    };
    // A screen asked for by number is that screen, whatever has the focus.
    let focused = if number.is_some() { 0 } else { unsafe { GetForegroundWindow() }.0 as isize };
    match snap::tiling_scroll(focused, Some(&screen), left, true) {
        Ok(focus) => {
            if let Some(h) = focus {
                wincontrol::focus(HWND(h as *mut _), reply.is_some());
            }
            answer(reply, Ok(json!({ "ok": true })));
        }
        Err(why) => {
            log::info!("scroll: {why}");
            answer(reply, refused(why));
        }
    }
}

/// Win+wheel over a Scrolling screen: the strip under the pointer scrolls a column,
/// and the focus stays where it is. `lag` is how long since the hook heard the wheel.
pub(super) fn wheel(app: &mut App, left: bool, (x, y): (i32, i32), lag: u32) {
    let Some(screen) = app.mons.iter().find(|m| m.rect.contains(x, y)).map(|m| m.id.clone()) else { return };
    match snap::tiling_scroll(0, Some(&screen), left, false) {
        Ok(_) => log::info!("scroll wheel: scrolled {lag} ms after the hook heard it"),
        Err(why) => log::info!("scroll wheel: {why}"),
    }
}

/// Move column left or right (`Some`), or Column width (`None`), for the focused window.
pub(super) fn column(left: Option<bool>) {
    if let Err(why) = snap::tiling_column(unsafe { GetForegroundWindow() }.0 as isize, left) {
        log::info!("column key: {why}");
    }
}

/// The screens a Win+wheel scrolls: those with Scrolling on for the virtual desktop in
/// front, while zones are on and Seam isn't paused or stepping aside. None while the
/// desktop in front isn't known, so a wheel is never swallowed for nothing.
pub(super) fn wheel_screens(app: &App) -> Vec<seam::geom::Rect> {
    if !app.cfg.zones.enabled || standby::reason().is_some() || aside(app).is_some() {
        return Vec::new();
    }
    let (Some(desk), Some(desktop)) = (app.cfg.desks.get(&app.sig), snap::tiling_desktop()) else { return Vec::new() };
    let scrolls = |id: &str| desk.tiling.get(id).is_some_and(|t| t.family == Family::Scrolling && t.on(&desktop));
    app.mons.iter().filter(|m| scrolls(&m.id)).map(|m| m.rect).collect()
}

/// "Float or tile this window", for the focused window.
pub(super) fn float_or_tile() {
    match snap::tiling_float(unsafe { GetForegroundWindow() }) {
        Ok(floats) => log::info!("float key: the window {}", if floats { "floats" } else { "is tiled again" }),
        Err(why) => log::info!("float key: {why}"),
    }
}

/// Each screen's tiling for the state, for the desktop in front.
pub(super) fn tiling_state(app: &App) -> Vec<TilingRef> {
    let desktop = snap::tiling_desktop();
    let counts = snap::tiled();
    let tiling = app.cfg.desks.get(&app.sig).map(|d| &d.tiling);
    let screen = |(i, m): (usize, &Mon)| {
        let entry = tiling.and_then(|t| t.get(&m.id));
        TilingRef {
            screen: i as u32 + 1,
            on: entry.zip(desktop.as_deref()).is_some_and(|(e, d)| e.on(d)),
            family: entry.map(|e| e.family).unwrap_or_default(),
            windows: counts.iter().find(|c| c.screen == m.id).map_or(0, |c| c.windows),
        }
    };
    app.mons.iter().enumerate().map(screen).collect()
}

/// The screens tiled on the desktop in front, by Line Up number, for the tray menu.
pub(super) fn tiled_screens(app: &App) -> Vec<usize> {
    tiling_state(app).into_iter().filter(|t| t.on).map(|t| t.screen as usize).collect()
}
