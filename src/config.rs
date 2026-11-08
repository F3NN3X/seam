//! Settings and saved layouts, stored as JSON in %APPDATA%\Seam\config.json.

use crate::geom::{EdgeMap, Side};
use crate::log;
use crate::palette;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CURRENT_VERSION: u32 = 2;

/// One calibrated border, keyed by stable monitor device paths.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedBorder {
    /// Device path of the reference monitor.
    pub a: String,
    /// Device path of the neighbour.
    pub b: String,
    /// Side of `a` that `b` sits on.
    pub side: Side,
    pub map: EdgeMap,
    /// The screens touch on the desk but Windows has a gap between them, so the
    /// border holds while `b` stays beyond that side of `a`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub glued: bool,
    /// The screens touch in Windows but not on the desk, so the pointer stops
    /// at this border instead of crossing it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_on_desk: bool,
}

/// What a left-click on the tray icon does.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrayClick {
    #[default]
    Open,
    LineUp,
}

/// Global shortcuts, in "Ctrl+Shift+Alt+L" form. `None` means not set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Hotkeys {
    pub line_up: Option<String>,
    pub pause: Option<String>,
    pub next_zone: Option<String>,
    pub prev_zone: Option<String>,
    pub capture_workspace: Option<String>,
    pub move_left: Option<String>,
    pub move_right: Option<String>,
    pub move_up: Option<String>,
    pub move_down: Option<String>,
    pub focus_left: Option<String>,
    pub focus_right: Option<String>,
    pub focus_up: Option<String>,
    pub focus_down: Option<String>,
    pub swap_left: Option<String>,
    pub swap_right: Option<String>,
    pub swap_up: Option<String>,
    pub swap_down: Option<String>,
    pub pin_top: Option<String>,
    pub pin_sticky: Option<String>,
    pub release_all: Option<String>,
    /// "Snap back to this workspace": switch to the active workspace again.
    pub snap_back: Option<String>,
    /// "Save windows to the active workspace": capture into the workspace in use (§7).
    pub quick_save: Option<String>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            line_up: Some("Ctrl+Shift+Alt+L".into()),
            pause: Some("Ctrl+Shift+Alt+P".into()),
            // New installs leave these unset (D107). The tray clears either one in a
            // saved file where it sits on a directional key.
            next_zone: None,
            prev_zone: None,
            capture_workspace: None,
            move_left: Some("Shift+Win+Alt+Left".into()),
            move_right: Some("Shift+Win+Alt+Right".into()),
            move_up: Some("Shift+Win+Alt+Up".into()),
            move_down: Some("Shift+Win+Alt+Down".into()),
            focus_left: Some("Ctrl+Shift+Alt+Left".into()),
            focus_right: Some("Ctrl+Shift+Alt+Right".into()),
            focus_up: Some("Ctrl+Shift+Alt+Up".into()),
            focus_down: Some("Ctrl+Shift+Alt+Down".into()),
            swap_left: None,
            swap_right: None,
            swap_up: None,
            swap_down: None,
            pin_top: None,
            pin_sticky: None,
            release_all: None,
            snap_back: None,
            quick_save: None,
        }
    }
}

impl Hotkeys {
    /// The directional shortcuts as (verb, direction, combo), in the order they register.
    pub fn directional(&self) -> [(&'static str, &'static str, Option<&String>); 12] {
        [
            ("move", "left", self.move_left.as_ref()),
            ("move", "right", self.move_right.as_ref()),
            ("move", "up", self.move_up.as_ref()),
            ("move", "down", self.move_down.as_ref()),
            ("focus", "left", self.focus_left.as_ref()),
            ("focus", "right", self.focus_right.as_ref()),
            ("focus", "up", self.focus_up.as_ref()),
            ("focus", "down", self.focus_down.as_ref()),
            ("swap", "left", self.swap_left.as_ref()),
            ("swap", "right", self.swap_right.as_ref()),
            ("swap", "up", self.swap_up.as_ref()),
            ("swap", "down", self.swap_down.as_ref()),
        ]
    }

