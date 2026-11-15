//! Stacks: windows that share one zone, one of them on top. A child of `snap`, so it
//! works on the same engine. `seam::stack` keeps the order; this keeps each stack in
//! its zones by the snapping's records, stamps its windows, raises its top and asks the
//! overlay for its mark. Hiding is by z-order only: every window keeps the zone's
//! place, so Alt+Tab and the taskbar still reach each one, and the one that takes the
//! focus is the top. Hidden windows are moved once the moves of their top are over,
//! never during a drag. Only a cycle, or `window.stack`, puts a window at the very top;
//! otherwise the others are put just below their top, so no window a user is working
//! in is covered.

use super::*;
use crate::wincontrol::tile::Back;
use seam::stack::{self, Stacks};
use windows::core::BOOL;

/// The stacks changed, so the tray publishes its state, and offers Undo when a drop
/// made a stack (`take_joined`).
pub const WM_APP_STACKS: u32 = WM_APP + 36;
/// A worker listed the windows that carry a stack stamp, after a restart of the tray.
pub const WM_APP_STACKS_FOUND: u32 = WM_APP + 37;

/// What Undo of the last stack join puts back: the screen, and the window's old place.
static JOINED: Mutex<Option<(String, Back)>> = Mutex::new(None);
static FOUND: Mutex<Option<Vec<(isize, u64)>>> = Mutex::new(None);

/// A stack as the state shows it.
pub struct StackInfo {
    pub screen: String,
    pub from: usize,
    pub to: usize,
    /// In the order Next steps through them.
    pub windows: Vec<isize>,
    pub top: isize,
}

/// The next or previous window of `hwnd`'s stack goes on top and takes the focus.
/// `nudge` as `wincontrol::focus`. Returns the window, its place in the stack from 1, and how many.
pub fn cycle(hwnd: HWND, forward: bool, nudge: bool) -> Result<(isize, usize, usize), String> {
    let done = with_engine(|e| e.stack_cycle(hwnd.0 as isize, forward)).unwrap_or_else(|| Err(BUSY.into()))?;
    let h = hwnd_of(done.0);
    winutil::raise(h);
    wincontrol::focus(h, nudge);
    Ok(done)
}

/// `window.stack`: on stacks `hwnd` with the other windows in its zones, on top of
/// them; off takes it out of its stack. Returns how many windows its stack has now.
pub fn set(hwnd: HWND, on: bool) -> Result<usize, String> {
    let n = with_engine(|e| e.stack_set(hwnd.0 as isize, on)).unwrap_or_else(|| Err(BUSY.into()))?;
    // It is the top now, so it shows, as a cycle's does.
    if on {
        winutil::raise(hwnd);
    }
    Ok(n)
}

/// Every stack, for the state.
pub fn stacks() -> Vec<StackInfo> {
    with_engine(|e| e.stack_infos()).unwrap_or_default()
}

/// The screen and old place of the window the last drop stacked, once, for Undo.
pub fn take_joined() -> Option<(String, Back)> {
    JOINED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The tray's handler of `WM_APP_STACKS_FOUND`.
pub fn stacks_found() {
    let list = FOUND.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
    with_engine(|e| e.stacks_found(list));
}

/// A workspace switch put these windows in their zones as stacks: the screen and zones,
/// and the windows from the top.
/// With none, the switch may still have closed or minimized windows of a stack.
pub fn rebuild(list: Vec<((String, usize, usize), Vec<isize>)>) {
    with_engine(|e| {
        if !list.is_empty() {
            e.stacks_rebuild(list);
        }
        e.stacks_settle();
    });
}

const BUSY: &str = "Seam is busy, try again";

unsafe extern "system" fn on_focus(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object == OBJID_WINDOW.0 {
        with_engine(|e| e.stack_focus(hwnd.0 as isize));
    }
}

unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<(isize, u64)>);
        let bits = winutil::stack_stamp(h);
        if bits != 0 {
            list.push((h.0 as isize, bits));
        }
        BOOL(1)
    }
}

