//! What a switch tells about itself (D711 to D716): a notice while it starts apps that Esc or a
//! click stops, and one after it for the apps it couldn't start or move.

use super::*;
use crate::switcher::{Skip, Starting, Switched};
use seam::workspace::report;

/// What the last switch of a workspace did, kept in memory until its next switch or a restart (D713).
pub(in crate::app) struct LastSwitch {
    /// Milliseconds since 1970.
    at: u64,
    cancelled: bool,
    skipped: Vec<Skip>,
}

/// How long a switch starts apps before the tray says so.
const STARTING_AFTER_MS: u32 = 2_000;

/// The skipped apps as the API tells them: the row and the app, why it was skipped, and the
/// words when that is a problem.
pub(in crate::app) fn skipped_json(skipped: &[Skip]) -> Vec<Value> {
    let entry = |s: &Skip| json!({ "row": s.row, "app": s.app, "reason": s.reason, "why": report::words(&s.reason) });
    skipped.iter().map(entry).collect()
}

/// `workspace.last_switch`: what the workspace's last switch did, or null.
pub(in crate::app) fn last_switch(app: &App, id: &str) -> Value {
    let Some(last) = app.last_switch.get(id) else { return Value::Null };
    json!({ "at": last.at, "cancelled": last.cancelled, "skipped": skipped_json(&last.skipped) })
}

/// The Seam window is connected and on screen, so it says it itself.
fn panel_open(app: &App) -> bool {
    crate::tray::panel_shows(app.clients.iter().any(|c| c == "panel"), crate::tray::panel_visible())
}

/// A switch begins starting apps: Esc stops it from now on, and the balloon follows if it takes a while.
pub(in crate::app) fn starting(app: &mut App) {
    let Some(starting) = switcher::take_starting() else { return };
    // Esc is the app in front's while Seam stands aside for a game.
    if standby::reason().is_none() {
        escape::install();
    }
    let data = json!({ "id": starting.id, "name": starting.name, "starting": starting.apps });
    api_server::broadcast("workspace.switching", &data);
    unsafe { SetTimer(Some(app.hwnd), TIMER_STARTING, STARTING_AFTER_MS, None) };
    app.starting_balloon = false;
    app.starting = Some(starting);
}

/// The start phase of switch `generation` ended, unless a newer one has begun.
pub(in crate::app) fn started(app: &mut App, generation: u64) {
    if app.starting.as_ref().is_some_and(|s| s.generation == generation) {
        end_start_phase(app);
    }
}

fn end_start_phase(app: &mut App) {
    escape::remove();
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_STARTING);
    }
    app.starting = None;
}

/// The switch has been starting apps for a while: say so, with a click that stops it. The Seam
/// window says it itself for a switch it asked for, while it is open.
pub(in crate::app) fn starting_due(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_STARTING);
    }
    let Some(Starting { name, apps, .. }) = &app.starting else { return };
    // The Seam window has its own notice with a button, so one stop is enough.
    if panel_open(app) {
        return;
    }
    let (title, body) = report::starting(name, *apps, true);
    app.starting_balloon = notify(app, &title, &body, BalloonAction::CancelSwitch);
}

/// A switch ended: remember what it did, and tell what went wrong, if anything. A balloon the
/// start phase showed goes, or is replaced by the summary.
pub(in crate::app) fn ended(app: &mut App, done: &Switched) {
    end_start_phase(app);
    let session = done.id == SESSION_ID;
    let named = done.skipped.iter().map(|s| (s.app.as_str(), s.reason.as_str()));
    let problems = if session { Vec::new() } else { report::problems(named) };
    let left = done.cancelled.then(|| report::left_unstarted(done.skipped.iter().map(|s| (s.row, s.reason.as_str()))));
    if !session {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let last = LastSwitch { at, cancelled: done.cancelled, skipped: done.skipped.clone() };
        app.last_switch.insert(done.id.clone(), last);
    }
    // The Seam window shows the result itself, while it is open.
    let panel_shows = done.from_panel && panel_open(app);
    let notice = if session || panel_shows { None } else { report::summary(&done.name, &problems, left) };
    let shown = std::mem::take(&mut app.starting_balloon);
    // A click on the summary opens the workspace it names, not a later one.
    app.switched_id = None;
    match notice {
        Some((title, body)) => {
            let click = if problems.is_empty() { BalloonAction::None } else { BalloonAction::OpenSwitched };
            if !problems.is_empty() {
                app.switched_id = Some(done.id.clone());
            }
            notify(app, &title, &body, click);
        }
        None if shown => clear_balloon(app),
        None => {}
    }
}

/// A click on the summary: the Seam window on the workspace that was switched.
pub(in crate::app) fn open_switched(app: &mut App) {
    let id = app.switched_id.clone();
    open_panel_with(app, Some("workspaces"), None, id.as_deref());
}
