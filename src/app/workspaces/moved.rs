//! The parts of "Move window to next workspace" that decide what is saved and what Undo
//! puts back, kept apart from the app so they can be tested.

use super::*;

/// The workspace a move leaves, without the row that holds this window's app, and which
/// row that was. `None` when it has no row for the window, so a move only adds.
pub(super) fn without_row(
    old: &Workspace,
    fresh: Rule,
    bound: &[Option<workspace::Bound>],
) -> Option<(Workspace, usize)> {
    let found = workspace::add_window(old, fresh, bound);
    if found.new {
        return None;
    }
    let without = workspace::edited(old, &workspace::Edit::RemoveRow(found.row)).ok()?;
    Some((without, found.row))
}

/// The workspace a move leaves: the one a step still running is heading to, else the one in
/// use, the same origin the target is counted from.
pub(super) fn origin(in_use: Option<&str>, step_to: Option<&str>) -> Option<String> {
    step_to.or(in_use).map(str::to_string)
}

/// What a move saves: the window added to its workspace, and when the workspace it
/// leaves has a row for it, that workspace without the row.
pub(super) struct Plan {
    pub added: workspace::Added,
    pub out: Option<Out>,
}

/// The workspace a move leaves, without the window's row.
pub(super) struct Out {
    pub without: Workspace,
    /// For each row it has now, the row it was, for Undo.
    pub from: Vec<Option<usize>>,
}

/// Work out a move. `source` is the workspace it leaves with its bindings, none for a plain add.
pub(super) fn plan(
    target: &Workspace,
    target_bound: &[Option<workspace::Bound>],
    rule: Rule,
    source: Option<(&Workspace, &[Option<workspace::Bound>])>,
) -> Plan {
    let out = source.and_then(|(old, bound)| {
        let (without, row) = without_row(old, rule.clone(), bound)?;
        Some(Out { without, from: workspace::survivors(old.windows.len(), row) })
    });
    Plan { added: workspace::add_window(target, rule, target_bound), out }
}

/// What Undo needs to put the workspace a move left back.
pub(super) fn record(id: &str, before: Value, after: Value, from: Vec<Option<usize>>) -> MovedFrom {
    MovedFrom { id: id.to_string(), before, after, from }
}

/// The desk's workspace list with an Undo applied, or `None` to leave it as it is. A
/// workspace the save wrote (`after`) must still be exactly that, and a move's other
/// workspace too: the move is undone whole or not at all. With no `before` the workspace
/// the save created goes.
pub(super) fn undone(
    list: &[Value],
    id: &str,
    after: &Value,
    before: Option<&Value>,
    source: Option<&MovedFrom>,
) -> Option<Vec<Value>> {
    let same = |v: &Value| workspace::parse(v).is_some_and(|w| w.id == id);
    if !list.iter().any(|v| same(v) && v == after) {
        return None;
    }
    if let Some(s) = source
        && !list.contains(&s.after)
    {
        return None;
    }
    let back = list
        .iter()
        .filter_map(|v| match (v, source) {
            (v, Some(s)) if *v == s.after => Some(s.before.clone()),
            (v, _) if same(v) => before.cloned(),
            (v, _) => Some(v.clone()),
        })
        .collect();
    Some(back)
}

#[cfg(test)]
mod tests {
    use super::*;
    use seam::workspace::Bound;

    fn rule(exe: &str) -> Rule {
        Rule { exe: exe.into(), class: "C".into(), monitor: "m".into(), launch: true, ..Rule::default() }
    }

    fn ws(id: &str, exes: &[&str]) -> Workspace {
        Workspace {
            id: id.into(),
            name: id.into(),
            windows: exes.iter().map(|e| rule(e)).collect(),
            ..Workspace::default()
        }
    }

    #[test]
    fn a_move_takes_the_windows_row_out_of_the_workspace_it_leaves() {
        let old = ws("a", &[r"C:\one.exe", r"C:\notepad.exe", r"C:\three.exe"]);
        let (without, row) = without_row(&old, rule(r"C:\notepad.exe"), &[None, None, None]).unwrap();
        assert_eq!(row, 1);
        let left: Vec<&str> = without.windows.iter().map(|r| r.exe.as_str()).collect();
        assert_eq!(left, [r"C:\one.exe", r"C:\three.exe"]);
        // The rows after it follow, as the editor's own removal says.
        assert_eq!(workspace::survivors(old.windows.len(), row), [Some(0), Some(2)]);
    }

