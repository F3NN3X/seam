//! Workspaces on the tray's side: capture the windows on screen, and switch to
//! a saved set. Both run on worker threads and post one message back. Nothing
//! here waits on another app from the main thread, and every call that could
//! wait on a hung app is the asynchronous one (`ShowWindowAsync`, `PostMessageW`,
//! `SWP_ASYNCWINDOWPOS`) or is skipped for a window that has stopped answering.

use crate::log;
use crate::wincontrol;
use crate::wm::Pinned;
use seam::api::PinKind;
use seam::classify::{classify, Verdict, WindowFacts};
use seam::config::WindowSize;
use seam::desk;
use seam::winutil;
use seam::workspace::{
    self, Candidate, OtherWindows, Pin, RowState, RowStatus, Rule, Shown, Target, WinState, Workspace, SESSION_ID,
};
use seam::zones::{self, Rect, ZoneLayout};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
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
/// While a switch waits for windows, how often it looks for itself, for an app
/// whose window shows without the event (or before it has a title).
const LAUNCH_POLL: Duration = Duration::from_secs(1);
/// What `skipped` says about an app whose window didn't show in time.
const TOOK_TOO_LONG: &str = "Took too long to open";
/// What `skipped` says about an app whose program runs with no window.
const RUNS_IN_BACKGROUND: &str = "Running in background";
/// How often an arranging switch looks for windows that have appeared.
const ARRANGE_POLL: Duration = Duration::from_millis(500);
/// Time for an app to react to a move before the next step.
const SETTLE: Duration = Duration::from_millis(120);
/// A window this close to where it was put counts as there.
const CLOSE_ENOUGH: i32 = 4;
/// A free place whose saved fractions come within this share of its size at
/// capture is on the screen it was captured on.
const SAME_SIZE: f32 = 0.1;
/// The gap slider's end, as in the snapping.
const MAX_GAP: u32 = 32;

/// A connected screen and its zones, as the tray knows them.
pub struct Screen {
    pub id: String,
    /// Line Up number.
    pub number: usize,
    pub bounds: Rect,
    pub layout: Option<ZoneLayout>,
    /// Physical pixels per inch from EDID, 0 when the screen doesn't report its size.
    pub raw_dpi: f64,
}

/// A screen with its work area and zones in pixels, worked out when a job runs so a moved taskbar counts.
struct Geo {
    id: String,
    number: usize,
    bounds: Rect,
    work: Rect,
    dpi: u32,
    raw_dpi: f64,
    zones: Vec<Rect>,
}

