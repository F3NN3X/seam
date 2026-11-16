//! The gaps minimized windows leave on a tiled screen: when each closes (D567), the
//! timer that wakes for the next, and the windows a workspace switch left minimized,
//! looked at as it ends and again once its grace is over (D564).

use super::*;

/// How soon a timer that found the engine busy tries again, in milliseconds.
const RETRY_MS: u32 = 50;

/// A workspace switch ended: the arrivals kept meanwhile are taken, except the windows
/// the switch placed (D564).
pub fn tiling_switch_ended() {
    crate::tiler::switch_ended();
    with_engine(|e| {
        e.tiled_rest_after_switch();
        let kept = std::mem::take(&mut e.tiling.paused).into_iter().map(|a| (a.hwnd, a)).collect();
        let placed = |h: isize| winutil::ws_stamp(hwnd_of(h)).is_some();
        for (_, a) in tiling::after_pause(kept, placed) {
            e.tile_arrived(a);
        }
        e.scroll_focus_waited();
        // A slow app's minimize may land inside the grace, which ignores it: look again once it ends.
        if e.tiling.grace_timer != 0 {
            unsafe {
                let _ = KillTimer(None, e.tiling.grace_timer);
            }
        }
        let ms = (crate::tiler::SWITCH_GRACE + Duration::from_millis(50)).as_millis() as u32;
        e.tiling.grace_timer = unsafe { SetTimer(None, 0, ms, Some(on_grace)) };
    });
}

/// The grace after a switch ended: a tiled window it left minimized since rests too.
/// Repeats until the engine is free to take it, as `on_gaps` does.
unsafe extern "system" fn on_grace(_: HWND, _: u32, id: usize, _: u32) {
    let taken = with_engine(|e| {
        if e.tiling.grace_timer == id {
            e.tiling.grace_timer = 0;
            // A switch started since ends with a look of its own.
            if !crate::switcher::switching() {
                e.tiled_rest_after_switch();
            }
        }
    });
    retry_or_kill(taken.is_some(), id, on_grace);
}

/// A timer callback that ran kills its timer; one that found the engine busy tries again
/// soon, whatever the timer's own delay was.
fn retry_or_kill(ran: bool, id: usize, callback: unsafe extern "system" fn(HWND, u32, usize, u32)) {
    unsafe {
        if ran {
            let _ = KillTimer(None, id);
        } else {
            SetTimer(None, id, RETRY_MS, Some(callback));
        }
    }
}

/// The timer repeats until the engine is free to take it, so a busy moment drops nothing.
unsafe extern "system" fn on_gaps(_: HWND, _: u32, id: usize, _: u32) {
    let taken = with_engine(|e| {
        if e.tiling.gap_timer == id {
            e.tiling.gap_timer = 0;
            e.tiled_gaps();
        }
    });
    retry_or_kill(taken.is_some(), id, on_gaps);
}

impl Engine {
    /// A switch has ended: a tiled window it left minimized rests, as one minimized by
    /// hand does, so its tile closes after the usual wait and a restore takes it back.
    /// It never counts in a burst (D564, D567).
    pub(super) fn tiled_rest_after_switch(&mut self) {
        let now = Instant::now();
        for i in 0..self.tiling.trees.len() {
            if !self.here(i) {
                continue;
            }
            if self.tiling.trees[i].strip.is_some() {
                self.scroll_rest_after_switch(i, now);
                continue;
            }
            let windows: Vec<isize> = self.tiling.trees[i].tree.windows().collect();
            for key in windows.into_iter().filter(|k| winutil::is_minimized(hwnd_of(*k))) {
                let t = &mut self.tiling.trees[i];
                let (Some(zone), Some(tree)) = (t.tree.zone_of(key), t.tree.vacate(key)) else { continue };
                t.tree = tree;
                t.resting.retain(|(k, ..)| *k != key);
                t.resting.push((key, zone, now, false));
                self.snapped.remove(&key);
                log::info!("tiling: a switch left a tiled window minimized, so its tile closes soon");
            }
        }
        self.tiled_arm_gaps();
        self.tiled_sync();
    }

    /// The tiles of tree `i`'s minimized windows that are still empty, for `tiling::gaps_due`.
    pub(super) fn resting_tiles(&self, i: usize) -> Vec<tiling::Resting> {
        let t = &self.tiling.trees[i];
        let empty = |z: usize| t.tree.tiles.get(z).is_some_and(|x| x.hwnd.is_none());
        t.resting.iter().filter(|(_, z, ..)| *z != usize::MAX && empty(*z)).map(|(_, z, at, b)| (*z, *at, *b)).collect()
    }

    /// Wake once, when the next minimized window's tile may close (`tiling::next_gap`); a
    /// burst's wait is longer (`BURST_KEEPS`). A tree with no window, as after Win+D, waits
    /// for one to come back, and with nothing due no timer runs.
    pub(super) fn tiled_arm_gaps(&mut self) {
        let next = (0..self.tiling.trees.len())
            .filter(|&i| self.tiling.trees[i].tree.windows().next().is_some())
            .filter_map(|i| tiling::next_gap(&self.resting_tiles(i), self.tiling.trees[i].back))
            .min();
        if self.tiling.gap_timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.tiling.gap_timer);
            }
            self.tiling.gap_timer = 0;
        }
        if let Some(at) = next {
            let ms = at.saturating_duration_since(Instant::now()).as_millis() as u32 + 1;
            self.tiling.gap_timer = unsafe { SetTimer(None, 0, ms, Some(on_gaps)) };
        }
    }

    /// The tiles minimized windows left close once `tiling::gaps_due` says, so the
    /// others fill the gap; a burst such as Win+D keeps them up to `BURST_KEEPS` for the windows to come
    /// back to (D567). The move goes through `tiled_apply`, so its rule holds.
    pub(super) fn tiled_gaps(&mut self) {
        let now = Instant::now();
        for i in 0..self.tiling.trees.len() {
            if self.tiling.trees[i].tree.windows().next().is_none() {
                continue;
            }
            let due = tiling::gaps_due(&self.resting_tiles(i), self.tiling.trees[i].back, now);
            if due.is_empty() {
                continue;
            }
            let t = &mut self.tiling.trees[i];
            for zone in due {
                let Some(tree) = t.tree.remove_zone(zone) else { continue };
                t.tree = tree;
                t.closed_zone(zone);
            }
            self.tiled_unstack(i);
            self.tiled_apply(i);
        }
        // Those that weren't due yet.
        self.tiled_arm_gaps();
    }
}
