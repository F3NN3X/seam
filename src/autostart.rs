//! "Launch at login" via the per-user Run key.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::*;

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("Seam");

pub fn is_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set(on: bool) -> bool {
    unsafe {
        if on {
            let mut buf = [0u16; 1024];
            let n = GetModuleFileNameW(None, &mut buf) as usize;
            if n == 0 {
                return false;
            }
            let cmd = format!("\"{}\"", String::from_utf16_lossy(&buf[..n]));
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
