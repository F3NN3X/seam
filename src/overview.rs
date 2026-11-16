//! The overview's rules, with no Win32 in them: which windows a search keeps and in
//! what order, where a screen's tiles go, one group per virtual desktop, where the
//! arrow keys go from a tile, and what a drop lands on. The tray's `overview_win`
//! draws them and moves the windows.

use crate::zones::Rect;
use std::sync::atomic::{AtomicU64, Ordering};

/// How well a search matches a window, higher is better, or None when some word of
/// it is in neither the title nor the app's name. Case doesn't count. Each word scores
/// most at the start of the app's name, then at the start of a word in the title,
/// then anywhere in either.
pub fn score(query: &str, title: &str, app: &str) -> Option<u32> {
    let (title, app) = (title.to_lowercase(), app.to_lowercase());
    let mut total = 0;
    for word in query.to_lowercase().split_whitespace() {
        let starts_word = |text: &str| {
            text.match_indices(word).any(|(at, _)| at == 0 || !text[..at].ends_with(char::is_alphanumeric))
        };
        total += if app.starts_with(word) {
            40
        } else if starts_word(&title) {
            30
        } else if starts_word(&app) {
            25
        } else if app.contains(word) {
            20
        } else if title.contains(word) {
            10
        } else {
            return None;
        };
    }
    Some(total)
}

/// The windows a search keeps, best first. Equal scores keep the order they came in,
/// which is the stacking order, so the window used last leads. An empty search keeps all.
pub fn rank(query: &str, windows: &[(&str, &str)]) -> Vec<usize> {
    let mut kept: Vec<(usize, u32)> =
        windows.iter().enumerate().filter_map(|(i, (title, app))| Some((i, score(query, title, app)?))).collect();
    kept.sort_by_key(|&(i, s)| (std::cmp::Reverse(s), i));
    kept.into_iter().map(|(i, _)| i).collect()
}

/// Sizes for laying out tiles, in pixels at the screen's DPI.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub gap: i32,
    /// The height of a group's label row, above its tiles.
    pub label: i32,
    /// The height of the caption under a tile's preview.
    pub caption: i32,
    /// No tile gets wider than this, so a few windows don't fill a screen.
    pub max_w: i32,
    /// The preview's shape, width to height.
    pub aspect: (i32, i32),
}

/// One virtual desktop's tiles on a screen: its label row and a tile per window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub label: Rect,
    pub tiles: Vec<Rect>,
}

/// The tile width that fits `counts` in `area` with `columns` to a row, or 0.
fn width_for(area: Rect, counts: &[usize], m: &Metrics, columns: i32) -> i32 {
    let across = (area.w - (columns - 1) * m.gap) / columns;
    let rows: i32 = counts.iter().map(|&n| (n as i32 + columns - 1) / columns).sum();
    let groups = counts.len() as i32;
    // Each group has its label and a gap below it, and the rows a gap between them.
    let fixed = groups * (m.label + m.gap) + (rows - groups).max(0) * m.gap + (groups - 1).max(0) * m.gap;
    let tile_h = (area.h - fixed) / rows.max(1);
    let preview_h = tile_h - m.caption;
    let down = preview_h * m.aspect.0 / m.aspect.1.max(1);
    across.min(down).min(m.max_w).max(0)
}

