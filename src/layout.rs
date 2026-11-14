//! Platform-independent decisions over the whole set of monitors: where a move
//! across a border lands, and how to rearrange monitors to match the borders.

use crate::geom::{
    EdgeMap, Rect, Side, aligned_along_origin, exits_of, facing, place_against, remap_crossing, touching,
};
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

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
