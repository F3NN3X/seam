//! Draws how the screens meet on the real screens, so you can check the Displays
//! map against the desk. It closes on Esc, on any click, or after a few seconds.

use crate::geom::{facing_gap, touching, Rect, Side};
use crate::layout::Link;
use crate::log;
use crate::monitors::Mon;
use seam::api::LayoutView;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("SeamShowLayout");
const SHOWN_MS: u32 = 8000;
const ALPHA: u8 = 200;
const BAND: i32 = 8;
const BACKGROUND: (u8, u8, u8) = (0x20, 0x20, 0x20);
const TEXT: (u8, u8, u8) = (0xF3, 0xF3, 0xF3);
const SECONDARY: (u8, u8, u8) = (0xB0, 0xB0, 0xB0);
const ACCENT: (u8, u8, u8) = (0xFF, 0xB0, 0x20);
const MUTED: (u8, u8, u8) = (0x94, 0x94, 0x94);

pub(crate) type Color = (u8, u8, u8);

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Solid,
    Dashed,
    Dotted,
}

/// A band along one edge of a screen, over the part it shares with a neighbour.
struct Mark {
    side: Side,
    /// Where the band runs along the edge, in desktop pixels.
    along: (i32, i32),
    label: String,
    style: Style,
    color: Color,
}

struct Plane {
    rect: Rect,
    number: usize,
    name: String,
    marks: Vec<Mark>,
}

thread_local! {
    static PLANES: RefCell<Vec<(isize, Plane)>> = const { RefCell::new(Vec::new()) };
}

/// The id of the thread showing the layout, or 0.
static OPEN: AtomicU32 = AtomicU32::new(0);

fn shared(a: &Rect, b: &Rect, side: Side) -> Option<(i32, i32)> {
    let (from, to) = if side.along_is_y() {
        (a.top.max(b.top), a.bottom.min(b.bottom))
    } else {
        (a.left.max(b.left), a.right.min(b.right))
    };
    (to > from).then_some((from, to))
}

/// What to draw on each screen: the Seam view marks the links you lined up, the
/// Windows view the neighbours Windows has and the gaps between them.
fn plan(view: LayoutView, mons: &[Mon], links: &[Link]) -> Vec<Plane> {
    let mut planes: Vec<Plane> = mons
        .iter()
        .enumerate()
        .map(|(i, m)| Plane { rect: m.rect, number: i + 1, name: m.name.clone(), marks: Vec::new() })
        .collect();
    let mut add = |from: usize, to: usize, side: Side, label: String, style: Style, color: Color| {
        if let Some(along) = shared(&mons[from].rect, &mons[to].rect, side) {
            planes[from].marks.push(Mark { side, along, label, style, color });
        }
    };
    match view {
        LayoutView::Seam => {
            for l in links {
                let (style, color) = match (l.not_on_desk, l.glued) {
                    (true, _) => (Style::Dotted, MUTED),
                    (false, true) => (Style::Dashed, ACCENT),
                    (false, false) => (Style::Solid, ACCENT),
                };
                let label = |to: usize| match (l.not_on_desk, l.glued) {
                    (true, _) => "Not on your desk".to_string(),
                    (false, true) => format!("to screen {}, glued", to + 1),
                    (false, false) => format!("to screen {}", to + 1),
                };
                add(l.a, l.b, l.side, label(l.b), style, color);
                add(l.b, l.a, l.side.opposite(), label(l.a), style, color);
            }
        }
        LayoutView::Windows => {
            for i in 0..mons.len() {
                for j in i + 1..mons.len() {
                    let (a, b) = (&mons[i].rect, &mons[j].rect);
                    if let Some(side) = touching(a, b) {
                        add(i, j, side, format!("to screen {}", j + 1), Style::Solid, TEXT);
                        add(j, i, side.opposite(), format!("to screen {}", i + 1), Style::Solid, TEXT);
                    } else if let Some((side, gap)) = facing_gap(a, b) {
                        add(i, j, side, format!("to screen {}, {gap} px gap", j + 1), Style::Dashed, TEXT);
                        add(j, i, side.opposite(), format!("to screen {}, {gap} px gap", i + 1), Style::Dashed, TEXT);
                    }
                }
            }
        }
    }
    planes
}

pub(crate) fn rgb((r, g, b): Color) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub(crate) unsafe fn fill(dc: HDC, r: RECT, c: Color) {
    let brush = CreateSolidBrush(rgb(c));
    FillRect(dc, &r, brush);
    let _ = DeleteObject(brush.into());
}

pub(crate) unsafe fn text(dc: HDC, s: &str, mut r: RECT, px: i32, c: Color, flags: DRAW_TEXT_FORMAT) {
    let font = CreateFontW(
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
    );
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, rgb(c));
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(dc, &mut buf, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    let _ = DeleteObject(font.into());
}

