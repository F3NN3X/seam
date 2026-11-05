//! "Launch at login" via the per-user Run key. The Store build uses its
//! StartupTask instead (`package.rs`).

use std::path::Path;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("Seam");
/// Tells a login launch from one the user started, so only the second opens the window.
pub const FLAG: &str = "--autostart";

pub fn is_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

/// The command the Run key holds, if any.
fn command() -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = size_of_val(&buf) as u32;
    let read = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    if read != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// The exe the Run key starts.
pub fn exe() -> Option<String> {
    let command = command()?;
    let command = command.trim_start_matches('"');
    Some(command.split('"').next().unwrap_or_default().to_string())
}

/// `exe` is the tray app, wherever this is called from.
pub fn set(on: bool, exe: &Path) -> bool {
    unsafe {
        if on {
            let cmd = format!("\"{}\" {FLAG}", exe.display());
            let data: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                NAME,
                REG_SZ.0,
                Some(data.as_ptr() as *const _),
                (data.len() * 2) as u32,
            ) == ERROR_SUCCESS
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) == ERROR_SUCCESS
        }
    }
}

/// A Run key from before `--autostart` existed would open the window at every
/// login, so add the flag to a key that points at this exe.
pub fn add_flag(exe: &Path) {
    let current = command().unwrap_or_default();
    if current == format!("\"{}\"", exe.display()) {
        set(true, exe);
    }
}
