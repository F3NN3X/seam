//! A desk is one set of connected screens. The tray and the panel both resolve
//! it from here, so they always agree on what's lined up.

use crate::config::{Config, Desk, SavedBorder};
use crate::geom::{touching, Rect};
use crate::layout::Link;
use crate::log;
use crate::state::{Border, BorderState};

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
