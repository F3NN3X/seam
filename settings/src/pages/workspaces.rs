//! Workspaces: save which apps are where on a desk, and switch between them.

use crate::icons;
use crate::pages::{app_picker, shortcuts, workspace_editor};
use crate::panel::{Panel, error_text, refusal};
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui::{self, card};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use list::{Marks, Moved};
use seam::state::Screen;
use seam::workspace::{self, Export, RowState, Rule, ScreenInfo, Target, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

mod close;
mod list;
mod review;
mod sections;

pub use close::ask as ask_to_close;

const LIST_WIDTH: f32 = 200.;

// ---- What is saved. The config keeps workspaces as raw JSON. Creating, deleting and
// importing send the whole list; the editor sends one workspace.

pub fn desk_sig(panel: &Panel) -> String {
    panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default()
}

/// The workspace this desk takes as in use at start when none is remembered.
fn default_id(panel: &Panel) -> Option<String> {
    panel.config.desks.get(&desk_sig(panel)).and_then(|d| d.default_workspace.clone())
}

fn saved(panel: &Panel) -> Vec<Value> {
    panel.config.desks.get(&desk_sig(panel)).map(|d| d.workspaces.clone()).unwrap_or_default()
}

pub fn all(panel: &Panel) -> Vec<Workspace> {
    workspace::list(&saved(panel))
}

/// The workspace on show: the one picked, or else the first.
fn selected(panel: &Panel) -> Option<Workspace> {
    let all = all(panel);
    let picked = panel.workspace.as_ref().and_then(|id| all.iter().find(|w| w.id == *id));
    picked.or(all.first()).cloned()
}

/// The id of the workspace on show, for its live status.
pub fn shown_id(panel: &Panel) -> Option<String> {
    selected(panel).map(|w| w.id)
}

fn save(panel: &mut Panel, list: Vec<Value>, window: &mut Window, cx: &mut Context<Panel>) {
    let sig = desk_sig(panel);
    panel.patch(json!({ "desks": { sig: { "workspaces": list } } }), window, cx);
}

/// Change one workspace's settings, with no Undo: its name, shortcut and switches.
fn edit(
    panel: &mut Panel,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Workspace),
) {
    workspace_editor::update(panel, id, None, window, cx, change);
}

/// Set or clear a workspace's shortcut. The tray registers it.
pub fn set_hotkey(panel: &mut Panel, id: &str, combo: Option<String>, window: &mut Window, cx: &mut Context<Panel>) {
    edit(panel, id, window, cx, |ws| ws.hotkey = combo);
}

/// The workspaces on this desk, for the Shortcuts page.
pub fn on_desk(panel: &Panel) -> Vec<Workspace> {
    all(panel)
}

/// Move a workspace to number `to` of the list, which is the number Add window to workspace
/// counts by. Undo moves it back.
fn move_workspace(panel: &mut Panel, id: &str, to: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let list = all(panel);
    let Some(from) = list.iter().position(|w| w.id == id) else { return };
    if from + 1 == to || panel.moving {
        return;
    }
    // Without a connection `call` never answers, so there is nothing to wait for.
    panel.moving = panel.client.is_some();
    let (name, id) = (list[from].name.clone(), id.to_string());
    panel.call("workspace.move", json!({ "id": id, "to": to }), window, cx, move |panel, result, window, cx| {
        panel.moving = false;
        match result {
            Ok(_) => {
                // The list shows the new order now, not after the reload, so the next move reads it right.
                let sig = desk_sig(panel);
                if let Ok(moved) = workspace::moved_to(&saved(panel), &id, to)
                    && let Some(desk) = panel.config_mut().desks.get_mut(&sig)
                {
                    desk.workspaces = moved;
                }
                panel.focus_workspace = Some(id.clone());
                panel.reload_config(window, cx);
                let (id, back) = (id.clone(), from + 1);
                let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                    let params = json!({ "id": id, "to": back });
                    panel.call("workspace.move", params, window, cx, |panel, result, window, cx| {
                        if let Err(e) = result {
                            window.push_notification(refusal("Workspace not moved", &error_text(&e)), cx);
                        }
                        panel.reload_config(window, cx);
                    });
                };
                panel.offer_undo(format!("Moved {name} to {to}"), undo, window, cx);
            }
            Err(e) => window.push_notification(refusal("Workspace not moved", &error_text(&e)), cx),
        }
    });
}

