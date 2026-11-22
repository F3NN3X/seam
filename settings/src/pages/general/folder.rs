//! Keeping the settings in a synced folder (brief G): the states of the Settings folder row, its
//! menu, the notes about the folder, and the dialog for a folder that already has settings.

use super::*;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::menu::PopupMenu;

/// Where the settings live, as the tray last said. Without the tray the pointer in the config is all there is.
fn state(panel: &Panel) -> FolderState {
    match (&panel.state, &panel.config.config_folder) {
        (Some(state), _) => state.config_sync.state,
        (None, Some(_)) => FolderState::Synced,
        (None, None) => FolderState::Local,
    }
}

/// The folder the settings are kept in, if they are.
pub(super) fn chosen(panel: &Panel) -> Option<String> {
    panel.config.config_folder.clone().filter(|_| state(panel) != FolderState::Local)
}

/// The path of the Settings folder row in `MONO`, then how the folder is doing and why a folder was refused.
pub(super) fn about(panel: &Panel, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let path = text_style(div(), tokens::MONO)
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(p.text_secondary)
        .child(folder_label(chosen(panel).as_deref()));
    let line = match state(panel) {
        FolderState::Local => None,
        FolderState::Synced => Some("Synced. Shortcuts and this PC's own settings stay here"),
        FolderState::Unavailable => Some("Not available. Using the copy on this PC"),
    };
    div()
        .flex()
        .flex_col()
        .child(path)
        .children(line.map(|line| ui::description(line, p)))
        .children(
            panel
                .folder_error
                .clone()
                .map(|error| text_style(div(), tokens::LABEL).text_color(p.negative).child(error)),
        )
        .into_any_element()
}

/// Open, and the `…` menu.
pub(super) fn control(panel: &Panel, cx: &mut Context<Panel>) -> Div {
    let target = chosen(panel);
    div()
        .flex()
        .gap(px(8.))
        .child(ui::secondary("open-folder", "Open", cx).on_click(move |_, _, _| open_folder(target.as_deref())))
        .child(menu(panel, cx))
}

/// The `…` next to Open: where the settings are kept, then Export… and Import….
fn menu(panel: &Panel, cx: &mut Context<Panel>) -> AnyElement {
    let weak = cx.entity().downgrade();
    let in_folder = chosen(panel).is_some();
    let menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<_>| {
        let (export_view, import_view, pick_view, off_view) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
        let pick = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let _ = pick_view.update(cx, |_, cx| choose(window, cx));
        };
        let menu = if in_folder {
            menu.item(PopupMenuItem::new("Change folder…").on_click(pick)).item(
                PopupMenuItem::new("Keep settings on this PC only").on_click(move |_, window, cx| {
                    let _ = off_view.update(cx, |panel, cx| keep_here(panel, window, cx));
                }),
            )
        } else {
            menu.item(PopupMenuItem::new("Keep settings in a folder…").on_click(pick))
        };
        menu.separator()
            .item(PopupMenuItem::new("Export…").on_click(move |_, window, cx| {
                let _ = export_view.update(cx, |panel, cx| export(panel, window, cx));
            }))
            .item(PopupMenuItem::new("Import…").on_click(move |_, window, cx| {
                let _ = import_view.update(cx, |_, cx| import(window, cx));
            }))
    };
    ui::secondary("folder-more", "…", cx)
        .accessibility_label("Settings folder, export or import settings")
        .dropdown_menu(menu)
        .into_any_element()
}

/// Notes under the page title about the settings folder: it isn't there, its file can't be used, two
/// PCs changed the same thing, or a sync app made its own copy.
pub(super) fn notes(panel: &Panel, p: Palette, cx: &mut Context<Panel>) -> Vec<Div> {
    let mut notes = Vec::new();
    if chosen(panel).is_none() {
        return notes;
    }
    if state(panel) == FolderState::Unavailable {
        let choose_it = ui::secondary("folder-choose", "Choose folder", cx)
            .on_click(cx.listener(|_, _, window, cx| choose(window, cx)));
        notes.push(ui::note(
            "Your settings folder isn't available, so Seam is using the copy on this PC. Changes go to the folder when it's back",
            Some(choose_it),
            p,
        ));
    }
    match panel.folder.problem {
        Some(Problem::Newer) => notes.push(ui::note(
            "That folder has settings from a newer Seam. Update Seam on this PC to use them",
            None,
            p,
        )),
        Some(Problem::Unreadable) => notes.push(ui::note(
            "Seam can't read the settings file in your folder, so it keeps the settings it has",
            None,
            p,
        )),
        None => {}
    }
    if let Some(kept) = panel.folder.kept.clone().filter(|k| panel.kept_seen.as_ref() != Some(k)) {
        let text = format!(
            "Your settings changed on another PC at the same time as here. Seam used the other PC's and kept this PC's in {kept}"
        );
        let open = ui::secondary("kept-open", "Open folder", cx).on_click(cx.listener(move |this, _, _, cx| {
            show_file(&kept);
            this.kept_seen = Some(kept.clone());
            cx.notify();
        }));
        notes.push(ui::note(text, Some(open), p));
    }
    if !panel.folder.conflict_files.is_empty() {
        let folder = chosen(panel);
        let open =
            ui::secondary("copies-open", "Open folder", cx).on_click(move |_, _, _| open_folder(folder.as_deref()));
        notes.push(ui::note(
            "Your sync app made another copy of your settings in the folder. Seam keeps using config.json",
            Some(open),
            p,
        ));
    }
    notes
}

