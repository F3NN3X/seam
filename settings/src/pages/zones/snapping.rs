//! The Snapping section: the keys, the gap and margins, the switches, and Reset to default.

use super::*;
use crate::theme::Palette;
use crate::ui::{Row, section_with, switch};
use seam::config::ZoneSettings;

/// The row "More snapping options" opens, and the rows behind it.
const MORE: &str = "More snapping options";
const HIDDEN: [&str; 6] = [
    "Restore size when dragged out",
    "Show keys while dragging",
    "Resize neighbours with a window",
    "Handles between snapped windows",
    "Edit layouts on the screen",
    "Keep windows in their zones",
];
/// The settings Reset to default puts back. Screen margins are per screen, so they stay.
const RESET: [&str; 12] = [
    "show_key",
    "span_key",
    "gap",
    "pause_to_snap",
    "swap_on_drop",
    "snap_beside",
    "restore_size",
    "drag_hints",
    "resize_neighbours",
    "divider_handles",
    "edit_on_screen",
    "refit",
];
/// The place of Pause in "Show zones while dragging", after the three keys.
const PAUSE: usize = 3;

/// A switch row that patches one of the zone settings.
fn toggle(
    (id, title, about): (&'static str, &'static str, &'static str),
    (key, on): (&'static str, bool),
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let view = view.clone();
    let control = switch(id, title, on, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| panel.patch(json!({ "zones": { key: on } }), window, cx))
    });
    row(title, Some(about.into()), control, p)
}

/// "Show zones while dragging": the three keys, or Pause for holding the window still.
fn show_row(
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let key_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.show_key).unwrap_or(0);
    let span_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.span_key).unwrap_or(1);
    let selected = if zones_cfg.pause_to_snap { PAUSE } else { key_at };
    let show = ui::segmented(
        "zone-show-key",
        "Show zones while dragging",
        &["Shift", "Ctrl", "Alt", "Pause"],
        selected,
        window,
        cx,
        {
            let view = view.clone();
            move |i, window, cx| {
                let zones = if i == PAUSE {
                    json!({ "pause_to_snap": true })
                } else {
                    // The span key can't be the same key, so it moves to the next one.
                    let mut zones = json!({ "show_key": KEYS[i].0, "pause_to_snap": false });
                    if KEYS[i].0 == KEYS[span_at].0 {
                        zones["span_key"] = json!(KEYS[(i + 1) % KEYS.len()].0);
                    }
                    zones
                };
                view.update(cx, |panel, cx| panel.patch(json!({ "zones": zones }), window, cx));
            }
        },
    );
    // Pause adds to the key in the tray, so a config with both says so.
    let about = if zones_cfg.pause_to_snap {
        format!("Pause, or hold {}, to show zones", KEYS[key_at].1)
    } else {
        "Hold a key, or pause with the window still".to_string()
    };
    row("Show zones while dragging", Some(about.into()), show, p)
}

/// "Span zones": the key that covers more than one zone. It can't be the one that shows them.
fn span_row(
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let show_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.show_key).unwrap_or(0);
    let span_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.span_key).unwrap_or(1);
    let labels = SPAN_ORDER.iter().map(|i| SharedString::from(KEYS[*i].1)).collect();
    let blocked = SPAN_ORDER.iter().position(|i| *i == show_at).map(|at| {
        let why = [
            "Ctrl is already used to show zones",
            "Shift is already used to show zones",
            "Alt is already used to show zones",
        ];
        (at, why[SPAN_ORDER[at]])
    });
    let selected = SPAN_ORDER.iter().position(|i| *i == span_at).unwrap_or(0);
    let span = ui::choice("zone-span-key", "Span zones", labels, blocked, selected, window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            let key = KEYS[SPAN_ORDER[i]].0;
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "span_key": key } }), window, cx));
        }
    });
    row("Span zones", Some("Add this key to cover more than one zone".into()), span, p)
}

