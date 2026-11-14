//! Running the rows stage by stage, catching windows as apps open them.

use super::*;

/// Place the windows of the rules `which` that have one, then pin again what the
/// rule says was pinned. A window already in place takes no call. Adds to `tally`.
pub(super) fn place_rules(run: &Run, found: &[Option<isize>], which: &[usize], seen: &[Seen], tally: &mut Tally) {
    let (ws, cancelled) = (run.ws, run.cancelled);
    let started = Instant::now();
    let (mut moves, mut ready) = (Vec::new(), 0);
    for i in which {
        let (rule, Some(hwnd)) = (&ws.windows[*i], found[*i]) else { continue };
        let h = hwnd_of(hwnd);
        // The window has its row now, so auto-apply leaves it alone and its moves count as drift.
        if let Some(key) = run.stamp {
            winutil::set_ws_stamp(h, key, *i);
        }
        if !unsafe { IsWindow(Some(h)) }.as_bool() {
            continue;
        }
        // A screen that isn't connected, or a window to keep where it is, isn't moved.
        let Some((geo, kind, note)) = destination(rule, &run.geos, h, run.size_mode, true) else { continue };
        let app = app_name(rule);
        let pin = winutil::pin_bits(h);
        if stays(pin, run.pins_stay) {
            let why = if pin & winutil::PIN_STICKY != 0 { "sticky" } else { "pinned" };
            tally.skipped.push((app, why.into()));
            continue;
        }
        let elevated = seen.iter().find(|s| s.cand.hwnd == hwnd).map(|s| s.elevated);
        if elevated.unwrap_or_else(|| winutil::is_elevated_over_us(h)) {
            tally.skipped.push((app, "elevated".into()));
            continue;
        }
        let Some(now) = now_of(h, geo.dpi) else { continue };
        if !matches!(kind, Kind::Min(_)) {
            let now_z = seen.iter().find(|s| s.cand.hwnd == hwnd).map(|s| s.cand.z);
            tally.stack.push(Stacked { hwnd, z: rule.z, now: now_z, focused: rule.focused });
        }
        if in_place(&kind, &now, geo.bounds) {
            ready += 1;
            continue;
        }
        let on_screen = geo.bounds.contains(centre(now.window).0, centre(now.window).1);
        let crosses = winutil::monitor_dpi(h) != geo.dpi;
        if let Some(note) = note {
            log::info!("{note}");
        }
        moves.push(Move { hwnd, app, kind, was: now.shown, on_screen, crosses, dpi: geo.dpi, before: now.window });
    }
    tally.plan += started.elapsed();
    tally.moves += moves.len();
    tally.in_place += ready;
    // Already in place counts as placed.
    tally.placed += ready + place_all(&moves, &mut tally.skipped, cancelled);
    if cancelled() {
        return;
    }
    // The last session pins what was pinned again, by rule, because a window's handle
    // doesn't outlive a sign-out.
    for i in which {
        if let (Some(pin), Some(hwnd)) = (ws.windows[*i].pin, found[*i]) {
            pin_again(hwnd, pin);
        }
    }
}

impl Starts {
    /// Start the app of row `i`, unless a switch this one replaced already did.
    fn start(&mut self, ws: &Workspace, i: usize, mine: u64, places: bool, tally: &mut Tally) {
        let rule = &ws.windows[i];
        let key = launch_key(rule);
        if let Some(at) = earlier_launch(&key, mine, &mut self.reused) {
            self.started.push(i);
            self.waiting.push((i, at));
            return;
        }
        match launch(rule, places) {
            Ok(()) => {
                note_launch(key, mine);
                self.launched += 1;
                self.started.push(i);
                self.waiting.push((i, Instant::now()));
            }
            Err(reason) => tally.skipped.push((app_name(rule), reason.into())),
        }
    }

    fn is_waiting(&self, i: usize) -> bool {
        self.waiting.iter().any(|(w, _)| *w == i)
    }
}

/// Give the rows still waiting the windows in `cands` that no row has, and place
/// each one that got its window at once.
fn catch(run: &Run, cands: Vec<Candidate>, waits: &mut Waits, starts: &mut Starts, tally: &mut Tally) {
    let claimed: HashSet<isize> = waits.found.iter().flatten().copied().collect();
    let fresh: Vec<Candidate> = cands.iter().filter(|c| !claimed.contains(&c.hwnd)).cloned().collect();
    waits.known.extend(cands);
    if fresh.is_empty() || starts.waiting.is_empty() {
        return;
    }
    let rules: Vec<Rule> = starts.waiting.iter().map(|(i, _)| run.ws.windows[*i].clone()).collect();
    // A place row started its app, so it takes only a window that wasn't there before
    // and isn't bound to any row (D336).
    let bound: Vec<Option<Bound>> = starts.waiting.iter().map(|(i, _)| run.bound[*i].map(|_| Bound::Unbound)).collect();
    let mut before: HashSet<isize> = waits.seen.iter().map(|s| s.cand.hwnd).collect();
    before.extend(bound_windows());
    let mut caught = Vec::new();
    for ((i, since), hit) in starts.waiting.iter().zip(workspace::assign_bound(&rules, &fresh, &bound, &before)) {
        if let Some(hwnd) = hit {
            waits.found[*i] = Some(hwnd);
            caught.push(*i);
            if run.bound[*i].is_some() {
                bind(&run.ws.id, *i, hwnd);
            }
            let app = app_name(&run.ws.windows[*i]);
            log::info!("{}: {app} showed after {} ms", run.ws.name, since.elapsed().as_millis());
        }
    }
    starts.waiting.retain(|(i, _)| !caught.contains(i));
    place_rules(run, waits.found, &caught, waits.seen, tally);
}

