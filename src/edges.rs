//! Dragging an edge of a window in a zone: the windows in the zones on the other
//! side follow it live. It is a child of `snap`, so it works on the same engine
//! and drag. A worker finds the group when the drag begins, each 16 ms tick solves
//! the layout for the window's rect then, and the end fits the window and saves the
//! layout once. The dragged window itself is never moved while Windows has it (D74).

use super::*;
use crate::wincontrol;
use seam::groups::{self, Edge, Side};
use seam::zones::{Node, Template};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// A neighbour gets its next place once it has landed on the last, or after this long.
const PACE: Duration = Duration::from_millis(50);
/// Only posts the move: a slow app can't hold up the tray.
const FOLLOW: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(
    SWP_ASYNCWINDOWPOS.0 | SWP_NOACTIVATE.0 | SWP_NOZORDER.0 | SWP_NOCOPYBITS.0 | SWP_NOOWNERZORDER.0,
);

static NEXT: AtomicU64 = AtomicU64::new(1);
/// The worker's answer, for the drag with that number.
static ANSWER: Mutex<Option<(u64, Group)>> = Mutex::new(None);

/// A window in the zones beside the dragged one.
struct Member {
    hwnd: isize,
    from: usize,
    to: usize,
    /// Its window rect and visible frame when the drag began. They give the invisible
    /// border once, so a tick reads no frame, and the window rect is where Esc puts it.
    window: Rect,
    visible: Rect,
    /// Its zones then.
    cell: Rect,
    sized: bool,
    /// The window rect it was last sent, and when.
    sent: Rect,
    at: Option<Instant>,
    /// Hung during this drag, so it's left where it is.
    dropped: bool,
}

impl Member {
    /// The window rect that puts its visible frame on `frame`. Its own zones give back
    /// exactly where it was.
    fn rect_for(&self, frame: Rect) -> Rect {
        if frame == self.cell {
            self.window
        } else {
            zones::frame_adjust(frame, self.window, self.visible)
        }
    }

    fn put(&self, r: Rect) {
        let flags = if self.sized { FOLLOW } else { FOLLOW | SWP_NOSIZE };
        unsafe {
            let _ = SetWindowPos(hwnd_of(self.hwnd), None, r.x, r.y, r.w, r.h, flags);
        }
    }

    /// The latest place wins. It's sent once the last one landed, or after `PACE`.
    fn follow(&mut self, frame: Rect) {
        let want = self.rect_for(frame);
        if self.dropped || want == self.sent {
            return;
        }
        let h = hwnd_of(self.hwnd);
        let landed = winutil::window_rect(h).is_some_and(|r| close(r, self.sent, self.sized));
        if !landed && self.at.is_some_and(|at| at.elapsed() < PACE) {
            return;
        }
        if !unsafe { IsWindow(Some(h)) }.as_bool() || winutil::is_hung(h) {
            self.dropped = true;
            return;
        }
        self.put(want);
        (self.sent, self.at) = (want, Some(Instant::now()));
    }
}

/// What the worker found: the other windows in the group, and each leaf's minimum.
pub(super) struct Group {
    members: Vec<Member>,
    mins: Vec<(i32, i32)>,
}

impl Group {
    /// Esc, or a drag cut short: every neighbour that moved goes back.
    fn restore(&self) {
        for m in self.members.iter().filter(|m| !m.dropped && m.sent != m.window) {
            m.put(m.window);
        }
    }
}

/// A resize of a window in a group of zones.
pub(super) struct Edges {
    id: u64,
    screen: String,
    template: Template,
    /// The layout when the drag began. Every tick solves from it with the whole move
    /// since then, so nothing drifts.
    tree: Node,
    area: Rect,
    gap: i32,
    /// The dragged window's zones.
    from: usize,
    to: usize,
    /// The divider along its left, right, top and bottom, or None at the screen's edge.
    sides: [Option<Edge>; 4],
    /// None until the worker answers. Until then the drag is a hand resize.
    group: Option<Group>,
    /// The window rect the last tick solved for, and the layout it gave.
    solved: Option<(Rect, Option<Node>)>,
    ticks: u32,
    longest: Duration,
}

impl Edges {
    /// The layout for the dragged window at `now`: each side that moved since `start`
    /// moves its divider, side to side first. None when no side with a divider moved.
    fn solve(&self, start: Rect, now: Rect, mins: &[(i32, i32)]) -> Option<Node> {
        let moved = [
            now.x - start.x,
            now.x + now.w - (start.x + start.w),
            now.y - start.y,
            now.y + now.h - (start.y + start.h),
        ];
        let mut tree: Option<Node> = None;
        for (edge, delta) in self.sides.iter().zip(moved) {
            let Some(edge) = edge.as_ref().filter(|_| delta != 0) else { continue };
            let from = tree.as_ref().unwrap_or(&self.tree);
            tree = Some(groups::solve(from, self.area, edge, delta, mins).tree);
        }
        tree
    }

