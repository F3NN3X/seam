//! The window's backdrop and look: Mica when Windows offers it, solid otherwise, the contrast
//! theme's colours, and Text size. Windows has no event for most of them, so each is read once
//! here and on coming back, never while drawing.

use crate::theme::{Contrast, MicaOn, MotionOn, set_text_scale, text_scale};
use gpui_kit::component::Theme;
use gpui_kit::*;
use seam::winver;

/// The first build where Mica works through DWM's system backdrop.
const FIRST_MICA_BUILD: u32 = 22621;

fn wanted() -> bool {
    winver::build_number().is_some_and(|b| b >= FIRST_MICA_BUILD)
        && winver::transparency_enabled()
        && !winver::high_contrast()
}

fn appearance(on: bool) -> WindowBackgroundAppearance {
    if on { WindowBackgroundAppearance::MicaBackdrop } else { WindowBackgroundAppearance::Opaque }
}

/// The kit's font size at 100 %, which `THEMES` sets.
const KIT_FONT_SIZE: f32 = 14.;

/// Over Mica the theme's ground is clear, because `Root` paints it over the whole window.
/// A contrast theme gives the kit the system's colours, and Text size grows the kit's text.
fn set(on: bool, cx: &mut App) {
    cx.set_global(MicaOn(on));
    let colours = cx.try_global::<Contrast>().and_then(|c| c.0);
    Theme::update(cx, |theme| {
        if on {
            theme.colors.background = transparent_black();
        }
        if let Some(colours) = colours {
            crate::theme::contrast_theme(theme, colours);
        }
        theme.font_size = px(KIT_FONT_SIZE * text_scale() as f32 / 100.);
    });
}

/// The contrast theme and Text size, as Windows has them now. Whether either changed.
fn read_look(cx: &mut App) -> bool {
    let colours = winver::contrast_colours();
    let changed = cx.try_global::<Contrast>().is_none_or(|c| c.0 != colours);
    cx.set_global(Contrast(colours));
    set_text_scale(winver::text_scale_percent()) || changed
}

/// Animation effects has no event of its own, so it's read here and kept, not on every frame.
fn read_motion(cx: &mut App) {
    cx.set_global(MotionOn(winver::animation_effects()));
}

/// Before the window opens: what it should open with.
pub fn init(cx: &mut App) -> WindowBackgroundAppearance {
    read_motion(cx);
    read_look(cx);
    let on = wanted();
    set(on, cx);
    appearance(on)
}

/// The system's light or dark changed: the theme reloads, and the backdrop is looked at again.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
    read_motion(cx);
    read_look(cx);
    let on = wanted();
    set(on, cx);
    window.set_background_appearance(appearance(on));
    // Every ground is read from the palette, so draw them all again.
    window.refresh();
}

/// Transparency effects, contrast themes and Text size have no event of their own, so look again on
/// coming back.
pub fn recheck(window: &mut Window, cx: &mut App) {
    read_motion(cx);
    if read_look(cx) || wanted() != cx.try_global::<MicaOn>().is_some_and(|m| m.0) {
        follow_system(window, cx);
    }
}
