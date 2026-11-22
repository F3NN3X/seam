//! A workspace's apps as the editor shows them: grouped by screen, and the windows of one app on
//! one screen merged into a row. It is a view of the saved list; nothing is saved in this shape.

use crate::icons;
use seam::state::Screen;
use seam::workspace::Rule;

/// One row of the editor: the places in `Workspace::windows` of the windows it stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRow {
    pub windows: Vec<usize>,
}

/// The rows on one screen, in start order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// The screen's device path, as the rules hold it.
    pub monitor: String,
    pub rows: Vec<AppRow>,
}

/// What a row merges on: the app and the screen, under the workspace's id.
pub fn split_key(workspace: &str, rule: &Rule) -> String {
    format!("{workspace}\u{1}{}\u{1}{}", rule.monitor, icons::identity(rule))
}

/// Group `windows` by screen: the connected screens in their order, then the others as they come.
/// Windows of one app on one screen are one row, unless `split` says that app is shown one by one.
pub fn group(windows: &[Rule], screens: &[Screen], split: impl Fn(&Rule) -> bool) -> Vec<Group> {
    let mut order: Vec<&Screen> = screens.iter().collect();
    order.sort_by_key(|s| s.number);
    let mut monitors: Vec<&str> = order.iter().map(|s| s.id.as_str()).collect();
    for rule in windows {
        if !monitors.contains(&rule.monitor.as_str()) {
            monitors.push(&rule.monitor);
        }
    }
    let mut groups = Vec::new();
    for monitor in monitors {
        let mut rows: Vec<(String, AppRow)> = Vec::new();
        for (i, rule) in windows.iter().enumerate().filter(|(_, r)| r.monitor == monitor) {
            // An item is a row of its own, whatever app it opens in (D731).
            let app = if rule.item.is_some() { format!("item {i}") } else { icons::identity(rule) };
            match rows.iter_mut().find(|row| row.0 == app).filter(|_| !split(rule) && !app.is_empty()) {
                Some((_, row)) => row.windows.push(i),
                None => rows.push((app, AppRow { windows: vec![i] })),
            }
        }
        if !rows.is_empty() {
            groups.push(Group { monitor: monitor.to_string(), rows: rows.into_iter().map(|(_, row)| row).collect() });
        }
    }
    groups
}

/// Move the windows at `moving` to just before the window at `before`, or after it, keeping their
/// order. `before` is a place in the list as it is now.
pub fn reorder(windows: &mut Vec<Rule>, moving: &[usize], before: usize, after: bool) {
    if before >= windows.len() || moving.contains(&before) {
        return;
    }
    // Where the target ends up once the moved windows are out of the way.
    let at = before - moving.iter().filter(|i| **i < before).count() + usize::from(after);
    let mut taken = Vec::new();
    for i in moving.iter().rev() {
        if *i < windows.len() {
            taken.push(windows.remove(*i));
        }
    }
    taken.reverse();
    let at = at.min(windows.len());
    windows.splice(at..at, taken);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(exe: &str, monitor: &str) -> Rule {
        Rule { exe: exe.into(), monitor: monitor.into(), ..Rule::default() }
    }

    fn screen(number: u32, id: &str) -> Screen {
        serde_json::from_value(serde_json::json!({ "number": number, "id": id, "name": id, "primary": false }))
            .expect("a screen")
    }

    fn exes(windows: &[Rule]) -> Vec<&str> {
        windows.iter().map(|r| r.exe.as_str()).collect()
    }

    #[test]
    fn windows_of_one_app_on_one_screen_are_one_row() {
        let windows = vec![rule("a.exe", "m1"), rule("b.exe", "m1"), rule("A.exe", "m1"), rule("a.exe", "m2")];
        let groups = group(&windows, &[screen(1, "m1"), screen(2, "m2")], |_| false);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows, vec![AppRow { windows: vec![0, 2] }, AppRow { windows: vec![1] }]);
        assert_eq!(groups[1].rows, vec![AppRow { windows: vec![3] }]);
    }

    #[test]
    fn items_are_never_merged_with_a_row_of_the_same_app() {
        let item =
            Rule { item: Some(seam::workspace::Item::Folder { folder: r"C:\p".into() }), ..rule("explorer.exe", "m1") };
        let windows = vec![item.clone(), rule("explorer.exe", "m1"), item, rule("explorer.exe", "m1")];
        let groups = group(&windows, &[screen(1, "m1")], |_| false);
        let rows: Vec<Vec<usize>> = groups[0].rows.iter().map(|r| r.windows.clone()).collect();
        assert_eq!(rows, [vec![0], vec![1, 3], vec![2]]);
    }

    #[test]
    fn a_split_app_is_a_row_for_each_window() {
        let windows = vec![rule("a.exe", "m1"), rule("a.exe", "m1")];
        let groups = group(&windows, &[screen(1, "m1")], |_| true);
        assert_eq!(groups[0].rows.len(), 2);
    }

    #[test]
    fn screens_come_in_order_and_unknown_ones_last() {
        let windows = vec![rule("a.exe", "gone"), rule("b.exe", "m2"), rule("c.exe", "m1")];
        let groups = group(&windows, &[screen(2, "m2"), screen(1, "m1")], |_| false);
        let order: Vec<&str> = groups.iter().map(|g| g.monitor.as_str()).collect();
        assert_eq!(order, ["m1", "m2", "gone"]);
    }

    #[test]
    fn reorder_puts_the_moved_windows_next_to_the_target() {
        let mut windows: Vec<Rule> = ["a", "b", "c", "d"].iter().map(|e| rule(e, "m")).collect();
        reorder(&mut windows, &[0], 2, true);
        assert_eq!(exes(&windows), ["b", "c", "a", "d"]);
        reorder(&mut windows, &[3], 0, false);
        assert_eq!(exes(&windows), ["d", "b", "c", "a"]);
        reorder(&mut windows, &[0, 2], 3, true);
        assert_eq!(exes(&windows), ["b", "a", "d", "c"]);
    }

    #[test]
    fn reorder_onto_itself_changes_nothing() {
        let mut windows: Vec<Rule> = ["a", "b"].iter().map(|e| rule(e, "m")).collect();
        reorder(&mut windows, &[0, 1], 1, true);
        assert_eq!(exes(&windows), ["a", "b"]);
    }
}