    /// Once the group is known, solve for the window's rect now and send each
    /// neighbour its place. Cheap calls only: no frame reads and no SendMessage.
    pub(super) fn tick(&mut self, hwnd: HWND, start: Rect) {
        let started = Instant::now();
        if self.group.is_none() {
            let mut answer = ANSWER.lock().unwrap_or_else(|e| e.into_inner());
            if answer.as_ref().is_some_and(|(id, _)| *id == self.id) {
                self.group = answer.take().map(|(_, group)| group);
            }
        }
        let Some(mins) = self.group.as_ref().map(|g| &g.mins) else { return };
        let Some(now) = winutil::window_rect(hwnd) else { return };
        if self.solved.as_ref().is_none_or(|(at, _)| *at != now) {
            self.solved = Some((now, self.solve(start, now, mins)));
        }
        let tree = self.solved.as_ref().and_then(|(_, t)| t.as_ref()).unwrap_or(&self.tree);
        let placed = zones::rects(tree, self.area, self.gap);
        if let Some(group) = self.group.as_mut() {
            for m in &mut group.members {
                m.follow(zones::span(placed[m.from], placed[m.to]));
            }
        }
        self.ticks += 1;
        self.longest = self.longest.max(started.elapsed());
    }
}

/// What the worker needs to find the group.
struct Ask {
    id: u64,
    hwnd: isize,
    from: usize,
    to: usize,
    screen: String,
    bounds: Rect,
    /// The screen's zones with the gap.
    placed: Vec<Rect>,
    tree: Node,
    work: Rect,
    gap: i32,
    dpi: u32,
    known: Vec<Zoned>,
    exclude: Vec<String>,
}

impl Ask {
    /// The members, as grow and shrink find them, and the minimums. A neighbour that
    /// is hung or runs as administrator can't follow, so it stays out.
    fn run(self) {
        let first = (hwnd_of(self.hwnd), self.from, self.to);
        let list =
            wincontrol::group_members(first, &self.screen, self.bounds, &self.placed, &self.known, &self.exclude);
        let mins = wincontrol::group_mins(&self.tree, self.work, self.gap, self.dpi, &list);
        let member = |(h, from, to): (HWND, usize, usize)| {
            if winutil::is_hung(h) || winutil::is_elevated_over_us(h) {
                return None;
            }
            let (window, visible) = (winutil::window_rect(h)?, winutil::visible_rect(h)?);
            let cell = zones::span(self.placed[from], self.placed[to]);
            let sized = winutil::is_resizable(h);
            Some(Member {
                hwnd: h.0 as isize,
                from,
                to,
                window,
                visible,
                cell,
                sized,
                sent: window,
                at: None,
                dropped: false,
            })
        };
        let members = list.into_iter().skip(1).filter_map(member).collect();
        let mut answer = ANSWER.lock().unwrap_or_else(|e| e.into_inner());
        // A slow worker of an earlier drag mustn't replace the answer for a newer one.
        if answer.as_ref().is_none_or(|(id, _)| *id < self.id) {
            *answer = Some((self.id, Group { members, mins }));
        }
    }
}

