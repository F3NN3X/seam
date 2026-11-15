//! Where each screen's tiles go, the live previews in them, and how a screen's window
//! is painted: the tiles and their search, or the zones while a window is dragged.

use super::*;

/// Lay out every screen's tiles from the windows the search keeps, grouped by desktop.
pub(super) fn lay_out(s: &mut State) {
    let labelled = s.desktops.len() > 1;
    for (i, p) in s.panes.iter_mut().enumerate() {
        let (pad, head, gap, foot) = (p.t(PAD), p.t(HEAD), p.t(GAP), p.t(FOOT));
        let top = pad + head + gap;
        let area =
            Rect { x: p.rect.x + pad, y: p.rect.y + top, w: p.rect.w - 2 * pad, h: p.rect.h - top - gap - foot - pad };
        let metrics = overview::Metrics {
            gap,
            label: if labelled { p.t(LABEL) } else { 0 },
            caption: p.t(CAPTION),
            max_w: p.t(TILE_MAX),
            aspect: (p.rect.w.max(1), p.rect.h.max(1)),
        };
        let members: Vec<Vec<usize>> = (0..s.desktops.len())
            .map(|d| s.shown.iter().copied().filter(|&w| s.wins[w].screen == i && s.wins[w].desktop == d).collect())
            .collect();
        let counts: Vec<usize> = members.iter().map(Vec::len).collect();
        let groups = overview::layout(area, &counts, &metrics);
        let filled = members.iter().enumerate().filter(|(_, m)| !m.is_empty());
        p.labels.clear();
        p.tiles.clear();
        for ((d, m), g) in filled.zip(groups) {
            if labelled {
                p.labels.push((d, g.label));
            }
            p.tiles.extend(m.iter().copied().zip(g.tiles));
        }
    }
}

/// The part of a tile its preview fills, in screen pixels.
fn preview(p: &Pane, tile: Rect) -> Rect {
    let inset = p.t(INSET);
    Rect { x: tile.x + inset, y: tile.y + inset, w: tile.w - 2 * inset, h: tile.h - p.t(CAPTION) - inset }
}

/// The largest rect of the source's shape in `box_`, centred.
fn fit((sw, sh): (i32, i32), b: Rect) -> Rect {
    let (sw, sh) = (i64::from(sw.max(1)), i64::from(sh.max(1)));
    let (w, h) = if i64::from(b.w) * sh <= i64::from(b.h) * sw {
        (b.w, (i64::from(b.w) * sh / sw) as i32)
    } else {
        ((i64::from(b.h) * sw / sh) as i32, b.h)
    };
    Rect { x: b.x + (b.w - w) / 2, y: b.y + (b.h - h) / 2, w, h }
}

/// A live preview for each window on the desktop in front that isn't minimized. One that
/// can't be made, or has nothing to show, falls back to the app's icon.
pub(super) fn register_thumbs(s: &mut State) -> usize {
    let mut made = 0;
    for w in &mut s.wins {
        if w.desktop != 0 || w.minimized || w.hung {
            continue;
        }
        let Some(p) = s.panes.get(w.screen) else { continue };
        let Ok(id) = (unsafe { DwmRegisterThumbnail(p.hwnd, w.hwnd) }) else { continue };
        match unsafe { DwmQueryThumbnailSourceSize(id) } {
            Ok(size) if size.cx > 0 && size.cy > 0 => {
                (w.thumb, w.source) = (id, (size.cx, size.cy));
                made += 1;
            }
            _ => {
                let _ = unsafe { DwmUnregisterThumbnail(id) };
            }
        }
    }
    made
}

/// Every preview goes. Done before the windows close.
pub(super) fn unregister_thumbs(s: &mut State) {
    for w in &mut s.wins {
        if w.thumb != 0 {
            let _ = unsafe { DwmUnregisterThumbnail(w.thumb) };
            w.thumb = 0;
        }
    }
}

/// Show each preview in its tile, and hide the ones of windows the search left out, or
/// all of them while a window is dragged, when the zones show instead.
pub(super) fn sync_thumbs(s: &State) {
    let shown = |w: usize| {
        let p = &s.panes[s.wins[w].screen];
        p.tiles.iter().find(|(x, _)| *x == w).map(|(_, r)| p.local(fit(s.wins[w].source, preview(p, *r))))
    };
    for (i, w) in s.wins.iter().enumerate() {
        if w.thumb == 0 {
            continue;
        }
        let place = if s.drag.is_some() { None } else { shown(i) };
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY | DWM_TNP_RECTDESTINATION,
            rcDestination: place.unwrap_or_default(),
            opacity: 255,
            fVisible: place.is_some().into(),
            fSourceClientAreaOnly: false.into(),
            ..Default::default()
        };
        let _ = unsafe { DwmUpdateThumbnailProperties(w.thumb, &props) };
    }
}

