//! "Add window to workspace": the window in front joins a workspace of the desk, from a
//! key or `workspace.add_window`. The worker reads the window as a capture would, and the
//! row is saved through the editor's checks, with Undo as a quick save has.

use super::capture::{check_rows, write_checked};
use super::*;
use std::path::Path;

/// Which workspace of the desk: its number in the list, from 1, or its id.
pub(in crate::app) enum Which {
    Number(usize),
    Id(String),
}

const TITLE: &str = "Add window to workspace";
const MOVE_TITLE: &str = "Move window to workspace";

/// Start adding `hwnd`, or the window in front, to a workspace. An API client hears how
/// it went through `reply`; a key shows a notice when it can't.
pub(in crate::app) fn start_add_window(app: &mut App, which: Which, hwnd: Option<isize>, reply: Option<Sender<Reply>>) {
    start(app, which, hwnd, reply, false);
}

/// "Move window to next workspace" or "previous": the window in front joins a workspace as
/// above, and its row goes from the workspace in use.
pub(in crate::app) fn start_move_window(app: &mut App, which: Which) {
    start(app, which, None, None, true);
}

/// A move's notice when there is no workspace to go to.
pub(in crate::app) fn refuse_move(app: &mut App, why: &str) {
    answer(app, None, Err(RpcError::new(api::NOT_FOUND, why)), true);
}

fn start(app: &mut App, which: Which, hwnd: Option<isize>, reply: Option<Sender<Reply>>, moving: bool) {
    if let Err(e) = begin(app, which, hwnd, reply.clone(), moving) {
        answer(app, reply, Err(e), moving);
    }
}

fn begin(
    app: &mut App,
    which: Which,
    hwnd: Option<isize>,
    reply: Option<Sender<Reply>>,
    moving: bool,
) -> Result<(), RpcError> {
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
    let move_from = if moving { app.workspace.clone() } else { None };
    app.capturing = Some(Capturing {
        reply,
        name: None,
        into,
        row: None,
        pick: None,
        sig: app.sig.clone(),
        quick: false,
        add: Some(hwnd),
        move_from,
        preview: false,
        closed: BTreeMap::new(),
        from_panel: false,
    });
    if switcher::capture_added(capture_job(app), hwnd) {
        return Ok(());
    }
    app.capturing = None;
    Err(RpcError::new(api::REFUSED, "Couldn't start reading the window"))
}

/// Say how it went: the answer for the API client if there is one, else a notice for a
/// refusal. A save has its own notice.
fn answer(app: &mut App, reply: Option<Sender<Reply>>, result: Reply, moving: bool) {
    match (reply, result) {
        (Some(tx), result) => {
            let _ = tx.send(result);
        }
        (None, Err(e)) => {
            log::info!("add window: {}", e.message);
            notify(app, if moving { MOVE_TITLE } else { TITLE }, &e.message, BalloonAction::None);
        }
        (None, Ok(_)) => {}
    }
}

/// The worker has read the window.
pub(super) fn added(app: &mut App) {
    let (Some(read), Some(job)) = (switcher::take_added(), app.capturing.take()) else { return };
    let result = save(app, &job, read);
    answer(app, job.reply, result, job.move_from.is_some());
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

/// A move's workspace on the leaving side: as it is stored, and without the window's row,
/// when it has one. `None` when it has no row for the window, so the move only adds.
fn leaving(app: &App, from: &str, rule: Rule, hwnd: isize, remember: bool) -> Option<Leaving> {
    let before = stored(app, from)?;
    let old = workspace::parse(&before)?;
    let (bound, _) = switcher::capture_bindings(&old, &[hwnd], remember);
    let (without, row) = super::moved::without_row(&old, rule, &bound)?;
    Some(Leaving { before, without, row, rows: old.windows.len() })
}

struct Leaving {
    before: Value,
    without: Workspace,
    /// The row that goes, and how many rows there were.
    row: usize,
    rows: usize,
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
    let going = job.move_from.as_deref().filter(|from| *from != id);
    let out = going.and_then(|from| leaving(app, from, rule.clone(), hwnd, remember));
    let added = workspace::add_window(&old, rule, &bound);
    // Both workspaces are checked first and written once, so a move is whole or not at all.
    let target = check_rows(app, id, workspace::to_value(&added.workspace), Some(added.row), false)?;
    let moved = match (going, out) {
        (Some(from), Some(Leaving { before, without, row, rows })) => {
            let source = check_rows(app, from, workspace::to_value(&without), None, false)?;
            write_checked(app, &[target, source])?;
            let after = stored(app, from);
            after.map(|after| MovedFrom { id: from.to_string(), before, after, from: workspace::survivors(rows, row) })
        }
        _ => {
            write_checked(app, &[target])?;
            None
        }
    };
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
        source: moved,
    });
    let left = app.quick_undo.as_ref().and_then(|u| u.source.as_ref()).and_then(|s| workspace::parse(&s.before));
    if left.is_some() {
        sync_live(app);
    }
    let off = saved.windows.get(added.row).is_some_and(|r| r.off);
    let (title, body) = match (added.new, off) {
        _ if left.is_some() => {
            let from = left.map(|w| w.name).unwrap_or_default();
            let body = if off {
                "It's off, so a switch leaves it alone. Click to undo".to_string()
            } else {
                format!("{name} is no longer in {from}. Click to undo")
            };
            (format!("Moved to {}", saved.name), body)
        }
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