    /// Next or previous zone on the same key as a directional command would
    /// never fire. Clear whichever does, and say which.
    pub fn drop_zone_clashes(&mut self) -> Vec<&'static str> {
        let taken: Vec<_> =
            self.directional().iter().filter_map(|(_, _, c)| crate::hotkey::parse(c.as_ref()?)).collect();
        let mut dropped = Vec::new();
        for (action, combo) in [("next_zone", &mut self.next_zone), ("prev_zone", &mut self.prev_zone)] {
            if combo.as_deref().and_then(crate::hotkey::parse).is_some_and(|c| taken.contains(&c)) {
                *combo = None;
                dropped.push(action);
            }
        }
        dropped
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ZoneSettings {
    pub enabled: bool,
    /// "shift", "ctrl" or "alt".
    pub show_key: String,
    pub span_key: String,
    pub gap: u32,
    pub restore_size: bool,
    /// Experimental: after a screen change, put snapped windows back in their zones.
    pub refit: bool,
    /// Edit layouts on the screen itself. Off shows a live view there and edits in the panel.
    pub edit_on_screen: bool,
    /// Exe file names, case-insensitive.
    pub exclude: Vec<String>,
}

impl Default for ZoneSettings {
    fn default() -> Self {
        ZoneSettings {
            enabled: true,
            show_key: "shift".into(),
            span_key: "ctrl".into(),
            gap: 8,
            restore_size: true,
            refit: false,
            edit_on_screen: true,
            exclude: Vec::new(),
        }
    }
}

/// Pins, the focus border and other window managers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WindowSettings {
    /// Colour the focused window's border (D108).
    pub focus_border: bool,
    /// "Another window manager handles my windows" (D110).
    pub other_wm: bool,
    /// "none", a name from `palette::SWATCHES` or `#RRGGBB`. Anything else reads as
    /// the default (D118, D122).
    pub focus_color: String,
    pub pin_color: String,
    /// "Tell me when a pinned window closes" (D119).
    pub notify_pinned_close: bool,
    /// "Pinned windows stay when I switch workspaces": a switch leaves a window
    /// pinned on top alone, as it does a sticky one. Capture still saves it.
    pub pins_stay_on_switch: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        WindowSettings {
            focus_border: false,
            other_wm: false,
            focus_color: palette::FOCUS_DEFAULT.into(),
            pin_color: palette::PIN_DEFAULT.into(),
            notify_pinned_close: false,
            pins_stay_on_switch: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ApiSettings {
    /// "Let other apps control Seam".
    pub enabled: bool,
}

impl Default for ApiSettings {
    fn default() -> Self {
        ApiSettings { enabled: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub auto_install: bool,
    pub prerelease: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings { auto_check: true, auto_install: false, prerelease: false }
    }
}

/// The size a window gets when Seam moves it to another screen (§11.4).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowSize {
    /// "Let Windows scale it": Windows' own DPI scaling, and nothing more.
    #[default]
    Windows,
    /// "Keep its pixel size": the same width and height in pixels as before the move.
    Pixels,
    /// "Keep its real size": the same size on the desk, from each screen's EDID size.
    Real,
}

/// Who reopens the apps of a restored last session (D128).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreApps {
    /// Windows' Restart apps, or the apps themselves. Seam only arranges them.
    #[default]
    Windows,
    /// Seam starts the missing ones, and turns Windows' Restart apps off meanwhile.
    Seam,
}

/// One set of screens and everything that belongs to it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Desk {
    pub name: String,
    /// Unix seconds. 0 until the tray has seen a migrated desk connected.
    pub last_seen: u64,
    pub borders: Vec<SavedBorder>,
    /// Unix seconds a line-up was last accepted. None for a desk lined up by an
    /// older build, or not at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lined_up_at: Option<u64>,
    /// Monitor device path -> zone layout. Kept as raw JSON until zones exist.
    pub zones: BTreeMap<String, serde_json::Value>,
    /// Kept as raw JSON until workspaces exist.
    pub workspaces: Vec<serde_json::Value>,
    /// The hidden "Last session" workspace (D120), raw like the workspaces. It is
    /// kept apart from them, so no list, menu or shortcut shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_session: Option<serde_json::Value>,
    /// The id of the workspace last in use here, so a restart shows it as in use again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_workspace: Option<String>,
    /// The id of the workspace taken as in use at start when none was remembered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
}

