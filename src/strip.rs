//! The Scrolling family of smart tiling: one screen's windows as columns on a strip
//! that can be wider than the screen. The view shows whole columns from one column's
//! left edge, and every column outside it is parked outside every monitor, so it never
//! shows on a neighbouring screen. Pure, so it is tested here; the snapping keeps a
//! strip per tiled screen and virtual desktop, and shows the view as the screen's zones
//! (`Strip::tree`), with an empty zone after the last column when they don't fill it.

use crate::tiling::{Family, Min, Tile, Tree};
use crate::zones::{Dir, LeafTag, Node, Rect};

/// The screen's width in the units columns are counted in: sixths, so thirds and halves
/// are both whole.
const SCREEN: u8 = 6;
/// How far outside the desk a parked window waits, in pixels, so no shadow or resize
/// border of it touches a screen.
pub const PARK_PX: i32 = 200;

/// A column's width, as a share of the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Width {
    Third,
    #[default]
    Half,
    TwoThirds,
    Full,
}

impl Width {
    pub const ALL: [Width; 4] = [Width::Third, Width::Half, Width::TwoThirds, Width::Full];

    fn units(self) -> u8 {
        match self {
            Width::Third => 2,
            Width::Half => 3,
            Width::TwoThirds => 4,
            Width::Full => 6,
        }
    }

    /// The width after this one, as Column width cycles them, full wrapping to a third.
    pub fn next(self) -> Width {
        match self {
            Width::Third => Width::Half,
            Width::Half => Width::TwoThirds,
            Width::TwoThirds => Width::Full,
            Width::Full => Width::Third,
        }
    }

    /// The width nearest a share of the screen, the smaller of two as near.
    pub fn nearest(share: f32) -> Width {
        let off = |w: &Width| ((f32::from(w.units()) / f32::from(SCREEN) - share).abs() * 1000.0) as i32;
        Width::ALL.into_iter().min_by_key(off).unwrap_or_default()
    }

    /// Whether a window `min_w` wide fits a column this wide on a screen `area_w` wide. The
    /// full width always does: the screen can't give more.
    pub fn holds(self, min_w: i32, area_w: i32) -> bool {
        self == Width::Full || i64::from(area_w) * i64::from(self.units()) / i64::from(SCREEN) >= i64::from(min_w)
    }

    /// The narrowest width from a half up that holds `min_w` on a screen `area_w` wide,
    /// or the full width when none does.
    pub fn fitting(min_w: i32, area_w: i32) -> Width {
        let fits = |w: &Width| i64::from(area_w) * i64::from(w.units()) / i64::from(SCREEN) >= i64::from(min_w);
        [Width::Half, Width::TwoThirds, Width::Full].into_iter().find(fits).unwrap_or(Width::Full)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub hwnd: isize,
    pub min: Min,
    pub width: Width,
}

/// Where a column's window goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// In view, in this rect of the screen's work area.
    View(Rect),
    /// Parked, at this size; on the left of the view or the right.
    Parked { size: (i32, i32), left: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strip {
    /// Left to right.
    pub cols: Vec<Column>,
    /// The first column in view. Always a column's index, or 0 on an empty strip.
    pub view: usize,
    /// The column last focused, which a new window opens after.
    pub focus: Option<isize>,
}

/// Columns side by side, `units` sixths each, with an empty zone after them when they
/// don't fill the screen.
fn chain(units: &[u8]) -> Node {
    let leaf = || Node::Leaf(LeafTag::Leaf);
    let used: u8 = units.iter().sum();
    let mut parts: Vec<u8> = units.to_vec();
    if used < SCREEN {
        parts.push(SCREEN - used);
    }
    let mut node = leaf();
    let mut rest: u8 = parts.last().copied().unwrap_or(SCREEN);
    for &u in parts.iter().rev().skip(1) {
        rest += u;
        node = Node::Split {
            split: Dir::Vertical,
            ratio: f32::from(u) / f32::from(rest),
            a: Box::new(leaf()),
            b: Box::new(node),
        };
    }
    node
}

/// The gap between two rects, squared: 0 when they touch or overlap.
fn apart(a: Rect, b: Rect) -> i64 {
    let dx = i64::from((b.x - (a.x + a.w)).max(a.x - (b.x + b.w)).max(0));
    let dy = i64::from((b.y - (a.y + a.h)).max(a.y - (b.y + b.h)).max(0));
    dx * dx + dy * dy
}

/// The screen nearest a rect, as Windows picks the monitor of a window off every screen:
/// the first of those as near.
pub fn nearest(screens: &[Rect], r: Rect) -> Option<usize> {
    (0..screens.len()).min_by_key(|&i| apart(screens[i], r))
}

/// Where a parked window of `size` waits: outside every screen, where its own screen
/// (`own`) is the nearest, so Windows keeps it at its own screen's DPI. Off the desk's
/// side the column is on, then above it, then below, then the other side; of those, the
/// one where its own screen is nearest by the widest margin. A desk where no spot has
/// its own screen nearest gets the first spot, and the window may take a neighbour's DPI
/// while it waits, which the move back into view puts right.
pub fn park(own: Rect, screens: &[Rect], size: (i32, i32), left: bool) -> Rect {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let all = screens.iter().copied().chain(std::iter::once(own));
    let (l, t, r, b) = all.fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(l, t, r, b), s| {
        (l.min(s.x), t.min(s.y), r.max(s.x + s.w), b.max(s.y + s.h))
    });
    // Over its own screen's middle, so it overlaps no neighbour along that side.
    let x = own.x + (own.w - w) / 2;
    let y = own.y + (own.h - h) / 2;
    let west = Rect { x: l - PARK_PX - w, y, w, h };
    let east = Rect { x: r + PARK_PX, y, w, h };
    let north = Rect { x, y: t - PARK_PX - h, w, h };
    let south = Rect { x, y: b + PARK_PX, w, h };
    let spots = if left { [west, north, south, east] } else { [east, north, south, west] };
    let others: Vec<Rect> = screens.iter().copied().filter(|s| *s != own).collect();
    let margin = |spot: &Rect| {
        let mine = apart(own, *spot);
        let theirs = others.iter().map(|s| apart(*s, *spot)).min().unwrap_or(i64::MAX);
        theirs.saturating_sub(mine)
    };
    // The first of the best, so the side the column is on wins a tie.
    let best = spots.iter().enumerate().fold(None, |best: Option<(usize, i64)>, (i, s)| match best {
        Some((_, m)) if m >= margin(s) => best,
        _ => Some((i, margin(s))),
    });
    match best {
        Some((i, m)) if m > 0 => spots[i],
        _ => spots[0],
    }
}

impl Strip {
    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }

