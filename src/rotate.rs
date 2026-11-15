//! Rotate windows on a screen with zones, on the window-control worker: every window
//! snapped in the zones moves to the next zone in zone order, or the previous one, and
//! a stack goes as one. Which zone goes where is `seam::turn`; this reads which windows
//! are in which zones, and which of them Seam may not move.

use super::tile::{At, Back, Done, Plan, Turn, Verb, placed, screen_of, topmost};
use super::{Job, Outcome, direction, geometry, into_zone, is_seam, movable};
use seam::turn::{self, Held};
use seam::winutil;

pub(super) fn run(job: &Job, at: At, forward: bool) -> Result<Outcome, String> {
    let screens = direction::desk_screens(&job.mons, &job.links);
    let geo = geometry(job, &screens);
    let s = screen_of(&geo, at).ok_or("No such screen")?;
    let g = &geo[s];
    if g.zones.len() < 2 {
        return Err(format!("Screen {} has fewer than two zones", g.number));
    }
    let own = std::process::id();
    let (mut held, mut blocked, mut windows) = (Vec::new(), Vec::new(), Vec::new());
    for h in winutil::top_level_windows() {
        if is_seam(h) || !winutil::is_snappable(h, own, &job.exclude) {
            continue;
        }
        let Some(rect) = winutil::visible_rect(h) else { continue };
        if !g.bounds.contains(rect.x + rect.w / 2, rect.y + rect.h / 2) {
            continue;
        }
        // A floating window isn't in any zone, so it stays.
        let Some((a, b)) = placed(job, &geo, s, h, rect) else { continue };
        if a != b {
            // A window over several zones keeps them, and the others step over them.
            blocked.extend(a..=b);
            continue;
        }
        let stuck = winutil::pin_bits(h) != 0 || topmost(h) || movable(h, &job.exclude).is_err();
        held.push(Held { hwnd: h.0 as isize, zone: a, stuck });
        windows.push(h);
    }
    let moves = turn::rotate(g.zones.len(), &held, &blocked, forward);
    if moves.is_empty() {
        return Err(format!("Nothing to rotate on screen {}", g.number));
    }
    let (mut list, mut back) = (Vec::new(), Vec::new());
    for (x, h) in held.iter().zip(&windows) {
        let Some((_, to)) = moves.iter().find(|(key, _)| *key == x.hwnd) else { continue };
        list.push(into_zone(*h, g, *to, *to, g.zones[*to]));
        let place = into_zone(*h, g, x.zone, x.zone, g.zones[x.zone]);
        back.push(Back { place, set: g.zones[*to], maximized: winutil::is_maximized(*h), pulled: None });
    }
    let done = Done {
        number: g.number,
        tiled: list.len(),
        left: held.len() - list.len(),
        screen: g.id.clone(),
        changed: None,
        back,
        keep: Vec::new(),
        pick: false,
        split: false,
        pulled: false,
        turn: Some(Turn { verb: Verb::Rotate, tree: None }),
    };
    Ok(Outcome::Tile(Box::new(Plan { list, layout: None, done })))
}
