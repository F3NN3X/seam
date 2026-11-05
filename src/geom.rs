//! Platform-independent geometry: how two touching screens map onto each other.
//!
//! Windows lays monitors out in raw pixels. When two monitors have different
//! pixel densities (or sit at a physical offset), the pixel layout cannot match
//! the desk. For each shared border we therefore keep a linear map between the
//! two edges, expressed in coordinates *relative to each monitor's own origin*
//! along that edge:
//!
//! ```text
//! b_rel = s * a_rel + o
//! ```
//!
//! `s` is the physical scale (B pixels per A pixel) and `o` the offset. Two
//! line pairs the user lines up by eye are enough to solve for both. Because the
//! map is relative, it survives rearranging monitors in Windows settings.

use serde::{Deserialize, Serialize};

/// Which side of monitor A the neighbouring monitor B sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }

    /// True when the shared edge is vertical (monitors side by side), so the
    /// coordinate *along* the edge is Y.
    pub fn along_is_y(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Rect {
        Rect { left, top, right: left + width, bottom: top + height }
    }
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
    /// Origin of the rect along the axis of an edge on `side`.
    pub fn along_origin(&self, side: Side) -> i32 {
        if side.along_is_y() {
            self.top
        } else {
            self.left
        }
    }
    /// Length of the rect along the axis of an edge on `side`.
    pub fn along_len(&self, side: Side) -> i32 {
        if side.along_is_y() {
            self.height()
        } else {
            self.width()
        }
    }
}

/// Linear map between two edges: `b_rel = s * a_rel + o`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeMap {
    pub s: f64,
    pub o: f64,
}

impl EdgeMap {
    pub fn apply(&self, a: f64) -> f64 {
        self.s * a + self.o
    }

    pub fn inverse(&self) -> EdgeMap {
        EdgeMap { s: 1.0 / self.s, o: -self.o / self.s }
    }

    /// Solve the map from two corresponding points. Rejects degenerate or
    /// mirrored solutions (the screens can't be flipped relative to each other).
    pub fn from_pairs(a1: f64, b1: f64, a2: f64, b2: f64) -> Option<EdgeMap> {
        let da = a2 - a1;
        if da.abs() < 1.0 {
            return None;
        }
        let s = (b2 - b1) / da;
        if !(s.is_finite()) || !(0.1..=10.0).contains(&s) {
            return None;
        }
        Some(EdgeMap { s, o: b1 - s * a1 })
    }
}

fn span_overlap(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    a1.min(b1) - a0.max(b0)
}

/// If `b` touches `a` along an edge segment of positive length, return the side
/// of `a` that `b` is on.
pub fn touching(a: &Rect, b: &Rect) -> Option<Side> {
    let vy = span_overlap(a.top, a.bottom, b.top, b.bottom) > 0;
    let hx = span_overlap(a.left, a.right, b.left, b.right) > 0;
    if a.right == b.left && vy {
        Some(Side::Right)
    } else if a.left == b.right && vy {
        Some(Side::Left)
    } else if a.bottom == b.top && hx {
        Some(Side::Bottom)
    } else if a.top == b.bottom && hx {
        Some(Side::Top)
    } else {
        None
    }
}

/// First guess before the user calibrates: scale from the monitors' physical
/// DPI (from EDID), offset so the middle of the current layout overlap lines up.
pub fn predict(a: &Rect, b: &Rect, side: Side, dpi_a: f64, dpi_b: f64) -> EdgeMap {
    let ratio = dpi_b / dpi_a;
    let s = if dpi_a > 0.0 && dpi_b > 0.0 && ratio.is_finite() && (0.25..=4.0).contains(&ratio) { ratio } else { 1.0 };
    let (ao, al) = (a.along_origin(side), a.along_len(side));
    let (bo, bl) = (b.along_origin(side), b.along_len(side));
    let lo = ao.max(bo);
    let hi = (ao + al).min(bo + bl);
    let mid = if hi > lo { (lo + hi) as f64 / 2.0 } else { ao as f64 + al as f64 / 2.0 };
    let a_mid = mid - ao as f64;
    let b_mid = mid - bo as f64;
    EdgeMap { s, o: b_mid - s * a_mid }
}

