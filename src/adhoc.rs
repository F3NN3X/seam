//! Snap beside a window, the ad-hoc groups it makes, and Fit group to screen. A
//! child of `snap`, so it works on the same engine and drag. During a snap drag,
//! the pointer in the band along an edge of the window under it makes that side
//! the target. On a screen with zones the drop splits the zones that window holds,
//! as a part from the drag's keys does (D370). Elsewhere the two windows become an
//! ad-hoc group: a tree over their area, as a screen's zones are, kept in memory
//! only. Its members follow each other's edges and get handles between them, and a
//! member dragged away, minimized, maximized or closed leaves it.

use super::*;
use crate::wincontrol::tile::Back;
use dragkeys::Part;
use seam::beside;
use seam::zones::{Node, Template};

/// An ad-hoc group's name where a screen's id goes: the divider handles and the
/// layout saves of their drags.
const PREFIX: &str = "adhoc:";
/// What a group, a window in zones or a window as big as its screen answers.
const FILLS: &str = "Already fills its screen";

/// Windows put side by side on a screen with no zones.
pub(super) struct Adhoc {
    pub(super) id: u64,
    /// The screen it's on.
    pub(super) screen: String,
    pub(super) area: Rect,
    /// The gap at that screen's scale when the group was made.
    pub(super) gap: i32,
    pub(super) tree: Node,
    /// Each window and its zone in `tree`.
    pub(super) members: Vec<(isize, usize)>,
}

impl Adhoc {
    /// Its name where a screen's id goes.
    pub(super) fn key(&self) -> String {
        format!("{PREFIX}{}", self.id)
    }

    /// Where its windows are, by zone.
    pub(super) fn cells(&self) -> Vec<Rect> {
        zones::rects(&self.tree, self.area, self.gap)
    }
}

/// Snap beside: the window whose edge the pointer is on, which side, and the share
/// the dropped window takes.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Beside {
    /// In `Engine::screens`.
    pub(super) screen: usize,
    pub(super) target: isize,
    /// Its visible frame from the drag's look.
    pub(super) at: Rect,
    pub(super) side: Side,
    pub(super) part: Part,
}

/// A drop beside a window on a screen with no zones, worked out.
pub(super) struct Plan {
    /// The group the target is in, or None for a new one.
    group: Option<usize>,
    area: Rect,
    gap: i32,
    tree: Node,
    /// Every member with its zone in `tree`, the dropped window last.
    members: Vec<(isize, usize)>,
    /// Where the dropped window and the target go.
    pub(super) rect: Rect,
    pub(super) target: Rect,
}

/// What Fit group to screen did, for its notice and Undo.
pub struct Fitted {
    /// Line Up number.
    pub number: usize,
    /// The screen's id.
    pub screen: String,
    /// Each moved window, for Undo.
    pub back: Vec<Back>,
    /// The ad-hoc group that was fitted, and its area before, which Undo gives back.
    pub group: Option<(u64, Rect)>,
}

/// Whether a screen's id is an ad-hoc group's name.
pub fn is_adhoc(screen: &str) -> bool {
    screen.starts_with(PREFIX)
}

/// A divider handle's drag changed an ad-hoc group: the group keeps the new tree,
/// and nothing is saved. False when `screen` is a real screen's id.
pub fn adhoc_dragged(screen: &str, tree: &Node) -> bool {
    let Some(id) = screen.strip_prefix(PREFIX) else { return false };
    let id = id.parse::<u64>().ok();
    with_engine(|e| {
        if let Some(a) = e.adhoc.iter_mut().find(|a| Some(a.id) == id) {
            a.tree = tree.clone();
        }
        e.sync_dividers();
    });
    true
}

/// Undo of a fit: the ad-hoc group's area goes back.
pub fn adhoc_area(id: u64, area: Rect) {
    with_engine(|e| {
        if let Some(a) = e.adhoc.iter_mut().find(|a| a.id == id) {
            a.area = area;
        }
        e.sync_dividers();
    });
}

/// Stretch the group of a window to fill its screen's work area, keeping its
/// windows' shares and order. A window in no group takes the work area itself.
pub fn fit(hwnd: HWND) -> Result<Fitted, String> {
    with_engine(|e| e.fit(hwnd)).unwrap_or_else(|| Err("Seam is busy, try again".into()))
}

