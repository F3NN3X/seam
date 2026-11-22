//! "Add window to workspace": the window in front joins a workspace of the desk, from a
//! key or `workspace.add_window`. The worker reads the window as a capture would, and the
//! row is saved through the editor's checks, with Undo as a quick save has.

use super::*;
use std::path::Path;

/// Which workspace of the desk: its number in the list, from 1, or its id.
pub(in crate::app) enum Which {
    Number(usize),
    Id(String),
}

const TITLE: &str = "Add window to workspace";

/// Start adding `hwnd`, or the window in front, to a workspace. An API client hears how
/// it went through `reply`; a key shows a notice when it can't.
pub(in crate::app) fn start_add_window(app: &mut App, which: Which, hwnd: Option<isize>, reply: Option<Sender<Reply>>) {
    if let Err(e) = begin(app, which, hwnd, reply.clone()) {
        answer(app, reply, Err(e));
    }
}

fn begin(app: &mut App, which: Which, hwnd: Option<isize>, reply: Option<Sender<Reply>>) -> Result<(), RpcError> {
    if app.closing.is_some() {
        return Err(RpcError::new(api::BUSY, "Windows of a workspace are still being closed or minimized"));
    }
    let list = desk_workspaces(app);
    let found = match &which {
        Which::Number(n) => workspace::numbered(&list, *n)
            .ok_or_else(|| RpcError::new(api::NOT_FOUND, format!("This desk has no workspace {n}")))?,
        Which::Id(id) => list
            .iter()
            .find(|w| w.id == *id)
            .ok_or_else(|| RpcError::new(api::NOT_FOUND, "no such workspace on this desk"))?,
    };
    if app.capturing.is_some() {
        return Err(RpcError::new(api::BUSY, "Already capturing"));
    }
    let hwnd = hwnd.unwrap_or_else(|| unsafe { GetForegroundWindow() }.0 as isize);
    if hwnd == 0 {
        return Err(RpcError::new(api::REFUSED, "No window is in front"));
    }
    let into = Some(found.id.clone());
    app.capturing = Some(Capturing {
        reply,
        name: None,
        into,
        row: None,
        pick: None,
        sig: app.sig.clone(),
        quick: false,
        add: Some(hwnd),
    });
    if switcher::capture_added(capture_job(app), hwnd) {
        return Ok(());
    }
    app.capturing = None;
    Err(RpcError::new(api::REFUSED, "Couldn't start reading the window"))
}

/// Say how it went: the answer for the API client if there is one, else a notice for a
/// refusal. A save has its own notice.
fn answer(app: &mut App, reply: Option<Sender<Reply>>, result: Reply) {
    match (reply, result) {
        (Some(tx), result) => {
            let _ = tx.send(result);
        }
        (None, Err(e)) => {
            log::info!("add window: {}", e.message);
            notify(app, TITLE, &e.message, BalloonAction::None);
        }
        (None, Ok(_)) => {}
    }
}

/// The worker has read the window.
pub(super) fn added(app: &mut App) {
    let (Some(read), Some(job)) = (switcher::take_added(), app.capturing.take()) else { return };
    let result = save(app, &job, read);
    answer(app, job.reply, result);
}

/// What to call the app in a notice: its program's name, or failing that its window's title.
fn label(rule: &Rule) -> String {
    let stem = Path::new(&rule.exe).file_stem().map(|s| s.to_string_lossy().into_owned());
    stem.filter(|s| !s.is_empty()).unwrap_or_else(|| rule.title_hint.clone())
}

/// The workspace as the file has it.
fn stored(app: &App, id: &str) -> Option<Value> {
    let desk = app.cfg.desks.get(&app.sig)?;
    desk.workspaces.iter().find(|v| workspace::parse(v).is_some_and(|w| w.id == id)).cloned()
}

/// Put the folder or page the window showed into its row. Only a capture reads one, so
/// the editor's checks leave it out and this adds it after them.
fn store_place(app: &mut App, id: &str, row: usize, place: workspace::Place) {
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return };
    let Some(slot) = desk.workspaces.iter_mut().find(|v| workspace::parse(v).is_some_and(|w| w.id == id)) else {
        return;
    };
    let Some(mut ws) = workspace::parse(slot) else { return };
    if let Some(rule) = ws.windows.get_mut(row) {
        rule.place = Some(place);
        *slot = workspace::to_value(&ws);
        persist(app);
    }
}

/// Fold the window into its workspace and save it the way an edit is saved, so the same
/// checks apply, then keep what Undo puts back.
fn save(app: &mut App, job: &Capturing, read: Result<(Rule, isize), String>) -> Reply {
    let (rule, hwnd) = read.map_err(|why| RpcError::new(api::REFUSED, why))?;
    let Some(id) = job.into.as_deref() else { return refused("No workspace to add to") };
    if job.sig != app.sig {
        return refused("The desk changed");
    }
    let old = desk_workspaces(app).into_iter().find(|w| w.id == id);
    let (Some(old), Some(before)) = (old, stored(app, id)) else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    let remember = app.cfg.workspaces_remember_places;
    let name = label(&rule);
    let (bound, _) = switcher::capture_bindings(&old, &[hwnd], remember);
    let added = workspace::add_window(&old, rule, &bound);
    super::capture::update_rows(app, id, workspace::to_value(&added.workspace), Some(added.row))?;
    if let Some(place) = added.place {
        store_place(app, id, added.row, place);
    }
    let (Some(after), Some(saved)) = (stored(app, id), desk_workspaces(app).into_iter().find(|w| w.id == id)) else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    // The window holds its row, as after a capture, so drift and auto-apply see it at once.
    if remember {
        switcher::bind_rows(&saved, &[(hwnd, added.row)]);
    }
    live::stamp(workspace::stamp_key(id), vec![(hwnd, added.row)], false);
    if app.workspace.as_deref() == Some(id) {
        sync_live(app);
        live::check_drift();
    }
    // Which old row each row is now, for Undo: the row it added is none of them.
    let from = (0..saved.windows.len()).map(|i| (i < old.windows.len()).then_some(i)).collect();
    let (in_use, until) = (app.workspace.clone(), Instant::now() + QUICK_UNDO_FOR);
    app.quick_undo = Some(QuickUndo {
        sig: app.sig.clone(),
        id: id.to_string(),
        before: Some(before),
        after,
        from: Some(from),
        in_use,
        until,
    });
    let off = saved.windows.get(added.row).is_some_and(|r| r.off);
    let (title, body) = match (added.new, off) {
        (true, _) => {
            (format!("Added to {}", saved.name), format!("{name} has a new row that won't start it. Click to undo"))
        }
        (false, true) => (
            format!("Updated {name} in {}", saved.name),
            "It's off, so a switch leaves it alone. Click to undo".to_string(),
        ),
        (false, false) => {
            (format!("Added to {}", saved.name), format!("{name}'s row now has where it is. Click to undo"))
        }
    };
    log::info!("add window: {name} {} {}", if added.new { "added to" } else { "updated in" }, saved.name);
    notify(app, &title, &body, BalloonAction::UndoQuickSave);
    publish(app);
    Ok(json!({ "id": id, "row": added.row, "new": added.new }))
}
