//! What the capture review and Update from screen ask before they save: which windows
//! start ticked, and which rows have no app open.

use super::capture::{app_open, as_candidates, pair};
use super::*;
use crate::classify::brief_window;

/// For each captured window, whether the review starts it ticked and, when it doesn't,
/// why. `elevated` says which windows run as administrator. A window with the same app
/// and title as one above it starts unticked.
pub fn ticks(rules: &[Rule], elevated: &[bool]) -> Vec<(bool, Option<&'static str>)> {
    let same =
        |a: &Rule, b: &Rule| a.title_hint == b.title_hint && (a.exe.eq_ignore_ascii_case(&b.exe) && a.aumid == b.aumid);
    rules
        .iter()
        .enumerate()
        .map(|(i, rule)| {
            let why = if elevated.get(i).copied().unwrap_or(false) {
                Some("runs as administrator, so a switch can't move it")
            } else if brief_window(&rule.exe) {
                Some("usually open only for a moment")
            } else if rules[..i].iter().any(|above| same(above, rule)) {
                Some("same as the one above")
            } else {
                None
            };
            (why.is_none(), why)
        })
        .collect()
}

/// The rows of `old` whose app has no window open, as a capture of `fresh` finds it, in
/// row order. Empty when none of the workspace's apps is open, so Update from screen
/// never empties a workspace. `bound`, `elsewhere` and `others` as `merge_capture`.
pub fn not_open(
    old: &Workspace,
    fresh: &[Rule],
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
    others: &[Candidate],
) -> Vec<usize> {
    let found = pair(&old.windows, fresh, bound, elsewhere);
    let mut taken = vec![false; fresh.len()];
    for i in found.iter().flatten() {
        taken[*i as usize] = true;
    }
    let cands = as_candidates(fresh);
    let unpaired = || cands.iter().zip(&taken).filter(|(_, t)| !**t).map(|(c, _)| c);
    // A row whose own window was closed isn't kept by another window of its app, unless
    // that one is held by another workspace's row or is one the capture made no row of.
    let held: Vec<&Candidate> = unpaired().filter(|c| elsewhere.contains(&c.hwnd)).chain(others).collect();
    let open: Vec<&Candidate> = unpaired().chain(others).collect();
    let closed: Vec<usize> = old
        .windows
        .iter()
        .zip(&found)
        .enumerate()
        .filter(|(o, (row, hit))| {
            let wins = match bound.get(*o).copied().flatten() {
                Some(Bound::Gone) => &held,
                // Its own window is open, though the capture took none of it.
                Some(Bound::Alive(_)) => return false,
                _ => &open,
            };
            hit.is_none() && !app_open(row, wins)
        })
        .map(|(o, _)| o)
        .collect();
    // A capture that finds nothing of the workspace would only empty it.
    if closed.len() == old.windows.len() { Vec::new() } else { closed }
}

/// What Update from screen did to the rows of apps that weren't open, for the balloon:
/// "Removed 2 apps that weren't open", "Turned off 1 app that wasn't open" or both.
pub fn closed_note(removed: usize, turned_off: usize) -> String {
    let apps = |n: usize| if n == 1 { "1 app that wasn't".to_string() } else { format!("{n} apps that weren't") };
    match (removed, turned_off) {
        (r, 0) => format!("Removed {} open", apps(r)),
        (0, t) => format!("Turned off {} open", apps(t)),
        (r, t) => format!("Turned off {t} and removed {r} of the apps that weren't open"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(exe: &str, title: &str) -> Rule {
        Rule { exe: exe.into(), title_hint: title.into(), monitor: "m".into(), ..Rule::default() }
    }

    #[test]
    fn elevated_and_brief_windows_start_unticked_with_a_reason() {
        let rules = [rule(r"C:\Windows\System32\Taskmgr.exe", "Task Manager"), rule("a.exe", "A"), rule("b.exe", "B")];
        let ticks = ticks(&rules, &[false, false, true]);
        assert_eq!(ticks[0], (false, Some("usually open only for a moment")));
        assert_eq!(ticks[1], (true, None));
        assert_eq!(ticks[2], (false, Some("runs as administrator, so a switch can't move it")));
    }

    #[test]
    fn a_second_window_with_the_same_title_starts_unticked() {
        let np = r"C:\Windows\notepad.exe";
        let rules = [rule(np, "Untitled"), rule(np, "Untitled"), rule(np, "notes.txt"), rule("other.exe", "Untitled")];
        let ticks = ticks(&rules, &[]);
        assert_eq!(ticks[0], (true, None));
        assert_eq!(ticks[1], (false, Some("same as the one above")));
        // A different title, or another app with the same title, isn't a copy.
        assert_eq!((ticks[2].0, ticks[3].0), (true, true));
        // Two Explorer windows on different folders are two windows.
        let explorer = r"C:\Windows\explorer.exe";
        let folders = [rule(explorer, "Docs"), rule(explorer, "Pictures")];
        assert!(ticks_of(&folders).iter().all(|(on, _)| *on));
    }

    fn ticks_of(rules: &[Rule]) -> Vec<(bool, Option<&'static str>)> {
        ticks(rules, &[])
    }

    #[test]
    fn the_balloon_says_what_changed() {
        assert_eq!(closed_note(1, 0), "Removed 1 app that wasn't open");
        assert_eq!(closed_note(2, 0), "Removed 2 apps that weren't open");
        assert_eq!(closed_note(0, 1), "Turned off 1 app that wasn't open");
        assert_eq!(closed_note(0, 3), "Turned off 3 apps that weren't open");
        assert_eq!(closed_note(2, 1), "Turned off 1 and removed 2 of the apps that weren't open");
    }

    #[test]
    fn everything_else_starts_ticked() {
        let rules = [rule("Code.exe", "main.rs"), rule("chrome.exe", "Docs"), rule("WindowsTerminal.exe", "pwsh")];
        assert!(ticks(&rules, &[false; 3]).iter().all(|t| *t == (true, None)));
    }
}
