//! Esc stops a switch that is starting apps (D715). A low-level keyboard hook that is in place
//! only for that long, on the tray thread: all it does is set a flag. Esc is never swallowed,
//! so the app in front still gets it.

use crate::{log, switcher};
use std::cell::Cell;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
}

/// Watch for Esc until `remove`. Nothing happens if the hook is already in place or can't be made.
pub(super) fn install() {
    HOOK.with(|hook| {
        if hook.get().is_some() {
            return;
        }
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0) } {
            Ok(h) => hook.set(Some(h)),
            Err(e) => log::info!("switch: no Esc hook: {e}"),
        }
    });
}

pub(super) fn remove() {
    HOOK.with(|hook| {
        if let Some(h) = hook.take() {
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
    });
}

/// Esc on its own cancels. With Ctrl, Alt, Shift or Win it belongs to the app.
fn cancels(vk: u32, modifier_down: bool) -> bool {
    vk == u32::from(VK_ESCAPE.0) && !modifier_down
}

fn modifier_down() -> bool {
    let pressed = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0;
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].into_iter().any(pressed)
}

unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            let key = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if cancels(key.vkCode, modifier_down()) {
                switcher::cancel();
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_esc_on_its_own_cancels() {
        assert!(cancels(0x1B, false));
        assert!(!cancels(0x1B, true));
        assert!(!cancels(0x41, false));
    }
}
