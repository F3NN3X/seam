//! Settings and saved layouts, stored as JSON in %APPDATA%\Seam\config.json.

use crate::geom::{EdgeMap, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            // Write-then-rename so a crash never leaves half a file.
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
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
}
