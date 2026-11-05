//! The zone overlay: click-through layered windows, painted into premultiplied
//! bitmaps. No blur and no animation. Each screen has a base window with every
//! zone and number, drawn once, and the lit zone is its own small window drawn
//! once per zone, so changing what is lit only shows and hides windows.

use crate::{log, overlay};
use seam::zones::Rect;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("SeamZones");
const RADIUS: f32 = 6.;
const OUTLINE: f32 = 1.5;
/// Zone outlines and numbers, `#F3F3F3` at 40%, on a `#202020` fill at 35%.
const INK: (u8, u8, u8) = (0xF3, 0xF3, 0xF3);
const SHADE: (u8, u8, u8) = (0x20, 0x20, 0x20);
/// The seam colour, `#FFB020`.
const AMBER: (u8, u8, u8) = (0xFF, 0xB0, 0x20);

type Key = (i32, i32, i32, i32);
type MaskKey = (String, i32, i32, i32);

fn key(r: &Rect) -> Key {
    (r.x, r.y, r.w, r.h)
}

/// What is drawn for one screen's base window, and the lit windows made for it.
struct Layers {
    work: Rect,
    scale: u32,
    zones: Vec<Rect>,
    lit: HashMap<Key, HWND>,
    shown: Option<Key>,
}

thread_local! {
    static LAYERS: RefCell<HashMap<isize, Layers>> = RefCell::new(HashMap::new());
    /// Digit coverage by label, glyph size and mask size.
    static MASKS: RefCell<HashMap<MaskKey, Vec<u8>>> = RefCell::new(HashMap::new());
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// A hidden overlay window. It is placed and shown by `paint`.
pub fn create() -> Option<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let styles = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        CreateWindowExW(styles, CLASS, w!("Seam zones"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst.into()), None)
            .ok()
    }
}

/// Hide the base window and whatever is lit.
pub fn hide(hwnd: HWND) {
    LAYERS.with(|layers| {
        if let Some(layers) = layers.borrow_mut().get_mut(&(hwnd.0 as isize)) {
            if let Some(lit) = layers.shown.take().and_then(|k| layers.lit.get(&k)) {
                hide_window(*lit);
            }
        }
    });
    hide_window(hwnd);
}

fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn destroy(hwnd: HWND) {
    let layers = LAYERS.with(|layers| layers.borrow_mut().remove(&(hwnd.0 as isize)));
    unsafe {
        for lit in layers.iter().flat_map(|l| l.lit.values()) {
            let _ = DestroyWindow(*lit);
        }
        let _ = DestroyWindow(hwnd);
    }
}

/// Premultiplied ARGB.
fn pm(rgb: (u8, u8, u8), alpha: f32) -> u32 {
    let c = |v: u8| (v as f32 * alpha).round() as u32;
    ((alpha * 255.).round() as u32) << 24 | c(rgb.0) << 16 | c(rgb.1) << 8 | c(rgb.2)
}

fn scale(px: u32, k: f32) -> u32 {
    let c = |shift: u32| (((px >> shift) & 0xFF) as f32 * k).round() as u32;
    c(24) << 24 | c(16) << 16 | c(8) << 8 | c(0)
}

/// `src` over `dst`, both premultiplied.
fn over(dst: u32, src: u32) -> u32 {
    let inverse = 255 - (src >> 24);
    let c = |shift: u32| {
        let (s, d) = ((src >> shift) & 0xFF, (dst >> shift) & 0xFF);
        (s + (d * inverse + 127) / 255).min(255)
    };
    c(24) << 24 | c(16) << 16 | c(8) << 8 | c(0)
}

/// An empty bitmap to draw on. Zones never overlap, so fills are stored, not blended.
struct Canvas<'a> {
    px: &'a mut [u32],
    w: i32,
    h: i32,
}

impl Canvas<'_> {
    fn blend(&mut self, x: i32, y: i32, src: u32) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let at = (y * self.w + x) as usize;
            self.px[at] = over(self.px[at], src);
        }
    }

    /// A rounded rectangle with an optional outline on its inside edge. Only
    /// the rim is measured pixel by pixel. The middle is stored in whole runs.
    fn rounded(&mut self, r: Rect, radius: f32, fill: u32, outline: Option<(u32, f32)>, s: f32) {
        let (radius, width) = (radius * s, outline.map_or(0., |o| o.1 * s));
        let (half_w, half_h) = (r.w as f32 / 2., r.h as f32 / 2.);
        let radius = radius.min(half_w).min(half_h);
        let (cx, cy) = (r.x as f32 + half_w, r.y as f32 + half_h);
        let distance = |x: i32, y: i32| {
            let qx = (x as f32 + 0.5 - cx).abs() - (half_w - radius);
            let qy = (y as f32 + 0.5 - cy).abs() - (half_h - radius);
            qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
        };
        let rim = (radius + width).ceil() as i32 + 2;
        let (left, right) = (r.x.max(0), (r.x + r.w).min(self.w));
        let (inner_left, inner_right) = ((r.x + rim).clamp(left, right), (r.x + r.w - rim).clamp(left, right));
        for y in r.y.max(0)..(r.y + r.h).min(self.h) {
            let near_y = y < r.y + rim || y >= r.y + r.h - rim;
            let row = (y * self.w) as usize;
            let spans = if !near_y && inner_left < inner_right {
                self.px[row + inner_left as usize..row + inner_right as usize].fill(fill);
                [left..inner_left, inner_right..right]
            } else {
                [left..right, 0..0]
            };
            for x in spans.into_iter().flatten() {
                let d = distance(x, y);
                let inside = (0.5 - d).clamp(0., 1.);
                if inside <= 0. {
                    continue;
                }
                self.blend(x, y, scale(fill, inside));
                if let Some((color, _)) = outline {
                    let ring = inside - (0.5 - (d + width)).clamp(0., 1.);
                    self.blend(x, y, scale(color, ring));
                }
            }
        }
    }

    /// Lay `label` in `ink` over the middle of `r`, using GDI only for the glyph shapes.
    fn label(&mut self, r: Rect, label: &str, ink: u32, s: f32) {
        let (mw, mh, size) = ((80. * s) as i32, (56. * s) as i32, (28. * s) as i32);
        let (left, top) = (r.x + (r.w - mw) / 2, r.y + (r.h - mh) / 2);
        MASKS.with(|masks| {
            let mut masks = masks.borrow_mut();
            let mask = masks
                .entry((label.to_string(), size, mw, mh))
                .or_insert_with(|| glyph_mask(label, size, mw, mh).unwrap_or_default());
            for (i, cover) in mask.iter().enumerate().filter(|(_, c)| **c > 0) {
                let (x, y) = (i as i32 % mw, i as i32 / mw);
                self.blend(left + x, top + y, scale(ink, *cover as f32 / 255.));
            }
        });
    }
}

