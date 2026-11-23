//! The tests for where screens stand in a span and which part of the picture each one shows.

use super::*;
use crate::geom::{EdgeMap, Rect, Side};

fn mon(id: &str, rect: Rect, raw_dpi: f64, primary: bool) -> Mon {
    Mon { id: id.into(), gdi: String::new(), name: id.into(), named: true, rect, raw_dpi, primary }
}

/// A border from screen `a` to `b` on `side`, with Line Up's offset of `mm` along the shared edge
/// as seen from `a`, whose raw density is `dpi`.
fn link(a: usize, b: usize, side: Side, mm: f64, dpi: f64) -> Link {
    let map = EdgeMap { s: 1.0, o: -mm / 25.4 * dpi };
    Link { a, b, side, map, glued: false, not_on_desk: false }
}

/// Two 1080p screens of the same size, side by side and lined up.
fn pair() -> ([Mon; 2], [Link; 1]) {
    let left = mon("a", Rect::new(0, 0, 1920, 1080), 96.0, true);
    let right = mon("b", Rect::new(1920, 0, 1920, 1080), 96.0, false);
    ([left, right], [link(0, 1, Side::Right, 0.0, 96.0)])
}

/// The worked example of D805: a 27" 4K and a 24" 1080p, the second 20 mm down.
fn unequal() -> ([Mon; 2], [Link; 1]) {
    let big = mon("a", Rect::new(0, 0, 3840, 2160), 163.0, true);
    let small = mon("b", Rect::new(3840, 0, 1920, 1080), 92.0, false);
    ([big, small], [link(0, 1, Side::Right, 20.0, 163.0)])
}

fn cuts_of(mons: &[Mon], links: &[Link], gap: f64, src: (u32, u32), fit: Fit) -> Vec<Option<Cut>> {
    cuts(src, &span_rects(mons, links, gap).0, fit)
}

fn cut_at(x: i32, y: i32, w: u32, h: u32) -> Option<Cut> {
    Some(Cut { x, y, w, h })
}

/// `got` is within 2 source pixels of `want` (x, y, w, h).
fn near(got: Option<Cut>, want: (i32, i32, i32, i32)) {
    let c = got.expect("a cut");
    for (g, e) in [(c.x, want.0), (c.y, want.1), (c.w as i32, want.2), (c.h as i32, want.3)] {
        assert!((g - e).abs() <= 2, "{c:?} is not near {want:?}");
    }
}

#[test]
fn two_equal_screens_split_the_picture_in_half() {
    let (mons, links) = pair();
    let got = cuts_of(&mons, &links, 0.0, (3840, 1080), Fit::Fill);
    assert_eq!(got, [cut_at(0, 0, 1920, 1080), cut_at(1920, 0, 1920, 1080)]);
}

#[test]
fn different_sizes_get_the_same_physical_scale() {
    let (mons, links) = unequal();
    let (rects, layout) = span_rects(&mons, &links, 0.0);
    assert_eq!(layout, Layout::Desk);
    let got = cuts((6000, 2000), &rects, Fit::Fill);
    near(got[0], (0, 106, 3181, 1788));
    near(got[1], (3181, 213, 2819, 1585));
    // Source pixels per mm are the same on both screens.
    let per_mm = |i: usize| got[i].unwrap().w as f64 / rects[i].unwrap().w;
    assert!((per_mm(0) - per_mm(1)).abs() < 0.01, "{} against {}", per_mm(0), per_mm(1));
    // Not lined up, each screen pixel gets the same piece of the picture, so the 24" screen's
    // piece is much bigger in the room than the 27" screen's.
    let pixels = cuts_of(&mons, &[], 0.0, (6000, 2000), Fit::Fill);
    let per_px = |i: usize| pixels[i].unwrap().w as f64 / mons[i].rect.width() as f64;
    assert!((per_px(0) - per_px(1)).abs() < 1e-3);
    let per_mm_in_room = |i: usize| per_px(i) * mons[i].raw_dpi / 25.4;
    assert!(per_mm_in_room(0) / per_mm_in_room(1) > 1.5);
}

#[test]
fn along_offset_from_line_up_shifts_the_cut() {
    let (mons, _) = pair();
    let level = cuts_of(&mons, &[link(0, 1, Side::Right, 0.0, 96.0)], 0.0, (3840, 1280), Fit::Fill);
    let down = cuts_of(&mons, &[link(0, 1, Side::Right, 25.4, 96.0)], 0.0, (3840, 1280), Fit::Fill);
    assert_eq!(level[0].unwrap().y, level[1].unwrap().y);
    // 25.4 mm is 96 px at 96 dpi, and 3840 px across 1016 mm is 96 dpi too.
    assert_eq!(down[1].unwrap().y - down[0].unwrap().y, 96);
}

#[test]
fn stacked_screens_cut_top_and_bottom() {
    let top = mon("a", Rect::new(0, 0, 1920, 1080), 96.0, true);
    let bottom = mon("b", Rect::new(0, 1080, 1920, 1080), 96.0, false);
    let got = cuts_of(&[top, bottom], &[link(0, 1, Side::Bottom, 0.0, 96.0)], 0.0, (1920, 2160), Fit::Fill);
    assert_eq!(got, [cut_at(0, 0, 1920, 1080), cut_at(0, 1080, 1920, 1080)]);
}

