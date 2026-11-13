//! Groups: the windows in one screen's zones, sized together. The solver moves
//! one divider and pushes the ones beyond it when a zone reaches its window's
//! minimum size, so no window is squeezed and nothing else shifts. Pure logic,
//! shared by grow, shrink and priority now and by dragging a window's edge later.

use crate::zones::{self, Dir, LeafTag, Node, Rect, Template, ZoneLayout, MIN_ZONE_PX};

/// The ratio clamp every split keeps, as in `zones`.
const LOW: f32 = 0.1;
const HIGH: f32 = 0.9;
/// The share Priority gives a window along the layout's main split.
const PRIORITY_SHARE: f32 = 2.0 / 3.0;
/// Grow and shrink step by this share of the group's size along the divider's axis, at least `MIN_STEP_PX`.
const STEP_SHARE: f32 = 0.05;
const MIN_STEP_PX: i32 = 32;

/// A side of a zone, or of the zones a window covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// A divider, named by the path down to its split as in `zones::Divider` (false is
/// the first child). A positive delta moves it right or down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub path: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Solved {
    pub tree: Node,
    /// The zones after the solve with no gap, so they tile the area. `zones::rects`
    /// with the gap gives the windows' rects.
    pub cells: Vec<Rect>,
    /// How far the divider moved, which is less than asked when a zone reached its minimum.
    pub applied_delta: i32,
}

/// The ratios before Priority, to give back exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct Stash {
    tree: Node,
}

/// The tree with every divider at its pixel position. `was` is the split's
/// (start, divider, end) when it was built, so one that didn't move keeps its ratio.
#[derive(Clone)]
enum Abs {
    Leaf(usize),
    Split { dir: Dir, ratio: f32, at: i32, was: (i32, i32, i32), a: Box<Abs>, b: Box<Abs> },
}

/// Start and length of `r` along the axis a split in `dir` divides.
fn span_of(r: Rect, dir: Dir) -> (i32, i32) {
    match dir {
        Dir::Vertical => (r.x, r.w),
        Dir::Horizontal => (r.y, r.h),
    }
}

/// `r` cut in two at `at`, the same way `zones::rects` cuts it.
fn cut(r: Rect, dir: Dir, at: i32) -> (Rect, Rect) {
    match dir {
        Dir::Vertical => (Rect { w: at - r.x, ..r }, Rect { x: at, w: r.x + r.w - at, ..r }),
        Dir::Horizontal => (Rect { h: at - r.y, ..r }, Rect { y: at, h: r.y + r.h - at, ..r }),
    }
}

/// Where a split's divider lands in `r`, rounded as `zones::rects` rounds it.
fn divider_at(r: Rect, dir: Dir, ratio: f32) -> i32 {
    let (lo, len) = span_of(r, dir);
    lo + (len as f32 * ratio.clamp(LOW, HIGH)).round() as i32
}

fn build(node: &Node, r: Rect, next: &mut usize) -> Abs {
    match node {
        Node::Leaf(_) => {
            *next += 1;
            Abs::Leaf(*next - 1)
        }
        Node::Split { split, ratio, a, b } => {
            let (lo, len) = span_of(r, *split);
            let at = divider_at(r, *split, *ratio);
            let (ra, rb) = cut(r, *split, at);
            let (a, b) = (Box::new(build(a, ra, next)), Box::new(build(b, rb, next)));
            Abs::Split { dir: *split, ratio: *ratio, at, was: (lo, at, lo + len), a, b }
        }
    }
}

/// The split at `path` and the rect it divides.
fn locate<'a>(node: &'a Abs, r: Rect, path: &[bool]) -> Option<(&'a Abs, Rect)> {
    match (node, path.split_first()) {
        (Abs::Split { .. }, None) => Some((node, r)),
        (Abs::Split { dir, at, a, b, .. }, Some((second, rest))) => {
            let (ra, rb) = cut(r, *dir, *at);
            if *second {
                locate(b, rb, rest)
            } else {
                locate(a, ra, rest)
            }
        }
        _ => None,
    }
}

fn locate_mut<'a>(node: &'a mut Abs, path: &[bool]) -> Option<&'a mut Abs> {
    match (node, path.split_first()) {
        (node @ Abs::Split { .. }, None) => Some(node),
        (Abs::Split { a, b, .. }, Some((second, rest))) => locate_mut(if *second { b } else { a }, rest),
        _ => None,
    }
}

/// The smallest a subtree can be along `dir`'s axis with every zone at its minimum.
fn need(node: &Abs, dir: Dir, mins: &[(i32, i32)]) -> i32 {
    match node {
        Abs::Leaf(i) => {
            let (w, h) = mins.get(*i).copied().unwrap_or_default();
            (if dir == Dir::Vertical { w } else { h }).max(MIN_ZONE_PX)
        }
        Abs::Split { dir: d, a, b, .. } if *d == dir => need(a, dir, mins) + need(b, dir, mins),
        Abs::Split { a, b, .. } => need(a, dir, mins).max(need(b, dir, mins)),
    }
}