/// Where the two edges physically overlap, in A-relative coordinates.
pub fn overlap_in_a(m: &EdgeMap, len_a: i32, len_b: i32) -> Option<(f64, f64)> {
    let inv = m.inverse();
    let x0 = inv.apply(0.0);
    let x1 = inv.apply(len_b as f64);
    let lo = x0.min(x1).max(0.0);
    let hi = x0.max(x1).min(len_a as f64);
    if hi - lo >= 2.0 {
        Some((lo, hi))
    } else {
        None
    }
}

/// The two fixed calibration lines on A: 20% and 80% into the (predicted)
/// physical overlap, so both lines also land on B.
pub fn anchor_points(m: &EdgeMap, len_a: i32, len_b: i32) -> (f64, f64) {
    let (lo, hi) = overlap_in_a(m, len_a, len_b).unwrap_or((0.0, len_a as f64));
    ((lo + 0.2 * (hi - lo)).round(), (lo + 0.8 * (hi - lo)).round())
}

/// One way the pointer left a monitor: the side, the absolute coordinate along
/// that edge where it crossed, and how many pixels past the edge it went.
pub type Exit = (Side, f64, i32);

/// How the pointer left monitor `a`. A move past a corner leaves through two
/// sides at once, so the side it went furthest past comes first.
pub fn exits_of(a: &Rect, prev: (i32, i32), pt: (i32, i32)) -> [Option<Exit>; 2] {
    let over_x = if pt.0 >= a.right {
        Some((Side::Right, pt.0 - a.right))
    } else if pt.0 < a.left {
        Some((Side::Left, a.left - 1 - pt.0))
    } else {
        None
    };
    let over_y = if pt.1 >= a.bottom {
        Some((Side::Bottom, pt.1 - a.bottom))
    } else if pt.1 < a.top {
        Some((Side::Top, a.top - 1 - pt.1))
    } else {
        None
    };
    let (first, second) = match (over_x, over_y) {
        (Some(x), Some(y)) if y.1 > x.1 => (Some(y), Some(x)),
        (Some(x), y) => (Some(x), y),
        (None, y) => (y, None),
    };
    [first, second].map(|e| e.map(|(side, over)| exit_through(a, prev, pt, side, over)))
}

fn exit_through(a: &Rect, prev: (i32, i32), pt: (i32, i32), side: Side, over: i32) -> Exit {
    // Interpolate where the straight move from prev to pt crossed the edge.
    let (p0, p1, q0, q1, edge) = match side {
        Side::Right => (prev.0, pt.0, prev.1, pt.1, a.right as f64 - 0.5),
        Side::Left => (prev.0, pt.0, prev.1, pt.1, a.left as f64 - 0.5),
        Side::Bottom => (prev.1, pt.1, prev.0, pt.0, a.bottom as f64 - 0.5),
        Side::Top => (prev.1, pt.1, prev.0, pt.0, a.top as f64 - 0.5),
    };
    let t = if p1 != p0 { ((edge - p0 as f64) / (p1 - p0) as f64).clamp(0.0, 1.0) } else { 0.0 };
    let along = q0 as f64 + t * (q1 - q0) as f64;
    let (lo, hi) = if side.along_is_y() { (a.top, a.bottom - 1) } else { (a.left, a.right - 1) };
    (side, along.clamp(lo as f64, hi as f64), over.max(0))
}

/// Where the pointer should enter `b` after leaving `a` on `side`. `None` means
/// there's no physical screen there, so the move is left to Windows.
pub fn remap_crossing(
    a: &Rect,
    b: &Rect,
    side: Side,
    m: &EdgeMap,
    along_abs: f64,
    overshoot: i32,
) -> Option<(i32, i32)> {
    let a_rel = along_abs - a.along_origin(side) as f64;
    let raw = m.apply(a_rel);
    let last = b.along_len(side) as f64 - 1.0;
    // One source pixel spans `s` pixels here, so a pointer that came in by
    // rounding can sit half of one past the edge on its way back.
    let slack = 0.5 * m.s.max(1.0) + 1e-9;
    if !(raw >= -slack && raw <= last + slack) {
        return None;
    }
    let along_b = b.along_origin(side) + raw.round().clamp(0.0, last) as i32;
    let over = overshoot.max(0);
    Some(match side {
        Side::Right => ((b.left + over).min(b.right - 1), along_b),
        Side::Left => ((b.right - 1 - over).max(b.left), along_b),
        Side::Bottom => (along_b, (b.top + over).min(b.bottom - 1)),
        Side::Top => (along_b, (b.bottom - 1 - over).max(b.top)),
    })
}