impl Desk {
    /// The name of a desk whose screens haven't told us theirs yet.
    pub fn placeholder(screens: usize) -> String {
        format!("Desk with {screens} screens")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Missing in v1 files, so it reads as 0 there rather than the current version.
    #[serde(default)]
    pub version: u32,
    /// Remap the pointer at calibrated borders.
    pub enabled: bool,
    /// After lining up, also move monitors in the Windows arrangement so the
    /// pixel layout matches at the middle of each border.
    pub align_arrangement: bool,
    /// Offer to line up a set of screens we haven't seen before.
    pub ask_new_desks: bool,
    /// "When a window moves to another screen": what size a key, API or workspace move
    /// to another screen gives it, outside zones (§11.4).
    pub window_size_on_move: WindowSize,
    /// "Also when I drag a window to another screen": the same after a free drag with
    /// the mouse that didn't snap. Does nothing while the size is Windows'.
    pub window_size_on_drag: bool,
    pub tray_click: TrayClick,
    pub debug_log: bool,
    pub hotkeys: Hotkeys,
    pub zones: ZoneSettings,
    pub windows: WindowSettings,
    /// "Offer to restore my windows after I sign in" (D120). Off saves no session
    /// and deletes the saved ones.
    pub restore_session: bool,
    /// Who reopens the last session's apps (D128).
    pub restore_apps: RestoreApps,
    /// "Put apps in place when they open": a window that opens and matches a rule of
    /// the active workspace goes to its place once (§11.1). Off by default.
    pub auto_apply: bool,
    /// The panel asked once whether to turn on Windows' "Ease cursor movement between
    /// displays", or Seam turned it on. Either way it isn't asked again.
    pub cursor_easing_asked: bool,
    /// Line Up lets you type the offset between two screens' edges in cm.
    pub lineup_measure: bool,
    /// Windows' `RestartApps` as it was before Seam turned it off, `u32::MAX` if it
    /// wasn't there, or `None` while Seam leaves it alone. Only the tray writes it.
    pub restore_apps_saved: Option<u32>,
    pub updates: UpdateSettings,
    pub api: ApiSettings,
    /// Layout signature (sorted device paths) -> desk.
    pub desks: BTreeMap<String, Desk>,
    /// Signatures we've already offered to line up, so we only ask once.
    pub seen: Vec<String>,
    /// v1 only. Read for migration, never written.
    #[serde(skip_serializing)]
    pub layouts: BTreeMap<String, Vec<SavedBorder>>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CURRENT_VERSION,
            enabled: true,
            align_arrangement: true,
            ask_new_desks: true,
            window_size_on_move: WindowSize::Windows,
            window_size_on_drag: false,
            tray_click: TrayClick::Open,
            debug_log: false,
            hotkeys: Hotkeys::default(),
            zones: ZoneSettings::default(),
            windows: WindowSettings::default(),
            restore_session: false,
            restore_apps: RestoreApps::Windows,
            auto_apply: false,
            cursor_easing_asked: false,
            lineup_measure: false,
            restore_apps_saved: None,
            updates: UpdateSettings::default(),
            api: ApiSettings::default(),
            desks: BTreeMap::new(),
            seen: Vec::new(),
            layouts: BTreeMap::new(),
        }
    }
}

