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

/// A screen's place in Windows' arrangement, in pixels.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Screen {
    /// Line Up number, from 1.
    pub number: u32,
    pub id: String,
    pub name: String,
    pub primary: bool,
    #[serde(default)]
    pub rect: ScreenRect,
    /// The id of the layout the screen has on this desk, as `zones.layouts` names it:
    /// a library id, a template's name or "none". None for a custom tree not in the
    /// library. It is here so a change of layout is a change of the state.
    #[serde(default)]
    pub layout: Option<String>,
}

/// A screen that is connected but turned off in Windows. Seam can't use it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InactiveScreen {
    pub id: String,
    /// Empty when Windows has no name for it.
    pub name: String,
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
    /// The screens are glued across a gap in Windows' arrangement.
    #[serde(default)]
    pub glued: bool,
    /// The screens touch in Windows but not on the desk.
    #[serde(default)]
    pub not_on_desk: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRef {
    pub id: String,
    pub name: String,
    /// A window of it was moved or minimized since, so Snap back has work to do.
    #[serde(default)]
    pub drifted: bool,
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

/// Window managers Seam steps aside for (D110): the exe, the name State uses, and the name to show.
/// LeopardWM's daemon binary is `leopardwm` (crates/daemon in jcardama/LeopardWM).
pub const OTHER_WMS: [(&str, &str, &str); 3] = [
    ("komorebi.exe", "komorebi", "komorebi"),
    ("glazewm.exe", "glazewm", "GlazeWM"),
    ("leopardwm.exe", "leopardwm", "LeopardWM"),
];

/// The name to show for a window manager State names.
pub fn other_wm_title(name: &str) -> &str {
    OTHER_WMS.iter().find(|(_, id, _)| *id == name).map_or(name, |(_, _, title)| title)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WmState {
    /// The window manager found running, if any, by its name in `OTHER_WMS`.
    pub other_wm: Option<String>,
    /// "Another window manager handles my windows" is on.
    pub manual_other_wm: bool,
    pub focus_border: bool,
    /// The colour settings as stored: "none", a swatch name or `#RRGGBB`.
    pub focus_color: String,
    pub pin_color: String,
}

/// The last session saved for the desk that is connected (D120).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRef {
    /// Unix seconds.
    pub saved_at: u64,
    pub windows: usize,
}

/// Windows that share one zone, one of them on top.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StackRef {
    /// Line Up number of its screen, from 1.
    pub screen: u32,
    /// Its zone, from 1, and the last one when it spans several.
    pub zone: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_to: Option<u32>,
    /// Window handles, in the order Next steps through them.
    pub windows: Vec<isize>,
    /// The window on top.
    pub top: isize,
}

/// Smart tiling on one screen, for the virtual desktop in front.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TilingRef {
    /// Line Up number of its screen, from 1.
    pub screen: u32,
    pub on: bool,
    pub family: crate::tiling::Family,
    /// Windows it tiles now, stacked ones included.
    pub windows: usize,
}

/// Where the settings live: on this PC, in a synced folder, or in one that isn't there now.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigSync {
    pub state: crate::config::folder::FolderState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub seam_version: String,
    pub packaged: bool,
    pub remap: Remap,
    pub zones: Zones,
    /// Spotlight is dimming everything but the focused window.
    pub spotlight: bool,
    /// Seam is paused: runtime only, a restart resumes.
    pub paused: bool,
    /// A game or other full-screen app is in front, so Seam waits (not set while paused).
    pub stepped_aside: bool,
    pub wm: WmState,
    pub desk: DeskRef,
    pub screens: Vec<Screen>,
    /// Screens Windows has turned off, which the desk and its workspaces ignore.
    pub screens_inactive: Vec<InactiveScreen>,
    pub borders: Vec<Border>,
    pub workspace: Option<WorkspaceRef>,
    pub session: Option<SessionRef>,
    /// Windows' "Restart apps" sign-in option was on when Seam last looked (D128).
    pub restart_apps: bool,
    /// Windows' "Ease cursor movement between displays" as the tray last read it.
    /// `None` on a Windows without the setting.
    pub windows_cursor_easing: Option<bool>,
    pub hotkey_errors: Vec<HotkeyError>,
    pub last_align: Option<AlignResult>,
    pub update: Option<UpdateState>,
    pub clients: Vec<String>,
    /// Every stack of windows in zones on this desk.
    pub stacks: Vec<StackRef>,
    /// Smart tiling on each screen, for the virtual desktop in front.
    pub tiling: Vec<TilingRef>,
    /// Whether the settings are kept in a synced folder.
    pub config_sync: ConfigSync,
}

/// What API reads are served from. The tray replaces it whole, so a reader
/// never sees a half-applied change and never holds up the tray.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub state: State,
    /// `config.json` as JSON.
    pub config: serde_json::Value,
}
