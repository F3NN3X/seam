//! An app rule that sends new windows to a workspace: where the window goes, and whether
//! the workspace in use owns its app away from the rule (brief I).

use super::Rule;

/// What a window an app rule sent to a workspace does on the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// The workspace isn't on this desk.
    Skip,
    /// Join it, leave the window where it opened: the workspace isn't in use.
    Save,
    /// Join it, and place the window where its row says: the workspace is in use.
    SaveAndPlace,
}

/// What a window sent to workspace `id` does, given the ids of this desk's workspaces and the
/// one in use (D776, D778).
pub fn join_target(id: &str, desk_ids: &[String], in_use: Option<&str>) -> Join {
    if !desk_ids.iter().any(|d| d == id) {
        Join::Skip
    } else if in_use == Some(id) {
        Join::SaveAndPlace
    } else {
        Join::Save
    }
}

/// Whether the workspace in use owns an app away from `rule`: it has a row for the app and the
/// rule isn't the one that sends windows to that workspace (D777).
pub fn owned_away(rule: &Rule, in_use: Option<&str>, has_row: bool) -> bool {
    has_row && !(rule.send_to.is_some() && rule.send_to.as_deref() == in_use)
}
