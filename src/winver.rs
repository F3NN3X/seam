//! Which Windows this is, for About and diagnostics.

use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{PCWSTR, w};

fn read(name: PCWSTR) -> String {
    let mut buf = [0u16; 64];
    let mut size = size_of_val(&buf) as u32;
    let key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let read = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    if read.is_err() {
        return String::new();
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// "24H2".
pub fn display_version() -> String {
    read(w!("DisplayVersion"))
}

pub fn build() -> String {
    read(w!("CurrentBuild"))
}
