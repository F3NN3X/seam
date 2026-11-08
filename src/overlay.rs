//! "Line Up" overlay: one full-screen window per monitor. For each shared border,
//! two coloured lines are drawn on the reference screen (A) and the user drags
//! the matching lines on the neighbour (B) until they run straight across the
//! bezel. Two line pairs fix both scale and offset; no measuring needed.

use crate::config::SavedBorder;
use crate::desk::{apart_neighbours, resolve};
use crate::geom::{anchor_points, overlap_in_a, predict, touching, EdgeMap, Side};
use crate::hook;
use crate::layout::{Link, Live};
use crate::monitors::Mon;
use std::cell::RefCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_CALIB_DONE: u32 = WM_APP + 2;
const CLASS: PCWSTR = w!("SeamOverlay");
const MIN_GAP: f64 = 8.0;

struct Step {
    a: usize,
    b: usize,
    /// Side of A that B is on (always Right or Bottom after normalising).
    side: Side,
    /// Fixed line positions on A (A-relative along the edge).
    a_pts: (f64, f64),
    /// Adjustable line positions on B (B-relative).
    b_pts: (f64, f64),
    /// The first guess, which R goes back to.
    guess: EdgeMap,
}

impl Step {
    fn map(&self) -> Option<EdgeMap> {
        EdgeMap::from_pairs(self.a_pts.0, self.b_pts.0, self.a_pts.1, self.b_pts.1)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Which {
    One(usize),
    Both,
}

struct Drag {
    which: Which,
    start: i32,
    b0: (f64, f64),
}

struct Calib {
    owner: HWND,
    mons: Vec<Mon>,
    steps: Vec<Step>,
    cur: usize,
    wins: Vec<HWND>,
    /// 0 / 1 = a single line, 2 = both.
    sel: usize,
    drag: Option<Drag>,
}

thread_local! {
    static CALIB: RefCell<Option<Calib>> = const { RefCell::new(None) };
    static RESULT: RefCell<Option<Vec<SavedBorder>>> = const { RefCell::new(None) };
}

pub fn is_active() -> bool {
    CALIB.with(|c| c.try_borrow().map(|c| c.is_some()).unwrap_or(true))
}

/// Accepted borders from the last finished run, if it was accepted.
pub fn take_result() -> Option<Vec<SavedBorder>> {
    RESULT.with(|r| r.borrow_mut().take())
}

fn saved_map(mons: &[Mon], a: usize, b: usize, side: Side, saved: &[SavedBorder]) -> Option<EdgeMap> {
    let (ia, ib) = (&mons[a].id, &mons[b].id);
    saved.iter().filter(|s| s.map.is_valid()).find_map(|s| {
        if &s.a == ia && &s.b == ib && s.side == side {
            Some(s.map)
        } else if &s.a == ib && &s.b == ia && s.side == side.opposite() {
            Some(s.map.inverse())
        } else {
            None
        }
    })
}

/// Every touching pair becomes one step, oriented so B is right of / below A,
/// then every pair that only meets on the desk.
fn build_steps(mons: &[Mon], saved: &[SavedBorder]) -> Vec<Step> {
    let mut pairs = Vec::new();
    for i in 0..mons.len() {
        for j in (i + 1)..mons.len() {
            let Some(side) = touching(&mons[i].rect, &mons[j].rect) else { continue };
            let (a, b, side) = match side {
                Side::Right | Side::Bottom => (i, j, side),
                Side::Left | Side::Top => (j, i, side.opposite()),
            };
            pairs.push((a, b, side, predict(&mons[a].rect, &mons[b].rect, side, mons[a].raw_dpi, mons[b].raw_dpi)));
        }
    }
    let links = resolve(mons, Some(saved));
    pairs.extend(apart_neighbours(mons, &links).into_iter().map(|p| (p.a, p.b, p.side, p.guess)));
    pairs
        .into_iter()
        .map(|(a, b, side, guess)| {
            let m = saved_map(mons, a, b, side, saved).unwrap_or(guess);
            let (la, lb) = (mons[a].rect.along_len(side), mons[b].rect.along_len(side));
            let a_pts = anchor_points(&m, la, lb);
            Step { a, b, side, a_pts, b_pts: (m.apply(a_pts.0), m.apply(a_pts.1)), guess }
        })
        .collect()
}

/// Push the in-progress maps to the hook so the pointer behaves as calibrated
/// while you adjust ("try while adjusting").
fn push_live(c: &Calib) {
    let links = c
        .steps
        .iter()
        .filter_map(|s| s.map().map(|map| Link { a: s.a, b: s.b, side: s.side, map, glued: false }))
        .collect();
    hook::set_live(Live { rects: c.mons.iter().map(|m| m.rect).collect(), links });
}

/// Start the overlay. Returns false if there's nothing to line up.
pub fn start(owner: HWND, mons: Vec<Mon>, saved: &[SavedBorder]) -> bool {
    if is_active() {
        return true;
    }
    let steps = build_steps(&mons, saved);
    if steps.is_empty() {
        return false;
    }
    let mut wins = Vec::new();
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        for (i, m) in mons.iter().enumerate() {
            let r = m.rect;
            if let Ok(h) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS,
                w!("Seam"),
                WS_POPUP,
                r.left,
                r.top,
                r.width(),
                r.height(),
                None,
                None,
                Some(hinst.into()),
                None,
            ) {
                SetWindowLongPtrW(h, GWLP_USERDATA, i as isize);
                wins.push(h);
            } else {
                wins.push(HWND::default());
            }
        }
    }
    let focus = wins[steps[0].b];
    let calib = Calib { owner, mons, steps, cur: 0, wins: wins.clone(), sel: 0, drag: None };
    push_live(&calib);
    CALIB.with(|c| *c.borrow_mut() = Some(calib));
    hook::enable(true);
    unsafe {
        for h in &wins {
            if !h.is_invalid() {
                let _ = ShowWindow(*h, SW_SHOW);
            }
        }
        let _ = SetForegroundWindow(focus);
        let _ = SetFocus(Some(focus));
    }
    true
}

