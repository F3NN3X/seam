//! Tile this screen's windows, on the window-control worker. The loose windows on
//! a screen go into its zones, or into a layout picked for how many there are when
//! the screen has none. The snapping's placement does the moving, so the zones are
//! stamped and recorded as for any snap; this works out which window goes where.
//! A layout picked on the Zones page moves that screen's snapped windows the same way.

use super::{
    centre, close, direction, free, geometry, into_zone, movable, regroup, zone_of, Geo, Job, Outcome, MAX_GAP,
};
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

/// A layout picked on the Zones page for a screen, and the one it had.
#[derive(Clone, Debug)]
pub struct Pick {
    /// The screen's id.
    pub screen: String,
    pub layout: ZoneLayout,
    pub before: Option<ZoneLayout>,
    /// The window to treat as focused, when it isn't the one in front.
    pub focused: Option<isize>,
}

/// A screen's layout a tile or a pick changed. Undo puts `before` back while the
/// screen still has `now`.
#[derive(Clone, Debug)]
pub struct Change {
    pub now: Option<ZoneLayout>,
    pub before: Option<ZoneLayout>,
}

/// What a tile, a pick or a split did, for the notice, the answer and Undo.
pub struct Done {
    /// Line Up number.
    pub number: usize,
    /// Windows put into zones.
    pub tiled: usize,
    /// Windows that found no zone, or were past the ninth.
    pub left: usize,
    /// The screen's id.
    pub screen: String,
    /// The layout change, if there was one.
    pub changed: Option<Change>,
    /// Each moved window, for Undo.
    pub back: Vec<Back>,
    /// A layout picked on the Zones page rather than a tile.
    pub pick: bool,
    /// A drop that split a zone with the drag's keys rather than a tile.
    pub split: bool,
}

/// A window the tile moved.
pub struct Back {
    /// Where it was.
    pub place: Placement,
    /// Where the tile put it, to tell whether it has moved since.
    pub set: Rect,
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
    let largest = open.iter().copied().enumerate().max_by_key(|(_, z)| (area(zones[*z]), std::cmp::Reverse(*z)));
    let Some((at, zone)) = largest.filter(|_| first < windows.len()) else { return Vec::new() };
    open.remove(at);
    let rest = (0..windows.len()).filter(|i| *i != first).map(|w| (w, centre(windows[w])));
    std::iter::once((first, zone)).chain(nearest(zones, &mut open, rest)).collect()
}

/// Each window in turn takes the free zone whose centre is nearest its point, as
/// (window, zone) pairs, and the zone is no longer free. Windows past the last free zone get none.
fn nearest(
    zones: &[Rect],
    open: &mut Vec<usize>,
    windows: impl IntoIterator<Item = (usize, (i32, i32))>,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (w, point) in windows {
        let Some((at, zone)) = open.iter().copied().enumerate().min_by_key(|(_, z)| (apart(zones[*z], point), *z))
        else {
            break;
        };
        open.remove(at);
        out.push((w, zone));
    }
    out
}

/// How far a zone's centre is from a point, squared.
fn apart(zone: Rect, (x, y): (i32, i32)) -> i64 {
    let (zx, zy) = centre(zone);
    i64::from(zx - x).pow(2) + i64::from(zy - y).pow(2)
}

