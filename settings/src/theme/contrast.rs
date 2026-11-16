//! Contrast themes: the palette and the kit's theme take Windows' system colours.

use super::Palette;
use gpui_kit::component::Theme;
use gpui_kit::*;
use seam::winver::SystemColours;

/// The contrast theme that is on, which `chrome` keeps up to date. `None` when none is.
pub struct Contrast(pub Option<SystemColours>);

impl Global for Contrast {}

fn colour(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Every colour taken from the system's: grounds are Window, text is Window text,
/// accent and selection are Highlight, disabled is Gray text, and hairlines are solid Window text.
pub fn palette(c: SystemColours) -> Palette {
    let window = colour(c.window);
    let text = colour(c.window_text);
    let face = colour(c.button_face);
    let highlight = colour(c.highlight);
    let hotlight = colour(c.hotlight);
    Palette {
        surface_sidebar: window,
        surface_page: window,
        surface_card: window,
        surface_raised: window,
        surface_segment: face,
        menu_hover: highlight,
        surface_input: window,
        surface_zone: face,
        surface_card_mica: window,
        surface_input_mica: window,
        surface_track_mica: window,
        mica: false,
        contrast: true,
        hairline: text,
        hairline_strong: text,
        control_stroke: text,
        text_primary: text,
        text_secondary: text,
        text_muted: colour(c.gray_text),
        fill_inverse: text,
        fill_inverse_hover: text,
        text_on_inverse: window,
        accent: highlight,
        accent_ink: hotlight,
        text_on_accent: colour(c.highlight_text),
        accent_tint: window,
        selection: highlight,
        focus_ring: text,
        positive: text,
        negative: hotlight,
    }
}

/// The kit's own colours, for its buttons, inputs, menus and sliders.
pub fn theme(theme: &mut Theme, c: SystemColours) {
    let window = colour(c.window);
    let text = colour(c.window_text);
    let face = colour(c.button_face);
    let highlight = colour(c.highlight);
    let highlight_text = colour(c.highlight_text);
    let colors = &mut theme.colors;
    colors.background = window;
    colors.foreground = text;
    colors.border = text;
    colors.input = text;
    colors.ring = text;
    colors.caret = text;
    colors.muted = window;
    colors.muted_foreground = colour(c.gray_text);
    colors.popover = window;
    colors.popover_foreground = text;
    colors.accent = highlight;
    colors.accent_foreground = highlight_text;
    colors.primary = text;
    colors.primary_hover = text;
    colors.primary_active = text;
    colors.primary_foreground = window;
    colors.secondary = face;
    colors.secondary_hover = highlight;
    colors.secondary_active = highlight;
    colors.secondary_foreground = text;
    colors.button_primary = text;
    colors.button_primary_hover = text;
    colors.button_primary_active = text;
    colors.button_primary_foreground = window;
    colors.button_secondary = face;
    colors.button_secondary_hover = highlight;
    colors.button_secondary_active = highlight;
    colors.button_secondary_foreground = text;
    colors.selection = highlight;
    colors.slider_bar = text;
    colors.slider_thumb = text;
    colors.link = colour(c.hotlight);
    colors.scrollbar_thumb = text;
    colors.scrollbar_thumb_hover = text;
}

#[cfg(test)]
mod tests {
    use super::{SystemColours, colour, palette};

    fn aquatic() -> SystemColours {
        SystemColours {
            window: 0x202020,
            window_text: 0xFFFFFF,
            highlight: 0x1AEBFF,
            highlight_text: 0x000000,
            gray_text: 0x3FF23F,
            button_face: 0x000000,
            hotlight: 0xFFFF00,
        }
    }

    #[test]
    fn grounds_are_window_and_text_is_window_text() {
        let p = palette(aquatic());
        assert_eq!(p.surface_page, colour(0x202020));
        assert_eq!(p.surface_card, colour(0x202020));
        assert_eq!(p.text_primary, colour(0xFFFFFF));
        assert_eq!(p.hairline, colour(0xFFFFFF));
        assert_eq!(p.accent, colour(0x1AEBFF));
        assert_eq!(p.text_on_accent, colour(0x000000));
        assert_eq!(p.text_muted, colour(0x3FF23F));
    }

    #[test]
    fn a_contrast_theme_turns_mica_off() {
        let p = palette(aquatic());
        assert!(p.contrast && !p.mica);
    }
}
