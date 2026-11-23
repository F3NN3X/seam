//! Wallpapers per workspace and per screen (brief K, D791-D800): which picture each screen shows,
//! how it fits the screen, the names and size of the cache, and the checks a picture passes before
//! Seam opens it. Windows has one position for every screen, so Seam renders each screen's picture
//! at the screen's own size and Windows shows it with Fill (D791). Pure, except `read_header`.

use self::span::{Cut, Place};
use crate::config::Desk;
pub use crate::config::{Fit, Wallpaper, WallpaperSet};
use crate::workspace::{self, Workspace};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod journal;
pub mod span;

#[cfg(windows)]
pub mod desktop;
#[cfg(windows)]
pub mod render;
#[cfg(windows)]
pub mod worker;

/// The file types Seam lets a screen show. Anything else is refused before it is opened.
pub const EXTENSIONS: [&str; 15] =
    ["jpg", "jpeg", "jfif", "png", "bmp", "dib", "gif", "tif", "tiff", "jxr", "wdp", "webp", "heic", "heif", "avif"];
/// The longest side a picture may have, in pixels.
pub const MAX_SIDE: u32 = 20_000;
/// The most pixels a picture may have.
pub const MAX_PIXELS: u64 = 200_000_000;
/// The biggest picture file, in bytes.
pub const MAX_FILE: u64 = 256 * 1024 * 1024;
/// How much of a file `read_header` reads to find its size.
const HEADER_READ: usize = 256 * 1024;
/// The most the renders in the cache may take together (D800).
pub const CACHE_CAP: u64 = 512 * 1024 * 1024;

pub const NOT_A_PICTURE: &str =
    "Seam can't open that kind of file. Use a JPG, PNG, BMP, GIF, TIFF, WebP or HEIC picture";
pub const NO_CODEC: &str =
    "Windows can't open this picture. For HEIC or AVIF, get the extension from the Microsoft Store";
pub const TOO_BIG: &str = "That picture is too big. Use one up to 20,000 pixels on each side";
pub const UNREADABLE: &str = "Seam couldn't read that picture";
/// D793a: a one-screen workspace sets a picture for its own screen only.
pub const ONE_SCREEN: &str = "This workspace is for one screen, so it can only set that screen's picture";
/// D801: a one-screen workspace has no picture across all screens.
pub const NO_SPAN: &str = "A workspace for one screen can't have a picture across all screens";

/// Where a screen's picture comes from (D793).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The workspace in use on the screen sets it.
    Workspace,
    /// The desk default.
    Desk,
    /// A part of the picture across all screens, of the workspace or the desk default (D802).
    Span,
    /// My own wallpaper, as it was before Seam changed it.
    Own,
}

/// What screen `screen` can show, best first: the picture the workspace in use there sets for it,
/// then the desk default. Within a set, the screen's own picture wins over the picture across all
/// screens (D802). After these comes my own wallpaper. Only `in_use_on` says which workspace is in
/// use, so a one-screen workspace with no picture gives way to the desk default.
pub fn choices(desk: &Desk, screen: &str) -> Vec<(Wallpaper, Origin)> {
    let set_by = |set: Option<&WallpaperSet>, origin| {
        let set = set?;
        match set.screens.get(screen) {
            Some(own) => Some((own.clone(), origin)),
            None => set.span.clone().map(|span| (span, Origin::Span)),
        }
    };
    let in_use = workspace::in_use_on(desk, screen).and_then(|id| desk.workspace_wallpapers.get(id));
    let from_workspace = set_by(in_use, Origin::Workspace);
    let from_desk = set_by(Some(&desk.wallpaper), Origin::Desk);
    from_workspace.into_iter().chain(from_desk).collect()
}

/// The picture each of `screens` shows, and where it comes from (D793). `None` is my own wallpaper.
pub fn wanted(desk: &Desk, screens: &[&str]) -> Vec<(String, Option<Wallpaper>, Origin)> {
    let first = |s: &str| choices(desk, s).into_iter().next();
    let each = screens.iter().map(|s| match first(s) {
        Some((w, origin)) => (s.to_string(), Some(w), origin),
        None => (s.to_string(), None, Origin::Own),
    });
    each.collect()
}

/// `desk` as it will be once a switch to `id` ends, for `scope`'s screen or all of them, so the
/// pictures can change while the windows move (D794). The `away` screens, which a Snap back of the
/// desk-wide workspace leaves to their own, keep what is in use on them.
pub fn after_switch(desk: &Desk, id: &str, scope: Option<&str>, screens: &[&str], away: &[String]) -> Desk {
    let mut next = desk.clone();
    workspace::after_switch(&mut next, id, scope, screens);
    for screen in away {
        match desk.screen_in_use.get(screen) {
            Some(entry) => next.screen_in_use.insert(screen.clone(), entry.clone()),
            None => next.screen_in_use.remove(screen),
        };
    }
    next
}