    pub fn has(&self, hwnd: isize) -> bool {
        self.index(hwnd).is_some()
    }

    pub fn index(&self, hwnd: isize) -> Option<usize> {
        self.cols.iter().position(|c| c.hwnd == hwnd)
    }

    pub fn windows(&self) -> impl Iterator<Item = isize> + '_ {
        self.cols.iter().map(|c| c.hwnd)
    }

    /// The columns in view: from `view`, as many whole ones as fit, and at least one.
    pub fn shown(&self) -> std::ops::Range<usize> {
        if self.cols.is_empty() {
            return 0..0;
        }
        let start = self.view.min(self.cols.len() - 1);
        let mut used = 0;
        let mut end = start;
        while end < self.cols.len() && (end == start || used + self.cols[end].width.units() <= SCREEN) {
            used += self.cols[end].width.units();
            end += 1;
        }
        start..end
    }

    /// Whether a window's column is in view.
    pub fn in_view(&self, hwnd: isize) -> bool {
        self.index(hwnd).is_some_and(|i| self.shown().contains(&i))
    }

    /// The view as the screen's zones: one tile per column in view, then an empty one
    /// when they don't fill the screen.
    pub fn tree(&self) -> Tree {
        if self.cols.is_empty() {
            return Tree::new(Family::Scrolling);
        }
        let shown = &self.cols[self.shown()];
        let units: Vec<u8> = shown.iter().map(|c| c.width.units()).collect();
        let mut tiles: Vec<Tile> = shown.iter().map(|c| Tile { hwnd: Some(c.hwnd), min: c.min }).collect();
        if units.iter().sum::<u8>() < SCREEN {
            tiles.push(Tile { hwnd: None, min: (0, 0) });
        }
        Tree { family: Family::Scrolling, node: Some(chain(&units)), tiles }
    }

    /// Every column as one tree of even columns, for the one rule, which looks at every
    /// window tiling would move, parked ones too.
    pub fn whole(&self) -> Tree {
        let tiles: Vec<Tile> = self.cols.iter().map(|c| Tile { hwnd: Some(c.hwnd), min: c.min }).collect();
        if tiles.is_empty() {
            return Tree::new(Family::Columns);
        }
        Tree { family: Family::Columns, node: Some(chain_even(tiles.len())), tiles }
    }

    /// A strip of these windows, in this order, each half the screen, from the first.
    pub fn of(tiles: &[Tile]) -> Strip {
        let cols =
            tiles.iter().filter_map(|t| Some(Column { hwnd: t.hwnd?, min: t.min, width: Width::Half })).collect();
        Strip { cols, view: 0, focus: None }
    }

    /// The view after the zones it showed (`tree`, from `Strip::tree`) were changed by
    /// something else: a drag that swapped, took out or dropped in a window, a window
    /// taking over its stack's tile. The columns in view become the windows those zones
    /// hold, in zone order, and empty zones go. A window new in view gets the width
    /// nearest its zone's; one already a column keeps its own. Parked columns stay.
    pub fn absorb(&self, tree: &Tree) -> Strip {
        if *tree == self.tree() {
            return self.clone();
        }
        let shares: Vec<f32> =
            tree.rects(Rect { x: 0, y: 0, w: 6000, h: 100 }, 0).iter().map(|r| r.w as f32 / 6000.0).collect();
        let fresh: Vec<Column> = tree
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(z, t)| {
                let hwnd = t.hwnd?;
                let known = self.cols.iter().find(|c| c.hwnd == hwnd);
                let width = known.map_or_else(|| Width::nearest(shares.get(z).copied().unwrap_or(0.5)), |c| c.width);
                Some(Column { hwnd, min: t.min, width })
            })
            .collect();
        let shown = self.shown();
        let taken = |c: &&Column| !fresh.iter().any(|f| f.hwnd == c.hwnd);
        let left: Vec<Column> = self.cols[..shown.start].iter().filter(taken).copied().collect();
        let right: Vec<Column> = self.cols[shown.end..].iter().filter(taken).copied().collect();
        let view = left.len();
        let cols: Vec<Column> = left.into_iter().chain(fresh).chain(right).collect();
        Strip { view, focus: self.focus, cols }.settled()
    }

    /// The strip with only the windows of `tiles` left, in their order, as the one rule
    /// left them (`plan_apply` on `whole`): a window gone or floated is out, and a window
    /// that took over a tile from its stack (`heirs`, as (the float, the heir)) takes
    /// over that column at its width.
    pub fn keep(&self, tiles: &[Tile], heirs: &[(isize, isize)]) -> Strip {
        let width_of = |h: isize| self.cols.iter().find(|c| c.hwnd == h).map(|c| c.width);
        let first = self.cols.get(self.shown().start).map(|c| c.hwnd);
        let first = first.map(|h| heirs.iter().find(|(f, _)| *f == h).map_or(h, |(_, k)| *k));
        let cols: Vec<Column> = tiles
            .iter()
            .filter_map(|t| {
                let hwnd = t.hwnd?;
                let took = heirs.iter().find(|(_, k)| *k == hwnd).and_then(|(f, _)| width_of(*f));
                Some(Column { hwnd, min: t.min, width: width_of(hwnd).or(took).unwrap_or_default() })
            })
            .collect();
        // The view stays on the first column it showed, or the one that took it over.
        let view = first.and_then(|h| cols.iter().position(|c| c.hwnd == h)).unwrap_or(self.view);
        Strip { cols, view, focus: self.focus }.settled()
    }

    /// The view aligned again: on a column, and as far left as leaves the last column in
    /// view without leaving room for another, so the end of the strip fills the screen.
    fn settled(mut self) -> Strip {
        if self.cols.is_empty() {
            self.view = 0;
        } else {
            self.view = self.view.min(self.cols.len() - 1);
            while self.view > 0 && self.fits_from(self.view - 1) {
                self.view -= 1;
            }
        }
        if self.focus.is_some_and(|f| !self.has(f)) {
            self.focus = None;
        }
        self
    }

    /// Whether every column from `start` to the end fits in the screen.
    fn fits_from(&self, start: usize) -> bool {
        self.cols[start..].iter().map(|c| u32::from(c.width.units())).sum::<u32>() <= u32::from(SCREEN)
    }

    /// Scrolled so column `at` is in view, as little as that takes.
    fn show(mut self, at: usize) -> Strip {
        if at >= self.cols.len() {
            return self.settled();
        }
        if at < self.view {
            self.view = at;
        } else {
            while !self.shown().contains(&at) {
                self.view += 1;
            }
        }
        self.settled()
    }

    /// A new window's column, after the focused one or else at the end, in view and focused.
    pub fn insert(&self, hwnd: isize, min: Min, width: Width) -> Strip {
        if self.has(hwnd) {
            return self.clone();
        }
        let at = self.focus.and_then(|f| self.index(f)).map_or(self.cols.len(), |i| i + 1);
        let mut next = self.clone();
        if at <= next.view && !next.cols.is_empty() && at < next.cols.len() {
            next.view += 1;
        }
        next.cols.insert(at, Column { hwnd, min, width });
        next.focus = Some(hwnd);
        next.show(at)
    }

    /// A column goes. The view keeps its place; the focus goes to the column on its left,
    /// or else the one that took its place.
    pub fn remove(&self, hwnd: isize) -> Strip {
        let Some(at) = self.index(hwnd) else { return self.clone() };
        let mut next = self.clone();
        next.cols.remove(at);
        if at < next.view {
            next.view -= 1;
        }
        if next.focus == Some(hwnd) {
            next.focus = next.cols.get(at.saturating_sub(1)).map(|c| c.hwnd);
        }
        next.settled()
    }

    /// A window's column swaps with its neighbour, and stays in view. None at the end it goes to.
    pub fn moved(&self, hwnd: isize, left: bool) -> Option<Strip> {
        let at = self.index(hwnd)?;
        let to = if left { at.checked_sub(1)? } else { at + 1 };
        if to >= self.cols.len() {
            return None;
        }
        let mut next = self.clone();
        next.cols.swap(at, to);
        Some(next.show(to))
    }

    /// A window's column at the next width its window fits on a screen `area_w` wide, in view.
    pub fn widened(&self, hwnd: isize, area_w: i32) -> Option<Strip> {
        let at = self.index(hwnd)?;
        let mut next = self.clone();
        let c = &mut next.cols[at];
        c.width = c.width.next();
        while !c.width.holds(c.min.0, area_w) {
            c.width = c.width.next();
        }
        Some(next.show(at))
    }

    /// Each column in view at the width nearest its share of the screen, from a handle drag,
    /// or the next one up that its window fits.
    pub fn with_shares(&self, shares: &[f32], area_w: i32) -> Strip {
        let shown = self.shown();
        let mut next = self.clone();
        for (c, share) in next.cols[shown].iter_mut().zip(shares) {
            c.width = Width::nearest(*share);
            while !c.width.holds(c.min.0, area_w) {
                c.width = c.width.next();
            }
        }
        next.settled()
    }

    /// The view one column further left or right, the focus where it is. None when the
    /// strip ends that way already.
    pub fn scrolled(&self, left: bool) -> Option<Strip> {
        let shown = self.shown();
        let mut next = self.clone();
        if left {
            next.view = self.view.checked_sub(1)?;
        } else if shown.end < self.cols.len() {
            next.view += 1;
        } else {
            return None;
        }
        Some(next)
    }

    /// The column next to the focused one, or to the view's first when none is focused,
    /// focused and in view. None when the strip ends that way.
    pub fn walked(&self, left: bool) -> Option<(Strip, isize)> {
        let from = self.focus.and_then(|f| self.index(f)).unwrap_or(self.shown().start);
        let to = if left { from.checked_sub(1)? } else { from + 1 };
        let hwnd = self.cols.get(to)?.hwnd;
        let mut next = self.clone();
        next.focus = Some(hwnd);
        Some((next.show(to), hwnd))
    }

    /// A window took the focus: its column is the focused one, scrolled into view.
    pub fn focused(&self, hwnd: isize) -> Strip {
        let Some(at) = self.index(hwnd) else { return self.clone() };
        let mut next = self.clone();
        next.focus = Some(hwnd);
        next.show(at)
    }

    /// Where each column's window goes on a work area `area` with `gap`: the ones in view
    /// in their zone, the others parked at the size they have in view.
    pub fn places(&self, area: Rect, gap: i32) -> Vec<(isize, Place)> {
        let shown = self.shown();
        let rects = self.tree().rects(area, gap);
        self.cols
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let place = if shown.contains(&i) {
                    Place::View(rects[i - shown.start])
                } else {
                    let r = crate::zones::rects(&chain(&[c.width.units()]), area, gap)[0];
                    Place::Parked { size: (r.w, r.h), left: i < shown.start }
                };
                (c.hwnd, place)
            })
            .collect()
    }
}

