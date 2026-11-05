//! Global shortcuts, and recording a new one.

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

/// Ctrl+Alt is AltGr on many layouts, so it can eat a typed character.
fn types_characters(combo: &str) -> bool {
    let printable = combo.rsplit('+').next().is_some_and(|key| key.chars().count() == 1);
    combo.contains("Ctrl") && combo.contains("Alt") && !combo.contains("Shift") && printable
}

fn stop_recording(panel: &mut Panel) {
    panel.recording = None;
    panel.recorder = None;
}

fn finish(panel: &mut Panel, action: &'static str, value: Value, window: &mut Window, cx: &mut Context<Panel>) {
    stop_recording(panel);
    panel.hotkey_warning = value.as_str().is_some_and(types_characters).then_some(action);
    panel.patch(json!({ "hotkeys": { action: value } }), window, cx);
}

/// Listens ahead of every key binding, so Tab and Ctrl+C can be recorded too.
fn start_recording(panel: &mut Panel, action: &'static str, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    panel.recording = Some(action);
    panel.recorder = Some(cx.intercept_keystrokes(move |event, window, cx| {
        let key = event.keystroke.clone();
        if ["shift", "control", "alt", "platform", "function"].contains(&key.key.as_str()) {
            return;
        }
        cx.stop_propagation();
        let _ = view.update(cx, |panel, cx| {
            match key.key.as_str() {
                "escape" => stop_recording(panel),
                "backspace" => finish(panel, action, Value::Null, window, cx),
                _ => {
                    if let Some(combo) = combo_of(&key) {
                        finish(panel, action, json!(combo), window, cx);
                    }
                }
            }
            cx.notify();
        });
    }));
    cx.notify();
}

fn shortcut_row(
    panel: &Panel,
    title: &'static str,
    action: &'static str,
    combo: Option<&String>,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    if panel.recording == Some(action) {
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
        let cancel = ui::secondary(("cancel", action.len()), "Cancel", cx).on_click(cx.listener(|panel, _, _, cx| {
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
    } else if panel.hotkey_warning == Some(action) {
        Some(note("Ctrl+Alt types characters on many keyboards. Add Shift", p.negative))
    } else {
        None
    };
    let keys = match combo {
        Some(combo) => keycaps(combo, conflict, p).into_any_element(),
        None => div().text_size(px(13.)).text_color(p.muted).child("Not set").into_any_element(),
    };
    let label = if combo.is_some() { "Change" } else { "Record" };
    let button = ui::secondary(("change", action.len()), label, cx)
        .on_click(cx.listener(move |panel, _, _, cx| start_recording(panel, action, cx)));
    row_with(title, about, div().flex().items_center().gap(px(16.)).child(keys).child(button))
}

pub fn render(panel: &mut Panel, _: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let (line_up, pause) = (panel.config.hotkeys.line_up.clone(), panel.config.hotkeys.pause.clone());
    let rows = vec![
        shortcut_row(panel, "Line up screens", "line_up", line_up.as_ref(), cx),
        shortcut_row(panel, "Pause or resume pointer remap", "pause", pause.as_ref(), cx),
    ];
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(ui::page_title("Shortcuts", "These work anywhere in Windows, even with this window closed", p))
        .child(section("Displays", card(rows, p), p))
}