/// What a screen is to show: a file Seam rendered, or my own wallpaper back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Show {
    Picture(String),
    Own,
}

/// The screens whose picture has to change, out of `wanted`, given the file Seam last set on
/// each screen. A screen Seam never set already shows my own wallpaper.
pub fn to_apply<'a>(applied: &BTreeMap<String, String>, wanted: &'a [(String, Show)]) -> Vec<&'a (String, Show)> {
    let differs = |(screen, show): &&(String, Show)| match show {
        Show::Picture(file) => applied.get(screen) != Some(file),
        Show::Own => applied.contains_key(screen),
    };
    wanted.iter().filter(differs).collect()
}

/// Whether `ws` (or the desk default, `None`) may set a picture for `screen` (D793a).
pub fn may_set(ws: Option<&Workspace>, screen: &str) -> Result<(), &'static str> {
    match ws.and_then(|w| w.scope.as_deref()) {
        Some(scope) if scope != screen => Err(ONE_SCREEN),
        _ => Ok(()),
    }
}

/// Whether `ws` (or the desk default, `None`) may have a picture across all screens (D801).
pub fn may_span(ws: Option<&Workspace>) -> Result<(), &'static str> {
    match ws.and_then(|w| w.scope.as_deref()) {
        Some(_) => Err(NO_SPAN),
        None => Ok(()),
    }
}

/// Whether a choice for `workspace` (the desk default with `None`) on `screen` can change what that
/// screen shows now: the desk default, or a workspace in use on it. Only such a choice ends the
/// hold after someone else changed the wallpaper (D799).
pub fn shows_now(desk: &Desk, workspace: Option<&str>, screen: &str) -> bool {
    workspace.is_none_or(|id| workspace::in_use_on(desk, screen) == Some(id))
}

/// Renders that failed, by cache key, with the screen each was for and why.
pub type Failures = BTreeMap<String, (String, &'static str)>;

/// Forget the failures of `screen`, or with `None` all of them, so their pictures are tried again:
/// a drive that was offline may be back, a codec may have come since.
pub fn retry(failures: &mut Failures, screen: Option<&str>) {
    failures.retain(|_, (on, _)| screen.is_some_and(|s| s != on));
}

/// Keep only the pictures a workspace on the desk can show: none for a deleted workspace, and for a
/// one-screen workspace only its own screen's, so a changed scope drops the old screen's picture
/// (D793a), and no picture across all screens (D801). Returns whether anything went.
pub fn keep_valid(desk: &mut Desk, list: &[Workspace]) -> bool {
    let before = desk.workspace_wallpapers.clone();
    desk.workspace_wallpapers.retain(|id, set| {
        let Some(ws) = list.iter().find(|w| w.id == *id) else { return false };
        if let Some(scope) = ws.scope.as_deref() {
            set.screens.retain(|screen, _| screen == scope);
            set.span = None;
        }
        !set.is_empty()
    });
    desk.workspace_wallpapers != before
}

/// A part of a picture, or of a screen, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// One copy of a part of the scaled picture onto the screen, at `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub from: Area,
    pub to: (u32, u32),
}

/// How a picture goes onto a screen: scaled to `scaled`, then copied as `piece`, once or, tiled,
/// again and again from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub scaled: (u32, u32),
    pub piece: Piece,
    pub tiled: bool,
    /// The screen's size.
    pub screen: (u32, u32),
}

impl Placement {
    /// Every copy onto the screen, left to right and top to bottom.
    pub fn pieces(&self) -> impl Iterator<Item = Piece> + '_ {
        let (sw, sh) = (self.piece.from.w.max(1), self.piece.from.h.max(1));
        let (dw, dh) = self.screen;
        let (cols, rows) = if self.tiled { (dw.div_ceil(sw), dh.div_ceil(sh)) } else { (1, 1) };
        (0..rows).flat_map(move |r| {
            (0..cols).map(move |c| match self.tiled {
                false => self.piece,
                true => {
                    let (x, y) = (c * sw, r * sh);
                    Piece { from: Area { x: 0, y: 0, w: sw.min(dw - x), h: sh.min(dh - y) }, to: (x, y) }
                }
            })
        })
    }
}

/// `a * b / c`, rounded, in 64 bits.
fn scale(a: u32, b: u32, c: u32) -> u32 {
    ((a as u64 * b as u64 + c as u64 / 2) / c.max(1) as u64) as u32
}

