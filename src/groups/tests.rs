//! The tests for the divider solver and the group moves.

use super::*;

const AREA: Rect = Rect { x: 0, y: 0, w: 1800, h: 1000 };

fn columns() -> Node {
    zones::template_tree(Template::Columns).unwrap()
}

fn xs(cells: &[Rect]) -> Vec<(i32, i32)> {
    cells.iter().map(|c| (c.x, c.w)).collect()
}

/// Every split's ratio as stored, before `zones` clamps it.
fn stored(n: &Node) -> Vec<f32> {
    match n {
        Node::Split { ratio, a, b, .. } => [vec![*ratio], stored(a), stored(b)].concat(),
        Node::Leaf(_) => Vec::new(),
    }
}

fn edge(path: &[bool]) -> Edge {
    Edge { path: path.to_vec() }
}

#[test]
fn a_cut_gives_the_window_its_part_on_its_side() {
    // A third on the right of the middle column: the rest keeps the left of it.
    let (tree, at, rest) = split_part(&columns(), (1, 1), 1.0 / 3.0, Side::Right).unwrap();
    let cells = zones::rects(&tree, AREA, 0);
    assert_eq!((at, rest), (2, 1));
    assert_eq!(xs(&cells), vec![(0, 600), (600, 400), (1000, 200), (1200, 600)]);
    // The whole screen, top half: two zones.
    let (tree, at, rest) = split_part(&columns(), (0, 2), 0.5, Side::Top).unwrap();
    let cells = zones::rects(&tree, AREA, 0);
    assert_eq!((at, rest, cells[at].h, cells[rest].y), (0, 1, 500, 500));
    // The first two columns aren't one part of the tree.
    assert_eq!(split_part(&columns(), (0, 1), 0.5, Side::Left), None);
}

#[test]
fn a_divider_moves_and_the_next_one_stays() {
    let s = solve(&columns(), AREA, &edge(&[]), 100, &[]);
    assert_eq!(s.applied_delta, 100);
    assert_eq!(xs(&s.cells), vec![(0, 700), (700, 500), (1200, 600)]);
    // Grow takes the right-hand divider on a tie, by 5% of the width.
    let g = grow(&columns(), AREA, &[1], None, &[]).unwrap();
    assert_eq!(xs(&g.cells), vec![(0, 600), (600, 690), (1290, 510)]);
}

#[test]
fn a_share_puts_the_divider_at_that_part_of_the_two_zones() {
    // Three quarters of the left two columns, asked from either side.
    let left = share(&columns(), AREA, &edge(&[]), (&[0], &[1]), 0.75, &[]).unwrap();
    assert_eq!(xs(&left.cells), vec![(0, 900), (900, 300), (1200, 600)]);
    let right = share(&columns(), AREA, &edge(&[]), (&[1], &[0]), 0.25, &[]).unwrap();
    assert_eq!(right.cells, left.cells);
    // The middle window can't go below 400 px, so the next divider gives way.
    let pushed = share(&columns(), AREA, &edge(&[]), (&[0], &[1]), 0.75, &[(0, 0), (400, 0), (0, 0)]).unwrap();
    assert_eq!(xs(&pushed.cells), vec![(0, 900), (900, 400), (1300, 500)]);
    // That divider isn't between the two.
    assert_eq!(share(&columns(), AREA, &edge(&[true]), (&[0], &[1]), 0.5, &[]), None);
}

#[test]
fn a_zone_at_its_minimum_pushes_the_divider_beyond_it() {
    // The middle window can't go below 300 px, so the first divider gives way.
    let mins = [(0, 0), (300, 0), (0, 0)];
    let s = solve(&columns(), AREA, &edge(&[true]), -500, &mins);
    assert_eq!(s.applied_delta, -500);
    assert_eq!(xs(&s.cells), vec![(0, 400), (400, 300), (700, 1100)]);
}