unsafe fn outline(dc: HDC, r: RECT, width: i32, c: Color) {
    unsafe {
        fill(dc, RECT { bottom: r.top + width, ..r }, c);
        fill(dc, RECT { top: r.bottom - width, ..r }, c);
        fill(dc, RECT { right: r.left + width, ..r }, c);
        fill(dc, RECT { left: r.right - width, ..r }, c);
    }
}

fn grown(r: RECT, by: i32) -> RECT {
    RECT { left: r.left - by, top: r.top - by, right: r.right + by, bottom: r.bottom + by }
}

fn row(left: i32, right: i32, top: i32, height: i32) -> RECT {
    RECT { left, top, right, bottom: top + height }
}

/// The app's icon, or its first letter on a square when it has none.
unsafe fn icon(dc: HDC, w: &Win, at: (i32, i32), size: i32, c: &Colours) {
    unsafe {
        if let Some(icon) = w.icon
            && DrawIconEx(dc, at.0, at.1, icon, size, size, 0, None, DI_NORMAL).is_ok()
        {
            return;
        }
        let square = RECT { left: at.0, top: at.1, right: at.0 + size, bottom: at.1 + size };
        fill(dc, square, c.raised);
        let letter: String = w.app.chars().next().map(|f| f.to_uppercase().collect()).unwrap_or_default();
        text(dc, &letter, square, size * 6 / 10, c.text, DT_CENTER | DT_VCENTER);
    }
}

