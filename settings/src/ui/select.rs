//! A Select: one value out of a list. A feature that can be off has Off or None as its first option.

use crate::theme::palette;
use crate::theme::tokens::{self, text_style};
use crate::ui::dot;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use std::rc::Rc;

/// An option of a Select, with a small colour dot when it stands for a colour.
#[derive(Clone)]
pub struct Choice {
    key: String,
    label: SharedString,
    dot: Option<Hsla>,
}

impl Choice {
    pub fn new(key: impl Into<String>, label: impl Into<SharedString>) -> Choice {
        Choice { key: key.into(), label: label.into(), dot: None }
    }

    pub fn dot(self, colour: Option<Hsla>) -> Choice {
        Choice { dot: colour, ..self }
    }
}

/// The Select: the chosen option on a button, and the options in a menu under it. `pick` gets the
/// key of the option chosen.
pub fn select(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    options: Vec<Choice>,
    selected: &str,
    width: f32,
    cx: &App,
    pick: impl Fn(&str, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let p = palette(cx);
    let current = options.iter().find(|c| c.key == selected);
    let shown = current.map(|c| c.label.clone()).unwrap_or_default();
    let face = div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .children(current.and_then(|c| c.dot).map(|colour| dot(colour, true)))
        .child(text_style(div(), tokens::LABEL).child(shown.clone()));
    let (pick, selected) = (Rc::new(pick), selected.to_string());
    Button::new(id)
        .secondary()
        .border_1()
        .border_color(p.hairline)
        .rounded(px(tokens::RADIUS_8))
        .dropdown_caret(true)
        .accessibility_label(format!("{}: {shown}", label.into()))
        .w(px(width))
        .child(face)
        .dropdown_menu(move |mut menu, _, _| {
            for choice in &options {
                let (pick, key) = (pick.clone(), choice.key.clone());
                let item = match choice.dot {
                    Some(colour) => {
                        let text = choice.label.clone();
                        PopupMenuItem::element(move |_, _| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(tokens::SPACE_8))
                                .child(dot(colour, true))
                                .child(text.clone())
                        })
                    }
                    None => PopupMenuItem::new(choice.label.clone()),
                };
                menu = menu
                    .item(item.checked(choice.key == selected).on_click(move |_, window, cx| pick(&key, window, cx)));
            }
            menu
        })
        .into_any_element()
}
