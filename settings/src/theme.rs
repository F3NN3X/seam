//! The "Seam" theme: the design tokens, for our own elements and for the kit's.

use gpui_kit::component::{ActiveTheme, Theme, ThemeRegistry};
use gpui_kit::*;

const THEMES: &str = r##"{"name":"Seam","themes":[
 {"name":"Seam Light","mode":"light","font.family":"Segoe UI Variable Text","font.size":14,
  "mono_font.family":"Cascadia Mono","mono_font.size":13,"radius":8,"radius.lg":12,"shadow":false,
  "colors":{"background":"#F3F3F3","foreground":"#1A1A1A","border":"#1A1A1A14","ring":"#1A1A1A","input.border":"#1A1A1A14",
   "muted.background":"#FBFBFB","muted.foreground":"#666666","popover.background":"#FFFFFF","popover.foreground":"#1A1A1A",
   "accent.background":"#FFFFFF","accent.foreground":"#1A1A1A","primary.background":"#1A1A1A","primary.foreground":"#FFFFFF",
   "primary.hover.background":"#333333","primary.active.background":"#333333","secondary.background":"#FFFFFF",
   "secondary.foreground":"#1A1A1A","secondary.hover.background":"#FBFBFB","secondary.active.background":"#FBFBFB",
   "button.primary.background":"#1A1A1A","button.primary.foreground":"#FFFFFF","button.primary.hover.background":"#333333",
   "button.primary.active.background":"#333333","button.secondary.background":"#FFFFFF","button.secondary.foreground":"#1A1A1A",
   "button.secondary.hover.background":"#FBFBFB","button.secondary.active.background":"#FBFBFB",
   "scrollbar.background":"#00000000","selection.background":"#FFB02066","caret":"#1A1A1A"}},
 {"name":"Seam Dark","mode":"dark","font.family":"Segoe UI Variable Text","font.size":14,
  "mono_font.family":"Cascadia Mono","mono_font.size":13,"radius":8,"radius.lg":12,"shadow":false,
  "colors":{"background":"#202020","foreground":"#F3F3F3","border":"#F3F3F314","ring":"#F3F3F3","input.border":"#F3F3F314",
   "muted.background":"#2A2A2A","muted.foreground":"#A0A0A0","popover.background":"#333333","popover.foreground":"#F3F3F3",
   "accent.background":"#3A3A3A","accent.foreground":"#F3F3F3","primary.background":"#F3F3F3","primary.foreground":"#1A1A1A",
   "primary.hover.background":"#FFFFFF","primary.active.background":"#FFFFFF","secondary.background":"#333333",
   "secondary.foreground":"#F3F3F3","secondary.hover.background":"#3A3A3A","secondary.active.background":"#3A3A3A",
   "button.primary.background":"#F3F3F3","button.primary.foreground":"#1A1A1A","button.primary.hover.background":"#FFFFFF",
   "button.primary.active.background":"#FFFFFF","button.secondary.background":"#333333","button.secondary.foreground":"#F3F3F3",
   "button.secondary.hover.background":"#3A3A3A","button.secondary.active.background":"#3A3A3A",
   "scrollbar.background":"#00000000","selection.background":"#FFB02066","caret":"#F3F3F3"}}]}"##;

pub fn install(cx: &mut App) {
    let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(THEMES);
    let (light, dark) = {
        let themes = ThemeRegistry::global(cx).themes();
        (themes.get("Seam Light").cloned(), themes.get("Seam Dark").cloned())
    };
    Theme::update(cx, |t| {
        if let Some(light) = light {
            t.light_theme = light;
        }
        if let Some(dark) = dark {
            t.dark_theme = dark;
        }
        t.notification.placement = Anchor::BottomRight;
        t.notification.margins.top = px(16.);
        t.notification.width = px(tokens::SIZE_NOTIFICATION);
        t.notification.max_items = 1;
    });
    Theme::sync_system_appearance(None, cx);
}

/// The design system's sizes, radii, type and motion, under its names.
pub mod tokens {
    use gpui_kit::*;

    pub const SPACE_2: f32 = 2.;
    pub const SPACE_4: f32 = 4.;
    pub const SPACE_8: f32 = 8.;
    pub const SPACE_12: f32 = 12.;
    pub const SPACE_16: f32 = 16.;
    pub const SPACE_24: f32 = 24.;
    pub const SPACE_32: f32 = 32.;

    #[allow(dead_code)]
    pub const SIZE_NAV_ITEM: f32 = 32.;
    // No 28 px buttons or 420 px dialogs exist yet; dialogs keep the kit's width.
    #[allow(dead_code)]
    pub const SIZE_BUTTON_SMALL: f32 = 28.;
    pub const SIZE_ROW: f32 = 48.;
    pub const SIZE_ROW_2: f32 = 56.;
    #[allow(dead_code)]
    pub const SIZE_MENU_ITEM: f32 = 32.;
    pub const SIZE_SIDEBAR: f32 = 240.;
    pub const SIZE_ICON: f32 = 16.;
    pub const SIZE_NOTIFICATION: f32 = 360.;
    #[allow(dead_code)]
    pub const SIZE_DIALOG: f32 = 420.;
    // For 0.8 (`modern-shell`): defined, not yet used.
    #[allow(dead_code)]
    pub const SIZE_MENU_ITEM_2: f32 = 48.;
    pub const SIZE_TITLE_BAR: f32 = 48.;
    /// gpui-kit's fixed window control width, for reference only.
    #[allow(dead_code)]
    pub const SIZE_WINDOW_CONTROL: f32 = 35.;
    pub const SIZE_COMMAND_BAR: f32 = 400.;
    #[allow(dead_code)]
    pub const SIZE_DIALOG_WIDE: f32 = 600.;

