//! A drag from its first move to the drop, and the worker that sizes a window dropped on another screen.

use super::*;

impl Engine {
    pub(super) fn drag_start(&mut self, hwnd: HWND) {
        self.cancel_drag();
        // Group members no longer where their group put them have left it. The moves
        // that closing up makes land below like any other waiting move.
        self.prune_adhoc(true);
        let key = hwnd.0 as isize;
        // A placement still waiting would yank the window back under the pointer.
        if self.pending.as_ref().is_some_and(|p| p.hwnd == key)
            && let Some(p) = self.pending.take()
        {
            unsafe {
                let _ = KillTimer(None, p.timer);
            }
        }
        // Moves still waiting land now, where they were going, and are recorded there:
        // dropping them left a swap half done, its other window still in its old zone
        // under the dropped one, and the snapping's records no longer what is on screen.
        // The dragged window's own is dropped, as the drag takes it. Each is only
        // posted, so a slow app can't hold up the drag.
        let landed: Vec<(isize, Rect)> = std::mem::take(&mut self.queued)
            .into_iter()
            .filter(|p| p.hwnd != key)
            .filter_map(|p| self.land_now(p))
            .collect();
        let wanted = self.active() || self.drags_sized();
        if !wanted || !winutil::is_snappable(hwnd, std::process::id(), &self.exclude) {
            return;
        }
        // Windows keeps a lower process from moving this one, so don't offer zones for it (D12).
        if winutil::is_elevated_over_us(hwnd) {
            if self.active() {
                self.blocked(hwnd);
            }
            return;
        }
        let geometry = if self.active() { self.geometry() } else { Vec::new() };
        let Some(start) = winutil::window_rect(hwnd) else { return };
        if geometry.is_empty() && !self.drags_sized() && !self.beside_on() {
            return;
        }
        if !geometry.is_empty() {
            // The work area may have moved since the overlay was last drawn. This only posts a list.
            zone_paint::build(specs(&geometry));
        }
        // A swap, or Undo of a split by the drag's keys, puts windows where this one was.
        let frame = (self.drags_sized() || !geometry.is_empty()).then(|| winutil::visible_rect(hwnd)).flatten();
        let current = self.current(hwnd).map(|s| (s.old, (s.screen.clone(), s.from, s.to)));
        let was_snapped = current.as_ref().map(|(old, _)| *old);
        let stamped = winutil::has_zone_stamp(hwnd);
        let button = Some(main_button()).filter(|b| held(*b));
        let timer = unsafe { SetTimer(None, 0, TICK_MS, Some(on_tick)) };
        let was_maximized = winutil::is_maximized(hwnd);
        let zone = current.map(|(_, zone)| zone);
        let old_zone =
            if geometry.is_empty() { zone.clone() } else { self.zones_held(hwnd, &geometry, frame, was_maximized) };
        let edges = self.edges_start(hwnd, &geometry, (zone, stamped), was_maximized);
        let edges = edges.or_else(|| self.adhoc_edges_start(hwnd, was_maximized));
        // The handles stay out of the way of any drag, and come back at its end.
        dividers::hide();
        self.drag = Some(Drag {
            hwnd: key,
            phase: Phase::Waiting,
            start,
            frame,
            was_maximized,
            stamped,
            old_zone,
            was_snapped,
            button,
            geometry,
            anchor: None,
            shown: None,
            timer,
            edges,
            keys: dragkeys::Steer::default(),
            rest: None,
            paused: false,
            others: Vec::new(),
            looked: false,
            landed,
            beside: None,
            tried: None,
            allowed: Vec::new(),
            alt: false,
            left: None,
        });
    }

