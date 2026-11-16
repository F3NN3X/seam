//! The small set of parts every page is built from.

use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub const SECTION_GAP: f32 = tokens::SPACE_24;
/// Room left above a row a search result scrolled to.
const REVEAL_MARGIN: f32 = 24.;
/// Clear space between a control and its focus ring.
const RING_GAP: f32 = 2.;
/// Thickness of the focus ring.
const RING_WIDTH: f32 = 2.;

/// The row or section a search result opened, by title, and the page's scroll.
struct Reveal {
    title: SharedString,
    scroll: ScrollHandle,
    /// The page being built has that row, so it scrolls there once laid out.
    found: bool,
}

thread_local! {
    static REVEAL: RefCell<Option<Reveal>> = const { RefCell::new(None) };
}

/// Scroll the row or section titled `title` into view when its page next draws.
pub fn reveal(title: &'static str, scroll: &ScrollHandle) {
    REVEAL.with(|r| *r.borrow_mut() = Some(Reveal { title: title.into(), scroll: scroll.clone(), found: false }));
}

/// The page has been built. A result it has no row for stays at the top, and isn't
/// left waiting to scroll some later page.
pub fn reveal_built() {
    REVEAL.with(|r| {
        let mut r = r.borrow_mut();
        match r.as_mut() {
            // Until the row is laid out, a page built again looks for it again.
            Some(w) if w.found => w.found = false,
            _ => *r = None,
        }
    });
}

/// `element`, scrolled to once it's laid out if a search result is waiting for `title`.
fn anchored(title: &SharedString, element: Div) -> AnyElement {
    let waiting = REVEAL.with(|r| {
        let mut r = r.borrow_mut();
        let w = r.as_mut().filter(|w| w.title == *title)?;
        w.found = true;
        Some(w.scroll.clone())
    });
    let Some(scroll) = waiting else { return element.into_any_element() };
    let anchor = ScrollAnchor::for_handle(scroll.clone());
    let target = anchor.clone();
    element
        .on_children_prepainted(move |_, window, cx| {
            if REVEAL.with(|r| r.borrow_mut().take()).is_none() {
                return;
            }
            // The anchor knows where the row is only once this frame is laid out, so
            // the scroll happens before the next one, with a little room above it.
            target.scroll_to(window, cx);
            let scroll = scroll.clone();
            window.on_next_frame(move |window, _| {
                let at = scroll.offset();
                scroll.set_offset(point(at.x, (at.y + px(REVEAL_MARGIN)).min(px(0.))));
                window.refresh();
            });
        })
        .id(ElementId::Name(title.clone()))
        .anchor_scroll(Some(anchor))
        .into_any_element()
}

/// A focus handle that lives as long as the element with this id.
fn focus_for(id: &ElementId, window: &mut Window, cx: &mut App) -> FocusHandle {
    window.use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle()).read(cx).clone()
}

/// The ring is for keyboard users. A click doesn't leave one behind.
pub fn focused(focus: &FocusHandle, window: &Window) -> bool {
    focus.is_focused(window) && window.last_input_was_keyboard()
}

/// A solid ring in the `focus_ring` colour, `RING_GAP` outside `el`'s edge. `radius` and
/// `border` are the element's own; the border is left as it is.
pub fn focus_ring<E: Styled + ParentElement>(el: E, radius: f32, border: f32, p: Palette) -> E {
    let out = -(RING_GAP + RING_WIDTH + border);
    el.relative().child(
        div()
            .absolute()
            .top(px(out))
            .left(px(out))
            .right(px(out))
            .bottom(px(out))
            .border_2()
            .border_color(p.focus_ring)
            .rounded(px(radius + RING_GAP + RING_WIDTH)),
    )
}

/// Tabular figures, so scales and offsets line up.
pub fn tabular<E: Styled>(element: E) -> E {
    element.font_features(FontFeatures(Arc::new(vec![("tnum".to_string(), 1)])))
}

pub fn page_title(title: impl Into<SharedString>, subtitle: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_4))
        .child(text_style(div().font_family(tokens::DISPLAY_FONT), tokens::PAGE_TITLE).child(title.into()))
        .child(description(subtitle, p))
}

/// The second line of anything: 13 px, secondary text.
pub fn description(text: impl Into<SharedString>, p: Palette) -> Div {
    text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(text.into())
}

pub fn page_header(title: impl Into<SharedString>, subtitle: impl Into<SharedString>, p: Palette) -> Div {
    div().flex().items_start().justify_between().gap(px(tokens::SPACE_16)).child(page_title(title, subtitle, p))
}

pub fn section(title: impl Into<SharedString>, body: impl IntoElement, p: Palette) -> AnyElement {
    let title = title.into();
    let section = div()
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_8))
        .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(title.clone()))
        .child(body);
    anchored(&title, section)
}

/// Rows in a group, with a hairline between them.
pub fn card(rows: Vec<AnyElement>, p: Palette) -> Div {
    let mut group =
        div().flex().flex_col().bg(p.surface_card).border_1().border_color(p.hairline).rounded(px(tokens::RADIUS_12));
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
    let height = if about.is_some() { px(tokens::SIZE_ROW_2) } else { px(tokens::SIZE_ROW) };
    let title = title.into();
    let name = text_style(div(), tokens::BODY).child(title.clone());
    let mut text = div().flex().flex_col().gap(px(tokens::SPACE_2)).flex_1().min_w_0().child(name);
    if let Some(about) = about {
        text = text.child(about);
    }
    let row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(tokens::SPACE_16))
        .min_h(height)
        .px(px(tokens::SPACE_16))
        .py(px(tokens::SPACE_8))
        .child(text)
        .child(div().flex().flex_none().items_center().gap(px(tokens::SPACE_8)).child(control));
    anchored(&title, row)
}

