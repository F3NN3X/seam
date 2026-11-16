//! What I do by hand on a tiled screen, and the desktop in front: drags re-slot or
//! float windows, the Float key, Free all, and switching virtual desktops.

use super::*;

/// A screen and the virtual desktop whose tree was freed.
pub type Freed = (String, String);

impl Engine {
    /// The worker said which desktop is in front, or `None` when it can't tell, as on a
    /// desktop with no windows. The snapping keeps only the records of the windows on
    /// the one in front, and its trees close the gaps left while it was away.
    pub(in crate::snap) fn tiled_desktop(&mut self, now: Option<String>) {
        let Some(now) = now else {
            // Nothing is tiled, and nothing taken for gone, until it is known again.
            if self.tiling.known {
                self.tiling.known = false;
                self.tiled_sync();
            }
            return;
        };
        let (first, was_known) = (!self.tiling.seen, self.tiling.known);
        (self.tiling.seen, self.tiling.known) = (true, true);
        if now == self.tiling.desktop && !first {
            if !was_known {
                self.tiled_sync();
            }
            return;
        }
        let before = std::mem::replace(&mut self.tiling.desktop, now.clone());
        for i in 0..self.tiling.trees.len() {
            let t = &self.tiling.trees[i];
            if t.desktop == before && !first {
                let windows = t.all();
                let parked: Vec<(isize, Snapped)> =
                    windows.into_iter().filter_map(|k| Some((k, self.snapped.remove(&k)?))).collect();
                self.tiling.trees[i].parked = parked;
            } else if t.desktop == now {
                for (k, s) in std::mem::take(&mut self.tiling.trees[i].parked) {
                    self.snapped.insert(k, s);
                }
            }
        }
        if self.drag.is_some() {
            self.cancel_drag();
        }
        // A desktop seen for the first time this run may hold windows a tray that ran before tiled.
        let seen = self.tiling.trees.iter().any(|t| t.desktop == now);
        if !first && !seen && self.tiling.config.values().any(|c| c.on(&now)) {
            crate::tiler::retile();
        }
        let screens: Vec<String> = self.screens.iter().map(|s| s.id.clone()).filter(|s| self.tiling_on(s)).collect();
        for screen in &screens {
            self.tiled_adopt_none(screen);
        }
        self.tiled_sync();
        for i in 0..self.tiling.trees.len() {
            if self.tiling.trees[i].desktop == now && self.tiling.trees[i].dirty {
                self.tiled_check(i);
                self.tiled_apply(i);
            }
        }
        // A parked window gone to on this desktop, by Alt+Tab or the taskbar, scrolls into view now.
        self.scroll_focus_waited();
    }

