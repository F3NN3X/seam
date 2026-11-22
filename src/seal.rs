//! Startup commands are stored sealed for this PC and this account (D736), with
//! `CryptProtectData`. A value sealed anywhere else, by anyone else, or typed into
//! config.json by hand doesn't open here, so it never runs.

use crate::workspace::{base64, unbase64};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::w;

/// Mixed into every seal, so a value sealed by another program for this account doesn't open as a command.
const ENTROPY: &[u8] = b"Seam startup command v1";

fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 }
}

/// What Windows returned, copied out and freed.
fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(out.pbData as *mut _))) };
    bytes
}

fn seal_with(text: &str, entropy: &[u8]) -> Option<String> {
    let (data, entropy) = (blob(text.as_bytes()), blob(entropy));
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    unsafe {
        CryptProtectData(
            &data,
            w!("Seam startup command"),
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    }
    .ok()?;
    Some(base64(&take(out)))
}

fn unseal_with(sealed: &str, entropy: &[u8]) -> Option<String> {
    let bytes = unbase64(sealed)?;
    let (data, entropy) = (blob(&bytes), blob(entropy));
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    unsafe { CryptUnprotectData(&data, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    String::from_utf8(take(out)).ok()
}

/// The text sealed for this user, as base64, or `None` when Windows couldn't.
pub fn seal(text: &str) -> Option<String> {
    seal_with(text, ENTROPY)
}

/// The text a value `seal` made holds, or `None` for anything else: another PC, another
/// account, other entropy, or no seal at all.
pub fn unseal(sealed: &str) -> Option<String> {
    unseal_with(sealed, ENTROPY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_round_trips_and_foreign_values_dont_unseal() {
        let command = "npm run dev -- --port \"3000\"; echo é";
        let sealed = seal(command).unwrap();
        assert!(!sealed.contains("npm"));
        assert_eq!(unseal(&sealed).as_deref(), Some(command));
        // Sealed with other entropy, as another program would, it doesn't open as a command.
        let foreign = seal_with(command, b"something else").unwrap();
        assert_eq!(unseal(&foreign), None);
        // Nor does text that was never sealed, in clear or as base64.
        assert_eq!(unseal(command), None);
        assert_eq!(unseal(&base64(command.as_bytes())), None);
        assert_eq!(unseal(""), None);
    }
}
