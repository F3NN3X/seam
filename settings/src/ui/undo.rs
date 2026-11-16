//! The Undo notification: what happened, an Undo button and a bar that counts the time left.

use crate::theme::tokens;
use crate::theme::{motion, palette};
use crate::ui;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// One look for every Undo: `message`, an Undo button that runs `undo` and closes it, and a 2 px
/// bar under the text that runs down over `lasts`. The kit's autohide is a fixed 5 s with no
/// countdown, so the caller removes it when `lasts` is up. `generation` restarts the bar for each
/// new notice. With Windows' Animation effects off there's no bar, only the notice and its timer.
pub fn notice<T: 'static>(
    message: String,
    generation: u64,
    lasts: Duration,
    undo: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Notification {
    // A second click while the notice is still leaving mustn't undo twice.
    let used = Rc::new(Cell::new(false));
    let undo: Undo = Rc::new(move |window, cx| {
        if !used.replace(true) {
            undo(window, cx);
        }
    });
    let p = palette(cx);
    Notification::new()
        .id::<T>()
        .message(message)
        .bg(p.surface_raised)
        .border_color(p.hairline)
        .rounded(px(tokens::RADIUS_12))
        .content(move |_, _, cx| {
            let p = palette(cx);
            if !motion(cx) {
                return div().into_any_element();
            }
            let id = ElementId::NamedInteger("undo-countdown".into(), generation);
            let bar =
                div()
                    .h_full()
                    .rounded_full()
                    .bg(p.accent)
                    .with_animation(id, Animation::new(lasts), |bar, left| bar.w(relative(1. - left)));
            div().mt(px(tokens::SPACE_8)).h(px(2.)).w_full().rounded_full().bg(p.hairline).child(bar).into_any_element()
        })
        .action(move |_, _, cx| undo_button(undo.clone(), cx))
}

type Undo = Rc<dyn Fn(&mut Window, &mut App)>;

fn undo_button(undo: Undo, cx: &mut Context<Notification>) -> Button {
    ui::secondary("undo", "Undo", cx).accessibility_label("Undo").on_click(cx.listener(
        move |note: &mut Notification, _, window, cx| {
            undo(window, cx);
            note.dismiss(window, cx);
        },
    ))
}
