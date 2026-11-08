//! The "Shim" half: write the Windows display arrangement so each neighbour sits
//! where the middle of the physical overlap lines up in pixels too. With the
//! hook running this is a nicety; without it, it's the best Windows can do.

use crate::geom::Rect;
use crate::layout::{plan, Link};
use crate::log;
use crate::monitors::Mon;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, POINTL};

/// Apply new desktop positions. Returns Ok(false) if nothing needed to move.
pub fn align(mons: &[Mon], links: &[Link]) -> Result<bool, String> {
    let rects: Vec<Rect> = mons.iter().map(|m| m.rect).collect();
    let primary = mons.iter().position(|m| m.primary).unwrap_or(0);
    let new = plan(&rects, primary, links).map_err(|e| e.to_string())?;
    log::info!("arrangement plan: {rects:?} -> {new:?}");
    if new == rects {
        return Ok(false);
    }
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return Err("Couldn't read the display configuration".into());
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return Err("Couldn't read the display configuration".into());
        }
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        let mut moved = 0;
        for p in &paths {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let n = src.viewGdiDeviceName.iter().position(|&c| c == 0).unwrap_or(32);
            let gdi = String::from_utf16_lossy(&src.viewGdiDeviceName[..n]);
            let Some(k) = mons.iter().position(|m| m.gdi.eq_ignore_ascii_case(&gdi)) else { continue };
            let idx = p.sourceInfo.Anonymous.modeInfoIdx as usize;
            if let Some(mode) = modes.get_mut(idx) {
                if mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
                    mode.Anonymous.sourceMode.position = POINTL { x: new[k].left, y: new[k].top };
                    moved += 1;
                }
            }
        }
        if moved == 0 {
            return Err("Couldn't match screens to the display configuration".into());
        }
        let flags = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE | SDC_ALLOW_CHANGES;
        let rc = SetDisplayConfig(Some(&paths), Some(&modes), flags);
        if rc != 0 {
            return Err(format!("Windows rejected the new arrangement (error {rc})"));
        }
    }
    Ok(true)
}
