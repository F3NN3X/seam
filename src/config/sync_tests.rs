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
        "workspaces": [{ "id": "w", "scope": "m" }], "last_session": { "id": "s" },
        "sign_in": { "mode": "nothing", "ask": true }, "active_workspace": "w", "previous_workspace": "w",
        "default_workspace": "w", "tiling": { "m": {} }, "margins": { "m": {} }, "screens": [{ "id": "m" }],
        "screen_in_use": { "m": { "workspace": "w", "previous": "w" } },
        "wallpaper": { "screens": { "m": { "path": "C:\\a.jpg" } } },
        "workspace_wallpapers": { "w": { "screens": { "m": { "path": "C:\\b.jpg", "fit": "tile" } } } },
    }))
    .unwrap();
    let config = Config {
        setup_done: true,
        workspaces_update_closed: Some(ClosedApps::Keep),
        workspaces_offer_restore: true,
        wallpapers_enabled: true,
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
    guard(&mut taken, &BTreeSet::new());
    assert!(taken.get("hotkeys").is_none() && taken["desks"]["sig"].get("last_seen").is_none());
}

#[test]
fn screen_in_use_is_local() {
    let (shared, local) = split(&every_field_set());
    // Which workspace each screen has in use stays on this PC; the scope goes with its workspace.
    assert!(shared["desks"]["sig"].get("screen_in_use").is_none());
    assert_eq!(local["desks"]["sig"]["screen_in_use"]["m"]["workspace"], "w");
    assert_eq!(shared["desks"]["sig"]["workspaces"][0]["scope"], "m");
    let mine = json!({ "desks": { "sig": { "screen_in_use": { "m": { "workspace": "mine" } } } } });
    let theirs = json!({ "desks": { "sig": { "screen_in_use": { "m": { "workspace": "evil" } } } } });
    assert_eq!(join(&theirs, &mine)["desks"]["sig"]["screen_in_use"]["m"]["workspace"], "mine");
}

#[test]
fn wallpaper_fields_stay_on_this_pc() {
    let (shared, local) = split(&every_field_set());
    // The pictures are files on this PC, and the switch says whether this PC changes them.
    assert!(shared.get("wallpapers_enabled").is_none());
    assert_eq!(local["wallpapers_enabled"], true);
    for key in ["wallpaper", "workspace_wallpapers"] {
        assert!(shared["desks"]["sig"].get(key).is_none(), "{key}");
    }
    assert_eq!(local["desks"]["sig"]["workspace_wallpapers"]["w"]["screens"]["m"]["fit"], "tile");
    // A file from the folder can't point this PC at a picture either.
    let theirs = json!({ "desks": { "sig": { "wallpaper": { "screens": { "m": { "path": "C:\\evil.png" } } } } } });
    let mine = json!({ "desks": { "sig": { "wallpaper": { "screens": { "m": { "path": "C:\\mine.png" } } } } } });
    assert_eq!(join(&theirs, &mine)["desks"]["sig"]["wallpaper"]["screens"]["m"]["path"], "C:\\mine.png");
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
    guard(&mut theirs, &BTreeSet::new());
    let joined = join(&theirs, &mine);
    let tab = &joined["desks"]["sig"]["workspaces"][0]["windows"][0]["item"]["tabs"][0];
    assert_eq!(tab["command_sealed"], "c2VhbGVk");
    assert!(tab.get("command").is_none());
    // A row whose app changed in the folder loses the command.
    let mut other_app = shared.clone();
    other_app["desks"]["sig"]["workspaces"][0]["windows"][0]["exe"] = json!(r"C:\Windows\notepad.exe");
    let joined = join(&other_app, &mine);
    assert!(!joined.to_string().contains("c2VhbGVk"));
    // A row that moved to another place in the list keeps it: the tab is the same.
    let mut moved = shared.clone();
    let rows = moved["desks"]["sig"]["workspaces"][0]["windows"].as_array_mut().unwrap();
    rows.insert(0, json!({ "exe": r"C:\Windows\notepad.exe" }));
    assert!(join(&moved, &mine).to_string().contains("c2VhbGVk"));
    // A tab whose folder or profile was changed in the folder gets no command.
    for (field, value) in [("folder", r"D:\OneDrive\Seam\evil"), ("profile", "Other")] {
        let mut changed = shared.clone();
        changed["desks"]["sig"]["workspaces"][0]["windows"][0]["item"]["tabs"][0][field] = json!(value);
        assert!(!join(&changed, &mine).to_string().contains("c2VhbGVk"), "{field}");
    }
    // A planted copy of the row with another folder doesn't take it, and the real row keeps it.
    let mut copied = shared.clone();
    let row = copied["desks"]["sig"]["workspaces"][0]["windows"][0].clone();
    let mut planted = row.clone();
    planted["item"]["tabs"][0]["folder"] = json!(r"D:\evil");
    copied["desks"]["sig"]["workspaces"][0]["windows"] = json!([planted, row]);
    let joined = join(&copied, &mine);
    let rows = &joined["desks"]["sig"]["workspaces"][0]["windows"];
    assert!(rows[0]["item"]["tabs"][0].get("command_sealed").is_none());
    assert_eq!(rows[1]["item"]["tabs"][0]["command_sealed"], "c2VhbGVk");
    let _ = local;
}

