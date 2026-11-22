//! Moving a workspace in the desk's list, which is its number (D691).

use super::*;

/// `workspace.move`: workspace `id` becomes number `to`. The shortcuts that count in list
/// order (Add window to workspace 1 to 9) and the tray's items follow at once, because
/// saving applies the hotkeys and rebuilds the menu. Each workspace's own shortcut stays.
pub(in crate::app) fn move_workspace(app: &mut App, id: &str, to: usize) -> Reply {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return refused("This desk has no workspaces") };
    let list = match workspace::moved_to(&desk.workspaces, id, to) {
        Ok(list) => list,
        Err(workspace::MoveError::NotFound) => {
            return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
        }
        Err(workspace::MoveError::OutOfRange) => {
            return Err(RpcError::new(api::INVALID_PARAMS, "to must be a workspace number on this desk"));
        }
    };
    let patch = json!({ "desks": { app.sig.clone(): { "workspaces": list } } });
    patch_config(app, &patch)?;
    log::info!("workspace {id} moved to number {to}");
    Ok(json!({ "ok": true, "number": to }))
}
