//! Platform-independent decisions over the whole set of monitors: where a move
//! across a border lands, and how to rearrange monitors to match the borders.

use crate::geom::{aligned_along_origin, exits_of, place_against, remap_crossing, touching, EdgeMap, Rect, Side};
use std::collections::VecDeque;

/// A calibrated border between two monitors (indices into `Live::rects`).
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Side of `a` that `b` sits on.
    pub side: Side,
    pub map: EdgeMap,
}

#[derive(Default)]
pub struct Live {
    pub rects: Vec<Rect>,
    pub links: Vec<Link>,
}

/// A remapped move: where it lands, and the border it crossed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossing {
    pub to: (i32, i32),
    pub src: usize,
    pub dst: usize,
    pub side: Side,
}

/// Where a move from `prev` to `pt` should land. `None` leaves it to Windows.
pub fn decide(live: &Live, prev: (i32, i32), pt: (i32, i32)) -> Option<Crossing> {
    let src = live.rects.iter().position(|r| r.contains(prev.0, prev.1))?;
    let a = live.rects[src];
    for (side, along, over) in exits_of(&a, prev, pt).into_iter().flatten() {
        for l in &live.links {
            let (dst, map) = if l.a == src && l.side == side {
                (l.b, l.map)
            } else if l.b == src && l.side.opposite() == side {
                (l.a, l.map.inverse())
            } else {
                continue;
            };
            if let Some(to) = remap_crossing(&a, &live.rects[dst], side, &map, along, over) {
                return Some(Crossing { to, src, dst, side });
            }
        }
    }
    None
}

/// Why `plan` refused an arrangement. Indices are into the monitor list; the
/// text shows them as screen numbers, counting from 1.
#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Moving `a` would put it on top of `b`.
    Overlap { a: usize, b: usize },
    /// Moving `moved` would take it away from `other`, which it touches now.
    Separates { moved: usize, other: usize },
    /// The border between `a` and `b` can't hold alongside the others.
    Unhonoured { a: usize, b: usize },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Overlap { a, b } => write!(f, "Moving screen {} would overlap screen {}", a + 1, b + 1),
            Self::Separates { moved, other } => {
                write!(f, "Moving screen {} would break its border with screen {}", moved + 1, other + 1)
            }
            Self::Unhonoured { a, b } => {
                write!(f, "The border between screen {} and screen {} can't hold with the others", a + 1, b + 1)
            }
        }
    }
}

