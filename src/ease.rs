//! Windows' "Ease cursor movement between displays" setting. Without it the
//! pointer can stop at an edge before Seam ever sees it cross.
//!
//! This is an undocumented registry value. A missing one means the default, on,
//! from Windows 11 build 22557, where the setting first came. Seam only ever
//! turns it on, and only when asked to.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WIN32_ERROR, WPARAM};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const CURSORS: PCWSTR = w!("Control Panel\\Cursors");
const NAME: PCWSTR = w!("CursorDeadzoneJumpingSetting");
/// The first build with the setting.
const FIRST_BUILD: u32 = 22557;

/// On or off, or `None` on a Windows without the setting.
pub fn read() -> Option<bool> {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            CURSORS,
            NAME,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    // Only a missing value needs the build.
    let build = if code == ERROR_FILE_NOT_FOUND { crate::winver::build().parse().ok() } else { None };
    interpret(code, value, build)
}

/// What a registry read means: the value if there was one, else on from `FIRST_BUILD`.
fn interpret(code: WIN32_ERROR, value: u32, build: Option<u32>) -> Option<bool> {
    match code {
        ERROR_SUCCESS => Some(value != 0),
        ERROR_FILE_NOT_FOUND => (build? >= FIRST_BUILD).then_some(true),
        _ => None,
    }
}

/// Off is the only state that can stop the pointer, so a missing setting counts as on.
pub fn is_on() -> bool {
    read() != Some(false)
}

/// Set the value to 1. Windows only picks it up once `announce` has run.
pub fn turn_on() -> Result<(), String> {
    let value = 1u32;
    let code = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            CURSORS,
            NAME,
            REG_DWORD.0,
            Some((&value as *const u32).cast()),
            size_of::<u32>() as u32,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(format!("Windows didn't save the setting ({})", code.0));
    }
    // Something may have put it back, so read it again.
    if read() == Some(true) { Ok(()) } else { Err("Windows didn't keep Ease cursor movement on".into()) }
}

/// Tell Windows the cursor settings changed, as Settings does, so no sign-out is needed.
/// Both calls can wait on other apps' windows, so this runs off the tray's thread.
pub fn announce() {
    unsafe {
        // Reloads the cursor settings from the registry, and broadcasts WM_SETTINGCHANGE.
        let reloaded = SystemParametersInfoW(SPI_SETCURSORS, 0, None, SPIF_SENDCHANGE);
        crate::log::info!("cursor easing: SPI_SETCURSORS {reloaded:?}");
        // The same broadcast naming the key, for whatever listens for it by name.
        let mut result = 0usize;
        let sent = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(CURSORS.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            Some(&mut result),
        );
        crate::log::info!("cursor easing: WM_SETTINGCHANGE sent {}", sent.0 != 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;

    #[test]
    fn a_missing_value_is_on_only_from_the_first_build() {
        assert_eq!(interpret(ERROR_SUCCESS, 0, None), Some(false));
        assert_eq!(interpret(ERROR_SUCCESS, 1, None), Some(true));
        assert_eq!(interpret(ERROR_FILE_NOT_FOUND, 1, Some(FIRST_BUILD)), Some(true));
        assert_eq!(interpret(ERROR_FILE_NOT_FOUND, 1, Some(FIRST_BUILD - 1)), None);
        assert_eq!(interpret(ERROR_FILE_NOT_FOUND, 1, None), None);
        assert_eq!(interpret(ERROR_ACCESS_DENIED, 1, Some(FIRST_BUILD)), None);
    }
}