    #[allow(dead_code)]
    pub const RADIUS_2: f32 = 2.;
    pub const RADIUS_4: f32 = 4.;
    pub const RADIUS_6: f32 = 6.;
    pub const RADIUS_8: f32 = 8.;
    pub const RADIUS_10: f32 = 10.;
    pub const RADIUS_12: f32 = 12.;

    pub const OPACITY_DISABLED: f32 = 0.4;

    // For 0.8 (`modern-shell`): defined, not yet used.
    #[allow(dead_code)]
    pub const MOTION_FADE_MS: u64 = 150;
    #[allow(dead_code)]
    pub const MOTION_EASE_MS: u64 = 200;

    pub const DISPLAY_FONT: &str = "Segoe UI Variable Display";

    /// One step of the type scale. Sizes stay plain numbers so `text_style` is the one
    /// place they get applied.
    #[derive(Clone, Copy)]
    pub struct TextStyle {
        pub size: f32,
        pub line: f32,
        pub weight: FontWeight,
    }

    const fn text(size: f32, line: f32, weight: FontWeight) -> TextStyle {
        TextStyle { size, line, weight }
    }

    pub const PAGE_TITLE: TextStyle = text(24., 32., FontWeight::NORMAL);
    #[allow(dead_code)]
    pub const CARD_TITLE: TextStyle = text(16., 22., FontWeight::NORMAL);
    #[allow(dead_code)]
    pub const CARD_TITLE_STRONG: TextStyle = text(16., 22., FontWeight::MEDIUM);
    pub const BODY: TextStyle = text(14., 20., FontWeight::NORMAL);
    pub const LABEL: TextStyle = text(13., 18., FontWeight::NORMAL);
    #[allow(dead_code)]
    pub const LABEL_STRONG: TextStyle = text(13., 18., FontWeight::MEDIUM);
    pub const CAPTION: TextStyle = text(12., 16., FontWeight::NORMAL);
    #[allow(dead_code)]
    pub const MICRO: TextStyle = text(11., 14., FontWeight::NORMAL);
    #[allow(dead_code)]
    pub const MONO: TextStyle = text(13., 18., FontWeight::NORMAL);

    /// Size, line height and weight of `s` on `e`.
    pub fn text_style<E: Styled>(e: E, s: TextStyle) -> E {
        e.text_size(px(s.size)).line_height(px(s.line)).font_weight(s.weight)
    }
}

/// The design system's colour tokens that have no kit equivalent.
#[derive(Clone, Copy)]
pub struct Palette {
    /// surface-sidebar: sidebar, segmented and tab track.
    pub surface_sidebar: Hsla,
    /// surface-page: window ground.
    pub surface_page: Hsla,
    /// surface-card: cards, tiles, status card, nav hover.
    pub surface_card: Hsla,
    /// surface-raised: active nav, secondary button, menus, dialogs.
    pub surface_raised: Hsla,
    /// surface-segment: chosen segment, secondary hover, menu hover.
    pub surface_segment: Hsla,
    /// surface-input: inputs, search.
    pub surface_input: Hsla,
    /// surface-zone: zone blocks, icon placeholders.
    #[allow(dead_code)] // Used by the `modern-shell` follow-up.
    pub surface_zone: Hsla,
    /// surface-card-mica: cards while Mica shows through.
    pub surface_card_mica: Hsla,
    /// surface-input-mica: inputs and the command bar over Mica.
    pub surface_input_mica: Hsla,
    /// surface-track-mica: segmented track and map well over Mica.
    pub surface_track_mica: Hsla,
    /// Mica shows through, so `surface_card`, `surface_input` and `surface_sidebar` (the
    /// track) already hold their Mica grounds and the window's grounds are clear.
    pub mica: bool,
    /// hairline: card borders, row dividers (decorative).
    pub hairline: Hsla,
    /// hairline-strong: dashed notes, keycaps (decorative).
    pub hairline_strong: Hsla,
    /// control-stroke: edges that carry meaning, at least 3:1.
    pub control_stroke: Hsla,
    /// text-primary: titles, values.
    pub text_primary: Hsla,
    /// text-secondary: second lines, section labels.
    pub text_secondary: Hsla,
    /// text-muted: placeholders, "Not set", version.
    pub text_muted: Hsla,
    /// fill-inverse: primary button, on Switch.
    pub fill_inverse: Hsla,
    /// fill-inverse-hover: primary hover and pressed.
    #[allow(dead_code)] // The kit's primary button carries its own hover.
    pub fill_inverse_hover: Hsla,
    /// text-on-inverse: label and knob on `fill_inverse`.
    pub text_on_inverse: Hsla,
    /// accent: Seam amber fills and 2px strokes.
    pub accent: Hsla,
    /// accent-ink: amber as text, a thin line or a 1px outline on light.
    #[allow(dead_code)] // Used by the `modern-shell` follow-up.
    pub accent_ink: Hsla,
    /// text-on-accent: glyphs on an amber fill.
    #[allow(dead_code)] // Used by the `modern-shell` follow-up.
    pub text_on_accent: Hsla,
    /// accent-tint: lit zone, selected tile ground.
    #[allow(dead_code)] // Used by the `modern-shell` follow-up.
    pub accent_tint: Hsla,
    /// selection: selected text.
    #[allow(dead_code)] // The kit reads it from `THEMES`.
    pub selection: Hsla,
    /// focus-ring: keyboard focus ring.
    pub focus_ring: Hsla,
    /// positive: good-state dot.
    pub positive: Hsla,
    /// negative: problem dot, conflict keycap edge.
    pub negative: Hsla,
}