/// New rects for every monitor, moving each neighbour only along its shared
/// edge, starting from the primary (which Windows pins at 0,0).
pub fn plan(rects: &[Rect], primary: usize, links: &[Link]) -> Result<Vec<Rect>, PlanError> {
    if rects.is_empty() {
        return Ok(Vec::new());
    }
    // A border between screens that don't touch in Windows can't place either one.
    let links: Vec<Link> =
        links.iter().copied().filter(|l| touching(&rects[l.a], &rects[l.b]) == Some(l.side)).collect();
    let mut out = rects.to_vec();
    let mut placed = vec![false; rects.len()];
    let mut queue = VecDeque::from([primary]);
    placed[primary] = true;
    while let Some(m) = queue.pop_front() {
        for l in &links {
            let (n, side, map) = if l.a == m && !placed[l.b] {
                (l.b, l.side, l.map)
            } else if l.b == m && !placed[l.a] {
                (l.a, l.side.opposite(), l.map.inverse())
            } else {
                continue;
            };
            if let Some(origin) = aligned_along_origin(&out[m], &out[n], side, &map) {
                out[n] = place_against(&out[m], &out[n], side, origin);
            }
            placed[n] = true;
            queue.push_back(n);
        }
    }
    // Every pair that touched before must still touch, and nothing may overlap.
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            // Name the screen that moved first; when both did, the lower one.
            let (moved, other) = if out[i] == rects[i] && out[j] != rects[j] { (j, i) } else { (i, j) };
            if out[i].intersects(&out[j]) {
                return Err(PlanError::Overlap { a: moved, b: other });
            }
            if touching(&rects[i], &rects[j]).is_some() && touching(&out[i], &out[j]).is_none() {
                return Err(PlanError::Separates { moved, other });
            }
        }
    }
    // Placing each screen follows only one border, so a loop of borders can
    // leave the last one misaligned. A rounding pixel is fine.
    for l in &links {
        let origin = out[l.b].along_origin(l.side);
        if aligned_along_origin(&out[l.a], &out[l.b], l.side, &l.map).is_some_and(|want| (want - origin).abs() > 1) {
            return Err(PlanError::Unhonoured { a: l.a, b: l.b });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

    fn link(a: usize, b: usize, side: Side, s: f64, o: f64) -> Link {
        Link { a, b, side, map: EdgeMap { s, o } }
    }

    fn live(rects: &[Rect], links: &[Link]) -> Live {
        Live { rects: rects.to_vec(), links: links.to_vec() }
    }

    /// A pointer step that leaves `r` through `side` at absolute coordinate
    /// `along`, moving `past` pixels beyond the edge (0 = the first pixel out).
    fn step_out(r: &Rect, side: Side, along: i32, past: i32) -> ((i32, i32), (i32, i32)) {
        match side {
            Side::Right => ((r.right - 1, along), (r.right + past, along)),
            Side::Left => ((r.left, along), (r.left - 1 - past, along)),
            Side::Bottom => ((along, r.bottom - 1), (along, r.bottom + past)),
            Side::Top => ((along, r.top), (along, r.top - 1 - past)),
        }
    }

    fn along_of(side: Side, p: (i32, i32)) -> i32 {
        if side.along_is_y() {
            p.1
        } else {
            p.0
        }
    }

    // ------------------------------------------------------------- decide --

    #[test]
    fn decide_crosses_every_side_both_ways() {
        let a = Rect::new(0, 0, 1000, 1000);
        for side in SIDES {
            let b = place_against(&a, &a, side, 0);
            let l = live(&[a, b], &[link(0, 1, side, 0.5, 100.0)]);

            let (prev, pt) = step_out(&a, side, 500, 2);
            let c = decide(&l, prev, pt).unwrap();
            assert_eq!((c.src, c.dst, c.side), (0, 1, side));
            assert_eq!(c.to, if side.along_is_y() { (pt.0, 350) } else { (350, pt.1) }, "{side:?}");

            // The same link read backwards through its inverse.
            let (prev, pt) = step_out(&b, side.opposite(), 350, 0);
            let c = decide(&l, prev, pt).unwrap();
            assert_eq!((c.src, c.dst, c.side), (1, 0, side.opposite()));
            assert_eq!(along_of(side, c.to), 500, "{side:?}");
        }
    }

    #[test]
    fn decide_ignores_moves_that_stay_on_a_screen() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 1000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 1.0, 0.0)]);
        assert_eq!(decide(&l, (500, 500), (999, 999)), None);
    }

    #[test]
    fn decide_leaves_uncalibrated_sides_and_unknown_positions_alone() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 1000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 1.0, 0.0)]);
        assert_eq!(decide(&l, (500, 999), (500, 1001)), None);
        assert_eq!(decide(&l, (-50, -50), (1500, 500)), None);
        assert_eq!(decide(&live(&[a, b], &[]), (999, 500), (1001, 500)), None);
    }

    #[test]
    fn decide_handles_an_exit_through_the_exact_corner() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 1000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 0.5, 100.0)]);
        // Equal overshoot on both axes counts as the sideways exit.
        let c = decide(&l, (998, 998), (1001, 1001)).unwrap();
        assert_eq!(c.to, (1001, 600));
    }

    #[test]
    fn decide_prefers_the_axis_it_went_furthest_on_when_both_sides_link() {
        let a = Rect::new(0, 0, 1000, 1000);
        let right = Rect::new(1000, 0, 1000, 1000);
        let below = Rect::new(0, 1000, 1000, 1000);
        let links = [link(0, 1, Side::Right, 1.0, 0.0), link(0, 2, Side::Bottom, 1.0, 0.0)];
        let l = live(&[a, right, below], &links);
        assert_eq!(decide(&l, (995, 998), (1010, 1001)).unwrap().dst, 1);
        assert_eq!(decide(&l, (998, 995), (1001, 1010)).unwrap().dst, 2);
    }

    #[test]
    fn decide_remaps_a_corner_exit_when_only_the_shallower_side_is_linked() {
        // B is taller, so the raw point lands on B even though the move went
        // further down than sideways. A has nothing below it.
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 2000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 0.5, 0.0)]);
        let c = decide(&l, (998, 995), (1001, 1010)).expect("the raw point is on B at the wrong height");
        assert_eq!((c.dst, c.to), (1, (1001, 500)));
    }

    #[test]
    fn decide_keeps_overshoot_into_the_neighbour() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 1000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 0.75, 0.0)]);
        // Interpolated crossing at y = 549.75, 100 px past the edge.
        assert_eq!(decide(&l, (900, 500), (1100, 600)).unwrap().to, (1100, 412));
    }

    #[test]
    fn decide_clamps_overshoot_past_the_far_side_of_the_neighbour() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 500, 1000);
        let c = Rect::new(1500, 0, 500, 1000);
        let links = [link(0, 1, Side::Right, 1.0, 0.0), link(1, 2, Side::Right, 1.0, 0.0)];
        let l = live(&[a, b, c], &links);
        // The raw point is on C; a single event must not skip a border.
        let hop = decide(&l, (999, 500), (1700, 500)).unwrap();
        assert_eq!((hop.dst, hop.to), (1, (1499, 500)));
    }

    #[test]
    fn decide_picks_the_right_neighbour_of_two_on_one_side() {
        let a = Rect::new(0, 0, 1000, 2000);
        let top = Rect::new(1000, 0, 1000, 1000);
        let bottom = Rect::new(1000, 1000, 1000, 1000);
        let links = [link(0, 1, Side::Right, 1.0, 0.0), link(0, 2, Side::Right, 1.0, -1000.0)];
        let l = live(&[a, top, bottom], &links);
        let (p, q) = step_out(&a, Side::Right, 500, 0);
        assert_eq!(decide(&l, p, q).unwrap().to, (1000, 500));
        let (p, q) = step_out(&a, Side::Right, 1500, 0);
        let c = decide(&l, p, q).unwrap();
        assert_eq!((c.dst, c.to), (2, (1000, 1500)));
    }

    #[test]
    fn decide_leaves_a_physical_gap_between_two_neighbours_alone() {
        let a = Rect::new(0, 0, 1000, 2000);
        let top = Rect::new(1000, 0, 1000, 1000);
        let bottom = Rect::new(1000, 1000, 1000, 1000);
        let links = [link(0, 1, Side::Right, 1.0, 0.0), link(0, 2, Side::Right, 1.0, -1200.0)];
        let l = live(&[a, top, bottom], &links);
        let (p, q) = step_out(&a, Side::Right, 1100, 0);
        assert_eq!(decide(&l, p, q), None);
    }

    #[test]
    fn decide_returns_none_outside_the_physical_overlap() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 500);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 1.0, -300.0)]);
        for y in [0, 299, 800, 999] {
            let (p, q) = step_out(&a, Side::Right, y, 0);
            assert_eq!(decide(&l, p, q), None, "row {y}");
        }
        for (y, row) in [(300, 0), (799, 499)] {
            let (p, q) = step_out(&a, Side::Right, y, 0);
            assert_eq!(decide(&l, p, q).unwrap().to, (1000, row), "row {y}");
        }
    }

    #[test]
    fn decide_never_acts_on_a_map_that_cannot_be_inverted() {
        // `s` of 0 can reach us from a hand-edited config.
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 1000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 0.0, 7.0)]);
        let (p, q) = step_out(&b, Side::Left, 500, 0);
        assert_eq!(decide(&l, p, q), None);
    }

    #[test]
    fn round_trip_from_the_last_row_finds_its_way_back() {
        let a = Rect::new(0, 0, 1000, 1000);
        let b = Rect::new(1000, 0, 1000, 1000);
        let l = live(&[a, b], &[link(0, 1, Side::Right, 0.5, 0.0)]);
        let (p, q) = step_out(&a, Side::Right, 999, 0);
        let landed = decide(&l, p, q).unwrap().to;
        assert_eq!(landed, (1000, 500));
        let (p, q) = step_out(&b, Side::Left, landed.1, 0);
        assert_eq!(decide(&l, p, q).map(|c| c.to), Some((999, 999)));
    }

    proptest! {
        #[test]
        fn round_trip_returns_within_a_pixel(
            side in prop::sample::select(&SIDES[..]),
            len_a in 200..5000i32,
            len_b in 200..5000i32,
            s in 0.5..2.0f64,
            o in -2000.0..2000.0f64,
            at in prop_oneof![Just(0.0), Just(1.0), 0.0..1.0f64],
        ) {
            let a = Rect::new(0, 0, len_a, len_a);
            let b = place_against(&a, &Rect::new(0, 0, len_b, len_b), side, 0);
            let l = live(&[a, b], &[link(0, 1, side, s, o)]);
            let start = ((at * len_a as f64) as i32).min(len_a - 1);

            let (p, q) = step_out(&a, side, start, 0);
            let out = decide(&l, p, q);
            prop_assume!(out.is_some());
            let landed = along_of(side, out.unwrap().to);

            let (p, q) = step_out(&b, side.opposite(), landed, 0);
            let back = decide(&l, p, q);
            prop_assert!(back.is_some(), "no way back from {} after leaving at {}", landed, start);
            prop_assert!((along_of(side, back.unwrap().to) - start).abs() <= 1);
        }

        #[test]
        fn pointer_always_lands_inside_the_target_monitor(
            sides in (prop::sample::select(&SIDES[..]), prop::sample::select(&SIDES[..])),
            sizes in prop::array::uniform3((400..4000i32, 400..4000i32)),
            shifts in (-1500..1500i32, -1500..1500i32),
            maps in prop::array::uniform2((0.1..10.0f64, -5000.0..5000.0f64)),
            from in (0..3usize, 0.0..1.0f64, 0.0..1.0f64),
            to in (-9000..9000i32, -9000..9000i32),
        ) {
            let r = |i: usize| Rect::new(0, 0, sizes[i].0, sizes[i].1);
            let a = r(0);
            let b = place_against(&a, &r(1), sides.0, shifts.0);
            let c = place_against(&b, &r(2), sides.1, shifts.1);
            prop_assume!(!c.intersects(&a));
            let rects = [a, b, c];
            let links = [
                link(0, 1, sides.0, maps[0].0, maps[0].1),
                link(1, 2, sides.1, maps[1].0, maps[1].1),
            ];
            let l = live(&rects, &links);
            let src = rects[from.0];
            let prev = (
                src.left + (from.1 * src.width() as f64) as i32,
                src.top + (from.2 * src.height() as f64) as i32,
            );
            if let Some(c) = decide(&l, prev, to) {
                prop_assert_ne!(c.src, c.dst);
                prop_assert!(rects[c.dst].contains(c.to.0, c.to.1), "{:?} outside {:?}", c.to, rects[c.dst]);
            }
        }

        #[test]
        fn plan_keeps_primary_and_sizes_and_never_overlaps(
            sides in (prop::sample::select(&SIDES[..]), prop::sample::select(&SIDES[..])),
            sizes in prop::array::uniform3((400..4000i32, 400..4000i32)),
            maps in prop::array::uniform2((0.25..4.0f64, -1000.0..1000.0f64)),
        ) {
            let r = |i: usize| Rect::new(0, 0, sizes[i].0, sizes[i].1);
            let a = r(0);
            let b = place_against(&a, &r(1), sides.0, 0);
            let c = place_against(&b, &r(2), sides.1, 0);
            prop_assume!(!c.intersects(&a));
            let rects = [a, b, c];
            let links = [
                link(0, 1, sides.0, maps[0].0, maps[0].1),
                link(1, 2, sides.1, maps[1].0, maps[1].1),
            ];
            if let Ok(out) = plan(&rects, 0, &links) {
                prop_assert_eq!(out[0], a);
                for (o, r) in out.iter().zip(&rects) {
                    prop_assert_eq!((o.width(), o.height()), (r.width(), r.height()));
                }
                for i in 0..3 {
                    for j in (i + 1)..3 {
                        prop_assert!(!out[i].intersects(&out[j]));
                    }
                }
                prop_assert_eq!(plan(&out, 0, &links).unwrap(), out, "planning twice must not move anything");
            }
        }
    }

    // --------------------------------------------------------------- plan --

    #[test]
    fn plan_two_screens() {
        let rects = [Rect::new(0, 0, 3840, 2160), Rect::new(3840, 0, 2560, 1440)];
        let out = plan(&rects, 0, &[link(0, 1, Side::Right, 2.0 / 3.0, 0.0)]).unwrap();
        assert_eq!(out, [rects[0], Rect::new(3840, 360, 2560, 1440)]);
    }

    #[test]
    fn plan_three_in_a_row_chains_offsets() {
        let rects = [Rect::new(0, 0, 1000, 1000), Rect::new(1000, 0, 1000, 1000), Rect::new(2000, 0, 1000, 1000)];
        let links = [link(0, 1, Side::Right, 1.0, -100.0), link(1, 2, Side::Right, 1.0, -100.0)];
        let out = plan(&rects, 0, &links).unwrap();
        assert_eq!(out, [rects[0], Rect::new(1000, 100, 1000, 1000), Rect::new(2000, 200, 1000, 1000)]);
    }

    #[test]
    fn plan_l_shape() {
        let rects = [Rect::new(0, 0, 1000, 1000), Rect::new(1000, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)];
        let links = [link(0, 1, Side::Right, 1.0, -100.0), link(0, 2, Side::Bottom, 1.0, 50.0)];
        let out = plan(&rects, 0, &links).unwrap();
        assert_eq!(out, [rects[0], Rect::new(1000, 100, 1000, 1000), Rect::new(-50, 1000, 1000, 1000)]);
    }

    fn grid() -> [Rect; 4] {
        [
            Rect::new(0, 0, 1000, 1000),
            Rect::new(1000, 0, 1000, 1000),
            Rect::new(0, 1000, 1000, 1000),
            Rect::new(1000, 1000, 1000, 1000),
        ]
    }

    #[test]
    fn plan_accepts_a_grid_whose_borders_all_agree() {
        let links = [
            link(0, 1, Side::Right, 1.0, 0.0),
            link(0, 2, Side::Bottom, 1.0, 0.0),
            link(1, 3, Side::Bottom, 1.0, 0.0),
            link(2, 3, Side::Right, 1.0, 0.0),
        ];
        assert_eq!(plan(&grid(), 0, &links).unwrap(), grid());
    }

    #[test]
    fn plan_refuses_a_grid_that_would_separate_screens() {
        let links = [
            link(0, 1, Side::Right, 1.0, 0.0),
            link(0, 2, Side::Bottom, 1.0, 0.0),
            link(1, 3, Side::Bottom, 1.0, -100.0),
            link(2, 3, Side::Right, 1.0, 0.0),
        ];
        assert_eq!(plan(&grid(), 0, &links), Err(PlanError::Separates { moved: 3, other: 2 }));
    }

    #[test]
    fn plan_refuses_a_grid_that_would_overlap_screens() {
        let links = [
            link(0, 1, Side::Right, 1.0, 0.0),
            link(0, 2, Side::Bottom, 1.0, 0.0),
            link(1, 3, Side::Bottom, 1.0, 100.0),
            link(2, 3, Side::Right, 1.0, 0.0),
        ];
        assert_eq!(plan(&grid(), 0, &links), Err(PlanError::Overlap { a: 3, b: 2 }));
    }

    #[test]
    fn plan_refuses_a_grid_when_one_border_cannot_be_honoured() {
        // B moves down 100, D follows it, and the screens still touch, but the
        // C-D border wants D level with C. No arrangement satisfies all four.
        let links = [
            link(0, 1, Side::Right, 1.0, -100.0),
            link(0, 2, Side::Bottom, 1.0, 0.0),
            link(1, 3, Side::Bottom, 1.0, 0.0),
            link(2, 3, Side::Right, 1.0, 0.0),
        ];
        assert_eq!(plan(&grid(), 0, &links), Err(PlanError::Unhonoured { a: 2, b: 3 }));
    }

    #[test]
    fn plan_when_primary_is_not_leftmost() {
        let rects = [Rect::new(-2560, 0, 2560, 1440), Rect::new(0, 0, 3840, 2160)];
        let out = plan(&rects, 1, &[link(0, 1, Side::Right, 1.5, 0.0)]).unwrap();
        assert_eq!(out, [Rect::new(-2560, 360, 2560, 1440), rects[1]]);
    }

    #[test]
    fn plan_with_negative_coordinates() {
        let rects = [Rect::new(0, 0, 3840, 2160), Rect::new(596, -1440, 2560, 1440)];
        let out = plan(&rects, 0, &[link(1, 0, Side::Bottom, 1.0, -300.0)]).unwrap();
        assert_eq!(out, [rects[0], Rect::new(-300, -1440, 2560, 1440)]);
    }

    #[test]
    fn plan_portrait_next_to_landscape() {
        let rects = [Rect::new(0, 0, 2560, 1440), Rect::new(2560, 0, 1440, 2560)];
        let out = plan(&rects, 0, &[link(0, 1, Side::Right, 1.0, -560.0)]).unwrap();
        assert_eq!(out, [rects[0], Rect::new(2560, 560, 1440, 2560)]);
    }

    #[test]
    fn plan_does_nothing_without_screens_or_borders() {
        assert_eq!(plan(&[], 0, &[]).unwrap(), Vec::<Rect>::new());
        let rects = [Rect::new(0, 0, 1000, 1000), Rect::new(1000, 0, 1000, 1000)];
        assert_eq!(plan(&rects, 0, &[]).unwrap(), rects);
    }

    #[test]
    fn plan_refuses_a_real_desk_where_a_chained_screen_would_hit_the_primary() {
        // Screen 3 moves up 229 px, taking screen 2 below it, and screen 2 hangs
        // 7 px past the corner of the primary, so they would collide.
        let rects = [
            Rect::new(0, 0, 3840, 2160),
            Rect::new(596, -1440, 2560, 1440),
            Rect::new(3833, 2178, 1920, 550),
            Rect::new(3840, -1262, 1440, 3440),
        ];
        let links = [
            link(1, 0, Side::Bottom, 1.4892473118279572, 28.763440860214928),
            link(0, 3, Side::Right, 0.7870036101083029, 1721.036101083033),
            link(3, 2, Side::Bottom, 1.0, 7.0),
        ];
        assert_eq!(plan(&rects, 0, &links), Err(PlanError::Overlap { a: 2, b: 0 }));
    }

    #[test]
    fn plan_errors_name_screens_counting_from_one() {
        assert_eq!(PlanError::Overlap { a: 2, b: 0 }.to_string(), "Moving screen 3 would overlap screen 1");
        assert_eq!(
            PlanError::Separates { moved: 3, other: 2 }.to_string(),
            "Moving screen 4 would break its border with screen 3"
        );
        assert_eq!(
            PlanError::Unhonoured { a: 2, b: 3 }.to_string(),
            "The border between screen 3 and screen 4 can't hold with the others"
        );
    }
}
