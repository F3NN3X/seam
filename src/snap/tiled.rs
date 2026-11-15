//! Smart tiling on the snapping's engine. A tiled screen shows its tree for the
//! virtual desktop in front as its zones, in memory only, so a drag, the gap handles,
//! S and the drag keys work on it as on any zones. Its own layout stays saved under
//! it and comes back when tiling is off. The tray thread only runs the pure tree step
//! (`seam::tiling`) on what the worker read (`tiler`), and the windows land through
//! the placement queue.
//!
//! It never fights a hand edit: a window that isn't where tiling put it is floated
//! before anything would move it back, and a window dragged out of its tile floats,
//! leaving the tile empty so nothing else moves for it (D562).

use super::*;
use crate::tiler::{Arrival, Event};
use seam::config::TileScreen;
use seam::tiling::{self, Family, Put, Tree};
use seam::zones::{LeafTag, Node, Template};
use std::collections::BTreeMap;

/// A tiled window this far off its tile, or this much bigger, was moved since. One that
/// came out a little smaller, as a terminal that sizes itself by whole characters does,
/// wasn't (`SMALLER_PX`).
const MOVED_PX: i32 = 4;
const SMALLER_PX: i32 = 48;

/// One screen's tree on one virtual desktop.
pub(super) struct Tiled {
    screen: String,
    desktop: String,
    tree: Tree,
    /// Windows stacked in a tile because they couldn't shrink to a share, and their
    /// smallest size. They take a tile of their own when room frees up.
    overflow: Vec<(isize, tiling::Min)>,
    /// Windows that left by minimizing, which take a share again when restored.
    resting: Vec<isize>,
    /// Where each window's frame was before tiling took it, for Free all.
    before: HashMap<isize, Rect>,
    /// The snapping's records of its windows while another desktop is in front.
    parked: Vec<(isize, Snapped)>,
    /// A window left while its desktop wasn't in front: the gap closes once it is.
    dirty: bool,
}

#[derive(Default)]
pub(super) struct Tiling {
    /// This desk's settings, by screen.
    config: BTreeMap<String, TileScreen>,
    trees: Vec<Tiled>,
    /// The virtual desktop in front, as the worker last said.
    desktop: String,
    /// The worker has said which desktop is in front this run.
    known: bool,
    /// The last `Seam_tile` order given.
    seq: u32,
    /// Windows that arrived during a drag, taken once it is over.
    waiting: Vec<Arrival>,
    /// The worker was asked for the windows a tray that ran before tiled.
    looked: bool,
    /// Handler timings while logging: batches, total and longest, in microseconds.
    stats: (u64, u128, u128),
}

/// A tiled screen as the state shows it.
pub struct TiledInfo {
    pub screen: String,
    pub windows: usize,
}

/// Take what the tiling worker found. Runs on the tray thread for `WM_APP_TILING`.
/// False when the snapping is busy, and the events wait for the next try.
pub fn tiling_events() -> bool {
    let taken = with_engine(|e| {
        let events = crate::tiler::take();
        if events.is_empty() {
            return;
        }
        let started = Instant::now();
        for event in events {
            match event {
                Event::Arrived(a) => e.tile_arrived(a),
                Event::Left { hwnd, minimized } => e.tile_left(hwnd, minimized),
                Event::Desktop(id) => e.tiled_desktop(id),
            }
        }
        if log::enabled() {
            // The desk gate: the tray thread's share of tiling stays far under 2 ms a message.
            let us = started.elapsed().as_micros();
            let s = &mut e.tiling.stats;
            *s = (s.0 + 1, s.1 + us, s.2.max(us));
            if s.0 % 20 == 1 || us > 2000 {
                log::info!(
                    "tiling stats: {} batches, avg {} us, max {} us, this {us} us",
                    s.0,
                    s.1 / u128::from(s.0),
                    s.2
                );
            }
        }
    });
    taken.is_some()
}

/// A drag of an edge or a gap handle changed a tiled screen's zones: the tree takes
/// them, and the screen's own layout isn't saved. False when the screen isn't tiled
/// on the desktop in front.
pub fn tiled_dragged(screen: &str, tree: &Node) -> bool {
    with_engine(|e| e.tiled_dragged(screen, tree)).unwrap_or(false)
}

