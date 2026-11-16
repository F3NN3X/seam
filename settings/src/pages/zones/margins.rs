//! Screen margins: four sliders behind a select, for the chosen screen, sent when let go.

use super::*;
use crate::theme::Palette;
use crate::ui::Row;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use seam::margins::{MAX_PX, Margins};

/// Top, right, bottom and left: the order of `Margins::sides` and of the sliders.
const SIDES: [&str; 4] = ["Top", "Right", "Bottom", "Left"];

/// The chosen screen's margins on this desk.
fn current(panel: &Panel) -> Margins {
    let sig = panel.state.as_ref().map(|s| s.desk.sig.as_str());
    let Some((mon, _)) = chosen(panel) else { return Margins::default() };
    sig.and_then(|sig| panel.config.desks.get(sig)).and_then(|d| d.margins.get(&mon.id)).copied().unwrap_or_default()
}

/// The desk, the screen and the margins of the chosen screen, to put them back.
pub fn saved(panel: &Panel) -> Option<(String, String, Margins)> {
    let sig = panel.state.as_ref().map(|s| s.desk.sig.clone())?;
    let id = chosen(panel).map(|(mon, _)| mon.id.clone())?;
    Some((sig, id, current(panel)))
}

/// Side `side` is now `px`: shown at once while the slider moves, sent when it is let go.
fn changed(panel: &mut Panel, side: usize, px: u32, release: bool, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some(id) = chosen(panel).map(|(mon, _)| mon.id.clone()) else { return };
    let next = current(panel).with(side, px);
    if release {
        panel.patch(json!({ "desks": { sig: { "margins": { id: next } } } }), window, cx);
    } else {
        panel.config_mut().desks.entry(sig).or_default().margins.insert(id, next);
        cx.notify();
    }
}

pub fn subscriptions(
    sliders: &[Entity<SliderState>; 4],
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<Subscription> {
    let one = |side: usize, slider: &Entity<SliderState>, window: &mut Window, cx: &mut Context<Panel>| {
        cx.subscribe_in(slider, window, move |this, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(value) => changed(this, side, value.end() as u32, false, window, cx),
            SliderEvent::Release(value) => changed(this, side, value.end() as u32, true, window, cx),
        })
    };
    sliders.iter().enumerate().map(|(side, slider)| one(side, slider, window, cx)).collect()
}

/// What the select shows: one number when every side has it.
fn summary(now: Margins) -> String {
    let [t, r, b, l] = now.sides();
    if t == r && r == b && b == l {
        format!("{t} px on every side")
    } else {
        format!("Top {t}, right {r}, bottom {b}, left {l} px")
    }
}

/// The "Screen margins" row for the chosen screen: a select that opens a slider for each side.
pub fn row(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let now = current(panel);
    // The sliders follow the setting when something else changed it, or another screen was chosen.
    for (slider, px) in panel.margins.iter().zip(now.sides()) {
        let px = px.min(MAX_PX) as f32;
        if slider.read(cx).value().end() != px {
            slider.update(cx, |slider, cx| slider.set_value(px, window, cx));
        }
    }
    let sliders = panel.margins.clone();
    let sides = move |_: &mut PopoverState, _: &mut Window, cx: &mut Context<PopoverState>| {
        let mut column = div().flex().flex_col().gap(px(tokens::SPACE_8));
        for (side, slider) in SIDES.iter().zip(&sliders) {
            let value = format!("{} px", slider.read(cx).value().end() as u32);
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(tokens::SPACE_12))
                    .child(text_style(div(), tokens::LABEL).w(px(48.)).child(*side))
                    .child(ui::slider_with_value(slider, value, true, side, p)),
            );
        }
        column
    };
    let select = Popover::new("screen-margins")
        .trigger(ui::secondary("screen-margins-select", summary(now), cx).dropdown_caret(true))
        .content(sides);
    ui::row("Screen margins", Some("Room kept free around the zones on this screen".into()), select, p)
}
