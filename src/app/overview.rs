//! The overview from the tray's side: opening it from the key, the tray menu or the
//! API, and the moves it hands back, which the snapping makes as placements by hand.
//! The overview itself runs on its own thread (`overview_win`).

use super::*;
use crate::overview_win;

/// Open the overview on every screen. The key opens it and closes it again; the API
/// leaves one that is up as it is.
pub(super) fn open_overview(app: &mut App, toggle: bool, from_api: bool) {
    let pressed = Instant::now();
    if toggle && overview_win::close_for_key() {
        return;
    }
    if from_api && overview_win::is_open() {
        return;
    }
    let targets = snap::overview_targets();
    let screen = |(i, m): (usize, &Mon)| {
        let target = targets.iter().find(|t| t.screen == m.id);
        overview_win::Screen {
            number: i as u32 + 1,
            id: m.id.clone(),
            rect: m.rect,
            zones: target.map(|t| t.zones.clone()).unwrap_or_default(),
            tiled: target.is_some_and(|t| t.tiled),
        }
    };
    overview_win::show(overview_win::Spec {
        screens: app.mons.iter().enumerate().map(screen).collect(),
        tray: app.hwnd.0 as isize,
        before: unsafe { GetForegroundWindow() }.0 as isize,
        from_api,
        pressed,
    });
}

/// The pointer rested in the overview's corner. Nothing opens during a drag, while
/// Seam stands by, with a full-screen app in front, or with the overview up already.
pub(super) fn corner_rest(app: &mut App) {
    if standby::reason().is_some() || overview_win::is_open() || dividers::dragging() || mousewatch::button_down() {
        return;
    }
    // Step aside may be off, and a full-screen app is still not to be covered.
    let front = unsafe { GetForegroundWindow() };
    if !front.is_invalid() && seam::winutil::is_full_screen(front) {
        return;
    }
    open_overview(app, false, false);
}

/// Make the moves the overview dropped, and say why one couldn't be made. False when
/// the snapping was busy, and the rest wait for the next try.
pub(super) fn overview_moves(app: &mut App) -> bool {
    let mut busy = Vec::new();
    for m in overview_win::take_moves() {
        if !busy.is_empty() {
            busy.push(m);
            continue;
        }
        match snap::overview_place(m.hwnd, &m.screen, m.zone, m.free) {
            Some(Ok(())) => {}
            Some(Err(why)) => {
                log::info!("overview: {why}");
                notify(app, "Overview", &why, BalloonAction::None);
            }
            None => busy.push(m),
        }
    }
    let done = busy.is_empty();
    if !done {
        overview_win::put_back(busy);
    }
    done
}
