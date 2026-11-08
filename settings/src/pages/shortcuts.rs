//! Global shortcuts, and recording a new one.

use crate::pages::workspaces;
use crate::panel::Panel;
use crate::theme::palette;
use crate::ui::{self, card, keycaps, row_with, section};
use gpui_kit::*;
use seam::hotkey;
use serde_json::{json, Value};

/// The config form of a keystroke, or `None` for a key Seam can't register.
fn combo_of(k: &Keystroke) -> Option<String> {
    let key = match k.key.as_str() {
        "left" | "right" | "up" | "down" | "space" | "tab" | "enter" | "home" | "end" | "delete" | "insert" => {
            let mut c = k.key.chars();
            c.next().map(|first| first.to_ascii_uppercase().to_string() + c.as_str())?
        }
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        key => key.to_ascii_uppercase(),
    };
    let m = &k.modifiers;
    let parts = [(m.control, "Ctrl"), (m.shift, "Shift"), (m.alt, "Alt"), (m.platform, "Win")];
    let held = parts.iter().filter(|(on, _)| *on).map(|(_, name)| name.to_string());
    let combo = held.chain([key]).collect::<Vec<_>>().join("+");
    hotkey::parse(&combo).map(|_| combo)
}

/// The Windows rows, in `Hotkeys::directional` order. Search lists the same titles.
const WINDOW_TITLES: [&str; 12] = [
    "Move window left",
    "Move window right",
    "Move window up",
    "Move window down",
    "Focus window to the left",
    "Focus window to the right",
    "Focus window above",
    "Focus window below",
    "Swap with window to the left",
    "Swap with window to the right",
    "Swap with window above",
    "Swap with window below",
];

/// Ctrl+Alt is AltGr on many layouts, so it can eat a typed character.
fn types_characters(combo: &str) -> bool {
    let printable = combo.rsplit('+').next().is_some_and(|key| key.chars().count() == 1);
    combo.contains("Ctrl") && combo.contains("Alt") && !combo.contains("Shift") && printable
}

fn stop_recording(panel: &mut Panel) {
    panel.recording = None;
    panel.recorder = None;
}

fn finish(panel: &mut Panel, action: &str, value: Value, window: &mut Window, cx: &mut Context<Panel>) {
    stop_recording(panel);
    panel.hotkey_warning = value.as_str().is_some_and(types_characters).then(|| action.to_string());
    // A workspace's shortcut is saved with the workspace, the rest with the settings.
    match action.strip_prefix("workspace:") {
        Some(id) => workspaces::set_hotkey(panel, id, value.as_str().map(String::from), window, cx),
        None => panel.patch(json!({ "hotkeys": { action: value } }), window, cx),
    }
}

/// Listens ahead of every key binding, so Tab and Ctrl+C can be recorded too.
fn start_recording(panel: &mut Panel, action: String, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    panel.recording = Some(action.clone());
    panel.recorder = Some(cx.intercept_keystrokes(move |event, window, cx| {
        let key = event.keystroke.clone();
        if ["shift", "control", "alt", "platform", "function"].contains(&key.key.as_str()) {
            return;
        }
        cx.stop_propagation();
        let _ = view.update(cx, |panel, cx| {
            match key.key.as_str() {
                "escape" => stop_recording(panel),
                "backspace" => finish(panel, &action, Value::Null, window, cx),
                _ => {
                    if let Some(combo) = combo_of(&key) {
                        finish(panel, &action, json!(combo), window, cx);
                    }
                }
            }
            cx.notify();
        });
    }));
    cx.notify();
}

