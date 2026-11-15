//! Smart tiling on the snapping's engine. A tiled screen shows its tree for the
//! virtual desktop in front as its zones, in memory only, so a drag, the gap handles,
//! S and the drag keys work on it as on any zones. Its own layout stays saved under
//! it and comes back when tiling is off. The tray thread only runs the pure tree step
//! (`seam::tiling`) on what the worker read (`tiler`), and the windows land through
//! the placement queue.
//!
//! It never fights a hand edit: a window that isn't where tiling put it floats before
//! anything would move it back, and a window dragged out of its tile floats and its
//! neighbours fill the gap (D562). While Seam stands by, is paused or zones are off,
//! nothing moves; the gaps close once it is back (D566).

use super::*;
use crate::tiler::{Arrival, Event};
use seam::config::TileScreen;
use seam::tiling::{self, Family, Put, Tree};
use seam::zones::{LeafTag, Node, Template};
use std::collections::BTreeMap;
use std::time::Duration;

mod hand;

/// A minimized window's tile waits this long for it, so Win+D or minimize all keeps
/// the shape for the windows to come back to (D567).
const GAP_AFTER: Duration = Duration::from_millis(600);

/// One screen's tree on one virtual desktop.
pub(super) struct Tiled {
    screen: String,
    desktop: String,
    tree: Tree,
    /// Windows stacked in a tile because they couldn't shrink to a share, and their
    /// smallest size. They take a tile of their own when room frees up.
    overflow: Vec<(isize, tiling::Min)>,
    /// Windows that left by minimizing, the tile they left empty (`usize::MAX` once it
    /// closed), and when. They take it back when restored, or a share.
    resting: Vec<(isize, usize, Instant)>,
    /// Where each window's frame was before tiling took it, for Free all.
    before: HashMap<isize, Rect>,
    /// The snapping's records of its windows while another desktop is in front.
    parked: Vec<(isize, Snapped)>,
    /// A window left while its desktop wasn't in front, or while tiling stood down:
    /// the gap closes once it can.
    dirty: bool,
}

#[derive(Default)]
pub(super) struct Tiling {
    /// This desk's settings, by screen.
    config: BTreeMap<String, TileScreen>,
    trees: Vec<Tiled>,
    /// The virtual desktop in front, as the worker last said.
    desktop: String,
    /// The worker has confirmed the desktop in front. Not while it can't tell, as on an
    /// empty desktop, when nothing is tiled and nothing is taken for gone.
    known: bool,
    /// Some desktop was said this run.
    seen: bool,
    /// The last `Seam_tile` order given.
    seq: u32,
    /// Windows that arrived during a drag, taken once it is over.
    waiting: Vec<Arrival>,
    /// The worker was asked for the windows a tray that ran before tiled.
    looked: bool,
    /// Wakes to close the tiles minimized windows left (`GAP_AFTER`).
    gap_timer: usize,
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

/// The desktop in front, once the worker has confirmed it.
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

/// The timer repeats until the engine is free to take it, so a busy moment drops nothing.
unsafe extern "system" fn on_gaps(_: HWND, _: u32, id: usize, _: u32) {
    let taken = with_engine(|e| {
        if e.tiling.gap_timer == id {
            e.tiling.gap_timer = 0;
            e.tiled_gaps();
        }
    });
    if taken.is_some() {
        unsafe {
            let _ = KillTimer(None, id);
        }
    }
}

impl Tiled {
    /// Zone `zone` went: the minimized windows' tiles after it move down one.
    fn closed_zone(&mut self, zone: usize) {
        for r in &mut self.resting {
            r.1 = match r.1 {
                z if z == zone => usize::MAX,
                z if z > zone && z != usize::MAX => z - 1,
                z => z,
            };
        }
    }

    /// `key` leaves its tile and the others fill the gap.
    fn remove(&mut self, key: isize) -> bool {
        let Some(zone) = self.tree.zone_of(key) else { return false };
        let Some(tree) = self.tree.remove(key) else { return false };
        self.tree = tree;
        self.closed_zone(zone);
        true
    }
}

/// Whether a window is gone from where its tree is: closed, or moved to another desktop.
fn gone(key: isize) -> bool {
    let h = hwnd_of(key);
    !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_cloaked(h)
}

impl Engine {
    /// Tiling is on for this screen on the desktop in front.
    pub(super) fn tiling_on(&self, screen: &str) -> bool {
        self.tiling.known && self.tiling.config.get(screen).is_some_and(|t| t.on(&self.tiling.desktop))
    }

