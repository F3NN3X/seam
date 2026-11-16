//! Recording a shortcut, and giving the ones with no keys suggested ones.

use super::catalog;
use super::clash;
use crate::pages::workspaces;
use crate::panel::Panel;
use gpui_kit::*;
use seam::hotkey;
use serde_json::{Map, Value, json};

/// Keys just saved that something else holds. They work or not, as before, and the row says why.
pub struct Clash {
    pub action: String,
    /// What holds them and what to do, for the line under the row.
    pub message: String,
}

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

pub fn stop_recording(panel: &mut Panel) {
    panel.recording = None;
    panel.recorder = None;
    panel.clash = None;
}

/// Set or clear one shortcut. A workspace's is saved with the workspace, the rest with the settings.
fn save(panel: &mut Panel, action: &str, value: Value, window: &mut Window, cx: &mut Context<Panel>) {
    match action.strip_prefix("workspace:") {
        Some(id) => workspaces::set_hotkey(panel, id, value.as_str().map(String::from), window, cx),
        None => panel.patch(json!({ "hotkeys": { action: value } }), window, cx),
    }
}

fn finish(panel: &mut Panel, action: &str, value: Value, window: &mut Window, cx: &mut Context<Panel>) {
    stop_recording(panel);
    panel.hotkey_warning = value.as_str().is_some_and(types_characters).then(|| action.to_string());
    save(panel, action, value, window, cx);
}

/// Listens ahead of every key binding, so Tab and Ctrl+C can be recorded too.
pub fn start_recording(panel: &mut Panel, action: String, cx: &mut Context<Panel>) {
    let view = cx.entity().downgrade();
    panel.recording = Some(action.clone());
    panel.clash = None;
    panel.hotkey_warning = None;
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
                        let message = clash::conflict(&combo, &action, &catalog::items(panel));
                        finish(panel, &action, json!(combo), window, cx);
                        panel.clash = message.map(|message| Clash { action: action.clone(), message });
                    }
                }
            }
            cx.notify();
        });
    }));
    cx.notify();
}

/// Give every shortcut with no keys a suggestion, with Undo.
pub fn use_suggested(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let chosen = clash::suggest(&catalog::items(panel));
    if chosen.is_empty() {
        return;
    }
    let hotkeys: Map<String, Value> = chosen
        .iter()
        .filter(|(action, _)| !action.starts_with("workspace:"))
        .map(|(action, keys)| (action.clone(), json!(keys)))
        .collect();
    if !hotkeys.is_empty() {
        panel.patch(json!({ "hotkeys": hotkeys }), window, cx);
    }
    for (action, keys) in chosen.iter().filter(|(action, _)| action.starts_with("workspace:")) {
        save(panel, action, json!(keys), window, cx);
    }
    let plural = if chosen.len() == 1 { "" } else { "s" };
    let message = format!("Set {} shortcut{plural}", chosen.len());
    panel.offer_undo(
        message,
        move |panel, window, cx| {
            // A shortcut changed since stays as it is.
            let now = catalog::items(panel);
            for (action, keys) in &chosen {
                let same = now.iter().any(|i| i.action == *action && i.combo.as_ref() == Some(keys));
                if same {
                    save(panel, action, Value::Null, window, cx);
                }
            }
        },
        window,
        cx,
    );
}
