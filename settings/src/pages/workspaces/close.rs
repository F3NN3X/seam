//! Close this workspace and Minimize this workspace. Closing lists the windows first and
//! asks each to close; minimizing doesn't ask, and offers Undo.

use super::*;
use gpui_kit::component::Disableable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::tooltip::Tooltip;
use std::cell::RefCell;

/// The longest "App: title" a row of the list shows, in characters.
const LABEL_MAX: usize = 56;

/// One window of the workspace the tray would close.
struct Item {
    hwnd: i64,
    app: String,
    title: String,
    screen: Option<u64>,
}

/// The tray's answer to `workspace.windows`: what a close takes, and why it keeps the rest.
struct Listed {
    windows: Vec<Item>,
    /// Why each window left alone is: `pinned`, `sticky` or `elevated`.
    kept: Vec<String>,
}

fn listed(answer: &Value) -> Listed {
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_string();
    let windows = answer["windows"].as_array().into_iter().flatten();
    let kept = answer["kept"].as_array().into_iter().flatten();
    Listed {
        windows: windows
            .filter_map(|w| {
                Some(Item {
                    hwnd: w["hwnd"].as_i64()?,
                    app: text(w, "app"),
                    title: text(w, "title"),
                    screen: w["screen"].as_u64(),
                })
            })
            .collect(),
        kept: kept.map(|k| text(k, "why")).collect(),
    }
}

/// "App: window title", the title cut to one line, or the app alone when it has none.
fn window_label(app: &str, title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        return app.to_string();
    }
    let room = LABEL_MAX.saturating_sub(app.chars().count() + 2).max(8);
    if title.chars().count() <= room {
        return format!("{app}: {title}");
    }
    let cut: String = title.chars().take(room - 1).collect();
    format!("{app}: {}…", cut.trim_end())
}

/// The primary button's label and whether it is disabled, from how many windows are ticked.
fn close_label(ticked: usize) -> (String, bool) {
    match ticked {
        0 => ("Close windows".into(), true),
        1 => ("Close 1 window".into(), false),
        n => (format!("Close {n} windows"), false),
    }
}

/// "Not closed: 1 pinned window, 1 kept out of switches, 1 running as administrator", or
/// `None` when nothing was left out.
fn not_closed(kept: &[String]) -> Option<String> {
    let count = |why: &str| kept.iter().filter(|k| *k == why).count();
    let pinned = match count("pinned") {
        0 => None,
        1 => Some("1 pinned window".to_string()),
        n => Some(format!("{n} pinned windows")),
    };
    let part = |n: usize, what: &str| (n > 0).then(|| format!("{n} {what}"));
    let parts: Vec<String> =
        [pinned, part(count("sticky"), "kept out of switches"), part(count("elevated"), "running as administrator")]
            .into_iter()
            .flatten()
            .collect();
    (!parts.is_empty()).then(|| format!("Not closed: {}", parts.join(", ")))
}

/// What the notification says once the windows were asked to close.
fn closed_note(name: &str, asked: usize, changed: usize) -> String {
    let windows = if asked == 1 { "1 window".to_string() } else { format!("{asked} windows") };
    let said = format!("Asked {windows} of {name} to close");
    match changed {
        0 => said,
        1 => format!("{said}. 1 had changed, so Seam left it"),
        n => format!("{said}. {n} had changed, so Seam left them"),
    }
}

fn minimized_note(name: &str, count: usize) -> String {
    match count {
        1 => format!("Minimized 1 window of {name}"),
        n => format!("Minimized {n} windows of {name}"),
    }
}

/// Whether any of the workspace's windows is open, as far as the tray last said. Unknown counts
/// as open, so the items aren't taken away on a guess.
pub(super) fn any_open(panel: &Panel, ws: &Workspace) -> bool {
    let open = |i: usize| match workspace_editor::row_state(panel, ws, i) {
        Some(state) => matches!(state, RowState::InPlace | RowState::Moved | RowState::Minimized),
        None => panel.status.as_ref().is_none_or(|(id, _)| *id != ws.id),
    };
    (0..ws.windows.len()).any(open)
}

