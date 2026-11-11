//! Tile this screen's windows on the main thread: starting it from a key, the tray
//! or the API, and the notice that offers Undo. The worker picks the windows and
//! the zones (`wincontrol::tile`). A layout picked on the Zones page moves the
//! screen's snapped windows and offers the same Undo.

use super::workspaces::{queue_window, set_screen_layout};
use super::*;
use wincontrol::tile::At;

/// Queue a tile. The worker answers an API client; a key or the tray hears of a
/// refusal in a notice.
pub(super) fn start_tile(app: &mut App, at: At, reply: Option<Sender<Reply>>) {
    let refusal = aside(app)
        .or_else(|| (!app.cfg.zones.enabled).then(|| "Zones are off".to_string()))
        .or_else(|| crate::switcher::switching().then(|| "A workspace switch is still running".to_string()));
    if let Some(why) = refusal {
        log::info!("tile screen: {why}");
        match reply {
            Some(tx) => {
                let _ = tx.send(refused(why));
            }
            None => {
                notify(app, "Tile windows", &why, BalloonAction::None);
            }
        }
        return;
    }
    queue_window(app, wincontrol::Task::Tile(at), snap::zoned(), reply);
}

/// Queue Save from windows, which only the API asks for. The worker answers.
pub(super) fn start_from_windows(app: &mut App, at: At, reply: Sender<Reply>) {
    let refusal = aside(app)
        .or_else(|| (!app.cfg.zones.enabled).then(|| "Zones are off".to_string()))
        .or_else(|| crate::switcher::switching().then(|| "A workspace switch is still running".to_string()));
    if let Some(why) = refusal {
        log::info!("save from windows: {why}");
        let _ = reply.send(refused(why));
        return;
    }
    queue_window(app, wincontrol::Task::FromWindows(at), Vec::new(), Some(reply));
}

/// The pointer's place, for a key.
pub(super) fn pointer() -> At {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    At::Point(pt.x, pt.y)
}

/// A tile finished: say what it did and keep what Undo puts back, or say why
/// nothing happened. Undo is a click on the notice, as with a quick save.
pub(super) fn tile_finished(app: &mut App, result: Option<Result<wincontrol::tile::Done, String>>) {
    match result {
        None => {}
        Some(Err(why)) => {
            notify(app, "Tile windows", &why, BalloonAction::None);
        }
        Some(Ok(done)) => {
            let n = done.tiled;
            // Picking another tile is its own undo when no window moved, so stay quiet.
            if done.pick && n == 0 {
                log::info!("zones pick screen {}: no window to move", done.number);
                return;
            }
            let windows = if n == 1 { "window" } else { "windows" };
            let (title, body) = if done.pick {
                let title = format!("Screen {} has a new layout", done.number);
                let body = match done.left {
                    0 => format!("{n} {windows} moved to the new zones. Click to undo"),
                    left => format!("{n} {windows} moved to the new zones, {left} left where they were. Click to undo"),
                };
                log::info!("zones pick screen {}: {n} moved, {} left", done.number, done.left);
                (title, body)
            } else {
                let body = match done.left {
                    0 => "Click to undo".to_string(),
                    left => format!("{left} left where they were. Click to undo"),
                };
                log::info!("tile screen {}: {n} placed, {} left", done.number, done.left);
                (format!("Tiled {n} {windows} on screen {}", done.number), body)
            };
            let until = Instant::now() + QUICK_UNDO_FOR;
            app.tile_undo = Some(TileUndo {
                sig: app.sig.clone(),
                screen: done.screen,
                changed: done.changed,
                back: done.back,
                until,
            });
            // With the overlay up, Ctrl+Z is the undo, and a notice would only get in its way.
            if !quicklayout::is_open() {
                notify(app, &title, &body, BalloonAction::UndoTile);
            }
        }
    }
}

/// Open the quick layout overlay on a screen, by Line Up number or the one under
/// the pointer. With zones off, or with another window manager running, it still
/// opens and says so, since a pick is saved either way.
pub(super) fn open_quick_layout(app: &mut App, number: Option<u32>, from_api: bool) -> Result<(), String> {
    let at = match number {
        Some(n) => n.checked_sub(1).map(|i| i as usize),
        None => {
            let At::Point(x, y) = pointer() else { return Err("No such screen".into()) };
            Some(app.mons.iter().position(|m| m.rect.contains(x, y)).unwrap_or(0))
        }
    };
    let mon = at.and_then(|i| app.mons.get(i)).ok_or("No such screen")?;
    let desk = app.cfg.desks.get(&app.sig).ok_or("This desk isn't saved yet")?;
    let now = desk.zones.get(&mon.id).and_then(zones::layout_of);
    let library = seam::library::entries(&app.cfg.custom_layouts);
    let note = aside(app).or_else(|| {
        (!app.cfg.zones.enabled).then(|| "Zones are off. A layout you pick is kept for when they are on".to_string())
    });
    quicklayout::show(quicklayout::Spec {
        number: at.map_or(1, |i| i as u32 + 1),
        rect: mon.rect,
        note,
        choices: quicklayout::choices(&library, &mon.id, now.as_ref()),
        now,
        tray: app.hwnd.0 as isize,
        from_api,
    });
    Ok(())
}

