//! The list of workspaces at the left of the page.

use super::app_title;
use super::numbers;
use crate::icons::{self, Icons};
use crate::pages::shortcuts;
use crate::panel::Panel;
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::state::Screen;
use seam::workspace::{Rule, Workspace};

/// How many icons an item shows before "+n", as many as fit in 200 px with the "+n" after them.
const ICONS_SHOWN: usize = 6;

/// A workspace being dragged to another place in the list.
#[derive(Clone)]
pub(super) struct DragWorkspace {
    pub id: String,
    title: SharedString,
}

impl Render for DragWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        text_style(div(), tokens::LABEL)
            .px(px(tokens::SPACE_12))
            .py(px(tokens::SPACE_4))
            .rounded(px(tokens::RADIUS_8))
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.hairline_strong)
            .text_color(p.text_primary)
            .child(self.title.clone())
    }
}

/// The number, from 1, a workspace gets when dragged from place `from` to just before place
/// `before`, or to the end when `before` is `count`.
pub(super) fn number_after(from: usize, before: usize, count: usize) -> usize {
    let at = if before > from { before - 1 } else { before };
    at.min(count.saturating_sub(1)) + 1
}

/// The number (from 1) Alt+Up or Alt+Down gives the workspace at place `i` of `count`, if it can move.
pub(super) fn alt_target(key: &str, i: usize, count: usize) -> Option<usize> {
    match key {
        "up" if i > 0 => Some(i),
        "down" if i + 1 < count => Some(i + 2),
        _ => None,
    }
}

/// The handle a workspace is dragged by, shown on hover and focus.
fn grip(i: usize, ws: &Workspace, shown: bool, group: SharedString, p: Palette) -> Stateful<Div> {
    let drag = DragWorkspace { id: ws.id.clone(), title: ws.name.clone().into() };
    div()
        .id(("workspace-grip", i))
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(tokens::SPACE_4))
        .flex()
        .items_center()
        .w(px(tokens::SIZE_ICON))
        .cursor_grab()
        .opacity(if shown { 1. } else { 0. })
        .group_hover(group, |d| d.opacity(1.))
        .child(Icon::new(assets::IconName::GripVertical).size(px(tokens::SIZE_ICON)).text_color(p.text_muted))
        .tooltip(|window, cx| Tooltip::new("Drag to change the order, or Alt+Up and Alt+Down").build(window, cx))
        .on_drag(drag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
}

/// The workspace's apps as a row of icons in rule order, each app once. An app that is off isn't shown.
fn icon_strip(i: usize, ws: &Workspace, icons: &Icons, p: Palette) -> Div {
    let mut seen: Vec<String> = Vec::new();
    let mut apps: Vec<&Rule> = Vec::new();
    for rule in ws.windows.iter().filter(|r| !r.off) {
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
            None => slot.child(div().size(px(tokens::SIZE_ICON)).rounded(px(tokens::RADIUS_4)).bg(p.surface_zone)),
        };
        slot.tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
    });
    div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_4))
        .children(shown)
        .when(apps.is_empty() && ws.windows.is_empty(), |d| {
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
    /// The screen a one-screen workspace is for; none for all screens.
    pub scope: Option<Scope>,
    /// Some while it's the workspace in use: how many of its windows moved, if that's known.
    pub active: Option<Moved>,
    /// The screens, by Line Up number, an all-screens workspace is in use on when that's some of
    /// them; empty when it holds every screen or is for one.
    pub only: Vec<u32>,
    pub default: bool,
    /// A move just ended: the keyboard focus goes to this item, which now has the workspace the user holds.
    pub focus: bool,
}

/// Whether the windows of the workspace in use are where it puts them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Moved {
    No,
    /// Some did, and the page doesn't know how many.
    Some,
    Count(usize),
}

/// The screen of a one-screen workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// Its Line Up number.
    Screen(u32),
    /// It isn't connected.
    Gone,
}

