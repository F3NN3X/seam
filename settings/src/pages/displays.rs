//! The physical map, the borders, saved desks and the pointer switches.

use crate::panel::{error_text, refusal, Panel};
use crate::theme::{palette, Palette};
use crate::ui::{self, card, row, section, switch};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::config::Desk;
use seam::desk::{month_day, relative_day, Placed};
use seam::geom::Side;
use seam::state::{BorderState, State};
use seam::winutil;
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAP_HEIGHT: f32 = 240.;
const MAP_PADDING: f32 = 12.;
const SEAM_WIDTH: f32 = 3.;
const SCALE_BAR_MM: f64 = 200.;
/// Below this height a screen's number and name sit side by side.
const SHORT_TILE: f64 = 48.;
const BORDER_NAME_WIDTH: f32 = 242.;
const UNDO_FOR: Duration = Duration::from_secs(8);

struct View {
    scale: f64,
    left: f64,
    top: f64,
}

/// Fit the screens into the map, keeping real proportions.
fn fit(layout: &[Placed], width: f64) -> View {
    let min_x = layout.iter().map(|s| s.x).fold(f64::MAX, f64::min);
    let min_y = layout.iter().map(|s| s.y).fold(f64::MAX, f64::min);
    let max_x = layout.iter().map(|s| s.x + s.w).fold(f64::MIN, f64::max);
    let max_y = layout.iter().map(|s| s.y + s.h).fold(f64::MIN, f64::max);
    let room = (MAP_PADDING * 2.) as f64;
    let (span_x, span_y) = ((max_x - min_x).max(1.), (max_y - min_y).max(1.));
    let scale = ((width - room) / span_x).min((MAP_HEIGHT as f64 - room) / span_y);
    View {
        scale,
        left: (width - span_x * scale) / 2. - min_x * scale,
        top: (MAP_HEIGHT as f64 - span_y * scale) / 2. - min_y * scale,
    }
}

fn screen_tile(panel: &Panel, s: &Placed, view: &View, p: Palette, cx: &mut Context<Panel>) -> impl IntoElement {
    let number = s.index + 1;
    let name = panel.mons.get(s.index).map_or("", |m| m.name.as_str()).to_string();
    let primary = panel.mons.get(s.index).is_some_and(|m| m.primary);
    let label = if primary { format!("{name}, main") } else { name };
    let height = s.h * view.scale;
    div()
        .id(("screen", s.index))
        .absolute()
        .left(px((s.x * view.scale + view.left) as f32))
        .top(px((s.y * view.scale + view.top) as f32))
        .w(px((s.w * view.scale) as f32))
        .h(px(height as f32))
        .flex()
        .when(height >= SHORT_TILE, |d| d.flex_col())
        .when(height < SHORT_TILE, |d| d.gap(px(8.)))
        .items_center()
        .justify_center()
        .overflow_hidden()
        .bg(p.card)
        .border_1()
        .border_color(p.hairline_strong)
        .when(!s.sized, |d| d.border_dashed())
        .rounded(px(4.))
        .cursor_pointer()
        .child(div().text_size(px(16.)).child(number.to_string()))
        .child(div().text_size(px(11.)).text_color(p.secondary).truncate().child(label))
        .on_click(cx.listener(move |panel, _, window, cx| {
            panel.command("screen.identify", json!({ "number": number }), window, cx);
        }))
}

/// A bar over the part of the edge two screens physically share.
fn seam_bar(a: &Placed, b: &Placed, side: Side, lined_up: bool, view: &View, p: Palette) -> Option<Div> {
    let bar = div().absolute();
    let color = if lined_up { p.accent } else { p.muted };
    let along_y = side.along_is_y();
    let (from, to) =
        if along_y { (a.y.max(b.y), (a.y + a.h).min(b.y + b.h)) } else { (a.x.max(b.x), (a.x + a.w).min(b.x + b.w)) };
    if to <= from {
        return None;
    }
    let edge = match side {
        Side::Right => a.x + a.w,
        Side::Left => a.x,
        Side::Bottom => a.y + a.h,
        Side::Top => a.y,
    };
    let (start, end, across) = if along_y {
        (from * view.scale + view.top, to * view.scale + view.top, edge * view.scale + view.left)
    } else {
        (from * view.scale + view.left, to * view.scale + view.left, edge * view.scale + view.top)
    };
    let half = SEAM_WIDTH / 2.;
    let length = px((end - start) as f32);
    let placed = if along_y {
        bar.left(px(across as f32 - half)).top(px(start as f32)).w(px(SEAM_WIDTH)).h(length)
    } else {
        bar.top(px(across as f32 - half)).left(px(start as f32)).h(px(SEAM_WIDTH)).w(length)
    };
    Some(if lined_up { placed.bg(color) } else { placed.border_1().border_dashed().border_color(color) })
}

