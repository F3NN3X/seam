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

/// The screen under the pointer: its gpui display, work area (physical pixels)
/// and DPI.
struct Target {
    display: DisplayId,
    work: seam::zones::Rect,
    dpi: u32,
}

impl Target {
    fn under_pointer() -> Option<Target> {
        let (monitor, work, dpi) = panelwin::screen_under_pointer()?;
        Some(Target { display: DisplayId::new(monitor), work, dpi })
    }

    /// The work area in logical pixels at this screen's scale.
    fn logical_work(&self) -> Bounds<Pixels> {
        let scale = self.dpi as f32 / 96.;
        let at = |v: i32| px(v as f32 / scale);
        Bounds::new(point(at(self.work.x), at(self.work.y)), size(at(self.work.w), at(self.work.h)))
    }
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
        let (w, h) = WINDOW_SIZE;
        let target = Target::under_pointer();
        // gpui puts the window on the screen under the pointer, so it never shows
        // on the primary. It sizes it with the scale of the screen it was made on,
        // not that screen's, so the position and size are set again in physical
        // pixels straight after, by `panelwin::open_centred`.
        let (bounds, display_id, work) = match &target {
            Some(t) => {
                let work = t.logical_work();
                (Bounds::centered_at(work.center(), size(px(w), px(h))), Some(t.display), Some(work.size))
            }
            None => (Bounds::centered(None, size(px(w), px(h)), cx), None, None),
        };
        // A screen smaller than the window needs a smaller least size, or the
        // window couldn't shrink to fit (64 is room for the title bar and frame).
        let room =
            |work: Option<Pixels>, least: f32| work.map_or(px(least), |v| px(least).min(v - px(64.)).max(px(300.)));
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some(tray::PANEL_TITLE.into()), ..Default::default() }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            window_min_size: Some(size(room(work.map(|s| s.width), 900.), room(work.map(|s| s.height), 640.))),
            ..Default::default()
        };
        let page = page.as_deref().and_then(Page::from_arg);
        open_window(options, cx, move |window, cx| cx.new(|cx| Panel::new(page, token, window, cx)))
            .expect("failed to open the Seam window");
        if let Some(t) = target {
            // gpui only hears about the new size if it isn't mid-update when Win32
            // sends it, so this runs as a task of its own.
            cx.spawn(async move |_| {
                panelwin::open_centred(tray::PANEL_TITLE, t.work, t.dpi, WINDOW_SIZE);
            })
            .detach();
        }
    });
}
