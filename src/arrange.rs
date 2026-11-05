//! The "Shim" half: write the Windows display arrangement so each neighbour sits
//! where the middle of the physical overlap lines up in pixels too. With the
//! hook running this is a nicety; without it, it's the best Windows can do.

use crate::geom::{aligned_along_origin, place_against, touching, Rect};
use crate::hook::Link;
use crate::monitors::Mon;
use std::collections::VecDeque;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, POINTL};

/// New rects for every monitor, moving each neighbour only along its shared
/// edge, starting from the primary (which Windows pins at 0,0).
pub fn plan(rects: &[Rect], primary: usize, links: &[Link]) -> Result<Vec<Rect>, String> {
    if rects.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = rects.to_vec();
    let mut placed = vec![false; rects.len()];
    let mut queue = VecDeque::from([primary]);
    placed[primary] = true;
    while let Some(m) = queue.pop_front() {
        for l in links {
            let (n, side, map) = if l.a == m && !placed[l.b] {
                (l.b, l.side, l.map)
            } else if l.b == m && !placed[l.a] {
                (l.a, l.side.opposite(), l.map.inverse())
            } else {
                continue;
            };
            if let Some(origin) = aligned_along_origin(&out[m], &out[n], side, &map) {
                out[n] = place_against(&out[m], &out[n], side, origin);
            }
            placed[n] = true;
            queue.push_back(n);
        }
    }
    // Every pair that touched before must still touch, and nothing may overlap.
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            if out[i].intersects(&out[j]) {
                return Err("aligning would make screens overlap".into());
            }
            if touching(&rects[i], &rects[j]).is_some() && touching(&out[i], &out[j]).is_none() {
                return Err("this layout is too complex to align automatically".into());
            }
        }
    }
    Ok(out)
}

/// Apply new desktop positions. Returns Ok(false) if nothing needed to move.
pub fn align(mons: &[Mon], links: &[Link]) -> Result<bool, String> {
    let rects: Vec<Rect> = mons.iter().map(|m| m.rect).collect();
    let primary = mons.iter().position(|m| m.primary).unwrap_or(0);
    let new = plan(&rects, primary, links)?;
    if new == rects {
        return Ok(false);
    }
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return Err("couldn't read the display configuration".into());
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return Err("couldn't read the display configuration".into());
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
            return Err("couldn't match screens to the display configuration".into());
        }
        let flags = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE | SDC_ALLOW_CHANGES;
        let rc = SetDisplayConfig(Some(&paths), Some(&modes), flags);
        if rc != 0 {
            return Err(format!("Windows rejected the new arrangement (error {rc})"));
        }
    }
    Ok(true)
}