/// The header: the screen's number, the search and how many windows it found.
unsafe fn header(dc: HDC, s: &State, p: &Pane, client: RECT) {
    unsafe {
        let c = &s.colours;
        let (pad, head) = (p.t(PAD), p.t(HEAD));
        let line = row(pad, client.right - pad, pad, head);
        text(dc, &format!("Screen {}", p.number), line, p.t(16), c.text, DT_LEFT | DT_VCENTER);
        let width = p.t(SEARCH_W).min(client.right - 2 * pad);
        let left = (client.right - width) / 2;
        let field = row(left, left + width, pad, head);
        fill(dc, field, c.raised);
        outline(dc, field, p.t(1), c.hairline);
        let inside = RECT { left: field.left + p.t(12), right: field.right - p.t(12), ..field };
        if s.query.is_empty() {
            // Secondary, not muted: muted on the raised field is under 4.5:1.
            text(dc, "Type to search by title or app", inside, p.t(14), c.secondary, DT_LEFT | DT_VCENTER);
        } else {
            text(dc, &s.query, inside, p.t(14), c.text, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
        }
        let found = match s.shown.len() {
            1 => "1 window".to_string(),
            n => format!("{n} windows"),
        };
        text(dc, &found, line, p.t(12), c.muted, DT_RIGHT | DT_VCENTER);
    }
}

unsafe fn footer(dc: HDC, p: &Pane, client: RECT, hint: &str, c: &Colours) {
    unsafe {
        let pad = p.t(PAD);
        let line = row(pad, client.right - pad, client.bottom - pad - p.t(FOOT), p.t(FOOT));
        text(dc, hint, line, p.t(12), c.muted, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
    }
}

unsafe fn tile(dc: HDC, s: &State, p: &Pane, w: usize, r: Rect) {
    unsafe {
        let (c, win) = (&s.colours, &s.wins[w]);
        let local = p.local(r);
        let selected = s.selected == Some(w);
        if selected {
            // The focus ring: 2 px, 2 px out from the tile.
            outline(dc, grown(local, p.t(4)), p.t(2), c.ring);
        }
        fill(dc, local, if selected { c.raised } else { c.card });
        outline(dc, local, p.t(1), c.hairline);
        if win.thumb == 0 {
            let area = p.local(preview(p, r));
            fill(dc, area, c.page);
            let big = p.t(BIG_ICON);
            let name_h = p.t(20);
            let top = area.top + (area.bottom - area.top - big - p.t(8) - name_h) / 2;
            icon(dc, win, ((area.left + area.right - big) / 2, top), big, c);
            let name = row(area.left + p.t(8), area.right - p.t(8), top + big + p.t(8), name_h);
            text(dc, &win.app, name, p.t(13), c.secondary, DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS);
        }
        let caption = row(local.left + p.t(INSET), local.right - p.t(INSET), local.bottom - p.t(CAPTION), p.t(CAPTION));
        let small = p.t(ICON);
        icon(dc, win, (caption.left, caption.top + (p.t(CAPTION) - small) / 2), small, c);
        let status = if win.hung {
            Some("Not responding")
        } else if win.hwnd.0 as isize == s.before {
            Some("In use")
        } else if win.minimized {
            Some("Minimized")
        } else {
            None
        };
        let mut title = RECT { left: caption.left + small + p.t(8), ..caption };
        if let Some(status) = status {
            // A selected tile is raised, where muted text is under 4.5:1.
            let tone = if selected { c.secondary } else { c.muted };
            text(dc, status, caption, p.t(12), tone, DT_RIGHT | DT_VCENTER);
            title.right -= p.t(if win.hung { 104 } else { 72 });
        }
        let name = if win.title.is_empty() { &win.app } else { &win.title };
        text(dc, name, title, p.t(13), c.text, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
    }
}

unsafe fn tiles(dc: HDC, s: &State, p: &Pane, client: RECT) {
    unsafe {
        let c = &s.colours;
        header(dc, s, p, client);
        for (d, r) in &p.labels {
            text(dc, &s.desktops[*d], p.local(*r), p.t(14), c.secondary, DT_LEFT | DT_VCENTER);
        }
        for (w, r) in &p.tiles {
            tile(dc, s, p, *w, *r);
        }
        if p.tiles.is_empty() {
            let empty = match (s.query.is_empty(), s.shown.is_empty()) {
                (true, _) => "No windows on this screen".to_string(),
                (false, true) => format!("No window matches \u{201C}{}\u{201D}. Backspace changes the search", s.query),
                (false, false) => "No match on this screen".to_string(),
            };
            text(dc, &empty, client, p.t(14), c.secondary, DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS);
        }
        match &s.notice {
            Some(notice) => {
                let pad = p.t(PAD);
                let line = row(pad, client.right - pad, client.bottom - pad - p.t(FOOT), p.t(FOOT));
                text(dc, notice, line, p.t(13), c.text, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
            }
            None => {
                let hint =
                    "Type to search, arrows and Enter go to a window, drag one to a zone or another screen, Esc closes";
                footer(dc, p, client, hint, c);
            }
        }
    }
}

/// While a window is dragged: this screen's zones, or the screen itself when it has
/// none, with the one the drop would land on lit.
unsafe fn zones(dc: HDC, s: &State, i: usize, p: &Pane, client: RECT, drag: &Drag) {
    unsafe {
        let c = &s.colours;
        let win = &s.wins[drag.win];
        let (pad, head) = (p.t(PAD), p.t(HEAD));
        let line = row(pad, client.right - pad, pad, head);
        let target = |r: Rect, lit: bool, label: &str| {
            let local = p.local(r);
            fill(dc, local, if lit { c.lit } else { c.card });
            outline(dc, local, if lit { p.t(2) } else { p.t(1) }, if lit { c.accent } else { c.hairline });
            text(dc, label, local, p.t(24), if lit { c.text } else { c.secondary }, DT_CENTER | DT_VCENTER);
        };
        if p.zones.is_empty() {
            let away = RECT { top: pad + head + p.t(GAP), bottom: client.bottom - pad - p.t(FOOT + GAP), ..line };
            if win.screen == i {
                text(dc, "This screen has no zones", away, p.t(14), c.muted, DT_CENTER | DT_VCENTER);
            } else {
                let lit = drag.over == Some(Landing::Screen(i));
                let work = Rect { y: p.work.y.max(p.rect.y + pad + head + p.t(GAP)), ..p.work };
                target(work, lit, &format!("Move to screen {}", p.number));
            }
        } else {
            for (z, r) in p.zones.iter().enumerate() {
                target(*r, drag.over == Some(Landing::Zone(i, z)), &(z + 1).to_string());
            }
        }
        // The header goes over the zones, which reach the top of the screen.
        text(dc, &format!("Screen {}", p.number), line, p.t(16), c.text, DT_LEFT | DT_VCENTER);
        let name = if win.title.is_empty() { &win.app } else { &win.title };
        let moving = RECT { left: line.left + p.t(120), right: line.right - p.t(120), ..line };
        text(dc, &format!("Moving {name}"), moving, p.t(14), c.text, DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS);
        text(dc, "Esc cancels", line, p.t(12), c.muted, DT_RIGHT | DT_VCENTER);
        let hint = if p.tiled && !p.zones.is_empty() {
            "Smart tiling arranges this screen, so a window dropped here floats over its zone"
        } else {
            "Drop on a zone, or on a screen with no zones"
        };
        footer(dc, p, client, hint, c);
    }
}

/// Whether a point is in a screen's header band, where the search field is, or on a
/// desktop's label, which a click doesn't close the overview from.
pub(super) fn in_header(s: &State, hwnd: HWND, at: (i32, i32)) -> bool {
    let Some(p) = s.panes.iter().find(|p| p.hwnd == hwnd) else { return false };
    at.1 < p.rect.y + p.t(PAD + HEAD + GAP) || p.labels.iter().any(|(_, r)| r.contains(at.0, at.1))
}

pub(super) unsafe fn paint(dc: HDC, s: &State, i: usize, client: RECT) {
    unsafe {
        let p = &s.panes[i];
        fill(dc, client, s.colours.page);
        match &s.drag {
            Some(drag) => zones(dc, s, i, p, client, drag),
            None => tiles(dc, s, p, client),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_keeps_its_window_shape_in_the_middle_of_its_box() {
        let b = Rect { x: 0, y: 0, w: 200, h: 100 };
        assert_eq!(fit((1600, 1600), b), Rect { x: 50, y: 0, w: 100, h: 100 });
        assert_eq!(fit((400, 100), b), Rect { x: 0, y: 25, w: 200, h: 50 });
    }
}
