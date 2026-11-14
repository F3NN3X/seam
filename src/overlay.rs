//! "Line Up" overlay: one full-screen window per monitor. For each shared border,
//! two coloured lines are drawn on the reference screen (A) and the user drags
//! the matching lines on the neighbour (B) until they run straight across the
//! bezel. Two line pairs fix both scale and offset; no measuring needed.

use crate::config::SavedBorder;
use crate::desk::{glue_candidates, resolve};
use crate::geom::{EdgeMap, Side, anchor_points, map_from_offset, overlap_in_a, predict, predict_centred, touching};
use crate::hook;
use crate::layout::Link;
use crate::monitors::Mon;
use std::cell::RefCell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

mod draw;
mod input;

use draw::paint;
use input::handle;

pub(crate) use draw::text;

pub const WM_APP_CALIB_DONE: u32 = WM_APP + 2;
const CLASS: PCWSTR = w!("SeamOverlay");
const MIN_GAP: f64 = 8.0;
/// How deep the stripe band is, and how far apart its stripes are, on the desk.
const STRIPE_BAND_MM: f64 = 60.0;
const STRIPE_GAP_MM: f64 = 10.0;

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
    /// For screens with a gap between them in Windows, whether to glue them.
    /// `None` for screens that touch.
    glue: Option<bool>,
    /// The saved border for this pair is glued.
    was_glued: bool,
    /// For screens that touch in Windows, that they don't on the desk, so the
    /// pointer stops here.
    not_on_desk: bool,
}

impl Step {
    fn map(&self) -> Option<EdgeMap> {
        EdgeMap::from_pairs(self.a_pts.0, self.b_pts.0, self.a_pts.1, self.b_pts.1)
    }

    /// False for a gap the user hasn't glued, which is skipped.
    fn on(&self) -> bool {
        self.glue != Some(false)
    }

    /// Whether the border is saved as glued. A glued pair that Windows has since
    /// closed up stays glued.
    fn glued(&self) -> bool {
        self.glue.unwrap_or(self.was_glued)
    }