    /// Tree `i` is on the desktop in front, as the worker confirmed it.
    fn here(&self, i: usize) -> bool {
        self.tiling.known && self.tiling.trees[i].desktop == self.tiling.desktop
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
            let resting = t.resting.iter().map(|(h, ..)| *h);
            t.tree.windows().chain(stacked).chain(resting).collect::<Vec<_>>()
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
        let was_on = |s: &String| before.get(s).is_some_and(|b| b.on(&self.tiling.desktop));
        let fresh: Vec<String> =
            self.tiling.config.keys().filter(|s| !was_on(s) && self.tiling_on(s)).cloned().collect();
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

    /// Tiling may move windows again, after zones were off, a pause or a full-screen
    /// app: the trees of the desktop in front close the gaps left meanwhile.
    pub(super) fn tiled_resume(&mut self) {
        if !self.active() {
            return;
        }
        for i in 0..self.tiling.trees.len() {
            if self.here(i) && self.tiling.trees[i].dirty {
                self.tiled_check(i);
                self.tiled_apply(i);
            }
        }
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

    /// A window opened, was restored or asked for: it takes back the tile it left by
    /// minimizing, or a share of the largest tile, or a stack in it when it can't shrink
    /// to that. Waits for a drag to end; never during a workspace switch (D564).
    fn tile_arrived(&mut self, a: Arrival) {
        if !self.active() || crate::switcher::switching() {
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
        let Some(screen) = self.screens.iter().find(|s| s.bounds.contains(x, y)).map(|s| s.id.clone()) else { return };
        if !self.tiling_on(&screen) {
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
            let (_, zone, _) = x.resting.iter().find(|(k, ..)| *k == a.hwnd)?;
            Some((t, *zone))
        });
        if a.restored && rested.is_none() {
            return;
        }
        for t in &mut self.tiling.trees {
            t.resting.retain(|(k, ..)| *k != a.hwnd);
        }
        let Some((work, _, gap)) = self.tile_area(&screen) else { return };
        self.tiled_check(i);
        let tree = &self.tiling.trees[i].tree;
        let back = rested.filter(|(t, _)| *t == i).and_then(|(_, zone)| tree.back_to(zone, a.hwnd, a.min));
        let placed = match back {
            Some(tree) => Some((tree.clone(), Put::Tile(tree.zone_of(a.hwnd).unwrap_or(0)))),
            None => tree.insert(work, gap, a.hwnd, a.min),
        };
        let Some((tree, put)) = placed else { return };
        let order = if a.order > 0 { a.order } else { self.tiling.seq + 1 };
        self.tiling.seq = self.tiling.seq.max(order);
        winutil::set_tile_stamp(h, order);
        winutil::set_floated(h, false);
        self.tiling.trees[i].before.entry(a.hwnd).or_insert(a.frame);
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
    fn tile_left(&mut self, key: isize, minimized: bool) {
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
            t.resting.push((key, zone, Instant::now()));
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
    fn tiled_heir(&mut self, i: usize, key: isize) -> bool {
        let Some(zone) = self.tiling.trees[i].tree.zone_of(key) else { return false };
        let live = |m: &isize| {
            *m != key && unsafe { IsWindow(Some(hwnd_of(*m))) }.as_bool() && !winutil::is_minimized(hwnd_of(*m))
        };
        let screen = &self.tiling.trees[i].screen;
        let same_zone = |k: &isize| self.snapped.get(k).is_some_and(|s| s.screen == *screen && s.from == zone);
        let stacked: Vec<isize> = self.tiling.trees[i].overflow.iter().map(|(k, _)| *k).filter(same_zone).collect();
        let heir = self.stacks.of(key).and_then(|s| s.members.iter().copied().find(live));
        let Some(heir) = heir.or_else(|| stacked.into_iter().find(live)) else { return false };
        let t = &mut self.tiling.trees[i];
        let min = t.overflow.iter().find(|(k, _)| *k == heir).map_or((0, 0), |(_, m)| *m);
        t.overflow.retain(|(k, _)| *k != heir);
        t.tree.tiles[zone] = tiling::Tile { hwnd: Some(heir), min };
        true
    }

    /// Wake once a minimized window's tile has waited `GAP_AFTER`.
    fn tiled_arm_gaps(&mut self) {
        let waiting = self.tiling.trees.iter().any(|t| t.resting.iter().any(|(_, z, _)| *z != usize::MAX));
        if waiting && self.tiling.gap_timer == 0 {
            let ms = GAP_AFTER.as_millis() as u32;
            self.tiling.gap_timer = unsafe { SetTimer(None, 0, ms, Some(on_gaps)) };
        }
    }

    /// The tiles minimized windows left close once they have waited, so the others
    /// fill the gap, except where every window of the tree is minimized, as after
    /// Win+D: there the shape waits for them to come back.
    fn tiled_gaps(&mut self) {
        let now = Instant::now();
        for i in 0..self.tiling.trees.len() {
            let t = &self.tiling.trees[i];
            if t.tree.windows().next().is_none() {
                continue;
            }
            let mut due: Vec<usize> = t
                .resting
                .iter()
                .filter(|(_, z, at)| *z != usize::MAX && now.duration_since(*at) >= GAP_AFTER)
                .map(|(_, z, _)| *z)
                .filter(|z| t.tree.tiles.get(*z).is_some_and(|x| x.hwnd.is_none()))
                .collect();
            if due.is_empty() {
                continue;
            }
            due.sort_unstable_by(|a, b| b.cmp(a));
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

    /// Before anything moves on the desktop in front: a window that closed or went to
    /// another desktop leaves tree `i` and the others close up; one that isn't where
    /// tiling put it, as the desk gate's "moved after hand placement" says, by hand, by
    /// its app or by a workspace switch, floats where it is, and its tile stays empty so
    /// nothing moves it back. Only moves within the tree follow, done by the caller.
    fn tiled_check(&mut self, i: usize) {
        if !self.here(i) {
            return;
        }
        let hidden = self.stack_hidden();
        let windows: Vec<isize> = self.tiling.trees[i].tree.windows().collect();
        let left: Vec<isize> = windows.iter().copied().filter(|k| gone(*k)).collect();
        for k in left {
            log::info!("tiling: a tiled window closed or went to another desktop unseen, so it leaves");
            let t = &mut self.tiling.trees[i];
            t.before.remove(&k);
            t.remove(k);
            winutil::set_tile_stamp(hwnd_of(k), 0);
            self.snapped.remove(&k);
        }
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
        let away = winutil::is_minimized(h) || winutil::is_maximized(h);
        let (Some(s), Some(now), false) = (self.snapped.get(&key), winutil::visible_rect(h), away) else {
            return false;
        };
        tiling::moved(now, s.at, winutil::is_resizable(h))
    }

    /// A window leaves its tile, which stays empty or goes to a window stacked in it, and
    /// floats where it is.
    fn tiled_float(&mut self, i: usize, key: isize) {
        if !self.tiled_heir(i, key) {
            let t = &mut self.tiling.trees[i];
            if let Some(tree) = t.tree.vacate(key) {
                t.tree = tree;
            }
        }
        let t = &mut self.tiling.trees[i];
        t.overflow.retain(|(k, _)| *k != key);
        t.before.remove(&key);
        let h = hwnd_of(key);
        winutil::set_tile_stamp(h, 0);
        winutil::set_floated(h, true);
        if self.snapped.contains_key(&key) {
            self.unsnap(h, true);
        }
    }

    /// Put every window of tree `i` in its zone, and record the zones at once so the
    /// screen's handles and a drag see them before the moves land. Only windows not
    /// there yet move, only on the desktop in front, and only while tiling is active;
    /// otherwise it waits (`dirty`).
    fn tiled_apply(&mut self, i: usize) {
        if !self.here(i) || !self.active() {
            self.tiling.trees[i].dirty = true;
            self.tiled_sync();
            return;
        }
        self.tiling.trees[i].dirty = false;
        self.tiled_sync();
        let t = &self.tiling.trees[i];
        let Some((work, dpi, gap)) = self.tile_area(&t.screen) else { return };
        let (screen, rects) = (t.screen.clone(), t.tree.rects(work, gap));
        let tiles: Vec<(usize, isize)> =
            t.tree.tiles.iter().enumerate().filter_map(|(z, x)| Some((z, x.hwnd?))).collect();
        let mut moves = Vec::new();
        for (zone, key) in tiles {
            let (target, h) = (rects[zone], hwnd_of(key));
            // Maximized by hand, or on another desktop now: it is left as it is.
            if winutil::is_maximized(h) || winutil::is_cloaked(h) {
                continue;
            }
            let there = self
                .snapped
                .get(&key)
                .is_some_and(|s| s.screen == screen && (s.from, s.to) == (zone, zone) && s.at == target);
            if there && self.current(h).is_some() {
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

    /// Lay tree `i`'s windows out in another family. A window moved by hand floats
    /// first, so the new shape never pulls it back.
    fn tiled_family(&mut self, i: usize, family: Family) {
        let Some((work, _, gap)) = self.tile_area(&self.tiling.trees[i].screen) else { return };
        self.tiled_check(i);
        let t = &mut self.tiling.trees[i];
        t.tree = t.tree.with_family(family, work, gap);
        // The tiles minimized windows left are gone in the new shape.
        for r in &mut t.resting {
            r.1 = usize::MAX;
        }
        self.tiled_apply(i);
    }
}
