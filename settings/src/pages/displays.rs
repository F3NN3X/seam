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

struct View {
    scale: f64,
    left: f64,
    top: f64,
}

/// A screen's rectangle on the map: x, y, width, height.
type Frame = (f64, f64, f64, f64);

/// Fit the screens into the map, keeping real proportions.
fn fit(frames: &[Frame], width: f64) -> View {
    let min_x = frames.iter().map(|f| f.0).fold(f64::MAX, f64::min);
    let min_y = frames.iter().map(|f| f.1).fold(f64::MAX, f64::min);
    let max_x = frames.iter().map(|f| f.0 + f.2).fold(f64::MIN, f64::max);
    let max_y = frames.iter().map(|f| f.1 + f.3).fold(f64::MIN, f64::max);
    let room = (MAP_PADDING * 2.) as f64;
    let (span_x, span_y) = ((max_x - min_x).max(1.), (max_y - min_y).max(1.));
    let scale = ((width - room) / span_x).min((MAP_HEIGHT as f64 - room) / span_y);
    View {
        scale,
        left: (width - span_x * scale) / 2. - min_x * scale,
        top: (MAP_HEIGHT as f64 - span_y * scale) / 2. - min_y * scale,
    }
}

fn screen_tile(
    panel: &Panel,
    index: usize,
    (x, y, w, h): Frame,
    sized: bool,
    view: &View,
    p: Palette,
    cx: &mut Context<Panel>,
) -> impl IntoElement + use<> {
    let number = index + 1;
    let name = panel.mons.get(index).map_or("", |m| m.name.as_str()).to_string();
    let primary = panel.mons.get(index).is_some_and(|m| m.primary);
    let label = if primary { format!("{name}, main") } else { name };
    let height = h * view.scale;
    div()
        .id(("screen", index))
        .absolute()
        .left(px((x * view.scale + view.left) as f32))
        .top(px((y * view.scale + view.top) as f32))
        .w(px((w * view.scale) as f32))
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
        .when(!sized, |d| d.border_dashed())
        .rounded(px(4.))
        .cursor_pointer()
        .child(div().text_size(px(16.)).child(number.to_string()))
        .child(div().text_size(px(11.)).text_color(p.secondary).truncate().child(label))
        .on_click(cx.listener(move |panel, _, window, cx| {
            panel.command("screen.identify", json!({ "number": number }), window, cx);
        }))
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    Solid,
    Dashed,
    Dotted,
}

/// Where a line runs on the map, in map pixels.
struct Run {
    along_y: bool,
    start: f64,
    end: f64,
    across: f64,
}

/// The stretch `from` to `to` of `a`'s edge on `side`, in the map's units.
fn edge_run(a: &Placed, side: Side, from: f64, to: f64, view: &View) -> Option<Run> {
    if to <= from {
        return None;
    }
    let along_y = side.along_is_y();
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
    Some(Run { along_y, start, end, across })
}

/// The part of the edge two screens physically share.
fn seam_run(a: &Placed, b: &Placed, side: Side, view: &View) -> Option<Run> {
    let (from, to) = if side.along_is_y() {
        (a.y.max(b.y), (a.y + a.h).min(b.y + b.h))
    } else {
        (a.x.max(b.x), (a.x + a.w).min(b.x + b.w))
    };
    edge_run(a, side, from, to, view)
}

/// Where Windows has two screens touching, drawn along the first one's edge. For a
/// border that isn't on the desk, where the screens don't sit together.
fn windows_run(panel: &Panel, a: &Placed, b: &Placed, side: Side, view: &View) -> Option<Run> {
    let (ra, rb) = (&panel.mons.get(a.index)?.rect, &panel.mons.get(b.index)?.rect);
    let (from, to, own, mm, size) = if side.along_is_y() {
        (ra.top.max(rb.top), ra.bottom.min(rb.bottom), ra.top, a.y, a.h / f64::from(ra.height()))
    } else {
        (ra.left.max(rb.left), ra.right.min(rb.right), ra.left, a.x, a.w / f64::from(ra.width()))
    };
    edge_run(a, side, mm + f64::from(from - own) * size, mm + f64::from(to - own) * size, view)
}

/// A line along one axis, solid or in dashes or dots.
fn stroke(run: &Run, width: f32, style: Line, color: Hsla) -> Vec<Div> {
    let (on, off) = match style {
        Line::Solid => (run.end - run.start, 0.),
        Line::Dashed => (8., 5.),
        Line::Dotted => (2., 4.),
    };
    let half = width / 2.;
    let mut parts = vec![];
    let mut at = run.start;
    while at < run.end {
        let length = px(on.min(run.end - at) as f32);
        let part = div().absolute().bg(color);
        parts.push(if run.along_y {
            part.left(px(run.across as f32 - half)).top(px(at as f32)).w(px(width)).h(length)
        } else {
            part.top(px(run.across as f32 - half)).left(px(at as f32)).h(px(width)).w(length)
        });
        at += on + off;
    }
    parts
}

/// A small note centred on a point of the map.
fn note_at(x: f64, y: f64, text: String, p: Palette) -> Div {
    const WIDTH: f32 = 120.;
    const HEIGHT: f32 = 16.;
    div()
        .absolute()
        .left(px(x as f32 - WIDTH / 2.))
        .top(px(y as f32 - HEIGHT / 2.))
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .child(div().px(px(4.)).bg(p.page).rounded(px(4.)).text_size(px(11.)).text_color(p.muted).child(text))
}