#[test]
fn l_shape_corner_is_not_shown() {
    let at = |id: &str, x, y, primary| mon(id, Rect::new(x, y, 1000, 1000), 0.0, primary);
    let mons = [at("a", 0, 0, true), at("b", 1000, 0, false), at("c", 0, 1000, false)];
    let got = cuts_of(&mons, &[], 0.0, (2000, 2000), Fit::Fill);
    assert_eq!(got, [cut_at(0, 0, 1000, 1000), cut_at(1000, 0, 1000, 1000), cut_at(0, 1000, 1000, 1000)]);
    // Nothing covers the empty corner, right and below.
    let corner = |c: &Cut| c.x + c.w as i32 > 1000 && c.y + c.h as i32 > 1000;
    assert!(!got.iter().flatten().any(corner));
}

#[test]
fn fit_puts_bars_in_the_box_not_per_screen() {
    let (mons, links) = pair();
    // A square picture in a box twice as wide as tall: bars at the two ends of the box, none between.
    let got = cuts_of(&mons, &links, 0.0, (1080, 1080), Fit::Fit);
    assert_eq!(got, [cut_at(-1380, 0, 1920, 1080), cut_at(540, 0, 1920, 1080)]);
    // The first screen is bars and then the left half of the picture, the second the right half and bars.
    let (clip, placed) = part((1080, 1080), got[0].unwrap(), (1920, 1080)).unwrap();
    assert_eq!(clip, Area { x: 0, y: 0, w: 540, h: 1080 });
    assert_eq!((placed.piece.to, placed.scaled), ((1380, 0), (540, 1080)));
    let (clip, placed) = part((1080, 1080), got[1].unwrap(), (1920, 1080)).unwrap();
    assert_eq!(clip, Area { x: 540, y: 0, w: 540, h: 1080 });
    assert_eq!((placed.piece.to, placed.scaled), ((0, 0), (540, 1080)));
}

#[test]
fn not_lined_up_uses_pixels() {
    let (mons, _) = pair();
    let (rects, layout) = span_rects(&mons, &[], 0.0);
    assert_eq!(layout, Layout::Windows);
    assert_eq!(rects[1], Some(RectF { x: 1920.0, y: 0.0, w: 1920.0, h: 1080.0 }));
    // The space between screens is for the physical layout only.
    assert_eq!(span_rects(&mons, &[], 10.0).0, rects);
}

#[test]
fn one_unsized_screen_uses_pixels() {
    let (mut mons, links) = pair();
    mons[1].raw_dpi = 0.0;
    let (rects, layout) = span_rects(&mons, &links, 0.0);
    assert_eq!(layout, Layout::Windows);
    assert_eq!(rects[0], Some(RectF { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }));
    assert_eq!(unsized_screens(&mons, &links), [1]);
}

#[test]
fn unreached_screen_is_left_out() {
    let ([a, b], links) = pair();
    let far = mon("c", Rect::new(9000, 0, 1920, 1080), 96.0, false);
    let mons = [a, b, far];
    let got = cuts_of(&mons, &[], 0.0, (3840, 1080), Fit::Fill);
    assert_eq!(got, [cut_at(0, 0, 1920, 1080), cut_at(1920, 0, 1920, 1080), None]);
    // A border marked not on the desk leaves the screen past it out, though they touch in Windows.
    let off = [Link { not_on_desk: true, ..links[0] }];
    let (rects, _) = span_rects(&mons[..2], &off, 0.0);
    assert_eq!((rects[0].is_some(), rects[1]), (true, None));
}

#[test]
fn a_gap_skips_the_bezels() {
    let (mons, links) = pair();
    // 25.4 mm is 96 px at 96 dpi: the picture loses 96 px between the screens.
    let got = cuts_of(&mons, &links, 25.4, (3936, 1080), Fit::Fill);
    assert_eq!(got, [cut_at(0, 0, 1920, 1080), cut_at(2016, 0, 1920, 1080)]);
}

#[test]
fn a_cut_inside_the_picture_fills_the_screen() {
    let cut = Cut { x: 100, y: 50, w: 400, h: 200 };
    let (clip, placed) = part((1000, 1000), cut, (800, 400)).unwrap();
    assert_eq!(clip, Area { x: 100, y: 50, w: 400, h: 200 });
    assert_eq!((placed.scaled, placed.piece.to), ((800, 400), (0, 0)));
    // All outside the picture: only bars.
    assert!(part((1000, 1000), Cut { x: -500, y: 0, w: 400, h: 400 }, (800, 400)).is_none());
}

#[test]
fn the_gap_goes_on_the_left_and_top_too() {
    // The primary is the second screen, with the other to its left, then above it.
    let at = |x, y, primary| mon("m", Rect::new(x, y, 1920, 1080), 96.0, primary);
    let left = [at(1920, 0, true), at(0, 0, false)];
    let (rects, _) = span_rects(&left, &[link(0, 1, Side::Left, 0.0, 96.0)], 25.4);
    // The other screen is 508 mm wide and ends 25.4 mm before the primary.
    assert!((rects[1].unwrap().x + 508.0 + 25.4).abs() < 1e-9, "{:?}", rects[1]);
    let above = [at(0, 1080, true), at(0, 0, false)];
    let (rects, _) = span_rects(&above, &[link(0, 1, Side::Top, 0.0, 96.0)], 25.4);
    assert!((rects[1].unwrap().y + 285.75 + 25.4).abs() < 1e-9, "{:?}", rects[1]);
}
