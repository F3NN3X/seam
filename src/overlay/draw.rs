//! Painting the Line Up screens.

use super::*;

// ---------------------------------------------------------------- drawing --

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

const BG: (u8, u8, u8) = (14, 16, 20);

const PANEL: (u8, u8, u8) = (30, 33, 40);

const GUIDE: (u8, u8, u8) = (64, 70, 82);

const EDGE: (u8, u8, u8) = (255, 176, 32);

const LINE_COLORS: [(u8, u8, u8); 2] = [(0, 200, 255), (255, 72, 176)];

unsafe fn fill(dc: HDC, r: RECT, c: (u8, u8, u8)) {
    unsafe {
        let br = CreateSolidBrush(rgb(c.0, c.1, c.2));
        FillRect(dc, &r, br);
        let _ = DeleteObject(br.into());
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn text(
    dc: HDC,
    s: &str,
    mut r: RECT,
    px: i32,
    bold: bool,
    c: (u8, u8, u8),
    flags: DRAW_TEXT_FORMAT,
    panel: bool,
) {
    unsafe {
        let weight = if bold { FW_SEMIBOLD.0 } else { FW_NORMAL.0 } as i32;
        let font = CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old = SelectObject(dc, font.into());
        let mut buf: Vec<u16> = s.encode_utf16().collect();
        if panel {
            let mut measure = r;
            DrawTextW(dc, &mut buf, &mut measure, flags | DT_CALCRECT);
            let (w, h) = (measure.right - measure.left, measure.bottom - measure.top);
            let cx = (r.left + r.right) / 2;
            let pad = px;
            let top = r.top;
            fill(
                dc,
                RECT { left: cx - w / 2 - pad, top: top - pad, right: cx + w / 2 + pad, bottom: top + h + pad },
                PANEL,
            );
            r = RECT { left: cx - w / 2, top, right: cx + w / 2 + 1, bottom: top + h };
        }
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, rgb(c.0, c.1, c.2));
        DrawTextW(dc, &mut buf, &mut r, flags);
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
    }
}

/// A line across the whole client area, perpendicular to the shared edge.
unsafe fn line(dc: HDC, w: i32, h: i32, along_is_y: bool, pos: f64, thick: i32, c: (u8, u8, u8)) {
    unsafe {
        let p = pos.round() as i32 - thick / 2;
        let r = if along_is_y {
            RECT { left: 0, top: p, right: w, bottom: p + thick }
        } else {
            RECT { left: p, top: 0, right: p + thick, bottom: h }
        };
        fill(dc, r, c);
    }
}

/// Small numbered tag next to a line, on the shared-edge side of the screen.
#[allow(clippy::too_many_arguments)]
unsafe fn tag(dc: HDC, w: i32, h: i32, edge: Side, pos: f64, label: &str, s: f64, c: (u8, u8, u8)) {
    unsafe {
        let size = (34.0 * s) as i32;
        let inset = (28.0 * s) as i32;
        let p = pos.round() as i32;
        let (x, y) = match edge {
            Side::Right => (w - inset - size, p - size - (6.0 * s) as i32),
            Side::Left => (inset, p - size - (6.0 * s) as i32),
            Side::Bottom => (p + (6.0 * s) as i32, h - inset - size),
            Side::Top => (p + (6.0 * s) as i32, inset),
        };
        let r = RECT { left: x, top: y, right: x + size, bottom: y + size };
        fill(dc, r, c);
        text(dc, label, r, (20.0 * s) as i32, true, BG, DT_CENTER | DT_VCENTER | DT_SINGLELINE, false);
    }
}

/// `c` faded most of the way into the background, for lines that are off.
fn dim(c: (u8, u8, u8)) -> (u8, u8, u8) {
    let mix = |x: u8, bg: u8| ((x as u16 * 2 + bg as u16 * 3) / 5) as u8;
    (mix(c.0, BG.0), mix(c.1, BG.1), mix(c.2, BG.2))
}

/// The shared edge drawn in dashes, for screens with a gap between them in Windows.
unsafe fn dashed_edge(dc: HDC, w: i32, h: i32, edge: Side, t: i32, s: f64, c: (u8, u8, u8)) {
    unsafe {
        let (dash, step) = ((24.0 * s) as i32, (36.0 * s) as i32);
        let strip = edge_strip(w, h, edge, t);
        let len = if edge.along_is_y() { h } else { w };
        for p in (0..len).step_by(step.max(2) as usize) {
            let r = if edge.along_is_y() {
                RECT { top: p, bottom: (p + dash).min(h), ..strip }
            } else {
                RECT { left: p, right: (p + dash).min(w), ..strip }
            };
            fill(dc, r, c);
        }
    }
}

/// How far apart Windows has the step's screens, in pixels.
fn windows_gap(c: &Calib, step: &Step) -> i32 {
    let (a, b) = (c.mons[step.a].rect, c.mons[step.b].rect);
    if step.side.along_is_y() { b.left - a.right } else { b.top - a.bottom }
}

/// A note beside the shared edge, saying how big the gap is in Windows.
unsafe fn gap_hint(dc: HDC, w: i32, h: i32, edge: Side, gap: i32, s: f64) {
    unsafe {
        let inset = (80.0 * s) as i32;
        let (bw, bh) = ((360.0 * s) as i32, (28.0 * s) as i32);
        let (r, align) = match edge {
            Side::Right => (RECT { left: w - inset - bw, top: h / 4, right: w - inset, bottom: h / 4 + bh }, DT_RIGHT),
            Side::Left => (RECT { left: inset, top: h / 4, right: inset + bw, bottom: h / 4 + bh }, DT_LEFT),
            Side::Bottom => (
                RECT { left: w / 2 - bw / 2, top: h - inset - bh, right: w / 2 + bw / 2, bottom: h - inset },
                DT_CENTER,
            ),
            Side::Top => {
                (RECT { left: w / 2 - bw / 2, top: inset, right: w / 2 + bw / 2, bottom: inset + bh }, DT_CENTER)
            }
        };
        let label = format!("Gap in Windows, {gap} px");
        text(dc, &label, r, (16.0 * s) as i32, false, (176, 176, 176), align | DT_SINGLELINE, false);
    }
}

/// The step's screen `i` as "3 (HDMI)", the number and model the corner shows.
fn screen_name(c: &Calib, i: usize) -> String {
    let m = &c.mons[i];
    if m.named { format!("{} ({})", i + 1, m.name) } else { format!("{}", i + 1) }
}

/// A band of 45 degree stripes along the shared edge, 10 mm apart on the desk. A's
/// are drawn in its own pixels. B's go through the step's map, so they run on
/// across the edge when the map is right, and step or change spacing when it isn't.
unsafe fn stripes(dc: HDC, w: i32, h: i32, c: &Calib, step: &Step, idx: usize, dpi_scale: f64) {
    unsafe {
        let Some((mm_a, mm_b)) = sizes(&c.mons, step) else { return };
        let role_a = idx == step.a;
        let depth = (STRIPE_BAND_MM / if role_a { mm_a } else { mm_b }).round() as i32;
        let period = STRIPE_GAP_MM / mm_a;
        if period < 3.0 {
            return;
        }
        let (along_len, perp_len) = if step.side.along_is_y() { (h, w) } else { (w, h) };
        let (edge, band) =
            if role_a { (step.side, (perp_len - depth, perp_len)) } else { (step.side.opposite(), (0, depth)) };
        // Each stripe is `along = perp + k`, with its own `k` on each screen.
        let ks: Vec<f64> = if role_a {
            let last = ((along_len + depth) as f64 / period).ceil() as i32;
            (0..=last).map(|n| n as f64 * period - perp_len as f64).collect()
        } else {
            let Some(m) = step.map() else { return };
            let inv = m.inverse();
            let first = (inv.apply(-depth as f64) / period).floor() as i32;
            let last = (inv.apply(along_len as f64) / period).ceil() as i32;
            (first..=last).map(|n| m.apply(n as f64 * period)).collect()
        };
        if ks.len() > 4000 {
            return;
        }
        let saved = SaveDC(dc);
        let strip = edge_strip(w, h, edge, depth);
        IntersectClipRect(dc, strip.left, strip.top, strip.right, strip.bottom);
        let tone = dim((110, 116, 130));
        let pen = CreatePen(PS_SOLID, ((2.0 * dpi_scale).round() as i32).max(1), rgb(tone.0, tone.1, tone.2));
        let old = SelectObject(dc, pen.into());
        for k in ks {
            let at = |perp: i32| {
                let along = (perp as f64 + k).round() as i32;
                if step.side.along_is_y() { (perp, along) } else { (along, perp) }
            };
            let ((x0, y0), (x1, y1)) = (at(band.0), at(band.1));
            let _ = MoveToEx(dc, x0, y0, None);
            let _ = LineTo(dc, x1, y1);
        }
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
        let _ = RestoreDC(dc, saved);
    }
}

fn edge_strip(w: i32, h: i32, edge: Side, t: i32) -> RECT {
    match edge {
        Side::Right => RECT { left: w - t, top: 0, right: w, bottom: h },
        Side::Left => RECT { left: 0, top: 0, right: t, bottom: h },
        Side::Bottom => RECT { left: 0, top: h - t, right: w, bottom: h },
        Side::Top => RECT { left: 0, top: 0, right: w, bottom: t },
    }
}

pub(super) unsafe fn paint(hwnd: HWND, idx: usize) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (w, h) = (client.right, client.bottom);
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old = SelectObject(mem, bmp.into());
        fill(mem, client, BG);
        let s = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;

        CALIB.with(|cell| {
            let Ok(guard) = cell.try_borrow() else { return };
            let Some(c) = guard.as_ref() else { return };
            let Some(mon) = c.mons.get(idx) else { return };
            let step = &c.steps[c.cur];
            if step.adjustable() && (idx == step.a || idx == step.b) {
                stripes(mem, w, h, c, step, idx, s);
            }
            corner_labels(mem, idx, &mon.name, (w, h), s);

            let role_a = idx == step.a;
            let role_b = idx == step.b;
            let center = RECT { left: w / 6, top: h / 2 - (110.0 * s) as i32, right: w - w / 6, bottom: h };
            if role_a || role_b {
                step_marks(mem, (w, h), c, step, (role_a, role_b), s);
                let msg = step_message(c, step, role_b);
                text(mem, &msg, center, (20.0 * s) as i32, false, (226, 230, 238), DT_CENTER | DT_WORDBREAK, true);
            } else {
                text(
                    mem,
                    "Not part of this step",
                    center,
                    (22.0 * s) as i32,
                    false,
                    (110, 116, 130),
                    DT_CENTER | DT_WORDBREAK,
                    false,
                );
            }
            footer(mem, c, (w, h), s);
        });

        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Big screen number, like Shim, and the screen's name under it.
unsafe fn corner_labels(dc: HDC, idx: usize, name: &str, (w, h): (i32, i32), s: f64) {
    unsafe {
        let pad = (36.0 * s) as i32;
        text(
            dc,
            &format!("{}", idx + 1),
            RECT { left: pad, top: pad / 2, right: w, bottom: h },
            (150.0 * s) as i32,
            true,
            (58, 63, 74),
            DT_LEFT | DT_TOP | DT_SINGLELINE,
            false,
        );
        text(
            dc,
            name,
            RECT { left: pad, top: pad / 2 + (170.0 * s) as i32, right: w, bottom: h },
            (22.0 * s) as i32,
            false,
            (110, 116, 130),
            DT_LEFT | DT_TOP | DT_SINGLELINE,
            false,
        );
    }
}

/// The faint guide lines, the edge and the two numbered lines of a screen in this step.
unsafe fn step_marks(mem: HDC, (w, h): (i32, i32), c: &Calib, step: &Step, (role_a, role_b): (bool, bool), s: f64) {
    unsafe {
        let along_y = step.side.along_is_y();
        let (la, lb) = (c.mons[step.a].rect.along_len(step.side), c.mons[step.b].rect.along_len(step.side));
        let map = step.map();
        let shade = |c: (u8, u8, u8)| if step.adjustable() { c } else { dim(c) };
        // Faint guide lines through the physical overlap: they all straighten
        // out once the two main lines are right.
        if let Some(m) = map
            && let Some((lo, hi)) = overlap_in_a(&m, la, lb)
        {
            for k in 0..=10 {
                let a = lo + (hi - lo) * k as f64 / 10.0;
                let pos = if role_a { a } else { m.apply(a) };
                line(mem, w, h, along_y, pos, (1.0 * s).max(1.0) as i32, shade(GUIDE));
            }
        }
        let my_edge = if role_a { step.side } else { step.side.opposite() };
        if step.glue.is_some() {
            dashed_edge(mem, w, h, my_edge, (6.0 * s) as i32, s, shade(EDGE));
            gap_hint(mem, w, h, my_edge, windows_gap(c, step), s);
        } else {
            fill(mem, edge_strip(w, h, my_edge, (6.0 * s) as i32), shade(EDGE));
        }
        let pts = if role_a { step.a_pts } else { step.b_pts };
        for (i, pos) in [pts.0, pts.1].into_iter().enumerate() {
            let selected = role_b && step.adjustable() && (c.sel == i || c.sel == 2);
            let thick = ((if selected { 5.0 } else { 3.0 }) * s) as i32;
            line(mem, w, h, along_y, pos, thick.max(2), shade(LINE_COLORS[i]));
            tag(mem, w, h, my_edge, pos, &format!("{}", i + 1), s, shade(LINE_COLORS[i]));
        }
    }
}

/// What the centre of a screen says in this step.
fn step_message(c: &Calib, step: &Step, role_b: bool) -> String {
    let along_y = step.side.along_is_y();
    let last = c.cur + 1 == c.steps.len();
    let (a, b) = (screen_name(c, step.a), screen_name(c, step.b));
    // The stripes' one line, or why there are none.
    let stripe_note = if !step.adjustable() {
        ""
    } else if sizes(&c.mons, step).is_some() {
        "\nIf the stripes run on straight across the edge, the screens are lined up"
    } else {
        "\nNo stripe check: a screen on this pair reports no size"
    };
    let measure_note = if !c.measure || !step.adjustable() {
        String::new()
    } else if sizes(&c.mons, step).is_none() {
        "\nTyping an offset isn't offered: a screen on this pair reports no size".to_string()
    } else if let Some(t) = &c.typing {
        let (an, bn) = (step.a + 1, step.b + 1);
        let (edge, phrase, flip) = match (along_y, t.after) {
            (true, true) => ("top", "below", "below/above"),
            (true, false) => ("top", "above", "below/above"),
            (false, true) => ("left", "to the right of", "right/left"),
            (false, false) => ("left", "to the left of", "right/left"),
        };
        format!(
            "\n\nScreen {bn}'s {edge} edge is [ {}_ ] cm {phrase} screen {an}'s {edge} edge\n\
             Tab flips {flip}  ·  Enter: apply  ·  Esc: close",
            t.text
        )
    } else {
        "\nM: type the offset in cm".to_string()
    };
    let next = if last { "done" } else { "next border" };
    // Where B is from A, and the reverse, in desk words: "straight below" reads
    // better than "continues across the bezel" when the screens are far apart.
    let (b_from_a, a_from_b, lands) = match step.side {
        Side::Bottom => ("below", "above", "straight below"),
        Side::Top => ("above", "below", "straight above"),
        Side::Right => ("to the right", "to the left", "level with"),
        Side::Left => ("to the left", "to the right", "level with"),
    };
    match (role_b, step.glue, step.not_on_desk) {
        (true, Some(false), _) => format!(
            "Glue screen {b} to screen {a}?\n\n\
             Do this if this screen touches screen {a} on your desk. Windows has a gap \
             between them, so today the pointer stops at the edge.\n\n\
             Press G, then line them up as they stand on your desk.\n\n\
             Enter: skip   ·   Backspace: back   ·   Esc: cancel"
        ),
        (true, None, true) => format!(
            "Screen {b} doesn't touch screen {a} on your desk\n\n\
             Windows has them touching, so Seam stops the pointer crossing here. \
             Where you've glued a screen past this edge, the pointer goes there instead.\n\n\
             N says they do touch\n\n\
             Enter: {next}   ·   Backspace: back   ·   Esc: cancel"
        ),
        (true, glue, _) => format!(
            "{title}\n\n\
             Screen {a}, {a_from_b}, has a blue line 1 and a pink line 2.\n\
             Move this screen's blue line 1 until it sits {lands} screen {a}'s blue line, \
             as if one line ran across both screens. Then do the same with the pink line 2.{gap}\n\n\
             Grab a line to move just that one, or drag anywhere else to move both.\n\
             Arrows nudge  ·  Shift ×10  ·  Ctrl = both  ·  Tab switches line  ·  R resets\n\
             The pointer follows your lines already, so cross over to check.{toggle}{stripe_note}{measure_note}\n\n\
             Enter: {next}   ·   Backspace: back   ·   Esc: cancel",
            title = if glue.is_some() {
                format!("Glue screen {b} to screen {a}")
            } else {
                format!("Line up screen {b} with screen {a}")
            },
            gap = if glue.is_some() {
                "\nThe gap doesn't matter: line them up as the screens stand on your desk."
            } else {
                ""
            },
            toggle = if glue.is_some() {
                "\nG unglues them".to_string()
            } else {
                format!(
                    "\nScreen {b} doesn't touch screen {a} on your desk? \
                     Press N, and Seam stops the pointer crossing here"
                )
            },
        ),
        (false, Some(false), _) => format!(
            "Reference screen\n\nScreen {b}, {b_from_a}, may touch this one on your desk. Look at screen {b} to glue them."
        ),
        (false, None, true) => format!(
            "Reference screen\n\nScreen {b}, {b_from_a}, doesn't touch this one on your desk, \
             so Seam stops the pointer crossing here."
        ),
        (false, ..) => format!(
            "Reference screen\n\nThe blue and pink lines here stay put.\n\
             On screen {b}, {b_from_a}, move its lines until each sits {lands} the matching line here.{stripe_note}"
        ),
    }
}

fn footer(dc: HDC, c: &Calib, (w, h): (i32, i32), s: f64) {
    let footer = RECT { left: 0, top: h - (64.0 * s) as i32, right: w, bottom: h };
    unsafe {
        text(
            dc,
            &format!("Seam  ·  border {} of {}", c.cur + 1, c.steps.len()),
            footer,
            (16.0 * s) as i32,
            false,
            (110, 116, 130),
            DT_CENTER | DT_SINGLELINE,
            false,
        );
    }
}