/// A workspace was dropped just before the one at place `before` of the list, or last when
/// that is the count.
fn dropped(panel: &mut Panel, id: &str, before: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let list = all(panel);
    let Some(from) = list.iter().position(|w| w.id == id) else { return };
    move_workspace(panel, id, list::number_after(from, before, list.len()), window, cx);
}

fn start_naming(panel: &mut Panel, cx: &mut Context<Panel>) {
    panel.naming = true;
    panel.focus_name = true;
    cx.notify();
}

/// Enter or leaving the field keeps the name. A name already in use gets " 2".
pub fn finish_naming(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    // Until the field has been filled in and focused it hasn't been used.
    if !panel.naming || panel.focus_name {
        return;
    }
    panel.naming = false;
    let name = panel.name_input.read(cx).value().trim().to_string();
    if let (Some(ws), false) = (selected(panel), name.is_empty()) {
        let others: Vec<String> = all(panel).into_iter().filter(|w| w.id != ws.id).map(|w| w.name).collect();
        let name = workspace::unique_name(&name, &others);
        if name != ws.name {
            edit(panel, &ws.id, window, cx, |ws| ws.name = name);
        }
    }
    cx.notify();
}

// ---- Actions

/// Capture current windows: list them first, then save the ones ticked.
pub fn capture(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    review::open(panel, window, cx);
}

fn update_from_screen(panel: &Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    review::update(panel, ws, window, cx);
}

/// A copy of the workspace with a name of its own, and no shortcut.
fn duplicate(panel: &mut Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    let before = saved(panel);
    let existing = workspace::list(&before);
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let name = workspace::unique_name(&format!("{} copy", ws.name), &names);
    let copy = Workspace { id: workspace::new_id(&ids), name: name.clone(), hotkey: None, ..ws.clone() };
    let mut list = before.clone();
    list.push(workspace::to_value(&copy));
    shortcuts::stop_recording(panel);
    panel.workspace = Some(copy.id.clone());
    save(panel, list, window, cx);
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        shortcuts::stop_recording(panel);
        panel.workspace = None;
        save(panel, before.clone(), window, cx)
    };
    panel.offer_undo(format!("Duplicated as {name}"), undo, window, cx);
}

fn create_empty(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let mut list = saved(panel);
    let existing = workspace::list(&list);
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let created = Workspace { id: workspace::new_id(&ids), name: workspace::next_name(&names), ..Workspace::default() };
    shortcuts::stop_recording(panel);
    panel.workspace = Some(created.id.clone());
    list.push(workspace::to_value(&created));
    save(panel, list, window, cx);
    start_naming(panel, cx);
}

/// The workspace taken as in use at start, or none. The tray saves it.
fn set_default(panel: &mut Panel, id: Option<String>, window: &mut Window, cx: &mut Context<Panel>) {
    panel.call("workspace.set_default", json!({ "id": id }), window, cx, |panel, result, window, cx| {
        if let Err(e) = result {
            window.push_notification(refusal("Default left as it was", &error_text(&e)), cx);
        }
        panel.reload_config(window, cx);
    });
}

fn delete(panel: &mut Panel, id: &str, window: &mut Window, cx: &mut Context<Panel>) {
    let before = saved(panel);
    let name = workspace::list(&before).into_iter().find(|w| w.id == id).map(|w| w.name).unwrap_or_default();
    let rest = before.iter().filter(|v| workspace::parse(v).is_none_or(|w| w.id != id)).cloned().collect();
    shortcuts::stop_recording(panel);
    panel.workspace = None;
    save(panel, rest, window, cx);
    panel.offer_undo(
        format!("Deleted {name}"),
        move |panel, window, cx| save(panel, before.clone(), window, cx),
        window,
        cx,
    );
}

