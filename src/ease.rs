//! Windows' "Ease cursor movement between displays" setting. Without it the
//! pointer can stop at an edge before Seam ever sees it cross.
//!
//! This is an undocumented registry value. A missing one means the default, on.

use windows::core::w;
use windows::Win32::System::Registry::*;

pub fn is_on() -> bool {
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Cursors"),
            w!("CursorDeadzoneJumpingSetting"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
    }
    value != 0
}
