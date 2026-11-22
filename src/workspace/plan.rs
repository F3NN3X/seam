//! What a switch does with each row, apart from the switcher that does it.

use super::*;

/// What a switch does with one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    /// Move its open window to its place.
    Place,
    /// Start its app, if it's closed.
    Start,
    /// Nothing.
    Leave,
}

/// What `skipped` says about an app whose saved screen isn't connected.
pub const SCREEN_GONE: &str = "screen not connected";

/// A row that is off takes no part in a switch, Snap back, auto-apply or in-use
/// detection (D693).
pub fn takes_part(rule: &Rule) -> bool {
    !rule.off
}

/// The rule sends its window to a screen that isn't among the `connected` ones.
pub fn screen_gone<'a>(rule: &Rule, mut connected: impl Iterator<Item = &'a str>) -> bool {
    rule.moves() && !connected.any(|id| id == rule.monitor)
}

/// What a switch does with a row, given the window it found. With the workspace's
/// `open_windows` at Leave, windows already open stay put, unless `put_back` (Snap
/// back) asks for them. A row with launch off is placed but never started.
pub fn row_action(rule: &Rule, found: Option<isize>, open_windows: OpenWindows, put_back: bool) -> RowAction {
    match found {
        _ if !takes_part(rule) => RowAction::Leave,
        Some(_) if open_windows == OpenWindows::Leave && !put_back => RowAction::Leave,
        Some(_) => RowAction::Place,
        None if rule.launch => RowAction::Start,
        None => RowAction::Leave,
    }
}

/// The tray's notice after a switch that left apps where they were because their screen
/// isn't connected (D696): the body, or `None` when no app was left. Counts apps, not windows.
pub fn screen_gone_note(skipped: &[(String, String)]) -> Option<String> {
    let mut apps: Vec<&str> =
        skipped.iter().filter(|(_, why)| why == SCREEN_GONE).map(|(app, _)| app.as_str()).collect();
    apps.sort_unstable();
    apps.dedup();
    match apps.as_slice() {
        [] => None,
        [app] => Some(format!("{app} stayed where it is, because its screen isn't connected")),
        many => Some(format!("{} apps stayed where they are, because their screen isn't connected", many.len())),
    }
}
