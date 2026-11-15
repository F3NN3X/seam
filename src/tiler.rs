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
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, PostMessageW, WM_APP};
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
    /// The virtual desktop in front, by id.
    Desktop(String),
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
}

static OWNER: AtomicIsize = AtomicIsize::new(0);
/// Tiling is on for some screen. Off, an open never starts the worker.
static ON: AtomicBool = AtomicBool::new(false);
/// Lower-case exe names Seam never manages.
static EXCLUDE: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Windows in a tree, so the seam-wm thread passes over every other close and minimize.
static TILED: Mutex<Option<HashSet<isize>>> = Mutex::new(None);
static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static WORKER: OnceLock<Option<Sender<Job>>> = OnceLock::new();

/// Who to tell, the apps tiling leaves alone, and whether it is on for any screen.
pub fn configure(owner: isize, exclude: Vec<String>, on: bool) {
    OWNER.store(owner, Ordering::SeqCst);
    ON.store(on, Ordering::SeqCst);
    *EXCLUDE.lock().unwrap_or_else(|e| e.into_inner()) = exclude;
}

/// The windows in a tree now, and those resting minimized. Called by the snapping on the tray thread.
pub fn set_tiled(list: HashSet<isize>) {
    *TILED.lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
}

/// Whether a window is tiled. Cheap, for the seam-wm thread's every close and minimize.
pub fn is_tiled(hwnd: isize) -> bool {
    TILED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|set| set.contains(&hwnd))
}

/// A window opened and settled, and no workspace row or app rule claimed it.
pub fn opened(hwnd: isize) {
    if ON.load(Ordering::SeqCst) {
        send(Job::Arrived { hwnd, restored: false, asked: false });
    }
}

/// A window came back from minimized and settled.
pub fn restored(hwnd: isize) {
    send(Job::Arrived { hwnd, restored: true, asked: false });
}

/// "Float or tile this window" asked to tile it.
pub fn asked(hwnd: isize) {
    send(Job::Arrived { hwnd, restored: false, asked: true });
}

/// A tiled window closed or was minimized. From the seam-wm thread.
pub fn left(hwnd: isize, minimized: bool) {
    send(Job::Left { hwnd, minimized });
}

/// Focus moved, maybe to another virtual desktop. From the seam-wm thread.
pub fn foreground(hwnd: isize) {
    send(Job::Foreground(hwnd));
}

/// Tiling turned on: tile again what a tray that ran before had tiled, and learn the desktop in front.
pub fn retile() {
    send(Job::Retile);
}

/// A setting waits to know which desktop is in front.
pub fn look() {
    send(Job::Look);
}

/// What the worker found, in order.
pub fn take() -> Vec<Event> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn send(job: Job) {
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("seam-tiling".into()).spawn(move || work(rx));
        spawned.ok().map(|_| tx)
    });
    match worker {
        Some(tx) if tx.send(job).is_ok() => {}
        _ => log::info!("tiling worker unavailable"),
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

fn work(jobs: Receiver<Job>) {
    // The virtual desktop manager is a COM object of the shell.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let desktops: Option<IVirtualDesktopManager> =
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }.ok();
    if desktops.is_none() {
        log::info!("tiling: no virtual desktop manager, so every desktop counts as one");
    }
    let mut front = None::<String>;
    while let Ok(first) = jobs.recv() {
        let started = Instant::now();
        let batch: Vec<Job> = std::iter::once(first).chain(jobs.try_iter()).collect();
        let mut events = Vec::new();
        // Only the last focus change of a batch says which desktop is in front.
        let focus = batch.iter().rev().find_map(|j| if let Job::Foreground(h) = j { Some(*h) } else { None });
        for job in batch {
            match job {
                Job::Arrived { hwnd, restored, asked } => match arrival(desktops.as_ref(), hwnd, restored, asked) {
                    Ok(a) => events.push(Event::Arrived(a)),
                    Err(why) if log::enabled() => {
                        log::info!("tiling: {} floats ({why})", winutil::exe_name(hwnd_of(hwnd)).unwrap_or_default())
                    }
                    Err(_) => {}
                },
                Job::Left { hwnd, minimized } => events.push(Event::Left { hwnd, minimized }),
                Job::Foreground(_) => {}
                Job::Look => {
                    let now = desktop_of(desktops.as_ref(), unsafe { GetForegroundWindow() }).or(front.clone());
                    front = now.clone();
                    events.push(Event::Desktop(now.unwrap_or_default()));
                }
                Job::Retile => {
                    events.extend(stamped(desktops.as_ref()).into_iter().map(Event::Arrived));
                    let now = desktop_of(desktops.as_ref(), unsafe { GetForegroundWindow() });
                    if let Some(now) = now.filter(|d| front.as_ref() != Some(d)) {
                        front = Some(now.clone());
                        events.push(Event::Desktop(now));
                    }
                }
            }
        }
        if let Some(h) = focus
            && let Some(now) = desktop_of(desktops.as_ref(), hwnd_of(h)).filter(|d| front.as_ref() != Some(d))
        {
            front = Some(now.clone());
            events.push(Event::Desktop(now));
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

/// The desktop a window is on, as text, or None when the shell doesn't say: a window on
/// every desktop, such as the taskbar, or a Windows with no desktops.
fn desktop_of(desktops: Option<&IVirtualDesktopManager>, h: HWND) -> Option<String> {
    let id = unsafe { desktops?.GetWindowDesktopId(h) }.ok()?;
    (id != GUID::zeroed()).then(|| format!("{id:?}"))
}

/// Whether smart tiling may take this window, and what it needs to know of it. Floats:
/// dialogs, owned and tool windows, fixed-size windows, apps under Left alone, windows
/// floated by hand, maximized ones, administrator windows, and windows a workspace or
/// app rule placed, unless the key asked for it.
fn arrival(
    desktops: Option<&IVirtualDesktopManager>,
    hwnd: isize,
    restored: bool,
    asked: bool,
) -> Result<Arrival, &'static str> {
    let h = hwnd_of(hwnd);
    if !unsafe { IsWindow(Some(h)) }.as_bool() {
        return Err("closed");
    }
    if winutil::is_floated(h) && !asked {
        return Err("floated by hand");
    }
    if winutil::ws_stamp(h).is_some() && !asked && !restored {
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
    let desktop = desktop_of(desktops, h).unwrap_or_default();
    Ok(Arrival { hwnd, frame, desktop, min, order: winutil::tile_stamp(h), restored })
}

/// The windows a tray that ran before tiled, oldest first, as arrivals.
fn stamped(desktops: Option<&IVirtualDesktopManager>) -> Vec<Arrival> {
    let mut found: Vec<Arrival> = winutil::top_level_windows()
        .into_iter()
        .filter(|h| winutil::tile_stamp(*h) != 0)
        .filter_map(|h| arrival(desktops, h.0 as isize, false, false).ok())
        .collect();
    found.sort_by_key(|a| a.order);
    found
}
