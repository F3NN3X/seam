//! Workspaces on the tray's side: capture the windows on screen, and switch to
//! a saved set. Both run on worker threads and post one message back. Nothing
//! here waits on another app from the main thread, and every call that could
//! wait on a hung app is the asynchronous one (`ShowWindowAsync`, `PostMessageW`,
//! `SWP_ASYNCWINDOWPOS`) or is skipped for a window that has stopped answering.

use crate::log;
use crate::wm::Pinned;
use seam::api::PinKind;
use seam::winutil;
use seam::workspace::{self, Candidate, OtherWindows, Pin, Rule, Target, WinState, Workspace};
use seam::zones::{self, Rect, ZoneLayout};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A worker finished a switch.
pub const WM_APP_SWITCHED: u32 = WM_APP + 16;
/// A worker finished a capture.
pub const WM_APP_CAPTURED: u32 = WM_APP + 17;
/// A worker finished capturing the last session.
pub const WM_APP_SESSION: u32 = WM_APP + 25;

/// How long a started app gets to show its window.
const LAUNCH_WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(250);
/// Time for an app to react to a move before the next step.
const SETTLE: Duration = Duration::from_millis(120);
/// A window this close to where it was put counts as there.
const CLOSE_ENOUGH: i32 = 4;
/// The gap slider's end, as in the snapping.
const MAX_GAP: u32 = 32;

/// A connected screen and its zones, as the tray knows them.
pub struct Screen {
    pub id: String,
    pub bounds: Rect,
    pub layout: Option<ZoneLayout>,
}

/// A screen with its work area and zones in pixels, worked out when a job runs so a moved taskbar counts.
struct Geo {
    id: String,
    bounds: Rect,
    work: Rect,
    dpi: u32,
    zones: Vec<Rect>,
}