impl Engine {
    /// A drag began on a window in its zones, with "Resize neighbours with a window"
    /// on: ask a worker for its group. `zone` is the window's own record when it is
    /// still where it was put, and `stamped` whether it carries a stamp; the drag
    /// read both already, so a plain move costs little more here than the worker.
    pub(super) fn edges_start(
        &self,
        hwnd: HWND,
        geometry: &[Placed],
        (zone, stamped): (Option<(String, usize, usize)>, bool),
        was_maximized: bool,
    ) -> Option<Edges> {
        if !self.settings.resize_neighbours || was_maximized || (zone.is_none() && !stamped) {
            return None;
        }
        if !winutil::is_resizable(hwnd) {
            return None;
        }
        let (s, from, to) = match zone {
            Some((screen, from, to)) => (geometry.iter().position(|g| g.id == screen)?, from, to),
            None => {
                // After a restart of the tray only the stamp on the window says.
                let r = winutil::visible_rect(hwnd)?;
                let s = geometry.iter().position(|g| g.bounds.contains(r.x + r.w / 2, r.y + r.h / 2))?;
                let (from, to, _) = wincontrol::stamped_in(hwnd, r, &geometry[s].zones)?;
                (s, from, to)
            }
        };
        let g = &geometry[s];
        let layout = self.screens.iter().find(|x| x.id == g.id)?.layout.clone()?;
        if from > to || to >= g.zones.len() {
            return None;
        }
        let leaves = refit::covered(&g.cells, from, to);
        let sides = [Side::Left, Side::Right, Side::Top, Side::Bottom]
            .map(|side| groups::edge_of(&layout.tree, g.work, &leaves, side));
        // A window alone on its screen has no neighbours to follow it.
        if sides.iter().all(Option::is_none) {
            return None;
        }
        let gap = (self.settings.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let ask = Ask {
            id,
            hwnd: hwnd.0 as isize,
            from,
            to,
            screen: g.id.clone(),
            bounds: g.bounds,
            placed: g.zones.clone(),
            tree: layout.tree.clone(),
            work: g.work,
            gap,
            dpi: g.dpi,
            known: self.zoned(),
            exclude: self.exclude.clone(),
        };
        if std::thread::Builder::new().name("seam-edges".into()).spawn(move || ask.run()).is_err() {
            log::info!("zone edges: couldn't start the worker");
            return None;
        }
        Some(Edges {
            id,
            screen: g.id.clone(),
            template: layout.template,
            tree: layout.tree,
            area: g.work,
            gap,
            from,
            to,
            sides,
            group: None,
            solved: None,
            ticks: 0,
            longest: Duration::ZERO,
        })
    }

    /// The resize is over, or `ended` is false because it was cut short while Windows
    /// still has the window. Esc or cut short: the neighbours go back and nothing is
    /// saved, and a window cut short is resized by hand from then on. Otherwise the
    /// window is fitted to its zones in the solved layout, the neighbours get their
    /// last places, and the layout is saved once. Returns false for a drag that is a
    /// hand resize as before.
    pub(super) fn edges_end(&mut self, drag: &Drag, hwnd: HWND, ended: bool) -> bool {
        let Some(edges) = drag.edges.as_ref() else { return false };
        let Some(group) = edges.group.as_ref() else {
            log::info!("zone edges: the resize ended before its group was found, so it's a hand resize");
            return false;
        };
        if edges.ticks > 0 {
            let us = edges.longest.as_secs_f64() * 1e6;
            log::info!("zone edges: {} neighbours, {} ticks, main max {us:.0}us", group.members.len(), edges.ticks);
        }
        if !ended {
            group.restore();
            self.unsnap(hwnd, drag.stamped);
            return true;
        }
        let now = winutil::window_rect(hwnd);
        let tree = now.filter(|r| *r != drag.start).and_then(|r| edges.solve(drag.start, r, &group.mins));
        let Some(tree) = tree else {
            group.restore();
            // Esc puts the window back, and it stays snapped. A side along the screen's
            // edge has no divider, so that's a hand resize.
            return now == Some(drag.start);
        };
        let placed = zones::rects(&tree, edges.area, edges.gap);
        let own = std::process::id();
        // A neighbour closed or minimized since the drag began is left out.
        let here = |m: &&Member| {
            let h = hwnd_of(m.hwnd);
            !m.dropped && unsafe { IsWindow(Some(h)) }.as_bool() && winutil::is_snappable(h, own, &self.exclude)
        };
        let members: Vec<&Member> = group.members.iter().filter(here).collect();
        for m in members {
            let at = zones::span(placed[m.from], placed[m.to]);
            let r = m.rect_for(at);
            if r != m.window || m.sent != m.window {
                m.put(r);
            }
            self.keep_snapped(m.hwnd, &edges.screen, (m.from, m.to), at);
        }
        // The usual placement, which also takes back any overshoot past a neighbour's minimum.
        let at = zones::span(placed[edges.from], placed[edges.to]);
        self.place(hwnd, at, false, false);
        self.keep_snapped(drag.hwnd, &edges.screen, (edges.from, edges.to), at);
        if tree != edges.tree {
            wincontrol::save_layout(self.owner, edges.screen.clone(), groups::relabel(edges.template, tree));
        }
        true
    }

    /// The window is in the same zones, at a new place. Its size from before it was
    /// snapped stays what it was.
    fn keep_snapped(&mut self, key: isize, screen: &str, (from, to): (usize, usize), at: Rect) {
        let entry = self.snapped.entry(key).or_insert(Snapped { old: None, screen: screen.to_string(), from, to, at });
        (entry.screen, entry.from, entry.to, entry.at) = (screen.to_string(), from, to, at);
    }
}
