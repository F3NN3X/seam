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

/// The workspaces a step on `screen` goes through, and the one it starts from (D785): that
/// screen's own and the All-screens ones, from the one in use there. With no screen, every
/// workspace whose screen is connected, from the one meant.
fn cycle_on(app: &App, screen: Option<&str>) -> (Vec<Workspace>, Option<String>) {
    let list = desk_workspaces(app);
    match screen {
        Some(screen) => (workspace::cycle(&list, screen), meant(app, Some(screen))),
        None => {
            let ids = screen_ids(app);
            (list.into_iter().filter(|w| workspace::scope_connected(w, &ids)).collect(), meant(app, None))
        }
    }
}

/// Switch to the next or previous workspace of the desk, counted in its list, on the screen
/// `on` means. With none named, from the one the tray means, on its screen if it has one.
pub(in crate::app) fn step(app: &mut App, dir: Step, on: OnScreen, alongside: bool, via: Via) -> Reply {
    let screen = screen_meant(app, on)?.or_else(|| {
        let id = meant(app, None)?;
        desk_workspaces(app).into_iter().find(|w| w.id == id)?.scope
    });
    let (list, in_use) = cycle_on(app, screen.as_deref());
    let Some(i) = step_target(&list, in_use.as_deref(), app.step_to.as_deref(), dir) else {
        return Err(RpcError::new(api::NOT_FOUND, nothing_to_step_to(&list)));
    };
    let id = list[i].id.clone();
    // Starting a switch drops a chain, so a refused one gives it back.
    let chain = app.step_to.clone();
    let reply = start_switch(app, Some(&id), None, alongside, via);
    app.step_to = if reply.is_ok() { Some(id) } else { chain };
    reply
}

/// The workspace "Move window to next workspace" or "previous" moves into, from the one in use
/// on the window's screen.
pub(in crate::app) fn step_add(app: &mut App, dir: Step) {
    let front = unsafe { GetForegroundWindow() };
    let at = seam::winutil::window_rect(front).map(|r| (r.x + r.w / 2, r.y + r.h / 2));
    let screen = at.and_then(|(x, y)| app.mons.iter().find(|m| m.rect.contains(x, y))).map(|m| m.id.clone());
    let (list, in_use) = cycle_on(app, screen.as_deref());
    // From where a step still running is headed, as the next step would go.
    match step_target(&list, in_use.as_deref(), app.step_to.as_deref(), dir) {
        Some(i) => {
            let from = super::moved::origin(in_use.as_deref(), app.step_to.as_deref());
            start_move_window(app, Which::Id(list[i].id.clone()), from);
        }
        None => refuse_move(app, nothing_to_step_to(&list)),
    }
}
