//! The chosen screen drawn with its zones.

use super::*;
use crate::theme::Palette;

/// The widest the drawn screen gets.
const PREVIEW_WIDTH: f64 = 480.;
/// How far each zone block is drawn inside its zone, so the card shows between neighbours.
const INSET: f32 = 1.;

/// How tall the editor's card is for a screen in `room`: the screen's shape, up to the most.
pub(super) fn preview_height(mon: &Mon, room: f64) -> f32 {
    let (w, h) = (mon.rect.width().max(1) as f64, mon.rect.height() as f64);
    (((room - PREVIEW_INSET) * h / w + PREVIEW_INSET) as f32).min(PREVIEW_MAX_HEIGHT)
}

/// The card: the screen at its own shape, and `footer` under it.
pub(super) fn preview(mon: &Mon, layout: Option<&ZoneLayout>, gap: u32, footer: Div, p: Palette) -> Div {
    let (w, h) = (mon.rect.width().max(1) as f64, mon.rect.height().max(1) as f64);
    let scale = (PREVIEW_WIDTH / w).min(PREVIEW_MAX_HEIGHT as f64 / h).max(0.01);
    let mut surface = div()
        .relative()
        .flex_none()
        .w(px((w * scale) as f32))
        .h(px((h * scale) as f32))
        .bg(p.surface_page)
        .border_1()
        .border_color(p.hairline_strong);
    if let Some(layout) = layout {
        let area = Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
        let (cells, drawn) = (zones::rects(&layout.tree, area, 0), zones::rects(&layout.tree, area, gap as i32));
        for (i, (cell, r)) in cells.iter().zip(&drawn).enumerate() {
            // Each block is inset a little, so neighbours read as separate shapes.
            let (from, size) = (
                |v: i32| px((v as f64 * scale) as f32 + INSET),
                |v: i32| px(((v as f64 * scale) as f32 - 2. * INSET).max(1.)),
            );
            surface = surface.child(
                div()
                    .id(("zone", i))
                    .absolute()
                    .left(from(r.x))
                    .top(from(r.y))
                    .w(size(r.w))
                    .h(size(r.h))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .bg(p.surface_zone)
                    .rounded(px(tokens::RADIUS_2))
                    .hover(|d| d.bg(p.accent_tint))
                    .child(text_style(div(), tokens::CARD_TITLE).child((i + 1).to_string()))
                    .child(ui::tabular(
                        text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(format!("{} px", cell.w)),
                    )),
            );
        }
    } else {
        surface = surface.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::description("No zones on this screen. Pick a layout", p)),
        );
    }
    div()
        .flex()
        .flex_col()
        .bg(p.surface_card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(tokens::RADIUS_12))
        .child(div().flex().justify_center().p(px(tokens::SPACE_16)).child(surface))
        .child(
            div().border_t_1().border_color(p.hairline).px(px(tokens::SPACE_16)).py(px(tokens::SPACE_12)).child(footer),
        )
}
