//! Capturing the windows on screen into rules, on a worker.

use super::*;

// ---- Capture

pub struct CaptureJob {
    pub owner: isize,
    pub screens: Vec<Screen>,
    pub gap: u32,
    /// Lower-case exe names never captured: the zones' exclude list.
    pub exclude: Vec<String>,
    /// "Remember folders and pages": read each Explorer window's folder and each
    /// browser's page. Never for the last session.
    pub places: bool,
    /// "Update from screen removes apps that aren't open": the capture also names the
    /// other windows of the desk, which costs a look at their processes.
    pub removes: bool,
}

/// A capture's rules, the window each came from, and the other windows of the desk
/// the capture made no rule of: sticky ones, off every screen, and on other desktops.
type Captured = (Vec<Rule>, Vec<isize>, Vec<Candidate>);

static CAPTURED: Mutex<Option<Captured>> = Mutex::new(None);

static SESSION: Mutex<Option<Vec<Rule>>> = Mutex::new(None);

/// Start a capture. The main thread gets `WM_APP_CAPTURED` when it's done.
pub fn capture(job: CaptureJob) -> bool {
    let spawned = std::thread::Builder::new().name("seam-capture".into()).spawn(move || {
        let _com = Com::enter();
        let captured = capture_rules(&job, false);
        *CAPTURED.lock().unwrap_or_else(|e| e.into_inner()) = Some(captured);
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_CAPTURED, WPARAM(0), LPARAM(0));
        }
    });
    spawned.is_ok()
}

/// Read one window as a capture would, with its folder or page: the one "Add an app"
/// picked from the windows open now. The main thread gets `WM_APP_CAPTURED`, with no
/// rule when the window is gone or isn't one a capture takes.
pub fn capture_window(job: CaptureJob, hwnd: isize) -> bool {
    let spawned = std::thread::Builder::new().name("seam-capture".into()).spawn(move || {
        let _com = Com::enter();
        let picked = seen_of(hwnd_of(hwnd), 0, &job.exclude, &mut Procs::new(), &mut |_, _| {});
        let rule =
            |c: Candidate| Rule { exe: c.exe, aumid: c.aumid, class: c.class, title_hint: c.title, ..Rule::default() };
        let mut rules: Vec<Rule> = picked.into_iter().map(|s| rule(s.cand)).collect();
        let hwnds: Vec<isize> = rules.iter().map(|_| hwnd).collect();
        if job.places {
            let read = places::read(&rules, &hwnds);
            for (rule, place) in rules.iter_mut().zip(read) {
                rule.place = place;
            }
        }
        *CAPTURED.lock().unwrap_or_else(|e| e.into_inner()) = Some((rules, hwnds, Vec::new()));
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_CAPTURED, WPARAM(0), LPARAM(0));
        }
    });
    spawned.is_ok()
}

pub fn take_captured() -> Option<Captured> {
    CAPTURED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Capture the last session (D120): every window, pinned ones with their pins.
/// The main thread gets `WM_APP_SESSION` when it's done.
pub fn capture_session(job: CaptureJob) -> bool {
    let spawned = std::thread::Builder::new().name("seam-session".into()).spawn(move || {
        let _com = Com::enter();
        let (rules, ..) = capture_rules(&job, true);
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(rules);
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_SESSION, WPARAM(0), LPARAM(0));
        }
    });
    spawned.is_ok()
}

