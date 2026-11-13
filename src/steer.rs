//! What a snap drag does beyond the show key: the keys the drag's hook passes on,
//! pause to snap, and where the drop will put every window, which the overlay
//! previews and the drop carries out. A child of `snap`, so it works on the same
//! engine and drag. A part from the keys splits the zones in two, the screen's
//! layout becomes Custom and is saved once, as an edge drag's is, with Undo.

use super::*;
use crate::wincontrol::tile::{Back, Change, Done};
use dragkeys::{Key, Part};

/// A step of the wheel: Tab's choice of span under the pointer, and the share of it.
type Step = (usize, Option<Part>);
use seam::groups;
use seam::zones::MIN_ZONE_PX;
use std::time::Duration;

/// Pause to snap: a tick's move under this counts as resting.
const REST_PX: i32 = 4;
/// Resting this long shows the zones.
const REST_FOR: Duration = Duration::from_millis(300);
/// Moving this far from where the pointer came to rest starts the wait again.
const CANCEL_PX: i32 = 24;

/// Where a drop puts the dropped window, and what it does to the others.
pub(super) struct Outcome {
    /// The dropped window's visible frame.
    pub(super) rect: Rect,
    pub(super) split: Option<Split>,
    pub(super) swap: Option<(Vec<Placement>, bool)>,
}

impl Outcome {
    /// Where other windows land, for the preview's outlines.
    pub(super) fn others(&self) -> Vec<Rect> {
        match &self.split {
            Some(split) => split.occupant.map(|_| split.zones[split.rest]).into_iter().collect(),
            None => self.swap.iter().flat_map(|(list, _)| list.iter().map(|p| p.target)).collect(),
        }
    }
}

/// Zones cut in two by a part from the drag's keys.
pub(super) struct Split {
    before: ZoneLayout,
    layout: ZoneLayout,
    /// The new layout's zones with the gap, and with none.
    zones: Vec<Rect>,
    cells: Vec<Rect>,
    /// The dropped window's zone and the rest's, in the new layout.
    at: usize,
    rest: usize,
    /// The old zones that were cut.
    lo: usize,
    hi: usize,
    /// The one window that held them, and where it was.
    occupant: Option<(HWND, Rect)>,
}

impl Engine {
    /// What a drop on zones `(a, b)` of the drag's screen `s` does. A part from the
    /// keys cuts the zones in two when they are one part of the layout, both pieces
    /// are big enough, and at most one window holds them. Otherwise it's the usual
    /// drop, with a swap.
    pub(super) fn outcome(&self, drag: &Drag, s: usize, (a, b): (usize, usize), cut: Option<(Part, Side)>) -> Outcome {
        let (a, b) = (a.min(b), a.max(b));
        if let Some(split) = cut.and_then(|cut| self.split_plan(drag, s, (a, b), cut)) {
            return Outcome { rect: split.zones[split.at], split: Some(split), swap: None };
        }
        let g = &drag.geometry[s];
        Outcome { rect: zones::span(g.zones[a], g.zones[b]), split: None, swap: self.swap_plan(drag, s, (a, b)) }
    }

