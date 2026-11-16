//! The physical map: screens and the lines between them, in the desk and Windows views.

use super::borders::{Kind, kind, off_desk};
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

/// One screen. `on_desk` is false for a screen the desk has no place for: no fill, a dashed edge.
fn screen_tile(
    panel: &Panel,
    index: usize,
    (x, y, w, h): Frame,
    (sized, on_desk): (bool, bool),
    (view, p): (&View, Palette),
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> impl IntoElement + use<> {
    let number = index + 1;
    let name = panel.mons.get(index).map_or("", |m| m.name.as_str()).to_string();
    let primary = panel.mons.get(index).is_some_and(|m| m.primary);
    let label = match (on_desk, primary) {
        (false, _) => format!("{name}, not on your desk"),
        (true, true) => format!("{name}, main"),
        (true, false) => name,
    };
    let height = h * view.scale;
    let tile = cx.entity();
    // A click flashes the screen's number on it, so Tab reaches it and Enter does the same.
    let identify = move |window: &mut Window, cx: &mut App| {
        tile.update(cx, |panel, cx| panel.command("screen.identify", json!({ "number": number }), window, cx));
    };
    let name = format!("Screen {number}, {label}. Show its number on the screen");
    ui::selectable_ringed(("screen", index), name, false, (tokens::RADIUS_2, 1.), window, cx, identify)
        .absolute()
        .left(px((x * view.scale + view.left) as f32))
        .top(px((y * view.scale + view.top) as f32))
        .w(px((w * view.scale) as f32))
        .h(px(height as f32))
        .flex()
        .when(height >= SHORT_TILE, |d| d.flex_col())
        .when(height < SHORT_TILE, |d| d.gap(px(tokens::SPACE_8)))
        .items_center()
        .justify_center()
        .overflow_hidden()
        .when(on_desk, |d| d.bg(p.surface_card))
        .border_1()
        .border_color(p.hairline_strong)
        .when(!sized || !on_desk, |d| d.border_dashed())
        .rounded(px(tokens::RADIUS_2))
        .cursor_pointer()
        .child(text_style(div(), tokens::CARD_TITLE).child(number.to_string()))
        .child(
            text_style(div(), tokens::MICRO)
                .text_color(if on_desk { p.text_secondary } else { p.text_muted })
                .truncate()
                .child(label),
        )
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Line {
    Solid,
    Dashed,
    Dotted,
}

/// How a kind of border is drawn: its line, and its colour.
pub(super) fn look(kind: Kind, p: Palette) -> (Line, Hsla) {
    match kind {
        Kind::LinedUp => (Line::Solid, p.accent_ink),
        Kind::Glued => (Line::Dashed, p.accent_ink),
        Kind::NotOnDesk => (Line::Dotted, p.text_muted),
        Kind::Loose => (Line::Dashed, p.text_muted),
    }
}

/// Where a line runs on the map, in map pixels.
pub(super) struct Run {
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
pub(super) fn stroke(run: &Run, width: f32, style: Line, color: Hsla) -> Vec<Div> {
    let (on, off) = match style {
        Line::Solid => (run.end - run.start, 0.),
        Line::Dashed => (6., 4.),
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

/// A short sample of how a kind of border is drawn, for the strip and the legend.
pub(super) fn sample(kind: Kind, p: Palette) -> Div {
    const WIDTH: f64 = 24.;
    let (style, color) = look(kind, p);
    let run = Run { along_y: false, start: 0., end: WIDTH, across: 2. };
    div().relative().flex_none().w(px(WIDTH as f32)).h(px(4.)).children(stroke(&run, 2., style, color))
}

/// A small chip centred on a point of the map.
fn chip_at(x: f64, y: f64, text: String, p: Palette) -> Div {
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
        .child(
            text_style(div(), tokens::MICRO)
                .px(px(tokens::SPACE_4))
                .bg(p.well())
                .rounded(px(tokens::RADIUS_4))
                .text_color(p.text_muted)
                .child(text),
        )
}

/// The line for one border on the desk view, and the 16 px strip that selects it. The line is
/// 4 px while its border is the selected one.
fn desk_border(
    panel: &Panel,
    at: usize,
    (a, other, b): (&Placed, &Placed, &Border),
    view: &View,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<AnyElement> {
    let which = kind(b);
    let (style, color) = look(which, p);
    let run = if b.not_on_desk { windows_run(panel, a, other, b.side, view) } else { seam_run(a, other, b.side, view) };
    let Some(run) = run else { return vec![] };
    let selected = panel.border_pick == Some((b.a, b.b));
    let mut parts: Vec<AnyElement> = stroke(&run, if selected { 4. } else { SEAM_WIDTH }, style, color)
        .into_iter()
        .map(IntoElement::into_any_element)
        .collect();
    let (from, to) = (b.a, b.b);
    let view = cx.entity();
    let pick = move |_: &mut Window, cx: &mut App| {
        view.update(cx, |panel, cx| {
            panel.border_pick = Some((from, to));
            cx.notify();
        })
    };
    let label = format!("Border between screen {from} and screen {to}");
    let hit = ui::selectable(("border", at), label, selected, window, cx, pick).absolute();
    let (start, length) = (run.start as f32, (run.end - run.start) as f32);
    let across = run.across as f32 - HIT_WIDTH / 2.;
    parts.push(
        if run.along_y {
            hit.left(px(across)).top(px(start)).w(px(HIT_WIDTH)).h(px(length))
        } else {
            hit.top(px(across)).left(px(start)).h(px(HIT_WIDTH)).w(px(length))
        }
        .into_any_element(),
    );
    parts
}

/// Where each screen sits on the desk, joined where you lined them up.
fn desk_map(panel: &Panel, state: &State, width: f64, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let frames: Vec<Frame> = panel.layout.iter().map(|s| (s.x, s.y, s.w, s.h)).collect();
    let view = fit(&frames, width);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.well()).rounded(px(tokens::RADIUS_8));
    for (s, frame) in panel.layout.iter().zip(&frames) {
        let on_desk = !off_desk(state, s.index + 1);
        surface = surface.child(screen_tile(panel, s.index, *frame, (s.sized, on_desk), (&view, p), window, cx));
    }
    for (at, b) in state.borders.iter().enumerate() {
        let find = |number: u32| panel.layout.iter().find(|s| s.index + 1 == number as usize);
        if let (Some(a), Some(other)) = (find(b.a), find(b.b)) {
            surface = surface.children(desk_border(panel, at, (a, other, b), &view, p, window, cx));
        }
    }
    surface.child(
        div()
            .absolute()
            .left(px(tokens::SPACE_16))
            .bottom(px(tokens::SPACE_8))
            .flex()
            .flex_col()
            .gap(px(tokens::SPACE_2))
            .text_color(p.text_muted)
            .child(div().w(px((SCALE_BAR_MM * view.scale) as f32)).h(px(1.)).bg(p.text_muted))
            .child(text_style(div(), tokens::MICRO).child("20 cm")),
    )
}

/// A chip for where two screens face each other across a gap or overlap in Windows.
struct Chip {
    x: i32,
    y: i32,
    text: String,
}

fn windows_chips(rects: &[Rect]) -> Vec<Chip> {
    let mut found = vec![];
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            if touching(a, b).is_some() {
                continue;
            }
            if let Some((side, gap)) = facing_gap(a, b) {
                let c = corridor(a, b, side);
                found.push(Chip {
                    x: (c.left + c.right) / 2,
                    y: (c.top + c.bottom) / 2,
                    text: format!("{gap} px gap"),
                });
            } else if a.intersects(b) {
                let (left, right) = (a.left.max(b.left), a.right.min(b.right));
                let (top, bottom) = (a.top.max(b.top), a.bottom.min(b.bottom));
                let overlap = (right - left).min(bottom - top);
                if (1..=LABEL_PX).contains(&overlap) {
                    let text = format!("{overlap} px overlap");
                    found.push(Chip { x: (left + right) / 2, y: (top + bottom) / 2, text });
                }
            }
        }
    }
    found
}

/// The screens as Windows arranges them, in pixels, with a chip where they don't meet.
fn windows_map(
    panel: &Panel,
    state: &State,
    width: f64,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
    let frames: Vec<Frame> = state
        .screens
        .iter()
        .map(|s| (f64::from(s.rect.x), f64::from(s.rect.y), f64::from(s.rect.w), f64::from(s.rect.h)))
        .collect();
    let rects: Vec<Rect> = state.screens.iter().map(|s| Rect::new(s.rect.x, s.rect.y, s.rect.w, s.rect.h)).collect();
    let view = fit(&frames, width);
    let (x_at, y_at) = (|v: i32| f64::from(v) * view.scale + view.left, |v: i32| f64::from(v) * view.scale + view.top);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.well()).rounded(px(tokens::RADIUS_8));
    for (s, frame) in state.screens.iter().zip(&frames) {
        let index = (s.number as usize).saturating_sub(1);
        surface = surface.child(screen_tile(panel, index, *frame, (true, true), (&view, p), window, cx));
    }
    for chip in windows_chips(&rects) {
        surface = surface.child(chip_at(x_at(chip.x), y_at(chip.y), chip.text, p));
    }
    surface
}

pub(super) fn map(
    panel: &Panel,
    state: &State,
    width: f64,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
    match panel.layout_view {
        LayoutView::Seam => desk_map(panel, state, width, p, window, cx),
        LayoutView::Windows => windows_map(panel, state, width, p, window, cx),
    }
}