#[test]
fn the_push_stops_at_the_area_edge() {
    let mins = [(400, 0), (300, 0), (0, 0)];
    let s = solve(&columns(), AREA, &edge(&[true]), -1100, &mins);
    assert_eq!(s.applied_delta, -500);
    assert_eq!(xs(&s.cells), vec![(0, 400), (400, 300), (700, 1100)]);
    // Already there, so nothing moves at all.
    let again = solve(&s.tree, AREA, &edge(&[true]), -50, &mins);
    assert_eq!((again.applied_delta, again.tree), (0, s.tree));
    // With no minimum the ratio clamp still holds the first zone at a tenth.
    let free = solve(&columns(), AREA, &edge(&[]), -1000, &[]);
    assert_eq!(free.cells[0].w, 180);
    // Deltas and steps far past the area don't overflow, and stop at the same place.
    assert_eq!(solve(&columns(), AREA, &edge(&[]), i32::MIN, &[]).cells, free.cells);
    let g = grow(&columns(), AREA, &[2], Some(i32::MAX), &[]).unwrap();
    // The first zone at a tenth of the width, the middle one at a tenth of the rest.
    assert_eq!(g.cells[2].w, 1800 - 180 - 162);
}

#[test]
fn splits_elsewhere_keep_their_place_and_ratio() {
    let grid = zones::template_tree(Template::Grid).unwrap();
    let s = solve(&grid, AREA, &edge(&[true]), 120, &[]);
    let before = zones::rects(&grid, AREA, 0);
    assert_eq!(s.applied_delta, 120);
    assert_eq!(&s.cells[..2], &before[..2]);
    let (Node::Split { a: old, .. }, Node::Split { a: new, .. }) = (&grid, &s.tree) else { panic!() };
    assert_eq!(old, new);
    // Moving the outer divider leaves the inner one where it is on screen.
    let s = solve(&columns(), AREA, &edge(&[]), -200, &[]);
    assert_eq!(s.cells[2].x, 1200);
}

#[test]
fn cells_always_tile_the_area() {
    let trees = [Template::Columns, Template::Rows, Template::Grid, Template::Priority]
        .map(|t| zones::template_tree(t).unwrap());
    let area = Rect { x: -1440, y: 200, w: 1440, h: 2520 };
    for tree in &trees {
        let n = zones::leaf_count(tree);
        let mins: Vec<(i32, i32)> = (0..n).map(|i| (150 + 90 * i as i32, 200)).collect();
        for d in zones::dividers(tree, area) {
            for delta in (-1500..=1500).step_by(125) {
                let s = solve(tree, area, &Edge { path: d.path.clone() }, delta, &mins);
                assert!(stored(&s.tree).iter().all(|r| (0.1..=0.9).contains(r)), "{tree:?} {delta}");
                let filled: i64 = s.cells.iter().map(|c| i64::from(c.w) * i64::from(c.h)).sum();
                assert_eq!(filled, i64::from(area.w) * i64::from(area.h), "{tree:?} {delta}");
                for (i, c) in s.cells.iter().enumerate() {
                    assert!(c.x >= area.x && c.y >= area.y && c.x + c.w <= area.x + area.w);
                    assert!(c.y + c.h <= area.y + area.h);
                    assert!(c.w >= mins[i].0 && c.h >= mins[i].1, "{tree:?} {delta}: zone {i} is {c:?}");
                }
                for (cells, gap) in [(s.cells.clone(), 0), (zones::rects(&s.tree, area, 8), 8)] {
                    assert!(cells.iter().all(|r| r.x >= area.x + gap && r.x + r.w <= area.x + area.w - gap));
                    assert!(cells.iter().all(|r| r.y >= area.y + gap && r.y + r.h <= area.y + area.h - gap));
                    for (i, a) in cells.iter().enumerate() {
                        let apart = |b: &Rect| {
                            a.x + a.w + gap <= b.x
                                || b.x + b.w + gap <= a.x
                                || a.y + a.h + gap <= b.y
                                || b.y + b.h + gap <= a.y
                        };
                        assert!(cells[i + 1..].iter().all(apart), "{tree:?} {delta}: zone {i} overlaps");
                    }
                }
            }
        }
    }
}