/// Close the overlay without saving (e.g. the monitors changed mid-way).
pub fn cancel() {
    finish(false);
}

fn finish(accept: bool) {
    let Some(c) = CALIB.with(|c| c.try_borrow_mut().ok().and_then(|mut c| c.take())) else { return };
    if accept {
        let res = c
            .steps
            .iter()
            .filter_map(|s| {
                s.map().map(|map| SavedBorder {
                    a: c.mons[s.a].id.clone(),
                    b: c.mons[s.b].id.clone(),
                    side: s.side,
                    map,
                    glued: false,
                })
            })
            .collect();
        RESULT.with(|r| *r.borrow_mut() = Some(res));
    }
    unsafe {
        let _ = ReleaseCapture();
        for h in &c.wins {
            if !h.is_invalid() {
                let _ = DestroyWindow(*h);
            }
        }
        let _ = PostMessageW(Some(c.owner), WM_APP_CALIB_DONE, WPARAM(accept as usize), LPARAM(0));
    }
}

fn redraw_all(c: &Calib) {
    for h in &c.wins {
        unsafe {
            let _ = InvalidateRect(Some(*h), None, false);
        }
    }
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// Screen coordinate of the pointer along the current border's axis.
fn pointer_along(side: Side) -> i32 {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    if side.along_is_y() {
        p.y
    } else {
        p.x
    }
}

fn set_b(step: &mut Step, which: Which, base: (f64, f64), delta: f64) {
    match which {
        Which::Both => step.b_pts = (base.0 + delta, base.1 + delta),
        Which::One(0) => step.b_pts.0 = (base.0 + delta).min(step.b_pts.1 - MIN_GAP),
        Which::One(_) => step.b_pts.1 = (base.1 + delta).max(step.b_pts.0 + MIN_GAP),
    }
}

enum After {
    Nothing,
    Capture(HWND),
    Release,
    /// Step changed: drop any capture and focus the new B screen.
    Focus(HWND),
    Finish(bool),
}

fn handle(hwnd: HWND, idx: usize, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<(LRESULT, After)> {
    CALIB.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        let c = guard.as_mut()?;
        let cur = c.cur;
        let n_steps = c.steps.len();
        let step = &mut c.steps[cur];
        let is_b = idx == step.b;
        let side = step.side;
        let mut after = After::Nothing;
        let mut changed = false;
        match msg {
            WM_LBUTTONDOWN if is_b => {
                let x = (lparam.0 & 0xffff) as i16 as f64;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as f64;
                let pos = if side.along_is_y() { y } else { x };
                let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f64;
                let reach = 24.0 * dpi / 96.0;
                let (d0, d1) = ((pos - step.b_pts.0).abs(), (pos - step.b_pts.1).abs());
                let which = if key_down(VK_CONTROL.0) {
                    Which::Both
                } else if d0.min(d1) <= reach {
                    Which::One(if d0 <= d1 { 0 } else { 1 })
                } else {
                    Which::Both
                };
                c.sel = match which {
                    Which::One(i) => i,
                    Which::Both => 2,
                };
                c.drag = Some(Drag { which, start: pointer_along(side), b0: step.b_pts });
                after = After::Capture(hwnd);
                changed = true;
            }
            WM_MOUSEMOVE => {
                if let Some(d) = &c.drag {
                    let delta = (pointer_along(side) - d.start) as f64;
                    let (which, b0) = (d.which, d.b0);
                    set_b(step, which, b0, delta);
                    changed = true;
                }
            }
            WM_LBUTTONUP => {
                c.drag = None;
                if unsafe { GetCapture() } == hwnd {
                    after = After::Release;
                }
            }
            WM_CAPTURECHANGED => {
                c.drag = None;
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wparam.0 as u16);
                let amount = if key_down(VK_SHIFT.0) { 10.0 } else { 1.0 };
                let nudge = match vk {
                    VK_UP | VK_LEFT => Some(-amount),
                    VK_DOWN | VK_RIGHT => Some(amount),
                    _ => None,
                };
                if let Some(delta) = nudge {
                    let which = if key_down(VK_CONTROL.0) || c.sel == 2 { Which::Both } else { Which::One(c.sel) };
                    let base = step.b_pts;
                    set_b(step, which, base, delta);
                    changed = true;
                } else {
                    match vk {
                        VK_ESCAPE => after = After::Finish(false),
                        VK_RETURN => {
                            if cur + 1 < n_steps {
                                c.cur += 1;
                                c.sel = 0;
                                c.drag = None;
                                after = After::Focus(c.wins[c.steps[c.cur].b]);
                                changed = true;
                            } else {
                                after = After::Finish(true);
                            }
                        }
                        VK_BACK if cur > 0 => {
                            c.cur -= 1;
                            c.sel = 0;
                            c.drag = None;
                            after = After::Focus(c.wins[c.steps[c.cur].b]);
                            changed = true;
                        }
                        VK_TAB => {
                            c.sel = (c.sel + 1) % 3;
                            changed = true;
                        }
                        VK_R => {
                            let (ma, mb) = (&c.mons[step.a], &c.mons[step.b]);
                            let m = step.guess;
                            step.a_pts = anchor_points(&m, ma.rect.along_len(side), mb.rect.along_len(side));
                            step.b_pts = (m.apply(step.a_pts.0), m.apply(step.a_pts.1));
                            changed = true;
                        }
                        _ => return None,
                    }
                }
            }
            WM_SETCURSOR => {
                let id = if is_b {
                    if side.along_is_y() {
                        IDC_SIZENS
                    } else {
                        IDC_SIZEWE
                    }
                } else {
                    IDC_ARROW
                };
                unsafe {
                    SetCursor(LoadCursorW(None, id).ok());
                }
                return Some((LRESULT(1), After::Nothing));
            }
            _ => return None,
        }
        if changed {
            push_live(c);
            redraw_all(c);
        }
        Some((LRESULT(0), after))
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    match msg {
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            paint(hwnd, idx);
            return LRESULT(0);
        }
        // Keep the window exactly on its monitor; never rescale it.
        WM_DPICHANGED => return LRESULT(0),
        WM_CLOSE => {
            finish(false);
            return LRESULT(0);
        }
        _ => {}
    }
    if let Some((res, after)) = handle(hwnd, idx, msg, wparam, lparam) {
        // Win32 calls that send messages back to us happen after the borrow ends.
        match after {
            After::Nothing => {}
            After::Capture(h) => {
                SetCapture(h);
            }
            After::Release => {
                let _ = ReleaseCapture();
            }
            After::Focus(h) => {
                let _ = ReleaseCapture();
                let _ = SetForegroundWindow(h);
                let _ = SetFocus(Some(h));
            }
            After::Finish(ok) => finish(ok),
        }
        return res;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ---------------------------------------------------------------- drawing --

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

const BG: (u8, u8, u8) = (14, 16, 20);
const PANEL: (u8, u8, u8) = (30, 33, 40);
const GUIDE: (u8, u8, u8) = (64, 70, 82);
const EDGE: (u8, u8, u8) = (255, 176, 32);
const LINE_COLORS: [(u8, u8, u8); 2] = [(0, 200, 255), (255, 72, 176)];

unsafe fn fill(dc: HDC, r: RECT, c: (u8, u8, u8)) {
    let br = CreateSolidBrush(rgb(c.0, c.1, c.2));
    FillRect(dc, &r, br);
    let _ = DeleteObject(br.into());
}

#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn text(
    dc: HDC,
    s: &str,
    mut r: RECT,
    px: i32,
    bold: bool,
    c: (u8, u8, u8),
    flags: DRAW_TEXT_FORMAT,
    panel: bool,
) {
    let weight = if bold { FW_SEMIBOLD.0 } else { FW_NORMAL.0 } as i32;
    let font = CreateFontW(
        -px,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        0,
        w!("Segoe UI"),
    );
    let old = SelectObject(dc, font.into());
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    if panel {
        let mut measure = r;
        DrawTextW(dc, &mut buf, &mut measure, flags | DT_CALCRECT);
        let (w, h) = (measure.right - measure.left, measure.bottom - measure.top);
        let cx = (r.left + r.right) / 2;
        let pad = px;
        let top = r.top;
        fill(
            dc,
            RECT { left: cx - w / 2 - pad, top: top - pad, right: cx + w / 2 + pad, bottom: top + h + pad },
            PANEL,
        );
        r = RECT { left: cx - w / 2, top, right: cx + w / 2 + 1, bottom: top + h };
    }
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, rgb(c.0, c.1, c.2));
    DrawTextW(dc, &mut buf, &mut r, flags);
    SelectObject(dc, old);
    let _ = DeleteObject(font.into());
}

