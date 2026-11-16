//! Smart tiling's worker (`seam-tiling`). It reads what tiling needs to know about a
//! window, which can wait on its app or on Explorer: whether it may be tiled at all
//! (dialogs, owned, tool and fixed-size windows float), its smallest size, and which
//! virtual desktop it is on (`IVirtualDesktopManager`). The tray thread only takes
//! the results and runs the pure tree step (`seam::tiling`), which takes microseconds,
//! and the windows land through the snapping's placement queue.
//!
//! Windows reach it from the seam-wm thread's events: an open after the auto-apply
//! pipeline (classifier, settle, no switch running) found no workspace row or app
//! rule for it, a restore after the same settle, a close or a minimize.

use crate::{log, minsize};
use seam::winutil;
use seam::zones::Rect;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, IsWindowVisible, PostMessageW, WM_APP};
use windows::core::GUID;

/// The worker has events for the snapping. Take them with `take`.
pub const WM_APP_TILING: u32 = WM_APP + 38;

/// A window smart tiling may take, as the worker read it.
#[derive(Clone, Debug)]
pub struct Arrival {
    pub hwnd: isize,
    /// Its visible frame now, which also says its screen, and which Free all gives back.
    pub frame: Rect,
    pub desktop: String,
    /// Its smallest visible frame at its screen's DPI.
    pub min: (i32, i32),
    /// Its `Seam_tile` order when a restart tiles it again, else 0.
    pub order: u32,
    /// It came back from minimized: only a window tiled before it was minimized counts.
    pub restored: bool,
}

pub enum Event {
    Arrived(Arrival),
    /// Closed or minimized.
    Left {
        hwnd: isize,
        minimized: bool,
    },
    /// The window "Float or tile this window" asked for can't be tiled, and why.
    Refused(&'static str),
    /// The virtual desktop in front, by id, or None when the shell can't tell, as on a
    /// desktop with no windows. An empty id is a Windows with no desktop manager.
    Desktop(Option<String>),
    /// A tiled window took the focus, which scrolls a Scrolling screen to show it.
    Focused(isize),
    /// A virtual desktop whose trees are stale (`tiling::stale`): it was deleted.
    Stale(String),
}

enum Job {
    Arrived {
        hwnd: isize,
        restored: bool,
        asked: bool,
    },
    Left {
        hwnd: isize,
        minimized: bool,
    },
    Foreground(isize),
    /// Find the windows a tray that ran before tiled, by their stamps.
    Retile,
    /// Say which desktop is in front, even when it is the one said last.
    Look,
    /// Whether these desktops, each with the windows of its trees, were deleted.
    Stale(Vec<(String, Vec<isize>)>),
}

static OWNER: AtomicIsize = AtomicIsize::new(0);
/// Tiling is on for some screen. Off, an open never starts the worker.
static ON: AtomicBool = AtomicBool::new(false);
/// Lower-case exe names Seam never manages.
static EXCLUDE: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Windows in a tree, so the seam-wm thread passes over every other close and minimize.
static TILED: Mutex<Option<HashSet<isize>>> = Mutex::new(None);
/// Of those, the windows resting minimized, which a restore tiles again whatever their stamp.
static RESTING: Mutex<Option<HashSet<isize>>> = Mutex::new(None);
static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
/// The shell's virtual desktop manager was there when the worker started.
static DESKTOPS: AtomicBool = AtomicBool::new(false);
/// When the last workspace switch ended, so a late minimize of its own isn't taken for mine.
static SWITCH_END: Mutex<Option<Instant>> = Mutex::new(None);
/// How long after a switch its minimizes still count as its own.
pub const SWITCH_GRACE: std::time::Duration = std::time::Duration::from_millis(500);
/// A `look` was answered since the last `take_looked`.
static LOOKED: AtomicBool = AtomicBool::new(false);
static WORKER: OnceLock<Option<Sender<Job>>> = OnceLock::new();

/// Who to tell, the apps tiling leaves alone, and whether it is on for any screen.
pub fn configure(owner: isize, exclude: Vec<String>, on: bool) {
    OWNER.store(owner, Ordering::SeqCst);
    ON.store(on, Ordering::SeqCst);
    *EXCLUDE.lock().unwrap_or_else(|e| e.into_inner()) = exclude;
}

/// The windows in a tree now, and those resting minimized. Called by the snapping on the tray thread.
pub fn set_tiled(list: HashSet<isize>, resting: HashSet<isize>) {
    *TILED.lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
    *RESTING.lock().unwrap_or_else(|e| e.into_inner()) = Some(resting);
}

fn is_resting(hwnd: isize) -> bool {
    RESTING.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|set| set.contains(&hwnd))
}

