//! Stacks: several windows in one zone, one of them on top and the rest behind it.
//! Only the order lives here, with no Win32 in it. The snapping keeps which zone a
//! stack is in, raises its top, and stamps each window with its stack and place in
//! it (`Seam_stack`), so a restart of the tray finds the stacks again.

/// The windows of one stack, in the order Next steps through them, and which is on top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub id: u32,
    pub members: Vec<isize>,
    /// Index into `members`.
    pub top: usize,
}

impl Stack {
    pub fn top(&self) -> isize {
        self.members[self.top.min(self.members.len() - 1)]
    }

    pub fn position(&self, h: isize) -> Option<usize> {
        self.members.iter().position(|m| *m == h)
    }

    /// `h` is on top now, as when it took the focus. False when it isn't in this stack.
    pub fn raise(&mut self, h: isize) -> bool {
        let Some(i) = self.position(h) else { return false };
        self.top = i;
        true
    }

    /// The next member goes on top, or the one before it, wrapping round.
    pub fn cycle(&mut self, forward: bool) -> isize {
        let n = self.members.len();
        self.top = if forward { (self.top + 1) % n } else { (self.top + n - 1) % n };
        self.top()
    }

    /// `h` joins at the end of the order, on top.
    fn add(&mut self, h: isize) {
        if !self.raise(h) {
            self.members.push(h);
            self.top = self.members.len() - 1;
        }
    }

    /// `h` leaves. When it was on top, the one before it in the order is on top now.
    fn remove(&mut self, h: isize) -> bool {
        let Some(i) = self.position(h) else { return false };
        self.members.remove(i);
        let n = self.members.len().max(1);
        self.top = match i {
            _ if i < self.top => self.top - 1,
            _ if i == self.top => (i + n - 1) % n,
            _ => self.top,
        };
        true
    }
}

/// Every stack on the desk. A stack always has at least two windows: one left alone
/// isn't a stack any more.
#[derive(Clone, Debug, Default)]
pub struct Stacks {
    list: Vec<Stack>,
    /// The last id given, so a new stack never takes one a stamp still carries.
    last: u32,
}

impl Stacks {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Stack> {
        self.list.iter()
    }

    pub fn of(&self, h: isize) -> Option<&Stack> {
        self.list.iter().find(|s| s.position(h).is_some())
    }

    fn of_mut(&mut self, h: isize) -> Option<&mut Stack> {
        self.list.iter_mut().find(|s| s.position(h).is_some())
    }

    fn next_id(&mut self) -> u32 {
        self.last = self.last.wrapping_add(1).max(1);
        self.last
    }

    /// `h` goes on top of the stack `onto` is in, or the two make a new one. `h` leaves
    /// any stack it was in first. Returns the windows that are in no stack any more, as
    /// `leave` does, and the stack's id.
    pub fn join(&mut self, onto: isize, h: isize) -> (Vec<isize>, u32) {
        if onto == h {
            return (Vec::new(), self.of(h).map_or(0, |s| s.id));
        }
        let mut out = if self.of(onto).is_some_and(|s| s.position(h).is_some()) { Vec::new() } else { self.leave(h) };
        out.retain(|w| *w != h && *w != onto);
        if let Some(s) = self.of_mut(onto) {
            s.add(h);
            return (out, s.id);
        }
        let id = self.next_id();
        self.list.push(Stack { id, members: vec![onto, h], top: 1 });
        (out, id)
    }

    /// A new stack of `members` in that order, with `top` on top. Each leaves the stack
    /// it was in first. None for fewer than two windows. Returns the windows in no stack
    /// any more, as `leave` does.
    pub fn make(&mut self, members: Vec<isize>, top: usize) -> (Vec<isize>, Option<u32>) {
        let mut members = members;
        let mut seen = Vec::new();
        members.retain(|m| {
            let new = !seen.contains(m);
            seen.push(*m);
            new
        });
        let mut out = Vec::new();
        for m in &members {
            out.extend(self.leave(*m));
        }
        if members.len() < 2 {
            return (out, None);
        }
        out.retain(|w| !members.contains(w));
        let id = self.next_id();
        let top = top.min(members.len() - 1);
        self.list.push(Stack { id, members, top });
        (out, Some(id))
    }

    /// `h` leaves its stack. Returns the windows that are in no stack any more: `h`, and
    /// the last one of a stack it leaves alone. Empty when `h` was in none.
    pub fn leave(&mut self, h: isize) -> Vec<isize> {
        let Some(i) = self.list.iter().position(|s| s.position(h).is_some()) else { return Vec::new() };
        self.list[i].remove(h);
        let mut out = vec![h];
        if self.list[i].members.len() < 2 {
            out.extend(self.list.remove(i).members);
        }
        out
    }

