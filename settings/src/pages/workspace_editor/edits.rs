//! Changing a workspace's rows and sending it to the tray.

use super::groups::reorder;
use super::*;
use crate::pages::app_picker;
use crate::pages::workspaces::all;
use crate::panel::{error_text, refusal};
use gpui_kit::component::notification::Notification;
use serde_json::json;
use std::rc::Rc;

// ---- Saving

fn by_id(panel: &Panel, id: &str) -> Option<Workspace> {
    all(panel).into_iter().find(|w| w.id == id)
}

/// Show the workspace as it will be saved, before the tray answers.
fn show_locally(panel: &mut Panel, ws: &Workspace) {
    let sig = desk_sig(panel);
    let Some(desk) = panel.config_mut().desks.get_mut(&sig) else { return };
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

/// Change the rows' windows together, with Undo. The note names the first one's app.
pub(super) fn update_rows(
    panel: &mut Panel,
    id: &str,
    windows: &[usize],
    note: impl FnOnce(&str) -> String,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl Fn(&mut Rule),
) {
    let Some(title) = windows.first().and_then(|i| by_id(panel, id)?.windows.get(*i).map(app_title)) else { return };
    update(panel, id, Some(note(&title)), window, cx, |ws| {
        for i in windows {
            if let Some(rule) = ws.windows.get_mut(*i) {
                change(rule);
            }
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

/// Keep what was typed in "Open with" for the row the field shows. A refusal stays
/// under the field, and nothing is saved.
pub(super) fn set_open_with(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let Some((id, i)) = panel.editing.open_with_for.clone() else { return };
    let typed = panel.editing.open_with.read(cx).value().trim().to_string();
    let Some(rule) = by_id(panel, &id).and_then(|ws| ws.windows.get(i).cloned()) else { return };
    let open = match typed.as_str() {
        "" => None,
        typed => match workspace::open_with_for(&rule, typed) {
            Ok(workspace::OpenWith::Path(open) | workspace::OpenWith::Link(open)) => Some(open),
            Err(why) => {
                panel.editing.open_with_error = Some(why.to_string());
                return cx.notify();
            }
        },
    };
    panel.editing.open_with_error = None;
    let note = |title: &str| match &open {
        Some(_) => format!("{title} opens with what I typed"),
        None => format!("{title} opens as usual"),
    };
    update_row(panel, &id, i, note, window, cx, |rule| rule.open_with = open.clone());
}

/// Open the Other tab's dialog on the item of row `i`, to change it. It needs what the tab
/// read about this PC, which is asked for when the row opens.
pub(super) fn change_item(panel: &mut Panel, id: &str, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(ws) = by_id(panel, id) else { return };
    let Some(rule) = ws.windows.get(i).cloned() else { return };
    let Some(kind) = rule.item.as_ref().map(app_picker::items::Kind::of) else { return };
    let Some(extras) = panel.editing.picker.extras.clone() else {
        app_picker::read_installed(panel, cx);
        return window.push_notification(
            Notification::new().message("Still reading what this PC has, try again in a moment"),
            cx,
        );
    };
    let id = id.to_string();
    let done: app_picker::items::Done = Rc::new(move |panel, built, window, cx| {
        let note = |title: &str| format!("Changed what {title} opens");
        update_row(panel, &id, i, note, window, cx, |rule| {
            rule.exe = built.exe.clone();
            rule.aumid = built.aumid.clone();
            rule.item = built.item.clone();
        });
    });
    app_picker::items::open(kind, extras, (ws.name, Some(rule)), done, window, cx);
}

/// Move the windows of a row next to the window at `target`, in the start order.
pub(super) fn move_rows(
    panel: &mut Panel,
    id: &str,
    moving: &[usize],
    (target, after): (usize, bool),
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some(ws) = by_id(panel, id) else { return };
    let Some(title) = moving.first().and_then(|i| ws.windows.get(*i)).map(app_title) else { return };
    let mut moved = ws.windows.clone();
    reorder(&mut moved, moving, target, after);
    if moved == ws.windows {
        return;
    }
    panel.editing.expanded = None;
    update(panel, id, Some(format!("Moved {title}")), window, cx, |ws| ws.windows = moved);
}

/// Send the windows of a row to another screen. A zone the screen doesn't have becomes its full size.
pub(super) fn send_to_screen(
    panel: &mut Panel,
    id: &str,
    moving: &[usize],
    screen: &Screen,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let sig = desk_sig(panel);
    let layout = panel.config.desks.get(&sig).and_then(|d| d.zones.get(&screen.id)).and_then(zones::layout_of);
    let zones = layout.map_or(0, |l| zones::leaf_count(&l.tree) as u32);
    let note = |title: &str| format!("{title} goes to screen {}", screen.number);
    update_rows(panel, id, moving, note, window, cx, |rule| {
        let target = match &rule.target {
            Target::Zone { zone, span_to } if *zone <= zones && span_to.is_none_or(|to| to <= zones) => {
                rule.target.clone()
            }
            Target::Zone { .. } => Target::State { state: WinState::Max },
            other => other.clone(),
        };
        rule.monitor = screen.id.clone();
        rule.set_target(target);
    });
}

/// The workspace without the windows at `taken`.
fn without(ws: &Workspace, taken: &[usize]) -> Workspace {
    let windows = ws.windows.iter().enumerate().filter(|(i, _)| !taken.contains(i)).map(|(_, r)| r.clone());
    Workspace { windows: windows.collect(), ..ws.clone() }
}

/// Take the windows of a row out of the workspace.
pub(super) fn remove_rows(
    panel: &mut Panel,
    id: &str,
    windows: &[usize],
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some(ws) = by_id(panel, id) else { return };
    let Some(title) = windows.first().and_then(|i| ws.windows.get(*i)).map(app_title) else { return };
    panel.editing.expanded = None;
    update(panel, id, Some(format!("Removed {title}")), window, cx, |ws| *ws = without(&ws.clone(), windows));
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

/// Remove the rows of this app and add it to the list the zones and capture leave alone.
/// Undo puts both back.
pub(super) fn never_include(
    panel: &mut Panel,
    id: &str,
    windows: &[usize],
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some(before) = by_id(panel, id) else { return };
    let Some(rule) = windows.first().and_then(|i| before.windows.get(*i)) else { return };
    let Some(name) = exe_name(rule) else { return };
    let title = app_title(rule);
    let after = without(&before, windows);
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
