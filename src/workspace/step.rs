//! Stepping through a desk's workspaces, and which screens a switch changed. Both are
//! pure, so a later per-screen scope can hand them a filtered list.

use super::Workspace;

/// Which way a step goes through the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
}

impl Step {
    pub fn parse(s: &str) -> Option<Step> {
        match s {
            "next" => Some(Step::Next),
            "previous" => Some(Step::Previous),
            _ => None,
        }
    }
}

/// The index in `list` a step from `current` lands on. With none in use, Next is the first
/// and Previous the last. `wrap` goes round at the ends; without it a step off the end is
/// `None`. `None` too when the list is empty, or its only workspace is the one in use. An id
/// that isn't in the list counts as none in use.
pub fn stepped(list: &[Workspace], current: Option<&str>, dir: Step, wrap: bool) -> Option<usize> {
    let last = list.len().checked_sub(1)?;
    let here = current.and_then(|id| list.iter().position(|w| w.id == id));
    match (here, dir) {
        (None, Step::Next) => Some(0),
        (None, Step::Previous) => Some(last),
        (Some(_), _) if last == 0 => None,
        (Some(i), Step::Next) if i < last => Some(i + 1),
        (Some(_), Step::Next) => wrap.then_some(0),
        (Some(i), Step::Previous) if i > 0 => Some(i - 1),
        (Some(_), Step::Previous) => wrap.then_some(last),
    }
}

/// The screens a switch to `ws` changes, as indexes into `screens` (their ids): those with
/// an on row of the workspace, or a layout of its own. A screen it only keeps isn't one.
pub fn screens_changed(ws: &Workspace, screens: &[&str]) -> Vec<usize> {
    let changes = |id: &str| ws.windows.iter().any(|r| !r.off && r.monitor == id) || ws.zones.contains_key(id);
    screens.iter().enumerate().filter(|(_, id)| changes(id)).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Rule;
    use serde_json::json;

    fn list(ids: &[&str]) -> Vec<Workspace> {
        ids.iter().map(|id| Workspace { id: (*id).into(), ..Workspace::default() }).collect()
    }

    #[test]
    fn next_goes_on_and_wraps() {
        let l = list(&["a", "b", "c"]);
        assert_eq!(stepped(&l, Some("a"), Step::Next, true), Some(1));
        assert_eq!(stepped(&l, Some("b"), Step::Next, true), Some(2));
        assert_eq!(stepped(&l, Some("c"), Step::Next, true), Some(0));
        assert_eq!(stepped(&l, Some("c"), Step::Next, false), None);
    }

    #[test]
    fn previous_goes_back_and_wraps() {
        let l = list(&["a", "b", "c"]);
        assert_eq!(stepped(&l, Some("c"), Step::Previous, true), Some(1));
        assert_eq!(stepped(&l, Some("a"), Step::Previous, true), Some(2));
        assert_eq!(stepped(&l, Some("a"), Step::Previous, false), None);
    }

    #[test]
    fn none_in_use_starts_at_the_ends() {
        let l = list(&["a", "b", "c"]);
        assert_eq!(stepped(&l, None, Step::Next, true), Some(0));
        assert_eq!(stepped(&l, None, Step::Previous, true), Some(2));
    }

    #[test]
    fn one_workspace_has_no_other_to_step_to() {
        let l = list(&["a"]);
        assert_eq!(stepped(&l, Some("a"), Step::Next, true), None);
        assert_eq!(stepped(&l, Some("a"), Step::Previous, true), None);
        assert_eq!(stepped(&l, None, Step::Next, true), Some(0));
        assert_eq!(stepped(&[], None, Step::Next, true), None);
    }

    #[test]
    fn an_unknown_current_counts_as_none() {
        let l = list(&["a", "b", "c"]);
        assert_eq!(stepped(&l, Some("gone"), Step::Next, true), Some(0));
        assert_eq!(stepped(&l, Some("gone"), Step::Previous, true), Some(2));
    }

    #[test]
    fn a_filtered_list_steps_within_itself() {
        let all = list(&["a", "b", "c", "d"]);
        let some: Vec<Workspace> = all.into_iter().filter(|w| w.id != "b" && w.id != "c").collect();
        assert_eq!(stepped(&some, Some("a"), Step::Next, true), Some(1));
        assert_eq!(some[1].id, "d");
        assert_eq!(stepped(&some, Some("d"), Step::Next, true), Some(0));
    }

    fn row(monitor: &str, off: bool) -> Rule {
        Rule { monitor: monitor.into(), off, ..Rule::default() }
    }

    #[test]
    fn screens_changed_counts_rows_and_layouts_but_not_off_rows() {
        let mut ws = Workspace { windows: vec![row("m1", false), row("m2", true)], ..Workspace::default() };
        ws.zones.insert("m3".into(), json!({ "template": "none", "tree": "leaf" }));
        assert_eq!(screens_changed(&ws, &["m1", "m2", "m3", "m4"]), vec![0, 2]);
    }

    #[test]
    fn screens_changed_is_empty_when_no_screen_matches() {
        let ws = Workspace { windows: vec![row("gone", false)], ..Workspace::default() };
        assert!(screens_changed(&ws, &["m1", "m2"]).is_empty());
    }
}
