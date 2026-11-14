//! The physical map: screens and the lines between them, in the Seam and Windows views.

use super::*;

struct View {
    scale: f64,
    left: f64,
    top: f64,
}

/// A screen's rectangle on the map: x, y, width, height.
type Frame = (f64, f64, f64, f64);

/// Fit the screens into the map, keeping real proportions.
fn fit(frames: &[Frame], width: f64) -> View {
    let min_x = frames.iter().map(|f| f.0).fold(f64::MAX, f64::min);
    let min_y = frames.iter().map(|f| f.1).fold(f64::MAX, f64::min);
    let max_x = frames.iter().map(|f| f.0 + f.2).fold(f64::MIN, f64::max);
    let max_y = frames.iter().map(|f| f.1 + f.3).fold(f64::MIN, f64::max);
    let room = (MAP_PADDING * 2.) as f64;
    let (span_x, span_y) = ((max_x - min_x).max(1.), (max_y - min_y).max(1.));
    let scale = ((width - room) / span_x).min((MAP_HEIGHT as f64 - room) / span_y);
    View {
        scale,
        left: (width - span_x * scale) / 2. - min_x * scale,
        top: (MAP_HEIGHT as f64 - span_y * scale) / 2. - min_y * scale,
    }
}

fn screen_tile(
    panel: &Panel,
    index: usize,
    (x, y, w, h): Frame,
    sized: bool,
    view: &View,
    p: Palette,
    cx: &mut Context<Panel>,
) -> impl IntoElement + use<> {
    let number = index + 1;
    let name = panel.mons.get(index).map_or("", |m| m.name.as_str()).to_string();
    let primary = panel.mons.get(index).is_some_and(|m| m.primary);
    let label = if primary { format!("{name}, main") } else { name };
    let height = h * view.scale;
    div()
        .id(("screen", index))
        .absolute()
        .left(px((x * view.scale + view.left) as f32))
        .top(px((y * view.scale + view.top) as f32))
        .w(px((w * view.scale) as f32))
        .h(px(height as f32))
        .flex()
        .when(height >= SHORT_TILE, |d| d.flex_col())
        .when(height < SHORT_TILE, |d| d.gap(px(8.)))
        .items_center()
        .justify_center()
        .overflow_hidden()
        .bg(p.card)
        .border_1()
        .border_color(p.hairline_strong)
        .when(!sized, |d| d.border_dashed())
        .rounded(px(4.))
        .cursor_pointer()
        .child(div().text_size(px(16.)).child(number.to_string()))
        .child(div().text_size(px(11.)).text_color(p.secondary).truncate().child(label))
        .on_click(cx.listener(move |panel, _, window, cx| {
            panel.command("screen.identify", json!({ "number": number }), window, cx);
        }))
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    Solid,
    Dashed,
    Dotted,
}

/// Where a line runs on the map, in map pixels.
struct Run {
    along_y: bool,
    start: f64,
    end: f64,
    across: f64,
}

/// The stretch `from` to `to` of `a`'s edge on `side`, in the map's units.
fn edge_run(a: &Placed, side: Side, from: f64, to: f64, view: &View) -> Option<Run> {
    if to <= from {
        return None;
    }
    let along_y = side.along_is_y();
    let edge = match side {
        Side::Right => a.x + a.w,
        Side::Left => a.x,
        Side::Bottom => a.y + a.h,
        Side::Top => a.y,
    };
    let (start, end, across) = if along_y {
        (from * view.scale + view.top, to * view.scale + view.top, edge * view.scale + view.left)
    } else {
        (from * view.scale + view.left, to * view.scale + view.left, edge * view.scale + view.top)
    };
    Some(Run { along_y, start, end, across })
}

/// The part of the edge two screens physically share.
fn seam_run(a: &Placed, b: &Placed, side: Side, view: &View) -> Option<Run> {
    let (from, to) = if side.along_is_y() {
        (a.y.max(b.y), (a.y + a.h).min(b.y + b.h))
    } else {
        (a.x.max(b.x), (a.x + a.w).min(b.x + b.w))
    };
    edge_run(a, side, from, to, view)
}