fn map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    let view = fit(&panel.layout, width);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.page).rounded(px(8.));
    for s in &panel.layout {
        surface = surface.child(screen_tile(panel, s, &view, p, cx));
    }
    for b in &state.borders {
        let find = |number: u32| panel.layout.iter().find(|s| s.index + 1 == number as usize);
        if let (Some(a), Some(other)) = (find(b.a), find(b.b)) {
            let lined_up = b.state == BorderState::LinedUp;
            surface = surface.children(seam_bar(a, other, b.side, lined_up, &view, p));
        }
    }
    surface.child(
        div()
            .absolute()
            .left(px(16.))
            .bottom(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(11.))
            .text_color(p.muted)
            .child(div().w(px((SCALE_BAR_MM * view.scale) as f32)).h(px(1.)).bg(p.muted))
            .child("20 cm"),
    )
}

fn border_rows(panel: &Panel, state: &State, p: Palette) -> Vec<AnyElement> {
    let edge = |side: Side| match side {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Top => "Top",
        Side::Bottom => "Bottom",
    };
    let lined_up_when = panel.config.desks.get(&state.desk.sig).and_then(|d| d.lined_up_at).and_then(relative);
    let rows = state.borders.iter().map(|b| {
        let (a, other) = (b.a as usize, b.b as usize);
        let link = panel.links.iter().find(|l| (l.a + 1, l.b + 1) == (a, other));
        let no_size = [a, other].into_iter().find(|n| panel.mons.get(n - 1).is_some_and(|m| m.raw_dpi <= 0.));
        let detail = match (link, no_size) {
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
                    .child(ui::description(format!("{} edge of {a}", edge(b.side)), p)),
            )
            .child(ui::tabular(ui::description(detail, p)).flex_1().min_w_0())
            .child(status)
            .into_any_element()
    });
    rows.collect()
}

/// "today", "3 days ago" or "12 Oct", by the local calendar.
fn relative(unix: u64) -> Option<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(relative_day(winutil::local_date(now)?, winutil::local_date(unix)?))
}

fn forget(panel: &mut Panel, sig: String, desk: Desk, window: &mut Window, cx: &mut Context<Panel>) {
    struct Undo;
    let name = desk.name.clone();
    panel.patch(json!({ "desks": { sig.clone(): null } }), window, cx);
    let view = cx.entity().downgrade();
    let note = Notification::new().id::<Undo>().message(format!("Forgot {name}")).action(move |_, _, cx| {
        let (view, sig, desk) = (view.clone(), sig.clone(), desk.clone());
        ui::primary("undo", "Undo").on_click(cx.listener(move |note: &mut Notification, _, window, cx| {
            let patch = json!({ "desks": { sig.clone(): desk } });
            let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
            note.dismiss(window, cx);
        }))
    });
    window.push_notification(note, cx);
    panel.toast_generation += 1;
    let generation = panel.toast_generation;
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(UNDO_FOR).await;
        let _ = this.update_in(cx, |panel, window, cx| {
            if panel.toast_generation == generation {
                window.remove_notification::<Undo>(cx);
            }
        });
    })
    .detach();
}

fn rename(sig: String, current: String, window: &mut Window, cx: &mut Context<Panel>) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Desk name").default_value(current));
    let view = cx.entity().downgrade();
    let first = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        let (name, view, sig) = (input.clone(), view.clone(), sig.clone());
        dialog
            .title("Rename desk")
            .child(Input::new(&input))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Cancel")))
                    .child(DialogAction::new().child(ui::primary("rename-ok", "Rename"))),
            )
            .on_ok(move |_, window, cx| {
                let text = name.read(cx).value();
                if text.trim().is_empty() {
                    return false;
                }
                let patch = json!({ "desks": { sig.clone(): { "name": text.trim() } } });
                let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
                true
            })
    });
}

