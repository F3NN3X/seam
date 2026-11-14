//! The tests for crossings and arrangements.

use super::*;
use proptest::prelude::*;

const SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

fn link(a: usize, b: usize, side: Side, s: f64, o: f64) -> Link {
    Link { a, b, side, map: EdgeMap { s, o }, glued: false, not_on_desk: false }
}

fn live(rects: &[Rect], links: &[Link]) -> Live {
    Live { rects: rects.to_vec(), links: links.to_vec(), ..Live::default() }
}

/// `decide` for a move that must cross, stop at the edge, or be left to Windows.
/// A stop must keep the pointer on the screen it came from.
fn cross(live: &Live, prev: (i32, i32), pt: (i32, i32)) -> Option<Crossing> {
    match decide(live, prev, pt) {
        Some(Move::Cross(c)) => Some(c),
        Some(Move::Block(at)) => {
            let src = live.rects.iter().find(|r| r.contains(prev.0, prev.1));
            assert!(src.is_some_and(|r| r.contains(at.0, at.1)), "stop {at:?} off the screen, from {prev:?} to {pt:?}");
            None
        }
        None => None,
    }
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

/// `r` moved `gap` pixels further out on `side`.
fn beyond(r: Rect, side: Side, gap: i32) -> Rect {
    let (dx, dy) = match side {
        Side::Right => (gap, 0),
        Side::Left => (-gap, 0),
        Side::Bottom => (0, gap),
        Side::Top => (0, -gap),
    };
    Rect::new(r.left + dx, r.top + dy, r.width(), r.height())
}

fn along_of(side: Side, p: (i32, i32)) -> i32 {
    if side.along_is_y() { p.1 } else { p.0 }
}

// ------------------------------------------------------------- decide --

#[test]
fn decide_crosses_every_side_both_ways() {
    let a = Rect::new(0, 0, 1000, 1000);
    for side in SIDES {
        let b = place_against(&a, &a, side, 0);
        let l = live(&[a, b], &[link(0, 1, side, 0.5, 100.0)]);

        let (prev, pt) = step_out(&a, side, 500, 2);
        let c = cross(&l, prev, pt).unwrap();
        assert_eq!((c.src, c.dst, c.side), (0, 1, side));
        assert_eq!(c.to, if side.along_is_y() { (pt.0, 350) } else { (350, pt.1) }, "{side:?}");

        // The same link read backwards through its inverse.
        let (prev, pt) = step_out(&b, side.opposite(), 350, 0);
        let c = cross(&l, prev, pt).unwrap();
        assert_eq!((c.src, c.dst, c.side), (1, 0, side.opposite()));
        assert_eq!(along_of(side, c.to), 500, "{side:?}");
    }
}

#[test]
fn decide_ignores_moves_that_stay_on_a_screen() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 1.0, 0.0)]);
    assert_eq!(cross(&l, (500, 500), (999, 999)), None);
}

#[test]
fn decide_leaves_uncalibrated_sides_and_unknown_positions_alone() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 1.0, 0.0)]);
    assert_eq!(cross(&l, (500, 999), (500, 1001)), None);
    assert_eq!(cross(&l, (-50, -50), (1500, 500)), None);
    assert_eq!(cross(&live(&[a, b], &[]), (999, 500), (1001, 500)), None);
}

#[test]
fn decide_handles_an_exit_through_the_exact_corner() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 0.5, 100.0)]);
    // Equal overshoot on both axes counts as the sideways exit.
    let c = cross(&l, (998, 998), (1001, 1001)).unwrap();
    assert_eq!(c.to, (1001, 600));
}

#[test]
fn decide_prefers_the_axis_it_went_furthest_on_when_both_sides_link() {
    let a = Rect::new(0, 0, 1000, 1000);
    let right = Rect::new(1000, 0, 1000, 1000);
    let below = Rect::new(0, 1000, 1000, 1000);
    let links = [link(0, 1, Side::Right, 1.0, 0.0), link(0, 2, Side::Bottom, 1.0, 0.0)];
    let l = live(&[a, right, below], &links);
    assert_eq!(cross(&l, (995, 998), (1010, 1001)).unwrap().dst, 1);
    assert_eq!(cross(&l, (998, 995), (1001, 1010)).unwrap().dst, 2);
}

