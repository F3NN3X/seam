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
use std::cell::RefCell;
use std::collections::HashSet;

/// An ad-hoc group's name where a screen's id goes: the divider handles and the
/// layout saves of their drags.
const PREFIX: &str = "adhoc:";
/// What a group, a window in zones or a window as big as its screen answers.
const FILLS: &str = "Already fills its screen";

/// What a drag needs to put a member back in its group: the group, where the
/// others were, and the members the group had once the window left.
pub(super) struct Left {
    group: Adhoc,
    rects: Vec<(isize, Rect)>,
    after: Option<Members>,
}

/// Each window of a group and its zone in the tree.
type Members = Vec<(isize, usize)>;

/// Windows put side by side on a screen with no zones.
#[derive(Clone)]
pub(super) struct Adhoc {
    pub(super) id: u64,
    /// The screen it's on.
    pub(super) screen: String,
    pub(super) area: Rect,
    /// The gap at that screen's scale when the group was made.
    pub(super) gap: i32,
    /// The screen's work area and DPI then. A group whose screen changed since ends.
    pub(super) work: Rect,
    pub(super) dpi: u32,
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

thread_local! {
    /// The groups' windows, for the hooks to pass over every other window without
    /// taking the engine. The hooks run on this thread.
    static MEMBERS: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
}

/// A member's state changed while a group is about: closed, minimized, maximized or moved.
unsafe extern "system" fn on_member(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    let key = hwnd.0 as isize;
    if object != OBJID_WINDOW.0 || child != 0 || key == 0 || !MEMBERS.with_borrow(|m| m.contains(&key)) {
        return;
    }
    with_engine(|e| e.member_changed(hwnd, event));
}

/// The timer repeats until the engine is free to take it, so a busy moment drops nothing.
unsafe extern "system" fn on_settled(_: HWND, _: u32, id: usize, _: u32) {
    let taken = with_engine(|e| {
        if e.adhoc_timer == id {
            e.adhoc_timer = 0;
            e.adhoc_settled();
        }
    });
    if taken.is_some() {
        unsafe {
            let _ = KillTimer(None, id);
        }
    }
}

/// A window's thread and process, or None when it is gone.
fn thread_of(key: isize) -> Option<(u32, u32)> {
    let mut pid = 0;
    let tid = unsafe { GetWindowThreadProcessId(hwnd_of(key), Some(&mut pid)) };
    (tid != 0).then_some((tid, pid))
}

/// How long a member has to stay put before Seam looks where it is.
const SETTLED_MS: u32 = 250;
/// A window this close to its new place isn't moved there.
const NEAR_PX: i32 = 2;

pub(super) fn near(a: Rect, b: Rect) -> bool {
    [(a.x, b.x), (a.y, b.y), (a.w, b.w), (a.h, b.h)].iter().all(|(p, q)| (p - q).abs() <= NEAR_PX)
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

    /// Group `i` once `key` has left it: its tree, area and members, renumbered. The
    /// zone beside the one it leaves takes its room, unless that leaves the area
    /// smaller. None when that leaves no group, as one window is none.
    fn closed_up(&self, i: usize, key: isize) -> Option<(Node, Rect, Members)> {
        let a = self.adhoc.get(i)?;
        let leaf = a.members.iter().find(|(k, _)| *k == key)?.1;
        let shift = |l: usize| if l > leaf { l - 1 } else { l };
        let members: Vec<(isize, usize)> =
            a.members.iter().filter(|(k, _)| *k != key).map(|&(k, l)| (k, shift(l))).collect();
        if members.len() < 2 {
            return None;
        }
        let (tree, area) = beside::remove(&a.tree, a.area, a.gap, leaf)?;
        Some((tree, area, members))
    }

    /// A member that a drag turned into a move leaves its group, keeping what puts
    /// it back, as a drag cancelled with Esc ends where it began.
    pub(super) fn leave_adhoc_dragged(&mut self, key: isize) -> Option<Left> {
        let (i, _) = self.adhoc_of(key)?;
        let group = self.adhoc[i].clone();
        let rects: Vec<(isize, Rect)> = group
            .members
            .iter()
            .filter(|(k, _)| *k != key)
            .filter_map(|&(k, _)| winutil::visible_rect(hwnd_of(k)).map(|r| (k, r)))
            .collect();
        self.leave_adhoc(key);
        let after = self.adhoc.iter().find(|a| a.id == group.id).map(|a| a.members.clone());
        Some(Left { group, rects, after })
    }

    /// The drag ended with the window back where it began: its group is put back,
    /// and the windows that closed up go back to their places, unless the group
    /// changed meanwhile. False when nothing was restored.
    pub(super) fn rejoin_adhoc(&mut self, key: isize, left: Left) -> bool {
        let now = self.adhoc.iter().find(|a| a.id == left.group.id).map(|a| &a.members);
        if now != left.after.as_ref() || self.adhoc_of(key).is_some() {
            return false;
        }
        let moves: Vec<Placement> = left
            .rects
            .iter()
            .filter_map(|&(k, r)| {
                let now = winutil::visible_rect(hwnd_of(k))?;
                (!near(now, r)).then_some(Placement {
                    hwnd: k,
                    target: r,
                    zone: None,
                    dpi: left.group.dpi,
                    maximize: false,
                })
            })
            .collect();
        log::info!("snap beside: a drag ended where it began, so the window is back in its group");
        self.adhoc.retain(|a| a.id != left.group.id);
        self.adhoc.push(left.group);
        self.queued.extend(moves);
        self.window_pump();
        self.sync_dividers();
        self.sync_adhoc_hooks();
        true
    }

    /// Take a window out of its ad-hoc group. The windows that close up move to
    /// their new places, and a group of one is no group.
    pub(super) fn leave_adhoc(&mut self, key: isize) {
        let Some((i, _)) = self.adhoc_of(key) else { return };
        let Some((tree, area, members)) = self.closed_up(i, key) else {
            log::info!("snap beside: a window left its group, which is no group now");
            self.adhoc.remove(i);
            self.sync_dividers();
            self.sync_adhoc_hooks();
            return;
        };
        let a = &self.adhoc[i];
        let after = zones::rects(&tree, area, a.gap);
        let mut moves = Vec::new();
        for &(k, l) in &members {
            // One that is going too isn't brought back to move it, and one already there stays.
            let h = hwnd_of(k);
            let away = winutil::is_minimized(h) || winutil::is_maximized(h);
            let now = winutil::visible_rect(h).filter(|_| !away);
            if let (Some(now), Some(set)) = (now, after.get(l).copied())
                && !near(now, set)
            {
                moves.push(Placement { hwnd: k, target: set, zone: None, dpi: a.dpi, maximize: false });
            }
        }
        log::info!("snap beside: a window left its group, {} stay", members.len());
        let a = &mut self.adhoc[i];
        (a.tree, a.area, a.members) = (tree, area, members);
        self.queued.extend(moves);
        self.window_pump();
        self.sync_dividers();
        self.sync_adhoc_hooks();
    }

    /// Hear of members closing, minimizing and moving while there is a group.
    /// Moves are hooked on the threads of the groups' windows only. Closing and
    /// minimizing have no thread to narrow to, so the callback filters them. Called whenever a group or Seam's standing changes.
    pub(super) fn sync_adhoc_hooks(&mut self) {
        let active = self.active();
        let members: Vec<isize> =
            self.adhoc.iter().flat_map(|a| a.members.iter().map(|(k, _)| *k)).filter(|_| active).collect();
        MEMBERS.with_borrow_mut(|m| *m = members.iter().copied().collect());
        self.sync_member_hook(!members.is_empty());
        let mut threads: Vec<(u32, u32)> = members.iter().filter_map(|k| thread_of(*k)).collect();
        threads.sort_unstable();
        threads.dedup();
        let (keep, gone): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.member_hooks).into_iter().partition(|(t, _)| threads.iter().any(|x| x.0 == *t));
        for hook in gone.into_iter().flat_map(|(_, hooks)| hooks) {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        self.member_hooks = keep;
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for (tid, pid) in threads {
            if self.member_hooks.iter().any(|(t, _)| *t == tid) {
                continue;
            }
            let event = EVENT_OBJECT_LOCATIONCHANGE;
            let hook = unsafe { SetWinEventHook(event, event, None, Some(on_member), pid, tid, flags) };
            if hook.0.is_null() {
                log::info!("snap beside: a group hook wasn't installed");
            }
            self.member_hooks.push((tid, Some(hook).filter(|h| !h.0.is_null()).into_iter().collect()));
        }
        if self.adhoc.is_empty() && self.adhoc_timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.adhoc_timer);
            }
            self.adhoc_timer = 0;
        }
    }

    /// One hook for every window's closing and minimizing, on while a group has
    /// members. Every window's destroy reaches the tray meanwhile, so the callback
    /// stays cheap: the object and child check first, then one hash lookup, and the
    /// engine is only taken for a member.
    fn sync_member_hook(&mut self, wanted: bool) {
        if !wanted {
            for hook in std::mem::take(&mut self.member_global) {
                let _ = unsafe { UnhookWinEvent(hook) };
            }
        } else if self.member_global.is_empty() {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            for event in [EVENT_SYSTEM_MINIMIZESTART, EVENT_OBJECT_DESTROY] {
                let hook = unsafe { SetWinEventHook(event, event, None, Some(on_member), 0, 0, flags) };
                if hook.0.is_null() {
                    log::info!("snap beside: a group hook wasn't installed");
                } else {
                    self.member_global.push(hook);
                }
            }
        }
    }

    /// The tray is quitting: the groups and their hooks go.
    pub(super) fn end_adhoc(&mut self) {
        self.adhoc.clear();
        self.sync_adhoc_hooks();
    }

    /// A member closed, minimized or maximized leaves its group at once. One that
    /// moved is looked at once it has stayed put a moment, since Seam's own moves of
    /// a group send the same event.
    fn member_changed(&mut self, hwnd: HWND, event: u32) {
        let key = hwnd.0 as isize;
        if self.adhoc_of(key).is_none() {
            return;
        }
        let gone = event == EVENT_OBJECT_DESTROY || event == EVENT_SYSTEM_MINIMIZESTART;
        if gone || winutil::is_minimized(hwnd) || winutil::is_maximized(hwnd) {
            self.leave_adhoc(key);
            return;
        }
        self.settle_later();
    }

    fn settle_later(&mut self) {
        unsafe {
            if self.adhoc_timer != 0 {
                let _ = KillTimer(None, self.adhoc_timer);
            }
            self.adhoc_timer = SetTimer(None, 0, SETTLED_MS, Some(on_settled));
        }
    }

    /// Members stayed put: the ones no longer where their group put them leave it.
    /// A drag of a window or a handle moves members on purpose, so it waits for that.
    fn adhoc_settled(&mut self) {
        if self.drag.is_some() || dividers::dragging() {
            self.settle_later();
            return;
        }
        self.prune_adhoc(true);
    }

    /// The screens or their work areas changed: a group on a screen that changed, or
    /// was given zones, ends where its windows are, with nothing moved.
    pub(super) fn adhoc_screens_changed(&mut self) {
        let before = self.adhoc.len();
        let screens = &self.screens;
        let same = |a: &Adhoc| {
            let s = screens.iter().find(|s| s.id == a.screen && s.layout.is_none());
            s.and_then(|s| work_area(s.bounds)) == Some((a.work, a.dpi))
        };
        self.adhoc.retain(same);
        if self.adhoc.len() < before {
            log::info!("snap beside: {} groups ended with a change of their screen", before - self.adhoc.len());
            self.sync_dividers();
        }
        self.sync_adhoc_hooks();
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
        let id = &self.screens[screen].id;
        let s = drag.geometry.iter().position(|g| g.id == *id);
        let under = s.and_then(|s| zones::zone_at(&drag.geometry[s].cells, pt.x, pt.y));
        if let Some((.., found)) = drag.tried.filter(|(t, u, _)| *t == b && *u == under) {
            return found.map(|view| (b, view));
        }
        let found = match s {
            Some(s) => self.beside_split(drag, s, &b, pt).map(Some),
            None => self.adhoc_plan(drag.hwnd, &b).map(|_| None),
        };
        drag.tried = Some((b, under, found));
        found.map(|view| (b, view))
    }

    /// On a screen with zones: the view that splits the zones the target holds, or
    /// the zone under the pointer when it holds none but mostly fits it. None when
    /// they can't be split, or a loose window lies over the zone, so the drop is the
    /// usual one.
    fn beside_split(&self, drag: &Drag, s: usize, b: &Beside, pt: POINT) -> Option<View> {
        let g = &drag.geometry[s];
        let held = self.zones_held(hwnd_of(b.target), &drag.geometry, Some(b.at), false);
        let (a, z) = match held.filter(|(id, ..)| *id == g.id) {
            Some((_, a, z)) => (a, z),
            None => {
                let z = zones::zone_at(&g.cells, pt.x, pt.y)?;
                swap::fits_mutually(b.at, g.zones[z]).then_some((z, z))?
            }
        };
        let cut = (b.part, b.side);
        self.split_plan(drag, s, (a, z), cut)?;
        Some(View { screen: s, hit: Some((a, z)), cut: Some(cut) })
    }

    /// On a screen with no zones: the target's zone in its group, or a new group of
    /// it alone, cut in two, with the dropped window in its part. A dragged member
    /// of the same group has left it first, as it will have at the drop. None when
    /// either piece would be too small.
    pub(super) fn adhoc_plan(&self, dragged: isize, b: &Beside) -> Option<Plan> {
        let screen = self.screens.get(b.screen)?;
        let found = self.adhoc_of(b.target).map(|(i, _)| i).filter(|i| self.adhoc[*i].screen == screen.id);
        let state = found.and_then(|i| {
            let a = &self.adhoc[i];
            if a.members.iter().any(|(k, _)| *k == dragged) {
                return self.closed_up(i, dragged).map(|(tree, area, members)| (i, area, a.gap, tree, members));
            }
            Some((i, a.area, a.gap, a.tree.clone(), a.members.clone()))
        });
        let (group, area, gap, tree, members) = match state {
            Some((i, area, gap, tree, members)) => (Some(i), area, gap, tree, members),
            None => {
                let gap = self.gap_px(work_area(screen.bounds)?.1);
                (None, beside::area_around(b.at, gap), gap, beside::single(), vec![(b.target, 0)])
            }
        };
        let leaf = members.iter().find(|(k, _)| *k == b.target)?.1;
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
        let Some((work, dpi)) = work_area(screen.bounds) else { return };
        let id = screen.id.clone();
        self.place(hwnd, plan.rect, winutil::monitor_dpi(hwnd) != dpi, false);
        self.queued.push_back(Placement { hwnd: b.target, target: plan.target, zone: None, dpi, maximize: false });
        self.window_pump();
        let n = plan.members.len();
        match plan.group.and_then(|i| self.adhoc.get_mut(i)) {
            Some(a) => (a.tree, a.members) = (plan.tree, plan.members),
            None => {
                self.adhoc_next += 1;
                let (area, gap, tree, members) = (plan.area, plan.gap, plan.tree, plan.members);
                self.adhoc.push(Adhoc { id: self.adhoc_next, screen: id, area, gap, work, dpi, tree, members });
            }
        }
        self.sync_adhoc_hooks();
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
        let place = Placement { hwnd: key, target: frame, zone: None, dpi, maximize: false };
        let back = vec![Back { place, set: work, maximized: winutil::is_maximized(hwnd), pulled: None }];
        self.queued.push_back(Placement { hwnd: key, target: work, zone: None, dpi, maximize: false });
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
        // No window goes below the smallest size its app allows, as Seam knows it now.
        for &(k, leaf) in &a.members {
            let (w, h) = minsize::cached(hwnd_of(k), dpi);
            if after.get(leaf).is_some_and(|c| c.w < w || c.h < h) {
                let name = winutil::app_name(hwnd_of(k)).unwrap_or_else(|| "A window".into());
                return Err(format!("{name} can't be made that small"));
            }
        }
        let mut back = Vec::new();
        for &(k, leaf) in &a.members {
            let (Some(was), Some(set)) = (before.get(leaf).copied(), after.get(leaf).copied()) else { continue };
            self.queued.push_back(Placement { hwnd: k, target: set, zone: None, dpi, maximize: false });
            let place = Placement { hwnd: k, target: was, zone: None, dpi, maximize: false };
            back.push(Back { place, set, maximized: false, pulled: None });
        }
        let (group, screen, n) = (Some((a.id, a.area)), a.screen.clone(), a.members.len());
        self.adhoc[i].area = work;
        self.window_pump();
        self.sync_dividers();
        log::info!("fit group: an ad-hoc group of {n} windows on screen {}", number + 1);
        Ok(Fitted { number: number + 1, screen, back, group })
    }
}
