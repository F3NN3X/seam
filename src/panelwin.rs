//! Win32 calls for the panel's on-screen zone editor, which gpui has no options
//! for: that its window sits just behind the Seam window, has no taskbar button,
//! and (for the live view) lets the mouse through. Both windows belong to the
//! panel process, and are found by their exact titles.

use crate::winutil::process_id;
use crate::zones::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The work area (taskbar excluded) of the screen at `bounds`, in physical pixels.
pub fn work_area(bounds: Rect) -> Option<Rect> {
    let r = RECT { left: bounds.x, top: bounds.y, right: bounds.x + bounds.w, bottom: bounds.y + bounds.h };
    unsafe {
        let monitor = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let w = info.rcWork;
        Some(Rect { x: w.left, y: w.top, w: w.right - w.left, h: w.bottom - w.top })
    }
}

/// The screen under the mouse pointer: its handle (gpui's display id), work area
/// in physical pixels, and effective DPI. None if Windows won't say.
pub fn screen_under_pointer() -> Option<(u64, Rect, u32)> {
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (0u32, 0u32);
        let dpi = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) {
            Ok(()) if dx > 0 => dx,
            _ => 96,
        };
        let w = info.rcWork;
        Some((monitor.0 as u64, Rect { x: w.left, y: w.top, w: w.right - w.left, h: w.bottom - w.top }, dpi))
    }
}

struct Search {
    pid: u32,
    title: Vec<u16>,
    found: Option<HWND>,
}

unsafe extern "system" fn matches(h: HWND, data: LPARAM) -> BOOL {
    let search = &mut *(data.0 as *mut Search);
    let mut buf = [0u16; 64];
    let len = GetWindowTextW(h, &mut buf).max(0) as usize;
    if buf[..len] == search.title[..] && process_id(h) == search.pid {
        search.found = Some(h);
        return BOOL(0);
    }
    BOOL(1)
}

/// This process's top-level window with exactly this title.
fn find(title: &str) -> Option<HWND> {
    let mut search = Search { pid: std::process::id(), title: title.encode_utf16().collect(), found: None };
    unsafe {
        let _ = EnumWindows(Some(matches), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

const STACK: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0);

/// Put the editor window on `area` (physical pixels), straight behind the panel
/// window, with no taskbar button. `passthrough` also makes it layered, click-through
/// and unable to take focus. Returns false if either window can't be found.
pub fn place(editor: &str, panel: &str, area: Rect, passthrough: bool) -> bool {
    let (Some(editor), Some(panel)) = (find(editor), find(panel)) else { return false };
    unsafe {
        let now = GetWindowLongPtrW(editor, GWL_EXSTYLE) as u32;
        let mut next = (now | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
        if passthrough {
            next |= WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0;
        }
        if next != now {
            // The taskbar only looks again when the window is shown again.
            let _ = ShowWindow(editor, SW_HIDE);
            SetWindowLongPtrW(editor, GWL_EXSTYLE, next as isize);
            if passthrough {
                let _ = SetLayeredWindowAttributes(editor, COLORREF(0), 255, LWA_ALPHA);
            }
            let _ = ShowWindow(editor, SW_SHOWNA);
        }
        // Moving first settles any change of scaling, which would otherwise
        // replace the size set next.
        let _ = SetWindowPos(
            editor,
            Some(panel),
            area.x,
            area.y,
            0,
            0,
            SET_WINDOW_POS_FLAGS(SWP_NOSIZE.0 | SWP_NOACTIVATE.0),
        );
        let _ = SetWindowPos(editor, Some(panel), area.x, area.y, area.w, area.h, SWP_NOACTIVATE);
    }
    true
}

/// Keep the editor window straight behind the panel window. `raise_panel` first
/// lifts the panel, for when a click on the editor has just brought it forward.
pub fn stack(editor: &str, panel: &str, raise_panel: bool) {
    let (Some(editor), Some(panel)) = (find(editor), find(panel)) else { return };
    unsafe {
        if raise_panel {
            let _ = SetWindowPos(panel, Some(HWND_TOP), 0, 0, 0, 0, STACK);
        }
        let _ = SetWindowPos(editor, Some(panel), 0, 0, 0, 0, STACK);
    }
}