/// The nearest split above `path` across the same axis that bounds it from the
/// start (`second`) or the end side: the next divider beyond it that way.
fn bound(root: &Abs, path: &[bool], dir: Dir, second: bool) -> Option<usize> {
    let across = |k: usize| matches!(locate(root, Rect::default(), &path[..k]), Some((Abs::Split { dir: d, .. }, _)) if *d == dir);
    (0..path.len()).rev().find(|k| path[*k] == second && across(*k))
}

/// The least a child keeps of a split `len` long: the ratio clamp's tenth.
fn tenth(len: i32) -> i32 {
    (len.max(0) + 9) / 10
}

/// A subtree's extent along `dir`'s axis went from `old` to `new`. Where a side
/// came in, push the dividers inside so no zone goes below its minimum, or below
/// its size now if it already is. Dividers with room stay where they are.
fn squeeze(node: &mut Abs, dir: Dir, old: (i32, i32), new: (i32, i32), mins: &[(i32, i32)]) {
    let Abs::Split { dir: d, at, a, b, .. } = node else { return };
    if *d != dir {
        squeeze(a, dir, old, new, mins);
        squeeze(b, dir, old, new, mins);
        return;
    }
    let was = *at;
    let mut to = was;
    if new.1 < old.1 {
        to = to.min(new.1 - need(b, dir, mins).min(old.1 - was));
    }
    if new.0 > old.0 {
        to = to.max(new.0 + need(a, dir, mins).min(was - old.0));
    }
    let to = to.clamp(new.0, new.1);
    *at = to;
    squeeze(a, dir, (old.0, was), (new.0, to), mins);
    squeeze(b, dir, (was, old.1), (to, new.1), mins);
}

/// Move the divider at `path` towards `want`. A zone reaching its minimum pushes
/// the next divider beyond it, and so on up to the area's edge. Returns where it ended.
fn shift(root: &mut Abs, area: Rect, path: &[bool], want: i32, mins: &[(i32, i32)]) -> i32 {
    let Some((Abs::Split { dir, at, a, b, .. }, r)) = locate(root, area, path) else { return want };
    let (dir, at) = (*dir, *at);
    let (need_a, need_b) = (need(a, dir, mins), need(b, dir, mins));
    let (lo, len) = span_of(r, dir);
    let hi = lo + len;
    // Each side keeps its minimum, or its size now if it is already smaller, and
    // the ratio clamp's tenth. When the divider beyond moves, this split grows, so
    // the side keeps a ninth of the rest to still be a tenth of it.
    let to = if want < at {
        let own = need_a.min(at - lo);
        let mut floor = lo + own.max(tenth(len));
        if want < floor {
            if let Some(k) = bound(root, path, dir, true) {
                let keep = own.max(hi.saturating_sub(want).saturating_add(8) / 9);
                let start = shift(root, area, &path[..k], want.saturating_sub(keep), mins);
                floor = start + own.max(tenth(hi - start));
            }
        }
        want.max(floor).min(at)
    } else if want > at {
        let own = need_b.min(hi - at);
        let mut ceiling = hi - own.max(tenth(len));
        if want > ceiling {
            if let Some(k) = bound(root, path, dir, false) {
                let keep = own.max(want.saturating_sub(lo).saturating_add(8) / 9);
                let end = shift(root, area, &path[..k], want.saturating_add(keep), mins);
                ceiling = end - own.max(tenth(end - lo));
            }
        }
        want.min(ceiling).max(at)
    } else {
        return at;
    };
    // The divider beyond may have moved this split's own start or end.
    let Some((_, r)) = locate(root, area, path) else { return at };
    let (lo, len) = span_of(r, dir);
    let Some(Abs::Split { at: own, a, b, .. }) = locate_mut(root, path) else { return at };
    *own = to;
    squeeze(a, dir, (lo, at), (lo, to), mins);
    squeeze(b, dir, (at, lo + len), (to, lo + len), mins);
    to
}

/// Every split that moved keeps its ratio between 0.1 and 0.9, so the tree draws
/// exactly where the dividers are.
fn valid(node: &Abs, r: Rect) -> bool {
    let Abs::Split { dir, at, was, a, b, .. } = node else { return true };
    let (lo, len) = span_of(r, *dir);
    let fits = (lo, *at, lo + len) == *was || (*at - lo >= tenth(len) && lo + len - *at >= tenth(len));
    let (ra, rb) = cut(r, *dir, *at);
    fits && valid(a, ra) && valid(b, rb)
}

/// Back to ratios. A split that didn't move keeps its ratio as it was, so a layout
/// only changes where it was changed.
fn rebuild(node: &Abs, r: Rect) -> Node {
    match node {
        Abs::Leaf(_) => Node::Leaf(LeafTag::Leaf),
        Abs::Split { dir, ratio, at, was, a, b } => {
            let (lo, len) = span_of(r, *dir);
            let ratio = if (lo, *at, lo + len) == *was || len <= 0 {
                *ratio
            } else {
                ((*at - lo) as f32 / len as f32).clamp(LOW, HIGH)
            };
            let (ra, rb) = cut(r, *dir, divider_at(r, *dir, ratio));
            Node::Split { split: *dir, ratio, a: Box::new(rebuild(a, ra)), b: Box::new(rebuild(b, rb)) }
        }
    }
}