/// A line across the whole client area, perpendicular to the shared edge.
unsafe fn line(dc: HDC, w: i32, h: i32, along_is_y: bool, pos: f64, thick: i32, c: (u8, u8, u8)) {
    let p = pos.round() as i32 - thick / 2;
    let r = if along_is_y {
        RECT { left: 0, top: p, right: w, bottom: p + thick }
    } else {
        RECT { left: p, top: 0, right: p + thick, bottom: h }
    };
    fill(dc, r, c);
}

/// Small numbered tag next to a line, on the shared-edge side of the screen.
#[allow(clippy::too_many_arguments)]
unsafe fn tag(dc: HDC, w: i32, h: i32, edge: Side, pos: f64, label: &str, s: f64, c: (u8, u8, u8)) {
    let size = (34.0 * s) as i32;
    let inset = (28.0 * s) as i32;
    let p = pos.round() as i32;
    let (x, y) = match edge {
        Side::Right => (w - inset - size, p - size - (6.0 * s) as i32),
        Side::Left => (inset, p - size - (6.0 * s) as i32),
        Side::Bottom => (p + (6.0 * s) as i32, h - inset - size),
        Side::Top => (p + (6.0 * s) as i32, inset),
    };
    let r = RECT { left: x, top: y, right: x + size, bottom: y + size };
    fill(dc, r, c);
    text(dc, label, r, (20.0 * s) as i32, true, BG, DT_CENTER | DT_VCENTER | DT_SINGLELINE, false);
}

