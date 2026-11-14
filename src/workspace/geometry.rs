//! Where a row's window sits, as fractions and zones.

use super::*;

/// `r` as fractions of `work`.
pub fn to_fractions(r: Rect, work: Rect) -> [f32; 4] {
    let (w, h) = (work.w.max(1) as f32, work.h.max(1) as f32);
    [(r.x - work.x) as f32 / w, (r.y - work.y) as f32 / h, r.w as f32 / w, r.h as f32 / h]
}

/// Fractions come from files, so a wild or NaN one is pulled back to something a screen can hold.
pub fn from_fractions(f: [f32; 4], work: Rect) -> Rect {
    let tame = |v: f32, lo: f32, hi: f32| if v.is_nan() { lo.max(0.) } else { v.clamp(lo, hi) };
    let (w, h) = (work.w as f32, work.h as f32);
    Rect {
        x: work.x.saturating_add((tame(f[0], -1., 2.) * w).round() as i32),
        y: work.y.saturating_add((tame(f[1], -1., 2.) * h).round() as i32),
        w: ((tame(f[2], 0.05, 2.) * w).round() as i32).max(1),
        h: ((tame(f[3], 0.05, 2.) * h).round() as i32).max(1),
    }
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x, b.x), (a.y, b.y), (a.w, b.w), (a.h, b.h)].iter().all(|(p, q)| (p - q).abs() <= ZONE_TOLERANCE)
}

/// The zone, or the span of two zones, that `visible` is within a few pixels of.
pub fn target_of(visible: Rect, zones: &[Rect]) -> Option<Target> {
    if let Some(i) = zones.iter().position(|z| near(visible, *z)) {
        return Some(Target::Zone { zone: i as u32 + 1, span_to: None });
    }
    (0..zones.len()).find_map(|a| {
        let b = (a + 1..zones.len()).find(|b| near(visible, zones::span(zones[a], zones[*b])))?;
        Some(Target::Zone { zone: a as u32 + 1, span_to: Some(b as u32 + 1) })
    })
}

/// Where a window with this target goes, in pixels. A zone that is gone means
/// the whole work area. `None` for max and keep, which aren't a place.
pub fn rect_for(target: &Target, zones: &[Rect], work: Rect) -> Option<Rect> {
    let zone = |n: u32| n.checked_sub(1).and_then(|i| zones.get(i as usize)).copied();
    match target {
        Target::Zone { zone: first, span_to } => Some(match (zone(*first), span_to.and_then(zone)) {
            (Some(a), Some(b)) => zones::span(a, b),
            (Some(a), None) => a,
            (None, _) => work,
        }),
        Target::Rect { rect } => Some(from_fractions(*rect, work)),
        Target::State { .. } => None,
    }
}
