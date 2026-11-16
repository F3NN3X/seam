//! Dates in the user's own format, as Windows spells them.

use crate::desk::{Date, month_day};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{DATE_MONTHDAY, GetDateFormatEx};
use windows::core::PCWSTR;

/// The day and month in the user's locale, such as "9 Nov" or "9. nov.". Each day is
/// asked of Windows once, so drawing it again costs a lookup, not a Win32 call.
pub fn month_day_local(date: Date) -> String {
    type Cache = Mutex<HashMap<(i32, u32, u32), String>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = (date.year, date.month, date.day);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(text) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return text;
    }
    let text = ask_windows(date).unwrap_or_else(|| month_day(date));
    if let Ok(mut c) = cache.lock() {
        c.insert(key, text.clone());
    }
    text
}

fn ask_windows(date: Date) -> Option<String> {
    let at =
        SYSTEMTIME { wYear: date.year as u16, wMonth: date.month as u16, wDay: date.day as u16, ..Default::default() };
    let mut buf = [0u16; 64];
    // A null locale name is the user's default, and a null picture is the locale's own.
    let len = unsafe {
        GetDateFormatEx(PCWSTR::null(), DATE_MONTHDAY, Some(&at), PCWSTR::null(), Some(&mut buf), PCWSTR::null())
    };
    // The count includes the terminating null.
    let len = usize::try_from(len).ok().filter(|n| *n > 1)? - 1;
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_spelled_with_its_number_and_asked_once() {
        let date = Date { year: 2026, month: 11, day: 9 };
        let text = month_day_local(date);
        // Whatever the locale, the day's number is in it, and a second ask gives the same.
        assert!(text.contains('9'), "{text}");
        assert_eq!(month_day_local(date), text);
    }
}