#[test]
fn decide_remaps_a_corner_exit_when_only_the_shallower_side_is_linked() {
    // B is taller, so the raw point lands on B even though the move went
    // further down than sideways. A has nothing below it.
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 2000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 0.5, 0.0)]);
    let c = cross(&l, (998, 995), (1001, 1010)).expect("the raw point is on B at the wrong height");
    assert_eq!((c.dst, c.to), (1, (1001, 500)));
}

#[test]
fn decide_keeps_overshoot_into_the_neighbour() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 0.75, 0.0)]);
    // Interpolated crossing at y = 549.75, 100 px past the edge.
    assert_eq!(cross(&l, (900, 500), (1100, 600)).unwrap().to, (1100, 412));
}

#[test]
fn decide_clamps_overshoot_past_the_far_side_of_the_neighbour() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 500, 1000);
    let c = Rect::new(1500, 0, 500, 1000);
    let links = [link(0, 1, Side::Right, 1.0, 0.0), link(1, 2, Side::Right, 1.0, 0.0)];
    let l = live(&[a, b, c], &links);
    // The raw point is on C; a single event must not skip a border.
    let hop = cross(&l, (999, 500), (1700, 500)).unwrap();
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
    assert_eq!(cross(&l, p, q).unwrap().to, (1000, 500));
    let (p, q) = step_out(&a, Side::Right, 1500, 0);
    let c = cross(&l, p, q).unwrap();
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
    assert_eq!(cross(&l, p, q), None);
}

#[test]
fn decide_returns_none_outside_the_physical_overlap() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 500);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 1.0, -300.0)]);
    for y in [0, 299, 800, 999] {
        let (p, q) = step_out(&a, Side::Right, y, 0);
        assert_eq!(cross(&l, p, q), None, "row {y}");
    }
    for (y, row) in [(300, 0), (799, 499)] {
        let (p, q) = step_out(&a, Side::Right, y, 0);
        assert_eq!(cross(&l, p, q).unwrap().to, (1000, row), "row {y}");
    }
}

#[test]
fn decide_leaves_a_glued_crossing_to_a_screen_windows_has_under_the_point() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1100, 0, 1000, 1000);
    let between = Rect::new(1000, 600, 100, 400);
    let glue = Link { glued: true, ..link(0, 1, Side::Right, 1.0, 0.0) };
    let l = live(&[a, b, between], &[glue]);
    assert_eq!(cross(&l, (999, 700), (1010, 700)), None);
    let c = cross(&l, (999, 300), (1010, 300)).unwrap();
    assert_eq!((c.dst, c.to), (1, (1110, 300)));
    // Far enough to land on the glued screen itself still lines it up.
    assert_eq!(cross(&l, (999, 300), (1150, 300)).unwrap().to, (1250, 300));
}

/// My desk: the 4K, the panel 18 px below it in Windows but touching the
/// U3477, and on the desk the panel under the 4K, glued to it.
fn panel_desk(not_on_desk: bool) -> Live {
    let rects = [Rect::new(0, 0, 3840, 2160), Rect::new(3833, 2178, 1920, 550), Rect::new(3840, -1262, 1440, 3440)];
    let glue = Link { glued: true, ..link(0, 1, Side::Bottom, 1.0, -2600.0) };
    let touch = Link { not_on_desk, ..link(2, 1, Side::Bottom, 1.0, 7.0) };
    live(&rects, &if not_on_desk { vec![glue, touch] } else { vec![glue] })
}

#[test]
fn decide_glues_across_a_screen_that_is_not_on_the_desk() {
    // Up off the panel, where the U3477 is under the raw point.
    let Some(Move::Cross(c)) = decide(&panel_desk(true), (4500, 2178), (4500, 2170)) else { panic!() };
    assert_eq!((c.dst, c.to, c.glue, c.hold), (0, (3267, 2152), Some(0), Some((4500, 2178))));
}

