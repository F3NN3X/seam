//! Workspaces for one screen (brief J): which workspace is in use on each screen, what a
//! switch changes of that, which windows a switch of one screen leaves alone, and where a
//! row added to such a workspace goes. Pure, so the tray, the API and the tests agree.

use super::{Rule, Target, WinState, Workspace};
use crate::config::Desk;
use crate::zones::Rect;
use std::collections::{BTreeMap, HashSet};

/// What a switch to a one-screen workspace whose screen isn't connected says (D787).
pub const SCOPE_GONE: &str = "This workspace's screen isn't connected";

/// The workspace in use on `screen`: its own one-screen workspace, else the desk-wide one (D782).
pub fn in_use_on<'a>(desk: &'a Desk, screen: &str) -> Option<&'a str> {
    let own = desk.screen_in_use.get(screen).and_then(|s| s.workspace.as_deref());
    own.or(desk.active_workspace.as_deref())
}

/// Each of `screens` with the workspace in use on it, for those that have one.
pub fn holds(desk: &Desk, screens: &[&str]) -> BTreeMap<String, String> {
    let held = |s: &&str| in_use_on(desk, s).map(|id| (s.to_string(), id.to_string()));
    screens.iter().filter_map(held).collect()
}

/// The screens of `screens` that have a one-screen workspace of their own in use, which a
/// desk-wide workspace no longer holds.
pub fn own_screens(desk: &Desk, screens: &[&str]) -> Vec<String> {
    let own = |s: &&&str| desk.screen_in_use.get(**s).is_some_and(|e| e.workspace.is_some());
    screens.iter().filter(own).map(|s| s.to_string()).collect()
}

/// The screens of `screens` a switch or a close of `ws` leaves alone (`reach` says which it
/// takes), each with the other workspace in use there, whose windows there it holds (D783).
pub fn holders_of(desk: &Desk, screens: &[&str], ws: &str, reach: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
    let holder = |s: &&str| in_use_on(desk, s).filter(|id| *id != ws).map(|id| (s.to_string(), id.to_string()));
    screens.iter().filter(|s| !reach(s)).filter_map(holder).collect()
}

/// The screens a close or minimize of `ws` reaches (D786): the ones where it is in use, or while
/// it is in use nowhere, its screen, or every screen.
pub fn close_reach(desk: &Desk, screens: &[&str], ws: &Workspace) -> Vec<String> {
    let mine: Vec<String> = holds(desk, screens).into_iter().filter(|(_, id)| *id == ws.id).map(|(s, _)| s).collect();
    if !mine.is_empty() {
        return screens.iter().filter(|s| mine.iter().any(|m| m == *s)).map(|s| s.to_string()).collect();
    }
    let own = |s: &&&str| ws.scope.as_deref().is_none_or(|own| own == **s);
    screens.iter().filter(own).map(|s| s.to_string()).collect()
}

/// The rows as matching sees them: a row the switch, the status or the close doesn't reach (on
/// another screen than `scope`, or on one of `away`) sits out like an off row, so it takes no
/// window from a row that is reached (D788).
pub fn sit_out(rules: &[Rule], scope: Option<&str>, away: &[String]) -> Vec<Rule> {
    let reached = |r: &Rule| in_scope(r, scope) && !away.contains(&r.monitor);
    rules.iter().map(|r| if reached(r) { r.clone() } else { Rule { off: true, ..r.clone() } }).collect()
}

/// What in use means after a switch to `id`, scoped to `scope`, ended on a desk of `screens`.
/// A one-screen switch changes its screen only (D783). An All-screens one takes every screen
/// and the desk-wide slots, as before one-screen workspaces (D784). Either way a screen keeps
/// what it had before as its previous, unless that is the same workspace.
pub fn after_switch(desk: &mut Desk, id: &str, scope: Option<&str>, screens: &[&str]) {
    match scope {
        Some(screen) => {
            let before = in_use_on(desk, screen).map(str::to_string);
            let entry = desk.screen_in_use.entry(screen.to_string()).or_default();
            if before.as_deref() != Some(id) {
                entry.previous = before.or(entry.previous.take());
            }
            entry.workspace = Some(id.to_string());
        }
        None => {
            let mut all: Vec<String> = screens.iter().map(|s| s.to_string()).collect();
            all.extend(desk.screen_in_use.keys().filter(|k| !screens.contains(&k.as_str())).cloned());
            for screen in all {
                let before = in_use_on(desk, &screen).map(str::to_string);
                let entry = desk.screen_in_use.entry(screen).or_default();
                if before.as_deref() != Some(id) && before.is_some() {
                    entry.previous = before;
                }
                entry.workspace = None;
            }
            if desk.active_workspace.as_deref() != Some(id) {
                // Only a change to another workspace leaves one behind; losing it doesn't.
                if desk.active_workspace.is_some() {
                    desk.previous_workspace = desk.active_workspace.take();
                }
                desk.active_workspace = Some(id.to_string());
            }
        }
    }
    tidy(desk);
}