/// Whether the window shows Mica, which `chrome` keeps up to date.
pub struct MicaOn(pub bool);

impl Global for MicaOn {}

impl Palette {
    /// The window's ground behind the sidebar.
    pub fn sidebar_ground(&self) -> Hsla {
        if self.mica { transparent_black() } else { self.surface_sidebar }
    }

    /// The window's ground behind the page.
    pub fn page_ground(&self) -> Hsla {
        if self.mica { transparent_black() } else { self.surface_page }
    }

    /// The sunk ground of the map well.
    pub fn well(&self) -> Hsla {
        if self.mica { self.surface_track_mica } else { self.surface_page }
    }
}

pub fn palette(cx: &App) -> Palette {
    let base = base_palette(cx);
    if !cx.try_global::<MicaOn>().is_some_and(|m| m.0) {
        return base;
    }
    Palette {
        surface_card: base.surface_card_mica,
        surface_input: base.surface_input_mica,
        surface_sidebar: base.surface_track_mica,
        mica: true,
        ..base
    }
}

fn base_palette(cx: &App) -> Palette {
    let c = |hex: u32| -> Hsla { rgb(hex).into() };
    let a = |hex: u32| -> Hsla { rgba(hex).into() };
    if cx.theme().is_dark() {
        Palette {
            surface_sidebar: c(0x1A1A1A),
            surface_page: c(0x202020),
            surface_card: c(0x2A2A2A),
            surface_raised: c(0x333333),
            surface_segment: c(0x3A3A3A),
            surface_input: c(0x262626),
            surface_zone: c(0x3A3A3A),
            surface_card_mica: a(0xFFFFFF0D),
            surface_input_mica: a(0xFFFFFF0F),
            surface_track_mica: a(0x0000004D),
            mica: false,
            hairline: a(0xF3F3F314),
            hairline_strong: a(0xF3F3F329),
            control_stroke: c(0x8A8A8A),
            text_primary: c(0xF3F3F3),
            text_secondary: c(0xB0B0B0),
            text_muted: c(0xA0A0A0),
            fill_inverse: c(0xF3F3F3),
            fill_inverse_hover: c(0xFFFFFF),
            text_on_inverse: c(0x1A1A1A),
            accent: c(0xFFB020),
            accent_ink: c(0xFFB020),
            text_on_accent: c(0x1A1A1A),
            accent_tint: a(0xFFB0201A),
            selection: a(0xFFB02066),
            focus_ring: c(0xF3F3F3),
            positive: c(0x4CC38A),
            negative: c(0xF07A7D),
        }
    } else {
        Palette {
            surface_sidebar: c(0xEBEBEB),
            surface_page: c(0xF3F3F3),
            surface_card: c(0xFBFBFB),
            surface_raised: c(0xFFFFFF),
            surface_segment: c(0xFFFFFF),
            surface_input: c(0xFFFFFF),
            surface_zone: c(0xE0E0E0),
            surface_card_mica: a(0xFFFFFFB3),
            surface_input_mica: a(0xFFFFFFCC),
            surface_track_mica: a(0x0000000F),
            mica: false,
            hairline: a(0x1A1A1A14),
            hairline_strong: a(0x1A1A1A29),
            control_stroke: c(0x868686),
            text_primary: c(0x1A1A1A),
            text_secondary: c(0x5C5C5C),
            text_muted: c(0x666666),
            fill_inverse: c(0x1A1A1A),
            fill_inverse_hover: c(0x333333),
            text_on_inverse: c(0xFFFFFF),
            accent: c(0xFFB020),
            accent_ink: c(0x9A5B00),
            text_on_accent: c(0x1A1A1A),
            accent_tint: a(0xFFB0201F),
            selection: a(0xFFB02066),
            focus_ring: c(0x1A1A1A),
            positive: c(0x15803D),
            negative: c(0xB91C1C),
        }
    }
}
