//! Zones: a layout for each screen, and how dragging a window snaps to it.

use crate::pages::app_rules;
use crate::pages::zone_editor::{self, Opened};
use crate::panel::{error_text, refusal, Panel};
use crate::theme::{palette, Palette};
use crate::ui::{self, card, row, section, switch};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerState};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::config::WindowSize;
use seam::desk::Mon;
use seam::library;
use seam::palette as colours;
use seam::state;
use seam::zones::{self, Node, Rect, Template, ZoneLayout};
use serde_json::{json, Value};
use std::rc::Rc;
use std::time::Duration;

/// Config value and label of each key a drag can use.
const KEYS: [(&str, &str); 3] = [("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];
const SPAN_ORDER: [usize; 3] = [1, 0, 2];
/// A tile's width and the size of its drawing: your layouts, and the smaller built-in ones.
const TILE: (f32, (f32, f32)) = (96., (80., 44.));
const BUILT_IN_TILE: (f32, (f32, f32)) = (76., (68., 38.));
const TILE_GAP: f32 = 8.;
/// The preview is as tall as the screen's shape asks, up to this.
const PREVIEW_MAX_HEIGHT: f32 = 320.;
/// The preview card's padding and border around the drawn screen.
const PREVIEW_INSET: f64 = 34.;
/// "When a window moves to another screen", in the order the control shows them.
const SIZE_MODES: [WindowSize; 3] = [WindowSize::Windows, WindowSize::Pixels, WindowSize::Real];
const SIZE_LABELS: [&str; 3] = ["Let Windows scale it", "Keep its pixel size", "Keep its real size"];

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

fn key_label(key: &str) -> &'static str {
    KEYS.iter().find(|(k, _)| *k == key).map_or("Shift", |(_, label)| label)
}

/// A tile in the layout picker.
#[derive(Clone)]
enum Pick {
    Template(Template),
    Library(library::Entry),
    /// The screen's own layout, which is no template's or library layout's.
    Changed(Node),
    New,
}

impl Pick {
    fn key(&self) -> String {
        match self {
            Pick::Template(t) => format!("zone-tile-{}", t.title()),
            Pick::Library(entry) => format!("zone-tile-library-{}", entry.id),
            Pick::Changed(_) => "zone-tile-changed".into(),
            Pick::New => "zone-tile-new".into(),
        }
    }

    fn title(&self) -> SharedString {
        match self {
            Pick::Template(t) => t.title().into(),
            Pick::Library(entry) => entry.name.clone().into(),
            Pick::Changed(_) => "Changed".into(),
            Pick::New => "New layout".into(),
        }
    }

    /// The tree the tile draws.
    fn art(&self) -> Option<Node> {
        match self {
            Pick::Template(t) => zones::template_tree(*t),
            Pick::Library(entry) => Some(entry.tree.clone()),
            Pick::Changed(tree) => Some(tree.clone()),
            Pick::New => None,
        }
    }

    fn built_in(&self) -> bool {
        matches!(self, Pick::Template(_))
    }

    /// Whether this is the layout the screen has now.
    fn selected(&self, saved: Option<&ZoneLayout>) -> bool {
        match self {
            Pick::Template(t) => saved.map_or(Template::None, |l| l.template) == *t,
            Pick::Library(entry) => saved.is_some_and(|l| l.custom.as_ref() == Some(&entry.id)),
            Pick::Changed(_) => true,
            Pick::New => false,
        }
    }
}

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
fn tile(
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
    // The built-in tiles are smaller, so their names are a size down to fit.
    let (width, (art_w, art_h), padding_x, text) =
        if choice.built_in() { (BUILT_IN_TILE.0, BUILT_IN_TILE.1, 4., 12.) } else { (TILE.0, TILE.1, 8., 13.) };
    let mut art = div().relative().w(px(art_w)).h(px(art_h));
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
                        .rounded(px(2.))
                        .bg(p.segment),
                );
            }
        }
        None if matches!(choice, Pick::New) => {
            let plus = div().size_full().flex().items_center().justify_center().rounded(px(4.)).border_1();
            art = art.child(plus.border_dashed().border_color(p.hairline_strong).text_color(p.secondary).child("+"));
        }
        None => art = art.child(div().size_full().rounded(px(4.)).bg(p.page)),
    }
    let title = choice.title();
    let more = match &choice {
        Pick::Library(entry) => Some(library_menu(i, entry, cx)),
        _ => None,
    };
    let choice = Rc::new(choice);
    let (click, key) = (choice.clone(), choice);
    let face = div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(padding_x))
        .py(px(8.))
        .w(px(width))
        .rounded(px(10.))
        .bg(p.card)
        .border_1()
        .border_color(if selected { p.accent } else { p.hairline })
        .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(text))
                        .text_color(if selected { p.text } else { p.secondary })
                        .child(title),
                )
                .children(note.map(|note| div().truncate().text_size(px(12.)).text_color(p.muted).child(note))),
        );
    let face = face
        .cursor_pointer()
        .on_click(cx.listener(move |panel, _, window, cx| pick(panel, &click, window, cx)))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                pick(panel, &key, window, cx);
            }
        }));
    // A second, fainter ring around the chosen tile.
    div()
        .id(("zone-halo", i))
        .relative()
        .p(px(1.))
        .rounded(px(12.))
        .border_1()
        .border_color(if selected { p.accent.opacity(0.4) } else { hsla(0., 0., 0., 0.) })
        .child(face)
        // Over the tile's corner, and a click on it stays there.
        .children(more.map(|more| div().absolute().top(px(4.)).right(px(4.)).occlude().child(more)))
        .into_any_element()
}

