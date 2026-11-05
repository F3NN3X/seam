//! The small set of parts every page is built from.

use crate::theme::{palette, Palette};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::ThemeStyled;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;
use std::sync::Arc;

pub const SECTION_GAP: f32 = 24.;

/// A focus handle that lives as long as the element with this id.
fn focus_for(id: &ElementId, window: &mut Window, cx: &mut App) -> FocusHandle {
    window.use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle()).read(cx).clone()
}

/// The ring is for keyboard users. A click doesn't leave one behind.
pub fn focused(focus: &FocusHandle, window: &Window) -> bool {
    focus.is_focused(window) && window.last_input_was_keyboard()
}

/// Tabular figures, so scales and offsets line up.
pub fn tabular<E: Styled>(element: E) -> E {
    element.font_features(FontFeatures(Arc::new(vec![("tnum".to_string(), 1)])))
}

pub fn page_title(title: impl Into<SharedString>, subtitle: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().font_family("Segoe UI Variable Display").text_size(px(24.)).child(title.into()))
        .child(description(subtitle, p))
}

/// The second line of anything: 13 px, secondary text.
pub fn description(text: impl Into<SharedString>, p: Palette) -> Div {
    div().text_size(px(13.)).line_height(px(18.)).text_color(p.secondary).child(text.into())
}

pub fn page_header(title: impl Into<SharedString>, subtitle: impl Into<SharedString>, p: Palette) -> Div {
    div().flex().items_start().justify_between().gap(px(16.)).child(page_title(title, subtitle, p))
}

pub fn section(title: impl Into<SharedString>, body: impl IntoElement, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().text_size(px(13.)).text_color(p.secondary).child(title.into()))
        .child(body)
}

/// Rows in a group, with a hairline between them.
pub fn card(rows: Vec<AnyElement>, p: Palette) -> Div {
    let mut group = div().flex().flex_col().bg(p.card).border_1().border_color(p.hairline).rounded(px(12.));
    for (i, row) in rows.into_iter().enumerate() {
        group = group.child(div().when(i > 0, |d| d.border_t_1().border_color(p.hairline)).child(row));
    }
    group
}

pub fn row(
    title: impl Into<SharedString>,
    about: Option<SharedString>,
    control: impl IntoElement,
    p: Palette,
) -> AnyElement {
    row_with(title, about.map(|a| description(a, p).into_any_element()), control)
}

pub fn row_with(title: impl Into<SharedString>, about: Option<AnyElement>, control: impl IntoElement) -> AnyElement {
    let height = if about.is_some() { px(56.) } else { px(48.) };
    let name = div().text_size(px(14.)).line_height(px(20.)).child(title.into());
    let mut text = div().flex().flex_col().gap(px(2.)).flex_1().min_w_0().child(name);
    if let Some(about) = about {
        text = text.child(about);
    }
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .min_h(height)
        .px(px(16.))
        .py(px(8.))
        .child(text)
        .child(div().flex().flex_none().items_center().gap(px(8.)).child(control))
        .into_any_element()
}

pub fn secondary(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    let p = palette(cx);
    Button::new(id).label(label).secondary().border_1().border_color(p.hairline).rounded(px(8.))
}

pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).label(label).primary().rounded(px(8.))
}

/// A dashed note, for what needs saying but not doing right now. It may carry one button.
pub fn note(text: impl Into<SharedString>, action: Option<Button>, p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .px(px(16.))
        .py(px(12.))
        .border_1()
        .border_dashed()
        .border_color(p.hairline_strong)
        .rounded(px(12.))
        .child(div().flex_1().text_size(px(13.)).line_height(px(18.)).text_color(p.secondary).child(text.into()))
        .children(action)
}

/// Status is always a dot and a word. Filled for good, hollow otherwise.
pub fn dot(color: Hsla, filled: bool) -> Div {
    let d = div().size(px(8.)).flex_none().rounded_full();
    if filled {
        d.bg(color)
    } else {
        d.border_1().border_color(color)
    }
}

pub fn switch(
    id: impl Into<ElementId>,
    on: bool,
    window: &mut Window,
    cx: &mut App,
    change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let p = palette(cx);
    let id = id.into();
    let focus = focus_for(&id, window, cx);
    let change = Rc::new(change);
    let (click, key) = (change.clone(), change);
    div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .flex_none()
        .items_center()
        .w(px(40.))
        .h(px(20.))
        .p(px(3.))
        .rounded(px(10.))
        .cursor_pointer()
        .map(|d| {
            if on {
                d.bg(p.text).justify_end()
            } else {
                d.bg(p.raised).border_1().border_color(p.hairline_strong).justify_start()
            }
        })
        .when(focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .child(div().size(px(14.)).rounded_full().bg(if on { p.page } else { p.muted }))
        .on_click(move |_, window, cx| click(!on, window, cx))
        .on_key_down(move |e: &KeyDownEvent, window, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                key(!on, window, cx);
            }
        })
}

/// A choice of a few short options. Arrow keys move the choice.
pub fn segmented(
    id: impl Into<ElementId>,
    options: &[&'static str],
    selected: usize,
    window: &mut Window,
    cx: &mut App,
    change: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let p = palette(cx);
    let id = id.into();
    let focus = focus_for(&id, window, cx);
    let change = Rc::new(change);
    let count = options.len();
    let keys = change.clone();
    let mut track = div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .gap(px(2.))
        .p(px(4.))
        .bg(p.sidebar)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(10.))
        .when(focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .on_key_down(move |e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
            "left" if selected > 0 => keys(selected - 1, window, cx),
            "right" if selected + 1 < count => keys(selected + 1, window, cx),
            _ => {}
        });
    for (i, label) in options.iter().enumerate() {
        let pick = change.clone();
        track = track.child(
            div()
                .id(i)
                .px(px(12.))
                .py(px(4.))
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .when(i == selected, |d| d.bg(p.segment).text_color(p.text))
                .when(i != selected, |d| d.text_color(p.secondary))
                .child(*label)
                .on_click(move |_, window, cx| pick(i, window, cx)),
        );
    }
    track
}

/// One keycap per key.
pub fn keycaps(combo: &str, conflict: bool, p: Palette) -> Div {
    let mut caps = div().flex().items_center().gap(px(4.));
    for key in combo.split('+') {
        let label = match key {
            "Left" => "←",
            "Right" => "→",
            "Up" => "↑",
            "Down" => "↓",
            other => other,
        };
        let edge = if conflict { p.negative } else { p.hairline_strong };
        caps = caps.child(
            div()
                .px(px(8.))
                .py(px(2.))
                .text_size(px(12.))
                .border_1()
                .border_b_2()
                .border_color(edge)
                .rounded(px(4.))
                .child(label.to_string()),
        );
    }
    caps
}
