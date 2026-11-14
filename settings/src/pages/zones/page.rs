//! The Zones page itself.

use super::*;
use crate::pages::app_rules;
use crate::theme::Palette;
use crate::ui::{section, switch};
use gpui_kit::component::slider::Slider;
use seam::config::{WindowSettings, ZoneSettings};

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let zones_cfg = panel.config.zones.clone();
    let view = cx.entity();

    let header = header(&zones_cfg, &view, p, window, cx);

    // The slider follows the setting when something else changed it.
    let gap = zones_cfg.gap.min(32) as f32;
    if panel.gap.read(cx).value().end() != gap {
        panel.gap.update(cx, |slider, cx| slider.set_value(gap, window, cx));
    }

    let snapping = snapping_section(panel, &zones_cfg, &view, p, window, cx);
    let notice = other_wm_notice(panel);
    let windows = windows_section(panel, &view, p, window, cx);

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
    let tabs = screen_tabs(panel, &view, window, cx);

    let number = panel.zone_screen.min(panel.mons.len() - 1) + 1;
    let (picker, own) = layout_picker(panel, &mon, saved.as_ref(), number, p, window, cx);
    let stage = stage(panel, &mon, saved.as_ref(), own, number, window, cx);

    body = body
        .child(div().flex().child(tabs))
        .child(div().flex().flex_col().min_w_0().child(stage))
        .child(picker)
        .child(snapping)
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
    let subtitle = format!("Hold {} while you drag a window to drop it into a zone", key_label(&zones_cfg.show_key));
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
            .child(div().text_size(px(14.)).text_color(p.secondary).child(if zones_cfg.enabled {
                "Zones on"
            } else {
                "Zones off"
            }))
            .child(on),
    )
}

/// The Snapping section: the keys, gap and restore card, then the switches under it.
fn snapping_section(
    panel: &Panel,
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let snapping = snapping_keys_card(panel, zones_cfg, view, p, window, cx);
    let rows = snapping_switches(zones_cfg, view, p, window, cx);
    section("Snapping", div().flex().flex_col().gap(px(12.)).child(snapping).child(card(rows, p)), p)
}

/// Show and span keys, the gap and restore size.
fn snapping_keys_card(
    panel: &Panel,
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
    let show_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.show_key).unwrap_or(0);
    let span_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.span_key).unwrap_or(1);
    let show = ui::segmented("zone-show-key", "Show zones", &["Shift", "Ctrl", "Alt"], show_at, window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            // The span key can't be the same key, so it moves to the next one.
            let mut zones = json!({ "show_key": KEYS[i].0 });
            if KEYS[i].0 == KEYS[span_at].0 {
                zones["span_key"] = json!(KEYS[(i + 1) % KEYS.len()].0);
            }
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": zones }), window, cx));
        }
    });
    let span_labels = SPAN_ORDER.iter().map(|i| SharedString::from(KEYS[*i].1)).collect();
    let span_blocked = SPAN_ORDER.iter().position(|i| *i == show_at).map(|at| {
        let why = [
            "Ctrl is already used to show zones",
            "Shift is already used to show zones",
            "Alt is already used to show zones",
        ];
        (at, why[SPAN_ORDER[at]])
    });
    let span_selected = SPAN_ORDER.iter().position(|i| *i == span_at).unwrap_or(0);
    let span = ui::choice("zone-span-key", "Span zones", span_labels, span_blocked, span_selected, window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            let key = KEYS[SPAN_ORDER[i]].0;
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "span_key": key } }), window, cx));
        }
    });
    let ends = |text: &'static str| div().text_size(px(12.)).text_color(p.muted).child(text);
    let slider = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ends("0"))
        .child(Slider::new(&panel.gap).horizontal().w(px(160.)))
        .child(ends("32"));
    let restore = switch("zone-restore", "Restore size when dragged out", zones_cfg.restore_size, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "restore_size": on } }), window, cx))
        }
    });
    let cell = |content: AnyElement| div().flex_1().min_w_0().child(content);
    div()
        .flex()
        .flex_col()
        .bg(p.card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(12.))
        .child(
            div().flex().child(cell(row("Show zones", Some("Hold while dragging".into()), show, p))).child(
                cell(row("Span zones", Some("Add this key to cover more".into()), span, p))
                    .border_l_1()
                    .border_color(p.hairline),
            ),
        )
        .child(
            div()
                .flex()
                .border_t_1()
                .border_color(p.hairline)
                .child(cell(row("Gap", Some(format!("{} px", zones_cfg.gap.min(32)).into()), slider, p)))
                .child(
                    cell(row(
                        "Restore size when dragged out",
                        Some("The window gets its old size back".into()),
                        restore,
                        p,
                    ))
                    .border_l_1()
                    .border_color(p.hairline),
                ),
        )
}