/// Move `edge` by `delta_px` within `area`. `mins` is each leaf's smallest (width,
/// height), in leaf order; a leaf never goes below `MIN_ZONE_PX` either. When the
/// ratio clamp and the minimums leave less room than asked, the move is cut short.
pub fn solve(tree: &Node, area: Rect, edge: &Edge, delta_px: i32, mins: &[(i32, i32)]) -> Solved {
    // No move goes further than the area is long, so a huge delta is the same as that.
    let reach = area.w.max(area.h).max(0);
    let delta_px = delta_px.clamp(-reach, reach);
    let base = build(tree, area, &mut 0);
    let attempt = |delta: i32| {
        let mut abs = base.clone();
        let Some((Abs::Split { at, .. }, _)) = locate(&abs, area, &edge.path) else { return None };
        let at = *at;
        let to = shift(&mut abs, area, &edge.path, at.saturating_add(delta), mins);
        valid(&abs, area).then_some((abs, to - at))
    };
    let mut best = attempt(delta_px);
    if best.is_none() {
        // The largest part of the move that keeps every ratio in range.
        let (mut fits, mut fails) = (0, delta_px.abs());
        while fails - fits > 1 {
            let mid = (fits + fails) / 2;
            match attempt(mid * delta_px.signum()) {
                Some(found) => {
                    fits = mid;
                    best = Some(found);
                }
                None => fails = mid,
            }
        }
    }
    let (tree, applied_delta) = match best {
        Some((abs, applied)) if applied != 0 => (rebuild(&abs, area), applied),
        _ => (tree.clone(), 0),
    };
    Solved { cells: zones::rects(&tree, area, 0), tree, applied_delta }
}

/// The split above `leaf` whose divider is its `side`, or None at the area's edge.
fn leaf_side(tree: &Node, leaf: usize, side: Side) -> Option<Vec<bool>> {
    let (dir, second) = match side {
        Side::Left => (Dir::Vertical, true),
        Side::Right => (Dir::Vertical, false),
        Side::Top => (Dir::Horizontal, true),
        Side::Bottom => (Dir::Horizontal, false),
    };
    let (mut node, mut index, mut path, mut found) = (tree, leaf, Vec::new(), None);
    while let Node::Split { split, a, b, .. } = node {
        let first = zones::leaf_count(a);
        let into_b = index >= first;
        if *split == dir && into_b == second {
            found = Some(path.clone());
        }
        path.push(into_b);
        (node, index) = if into_b { (&**b, index - first) } else { (&**a, index) };
    }
    found.filter(|_| index == 0)
}

/// The bounding box of some zones.
fn cover(cells: &[Rect], leaves: &[usize]) -> Option<Rect> {
    leaves.iter().map(|i| cells.get(*i).copied()).collect::<Option<Vec<_>>>()?.into_iter().reduce(zones::span)
}

/// The divider along `side` of the zones a window covers. None at the area's edge,
/// or when that side runs along more than one divider.
pub fn edge_of(tree: &Node, area: Rect, leaves: &[usize], side: Side) -> Option<Edge> {
    let cells = zones::rects(tree, area, 0);
    let s = cover(&cells, leaves)?;
    let on = |c: Rect| match side {
        Side::Left => c.x == s.x,
        Side::Right => c.x + c.w == s.x + s.w,
        Side::Top => c.y == s.y,
        Side::Bottom => c.y + c.h == s.y + s.h,
    };
    let mut found: Option<Vec<bool>> = None;
    for leaf in leaves.iter().copied().filter(|i| on(cells[*i])) {
        let path = leaf_side(tree, leaf, side)?;
        if found.as_ref().is_some_and(|f| *f != path) {
            return None;
        }
        found = Some(path);
    }
    found.map(|path| Edge { path })
}

/// Move `edge` so the zones `own` covers get `fraction` of the span from their far
/// side to the far side of `other`, the zones across that divider from them, along
/// its axis. Through `solve`, so the minimums and the ratio clamp still hold. None
/// when `edge` isn't a divider of the tree or doesn't lie between the two.
pub fn share(
    tree: &Node,
    area: Rect,
    edge: &Edge,
    (own, other): (&[usize], &[usize]),
    fraction: f32,
    mins: &[(i32, i32)],
) -> Option<Solved> {
    let d = zones::dividers(tree, area).into_iter().find(|d| d.path == edge.path)?;
    let at = divider_at(d.area, d.dir, d.ratio);
    let cells = zones::rects(tree, area, 0);
    let (own, other) = (span_of(cover(&cells, own)?, d.dir), span_of(cover(&cells, other)?, d.dir));
    let (own_end, other_end) = (own.0 + own.1, other.0 + other.1);
    let target = if own_end == at && other.0 == at {
        own.0 + ((other_end - own.0) as f32 * fraction).round() as i32
    } else if other_end == at && own.0 == at {
        own_end - ((own_end - other.0) as f32 * fraction).round() as i32
    } else {
        return None;
    };
    Some(solve(tree, area, edge, target - at, mins))
}

