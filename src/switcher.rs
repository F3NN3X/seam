//! Workspaces on the tray's side: capture the windows on screen, and switch to
//! a saved set. Both run on worker threads and post one message back. Nothing
//! here waits on another app from the main thread, and every call that could
//! wait on a hung app is the asynchronous one (`ShowWindowAsync`, `PostMessageW`,
//! `SWP_ASYNCWINDOWPOS`) or is skipped for a window that has stopped answering.

use crate::log;
use crate::places;
use crate::wincontrol;
use crate::wm::Pinned;
use seam::api::PinKind;
use seam::classify::{Verdict, WindowFacts, classify};
use seam::config::WindowSize;
use seam::desk;
use seam::winutil;
use seam::workspace::{
    self, Bound, Candidate, OtherWindows, Pin, Place, RowState, RowStatus, Rule, SESSION_ID, Shown, Target, WinState,
    Workspace,
};
use seam::zones::{self, Rect, ZoneLayout};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

mod bind;
mod capture;
mod launch;
mod place;
mod rows;
mod rules;
mod run;
mod stages;

use bind::{bind, bindings, bound_windows, match_rows, unbind};
use launch::{earlier_launch, launch, launch_key, note_launch, place_kind};
use place::{destination, in_place, now_of, place_all, restack, to_focus};
use rows::move_opened;
use run::pin_again;
use stages::{place_rules, run_stages, usable_place};

pub use bind::{bind_rows, capture_bindings, keep_bindings, rebind};
pub use capture::{CaptureJob, capture, capture_session, capture_window, take_captured, take_session, wait_session};
pub use rows::{LiveWorkspace, place_opened, status};
pub use rules::{AppRules, closed_name, pin_by_rules, place_by_rule, reopen};
pub use run::{SwitchJob, outside, switch, switching, take_switched};

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

/// A window's screen and place, and what to log when it's moved there.
type Planned<'g> = (&'g Geo, Kind, Option<String>);

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

/// What every step of a switch reads.
struct Run<'a> {
    ws: &'a Workspace,
    geos: Vec<Geo>,
    pins_stay: bool,
    size_mode: WindowSize,
    cancelled: &'a dyn Fn() -> bool,
    /// The `Seam_ws` key the matched windows are stamped with. The last session stamps none.
    stamp: Option<u32>,
    /// Apps started open on their row's folder or page.
    places: bool,
    /// Each row's remembered window as the switch began, `None` for a row without a
    /// place (D336).
    bound: Vec<Option<Bound>>,
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

/// What a switch waiting for windows reads and changes.
struct Waits<'a> {
    found: &'a mut Vec<Option<isize>>,
    known: &'a mut Vec<Candidate>,
    procs: &'a mut Procs,
    seen: &'a [Seen],
}