/// A workspace's scope changed from `old` to `new` while it may be in use. All screens to
/// one: it becomes that screen's own, unless the screen has its own, and the desk-wide slot
/// is cleared. One screen to another: its entry moves, and the old screen falls back to the
/// desk-wide one. One screen to All screens: its screen falls back to the desk-wide one.
pub fn rescope(desk: &mut Desk, id: &str, old: Option<&str>, new: Option<&str>) {
    if old == new {
        return;
    }
    let mine = |desk: &Desk, s: &str| desk.screen_in_use.get(s).and_then(|e| e.workspace.as_deref()) == Some(id);
    let in_use = match old {
        None => desk.active_workspace.as_deref() == Some(id),
        Some(s) => mine(desk, s),
    };
    if desk.active_workspace.as_deref() == Some(id) {
        desk.active_workspace = None;
    }
    for entry in desk.screen_in_use.values_mut().filter(|e| e.workspace.as_deref() == Some(id)) {
        entry.workspace = None;
    }
    if let (true, Some(screen)) = (in_use, new) {
        let own = desk.screen_in_use.get(screen).is_some_and(|e| e.workspace.is_some());
        if old.is_some() || !own {
            after_switch(desk, id, Some(screen), &[]);
        }
    }
    tidy(desk);
}

/// A deleted workspace is in use, before or the default nowhere: only ids in `ids` stay, on
/// the desk and on every screen. Returns whether anything changed.
pub fn forget(desk: &mut Desk, ids: &[String]) -> bool {
    let before = desk.clone();
    let exists = |id: &String| ids.contains(id);
    for entry in desk.screen_in_use.values_mut() {
        entry.workspace = entry.workspace.take().filter(exists);
        entry.previous = entry.previous.take().filter(exists);
    }
    for slot in [&mut desk.active_workspace, &mut desk.previous_workspace, &mut desk.default_workspace] {
        *slot = slot.take().filter(exists);
    }
    tidy(desk);
    *desk != before
}

/// At start, keep what still holds (D790): `forget`, and a screen's own workspace only while
/// it is still for that screen. Returns whether anything changed.
pub fn keep_valid(desk: &mut Desk, list: &[Workspace]) -> bool {
    let before = desk.clone();
    for (screen, entry) in desk.screen_in_use.iter_mut() {
        let own = |id: &String| list.iter().any(|w| w.id == *id && w.scope.as_deref() == Some(screen.as_str()));
        entry.workspace = entry.workspace.take().filter(own);
    }
    forget(desk, &list.iter().map(|w| w.id.clone()).collect::<Vec<_>>());
    *desk != before
}

/// An entry with no workspace of its own and no previous, or the same previous as the desk's,
/// says nothing, so a desk that never uses one-screen workspaces never writes the field (D782).
fn tidy(desk: &mut Desk) {
    let previous = desk.previous_workspace.clone();
    desk.screen_in_use.retain(|_, e| e.workspace.is_some() || (e.previous.is_some() && e.previous != previous));
}

/// Switch back on `screen`: what was in use there before, else the desk's previous (D785).
pub fn switch_back_target<'a>(desk: &'a Desk, screen: &str) -> Option<&'a str> {
    let own = desk.screen_in_use.get(screen).and_then(|e| e.previous.as_deref());
    own.or(desk.previous_workspace.as_deref())
}

/// The workspaces Next and Previous step through on `screen`: its own and the All-screens
/// ones, in list order (D785). One for another screen, or for a screen that isn't connected,
/// isn't among them.
pub fn cycle(list: &[Workspace], screen: &str) -> Vec<Workspace> {
    list.iter().filter(|w| w.scope.as_deref().is_none_or(|s| s == screen)).cloned().collect()
}

/// The row is on the workspace's screen, or the workspace is for all screens (D788).
pub fn in_scope(rule: &Rule, scope: Option<&str>) -> bool {
    scope.is_none_or(|s| rule.monitor == s)
}

/// The workspace is for all screens, or its screen is among `connected` (D787).
pub fn scope_connected(ws: &Workspace, connected: &[&str]) -> bool {
    ws.scope.as_deref().is_none_or(|s| connected.contains(&s))
}

