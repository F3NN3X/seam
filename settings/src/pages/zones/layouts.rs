//! Layout tiles and the saved layouts: pick, rename, duplicate, delete and save as.

use super::*;
use gpui_kit::component::Icon;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};

fn pieces(tree: &Node, width: f32, height: f32, gap: i32) -> Vec<Rect> {
    zones::rects(tree, Rect { x: 0, y: 0, w: width as i32, h: height as i32 }, gap)
}

/// Picking a tile. Unsaved edits ask first.
fn pick(panel: &mut Panel, choice: &Pick, window: &mut Window, cx: &mut Context<Panel>) {
    let then: zone_editor::Then = match choice.clone() {
        Pick::Template(t) => Rc::new(move |panel, window, cx| set_layout(panel, ZoneLayout::of(t), window, cx)),
        Pick::Library(entry) => Rc::new(move |panel, window, cx| set_layout(panel, Some(entry.layout()), window, cx)),
        Pick::Changed(_) => return,
        Pick::New => Rc::new(|panel, window, cx| zone_editor::open(panel, Opened::New, window, cx)),
    };
    zone_editor::guard(panel, window, cx, then);
}

/// A tile in the picker. `note` is a muted line under the name.
pub(super) fn tile(
    i: usize,
    choice: Pick,
    selected: bool,
    note: Option<SharedString>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    let id = ElementId::Name(choice.key().into());
    let focus = window.use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle()).read(cx).clone();
    let built_in = choice.built_in();
    let (width, (art_w, art_h)) = if built_in { BUILT_IN_TILE } else { TILE };
    let mut art = div().relative().flex_none().w(px(art_w)).h(px(art_h));
    match choice.art() {
        Some(tree) => {
            for r in pieces(&tree, art_w, art_h, 2) {
                art = art.child(
                    div()
                        .absolute()
                        .left(px(r.x as f32))
                        .top(px(r.y as f32))
                        .w(px(r.w as f32))
                        .h(px(r.h as f32))
                        .rounded(px(tokens::RADIUS_2))
                        .bg(p.surface_zone),
                );
            }
        }
        None if matches!(choice, Pick::New) => {
            let plus = Icon::new(assets::IconName::Plus).size(px(tokens::SIZE_ICON)).text_color(p.text_secondary);
            art = art.child(div().size_full().flex().items_center().justify_center().child(plus));
        }
        None => art = art.child(div().size_full().rounded(px(tokens::RADIUS_4)).bg(p.surface_page)),
    }
    let title = choice.title();
    let more = match &choice {
        Pick::Library(entry) => Some(library_menu(i, entry, cx)),
        _ => None,
    };
    let is_new = matches!(choice, Pick::New);
    let choice = Rc::new(choice);
    let (click, key) = (choice.clone(), choice);
    let name = text_style(div(), if built_in { tokens::CAPTION } else { tokens::LABEL })
        .truncate()
        .text_color(if selected { p.text_primary } else { p.text_secondary })
        .child(title);
    let face = div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_8))
        // The selected outline is 2 px, so the padding gives a pixel back.
        .p(px(if selected { tokens::SPACE_8 - 1. } else { tokens::SPACE_8 }))
        .w(px(width))
        .rounded(px(tokens::RADIUS_10))
        .when(selected, |d| d.bg(p.accent_tint).border_2().border_color(p.accent_ink))
        .when(!selected, |d| d.border_1().border_color(if is_new { p.hairline_strong } else { p.hairline }))
        .when(is_new, |d| d.border_dashed())
        .when(ui::focused(&focus, window), |d| ui::focus_ring(d, tokens::RADIUS_10, if selected { 2. } else { 1. }, p))
        .child(art)
        .child(div().flex().flex_col().gap(px(tokens::SPACE_2)).min_w_0().child(name).children(
            note.map(|note| text_style(div(), tokens::CAPTION).truncate().text_color(p.text_muted).child(note)),
        ));
    let face = face
        .cursor_pointer()
        .on_click(cx.listener(move |panel, _, window, cx| pick(panel, &click, window, cx)))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                pick(panel, &key, window, cx);
            }
        }));
    // The card ground is the outer element's, so the selected tint sits on it.
    div()
        .id(("zone-tile", i))
        .relative()
        .rounded(px(tokens::RADIUS_10))
        .bg(p.surface_card)
        .when(!selected, |d| d.hover(|d| d.bg(p.surface_raised)))
        .child(face)
        // Over the tile's corner, and a click on it stays there.
        .children(more.map(|more| div().absolute().top(px(4.)).right(px(4.)).occlude().child(more)))
        .into_any_element()
}