    #[test]
    fn a_move_leaves_a_workspace_with_no_row_for_the_window_alone() {
        let old = ws("a", &[r"C:\one.exe"]);
        assert!(without_row(&old, rule(r"C:\notepad.exe"), &[None]).is_none());
    }

    #[test]
    fn a_row_held_by_another_window_stays() {
        let old = ws("a", &[r"C:\notepad.exe"]);
        // The only row of its app is bound to a different window, and this one is on another screen.
        let fresh = Rule { monitor: "other".into(), ..rule(r"C:\notepad.exe") };
        assert!(without_row(&old, fresh, &[Some(Bound::Alive(7))]).is_none());
    }

    #[test]
    fn a_move_plans_the_row_into_one_workspace_and_out_of_the_other() {
        let (to, from) = (ws("b", &[r"C:\y.exe"]), ws("a", &[r"C:\notepad.exe", r"C:\x.exe"]));
        let p = plan(&to, &[None], rule(r"C:\notepad.exe"), Some((&from, &[None, None])));
        // The target gets a row at the end, and the source loses its row.
        assert!(p.added.new && p.added.workspace.windows.len() == 2);
        let out = p.out.expect("the source has a row for it");
        assert_eq!(out.without.windows.len(), 1);
        assert_eq!(out.without.windows[0].exe, r"C:\x.exe");
        assert_eq!(out.from, [Some(1)]);
        // What is recorded for Undo restores the source.
        let rec = record("a", workspace::to_value(&from), workspace::to_value(&out.without), out.from);
        assert_eq!((rec.id.as_str(), rec.before), ("a", workspace::to_value(&from)));
    }

    #[test]
    fn a_move_leaves_the_workspace_a_step_is_heading_to() {
        assert_eq!(origin(Some("a"), None).as_deref(), Some("a"));
        assert_eq!(origin(Some("a"), Some("b")).as_deref(), Some("b"));
        assert_eq!(origin(None, None), None);
    }

    #[test]
    fn a_plain_add_or_a_source_without_the_row_plans_nothing_to_remove() {
        let (to, from) = (ws("b", &[]), ws("a", &[r"C:\x.exe"]));
        assert!(plan(&to, &[], rule(r"C:\notepad.exe"), None).out.is_none());
        assert!(plan(&to, &[], rule(r"C:\notepad.exe"), Some((&from, &[None]))).out.is_none());
    }

    fn value(id: &str, exes: &[&str]) -> Value {
        workspace::to_value(&ws(id, exes))
    }

    /// A move of notepad from `a` to `b`: what the desk holds after, and what Undo records.
    fn moved() -> (Vec<Value>, Value, Value, MovedFrom) {
        let (a_before, b_before) = (value("a", &["n", "x"]), value("b", &["y"]));
        let (a_after, b_after) = (value("a", &["x"]), value("b", &["y", "n"]));
        let source =
            MovedFrom { id: "a".into(), before: a_before, after: a_after.clone(), from: workspace::survivors(2, 0) };
        (vec![a_after, b_after.clone(), value("c", &[])], b_after, b_before, source)
    }

    #[test]
    fn undo_puts_both_workspaces_back() {
        let (list, after, before, source) = moved();
        let back = undone(&list, "b", &after, Some(&before), Some(&source)).unwrap();
        assert_eq!(back, [source.before.clone(), before, value("c", &[])]);
    }

    #[test]
    fn undo_is_refused_when_the_target_changed() {
        let (mut list, after, before, source) = moved();
        list[1] = value("b", &["y", "n", "z"]);
        assert!(undone(&list, "b", &after, Some(&before), Some(&source)).is_none());
    }

    #[test]
    fn undo_is_refused_when_the_source_changed_so_no_row_is_lost() {
        let (mut list, after, before, source) = moved();
        list[0] = value("a", &["x", "w"]);
        assert!(undone(&list, "b", &after, Some(&before), Some(&source)).is_none());
    }

    #[test]
    fn undo_is_refused_when_the_source_was_deleted() {
        let (mut list, after, before, source) = moved();
        list.remove(0);
        assert!(undone(&list, "b", &after, Some(&before), Some(&source)).is_none());
    }

    #[test]
    fn undo_of_a_plain_add_needs_no_source_and_a_created_workspace_goes() {
        let (list, after, before, _) = moved();
        let back = undone(&list, "b", &after, Some(&before), None).unwrap();
        assert_eq!(back[1], before);
        let gone = undone(&list, "b", &after, None, None).unwrap();
        assert_eq!(gone.len(), 2);
    }
}