/// Lay out the groups of a screen in `area`, top down, with one column count for all
/// of them: the one that gives the widest tiles, the most columns of those. The rows are centred across the area,
/// a short last row starting at the left. Empty groups get no place. Tiles are as wide
/// as their preview, which keeps `aspect`, and as tall as the preview and its caption.
pub fn layout(area: Rect, counts: &[usize], m: &Metrics) -> Vec<Group> {
    let counts: Vec<usize> = counts.iter().copied().filter(|&n| n > 0).collect();
    let Some(&most) = counts.iter().max() else { return Vec::new() };
    let best = (1..=most as i32).map(|c| (width_for(area, &counts, m, c), c)).max_by_key(|&(w, c)| (w, c));
    let Some((w, columns)) = best else { return Vec::new() };
    let preview_h = w * m.aspect.1 / m.aspect.0.max(1);
    let tile_h = preview_h + m.caption;
    let used = counts.iter().map(|&n| n.min(columns as usize) as i32).max().unwrap_or(1);
    let block = used * w + (used - 1) * m.gap;
    let left = area.x + (area.w - block).max(0) / 2;
    let mut y = area.y;
    let mut groups = Vec::new();
    for n in counts {
        let label = Rect { x: left, y, w: block, h: m.label };
        y += m.label + m.gap;
        let tiles = (0..n as i32)
            .map(|i| {
                let (col, row) = (i % columns, i / columns);
                Rect { x: left + col * (w + m.gap), y: y + row * (tile_h + m.gap), w, h: tile_h }
            })
            .collect();
        let rows = (n as i32 + columns - 1) / columns;
        y += rows * (tile_h + m.gap);
        groups.push(Group { label, tiles });
    }
    groups
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The tile the arrow goes to from `from`, by the tiles' places on the desk, so it goes
/// on to the next screen past the last tile. A tile level with this one wins, the
/// nearest first; without one, the nearest by distance along plus twice across.
pub fn step(tiles: &[Rect], from: usize, dir: Dir) -> Option<usize> {
    let here = *tiles.get(from)?;
    let centre = |r: &Rect| (r.x + r.w / 2, r.y + r.h / 2);
    let (cx, cy) = centre(&here);
    // Distance along the arrow, distance across it, and whether the spans across overlap.
    let measure = |r: &Rect| {
        let (x, y) = centre(r);
        let (along, across, level) = match dir {
            Dir::Left => (cx - x, (y - cy).abs(), r.y < here.y + here.h && here.y < r.y + r.h),
            Dir::Right => (x - cx, (y - cy).abs(), r.y < here.y + here.h && here.y < r.y + r.h),
            Dir::Up => (cy - y, (x - cx).abs(), r.x < here.x + here.w && here.x < r.x + r.w),
            Dir::Down => (y - cy, (x - cx).abs(), r.x < here.x + here.w && here.x < r.x + r.w),
        };
        (along > 0).then_some((along, across, level))
    };
    let ahead: Vec<(usize, (i32, i32, bool))> =
        tiles.iter().enumerate().filter(|(i, _)| *i != from).filter_map(|(i, r)| Some((i, measure(r)?))).collect();
    let level = ahead.iter().filter(|(_, m)| m.2).min_by_key(|(_, m)| (m.0, m.1));
    let any = || ahead.iter().min_by_key(|(_, m)| m.0 + 2 * m.1);
    level.or_else(any).map(|(i, _)| *i)
}

/// What a drop lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// A zone of a screen, by their indexes.
    Zone(usize, usize),
    /// A screen with no zones, by its index.
    Screen(usize),
}

/// What the point is over: a zone, or a screen that has none. A point in the gap
/// between zones is over nothing, as in a drag.
pub fn drop_at(screens: &[(Rect, Vec<Rect>)], (x, y): (i32, i32)) -> Option<Landing> {
    let s = screens.iter().position(|(r, _)| r.contains(x, y))?;
    let zones = &screens[s].1;
    if zones.is_empty() {
        return Some(Landing::Screen(s));
    }
    zones.iter().position(|z| z.contains(x, y)).map(|z| Landing::Zone(s, z))
}

/// Where a window of `size` goes on a screen with no zones: the middle of its work
/// area, no larger than it.
pub fn centred(work: Rect, (w, h): (i32, i32)) -> Rect {
    let (w, h) = (w.min(work.w), h.min(work.h));
    Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h }
}

/// The order desktops show in, by their place in Windows' list (None when unknown), the
/// first being the one in front, which always leads. Unknown ones follow in the order
/// they were met. Returns the order and, for each desktop, its new index.
pub fn desktop_ranks(places: &[Option<usize>]) -> (Vec<usize>, Vec<usize>) {
    let mut ranked: Vec<usize> = (0..places.len()).collect();
    ranked.sort_by_key(|&i| (i != 0, places[i].unwrap_or(usize::MAX), i));
    let mut index = vec![0; places.len()];
    for (new, &old) in ranked.iter().enumerate() {
        index[old] = new;
    }
    (ranked, index)
}

