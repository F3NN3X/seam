//! The zone editor on the real screen: a borderless window over the screen's work
//! area, straight behind the Seam window, drawn from the panel's own `Editor` so
//! the two views can't disagree. One screen pixel is one window pixel at any
//! scaling. In the live view it takes no mouse input and clicks pass through.

use super::zone_editor::{
    begin_drag, discard, drag_to, end_drag, select, undo, DiscardEdit, DragDivider, UndoEdit, CONTEXT,
};
use crate::panel::{Page, Panel};
use crate::theme::palette;
use crate::ui;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::desk::Mon;
use seam::panelwin;
use seam::tray::PANEL_TITLE;
use seam::zones::{self, Dir, Rect};

/// How the window is found, and what a screen reader calls it.
const TITLE: &str = "Seam screen editor";
/// Room for a pointer to grab a divider, in window pixels.
const GRAB: f32 = 10.;
/// How often a window that came out the wrong size is put right.
const RETRIES: u8 = 3;

/// The area a layout is drawn in: the screen's work area, which is also what the
/// tray snaps windows into.
pub fn work_of(mon: &Mon) -> Rect {
    panelwin::work_area(Rect { x: mon.rect.left, y: mon.rect.top, w: mon.rect.width(), h: mon.rect.height() })
        .unwrap_or(Rect { x: mon.rect.left, y: mon.rect.top, w: mon.rect.width(), h: mon.rect.height() })
}

/// The window the panel has open, and what it was opened for.
pub struct Shown {
    handle: WindowHandle<ScreenEditor>,
    area: Rect,
    mirror: bool,
}

/// Where the window should be right now, and whether it only mirrors.
fn wanted(panel: &Panel) -> Option<(Rect, bool)> {
    let editor = panel.editor.as_ref()?;
    if panel.page != Page::Zones {
        return None;
    }
    let mon = panel.mons.iter().find(|m| m.id == editor.screen)?;
    Some((work_of(mon), !panel.config.zones.edit_on_screen))
}

/// Make the window match the editor: open it, move it to another screen, swap
/// its mode, or close it. Called from every draw, and a no-op once it matches.
pub fn sync(panel: &mut Panel, cx: &mut Context<Panel>) {
    let matches = match (&panel.screen_window, wanted(panel)) {
        (None, None) => true,
        (Some(shown), Some((area, mirror))) => shown.area == area && shown.mirror == mirror,
        _ => false,
    };
    if !matches {
        // A new window draws as it opens and reads the panel, so this has to
        // run with the panel free, not just after its draw.
        let entity = cx.entity();
        cx.defer(move |cx| apply(entity, cx));
    }
}

fn apply(entity: Entity<Panel>, cx: &mut App) {
    let (old, want) = entity.update(cx, |panel, _| (panel.screen_window.take(), wanted(panel)));
    let shown = match (old, want) {
        (None, None) => None,
        (Some(old), None) => {
            close(&old, cx);
            None
        }
        (None, Some((area, mirror))) => open(&entity, area, mirror, cx),
        (Some(old), Some((area, mirror))) if old.mirror == mirror => {
            let moved = old.handle.update(cx, |view, _, _| view.area = area).is_ok();
            moved.then(|| {
                place_later(area, mirror, cx);
                Shown { area, ..old }
            })
        }
        (Some(old), Some((area, mirror))) => {
            close(&old, cx);
            open(&entity, area, mirror, cx)
        }
    };
    entity.update(cx, |panel, cx| {
        panel.screen_window = shown;
        cx.notify();
    });
}

fn close(shown: &Shown, cx: &mut App) {
    let _ = shown.handle.update(cx, |_, window, _| window.remove_window());
}

/// Close the window with no say from the editor, for when the panel goes.
pub fn close_all(shown: Option<&Shown>, cx: &mut App) {
    if let Some(shown) = shown {
        close(shown, cx);
    }
}

fn open(panel: &Entity<Panel>, area: Rect, mirror: bool, cx: &mut App) -> Option<Shown> {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions { title: Some(TITLE.into()), appears_transparent: true, ..Default::default() }),
        // Only a starting point. Win32 puts it on the screen, in physical pixels.
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(640.), px(360.))))),
        window_background: WindowBackgroundAppearance::Transparent,
        is_resizable: false,
        is_minimizable: false,
        focus: false,
        ..Default::default()
    };
    let handle = cx
        .open_window(options, |window, cx| {
            // Closing it any other way would leave the panel believing it's open.
            window.on_window_should_close(cx, |_, _| false);
            cx.new(|cx| ScreenEditor::new(panel.clone(), area, mirror, window, cx))
        })
        .ok()?;
    place_later(area, mirror, cx);
    Some(Shown { handle, area, mirror })
}

/// gpui only hears about the new size if it isn't mid-update when Win32 sends
/// it, so this runs as a task of its own.
fn place_later(area: Rect, mirror: bool, cx: &mut App) {
    cx.spawn(async move |_| {
        panelwin::place(TITLE, PANEL_TITLE, area, mirror);
    })
    .detach();
}

/// The panel's activation brings the editor window back behind it.
pub fn restack() {
    panelwin::stack(TITLE, PANEL_TITLE, false);
}

struct ScreenEditor {
    panel: Entity<Panel>,
    /// The work area in physical pixels, in the desktop's coordinates.
    area: Rect,
    mirror: bool,
    focus: FocusHandle,
    retries: u8,
    _subscriptions: Vec<Subscription>,
}

