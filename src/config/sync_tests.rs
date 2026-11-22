//! The tests for the settings folder (brief G).

use super::folder::{self, Push};
use super::sync::*;
use super::*;
use proptest::prelude::*;
use serde_json::{Value, json};

const SHARED_TOP: &[&str] = &[
    "enabled",
    "align_arrangement",
    "lineup_measure",
    "ask_new_desks",
    "window_size_on_move",
    "window_size_on_drag",
    "step_aside_fullscreen",
    "zones",
    "windows",
    "auto_apply",
    "workspaces_remember_places",
    "workspaces_update_removes_closed",
    "workspaces_update_closed",
    "workspaces_offer_restore",
    "app_rules",
    "custom_layouts",
    "pointer",
    "restore_session",
    "desks",
];
const SHARED_DESK: &[&str] = &[
    "name",
    "borders",
    "lined_up_at",
    "zones",
    "workspaces",
    "sign_in",
    "default_workspace",
    "tiling",
    "margins",
    "screens",
];

fn every_field_set() -> Value {
    let desk: Desk = serde_json::from_value(json!({
        "name": "d", "last_seen": 1, "borders": [], "lined_up_at": 2, "zones": { "m": {} },
        "workspaces": [{ "id": "w" }], "last_session": { "id": "s" },
        "sign_in": { "mode": "nothing", "ask": true }, "active_workspace": "w", "previous_workspace": "w",
        "default_workspace": "w", "tiling": { "m": {} }, "margins": { "m": {} }, "screens": [{ "id": "m" }],
    }))
    .unwrap();
    let config = Config {
        setup_done: true,
        workspaces_update_closed: Some(ClosedApps::Keep),
        workspaces_offer_restore: true,
        custom_layouts: vec![json!(1)],
        restore_apps_saved: Some(1),
        seen: vec!["x".into()],
        desks: BTreeMap::from([("sig".to_string(), desk)]),
        config_folder: Some(r"D:\Sync".into()),
        sync: Some(SyncStamp { pc: "PC".into(), at: 1 }),
        ..Config::default()
    };
    serde_json::to_value(config).unwrap()
}

fn keys(v: &Value) -> BTreeSet<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

use std::collections::BTreeSet;

#[test]
fn every_config_field_is_classified() {
    let all = every_field_set();
    let known: BTreeSet<&str> =
        LOCAL_TOP.iter().chain(SHARED_TOP).chain(&["version", "sync"]).copied().filter(|k| *k != "layouts").collect();
    // A field added to Config that is in no list fails here: place it in LOCAL_TOP or in SHARED_TOP.
    assert_eq!(keys(&all), known);
    let desk = &all["desks"]["sig"];
    let known: BTreeSet<&str> = LOCAL_DESK.iter().chain(SHARED_DESK).copied().collect();
    assert_eq!(keys(desk), known);
    // And the cut agrees with the lists.
    let (shared, local) = split(&all);
    let want: BTreeSet<&str> = SHARED_TOP.iter().chain(&["version", "sync"]).copied().collect();
    assert_eq!(keys(&shared), want);
    assert_eq!(keys(&shared["desks"]["sig"]), SHARED_DESK.iter().copied().collect());
    assert_eq!(keys(&local["desks"]["sig"]), LOCAL_DESK.iter().copied().collect());
}

#[test]
fn local_fields_never_reach_the_shared_file() {
    let (shared, local) = split(&every_field_set());
    for key in ["hotkeys", "api", "updates", "seen", "config_folder", "setup_done", "tray_click", "debug_log"] {
        assert!(shared.get(key).is_none(), "{key}");
    }
    for key in LOCAL_DESK {
        assert!(shared["desks"]["sig"].get(*key).is_none(), "{key}");
        assert!(local["desks"]["sig"].get(*key).is_some(), "{key}");
    }
    assert!(local.get("sync").is_none() && local.get("zones").is_none());
    // A file from the folder can't carry them in either.
    let hostile = json!({ "hotkeys": { "lineup": "Ctrl+A" }, "api": { "enabled": true }, "config_folder": "C:\\x",
        "desks": { "sig": { "name": "n", "last_seen": 9, "active_workspace": "evil" } } });
    let mine = json!({ "hotkeys": { "lineup": "Win+Z" }, "version": 2,
        "desks": { "sig": { "last_seen": 3, "active_workspace": "mine" } } });
    let joined = join(&hostile, &mine);
    assert_eq!(joined["hotkeys"]["lineup"], "Win+Z");
    assert!(joined.get("api").is_none() && joined.get("config_folder").is_none());
    assert_eq!(joined["desks"]["sig"]["last_seen"], 3);
    assert_eq!(joined["desks"]["sig"]["active_workspace"], "mine");
    let mut taken = hostile.clone();
    intake(None, &mut taken);
    assert!(taken.get("hotkeys").is_none() && taken["desks"]["sig"].get("last_seen").is_none());
}

