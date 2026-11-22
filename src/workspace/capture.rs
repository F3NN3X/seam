//! Folding a fresh capture into a saved workspace, and checked edits.

use super::secrets::settle;
use super::*;
use crate::config::ClosedApps;

/// A capture's rules as the matcher sees windows, so a recapture finds the rows a
/// switch would.
pub(super) fn as_candidates(fresh: &[Rule]) -> Vec<Candidate> {
    let cand = |(i, r): (usize, &Rule)| Candidate {
        hwnd: i as isize,
        exe: r.exe.clone(),
        aumid: r.aumid.clone(),
        class: r.class.clone(),
        title: r.title_hint.clone(),
        z: i,
    };
    fresh.iter().enumerate().map(cand).collect()
}

/// The capture's window, by index, for each row of `old`, paired as a switch matches.
/// `bound` and `elsewhere` are the tray's bindings (D336) with windows as capture
/// indices, so a place row pairs with its own window. Empty for plain matching.
pub(super) fn pair(
    old: &[Rule],
    fresh: &[Rule],
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
) -> Vec<Option<isize>> {
    assign_bound_all(old, &as_candidates(fresh), bound, elsewhere)
}

/// What "Update from screen" makes of a workspace's rows.
pub struct Merged {
    pub windows: Vec<Rule>,
    /// For each row of `windows`, the row of the old workspace it kept, or `None` for
    /// one the capture added.
    pub from: Vec<Option<usize>>,
    /// For each window of the capture, its row in `windows`.
    pub rows: Vec<usize>,
    /// How many rows were removed.
    pub removed: usize,
    /// How many rows were turned off, that weren't off already.
    pub turned_off: usize,
}

/// Whether some window in `wins` is of the row's app: the same program, or the same AUMID.
pub(super) fn app_open(row: &Rule, wins: &[&Candidate]) -> bool {
    wins.iter().any(|w| {
        (!row.exe.is_empty() && w.exe.eq_ignore_ascii_case(&row.exe))
            || row.aumid.as_ref().is_some_and(|a| w.aumid.as_ref() == Some(a))
    })
}

/// "Update from screen": each row the capture has a window for takes where and how
/// that window is now, and keeps its launch details and its place in the start
/// order. A row with no window right now stays. What `closed` says for it (by its row
/// of `old`) decides: Keep leaves it, Turn off sets it off, Remove takes it out. That
/// is only for a row whose app has no window the capture didn't give a row: bound
/// elsewhere, or in `others` (windows of the desk it made no rule of, such as on
/// another desktop). Nothing is removed or turned off when that would be every row of
/// `old` (`review::not_open`). New windows go at the end, topmost first. `bound` and
/// `elsewhere` as `pair`.
pub fn merge_capture(
    old: &Workspace,
    fresh: Vec<Rule>,
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
    others: &[Candidate],
    closed: &dyn Fn(usize) -> ClosedApps,
) -> Merged {
    let found = pair(&old.windows, &fresh, bound, elsewhere);
    let mut taken = vec![false; fresh.len()];
    for i in found.iter().flatten() {
        taken[*i as usize] = true;
    }
    let not_open = review::not_open(old, &fresh, bound, elsewhere, others);
    let (mut removed, mut turned_off) = (0, 0);
    let mut windows = Vec::with_capacity(old.windows.len());
    let mut from = Vec::with_capacity(old.windows.len());
    let mut rows = vec![0; fresh.len()];
    for (o, (row, hit)) in old.windows.iter().zip(&found).enumerate() {
        let choice = if not_open.contains(&o) { closed(o) } else { ClosedApps::Keep };
        if choice == ClosedApps::Remove {
            removed += 1;
            continue;
        }
        let mut row = row.clone();
        if choice == ClosedApps::TurnOff {
            turned_off += usize::from(!row.off);
            row.off = true;
        }
        match hit {
            Some(i) => {
                rows[*i as usize] = windows.len();
                row.take_place(&fresh[*i as usize]);
            }
            // With no window, nothing says how it stacks or whether it has the focus.
            None => {
                row.z = u32::MAX;
                row.focused = false;
            }
        }
        windows.push(row);
        from.push(Some(o));
    }
    for (i, rule) in fresh.into_iter().enumerate().filter(|(i, _)| !taken[*i]) {
        rows[i] = windows.len();
        windows.push(rule);
        from.push(None);
    }
    Merged { windows, from, rows, removed, turned_off }
}

