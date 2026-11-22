//! Esc stops a switch that is starting apps (D715). A low-level keyboard hook that is in place
//! only for that long, on a thread of its own: all it does is set a flag. Esc is never swallowed,
//! so the app in front still gets it.

use crate::{log, switcher};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::channel;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The hook thread's id while one runs, else 0.
static THREAD: AtomicU32 = AtomicU32::new(0);

/// Watch for Esc until `remove`, with a fresh hook on a thread of its own, so one Windows dropped
/// for being slow is replaced at every start phase. The hook never waits on the tray thread.
pub(super) fn install() {
    remove();
    let (ready, wait) = channel();
    let spawned = std::thread::Builder::new().name("seam-esc".into()).spawn(move || unsafe {
        let mut msg = MSG::default();
        // Creates this thread's queue, so a stop from now on reaches it.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0);
        match &hook {
            Ok(_) => {
                let _ = ready.send(GetCurrentThreadId());
            }
            Err(e) => {
                log::info!("switch: no Esc hook: {e}");
                let _ = ready.send(0);
            }
        }
        if let Ok(hook) = hook {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
            let _ = UnhookWindowsHookEx(hook);
        }
    });
    if spawned.is_ok() {
        THREAD.store(wait.recv().unwrap_or(0), Ordering::SeqCst);
    }
}

pub(super) fn remove() {
    let id = THREAD.swap(0, Ordering::SeqCst);
    if id != 0 {
        let _ = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
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
