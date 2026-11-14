//! Picking each screen's zones for a workspace.

use super::*;
use seam::library;
use seam::zones::{Template, ZoneLayout};
use serde_json::Value;

// ---- Zones: the layout a switch gives each screen

/// The built-in layouts a screen's Zones menu lists, after No zones.
const BUILT_IN: [Template; 6] =
    [Template::Columns, Template::Rows, Template::Grid, Template::Priority, Template::MainStack, Template::StackMain];
/// The thumbnail of a screen's layout.
const THUMB: (f32, f32) = (40., 24.);

/// A choice in a screen's Zones menu. No entry keeps the screen as it is.
#[derive(Clone)]
struct ZoneChoice {
    label: String,
    entry: Option<Value>,
}

/// Keep as is, No zones, the built-in layouts, then the library's.
fn zone_choices(panel: &Panel) -> Vec<ZoneChoice> {
    let keep = ZoneChoice { label: "Keep as is".into(), entry: None };
    let none = ZoneChoice { label: "No zones".into(), entry: Some(workspace::no_zones()) };
    let built_in = BUILT_IN.iter().filter_map(|t| {
        let entry = serde_json::to_value(ZoneLayout::of(*t)?).ok()?;
        Some(ZoneChoice { label: t.title().into(), entry: Some(entry) })
    });
    let custom = library::entries(&panel.config.custom_layouts).into_iter().filter_map(|e| {
        let entry = serde_json::to_value(e.layout()).ok()?;
        Some(ZoneChoice { label: e.name, entry: Some(entry) })
    });
    [keep, none].into_iter().chain(built_in).chain(custom).collect()
}

/// A screen's entry, in the words of its menu.
fn zones_label(panel: &Panel, saved: Option<&Value>) -> String {
    match saved.and_then(workspace::screen_layout) {
        None => "Keep as is".into(),
        Some(None) => "No zones".into(),
        Some(Some(layout)) => {
            let linked = layout.custom.as_deref().and_then(|id| library::find(&panel.config.custom_layouts, id));
            linked.map_or_else(|| layout.template.title().into(), |entry| entry.name)
        }
    }
}

/// A small drawing of what an entry gives its screen. Keep as is is a dashed outline.
fn zones_thumb(saved: Option<&Value>, p: Palette) -> Div {
    let frame = div().relative().flex_none().w(px(THUMB.0)).h(px(THUMB.1)).rounded(px(4.));
    match saved.and_then(workspace::screen_layout) {
        None => frame.border_1().border_dashed().border_color(p.hairline_strong),
        Some(None) => frame.bg(p.page),
        Some(Some(layout)) => {
            let area = zones::Rect { x: 0, y: 0, w: THUMB.0 as i32, h: THUMB.1 as i32 };
            frame.children(zones::rects(&layout.tree, area, 2).into_iter().map(|r| {
                let at = |v: i32| px(v as f32);
                div().absolute().left(at(r.x)).top(at(r.y)).w(at(r.w)).h(at(r.h)).rounded(px(2.)).bg(p.segment)
            }))
        }
    }
}

/// Give a screen another entry, or none to keep it as it is, with Undo.
fn set_zones(
    panel: &mut Panel,
    id: &str,
    screen: &str,
    entry: Option<Value>,
    note: String,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let screen = screen.to_string();
    update(panel, id, Some(note), window, cx, move |ws| match entry {
        Some(entry) => {
            ws.zones.insert(screen, entry);
        }
        None => {
            ws.zones.remove(&screen);
        }
    });
}

/// The screen's layout now isn't the one the workspace gives it, so a switch changes it.
fn screen_moved(panel: &Panel, ws: &Workspace, screen: &str) -> bool {
    let Some(saved) = ws.zones.get(screen) else { return false };
    let now = panel.config.desks.get(&desk_sig(panel)).and_then(|d| d.zones.get(screen)).and_then(zones::layout_of);
    !workspace::zones_match(saved, now.as_ref())
}

/// A connected screen's layout isn't the workspace's, which Put windows back changes.
pub fn zones_moved(panel: &Panel, ws: &Workspace) -> bool {
    panel.mons.iter().any(|m| screen_moved(panel, ws, &m.id))
}

