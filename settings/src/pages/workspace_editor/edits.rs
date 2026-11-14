//! Changing a workspace's rows and sending it to the tray.

use super::*;
use crate::pages::workspaces::all;
use crate::panel::{error_text, refusal};
use serde_json::json;

// ---- Saving

fn by_id(panel: &Panel, id: &str) -> Option<Workspace> {
    all(panel).into_iter().find(|w| w.id == id)
}

/// Show the workspace as it will be saved, before the tray answers.
fn show_locally(panel: &mut Panel, ws: &Workspace) {
    let sig = desk_sig(panel);
    let Some(desk) = panel.config.desks.get_mut(&sig) else { return };
    if let Some(slot) = desk.workspaces.iter_mut().find(|v| workspace::parse(v).is_some_and(|w| w.id == ws.id)) {
        *slot = workspace::to_value(ws);
    }
}

type Then = Box<dyn FnOnce(&mut Panel, &mut Window, &mut Context<Panel>)>;

/// Save a workspace through the tray, which checks it. `then` runs once it's saved.
fn send(panel: &mut Panel, ws: Workspace, then: Option<Then>, window: &mut Window, cx: &mut Context<Panel>) {
    show_locally(panel, &ws);
    cx.notify();
    // `zones` is sent even when empty, or the tray keeps the layouts it has.
    let mut sent = workspace::to_value(&ws);
    sent["zones"] = json!(ws.zones);
    let params = json!({ "id": ws.id, "workspace": sent });
    panel.call("workspace.update", params, window, cx, move |panel, result, window, cx| match (result, then) {
        (Err(e), _) => {
            window.push_notification(refusal("Workspace left as it was", &error_text(&e)), cx);
            panel.reload_config(window, cx);
        }
        (Ok(_), Some(then)) => then(panel, window, cx),
        (Ok(_), None) => panel.reload_config(window, cx),
    });
}

/// Change one workspace of this desk. It shows at once and the tray has the last
/// word. With a `note`, that change can be undone for 8 s.
pub fn update(
    panel: &mut Panel,
    id: &str,
    note: Option<String>,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Workspace),
) {
    let Some(before) = by_id(panel, id) else { return };
    let mut after = before.clone();
    change(&mut after);
    if after == before {
        return;
    }
    send(panel, after, None, window, cx);
    if let Some(note) = note {
        panel.offer_undo(note, move |panel, window, cx| send(panel, before.clone(), None, window, cx), window, cx);
    }
}

/// Change one row, with Undo.
pub(super) fn update_row(
    panel: &mut Panel,
    id: &str,
    i: usize,
    note: impl FnOnce(&str) -> String,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Rule),
) {
    let Some(title) = by_id(panel, id).and_then(|ws| ws.windows.get(i).map(app_title)) else { return };
    update(panel, id, Some(note(&title)), window, cx, |ws| {
        if let Some(rule) = ws.windows.get_mut(i) {
            change(rule);
        }
    });
}

pub(super) fn seconds(ms: u32) -> String {
    match ms % 1000 {
        0 => format!("{} s", ms / 1000),
        _ => format!("{:.1} s", ms as f32 / 1000.),
    }
}

pub(super) fn set_delay(panel: &mut Panel, ms: u32, window: &mut Window, cx: &mut Context<Panel>) {
    let Some((id, i)) = panel.editing.expanded.clone() else { return };
    let ms = workspace::tame_delay(ms);
    let note = |title: &str| match ms {
        0 => format!("{title} opens at once"),
        ms => format!("{title} opens after {}", seconds(ms)),
    };
    update_row(panel, &id, i, note, window, cx, |rule| rule.delay_ms = ms);
}

/// Move a row to another place in the start order.
pub(super) fn move_row(
    panel: &mut Panel,
    id: &str,
    from: usize,
    to: usize,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some(ws) = by_id(panel, id) else { return };
    if from == to || from >= ws.windows.len() || to >= ws.windows.len() {
        return;
    }
    let title = app_title(&ws.windows[from]);
    if panel.editing.expanded.as_ref().is_some_and(|(e, row)| e == id && *row == from) {
        panel.editing.expanded = Some((id.to_string(), to));
    }
    update(panel, id, Some(format!("Moved {title} to place {}", to + 1)), window, cx, |ws| {
        let rule = ws.windows.remove(from);
        ws.windows.insert(to, rule);
    });
}

