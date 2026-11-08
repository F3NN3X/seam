//! A desk is one set of connected screens. The tray and the panel both resolve
//! it from here, so they always agree on what's lined up.

use crate::config::{Config, Desk, SavedBorder, WindowSize};
use crate::geom::{touching, EdgeMap, Rect, Side};
use crate::layout::Link;
use crate::log;
use crate::state::{Border, BorderState};
use crate::zones;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Mon {
    /// Stable monitor device path (survives reboots and port changes on the same GPU).
    pub id: String,
    /// GDI name like \\.\DISPLAY1, used to match display-config source modes.
    pub gdi: String,
    /// Friendly name from EDID, e.g. "DELL U2723QE".
    pub name: String,
    /// False when `name` is only a stand-in because Windows reported none.
    pub named: bool,
    /// Desktop rect in physical pixels.
    pub rect: Rect,
    /// Physical pixels per inch from EDID (0 if unknown, e.g. some TVs).
    pub raw_dpi: f64,
    pub primary: bool,
}

/// Resolve saved borders against the monitors that are connected right now.
/// A border applies if the two screens still touch on the same side, or if
/// they still sit side by side on the desk without touching in Windows.
pub fn resolve(mons: &[Mon], saved: Option<&[SavedBorder]>) -> Vec<Link> {
    let Some(saved) = saved else {
        log::info!("no saved layout for these screens");
        return Vec::new();
    };
    let known: Vec<Link> = saved
        .iter()
        .filter_map(|s| {
            let a = mons.iter().position(|m| m.id == s.a)?;
            let b = mons.iter().position(|m| m.id == s.b)?;
            if !s.map.is_valid() {
                log::info!("link {a}->{b} dropped: invalid map {:?}", s.map);
                return None;
            }
            Some(Link { a, b, side: s.side, map: s.map })
        })
        .collect();
    let now = |l: &Link| touching(&mons[l.a].rect, &mons[l.b].rect);
    let mut links: Vec<Link> = known.iter().copied().filter(|l| now(l) == Some(l.side)).collect();
    // Screens apart in Windows are placed through the touching borders, so those come first.
    let apart = apart_neighbours(mons, &links);
    for l in known.iter().filter(|l| now(l) != Some(l.side)) {
        let still = apart
            .iter()
            .any(|p| (p.a, p.b, p.side) == (l.a, l.b, l.side) || (p.a, p.b, p.side) == (l.b, l.a, l.side.opposite()));
        if still && now(l).is_none() {
            links.push(*l);
        } else {
            log::info!("link {}->{} dropped: saved side {:?}, now {:?}", l.a, l.b, l.side, now(l));
        }
    }
    log::info!("resolved {} of {} saved link(s)", links.len(), saved.len());
    for l in &links {
        log::info!("link {}->{} {:?} s={} o={}", l.a, l.b, l.side, l.map.s, l.map.o);
    }
    links
}

pub fn has_neighbours(mons: &[Mon]) -> bool {
    (0..mons.len()).any(|i| (i + 1..mons.len()).any(|j| touching(&mons[i].rect, &mons[j].rect).is_some()))
}

/// Every pair of touching screens: the lined-up ones first, then the rest.
pub fn border_states(mons: &[Mon], links: &[Link]) -> Vec<Border> {
    let number = |i: usize| i as u32 + 1;
    let lined_up =
        links.iter().map(|l| Border { a: number(l.a), b: number(l.b), side: l.side, state: BorderState::LinedUp });
    let linked = |i: usize, j: usize| links.iter().any(|l| (l.a, l.b) == (i, j) || (l.a, l.b) == (j, i));
    let open = (0..mons.len()).flat_map(|i| (i + 1..mons.len()).map(move |j| (i, j))).filter_map(|(i, j)| {
        let side = touching(&mons[i].rect, &mons[j].rect).filter(|_| !linked(i, j))?;
        Some(Border { a: number(i), b: number(j), side, state: BorderState::NotLinedUp })
    });
    let apart = apart_neighbours(mons, links).into_iter().filter(|p| !linked(p.a, p.b)).map(|p| Border {
        a: number(p.a),
        b: number(p.b),
        side: p.side,
        state: BorderState::NotLinedUp,
    });
    lined_up.chain(open).chain(apart).collect()
}

