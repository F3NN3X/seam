//! App-wide things: startup, the tray icon, other apps, files.

use crate::panel::{error_text, refusal, Panel};
use crate::theme::palette;
use crate::ui::{self, card, row, row_with, section, switch};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::*;
use seam::autostart;
use seam::config::{Config, RestoreApps, TrayClick};
use seam::package::{self, Startup};
use serde_json::json;
use std::path::PathBuf;

fn export(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let dir = dirs_home();
    let chosen = cx.prompt_for_new_path(&dir, Some("seam-config.json"));
    panel.call("config.get", json!({}), window, cx, move |_, result, window, cx| {
        let config = match result {
            Ok(config) => config,
            Err(e) => return window.push_notification(refusal("Settings not exported", &error_text(&e)), cx),
        };
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(path))) = chosen.await else { return };
            let saved = serde_json::to_string_pretty(&config).map_err(std::io::Error::other);
            let saved = saved.and_then(|text| std::fs::write(&path, text));
            let _ = cx.update(|window, cx| match saved {
                Ok(()) => window.push_notification(Notification::new().message("Exported your settings"), cx),
                Err(e) => window.push_notification(refusal("Settings not exported", &e.to_string()), cx),
            });
        })
        .detach();
    });
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn import(window: &mut Window, cx: &mut Context<Panel>) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Import".into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        let Some(path) = paths.first() else { return };
        let read = std::fs::read(path).map_err(|e| e.to_string());
        let parsed = read.and_then(|bytes| Config::from_import(&bytes));
        let _ = this.update_in(cx, move |this, window, cx| match parsed {
            Ok(next) => confirm_import(this, next, window, cx),
            Err(e) => window.push_notification(refusal("That file can't be imported", &e), cx),
        });
    })
    .detach();
}

fn confirm_import(panel: &Panel, next: Config, window: &mut Window, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    let current = serde_json::to_string_pretty(&panel.config).unwrap_or_default();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, next, current) = (view.clone(), next.clone(), current.clone());
        alert
            .title("Replace your settings?")
            .description("A copy of your current settings stays in the settings folder")
            .ok_text("Import")
            .on_ok(move |_, window, cx| {
                let backup = Config::path().with_file_name("config.before-import.json");
                if let Err(e) = std::fs::write(backup, &current) {
                    window.push_notification(refusal("Nothing imported", &e.to_string()), cx);
                    return true;
                }
                let next = next.clone();
                let _ = view.update(cx, |panel, cx| panel.replace_config(&next, window, cx));
                window.push_notification(Notification::new().message("Imported your settings"), cx);
                true
            })
    });
}

/// Seam reopening the apps turns Windows' Restart apps off, so ask first (D128).
/// Cancel leaves Windows reopening them.
fn confirm_seam_reopens(window: &mut Window, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let view = view.clone();
        alert
            .title("Turn off Windows' Restart apps?")
            .description(
                "Windows would also reopen apps at sign-in, so they'd open twice. Seam turns its setting off while Seam reopens them, and puts it back when you switch back or turn restore off.",
            )
            .ok_text("Turn it off")
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let _ = view.update(cx, |panel, cx| panel.patch(json!({ "restore_apps": "seam" }), window, cx));
                true
            })
    });
}

fn open_folder() {
    let _ = std::process::Command::new("explorer").arg(package::settings_dir()).spawn();
}