impl Engine {
    /// Write the stamps of stack `id`'s windows, and clear those of `out`. Each window's
    /// process and its start are kept as it joins, so a handle Windows gives to another
    /// window later isn't taken for it.
    pub(super) fn stack_stamps(&mut self, out: &[isize], id: Option<u32>) {
        for w in out {
            winutil::set_stack_stamp(hwnd_of(*w), 0);
            self.stack_owners.remove(w);
        }
        let Some(s) = id.and_then(|id| self.stacks.iter().find(|s| s.id == id)) else { return };
        for (i, m) in s.members.iter().enumerate() {
            winutil::set_stack_stamp(hwnd_of(*m), stack::stamp(s.id, i));
            self.stack_owners.entry(*m).or_insert_with(|| owner_of(*m));
        }
    }

    /// Whether `m` is still the window that joined: still stamped as in a stack, which a
    /// handle Windows gives to a new window isn't, and in the same process. Cheap, for
    /// every settle. `full` also reads when the process started, for a cycle, which is
    /// about to give the window the focus.
    fn same_window(&self, m: isize, full: bool) -> bool {
        let h = hwnd_of(m);
        let owner = self.stack_owners.get(&m);
        unsafe { IsWindow(Some(h)) }.as_bool()
            && winutil::stack_stamp(h) != 0
            && owner.is_none_or(|o| if full { *o == owner_of(m) } else { o.0 == winutil::process_id(h) })
    }

    /// `key` leaves its stack, if it is in one. A stack left with one window is gone.
    /// When `key` was on top and is still on screen, the next one goes just below it,
    /// never over it; when it closed or minimized, the highest one left is the top.
    pub(super) fn stack_leave(&mut self, key: isize) {
        let Some(s) = self.stacks.of(key) else { return };
        let (id, was_top) = (s.id, s.top() == key);
        let out = self.stacks.leave(key);
        self.stack_stamps(&out, Some(id));
        let rest = self.stacks.iter().find(|s| s.id == id).filter(|_| was_top).map(|s| (s.top(), s.members.clone()));
        if let Some((next, members)) = rest {
            let h = hwnd_of(key);
            if unsafe { IsWindow(Some(h)) }.as_bool() && !winutil::is_minimized(h) {
                winutil::place_below(hwnd_of(next), h);
            } else if let Some(t) = winutil::topmost_of(&members) {
                self.stacks.raise(t);
            }
        }
        self.stacks_changed();
    }

    /// A window of a stack closed or minimized, as the hook on every window's closing
    /// and minimizing saw: it leaves at once.
    pub(super) fn stack_gone(&mut self, key: isize) {
        if self.stacks.of(key).is_some() {
            self.stack_leave(key);
        }
    }

    /// The windows of `top`'s stack go just below it, so it shows and nothing else moves.
    fn stack_order(&self, top: isize) {
        let Some(s) = self.stacks.of(top) else { return };
        for m in s.members.iter().filter(|m| **m != top) {
            winutil::place_below(hwnd_of(*m), hwnd_of(top));
        }
    }

    /// Every stack's windows, by the same id, for the hook on closing and minimizing.
    pub(super) fn stack_members(&self) -> Vec<isize> {
        self.stacks.iter().flat_map(|s| s.members.iter().copied()).collect()
    }

    /// A window is about to be put in zones `zone`: it leaves its stack for other zones,
    /// unless its whole stack is being sent there with it.
    pub(super) fn stack_moving(&mut self, key: isize, (screen, from, to): (&str, usize, usize)) {
        if let Some(i) = self.carried.iter().position(|k| *k == key) {
            self.carried.swap_remove(i);
            return;
        }
        if self.stacks.of(key).is_none() {
            return;
        }
        let same = self.snapped.get(&key).is_some_and(|x| x.screen == screen && (x.from, x.to) == (from, to));
        if !same {
            self.stack_leave(key);
        }
    }

    /// The windows behind each stack's top.
    pub(super) fn stack_hidden(&self) -> Vec<isize> {
        let hidden = |s: &stack::Stack| s.members.iter().copied().filter(|m| *m != s.top()).collect::<Vec<_>>();
        self.stacks.iter().flat_map(hidden).collect()
    }