/// The connected screens as an export file describes them.
fn screen_infos(panel: &Panel) -> BTreeMap<String, ScreenInfo> {
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    let info = |s: &Screen| {
        let size = panel.mons.iter().find(|m| m.id == s.id).map(|m| (m.rect.width(), m.rect.height()));
        let (width, height) = size.unwrap_or_default();
        (s.id.clone(), ScreenInfo { number: s.number, name: s.name.clone(), width, height })
    };
    screens.iter().map(info).collect()
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn export(panel: &Panel, mut ws: Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    // Folders and pages say where I've been, so they stay on this PC (D330).
    ws.windows.iter_mut().for_each(|r| (r.place, r.open_with) = (None, None));
    let refers = |id: &String| ws.windows.iter().any(|r| r.monitor == *id) || ws.zones.contains_key(id);
    let used: BTreeMap<String, ScreenInfo> = screen_infos(panel).into_iter().filter(|(id, _)| refers(id)).collect();
    let name: String = ws.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '-' }).collect();
    let file = Export { seam_workspace: Export::VERSION, workspace: ws, screens: used };
    let chosen = cx.prompt_for_new_path(&home(), Some(&format!("{name}.seam-workspace.json")));
    cx.spawn_in(window, async move |_, cx| {
        let Ok(Ok(Some(path))) = chosen.await else { return };
        let written = serde_json::to_string_pretty(&file).map_err(std::io::Error::other);
        let written = written.and_then(|text| std::fs::write(&path, text));
        let _ = cx.update(|window, cx| match written {
            Ok(()) => window.push_notification(Notification::new().message("Exported the workspace"), cx),
            Err(e) => window.push_notification(refusal("Workspace not exported", &e.to_string()), cx),
        });
    })
    .detach();
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
        let parsed = read.and_then(|bytes| serde_json::from_slice::<Export>(&bytes).map_err(|e| e.to_string()));
        let _ = this.update_in(cx, move |panel, window, cx| match parsed {
            Ok(file) if file.seam_workspace == Export::VERSION => add_imported(panel, file, window, cx),
            Ok(_) => window.push_notification(refusal("That file can't be imported", "It's from a newer Seam"), cx),
            Err(e) => window.push_notification(refusal("That file can't be imported", &e), cx),
        });
    })
    .detach();
}

/// Put an imported workspace on this desk, with its windows on the screens that match.
fn add_imported(panel: &mut Panel, file: Export, window: &mut Window, cx: &mut Context<Panel>) {
    let mut list = saved(panel);
    let existing = workspace::list(&list);
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let imported = workspace::import(file, &screen_infos(panel), &names, &ids);
    let note = Notification::new().message(format!("Imported {}", imported.name));
    shortcuts::stop_recording(panel);
    panel.workspace = Some(imported.id.clone());
    list.push(workspace::to_value(&imported));
    save(panel, list, window, cx);
    window.push_notification(note, cx);
}

// ---- What is drawn

/// "Windows Terminal" from `WindowsTerminal.exe`, "Firefox" from `firefox.exe`.
pub fn app_title(rule: &Rule) -> String {
    let stem = rule.exe.rsplit('\\').next().unwrap_or(&rule.exe);
    let stem = stem.strip_suffix(".exe").or_else(|| stem.strip_suffix(".EXE")).unwrap_or(stem);
    let mut title = String::new();
    let mut before = ' ';
    for c in stem.chars() {
        if c.is_uppercase() && before.is_lowercase() {
            title.push(' ');
        }
        title.push(if title.is_empty() { c.to_ascii_uppercase() } else { c });
        before = c;
    }
    if title.is_empty() { rule.title_hint.clone() } else { title }
}

/// How many windows of the workspace aren't where it puts them, as far as the tray last said.
fn moved_windows(panel: &Panel, ws: &Workspace) -> usize {
    let moved =
        |i: &usize| matches!(workspace_editor::row_state(panel, ws, *i), Some(RowState::Moved | RowState::Minimized));
    (0..ws.windows.len()).filter(moved).count()
}