fn launches(file: &Value) -> bool {
    file["desks"]["sig"]["workspaces"][0]["windows"][0]["launch"] == true
}

#[test]
fn a_row_the_folder_brings_arrives_with_launch_off_until_this_pc_accepts_it() {
    let mut row = json!({ "exe": r"C:\Windows\notepad.exe", "launch": true, "open_with": "C:\\notes\\a.txt" });
    row["place"] = json!({ "folder": "C:\\x" });
    let file = with_row(row);
    // New here: no launch, and no "Open with" or folder or page.
    let mut got = file.clone();
    guard(&mut got, &BTreeSet::new());
    let row = &got["desks"]["sig"]["workspaces"][0]["windows"][0];
    assert!(!launches(&got) && row.get("open_with").is_none() && row.get("place").is_none());
    // A planted program is no different.
    let mut planted = with_row(json!({ "exe": r"D:\OneDrive\Seam\payload.exe", "launch": true }));
    guard(&mut planted, &BTreeSet::new());
    assert!(!launches(&planted));
    // The same row again in a later write, after this PC took the first in, still doesn't launch (the two-step bypass).
    let held = got.clone();
    let accepted = accepted_of(&held);
    let mut again = file.clone();
    again["desks"]["sig"]["name"] = json!("other change");
    guard(&mut again, &accepted);
    assert!(!launches(&again));
    // The user turns the row on here: that is accepted, for the row as it is here.
    let mut on = held.clone();
    on["desks"]["sig"]["workspaces"][0]["windows"][0]["launch"] = json!(true);
    let accepted = accepted_of(&on);
    let mut next = file.clone();
    guard(&mut next, &accepted);
    assert!(launches(&next));
    // A change to the row, by app, item or "Open with", is a new row again.
    let mut changed = file.clone();
    changed["desks"]["sig"]["workspaces"][0]["windows"][0]["exe"] = json!(r"C:\Windows\calc.exe");
    guard(&mut changed, &accepted);
    assert!(!launches(&changed));
}

#[test]
fn other_windows_close_arrives_as_leave_unless_accepted() {
    let file =
        json!({ "desks": { "sig": { "workspaces": [{ "id": "w1", "other_windows": "close", "windows": [] }] } } });
    let mut got = file.clone();
    guard(&mut got, &BTreeSet::new());
    assert_eq!(got["desks"]["sig"]["workspaces"][0]["other_windows"], "leave");
    // The same workspace set to Close on this PC stays so.
    let accepted = accepted_of(&file);
    let mut kept = file.clone();
    guard(&mut kept, &accepted);
    assert_eq!(kept["desks"]["sig"]["workspaces"][0]["other_windows"], "close");
    let mut minimize = json!({ "desks": { "sig": { "workspaces": [{ "id": "w1", "other_windows": "minimize" }] } } });
    guard(&mut minimize, &BTreeSet::new());
    assert_eq!(minimize["desks"]["sig"]["workspaces"][0]["other_windows"], "minimize");
}