/// Where each row of `old` went in `new` after an edit reordered, removed or added
/// rows: the same rule first, then the same app the way a switch matches, for a
/// row that also changed. `None` for a row that was removed.
pub fn moved_rows(old: &[Rule], new: &[Rule]) -> Vec<Option<usize>> {
    let mut taken = vec![false; new.len()];
    let mut moves: Vec<Option<usize>> = old
        .iter()
        .map(|rule| {
            let j = (0..new.len()).find(|j| !taken[*j] && new[*j] == *rule)?;
            taken[j] = true;
            Some(j)
        })
        .collect();
    let left: Vec<usize> = (0..new.len()).filter(|j| !taken[*j]).collect();
    let changed: Vec<usize> = (0..old.len()).filter(|i| moves[*i].is_none()).collect();
    let rules: Vec<Rule> = changed.iter().map(|i| old[*i].clone()).collect();
    let cands = as_candidates(&left.iter().map(|j| new[*j].clone()).collect::<Vec<_>>());
    for (i, hit) in changed.into_iter().zip(assign(&rules, &cands)) {
        moves[i] = hit.map(|k| left[k as usize]);
    }
    moves
}

/// "Where it is now" for one row: the place of the window a capture has for it,
/// and which of the capture's windows that is, or `None` when the app has no window open.
/// `bound` and `elsewhere` as `pair`, for every row of `old`.
pub fn recapture_row(
    old: &Workspace,
    row: usize,
    fresh: &[Rule],
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
) -> Option<(Workspace, usize)> {
    let rule = old.windows.get(row)?;
    let bound = [bound.get(row).copied().flatten()];
    let hit = pair(std::slice::from_ref(rule), fresh, &bound, elsewhere).into_iter().next().flatten()?;
    let from = &fresh[hit as usize];
    let mut updated = old.clone();
    if from.focused {
        updated.windows.iter_mut().for_each(|r| r.focused = false);
    }
    updated.windows[row].take_place(from);
    Some((updated, hit as usize))
}

/// "Add an app" from the windows open now: the row takes the folder or page the tray
/// read from the one window picked, `picked`. Only a row that is still the one the
/// panel added (its `exe` and `class`), has no place yet, and is the picked window's
/// app and class. `None` otherwise, or when the window has none.
pub fn take_window_place(old: &Workspace, row: usize, exe: &str, class: &str, picked: &[Rule]) -> Option<Workspace> {
    let rule = old.windows.get(row).filter(|r| r.exe == exe && r.class == class && r.place.is_none())?;
    let [from] = picked else { return None };
    if from.class != rule.class {
        return None;
    }
    assign(std::slice::from_ref(rule), &as_candidates(picked)).into_iter().next().flatten()?;
    let mut updated = old.clone();
    updated.windows[row].place = Some(from.place.clone()?);
    Some(updated)
}

/// Workspace `n` of the desk's list, counted from 1 in list order.
pub fn numbered(list: &[Workspace], n: usize) -> Option<&Workspace> {
    list.get(n.checked_sub(1)?)
}

/// What "Add window to workspace" made of a workspace.
pub struct Added {
    /// The workspace with the window in it. A folder or page the window showed isn't in
    /// it: only a capture may set one, so the tray stores `place` after the checks.
    pub workspace: Workspace,
    pub row: usize,
    /// The window got a row of its own, where an existing row would have been updated.
    pub new: bool,
    /// The folder or page the window showed, for its row.
    pub place: Option<Place>,
}

