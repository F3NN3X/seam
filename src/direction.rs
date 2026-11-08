//! Directions on the desk. "Left" means left where the screens physically
//! stand, so every rect is measured in millimetres through its own screen
//! before anything is compared (D109).

use crate::desk::{physical_layout, Mon, Placed};
use crate::layout::Link;
use crate::zones::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Left, Dir::Right, Dir::Up, Dir::Down];

    pub fn parse(text: &str) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.name() == text)
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::Left => "left",
            Dir::Right => "right",
            Dir::Up => "up",
            Dir::Down => "down",
        }
    }
}

/// A screen: where it is in Windows' pixels, and where it stands on the desk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub px: Rect,
    pub mm: Placed,
}

/// The connected screens with their places on the desk.
pub fn desk_screens(mons: &[Mon], links: &[Link]) -> Vec<Screen> {
    let rect = |m: &Mon| Rect { x: m.rect.left, y: m.rect.top, w: m.rect.width(), h: m.rect.height() };
    physical_layout(mons, links).into_iter().map(|mm| Screen { px: rect(&mons[mm.index]), mm }).collect()
}

/// What lies in a direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// The index of the window in the list it was picked from.
    Window(usize),
    /// No window that way, but this screen (an index into the screens) is.
    Screen(usize),
    Nothing,
}

/// A rect on the desk, in mm.
#[derive(Clone, Copy, Debug)]
struct Mm {
    l: f64,
    t: f64,
    r: f64,
    b: f64,
}

impl Mm {
    fn centre(&self) -> (f64, f64) {
        ((self.l + self.r) / 2.0, (self.t + self.b) / 2.0)
    }
}

fn of_screen(s: &Screen) -> Mm {
    Mm { l: s.mm.x, t: s.mm.y, r: s.mm.x + s.mm.w, b: s.mm.y + s.mm.h }
}

/// The screen holding the centre of `r`, or failing that the nearest one.
fn screen_of(screens: &[Screen], r: Rect) -> Option<usize> {
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    let away = |s: &Screen| {
        let dx = (s.px.x - cx).max(cx - (s.px.x + s.px.w - 1)).max(0) as i64;
        let dy = (s.px.y - cy).max(cy - (s.px.y + s.px.h - 1)).max(0) as i64;
        dx * dx + dy * dy
    };
    (0..screens.len()).min_by_key(|i| away(&screens[*i]))
}

/// `r` on the desk, through the pixels-to-mm of the screen that holds its centre.
fn to_mm(screens: &[Screen], r: Rect) -> Option<Mm> {
    let s = &screens[screen_of(screens, r)?];
    let (kx, ky) = (s.mm.w / s.px.w.max(1) as f64, s.mm.h / s.px.h.max(1) as f64);
    let x = |px: i32| s.mm.x + (px - s.px.x) as f64 * kx;
    let y = |px: i32| s.mm.y + (px - s.px.y) as f64 * ky;
    Some(Mm { l: x(r.x), t: y(r.y), r: x(r.x + r.w), b: y(r.y + r.h) })
}

/// How far `to` is from `from` along the direction, and across it. None when
/// `to` isn't that way at all.
fn offsets(from: (f64, f64), to: (f64, f64), dir: Dir) -> Option<(f64, f64)> {
    let (along, across) = match dir {
        Dir::Left => (from.0 - to.0, to.1 - from.1),
        Dir::Right => (to.0 - from.0, to.1 - from.1),
        Dir::Up => (from.1 - to.1, to.0 - from.0),
        Dir::Down => (to.1 - from.1, to.0 - from.0),
    };
    (along > 0.0).then_some((along, across.abs()))
}