#[test]
fn what_goes_back_to_the_folder_keeps_the_other_pcs_values() {
    let raw = json!({ "version": 2, "auto_apply": false, "desks": { "sig": { "name": "d", "workspaces": [
        { "id": "w1", "name": "A", "other_windows": "close", "windows": [
            { "exe": r"C:\Windows\notepad.exe", "launch": true, "open_with": "C:\\a.txt" },
            { "exe": r"C:\Windows\calc.exe", "launch": true } ] }] } } });
    let mut held = raw.clone();
    guard(&mut held, &BTreeSet::new());
    assert!(!launches(&held));
    // Nothing changed here: it goes back as the folder had it, guard or not.
    assert_eq!(outgoing(&held, &held, &raw), raw);
    // A setting changed elsewhere in the file: the other keys are the folder's.
    let mut ours = held.clone();
    ours["auto_apply"] = json!(true);
    let out = outgoing(&held, &ours, &raw);
    assert_eq!(out["auto_apply"], true);
    assert_eq!(out["desks"]["sig"]["workspaces"], raw["desks"]["sig"]["workspaces"]);
    // This PC edited the workspace list (a name, and the second row's app): what it left alone stays the other PC's.
    let mut edited = held.clone();
    edited["desks"]["sig"]["workspaces"][0]["name"] = json!("Renamed");
    edited["desks"]["sig"]["workspaces"][0]["windows"][1]["exe"] = json!(r"C:\Windows\mspaint.exe");
    let out = outgoing(&held, &edited, &raw);
    let ws = &out["desks"]["sig"]["workspaces"][0];
    assert_eq!(ws["name"], "Renamed");
    assert_eq!(ws["other_windows"], "close");
    assert_eq!(ws["windows"][0], raw["desks"]["sig"]["workspaces"][0]["windows"][0]);
    assert_eq!(ws["windows"][1]["exe"], r"C:\Windows\mspaint.exe");
}

