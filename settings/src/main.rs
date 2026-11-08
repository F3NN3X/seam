//! The Seam window, a separate process so its UI can never delay the mouse
//! hook. It reaches the tray only through the local API.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod help;
mod icons;
mod pages;
mod panel;
mod search;
mod theme;
mod ui;
mod update;

use gpui_kit::*;
use panel::{Page, Panel};
use seam::api::Client;
use seam::geom;
use seam::panelwin;
use seam::tray;
use serde_json::json;
use std::time::{Duration, Instant};

const TRAY_START_TIMEOUT: Duration = Duration::from_secs(3);

fn arg_after(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn next_to_this_exe(name: &str) -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.with_file_name(name))
}

/// Only a panel the tray started has the session's token, so any other launch
/// makes sure the tray is up and asks it for one.
fn hand_over_to_tray(page: Option<String>) {
    if Client::connect("seam-settings", None).is_err() {
        if let Some(tray) = next_to_this_exe("seam.exe") {
            // Only the window was asked for, so the tray needn't open another.
            let _ = std::process::Command::new(tray).arg("--tray-only").spawn();
        }
    }
    let started = Instant::now();
    while started.elapsed() < TRAY_START_TIMEOUT {
        if let Ok(mut client) = Client::connect("seam-settings", None) {
            let params = page.map_or_else(|| json!({}), |p| json!({ "page": p }));
            let _ = client.call("panel.open", params);
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Leave this window for one with a fresh token, starting the tray if needed.
pub fn restart_panel(cx: &mut App) {
    if let Some(panel) = next_to_this_exe("seam-settings.exe") {
        let _ = std::process::Command::new(panel).spawn();
    }
    cx.quit();
}

/// The window's logical size, which is the same on every screen.
const WINDOW_SIZE: (f32, f32) = (1160., 780.);

/// Where the window opens: centred in the work area of the screen under the
/// pointer, and the display gpui should open it on. The sums are in physical
/// pixels, where the size depends on that screen's scale; gpui is given back the
/// logical equivalent. Without an answer from Windows it is the primary screen.
fn opening_bounds(cx: &App) -> (Bounds<Pixels>, Option<DisplayId>) {
    let (w, h) = WINDOW_SIZE;
    let Some((monitor, work, dpi)) = panelwin::screen_under_pointer() else {
        return (Bounds::centered(None, size(px(w), px(h)), cx), None);
    };
    let scale = dpi as f32 / 96.;
    let work = geom::Rect::new(work.x, work.y, work.w, work.h);
    let at = geom::centre_in(&work, (w * scale).round() as i32, (h * scale).round() as i32);
    let bounds = Bounds::new(
        point(px(at.left as f32 / scale), px(at.top as f32 / scale)),
        size(px(at.width() as f32 / scale), px(at.height() as f32 / scale)),
    );
    (bounds, Some(DisplayId::new(monitor)))
}

fn main() {
    // The updater's headless modes, and the installer's. None opens a window.
    if std::env::args().any(|a| a == "--check-updates") {
        std::process::exit(update::check_main());
    }
    if std::env::args().any(|a| a == "--quit-tray") {
        std::process::exit(update::quit_main());
    }
    if let Some(version) = arg_after("--install") {
        std::process::exit(update::install_main(&version));
    }
    let page = arg_after("--page");
    let Some(token) = arg_after("--token") else {
        hand_over_to_tray(page);
        return;
    };
    if tray::raise_open_panel() {
        return;
    }
    application().with_assets(assets::AllAssets).run(move |cx| {
        init(cx);
        theme::install(cx);
        Panel::bind_keys(cx);
        let (bounds, display_id) = opening_bounds(cx);
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some(tray::PANEL_TITLE.into()), ..Default::default() }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            window_min_size: Some(size(px(900.), px(640.))),
            ..Default::default()
        };
        let page = page.as_deref().and_then(Page::from_arg);
        open_window(options, cx, move |window, cx| cx.new(|cx| Panel::new(page, token, window, cx)))
            .expect("failed to open the Seam window");
    });
}
