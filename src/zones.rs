//! Zone layouts: a split tree per screen, turned into pixel rects on demand.
//! Pure logic, so the tray's snapping and the panel's preview agree.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `Vertical` puts the two children side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeafTag {
    Leaf,
}

/// JSON: "leaf" or {"split": "vertical", "ratio": 0.25, "a": …, "b": …}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Split { split: Dir, ratio: f32, a: Box<Node>, b: Box<Node> },
    Leaf(LeafTag),
}

/// What a layout started as. Only the tile highlight reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    #[default]
    None,
    Columns,
    Rows,
    Grid,
    Priority,
    Custom,
}

impl Template {
    pub const ALL: [Template; 6] =
        [Template::None, Template::Columns, Template::Rows, Template::Grid, Template::Priority, Template::Custom];

    pub fn title(self) -> &'static str {
        match self {
            Template::None => "None",
            Template::Columns => "Columns",
            Template::Rows => "Rows",
            Template::Grid => "Grid",
            Template::Priority => "Priority",
            Template::Custom => "Custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneLayout {
    pub template: Template,
    pub tree: Node,
}

impl ZoneLayout {
    /// The layout a template stands for. `None` and `Custom` have no tree of their own.
    pub fn of(template: Template) -> Option<ZoneLayout> {
        template_tree(template).map(|tree| ZoneLayout { template, tree })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

fn leaf() -> Box<Node> {
    Box::new(Node::Leaf(LeafTag::Leaf))
}

fn split(dir: Dir, ratio: f32, a: Box<Node>, b: Box<Node>) -> Box<Node> {
    Box::new(Node::Split { split: dir, ratio, a, b })
}

pub fn template_tree(t: Template) -> Option<Node> {
    use Dir::*;
    Some(*match t {
        Template::None | Template::Custom => return None,
        Template::Columns => split(Vertical, 1.0 / 3.0, leaf(), split(Vertical, 0.5, leaf(), leaf())),
        Template::Rows => split(Horizontal, 0.5, leaf(), leaf()),
        Template::Grid => {
            split(Vertical, 0.5, split(Horizontal, 0.5, leaf(), leaf()), split(Horizontal, 0.5, leaf(), leaf()))
        }
        Template::Priority => split(Vertical, 0.25, leaf(), split(Vertical, 2.0 / 3.0, leaf(), leaf())),
    })
}

/// A layout as saved in `desks[sig].zones[screen]`. A ratio outside 0.1 to 0.9
/// is clamped, and anything that isn't a layout reads as no zones.
pub fn layout_of(saved: &Value) -> Option<ZoneLayout> {
    let mut layout: ZoneLayout = serde_json::from_value(saved.clone()).ok()?;
    clamp(&mut layout.tree);
    Some(layout)
}

fn clamp(node: &mut Node) {
    if let Node::Split { ratio, a, b, .. } = node {
        *ratio = ratio.clamp(0.1, 0.9);
        clamp(a);
        clamp(b);
    }
}

/// Leaves in order (left subtree first), as rects inside `area`.
/// Edges of the area get `gap`; between zones each side gives gap/2.
pub fn rects(tree: &Node, area: Rect, gap: i32) -> Vec<Rect> {
    let mut out = Vec::new();
    walk(tree, area, &mut out);
    out.into_iter()
        .map(|r| {
            let l = if r.x == area.x { gap } else { gap / 2 };
            let t = if r.y == area.y { gap } else { gap / 2 };
            let rr = if r.x + r.w == area.x + area.w { gap } else { gap - gap / 2 };
            let b = if r.y + r.h == area.y + area.h { gap } else { gap - gap / 2 };
            Rect { x: r.x + l, y: r.y + t, w: (r.w - l - rr).max(1), h: (r.h - t - b).max(1) }
        })
        .collect()
}

fn walk(n: &Node, r: Rect, out: &mut Vec<Rect>) {
    match n {
        Node::Leaf(_) => out.push(r),
        Node::Split { split, ratio, a, b } => {
            let ratio = ratio.clamp(0.1, 0.9);
            match split {
                Dir::Vertical => {
                    let wa = (r.w as f32 * ratio).round() as i32;
                    walk(a, Rect { w: wa, ..r }, out);
                    walk(b, Rect { x: r.x + wa, w: r.w - wa, ..r }, out);
                }
                Dir::Horizontal => {
                    let ha = (r.h as f32 * ratio).round() as i32;
                    walk(a, Rect { h: ha, ..r }, out);
                    walk(b, Rect { y: r.y + ha, h: r.h - ha, ..r }, out);
                }
            }
        }
    }
}

/// The zone a point is in. Pass rects made with no gap, so the gaps between
/// zones belong to one of them.
pub fn zone_at(cells: &[Rect], x: i32, y: i32) -> Option<usize> {
    cells.iter().position(|r| r.contains(x, y))
}

/// Bounding box of two zones (Ctrl-span).
pub fn span(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let r = (a.x + a.w).max(b.x + b.w);
    let btm = (a.y + a.h).max(b.y + b.h);
    Rect { x, y, w: r - x, h: btm - y }
}

/// `window` = GetWindowRect, `visible` = DWMWA_EXTENDED_FRAME_BOUNDS.
/// Returns the rect to pass to SetWindowPos so the *visible* frame lands on `target`.
pub fn frame_adjust(target: Rect, window: Rect, visible: Rect) -> Rect {
    let l = visible.x - window.x;
    let t = visible.y - window.y;
    let r = (window.x + window.w) - (visible.x + visible.w);
    let b = (window.y + window.h) - (visible.y + visible.h);
    Rect { x: target.x - l, y: target.y - t, w: target.w + l + r, h: target.h + t + b }
}

// ---- Editing. Every function returns a new tree and leaves the old one alone.

/// Most zones in a layout: the stamp on a window holds zone numbers 0 to 127.
pub const MAX_ZONES: usize = 128;
/// Deepest a tree may nest, so no layout gets absurd.
pub const MAX_DEPTH: usize = 12;
/// Smallest side, in screen pixels, a split may leave a zone with.
pub const MIN_ZONE_PX: i32 = 120;

pub fn leaf_count(node: &Node) -> usize {
    match node {
        Node::Leaf(_) => 1,
        Node::Split { a, b, .. } => leaf_count(a) + leaf_count(b),
    }
}

/// How many splits lie above zone `index`, counting from 1 zone = 0.
pub fn depth_of(node: &Node, index: usize) -> Option<usize> {
    match node {
        Node::Leaf(_) => (index == 0).then_some(0),
        Node::Split { a, b, .. } => {
            let first = leaf_count(a);
            if index < first {
                depth_of(a, index).map(|d| d + 1)
            } else {
                depth_of(b, index - first).map(|d| d + 1)
            }
        }
    }
}

/// Whether the layout has room for another zone.
pub fn can_split(tree: &Node) -> bool {
    leaf_count(tree) < MAX_ZONES
}

/// Cut zone `index` in two at 0.5. `None` if there's no such zone, there are
/// already `MAX_ZONES` zones, or the tree would nest deeper than `MAX_DEPTH`.
pub fn split_leaf(tree: &Node, index: usize, dir: Dir) -> Option<Node> {
    if !can_split(tree) || depth_of(tree, index)? >= MAX_DEPTH {
        return None;
    }
    split_at(tree, index, dir)
}

fn split_at(node: &Node, index: usize, dir: Dir) -> Option<Node> {
    match node {
        Node::Leaf(_) => (index == 0).then(|| Node::Split { split: dir, ratio: 0.5, a: leaf(), b: leaf() }),
        Node::Split { split, ratio, a, b } => {
            let first = leaf_count(a);
            let (split, ratio) = (*split, *ratio);
            Some(if index < first {
                Node::Split { split, ratio, a: Box::new(split_at(a, index, dir)?), b: b.clone() }
            } else {
                Node::Split { split, ratio, a: a.clone(), b: Box::new(split_at(b, index - first, dir)?) }
            })
        }
    }
}

/// Take away the split above zone `index`, joining it with its neighbour. Only
/// when that neighbour is a single zone. Returns the tree and the number of the joined zone.
pub fn merge_leaf(tree: &Node, index: usize) -> Option<(Node, usize)> {
    match tree {
        Node::Leaf(_) => None,
        Node::Split { split, ratio, a, b } => {
            let first = leaf_count(a);
            if matches!((&**a, &**b), (Node::Leaf(_), Node::Leaf(_))) {
                return (index < 2).then_some((Node::Leaf(LeafTag::Leaf), 0));
            }
            let (split, ratio) = (*split, *ratio);
            if index < first {
                let (merged, at) = merge_leaf(a, index)?;
                Some((Node::Split { split, ratio, a: Box::new(merged), b: b.clone() }, at))
            } else {
                let (merged, at) = merge_leaf(b, index - first)?;
                Some((Node::Split { split, ratio, a: a.clone(), b: Box::new(merged) }, at + first))
            }
        }
    }
}

/// A line between two parts of the tree, which the editor lets you drag.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    /// The way down to its split: false is the first child, true the second.
    pub path: Vec<bool>,
    pub dir: Dir,
    /// The area the split divides.
    pub area: Rect,
    pub ratio: f32,
}

pub fn dividers(tree: &Node, area: Rect) -> Vec<Divider> {
    let mut out = Vec::new();
    collect_dividers(tree, area, &mut Vec::new(), &mut out);
    out
}

fn collect_dividers(node: &Node, r: Rect, path: &mut Vec<bool>, out: &mut Vec<Divider>) {
    let Node::Split { split, ratio, a, b } = node else { return };
    let ratio = ratio.clamp(0.1, 0.9);
    out.push(Divider { path: path.clone(), dir: *split, area: r, ratio });
    let (first, second) = match split {
        Dir::Vertical => {
            let wa = (r.w as f32 * ratio).round() as i32;
            (Rect { w: wa, ..r }, Rect { x: r.x + wa, w: r.w - wa, ..r })
        }
        Dir::Horizontal => {
            let ha = (r.h as f32 * ratio).round() as i32;
            (Rect { h: ha, ..r }, Rect { y: r.y + ha, h: r.h - ha, ..r })
        }
    };
    path.push(false);
    collect_dividers(a, first, path, out);
    path.pop();
    path.push(true);
    collect_dividers(b, second, path, out);
    path.pop();
}

/// The same tree with the split at `path` moved to `ratio`, kept between 0.1 and 0.9.
pub fn set_ratio(tree: &Node, path: &[bool], ratio: f32) -> Node {
    match (tree, path.split_first()) {
        (Node::Split { split, a, b, .. }, None) => {
            Node::Split { split: *split, ratio: ratio.clamp(0.1, 0.9), a: a.clone(), b: b.clone() }
        }
        (Node::Split { split, ratio: own, a, b }, Some((false, rest))) => {
            Node::Split { split: *split, ratio: *own, a: Box::new(set_ratio(a, rest, ratio)), b: b.clone() }
        }
        (Node::Split { split, ratio: own, a, b }, Some((true, rest))) => {
            Node::Split { split: *split, ratio: *own, a: a.clone(), b: Box::new(set_ratio(b, rest, ratio)) }
        }
        (leaf, _) => leaf.clone(),
    }
}

/// A dragged ratio, pulled to 1/4, 1/3, 1/2, 2/3 or 3/4 when it's within 2% of one.
pub fn snap_ratio(ratio: f32) -> f32 {
    const MARKS: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];
    let near = MARKS.into_iter().find(|m| (ratio - m).abs() <= 0.02);
    near.unwrap_or(ratio).clamp(0.1, 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test #2: every template at a landscape and a portrait work area, with and without a gap, plus a span.
    #[test]
    fn templates_tile_the_work_area() {
        let areas = [Rect { x: 0, y: 0, w: 1920, h: 1040 }, Rect { x: 3840, y: -1000, w: 1440, h: 3400 }];
        for t in [Template::Columns, Template::Rows, Template::Grid, Template::Priority] {
            for area in areas {
                for gap in [0, 8] {
                    let rs = rects(&template_tree(t).unwrap(), area, gap);
                    let filled: i64 = rs.iter().map(|r| r.w as i64 * r.h as i64).sum();
                    assert!(filled <= area.w as i64 * area.h as i64, "{t:?} overlaps");
                    if gap == 0 {
                        assert_eq!(filled, area.w as i64 * area.h as i64, "{t:?} leaves holes");
                    }
                    assert!(rs.iter().all(|r| r.x >= area.x + gap && r.y >= area.y + gap));
                    assert!(rs
                        .iter()
                        .all(|r| r.x + r.w <= area.x + area.w - gap && r.y + r.h <= area.y + area.h - gap));
                }
            }
        }
        let p = rects(&template_tree(Template::Priority).unwrap(), Rect { x: 0, y: 0, w: 3840, h: 2100 }, 0);
        assert_eq!(p.iter().map(|r| r.w).collect::<Vec<_>>(), vec![960, 1920, 960]);
        assert_eq!(span(p[0], p[1]), Rect { x: 0, y: 0, w: 2880, h: 2100 });
        // With a gap the zones still start 8 px in from the edge and sit 8 px apart.
        let g = rects(&template_tree(Template::Rows).unwrap(), Rect { x: 0, y: 0, w: 1920, h: 1040 }, 8);
        assert_eq!((g[0].y, g[0].h, g[1].y - (g[0].y + g[0].h)), (8, 508, 8));
    }

    #[test]
    fn frame_adjust_adds_invisible_borders() {
        let window = Rect { x: 93, y: 100, w: 814, h: 607 };
        let visible = Rect { x: 100, y: 100, w: 800, h: 600 }; // 7 px left/right/bottom, 0 top
        let got = frame_adjust(Rect { x: 0, y: 0, w: 960, h: 1040 }, window, visible);
        assert_eq!(got, Rect { x: -7, y: 0, w: 974, h: 1047 });
    }
}
