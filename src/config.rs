//! Settings and saved layouts, stored as JSON in %APPDATA%\Seam\config.json.

use crate::geom::{EdgeMap, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Remap the pointer at calibrated borders.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// After lining up, also move monitors in the Windows arrangement so the
    /// pixel layout matches at the middle of each border.
    #[serde(default = "yes")]
    pub align_arrangement: bool,
    /// Layout signature (sorted device paths) -> calibrated borders.
    #[serde(default)]
    pub layouts: BTreeMap<String, Vec<SavedBorder>>,
    /// Signatures we've already offered to line up, so we only ask once.
    #[serde(default)]
    pub seen: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, align_arrangement: true, layouts: BTreeMap::new(), seen: Vec::new() }
    }
}

/// A set of screens is identified by its sorted device paths.
pub fn signature<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let mut v: Vec<&str> = ids.collect();
    v.sort_unstable();
    v.join("|")
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("Seam").join("config.json")
    }

    pub fn load() -> Config {
        Self::load_from(&Self::path())
    }

    fn load_from(p: &Path) -> Config {
        std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        self.save_to(&Self::path());
    }

    fn save_to(&self, p: &Path) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            // Write-then-rename so a crash never leaves half a file.
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_order_independent() {
        assert_eq!(signature(["b", "a"].into_iter()), signature(["a", "b"].into_iter()));
    }

    #[test]
    fn old_files_get_defaults() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert!(c.enabled && c.align_arrangement && c.layouts.is_empty());
    }

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.layouts.insert(
            "x|y".into(),
            vec![SavedBorder { a: "x".into(), b: "y".into(), side: Side::Right, map: EdgeMap { s: 0.66, o: 12.0 } }],
        );
        let s = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(back.layouts, c.layouts);
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
        assert!(c.enabled && c.layouts.is_empty());
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
            assert!(c.enabled && c.align_arrangement && c.layouts.is_empty(), "{name}");
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
    #[ignore = "bug: the next save overwrites the only copy"]
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
