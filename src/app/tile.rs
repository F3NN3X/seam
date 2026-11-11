//! Tile this screen's windows on the main thread: starting it from a key, the tray
//! or the API, and the notice that offers Undo. The worker picks the windows and
//! the zones (`wincontrol::tile`).

use super::workspaces::queue_window;
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

/// The notice was clicked: take away the layout the tile picked, if it is still the
/// screen's, and put each window that is still where the tile set it back where it
/// was, while that's still within 8 s. A window that was maximized comes back at its
/// size from then, not maximized: placement has no way to maximize.
pub(super) fn undo_tile(app: &mut App) {
    let Some(undo) = app.tile_undo.take() else { return };
    if Instant::now() > undo.until || undo.sig != app.sig {
        log::info!("tile screen: too late to undo");
        return;
    }
    if let Some(picked) = undo.picked {
        if let Some(desk) = app.cfg.desks.get_mut(&app.sig) {
            let same = desk.zones.get(&undo.screen).and_then(zones::layout_of).is_some_and(|now| now == picked);
            if same {
                desk.zones.remove(&undo.screen);
                persist(app);
                snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
                publish(app);
            }
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
