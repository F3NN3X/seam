//! A desk is one set of connected screens. The tray and the panel both resolve
//! it from here, so they always agree on what's lined up.

use crate::config::{Config, Desk, SavedBorder};
use crate::geom::{touching, Rect, Side};
use crate::layout::Link;
use crate::log;
use crate::state::{Border, BorderState};
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
/// A border only applies if the two screens still touch on the same side.
pub fn resolve(mons: &[Mon], saved: Option<&[SavedBorder]>) -> Vec<Link> {
    let Some(saved) = saved else {
        log::info!("no saved layout for these screens");
        return Vec::new();
    };
    let links: Vec<Link> = saved
        .iter()
        .filter_map(|s| {
            let a = mons.iter().position(|m| m.id == s.a)?;
            let b = mons.iter().position(|m| m.id == s.b)?;
            if !s.map.is_valid() {
                log::info!("link {a}->{b} dropped: invalid map {:?}", s.map);
                return None;
            }
            let now = touching(&mons[a].rect, &mons[b].rect);
            if now != Some(s.side) {
                log::info!("link {a}->{b} dropped: saved side {:?}, now {now:?}", s.side);
            }
            (now == Some(s.side)).then_some(Link { a, b, side: s.side, map: s.map })
        })
        .collect();
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
    lined_up.chain(open).collect()
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

/// "PG32UCDM + 3 more", or the generic name when no screen reports a name.
pub fn default_name(mons: &[Mon]) -> String {
    match (mons.iter().find(|m| m.named), mons.len()) {
        (Some(m), 1) => m.name.clone(),
        (Some(m), n) => format!("{} + {} more", m.name, n - 1),
        (None, n) => Desk::placeholder(n),
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
