//! The hooks, timers and window events behind workspace drift and auto-apply.

use super::*;

impl Wm {
    /// Only what a feature that is on needs: focus changes for the border and for
    /// pins that apps may paint over, closed windows while anything is pinned,
    /// shown windows while a switch waits for them, moves, snaps and minimizes while a
    /// workspace is in use, opened windows while auto-apply runs or an app rule exists, focus changes and
    /// minimizes while spotlight is on, and the refresh timer while the first applies.
    pub(super) fn sync_hooks(&mut self) {
        let painting = self.painting();
        let auto = self.opening();
        if auto && self.name_hook.is_none() {
            // What is there already never counts as opening.
            self.seen_before = winutil::top_level_windows().into_iter().map(|h| h.0 as isize).collect();
        } else if !auto {
            self.seen_before.clear();
            self.settling.clear();
            self.stop_settle_timer();
        }
        let (show, destroy) = (EVENT_OBJECT_SHOW, EVENT_OBJECT_DESTROY);
        let foreground = EVENT_SYSTEM_FOREGROUND;
        sync_hook(&mut self.foreground_hook, foreground, foreground, painting || self.spotlight);
        sync_hook(&mut self.destroy_hook, destroy, destroy, !self.pinned.is_empty() || auto);
        sync_hook(&mut self.show_hook, show, show, self.shown_to.is_some() || auto);
        sync_hook(&mut self.name_hook, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, auto);
        let active = self.workspace_active;
        sync_hook(
            &mut self.movesize_hook,
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MOVESIZEEND,
            active || self.app_rules,
        );
        let minimize = (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND);
        sync_hook(&mut self.minimize_hook, minimize.0, minimize.1, active || self.spotlight);
        // Win+arrow snaps and the maximize button send no drag or minimize event.
        let location = EVENT_OBJECT_LOCATIONCHANGE;
        sync_hook(&mut self.location_hook, location, location, active);
        if !active {
            self.relocated.clear();
            self.stop_relocated_timer();
        }
        match (painting, self.refresh_timer) {
            (true, 0) => self.refresh_timer = unsafe { SetTimer(None, 0, REFRESH_EVERY_MS, None) },
            (false, id) if id != 0 => {
                let _ = unsafe { KillTimer(None, id) };
                self.refresh_timer = 0;
                self.stop_repaint();
            }
            _ => {}
        }
    }

    /// The focus border or a pin colour is painted, so focus changes matter to it.
    fn painting(&self) -> bool {
        let wanted = (self.focus_border && self.focus_colour.is_some())
            || (!self.pinned.is_empty() && self.pin_colour.is_some());
        wanted && borders_supported()
    }

    pub(super) fn auto_applying(&self) -> bool {
        self.workspace_active && self.auto_apply
    }

    /// Windows that open are looked at: auto-apply runs or an app rule exists.
    fn opening(&self) -> bool {
        self.auto_applying() || self.app_rules
    }

    fn stop_settle_timer(&mut self) {
        if self.settle_timer != 0 {
            let _ = unsafe { KillTimer(None, self.settle_timer) };
            self.settle_timer = 0;
        }
    }

    /// Wake when the next opened window has settled.
    fn arm_settle_timer(&mut self) {
        let Some(next) = self.settling.iter().map(|(_, at)| *at).min() else { return self.stop_settle_timer() };
        let ms = next.saturating_duration_since(Instant::now()).as_millis().max(1) as u32;
        self.settle_timer = unsafe { SetTimer(None, self.settle_timer, ms, None) };
    }

    /// A window showed or got its title. If it's new, it's placed once it has
    /// settled. A window that was there, a minimized one and one already stamped
    /// aren't opens. One that opens during a switch waits on the worker for it to end.
    fn opened(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.seen_before.contains(&key) {
            return;
        }
        // A window still hidden gets its title first. It counts once it shows.
        if !unsafe { IsWindowVisible(h) }.as_bool() {
            return;
        }
        self.seen_before.insert(key);
        if winutil::is_minimized(h) || winutil::ws_stamp(h).is_some() {
            return;
        }
        // A window that shows while a switch runs is the switch's, even if it settles after.
        if crate::switcher::switching() {
            crate::live::owned_by_switch(key);
        }
        self.settling.push((key, Instant::now() + SETTLE));
        self.arm_settle_timer();
    }