/// The "..." of a library layout's tile: Edit, Rename, Duplicate, Used on and Delete.
fn library_menu(i: usize, entry: &library::Entry, cx: &mut Context<Panel>) -> AnyElement {
    let view = cx.entity().downgrade();
    let entry = entry.clone();
    ui::secondary(("zone-tile-more", i), "", cx)
        .ghost()
        .xsmall()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {}", entry.name))
        .dropdown_menu(move |menu, _, _| {
            let (editing, renaming, copying, deleting) = (view.clone(), view.clone(), view.clone(), view.clone());
            let (edit, rename, copy, delete) = (entry.clone(), entry.clone(), entry.clone(), entry.clone());
            let (tying, tie) = (view.clone(), entry.clone());
            menu.item(PopupMenuItem::new("Edit").on_click(move |_, window, cx| {
                let id = edit.id.clone();
                let _ = editing.update(cx, |panel, cx| {
                    let then: zone_editor::Then = Rc::new(move |panel, window, cx| {
                        zone_editor::open(panel, Opened::Library(id.clone()), window, cx)
                    });
                    zone_editor::guard(panel, window, cx, then);
                });
            }))
            .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                let entry = rename.clone();
                let _ = renaming.update(cx, |_, cx| rename_layout(entry, window, cx));
            }))
            .item(PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
                let entry = copy.clone();
                let _ = copying.update(cx, |panel, cx| duplicate(panel, entry, window, cx));
            }))
            .item(PopupMenuItem::new("Used on…").on_click(move |_, window, cx| {
                let entry = tie.clone();
                let _ = tying.update(cx, |panel, cx| used_on(panel, entry, window, cx));
            }))
            .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                let entry = delete.clone();
                let _ = deleting.update(cx, |panel, cx| delete_layout(panel, entry, window, cx));
            }))
        })
        .into_any_element()
}

/// Make one change to the latest library and save the whole list. The tray passes a
/// changed tree on to the screens that follow it, and a deleted layout's screens
/// keep their zones.
fn edit_library(
    panel: &mut Panel,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Vec<Value>) + 'static,
) {
    zone_editor::with_latest(panel, window, cx, move |panel, window, cx| {
        let mut list = panel.config.custom_layouts.clone();
        change(&mut list);
        if list != panel.config.custom_layouts {
            panel.patch(json!({ "custom_layouts": list }), window, cx);
        }
    });
}

fn position(list: &[Value], id: &str) -> Option<usize> {
    list.iter().position(|v| v.get("id").and_then(Value::as_str) == Some(id))
}

fn rename_layout(entry: library::Entry, window: &mut Window, cx: &mut Context<Panel>) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Layout name").default_value(entry.name));
    let view = cx.entity().downgrade();
    let first = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        let (name, view, id) = (input.clone(), view.clone(), entry.id.clone());
        dialog
            .title("Rename layout")
            .child(Input::new(&input))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Cancel")))
                    .child(DialogAction::new().child(ui::primary("rename-layout-ok", "Rename"))),
            )
            .on_ok(move |_, window, cx| {
                let text = name.read(cx).value();
                if text.trim().is_empty() {
                    return false;
                }
                let (id, name) = (id.clone(), text.trim().to_string());
                let _ = view.update(cx, |panel, cx| {
                    edit_library(panel, window, cx, move |list| {
                        if let Some(at) = position(list, &id) {
                            list[at]["name"] = json!(name);
                        }
                    })
                });
                true
            })
    });
}

/// Each connected screen's device path and name, as the tabs show it.
pub(super) fn screen_names(panel: &Panel) -> Vec<(String, SharedString)> {
    panel.mons.iter().enumerate().map(|(i, m)| (m.id.clone(), format!("{}  {}", i + 1, m.name).into())).collect()
}

/// Under the tile of a layout for other screens: which, by number.
pub(super) fn for_screens(entry: &library::Entry, panel: &Panel) -> SharedString {
    let numbers: Vec<String> = panel
        .mons
        .iter()
        .enumerate()
        .filter(|(_, m)| entry.screens.iter().any(|s| s.eq_ignore_ascii_case(&m.id)))
        .map(|(i, _)| (i + 1).to_string())
        .collect();
    match numbers.as_slice() {
        [] => "For a screen not connected".into(),
        [n] => format!("For screen {n}").into(),
        [first @ .., last] => format!("For screens {} and {last}", first.join(", ")).into(),
    }
}