/// The closest window and free zone of all go together first, then the closest of
/// the rest, and so on, so no two windows share a zone and none takes a zone
/// another fits better only because it came first. Ties go to the earlier window.
fn closest_first(zones: &[Rect], open: &mut Vec<usize>, windows: &[(usize, (i32, i32))]) -> Vec<(usize, usize)> {
    let mut pairs: Vec<(i64, usize, usize)> = (0..windows.len())
        .flat_map(|turn| open.iter().map(move |z| (turn, *z)))
        .map(|(turn, z)| (apart(zones[z], windows[turn].1), turn, z))
        .collect();
    pairs.sort_unstable();
    let mut out = Vec::new();
    let mut placed = vec![false; windows.len()];
    for (_, turn, z) in pairs {
        if placed[turn] || !open.contains(&z) {
            continue;
        }
        placed[turn] = true;
        open.retain(|o| *o != z);
        out.push((windows[turn].0, z));
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

/// The screen `at` names, as its place in `geo`.
pub(super) fn screen_of(geo: &[Geo], at: At) -> Option<usize> {
    let s = match at {
        At::Screen(number) => geo.iter().position(|g| g.number == number),
        At::Point(x, y) => geo.iter().position(|g| g.bounds.contains(x, y)).or(Some(0)),
    };
    s.filter(|s| *s < geo.len())
}

/// Normal app windows Seam may move on screen `g`, topmost first. Pinned, sticky
/// and topmost windows aren't among them.
pub(super) fn windows_on(job: &Job, g: &Geo) -> Vec<(HWND, Rect)> {
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
    seen
}

pub(super) fn run(job: &Job, at: At) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let geo = geometry(job, &screens);
    let s = screen_of(&geo, at).ok_or("No such screen")?;
    let g = &geo[s];

    // Windows to tile, topmost first: normal app windows Seam may move, on this screen.
    let seen = windows_on(job, g);

    let existing = job.layouts.get(screens[s].mm.index).and_then(Option::as_ref);
    let layout = match existing {
        Some(layout) => layout.clone(),
        None => groups::pick_for_count(seen.len().min(MAX_TILED)).ok_or("Nothing to tile")?,
    };
    let gap = (job.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
    let (zones, cells) = (zones::rects(&layout.tree, g.work, gap), zones::rects(&layout.tree, g.work, 0));

    // Windows already in a zone keep it. The others are loose.
    let mut taken = vec![false; zones.len()];
    let mut loose: Vec<(HWND, Rect)> = Vec::new();
    // Every window is checked before the cap, so a zone held past the ninth isn't taken as free.
    for (h, rect) in seen {
        match existing.and_then(|_| placed(job, &geo, s, h, rect)) {
            Some((a, b)) => regroup::covered(&cells, a, b).into_iter().for_each(|i| taken[i] = true),
            None => loose.push((h, rect)),
        }
    }
    if loose.is_empty() {
        return Err("Nothing to tile".into());
    }
    let beyond = loose.len().saturating_sub(MAX_TILED);
    loose.truncate(MAX_TILED);
    let open: Vec<usize> = (0..zones.len()).filter(|i| !taken[*i]).collect();
    if open.is_empty() {
        return Err("Every zone already has a window".into());
    }
    // The focused window if it is one of these, else the most recently active.
    let first = loose.iter().position(|(h, _)| h.0 as isize == job.hwnd).unwrap_or(0);
    let rects: Vec<Rect> = loose.iter().map(|(_, r)| *r).collect();
    let picks = assign(&zones, open, &rects, first);

    let list = picks.iter().map(|(w, z)| into_zone(loose[*w].0, g, *z, *z, zones[*z])).collect();
    let back = picks.iter().map(|(w, z)| Back { place: free(loose[*w].0, g, loose[*w].1), set: zones[*z] }).collect();
    let done = Done {
        number: g.number,
        tiled: picks.len(),
        left: beyond + loose.len() - picks.len(),
        screen: g.id.clone(),
        changed: existing.is_none().then(|| Change { now: Some(layout.clone()), before: None }),
        back,
        pick: false,
        split: false,
    };
    let layout = existing.is_none().then(|| (g.id.clone(), layout));
    Ok(Outcome::Tile(Box::new(Plan { list, layout, done })))
}

/// A layout picked for a screen: the windows Tile would take there each get a zone
/// of their own. Windows snapped in the old zones go first, the closest pair of a
/// window's old zones and a free zone at a time; then the loose ones, the
/// focused one first, each to the free zone nearest its own middle. Windows past
/// the last zone stay where they are. The screen's old layout is still in the job,
/// so the stamps are read against it.
pub(super) fn pick(job: &Job, p: &Pick) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let geo = geometry(job, &screens);
    let s = geo.iter().position(|g| g.id == p.screen).ok_or("No such screen")?;
    let g = &geo[s];
    let gap = (job.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
    let zones = zones::rects(&p.layout.tree, g.work, gap);
    let seen = windows_on(job, g);
    // The old zones each window is snapped in, if it is.
    let old: Vec<Option<(usize, usize)>> =
        seen.iter().map(|(h, rect)| (!g.zones.is_empty()).then(|| placed(job, &geo, s, *h, *rect)).flatten()).collect();
    let span = |(a, b): (usize, usize)| zones::span(g.zones[a], g.zones[b]);
    let mut open: Vec<usize> = (0..zones.len()).collect();
    let snapped: Vec<_> = (0..seen.len()).filter_map(|w| old[w].map(|ab| (w, centre(span(ab))))).collect();
    let mut picks = closest_first(&zones, &mut open, &snapped);
    let front = p.focused.unwrap_or(job.hwnd);
    let focused = seen.iter().position(|(h, _)| h.0 as isize == front);
    let loose = (0..seen.len()).filter(|w| old[*w].is_none());
    let loose = focused.filter(|f| old[*f].is_none()).into_iter().chain(loose.filter(|w| Some(*w) != focused));
    picks.extend(nearest(&zones, &mut open, loose.map(|w| (w, centre(seen[w].1))).collect::<Vec<_>>()));

    let list = picks.iter().map(|(w, z)| into_zone(seen[*w].0, g, *z, *z, zones[*z])).collect();
    let back = picks
        .iter()
        .map(|(w, z)| {
            let (h, rect) = seen[*w];
            let place = match old[*w] {
                Some((a, b)) => into_zone(h, g, a, b, span((a, b))),
                None => free(h, g, rect),
            };
            Back { place, set: zones[*z] }
        })
        .collect();
    let done = Done {
        number: g.number,
        tiled: picks.len(),
        left: seen.len() - picks.len(),
        screen: p.screen.clone(),
        changed: Some(Change { now: Some(p.layout.clone()), before: p.before.clone() }),
        back,
        pick: true,
        split: false,
    };
    Ok(Outcome::Tile(Box::new(Plan { list, layout: Some((p.screen.clone(), p.layout.clone())), done })))
}
