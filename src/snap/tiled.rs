//! Smart tiling on the snapping's engine. A tiled screen shows its tree for the
//! virtual desktop in front as its zones, in memory only, so a drag, the gap handles,
//! S and the drag keys work on it as on any zones. Its own layout stays saved under
//! it and comes back when tiling is off. The tray thread only runs the pure tree step
//! (`seam::tiling`) on what the worker read (`tiler`), and the windows land through
//! the placement queue.
//!
//! It never fights a hand edit: one rule (`tiling::may_move`) runs before every move,
//! so a float, a window moved off its tile without a drag and one a workspace switch
//! placed never move; they float where they are and their neighbours fill the gap, as
//! after a drag out (D562, D564). While Seam stands by, is paused or zones are off,
//! nothing moves; the gaps close once it is back (D566).

use super::*;
use crate::tiler::{Arrival, Event};
use seam::config::TileScreen;
use seam::strip::{Strip, Width};
use seam::tiling::{self, Family, Put, Seen, Tree};
use seam::zones::{LeafTag, Node, Template};
use std::collections::BTreeMap;
use std::time::Duration;

mod arrive;
mod gaps;
mod hand;
mod scroll;
mod turn;

pub use gaps::tiling_switch_ended;
pub use hand::FreeUndo;
pub use scroll::{tiling_column, tiling_display_changed, tiling_scroll, tiling_unpark_for_quit, tiling_unstrand};
pub use turn::Turned;

/// How long Undo of a split waits for its resync before the splits it keeps are dropped.
const UNSPLIT_FOR: Duration = Duration::from_secs(10);

