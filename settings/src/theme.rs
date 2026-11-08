//! The "Seam" theme: the design tokens, for our own elements and for the kit's.

use gpui_kit::component::{ActiveTheme, Theme, ThemeRegistry};
use gpui_kit::*;

const THEMES: &str = r##"{"name":"Seam","themes":[
 {"name":"Seam Light","mode":"light","font.family":"Segoe UI Variable Text","font.size":14,
  "mono_font.family":"Cascadia Mono","mono_font.size":13,"radius":8,"radius.lg":12,"shadow":false,
  "colors":{"background":"#F3F3F3","foreground":"#1A1A1A","border":"#1A1A1A14","ring":"#1A1A1A","input.border":"#1A1A1A14",
   "muted.background":"#FBFBFB","muted.foreground":"#707070","popover.background":"#FFFFFF","popover.foreground":"#1A1A1A",
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
   "muted.background":"#2A2A2A","muted.foreground":"#949494","popover.background":"#333333","popover.foreground":"#F3F3F3",
   "accent.background":"#333333","accent.foreground":"#F3F3F3","primary.background":"#F3F3F3","primary.foreground":"#1A1A1A",
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
        t.notification.width = px(360.);
        t.notification.max_items = 1;
    });
    Theme::sync_system_appearance(None, cx);
}

/// Design tokens that have no kit equivalent, from design-rules.md.
#[derive(Clone, Copy)]
pub struct Palette {
    pub sidebar: Hsla,
    pub page: Hsla,
    pub card: Hsla,
    pub raised: Hsla,
    pub segment: Hsla,
    pub input: Hsla,
    pub hairline: Hsla,
    pub hairline_strong: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub positive: Hsla,
    pub negative: Hsla,
}

pub fn palette(cx: &App) -> Palette {
    let c = |hex: u32| -> Hsla { rgb(hex).into() };
    let a = |hex: u32| -> Hsla { rgba(hex).into() };
    if cx.theme().is_dark() {
        Palette {
            sidebar: c(0x1A1A1A),
            page: c(0x202020),
            card: c(0x2A2A2A),
            raised: c(0x333333),
            segment: c(0x3A3A3A),
            input: c(0x262626),
            hairline: a(0xF3F3F314),
            hairline_strong: a(0xF3F3F329),
            text: c(0xF3F3F3),
            secondary: c(0xB0B0B0),
            muted: c(0x949494),
            accent: c(0xFFB020),
            positive: c(0x4CC38A),
            negative: c(0xF07A7D),
        }
    } else {
        Palette {
            sidebar: c(0xEBEBEB),
            page: c(0xF3F3F3),
            card: c(0xFBFBFB),
            raised: c(0xFFFFFF),
            segment: c(0xFFFFFF),
            input: c(0xFFFFFF),
            hairline: a(0x1A1A1A14),
            hairline_strong: a(0x1A1A1A29),
            text: c(0x1A1A1A),
            secondary: c(0x5C5C5C),
            muted: c(0x707070),
            accent: c(0xFFB020),
            positive: c(0x16A34A),
            negative: c(0xDC2626),
        }
    }
}
