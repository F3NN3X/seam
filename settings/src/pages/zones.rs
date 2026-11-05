//! Zones: a layout for each screen, and how dragging a window snaps to it.

use crate::pages::zone_editor;
use crate::panel::Panel;
use crate::theme::{palette, Palette};
use crate::ui::{self, card, row, section, switch};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::ThemeStyled;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::desk::Mon;
use seam::zones::{self, Dir, LeafTag, Node, Rect, Template, ZoneLayout};
use serde_json::{json, Value};
use std::rc::Rc;

/// Config value and label of each key a drag can use.
const KEYS: [(&str, &str); 3] = [("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];
const SPAN_ORDER: [usize; 3] = [1, 0, 2];
const TILE_WIDTH: f32 = 96.;
const TILE_GAP: f32 = 8.;
const TILE_ART: (f32, f32) = (80., 44.);
/// Three tile rows: the preview card is as tall.
const PREVIEW_HEIGHT: f32 = 268.;

fn key_label(key: &str) -> &'static str {
    KEYS.iter().find(|(k, _)| *k == key).map_or("Shift", |(_, label)| label)
}

/// The tree a tile draws. Custom has none of its own, so it shows a sample.
fn art_tree(template: Template, own: Option<&ZoneLayout>) -> Option<Node> {
    let leaf = || Box::new(Node::Leaf(LeafTag::Leaf));
    match template {
        Template::None => None,
        Template::Custom => own.map(|l| l.tree.clone()).or(Some(Node::Split {
            split: Dir::Vertical,
            ratio: 0.6,
            a: leaf(),
            b: Box::new(Node::Split { split: Dir::Horizontal, ratio: 0.5, a: leaf(), b: leaf() }),
        })),
        t => zones::template_tree(t),
    }
}

fn pieces(tree: &Node, width: f32, height: f32, gap: i32) -> Vec<Rect> {
    zones::rects(tree, Rect { x: 0, y: 0, w: width as i32, h: height as i32 }, gap)
}

fn tile(
    i: usize,
    template: Template,
    layout: Option<&ZoneLayout>,
    selected: bool,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    let id = ElementId::Name(format!("zone-tile-{}", template.title()).into());
    let focus = window.use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle()).read(cx).clone();
    let mut art = div().relative().w(px(TILE_ART.0)).h(px(TILE_ART.1));
    match art_tree(template, layout.filter(|l| l.template == template)) {
        Some(tree) => {
            for r in pieces(&tree, TILE_ART.0, TILE_ART.1, 2) {
                art = art.child(
                    div()
                        .absolute()
                        .left(px(r.x as f32))
                        .top(px(r.y as f32))
                        .w(px(r.w as f32))
                        .h(px(r.h as f32))
                        .rounded(px(2.))
                        .bg(p.segment),
                );
            }
        }
        None => art = art.child(div().size_full().rounded(px(4.)).bg(p.page)),
    }
    // Custom is made in the editor, so its tile opens it.
    let pick = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        if template == Template::Custom {
            return zone_editor::open(panel, window, cx);
        }
        let then: zone_editor::Then = Rc::new(move |panel, window, cx| set_template(panel, template, window, cx));
        zone_editor::guard(panel, window, cx, then);
    };
    let face = div()
        .id(id)
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(8.))
        .w(px(TILE_WIDTH))
        .rounded(px(10.))
        .bg(p.card)
        .border_1()
        .border_color(if selected { p.accent } else { p.hairline })
        .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .child(art)
        .child(
            div().text_size(px(13.)).text_color(if selected { p.text } else { p.secondary }).child(template.title()),
        );
    let face = face
        .cursor_pointer()
        .on_click(cx.listener(move |panel, _, window, cx| pick(panel, window, cx)))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                pick(panel, window, cx);
            }
        }));
    // A second, fainter ring around the chosen tile.
    div()
        .id(("zone-halo", i))
        .p(px(1.))
        .rounded(px(12.))
        .border_1()
        .border_color(if selected { p.accent.opacity(0.4) } else { hsla(0., 0., 0., 0.) })
        .child(face)
        .into_any_element()
}

/// The screen the page is showing, with the layout saved for it on this desk.
pub fn chosen(panel: &Panel) -> Option<(&Mon, Option<ZoneLayout>)> {
    let mon = panel.mons.get(panel.zone_screen.min(panel.mons.len().checked_sub(1)?))?;
    let sig = panel.state.as_ref().map(|s| s.desk.sig.as_str())?;
    let saved = panel.config.desks.get(sig).and_then(|d| d.zones.get(&mon.id)).and_then(zones::layout_of);
    Some((mon, saved))
}