    /// After a drag, and once moves are over: who holds each zone of a tiled screen is
    /// what the snapping recorded, so a swap, a split, a stack by S or a drop from
    /// elsewhere re-slots windows by the drag's rules. A tiled window dragged out of the
    /// screen's zones (`dragged`) floats, and its neighbours fill the gap, as the Float
    /// key does (D562). Windows that closed or went to another desktop leave first.
    pub(in crate::snap) fn tiled_resync(&mut self, dragged: Option<isize>) {
        if self.tiling.trees.is_empty() && !self.tiling.config.values().any(|c| c.on(&self.tiling.desktop)) {
            return;
        }
        if self.drag.is_some() || self.pending.is_some() || !self.queued.is_empty() || !self.tiling.known {
            return;
        }
        let screens: Vec<(String, Node)> = self
            .screens
            .iter()
            .filter(|s| self.tiling_on(&s.id))
            .filter_map(|s| Some((s.id.clone(), s.layout.as_ref()?.tree.clone())))
            .collect();
        let mut changed = false;
        for (screen, node) in screens {
            let desktop = self.tiling.desktop.clone();
            let i = self.tree_index(&screen, &desktop);
            // A window gone unseen leaves, and the others close up: this runs again once they have moved.
            if self.tiling.trees[i].tree.windows().any(gone) {
                self.tiled_check(i);
                self.tiled_apply(i);
                continue;
            }
            let n = zones::leaf_count(&node);
            let old = self.tiling.trees[i].tree.clone();
            let mut held: Vec<(usize, isize, bool)> = Vec::new();
            for (k, s) in &self.snapped {
                let h = hwnd_of(*k);
                let live = unsafe { IsWindow(Some(h)) }.as_bool() && !winutil::is_cloaked(h);
                if s.screen == screen && s.from == s.to && s.from < n && live {
                    held.push((s.from, *k, self.stacks.of(*k).is_none_or(|st| st.top() == *k)));
                }
            }
            // A tiled window minimized keeps its tile until the worker says so.
            let resting = |h: isize| winutil::is_minimized(hwnd_of(h));
            let kept: Vec<Option<isize>> =
                old.tiles.iter().map(|t| t.hwnd.filter(|h| n == old.tiles.len() && resting(*h))).collect();
            let olds: Vec<Option<isize>> = old.tiles.iter().map(|t| t.hwnd).collect();
            let holders = tiling::holders(n, &olds, &kept, &held);
            let min_of = |k: isize| old.tiles.iter().find(|t| t.hwnd == Some(k)).map_or((0, 0), |t| t.min);
            let tiles: Vec<tiling::Tile> =
                holders.iter().map(|h| tiling::Tile { hwnd: *h, min: h.map_or((0, 0), min_of) }).collect();
            let stacked: HashSet<isize> =
                holders.iter().flatten().filter_map(|h| self.stacks.of(*h)).flat_map(|s| s.members.clone()).collect();
            let family = old.family;
            let mut tree = if tiles.iter().all(|t| t.hwnd.is_none()) && old.windows().next().is_some() {
                Tree::new(family)
            } else {
                Tree { family, node: Some(node), tiles }
            };
            let mut close = false;
            let out: Vec<isize> =
                old.windows().filter(|k| tree.zone_of(*k).is_none() && !stacked.contains(k)).collect();
            for k in out {
                winutil::set_tile_stamp(hwnd_of(k), 0);
                self.tiling.trees[i].before.remove(&k);
                if Some(k) != dragged {
                    continue;
                }
                log::info!("tiling: a tiled window was dragged out of its tile, so it floats and the others close up");
                winutil::set_floated(hwnd_of(k), true);
                // Its tile held only as the drop target while the drag ran.
                let zone = old.zone_of(k).filter(|z| n == old.tiles.len() && tree.tiles[*z].hwnd.is_none());
                if let Some(less) = zone.and_then(|z| tree.remove_zone(z)) {
                    tree = less;
                    self.tiling.trees[i].closed_zone(zone.unwrap_or(usize::MAX));
                    close = true;
                }
            }
            if tree == old {
                continue;
            }
            changed = true;
            for k in tree.windows().filter(|k| old.zone_of(*k).is_none()) {
                let h = hwnd_of(k);
                winutil::set_floated(h, false);
                self.tiling.seq += 1;
                winutil::set_tile_stamp(h, self.tiling.seq);
                if let Some(r) = winutil::visible_rect(h) {
                    self.tiling.trees[i].before.entry(k).or_insert(r);
                }
            }
            let t = &mut self.tiling.trees[i];
            t.overflow.retain(|(k, _)| stacked.contains(k));
            t.tree = tree;
            // Scrolling: the view's columns follow the drag, and the strip settles around them.
            if close || t.strip.is_some() {
                self.tiled_unstack(i);
                self.tiled_apply(i);
            }
        }
        if changed {
            self.tiled_sync();
        }
    }

    pub(in crate::snap) fn tiled_dragged(&mut self, screen: &str, tree: &Node) -> bool {
        if !self.tiling_on(screen) {
            return false;
        }
        if let Some(s) = self.screens.iter_mut().find(|s| s.id == screen) {
            s.layout = Some(ZoneLayout { template: Template::Custom, custom: None, from: None, tree: tree.clone() });
        }
        let desktop = self.tiling.desktop.clone();
        let i = self.tree_index(screen, &desktop);
        if self.tiling.trees[i].strip.is_some() {
            // A column takes the width nearest the one the handle gave it.
            self.scroll_dragged(i, tree);
            return true;
        }
        if zones::leaf_count(tree) == self.tiling.trees[i].tree.tiles.len() {
            self.tiling.trees[i].tree.node = Some(tree.clone());
        }
        self.tiled_resync(None);
        self.tiled_sync();
        true
    }

    /// The drag ended: the tiled screens follow it, then the arrivals that waited for it.
    pub(in crate::snap) fn tiled_after_drag(&mut self, dragged: isize) {
        self.tiled_resync(Some(dragged));
        for a in std::mem::take(&mut self.tiling.waiting) {
            self.tile_arrived(a);
        }
        self.scroll_focus_waited();
    }

    pub(in crate::snap) fn tiling_free(&mut self, screen: &str) -> usize {
        let desktop = self.tiling.desktop.clone();
        self.free_screen(screen, &desktop, None).0
    }