/// A screen's place on the desk, in millimetres from the primary's corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Index into the monitor list, so the Line Up number is this plus one.
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// False when the screen reports no size, so 96 dpi is a guess.
    pub sized: bool,
}

fn dpi(m: &Mon) -> f64 {
    if m.raw_dpi > 0.0 {
        m.raw_dpi
    } else {
        96.0
    }
}

fn to_mm(px: f64, dpi: f64) -> f64 {
    px / dpi * 25.4
}

/// Where `next` starts along the edge it shares with the placed screen, in mm
/// from the placed screen's origin. A lined-up border knows this physically;
/// otherwise the Windows pixel offset is the best there is.
fn along_offset(mons: &[Mon], links: &[Link], placed: usize, next: usize, side: Side) -> f64 {
    let map = |a, b| links.iter().find(|l| (l.a, l.b) == (a, b)).map(|l| l.map);
    if let Some(m) = map(placed, next) {
        return to_mm(-m.o / m.s, dpi(&mons[placed]));
    }
    if let Some(m) = map(next, placed) {
        return to_mm(m.o, dpi(&mons[placed]));
    }
    let px = mons[next].rect.along_origin(side) - mons[placed].rect.along_origin(side);
    to_mm(px as f64, dpi(&mons[placed]))
}

/// The screens as they stand on the desk, for the Displays map. Walks outward
/// from the primary across touching screens.
pub fn physical_layout(mons: &[Mon], links: &[Link]) -> Vec<Placed> {
    let size = |i: usize| {
        let m = &mons[i];
        (to_mm(m.rect.width() as f64, dpi(m)), to_mm(m.rect.height() as f64, dpi(m)))
    };
    let at = |i: usize, x: f64, y: f64| {
        let (w, h) = size(i);
        Placed { index: i, x, y, w, h, sized: mons[i].raw_dpi > 0.0 }
    };
    let Some(first) = mons.iter().position(|m| m.primary).or((!mons.is_empty()).then_some(0)) else {
        return Vec::new();
    };
    let mut placed = vec![at(first, 0.0, 0.0)];
    let mut queue = VecDeque::from([first]);
    while let Some(i) = queue.pop_front() {
        let Some(from) = placed.iter().find(|p| p.index == i).copied() else { continue };
        for j in 0..mons.len() {
            if placed.iter().any(|p| p.index == j) {
                continue;
            }
            let Some(side) = touching(&mons[i].rect, &mons[j].rect) else { continue };
            let along = along_offset(mons, links, i, j, side);
            let (w, h) = size(j);
            placed.push(match side {
                Side::Right => at(j, from.x + from.w, from.y + along),
                Side::Left => at(j, from.x - w, from.y + along),
                Side::Bottom => at(j, from.x + along, from.y + from.h),
                Side::Top => at(j, from.x + along, from.y - h),
            });
            queue.push_back(j);
        }
    }
    // Screens that touch nothing sit in a row to the right.
    for j in 0..mons.len() {
        if !placed.iter().any(|p| p.index == j) {
            let right = placed.iter().map(|p| p.x + p.w).fold(f64::MIN, f64::max);
            placed.push(at(j, right + 50.0, 0.0));
        }
    }
    placed
}

/// A size in millimetres is a size on a screen of this many "pixels" per inch.
pub const MM_PER_INCH: f64 = 25.4;

