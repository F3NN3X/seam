//! What the updater found, kept in %LOCALAPPDATA%\Seam\updates\state.json. The
//! updater in `seam-settings.exe` writes it. The tray only reads it, so the tray
//! never does network I/O or parses versions.

use crate::config::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdateFile {
    /// Unix seconds.
    pub last_checked: u64,
    pub available: Option<String>,
    /// Markdown bullets from the release.
    pub notes: String,
    /// "2026-11-04".
    pub date: String,
    /// Bytes to download, both exes together.
    pub size: u64,
    /// Both exes are downloaded and checked.
    pub ready: bool,
    pub error: Option<String>,
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Seam").join("updates")
}

/// A version that is safe to use in a path: digits, letters, dots, hyphens and
/// plus, starting with a digit. The updater checks the real thing with `semver`.
pub fn is_plain_version(version: &str) -> bool {
    version.starts_with(|c: char| c.is_ascii_digit())
        && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        && !version.contains("..")
}

pub fn version_dir(version: &str) -> PathBuf {
    dir().join(version)
}

fn state_path() -> PathBuf {
    dir().join("state.json")
}

pub fn read() -> UpdateFile {
    std::fs::read(state_path()).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn write(file: &UpdateFile) -> std::io::Result<()> {
    std::fs::create_dir_all(dir())?;
    write_atomic(&state_path(), &serde_json::to_string_pretty(file).map_err(std::io::Error::other)?)
}

/// The version the user was last told about, so each one is announced once.
pub fn notified() -> Option<String> {
    std::fs::read_to_string(dir().join("notified.txt")).ok().map(|v| v.trim().to_string())
}

pub fn set_notified(version: &str) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(dir().join("notified.txt"), version);
}

/// After an update: forget what was staged. Best effort.
pub fn clear() {
    let _ = std::fs::remove_dir_all(dir());
}
