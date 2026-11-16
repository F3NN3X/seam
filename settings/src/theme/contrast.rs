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
        text_muted: text,
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

/// The kit's own colours, for its buttons, inputs, menus, tabs, lists, tables and sliders. Fills of
/// Highlight pair with Highlight text, and everything else is Window and Window text.
pub fn theme(theme: &mut Theme, c: SystemColours) {
    let window = colour(c.window);
    let text = colour(c.window_text);
    let face = colour(c.button_face);
    let highlight = colour(c.highlight);
    let highlight_text = colour(c.highlight_text);
    let hotlight = colour(c.hotlight);
    let colors = &mut theme.colors;
    // Grounds.
    for ground in [
        &mut colors.background,
        &mut colors.accordion,
        &mut colors.popover,
        &mut colors.muted,
        &mut colors.list,
        &mut colors.list_even,
        &mut colors.list_head,
        &mut colors.table,
        &mut colors.table_even,
        &mut colors.table_head,
        &mut colors.table_foot,
        &mut colors.table_hover,
        &mut colors.sidebar,
        &mut colors.tab,
        &mut colors.tab_bar,
        &mut colors.tab_bar_segmented,
        &mut colors.title_bar,
        &mut colors.status_bar,
        &mut colors.group_box,
        &mut colors.description_list_label,
        &mut colors.scrollbar,
        &mut colors.switch_thumb,
        &mut colors.primary_foreground,
        &mut colors.button_primary_foreground,
        &mut colors.danger_foreground,
        &mut colors.success_foreground,
        &mut colors.warning_foreground,
        &mut colors.info_foreground,
        &mut colors.button_danger_foreground,
        &mut colors.button_success_foreground,
        &mut colors.button_warning_foreground,
        &mut colors.button_info_foreground,
    ] {
        *ground = window;
    }
    // Text, edges and marks.
    for ink in [
        &mut colors.foreground,
        &mut colors.muted_foreground,
        &mut colors.popover_foreground,
        &mut colors.group_box_foreground,
        &mut colors.description_list_label_foreground,
        &mut colors.sidebar_foreground,
        &mut colors.tab_foreground,
        &mut colors.table_head_foreground,
        &mut colors.table_foot_foreground,
        &mut colors.button_foreground,
        &mut colors.secondary_foreground,
        &mut colors.button_secondary_foreground,
        &mut colors.border,
        &mut colors.input,
        &mut colors.ring,
        &mut colors.caret,
        &mut colors.sidebar_border,
        &mut colors.title_bar_border,
        &mut colors.status_bar_border,
        &mut colors.table_row_border,
        &mut colors.window_border,
        &mut colors.list_active_border,
        &mut colors.table_active_border,
        &mut colors.drag_border,
        &mut colors.chart_grid,
        &mut colors.chart_bullish,
        &mut colors.chart_bearish,
        &mut colors.slider_bar,
        &mut colors.slider_thumb,
        &mut colors.switch,
        &mut colors.scrollbar_thumb,
        &mut colors.scrollbar_thumb_hover,
        &mut colors.primary,
        &mut colors.primary_hover,
        &mut colors.primary_active,
        &mut colors.button_primary,
        &mut colors.button_primary_hover,
        &mut colors.button_primary_active,
        &mut colors.danger,
        &mut colors.danger_hover,
        &mut colors.danger_active,
        &mut colors.button_danger,
        &mut colors.button_danger_hover,
        &mut colors.button_danger_active,
        &mut colors.success,
        &mut colors.success_hover,
        &mut colors.success_active,
        &mut colors.button_success,
        &mut colors.button_success_hover,
        &mut colors.button_success_active,
        &mut colors.warning,
        &mut colors.warning_hover,
        &mut colors.warning_active,
        &mut colors.button_warning,
        &mut colors.button_warning_hover,
        &mut colors.button_warning_active,
        &mut colors.info,
        &mut colors.info_hover,
        &mut colors.info_active,
        &mut colors.button_info,
        &mut colors.button_info_hover,
        &mut colors.button_info_active,
    ] {
        *ink = text;
    }
    // Buttons keep their face, and show a press or hover with their edge.
    for button in [
        &mut colors.button,
        &mut colors.button_hover,
        &mut colors.button_active,
        &mut colors.secondary,
        &mut colors.secondary_hover,
        &mut colors.secondary_active,
        &mut colors.button_secondary,
        &mut colors.button_secondary_hover,
        &mut colors.button_secondary_active,
    ] {
        *button = face;
    }
    // Chosen things: a Highlight fill, with Highlight text on it.
    for fill in [
        &mut colors.accent,
        &mut colors.list_active,
        &mut colors.list_hover,
        &mut colors.table_active,
        &mut colors.tab_active,
        &mut colors.sidebar_accent,
        &mut colors.sidebar_primary,
        &mut colors.selection,
        &mut colors.drop_target,
        &mut colors.progress_bar,
    ] {
        *fill = highlight;
    }
    for on_fill in [
        &mut colors.accent_foreground,
        &mut colors.tab_active_foreground,
        &mut colors.sidebar_accent_foreground,
        &mut colors.sidebar_primary_foreground,
    ] {
        *on_fill = highlight_text;
    }
    for link in [&mut colors.link, &mut colors.link_hover, &mut colors.link_active] {
        *link = hotlight;
    }
    colors.overlay = window.opacity(0.7);
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
        assert_eq!(p.text_muted, colour(0xFFFFFF));
    }

    #[test]
    fn a_contrast_theme_turns_mica_off() {
        let p = palette(aquatic());
        assert!(p.contrast && !p.mica);
    }
}