/// A frame's width and height in millimetres on a screen of this raw density, or
/// `None` when the screen doesn't report its size.
pub fn size_mm(w: i32, h: i32, raw_dpi: f64) -> Option<[f32; 2]> {
    (raw_dpi > 0.0).then(|| [to_mm(w as f64, raw_dpi) as f32, to_mm(h as f64, raw_dpi) as f32])
}

/// A frame `size` pixels big, centred where the move put it (`at`) and kept inside
/// the work area.
pub fn centred(size: (f64, f64), at: zones::Rect, work: zones::Rect) -> zones::Rect {
    let w = (size.0.round() as i32).clamp(1, work.w.max(1));
    let h = (size.1.round() as i32).clamp(1, work.h.max(1));
    let (cx, cy) = (at.x + at.w / 2, at.y + at.h / 2);
    let x = (cx - w / 2).min(work.x + work.w - w).max(work.x);
    let y = (cy - h / 2).min(work.y + work.h - h).max(work.y);
    zones::Rect { x, y, w, h }
}

/// Real size (§11.4): a frame `size` pixels big on a screen of `src_dpi` raw
/// density, the same size on the desk on one of `dst_dpi`. That is
/// `px_dst = px_src × raw_dpi_dst / raw_dpi_src` per axis, `centred` on `at`.
/// `None` when either screen doesn't report its size.
pub fn physical_rect(
    size: (f64, f64),
    src_dpi: f64,
    dst_dpi: f64,
    at: zones::Rect,
    work: zones::Rect,
) -> Option<zones::Rect> {
    if !(src_dpi > 0.0 && dst_dpi > 0.0) {
        return None;
    }
    let k = dst_dpi / src_dpi;
    Some(centred((size.0 * k, size.1 * k), at, work))
}

/// The frame a window moved to another screen gets, as `mode` says: the pixels it
/// had there, or its real size, `centred` on where the move put it (`at`). `None`
/// leaves the size to Windows: its own mode, or a real size where either screen
/// doesn't report its size.
pub fn moved_rect(
    mode: WindowSize,
    size: (f64, f64),
    (src_dpi, dst_dpi): (f64, f64),
    at: zones::Rect,
    work: zones::Rect,
) -> Option<zones::Rect> {
    match mode {
        WindowSize::Windows => None,
        WindowSize::Pixels => Some(centred(size, at, work)),
        WindowSize::Real => physical_rect(size, src_dpi, dst_dpi, at, work),
    }
}

/// How far apart two edges can be on the desk and still count as side by side, in mm.
const NEAR_MM: f64 = 25.0;
/// How much of an edge two screens must share to count as neighbours, in mm.
const SHARED_MM: f64 = 50.0;

/// Two screens that sit side by side on the desk but don't touch in Windows'
/// pixel layout, so Line Up can still give them a border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Apart {
    pub a: usize,
    /// Right of or below `a`.
    pub b: usize,
    pub side: Side,
    /// Where the screens' sizes put the border, before it's lined up.
    pub guess: EdgeMap,
}

fn shared(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    a1.min(b1) - a0.max(b0)
}

/// Whether `q` sits against the `side` (Right or Bottom) of `p` on the desk.
fn beside(p: &Placed, q: &Placed, side: Side) -> bool {
    match side {
        Side::Right => (q.x - (p.x + p.w)).abs() <= NEAR_MM && shared(p.y, p.y + p.h, q.y, q.y + q.h) >= SHARED_MM,
        Side::Bottom => (q.y - (p.y + p.h)).abs() <= NEAR_MM && shared(p.x, p.x + p.w, q.x, q.x + q.w) >= SHARED_MM,
        Side::Left | Side::Top => false,
    }
}

