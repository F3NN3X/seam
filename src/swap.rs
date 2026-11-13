//! Dropping a window into a zone another snapped window holds: the windows there
//! take the dropped window's old place instead of lying under it. A child of
//! `snap`, so it reads the engine and the drag. The decision is one pass over the
//! snap registry on the main thread, and the moves go through window control's
//! queue, so they land one after another like any other placement.

use super::*;
use crate::wincontrol;

/// A mapped rect this close to a span of zones counts as being that span.
const MATCH_SLOP: i32 = 12;

/// Where an occupant goes.
enum Dest<'a> {
    /// The zones the dropped window was snapped to, on their screen.
    Zones(&'a Placed, usize, usize, Rect),
    /// The dropped window's rect before the drag, and the DPI of the screen it was on.
    Rect(Rect, u32),
}

/// Why a drop doesn't swap, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Skip {
    /// "Swap with the window already in a zone" is off.
    Off,
    /// No place to send them: the old zones or the old frame aren't known.
    NoPlace,
    /// The window was dropped over its own zones.
    OwnSpan,
    NoOccupants,
    /// The snapping has windows in those zones, but none is still mostly there.
    NotCurrent,
    /// One of them can't be moved or isn't responding.
    Refused,
}

/// How far each rect is shrunk before two count as overlapping, so zones that
/// only share an edge, or round a pixel into each other, don't.
const EDGE_SLOP: i32 = 2;