    /// Stamps carry ids up to `id`, so a new stack takes a later one.
    pub fn seed(&mut self, id: u32) {
        self.last = self.last.max(id);
    }

    /// `h` took the focus: it is its stack's top now. False when it is in none, or on top already.
    pub fn raise(&mut self, h: isize) -> bool {
        match self.of_mut(h) {
            Some(s) if s.top() != h => s.raise(h),
            _ => false,
        }
    }

    /// The next or previous window of `h`'s stack goes on top. None when `h` is in none.
    pub fn cycle(&mut self, h: isize, forward: bool) -> Option<isize> {
        self.of_mut(h).map(|s| s.cycle(forward))
    }

    /// The stacks the stamps on the windows describe after a restart. `found` is each
    /// stamped window with its stamp's bits, topmost first, so the topmost of a stack is
    /// its top. A stack the stamps leave with one window is none.
    pub fn rebuild(found: &[(isize, u64)]) -> Stacks {
        let mut placed: Vec<(u32, usize, isize)> =
            found.iter().filter_map(|(h, bits)| unstamp(*bits).map(|(id, pos)| (id, pos, *h))).collect();
        let tops: Vec<(u32, isize)> = {
            let mut tops: Vec<(u32, isize)> = Vec::new();
            for (id, _, h) in &placed {
                if !tops.iter().any(|(t, _)| t == id) {
                    tops.push((*id, *h));
                }
            }
            tops
        };
        placed.sort_by_key(|(id, pos, _)| (*id, *pos));
        let mut stacks = Stacks { list: Vec::new(), last: placed.iter().map(|(id, ..)| *id).max().unwrap_or(0) };
        for (id, top) in tops {
            let members: Vec<isize> = placed.iter().filter(|(i, ..)| *i == id).map(|(.., h)| *h).collect();
            if members.len() < 2 {
                continue;
            }
            let top = members.iter().position(|m| *m == top).unwrap_or(0);
            stacks.list.push(Stack { id, members, top });
        }
        stacks
    }
}

/// What the snapping knows of one window of a stack, for `settle`. `Z` is its zones
/// and `A` its place in them.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen<Z, A> {
    pub hwnd: isize,
    /// Still the window that joined, stamped as in a stack, and neither minimized nor maximized.
    pub live: bool,
    /// Still where the snapping put it.
    pub in_place: bool,
    /// The zones and place the snapping has it in, or None.
    pub place: Option<(Z, A)>,
}

/// What `settle` decided for one stack.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settle {
    pub leaving: Vec<isize>,
    /// Windows that stay but aren't where the top is: they go there.
    pub follow: Vec<isize>,
    /// The window that says where the stack is, its top from now on. None when every window leaves.
    pub top: Option<isize>,
}

/// Once the moves are over, for one stack's windows in their order with `top` on top:
/// the first one from the top that is live and where it was put says where the stack
/// is. A window that isn't live, isn't where it was put, or is in other zones leaves.
/// One in the same zones at another place follows it there.
pub fn settle<Z: PartialEq, A: PartialEq>(members: &[Seen<Z, A>], top: usize) -> Settle {
    let n = members.len();
    let from_top = (0..n).map(|i| &members[(top + i) % n]);
    let anchor = from_top.filter(|m| m.live && m.in_place).find_map(|m| Some((m.hwnd, m.place.as_ref()?)));
    let Some((anchor, (zone, at))) = anchor else {
        return Settle { leaving: members.iter().map(|m| m.hwnd).collect(), ..Settle::default() };
    };
    let stays = |m: &Seen<Z, A>| m.live && m.in_place && m.place.as_ref().is_some_and(|(z, _)| z == zone);
    let leaving = members.iter().filter(|m| !stays(m)).map(|m| m.hwnd).collect();
    let elsewhere = |m: &&Seen<Z, A>| m.hwnd != anchor && m.place.as_ref().is_some_and(|(_, a)| a != at);
    let follow = members.iter().filter(|m| stays(m)).filter(elsewhere).map(|m| m.hwnd).collect();
    Settle { leaving, follow, top: Some(anchor) }
}

/// The `Seam_stack` bits for a window at `position` in stack `id`. Never 0, which is no stamp.
pub fn stamp(id: u32, position: usize) -> u64 {
    (u64::from(id) << 16) | (position.min(0xfffe) as u64 + 1)
}

