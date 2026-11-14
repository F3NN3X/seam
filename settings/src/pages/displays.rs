//! The physical map, the borders, saved desks and the pointer switches.

use crate::panel::{Panel, error_text, refusal};
use crate::theme::{Palette, palette};
use crate::ui::{self, card, row, section, switch};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::{Disableable, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::api::LayoutView;
use seam::config::{Desk, MAX_EDGE_RESISTANCE_PX, MAX_GLUE_PUSH_PX, SavedBorder};
use seam::desk::{Placed, month_day, relative_day};
use seam::geom::{EdgeMap, Rect, Side, corridor, facing_gap, touching};
use seam::state::{Border, BorderState, State};
use seam::winutil;
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod desks;
mod physical_map;

use desks::{desk_rows, relative};
use physical_map::map;

const MAP_HEIGHT: f32 = 240.;
const MAP_PADDING: f32 = 12.;
const SEAM_WIDTH: f32 = 3.;
const SCALE_BAR_MM: f64 = 200.;
/// Below this height a screen's number and name sit side by side.
const SHORT_TILE: f64 = 48.;
const BORDER_NAME_WIDTH: f32 = 242.;
/// A line shorter than this has no room for its note.
const NOTE_FITS: f64 = 96.;
/// The widest gap or overlap in Windows' arrangement that the Windows view labels.
const LABEL_PX: i32 = 400;
const UNDO_FOR: Duration = Duration::from_secs(8);

fn border_rows(panel: &Panel, state: &State, p: Palette, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let edge = |side: Side| match side {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Top => "Top",
        Side::Bottom => "Bottom",
    };
    let lined_up_when = panel.config.desks.get(&state.desk.sig).and_then(|d| d.lined_up_at).and_then(relative);
    let rows = state.borders.iter().enumerate().map(|(i, b)| {
        let (a, other) = (b.a as usize, b.b as usize);
        let link = panel.links.iter().find(|l| (l.a + 1, l.b + 1) == (a, other));
        let no_size = [a, other].into_iter().find(|n| panel.mons.get(n - 1).is_some_and(|m| m.raw_dpi <= 0.));
        let detail = match (link, no_size) {
            (Some(l), _) if l.not_on_desk => "The pointer stops at this edge".to_string(),
            (Some(l), _) => format!("Scale {:.2}, offset {:+.0} px", l.map.s, l.map.o),
            (None, Some(n)) => format!("Screen {n} reports no size. Line up guesses a scale of 1"),
            (None, None) => "Line up this border to set it".to_string(),
        };
        let lined_up = b.state == BorderState::LinedUp;
        let status = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .child(ui::dot(if lined_up { p.positive } else { p.muted }, lined_up))
            .child(match (lined_up, lined_up_when.as_deref()) {
                (true, Some(when)) => format!("Lined up {when}"),
                (true, None) => "Lined up".to_string(),
                (false, _) => "Not lined up".to_string(),
            });
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .min_h(px(56.))
            .px(px(16.))
            .py(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_none()
                    .w(px(BORDER_NAME_WIDTH))
                    .child(div().text_size(px(14.)).line_height(px(20.)).child(format!("Screen {a} to screen {other}")))
                    .child(ui::description(
                        if b.not_on_desk {
                            "Not on your desk".to_string()
                        } else if b.glued {
                            format!("Glued to the {} edge of {a}", edge(b.side).to_lowercase())
                        } else {
                            format!("{} edge of {a}", edge(b.side))
                        },
                        p,
                    )),
            )
            .child(ui::tabular(ui::description(detail, p)).flex_1().min_w_0())
            .child(status)
            .child(border_menu(panel, b, i, cx))
            .into_any_element()
    });
    rows.collect()
}

/// The "..." of a border that touches in Windows, which can be marked as not on the desk.
/// Other borders get an empty space of its size, so the rows line up.
fn border_menu(panel: &Panel, b: &Border, i: usize, cx: &mut Context<Panel>) -> AnyElement {
    let touch = |n: u32| panel.mons.get(n as usize - 1).map(|m| m.rect);
    let touching_now = touch(b.a).zip(touch(b.b)).is_some_and(|(a, other)| touching(&a, &other).is_some());
    if !touching_now {
        return div().w(px(32.)).flex_none().into_any_element();
    }
    let view = cx.entity().downgrade();
    let border = b.clone();
    let label = if b.not_on_desk { "On my desk" } else { "Not on my desk" };
    ui::secondary(("border-more", i), "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for screen {} to screen {}", b.a, b.b))
        .dropdown_menu(move |menu, _, _| {
            let (view, border) = (view.clone(), border.clone());
            menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let border = border.clone();
                let _ = view.update(cx, |panel, cx| toggle_not_on_desk(panel, &border, window, cx));
            }))
        })
        .into_any_element()
}

