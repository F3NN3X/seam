//! Global shortcuts in their config form, "Ctrl+Shift+Alt+L".

const MOD_ALT: u32 = 0x1;
const MOD_CONTROL: u32 = 0x2;
const MOD_SHIFT: u32 = 0x4;
const MOD_WIN: u32 = 0x8;
const MOD_NOREPEAT: u32 = 0x4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    /// `RegisterHotKey` modifier flags.
    pub mods: u32,
    /// Virtual-key code.
    pub vk: u32,
}

fn key_code(key: &str) -> Option<u32> {
    let upper = key.to_ascii_uppercase();
    if let [c @ (b'A'..=b'Z' | b'0'..=b'9')] = upper.as_bytes() {
        return Some(*c as u32);
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0x6F + n);
    }
    Some(match upper.as_str() {
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "SPACE" => 0x20,
        "TAB" => 0x09,
        "ENTER" => 0x0D,
        "ESC" => 0x1B,
        "BACKSPACE" => 0x08,
        "DELETE" => 0x2E,
        "INSERT" => 0x2D,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" => 0x21,
        "PAGEDOWN" => 0x22,
        _ => return None,
    })
}

/// A combo needs at least one of Ctrl, Alt or Win plus exactly one key.
pub fn parse(text: &str) -> Option<Combo> {
    let (mods, key) = text.rsplit_once('+')?;
    let mut flags = 0;
    for part in mods.split('+') {
        flags |= match part.trim().to_ascii_uppercase().as_str() {
            "CTRL" => MOD_CONTROL,
            "SHIFT" => MOD_SHIFT,
            "ALT" => MOD_ALT,
            "WIN" => MOD_WIN,
            _ => return None,
        };
    }
    if flags & (MOD_CONTROL | MOD_ALT | MOD_WIN) == 0 {
        return None;
    }
    Some(Combo { mods: flags | MOD_NOREPEAT, vk: key_code(key.trim())? })
}
