//! Windows coming to a tiled screen and leaving it: an open or a restore takes a tile,
//! a share or a stack, and a close or a minimize leaves a gap the others fill.

use super::*;

impl Engine {
    /// A window opened, was restored or asked for: it takes back the tile it left by
    /// minimizing, or a share of the largest tile, or a stack in it when it can't shrink
    /// to that. Waits for a drag to end; never during a workspace switch (D564).
    pub(super) fn tile_arrived(&mut self, a: Arrival) {
        if !self.active() {
            return;
        }
        // Kept for the end of the switch, as for a drag.
        if crate::switcher::switching() {
            self.tiling.paused.push(a);
            return;
        }
        if self.drag.is_some() {
            self.tiling.waiting.push(a);
            return;
        }
        // It may have closed, minimized or maximized since the worker looked.
        let h = hwnd_of(a.hwnd);
        if !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_minimized(h) || winutil::is_maximized(h) {
            return;
        }
        // It just showed, so its desktop is the one in front.
        if !a.desktop.is_empty() {
            self.tiled_desktop(Some(a.desktop.clone()));
        } else if !crate::tiler::has_desktops() {
            self.tiled_desktop(Some(String::new()));
        }
        let (x, y) = (a.frame.x + a.frame.w / 2, a.frame.y + a.frame.h / 2);
        let on = self.screens.iter().find(|s| s.bounds.contains(x, y)).map(|s| s.id.clone());
        // A parked window that comes back off every screen, restored or tiled again after a
        // crash, belongs to the screen it is nearest, as Windows has it.
        let known = a.order > 0 || self.tiling.trees.iter().any(|t| t.resting.iter().any(|(k, ..)| *k == a.hwnd));
        let Some(screen) = on.or_else(|| self.parked_home(a.frame).filter(|_| known)) else { return };
        if !self.tiling_on(&screen) {
            if known {
                self.rescue(a.hwnd, &screen, a.frame);
            }
            return;
        }
        if self.tree_of(a.hwnd).is_some() {
            // Restored where it still has its tile, as after a switch: nothing to do.
            if !a.restored {
                // The desk gate: a window is never placed twice.
                log::info!("tiling desk gate: placed twice refused, the window is tiled already");
            }
            return;
        }
        let desktop = self.tiling.desktop.clone();
        let i = self.tree_index(&screen, &desktop);
        let rested = self.tiling.trees.iter().enumerate().find_map(|(t, x)| {
            let (_, zone, ..) = x.resting.iter().find(|(k, ..)| *k == a.hwnd)?;
            Some((t, *zone))
        });
        if a.restored && rested.is_none() {
            return;
        }
        if self.tiling.trees[i].strip.as_ref().is_some_and(Strip::is_full) {
            log::info!("scrolling: the strip holds {} columns already, so a window floats", seam::strip::MAX_COLUMNS);
            // Nothing left to tile it again: no rest to come back to, no stamp for the next start.
            for t in &mut self.tiling.trees {
                t.resting.retain(|(k, ..)| *k != a.hwnd);
            }
            winutil::set_tile_stamp(h, 0);
            return;
        }
        for t in &mut self.tiling.trees {
            t.resting.retain(|(k, ..)| *k != a.hwnd);
            t.offscreen.remove(&a.hwnd);
        }
        let Some((work, _, gap)) = self.tile_area(&screen) else { return };
        self.tiled_check(i);
        let tree = &self.tiling.trees[i].tree;
        let back = rested.filter(|(t, _)| *t == i).and_then(|(_, zone)| tree.back_to(zone, a.hwnd, a.min));
        let placed = match (back, &self.tiling.trees[i].strip) {
            // A new column after the focused one, wide enough for it, and in view.
            (_, Some(strip)) => {
                let strip = strip.insert(a.hwnd, a.min, Width::fitting(a.min.0, work.w));
                let tree = strip.tree();
                self.tiling.trees[i].strip = Some(strip);
                Some((tree.clone(), Put::Tile(tree.zone_of(a.hwnd).unwrap_or(0))))
            }
            (Some(tree), None) => Some((tree.clone(), Put::Tile(tree.zone_of(a.hwnd).unwrap_or(0)))),
            (None, None) => tree.insert(work, gap, a.hwnd, a.min),
        };
        let Some((tree, put)) = placed else { return };
        let order = if a.order > 0 { a.order } else { self.tiling.seq + 1 };
        self.tiling.seq = self.tiling.seq.max(order);
        winutil::set_tile_stamp(h, order);
        winutil::set_floated(h, false);
        // Its place before tiling: a parked window's stamp keeps it, and a frame off every
        // screen (a window a crash left parked) is never one.
        let screens: Vec<Rect> = self.screens.iter().map(|s| s.bounds).collect();
        let before = winutil::park_stamp(h).or(Some(a.frame).filter(|f| !seam::strip::stranded(&screens, *f)));
        let t = &mut self.tiling.trees[i];
        if let Some(before) = before {
            t.before.entry(a.hwnd).or_insert(before);
        }
        if a.restored {
            t.back = Some(Instant::now());
        }
        let app = if log::enabled() { winutil::exe_name(h).unwrap_or_default() } else { String::new() };
        match put {
            Put::Tile(zone) => {
                log::info!("tiling: {app} takes zone {} of {}", zone + 1, tree.tiles.len());
                self.tiling.trees[i].tree = tree;
                self.tiled_apply(i);
            }
            Put::Stack(onto) => {
                log::info!("tiling: {app} can't shrink to a share, so it joins a stack");
                self.tiled_stack(i, a.hwnd, a.min, onto);
            }
        }
        // A window back from minimize all: the tiles of those still minimized close soon.
        self.tiled_arm_gaps();
    }

