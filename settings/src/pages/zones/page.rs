//! The Zones page itself.

use super::*;
use crate::pages::app_rules;
use crate::theme::Palette;
use crate::ui::{Card, Row, section, switch};
use seam::config::{WindowSettings, ZoneSettings};
use seam::tiling::Family;

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
    let windows = windows_section(panel, &view, p, window, cx);
    let tabs = screen_tabs(panel, &view, window, cx);

    let number = panel.zone_screen.min(panel.mons.len() - 1) + 1;
    let (picker, own) = picker::section_of(panel, &mon, saved.as_ref(), number, p, window, cx);
    let stage = stage(panel, &mon, saved.as_ref(), own, number, window, cx);
    let tiling = tiling_card(panel, number, p, window, cx);

    // Smart tiling, Windows and Where apps open are Windows page rows that haven't moved yet.
    body = body
        .child(div().flex().child(tabs))
        .child(div().flex().flex_col().min_w_0().child(stage))
        .child(picker)
        .child(snapping)
        .child(tiling)
        .child(windows)
        .child(app_rules::view(panel, cx));
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
        "Pause while you drag a window to drop it into a zone".to_string()
    } else {
        format!("Hold {} while you drag a window to drop it into a zone", key_label(&zones_cfg.show_key))
    };
    let on = switch("zones-on", "Zones", zones_cfg.enabled, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "enabled": on } }), window, cx))
        }
    });
    ui::page_header("Zones", subtitle, p).child(
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

/// The Windows section: focus and pin borders, alt-drag, screen moves, other window managers.
fn windows_section(
    panel: &Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let detected = panel.state.as_ref().and_then(|s| s.wm.other_wm.clone());
    let other_wm = panel.config.windows.other_wm;
    let windows_cfg = panel.config.windows.clone();
    let border = switch("focus-border", "Focus border", windows_cfg.focus_border, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "windows": { "focus_border": on } }), window, cx))
        }
    });
    let manual = switch("other-wm", "Another window manager handles my windows", other_wm, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "windows": { "other_wm": on } }), window, cx))
        }
    });
    let notify_close = switch(
        "notify-pinned-close",
        "Tell me when a pinned window closes",
        windows_cfg.notify_pinned_close,
        window,
        cx,
        {
            let view = view.clone();
            move |on, window, cx| {
                view.update(cx, |panel, cx| {
                    panel.patch(json!({ "windows": { "notify_pinned_close": on } }), window, cx)
                })
            }
        },
    );
    let pins_stay = switch(
        "pins-stay-on-switch",
        "Pinned windows stay when I switch workspaces",
        windows_cfg.pins_stay_on_switch,
        window,
        cx,
        {
            let view = view.clone();
            move |on, window, cx| {
                view.update(cx, |panel, cx| {
                    panel.patch(json!({ "windows": { "pins_stay_on_switch": on } }), window, cx)
                })
            }
        },
    );
    let drag_row = alt_drag_row(&windows_cfg, view, p, window, cx);
    let corner_row = corner_overview_row(&windows_cfg, view, p, window, cx);
    let size_mode = panel.config.window_size_on_move;
    let (size_row, on_drag_row) = size_rows(panel, size_mode, view, p, window, cx);
    let other_about = match &detected {
        Some(name) => format!("{} found running", state::other_wm_title(name)),
        None => "Snapping and window moves step aside. The pointer remap keeps working.".to_string(),
    };
    let border_off = (!windows_cfg.focus_border).then_some("Turn on Focus border to pick a colour");
    let (focus_picker, pin_picker) = (panel.focus_picker.clone(), panel.pin_picker.clone());
    let focus_swatches = swatches(
        "focus_color",
        &focus_picker,
        &windows_cfg.focus_color,
        colours::FOCUS_DEFAULT,
        border_off,
        window,
        cx,
    );
    let pin_swatches =
        swatches("pin_color", &pin_picker, &windows_cfg.pin_color, colours::PIN_DEFAULT, None, window, cx);
    let focus_colour_row = row(
        "Focus border colour",
        Some("Windows accent follows Settings > Personalisation > Colours".into()),
        focus_swatches,
        p,
    );
    let pin_colour_row = row(
        "Pin border colour",
        Some("Border of a window pinned on top or kept out of workspace switches".into()),
        pin_swatches,
        p,
    );
    let windows_card = card(
        vec![
            row("Focus border", Some("Colour the border of the window that has focus".into()), border, p),
            focus_colour_row
                .map(|el| div().when(border_off.is_some(), |d| d.opacity(0.4)).child(el).into_any_element()),
            pin_colour_row,
            row(
                "Tell me when a pinned window closes",
                Some("Click the notification to start the app again and pin its window".into()),
                notify_close,
                p,
            ),
            row(
                "Pinned windows stay when I switch workspaces",
                Some("A window pinned on top isn't moved, minimized or closed, and capture still saves it".into()),
                pins_stay,
                p,
            ),
            drag_row,
            corner_row,
            size_row,
            on_drag_row,
            row("Another window manager handles my windows", Some(other_about.into()), manual, p),
        ],
        p,
    );
    section("Windows", windows_card, p)
}

