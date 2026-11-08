//! The border colours the tray paints and the panel offers (D118). The config
//! keeps a name, so the colours can change here without touching saved settings.

/// One choice in a swatch row. `rgb` is 0xRRGGBB, and None means Windows' accent colour.
pub struct Swatch {
    pub name: &'static str,
    pub title: &'static str,
    pub rgb: Option<u32>,
}

pub const FOCUS_DEFAULT: &str = "accent";
pub const PIN_DEFAULT: &str = "amber";

pub static SWATCHES: [Swatch; 8] = [
    Swatch { name: "accent", title: "Windows accent", rgb: None },
    Swatch { name: "amber", title: "Seam amber", rgb: Some(0xFFB020) },
    Swatch { name: "blue", title: "Blue", rgb: Some(0x4C9AFF) },
    Swatch { name: "teal", title: "Teal", rgb: Some(0x2EC4B6) },
    Swatch { name: "green", title: "Green", rgb: Some(0x3DD68C) },
    Swatch { name: "red", title: "Red", rgb: Some(0xFF5C5C) },
    Swatch { name: "pink", title: "Pink", rgb: Some(0xFF6FB5) },
    Swatch { name: "white", title: "White", rgb: Some(0xF2F2F2) },
];

/// The swatch called `name`, or the one called `default` if there is none.
pub fn find(name: &str, default: &str) -> &'static Swatch {
    let by_name = |n: &str| SWATCHES.iter().find(|s| s.name == n);
    by_name(name).or_else(|| by_name(default)).unwrap_or(&SWATCHES[0])
}

/// 0xRRGGBB as a COLORREF (0x00BBGGRR), which is what DWM takes. The swap is
/// its own inverse, so it also turns a COLORREF back into 0xRRGGBB.
pub fn colorref(rgb: u32) -> u32 {
    (rgb >> 16 & 0xFF) | (rgb & 0xFF00) | (rgb & 0xFF) << 16
}

/// Windows' accent colour as a COLORREF, or None if neither source can be read.
/// `DWM\AccentColor` is the one Settings shows under Colours, stored as
/// 0xAABBGGRR, so dropping the alpha leaves a COLORREF. The colorization colour
/// (0xAARRGGBB) is the fallback.
#[cfg(windows)]
pub fn accent_colorref() -> Option<u32> {
    use windows::core::{w, BOOL};
    use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    let mut value = 0u32;
    let mut size = 4u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\DWM"),
            w!("AccentColor"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as _),
            Some(&mut size),
        )
    };
    if read.is_ok() {
        return Some(value & 0x00FF_FFFF);
    }
    let (mut argb, mut opaque) = (0u32, BOOL(0));
    unsafe { DwmGetColorizationColor(&mut argb, &mut opaque) }.ok()?;
    Some(colorref(argb & 0x00FF_FFFF))
}

/// The colour to paint for a config value, as a COLORREF. The accent is read now.
#[cfg(windows)]
pub fn resolve(name: &str, default: &str) -> u32 {
    match find(name, default).rgb {
        Some(rgb) => colorref(rgb),
        None => accent_colorref().unwrap_or(windows::Win32::Graphics::Dwm::DWMWA_COLOR_DEFAULT),
    }
}