    /// A tiled window closed or was minimized: another window of its stack takes its
    /// tile, or its neighbours fill the gap and a stacked window that fits now leaves
    /// its stack. A minimized window's tile waits for it a moment first (`GAP_AFTER`).
    pub(super) fn tile_left(&mut self, key: isize, minimized: bool) {
        if !minimized {
            for t in &mut self.tiling.trees {
                t.resting.retain(|(k, ..)| *k != key);
                t.before.remove(&key);
            }
        }
        let Some(i) = self.tree_of(key) else { return };
        if !minimized {
            winutil::set_tile_stamp(hwnd_of(key), 0);
        }
        if self.tiling.trees[i].strip.is_some() {
            return self.scroll_left(i, key, minimized);
        }
        let t = &mut self.tiling.trees[i];
        if let Some(at) = t.overflow.iter().position(|(k, _)| *k == key) {
            t.overflow.remove(at);
            self.tiled_sync();
            return;
        }
        if self.here(i) {
            self.snapped.remove(&key);
        }
        if self.tiled_heir(i, key) {
            self.tiled_sync();
            return;
        }
        let t = &mut self.tiling.trees[i];
        if minimized && let (Some(zone), Some(tree)) = (t.tree.zone_of(key), t.tree.vacate(key)) {
            t.tree = tree;
            t.resting.push((key, zone, Instant::now(), true));
            self.tiled_arm_gaps();
            self.tiled_sync();
            return;
        }
        self.tiled_check(i);
        if !self.tiling.trees[i].remove(key) {
            return;
        }
        self.tiled_unstack(i);
        self.tiled_apply(i);
    }

    /// `key` leaves its tile, and a window stacked in it takes the tile instead: its
    /// stack's next window, or one tiling stacked there. Whether one did.
    pub(super) fn tiled_heir(&mut self, i: usize, key: isize) -> bool {
        let Some(zone) = self.tiling.trees[i].tree.zone_of(key) else { return false };
        let Some((heir, min)) = self.heir_of(i, key) else { return false };
        let t = &mut self.tiling.trees[i];
        t.overflow.retain(|(k, _)| *k != heir);
        t.tree.tiles[zone] = tiling::Tile { hwnd: Some(heir), min };
        true
    }

    /// The window stacked in `key`'s tile that would take it, and its smallest size.
    pub(super) fn heir_of(&self, i: usize, key: isize) -> Option<(isize, tiling::Min)> {
        let zone = self.tiling.trees[i].tree.zone_of(key)?;
        let live = |m: &isize| {
            *m != key && unsafe { IsWindow(Some(hwnd_of(*m))) }.as_bool() && !winutil::is_minimized(hwnd_of(*m))
        };
        let t = &self.tiling.trees[i];
        let same_zone = |k: &isize| self.snapped.get(k).is_some_and(|s| s.screen == t.screen && s.from == zone);
        let stacked: Vec<isize> = t.overflow.iter().map(|(k, _)| *k).filter(same_zone).collect();
        let heir = self.stacks.of(key).and_then(|s| s.members.iter().copied().find(live));
        let heir = heir.or_else(|| stacked.into_iter().find(live))?;
        Some((heir, t.overflow.iter().find(|(k, _)| *k == heir).map_or((0, 0), |(_, m)| *m)))
    }

    /// A window that can't shrink to a share joins `onto`'s stack, in its zone.
    pub(super) fn tiled_stack(&mut self, i: usize, key: isize, min: tiling::Min, onto: isize) {
        let t = &self.tiling.trees[i];
        let Some((work, dpi, gap)) = self.tile_area(&t.screen) else { return };
        let Some(zone) = t.tree.zone_of(onto) else { return };
        let (screen, target) = (t.screen.clone(), t.tree.rects(work, gap)[zone]);
        let h = hwnd_of(key);
        // In the zone before it lands, so its move doesn't take it out of the stack it joins.
        let old = winutil::window_rect(h).map(|r| (r.w, r.h, winutil::monitor_dpi(h)));
        self.snapped.insert(key, Snapped { old, screen: screen.clone(), from: zone, to: zone, at: target });
        let (out, id) = self.stacks.join(onto, key);
        self.stack_stamps(&out, Some(id));
        self.tiling.trees[i].overflow.push((key, min));
        self.stacks_changed();
        self.tiled_sync();
        self.queued.push_back(Placement { hwnd: key, target, zone: Some((screen, zone, zone)), dpi, maximize: false });
        self.window_pump();
    }
}
