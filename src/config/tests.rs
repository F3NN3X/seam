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
