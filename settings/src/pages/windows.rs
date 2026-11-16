//! Windows: how Seam tiles, moves and marks the windows on your screens.

use crate::panel::Panel;
use crate::theme::{Palette, palette};
use crate::ui::{self, Row, card, row, section, switch};
use gpui_kit::*;
use seam::state;
use serde_json::{Value, json};

mod apps;
mod colours;
mod moving;
mod tiling;

pub use colours::picked;

/// A switch row whose change patches the config with `patch`.
fn toggle(
    (id, title, about): (&'static str, &'static str, impl Into<SharedString>),
    (on, patch): (bool, fn(bool) -> Value),
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let view = view.clone();
    let control = switch(id, title, on, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| panel.patch(patch(on), window, cx))
    });
    row(title, Some(about.into()), control, p)
}

/// The Borders section: the focus border and the pin border, each a Select of colours.
fn borders_section(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> AnyElement {
    let windows = &panel.config.windows;
    let (focus_picker, pin_picker) = (panel.focus_picker.clone(), panel.pin_picker.clone());
    let focus = colours::colour_select(
        ("focus_color", "Focus border"),
        !windows.focus_border,
        (&windows.focus_color, seam::palette::FOCUS_DEFAULT),
        &focus_picker,
        window,
        cx,
    );
    let pin = colours::colour_select(
        ("pin_color", "Pin border"),
        false,
        (&windows.pin_color, seam::palette::PIN_DEFAULT),
        &pin_picker,
        window,
        cx,
    );
    let rows = vec![
        row(
            "Focus border",
            Some("Windows accent follows Settings › Personalisation › Colours. None turns it off".into()),
            focus,
            p,
        ),
        row("Pin border", Some("Border of a window pinned on top or kept out of workspace switches".into()), pin, p),
    ];
    section("Borders", card(rows, p), p)
}

/// The Pinned windows section.
fn pinned_section(
    panel: &Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let windows = &panel.config.windows;
    let rows = vec![
        toggle(
            (
                "notify-pinned-close",
                "Tell me when a pinned window closes",
                "Click the notification to start the app again and pin its window",
            ),
            (windows.notify_pinned_close, |on| json!({ "windows": { "notify_pinned_close": on } })),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            (
                "pins-stay-on-switch",
                "Pinned windows stay when I switch workspaces",
                "A window pinned on top isn't moved, minimized or closed, and capture still saves it",
            ),
            (windows.pins_stay_on_switch, |on| json!({ "windows": { "pins_stay_on_switch": on } })),
            view,
            p,
            window,
            cx,
        ),
    ];
    section("Pinned windows", card(rows, p), p)
}

/// The Games and other window managers section.
fn others_section(
    panel: &Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let detected = panel.state.as_ref().and_then(|s| s.wm.other_wm.clone());
    let other_about = match &detected {
        Some(name) => format!("{} found running", state::other_wm_title(name)),
        None => "Snapping and window moves step aside. The pointer remap keeps working".to_string(),
    };
    let rows = vec![
        toggle(
            (
                "step-aside-fullscreen",
                "Step aside for games and full-screen apps",
                "Shortcuts, snapping and Spotlight wait until you leave it",
            ),
            (panel.config.step_aside_fullscreen, |on| json!({ "step_aside_fullscreen": on })),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            ("other-wm", "Another window manager handles my windows", &other_about),
            (panel.config.windows.other_wm, |on| json!({ "windows": { "other_wm": on } })),
            view,
            p,
            window,
            cx,
        ),
    ];
    section("Games and other window managers", card(rows, p), p)
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let view = cx.entity();
    let header = ui::page_header("Windows", "How Seam tiles, moves and marks the windows on your screens", p);
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(header)
        .child(tiling::tiling_section(panel, &view, p, window, cx))
        .child(borders_section(panel, p, window, cx))
        .child(pinned_section(panel, &view, p, window, cx))
        .child(moving::moving_section(panel, &view, p, window, cx))
        .child(apps::section_view(panel, window, cx))
        .child(others_section(panel, &view, p, window, cx))
}
