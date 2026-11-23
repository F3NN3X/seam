//! Keeping snapped windows in their zones after a screen or resolution change.
//! Experimental and off by default (`zones.refit`). It is a child of `snap`, so
//! it works on the same engine: a debounce timer, a worker that finds the
//! stamped windows, then one placement at a time through the usual steps.

use super::*;
use std::sync::Mutex;
use windows::core::BOOL;

/// A worker has listed the windows that carry a zone stamp.
pub const WM_APP_REFIT: u32 = WM_APP + 15;
/// A run has moved the last window it found back, for the restore offer to look after it.
pub const WM_APP_REFIT_DONE: u32 = WM_APP + 45;
/// Windows is still moving windows for a while after a display change.
const DEBOUNCE_MS: u32 = 1500;
/// The gap slider is let go once, but `config.patch` can come in bursts.
const REGAP_DEBOUNCE_MS: u32 = 150;

static FOUND: Mutex<Option<Vec<isize>>> = Mutex::new(None);

struct Run {
    geometry: Vec<Placed>,
    /// For a gap run: the screens' zones at the gap the windows were placed at.
    before: Vec<Placed>,
    /// A gap change asked for this run.
    regap: bool,
    found: usize,
    moved: usize,
    skipped: usize,
}

#[derive(Default)]
pub(super) struct Refit {
    timer: usize,
    /// A change came while a drag, Line Up or another run was going. Go once it ends.
    waiting: bool,
    /// A gap change asked for a run, so it goes whether or not `zones.refit` is on: the gap the
    /// windows were placed at, from the first change asked since the last run began.
    regap: Option<u32>,
    /// The run now starting is for a gap change, from this gap.
    starting: Option<u32>,
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

/// Whether a change of the gap from `old` to `new` moves the windows in zones.
/// Stored gaps past the slider's end are cut back when used, so they count as the end.
pub(super) fn gap_changed(old: u32, new: u32) -> bool {
    old.min(MAX_GAP) != new.min(MAX_GAP)
}

impl Refit {
    /// A gap change from `old` asks for a run. A change asked for while one is waiting keeps
    /// the earlier gap, which is where the windows still are.
    fn request(&mut self, old: u32) {
        self.regap = Some(self.regap.unwrap_or(old));
    }

    /// A run is about to begin: it takes the request, and a change that comes after asks again.
    fn take_start(&mut self) -> Option<u32> {
        self.starting = self.regap.take();
        self.starting
    }

    /// A gap change is waiting for a run, or a run for one is on its way or going.
    fn regap_live(&self) -> bool {
        self.regap.is_some() || self.starting.is_some() || self.run.as_ref().is_some_and(|r| r.regap)
    }
}

/// Whether something asks for a run: the setting, for screen changes, or a gap change.
fn run_wanted(refit_on: bool, regap_live: bool, active: bool) -> bool {
    active && (refit_on || regap_live)
}

/// Whether `update` drops what was lined up: stepping aside, nothing asks for a run, or zones
/// are off while a gap run is. A screen-change re-fit with zones off waits, as it always did.
fn run_stops(refit_on: bool, regap_live: bool, enabled: bool, aside: bool, standing_by: bool) -> bool {
    aside || standing_by || !(refit_on || regap_live) || (regap_live && !enabled)
}

/// Whether a gap run moves a window: one the snapping put where it still is, or one whose stamp
/// names a zone it sits in at the old gap. Never one that isn't on this desktop, which Windows
/// cloaks, or that is minimized. Anything else was moved by the user since.
fn regap_takes(minimized: bool, cloaked: bool, kept_in_place: bool, fits_old_gap: bool) -> bool {
    !minimized && !cloaked && (kept_in_place || fits_old_gap)
}

/// Screens or resolution changed, or the PC woke up. Starts the debounce, and
/// does nothing at all when the setting is off.
pub fn display_changed() {
    with_engine(|e| {
        e.adhoc_screens_changed();
        e.refit_schedule();
    });
    // A parked place may be on a screen that just came, and a place to come back to on one
    // that went: every parked window comes back now onto the screens there are, read fresh,
    // and parks again once `update` has them.
    tiled::tiling_display_changed(&seam::monitors::enumerate());
}

/// Line Up closed, so a re-fit that was waiting for it can go.
pub fn line_up_ended() {
    with_engine(Engine::refit_resume);
}

/// A switch ended, so a re-fit that was waiting for it can go.
pub fn switch_ended() {
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
    unsafe {
        with_engine(|e| {
            if e.refit.timer == id {
                let _ = KillTimer(None, id);
                e.refit.timer = 0;
                e.refit_begin();
            }
        });
    }
}

unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<isize>);
        if winutil::has_zone_stamp(h) {
            list.push(h.0 as isize);
        }
        BOOL(1)
    }
}

