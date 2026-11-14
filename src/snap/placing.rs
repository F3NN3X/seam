//! Moving windows into their zones, a step at a time, and back out.

use super::*;

impl Engine {
    /// The window stopped being in a zone, so forget it, including the stamp on the window.
    pub(super) fn unsnap(&mut self, hwnd: HWND, stamped: bool) {
        self.snapped.remove(&(hwnd.0 as isize));
        if stamped {
            winutil::clear_zone_stamp(hwnd);
        }
        self.sync_dividers();
    }

    pub(super) fn snap_to(
        &mut self,
        hwnd: HWND,
        g: &Placed,
        (from, to): (usize, usize),
        rect: Rect,
        was: Option<Option<Old>>,
    ) {
        let key = hwnd.0 as isize;
        // A window put in a zone a moment ago isn't there yet, but its old size still counts.
        let known = self.current(hwnd).is_some() || self.pending.as_ref().is_some_and(|p| p.hwnd == key);
        let old = match was {
            Some(old) => old,
            None => match self.snapped.get(&key).filter(|_| known) {
                Some(s) => s.old,
                None if winutil::is_maximized(hwnd) => None,
                None => winutil::window_rect(hwnd).map(|r| (r.w, r.h, winutil::monitor_dpi(hwnd))),
            },
        };
        // The screen's DPI, not the window's: a window that ignores DPI needs no extra step.
        let crosses = winutil::monitor_dpi(hwnd) != g.dpi;
        // Into zones, it's out of any ad-hoc group.
        self.leave_adhoc(key);
        self.place(hwnd, rect, crosses, false);
        self.snapped.insert(key, Snapped { old, screen: g.id.clone(), from, to, at: rect });
        // The zones the window covers, kept on the window so a restart of the tray remembers them.
        winutil::stamp_zones(hwnd, &refit::covered(&g.cells, from, to));
        if self.snapped.len() > MAX_SNAPPED {
            // Windows that closed since. A system-wide destroy hook would cost more than this.
            self.snapped.retain(|k, _| unsafe { IsWindow(Some(hwnd_of(*k))) }.as_bool());
            if self.snapped.len() > MAX_SNAPPED {
                self.snapped.clear();
            }
        }
        self.sync_dividers();
    }