/// The window a desktop jump selects: `tiles` are the shown windows in order, each with its
/// desktop's index, and `by` is 1 for the next desktop and -1 for the previous. Only desktops
/// with a shown window count, in index order, and the jump wraps. It lands on the first
/// window of that desktop. With nothing selected it starts from the edge it moves away from.
pub fn desktop_jump(tiles: &[(usize, usize)], selected: Option<usize>, by: isize) -> Option<usize> {
    let mut groups: Vec<usize> = tiles.iter().map(|&(_, d)| d).collect();
    groups.sort_unstable();
    groups.dedup();
    let here =
        tiles.iter().find(|&&(w, _)| Some(w) == selected).and_then(|&(_, d)| groups.iter().position(|&g| g == d));
    let n = groups.len() as isize;
    let to = match here {
        Some(i) => (i as isize + by).rem_euclid(n.max(1)),
        None if by < 0 => n - 1,
        None => 0,
    };
    let group = *groups.get(usize::try_from(to).ok()?)?;
    tiles.iter().find(|&&(_, d)| d == group).map(|&(w, _)| w)
}

/// What the snapping knows about a drop, read before anything changes.
#[derive(Clone, Debug)]
pub struct DropFacts<'a> {
    pub alive: bool,
    pub elevated: bool,
    /// Smart tiling has the window in a tile, so the move floats it.
    pub tiled_window: bool,
    /// The zone it was dropped on, or None for a screen with no zones.
    pub zone: Option<usize>,
    /// The target screen's zones now, empty when it has none or zones are off.
    pub zones_now: &'a [Rect],
    /// The zone's rect as the overview showed it.
    pub shown: Rect,
    /// Smart tiling arranges the target screen.
    pub target_tiled: bool,
}

/// What a drop does. Every check comes first, so a refused drop changes nothing: the
/// window isn't floated and tiling keeps its tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropPlan {
    Refuse(&'static str),
    Move {
        /// Float the window out of its tile, then move it.
        float_first: bool,
        /// Snap it into this zone.
        snap: Option<usize>,
        /// Put it over a tiled screen's zone, floating, without taking the tile.
        float_over: bool,
    },
}

pub fn plan_drop(f: &DropFacts) -> DropPlan {
    if !f.alive {
        return DropPlan::Refuse("The window closed");
    }
    if f.elevated {
        return DropPlan::Refuse("That window runs as administrator, so Seam can't move it");
    }
    if let Some(z) = f.zone
        && f.zones_now.get(z) != Some(&f.shown)
    {
        return DropPlan::Refuse("The zones changed while the overview was open, so the window stayed");
    }
    let float_over = f.zone.is_some() && f.target_tiled;
    DropPlan::Move { float_first: f.tiled_window, snap: f.zone.filter(|_| !f.target_tiled), float_over }
}

/// What Esc does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    /// A drag is on: it ends with no move, and the overview stays.
    CancelDrag,
    /// The overview closes and this window, the one in front before, gets the focus back.
    Back(usize),
    /// The overview closes.
    Close,
}

/// Esc always closes the overview, but for ending a drag first. The window that was in
/// front gets the focus back only while it still exists; one that closed is skipped.
pub fn escape(dragging: bool, before: Option<usize>, alive: impl Fn(usize) -> bool) -> Escape {
    match before {
        _ if dragging => Escape::CancelDrag,
        Some(w) if alive(w) => Escape::Back(w),
        _ => Escape::Close,
    }
}

/// A virtual desktop's label: this one, or by its name, or its place in Windows' list
/// from 1, or a plain word when Windows' list can't be read.
pub fn desktop_label(current: bool, name: Option<&str>, place: Option<usize>) -> String {
    match (current, name.map(str::trim).filter(|n| !n.is_empty()), place) {
        (true, ..) => "This desktop".to_string(),
        (false, Some(name), _) => name.to_string(),
        (false, None, Some(i)) => format!("Desktop {}", i + 1),
        (false, None, None) => "Another desktop".to_string(),
    }
}

