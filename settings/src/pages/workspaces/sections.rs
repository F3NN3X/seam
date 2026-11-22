//! The two sections under a workspace: what happens when you switch to it, and what holds for all.

use super::edit;
use crate::pages::shortcuts::shortcut_row;
use crate::pages::workspace_editor;
use crate::panel::{Panel, error_text, refusal};
use crate::theme::Palette;
use crate::theme::tokens;
use crate::ui::{self, Row, card, row, section, segmented, switch};
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use seam::workspace::{self, OpenWindows, OtherWindows, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Slot of the shortcut row's buttons, past the Shortcuts page's own.
const SHORTCUT_SLOT: usize = 7000;

/// "When you switch to «Name»": its shortcut, and what a switch starts, shows and leaves alone.
pub(super) fn switch_section(
    panel: &mut Panel,
    ws: &Workspace,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let view = cx.entity();
    let action = format!("workspace:{}", ws.id);
    let about = format!("Switch to {} from anywhere in Windows", ws.name);
    let shortcut = shortcut_row(panel, "Shortcut".into(), action, ws.hotkey.as_ref(), (SHORTCUT_SLOT, Some(about)), cx);
    let id = ws.id.clone();
    let start_missing = switch("start-missing", "Start missing apps", ws.launch_missing, window, cx, {
        let (view, id) = (view.clone(), id.clone());
        move |on, window, cx| {
            let id = id.clone();
            view.update(cx, |panel, cx| edit(panel, &id, window, cx, |ws| ws.launch_missing = on));
        }
    });
    let show_all = switch("show-all", "Bring back minimized windows", ws.show_all, window, cx, {
        let (view, id) = (view.clone(), id.clone());
        move |on, window, cx| {
            let id = id.clone();
            view.update(cx, |panel, cx| edit(panel, &id, window, cx, |ws| ws.show_all = on));
        }
    });
    let open_selected = usize::from(ws.open_windows == OpenWindows::Leave);
    let open = segmented(
        "open-windows",
        "Windows already open",
        &["Move back", "Leave where they are"],
        open_selected,
        window,
        cx,
        {
            let (view, id) = (view.clone(), id.clone());
            move |i, window, cx| {
                let id = id.clone();
                let mode = [OpenWindows::Move, OpenWindows::Leave][i.min(1)];
                view.update(cx, |panel, cx| edit(panel, &id, window, cx, |ws| ws.open_windows = mode));
            }
        },
    );
    let selected = match ws.other_windows {
        OtherWindows::Leave => 0,
        OtherWindows::Minimize => 1,
        OtherWindows::Close => 2,
    };
    let others = segmented("other-windows", "Other windows", &["Leave", "Minimize", "Close"], selected, window, cx, {
        let id = id.clone();
        move |i, window, cx| {
            let id = id.clone();
            let mode = [OtherWindows::Leave, OtherWindows::Minimize, OtherWindows::Close][i.min(2)];
            view.update(cx, |panel, cx| edit(panel, &id, window, cx, |ws| ws.other_windows = mode));
        }
    });
    let mut rows: Vec<ui::Row> = vec![
        shortcut,
        row("Start missing apps", Some("Apps that aren't open start, in the order above".into()), start_missing, p),
        row(
            "Windows already open",
            Some("New windows always go to their place. Put windows back moves every window".into()),
            open,
            p,
        ),
        row(
            "Bring back minimized windows",
            Some("A window saved minimized comes back, in its place".into()),
            show_all,
            p,
        ),
        row("Other windows", Some("Close asks each app first, so it can save".into()), others, p),
    ];
    rows.extend(workspace_editor::zones_section(panel, ws, window, cx).into_iter().map(Row::from));
    section(format!("When you switch to {}", ws.name), card(rows, p), p)
}

/// The one switch for every workspace: put apps in place when they open (§11.1).
fn auto_apply_row(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let p = crate::theme::palette(cx);
    let view = cx.entity();
    let on = panel.config.auto_apply;
    let toggle = switch("auto-apply", "Put apps in place when they open", on, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| {
            panel.config_mut().auto_apply = on;
            cx.notify();
            panel.call("autoapply.set", json!({ "on": on }), window, cx, |panel, result, window, cx| {
                if let Err(e) = result {
                    window.push_notification(refusal("Setting left as it was", &error_text(&e)), cx);
                }
                panel.reload_config(window, cx);
            });
        });
    });
    let about = "A window that opens goes where the active workspace puts it, once";
    row("Put apps in place when they open", Some(about.into()), toggle, p)
}

/// "Remember folders and pages" (D330), and while some are still saved, the button that
/// forgets them on every desk.
fn places_row(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let p = crate::theme::palette(cx);
    let view = cx.entity();
    let on = panel.config.workspaces_remember_places;
    let title = "Remember folders and pages";
    let toggle = switch("remember-places", title, on, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| panel.patch(json!({ "workspaces_remember_places": on }), window, cx));
    });
    let with_places = |panel: &Panel| -> BTreeMap<String, Vec<Value>> {
        let desks = panel.config.desks.iter().filter(|(_, d)| workspace::has_places(&d.workspaces));
        desks.map(|(sig, d)| (sig.clone(), workspace::without_places(&d.workspaces))).collect()
    };
    let forget = (!with_places(panel).is_empty()).then(|| {
        ui::secondary("forget-places", "Forget saved", cx).on_click(cx.listener(move |panel, _, window, cx| {
            let desks: serde_json::Map<String, Value> =
                with_places(panel).into_iter().map(|(sig, list)| (sig, json!({ "workspaces": list }))).collect();
            panel.patch(json!({ "desks": desks }), window, cx);
        }))
    });
    let about = "Explorer opens the same folder and browsers the same page when a workspace starts them";
    row(
        title,
        Some(about.into()),
        div().flex().items_center().gap(px(tokens::SPACE_12)).children(forget).child(toggle),
        p,
    )
}

/// "Update from screen removes apps that aren't open": off keeps the rows of closed apps.
fn removes_row(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let p = crate::theme::palette(cx);
    let view = cx.entity();
    let title = "Update from screen removes apps that aren't open";
    let toggle = switch(
        "update-removes",
        title,
        panel.config.workspaces_update_removes_closed,
        window,
        cx,
        move |on, window, cx| {
            view.update(cx, |panel, cx| panel.patch(json!({ "workspaces_update_removes_closed": on }), window, cx));
        },
    );
    row(title, Some("Off keeps them, so a closed app still starts when you switch".into()), toggle, p)
}

/// "All workspaces": what holds whichever one is in use.
pub(super) fn all_section(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> AnyElement {
    let rows = vec![auto_apply_row(panel, window, cx), places_row(panel, window, cx), removes_row(panel, window, cx)];
    section("All workspaces", card(rows, p), p)
}
