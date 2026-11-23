//! One picture across the screens (brief L, D804, D805): where each screen stands, and the part of
//! the picture it shows. Pure geometry; the pictures themselves are the worker's.

use super::{Area, Fit, Piece, Placement};
use crate::desk::{Mon, walk};
use crate::layout::Link;

/// A place on the desk: millimetres from the primary's corner when lined up, else Windows' pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What the places are measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The physical layout from Line Up, in mm (D804.1).
    Desk,
    /// Windows' arrangement, in pixels (D804.2).
    Windows,
}

/// Where a screen stands in the span: its own place, and the box round every screen in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub bounds: RectF,
    pub rect: RectF,
}

/// A part of the source picture, in source pixels, for one screen. Fit can put it partly outside
/// the picture (`part` turns that into bars).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Where each screen stands (D804), `None` for one the walk from the primary doesn't reach, which is
/// left out. `gap_mm` is the space between screens that meet, in the physical layout only. Lined up
/// and every reached screen sized: the physical layout. Otherwise, one wrong size would shift every
/// part, so Windows' pixels.
pub fn span_rects(mons: &[Mon], links: &[Link], gap_mm: f64) -> (Vec<Option<RectF>>, Layout) {
    let (placed, reached) = walk(mons, links, gap_mm);
    if !links.is_empty() && placed.iter().filter(|p| reached[p.index]).all(|p| p.sized) {
        let mut rects = vec![None; mons.len()];
        for p in placed.iter().filter(|p| reached[p.index]) {
            rects[p.index] = Some(RectF { x: p.x, y: p.y, w: p.w, h: p.h });
        }
        return (rects, Layout::Desk);
    }
    let pixels = |m: &Mon| RectF {
        x: m.rect.left as f64,
        y: m.rect.top as f64,
        w: m.rect.width().max(1) as f64,
        h: m.rect.height().max(1) as f64,
    };
    let rects = mons.iter().zip(&reached).map(|(m, in_span)| in_span.then(|| pixels(m)));
    (rects.collect(), Layout::Windows)
}

/// The screens in the span that don't report their size, by index.
pub fn unsized_screens(mons: &[Mon], links: &[Link]) -> Vec<usize> {
    let (_, reached) = walk(mons, links, 0.0);
    (0..mons.len()).filter(|i| reached[*i] && mons[*i].raw_dpi <= 0.0).collect()
}

/// The box round the places.
pub fn bounds(rects: &[Option<RectF>]) -> Option<RectF> {
    let mut each = rects.iter().flatten();
    let first = each.next()?;
    let start = (first.x, first.y, first.x + first.w, first.y + first.h);
    let (left, top, right, bottom) =
        each.fold(start, |(l, t, r, b), s| (l.min(s.x), t.min(s.y), r.max(s.x + s.w), b.max(s.y + s.h)));
    Some(RectF { x: left, y: top, w: right - left, h: bottom - top })
}

/// Every screen's place and its box, `None` for a screen that is left out.
pub fn places(rects: &[Option<RectF>]) -> Vec<Option<Place>> {
    let bounds = bounds(rects);
    rects.iter().map(|r| Some(Place { bounds: bounds?, rect: (*r)? })).collect()
}

/// Where a picture of size `src` sits in the box, in the box's units: Fill covers the box, Fit goes
/// inside it and Stretch fills it exactly, centred. It goes past the box for Fill.
pub fn fitted(src: (u32, u32), bounds: RectF, fit: Fit) -> RectF {
    let (sw, sh) = (src.0.max(1) as f64, src.1.max(1) as f64);
    let (wide, tall) = (bounds.w / sw, bounds.h / sh);
    let (ux, uy) = match fit {
        Fit::Stretch => (wide, tall),
        Fit::Fit => (wide.min(tall), wide.min(tall)),
        _ => (wide.max(tall), wide.max(tall)),
    };
    let (w, h) = (sw * ux, sh * uy);
    RectF { x: bounds.x + (bounds.w - w) / 2.0, y: bounds.y + (bounds.h - h) / 2.0, w, h }
}

/// The part of a picture of size `src` that `place` shows (D805). The picture is fitted to the box:
/// Fill covers it, Fit goes inside it and Stretch fills it exactly. The part is the screen's place
/// in the box, mapped into source pixels.
pub fn cut(src: (u32, u32), place: &Place, fit: Fit) -> Cut {
    let (sw, sh) = (src.0.max(1) as f64, src.1.max(1) as f64);
    let r = place.rect;
    let picture = fitted(src, place.bounds, fit);
    // Units of the box per source pixel, and where the picture starts.
    let (ux, uy) = ((picture.w / sw).max(1e-9), (picture.h / sh).max(1e-9));
    let (ox, oy) = (picture.x, picture.y);
    let (x0, x1) = (((r.x - ox) / ux).round() as i32, ((r.x + r.w - ox) / ux).round() as i32);
    let (y0, y1) = (((r.y - oy) / uy).round() as i32, ((r.y + r.h - oy) / uy).round() as i32);
    Cut { x: x0, y: y0, w: (x1 - x0).max(1) as u32, h: (y1 - y0).max(1) as u32 }
}

/// `cut` for every screen of the span, `None` for one left out.
pub fn cuts(src: (u32, u32), rects: &[Option<RectF>], fit: Fit) -> Vec<Option<Cut>> {
    places(rects).iter().map(|p| p.as_ref().map(|p| cut(src, p, fit))).collect()
}

/// How a cut of a picture of size `src` goes onto a screen of size `target`: the piece of the
/// picture to read (what of the cut is inside the picture), and where it lands scaled on the
/// screen, for `compose`. The rest of the screen is bars. `None`: the cut is all outside the picture.
pub fn part(src: (u32, u32), cut: Cut, target: (u32, u32)) -> Option<(Area, Placement)> {
    let (cx0, cy0) = (cut.x as i64, cut.y as i64);
    let (cx1, cy1) = (cx0 + cut.w as i64, cy0 + cut.h as i64);
    let (x0, y0, x1, y1) = (cx0.max(0), cy0.max(0), cx1.min(src.0 as i64), cy1.min(src.1 as i64));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (tw, th) = (target.0.max(1) as i64, target.1.max(1) as i64);
    // A source pixel's place on the screen, rounded.
    let onto = |p: i64, from: i64, len: u32, to: i64| ((p - from) * to + len as i64 / 2) / len as i64;
    let at = |p: i64, from: i64, len: u32, to: i64| onto(p, from, len, to).clamp(0, to);
    let (dx0, dx1) = (at(x0, cx0, cut.w, tw).min(tw - 1), at(x1, cx0, cut.w, tw));
    let (dy0, dy1) = (at(y0, cy0, cut.h, th).min(th - 1), at(y1, cy0, cut.h, th));
    let (dw, dh) = ((dx1 - dx0).max(1) as u32, (dy1 - dy0).max(1) as u32);
    let clip = Area { x: x0 as u32, y: y0 as u32, w: (x1 - x0) as u32, h: (y1 - y0) as u32 };
    let piece = Piece { from: Area { x: 0, y: 0, w: dw, h: dh }, to: (dx0 as u32, dy0 as u32) };
    Some((clip, Placement { scaled: (dw, dh), piece, tiled: false, screen: (tw as u32, th as u32) }))
}

#[cfg(test)]
mod tests;
