//! What the tray is doing right now, shared with API clients. It's runtime
//! state, not settings, and is never written to disk.

use crate::geom::Side;
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Zones {
    On,
    #[default]
    Off,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DeskRef {
    pub sig: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Screen {
    /// Line Up number, from 1.
    pub number: u32,
    pub id: String,
    pub name: String,
    pub primary: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BorderState {
    LinedUp,
    NotLinedUp,
}

/// `a` and `b` are Line Up numbers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Border {
    pub a: u32,
    pub b: u32,
    pub side: Side,
    pub state: BorderState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRef {
    pub id: String,
    pub name: String,
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
pub struct UpdateState {
    pub available: String,
    pub ready: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub seam_version: String,
    pub packaged: bool,
    pub remap: Remap,
    pub zones: Zones,
    pub desk: DeskRef,
    pub screens: Vec<Screen>,
    pub borders: Vec<Border>,
    pub workspace: Option<WorkspaceRef>,
    pub hotkey_errors: Vec<HotkeyError>,
    pub last_align: Option<AlignResult>,
    pub update: Option<UpdateState>,
    pub clients: Vec<String>,
}

/// What API reads are served from. The tray replaces it whole, so a reader
/// never sees a half-applied change and never holds up the tray.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub state: State,
    /// `config.json` as JSON.
    pub config: serde_json::Value,
}