    /// Put the window's visible frame on `target`, a step at a time with the
    /// app's own reactions in between (D8).
    pub(super) fn place(&mut self, hwnd: HWND, target: Rect, crosses_dpi: bool, quiet: bool) -> bool {
        let maximize = self.maximize.remove(&(hwnd.0 as isize));
        let Some(before) = winutil::window_rect(hwnd) else { return false };
        if winutil::visible_rect(hwnd) == Some(target) {
            if maximize {
                winutil::maximize(hwnd);
            }
            return false;
        }
        let mut steps = VecDeque::new();
        if winutil::is_maximized(hwnd) {
            steps.push_back(Step::Restore);
        }
        if crosses_dpi {
            steps.push_back(Step::Move);
        }
        steps.push_back(Step::Fit);
        steps.push_back(Step::Verify);
        if let Some(earlier) = self.pending.take() {
            unsafe {
                let _ = KillTimer(None, earlier.timer);
                // Another window's placement is cut short, so land it where it was going.
                let other = hwnd_of(earlier.hwnd);
                let unfinished = earlier.steps.iter().any(|s| matches!(s, Step::Fit));
                if earlier.hwnd != hwnd.0 as isize && unfinished && IsWindow(Some(other)).as_bool() {
                    winutil::fit(other, earlier.target);
                }
            }
        }
        self.pending =
            Some(Pending { hwnd: hwnd.0 as isize, target, before, steps, retries: 0, quiet, maximize, timer: 0 });
        self.advance_step();
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_WINDOWS_CHANGED, WPARAM(0), LPARAM(0));
        }
        true
    }

    /// The settle timer fired. When the placement is over, the next re-fit can start.
    pub(super) fn advance(&mut self) {
        self.advance_step();
        if self.pending.is_none() {
            self.window_pump();
        }
        if self.pending.is_none() {
            self.refit_pump();
        }
    }

    /// Start window control's next move once the one before is over.
    pub(super) fn window_pump(&mut self) {
        while self.pending.is_none() && self.drag.is_none() {
            let Some(p) = self.queued.pop_front() else { break };
            self.place_queued(p);
        }
    }

    /// One of window control's moves, posted at once without the steps between, and
    /// recorded where it goes. Returns the window and its frame's new place.
    pub(super) fn land_now(&mut self, p: Placement) -> Option<(isize, Rect)> {
        let h = hwnd_of(p.hwnd);
        if !unsafe { IsWindow(Some(h)) }.as_bool() {
            return None;
        }
        // Across a DPI change, or out of maximized, it takes the steps with their checks.
        if winutil::is_maximized(h) || winutil::monitor_dpi(h) != p.dpi {
            let at = (p.hwnd, p.target);
            self.place_queued(p);
            return Some(at);
        }
        let window = winutil::window_rect(h)?;
        let visible = winutil::visible_rect(h).unwrap_or(window);
        let r = zones::frame_adjust(p.target, window, visible);
        let size = if winutil::is_resizable(h) { SWP_NOACTIVATE } else { SWP_NOACTIVATE | SWP_NOSIZE };
        unsafe {
            let _ = SetWindowPos(h, None, r.x, r.y, r.w, r.h, SWP_ASYNCWINDOWPOS | SWP_NOZORDER | size);
        }
        let geometry = self.geometry();
        match p.zone.and_then(|(id, from, to)| Some((geometry.iter().find(|g| g.id == id)?, from, to))) {
            Some((g, from, to)) if to < g.zones.len() => {
                self.keep_snapped(p.hwnd, &g.id, (from, to), p.target);
                winutil::stamp_zones(h, &refit::covered(&g.cells, from, to));
            }
            _ => self.unsnap(h, winutil::has_zone_stamp(h)),
        }
        Some((p.hwnd, p.target))
    }

    pub(super) fn put_back(&mut self, p: Placement, old: Option<Old>) {
        let hwnd = hwnd_of(p.hwnd);
        let Some((id, from, to)) = p.zone else { return };
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() || winutil::is_minimized(hwnd) || winutil::is_maximized(hwnd) {
            return;
        }
        let geometry = self.geometry();
        let Some(g) = geometry.iter().find(|g| g.id == id && to < g.zones.len()) else { return };
        let on = winutil::visible_rect(hwnd).is_some_and(|r| g.bounds.contains(r.x + r.w / 2, r.y + r.h / 2));
        let held = self
            .snapped
            .iter()
            .any(|(k, x)| *k != p.hwnd && x.screen == id && refit::covered(&g.cells, x.from, x.to).contains(&from));
        if held {
            log::info!("pull out: Undo left the window where it is, another window holds its zone");
            return;
        }
        if on {
            self.snap_to(hwnd, g, (from, to), p.target, Some(old));
        }
    }

    /// Start one of window control's moves, recorded where it goes: snapped in its
    /// zones, or loose.
    fn place_queued(&mut self, p: Placement) {
        let hwnd = hwnd_of(p.hwnd);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return;
        }
        let geometry = self.geometry();
        match p.zone.and_then(|(id, from, to)| Some((geometry.iter().find(|g| g.id == id)?, from, to))) {
            Some((g, from, to)) if to < g.zones.len() => self.snap_to(hwnd, g, (from, to), p.target, None),
            _ => {
                // The screen's DPI, as in `snap_to`.
                self.place(hwnd, p.target, winutil::monitor_dpi(hwnd) != p.dpi, false);
                self.unsnap(hwnd, winutil::has_zone_stamp(hwnd));
            }
        }
    }

    fn advance_step(&mut self) {
        let Some(mut p) = self.pending.take() else { return };
        if p.timer != 0 {
            unsafe {
                let _ = KillTimer(None, p.timer);
            }
            p.timer = 0;
        }
        let hwnd = hwnd_of(p.hwnd);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return;
        }
        match p.steps.pop_front() {
            Some(Step::Restore) => winutil::restore(hwnd),
            Some(Step::Move) => winutil::move_to(hwnd, p.target),
            Some(Step::Fit) => winutil::fit(hwnd, p.target),
            Some(Step::Verify) => {
                if winutil::window_rect(hwnd) == Some(p.before) {
                    if !p.quiet {
                        self.blocked(hwnd);
                    }
                    return;
                }
                // It moved, but an app that answered a DPI change late can leave it off by its margins.
                let (visible, sized) = (winutil::visible_rect(hwnd), winutil::is_resizable(hwnd));
                let landed = visible.is_some_and(|v| close(v, p.target, sized));
                if landed || p.retries >= MAX_RETRIES {
                    // Still bigger than its place after the retries: that's as small as the app goes.
                    if let (false, true, Some(v)) = (landed, sized, visible) {
                        minsize::observed(hwnd, v, p.target, CLOSE_ENOUGH);
                    }
                    if let (false, Some(v)) = (landed, visible) {
                        let t = p.target;
                        let (dx, dy, dw, dh) = (v.x - t.x, v.y - t.y, v.w - t.w, v.h - t.h);
                        log::info!("zone place: landed {dx},{dy} px off its place, {dw}x{dh} px in size");
                    }
                    if p.maximize {
                        winutil::maximize(hwnd);
                    }
                    return;
                }
                p.retries += 1;
                p.steps.extend([Step::Fit, Step::Verify]);
            }
            None => return,
        }
        p.timer = unsafe { SetTimer(None, 0, SETTLE_MS, Some(on_settle)) };
        self.pending = Some(p);
    }

    /// Say so once per app per run, because a window that won't move is nearly
    /// always one running as administrator (D12).
    pub(super) fn blocked(&mut self, hwnd: HWND) {
        log::info!("a window didn't move, probably because it runs as administrator");
        let path = winutil::exe_path_of(winutil::process_id(hwnd)).unwrap_or_default();
        if self.warned.insert(path) {
            *BLOCKED_APP.lock().unwrap_or_else(|e| e.into_inner()) = winutil::app_name(hwnd);
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_SNAP_BLOCKED, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub(super) fn move_foreground(&mut self, next: bool) -> Result<(), String> {
        if let Some(why) = &self.aside {
            return Err(why.clone());
        }
        if !self.enabled {
            return Err("Zones are off".into());
        }
        let geometry = self.geometry();
        if geometry.is_empty() {
            return Err("No screen has zones yet. Pick a layout on the Zones page".into());
        }
        let hwnd = unsafe { GetForegroundWindow() };
        if !winutil::is_snappable(hwnd, std::process::id(), &self.exclude) {
            return Err("That window can't be moved into a zone".into());
        }
        // Every zone in order, screen by screen, so the last one wraps onto the next screen.
        let flat: Vec<(usize, usize)> =
            geometry.iter().enumerate().flat_map(|(s, g)| (0..g.zones.len()).map(move |z| (s, z))).collect();
        let position = |id: &str, zone: usize| geometry.iter().position(|g| g.id == id).map(|s| (s, zone));
        let snapped = self.current(hwnd).and_then(|s| position(&s.screen, if next { s.to } else { s.from }));
        let centre = winutil::visible_rect(hwnd).map(|r| (r.x + r.w / 2, r.y + r.h / 2));
        let under = centre.and_then(|(x, y)| {
            let s = geometry.iter().position(|g| g.bounds.contains(x, y))?;
            Some((s, zones::zone_at(&geometry[s].cells, x, y)?))
        });
        let n = flat.len();
        let index = match (snapped, under) {
            (Some(at), _) => {
                flat.iter().position(|f| *f == at).map(|i| if next { (i + 1) % n } else { (i + n - 1) % n })
            }
            (None, Some(at)) => flat.iter().position(|f| *f == at),
            (None, None) => Some(if next { 0 } else { n - 1 }),
        };
        let (screen, zone) = flat[index.unwrap_or(0)];
        let g = &geometry[screen];
        self.snap_to(hwnd, g, (zone, zone), g.zones[zone], None);
        Ok(())
    }
}
