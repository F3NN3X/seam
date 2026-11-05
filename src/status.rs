//! Runtime state the tray publishes for the panel in %APPDATA%\Seam\status.json.
//! It's what is happening now, not settings, so it never goes in config.json.

use crate::config::{write_atomic, Config};
use crate::geom::Side;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Remap {
    On,
    Paused,
    #[default]
    OffNoBorders,
}

impl Remap {
    pub fn of(enabled: bool, lined_up: usize) -> Remap {
        match (lined_up, enabled) {
            (0, _) => Remap::OffNoBorders,
            (_, true) => Remap::On,
            (_, false) => Remap::Paused,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BorderState {
    LinedUp,
    NotLinedUp,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BorderStatus {
    pub a: String,
    pub b: String,
    pub side: Side,
    pub state: BorderState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HotkeyError {
    pub action: String,
    pub combo: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AlignResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateStatus {
    pub available: String,
    pub downloaded: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Status {
    pub pid: u32,
    pub version: String,
    pub written_at: u64,
    pub remap: Remap,
    /// Signature of the connected desk.
    pub desk: String,
    /// Device paths in Line Up number order.
    pub monitors: Vec<String>,
    pub borders: Vec<BorderStatus>,
    pub hotkey_errors: Vec<HotkeyError>,
    pub last_align: Option<AlignResult>,
    pub workspace: Option<String>,
    pub update: Option<UpdateStatus>,
}

impl Status {
    pub fn path() -> PathBuf {
        Config::path().with_file_name("status.json")
    }

    pub fn read() -> Option<Status> {
        serde_json::from_slice(&std::fs::read(Self::path()).ok()?).ok()
    }

    pub fn write(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(&path, &serde_json::to_string_pretty(self)?)
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