/// Where Windows has two screens touching, drawn along the first one's edge. For a
/// border that isn't on the desk, where the screens don't sit together.
fn windows_run(panel: &Panel, a: &Placed, b: &Placed, side: Side, view: &View) -> Option<Run> {
    let (ra, rb) = (&panel.mons.get(a.index)?.rect, &panel.mons.get(b.index)?.rect);
    let (from, to, own, mm, size) = if side.along_is_y() {
        (ra.top.max(rb.top), ra.bottom.min(rb.bottom), ra.top, a.y, a.h / f64::from(ra.height()))
    } else {
        (ra.left.max(rb.left), ra.right.min(rb.right), ra.left, a.x, a.w / f64::from(ra.width()))
    };
    edge_run(a, side, mm + f64::from(from - own) * size, mm + f64::from(to - own) * size, view)
}

/// A line along one axis, solid or in dashes or dots.
fn stroke(run: &Run, width: f32, style: Line, color: Hsla) -> Vec<Div> {
    let (on, off) = match style {
        Line::Solid => (run.end - run.start, 0.),
        Line::Dashed => (8., 5.),
        Line::Dotted => (2., 4.),
    };
    let half = width / 2.;
    let mut parts = vec![];
    let mut at = run.start;
    while at < run.end {
        let length = px(on.min(run.end - at) as f32);
        let part = div().absolute().bg(color);
        parts.push(if run.along_y {
            part.left(px(run.across as f32 - half)).top(px(at as f32)).w(px(width)).h(length)
        } else {
            part.top(px(run.across as f32 - half)).left(px(at as f32)).h(px(width)).w(length)
        });
        at += on + off;
    }
    parts
}

/// A small note centred on a point of the map.
fn note_at(x: f64, y: f64, text: String, p: Palette) -> Div {
    const WIDTH: f32 = 120.;
    const HEIGHT: f32 = 16.;
    div()
        .absolute()
        .left(px(x as f32 - WIDTH / 2.))
        .top(px(y as f32 - HEIGHT / 2.))
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .child(div().px(px(4.)).bg(p.page).rounded(px(4.)).text_size(px(11.)).text_color(p.muted).child(text))
}

/// The line, and its note if it fits, for one border on the Seam view.
fn seam_border(panel: &Panel, a: &Placed, other: &Placed, b: &Border, view: &View, p: Palette) -> Vec<Div> {
    let lined_up = b.state == BorderState::LinedUp;
    let (style, color) = match (b.not_on_desk, b.glued, lined_up) {
        (true, ..) => (Line::Dotted, p.muted),
        (_, true, _) => (Line::Dashed, p.accent),
        (_, _, true) => (Line::Solid, p.accent),
        _ => (Line::Dashed, p.muted),
    };
    let run = if b.not_on_desk { windows_run(panel, a, other, b.side, view) } else { seam_run(a, other, b.side, view) };
    let Some(run) = run else { return vec![] };
    let mut parts = stroke(&run, SEAM_WIDTH, style, color);
    if b.not_on_desk && run.end - run.start >= NOTE_FITS {
        let mid = (run.start + run.end) / 2.;
        let (x, y) = if run.along_y { (run.across, mid) } else { (mid, run.across) };
        parts.push(note_at(x, y, "Not on your desk".into(), p));
    }
    parts
}

