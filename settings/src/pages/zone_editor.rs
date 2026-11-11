//! The zone editor: split, merge and drag dividers on a copy of the layout's
//! tree. Every edit stays in the panel, and only Done writes, through
//! `config.patch`: into the screen's own layout, a library layout, or a new
//! library layout. Esc throws the edits away and Ctrl+Z undoes the last one.

use crate::pages::zones::chosen;
use crate::panel::Panel;
use crate::theme::palette;
use crate::ui;
use gpui_kit::component::{Disableable, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::desk::Mon;
use seam::library;
use seam::zones::{self, Dir, LeafTag, Node, Rect, Template, ZoneLayout};
use serde_json::{json, Value};
use std::rc::Rc;

actions!(seam_zone_editor, [DiscardEdit, UndoEdit]);

/// Esc and Ctrl+Z only mean something to an element inside this context.
pub(super) const CONTEXT: &str = "SeamZoneEditor";
/// Room for a pointer to grab a divider, in preview pixels.
const GRAB: f32 = 10.;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", DiscardEdit, Some(CONTEXT)),
        KeyBinding::new("ctrl-z", UndoEdit, Some(CONTEXT)),
    ]);
}

/// What the page does once it's clear that unsaved edits can go.
pub type Then = Rc<dyn Fn(&mut Panel, &mut Window, &mut Context<Panel>)>;

/// The tree as it was before one edit, with the name Undo gives it.
struct Step {
    name: &'static str,
    tree: Node,
    selected: usize,
}

pub(super) struct Dragging {
    pub(super) path: Vec<bool>,
    before: Node,
}

/// What a divider drag carries. The pointer's position does the rest.
#[derive(Clone)]
pub(super) struct DragDivider;

impl Render for DragDivider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// What the editor was opened on.
#[derive(Clone, Debug, PartialEq)]
pub enum Opened {
    /// The screen's layout, from Edit layout.
    Screen,
    /// A library layout, from its tile's menu.
    Library(String),
    /// A new library layout, from the New layout tile.
    New,
}

/// What Done saves the tree as.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// The screen's own layout, which then follows no library layout.
    Screen,
    /// A library layout, and with it every screen that follows it.
    Library(String),
    /// A new library layout, which the screen is given. A template is never changed.
    New,
}

pub struct Editor {
    /// The desk and screen being edited.
    pub sig: String,
    pub screen: String,
    pub opened: Opened,
    target: Target,
    pub(super) tree: Node,
    start: Node,
    /// What Reset goes back to: the template, or the tree the session began with.
    base: Node,
    pub(super) selected: usize,
    history: Vec<Step>,
    pub(super) dragging: Option<Dragging>,
    focus: FocusHandle,
}

impl Editor {
    pub fn dirty(&self) -> bool {
        self.tree != self.start
    }

    fn edit(&mut self, name: &'static str, tree: Node, selected: usize) {
        self.history.push(Step { name, tree: std::mem::replace(&mut self.tree, tree), selected: self.selected });
        self.selected = selected;
    }

    fn undo(&mut self) {
        if let Some(step) = self.history.pop() {
            self.tree = step.tree;
            self.selected = step.selected;
        }
    }
}

/// What Edit layout edits on a screen with `saved`: a template's tree as a new
/// library layout, the library layout the screen follows, or the screen's own
/// layout. A screen with no layout starts a new one from a single zone. Also the
/// tree to start from and what Reset goes back to.
fn screen_edit(saved: Option<&ZoneLayout>, library: &[Value]) -> (Target, Node, Node) {
    let leaf = Node::Leaf(LeafTag::Leaf);
    let Some(saved) = saved else { return (Target::New, leaf.clone(), leaf) };
    if let Some(template) = zones::template_tree(saved.template) {
        return (Target::New, saved.tree.clone(), template);
    }
    match saved.custom.as_deref().and_then(|id| library::find(library, id)) {
        Some(entry) => (Target::Library(entry.id), entry.tree.clone(), entry.tree),
        None => (Target::Screen, saved.tree.clone(), saved.tree.clone()),
    }
}

