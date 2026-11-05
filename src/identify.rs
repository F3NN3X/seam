//! Shows a screen's Line Up number for a moment, so the Displays map can point
//! at a real screen.

use crate::geom::Rect;
use crate::overlay;
use std::cell::Cell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("SeamIdentify");
const SHOWN_MS: u32 = 1500;
const TIMER_CLOSE: usize = 1;
const SIZE: i32 = 240;
const BACKGROUND: COLORREF = COLORREF(0x002A2A2A);
const TEXT: (u8, u8, u8) = (0xF3, 0xF3, 0xF3);

thread_local! {
    static SHOWING: Cell<(isize, u32)> = const { Cell::new((0, 0)) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
            let number = SHOWING.with(Cell::get).1.to_string();
            let flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
            overlay::text(dc, &number, client, (120.0 * scale) as i32, false, TEXT, flags, false);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            SHOWING.with(|s| s.set((0, 0)));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Show `number` in the middle of `screen`, replacing one that's still up.
pub fn show(number: u32, screen: Rect) {
    unsafe {
        let (open, _) = SHOWING.with(Cell::get);
        if open != 0 {
            let _ = DestroyWindow(HWND(open as *mut _));
        }
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!("Seam"),
            WS_POPUP,
            screen.left,
            screen.top,
            SIZE,
            SIZE,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            return;
        };
        SHOWING.with(|s| s.set((hwnd.0 as isize, number)));
        // Sized after creation, because the window now knows its screen's DPI.
        let side = SIZE * GetDpiForWindow(hwnd).max(96) as i32 / 96;
        let left = screen.left + (screen.width() - side) / 2;
        let top = screen.top + (screen.height() - side) / 2;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), left, top, side, side, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        SetTimer(Some(hwnd), TIMER_CLOSE, SHOWN_MS, None);
    }
}