/// What a screen reader hears of the overview: the selected window by app and title, or
/// how many windows are shown when nothing is selected.
pub fn spoken(selected: Option<(&str, &str)>, shown: usize) -> String {
    // One line and no longer than a reader should be made to listen to.
    let line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>();
    match selected {
        Some((app, title)) => match (line(app).as_str(), line(title).as_str()) {
            ("", "") => "Seam overview".to_string(),
            (app, "") => format!("Seam overview: {app}"),
            ("", title) => format!("Seam overview: {title}"),
            (app, title) => format!("Seam overview: {app} - {title}"),
        },
        None => match shown {
            1 => "Seam overview: 1 window".to_string(),
            n => format!("Seam overview: {n} windows"),
        },
    }
}

/// The places where two lists of the same length differ.
pub fn changed<T: PartialEq>(before: &[T], after: &[T]) -> Vec<usize> {
    debug_assert_eq!(before.len(), after.len());
    before.iter().zip(after).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect()
}

/// What a details slice needs repainted.
#[derive(Debug, PartialEq, Eq)]
pub enum Repaint {
    Nothing,
    /// Only these windows' tiles.
    Tiles(Vec<usize>),
    /// The windows shown changed, so the tiles moved.
    All,
}

/// The whole view when the search now keeps other windows or in another order, else just
/// the tiles of the shown windows whose details changed.
pub fn repaint(before: &[usize], after: &[usize], changed: &[usize]) -> Repaint {
    if before != after {
        return Repaint::All;
    }
    let dirty: Vec<usize> = changed.iter().copied().filter(|w| after.contains(w)).collect();
    if dirty.is_empty() { Repaint::Nothing } else { Repaint::Tiles(dirty) }
}

/// How long a request to open may stay unanswered before another is let through, in case
/// the overview's thread died before its windows existed.
pub const OPENING_LIMIT_MS: u64 = 5_000;

/// Set by the tray when it asks for the overview and cleared by the overview's thread once
/// its windows exist, so a second request in between is dropped. Holds the time of the
/// request, in ms from some fixed start, plus one, or 0 when none is out.
#[derive(Debug, Default)]
pub struct Opening(AtomicU64);

impl Opening {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Ask to open at `now`. None when a request is already out and not yet past the limit,
    /// else the token to give `done`.
    pub fn begin(&self, now: u64) -> Option<u64> {
        let out = self.0.load(Ordering::SeqCst);
        if out != 0 && now.saturating_sub(out - 1) < OPENING_LIMIT_MS {
            return None;
        }
        let token = now + 1;
        self.0.compare_exchange(out, token, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| token)
    }