/// Runs on the worker for a re-fit, and on the tray for a split or pull-out that
/// looks for stamps the snapping holds no record of.
fn stamped_windows() -> Vec<isize> {
    let mut list: Vec<isize> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

impl Engine {
    /// Windows on the screen at `bounds` that carry a zone stamp the snapping holds
    /// no record of, as after a restart of the tray: the window, its zones' first
    /// and last, and where it is. `skip` are left out.
    /// Only stamps from an earlier run can be such, so once a pass finds none the
    /// windows aren't listed again.
    pub(super) fn stamped_only(&mut self, bounds: Rect, skip: &[isize]) -> Vec<(isize, usize, usize, Rect)> {
        if self.no_stray_stamps {
            return Vec::new();
        }
        let started = log::enabled().then(Instant::now);
        let known = |key: &isize| self.snapped.contains_key(key) || skip.contains(key);
        let found = |key: isize| {
            let h = hwnd_of(key);
            let set = winutil::zone_stamp(h);
            let (from, to) = (*set.iter().min()?, *set.iter().max()?);
            let at = if winutil::is_minimized(h) { winutil::normal_rect(h) } else { winutil::visible_rect(h) }?;
            bounds.contains(at.x + at.w / 2, at.y + at.h / 2).then_some((key, from, to, at))
        };
        let strays: Vec<isize> = stamped_windows().into_iter().filter(|key| !known(key)).collect();
        let list = strays.iter().copied().filter_map(found).collect();
        if let Some(started) = started {
            let us = started.elapsed().as_secs_f64() * 1e6;
            log::info!("zone stamps: looked at {} unrecorded windows in {us:.0}us", strays.len());
        }
        self.no_stray_stamps = strays.is_empty();
        list
    }

    fn refit_schedule(&mut self) {
        if !self.settings.refit || !self.active() {
            return;
        }
        self.refit_arm(DEBOUNCE_MS);
    }

    /// The gap changed from `old`, by the slider or `config.patch`: put the windows in zones at
    /// the new gap, once, with `zones.refit` on or off. One that comes during a run runs again.
    pub(super) fn regap(&mut self, old: u32) {
        if !self.active() {
            return;
        }
        self.refit.request(old);
        self.refit_arm(REGAP_DEBOUNCE_MS);
    }

    /// A run can go: a screen change with the setting on, or a gap change, with zones active.
    fn refit_wanted(&self) -> bool {
        run_wanted(self.settings.refit, self.refit.regap_live(), self.active())
    }

    /// `update` should drop what was lined up. Not the opposite of `refit_wanted`: that one
    /// asks whether zones are active right now, this one whether to give up what is queued.
    pub(super) fn refit_unwanted(&self) -> bool {
        run_stops(self.settings.refit, self.refit.regap_live(), self.enabled, self.aside.is_some(), self.standing_by)
    }

    fn refit_arm(&mut self, ms: u32) {
        if self.refit.timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.refit.timer);
            }
        }
        self.refit.timer = unsafe { SetTimer(None, 0, ms, Some(on_debounce)) };
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
        if !self.active() {
            self.refit.regap = None;
        }
        if !self.refit_wanted() {
            return;
        }
        // A switch places windows itself, and stamps them where it puts them.
        if self.drag.is_some()
            || overlay::is_active()
            || crate::switcher::switching()
            || self.refit.busy
            || self.refit.run.is_some()
        {
            self.refit.waiting = true;
            return;
        }
        self.refit.waiting = false;
        self.refit.busy = true;
        self.refit.take_start();
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
            self.refit.starting = None;
        }
    }

    fn refit_found(&mut self, list: Vec<isize>) {
        self.refit.busy = false;
        if !self.refit_wanted() {
            self.refit.starting = None;
            return;
        }
        let geometry = self.geometry();
        let from = self.refit.starting.take();
        let before = from.map(|gap| self.geometry_at(gap)).unwrap_or_default();
        self.refit.run = Some(Run { geometry, before, regap: from.is_some(), found: list.len(), moved: 0, skipped: 0 });
        self.refit.queue = list.into();
        self.refit_pump();
    }

    /// Start a run that was waiting, or carry on with the queue.
    pub(super) fn refit_resume(&mut self) {
        if !self.refit.queue.is_empty() {
            self.refit_pump();
            return;
        }
        let free = self.drag.is_none()
            && self.refit.run.is_none()
            && !self.refit.busy
            && !overlay::is_active()
            && !crate::switcher::switching();
        if self.refit.waiting && free && self.refit_wanted() {
            self.refit.waiting = false;
            self.refit_arm(if self.refit.regap.is_some() { REGAP_DEBOUNCE_MS } else { DEBOUNCE_MS });
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
                if run.regap {
                    log::info!("zones gap: moved {} of {} windows to the new gap", run.moved, run.found);
                } else {
                    log::info!("zone refit: {} found, {} moved, {} skipped", run.found, run.moved, run.skipped);
                }
                self.sync_dividers();
                // Every window of a stack was fitted on its own, so its top goes back over the rest.
                self.stack_raise_tops();
                // The restore offer waits for a screen change's re-fit, not for a gap's.
                if !run.regap {
                    unsafe {
                        let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_REFIT_DONE, WPARAM(0), LPARAM(0));
                    }
                }
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
        if run.regap {
            // A tiled screen is laid out again by the tiling, at the new gap.
            if self.tiling_on(&g.id) {
                return false;
            }
            let visible = winutil::visible_rect(hwnd).unwrap_or(now);
            let sized = winutil::is_resizable(hwnd);
            let key = hwnd.0 as isize;
            let kept = self.snapped.get(&key).is_some_and(|s| close(visible, s.at, sized));
            let old_span = run
                .before
                .iter()
                .find(|b| b.id == g.id)
                .and_then(|b| set.iter().map(|i| b.zones.get(*i).copied()).collect::<Option<Vec<_>>>())
                .and_then(|zs| zs.into_iter().reduce(zones::span));
            let fits = old_span.is_some_and(|span| close(visible, span, sized));
            if !regap_takes(winutil::is_minimized(hwnd), winutil::is_cloaked(hwnd), kept, fits) {
                return false;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_change_asks_for_a_run_and_the_same_gap_does_not() {
        assert!(gap_changed(8, 0));
        assert!(gap_changed(0, 16));
        assert!(!gap_changed(8, 8));
        assert!(!gap_changed(MAX_GAP + 5, MAX_GAP + 9), "both are cut to the end");
        assert!(gap_changed(MAX_GAP + 5, 8));
    }

    #[test]
    fn a_gap_change_during_a_run_runs_again() {
        let mut r = Refit::default();
        r.request(8);
        assert_eq!(r.take_start(), Some(8), "the run takes the request");
        assert!(r.regap.is_none());
        r.request(16);
        assert_eq!(r.regap, Some(16), "a change during the run asks again");
        assert!(run_wanted(false, r.regap_live(), true), "so it is still wanted with the setting off");
        assert_eq!(r.take_start(), Some(16));
    }

    #[test]
    fn a_burst_keeps_the_gap_the_windows_are_at() {
        let mut r = Refit::default();
        r.request(8);
        r.request(12);
        assert_eq!(r.regap, Some(8));
    }

    #[test]
    fn nothing_is_wanted_without_the_setting_or_a_gap_change() {
        assert!(!run_wanted(false, false, true));
        assert!(run_wanted(true, false, true));
        assert!(run_wanted(false, true, true));
        assert!(!run_wanted(true, true, false), "zones aren't active");
    }

    #[test]
    fn zones_off_stops_a_gap_run_and_stepping_aside_stops_any() {
        assert!(run_stops(false, true, false, false, false), "zones off");
        assert!(!run_stops(true, false, false, false, false), "a screen change re-fit waits as before");
        assert!(run_stops(true, false, true, true, false), "stepping aside");
        assert!(run_stops(true, true, true, false, true), "standing by");
        assert!(run_stops(false, false, true, false, false), "nothing asks");
        assert!(!run_stops(false, true, true, false, false));
    }

    #[test]
    fn a_gap_run_moves_what_is_in_place_or_fits_the_old_gap() {
        assert!(regap_takes(false, false, true, false), "the snapping's own");
        assert!(regap_takes(false, false, false, true), "stamped and on the old gap's zone");
        assert!(!regap_takes(false, false, false, false), "moved away since");
        assert!(!regap_takes(true, false, true, true), "minimized");
        assert!(!regap_takes(false, true, true, true), "cloaked, as on another desktop");
    }
}
