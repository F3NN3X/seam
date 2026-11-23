//! Workspace edits an assistant makes through the API (D534). Each is saved as the
//! editor would save it, shows the Undo notice a quick save does, and is logged with the client.

use super::*;

/// The stored copy of a workspace of this desk, as the config holds it.
fn stored(app: &App, id: &str) -> Option<Value> {
    let desk = app.cfg.desks.get(&app.sig)?;
    desk.workspaces.iter().find(|v| workspace::parse(v).is_some_and(|w| w.id == id)).cloned()
}

/// Keep what Undo puts back, and say so. Undo is a click on the notice, as after a quick save.
fn offer_undo(app: &mut App, saved: &Workspace, before: Option<Value>, from: Option<Vec<Option<usize>>>, what: &str) {
    let Some(after) = stored(app, &saved.id) else { return };
    let in_use = in_use_now(app);
    let until = Instant::now() + QUICK_UNDO_FOR;
    app.quick_undo = Some(QuickUndo {
        sig: app.sig.clone(),
        id: saved.id.clone(),
        before,
        after,
        from,
        in_use,
        until,
        source: None,
    });
    notify(app, &format!("Changed {}", saved.name), &format!("{what}. Click to undo"), BalloonAction::UndoQuickSave);
}

/// `workspace.create`: an empty workspace named as asked, not yet in use.
pub(in crate::app) fn create_workspace(app: &mut App, client: &str, assistant: bool, name: &str) -> Reply {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return refused("This desk has no workspaces") };
    let existing = workspace::list(&desk.workspaces);
    if assistant && existing.len() >= workspace::MAX_ASSISTANT_WORKSPACES {
        return refused(format!(
            "This desk already has {} workspaces, so Seam won't make another for an assistant",
            workspace::MAX_ASSISTANT_WORKSPACES
        ));
    }
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let created =
        Workspace { id: workspace::new_id(&ids), name: workspace::unique_name(name, &names), ..Workspace::default() };
    let mut list = desk.workspaces.clone();
    list.push(workspace::to_value(&created));
    let patch = json!({ "desks": { app.sig.clone(): { "workspaces": list } } });
    patch_config(app, &patch)?;
    log::info!("assistant {client}: created workspace {}", created.name);
    offer_undo(app, &created, None, None, "Created an empty workspace");
    Ok(json!({ "id": created.id, "name": created.name }))
}

/// Rename, edit a row, remove a row or set Show every window, through the editor's own check.
pub(in crate::app) fn edit_workspace(app: &mut App, client: &str, id: &str, edit: &workspace::Edit) -> Reply {
    use workspace::Edit;
    let Some(before) = stored(app, id) else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    let Some(old) = workspace::parse(&before) else { return refused("That workspace can't be read") };
    let edited = match workspace::edited(&old, edit) {
        Ok(ws) => ws,
        Err(why) => return refused(why),
    };
    update_workspace(app, id, workspace::to_value(&edited))?;
    let (what, from) = match edit {
        Edit::Rename(_) => ("Renamed it".to_string(), None),
        Edit::ShowAll(on) => (format!("Show every window {}", if *on { "on" } else { "off" }), None),
        Edit::Row { row, .. } => (format!("Changed row {}", row + 1), None),
        Edit::RemoveRow(row) => {
            (format!("Removed row {}", row + 1), Some(workspace::survivors(old.windows.len(), *row)))
        }
    };
    log::info!("assistant {client}: {what} in {}", old.name);
    let saved = stored(app, id).and_then(|v| workspace::parse(&v)).unwrap_or(edited);
    offer_undo(app, &saved, Some(before), from, &what);
    // A name that was taken got " 2", so the assistant is told what it ended up as.
    Ok(json!({ "ok": true, "name": saved.name }))
}
