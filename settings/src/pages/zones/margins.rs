//! Screen margins: four sliders on the Zones page for the chosen screen, sent when let go.

use super::*;
use crate::theme::Palette;
use crate::ui::Row;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use seam::margins::{MAX_PX, Margins};

/// Top, right, bottom and left: the order of `Margins::sides` and of the sliders.
const SIDES: [&str; 4] = ["Top", "Right", "Bottom", "Left"];

/// The chosen screen's margins on this desk.
fn current(panel: &Panel) -> Margins {
    let sig = panel.state.as_ref().map(|s| s.desk.sig.as_str());
    let Some((mon, _)) = chosen(panel) else { return Margins::default() };
    sig.and_then(|sig| panel.config.desks.get(sig)).and_then(|d| d.margins.get(&mon.id)).copied().unwrap_or_default()
}

/// Side `side` is now `px`: shown at once while the slider moves, sent when it is let go.
fn changed(panel: &mut Panel, side: usize, px: u32, release: bool, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some(id) = chosen(panel).map(|(mon, _)| mon.id.clone()) else { return };
    let next = current(panel).with(side, px);
    if release {
        panel.patch(json!({ "desks": { sig: { "margins": { id: next } } } }), window, cx);
    } else {
        panel.config.desks.entry(sig).or_default().margins.insert(id, next);
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

/// The "Screen margins" row for the chosen screen, with its four sliders.
pub fn row(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let now = current(panel);
    // The sliders follow the setting when something else changed it, or another screen was chosen.
    for (slider, px) in panel.margins.iter().zip(now.sides()) {
        let px = px.min(MAX_PX) as f32;
        if slider.read(cx).value().end() != px {
            slider.update(cx, |slider, cx| slider.set_value(px, window, cx));
        }
    }
    let about = if now.is_zero() {
        "Room kept free around the zones on this screen".to_string()
    } else {
        let [t, r, b, l] = now.sides();
        format!("Top {t}, right {r}, bottom {b}, left {l} px")
    };
    let sliders = |range: std::ops::Range<usize>| {
        let mut column = div().flex().flex_col().gap(px(4.));
        for i in range {
            let label = div().w(px(48.)).text_size(px(12.)).text_color(p.text_muted).child(SIDES[i]);
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label)
                    .child(Slider::new(&panel.margins[i]).horizontal().w(px(120.))),
            );
        }
        column
    };
    let control = div().flex().items_center().gap(px(24.)).child(sliders(0..2)).child(sliders(2..4));
    ui::row_with("Screen margins", Some(ui::description(about, p).into_any_element()), control)
}
