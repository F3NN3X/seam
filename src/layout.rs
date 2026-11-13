//! Platform-independent decisions over the whole set of monitors: where a move
//! across a border lands, and how to rearrange monitors to match the borders.

use crate::geom::{
    EdgeMap, Rect, Side, aligned_along_origin, exits_of, facing, place_against, remap_crossing, touching,
};
use std::collections::VecDeque;

/// A calibrated border between two monitors (indices into `Live::rects`).
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Side of `a` that `b` sits on.
    pub side: Side,
    pub map: EdgeMap,
    /// The screens meet on the desk across a gap in Windows' arrangement.
    pub glued: bool,
    /// The screens touch in Windows but not on the desk, so the pointer never
    /// crosses here and the map goes unused.
    pub not_on_desk: bool,
}

impl Link {
    /// The screen past side `side` of `src` through this link, and the map from
    /// `src` to it.
    fn toward(&self, src: usize, side: Side) -> Option<(usize, EdgeMap)> {
        if self.a == src && self.side == side {
            Some((self.b, self.map))
        } else if self.b == src && self.side.opposite() == side {
            Some((self.a, self.map.inverse()))
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct Live {
    pub rects: Vec<Rect>,
    pub links: Vec<Link>,
    /// How far the pointer must push past any edge to another screen, in pixels
    /// of the screen it leaves, before it crosses. 0 is off.
    pub edge_resistance: i32,
    /// The same for a glued edge. Until then the pointer rests at the edge, so
    /// an auto-hide taskbar there can still show.
    pub glue_push: i32,
}

impl Live {
    /// How far a crossing must push past the edge. Edge resistance gives way
    /// while a mouse button is down, so drags are never slowed; `buttons` is
    /// only asked when it's on. A glued edge takes the larger of the two.
    pub fn push_needed(&self, glued: bool, buttons: impl FnOnce() -> bool) -> i32 {
        let edge = if self.edge_resistance > 0 && !buttons() { self.edge_resistance } else { 0 };
        if glued { edge.max(self.glue_push) } else { edge }
    }
}

/// A remapped move: where it lands, and the border it crossed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossing {
    pub to: (i32, i32),
    pub src: usize,
    pub dst: usize,
    pub side: Side,
    /// Pixels past the edge, 0 for the first one out.
    pub over: i32,
    /// The glued link it crosses, as an index into `Live::links`. `None` for a
    /// border that touches in Windows.
    pub glue: Option<usize>,
    /// Where the pointer rests while the crossing is held back. Always set for a
    /// glued link, and for every crossing while edge resistance is on.
    pub hold: Option<(i32, i32)>,
}

/// What to do with a move off a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Cross(Crossing),
    /// Keep the pointer here, at the edge: Windows has a screen past it that
    /// isn't there on the desk, or it's past the end of a glue.
    Block((i32, i32)),
}

/// Where a move from `prev` to `pt` should land. `None` leaves it to Windows.
pub fn decide(live: &Live, prev: (i32, i32), pt: (i32, i32)) -> Option<Move> {
    let src = live.rects.iter().position(|r| r.contains(prev.0, prev.1))?;
    let a = live.rects[src];
    let exits = exits_of(&a, prev, pt);
    // The screen under the raw point, looked up once and only when needed.
    let mut under: Option<Option<usize>> = None;
    let off_desk =
        |j: usize| live.links.iter().any(|l| l.not_on_desk && ((l.a, l.b) == (src, j) || (l.a, l.b) == (j, src)));
    for (side, along, over) in exits.into_iter().flatten() {
        for (i, l) in live.links.iter().enumerate() {
            let Some((dst, map)) = l.toward(src, side).filter(|_| !l.not_on_desk) else { continue };
            let mut hold = None;
            if l.glued {
                // A screen Windows has under the point wins over a glued one,
                // unless it isn't there on the desk.
                let on = *under.get_or_insert_with(|| live.rects.iter().position(|r| r.contains(pt.0, pt.1)));
                if on.is_some_and(|j| j != dst && !off_desk(j)) {
                    continue;
                }
                // Hold it here ourselves, never by passing the move on: Windows would
                // put it on the screen under the point, or with Ease cursor movement
                // on, jump it from a dead edge to the nearest screen.
                hold = Some(inside(&a, pt));
            } else if live.edge_resistance > 0 {
                hold = Some(inside(&a, pt));
            }
            if let Some(to) = remap_crossing(&a, &live.rects[dst], side, &map, along, over) {
                return Some(Move::Cross(Crossing { to, src, dst, side, over, glue: l.glued.then_some(i), hold }));
            }
        }
    }
    // Nothing maps the point, so a screen there that isn't on the desk stops it.
    for (side, ..) in exits.into_iter().flatten() {
        for l in live.links.iter().filter(|l| l.not_on_desk) {
            let Some((dst, _)) = l.toward(src, side) else { continue };
            if *under.get_or_insert_with(|| live.rects.iter().position(|r| r.contains(pt.0, pt.1))) == Some(dst) {
                return Some(Move::Block(inside(&a, pt)));
            }
        }
    }
    // Past the end of a glue, with no screen there in Windows, the edge is a dead
    // one. Stop the pointer ourselves: with Ease cursor movement on, Windows would
    // jump it to the nearest screen, which on the desk isn't there.
    let glued_edge = exits
        .into_iter()
        .flatten()
        .any(|(side, ..)| live.links.iter().any(|l| l.glued && !l.not_on_desk && l.toward(src, side).is_some()));
    if glued_edge && under.get_or_insert_with(|| live.rects.iter().position(|r| r.contains(pt.0, pt.1))).is_none() {
        return Some(Move::Block(inside(&a, pt)));
    }
    // With edge resistance on, a move onto any other screen is held at the edge
    // too, and then goes where Windows puts it. So is a move onto no screen beside
    // a neighbour that reaches past that edge, where Ease cursor movement would
    // carry the pointer across at once.
    if live.edge_resistance > 0 && exits[0].is_some() {
        let on = *under.get_or_insert_with(|| live.rects.iter().position(|r| r.contains(pt.0, pt.1)));
        let mut sides = exits.into_iter().flatten();
        let found = match on {
            // The exit that faces it, so a move out through a corner names the right side.
            Some(dst) if dst != src => {
                sides.clone().find(|e| facing(&a, &live.rects[dst], e.0)).or(exits[0]).map(|e| (dst, e))
            }
            Some(_) => None,
            None => sides.find_map(|e| {
                let dst = live.rects.iter().position(|r| touching(&a, r).is_some() && reaches_past(r, &a, e.0));
                dst.map(|dst| (dst, e))
            }),
        };
        if let Some((dst, (side, _, over))) = found {
            let hold = Some(inside(&a, pt));
            return Some(Move::Cross(Crossing { to: pt, src, dst, side, over, glue: None, hold }));
        }
    }
    None
}

/// `pt` moved onto the nearest pixel of `a`.
fn inside(a: &Rect, pt: (i32, i32)) -> (i32, i32) {
    (pt.0.clamp(a.left, a.right - 1), pt.1.clamp(a.top, a.bottom - 1))
}

/// Some of `r` lies past side `side` of `a`.
fn reaches_past(r: &Rect, a: &Rect, side: Side) -> bool {
    match side {
        Side::Right => r.right > a.right,
        Side::Left => r.left < a.left,
        Side::Bottom => r.bottom > a.bottom,
        Side::Top => r.top < a.top,
    }
}

/// How far the pointer has pushed past an edge, kept by the hook between moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Push {
    /// The screen the push is leaving, the one it's heading for and the glued
    /// link, if any.
    edge: Option<(usize, usize, Option<usize>)>,
    px: i32,
}