/// The default grow and shrink step for a group `len` pixels along the axis.
pub fn step(len: i32) -> i32 {
    ((len as f32 * STEP_SHARE).round() as i32).max(MIN_STEP_PX)
}

/// Grow the zones a window covers into a neighbour by `step_px`, or the default step.
/// The edge towards the bigger neighbour goes first, side to side before up and down;
/// one that can't move gives way to the next. None when no edge can move.
pub fn grow(tree: &Node, area: Rect, leaves: &[usize], step_px: Option<i32>, mins: &[(i32, i32)]) -> Option<Solved> {
    resize(tree, area, leaves, step_px, mins, true)
}

/// Shrink them by the same rules, giving the room to that neighbour.
pub fn shrink(tree: &Node, area: Rect, leaves: &[usize], step_px: Option<i32>, mins: &[(i32, i32)]) -> Option<Solved> {
    resize(tree, area, leaves, step_px, mins, false)
}

fn resize(
    tree: &Node,
    area: Rect,
    leaves: &[usize],
    step_px: Option<i32>,
    mins: &[(i32, i32)],
    out: bool,
) -> Option<Solved> {
    let dividers = zones::dividers(tree, area);
    // How much room lies beyond a side's divider: the neighbour's size along the axis.
    let room = |edge: &Edge, side: Side| {
        let d = dividers.iter().find(|d| d.path == edge.path)?;
        let at = divider_at(d.area, d.dir, d.ratio);
        let (lo, len) = span_of(d.area, d.dir);
        Some(if matches!(side, Side::Right | Side::Bottom) { lo + len - at } else { at - lo })
    };
    for sides in [[Side::Right, Side::Left], [Side::Bottom, Side::Top]] {
        let mut found: Vec<(Side, Edge, i32)> = sides
            .into_iter()
            .filter_map(|side| {
                let edge = edge_of(tree, area, leaves, side)?;
                let room = room(&edge, side)?;
                Some((side, edge, room))
            })
            .collect();
        found.sort_by_key(|(_, _, room)| std::cmp::Reverse(*room));
        for (side, edge, _) in found {
            let len = if matches!(side, Side::Left | Side::Right) { area.w } else { area.h };
            let step = step_px.unwrap_or_else(|| step(len)).max(1);
            let outward = if matches!(side, Side::Right | Side::Bottom) { step } else { -step };
            let solved = solve(tree, area, &edge, if out { outward } else { -outward }, mins);
            if solved.applied_delta != 0 {
                return Some(solved);
            }
        }
    }
    None
}

/// Give the zones a window covers about two thirds of the group along the layout's
/// main split, within the minimums. The room left keeps its split between the two
/// sides. The stash gives the old tree back.
pub fn priority(tree: &Node, area: Rect, leaves: &[usize], mins: &[(i32, i32)]) -> (Node, Stash) {
    let stash = Stash { tree: tree.clone() };
    let Node::Split { split: dir, .. } = tree else { return (tree.clone(), stash) };
    let (sides, (a0, len)) = match dir {
        Dir::Vertical => ([Side::Right, Side::Left], span_of(area, Dir::Vertical)),
        Dir::Horizontal => ([Side::Bottom, Side::Top], span_of(area, Dir::Horizontal)),
    };
    let Some(now) = cover(&zones::rects(tree, area, 0), leaves) else { return (tree.clone(), stash) };
    let (s0, slen) = span_of(now, *dir);
    let (before, after) = (s0 - a0, a0 + len - (s0 + slen));
    let want = (len as f32 * PRIORITY_SHARE).round() as i32;
    let free = len - want;
    let lead = if before + after > 0 { (free as i64 * before as i64 / (before + after) as i64) as i32 } else { 0 };
    let (start, end) = (a0 + lead, a0 + lead + want);
    let mut out = tree.clone();
    for side in sides {
        let Some(edge) = edge_of(&out, area, leaves, side) else { continue };
        let Some(r) = cover(&zones::rects(&out, area, 0), leaves) else { continue };
        let (r0, rlen) = span_of(r, *dir);
        let delta = if side == sides[0] { end - (r0 + rlen) } else { start - r0 };
        out = solve(&out, area, &edge, delta, mins).tree;
    }
    (out, stash)
}

/// The tree as it was before Priority, ratios and all.
pub fn restore(stash: Stash) -> Node {
    stash.tree
}

