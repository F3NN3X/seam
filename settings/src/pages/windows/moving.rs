//! The Moving and resizing section: the size a window keeps on another screen, the key to move
//! it from anywhere, and the corner that opens the overview.

use crate::panel::Panel;
use crate::theme::Palette;
use crate::ui::{self, Choice, Row, card, row, section, switch};
use gpui_kit::*;
use seam::config::{WindowSettings, WindowSize};
use serde_json::{Value, json};

/// The width of the two Selects.
const WIDTH: f32 = 168.;
const OFF: &str = "off";
/// "When a window moves to another screen", in the order the control shows them.
const SIZE_MODES: [WindowSize; 3] = [WindowSize::Windows, WindowSize::Pixels, WindowSize::Real];
const SIZE_LABELS: [&str; 3] = ["Let Windows scale it", "Keep pixel size", "Keep real size"];
/// Config value and label of each key "Move and resize from anywhere in a window" can be held with.
const DRAG_KEYS: [(&str, &str); 3] = [("alt", "Alt"), ("win", "Win"), ("ctrl_alt", "Ctrl+Alt")];
/// The corners "Open the overview from a corner" offers.
const CORNERS: [(&str, &str); 4] = [
    ("top_left", "Top left"),
    ("top_right", "Top right"),
    ("bottom_left", "Bottom left"),
    ("bottom_right", "Bottom right"),
];

/// Under "When a window moves to another screen", only for a real size: the screens
/// that don't report their size, where Windows scales windows instead.
fn size_caption(panel: &Panel, mode: WindowSize) -> Option<SharedString> {
    if mode != WindowSize::Real {
        return None;
    }
    let no_size: Vec<String> =
        panel.mons.iter().enumerate().filter(|(_, m)| m.raw_dpi <= 0.).map(|(i, _)| (i + 1).to_string()).collect();
    match no_size.as_slice() {
        [] => None,
        [n] => Some(format!("Screen {n} doesn't report its size, so windows there are scaled by Windows").into()),
        [first @ .., last] => Some(
            format!(
                "Screens {} and {last} don't report their size, so windows there are scaled by Windows",
                first.join(", ")
            )
            .into(),
        ),
    }
}

/// The rows for a window that moves to another screen, and for one dragged there.
fn size_rows(
    panel: &Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> [Row; 2] {
    let size_mode = panel.config.window_size_on_move;
    let size_at = SIZE_MODES.iter().position(|m| *m == size_mode).unwrap_or(0);
    let size_choice = ui::segmented(
        "window-size-on-move",
        "When a window moves to another screen",
        &SIZE_LABELS,
        size_at,
        window,
        cx,
        {
            let view = view.clone();
            move |i, window, cx| {
                let mode = SIZE_MODES.get(i).copied().unwrap_or_default();
                view.update(cx, |panel, cx| panel.patch(json!({ "window_size_on_move": mode }), window, cx))
            }
        },
    );
    // Only Seam's own sizes apply to a drag, so with Windows' scaling it can't be used.
    let scaled = size_mode == WindowSize::Windows;
    let on_drag = switch(
        "window-size-on-drag",
        "Also when I drag a window to another screen",
        panel.config.window_size_on_drag,
        window,
        cx,
        {
            let view = view.clone();
            move |on, window, cx| {
                if !scaled {
                    view.update(cx, |panel, cx| panel.patch(json!({ "window_size_on_drag": on }), window, cx));
                }
            }
        },
    );
    let about = size_caption(panel, size_mode)
        .unwrap_or("Real size keeps it the same width in centimetres across screens".into());
    let on_drag = row(
        "Also when I drag a window to another screen",
        Some("Needs Keep pixel size or Keep real size".into()),
        on_drag,
        p,
    );
    [
        row("When a window moves to another screen", Some(about), size_choice, p),
        ui::dimmed(
            on_drag,
            "window-size-on-drag-row",
            scaled.then_some("Pick Keep pixel size or Keep real size first"),
        ),
    ]
}

/// What an Off-first Select of `on` and a key writes into a `windows` setting.
fn patch_for(setting: &'static str, field: &'static str, key: &str) -> Value {
    match key {
        OFF => json!({ "windows": { setting: { "on": false } } }),
        key => json!({ "windows": { setting: { "on": true, field: key } } }),
    }
}

/// An Off-first Select over `options`, for a setting that is a switch and a choice in the config.
fn off_select(
    (id, title): (&'static str, &'static str),
    (setting, field): (&'static str, &'static str),
    options: &[(&'static str, &'static str)],
    selected: &str,
    view: &Entity<Panel>,
    cx: &App,
) -> AnyElement {
    let choices =
        [Choice::new(OFF, "Off")].into_iter().chain(options.iter().map(|(k, l)| Choice::new(*k, *l))).collect();
    let view = view.clone();
    ui::select(id, title, choices, selected, WIDTH, cx, move |key, window, cx| {
        let patch = patch_for(setting, field, key);
        view.update(cx, |panel, cx| panel.patch(patch, window, cx));
    })
}

/// "Move and resize from anywhere in a window": Off, or the key to hold.
fn alt_drag_row(windows: &WindowSettings, view: &Entity<Panel>, p: Palette, cx: &App) -> Row {
    let title = "Move and resize from anywhere in a window";
    let drag = &windows.alt_drag;
    // A key the config holds that the menu doesn't know reads as Alt.
    let key = DRAG_KEYS.iter().find(|(k, _)| *k == drag.key).map_or("alt", |(k, _)| *k);
    let selected = if drag.on { key } else { OFF };
    let select = off_select(("alt-drag", title), ("alt_drag", "key"), &DRAG_KEYS, selected, view, cx);
    let about = "Hold a key and drag to move, or right-drag to resize. Win leaves Alt+click to apps";
    row(title, Some(about.into()), select, p)
}

/// "Open the overview from a corner": Off, or the corner.
fn corner_row(windows: &WindowSettings, view: &Entity<Panel>, p: Palette, cx: &App) -> Row {
    let title = "Open the overview from a corner";
    let corner = &windows.corner_overview;
    let key = CORNERS.iter().find(|(k, _)| *k == corner.corner).map_or("top_left", |(k, _)| *k);
    let selected = if corner.on { key } else { OFF };
    let select = off_select(("corner-overview", title), ("corner_overview", "corner"), &CORNERS, selected, view, cx);
    let about = "Rest the pointer in a corner of the whole desk, with no button down. Off, or pick the corner";
    row(title, Some(about.into()), select, p)
}

/// The Moving and resizing section.
pub(super) fn moving_section(
    panel: &Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let [size, on_drag] = size_rows(panel, view, p, window, cx);
    let windows = &panel.config.windows;
    let rows = vec![size, on_drag, alt_drag_row(windows, view, p, cx), corner_row(windows, view, p, cx)];
    section("Moving and resizing", card(rows, p), p)
}