/// "1, 2 and 4".
fn numbers(of: &[u32]) -> String {
    let words: Vec<String> = of.iter().map(u32::to_string).collect();
    match words.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

/// The line under the name: which screens have zones, and how many windows have moved.
fn summary(panel: &Panel, ws: &Workspace) -> String {
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    let mut used: Vec<u32> = ws
        .windows
        .iter()
        .filter(|r| matches!(r.target, Target::Zone { .. }))
        .filter_map(|r| screens.iter().find(|s| s.id == r.monitor).map(|s| s.number))
        .collect();
    used.sort_unstable();
    used.dedup();
    let zones = match used.as_slice() {
        [] if ws.windows.is_empty() => "No windows yet".to_string(),
        [] => format!("{} window{}", ws.windows.len(), if ws.windows.len() == 1 { "" } else { "s" }),
        [only] => format!("Zones on screen {only}"),
        many => format!("Zones on screens {}", numbers(many)),
    };
    match moved_windows(panel, ws) {
        0 => zones,
        1 => format!("{zones}. 1 window isn't where this workspace puts it"),
        n => format!("{zones}. {n} windows aren't where this workspace puts them"),
    }
}

/// Whether this is the workspace in use.
fn in_use(panel: &Panel, ws: &Workspace) -> bool {
    panel.state.as_ref().and_then(|s| s.workspace.as_ref()).is_some_and(|w| w.id == ws.id)
}

/// Move the windows of the workspace in use back to where it puts them.
fn put_back(panel: &mut Panel, count: usize, window: &mut Window, cx: &mut Context<Panel>) {
    panel.call("workspace.snap_back", json!({}), window, cx, move |_, result, window, cx| match result {
        Ok(_) => {
            let said = match count {
                0 => "Put windows back".to_string(),
                1 => "Put 1 window back".to_string(),
                n => format!("Put {n} windows back"),
            };
            window.push_notification(Notification::new().message(said), cx);
        }
        Err(e) => window.push_notification(refusal("Windows left as they were", &error_text(&e)), cx),
    });
}

/// Switch to the workspace. Undo goes back to the one that was in use, when there was another.
fn switch_to(panel: &mut Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    let before = panel.state.as_ref().and_then(|s| s.workspace.as_ref()).map(|w| w.id.clone());
    // The notice comes when the switch ends, so what it says is ready before the tray starts.
    panel.expect_switch(ws.id.clone(), ws.name.clone(), before.is_some_and(|id| id != ws.id));
    panel.call("workspace.switch", json!({ "id": ws.id }), window, cx, move |panel, result, window, cx| {
        if let Err(e) = result {
            panel.asked = None;
            window.push_notification(refusal("Seam can't do that", &error_text(&e)), cx);
        }
    });
}

/// The one primary button of the card: "Put windows back" while the workspace is in use and its
/// windows moved, else "Switch now".
fn primary_action(panel: &Panel, ws: &Workspace, cx: &mut Context<Panel>) -> Button {
    let moved = moved_windows(panel, ws);
    let drifted = moved > 0 || workspace_editor::zones_moved(panel, ws);
    let ws = ws.clone();
    if in_use(panel, &ws) && drifted {
        return ui::primary("snap-back", "Put windows back")
            .on_click(cx.listener(move |panel, _, window, cx| put_back(panel, moved, window, cx)));
    }
    ui::primary("switch-workspace", "Switch now")
        .on_click(cx.listener(move |panel, _, window, cx| switch_to(panel, &ws, window, cx)))
}

fn detail(panel: &mut Panel, ws: Workspace, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let header = detail_header(panel, &ws, p, window, cx);

    let mut rows = Vec::new();
    rows.extend(problems_note(panel, &ws, p, cx));
    rows.push(header);
    rows.extend(workspace_editor::rows(panel, &ws, window, cx));
    let id = ws.id.clone();
    let add_app: app_picker::Add =
        Rc::new(move |panel, rule, hwnd, window, cx| workspace_editor::add_app(panel, &id, rule, hwnd, window, cx));
    let add = app_picker::button(panel, &ws.windows, "In this workspace", add_app, cx);
    let hint = ui::description("Drag an app onto another screen to move it there", p);
    rows.push(
        div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_12))
            .px(px(tokens::SPACE_16))
            .py(px(tokens::SPACE_12))
            .child(add)
            .child(hint)
            .into_any_element(),
    );
    let switching = sections::switch_section(panel, &ws, p, window, cx);
    div().flex().flex_col().gap(px(ui::SECTION_GAP)).flex_1().min_w_0().child(card(rows, p)).child(switching)
}