pub fn take_session() -> Option<Vec<Rule>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The session capture's result, waiting for it no longer than `limit`. Only for
/// the end of a Windows session, when there's no later message to wait for.
pub fn wait_session(limit: Duration) -> Option<Vec<Rule>> {
    let started = Instant::now();
    loop {
        if let Some(rules) = take_session() {
            return Some(rules);
        }
        if started.elapsed() >= limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The rules, the window each came from, and the other windows of the desk that make
/// none: an app with one of those is still open, for "Update from screen removes apps
/// that aren't open". Windows on other desktops and off every screen count; hidden ones
/// don't.
fn capture_rules(job: &CaptureJob, session: bool) -> Captured {
    let started = Instant::now();
    let geos = geometry(&job.screens, job.gap);
    let mut skips: BTreeMap<&'static str, usize> = BTreeMap::new();
    let (mut procs, mut seen, mut away) = (Procs::new(), Vec::new(), Vec::new());
    for h in winutil::top_level_windows() {
        let z = seen.len();
        let mut elsewhere = false;
        seen.extend(seen_of(h, z, &job.exclude, &mut procs, &mut |why, facts| {
            *skips.entry(why).or_default() += 1;
            elsewhere |= matches!(why, "cloaked" | "off screen");
            // Hidden and cloaked windows are most of them, so those are only counted.
            if !matches!(why, "hidden" | "cloaked") {
                log::info!("capture skipped {} {}: {why}", facts.class, facts.exe_name);
            }
        }));
        if elsewhere && job.removes && !session {
            away.push(h);
        }
    }
    let mut others: Vec<Candidate> = away
        .into_iter()
        .filter_map(|h| {
            let path = proc_of(&mut procs, winutil::process_id(h)).path.clone()?;
            let exe = winutil::exe_path_from(h, path)?;
            Some(Candidate {
                hwnd: h.0 as isize,
                exe,
                aumid: winutil::aumid(h),
                class: String::new(),
                title: String::new(),
                z: 0,
            })
        })
        .collect();
    let foreground = unsafe { GetForegroundWindow() }.0 as isize;
    let (mut rules, mut hwnds) = (Vec::new(), Vec::new());
    // A sticky window belongs to no workspace. The last session keeps it, pinned.
    for seen in seen {
        if !session && seen.pin & winutil::PIN_STICKY != 0 {
            if job.removes {
                others.push(seen.cand);
            }
            continue;
        }
        let h = hwnd_of(seen.cand.hwnd);
        let shown = shown_of(h);
        let normal = winutil::normal_rect(h);
        // A minimized window has no place on screen, so it counts where it would restore to.
        let visible = if shown == Shown::Minimized { normal } else { winutil::visible_rect(h) };
        let placed =
            visible.and_then(|v| geos.iter().find(|g| g.bounds.contains(centre(v).0, centre(v).1)).map(|g| (v, g)));
        let Some((visible, geo)) = placed else {
            if job.removes && !session {
                others.push(seen.cand);
            }
            continue;
        };
        let visible = match shown {
            Shown::Minimized => winutil::normal_frame(h, geo.dpi).unwrap_or(visible),
            _ => visible,
        };
        let target = match shown {
            Shown::Maximized => Target::State { state: WinState::Max },
            _ => workspace::target_of(visible, &geo.zones)
                .unwrap_or_else(|| Target::Rect { rect: workspace::to_fractions(visible, geo.work) }),
        };
        // Its size in pixels and millimetres, for a restore on another screen.
        let free = matches!(shown, Shown::Normal | Shown::Minimized);
        let size_px = free.then_some([visible.w, visible.h]);
        let size_mm = free.then(|| desk::size_mm(visible.w, visible.h, geo.raw_dpi)).flatten();
        let c = seen.cand;
        hwnds.push(c.hwnd);
        rules.push(Rule {
            exe: c.exe,
            aumid: c.aumid,
            class: c.class,
            title_hint: c.title,
            monitor: geo.id.clone(),
            target,
            normal_rect: normal.map(|n| workspace::to_fractions(n, geo.work)),
            launch: true,
            pin: if session { Pin::of_bits(winutil::pin_bits(h)) } else { None },
            shown,
            z: rules.len() as u32,
            focused: c.hwnd == foreground,
            size_px,
            size_mm,
            captured_on: Some(geo.id.clone()),
            ..Rule::default()
        });
    }
    if job.places && !session {
        let places = places::read(&rules, &hwnds);
        for (rule, place) in rules.iter_mut().zip(places) {
            rule.place = place;
        }
    }
    let skipped: Vec<String> = skips.iter().map(|(why, n)| format!("{why} {n}")).collect();
    let ms = started.elapsed().as_millis();
    log::info!("workspace capture: {} windows in {ms} ms, skipped {}", rules.len(), skipped.join(", "));
    (rules, hwnds, others)
}
