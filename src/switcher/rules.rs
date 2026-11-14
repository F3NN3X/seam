//! App rules: placing and pinning windows by rule, and reopening closed pins.

use super::*;

/// The app rules, with what placing a window by one needs (D270).
pub struct AppRules {
    pub rules: Vec<Rule>,
    pub screens: Vec<Screen>,
    pub gap: u32,
    pub pins_stay: bool,
    /// Lower-case exe names Seam never manages: a rule doesn't apply to them.
    pub exclude: Vec<String>,
    pub size_mode: WindowSize,
    /// "Remember folders and pages", for whether the workspace in use owns a window.
    pub places: bool,
}

/// The app rule that applies to a window: not an administrator window, not one a
/// switch leaves alone, not one the workspace in use owns. Its place in `set.rules`.
fn rule_hit(set: &AppRules, owner: Option<&Workspace>, h: HWND, procs: &mut Procs) -> Option<usize> {
    let seen = seen_of(h, 0, &set.exclude, procs, &mut |_, _| {})?;
    if seen.elevated || stays(seen.pin, set.pins_stay) {
        return None;
    }
    // The workspace in use owns every window of an app it has a row for.
    let cand = std::slice::from_ref(&seen.cand);
    if owner.is_some_and(|ws| match_rows(ws, cand, set.places).iter().any(Option::is_some)) {
        return None;
    }
    workspace::assign(&set.rules, cand).iter().position(Option::is_some)
}

/// A window the seam-wm thread refused to pin, by handle and its process's start time,
/// so a reused handle isn't skipped. The passes leave it alone; its next show gets one
/// more try (`retried`), and then it is left alone for good.
struct Refused {
    hwnd: isize,
    started: u64,
    retried: bool,
}

static REFUSED: Mutex<Vec<Refused>> = Mutex::new(Vec::new());
/// Refused windows remembered before the oldest goes.
const MAX_REFUSED: usize = 64;

fn started_of(h: HWND) -> u64 {
    seam::process::started(winutil::process_id(h)).unwrap_or(0)
}

/// Forget a window's refusal: it took the pin, or its rule changed.
fn forgive(h: HWND) {
    REFUSED.lock().unwrap_or_else(|e| e.into_inner()).retain(|r| r.hwnd != h.0 as isize);
}

/// Whether to leave a window alone because the wm refused it. A `retry` (the window was
/// just shown) is let through once.
fn is_refused(h: HWND, retry: bool) -> bool {
    let (key, started) = (h.0 as isize, started_of(h));
    let mut list = REFUSED.lock().unwrap_or_else(|e| e.into_inner());
    // Another window under the same handle isn't the one that was refused.
    list.retain(|r| r.hwnd != key || r.started == started);
    match list.iter_mut().find(|r| r.hwnd == key) {
        Some(r) if retry && !r.retried => {
            r.retried = true;
            false
        }
        Some(_) => true,
        None => false,
    }
}

/// Remember that the wm refused this window, and say so once.
fn refuse(h: HWND, rule: &Rule) {
    let mut list = REFUSED.lock().unwrap_or_else(|e| e.into_inner());
    // A retry that was refused again is already in the list and already said.
    if list.iter().any(|r| r.hwnd == h.0 as isize) {
        return;
    }
    if list.len() >= MAX_REFUSED {
        // Windows that closed, or whose handle another one took, go before the oldest.
        list.retain(|r| {
            let gone = hwnd_of(r.hwnd);
            unsafe { IsWindow(Some(gone)) }.as_bool() && started_of(gone) == r.started
        });
    }
    if list.len() >= MAX_REFUSED {
        list.remove(0);
    }
    list.push(Refused { hwnd: h.0 as isize, started: started_of(h), retried: false });
    log::info!("app rules: the window thread refused to pin {}, so the passes stop asking", app_name(rule));
}

/// Pin a window as its rule says, unless the rule did already. What the rule pinned
/// stays on the window, so after Release or an unpin by hand the rule leaves it alone
/// until it's opened anew or the rule gains a pin (`fresh`). Only a pin the seam-wm
/// thread applied is kept and counts. Whether it pinned. `stalled` is set when that
/// thread didn't answer, and then no more is asked. A pin that thread refuses isn't
/// asked for again on a pass, only once more when the window is shown (`retry`).
fn pin_by_marker(h: HWND, rule: &Rule, fresh: bool, retry: bool, stalled: &mut bool) -> bool {
    if fresh {
        winutil::set_rule_pin_bits(h, 0);
        forgive(h);
    }
    let done = winutil::rule_pin_bits(h);
    let want = rule.pin.map_or(0, |p| p.bits());
    let skip = want & !done != 0 && is_refused(h, retry);
    let mut now = done;
    for (kind, bit) in [(PinKind::Top, winutil::PIN_TOP), (PinKind::Sticky, winutil::PIN_STICKY)] {
        if !skip && want & !done & bit != 0 && pinned_by_wm(kind, h, stalled) {
            now |= bit;
        }
    }
    if !skip && !*stalled && want & !now != 0 {
        refuse(h, rule);
    } else if want & !now == 0 {
        forgive(h);
    }
    winutil::set_rule_pin_bits(h, now);
    now != done
}

