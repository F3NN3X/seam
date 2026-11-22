//! Next and previous workspace, from a key or `workspace.step`, and moving the window in
//! front into one.

use super::*;

/// The index in `list` a step lands on. While an earlier step's switch is still running, a
/// quick press goes on from where that one is headed (`step_to`), not from the workspace
/// still in use.
pub(super) fn step_target(list: &[Workspace], in_use: Option<&str>, step_to: Option<&str>, dir: Step) -> Option<usize> {
    workspace::stepped(list, step_to.or(in_use), dir, true)
}

/// What to say when there is nothing to step to.
fn nothing_to_step_to(list: &[Workspace]) -> &'static str {
    if list.is_empty() { "This desk has no workspaces" } else { "This desk has only one workspace" }
}

/// Switch to the next or previous workspace of the desk, counted in its list. `screen` is for
/// a later per-screen scope and is ignored.
pub(in crate::app) fn step(app: &mut App, dir: Step, _screen: Option<usize>, alongside: bool, via: Via) -> Reply {
    let list = desk_workspaces(app);
    let Some(i) = step_target(&list, app.workspace.as_deref(), app.step_to.as_deref(), dir) else {
        return Err(RpcError::new(api::NOT_FOUND, nothing_to_step_to(&list)));
    };
    let id = list[i].id.clone();
    // Starting a switch drops a chain, so a refused one gives it back.
    let chain = app.step_to.clone();
    let reply = start_switch(app, Some(&id), None, alongside, via);
    app.step_to = if reply.is_ok() { Some(id) } else { chain };
    reply
}

/// The workspace "Move window to next workspace" or "previous" moves into, from the one in use.
pub(in crate::app) fn step_add(app: &mut App, dir: Step) {
    let list = desk_workspaces(app);
    // From where a step still running is headed, as the next step would go.
    match step_target(&list, app.workspace.as_deref(), app.step_to.as_deref(), dir) {
        Some(i) => start_move_window(app, Which::Id(list[i].id.clone())),
        None => refuse_move(app, nothing_to_step_to(&list)),
    }
}