/// `n` equal columns, for `Strip::whole`.
fn chain_even(n: usize) -> Node {
    let leaf = || Node::Leaf(LeafTag::Leaf);
    (1..n).rev().fold(leaf(), |node, i| Node::Split {
        split: Dir::Vertical,
        ratio: 1.0 / (n - i + 1) as f32,
        a: Box::new(leaf()),
        b: Box::new(node),
    })
}

/// Whether a window's frame is on no screen at all, so nobody can see or reach it.
pub fn stranded(screens: &[Rect], frame: Rect) -> bool {
    !screens
        .iter()
        .any(|s| frame.x < s.x + s.w && s.x < frame.x + frame.w && frame.y < s.y + s.h && s.y < frame.y + frame.h)
}

/// Where a window leaving a strip parked goes back to: the first of `choices` that is
/// on some screen (the place it had before tiling took it, then the one its park stamp
/// kept), else the middle of `area`, the work area of its screen or the nearest one, at
/// `size` up to the area's. A parked place is never chosen, so it can't strand it again.
pub fn way_back(choices: &[Option<Rect>], screens: &[Rect], area: Rect, size: (i32, i32)) -> Rect {
    if let Some(r) = choices.iter().flatten().find(|r| r.w > 0 && r.h > 0 && !stranded(screens, **r)) {
        return *r;
    }
    let (w, h) = (size.0.clamp(1, area.w.max(1)), size.1.clamp(1, area.h.max(1)));
    Rect { x: area.x + (area.w - w) / 2, y: area.y + (area.h - h) / 2, w, h }
}