/// A row added to a one-screen workspace, on its screen (D788): the monitor becomes `screen`;
/// a rect's fractions and a state are kept; a zone the screen doesn't have (it has
/// `zone_count`) becomes full screen.
pub fn retarget(rule: &Rule, screen: &str, zone_count: usize) -> Rule {
    let mut moved = Rule { monitor: screen.to_string(), ..rule.clone() };
    let fits = |z: u32| (1..=zone_count).contains(&(z as usize));
    if let Target::Zone { zone, span_to } = rule.target
        && !(fits(zone) && span_to.is_none_or(fits))
    {
        moved.set_target(Target::State { state: WinState::Max });
    }
    moved
}

/// The windows a switch leaves alone because another screen's workspace holds them (D783):
/// each stamped by the workspace in use on the screen its middle is on. `windows` are each
/// window's handle, `Seam_ws` key and middle; `holders` are the screens the switch doesn't
/// take, with the key of the workspace in use there.
pub fn held(windows: &[(isize, Option<u32>, (i32, i32))], holders: &[(Rect, u32)]) -> HashSet<isize> {
    let holder = |(x, y): (i32, i32)| holders.iter().find(|(r, _)| r.contains(x, y)).map(|(_, key)| *key);
    let held = |(_, key, at): &&(isize, Option<u32>, (i32, i32))| key.is_some() && *key == holder(*at);
    windows.iter().filter(held).map(|(h, ..)| *h).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScreenInUse;

    fn desk(active: Option<&str>, previous: Option<&str>) -> Desk {
        Desk {
            active_workspace: active.map(Into::into),
            previous_workspace: previous.map(Into::into),
            ..Desk::default()
        }
    }

    fn own(workspace: Option<&str>, previous: Option<&str>) -> ScreenInUse {
        ScreenInUse { workspace: workspace.map(Into::into), previous: previous.map(Into::into) }
    }

    fn ws(id: &str, scope: Option<&str>) -> Workspace {
        Workspace { id: id.into(), scope: scope.map(Into::into), ..Workspace::default() }
    }

    #[test]
    fn in_use_falls_back_to_the_desk_wide_one() {
        let mut d = desk(Some("a"), None);
        assert_eq!(in_use_on(&d, "s1"), Some("a"));
        d.screen_in_use.insert("s2".into(), own(Some("y"), Some("a")));
        assert_eq!(in_use_on(&d, "s1"), Some("a"));
        assert_eq!(in_use_on(&d, "s2"), Some("y"));
        // An entry with only a previous leaves the desk-wide one in use there.
        d.screen_in_use.insert("s3".into(), own(None, Some("y")));
        assert_eq!(in_use_on(&d, "s3"), Some("a"));
        assert_eq!(holds(&d, &["s1", "s2"]), BTreeMap::from([("s1".into(), "a".into()), ("s2".into(), "y".into())]));
        assert_eq!(own_screens(&d, &["s1", "s2", "s3"]), ["s2"]);
        assert!(holds(&desk(None, None), &["s1"]).is_empty());
    }

    #[test]
    fn one_screen_switch_changes_only_its_screen() {
        let mut d = desk(Some("a"), Some("z"));
        after_switch(&mut d, "y", Some("s2"), &["s1", "s2"]);
        assert_eq!((in_use_on(&d, "s1"), in_use_on(&d, "s2")), (Some("a"), Some("y")));
        // The desk-wide slots stay as they were.
        assert_eq!((d.active_workspace.as_deref(), d.previous_workspace.as_deref()), (Some("a"), Some("z")));
        assert_eq!(d.screen_in_use["s2"], own(Some("y"), Some("a")));
        assert!(!d.screen_in_use.contains_key("s1"));
        // Another one on the same screen remembers the first; the same one again changes nothing.
        after_switch(&mut d, "x", Some("s2"), &["s1", "s2"]);
        assert_eq!(d.screen_in_use["s2"], own(Some("x"), Some("y")));
        after_switch(&mut d, "x", Some("s2"), &["s1", "s2"]);
        assert_eq!(d.screen_in_use["s2"], own(Some("x"), Some("y")));
    }

    #[test]
    fn all_screens_switch_clears_every_screen() {
        let mut d = desk(Some("a"), Some("z"));
        after_switch(&mut d, "y", Some("s2"), &["s1", "s2"]);
        after_switch(&mut d, "b", None, &["s1", "s2"]);
        assert_eq!((in_use_on(&d, "s1"), in_use_on(&d, "s2")), (Some("b"), Some("b")));
        assert_eq!((d.active_workspace.as_deref(), d.previous_workspace.as_deref()), (Some("b"), Some("a")));
        // Screen 2 had its own, so it goes back there; screen 1 had the desk's.
        assert_eq!(d.screen_in_use["s2"], own(None, Some("y")));
        assert!(!d.screen_in_use.contains_key("s1"));
        // With nothing one-screen in use it is exactly the old switch.
        let mut plain = desk(Some("a"), None);
        after_switch(&mut plain, "b", None, &["s1", "s2"]);
        assert_eq!(plain, desk(Some("b"), Some("a")));
    }

    #[test]
    fn redundant_entries_are_dropped() {
        let mut d = desk(Some("a"), Some("z"));
        d.screen_in_use.insert("s1".into(), own(None, Some("z")));
        d.screen_in_use.insert("s2".into(), own(None, Some("q")));
        after_switch(&mut d, "a", None, &["s1", "s2"]);
        // s1 says what the desk says, so it goes; s2's own history stays.
        assert_eq!(d.screen_in_use.keys().collect::<Vec<_>>(), ["s2"]);
        // A deleted workspace's slots go, and a screen's own only while it is still for that screen.
        let mut d = desk(Some("a"), Some("gone"));
        d.screen_in_use.insert("s1".into(), own(Some("y"), Some("a")));
        d.screen_in_use.insert("s2".into(), own(Some("x"), None));
        let list = [ws("a", None), ws("y", Some("s1")), ws("x", Some("s1"))];
        assert!(keep_valid(&mut d, &list));
        assert_eq!(d.previous_workspace, None);
        assert_eq!(d.screen_in_use["s1"], own(Some("y"), Some("a")));
        assert!(!d.screen_in_use.contains_key("s2"));
        assert!(!keep_valid(&mut d, &list));
    }

    #[test]
    fn switch_back_on_a_screen_follows_its_history() {
        let mut d = desk(Some("a"), Some("z"));
        after_switch(&mut d, "y", Some("s2"), &["s1", "s2"]);
        assert_eq!(switch_back_target(&d, "s2"), Some("a"));
        assert_eq!(switch_back_target(&d, "s1"), Some("z"));
        // Back to a on the desk: screen 2 goes back to y next time, screen 1 to the desk's previous.
        after_switch(&mut d, "a", None, &["s1", "s2"]);
        assert_eq!(switch_back_target(&d, "s2"), Some("y"));
        assert_eq!(switch_back_target(&d, "s1"), Some("z"));
    }

    #[test]
    fn scope_changed_while_in_use_moves_what_is_in_use() {
        // All screens to screen 2: it is screen 2's own, the desk-wide slot is cleared.
        let mut d = desk(Some("a"), Some("z"));
        rescope(&mut d, "a", None, Some("s2"));
        assert_eq!((d.active_workspace.as_deref(), in_use_on(&d, "s2")), (None, Some("a")));
        assert_eq!(d.previous_workspace.as_deref(), Some("z"));
        // Screen 2 to screen 1: the entry moves, and screen 2 falls back.
        let mut d = desk(Some("b"), None);
        after_switch(&mut d, "y", Some("s2"), &["s1", "s2"]);
        rescope(&mut d, "y", Some("s2"), Some("s1"));
        assert_eq!((in_use_on(&d, "s1"), in_use_on(&d, "s2")), (Some("y"), Some("b")));
        // Back to All screens: its screen falls back to the desk-wide one.
        rescope(&mut d, "y", Some("s1"), None);
        assert_eq!(in_use_on(&d, "s1"), Some("b"));
    }

    #[test]
    fn cycle_has_the_screens_and_the_all_screens_ones_in_list_order() {
        let list = [ws("a", None), ws("y", Some("s2")), ws("x", Some("s1")), ws("b", None)];
        let ids = |l: Vec<Workspace>| l.into_iter().map(|w| w.id).collect::<Vec<_>>();
        assert_eq!(ids(cycle(&list, "s2")), ["a", "y", "b"]);
        assert_eq!(ids(cycle(&list, "s1")), ["a", "x", "b"]);
    }

    #[test]
    fn cycle_skips_unconnected() {
        let list = [ws("a", None), ws("gone", Some("unplugged")), ws("y", Some("s2"))];
        let ids: Vec<String> = cycle(&list, "s2").into_iter().map(|w| w.id).collect();
        assert_eq!(ids, ["a", "y"]);
        assert!(!scope_connected(&list[1], &["s1", "s2"]));
        assert!(scope_connected(&list[0], &[]) && scope_connected(&list[2], &["s2"]));
    }

    #[test]
    fn retarget_keeps_rects_and_zones_it_can_and_maximizes_the_rest() {
        let on = |target: Target| Rule { monitor: "s1".into(), target, ..Rule::default() };
        let rect = on(Target::Rect { rect: [0.1, 0.2, 0.3, 0.4] });
        assert_eq!(retarget(&rect, "s2", 2), Rule { monitor: "s2".into(), ..rect.clone() });
        let keep = on(Target::State { state: WinState::Keep });
        assert_eq!(retarget(&keep, "s2", 0).target, keep.target);
        let zone = on(Target::Zone { zone: 2, span_to: Some(3) });
        assert_eq!(retarget(&zone, "s2", 3).target, zone.target);
        // Screen 2 has two zones: zone 3 isn't there, so the window fills the screen.
        let full = retarget(&zone, "s2", 2);
        assert_eq!((full.monitor.as_str(), &full.target), ("s2", &Target::State { state: WinState::Max }));
        assert_eq!(full.shown, crate::workspace::Shown::Maximized);
        assert!(in_scope(&full, Some("s2")) && !in_scope(&zone, Some("s2")) && in_scope(&zone, None));
    }

    #[test]
    fn with_no_one_screen_workspace_a_close_holds_nothing_back() {
        let screens = ["s1", "s2"];
        let d = desk(Some("x"), None);
        for ws in [ws("x", None), ws("y", None)] {
            let reach = close_reach(&d, &screens, &ws);
            assert_eq!(reach, ["s1", "s2"], "{}", ws.id);
            assert!(holders_of(&d, &screens, &ws.id, &|s| reach.iter().any(|r| r == s)).is_empty(), "{}", ws.id);
        }
        // With y in use on screen 2, x closes on screen 1 only, and y holds screen 2.
        let mut d = d;
        after_switch(&mut d, "y", Some("s2"), &screens);
        let reach = close_reach(&d, &screens, &ws("x", None));
        assert_eq!(reach, ["s1"]);
        let held = holders_of(&d, &screens, "x", &|s| reach.iter().any(|r| r == s));
        assert_eq!(held, [("s2".to_string(), "y".to_string())]);
        // A workspace in use nowhere reaches its screen, or every screen.
        assert_eq!(close_reach(&d, &screens, &ws("z", Some("s2"))), ["s2"]);
        assert_eq!(close_reach(&d, &screens, &ws("z", None)), ["s1", "s2"]);
    }

    #[test]
    fn a_row_on_another_screen_takes_no_window_from_one_that_is_reached() {
        use crate::workspace::{Candidate, assign_bound};
        let chrome = |monitor: &str| Rule { exe: r"C:\chrome.exe".into(), monitor: monitor.into(), ..Rule::default() };
        // The row kept from before the scope was set comes first.
        let rows = [chrome("s1"), chrome("s2")];
        let open = Candidate {
            hwnd: 7,
            exe: r"C:\chrome.exe".into(),
            aumid: None,
            class: String::new(),
            title: String::new(),
            z: 0,
        };
        let found =
            assign_bound(&sit_out(&rows, Some("s2"), &[]), std::slice::from_ref(&open), &[None, None], &HashSet::new());
        assert_eq!(found, [None, Some(7)]);
        // The same for a desk-wide Snap back that leaves screen 1 to its own workspace.
        let found = assign_bound(&sit_out(&rows, None, &["s1".into()]), &[open], &[None, None], &HashSet::new());
        assert_eq!(found, [None, Some(7)]);
        assert!(!sit_out(&rows, None, &[]).iter().any(|r| r.off));
    }

    #[test]
    fn a_one_screen_switch_never_claims_a_window_held_on_another_screen() {
        let s1 = Rect { x: 0, y: 0, w: 100, h: 100 };
        let (x, y) = (7, 9);
        // Screen 1 isn't switched and has x in use.
        let holders = [(s1, x)];
        let windows = [
            (1, Some(x), (50, 50)),  // x's, on screen 1: held
            (2, Some(y), (50, 50)),  // y's, dragged to screen 1: not x's, so not held
            (3, None, (50, 50)),     // nobody's: may be pulled in
            (4, Some(x), (150, 50)), // x's, but on the screen being switched
        ];
        assert_eq!(held(&windows, &holders), HashSet::from([1]));
        assert!(held(&windows, &[]).is_empty());
    }
}