/// Each leaf's minimum for `solve`: the largest minimum (width, height) of the
/// visible frames of the windows covering it, plus what the gap takes from it. A
/// window over several leaves shares its minimum between them in proportion to
/// their size now. A leaf with no window gets 0, which `solve` raises to `MIN_ZONE_PX`.
pub fn leaf_mins(tree: &Node, area: Rect, gap: i32, windows: &[(Vec<usize>, (i32, i32))]) -> Vec<(i32, i32)> {
    let cells = zones::rects(tree, area, 0);
    let zones = zones::rects(tree, area, gap);
    let mut out = vec![(0, 0); cells.len()];
    let share = |need: i32, part: i32, whole: i32| {
        let whole = i64::from(whole.max(1));
        ((i64::from(need) * i64::from(part) + whole - 1) / whole) as i32
    };
    for (leaves, (w, h)) in windows {
        let (Some(cell), Some(zone)) = (cover(&cells, leaves), cover(&zones, leaves)) else { continue };
        let (w, h) = (w + cell.w - zone.w, h + cell.h - zone.h);
        for &i in leaves {
            let c = cells[i];
            out[i] = (out[i].0.max(share(w, c.w, cell.w)), out[i].1.max(share(h, c.h, cell.h)));
        }
    }
    out
}

/// A changed layout keeps its template while its tree is still the template's,
/// and is Custom otherwise (decision 1). Either way it no longer follows a library layout.
pub fn relabel(template: Template, tree: Node) -> ZoneLayout {
    let same = zones::template_tree(template).is_some_and(|t| t == tree);
    ZoneLayout { template: if same { template } else { Template::Custom }, custom: None, tree }
}

/// Zones `lo` to `hi`, when they are one part of the tree, cut in two: `part` of it
/// on `side` for a dropped window, and the rest beside it as one zone. The new tree,
/// and the zone numbers of the window's part and of the rest. None when the zones
/// aren't one part of the tree, or the tree would be past the editor's limits.
pub fn split_part(tree: &Node, (lo, hi): (usize, usize), part: f32, side: Side) -> Option<(Node, usize, usize)> {
    let (split, ratio, first) = match side {
        Side::Left => (Dir::Vertical, part, true),
        Side::Right => (Dir::Vertical, 1.0 - part, false),
        Side::Top => (Dir::Horizontal, part, true),
        Side::Bottom => (Dir::Horizontal, 1.0 - part, false),
    };
    let leaf = || Box::new(Node::Leaf(LeafTag::Leaf));
    let two = Node::Split { split, ratio, a: leaf(), b: leaf() };
    let tree = replace_part(tree, 0, (lo, hi), &two)?;
    if !zones::within_limits(&tree) {
        return None;
    }
    Some(if first { (tree, lo, lo + 1) } else { (tree, lo + 1, lo) })
}

/// `tree` with the part holding exactly zones `lo` to `hi` replaced by `with`. `offset`
/// is the number of `node`'s first zone.
fn replace_part(node: &Node, offset: usize, (lo, hi): (usize, usize), with: &Node) -> Option<Node> {
    if offset == lo && offset + zones::leaf_count(node) == hi + 1 {
        return Some(with.clone());
    }
    let Node::Split { split, ratio, a, b } = node else { return None };
    let (split, ratio, first) = (*split, *ratio, offset + zones::leaf_count(a));
    if hi < first {
        Some(Node::Split { split, ratio, a: Box::new(replace_part(a, offset, (lo, hi), with)?), b: b.clone() })
    } else if lo >= first {
        Some(Node::Split { split, ratio, a: a.clone(), b: Box::new(replace_part(b, first, (lo, hi), with)?) })
    } else {
        None
    }
}

/// `n` equal parts along `dir`, each made by `part`.
fn equal_parts(dir: Dir, n: usize, part: &dyn Fn() -> Node) -> Node {
    if n <= 1 {
        return part();
    }
    Node::Split { split: dir, ratio: 1.0 / n as f32, a: Box::new(part()), b: Box::new(equal_parts(dir, n - 1, part)) }
}

/// A grid of equal zones, `cols` across and `rows` down, numbered column by column.
fn grid(cols: usize, rows: usize) -> Node {
    equal_parts(Dir::Vertical, cols, &|| equal_parts(Dir::Horizontal, rows, &|| Node::Leaf(LeafTag::Leaf)))
}

/// The layout Tile gives a screen with none, for `n` windows: two columns, Main
/// and stack, the 2x2 grid, then 3x2 and 3x3. One window or none needs none, and
/// nine is as many as Tile takes. Only Main + stack and Grid are templates; the
/// others are Custom, as no template has their tree.
pub fn pick_for_count(n: usize) -> Option<ZoneLayout> {
    let tree = match n {
        0 | 1 => return None,
        2 => grid(2, 1),
        3 => return ZoneLayout::of(Template::MainStack),
        4 => return ZoneLayout::of(Template::Grid),
        5 | 6 => grid(3, 2),
        _ => grid(3, 3),
    };
    Some(ZoneLayout { template: Template::Custom, custom: None, tree })
}

/// How far two windows may overlap and still lie either side of a cut, as frames do.
const OVERLAP_PX: i32 = 16;
/// A cut this close to a quarter, a third, a half, two thirds or three quarters goes there.
const SNAP_PX: i32 = 12;

