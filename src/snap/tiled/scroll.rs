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

/// Seam is quitting: every parked window comes back onto its screen, so none is left
/// outside every monitor. Free all for quit has put them back already when it ran.
pub fn tiling_unpark_for_quit() {
    with_engine(|e| {
        let trees = std::mem::take(&mut e.tiling.trees);
        for t in &trees {
            e.unpark(t);
        }
        e.tiling.trees = trees;
    });
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
        let next = match left {
            Some(left) => strip.moved(focused, left).ok_or("The column is at the end of the strip")?,
            None => strip.widened(focused).ok_or("The focused window isn't a column")?,
        };
        self.scroll_set(i, next);
        Ok(())
    }

    /// A tiled window took the focus. On a scrolling screen its column is the focused one,
    /// and a parked one is scrolled into view: Alt+Tab, the taskbar, the overview's Enter.
    /// While nothing may move, it waits for tiling to be active again (D566).
    pub(in crate::snap) fn tiled_focused(&mut self, key: isize) {
        let Some(i) = self.tree_of(key).filter(|i| self.here(*i)) else { return };
        let Some(strip) = &self.tiling.trees[i].strip else { return };
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
        let next = strip.with_shares(&shares);
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
            let target = self.unpark_target(&self.tiling.trees[i], k);
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
        t.offscreen.clear();
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

    /// Where a window comes back to on `t`'s screen when it leaves the strip parked: the
    /// middle of the work area, at its size up to the work area's.
    pub(super) fn unpark_target(&self, t: &Tiled, key: isize) -> Option<Rect> {
        let (work, ..) = self.tile_area(&t.screen)?;
        let now = winutil::visible_rect(hwnd_of(key))?;
        let (w, h) = (now.w.min(work.w), now.h.min(work.h));
        Some(Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h })
    }

    /// A window off every screen that tiling knows, which nothing tiles now: it comes
    /// back onto `screen`.
    pub(super) fn rescue(&mut self, key: isize, screen: &str, frame: Rect) {
        if self.screens.iter().any(|s| touches(s.bounds, frame)) {
            return;
        }
        let Some((work, dpi, _)) = self.tile_area(screen) else { return };
        let (w, h) = (frame.w.min(work.w), frame.h.min(work.h));
        let target = Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h };
        for t in &mut self.tiling.trees {
            t.offscreen.remove(&key);
        }
        log::info!("scrolling: a window left off every screen comes back onto its screen");
        self.land_now(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
    }

    /// Every parked window of `t` comes back onto its screen, to where it was before
    /// tiling took it, or else the middle of the work area. A minimized one is given that
    /// place to restore to, on a thread of its own, since that waits on its app.
    pub(super) fn unpark(&mut self, t: &Tiled) {
        if t.offscreen.is_empty() {
            return;
        }
        let dpi = self.tile_area(&t.screen).map_or(96, |(_, dpi, _)| dpi);
        let mut minimized = Vec::new();
        for &key in t.offscreen.keys() {
            let h = hwnd_of(key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                continue;
            }
            let Some(target) = t.before.get(&key).copied().or_else(|| self.unpark_target(t, key)) else { continue };
            if winutil::is_minimized(h) {
                minimized.push((key, target));
            } else {
                self.land_now(Placement { hwnd: key, target, zone: None, dpi, maximize: false });
            }
        }
        log::info!("scrolling: {} parked windows come back onto the screen", t.offscreen.len());
        if !minimized.is_empty() {
            let spawned = std::thread::Builder::new().name("seam-unpark".into()).spawn(move || {
                for (key, target) in minimized {
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
        let trees = std::mem::take(&mut self.tiling.trees);
        for t in &trees {
            self.unpark(t);
        }
        self.tiling.trees = trees;
        for t in &mut self.tiling.trees {
            if !t.offscreen.is_empty() {
                t.offscreen.clear();
                t.dirty = true;
            }
        }
    }
}

/// Whether a rect is on a screen, in part.
fn touches(screen: Rect, r: Rect) -> bool {
    r.x < screen.x + screen.w && screen.x < r.x + r.w && r.y < screen.y + screen.h && screen.y < r.y + r.h
}
