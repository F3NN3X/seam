//! Enumerate monitors with stable IDs, friendly names, pixel rects and physical DPI.

use crate::geom::Rect;
use crate::log;
use windows::core::BOOL;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_RAW_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

#[derive(Clone, Debug)]
pub struct Mon {
    /// Stable monitor device path (survives reboots and port changes on the same GPU).
    pub id: String,
    /// GDI name like \\.\DISPLAY1, used to match display-config source modes.
    pub gdi: String,
    /// Friendly name from EDID, e.g. "DELL U2723QE".
    pub name: String,
    /// Desktop rect in physical pixels.
    pub rect: Rect,
    /// Physical pixels per inch from EDID (0 if unknown, e.g. some TVs).
    pub raw_dpi: f64,
    pub primary: bool,
}

fn wstr(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
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
            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = p.targetInfo.adapterId;
            tgt.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
                continue;
            }
            out.push((
                wstr(&src.viewGdiDeviceName),
                wstr(&tgt.monitorDevicePath),
                wstr(&tgt.monitorFriendlyDeviceName),
            ));
        }
    }
    out
}

unsafe extern "system" fn enum_proc(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<HMONITOR>);
    list.push(hmon);
    BOOL(1)
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
