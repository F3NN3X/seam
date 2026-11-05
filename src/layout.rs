//! Platform-independent decisions over the whole set of monitors: where a move
//! across a border lands, and how to rearrange monitors to match the borders.

use crate::geom::{aligned_along_origin, exit_of, place_against, remap_crossing, touching, EdgeMap, Rect, Side};
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

/// Where a move from `prev` to `pt` should land. `None` leaves it to Windows.
pub fn decide(live: &Live, prev: (i32, i32), pt: (i32, i32)) -> Option<(i32, i32)> {
    let src = live.rects.iter().position(|r| r.contains(prev.0, prev.1))?;
    let a = live.rects[src];
    let (side, along, over) = exit_of(&a, prev, pt)?;
    for l in &live.links {
        let (dst, map) = if l.a == src && l.side == side {
            (l.b, l.map)
        } else if l.b == src && l.side.opposite() == side {
            (l.a, l.map.inverse())
        } else {
            continue;
        };
        if let Some(p) = remap_crossing(&a, &live.rects[dst], side, &map, along, over) {
            return Some(p);
        }
    }
    None
}

/// New rects for every monitor, moving each neighbour only along its shared
/// edge, starting from the primary (which Windows pins at 0,0).
pub fn plan(rects: &[Rect], primary: usize, links: &[Link]) -> Result<Vec<Rect>, String> {
    if rects.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = rects.to_vec();
    let mut placed = vec![false; rects.len()];
    let mut queue = VecDeque::from([primary]);
    placed[primary] = true;
    while let Some(m) = queue.pop_front() {
        for l in links {
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
            if out[i].intersects(&out[j]) {
                return Err("aligning would make screens overlap".into());
            }
            if touching(&rects[i], &rects[j]).is_some() && touching(&out[i], &out[j]).is_none() {
                return Err("this layout is too complex to align automatically".into());
            }
        }
    }
    Ok(out)
}
