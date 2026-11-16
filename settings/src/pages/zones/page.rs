//! The Zones page itself.

use super::*;
use crate::theme::Palette;
use crate::ui::switch;
use seam::config::ZoneSettings;

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let zones_cfg = panel.config.zones.clone();
    let view = cx.entity();

    let header = header(&zones_cfg, &view, p, window, cx);
    let notice = other_wm_notice(panel);

    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(header)
        .children(notice.map(|text| ui::note(text, None, p)));
    let Some((mon, saved)) = chosen(panel).map(|(m, l)| (m.clone(), l)) else {
        let note = ui::description("Start Seam to set up zones", p);
        return body.child(note);
    };
    // The desk or screen changed under the editor, so its tree is for something else.
    let sig = panel.state.as_ref().map(|s| s.desk.sig.clone());
    if panel.editor.as_ref().is_some_and(|e| Some(&e.sig) != sig.as_ref() || e.screen != mon.id) {
        panel.editor = None;
    }
    // Built here, past the early return, so only sections that are drawn count as shown.
    let snapping = snapping_section(panel, &view, p, window, cx);
    let tabs = screen_tabs(panel, &view, window, cx);

    let number = panel.zone_screen.min(panel.mons.len() - 1) + 1;
    let (picker, own) = picker::section_of(panel, &mon, saved.as_ref(), number, p, window, cx);
    let stage = stage(panel, &mon, saved.as_ref(), own, number, window, cx);

    body = body
        .child(div().flex().child(tabs))
        .child(div().flex().flex_col().min_w_0().child(stage))
        .child(picker)
        .child(snapping);
    body
}

/// The title row, with the switch that turns zones on and off.
fn header(
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
    let subtitle = if zones_cfg.pause_to_snap {
        format!("Pause, or hold {}, while you drag a window to drop it into a zone", key_label(&zones_cfg.show_key))
    } else {
        format!("Hold {} while you drag a window to drop it into a zone", key_label(&zones_cfg.show_key))
    };
    let on = switch("zones-on", "Zones", zones_cfg.enabled, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "enabled": on } }), window, cx))
        }
    });
    ui::page_header("Zones", subtitle, Some("Zones"), p, cx).child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .child(div().text_size(px(14.)).text_color(p.text_secondary).child(if zones_cfg.enabled {
                "Zones on"
            } else {
                "Zones off"
            }))
            .child(on),
    )
}

/// Another window manager places the windows, so snapping steps aside (D110).
fn other_wm_notice(panel: &Panel) -> Option<String> {
    let detected = panel.state.as_ref().and_then(|s| s.wm.other_wm.clone());
    let other_wm = panel.config.windows.other_wm;
    match (&detected, other_wm) {
        (Some(name), _) => Some(format!(
            "{} is running, so snapping, zone keys and window moves are off. The pointer remap, workspaces and focus keys still work.",
            state::other_wm_title(name)
        )),
        (None, true) => Some(
            "Another window manager handles your windows, so snapping, zone keys and window moves are off. The pointer remap, workspaces and focus keys still work."
                .to_string(),
        ),
        (None, false) => None,
    }
}

/// The screen picker above the preview.
fn screen_tabs(panel: &Panel, view: &Entity<Panel>, window: &mut Window, cx: &mut Context<Panel>) -> Stateful<Div> {
    let labels: Vec<SharedString> = screen_names(panel).into_iter().map(|(_, name)| name).collect();
    ui::choice("zone-screen", "Screen", labels, None, panel.zone_screen.min(panel.mons.len() - 1), window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            view.update(cx, |panel, cx| {
                // An editor that's open carries on with the screen picked.
                let editing = panel.editor.as_ref().map(|e| e.opened.clone());
                let then: zone_editor::Then = Rc::new(move |panel, window, cx| {
                    panel.zone_screen = i;
                    if let Some(opened) = editing.clone() {
                        zone_editor::open(panel, opened, window, cx);
                    }
                    cx.notify();
                });
                zone_editor::guard(panel, window, cx, then);
            })
        }
    })
}

/// The screen preview with its footer, or the editor in its place.
fn stage(
    panel: &mut Panel,
    mon: &Mon,
    saved: Option<&ZoneLayout>,
    own: Option<Node>,
    number: usize,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    let gap_px = panel.config.zones.gap.min(32);
    if panel.editor.is_some() {
        let room = f64::from(window.viewport_size().width) - 240. - 64. - 20.;
        return zone_editor::view(panel, mon, gap_px, room, preview_height(mon, room), window, cx);
    }
    let caption =
        ui::description("Stored as parts of the screen, so a new resolution keeps the layout", p).flex_1().min_w_0();
    let edit_layout = ui::secondary("edit-layout", "Edit layout", cx)
        .on_click(cx.listener(|panel, _, window, cx| zone_editor::open(panel, Opened::Screen, window, cx)));
    let view = cx.entity().downgrade();
    let more = ui::secondary("preview-more", "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label("More for this layout")
        .dropdown_menu(move |menu, _, _| {
            let (saving, windows) = (view.clone(), view.clone());
            let tree = own.clone();
            menu.item(PopupMenuItem::new("Save as layout").disabled(tree.is_none()).on_click(move |_, window, cx| {
                let Some(tree) = tree.clone() else { return };
                let _ = saving.update(cx, |panel, cx| save_as_layout(panel, tree, window, cx));
            }))
            .item(PopupMenuItem::new("Save from windows").on_click(move |_, window, cx| {
                let _ = windows.update(cx, |panel, cx| {
                    panel.command("zones.from_windows", json!({ "number": number }), window, cx)
                });
            }))
        });
    let buttons = div().flex().flex_none().items_center().gap(px(tokens::SPACE_8)).child(edit_layout).child(more);
    let footer = div().flex().items_center().justify_between().gap(px(tokens::SPACE_16)).child(caption).child(buttons);
    preview(mon, saved, gap_px, footer, p).into_any_element()
}