/// Mark a border as not on the desk, or back. A border that was never lined up gets a
/// stand-in map, which "On my desk" takes away again, because nothing was measured.
fn toggle_not_on_desk(panel: &mut Panel, border: &Border, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(state) = panel.state.as_ref() else { return };
    let id = |n: u32| state.screens.iter().find(|s| s.number == n).map(|s| s.id.clone());
    let (Some(a), Some(b)) = (id(border.a), id(border.b)) else { return };
    let sig = state.desk.sig.clone();
    let known = panel.config.desks.get(&sig);
    let mut saved = known.map(|d| d.borders.clone()).unwrap_or_default();
    let stand_in = EdgeMap { s: 1., o: 0. };
    match saved.iter().position(|s| (&s.a, &s.b) == (&a, &b) || (&s.a, &s.b) == (&b, &a)) {
        Some(at) if saved[at].not_on_desk && saved[at].map == stand_in => {
            saved.remove(at);
        }
        Some(at) => saved[at].not_on_desk = !saved[at].not_on_desk,
        None => saved.push(SavedBorder { a, b, side: border.side, map: stand_in, glued: false, not_on_desk: true }),
    }
    let mut desk = json!({ "borders": saved });
    if known.is_none() {
        desk["name"] = json!(state.desk.name);
    }
    panel.patch(json!({ "desks": { sig: desk } }), window, cx);
}

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
        .bg(p.card)
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
    let screens = panel.mons.len();
    let subtitle = format!("{screens} screen{} on {}", if screens == 1 { "" } else { "s" }, panel.desk_name());
    let one_screen = screens < 2;

    let actions = header_actions(one_screen, cx);

    let width = f64::from(window.viewport_size().width) - 240. - 64. - 34.;
    let mut content = vec![];
    if let Some(state) = panel.state.clone() {
        content.push(layout_card(panel, &state, one_screen, width, p, window, cx));
    }

    let pointer = pointer_card(panel, p, window, cx);
    let mut rows = desk_rows(panel, window, cx);
    if rows.is_empty() {
        rows.push(ui::row("No saved desks yet", Some("Line up your screens to save this one".into()), div(), p));
    }
    let desks = card(rows, p);

    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(ui::page_header("Displays", subtitle, p).child(actions))
        .when(!panel.ease_on, |d| d.child(ease_alert(p, cx)))
        .children(content)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(24.))
                .child(div().flex_1().min_w_0().child(section("Saved desks", desks, p)))
                .child(div().flex_1().min_w_0().child(section("Pointer", pointer, p))),
        )
}