pub(super) fn remove_row(panel: &mut Panel, id: &str, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(title) = by_id(panel, id).and_then(|ws| ws.windows.get(i).map(app_title)) else { return };
    panel.editing.expanded = None;
    update(panel, id, Some(format!("Removed {title}")), window, cx, |ws| {
        ws.windows.remove(i);
    });
}

/// Add an app the picker offered, at the end of the start order. One picked from the
/// windows open now, with "Remember folders and pages" on, then has the tray read that
/// window's folder or page into the new row (D330). Only the tray reads it.
pub fn add_app(
    panel: &mut Panel,
    id: &str,
    rule: Rule,
    hwnd: Option<isize>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let title = app_title(&rule);
    let Some(before) = by_id(panel, id) else { return };
    let row = before.windows.len();
    // The tray checks the row is still this app's before it takes the place.
    let (exe, class) = (rule.exe.clone(), rule.class.clone());
    let mut after = before.clone();
    after.windows.push(rule);
    let read = hwnd.filter(|_| panel.config.workspaces_remember_places).map(|hwnd| -> Then {
        let params = json!({ "into": id, "row": row, "hwnd": hwnd, "exe": exe, "class": class });
        // A window with no folder or page to remember is no news, so the answer only reloads.
        Box::new(move |panel, window, cx| {
            panel.call("workspace.capture", params, window, cx, |panel, _, window, cx| panel.reload_config(window, cx))
        })
    });
    send(panel, after, read, window, cx);
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        send(panel, before.clone(), None, window, cx)
    };
    panel.offer_undo(format!("Added {title}"), undo, window, cx);
}

/// The exe name the "Never manage" list holds for this row, if it has one. A Store
/// app added from Installed has no exe yet.
pub(super) fn exe_name(rule: &Rule) -> Option<String> {
    let name = rule.exe.rsplit('\\').next().unwrap_or(&rule.exe).to_lowercase();
    (!name.is_empty()).then_some(name)
}

/// Remove the row and add its app to the list the zones and capture leave alone.
/// Undo puts both back.
pub(super) fn never_include(panel: &mut Panel, id: &str, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(before) = by_id(panel, id) else { return };
    let Some((rule, name)) = before.windows.get(i).and_then(|r| Some((r, exe_name(r)?))) else { return };
    let title = app_title(rule);
    let mut after = before.clone();
    after.windows.remove(i);
    let old_list = panel.config.zones.exclude.clone();
    let mut list = old_list.clone();
    if !list.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
        list.push(name);
    }
    panel.editing.expanded = None;
    let exclude = move |list: Vec<String>| -> Then {
        Box::new(move |panel, window, cx| panel.patch(json!({ "zones": { "exclude": list } }), window, cx))
    };
    send(panel, after, Some(exclude(list)), window, cx);
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        send(panel, before.clone(), Some(exclude(old_list.clone())), window, cx);
    };
    panel.offer_undo(format!("Never including {title}"), undo, window, cx);
}

/// "Where it is now": the tray captures the row's window and saves its place.
pub(super) fn where_it_is(panel: &mut Panel, ws: &Workspace, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let (before, title) = (ws.clone(), ws.windows.get(i).map(app_title).unwrap_or_default());
    let params = json!({ "into": ws.id, "row": i });
    panel.call("workspace.capture", params, window, cx, move |panel, result, window, cx| match result {
        Ok(_) => {
            panel.reload_config(window, cx);
            let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                send(panel, before.clone(), None, window, cx)
            };
            panel.offer_undo(format!("{title} goes where it is now"), undo, window, cx);
        }
        Err(e) => {
            window.push_notification(refusal(&format!("Where {title} goes stays as it was"), &error_text(&e)), cx)
        }
    });
}
