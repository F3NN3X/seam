//! Windows: how Seam tiles, moves and marks the windows on your screens.

use crate::theme::palette;
use crate::ui;
use gpui_kit::*;

/// Only the header for now; the rows move in from Zones and Where apps open.
pub fn render(cx: &mut App) -> impl IntoElement + use<> {
    let p = palette(cx);
    div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(ui::page_header(
        "Windows",
        "How Seam tiles, moves and marks the windows on your screens",
        p,
    ))
}