/// Where a picture of size `src` goes on a screen of size `dst` for `fit` (D796). Fill and Center
/// crop what doesn't fit, centred; Fit and Center leave bars; Tile repeats it at its own size.
pub fn fit_rect(src: (u32, u32), dst: (u32, u32), fit: Fit) -> Placement {
    let (sw, sh) = (src.0.max(1), src.1.max(1));
    let (dw, dh) = (dst.0.max(1), dst.1.max(1));
    // The picture is wider than the screen, for its height.
    let wider = sw as u64 * dh as u64 > dw as u64 * sh as u64;
    let whole = |w, h| Area { x: 0, y: 0, w, h };
    let place = |scaled: (u32, u32), piece| Placement { scaled, piece, tiled: false, screen: (dw, dh) };
    match fit {
        Fit::Fill => {
            let scaled = if wider { (scale(sw, dh, sh).max(dw), dh) } else { (dw, scale(sh, dw, sw).max(dh)) };
            let from = Area { x: (scaled.0 - dw) / 2, y: (scaled.1 - dh) / 2, w: dw, h: dh };
            place(scaled, Piece { from, to: (0, 0) })
        }
        Fit::Fit => {
            let scaled =
                if wider { (dw, scale(sh, dw, sw).clamp(1, dh)) } else { (scale(sw, dh, sh).clamp(1, dw), dh) };
            let to = ((dw - scaled.0) / 2, (dh - scaled.1) / 2);
            place(scaled, Piece { from: whole(scaled.0, scaled.1), to })
        }
        Fit::Stretch => place((dw, dh), Piece { from: whole(dw, dh), to: (0, 0) }),
        Fit::Center => {
            let (w, h) = (sw.min(dw), sh.min(dh));
            let from = Area { x: (sw - w) / 2, y: (sh - h) / 2, w, h };
            place((sw, sh), Piece { from, to: ((dw - w) / 2, (dh - h) / 2) })
        }
        Fit::Tile => {
            let piece = Piece { from: whole(sw.min(dw), sh.min(dh)), to: (0, 0) };
            Placement { scaled: (sw, sh), piece, tiled: true, screen: (dw, dh) }
        }
    }
}

/// Put a picture on a screen-sized canvas of 32-bit BGRA pixels, filled first with `bars` (blue,
/// green, red). `picture` holds `placement.piece.from` of the scaled picture, as premultiplied
/// BGRA, and goes over the bars where it is see-through.
pub fn compose(placement: &Placement, picture: &[u8], bars: [u8; 3]) -> Vec<u8> {
    let (dw, dh) = placement.screen;
    let mut canvas: Vec<u8> = [bars[0], bars[1], bars[2], 255].repeat(dw as usize * dh as usize);
    let stride = placement.piece.from.w as usize * 4;
    for piece in placement.pieces() {
        for row in 0..piece.from.h as usize {
            let src = &picture[row * stride..][..piece.from.w as usize * 4];
            let at = ((piece.to.1 as usize + row) * dw as usize + piece.to.0 as usize) * 4;
            let dst = &mut canvas[at..][..piece.from.w as usize * 4];
            for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                let clear = 255 - s[3] as u16;
                for (d, s) in d.iter_mut().zip(s).take(3) {
                    *d = (*s as u16 + (*d as u16 * clear + 127) / 255).min(255) as u8;
                }
            }
        }
    }
    canvas
}