/// The screen a workspace is for, `None` when it is for all screens.
pub(super) fn scope_of(ws: &Workspace, screens: &[Screen]) -> Option<Scope> {
    let id = ws.scope.as_deref()?;
    Some(screens.iter().find(|s| s.id == id).map_or(Scope::Gone, |s| Scope::Screen(s.number)))
}

/// "Screen 2", or "Screen not connected".
fn scope_words(scope: Scope) -> String {
    match scope {
        Scope::Screen(n) => format!("Screen {n}"),
        Scope::Gone => "Screen not connected".into(),
    }
}

/// "Active", "Active, moved" or "Active, 6 moved"; "Active on screens 1 and 3, 2 moved" for an
/// all-screens workspace that holds only `only`.
fn active_words(moved: Moved, only: &[u32]) -> String {
    let active = match only {
        [] => "Active".to_string(),
        [one] => format!("Active on screen {one}"),
        many => format!("Active on screens {}", numbers(many)),
    };
    match moved {
        Moved::No => active,
        Moved::Some => format!("{active}, moved"),
        Moved::Count(n) => format!("{active}, {n} moved"),
    }
}

/// What the item says besides its name: "Screen 2, Active, 3 moved". `None` when it says nothing.
fn meta_line(scope: Option<Scope>, active: Option<(Moved, &[u32])>) -> Option<String> {
    let parts: Vec<String> =
        scope.map(scope_words).into_iter().chain(active.map(|(moved, only)| active_words(moved, only))).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// The status on the item of the workspace in use: a dot and a word, as the editor's rows have.
fn active_status(moved: Moved, only: &[u32], p: Palette) -> AnyElement {
    let dot = if moved == Moved::No { ui::dot(p.positive, true) } else { ui::dot(p.text_muted, false) };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .child(dot)
        .child(text_style(div(), tokens::CAPTION).text_color(p.text_secondary).child(active_words(moved, only)))
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
    let (id, moving) = (ws.id.clone(), ws.id.clone());
    let view = cx.entity();
    let focus = ui::item_focus(("workspace", i), window, cx);
    if marks.focus {
        let to_focus = focus.clone();
        window.defer(cx, move |window, cx| to_focus.focus(window, cx));
    }
    let group = SharedString::from(format!("workspace-item-{i}"));
    let grip = grip(i, ws, ui::focused(&focus, window), group.clone(), p);
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
    let scope = marks
        .scope
        .map(|scope| text_style(div(), tokens::CAPTION).text_color(p.text_secondary).child(scope_words(scope)));
    let meta = (marks.default || marks.active.is_some() || marks.scope.is_some()).then(|| {
        div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_8))
            .children(scope)
            .children(default)
            .children(marks.active.map(|moved| active_status(moved, &marks.only, p)))
    });
    let mut label = vec![ws.name.clone()];
    if marks.default {
        label.push("default".into());
    }
    if let Some(line) = meta_line(marks.scope, marks.active.map(|moved| (moved, marks.only.as_slice()))) {
        label.push(line.to_lowercase());
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
    // Alt+arrow moves the workspace. It goes first, so the arrow keys' focus move doesn't see it.
    let item = item.on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
        if !e.keystroke.modifiers.alt {
            return;
        }
        if let Some(to) = alt_target(e.keystroke.key.as_str(), i, count) {
            cx.stop_propagation();
            // A move still on its way leaves the list as it was, so a press now would act on a stale one.
            if !panel.moving {
                super::move_workspace(panel, &moving, to, window, cx);
            }
        }
    }));
    ui::arrow_keys(item, before, Some(after))
        .relative()
        .group(group)
        .drag_over::<DragWorkspace>(move |d, _, _, _| d.bg(p.surface_raised))
        .on_drop(cx.listener(move |panel, drag: &DragWorkspace, window, cx| {
            super::dropped(panel, &drag.id, i, window, cx);
        }))
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_4))
        .p(px(tokens::SPACE_12))
        .rounded(px(tokens::RADIUS_10))
        .border_1()
        .cursor_pointer()
        .when(picked, |d| d.bg(p.surface_raised).border_color(p.hairline_strong))
        // A contrast theme marks the chosen one with a thick edge, whatever its fill.
        .when(picked && p.contrast, |d| d.border_2().border_color(p.text_primary))
        .when(!picked, |d| {
            d.bg(p.surface_card)
                .border_color(p.hairline)
                .hover(|d| ui::outlined(d, p.surface_raised, p))
                .active(|d| ui::outlined(d, p.surface_sidebar, p))
        })
        .child(grip)
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
    let count = last.map_or(0, |i| i + 1);
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
        .drag_over::<DragWorkspace>(move |d, _, _, _| d.bg(p.surface_raised))
        .on_drop(cx.listener(move |panel, drag: &DragWorkspace, window, cx| {
            super::dropped(panel, &drag.id, count, window, cx);
        }))
        .flex()
        .items_center()
        .justify_center()
        .min_h(px(tokens::SIZE_ROW))
        .border_1()
        .border_dashed()
        .border_color(p.hairline_strong)
        .rounded(px(tokens::RADIUS_10))
        .hover(|d| ui::lit(d, p.surface_card, p))
        .active(|d| ui::lit(d, p.surface_sidebar, p))
        .child(text_style(div(), tokens::BODY).text_color(p.text_secondary).child("New empty workspace"))
}