/// Start editing on the chosen screen: its layout, a library layout or a new one.
pub fn open(panel: &mut Panel, opened: Opened, window: &mut Window, cx: &mut Context<Panel>) {
    if panel.editor.is_some() {
        return;
    }
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some((screen, saved)) = chosen(panel).map(|(m, l)| (m.id.clone(), l)) else { return };
    let leaf = Node::Leaf(LeafTag::Leaf);
    let (target, tree, base) = match &opened {
        Opened::Screen => screen_edit(saved.as_ref(), &panel.config.custom_layouts),
        Opened::Library(id) => {
            let Some(entry) = library::find(&panel.config.custom_layouts, id) else { return };
            (Target::Library(entry.id), entry.tree.clone(), entry.tree)
        }
        Opened::New => (Target::New, leaf.clone(), leaf),
    };
    let focus = cx.focus_handle();
    focus.focus(window, cx);
    let editor = Editor {
        sig,
        screen,
        opened,
        target,
        start: tree.clone(),
        tree,
        base,
        selected: 0,
        history: Vec::new(),
        dragging: None,
        focus,
    };
    panel.editor = Some(editor);
    cx.notify();
}

/// The tray changed the layout under Edit layout with no edits, after a window's
/// edge was dragged or a grow key pressed: start again from the saved one, so edits
/// made next don't build on an older layout.
pub fn reload(panel: &mut Panel) {
    let Some(editor) = panel.editor.as_ref().filter(|e| !e.dirty() && e.dragging.is_none()) else { return };
    if editor.opened != Opened::Screen {
        return;
    }
    let desk = panel.config.desks.get(&editor.sig);
    let saved = desk.and_then(|d| d.zones.get(&editor.screen)).and_then(zones::layout_of);
    let (target, tree, base) = screen_edit(saved.as_ref(), &panel.config.custom_layouts);
    if tree == editor.start && target == editor.target {
        return;
    }
    if let Some(editor) = panel.editor.as_mut() {
        editor.selected = editor.selected.min(zones::leaf_count(&tree) - 1);
        editor.history.clear();
        (editor.target, editor.start, editor.tree, editor.base) = (target, tree.clone(), tree, base);
    }
}

/// Run `then` once unsaved edits are out of the way. Without edits that's at
/// once. With them, one dialog asks.
pub fn guard(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>, then: Then) {
    if !panel.editor.as_ref().is_some_and(Editor::dirty) {
        panel.editor = None;
        then(panel, window, cx);
        return;
    }
    let view = cx.entity().downgrade();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, then) = (view.clone(), then.clone());
        alert
            .title("Discard layout changes?")
            .description("The edits to this layout haven't been saved")
            .ok_text("Discard")
            .cancel_text("Keep editing")
            .on_ok(move |_, window, cx| {
                let then = then.clone();
                let _ = view.update(cx, |panel, cx| {
                    panel.editor = None;
                    then(panel, window, cx);
                });
                true
            })
    });
}

pub(super) fn discard(panel: &mut Panel, cx: &mut Context<Panel>) {
    panel.editor = None;
    cx.notify();
}

pub(super) fn undo(panel: &mut Panel, cx: &mut Context<Panel>) {
    if let Some(editor) = panel.editor.as_mut() {
        editor.undo();
    }
    cx.notify();
}

/// Save the tree: as the screen's own layout, into the library layout being edited,
/// which the tray passes on to every screen that follows it, or as a new library
/// layout that the screen is given.
pub(super) fn done(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(editor) = panel.editor.take() else { return };
    cx.notify();
    if !editor.dirty() {
        return;
    }
    let Editor { sig, screen, opened, target, tree, .. } = editor;
    with_latest(panel, window, cx, move |panel, window, cx| {
        // Edit layout looks at the screen again: a window's edge dragged since it
        // opened has made the layout the screen's own.
        let target = match opened {
            Opened::Screen => {
                let saved = panel.config.desks.get(&sig).and_then(|d| d.zones.get(&screen)).and_then(zones::layout_of);
                screen_edit(saved.as_ref(), &panel.config.custom_layouts).0
            }
            _ => target,
        };
        let mut list = panel.config.custom_layouts.clone();
        let found = match &target {
            Target::Library(id) => list.iter().position(|v| v.get("id").and_then(Value::as_str) == Some(id.as_str())),
            _ => None,
        };
        let patch = match (target, found) {
            (Target::Screen, _) => {
                let layout = ZoneLayout { template: Template::Custom, custom: None, tree };
                let Some(layout) = library::screen_patch(&layout) else { return };
                json!({ "desks": { sig: { "zones": { screen: layout } } } })
            }
            (Target::Library(_), Some(at)) => {
                list[at]["tree"] = json!(tree);
                json!({ "custom_layouts": list })
            }
            // A library layout deleted while it was being edited from its tile comes
            // back as a new one, given to no screen.
            (Target::Library(_), None) if opened != Opened::Screen => {
                let (id, name) = (library::new_id(&list), library::next_name(&list));
                list.push(library::Entry { id, name, tree, screens: Vec::new() }.value());
                json!({ "custom_layouts": list })
            }
            // A new one is for the screen it was made on.
            (Target::Library(_) | Target::New, _) => {
                let (id, name) = (library::new_id(&list), library::next_name(&list));
                let entry = library::Entry { id, name, tree, screens: vec![screen.clone()] };
                list.push(entry.value());
                let Some(layout) = library::screen_patch(&entry.layout()) else { return };
                json!({ "custom_layouts": list, "desks": { sig: { "zones": { screen: layout } } } })
            }
        };
        panel.patch(patch, window, cx);
    });
}