/// The stack and position a stamp's bits say, or None for no stamp.
pub fn unstamp(bits: u64) -> Option<(u32, usize)> {
    let pos = (bits & 0xffff) as usize;
    (pos != 0).then(|| ((bits >> 16) as u32, pos - 1))
}

/// For a capture: the stack each window's stamp names, topmost first, turned into each
/// window's place in its stack from the top, 0 being the top. A stack only one window
/// of it was captured from has none.
pub fn ranks(ids: &[Option<u32>]) -> Vec<Option<u32>> {
    let count = |id: u32| ids.iter().filter(|x| **x == Some(id)).count();
    ids.iter()
        .enumerate()
        .map(|(i, id)| {
            let id = (*id).filter(|id| count(*id) >= 2)?;
            Some(ids[..i].iter().filter(|x| **x == Some(id)).count() as u32)
        })
        .collect()
}

/// For a switch: the rows that make a stack, from each row's zone and place in its
/// stack (`Rule.stack`). Each group has at least two rows, ordered from the top.
pub fn groups<K: PartialEq>(rows: &[Option<(K, u32)>]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let Some((key, _)) = row else { continue };
        let same = |j: &usize| rows[*j].as_ref().is_some_and(|(k, _)| k == key);
        match out.iter_mut().find(|g| g.first().is_some_and(same)) {
            Some(group) => group.push(i),
            None => out.push(vec![i]),
        }
    }
    out.retain(|g| g.len() >= 2);
    for group in &mut out {
        group.sort_by_key(|i| rows[*i].as_ref().map_or(u32::MAX, |(_, rank)| *rank));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_makes_a_stack_with_the_dropped_window_on_top() {
        let mut s = Stacks::default();
        let (out, id) = s.join(1, 2);
        assert!(out.is_empty());
        let stack = s.of(1).unwrap();
        assert_eq!((stack.id, stack.members.clone(), stack.top()), (id, vec![1, 2], 2));
        // A third goes on top of the same stack.
        assert_eq!(s.join(2, 3).1, id);
        assert_eq!(s.of(3).unwrap().top(), 3);
        assert_eq!(s.iter().count(), 1);
    }

    #[test]
    fn cycling_wraps_both_ways() {
        let mut s = Stacks::default();
        s.make(vec![1, 2, 3], 0);
        assert_eq!(s.cycle(1, true), Some(2));
        assert_eq!(s.cycle(1, true), Some(3));
        assert_eq!(s.cycle(1, true), Some(1));
        assert_eq!(s.cycle(1, false), Some(3));
        assert_eq!(s.cycle(9, true), None);
    }

    #[test]
    fn the_focus_moves_the_top() {
        let mut s = Stacks::default();
        s.make(vec![1, 2, 3], 0);
        assert!(s.raise(3));
        assert!(!s.raise(3));
        assert!(!s.raise(9));
        assert_eq!(s.of(1).unwrap().top(), 3);
    }

    #[test]
    fn leaving_the_top_puts_the_one_before_it_on_top() {
        let mut s = Stacks::default();
        s.make(vec![1, 2, 3], 1);
        assert_eq!(s.leave(2), vec![2]);
        assert_eq!(s.of(1).unwrap().top(), 1);
        // Below the top, the top stays the same window.
        let mut s = Stacks::default();
        s.make(vec![1, 2, 3], 2);
        s.leave(1);
        assert_eq!(s.of(2).unwrap().top(), 3);
        // The first one leaving from the top wraps to the last.
        let mut s = Stacks::default();
        s.make(vec![1, 2, 3], 0);
        s.leave(1);
        assert_eq!(s.of(2).unwrap().top(), 3);
    }

    #[test]
    fn a_stack_left_with_one_window_is_gone() {
        let mut s = Stacks::default();
        s.join(1, 2);
        let mut out = s.leave(2);
        out.sort();
        assert_eq!(out, vec![1, 2]);
        assert!(s.is_empty());
        assert!(s.leave(1).is_empty());
    }

    #[test]
    fn joining_another_stack_leaves_the_old_one() {
        let mut s = Stacks::default();
        s.make(vec![1, 2], 0);
        s.make(vec![3, 4], 0);
        // 2 leaves 1 alone, so 1 is out of every stack.
        let (out, _) = s.join(3, 2);
        assert_eq!(out, vec![1]);
        assert!(s.of(1).is_none());
        assert_eq!(s.of(3).unwrap().members, vec![3, 4, 2]);
        // Joining its own stack again only raises it.
        let (out, _) = s.join(4, 3);
        assert!(out.is_empty());
        assert_eq!(s.of(3).unwrap().top(), 3);
        assert_eq!(s.join(3, 3).0, Vec::<isize>::new());
    }

    #[test]
    fn make_takes_its_windows_from_their_stacks() {
        let mut s = Stacks::default();
        s.make(vec![1, 2], 0);
        let (out, id) = s.make(vec![2, 3, 3], 5);
        assert_eq!(out, vec![1]);
        let stack = s.of(2).unwrap();
        assert_eq!((Some(stack.id), stack.members.clone(), stack.top()), (id, vec![2, 3], 3));
        assert_eq!(s.make(vec![7], 0).1, None);
    }

    #[test]
    fn stamps_round_trip() {
        assert_eq!(unstamp(stamp(7, 0)), Some((7, 0)));
        assert_eq!(unstamp(stamp(u32::MAX, 3)), Some((u32::MAX, 3)));
        assert_eq!(unstamp(0), None);
        assert_ne!(stamp(0, 0), 0);
    }

    #[test]
    fn a_restart_finds_the_stacks_and_their_tops() {
        // Topmost first: 30 is the top of stack 4, which is in order 10, 20, 30.
        let found = [(30, stamp(4, 2)), (10, stamp(4, 0)), (20, stamp(4, 1)), (50, stamp(9, 0)), (60, 0)];
        let mut s = Stacks::rebuild(&found);
        let stack = s.of(10).unwrap();
        assert_eq!((stack.id, stack.members.clone(), stack.top()), (4, vec![10, 20, 30], 30));
        // A stamp alone in its stack makes none.
        assert!(s.of(50).is_none());
        // A new stack doesn't take an id a stamp still has.
        assert!(s.join(50, 60).1 > 9);
    }

    #[test]
    fn a_capture_ranks_each_stack_from_its_top() {
        let ranked = ranks(&[Some(4), None, Some(9), Some(4), Some(4)]);
        assert_eq!(ranked, vec![Some(0), None, None, Some(1), Some(2)]);
    }

    fn seen(hwnd: isize, live: bool, in_place: bool, place: Option<(u8, u8)>) -> Seen<u8, u8> {
        Seen { hwnd, live, in_place, place }
    }

    #[test]
    fn settling_keeps_a_stack_that_is_where_it_was() {
        let members = [seen(1, true, true, Some((1, 5))), seen(2, true, true, Some((1, 5)))];
        assert_eq!(settle(&members, 1), Settle { top: Some(2), ..Settle::default() });
    }

    #[test]
    fn a_top_moved_away_leaves_and_the_next_says_where_the_stack_is() {
        let members =
            [seen(1, true, true, Some((1, 5))), seen(2, true, false, Some((1, 5))), seen(3, true, true, Some((1, 5)))];
        let s = settle(&members, 1);
        assert_eq!((s.leaving, s.top), (vec![2], Some(3)));
    }

    #[test]
    fn a_closed_or_reused_window_leaves() {
        // A stale stamp or another process behind the same handle isn't live.
        let members = [seen(1, false, true, Some((1, 5))), seen(2, true, true, Some((1, 5)))];
        assert_eq!(settle(&members, 1).leaving, vec![1]);
    }

    #[test]
    fn a_hidden_window_in_other_zones_leaves_and_one_elsewhere_in_them_follows() {
        let members =
            [seen(1, true, true, Some((1, 5))), seen(2, true, true, Some((2, 5))), seen(3, true, true, Some((1, 4)))];
        let s = settle(&members, 0);
        assert_eq!((s.leaving, s.follow, s.top), (vec![2], vec![3], Some(1)));
    }

    #[test]
    fn a_stack_with_nothing_in_place_leaves_whole() {
        let members =
            [seen(1, true, false, Some((1, 5))), seen(2, false, true, Some((1, 5))), seen(3, true, true, None)];
        assert_eq!(settle(&members, 0), Settle { leaving: vec![1, 2, 3], ..Settle::default() });
    }

    #[test]
    fn a_seed_keeps_new_ids_past_old_stamps() {
        let mut s = Stacks::default();
        s.seed(40);
        assert_eq!(s.join(1, 2).1, 41);
    }

    #[test]
    fn a_switch_groups_rows_by_zone_in_stack_order() {
        let rows = [Some(("a", 1)), None, Some(("b", 0)), Some(("a", 0)), Some(("c", 0))];
        assert_eq!(groups(&rows), vec![vec![3, 0]]);
    }
}