/// One row. `slot` numbers the row, so its buttons get ids of their own.
fn shortcut_row(
    panel: &Panel,
    title: String,
    action: String,
    combo: Option<&String>,
    slot: usize,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    if panel.recording.as_deref() == Some(action.as_str()) {
        let field = div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(160.))
            .h(px(32.))
            .border_1()
            .border_color(p.accent)
            .rounded(px(8.))
            .text_size(px(13.))
            .child("Press the keys…");
        let cancel = ui::secondary(("cancel", slot), "Cancel", cx).on_click(cx.listener(|panel, _, _, cx| {
            stop_recording(panel);
            cx.notify();
        }));
        return row_with(title, None, div().flex().items_center().gap(px(16.)).child(field).child(cancel));
    }
    let conflict = panel.state.as_ref().is_some_and(|s| s.hotkey_errors.iter().any(|e| e.action == action));
    let note =
        |text: &'static str, color: Hsla| div().text_size(px(13.)).text_color(color).child(text).into_any_element();
    let about = if conflict {
        Some(note("Another app already uses this shortcut. Choose a different one", p.negative))
    } else if panel.hotkey_warning.as_deref() == Some(action.as_str()) {
        Some(note("Ctrl+Alt types characters on many keyboards. Add Shift", p.negative))
    } else {
        None
    };
    let keys = match combo {
        Some(combo) => keycaps(combo, conflict, p).into_any_element(),
        None => div().text_size(px(13.)).text_color(p.muted).child("Not set").into_any_element(),
    };
    let label = if combo.is_some() { "Change" } else { "Record" };
    let button = ui::secondary(("change", slot), label, cx)
        .on_click(cx.listener(move |panel, _, _, cx| start_recording(panel, action.clone(), cx)));
    row_with(title, about, div().flex().items_center().gap(px(16.)).child(keys).child(button))
}

pub fn render(panel: &mut Panel, _: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let keys = panel.config.hotkeys.clone();
    let mut row = |title: &str, action: &str, combo: Option<&String>, slot: usize| {
        shortcut_row(panel, title.to_string(), action.to_string(), combo, slot, cx)
    };
    let rows = vec![
        row("Line up screens", "line_up", keys.line_up.as_ref(), 0),
        row("Pause or resume pointer remap", "pause", keys.pause.as_ref(), 1),
    ];
    let zone_rows = vec![
        row("Move window to next zone", "next_zone", keys.next_zone.as_ref(), 2),
        row("Move window to previous zone", "prev_zone", keys.prev_zone.as_ref(), 3),
    ];
    // Move, focus and swap in each direction on the desk.
    let mut window_rows: Vec<AnyElement> = keys
        .directional()
        .into_iter()
        .zip(WINDOW_TITLES)
        .enumerate()
        .map(|(i, ((verb, dir, combo), title))| row(title, &format!("{verb}_{dir}"), combo, 5 + i))
        .collect();
    window_rows.extend([
        row("Pin window on top", "pin_top", keys.pin_top.as_ref(), 17),
        row("Keep window out of workspace switches", "pin_sticky", keys.pin_sticky.as_ref(), 18),
        row("Release pinned windows", "release_all", keys.release_all.as_ref(), 19),
    ]);
    // One row for each workspace on this desk, then the capture shortcut.
    let on_desk = workspaces::on_desk(panel);
    let mut workspace_rows: Vec<AnyElement> = on_desk
        .iter()
        .enumerate()
        .map(|(i, ws)| row(&ws.name, &format!("workspace:{}", ws.id), ws.hotkey.as_ref(), 20 + i))
        .collect();
    workspace_rows.push(row("Capture current windows", "capture_workspace", keys.capture_workspace.as_ref(), 4));
    workspace_rows.push(row("Snap back to this workspace", "snap_back", keys.snap_back.as_ref(), 20 + on_desk.len()));
    workspace_rows.push(row(
        "Save windows to the active workspace",
        "quick_save",
        keys.quick_save.as_ref(),
        21 + on_desk.len(),
    ));
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(ui::page_title("Shortcuts", "These work anywhere in Windows, even with this window closed", p))
        .child(section("Displays", card(rows, p), p))
        .child(section("Windows", card(window_rows, p), p))
        .child(section("Zones", card(zone_rows, p), p))
        .child(section("Workspaces", card(workspace_rows, p), p))
}