fn config_with(
    enabled: bool,
    zones_on: bool,
    debug: bool,
    names: &[String],
    seen: u64,
    folder: Option<String>,
) -> Value {
    let mut config = Config { enabled, debug_log: debug, config_folder: folder, ..Config::default() };
    config.zones.enabled = zones_on;
    for (n, name) in names.iter().enumerate() {
        let desk = Desk { name: name.clone(), last_seen: seen + n as u64, ..Desk::default() };
        config.desks.insert(format!("sig{n}"), desk);
    }
    serde_json::to_value(config).unwrap()
}

proptest! {
    #[test]
    fn split_and_join_round_trip(
        enabled: bool, zones_on: bool, debug: bool, seen in 0u64..1_000_000,
        names in proptest::collection::vec("[a-z ]{0,6}", 0..4),
        folder in proptest::option::of("[A-Z]:\\\\[a-z]{1,5}"),
    ) {
        let whole = config_with(enabled, zones_on, debug, &names, seen, folder);
        let (shared, local) = split(&whole);
        prop_assert_eq!(join(&shared, &local), whole);
    }
}

fn shared(names: &[(&str, &str)], enabled: bool) -> Value {
    let desks: serde_json::Map<String, Value> = names
        .iter()
        .map(|(sig, name)| ((*sig).to_string(), json!({ "name": name, "zones": {}, "workspaces": [] })))
        .collect();
    json!({ "version": 2, "enabled": enabled, "auto_apply": false, "desks": desks })
}

#[test]
fn merge_takes_one_sided_changes_from_either_side() {
    let base = shared(&[("a", "Home")], true);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours["enabled"] = json!(false);
    theirs["auto_apply"] = json!(true);
    let merged = merge3(&base, &ours, &theirs);
    assert!(merged.clashed.is_empty());
    assert_eq!(merged.value["enabled"], false);
    assert_eq!(merged.value["auto_apply"], true);
    // A desk theirs added comes in, and one theirs removed goes.
    let mut theirs = shared(&[("a", "Home"), ("b", "Office")], true);
    assert_eq!(merge3(&base, &base, &theirs).value["desks"]["b"]["name"], "Office");
    theirs["desks"].as_object_mut().unwrap().remove("a");
    assert!(merge3(&base, &base, &theirs).value["desks"].get("a").is_none());
}

#[test]
fn merge_clash_takes_theirs_and_reports_the_key() {
    let base = shared(&[("a", "Home")], true);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours["enabled"] = json!(false);
    theirs["enabled"] = json!("x");
    ours["desks"]["a"]["name"] = json!("Mine");
    theirs["desks"]["a"]["name"] = json!("Theirs");
    let merged = merge3(&base, &ours, &theirs);
    assert_eq!(merged.value["enabled"], "x");
    assert_eq!(merged.value["desks"]["a"]["name"], "Theirs");
    assert_eq!(merged.clashed, ["enabled", "desks.a.name"]);
    // Both sides making the same change isn't a clash.
    theirs["enabled"] = json!(false);
    assert!(!merge3(&base, &ours, &theirs).clashed.contains(&"enabled".to_string()));
}

#[test]
fn merge_is_per_desk_field() {
    let base = shared(&[("a", "Home"), ("b", "Office")], true);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours["desks"]["a"]["workspaces"] = json!([{ "id": "w1" }]);
    theirs["desks"]["b"]["workspaces"] = json!([{ "id": "w2" }]);
    // The same desk, a different field.
    ours["desks"]["b"]["zones"] = json!({ "m": 1 });
    let merged = merge3(&base, &ours, &theirs);
    assert!(merged.clashed.is_empty());
    assert_eq!(merged.value["desks"]["a"]["workspaces"][0]["id"], "w1");
    assert_eq!(merged.value["desks"]["b"]["workspaces"][0]["id"], "w2");
    assert_eq!(merged.value["desks"]["b"]["zones"]["m"], 1);
}

