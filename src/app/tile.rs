//! Tile this screen's windows on the main thread: starting it from a key, the tray
//! or the API, and the notice that offers Undo. The worker picks the windows and
//! the zones (`wincontrol::tile`).

use super::workspaces::queue_window;
use super::*;
use wincontrol::tile::At;

/// Queue a tile. The worker answers an API client; a key or the tray hears of a
/// refusal in a notice.
pub(super) fn start_tile(app: &mut App, at: At, reply: Option<Sender<Reply>>) {
    let refusal = aside(app).or_else(|| (!app.cfg.zones.enabled).then(|| "Zones are off".to_string()));
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
            let title = format!("Tiled {n} {} on screen {}", if n == 1 { "window" } else { "windows" }, done.number);
            let body = match done.left {
                0 => "Click to undo".to_string(),
                left => format!("{left} left where they were. Click to undo"),
            };
            log::info!("tile screen {}: {n} placed, {} left", done.number, done.left);
            let until = Instant::now() + QUICK_UNDO_FOR;
            app.tile_undo = Some(TileUndo {
                sig: app.sig.clone(),
                screen: done.screen,
                picked: done.picked,
                back: done.back,
                until,
            });
            notify(app, &title, &body, BalloonAction::UndoTile);
        }
    }
}

/// The notice was clicked: give the screen its layout back, none if the tile picked
/// one, and put each window where it was, while that's still within 8 s.
pub(super) fn undo_tile(app: &mut App) {
    let Some(undo) = app.tile_undo.take() else { return };
    if Instant::now() > undo.until || undo.sig != app.sig {
        log::info!("tile screen: too late to undo");
        return;
    }
    if undo.picked {
        if let Some(desk) = app.cfg.desks.get_mut(&app.sig) {
            desk.zones.remove(&undo.screen);
        }
        persist(app);
        snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
        publish(app);
    }
    snap::place_all(undo.back);
    log::info!("tile screen: undone");
}