/// The split tree a screen's windows line up into, one zone each, inside `area`.
/// Guillotine cuts: a straight cut across the whole part with windows on both sides
/// and none across it, side by side before top and bottom, then the same in each
/// side. A cut sits in the middle of the room between the windows, and on the
/// editor's quarter or third marks when it's within a few pixels of one. None
/// when windows overlap, no cut separates them, the tree passes the editor's limits,
/// or the windows' centres don't each fall in a zone of their own.
pub fn from_rects(rects: &[Rect], area: Rect) -> Option<Node> {
    if rects.is_empty() || rects.len() > zones::MAX_ZONES || area.w <= 0 || area.h <= 0 {
        return None;
    }
    let tree = cut_up(rects.to_vec(), area)?;
    let deepest = (0..rects.len()).filter_map(|i| zones::depth_of(&tree, i)).max().unwrap_or(0);
    let cells = zones::rects(&tree, area, 0);
    let mut held = vec![false; cells.len()];
    for r in rects {
        let zone = zones::zone_at(&cells, r.x + r.w / 2, r.y + r.h / 2)?;
        if std::mem::replace(&mut held[zone], true) {
            return None;
        }
    }
    (deepest <= zones::MAX_DEPTH).then_some(tree)
}

fn cut_up(mut rects: Vec<Rect>, area: Rect) -> Option<Node> {
    if rects.len() == 1 {
        return Some(Node::Leaf(LeafTag::Leaf));
    }
    for dir in [Dir::Vertical, Dir::Horizontal] {
        rects.sort_by_key(|r| span_of(*r, dir));
        let mut reach = i32::MIN;
        for k in 1..rects.len() {
            let (start, len) = span_of(rects[k - 1], dir);
            reach = reach.max(start + len);
            let next = span_of(rects[k], dir).0;
            if reach > next + OVERLAP_PX {
                continue;
            }
            let ratio = ratio_at((reach + next) / 2, area, dir);
            let (ra, rb) = cut(area, dir, divider_at(area, dir, ratio));
            let rest = rects.split_off(k);
            let (a, b) = (cut_up(rects, ra)?, cut_up(rest, rb)?);
            return Some(Node::Split { split: dir, ratio, a: Box::new(a), b: Box::new(b) });
        }
    }
    None
}