fn dib_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// How much of each pixel `label` covers, drawn white on black.
fn glyph_mask(label: &str, size: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bits = std::ptr::null_mut();
        let mut mask = None;
        if let Ok(bitmap) = CreateDIBSection(Some(dc), &dib_info(w, h), DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                let old = SelectObject(dc, bitmap.into());
                let flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
                let area = RECT { left: 0, top: 0, right: w, bottom: h };
                overlay::text(dc, label, area, size, false, (255, 255, 255), flags, false);
                let _ = GdiFlush();
                let pixels = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
                let cover = |p: &u32| ((((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF)) / 3) as u8;
                mask = Some(pixels.iter().map(cover).collect());
                SelectObject(dc, old);
            }
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        mask
    }
}

/// Draw into a bitmap the size of `area`, and make it `hwnd`'s picture there.
/// This neither shows nor hides the window.
fn render(hwnd: HWND, area: Rect, draw: impl FnOnce(&mut Canvas)) {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bits = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(dc), &dib_info(area.w, area.h), DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                let old = SelectObject(dc, bitmap.into());
                let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (area.w * area.h) as usize);
                draw(&mut Canvas { px: pixels, w: area.w, h: area.h });
                let _ = GdiFlush();
                let (at, size) = (POINT { x: area.x, y: area.y }, SIZE { cx: area.w, cy: area.h });
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

/// A hidden window holding the lit look of `zone`.
fn lit_window(zone: Rect, s: f32) -> Option<HWND> {
    let hwnd = create()?;
    render(hwnd, zone, |canvas| {
        let area = Rect { x: 0, y: 0, ..zone };
        canvas.rounded(area, RADIUS, pm(AMBER, 0.1), Some((pm(AMBER, 1.), OUTLINE)), s);
    });
    Some(hwnd)
}

fn show_on_top(hwnd: HWND) {
    unsafe {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
    }
}

/// The base: every zone and its number, in screen pixels turned local to `work`.
fn draw_base(canvas: &mut Canvas, work: Rect, zones: &[Rect], s: f32) {
    let (fill, outline, ink) = (pm(SHADE, 0.35), pm(INK, 0.4), pm(INK, 0.4));
    for (i, zone) in zones.iter().enumerate() {
        let zone = Rect { x: zone.x - work.x, y: zone.y - work.y, ..*zone };
        canvas.rounded(zone, RADIUS, fill, Some((outline, OUTLINE)), s);
        canvas.label(zone, &(i + 1).to_string(), ink, s);
    }
}

/// Show the overlay over `work` with `highlight` lit. Zones and the highlight
/// are in screen pixels, and `s` is the screen's DPI over 96. The base and
/// each zone's lit window are drawn the first time they are needed, so later
/// changes only show and hide windows.
pub fn paint(hwnd: HWND, work: Rect, s: f32, zones: &[Rect], highlight: Option<Rect>) {
    let started = log::enabled().then(Instant::now);
    let (mut redrawn, mut made) = (false, false);
    LAYERS.with(|layers| {
        let mut layers = layers.borrow_mut();
        let id = hwnd.0 as isize;
        let stale = layers.get(&id).is_none_or(|l| l.work != work || l.scale != s.to_bits() || l.zones != zones);
        if stale {
            for lit in layers.remove(&id).iter().flat_map(|l| l.lit.values()) {
                unsafe {
                    let _ = DestroyWindow(*lit);
                }
            }
            render(hwnd, work, |canvas| draw_base(canvas, work, zones, s));
            let lit = zones.iter().filter_map(|z| Some((key(z), lit_window(*z, s)?))).collect();
            layers.insert(id, Layers { work, scale: s.to_bits(), zones: zones.to_vec(), lit, shown: None });
            redrawn = true;
        }
        let Some(current) = layers.get_mut(&id) else { return };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let wanted = highlight.as_ref().map(key);
        if current.shown != wanted {
            if let Some(previous) = current.shown.and_then(|k| current.lit.get(&k)) {
                hide_window(*previous);
            }
            if let (Some(zone), Some(k)) = (highlight, wanted) {
                // A span: its bounding box isn't one of the zones.
                if let Entry::Vacant(slot) = current.lit.entry(k) {
                    if let Some(window) = lit_window(zone, s) {
                        slot.insert(window);
                        made = true;
                    }
                }
                if let Some(window) = current.lit.get(&k) {
                    show_on_top(*window);
                }
            }
            current.shown = wanted;
        }
    });
    if let Some(t0) = started {
        let us = t0.elapsed().as_secs_f64() * 1e6;
        log::info!("zone overlay: {us:.0}us, base redrawn: {redrawn}, lit window drawn: {made}");
    }
}