    /// A snapped window dropped outside any zone gets its old size back, scaled
    /// to the screen it's on, with the pointer at the same place along its width.
    fn restore_size(&mut self, hwnd: HWND, (w, h, dpi): Old) {
        let Some(now) = winutil::window_rect(hwnd) else { return };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() || now.w <= 0 {
            return;
        }
        let k = winutil::monitor_dpi(hwnd).max(1) as f32 / dpi.max(1) as f32;
        let (w, h) = ((w as f32 * k).round() as i32, (h as f32 * k).round() as i32);
        let along = (pt.x - now.x) as f32 / now.w as f32;
        let x = pt.x - (along * w as f32).round() as i32;
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, now.y, w, h, flags);
        }
    }

    pub(super) fn tick(&mut self) {
        let Some(mut drag) = self.drag.take() else { return };
        let mut pt = POINT::default();
        if !unsafe { IsWindow(Some(hwnd_of(drag.hwnd))) }.as_bool() {
            self.finish(drag, false);
            return;
        }
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            self.drag = Some(drag);
            return;
        }
        // The end of the drag never arrived, so don't let a drop snap by surprise.
        if drag.button.is_some_and(|b| !held(b)) {
            self.finish(drag, false);
            return;
        }
        if drag.phase == Phase::Waiting {
            let hwnd = hwnd_of(drag.hwnd);
            let Some(now) = winutil::window_rect(hwnd) else {
                self.drag = Some(drag);
                return;
            };
            if now != drag.start {
                let resized = !drag.was_maximized && (now.w != drag.start.w || now.h != drag.start.h);
                drag.phase = if resized { Phase::Resizing } else { Phase::Moving };
                // A member dragged away leaves its group now, not once it has stayed put.
                // A resize can be the group's own, through its edges, so that waits for the drop.
                if !resized {
                    drag.left = self.leave_adhoc_dragged(drag.hwnd);
                }
                if resized && drag.edges.is_none() {
                    // Zones have nothing to do with a resize, unless the neighbours follow it.
                    unsafe {
                        let _ = KillTimer(None, drag.timer);
                    }
                }
            }
        }
        if drag.phase == Phase::Resizing {
            let (hwnd, start) = (hwnd_of(drag.hwnd), drag.start);
            if let Some(edges) = drag.edges.as_mut() {
                edges.tick(hwnd, start);
            }
        }
        if drag.phase != Phase::Moving {
            self.drag = Some(drag);
            return;
        }
        let show_key = key_code(&self.settings.show_key).unwrap_or(VK_SHIFT);
        let span_key = key_code(&self.settings.span_key).filter(|k| *k != show_key);
        let pressed = held(show_key);
        self.pause(&mut drag, pt, pressed);
        let show = (pressed || drag.paused) && !drag.keys.off;
        let span = show && span_key.is_some_and(held);
        // A window's edge under the pointer comes first, then the zones as usual.
        let beside = (show && self.beside_on()).then(|| self.beside_at(&mut drag, pt)).flatten();
        // A share picked beside a window is that edge's, and doesn't carry into the zones.
        if beside.is_none() && drag.beside.is_some() {
            drag.keys.part = None;
        }
        let zoned = show && beside.is_none();
        let view = if zoned { Self::zone_view(&mut drag, pt, span) } else { None };
        let (view, beside) = match beside {
            Some((b, view)) => (view, Some(b)),
            None => (view, None),
        };
        // Repaint only when what is lit changes.
        if view != drag.shown || beside != drag.beside {
            self.present(&mut drag, view, beside);
        }
        self.drag = Some(drag);
    }

    /// The zones lit under the pointer, with the keys' choice of zone and share applied.
    fn zone_view(drag: &mut Drag, pt: POINT, span: bool) -> Option<View> {
        drag.geometry.iter().position(|g| g.bounds.contains(pt.x, pt.y)).map(|screen| {
            let cell = zones::zone_at(&drag.geometry[screen].cells, pt.x, pt.y);
            let hit = cell.map(|zone| {
                // Tab's choice holds while the pointer stays in the zone it was made in.
                if drag.keys.cycle_at != Some((screen, zone)) {
                    drag.keys.cycle = 0;
                }
                // An arrow's side holds in its zone, or the first zone the pointer enters
                // after it; elsewhere the side follows the pointer.
                if drag.keys.side.is_some() && drag.keys.side_at.is_none() {
                    drag.keys.side_at = Some((screen, zone));
                }
                if drag.keys.side_at != Some((screen, zone)) {
                    drag.keys.side = None;
                }
                if drag.keys.cycle > 0 {
                    let list = dragkeys::candidates(&drag.geometry[screen].cells, zone, (pt.x, pt.y));
                    return list[drag.keys.cycle.min(list.len() - 1)];
                }
                if !span {
                    drag.anchor = None;
                    return (zone, zone);
                }
                match drag.anchor {
                    Some((s, from)) if s == screen => (from, zone),
                    _ => {
                        drag.anchor = Some((screen, zone));
                        (zone, zone)
                    }
                }
            });
            let g = &drag.geometry[screen];
            let cut = hit.zip(drag.keys.part).map(|((a, b), part)| {
                let r = zones::span(g.zones[a.min(b)], g.zones[a.max(b)]);
                (part, drag.keys.side.unwrap_or_else(|| dragkeys::nearest_side(r, (pt.x, pt.y))))
            });
            View { screen, hit, cut }
        })
    }

    fn hide_all(&self) {
        zone_paint::hide_all();
    }

    pub(super) fn present(&mut self, drag: &mut Drag, view: Option<View>, beside: Option<adhoc::Beside>) {
        let started = log::enabled().then(Instant::now);
        // The screen each shows on: a view's zones, or the screen of the window beside.
        let on = |v: Option<View>, b: Option<adhoc::Beside>| {
            v.map(|v| drag.geometry[v.screen].id.clone()).or_else(|| b.map(|b| self.screens[b.screen].id.clone()))
        };
        let (was, now) = (on(drag.shown, drag.beside), on(view, beside));
        if let Some(was) = was.filter(|w| now.as_ref() != Some(w)) {
            zone_paint::hide(&was);
        }
        let live = view.is_some() || beside.is_some();
        // The keys are the drag's from the first time zones show, until it ends or Esc.
        // One look at the windows a drop could swap with, the first time zones show.
        if live && !drag.looked {
            drag.others = self.snapshot(drag.hwnd, &drag.landed);
            drag.looked = true;
        }
        if live && !drag.keys.hooked && !drag.keys.off && !drag.keys.capped {
            let show_key = key_code(&self.settings.show_key).unwrap_or(VK_SHIFT);
            let span_key = key_code(&self.settings.span_key);
            let own = |vk: VIRTUAL_KEY| show_key == vk || span_key == Some(vk);
            drag.keys.generation = dragkeys::start(self.owner, own(VK_CONTROL), own(VK_MENU));
            drag.keys.hooked = true;
        }
        if drag.keys.hooked && !drag.keys.off {
            dragkeys::set_live(live);
        }
        let hint = (drag.keys.hooked && self.settings.drag_hints).then(|| dragkeys::HINT.to_string());
        if let Some(view) = view {
            let g = &drag.geometry[view.screen];
            // The whole span stays lit, and a share of it from the keys is lit again inside it.
            let (lit, part, others) = match view.hit {
                Some((a, b)) => {
                    let out = self.outcome(drag, view.screen, (a, b), view.cut);
                    let part = out.split.is_some().then_some(out.rect);
                    (Some(zones::span(g.zones[a], g.zones[b])), part, out.others())
                }
                None => (None, None, Vec::new()),
            };
            zone_paint::preview(&g.id, lit, part, others, hint);
        } else if let Some((id, plan)) = now.zip(beside.and_then(|b| self.adhoc_plan(drag.hwnd, &b))) {
            // Beside a window on a screen with no zones: the dropped window's part, and the other's.
            zone_paint::preview(&id, None, Some(plan.rect), vec![plan.target], hint);
        }
        drag.shown = view;
        drag.beside = beside;
        if let Some(started) = started {
            log::info!("zone overlay: main {:.0}us", started.elapsed().as_secs_f64() * 1e6);
        }
    }

    pub(super) fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            self.finish(drag, false);
        }
    }

    pub(super) fn drag_end(&mut self, hwnd: HWND) {
        match self.drag.take() {
            Some(drag) if drag.hwnd == hwnd.0 as isize => self.finish(drag, true),
            other => self.drag = other,
        }
    }

    /// Stop the timer and hide the overlay. If it was showing a zone and the
    /// drag ended normally, put the window there.
    fn finish(&mut self, drag: Drag, dropped: bool) {
        unsafe {
            let _ = KillTimer(None, drag.timer);
        }
        self.hide_all();
        if drag.keys.hooked {
            dragkeys::release();
        }
        if !drag.keys.used.is_empty() {
            log::info!("drag keys: {}", drag.keys.used.join(", "));
        }
        let hwnd = hwnd_of(drag.hwnd);
        let mut drag = drag;
        if dropped {
            self.refresh(&mut drag);
        }
        self.finish_drag(&mut drag, dropped, hwnd);
        self.sync_dividers();
        // A re-fit that had to wait for this drag can go now.
        self.refit_resume();
    }

    /// Ctrl is the show key, so it can be held for a drag that never meant a pull-out.
    fn ctrl_is_a_key(&self) -> bool {
        key_code(&self.settings.show_key) == Some(VK_CONTROL)
    }

    fn finish_drag(&mut self, drag: &mut Drag, dropped: bool, hwnd: HWND) {
        // Cancelled with Esc, Windows puts the window back, and so does its group.
        if let Some(left) = drag.left.take()
            && winutil::window_rect(hwnd).is_some_and(|r| adhoc::near(r, drag.start))
            && self.rejoin_adhoc(drag.hwnd, left)
        {
            return;
        }
        // Dragged away, a window leaves its ad-hoc group.
        if drag.phase == Phase::Moving {
            self.leave_adhoc(drag.hwnd);
        }
        // A window in a group of zones stays snapped, and its neighbours land.
        if drag.phase == Phase::Resizing {
            let ended = dropped || drag.button.is_some_and(|b| !held(b));
            if self.edges_end(drag, hwnd, ended) {
                return;
            }
        }
        // Resized by hand: it's no longer in its zone, or its group.
        if dropped && drag.phase == Phase::Resizing {
            self.unsnap(hwnd, drag.stamped);
            self.leave_adhoc(drag.hwnd);
            return;
        }
        // Maximized by the drag itself is Windows' top-edge snap, or a browser tab
        // merging back into a window. Either way it isn't ours to place.
        if !dropped || drag.phase != Phase::Moving {
            return;
        }
        if winutil::is_maximized(hwnd) && !drag.was_maximized {
            self.unsnap(hwnd, drag.stamped);
            return;
        }
        if let (None, Some(b)) = (drag.shown, drag.beside) {
            self.beside_drop(drag, hwnd, b);
            return;
        }
        let lit = drag.shown.and_then(|view| Some((view.screen, view.hit?, view.cut)));
        let Some((screen, (a, b), cut)) = lit else {
            // With Ctrl held, a window alone in its zone leaves the layout, and the others close the gap.
            if held(VK_CONTROL) && !(drag.alt && self.alt_holds_ctrl) && !self.ctrl_is_a_key() {
                self.pull_out(drag);
            }
            // Dropped outside every zone, so a snapped window gets its old size back.
            let mut restored = false;
            if let (Some(Some(old)), true) = (drag.was_snapped, self.settings.restore_size)
                && !winutil::is_maximized(hwnd)
                && winutil::is_resizable(hwnd)
            {
                self.restore_size(hwnd, old);
                restored = true;
            }
            self.unsnap(hwnd, drag.stamped);
            if !restored && !drag.was_maximized {
                self.keep_size(drag, hwnd);
            }
            return;
        };
        let out = self.outcome(drag, screen, (a, b), cut);
        if let Some(split) = out.split {
            if let Some(b) = drag.beside {
                log::info!("snap beside: zone split on screen {}, {} {:?}", b.screen + 1, b.part.name(), b.side);
            }
            self.split_drop(drag, hwnd, screen, split);
            return;
        }
        if log::enabled() {
            self.audit(drag, screen, (a.min(b), a.max(b)));
            if let (None, Err(why)) = (&out.swap, self.swap_why(drag, screen, (a.min(b), a.max(b))))
                && why != swap::Skip::Off
            {
                log::info!("zone swap: skipped, {why:?}");
            }
        }
        let g = &drag.geometry[screen];
        self.snap_to(hwnd, g, (a.min(b), a.max(b)), out.rect, drag.was_snapped);
        self.swap_start(out.swap);
    }

    /// A free drag that ended on another screen: with "Also when I drag a window to
    /// another screen" on, a worker gives the window the size the chosen mode keeps
    /// (§11.4). Only the screens are looked up here.
    fn keep_size(&self, drag: &Drag, hwnd: HWND) {
        let Some(frame) = drag.frame.filter(|_| self.drags_sized()) else { return };
        if winutil::is_maximized(hwnd) || !winutil::is_resizable(hwnd) {
            return;
        }
        let Some(now) = winutil::window_rect(hwnd) else { return };
        let at = |r: Rect| self.screens.iter().position(|s| s.bounds.contains(r.x + r.w / 2, r.y + r.h / 2));
        let (Some(from), Some(to)) = (at(frame), at(now)) else { return };
        if from == to {
            return;
        }
        let (a, b) = (&self.screens[from], &self.screens[to]);
        let (from, to) = ((from + 1, a.raw_dpi), (to + 1, b.raw_dpi, b.bounds));
        let job = DragJob { hwnd: drag.hwnd, mode: self.drag_size, frame, from, to };
        if std::thread::Builder::new().name("seam-window-size".into()).spawn(move || job.run()).is_err() {
            log::info!("window size: couldn't start the worker");
        }
    }
}