fn edge_strip(w: i32, h: i32, edge: Side, t: i32) -> RECT {
    match edge {
        Side::Right => RECT { left: w - t, top: 0, right: w, bottom: h },
        Side::Left => RECT { left: 0, top: 0, right: t, bottom: h },
        Side::Bottom => RECT { left: 0, top: h - t, right: w, bottom: h },
        Side::Top => RECT { left: 0, top: 0, right: w, bottom: t },
    }
}

unsafe fn paint(hwnd: HWND, idx: usize) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let (w, h) = (client.right, client.bottom);
    let mem = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old = SelectObject(mem, bmp.into());
    fill(mem, client, BG);
    let s = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;

    CALIB.with(|cell| {
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(c) = guard.as_ref() else { return };
        let Some(mon) = c.mons.get(idx) else { return };
        let step = &c.steps[c.cur];
        let along_y = step.side.along_is_y();

        // Big screen number, like Shim.
        let pad = (36.0 * s) as i32;
        text(mem, &format!("{}", idx + 1), RECT { left: pad, top: pad / 2, right: w, bottom: h }, (150.0 * s) as i32, true, (58, 63, 74), DT_LEFT | DT_TOP | DT_SINGLELINE, false);
        text(mem, &mon.name, RECT { left: pad, top: pad / 2 + (170.0 * s) as i32, right: w, bottom: h }, (22.0 * s) as i32, false, (110, 116, 130), DT_LEFT | DT_TOP | DT_SINGLELINE, false);

        let role_a = idx == step.a;
        let role_b = idx == step.b;
        let center = RECT { left: w / 6, top: h / 2 - (110.0 * s) as i32, right: w - w / 6, bottom: h };
        if role_a || role_b {
            let (la, lb) = (c.mons[step.a].rect.along_len(step.side), c.mons[step.b].rect.along_len(step.side));
            let map = step.map();
            // Faint guide lines through the physical overlap: they all straighten
            // out once the two main lines are right.
            if let Some(m) = map {
                if let Some((lo, hi)) = overlap_in_a(&m, la, lb) {
                    for k in 0..=10 {
                        let a = lo + (hi - lo) * k as f64 / 10.0;
                        let pos = if role_a { a } else { m.apply(a) };
                        line(mem, w, h, along_y, pos, (1.0 * s).max(1.0) as i32, GUIDE);
                    }
                }
            }
            let my_edge = if role_a { step.side } else { step.side.opposite() };
            fill(mem, edge_strip(w, h, my_edge, (6.0 * s) as i32), EDGE);
            let pts = if role_a { step.a_pts } else { step.b_pts };
            for (i, pos) in [pts.0, pts.1].into_iter().enumerate() {
                let selected = role_b && (c.sel == i || c.sel == 2);
                let thick = ((if selected { 5.0 } else { 3.0 }) * s) as i32;
                line(mem, w, h, along_y, pos, thick.max(2), LINE_COLORS[i]);
                tag(mem, w, h, my_edge, pos, &format!("{}", i + 1), s, LINE_COLORS[i]);
            }
            let last = c.cur + 1 == c.steps.len();
            let msg = if role_b {
                format!(
                    "Line up screen {b} with screen {a}\n\n\
                     Drag the lines on this screen until each one continues straight across the bezel from screen {a}.\n\
                     Grab a line to move just that one. Drag anywhere else to move both.\n\n\
                     Arrows nudge  ·  Shift ×10  ·  Ctrl = both  ·  Tab switches line  ·  R resets\n\
                     The pointer already follows your lines, so try crossing as you go.\n\n\
                     Enter: {next}   ·   Backspace: back   ·   Esc: cancel",
                    a = step.a + 1,
                    b = step.b + 1,
                    next = if last { "done" } else { "next border" },
                )
            } else {
                format!("Reference screen\n\nThe lines here stay put. Adjust the matching lines on screen {}.", step.b + 1)
            };
            text(mem, &msg, center, (20.0 * s) as i32, false, (226, 230, 238), DT_CENTER | DT_WORDBREAK, true);
        } else {
            text(mem, "Not part of this step", center, (22.0 * s) as i32, false, (110, 116, 130), DT_CENTER | DT_WORDBREAK, false);
        }
        let footer = RECT { left: 0, top: h - (64.0 * s) as i32, right: w, bottom: h };
        text(
            mem,
            &format!("Seam  ·  border {} of {}", c.cur + 1, c.steps.len()),
            footer,
            (16.0 * s) as i32,
            false,
            (110, 116, 130),
            DT_CENTER | DT_SINGLELINE,
            false,
        );
    });

    let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
    SelectObject(mem, old);
    let _ = DeleteObject(bmp.into());
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}