#[test]
fn priority_gives_two_thirds_and_restore_gives_it_back() {
    for leaf in 0..3 {
        let (tree, stash) = priority(&columns(), AREA, &[leaf], &[]);
        let w = zones::rects(&tree, AREA, 0)[leaf].w;
        assert!((w - 1200).abs() <= 2, "leaf {leaf}: {w}");
        assert_eq!(restore(stash), columns());
    }
    assert_eq!(relabel(Template::Columns, columns()).template, Template::Columns);
    let (tree, _) = priority(&columns(), AREA, &[1], &[]);
    assert_eq!(relabel(Template::Columns, tree).template, Template::Custom);
}

#[test]
fn tile_picks_a_layout_with_room_for_every_window() {
    let area = Rect { x: -1440, y: 200, w: 1440, h: 2520 };
    assert!(pick_for_count(0).is_none() && pick_for_count(1).is_none());
    for (n, zones) in [(2, 2), (3, 3), (4, 4), (5, 6), (6, 6), (7, 9), (8, 9), (9, 9)] {
        let layout = pick_for_count(n).unwrap();
        let cells = zones::rects(&layout.tree, area, 0);
        assert_eq!(cells.len(), zones, "{n} windows");
        let filled: i64 = cells.iter().map(|c| i64::from(c.w) * i64::from(c.h)).sum();
        assert_eq!(filled, i64::from(area.w) * i64::from(area.h), "{n} windows leave holes or overlap");
        assert!(cells.iter().all(|c| c.x >= area.x && c.y >= area.y && c.x + c.w <= area.x + area.w));
        assert!(cells.iter().all(|c| c.y + c.h <= area.y + area.h && c.w > 0 && c.h > 0));
        assert_eq!(layout.template == Template::Custom, n != 3 && n != 4, "{n} windows");
    }
    assert_eq!(pick_for_count(3).unwrap().template, Template::MainStack);
    assert_eq!(pick_for_count(4).unwrap().template, Template::Grid);
}

#[test]
fn windows_line_up_into_the_zones_they_stand_in() {
    // Windows as the zones would place them with an 8 px gap, on a screen left of the main one.
    let area = Rect { x: -1920, y: 0, w: 1920, h: 1040 };
    let main_stack = zones::template_tree(Template::MainStack).unwrap();
    for tree in [grid(2, 1), main_stack, grid(3, 2)] {
        let windows = zones::rects(&tree, area, 8);
        assert_eq!(from_rects(&windows, area), Some(tree.clone()), "{tree:?}");
        // In another order, and a few pixels off the marks, it's the same tree.
        let mut nudged: Vec<Rect> = windows.iter().rev().map(|r| Rect { x: r.x + 3, w: r.w - 5, ..*r }).collect();
        nudged.rotate_left(1);
        assert_eq!(from_rects(&nudged, area), Some(tree.clone()), "{tree:?} nudged");
    }
    // Off the marks, a cut sits in the middle of the room between the windows.
    let two = [Rect { x: 0, y: 0, w: 700, h: 1000 }, Rect { x: 740, y: 0, w: 1060, h: 1000 }];
    let Some(Node::Split { split: Dir::Vertical, ratio, .. }) = from_rects(&two, AREA) else { panic!() };
    assert_eq!(divider_at(AREA, Dir::Vertical, ratio), 720);
    // Where two marks are near, the nearer one wins.
    assert_eq!(ratio_at(19, Rect { x: 0, y: 0, w: 60, h: 10 }, Dir::Vertical), 1.0 / 3.0);
    // A pinwheel has no straight cut across it, and overlapping windows have none either.
    let pinwheel = [
        Rect { x: 0, y: 0, w: 1200, h: 400 },
        Rect { x: 1200, y: 0, w: 600, h: 700 },
        Rect { x: 600, y: 700, w: 1200, h: 300 },
        Rect { x: 0, y: 400, w: 600, h: 600 },
    ];
    assert_eq!(from_rects(&pinwheel, AREA), None);
    let overlapping = [Rect { x: 0, y: 0, w: 1000, h: 600 }, Rect { x: 800, y: 400, w: 1000, h: 600 }];
    assert_eq!(from_rects(&overlapping, AREA), None);
    // One window is one zone; none is nothing.
    assert_eq!(from_rects(&[AREA], AREA), Some(Node::Leaf(LeafTag::Leaf)));
    assert_eq!(from_rects(&[], AREA), None);
}
