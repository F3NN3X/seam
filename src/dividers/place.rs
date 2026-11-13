//! Where the handles go, worked out from the zones and the windows in them, and
//! how a handle looks.

use super::*;

/// Where a divider's line is, as `zones::rects` draws it.
fn line(d: &zones::Divider) -> i32 {
    match d.dir {
        Dir::Vertical => d.area.x + (d.area.w as f32 * d.ratio).round() as i32,
        Dir::Horizontal => d.area.y + (d.area.h as f32 * d.ratio).round() as i32,
    }
}

/// The zones of `a`, before a divider, and of `b`, after it, in `placed`.
pub(super) fn zones_of(placed: &[Rect], a: (isize, usize, usize), b: (isize, usize, usize)) -> Option<(Rect, Rect)> {
    let span = |(_, from, to): (isize, usize, usize)| Some(zones::span(*placed.get(from)?, *placed.get(to)?));
    Some((span(a)?, span(b)?))
}

/// The gap between two zones either side of a divider, along the stretch where
/// they lie side by side. None where it's too narrow for a handle.
pub(super) fn gap_between((za, zb): (Rect, Rect), dir: Dir) -> Option<Rect> {
    let r = match dir {
        Dir::Vertical => {
            let (x, y) = (za.x + za.w, za.y.max(zb.y));
            Rect { x, y, w: zb.x - x, h: (za.y + za.h).min(zb.y + zb.h) - y }
        }
        Dir::Horizontal => {
            let (x, y) = (za.x.max(zb.x), za.y + za.h);
            Rect { x, y, w: (za.x + za.w).min(zb.x + zb.w) - x, h: zb.y - y }
        }
    };
    let thick = if dir == Dir::Vertical { r.w } else { r.h };
    (thick >= MIN_GAP_PX && r.w > 0 && r.h > 0).then_some(r)
}

/// Every handle the zones call for: one per pair of windows either side of a
/// divider, where both lie wholly in that divider's split. Worker only: it reads
/// every window's rect. Also every window in zones it found.
pub(super) fn spots(scene: &Scene) -> (Vec<Spot>, HashSet<isize>) {
    let (mut out, mut seen) = (Vec::new(), HashSet::new());
    for (i, s) in scene.screens.iter().enumerate().filter(|(_, s)| s.gap >= MIN_GAP_PX) {
        let tree = &s.layout.tree;
        let (cells, placed) = (zones::rects(tree, s.work, 0), zones::rects(tree, s.work, s.gap));
        let found = wincontrol::screen_members(&s.id, s.bounds, &placed, &scene.known, &scene.exclude);
        seen.extend(found.iter().map(|(h, ..)| h.0 as isize));
        // A window Seam may not move gets no handle.
        let members: Vec<(isize, usize, usize)> = found
            .into_iter()
            .filter(|(h, ..)| wincontrol::movable(*h, &scene.exclude).is_ok())
            .map(|(h, from, to)| (h.0 as isize, from, to))
            .collect();
        for d in zones::dividers(tree, s.work) {
            let at = line(&d);
            let ends = |(_, from, to): &(isize, usize, usize)| {
                let c = zones::span(*cells.get(*from)?, *cells.get(*to)?);
                let inside = c.x >= d.area.x
                    && c.y >= d.area.y
                    && c.x + c.w <= d.area.x + d.area.w
                    && c.y + c.h <= d.area.y + d.area.h;
                let ends = match d.dir {
                    Dir::Vertical => (c.x, c.x + c.w),
                    Dir::Horizontal => (c.y, c.y + c.h),
                };
                inside.then_some(ends)
            };
            let before: Vec<_> = members.iter().filter(|m| ends(m).is_some_and(|(_, end)| end == at)).collect();
            let after: Vec<_> = members.iter().filter(|m| ends(m).is_some_and(|(start, _)| start == at)).collect();
            for (a, b) in before.iter().flat_map(|a| after.iter().map(move |b| (**a, **b))) {
                let Some(pair) = zones_of(&placed, a, b) else { continue };
                if let Some(rect) = gap_between(pair, d.dir) {
                    let (path, dir, dpi) = (d.path.clone(), d.dir, s.dpi);
                    out.push(Spot { screen: i, path, dir, a, b, zones: pair, rect, dpi });
                }
            }
        }
    }
    (out, seen)
}

/// Put a handle on `r` with the bar along its middle, or with nothing to see.
pub(super) fn paint(hwnd: HWND, r: Rect, dir: Dir, bar: bool, dpi: u32) {
    let thick = (BAR_PX * dpi as f32 / 96.0).round() as i32;
    let across = if dir == Dir::Vertical { r.w } else { r.h };
    let thick = thick.clamp(1, across.max(1));
    let (lo, hi) = ((across - thick) / 2, (across - thick) / 2 + thick);
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.w,
                biHeight: -r.h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                let old = SelectObject(dc, bitmap.into());
                let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (r.w * r.h) as usize);
                for (i, px) in pixels.iter_mut().enumerate() {
                    let (x, y) = (i as i32 % r.w, i as i32 / r.w);
                    let at = if dir == Dir::Vertical { x } else { y };
                    *px = if bar && at >= lo && at < hi { BAR } else { CLEAR };
                }
                let _ = GdiFlush();
                let (at, size) = (POINT { x: r.x, y: r.y }, SIZE { cx: r.w, cy: r.h });
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    hwnd,
                    Some(screen),
                    Some(&at),
                    Some(&size),
                    Some(dc),
                    Some(&POINT::default()),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(dc, old);
            }
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}