    pub(super) fn split_plan(
        &self,
        drag: &Drag,
        s: usize,
        (a, b): (usize, usize),
        (part, side): (Part, Side),
    ) -> Option<Split> {
        let g = &drag.geometry[s];
        let before = self.screens.iter().find(|x| x.id == g.id)?.layout.clone()?;
        let leaves = refit::covered(&g.cells, a, b);
        let (lo, hi) = (*leaves.first()?, *leaves.last()?);
        if hi - lo + 1 != leaves.len() {
            return None;
        }
        let (tree, at, rest) = groups::split_part(&before.tree, (lo, hi), part.share(), side)?;
        let cells = zones::rects(&tree, g.work, 0);
        if [at, rest].iter().any(|i| cells[*i].w < MIN_ZONE_PX || cells[*i].h < MIN_ZONE_PX) {
            return None;
        }
        let occupants = self.occupants(drag, g, zones::span(g.zones[a], g.zones[b]))?;
        // Every window the snapping has in these zones, shown or not. Only one that is
        // visible and wholly inside them can move to the rest; anything else is the usual drop.
        let leaf_in = |i: &usize| (lo..=hi).contains(i);
        let held: Vec<(isize, Vec<usize>)> = self
            .snapped
            .iter()
            .filter(|(k, x)| **k != drag.hwnd && x.screen == g.id && x.from.max(x.to) < g.cells.len())
            .map(|(k, x)| (*k, refit::covered(&g.cells, x.from, x.to)))
            .filter(|(_, leaves)| leaves.iter().any(leaf_in))
            .collect();
        match (held.as_slice(), occupants.as_slice()) {
            // A window on screen the snapping has no record of is one it can move too.
            ([], []) | ([], [_]) => {}
            ([(k, leaves)], [(h, _)]) if *k == h.0 as isize && leaves.iter().all(leaf_in) => {}
            _ => return None,
        }
        let gap = (self.settings.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
        Some(Split {
            layout: groups::relabel(before.template, tree.clone()),
            zones: zones::rects(&tree, g.work, gap),
            before,
            cells,
            at,
            rest,
            lo,
            hi,
            occupant: occupants.first().copied(),
        })
    }

    /// Drop with a split: the screen takes the new layout at once, the window goes
    /// to its part, the one window that held the zones goes to the rest, and the
    /// windows snapped in zones after the cut are renumbered. The layout is saved
    /// once, and the notice's Undo puts the layout and every window back.
    pub(super) fn split_drop(&mut self, drag: &Drag, hwnd: HWND, s: usize, split: Split) {
        let id = drag.geometry[s].id.clone();
        let Some(number) = self.screens.iter().position(|x| x.id == id) else { return };
        self.screens[number].layout = Some(split.layout.clone());
        let (g, cut) = (&drag.geometry[s], split.hi - split.lo + 1);
        let renumber = |i: usize| if i > split.hi { i + 2 - cut } else { i };
        let occupant = split.occupant.map(|(h, _)| h.0 as isize);
        let (mut back, mut keep) = (Vec::new(), Vec::new());
        // Every window snapped after the cut, hidden ones too, takes its new numbers.
        // Undo gives them back their old ones without moving them.
        for (key, x) in self.snapped.iter_mut() {
            if x.screen != id || *key == drag.hwnd || Some(*key) == occupant || x.from.max(x.to) <= split.hi {
                continue;
            }
            keep.push(Zoned { hwnd: *key, zone: (id.clone(), x.from, x.to), at: x.at, moving: false });
            (x.from, x.to) = (renumber(x.from), renumber(x.to));
            winutil::stamp_zones(hwnd_of(*key), &refit::covered(&split.cells, x.from, x.to));
        }
        if let Some(frame) = drag.frame {
            let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
            let on = self.screens.iter().find(|on| on.bounds.contains(x, y));
            let dpi = on.and_then(|on| work_area(on.bounds)).map_or(g.dpi, |(_, dpi)| dpi);
            let place = Placement { hwnd: drag.hwnd, target: frame, zone: drag.old_zone.clone(), dpi };
            back.push(Back { place, set: split.zones[split.at], pulled: None });
        }
        let placed = Placed {
            id: id.clone(),
            bounds: g.bounds,
            work: g.work,
            dpi: g.dpi,
            cells: split.cells.clone(),
            zones: split.zones.clone(),
        };
        self.snap_to(hwnd, &placed, (split.at, split.at), split.zones[split.at], drag.was_snapped);
        if let Some((h, at)) = split.occupant {
            let key = h.0 as isize;
            if let Some(x) = self.snapped.get(&key) {
                let place = Placement { hwnd: key, target: at, zone: Some((id.clone(), x.from, x.to)), dpi: g.dpi };
                back.push(Back { place, set: split.zones[split.rest], pulled: None });
            }
            let target = split.zones[split.rest];
            let zone = Some((id.clone(), split.rest, split.rest));
            self.queued.push_back(Placement { hwnd: key, target, zone, dpi: g.dpi });
            self.window_pump();
        }
        self.sync_overlay();
        let side = drag.shown.and_then(|v| v.cut).map(|(part, side)| format!("{} {side:?}", part.name()));
        log::info!(
            "zone split: screen {}, {}, the window that held it moved: {}",
            number + 1,
            side.unwrap_or_default(),
            split.occupant.is_some()
        );
        let done = Done {
            number: number + 1,
            tiled: 1,
            left: 0,
            screen: id.clone(),
            changed: Some(Change { now: Some(split.layout.clone()), before: Some(split.before) }),
            back,
            keep,
            pick: false,
            split: true,
            pulled: false,
        };
        wincontrol::save_split(self.owner, id, split.layout, done);
    }

    /// Drop outside every zone with Ctrl held: a window that was alone in its zone
    /// takes the zone out of the layout, its sibling takes the space, and the windows
    /// in the zones that grew move to fill them. As a split's, the layout is saved
    /// once and the notice's Undo puts it and every window back. The dragged window is
    /// the drop's to place.
    pub(super) fn pull_out(&mut self, drag: &Drag) {
        let Some(own) = self.snapped.get(&drag.hwnd).filter(|_| drag.was_snapped.is_some()) else { return };
        let (id, k) = (own.screen.clone(), own.from);
        if own.from != own.to {
            return;
        }
        let Some(g) = drag.geometry.iter().find(|g| g.id == id && k < g.cells.len()) else { return };
        let Some(number) = self.screens.iter().position(|x| x.id == id) else { return };
        let Some(before) = self.screens[number].layout.clone() else { return };
        let Some(tree) = zones::remove_leaf(&before.tree, k) else { return };
        let shared = self
            .snapped
            .iter()
            .any(|(key, x)| *key != drag.hwnd && x.screen == id && refit::covered(&g.cells, x.from, x.to).contains(&k));
        if shared {
            log::info!("pull out: screen {}, zone {}, another window holds it, so a plain drop", number + 1, k + 1);
            return;
        }
        let layout = groups::relabel(before.template, tree.clone());
        let gap = (self.settings.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
        let (cells, zone_rects) = (zones::rects(&tree, g.work, 0), zones::rects(&tree, g.work, gap));
        self.screens[number].layout = Some(layout.clone());
        let renumber = |i: usize| if i > k { i - 1 } else { i };
        let (mut back, mut keep, mut queue) = (Vec::new(), Vec::new(), Vec::new());
        for (key, x) in self.snapped.iter_mut() {
            if x.screen != id || *key == drag.hwnd {
                continue;
            }
            let (from, to) = (renumber(x.from), renumber(x.to));
            let ends = |rects: &[Rect], (a, b): (usize, usize)| Some(zones::span(*rects.get(a)?, *rects.get(b)?));
            let (Some(old), Some(new)) = (ends(&g.zones, (x.from, x.to)), ends(&zone_rects, (from, to))) else {
                continue;
            };
            let zone = (id.clone(), x.from, x.to);
            if new == old && (from, to) == (x.from, x.to) {
                continue;
            }
            // Undo gives every window it changed its old numbers back, moved back or not.
            keep.push(Zoned { hwnd: *key, zone: zone.clone(), at: x.at, moving: false });
            if new != old && !winutil::is_minimized(hwnd_of(*key)) {
                let place = Placement { hwnd: *key, target: x.at, zone: Some(zone), dpi: g.dpi };
                back.push(Back { place, set: new, pulled: None });
                queue.push(Placement { hwnd: *key, target: new, zone: Some((id.clone(), from, to)), dpi: g.dpi });
            }
            (x.from, x.to) = (from, to);
            winutil::stamp_zones(hwnd_of(*key), &refit::covered(&cells, from, to));
        }
        // Undo puts the pulled window back in its zone too, and with the size it had before it was snapped.
        let place = Placement { hwnd: drag.hwnd, target: g.zones[k], zone: Some((id.clone(), k, k)), dpi: g.dpi };
        back.push(Back { place, set: g.zones[k], pulled: drag.was_snapped });
        self.unsnap(hwnd_of(drag.hwnd), drag.stamped);
        let moved = queue.len();
        self.queued.extend(queue);
        self.window_pump();
        self.sync_overlay();
        log::info!("pull out: screen {}, zone {}, {moved} windows moved", number + 1, k + 1);
        let done = Done {
            number: number + 1,
            tiled: moved,
            left: 0,
            screen: id.clone(),
            changed: Some(Change { now: Some(layout.clone()), before: Some(before) }),
            back,
            keep,
            pick: false,
            split: false,
            pulled: true,
        };
        wincontrol::save_split(self.owner, id, layout, done);
    }

    /// Undo of a split: windows snapped after the cut take their old zone numbers
    /// back, stamps included, where the screen has its old layout again.
    pub(super) fn rezone(&mut self, list: Vec<Zoned>) {
        let geometry = self.geometry();
        for z in list {
            let (screen, from, to) = z.zone;
            let Some(g) = geometry.iter().find(|g| g.id == screen && from.max(to) < g.cells.len()) else { continue };
            winutil::stamp_zones(hwnd_of(z.hwnd), &refit::covered(&g.cells, from, to));
            self.keep_snapped(z.hwnd, &screen, (from, to), z.at);
        }
    }

    /// Pause to snap: a pointer that rests over a screen with zones for a moment
    /// shows them, and from then on the drag is as if the show key were held. With
    /// snap beside on, so does one that rests on a window's edge it can drop beside.
    pub(super) fn pause(&self, drag: &mut Drag, pt: POINT, pressed: bool) {
        let armed = self.settings.pause_to_snap && !drag.paused && !pressed;
        let zoned = |drag: &Drag| drag.geometry.iter().any(|g| g.bounds.contains(pt.x, pt.y));
        let over = armed && (zoned(drag) || (self.beside_on() && self.beside_at(drag, pt).is_some()));
        if !over {
            drag.rest = None;
            return;
        }
        let far = |a: POINT, b: POINT, px: i32| (a.x - b.x).abs().max((a.y - b.y).abs()) >= px;
        drag.rest = match drag.rest {
            Some((anchor, last, since)) if !far(pt, last, REST_PX) && !far(pt, anchor, CANCEL_PX + 1) => {
                drag.paused = since.elapsed() >= REST_FOR;
                Some((anchor, pt, since))
            }
            _ => Some((pt, pt, Instant::now())),
        };
    }

    /// A key from the drag's hook, while zones show: it changes what the drop does,
    /// and the overlay follows on the spot.
    pub(super) fn drag_key(&mut self, vk: u32, generation: u64) {
        if vk == 0 {
            // The hook took itself out at the cap: the strip goes, and the keys are the apps'.
            // A late one from an earlier drag's hook is about nothing now.
            let Some(mut drag) = self.drag.take() else { return };
            if !drag.keys.hooked || drag.keys.generation != generation {
                self.drag = Some(drag);
                return;
            }
            (drag.keys.hooked, drag.keys.capped) = (false, true);
            let (view, beside) = (drag.shown, drag.beside);
            if view.is_some() || beside.is_some() {
                self.present(&mut drag, view, beside);
            }
            self.drag = Some(drag);
            return;
        }
        let Some(key) = dragkeys::key_of(vk) else { return };
        let Some(drag) = self.drag.as_mut().filter(|d| (d.shown.is_some() || d.beside.is_some()) && !d.keys.off) else {
            return;
        };
        let name = dragkeys::name(key);
        if !drag.keys.used.contains(&name) {
            drag.keys.used.push(name);
        }
        // Beside a window, the wheel steps the share through 1/2, 1/3 and 2/3, stopping
        // at the ends; the side is the edge's, so Tab and the arrows do nothing.
        if drag.beside.is_some() && matches!(key, Key::Next | Key::Scroll(_) | Key::Side(_)) {
            if let Key::Scroll(next) = key {
                let shares = [Part::Half, Part::Third, Part::TwoThirds];
                let i = shares.iter().position(|p| Some(*p) == drag.keys.part).unwrap_or(0);
                let i = if next { (i + 1).min(shares.len() - 1) } else { i.saturating_sub(1) };
                drag.keys.part = Some(shares[i]);
            }
            self.tick();
            return;
        }
        match key {
            Key::Next => {
                let mut pt = POINT::default();
                let view = drag.shown.filter(|_| unsafe { GetCursorPos(&mut pt) }.is_ok());
                let Some(cells) = view.map(|v| &drag.geometry[v.screen].cells) else { return };
                let Some(zone) = zones::zone_at(cells, pt.x, pt.y) else { return };
                let n = dragkeys::candidates(cells, zone, (pt.x, pt.y)).len();
                let show_key = key_code(&self.settings.show_key).unwrap_or(VK_SHIFT);
                let back = show_key != VK_SHIFT && held(VK_SHIFT);
                let at = view.map(|v| (v.screen, zone));
                let now = if drag.keys.cycle_at == at { drag.keys.cycle.min(n - 1) } else { 0 };
                drag.keys.cycle = if back { (now + n - 1) % n } else { (now + 1) % n };
                drag.keys.cycle_at = at;
            }
            Key::Scroll(next) => {
                let mut pt = POINT::default();
                let view = drag.shown.filter(|_| unsafe { GetCursorPos(&mut pt) }.is_ok());
                let Some(cells) = view.map(|v| &drag.geometry[v.screen].cells) else { return };
                let Some(zone) = zones::zone_at(cells, pt.x, pt.y) else { return };
                let list = dragkeys::candidates(cells, zone, (pt.x, pt.y));
                let at = view.map(|v| (v.screen, zone));
                let shares = [None, Some(Part::Half), Some(Part::Third), Some(Part::TwoThirds)];
                let mut steps: Vec<Step> = shares.into_iter().map(|part| (0, part)).collect();
                steps.extend((1..list.len()).map(|i| (i, None)));
                let cycle = if drag.keys.cycle_at == at { drag.keys.cycle.min(list.len() - 1) } else { 0 };
                let now = (cycle, if cycle == 0 { drag.keys.part } else { None });
                let i = steps.iter().position(|s| *s == now).unwrap_or(0);
                // The ends stop the wheel rather than wrap, so a long turn can't overshoot.
                let i = if next { (i + 1).min(steps.len() - 1) } else { i.saturating_sub(1) };
                (drag.keys.cycle, drag.keys.part, drag.keys.cycle_at) = (steps[i].0, steps[i].1, at);
            }
            Key::Part(part) => drag.keys.part = part,
            Key::Side(side) => {
                let mut pt = POINT::default();
                let view = drag.shown.filter(|_| unsafe { GetCursorPos(&mut pt) }.is_ok());
                let at =
                    view.and_then(|v| Some((v.screen, zones::zone_at(&drag.geometry[v.screen].cells, pt.x, pt.y)?)));
                (drag.keys.side, drag.keys.side_at) = (Some(side), at);
            }
            Key::Cancel | Key::Button => {
                // This drop doesn't snap, so the keys go back to the apps, once the
                // release of this press has been swallowed too.
                drag.keys.off = true;
                dragkeys::release();
            }
        }
        self.tick();
    }
}