#[cfg(test)]
mod tests {
    use super::{Moved, Scope, alt_target, meta_line, number_after};

    #[test]
    fn meta_line_names_the_screen() {
        // A one-screen workspace starts with its screen, then today's words.
        assert_eq!(
            meta_line(Some(Scope::Screen(2)), Some((Moved::Count(3), &[]))).as_deref(),
            Some("Screen 2, Active, 3 moved")
        );
        assert_eq!(meta_line(Some(Scope::Screen(2)), Some((Moved::No, &[]))).as_deref(), Some("Screen 2, Active"));
        assert_eq!(meta_line(Some(Scope::Screen(1)), None).as_deref(), Some("Screen 1"));
        assert_eq!(meta_line(Some(Scope::Gone), None).as_deref(), Some("Screen not connected"));
        // An all-screens workspace holding only some screens says which.
        assert_eq!(meta_line(None, Some((Moved::No, &[1, 3]))).as_deref(), Some("Active on screens 1 and 3"));
        assert_eq!(
            meta_line(None, Some((Moved::Count(2), &[1, 3]))).as_deref(),
            Some("Active on screens 1 and 3, 2 moved")
        );
        assert_eq!(meta_line(None, Some((Moved::Some, &[2]))).as_deref(), Some("Active on screen 2, moved"));
        // Holding every screen, or none in use, is as before.
        assert_eq!(meta_line(None, Some((Moved::Some, &[]))).as_deref(), Some("Active, moved"));
        assert_eq!(meta_line(None, None), None);
    }

    #[test]
    fn alt_arrows_move_a_workspace_only_inside_the_list() {
        assert_eq!(alt_target("up", 0, 3), None);
        assert_eq!(alt_target("up", 2, 3), Some(2));
        assert_eq!(alt_target("down", 0, 3), Some(2));
        assert_eq!(alt_target("down", 2, 3), None);
        assert_eq!(alt_target("left", 1, 3), None);
    }

    #[test]
    fn dropping_before_a_workspace_gives_the_number_it_takes() {
        // Four workspaces, a b c d. Dragging a before d puts it third.
        assert_eq!(number_after(0, 3, 4), 3);
        // Dragging d before b puts it second.
        assert_eq!(number_after(3, 1, 4), 2);
        // On the "New empty workspace" box, it goes last.
        assert_eq!(number_after(0, 4, 4), 4);
        assert_eq!(number_after(2, 4, 4), 4);
        // Dropped before itself or just after itself, it stays where it is.
        assert_eq!(number_after(1, 1, 4), 2);
        assert_eq!(number_after(1, 2, 4), 2);
    }
}