/// The candidate nearest in the direction: its centre must lie that way from
/// the source's centre, and it scores the distance along plus twice the offset
/// across, in mm. Candidates are topmost first, so a tie goes to the higher one.
pub fn pick(screens: &[Screen], candidates: &[Rect], source: Rect, dir: Dir) -> Option<usize> {
    let from = to_mm(screens, source)?.centre();
    let mut best: Option<(usize, f64)> = None;
    for (i, c) in candidates.iter().enumerate() {
        let Some((along, across)) = to_mm(screens, *c).and_then(|m| offsets(from, m.centre(), dir)) else { continue };
        let score = along + 2.0 * across;
        if best.is_none_or(|(_, b)| score < b) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// The screen physically next in the direction from `source`: one that shares
/// some of the source's span across the direction, with the nearest edge.
pub fn next_screen(screens: &[Screen], source: Rect, dir: Dir) -> Option<usize> {
    let own = screen_of(screens, source)?;
    let src = to_mm(screens, source)?;
    let from = src.centre();
    let mut best: Option<(usize, f64)> = None;
    for (i, s) in screens.iter().enumerate().filter(|(i, _)| *i != own) {
        let q = of_screen(s);
        if offsets(from, q.centre(), dir).is_none() {
            continue;
        }
        let (shared, edge, across) = match dir {
            Dir::Left | Dir::Right => {
                let edge = if dir == Dir::Left { from.0 - q.r } else { q.l - from.0 };
                (src.b.min(q.b) - src.t.max(q.t), edge, (q.t - from.1).max(from.1 - q.b).max(0.0))
            }
            Dir::Up | Dir::Down => {
                let edge = if dir == Dir::Up { from.1 - q.b } else { q.t - from.1 };
                (src.r.min(q.r) - src.l.max(q.l), edge, (q.l - from.0).max(from.0 - q.r).max(0.0))
            }
        };
        if shared <= 0.0 {
            continue;
        }
        let score = edge.max(0.0) + 2.0 * across;
        if best.is_none_or(|(_, b)| score < b) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// The window in the direction, or the screen to fall through to.
pub fn find(screens: &[Screen], windows: &[Rect], source: Rect, dir: Dir) -> Found {
    if let Some(i) = pick(screens, windows, source, dir) {
        return Found::Window(i);
    }
    next_screen(screens, source, dir).map_or(Found::Nothing, Found::Screen)
}

/// The screen (an index into `screens`) that holds the centre of `r`.
pub fn screen_at(screens: &[Screen], r: Rect) -> Option<usize> {
    screen_of(screens, r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{EdgeMap, Rect as PxRect, Side};

    fn mon(id: &str, rect: PxRect, raw_dpi: f64, primary: bool) -> Mon {
        Mon { id: id.into(), gdi: String::new(), name: id.into(), named: true, rect, raw_dpi, primary }
    }

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Test #7 (D109). The desk: a PG32UCDM, an LG above it, and a tall U3477
    /// to the right of both. In Windows' pixels the U3477 only touches the
    /// PG32UCDM, so "left" from its top half must still reach the LG.
    #[test]
    fn direction_search_works_in_desk_millimetres() {
        let mons = [
            mon("PG32UCDM", PxRect::new(0, 0, 3840, 2160), 138.5, true),
            mon("LG", PxRect::new(0, -1440, 2560, 1440), 93.0, false),
            mon("U3477", PxRect::new(3840, -1280, 1440, 3440), 109.0, false),
        ];
        let links = [
            Link { a: 1, b: 0, side: Side::Bottom, map: EdgeMap { s: 1.4892, o: 34.76 }, glued: false },
            Link { a: 0, b: 2, side: Side::Right, map: EdgeMap { s: 0.787, o: 1720.04 }, glued: false },
        ];
        let screens = desk_screens(&mons, &links);
        let number = |found: Found| match found {
            Found::Screen(i) => Some(screens[i].mm.index),
            _ => None,
        };
        // The physical map the cases rely on: the U3477's top is level with the LG.
        let u = screens.iter().find(|s| s.mm.index == 2).unwrap().mm;
        let lg = screens.iter().find(|s| s.mm.index == 1).unwrap().mm;
        assert!((u.x - 704.2).abs() < 0.5 && (u.y + 400.8).abs() < 0.5, "{u:?}");
        assert!(u.y < lg.y && lg.y < u.y + 50.0, "{lg:?}");

        let left_half = r(0, 0, 1920, 2160);
        let right_half = r(1920, 0, 1920, 2160);
        let lg_full = r(0, -1440, 2560, 1440);
        let u_top = r(3840, -1280, 1440, 1200);
        let u_bottom = r(3840, 1200, 1440, 960);
        let cases: [(&str, Vec<Rect>, Rect, Dir, Found); 9] = [
            // Same screen: the right half, not the LG above it or the screen beyond.
            ("same screen", vec![lg_full, right_half], left_half, Dir::Right, Found::Window(1)),
            // Across screens: right from the PG32UCDM's right half is the U3477's window level with it.
            ("cross screen", vec![u_top, u_bottom, left_half], right_half, Dir::Right, Found::Window(1)),
            ("cross screen up", vec![u_top, lg_full], left_half, Dir::Up, Found::Window(1)),
            // Two windows in the same place: the higher one in the z-order.
            ("z-order tie", vec![right_half, right_half], left_half, Dir::Right, Found::Window(0)),
            // A window that isn't that way at all is never picked, so it falls through to the U3477.
            ("behind", vec![left_half], right_half, Dir::Right, Found::Screen(2)),
            // No window: fall through to the screen physically next.
            ("portrait top", vec![], r(3940, -1180, 1240, 800), Dir::Left, Found::Screen(1)),
            ("portrait bottom", vec![], r(3940, 1300, 1240, 700), Dir::Left, Found::Screen(0)),
            ("nothing above", vec![], lg_full, Dir::Up, Found::Nothing),
            ("nothing right", vec![lg_full], u_top, Dir::Right, Found::Nothing),
        ];
        for (name, windows, source, dir, expected) in cases {
            let found = find(&screens, &windows, source, dir);
            match expected {
                Found::Screen(n) => assert_eq!(number(found), Some(n), "{name}: {found:?}"),
                _ => assert_eq!(found, expected, "{name}"),
            }
        }
    }
}