/// Ask the seam-wm thread to pin a window and wait for its answer.
fn pinned_by_wm(kind: PinKind, h: HWND, stalled: &mut bool) -> bool {
    if *stalled {
        return false;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    crate::wm::pin(kind, Some(true), h.0 as isize, Some(tx));
    let answer = rx.recv_timeout(Duration::from_secs(1));
    *stalled = answer.is_err();
    matches!(answer, Ok(Ok(_)))
}

/// Put a window that just opened where its app's rule puts it, once, then pin it if
/// the rule says so. A rule whose screen isn't connected leaves the window where it
/// is. Returns what happened, for the log, or `None` for a window no rule applies to.
pub fn place_by_rule(set: &AppRules, owner: Option<&Workspace>, hwnd: isize) -> Option<String> {
    let started = Instant::now();
    let h = hwnd_of(hwnd);
    let valid = unsafe { IsWindow(Some(h)) }.as_bool();
    if !valid || winutil::ws_stamp(h).is_some() || winutil::is_minimized(h) || switching() {
        return None;
    }
    let _com = Com::enter();
    let hit = rule_hit(set, owner, h, &mut Procs::new())?;
    let rule = &set.rules[hit];
    let geos = geometry(&set.screens, set.gap);
    let mut done = Vec::new();
    if rule.moves() && geos.iter().any(|g| g.id == rule.monitor) {
        let moved = move_opened(h, rule, &geos, set.size_mode);
        // Stamped either way, so it's never placed again. No workspace has this key.
        winutil::set_ws_stamp(h, workspace::stamp_key("app_rule"), hit);
        done.push(if moved { "placed" } else { "already in place" });
    }
    if pin_by_marker(h, rule, false, true, &mut false) {
        done.push("pinned");
    }
    if done.is_empty() {
        return None;
    }
    Some(format!("{}: {} in {} ms", app_name(rule), done.join(", "), started.elapsed().as_millis()))
}

/// Pin the open windows the rules say to keep on top: for a rule made or changed while
/// its app was open, and after a pause. Nothing is moved. The windows of a `fresh` rule
/// are pinned even if they were released. Returns how many it pinned.
pub fn pin_by_rules(set: &AppRules, owner: Option<&Workspace>, fresh: &[Rule]) -> usize {
    if switching() {
        return 0;
    }
    let _com = Com::enter();
    let mut procs = Procs::new();
    let (mut pinned, mut stalled) = (0, false);
    for h in winutil::top_level_windows() {
        if stalled {
            log::info!("app rules: the window thread didn't answer, pass stopped");
            break;
        }
        // A workspace that isn't in use placed it, and the rule's own stamp doesn't count.
        if winutil::ws_stamp(h).is_some_and(|(key, _)| key != workspace::stamp_key("app_rule")) {
            continue;
        }
        let hit = rule_hit(set, owner, h, &mut procs);
        pinned += usize::from(
            hit.is_some_and(|i| pin_by_marker(h, &set.rules[i], fresh.contains(&set.rules[i]), false, &mut stalled)),
        );
    }
    pinned
}

// ---- Reopen (D119)

/// What the notice calls the app of a pinned window that closed.
pub fn closed_name(gone: &Pinned) -> String {
    app_name(&Rule { exe: gone.exe.clone(), title_hint: gone.title.clone(), ..Rule::default() })
}

/// Start the app of a pinned window that closed, the way a workspace starts a missing
/// app, then pin its first new window the same way. Runs on a worker and only logs.
pub fn reopen(gone: Pinned) -> bool {
    let spawned = std::thread::Builder::new().name("seam-reopen".into()).spawn(move || {
        let _com = Com::enter();
        let rule = Rule {
            exe: gone.exe.clone(),
            aumid: gone.aumid.clone(),
            class: gone.class.clone(),
            title_hint: gone.title.clone(),
            ..Rule::default()
        };
        let app = app_name(&rule);
        // Only a window that wasn't there before the launch counts as the new one.
        let mut procs = Procs::new();
        let before: HashSet<isize> = snapshot(&mut procs).into_iter().map(|s| s.cand.hwnd).collect();
        if let Err(reason) = launch(&rule, false) {
            log::info!("reopen {app}: not started ({reason})");
            return;
        }
        let started = Instant::now();
        while started.elapsed() < LAUNCH_WAIT {
            std::thread::sleep(POLL);
            let fresh: Vec<Candidate> =
                snapshot(&mut procs).into_iter().map(|s| s.cand).filter(|c| !before.contains(&c.hwnd)).collect();
            if let Some(Some(hwnd)) = workspace::assign(std::slice::from_ref(&rule), &fresh).first() {
                log::info!("reopen {app}: started, pinning its new window");
                if let Some(pin) = Pin::of_bits(gone.bits) {
                    pin_again(*hwnd, pin);
                }
                return;
            }
        }
        log::info!("reopen {app}: no window within {} s", LAUNCH_WAIT.as_secs());
    });
    spawned.is_ok()
}