/// The system folder picker, then `config.set_folder` with what was picked.
fn choose(window: &mut Window, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Use folder".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.first().map(|p| p.display().to_string()) else { return };
            let _ = cx.update(|window, cx| {
                if let Some(view) = window.root::<Panel>().flatten() {
                    view.update(cx, |panel, cx| set_folder(panel, path, None, window, cx));
                }
            });
        })
        .detach();
}

/// The name of the last folder of a path, for what Seam says about it.
fn folder_name(path: &str) -> String {
    let path = std::path::Path::new(path);
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

/// Ask the tray to keep the settings in `path`. `take` says whose settings a folder that has some goes by.
fn set_folder(
    panel: &mut Panel,
    path: String,
    take: Option<&'static str>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    panel.folder_error = None;
    let mut params = json!({ "path": path });
    if let Some(take) = take {
        params["take"] = json!(take);
    }
    cx.notify();
    panel.call("config.set_folder", params, window, cx, move |panel, result, window, cx| match result {
        Ok(_) => {
            let message = format!("Your settings are now in {}", folder_name(&path));
            window.push_notification(Notification::new().message(message), cx);
            panel.reload_config(window, cx);
            panel.refresh_folder(cx);
        }
        Err(CallError::Rpc(e)) if e.message.starts_with("has_config") => {
            let who = serde_json::from_str::<Value>(e.message.trim_start_matches("has_config").trim()).ok();
            ask_which(&path, who, window, cx);
        }
        Err(e) => {
            panel.folder_error = Some(error_text(&e));
            cx.notify();
        }
    });
}

/// The folder already has settings: use them, or write this PC's over them. It can't be undone,
/// so a dialog says so, and closing it with Esc does neither.
fn ask_which(path: &str, who: Option<Value>, window: &mut Window, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    let name = folder_name(path);
    let saved = who.as_ref().and_then(|w| {
        let (pc, at) = (w["pc"].as_str()?, w["at"].as_u64()?);
        let date = seam::winutil::local_date(at).map(seam::locale::month_day_local)?;
        Some(format!(", saved {date} on {pc}"))
    });
    let body = format!(
        "{name} already has Seam settings{}. Shortcuts and this PC's own settings stay as they are either way",
        saved.unwrap_or_default()
    );
    let path = path.to_string();
    window.open_dialog(cx, move |dialog, _, cx| {
        let p = palette(cx);
        let (here_view, there_view) = (view.clone(), view.clone());
        let (here_path, there_path) = (path.clone(), path.clone());
        let keep_here =
            ui::secondary("sync-keep-here", "Keep this PC's settings", cx).on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let path = here_path.clone();
                let _ = here_view.update(cx, |panel, cx| set_folder(panel, path, Some("here"), window, cx));
            });
        let use_folder = ui::primary("sync-use-folder", "Use the folder's settings").on_click(move |_, window, cx| {
            window.close_dialog(cx);
            let path = there_path.clone();
            let _ = there_view.update(cx, |panel, cx| set_folder(panel, path, Some("folder"), window, cx));
        });
        dialog
            .w(px(420.))
            .title("Use the settings in this folder?")
            .child(text_style(div(), tokens::BODY).text_color(p.text_secondary).child(body.clone()))
            .footer(DialogFooter::new().child(keep_here).child(use_folder))
    });
}

/// "Keep settings on this PC only": no dialog, since choosing the folder again finds the file.
fn keep_here(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    panel.folder_error = None;
    panel.call("config.set_folder", json!({ "path": null }), window, cx, |panel, result, window, cx| {
        match result {
            Ok(_) => window.push_notification(Notification::new().message("Settings are on this PC only again"), cx),
            Err(e) => window.push_notification(refusal("Settings folder not changed", &error_text(&e)), cx),
        }
        panel.reload_config(window, cx);
        panel.refresh_folder(cx);
    });
}