/// Minimize the windows of the workspace now, with Undo for 8 s.
pub(super) fn minimize(panel: &mut Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    let name = ws.name.clone();
    panel.call(
        "workspace.minimize",
        json!({ "id": ws.id }),
        window,
        cx,
        move |panel, result, window, cx| match result {
            Ok(answer) => match answer["minimized"].as_u64().unwrap_or(0) as usize {
                0 => {
                    window.push_notification(Notification::new().message(format!("Nothing of {name} to minimize")), cx)
                }
                n => {
                    let undo = |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                        panel.command("workspace.minimize_undo", json!({}), window, cx)
                    };
                    panel.offer_undo(minimized_note(&name, n), undo, window, cx);
                }
            },
            Err(e) => window.push_notification(refusal("Seam can't do that", &error_text(&e)), cx),
        },
    );
}

/// Show the workspace, ask the tray which of its windows are open, and open the list.
pub fn ask(panel: &mut Panel, id: &str, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(ws) = all(panel).into_iter().find(|w| w.id == id) else { return };
    panel.workspace = Some(ws.id.clone());
    cx.notify();
    let name = ws.name.clone();
    panel.call("workspace.windows", json!({ "id": ws.id }), window, cx, move |_, result, window, cx| match result {
        Ok(answer) => {
            let listed = listed(&answer);
            if listed.windows.is_empty() {
                window.push_notification(Notification::new().message(format!("Nothing of {name} is open")), cx);
            } else {
                open_list((ws.id, name), listed, window, cx);
            }
        }
        Err(e) => window.push_notification(refusal("Seam can't do that", &error_text(&e)), cx),
    });
}

/// The dialog: every window ticked, and "Close n windows" follows the ticks.
fn open_list((id, name): (String, String), listed: Listed, window: &mut Window, cx: &mut Context<Panel>) {
    let ticked = Rc::new(RefCell::new(vec![true; listed.windows.len()]));
    let (listed, view) = (Rc::new(listed), cx.entity().downgrade());
    window.open_dialog(cx, move |dialog, _, cx| {
        let p = palette(cx);
        let now = ticked.borrow().clone();
        let several = listed.windows.iter().any(|w| w.screen != listed.windows[0].screen);
        let mut list = div().flex().flex_col().gap(px(12.));
        let mut screen = None;
        for (i, item) in listed.windows.iter().enumerate() {
            if several && screen != Some(item.screen) {
                let heading = item.screen.map_or("Other screens".to_string(), |n| format!("Screen {n}"));
                list = list.child(ui::description(heading, p));
                screen = Some(item.screen);
            }
            let ticked = ticked.clone();
            let check = Checkbox::new(("close-workspace", i))
                .label(window_label(&item.app, &item.title))
                .checked(now[i])
                .on_click(move |on, window, _| {
                    ticked.borrow_mut()[i] = *on;
                    window.refresh();
                });
            list = list.child(check);
        }
        let (label, disabled) = close_label(now.iter().filter(|t| **t).count());
        let mut ok = ui::primary("close-workspace-ok", label);
        if disabled {
            ok = ok.disabled(true).tooltip("Tick a window to close it");
        }
        let (view, id, name, listed, ticked) = (view.clone(), id.clone(), name.clone(), listed.clone(), ticked.clone());
        let mut body = div().flex().flex_col().gap(px(tokens::SPACE_12)).child(ui::description(
            "Each window gets a normal close request, so an app with unsaved work asks you first. Untick a window to keep it open",
            p,
        ));
        body = body.child(list);
        if let Some(left_out) = not_closed(&listed.kept) {
            body = body.child(ui::description(left_out, p));
        }
        dialog
            .w(px(420.))
            .title(format!("Close {name}?"))
            .child(body)
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Keep them")))
                    .child(DialogAction::new().child(ok)),
            )
            .on_ok(move |_, window, cx| {
                let picked: Vec<i64> = listed
                    .windows
                    .iter()
                    .zip(ticked.borrow().iter())
                    .filter_map(|(item, on)| on.then_some(item.hwnd))
                    .collect();
                if picked.is_empty() {
                    return false;
                }
                let (id, name) = (id.clone(), name.clone());
                let _ = view.update(cx, |panel, cx| {
                    let params = json!({ "id": id, "hwnds": picked });
                    panel.call("workspace.close", params, window, cx, move |_, result, window, cx| match result {
                        Ok(answer) => {
                            let (asked, changed) = (answer["asked"].as_u64(), answer["changed"].as_u64());
                            let said = closed_note(&name, asked.unwrap_or(0) as usize, changed.unwrap_or(0) as usize);
                            window.push_notification(Notification::new().message(said), cx);
                        }
                        Err(e) => window.push_notification(refusal("Seam can't do that", &error_text(&e)), cx),
                    });
                });
                true
            })
    });
}

