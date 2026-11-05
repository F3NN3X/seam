//! Win32 helpers for moving other apps' windows: which ones may move, and
//! placing one so its visible frame lands on a rect.

use crate::zones::{frame_adjust, Rect};
use std::sync::OnceLock;
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
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
        let (sizable, caption) = (style & WS_THICKFRAME.0 != 0, style & WS_CAPTION.0 == WS_CAPTION.0);
        let boxes = style & (WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0) != 0;
        // A popup is only a real window if it can be resized and has a caption or a min or max box.
        let framed = if style & WS_POPUP.0 != 0 { sizable && (caption || boxes) } else { sizable || caption };
        if !framed {
            return false;
        }
        // An owner that is on screen makes this a dialog. A hidden or empty one, like a
        // browser's helper window, doesn't.
        let owned = GetWindow(h, GW_OWNER).is_ok_and(|o| {
            !o.is_invalid() && IsWindowVisible(o).as_bool() && window_rect(o).is_some_and(|r| r.w > 0 && r.h > 0)
        });
        if owned {
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

/// A window with no sizing border keeps its size.
pub fn is_resizable(h: HWND) -> bool {
    unsafe { GetWindowLongW(h, GWL_STYLE) as u32 & WS_THICKFRAME.0 != 0 }
}

/// The effective DPI of the screen the window is on, which is what a window that
/// ignores DPI is scaled by too.
pub fn monitor_dpi(h: HWND) -> u32 {
    let (mut dx, mut dy) = (0u32, 0u32);
    unsafe {
        let monitor = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 {
            dx
        } else {
            96
        }
    }
}

/// Whether the process is elevated.
fn token_elevated(process: HANDLE) -> Result<bool, windows::core::Error> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        let read = GetTokenInformation(token, TokenElevation, Some(&mut elevation as *mut _ as _), size, &mut len);
        let _ = CloseHandle(token);
        read.map(|()| elevation.TokenIsElevated != 0)
    }
}

/// Whether the window's process is elevated and Seam isn't, so Windows won't let
/// Seam move it. Being refused the process or its token counts as elevated.
pub fn is_elevated_over_us(h: HWND) -> bool {
    static OWN: OnceLock<bool> = OnceLock::new();
    let refused = |e: &windows::core::Error| e.code() == ERROR_ACCESS_DENIED.to_hresult();
    unsafe {
        if *OWN.get_or_init(|| token_elevated(GetCurrentProcess()).unwrap_or(false)) {
            return false;
        }
        let process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id(h)) {
            Ok(process) => process,
            Err(e) => return refused(&e),
        };
        let elevated = token_elevated(process).unwrap_or_else(|e| refused(&e));
        let _ = CloseHandle(process);
        elevated
    }
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
    // A window with no sizing border keeps its size and only goes to the zone's corner.
    if !is_resizable(h) {
        return move_to(h, target);
    }
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

/// Which zones a window is in, kept on the window itself (as FancyZones does) so a
/// restart of the tray still knows. Bit i is zone i, in two 64-bit halves.
const STAMP_LOW: PCWSTR = w!("Seam_zones");
const STAMP_HIGH: PCWSTR = w!("Seam_zones_max128");

fn stamp_part(h: HWND, name: PCWSTR) -> u64 {
    unsafe { GetPropW(h, name).0 as usize as u64 }
}

pub fn has_zone_stamp(h: HWND) -> bool {
    stamp_part(h, STAMP_LOW) != 0 || stamp_part(h, STAMP_HIGH) != 0
}

pub fn zone_stamp(h: HWND) -> Vec<usize> {
    let (low, high) = (stamp_part(h, STAMP_LOW), stamp_part(h, STAMP_HIGH));
    (0..64).filter(|i| low >> i & 1 == 1).chain((0..64).filter(|i| high >> i & 1 == 1).map(|i| i + 64)).collect()
}

/// Replace the stamp. Zone numbers past 127 can't be kept, so such a window is left unstamped.
pub fn stamp_zones(h: HWND, zones: &[usize]) {
    clear_zone_stamp(h);
    let (mut low, mut high) = (0u64, 0u64);
    for &zone in zones {
        match zone {
            0..=63 => low |= 1 << zone,
            64..=127 => high |= 1 << (zone - 64),
            _ => return,
        }
    }
    unsafe {
        for (name, bits) in [(STAMP_LOW, low), (STAMP_HIGH, high)] {
            if bits != 0 {
                let _ = SetPropW(h, name, Some(HANDLE(bits as usize as *mut _)));
            }
        }
    }
}

pub fn clear_zone_stamp(h: HWND) {
    unsafe {
        let _ = RemovePropW(h, STAMP_LOW);
        let _ = RemovePropW(h, STAMP_HIGH);
    }
}

