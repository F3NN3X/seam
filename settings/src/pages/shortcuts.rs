//! Global shortcuts: the ones in use, the ones with no keys yet, and recording a new one.

use crate::panel::Panel;
use crate::theme::palette;
use crate::theme::tokens::{self, text_style};
use crate::ui::{self, Row, card, keycaps, row_with, section};
use catalog::{Group, Item, Section};
use gpui_kit::component::Icon;
use gpui_kit::component::input::Input;
use gpui_kit::*;

mod catalog;
mod clash;
mod record;

pub use record::{Clash, stop_recording};

/// A line under a row's title, for the shortcuts whose name doesn't say enough.
fn caption(action: &str) -> Option<&'static str> {
    match action {
        "quick_layout" => Some("Pick this screen's layout from the keyboard"),
        "divider_menu" => Some("Share or take out the focused snapped window"),
        "pause_seam" => Some("Pause or resume everything Seam does"),
        "fit_group" => Some("Stretch the focused window's group to fill its screen"),
        "stack_next" => Some("Put the next window of the focused window's stack on top"),
        "float_tile" => Some("Take the focused window out of smart tiling, or put it back in"),
        "free_tiling" => Some("Stop smart tiling on the screen under the pointer, and put its windows back"),
        "stack_prev" => Some("Put the previous window of the focused window's stack on top"),
        "overview" => Some("Every window on every screen and desktop, to search and go to"),
        "rotate" => {
            Some("Move every snapped window on the screen under the pointer to the next zone. Shift added goes back")
        }
        "reset_sizes" => {
            Some("Put the screen under the pointer back to the layout it came from, or even out its tiles")
        }
        "scroll_left" => Some("On a Scrolling screen, focus the column on the left and scroll to it"),
        "scroll_right" => Some("On a Scrolling screen, focus the column on the right and scroll to it"),
        "column_left" => Some("Swap the focused window's column with the one on its left"),
        "column_right" => Some("Swap the focused window's column with the one on its right"),
        "column_width" => Some("Give the focused window's column the next width: a third, a half, two thirds or full"),
        a if a.starts_with("add_to_") => {
            Some("Add the focused window to this workspace, counted in the order of the Workspaces page")
        }
        _ => None,
    }
}

/// One row. `slot` numbers the row, so its buttons get ids of their own. `about` is the line under
/// the title for a row whose name doesn't say enough and has no caption of its own.
pub fn shortcut_row(
    panel: &Panel,
    title: String,
    action: String,
    combo: Option<&String>,
    (slot, about): (usize, Option<String>),
    cx: &mut Context<Panel>,
) -> Row {
    let p = palette(cx);
    let note = |text: SharedString, color: Hsla| text_style(div(), tokens::LABEL).text_color(color).child(text);
    if panel.recording.as_deref() == Some(action.as_str()) {
        let clash = panel.clash.as_ref();
        let keys = match clash {
            Some(clash) => keycaps(&clash.combo, true, p).into_any_element(),
            None => {
                text_style(div(), tokens::LABEL).text_color(p.text_secondary).child("Press keys").into_any_element()
            }
        };
        let said = clash.map(|clash| note(clash.message.clone().into(), p.negative).into_any_element());
        let cancel = ui::secondary(("cancel", slot), "Cancel", cx).on_click(cx.listener(|panel, _, _, cx| {
            stop_recording(panel);
            cx.notify();
        }));
        let control = div().flex().items_center().gap(px(tokens::SPACE_16)).child(keys).child(cancel);
        return row_with(title, said, control).map(|row| div().bg(p.surface_raised).child(row).into_any_element());
    }
    let conflict = panel.state.as_ref().is_some_and(|s| s.hotkey_errors.iter().any(|e| e.action == action));
    let about = if conflict {
        Some(note("Another app already uses this shortcut. Choose a different one".into(), p.negative))
    } else if panel.hotkey_warning.as_deref() == Some(action.as_str()) {
        Some(note("Ctrl+Alt types characters on many keyboards. Add Shift".into(), p.negative))
    } else {
        let text = about.map(SharedString::from).or_else(|| caption(&action).map(SharedString::from));
        text.map(|text| note(text, p.text_secondary))
    };
    let keys = match combo {
        Some(combo) => keycaps(combo, conflict, p).into_any_element(),
        None => text_style(div(), tokens::LABEL).text_color(p.text_muted).child("Not set").into_any_element(),
    };
    let label = if combo.is_some() { "Change" } else { "Record" };
    let button = ui::secondary(("change", slot), label, cx)
        .on_click(cx.listener(move |panel, _, _, cx| record::start_recording(panel, action.clone(), cx)));
    row_with(
        title,
        about.map(IntoElement::into_any_element),
        div().flex().items_center().gap(px(16.)).child(keys).child(button),
    )
}

