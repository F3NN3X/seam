//! Tile this screen's windows, on the window-control worker. The loose windows on
//! a screen go into its zones, or into a layout picked for how many there are when
//! the screen has none. The snapping's placement does the moving, so the zones are
//! stamped and recorded as for any snap; this works out which window goes where.

use super::{centre, close, direction, free, geometry, into_zone, movable, regroup, zone_of, Job, Outcome, MAX_GAP};
use crate::snap::Placement;
use seam::groups;
use seam::winutil;
use seam::zones::{self, Rect, ZoneLayout};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_EXSTYLE, WS_EX_TOPMOST};

/// Most windows one tile takes; the rest are left alone.
const MAX_TILED: usize = 9;

/// Which screen to tile.
#[derive(Clone, Copy, Debug)]
pub enum At {
    /// By Line Up number, from 1.
    Screen(usize),
    /// The screen this point is on.
    Point(i32, i32),
}

/// What a tile did, for the notice, the answer and Undo.
pub struct Done {
    /// Line Up number.
    pub number: usize,
    pub tiled: usize,
    /// Windows that found no zone, or were past the ninth.
    pub left: usize,
    /// The screen's id.
    pub screen: String,
    /// The tile gave the screen its layout, which Undo takes away again.
    pub picked: bool,
    /// Each moved window where it was.
    pub back: Vec<Placement>,
}

pub struct Plan {
    pub list: Vec<Placement>,
    /// The layout the screen is given, to save before anything moves.
    pub layout: Option<(String, ZoneLayout)>,
    pub done: Done,
}

fn topmost(h: HWND) -> bool {
    unsafe { GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

fn area(r: Rect) -> i64 {
    i64::from(r.w) * i64::from(r.h)
}

/// Which window takes which zone, as (window, zone) pairs. `first` takes the
/// largest free zone, then the others in turn take the free zone whose centre is
/// nearest their own. Windows past the last free zone get none.
fn assign(zones: &[Rect], mut open: Vec<usize>, windows: &[Rect], first: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let order = std::iter::once(first).chain((0..windows.len()).filter(|i| *i != first));
    for (turn, w) in order.enumerate() {
        let (cx, cy) = centre(windows[w]);
        let apart = |z: usize| {
            let (zx, zy) = centre(zones[z]);
            i64::from(zx - cx).pow(2) + i64::from(zy - cy).pow(2)
        };
        let best = if turn == 0 {
            open.iter().copied().enumerate().max_by_key(|(_, z)| (area(zones[*z]), std::cmp::Reverse(*z)))
        } else {
            open.iter().copied().enumerate().min_by_key(|(_, z)| (apart(*z), *z))
        };
        let Some((at, zone)) = best else { break };
        open.remove(at);
        out.push((w, zone));
    }
    out
}

/// The zones a window is already snapped in on screen `s`: the snapping's own
/// record if the window is still where it put it, or else the stamp if it fits.
fn placed(job: &Job, geo: &[super::Geo], s: usize, h: HWND, rect: Rect) -> Option<(usize, usize)> {
    let g = &geo[s];
    let sized = winutil::is_resizable(h);
    let record = job.members.iter().find(|m| {
        let (id, a, b) = &m.zone;
        m.hwnd == h.0 as isize
            && *id == g.id
            && *a <= *b
            && *b < g.zones.len()
            && (m.moving || close(rect, m.at, sized))
    });
    match record {
        Some(m) => Some((m.zone.1, m.zone.2)),
        None => zone_of(geo, h, rect, None).filter(|(here, ..)| *here == s).map(|(_, a, b, _)| (a, b)),
    }
}

pub(super) fn run(job: &Job, at: At) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let geo = geometry(job, &screens);
    let s = match at {
        At::Screen(number) => geo.iter().position(|g| g.number == number),
        At::Point(x, y) => geo.iter().position(|g| g.bounds.contains(x, y)).or(Some(0)),
    };
    let s = s.filter(|s| *s < geo.len()).ok_or("No such screen")?;
    let g = &geo[s];

    // Windows to tile, topmost first: normal app windows Seam may move, on this screen.
    let own = std::process::id();
    let mut seen: Vec<(HWND, Rect)> = Vec::new();
    for h in winutil::top_level_windows() {
        if winutil::pin_bits(h) != 0 || topmost(h) || !winutil::is_capturable(h, own) {
            continue;
        }
        let Ok(rect) = movable(h, &job.exclude) else { continue };
        let (x, y) = centre(rect);
        if g.bounds.contains(x, y) {
            seen.push((h, rect));
        }
    }
    let beyond = seen.len().saturating_sub(MAX_TILED);
    seen.truncate(MAX_TILED);

    let existing = job.layouts.get(screens[s].mm.index).and_then(Option::as_ref);
    let layout = match existing {
        Some(layout) => layout.clone(),
        None => groups::pick_for_count(seen.len()).ok_or("Nothing to tile")?,
    };
    let gap = (job.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
    let (zones, cells) = (zones::rects(&layout.tree, g.work, gap), zones::rects(&layout.tree, g.work, 0));

    // Windows already in a zone keep it. The others are loose.
    let mut taken = vec![false; zones.len()];
    let mut loose: Vec<(HWND, Rect)> = Vec::new();
    for (h, rect) in seen {
        match existing.and_then(|_| placed(job, &geo, s, h, rect)) {
            Some((a, b)) => regroup::covered(&cells, a, b).into_iter().for_each(|i| taken[i] = true),
            None => loose.push((h, rect)),
        }
    }
    if loose.is_empty() {
        return Err("Nothing to tile".into());
    }
    let open: Vec<usize> = (0..zones.len()).filter(|i| !taken[*i]).collect();
    if open.is_empty() {
        return Err("Every zone already has a window".into());
    }
    // The focused window if it is one of these, else the most recently active.
    let first = loose.iter().position(|(h, _)| h.0 as isize == job.hwnd).unwrap_or(0);
    let rects: Vec<Rect> = loose.iter().map(|(_, r)| *r).collect();
    let picks = assign(&zones, open, &rects, first);

    let list = picks.iter().map(|(w, z)| into_zone(loose[*w].0, g, *z, *z, zones[*z])).collect();
    let back = picks.iter().map(|(w, _)| free(loose[*w].0, g, loose[*w].1)).collect();
    let done = Done {
        number: g.number,
        tiled: picks.len(),
        left: beyond + loose.len() - picks.len(),
        screen: g.id.clone(),
        picked: existing.is_none(),
        back,
    };
    let layout = existing.is_none().then(|| (g.id.clone(), layout));
    Ok(Outcome::Tile(Box::new(Plan { list, layout, done })))
}
