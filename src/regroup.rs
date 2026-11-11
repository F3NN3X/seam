//! Grow, shrink and Priority on the window-control worker. A group is the focused
//! window's screen with its layout, and every window in those zones: the ones the
//! snapping knows, and those whose stamp still fits after a restart of the tray.
//! The solver works out the new layout, then every member whose place changed moves.
//! A drag of a window's edge finds its group with the same two functions.

use super::{close, direction, geometry, hwnd_of, into_zone, movable, stamped_in, zone_of, Job, Outcome, MAX_GAP};
use crate::{minsize, snap};
use seam::api::GroupVerb;
use seam::groups::{self, Stash};
use seam::winutil;
use seam::zones::{self, Node, Rect, Template, ZoneLayout};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

/// What Priority changed on a screen, to give back on the next press.
pub(super) struct Given {
    template: Template,
    stash: Stash,
    /// The tree Priority made. Any other change to the group drops the stash.
    after: Node,
}

/// Per screen id. The worker reads it; it changes only once a layout is saved.
static GIVEN: Mutex<Option<HashMap<String, Given>>> = Mutex::new(None);

/// A group command's layout was saved: drop what Priority kept for the screen, and
/// keep what a new Priority gave.
pub(super) fn saved(screen: &str, keep: Option<Given>) {
    let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
    let given = given.get_or_insert_with(HashMap::new);
    given.remove(screen);
    if let Some(keep) = keep {
        given.insert(screen.to_string(), keep);
    }
}

/// The zones a window covers when it spans `from` to `to`, as the snapping counts them.
fn covered(cells: &[Rect], from: usize, to: usize) -> Vec<usize> {
    let (Some(a), Some(b)) = (cells.get(from), cells.get(to)) else { return Vec::new() };
    let r = zones::span(*a, *b);
    let inside = |c: &Rect| c.x >= r.x && c.y >= r.y && c.x + c.w <= r.x + r.w && c.y + c.h <= r.y + r.h;
    cells.iter().enumerate().filter(|(_, c)| inside(c)).map(|(i, _)| i).collect()
}

/// A window that still exists, can be snapped, and is where the snapping put it.
fn still_there(h: HWND, own: u32, exclude: &[String], at: Rect, moving: bool) -> bool {
    if !unsafe { IsWindow(Some(h)) }.as_bool() || !winutil::is_snappable(h, own, exclude) {
        return false;
    }
    moving || winutil::visible_rect(h).is_some_and(|r| close(r, at, winutil::is_resizable(h)))
}

/// The windows in one screen's zones: `first`, then the snapping's own record in
/// `known`, then the windows whose stamp still fits. `placed` is the screen's zones
/// with the gap. Windows that closed, were minimized or moved away since aren't in
/// the group. Worker only: it reads every window's rect.
pub fn group_members(
    first: (HWND, usize, usize),
    screen: &str,
    bounds: Rect,
    placed: &[Rect],
    known: &[snap::Zoned],
    exclude: &[String],
) -> Vec<(HWND, usize, usize)> {
    let own = std::process::id();
    let mut members = vec![first];
    for m in known {
        let (id, a, b) = &m.zone;
        let (h, fits) = (hwnd_of(m.hwnd), id == screen && *a <= *b && *b < placed.len());
        if fits && h != first.0 && still_there(h, own, exclude, m.at, m.moving) {
            members.push((h, *a, *b));
        }
    }
    for h in winutil::top_level_windows() {
        if members.iter().any(|(m, ..)| *m == h) || !winutil::has_zone_stamp(h) {
            continue;
        }
        if !winutil::is_snappable(h, own, exclude) || winutil::is_hung(h) {
            continue;
        }
        let Some(r) = winutil::visible_rect(h) else { continue };
        if !bounds.contains(r.x + r.w / 2, r.y + r.h / 2) {
            continue;
        }
        if let Some((a, b, _)) = stamped_in(h, r, placed) {
            members.push((h, a, b));
        }
    }
    members
}

/// Each leaf's minimum for the solver, from the smallest size of every member's
/// app at `dpi`. Worker only: asking an app can wait on it.
pub fn group_mins(tree: &Node, work: Rect, gap: i32, dpi: u32, members: &[(HWND, usize, usize)]) -> Vec<(i32, i32)> {
    let cells = zones::rects(tree, work, 0);
    let sizes: Vec<(Vec<usize>, (i32, i32))> =
        members.iter().map(|(h, a, b)| (covered(&cells, *a, *b), minsize::of(*h, dpi))).collect();
    groups::leaf_mins(tree, work, gap, &sizes)
}

pub(super) fn run(job: &Job, verb: GroupVerb) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let source = hwnd_of(job.hwnd);
    let rect = movable(source, &job.exclude)?;
    let geo = geometry(job, &screens);
    let (s, from, to, _) = zone_of(&geo, source, rect, job.snapped.as_ref()).ok_or("That window isn't in a zone")?;
    let g = &geo[s];
    let layout = job.layouts.get(screens[s].mm.index).and_then(Option::as_ref).ok_or("That window isn't in a zone")?;
    let members = group_members((source, from, to), &g.id, g.bounds, &g.zones, &job.members, &job.exclude);
    let gap = (job.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
    let mins = group_mins(&layout.tree, g.work, gap, g.dpi, &members);
    let leaves = &covered(&zones::rects(&layout.tree, g.work, 0), from, to);

    let earlier = {
        let given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        let found = given.as_ref().and_then(|m| m.get(&g.id)).filter(|p| p.after == layout.tree);
        found.map(|p| (p.template, p.stash.clone()))
    };
    let (new, keep) = match verb {
        GroupVerb::Grow(step) => {
            let solved =
                groups::grow(&layout.tree, g.work, leaves, step, &mins).ok_or("No room to grow that window")?;
            (groups::relabel(layout.template, solved.tree), None)
        }
        GroupVerb::Shrink(step) => {
            let solved =
                groups::shrink(&layout.tree, g.work, leaves, step, &mins).ok_or("That window can't get any smaller")?;
            (groups::relabel(layout.template, solved.tree), None)
        }
        GroupVerb::Priority => match earlier {
            // A second press gives the group back as it was.
            Some((template, stash)) => (ZoneLayout { template, tree: groups::restore(stash) }, None),
            None => {
                let (tree, stash) = groups::priority(&layout.tree, g.work, leaves, &mins);
                let keep = Given { template: layout.template, stash, after: tree.clone() };
                (groups::relabel(layout.template, tree), Some(keep))
            }
        },
    };
    if new.tree == layout.tree {
        return Err("That window already has its share of the group".into());
    }
    let before = zones::rects(&layout.tree, g.work, gap);
    let after = zones::rects(&new.tree, g.work, gap);
    let mut list = Vec::new();
    for (h, a, b) in members {
        let target = zones::span(after[a], after[b]);
        if target == zones::span(before[a], before[b]) {
            continue;
        }
        // A neighbour that can't move, hung or running as administrator, stops the
        // whole change, as with Swap.
        movable(h, &job.exclude)?;
        list.push(into_zone(h, g, a, b, target));
    }
    Ok(Outcome::Regroup(list, g.id.clone(), new, keep))
}
