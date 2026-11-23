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

/// What `skipped` says about a row of a one-screen workspace that is on another screen (D788).
/// Not a problem: the row is kept, and the editor offers to move it.
pub const OTHER_SCREEN: &str = "on another screen";

/// A row that is off takes no part in a switch, Snap back, auto-apply or in-use
/// detection (D693).
pub fn takes_part(rule: &Rule) -> bool {
    !rule.off
}

/// A switch keeps the row's app from Other windows though the row takes no part: it is off
/// (D694), or on another screen than its workspace's (D788).
pub fn keeps_its_app(rule: &Rule, scope: Option<&str>) -> bool {
    !takes_part(rule) || !in_scope(rule, scope)
}

/// The rule sends its window to a screen that isn't among the `connected` ones.
pub fn screen_gone<'a>(rule: &Rule, mut connected: impl Iterator<Item = &'a str>) -> bool {
    rule.moves() && !connected.any(|id| id == rule.monitor)
}

/// What a switch does with a row, given the window it found. With the workspace's
/// `open_windows` at Leave, windows already open stay put, unless `put_back` (Snap
/// back) asks for them. A row with launch off is placed but never started. A row on
/// another screen than the workspace's `scope` is left alone.
pub fn row_action(
    rule: &Rule,
    found: Option<isize>,
    open_windows: OpenWindows,
    put_back: bool,
    scope: Option<&str>,
) -> RowAction {
    match found {
        _ if keeps_its_app(rule, scope) => RowAction::Leave,
        Some(_) if open_windows == OpenWindows::Leave && !put_back => RowAction::Leave,
        Some(_) => RowAction::Place,
        None if rule.launch => RowAction::Start,
        None => RowAction::Leave,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_on_another_screen_take_no_part_and_their_apps_stay_out_of_other_windows() {
        let row = |monitor: &str| Rule { exe: "a.exe".into(), monitor: monitor.into(), ..Rule::default() };
        let (here, there) = (row("s2"), row("s1"));
        let act = |r: &Rule, found| row_action(r, found, OpenWindows::Move, false, Some("s2"));
        assert_eq!((act(&here, Some(1)), act(&here, None)), (RowAction::Place, RowAction::Start));
        // On screen 1 of a workspace for screen 2: neither placed nor started.
        assert_eq!((act(&there, Some(1)), act(&there, None)), (RowAction::Leave, RowAction::Leave));
        assert!(keeps_its_app(&there, Some("s2")) && !keeps_its_app(&here, Some("s2")));
        // For all screens, every row takes part, and an off row is just off.
        assert_eq!(row_action(&there, None, OpenWindows::Move, false, None), RowAction::Start);
        let off = Rule { off: true, ..row("s1") };
        assert!(keeps_its_app(&off, None) && keeps_its_app(&off, Some("s2")));
    }
}