/// Run `then` with the latest config from the tray, so a change to the library made
/// elsewhere since the panel last heard isn't lost when the whole list is saved.
pub fn with_latest(
    panel: &mut Panel,
    window: &mut Window,
    cx: &mut Context<Panel>,
    then: impl FnOnce(&mut Panel, &mut Window, &mut Context<Panel>) + 'static,
) {
    panel.call("config.get", json!({}), window, cx, move |panel, result, window, cx| {
        // Building the list from an older copy could drop a layout added elsewhere.
        let Some(config) = result.ok().and_then(|c| serde_json::from_value(c).ok()) else {
            let note = gpui_kit::component::notification::Notification::new()
                .message("Couldn't read your layouts, so nothing was saved. Try again");
            window.push_notification(note, cx);
            return;
        };
        panel.config = config;
        then(panel, window, cx);
    });
}

fn split(panel: &mut Panel, dir: Dir, cx: &mut Context<Panel>) {
    if let Some(editor) = panel.editor.as_mut() {
        // The new zones are numbered from the old one, so it stays selected.
        if let Some(tree) = zones::split_leaf(&editor.tree, editor.selected, dir) {
            let selected = editor.selected;
            editor.edit("split", tree, selected);
        }
    }
    cx.notify();
}

fn merge(panel: &mut Panel, cx: &mut Context<Panel>) {
    if let Some(editor) = panel.editor.as_mut() {
        if let Some((tree, joined)) = zones::merge_leaf(&editor.tree, editor.selected) {
            editor.edit("merge", tree, joined);
        }
    }
    cx.notify();
}

fn reset(panel: &mut Panel, cx: &mut Context<Panel>) {
    if let Some(editor) = panel.editor.as_mut() {
        if editor.tree != editor.base {
            let base = editor.base.clone();
            editor.edit("reset", base, 0);
        }
    }
    cx.notify();
}

pub(super) fn select(panel: &mut Panel, zone: usize, cx: &mut Context<Panel>) {
    if let Some(editor) = panel.editor.as_mut() {
        editor.selected = zone.min(zones::leaf_count(&editor.tree) - 1);
    }
    cx.notify();
}

pub(super) fn begin_drag(panel: &mut Panel, path: Vec<bool>) {
    if let Some(editor) = panel.editor.as_mut() {
        editor.dragging = Some(Dragging { path, before: editor.tree.clone() });
    }
}

/// The pointer moved with a divider held. `origin` is the drawing's top-left, and
/// `scale` how many window pixels a screen pixel takes.
pub(super) fn drag_to(
    panel: &mut Panel,
    pointer: Point<Pixels>,
    origin: Point<Pixels>,
    (scale, area): (f64, Rect),
    cx: &mut Context<Panel>,
) {
    let Some(editor) = panel.editor.as_mut() else { return };
    let Some(path) = editor.dragging.as_ref().map(|d| d.path.clone()) else { return };
    let Some(divider) = zones::dividers(&editor.tree, area).into_iter().find(|d| d.path == path) else {
        return;
    };
    let (x, y) = (f64::from(pointer.x - origin.x) / scale, f64::from(pointer.y - origin.y) / scale);
    let ratio = match divider.dir {
        Dir::Vertical => (x - f64::from(divider.area.x)) / f64::from(divider.area.w.max(1)),
        Dir::Horizontal => (y - f64::from(divider.area.y)) / f64::from(divider.area.h.max(1)),
    };
    editor.tree = zones::set_ratio(&editor.tree, &path, zones::snap_ratio(ratio as f32));
    cx.notify();
}