#[test]
fn older_seam_saves_merge_as_changes_here() {
    // An older Seam changed the local copy and never wrote the folder: the file there is still the base.
    let base = shared(&[("a", "Home")], true);
    let mut ours = base.clone();
    ours["desks"]["a"]["name"] = json!("Renamed");
    ours["auto_apply"] = json!(true);
    let merged = merge3(&base, &ours, &base);
    assert_eq!(merged.value, ours);
    assert!(merged.clashed.is_empty());
}

#[test]
fn partial_or_empty_folder_file_is_retried_not_applied() {
    let good = serde_json::to_vec(&Config::default()).unwrap();
    assert!(matches!(attempt(&good, 0), Attempt::Take(_)));
    let half = &good[..good.len() / 2];
    for bytes in [&b""[..], half, b"not json", b"[1, 2]"] {
        for tries in 0..READ_TRIES - 1 {
            assert!(matches!(attempt(bytes, tries), Attempt::Retry), "{bytes:?} on try {tries}");
        }
        // The fifth read that fails gives up.
        assert!(matches!(attempt(bytes, READ_TRIES - 1), Attempt::GiveUp));
    }
    // One that is too big is refused at once.
    assert!(matches!(attempt(&vec![b' '; MAX_READ + 1], 0), Attempt::GiveUp));
    // A v1 file is migrated as a local load does.
    let v1 = br#"{"layouts": {"desk": []}}"#;
    let Attempt::Take(config) = attempt(v1, 0) else { panic!("v1 file wasn't taken") };
    assert!(config.desks.contains_key("desk") && config.version == CURRENT_VERSION);
}

#[test]
fn newer_version_in_folder_is_not_taken() {
    let newer = json!({ "version": CURRENT_VERSION + 1, "enabled": false }).to_string();
    assert!(matches!(attempt(newer.as_bytes(), 0), Attempt::Newer));
    let same = json!({ "version": CURRENT_VERSION }).to_string();
    assert!(matches!(attempt(same.as_bytes(), 0), Attempt::Take(_)));
}

#[test]
fn conflict_copy_names() {
    for name in [
        "config-DESKTOP-ABC.json",
        "config (1).json",
        "config (conflicted copy 2026-01-02).json",
        "config (Ola's conflicted copy 2026-01-02).json",
        "Config-Laptop.json",
        "config.sync-conflict-20260102-030405-ABCDEFG.json",
    ] {
        assert!(is_conflict_copy(name), "{name}");
    }
    for name in [
        "config.json",
        "config.json.tmp",
        "config.sync-base.json",
        "config.before-sync.json",
        "config.before-import.json",
        "settings.json",
        "config.txt",
        "configs.json",
    ] {
        assert!(!is_conflict_copy(name), "{name}");
    }
}

