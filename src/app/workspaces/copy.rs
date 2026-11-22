//! Copying another desk's workspaces onto this empty one (D741-D745).

use super::*;

/// `workspace.copy_from_desk`: the workspaces of desk `from` become this desk's, each screen's
/// apps on the screen in the same place. Answers the ids of the copies.
pub(in crate::app) fn copy_from_desk(app: &mut App, from: &str) -> Reply {
    let here = app.sig.clone();
    let copied = workspace::copy_between(&mut app.cfg, from, &here, workspace::new_id).map_err(|e| {
        let code = if e == workspace::CopyError::NoSource { api::NOT_FOUND } else { api::REFUSED };
        RpcError::new(code, e.message())
    })?;
    log::info!("copied {} workspaces from another desk", copied.len());
    persist(app);
    // The copies' own shortcuts are kept, and act on this desk.
    apply_hotkeys(app);
    publish(app);
    Ok(json!({ "ids": copied }))
}