/// The menu of a connected screen's layout. `number` is its place on the Zones tabs.
fn zones_menu(panel: &Panel, number: usize, screen: &str, ws: &Workspace, cx: &mut Context<Panel>) -> AnyElement {
    let current = zones_label(panel, ws.zones.get(screen));
    let weak = cx.entity().downgrade();
    let (options, shown, id, screen) = (zone_choices(panel), current.clone(), ws.id.clone(), screen.to_string());
    let menu = move |mut menu: gpui_kit::component::menu::PopupMenu, _: &mut Window, _: &mut Context<_>| {
        for (n, choice) in options.iter().enumerate() {
            let (weak, choice, id, screen) = (weak.clone(), choice.clone(), id.clone(), screen.clone());
            let checked = choice.label == shown;
            let item = PopupMenuItem::new(choice.label.clone()).checked(checked).on_click(move |_, window, cx| {
                let (entry, id) = (choice.entry.clone(), id.clone());
                let note = match (&entry, choice.label.as_str()) {
                    (None, _) => format!("Screen {number} keeps its layout"),
                    (_, "No zones") => format!("Screen {number} gets no zones"),
                    (_, label) => format!("Screen {number} gets {label}"),
                };
                let _ = weak.update(cx, |panel, cx| set_zones(panel, &id, &screen, entry, note, window, cx));
            });
            menu = menu.item(item);
            if (n == 1 || n == BUILT_IN.len() + 1) && n + 1 < options.len() {
                menu = menu.separator();
            }
        }
        menu
    };
    ui::secondary(("ws-zones", number), current.clone(), cx)
        .dropdown_caret(true)
        .accessibility_label(format!("Layout for screen {number}: {current}"))
        .dropdown_menu(menu)
        .into_any_element()
}

/// A screen's model from its device path, for an entry whose screen isn't connected.
fn model_of(screen: &str) -> String {
    screen.split('#').nth(1).filter(|m| !m.is_empty()).unwrap_or("Screen").to_string()
}

/// The Zones section: the layout a switch gives each screen, one row per connected
/// screen, then the entries of screens that aren't connected. Closed until opened.
pub fn zones_section(
    panel: &mut Panel,
    ws: &Workspace,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<AnyElement> {
    let p = palette(cx);
    let open = panel.editing.zones_open.as_deref() == Some(ws.id.as_str());
    let about = match ws.zones.len() {
        0 => "Every screen keeps its layout".to_string(),
        1 => "Sets the layout of 1 screen".to_string(),
        n => format!("Sets the layout of {n} screens"),
    };
    let chevron = if open { assets::IconName::ChevronDown } else { assets::IconName::ChevronRight };
    let focus = window.use_keyed_state("ws-zones-head", cx, |_, cx| cx.focus_handle()).read(cx).clone();
    let (click, key) = (ws.id.clone(), ws.id.clone());
    let toggle = |panel: &mut Panel, id: &str, cx: &mut Context<Panel>| {
        let this = Some(id.to_string());
        panel.editing.zones_open = if panel.editing.zones_open == this { None } else { this };
        cx.notify();
    };
    let head = div()
        .id("ws-zones-head")
        .track_focus(&focus)
        .tab_index(0)
        .role(Role::Button)
        .aria_label(format!("Zones, {about}"))
        .cursor_pointer()
        .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .child(row("Zones", Some(about.clone().into()), Icon::new(chevron).size(px(16.)).text_color(p.muted), p))
        .on_click(cx.listener(move |panel, _, _, cx| toggle(panel, &click, cx)))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                toggle(panel, &key, cx);
            }
        }));
    let mut rows = vec![head.into_any_element()];
    if !open {
        return rows;
    }
    let mons: Vec<(String, String)> = panel.mons.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
    for (i, (screen, name)) in mons.iter().enumerate() {
        let saved = ws.zones.get(screen);
        let moved = screen_moved(panel, ws, screen).then(|| {
            let word = div().text_size(px(12.)).text_color(p.secondary).child("Moved");
            div().flex().items_center().gap(px(8.)).child(ui::dot(p.secondary, false)).child(word).into_any_element()
        });
        let menu = zones_menu(panel, i + 1, screen, ws, cx);
        let control = div().flex().items_center().gap(px(8.)).child(zones_thumb(saved, p)).child(menu);
        rows.push(ui::row_with(format!("{}  {name}", i + 1), moved, control));
    }
    let gone = ws.zones.iter().filter(|(screen, _)| !mons.iter().any(|(id, _)| id == *screen));
    for (k, (screen, saved)) in gone.enumerate() {
        let (model, id, screen) = (model_of(screen), ws.id.clone(), screen.clone());
        let note = format!("Removed the layout for {model}");
        let remove = ui::secondary(("ws-zones-remove", k), "Remove", cx)
            .accessibility_label(format!("Remove the layout for {model}"))
            .on_click(cx.listener(move |panel, _, window, cx| {
                set_zones(panel, &id, &screen, None, note.clone(), window, cx);
            }));
        let about = div().text_size(px(13.)).text_color(p.muted).child("Not connected").into_any_element();
        let control =
            div().flex().items_center().gap(px(8.)).child(zones_thumb(Some(saved), p).opacity(0.5)).child(remove);
        rows.push(div().text_color(p.muted).child(ui::row_with(model, Some(about), control)).into_any_element());
    }
    rows
}
