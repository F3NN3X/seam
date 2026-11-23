//! Which window belongs to which row, kept across switches.

use super::*;

/// "Remember folders and pages" (D336): the window of each place row, by workspace id
/// and row, in memory only. `None` once that window was closed. A row is its index,
/// carried over edits by `workspace::moved_rows` as the switch stamps are. A window
/// is one row's only.
static BOUND: Mutex<Vec<(String, usize, Option<Held>)>> = Mutex::new(Vec::new());

/// The item rows the tray found saved when it started, by `item_key`. After a restart such a row
/// matches by app once (D733). A row added, or whose item changed, since has no window of its
/// own to match, so it starts its item instead of taking an unrelated open window.
static STARTUP_ITEMS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn item_key(id: &str, rule: &Rule) -> String {
    format!("{id}\u{1}{}", workspace::item_shape(rule))
}

/// Remember the item rows of these workspaces as the ones that existed at start.
/// A row's item was edited, so its window isn't the new item's: the next switch starts it.
pub fn forget_edited(id: &str, rows: &[usize]) {
    rows.iter().for_each(|row| unbind(id, *row));
}

pub fn know_item_rows(workspaces: &[Workspace]) {
    let keys =
        workspaces.iter().flat_map(|ws| ws.windows.iter().filter(|r| r.item.is_some()).map(|r| item_key(&ws.id, r)));
    *STARTUP_ITEMS.lock().unwrap_or_else(|e| e.into_inner()) = Some(keys.collect());
}

/// What an item row with no binding is: a row known at start matches by app, any other starts its item.
fn first_state(known: Option<&HashSet<String>>, key: &str) -> Bound {
    if known.is_some_and(|k| k.contains(key)) { Bound::Unbound } else { Bound::Gone }
}

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

/// A row whose window the tray remembers: an item's, whatever "Remember folders and pages"
/// says, because an item is something I typed (D733), and a row with a place, with it on.
fn bindable(rule: &Rule, remember: bool) -> bool {
    workspace::takes_part(rule) && (rule.item.is_some() || remember && usable_place(rule).is_some())
}

/// Bind the windows a capture read, each to its row, for the rows of `ws` with a place or an item.
pub fn bind_rows(ws: &Workspace, windows: &[(isize, usize)]) {
    for (hwnd, row) in windows {
        if ws.windows.get(*row).is_some_and(|r| bindable(r, true)) {
            bind(&ws.id, *row, *hwnd);
        }
    }
}

/// The bindings of `ws` with each window as its index in a capture's `hwnds`, for
/// pairing the capture with the rows as a switch would (D336). A row bound to a window
/// the capture didn't take pairs with none.
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

/// Each row's binding for `workspace::assign_bound`, `None` for a row without an item or a usable
/// place, a row that is off, or a place row with the setting off, and the windows bound to other
/// workspaces' rows.
pub(super) fn bindings(ws: &Workspace, remember: bool) -> (Vec<Option<Bound>>, HashSet<isize>) {
    let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
    prune(&mut bound);
    let startup = STARTUP_ITEMS.lock().unwrap_or_else(|e| e.into_inner());
    let of = |row: usize| match bound.iter().find(|(w, r, _)| *w == ws.id && *r == row) {
        Some((_, _, Some(h))) => Bound::Alive(h.hwnd),
        Some((_, _, None)) => Bound::Gone,
        None if ws.windows[row].item.is_some() => first_state(startup.as_ref(), &item_key(&ws.id, &ws.windows[row])),
        None => Bound::Unbound,
    };
    let rows = ws.windows.iter().enumerate().map(|(i, r)| bindable(r, remember).then(|| of(i))).collect();
    let elsewhere = bound.iter().filter(|(w, _, _)| *w != ws.id).filter_map(|(_, _, h)| h.map(|h| h.hwnd)).collect();
    (rows, elsewhere)
}

/// `workspace::assign_bound` for a workspace, with the windows the tray remembers.
pub(super) fn match_rows(ws: &Workspace, wins: &[Candidate], remember: bool) -> Vec<Option<isize>> {
    let (bound, elsewhere) = bindings(ws, remember);
    workspace::assign_bound(&ws.windows, wins, &bound, &elsewhere)
}

/// `match_rows`, with the rows on a screen it doesn't reach (another than the workspace's scope,
/// or one of `away`) sitting out like off rows, so they take no window (D788).
pub(super) fn match_reached(ws: &Workspace, wins: &[Candidate], remember: bool, away: &[String]) -> Vec<Option<isize>> {
    let (bound, elsewhere) = bindings(ws, remember);
    let rows = workspace::sit_out(&ws.windows, ws.scope.as_deref(), away);
    workspace::assign_bound(&rows, wins, &bound, &elsewhere)
}

#[cfg(test)]
mod tests {
    use super::*;
    use workspace::Item;

    fn item_row(folder: &str) -> Rule {
        Rule {
            exe: r"C:\Windows\explorer.exe".into(),
            class: "CabinetWClass".into(),
            item: Some(Item::Folder { folder: folder.into() }),
            ..Rule::default()
        }
    }

    fn place_row() -> Rule {
        Rule {
            exe: r"C:\Windows\explorer.exe".into(),
            class: "CabinetWClass".into(),
            place: Some(Place::Folder(r"C:\p".into())),
            ..Rule::default()
        }
    }

    #[test]
    fn item_rows_bind_whatever_the_setting_says() {
        assert!(bindable(&item_row(r"C:\a"), false));
        assert!(bindable(&item_row(r"C:\a"), true));
        assert!(!bindable(&place_row(), false));
        assert!(bindable(&place_row(), true));
        assert!(!bindable(&Rule { off: true, ..item_row(r"C:\a") }, true));
        assert!(!bindable(&Rule::default(), true));
    }

    #[test]
    fn bindings_cover_item_rows_with_the_setting_off() {
        let ws = Workspace {
            id: "bind-test-ws".into(),
            windows: vec![item_row(r"C:\a"), place_row()],
            ..Workspace::default()
        };
        let (off, _) = bindings(&ws, false);
        assert!(off[0].is_some() && off[1].is_none());
        let (on, _) = bindings(&ws, true);
        assert!(on[0].is_some() && on[1].is_some());
    }

    #[test]
    fn an_item_row_added_since_start_starts_its_item() {
        let (here, new) = (item_row(r"C:\a"), item_row(r"C:\b"));
        let known = HashSet::from([item_key("w", &here)]);
        assert_eq!(first_state(Some(&known), &item_key("w", &here)), Bound::Unbound);
        // A changed item, another workspace and no list at all are all rows the tray didn't know.
        assert_eq!(first_state(Some(&known), &item_key("w", &new)), Bound::Gone);
        assert_eq!(first_state(Some(&known), &item_key("x", &here)), Bound::Gone);
        assert_eq!(first_state(None, &item_key("w", &here)), Bound::Gone);
    }
}
