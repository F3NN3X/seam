//! The Scrolling family on the snapping's engine. A scrolling screen keeps its windows
//! as columns on a strip (`seam::strip`); its tree is the view, which shows as the
//! screen's zones, and the windows of every other column are parked outside every
//! monitor, at their screen's DPI, so none ever shows on a neighbouring screen. They
//! still reach Alt+Tab, the taskbar and the overview, and a parked window that takes
//! the focus is scrolled into view.
//!
//! Every move goes through `tiled_apply`, so the one rule (`tiling::plan_apply`) runs
//! first over every column, parked ones too: a window someone else placed since, its app
//! included, floats where it is. No window is left parked when tiling ends on its
//! screen, when Seam pauses, steps aside or has zones off, or when Seam quits.

use super::*;
use seam::strip::{self, Place};

/// Scroll a Scrolling screen one column. With `walk`, as the keys and `tiling.scroll`
/// do, the column next to the focused one takes the focus and the view follows it;
/// without, as the wheel does, only the view moves. The strip is the focused window's
/// (`focused`), or else `screen`'s. Returns the window to focus.
pub fn tiling_scroll(focused: isize, screen: Option<&str>, left: bool, walk: bool) -> Result<Option<isize>, String> {
    with_engine(|e| e.tiled_scroll(focused, screen, left, walk))
        .unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Move the focused window's column left or right (`Some`), or give it the next width (`None`).
pub fn tiling_column(focused: isize, left: Option<bool>) -> Result<(), String> {
    with_engine(|e| e.tiled_column(focused, left)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Seam is quitting: every parked window comes back onto its screen, a minimized one
/// given that place to restore to, so none is left outside every monitor. Free all for
/// quit has put them back already when it ran.
pub fn tiling_unpark_for_quit(until: Instant) {
    with_engine(|e| {
        let screens = e.screens_now();
        let trees = std::mem::take(&mut e.tiling.trees);
        for t in &trees {
            e.unpark(t, Some(until), &[], &screens);
        }
        e.tiling.trees = trees;
    });
}

/// The display changed: every parked window comes back at once, onto the screens there
/// are now (`mons`, read fresh), not the ones the snapping knew, which may be gone. The
/// strips park them again once `update` has the new screens.
pub fn tiling_display_changed(mons: &[Mon]) {
    let screens: Vec<(Rect, Rect)> =
        mons.iter().filter_map(|m| Some((rect_of(&m.rect), work_area(rect_of(&m.rect))?.0))).collect();
    with_engine(|e| e.scroll_unpark_all(&screens));
}

/// Seam starts: a window a Seam that crashed left parked, by its park stamp, or a tiled
/// window wholly off every screen, comes back to where it was before tiling took it, or
/// else the middle of the nearest screen. A minimized one is given that place to restore
/// to. Smart tiling then tiles it again from there. Before the tray's services start, so
/// the windows are back before anything looks at them; a hung window is left alone.
pub fn tiling_unstrand(mons: &[Mon]) {
    let screens: Vec<Rect> = mons.iter().map(|m| rect_of(&m.rect)).collect();
    let mut back = 0;
    for h in winutil::top_level_windows() {
        let stamp = winutil::park_stamp(h);
        let minimized = winutil::is_minimized(h);
        let frame = if minimized { winutil::normal_rect(h) } else { winutil::visible_rect(h) };
        let Some(frame) = frame else { continue };
        let lost = strip::stranded(&screens, frame);
        if !(stamp.is_some() || winutil::tile_stamp(h) != 0 && lost) {
            continue;
        }
        // A hung or administrator window keeps its stamp, so a later start tries again.
        if lost && (winutil::is_hung(h) || winutil::is_elevated_over_us(h)) {
            continue;
        }
        winutil::set_park_stamp(h, None);
        if !lost {
            continue;
        }
        let Some(near) = strip::nearest(&screens, frame) else { continue };
        let Some((work, dpi)) = work_area(screens[near]) else { continue };
        let target = strip::way_back(&[stamp], &screens, work, (frame.w, frame.h));
        if minimized {
            winutil::place_minimized(h, target, dpi);
        } else {
            winutil::fit(h, target);
        }
        back += 1;
    }
    if back > 0 {
        log::info!("scrolling: {back} windows a Seam before left off every screen came back");
    }
}

impl Engine {
    /// The scrolling tree of the strip `focused` is a column of, or else of `screen`, on
    /// the desktop in front, or why there is none.
    fn scroll_tree(&self, focused: isize, screen: Option<&str>) -> Result<usize, String> {
        if !self.active() {
            return Err("Zones are off, or Seam is standing aside".into());
        }
        if crate::switcher::switching() {
            return Err("A workspace is switching".into());
        }
        let scrolling = |i: &usize| self.here(*i) && self.tiling.trees[*i].strip.is_some();
        let by_focus = self.tree_of(focused).filter(scrolling);
        let by_screen = || {
            let screen = screen?;
            let i = self.tiling.trees.iter().position(|t| t.screen == screen && t.desktop == self.tiling.desktop)?;
            Some(i).filter(scrolling).filter(|_| self.tiling_on(screen))
        };
        by_focus.or_else(by_screen).ok_or_else(|| "Scrolling is off on this screen".into())
    }

    /// A strip changed: its view is the tree, and its windows move.
    fn scroll_set(&mut self, i: usize, strip: Strip) {
        let t = &mut self.tiling.trees[i];
        t.tree = strip.tree();
        t.strip = Some(strip);
        self.tiled_apply(i);
    }

    pub(in crate::snap) fn tiled_scroll(
        &mut self,
        focused: isize,
        screen: Option<&str>,
        left: bool,
        walk: bool,
    ) -> Result<Option<isize>, String> {
        let i = self.scroll_tree(focused, screen)?;
        let Some(strip) = &self.tiling.trees[i].strip else { return Err("Scrolling is off on this screen".into()) };
        let (next, focus) =
            if walk { strip.walked(left).map(|(s, h)| (s, Some(h))) } else { strip.scrolled(left).map(|s| (s, None)) }
                .ok_or("The strip ends there")?;
        log::info!("scrolling: the view is on column {} of {}", next.view + 1, next.cols.len());
        self.scroll_set(i, next);
        Ok(focus)
    }

    pub(in crate::snap) fn tiled_column(&mut self, focused: isize, left: Option<bool>) -> Result<(), String> {
        let i = self.scroll_tree(focused, None)?;
        let Some(strip) = &self.tiling.trees[i].strip else { return Err("Scrolling is off on this screen".into()) };
        let area_w = self.tile_area(&self.tiling.trees[i].screen).map_or(0, |(work, ..)| work.w);
        let next = match left {
            Some(left) => strip.moved(focused, left).ok_or("The column is at the end of the strip")?,
            None => strip.widened(focused, area_w).ok_or("The focused window isn't a column")?,
        };
        self.scroll_set(i, next);
        Ok(())
    }

    /// A tiled window took the focus. On a scrolling screen its column is the focused one,
    /// and a parked one is scrolled into view: Alt+Tab, the taskbar, the overview's Enter.
    /// While nothing may move, it waits for tiling to be active again (D566).
    pub(in crate::snap) fn tiled_focused(&mut self, key: isize) {
        let Some(i) = self.tree_of(key) else { return };
        let Some(strip) = &self.tiling.trees[i].strip else { return };
        // Gone to on another desktop: the worker says the desktop changed next, then it scrolls.
        if !self.here(i) {
            self.tiling.focus_wait = Some(key);
            return;
        }
        let next = strip.focused(key);
        if next.tree() == self.tiling.trees[i].tree {
            self.tiling.trees[i].strip = Some(next);
            return;
        }
        if !self.active() || crate::switcher::switching() || self.drag.is_some() {
            self.tiling.focus_wait = Some(key);
            return;
        }
        log::info!("scrolling: a parked window took the focus, so its column comes into view");
        self.scroll_set(i, next);
    }

    /// Tiling is active again: a parked window that took the focus meanwhile comes into view.
    pub(in crate::snap) fn scroll_focus_waited(&mut self) {
        // Only while it still has the focus: one gone to since is in view already, or wasn't wanted.
        if let Some(key) = self.tiling.focus_wait.take()
            && unsafe { GetForegroundWindow() }.0 as isize == key
        {
            self.tiled_focused(key);
        }
    }

    /// Tree `i`'s view as something else left it (a drag, a window taking over its stack's
    /// tile) goes into the strip, and the view is the strip's again.
    pub(super) fn scroll_absorb(&mut self, i: usize) {
        let t = &mut self.tiling.trees[i];
        let Some(strip) = &t.strip else { return };
        let strip = strip.absorb(&t.tree);
        t.tree = strip.tree();
        t.strip = Some(strip);
    }

    /// A column's window closed or was minimized: another window of its stack takes its
    /// column, or the column goes. A minimized one takes a new column when restored.
    pub(super) fn scroll_left(&mut self, i: usize, key: isize, minimized: bool) {
        if self.here(i) {
            self.snapped.remove(&key);
        }
        if !self.tiled_heir(i, key) {
            let t = &mut self.tiling.trees[i];
            let Some(strip) = &t.strip else { return };
            let strip = strip.remove(key);
            t.tree = strip.tree();
            t.strip = Some(strip);
            // A parked window minimized keeps its record, so a quit brings it back onto the screen.
            if !minimized {
                t.offscreen.remove(&key);
            }
            if minimized {
                t.resting.retain(|(k, ..)| *k != key);
                t.resting.push((key, usize::MAX, Instant::now(), false));
            }
        }
        self.tiled_apply(i);
    }

    /// A switch has ended: a column it left minimized goes, and takes a new column when restored.
    pub(super) fn scroll_rest_after_switch(&mut self, i: usize, now: Instant) {
        let t = &self.tiling.trees[i];
        let Some(strip) = &t.strip else { return };
        let down: Vec<isize> = strip.windows().filter(|k| winutil::is_minimized(hwnd_of(*k))).collect();
        if down.is_empty() {
            return;
        }
        let mut strip = strip.clone();
        for &key in &down {
            strip = strip.remove(key);
            self.snapped.remove(&key);
        }
        let t = &mut self.tiling.trees[i];
        for key in down {
            t.resting.retain(|(k, ..)| *k != key);
            t.resting.push((key, usize::MAX, now, false));
        }
        log::info!("scrolling: a switch left columns minimized, so they leave the strip");
        self.scroll_set(i, strip);
    }

    /// A handle drag between columns: each column in view takes the width nearest its share.
    pub(super) fn scroll_dragged(&mut self, i: usize, node: &Node) {
        let Some(strip) = &self.tiling.trees[i].strip else { return };
        let unit = Rect { x: 0, y: 0, w: 6000, h: 100 };
        let shares: Vec<f32> = zones::rects(node, unit, 0).iter().map(|r| r.w as f32 / 6000.0).collect();
        let area_w = self.tile_area(&self.tiling.trees[i].screen).map_or(0, |(work, ..)| work.w);
        let next = strip.with_shares(&shares, area_w);
        self.scroll_set(i, next);
    }

    /// Another family for tree `i`, to Scrolling or from it. Into Scrolling every window,
    /// stacked ones too, becomes a half-wide column in its order. Out of it the columns
    /// become tiles in their order, and a strip past what a tree holds (`MAX_TILES`)
    /// leaves the rest floating, on the screen.
    pub(super) fn scroll_family(&mut self, i: usize, family: Family) {
        let Some((work, dpi, gap)) = self.tile_area(&self.tiling.trees[i].screen) else { return };
        let t = &self.tiling.trees[i];
        if family == Family::Scrolling {
            let mut tiles: Vec<tiling::Tile> = t.tree.tiles.iter().copied().filter(|x| x.hwnd.is_some()).collect();
            let stacked = t.overflow.clone();
            tiles.extend(stacked.iter().map(|(k, min)| tiling::Tile { hwnd: Some(*k), min: *min }));
            for (k, _) in &stacked {
                self.stack_leave(*k);
            }
            let t = &mut self.tiling.trees[i];
            t.overflow.clear();
            for r in &mut t.resting {
                r.1 = usize::MAX;
            }
            return self.scroll_set(i, Strip::of(&tiles));
        }
        let mut whole = t.strip.as_ref().map(Strip::whole).unwrap_or_else(|| t.tree.clone());
        let extra: Vec<isize> = whole.tiles.iter().skip(tiling::MAX_TILES).filter_map(|x| x.hwnd).collect();
        whole.tiles.truncate(tiling::MAX_TILES);
        for k in extra {
            log::info!("scrolling: more columns than another family holds, so one floats");
            let target = self.home_of(&self.tiling.trees[i], k);
            winutil::set_park_stamp(hwnd_of(k), None);
            let t = &mut self.tiling.trees[i];
            t.before.remove(&k);
            if t.offscreen.remove(&k).is_some()
                && let Some(target) = target
            {
                self.queued.push_back(Placement { hwnd: k, target, zone: None, dpi, maximize: false });
            }
            winutil::set_tile_stamp(hwnd_of(k), 0);
            winutil::set_floated(hwnd_of(k), true);
        }
        let t = &mut self.tiling.trees[i];
        t.strip = None;
        // The parked windows are tiles now, and move into them.
        for k in std::mem::take(&mut t.offscreen).into_keys() {
            winutil::set_park_stamp(hwnd_of(k), None);
        }
        t.tree = whole.with_family(family, work, gap);
        self.tiled_apply(i);
    }

    /// Whether a parked window isn't where it was parked: its app, or something else,
    /// moved it back. It floats, rather than be fought over.
    pub(super) fn parked_moved(&self, key: isize) -> bool {
        let h = hwnd_of(key);
        let Some(at) = self.tiling.trees.iter().find_map(|t| t.offscreen.get(&key)) else { return false };
        if winutil::is_minimized(h) || winutil::is_maximized(h) {
            return false;
        }
        winutil::visible_rect(h).is_some_and(|now| tiling::moved(now, *at, winutil::is_resizable(h)))
    }

    /// The parked windows of tree `i` that may move and aren't parked yet, as moves off
    /// every screen. Their records in the zones go at once, so the view's zones are
    /// only the view's windows.
    pub(super) fn scroll_parks(&mut self, i: usize, movable: &[isize]) -> Vec<Placement> {
        let t = &self.tiling.trees[i];
        let Some(strip) = &t.strip else { return Vec::new() };
        let Some((work, dpi, gap)) = self.tile_area(&t.screen) else { return Vec::new() };
        let Some(own) = self.screens.iter().find(|s| s.id == t.screen).map(|s| s.bounds) else { return Vec::new() };
        let screens: Vec<Rect> = self.screens.iter().map(|s| s.bounds).collect();
        let mut parks = Vec::new();
        let mut parked = HashMap::new();
        for (key, place) in strip.places(work, gap) {
            let Place::Parked { size, left } = place else { continue };
            let target = strip::park(own, &screens, size, left);
            let h = hwnd_of(key);
            let there = t.offscreen.get(&key) == Some(&target)
                && winutil::visible_rect(h).is_some_and(|r| close(r, target, winutil::is_resizable(h)));
            if movable.contains(&key) && !there {
                // Where it goes back to, kept on the window before it leaves, so a crash can't strand it.
                let now = winutil::visible_rect(h);
                let choices = [t.before.get(&key).copied(), winutil::park_stamp(h), now];
                let size = now.map_or((work.w / 2, work.h), |r| (r.w, r.h));
                winutil::set_park_stamp(h, Some(strip::way_back(&choices, &screens, work, size)));
                parks.push(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
            }
            // One that may not move now keeps the record it had, if any, so the rule sees where it went.
            let record = if movable.contains(&key) { Some(target) } else { t.offscreen.get(&key).copied() };
            if let Some(at) = record {
                parked.insert(key, at);
            }
        }
        for p in &parks {
            self.snapped.remove(&p.hwnd);
        }
        // Back in view: no longer parked.
        for k in self.tiling.trees[i].offscreen.keys().filter(|k| !parked.contains_key(*k)) {
            winutil::set_park_stamp(hwnd_of(*k), None);
        }
        if log::enabled() && !parks.is_empty() {
            log::info!("scrolling: {} columns park off every screen", parks.len());
        }
        self.tiling.trees[i].offscreen = parked;
        parks
    }

    /// The screen a window off every screen is nearest, as Windows has it.
    pub(super) fn parked_home(&self, frame: Rect) -> Option<String> {
        let screens: Vec<Rect> = self.screens.iter().map(|s| s.bounds).collect();
        strip::nearest(&screens, frame).map(|i| self.screens[i].id.clone())
    }

    /// The screens the snapping knows, each as (bounds, work area), for `strip::home`.
    pub(super) fn screens_now(&self) -> Vec<(Rect, Rect)> {
        self.screens.iter().filter_map(|s| Some((s.bounds, work_area(s.bounds)?.0))).collect()
    }

    /// Where a window of `t` comes back to when it leaves the strip parked, by `home_on`
    /// on the screens the snapping knows.
    pub(super) fn home_of(&self, t: &Tiled, key: isize) -> Option<Rect> {
        self.home_on(t, key, &self.screens_now())
    }

    /// Where a window of `t` comes back to on `screens`, by `strip::home`: its place
    /// before tiling, then its park stamp's, when on one of them; else the middle of its
    /// own screen, or the nearest one when its own is gone or moved. None with no screen.
    fn home_on(&self, t: &Tiled, key: isize, screens: &[(Rect, Rect)]) -> Option<Rect> {
        let h = hwnd_of(key);
        let now = if winutil::is_minimized(h) { winutil::normal_rect(h) } else { winutil::visible_rect(h) };
        let before = t.before.get(&key).copied();
        let size = now.or(before).map_or((800, 600), |r| (r.w, r.h));
        let own = self.screens.iter().find(|s| s.id == t.screen).map(|s| s.bounds);
        strip::home(&[before, winutil::park_stamp(h)], screens, own, now.or(before), size)
    }

    /// The DPI of the screen a place is on, for its move.
    fn dpi_at(&self, r: Rect) -> u32 {
        let on = self.screens.iter().find(|s| s.bounds.contains(r.x + r.w / 2, r.y + r.h / 2));
        on.and_then(|s| work_area(s.bounds)).map_or(96, |(_, dpi)| dpi)
    }

    /// Moves queued or under way for these windows are dropped, so a park posted a moment
    /// ago can't send a window back off the screen after it came back. The queue goes on
    /// with the next window's move.
    fn cancel_moves(&mut self, keys: &[isize]) {
        self.queued.retain(|p| !keys.contains(&p.hwnd));
        if let Some(p) = self.pending.take_if(|p| keys.contains(&p.hwnd)) {
            unsafe {
                let _ = KillTimer(None, p.timer);
            }
            self.window_pump();
        }
    }

    /// Every window `t` holds or parked that is off every screen of `screens` lands on one
    /// of them, by `strip::land`. A minimized one is left to restore where Windows puts it.
    fn land_lost(&mut self, t: &Tiled, screens: &[(Rect, Rect)]) {
        let mut keys = t.all();
        keys.extend(t.offscreen.keys().filter(|k| !keys.contains(k)).copied().collect::<Vec<_>>());
        let own = self.screens.iter().find(|s| s.id == t.screen).map(|s| s.bounds);
        for key in keys {
            let h = hwnd_of(key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_minimized(h) {
                continue;
            }
            let Some(frame) = winutil::visible_rect(h) else { continue };
            let choices = [t.before.get(&key).copied(), winutil::park_stamp(h)];
            let Some(target) = strip::land(frame, &choices, screens, own, (frame.w, frame.h)) else { continue };
            log::info!("scrolling: a tiled window was off every screen after a display change, so it comes back");
            winutil::set_park_stamp(h, None);
            self.cancel_moves(&[key]);
            let dpi = self.dpi_at(target);
            self.land_now(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
        }
    }

    /// A parked window of tree `i` leaves the strip (it floats, or went to another
    /// desktop): wherever it is off every screen, it comes back onto one.
    pub(super) fn scroll_let_go(&mut self, i: usize, key: isize) {
        let t = &self.tiling.trees[i];
        if !t.offscreen.contains_key(&key) && winutil::park_stamp(hwnd_of(key)).is_none() {
            return;
        }
        let h = hwnd_of(key);
        let screens: Vec<Rect> = self.screens.iter().map(|s| s.bounds).collect();
        let lost = winutil::visible_rect(h).is_some_and(|r| strip::stranded(&screens, r));
        let home = self.home_of(t, key);
        let dpi = self.tile_area(&t.screen).map_or(96, |(_, dpi, _)| dpi);
        winutil::set_park_stamp(h, None);
        self.tiling.trees[i].offscreen.remove(&key);
        if let (true, Some(target)) = (lost && unsafe { IsWindow(Some(h)) }.as_bool(), home) {
            log::info!("scrolling: a parked window left the strip off every screen, so it comes back onto one");
            self.cancel_moves(&[key]);
            self.land_now(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
        }
    }

    /// The screens changed (one went, came, moved or rescaled): every parked window comes
    /// back first, a strip whose screen is gone ends, and the rest park again for the
    /// screens there are now. A window of a strip left off every screen is brought back too.
    pub(in crate::snap) fn scroll_screens_changed(&mut self) {
        let screens = self.screens_now();
        self.scroll_unpark_all(&screens);
        // A moment with no screen at all, mid-change, ends nothing.
        if self.screens.is_empty() {
            return;
        }
        let gone: Vec<usize> = (0..self.tiling.trees.len())
            .filter(|&i| !self.screens.iter().any(|s| s.id == self.tiling.trees[i].screen))
            .collect();
        for i in gone.into_iter().rev() {
            let t = self.tiling.trees.remove(i);
            log::info!("scrolling: a tiled screen went, so its tiling ends there");
            // Its windows, parked a moment ago included, land on a screen still there.
            self.land_lost(&t, &screens);
            self.tiled_release(t);
        }
        let trees = std::mem::take(&mut self.tiling.trees);
        for t in &trees {
            self.land_lost(t, &screens);
        }
        self.tiling.trees = trees;
        self.tiled_sync();
    }

    /// Every parked window on every strip comes back onto `screens` at once, as a display
    /// change starts, and the strips park them again once the screens are known.
    pub(in crate::snap) fn scroll_unpark_all(&mut self, screens: &[(Rect, Rect)]) {
        let trees = std::mem::take(&mut self.tiling.trees);
        for t in &trees {
            self.unpark(t, None, &[], screens);
        }
        self.tiling.trees = trees;
        for t in &mut self.tiling.trees {
            if !t.offscreen.is_empty() {
                t.offscreen.clear();
                t.dirty = true;
            }
        }
    }

    /// A window off every screen that tiling knows, which nothing tiles now: it comes
    /// back onto `screen`.
    pub(super) fn rescue(&mut self, key: isize, screen: &str, frame: Rect) {
        let screens: Vec<Rect> = self.screens.iter().map(|s| s.bounds).collect();
        if !strip::stranded(&screens, frame) {
            return;
        }
        winutil::set_park_stamp(hwnd_of(key), None);
        let Some((work, dpi, _)) = self.tile_area(screen) else { return };
        let (w, h) = (frame.w.min(work.w), frame.h.min(work.h));
        let target = Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h };
        for t in &mut self.tiling.trees {
            t.offscreen.remove(&key);
        }
        log::info!("scrolling: a window left off every screen comes back onto its screen");
        self.land_now(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
    }

    /// Every parked window of `t` but `skip` comes back onto one of `screens`, by `home_on`,
    /// and any move still queued for it is dropped. A minimized one is given that place to
    /// restore to without showing it: at once when Seam quits, until `until`, else on a
    /// thread of its own, since that waits on its app.
    pub(super) fn unpark(&mut self, t: &Tiled, until: Option<Instant>, skip: &[isize], screens: &[(Rect, Rect)]) {
        let keys: Vec<isize> = t.offscreen.keys().copied().filter(|k| !skip.contains(k)).collect();
        if keys.is_empty() {
            return;
        }
        self.cancel_moves(&keys);
        let mut minimized = Vec::new();
        for &key in &keys {
            let h = hwnd_of(key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                continue;
            }
            let Some(target) = self.home_on(t, key, screens) else { continue };
            let dpi = self.dpi_at(target);
            winutil::set_park_stamp(h, None);
            if winutil::is_minimized(h) {
                minimized.push((key, target, dpi));
            } else {
                self.land_now(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
            }
        }
        log::info!("scrolling: {} parked windows come back onto the screen", keys.len());
        if let Some(until) = until {
            // The quit's one second, shared with the pins and Free all: a slow app never holds it up.
            for (key, target, dpi) in minimized {
                if Instant::now() >= until {
                    log::info!("scrolling: out of time on quit, a minimized parked window keeps its place");
                    break;
                }
                if !winutil::is_hung(hwnd_of(key)) {
                    winutil::place_minimized(hwnd_of(key), target, dpi);
                }
            }
        } else if !minimized.is_empty() {
            let spawned = std::thread::Builder::new().name("seam-unpark".into()).spawn(move || {
                for (key, target, dpi) in minimized {
                    let h = hwnd_of(key);
                    if !winutil::is_hung(h) {
                        winutil::place_minimized(h, target, dpi);
                    }
                }
            });
            if spawned.is_err() {
                log::info!("scrolling: couldn't start the thread for minimized parked windows");
            }
        }
    }

    /// Whether tiling stands down now: Seam paused, stepped aside or zones off. A full-screen
    /// app in front isn't that; the parked windows wait behind it.
    pub(in crate::snap) fn scroll_standing(&mut self, down: bool) {
        if down && !self.tiling.stood_down {
            self.scroll_stand_down();
        }
        self.tiling.stood_down = down;
    }

    /// Tiling stood down: no window may stay parked where nothing scrolls it back, so
    /// every parked window comes onto its screen, and the strips park them again once
    /// tiling is back.
    fn scroll_stand_down(&mut self) {
        let screens = self.screens_now();
        self.scroll_unpark_all(&screens);
    }
}
