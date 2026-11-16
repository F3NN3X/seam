//! The window's backdrop: Mica when Windows offers it, solid otherwise.

use crate::theme::MicaOn;
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

/// Over Mica the theme's ground is clear, because `Root` paints it over the whole window.
fn set(on: bool, cx: &mut App) {
    cx.set_global(MicaOn(on));
    if on {
        Theme::update(cx, |theme| theme.colors.background = transparent_black());
    }
}

/// Before the window opens: what it should open with.
pub fn init(cx: &mut App) -> WindowBackgroundAppearance {
    let on = wanted();
    set(on, cx);
    appearance(on)
}

/// The system's light or dark changed: the theme reloads, and the backdrop is looked at again.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
    let on = wanted();
    set(on, cx);
    window.set_background_appearance(appearance(on));
    // Every ground is read from the palette, so draw them all again.
    window.refresh();
}

/// Transparency effects and contrast themes have no event of their own, so look again on coming back.
pub fn recheck(window: &mut Window, cx: &mut App) {
    if wanted() != cx.try_global::<MicaOn>().is_some_and(|m| m.0) {
        follow_system(window, cx);
    }
}
