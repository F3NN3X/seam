//! The Pointer card: the remap switch, edge resistance, and the options behind "More pointer options".

use super::*;

/// The rows "More pointer options" opens, and its key.
const MORE: &str = "More pointer options";
const HIDDEN: [&str; 2] = ["Glued edge push", "Type measurements in Line Up"];
const NEEDS_REMAP: &str = "Needs Remap pointer at borders";

/// The Pointer card.
pub(super) fn card_of(panel: &mut Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Card {
    let open = panel.disclosure_open(MORE, &HIDDEN);
    let view = cx.entity();
    let on = panel.config.enabled;
    let remap = switch("remap", "Remap pointer at borders", on, window, cx, {
        let view = view.clone();
        move |on, window, cx| view.update(cx, |panel, cx| panel.patch(json!({ "enabled": on }), window, cx))
    });
    let measure = switch("lineup-measure", "Type measurements in Line Up", panel.config.lineup_measure, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "lineup_measure": on }), window, cx));
        }
    });
    let also_align =
        switch("align-arrangement", "Also align Windows arrangement", panel.config.align_arrangement, window, cx, {
            let view = view.clone();
            move |on, window, cx| {
                view.update(cx, |panel, cx| panel.patch(json!({ "align_arrangement": on }), window, cx));
            }
        });
    // The sliders follow the settings when something else changed them.
    let pointer_cfg = &panel.config.pointer;
    let resistance = pointer_cfg.edge_resistance_px.min(MAX_EDGE_RESISTANCE_PX);
    let push = pointer_cfg.glue_push_px.min(MAX_GLUE_PUSH_PX);
    for (state, value) in [(&panel.edge_resistance, resistance), (&panel.glue_push, push)] {
        if state.read(cx).value().end() != value as f32 {
            state.update(cx, |state, cx| state.set_value(value as f32, window, cx));
        }
    }
    let why = (!on).then_some(NEEDS_REMAP);
    let resistance_value = if resistance == 0 { "Off".into() } else { format!("{resistance} px") };
    let mut rows = vec![
        row("Remap pointer at borders", Some("Cross where the screens physically meet".into()), remap, p),
        ui::dimmed(
            ui::row(
                "Edge resistance",
                Some(
                    "Push this far past a screen's edge before the pointer crosses, so you can grab a window's edge \
                     there. 0 is off"
                        .into(),
                ),
                ui::slider_with_value(&panel.edge_resistance, resistance_value, on, "Edge resistance", p),
                p,
            ),
            "edge-resistance-off",
            why,
        ),
        row("Also align Windows arrangement", Some("Move screens in Display settings to match".into()), also_align, p),
        ui::disclosure_row(
            "more-pointer-options",
            MORE,
            "Glued edge push and typing measurements in Line Up",
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
        rows.push(ui::dimmed(
            ui::row(
                "Glued edge push",
                Some(
                    "How far to push past a glued edge before the pointer crosses. An auto-hide taskbar there shows \
                     while you push"
                        .into(),
                ),
                ui::slider_with_value(&panel.glue_push, format!("{push} px"), on, "Glued edge push", p),
                p,
            ),
            "glue-push-off",
            why,
        ));
        rows.push(row(
            "Type measurements in Line Up",
            Some("Adds a field to type how far the screens are offset, in cm".into()),
            measure,
            p,
        ));
    }
    card(rows, p)
}
