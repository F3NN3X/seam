//! Which Windows this is, for About and diagnostics.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, PCWSTR, w};

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

/// The build as a number, `None` if Windows doesn't say.
pub fn build_number() -> Option<u32> {
    build().parse().ok()
}

/// Settings > Personalization > Colors > Transparency effects.
pub fn transparency_enabled() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    // Windows has it on until someone turns it off, so a missing value is on.
    read == ERROR_FILE_NOT_FOUND || (read == ERROR_SUCCESS && value != 0)
}

/// A contrast theme is on.
pub fn high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// Settings > Accessibility > Visual effects > Animation effects.
pub fn animation_effects() -> bool {
    let mut on = BOOL(1);
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    // Windows has them on until someone turns them off.
    read.is_err() || on.as_bool()
}

/// Settings > Accessibility > Text size, as a percentage: 100 to 225, 100 until someone changes it.
pub fn text_scale_percent() -> u32 {
    let mut value = 100u32;
    let mut size = size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Accessibility"),
            w!("TextScaleFactor"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if read == ERROR_SUCCESS { value.clamp(100, 225) } else { 100 }
}

/// The colours of the contrast theme that is on, as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemColours {
    pub window: u32,
    pub window_text: u32,
    pub highlight: u32,
    pub highlight_text: u32,
    pub gray_text: u32,
    pub button_face: u32,
    pub hotlight: u32,
}

/// One system colour, from Windows' `0x00BBGGRR` to `0xRRGGBB`.
fn system_colour(index: SYS_COLOR_INDEX) -> u32 {
    let bgr = unsafe { GetSysColor(index) };
    (bgr & 0xFF) << 16 | (bgr & 0xFF00) | (bgr >> 16 & 0xFF)
}

/// The system colours while a contrast theme is on, `None` otherwise.
pub fn contrast_colours() -> Option<SystemColours> {
    high_contrast().then(|| SystemColours {
        window: system_colour(COLOR_WINDOW),
        window_text: system_colour(COLOR_WINDOWTEXT),
        highlight: system_colour(COLOR_HIGHLIGHT),
        highlight_text: system_colour(COLOR_HIGHLIGHTTEXT),
        gray_text: system_colour(COLOR_GRAYTEXT),
        button_face: system_colour(COLOR_BTNFACE),
        hotlight: system_colour(COLOR_HOTLIGHT),
    })
}