/// Align arrangement and Line up screens, at the right of the page header.
fn header_actions(one_screen: bool, cx: &mut Context<Panel>) -> Div {
    let align = ui::secondary("align", "Align arrangement", cx).on_click(cx.listener(|panel, _, window, cx| {
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
    div().flex().flex_none().gap(px(8.)).child(align).child(line_up)
}

/// The physical layout: the map, then a row for each border.
fn layout_card(
    panel: &Panel,
    state: &State,
    one_screen: bool,
    width: f64,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let seam_view = panel.layout_view == LayoutView::Seam;
    let caption = if seam_view {
        "Real sizes from each screen's EDID, joined where you lined them up"
    } else {
        "How Windows arranges them, in pixels"
    };
    let view = cx.entity();
    let views = ui::segmented(
        "layout-view",
        "Layout view",
        &["Windows", "Seam"],
        usize::from(seam_view),
        window,
        cx,
        move |at, _, cx| {
            view.update(cx, |panel, cx| {
                panel.layout_view = if at == 1 { LayoutView::Seam } else { LayoutView::Windows };
                cx.notify();
            });
        },
    );
    let show = ui::secondary("show-layout", "Show on screens", cx).on_click(cx.listener(|panel, _, window, cx| {
        let view = if panel.layout_view == LayoutView::Seam { "seam" } else { "windows" };
        panel.command("layout.show", json!({ "view": view }), window, cx);
    }));
    let title = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(16.)).child("Physical layout"))
                .child(div().text_size(px(12.)).text_color(p.muted).child(caption)),
        )
        .when(!one_screen, |d| d.child(div().flex().flex_none().items_center().gap(px(8.)).child(views).child(show)));
    let body = if one_screen {
        div()
            .text_size(px(13.))
            .text_color(p.secondary)
            .child("Only one screen is connected, so there's nothing to line up")
    } else {
        map(panel, state, width, p, cx)
    };
    let top = div().flex().flex_col().gap(px(12.)).p(px(16.)).child(title).child(body).into_any_element();
    let mut rows = vec![top];
    rows.extend(border_rows(panel, state, p, cx));
    card(rows, p).into_any_element()
}

/// The Pointer card: the remap switch, the two sliders and the Line Up switches.
fn pointer_card(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let view = cx.entity();
    let remap = switch("remap", "Remap pointer at borders", panel.config.enabled, window, cx, {
        let view = view.clone();
        move |on, window, cx| view.update(cx, |panel, cx| panel.patch(json!({ "enabled": on }), window, cx))
    });
    let measure = switch("lineup-measure", "Type measurements in Line Up", panel.config.lineup_measure, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "lineup_measure": on }), window, cx));
        }
    });
    let also_align = switch(
        "align-arrangement",
        "Also align Windows arrangement",
        panel.config.align_arrangement,
        window,
        cx,
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "align_arrangement": on }), window, cx));
        },
    );
    // The sliders follow the settings when something else changed them.
    let pointer_cfg = &panel.config.pointer;
    let resistance = pointer_cfg.edge_resistance_px.min(MAX_EDGE_RESISTANCE_PX);
    let push = pointer_cfg.glue_push_px.min(MAX_GLUE_PUSH_PX);
    for (slider, value) in [(&panel.edge_resistance, resistance), (&panel.glue_push, push)] {
        if slider.read(cx).value().end() != value as f32 {
            slider.update(cx, |slider, cx| slider.set_value(value as f32, window, cx));
        }
    }
    let ends = |low: &'static str, high: &'static str, slider: &Entity<SliderState>| {
        let end = |text| div().text_size(px(12.)).text_color(p.muted).child(text);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .child(end(low))
            .child(Slider::new(slider).horizontal().w(px(120.)))
            .child(end(high))
    };
    // The value sits under the title, as on the Gap slider, above the caption.
    let about = |value: String, caption: &'static str| {
        div().flex().flex_col().child(ui::description(value, p)).child(ui::description(caption, p)).into_any_element()
    };
    let resistance_value = if resistance == 0 { "Off".into() } else { format!("{resistance} px") };
    card(
        vec![
            row("Remap pointer at borders", Some("Cross where the screens physically meet".into()), remap, p),
            ui::row_with(
                "Edge resistance",
                Some(about(
                    resistance_value,
                    "Push this many pixels past a screen's edge before the pointer crosses, so you can grab a \
                     window's edge there. 0 is off",
                )),
                ends("0", "100", &panel.edge_resistance),
            ),
            ui::row_with(
                "Glued edge push",
                Some(about(
                    format!("{push} px"),
                    "How far to push past a glued edge before the pointer crosses. An auto-hide taskbar there \
                     shows while you push",
                )),
                ends("0", "200", &panel.glue_push),
            ),
            row(
                "Also align Windows arrangement",
                Some("Move screens in Display settings to match".into()),
                also_align,
                p,
            ),
            row(
                "Type measurements in Line Up",
                Some("Adds a field to type how far the screens are offset, in cm".into()),
                measure,
                p,
            ),
        ],
        p,
    )
}
