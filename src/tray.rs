//! How the settings panel and the tray app find each other.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: PCWSTR = w!("SeamTray");
pub const WM_APP_LINEUP: u32 = WM_APP + 3;
pub const WM_APP_RELOAD: u32 = WM_APP + 4;
/// Wakes the main thread to drain the API command queue.
pub const WM_APP_API: u32 = WM_APP + 5;
pub const PANEL_TITLE: &str = "Seam";

/// One panel at a time. Returns true after raising a panel that's already open.
pub fn raise_open_panel() -> bool {
    unsafe {
        // The mutex stays held until this process exits.
        let _ = CreateMutexW(None, true, w!("Local\\Seam.Settings"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(panel) = FindWindowW(None, w!("Seam")) {
            let _ = ShowWindow(panel, SW_RESTORE);
            let _ = SetForegroundWindow(panel);
        }
        true
    }
}