    /// Whether the lines can move: not for an unglued gap or a border that
    /// isn't on the desk.
    fn adjustable(&self) -> bool {
        self.on() && !self.not_on_desk
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

/// The typed offset being entered on B's screen.
struct Typing {
    text: String,
    /// B's start edge is after A's (below / to the right) rather than before it.
    after: bool,
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
    /// `lineup_measure`: M offers a typed offset.
    measure: bool,
    typing: Option<Typing>,
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

fn saved_glued(mons: &[Mon], a: usize, b: usize, saved: &[SavedBorder]) -> bool {
    let (ia, ib) = (&mons[a].id, &mons[b].id);
    saved.iter().any(|s| s.glued && ((&s.a, &s.b) == (ia, ib) || (&s.a, &s.b) == (ib, ia)))
}

fn saved_not_on_desk(mons: &[Mon], a: usize, b: usize, saved: &[SavedBorder]) -> bool {
    let (ia, ib) = (&mons[a].id, &mons[b].id);
    saved.iter().any(|s| s.not_on_desk && ((&s.a, &s.b) == (ia, ib) || (&s.a, &s.b) == (ib, ia)))
}

/// `a` and `b` swapped if needed so B is right of / below A.
fn oriented(a: usize, b: usize, side: Side) -> (usize, usize, Side) {
    match side {
        Side::Right | Side::Bottom => (a, b, side),
        Side::Left | Side::Top => (b, a, side.opposite()),
    }
}

/// Every touching pair becomes one step, oriented so B is right of / below A,
/// then every pair that could be glued across a gap in Windows. Those start
/// glued when the screens' sizes put them side by side or they were glued
/// before, and off when only the pixel layout has them facing.
fn build_steps(mons: &[Mon], saved: &[SavedBorder]) -> Vec<Step> {
    let mut pairs = Vec::new();
    for i in 0..mons.len() {
        for j in (i + 1)..mons.len() {
            let Some(side) = touching(&mons[i].rect, &mons[j].rect) else { continue };
            let (a, b, side) = oriented(i, j, side);
            let guess = predict(&mons[a].rect, &mons[b].rect, side, mons[a].raw_dpi, mons[b].raw_dpi);
            pairs.push((a, b, side, guess, None));
        }
    }
    let links = resolve(mons, Some(saved));
    for c in glue_candidates(mons, &links) {
        let on = c.apart || saved_glued(mons, c.a, c.b, saved);
        pairs.push((c.a, c.b, c.side, c.guess, Some(on)));
    }
    // A glued border holds past what detection finds, so Line Up must offer it too.
    for l in links.iter().filter(|l| l.glued && touching(&mons[l.a].rect, &mons[l.b].rect).is_none()) {
        let (a, b, side) = oriented(l.a, l.b, l.side);
        if !pairs.iter().any(|p| (p.0, p.1) == (a, b)) {
            let guess = predict_centred(&mons[a].rect, &mons[b].rect, side, mons[a].raw_dpi, mons[b].raw_dpi);
            pairs.push((a, b, side, guess, Some(true)));
        }
    }
    pairs
        .into_iter()
        .map(|(a, b, side, guess, glue)| {
            let m = saved_map(mons, a, b, side, saved).unwrap_or(guess);
            let (la, lb) = (mons[a].rect.along_len(side), mons[b].rect.along_len(side));
            let a_pts = anchor_points(&m, la, lb);
            let was_glued = saved_glued(mons, a, b, saved);
            let not_on_desk = glue.is_none() && saved_not_on_desk(mons, a, b, saved);
            let b_pts = (m.apply(a_pts.0), m.apply(a_pts.1));
            Step { a, b, side, a_pts, b_pts, guess, glue, was_glued, not_on_desk }
        })
        .collect()
}

/// Push the in-progress maps to the hook so the pointer behaves as calibrated
/// while you adjust ("try while adjusting").
fn push_live(c: &Calib) {
    let links = c
        .steps
        .iter()
        .filter(|s| s.on())
        .filter_map(|s| {
            s.map().map(|map| Link { a: s.a, b: s.b, side: s.side, map, glued: s.glued(), not_on_desk: s.not_on_desk })
        })
        .collect();
    hook::set_borders(c.mons.iter().map(|m| m.rect).collect(), links);
}

/// Start the overlay. Returns false if there's nothing to line up.
pub fn start(owner: HWND, mons: Vec<Mon>, saved: &[SavedBorder], measure: bool) -> bool {
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
            match CreateWindowExW(
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
                Ok(h) => {
                    SetWindowLongPtrW(h, GWLP_USERDATA, i as isize);
                    wins.push(h);
                }
                _ => {
                    wins.push(HWND::default());
                }
            }
        }
    }
    let focus = wins[steps[0].b];
    let calib = Calib { owner, mons, steps, cur: 0, wins: wins.clone(), sel: 0, drag: None, measure, typing: None };
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
            .filter(|s| s.on())
            .filter_map(|s| {
                s.map().map(|map| SavedBorder {
                    a: c.mons[s.a].id.clone(),
                    b: c.mons[s.b].id.clone(),
                    side: s.side,
                    map,
                    glued: s.glued() && !s.not_on_desk,
                    not_on_desk: s.not_on_desk,
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

/// Millimetres per pixel of each screen of the step, `None` if either reports no size.
fn sizes(mons: &[Mon], step: &Step) -> Option<(f64, f64)> {
    let mm = |m: &Mon| (m.raw_dpi > 0.0).then(|| 25.4 / m.raw_dpi);
    Some((mm(&mons[step.a])?, mm(&mons[step.b])?))
}

enum After {
    Nothing,
    Capture(HWND),
    Release,
    /// Step changed: drop any capture and focus the new B screen.
    Focus(HWND),
    Finish(bool),
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
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
}