impl Engine {
    /// The gap at a screen's scale.
    fn gap_px(&self, dpi: u32) -> i32 {
        (self.settings.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32
    }

    /// A window on its way somewhere counts as being there.
    fn moving(&self, key: isize) -> bool {
        self.pending.as_ref().is_some_and(|p| p.hwnd == key) || self.queued.iter().any(|q| q.hwnd == key)
    }

    /// The ad-hoc group a window is in, and its zone there.
    pub(super) fn adhoc_of(&self, key: isize) -> Option<(usize, usize)> {
        self.adhoc.iter().enumerate().find_map(|(i, a)| a.members.iter().find(|(k, _)| *k == key).map(|(_, l)| (i, *l)))
    }

    /// The group's tree after its divider moved, from an edge drag.
    pub(super) fn adhoc_set_tree(&mut self, id: u64, tree: Node) {
        if let Some(a) = self.adhoc.iter_mut().find(|a| a.id == id) {
            a.tree = tree;
        }
    }

    /// Take a window out of its ad-hoc group. The zone beside it takes its room and
    /// its windows move there, unless that leaves the area smaller, and a group of
    /// one is no group.
    pub(super) fn leave_adhoc(&mut self, key: isize) {
        let Some((i, leaf)) = self.adhoc_of(key) else { return };
        let a = &mut self.adhoc[i];
        a.members.retain(|(k, _)| *k != key);
        let closed = beside::remove(&a.tree, a.area, a.gap, leaf).filter(|_| a.members.len() >= 2);
        let Some((tree, area)) = closed else {
            log::info!("snap beside: a window left its group, which is no group now");
            self.adhoc.remove(i);
            self.sync_dividers();
            return;
        };
        let before = a.cells();
        (a.tree, a.area) = (tree, area);
        let after = a.cells();
        let dpi = self.screens.iter().find(|s| s.id == a.screen).and_then(|s| work_area(s.bounds)).map_or(96, |w| w.1);
        let mut moves = Vec::new();
        for (k, l) in a.members.iter_mut() {
            let old = *l;
            if *l > leaf {
                *l -= 1;
            }
            // One that is going too isn't brought back to move it.
            let h = hwnd_of(*k);
            let away = winutil::is_minimized(h) || winutil::is_maximized(h);
            if let (Some(was), Some(set), false) = (before.get(old), after.get(*l), away)
                && was != set
            {
                moves.push(Placement { hwnd: *k, target: *set, zone: None, dpi });
            }
        }
        log::info!("snap beside: a window left its group, {} stay", a.members.len());
        self.queued.extend(moves);
        self.window_pump();
        self.sync_dividers();
    }

    /// Drop the members that are gone: closed, minimized or maximized, and with
    /// `placed`, no longer where their group put them. A window on its way counts as
    /// there, and one on another desktop is left alone.
    pub(super) fn prune_adhoc(&mut self, placed: bool) {
        let mut gone = Vec::new();
        for a in &self.adhoc {
            let cells = a.cells();
            for &(k, leaf) in &a.members {
                let h = hwnd_of(k);
                if !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_minimized(h) || winutil::is_maximized(h) {
                    gone.push(k);
                    continue;
                }
                if !placed || self.moving(k) || winutil::is_cloaked(h) {
                    continue;
                }
                let there =
                    winutil::visible_rect(h).zip(cells.get(leaf)).is_some_and(|(r, c)| swap::mostly_inside(r, *c));
                if !there {
                    gone.push(k);
                }
            }
        }
        for k in gone {
            self.leave_adhoc(k);
        }
    }

    /// The ad-hoc groups for the divider handles, as screens of their own.
    pub(super) fn adhoc_scene(&self, screens: &mut Vec<dividers::Screen>, known: &mut Vec<Zoned>) {
        for a in &self.adhoc {
            let Some((_, dpi)) = self.screens.iter().find(|s| s.id == a.screen).and_then(|s| work_area(s.bounds))
            else {
                continue;
            };
            let layout = ZoneLayout { template: Template::Custom, custom: None, tree: a.tree.clone() };
            screens.push(dividers::Screen { id: a.key(), bounds: a.area, work: a.area, dpi, gap: a.gap, layout });
            let cells = a.cells();
            for &(k, leaf) in &a.members {
                let Some(at) = cells.get(leaf).copied() else { continue };
                known.push(Zoned { hwnd: k, zone: (a.key(), leaf, leaf), at, moving: self.moving(k) });
            }
        }
    }

    /// Whether a window may be the target, asked once per drag: the same windows
    /// as everything Seam moves.
    fn may_take(&self, drag: &mut Drag, key: isize) -> bool {
        if let Some((_, ok)) = drag.allowed.iter().find(|(k, _)| *k == key) {
            return *ok;
        }
        let ok = wincontrol::movable(hwnd_of(key), &self.exclude).is_ok();
        drag.allowed.push((key, ok));
        ok
    }

    /// What the pointer at `pt` is beside: the window under it whose edge band it is
    /// in, with the zones view that splits its zones on a screen with zones, or no
    /// view for an ad-hoc group elsewhere. None when the pointer isn't on an edge, or
    /// a drop there could do nothing, so the drag shows zones as usual. A pass over
    /// the drag's one look at the windows; what it works out is kept until it changes.
    pub(super) fn beside_at(&self, drag: &mut Drag, pt: POINT) -> Option<(Beside, Option<View>)> {
        if !drag.looked {
            drag.others = self.snapshot(drag.hwnd, &drag.landed);
            drag.looked = true;
        }
        let (target, at) = drag.others.iter().copied().find(|(_, r)| r.contains(pt.x, pt.y))?;
        let side = beside::band(at, (pt.x, pt.y))?;
        if winutil::is_maximized(hwnd_of(target)) || !self.may_take(drag, target) {
            return None;
        }
        let screen = self.screens.iter().position(|s| s.bounds.contains(at.x + at.w / 2, at.y + at.h / 2))?;
        let b = Beside { screen, target, at, side, part: drag.keys.part.unwrap_or(Part::Half) };
        if let Some((_, found)) = drag.tried.filter(|(t, _)| *t == b) {
            return found.map(|view| (b, view));
        }
        let id = &self.screens[screen].id;
        let found = match drag.geometry.iter().position(|g| g.id == *id) {
            Some(s) => self.beside_split(drag, s, &b, pt).map(Some),
            None => self.adhoc_plan(drag.hwnd, &b).map(|_| None),
        };
        drag.tried = Some((b, found));
        found.map(|view| (b, view))
    }

    /// On a screen with zones: the view that splits the zones the target holds, or
    /// the zone under the pointer when it holds none. None when they can't be split.
    fn beside_split(&self, drag: &Drag, s: usize, b: &Beside, pt: POINT) -> Option<View> {
        let g = &drag.geometry[s];
        let held = self.zones_held(hwnd_of(b.target), &drag.geometry, Some(b.at), false);
        let (a, z) = match held.filter(|(id, ..)| *id == g.id) {
            Some((_, a, z)) => (a, z),
            None => zones::zone_at(&g.cells, pt.x, pt.y).map(|z| (z, z))?,
        };
        let cut = (b.part, b.side);
        self.split_plan(drag, s, (a, z), cut)?;
        Some(View { screen: s, hit: Some((a, z)), cut: Some(cut) })
    }

    /// On a screen with no zones: the target's zone in its group, or a new group of
    /// it alone, cut in two, with the dropped window in its part. None when either
    /// piece would be too small.
    pub(super) fn adhoc_plan(&self, dragged: isize, b: &Beside) -> Option<Plan> {
        let screen = self.screens.get(b.screen)?;
        let found = self.adhoc_of(b.target).filter(|(i, _)| self.adhoc[*i].screen == screen.id);
        let (group, area, gap, tree, members, leaf) = match found {
            Some((i, leaf)) => {
                let a = &self.adhoc[i];
                let members = a.members.iter().filter(|(k, _)| *k != dragged).copied().collect();
                (Some(i), a.area, a.gap, a.tree.clone(), members, leaf)
            }
            None => {
                let gap = self.gap_px(work_area(screen.bounds)?.1);
                (None, beside::area_around(b.at, gap), gap, beside::single(), vec![(b.target, 0)], 0)
            }
        };
        let (tree, at, rest) = beside::cut(&tree, area, leaf, b.side, b.part.share())?;
        let moved = |l: usize| match l {
            l if l == leaf => rest,
            l if l > leaf => l + 1,
            l => l,
        };
        let mut members: Vec<(isize, usize)> = members.into_iter().map(|(k, l)| (k, moved(l))).collect();
        members.push((dragged, at));
        let rects = zones::rects(&tree, area, gap);
        Some(Plan { group, area, gap, tree, members, rect: rects[at], target: rects[rest] })
    }

    /// The drop beside a window on a screen with no zones: the two share its place,
    /// in its group or a new one. A target that moved or went since the drag's look
    /// leaves the drop where it is.
    pub(super) fn beside_drop(&mut self, drag: &Drag, hwnd: HWND, b: Beside) {
        let target = hwnd_of(b.target);
        let there = unsafe { IsWindow(Some(target)) }.as_bool() && !winutil::is_minimized(target);
        let still = winutil::visible_rect(target).filter(|_| there).is_some_and(|r| close(r, b.at, true));
        self.unsnap(hwnd, drag.stamped);
        let plan = still.then(|| self.adhoc_plan(drag.hwnd, &b)).flatten();
        let Some(plan) = plan.filter(|_| wincontrol::movable(target, &self.exclude).is_ok()) else {
            log::info!("snap beside: the window it was dropped beside moved or can't be moved");
            return;
        };
        let screen = &self.screens[b.screen];
        let (id, dpi) = (screen.id.clone(), work_area(screen.bounds).map_or(96, |w| w.1));
        self.place(hwnd, plan.rect, winutil::monitor_dpi(hwnd) != dpi, false);
        self.queued.push_back(Placement { hwnd: b.target, target: plan.target, zone: None, dpi });
        self.window_pump();
        let n = plan.members.len();
        match plan.group.and_then(|i| self.adhoc.get_mut(i)) {
            Some(a) => (a.tree, a.members) = (plan.tree, plan.members),
            None => {
                self.adhoc_next += 1;
                let (area, gap, tree, members) = (plan.area, plan.gap, plan.tree, plan.members);
                self.adhoc.push(Adhoc { id: self.adhoc_next, screen: id, area, gap, tree, members });
            }
        }
        log::info!("snap beside: ad-hoc group on screen {}, {} {:?}, {n} windows", b.screen + 1, b.part.name(), b.side);
        self.sync_dividers();
    }

    /// Fit group to screen, or why not.
    fn fit(&mut self, hwnd: HWND) -> Result<Fitted, String> {
        if let Some(why) = &self.aside {
            return Err(why.clone());
        }
        let frame = wincontrol::movable(hwnd, &self.exclude)?;
        self.prune_adhoc(true);
        let key = hwnd.0 as isize;
        if let Some((i, _)) = self.adhoc_of(key) {
            return self.fit_adhoc(i);
        }
        // A screen's zones cover its work area already.
        if self.zone_here(hwnd, frame).is_some() {
            return Err(FILLS.into());
        }
        let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
        let number =
            self.screens.iter().position(|s| s.bounds.contains(x, y)).ok_or("That window isn't on a screen")?;
        let s = &self.screens[number];
        let (work, dpi) = work_area(s.bounds).ok_or("That window isn't on a screen")?;
        if close(frame, work, true) && !winutil::is_maximized(hwnd) {
            return Err(FILLS.into());
        }
        let screen = s.id.clone();
        let back = vec![Back { place: Placement { hwnd: key, target: frame, zone: None, dpi }, set: work }];
        self.queued.push_back(Placement { hwnd: key, target: work, zone: None, dpi });
        self.window_pump();
        log::info!("fit group: a loose window on screen {}", number + 1);
        Ok(Fitted { number: number + 1, screen, back, group: None })
    }

    /// An ad-hoc group's tree over the work area of its screen, so every window
    /// keeps its share and its place in the order.
    fn fit_adhoc(&mut self, i: usize) -> Result<Fitted, String> {
        let a = &self.adhoc[i];
        let gone = "That window's screen isn't connected";
        let number = self.screens.iter().position(|s| s.id == a.screen).ok_or(gone)?;
        let (work, dpi) = work_area(self.screens[number].bounds).ok_or(gone)?;
        if a.area == work {
            return Err(FILLS.into());
        }
        // One window that can't move stops it all, as with grow and shrink.
        for (k, _) in &a.members {
            wincontrol::movable(hwnd_of(*k), &self.exclude)?;
        }
        let (before, after) = (a.cells(), zones::rects(&a.tree, work, a.gap));
        let mut back = Vec::new();
        for &(k, leaf) in &a.members {
            let (Some(was), Some(set)) = (before.get(leaf).copied(), after.get(leaf).copied()) else { continue };
            self.queued.push_back(Placement { hwnd: k, target: set, zone: None, dpi });
            back.push(Back { place: Placement { hwnd: k, target: was, zone: None, dpi }, set });
        }
        let (group, screen, n) = (Some((a.id, a.area)), a.screen.clone(), a.members.len());
        self.adhoc[i].area = work;
        self.window_pump();
        self.sync_dividers();
        log::info!("fit group: an ad-hoc group of {n} windows on screen {}", number + 1);
        Ok(Fitted { number: number + 1, screen, back, group })
    }
}
