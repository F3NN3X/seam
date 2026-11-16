//! The overview's fonts, made once per pixel size and kept across opens, so an open
//! makes none. A size is the point size at a screen's DPI, so a few DPIs hold a few
//! dozen at most.

use super::*;
use crate::showlayout::rgb;

/// Sizes kept: some ten per DPI, for three or four DPIs. Past it the cache starts over.
const KEPT: usize = 40;

/// Fonts by pixel size. A font is a GDI handle any thread can use, kept as a number.
static FONTS: Mutex<Vec<(i32, isize)>> = Mutex::new(Vec::new());

fn made(px: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

/// The font for a pixel size, made the first time it is asked for.
fn font(px: i32) -> HFONT {
    let mut kept = locked(&FONTS);
    if let Some(&(_, f)) = kept.iter().find(|(p, _)| *p == px) {
        return HFONT(f as *mut _);
    }
    if kept.len() >= KEPT {
        for (_, f) in kept.drain(..) {
            let _ = unsafe { DeleteObject(HFONT(f as *mut _).into()) };
        }
    }
    let f = made(px);
    kept.push((px, f.0 as isize));
    f
}

/// Every font goes, when Seam quits.
pub fn release() {
    for (_, f) in locked(&FONTS).drain(..) {
        let _ = unsafe { DeleteObject(HFONT(f as *mut _).into()) };
    }
}

/// One line of text in a rect, `px` tall, as `showlayout::text` does it but with a kept font.
pub(super) unsafe fn text(dc: HDC, s: &str, mut r: RECT, px: i32, c: Color, flags: DRAW_TEXT_FORMAT) {
    unsafe {
        let old = SelectObject(dc, font(px).into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, rgb(c));
        let mut buf: Vec<u16> = s.encode_utf16().collect();
        DrawTextW(dc, &mut buf, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}