/// A free drag between screens, for its worker.
struct DragJob {
    hwnd: isize,
    /// Pixels or real size, never Windows'.
    mode: WindowSize,
    /// The visible frame before the drag.
    frame: Rect,
    /// Line Up number and raw density of the screen it left.
    from: (usize, f64),
    /// The same of the screen it was dropped on, and its bounds.
    to: (usize, f64, Rect),
}

impl DragJob {
    /// Once the app has answered the DPI change of the drop (D8), undo Windows' rescale:
    /// fit the frame to the size the mode keeps, centred where it was dropped. Only the
    /// frame is resized.
    fn run(self) {
        std::thread::sleep(std::time::Duration::from_millis(SETTLE_MS.into()));
        let h = hwnd_of(self.hwnd);
        let ((from, src), (to, dst, bounds)) = (self.from, self.to);
        let (Some(now), Some((work, _))) = (winutil::visible_rect(h), work_area(bounds)) else { return };
        let app = winutil::exe_name(h).unwrap_or_default();
        let (w, ht, mode) = (self.frame.w, self.frame.h, self.mode);
        let Some(r) = desk::moved_rect(mode, (w as f64, ht as f64), (src, dst), now, work) else {
            log::info!("window size: {app} scaled by Windows, screen {from} or {to} doesn't report its size");
            return;
        };
        log::info!("window size ({mode:?}): {app} screen {from} → {to}, {w}×{ht} px → {}×{} px", r.w, r.h);
        if !winutil::is_hung(h) {
            winutil::fit(h, r);
        }
    }
}
