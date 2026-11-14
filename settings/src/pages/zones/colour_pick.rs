//! The border colour swatches and the custom colour picker.

use super::*;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerState};
use gpui_kit::component::tooltip::Tooltip;
use std::time::Duration;

/// One swatch in a row: "none" first, the named colours, then the custom one.
enum Choice {
    None,
    Named(&'static colours::Swatch),
    Custom,
}

impl Choice {
    fn all() -> impl Iterator<Item = Choice> {
        [Choice::None].into_iter().chain(colours::SWATCHES.iter().map(Choice::Named)).chain([Choice::Custom])
    }

    fn key(&self) -> &'static str {
        match self {
            Choice::None => colours::NONE,
            Choice::Named(s) => s.name,
            Choice::Custom => "custom",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Choice::None => "None",
            Choice::Named(s) => s.title,
            Choice::Custom => "Custom colour",
        }
    }

    fn selected(&self, chosen: &str) -> bool {
        match self {
            Choice::Custom => chosen.starts_with('#'),
            other => other.key() == chosen,
        }
    }

    /// What the swatch is filled with. Windows' accent is read here, and None means
    /// there is no colour to show: a neutral swatch with a glyph instead.
    fn fill(&self, chosen: &str) -> Option<Hsla> {
        let hex = match self {
            Choice::None => return None,
            Choice::Named(s) => match s.rgb {
                Some(hex) => hex,
                None => colours::colorref(colours::accent_colorref()?),
            },
            Choice::Custom => colours::parse_hex(chosen)?,
        };
        Some(rgb(hex).into())
    }

    /// Shown on a neutral swatch that isn't chosen.
    fn glyph(&self) -> &'static str {
        match self {
            Choice::None => "/",
            Choice::Named(_) => "A",
            Choice::Custom => "+",
        }
    }
}

/// A row of round swatches for one `windows` colour setting. A swatch is one
/// tab stop, and Enter or Space picks it. Off, the row shows `why_off` and does nothing.
/// The custom swatch opens the colour picker instead of picking.
pub(super) fn swatches(
    field: &'static str,
    picker: &Entity<ColorPickerState>,
    current: &str,
    default: &str,
    why_off: Option<&'static str>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
    let p = palette(cx);
    let chosen = colours::normalise(current, default);
    seed_picker(picker, &chosen, window, cx);
    let mut group = div().flex().items_center().gap(px(4.));
    for choice in Choice::all() {
        let selected = choice.selected(&chosen);
        let id = ElementId::Name(format!("swatch-{field}-{}", choice.key()).into());
        let fill = choice.fill(&chosen);
        let dot = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded_full()
            .border_1()
            .border_color(p.hairline_strong)
            .bg(fill.unwrap_or(p.raised))
            .text_size(px(12.))
            .when(fill.is_none() && !selected, |d| d.text_color(p.secondary).child(choice.glyph()))
            .when(selected, |d| {
                // Black or white, whichever reads better on the swatch.
                let ink = fill.map_or(p.text, |f| {
                    let c = Rgba::from(f);
                    let luma = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
                    if luma > 0.5 { hsla(0., 0., 0., 1.) } else { hsla(0., 0., 1., 1.) }
                });
                d.text_color(ink).child("✓")
            });
        let tip = why_off.unwrap_or(choice.title());
        let mut face = div()
            .id(id.clone())
            .p(px(2.))
            .rounded_full()
            .child(dot)
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx));
        if why_off.is_none() {
            let custom = matches!(choice, Choice::Custom);
            let value = choice.key();
            let picker = picker.clone();
            let focus = window.use_keyed_state(id, cx, |_, cx| cx.focus_handle()).read(cx).clone();
            let pick = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                if custom {
                    picker.update(cx, |state, cx| state.set_open(true, cx));
                } else {
                    panel.patch(json!({ "windows": { field: value } }), window, cx);
                }
            };
            let pick = Rc::new(pick);
            let (click, key) = (pick.clone(), pick);
            face = face
                .track_focus(&focus)
                .tab_index(0)
                .cursor_pointer()
                .when(selected || ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
                .when(!custom, |d| d.on_click(cx.listener(move |panel, _, window, cx| click(panel, window, cx))))
                .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
                    if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                        key(panel, window, cx);
                    }
                }));
        } else {
            face = face.when(selected, |d| d.focus_ring_style(window, cx));
        }
        // The picker's own button is laid over the swatch, unseen, so the popover opens
        // from the swatch and the swatch keeps its ring, check and tooltip.
        if why_off.is_none() && matches!(choice, Choice::Custom) {
            let trigger = ColorPicker::new(picker).anchor(Anchor::TopRight).accessibility_label("Custom colour");
            face = face.relative().child(div().absolute().top_0().left_0().size_full().opacity(0.).child(trigger));
        }
        group = group.child(face);
    }
    group
}

/// A picked colour as 0xRRGGBB. Alpha is dropped.
fn hex_of(colour: Hsla) -> u32 {
    let c = Rgba::from(colour);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
}

/// The picker moved to a colour. A drag sends many, so the row shows each at once
/// and only the last one that stays for 250 ms is saved.
pub fn picked(
    field: &'static str,
    colour: Option<Hsla>,
    panel: &mut Panel,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some(colour) = colour else { return };
    let patch = json!({ "windows": { field: format!("#{:06X}", hex_of(colour)) } });
    if let Ok(config) = panel.config.patched(&patch) {
        panel.config = config;
    }
    panel.colour_generation += 1;
    let generation = panel.colour_generation;
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(Duration::from_millis(250)).await;
        let _ = this.update_in(cx, |panel, window, cx| {
            if panel.colour_generation == generation {
                panel.patch(patch, window, cx);
            }
        });
    })
    .detach();
}

/// Keep the picker on the row's colour: the custom one, else the colour the row has
/// now. It isn't touched while open, since the picker is then what moves the colour.
fn seed_picker(picker: &Entity<ColorPickerState>, chosen: &str, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(hex) = colours::parse_hex(chosen).or_else(|| match colours::colour(chosen, chosen) {
        colours::Colour::Rgb(hex) => Some(hex),
        colours::Colour::Accent => colours::accent_colorref().map(colours::colorref),
        colours::Colour::None => None,
    }) else {
        return;
    };
    let (open, held) = {
        let state = picker.read(cx);
        (state.is_open(), state.value().map(hex_of))
    };
    if !open && held != Some(hex) {
        picker.update(cx, |state, cx| state.set_value(rgb(hex), window, cx));
    }
}