/// The note at the top of the card when the workspace's last switch had problems (D713): it
/// goes when the workspace is switched again or its button is pressed.
fn problems_note(panel: &Panel, ws: &Workspace, p: Palette, cx: &mut Context<Panel>) -> Option<AnyElement> {
    let (id, last) = panel.last_switch.as_ref()?;
    if *id != ws.id || panel.note_gone.as_ref() == Some(&(ws.id.clone(), last.at)) {
        return None;
    }
    let count = last.problems().len();
    if count == 0 {
        return None;
    }
    let at = last.at;
    let again = ui::secondary("switch-again", "Switch again", cx).on_click({
        let ws = ws.clone();
        cx.listener(move |panel, _, window, cx| {
            panel.note_gone = Some((ws.id.clone(), at));
            switch_to(panel, &ws, window, cx);
        })
    });
    let text = format!("{} had a problem the last time you switched", seam::workspace::report::apps(count));
    Some(
        div()
            .px(px(tokens::SPACE_16))
            .py(px(tokens::SPACE_12))
            .child(ui::note(text, Some(again), p))
            .into_any_element(),
    )
}

/// The name, with Update from screen, the primary action and the more menu.
fn detail_header(
    panel: &mut Panel,
    ws: &Workspace,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let summary = summary(panel, ws);

    // A new or renamed workspace opens its name for editing.
    if panel.focus_name {
        let name = ws.name.clone();
        panel.name_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        panel.focus_name = false;
    }
    let title = if panel.naming {
        div().w(px(220.)).child(Input::new(&panel.name_input)).into_any_element()
    } else {
        text_style(div(), tokens::CARD_TITLE_STRONG).truncate().child(ws.name.clone()).into_any_element()
    };
    let update = ui::secondary("update-workspace", "Update from screen", cx).on_click({
        let ws = ws.clone();
        cx.listener(move |panel, _, window, cx| update_from_screen(panel, &ws, window, cx))
    });
    let primary = primary_action(panel, ws, cx);
    let more = more_button(panel, ws, cx);
    div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .px(px(tokens::SPACE_16))
        .py(px(tokens::SPACE_12))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(tokens::SPACE_2))
                .flex_1()
                .min_w_0()
                .child(title)
                .child(ui::description(summary, p)),
        )
        .child(div().flex_none().child(update))
        .child(div().flex_none().child(primary))
        .child(div().flex_none().child(more))
        .into_any_element()
}

/// The menu behind the ellipsis: rename, default, duplicate, open alongside, minimize, close, export,
/// import, delete.
fn more_button(panel: &Panel, ws: &Workspace, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let view = cx.entity().downgrade();
    let more_ws = ws.clone();
    let is_default = default_id(panel).as_deref() == Some(ws.id.as_str());
    let open = close::any_open(panel, ws);
    ui::secondary("more-workspace", "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {}", ws.name))
        .dropdown_menu(move |menu, _, _| {
            let (rename, copying, exporting, importing, deleting) =
                (view.clone(), view.clone(), view.clone(), view.clone(), view.clone());
            let (to_copy, to_export, id) = (more_ws.clone(), more_ws.clone(), more_ws.id.clone());
            let (alongside, alongside_id) = (view.clone(), more_ws.id.clone());
            let (defaulting, default_to) = (view.clone(), (!is_default).then(|| more_ws.id.clone()));
            let default_label = if is_default { "Stop being the default" } else { "Set as default" };
            let menu = menu
                .item(PopupMenuItem::new("Rename").on_click(move |_, _, cx| {
                    let _ = rename.update(cx, start_naming);
                }))
                .item(PopupMenuItem::new(default_label).on_click(move |_, window, cx| {
                    let id = default_to.clone();
                    let _ = defaulting.update(cx, |panel, cx| set_default(panel, id, window, cx));
                }))
                .item(PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
                    let ws = to_copy.clone();
                    let _ = copying.update(cx, |panel, cx| duplicate(panel, &ws, window, cx));
                }))
                .item(PopupMenuItem::new("Open alongside").on_click(move |_, window, cx| {
                    let id = alongside_id.clone();
                    let _ = alongside.update(cx, |panel, cx| {
                        panel.command("workspace.switch", json!({ "id": id, "alongside": true }), window, cx);
                    });
                }));
            let menu = close::menu_items(menu, &view, &more_ws, open);
            menu.item(PopupMenuItem::new("Export…").on_click(move |_, window, cx| {
                let to_export = to_export.clone();
                let _ = exporting.update(cx, |panel, cx| export(panel, to_export, window, cx));
            }))
            .item(PopupMenuItem::new("Import…").on_click(move |_, window, cx| {
                let _ = importing.update(cx, |_, cx| import(window, cx));
            }))
            .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                let id = id.clone();
                let _ = deleting.update(cx, |panel, cx| delete(panel, &id, window, cx));
            }))
        })
}

