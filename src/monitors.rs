//! Enumerate monitors with stable IDs, friendly names, pixel rects and physical DPI.

pub use crate::desk::Mon;
use crate::geom::Rect;
use crate::log;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, LUID, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_RAW_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::BOOL;

fn wstr(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// A display target's device path and friendly name.
fn target_info(adapter: LUID, id: u32) -> Option<(String, String)> {
    unsafe {
        let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        tgt.header.adapterId = adapter;
        tgt.header.id = id;
        if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
            return None;
        }
        Some((wstr(&tgt.monitorDevicePath), wstr(&tgt.monitorFriendlyDeviceName)))
    }
}

/// GDI device name -> (device path, friendly name), via the display-config API.
fn target_names() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return out;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return out;
        }
        paths.truncate(np as usize);
        for p in &paths {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let Some((path, name)) = target_info(p.targetInfo.adapterId, p.targetInfo.id) else { continue };
            out.push((wstr(&src.viewGdiDeviceName), path, name));
        }
    }
    out
}

/// What `QDC_ALL_PATHS` says about one display target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetFacts {
    pub path: String,
    pub name: String,
    /// A monitor is connected to it.
    pub available: bool,
}

/// The connected targets that have no active path, once each by device path: screens
/// that are plugged in but turned off in Windows (D697). `active` is the device paths
/// of the screens in use.
pub fn inactive_targets(all: &[TargetFacts], active: &[String]) -> Vec<(String, String)> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for t in all.iter().filter(|t| t.available && !t.path.is_empty()) {
        let known = |p: &str| p.eq_ignore_ascii_case(&t.path);
        if active.iter().any(|a| known(a)) || seen.iter().any(|s| known(s)) {
            continue;
        }
        seen.push(&t.path);
        out.push((t.path.clone(), t.name.clone()));
    }
    out
}

/// Screens that are connected but turned off in Windows: device path and friendly name,
/// which is empty when Windows has none. Information only.
pub fn inactive() -> Vec<(String, String)> {
    let mut all = Vec::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ALL_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ALL_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return Vec::new();
        }
        paths.truncate(np as usize);
        for p in &paths {
            let available = p.targetInfo.targetAvailable.as_bool();
            let Some((path, name)) = target_info(p.targetInfo.adapterId, p.targetInfo.id) else { continue };
            all.push(TargetFacts { path, name, available });
        }
    }
    let active: Vec<String> = target_names().into_iter().map(|(_, path, _)| path).collect();
    inactive_targets(&all, &active)
}

unsafe extern "system" fn enum_proc(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<HMONITOR>);
        list.push(hmon);
        BOOL(1)
    }
}

pub fn enumerate() -> Vec<Mon> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(&mut handles as *mut _ as isize));
    }
    let names = target_names();
    let mut mons = Vec::new();
    for (i, h) in handles.into_iter().enumerate() {
        unsafe {
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(h, &mut mi.monitorInfo as *mut MONITORINFO).as_bool() {
                continue;
            }
            let r = mi.monitorInfo.rcMonitor;
            let gdi = wstr(&mi.szDevice);
            let (id, name) = names
                .iter()
                .find(|(g, _, _)| g.eq_ignore_ascii_case(&gdi))
                .map(|(_, path, name)| (path.clone(), name.clone()))
                .unwrap_or_else(|| (gdi.clone(), String::new()));
            let (mut dx, mut dy) = (0u32, 0u32);
            let raw_dpi = if GetDpiForMonitor(h, MDT_RAW_DPI, &mut dx, &mut dy).is_ok() && dx > 0 && dy > 0 {
                (dx as f64 + dy as f64) / 2.0
            } else {
                0.0
            };
            mons.push(Mon {
                id,
                gdi,
                named: !name.is_empty(),
                name: if name.is_empty() { format!("Display {}", i + 1) } else { name },
                rect: Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
                raw_dpi,
                primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
    }
    // Stable, human-friendly numbering: left-to-right, then top-to-bottom.
    mons.sort_by_key(|m| (m.rect.left, m.rect.top));
    log::info!("enumerated {} monitor(s)", mons.len());
    for (i, m) in mons.iter().enumerate() {
        log::info!(
            "monitor {i}: id={} name={:?} rect={:?} raw_dpi={} primary={}",
            m.id,
            m.name,
            m.rect,
            m.raw_dpi,
            m.primary
        );
    }
    mons
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(path: &str, available: bool) -> TargetFacts {
        TargetFacts { path: path.into(), name: format!("name of {path}"), available }
    }

    #[test]
    fn inactive_targets_are_available_and_not_active_once_each() {
        let all = [
            target("a", true),
            target("b", true),
            // A path per source for the same target.
            target("b", true),
            target("c", false),
            target("d", true),
            target("", true),
        ];
        let got = inactive_targets(&all, &["A".to_string(), "d".to_string()]);
        assert_eq!(got, vec![("b".to_string(), "name of b".to_string())]);
        assert!(inactive_targets(&all, &[]).iter().all(|(p, _)| !p.is_empty() && p != "c"));
    }
}
