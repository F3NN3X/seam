//! Windows' "Restart apps" sign-in option, which reopens apps at sign-in as Seam's
//! last session can. Seam turns it off while it reopens the apps itself (D128).

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::*;
use windows::core::{PCWSTR, w};

const WINLOGON: PCWSTR = w!("Software\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon");
const NAME: PCWSTR = w!("RestartApps");
/// What config keeps for a `RestartApps` that wasn't there, so giving it back deletes it.
pub const MISSING: u32 = u32::MAX;

fn result(code: WIN32_ERROR) -> Result<(), WIN32_ERROR> {
    if code == ERROR_SUCCESS { Ok(()) } else { Err(code) }
}

/// The value as it is now, or `MISSING`.
pub fn read() -> Result<u32, WIN32_ERROR> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            WINLOGON,
            NAME,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    match code {
        ERROR_SUCCESS => Ok(value),
        ERROR_FILE_NOT_FOUND => Ok(MISSING),
        other => Err(other),
    }
}

/// Windows' Restart apps is on.
pub fn is_on() -> bool {
    read().is_ok_and(|v| v == 1)
}

/// Set the value, or with `MISSING` delete it.
pub fn write(value: u32) -> Result<(), WIN32_ERROR> {
    let code = unsafe {
        if value == MISSING {
            match RegDeleteKeyValueW(HKEY_CURRENT_USER, WINLOGON, NAME) {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                other => other,
            }
        } else {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                WINLOGON,
                NAME,
                REG_DWORD.0,
                Some((&value as *const u32).cast()),
                size_of::<u32>() as u32,
            )
        }
    };
    result(code)
}
