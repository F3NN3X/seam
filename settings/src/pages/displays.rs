//! Displays: the desk, the physical map, the borders and the pointer.

use crate::panel::{Panel, error_text, refusal};
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui::{self, Card, Row, card, row, section, switch};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::api::LayoutView;
use seam::config::{Desk, MAX_EDGE_RESISTANCE_PX, MAX_GLUE_PUSH_PX, SavedBorder};
use seam::desk::{Placed, month_day};
use seam::geom::{EdgeMap, Rect, Side, corridor, facing_gap, touching};
use seam::state::{Border, BorderState, State};
use seam::winutil;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

mod borders;
mod desks;
mod layout;
mod physical_map;
mod pointer;

const MAP_HEIGHT: f32 = 400.;
const MAP_PADDING: f32 = 12.;
const SEAM_WIDTH: f32 = 2.;
/// A border's line can be clicked this far across.
const HIT_WIDTH: f32 = 16.;
const SCALE_BAR_MM: f64 = 200.;
/// Below this height a screen's number and name sit side by side.
const SHORT_TILE: f64 = 48.;
/// The widest gap or overlap in Windows' arrangement that the Windows view labels.
const LABEL_PX: i32 = 400;
/// The page's widest content, less what the map's card takes around it.
const CONTENT_WIDTH: f64 = 1040. - 34.;

/// Ask the tray to turn on Windows' Ease cursor movement. Seam never turns it off.
pub fn turn_on_easing(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    panel.call("cursor_easing.set", json!({ "on": true }), window, cx, |panel, result, window, cx| {
        panel.ease_on = seam::ease::is_on();
        match result {
            Ok(_) => window.push_notification(Notification::new().message("Turned on Ease cursor movement"), cx),
            Err(e) => window.push_notification(refusal("Ease cursor movement left as it was", &error_text(&e)), cx),
        }
        panel.reload_config(window, cx);
        cx.notify();
    });
}

fn ease_alert(p: Palette, cx: &mut Context<Panel>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .p(px(16.))
        .bg(p.surface_card)
        .border_1()
        .border_color(p.accent)
        .rounded(px(12.))
        .child(Icon::new(assets::IconName::TriangleAlert).size(px(16.)).text_color(p.accent))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .child(div().text_size(px(14.)).child("Turn on Ease cursor movement between displays"))
                .child(ui::description("Without it the pointer can get stuck at some borders", p)),
        )
        .child(ui::secondary("open-display-settings", "Open Display settings", cx).on_click(|_, _, cx| {
            cx.open_url("ms-settings:display");
        }))
        .child(
            ui::primary("turn-on-easing", "Turn it on")
                .on_click(cx.listener(|panel, _, window, cx| turn_on_easing(panel, window, cx))),
        )
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let one_screen = panel.mons.len() < 2;
    let header = ui::page_header_with("Displays", Some("Pointer and borders"), desks::desk_menu(panel, cx), cx)
        .child(header_actions(panel, one_screen, cx));

    let width = (f64::from(window.viewport_size().width) - 240. - 64. - 34.).min(CONTENT_WIDTH);
    let mut content = vec![];
    if let Some(state) = panel.state.clone() {
        content.push(layout::card_of(panel, &state, width, p, window, cx));
        content.push(borders::section(panel, &state, p, window, cx));
    }
    let pointer = pointer::card_of(panel, p, window, cx);

    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(header)
        .when(!panel.ease_on, |d| d.child(ease_alert(p, cx)))
        .children(content)
        .child(section("Pointer", pointer, p))
}

/// "Match Windows to this layout", while Seam isn't doing it for you, and "Line up screens".
fn header_actions(panel: &Panel, one_screen: bool, cx: &mut Context<Panel>) -> Div {
    let align =
        ui::secondary("align", "Match Windows to this layout", cx).on_click(cx.listener(|panel, _, window, cx| {
            panel.call("arrangement.align", json!({}), window, cx, |_, result, window, cx| match result {
                Ok(_) => window.push_notification(Notification::new().message("Aligned the arrangement"), cx),
                Err(e) => {
                    let body = format!("{}. The pointer remap still works", error_text(&e));
                    window.push_notification(refusal("Arrangement left as it was", &body), cx);
                }
            });
        }));
    let line_up = ui::primary("line-up", "Line up screens")
        .disabled(one_screen)
        .when(one_screen, |b| b.tooltip("Only one screen is connected, so there's nothing to line up"))
        .on_click(cx.listener(|panel, _, window, cx| panel.command("lineup.start", json!({}), window, cx)));
    div()
        .flex()
        .flex_none()
        .gap(px(tokens::SPACE_8))
        .when(!panel.config.align_arrangement, |d| d.child(align))
        .child(line_up)
}
