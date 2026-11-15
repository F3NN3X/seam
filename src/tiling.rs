//! Smart tiling: the windows tiled on one screen of one virtual desktop, as a zone
//! tree (`zones::Node`) and the window in each zone. Pure, so every family is
//! tested here, and the snapping only carries out what it decides. Smart splits
//! the largest tile along its longer side for a new window (dwindle by size);
//! Columns, Main + stack and Grid lay every window out again by count.

use crate::zones::{self, Dir, LeafTag, Node, Rect};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// How a tiled screen lays its windows out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    #[default]
    Smart,
    Columns,
    MainStack,
    Grid,
}

impl Family {
    pub const ALL: [Family; 4] = [Family::Smart, Family::Columns, Family::MainStack, Family::Grid];

    pub fn title(self) -> &'static str {
        match self {
            Family::Smart => "Smart",
            Family::Columns => "Columns",
            Family::MainStack => "Main + stack",
            Family::Grid => "Grid",
        }
    }

    /// Its name in the config and the API.
    pub fn name(self) -> &'static str {
        match self {
            Family::Smart => "smart",
            Family::Columns => "columns",
            Family::MainStack => "main_stack",
            Family::Grid => "grid",
        }
    }

    pub fn parse(name: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.name() == name)
    }
}

/// Most tiles a screen gets. A window past them joins a stack, and the tree stays
/// inside `zones::MAX_DEPTH` whatever the family.
pub const MAX_TILES: usize = 12;
/// The main tile's share in Main + stack.
const MAIN_SHARE: f32 = 0.6;

/// A window's smallest visible frame, in pixels.
pub type Min = (i32, i32);

/// One zone of the tree. A tile whose window left by hand stays empty, so nothing
/// else moves for it, and the next window takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub hwnd: Option<isize>,
    pub min: Min,
}

/// Where a new window went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Put {
    /// Its own tile, by zone.
    Tile(usize),
    /// No share is big enough for it, or the screen is full: it joins a stack on this window.
    Stack(isize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub family: Family,
    /// None while nothing is tiled.
    pub node: Option<Node>,
    /// The tiles in zone order: zone i is `tiles[i]`.
    pub tiles: Vec<Tile>,
}

fn leaf() -> Node {
    Node::Leaf(LeafTag::Leaf)
}

fn split(dir: Dir, ratio: f32, a: Node, b: Node) -> Node {
    Node::Split { split: dir, ratio, a: Box::new(a), b: Box::new(b) }
}

/// `n` equal parts side by side (`Vertical`) or one above the other.
fn even(dir: Dir, n: usize) -> Node {
    match n {
        0 | 1 => leaf(),
        _ => split(dir, 1.0 / n as f32, leaf(), even(dir, n - 1)),
    }
}

/// Columns of rows, as square as `n` allows, filled column by column.
fn grid(n: usize) -> Node {
    let cols = (1..=n).find(|c| c * c >= n).unwrap_or(1);
    let rows: Vec<usize> = (0..cols).map(|c| n / cols + usize::from(c < n % cols)).collect();
    fn chain(rows: &[usize]) -> Node {
        match rows {
            [] => leaf(),
            [one] => even(Dir::Horizontal, *one),
            [first, rest @ ..] => {
                split(Dir::Vertical, 1.0 / rows.len() as f32, even(Dir::Horizontal, *first), chain(rest))
            }
        }
    }
    chain(&rows)
}

/// A fixed family's tree for `n` windows. Main + stack keeps the main share a handle
/// gave `before`; Columns and Grid are even by count, which is what they promise (D565).
fn fixed(family: Family, n: usize, before: Option<&Node>) -> Node {
    let main = match before {
        Some(Node::Split { split: Dir::Vertical, ratio, .. }) => *ratio,
        _ => MAIN_SHARE,
    };
    match family {
        Family::Columns | Family::Smart => even(Dir::Vertical, n),
        Family::MainStack if n > 1 => split(Dir::Vertical, main, leaf(), even(Dir::Horizontal, n - 1)),
        Family::MainStack => leaf(),
        Family::Grid => grid(n),
    }
}

/// What tiling knows of a tiled window just before it would move it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// Floated by hand: the Float key or a drag out.
    pub floated: bool,
    /// Off its tile since tiling put it there, without a drag: Win+arrow, the keyboard,
    /// its app or a workspace switch that moved it (`moved`). A capture or Quick save,
    /// which only stamps it, doesn't count.
    pub moved: bool,
    /// Maximized, minimized or on another desktop: left as it is, still tiled.
    pub away: bool,
    /// Closed, or moved to another desktop: it leaves the tree, and doesn't float.
    pub gone: bool,
}

/// The one rule before any move (D562, D564): tiling moves a window only when nobody
/// else placed it since. A float and a window moved off its tile never move; one away
/// or gone is left as it is.
pub fn may_move(s: &Seen) -> bool {
    !(s.floated || s.moved || s.away || s.gone)
}

/// A window that may not move because someone else placed it leaves the tree and
/// floats, and its neighbours fill the gap. One only away stays.
pub fn floats(s: &Seen) -> bool {
    (s.floated || s.moved) && !s.gone
}

/// The windows of `tree` tiling may move now, in zone order.
pub fn to_move(tree: &Tree, seen: impl Fn(isize) -> Seen) -> Vec<isize> {
    tree.windows().filter(|h| may_move(&seen(*h))).collect()
}