    /// The windows exist, or the thread ended without them. A late call for a request that
    /// timed out leaves a newer request's flag alone.
    pub fn done(&self, token: u64) {
        let _ = self.0.compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub fn is_out(&self) -> bool {
        self.0.load(Ordering::SeqCst) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn an_empty_search_keeps_every_window_in_order() {
        assert_eq!(rank("", &[("Inbox", "Outlook"), ("Notes", "Obsidian")]), vec![0, 1]);
        assert_eq!(rank("   ", &[("a", "b")]), vec![0]);
    }

    #[test]
    fn each_word_must_match_the_title_or_the_app() {
        assert_eq!(score("seam pu", "Seam pull request", "Firefox"), Some(60));
        assert_eq!(score("seam zz", "Seam pull request", "Firefox"), None);
        assert_eq!(score("FIRE", "x", "Firefox"), Some(40));
    }

    #[test]
    fn the_app_name_leads_then_a_title_word_then_anywhere() {
        assert_eq!(score("code", "main.rs", "Code"), Some(40));
        assert_eq!(score("code", "Source code", "Notepad"), Some(30));
        assert_eq!(score("pad", "x", "Note pad"), Some(25));
        assert_eq!(score("pad", "x", "Notepad"), Some(20));
        assert_eq!(score("ode", "Source code", "Notepad"), Some(10));
    }

    #[test]
    fn ranking_puts_the_best_first_and_keeps_order_on_ties() {
        let windows = [("Mail", "Notepad"), ("x", "Mail"), ("Re: mail", "Outlook"), ("Mailbox", "Edge")];
        // The app name first, then the title words, in the order they came.
        assert_eq!(rank("mail", &windows), vec![1, 0, 2, 3]);
        assert_eq!(rank("box", &windows), vec![3]);
    }

    #[test]
    fn a_word_start_counts_after_punctuation() {
        assert_eq!(score("rs", "main.rs", "Code"), Some(30));
        assert_eq!(score("ain", "main.rs", "Code"), Some(10));
    }

    const M: Metrics = Metrics { gap: 10, label: 20, caption: 30, max_w: 400, aspect: (16, 10) };

    #[test]
    fn a_few_windows_get_the_widest_tiles_allowed_in_one_row() {
        let groups = layout(r(0, 0, 2000, 1000), &[3], &M);
        assert_eq!(groups.len(), 1);
        let tiles = &groups[0].tiles;
        assert_eq!(tiles.len(), 3);
        // 400 wide, 250 for the preview and 30 for the caption, centred across 2000.
        assert_eq!(tiles[0], r(390, 30, 400, 280));
        assert_eq!(tiles[2], r(1210, 30, 400, 280));
        assert_eq!(groups[0].label, r(390, 0, 1220, 20));
    }

    #[test]
    fn many_windows_wrap_into_rows_that_fit_the_area() {
        let area = r(0, 0, 1000, 800);
        let groups = layout(area, &[30], &M);
        let tiles = &groups[0].tiles;
        assert_eq!(tiles.len(), 30);
        let last = tiles[29];
        assert!(last.y + last.h <= area.h, "{last:?}");
        assert!(tiles.iter().all(|t| t.x >= 0 && t.x + t.w <= area.w));
        // Every tile the same size, none overlapping the next.
        assert!(tiles.iter().all(|t| (t.w, t.h) == (tiles[0].w, tiles[0].h)));
        assert!(tiles[1].x >= tiles[0].x + tiles[0].w);
    }

    #[test]
    fn groups_stack_down_the_screen_with_one_column_count() {
        let groups = layout(r(0, 0, 1000, 1000), &[5, 0, 2], &M);
        // The empty desktop has no place.
        assert_eq!(groups.len(), 2);
        let (a, b) = (&groups[0], &groups[1]);
        let bottom = a.tiles.iter().map(|t| t.y + t.h).max().unwrap();
        assert!(b.label.y >= bottom, "{:?} under {bottom}", b.label);
        assert_eq!(b.tiles[0].y, b.label.y + M.label + M.gap);
        assert_eq!(a.tiles[0].w, b.tiles[0].w);
        assert_eq!(a.tiles[0].x, b.tiles[0].x);
    }

    #[test]
    fn nothing_to_lay_out_gives_no_groups() {
        assert!(layout(r(0, 0, 100, 100), &[], &M).is_empty());
        assert!(layout(r(0, 0, 100, 100), &[0, 0], &M).is_empty());
    }

    #[test]
    fn arrows_move_along_a_row_and_between_rows() {
        // Two rows of three.
        let tiles: Vec<Rect> = (0..6).map(|i| r((i % 3) * 110, (i / 3) * 110, 100, 100)).collect();
        assert_eq!(step(&tiles, 0, Dir::Right), Some(1));
        assert_eq!(step(&tiles, 1, Dir::Left), Some(0));
        assert_eq!(step(&tiles, 1, Dir::Down), Some(4));
        assert_eq!(step(&tiles, 5, Dir::Up), Some(2));
        assert_eq!(step(&tiles, 0, Dir::Left), None);
        assert_eq!(step(&tiles, 0, Dir::Up), None);
    }

    #[test]
    fn down_from_a_full_row_onto_a_short_one_takes_the_nearest() {
        // Three above, one below at the left.
        let tiles = [r(0, 0, 100, 100), r(110, 0, 100, 100), r(220, 0, 100, 100), r(0, 110, 100, 100)];
        assert_eq!(step(&tiles, 2, Dir::Down), Some(3));
        assert_eq!(step(&tiles, 3, Dir::Up), Some(0));
    }

    #[test]
    fn arrows_cross_to_the_next_screen() {
        // Screen 1's last tile at the right, screen 2's tiles to the right of it and a bit lower.
        let tiles = [r(0, 0, 100, 100), r(110, 0, 100, 100), r(2000, 40, 100, 100), r(2000, 600, 100, 100)];
        assert_eq!(step(&tiles, 1, Dir::Right), Some(2));
        assert_eq!(step(&tiles, 2, Dir::Left), Some(1));
        // Nothing level below and to the left: the nearest counts.
        assert_eq!(step(&tiles, 3, Dir::Left), Some(1));
    }

    #[test]
    fn a_level_tile_beats_a_nearer_one_off_the_row() {
        let tiles = [r(0, 0, 100, 100), r(150, 150, 100, 100), r(400, 0, 100, 100)];
        assert_eq!(step(&tiles, 0, Dir::Right), Some(2));
    }

    #[test]
    fn a_drop_finds_a_zone_or_a_screen_without_zones() {
        let screens =
            vec![(r(0, 0, 100, 100), vec![r(0, 0, 48, 100), r(52, 0, 48, 100)]), (r(100, 0, 100, 100), vec![])];
        assert_eq!(drop_at(&screens, (10, 10)), Some(Landing::Zone(0, 0)));
        assert_eq!(drop_at(&screens, (60, 10)), Some(Landing::Zone(0, 1)));
        // In the gap between zones.
        assert_eq!(drop_at(&screens, (50, 10)), None);
        assert_eq!(drop_at(&screens, (150, 10)), Some(Landing::Screen(1)));
        assert_eq!(drop_at(&screens, (300, 10)), None);
    }

    #[test]
    fn a_window_moved_to_a_screen_is_centred_and_fits() {
        assert_eq!(centred(r(100, 0, 1000, 800), (400, 300)), r(400, 250, 400, 300));
        assert_eq!(centred(r(0, 0, 1000, 800), (1200, 300)), r(0, 250, 1000, 300));
    }

    fn facts(zones_now: &[Rect]) -> DropFacts<'_> {
        DropFacts {
            alive: true,
            elevated: false,
            tiled_window: true,
            zone: Some(1),
            zones_now,
            shown: r(50, 0, 50, 100),
            target_tiled: false,
        }
    }

    #[test]
    fn a_refused_drop_floats_nothing() {
        let zones = [r(0, 0, 50, 100), r(50, 0, 50, 100)];
        let refused = |f: DropFacts| matches!(plan_drop(&f), DropPlan::Refuse(_));
        assert!(refused(DropFacts { alive: false, ..facts(&zones) }));
        assert!(refused(DropFacts { elevated: true, ..facts(&zones) }));
        // The zone moved, or is gone, since the overview opened.
        assert!(refused(DropFacts { shown: r(40, 0, 60, 100), ..facts(&zones) }));
        assert!(refused(facts(&zones[..1])));
        assert!(refused(facts(&[])));
    }

    #[test]
    fn a_drop_floats_a_tiled_window_then_snaps_or_floats_over_a_tile() {
        let zones = [r(0, 0, 50, 100), r(50, 0, 50, 100)];
        let want = DropPlan::Move { float_first: true, snap: Some(1), float_over: false };
        assert_eq!(plan_drop(&facts(&zones)), want);
        let tiled = DropFacts { target_tiled: true, ..facts(&zones) };
        assert_eq!(plan_drop(&tiled), DropPlan::Move { float_first: true, snap: None, float_over: true });
        // To a screen with no zones; a window no tiling holds isn't floated.
        let free = DropFacts { zone: None, tiled_window: false, ..facts(&[]) };
        assert_eq!(plan_drop(&free), DropPlan::Move { float_first: false, snap: None, float_over: false });
    }

    #[test]
    fn esc_closes_even_when_the_window_in_front_is_gone() {
        assert_eq!(escape(false, Some(3), |_| true), Escape::Back(3));
        assert_eq!(escape(false, Some(3), |_| false), Escape::Close);
        assert_eq!(escape(false, None, |_| true), Escape::Close);
        // A drag ends first; the next Esc closes.
        assert_eq!(escape(true, Some(3), |_| true), Escape::CancelDrag);
    }

    #[test]
    fn a_search_handles_repeats_empty_fields_and_other_scripts() {
        // Each word counts, a repeated one too.
        assert_eq!(score("mail mail", "Mail", "Outlook"), Some(60));
        assert_eq!(score("x", "", ""), None);
        assert_eq!(score("note", "", "Notepad"), Some(40));
        assert_eq!(score("ÉTÉ", "Photos d'été", "Photos"), Some(30));
        assert_eq!(score("longer than the title", "long", "app"), None);
    }

    #[test]
    fn many_desktops_on_a_short_screen_stay_inside_it() {
        let area = r(0, 0, 1200, 900);
        let groups = layout(area, &[6, 4, 3, 2], &M);
        assert_eq!(groups.len(), 4);
        let bottom = groups.iter().flat_map(|g| &g.tiles).map(|t| t.y + t.h).max().unwrap();
        assert!(bottom <= area.h, "{bottom}");
        // The widest allowed tile still caps them.
        assert!(groups.iter().flat_map(|g| &g.tiles).all(|t| t.w <= M.max_w));
    }

    #[test]
    fn desktops_follow_windows_order_after_the_one_in_front() {
        // In front (third in Windows' list), one unknown, then the first and second.
        let (ranked, index) = desktop_ranks(&[Some(2), None, Some(1), Some(0)]);
        assert_eq!(ranked, vec![0, 3, 2, 1]);
        assert_eq!(index, vec![0, 3, 2, 1]);
        assert_eq!(desktop_ranks(&[None]), (vec![0], vec![0]));
    }

    #[test]
    fn arrows_stop_at_the_edges_and_ignore_a_tile_out_of_range() {
        let tiles = [r(0, 0, 100, 100), r(0, 110, 100, 100)];
        assert_eq!(step(&tiles, 1, Dir::Down), None);
        assert_eq!(step(&tiles, 0, Dir::Up), None);
        assert_eq!(step(&tiles, 5, Dir::Down), None);
    }

    #[test]
    fn a_screen_too_small_gives_tiles_no_width_rather_than_panicking() {
        let groups = layout(r(0, 0, 0, 0), &[3, 2], &M);
        assert!(groups.iter().flat_map(|g| &g.tiles).all(|t| t.w == 0));
    }

    #[test]
    fn a_drop_on_a_screen_left_of_the_primary_counts() {
        let screens = vec![(r(-1920, 0, 1920, 1080), vec![]), (r(0, 0, 100, 100), vec![])];
        assert_eq!(drop_at(&screens, (-10, 10)), Some(Landing::Screen(0)));
        // The right edge belongs to the next screen.
        assert_eq!(drop_at(&screens, (0, 10)), Some(Landing::Screen(1)));
    }

    // Windows 10 and 11 on desktop 0, 12 on 1, 20 and 21 on 2.
    const SHOWN: [(usize, usize); 5] = [(10, 0), (11, 0), (12, 1), (20, 2), (21, 2)];

    #[test]
    fn a_jump_lands_on_the_first_window_of_the_next_desktop() {
        assert_eq!(desktop_jump(&SHOWN, Some(11), 1), Some(12));
        assert_eq!(desktop_jump(&SHOWN, Some(12), 1), Some(20));
        assert_eq!(desktop_jump(&SHOWN, Some(21), -1), Some(12));
    }

    #[test]
    fn a_jump_wraps_around_both_ways() {
        assert_eq!(desktop_jump(&SHOWN, Some(21), 1), Some(10));
        assert_eq!(desktop_jump(&SHOWN, Some(10), -1), Some(20));
    }

    #[test]
    fn with_one_desktop_a_jump_goes_to_its_first_window() {
        let one = [(3, 4), (5, 4)];
        assert_eq!(desktop_jump(&one, Some(5), 1), Some(3));
        assert_eq!(desktop_jump(&one, Some(3), -1), Some(3));
    }

    #[test]
    fn a_desktop_with_no_windows_is_skipped_and_nothing_shown_gives_nothing() {
        // Desktop 1 has no window shown, as when the search keeps none of its windows.
        let kept = [(10, 0), (20, 2)];
        assert_eq!(desktop_jump(&kept, Some(10), 1), Some(20));
        assert_eq!(desktop_jump(&kept, Some(20), 1), Some(10));
        assert_eq!(desktop_jump(&[], None, 1), None);
        assert_eq!(desktop_jump(&[], Some(1), -1), None);
    }

    #[test]
    fn with_nothing_selected_a_jump_starts_from_the_edge() {
        assert_eq!(desktop_jump(&SHOWN, None, 1), Some(10));
        assert_eq!(desktop_jump(&SHOWN, None, -1), Some(20));
        // A selection that isn't shown counts as none.
        assert_eq!(desktop_jump(&SHOWN, Some(99), 1), Some(10));
    }

    #[test]
    fn desktops_are_named_by_windows_or_numbered() {
        assert_eq!(desktop_label(true, Some("Work"), Some(1)), "This desktop");
        assert_eq!(desktop_label(false, Some("Work"), Some(1)), "Work");
        assert_eq!(desktop_label(false, Some("  "), Some(1)), "Desktop 2");
        assert_eq!(desktop_label(false, None, None), "Another desktop");
    }

    #[test]
    fn the_name_read_out_is_the_app_and_title_or_the_count() {
        assert_eq!(spoken(Some(("Firefox", "Inbox")), 5), "Seam overview: Firefox - Inbox");
        assert_eq!(spoken(Some(("Firefox", " ")), 5), "Seam overview: Firefox");
        assert_eq!(spoken(Some(("", "Inbox")), 5), "Seam overview: Inbox");
        assert_eq!(spoken(Some(("", "")), 5), "Seam overview");
        assert_eq!(spoken(None, 1), "Seam overview: 1 window");
        assert_eq!(spoken(None, 0), "Seam overview: 0 windows");
        assert_eq!(spoken(None, 7), "Seam overview: 7 windows");
    }

    #[test]
    fn a_details_slice_repaints_only_the_shown_tiles_that_changed() {
        assert_eq!(changed(&["a", "b", "c"], &["a", "x", "y"]), vec![1, 2]);
        assert!(changed(&[1, 2], &[1, 2]).is_empty());
        assert_eq!(repaint(&[0, 1, 2], &[0, 1, 2], &[]), Repaint::Nothing);
        assert_eq!(repaint(&[0, 1, 2], &[0, 1, 2], &[1, 2]), Repaint::Tiles(vec![1, 2]));
        // A window the search leaves out has no tile to repaint.
        assert_eq!(repaint(&[0, 2], &[0, 2], &[1]), Repaint::Nothing);
        assert_eq!(repaint(&[0, 2], &[0, 2], &[1, 2]), Repaint::Tiles(vec![2]));
        // The search keeps other windows, or the same in another order: every tile moved.
        assert_eq!(repaint(&[0, 1], &[1, 0], &[]), Repaint::All);
        assert_eq!(repaint(&[0, 1], &[0], &[1]), Repaint::All);
    }

    #[test]
    fn a_second_request_while_one_is_opening_is_dropped() {
        let o = Opening::new();
        assert!(!o.is_out());
        let first = o.begin(0).unwrap();
        assert!(o.is_out());
        assert!(o.begin(1).is_none());
        assert!(o.begin(OPENING_LIMIT_MS - 1).is_none());
        o.done(first);
        assert!(!o.is_out());
        assert!(o.begin(2).is_some());
    }

    #[test]
    fn a_request_older_than_the_limit_is_let_past_in_case_the_thread_died() {
        let o = Opening::new();
        let stale = o.begin(100).unwrap();
        assert!(o.begin(100 + OPENING_LIMIT_MS - 1).is_none());
        let newer = o.begin(100 + OPENING_LIMIT_MS).unwrap();
        assert!(o.begin(100 + OPENING_LIMIT_MS + 1).is_none());
        // The first thread, back late, doesn't clear the newer request.
        o.done(stale);
        assert!(o.is_out());
        assert!(o.begin(100 + OPENING_LIMIT_MS + 2).is_none());
        o.done(newer);
        assert!(!o.is_out());
    }

    #[test]
    fn a_long_or_many_lined_title_is_read_as_one_short_line() {
        assert_eq!(spoken(Some(("App", "a\n  b\tc")), 1), "Seam overview: App - a b c");
        let said = spoken(Some(("App", &"x".repeat(500))), 1);
        assert_eq!(said.chars().count(), "Seam overview: App - ".len() + 200);
    }
}
