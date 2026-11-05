//! The Seam window, a separate process so its UI can never delay the mouse
//! hook. It reaches the tray only through the local API.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod pages;
mod panel;
mod search;
mod theme;
mod ui;

use gpui_kit::*;
use panel::{Page, Panel};
use seam::api::Client;
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
            let _ = std::process::Command::new(tray).spawn();
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

fn main() {
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
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some(tray::PANEL_TITLE.into()), ..Default::default() }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1080.), px(780.)), cx))),
            window_min_size: Some(size(px(900.), px(640.))),
            ..Default::default()
        };
        let page = page.as_deref().and_then(Page::from_arg);
        open_window(options, cx, move |window, cx| cx.new(|cx| Panel::new(page, token, window, cx)))
            .expect("failed to open the Seam window");
    });
}
