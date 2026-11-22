//! Names the workspace a switch ended on, for a moment, on each screen it changed. The
//! look is Identify's: it never takes focus or a click.

use crate::geom::Rect;
use crate::overlay;
use std::cell::RefCell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const CLASS: PCWSTR = w!("SeamSwitchLabel");
const SHOWN_MS: u32 = 1200;
const TIMER_CLOSE: usize = 1;
/// Size in dips, at 96 dpi.
const WIDTH: i32 = 360;
const HEIGHT: i32 = 120;
const CORNER: i32 = 16;
const BACKGROUND: COLORREF = COLORREF(0x002A2A2A);
const TEXT: (u8, u8, u8) = (0xF3, 0xF3, 0xF3);
const MUTED: (u8, u8, u8) = (0xA0, 0xA0, 0xA0);

#[derive(Default)]
struct Showing {
    /// The windows up, one per screen.
    windows: Vec<isize>,
    name: String,
    /// "2 of 5".
    place: String,
}

thread_local! {
    static SHOWING: RefCell<Showing> = RefCell::new(Showing::default());
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let brush = CreateSolidBrush(BACKGROUND);
                FillRect(dc, &client, brush);
                let _ = DeleteObject(brush.into());
                let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
                let dip = |n: i32| (n as f64 * scale) as i32;
                let (name, place) = SHOWING.with(|s| {
                    let s = s.borrow();
                    (s.name.clone(), s.place.clone())
                });
                let flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;
                let line = |top: i32, bottom: i32| RECT {
                    left: client.left + dip(16),
                    top: client.top + dip(top),
                    right: client.right - dip(16),
                    bottom: client.top + dip(bottom),
                };
                overlay::text(dc, &name, line(22, 72), dip(36), true, TEXT, flags, false);
                overlay::text(dc, &place, line(76, 104), dip(16), false, MUTED, flags, false);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            // A click goes to the window under the label.
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_TIMER if wparam.0 == TIMER_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                SHOWING.with(|s| s.borrow_mut().windows.retain(|h| *h != hwnd.0 as isize));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Show `name` and `place` in the middle of each of `screens`, replacing labels still up.
pub fn show(name: &str, place: &str, screens: &[Rect]) {
    unsafe {
        let open = SHOWING.with(|s| std::mem::take(&mut s.borrow_mut().windows));
        for hwnd in open {
            let _ = DestroyWindow(HWND(hwnd as *mut _));
        }
        SHOWING.with(|s| {
            let mut s = s.borrow_mut();
            s.name = name.to_string();
            s.place = place.to_string();
        });
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        for screen in screens {
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                CLASS,
                w!("Seam workspace name"),
                WS_POPUP,
                screen.left,
                screen.top,
                WIDTH,
                HEIGHT,
                None,
                None,
                Some(hinst.into()),
                None,
            ) else {
                continue;
            };
            SHOWING.with(|s| s.borrow_mut().windows.push(hwnd.0 as isize));
            // Sized after creation, because the window now knows its screen's DPI.
            let dpi = GetDpiForWindow(hwnd).max(96) as i32;
            let (width, height) = (WIDTH * dpi / 96, HEIGHT * dpi / 96);
            let left = screen.left + (screen.width() - width) / 2;
            let top = screen.top + (screen.height() - height) / 2;
            let corner = CORNER * dpi / 96;
            let round = CreateRoundRectRgn(0, 0, width + 1, height + 1, corner, corner);
            SetWindowRgn(hwnd, Some(round), true);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), left, top, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SetTimer(Some(hwnd), TIMER_CLOSE, SHOWN_MS, None);
        }
    }
}
