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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