/// Start the apps in stages and place each window as soon as it shows (§11.3).
/// Apps in a stage start together, each after its own delay. The next stage
/// starts when the stage's waiting apps have a window or took too long. Windows
/// come from `seam-wm`'s shown events, with a look every second as the fallback.
/// Returns `None` if a newer switch replaced this one.
pub(super) fn run_stages(
    run: &Run,
    mine: u64,
    starting: &[usize],
    waits: &mut Waits,
    starts: &mut Starts,
    tally: &mut Tally,
) -> Option<()> {
    let ws = run.ws;
    let stages = workspace::stages(&ws.windows, starting);
    let shown = crate::wm::watch_shown(mine);
    let delay = |i: usize| Duration::from_millis(workspace::tame_delay(ws.windows[i].delay_ms) as u64);
    let (mut stage, mut stage_began) = (0, Instant::now());
    let mut due: Vec<(usize, Instant)> = stages[0].iter().map(|&i| (i, stage_began + delay(i))).collect();
    let (mut looked, mut late) = (Instant::now(), Vec::new());
    let finished = loop {
        let now = Instant::now();
        for (i, _) in due.iter().filter(|(_, at)| *at <= now) {
            starts.start(ws, *i, mine, run.places, tally);
        }
        due.retain(|(_, at)| *at > now);
        // Each app's time runs from when it started.
        for (i, _) in starts.waiting.iter().filter(|(_, since)| since.elapsed() >= LAUNCH_WAIT) {
            tally.skipped.push((app_name(&ws.windows[*i]), TOOK_TOO_LONG.into()));
            late.push(*i);
            // Nothing catches its window after this, so the next switch matches the
            // row by app and binds the window that showed late (D336).
            if run.bound[*i].is_some() {
                unbind(&ws.id, *i);
            }
        }
        starts.waiting.retain(|(_, since)| since.elapsed() < LAUNCH_WAIT);
        let holds = |i: &usize| ws.windows[*i].wait_for_window && starts.is_waiting(*i);
        let held = stages.get(stage).is_some_and(|s| s.iter().any(holds));
        if stage < stages.len() && due.is_empty() && !held {
            let names: Vec<String> =
                stages[stage].iter().filter(|i| late.contains(*i)).map(|i| app_name(&ws.windows[*i])).collect();
            let names = if names.is_empty() { String::new() } else { format!(", too long: {}", names.join(", ")) };
            let (n, ms) = (stages[stage].len(), stage_began.elapsed().as_millis());
            log::info!("{}: stage {} of {}, {n} apps, {ms} ms{names}", ws.name, stage + 1, stages.len());
            stage += 1;
            stage_began = Instant::now();
            if let Some(next) = stages.get(stage) {
                due = next.iter().map(|&i| (i, stage_began + delay(i))).collect();
                continue;
            }
        }
        if stage >= stages.len() && starts.waiting.is_empty() {
            break true;
        }
        // Sleep until an app is due, a window shows, the next look, or an app's time runs out.
        let wake = due
            .iter()
            .map(|(_, at)| *at)
            .chain(starts.waiting.iter().map(|(_, since)| *since + LAUNCH_WAIT))
            .chain([looked + LAUNCH_POLL])
            .min()
            .unwrap_or(looked + LAUNCH_POLL);
        let left = wake.saturating_duration_since(Instant::now());
        let mut hwnds = Vec::new();
        match shown.recv_timeout(left) {
            Ok(h) => hwnds.push(h),
            // The window thread isn't there, so only the looks find windows.
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(left),
            Err(RecvTimeoutError::Timeout) => {}
        }
        hwnds.extend(shown.try_iter());
        if (run.cancelled)() {
            break false;
        }
        let cands: Vec<Candidate> = if looked.elapsed() >= LAUNCH_POLL {
            looked = Instant::now();
            snapshot(waits.procs).into_iter().map(|s| s.cand).collect()
        } else {
            let mut cands = Vec::new();
            for h in hwnds {
                let z = waits.known.len() + cands.len();
                cands.extend(seen_of(hwnd_of(h), z, &[], waits.procs, &mut |_, _| {}).map(|s| s.cand));
            }
            cands
        };
        if !cands.is_empty() {
            catch(run, cands, waits, starts, tally);
        }
    };
    crate::wm::unwatch_shown(mine);
    finished.then_some(())
}

/// The row's place, if it's one a switch can open: a folder on an Explorer row, a page
/// on a browser row. Any other counts as none.
pub(super) fn usable_place(rule: &Rule) -> Option<&Place> {
    let place = rule.place.as_ref()?;
    let kind = match place {
        Place::Folder(_) => places::Kind::Explorer,
        Place::Url(_) => places::Kind::Browser,
    };
    (places::kind_of(rule) == Some(kind)).then_some(place)
}