#[test]
fn decide_blocks_at_a_border_not_on_the_desk_where_no_glue_maps() {
    let l = panel_desk(true);
    assert_eq!(decide(&l, (5200, 2178), (5200, 2170)), Some(Move::Block((5200, 2178))));
    // Down off the U3477 too.
    assert_eq!(decide(&l, (5000, 2177), (5000, 2185)), Some(Move::Block((5000, 2177))));
}

#[test]
fn decide_still_yields_glue_to_a_screen_that_touches_on_the_desk() {
    assert_eq!(decide(&panel_desk(false), (4500, 2178), (4500, 2170)), None);
}

#[test]
fn a_glued_edge_holds_the_pointer_until_the_push_adds_up() {
    let glued =
        |link, over| Crossing { to: (0, 0), src: 0, dst: 1, side: Side::Bottom, over, glue: Some(link), hold: None };
    let push =
        |moves: &[Option<Crossing>]| moves.iter().fold((Push::default(), false), |(p, _), c| p.after(c.as_ref(), 60)).1;
    // A slow push of 5 px a move crosses on the twelfth, at 60.
    assert!(!push(&[Some(glued(0, 4)); 11]));
    assert!(push(&[Some(glued(0, 4)); 12]));
    // Sliding along the edge, or pushing on another glued edge, starts again.
    assert!(!push(&[Some(glued(0, 39)), None, Some(glued(0, 39))]));
    assert!(!push(&[Some(glued(0, 39)), Some(glued(1, 39))]));
    // A flick crosses at once.
    assert!(push(&[Some(glued(0, 59))]));
}

#[test]
fn edge_resistance_holds_the_pointer_at_any_border_until_the_push_adds_up() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let below = Rect::new(0, 1000, 1000, 1000);
    // No border is lined up, so only the resistance acts.
    let mut l = Live { edge_resistance: 10, glue_push: 60, ..live(&[a, b, below], &[]) };
    let right = |over| match decide(&l, (999, 500), (1000 + over, 500)) {
        Some(Move::Cross(c)) => c,
        m => panic!("{m:?}"),
    };
    let c = right(3);
    assert_eq!((c.src, c.dst, c.to, c.glue, c.hold), (0, 1, (1003, 500), None, Some((999, 500))));
    let need = l.push_needed(false, || false);
    let push = |moves: &[Crossing]| moves.iter().fold((Push::default(), false), |(p, _), c| p.after(Some(c), need));
    // 3 px a move holds for three moves and crosses on the fourth, at 10.
    assert!(!push(&[right(2); 3]).1);
    assert!(push(&[right(2); 4]).1);
    // Crossing somewhere else, or a move that crosses nothing, starts again.
    let down = match decide(&l, (500, 999), (500, 1004)) {
        Some(Move::Cross(c)) => c,
        m => panic!("{m:?}"),
    };
    assert_eq!(down.dst, 2);
    assert!(!push(&[right(4), down, right(4)]).1);
    let (p, _) = Push::default().after(Some(&right(4)), need);
    assert!(!p.after(None, need).0.after(Some(&right(4)), need).1);
    // A held button, or resistance at 0, crosses at once; glue keeps its own push.
    assert_eq!(l.push_needed(false, || true), 0);
    assert_eq!(l.push_needed(true, || true), 60);
    assert_eq!(l.push_needed(true, || false), 60);
    assert_eq!(Live { edge_resistance: 90, ..live(&[], &[]) }.push_needed(true, || false), 90);
    assert!(Push::default().after(Some(&right(0)), 0).1);
    // Off the bottom of B onto no screen: A's neighbour below doesn't touch B,
    // so nothing reaches past that edge and it's left to Windows.
    assert_eq!(decide(&l, (1500, 999), (1500, 1004)), None);
    // A short screen beside a tall one: off its bottom, where only the tall one
    // goes on, Ease would carry the pointer over, so it's held there too.
    let short = Rect::new(0, 0, 1000, 500);
    let tall = Live { edge_resistance: 10, ..live(&[short, b], &[]) };
    let Some(Move::Cross(c)) = decide(&tall, (500, 499), (500, 503)) else { panic!() };
    assert_eq!((c.dst, c.side, c.to, c.hold), (1, Side::Bottom, (500, 503), Some((500, 499))));
    // Out through its bottom-right corner onto the tall one, the side is the one facing it.
    let Some(Move::Cross(c)) = decide(&tall, (995, 495), (1003, 510)) else { panic!() };
    assert_eq!((c.dst, c.side, c.over), (1, Side::Right, 3));
    l.edge_resistance = 0;
    assert_eq!(decide(&l, (999, 500), (1003, 500)), None);
    assert_eq!(l.push_needed(false, || panic!("buttons asked while off")), 0);
}

