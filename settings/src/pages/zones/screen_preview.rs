//! The chosen screen drawn with its zones.

use super::*;
use crate::theme::Palette;

/// How tall the preview card is for a screen in `room`: the screen's shape, up to the most.
pub(super) fn preview_height(mon: &Mon, room: f64) -> f32 {
    let (w, h) = (mon.rect.width().max(1) as f64, mon.rect.height() as f64);
    (((room - PREVIEW_INSET) * h / w + PREVIEW_INSET) as f32).min(PREVIEW_MAX_HEIGHT)
}

pub(super) fn preview(mon: &Mon, layout: Option<&ZoneLayout>, gap: u32, room: f64, height: f32, p: Palette) -> Div {
    let card = div()
        .flex()
        .items_center()
        .justify_center()
        .flex_1()
        .min_w_0()
        .h(px(height))
        .p(px(16.))
        .bg(p.card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(12.));
    let (w, h) = (mon.rect.width() as f64, mon.rect.height() as f64);
    let scale = ((room - PREVIEW_INSET) / w).min((height as f64 - PREVIEW_INSET) / h).max(0.01);
    let mut surface = div().relative().w(px((w * scale) as f32)).h(px((h * scale) as f32));
    let Some(layout) = layout else {
        let empty = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_dashed()
            .border_color(p.hairline_strong)
            .rounded(px(8.))
            .child(ui::description("No zones on this screen. Pick a layout", p));
        return card.child(surface.child(empty));
    };
    let area = Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
    let (cells, drawn) = (zones::rects(&layout.tree, area, 0), zones::rects(&layout.tree, area, gap as i32));
    for (i, (cell, r)) in cells.iter().zip(&drawn).enumerate() {
        let at = |v: i32| px((v as f64 * scale) as f32);
        surface = surface.child(
            div()
                .id(("zone", i))
                .absolute()
                .left(at(r.x))
                .top(at(r.y))
                .w(at(r.w))
                .h(at(r.h))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .bg(p.raised)
                .border_1()
                .border_color(p.hairline_strong)
                .rounded(px(6.))
                .hover(|d| d.bg(p.accent.opacity(0.1)).border_color(p.accent))
                .child(div().text_size(px(24.)).child((i + 1).to_string()))
                .child(ui::tabular(div().text_size(px(13.)).text_color(p.secondary).child(format!("{} px", cell.w)))),
        );
    }
    card.child(surface)
}
