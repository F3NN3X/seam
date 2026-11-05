//! Puts the app icon in the exe as resource 1, where Explorer, the taskbar and
//! GPUI's window class all look for it. Both exes use this script. The linker
//! takes a compiled `.res` directly, so this needs no resource compiler.

use std::path::Path;
use std::{env, fs};

const ICON: &[u8] = include_bytes!("assets/seam.ico");
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const MOVEABLE_DISCARDABLE: u16 = 0x1010;
const MOVEABLE_PURE_DISCARDABLE: u16 = 0x1030;
const LANG_EN_US: u16 = 0x0409;

fn entry(res: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
    res.extend((data.len() as u32).to_le_bytes());
    res.extend(32u32.to_le_bytes());
    for word in [0xFFFF, kind, 0xFFFF, id] {
        res.extend(word.to_le_bytes());
    }
    res.extend(0u32.to_le_bytes());
    res.extend(flags.to_le_bytes());
    res.extend(LANG_EN_US.to_le_bytes());
    res.extend([0; 8]);
    res.extend(data);
    res.resize(res.len().next_multiple_of(4), 0);
}

/// A `.ico` and an icon group share a layout, except that the group names each
/// image by resource id where the file gives its offset.
fn compile(ico: &[u8]) -> Vec<u8> {
    let le32 = |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]) as usize;
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut res = Vec::new();
    entry(&mut res, 0, 0, 0, &[]);
    let mut group = ico[..6].to_vec();
    for i in 0..count {
        let at = 6 + i * 16;
        let id = i as u16 + 1;
        let (size, offset) = (le32(at + 8), le32(at + 12));
        entry(&mut res, RT_ICON, id, MOVEABLE_DISCARDABLE, &ico[offset..offset + size]);
        group.extend(&ico[at..at + 12]);
        group.extend(id.to_le_bytes());
    }
    entry(&mut res, RT_GROUP_ICON, 1, MOVEABLE_PURE_DISCARDABLE, &group);
    res
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rerun-if-changed={}", Path::new(&manifest).join(file!()).display());
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ENV");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let out = Path::new(&env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("icon.res");
    fs::write(&out, compile(ICON)).expect("write icon.res");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