/// Free all windows: tiling ends on the screen for the desktop in front, and every
/// window it tiled goes back to where it was before. Returns how many.
pub fn tiling_free(screen: &str) -> usize {
    with_engine(|e| e.tiling_free(screen)).unwrap_or(0)
}

/// "Float or tile this window": a tiled window floats and the others close up, and a
/// window on a tiled screen is asked for again. Returns whether it floats now.
pub fn tiling_float(hwnd: HWND) -> Result<bool, String> {
    with_engine(|e| e.tiling_float(hwnd.0 as isize)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Whether a screen is tiled on the desktop in front.
pub fn is_tiled(screen: &str) -> bool {
    with_engine(|e| e.tiling_on(screen)).unwrap_or(false)
}

/// The desktop in front, once the worker has said.
pub fn tiling_desktop() -> Option<String> {
    with_engine(|e| e.tiling.known.then(|| e.tiling.desktop.clone())).flatten()
}

/// Each screen tiled on the desktop in front, with how many windows it tiles.
pub fn tiled() -> Vec<TiledInfo> {
    let info = |e: &Engine| {
        let here = e.tiling.trees.iter().filter(|t| t.desktop == e.tiling.desktop);
        here.map(|t| TiledInfo { screen: t.screen.clone(), windows: t.tree.windows().count() + t.overflow.len() })
            .collect()
    };
    with_engine(|e| info(e)).unwrap_or_default()
}

impl Engine {
    /// Tiling is on for this screen on the desktop in front.
    pub(super) fn tiling_on(&self, screen: &str) -> bool {
        self.tiling.known && self.tiling.config.get(screen).is_some_and(|t| t.on(&self.tiling.desktop))
    }

    /// The zones a tiled screen shows in place of its own: its tree on the desktop in front.
    fn tiled_layout(&self, screen: &str) -> Option<ZoneLayout> {
        if !self.tiling_on(screen) {
            return None;
        }
        let tree = self.tiling.trees.iter().find(|t| t.screen == screen && t.desktop == self.tiling.desktop);
        let tree = tree.map_or(Node::Leaf(LeafTag::Leaf), |t| t.tree.layout());
        Some(ZoneLayout { template: Template::Custom, custom: None, tree })
    }

    /// The screens with each tiled one's tree as its zones.
    pub(super) fn with_tiled(&self, screens: Vec<Screen>) -> Vec<Screen> {
        let tiled = |mut s: Screen| {
            s.layout = self.tiled_layout(&s.id).or_else(|| s.own.clone());
            s
        };
        screens.into_iter().map(tiled).collect()
    }

    /// A tree changed: the screens show it, and the seam-wm thread hears which windows are tiled.
    fn tiled_sync(&mut self) {
        let taken = std::mem::take(&mut self.screens);
        let screens = self.with_tiled(taken);
        let layouts = |list: &[Screen]| list.iter().map(|s| s.layout.clone()).collect::<Vec<_>>();
        let changed = layouts(&screens) != layouts(&self.screens);
        self.screens = screens;
        if changed {
            self.sync_hook();
            self.sync_overlay();
        }
        self.sync_dividers();
        let tiled = |t: &Tiled| {
            let stacked = t.overflow.iter().map(|(h, _)| *h);
            t.tree.windows().chain(stacked).chain(t.resting.iter().copied()).collect::<Vec<_>>()
        };
        crate::tiler::set_tiled(self.tiling.trees.iter().flat_map(tiled).collect());
    }

    /// This desk's tiling settings, from `update`. Trees whose screen is off on their
    /// desktop end, their windows staying where they are. A changed family lays the
    /// windows out again. The first time any is on, the worker looks for the windows a
    /// tray that ran before tiled, and which desktop is in front.
    pub(super) fn tiling_config(&mut self, config: BTreeMap<String, TileScreen>) {
        if config == self.tiling.config {
            return;
        }
        let before = std::mem::replace(&mut self.tiling.config, config);
        let off = |t: &Tiled, c: &BTreeMap<String, TileScreen>| c.get(&t.screen).is_none_or(|s| !s.on(&t.desktop));
        let ended: Vec<Tiled> = {
            let (gone, kept) =
                std::mem::take(&mut self.tiling.trees).into_iter().partition(|t| off(t, &self.tiling.config));
            self.tiling.trees = kept;
            gone
        };
        for t in ended {
            log::info!("tiling: off on a screen, {} windows stay where they are", t.tree.windows().count());
            self.tiled_release(t);
        }
        for i in 0..self.tiling.trees.len() {
            let t = &self.tiling.trees[i];
            let family = self.tiling.config.get(&t.screen).map(|c| c.family).unwrap_or_default();
            if family != t.tree.family {
                self.tiled_family(i, family);
            }
        }
        // A screen that is tiled now: what the snapping held in its own zones isn't tiling's.
        let fresh: Vec<String> = self
            .tiling
            .config
            .keys()
            .filter(|s| !before.get(*s).is_some_and(|b| b.on(&self.tiling.desktop)))
            .cloned()
            .collect();
        let fresh: Vec<String> = fresh.into_iter().filter(|s| self.tiling_on(s)).collect();
        for screen in &fresh {
            self.tiled_adopt_none(screen);
        }
        let any = self.tiling.config.values().any(|c| !c.desktops.is_empty());
        if any && !self.tiling.looked {
            self.tiling.looked = true;
            crate::tiler::retile();
        }
        self.tiled_sync();
    }

    /// The snapping forgets the windows it held in a screen's own zones, which tiling's
    /// tree replaces. They stay where they are.
    fn tiled_adopt_none(&mut self, screen: &str) {
        let tiled: HashSet<isize> = self.tiling.trees.iter().flat_map(|t| t.tree.windows()).collect();
        let foreign: Vec<isize> = self
            .snapped
            .iter()
            .filter(|(k, s)| s.screen == screen && !tiled.contains(k) && !winutil::is_cloaked(hwnd_of(**k)))
            .map(|(k, _)| *k)
            .collect();
        for k in foreign {
            self.unsnap(hwnd_of(k), true);
        }
    }

    /// A tree ends: its windows stay where they are, out of tiling and out of zones.
    fn tiled_release(&mut self, t: Tiled) {
        let windows: Vec<isize> = t.tree.windows().chain(t.overflow.iter().map(|(h, _)| *h)).collect();
        for k in windows {
            winutil::set_tile_stamp(hwnd_of(k), 0);
            if t.desktop == self.tiling.desktop {
                self.unsnap(hwnd_of(k), true);
            } else {
                // Its zones were the tree's, which a re-fit mustn't read as the screen's own.
                winutil::clear_zone_stamp(hwnd_of(k));
            }
        }
    }

    /// The tree of a screen on a desktop, made when there is none.
    fn tree_index(&mut self, screen: &str, desktop: &str) -> usize {
        if let Some(i) = self.tiling.trees.iter().position(|t| t.screen == screen && t.desktop == desktop) {
            return i;
        }
        let family = self.tiling.config.get(screen).map(|c| c.family).unwrap_or_default();
        self.tiling.trees.push(Tiled {
            screen: screen.to_string(),
            desktop: desktop.to_string(),
            tree: Tree::new(family),
            overflow: Vec::new(),
            resting: Vec::new(),
            before: HashMap::new(),
            parked: Vec::new(),
            dirty: false,
        });
        self.tiling.trees.len() - 1
    }

    /// The tree a window is tiled in, in a tile or stacked in one.
    fn tree_of(&self, key: isize) -> Option<usize> {
        let has = |t: &Tiled| t.tree.zone_of(key).is_some() || t.overflow.iter().any(|(k, _)| *k == key);
        self.tiling.trees.iter().position(has)
    }

    /// A tiled screen's work area, DPI and gap at its scale.
    fn tile_area(&self, screen: &str) -> Option<(Rect, u32, i32)> {
        let s = self.screens.iter().find(|s| s.id == screen)?;
        let (work, dpi) = work_area(s.bounds)?;
        let gap = (self.settings.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        Some((work, dpi, gap))
    }

    /// A window opened, was restored or asked for: it takes a share of the largest tile,
    /// or a stack in it when it can't shrink to that. Waits for a drag to end.
    fn tile_arrived(&mut self, a: Arrival) {
        if !self.active() {
            return;
        }
        if self.drag.is_some() {
            self.tiling.waiting.push(a);
            return;
        }
        // It just showed, so its desktop is the one in front.
        if !a.desktop.is_empty() || !self.tiling.known {
            self.tiled_desktop(a.desktop.clone());
        }
        let (x, y) = (a.frame.x + a.frame.w / 2, a.frame.y + a.frame.h / 2);
        let Some(screen) = self.screens.iter().find(|s| s.bounds.contains(x, y)).map(|s| s.id.clone()) else { return };
        if !self.tiling_on(&screen) {
            return;
        }
        if self.tree_of(a.hwnd).is_some() {
            // The desk gate: a window is never placed twice.
            log::info!("tiling desk gate: placed twice refused, the window is tiled already");
            return;
        }
        let rested = self.tiling.trees.iter().any(|t| t.resting.contains(&a.hwnd));
        if a.restored && !rested {
            return;
        }
        for t in &mut self.tiling.trees {
            t.resting.retain(|k| *k != a.hwnd);
        }
        let Some((work, _, gap)) = self.tile_area(&screen) else { return };
        let desktop = self.tiling.desktop.clone();
        let i = self.tree_index(&screen, &desktop);
        self.tiled_check(i);
        let Some((tree, put)) = self.tiling.trees[i].tree.insert(work, gap, a.hwnd, a.min) else { return };
        let h = hwnd_of(a.hwnd);
        let order = if a.order > 0 { a.order } else { self.tiling.seq + 1 };
        self.tiling.seq = self.tiling.seq.max(order);
        winutil::set_tile_stamp(h, order);
        winutil::set_floated(h, false);
        self.tiling.trees[i].before.entry(a.hwnd).or_insert(a.frame);
        let app = winutil::exe_name(h).unwrap_or_default();
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
    }

    /// A tiled window closed or was minimized: another window of its stack takes its
    /// tile, or its neighbours fill the gap and a stacked window that fits now leaves
    /// its stack. On a desktop that isn't in front, the gap closes once it is.
    fn tile_left(&mut self, key: isize, minimized: bool) {
        if !minimized {
            for t in &mut self.tiling.trees {
                t.resting.retain(|k| *k != key);
            }
        }
        let Some(i) = self.tree_of(key) else { return };
        let h = hwnd_of(key);
        if minimized {
            self.tiling.trees[i].resting.push(key);
        } else {
            winutil::set_tile_stamp(h, 0);
        }
        let t = &mut self.tiling.trees[i];
        if let Some(at) = t.overflow.iter().position(|(k, _)| *k == key) {
            t.overflow.remove(at);
            self.tiled_sync();
            return;
        }
        let here = t.desktop == self.tiling.desktop;
        if here {
            self.snapped.remove(&key);
        }
        let zone = self.tiling.trees[i].tree.zone_of(key);
        let live = |m: &isize| {
            *m != key && unsafe { IsWindow(Some(hwnd_of(*m))) }.as_bool() && !winutil::is_minimized(hwnd_of(*m))
        };
        // Its stack's next window, or a window tiling stacked in the same zone.
        let screen = self.tiling.trees[i].screen.clone();
        let same_zone = |k: &isize| self.snapped.get(k).is_some_and(|s| s.screen == screen && Some(s.from) == zone);
        let stacked: Vec<isize> = self.tiling.trees[i].overflow.iter().map(|(k, _)| *k).filter(same_zone).collect();
        let heir = self.stacks.of(key).and_then(|s| s.members.iter().copied().find(live));
        let heir = heir.or_else(|| stacked.into_iter().find(live));
        if let (Some(heir), Some(zone)) = (heir, zone) {
            let t = &mut self.tiling.trees[i];
            let min = t.overflow.iter().find(|(k, _)| *k == heir).map_or((0, 0), |(_, m)| *m);
            t.overflow.retain(|(k, _)| *k != heir);
            t.tree.tiles[zone] = tiling::Tile { hwnd: Some(heir), min };
            self.tiled_sync();
            return;
        }
        if here {
            self.tiled_check(i);
        }
        let t = &mut self.tiling.trees[i];
        let Some(tree) = t.tree.remove(key) else { return };
        t.tree = tree;
        self.tiled_unstack(i);
        if here {
            self.tiled_apply(i);
        } else {
            self.tiling.trees[i].dirty = true;
            self.tiled_sync();
        }
    }

    /// Stacked windows that fit a share now take one, and leave their stacks.
    fn tiled_unstack(&mut self, i: usize) {
        let Some((work, _, gap)) = self.tile_area(&self.tiling.trees[i].screen) else { return };
        for (key, min) in self.tiling.trees[i].overflow.clone() {
            let t = &mut self.tiling.trees[i];
            if let Some((tree, Put::Tile(_))) = t.tree.insert(work, gap, key, min) {
                t.tree = tree;
                t.overflow.retain(|(k, _)| *k != key);
                log::info!("tiling: a stacked window has room now, so it takes a share");
                self.stack_leave(key);
            }
        }
    }

    /// The desk gate's "moved after hand placement": a tiled window on the desktop in
    /// front that isn't where tiling put it was moved or resized since, by hand or by its
    /// app. It floats and keeps its place, and its tile stays empty, so nothing moves it back.
    fn tiled_check(&mut self, i: usize) {
        if self.tiling.trees[i].desktop != self.tiling.desktop {
            return;
        }
        let hidden = self.stack_hidden();
        let moved: Vec<isize> = self.tiling.trees[i]
            .tree
            .windows()
            .filter(|k| !hidden.contains(k) && !self.moving(*k) && self.tiled_moved(*k))
            .collect();
        for k in moved {
            log::info!("tiling desk gate: a tiled window moved since it was placed, so it floats where it is");
            self.tiled_float(i, k);
        }
    }

    /// Whether a tiled window isn't where tiling put it. A maximized or minimized one is
    /// left alone, and so is one the snapping has no record of.
    fn tiled_moved(&self, key: isize) -> bool {
        let h = hwnd_of(key);
        let away = !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_minimized(h) || winutil::is_maximized(h);
        let (Some(s), Some(now), false) = (self.snapped.get(&key), winutil::visible_rect(h), away) else {
            return false;
        };
        let at = s.at;
        let off = (now.x - at.x).abs() > MOVED_PX || (now.y - at.y).abs() > MOVED_PX;
        let bigger = now.w > at.w + MOVED_PX || now.h > at.h + MOVED_PX;
        let smaller = winutil::is_resizable(h) && (now.w < at.w - SMALLER_PX || now.h < at.h - SMALLER_PX);
        off || bigger || smaller
    }

    /// A window leaves its tile, which stays empty, and floats where it is.
    fn tiled_float(&mut self, i: usize, key: isize) {
        let t = &mut self.tiling.trees[i];
        if let Some(tree) = t.tree.vacate(key) {
            t.tree = tree;
        }
        t.overflow.retain(|(k, _)| *k != key);
        let h = hwnd_of(key);
        winutil::set_tile_stamp(h, 0);
        winutil::set_floated(h, true);
        if self.snapped.contains_key(&key) {
            self.unsnap(h, true);
        }
    }

    /// Put every window of tree `i` in its zone, and record the zones at once so the
    /// screen's handles and a drag see them before the moves land. Only windows not
    /// there yet move, and only on the desktop in front.
    fn tiled_apply(&mut self, i: usize) {
        self.tiling.trees[i].dirty = false;
        self.tiled_sync();
        let t = &self.tiling.trees[i];
        let Some((work, dpi, gap)) = self.tile_area(&t.screen) else { return };
        if t.desktop != self.tiling.desktop {
            return;
        }
        let (screen, rects) = (t.screen.clone(), t.tree.rects(work, gap));
        let tiles: Vec<(usize, isize)> =
            t.tree.tiles.iter().enumerate().filter_map(|(z, x)| Some((z, x.hwnd?))).collect();
        let mut moves = Vec::new();
        for (zone, key) in tiles {
            let target = rects[zone];
            // Maximized by hand: it keeps that, and its tile waits for it.
            if winutil::is_maximized(hwnd_of(key)) {
                continue;
            }
            let there = self
                .snapped
                .get(&key)
                .is_some_and(|s| s.screen == screen && (s.from, s.to) == (zone, zone) && s.at == target);
            if there && self.current(hwnd_of(key)).is_some() {
                continue;
            }
            // Its stack is in the same zone, so the hidden windows aren't taken for strays.
            let members = self.stacks.of(key).map_or_else(|| vec![key], |s| s.members.clone());
            for m in members {
                if let Some(s) = self.snapped.get_mut(&m).filter(|s| s.screen == screen) {
                    (s.from, s.to) = (zone, zone);
                }
            }
            moves.push(Placement { hwnd: key, target, zone: Some((screen.clone(), zone, zone)), dpi, maximize: false });
        }
        self.sync_dividers();
        if !moves.is_empty() {
            self.queued.extend(moves);
            self.window_pump();
        }
    }

    /// A window that can't shrink to a share joins `onto`'s stack, in its zone.
    fn tiled_stack(&mut self, i: usize, key: isize, min: tiling::Min, onto: isize) {
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

    /// Lay tree `i`'s windows out in another family.
    fn tiled_family(&mut self, i: usize, family: Family) {
        let Some((work, _, gap)) = self.tile_area(&self.tiling.trees[i].screen) else { return };
        let t = &mut self.tiling.trees[i];
        t.tree = t.tree.with_family(family, work, gap);
        if t.desktop == self.tiling.desktop {
            self.tiled_apply(i);
        } else {
            t.dirty = true;
        }
    }

    /// The desktop in front changed. The snapping keeps only the records of the
    /// windows on it, and its trees close the gaps left while it was away.
    pub(super) fn tiled_desktop(&mut self, now: String) {
        let first = !self.tiling.known;
        self.tiling.known = true;
        if now == self.tiling.desktop && !first {
            return;
        }
        let before = std::mem::replace(&mut self.tiling.desktop, now.clone());
        for i in 0..self.tiling.trees.len() {
            let t = &self.tiling.trees[i];
            if t.desktop == before && !first {
                let windows: Vec<isize> = t.tree.windows().chain(t.overflow.iter().map(|(k, _)| *k)).collect();
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
    }

    /// After a drag, and once moves are over: who holds each zone of a tiled screen is
    /// what the snapping recorded, so a swap, a split, a stack by S or a drop from
    /// elsewhere re-slots windows by the drag's rules. A tiled window dragged out of the
    /// screen's zones floats (`dragged`), and its tile stays empty.
    pub(super) fn tiled_resync(&mut self, dragged: Option<isize>) {
        if self.tiling.trees.is_empty() && !self.tiling.config.values().any(|c| c.on(&self.tiling.desktop)) {
            return;
        }
        if self.drag.is_some() || self.pending.is_some() || !self.queued.is_empty() {
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
            let n = zones::leaf_count(&node);
            let old = self.tiling.trees[i].tree.clone();
            let mut holders: Vec<Option<isize>> = vec![None; n];
            for (k, s) in &self.snapped {
                let h = hwnd_of(*k);
                let live = unsafe { IsWindow(Some(h)) }.as_bool() && !winutil::is_cloaked(h);
                if s.screen != screen || s.from != s.to || s.from >= n || !live {
                    continue;
                }
                // The window tiling had there stays the tile's; else a stack's top, else any.
                let keep =
                    |h: Option<isize>| h.is_some_and(|h| old.tiles.get(s.from).is_some_and(|t| t.hwnd == Some(h)));
                let top = self.stacks.of(*k).is_none_or(|st| st.top() == *k);
                if holders[s.from].is_none() || (!keep(holders[s.from]) && (keep(Some(*k)) || top)) {
                    holders[s.from] = Some(*k);
                }
            }
            // A tiled window that closed or minimized keeps its tile until the worker says so,
            // which closes the gap.
            let gone = |h: isize| !unsafe { IsWindow(Some(hwnd_of(h))) }.as_bool() || winutil::is_minimized(hwnd_of(h));
            if n == old.tiles.len() {
                for (z, t) in old.tiles.iter().enumerate() {
                    if holders[z].is_none() && t.hwnd.is_some_and(gone) {
                        holders[z] = t.hwnd;
                    }
                }
            }
            let min_of = |k: isize| old.tiles.iter().find(|t| t.hwnd == Some(k)).map_or((0, 0), |t| t.min);
            let tiles: Vec<tiling::Tile> =
                holders.iter().map(|h| tiling::Tile { hwnd: *h, min: h.map_or((0, 0), min_of) }).collect();
            let stacked: HashSet<isize> =
                holders.iter().flatten().filter_map(|h| self.stacks.of(*h)).flat_map(|s| s.members.clone()).collect();
            let family = old.family;
            let tree = if tiles.iter().all(|t| t.hwnd.is_none()) {
                Tree::new(family)
            } else {
                Tree { family, node: Some(node), tiles }
            };
            if tree == old {
                continue;
            }
            changed = true;
            for k in old.windows().filter(|k| tree.zone_of(*k).is_none() && !stacked.contains(k)) {
                winutil::set_tile_stamp(hwnd_of(k), 0);
                if Some(k) == dragged {
                    log::info!("tiling: a tiled window was dragged out of its tile, so it floats");
                    winutil::set_floated(hwnd_of(k), true);
                }
            }
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
        }
        if changed {
            self.tiled_sync();
        }
    }

    fn tiled_dragged(&mut self, screen: &str, tree: &Node) -> bool {
        if !self.tiling_on(screen) {
            return false;
        }
        if let Some(s) = self.screens.iter_mut().find(|s| s.id == screen) {
            s.layout = Some(ZoneLayout { template: Template::Custom, custom: None, tree: tree.clone() });
        }
        let desktop = self.tiling.desktop.clone();
        let i = self.tree_index(screen, &desktop);
        if zones::leaf_count(tree) == self.tiling.trees[i].tree.tiles.len() {
            self.tiling.trees[i].tree.node = Some(tree.clone());
        }
        self.tiled_resync(None);
        self.tiled_sync();
        true
    }

    /// Arrivals that waited for a drag.
    pub(super) fn tiled_after_drag(&mut self, dragged: isize) {
        self.tiled_resync(Some(dragged));
        for a in std::mem::take(&mut self.tiling.waiting) {
            self.tile_arrived(a);
        }
    }

    fn tiling_free(&mut self, screen: &str) -> usize {
        let desktop = self.tiling.desktop.clone();
        let Some(i) = self.tiling.trees.iter().position(|t| t.screen == screen && t.desktop == desktop) else {
            return 0;
        };
        let t = self.tiling.trees.remove(i);
        let dpi = self.tile_area(screen).map_or(96, |(_, dpi, _)| dpi);
        let windows: Vec<isize> = t.tree.windows().chain(t.overflow.iter().map(|(k, _)| *k)).collect();
        let mut moves = Vec::new();
        for k in &windows {
            let h = hwnd_of(*k);
            winutil::set_tile_stamp(h, 0);
            if self.stacks.of(*k).is_some() {
                self.stack_leave(*k);
            }
            let away = !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_minimized(h) || winutil::is_maximized(h);
            if let Some(r) = t.before.get(k).filter(|_| !away) {
                moves.push(Placement { hwnd: *k, target: *r, zone: None, dpi, maximize: false });
            } else if self.snapped.contains_key(k) {
                self.unsnap(h, true);
            }
        }
        log::info!("tiling: freed {} windows", windows.len());
        self.tiled_sync();
        self.queued.extend(moves);
        self.window_pump();
        windows.len()
    }

    fn tiling_float(&mut self, key: isize) -> Result<bool, String> {
        if !self.active() {
            return Err("Zones are off, or Seam is standing aside".into());
        }
        let h = hwnd_of(key);
        if let Some(i) = self.tree_of(key) {
            winutil::set_tile_stamp(h, 0);
            winutil::set_floated(h, true);
            let t = &mut self.tiling.trees[i];
            if t.overflow.iter().any(|(k, _)| *k == key) {
                t.overflow.retain(|(k, _)| *k != key);
            } else if let Some(tree) = t.tree.remove(key) {
                t.tree = tree;
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
        winutil::set_floated(h, false);
        crate::tiler::asked(key);
        Ok(false)
    }
}
