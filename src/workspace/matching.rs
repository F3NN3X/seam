//! Matching rows to the windows that are open.

use super::*;

/// A window on screen, as the matcher sees it.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub hwnd: isize,
    pub exe: String,
    pub aumid: Option<String>,
    pub class: String,
    pub title: String,
    /// 0 is the topmost.
    pub z: usize,
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// For each rule, in order, the window it gets, or `None` (launch it or skip it).
/// An AUMID beats an exe. Ties go to the same class, then the longest title
/// prefix, then z-order. A window is claimed once.
pub fn assign(rules: &[Rule], wins: &[Candidate]) -> Vec<Option<isize>> {
    let mut taken = vec![false; wins.len()];
    rules.iter().map(|r| pick(r, wins, &mut taken, |_| true)).collect()
}

/// Whether the window is one of the rule's app: by AUMID when the rule has one, else by exe.
fn same_app(r: &Rule, w: &Candidate) -> bool {
    match (&r.aumid, &w.aumid) {
        (Some(a), Some(b)) => a == b,
        (Some(_), None) => false,
        (None, _) => r.exe.eq_ignore_ascii_case(&w.exe),
    }
}

/// The best window not taken yet, among those `allowed`, for the rule, which then takes it.
fn pick(r: &Rule, wins: &[Candidate], taken: &mut [bool], allowed: impl Fn(usize) -> bool) -> Option<isize> {
    let (i, w) = wins
        .iter()
        .enumerate()
        .filter(|(i, w)| !taken[*i] && allowed(*i) && same_app(r, w))
        .max_by_key(|(_, w)| (w.class == r.class, common_prefix(&r.title_hint, &w.title), usize::MAX - w.z))?;
    taken[i] = true;
    Some(w.hwnd)
}

/// The window the tray remembers for a row with a saved folder or page, with
/// "Remember folders and pages" on (D336).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// None yet: the tray started since, or the row is older.
    Unbound,
    /// This window, which is still open.
    Alive(isize),
    /// Its window was closed.
    Gone,
}

/// `assign`, with the windows the tray remembers for place rows (D336). `bound[i]` is
/// `None` for a row that isn't one, which matches as `assign` does but never takes a
/// window bound to a place row here. A place row bound to an open window takes that
/// window only, and one whose window was closed takes none, so it's started on its
/// place. An unbound one matches by app, but never takes a window bound to a place
/// row here or in `elsewhere` (other workspaces, or windows a launch must not take).
pub fn assign_bound(
    rules: &[Rule],
    wins: &[Candidate],
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
) -> Vec<Option<isize>> {
    let bound_of = |i: usize| bound.get(i).copied().flatten();
    let mine: HashSet<isize> = (0..rules.len())
        .filter_map(|i| match bound_of(i) {
            Some(Bound::Alive(h)) => Some(h),
            _ => None,
        })
        .collect();
    let mut taken = vec![false; wins.len()];
    let mut found = vec![None; rules.len()];
    // Bound windows first, so no row before them in the order can take them.
    for (i, r) in rules.iter().enumerate() {
        if let Some(Bound::Alive(h)) = bound_of(i) {
            found[i] = pick(r, wins, &mut taken, |w| wins[w].hwnd == h);
        }
    }
    let free = |w: usize| !mine.contains(&wins[w].hwnd);
    for (i, r) in rules.iter().enumerate() {
        found[i] = match bound_of(i) {
            Some(Bound::Alive(_)) => found[i],
            Some(Bound::Gone) => None,
            Some(Bound::Unbound) => pick(r, wins, &mut taken, |w| free(w) && !elsewhere.contains(&wins[w].hwnd)),
            None => pick(r, wins, &mut taken, free),
        };
    }
    found
}