fn geometry(screens: &[Screen], gap: u32) -> Vec<Geo> {
    let placed = screens.iter().filter_map(|s| {
        let (work, dpi) = winutil::work_area(s.bounds)?;
        let gap = (gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        let zones = s.layout.as_ref().map(|l| zones::rects(&l.tree, work, gap)).unwrap_or_default();
        Some(Geo { id: s.id.clone(), bounds: s.bounds, work, dpi, zones })
    });
    placed.collect()
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// Windows for COM calls (the property store, `ShellExecuteW`) on a worker.
struct Com;

impl Com {
    fn enter() -> Com {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Com
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// A capturable window, with what the matcher and the placement need to know.
struct Seen {
    cand: Candidate,
    elevated: bool,
    /// Pinned as sticky, so a switch never moves, minimizes or closes it (D106).
    sticky: bool,
}

fn is_seam(exe: &str) -> bool {
    let name = exe.rsplit('\\').next().unwrap_or(exe).to_lowercase();
    name == "seam.exe" || name == "seam-settings.exe"
}

/// The windows a workspace may use, topmost first. Seam's own are never among them.
fn snapshot() -> Vec<Seen> {
    let own = std::process::id();
    let capturable = winutil::top_level_windows().into_iter().filter(|h| winutil::is_capturable(*h, own));
    let mut seen = Vec::new();
    for h in capturable {
        let Some(exe) = winutil::exe_path(h).filter(|exe| !is_seam(exe)) else { continue };
        let cand = Candidate {
            hwnd: h.0 as isize,
            exe,
            aumid: winutil::aumid(h),
            class: winutil::class_name(h),
            title: winutil::title(h),
            z: seen.len(),
        };
        seen.push(Seen { cand, elevated: winutil::is_elevated_over_us(h), sticky: winutil::is_sticky(h) });
    }
    seen
}

/// What to call an app in a message: its exe's name, or failing that its window title.
fn app_name(rule: &Rule) -> String {
    let stem = Path::new(&rule.exe).file_stem().map(|s| s.to_string_lossy().into_owned());
    stem.filter(|s| !s.is_empty()).unwrap_or_else(|| rule.title_hint.clone())
}

// ---- Capture

pub struct CaptureJob {
    pub owner: isize,
    pub screens: Vec<Screen>,
    pub gap: u32,
}

static CAPTURED: Mutex<Option<Vec<Rule>>> = Mutex::new(None);
static SESSION: Mutex<Option<Vec<Rule>>> = Mutex::new(None);

/// Start a capture. The main thread gets `WM_APP_CAPTURED` when it's done.
pub fn capture(job: CaptureJob) -> bool {
    let spawned = std::thread::Builder::new().name("seam-capture".into()).spawn(move || {
        let _com = Com::enter();
        let rules = capture_rules(&job, false);
        log::info!("workspace capture: {} windows", rules.len());
        *CAPTURED.lock().unwrap_or_else(|e| e.into_inner()) = Some(rules);
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(job.owner)), WM_APP_CAPTURED, WPARAM(0), LPARAM(0));
        }
    });
    spawned.is_ok()
}

pub fn take_captured() -> Option<Vec<Rule>> {
    CAPTURED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Capture the last session (D120): every window, pinned ones with their pins.
/// The main thread gets `WM_APP_SESSION` when it's done.
pub fn capture_session(job: CaptureJob) -> bool {
    let spawned = std::thread::Builder::new().name("seam-session".into()).spawn(move || {
        let _com = Com::enter();
        let rules = capture_rules(&job, true);
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

fn capture_rules(job: &CaptureJob, session: bool) -> Vec<Rule> {
    let geos = geometry(&job.screens, job.gap);
    let mut rules = Vec::new();
    // A sticky window belongs to no workspace. The last session keeps it, pinned.
    for seen in snapshot().into_iter().filter(|s| session || !s.sticky) {
        let h = hwnd_of(seen.cand.hwnd);
        let normal = winutil::normal_rect(h);
        // A minimized window has no place on screen, so it counts where it would restore to.
        let visible = if winutil::is_minimized(h) { normal } else { winutil::visible_rect(h) };
        let Some(visible) = visible else { continue };
        let Some(geo) = geos.iter().find(|g| g.bounds.contains(centre(visible).0, centre(visible).1)) else {
            continue;
        };
        let target = workspace::target_of(visible, &geo.zones).unwrap_or_else(|| {
            if winutil::is_maximized(h) {
                Target::State { state: WinState::Max }
            } else {
                Target::Rect { rect: workspace::to_fractions(visible, geo.work) }
            }
        });
        let c = seen.cand;
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
        });
    }
    rules
}

// ---- Switch

pub struct SwitchJob {
    pub owner: isize,
    pub workspace: Workspace,
    pub screens: Vec<Screen>,
    pub gap: u32,
}

/// How a switch went: what the event says.
pub struct Switched {
    pub id: String,
    pub name: String,
    pub launched: usize,
    pub placed: usize,
    /// App and reason.
    pub skipped: Vec<(String, String)>,
}

/// Bumped by every switch, so a running one can see a newer one has replaced it.
static SWITCH_GEN: AtomicU64 = AtomicU64::new(0);
static SWITCHED: Mutex<Option<Switched>> = Mutex::new(None);

/// Start a switch, replacing one that is still running. The main thread gets
/// `WM_APP_SWITCHED` when it's done, and nothing if a newer switch took over.
pub fn switch(job: SwitchJob) -> bool {
    let (start, started) = std::sync::mpsc::channel::<u64>();
    let spawned = std::thread::Builder::new().name("seam-switch".into()).spawn(move || {
        // Waits until the thread really exists, so a failed spawn replaces nothing.
        let Ok(mine) = started.recv() else { return };
        let _com = Com::enter();
        let Some(result) = run_switch(&job, mine) else { return };
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

/// Apps a switch has started and is still waiting on: key, when, and which switch. A
/// newer switch that replaces it waits for these instead of starting a second copy.
static LAUNCHES: Mutex<Vec<(String, Instant, u64)>> = Mutex::new(Vec::new());

fn launch_key(rule: &Rule) -> String {
    match rule.aumid.as_deref().filter(|a| workspace::safe_aumid(a)) {
        Some(aumid) => aumid.to_string(),
        None => rule.exe.to_lowercase(),
    }
}

/// An earlier switch's launch of this app that no rule of this switch has taken over yet.
fn earlier_launch(key: &str, mine: u64, taken: &mut Vec<usize>) -> Option<Instant> {
    let mut launches = LAUNCHES.lock().unwrap_or_else(|e| e.into_inner());
    launches.retain(|(_, at, _)| at.elapsed() < LAUNCH_WAIT);
    let i = launches.iter().enumerate().position(|(i, (k, _, gen))| k == key && *gen != mine && !taken.contains(&i))?;
    taken.push(i);
    Some(launches[i].1)
}

fn note_launch(key: String, mine: u64) {
    LAUNCHES.lock().unwrap_or_else(|e| e.into_inner()).push((key, Instant::now(), mine));
}

pub fn take_switched() -> Option<Switched> {
    SWITCHED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Explorer by its full path, so a copy of it in the app folder never wins.
fn explorer_path() -> String {
    let mut buf = [0u16; 260];
    let len = unsafe { GetWindowsDirectoryW(Some(&mut buf)) } as usize;
    match len {
        1..=259 => format!(r"{}\explorer.exe", String::from_utf16_lossy(&buf[..len])),
        _ => r"C:\Windows\explorer.exe".to_string(),
    }
}

fn shell_open(file: &str, parameters: Option<&str>) -> bool {
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (file, parameters) = (wide(file), parameters.map(wide));
    let parameters = parameters.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
    // A value above 32 means the shell took it.
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(file.as_ptr()), parameters, PCWSTR::null(), SW_SHOWNORMAL).0 as isize
            > 32
    }
}

/// Start an app that has no window yet. A packaged app, a PWA or a Squirrel app
/// starts through its AUMID, because the exe that was running isn't the thing to start.
fn launch(rule: &Rule) -> Result<(), &'static str> {
    if rule.exe.to_lowercase().contains(r"\steamapps\") {
        return Err("steam game");
    }
    let aumid = rule.aumid.as_deref().filter(|a| workspace::safe_aumid(a));
    if let Some(aumid) = aumid {
        if shell_open(&explorer_path(), Some(&format!(r"shell:AppsFolder\{aumid}"))) {
            return Ok(());
        }
    }
    // Any workspace, imported or not, only starts an .exe on a drive letter, never a network drive.
    if !workspace::safe_exe(&rule.exe) || is_network_drive(&rule.exe) {
        return Err("refused");
    }
    if !Path::new(&rule.exe).is_file() {
        return Err("not installed");
    }
    shell_open(&rule.exe, None).then_some(()).ok_or("not installed")
}

/// `GetDriveTypeW` answer for a network drive (`DRIVE_REMOTE`).
const DRIVE_REMOTE_TYPE: u32 = 4;

/// Whether the drive the path is on is a network drive (a mapped share).
fn is_network_drive(exe: &str) -> bool {
    let root: Vec<u16> = exe.chars().take(2).chain(['\\']).map(|c| c as u16).chain([0]).collect();
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == DRIVE_REMOTE_TYPE }
}

fn close_to(a: Rect, b: Rect, sized: bool) -> bool {
    let size = [(a.w, b.w), (a.h, b.h)];
    let sides = [(a.x, b.x), (a.y, b.y)].into_iter().chain(size.into_iter().filter(|_| sized));
    sides.into_iter().all(|(p, q)| (p - q).abs() <= CLOSE_ENOUGH)
}

enum Kind {
    Rect(Rect),
    Max { work: Rect, normal: Option<Rect> },
}

/// One window on its way to its place.
struct Move {
    hwnd: isize,
    app: String,
    kind: Kind,
    /// The screen it's going to has another DPI, so it takes a Move step first (D8).
    crosses: bool,
    before: Rect,
}

impl Move {
    /// Where the visible frame goes first. A window that will be maximized goes to its screen.
    fn frame(&self) -> Rect {
        match self.kind {
            Kind::Rect(r) => r,
            Kind::Max { work, .. } => work,
        }
    }
}

fn pause(cancelled: &dyn Fn() -> bool, d: Duration) -> bool {
    std::thread::sleep(d);
    !cancelled()
}

/// Put each window where it goes, one step for all of them at a time, with the
/// apps' own reactions in between, as the zone snapping does one window at a time.
/// Returns how many moved.
fn place_all(moves: &[Move], skipped: &mut Vec<(String, String)>, cancelled: &dyn Fn() -> bool) -> usize {
    // Runs a step on every window. True if any of them did something.
    let again = |step: &mut dyn FnMut(&Move) -> bool| moves.iter().fold(false, |any, m| step(m) | any);
    let restored = again(&mut |m| {
        let h = hwnd_of(m.hwnd);
        // Seam is moving it, so the zone it was snapped to no longer holds (a re-fit would pull it back).
        winutil::clear_zone_stamp(h);
        let restore = winutil::is_minimized(h) || winutil::is_maximized(h);
        if restore {
            winutil::restore(h);
        }
        restore
    });
    if restored && !pause(cancelled, SETTLE) {
        return 0;
    }
    let crossed = again(&mut |m| {
        if m.crosses {
            winutil::move_to(hwnd_of(m.hwnd), m.frame());
        }
        m.crosses
    });
    if crossed && !pause(cancelled, SETTLE) {
        return 0;
    }
    again(&mut |m| {
        winutil::fit(hwnd_of(m.hwnd), m.frame());
        true
    });
    if !pause(cancelled, SETTLE) {
        return 0;
    }
    // An app that answered a DPI change late can leave a window off by its margins.
    let retried = again(&mut |m| {
        let h = hwnd_of(m.hwnd);
        let landed = winutil::visible_rect(h).is_some_and(|v| close_to(v, m.frame(), winutil::is_resizable(h)));
        if !landed {
            winutil::fit(h, m.frame());
        }
        !landed
    });
    if retried && !pause(cancelled, SETTLE) {
        return 0;
    }
    again(&mut |m| {
        let Kind::Max { normal, .. } = m.kind else { return false };
        let h = hwnd_of(m.hwnd);
        if winutil::is_hung(h) {
            skipped.push((m.app.clone(), "timeout".into()));
        } else {
            winutil::place_maximized(h, normal);
        }
        false
    });
    let mut moved = 0;
    for m in moves {
        let h = hwnd_of(m.hwnd);
        // Already in place counts as placed, so compare with where it was meant to go.
        let landed = match m.kind {
            Kind::Max { .. } => winutil::is_maximized(h),
            Kind::Rect(r) => winutil::visible_rect(h).is_some_and(|v| close_to(v, r, winutil::is_resizable(h))),
        };
        if landed || winutil::window_rect(h).is_some_and(|now| now != m.before) {
            moved += 1;
        } else if !skipped.iter().any(|(app, _)| *app == m.app) {
            skipped.push((m.app.clone(), "didn't move".into()));
        }
    }
    moved
}

/// Does the whole switch. Returns `None` if a newer one replaced it.
fn run_switch(job: &SwitchJob, mine: u64) -> Option<Switched> {
    let cancelled = || SWITCH_GEN.load(Ordering::SeqCst) != mine;
    let ws = &job.workspace;
    let geos = geometry(&job.screens, job.gap);
    let mut skipped: Vec<(String, String)> = Vec::new();

    // Snapshot and match.
    let seen = snapshot();
    let mut known: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let mut found = workspace::assign(&ws.windows, &known);

    // Launch what has no window, all at once, then wait for the windows to appear.
    let mut launched = 0;
    let mut waiting: Vec<(usize, Instant)> = Vec::new();
    let mut started: Vec<usize> = Vec::new();
    let mut reused: Vec<usize> = Vec::new();
    let wanted = ws.windows.iter().enumerate().filter(|(i, r)| ws.launch_missing && found[*i].is_none() && r.launch);
    for (i, rule) in wanted {
        let key = launch_key(rule);
        // A switch that was replaced already started this app, so wait for its window.
        if let Some(at) = earlier_launch(&key, mine, &mut reused) {
            started.push(i);
            waiting.push((i, at));
            continue;
        }
        match launch(rule) {
            Ok(()) => {
                note_launch(key, mine);
                launched += 1;
                started.push(i);
                waiting.push((i, Instant::now()));
            }
            Err(reason) => skipped.push((app_name(rule), reason.into())),
        }
    }
    while !waiting.is_empty() {
        std::thread::sleep(POLL);
        if cancelled() {
            return None;
        }
        let claimed: HashSet<isize> = found.iter().flatten().copied().collect();
        let now: Vec<Candidate> = snapshot().into_iter().map(|s| s.cand).collect();
        let fresh: Vec<Candidate> = now.iter().filter(|c| !claimed.contains(&c.hwnd)).cloned().collect();
        known.extend(now);
        let rules: Vec<Rule> = waiting.iter().map(|(i, _)| ws.windows[*i].clone()).collect();
        let hits = workspace::assign(&rules, &fresh);
        let mut still = Vec::new();
        for ((i, since), hit) in waiting.into_iter().zip(hits) {
            match hit {
                Some(hwnd) => found[i] = Some(hwnd),
                None if since.elapsed() >= LAUNCH_WAIT => skipped.push((app_name(&ws.windows[i]), "timeout".into())),
                None => still.push((i, since)),
            }
        }
        waiting = still;
    }

    // Place.
    let mut moves = Vec::new();
    for (rule, hwnd) in ws.windows.iter().zip(&found) {
        let Some(hwnd) = *hwnd else { continue };
        let h = hwnd_of(hwnd);
        // A screen that isn't connected, or a window to keep where it is, isn't moved.
        let Some(geo) = geos.iter().find(|g| g.id == rule.monitor) else { continue };
        if rule.target == (Target::State { state: WinState::Keep }) || !unsafe { IsWindow(Some(h)) }.as_bool() {
            continue;
        }
        let app = app_name(rule);
        if winutil::is_sticky(h) {
            skipped.push((app, "sticky".into()));
            continue;
        }
        if winutil::is_elevated_over_us(h) {
            skipped.push((app, "elevated".into()));
            continue;
        }
        let Some(before) = winutil::window_rect(h) else { continue };
        let kind = match workspace::rect_for(&rule.target, &geo.zones, geo.work) {
            Some(rect) => Kind::Rect(rect),
            None => {
                let on_it = geo.bounds.contains(centre(before).0, centre(before).1);
                if winutil::is_maximized(h) && on_it {
                    continue;
                }
                Kind::Max { work: geo.work, normal: rule.normal_rect.map(|f| workspace::from_fractions(f, geo.work)) }
            }
        };
        let crosses = winutil::monitor_dpi(h) != geo.dpi;
        moves.push(Move { hwnd, app, kind, crosses, before });
    }
    let placed = place_all(&moves, &mut skipped, &cancelled);
    if cancelled() {
        return None;
    }
    // The last session pins what was pinned again, by rule, because a window's handle
    // doesn't outlive a sign-out.
    for (rule, hwnd) in ws.windows.iter().zip(&found) {
        if let (Some(pin), Some(hwnd)) = (rule.pin, *hwnd) {
            pin_again(hwnd, pin);
        }
    }

    // Other windows on this desk's screens: only ones that were there before the switch began,
    // and never anything that belongs to an app this workspace has or has started, whose
    // splash screens, updaters and real main window may open late.
    if ws.other_windows != OtherWindows::Leave && found.iter().all(Option::is_none) {
        // Without a single match the workspace says nothing about what is on screen.
        skipped.push(("Other windows".into(), "nothing matched".into()));
    } else if ws.other_windows != OtherWindows::Leave {
        let claimed: HashSet<isize> = found.iter().flatten().copied().collect();
        let (mut exes, mut aumids): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
        let mine_rules = ws.windows.iter().enumerate().filter(|(i, _)| found[*i].is_some() || started.contains(i));
        for (_, rule) in mine_rules {
            exes.insert(rule.exe.to_lowercase());
            aumids.extend(rule.aumid.clone());
        }
        for c in known.iter().filter(|c| claimed.contains(&c.hwnd)) {
            exes.insert(c.exe.to_lowercase());
            aumids.extend(c.aumid.clone());
        }
        let others = seen.iter().filter(|s| !s.elevated && !s.sticky && !claimed.contains(&s.cand.hwnd));
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
                .is_some_and(|r| geos.iter().any(|g| g.bounds.contains(centre(r).0, centre(r).1)));
            if !on_desk {
                continue;
            }
            // Asks only. The app decides, so one with unsaved work keeps its window.
            unsafe {
                match ws.other_windows {
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

    // Focus the first window. The tray has foreground rights after a hotkey; otherwise this may be refused, and then it stays.
    if let Some(Some(first)) = found.first() {
        let h = hwnd_of(*first);
        unsafe {
            if winutil::is_minimized(h) {
                let _ = ShowWindowAsync(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
    }
    Some(Switched { id: ws.id.clone(), name: ws.name.clone(), launched, placed, skipped })
}

/// Pin a window as a rule says. The seam-wm thread does it and logs how it went.
fn pin_again(hwnd: isize, pin: Pin) {
    if pin.bits() & winutil::PIN_TOP != 0 {
        crate::wm::pin(PinKind::Top, Some(true), hwnd, None);
    }
    if pin.bits() & winutil::PIN_STICKY != 0 {
        crate::wm::pin(PinKind::Sticky, Some(true), hwnd, None);
    }
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
        let before: HashSet<isize> = snapshot().into_iter().map(|s| s.cand.hwnd).collect();
        if let Err(reason) = launch(&rule) {
            log::info!("reopen {app}: not started ({reason})");
            return;
        }
        let started = Instant::now();
        while started.elapsed() < LAUNCH_WAIT {
            std::thread::sleep(POLL);
            let fresh: Vec<Candidate> =
                snapshot().into_iter().map(|s| s.cand).filter(|c| !before.contains(&c.hwnd)).collect();
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