pub fn secondary(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    let p = palette(cx);
    Button::new(id).label(label).secondary().border_1().border_color(p.hairline).rounded(px(tokens::RADIUS_8))
}

pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).label(label).primary().rounded(px(tokens::RADIUS_8))
}

/// A dashed note, for what needs saying but not doing right now. It may carry one button.
pub fn note(text: impl Into<SharedString>, action: Option<Button>, p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(tokens::SPACE_16))
        .px(px(tokens::SPACE_16))
        .py(px(tokens::SPACE_12))
        .border_1()
        .border_dashed()
        .border_color(p.hairline_strong)
        .rounded(px(tokens::RADIUS_12))
        .child(text_style(div().flex_1(), tokens::LABEL).text_color(p.text_secondary).child(text.into()))
        .children(action)
}

/// Status is always a dot and a word. Filled for good, hollow otherwise.
pub fn dot(color: Hsla, filled: bool) -> Div {
    let d = div().size(px(8.)).flex_none().rounded_full();
    if filled { d.bg(color) } else { d.border_1().border_color(color) }
}

pub fn switch(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    window: &mut Window,
    cx: &mut App,
    change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let p = palette(cx);
    let id = id.into();
    let focus = focus_for(&id, window, cx);
    let change = Rc::new(change);
    let (click, key) = (change.clone(), change);
    div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .role(Role::Switch)
        .aria_label(label)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .flex()
        .flex_none()
        .items_center()
        .w(px(40.))
        .h(px(20.))
        .p(px(3.))
        .rounded(px(tokens::RADIUS_10))
        .cursor_pointer()
        .map(|d| {
            if on {
                d.bg(p.fill_inverse).justify_end()
            } else {
                d.bg(p.surface_raised).border_1().border_color(p.control_stroke).justify_start()
            }
        })
        .when(focused(&focus, window), |d| focus_ring(d, tokens::RADIUS_10, if on { 0. } else { 1. }, p))
        .child(if on {
            div().size(px(14.)).rounded_full().bg(p.text_on_inverse)
        } else {
            div().size(px(12.)).rounded_full().bg(p.control_stroke)
        })
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
    label: impl Into<SharedString>,
    options: &[&'static str],
    selected: usize,
    window: &mut Window,
    cx: &mut App,
    change: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let labels = options.iter().map(|label| SharedString::from(*label)).collect();
    choice(id, label, labels, None, selected, window, cx, change)
}

/// A segmented control whose labels are made at run time, and whose `blocked`
/// option can't be picked and says why.
#[allow(clippy::too_many_arguments)]
pub fn choice(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    options: Vec<SharedString>,
    blocked: Option<(usize, &'static str)>,
    selected: usize,
    window: &mut Window,
    cx: &mut App,
    change: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let p = palette(cx);
    let id = id.into();
    let focus = focus_for(&id, window, cx);
    let change = Rc::new(change);
    let count = options.len();
    let keys = change.clone();
    let blocked_at = blocked.map(|(i, _)| i);
    // The next option in a direction, jumping over the blocked one.
    let neighbour = move |from: usize, back: bool| {
        let to = if back { from.checked_sub(1)? } else { from + 1 };
        let to = if Some(to) == blocked_at { if back { to.checked_sub(1)? } else { to + 1 } } else { to };
        (to < count).then_some(to)
    };
    let mut track = div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .role(Role::RadioGroup)
        .aria_label(label)
        .flex()
        .gap(px(tokens::SPACE_2))
        .p(px(tokens::SPACE_4))
        .bg(p.surface_sidebar)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(tokens::RADIUS_10))
        .when(focused(&focus, window), |d| focus_ring(d, tokens::RADIUS_10, 1., p))
        .on_key_down(move |e: &KeyDownEvent, window, cx| {
            let to = match e.keystroke.key.as_str() {
                "left" => neighbour(selected, true),
                "right" => neighbour(selected, false),
                _ => None,
            };
            if let Some(to) = to {
                keys(to, window, cx);
            }
        });
    for (i, label) in options.into_iter().enumerate() {
        let pick = change.clone();
        let why = blocked.filter(|(at, _)| *at == i).map(|(_, why)| why);
        let segment = div()
            .id(i)
            .role(Role::RadioButton)
            .aria_label(label.clone())
            .aria_toggled(if i == selected { Toggled::True } else { Toggled::False })
            .px(px(tokens::SPACE_12))
            .py(px(tokens::SPACE_4))
            .rounded(px(tokens::RADIUS_6))
            .when(i == selected, |d| d.bg(p.surface_segment).text_color(p.text_primary))
            .when(i != selected, |d| d.text_color(p.text_secondary))
            .child(label);
        let segment = text_style(segment, tokens::LABEL);
        track = track.child(match why {
            Some(why) => segment
                .opacity(tokens::OPACITY_DISABLED)
                .tooltip(move |window, cx| Tooltip::new(why).build(window, cx))
                .into_any_element(),
            None => segment.cursor_pointer().on_click(move |_, window, cx| pick(i, window, cx)).into_any_element(),
        });
    }
    track
}

/// One keycap per key.
pub fn keycaps(combo: &str, conflict: bool, p: Palette) -> Div {
    let mut caps = div().flex().items_center().gap(px(tokens::SPACE_4));
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
            text_style(div(), tokens::CAPTION)
                .px(px(tokens::SPACE_8))
                .py(px(tokens::SPACE_2))
                .border_1()
                .border_b_2()
                .border_color(edge)
                .rounded(px(tokens::RADIUS_4))
                .child(label.to_string()),
        );
    }
    caps
}