/// Origin for `b` along the shared edge so Windows' own pixel layout lines up
/// at the middle of the physical overlap. This is what "align Windows
/// arrangement" writes, so the screens also behave sensibly without Seam.
pub fn aligned_along_origin(a: &Rect, b: &Rect, side: Side, m: &EdgeMap) -> Option<i32> {
    let (lo, hi) = overlap_in_a(m, a.along_len(side), b.along_len(side))?;
    let a_mid = (lo + hi) / 2.0;
    let b_mid = m.apply(a_mid);
    Some(a.along_origin(side) + (a_mid - b_mid).round() as i32)
}

/// Rect for `b` placed against `a` on `side`, at the given along-edge origin.
pub fn place_against(a: &Rect, b: &Rect, side: Side, along_origin: i32) -> Rect {
    let (w, h) = (b.width(), b.height());
    match side {
        Side::Right => Rect::new(a.right, along_origin, w, h),
        Side::Left => Rect::new(a.left - w, along_origin, w, h),
        Side::Bottom => Rect::new(along_origin, a.bottom, w, h),
        Side::Top => Rect::new(along_origin, a.top - h, w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 27" 4K (163 dpi) left, 27" 1440p (109 dpi) right, tops aligned in Windows.
    fn uhd() -> Rect {
        Rect::new(0, 0, 3840, 2160)
    }
    fn qhd() -> Rect {
        Rect::new(3840, 0, 2560, 1440)
    }

    #[test]
    fn touching_sides() {
        assert_eq!(touching(&uhd(), &qhd()), Some(Side::Right));
        assert_eq!(touching(&qhd(), &uhd()), Some(Side::Left));
        let below = Rect::new(500, 2160, 1920, 1080);
        assert_eq!(touching(&uhd(), &below), Some(Side::Bottom));
        assert_eq!(touching(&below, &uhd()), Some(Side::Top));
        let corner = Rect::new(3840, 2160, 100, 100);
        assert_eq!(touching(&uhd(), &corner), None);
    }

    #[test]
    fn pairs_roundtrip() {
        let m = EdgeMap::from_pairs(100.0, 50.0, 1100.0, 717.0).unwrap();
        assert!((m.apply(100.0) - 50.0).abs() < 1e-9);
        assert!((m.apply(1100.0) - 717.0).abs() < 1e-9);
        let inv = m.inverse();
        assert!((inv.apply(m.apply(600.0)) - 600.0).abs() < 1e-9);
        assert!(EdgeMap::from_pairs(100.0, 50.0, 100.4, 70.0).is_none());
        assert!(EdgeMap::from_pairs(100.0, 500.0, 900.0, 100.0).is_none()); // mirrored
    }

    #[test]
    fn predicted_scale_from_dpi() {
        let m = predict(&uhd(), &qhd(), Side::Right, 163.0, 109.0);
        assert!((m.s - 109.0 / 163.0).abs() < 1e-9);
        // The first guess pins the middle of the current Windows overlap
        // (rows 0..1440, mid 720) to the same row on both screens.
        assert!((m.apply(720.0) - 720.0).abs() < 1e-9);
        let ov = overlap_in_a(&m, 2160, 1440).unwrap();
        assert!(ov.0 == 0.0 && (ov.1 - 1796.0).abs() < 1.0);
    }

    #[test]
    fn crossing_keeps_physical_height() {
        // Physically identical 27" panels, tops aligned: map is pure scale.
        let m = EdgeMap { s: 1440.0 / 2160.0, o: 0.0 };
        // Leave the 4K screen at y=1620 (75% down) moving right.
        let (side, along, over) = exits_of(&uhd(), (3839, 1620), (3842, 1620))[0].unwrap();
        assert_eq!(side, Side::Right);
        assert_eq!(over, 2);
        let dest = remap_crossing(&uhd(), &qhd(), side, &m, along, over).unwrap();
        assert_eq!(dest, (3842, 1080)); // 75% down the 1440p screen
                                        // And back again: 1080 on the QHD maps to 1620 on the UHD.
        let (side2, along2, over2) = exits_of(&qhd(), (3840, 1080), (3839, 1080))[0].unwrap();
        assert_eq!(side2, Side::Left);
        let back = remap_crossing(&qhd(), &uhd(), side2, &m.inverse(), along2, over2).unwrap();
        assert_eq!(back, (3839, 1620));
    }

    #[test]
    fn crossing_outside_physical_overlap_is_left_alone() {
        // Smaller screen covering only the middle of the big one's edge.
        let m = EdgeMap { s: 1.0, o: -500.0 };
        let dest = remap_crossing(&uhd(), &qhd(), Side::Right, &m, 200.0, 0);
        assert_eq!(dest, None);
    }

    #[test]
    fn diagonal_exit_interpolates() {
        let a = Rect::new(0, 0, 100, 100);
        let (side, along, _) = exits_of(&a, (98, 50), (102, 54))[0].unwrap();
        assert_eq!(side, Side::Right);
        assert!((along - 51.5).abs() < 1e-9);
    }

    #[test]
    fn aligned_origin_centres_overlap() {
        // 1440p screen physically centred on the 4K edge: in UHD pixels it spans
        // 0..2160 when same size; with an offset it should shift.
        let m = EdgeMap { s: 1440.0 / 2160.0, o: 0.0 };
        let origin = aligned_along_origin(&uhd(), &qhd(), Side::Right, &m).unwrap();
        // Overlap midpoint on A is 1080; on B it's 720 → B.top = 0 + 1080 - 720.
        assert_eq!(origin, 360);
        let placed = place_against(&uhd(), &qhd(), Side::Right, origin);
        assert_eq!(placed, Rect::new(3840, 360, 2560, 1440));
        assert_eq!(touching(&uhd(), &placed), Some(Side::Right));
    }

    #[test]
    fn anchors_fall_inside_overlap() {
        let m = EdgeMap { s: 1.0, o: -1000.0 }; // B covers A 1000..2440
        let (a1, a2) = anchor_points(&m, 2160, 1440);
        assert!(a1 >= 1000.0 && a2 <= 2160.0 && a2 > a1);
    }

    #[test]
    fn pairs_reject_degenerate_input() {
        assert_eq!(EdgeMap::from_pairs(100.0, 50.0, 100.0, 70.0), None); // same line twice
        assert_eq!(EdgeMap::from_pairs(100.0, 50.0, 100.9, 50.9), None); // under a pixel apart
        assert_eq!(EdgeMap::from_pairs(100.0, 50.0, 500.0, 50.0), None); // zero scale
        assert_eq!(EdgeMap::from_pairs(0.0, 0.0, 1000.0, 50.0), None); // scale 0.05
        assert_eq!(EdgeMap::from_pairs(0.0, 0.0, 100.0, 1100.0), None); // scale 11
    }

    #[test]
    fn pairs_reject_non_finite_input() {
        assert_eq!(EdgeMap::from_pairs(f64::NAN, 0.0, 100.0, 100.0), None);
        assert_eq!(EdgeMap::from_pairs(0.0, f64::NAN, 100.0, 100.0), None);
        assert_eq!(EdgeMap::from_pairs(0.0, 0.0, f64::INFINITY, 100.0), None);
        assert_eq!(EdgeMap::from_pairs(0.0, 0.0, 100.0, f64::INFINITY), None);
    }

    #[test]
    fn pairs_reject_mirrored_input_either_way_round() {
        assert_eq!(EdgeMap::from_pairs(100.0, 500.0, 900.0, 100.0), None);
        assert_eq!(EdgeMap::from_pairs(900.0, 100.0, 100.0, 500.0), None);
    }

    #[test]
    fn pairs_accept_the_limits_and_either_order() {
        assert_eq!(EdgeMap::from_pairs(0.0, 0.0, 1.0, 1.0), Some(EdgeMap { s: 1.0, o: 0.0 }));
        assert_eq!(EdgeMap::from_pairs(0.0, 5.0, 100.0, 15.0), Some(EdgeMap { s: 0.1, o: 5.0 }));
        assert_eq!(EdgeMap::from_pairs(0.0, 5.0, 100.0, 1005.0), Some(EdgeMap { s: 10.0, o: 5.0 }));
        assert_eq!(EdgeMap::from_pairs(100.0, 15.0, 0.0, 5.0), Some(EdgeMap { s: 0.1, o: 5.0 }));
    }
}