#[test]
fn files_with_too_many_things_are_refused_quickly() {
    let workspaces: Vec<Value> = (0..300).map(|n| json!({ "id": format!("w{n}"), "windows": [] })).collect();
    let many = json!({ "version": 2, "desks": { "sig": { "workspaces": workspaces } } }).to_string();
    let started = std::time::Instant::now();
    assert!(matches!(attempt(many.as_bytes(), 0), Attempt::GiveUp));
    let rows: Vec<Value> = (0..101).map(|_| json!({ "exe": "a.exe" })).collect();
    let fat = json!({ "desks": { "sig": { "workspaces": [{ "id": "w", "windows": rows }] } } }).to_string();
    assert!(matches!(attempt(fat.as_bytes(), 0), Attempt::GiveUp));
    let desks: serde_json::Map<String, Value> = (0..65).map(|n| (format!("d{n}"), json!({}))).collect();
    assert!(matches!(attempt(json!({ "desks": desks }).to_string().as_bytes(), 0), Attempt::GiveUp));
    let tabs: Vec<Value> = (0..9).map(|_| json!({})).collect();
    let item = json!({ "kind": "terminal", "tabs": tabs });
    let tall = json!({ "desks": { "sig": { "workspaces": [{ "id": "w", "windows": [{ "item": item }] }] } } });
    assert!(matches!(attempt(tall.to_string().as_bytes(), 0), Attempt::GiveUp));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    // A file within the limits is taken.
    let fine = json!({ "version": 2, "desks": { "sig": { "workspaces": [{ "id": "w", "windows": [] }] } } });
    assert!(matches!(attempt(fine.to_string().as_bytes(), 0), Attempt::Take(_)));
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
    let _lock = folder::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("seam-test-{}-push", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.json");
    folder::point_at(Some(dir.clone()), None);
    let mut config = Config { enabled: false, ..Config::default() };
    // No file there yet: this PC's shared part is written, with a stamp.
    assert_eq!(folder::push(&config, 100, &|_| true), Push::Quiet);
    let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(written["enabled"], false);
    assert_eq!(written["sync"]["at"], 100);
    assert!(written.get("hotkeys").is_none() && written.get("config_folder").is_none());
    assert!(!dir.join("config.json.tmp").exists());
    // Its own write is recognised, and an unchanged save writes nothing.
    assert!(folder::is_own_write(&std::fs::read(&file).unwrap()));
    std::fs::write(dir.join("marker"), b"").unwrap();
    let before = std::fs::read(&file).unwrap();
    assert_eq!(folder::push(&config, 200, &|_| true), Push::Quiet);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    // A change here is written.
    config.zones.enabled = !config.zones.enabled;
    assert_eq!(folder::push(&config, 300, &|_| true), Push::Quiet);
    assert_ne!(std::fs::read(&file).unwrap(), before);
    // Someone else changed the file meanwhile: nothing is written over it, the watcher is woken.
    let mut theirs: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    theirs["auto_apply"] = json!(true);
    std::fs::write(&file, theirs.to_string()).unwrap();
    config.enabled = true;
    let there = std::fs::read(&file).unwrap();
    assert_eq!(folder::push(&config, 400, &|_| true), Push::Wake);
    assert_eq!(std::fs::read(&file).unwrap(), there);
    // A file that doesn't parse is never written over.
    std::fs::write(&file, b"{\"enab").unwrap();
    assert_eq!(folder::push(&config, 500, &|_| true), Push::Wake);
    assert_eq!(std::fs::read(&file).unwrap(), b"{\"enab");
    // A newer Seam's file isn't taken or written over, and the state says why.
    std::fs::write(&file, json!({ "version": CURRENT_VERSION + 1 }).to_string()).unwrap();
    assert_eq!(folder::push(&config, 600, &|_| true), Push::Changed);
    assert_eq!(folder::info().problem, Some(folder::Problem::Newer));
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"version\":"));
    // A folder that is gone is a state, not a crash.
    folder::point_at(Some(dir.join("gone")), None);
    assert_eq!(folder::push(&config, 700, &|_| true), Push::Changed);
    assert_eq!(folder::state(), folder::FolderState::Unavailable);
    folder::point_at(None, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changes_made_while_the_folder_was_away_are_written_when_it_is_back() {
    let _lock = folder::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("seam-test-{}-away", std::process::id()));
    let gone = dir.with_extension("away");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&gone);
    std::fs::create_dir_all(&dir).unwrap();
    folder::point_at(Some(dir.clone()), None);
    let mut config = Config::default();
    assert_eq!(folder::push(&config, 1, &|_| true), Push::Quiet);
    std::fs::rename(&dir, &gone).unwrap();
    config.enabled = false;
    assert_eq!(folder::push(&config, 2, &|_| true), Push::Changed);
    assert_eq!(folder::state(), folder::FolderState::Unavailable);
    std::fs::rename(&gone, &dir).unwrap();
    assert_eq!(folder::push(&config, 3, &|_| true), Push::Changed);
    let file: Value = serde_json::from_slice(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(file["enabled"], false);
    assert_eq!(folder::state(), folder::FolderState::Synced);
    folder::point_at(None, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn absorb_then_push_keeps_the_other_pcs_row() {
    let _lock = folder::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("seam-test-{}-absorb", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The other PC's file: a row with an "Open with", launching.
    let mut theirs: Config =
        serde_json::from_value(json!({ "version": 2, "desks": { "sig": { "name": "d", "workspaces": [
        { "id": "w1", "name": "A", "windows": [
            { "exe": r"C:\Windows\notepad.exe", "launch": true, "open_with": "C:\\notes\\a.txt" } ] }] } } }))
        .unwrap();
    theirs.migrate_sign_in();
    let raw = folder::shared_of(&theirs);
    let mut held = raw.clone();
    guard(&mut held, &BTreeSet::new());
    assert!(!launches(&held));
    // This PC took it in, and then changed another setting.
    let mut ours = serde_json::to_value(&theirs).unwrap();
    ours["desks"]["sig"]["workspaces"] = held["desks"]["sig"]["workspaces"].clone();
    let mut config: Config = serde_json::from_value(ours).unwrap();
    config.enabled = false;
    std::fs::write(dir.join("config.json"), serde_json::to_string(&theirs).unwrap()).unwrap();
    folder::point_at(Some(dir.clone()), Some(raw.clone()));
    folder::set_base(raw, held);
    assert_eq!(folder::push(&config, 5, &|_| true), Push::Quiet);
    let file: Value = serde_json::from_slice(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(file["enabled"], false);
    let row = &file["desks"]["sig"]["workspaces"][0]["windows"][0];
    assert_eq!(row["launch"], true);
    assert_eq!(row["open_with"], "C:\\notes\\a.txt");
    folder::point_at(None, None);
    let _ = std::fs::remove_dir_all(&dir);
}

fn workspace_with(rows: Value) -> Value {
    json!({ "version": 2, "desks": { "sig": { "name": "d", "workspaces": [{ "id": "w1", "name": "A", "windows": rows }] } } })
}

fn rows_of(file: &Value) -> Vec<Value> {
    file["desks"]["sig"]["workspaces"][0]["windows"].as_array().cloned().unwrap_or_default()
}

#[test]
fn acceptance_is_the_whole_identity_not_a_hash_of_it() {
    let file = workspace_with(json!([{ "exe": r"C:\Windows\notepad.exe", "launch": true }]));
    let set = accepted_of(&file);
    assert_eq!(set.len(), 1);
    let parts: Vec<String> = serde_json::from_str(set.iter().next().unwrap()).unwrap();
    assert_eq!(parts.len(), 6);
    assert_eq!(parts[0], "w1");
    assert!(parts[1].contains("notepad.exe"));
    // A workspace id that carries what looks like the next field doesn't make another row's identity.
    let sneaky = json!({ "desks": { "sig": { "workspaces": [{ "id": "w1\u{1}\"C:\\Windows\\notepad.exe\"", "windows": [{ "launch": true }] }] } } });
    assert!(accepted_of(&sneaky).is_disjoint(&set));
    // Only an identical row passes, whatever else a file says.
    let mut other = file.clone();
    other["desks"]["sig"]["workspaces"][0]["windows"][0]["exe"] = json!(r"C:\Windows\notepad.exe ");
    guard(&mut other, &set);
    assert!(!launches(&other));
}

/// The other PC's file: a plain row and one with an "Open with" and a place, both launching.
fn two_rows() -> (Value, Value) {
    let plain = json!({ "exe": r"C:\Windows\notepad.exe", "launch": true });
    let with = json!({ "exe": r"C:\Windows\calc.exe", "launch": true, "open_with": "C:\\a.txt", "place": { "folder": "C:\\p" } });
    (workspace_with(json!([plain, with])), workspace_with(json!([plain])))
}

#[test]
fn turning_a_held_row_on_changes_only_what_this_pc_changed() {
    let (raw, accepts) = two_rows();
    let mut held = raw.clone();
    guard(&mut held, &accepted_of(&accepts));
    assert_eq!(rows_of(&held)[1]["launch"], false);
    // Off in the file, on here: the folder's row has it on already, so make it off there first.
    let mut raw_off = raw.clone();
    raw_off["desks"]["sig"]["workspaces"][0]["windows"][1]["launch"] = json!(false);
    let mut held_off = raw_off.clone();
    guard(&mut held_off, &accepted_of(&accepts));
    let mut ours = held_off.clone();
    ours["desks"]["sig"]["workspaces"][0]["windows"][1]["launch"] = json!(true);
    let out = outgoing(&held_off, &ours, &raw_off);
    let row = &rows_of(&out)[1];
    assert_eq!(row["launch"], true);
    assert_eq!(row["open_with"], "C:\\a.txt");
    assert_eq!(row["place"]["folder"], "C:\\p");
    assert_eq!(rows_of(&out)[0], rows_of(&raw_off)[0]);
}

#[test]
fn deleting_or_moving_a_row_here_leaves_the_other_rows_as_the_folder_has_them() {
    let (raw, accepts) = two_rows();
    let mut held = raw.clone();
    guard(&mut held, &accepted_of(&accepts));
    let held_rows = rows_of(&held);
    let raw_rows = rows_of(&raw);
    // The row before the guarded one is deleted here.
    let deleted = workspace_with(json!([held_rows[1]]));
    assert_eq!(rows_of(&outgoing(&held, &deleted, &raw)), [raw_rows[1].clone()]);
    // The two are swapped.
    let swapped = workspace_with(json!([held_rows[1], held_rows[0]]));
    assert_eq!(rows_of(&outgoing(&held, &swapped, &raw)), [raw_rows[1].clone(), raw_rows[0].clone()]);
    // A new row goes in first.
    let new = json!({ "exe": r"C:\Windows\mspaint.exe", "launch": true });
    let added = workspace_with(json!([new, held_rows[0], held_rows[1]]));
    assert_eq!(rows_of(&outgoing(&held, &added, &raw)), [new, raw_rows[0].clone(), raw_rows[1].clone()]);
}

#[test]
fn a_config_over_the_limits_is_never_written_to_the_folder() {
    let _lock = folder::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("seam-test-{}-big", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    folder::point_at(Some(dir.clone()), None);
    let mut config = Config::default();
    let workspaces: Vec<Value> = (0..201).map(|n| json!({ "id": format!("w{n}"), "windows": [] })).collect();
    config.desks.insert("sig".into(), Desk { workspaces, ..Desk::default() });
    config.migrate_sign_in();
    // Over the limits: nothing is written, and the state says why. The file Seam would refuse to read isn't made.
    assert_eq!(folder::push(&config, 1, &|_| true), Push::Changed);
    assert!(!dir.join("config.json").exists());
    assert_eq!(folder::info().problem, Some(folder::Problem::TooBig));
    assert_eq!(folder::push(&config, 2, &|_| true), Push::Quiet);
    // A read of the same bytes by anyone, this PC included, is refused too.
    let shared = folder::shared_of(&config);
    assert!(matches!(attempt(shared.to_string().as_bytes(), 0), Attempt::GiveUp));
    // Back within them: it is written, and the problem goes.
    config.desks.get_mut("sig").unwrap().workspaces.truncate(5);
    assert_eq!(folder::push(&config, 3, &|_| true), Push::Changed);
    assert!(dir.join("config.json").exists());
    assert_eq!(folder::info().problem, None);
    // A folder the check no longer allows is not written to.
    config.enabled = false;
    assert_eq!(folder::push(&config, 4, &|_| false), Push::Changed);
    assert_eq!(folder::state(), folder::FolderState::Unavailable);
    folder::point_at(None, None);
    let _ = std::fs::remove_dir_all(&dir);
}

fn identical_rows(open_with: [&str; 3]) -> (Vec<Value>, Vec<Value>) {
    // Held rows are identical (the guard stripped the Open with); the folder's own rows differ in it.
    let held = vec![json!({ "exe": "a.exe", "launch": false }); 3];
    let raw = open_with.map(|w| json!({ "exe": "a.exe", "launch": true, "open_with": w })).to_vec();
    (held, raw)
}

#[test]
fn identical_rows_pair_in_order_and_an_edited_one_does_not_take_a_neighbours_values() {
    let (held, raw) = identical_rows(["one", "two", "three"]);
    let (h, o, r) = (workspace_with(json!(held)), workspace_with(json!(held)), workspace_with(json!(raw)));
    assert_eq!(rows_of(&outgoing(&h, &o, &r)), raw);
    // The first is edited: the others still get their own, not each other's.
    let mut edited = held.clone();
    edited[0]["launch"] = json!(true);
    let out = rows_of(&outgoing(&h, &workspace_with(json!(edited)), &r));
    assert_eq!(out[1], raw[1]);
    assert_eq!(out[2], raw[2]);
    // The edited one takes the held row still unpaired, and only what it changed goes on the folder's row.
    assert_eq!(out[0], json!({ "exe": "a.exe", "launch": true, "open_with": "one" }));
    // The second is deleted: the first and the last keep their own.
    let out = rows_of(&outgoing(&h, &workspace_with(json!([held[0], held[2]])), &r));
    assert_eq!(out[0], raw[0]);
}

#[test]
fn turning_on_one_of_two_same_app_rows_keeps_the_other_pcs_fields_on_both() {
    let held = json!([{ "exe": "wt.exe", "launch": false }, { "exe": "wt.exe", "launch": false }]);
    let raw = json!([
        { "exe": "wt.exe", "launch": false, "open_with": "a", "place": { "folder": "C:\\a" } },
        { "exe": "wt.exe", "launch": false, "open_with": "b", "place": { "folder": "C:\\b" } },
    ]);
    let (h, r) = (workspace_with(held.clone()), workspace_with(raw.clone()));
    // The second is turned on here.
    let mut ours = held.clone();
    ours[1]["launch"] = json!(true);
    let out = rows_of(&outgoing(&h, &workspace_with(ours), &r));
    assert_eq!(out[0], raw[0]);
    assert_eq!(out[1]["launch"], true);
    assert_eq!(out[1]["open_with"], "b");
    assert_eq!(out[1]["place"]["folder"], "C:\\b");
}

#[test]
fn a_great_many_identical_rows_are_paired_quickly() {
    let held = vec![json!({ "exe": "a.exe", "launch": false }); 50_000];
    let raw = vec![json!({ "exe": "a.exe", "launch": true }); 50_000];
    let mut rows = held.clone();
    rows[10]["launch"] = json!(true);
    rows.remove(20);
    let started = std::time::Instant::now();
    let out = outgoing(&workspace_with(json!(held)), &workspace_with(json!(rows)), &workspace_with(json!(raw)));
    assert_eq!(rows_of(&out).len(), 49_999);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

struct Mount(&'static [char], &'static [char]);

impl Drives for Mount {
    fn fixed(&self, letter: char) -> bool {
        self.0.contains(&letter)
    }

    fn mounted(&self, letter: char) -> bool {
        self.1.contains(&letter)
    }
}

#[test]
fn a_folder_on_a_drive_that_isnt_mounted_yet_is_waited_for() {
    let own = Path::new(r"C:\Users\x\AppData\Roaming\Seam");
    // A drive with no volume: the disk may come back. Refused for good once it is there and isn't fixed.
    assert!(may_appear(r"Z:\Sync\Seam", own, &Mount(&['C'], &['C'])));
    assert!(!may_appear(r"Z:\Sync\Seam", own, &Mount(&['C'], &['C', 'Z'])));
    assert!(!may_appear(r"\\server\share", own, &Mount(&['C'], &['C'])));
    assert!(!may_appear(r"C:\..\x", own, &Mount(&['C'], &['C'])));
    // A fixed drive's folder that isn't made yet.
    let missing = std::env::temp_dir().join(format!("seam-test-{}-nothere", std::process::id()));
    let letter = missing.to_string_lossy().chars().next().unwrap().to_ascii_uppercase();
    let (fixed, mounted): (&'static [char], &'static [char]) =
        (&['C', 'D', 'E', 'F', 'G', 'H'], &['C', 'D', 'E', 'F', 'G', 'H']);
    assert!(fixed.contains(&letter));
    assert!(may_appear(&missing.display().to_string(), own, &Mount(fixed, mounted)));
}
