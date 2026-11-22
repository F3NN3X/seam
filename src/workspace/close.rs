//! Which windows "Close this workspace" and "Minimize this workspace" take (D701, D702).
//! Pure, so the tray's worker and the tests agree.

use super::Rule;
use crate::winutil::{PIN_STICKY, PIN_TOP};

/// Why a window the workspace has open is left alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Pinned on top.
    Pinned,
    /// Kept out of workspace switches.
    Sticky,
    /// Running as administrator, which a normal app can't send a close request.
    Elevated,
}

impl Why {
    /// The word the API uses.
    pub fn word(self) -> &'static str {
        match self {
            Why::Pinned => "pinned",
            Why::Sticky => "sticky",
            Why::Elevated => "elevated",
        }
    }
}

/// What the matcher found for a window: its handle, `Seam_pin` bits and whether it is elevated.
pub type Fact = (isize, usize, bool);

/// A window taken, with its row.
pub type Taken = (usize, isize);

/// A window kept, with its row and why.
pub type Kept = (usize, isize, Why);

/// The windows matched to the rows, split into the ones taken and the ones kept, each with
/// its row. A row that is off takes nothing, and a window with any pin bit is never taken,
/// whatever "Pins stay on switch" says.
pub fn sort_out(rows: &[Rule], matched: &[Option<isize>], facts: &[Fact]) -> (Vec<Taken>, Vec<Kept>) {
    let (mut take, mut kept) = (Vec::new(), Vec::new());
    for (row, _) in rows.iter().enumerate().filter(|(_, r)| !r.off) {
        let Some(hwnd) = matched.get(row).copied().flatten() else { continue };
        let Some(&(_, pin, elevated)) = facts.iter().find(|f| f.0 == hwnd) else { continue };
        let why = if pin & PIN_STICKY != 0 {
            Some(Why::Sticky)
        } else if pin & PIN_TOP != 0 {
            Some(Why::Pinned)
        } else {
            elevated.then_some(Why::Elevated)
        };
        match why {
            Some(why) => kept.push((row, hwnd, why)),
            None => take.push((row, hwnd)),
        }
    }
    (take, kept)
}

/// The windows asked for that a fresh look still matches, and how many it doesn't. Never
/// a window the request didn't name.
pub fn still_ours(asked: &[isize], fresh: &[isize]) -> (Vec<isize>, usize) {
    let ours: Vec<isize> = asked.iter().copied().filter(|h| fresh.contains(h)).collect();
    let changed = asked.len() - ours.len();
    (ours, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Candidate, assign};

    fn rule(exe: &str, title: &str) -> Rule {
        Rule { exe: exe.into(), title_hint: title.into(), monitor: "m".into(), ..Rule::default() }
    }

    fn win(hwnd: isize, exe: &str, title: &str, z: usize) -> Candidate {
        Candidate { hwnd, exe: exe.into(), aumid: None, class: String::new(), title: title.into(), z }
    }

    fn plain(hwnds: &[isize]) -> Vec<Fact> {
        hwnds.iter().map(|h| (*h, 0, false)).collect()
    }

    #[test]
    fn only_matched_windows_are_taken() {
        let rows = [rule("a.exe", ""), rule("b.exe", ""), rule("c.exe", "")];
        let matched = [Some(1), None, Some(3)];
        let (take, kept) = sort_out(&rows, &matched, &plain(&[1, 3, 7]));
        assert_eq!(take, [(0, 1), (2, 3)]);
        assert!(kept.is_empty());
    }

    #[test]
    fn a_second_window_of_the_app_is_left_alone() {
        let rows = [rule("chrome.exe", "")];
        let wins = [win(1, "chrome.exe", "one", 0), win(2, "chrome.exe", "two", 1)];
        let matched = assign(&rows, &wins);
        let (take, _) = sort_out(&rows, &matched, &plain(&[1, 2]));
        assert_eq!(take.len(), 1);
        assert_eq!(Some(take[0].1), matched[0]);
    }

    #[test]
    fn pinned_sticky_and_elevated_are_kept_with_a_reason() {
        let rows = [rule("a.exe", ""), rule("b.exe", ""), rule("c.exe", ""), rule("d.exe", "")];
        let matched = [Some(1), Some(2), Some(3), Some(4)];
        let facts = [(1, PIN_TOP, false), (2, PIN_STICKY, false), (3, 0, true), (4, 0, false)];
        let (take, kept) = sort_out(&rows, &matched, &facts);
        assert_eq!(take, [(3, 4)]);
        assert_eq!(kept, [(0, 1, Why::Pinned), (1, 2, Why::Sticky), (2, 3, Why::Elevated)]);
        // Both bits: kept out of switches is the stronger word.
        let (_, both) = sort_out(&rows[..1], &matched[..1], &[(1, PIN_TOP | PIN_STICKY, false)]);
        assert_eq!(both, [(0, 1, Why::Sticky)]);
    }

    #[test]
    fn off_rows_take_nothing() {
        let off = Rule { off: true, ..rule("a.exe", "") };
        let rows = [off, rule("b.exe", "")];
        let (take, kept) = sort_out(&rows, &[Some(1), Some(2)], &plain(&[1, 2]));
        assert_eq!(take, [(1, 2)]);
        assert!(kept.is_empty());
    }

    #[test]
    fn the_recheck_never_adds_a_window() {
        // 9 appeared since the list; 2 went.
        let (ours, changed) = still_ours(&[1, 2, 3], &[1, 3, 9]);
        assert_eq!(ours, [1, 3]);
        assert_eq!(changed, 1);
        assert_eq!(still_ours(&[], &[1, 2]), (vec![], 0));
    }
}