/// Two numbers in one window property, as the park stamp keeps a rect in two.
pub fn pack(a: i32, b: i32) -> u64 {
    (u64::from(a as u32) << 32) | u64::from(b as u32)
}

pub fn unpack(v: u64) -> (i32, i32) {
    ((v >> 32) as u32 as i32, v as u32 as i32)
}

/// Where a window goes back to on the screens there are, each as (bounds, work area): the
/// first of `choices` on one of them (`way_back`), else the middle of its own screen's
/// work area while `own` is still there as it was, or else of the screen nearest `at`
/// (where it is now, or was). None with no screen to go to.
pub fn home(
    choices: &[Option<Rect>],
    screens: &[(Rect, Rect)],
    own: Option<Rect>,
    at: Option<Rect>,
    size: (i32, i32),
) -> Option<Rect> {
    let bounds: Vec<Rect> = screens.iter().map(|s| s.0).collect();
    let mine = own.and_then(|o| screens.iter().find(|s| s.0 == o)).map(|s| s.1);
    let area = match mine {
        Some(work) => work,
        None => screens[nearest(&bounds, at.or_else(|| choices.iter().flatten().next().copied())?)?].1,
    };
    Some(way_back(choices, &bounds, area, size))
}

/// Where a window at `frame` must land so it is on a screen again, by `home`, or None when
/// it is on one of `screens` already.
pub fn land(
    frame: Rect,
    choices: &[Option<Rect>],
    screens: &[(Rect, Rect)],
    own: Option<Rect>,
    size: (i32, i32),
) -> Option<Rect> {
    let bounds: Vec<Rect> = screens.iter().map(|s| s.0).collect();
    if !stranded(&bounds, frame) {
        return None;
    }
    home(choices, screens, own, Some(frame), size)
}

