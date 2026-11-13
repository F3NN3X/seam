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

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
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
    /// The snapped windows still in zones of `g` that overlap `target`, and where
    /// each is. None when one of them can't be moved or is hung, which leaves the
    /// drop as it always was.
    pub(super) fn occupants(&self, drag: &Drag, g: &Placed, target: Rect) -> Option<Vec<(HWND, Rect)>> {
        let mut occupants = Vec::new();
        for (key, s) in &self.snapped {
            if *key == drag.hwnd || s.screen != g.id || s.from.max(s.to) >= g.zones.len() {
                continue;
            }
            if !overlaps(zones::span(g.zones[s.from], g.zones[s.to]), target) {
                continue;
            }
            let h = hwnd_of(*key);
            let there = unsafe { IsWindow(Some(h)) }.as_bool() && unsafe { IsWindowVisible(h) }.as_bool();
            if !there || winutil::is_minimized(h) || winutil::is_cloaked(h) {
                continue;
            }
            let Some(at) = self.current(h).map(|c| c.at) else { continue };
            wincontrol::movable(h, &self.exclude).ok()?;
            occupants.push((h, at));
        }
        Some(occupants)
    }

    /// The moves that give the windows in the zones `(a, b)` of screen `screen` the
    /// dropped window's old place, and whether it was snapped. None when nothing
    /// swaps, and the drop is as it always was.
    pub(super) fn swap_plan(
        &self,
        drag: &Drag,
        screen: usize,
        (a, b): (usize, usize),
    ) -> Option<(Vec<Placement>, bool)> {
        if !self.settings.swap_on_drop {
            return None;
        }
        let g = &drag.geometry[screen];
        let target = zones::span(*g.zones.get(a)?, *g.zones.get(b)?);
        let dest = match &drag.old_zone {
            Some((id, from, to)) => {
                let old = drag.geometry.iter().find(|x| x.id == *id)?;
                let span = zones::span(*old.zones.get(*from)?, *old.zones.get(*to)?);
                // Growing or shrinking over its own area isn't a swap.
                if old.id == g.id && overlaps(span, target) {
                    return None;
                }
                Dest::Zones(old, *from, *to, span)
            }
            None => {
                let frame = drag.frame.filter(|_| !drag.was_maximized)?;
                let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
                let on = self.screens.iter().find(|s| s.bounds.contains(x, y))?;
                Dest::Rect(frame, work_area(on.bounds)?.1)
            }
        };
        let occupants = self.occupants(drag, g, target)?;
        if occupants.is_empty() {
            return None;
        }
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
        Some((occupants.iter().map(place).collect(), drag.old_zone.is_some()))
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
    fn a_mapped_rect_finds_its_zones() {
        let zones = [r(0, 0, 100, 100), r(100, 0, 100, 100), r(0, 100, 200, 100)];
        assert_eq!(zones_of(&zones, r(3, 0, 98, 101)), Some((0, 0)));
        assert_eq!(zones_of(&zones, r(0, 0, 200, 100)), Some((0, 1)));
        assert_eq!(zones_of(&zones, r(0, 0, 150, 70)), None);
    }
}
