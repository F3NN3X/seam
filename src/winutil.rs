//! Win32 helpers for moving other apps' windows: which ones may move, and
//! placing one so its visible frame lands on a rect.

use crate::classify::{classify, Verdict, WindowFacts};
use crate::zones::{frame_adjust, Rect};
use std::sync::OnceLock;
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
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

pub fn has_caption(h: HWND) -> bool {
    unsafe { GetWindowLongW(h, GWL_STYLE) as u32 & WS_CAPTION.0 == WS_CAPTION.0 }
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

/// Borderless and covering its whole screen, but not maximized: a video,
/// a slideshow or a browser after F11.
pub fn is_full_screen(h: HWND) -> bool {
    unsafe { !IsIconic(h).as_bool() && !IsZoomed(h).as_bool() && is_fullscreen(h, GetWindowLongW(h, GWL_STYLE) as u32) }
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
    elevated_over_us(process_id(h))
}

/// `is_elevated_over_us` by process ID, so a caller can ask once per process.
pub fn elevated_over_us(pid: u32) -> bool {
    static OWN: OnceLock<bool> = OnceLock::new();
    let refused = |e: &windows::core::Error| e.code() == ERROR_ACCESS_DENIED.to_hresult();
    unsafe {
        if *OWN.get_or_init(|| token_elevated(GetCurrentProcess()).unwrap_or(false)) {
            return false;
        }
        let process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
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

/// Pins and the focus border, kept on the window like the zone stamp, so a
/// tray that starts again finds them (§3.5).
const PIN: PCWSTR = w!("Seam_pin");
const BORDER: PCWSTR = w!("Seam_border");
/// `Seam_pin` bits: Seam made the window topmost, and workspaces leave it alone.
pub const PIN_TOP: usize = 1;
pub const PIN_STICKY: usize = 2;

pub fn pin_bits(h: HWND) -> usize {
    unsafe { GetPropW(h, PIN).0 as usize }
}

/// Replace the pin bits. None at all removes the prop.
pub fn set_pin_bits(h: HWND, bits: usize) {
    unsafe {
        if bits == 0 {
            let _ = RemovePropW(h, PIN);
        } else {
            let _ = SetPropW(h, PIN, Some(HANDLE(bits as *mut _)));
        }
    }
}

/// Workspace switches and capture leave this window alone (D106).
pub fn is_sticky(h: HWND) -> bool {
    pin_bits(h) & PIN_STICKY != 0
}

/// Seam coloured this window's border.
pub fn has_border_mark(h: HWND) -> bool {
    unsafe { !GetPropW(h, BORDER).is_invalid() }
}

/// Keep the colour Seam gave the border on the window, or None to forget it.
pub fn mark_border(h: HWND, colour: Option<u32>) {
    unsafe {
        match colour {
            Some(colour) => {
                // The prop's value is the colour plus one, so black still counts.
                let _ = SetPropW(h, BORDER, Some(HANDLE((colour as usize + 1) as *mut _)));
            }
            None => {
                let _ = RemovePropW(h, BORDER);
            }
        }
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

/// The full path of the program a process runs.
pub fn exe_path_of(pid: u32) -> Option<String> {
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
    exe_path_from(h, exe_path_of(process_id(h))?)
}

/// `exe_path`, given the path of the window's own process.
pub fn exe_path_from(h: HWND, path: String) -> Option<String> {
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

/// The Alt+Tab walk (Raymond Chen's): from the root owner, follow the last
/// active popup while it is hidden. A window counts if the walk ends on it.
fn is_alt_tab_root(h: HWND) -> bool {
    unsafe {
        let mut walk = GetAncestor(h, GA_ROOTOWNER);
        // A chain of popups is short. The bound only guards against a loop.
        for _ in 0..32 {
            let next = GetLastActivePopup(walk);
            if next == walk || IsWindowVisible(next).as_bool() {
                break;
            }
            walk = next;
        }
        walk == h
    }
}

/// The alpha a layered window set, if it set one.
fn layered_alpha(h: HWND) -> Option<u8> {
    let (mut alpha, mut flags) = (0u8, LAYERED_WINDOW_ATTRIBUTES_FLAGS(0));
    unsafe { GetLayeredWindowAttributes(h, None, Some(&mut alpha), Some(&mut flags)).ok()? };
    (flags.0 & LWA_ALPHA.0 != 0).then_some(alpha)
}

/// What the capture rules look at, without opening the window's process: the
/// caller fills in the exe's name, once per process. A hidden window stops at that.
pub fn window_facts(h: HWND, own_pid: u32) -> WindowFacts {
    unsafe {
        if h.is_invalid() || !IsWindowVisible(h).as_bool() {
            return WindowFacts::default();
        }
        let ex_style = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let minimized = IsIconic(h).as_bool();
        // A minimized window is parked off screen, so where it restores to counts.
        let rect = if minimized { normal_rect(h) } else { visible_rect(h).or_else(|| window_rect(h)) };
        let rect = rect.unwrap_or_default();
        let r = RECT { left: rect.x, top: rect.y, right: rect.x + rect.w, bottom: rect.y + rect.h };
        WindowFacts {
            style: GetWindowLongW(h, GWL_STYLE) as u32,
            ex_style,
            class: class_name(h),
            title_len: GetWindowTextLengthW(h).max(0) as usize,
            visible: true,
            cloaked: is_cloaked(h),
            minimized,
            owned: GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()),
            root_owner_is_self: is_alt_tab_root(h),
            rect,
            on_any_screen: !MonitorFromRect(&r, MONITOR_DEFAULTTONULL).is_invalid(),
            layered_alpha: if ex_style & WS_EX_LAYERED.0 != 0 { layered_alpha(h) } else { None },
            own_process: process_id(h) == own_pid,
            exe_name: String::new(),
        }
    }
}

/// The lower-case file name of an exe path.
pub fn file_name(path: &str) -> String {
    path.rsplit('\\').next().unwrap_or(path).to_lowercase()
}

/// Whether a window is a real app window, one with a taskbar button, by the
/// capture rules but without anyone's exclude list. `own_pid` keeps Seam's windows out.
pub fn is_capturable(h: HWND, own_pid: u32) -> bool {
    let mut facts = window_facts(h, own_pid);
    if classify(&facts, &|_| false) != Verdict::Capture {
        return false;
    }
    facts.exe_name = exe_name(h).unwrap_or_default();
    classify(&facts, &|_| false) == Verdict::Capture
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

/// Show the window as `show` says, with `normal` (screen pixels) as the size and
/// place it has when it isn't maximized or minimized. This call waits on the app,
/// so the caller checks `is_hung` first.
fn place(h: HWND, normal: Option<Rect>, show: SHOW_WINDOW_CMD) -> bool {
    let Some(mut wp) = placement_of(h) else { return false };
    if let Some(n) = normal {
        let (dx, dy) = workspace_offset();
        wp.rcNormalPosition = RECT { left: n.x - dx, top: n.y - dy, right: n.x + n.w - dx, bottom: n.y + n.h - dy };
    }
    if show != SW_SHOWMAXIMIZED {
        // It restores to `normal`, not to maximized.
        wp.flags &= !WPF_RESTORETOMAXIMIZED;
    }
    wp.showCmd = show.0 as u32;
    unsafe { SetWindowPlacement(h, &wp).is_ok() }
}

/// Maximize on the screen the window is on now, and set the size it gets when
/// un-maximized. This call waits on the app, so the caller checks `is_hung` first.
pub fn place_maximized(h: HWND, normal: Option<Rect>) -> bool {
    place(h, normal, SW_SHOWMAXIMIZED)
}

/// Minimize without showing or activating the window, so it restores onto
/// `frame` later. Waits on the app, like `place_maximized`.
pub fn place_minimized(h: HWND, frame: Rect, dpi: u32) -> bool {
    place(h, Some(grow(frame, frame_margin(h, dpi))), SW_SHOWMINNOACTIVE)
}

/// Restore a minimized or maximized window straight onto `frame`, without
/// activating it. Waits on the app, like `place_maximized`.
pub fn place_restored(h: HWND, frame: Rect, dpi: u32) -> bool {
    let window = match (is_minimized(h), window_rect(h), visible_rect(h)) {
        // A minimized window's frame can't be read, so its margins are Windows' usual ones.
        (false, Some(window), Some(visible)) => frame_adjust(frame, window, visible),
        _ => grow(frame, frame_margin(h, dpi)),
    };
    place(h, Some(window), SW_SHOWNOACTIVATE)
}

/// Put the window exactly on `r`, with no frame correction: a full-screen window has none.
pub fn set_rect(h: HWND, r: Rect) {
    unsafe {
        let _ = SetWindowPos(h, None, r.x, r.y, r.w, r.h, FLAGS);
    }
}

/// Raise the window to the top of the others that aren't topmost, without activating it.
pub fn raise(h: HWND) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
    unsafe {
        let _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, flags);
    }
}

/// The invisible resize border Windows gives a sizable window on the left,
/// right and bottom at `dpi`. Only for a window whose real margins can't be read.
fn frame_margin(h: HWND, dpi: u32) -> i32 {
    if !is_resizable(h) {
        return 0;
    }
    unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) }
}

fn grow(r: Rect, m: i32) -> Rect {
    Rect { x: r.x - m, y: r.y, w: r.w + 2 * m, h: r.h + m }
}

fn shrink(r: Rect, m: i32) -> Rect {
    Rect { x: r.x + m, y: r.y, w: (r.w - 2 * m).max(1), h: (r.h - m).max(1) }
}

/// Where a minimized window's visible frame will be when it restores, at `dpi`.
/// The inverse of what `place_minimized` sets, so a capture and a switch agree.
pub fn normal_frame(h: HWND, dpi: u32) -> Option<Rect> {
    normal_rect(h).map(|n| shrink(n, frame_margin(h, dpi)))
}