/// The "..." of a library layout's tile: Edit, Rename, Used on, Duplicate and Delete.
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
            .item(PopupMenuItem::new("Used on...").on_click(move |_, window, cx| {
                let entry = tie.clone();
                let _ = tying.update(cx, |panel, cx| used_on(panel, entry, window, cx));
            }))
            .item(PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
                let entry = copy.clone();
                let _ = copying.update(cx, |panel, cx| duplicate(panel, entry, window, cx));
            }))
            .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                let entry = delete.clone();
                let _ = deleting.update(cx, |_, cx| delete_layout(entry, window, cx));
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
fn screen_names(panel: &Panel) -> Vec<(String, SharedString)> {
    panel.mons.iter().enumerate().map(|(i, m)| (m.id.clone(), format!("{}  {}", i + 1, m.name).into())).collect()
}

/// Under the tile of a layout for other screens: which, by number.
fn for_screens(entry: &library::Entry, panel: &Panel) -> SharedString {
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

fn delete_layout(entry: library::Entry, window: &mut Window, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, id) = (view.clone(), entry.id.clone());
        alert
            .title(format!("Delete {}?", entry.name))
            .description("Screens using it keep their zones")
            .ok_text("Delete")
            .cancel_text("Cancel")
            .on_ok(move |_, window, cx| {
                let id = id.clone();
                let _ = view.update(cx, |panel, cx| {
                    edit_library(panel, window, cx, move |list| {
                        list.retain(|v| v.get("id").and_then(Value::as_str) != Some(id.as_str()))
                    })
                });
                true
            })
    });
}