/// One screen's tree on one virtual desktop.
pub(super) struct Tiled {
    screen: String,
    desktop: String,
    tree: Tree,
    /// Windows stacked in a tile because they couldn't shrink to a share, and their
    /// smallest size. They take a tile of their own when room frees up.
    overflow: Vec<(isize, tiling::Min)>,
    /// Windows that left by minimizing, the tile they left empty (`usize::MAX` once it
    /// closed), when, and whether it may count in a burst (not after a switch). They
    /// take it back when restored, or a share.
    resting: Vec<(isize, usize, Instant, bool)>,
    /// Where each window's frame was before tiling took it, for Free all.
    before: HashMap<isize, Rect>,
    /// The snapping's records of its windows while another desktop is in front.
    parked: Vec<(isize, Snapped)>,
    /// A window left while its desktop wasn't in front, or while tiling stood down:
    /// the gap closes once it can.
    dirty: bool,
    /// When a minimized window last came back, which holds the other tiles (D567).
    back: Option<Instant>,
    /// Scrolling only: every column, the view's and the parked ones. `tree` is its view.
    strip: Option<Strip>,
    /// Scrolling only: where each parked window was sent, off every screen.
    offscreen: HashMap<isize, Rect>,
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
    /// Wakes once a switch's grace is over, for its late minimizes (D564).
    grace_timer: usize,
    /// Arrivals and restores that came during a workspace switch, taken once it ends.
    paused: Vec<Arrival>,
    /// Why the Float or tile key's window wasn't tiled, for the tray's notice.
    refused: Option<String>,
    /// Handler timings while logging: batches, total and longest, in microseconds.
    stats: (u64, u128, u128),
    /// A parked window that took the focus while nothing could move, brought into view
    /// once tiling is active again.
    focus_wait: Option<isize>,
    /// Seam is paused, stepped aside or has zones off, so no window stays parked.
    stood_down: bool,
    /// A screen whose split was undone, its splits before it, and when, for the next
    /// resync within `UNSPLIT_FOR`.
    unsplit: Option<(String, Node, Instant)>,
    /// A drop past the strip's column cap floated a window, for the tray's notice.
    capped: bool,
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
                Event::Focused(hwnd) => e.tiled_focused(hwnd),
                Event::Refused(why) => e.tiling.refused = Some(why.to_string()),
                Event::Stale(desktop) => e.tiled_end_desktop(&desktop),
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
/// window it tiled goes back to where it was before. Returns how many, and what its Undo
/// needs.
pub fn tiling_free(screen: &str) -> (usize, Option<FreeUndo>) {
    with_engine(|e| e.tiling_free(screen)).unwrap_or((0, None))
}

/// Whether Undo of Free all can still tile its windows: its desktop is in front, the
/// screen's family is the same, and nothing is tiled there since. Asked before tiling is
/// turned on for it.
pub fn tiling_can_unfree(undo: &FreeUndo) -> bool {
    with_engine(|e| e.free_undoable(undo)).unwrap_or(false)
}

/// The Undo of Free all itself: the windows still where it left them take their tiles
/// back. Returns how many.
pub fn tiling_unfree(undo: FreeUndo) -> usize {
    with_engine(|e| e.tiling_unfree(undo)).unwrap_or(0)
}

/// Rotate windows on a tiled screen: its tiles' windows step to the next tile in zone
/// order, or the previous one, by the one rule.
pub fn tiling_rotate(screen: &str, forward: bool) -> Result<Turned, String> {
    with_engine(|e| e.tiled_rotate(screen, forward)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Reset sizes on a tiled screen: every split even again, and its windows follow.
pub fn tiling_reset(screen: &str) -> Result<Turned, String> {
    with_engine(|e| e.tiled_reset(screen)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Undo of a reset: a tiled screen's splits as they were.
/// False when they aren't the evened ones any more, and nothing changed.
pub fn tiling_restore(screen: &str, before: Node, after: Node) -> bool {
    with_engine(|e| e.tiled_restore(screen, before, after)).unwrap_or(false)
}

/// Undo of a split on a tiled screen: its tree takes back the splits it had before, once
/// Undo's windows land (`tiling_resync`). False when they aren't the split's any more.
pub fn tiling_unsplit(screen: &str, before: Node, after: &Node) -> bool {
    with_engine(|e| e.tiled_unsplit(screen, before, after)).unwrap_or(false)
}

/// The tiled screens follow where the windows are, now or once the moves under way land.
pub fn tiling_resync() {
    with_engine(|e| e.tiled_resync(None));
}

/// Seam is quitting and the user wants the windows back: every tiled screen on every virtual
/// desktop is freed, and the moves are posted. Waits for them to land, but not past `until`
/// (a hung window never holds the quit). Returns the (screen, desktop) pairs freed.
pub fn tiling_free_for_quit(until: Instant) -> Vec<(String, String)> {
    let (landing, freed) = with_engine(|e| e.tiling_free_for_quit(until)).unwrap_or_default();
    let started = Instant::now();
    // A window that can't take its place (a minimum size) stops changing, so it counts as landed,
    // but not before an app has had a moment to start on a move already posted.
    let mut last: Vec<Option<Rect>> = vec![None; landing.len()];
    let mut still = vec![0u8; landing.len()];
    while Instant::now() < until {
        let settling = started.elapsed() < Duration::from_millis(100);
        let mut waiting = settling && !landing.is_empty();
        for (i, (key, target)) in landing.iter().enumerate() {
            let h = hwnd_of(*key);
            let now = winutil::visible_rect(h);
            let gone = !unsafe { IsWindow(Some(h)) }.as_bool();
            still[i] = if now == last[i] { still[i].saturating_add(1) } else { 0 };
            last[i] = now;
            waiting |= !gone && now != Some(*target) && still[i] < 3;
        }
        if !waiting {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    freed
}

/// "Float or tile this window": a tiled window floats and the others close up, and a
/// window on a tiled screen is asked for again. Returns whether it floats now.
pub fn tiling_float(hwnd: HWND) -> Result<bool, String> {
    with_engine(|e| e.tiling_float(hwnd.0 as isize)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

/// Whether a drop past a strip's column cap floated a window since this was last asked.
pub fn take_tiling_capped() -> bool {
    with_engine(|e| std::mem::take(&mut e.tiling.capped)).unwrap_or(false)
}

/// Why the Float or tile key's window wasn't tiled, once.
pub fn take_tiling_refused() -> Option<String> {
    with_engine(|e| e.tiling.refused.take()).flatten()
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
        here.map(|t| TiledInfo { screen: t.screen.clone(), windows: t.all().len() }).collect()
    };
    with_engine(|e| info(e)).unwrap_or_default()
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

    /// Every window it holds: in a tile, a column of its strip, or stacked.
    fn all(&self) -> Vec<isize> {
        let mut all: Vec<isize> = self.tree.windows().collect();
        let columns = self.strip.iter().flat_map(|s| s.windows());
        for k in columns.chain(self.overflow.iter().map(|(k, _)| *k)) {
            if !all.contains(&k) {
                all.push(k);
            }
        }
        all
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
        Some(ZoneLayout { template: Template::Custom, custom: None, from: None, tree })
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
        let resting: HashSet<isize> =
            self.tiling.trees.iter().flat_map(|t| t.resting.iter().map(|(h, ..)| *h)).collect();
        let tiled = self.tiling.trees.iter().flat_map(Tiled::all).chain(resting.iter().copied()).collect();
        crate::tiler::set_tiled(tiled, resting);
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
        self.scroll_focus_waited();
    }

    /// The work area changed under the trees of the desktop in front: each puts its windows in
    /// the zones it has now.
    pub(super) fn tiled_refit(&mut self) {
        for i in 0..self.tiling.trees.len() {
            if self.here(i) {
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

    /// A tree ends: its windows stay where they are, out of tiling and out of zones, except
    /// parked ones, which come back onto the screen.
    fn tiled_release(&mut self, t: Tiled) {
        let screens = self.screens_now();
        self.unpark(&t, None, &[], &screens);
        self.tiling.unsplit.take_if(|(s, ..)| *s == t.screen);
        for k in t.all() {
            winutil::set_tile_stamp(hwnd_of(k), 0);
            if t.desktop == self.tiling.desktop {
                self.unsnap(hwnd_of(k), true);
            } else {
                // Its zones were the tree's, which a re-fit mustn't read as the screen's own.
                winutil::clear_zone_stamp(hwnd_of(k));
            }
        }
        // The unpark dropped moves for its windows; the rest go on.
        self.window_pump();
    }

    /// The tree of a screen on a desktop, made when there is none.
    fn tree_index(&mut self, screen: &str, desktop: &str) -> usize {
        if let Some(i) = self.tiling.trees.iter().position(|t| t.screen == screen && t.desktop == desktop) {
            return i;
        }
        let family = self.tiling.config.get(screen).map(|c| c.family).unwrap_or_default();
        let strip = (family == Family::Scrolling).then(Strip::default);
        self.tiling.trees.push(Tiled {
            screen: screen.to_string(),
            desktop: desktop.to_string(),
            tree: Tree::new(family),
            overflow: Vec::new(),
            resting: Vec::new(),
            before: HashMap::new(),
            parked: Vec::new(),
            dirty: false,
            back: None,
            strip,
            offscreen: HashMap::new(),
        });
        self.tiling.trees.len() - 1
    }

    /// Whether a window is in a tiled tree, on any desktop.
    pub(in crate::snap) fn tiled_window(&self, key: isize) -> bool {
        self.tree_of(key).is_some()
    }

    /// The tree a window is tiled in, in a tile or stacked in one.
    fn tree_of(&self, key: isize) -> Option<usize> {
        let has = |t: &Tiled| {
            t.tree.zone_of(key).is_some()
                || t.overflow.iter().any(|(k, _)| *k == key)
                || t.strip.as_ref().is_some_and(|s| s.has(key))
        };
        self.tiling.trees.iter().position(has)
    }

    /// A tiled screen's work area, DPI and gap at its scale.
    fn tile_area(&self, screen: &str) -> Option<(Rect, u32, i32)> {
        let s = self.screens.iter().find(|s| s.id == screen)?;
        let (work, dpi) = work_area(s.bounds)?;
        let gap = (self.settings.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        Some((work, dpi, gap))
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

    /// What tiling knows of a tiled window before it would move it. One a stack hides
    /// is only ever gone or away.
    fn tile_seen(&self, key: isize, hidden: &[isize]) -> Seen {
        let h = hwnd_of(key);
        let shown = !hidden.contains(&key);
        Seen {
            floated: shown && winutil::is_floated(h),
            moved: shown && !self.moving(key) && (self.tiled_moved(key) || self.parked_moved(key)),
            away: winutil::is_minimized(h) || winutil::is_maximized(h) || winutil::is_cloaked(h),
            gone: gone(key),
        }
    }

    /// Before anything moves on the desktop in front, by `tiling::plan_apply`: a window
    /// that closed or went to another desktop leaves tree `i`; one someone else placed,
    /// by hand or by its app (the desk gate's "moved after hand placement"), floats
    /// where it is. Either way the others close up. The windows that may move then;
    /// `tiled_apply` moves them.
    fn tiled_check(&mut self, i: usize) -> Vec<isize> {
        if !self.here(i) {
            return Vec::new();
        }
        // Scrolling: a drag's changes to the view go into the strip, and the rule looks at
        // every column, parked ones too.
        self.scroll_absorb(i);
        let hidden = self.stack_hidden();
        let t = &self.tiling.trees[i];
        let looked = t.strip.as_ref().map_or_else(|| t.tree.clone(), Strip::whole);
        let plan = {
            let seen = |k: isize| self.tile_seen(k, &hidden);
            tiling::plan_apply(&looked, seen, |k| self.heir_of(i, k))
        };
        for &k in &plan.left {
            log::info!("tiling: a tiled window closed or went to another desktop unseen, so it leaves");
            self.scroll_let_go(i, k);
            self.tiling.trees[i].before.remove(&k);
            self.tiling.trees[i].offscreen.remove(&k);
            winutil::set_tile_stamp(hwnd_of(k), 0);
            self.snapped.remove(&k);
        }
        let t = &mut self.tiling.trees[i];
        for (_, heir) in &plan.heirs {
            t.overflow.retain(|(k, _)| k != heir);
        }
        if let Some(strip) = &t.strip {
            let strip = strip.keep(&plan.tree.tiles, &plan.heirs);
            t.tree = strip.tree();
            t.strip = Some(strip);
        } else {
            t.tree = plan.tree;
            for &zone in &plan.closed {
                t.closed_zone(zone);
            }
        }
        for &k in &plan.floated {
            log::info!("tiling desk gate: a tiled window was placed by someone else since, so it floats where it is");
            self.floated_out(i, k);
        }
        let mut moves = plan.moves;
        if !plan.floated.is_empty() {
            let before: Vec<isize> = self.tiling.trees[i].tree.windows().collect();
            self.tiled_unstack(i);
            // A stacked window that took a share now is placed too, by the same rule.
            let fresh: Vec<isize> = self.tiling.trees[i].tree.windows().filter(|k| !before.contains(k)).collect();
            moves.extend(fresh.into_iter().filter(|k| tiling::may_move(&self.tile_seen(*k, &hidden))));
        }
        moves
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

    /// A window out of tree `i`, its tile gone to a window stacked in it or closed so the
    /// neighbours fill the gap, floats where it is (D562).
    fn floated_out(&mut self, i: usize, key: isize) {
        // A parked one that floats off every screen comes back onto one first.
        self.scroll_let_go(i, key);
        let t = &mut self.tiling.trees[i];
        t.overflow.retain(|(k, _)| *k != key);
        t.before.remove(&key);
        t.offscreen.remove(&key);
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
        // The one rule before any move: whoever else placed a window since, it floats.
        let movable = self.tiled_check(i);
        self.tiling.trees[i].dirty = false;
        self.tiled_sync();
        let t = &self.tiling.trees[i];
        let Some((work, dpi, gap)) = self.tile_area(&t.screen) else { return };
        let (screen, rects) = (t.screen.clone(), t.tree.rects(work, gap));
        let tiles: Vec<(usize, isize)> = t
            .tree
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(z, x)| Some((z, x.hwnd?)))
            .filter(|(_, k)| movable.contains(k))
            .collect();
        let mut moves = Vec::new();
        for (zone, key) in tiles {
            let (target, h) = (rects[zone], hwnd_of(key));
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
        // Scrolling: the columns out of view park first, so none is ever over another.
        let parks = self.scroll_parks(i, &movable);
        self.sync_dividers();
        if !moves.is_empty() || !parks.is_empty() {
            self.queued.extend(parks);
            self.queued.extend(moves);
            self.window_pump();
        }
    }

    /// Lay tree `i`'s windows out in another family. A window moved by hand floats
    /// first, so the new shape never pulls it back.
    fn tiled_family(&mut self, i: usize, family: Family) {
        let Some((work, _, gap)) = self.tile_area(&self.tiling.trees[i].screen) else { return };
        self.tiled_check(i);
        if family == Family::Scrolling || self.tiling.trees[i].strip.is_some() {
            return self.scroll_family(i, family);
        }
        let t = &mut self.tiling.trees[i];
        t.tree = t.tree.with_family(family, work, gap);
        // The tiles minimized windows left are gone in the new shape.
        for r in &mut t.resting {
            r.1 = usize::MAX;
        }
        self.tiled_apply(i);
    }
}
