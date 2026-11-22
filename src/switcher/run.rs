//! One switch from start to finish: the job, its result and the worker.

use super::*;

// ---- Switch

pub struct SwitchJob {
    pub owner: isize,
    pub workspace: Workspace,
    pub screens: Vec<Screen>,
    pub gap: u32,
    /// Start nothing, and wait this long for the windows to appear, placing each as it
    /// does: a last session whose apps Windows reopens (D128).
    pub arrange_for: Option<Duration>,
    /// "Pinned windows stay when I switch workspaces".
    pub pins_stay: bool,
    /// "When a window moves to another screen".
    pub size_mode: WindowSize,
    /// "Open alongside": leave the other windows alone this once, whatever the workspace says.
    pub alongside: bool,
    /// "Remember folders and pages": an app this switch starts opens on its row's folder or page.
    pub places: bool,
    /// Snap back: windows already open are moved even when the workspace leaves them where they are.
    pub put_back: bool,
    /// The Seam window asked for it, which shows the result itself.
    pub from_panel: bool,
}

/// How a switch went: what the event says.
pub struct Switched {
    pub id: String,
    pub name: String,
    pub launched: usize,
    pub placed: usize,
    pub skipped: Vec<Skip>,
    /// The windows the rows put in one zone as a stack: the screen and zones, from 0,
    /// and the windows from the top.
    pub stacks: Vec<(Zones, Vec<isize>)>,
    /// The Seam window asked for it, so the tray stays quiet.
    pub from_panel: bool,
    /// The user stopped it while it started apps.
    pub cancelled: bool,
}

/// An app a switch couldn't start or move, or left alone on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip {
    /// The row, from 0, or `None` for Other windows.
    pub row: Option<usize>,
    pub app: String,
    pub reason: String,
}

impl Skip {
    pub(super) fn row(row: usize, rule: &Rule, reason: &str) -> Skip {
        Skip { row: Some(row), app: app_name(rule), reason: reason.into() }
    }

    pub(super) fn of(m: &Move, reason: &str) -> Skip {
        Skip { row: m.row, app: m.app.clone(), reason: reason.into() }
    }
}

/// A switch that begins starting apps, for the notice and the Esc hook.
pub struct Starting {
    pub generation: u64,
    pub id: String,
    pub name: String,
    pub apps: usize,
}

/// A screen's id and the first and last of its zones, from 0.
pub type Zones = (String, usize, usize);

/// Bumped by every switch, so a running one can see a newer one has replaced it.
static SWITCH_GEN: AtomicU64 = AtomicU64::new(0);

static SWITCHED: Mutex<Option<Switched>> = Mutex::new(None);

static STARTING: Mutex<Option<Starting>> = Mutex::new(None);

/// The switch whose start phase runs now, 0 for none.
static STARTING_GEN: AtomicU64 = AtomicU64::new(0);

/// The switch the user asked to stop.
static CANCEL: AtomicU64 = AtomicU64::new(0);

/// A switch starts the app it is at unless it was cancelled.
fn should_start(generation: u64, cancel_gen: u64) -> bool {
    generation != cancel_gen
}

/// Stop the switch that is starting apps: it starts nothing more and waits for nothing.
/// False when none is (D714).
pub fn cancel() -> bool {
    let running = STARTING_GEN.load(Ordering::SeqCst);
    if running != 0 {
        CANCEL.store(running, Ordering::SeqCst);
    }
    running != 0
}

pub fn take_starting() -> Option<Starting> {
    STARTING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The start phase begins: tell the tray, and let a cancel reach this switch.
pub(super) fn begin_starting(run: &Run, generation: u64, apps: usize) {
    let ws = run.ws;
    let (id, name) = (ws.id.clone(), ws.name.clone());
    *STARTING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Starting { generation, id, name, apps });
    STARTING_GEN.store(generation, Ordering::SeqCst);
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(run.owner)), WM_APP_SWITCH_STARTING, WPARAM(generation as usize), LPARAM(0));
    }
}

/// The start phase is over, however it ended.
pub(super) fn end_starting(run: &Run, generation: u64) {
    let _ = STARTING_GEN.compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst);
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(run.owner)), WM_APP_SWITCH_STARTED, WPARAM(generation as usize), LPARAM(0));
    }
}

