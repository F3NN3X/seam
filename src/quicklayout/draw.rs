//! The panel's geometry and how it is painted.

use super::*;

/// The panel's size and the grid's top, in pixels.
pub(super) fn frame(o: &Overlay, t: &dyn Fn(i32) -> i32) -> (i32, i32, i32) {
    let rows = o.shown().div_ceil(COLUMNS) as i32;
    let note = if o.note.is_some() { NOTE } else { 0 };
    let top = t(PAD + HEAD + note);
    let width = t(PAD * 2 + COLUMNS as i32 * CELL.0 + (COLUMNS as i32 - 1) * GAP);
    let height = top + t(rows * CELL.1 + (rows - 1) * GAP + 12 + FOOT + PAD);
    (width, height, top)
}

/// Where the thumbnail at `slot` (0 to 8) sits.
pub(super) fn cell(slot: usize, top: i32, t: &dyn Fn(i32) -> i32) -> RECT {
    let (col, row) = ((slot % COLUMNS) as i32, (slot / COLUMNS) as i32);
    let left = t(PAD + col * (CELL.0 + GAP));
    let y = top + t(row * (CELL.1 + GAP));
    RECT { left, top: y, right: left + t(CELL.0), bottom: y + t(CELL.1) }
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

/// The zones of a layout inside a box, the box being the screen's shape.
fn art(tree: Option<&Node>, shape: (i32, i32), at: RECT, t: &dyn Fn(i32) -> i32) -> (RECT, Vec<RECT>) {
    let (bw, bh) = (at.right - at.left, at.bottom - at.top);
    let (sw, sh) = (i64::from(shape.0.max(1)), i64::from(shape.1.max(1)));
    // The largest box of the screen's shape that fits.
    let (w, h) = if i64::from(bw) * sh <= i64::from(bh) * sw {
        (bw, (i64::from(bw) * sh / sw) as i32)
    } else {
        ((i64::from(bh) * sw / sh) as i32, bh)
    };
    let (x, y) = (at.left + (bw - w) / 2, at.top + (bh - h) / 2);
    let face = RECT { left: x, top: y, right: x + w, bottom: y + h };
    let pieces = tree
        .map(|tree| {
            zones::rects(tree, zones::Rect { x, y, w, h }, t(2))
                .into_iter()
                .map(|r| RECT { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h })
                .collect()
        })
        .unwrap_or_default();
    (face, pieces)
}

pub(super) unsafe fn draw(dc: HDC, o: &Overlay, client: RECT, scale: f64) {
    unsafe {
        let t = |v: i32| (v as f64 * scale).round() as i32;
        fill(dc, client, PAGE);
        outline(dc, client, 1, HAIRLINE);
        let (width, _, top) = frame(o, &t);
        let inner = |from: i32, to: i32, y: i32, h: i32| RECT { left: from, top: y, right: to, bottom: y + h };
        let (left, right) = (t(PAD), width - t(PAD));
        let title = format!("Layout for screen {}", o.number);
        text(dc, &title, inner(left, right, t(PAD), t(HEAD)), t(16), TEXT, DT_LEFT | DT_VCENTER);
        if o.choices.len() > VISIBLE {
            let last = (o.first + o.shown()).min(o.choices.len());
            let range = format!("{} to {} of {}", o.first + 1, last, o.choices.len());
            text(dc, &range, inner(left, right, t(PAD), t(HEAD)), t(12), MUTED, DT_RIGHT | DT_VCENTER);
        }
        if let Some(note) = &o.note {
            text(
                dc,
                note,
                inner(left, right, t(PAD + HEAD), t(NOTE)),
                t(13),
                SECONDARY,
                DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS,
            );
        }
        let current = o.current();
        for slot in 0..o.shown() {
            let at = o.first + slot;
            let choice = &o.choices[at];
            let r = cell(slot, top, &t);
            if current == Some(at) {
                outline(dc, grown(r, t(2)), t(1), HALO);
            }
            if o.selected == at {
                outline(dc, grown(r, t(5)), t(2), TEXT);
            }
            fill(dc, r, if o.selected == at { RAISED } else { CARD });
            outline(dc, r, t(1), if current == Some(at) { ACCENT } else { HAIRLINE });
            let box_ =
                RECT { left: r.left + t(8), top: r.top + t(8), right: r.right - t(8), bottom: r.top + t(8 + ART.1) };
            let tree = choice.layout.as_ref().map(|l| &l.tree);
            let (face, pieces) = art(tree, o.shape, box_, &t);
            fill(dc, face, PAGE);
            for piece in pieces {
                fill(dc, piece, SEGMENT);
            }
            let badge = RECT { left: r.left + t(8), top: r.top + t(8), right: r.left + t(28), bottom: r.top + t(28) };
            fill(dc, badge, CARD);
            text(dc, &(slot + 1).to_string(), badge, t(12), TEXT, DT_CENTER | DT_VCENTER);
            let name_top = r.top + t(8 + ART.1 + 8);
            let name = RECT { left: r.left + t(8), top: name_top, right: r.right - t(8), bottom: name_top + t(18) };
            let colour = if current == Some(at) { TEXT } else { SECONDARY };
            if current == Some(at) {
                text(dc, "In use", name, t(12), MUTED, DT_RIGHT | DT_VCENTER);
                let room = RECT { right: name.right - t(52), ..name };
                text(dc, &choice.title, room, t(13), colour, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
            } else {
                text(dc, &choice.title, name, t(13), colour, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);
            }
        }
        let hint = "1 to 9 pick, arrows and Enter, Ctrl+Z undo, Esc close";
        let foot_top = client.bottom - t(PAD + FOOT);
        text(dc, hint, inner(left, right, foot_top, t(FOOT)), t(12), MUTED, DT_LEFT | DT_VCENTER);
    }
}

pub(super) fn scale_of(hwnd: HWND) -> f64 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f64 / 96.0
}
