//! A live workspace's row states, and placing a window that opens late.

use super::*;

// ---- Live status (§5) and auto-apply (§11.1)

/// A workspace of this desk, with what checking or placing its windows needs.
pub struct LiveWorkspace {
    pub workspace: Workspace,
    pub screens: Vec<Screen>,
    pub gap: u32,
    pub pins_stay: bool,
    /// Lower-case exe names Seam never manages: auto-apply leaves them alone.
    pub exclude: Vec<String>,
    /// "When a window moves to another screen".
    pub size_mode: WindowSize,
    /// "Remember folders and pages": place rows match their remembered windows (D336).
    pub places: bool,
}

/// How each row's app is doing, the way a switch would see it. Runs on a worker.
pub fn status(live: &LiveWorkspace) -> Vec<RowStatus> {
    let _com = Com::enter();
    let geos = geometry(&live.screens, live.gap);
    let mut procs = Procs::new();
    let seen = snapshot(&mut procs);
    let cands: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let found = match_rows(&live.workspace, &cands, live.places);
    // An app with no window may still run, closed to the tray.
    let mut background = Background::default();
    let rows = live.workspace.windows.iter().zip(found).enumerate();
    let row = |(i, (rule, hit)): (usize, (&Rule, Option<isize>))| {
        let state = match hit {
            None if background.runs(rule, &mut procs) => RowState::Background,
            _ => row_state(rule, hit, &seen, &geos, live),
        };
        RowStatus { rule_index: i, state }
    };
    rows.map(row).collect()
}

/// One row's state. What a switch would leave where it is counts as in place,
/// so Snap back only shows when it has something to do.
fn row_state(rule: &Rule, hit: Option<isize>, seen: &[Seen], geos: &[Geo], live: &LiveWorkspace) -> RowState {
    let Some(hwnd) = hit else { return RowState::NotRunning };
    let known = seen.iter().find(|s| s.cand.hwnd == hwnd);
    if known.is_some_and(|s| s.elevated) {
        return RowState::CantMove;
    }
    if known.is_some_and(|s| stays(s.pin, live.pins_stay)) {
        return RowState::InPlace;
    }
    let h = hwnd_of(hwnd);
    let Some((geo, kind, _)) = destination(rule, geos, h, live.size_mode, false) else { return RowState::InPlace };
    let Some(now) = now_of(h, geo.dpi) else { return RowState::InPlace };
    if now.shown == Shown::Minimized && !matches!(kind, Kind::Min(_)) {
        RowState::Minimized
    } else if in_place(&kind, &now, geo.bounds) {
        RowState::InPlace
    } else {
        RowState::Moved
    }
}

/// Put a window that just opened where the active workspace puts it, once: the
/// best free row for it, the same way a switch matches. A row is free when no
/// window carries its stamp. The window is stamped either way, so it's never
/// placed again. Returns what happened, for the log, or `None` for a window the
/// workspace doesn't have a free row for.
pub fn place_opened(live: &LiveWorkspace, hwnd: isize) -> Option<String> {
    let started = Instant::now();
    let h = hwnd_of(hwnd);
    let valid = unsafe { IsWindow(Some(h)) }.as_bool();
    if !valid || winutil::ws_stamp(h).is_some() || winutil::is_minimized(h) || switching() {
        return None;
    }
    let _com = Com::enter();
    let seen = seen_of(h, 0, &live.exclude, &mut Procs::new(), &mut |_, _| {})?;
    // Administrator windows are left alone without a word, as a switch's are.
    if seen.elevated || stays(seen.pin, live.pins_stay) {
        return None;
    }
    let ws = &live.workspace;
    let key = workspace::stamp_key(&ws.id);
    let others = winutil::top_level_windows().into_iter().filter(|w| *w != h);
    let taken: HashSet<usize> =
        others.filter_map(winutil::ws_stamp).filter(|(k, _)| *k == key).map(|(_, r)| r).collect();
    let free: Vec<usize> = (0..ws.windows.len()).filter(|i| !taken.contains(i)).collect();
    let rules: Vec<Rule> = free.iter().map(|i| ws.windows[*i].clone()).collect();
    // A place row takes only its remembered window, as in a switch (D336).
    let (bound, elsewhere) = bindings(ws, live.places);
    let bound: Vec<Option<Bound>> = free.iter().map(|i| bound[*i]).collect();
    let cand = std::slice::from_ref(&seen.cand);
    let hit = workspace::assign_bound(&rules, cand, &bound, &elsewhere).iter().position(Option::is_some)?;
    let (row, rule) = (free[hit], &ws.windows[free[hit]]);
    let app = app_name(rule);
    let geos = geometry(&live.screens, live.gap);
    let moved = move_opened(h, rule, &geos, live.size_mode);
    winutil::set_ws_stamp(h, key, row);
    let what = if moved { "placed" } else { "already in place" };
    Some(format!("{app} for row {}: {what} in {} ms", row + 1, started.elapsed().as_millis()))
}

/// Move a window that just opened to where the rule puts it. Whether it moved.
pub(super) fn move_opened(h: HWND, rule: &Rule, geos: &[Geo], mode: WindowSize) -> bool {
    let target = destination(rule, geos, h, mode, true);
    let now = target.as_ref().and_then(|(geo, ..)| now_of(h, geo.dpi));
    match (target, now) {
        (Some((geo, kind, note)), Some(now)) if !in_place(&kind, &now, geo.bounds) => {
            if let Some(note) = note {
                log::info!("{note}");
            }
            let on_screen = geo.bounds.contains(centre(now.window).0, centre(now.window).1);
            let crosses = winutil::monitor_dpi(h) != geo.dpi;
            let (was, before, dpi) = (now.shown, now.window, geo.dpi);
            let m = Move { hwnd: h.0 as isize, app: app_name(rule), kind, was, on_screen, crosses, dpi, before };
            // A switch that starts meanwhile has the last word.
            place_all(std::slice::from_ref(&m), &mut Vec::new(), &switching) > 0
        }
        _ => false,
    }
}