/// The focused window, read as a capture would, goes into `old`. The row it updates is,
/// in this order: the one bound to this very window (`bound`, as a switch keeps them);
/// the only row of its app, even when the window has moved, unless it is bound to another window; with several rows of its app,
/// the one on the window's screen and in its place. That row takes where the window is now,
/// its stack included, and keeps its launch details, its start order and its focus. With no
/// such row the window gets one at the end, with launch off and no Open with. Never
/// removes a row. `bound` has this window as index 0.
pub fn add_window(old: &Workspace, fresh: Rule, bound: &[Option<Bound>]) -> Added {
    let this = as_candidates(std::slice::from_ref(&fresh));
    let same_app = |r: &Rule| matching::same_app(r, &this[0]);
    let app_rows: Vec<usize> = (0..old.windows.len()).filter(|i| same_app(&old.windows[*i])).collect();
    let held = app_rows.iter().copied().find(|i| matches!(bound.get(*i).copied().flatten(), Some(Bound::Alive(0))));
    // An item is one row of its own, so a window that isn't its own never updates it (D731).
    let of_app: Vec<usize> = app_rows.into_iter().filter(|i| old.windows[*i].item.is_none()).collect();
    let only = (of_app.len() == 1)
        .then(|| of_app[0])
        .filter(|i| !matches!(bound.get(*i).copied().flatten(), Some(Bound::Alive(h)) if h != 0));
    let filled = || {
        of_app.iter().copied().find(|i| {
            let r = &old.windows[*i];
            r.monitor == fresh.monitor && r.target == fresh.target
        })
    };
    let place = fresh.place.clone();
    let mut workspace = old.clone();
    let (row, new) = match held.or(only).or_else(filled) {
        Some(row) => {
            let rule = &mut workspace.windows[row];
            let (focused, z, kept) = (rule.focused, rule.z, rule.place.clone());
            rule.take_place(&fresh);
            (rule.focused, rule.z, rule.place) = (focused, z, kept);
            (row, false)
        }
        None => {
            let z = old.windows.len() as u32;
            let rule = Rule { launch: false, open_with: None, focused: false, z, place: None, ..fresh };
            workspace.windows.push(rule);
            (old.windows.len(), true)
        }
    };
    Added { workspace, row, new, place }
}

/// Why a window can't be added to a workspace, in the notice's words. `skip` is the
/// reason a capture leaves a window out, or one of `administrator`, `sticky` and `off screen`.
pub fn why_not(skip: &str) -> &'static str {
    match skip {
        "seam" => "That's one of Seam's own windows",
        "never include" => "That app is under Left alone, so Seam never saves it",
        "administrator" => "That app runs as administrator, so Seam couldn't move it later",
        "sticky" => "That window is kept out of workspace switches, so no workspace holds it",
        "too small" => "That window is too small for Seam to save",
        "no title" => "That window has no title, so Seam doesn't save it",
        "transparent" | "system" | "shell" => "That's part of Windows, not an app window Seam saves",
        "hidden" | "cloaked" | "off screen" => "That window isn't showing on a screen of this desk",
        "no exe" => "Seam can't tell which app that window belongs to",
        _ => "That window has no taskbar button, so Seam doesn't save it",
    }
}

/// The rows a switch starts, in start order, cut into stages. A stage ends at a
/// row that waits for its window. Apps in a stage start together, each after its
/// own delay, and the next stage starts once the waiting ones are up.
pub fn stages(rules: &[Rule], starting: &[usize]) -> Vec<Vec<usize>> {
    let mut stages: Vec<Vec<usize>> = Vec::new();
    let mut open = false;
    for &i in starting {
        match stages.last_mut() {
            Some(stage) if open => stage.push(i),
            _ => stages.push(vec![i]),
        }
        open = !rules.get(i).is_some_and(|r| r.wait_for_window);
    }
    stages
}

/// A delay as a switch uses it: 0 to 30 s, in half seconds.
pub fn tame_delay(ms: u32) -> u32 {
    (ms.min(MAX_DELAY_MS) + DELAY_STEP_MS / 2) / DELAY_STEP_MS * DELAY_STEP_MS
}

/// What the editor sends, made safe to save over `old`. The id stays, a zone it
/// picks must be on its screen (`zones` says how many a screen has), and a program or
/// AUMID that wasn't in the workspace already must pass the same checks as an
/// import. A row with nothing safe to start can't be ticked to start, which keeps
/// an imported workspace's cleared rows from launching. Errors say why, in the
/// panel's words.
pub fn checked_update(old: &Workspace, new: Workspace, zones: &dyn Fn(&str) -> usize) -> Result<Workspace, String> {
    checked_rows(old, new, zones, None)
}