/// A set of screens is identified by its sorted device paths.
pub fn signature<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let mut v: Vec<&str> = ids.collect();
    v.sort_unstable();
    v.join("|")
}

/// Apply a JSON merge patch (RFC 7386): objects merge, null removes.
pub fn merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let Some(fields) = patch.as_object() else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = serde_json::json!({});
    }
    if let Some(object) = target.as_object_mut() {
        for (key, value) in fields {
            if value.is_null() {
                object.remove(key);
            } else {
                merge_patch(object.entry(key.clone()).or_insert(serde_json::Value::Null), value);
            }
        }
    }
}

/// The merge patch that turns `old` into `new`.
pub fn diff_patch(old: &serde_json::Value, new: &serde_json::Value) -> serde_json::Value {
    let (Some(before), Some(after)) = (old.as_object(), new.as_object()) else { return new.clone() };
    let mut patch = serde_json::Map::new();
    for key in before.keys().filter(|k| !after.contains_key(*k)) {
        patch.insert(key.clone(), serde_json::Value::Null);
    }
    for (key, value) in after {
        match before.get(key) {
            Some(same) if same == value => {}
            Some(previous) => {
                patch.insert(key.clone(), diff_patch(previous, value));
            }
            None => {
                patch.insert(key.clone(), value.clone());
            }
        }
    }
    serde_json::Value::Object(patch)
}

/// Write-then-rename so a crash never leaves half a file.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

impl Config {
    /// This config with a merge patch applied. A patch's null deletes a key,
    /// but a deleted shortcut would come back as its default, so "not set"
    /// stays an explicit null.
    pub fn patched(&self, patch: &serde_json::Value) -> serde_json::Result<Config> {
        let mut merged = serde_json::to_value(self)?;
        merge_patch(&mut merged, patch);
        let hotkeys = patch.get("hotkeys").and_then(|h| h.as_object()).into_iter().flatten();
        for (action, _) in hotkeys.filter(|(_, value)| value.is_null()) {
            merged["hotkeys"][action] = serde_json::Value::Null;
        }
        let mut config: Config = serde_json::from_value(merged)?;
        config.windows.focus_color = palette::normalise(&config.windows.focus_color, palette::FOCUS_DEFAULT);
        config.windows.pin_color = palette::normalise(&config.windows.pin_color, palette::PIN_DEFAULT);
        if !config.restore_session {
            config.desks.values_mut().for_each(|desk| desk.last_session = None);
        }
        Ok(config)
    }

    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("Seam").join("config.json")
    }

    pub fn load() -> Config {
        Self::load_from(&Self::path())
    }

    fn load_from(p: &Path) -> Config {
        let Ok(bytes) = std::fs::read(p) else { return Config::default() };
        match serde_json::from_slice::<Config>(&bytes) {
            Ok(mut c) => {
                if c.version < CURRENT_VERSION {
                    keep_v1_copy(p, &bytes);
                    c.migrate_v1();
                }
                c
            }
            Err(_) => {
                // Set it aside: the next save would otherwise destroy the layouts in it.
                let _ = std::fs::rename(p, p.with_extension("json.corrupt"));
                Config::default()
            }
        }
    }

    /// Read a file the user chose to import. Old files convert, as on load.
    pub fn from_import(bytes: &[u8]) -> Result<Config, String> {
        let mut c: Config = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if c.version < CURRENT_VERSION {
            c.migrate_v1();
        }
        Ok(c)
    }

    /// Each v1 layout becomes a desk. Tray left-click stays on Line Up, so
    /// nobody's habit breaks.
    fn migrate_v1(&mut self) {
        for (sig, borders) in std::mem::take(&mut self.layouts) {
            let name = Desk::placeholder(sig.split('|').count());
            self.desks.entry(sig).or_insert_with(|| Desk { name, borders, ..Desk::default() });
        }
        self.tray_click = TrayClick::LineUp;
        self.version = CURRENT_VERSION;
    }

    pub fn save(&self) {
        self.save_to(&Self::path());
    }

    fn save_to(&self, p: &Path) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = write_atomic(p, &s);
        }
    }
}