/// Save as layout: the screen's own layout becomes a new library layout for it,
/// which the screen then follows.
fn save_as_layout(panel: &mut Panel, tree: Node, window: &mut Window, cx: &mut Context<Panel>) {
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

/// One swatch in a row: "none" first, the named colours, then the custom one.
enum Choice {
    None,
    Named(&'static colours::Swatch),
    Custom,
}

impl Choice {
    fn all() -> impl Iterator<Item = Choice> {
        [Choice::None].into_iter().chain(colours::SWATCHES.iter().map(Choice::Named)).chain([Choice::Custom])
    }

    fn key(&self) -> &'static str {
        match self {
            Choice::None => colours::NONE,
            Choice::Named(s) => s.name,
            Choice::Custom => "custom",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Choice::None => "None",
            Choice::Named(s) => s.title,
            Choice::Custom => "Custom colour",
        }
    }

    fn selected(&self, chosen: &str) -> bool {
        match self {
            Choice::Custom => chosen.starts_with('#'),
            other => other.key() == chosen,
        }
    }

    /// What the swatch is filled with. Windows' accent is read here, and None means
    /// there is no colour to show: a neutral swatch with a glyph instead.
    fn fill(&self, chosen: &str) -> Option<Hsla> {
        let hex = match self {
            Choice::None => return None,
            Choice::Named(s) => match s.rgb {
                Some(hex) => hex,
                None => colours::colorref(colours::accent_colorref()?),
            },
            Choice::Custom => colours::parse_hex(chosen)?,
        };
        Some(rgb(hex).into())
    }

    /// Shown on a neutral swatch that isn't chosen.
    fn glyph(&self) -> &'static str {
        match self {
            Choice::None => "/",
            Choice::Named(_) => "A",
            Choice::Custom => "+",
        }
    }
}

/// A row of round swatches for one `windows` colour setting. A swatch is one
/// tab stop, and Enter or Space picks it. Off, the row shows `why_off` and does nothing.
/// The custom swatch opens the colour picker instead of picking.
fn swatches(
    field: &'static str,
    picker: &Entity<ColorPickerState>,
    current: &str,
    default: &str,
    why_off: Option<&'static str>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
    let p = palette(cx);
    let chosen = colours::normalise(current, default);
    seed_picker(picker, &chosen, window, cx);
    let mut group = div().flex().items_center().gap(px(4.));
    for choice in Choice::all() {
        let selected = choice.selected(&chosen);
        let id = ElementId::Name(format!("swatch-{field}-{}", choice.key()).into());
        let fill = choice.fill(&chosen);
        let dot = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded_full()
            .border_1()
            .border_color(p.hairline_strong)
            .bg(fill.unwrap_or(p.raised))
            .text_size(px(12.))
            .when(fill.is_none() && !selected, |d| d.text_color(p.secondary).child(choice.glyph()))
            .when(selected, |d| {
                // Black or white, whichever reads better on the swatch.
                let ink = fill.map_or(p.text, |f| {
                    let c = Rgba::from(f);
                    let luma = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
                    if luma > 0.5 {
                        hsla(0., 0., 0., 1.)
                    } else {
                        hsla(0., 0., 1., 1.)
                    }
                });
                d.text_color(ink).child("✓")
            });
        let tip = why_off.unwrap_or(choice.title());
        let mut face = div()
            .id(id.clone())
            .p(px(2.))
            .rounded_full()
            .child(dot)
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx));
        if why_off.is_none() {
            let custom = matches!(choice, Choice::Custom);
            let value = choice.key();
            let picker = picker.clone();
            let focus = window.use_keyed_state(id, cx, |_, cx| cx.focus_handle()).read(cx).clone();
            let pick = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                if custom {
                    picker.update(cx, |state, cx| state.set_open(true, cx));
                } else {
                    panel.patch(json!({ "windows": { field: value } }), window, cx);
                }
            };
            let pick = Rc::new(pick);
            let (click, key) = (pick.clone(), pick);
            face = face
                .track_focus(&focus)
                .tab_index(0)
                .cursor_pointer()
                .when(selected || ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
                .when(!custom, |d| d.on_click(cx.listener(move |panel, _, window, cx| click(panel, window, cx))))
                .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
                    if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                        key(panel, window, cx);
                    }
                }));
        } else {
            face = face.when(selected, |d| d.focus_ring_style(window, cx));
        }
        // The picker's own button is laid over the swatch, unseen, so the popover opens
        // from the swatch and the swatch keeps its ring, check and tooltip.
        if why_off.is_none() && matches!(choice, Choice::Custom) {
            let trigger = ColorPicker::new(picker).anchor(Anchor::TopRight).accessibility_label("Custom colour");
            face = face.relative().child(div().absolute().top_0().left_0().size_full().opacity(0.).child(trigger));
        }
        group = group.child(face);
    }
    group
}