fn empty_state(panel: &Panel, p: Palette, cx: &mut Context<Panel>) -> Div {
    let capture = ui::primary("capture-empty", "Capture current windows")
        .on_click(cx.listener(|panel, _, window, cx| capture(panel, window, cx)));
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(tokens::SPACE_12))
        .p(px(tokens::SPACE_32))
        .border_1()
        .border_dashed()
        .border_color(p.hairline_strong)
        .rounded(px(tokens::RADIUS_12))
        .child(text_style(div(), tokens::CARD_TITLE).child(format!("No workspaces for {} yet", panel.desk_name())))
        .child(ui::description("Arrange your windows the way you like them, then capture them", p))
        .child(capture)
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let list = all(panel);
    let header = ui::page_header(
        "Workspaces",
        "Save which apps go in which zone, then switch with one shortcut",
        Some("Workspaces"),
        p,
        cx,
    );
    if list.is_empty() {
        return div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header).child(empty_state(panel, p, cx));
    }
    let capture = ui::secondary("capture", "Capture current windows", cx)
        .on_click(cx.listener(|panel, _, window, cx| capture(panel, window, cx)));
    let rules: Vec<Rule> = list.iter().flat_map(|ws| ws.windows.iter().cloned()).collect();
    icons::request(panel, &rules, cx);
    let active = panel.state.as_ref().and_then(|s| s.workspace.as_ref()).map(|w| (w.id.clone(), w.drifted));
    let default = default_id(panel);
    let current = selected(panel);
    // A workspace on show for the first time asks how its apps are doing.
    let known = panel.status.as_ref().map(|(id, _)| id.as_str());
    if current.as_ref().is_some_and(|c| known != Some(c.id.as_str())) && panel.asking_status.is_none() {
        panel.ask_status(cx);
    }
    let count = list.len();
    let cards: Vec<AnyElement> = list
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let picked = current.as_ref().is_some_and(|c| c.id == ws.id);
            let moved = active.as_ref().filter(|(id, _)| *id == ws.id).map(|(_, drifted)| match (drifted, picked) {
                (false, _) => Moved::No,
                (true, true) => match moved_windows(panel, ws) {
                    0 => Moved::Some,
                    n => Moved::Count(n),
                },
                (true, false) => Moved::Some,
            });
            let focus = panel.focus_workspace.as_deref() == Some(ws.id.as_str());
            let marks = Marks { picked, active: moved, default: default.as_deref() == Some(ws.id.as_str()), focus };
            list::item((i, count), ws, &panel.icons, marks, p, window, cx)
        })
        .collect();
    panel.focus_workspace = None;
    let sidebar = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(tokens::SPACE_8))
        .w(px(LIST_WIDTH))
        .children(cards)
        .child(list::new_empty(count.checked_sub(1), p, window, cx))
        .when(count >= 2, |d| {
            d.child(
                text_style(div(), tokens::LABEL)
                    .text_color(p.text_secondary)
                    .child("Drag to change the order. Add window to workspace 1 to 9 counts in this order"),
            )
        });
    let body = div().flex().items_start().gap(px(tokens::SPACE_16)).child(sidebar);
    let body = match current {
        Some(ws) => body.child(detail(panel, ws, window, cx)),
        None => body,
    };
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(header.child(capture))
        .child(body)
        .child(sections::all_section(panel, p, window, cx))
}
