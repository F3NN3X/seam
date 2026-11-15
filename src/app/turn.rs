//! Rotate windows and Reset sizes from the tray's side: the refusals, the screen they
//! mean, and the two ways a screen goes. A screen with zones goes through the window
//! worker, which reads where the windows are; a tiled screen, whose zones are its tree
//! in memory, goes through the snapping, by its one rule. Either ends in a tile's
//! notice with Undo (`tile_finished`).

use super::workspaces::queue_window;
use super::*;
use seam::library::Origin;
use wincontrol::tile::{Done, Pick, Turn};

/// Why neither can run now, if it can't.
fn refusal(app: &App) -> Option<String> {
    aside(app)
        .or_else(|| (!app.cfg.zones.enabled).then(|| "Zones are off".to_string()))
        .or_else(|| crate::switcher::switching().then(|| "A workspace switch is still running".to_string()))
}

/// Say why nothing happened: to an API client, or in a notice for a key or the tray.
fn refuse(app: &mut App, title: &str, why: String, reply: Option<Sender<Reply>>) {
    log::info!("{}: {why}", title.to_lowercase());
    match reply {
        Some(tx) => {
            let _ = tx.send(refused(why));
        }
        None => {
            notify(app, title, &why, BalloonAction::None);
        }
    }
}

/// Start a rotation, forward or back, or a reset, on the screen with this Line Up number
/// or the one under the pointer. The worker, or the snapping for a tiled screen, answers.
pub(super) fn start_turn(app: &mut App, verb: Verb, number: Option<u32>, forward: bool, reply: Option<Sender<Reply>>) {
    let title = if verb == Verb::Rotate { "Rotate windows" } else { "Reset sizes" };
    if let Some(why) = refusal(app) {
        return refuse(app, title, why, reply);
    }
    let Some(screen) = tiling::screen_of(app, number) else {
        if let Some(tx) = reply {
            let _ = tx.send(Err(RpcError::new(api::NOT_FOUND, "no such screen")));
        }
        return;
    };
    let n = app.mons.iter().position(|m| m.id == screen).map_or(1, |i| i + 1);
    if snap::is_tiled(&screen) {
        let turned = match verb {
            Verb::Rotate => snap::tiling_rotate(&screen, forward),
            Verb::Reset => snap::tiling_reset(&screen),
        };
        return tiled_turned(app, (verb, title), n, screen, turned, reply);
    }
    match verb {
        Verb::Rotate => queue_window(app, wincontrol::Task::Rotate(At::Screen(n), forward), snap::zoned(), reply),
        Verb::Reset => match reset_pick(app, &screen, n) {
            Ok(pick) => queue_window(app, wincontrol::Task::Reset(Box::new(pick)), snap::zoned(), reply),
            Err(why) => refuse(app, title, why, reply),
        },
    }
}

/// The layout a screen goes back to, for a reset: only one the Zones page shows as
/// Changed has one, and the library finds which it came from.
fn reset_pick(app: &App, screen: &str, number: usize) -> Result<Pick, String> {
    let desk = app.cfg.desks.get(&app.sig).ok_or("This desk isn't saved yet")?;
    let layout =
        desk.zones.get(screen).and_then(zones::layout_of).ok_or_else(|| format!("Screen {number} has no zones"))?;
    let library = seam::library::entries(&app.cfg.custom_layouts);
    if !seam::library::changed(&library, &layout) {
        return Err(format!("Screen {number}'s layout hasn't been changed"));
    }
    let from = match seam::library::origin(&app.cfg.custom_layouts, screen, &layout.tree) {
        Origin::Layout(from) => from,
        Origin::Unclear => {
            return Err(format!(
                "Screen {number}'s layout could have come from more than one layout, so Seam won't guess"
            ));
        }
        Origin::Unknown => return Err(format!("Seam can't tell which layout screen {number} came from")),
    };
    Ok(Pick { screen: screen.to_string(), layout: from, before: Some(layout), focused: None })
}

/// A tiled screen's turn is over: the notice offers Undo as a tile's does, and the API client hears how many moved.
fn tiled_turned(
    app: &mut App,
    (verb, title): (Verb, &str),
    number: usize,
    screen: String,
    turned: Result<snap::Turned, String>,
    reply: Option<Sender<Reply>>,
) {
    let turned = match turned {
        Ok(turned) => turned,
        Err(why) => return refuse(app, title, why, reply),
    };
    let (moved, left) = (turned.moved, turned.left);
    let done = Done {
        number,
        tiled: moved,
        left,
        screen,
        changed: None,
        back: turned.back,
        keep: Vec::new(),
        pick: false,
        split: false,
        pulled: false,
        turn: Some(Turn { verb, tree: turned.tree }),
    };
    tile_finished(app, Some(Ok(done)));
    if let Some(tx) = reply {
        let _ = tx.send(Ok(json!({ "ok": true, "moved": moved, "left": left })));
    }
}
