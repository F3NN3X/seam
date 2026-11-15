//! Which windows the overview lists, and on which screen and virtual desktop each is.
//! The rules are the capture's (`classify`), so only real app windows with a taskbar
//! button count, never Seam's own. The shell cloaks the windows of the other virtual
//! desktops, so a cloaked window counts only when the shell cloaked it and the desktop
//! manager places it on another desktop; one cloaked otherwise is truly hidden.

use super::*;
use seam::classify::{Verdict, classify};
use std::collections::HashMap;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
use windows::core::{GUID, HSTRING};

/// Where Explorer keeps the desktops' order and names. Read for the labels only, and
/// a plain label stands in when it can't be read (D573).
const DESKTOPS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";
/// How long the app may take to answer for its icon.
const ICON_WAIT_MS: u32 = 50;

/// How DWM cloaked the window, 0 for not at all.
fn cloak(h: HWND) -> u32 {
    let mut v: u32 = 0;
    let read = unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut _ as _, 4) };
    if read.is_ok() { v } else { 0 }
}

/// The GUID in the registry's form, with braces.
fn braced(g: &GUID) -> String {
    let d = g.data4;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

/// Explorer's list of desktops, in order, or empty when it can't be read.
fn desktop_order() -> Vec<GUID> {
    let (key, value) = (HSTRING::from(DESKTOPS_KEY), HSTRING::from("VirtualDesktopIDs"));
    let mut bytes = vec![0u8; 16 * 64];
    let mut len = bytes.len() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &value,
            RRF_RT_REG_BINARY,
            None,
            Some(bytes.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if read.is_err() {
        return Vec::new();
    }
    bytes.truncate(len as usize);
    let guid = |c: &[u8; 16]| GUID {
        data1: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
        data2: u16::from_le_bytes([c[4], c[5]]),
        data3: u16::from_le_bytes([c[6], c[7]]),
        data4: [c[8], c[9], c[10], c[11], c[12], c[13], c[14], c[15]],
    };
    bytes.as_chunks::<16>().0.iter().map(guid).collect()
}

/// The name I gave a desktop, if I renamed it.
fn desktop_name(id: &GUID) -> Option<String> {
    let key = HSTRING::from(format!(r"{DESKTOPS_KEY}\Desktops\{}", braced(id)));
    let value = HSTRING::from("Name");
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if read.is_err() {
        return None;
    }
    let chars = (len as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').to_string())
}

/// The app's name before its description is read: the exe's file name without `.exe`.
fn stem(exe: &str) -> String {
    let name = exe.rsplit('\\').next().unwrap_or(exe);
    let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name);
    let mut c = name.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The screen a frame is on: the one holding its middle, else the one it overlaps most.
fn screen_of(screens: &[Rect], r: Rect) -> usize {
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    if let Some(i) = screens.iter().position(|s| s.contains(x, y)) {
        return i;
    }
    let overlap = |s: &Rect| {
        let w = (s.x + s.w).min(r.x + r.w) - s.x.max(r.x);
        let h = (s.y + s.h).min(r.y + r.h) - s.y.max(r.y);
        i64::from(w.max(0)) * i64::from(h.max(0))
    };
    (0..screens.len()).max_by_key(|&i| overlap(&screens[i])).unwrap_or(0)
}

/// The windows to show, topmost first, and each desktop's label, the one in front first.
/// Only cheap facts are read here; `details` reads the rest after the first paint.
pub(super) fn windows(screens: &[Rect]) -> (Vec<Win>, Vec<String>) {
    let own = std::process::id();
    let manager: Option<IVirtualDesktopManager> =
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }.ok();
    // Desktop ids in the order windows are met, the one in front as None.
    let mut met: Vec<Option<GUID>> = vec![None];
    let mut wins = Vec::new();
    let mut paths: HashMap<u32, Option<String>> = HashMap::new();
    for h in winutil::top_level_windows() {
        let mut facts = winutil::window_facts(h, own);
        if !facts.visible {
            continue;
        }
        let mut desktop = None;
        if facts.cloaked {
            // Only the shell's cloak of a window on another desktop counts.
            let elsewhere = cloak(h) == DWM_CLOAKED_SHELL
                && manager
                    .as_ref()
                    .is_some_and(|m| unsafe { m.IsWindowOnCurrentVirtualDesktop(h) }.is_ok_and(|on| !on.as_bool()));
            let id = elsewhere
                .then(|| manager.as_ref().and_then(|m| unsafe { m.GetWindowDesktopId(h) }.ok()))
                .flatten()
                .filter(|id| *id != GUID::zeroed());
            let Some(id) = id else { continue };
            facts.cloaked = false;
            desktop = Some(id);
        }
        if classify(&facts, &|_| false) != Verdict::Capture {
            continue;
        }
        let pid = winutil::process_id(h);
        let path = paths.entry(pid).or_insert_with(|| winutil::exe_path_of(pid)).clone();
        let Some(path) = path.and_then(|p| winutil::exe_path_from(h, p)) else { continue };
        facts.exe_name = winutil::file_name(&path);
        if classify(&facts, &|_| false) != Verdict::Capture {
            continue;
        }
        let index = match met.iter().position(|d| *d == desktop) {
            Some(i) => i,
            None => {
                met.push(desktop);
                met.len() - 1
            }
        };
        let icon = unsafe { GetClassLongPtrW(h, GCLP_HICON) };
        wins.push(Win {
            hwnd: h,
            title: winutil::title(h),
            app: stem(&path),
            exe: path,
            screen: screen_of(screens, facts.rect),
            desktop: index,
            minimized: facts.minimized,
            frame: facts.rect,
            icon: (icon != 0).then_some(HICON(icon as *mut _)),
            thumb: 0,
            source: (0, 0),
        });
    }
    // The desktops in Explorer's order, the one in front first.
    let order = desktop_order();
    let place = |d: &Option<GUID>| d.and_then(|id| order.iter().position(|o| *o == id));
    let mut ranked: Vec<usize> = (0..met.len()).collect();
    ranked.sort_by_key(|&i| (i != 0, place(&met[i]).unwrap_or(usize::MAX), i));
    let new_index: Vec<usize> = (0..met.len()).map(|i| ranked.iter().position(|&r| r == i).unwrap_or(i)).collect();
    for w in &mut wins {
        w.desktop = new_index[w.desktop];
    }
    let labels = ranked
        .iter()
        .map(|&i| {
            let name = met[i].as_ref().and_then(desktop_name);
            overview::desktop_label(i == 0, name.as_deref(), place(&met[i]))
        })
        .collect();
    (wins, labels)
}

/// The app's icon by asking the window, which may wait on the app a moment.
fn asked_icon(h: HWND) -> Option<HICON> {
    let ask = |kind: u32| {
        let mut out = 0usize;
        let flags = SMTO_ABORTIFHUNG | SMTO_BLOCK;
        let sent = unsafe {
            SendMessageTimeoutW(h, WM_GETICON, WPARAM(kind as usize), LPARAM(0), flags, ICON_WAIT_MS, Some(&mut out))
        };
        (sent.0 != 0 && out != 0).then_some(HICON(out as *mut _))
    };
    if winutil::is_hung(h) {
        return None;
    }
    ask(ICON_BIG).or_else(|| ask(ICON_SMALL2))
}

/// The slower facts, read after the first paint: the app's own name and its icon.
pub(super) fn details(wins: &mut [Win]) {
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    for w in wins.iter_mut() {
        // A Store app's frame belongs to ApplicationFrameHost, whose description isn't the app's.
        let framed = winutil::class_name(w.hwnd) == "ApplicationFrameWindow";
        let read = || winutil::app_name(w.hwnd).filter(|_| !framed);
        let name = names.entry(w.exe.clone()).or_insert_with(read).clone();
        if let Some(name) = name {
            w.app = name;
        }
        if let Some(icon) = asked_icon(w.hwnd) {
            w.icon = Some(icon);
        }
    }
}

/// Start a COM apartment for the desktop manager. False when one of another kind was there.
pub(super) fn com_start() -> bool {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok()
}

pub(super) fn com_end(started: bool) {
    if started {
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_id_reads_as_the_registry_writes_it() {
        let g = GUID::from_u128(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        assert_eq!(braced(&g), "{01234567-89AB-CDEF-0011-223344556677}");
    }

    #[test]
    fn an_app_is_named_by_its_exe_until_its_description_is_read() {
        assert_eq!(stem(r"C:\Program Files\Mozilla Firefox\firefox.exe"), "Firefox");
        assert_eq!(stem("CODE.EXE"), "CODE");
    }

    #[test]
    fn a_window_is_on_the_screen_holding_its_middle_or_overlapping_it_most() {
        let screens = [Rect { x: 0, y: 0, w: 100, h: 100 }, Rect { x: 100, y: 0, w: 100, h: 100 }];
        assert_eq!(screen_of(&screens, Rect { x: 120, y: 10, w: 40, h: 40 }), 1);
        assert_eq!(screen_of(&screens, Rect { x: 40, y: -300, w: 100, h: 320 }), 0);
    }
}