    /// What the snapping knows of each of a stack's windows, for `stack::settle`.
    fn seen(&self, s: &stack::Stack) -> Vec<stack::Seen<(String, usize, usize), Rect>> {
        let seen = |m: &isize| {
            let h = hwnd_of(*m);
            let record = self.snapped.get(m);
            stack::Seen {
                hwnd: *m,
                live: self.same_window(*m, false) && !winutil::is_minimized(h) && !winutil::is_maximized(h),
                in_place: self.current(h).is_some(),
                place: record.map(|x| ((x.screen.clone(), x.from, x.to), x.at)),
            }
        };
        s.members.iter().map(seen).collect()
    }

    /// Once the moves are over, `stack::settle` decides each stack: a window that closed,
    /// was minimized or maximized, isn't where it was put, or is in other zones leaves.
    /// Each hidden window in the same zones at another place goes where its top is, which
    /// is how an edge or a divider's drag resizes them, at its end. A stack whose top is
    /// on another desktop is left as it is. A carried stack's top shows again, and the
    /// marks follow.
    pub(super) fn stacks_settle(&mut self) {
        if self.stacks.is_empty() || self.pending.is_some() || !self.queued.is_empty() || self.drag.is_some() {
            return;
        }
        self.carried.clear();
        let (mut leaving, mut follow, mut tops) = (Vec::new(), Vec::new(), Vec::new());
        for s in self.stacks.iter().filter(|s| !winutil::is_cloaked(hwnd_of(s.top()))) {
            let decided = stack::settle(&self.seen(s), s.top);
            leaving.extend(decided.leaving);
            follow.extend(decided.follow);
            tops.extend(decided.top);
        }
        leaving.sort_unstable();
        leaving.dedup();
        for key in leaving {
            self.stack_leave(key);
        }
        for t in tops {
            self.stacks.raise(t);
        }
        let geometry = self.geometry();
        let place = |m: &isize| {
            let t = self.snapped.get(&self.stacks.of(*m)?.top())?;
            let dpi = geometry.iter().find(|g| g.id == t.screen)?.dpi;
            let zone = Some((t.screen.clone(), t.from, t.to));
            Some(Placement { hwnd: *m, target: t.at, zone, dpi, maximize: false })
        };
        let follow: Vec<Placement> = follow.iter().filter_map(place).collect();
        if !follow.is_empty() {
            log::info!("stacks: {} hidden windows follow their top", follow.len());
            self.queued.extend(follow);
            self.window_pump();
        }
        // A carried top shows once its windows have landed; until then it waits.
        if self.pending.is_none() && self.queued.is_empty() {
            for top in std::mem::take(&mut self.restack) {
                if self.stacks.of(top).is_some_and(|s| s.top() == top) {
                    winutil::raise(hwnd_of(top));
                    self.stack_order(top);
                }
            }
        }
        self.stack_marks();
    }