    /// Seam quits and was asked to put windows back: every tiled screen on every desktop is
    /// freed, and its moves are posted at once, since no timer will run them. Returns where
    /// the windows are going, and the (screen, desktop) pairs freed.
    pub(in crate::snap) fn tiling_free_for_quit(&mut self, until: Instant) -> (Vec<(isize, Rect)>, Vec<Freed>) {
        let freed: Vec<Freed> = self.tiling.trees.iter().map(|t| (t.screen.clone(), t.desktop.clone())).collect();
        let landing = freed.iter().flat_map(|(s, d)| self.free_screen(s, d, Some(until)).1).collect();
        (landing, freed)
    }

    /// Free one screen. When Seam quits (`until`) the moves are posted here rather than queued.
    fn free_screen(&mut self, screen: &str, desktop: &str, until: Option<Instant>) -> (usize, Vec<(isize, Rect)>) {
        let now = until.is_some();
        let front = desktop == self.tiling.desktop;
        let Some(i) = self.tiling.trees.iter().position(|t| t.screen == screen && t.desktop == desktop) else {
            return (0, Vec::new());
        };
        let t = self.tiling.trees.remove(i);
        let dpi = self.tile_area(screen).map_or(96, |(_, dpi, _)| dpi);
        let windows = t.all();
        let parked = |k: &isize| t.offscreen.contains_key(k);
        // Parked windows that aren't columns now (minimized ones) come back here, given their
        // place to restore to; the columns come back with the others below.
        let columns: Vec<isize> =
            windows.iter().copied().filter(|k| parked(k) && !winutil::is_minimized(hwnd_of(*k))).collect();
        let shown = self.screens_now();
        self.unpark(&t, until, &columns, &shown);
        let screens: Vec<Rect> = self.screens.iter().map(|s| s.bounds).collect();
        let mut moves = Vec::new();
        for k in &windows {
            let h = hwnd_of(*k);
            winutil::set_tile_stamp(h, 0);
            if self.stacks.of(*k).is_some() {
                self.stack_leave(*k);
            }
            // Gone to another desktop, it isn't moved from here. On a desktop not in front every
            // window is cloaked, so only a closed window counts as gone.
            let away = if front { gone(*k) } else { !unsafe { IsWindow(Some(h)) }.as_bool() }
                || winutil::is_minimized(h)
                || winutil::is_maximized(h);
            // A parked window always comes back, to where it was before or onto its screen. A
            // place before off every screen (a window a crash left parked) is never gone back to.
            let back = if columns.contains(k) {
                winutil::set_park_stamp(h, None);
                self.home_of(&t, *k)
            } else if parked(k) {
                None
            } else {
                t.before.get(k).copied().filter(|r| !seam::strip::stranded(&screens, *r))
            };
            if let Some(r) = back.filter(|_| !away || columns.contains(k)) {
                moves.push(Placement { hwnd: *k, target: r, zone: None, dpi, maximize: false });
            } else if self.snapped.contains_key(k) {
                self.unsnap(h, true);
            }
        }
        log::info!("tiling: freed {} windows", windows.len());
        self.tiled_sync();
        if now {
            return (windows.len(), moves.into_iter().filter_map(|p| self.land_now(p)).collect());
        }
        self.queued.extend(moves);
        self.window_pump();
        (windows.len(), Vec::new())
    }

    pub(in crate::snap) fn tiling_float(&mut self, key: isize) -> Result<bool, String> {
        if !self.active() {
            return Err("Zones are off, or Seam is standing aside".into());
        }
        let h = hwnd_of(key);
        if let Some(i) = self.tree_of(key) {
            winutil::set_tile_stamp(h, 0);
            winutil::set_floated(h, true);
            let t = &mut self.tiling.trees[i];
            t.before.remove(&key);
            if let Some(strip) = &t.strip {
                let strip = strip.remove(key);
                t.tree = strip.tree();
                t.strip = Some(strip);
                t.offscreen.remove(&key);
            } else if t.overflow.iter().any(|(k, _)| *k == key) {
                t.overflow.retain(|(k, _)| *k != key);
            } else if t.remove(key) {
                self.tiled_unstack(i);
            }
            if self.snapped.contains_key(&key) {
                self.unsnap(h, true);
            }
            self.tiled_check(i);
            self.tiled_apply(i);
            log::info!("tiling: a window floats by the key");
            return Ok(true);
        }
        let at = winutil::visible_rect(h).ok_or("No window has focus")?;
        let (x, y) = (at.x + at.w / 2, at.y + at.h / 2);
        let screen = self.screens.iter().find(|s| s.bounds.contains(x, y)).map(|s| s.id.clone());
        if !screen.is_some_and(|s| self.tiling_on(&s)) {
            return Err("Smart tiling is off on this window's screen".into());
        }
        // Its float mark stays until tiling takes it, so a refusal leaves it floating.
        crate::tiler::asked(key);
        Ok(false)
    }
}