/// One drag is one Undo step.
pub(super) fn end_drag(panel: &mut Panel, cx: &mut Context<Panel>) {
    if let Some(editor) = panel.editor.as_mut() {
        if let Some(Dragging { before, .. }) = editor.dragging.take() {
            if before != editor.tree {
                let current = std::mem::replace(&mut editor.tree, before);
                let selected = editor.selected;
                editor.edit("divider move", current, selected);
            }
        }
    }
    cx.notify();
}

/// Why splitting the selected zone isn't possible, if it isn't.
fn split_blocked(tree: &Node, selected: usize, cells: &[Rect], dir: Dir) -> Option<&'static str> {
    if !zones::can_split(tree) {
        return Some("A layout can have 128 zones at most");
    }
    if zones::depth_of(tree, selected).is_none_or(|d| d >= zones::MAX_DEPTH) {
        return Some("This zone is already split as deep as it can go");
    }
    let cell = cells.get(selected)?;
    let side = if dir == Dir::Vertical { cell.w } else { cell.h };
    (side / 2 < zones::MIN_ZONE_PX).then_some("This zone is too small to split further")
}

/// The editor in place of the preview: the tree with its dividers, then the toolbar.
pub fn view(
    panel: &mut Panel,
    mon: &Mon,
    gap: u32,
    room: f64,
    height: f32,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    let work = super::screen_editor::work_of(mon);
    let (w, h) = (work.w, work.h);
    let scale = ((room - 34.) / f64::from(w)).min((f64::from(height) - 34.) / f64::from(h)).max(0.01);
    let area = Rect { x: 0, y: 0, w, h };
    let Some(editor) = panel.editor.as_ref() else { return div().into_any_element() };
    let (tree, selected, focus) = (editor.tree.clone(), editor.selected, editor.focus.clone());
    let dragging = editor.dragging.as_ref().map(|d| d.path.clone());
    let undo_name = editor.history.last().map(|s| s.name);

    let (cells, drawn) = (zones::rects(&tree, area, 0), zones::rects(&tree, area, gap as i32));
    let at = |v: i32| px((f64::from(v) * scale) as f32);
    let count = cells.len();
    let mut surface = div()
        .id("zone-editor-surface")
        .track_focus(&focus)
        .tab_index(0)
        .relative()
        .w(px((f64::from(w) * scale) as f32))
        .h(px((f64::from(h) * scale) as f32))
        .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, _, cx| match e.keystroke.key.as_str() {
            "left" | "up" => select(panel, selected.saturating_sub(1), cx),
            "right" | "down" => select(panel, (selected + 1).min(count - 1), cx),
            _ => {}
        }))
        .on_drag_move(cx.listener(move |panel, e: &DragMoveEvent<DragDivider>, _, cx| {
            drag_to(panel, e.event.position, e.bounds.origin, (scale, area), cx);
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|panel, _, _, cx| end_drag(panel, cx)))
        .on_mouse_up_out(MouseButton::Left, cx.listener(|panel, _, _, cx| end_drag(panel, cx)));
    for (i, (cell, r)) in cells.iter().zip(&drawn).enumerate() {
        let chosen = i == selected;
        let focus = focus.clone();
        surface = surface.child(
            div()
                .id(("edit-zone", i))
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
                .border_1()
                .rounded(px(6.))
                .cursor_pointer()
                .when(chosen, |d| d.bg(p.accent.opacity(0.1)).border_color(p.accent))
                .when(!chosen, |d| {
                    d.bg(p.raised)
                        .border_color(p.hairline_strong)
                        .hover(|d| d.bg(p.accent.opacity(0.1)).border_color(p.accent))
                })
                .child(div().text_size(px(24.)).child((i + 1).to_string()))
                .child(ui::tabular(div().text_size(px(13.)).text_color(p.secondary).child(format!("{} px", cell.w))))
                .on_click(cx.listener(move |panel, _, window, cx| {
                    select(panel, i, cx);
                    focus.focus(window, cx);
                })),
        );
    }
    for (k, d) in zones::dividers(&tree, area).into_iter().enumerate() {
        let vertical = d.dir == Dir::Vertical;
        let along = if vertical { d.area.w } else { d.area.h };
        let first = (along as f32 * d.ratio).round() as i32;
        let held = dragging.as_ref() == Some(&d.path);
        let (x, y) = if vertical { (d.area.x + first, d.area.y) } else { (d.area.x, d.area.y + first) };
        let (length_w, length_h) = (at(d.area.w), at(d.area.h));
        let path = d.path.clone();
        let line = div().bg(if held { p.accent } else { p.secondary });
        let handle = div()
            .id(("divider", k))
            .absolute()
            .flex()
            .items_center()
            .justify_center()
            .hover(|d| d.bg(p.accent.opacity(0.25)))
            .when(vertical, |d| d.left(at(x) - px(GRAB / 2.)).top(at(y)).w(px(GRAB)).h(length_h).cursor_col_resize())
            .when(!vertical, |d| d.top(at(y) - px(GRAB / 2.)).left(at(x)).h(px(GRAB)).w(length_w).cursor_row_resize())
            .child(if vertical { line.w(px(2.)).h_full() } else { line.h(px(2.)).w_full() })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |panel, _: &MouseDownEvent, _, _| begin_drag(panel, path.clone())),
            )
            .on_drag(DragDivider, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            });
        surface = surface.child(handle);
        if held {
            // The widths either side, while the divider is held.
            let (a, b) = (first, along - first);
            let tag = div()
                .absolute()
                .left(at(x) + px(if vertical { 12. } else { 8. }))
                .top(at(y) + px(if vertical { 8. } else { 12. }))
                .px(px(8.))
                .py(px(4.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hairline_strong)
                .rounded(px(6.))
                .text_size(px(12.))
                .child(ui::tabular(div().child(format!("{a} px and {b} px"))));
            surface = surface.child(tag);
        }
    }
    let card = div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(height))
        .p(px(16.))
        .bg(p.card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(12.))
        .child(surface);

    let split_side = split_blocked(&tree, selected, &cells, Dir::Vertical);
    let split_stack = split_blocked(&tree, selected, &cells, Dir::Horizontal);
    let can_merge = zones::merge_leaf(&tree, selected).is_some();
    let can_reset = tree != editor.base;
    let button = |id: &'static str, label: &'static str, why: Option<&'static str>, cx: &mut Context<Panel>| {
        ui::secondary(id, label, cx).disabled(why.is_some()).when_some(why, |b, why| b.tooltip(why))
    };
    let toolbar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    button("edit-split-side", "Split side by side", split_side, cx)
                        .on_click(cx.listener(|panel, _, _, cx| split(panel, Dir::Vertical, cx))),
                )
                .child(
                    button("edit-split-stack", "Split top and bottom", split_stack, cx)
                        .on_click(cx.listener(|panel, _, _, cx| split(panel, Dir::Horizontal, cx))),
                )
                .child(
                    button(
                        "edit-merge",
                        "Merge with neighbour",
                        (!can_merge).then_some("Merging needs the zone next to it to be a single zone"),
                        cx,
                    )
                    .on_click(cx.listener(|panel, _, _, cx| merge(panel, cx))),
                ),
        )
        // Stays together, so Done never wraps onto a line of its own.
        .child(
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap(px(8.))
                .child({
                    let label =
                        undo_name.map_or_else(|| "Nothing to undo".to_string(), |n| format!("Undo {n} (Ctrl+Z)"));
                    ui::secondary("edit-undo", "Undo", cx)
                        .disabled(undo_name.is_none())
                        .tooltip(label)
                        .on_click(cx.listener(|panel, _, _, cx| undo(panel, cx)))
                })
                .child(
                    button("edit-reset", "Reset", (!can_reset).then_some("Nothing to reset"), cx)
                        .on_click(cx.listener(|panel, _, _, cx| reset(panel, cx))),
                )
                .child(
                    ui::primary("edit-done", "Done")
                        .on_click(cx.listener(|panel, _, window, cx| done(panel, window, cx))),
                ),
        );

    div()
        .key_context(CONTEXT)
        .on_action(cx.listener(|panel, _: &DiscardEdit, _, cx| discard(panel, cx)))
        .on_action(cx.listener(|panel, _: &UndoEdit, _, cx| undo(panel, cx)))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(card)
        .child(toolbar)
        .into_any_element()
}