/// What tiling does with a tree just before it moves windows: the tree after, the
/// windows that left (gone) or float, each float a window stacked in its tile took
/// over from, the zones that closed in turn, and the windows that may move then. The snapping's `tiled_apply` runs
/// exactly this.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub tree: Tree,
    pub left: Vec<isize>,
    pub floated: Vec<isize>,
    /// (the float, the window that takes its tile).
    pub heirs: Vec<(isize, isize)>,
    /// Each zone that closed, as it closed, for the tiles that count zones.
    pub closed: Vec<usize>,
    pub moves: Vec<isize>,
}

/// Plan a move of `tree` by the one rule. A window gone leaves and the others close
/// up; a float leaves its tile to the window stacked in it (`heir`), or the others
/// close up; then the windows that may move are the ones to place.
pub fn plan_apply(tree: &Tree, seen: impl Fn(isize) -> Seen, heir: impl Fn(isize) -> Option<(isize, Min)>) -> Plan {
    let mut plan = Plan {
        tree: tree.clone(),
        left: Vec::new(),
        floated: Vec::new(),
        heirs: Vec::new(),
        closed: Vec::new(),
        moves: Vec::new(),
    };
    for h in tree.windows() {
        let s = seen(h);
        if s.gone {
            plan.left.push(h);
        } else if floats(&s) {
            plan.floated.push(h);
        }
    }
    let close = |plan: &mut Plan, h: isize| {
        if let (Some(z), Some(less)) = (plan.tree.zone_of(h), plan.tree.remove(h)) {
            plan.tree = less;
            plan.closed.push(z);
        }
    };
    for h in plan.left.clone() {
        close(&mut plan, h);
    }
    for h in plan.floated.clone() {
        let zone = plan.tree.zone_of(h);
        match (zone, heir(h).filter(|(k, _)| plan.tree.zone_of(*k).is_none())) {
            (Some(z), Some((k, min))) => {
                plan.tree.tiles[z] = Tile { hwnd: Some(k), min };
                plan.heirs.push((h, k));
            }
            _ => close(&mut plan, h),
        }
    }
    plan.moves = to_move(&plan.tree, &seen);
    plan
}

/// Minimized windows this close together came as one minimize all, such as Win+D.
pub const BURST: Duration = Duration::from_millis(250);
/// A burst keeps its tiles this long with no restore, then they close too (D567).
pub const BURST_KEEPS: Duration = Duration::from_secs(30);
/// A minimized window's tile waits this long, from its minimize or the tree's last
/// restore, whichever is later, before the others fill it (D567).
pub const GAP_AFTER: Duration = Duration::from_millis(600);

/// A minimized window's tile as `gaps_due` reads it: its zone, when it was minimized,
/// and whether it may count in a burst. One a workspace switch minimized never does.
pub type Resting = (usize, Instant, bool);

/// Which tiles of minimized windows close now. One that came in a burst with another
/// keeps its tile until it is restored or closed, so Win+D then a restore is exact
/// however long the restores take, or until `BURST_KEEPS` passes with no restore.
/// Another closes once `GAP_AFTER` has passed since its minimize and since the last
/// restore (`back`), so a run of restores is never cut short. Highest zone first, for
/// `remove_zone`.
pub fn gaps_due(resting: &[Resting], back: Option<Instant>, now: Instant) -> Vec<usize> {
    let near = |a: Instant, b: Instant| a.max(b).duration_since(a.min(b)) <= BURST;
    let burst = |i: usize, at: Instant, may: bool| {
        may && resting.iter().enumerate().any(|(j, (_, other, counts))| j != i && *counts && near(at, *other))
    };
    let mut due: Vec<usize> = resting
        .iter()
        .enumerate()
        .filter(|(i, (_, at, may))| {
            let wait = if burst(*i, *at, *may) { BURST_KEEPS } else { GAP_AFTER };
            now.duration_since(back.map_or(*at, |b| b.max(*at))) >= wait
        })
        .map(|(_, (zone, ..))| *zone)
        .collect();
    due.sort_unstable_by(|a, b| b.cmp(a));
    due.dedup();
    due
}

/// The arrivals kept during a pause (a workspace switch) to take once it ends, by
/// window: the latest of each, in the order they came, without the windows the switch
/// itself placed or moved.
pub fn after_pause<T>(pending: Vec<(isize, T)>, by_switch: impl Fn(isize) -> bool) -> Vec<(isize, T)> {
    let mut kept: Vec<(isize, T)> = Vec::new();
    for (h, item) in pending {
        kept.retain(|(k, _)| *k != h);
        kept.push((h, item));
    }
    kept.retain(|(h, _)| !by_switch(*h));
    kept
}

/// Who holds each of `n` zones after a drag, from the windows the snapping recorded
/// there as (zone, window, on top of its stack). Where several are, the window tiling
/// had there (`old`) wins, then a stack's top, then the lowest handle, so the choice
/// never depends on the order they came in. A zone none holds keeps `kept`, a
/// minimized window waiting for it, or is empty.
pub fn holders(
    n: usize,
    old: &[Option<isize>],
    kept: &[Option<isize>],
    held: &[(usize, isize, bool)],
) -> Vec<Option<isize>> {
    (0..n)
        .map(|z| {
            let here = held.iter().filter(|(at, ..)| *at == z);
            let rank =
                |&&(_, h, top): &&(usize, isize, bool)| (old.get(z) == Some(&Some(h)), top, std::cmp::Reverse(h));
            here.max_by_key(rank).map(|(_, h, _)| *h).or_else(|| kept.get(z).copied().flatten())
        })
        .collect()
}