struct Fake(&'static [char]);

impl Drives for Fake {
    fn fixed(&self, letter: char) -> bool {
        self.0.contains(&letter)
    }
}

#[test]
fn folder_rules() {
    let dir = std::env::temp_dir().join(format!("seam-test-{}-folder", std::process::id()));
    let own = dir.join("own");
    let there = dir.join("there");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&own).unwrap();
    std::fs::create_dir_all(&there).unwrap();
    let file = dir.join("a-file.txt");
    std::fs::write(&file, b"x").unwrap();
    let letter = there.to_string_lossy().chars().next().unwrap().to_ascii_uppercase();
    let text = |p: &Path| p.display().to_string();
    let fixed = Fake(&['C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T']);
    assert!(fixed.fixed(letter), "the test folder is on a fixed drive");
    assert_eq!(folder_ok(&text(&there), &own, &fixed), Ok(there.clone()));
    // Case, a trailing slash and a prefix don't hide Seam's own folder.
    let upper = text(&own).to_uppercase();
    assert_eq!(folder_ok(&upper, &own, &fixed), Err(OWN_FOLDER));
    assert_eq!(folder_ok(&format!("{}\\", text(&own)), &own, &fixed), Err(OWN_FOLDER));
    // A drive that isn't this PC's own: network or removable.
    assert_eq!(folder_ok(&text(&there), &own, &Fake(&[])), Err(NOT_LOCAL));
    // A UNC path, a relative one, `..`, a stream, a path too long.
    for bad in [
        r"\\server\share\Seam",
        r"\\?\C:\Users",
        r"Sync\Seam",
        r"C:Seam",
        &format!(r"{}\..\there", text(&own)),
        &format!(r"{}:stream", text(&there)),
        &format!("C:\\{}", "a".repeat(300)),
        "",
    ] {
        assert_eq!(folder_ok(bad, &own, &fixed), Err(NOT_LOCAL), "{bad}");
    }
    // A file, and a folder that isn't there.
    assert_eq!(folder_ok(&text(&file), &own, &fixed), Err(NOT_THERE));
    assert_eq!(folder_ok(&text(&dir.join("missing")), &own, &fixed), Err(NOT_THERE));
    let _ = std::fs::remove_dir_all(&dir);
}

fn terminal_row(sealed: Option<&str>) -> Value {
    let mut tab = json!({ "profile": "PowerShell", "folder": "C:\\Projects" });
    if let Some(sealed) = sealed {
        tab["command_sealed"] = json!(sealed);
    }
    json!({
        "exe": r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\wt.exe",
        "aumid": "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
        "launch": true,
        "item": { "kind": "terminal", "tabs": [tab] },
    })
}

fn with_row(row: Value) -> Value {
    json!({ "version": 2, "desks": { "sig": { "name": "d", "workspaces": [{ "id": "w1", "name": "A", "windows": [row] }] } } })
}

#[test]
fn startup_commands_never_reach_the_shared_file_and_stay_local() {
    let whole = with_row(terminal_row(Some("c2VhbGVk")));
    let (shared, local) = split(&whole);
    assert!(!shared.to_string().contains("command_sealed") && !shared.to_string().contains("c2VhbGVk"));
    // The local copy is the complete one.
    let mine = whole.clone();
    // Another PC's file carries a command it sealed, and a clear one: neither survives.
    let mut theirs = with_row(terminal_row(Some("b3RoZXI=")));
    theirs["desks"]["sig"]["workspaces"][0]["windows"][0]["item"]["tabs"][0]["command"] = json!("calc");
    intake(None, &mut theirs);
    let joined = join(&theirs, &mine);
    let tab = &joined["desks"]["sig"]["workspaces"][0]["windows"][0]["item"]["tabs"][0];
    assert_eq!(tab["command_sealed"], "c2VhbGVk");
    assert!(tab.get("command").is_none());
    // A row whose app changed in the folder loses the command.
    let mut other_app = shared.clone();
    other_app["desks"]["sig"]["workspaces"][0]["windows"][0]["exe"] = json!(r"C:\Windows\notepad.exe");
    let joined = join(&other_app, &mine);
    assert!(!joined.to_string().contains("c2VhbGVk"));
    // A row that moved to another place in the list loses it too.
    let mut moved = shared.clone();
    let rows = moved["desks"]["sig"]["workspaces"][0]["windows"].as_array_mut().unwrap();
    rows.insert(0, json!({ "exe": r"C:\Windows\notepad.exe" }));
    assert!(!join(&moved, &mine).to_string().contains("c2VhbGVk"));
    let _ = local;
}

#[test]
fn open_with_changed_in_the_folder_is_dropped_with_launch_off() {
    let row = |open_with: &str| {
        let mut row = json!({ "exe": r"C:\Windows\notepad.exe", "launch": true });
        row["open_with"] = json!(open_with);
        with_row(row)
    };
    let base = row("C:\\notes\\a.txt");
    // Unchanged from what this PC has: it stays.
    let mut same = base.clone();
    intake(Some(&base), &mut same);
    assert_eq!(same["desks"]["sig"]["workspaces"][0]["windows"][0]["open_with"], "C:\\notes\\a.txt");
    // Changed on the other PC, or new there: dropped, and the row doesn't launch.
    for mut changed in [row("C:\\notes\\b.txt"), same.clone()] {
        let against = if changed == same { None } else { Some(&base) };
        intake(against, &mut changed);
        let got = &changed["desks"]["sig"]["workspaces"][0]["windows"][0];
        assert!(got.get("open_with").is_none(), "{got}");
        assert_eq!(got["launch"], false);
    }
}