fn overlaps(a: Rect, b: Rect) -> bool {
    let shrink =
        |r: Rect| Rect { x: r.x + EDGE_SLOP, y: r.y + EDGE_SLOP, w: r.w - 2 * EDGE_SLOP, h: r.h - 2 * EDGE_SLOP };
    let (a, b) = (shrink(a), shrink(b));
    a.w > 0 && a.h > 0 && b.w > 0 && b.h > 0 && a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Whether at least half of `r` lies inside `zone`. A window an app keeps a few
/// pixels off its zones, or bigger than them for its minimum size, still counts.
fn mostly_inside(r: Rect, zone: Rect) -> bool {
    let w = (r.x + r.w).min(zone.x + zone.w) - r.x.max(zone.x);
    let h = (r.y + r.h).min(zone.y + zone.h) - r.y.max(zone.y);
    let area = |w: i32, h: i32| i64::from(w.max(0)) * i64::from(h.max(0));
    area(r.w, r.h) > 0 && 2 * area(w, h) >= area(r.w, r.h)
}

/// Where `r`, the part of it inside `from`, lands when `from` is stretched over `to`.
fn map_rect(r: Rect, from: Rect, to: Rect) -> Rect {
    let (x0, y0) = (r.x.max(from.x), r.y.max(from.y));
    let (x1, y1) = ((r.x + r.w).min(from.x + from.w), (r.y + r.h).min(from.y + from.h));
    let (kx, ky) = (to.w as f64 / from.w.max(1) as f64, to.h as f64 / from.h.max(1) as f64);
    Rect {
        x: to.x + ((x0 - from.x) as f64 * kx).round() as i32,
        y: to.y + ((y0 - from.y) as f64 * ky).round() as i32,
        w: (((x1 - x0).max(0) as f64 * kx).round() as i32).max(1),
        h: (((y1 - y0).max(0) as f64 * ky).round() as i32).max(1),
    }
}

/// The first and last of `placed` whose bounding box is `r`, give or take a few pixels.
fn zones_of(placed: &[Rect], r: Rect) -> Option<(usize, usize)> {
    let near =
        |a: Rect| [(a.x, r.x), (a.y, r.y), (a.w, r.w), (a.h, r.h)].iter().all(|(p, q)| (p - q).abs() <= MATCH_SLOP);
    (0..placed.len())
        .flat_map(|i| (i..placed.len()).map(move |j| (i, j)))
        .find(|(i, j)| near(zones::span(placed[*i], placed[*j])))
}

impl Engine {
    /// The snapped windows in zones of `g` that overlap `target`, and where each was
    /// put. A window counts while at least half of it is still inside its zones.
    /// None when one of them can't be moved or is hung, which leaves the drop as it
    /// always was.
    pub(super) fn occupants(&self, drag: &Drag, g: &Placed, target: Rect) -> Option<Vec<(HWND, Rect)>> {
        match self.held_in(drag, g, target) {
            Ok(list) => Some(list),
            Err(Skip::NoOccupants | Skip::NotCurrent) => Some(Vec::new()),
            Err(_) => None,
        }
    }

    fn held_in(&self, drag: &Drag, g: &Placed, target: Rect) -> Result<Vec<(HWND, Rect)>, Skip> {
        let (mut occupants, mut gone) = (Vec::new(), 0);
        for (key, s) in &self.snapped {
            if *key == drag.hwnd || s.screen != g.id || s.from.max(s.to) >= g.zones.len() {
                continue;
            }
            let zones = zones::span(g.zones[s.from], g.zones[s.to]);
            if !overlaps(zones, target) {
                continue;
            }
            let h = hwnd_of(*key);
            let there = unsafe { IsWindow(Some(h)) }.as_bool() && unsafe { IsWindowVisible(h) }.as_bool();
            if !there || winutil::is_minimized(h) || winutil::is_cloaked(h) {
                continue;
            }
            if !winutil::visible_rect(h).is_some_and(|r| mostly_inside(r, zones)) {
                gone += 1;
                continue;
            }
            wincontrol::movable(h, &self.exclude).map_err(|_| Skip::Refused)?;
            occupants.push((h, s.at));
        }
        match (occupants.is_empty(), gone) {
            (true, 0) => Err(Skip::NoOccupants),
            (true, _) => Err(Skip::NotCurrent),
            _ => Ok(occupants),
        }
    }

    /// The moves that give the windows in the zones `(a, b)` of screen `screen` the
    /// dropped window's old place, and whether it was snapped. None when nothing
    /// swaps, and the drop is as it always was.
    pub(super) fn swap_plan(&self, drag: &Drag, screen: usize, ab: (usize, usize)) -> Option<(Vec<Placement>, bool)> {
        self.swap_why(drag, screen, ab).ok()
    }

    /// `swap_plan`, or why there is no swap.
    pub(super) fn swap_why(
        &self,
        drag: &Drag,
        screen: usize,
        (a, b): (usize, usize),
    ) -> Result<(Vec<Placement>, bool), Skip> {
        if !self.settings.swap_on_drop {
            return Err(Skip::Off);
        }
        let g = &drag.geometry[screen];
        let zone = |z: &Placed, i: usize| z.zones.get(i).copied().ok_or(Skip::NoPlace);
        let target = zones::span(zone(g, a)?, zone(g, b)?);
        let dest = match &drag.old_zone {
            Some((id, from, to)) => {
                let old = drag.geometry.iter().find(|x| x.id == *id).ok_or(Skip::NoPlace)?;
                let span = zones::span(zone(old, *from)?, zone(old, *to)?);
                // Growing or shrinking over its own area isn't a swap.
                if old.id == g.id && overlaps(span, target) {
                    return Err(Skip::OwnSpan);
                }
                Dest::Zones(old, *from, *to, span)
            }
            None => {
                let frame = drag.frame.filter(|_| !drag.was_maximized).ok_or(Skip::NoPlace)?;
                let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
                let on = self.screens.iter().find(|s| s.bounds.contains(x, y)).ok_or(Skip::NoPlace)?;
                Dest::Rect(frame, work_area(on.bounds).ok_or(Skip::NoPlace)?.1)
            }
        };
        let occupants = self.held_in(drag, g, target)?;
        let alone = occupants.len() == 1;
        let place = |(h, at): &(HWND, Rect)| {
            let hwnd = h.0 as isize;
            match &dest {
                Dest::Zones(old, from, to, span) => {
                    let zone = |f: usize, t: usize| Some((old.id.clone(), f, t));
                    if alone {
                        return Placement { hwnd, target: *span, zone: zone(*from, *to), dpi: old.dpi };
                    }
                    let mut mapped = map_rect(*at, target, *span);
                    match zones_of(&old.zones, mapped) {
                        Some((f, t)) => {
                            let target = zones::span(old.zones[f], old.zones[t]);
                            Placement { hwnd, target, zone: zone(f, t), dpi: old.dpi }
                        }
                        None => {
                            let (w, ht) = minsize::cached(*h, old.dpi);
                            mapped.w = mapped.w.max(w);
                            mapped.h = mapped.h.max(ht);
                            Placement { hwnd, target: mapped, zone: None, dpi: old.dpi }
                        }
                    }
                }
                Dest::Rect(frame, dpi) => {
                    let mut mapped = if alone { *frame } else { map_rect(*at, target, *frame) };
                    let (w, ht) = minsize::cached(*h, *dpi);
                    mapped.w = mapped.w.max(w);
                    mapped.h = mapped.h.max(ht);
                    Placement { hwnd, target: mapped, zone: None, dpi: *dpi }
                }
            }
        };
        Ok((occupants.iter().map(place).collect(), drag.old_zone.is_some()))
    }

    /// Start the moves `swap_plan` made, once the dropped window has been sent to its zones.
    pub(super) fn swap_start(&mut self, plan: Option<(Vec<Placement>, bool)>) {
        let Some((list, snapped)) = plan else { return };
        let was = if snapped { "snapped" } else { "loose" };
        log::info!("zone swap: moved {} window(s), the dropped window was {was}", list.len());
        self.queued.extend(list);
        self.window_pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn occupants_keep_their_place_in_the_old_span() {
        // The left half of a 1000x500 span lands on the left half of a 400x200 one.
        let m = map_rect(r(0, 0, 500, 500), r(0, 0, 1000, 500), r(100, 50, 400, 200));
        assert_eq!(m, r(100, 50, 200, 200));
        // What sticks out of the span is cut to it.
        let m = map_rect(r(-100, 0, 600, 500), r(0, 0, 1000, 500), r(0, 0, 400, 200));
        assert_eq!(m, r(0, 0, 200, 200));
    }

    #[test]
    fn zones_that_share_an_edge_dont_overlap() {
        // Two rows at gap 0, and a pixel of rounding into each other.
        assert!(!overlaps(r(0, 0, 1000, 500), r(0, 500, 1000, 500)));
        assert!(!overlaps(r(0, 0, 1000, 501), r(0, 500, 1000, 500)));
        assert!(overlaps(r(0, 0, 1000, 600), r(0, 500, 1000, 500)));
        // A window a few pixels off, or taller for its minimum, is still in its zone.
        assert!(mostly_inside(r(3, -7, 1000, 520), r(0, 0, 1000, 500)));
        assert!(!mostly_inside(r(0, 400, 1000, 500), r(0, 0, 1000, 500)));
    }

    #[test]
    fn a_mapped_rect_finds_its_zones() {
        let zones = [r(0, 0, 100, 100), r(100, 0, 100, 100), r(0, 100, 200, 100)];
        assert_eq!(zones_of(&zones, r(3, 0, 98, 101)), Some((0, 0)));
        assert_eq!(zones_of(&zones, r(0, 0, 200, 100)), Some((0, 1)));
        assert_eq!(zones_of(&zones, r(0, 0, 150, 70)), None);
    }
}
