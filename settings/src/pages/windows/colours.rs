//! The border colours: a Select for each, and the custom colour picker behind its last option.

use crate::panel::Panel;
use crate::theme::tokens;
use crate::ui::{self, Choice};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerState};
use gpui_kit::*;
use seam::palette as colours;
use serde_json::{Value, json};
use std::time::Duration;

/// The width of a colour Select.
const WIDTH: f32 = 168.;
const CUSTOM: &str = "custom";

/// What a named swatch is filled with. Windows' accent is read here.
fn swatch_fill(swatch: &colours::Swatch) -> Option<Hsla> {
    let hex = match swatch.rgb {
        Some(hex) => hex,
        None => colours::colorref(colours::accent_colorref()?),
    };
    Some(rgb(hex).into())
}

/// None, the named colours, then Custom with the colour it holds.
fn options(chosen: &str) -> Vec<Choice> {
    let named = colours::SWATCHES.iter().map(|s| Choice::new(s.name, s.title).dot(swatch_fill(s)));
    let custom = colours::parse_hex(chosen).map(|hex| rgb(hex).into());
    let (none, custom) = (Choice::new(colours::NONE, "None"), Choice::new(CUSTOM, "Custom").dot(custom));
    [none].into_iter().chain(named).chain([custom]).collect()
}

/// What a colour writes. The focus border has a switch of its own in the config: None turns it
/// off, and any colour turns it on.
fn patch_for(field: &'static str, key: &str) -> Value {
    match (field, key) {
        ("focus_color", colours::NONE) => json!({ "windows": { "focus_border": false } }),
        ("focus_color", key) => json!({ "windows": { "focus_border": true, "focus_color": key } }),
        (field, key) => json!({ "windows": { field: key } }),
    }
}

/// A Select for one `windows` colour setting. `off` says the border is off whatever the colour
/// is. Custom opens the colour picker under the Select.
pub(super) fn colour_select(
    (field, title): (&'static str, &'static str),
    off: bool,
    (current, default): (&str, &str),
    picker: &Entity<ColorPickerState>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let chosen = colours::normalise(current, default);
    seed_picker(picker, &chosen, window, cx);
    let selected = match chosen.as_str() {
        _ if off || chosen == colours::NONE => colours::NONE,
        hex if hex.starts_with('#') => CUSTOM,
        name => name,
    };
    let choose = {
        let (view, picker) = (cx.entity(), picker.clone());
        move |key: &str, window: &mut Window, cx: &mut App| {
            if key != CUSTOM {
                let patch = patch_for(field, key);
                return view.update(cx, |panel, cx| panel.patch(patch, window, cx));
            }
            // The menu is still closing, so the picker opens once it has.
            let (view, picker) = (view.clone(), picker.clone());
            cx.defer(move |cx| {
                picker.update(cx, |state, cx| state.set_open(true, cx));
                view.update(cx, |_, cx| cx.notify());
            });
        }
    };
    let select = ui::select(format!("select-{field}"), title, options(&chosen), selected, WIDTH, cx, choose);
    // The picker's own button sits under the Select, unseen, so its popover opens from there.
    let trigger = picker.read(cx).is_open().then(|| {
        let picker = ColorPicker::new(picker).anchor(Anchor::TopLeft).accessibility_label(title);
        div().absolute().left_0().bottom_0().w(px(tokens::SPACE_4)).h(px(tokens::SPACE_4)).opacity(0.).child(picker)
    });
    div().relative().child(select).children(trigger).into_any_element()
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
    let patch = patch_for(field, &format!("#{:06X}", hex_of(colour)));
    if let Ok(config) = panel.config.patched(&patch) {
        panel.set_config(config);
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