/// Everything in the section that has a switch of its own, behind "More snapping options".
fn more_rows(
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<Row> {
    let mut rows = vec![
        toggle(
            ("zone-restore", "Restore size when dragged out", "The window gets its old size back"),
            ("restore_size", zones_cfg.restore_size),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            (
                "zone-hints",
                "Show keys while dragging",
                "A strip on the screen names the wheel, right-click and the keys",
            ),
            ("drag_hints", zones_cfg.drag_hints),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            ("zone-neighbours", "Resize neighbours with a window", "Windows in the zones next to it follow its edge"),
            ("resize_neighbours", zones_cfg.resize_neighbours),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            ("zone-handles", "Handles between snapped windows", "Drag one to resize both sides. Right-click for sizes"),
            ("divider_handles", zones_cfg.divider_handles),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            (
                "zone-on-screen",
                "Edit layouts on the screen",
                "Off shows a live view on the screen, and you edit in the preview here",
            ),
            ("edit_on_screen", zones_cfg.edit_on_screen),
            view,
            p,
            window,
            cx,
        ),
    ];
    rows.push(toggle(
        (
            "zone-refit",
            "Keep windows in their zones",
            "Experimental. Moves snapped windows back after a screen or resolution change",
        ),
        ("refit", zones_cfg.refit),
        view,
        p,
        window,
        cx,
    ));
    rows
}

/// Put every row of the section back to its default, and offer Undo.
fn reset(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let (Ok(now), Ok(default)) =
        (serde_json::to_value(&panel.config.zones), serde_json::to_value(ZoneSettings::default()))
    else {
        return;
    };
    let pick = |from: &Value| -> Value {
        RESET.iter().filter_map(|key| from.get(*key).map(|v| (key.to_string(), v.clone()))).collect()
    };
    let (before, after) = (pick(&now), pick(&default));
    panel.patch(json!({ "zones": after }), window, cx);
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        panel.patch(json!({ "zones": before.clone() }), window, cx);
    };
    panel.offer_undo("Snapping is back to its defaults".to_string(), undo, window, cx);
}

/// The Snapping section.
pub(super) fn snapping_section(
    panel: &mut Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let zones_cfg = panel.config.zones.clone();
    let open = panel.disclosure_open(MORE, &HIDDEN);
    // The slider follows the setting when something else changed it.
    let gap = zones_cfg.gap.min(32);
    if panel.gap.read(cx).value().end() != gap as f32 {
        panel.gap.update(cx, |slider, cx| slider.set_value(gap as f32, window, cx));
    }
    let gap_row = row(
        "Gap between zones",
        None,
        ui::slider_with_value(&panel.gap, format!("{gap} px"), true, "Gap between zones", p),
        p,
    );
    let mut rows = vec![
        show_row(&zones_cfg, view, p, window, cx),
        span_row(&zones_cfg, view, p, window, cx),
        gap_row,
        margins::row(panel, p, window, cx),
        toggle(
            ("zone-swap", "Swap with the window already in a zone", "Off lets windows overlap in a zone"),
            ("swap_on_drop", zones_cfg.swap_on_drop),
            view,
            p,
            window,
            cx,
        ),
        toggle(
            ("zone-beside", "Snap beside a window", "Drop on a window's edge to put the two side by side"),
            ("snap_beside", zones_cfg.snap_beside),
            view,
            p,
            window,
            cx,
        ),
        ui::disclosure_row(
            "more-snapping-options",
            MORE,
            "Handles between windows, resizing neighbours, keeping windows in their zones and 3 more",
            open,
            window,
            cx,
            {
                let view = view.clone();
                move |_, cx| {
                    view.update(cx, |panel, cx| {
                        panel.toggle_disclosure(MORE);
                        cx.notify();
                    })
                }
            },
        ),
    ];
    if open {
        rows.extend(more_rows(&zones_cfg, view, p, window, cx));
    }
    let reset = ui::secondary("reset-snapping", "Reset to default", cx)
        .ghost()
        .small()
        .on_click(cx.listener(|panel, _, window, cx| reset(panel, window, cx)))
        .into_any_element();
    section_with("Snapping", None, Some(reset), card(rows, p), p)
}