/// A tiled window this far off its tile, or this much bigger, was moved since. One that
/// came out a little smaller, as a terminal that sizes itself by whole characters does,
/// wasn't, and neither was one that can't be resized.
pub const MOVED_PX: i32 = 4;
pub const SMALLER_PX: i32 = 48;

/// Whether a window whose frame is `now` was moved or resized off the tile `at`.
pub fn moved(now: Rect, at: Rect, resizable: bool) -> bool {
    let off = (now.x - at.x).abs() > MOVED_PX || (now.y - at.y).abs() > MOVED_PX;
    let bigger = now.w > at.w + MOVED_PX || now.h > at.h + MOVED_PX;
    let smaller = resizable && (now.w < at.w - SMALLER_PX || now.h < at.h - SMALLER_PX);
    off || bigger || smaller
}

fn area_of(r: &Rect) -> i64 {
    i64::from(r.w) * i64::from(r.h)
}

impl Tree {
    pub fn new(family: Family) -> Tree {
        Tree { family, node: None, tiles: Vec::new() }
    }

    /// The tree as zones: one zone, the whole screen, while nothing is tiled.
    pub fn layout(&self) -> Node {
        self.node.clone().unwrap_or_else(leaf)
    }

    /// The tiled windows, in zone order.
    pub fn windows(&self) -> impl Iterator<Item = isize> + '_ {
        self.tiles.iter().filter_map(|t| t.hwnd)
    }

    /// The zone a window holds.
    pub fn zone_of(&self, hwnd: isize) -> Option<usize> {
        self.tiles.iter().position(|t| t.hwnd == Some(hwnd))
    }

    /// Each zone's rect inside `area`, with `gap` as the zones have it.
    pub fn rects(&self, area: Rect, gap: i32) -> Vec<Rect> {
        zones::rects(&self.layout(), area, gap)
    }

    /// Every window's share is at least its smallest size.
    fn fits(&self, area: Rect, gap: i32) -> bool {
        let rects = self.rects(area, gap);
        self.tiles.iter().zip(&rects).all(|(t, r)| t.hwnd.is_none() || (r.w >= t.min.0 && r.h >= t.min.1))
    }

    /// The window of the largest tile that has one: where a window that fits nowhere stacks.
    fn largest_window(&self, area: Rect) -> Option<isize> {
        let rects = self.rects(area, 0);
        let filled = self.tiles.iter().zip(&rects).filter_map(|(t, r)| Some((t.hwnd?, area_of(r))));
        // The first of equals, so the choice doesn't depend on the order of a sort.
        filled
            .fold(None, |best: Option<(isize, i64)>, (h, a)| match best {
                Some((_, b)) if b >= a => best,
                _ => Some((h, a)),
            })
            .map(|(h, _)| h)
    }

    /// Where `hwnd` goes, and the tree after. The largest empty tile it fits first;
    /// then Smart splits the largest tile along its longer side and gives it the
    /// second half, and a fixed family lays every window out again with it last. When
    /// that leaves any window below its smallest size, or the screen has `MAX_TILES`,
    /// it stacks on the largest tile's window. None when it is tiled already, or
    /// nothing is there to stack on.
    pub fn insert(&self, area: Rect, gap: i32, hwnd: isize, min: Min) -> Option<(Tree, Put)> {
        if self.zone_of(hwnd).is_some() {
            return None;
        }
        let rects = self.rects(area, gap);
        let empty = self
            .tiles
            .iter()
            .zip(&rects)
            .enumerate()
            .filter(|(_, (t, r))| t.hwnd.is_none() && r.w >= min.0 && r.h >= min.1);
        if let Some((i, _)) = empty.max_by_key(|(i, (_, r))| (area_of(r), std::cmp::Reverse(*i))) {
            let mut tree = self.clone();
            tree.tiles[i] = Tile { hwnd: Some(hwnd), min };
            return Some((tree, Put::Tile(i)));
        }
        let tile = Tile { hwnd: Some(hwnd), min };
        let grown = match (&self.node, self.family) {
            (None, _) => Some(Tree { family: self.family, node: Some(leaf()), tiles: vec![tile] }),
            _ if self.tiles.len() >= MAX_TILES => None,
            (Some(node), Family::Smart) => {
                let cells = zones::rects(node, area, 0);
                let at = (0..cells.len())
                    .fold(0, |best, i| if area_of(&cells[i]) > area_of(&cells[best]) { i } else { best });
                let dir = if cells[at].w >= cells[at].h { Dir::Vertical } else { Dir::Horizontal };
                zones::split_leaf(node, at, dir).map(|node| {
                    let mut tiles = self.tiles.clone();
                    tiles.insert(at + 1, tile);
                    Tree { family: self.family, node: Some(node), tiles }
                })
            }
            (Some(_), family) => {
                let mut tiles = self.tiles.clone();
                tiles.push(tile);
                Some(Tree { family, node: Some(fixed(family, tiles.len(), self.node.as_ref())), tiles })
            }
        };
        // The first window takes the screen whatever its size.
        match grown.filter(|t| self.node.is_none() || t.fits(area, gap)) {
            Some(tree) => {
                let at = tree.zone_of(hwnd)?;
                Some((tree, Put::Tile(at)))
            }
            None => self.largest_window(area).map(|onto| (self.clone(), Put::Stack(onto))),
        }
    }

    /// `hwnd` closed or was minimized: its tile goes and the others fill the gap,
    /// in Smart as the zones' own removal does (its sibling takes the space), in a
    /// fixed family by laying the rest out again. None when it isn't tiled here.
    pub fn remove(&self, hwnd: isize) -> Option<Tree> {
        self.remove_zone(self.zone_of(hwnd)?)
    }

    /// Zone `at` goes, whoever holds it, and the others fill the gap as `remove` says.
    /// A tree left with no window is empty.
    pub fn remove_zone(&self, at: usize) -> Option<Tree> {
        let mut tiles = self.tiles.clone();
        if at >= tiles.len() {
            return None;
        }
        tiles.remove(at);
        if tiles.iter().all(|t| t.hwnd.is_none()) {
            return Some(Tree::new(self.family));
        }
        let node = match self.family {
            Family::Smart => zones::remove_leaf(self.node.as_ref()?, at)?,
            family => fixed(family, tiles.len(), self.node.as_ref()),
        };
        Some(Tree { family: self.family, node: Some(node), tiles })
    }

    /// `hwnd` left its tile and the tile stays, empty, so nothing moves for it: a window
    /// minimized, which may come back to it, or moved without a drag. Every tile may be
    /// empty, so Win+D keeps the shape for the windows to come back to.
    pub fn vacate(&self, hwnd: isize) -> Option<Tree> {
        let at = self.zone_of(hwnd)?;
        let mut tree = self.clone();
        tree.tiles[at].hwnd = None;
        Some(tree)
    }

    /// `hwnd` goes back to zone `at` if that tile is still empty.
    pub fn back_to(&self, at: usize, hwnd: isize, min: Min) -> Option<Tree> {
        if self.zone_of(hwnd).is_some() || self.tiles.get(at)?.hwnd.is_some() {
            return None;
        }
        let mut tree = self.clone();
        tree.tiles[at] = Tile { hwnd: Some(hwnd), min };
        Some(tree)
    }

    /// The same windows, in the same order, laid out by another family. Empty tiles go.
    pub fn with_family(&self, family: Family, area: Rect, gap: i32) -> Tree {
        let tiles: Vec<Tile> = self.tiles.iter().copied().filter(|t| t.hwnd.is_some()).collect();
        if tiles.is_empty() {
            return Tree::new(family);
        }
        if family != Family::Smart {
            return Tree { family, node: Some(fixed(family, tiles.len(), None)), tiles };
        }
        let mut tree = Tree::new(family);
        for t in &tiles {
            let Some(hwnd) = t.hwnd else { continue };
            // Every window was tiled before, so none of them ends up stacked by its size here.
            match tree.insert(area, gap, hwnd, (0, 0)) {
                Some((mut next, Put::Tile(at))) => {
                    next.tiles[at].min = t.min;
                    tree = next;
                }
                // Past what Smart can split, it is laid out as columns rather than lost.
                _ => return Tree { family, node: Some(fixed(Family::Columns, tiles.len(), None)), tiles },
            }
        }
        tree
    }

    /// The same windows in the same tiles with every split even again, as Reset sizes
    /// asks. Smart goes back to halves; a fixed family is laid out as it is for a new
    /// window, Main + stack at its default main share.
    pub fn evened(&self) -> Tree {
        let Some(node) = &self.node else { return self.clone() };
        fn halved(node: &Node) -> Node {
            match node {
                Node::Split { split: dir, a, b, .. } => split(*dir, 0.5, halved(a), halved(b)),
                Node::Leaf(_) => leaf(),
            }
        }
        let node = match self.family {
            Family::Smart => halved(node),
            family => fixed(family, self.tiles.len(), None),
        };
        Tree { node: Some(node), ..self.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };
    const NONE: Min = (0, 0);

    fn grow(family: Family, n: isize) -> Tree {
        let mut tree = Tree::new(family);
        for h in 1..=n {
            let (next, put) = tree.insert(AREA, 0, h, NONE).unwrap();
            assert!(matches!(put, Put::Tile(_)), "{family:?} window {h} wasn't tiled");
            tree = next;
        }
        tree
    }

    /// The tiles cover the area exactly, with no overlap.
    fn covers(tree: &Tree) {
        let rects = tree.rects(AREA, 0);
        assert_eq!(rects.len(), tree.tiles.len());
        assert_eq!(rects.iter().map(area_of).sum::<i64>(), area_of(&AREA));
    }

    fn rect_of(tree: &Tree, h: isize) -> Rect {
        tree.rects(AREA, 0)[tree.zone_of(h).unwrap()]
    }

    #[test]
    fn smart_splits_the_largest_tile_along_its_longer_side() {
        let one = grow(Family::Smart, 1);
        assert_eq!(rect_of(&one, 1), AREA);
        // Landscape: side by side, the new window second.
        let two = grow(Family::Smart, 2);
        assert_eq!(rect_of(&two, 1), Rect { x: 0, y: 0, w: 960, h: 1080 });
        assert_eq!(rect_of(&two, 2), Rect { x: 960, y: 0, w: 960, h: 1080 });
        // Both halves are as large: the first is split, and it is taller than wide.
        let three = grow(Family::Smart, 3);
        assert_eq!(rect_of(&three, 1), Rect { x: 0, y: 0, w: 960, h: 540 });
        assert_eq!(rect_of(&three, 3), Rect { x: 0, y: 540, w: 960, h: 540 });
        assert_eq!(rect_of(&three, 2), Rect { x: 960, y: 0, w: 960, h: 1080 });
        // The largest tile now is window 2's.
        let four = grow(Family::Smart, 4);
        assert_eq!(rect_of(&four, 2), Rect { x: 960, y: 0, w: 960, h: 540 });
        assert_eq!(rect_of(&four, 4), Rect { x: 960, y: 540, w: 960, h: 540 });
        for n in 1..=MAX_TILES as isize {
            covers(&grow(Family::Smart, n));
        }
    }

    #[test]
    fn fixed_families_lay_every_window_out_by_count() {
        let cols = grow(Family::Columns, 3);
        assert_eq!(cols.rects(AREA, 0).iter().map(|r| r.w).collect::<Vec<_>>(), vec![640, 640, 640]);
        let main = grow(Family::MainStack, 3);
        assert_eq!(rect_of(&main, 1), Rect { x: 0, y: 0, w: 1152, h: 1080 });
        assert_eq!(rect_of(&main, 2), Rect { x: 1152, y: 0, w: 768, h: 540 });
        assert_eq!(rect_of(&main, 3), Rect { x: 1152, y: 540, w: 768, h: 540 });
        let grid5 = grow(Family::Grid, 5);
        // Three columns: two rows, two rows, one.
        let widths: Vec<i32> = grid5.rects(AREA, 0).iter().map(|r| r.w).collect();
        assert_eq!(widths, vec![640, 640, 640, 640, 640]);
        assert_eq!(rect_of(&grid5, 5).h, 1080);
        for family in Family::ALL {
            for n in 1..=MAX_TILES as isize {
                let tree = grow(family, n);
                covers(&tree);
                assert!(zones::within_limits(&tree.layout()), "{family:?} {n}");
            }
        }
    }

    #[test]
    fn closing_a_window_lets_its_neighbours_fill_the_gap() {
        for family in Family::ALL {
            let tree = grow(family, 5);
            let less = tree.remove(3).unwrap();
            let mut left: Vec<isize> = less.windows().collect();
            left.sort_unstable();
            assert_eq!((left, less.tiles.len()), (vec![1, 2, 4, 5], 4), "{family:?}");
            covers(&less);
            assert_eq!(less.remove(3), None);
        }
        // Smart: the sibling of the tile that went takes its space, nothing else moves.
        let three = grow(Family::Smart, 3);
        let two = three.remove(3).unwrap();
        assert_eq!(rect_of(&two, 1), Rect { x: 0, y: 0, w: 960, h: 1080 });
        assert_eq!(rect_of(&two, 2), rect_of(&three, 2));
        // The last window going leaves an empty tree.
        assert_eq!(grow(Family::Smart, 1).remove(1), Some(Tree::new(Family::Smart)));
    }

    #[test]
    fn a_window_moved_by_hand_leaves_its_tile_empty_for_the_next() {
        let three = grow(Family::Smart, 3);
        let left = three.vacate(2).unwrap();
        assert_eq!(left.rects(AREA, 0), three.rects(AREA, 0));
        assert_eq!(left.zone_of(2), None);
        let (next, put) = left.insert(AREA, 0, 9, NONE).unwrap();
        assert_eq!(put, Put::Tile(three.zone_of(2).unwrap()));
        assert_eq!(next.rects(AREA, 0), three.rects(AREA, 0));
        // A window too big for the empty tile splits another instead.
        let (_, put) = left.insert(AREA, 0, 9, (1000, 100)).unwrap();
        assert_eq!(put, Put::Stack(1));
    }

    #[test]
    fn a_window_that_cant_shrink_to_its_share_stacks_on_the_largest() {
        let two = grow(Family::Smart, 2);
        // A third would get 960 x 540: too small for it.
        let (same, put) = two.insert(AREA, 0, 3, (1000, 600)).unwrap();
        assert_eq!((same, put), (two.clone(), Put::Stack(1)));
        // And when a share would be too small for a window already tiled.
        let mut wide = Tree::new(Family::Columns);
        wide = wide.insert(AREA, 0, 1, (1200, 100)).unwrap().0;
        assert_eq!(wide.insert(AREA, 0, 2, NONE).unwrap().1, Put::Stack(1));
        // A full screen stacks too.
        let full = grow(Family::Columns, MAX_TILES as isize);
        assert!(matches!(full.insert(AREA, 0, 99, NONE).unwrap().1, Put::Stack(_)));
        // Tiled already is no insert, and an empty tree takes any window.
        assert_eq!(two.insert(AREA, 0, 2, NONE), None);
        assert!(Tree::new(Family::Grid).insert(AREA, 0, 1, (5000, 5000)).is_some());
    }

    /// No overlap, nothing outside the area, no zero-size zone, one zone per tile.
    fn sound(tree: &Tree, area: Rect, gap: i32) {
        if tree.node.is_none() {
            assert!(tree.tiles.is_empty());
            return;
        }
        let rects = tree.rects(area, gap);
        assert_eq!(rects.len(), tree.tiles.len());
        for (i, r) in rects.iter().enumerate() {
            assert!(r.w > 0 && r.h > 0, "{r:?}");
            assert!(r.x >= area.x && r.y >= area.y && r.x + r.w <= area.x + area.w && r.y + r.h <= area.y + area.h);
            for o in &rects[i + 1..] {
                let apart = r.x + r.w <= o.x || o.x + o.w <= r.x || r.y + r.h <= o.y || o.y + o.h <= r.y;
                assert!(apart, "{r:?} overlaps {o:?}");
            }
        }
        let mut seen: Vec<isize> = tree.windows().collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), tree.windows().count(), "a window is in two tiles");
        assert!(tree.tiles.len() <= MAX_TILES);
        assert!(zones::within_limits(&tree.layout()));
    }

    #[test]
    fn odd_areas_and_gaps_stay_sound_for_every_family() {
        let areas = [
            Rect { x: 0, y: 0, w: 1366, h: 728 },
            Rect { x: -2560, y: 40, w: 2560, h: 1392 },
            Rect { x: 7, y: 3, w: 1001, h: 997 },
            Rect { x: 0, y: 0, w: 800, h: 600 },
        ];
        for area in areas {
            for gap in [0, 8, 12] {
                for family in Family::ALL {
                    let mut tree = Tree::new(family);
                    for h in 1..=14 {
                        tree = tree.insert(area, gap, h, NONE).unwrap().0;
                        sound(&tree, area, gap);
                    }
                }
            }
        }
    }

    /// A small fixed-seed generator, so the walk is the same on every run.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    #[test]
    fn random_steps_keep_the_tree_sound() {
        for family in Family::ALL {
            let mut rng = Lcg(0x5eed ^ family as u64);
            let mut tree = Tree::new(family);
            let mut known: Vec<isize> = Vec::new();
            for step in 0..4000 {
                let next = step as isize + 100;
                match rng.next(10) {
                    0..=4 => {
                        let min = (rng.next(900) as i32, rng.next(600) as i32);
                        if let Some((t, put)) = tree.insert(AREA, 8, next, min) {
                            if let Put::Tile(_) = put {
                                known.push(next);
                            }
                            tree = t;
                        }
                    }
                    5..=6 if !known.is_empty() => {
                        let h = known.remove(rng.next(known.len() as u64) as usize);
                        tree = tree.remove(h).unwrap();
                    }
                    7 if !known.is_empty() => {
                        let h = known.remove(rng.next(known.len() as u64) as usize);
                        tree = tree.vacate(h).unwrap();
                    }
                    8 => {
                        let to = Family::ALL[rng.next(4) as usize];
                        tree = tree.with_family(to, AREA, 8);
                        // An empty tile goes with another family.
                        assert_eq!(tree.windows().count(), known.len());
                    }
                    _ => {}
                }
                sound(&tree, AREA, 8);
                let mut have: Vec<isize> = tree.windows().collect();
                let mut want = known.clone();
                have.sort_unstable();
                want.sort_unstable();
                assert_eq!(have, want, "{family:?} step {step}");
            }
        }
    }

    #[test]
    fn smart_removes_with_empty_tiles_about() {
        let three = grow(Family::Smart, 3);
        let holed = three.vacate(2).unwrap();
        let less = holed.remove(1).unwrap();
        assert_eq!((less.tiles.len(), less.windows().collect::<Vec<_>>()), (2, vec![3]));
        covers(&less);
        // The last window going, with only empty tiles left, leaves an empty tree.
        assert_eq!(less.remove(3), Some(Tree::new(Family::Smart)));
        // Every tile empty keeps the shape, and each window can go back to its own.
        let rested = three.vacate(1).unwrap().vacate(2).unwrap().vacate(3).unwrap();
        assert_eq!(rested.rects(AREA, 0), three.rects(AREA, 0));
        let back = rested.back_to(2, 2, NONE).unwrap();
        assert_eq!(back.zone_of(2), three.zone_of(2));
        assert_eq!(back.back_to(2, 9, NONE), None);
    }

    #[test]
    fn fixed_families_fill_an_empty_tile_first_then_close_up() {
        for family in [Family::Columns, Family::MainStack, Family::Grid] {
            let four = grow(family, 4);
            let holed = four.vacate(2).unwrap();
            let (filled, put) = holed.insert(AREA, 0, 9, NONE).unwrap();
            assert_eq!((put, filled.rects(AREA, 0)), (Put::Tile(four.zone_of(2).unwrap()), four.rects(AREA, 0)));
            let less = filled.remove(9).unwrap();
            assert_eq!(less.tiles.len(), 3, "{family:?}");
            covers(&less);
        }
    }

    #[test]
    fn counts_and_shapes_by_family() {
        for n in 1..=MAX_TILES {
            let cols = (1..=n).find(|c| c * c >= n).unwrap();
            let grid = grow(Family::Grid, n as isize);
            covers(&grid);
            let mut lefts: Vec<i32> = grid.rects(AREA, 0).iter().map(|r| r.x).collect();
            lefts.dedup();
            assert_eq!((grid.tiles.len(), lefts.len()), (n, cols), "grid of {n}");
        }
        let main = grow(Family::MainStack, 2);
        assert_eq!(main.rects(AREA, 0).iter().map(|r| r.w).collect::<Vec<_>>(), vec![1152, 768]);
        assert_eq!(grow(Family::Columns, 1).rects(AREA, 0), vec![AREA]);
    }

    #[test]
    fn main_stack_keeps_a_handle_dragged_main_share() {
        let mut three = grow(Family::MainStack, 3);
        three.node = Some(zones::set_ratio(&three.layout(), &[], 0.75));
        let four = three.insert(AREA, 0, 4, NONE).unwrap().0;
        assert_eq!(four.rects(AREA, 0)[0].w, 1440);
        assert_eq!(four.remove(4).unwrap().rects(AREA, 0)[0].w, 1440);
    }

    #[test]
    fn smart_with_a_window_past_its_share_loses_none() {
        let mut tree = Tree::new(Family::Columns);
        for h in 1..=MAX_TILES as isize {
            tree = tree.insert(AREA, 0, h, NONE).unwrap().0;
        }
        tree.tiles[0].min = (1900, 1000);
        let smart = tree.with_family(Family::Smart, AREA, 0);
        assert_eq!(smart.windows().count(), MAX_TILES);
        assert_eq!(smart.tiles[smart.zone_of(1).unwrap()].min, (1900, 1000));
    }

    #[test]
    fn a_window_counts_as_moved_off_its_tile() {
        let at = Rect { x: 0, y: 0, w: 960, h: 1080 };
        assert!(!moved(at, at, true));
        // A terminal a few characters short of its tile stays.
        assert!(!moved(Rect { w: 930, h: 1050, ..at }, at, true));
        assert!(moved(Rect { w: 900, ..at }, at, true));
        assert!(!moved(Rect { w: 900, ..at }, at, false));
        assert!(moved(Rect { x: 5, ..at }, at, true));
        assert!(moved(Rect { w: 965, ..at }, at, true));
        assert!(!moved(Rect { x: 4, w: 964, ..at }, at, true));
    }

    #[test]
    fn only_a_window_nobody_else_placed_moves() {
        assert!(may_move(&Seen::default()));
        for s in [
            Seen { floated: true, ..Seen::default() },
            Seen { moved: true, ..Seen::default() },
            Seen { away: true, ..Seen::default() },
            Seen { gone: true, ..Seen::default() },
        ] {
            assert!(!may_move(&s), "{s:?}");
        }
        assert!(!floats(&Seen { away: true, ..Seen::default() }));
        assert!(!floats(&Seen { moved: true, gone: true, ..Seen::default() }));
        assert!(floats(&Seen { moved: true, away: true, ..Seen::default() }));
    }

    #[test]
    fn the_plan_floats_and_closes_up_before_anything_moves() {
        let four = grow(Family::Smart, 4);
        // 1 moved by hand, 2 closed unseen, 3 floated with a stacked window 9 under it.
        let seen = |h: isize| Seen { moved: h == 1, gone: h == 2, floated: h == 3, ..Seen::default() };
        let heir = |h: isize| (h == 3).then_some((9, (0, 0)));
        let plan = plan_apply(&four, seen, heir);
        let mut floated = plan.floated.clone();
        floated.sort_unstable();
        assert_eq!((plan.left, floated, plan.heirs), (vec![2], vec![1, 3], vec![(3, 9)]));
        let mut moves = plan.moves.clone();
        moves.sort_unstable();
        assert_eq!(moves, vec![4, 9]);
        assert_eq!(plan.tree.tiles.len(), 2);
        sound(&plan.tree, AREA, 0);
        // A capture or Quick save stamps windows but moves none: nothing floats.
        let quiet = plan_apply(&four, |_| Seen::default(), |_| None);
        assert_eq!((&quiet.tree, quiet.moves), (&four, four.windows().collect::<Vec<_>>()));
    }

    #[test]
    fn closing_a_minimized_tile_never_moves_a_hand_moved_window() {
        // Three windows; 2 was minimized, and 1 was moved by Win+Left since.
        let three = grow(Family::Smart, 3);
        let at = Instant::now();
        let rested = three.vacate(2).unwrap();
        let due = gaps_due(&[(three.zone_of(2).unwrap(), at, true)], None, at + GAP_AFTER);
        assert_eq!(due, vec![three.zone_of(2).unwrap()]);
        let mut tree = rested;
        for z in due {
            tree = tree.remove_zone(z).unwrap();
        }
        // The gap closing goes through the same plan as every move: the hand-moved window floats.
        let plan = plan_apply(&tree, |h| Seen { moved: h == 1, ..Seen::default() }, |_| None);
        assert_eq!((plan.floated, plan.moves), (vec![1], vec![3]));
    }

    #[test]
    fn win_d_keeps_every_tile_and_a_lone_minimize_closes() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        // Win+D: three minimized within a burst keep their tiles while restores keep coming.
        let burst = [(0, t0, true), (1, ms(40), true), (2, ms(90), true)];
        assert!(gaps_due(&burst, None, ms(29_000)).is_empty());
        assert!(gaps_due(&burst[1..], Some(ms(5000)), ms(34_000)).is_empty());
        // A burst with no restore for 30 s closes after all.
        assert_eq!(gaps_due(&burst, None, ms(30_100)), vec![2, 1, 0]);
        assert_eq!(gaps_due(&burst[1..], Some(ms(5000)), ms(35_000)), vec![2, 1]);
        // One minimize alone closes after the wait, not before.
        assert!(gaps_due(&[(1, t0, true)], None, ms(599)).is_empty());
        assert_eq!(gaps_due(&[(1, t0, true)], None, ms(600)), vec![1]);
        // A restore just now holds the others a moment longer.
        assert!(gaps_due(&[(1, t0, true)], Some(ms(900)), ms(1000)).is_empty());
        assert_eq!(gaps_due(&[(3, t0, true), (1, ms(2000), true)], None, ms(3000)), vec![3, 1]);
        // Windows a switch minimized together are no burst: they close at the normal wait.
        assert_eq!(gaps_due(&[(0, t0, false), (1, t0, false)], None, ms(600)), vec![1, 0]);
    }

    #[test]
    fn after_a_pause_the_latest_of_each_arrival_is_taken_but_not_the_switchs() {
        let pending = vec![(1, "open"), (2, "restore"), (1, "again"), (3, "open")];
        let taken = after_pause(pending, |h| h == 3);
        assert_eq!(taken, vec![(2, "restore"), (1, "again")]);
        assert!(after_pause(Vec::<(isize, ())>::new(), |_| false).is_empty());
    }

    #[test]
    fn a_drag_decides_who_holds_each_zone() {
        // A swap: each window is recorded in the other's zone.
        let swapped = holders(2, &[Some(1), Some(2)], &[None, None], &[(0, 2, true), (1, 1, true)]);
        assert_eq!(swapped, vec![Some(2), Some(1)]);
        // A stack by S: the window tiling had there keeps the tile, whoever is on top.
        assert_eq!(holders(1, &[Some(1)], &[None], &[(0, 9, true), (0, 1, false)]), vec![Some(1)]);
        // A drop from elsewhere into an empty tile; else a stack's top, else the lowest handle.
        assert_eq!(holders(1, &[None], &[None], &[(0, 7, false), (0, 8, true)]), vec![Some(8)]);
        assert_eq!(holders(1, &[None], &[None], &[(0, 8, false), (0, 7, false)]), vec![Some(7)]);
        // Dragged out: its zone is empty, unless a minimized window waits for it.
        assert_eq!(holders(2, &[Some(1), Some(2)], &[None, Some(2)], &[(0, 1, true)]), vec![Some(1), Some(2)]);
        assert_eq!(holders(2, &[Some(1), Some(2)], &[None, None], &[(0, 1, true)]), vec![Some(1), None]);
    }

    #[test]
    fn another_family_keeps_the_windows_and_their_order() {
        let smart = grow(Family::Smart, 4);
        let order: Vec<isize> = smart.windows().collect();
        let cols = smart.vacate(order[1]).unwrap().with_family(Family::Columns, AREA, 0);
        let kept: Vec<isize> = order.iter().copied().filter(|h| *h != order[1]).collect();
        assert_eq!(cols.windows().collect::<Vec<_>>(), kept);
        assert_eq!(cols.tiles.len(), 3);
        let back = cols.with_family(Family::Smart, AREA, 0);
        assert_eq!(back.windows().count(), 3);
        covers(&back);
        assert_eq!(Family::parse("main_stack"), Some(Family::MainStack));
        assert_eq!(Family::parse("dwindle"), None);
    }

    #[test]
    fn evening_out_gives_every_family_its_proportions_back() {
        let ratio_of = |tree: &Tree| match tree.layout() {
            Node::Split { ratio, .. } => ratio,
            Node::Leaf(_) => panic!("no split"),
        };
        for family in Family::ALL {
            let tree = grow(family, 4);
            // Fresh from tiling a family is even already.
            assert_eq!(tree.evened(), tree, "{family:?}");
            let dragged = Tree { node: Some(zones::set_ratio(&tree.layout(), &[], 0.8)), ..tree.clone() };
            let even = dragged.evened();
            assert_eq!(even.tiles, tree.tiles, "{family:?} keeps the windows in their tiles");
            assert_eq!(even.layout(), tree.layout(), "{family:?}");
            covers(&even);
        }
        // Main + stack goes back to its default main share, whatever a handle gave it.
        let main = grow(Family::MainStack, 3);
        let dragged = Tree { node: Some(zones::set_ratio(&main.layout(), &[], 0.8)), ..main.clone() };
        assert_eq!(ratio_of(&dragged), 0.8);
        assert_eq!(ratio_of(&dragged.evened()), MAIN_SHARE);
        // Smart: every split back to halves, deep ones too, with the same shape.
        let smart = grow(Family::Smart, 4);
        let deep = Tree { node: Some(zones::set_ratio(&smart.layout(), &[true], 0.3)), ..smart.clone() };
        assert_ne!(deep.layout(), smart.layout());
        assert_eq!(deep.evened().layout(), smart.layout());
        // Nothing tiled, nothing to even.
        assert_eq!(Tree::new(Family::Grid).evened(), Tree::new(Family::Grid));
    }
}