/// A row for each of `items`.
fn rows(panel: &Panel, items: &[&Item], cx: &mut Context<Panel>) -> Vec<Row> {
    let row = |i: &&Item| shortcut_row(panel, i.title.clone(), i.action.clone(), i.combo.as_ref(), (i.slot, None), cx);
    items.iter().map(row).collect()
}

/// "In use" and "All" with how many each lists, and the filter.
fn toolbar(panel: &Panel, (set, total): (usize, usize), window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let view = cx.entity();
    let labels = vec![format!("In use {set}").into(), format!("All {total}").into()];
    let tabs = ui::choice(
        "shortcuts-shown",
        "Shortcuts shown",
        labels,
        None,
        usize::from(panel.all_shortcuts),
        window,
        cx,
        move |i, _, cx| {
            view.update(cx, |panel, cx| {
                panel.all_shortcuts = i == 1;
                cx.notify();
            })
        },
    );
    let search = Input::new(&panel.shortcut_filter)
        .appearance(false)
        .aria_label("Find a shortcut or a key")
        .prefix(Icon::new(assets::IconName::Search).size(px(tokens::SIZE_ICON)).text_color(p.text_muted));
    let filter = div()
        .flex()
        .items_center()
        .w(px(280.))
        .h(px(32.))
        .px(px(tokens::SPACE_8))
        .rounded(px(tokens::RADIUS_8))
        .bg(p.surface_input)
        .border_1()
        .border_color(p.hairline)
        .child(search);
    div().flex().items_center().gap(px(tokens::SPACE_16)).child(tabs).child(filter)
}

/// A group of "Not set yet": its name, what's in it, and how many of them have no keys.
fn group_row(group: Group, count: usize, open: bool, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let p = palette(cx);
    let view = cx.entity();
    let count = text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(count.to_string());
    ui::disclosure_row_with(
        ElementId::Name(format!("shortcut-group-{}", group.title()).into()),
        (group.title(), group.names()),
        count,
        open,
        window,
        cx,
        move |_, cx| {
            view.update(cx, |panel, cx| {
                panel.toggle_disclosure(group.title());
                cx.notify();
            })
        },
    )
}

/// "Not set yet": the way to give them all keys at once, then a row for each group that opens its shortcuts.
fn not_set_yet(panel: &mut Panel, items: &[Item], window: &mut Window, cx: &mut Context<Panel>) -> Option<AnyElement> {
    let p = palette(cx);
    let unset: Vec<&Item> = items.iter().filter(|i| i.combo.is_none()).collect();
    if unset.is_empty() {
        return None;
    }
    let suggest = ui::secondary("use-suggested", "Use suggested keys", cx)
        .on_click(cx.listener(|panel, _, window, cx| record::use_suggested(panel, window, cx)));
    let about = format!(
        "Seam picks keys for all {} that don't clash with Windows or your other shortcuts. You can change any of them after",
        unset.len()
    );
    let mut list = vec![ui::row("Use suggested keys", Some(about.into()), suggest, p)];
    for group in Group::ALL {
        let inside: Vec<&Item> = unset.iter().copied().filter(|i| i.group == group).collect();
        if inside.is_empty() {
            continue;
        }
        let titles: Vec<&str> = inside.iter().map(|i| i.title.as_str()).collect();
        let open = panel.disclosure_open(group.title(), &titles);
        list.push(group_row(group, inside.len(), open, window, cx));
        if open {
            list.extend(rows(panel, &inside, cx));
        }
    }
    Some(section("Not set yet", card(list, p), p))
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let items = catalog::items(panel);
    let query = panel.shortcut_filter.read(cx).value();
    let finding = !query.trim().is_empty();
    // A filter looks in the shortcuts with no keys too.
    let all = panel.all_shortcuts || finding;
    let set = items.iter().filter(|i| i.combo.is_some()).count();
    let header = ui::page_title(
        "Shortcuts",
        "These work anywhere in Windows, even with this window closed",
        Some("Shortcuts"),
        p,
        cx,
    );
    let toolbar = toolbar(panel, (set, items.len()), window, cx);
    let shown: Vec<&Item> = items.iter().filter(|i| i.matches(&query) && (all || i.combo.is_some())).collect();
    let mut page = div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header).child(toolbar);
    for which in Section::ALL {
        let of: Vec<&Item> = shown.iter().copied().filter(|i| i.section == which).collect();
        if !of.is_empty() {
            page = page.child(section(which.title(), card(rows(panel, &of, cx), p), p));
        }
    }
    if finding && shown.is_empty() {
        page = page.child(ui::description("No shortcut matches", p));
    }
    if !all {
        page = page.children(not_set_yet(panel, &items, window, cx));
    }
    page
}