/// Whether the user stopped this switch.
pub(super) fn stopped(generation: u64) -> bool {
    !should_start(generation, CANCEL.load(Ordering::SeqCst))
}

/// Switches running now. A replaced one counts until its thread has stopped.
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Counts a switch as running for as long as it lives.
struct Running;

impl Running {
    fn enter() -> Running {
        RUNNING.fetch_add(1, Ordering::SeqCst);
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A switch is placing windows, so auto-apply keeps out of its way.
pub fn switching() -> bool {
    RUNNING.load(Ordering::SeqCst) > 0
}

/// Start a switch, replacing one that is still running. The main thread gets
/// `WM_APP_SWITCHED` when it's done, and nothing if a newer switch took over.
pub fn switch(job: SwitchJob) -> bool {
    let (start, started) = std::sync::mpsc::channel::<u64>();
    let spawned = std::thread::Builder::new().name("seam-switch".into()).spawn(move || {
        // Waits until the thread really exists, so a failed spawn replaces nothing.
        let Ok(mine) = started.recv() else { return };
        let running = Running::enter();
        let _com = Com::enter();
        let result = run_switch(&job, mine);
        // Over before the tray hears, so auto-apply can take the windows that opened meanwhile.
        drop(running);
        let Some(result) = result else { return };
        if SWITCH_GEN.load(Ordering::SeqCst) != mine {
            return;
        }
        *SWITCHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_SWITCHED, WPARAM(0), LPARAM(0));
        }
    });
    if spawned.is_err() {
        return false;
    }
    let _ = start.send(SWITCH_GEN.fetch_add(1, Ordering::SeqCst) + 1);
    true
}

