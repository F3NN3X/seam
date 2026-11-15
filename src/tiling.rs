//! Smart tiling: the windows tiled on one screen of one virtual desktop, as a zone
//! tree (`zones::Node`) and the window in each zone. Pure, so every family is
//! tested here, and the snapping only carries out what it decides. Smart splits
//! the largest tile along its longer side for a new window (dwindle by size);
//! Columns, Main + stack and Grid lay every window out again by count.

use crate::zones::{self, Dir, LeafTag, Node, Rect};
use serde::{Deserialize, Serialize};

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

/// A fixed family's tree for `n` windows.
fn fixed(family: Family, n: usize) -> Node {
    match family {
        Family::Columns | Family::Smart => even(Dir::Vertical, n),
        Family::MainStack if n > 1 => split(Dir::Vertical, MAIN_SHARE, leaf(), even(Dir::Horizontal, n - 1)),
        Family::MainStack => leaf(),
        Family::Grid => grid(n),
    }
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
                Some(Tree { family, node: Some(fixed(family, tiles.len())), tiles })
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
        let at = self.zone_of(hwnd)?;
        let mut tiles = self.tiles.clone();
        tiles.remove(at);
        if tiles.iter().all(|t| t.hwnd.is_none()) {
            return Some(Tree::new(self.family));
        }
        let node = match self.family {
            Family::Smart => zones::remove_leaf(self.node.as_ref()?, at)?,
            family => fixed(family, tiles.len()),
        };
        Some(Tree { family: self.family, node: Some(node), tiles })
    }

    /// `hwnd` left its tile by hand: the tile stays, empty, so nothing moves for it.
    pub fn vacate(&self, hwnd: isize) -> Option<Tree> {
        let at = self.zone_of(hwnd)?;
        let mut tree = self.clone();
        tree.tiles[at].hwnd = None;
        if tree.tiles.iter().all(|t| t.hwnd.is_none()) {
            return Some(Tree::new(self.family));
        }
        Some(tree)
    }

    /// The same windows, in the same order, laid out by another family. Empty tiles go.
    pub fn with_family(&self, family: Family, area: Rect, gap: i32) -> Tree {
        let tiles: Vec<Tile> = self.tiles.iter().copied().filter(|t| t.hwnd.is_some()).collect();
        if tiles.is_empty() {
            return Tree::new(family);
        }
        if family != Family::Smart {
            return Tree { family, node: Some(fixed(family, tiles.len())), tiles };
        }
        let mut tree = Tree::new(family);
        for t in &tiles {
            let Some(hwnd) = t.hwnd else { continue };
            // Every window was tiled before, so none of them ends up stacked by its size here.
            let relaxed = tree.insert(area, gap, hwnd, (0, 0));
            if let Some((mut next, Put::Tile(at))) = relaxed {
                next.tiles[at].min = t.min;
                tree = next;
            }
        }
        tree
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
}
