//! Keys that something already holds, and keys Seam can suggest for the shortcuts with none.

use super::catalog::Item;
use seam::hotkey::{self, Combo};

/// Keys Windows keeps for itself, and what for. A shortcut Seam can register may still never
/// reach it, so the page refuses these.
const RESERVED: &[(&str, &str)] = &[
    ("Ctrl+Alt+Delete", "the security screen"),
    ("Ctrl+Alt+Tab", "switching windows"),
    ("Alt+Tab", "switching windows"),
    ("Alt+F4", "closing a window"),
    ("Alt+Space", "the window menu"),
    ("Win+Tab", "Task View"),
    ("Win+Space", "switching the input language"),
    ("Win+Left", "snapping windows"),
    ("Win+Right", "snapping windows"),
    ("Win+Up", "snapping windows"),
    ("Win+Down", "snapping windows"),
    ("Win+Home", "minimising all but the active window"),
    ("Win+Shift+Left", "moving a window to another screen"),
    ("Win+Shift+Right", "moving a window to another screen"),
    ("Win+Ctrl+Left", "switching desktops"),
    ("Win+Ctrl+Right", "switching desktops"),
    ("Win+Ctrl+D", "a new desktop"),
    ("Win+Ctrl+F4", "closing a desktop"),
    ("Win+Shift+S", "Snipping Tool"),
    ("Win+A", "quick settings"),
    ("Win+B", "the notification area"),
    ("Win+C", "Copilot"),
    ("Win+D", "showing the desktop"),
    ("Win+E", "File Explorer"),
    ("Win+F", "Feedback Hub"),
    ("Win+G", "Game Bar"),
    ("Win+H", "voice typing"),
    ("Win+I", "Settings"),
    ("Win+K", "casting"),
    ("Win+L", "locking the PC"),
    ("Win+M", "minimising all windows"),
    ("Win+N", "notifications"),
    ("Win+O", "locking the screen's orientation"),
    ("Win+P", "projecting to a screen"),
    ("Win+Q", "search"),
    ("Win+R", "Run"),
    ("Win+S", "search"),
    ("Win+T", "the taskbar"),
    ("Win+U", "accessibility settings"),
    ("Win+V", "Clipboard history"),
    ("Win+W", "widgets"),
    ("Win+X", "the Start button menu"),
    ("Win+Z", "snap layouts"),
];

/// What Windows keeps `combo` for, with the keys as they read in a sentence.
fn reserved(combo: Combo) -> Option<(String, &'static str)> {
    RESERVED.iter().find(|(keys, _)| hotkey::parse(keys) == Some(combo)).map(|(keys, what)| {
        let words = keys.split('+').map(|key| match key {
            "Left" => "←",
            "Right" => "→",
            "Up" => "↑",
            "Down" => "↓",
            other => other,
        });
        (words.collect::<Vec<_>>().join(" "), *what)
    })
}

/// What `combo` clashes with when it's given to `action`, as the line to show under the row.
pub fn conflict(combo: &str, action: &str, items: &[Item]) -> Option<String> {
    let pressed = hotkey::parse(combo)?;
    if let Some((keys, what)) = reserved(pressed) {
        return Some(format!("Windows uses {keys} for {what}. Press other keys"));
    }
    let other = items.iter().find(|i| {
        i.action != action && i.combo.as_deref().and_then(hotkey::parse).is_some_and(|held| held == pressed)
    })?;
    Some(format!("Also used by {}", other.title))
}

/// Keys to try after a shortcut's own preference, none of them Windows' or Seam's already.
fn pool() -> impl Iterator<Item = String> {
    ["Ctrl+Shift+Alt", "Ctrl+Shift+Win", "Ctrl+Win+Alt"]
        .into_iter()
        .flat_map(|mods| ('A'..='Z').map(move |letter| format!("{mods}+{letter}")))
}

/// Keys for every shortcut with none, as (action, keys). None clashes with Windows, with a
/// shortcut that is set, or with another suggestion.
pub fn suggest(items: &[Item]) -> Vec<(String, String)> {
    let mut taken: Vec<Combo> = items.iter().filter_map(|i| i.combo.as_deref().and_then(hotkey::parse)).collect();
    let mut chosen = Vec::new();
    for item in items.iter().filter(|i| i.combo.is_none()) {
        let mut free = |keys: &String| {
            let combo = hotkey::parse(keys)?;
            let usable = !taken.contains(&combo) && reserved(combo).is_none();
            usable.then(|| taken.push(combo))
        };
        let pick = item.preferred.iter().cloned().chain(pool()).find(|keys| free(keys).is_some());
        if let Some(keys) = pick {
            chosen.push((item.action.clone(), keys));
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::shortcuts::catalog::{Group, Section};

    fn item(action: &str, combo: Option<&str>, preferred: Option<&str>) -> Item {
        Item {
            title: action.to_uppercase(),
            action: action.into(),
            combo: combo.map(String::from),
            slot: 0,
            section: Section::Windows,
            group: Group::Sizes,
            preferred: preferred.map(String::from),
        }
    }

    #[test]
    fn windows_keys_are_refused_in_any_order() {
        let said = conflict("Shift+Win+S", "grow", &[]).unwrap();
        assert_eq!(said, "Windows uses Win Shift S for Snipping Tool. Press other keys");
        assert!(conflict("Win+Left", "grow", &[]).unwrap().starts_with("Windows uses Win ← for"));
        assert!(conflict("Ctrl+Shift+Alt+S", "grow", &[]).is_none());
    }

    #[test]
    fn another_shortcut_of_seam_is_named_but_not_the_one_being_set() {
        let items = [item("grow", Some("Ctrl+Alt+Shift+G"), None), item("shrink", None, None)];
        assert_eq!(conflict("Ctrl+Shift+Alt+G", "shrink", &items).as_deref(), Some("Also used by GROW"));
        assert!(conflict("Ctrl+Shift+Alt+G", "grow", &items).is_none());
    }

    #[test]
    fn suggestions_fill_only_the_unset_and_never_repeat_or_clash() {
        let items = [
            item("a", Some("Ctrl+Shift+Alt+A"), None),
            item("b", None, Some("Ctrl+Shift+Alt+A")),
            item("c", None, Some("Win+S")),
            item("d", None, Some("Ctrl+Shift+Alt+Z")),
            item("e", None, Some("Ctrl+Shift+Alt+Z")),
        ];
        let picked = suggest(&items);
        assert_eq!(picked.iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>(), ["b", "c", "d", "e"]);
        let combos: Vec<Combo> = picked.iter().filter_map(|(_, k)| hotkey::parse(k)).collect();
        assert_eq!(combos.len(), 4);
        for (i, combo) in combos.iter().enumerate() {
            assert!(!combos[..i].contains(combo));
            assert!(reserved(*combo).is_none());
            assert_ne!(Some(*combo), hotkey::parse("Ctrl+Shift+Alt+A"));
        }
        assert_eq!(picked[2].1, "Ctrl+Shift+Alt+Z");
    }

    #[test]
    fn every_reserved_key_can_be_pressed() {
        for (keys, _) in RESERVED {
            assert!(hotkey::parse(keys).is_some(), "{keys}");
        }
    }
}