pub fn take_switched() -> Option<Switched> {
    SWITCHED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Does the whole switch. Returns `None` if a newer one replaced it.
fn run_switch(job: &SwitchJob, mine: u64) -> Option<Switched> {
    let begun = Instant::now();
    let cancelled = || SWITCH_GEN.load(Ordering::SeqCst) != mine;
    let ws = &job.workspace;
    let stamp = (ws.id != SESSION_ID).then(|| workspace::stamp_key(&ws.id));
    let geos = geometry(&job.screens, job.gap);
    let (pins_stay, size_mode, places) = (job.pins_stay, job.size_mode, job.places);
    let (bound, elsewhere) = bindings(ws, places);
    let put_back = job.put_back;
    let run =
        Run { owner: job.owner, ws, geos, pins_stay, size_mode, cancelled: &cancelled, stamp, places, put_back, bound };
    let mut tally = Tally::default();
    let mut procs = Procs::new();

    // Snapshot and match.
    let seen = snapshot(&mut procs);
    let mut known: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let mut found = workspace::assign_bound(&ws.windows, &known, &run.bound, &elsewhere);
    bind_unbound(&run, &found);
    if let Some(key) = stamp {
        clear_stale_stamps(key, &found, &seen);
    }
    let mut starts = Starts::default();

    let mut waits = Waits { found: &mut found, known: &mut known, procs: &mut procs, seen: &seen };
    if let Some(wait) = job.arrange_for {
        arrange_as_they_open(&run, wait, &mut waits, &elsewhere, &mut tally)?;
    } else {
        place_then_start(&run, mine, &mut waits, &mut starts, &mut tally)?;
    }
    if cancelled() {
        return None;
    }

    // Stacking and focus last. A switch that waited up to a minute would take them from
    // whatever the user opened since.
    if job.arrange_for.is_none() {
        restack_and_focus(&tally);
    }

    // Other windows on this desk's screens: only ones that were there before the switch began,
    // and never anything that belongs to an app this workspace has or has started, whose
    // splash screens, updaters and real main window may open late.
    // A cancelled switch leaves them alone: the user stopped it (D714).
    let other_windows = if job.alongside || tally.cancelled { OtherWindows::Leave } else { ws.other_windows };
    if other_windows != OtherWindows::Leave && found.iter().all(Option::is_none) {
        // Without a single match the workspace says nothing about what is on screen.
        tally.skipped.push(Skip { row: None, app: "Other windows".into(), reason: "nothing matched".into() });
    } else if other_windows != OtherWindows::Leave {
        deal_with_others(job, &run, other_windows, (&found, &known, &seen), &starts);
    }

    log::info!(
        "{}: switched in {} ms, plan {} ms, {} moves, {} already in place",
        ws.name,
        begun.elapsed().as_millis(),
        tally.plan.as_millis(),
        tally.moves,
        tally.in_place
    );
    let launched = starts.launched;
    let stacks = stacks_of(ws, &found);
    let (id, name, placed, skipped) = (ws.id.clone(), ws.name.clone(), tally.placed, tally.skipped);
    let cancelled = tally.cancelled;
    Some(Switched { id, name, launched, placed, skipped, stacks, from_panel: job.from_panel, cancelled })
}

/// The rows with a place in a stack, whose windows were found, grouped by screen and zones.
fn stacks_of(ws: &Workspace, found: &[Option<isize>]) -> Vec<(Zones, Vec<isize>)> {
    let row = |(rule, hwnd): (&Rule, &Option<isize>)| {
        let (rank, _) = rule.stack.zip(*hwnd)?;
        let Target::Zone { zone, span_to } = rule.target else { return None };
        let from = zone.checked_sub(1)? as usize;
        let to = span_to.and_then(|t| t.checked_sub(1)).map_or(from, |t| t as usize);
        Some(((rule.monitor.clone(), from, to.max(from)), rank))
    };
    let rows: Vec<Option<(Zones, u32)>> = ws.windows.iter().zip(found).map(row).collect();
    let groups = stack::groups(&rows);
    groups
        .into_iter()
        .filter_map(|group| {
            let zone = rows[*group.first()?].as_ref()?.0.clone();
            Some((zone, group.iter().filter_map(|i| found[*i]).collect()))
        })
        .collect()
}

/// An unbound place row keeps the window it matched by app from now on.
fn bind_unbound(run: &Run, found: &[Option<isize>]) {
    let ws = run.ws;
    for (i, b) in run.bound.iter().enumerate() {
        let app = app_name(&ws.windows[i]);
        match (b, found[i]) {
            (Some(Bound::Alive(_)), _) => log::info!("{app}: its window is open"),
            (Some(Bound::Unbound), hit) => {
                log::info!("{app}: not bound yet, matching by app");
                if let Some(hwnd) = hit {
                    bind(&ws.id, i, hwnd);
                }
            }
            _ => {}
        }
    }
}

/// This workspace's stamps on windows it didn't match this time are out of date.
fn clear_stale_stamps(key: u32, found: &[Option<isize>], seen: &[Seen]) {
    let matched: HashSet<isize> = found.iter().flatten().copied().collect();
    for s in seen.iter().filter(|s| !matched.contains(&s.cand.hwnd)) {
        let h = hwnd_of(s.cand.hwnd);
        if winutil::ws_stamp(h).is_some_and(|(k, _)| k == key) {
            winutil::clear_ws_stamp(h);
        }
    }
}

/// Something else opens the windows, so place each one as it appears, once per rule.
fn arrange_as_they_open(
    run: &Run,
    wait: Duration,
    waits: &mut Waits,
    elsewhere: &HashSet<isize>,
    tally: &mut Tally,
) -> Option<()> {
    let ws = run.ws;
    let mut done = vec![false; ws.windows.len()];
    let since = Instant::now();
    loop {
        let ready: Vec<usize> = (0..done.len()).filter(|i| !done[*i] && waits.found[*i].is_some()).collect();
        if !ready.is_empty() {
            place_rules(run, waits.found, &ready, waits.seen, tally);
            ready.iter().for_each(|i| done[*i] = true);
        }
        if done.iter().all(|d| *d) || since.elapsed() >= wait {
            break;
        }
        std::thread::sleep(ARRANGE_POLL);
        if (run.cancelled)() {
            return None;
        }
        let claimed: HashSet<isize> = waits.found.iter().flatten().copied().collect();
        let fresh: Vec<Candidate> =
            snapshot(waits.procs).into_iter().map(|s| s.cand).filter(|c| !claimed.contains(&c.hwnd)).collect();
        let missing: Vec<usize> = (0..done.len()).filter(|i| waits.found[*i].is_none()).collect();
        let rules: Vec<Rule> = missing.iter().map(|i| ws.windows[*i].clone()).collect();
        let bound: Vec<Option<Bound>> = missing.iter().map(|i| run.bound[*i]).collect();
        for (i, hit) in missing.into_iter().zip(workspace::assign_bound(&rules, &fresh, &bound, elsewhere)) {
            waits.found[i] = hit;
        }
    }
    let found = &*waits.found;
    let absent: Vec<(usize, &Rule)> =
        ws.windows.iter().enumerate().zip(found).filter(|(_, h)| h.is_none()).map(|(r, _)| r).collect();
    let names: Vec<String> = absent.iter().map(|(_, r)| app_name(r)).collect();
    let shown = found.iter().flatten().count();
    log::info!("{}: placed {shown} of {}, never showed: {}", ws.name, found.len(), names.join(", "));
    let mut background = Background::default();
    for (i, rule) in absent {
        let why = if background.runs(rule, waits.procs) { RUNS_IN_BACKGROUND } else { "timeout" };
        tally.skipped.push(Skip::row(i, rule, why));
    }
    Some(())
}

/// What is running already goes in one pass, then the missing apps start in stages.
fn place_then_start(run: &Run, mine: u64, waits: &mut Waits, starts: &mut Starts, tally: &mut Tally) -> Option<()> {
    let ws = run.ws;
    let action = |i: usize| workspace::row_action(&ws.windows[i], waits.found[i], ws.open_windows, run.put_back);
    let running: Vec<usize> = (0..waits.found.len()).filter(|i| action(*i) == RowAction::Place).collect();
    // A window left where it is still has its row, so auto-apply doesn't give the row to a second window.
    if let Some(key) = run.stamp {
        let left = (0..waits.found.len()).filter(|i| action(*i) == RowAction::Leave);
        for i in left {
            if let Some(hwnd) = waits.found[i] {
                winutil::set_ws_stamp(hwnd_of(hwnd), key, i);
            }
        }
    }
    place_rules(run, waits.found, &running, waits.seen, tally);
    if (run.cancelled)() {
        return None;
    }
    // A place row whose window is open but not one a switch takes (on another
    // desktop, say) isn't started a second time.
    let open = |i: usize| matches!(run.bound[i], Some(Bound::Alive(_)));
    // Snap back puts windows back and never starts an item that wasn't running (D740).
    let item_left = |i: usize| run.put_back && ws.windows[i].item.is_some();
    let wanted =
        |i: usize| (ws.launch_missing && action(i) == RowAction::Start && !open(i) && !item_left(i)).then_some(i);
    let starting: Vec<usize> = (0..ws.windows.len()).filter_map(wanted).collect();
    for &i in starting.iter().filter(|i| run.bound[**i] == Some(Bound::Gone)) {
        let (app, kind) = (app_name(&ws.windows[i]), usable_place(&ws.windows[i]).map_or("place", place_kind));
        log::info!("{app}: its window was closed, starting on its saved {kind}");
    }
    if !starting.is_empty() {
        run_stages(run, mine, &starting, waits, starts, tally)?;
    }
    // An app this switch doesn't start and that has no window is reported when its
    // program still runs, closed to the tray.
    let mut background = Background::default();
    let idle = ws
        .windows
        .iter()
        .enumerate()
        .filter(|(i, r)| workspace::takes_part(r) && waits.found[*i].is_none() && !starting.contains(i));
    for (i, rule) in idle {
        if background.runs(rule, waits.procs) {
            tally.skipped.push(Skip::row(i, rule, RUNS_IN_BACKGROUND));
        }
    }
    Some(())
}

fn restack_and_focus(tally: &Tally) {
    restack(&tally.stack, tally.moves > 0);
    // The tray has foreground rights after a hotkey; otherwise the focus may be refused, and then it stays.
    let foreground = unsafe { GetForegroundWindow() }.0 as isize;
    if let Some(hwnd) = to_focus(&tally.stack).filter(|h| *h != foreground) {
        wincontrol::focus(hwnd_of(hwnd), true);
    }
}

/// Minimizes or closes the windows on this desk's screens that the workspace doesn't use.
fn deal_with_others(
    job: &SwitchJob,
    run: &Run,
    other_windows: OtherWindows,
    (found, known, seen): (&[Option<isize>], &[Candidate], &[Seen]),
    starts: &Starts,
) {
    let ws = run.ws;
    let claimed: HashSet<isize> = found.iter().flatten().copied().collect();
    let (mut exes, mut aumids): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    // A place row whose window is open but out of reach (on another desktop) still has its app here.
    let open = |i: usize| matches!(run.bound[i], Some(Bound::Alive(_)));
    let mine_rules = ws
        .windows
        .iter()
        .enumerate()
        .filter(|(i, r)| r.off || found[*i].is_some() || starts.started.contains(i) || open(*i));
    for (_, rule) in mine_rules {
        exes.insert(rule.exe.to_lowercase());
        aumids.extend(rule.aumid.clone());
    }
    for c in known.iter().filter(|c| claimed.contains(&c.hwnd)) {
        exes.insert(c.exe.to_lowercase());
        aumids.extend(c.aumid.clone());
    }
    let others = seen.iter().filter(|s| !s.elevated && !stays(s.pin, job.pins_stay) && !claimed.contains(&s.cand.hwnd));
    for s in others.filter(|s| !exes.contains(&s.cand.exe.to_lowercase())) {
        if s.cand.aumid.as_ref().is_some_and(|a| aumids.contains(a)) {
            continue;
        }
        let h = hwnd_of(s.cand.hwnd);
        // A dialog of a claimed window, like a save prompt, goes with its owner.
        let owner = unsafe { GetWindow(h, GW_OWNER) }.ok().filter(|o| !o.is_invalid());
        if owner.is_some_and(|o| claimed.contains(&(o.0 as isize))) || !unsafe { IsWindow(Some(h)) }.as_bool() {
            continue;
        }
        let on_desk = winutil::window_rect(h)
            .is_some_and(|r| run.geos.iter().any(|g| g.bounds.contains(centre(r).0, centre(r).1)));
        if !on_desk {
            continue;
        }
        // Asks only. The app decides, so one with unsaved work keeps its window.
        unsafe {
            match other_windows {
                OtherWindows::Minimize => {
                    let _ = ShowWindowAsync(h, SW_MINIMIZE);
                }
                OtherWindows::Close => {
                    // Seam closing a pinned window isn't the news D119 tells about.
                    if winutil::pin_bits(h) != 0 {
                        crate::wm::closing(s.cand.hwnd);
                    }
                    let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                OtherWindows::Leave => {}
            }
        }
    }
}

/// The windows a workspace doesn't use, topmost first, matched as a switch does:
/// the rows' windows and other windows of the same apps stay out of it, and so do
/// elevated windows and ones that stay (sticky, or pinned on top).
pub fn outside(ws: &Workspace, exclude: &[String], remember: bool) -> Result<Vec<isize>, &'static str> {
    let seen = snapshot_with(exclude, &mut Procs::new(), &mut |_, _| {});
    let known: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let found = match_rows(ws, &known, remember);
    if found.iter().all(Option::is_none) {
        return Err("Nothing from this workspace is open");
    }
    let claimed: HashSet<isize> = found.iter().flatten().copied().collect();
    let (mut exes, mut aumids): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    for c in known.iter().filter(|c| claimed.contains(&c.hwnd)) {
        exes.insert(c.exe.to_lowercase());
        aumids.extend(c.aumid.clone());
    }
    let others = seen.iter().filter(|s| !s.elevated && !stays(s.pin, true) && !claimed.contains(&s.cand.hwnd));
    let apart = others.filter(|s| {
        !exes.contains(&s.cand.exe.to_lowercase()) && !s.cand.aumid.as_ref().is_some_and(|a| aumids.contains(a))
    });
    Ok(apart.map(|s| s.cand.hwnd).collect())
}

/// Pin a window as a rule says. The seam-wm thread does it and logs how it went.
pub(super) fn pin_again(hwnd: isize, pin: Pin) {
    if pin.bits() & winutil::PIN_TOP != 0 {
        crate::wm::pin(PinKind::Top, Some(true), hwnd, None);
    }
    if pin.bits() & winutil::PIN_STICKY != 0 {
        crate::wm::pin(PinKind::Sticky, Some(true), hwnd, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_stops_only_the_switch_it_names() {
        // Nothing cancelled yet, or a cancel for an earlier switch.
        assert!(should_start(5, 0));
        assert!(should_start(5, 4));
        // The switch the cancel names stops starting, and stays stopped.
        assert!(!should_start(5, 5));
        // A newer switch isn't held by an older cancel.
        assert!(should_start(6, 5));
    }
}
