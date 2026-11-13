//! Save from windows, on the window-control worker: the windows on a screen, where
//! they are now, become a new library layout with a zone for each, which the screen
//! is given. Each window is then snapped into its zone, so it is stamped as for any snap.

use super::tile::{At, screen_of, windows_on};
use super::{Job, MAX_GAP, Outcome, centre, direction, geometry, into_zone};
use seam::groups;
use seam::library;
use seam::zones::{self, Rect, Template, ZoneLayout};

/// The part of `r` inside `area`, or None when nothing of it is.
fn inside(r: Rect, area: Rect) -> Option<Rect> {
    let (x, y) = (r.x.max(area.x), r.y.max(area.y));
    let (right, bottom) = ((r.x + r.w).min(area.x + area.w), (r.y + r.h).min(area.y + area.h));
    (right > x && bottom > y).then_some(Rect { x, y, w: right - x, h: bottom - y })
}

pub(super) fn run(job: &Job, at: At) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let geo = geometry(job, &screens);
    let s = screen_of(&geo, at).ok_or("No such screen")?;
    let g = &geo[s];
    // Each window's frame as far as it is in the work area.
    let seen: Vec<_> = windows_on(job, g).into_iter().filter_map(|(h, r)| Some((h, inside(r, g.work)?))).collect();
    if seen.is_empty() {
        return Err("No windows on that screen".into());
    }
    let rects: Vec<Rect> = seen.iter().map(|(_, r)| *r).collect();
    let tree = groups::from_rects(&rects, g.work).ok_or("These windows don't line up into zones")?;
    let gap = (job.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
    let (placed, cells) = (zones::rects(&tree, g.work, gap), zones::rects(&tree, g.work, 0));
    // `from_rects` has checked that each window's centre is in a zone of its own.
    let list = seen
        .iter()
        .filter_map(|(h, r)| {
            let (x, y) = centre(*r);
            let zone = zones::zone_at(&cells, x, y)?;
            Some(into_zone(*h, g, zone, zone, placed[zone]))
        })
        .collect();
    // The id is new to the library: the tray adds the layout under it when it saves.
    let layout = ZoneLayout { template: Template::Custom, custom: Some(library::new_id(&[])), tree };
    Ok(Outcome::Saved(list, g.id.clone(), layout))
}