/// A v0.1 build can't read a v2 file, so keep the original to go back to. Only
/// the first copy counts: a later one could already hold another run's changes.
fn keep_v1_copy(config: &Path, bytes: &[u8]) {
    let backup = config.with_file_name("config.v1.json.bak");
    if !backup.exists() {
        if let Err(e) = std::fs::write(&backup, bytes) {
            log::info!("config.v1.json.bak not written: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::{note_active, Mon};
    use crate::geom::Rect;

    fn border(a: &str, b: &str, side: Side, s: f64, o: f64) -> SavedBorder {
        SavedBorder { a: a.into(), b: b.into(), side, map: EdgeMap { s, o }, glued: false, not_on_desk: false }
    }

    fn mon(name: &str) -> Mon {
        Mon {
            id: name.into(),
            gdi: String::new(),
            name: name.into(),
            named: true,
            rect: Rect { left: 0, top: 0, right: 1, bottom: 1 },
            raw_dpi: 96.0,
            primary: false,
        }
    }

    #[test]
    fn signature_is_order_independent() {
        assert_eq!(signature(["b", "a"].into_iter()), signature(["a", "b"].into_iter()));
    }

    #[test]
    fn old_files_get_defaults() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert!(c.enabled && c.align_arrangement && c.desks.is_empty());
    }

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        let borders = vec![border("x", "y", Side::Right, 0.66, 12.0)];
        c.desks.insert("x|y".into(), Desk { borders, ..Desk::default() });
        let s = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(back.desks, c.desks);
    }

    #[test]
    fn only_a_glued_border_writes_the_flag() {
        let plain = serde_json::to_value(border("x", "y", Side::Right, 1.0, 0.0)).unwrap();
        assert!(plain.get("glued").is_none());
        let glued = SavedBorder { glued: true, ..border("x", "y", Side::Right, 1.0, 0.0) };
        let json = serde_json::to_string(&glued).unwrap();
        assert_eq!(serde_json::from_str::<SavedBorder>(&json).unwrap(), glued);
    }

    /// A private empty directory holding `config.json` with the given bytes.
    fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("seam-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn partial_files_keep_their_values_and_default_the_rest() {
        let c: Config = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
        assert!(!c.enabled && c.align_arrangement && c.seen.is_empty());
        let c: Config = serde_json::from_str(r#"{"seen": ["x"], "from_the_future": 3}"#).unwrap();
        assert!(c.enabled && c.seen == ["x"]);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let p = std::env::temp_dir().join("seam-test-no-such-dir").join("config.json");
        let c = Config::load_from(&p);
        assert!(c.enabled && c.desks.is_empty());
    }

    #[test]
    fn corrupt_files_fall_back_to_defaults() {
        let cases: [(&str, &[u8]); 6] = [
            ("empty", b""),
            ("garbage", b"not json at all"),
            ("truncated", br#"{"enabled": false, "lay"#),
            ("wrong-type", br#"{"enabled": "yes"}"#),
            ("bad-side", br#"{"enabled": false, "layouts": {"k": [{"a": "x", "b": "y", "side": "Up", "map": {"s": 1, "o": 0}}]}}"#),
            ("not-utf8", &[0xff, 0xfe, 0x00, 0x7b]),
        ];
        for (name, bytes) in cases {
            let p = scratch(name, bytes);
            let c = Config::load_from(&p);
            assert!(c.enabled && c.align_arrangement && c.desks.is_empty(), "{name}");
            let _ = std::fs::remove_dir_all(p.parent().unwrap());
        }
    }

    #[test]
    fn save_then_load_on_disk() {
        let p = scratch("save", b"");
        let mut c = Config { enabled: false, ..Config::default() };
        c.seen.push("sig".into());
        c.save_to(&p);
        let back = Config::load_from(&p);
        assert!(!back.enabled && back.seen == ["sig"]);
        assert!(!p.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn changing_one_field_keeps_saved_layouts() {
        let p = scratch("edit", br#"{"layouts": {"desk": []}, "seen": ["desk"]}"#);
        let mut c = Config::load_from(&p);
        c.enabled = false;
        c.save_to(&p);
        let back = Config::load_from(&p);
        assert!(!back.enabled && back.desks.contains_key("desk") && back.seen == ["desk"]);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    const V1: &str = r#"{
      "enabled": true,
      "align_arrangement": true,
      "layouts": {
        "A|B|C|D": [
          { "a": "B", "b": "A", "side": "Bottom", "map": { "s": 1.2844, "o": 64.0 } },
          { "a": "A", "b": "D", "side": "Right", "map": { "s": 0.7807, "o": 212.0 } }
        ]
      },
      "seen": ["A|B|C|D", "E"]
    }"#;

    #[test]
    fn a_v1_file_becomes_v2_without_losing_anything() {
        let p = scratch("migrate", V1.as_bytes());
        let backup = p.with_file_name("config.v1.json.bak");
        let c = Config::load_from(&p);

        assert_eq!(c.version, 2);
        assert_eq!(c.tray_click, TrayClick::LineUp);
        assert_eq!(c.seen, ["A|B|C|D", "E"]);
        let desk = &c.desks["A|B|C|D"];
        assert_eq!(desk.name, "Desk with 4 screens");
        assert_eq!(desk.last_seen, 0);
        assert_eq!(desk.lined_up_at, None);
        assert_eq!(desk.borders.len(), 2);
        assert_eq!(desk.borders[0], border("B", "A", Side::Bottom, 1.2844, 64.0));
        assert_eq!(desk.borders[1], border("A", "D", Side::Right, 0.7807, 212.0));

        // The first save writes v2 only, and the original stays in the backup.
        c.save_to(&p);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), V1);
        let saved = std::fs::read_to_string(&p).unwrap();
        assert!(!saved.contains("layouts"), "{saved}");
        let back = Config::load_from(&p);
        assert_eq!((back.version, back.tray_click), (2, TrayClick::LineUp));
        assert_eq!(back.desks, c.desks);
        assert!(!saved.contains("lined_up_at"), "{saved}");

        // Connecting the desk names it, and a name the user chose stays.
        let mons: Vec<Mon> = ["PG32UCDM", "LG HDR QHD", "HDMI panel", "U3477"].map(mon).into();
        let mut named = back.clone();
        note_active(&mut named, "A|B|C|D", &mons, 1793890000);
        assert_eq!(named.desks["A|B|C|D"].name, "PG32UCDM + 3 more");
        assert_eq!(named.desks["A|B|C|D"].last_seen, 1793890000);
        named.desks.get_mut("A|B|C|D").unwrap().name = "Home desk".into();
        note_active(&mut named, "A|B|C|D", &mons, 1793890001);
        assert_eq!(named.desks["A|B|C|D"].name, "Home desk");

        // A later v1 file must not replace the first backup.
        std::fs::write(&p, r#"{"enabled": false}"#).unwrap();
        Config::load_from(&p);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), V1);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_is_not_destroyed_by_the_next_save() {
        let original = br#"{"layouts": {"my-desk": [ damaged"#;
        let p = scratch("keep", original);
        Config::load_from(&p).save_to(&p);
        let dir = p.parent().unwrap();
        let kept =
            std::fs::read_dir(dir).unwrap().any(|e| std::fs::read(e.unwrap().path()).is_ok_and(|b| b == original));
        let _ = std::fs::remove_dir_all(dir);
        assert!(kept, "the only copy of the saved layouts is gone");
    }
}