/// Whether a window is tiled. Cheap, for the seam-wm thread's every close and minimize.
pub fn is_tiled(hwnd: isize) -> bool {
    TILED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|set| set.contains(&hwnd))
}

/// A window opened and settled, and no workspace row or app rule claimed it.
pub fn opened(hwnd: isize) {
    if ON.load(Ordering::SeqCst) {
        let _ = send(Job::Arrived { hwnd, restored: false, asked: false });
    }
}

/// A window came back from minimized and settled.
pub fn restored(hwnd: isize) {
    if !ON.load(Ordering::SeqCst) {
        return;
    }
    let _ = send(Job::Arrived { hwnd, restored: true, asked: false });
}

/// "Float or tile this window" asked to tile it.
pub fn asked(hwnd: isize) {
    let _ = send(Job::Arrived { hwnd, restored: false, asked: true });
}

/// A tiled window closed or was minimized. From the seam-wm thread.
pub fn left(hwnd: isize, minimized: bool) {
    let _ = send(Job::Left { hwnd, minimized });
}

/// Focus moved, maybe to another virtual desktop. From the seam-wm thread.
pub fn foreground(hwnd: isize) {
    let _ = send(Job::Foreground(hwnd));
}

/// Tiling turned on: tile again what a tray that ran before had tiled, and learn the desktop in front.
pub fn retile() {
    let _ = send(Job::Retile);
}

/// The desktop in front changed: look whether the other desktops with trees, each with
/// the windows of its trees, are still there. From the tray thread.
pub fn check_stale(desktops: Vec<(String, Vec<isize>)>) {
    if !desktops.is_empty() {
        let _ = send(Job::Stale(desktops));
    }
}

/// A workspace switch ended. From the tray thread.
pub fn switch_ended() {
    *SWITCH_END.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// A switch runs, or ended a moment ago: its minimizes are its own (D564).
pub fn switch_settling() -> bool {
    let ended = *SWITCH_END.lock().unwrap_or_else(|e| e.into_inner());
    crate::switcher::switching() || ended.is_some_and(|at| at.elapsed() < SWITCH_GRACE)
}

/// A setting waits to know which desktop is in front. False when the worker couldn't start.
pub fn look() -> bool {
    send(Job::Look)
}

/// Whether a `look` was answered since this was last asked.
pub fn take_looked() -> bool {
    LOOKED.swap(false, Ordering::SeqCst)
}

/// Whether Windows says which virtual desktop a window is on. Without it every
/// desktop counts as one, with an empty id.
pub fn has_desktops() -> bool {
    DESKTOPS.load(Ordering::SeqCst)
}

/// What the worker found, in order.
pub fn take() -> Vec<Event> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn send(job: Job) -> bool {
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("seam-tiling".into()).spawn(move || work(rx));
        spawned.ok().map(|_| tx)
    });
    match worker {
        Some(tx) if tx.send(job).is_ok() => true,
        _ => {
            log::info!("tiling worker unavailable");
            false
        }
    }
}