    /// Hand the windows that have settled to the worker that places them.
    pub(super) fn settled(&mut self) {
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) = self.settling.drain(..).partition(|(_, at)| *at <= now);
        self.settling = waiting;
        for (key, _) in due {
            crate::live::opened(key);
        }
        self.arm_settle_timer();
    }

    /// A move or minimize of a window a workspace placed: tell the tray which workspace.
    fn moved(&self, h: HWND) {
        if let Some((key, _)) = winutil::ws_stamp(h) {
            self.tell_moved(key);
        }
    }

    fn tell_moved(&self, key: u32) {
        if self.owner != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_WS_MOVED, WPARAM(key as usize), LPARAM(0));
            }
        }
    }

    /// Some window moved or changed size. Only a stamped one counts, which its
    /// property alone says, and its workspace is told once the burst is over.
    fn relocated(&mut self, h: HWND) {
        let Some((key, _)) = winutil::ws_stamp(h) else { return };
        if !self.relocated.contains(&key) {
            self.relocated.push(key);
        }
        // The same id starts the wait again.
        self.relocated_timer = unsafe { SetTimer(None, self.relocated_timer, RELOCATED_QUIET_MS, None) };
    }

    /// The stamped windows that moved have been still a moment.
    pub(super) fn relocated_due(&mut self) {
        self.stop_relocated_timer();
        for key in std::mem::take(&mut self.relocated) {
            self.tell_moved(key);
        }
    }

    fn stop_relocated_timer(&mut self) {
        if self.relocated_timer != 0 {
            let _ = unsafe { KillTimer(None, self.relocated_timer) };
            self.relocated_timer = 0;
        }
    }

    pub(super) fn stop_repaint(&mut self) {
        if self.repaint_timer != 0 {
            let _ = unsafe { KillTimer(None, self.repaint_timer) };
            self.repaint_timer = 0;
        }
        self.repaint.clear();
    }

    /// A tray that starts again keeps the pins it finds, and takes back borders
    /// a tray that stopped left behind.
    pub(super) fn adopt(&mut self) {
        let mut reset = 0;
        for h in winutil::top_level_windows() {
            let bits = winutil::pin_bits(h);
            if bits != 0 {
                self.pinned.push(h.0 as isize);
                self.known.insert(h.0 as isize, Pinned::of(h, bits));
            }
            if winutil::has_border_mark(h) {
                reset_border(h);
                reset += 1;
            }
        }
        if !self.pinned.is_empty() || reset > 0 {
            log::info!("wm start: {} pinned windows kept, {reset} borders reset", self.pinned.len());
        }
        self.sync_hooks();
    }

    /// Paint the right colour on each of these windows again, if they still exist.
    pub(super) fn paint_again(&self, keys: &[isize]) {
        for key in keys {
            let h = hwnd_of(*key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                continue;
            }
            if self.pin_owns(*key) {
                self.paint_pin(h);
            } else if self.bordered == Some(*key) {
                self.paint_focus();
            }
        }
    }

    /// Focus moved: paint the new window and the pinned one it left again soon,
    /// in case their app paints its own border colour over ours. A newer focus
    /// change starts over.
    fn focus_moved(&mut self, key: isize) {
        let left = self.foreground.replace(key).filter(|prev| self.pinned.contains(prev));
        self.repaint = std::iter::once(key).chain(left).collect();
        self.repaint_second = false;
        self.repaint_timer = unsafe { SetTimer(None, self.repaint_timer, REPAINT_FIRST_MS, None) };
    }

    pub(super) fn event(&mut self, event: u32, h: HWND) {
        let started = log::enabled().then(Instant::now);
        match event {
            EVENT_SYSTEM_FOREGROUND => {
                if self.painting() {
                    if self.focus_border {
                        self.border(h);
                    }
                    self.focus_moved(h.0 as isize);
                }
                if self.spotlight {
                    crate::spotlight::reorder();
                }
            }
            EVENT_OBJECT_DESTROY => {
                let key = h.0 as isize;
                self.seen_before.remove(&key);
                self.settling.retain(|(k, _)| *k != key);
                self.pinned.retain(|p| *p != key);
                if self.bordered == Some(key) {
                    self.bordered = None;
                }
                if let Some(gone) = self.known.remove(&key) {
                    self.pin_closed(gone);
                }
                if self.pinned.is_empty() {
                    self.sync_hooks();
                }
            }
            // Only top-level windows. A switch that has stopped listening lets the hook go.
            EVENT_OBJECT_SHOW if unsafe { GetAncestor(h, GA_ROOT) } == h => {
                let gone = self.shown_to.as_ref().is_some_and(|(_, tx)| tx.send(h.0 as isize).is_err());
                if gone {
                    self.shown_to = None;
                    self.sync_hooks();
                }
                if self.opening() {
                    self.opened(h);
                }
            }
            // Some apps never send a show for their window, only its title (§11.1).
            EVENT_OBJECT_NAMECHANGE if self.opening() && unsafe { GetAncestor(h, GA_ROOT) } == h => {
                self.opened(h);
            }
            // Dragging a window that hasn't been placed yet: the user is placing it.
            EVENT_SYSTEM_MOVESIZESTART => {
                let key = h.0 as isize;
                self.settling.retain(|(k, _)| *k != key);
            }
            EVENT_SYSTEM_MOVESIZEEND => self.moved(h),
            EVENT_SYSTEM_MINIMIZESTART | EVENT_SYSTEM_MINIMIZEEND => {
                if self.workspace_active {
                    self.moved(h);
                }
                if self.spotlight {
                    crate::spotlight::reorder();
                }
            }
            EVENT_OBJECT_LOCATIONCHANGE => self.relocated(h),
            _ => {}
        }
        if let Some(started) = started {
            let ns = started.elapsed().as_nanos() as u64;
            let s = &mut self.stats;
            (s.events, s.total_ns, s.max_ns) = (s.events + 1, s.total_ns + ns, s.max_ns.max(ns));
        }
    }

    pub(super) fn write_stats(&mut self) {
        let s = std::mem::take(&mut self.stats);
        if s.events > 0 {
            log::info!(
                "wm stats: {} events, avg {:.1}us, max {:.1}us",
                s.events,
                s.total_ns as f64 / s.events as f64 / 1e3,
                s.max_ns as f64 / 1e3
            );
        }
    }
}