/// Where the settings are. The Store copy's live in its package folder.
fn folder_label() -> String {
    let dir = package::settings_dir().display().to_string();
    match std::env::var("LOCALAPPDATA") {
        Ok(local) if package::is_packaged() => dir.replacen(&local, "%LOCALAPPDATA%", 1),
        _ => r"%APPDATA%\Seam".to_string(),
    }
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let view = cx.entity();
    let setter = |patch: fn(bool) -> serde_json::Value| {
        let view = view.clone();
        move |on: bool, window: &mut Window, cx: &mut App| {
            view.update(cx, |panel, cx| panel.patch(patch(on), window, cx));
        }
    };

    // The Store copy asks Windows, which can keep the task off. Until it
    // answers, the switch can't be used.
    let packaged = package::is_packaged();
    let locked = packaged && panel.startup.is_none_or(Startup::locked);
    let blocked = packaged && panel.startup.is_some_and(Startup::locked);
    let start_on = if packaged { panel.startup.is_some_and(Startup::is_on) } else { autostart::is_enabled() };
    let start_with_windows = div().opacity(if locked { 0.5 } else { 1. }).child(switch(
        "autostart",
        "Start with Windows",
        start_on,
        window,
        cx,
        {
            let view = view.clone();
            move |on, _, cx| {
                if locked {
                    return;
                }
                if packaged {
                    view.update(cx, |panel, cx| panel.set_startup(on, cx));
                    return;
                }
                if let Ok(tray) = std::env::current_exe().map(|e| e.with_file_name("seam.exe")) {
                    autostart::set(on, &tray);
                }
                view.update(cx, |_, cx| cx.notify());
            }
        },
    ));
    let start_about = if blocked {
        "Turned off in Windows Settings, Startup apps"
    } else {
        "Seam waits in the tray after you sign in"
    };
    let offer = switch(
        "ask-new-desks",
        "Offer to line up new screens",
        panel.config.ask_new_desks,
        window,
        cx,
        setter(|on| json!({ "ask_new_desks": on })),
    );
    let restore = switch(
        "restore-session",
        "Offer to restore my windows after I sign in",
        panel.config.restore_session,
        window,
        cx,
        setter(|on| json!({ "restore_session": on })),
    );
    let seam_reopens = panel.config.restore_apps == RestoreApps::Seam;
    let reopen_choice = ui::segmented(
        "restore-apps",
        "Who reopens the apps",
        &["Windows reopens them", "Seam reopens them"],
        usize::from(seam_reopens),
        window,
        cx,
        {
            let view = view.clone();
            move |i, window, cx| {
                view.update(cx, |panel, cx| match i {
                    0 => panel.patch(json!({ "restore_apps": "windows" }), window, cx),
                    _ if panel.config.restore_apps != RestoreApps::Seam => confirm_seam_reopens(window, cx),
                    _ => {}
                })
            }
        },
    );
    let reopen_about = if seam_reopens {
        "Seam starts the apps that aren't running, and turns Windows' Restart apps off meanwhile"
    } else {
        "Windows' Restart apps or the apps themselves open them, and Seam puts each window back"
    };
    let restart_note = (panel.config.restore_session
        && seam_reopens
        && panel.state.as_ref().is_some_and(|s| s.restart_apps))
    .then(|| {
        let open = ui::secondary("open-sign-in-options", "Open Sign-in options", cx)
            .on_click(|_, _, cx| cx.open_url("ms-settings:signinoptions"));
        ui::note("Windows' Restart apps is on again, so apps may open twice", Some(open), p)
    });
    let selected = usize::from(panel.config.tray_click == TrayClick::LineUp);
    let left_click =
        ui::segmented("tray-click", "Left-click", &["Open Seam", "Start Line Up"], selected, window, cx, {
            let view = view.clone();
            move |i, window, cx| {
                let value = if i == 0 { "open" } else { "line_up" };
                view.update(cx, |panel, cx| panel.patch(json!({ "tray_click": value }), window, cx));
            }
        });
    let connected = panel.state.as_ref().map_or(0, |s| s.clients.iter().filter(|c| *c != "panel").count());
    let api_about =
        format!("Stream Deck and tools on this PC can read status and switch workspaces. {connected} connected");
    let api = switch(
        "api-enabled",
        "Let other apps control Seam",
        panel.config.api.enabled,
        window,
        cx,
        setter(|on| json!({ "api": { "enabled": on } })),
    );
    let debug_log =
        switch("debug-log", "Debug log", panel.config.debug_log, window, cx, setter(|on| json!({ "debug_log": on })));

    let folder = div()
        .flex()
        .gap(px(8.))
        .child(
            ui::secondary("export", "Export…", cx)
                .on_click(cx.listener(|panel, _, window, cx| export(panel, window, cx))),
        )
        .child(ui::secondary("import", "Import…", cx).on_click(cx.listener(|_, _, window, cx| import(window, cx))))
        .child(ui::secondary("open-folder", "Open", cx).on_click(|_, _, _| open_folder()));
    let folder_path = div()
        .font_family(cx.theme().mono_font_family.clone())
        .font_weight(FontWeight::NORMAL)
        .text_size(px(13.))
        .text_color(p.secondary)
        .child(folder_label());

    let mut startup_rows = vec![
        row("Start with Windows", Some(start_about.into()), start_with_windows, p),
        row("Offer to line up new screens", Some("Asks once when screens Seam hasn't seen connect".into()), offer, p),
        row(
            "Offer to restore my windows after I sign in",
            Some("Seam remembers which apps were where and asks before it starts any".into()),
            restore,
            p,
        ),
    ];
    if panel.config.restore_session {
        startup_rows.push(row("Who reopens the apps", Some(reopen_about.into()), reopen_choice, p));
    }

    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(ui::page_title("General", "How Seam starts and where it keeps its files", p))
        .child(section(
            "Startup",
            div().flex().flex_col().gap(px(8.)).child(card(startup_rows, p)).children(restart_note),
            p,
        ))
        .child(section(
            "Tray icon",
            card(vec![row("Left-click", Some("Right-click always shows the menu".into()), left_click, p)], p),
            p,
        ))
        .child(section(
            "Other apps",
            card(vec![row("Let other apps control Seam", Some(api_about.into()), api, p)], p),
            p,
        ))
        .child(section(
            "Files and troubleshooting",
            card(
                vec![
                    row_with("Settings folder", Some(folder_path.into_any_element()), folder),
                    row("Debug log", Some("Writes remaps and screen changes to seam.log".into()), debug_log, p),
                ],
                p,
            ),
            p,
        ))
        .child(div().text_size(px(12.)).text_color(p.muted).child("Seam follows your Windows light or dark mode"))
}
