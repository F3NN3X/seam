//! The GitHub installer's footprint: its entry in Installed apps, and whether
//! this exe is the copy it installed. The installer itself is `packaging/seam.iss`.

use crate::{autostart, log, package};
use std::path::Path;
use std::sync::OnceLock;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

/// The one place the installer's `AppId` is kept. `seam.iss` gets it from the same file.
const INSTALLER: &str = include_str!("../packaging/installer.json");

fn app_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let json: serde_json::Value = serde_json::from_str(INSTALLER).unwrap_or_default();
        json["app_id"].as_str().unwrap_or_default().to_string()
    })
}

/// Inno Setup's uninstall entry for this app.
fn key() -> HSTRING {
    HSTRING::from(format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}_is1", app_id()))
}

/// Where the installer put Seam, as the entry records it.
fn installed_dir() -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = size_of_val(&buf) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key(),
            w!("InstallLocation"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if read != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Lower-cased, resolved and without the trailing backslash Inno stores.
fn normal(path: &str) -> String {
    let path = path.trim_end_matches('\\');
    let resolved = std::fs::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.display().to_string());
    resolved.trim_end_matches('\\').to_lowercase()
}

/// True when this exe sits in the folder the installer recorded.
pub fn is_installed_copy() -> bool {
    let dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.display().to_string()));
    match (installed_dir(), dir) {
        (Some(installed), Some(dir)) => normal(&installed) == normal(&dir),
        _ => false,
    }
}

/// Which kind of copy this is, for diagnostics.
pub fn kind() -> &'static str {
    if package::is_packaged() {
        "Store"
    } else if is_installed_copy() {
        "installer"
    } else {
        "portable"
    }
}

/// Write the version the updater just installed into Installed apps, so it
/// never shows one that isn't running. Nothing to do for any other copy.
pub fn note_version(version: &str) -> Result<(), String> {
    if !is_installed_copy() {
        return Ok(());
    }
    let data: Vec<u16> = version.encode_utf16().chain(std::iter::once(0)).collect();
    let written = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &key(),
            w!("DisplayVersion"),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if written == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("Couldn't write the version to Installed apps ({})", written.0))
    }
}

/// A portable copy may have taken autostart. The installed copy takes it back,
/// but only when it is the one that started, so a portable copy never does.
pub fn take_over_autostart(exe: &Path) {
    let Some(current) = autostart::exe() else { return };
    if is_installed_copy() && normal(&current) != normal(&exe.display().to_string()) {
        autostart::set(true, exe);
        log::info!("autostart now starts the installed copy");
    }
}