impl ScreenEditor {
    fn new(panel: Entity<Panel>, area: Rect, mirror: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        if !mirror {
            focus.focus(window, cx);
        }
        let subscriptions = vec![
            cx.observe(&panel, |_, _, cx| cx.notify()),
            // A click here brings it forward, so the panel is lifted back over it.
            cx.observe_window_activation(window, |this, window, _| {
                if window.is_window_active() && !this.mirror {
                    panelwin::stack(TITLE, PANEL_TITLE, true);
                }
            }),
        ];
        ScreenEditor { panel, area, mirror, focus, retries: 0, _subscriptions: subscriptions }
    }
}

impl Render for ScreenEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let scale = window.scale_factor();
        // Win32 sized the window in physical pixels. If a change of scaling moved
        // that, try again, a few times.
        let size = window.viewport_size();
        let (got_w, got_h) =
            ((f32::from(size.width) * scale).round() as i32, (f32::from(size.height) * scale).round() as i32);
        if ((got_w - self.area.w).abs() > 1 || (got_h - self.area.h).abs() > 1) && self.retries < RETRIES {
            self.retries += 1;
            place_later(self.area, self.mirror, cx);
        }
        let panel = self.panel.read(cx);
        let Some(editor) = panel.editor.as_ref() else { return div().into_any_element() };
        let gap = panel.config.zones.gap.min(32);
        let (tree, selected) = (editor.tree.clone(), editor.selected);
        let dragging = editor.dragging.as_ref().map(|d| d.path.clone());

        // One window pixel is 1 / scale logical pixels, so a screen pixel is one.
        let at = |v: i32| px(v as f32 / scale);
        let area = Rect { x: 0, y: 0, w: self.area.w, h: self.area.h };
        let ratio = f64::from(1. / scale);
        let (cells, drawn) = (zones::rects(&tree, area, 0), zones::rects(&tree, area, gap as i32));
        let mirror = self.mirror;
        let mut root = div().relative().size_full();
        for (i, (cell, r)) in cells.iter().zip(&drawn).enumerate() {
            let chosen = i == selected;
            let panel = self.panel.clone();
            let label = div()
                .flex()
                .flex_col()
                .items_center()
                .px(px(12.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(p.raised.opacity(0.85))
                .text_color(p.text)
                .child(div().text_size(px(24.)).child((i + 1).to_string()))
                .child(ui::tabular(div().text_size(px(13.)).text_color(p.secondary).child(format!("{} px", cell.w))));
            root = root.child(
                div()
                    .id(("screen-zone", i))
                    .absolute()
                    .left(at(r.x))
                    .top(at(r.y))
                    .w(at(r.w))
                    .h(at(r.h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .border_1()
                    .rounded(px(6.))
                    .when(chosen, |d| d.bg(p.accent.opacity(0.18)).border_color(p.accent))
                    .when(!chosen, |d| d.bg(p.raised.opacity(0.3)).border_color(p.hairline_strong))
                    .when(!mirror, |d| {
                        d.cursor_pointer().on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            panel.update(cx, |panel, cx| select(panel, i, cx));
                        })
                    })
                    .child(label),
            );
        }
        for (k, d) in zones::dividers(&tree, area).into_iter().enumerate() {
            let vertical = d.dir == Dir::Vertical;
            let along = if vertical { d.area.w } else { d.area.h };
            let first = (along as f32 * d.ratio).round() as i32;
            let held = dragging.as_ref() == Some(&d.path);
            let (x, y) = if vertical { (d.area.x + first, d.area.y) } else { (d.area.x, d.area.y + first) };
            let (length_w, length_h) = (at(d.area.w), at(d.area.h));
            let line = div().bg(if held { p.accent } else { p.secondary });
            let thin = px(2. / scale);
            let mut handle =
                div().id(("screen-divider", k)).absolute().flex().items_center().justify_center().child(if vertical {
                    line.w(thin).h_full()
                } else {
                    line.h(thin).w_full()
                });
            handle = if vertical {
                handle.left(at(x) - px(GRAB / 2.)).top(at(y)).w(px(GRAB)).h(length_h)
            } else {
                handle.top(at(y) - px(GRAB / 2.)).left(at(x)).h(px(GRAB)).w(length_w)
            };
            if !mirror {
                let panel = self.panel.clone();
                let path = d.path.clone();
                handle = if vertical { handle.cursor_col_resize() } else { handle.cursor_row_resize() }
                    .hover(|d| d.bg(p.accent.opacity(0.25)))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        panel.update(cx, |panel, _| begin_drag(panel, path.clone()));
                    })
                    .on_drag(DragDivider, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    });
            }
            root = root.child(handle);
            if held {
                // The widths either side, in real pixels.
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
                    .text_color(p.text)
                    .child(ui::tabular(div().child(format!("{} px and {} px", first, along - first))));
                root = root.child(tag);
            }
        }
        if mirror {
            return root.into_any_element();
        }
        let (panel, focus) = (self.panel.clone(), self.focus.clone());
        let drag = panel.clone();
        let (up, out) = (panel.clone(), panel.clone());
        let (esc, back) = (panel.clone(), panel.clone());
        root.id("screen-editor")
            .key_context(CONTEXT)
            .track_focus(&focus)
            .on_action(move |_: &DiscardEdit, _, cx| {
                esc.update(cx, discard);
            })
            .on_action(move |_: &UndoEdit, _, cx| {
                back.update(cx, undo);
            })
            .on_mouse_down(MouseButton::Left, {
                let focus = focus.clone();
                move |_, window, cx| focus.focus(window, cx)
            })
            .on_drag_move(move |e: &DragMoveEvent<DragDivider>, _, cx| {
                drag.update(cx, |panel, cx| drag_to(panel, e.event.position, e.bounds.origin, (ratio, area), cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                up.update(cx, end_drag);
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                out.update(cx, end_drag);
            })
            .into_any_element()
    }
}