/// FNV-1a, 64 bits: stable across builds, so the cache's names stay the same after an update.
fn hash(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.iter().chain(&[0xff]) {
            h = (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// The first half of a render's name: the picture, how it fits, the screen's size and, for a part
/// of a picture across all screens, where the screen stands in it (D806). That is known without the
/// file, so a picture that is gone is found by it, also after a restart (D800).
pub fn cache_prefix(path: &str, fit: Fit, colour: u32, size: (u32, u32), place: Option<&Place>) -> String {
    let (path, fit) = (path.to_lowercase(), format!("{fit:?}"));
    let place: Vec<[u8; 8]> = place
        .iter()
        .flat_map(|p| [p.bounds, p.rect])
        .flat_map(|r| [r.x, r.y, r.w, r.h].map(f64::to_le_bytes))
        .collect();
    let (colour, w, h) = (colour.to_le_bytes(), size.0.to_le_bytes(), size.1.to_le_bytes());
    let mut parts: Vec<&[u8]> = vec![path.as_bytes(), fit.as_bytes(), &colour, &w, &h];
    parts.extend(place.iter().map(|b| b.as_slice()));
    format!("{:016x}", hash(&parts))
}

/// A render's file name, without `.png`: the prefix, then the file's size and write time and, for
/// a part, the cut of the source it was made from (D806).
#[allow(clippy::too_many_arguments)]
pub fn cache_key(
    path: &str,
    len: u64,
    written: u64,
    fit: Fit,
    colour: u32,
    size: (u32, u32),
    place: Option<&Place>,
    cut: Option<Cut>,
) -> String {
    let cut: Vec<[u8; 4]> =
        cut.iter().flat_map(|c| [c.x.to_le_bytes(), c.y.to_le_bytes(), c.w.to_le_bytes(), c.h.to_le_bytes()]).collect();
    let (len, written) = (len.to_le_bytes(), written.to_le_bytes());
    let mut parts: Vec<&[u8]> = vec![&len, &written];
    parts.extend(cut.iter().map(|b| b.as_slice()));
    format!("{}-{:016x}", cache_prefix(path, fit, colour, size, place), hash(&parts))
}

/// A file in the cache: its size and when it was last used, in Unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cached {
    pub path: PathBuf,
    pub len: u64,
    pub used: u64,
}

/// What to delete so the cache is back under `cap`: the least recently used first, never one `keep` names.
pub fn evict(files: &[Cached], cap: u64, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut total: u64 = files.iter().map(|f| f.len).sum();
    let mut oldest: Vec<&Cached> = files.iter().filter(|f| !keep(&f.path)).collect();
    oldest.sort_by_key(|f| f.used);
    let mut gone = Vec::new();
    for f in oldest {
        if total <= cap {
            break;
        }
        total -= f.len;
        gone.push(f.path.clone());
    }
    gone
}

/// Whether a file's extension is one of `EXTENSIONS`, in any case.
pub fn accepted_extension(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    ext.is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()))
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le32(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A JPEG's size from its first start-of-frame segment.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        // Fill bytes before a marker.
        while *b.get(at)? == 0xff && *b.get(at + 1)? == 0xff {
            at += 1;
        }
        if *b.get(at)? != 0xff {
            return None;
        }
        let marker = *b.get(at + 1)?;
        let len = be16(b, at + 2)? as usize;
        let frame = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if frame {
            return Some((be16(b, at + 7)?, be16(b, at + 5)?));
        }
        at += 2 + len;
    }
}

/// A picture's size from the start of its file, for PNG, JPEG, BMP and GIF. `None` for another
/// kind, or one whose size isn't in `bytes`: the worker finds out when it opens it.
pub fn header_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => {
            let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
            let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
            Some((w, h))
        }
        [0xff, 0xd8, ..] => jpeg_size(bytes),
        [b'G', b'I', b'F', ..] => Some((le16(bytes, 6)?, le16(bytes, 8)?)),
        [b'B', b'M', ..] if le32(bytes, 14)? == 12 => Some((le16(bytes, 18)?, le16(bytes, 20)?)),
        [b'B', b'M', ..] => Some((le32(bytes, 18)?.unsigned_abs(), le32(bytes, 22)?.unsigned_abs())),
        _ => None,
    }
}

/// Whether a picture of this size is over the limits.
pub fn too_big((w, h): (u32, u32)) -> bool {
    w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS
}

/// The checks a picture passes before it is saved, from its name, its length and the start of
/// the file. The decode happens later, on the worker.
pub fn check_picture(path: &str, len: u64, header: &[u8]) -> Result<(), &'static str> {
    if !accepted_extension(path) {
        return Err(NOT_A_PICTURE);
    }
    if len > MAX_FILE {
        return Err(TOO_BIG);
    }
    if header.is_empty() {
        return Err(UNREADABLE);
    }
    match header_size(header) {
        Some(size) if too_big(size) => Err(TOO_BIG),
        _ => Ok(()),
    }
}

/// A picture's size from the start of its file, for the preview of a picture across all screens.
/// `None` for a kind `header_size` doesn't know, or a file that doesn't read.
pub fn file_size(path: &str) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut header = Vec::new();
    std::fs::File::open(path).ok()?.take(HEADER_READ as u64).read_to_end(&mut header).ok()?;
    header_size(&header)
}

/// `check_picture` for a file on disk.
pub fn read_header(path: &str) -> Result<(), &'static str> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| UNREADABLE)?;
    let len = file.metadata().map_err(|_| UNREADABLE)?.len();
    let mut header = Vec::new();
    file.take(HEADER_READ as u64).read_to_end(&mut header).map_err(|_| UNREADABLE)?;
    check_picture(path, len, &header)
}

#[cfg(test)]
mod tests;