/// A picked colour as 0xRRGGBB. Alpha is dropped.
fn hex_of(colour: Hsla) -> u32 {
    let c = Rgba::from(colour);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
}

/// The picker moved to a colour. A drag sends many, so the row shows each at once
/// and only the last one that stays for 250 ms is saved.
pub fn picked(
    field: &'static str,
    colour: Option<Hsla>,
    panel: &mut Panel,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some(colour) = colour else { return };
    let patch = json!({ "windows": { field: format!("#{:06X}", hex_of(colour)) } });
    if let Ok(config) = panel.config.patched(&patch) {
        panel.config = config;
    }
    panel.colour_generation += 1;
    let generation = panel.colour_generation;
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(Duration::from_millis(250)).await;
        let _ = this.update_in(cx, |panel, window, cx| {
            if panel.colour_generation == generation {
                panel.patch(patch, window, cx);
            }
        });
    })
    .detach();
}

/// Keep the picker on the row's colour: the custom one, else the colour the row has
/// now. It isn't touched while open, since the picker is then what moves the colour.
fn seed_picker(picker: &Entity<ColorPickerState>, chosen: &str, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(hex) = colours::parse_hex(chosen).or_else(|| match colours::colour(chosen, chosen) {
        colours::Colour::Rgb(hex) => Some(hex),
        colours::Colour::Accent => colours::accent_colorref().map(colours::colorref),
        colours::Colour::None => None,
    }) else {
        return;
    };
    let (open, held) = {
        let state = picker.read(cx);
        (state.is_open(), state.value().map(hex_of))
    };
    if !open && held != Some(hex) {
        picker.update(cx, |state, cx| state.set_value(rgb(hex), window, cx));
    }
}

/// The screen the page is showing, with the layout saved for it on this desk.
pub fn chosen(panel: &Panel) -> Option<(&Mon, Option<ZoneLayout>)> {
    let mon = panel.mons.get(panel.zone_screen.min(panel.mons.len().checked_sub(1)?))?;
    let sig = panel.state.as_ref().map(|s| s.desk.sig.as_str())?;
    let saved = panel.config.desks.get(sig).and_then(|d| d.zones.get(&mon.id)).and_then(zones::layout_of);
    Some((mon, saved))
}

/// Give the chosen screen a layout, or no zones. It shows at once, and the tray
/// saves it and moves the screen's snapped windows into the new zones, with Undo.
fn set_layout(panel: &mut Panel, layout: Option<ZoneLayout>, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some((mon, _)) = chosen(panel) else { return };
    let number = panel.zone_screen.min(panel.mons.len() - 1) + 1;
    // No entry means no zones, and a patch's null removes the entry.
    let shown = layout.as_ref().and_then(library::screen_patch).unwrap_or(Value::Null);
    if let Ok(config) = panel.config.patched(&json!({ "desks": { sig: { "zones": { mon.id.clone(): shown } } } })) {
        panel.config = config;
    }
    cx.notify();
    let sent = layout.and_then(|l| serde_json::to_value(l).ok()).unwrap_or(Value::Null);
    panel.call("zones.pick", json!({ "number": number, "layout": sent }), window, cx, |panel, result, window, cx| {
        if let Err(e) = result {
            window.push_notification(refusal("Layout left as it was", &error_text(&e)), cx);
        }
        panel.reload_config(window, cx);
    });
}

/// How tall the preview card is for a screen in `room`: the screen's shape, up to the most.
fn preview_height(mon: &Mon, room: f64) -> f32 {
    let (w, h) = (mon.rect.width().max(1) as f64, mon.rect.height() as f64);
    (((room - PREVIEW_INSET) * h / w + PREVIEW_INSET) as f32).min(PREVIEW_MAX_HEIGHT)
}

