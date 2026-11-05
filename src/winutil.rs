//! Win32 helpers for moving other apps' windows: which ones may move, and
//! placing one so its visible frame lands on a rect.

use crate::zones::{frame_adjust, Rect};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::*;

fn rect_of(r: RECT) -> Rect {
    Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
}

pub fn is_cloaked(h: HWND) -> bool {
    let mut v: u32 = 0;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut _ as _, 4).is_ok() && v != 0 }
}

pub fn window_rect(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r).ok()? };
    Some(rect_of(r))
}

/// The frame the user sees, without the invisible resize borders.
pub fn visible_rect(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    let size = std::mem::size_of::<RECT>() as u32;
    unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as _, size).ok()? };
    Some(rect_of(r))
}

pub fn process_id(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// The exe's file name in lower case, for the exclude list.
pub fn exe_name(h: HWND) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id(h)).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        let path = String::from_utf16_lossy(buf.get(..len as usize)?);
        ok.then(|| path.rsplit('\\').next().unwrap_or(&path).to_lowercase())
    }
}

fn is_fullscreen(h: HWND, style: u32) -> bool {
    let caption = WS_CAPTION.0;
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let monitor = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let (Some(window), true) = (window_rect(h), GetMonitorInfoW(monitor, &mut mi).as_bool()) else {
            return false;
        };
        style & caption != caption && window == rect_of(mi.rcMonitor)
    }
}

/// Whether a window may be snapped. `own_pid` keeps Seam's own windows out and
/// `exclude` holds lower-case exe names.
pub fn is_snappable(h: HWND, own_pid: u32, exclude: &[String]) -> bool {
    unsafe {
        if h.is_invalid() || !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() || is_cloaked(h) {
            return false;
        }
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if style & WS_CHILD.0 != 0 || ex & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        if style & (WS_THICKFRAME.0 | WS_CAPTION.0) == 0 {
            return false;
        }
        if GetWindow(h, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        if process_id(h) == own_pid || is_fullscreen(h, style) {
            return false;
        }
        exclude.is_empty() || exe_name(h).is_none_or(|name| !exclude.contains(&name))
    }
}

pub fn is_maximized(h: HWND) -> bool {
    unsafe { IsZoomed(h).as_bool() }
}

const FLAGS: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_ASYNCWINDOWPOS.0);

/// Ask the window to restore from maximized. Returns at once.
pub fn restore(h: HWND) {
    unsafe {
        let _ = ShowWindowAsync(h, SW_RESTORE);
    }
}

/// Resize and move so the visible frame lands on `target`. The margins are
/// read now, so call it again after a DPI change. Returns at once.
pub fn fit(h: HWND, target: Rect) {
    let Some(window) = window_rect(h) else { return };
    let visible = visible_rect(h).unwrap_or(window);
    let r = frame_adjust(target, window, visible);
    unsafe {
        let _ = SetWindowPos(h, None, r.x, r.y, r.w, r.h, FLAGS);
    }
}

/// Move the window's top-left so its visible frame starts at `target`, keeping
/// its size. This is the first step across a DPI change.
pub fn move_to(h: HWND, target: Rect) {
    let Some(window) = window_rect(h) else { return };
    let visible = visible_rect(h).unwrap_or(window);
    let (x, y) = (target.x - (visible.x - window.x), target.y - (visible.y - window.y));
    unsafe {
        let _ = SetWindowPos(h, None, x, y, 0, 0, FLAGS | SWP_NOSIZE);
    }
}