/// The two items of the more menu. With none of the windows open they are dimmed, saying why.
pub(super) fn menu_items(
    menu: gpui_kit::component::menu::PopupMenu,
    view: &WeakEntity<Panel>,
    ws: &Workspace,
    open: bool,
) -> gpui_kit::component::menu::PopupMenu {
    let (minimizing, closing) = (view.clone(), view.clone());
    let (to_minimize, id) = (ws.clone(), ws.id.clone());
    let item = |label: &'static str, key: &'static str| {
        let none = "None of its windows are open";
        PopupMenuItem::element(move |_, _| {
            let row = div().id(key).child(label);
            if open {
                row
            } else {
                row.opacity(tokens::OPACITY_DISABLED).tooltip(move |window, cx| Tooltip::new(none).build(window, cx))
            }
        })
        .disabled(!open)
    };
    menu.item(PopupMenuItem::separator())
        .item(item("Minimize this workspace", "minimize-workspace").on_click(move |_, window, cx| {
            let ws = to_minimize.clone();
            let _ = minimizing.update(cx, |panel, cx| minimize(panel, &ws, window, cx));
        }))
        .item(item("Close this workspace…", "close-workspace").on_click(move |_, window, cx| {
            let id = id.clone();
            let _ = closing.update(cx, |panel, cx| ask(panel, &id, window, cx));
        }))
}

#[cfg(test)]
mod tests {
    use super::{LABEL_MAX, close_label, closed_note, minimized_note, not_closed, window_label};

    #[test]
    fn the_button_follows_the_ticks() {
        assert_eq!(close_label(4), ("Close 4 windows".to_string(), false));
        assert_eq!(close_label(1), ("Close 1 window".to_string(), false));
        assert!(close_label(0).1);
    }

    #[test]
    fn what_was_left_out_is_named_by_count() {
        let kept: Vec<String> = ["pinned", "sticky", "elevated"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            not_closed(&kept).as_deref(),
            Some("Not closed: 1 pinned window, 1 kept out of switches, 1 running as administrator")
        );
        let two: Vec<String> = ["elevated", "elevated", "pinned", "pinned"].iter().map(|s| s.to_string()).collect();
        assert_eq!(not_closed(&two).as_deref(), Some("Not closed: 2 pinned windows, 2 running as administrator"));
        assert_eq!(not_closed(&[]), None);
    }

    #[test]
    fn a_window_is_named_by_app_and_title() {
        assert_eq!(window_label("Notepad", "notes.txt"), "Notepad: notes.txt");
        assert_eq!(window_label("Notepad", "  "), "Notepad");
        let long = window_label("Chrome", &"x".repeat(200));
        assert!(long.ends_with('…') && long.chars().count() <= LABEL_MAX);
    }

    #[test]
    fn the_result_says_what_changed() {
        assert_eq!(closed_note("Coding", 4, 0), "Asked 4 windows of Coding to close");
        assert_eq!(closed_note("Coding", 3, 1), "Asked 3 windows of Coding to close. 1 had changed, so Seam left it");
        assert_eq!(closed_note("Coding", 1, 2), "Asked 1 window of Coding to close. 2 had changed, so Seam left them");
        assert_eq!(minimized_note("Coding", 1), "Minimized 1 window of Coding");
    }
}