/// Where each screen sits on the desk, joined where you lined them up.
fn seam_map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    let frames: Vec<Frame> = panel.layout.iter().map(|s| (s.x, s.y, s.w, s.h)).collect();
    let view = fit(&frames, width);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.page).rounded(px(8.));
    for (s, frame) in panel.layout.iter().zip(&frames) {
        surface = surface.child(screen_tile(panel, s.index, *frame, s.sized, &view, p, cx));
    }
    for b in &state.borders {
        let find = |number: u32| panel.layout.iter().find(|s| s.index + 1 == number as usize);
        if let (Some(a), Some(other)) = (find(b.a), find(b.b)) {
            surface = surface.children(seam_border(panel, a, other, b, &view, p));
        }
    }
    surface.child(
        div()
            .absolute()
            .left(px(16.))
            .bottom(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(11.))
            .text_color(p.muted)
            .child(div().w(px((SCALE_BAR_MM * view.scale) as f32)).h(px(1.)).bg(p.muted))
            .child("20 cm"),
    )
}

/// What the Windows view draws between two screens: a line where they touch, or
/// a note where they face each other across a gap or overlap.
enum Between {
    Touch { side: Side, from: i32, to: i32, edge: i32 },
    Note { x: i32, y: i32, text: String },
}

fn windows_between(rects: &[Rect]) -> Vec<Between> {
    let mut found = vec![];
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            if let Some(side) = touching(a, b) {
                let (from, to) = if side.along_is_y() {
                    (a.top.max(b.top), a.bottom.min(b.bottom))
                } else {
                    (a.left.max(b.left), a.right.min(b.right))
                };
                let edge = match side {
                    Side::Right => a.right,
                    Side::Left => a.left,
                    Side::Bottom => a.bottom,
                    Side::Top => a.top,
                };
                found.push(Between::Touch { side, from, to, edge });
            } else if let Some((side, gap)) = facing_gap(a, b) {
                let c = corridor(a, b, side);
                let text = format!("{gap} px gap");
                found.push(Between::Note { x: (c.left + c.right) / 2, y: (c.top + c.bottom) / 2, text });
            } else if a.intersects(b) {
                let (left, right) = (a.left.max(b.left), a.right.min(b.right));
                let (top, bottom) = (a.top.max(b.top), a.bottom.min(b.bottom));
                let overlap = (right - left).min(bottom - top);
                if (1..=LABEL_PX).contains(&overlap) {
                    let text = format!("{overlap} px overlap");
                    found.push(Between::Note { x: (left + right) / 2, y: (top + bottom) / 2, text });
                }
            }
        }
    }
    found
}

/// The screens as Windows arranges them, in pixels.
fn windows_map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    let frames: Vec<Frame> = state
        .screens
        .iter()
        .map(|s| (f64::from(s.rect.x), f64::from(s.rect.y), f64::from(s.rect.w), f64::from(s.rect.h)))
        .collect();
    let rects: Vec<Rect> = state.screens.iter().map(|s| Rect::new(s.rect.x, s.rect.y, s.rect.w, s.rect.h)).collect();
    let view = fit(&frames, width);
    let (x_at, y_at) = (|v: i32| f64::from(v) * view.scale + view.left, |v: i32| f64::from(v) * view.scale + view.top);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.page).rounded(px(8.));
    for (s, frame) in state.screens.iter().zip(&frames) {
        surface = surface.child(screen_tile(panel, (s.number as usize).saturating_sub(1), *frame, true, &view, p, cx));
    }
    for between in windows_between(&rects) {
        match between {
            Between::Touch { side, from, to, edge } => {
                let along_y = side.along_is_y();
                let run = if along_y {
                    Run { along_y, start: y_at(from), end: y_at(to), across: x_at(edge) }
                } else {
                    Run { along_y, start: x_at(from), end: x_at(to), across: y_at(edge) }
                };
                surface = surface.children(stroke(&run, 2., Line::Solid, p.secondary));
            }
            Between::Note { x, y, text } => surface = surface.child(note_at(x_at(x), y_at(y), text, p)),
        }
    }
    surface
}

pub(super) fn map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    match panel.layout_view {
        LayoutView::Seam => seam_map(panel, state, width, p, cx),
        LayoutView::Windows => windows_map(panel, state, width, p, cx),
    }
}