#[test]
fn stamps_are_utc_and_sortable() {
    assert_eq!(stamp(0), "19700101-000000");
    assert_eq!(stamp(1_700_000_000), "20231114-221320");
    assert_eq!(stamp(1_709_164_800 + 3_661), "20240229-010101");
}

#[test]
fn a_local_load_never_keeps_a_stamp_and_an_import_never_a_folder() {
    let p = {
        let dir = std::env::temp_dir().join(format!("seam-test-{}-stamp", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    };
    std::fs::write(&p, br#"{"sync": {"pc": "X", "at": 1}, "config_folder": "D:\\Sync"}"#).unwrap();
    let loaded = Config::load_from(&p);
    assert!(loaded.sync.is_none() && loaded.config_folder.as_deref() == Some(r"D:\Sync"));
    let imported = Config::from_import(br#"{"sync": {"pc": "X", "at": 1}, "config_folder": "D:\\Sync"}"#).unwrap();
    assert!(imported.sync.is_none() && imported.config_folder.is_none());
    // A file that doesn't parse still points at the folder it was using.
    std::fs::write(&p, br#"{"enabled": "yes", "config_folder": "D:\\Sync"}"#).unwrap();
    assert_eq!(Config::load_from(&p).config_folder.as_deref(), Some(r"D:\Sync"));
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

#[test]
fn push_writes_only_when_the_file_is_what_this_pc_saw() {
    let dir = std::env::temp_dir().join(format!("seam-test-{}-push", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.json");
    folder::point_at(Some(dir.clone()), None);
    let mut config = Config { enabled: false, ..Config::default() };
    // No file there yet: this PC's shared part is written, with a stamp.
    assert_eq!(folder::push(&config, 100), Push::Quiet);
    let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(written["enabled"], false);
    assert_eq!(written["sync"]["at"], 100);
    assert!(written.get("hotkeys").is_none() && written.get("config_folder").is_none());
    assert!(!dir.join("config.json.tmp").exists());
    // Its own write is recognised, and an unchanged save writes nothing.
    assert!(folder::is_own_write(&std::fs::read(&file).unwrap()));
    std::fs::write(dir.join("marker"), b"").unwrap();
    let before = std::fs::read(&file).unwrap();
    assert_eq!(folder::push(&config, 200), Push::Quiet);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    // A change here is written.
    config.zones.enabled = !config.zones.enabled;
    assert_eq!(folder::push(&config, 300), Push::Quiet);
    assert_ne!(std::fs::read(&file).unwrap(), before);
    // Someone else changed the file meanwhile: nothing is written over it, the watcher is woken.
    let mut theirs: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    theirs["auto_apply"] = json!(true);
    std::fs::write(&file, theirs.to_string()).unwrap();
    config.enabled = true;
    let there = std::fs::read(&file).unwrap();
    assert_eq!(folder::push(&config, 400), Push::Wake);
    assert_eq!(std::fs::read(&file).unwrap(), there);
    // A file that doesn't parse is never written over.
    std::fs::write(&file, b"{\"enab").unwrap();
    assert_eq!(folder::push(&config, 500), Push::Wake);
    assert_eq!(std::fs::read(&file).unwrap(), b"{\"enab");
    // A newer Seam's file isn't taken or written over, and the state says why.
    std::fs::write(&file, json!({ "version": CURRENT_VERSION + 1 }).to_string()).unwrap();
    assert_eq!(folder::push(&config, 600), Push::Changed);
    assert_eq!(folder::info().problem, Some(folder::Problem::Newer));
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"version\":"));
    // A folder that is gone is a state, not a crash.
    folder::point_at(Some(dir.join("gone")), None);
    assert_eq!(folder::push(&config, 700), Push::Changed);
    assert_eq!(folder::state(), folder::FolderState::Unavailable);
    folder::point_at(None, None);
    let _ = std::fs::remove_dir_all(&dir);
}