fn preview(mon: &Mon, layout: Option<&ZoneLayout>, gap: u32, room: f64, height: f32, p: Palette) -> Div {
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

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let zones_cfg = panel.config.zones.clone();
    let subtitle = format!("Hold {} while you drag a window to drop it into a zone", key_label(&zones_cfg.show_key));
    let view = cx.entity();

    let on = switch("zones-on", "Zones", zones_cfg.enabled, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "enabled": on } }), window, cx))
        }
    });
    let header = ui::page_header("Zones", subtitle, p).child(
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
    );

    // The slider follows the setting when something else changed it.
    let gap = zones_cfg.gap.min(32) as f32;
    if panel.gap.read(cx).value().end() != gap {
        panel.gap.update(cx, |slider, cx| slider.set_value(gap, window, cx));
    }

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
    let cell = |content: AnyElement| div().flex_1().min_w_0().child(content);
    let snapping = div()
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
        );

    // Another window manager places the windows, so snapping steps aside (D110).
    let detected = panel.state.as_ref().and_then(|s| s.wm.other_wm.clone());
    let other_wm = panel.config.windows.other_wm;
    let notice = match (&detected, other_wm) {
        (Some(name), _) => Some(format!(
            "{} is running, so snapping, zone keys and window moves are off. The pointer remap, workspaces and focus keys still work.",
            state::other_wm_title(name)
        )),
        (None, true) => Some(
            "Another window manager handles your windows, so snapping, zone keys and window moves are off. The pointer remap, workspaces and focus keys still work."
                .to_string(),
        ),
        (None, false) => None,
    };
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
            row("When a window moves to another screen", size_caption(panel, size_mode), size_choice, p),
            div()
                .when(scaled, |d| d.opacity(0.4))
                .child(row("Also when I drag a window to another screen", None, on_drag, p))
                .into_any_element(),
            row("Another window manager handles my windows", Some(other_about.into()), manual, p),
        ],
        p,
    );

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
    let labels: Vec<SharedString> = screen_names(panel).into_iter().map(|(_, name)| name).collect();
    let tabs =
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
        });

    let number = panel.zone_screen.min(panel.mons.len() - 1) + 1;
    // Built in: the templates. Your layouts: the library layouts for this screen, or
    // all of them, then New layout. A layout that is none of them shows as Changed,
    // selected, before New layout.
    let library = library::entries(&panel.config.custom_layouts);
    let own = saved.as_ref().filter(|l| {
        l.template == Template::Custom && !l.custom.as_ref().is_some_and(|id| library.iter().any(|e| e.id == *id))
    });
    let own = own.map(|l| l.tree.clone());
    let mut built_in = div().flex().flex_wrap().gap(px(TILE_GAP));
    let templates = Template::ALL.into_iter().filter(|t| *t != Template::Custom).map(Pick::Template);
    let mut at = 0;
    for choice in templates {
        let selected = choice.selected(saved.as_ref());
        built_in = built_in.child(tile(at, choice, selected, None, window, cx));
        at += 1;
    }
    // The layout the screen uses always shows, so the picker never hides what's in use.
    let in_use = saved.as_ref().and_then(|l| l.custom.clone());
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
        let selected = choice.selected(saved.as_ref());
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
    let save_as = own.map(|tree| {
        ui::secondary("save-as-layout", "Save as layout", cx)
            .on_click(cx.listener(move |panel, _, window, cx| save_as_layout(panel, tree.clone(), window, cx)))
    });
    let room = f64::from(window.viewport_size().width) - 240. - 64. - 20.;
    let height = preview_height(&mon, room);
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
    let gap_px = zones_cfg.gap.min(32);
    let stage = if panel.editor.is_some() {
        zone_editor::view(panel, &mon, gap_px, room, height, window, cx)
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(preview(&mon, saved.as_ref(), gap_px, room, height, p))
            .child(below)
            .into_any_element()
    };

    body = body
        .child(div().flex().child(tabs))
        .child(div().flex().flex_col().min_w_0().child(stage))
        .child(picker)
        .child(section(
            "Snapping",
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(snapping)
                .child(card(vec![neighbours_row, handles_row, on_screen_row, refit_row], p)),
            p,
        ))
        .child(section("Windows", windows_card, p))
        .child(app_rules::view(panel, cx));
    body
}