/// The work area (taskbar excluded) and effective DPI of the screen at `bounds`.
pub fn work_area(bounds: Rect) -> Option<(Rect, u32)> {
    let r = RECT { left: bounds.x, top: bounds.y, right: bounds.x + bounds.w, bottom: bounds.y + bounds.h };
    unsafe {
        let monitor = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (0u32, 0u32);
        let dpi =
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 { dx } else { 96 };
        Some((rect_of(info.rcWork), dpi))
    }
}

// ---- Workspaces: finding, describing and placing other apps' windows.

unsafe extern "system" fn collect_window(h: HWND, data: LPARAM) -> BOOL {
    (*(data.0 as *mut Vec<HWND>)).push(h);
    BOOL(1)
}

/// Every top-level window, topmost first.
pub fn top_level_windows() -> Vec<HWND> {
    let mut list: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(buf.get(..len.max(0) as usize).unwrap_or_default())
}

pub fn title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(buf.get(..len.max(0) as usize).unwrap_or_default())
}

fn exe_path_of(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        let path = String::from_utf16_lossy(buf.get(..len as usize)?);
        ok.then_some(path)
    }
}

unsafe extern "system" fn find_core_window(h: HWND, data: LPARAM) -> BOOL {
    if class_name(h) == "Windows.UI.Core.CoreWindow" {
        *(data.0 as *mut Option<HWND>) = Some(h);
        return BOOL(0);
    }
    BOOL(1)
}

/// The full path of the program that owns the window. A packaged app's frame is
/// drawn by ApplicationFrameHost, so its hosted `CoreWindow` says whose it is.
pub fn exe_path(h: HWND) -> Option<String> {
    let path = exe_path_of(process_id(h))?;
    if !path.to_lowercase().ends_with(r"\applicationframehost.exe") {
        return Some(path);
    }
    let mut core: Option<HWND> = None;
    unsafe {
        let _ = EnumChildWindows(Some(h), Some(find_core_window), LPARAM(&mut core as *mut _ as isize));
    }
    core.and_then(|c| exe_path_of(process_id(c))).or(Some(path))
}

/// The window's AppUserModelID, if it has one.
pub fn aumid(h: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(h).ok()?;
        let value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let text = PropVariantToStringAlloc(&value).ok()?;
        let id = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const _));
        id.filter(|id| !id.is_empty())
    }
}

const IGNORED_CLASSES: [&str; 5] =
    ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"];

/// Whether a window is one the user would call an app window: visible, not
/// cloaked, titled, not a tool window, and not the shell's own. `own_pid` keeps
/// Seam's windows out.
pub fn is_capturable(h: HWND, own_pid: u32) -> bool {
    unsafe {
        if h.is_invalid() || !IsWindowVisible(h).as_bool() || is_cloaked(h) || GetWindowTextLengthW(h) == 0 {
            return false;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_APPWINDOW.0 == 0 {
            return false;
        }
        process_id(h) != own_pid && !IGNORED_CLASSES.contains(&class_name(h).as_str())
    }
}

pub fn is_minimized(h: HWND) -> bool {
    unsafe { IsIconic(h).as_bool() }
}

/// The app has stopped taking messages. Calls that wait on it must be skipped.
pub fn is_hung(h: HWND) -> bool {
    unsafe { IsHungAppWindow(h).as_bool() }
}

/// `rcNormalPosition` is in workspace coordinates, which start at the primary
/// screen's work area rather than at the screen's corner.
fn workspace_offset() -> (i32, i32) {
    unsafe {
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return (0, 0);
        }
        (info.rcWork.left - info.rcMonitor.left, info.rcWork.top - info.rcMonitor.top)
    }
}

fn placement_of(h: HWND) -> Option<WINDOWPLACEMENT> {
    let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    unsafe { GetWindowPlacement(h, &mut wp).ok()? };
    Some(wp)
}

/// The size and place the window has when it isn't maximized or minimized, in screen pixels.
pub fn normal_rect(h: HWND) -> Option<Rect> {
    let (dx, dy) = workspace_offset();
    let r = placement_of(h)?.rcNormalPosition;
    Some(rect_of(RECT { left: r.left + dx, top: r.top + dy, right: r.right + dx, bottom: r.bottom + dy }))
}

/// Maximize on the screen the window is on now, and set the size it gets when
/// un-maximized. This call waits on the app, so the caller checks `is_hung` first.
pub fn place_maximized(h: HWND, normal: Option<Rect>) -> bool {
    let Some(mut wp) = placement_of(h) else { return false };
    if let Some(n) = normal {
        let (dx, dy) = workspace_offset();
        wp.rcNormalPosition = RECT { left: n.x - dx, top: n.y - dy, right: n.x + n.w - dx, bottom: n.y + n.h - dy };
    }
    wp.showCmd = SW_SHOWMAXIMIZED.0 as u32;
    unsafe { SetWindowPlacement(h, &wp).is_ok() }
}
