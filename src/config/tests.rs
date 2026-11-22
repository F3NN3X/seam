//! The tests for the config file.

use super::*;
use crate::desk::{Mon, note_active};
use crate::geom::Rect;
use serde_json::json;

#[test]
fn sign_in_migrates_once_and_patches_merge() {
    let mut config = Config { restore_session: true, ..Config::default() };
    config.desks.insert("a".into(), Desk::default());
    config.migrate_sign_in();
    assert!(config.keeps_session("a"));
    // A desk seen later starts at Nothing, and a second migration leaves the first alone.
    config.desks.insert("b".into(), Desk::default());
    config.migrate_sign_in();
    assert!(!config.keeps_session("b"));
    // A partial patch keeps the mode, and the old switch states every desk's mode.
    let asked = config.patched(&json!({ "desks": { "a": { "sign_in": { "ask": false } } } })).unwrap();
    assert!(asked.keeps_session("a") && !asked.desks["a"].choice().ask);
    let off = config.patched(&json!({ "restore_session": false })).unwrap();
    assert!(!off.keeps_session("a") && !off.restore_session);
}

#[test]
fn margins_are_per_screen_and_an_older_desk_has_none() {
    let desk: Desk = serde_json::from_value(json!({ "name": "d" })).unwrap();
    assert!(desk.margins.is_empty());
    assert!(!serde_json::to_string(&desk).unwrap().contains("margins"));
    let mut config = Config::default();
    config.desks.insert("a".into(), desk);
    let patched = config.patched(&json!({ "desks": { "a": { "margins": { "m1": { "top": 12, "left": 4 } } } } }));
    let m = patched.unwrap().desks["a"].margins["m1"];
    assert_eq!(m.sides(), [12, 0, 0, 4]);
}

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
fn an_older_file_has_no_stack_keys_and_an_older_state_no_stacks() {
    let c: Config = serde_json::from_str(r#"{"hotkeys": {"line_up": "Ctrl+Alt+L"}}"#).unwrap();
    assert_eq!((c.hotkeys.stack_next, c.hotkeys.stack_prev), (None, None));
    let s: crate::state::State = serde_json::from_str(r#"{"seam_version": "0.8.0"}"#).unwrap();
    assert!(s.stacks.is_empty());
    let one = crate::state::StackRef { screen: 1, zone: 2, span_to: None, windows: vec![5, 6], top: 6 };
    assert!(serde_json::to_value(&one).unwrap().get("span_to").is_none());
}

#[test]
fn an_older_file_tiles_nothing_and_an_empty_tiling_isnt_written() {
    let c: Config = serde_json::from_str(r#"{"desks": {"d": {"name": "Desk"}}}"#).unwrap();
    assert!(c.desks["d"].tiling.is_empty());
    assert_eq!((c.hotkeys.float_tile, c.hotkeys.free_tiling, c.hotkeys.overview), (None, None, None));
    assert!(serde_json::to_value(&c.desks["d"]).unwrap().get("tiling").is_none());
    let on: Desk = serde_json::from_str(r#"{"tiling": {"screen-a": {"desktops": ["x"]}}}"#).unwrap();
    let screen = &on.tiling["screen-a"];
    assert_eq!(screen.family, crate::tiling::Family::Smart);
    assert!(screen.on("x") && !screen.on("y"));
}

#[test]
fn scrolling_is_saved_so_an_older_seam_reads_smart() {
    let scrolling = TileScreen { family: crate::tiling::Family::Scrolling, desktops: vec!["x".into()] };
    let saved = serde_json::to_value(&scrolling).unwrap();
    assert_eq!((saved["family"].as_str(), saved["scrolling"].as_bool()), (Some("smart"), Some(true)));
    let back: TileScreen = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back, scrolling);
    // An older Seam's own type, which ignores the field it doesn't know.
    #[derive(serde::Deserialize)]
    struct Older {
        family: String,
    }
    let older: Older = serde_json::from_value(saved).unwrap();
    assert_eq!(older.family, "smart");
    // Another family writes no `scrolling`, and the keys and hooks of Scrolling are unset in an older file.
    let grid = TileScreen { family: crate::tiling::Family::Grid, desktops: Vec::new() };
    assert!(serde_json::to_value(&grid).unwrap().get("scrolling").is_none());
    let c: Config = serde_json::from_str(r#"{"hotkeys": {"overview": "Ctrl+Alt+O"}}"#).unwrap();
    let keys = &c.hotkeys;
    assert!(
        [&keys.scroll_left, &keys.scroll_right, &keys.column_left, &keys.column_right, &keys.column_width]
            .iter()
            .all(|k| k.is_none())
    );
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
        (
            "bad-side",
            br#"{"enabled": false, "layouts": {"k": [{"a": "x", "b": "y", "side": "Up", "map": {"s": 1, "o": 0}}]}}"#,
        ),
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
    let kept = std::fs::read_dir(dir).unwrap().any(|e| std::fs::read(e.unwrap().path()).is_ok_and(|b| b == original));
    let _ = std::fs::remove_dir_all(dir);
    assert!(kept, "the only copy of the saved layouts is gone");
}

// What I typed in "Open with" stays on this PC: an imported file never brings it, and
// a client that is not the panel never reads it.
#[test]
fn open_with_is_dropped_on_import_and_hidden_from_other_clients() {
    let row = json!({ "exe": r"C:\a.exe", "open_with": r"C:\x", "place": { "folder": r"C:\y" } });
    let desk = json!({ "workspaces": [{ "id": "w", "windows": [row] }] });
    let file = json!({ "version": CURRENT_VERSION, "desks": { "a": desk } });
    let imported = Config::from_import(file.to_string().as_bytes()).unwrap();
    let desk = &imported.desks["a"];
    assert!(desk.workspaces[0]["windows"][0].get("open_with").is_none());

    let config = json!({ "desks": { "a": { "workspaces": [{ "id": "w", "windows": [{ "exe": r"C:\a.exe", "open_with": r"C:\x" }] }] } } });
    let seen = crate::api::config_for_others(&config);
    assert!(seen["desks"]["a"]["workspaces"][0]["windows"][0].get("open_with").is_none());
    assert_eq!(seen["desks"]["a"]["workspaces"][0]["windows"][0]["exe"], r"C:\a.exe");
    // The saved config, which the panel reads whole, still has it.
    assert_eq!(config["desks"]["a"]["workspaces"][0]["windows"][0]["open_with"], r"C:\x");
}

#[test]
fn an_older_file_has_no_rotate_or_reset_key() {
    let c: Config = serde_json::from_str(r#"{"hotkeys": {"overview": "Ctrl+Alt+O"}}"#).unwrap();
    assert_eq!((c.hotkeys.rotate, c.hotkeys.reset_sizes), (None, None));
    let set: Config = serde_json::from_str(r#"{"hotkeys": {"rotate": "Ctrl+Alt+R"}}"#).unwrap();
    assert_eq!(set.hotkeys.rotate.as_deref(), Some("Ctrl+Alt+R"));
}

#[test]
fn an_older_file_has_no_add_to_workspace_keys() {
    let c: Config = serde_json::from_str(r#"{"hotkeys": {"overview": "Ctrl+Alt+O"}}"#).unwrap();
    assert!(c.hotkeys.add_to_workspace().iter().all(Option::is_none));
    let set: Config = serde_json::from_str(r#"{"hotkeys": {"add_to_3": "Ctrl+Alt+3"}}"#).unwrap();
    assert_eq!(set.hotkeys.add_to_workspace()[2].map(String::as_str), Some("Ctrl+Alt+3"));
}

#[test]
fn quitting_clears_tiling_for_every_freed_desktop_and_an_old_config_has_the_switch_off() {
    let on = |ids: &[&str]| TileScreen { desktops: ids.iter().map(|d| d.to_string()).collect(), ..Default::default() };
    let mut tiling = BTreeMap::from([("a".to_string(), on(&["d1", "d2"])), ("b".to_string(), on(&["d1"]))]);
    // Screen b keeps its family, and desktops never tiled this session go too.
    tiling.insert("b".into(), TileScreen { family: crate::tiling::Family::Grid, ..on(&["d3"]) });
    clear_tiling(&mut tiling);
    assert_eq!(tiling.len(), 1);
    assert!(tiling["b"].desktops.is_empty());
    let old: Config = serde_json::from_value(json!({ "zones": { "gap": 4 } })).unwrap();
    assert!(!old.zones.tiling_free_on_quit);
}

#[test]
fn an_older_file_has_the_corner_overview_off() {
    let old: Config = serde_json::from_value(json!({ "windows": { "alt_drag": { "on": true } } })).unwrap();
    assert!(!old.windows.corner_overview.on);
    assert_eq!(old.windows.corner_overview.corner, "top_left");
    let set: Config = serde_json::from_value(json!({ "windows": { "corner_overview": { "on": true } } })).unwrap();
    assert!(set.windows.corner_overview.on);
}

#[test]
fn setup_is_done_for_a_file_from_before_it_and_not_for_a_new_install() {
    let old: Config = serde_json::from_str(r#"{"version": 2, "enabled": false}"#).unwrap();
    assert!(old.setup_done);
    assert!(!Config::default().setup_done);
    let kept = Config::default().patched(&json!({ "setup_done": true })).unwrap();
    assert!(kept.setup_done);
    // A new install's file says so, so it reads back as not done.
    let back: Config = serde_json::from_str(&serde_json::to_string(&Config::default()).unwrap()).unwrap();
    assert!(!back.setup_done);
}

#[test]
fn a_0_7_0_config_loads_with_its_values_and_counts_as_set_up() {
    // The shape 0.7.0 wrote: a desk with borders, zones and a workspace, a hotkey, an app
    // rule and no `setup_done`. Only fields `v0.7.0:src/config.rs` has.
    let file = json!({
        "version": 2,
        "enabled": false,
        "tray_click": "line_up",
        "hotkeys": { "line_up": "Ctrl+Shift+Alt+L", "pause": null, "move_left": "Shift+Win+Alt+Left" },
        "zones": { "gap": 8, "show_key": "ctrl" },
        "app_rules": [{ "exe": "C:\\Apps\\notes.exe", "class": "Notes", "monitor": "mon-a" }],
        "pointer": { "edge_resistance_px": 30, "glue_push_px": 60 },
        "seen": ["mon-a|mon-b"],
        "desks": {
            "mon-a|mon-b": {
                "name": "Desk",
                "last_seen": 1_730_000_000u64,
                "lined_up_at": 1_730_000_000u64,
                "borders": [{ "a": "mon-a", "b": "mon-b", "side": "Right", "map": { "s": 1.0, "o": 12.5 } }],
                "zones": { "mon-a": { "id": "two", "name": "Two" } },
                "workspaces": [{ "id": "w1", "name": "Work", "windows": [] }]
            }
        }
    });
    let p = scratch("v0_7_0", file.to_string().as_bytes());
    let c = Config::load_from(&p);
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
    assert_eq!(c.version, CURRENT_VERSION);
    assert!(c.setup_done);
    assert!(!c.enabled);
    assert_eq!(c.tray_click, TrayClick::LineUp);
    assert_eq!(c.hotkeys.pause, None);
    assert_eq!(c.hotkeys.move_left.as_deref(), Some("Shift+Win+Alt+Left"));
    assert_eq!((c.zones.gap, c.zones.show_key.as_str()), (8, "ctrl"));
    assert_eq!(c.app_rules.len(), 1);
    assert_eq!((c.pointer.edge_resistance_px, c.pointer.glue_push_px), (30, 60));
    assert_eq!(c.seen, ["mon-a|mon-b"]);
    let desk = &c.desks["mon-a|mon-b"];
    assert_eq!((desk.name.as_str(), desk.last_seen, desk.lined_up_at), ("Desk", 1_730_000_000, Some(1_730_000_000)));
    assert_eq!(desk.borders.len(), 1);
    assert_eq!((desk.borders[0].side, desk.borders[0].map.o), (Side::Right, 12.5));
    assert!(desk.zones.contains_key("mon-a"));
    assert_eq!(desk.workspaces.len(), 1);
}

#[test]
fn hotkeys_without_close_and_minimize_load_unchanged() {
    let c: Config = serde_json::from_str(r#"{"hotkeys": {"line_up": "Ctrl+Alt+L"}}"#).unwrap();
    assert_eq!((c.hotkeys.close_workspace, c.hotkeys.minimize_workspace), (None, None));
    assert_eq!(c.hotkeys.line_up.as_deref(), Some("Ctrl+Alt+L"));
    let set: Config =
        serde_json::from_str(r#"{"hotkeys": {"close_workspace": "Ctrl+Alt+X", "minimize_workspace": "Ctrl+Alt+M"}}"#)
            .unwrap();
    assert_eq!(set.hotkeys.close_workspace.as_deref(), Some("Ctrl+Alt+X"));
    assert_eq!(set.hotkeys.minimize_workspace.as_deref(), Some("Ctrl+Alt+M"));
}

#[test]
fn old_removes_flag_maps_to_the_choice() {
    let read = |v: serde_json::Value| serde_json::from_value::<Config>(v).unwrap().closed_apps();
    assert_eq!(read(json!({ "workspaces_update_removes_closed": true })), ClosedApps::Remove);
    assert_eq!(read(json!({ "workspaces_update_removes_closed": false })), ClosedApps::Keep);
    assert_eq!(read(json!({})), ClosedApps::Keep);
    // The choice wins over the flag once it is there.
    let both = json!({ "workspaces_update_closed": "turn_off", "workspaces_update_removes_closed": true });
    assert_eq!(read(both), ClosedApps::TurnOff);
}

#[test]
fn writing_a_choice_writes_the_old_flag() {
    for (choice, flag) in [(ClosedApps::Keep, false), (ClosedApps::TurnOff, false), (ClosedApps::Remove, true)] {
        let c = Config::default().patched(&choice.patch()).unwrap();
        assert_eq!((c.closed_apps(), c.workspaces_update_removes_closed), (choice, flag));
    }
}

#[test]
fn new_field_is_skipped_when_unset() {
    let written = serde_json::to_value(Config::default()).unwrap();
    assert!(written.get("workspaces_update_closed").is_none());
    assert_eq!(written["workspaces_update_removes_closed"], false);
    let set = serde_json::to_value(Config::default().patched(&ClosedApps::TurnOff.patch()).unwrap()).unwrap();
    assert_eq!(set["workspaces_update_closed"], "turn_off");
}

const SEALED_ROW: &str = r#"{ "exe": "C:\\wt.exe", "aumid": "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App", "item": { "kind": "terminal", "tabs": [
    { "profile": "PowerShell", "folder": "C:\\p", "command_sealed": "c2VhbGVk" } ] } }"#;

// A startup command stays on this PC and in this account: another client reads only the kind
// of an item, and the panel the command in clear.
#[test]
fn config_get_shows_only_kind_to_other_clients() {
    let row: serde_json::Value = serde_json::from_str(SEALED_ROW).unwrap();
    let config = json!({ "desks": { "a": { "workspaces": [{ "id": "w", "windows": [row] }] } } });
    let seen = crate::api::config_for_others(&config);
    let item = &seen["desks"]["a"]["workspaces"][0]["windows"][0]["item"];
    assert_eq!(item, &json!({ "kind": "terminal" }));
    assert!(!seen.to_string().contains("c2VhbGVk") && !seen.to_string().contains("PowerShell"));
    let panel = crate::api::config_for_panel(&config, &|s| (s == "c2VhbGVk").then(|| "npm run dev".to_string()));
    let tab = &panel["desks"]["a"]["workspaces"][0]["windows"][0]["item"]["tabs"][0];
    assert_eq!(tab["command"], "npm run dev");
    assert!(tab.get("command_sealed").is_none());
}

#[test]
fn a_settings_file_brings_no_startup_commands() {
    let row: serde_json::Value = serde_json::from_str(SEALED_ROW).unwrap();
    let desk = json!({ "workspaces": [{ "id": "w", "windows": [row] }] });
    let file = json!({ "version": CURRENT_VERSION, "desks": { "a": desk } });
    let imported = Config::from_import(file.to_string().as_bytes()).unwrap();
    let tab = &imported.desks["a"].workspaces[0]["windows"][0]["item"]["tabs"][0];
    assert!(tab.get("command_sealed").is_none() && tab.get("command").is_none());
    assert_eq!(tab["profile"], "PowerShell");
}

#[test]
fn a_config_file_loads_without_clear_commands_or_session_items() {
    let tab = json!({ "profile": "P", "command": "calc", "unreadable": true, "command_sealed": "c2Vh" });
    let row = json!({ "exe": "C:\\wt.exe", "item": { "kind": "terminal", "tabs": [tab] } });
    let desk = json!({ "sign_in": { "mode": "last_session" }, "workspaces": [{ "id": "w", "windows": [row.clone()] }], "last_session": { "id": "s", "windows": [row] } });
    let file = json!({ "version": CURRENT_VERSION, "desks": { "a": desk } });
    let path = std::env::temp_dir().join(format!("seam-load-{}.json", std::process::id()));
    std::fs::write(&path, file.to_string()).unwrap();
    let loaded = Config::load_from(&path);
    let _ = std::fs::remove_file(&path);
    let desk = &loaded.desks["a"];
    let tab = &desk.workspaces[0]["windows"][0]["item"]["tabs"][0];
    assert!(tab.get("command").is_none() && tab.get("unreadable").is_none());
    assert_eq!(tab["command_sealed"], "c2Vh");
    assert!(desk.last_session.as_ref().unwrap()["windows"][0].get("item").is_none());
}

#[test]
fn desks_without_screens_load_unchanged() {
    let file = json!({ "version": CURRENT_VERSION, "desks": { "a|b": { "name": "Desk" } } });
    let loaded: Config = serde_json::from_value(file).unwrap();
    assert!(loaded.desks["a|b"].screens.is_empty());
    // And nothing is written for them.
    let saved = serde_json::to_value(&loaded).unwrap();
    assert!(saved["desks"]["a|b"].get("screens").is_none());
}

#[test]
fn screens_round_trip() {
    let screens = vec![
        DeskScreen {
            id: "a".into(),
            number: 1,
            name: "Left".into(),
            rect: [0, 0, 1920, 1080],
            primary: true,
            mm: None,
        },
        DeskScreen {
            id: "b".into(),
            number: 2,
            name: "Right".into(),
            rect: [1920, -10, 2560, 1440],
            primary: false,
            mm: Some([530.5, 0.0, 600.0, 340.0]),
        },
    ];
    let mut config = Config::default();
    config.desks.insert("a|b".into(), Desk { screens: screens.clone(), ..Desk::default() });
    let saved = serde_json::to_value(&config).unwrap();
    // A false main screen and an unknown position are left out.
    assert!(saved["desks"]["a|b"]["screens"][1].get("primary").is_none());
    assert!(saved["desks"]["a|b"]["screens"][0].get("mm").is_none());
    let back: Config = serde_json::from_value(saved).unwrap();
    assert_eq!(back.desks["a|b"].screens, screens);
}

#[test]
fn offer_restore_defaults_off_and_is_skipped_when_off() {
    assert!(!Config::default().workspaces_offer_restore);
    let saved = serde_json::to_value(Config::default()).unwrap();
    assert!(saved.get("workspaces_offer_restore").is_none());
    let on: Config = serde_json::from_value(json!({ "workspaces_offer_restore": true })).unwrap();
    assert!(on.workspaces_offer_restore);
    assert_eq!(serde_json::to_value(&on).unwrap()["workspaces_offer_restore"], true);
    let older: Config = serde_json::from_value(json!({})).unwrap();
    assert!(!older.workspaces_offer_restore);
}

#[test]
fn an_older_file_has_no_step_keys() {
    let c: Config = serde_json::from_str(r#"{"hotkeys": {"line_up": "Ctrl+Alt+L"}}"#).unwrap();
    let h = c.hotkeys;
    assert_eq!((h.workspace_next, h.workspace_prev, h.add_to_next, h.add_to_prev), (None, None, None, None));
}

#[test]
fn step_keys_round_trip_and_are_skipped_when_unset() {
    let saved = serde_json::to_value(Config::default()).unwrap();
    for key in ["workspace_next", "workspace_prev", "add_to_next", "add_to_prev"] {
        assert!(saved["hotkeys"].get(key).is_none(), "{key} was written while unset");
    }
    let set: Config = serde_json::from_value(json!({ "hotkeys": {
        "workspace_next": "Ctrl+Alt+Right", "workspace_prev": "Ctrl+Alt+Left",
        "add_to_next": "Ctrl+Shift+Alt+Right", "add_to_prev": "Ctrl+Shift+Alt+Left"
    } }))
    .unwrap();
    let back: Config = serde_json::from_value(serde_json::to_value(&set).unwrap()).unwrap();
    assert_eq!(back.hotkeys.workspace_next.as_deref(), Some("Ctrl+Alt+Right"));
    assert_eq!(back.hotkeys.workspace_prev.as_deref(), Some("Ctrl+Alt+Left"));
    assert_eq!(back.hotkeys.add_to_next.as_deref(), Some("Ctrl+Shift+Alt+Right"));
    assert_eq!(back.hotkeys.add_to_prev.as_deref(), Some("Ctrl+Shift+Alt+Left"));
}

#[test]
fn switch_label_defaults_on_and_is_skipped_while_on() {
    assert!(Config::default().workspaces_switch_label);
    let saved = serde_json::to_value(Config::default()).unwrap();
    assert!(saved.get("workspaces_switch_label").is_none());
    let older: Config = serde_json::from_value(json!({})).unwrap();
    assert!(older.workspaces_switch_label);
}

#[test]
fn switch_label_off_round_trips() {
    let off: Config = serde_json::from_value(json!({ "workspaces_switch_label": false })).unwrap();
    assert!(!off.workspaces_switch_label);
    let saved = serde_json::to_value(&off).unwrap();
    assert_eq!(saved["workspaces_switch_label"], false);
    let back: Config = serde_json::from_value(saved).unwrap();
    assert!(!back.workspaces_switch_label);
}