fn set_template(panel: &mut Panel, template: Template, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some((mon, _)) = chosen(panel) else { return };
    // No entry means no zones, and a patch's null removes the entry.
    let layout = ZoneLayout::of(template).and_then(|l| serde_json::to_value(l).ok()).unwrap_or(Value::Null);
    let patch = json!({ "desks": { sig: { "zones": { mon.id.clone(): layout } } } });
    panel.patch(patch, window, cx);
}

fn preview(mon: &Mon, layout: Option<&ZoneLayout>, gap: u32, room: f64, p: Palette) -> Div {
    let card = div()
        .flex()
        .items_center()
        .justify_center()
        .flex_1()
        .min_w_0()
        .h(px(PREVIEW_HEIGHT))
        .p(px(16.))
        .bg(p.card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(12.));
    let (w, h) = (mon.rect.width() as f64, mon.rect.height() as f64);
    let scale = ((room - 34.) / w).min((PREVIEW_HEIGHT as f64 - 34.) / h).max(0.01);
    let mut surface = div().relative().w(px((w * scale) as f32)).h(px((h * scale) as f32));
    let Some(layout) = layout else {
        let empty = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_dashed()
            .border_color(p.hairline_strong)
            .rounded(px(8.))
            .child(ui::description("No zones on this screen. Pick a layout", p));
        return card.child(surface.child(empty));
    };
    let area = Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
    let (cells, drawn) = (zones::rects(&layout.tree, area, 0), zones::rects(&layout.tree, area, gap as i32));
    for (i, (cell, r)) in cells.iter().zip(&drawn).enumerate() {
        let at = |v: i32| px((v as f64 * scale) as f32);
        surface = surface.child(
            div()
                .id(("zone", i))
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
                .bg(p.raised)
                .border_1()
                .border_color(p.hairline_strong)
                .rounded(px(6.))
                .hover(|d| d.bg(p.accent.opacity(0.1)).border_color(p.accent))
                .child(div().text_size(px(24.)).child((i + 1).to_string()))
                .child(ui::tabular(div().text_size(px(13.)).text_color(p.secondary).child(format!("{} px", cell.w)))),
        );
    }
    card.child(surface)
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let zones_cfg = panel.config.zones.clone();
    let subtitle = format!("Hold {} while you drag a window to drop it into a zone", key_label(&zones_cfg.show_key));
    let view = cx.entity();

    let on = switch("zones-on", zones_cfg.enabled, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "enabled": on } }), window, cx))
        }
    });
    let header = ui::page_header("Zones", subtitle, p).child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .child(div().text_size(px(14.)).text_color(p.secondary).child(if zones_cfg.enabled {
                "Zones on"
            } else {
                "Zones off"
            }))
            .child(on),
    );

    // The slider follows the setting when something else changed it.
    let gap = zones_cfg.gap.min(32) as f32;
    if panel.gap.read(cx).value().end() != gap {
        panel.gap.update(cx, |slider, cx| slider.set_value(gap, window, cx));
    }

    let show_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.show_key).unwrap_or(0);
    let span_at = KEYS.iter().position(|(k, _)| *k == zones_cfg.span_key).unwrap_or(1);
    let show = ui::segmented("zone-show-key", &["Shift", "Ctrl", "Alt"], show_at, window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            // The span key can't be the same key, so it moves to the next one.
            let mut zones = json!({ "show_key": KEYS[i].0 });
            if KEYS[i].0 == KEYS[span_at].0 {
                zones["span_key"] = json!(KEYS[(i + 1) % KEYS.len()].0);
            }
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": zones }), window, cx));
        }
    });
    let span_labels = SPAN_ORDER.iter().map(|i| SharedString::from(KEYS[*i].1)).collect();
    let span_blocked = SPAN_ORDER.iter().position(|i| *i == show_at).map(|at| {
        let why = [
            "Ctrl is already used to show zones",
            "Shift is already used to show zones",
            "Alt is already used to show zones",
        ];
        (at, why[SPAN_ORDER[at]])
    });
    let span_selected = SPAN_ORDER.iter().position(|i| *i == span_at).unwrap_or(0);
    let span = ui::choice("zone-span-key", span_labels, span_blocked, span_selected, window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            let key = KEYS[SPAN_ORDER[i]].0;
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "span_key": key } }), window, cx));
        }
    });
    let ends = |text: &'static str| div().text_size(px(12.)).text_color(p.muted).child(text);
    let slider = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ends("0"))
        .child(Slider::new(&panel.gap).horizontal().w(px(160.)))
        .child(ends("32"));
    let restore = switch("zone-restore", zones_cfg.restore_size, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "restore_size": on } }), window, cx))
        }
    });
    let refit = switch("zone-refit", zones_cfg.refit, window, cx, {
        let view = view.clone();
        move |on, window, cx| view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "refit": on } }), window, cx))
    });
    let on_screen = switch("zone-on-screen", zones_cfg.edit_on_screen, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "zones": { "edit_on_screen": on } }), window, cx))
        }
    });
    let on_screen_row = row(
        "Edit layouts on the screen",
        Some("Off shows a live view on the screen, and you edit in the preview here".into()),
        on_screen,
        p,
    );
    let refit_row = row(
        "Keep windows in their zones",
        Some("Experimental. Moves snapped windows back after a screen or resolution change".into()),
        refit,
        p,
    );
    let cell = |content: AnyElement| div().flex_1().min_w_0().child(content);
    let snapping = div()
        .flex()
        .flex_col()
        .bg(p.card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(12.))
        .child(
            div().flex().child(cell(row("Show zones", Some("Hold while dragging".into()), show, p))).child(
                cell(row("Span zones", Some("Add this key to cover more".into()), span, p))
                    .border_l_1()
                    .border_color(p.hairline),
            ),
        )
        .child(
            div()
                .flex()
                .border_t_1()
                .border_color(p.hairline)
                .child(cell(row("Gap", Some(format!("{} px", zones_cfg.gap.min(32)).into()), slider, p)))
                .child(
                    cell(row(
                        "Restore size when dragged out",
                        Some("The window gets its old size back".into()),
                        restore,
                        p,
                    ))
                    .border_l_1()
                    .border_color(p.hairline),
                ),
        );

    let mut body = div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header);
    let Some((mon, saved)) = chosen(panel).map(|(m, l)| (m.clone(), l)) else {
        let note = ui::description("Start Seam to set up zones", p);
        return body.child(note);
    };
    // The desk or screen changed under the editor, so its tree is for something else.
    let sig = panel.state.as_ref().map(|s| s.desk.sig.clone());
    if panel.editor.as_ref().is_some_and(|e| Some(&e.sig) != sig.as_ref() || e.screen != mon.id) {
        panel.editor = None;
    }
    let selected = saved.as_ref().map_or(Template::None, |l| l.template);
    let labels: Vec<SharedString> =
        panel.mons.iter().enumerate().map(|(i, m)| format!("{}  {}", i + 1, m.name).into()).collect();
    let tabs = ui::choice("zone-screen", labels, None, panel.zone_screen.min(panel.mons.len() - 1), window, cx, {
        let view = view.clone();
        move |i, window, cx| {
            view.update(cx, |panel, cx| {
                // An editor that's open carries on with the screen picked.
                let editing = panel.editor.is_some();
                let then: zone_editor::Then = Rc::new(move |panel, window, cx| {
                    panel.zone_screen = i;
                    if editing {
                        zone_editor::open(panel, window, cx);
                    }
                    cx.notify();
                });
                zone_editor::guard(panel, window, cx, then);
            })
        }
    });

    let mut tiles =
        div().flex().flex_wrap().content_start().w(px(TILE_WIDTH * 2. + TILE_GAP * 2. + 4.)).gap(px(TILE_GAP));
    for (i, template) in Template::ALL.into_iter().enumerate() {
        tiles = tiles.child(tile(i, template, saved.as_ref(), template == selected, window, cx));
    }
    let room = f64::from(window.viewport_size().width) - 240. - 64. - TILE_WIDTH as f64 * 2. - 24. - 20.;
    let caption = ui::description("Stored as parts of the screen, so a new resolution keeps the layout", p);
    let edit_layout = ui::secondary("edit-layout", "Edit layout", cx)
        .on_click(cx.listener(|panel, _, window, cx| zone_editor::open(panel, window, cx)));
    let below = div().flex().items_center().justify_between().gap(px(16.)).child(caption).child(edit_layout);
    let gap_px = zones_cfg.gap.min(32);
    let stage = if panel.editor.is_some() {
        zone_editor::view(panel, &mon, gap_px, room, PREVIEW_HEIGHT, window, cx)
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(preview(&mon, saved.as_ref(), gap_px, room, p))
            .child(below)
            .into_any_element()
    };

    body = body
        .child(div().flex().child(tabs))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(24.))
                .child(tiles)
                .child(div().flex().flex_col().flex_1().min_w_0().child(stage)),
        )
        .child(section(
            "Snapping",
            div().flex().flex_col().gap(px(12.)).child(snapping).child(card(vec![on_screen_row, refit_row], p)),
            p,
        ));
    body
}