/// Used on: the screens whose tabs offer a library layout, or any screen. Screens
/// it's for that aren't connected now stay in the list, unshown.
fn used_on(panel: &Panel, entry: library::Entry, window: &mut Window, cx: &mut Context<Panel>) {
    let screens = screen_names(panel);
    let chosen = Rc::new(std::cell::RefCell::new(entry.screens.clone()));
    let view = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        let now = chosen.borrow().clone();
        let any = {
            let chosen = chosen.clone();
            Checkbox::new("used-on-any").label("Any screen").checked(now.is_empty()).on_click(move |_, window, _| {
                chosen.borrow_mut().clear();
                window.refresh();
            })
        };
        let mut list = div().flex().flex_col().gap(px(12.)).child(any);
        for (i, (id, name)) in screens.iter().enumerate() {
            let (chosen, id) = (chosen.clone(), id.clone());
            let ticked = now.iter().any(|s| s.eq_ignore_ascii_case(&id));
            let check =
                Checkbox::new(("used-on", i)).label(name.clone()).checked(ticked).on_click(move |on, window, _| {
                    let mut list = chosen.borrow_mut();
                    list.retain(|s| !s.eq_ignore_ascii_case(&id));
                    if *on {
                        list.push(id.clone());
                    }
                    window.refresh();
                });
            list = list.child(check);
        }
        let (view, id, chosen) = (view.clone(), entry.id.clone(), chosen.clone());
        dialog
            .title(format!("Use {} on", entry.name))
            .child(list)
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Cancel")))
                    .child(DialogAction::new().child(ui::primary("used-on-ok", "Save"))),
            )
            .on_ok(move |_, window, cx| {
                let (id, screens) = (id.clone(), chosen.borrow().clone());
                let _ = view.update(cx, |panel, cx| {
                    zone_editor::with_latest(panel, window, cx, move |panel, window, cx| {
                        let mut list = panel.config.custom_layouts.clone();
                        let Some(at) = position(&list, &id) else {
                            let note = gpui_kit::component::notification::Notification::new()
                                .message("That layout was deleted, so nothing changed");
                            window.push_notification(note, cx);
                            return;
                        };
                        if !screens.is_empty() {
                            list[at]["screens"] = json!(screens);
                        } else if let Some(fields) = list[at].as_object_mut() {
                            fields.remove("screens");
                        }
                        if list != panel.config.custom_layouts {
                            panel.patch(json!({ "custom_layouts": list }), window, cx);
                        }
                    })
                });
                true
            })
    });
}

/// A copy of a library layout, as a new one beside it, for the same screens.
fn duplicate(panel: &mut Panel, entry: library::Entry, window: &mut Window, cx: &mut Context<Panel>) {
    edit_library(panel, window, cx, move |list| {
        let name = format!("{} copy", entry.name);
        list.push(library::Entry { id: library::new_id(list), name, tree: entry.tree, screens: entry.screens }.value());
    });
}

/// Whether a screen on any desk uses the layout.
fn in_use(panel: &Panel, id: &str) -> bool {
    let uses = |v: &Value| zones::layout_of(v).is_some_and(|l| l.custom.as_deref() == Some(id));
    panel.config.desks.values().any(|d| d.zones.values().any(uses))
}

/// Take a layout out of the library. Screens using it keep their zones, and Undo puts back
/// that layout alone, where it was, so edits made since stay.
fn remove_layout(panel: &mut Panel, entry: library::Entry, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(at) = position(&panel.config.custom_layouts, &entry.id) else { return };
    let removed = panel.config.custom_layouts[at].clone();
    let id = entry.id.clone();
    edit_library(panel, window, cx, move |list| {
        list.retain(|v| v.get("id").and_then(Value::as_str) != Some(id.as_str()))
    });
    let id = entry.id.clone();
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        let mut list = panel.config.custom_layouts.clone();
        if position(&list, &id).is_none() {
            list.insert(at.min(list.len()), removed.clone());
            panel.patch(json!({ "custom_layouts": list }), window, cx);
        }
    };
    panel.offer_undo(format!("Deleted {}", entry.name), undo, window, cx);
}

/// Delete a layout. It asks first only when a screen uses it.
fn delete_layout(panel: &mut Panel, entry: library::Entry, window: &mut Window, cx: &mut Context<Panel>) {
    if !in_use(panel, &entry.id) {
        remove_layout(panel, entry, window, cx);
        return;
    }
    let view = cx.entity().downgrade();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, entry) = (view.clone(), entry.clone());
        alert
            .title(format!("Delete {}?", entry.name))
            .description("Screens using it keep their zones")
            .ok_text("Delete layout")
            .cancel_text("Cancel")
            .on_ok(move |_, window, cx| {
                let entry = entry.clone();
                let _ = view.update(cx, |panel, cx| remove_layout(panel, entry, window, cx));
                true
            })
    });
}

/// Save as layout: the screen's own layout becomes a new library layout for it,
/// which the screen then follows.
pub(super) fn save_as_layout(panel: &mut Panel, tree: Node, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some(screen) = chosen(panel).map(|(m, _)| m.id.clone()) else { return };
    zone_editor::with_latest(panel, window, cx, move |panel, window, cx| {
        let mut list = panel.config.custom_layouts.clone();
        let (id, name) = (library::new_id(&list), library::next_name(&list));
        let entry = library::Entry { id, name, tree, screens: vec![screen.clone()] };
        list.push(entry.value());
        let Some(layout) = library::screen_patch(&entry.layout()) else { return };
        panel.patch(json!({ "custom_layouts": list, "desks": { sig: { "zones": { screen: layout } } } }), window, cx);
    });
}