/// The overlay picked layouts: apply each as a click on the Zones page would.
pub(super) fn apply_quick_picks(app: &mut App) {
    for (number, layout) in quicklayout::take_picks() {
        let (reply, _) = channel();
        pick_layout(app, number, layout, reply);
    }
}

/// A layout picked on the Zones page. With zones on, the screen's snapped windows
/// follow it into the new zones on the worker, and a notice offers Undo. Otherwise,
/// or for no zones, it is only saved, still with Undo while zones are on.
pub(super) fn pick_layout(app: &mut App, number: u32, layout: Option<Value>, reply: Sender<Reply>) {
    let Some(screen) = number.checked_sub(1).and_then(|i| app.mons.get(i as usize)).map(|m| m.id.clone()) else {
        let _ = reply.send(Err(RpcError::new(api::NOT_FOUND, "no such screen")));
        return;
    };
    let mut layout = match layout.map(|v| zones::layout_of(&v)) {
        None => None,
        Some(Some(layout)) => Some(layout),
        Some(None) => {
            let _ = reply.send(Err(RpcError::new(api::INVALID_PARAMS, "layout isn't a zone layout")));
            return;
        }
    };
    let Some(desk) = app.cfg.desks.get(&app.sig) else {
        let _ = reply.send(refused("This desk isn't saved yet"));
        return;
    };
    let before = desk.zones.get(&screen).and_then(zones::layout_of);
    if let Some(layout) = layout.as_mut() {
        seam::library::link(&mut app.cfg.custom_layouts, layout, None);
    }
    if layout == before {
        let _ = reply.send(Ok(json!({ "ok": true })));
        return;
    }
    let moving = aside(app).is_none() && app.cfg.zones.enabled && !crate::switcher::switching();
    match layout {
        Some(layout) if moving => {
            let pick = wincontrol::tile::Pick { screen, layout, before };
            queue_window(app, wincontrol::Task::Pick(Box::new(pick)), snap::zoned(), Some(reply));
        }
        layout => {
            if !set_screen_layout(app, &screen, layout.clone(), false) {
                let _ = reply.send(refused("This desk isn't saved yet"));
                return;
            }
            if moving {
                let done = wincontrol::tile::Done {
                    number: number as usize,
                    tiled: 0,
                    left: 0,
                    screen,
                    changed: Some(wincontrol::tile::Change { now: layout, before }),
                    back: Vec::new(),
                    pick: true,
                };
                tile_finished(app, Some(Ok(done)));
            }
            let _ = reply.send(Ok(json!({ "ok": true })));
        }
    }
}

/// The notice was clicked: give the screen back the layout it had before the tile
/// or the pick, if it still has the one they gave it, and put each window that is
/// still where it was set back where it was, while that's still within 8 s. A window
/// that was maximized comes back at its size from then, not maximized: placement
/// has no way to maximize.
pub(super) fn undo_tile(app: &mut App) {
    let Some(undo) = app.tile_undo.take() else { return };
    if Instant::now() > undo.until || undo.sig != app.sig {
        log::info!("tile screen: too late to undo");
        return;
    }
    if let Some(change) = undo.changed {
        let desk = app.cfg.desks.get(&app.sig);
        let now = desk.and_then(|d| d.zones.get(&undo.screen)).and_then(zones::layout_of);
        if desk.is_some() && now == change.now {
            set_screen_layout(app, &undo.screen, change.before, false);
        }
    }
    let here = |b: &wincontrol::tile::Back| {
        let h = HWND(b.place.hwnd as *mut _);
        let (set, sized) = (b.set, seam::winutil::is_resizable(h));
        let near = |a: i32, b: i32| (a - b).abs() <= 4;
        seam::winutil::visible_rect(h)
            .is_some_and(|r| near(r.x, set.x) && near(r.y, set.y) && (!sized || (near(r.w, set.w) && near(r.h, set.h))))
    };
    snap::place_more(undo.back.into_iter().filter(here).map(|b| b.place).collect());
    log::info!("tile screen: undone");
}
