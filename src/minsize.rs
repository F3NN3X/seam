//! How small a window can go, for sizing a group of zones. The app is asked with
//! WM_GETMINMAXINFO, which can wait on it, so only the window-control worker asks,
//! never the tray's main thread. A window that came out bigger than its zone
//! counts too. Kept in memory by program and window class, at 96 dpi.

use seam::winutil;
use seam::zones::Rect;
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Win32::Foundation::*;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::*;

/// How long an app has to answer.
const ASK_MS: u32 = 100;
/// Apps remembered before the cache starts again.
const MAX_KEPT: usize = 512;

/// (exe path, window class) to the smallest visible frame at 96 dpi.
type Known = HashMap<(String, String), (i32, i32)>;

static KNOWN: Mutex<Option<Known>> = Mutex::new(None);
/// Sizes the snapping saw, waiting for the worker: window, width, height (0 when
/// that side fitted) and the DPI they were seen at.
static SEEN: Mutex<Vec<(isize, i32, i32, u32)>> = Mutex::new(Vec::new());

fn scale(v: i32, from: u32, to: u32) -> i32 {
    (v as f64 * to as f64 / from.max(1) as f64).round() as i32
}

/// The system's smallest window at 96 dpi, as a visible frame.
fn floor(margins: (i32, i32)) -> (i32, i32) {
    let (w, h) = unsafe { (GetSystemMetricsForDpi(SM_CXMINTRACK, 96), GetSystemMetricsForDpi(SM_CYMINTRACK, 96)) };
    ((w - margins.0).max(1), (h - margins.1).max(1))
}

/// The invisible resize borders, at 96 dpi.
fn margins(h: HWND) -> (i32, i32) {
    let (Some(w), Some(v)) = (winutil::window_rect(h), winutil::visible_rect(h)) else { return (0, 0) };
    let dpi = winutil::monitor_dpi(h);
    (scale(w.w - v.w, dpi, 96).max(0), scale(w.h - v.h, dpi, 96).max(0))
}

/// Ask the app. Its answer is a window rect in its own DPI; 0 means it didn't say.
fn ask(h: HWND, margins: (i32, i32)) -> Option<(i32, i32)> {
    let mut info = MINMAXINFO::default();
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            h,
            WM_GETMINMAXINFO,
            WPARAM(0),
            LPARAM(&mut info as *mut _ as isize),
            SMTO_ABORTIFHUNG,
            ASK_MS,
            Some(&mut result),
        )
    };
    let (w, ht) = (info.ptMinTrackSize.x, info.ptMinTrackSize.y);
    if sent.0 == 0 || w <= 0 || ht <= 0 {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(h) }.max(96);
    Some((scale(w, dpi, 96) - margins.0, scale(ht, dpi, 96) - margins.1))
}

fn key(h: HWND) -> Option<(String, String)> {
    Some((winutil::exe_path(h)?, winutil::class_name(h)))
}

/// Fold in what the snapping saw since last time.
fn take_seen(known: &mut Known) {
    let seen = std::mem::take(&mut *SEEN.lock().unwrap_or_else(|e| e.into_inner()));
    for (hwnd, w, h, dpi) in seen {
        let window = HWND(hwnd as *mut _);
        if !unsafe { IsWindow(Some(window)) }.as_bool() {
            continue;
        }
        let Some(key) = key(window) else { continue };
        let floor = floor(margins(window));
        let old = known.get(&key).copied().unwrap_or(floor);
        let (w, h) = (scale(w, dpi, 96), scale(h, dpi, 96));
        known.insert(key, (old.0.max(w), old.1.max(h)));
    }
}

/// The smallest visible frame of `h`, in pixels at `dpi`. Worker only: asking can
/// wait up to 100 ms on the app. A hung window gets the system's floor.
pub fn of(h: HWND, dpi: u32) -> (i32, i32) {
    let mut guard = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let known = guard.get_or_insert_with(HashMap::new);
    take_seen(known);
    let margins = margins(h);
    let floor = floor(margins);
    let at96 = if winutil::is_hung(h) {
        floor
    } else {
        match key(h) {
            Some(key) => *known.entry(key).or_insert_with(|| {
                let asked = ask(h, margins).unwrap_or(floor);
                (asked.0.max(floor.0), asked.1.max(floor.1))
            }),
            None => ask(h, margins).unwrap_or(floor),
        }
    };
    if known.len() > MAX_KEPT {
        known.clear();
    }
    (scale(at96.0, 96, dpi), scale(at96.1, 96, dpi))
}

/// A placement ended with the window bigger than `target`, so the app wouldn't go
/// smaller: keep that side as its minimum. Only queues it, for the main thread.
pub fn observed(h: HWND, got: Rect, target: Rect, slack: i32) {
    let w = if got.w > target.w + slack { got.w } else { 0 };
    let ht = if got.h > target.h + slack { got.h } else { 0 };
    if w == 0 && ht == 0 {
        return;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() < MAX_KEPT {
        seen.push((h.0 as isize, w, ht, winutil::monitor_dpi(h)));
    }
}