/// The switch rows of the Snapping card.
fn snapping_switches(
    zones_cfg: &ZoneSettings,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<AnyElement> {
    let refit = switch("zone-refit", "Keep windows in their zones", zones_cfg.refit, window, cx, {
        let view = view.clone();
        move |on, window, cx| view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "refit": on } }), window, cx))
    });
    let on_screen = switch("zone-on-screen", "Edit layouts on the screen", zones_cfg.edit_on_screen, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "edit_on_screen": on } }), window, cx))
        }
    });
    let neighbours =
        switch("zone-neighbours", "Resize neighbours with a window", zones_cfg.resize_neighbours, window, cx, {
            let view = view.clone();
            move |on, window, cx| {
                view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "resize_neighbours": on } }), window, cx))
            }
        });
    let swap = switch("zone-swap", "Swap with the window already in a zone", zones_cfg.swap_on_drop, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "swap_on_drop": on } }), window, cx))
        }
    });
    let swap_row =
        row("Swap with the window already in a zone", Some("Off lets windows overlap in a zone".into()), swap, p);
    let beside = switch("zone-beside", "Snap beside a window", zones_cfg.snap_beside, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "snap_beside": on } }), window, cx))
        }
    });
    let beside_row =
        row("Snap beside a window", Some("Drop on a window's edge to put the two side by side".into()), beside, p);
    let pause = switch("zone-pause", "Pause to snap", zones_cfg.pause_to_snap, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "pause_to_snap": on } }), window, cx))
        }
    });
    let pause_row =
        row("Pause to snap", Some("Hold still while dragging to show zones, no key needed".into()), pause, p);
    let hints = switch("zone-hints", "Show keys while dragging", zones_cfg.drag_hints, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "drag_hints": on } }), window, cx))
        }
    });
    let hints_row = row(
        "Show keys while dragging",
        Some("A strip on the screen names the wheel, right-click and the keys".into()),
        hints,
        p,
    );
    let neighbours_row = row(
        "Resize neighbours with a window",
        Some("Windows in the zones next to it follow its edge".into()),
        neighbours,
        p,
    );
    let handles = switch("zone-handles", "Handles between snapped windows", zones_cfg.divider_handles, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "divider_handles": on } }), window, cx))
        }
    });
    let handles_row = row(
        "Handles between snapped windows",
        Some("Drag one to resize both sides. Right-click for sizes".into()),
        handles,
        p,
    );
    let on_screen_row = row(
        "Edit layouts on the screen",
        Some("Off shows a live view on the screen, and you edit in the preview here".into()),
        on_screen,
        p,
    );
    let refit_row = row(
        "Keep windows in their zones",
        Some("Experimental. Moves snapped windows back after a screen or resolution change".into()),
        refit,
        p,
    );
    vec![swap_row, beside_row, pause_row, hints_row, neighbours_row, handles_row, on_screen_row, refit_row]
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
            div().when(border_off.is_some(), |d| d.opacity(0.4)).child(focus_colour_row).into_any_element(),
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
) -> AnyElement {
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

/// The rows for a window that moves to another screen, and for one dragged there.
fn size_rows(
    panel: &Panel,
    size_mode: WindowSize,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> (AnyElement, AnyElement) {
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
        div()
            .when(scaled, |d| d.opacity(0.4))
            .child(row("Also when I drag a window to another screen", None, on_drag, p))
            .into_any_element(),
    )
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

/// The layout tiles: built in, then yours. Also gives the tree of a layout that is
/// none of them, so it can be saved.
fn layout_picker(
    panel: &Panel,
    mon: &Mon,
    saved: Option<&ZoneLayout>,
    number: usize,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> (Div, Option<Node>) {
    // Built in: the templates. Your layouts: the library layouts for this screen, or
    // all of them, then New layout. A layout that is none of them shows as Changed,
    // selected, before New layout.
    let library = library::entries(&panel.config.custom_layouts);
    let own = saved.filter(|l| {
        l.template == Template::Custom && !l.custom.as_ref().is_some_and(|id| library.iter().any(|e| e.id == *id))
    });
    let own = own.map(|l| l.tree.clone());
    let mut built_in = div().flex().flex_wrap().gap(px(TILE_GAP));
    let templates = Template::ALL.into_iter().filter(|t| *t != Template::Custom).map(Pick::Template);
    let mut at = 0;
    for choice in templates {
        let selected = choice.selected(saved);
        built_in = built_in.child(tile(at, choice, selected, None, window, cx));
        at += 1;
    }
    // The layout the screen uses always shows, so the picker never hides what's in use.
    let in_use = saved.and_then(|l| l.custom.clone());
    let fits: Vec<&str> = library::for_screen(&library, &mon.id).map(|e| e.id.as_str()).collect();
    let all = panel.all_layouts;
    let shown: Vec<(library::Entry, Option<SharedString>)> = library
        .iter()
        .filter(|e| all || fits.contains(&e.id.as_str()) || in_use.as_ref() == Some(&e.id))
        .map(|e| (e.clone(), (!fits.contains(&e.id.as_str())).then(|| for_screens(e, panel))))
        .collect();
    let mut yours = div().flex().flex_wrap().items_center().gap(px(TILE_GAP));
    let picks = shown
        .into_iter()
        .map(|(e, note)| (Pick::Library(e), note))
        .chain(own.clone().map(|tree| (Pick::Changed(tree), None)))
        .chain([(Pick::New, None)]);
    for (choice, note) in picks {
        let selected = choice.selected(saved);
        yours = yours.child(tile(at, choice, selected, note, window, cx));
        at += 1;
    }
    if library.is_empty() && own.is_none() {
        yours = yours.child(ui::description("Make your own with New layout, or Save from windows", p).ml(px(8.)));
    }
    // Only offered when it would show more, or to go back.
    let toggle = (all || library.len() > fits.len()).then(|| {
        let label = if all { format!("Only screen {number}") } else { "Show all screens".to_string() };
        ui::secondary("zone-all-screens", label, cx).ghost().small().on_click(cx.listener(|panel, _, _, cx| {
            panel.all_layouts = !panel.all_layouts;
            cx.notify();
        }))
    });
    let label = |text: &'static str| div().text_size(px(13.)).text_color(p.secondary).child(text);
    let picker = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(label("Built in"))
        .child(built_in)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .mt(px(16.))
                .child(label("Your layouts"))
                .children(toggle),
        )
        .child(yours);
    (picker, own)
}

/// The screen preview with its buttons, or the editor in its place.
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
    let save_as = own.map(|tree| {
        ui::secondary("save-as-layout", "Save as layout", cx)
            .on_click(cx.listener(move |panel, _, window, cx| save_as_layout(panel, tree.clone(), window, cx)))
    });
    let room = f64::from(window.viewport_size().width) - 240. - 64. - 20.;
    let height = preview_height(mon, room);
    let caption =
        ui::description("Stored as parts of the screen, so a new resolution keeps the layout", p).flex_1().min_w_0();
    let from_windows = ui::secondary("save-from-windows", "Save from windows", cx)
        .tooltip("Make a layout from where the windows on this screen are now")
        .on_click(cx.listener(move |panel, _, window, cx| {
            panel.command("zones.from_windows", json!({ "number": number }), window, cx)
        }));
    let edit_layout = ui::secondary("edit-layout", "Edit layout", cx)
        .on_click(cx.listener(|panel, _, window, cx| zone_editor::open(panel, Opened::Screen, window, cx)));
    let buttons =
        div().flex().flex_none().items_center().gap(px(8.)).children(save_as).child(from_windows).child(edit_layout);
    let below = div().flex().items_center().justify_between().gap(px(16.)).child(caption).child(buttons);
    if panel.editor.is_some() {
        zone_editor::view(panel, mon, gap_px, room, height, window, cx)
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(preview(mon, saved, gap_px, room, height, p))
            .child(below)
            .into_any_element()
    }
}