fn post(events: Vec<Event>) {
    if events.is_empty() {
        return;
    }
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).extend(events);
    let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
    unsafe {
        let _ = PostMessageW(Some(owner), WM_APP_TILING, WPARAM(0), LPARAM(0));
    }
}

/// The shell's virtual desktop manager, made again once if Explorer restarted under it.
struct Desktops(Option<IVirtualDesktopManager>);

/// The manager's process went away: Explorer restarted.
const RPC_E_DISCONNECTED: i32 = 0x8001_0108_u32 as i32;
const RPC_S_SERVER_UNAVAILABLE: i32 = 0x8007_06BA_u32 as i32;

impl Desktops {
    fn make() -> Option<IVirtualDesktopManager> {
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }.ok()
    }

    /// The desktop a window is on, as text, or None when the shell doesn't say: a window
    /// on every desktop, such as the taskbar, or a Windows with no desktops.
    fn of(&mut self, h: HWND) -> Option<String> {
        let read = |m: &IVirtualDesktopManager| unsafe { m.GetWindowDesktopId(h) };
        let id = match read(self.0.as_ref()?) {
            Err(e) if [RPC_E_DISCONNECTED, RPC_S_SERVER_UNAVAILABLE].contains(&e.code().0) => {
                self.0 = Desktops::make();
                read(self.0.as_ref()?)
            }
            other => other,
        };
        id.ok().filter(|id| *id != GUID::zeroed()).map(|id| format!("{id:?}"))
    }

    /// The desktop in front: the focused window's, or else that of any shown window not
    /// cloaked, which only the windows of the desktop in front are. None when it can't
    /// tell, and an empty id with no manager.
    fn front(&mut self, focus: HWND) -> Option<String> {
        if self.0.is_none() {
            return Some(String::new());
        }
        self.of(focus).or_else(|| {
            let shown = |h: &HWND| {
                unsafe { IsWindowVisible(*h) }.as_bool() && !winutil::is_cloaked(*h) && !winutil::is_minimized(*h)
            };
            winutil::top_level_windows().into_iter().filter(shown).find_map(|h| self.of(h))
        })
    }
}

fn work(jobs: Receiver<Job>) {
    // The virtual desktop manager is a COM object of the shell.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let mut desktops = Desktops(Desktops::make());
    DESKTOPS.store(desktops.0.is_some(), Ordering::SeqCst);
    if desktops.0.is_none() {
        log::info!("tiling: no virtual desktop manager, so every desktop counts as one");
    }
    let mut front = None::<String>;
    while let Ok(first) = jobs.recv() {
        let started = Instant::now();
        let mut batch: Vec<Job> = std::iter::once(first).chain(jobs.try_iter()).collect();
        // A close or minimize never waits behind a slow app's arrival.
        batch.sort_by_key(|j| !matches!(j, Job::Left { .. }));
        let mut events = Vec::new();
        // Only the last focus change of a batch says which desktop is in front.
        let focus = batch.iter().rev().find_map(|j| if let Job::Foreground(h) = j { Some(*h) } else { None });
        let mut said = false;
        for job in batch {
            match job {
                Job::Arrived { hwnd, restored, asked } => match arrival(&mut desktops, hwnd, restored, asked) {
                    Ok(a) => events.push(Event::Arrived(a)),
                    Err(why) if asked => events.push(Event::Refused(why)),
                    Err(why) if log::enabled() => {
                        log::info!("tiling: {} floats ({why})", winutil::exe_name(hwnd_of(hwnd)).unwrap_or_default())
                    }
                    Err(_) => {}
                },
                Job::Left { hwnd, minimized } => events.push(Event::Left { hwnd, minimized }),
                Job::Foreground(_) => {}
                Job::Look => {
                    front = desktops.front(unsafe { GetForegroundWindow() });
                    events.push(Event::Desktop(front.clone()));
                    LOOKED.store(true, Ordering::SeqCst);
                    said = true;
                }
                Job::Stale(list) => {
                    for (desktop, windows) in list {
                        // A window the shell won't place, as one on every desktop, counts as still there.
                        let found = |k: &isize| {
                            let h = hwnd_of(*k);
                            unsafe { IsWindow(Some(h)) }
                                .as_bool()
                                .then(|| desktops.of(h).unwrap_or_else(|| desktop.clone()))
                        };
                        let found: Vec<Option<String>> = windows.iter().map(found).collect();
                        if seam::tiling::stale(&desktop, &found) {
                            events.push(Event::Stale(desktop));
                        }
                    }
                }
                Job::Retile => {
                    events.extend(stamped(&mut desktops).into_iter().map(Event::Arrived));
                    front = desktops.front(unsafe { GetForegroundWindow() });
                    events.push(Event::Desktop(front.clone()));
                    said = true;
                }
            }
        }
        // The last focus of a batch, when it is a tiled window: a parked column comes into view.
        if let Some(h) = focus.filter(|h| is_tiled(*h)) {
            events.push(Event::Focused(h));
        }
        if let Some(h) = focus.filter(|_| !said) {
            let now = desktops.front(hwnd_of(h));
            if now != front {
                front = now.clone();
                events.push(Event::Desktop(now));
            }
        }
        if log::enabled() && !events.is_empty() {
            log::info!("tiling worker: {} events in {} us", events.len(), started.elapsed().as_micros());
        }
        post(events);
    }
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut _)
}