impl Push {
    /// The push after this move, and whether the move crosses now. A crossing
    /// adds its pixels past the edge and crosses once they reach `need`, from
    /// `Live::push_needed`. Anything else, such as a move that stays on the
    /// screen, slides along the edge or crosses somewhere else, starts the count
    /// again.
    pub fn after(self, c: Option<&Crossing>, need: i32) -> (Push, bool) {
        let Some(c) = c else { return (Push::default(), false) };
        if need <= 0 {
            return (Push::default(), true);
        }
        let edge = Some((c.src, c.dst, c.glue));
        // `over` counts the first pixel out as 0.
        let px = if self.edge == edge { self.px } else { 0 } + c.over + 1;
        if px >= need { (Push::default(), true) } else { (Push { edge, px }, false) }
    }
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
    /// Moving `a` would take it out from beyond the side of `b` it's glued to.
    BreaksGlue { a: usize, b: usize },
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
            Self::BreaksGlue { a, b } => {
                write!(f, "Moving screen {} would break its glue with screen {}", a + 1, b + 1)
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
    // Glued screens keep their gap, so they must still face each other afterwards.
    let glued: Vec<Link> =
        links.iter().copied().filter(|l| l.glued && facing(&rects[l.a], &rects[l.b], l.side)).collect();
    // A border between screens that don't touch in Windows, or don't on the desk, can't place either one.
    let links: Vec<Link> = links
        .iter()
        .copied()
        .filter(|l| !l.not_on_desk && touching(&rects[l.a], &rects[l.b]) == Some(l.side))
        .collect();
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
    for l in &glued {
        if !facing(&out[l.a], &out[l.b], l.side) {
            let (moved, other) = if out[l.a] == rects[l.a] { (l.b, l.a) } else { (l.a, l.b) };
            return Err(PlanError::BreaksGlue { a: moved, b: other });
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
                assert!(
                    src.is_some_and(|r| r.contains(at.0, at.1)),
                    "stop {at:?} off the screen, from {prev:?} to {pt:?}"
                );
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
        let glued = |link, over| Crossing {
            to: (0, 0),
            src: 0,
            dst: 1,
            side: Side::Bottom,
            over,
            glue: Some(link),
            hold: None,
        };
        let push = |moves: &[Option<Crossing>]| {
            moves.iter().fold((Push::default(), false), |(p, _), c| p.after(c.as_ref(), 60)).1
        };
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
}
