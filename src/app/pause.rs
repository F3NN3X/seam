//! Pause Seam and stepping aside for full-screen apps (D390), moved out of `app.rs`.
//! The features ask `standby::reason`; this applies a change of it to the ones that
//! hold something: shortcuts, snapping, Spotlight, the focus border and the drift check.

use super::*;
use crate::standby::{self, Standby};

/// A foreground change, or the full-screen window going: look at what is in front.
pub(super) fn standby_check(app: &mut App) {
    let before = standby::reason();
    if standby::check() {
        standby_changed(app, before);
    }
}

/// "Step aside for games and full-screen apps" may have changed, or Seam is starting.
pub(super) fn standby_configure(app: &mut App) {
    let before = standby::reason();
    if standby::configure(app.hwnd, app.cfg.step_aside_fullscreen) {
        standby_changed(app, before);
    }
}

/// Pause or resume, or toggle with `None`. Returns whether Seam is paused now.
pub(super) fn set_paused(app: &mut App, on: Option<bool>) -> bool {
    let before = standby::reason();
    let now = on.unwrap_or(!standby::paused());
    if now != standby::paused() {
        standby::set_paused(now);
        log::info!("seam {}", if now { "paused" } else { "resumed" });
        standby_changed(app, before);
    }
    now
}

/// Why Seam stands by went from `before` to what it is now. One pass over everything
/// that holds something, so leaving is one rebuild of the handles, not one per feature.
fn standby_changed(app: &mut App, before: Option<Standby>) {
    let now = standby::reason();
    match (before, now) {
        (None, Some(_)) if spotlight::is_on() => {
            app.spotlight_resume = true;
            show_spotlight(app, false);
        }
        (Some(_), None) if std::mem::take(&mut app.spotlight_resume) => {
            show_spotlight(app, true);
        }
        _ => {}
    }
    if now == Some(Standby::Paused) && before != now {
        cancel_sign_in(app);
    }
    apply_hotkeys(app);
    relink(app);
    // Moves made while paused were not looked at, so look once now.
    if before == Some(Standby::Paused) && now != Some(Standby::Paused) {
        app.recheck = true;
        unsafe { SetTimer(Some(app.hwnd), TIMER_DRIFT, DRIFT_AFTER_MS, None) };
    }
    publish(app);
}