/// The line, and its note if it fits, for one border on the Seam view.
fn seam_border(panel: &Panel, a: &Placed, other: &Placed, b: &Border, view: &View, p: Palette) -> Vec<Div> {
    let lined_up = b.state == BorderState::LinedUp;
    let (style, color) = match (b.not_on_desk, b.glued, lined_up) {
        (true, ..) => (Line::Dotted, p.muted),
        (_, true, _) => (Line::Dashed, p.accent),
        (_, _, true) => (Line::Solid, p.accent),
        _ => (Line::Dashed, p.muted),
    };
    let run = if b.not_on_desk { windows_run(panel, a, other, b.side, view) } else { seam_run(a, other, b.side, view) };
    let Some(run) = run else { return vec![] };
    let mut parts = stroke(&run, SEAM_WIDTH, style, color);
    if b.not_on_desk && run.end - run.start >= NOTE_FITS {
        let mid = (run.start + run.end) / 2.;
        let (x, y) = if run.along_y { (run.across, mid) } else { (mid, run.across) };
        parts.push(note_at(x, y, "Not on your desk".into(), p));
    }
    parts
}

/// Where each screen sits on the desk, joined where you lined them up.
fn seam_map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    let frames: Vec<Frame> = panel.layout.iter().map(|s| (s.x, s.y, s.w, s.h)).collect();
    let view = fit(&frames, width);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.page).rounded(px(8.));
    for (s, frame) in panel.layout.iter().zip(&frames) {
        surface = surface.child(screen_tile(panel, s.index, *frame, s.sized, &view, p, cx));
    }
    for b in &state.borders {
        let find = |number: u32| panel.layout.iter().find(|s| s.index + 1 == number as usize);
        if let (Some(a), Some(other)) = (find(b.a), find(b.b)) {
            surface = surface.children(seam_border(panel, a, other, b, &view, p));
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

/// What the Windows view draws between two screens: a line where they touch, or
/// a note where they face each other across a gap or overlap.
enum Between {
    Touch { side: Side, from: i32, to: i32, edge: i32 },
    Note { x: i32, y: i32, text: String },
}

fn windows_between(rects: &[Rect]) -> Vec<Between> {
    let mut found = vec![];
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            if let Some(side) = touching(a, b) {
                let (from, to) = if side.along_is_y() {
                    (a.top.max(b.top), a.bottom.min(b.bottom))
                } else {
                    (a.left.max(b.left), a.right.min(b.right))
                };
                let edge = match side {
                    Side::Right => a.right,
                    Side::Left => a.left,
                    Side::Bottom => a.bottom,
                    Side::Top => a.top,
                };
                found.push(Between::Touch { side, from, to, edge });
            } else if let Some((side, gap)) = facing_gap(a, b) {
                let c = corridor(a, b, side);
                let text = format!("{gap} px gap");
                found.push(Between::Note { x: (c.left + c.right) / 2, y: (c.top + c.bottom) / 2, text });
            } else if a.intersects(b) {
                let (left, right) = (a.left.max(b.left), a.right.min(b.right));
                let (top, bottom) = (a.top.max(b.top), a.bottom.min(b.bottom));
                let overlap = (right - left).min(bottom - top);
                if (1..=LABEL_PX).contains(&overlap) {
                    let text = format!("{overlap} px overlap");
                    found.push(Between::Note { x: (left + right) / 2, y: (top + bottom) / 2, text });
                }
            }
        }
    }
    found
}

/// The screens as Windows arranges them, in pixels.
fn windows_map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    let frames: Vec<Frame> = state
        .screens
        .iter()
        .map(|s| (f64::from(s.rect.x), f64::from(s.rect.y), f64::from(s.rect.w), f64::from(s.rect.h)))
        .collect();
    let rects: Vec<Rect> = state.screens.iter().map(|s| Rect::new(s.rect.x, s.rect.y, s.rect.w, s.rect.h)).collect();
    let view = fit(&frames, width);
    let (x_at, y_at) = (|v: i32| f64::from(v) * view.scale + view.left, |v: i32| f64::from(v) * view.scale + view.top);
    let mut surface = div().relative().w_full().h(px(MAP_HEIGHT)).bg(p.page).rounded(px(8.));
    for (s, frame) in state.screens.iter().zip(&frames) {
        surface = surface.child(screen_tile(panel, (s.number as usize).saturating_sub(1), *frame, true, &view, p, cx));
    }
    for between in windows_between(&rects) {
        match between {
            Between::Touch { side, from, to, edge } => {
                let along_y = side.along_is_y();
                let run = if along_y {
                    Run { along_y, start: y_at(from), end: y_at(to), across: x_at(edge) }
                } else {
                    Run { along_y, start: x_at(from), end: x_at(to), across: y_at(edge) }
                };
                surface = surface.children(stroke(&run, 2., Line::Solid, p.secondary));
            }
            Between::Note { x, y, text } => surface = surface.child(note_at(x_at(x), y_at(y), text, p)),
        }
    }
    surface
}

fn map(panel: &Panel, state: &State, width: f64, p: Palette, cx: &mut Context<Panel>) -> Div {
    match panel.layout_view {
        LayoutView::Seam => seam_map(panel, state, width, p, cx),
        LayoutView::Windows => windows_map(panel, state, width, p, cx),
    }
}

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
            .accessibility_label(format!("More for {}", desk.name))
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
