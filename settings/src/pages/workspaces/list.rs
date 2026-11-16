//! The list of workspaces at the left of the page.

use super::app_title;
use crate::icons::{self, Icons};
use crate::pages::shortcuts;
use crate::panel::Panel;
use crate::theme::Palette;
use crate::theme::tokens::{self, text_style};
use crate::ui;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::workspace::{Rule, Workspace};

/// How many icons an item shows before "+n", as many as fit in 200 px with the "+n" after them.
const ICONS_SHOWN: usize = 6;

/// The workspace's apps as a row of icons in rule order, each app once.
fn icon_strip(i: usize, ws: &Workspace, icons: &Icons, p: Palette) -> Div {
    let mut seen: Vec<String> = Vec::new();
    let mut apps: Vec<&Rule> = Vec::new();
    for rule in &ws.windows {
        let key = icons::identity(rule);
        if !seen.contains(&key) {
            seen.push(key);
            apps.push(rule);
        }
    }
    let more = apps.len().saturating_sub(ICONS_SHOWN);
    let shown = apps.iter().take(ICONS_SHOWN).enumerate().map(|(n, rule)| {
        let slot = div().id(("app-icon", i * 100 + n)).flex_none().size(px(tokens::SIZE_ICON));
        let name = SharedString::from(app_title(rule));
        let slot = match icons.get(rule) {
            Some(image) => slot.child(img(image).size(px(tokens::SIZE_ICON))),
            None => slot.child(div().size(px(tokens::SIZE_ICON)).rounded(px(tokens::RADIUS_4)).bg(p.text_muted)),
        };
        slot.tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
    });
    div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_4))
        .children(shown)
        .when(apps.is_empty(), |d| {
            d.child(text_style(div(), tokens::CAPTION).text_color(p.text_muted).child("No windows yet"))
        })
        .when(more > 0, |d| {
            d.child(
                text_style(div().flex_none().whitespace_nowrap(), tokens::CAPTION)
                    .text_color(p.text_muted)
                    .child(format!("+{more}")),
            )
        })
}

/// What a workspace item says about itself besides its name.
pub(super) struct Marks {
    pub picked: bool,
    /// Some while it's the workspace in use: how many of its windows moved, if that's known.
    pub active: Option<Moved>,
    pub default: bool,
}

/// Whether the windows of the workspace in use are where it puts them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Moved {
    No,
    /// Some did, and the page doesn't know how many.
    Some,
    Count(usize),
}

/// "Active", "Active, moved" or "Active, 6 moved".
fn active_words(moved: Moved) -> String {
    match moved {
        Moved::No => "Active".into(),
        Moved::Some => "Active, moved".into(),
        Moved::Count(n) => format!("Active, {n} moved"),
    }
}

/// The status on the item of the workspace in use: a dot and a word, as the editor's rows have.
fn active_status(moved: Moved, p: Palette) -> AnyElement {
    let dot = if moved == Moved::No { ui::dot(p.positive, true) } else { ui::dot(p.text_muted, false) };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .child(dot)
        .child(text_style(div(), tokens::CAPTION).text_color(p.text_secondary).child(active_words(moved)))
        .into_any_element()
}

pub(super) fn item(
    (i, count): (usize, usize),
    ws: &Workspace,
    icons: &Icons,
    marks: Marks,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let id = ws.id.clone();
    let view = cx.entity();
    // Up and Down walk the list, which ends with "New empty workspace".
    let before = (i > 0).then(|| ui::item_focus(("workspace", i - 1), window, cx));
    let after = if i + 1 < count {
        ui::item_focus(("workspace", i + 1), window, cx)
    } else {
        ui::item_focus("new-workspace", window, cx)
    };
    let picked = marks.picked;
    let default =
        marks.default.then(|| text_style(div(), tokens::CAPTION).text_color(p.text_secondary).child("Default"));
    // The marks get a line of their own, so the name keeps the item's full width.
    let meta = (marks.default || marks.active.is_some()).then(|| {
        div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_8))
            .children(default)
            .children(marks.active.map(|moved| active_status(moved, p)))
    });
    let mut label = vec![ws.name.clone()];
    if marks.default {
        label.push("default".into());
    }
    if let Some(moved) = marks.active {
        label.push(active_words(moved).to_lowercase());
    }
    let name =
        text_style(div().min_w_0(), tokens::BODY).font_weight(FontWeight::MEDIUM).truncate().child(ws.name.clone());
    let pick = move |_: &mut Window, cx: &mut App| {
        view.update(cx, |panel, cx| {
            shortcuts::stop_recording(panel);
            panel.workspace = Some(id.clone());
            panel.naming = false;
            panel.editing.expanded = None;
            cx.notify();
        })
    };
    let item =
        ui::selectable_ringed(("workspace", i), label.join(", "), picked, (tokens::RADIUS_10, 1.), window, cx, pick)
            .role(Role::ListItem);
    ui::arrow_keys(item, before, Some(after))
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_4))
        .p(px(tokens::SPACE_12))
        .rounded(px(tokens::RADIUS_10))
        .border_1()
        .cursor_pointer()
        .when(picked, |d| d.bg(p.surface_raised).border_color(p.hairline_strong))
        .when(!picked, |d| {
            d.bg(p.surface_card)
                .border_color(p.hairline)
                .hover(|d| d.bg(p.surface_raised))
                .active(|d| d.bg(p.surface_sidebar))
        })
        .child(name)
        .children(meta)
        .child(icon_strip(i, ws, icons, p))
        .into_any_element()
}

/// "New empty workspace": a dashed box, 48 px high.
pub(super) fn new_empty(
    last: Option<usize>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Stateful<Div> {
    let view = cx.entity();
    let before = last.map(|i| ui::item_focus(("workspace", i), window, cx));
    let button = ui::selectable_ringed(
        "new-workspace",
        "New empty workspace",
        false,
        (tokens::RADIUS_10, 1.),
        window,
        cx,
        move |window, cx| {
            view.update(cx, |panel, cx| super::create_empty(panel, window, cx));
        },
    );
    ui::arrow_keys(button, before, None)
        .flex()
        .items_center()
        .justify_center()
        .min_h(px(tokens::SIZE_ROW))
        .border_1()
        .border_dashed()
        .border_color(p.hairline_strong)
        .rounded(px(tokens::RADIUS_10))
        .hover(|d| d.bg(p.surface_card))
        .active(|d| d.bg(p.surface_sidebar))
        .child(text_style(div(), tokens::BODY).text_color(p.text_secondary).child("New empty workspace"))
}
