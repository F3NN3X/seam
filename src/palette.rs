//! The border colours the tray paints and the panel offers (D118, D122). The config
//! keeps a name, "none" or a `#RRGGBB` colour, so the named colours can change here
//! without touching saved settings.

/// One choice in a swatch row. `rgb` is 0xRRGGBB, and None means Windows' accent colour.
pub struct Swatch {
    pub name: &'static str,
    pub title: &'static str,
    pub rgb: Option<u32>,
}

/// The value for "leave the border alone".
pub const NONE: &str = "none";
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

/// What a config value asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    /// Seam leaves that border alone.
    None,
    /// Windows' accent colour, read when it is painted.
    Accent,
    Rgb(u32),
}

/// `#RRGGBB` as 0xRRGGBB. Nothing else counts: no short form, no alpha.
pub fn parse_hex(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// "none", a swatch name or `#RRGGBB`.
fn parse(value: &str) -> Option<Colour> {
    if value == NONE {
        return Some(Colour::None);
    }
    if let Some(rgb) = parse_hex(value) {
        return Some(Colour::Rgb(rgb));
    }
    SWATCHES.iter().find(|s| s.name == value).map(|s| s.rgb.map_or(Colour::Accent, Colour::Rgb))
}

/// The colour a config value stands for, or the default's if it isn't one.
pub fn colour(value: &str, default: &str) -> Colour {
    parse(value).or_else(|| parse(default)).unwrap_or(Colour::Accent)
}

/// The value to keep: a hex colour in upper case, or the default for a value that isn't a colour.
pub fn normalise(value: &str, default: &str) -> String {
    match (parse_hex(value), parse(value)) {
        (Some(rgb), _) => format!("#{rgb:06X}"),
        (None, Some(_)) => value.to_string(),
        (None, None) => default.to_string(),
    }
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

/// The colour to paint for a config value, as a COLORREF, or None to leave the
/// border alone. The accent is read now.
#[cfg(windows)]
pub fn resolve(value: &str, default: &str) -> Option<u32> {
    match colour(value, default) {
        Colour::None => None,
        Colour::Rgb(rgb) => Some(colorref(rgb)),
        Colour::Accent => Some(accent_colorref().unwrap_or(windows::Win32::Graphics::Dwm::DWMWA_COLOR_DEFAULT)),
    }
}