#[test]
fn decide_never_acts_on_a_map_that_cannot_be_inverted() {
    // `s` of 0 can reach us from a hand-edited config.
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 0.0, 7.0)]);
    let (p, q) = step_out(&b, Side::Left, 500, 0);
    assert_eq!(cross(&l, p, q), None);
}

#[test]
fn round_trip_from_the_last_row_finds_its_way_back() {
    let a = Rect::new(0, 0, 1000, 1000);
    let b = Rect::new(1000, 0, 1000, 1000);
    let l = live(&[a, b], &[link(0, 1, Side::Right, 0.5, 0.0)]);
    let (p, q) = step_out(&a, Side::Right, 999, 0);
    let landed = cross(&l, p, q).unwrap().to;
    assert_eq!(landed, (1000, 500));
    let (p, q) = step_out(&b, Side::Left, landed.1, 0);
    assert_eq!(cross(&l, p, q).map(|c| c.to), Some((999, 999)));
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
        let out = cross(&l, p, q);
        prop_assume!(out.is_some());
        let landed = along_of(side, out.unwrap().to);

        let (p, q) = step_out(&b, side.opposite(), landed, 0);
        let back = cross(&l, p, q);
        prop_assert!(back.is_some(), "no way back from {} after leaving at {}", landed, start);
        prop_assert!((along_of(side, back.unwrap().to) - start).abs() <= 1);
    }

    #[test]
    fn pointer_always_lands_inside_the_target_monitor(
        sides in (prop::sample::select(&SIDES[..]), prop::sample::select(&SIDES[..])),
        sizes in prop::array::uniform3((400..4000i32, 400..4000i32)),
        shifts in (-1500..1500i32, -1500..1500i32),
        gaps in (0..400i32, 0..400i32),
        maps in prop::array::uniform2((0.1..10.0f64, -5000.0..5000.0f64)),
        from in (0..3usize, 0.0..1.0f64, 0.0..1.0f64),
        to in (-9000..9000i32, -9000..9000i32),
    ) {
        let r = |i: usize| Rect::new(0, 0, sizes[i].0, sizes[i].1);
        let a = r(0);
        let b = beyond(place_against(&a, &r(1), sides.0, shifts.0), sides.0, gaps.0);
        let c = beyond(place_against(&b, &r(2), sides.1, shifts.1), sides.1, gaps.1);
        prop_assume!(!c.intersects(&a));
        let rects = [a, b, c];
        let links = [
            Link { glued: gaps.0 > 0, ..link(0, 1, sides.0, maps[0].0, maps[0].1) },
            Link { glued: gaps.1 > 0, ..link(1, 2, sides.1, maps[1].0, maps[1].1) },
        ];
        let l = live(&rects, &links);
        let src = rects[from.0];
        let prev = (
            src.left + (from.1 * src.width() as f64) as i32,
            src.top + (from.2 * src.height() as f64) as i32,
        );
        if let Some(c) = cross(&l, prev, to) {
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
fn plan_refuses_to_move_a_screen_past_the_one_it_is_glued_to() {
    // B slides right under A, which would put it past the left edge of C.
    let rects = [Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000), Rect::new(1100, 2000, 1000, 1000)];
    let links = [link(0, 1, Side::Bottom, 1.0, -500.0), Link { glued: true, ..link(1, 2, Side::Right, 1.0, 0.0) }];
    assert_eq!(plan(&rects, 0, &links), Err(PlanError::BreaksGlue { a: 1, b: 2 }));
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