/// `checked_update`, but with `only` set the Open with of every other row is left as it
/// is and not checked again, so an add isn't refused for another row's stale one.
pub fn checked_rows(
    old: &Workspace,
    mut new: Workspace,
    zones: &dyn Fn(&str) -> usize,
    only: Option<usize>,
) -> Result<Workspace, String> {
    if new.id != old.id {
        return Err("That's a different workspace".into());
    }
    let name = new.name.trim().to_string();
    new.name = if name.is_empty() { old.name.clone() } else { name };
    let mut focused = false;
    // Sealed commands this workspace holds already: the only ones a save may keep.
    let held = sealed_in(&[to_value(old)]);
    for (i, rule) in new.windows.iter_mut().enumerate() {
        let known = old.windows.iter().any(|o| o.exe == rule.exe && o.aumid == rule.aumid);
        let exe_ok = rule.exe.is_empty() || safe_exe(&rule.exe);
        let aumid_ok = rule.aumid.as_deref().is_none_or(safe_aumid);
        if !(known || exe_ok && aumid_ok) {
            return Err(format!("{} wasn't added, because Seam won't start that program", rule.title_hint));
        }
        // Only a place the editor chose is checked, so a layout changed since keeps the
        // workspace editable.
        let placed = old.windows.iter().any(|o| o.monitor == rule.monitor && o.target == rule.target);
        if let (Target::Zone { zone, span_to }, false) = (&rule.target, placed) {
            let (zone, span_to, count) = (*zone, *span_to, zones(&rule.monitor) as u32);
            if !(1..=count).contains(&zone) || span_to.is_some_and(|to| !(1..=count).contains(&to)) {
                return Err("That zone isn't on its screen any more".into());
            }
        }
        let startable = (exe_ok && !rule.exe.is_empty()) || rule.aumid.as_deref().is_some_and(safe_aumid);
        rule.launch &= startable;
        rule.delay_ms = tame_delay(rule.delay_ms);
        // Pins belong to the last session.
        rule.pin = None;
        // Only a capture finds a folder or page, so the editor can keep or drop one but not
        // set one, nor move one to another app's row.
        let place = rule.place.take();
        let same_app = |o: &Rule| o.exe == rule.exe && (rule.class.is_empty() || o.class == rule.class);
        let kept = place.filter(|p| old.windows.iter().any(|o| same_app(o) && o.place.as_ref() == Some(p)));
        rule.place = kept;
        // What I typed, checked and stored as it's cleaned.
        let skip = only.is_some_and(|row| row != i);
        let typed = rule.open_with.take_if(|_| !skip).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        if let Some(typed) = typed {
            match open_with_for(rule, &typed) {
                Ok(OpenWith::Path(open) | OpenWith::Link(open)) => rule.open_with = Some(open),
                Err(why) => return Err(format!("{}: {why}", rule.title_hint)),
            }
        }
        // An item is what I typed too, and it wins over Open with and a saved place (D732).
        if let Some(item) = rule.item.take() {
            // A row `only` leaves alone is still settled, and one that fails its check keeps no command.
            let checked = check_item(&item, &rule.exe, rule.aumid.as_deref());
            let mut item = match (checked, skip) {
                (Ok(item), _) => item,
                (Err(_), true) => item.without_commands(),
                (Err(why), false) => return Err(format!("{}: {why}", rule.title_hint)),
            };
            if settle(&mut item, &|sealed| held.contains(sealed)) > 0 {
                crate::log::info!(
                    "workspace {}, row {i}: a startup command with no reference of the tray's is dropped",
                    new.name
                );
            }
            rule.item = Some(item);
        }
        if rule.item.is_some() {
            rule.open_with = None;
            rule.place = None;
        }
        // At most one window has the focus.
        rule.focused &= !focused;
        focused |= rule.focused;
    }
    // A screen's entry the editor changed must be a layout within the editor's limits,
    // and is stored as read. One it didn't change stays as it was.
    let mut zones = BTreeMap::new();
    for (screen, saved) in std::mem::take(&mut new.zones) {
        let entry = if old.zones.get(&screen) == Some(&saved) { Some(saved) } else { clean_entry(&saved) };
        let Some(entry) = entry else { return Err("That layout can't be used for a screen".into()) };
        zones.insert(screen, entry);
    }
    new.zones = zones;
    Ok(new)
}
