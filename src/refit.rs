//! Keeping snapped windows in their zones after a screen or resolution change.
//! Experimental and off by default (`zones.refit`). It is a child of `snap`, so
//! it works on the same engine: a debounce timer, a worker that finds the
//! stamped windows, then one placement at a time through the usual steps.

use super::*;
use std::sync::Mutex;
use windows::core::BOOL;

/// A worker has listed the windows that carry a zone stamp.
pub const WM_APP_REFIT: u32 = WM_APP + 15;
/// Windows is still moving windows for a while after a display change.
const DEBOUNCE_MS: u32 = 1500;

static FOUND: Mutex<Option<Vec<isize>>> = Mutex::new(None);

struct Run {
    geometry: Vec<Placed>,
    found: usize,
    moved: usize,
    skipped: usize,
}

#[derive(Default)]
pub(super) struct Refit {
    timer: usize,
    /// A change came while a drag, Line Up or another run was going. Go once it ends.
    waiting: bool,
    /// A worker is listing windows.
    busy: bool,
    queue: VecDeque<isize>,
    run: Option<Run>,
}

/// The zones a window covers when it spans `from` to `to`: every zone inside their bounding box.
pub(super) fn covered(cells: &[Rect], from: usize, to: usize) -> Vec<usize> {
    let (Some(a), Some(b)) = (cells.get(from), cells.get(to)) else { return Vec::new() };
    let r = zones::span(*a, *b);
    let inside = |c: &Rect| c.x >= r.x && c.y >= r.y && c.x + c.w <= r.x + r.w && c.y + c.h <= r.y + r.h;
    cells.iter().enumerate().filter(|(_, c)| inside(c)).map(|(i, _)| i).collect()
}

/// Screens or resolution changed, or the PC woke up. Starts the debounce, and
/// does nothing at all when the setting is off.
pub fn display_changed() {
    with_engine(Engine::refit_schedule);
}

/// Line Up closed, so a re-fit that was waiting for it can go.
pub fn line_up_ended() {
    with_engine(Engine::refit_resume);
}

/// The worker's list is ready.
pub fn found() {
    with_engine(|e| {
        let list = FOUND.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
        e.refit_found(list);
    });
}

unsafe extern "system" fn on_debounce(_: HWND, _: u32, id: usize, _: u32) {
    with_engine(|e| {
        if e.refit.timer == id {
            let _ = KillTimer(None, id);
            e.refit.timer = 0;
            e.refit_begin();
        }
    });
}

unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<isize>);
    if winutil::has_zone_stamp(h) {
        list.push(h.0 as isize);
    }
    BOOL(1)
}

/// Runs on the worker.
fn stamped_windows() -> Vec<isize> {
    let mut list: Vec<isize> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

impl Engine {
    fn refit_schedule(&mut self) {
        if !self.settings.refit || !self.enabled {
            return;
        }
        if self.refit.timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.refit.timer);
            }
        }
        self.refit.timer = unsafe { SetTimer(None, 0, DEBOUNCE_MS, Some(on_debounce)) };
    }

    /// The setting went off: drop everything that was lined up.
    pub(super) fn refit_stop(&mut self) {
        if self.refit.timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.refit.timer);
            }
        }
        self.refit = Refit { busy: self.refit.busy, ..Refit::default() };
    }

    fn refit_begin(&mut self) {
        if !self.settings.refit || !self.enabled {
            return;
        }
        if self.drag.is_some() || overlay::is_active() || self.refit.busy || self.refit.run.is_some() {
            self.refit.waiting = true;
            return;
        }
        self.refit.waiting = false;
        self.refit.busy = true;
        let owner = self.owner;
        let spawned = std::thread::Builder::new().name("seam-refit".into()).spawn(move || {
            let list = stamped_windows();
            *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(owner)), WM_APP_REFIT, WPARAM(0), LPARAM(0));
            }
        });
        if spawned.is_err() {
            self.refit.busy = false;
        }
    }

    fn refit_found(&mut self, list: Vec<isize>) {
        self.refit.busy = false;
        if !self.settings.refit || !self.enabled {
            return;
        }
        let geometry = self.geometry();
        self.refit.run = Some(Run { geometry, found: list.len(), moved: 0, skipped: 0 });
        self.refit.queue = list.into();
        self.refit_pump();
    }

    /// Start a run that was waiting, or carry on with the queue.
    pub(super) fn refit_resume(&mut self) {
        if !self.refit.queue.is_empty() {
            self.refit_pump();
            return;
        }
        let free = self.drag.is_none() && self.refit.run.is_none() && !self.refit.busy && !overlay::is_active();
        if self.refit.waiting && free {
            self.refit.waiting = false;
            self.refit_schedule();
        }
    }

    /// One window at a time: the next starts when the placement before it is over.
    pub(super) fn refit_pump(&mut self) {
        while self.pending.is_none() && self.drag.is_none() {
            let Some(key) = self.refit.queue.pop_front() else { break };
            let moved = self.refit_one(hwnd_of(key));
            if let Some(run) = self.refit.run.as_mut() {
                if moved {
                    run.moved += 1;
                } else {
                    run.skipped += 1;
                }
            }
        }
        if self.pending.is_none() && self.drag.is_none() && self.refit.queue.is_empty() {
            if let Some(run) = self.refit.run.take() {
                log::info!("zone refit: {} found, {} moved, {} skipped", run.found, run.moved, run.skipped);
            }
            self.refit_resume();
        }
    }

    /// Put one stamped window back in its zones. Returns whether a move started.
    fn refit_one(&mut self, hwnd: HWND) -> bool {
        let set = winutil::zone_stamp(hwnd);
        let usable = !set.is_empty()
            && unsafe { IsWindow(Some(hwnd)) }.as_bool()
            && winutil::is_snappable(hwnd, std::process::id(), &self.exclude)
            && !winutil::is_maximized(hwnd)
            && !winutil::is_elevated_over_us(hwnd);
        let Some(now) = winutil::window_rect(hwnd).filter(|_| usable) else { return false };
        let (x, y) = (now.x + now.w / 2, now.y + now.h / 2);
        let Some(run) = self.refit.run.as_ref() else { return false };
        // The screen it is on now, and that screen's layout.
        let Some(g) = run.geometry.iter().find(|g| g.bounds.contains(x, y)) else { return false };
        // A zone that isn't in this layout means the window isn't ours to move.
        if set.iter().any(|i| *i >= g.zones.len()) {
            return false;
        }
        let target = set.iter().map(|i| g.zones[*i]).reduce(zones::span);
        let (screen, dpi) = (g.id.clone(), g.dpi);
        let (Some(target), Some(from), Some(to)) = (target, set.iter().min(), set.iter().max()) else { return false };
        let crosses = winutil::monitor_dpi(hwnd) != dpi;
        let moved = self.place(hwnd, target, crosses, true);
        // The size it had before it was snapped stays what it was.
        let entry = self.snapped.entry(hwnd.0 as isize).or_insert(Snapped {
            old: None,
            screen: screen.clone(),
            from: *from,
            to: *to,
            at: target,
        });
        (entry.screen, entry.from, entry.to, entry.at) = (screen, *from, *to, target);
        moved
    }
}