/// A band from `along.0` to `along.1`, solid or as dashes or dots.
unsafe fn band(dc: HDC, mark: &Mark, origin: i32, size: (i32, i32), t: &dyn Fn(i32) -> i32) {
    let thick = t(BAND);
    let (on, off) = match mark.style {
        Style::Solid => (i32::MAX, 0),
        Style::Dashed => (t(24), t(12)),
        Style::Dotted => (t(4), t(8)),
    };
    let (from, to) = (mark.along.0 - origin, mark.along.1 - origin);
    let mut at = from;
    while at < to {
        let end = at.saturating_add(on).min(to);
        let r = match mark.side {
            Side::Left => RECT { left: 0, top: at, right: thick, bottom: end },
            Side::Right => RECT { left: size.0 - thick, top: at, right: size.0, bottom: end },
            Side::Top => RECT { left: at, top: 0, right: end, bottom: thick },
            Side::Bottom => RECT { left: at, top: size.1 - thick, right: end, bottom: size.1 },
        };
        fill(dc, r, mark.color);
        at = end + off.max(1);
    }
}

unsafe fn draw(dc: HDC, plane: &Plane, client: RECT, scale: f64) {
    let t = |v: i32| (v as f64 * scale).round() as i32;
    fill(dc, client, BACKGROUND);
    let size = (client.right, client.bottom);
    for mark in &plane.marks {
        let along_y = mark.side.along_is_y();
        let origin = if along_y { plane.rect.top } else { plane.rect.left };
        band(dc, mark, origin, size, &t);
        let mid = (mark.along.0 + mark.along.1) / 2 - origin;
        let (thick, gap, half) = (t(BAND), t(12), t(20));
        let (r, flags) = match mark.side {
            Side::Left => {
                (RECT { left: thick + gap, top: mid - half, right: size.0, bottom: mid + half }, DT_LEFT | DT_VCENTER)
            }
            Side::Right => (
                RECT { left: 0, top: mid - half, right: size.0 - thick - gap, bottom: mid + half },
                DT_RIGHT | DT_VCENTER,
            ),
            Side::Top => (
                RECT { left: mid - t(200), top: thick + gap, right: mid + t(200), bottom: thick + gap + 2 * half },
                DT_CENTER | DT_TOP,
            ),
            Side::Bottom => (
                RECT {
                    left: mid - t(200),
                    top: size.1 - thick - gap - 2 * half,
                    right: mid + t(200),
                    bottom: size.1 - thick - gap,
                },
                DT_CENTER | DT_BOTTOM,
            ),
        };
        text(dc, &mark.label, r, t(16), TEXT, flags);
    }
    let corner = t(48);
    let big = RECT { left: corner, top: corner, right: size.0, bottom: corner + t(120) };
    text(dc, &plane.number.to_string(), big, t(96), TEXT, DT_LEFT | DT_TOP | DT_NOCLIP);
    let name = RECT { left: corner, top: corner + t(120), right: size.0, bottom: corner + t(160) };
    text(dc, &plane.name, name, t(28), SECONDARY, DT_LEFT | DT_TOP | DT_NOCLIP);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
            PLANES.with(|planes| {
                if let Some((_, plane)) = planes.borrow().iter().find(|(h, _)| *h == hwnd.0 as isize) {
                    draw(dc, plane, client, scale);
                }
            });
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Esc and clicks end the show, and go on to whatever they were for.
unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let key = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if key.vkCode == VK_ESCAPE.0 as u32 {
            PostQuitMessage(0);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
        PostQuitMessage(0);
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Create one click-through window over a screen. It never takes focus.
unsafe fn open_window(plane: &Plane) -> Option<HWND> {
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        CLASS,
        w!("Seam layout"),
        WS_POPUP,
        plane.rect.left,
        plane.rect.top,
        1,
        1,
        None,
        None,
        Some(hinst.into()),
        None,
    )
    .ok()?;
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), ALPHA, LWA_ALPHA);
    // Sized after creation, because the window now knows its screen's DPI.
    let r = plane.rect;
    let _ =
        SetWindowPos(hwnd, Some(HWND_TOPMOST), r.left, r.top, r.width(), r.height(), SWP_NOACTIVATE | SWP_SHOWWINDOW);
    Some(hwnd)
}

fn run(planes: Vec<Plane>) {
    unsafe {
        let id = GetCurrentThreadId();
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let windows: Vec<HWND> = planes.iter().filter_map(|p| open_window(p)).collect();
        PLANES.with(|shown| {
            *shown.borrow_mut() = planes.into_iter().zip(&windows).map(|(p, h)| (h.0 as isize, p)).collect();
        });
        for h in &windows {
            let _ = InvalidateRect(Some(*h), None, true);
        }
        let hooks = [
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0),
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0),
        ];
        SetTimer(None, 0, SHOWN_MS, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for hook in hooks.into_iter().flatten() {
            let _ = UnhookWindowsHookEx(hook);
        }
        for h in windows {
            let _ = DestroyWindow(h);
        }
        let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Show `view` on every screen, replacing a show that's still up. The drawing and
/// the waiting for Esc or a click are on a thread of their own.
pub fn show(view: LayoutView, mons: &[Mon], links: &[Link]) {
    let open = OPEN.swap(0, Ordering::SeqCst);
    if open != 0 {
        unsafe {
            let _ = PostThreadMessageW(open, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    let planes = plan(view, mons, links);
    let spawned = std::thread::Builder::new().name("seam-showlayout".into()).spawn(move || {
        OPEN.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        run(planes);
    });
    if let Err(e) = spawned {
        log::info!("show layout: couldn't start: {e}");
    }
}