/// "Move and resize from anywhere in a window", with the key to hold.
fn alt_drag_row(
    windows_cfg: &WindowSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let alt_drag = windows_cfg.alt_drag.clone();
    let drag_key = DRAG_KEYS.iter().find(|(k, _)| *k == alt_drag.key).map_or("Alt", |(_, label)| label);
    let drag_switch = switch("alt-drag", "Move and resize from anywhere in a window", alt_drag.on, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "windows": { "alt_drag": { "on": on } } }), window, cx))
        }
    });
    let drag_menu = ui::secondary("alt-drag-key", drag_key, cx)
        .dropdown_caret(true)
        .accessibility_label(format!("Key to hold: {drag_key}"))
        .dropdown_menu({
            let view = view.downgrade();
            move |mut menu, _, _| {
                for (key, label) in DRAG_KEYS {
                    let view = view.clone();
                    let item = PopupMenuItem::new(label).checked(label == drag_key).on_click(move |_, window, cx| {
                        let patch = json!({ "windows": { "alt_drag": { "key": key } } });
                        let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
                    });
                    menu = menu.item(item);
                }
                menu
            }
        });
    let drag_control = div().flex().items_center().gap(px(12.)).child(drag_menu).child(drag_switch);
    // Alt+click is some apps' own, such as VS Code's extra cursor.
    let drag_about = match drag_key {
        "Alt" => "Hold Alt and drag to move, or right-drag to resize. Win leaves Alt+click to apps".to_string(),
        key => format!("Hold {key} and drag to move, or right-drag to resize"),
    };
    row("Move and resize from anywhere in a window", Some(drag_about.into()), drag_control, p)
}

/// "Open the overview when the pointer rests in a corner", with the corner.
fn corner_overview_row(
    windows_cfg: &WindowSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let corner = windows_cfg.corner_overview.clone();
    let title = "Open the overview when the pointer rests in a corner";
    let label = CORNERS.iter().find(|(k, _)| *k == corner.corner).map_or("Top left", |(_, label)| label);
    let corner_switch = switch("corner-overview", title, corner.on, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| {
                panel.patch(json!({ "windows": { "corner_overview": { "on": on } } }), window, cx)
            })
        }
    });
    let corner_menu = ui::secondary("corner-overview-corner", label, cx)
        .dropdown_caret(true)
        .accessibility_label(format!("Corner: {label}"))
        .dropdown_menu({
            let view = view.downgrade();
            move |mut menu, _, _| {
                for (key, name) in CORNERS {
                    let view = view.clone();
                    let item = PopupMenuItem::new(name).checked(name == label).on_click(move |_, window, cx| {
                        let patch = json!({ "windows": { "corner_overview": { "corner": key } } });
                        let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
                    });
                    menu = menu.item(item);
                }
                menu
            }
        });
    let control = div().flex().items_center().gap(px(12.)).child(corner_menu).child(corner_switch);
    let about =
        "Rest it there a moment, with no button down. Only a corner of the whole desk counts, not one between screens";
    row(title, Some(about.into()), control, p)
}

/// The rows for a window that moves to another screen, and for one dragged there.
fn size_rows(
    panel: &Panel,
    size_mode: WindowSize,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> (Row, Row) {
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
    (
        row("When a window moves to another screen", size_caption(panel, size_mode), size_choice, p),
        row("Also when I drag a window to another screen", None, on_drag, p)
            .map(|el| div().when(scaled, |d| d.opacity(0.4)).child(el).into_any_element()),
    )
}

/// Smart tiling on the chosen screen, for the virtual desktop in front: the switch,
/// and while it is on, the family. The tray keeps it per desktop, so the state says.
fn tiling_card(panel: &Panel, number: usize, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Card {
    let now = panel.state.as_ref().and_then(|s| s.tiling.iter().find(|t| t.screen as usize == number).cloned());
    let (on, family) = now.map_or((false, Family::Smart), |t| (t.on, t.family));
    let view = cx.entity();
    let toggle = switch("zone-tiling", "Arrange new windows automatically", on, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| {
                panel.command("tiling.set", json!({ "number": number, "on": on }), window, cx);
            })
        }
    });
    let mut rows = vec![row(
        "Arrange new windows automatically",
        Some("Smart tiling: each new window takes a share of this screen, on this desktop".into()),
        toggle,
        p,
    )];
    let free = panel.config.zones.tiling_free_on_quit;
    let put_back = switch("zone-tiling-quit", "Put windows back when Seam quits", free, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "tiling_free_on_quit": on } }), window, cx))
        }
    });
    if on {
        let titles: Vec<&'static str> = Family::ALL.iter().map(|f| f.title()).collect();
        let at = Family::ALL.iter().position(|f| *f == family).unwrap_or(0);
        let choice = ui::segmented("zone-tiling-family", "How windows are arranged", &titles, at, window, cx, {
            move |i, window, cx| {
                view.update(cx, |panel, cx| {
                    let family = Family::ALL[i].name();
                    panel.command("tiling.set", json!({ "number": number, "family": family }), window, cx);
                })
            }
        });
        let about = "Smart splits the largest window's space. Scrolling puts windows side by side on a strip you scroll. The others share the screen evenly";
        rows.push(row("How windows are arranged", Some(about.into()), choice, p));
    }
    rows.push(row(
        "Put windows back when Seam quits",
        Some("Quit from the tray frees every tiled screen, on every desktop, first. An update or a restart keeps the arrangement".into()),
        put_back,
        p,
    ));
    card(rows, p)
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