/// Screens that meet on the desk but not in Windows, such as a tall screen
/// beside two stacked ones. Windows must still have them the same way round,
/// so a move off that edge heads towards the other screen.
pub fn apart_neighbours(mons: &[Mon], links: &[Link]) -> Vec<Apart> {
    let placed = physical_layout(mons, links);
    let at = |i: usize| placed.iter().find(|p| p.index == i).filter(|p| p.sized);
    let mut out = Vec::new();
    for i in 0..mons.len() {
        for j in (i + 1)..mons.len() {
            let (ri, rj) = (&mons[i].rect, &mons[j].rect);
            let (Some(pi), Some(pj)) = (at(i), at(j)) else { continue };
            if touching(ri, rj).is_some() {
                continue;
            }
            let found = [
                (rj.left >= ri.right && beside(pi, pj, Side::Right)).then_some((i, j, Side::Right)),
                (ri.left >= rj.right && beside(pj, pi, Side::Right)).then_some((j, i, Side::Right)),
                (rj.top >= ri.bottom && beside(pi, pj, Side::Bottom)).then_some((i, j, Side::Bottom)),
                (ri.top >= rj.bottom && beside(pj, pi, Side::Bottom)).then_some((j, i, Side::Bottom)),
            ];
            if let Some((a, b, side)) = found.into_iter().flatten().next() {
                let (pa, pb) = if a == i { (pi, pj) } else { (pj, pi) };
                let (start_a, start_b) = if side.along_is_y() { (pa.y, pb.y) } else { (pa.x, pb.x) };
                let (da, db) = (dpi(&mons[a]), dpi(&mons[b]));
                let guess = EdgeMap { s: db / da, o: (start_a - start_b) / 25.4 * db };
                out.push(Apart { a, b, side, guess });
            }
        }
    }
    out
}

/// "PG32UCDM + 3 more", or the generic name when no screen reports a name.
pub fn default_name(mons: &[Mon]) -> String {
    match (mons.iter().find(|m| m.named), mons.len()) {
        (Some(m), 1) => m.name.clone(),
        (Some(m), n) => format!("{} + {} more", m.name, n - 1),
        (None, n) => Desk::placeholder(n),
    }
}

/// A calendar day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// Days since 1970-01-01 (Howard Hinnant's algorithm).
    fn days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = i64::from((self.month + 9) % 12);
        let doy = (153 * mp + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

/// "12 Oct".
pub fn month_day(date: Date) -> String {
    let names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {}", date.day, names[(date.month as usize).clamp(1, 12) - 1])
}

/// How long ago `then` was, by the calendar: "today", "yesterday", "3 days ago",
/// then the date from a week on. A day in the future counts as today.
pub fn relative_day(today: Date, then: Date) -> String {
    match today.days() - then.days() {
        ..=0 => "today".to_string(),
        1 => "yesterday".to_string(),
        n @ 2..=6 => format!("{n} days ago"),
        _ => month_day(then),
    }
}

/// Record that this desk is the connected one, creating it the first time and
/// replacing a migrated desk's generic name now that the screens can be asked.
pub fn note_active(cfg: &mut Config, sig: &str, mons: &[Mon], now: u64) {
    let desk = cfg.desks.entry(sig.to_string()).or_default();
    if desk.name.is_empty() || desk.name == Desk::placeholder(mons.len()) {
        desk.name = default_name(mons);
    }
    desk.last_seen = now;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn days_are_counted_across_month_and_year_ends() {
        let today = day(2026, 1, 2);
        assert_eq!(relative_day(today, day(2026, 1, 2)), "today");
        assert_eq!(relative_day(today, day(2026, 1, 1)), "yesterday");
        assert_eq!(relative_day(today, day(2025, 12, 31)), "2 days ago");
        assert_eq!(relative_day(today, day(2025, 12, 27)), "6 days ago");
        assert_eq!(relative_day(today, day(2025, 12, 26)), "26 Dec");
        // A leap day is a day, and the clock running ahead isn't "ago".
        assert_eq!(relative_day(day(2024, 3, 1), day(2024, 2, 28)), "2 days ago");
        assert_eq!(relative_day(today, day(2026, 1, 5)), "today");
    }
}
