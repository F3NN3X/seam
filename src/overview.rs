//! The overview's rules, with no Win32 in them: which windows a search keeps and in
//! what order, where a screen's tiles go, one group per virtual desktop, where the
//! arrow keys go from a tile, and what a drop lands on. The tray's `overview_win`
//! draws them and moves the windows.

use crate::zones::Rect;

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

    #[test]
    fn desktops_are_named_by_windows_or_numbered() {
        assert_eq!(desktop_label(true, Some("Work"), Some(1)), "This desktop");
        assert_eq!(desktop_label(false, Some("Work"), Some(1)), "Work");
        assert_eq!(desktop_label(false, Some("  "), Some(1)), "Desktop 2");
        assert_eq!(desktop_label(false, None, None), "Another desktop");
    }
}