    /// The stacks changed: the focus hook, the marks and the tray's state follow.
    pub(super) fn stacks_changed(&mut self) {
        self.sync_stack_hook();
        self.sync_adhoc_hooks();
        self.stack_marks();
        // The handles are the top's, so they follow a new one.
        self.sync_dividers();
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_STACKS, WPARAM(0), LPARAM(0));
        }
    }

    fn sync_stack_hook(&mut self) {
        match (self.stacks.is_empty(), self.stack_hook) {
            (false, None) => unsafe {
                let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                let fg = EVENT_SYSTEM_FOREGROUND;
                let hook = SetWinEventHook(fg, fg, None, Some(on_focus), 0, 0, flags);
                if hook.0.is_null() {
                    log::info!("stacks: focus hook not installed");
                } else {
                    self.stack_hook = Some(hook);
                }
            },
            (true, Some(hook)) => {
                let _ = unsafe { UnhookWinEvent(hook) };
                self.stack_hook = None;
            }
            _ => {}
        }
    }

    /// A window took the focus: in a stack, it is that stack's top now, and the stacks
    /// are looked at, as a close or minimize of the top hands the focus on. The mark
    /// shows only for the window in front.
    fn stack_focus(&mut self, key: isize) {
        if self.stacks.raise(key) {
            self.sync_dividers();
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), WM_APP_STACKS, WPARAM(0), LPARAM(0));
            }
        }
        if self.stacks.of(key).is_some() {
            self.stacks_settle();
        }
        self.stack_marks();
    }

    /// The mark of the stack a window in front belongs to, on the current desktop, or
    /// none, and none while zones are off, Seam stands by or another window manager has
    /// the windows. The window in front has nothing over it but topmost windows, so the
    /// mark never sits over another window. Any window of the stack counts, so a cycle,
    /// which changes the top before the focus follows, redraws the mark in place rather
    /// than taking it down. Only a screen with zones has the overlay that draws it.
    pub(super) fn stack_marks(&mut self) {
        let front = unsafe { GetForegroundWindow() }.0 as isize;
        let mark = |s: &stack::Stack| {
            if s.position(front).is_none() || winutil::is_cloaked(hwnd_of(front)) {
                return None;
            }
            let t = self.snapped.get(&s.top())?;
            let screen = self.screens.iter().find(|x| x.id == t.screen && x.layout.is_some())?;
            let (_, dpi) = work_area(screen.bounds)?;
            let top = s.position(s.top())?;
            Some(zone_paint::Mark { zone: t.at, count: s.members.len(), top, scale: dpi as f32 / 96.0 })
        };
        let list: Vec<zone_paint::Mark> =
            if self.active() { self.stacks.iter().filter_map(mark).collect() } else { Vec::new() };
        if list != self.marks {
            self.marks = list.clone();
            zone_paint::marks(list);
        }
    }

    /// The windows a drop with the stack key on would stack onto in the zones `(a, b)`
    /// of the drag's screen `s`: the one the drag's look sees filling them. None when
    /// none does, which leaves the drop as it always was, swap and all.
    pub(super) fn stack_onto(&self, drag: &Drag, s: usize, (a, b): (usize, usize)) -> Option<HWND> {
        let g = &drag.geometry[s];
        let target = zones::span(*g.zones.get(a)?, *g.zones.get(b)?);
        let occupants = self.occupants(drag, g, target)?;
        let (h, _) = occupants.into_iter().find(|(h, r)| {
            // An ad-hoc group is on a screen with no zones, so never in a stack.
            let key = h.0 as isize;
            key != drag.hwnd && swap::fits_mutually(*r, target) && self.adhoc_of(key).is_none()
        })?;
        Some(h)
    }

    /// The drop put `hwnd` in the zones `onto` fills, with the stack key on: it goes on
    /// top of `onto`'s stack, or the two make one. `onto` is put in those zones too when
    /// the snapping has it anywhere else. Undo, from the notice, puts `hwnd` back where
    /// the drag began.
    pub(super) fn stack_join(&mut self, drag: &Drag, onto: HWND, g: &Placed, (a, b): (usize, usize), at: Rect) {
        let (key, under) = (drag.hwnd, onto.0 as isize);
        let held = self.snapped.get(&under).is_some_and(|x| x.screen == g.id && (x.from, x.to) == (a, b));
        if !held {
            let zone = Some((g.id.clone(), a, b));
            self.queued.push_back(Placement { hwnd: under, target: at, zone, dpi: g.dpi, maximize: false });
            self.carried.push(under);
            self.window_pump();
        }
        let (out, id) = self.stacks.join(under, key);
        self.stack_stamps(&out, Some(id));
        let size = self.stacks.of(key).map_or(0, |s| s.members.len());
        log::info!("stacks: a drop made a stack of {size} windows on screen {}", g.id);
        if let Some(frame) = drag.frame {
            let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
            let on = self.screens.iter().find(|on| on.bounds.contains(x, y));
            let dpi = on.and_then(|on| work_area(on.bounds)).map_or(g.dpi, |(_, dpi)| dpi);
            let place = Placement { hwnd: key, target: frame, zone: drag.old_zone.clone(), dpi, maximize: false };
            let back = Back { place, set: at, maximized: drag.was_maximized, pulled: None };
            *JOINED.lock().unwrap_or_else(|e| e.into_inner()) = Some((g.id.clone(), back));
        }
        self.stacks_changed();
    }

    /// A swap is about to send these windows to other zones: a stack's top takes its
    /// whole stack along, to the same place. Returns the hidden windows' moves.
    pub(super) fn stack_carry(&mut self, list: &[Placement]) -> Vec<Placement> {
        let mut more = Vec::new();
        for p in list.iter().filter(|p| p.zone.is_some()) {
            let Some(s) = self.stacks.of(p.hwnd) else { continue };
            // It lands in the dropped window's old zones, maybe over a stack left there: it
            // shows once every window has landed.
            self.restack.push(s.top());
            for m in s.members.clone() {
                self.carried.push(m);
                if m != p.hwnd {
                    more.push(Placement {
                        hwnd: m,
                        target: p.target,
                        zone: p.zone.clone(),
                        dpi: p.dpi,
                        maximize: false,
                    });
                }
            }
        }
        more
    }

    /// A re-fit is over: each stack's others go back below its top.
    pub(super) fn stack_raise_tops(&self) {
        for s in self.stacks.iter() {
            self.stack_order(s.top());
        }
    }

    /// Why stacks can't be used now, if they can't.
    fn stack_refusal(&self) -> Option<String> {
        if let Some(why) = &self.aside {
            return Some(why.clone());
        }
        (!self.enabled).then(|| "Zones are off".to_string())
    }

    fn stack_cycle(&mut self, key: isize, forward: bool) -> Result<(isize, usize, usize), String> {
        if let Some(why) = self.stack_refusal() {
            return Err(why);
        }
        self.stacks_settle();
        // About to take the focus: each window is checked in full first.
        let stale: Vec<isize> = self
            .stacks
            .of(key)
            .map(|s| s.members.iter().copied().filter(|m| !self.same_window(*m, true)).collect())
            .unwrap_or_default();
        for m in stale {
            self.stack_leave(m);
        }
        let new = self.stacks.cycle(key, forward).ok_or("That window isn't in a stack")?;
        let s = self.stacks.of(new).ok_or("That window isn't in a stack")?;
        let done = (new, s.position(new).unwrap_or(0) + 1, s.members.len());
        self.stacks_changed();
        Ok(done)
    }

    fn stack_set(&mut self, key: isize, on: bool) -> Result<usize, String> {
        if let Some(why) = self.stack_refusal() {
            return Err(why);
        }
        if !on {
            self.stacks.of(key).ok_or("That window isn't in a stack")?;
            self.stack_leave(key);
            return Ok(0);
        }
        let h = hwnd_of(key);
        let s = self.current(h).ok_or("That window isn't in a zone")?;
        let zone = (s.screen.clone(), s.from, s.to);
        // The other windows still where the snapping put them in the same zones, on this
        // desktop, in the order of their stack if they have one, then of their old stamp.
        let mut members: Vec<isize> = self.stacks.of(key).map(|s| s.members.clone()).unwrap_or_default();
        let shown = |h: HWND| unsafe { IsWindowVisible(h) }.as_bool() && !winutil::is_cloaked(h);
        let mut others: Vec<isize> = self
            .snapped
            .iter()
            .filter(|(k, x)| **k != key && (x.screen.clone(), x.from, x.to) == zone)
            .map(|(k, _)| *k)
            .filter(|k| !members.contains(k) && shown(hwnd_of(*k)) && self.current(hwnd_of(*k)).is_some())
            .collect();
        let was = |k: &isize| stack::unstamp(winutil::stack_stamp(hwnd_of(*k))).map_or(usize::MAX, |(_, pos)| pos);
        others.sort_unstable_by_key(|k| (was(k), *k));
        members.retain(|m| *m != key);
        members.extend(others);
        if members.is_empty() {
            return Err("No other window shares its zone".into());
        }
        members.push(key);
        let top = members.len() - 1;
        let (out, id) = self.stacks.make(members, top);
        self.stack_stamps(&out, id);
        self.stacks_changed();
        Ok(self.stacks.of(key).map_or(0, |s| s.members.len()))
    }

    fn stack_infos(&self) -> Vec<StackInfo> {
        let info = |s: &stack::Stack| {
            let t = self.snapped.get(&s.top())?;
            Some(StackInfo {
                screen: t.screen.clone(),
                from: t.from,
                to: t.to,
                windows: s.members.clone(),
                top: s.top(),
            })
        };
        self.stacks.iter().filter_map(info).collect()
    }

    /// After a restart of the tray, look once for the windows that carry a stack stamp.
    /// A worker lists them, topmost first.
    pub(super) fn stacks_look(&mut self) {
        if self.stacks_looked || !self.active() || self.screens.iter().all(|s| s.layout.is_none()) {
            return;
        }
        self.stacks_looked = true;
        let owner = self.owner;
        let spawned = std::thread::Builder::new().name("seam-stacks".into()).spawn(move || {
            let mut list: Vec<(isize, u64)> = Vec::new();
            unsafe {
                let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize));
            }
            *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(owner)), WM_APP_STACKS_FOUND, WPARAM(0), LPARAM(0));
            }
        });
        if spawned.is_err() {
            log::info!("stacks: couldn't start the look for stamps");
        }
    }

    /// The stamps the worker found make the stacks again. Each window's zones come from
    /// its zone stamp, as a re-fit reads them; one with no zones on its screen now leaves.
    fn stacks_found(&mut self, list: Vec<(isize, u64)>) {
        // A switch may have made a stack first: its ids must not meet the stamps' later.
        let last = list.iter().filter_map(|(_, bits)| stack::unstamp(*bits)).map(|(id, _)| id).max();
        self.stacks.seed(last.unwrap_or(0));
        if list.is_empty() || !self.stacks.is_empty() {
            return;
        }
        let geometry = self.geometry();
        let mut keep: Vec<(isize, u64)> = Vec::new();
        for (key, bits) in list {
            let h = hwnd_of(key);
            let set = winutil::zone_stamp(h);
            let at = winutil::visible_rect(h).filter(|_| unsafe { IsWindow(Some(h)) }.as_bool());
            let zoned = at.and_then(|r| {
                let g = geometry.iter().find(|g| g.bounds.contains(r.x + r.w / 2, r.y + r.h / 2))?;
                let (from, to) = (*set.iter().min()?, *set.iter().max()?);
                (to < g.zones.len()).then_some((g, from, to))
            });
            let Some((g, from, to)) = zoned else {
                winutil::set_stack_stamp(h, 0);
                continue;
            };
            let target = zones::span(g.zones[from], g.zones[to]);
            if !self.snapped.contains_key(&key) {
                self.keep_snapped(key, &g.id, (from, to), target);
            }
            keep.push((key, bits));
        }
        let mut rebuilt = Stacks::rebuild(&keep);
        rebuilt.seed(last.unwrap_or(0));
        self.stacks = rebuilt;
        let ids: Vec<u32> = self.stacks.iter().map(|s| s.id).collect();
        // A stamp whose stack came back with one window means nothing now.
        let alone: Vec<isize> = keep.iter().map(|(k, _)| *k).filter(|k| self.stacks.of(*k).is_none()).collect();
        self.stack_stamps(&alone, None);
        for id in ids {
            self.stack_stamps(&[], Some(id));
        }
        for top in self.stacks.iter().map(|s| s.top()).collect::<Vec<_>>() {
            self.stack_order(top);
        }
        log::info!("stacks: found {} again after a restart", self.stacks.iter().count());
        self.stacks_changed();
        self.stacks_settle();
    }

    fn stacks_rebuild(&mut self, list: Vec<((String, usize, usize), Vec<isize>)>) {
        if let Some(why) = self.stack_refusal() {
            log::info!("stacks: a switch's stacks left as windows, {why}");
            return;
        }
        let geometry = self.geometry();
        for ((screen, from, to), windows) in list {
            let Some(g) = geometry.iter().find(|g| g.id == screen && from <= to && to < g.zones.len()) else {
                continue;
            };
            let at = zones::span(g.zones[from], g.zones[to]);
            let windows: Vec<isize> =
                windows.into_iter().filter(|w| unsafe { IsWindow(Some(hwnd_of(*w))) }.as_bool()).collect();
            for w in &windows {
                self.keep_snapped(*w, &screen, (from, to), at);
                winutil::stamp_zones(hwnd_of(*w), &refit::covered(&g.cells, from, to));
            }
            let (out, id) = self.stacks.make(windows, 0);
            self.stack_stamps(&out, id);
            if let Some(top) = id.and_then(|id| self.stacks.iter().find(|s| s.id == id)).map(|s| s.top()) {
                self.stack_order(top);
            }
        }
        self.stacks_changed();
    }
}

/// A window's process and when it started, which a later window given the same handle doesn't share.
fn owner_of(m: isize) -> (u32, u64) {
    let pid = winutil::process_id(hwnd_of(m));
    (pid, seam::process::started(pid).unwrap_or(0))
}
