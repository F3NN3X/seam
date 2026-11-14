//! Which window belongs to which row, kept across switches.

use super::*;

/// "Remember folders and pages" (D336): the window of each place row, by workspace id
/// and row, in memory only. `None` once that window was closed. A row is its index,
/// carried over edits by `workspace::moved_rows` as the switch stamps are. A window
/// is one row's only.
static BOUND: Mutex<Vec<(String, usize, Option<Held>)>> = Mutex::new(Vec::new());

/// A bound window with the process it belonged to, so a handle Windows gives to a new window isn't taken for it.
#[derive(Clone, Copy)]
struct Held {
    hwnd: isize,
    pid: u32,
    started: u64,
}

fn held(hwnd: isize) -> Held {
    let pid = winutil::process_id(hwnd_of(hwnd));
    Held { hwnd, pid, started: seam::process::started(pid).unwrap_or(0) }
}

/// The window is still open and is the same one: its handle may since belong to another process's window.
/// A start time that couldn't be read (0), then or now, doesn't tell them apart.
fn alive(h: &Held) -> bool {
    let started = |t: u64| t == 0 || h.started == 0 || t == h.started;
    unsafe { IsWindow(Some(hwnd_of(h.hwnd))) }.as_bool()
        && winutil::process_id(hwnd_of(h.hwnd)) == h.pid
        && started(seam::process::started(h.pid).unwrap_or(0))
}

/// Remember `hwnd` as the window of row `row` of workspace `id`.
pub(super) fn bind(id: &str, row: usize, hwnd: isize) {
    let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
    bound.retain(|(w, r, h)| !(w == id && *r == row) && h.map(|h| h.hwnd) != Some(hwnd));
    bound.push((id.to_string(), row, Some(held(hwnd))));
}

/// Bind the windows a capture read, each to its row, for the rows of `ws` with a place.
pub fn bind_rows(ws: &Workspace, windows: &[(isize, usize)]) {
    for (hwnd, row) in windows {
        if ws.windows.get(*row).is_some_and(|r| usable_place(r).is_some()) {
            bind(&ws.id, *row, *hwnd);
        }
    }
}

/// The bindings of `ws` with each window as its index in a capture's `hwnds`, for
/// pairing the capture with the rows as a switch would (D336). A row bound to a window
/// the capture didn't take pairs with none. Empty with the setting off.
pub fn capture_bindings(ws: &Workspace, hwnds: &[isize], remember: bool) -> (Vec<Option<Bound>>, HashSet<isize>) {
    let (bound, elsewhere) = bindings(ws, remember);
    let index = |h: isize| hwnds.iter().position(|w| *w == h).map(|i| i as isize);
    let bound = bound
        .into_iter()
        .map(|b| match b {
            Some(Bound::Alive(h)) => Some(Bound::Alive(index(h).unwrap_or(-1))),
            other => other,
        })
        .collect();
    (bound, elsewhere.into_iter().filter_map(index).collect())
}

/// Workspace `id`'s row `row` remembers no window, so the next switch matches it by app.
pub(super) fn unbind(id: &str, row: usize) {
    BOUND.lock().unwrap_or_else(|e| e.into_inner()).retain(|(w, r, _)| !(w == id && *r == row));
}

/// Workspaces were deleted: only those in `ids` keep their bindings.
pub fn keep_bindings(ids: &[String]) {
    BOUND.lock().unwrap_or_else(|e| e.into_inner()).retain(|(w, _, _)| ids.contains(w));
}

/// An edit reordered, removed or added rows of workspace `id`: its bindings follow.
pub fn rebind(id: &str, moves: &[Option<usize>]) {
    let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
    bound.retain_mut(|(w, r, _)| {
        if w != id {
            return true;
        }
        match moves.get(*r).copied().flatten() {
            Some(to) => {
                *r = to;
                true
            }
            None => false,
        }
    });
}

/// Every open window bound to any row. Closed ones are marked as they're read.
pub(super) fn bound_windows() -> HashSet<isize> {
    let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
    prune(&mut bound);
    bound.iter().filter_map(|(_, _, h)| h.map(|h| h.hwnd)).collect()
}

fn prune(bound: &mut [(String, usize, Option<Held>)]) {
    for entry in bound.iter_mut().filter(|(_, _, h)| h.as_ref().is_some_and(|h| !alive(h))) {
        entry.2 = None;
    }
}

/// Each row's binding for `workspace::assign_bound`, `None` for a row without a usable
/// place or with the setting off, and the windows bound to other workspaces' rows.
pub(super) fn bindings(ws: &Workspace, remember: bool) -> (Vec<Option<Bound>>, HashSet<isize>) {
    if !remember {
        return (vec![None; ws.windows.len()], HashSet::new());
    }
    let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
    prune(&mut bound);
    let of = |row: usize| match bound.iter().find(|(w, r, _)| *w == ws.id && *r == row) {
        Some((_, _, Some(h))) => Bound::Alive(h.hwnd),
        Some((_, _, None)) => Bound::Gone,
        None => Bound::Unbound,
    };
    let rows = ws.windows.iter().enumerate().map(|(i, r)| usable_place(r).map(|_| of(i))).collect();
    let elsewhere = bound.iter().filter(|(w, _, _)| *w != ws.id).filter_map(|(_, _, h)| h.map(|h| h.hwnd)).collect();
    (rows, elsewhere)
}

/// `workspace::assign_bound` for a workspace, with the windows the tray remembers.
pub(super) fn match_rows(ws: &Workspace, wins: &[Candidate], remember: bool) -> Vec<Option<isize>> {
    let (bound, elsewhere) = bindings(ws, remember);
    workspace::assign_bound(&ws.windows, wins, &bound, &elsewhere)
}