/// The ratio of a cut at `at`, on the nearest mark when it's close to one.
fn ratio_at(at: i32, area: Rect, dir: Dir) -> f32 {
    const MARKS: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];
    let (lo, len) = span_of(area, dir);
    let off = |m: f32| (lo + (len as f32 * m).round() as i32 - at).abs();
    let mark = MARKS.into_iter().filter(|m| off(*m) <= SNAP_PX).min_by_key(|m| off(*m));
    mark.unwrap_or(((at - lo) as f32 / len as f32).clamp(LOW, HIGH))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0, y: 0, w: 1800, h: 1000 };

    fn columns() -> Node {
        zones::template_tree(Template::Columns).unwrap()
    }

    fn xs(cells: &[Rect]) -> Vec<(i32, i32)> {
        cells.iter().map(|c| (c.x, c.w)).collect()
    }

    /// Every split's ratio as stored, before `zones` clamps it.
    fn stored(n: &Node) -> Vec<f32> {
        match n {
            Node::Split { ratio, a, b, .. } => [vec![*ratio], stored(a), stored(b)].concat(),
            Node::Leaf(_) => Vec::new(),
        }
    }

    fn edge(path: &[bool]) -> Edge {
        Edge { path: path.to_vec() }
    }

    #[test]
    fn a_cut_gives_the_window_its_part_on_its_side() {
        // A third on the right of the middle column: the rest keeps the left of it.
        let (tree, at, rest) = split_part(&columns(), (1, 1), 1.0 / 3.0, Side::Right).unwrap();
        let cells = zones::rects(&tree, AREA, 0);
        assert_eq!((at, rest), (2, 1));
        assert_eq!(xs(&cells), vec![(0, 600), (600, 400), (1000, 200), (1200, 600)]);
        // The whole screen, top half: two zones.
        let (tree, at, rest) = split_part(&columns(), (0, 2), 0.5, Side::Top).unwrap();
        let cells = zones::rects(&tree, AREA, 0);
        assert_eq!((at, rest, cells[at].h, cells[rest].y), (0, 1, 500, 500));
        // The first two columns aren't one part of the tree.
        assert_eq!(split_part(&columns(), (0, 1), 0.5, Side::Left), None);
    }

    #[test]
    fn a_divider_moves_and_the_next_one_stays() {
        let s = solve(&columns(), AREA, &edge(&[]), 100, &[]);
        assert_eq!(s.applied_delta, 100);
        assert_eq!(xs(&s.cells), vec![(0, 700), (700, 500), (1200, 600)]);
        // Grow takes the right-hand divider on a tie, by 5% of the width.
        let g = grow(&columns(), AREA, &[1], None, &[]).unwrap();
        assert_eq!(xs(&g.cells), vec![(0, 600), (600, 690), (1290, 510)]);
    }

    #[test]
    fn a_share_puts_the_divider_at_that_part_of_the_two_zones() {
        // Three quarters of the left two columns, asked from either side.
        let left = share(&columns(), AREA, &edge(&[]), (&[0], &[1]), 0.75, &[]).unwrap();
        assert_eq!(xs(&left.cells), vec![(0, 900), (900, 300), (1200, 600)]);
        let right = share(&columns(), AREA, &edge(&[]), (&[1], &[0]), 0.25, &[]).unwrap();
        assert_eq!(right.cells, left.cells);
        // The middle window can't go below 400 px, so the next divider gives way.
        let pushed = share(&columns(), AREA, &edge(&[]), (&[0], &[1]), 0.75, &[(0, 0), (400, 0), (0, 0)]).unwrap();
        assert_eq!(xs(&pushed.cells), vec![(0, 900), (900, 400), (1300, 500)]);
        // That divider isn't between the two.
        assert_eq!(share(&columns(), AREA, &edge(&[true]), (&[0], &[1]), 0.5, &[]), None);
    }

    #[test]
    fn a_zone_at_its_minimum_pushes_the_divider_beyond_it() {
        // The middle window can't go below 300 px, so the first divider gives way.
        let mins = [(0, 0), (300, 0), (0, 0)];
        let s = solve(&columns(), AREA, &edge(&[true]), -500, &mins);
        assert_eq!(s.applied_delta, -500);
        assert_eq!(xs(&s.cells), vec![(0, 400), (400, 300), (700, 1100)]);
    }

    #[test]
    fn the_push_stops_at_the_area_edge() {
        let mins = [(400, 0), (300, 0), (0, 0)];
        let s = solve(&columns(), AREA, &edge(&[true]), -1100, &mins);
        assert_eq!(s.applied_delta, -500);
        assert_eq!(xs(&s.cells), vec![(0, 400), (400, 300), (700, 1100)]);
        // Already there, so nothing moves at all.
        let again = solve(&s.tree, AREA, &edge(&[true]), -50, &mins);
        assert_eq!((again.applied_delta, again.tree), (0, s.tree));
        // With no minimum the ratio clamp still holds the first zone at a tenth.
        let free = solve(&columns(), AREA, &edge(&[]), -1000, &[]);
        assert_eq!(free.cells[0].w, 180);
        // Deltas and steps far past the area don't overflow, and stop at the same place.
        assert_eq!(solve(&columns(), AREA, &edge(&[]), i32::MIN, &[]).cells, free.cells);
        let g = grow(&columns(), AREA, &[2], Some(i32::MAX), &[]).unwrap();
        // The first zone at a tenth of the width, the middle one at a tenth of the rest.
        assert_eq!(g.cells[2].w, 1800 - 180 - 162);
    }

    #[test]
    fn splits_elsewhere_keep_their_place_and_ratio() {
        let grid = zones::template_tree(Template::Grid).unwrap();
        let s = solve(&grid, AREA, &edge(&[true]), 120, &[]);
        let before = zones::rects(&grid, AREA, 0);
        assert_eq!(s.applied_delta, 120);
        assert_eq!(&s.cells[..2], &before[..2]);
        let (Node::Split { a: old, .. }, Node::Split { a: new, .. }) = (&grid, &s.tree) else { panic!() };
        assert_eq!(old, new);
        // Moving the outer divider leaves the inner one where it is on screen.
        let s = solve(&columns(), AREA, &edge(&[]), -200, &[]);
        assert_eq!(s.cells[2].x, 1200);
    }

    #[test]
    fn cells_always_tile_the_area() {
        let trees = [Template::Columns, Template::Rows, Template::Grid, Template::Priority]
            .map(|t| zones::template_tree(t).unwrap());
        let area = Rect { x: -1440, y: 200, w: 1440, h: 2520 };
        for tree in &trees {
            let n = zones::leaf_count(tree);
            let mins: Vec<(i32, i32)> = (0..n).map(|i| (150 + 90 * i as i32, 200)).collect();
            for d in zones::dividers(tree, area) {
                for delta in (-1500..=1500).step_by(125) {
                    let s = solve(tree, area, &Edge { path: d.path.clone() }, delta, &mins);
                    assert!(stored(&s.tree).iter().all(|r| (0.1..=0.9).contains(r)), "{tree:?} {delta}");
                    let filled: i64 = s.cells.iter().map(|c| i64::from(c.w) * i64::from(c.h)).sum();
                    assert_eq!(filled, i64::from(area.w) * i64::from(area.h), "{tree:?} {delta}");
                    for (i, c) in s.cells.iter().enumerate() {
                        assert!(c.x >= area.x && c.y >= area.y && c.x + c.w <= area.x + area.w);
                        assert!(c.y + c.h <= area.y + area.h);
                        assert!(c.w >= mins[i].0 && c.h >= mins[i].1, "{tree:?} {delta}: zone {i} is {c:?}");
                    }
                    for (cells, gap) in [(s.cells.clone(), 0), (zones::rects(&s.tree, area, 8), 8)] {
                        assert!(cells.iter().all(|r| r.x >= area.x + gap && r.x + r.w <= area.x + area.w - gap));
                        assert!(cells.iter().all(|r| r.y >= area.y + gap && r.y + r.h <= area.y + area.h - gap));
                        for (i, a) in cells.iter().enumerate() {
                            let apart = |b: &Rect| {
                                a.x + a.w + gap <= b.x
                                    || b.x + b.w + gap <= a.x
                                    || a.y + a.h + gap <= b.y
                                    || b.y + b.h + gap <= a.y
                            };
                            assert!(cells[i + 1..].iter().all(apart), "{tree:?} {delta}: zone {i} overlaps");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn priority_gives_two_thirds_and_restore_gives_it_back() {
        for leaf in 0..3 {
            let (tree, stash) = priority(&columns(), AREA, &[leaf], &[]);
            let w = zones::rects(&tree, AREA, 0)[leaf].w;
            assert!((w - 1200).abs() <= 2, "leaf {leaf}: {w}");
            assert_eq!(restore(stash), columns());
        }
        assert_eq!(relabel(Template::Columns, columns()).template, Template::Columns);
        let (tree, _) = priority(&columns(), AREA, &[1], &[]);
        assert_eq!(relabel(Template::Columns, tree).template, Template::Custom);
    }

    #[test]
    fn tile_picks_a_layout_with_room_for_every_window() {
        let area = Rect { x: -1440, y: 200, w: 1440, h: 2520 };
        assert!(pick_for_count(0).is_none() && pick_for_count(1).is_none());
        for (n, zones) in [(2, 2), (3, 3), (4, 4), (5, 6), (6, 6), (7, 9), (8, 9), (9, 9)] {
            let layout = pick_for_count(n).unwrap();
            let cells = zones::rects(&layout.tree, area, 0);
            assert_eq!(cells.len(), zones, "{n} windows");
            let filled: i64 = cells.iter().map(|c| i64::from(c.w) * i64::from(c.h)).sum();
            assert_eq!(filled, i64::from(area.w) * i64::from(area.h), "{n} windows leave holes or overlap");
            assert!(cells.iter().all(|c| c.x >= area.x && c.y >= area.y && c.x + c.w <= area.x + area.w));
            assert!(cells.iter().all(|c| c.y + c.h <= area.y + area.h && c.w > 0 && c.h > 0));
            assert_eq!(layout.template == Template::Custom, n != 3 && n != 4, "{n} windows");
        }
        assert_eq!(pick_for_count(3).unwrap().template, Template::MainStack);
        assert_eq!(pick_for_count(4).unwrap().template, Template::Grid);
    }

    #[test]
    fn windows_line_up_into_the_zones_they_stand_in() {
        // Windows as the zones would place them with an 8 px gap, on a screen left of the main one.
        let area = Rect { x: -1920, y: 0, w: 1920, h: 1040 };
        let main_stack = zones::template_tree(Template::MainStack).unwrap();
        for tree in [grid(2, 1), main_stack, grid(3, 2)] {
            let windows = zones::rects(&tree, area, 8);
            assert_eq!(from_rects(&windows, area), Some(tree.clone()), "{tree:?}");
            // In another order, and a few pixels off the marks, it's the same tree.
            let mut nudged: Vec<Rect> = windows.iter().rev().map(|r| Rect { x: r.x + 3, w: r.w - 5, ..*r }).collect();
            nudged.rotate_left(1);
            assert_eq!(from_rects(&nudged, area), Some(tree.clone()), "{tree:?} nudged");
        }
        // Off the marks, a cut sits in the middle of the room between the windows.
        let two = [Rect { x: 0, y: 0, w: 700, h: 1000 }, Rect { x: 740, y: 0, w: 1060, h: 1000 }];
        let Some(Node::Split { split: Dir::Vertical, ratio, .. }) = from_rects(&two, AREA) else { panic!() };
        assert_eq!(divider_at(AREA, Dir::Vertical, ratio), 720);
        // Where two marks are near, the nearer one wins.
        assert_eq!(ratio_at(19, Rect { x: 0, y: 0, w: 60, h: 10 }, Dir::Vertical), 1.0 / 3.0);
        // A pinwheel has no straight cut across it, and overlapping windows have none either.
        let pinwheel = [
            Rect { x: 0, y: 0, w: 1200, h: 400 },
            Rect { x: 1200, y: 0, w: 600, h: 700 },
            Rect { x: 600, y: 700, w: 1200, h: 300 },
            Rect { x: 0, y: 400, w: 600, h: 600 },
        ];
        assert_eq!(from_rects(&pinwheel, AREA), None);
        let overlapping = [Rect { x: 0, y: 0, w: 1000, h: 600 }, Rect { x: 800, y: 400, w: 1000, h: 600 }];
        assert_eq!(from_rects(&overlapping, AREA), None);
        // One window is one zone; none is nothing.
        assert_eq!(from_rects(&[AREA], AREA), Some(Node::Leaf(LeafTag::Leaf)));
        assert_eq!(from_rects(&[], AREA), None);
    }
}