/// Whether smart tiling may take this window, and what it needs to know of it. Floats:
/// dialogs, owned and tool windows, fixed-size windows, apps under Left alone, windows
/// floated by hand, maximized ones, administrator windows, and windows a workspace or
/// app rule placed, unless the key asked for it.
fn arrival(desktops: &mut Desktops, hwnd: isize, restored: bool, asked: bool) -> Result<Arrival, &'static str> {
    let h = hwnd_of(hwnd);
    if !unsafe { IsWindow(Some(h)) }.as_bool() {
        return Err("closed");
    }
    if winutil::is_floated(h) && !asked {
        return Err("floated by hand");
    }
    // A restore too: a window a workspace switch placed is the workspace's (D564). One
    // tiling held before it was minimized is tiling's still, captured or not.
    if winutil::ws_stamp(h).is_some() && !seam::tiling::stamped_may_tile(asked, restored, is_resting(hwnd)) {
        return Err("placed by a workspace or an app rule");
    }
    if winutil::is_maximized(h) || winutil::is_minimized(h) {
        return Err("maximized or minimized");
    }
    let exclude = EXCLUDE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // Hidden, cloaked, tool, owned by a shown window, frameless popups and full screen.
    if !winutil::is_snappable(h, std::process::id(), &exclude) {
        return Err("not a window zones take, or left alone");
    }
    if !winutil::is_capturable(h, std::process::id()) {
        return Err("no taskbar button");
    }
    if !winutil::is_resizable(h) || winutil::class_name(h) == "#32770" {
        return Err("a dialog or fixed size");
    }
    if winutil::is_elevated_over_us(h) {
        return Err("runs as administrator");
    }
    let frame = winutil::visible_rect(h).ok_or("no frame")?;
    let min = minsize::of(h, winutil::monitor_dpi(h));
    let desktop = desktops.of(h).unwrap_or_default();
    Ok(Arrival { hwnd, frame, desktop, min, order: winutil::tile_stamp(h), restored })
}

/// The windows a tray that ran before tiled, oldest first, as arrivals.
fn stamped(desktops: &mut Desktops) -> Vec<Arrival> {
    let mut found: Vec<Arrival> = winutil::top_level_windows()
        .into_iter()
        .filter(|h| winutil::tile_stamp(*h) != 0)
        .filter_map(|h| arrival(desktops, h.0 as isize, false, false).ok())
        .collect();
    found.sort_by_key(|a| a.order);
    found
}
