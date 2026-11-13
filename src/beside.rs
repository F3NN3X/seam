//! Snap beside a window: which edge of a window the pointer is on during a drag,
//! and how a window dropped there shares that window's place. An ad-hoc group is
//! a tree over an area, as a screen's zones are, so the two halves are its zones.
//! Pure logic, so the snapping and its test agree.

use crate::groups::{self, Side};
use crate::zones::{self, LeafTag, MIN_ZONE_PX, Node, Rect};

/// The band along an edge is this share of the window's size across it, within the two limits.
const BAND_SHARE: f32 = 0.15;
const BAND_MIN_PX: i32 = 24;
const BAND_MAX_PX: i32 = 120;

/// The edge of `r` whose band `(x, y)` is in, the nearest when it is in two. None
/// in the middle of `r` or outside it.
pub fn band(r: Rect, (x, y): (i32, i32)) -> Option<Side> {
    if !r.contains(x, y) {
        return None;
    }
    let width = |len: i32| ((len as f32 * BAND_SHARE).round() as i32).clamp(BAND_MIN_PX, BAND_MAX_PX);
    let (across, down) = (width(r.w), width(r.h));
    let edges = [
        (Side::Left, x - r.x, across),
        (Side::Right, r.x + r.w - 1 - x, across),
        (Side::Top, y - r.y, down),
        (Side::Bottom, r.y + r.h - 1 - y, down),
    ];
    edges.into_iter().filter(|(_, off, band)| off < band).min_by_key(|(_, off, _)| *off).map(|(side, ..)| side)
}

/// The area of a new group around one window's frame: the frame with the gap
/// around it, so the group's one zone is the frame itself.
pub fn area_around(frame: Rect, gap: i32) -> Rect {
    Rect { x: frame.x - gap, y: frame.y - gap, w: frame.w + 2 * gap, h: frame.h + 2 * gap }
}

/// A group of one zone, for a window that isn't in a group yet.
pub fn single() -> Node {
    Node::Leaf(LeafTag::Leaf)
}

/// Zone `leaf` of `tree` over `area` cut in two for a window dropped on its `side`,
/// the dropped window taking `share` of it. The new tree, the dropped window's zone
/// and the rest's; zones after `leaf` move up by one. None when a piece would be
/// under `MIN_ZONE_PX` or the tree would pass the editor's limits.
pub fn cut(tree: &Node, area: Rect, leaf: usize, side: Side, share: f32) -> Option<(Node, usize, usize)> {
    let (tree, at, rest) = groups::split_part(tree, (leaf, leaf), share, side)?;
    let cells = zones::rects(&tree, area, 0);
    let small = |i: usize| cells.get(i).is_none_or(|c| c.w < MIN_ZONE_PX || c.h < MIN_ZONE_PX);
    (!small(at) && !small(rest)).then_some((tree, at, rest))
}

/// Zone `leaf` taken out of `tree` over `area`: the zone beside it in the tree takes
/// its room, and zones after it move down by one. When the zone beside it is one
/// whole side of the area, the area shrinks to that side instead, so its windows
/// stay where they are. None for a tree of one zone.
pub fn remove(tree: &Node, area: Rect, gap: i32, leaf: usize) -> Option<(Node, Rect)> {
    let Node::Split { a, b, .. } = tree else { return None };
    let first = zones::leaf_count(a);
    let rects = zones::rects(tree, area, gap);
    let other = match (&**a, &**b) {
        (Node::Leaf(_), _) if leaf == 0 => Some((b, 1..rects.len())),
        (_, Node::Leaf(_)) if leaf == first => Some((a, 0..first)),
        _ => None,
    };
    if let Some((side, range)) = other {
        let span = rects[range].iter().copied().reduce(zones::span)?;
        return Some(((**side).clone(), area_around(span, gap)));
    }
    Some((without(tree, leaf)?, area))
}

/// `node` with zone `leaf` gone and its sibling in its parent's place.
fn without(node: &Node, leaf: usize) -> Option<Node> {
    let Node::Split { split, ratio, a, b } = node else { return None };
    let (split, ratio, first) = (*split, *ratio, zones::leaf_count(a));
    if leaf < first {
        if matches!(**a, Node::Leaf(_)) {
            return Some((**b).clone());
        }
        return Some(Node::Split { split, ratio, a: Box::new(without(a, leaf)?), b: b.clone() });
    }
    if matches!(**b, Node::Leaf(_)) {
        return Some((**a).clone());
    }
    Some(Node::Split { split, ratio, a: a.clone(), b: Box::new(without(b, leaf - first)?) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_on_an_edge_band_shares_that_window() {
        // 15% of 1000 is 150, held to 120; 15% of 100 is 15, raised to 24.
        let r = Rect { x: 0, y: 0, w: 1000, h: 100 };
        assert_eq!(band(r, (119, 50)), Some(Side::Left));
        assert_eq!(band(r, (120, 50)), None);
        assert_eq!(band(r, (500, 23)), Some(Side::Top));
        assert_eq!(band(r, (500, 50)), None);
        assert_eq!(band(r, (995, 98)), Some(Side::Bottom));
        assert_eq!(band(r, (1000, 50)), None);
        // A window dropped on the right edge takes the right third, with the gap between them.
        let frame = Rect { x: 100, y: 100, w: 900, h: 600 };
        let area = area_around(frame, 8);
        assert_eq!(zones::rects(&single(), area, 8), vec![frame]);
        let (tree, at, rest) = cut(&single(), area, 0, Side::Right, 1.0 / 3.0).unwrap();
        let rects = zones::rects(&tree, area, 8);
        assert_eq!((at, rest), (1, 0));
        assert_eq!(rects[rest], Rect { x: 100, y: 100, w: 599, h: 600 });
        assert_eq!(rects[at], Rect { x: 707, y: 100, w: 293, h: 600 });
        // A third beside it on the left of the first: zones after it move up by one.
        let (tree, at, rest) = cut(&tree, area, 0, Side::Left, 0.5).unwrap();
        assert_eq!((at, rest, zones::leaf_count(&tree)), (0, 1, 3));
        // Too small a piece is no cut.
        assert_eq!(cut(&single(), Rect { x: 0, y: 0, w: 200, h: 600 }, 0, Side::Left, 0.5), None);
    }

    #[test]
    fn a_window_leaving_its_group_closes_it_up() {
        let area = Rect { x: 0, y: 0, w: 1800, h: 1000 };
        let three = zones::template_tree(zones::Template::Columns).unwrap();
        let xs = |tree: &Node, area: Rect| zones::rects(tree, area, 0).iter().map(|c| (c.x, c.w)).collect::<Vec<_>>();
        // The first column, one whole side: the area shrinks to the other two, which stay.
        let (tree, shrunk) = remove(&three, area, 0, 0).unwrap();
        assert_eq!(shrunk, Rect { x: 600, y: 0, w: 1200, h: 1000 });
        assert_eq!(xs(&tree, shrunk), vec![(600, 600), (1200, 600)]);
        // The last column: the middle one takes its room, and the area stays.
        let (tree, same) = remove(&three, area, 0, 2).unwrap();
        assert_eq!(same, area);
        assert_eq!(xs(&tree, same), vec![(0, 600), (600, 1200)]);
        // With the gap, the area keeps the gap around what is left.
        let (_, shrunk) = remove(&three, area, 8, 0).unwrap();
        assert_eq!(shrunk, Rect { x: 596, y: 0, w: 1204, h: 1000 });
        // One zone is no group to leave.
        assert_eq!(remove(&single(), area, 0, 0), None);
    }
}