/// Which way a Win+wheel turn scrolls, and what is left of a notch, from what was left
/// (`turned`) and this event's `delta`: up on a wheel and left on a tilt are left. A turn
/// the other way starts again. Returns the columns to scroll, left ones as negative.
pub fn notches(turned: i32, delta: i32, vertical: bool, notch: i32) -> (i32, i32) {
    let delta = if vertical { delta } else { -delta };
    let mut turned = if turned.signum() == delta.signum() { turned + delta } else { delta };
    let mut steps = 0;
    while turned.abs() >= notch {
        steps -= turned.signum();
        turned -= notch * turned.signum();
    }
    (steps, turned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    fn grow(n: isize) -> Strip {
        (1..=n).fold(Strip::default(), |s, h| s.insert(h, (0, 0), Width::Half))
    }

    fn shown(s: &Strip) -> Vec<isize> {
        s.cols[s.shown()].iter().map(|c| c.hwnd).collect()
    }

    #[test]
    fn new_columns_are_half_wide_and_open_after_the_focused_one() {
        let two = grow(2);
        assert_eq!(shown(&two), vec![1, 2]);
        assert_eq!(two.focus, Some(2));
        // A third scrolls the view so it is in view, as little as that takes.
        let three = grow(3);
        assert_eq!((three.view, shown(&three)), (1, vec![2, 3]));
        // After the focused column, not at the end.
        let mid = three.focused(1).insert(9, (0, 0), Width::Half);
        assert_eq!(mid.windows().collect::<Vec<_>>(), vec![1, 9, 2, 3]);
        assert_eq!((shown(&mid), mid.focus), (vec![1, 9], Some(9)));
        let rects = two.tree().rects(AREA, 0);
        assert_eq!(rects, vec![Rect { x: 0, y: 0, w: 960, h: 1080 }, Rect { x: 960, y: 0, w: 960, h: 1080 }]);
    }

    #[test]
    fn a_view_that_isnt_full_shows_an_empty_zone_after_its_columns() {
        let one = grow(1);
        let tree = one.tree();
        assert_eq!(tree.tiles.len(), 2);
        assert_eq!(tree.tiles[1].hwnd, None);
        assert_eq!(tree.rects(AREA, 0)[0].w, 960);
        // A half and a two-thirds don't both fit: the view shows the half and an empty zone.
        let mixed = grow(2).widened(2, 1920).unwrap().focused(1);
        assert_eq!(shown(&mixed), vec![1]);
        assert_eq!(mixed.tree().tiles.len(), 2);
        // A full column takes the screen alone.
        let full = Strip::default().insert(1, (0, 0), Width::Full);
        assert_eq!(full.tree().rects(AREA, 0), vec![AREA]);
        assert_eq!(Strip::default().tree(), Tree::new(Family::Scrolling));
    }

    #[test]
    fn widths_cycle_and_the_nearest_is_found() {
        let mut w = Width::Half;
        let mut seen = Vec::new();
        for _ in 0..4 {
            w = w.next();
            seen.push(w);
        }
        assert_eq!(seen, vec![Width::TwoThirds, Width::Full, Width::Third, Width::Half]);
        assert_eq!(Width::nearest(0.34), Width::Third);
        assert_eq!(Width::nearest(0.52), Width::Half);
        assert_eq!(Width::nearest(0.7), Width::TwoThirds);
        assert_eq!(Width::nearest(0.95), Width::Full);
        assert_eq!(Width::fitting(0, 1920), Width::Half);
        assert_eq!(Width::fitting(1000, 1920), Width::TwoThirds);
        assert_eq!(Width::fitting(1500, 1920), Width::Full);
        assert_eq!(Width::fitting(5000, 1920), Width::Full);
    }

    #[test]
    fn scrolling_moves_the_view_a_column_and_stops_at_the_ends() {
        let four = grow(4);
        assert_eq!(shown(&four), vec![3, 4]);
        assert!(four.scrolled(false).is_none(), "the last column is in view");
        let back = four.scrolled(true).unwrap();
        assert_eq!((shown(&back), back.focus), (vec![2, 3], Some(4)));
        let start = back.scrolled(true).unwrap();
        assert_eq!(shown(&start), vec![1, 2]);
        assert!(start.scrolled(true).is_none());
    }

    #[test]
    fn walking_focuses_the_next_column_and_brings_it_into_view() {
        let four = grow(4).focused(1);
        assert_eq!(shown(&four), vec![1, 2]);
        let (next, h) = four.walked(false).unwrap();
        assert_eq!((h, shown(&next)), (2, vec![1, 2]));
        let (next, h) = next.walked(false).unwrap();
        assert_eq!((h, shown(&next)), (3, vec![2, 3]));
        assert!(grow(4).walked(false).is_none());
        assert!(four.walked(true).is_none());
    }

    #[test]
    fn a_parked_window_that_takes_the_focus_comes_into_view() {
        let four = grow(4);
        assert!(!four.in_view(1));
        let back = four.focused(1);
        assert!(back.in_view(1));
        assert_eq!((back.view, back.focus), (0, Some(1)));
        // A window that isn't a column changes nothing.
        assert_eq!(four.focused(99), four);
    }

    #[test]
    fn moving_a_column_swaps_it_with_its_neighbour() {
        let three = grow(3);
        let moved = three.moved(3, true).unwrap();
        assert_eq!(moved.windows().collect::<Vec<_>>(), vec![1, 3, 2]);
        assert!(moved.in_view(3));
        assert!(three.moved(1, true).is_none());
        assert!(three.moved(3, false).is_none());
        assert!(three.moved(99, false).is_none());
    }

    #[test]
    fn removing_keeps_the_view_aligned_and_the_screen_full() {
        let four = grow(4);
        // The view was on 3; with 3 gone, 2 and 4 fill the screen.
        let less = four.remove(3);
        assert_eq!(shown(&less), vec![2, 4]);
        // A column left of the view going moves the view's index with it.
        let left = four.remove(1);
        assert_eq!(shown(&left), vec![3, 4]);
        // The focus goes to the left neighbour.
        assert_eq!(four.remove(4).focus, Some(3));
        assert_eq!(grow(1).remove(1), Strip::default());
    }

    #[test]
    fn a_drag_in_the_view_is_taken_back_into_the_strip() {
        let four = grow(4);
        // A swap of the two in view.
        let mut tree = four.tree();
        tree.tiles.swap(0, 1);
        let swapped = four.absorb(&tree);
        assert_eq!(swapped.windows().collect::<Vec<_>>(), vec![1, 2, 4, 3]);
        // A window dragged out: its zone is empty, and it goes.
        let mut tree = four.tree();
        tree.tiles[0].hwnd = None;
        let out = four.absorb(&tree);
        assert_eq!(out.windows().collect::<Vec<_>>(), vec![1, 2, 4]);
        // A window dropped into the empty zone after one column takes its width.
        let one = grow(1);
        let mut tree = one.tree();
        tree.tiles[1].hwnd = Some(7);
        let dropped = one.absorb(&tree);
        assert_eq!(dropped.cols[1], Column { hwnd: 7, min: (0, 0), width: Width::Half });
        // Nothing changed, nothing moves.
        assert_eq!(four.absorb(&four.tree()), four);
    }

    #[test]
    fn handle_drags_take_the_nearest_widths() {
        let two = grow(2);
        let wider = two.with_shares(&[0.66, 0.34], 1920);
        assert_eq!(wider.cols.iter().map(|c| c.width).collect::<Vec<_>>(), vec![Width::TwoThirds, Width::Third]);
    }

    #[test]
    fn the_one_rule_keeps_widths_and_hands_a_column_to_an_heir() {
        let three = grow(3).widened(1, 1920).unwrap();
        let mut whole = three.whole();
        assert_eq!(whole.tiles.len(), 3);
        whole.tiles.remove(1);
        let kept = three.keep(&whole.tiles, &[]);
        assert_eq!(kept.windows().collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(kept.cols[0].width, Width::TwoThirds);
        // An heir takes the width of the column it took over.
        let mut whole = three.whole();
        whole.tiles[0].hwnd = Some(8);
        let heir = three.keep(&whole.tiles, &[(1, 8)]);
        assert_eq!(heir.cols[0], Column { hwnd: 8, min: (0, 0), width: Width::TwoThirds });
    }

    #[test]
    fn parked_columns_keep_their_size_and_their_side() {
        let four = grow(4).focused(2);
        let places = four.places(AREA, 0);
        assert_eq!(places[0], (1, Place::Parked { size: (960, 1080), left: true }));
        assert!(matches!(places[1].1, Place::View(r) if r.x == 0));
        assert!(matches!(places[2].1, Place::View(r) if r.x == 960));
        assert_eq!(places[3], (4, Place::Parked { size: (960, 1080), left: false }));
    }

    #[test]
    fn a_column_never_gets_narrower_than_its_window() {
        // A window 1000 px wide on a 1920 px screen: a third and a half are too narrow.
        let s = Strip::default().insert(1, (1000, 0), Width::TwoThirds);
        let widths: Vec<Width> = (0..4)
            .scan(s.clone(), |s, _| {
                *s = s.widened(1, 1920).unwrap();
                Some(s.cols[0].width)
            })
            .collect();
        assert_eq!(widths, vec![Width::Full, Width::TwoThirds, Width::Full, Width::TwoThirds]);
        // A handle drag to a third gives it the narrowest width that holds it.
        let dragged = s.with_shares(&[0.33], 1920);
        assert_eq!(dragged.cols[0].width, Width::TwoThirds);
        assert!(Width::Full.holds(5000, 1920));
        assert!(!Width::Half.holds(961, 1920) && Width::Half.holds(960, 1920));
    }

    #[test]
    fn the_focused_column_is_in_view_after_every_key() {
        let four = grow(4).focused(1);
        let (walked, h) = four.walked(false).unwrap();
        assert!(walked.in_view(h) && walked.focus == Some(h));
        let moved = four.moved(1, false).unwrap();
        assert!(moved.in_view(1));
        let wide = four.widened(1, 1920).unwrap();
        assert!(wide.in_view(1));
        // Widths go with their columns when they move, and the rule's own tiles change nothing.
        let mixed = grow(3).widened(2, 1920).unwrap();
        let swapped = mixed.moved(2, true).unwrap();
        assert_eq!(swapped.cols[0], Column { hwnd: 2, min: (0, 0), width: Width::TwoThirds });
        assert_eq!(mixed.keep(&mixed.whole().tiles, &[]), mixed);
        assert_eq!(mixed.absorb(&mixed.tree()), mixed);
    }

    #[test]
    fn every_column_out_of_view_is_parked_on_its_side() {
        for strip in [grow(1), grow(5), grow(6).focused(3), grow(7).widened(4, 1920).unwrap()] {
            let shown = strip.shown();
            for (i, (_, place)) in strip.places(AREA, 8).into_iter().enumerate() {
                match place {
                    Place::View(_) => assert!(shown.contains(&i)),
                    Place::Parked { left, .. } => {
                        assert!(!shown.contains(&i));
                        assert_eq!(left, i < shown.start);
                    }
                }
            }
        }
    }

    #[test]
    fn a_window_goes_back_to_a_place_on_a_screen_never_a_parked_one() {
        let desk = [LEFT, MAIN, RIGHT];
        let work = Rect { x: 0, y: 0, w: 2560, h: 1400 };
        let before = Rect { x: 100, y: 100, w: 800, h: 600 };
        let parked = park(MAIN, &desk, (800, 600), true);
        assert!(stranded(&desk, parked));
        // Its place before tiling, when that is on a screen.
        assert_eq!(way_back(&[Some(before), Some(parked)], &desk, work, (800, 600)), before);
        // A `before` taken from a parked window after a crash is passed over for the stamp's.
        assert_eq!(way_back(&[Some(parked), Some(before)], &desk, work, (800, 600)), before);
        // Nothing on a screen: the middle of the work area, at its size up to the area's.
        assert_eq!(way_back(&[Some(parked), None], &desk, work, (800, 600)), Rect { x: 880, y: 400, w: 800, h: 600 });
        assert_eq!(way_back(&[], &desk, work, (9000, 9000)), work);
        // A screen that went: its windows' places are off the desk now.
        assert!(stranded(&[MAIN], Rect { x: -1500, y: 10, w: 800, h: 600 }));
        assert!(!stranded(&desk, Rect { x: -10, y: 10, w: 800, h: 600 }));
        assert!(stranded(&desk, Rect { x: 3640, y: 0, w: 10, h: 10 }), "touching an edge isn't on it");
    }

    /// A screen and its work area, the taskbar along the bottom.
    fn with_work(r: Rect) -> (Rect, Rect) {
        (r, Rect { h: r.h - 48, ..r })
    }

    #[test]
    fn a_window_of_a_screen_that_went_lands_on_one_still_there() {
        let before: [(Rect, Rect); 3] = [with_work(LEFT), with_work(MAIN), with_work(RIGHT)];
        // Screen LEFT unplugged: a window on it, its place before tiling there too.
        let after = [with_work(MAIN), with_work(RIGHT)];
        let on_left = Rect { x: -1500, y: 100, w: 800, h: 600 };
        let target = land(on_left, &[Some(on_left), None], &after, Some(LEFT), (800, 600)).unwrap();
        assert!(!stranded(&[MAIN, RIGHT], target));
        assert_eq!(target, Rect { x: 880, y: 396, w: 800, h: 600 }, "the middle of the nearest screen still there");
        // A place before on a screen still there wins.
        let kept = Rect { x: 100, y: 100, w: 800, h: 600 };
        assert_eq!(land(on_left, &[Some(kept)], &after, Some(LEFT), (800, 600)), Some(kept));
        // A window on a screen still there doesn't move.
        assert_eq!(land(kept, &[None], &after, Some(MAIN), (800, 600)), None);
        // Nothing to land on.
        assert_eq!(land(on_left, &[None], &[], Some(LEFT), (800, 600)), None);
        // On the old screens, the same place was fine: a stale list can't decide this.
        assert_eq!(land(on_left, &[Some(on_left)], &before, Some(LEFT), (800, 600)), None);
    }

    #[test]
    fn a_window_of_a_screen_that_moved_lands_on_the_screens_as_they_are() {
        // RIGHT moved from the right of MAIN to its left.
        let moved = Rect { x: -1080, y: 0, w: 1080, h: 1920 };
        let after = [with_work(moved), with_work(MAIN)];
        let at_old = Rect { x: 2700, y: 300, w: 900, h: 900 };
        let target = land(at_old, &[Some(at_old)], &after, Some(RIGHT), (900, 900)).unwrap();
        assert!(!stranded(&[moved, MAIN], target));
        // Its own screen, as it is now, holds it when it is still where it was.
        let home_now = home(&[None], &after, Some(moved), Some(at_old), (900, 900)).unwrap();
        assert!(home_now.x >= moved.x && home_now.x + home_now.w <= moved.x + moved.w);
    }

    #[test]
    fn a_rect_survives_its_stamp() {
        for (a, b) in [(0, 0), (-1920, 1080), (i32::MIN, i32::MAX), (-1, -1)] {
            assert_eq!(unpack(pack(a, b)), (a, b));
        }
    }

    #[test]
    fn the_wheel_scrolls_a_column_a_notch() {
        // A wheel notch up is one column left; a touchpad's small steps add up to one.
        assert_eq!(notches(0, 120, true, 120), (-1, 0));
        assert_eq!(notches(0, -240, true, 120), (2, 0));
        assert_eq!(notches(0, 40, true, 120), (0, 40));
        assert_eq!(notches(80, 40, true, 120), (-1, 0));
        // The other way starts again; a tilt right is right.
        assert_eq!(notches(80, -40, true, 120), (0, -40));
        assert_eq!(notches(0, 120, false, 120), (1, 0));
    }

    #[test]
    fn a_tie_still_parks_off_every_screen() {
        // A full-width column on the middle of three level screens is as near its
        // neighbours above and below, and falls back to the side it is on.
        let middle = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        let desk = [Rect { x: -1920, y: 0, w: 1920, h: 1080 }, middle, Rect { x: 1920, y: 0, w: 1920, h: 1080 }];
        for left in [true, false] {
            let spot = park(middle, &desk, (1920, 1040), left);
            assert!(stranded(&desk, spot));
            assert_eq!(spot.x < 0, left);
        }
        // Stacked screens, one above the other, with negative y.
        let stacked = [Rect { x: 0, y: -1080, w: 1920, h: 1080 }, Rect { x: 0, y: 0, w: 1920, h: 1080 }];
        for own in stacked {
            let spot = park(own, &stacked, (960, 1000), true);
            assert!(stranded(&stacked, spot));
            assert_eq!(stacked[nearest(&stacked, spot).unwrap()], own);
        }
    }

    const LEFT: Rect = Rect { x: -1920, y: 0, w: 1920, h: 1080 };
    const MAIN: Rect = Rect { x: 0, y: 0, w: 2560, h: 1440 };
    const RIGHT: Rect = Rect { x: 2560, y: 200, w: 1080, h: 1920 };

    fn on_no_screen(screens: &[Rect], r: Rect) -> bool {
        screens.iter().all(|s| apart(*s, r) > 0)
    }

    #[test]
    fn a_parked_window_is_off_every_screen_and_nearest_its_own() {
        let desk = [LEFT, MAIN, RIGHT];
        for own in desk {
            for left in [true, false] {
                for size in [(640, 1000), (1280, 1400), (2560, 1440), (1, 1)] {
                    let spot = park(own, &desk, size, left);
                    assert!(on_no_screen(&desk, spot), "{own:?} {left} {size:?}: {spot:?}");
                    let gap = desk.iter().map(|s| apart(*s, spot)).min().unwrap();
                    assert!(gap >= i64::from(PARK_PX) * i64::from(PARK_PX), "{spot:?}");
                    // A window as wide as a screen between two, with level tops and bottoms,
                    // is as near its neighbours above and below, so only that may tie.
                    if size.0 < own.w {
                        assert_eq!(desk[nearest(&desk, spot).unwrap()], own, "{own:?} {left} {size:?}: {spot:?}");
                    }
                }
            }
        }
        // One screen: the side the column is on.
        assert!(park(MAIN, &[MAIN], (100, 100), true).x < 0);
        assert!(park(MAIN, &[MAIN], (100, 100), false).x >= 2560);
    }

    #[test]
    fn a_screen_between_two_parks_above_or_below() {
        let desk = [LEFT, MAIN, RIGHT];
        let spot = park(MAIN, &desk, (1280, 1440), true);
        assert!(spot.y + spot.h < 0 || spot.y > 2120, "{spot:?}");
    }

    #[test]
    fn the_nearest_screen_of_a_rect_off_the_desk() {
        let desk = [LEFT, MAIN, RIGHT];
        assert_eq!(nearest(&desk, Rect { x: -5000, y: 0, w: 10, h: 10 }), Some(0));
        assert_eq!(nearest(&desk, Rect { x: 9000, y: 900, w: 10, h: 10 }), Some(2));
        assert_eq!(nearest(&desk, Rect { x: 100, y: -3000, w: 10, h: 10 }), Some(1));
        assert_eq!(nearest(&[], AREA), None);
    }

    /// A small fixed-seed generator, so the walk is the same on every run.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    /// No two columns in view overlap, all are inside the area and start at its left,
    /// the view is on a column, and the end of the strip fills the screen.
    fn sound(s: &Strip, area: Rect, gap: i32) {
        if s.cols.is_empty() {
            assert_eq!((s.view, s.tree()), (0, Tree::new(Family::Scrolling)));
            return;
        }
        assert!(s.view < s.cols.len());
        let shown = s.shown();
        assert_eq!(shown.start, s.view);
        let used: u8 = s.cols[shown.clone()].iter().map(|c| c.width.units()).sum();
        assert!(used <= SCREEN || shown.len() == 1);
        assert!(s.view == 0 || !s.fits_from(s.view - 1), "the view could show more of the end");
        let tree = s.tree();
        assert!(crate::zones::within_limits(&tree.layout()));
        let rects = tree.rects(area, gap);
        assert_eq!(rects.len(), tree.tiles.len());
        let mut views = Vec::new();
        for (h, place) in s.places(area, gap) {
            match place {
                Place::View(r) => views.push((h, r)),
                Place::Parked { size, .. } => assert!(size.0 > 0 && size.1 > 0),
            }
        }
        assert_eq!(views.len(), shown.len());
        assert!(views[0].1.x <= area.x + gap, "the view starts at a column's left edge");
        for (i, (_, r)) in views.iter().enumerate() {
            assert!(r.w > 0 && r.h > 0);
            assert!(r.x >= area.x && r.x + r.w <= area.x + area.w && r.y >= area.y && r.y + r.h <= area.y + area.h);
            for (_, o) in &views[i + 1..] {
                assert!(r.x + r.w <= o.x || o.x + o.w <= r.x, "{r:?} overlaps {o:?}");
            }
        }
        let mut seen: Vec<isize> = s.windows().collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), s.cols.len(), "a window is in two columns");
        assert!(s.focus.is_none_or(|f| s.has(f)));
    }

    #[test]
    fn random_steps_keep_the_strip_sound() {
        let areas = [AREA, Rect { x: -2560, y: 40, w: 2560, h: 1392 }, Rect { x: 7, y: 3, w: 1001, h: 997 }];
        for (n, area) in areas.into_iter().enumerate() {
            let mut rng = Lcg(0x5c40 + n as u64);
            let mut strip = Strip::default();
            let mut known: Vec<isize> = Vec::new();
            for step in 0..5000 {
                let pick = |rng: &mut Lcg, known: &[isize]| known[rng.next(known.len() as u64) as usize];
                let next = step as isize + 100;
                strip = match rng.next(11) {
                    0..=2 => {
                        known.push(next);
                        strip.insert(next, (0, 0), Width::ALL[rng.next(4) as usize])
                    }
                    3 if !known.is_empty() => {
                        let h = known.remove(rng.next(known.len() as u64) as usize);
                        strip.remove(h)
                    }
                    4 if !known.is_empty() => {
                        let h = pick(&mut rng, &known);
                        strip.moved(h, rng.next(2) == 0).unwrap_or(strip)
                    }
                    5 if !known.is_empty() => {
                        let h = pick(&mut rng, &known);
                        strip.widened(h, area.w).unwrap()
                    }
                    6 => strip.scrolled(rng.next(2) == 0).unwrap_or(strip),
                    7 => strip.walked(rng.next(2) == 0).map_or(strip, |(s, _)| s),
                    8 if !known.is_empty() => {
                        let h = pick(&mut rng, &known);
                        let s = strip.focused(h);
                        assert!(s.in_view(h));
                        s
                    }
                    9 if !known.is_empty() => {
                        // A drag in the view: a swap, or a window taken out.
                        let mut tree = strip.tree();
                        let filled: Vec<usize> =
                            (0..tree.tiles.len()).filter(|z| tree.tiles[*z].hwnd.is_some()).collect();
                        if rng.next(2) == 0 && filled.len() > 1 {
                            tree.tiles.swap(filled[0], filled[1]);
                        } else {
                            let z = filled[rng.next(filled.len() as u64) as usize];
                            let h = tree.tiles[z].hwnd.unwrap();
                            tree.tiles[z].hwnd = None;
                            known.retain(|k| *k != h);
                        }
                        strip.absorb(&tree)
                    }
                    10 if !known.is_empty() => {
                        // The one rule floats one, anywhere on the strip.
                        let mut whole = strip.whole();
                        let h = pick(&mut rng, &known);
                        whole.tiles.retain(|t| t.hwnd != Some(h));
                        known.retain(|k| *k != h);
                        strip.keep(&whole.tiles, &[])
                    }
                    _ => strip,
                };
                sound(&strip, area, 8);
                let mut have: Vec<isize> = strip.windows().collect();
                let mut want = known.clone();
                have.sort_unstable();
                want.sort_unstable();
                assert_eq!(have, want, "step {step}: a window was lost or made up");
            }
        }
    }
}
