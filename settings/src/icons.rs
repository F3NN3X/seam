//! App icons for the workspace list. The shell hands them over as RGBA pixels off the
//! UI thread; gpui takes an encoded image, so they are wrapped as a PNG.

use crate::panel::Panel;
use gpui_kit::*;
use seam::appicon::{self, Icon};
use seam::workspace::Rule;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Asked for at twice the drawn size, so it stays sharp at 200% scaling.
const REQUEST_PX: i32 = 32;
/// An icon that couldn't be had is asked for again at most this often.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// One app's icon. Loading and failed both draw the placeholder.
enum Slot {
    Loading,
    Ready(Arc<Image>),
    /// Since when, so it's asked for again only after a while.
    Failed(Instant),
}

/// Icons by app identity, kept for the life of the panel.
#[derive(Default)]
pub struct Icons {
    by_app: HashMap<String, Slot>,
}

impl Icons {
    pub fn get(&self, rule: &Rule) -> Option<Arc<Image>> {
        match self.by_app.get(&identity(rule)) {
            Some(Slot::Ready(image)) => Some(image.clone()),
            _ => None,
        }
    }

    /// The Workspaces page shows again: icons that failed a minute or more ago are
    /// forgotten, so the next draw asks for them again.
    pub fn retry_failed(&mut self) {
        self.by_app.retain(|_, slot| !matches!(slot, Slot::Failed(at) if at.elapsed() >= RETRY_AFTER));
    }
}

/// The package for a Store app, else the exe path.
pub fn identity(rule: &Rule) -> String {
    match &rule.aumid {
        Some(id) if !id.is_empty() => id.to_lowercase(),
        _ => rule.exe.to_lowercase(),
    }
}

/// Starts loading every icon in `rules` that hasn't been asked for yet.
pub fn request(panel: &mut Panel, rules: &[Rule], cx: &mut Context<Panel>) {
    for rule in rules {
        let key = identity(rule);
        if key.is_empty() || panel.icons.by_app.contains_key(&key) {
            continue;
        }
        panel.icons.by_app.insert(key.clone(), Slot::Loading);
        let (exe, aumid) = (rule.exe.clone(), rule.aumid.clone());
        cx.spawn(async move |this, cx| {
            let image = cx
                .background_executor()
                .spawn(async move { appicon::load(&exe, aumid.as_deref(), REQUEST_PX).map(|i| Arc::new(to_image(&i))) })
                .await;
            let _ = this.update(cx, |panel, cx| match image {
                Some(image) => {
                    panel.icons.by_app.insert(key, Slot::Ready(image));
                    cx.notify();
                }
                None => {
                    panel.icons.by_app.insert(key, Slot::Failed(Instant::now()));
                }
            });
        })
        .detach();
    }
}

fn to_image(icon: &Icon) -> Image {
    Image::from_bytes(ImageFormat::Png, png(icon))
}

/// An 8-bit RGBA PNG with stored (uncompressed) blocks; an icon is a few KB either way.
fn png(icon: &Icon) -> Vec<u8> {
    let stride = icon.width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * icon.height as usize);
    for row in icon.rgba.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&icon.width.to_be_bytes());
    header.extend_from_slice(&icon.height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