fn desk_rows(panel: &Panel, window: &Window, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let p = palette(cx);
    let _ = window;
    let active = panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default();
    let mut desks: Vec<(String, Desk)> = panel.config.desks.iter().map(|(s, d)| (s.clone(), d.clone())).collect();
    desks.sort_by_key(|(sig, d)| (*sig != active, std::cmp::Reverse(d.last_seen)));
    let rows = desks.into_iter().enumerate().map(|(i, (sig, desk))| {
        let in_use = sig == active;
        let screens = sig.split('|').count();
        let count = format!("{screens} screen{}", if screens == 1 { "" } else { "s" });
        let about = match (in_use, desk.last_seen) {
            (true, _) if !desk.borders.is_empty() => match desk.lined_up_at.and_then(relative) {
                Some(when) => format!("{count}, lined up {when}"),
                None => format!("{count}, lined up"),
            },
            (true, _) | (false, 0) => count,
            (false, seen) => match winutil::local_date(seen) {
                Some(date) => format!("{count}, last connected {}", month_day(date)),
                None => count,
            },
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(14.))
            .child(desk.name.clone())
            .when(in_use, |d| d.child(div().text_size(px(12.)).text_color(p.muted).child("in use")));
        let view = cx.entity().downgrade();
        let more = ui::secondary(("more", i), "", cx)
            .ghost()
            .icon(Icon::new(assets::IconName::Ellipsis))
            .dropdown_menu(move |menu, _, _| {
                let (rename_sig, name) = (sig.clone(), desk.name.clone());
                let (forget_sig, forget_desk) = (sig.clone(), desk.clone());
                let (first, second) = (view.clone(), view.clone());
                menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                    let (sig, name) = (rename_sig.clone(), name.clone());
                    let _ = first.update(cx, |_, cx| rename(sig, name, window, cx));
                }))
                .item(PopupMenuItem::new("Forget").on_click(move |_, window, cx| {
                    let (sig, desk) = (forget_sig.clone(), forget_desk.clone());
                    let _ = second.update(cx, |panel, cx| forget(panel, sig, desk, window, cx));
                }))
            });
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .min_h(px(56.))
            .px(px(16.))
            .py(px(8.))
            .child(div().flex().flex_col().gap(px(2.)).flex_1().child(title).child(ui::description(about, p)))
            .child(more)
            .into_any_element()
    });
    rows.collect()
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

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let screens = panel.mons.len();
    let subtitle = format!("{screens} screen{} on {}", if screens == 1 { "" } else { "s" }, panel.desk_name());
    let one_screen = screens < 2;

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

    let width = f64::from(window.viewport_size().width) - 240. - 64. - 34.;
    let mut content = vec![];
    if let Some(state) = panel.state.clone() {
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(div().text_size(px(16.)).child("Physical layout"))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child("Real sizes from each screen's EDID. Click a screen to identify it"),
            );
        let body = if one_screen {
            div()
                .text_size(px(13.))
                .text_color(p.secondary)
                .child("Only one screen is connected, so there's nothing to line up")
        } else {
            map(panel, &state, width, p, cx)
        };
        let top = div().flex().flex_col().gap(px(12.)).p(px(16.)).child(title).child(body).into_any_element();
        let mut rows = vec![top];
        rows.extend(border_rows(panel, &state, p));
        content.push(card(rows, p).into_any_element());
    }

    let view = cx.entity();
    let remap = switch("remap", panel.config.enabled, window, cx, {
        let view = view.clone();
        move |on, window, cx| view.update(cx, |panel, cx| panel.patch(json!({ "enabled": on }), window, cx))
    });
    let also_align = switch("align-arrangement", panel.config.align_arrangement, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| panel.patch(json!({ "align_arrangement": on }), window, cx));
    });
    let pointer = card(
        vec![
            row("Remap pointer at borders", Some("Cross where the screens physically meet".into()), remap, p),
            row(
                "Also align Windows arrangement",
                Some("Move screens in Display settings to match".into()),
                also_align,
                p,
            ),
        ],
        p,
    );
    let mut rows = desk_rows(panel, window, cx);
    if rows.is_empty() {
        rows.push(ui::row("No saved desks yet", Some("Line up your screens to save this one".into()), div(), p));
    }
    let desks = card(rows, p);

    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(
            ui::page_header("Displays", subtitle, p)
                .child(div().flex().flex_none().gap(px(8.)).child(align).child(line_up)),
        )
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