fn geometry(screens: &[Screen], gap: u32) -> Vec<Geo> {
    let placed = screens.iter().filter_map(|s| {
        let (work, dpi) = winutil::work_area(s.bounds)?;
        let gap = (gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        let zones = s.layout.as_ref().map(|l| zones::rects(&l.tree, work, gap)).unwrap_or_default();
        Some(Geo { id: s.id.clone(), number: s.number, bounds: s.bounds, work, dpi, raw_dpi: s.raw_dpi, zones })
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
    /// `Seam_pin` bits. A sticky window is never moved, minimized or closed by a
    /// switch (D106), and with pins staying neither is one pinned on top.
    pin: usize,
}

/// A window a switch leaves alone: sticky, or pinned on top while pins stay.
fn stays(pin: usize, pins_stay: bool) -> bool {
    pin & winutil::PIN_STICKY != 0 || (pins_stay && pin & winutil::PIN_TOP != 0)
}

/// What a capture or a switch knows about a process, read once per process ID.
/// The AUMID isn't here: windows of one process can have different ones (PWAs).
struct Proc {
    path: Option<String>,
    elevated: bool,
}

type Procs = HashMap<u32, Proc>;

fn proc_of(procs: &mut Procs, pid: u32) -> &Proc {
    procs
        .entry(pid)
        .or_insert_with(|| Proc { path: winutil::exe_path_of(pid), elevated: winutil::elevated_over_us(pid) })
}

/// Whether an app with no window still runs, as one closed to the tray does. The
/// process list is read once, when first needed, and paths come from `Procs`.
#[derive(Default)]
struct Background {
    running: Option<Vec<(u32, String)>>,
    families: HashMap<u32, Option<String>>,
}

impl Background {
    /// A process runs the rule's program, or for a Store app, belongs to its package.
    fn runs(&mut self, rule: &Rule, procs: &mut Procs) -> bool {
        let Background { running, families } = self;
        let running = running.get_or_insert_with(winutil::processes);
        let name = winutil::file_name(&rule.exe);
        let mut same_exe =
            |pid: u32| proc_of(procs, pid).path.as_deref().is_some_and(|path| path.eq_ignore_ascii_case(&rule.exe));
        if !rule.exe.is_empty() && running.iter().any(|(pid, n)| *n == name && same_exe(*pid)) {
            return true;
        }
        let Some((family, _)) = rule.aumid.as_deref().and_then(|a| a.split_once('!')) else { return false };
        running.iter().any(|(pid, _)| {
            let of = families.entry(*pid).or_insert_with(|| winutil::package_family_of(*pid));
            of.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(family))
        })
    }
}

/// The windows a workspace may use, topmost first. Seam's own are never among them.
fn snapshot(procs: &mut Procs) -> Vec<Seen> {
    snapshot_with(&[], procs, &mut |_, _| {})
}

/// `snapshot`, leaving out the apps in `exclude` (lower-case exe names) and telling
/// `skip` why each other window was left out. The window's own facts decide first,
/// so only windows that pass them open their process.
fn snapshot_with(exclude: &[String], procs: &mut Procs, skip: &mut dyn FnMut(&'static str, &WindowFacts)) -> Vec<Seen> {
    let mut seen = Vec::new();
    for h in winutil::top_level_windows() {
        let z = seen.len();
        seen.extend(seen_of(h, z, exclude, procs, skip));
    }
    seen
}

/// One window as a snapshot sees it, or `None` if it isn't a real app window.
fn seen_of(
    h: HWND,
    z: usize,
    exclude: &[String],
    procs: &mut Procs,
    skip: &mut dyn FnMut(&'static str, &WindowFacts),
) -> Option<Seen> {
    let excluded = |name: &str| exclude.iter().any(|x| x == name);
    let mut facts = winutil::window_facts(h, std::process::id());
    if let Verdict::Skip(why) = classify(&facts, &excluded) {
        skip(why, &facts);
        return None;
    }
    let proc = proc_of(procs, winutil::process_id(h));
    let Some(exe) = proc.path.clone().and_then(|path| winutil::exe_path_from(h, path)) else {
        skip("no exe", &facts);
        return None;
    };
    facts.exe_name = winutil::file_name(&exe);
    if let Verdict::Skip(why) = classify(&facts, &excluded) {
        skip(why, &facts);
        return None;
    }
    let elevated = proc.elevated;
    let cand = Candidate {
        hwnd: h.0 as isize,
        exe,
        aumid: winutil::aumid(h),
        class: facts.class,
        title: winutil::title(h),
        z,
    };
    Some(Seen { cand, elevated, pin: winutil::pin_bits(h) })
}

/// How the window is shown now.
fn shown_of(h: HWND) -> Shown {
    if winutil::is_minimized(h) {
        Shown::Minimized
    } else if winutil::is_maximized(h) {
        Shown::Maximized
    } else if winutil::is_full_screen(h) {
        Shown::Fullscreen
    } else {
        Shown::Normal
    }
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
    /// Lower-case exe names never captured: the zones' exclude list.
    pub exclude: Vec<String>,
}

/// A capture's rules, and the window each came from.
static CAPTURED: Mutex<Option<(Vec<Rule>, Vec<isize>)>> = Mutex::new(None);
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

pub fn take_captured() -> Option<(Vec<Rule>, Vec<isize>)> {
    CAPTURED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Capture the last session (D120): every window, pinned ones with their pins.
/// The main thread gets `WM_APP_SESSION` when it's done.
pub fn capture_session(job: CaptureJob) -> bool {
    let spawned = std::thread::Builder::new().name("seam-session".into()).spawn(move || {
        let _com = Com::enter();
        let (rules, _) = capture_rules(&job, true);
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

/// The rules, and the window each came from.
fn capture_rules(job: &CaptureJob, session: bool) -> (Vec<Rule>, Vec<isize>) {
    let started = Instant::now();
    let geos = geometry(&job.screens, job.gap);
    let mut skips: BTreeMap<&'static str, usize> = BTreeMap::new();
    let seen = snapshot_with(&job.exclude, &mut Procs::new(), &mut |why, facts| {
        *skips.entry(why).or_default() += 1;
        // Hidden and cloaked windows are most of them, so those are only counted.
        if !matches!(why, "hidden" | "cloaked") {
            log::info!("capture skipped {} {}: {why}", facts.class, facts.exe_name);
        }
    });
    let foreground = unsafe { GetForegroundWindow() }.0 as isize;
    let (mut rules, mut hwnds) = (Vec::new(), Vec::new());
    // A sticky window belongs to no workspace. The last session keeps it, pinned.
    for seen in seen.into_iter().filter(|s| session || s.pin & winutil::PIN_STICKY == 0) {
        let h = hwnd_of(seen.cand.hwnd);
        let shown = shown_of(h);
        let normal = winutil::normal_rect(h);
        // A minimized window has no place on screen, so it counts where it would restore to.
        let visible = if shown == Shown::Minimized { normal } else { winutil::visible_rect(h) };
        let Some(visible) = visible else { continue };
        let Some(geo) = geos.iter().find(|g| g.bounds.contains(centre(visible).0, centre(visible).1)) else {
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
    let skipped: Vec<String> = skips.iter().map(|(why, n)| format!("{why} {n}")).collect();
    let ms = started.elapsed().as_millis();
    log::info!("workspace capture: {} windows in {ms} ms, skipped {}", rules.len(), skipped.join(", "));
    (rules, hwnds)
}

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

/// Where a window goes, and how it's shown there.
enum Kind {
    /// Shown normally, its visible frame on the rect.
    Rect(Rect),
    Max {
        work: Rect,
        normal: Option<Rect>,
    },
    /// Minimized, restoring onto the rect later.
    Min(Rect),
    /// Borderless on the whole screen.
    Full(Rect),
}

/// A matched window as it is now, read once before planning.
struct Now {
    window: Rect,
    /// The visible frame, or for a minimized window the frame it restores to.
    frame: Rect,
    shown: Shown,
    resizable: bool,
}

fn now_of(h: HWND, dpi: u32) -> Option<Now> {
    let window = winutil::window_rect(h)?;
    let shown = shown_of(h);
    let frame = match shown {
        Shown::Minimized => winutil::normal_frame(h, dpi)?,
        _ => winutil::visible_rect(h).unwrap_or(window),
    };
    Some(Now { window, frame, shown, resizable: winutil::is_resizable(h) })
}

/// What a rule asks for on its screen, or `None` for a window to keep where it is.
fn plan(rule: &Rule, geo: &Geo) -> Option<Kind> {
    if rule.target == (Target::State { state: WinState::Keep }) {
        return None;
    }
    let rect = workspace::rect_for(&rule.target, &geo.zones, geo.work);
    let normal = rule.normal_rect.map(|f| workspace::from_fractions(f, geo.work));
    Some(match (rule.shown, rect) {
        (Shown::Minimized, _) => Kind::Min(rect.or(normal).unwrap_or(geo.work)),
        (Shown::Fullscreen, _) => Kind::Full(geo.bounds),
        (Shown::Normal, Some(r)) => Kind::Rect(r),
        (Shown::Maximized, _) | (Shown::Normal, None) => Kind::Max { work: geo.work, normal: normal.or(rect) },
    })
}

/// Where a rule puts its window, and on which screen. With "Keep its pixel size" or
/// "Keep its real size", a free place restored on a screen other than the one it was
/// captured on (`captured_on`, or without it, one where the saved fractions no longer
/// give that size) keeps the pixels or the millimetres it had at capture (§11.4): on
/// its own screen, or, with that screen gone, on the one the window is on now. Then it comes with the line to log once
/// it's moved. `tell` logs a window Windows scales because a screen doesn't report
/// its size. `None` leaves the window where it is.
fn destination<'g>(rule: &Rule, geos: &'g [Geo], h: HWND, mode: WindowSize, tell: bool) -> Option<Planned<'g>> {
    let saved = geos.iter().find(|g| g.id == rule.monitor);
    let px = rule.size_px.map(|p| [p[0] as f32, p[1] as f32]);
    let kept = match mode {
        WindowSize::Windows => None,
        WindowSize::Pixels => px,
        WindowSize::Real => rule.size_mm,
    };
    let kept = kept.filter(|_| matches!(rule.target, Target::Rect { .. }));
    let Some(kept) = kept else { return saved.and_then(|geo| Some((geo, plan(rule, geo)?, None))) };
    let geo = match saved {
        Some(geo) => geo,
        None => {
            let now = winutil::visible_rect(h)?;
            geos.iter().find(|g| g.bounds.contains(centre(now).0, centre(now).1))?
        }
    };
    let kind = plan(rule, geo)?;
    let (Kind::Rect(at) | Kind::Min(at)) = kind else { return Some((geo, kind, None)) };
    // Restored on the screen it was captured on. Without that record, the screen
    // it was captured on is the one whose fractions still give the size it had:
    // in millimetres where both screens report one, else in pixels.
    let same = match &rule.captured_on {
        Some(id) => *id == geo.id,
        None => {
            let near = |now: [f32; 2], then: [f32; 2]| (0..2).all(|i| (now[i] - then[i]).abs() <= then[i] * SAME_SIZE);
            let size = match (rule.size_mm, desk::size_mm(at.w, at.h, geo.raw_dpi)) {
                (Some(then), Some(now)) => near(now, then),
                _ => px.is_none_or(|then| near([at.w as f32, at.h as f32], then)),
            };
            saved.is_some() && size
        }
    };
    if same {
        return Some((geo, kind, None));
    }
    let size = (f64::from(kept[0]), f64::from(kept[1]));
    let resized = match mode {
        WindowSize::Pixels => Some(desk::centred(size, at, geo.work)),
        _ => desk::physical_rect(size, desk::MM_PER_INCH, geo.raw_dpi, at, geo.work),
    };
    let Some(r) = resized else {
        if tell {
            log::info!(
                "window size: {} scaled by Windows, screen {} doesn't report its size",
                app_name(rule),
                geo.number
            );
        }
        // On its own screen it keeps the saved fractions. A screen that's gone leaves it where it is.
        return saved.map(|geo| (geo, kind, None));
    };
    let (app, to) = (app_name(rule), geo.number);
    let note =
        format!("window size ({mode:?}): {app} screen at capture → {to}, {}×{} px → {}×{} px", at.w, at.h, r.w, r.h);
    Some((geo, if matches!(kind, Kind::Min(_)) { Kind::Min(r) } else { Kind::Rect(r) }, Some(note)))
}

/// A window's screen and place, and what to log when it's moved there.
type Planned<'g> = (&'g Geo, Kind, Option<String>);

/// Whether the window already is where, and as, the plan wants it. Such a
/// window takes no call at all.
fn in_place(kind: &Kind, now: &Now, screen: Rect) -> bool {
    match *kind {
        Kind::Rect(r) => now.shown == Shown::Normal && close_to(now.frame, r, now.resizable),
        Kind::Min(r) => now.shown == Shown::Minimized && close_to(now.frame, r, now.resizable),
        Kind::Max { .. } => {
            now.shown == Shown::Maximized && screen.contains(centre(now.window).0, centre(now.window).1)
        }
        Kind::Full(r) => now.shown == Shown::Fullscreen && now.window == r,
    }
}

/// One window on its way to its place.
struct Move {
    hwnd: isize,
    app: String,
    kind: Kind,
    /// How it's shown before the move.
    was: Shown,
    /// It's going to another screen, which a maximized window can't do in one step.
    on_screen: bool,
    /// The screen it's going to has another DPI, so it takes a Move step first (D8).
    crosses: bool,
    /// The DPI of the screen it's going to.
    dpi: u32,
    before: Rect,
}

/// How the switch is going, for the log and the event.
#[derive(Default)]
struct Tally {
    /// App and reason.
    skipped: Vec<(String, String)>,
    placed: usize,
    moves: usize,
    in_place: usize,
    plan: Duration,
    /// The windows shown on screen after placing, for the stacking and the focus.
    stack: Vec<Stacked>,
}

/// A window the switch shows, with where it stood at capture.
struct Stacked {
    hwnd: isize,
    /// The rule's stacking, 0 the topmost.
    z: u32,
    /// Its stacking now, if the first snapshot saw it.
    now: Option<usize>,
    focused: bool,
}

fn pause(cancelled: &dyn Fn() -> bool, d: Duration) -> bool {
    std::thread::sleep(d);
    !cancelled()
}

/// The calls that wait on the app are skipped for one that has stopped answering.
fn answering(m: &Move, skipped: &mut Vec<(String, String)>) -> bool {
    let hung = winutil::is_hung(hwnd_of(m.hwnd));
    if hung {
        skipped.push((m.app.clone(), "timeout".into()));
    }
    !hung
}

/// Whether the window ended up as the move wanted.
fn landed(h: HWND, kind: &Kind) -> bool {
    match *kind {
        Kind::Rect(r) => winutil::visible_rect(h).is_some_and(|v| close_to(v, r, winutil::is_resizable(h))),
        Kind::Max { .. } => winutil::is_maximized(h),
        Kind::Min(_) => winutil::is_minimized(h),
        Kind::Full(r) => winutil::window_rect(h) == Some(r),
    }
}

/// Put each window where it goes, in one pass. Only a window that goes to a screen
/// with another DPI, or a maximized one that changes screens, takes a second step
/// after the apps have had a moment to react (D8), and only those are checked and
/// retried once. Returns how many moved.
fn place_all(moves: &[Move], skipped: &mut Vec<(String, String)>, cancelled: &dyn Fn() -> bool) -> usize {
    let mut moved = 0;
    let mut later: Vec<&Move> = Vec::new();
    for m in moves {
        let h = hwnd_of(m.hwnd);
        // Seam is moving it, so the zone it was snapped to no longer holds (a re-fit would pull it back).
        winutil::clear_zone_stamp(h);
        let restoring = matches!(m.was, Shown::Minimized | Shown::Maximized);
        match m.kind {
            Kind::Min(r) => {
                if answering(m, skipped) && winutil::place_minimized(h, r, m.dpi) {
                    moved += 1;
                }
            }
            Kind::Max { normal, .. } if m.on_screen => {
                if answering(m, skipped) && winutil::place_maximized(h, normal) {
                    moved += 1;
                }
            }
            Kind::Max { work, .. } => {
                if !restoring {
                    winutil::move_to(h, work);
                } else if !answering(m, skipped) || !winutil::place_restored(h, work, m.dpi) {
                    continue;
                }
                later.push(m);
            }
            // A window that has its caption back isn't full screen any more.
            Kind::Full(_) if winutil::has_caption(h) => skipped.push((m.app.clone(), "not full screen".into())),
            Kind::Rect(r) | Kind::Full(r) => {
                if restoring {
                    if !answering(m, skipped) || !winutil::place_restored(h, r, m.dpi) {
                        continue;
                    }
                } else if m.crosses {
                    winutil::move_to(h, r);
                } else if matches!(m.kind, Kind::Full(_)) {
                    winutil::set_rect(h, r);
                } else {
                    winutil::fit(h, r);
                }
                if m.crosses {
                    later.push(m);
                } else {
                    moved += 1;
                }
            }
        }
    }
    if later.is_empty() || !pause(cancelled, SETTLE) {
        return moved;
    }
    // On the new screen, with its DPI, the margins and size are right this time.
    for m in &later {
        let h = hwnd_of(m.hwnd);
        match m.kind {
            Kind::Rect(r) => winutil::fit(h, r),
            Kind::Full(r) => winutil::set_rect(h, r),
            Kind::Max { normal, .. } => {
                if answering(m, skipped) {
                    winutil::place_maximized(h, normal);
                }
            }
            Kind::Min(_) => {}
        }
    }
    if !pause(cancelled, SETTLE) {
        return moved;
    }
    for m in later {
        let h = hwnd_of(m.hwnd);
        // An app that answered a DPI change late can leave a window off by its margins.
        if let (Kind::Rect(r), false) = (&m.kind, landed(h, &m.kind)) {
            winutil::fit(h, *r);
        }
        if landed(h, &m.kind) || winutil::window_rect(h).is_some_and(|now| now != m.before) {
            moved += 1;
        } else if !skipped.iter().any(|(app, _)| *app == m.app) {
            skipped.push((m.app.clone(), "didn't move".into()));
        }
    }
    moved
}

/// What every step of a switch reads.
struct Run<'a> {
    ws: &'a Workspace,
    geos: Vec<Geo>,
    pins_stay: bool,
    size_mode: WindowSize,
    cancelled: &'a dyn Fn() -> bool,
    /// The `Seam_ws` key the matched windows are stamped with. The last session stamps none.
    stamp: Option<u32>,
}

/// Place the windows of the rules `which` that have one, then pin again what the
/// rule says was pinned. A window already in place takes no call. Adds to `tally`.
fn place_rules(run: &Run, found: &[Option<isize>], which: &[usize], seen: &[Seen], tally: &mut Tally) {
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

/// Stack the shown windows as they were at capture, bottom first, unless they
/// already are. Windows whose stacking wasn't captured are left where they are.
fn restack(stack: &[Stacked], moved: bool) {
    let mut known: Vec<&Stacked> = stack.iter().filter(|s| s.z != u32::MAX).collect();
    known.sort_by_key(|s| s.z);
    let mut now = known.clone();
    now.sort_by_key(|s| s.now.unwrap_or(usize::MAX));
    // A move can change the stacking, and a window found after the first look has none to compare.
    let stacked = !moved && known.iter().zip(&now).all(|(a, b)| a.hwnd == b.hwnd && a.now.is_some());
    if stacked {
        return;
    }
    for s in known.iter().rev() {
        winutil::raise(hwnd_of(s.hwnd));
    }
}

/// The window to focus: the one that had the focus at capture, or else the
/// topmost. A minimized window is never in `stack`, so never focused.
fn to_focus(stack: &[Stacked]) -> Option<isize> {
    let focused = stack.iter().find(|s| s.focused);
    focused.or_else(|| stack.iter().min_by_key(|s| s.z)).map(|s| s.hwnd)
}

/// The apps a switch has started.
#[derive(Default)]
struct Starts {
    /// Started by this switch.
    launched: usize,
    /// Rows started by this switch or a replaced one.
    started: Vec<usize>,
    /// Rows still waiting for their window, and since when.
    waiting: Vec<(usize, Instant)>,
    /// A replaced switch's launches this one took over.
    reused: Vec<usize>,
}

impl Starts {
    /// Start the app of row `i`, unless a switch this one replaced already did.
    fn start(&mut self, ws: &Workspace, i: usize, mine: u64, tally: &mut Tally) {
        let rule = &ws.windows[i];
        let key = launch_key(rule);
        if let Some(at) = earlier_launch(&key, mine, &mut self.reused) {
            self.started.push(i);
            self.waiting.push((i, at));
            return;
        }
        match launch(rule) {
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

/// What a switch waiting for windows reads and changes.
struct Waits<'a> {
    found: &'a mut Vec<Option<isize>>,
    known: &'a mut Vec<Candidate>,
    procs: &'a mut Procs,
    seen: &'a [Seen],
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
    let mut caught = Vec::new();
    for ((i, since), hit) in starts.waiting.iter().zip(workspace::assign(&rules, &fresh)) {
        if let Some(hwnd) = hit {
            waits.found[*i] = Some(hwnd);
            caught.push(*i);
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
fn run_stages(
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
            starts.start(ws, *i, mine, tally);
        }
        due.retain(|(_, at)| *at > now);
        // Each app's time runs from when it started.
        for (i, _) in starts.waiting.iter().filter(|(_, since)| since.elapsed() >= LAUNCH_WAIT) {
            tally.skipped.push((app_name(&ws.windows[*i]), TOOK_TOO_LONG.into()));
            late.push(*i);
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

/// Does the whole switch. Returns `None` if a newer one replaced it.
fn run_switch(job: &SwitchJob, mine: u64) -> Option<Switched> {
    let begun = Instant::now();
    let cancelled = || SWITCH_GEN.load(Ordering::SeqCst) != mine;
    let ws = &job.workspace;
    let stamp = (ws.id != SESSION_ID).then(|| workspace::stamp_key(&ws.id));
    let geos = geometry(&job.screens, job.gap);
    let run = Run { ws, geos, pins_stay: job.pins_stay, size_mode: job.size_mode, cancelled: &cancelled, stamp };
    let mut tally = Tally::default();
    let mut procs = Procs::new();

    // Snapshot and match.
    let seen = snapshot(&mut procs);
    let mut known: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let mut found = workspace::assign(&ws.windows, &known);
    // This workspace's stamps on windows it didn't match this time are out of date.
    if let Some(key) = stamp {
        let matched: HashSet<isize> = found.iter().flatten().copied().collect();
        for s in seen.iter().filter(|s| !matched.contains(&s.cand.hwnd)) {
            let h = hwnd_of(s.cand.hwnd);
            if winutil::ws_stamp(h).is_some_and(|(k, _)| k == key) {
                winutil::clear_ws_stamp(h);
            }
        }
    }
    let mut starts = Starts::default();

    if let Some(wait) = job.arrange_for {
        // Something else opens the windows, so place each one as it appears, once per rule.
        let mut done = vec![false; ws.windows.len()];
        let since = Instant::now();
        loop {
            let ready: Vec<usize> = (0..done.len()).filter(|i| !done[*i] && found[*i].is_some()).collect();
            if !ready.is_empty() {
                place_rules(&run, &found, &ready, &seen, &mut tally);
                ready.iter().for_each(|i| done[*i] = true);
            }
            if done.iter().all(|d| *d) || since.elapsed() >= wait {
                break;
            }
            std::thread::sleep(ARRANGE_POLL);
            if cancelled() {
                return None;
            }
            let claimed: HashSet<isize> = found.iter().flatten().copied().collect();
            let fresh: Vec<Candidate> =
                snapshot(&mut procs).into_iter().map(|s| s.cand).filter(|c| !claimed.contains(&c.hwnd)).collect();
            let missing: Vec<usize> = (0..done.len()).filter(|i| found[*i].is_none()).collect();
            let rules: Vec<Rule> = missing.iter().map(|i| ws.windows[*i].clone()).collect();
            for (i, hit) in missing.into_iter().zip(workspace::assign(&rules, &fresh)) {
                found[i] = hit;
            }
        }
        let absent: Vec<&Rule> = ws.windows.iter().zip(&found).filter(|(_, h)| h.is_none()).map(|(r, _)| r).collect();
        let names: Vec<String> = absent.iter().map(|r| app_name(r)).collect();
        let shown = found.iter().flatten().count();
        log::info!("{}: placed {shown} of {}, never showed: {}", ws.name, found.len(), names.join(", "));
        let mut background = Background::default();
        for rule in absent {
            let why = if background.runs(rule, &mut procs) { RUNS_IN_BACKGROUND } else { "timeout" };
            tally.skipped.push((app_name(rule), why.into()));
        }
    } else {
        // What is running already goes in one pass, before anything starts.
        let running: Vec<usize> = (0..found.len()).filter(|i| found[*i].is_some()).collect();
        place_rules(&run, &found, &running, &seen, &mut tally);
        if cancelled() {
            return None;
        }
        let wanted = |(i, r): (usize, &Rule)| (ws.launch_missing && found[i].is_none() && r.launch).then_some(i);
        let starting: Vec<usize> = ws.windows.iter().enumerate().filter_map(wanted).collect();
        if !starting.is_empty() {
            let mut waits = Waits { found: &mut found, known: &mut known, procs: &mut procs, seen: &seen };
            run_stages(&run, mine, &starting, &mut waits, &mut starts, &mut tally)?;
        }
        // An app this switch doesn't start and that has no window is reported when its
        // program still runs, closed to the tray.
        let mut background = Background::default();
        let idle = ws.windows.iter().enumerate().filter(|(i, _)| found[*i].is_none() && !starting.contains(i));
        for (_, rule) in idle {
            if background.runs(rule, &mut procs) {
                tally.skipped.push((app_name(rule), RUNS_IN_BACKGROUND.into()));
            }
        }
    }
    if cancelled() {
        return None;
    }

    // Stacking and focus last. A switch that waited up to a minute would take them from
    // whatever the user opened since.
    if job.arrange_for.is_none() {
        restack(&tally.stack, tally.moves > 0);
        // The tray has foreground rights after a hotkey; otherwise the focus may be refused, and then it stays.
        let foreground = unsafe { GetForegroundWindow() }.0 as isize;
        if let Some(hwnd) = to_focus(&tally.stack).filter(|h| *h != foreground) {
            wincontrol::focus(hwnd_of(hwnd), true);
        }
    }

    // Other windows on this desk's screens: only ones that were there before the switch began,
    // and never anything that belongs to an app this workspace has or has started, whose
    // splash screens, updaters and real main window may open late.
    let other_windows = if job.alongside { OtherWindows::Leave } else { ws.other_windows };
    if other_windows != OtherWindows::Leave && found.iter().all(Option::is_none) {
        // Without a single match the workspace says nothing about what is on screen.
        tally.skipped.push(("Other windows".into(), "nothing matched".into()));
    } else if other_windows != OtherWindows::Leave {
        let claimed: HashSet<isize> = found.iter().flatten().copied().collect();
        let (mut exes, mut aumids): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
        let mine_rules =
            ws.windows.iter().enumerate().filter(|(i, _)| found[*i].is_some() || starts.started.contains(i));
        for (_, rule) in mine_rules {
            exes.insert(rule.exe.to_lowercase());
            aumids.extend(rule.aumid.clone());
        }
        for c in known.iter().filter(|c| claimed.contains(&c.hwnd)) {
            exes.insert(c.exe.to_lowercase());
            aumids.extend(c.aumid.clone());
        }
        let others =
            seen.iter().filter(|s| !s.elevated && !stays(s.pin, job.pins_stay) && !claimed.contains(&s.cand.hwnd));
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

    log::info!(
        "{}: switched in {} ms, plan {} ms, {} moves, {} already in place",
        ws.name,
        begun.elapsed().as_millis(),
        tally.plan.as_millis(),
        tally.moves,
        tally.in_place
    );
    let launched = starts.launched;
    Some(Switched { id: ws.id.clone(), name: ws.name.clone(), launched, placed: tally.placed, skipped: tally.skipped })
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
}

/// How each row's app is doing, the way a switch would see it. Runs on a worker.
pub fn status(live: &LiveWorkspace) -> Vec<RowStatus> {
    let _com = Com::enter();
    let geos = geometry(&live.screens, live.gap);
    let mut procs = Procs::new();
    let seen = snapshot(&mut procs);
    let cands: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let found = workspace::assign(&live.workspace.windows, &cands);
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
    let hit = workspace::assign(&rules, std::slice::from_ref(&seen.cand)).iter().position(Option::is_some)?;
    let (row, rule) = (free[hit], &ws.windows[free[hit]]);
    let app = app_name(rule);
    let geos = geometry(&live.screens, live.gap);
    let target = destination(rule, &geos, h, live.size_mode, true);
    let now = target.as_ref().and_then(|(geo, ..)| now_of(h, geo.dpi));
    let moved = match (target, now) {
        (Some((geo, kind, note)), Some(now)) if !in_place(&kind, &now, geo.bounds) => {
            if let Some(note) = note {
                log::info!("{note}");
            }
            let on_screen = geo.bounds.contains(centre(now.window).0, centre(now.window).1);
            let crosses = winutil::monitor_dpi(h) != geo.dpi;
            let (was, before, dpi) = (now.shown, now.window, geo.dpi);
            let m = Move { hwnd, app: app.clone(), kind, was, on_screen, crosses, dpi, before };
            // A switch that starts meanwhile has the last word.
            place_all(std::slice::from_ref(&m), &mut Vec::new(), &switching) > 0
        }
        _ => false,
    };
    winutil::set_ws_stamp(h, key, row);
    let what = if moved { "placed" } else { "already in place" };
    Some(format!("{app} for row {}: {what} in {} ms", row + 1, started.elapsed().as_millis()))
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
        if let Err(reason) = launch(&rule) {
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
