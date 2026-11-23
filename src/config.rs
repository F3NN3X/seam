//! Settings and saved layouts, stored as JSON in %APPDATA%\Seam\config.json.

use crate::geom::{EdgeMap, Side};
use crate::log;
use crate::palette;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod folder;
mod migrate;
mod store;
pub mod sync;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tests;

use migrate::keep_v1_copy;
pub(crate) use store::write_atomic;

const CURRENT_VERSION: u32 = 2;

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
    /// The screens touch on the desk but Windows has a gap between them, so the
    /// border holds while `b` stays beyond that side of `a`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub glued: bool,
    /// The screens touch in Windows but not on the desk, so the pointer stops
    /// at this border instead of crossing it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_on_desk: bool,
}

/// What a left-click on the tray icon does.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrayClick {
    #[default]
    Open,
    LineUp,
}

/// Global shortcuts, in "Ctrl+Shift+Alt+L" form. `None` means not set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Hotkeys {
    pub line_up: Option<String>,
    pub pause: Option<String>,
    pub next_zone: Option<String>,
    pub prev_zone: Option<String>,
    pub capture_workspace: Option<String>,
    pub move_left: Option<String>,
    pub move_right: Option<String>,
    pub move_up: Option<String>,
    pub move_down: Option<String>,
    pub focus_left: Option<String>,
    pub focus_right: Option<String>,
    pub focus_up: Option<String>,
    pub focus_down: Option<String>,
    pub swap_left: Option<String>,
    pub swap_right: Option<String>,
    pub swap_up: Option<String>,
    pub swap_down: Option<String>,
    pub pin_top: Option<String>,
    pub pin_sticky: Option<String>,
    pub release_all: Option<String>,
    /// "Snap back to this workspace": switch to the active workspace again.
    pub snap_back: Option<String>,
    /// "Save windows to the active workspace": capture into the workspace in use (§7).
    pub quick_save: Option<String>,
    /// "Grow window into its neighbour", within its group of zones.
    pub grow: Option<String>,
    pub shrink: Option<String>,
    /// "Give window two thirds of its group", or back as it was on a second press.
    pub priority: Option<String>,
    /// "Switch back to the previous workspace" on this desk.
    pub switch_back: Option<String>,
    /// "Close this workspace": list the windows of the workspace in use, then ask each to close.
    pub close_workspace: Option<String>,
    /// "Minimize this workspace": minimize the windows of the workspace in use.
    pub minimize_workspace: Option<String>,
    /// "Tile this screen's windows", on the screen under the pointer.
    pub tile_screen: Option<String>,
    /// "Quick layout": pick the layout of the screen under the pointer from the keyboard.
    pub quick_layout: Option<String>,
    /// "Focus on this window": minimize the others on its screen, and bring them back.
    pub focus_only: Option<String>,
    /// "Spotlight": dim everything but the window in use.
    pub spotlight: Option<String>,
    /// "Size menu": share or take out the focused snapped window.
    pub divider_menu: Option<String>,
    /// "Pause Seam": pause or resume everything Seam does, until it is pressed again or Seam restarts.
    pub pause_seam: Option<String>,
    /// "Fit group to screen": stretch the focused window's group to fill its screen.
    pub fit_group: Option<String>,
    /// "Next window in stack": the next window of the focused window's stack goes on top.
    pub stack_next: Option<String>,
    /// "Previous window in stack".
    pub stack_prev: Option<String>,
    /// "Float or tile this window": the focused window leaves smart tiling, or joins it again.
    pub float_tile: Option<String>,
    /// "Free all windows": smart tiling stops on the screen under the pointer, and its
    /// windows go back to where they were before.
    pub free_tiling: Option<String>,
    /// "Show overview": every window on every screen and virtual desktop, to search and go to.
    pub overview: Option<String>,
    /// "Rotate windows": every snapped window on the screen under the pointer moves to the
    /// next zone. The same key with Shift added goes the other way.
    pub rotate: Option<String>,
    /// "Reset sizes": the screen under the pointer goes back to the layout it came from,
    /// with that layout's proportions, or a tiled screen's splits even out.
    pub reset_sizes: Option<String>,
    /// "Add window to workspace 1" to "9": the focused window joins the desk's workspace
    /// with that number, in list order.
    pub add_to_1: Option<String>,
    pub add_to_2: Option<String>,
    pub add_to_3: Option<String>,
    pub add_to_4: Option<String>,
    pub add_to_5: Option<String>,
    pub add_to_6: Option<String>,
    pub add_to_7: Option<String>,
    pub add_to_8: Option<String>,
    pub add_to_9: Option<String>,
    /// "Switch to next workspace" and "Switch to previous workspace": the next or previous one
    /// in the desk's list, from the one in use. They wrap around.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_prev: Option<String>,
    /// "Move window to next workspace" and "Move window to previous workspace": the focused
    /// window's row goes from the workspace in use to the next or previous one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_to_next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_to_prev: Option<String>,
    /// "Scroll left" and "Scroll right": on a Scrolling screen, the column next to the
    /// focused one takes the focus, and the strip scrolls to show it.
    pub scroll_left: Option<String>,
    pub scroll_right: Option<String>,
    /// "Move column left" and "Move column right": the focused window's column swaps with its neighbour.
    pub column_left: Option<String>,
    pub column_right: Option<String>,
    /// "Column width": the focused window's column goes to the next of 1/3, 1/2, 2/3 and full.
    pub column_width: Option<String>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            line_up: Some("Ctrl+Shift+Alt+L".into()),
            pause: Some("Ctrl+Shift+Alt+P".into()),
            // New installs leave these unset (D107). The tray clears either one in a
            // saved file where it sits on a directional key.
            next_zone: None,
            prev_zone: None,
            capture_workspace: None,
            move_left: Some("Shift+Win+Alt+Left".into()),
            move_right: Some("Shift+Win+Alt+Right".into()),
            move_up: Some("Shift+Win+Alt+Up".into()),
            move_down: Some("Shift+Win+Alt+Down".into()),
            focus_left: Some("Ctrl+Shift+Alt+Left".into()),
            focus_right: Some("Ctrl+Shift+Alt+Right".into()),
            focus_up: Some("Ctrl+Shift+Alt+Up".into()),
            focus_down: Some("Ctrl+Shift+Alt+Down".into()),
            swap_left: None,
            swap_right: None,
            swap_up: None,
            swap_down: None,
            pin_top: None,
            pin_sticky: None,
            release_all: None,
            snap_back: None,
            quick_save: None,
            grow: None,
            shrink: None,
            priority: None,
            switch_back: None,
            close_workspace: None,
            minimize_workspace: None,
            tile_screen: None,
            quick_layout: None,
            focus_only: None,
            spotlight: None,
            divider_menu: None,
            pause_seam: None,
            fit_group: None,
            stack_next: None,
            stack_prev: None,
            float_tile: None,
            free_tiling: None,
            overview: None,
            rotate: None,
            reset_sizes: None,
            add_to_1: None,
            add_to_2: None,
            add_to_3: None,
            add_to_4: None,
            add_to_5: None,
            add_to_6: None,
            add_to_7: None,
            add_to_8: None,
            add_to_9: None,
            workspace_next: None,
            workspace_prev: None,
            add_to_next: None,
            add_to_prev: None,
            scroll_left: None,
            scroll_right: None,
            column_left: None,
            column_right: None,
            column_width: None,
        }
    }
}

impl Hotkeys {
    /// The directional shortcuts as (verb, direction, combo), in the order they register.
    pub fn directional(&self) -> [(&'static str, &'static str, Option<&String>); 12] {
        [
            ("move", "left", self.move_left.as_ref()),
            ("move", "right", self.move_right.as_ref()),
            ("move", "up", self.move_up.as_ref()),
            ("move", "down", self.move_down.as_ref()),
            ("focus", "left", self.focus_left.as_ref()),
            ("focus", "right", self.focus_right.as_ref()),
            ("focus", "up", self.focus_up.as_ref()),
            ("focus", "down", self.focus_down.as_ref()),
            ("swap", "left", self.swap_left.as_ref()),
            ("swap", "right", self.swap_right.as_ref()),
            ("swap", "up", self.swap_up.as_ref()),
            ("swap", "down", self.swap_down.as_ref()),
        ]
    }

    /// The "Add window to workspace" shortcuts, for workspaces 1 to 9, in the order they register.
    pub fn add_to_workspace(&self) -> [Option<&String>; 9] {
        [
            self.add_to_1.as_ref(),
            self.add_to_2.as_ref(),
            self.add_to_3.as_ref(),
            self.add_to_4.as_ref(),
            self.add_to_5.as_ref(),
            self.add_to_6.as_ref(),
            self.add_to_7.as_ref(),
            self.add_to_8.as_ref(),
            self.add_to_9.as_ref(),
        ]
    }

    /// Next or previous zone on the same key as a directional command would
    /// never fire. Clear whichever does, and say which.
    pub fn drop_zone_clashes(&mut self) -> Vec<&'static str> {
        let taken: Vec<_> =
            self.directional().iter().filter_map(|(_, _, c)| crate::hotkey::parse(c.as_ref()?)).collect();
        let mut dropped = Vec::new();
        for (action, combo) in [("next_zone", &mut self.next_zone), ("prev_zone", &mut self.prev_zone)] {
            if combo.as_deref().and_then(crate::hotkey::parse).is_some_and(|c| taken.contains(&c)) {
                *combo = None;
                dropped.push(action);
            }
        }
        dropped
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ZoneSettings {
    pub enabled: bool,
    /// "shift", "ctrl" or "alt".
    pub show_key: String,
    pub span_key: String,
    pub gap: u32,
    pub restore_size: bool,
    /// Experimental: after a screen change, put snapped windows back in their zones.
    pub refit: bool,
    /// Edit layouts on the screen itself. Off shows a live view there and edits in the panel.
    pub edit_on_screen: bool,
    /// Dragging the edge of a window in a zone moves the windows in the zones beside it.
    pub resize_neighbours: bool,
    /// Thin handles in the gap between snapped windows, to drag or right-click.
    pub divider_handles: bool,
    /// Dropping a window into a zone another window holds swaps them instead of overlapping.
    pub swap_on_drop: bool,
    /// Holding the pointer still during a window drag shows the zones, as the show key does.
    pub pause_to_snap: bool,
    /// A strip on the lit screen lists the keys a drag takes while zones show.
    pub drag_hints: bool,
    /// Dropping a window on another window's edge puts the two side by side.
    pub snap_beside: bool,
    /// "Put windows back when Seam quits": a quit the user asked for frees every tiled screen first.
    pub tiling_free_on_quit: bool,
    /// Exe file names, case-insensitive.
    pub exclude: Vec<String>,
}

impl Default for ZoneSettings {
    fn default() -> Self {
        ZoneSettings {
            enabled: true,
            show_key: "shift".into(),
            span_key: "ctrl".into(),
            gap: 8,
            restore_size: true,
            refit: false,
            edit_on_screen: true,
            resize_neighbours: true,
            divider_handles: true,
            swap_on_drop: true,
            pause_to_snap: false,
            drag_hints: false,
            snap_beside: true,
            tiling_free_on_quit: false,
            exclude: Vec::new(),
        }
    }
}

/// Pins, the focus border and other window managers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WindowSettings {
    /// Colour the focused window's border (D108).
    pub focus_border: bool,
    /// "Another window manager handles my windows" (D110).
    pub other_wm: bool,
    /// "none", a name from `palette::SWATCHES` or `#RRGGBB`. Anything else reads as
    /// the default (D118, D122).
    pub focus_color: String,
    pub pin_color: String,
    /// "Tell me when a pinned window closes" (D119).
    pub notify_pinned_close: bool,
    /// "Pinned windows stay when I switch workspaces": a switch leaves a window
    /// pinned on top alone, as it does a sticky one. Capture still saves it.
    pub pins_stay_on_switch: bool,
    /// "Move and resize from anywhere in a window".
    pub alt_drag: AltDrag,
    /// "Open the overview when the pointer rests in a corner".
    pub corner_overview: CornerOverview,
}

/// Hold a key and drag anywhere in a window to move it, or right-drag to resize it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AltDrag {
    pub on: bool,
    /// "alt", "win" or "ctrl_alt". Anything else reads as "alt".
    pub key: String,
}

/// Rest the pointer in an outer corner of the desk to open the overview.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CornerOverview {
    pub on: bool,
    /// "top_left", "top_right", "bottom_left" or "bottom_right". Anything else reads as "top_left".
    pub corner: String,
}

impl Default for WindowSettings {
    fn default() -> Self {
        WindowSettings {
            focus_border: false,
            other_wm: false,
            focus_color: palette::FOCUS_DEFAULT.into(),
            pin_color: palette::PIN_DEFAULT.into(),
            notify_pinned_close: false,
            pins_stay_on_switch: false,
            alt_drag: AltDrag { on: false, key: "alt".into() },
            corner_overview: CornerOverview { on: false, corner: "top_left".into() },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ApiSettings {
    /// "Let other apps control Seam".
    pub enabled: bool,
    /// "Let assistants use Seam (MCP)": `seam-mcp.exe` may use the API, within its own list of methods.
    pub mcp: bool,
    /// "Let assistants change workspaces". Only counts while `mcp` is on.
    pub assistants_edit: bool,
}

impl Default for ApiSettings {
    fn default() -> Self {
        ApiSettings { enabled: true, mcp: false, assistants_edit: false }
    }
}

impl ApiSettings {
    /// Assistants may change workspaces: the second switch, under the first.
    pub fn assistants_may_edit(&self) -> bool {
        self.mcp && self.assistants_edit
    }
}

/// How hard the pointer pushes past a screen's edge before it crosses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PointerSettings {
    /// "Edge resistance": pixels past the edge to any other screen, 0 for off.
    /// Up to `MAX_EDGE_RESISTANCE_PX`.
    pub edge_resistance_px: u32,
    /// "Glued edge push": pixels past a glued edge, 0 to cross at once. Up to
    /// `MAX_GLUE_PUSH_PX`.
    pub glue_push_px: u32,
}

pub const MAX_EDGE_RESISTANCE_PX: u32 = 100;
pub const MAX_GLUE_PUSH_PX: u32 = 200;

impl Default for PointerSettings {
    fn default() -> Self {
        PointerSettings { edge_resistance_px: 0, glue_push_px: 60 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub auto_install: bool,
    pub prerelease: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings { auto_check: true, auto_install: false, prerelease: false }
    }
}

/// The size a window gets when Seam moves it to another screen (§11.4).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowSize {
    /// "Let Windows scale it": Windows' own DPI scaling, and nothing more.
    #[default]
    Windows,
    /// "Keep its pixel size": the same width and height in pixels as before the move.
    Pixels,
    /// "Keep its real size": the same size on the desk, from each screen's EDID size.
    Real,
}

/// Who reopens the apps of a restored last session (D128).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreApps {
    /// Windows' Restart apps, or the apps themselves. Seam only arranges them.
    #[default]
    Windows,
    /// Seam starts the missing ones, and turns Windows' Restart apps off meanwhile.
    Seam,
}

/// What Update from screen does with a row whose app isn't open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosedApps {
    /// The row stays, and a switch starts the app.
    Keep,
    /// The row stays, with the app off.
    TurnOff,
    /// The row goes.
    Remove,
}

impl ClosedApps {
    /// The patch that sets the choice. It writes the older flag too, true only for
    /// Remove, so a Seam from before the choice does the closest thing.
    pub fn patch(self) -> serde_json::Value {
        serde_json::json!({
            "workspaces_update_closed": self,
            "workspaces_update_removes_closed": self == ClosedApps::Remove,
        })
    }
}

/// What Seam does when I sign in.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignInMode {
    #[default]
    Nothing,
    /// Offer the desk's last session (D120).
    LastSession,
    /// Open one of the desk's workspaces.
    Workspace,
}

/// "When I sign in", per desk.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SignIn {
    pub mode: SignInMode,
    /// The workspace to open, in `Workspace` mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// "Ask first": a notice to click, instead of switching straight away.
    pub ask: bool,
}

impl Default for SignIn {
    fn default() -> Self {
        SignIn { mode: SignInMode::Nothing, workspace: None, ask: true }
    }
}

/// One set of screens and everything that belongs to it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Desk {
    pub name: String,
    /// Unix seconds. 0 until the tray has seen a migrated desk connected.
    pub last_seen: u64,
    pub borders: Vec<SavedBorder>,
    /// Unix seconds a line-up was last accepted. None for a desk lined up by an
    /// older build, or not at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lined_up_at: Option<u64>,
    /// Monitor device path -> zone layout. Kept as raw JSON until zones exist.
    pub zones: BTreeMap<String, serde_json::Value>,
    /// Kept as raw JSON until workspaces exist.
    pub workspaces: Vec<serde_json::Value>,
    /// The hidden "Last session" workspace (D120), raw like the workspaces. It is
    /// kept apart from them, so no list, menu or shortcut shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_session: Option<serde_json::Value>,
    /// "When I sign in". Absent in a file from before it existed: `sign_in_or` reads
    /// that as the old `restore_session` switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<SignIn>,
    /// The id of the workspace last in use here, so a restart shows it as in use again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_workspace: Option<String>,
    /// The id of the workspace that was in use before the current one, for Switch back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_workspace: Option<String>,
    /// The id of the workspace taken as in use at start when none was remembered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    /// Monitor device path -> smart tiling on that screen. Left out while empty, and
    /// an older Seam drops it, which turns tiling off.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tiling: BTreeMap<String, TileScreen>,
    /// Monitor device path -> screen margins on that screen. Left out while empty, and an
    /// older Seam drops it, which gives the zones the whole work area again.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub margins: BTreeMap<String, crate::margins::Margins>,
    /// The screens of this desk as they were when it was last connected, so another desk
    /// can copy its workspaces by where each screen is. Left out while empty, and an older
    /// Seam drops it, which only means the desk isn't offered as a source until it's connected again.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<DeskScreen>,
    /// Monitor device path -> the workspace in use on that screen when it has its own, and
    /// what was in use there before (D782). Left out while empty, and an older Seam drops
    /// it, which gives every screen the desk-wide workspace again. Kept on this PC.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub screen_in_use: BTreeMap<String, ScreenInUse>,
}

/// One screen's workspaces in use (D782).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenInUse {
    /// The one-screen workspace in use on this screen. `None`: the desk-wide one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// What was in use on this screen before, of either kind, for Switch back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

/// One screen of a desk, as it was the last time the desk was connected.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeskScreen {
    /// Monitor device path.
    pub id: String,
    /// The Line Up number, from 1.
    pub number: u32,
    /// The name the screen reports.
    pub name: String,
    /// Where Windows has it, in physical pixels: x, y, width, height.
    pub rect: [i32; 4],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub primary: bool,
    /// Where it stands on the desk when the desk is lined up, in mm: x, y, width, height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mm: Option<[f32; 4]>,
}

/// Tiling off on every screen and virtual desktop of a desk, as quitting with Put windows back
/// leaves it, including those that were never tiled this session. A screen left with no
/// desktops and its default family is dropped.
pub fn clear_tiling(tiling: &mut BTreeMap<String, TileScreen>) {
    tiling.values_mut().for_each(|entry| entry.desktops.clear());
    tiling.retain(|_, entry| *entry != TileScreen::default());
}

/// Smart tiling on one screen of a desk: the family, and the virtual desktops it is
/// on for, by id. Off on every desktop by default.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "TileScreenFile", into = "TileScreenFile")]
pub struct TileScreen {
    pub family: crate::tiling::Family,
    /// Ids of the virtual desktops it tiles on. An empty id is a Windows that doesn't
    /// say which desktop a window is on.
    pub desktops: Vec<String>,
}

/// `TileScreen` as the file has it. Scrolling is saved as Smart with `scrolling` set, so
/// an older Seam, which knows no Scrolling, reads Smart rather than failing on the file.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct TileScreenFile {
    family: crate::tiling::Family,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    scrolling: bool,
    desktops: Vec<String>,
}

impl From<TileScreenFile> for TileScreen {
    fn from(f: TileScreenFile) -> TileScreen {
        let family = if f.scrolling { crate::tiling::Family::Scrolling } else { f.family };
        TileScreen { family, desktops: f.desktops }
    }
}

impl From<TileScreen> for TileScreenFile {
    fn from(t: TileScreen) -> TileScreenFile {
        let scrolling = t.family == crate::tiling::Family::Scrolling;
        let family = if scrolling { crate::tiling::Family::Smart } else { t.family };
        TileScreenFile { family, scrolling, desktops: t.desktops }
    }
}

impl TileScreen {
    pub fn on(&self, desktop: &str) -> bool {
        self.desktops.iter().any(|d| d == desktop)
    }
}

impl Desk {
    /// The name of a desk whose screens haven't told us theirs yet.
    pub fn placeholder(screens: usize) -> String {
        format!("Desk with {screens} screens")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Missing in v1 files, so it reads as 0 there rather than the current version.
    #[serde(default)]
    pub version: u32,
    /// First-run setup was finished or skipped. A new install starts false; a file
    /// from before the field reads true, so current users never see it.
    #[serde(default = "setup_done_in_old_files")]
    pub setup_done: bool,
    /// Remap the pointer at calibrated borders.
    pub enabled: bool,
    /// After lining up, also move monitors in the Windows arrangement so the
    /// pixel layout matches at the middle of each border.
    pub align_arrangement: bool,
    /// Line Up adds a field to type how far the screens are offset, in cm.
    pub lineup_measure: bool,
    /// Offer to line up a set of screens we haven't seen before.
    pub ask_new_desks: bool,
    /// "When a window moves to another screen": what size a key, API or workspace move
    /// to another screen gives it, outside zones (§11.4).
    pub window_size_on_move: WindowSize,
    /// "Also when I drag a window to another screen": the same after a free drag with
    /// the mouse that didn't snap. Does nothing while the size is Windows'.
    pub window_size_on_drag: bool,
    pub tray_click: TrayClick,
    /// "Step aside for games and full-screen apps": while one is in front, hotkeys, snapping
    /// and Spotlight wait until it is gone (D390).
    pub step_aside_fullscreen: bool,
    pub debug_log: bool,
    pub hotkeys: Hotkeys,
    pub zones: ZoneSettings,
    pub windows: WindowSettings,
    /// Some desk offers its last session at sign-in (D120). Written for older builds,
    /// and read only to migrate a file from before each desk's `sign_in`.
    pub restore_session: bool,
    /// Who reopens the last session's apps (D128).
    pub restore_apps: RestoreApps,
    /// "Put apps in place when they open": a window that opens and matches a rule of
    /// the active workspace goes to its place once (§11.1). Off by default.
    pub auto_apply: bool,
    /// "Remember folders and pages": a capture keeps the folder an Explorer window
    /// shows and the page a browser shows, and a switch that starts the app opens it
    /// there (D330). Off by default. Off, nothing is read and saved ones aren't used.
    pub workspaces_remember_places: bool,
    /// "Update from screen removes apps that aren't open": the rows a capture has no
    /// window for are removed from the workspace, with Undo. Off keeps them. Only
    /// Update from screen; a quick save never removes a row.
    pub workspaces_update_removes_closed: bool,
    /// What Update from screen does with an app that isn't open, to start each one at.
    /// `None` reads `workspaces_update_removes_closed`, so an older file behaves as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces_update_closed: Option<ClosedApps>,
    /// "Offer to restore after screens change": when the screens change or the PC wakes and
    /// the windows of the workspace in use moved, ask before putting them back. Off by default.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub workspaces_offer_restore: bool,
    /// "Show the workspace name when I switch": a short label names the workspace on each
    /// screen the switch changed. On by default, and left out of the file while on.
    #[serde(skip_serializing_if = "is_true")]
    pub workspaces_switch_label: bool,
    /// "Where apps open": an app's window goes to its place whenever it opens, with
    /// or without a workspace. Raw JSON like workspaces, so an entry this build
    /// can't read is kept as it is and ignored.
    pub app_rules: Vec<serde_json::Value>,
    /// The library of custom layouts, `{id, name, tree, screens}`, shared by every desk
    /// and screen. Raw JSON, so an entry this build can't read is kept and ignored.
    /// Left out of the file while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_layouts: Vec<serde_json::Value>,
    /// The panel asked once whether to turn on Windows' "Ease cursor movement between
    /// displays", or Seam turned it on. Either way it isn't asked again.
    pub cursor_easing_asked: bool,
    /// Windows' `RestartApps` as it was before Seam turned it off, `u32::MAX` if it
    /// wasn't there, or `None` while Seam leaves it alone. Only the tray writes it.
    pub restore_apps_saved: Option<u32>,
    pub updates: UpdateSettings,
    pub api: ApiSettings,
    pub pointer: PointerSettings,
    /// Layout signature (sorted device paths) -> desk.
    pub desks: BTreeMap<String, Desk>,
    /// Signatures we've already offered to line up, so we only ask once.
    pub seen: Vec<String>,
    /// v1 only. Read for migration, never written.
    #[serde(skip_serializing)]
    pub layouts: BTreeMap<String, Vec<SavedBorder>>,
    /// Folder that holds the shared part of the config (brief G). None: settings live on this PC only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_folder: Option<String>,
    /// Who last wrote the shared file, for the "Use the settings in this folder?" dialog. Only the
    /// shared file has it: a local file's is ignored and a loaded config never keeps one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncStamp>,
}

/// Who wrote the shared file, and when.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncStamp {
    /// COMPUTERNAME.
    pub pc: String,
    /// Unix seconds.
    pub at: u64,
}

/// For `skip_serializing_if` on a field that defaults to on.
fn is_true(b: &bool) -> bool {
    *b
}

fn setup_done_in_old_files() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CURRENT_VERSION,
            setup_done: false,
            enabled: true,
            align_arrangement: true,
            lineup_measure: false,
            ask_new_desks: true,
            window_size_on_move: WindowSize::Windows,
            window_size_on_drag: false,
            tray_click: TrayClick::Open,
            step_aside_fullscreen: true,
            debug_log: false,
            hotkeys: Hotkeys::default(),
            zones: ZoneSettings::default(),
            windows: WindowSettings::default(),
            restore_session: false,
            restore_apps: RestoreApps::Windows,
            auto_apply: false,
            workspaces_remember_places: false,
            workspaces_update_removes_closed: false,
            workspaces_update_closed: None,
            workspaces_offer_restore: false,
            workspaces_switch_label: true,
            app_rules: Vec::new(),
            custom_layouts: Vec::new(),
            cursor_easing_asked: false,
            restore_apps_saved: None,
            updates: UpdateSettings::default(),
            api: ApiSettings::default(),
            pointer: PointerSettings::default(),
            desks: BTreeMap::new(),
            seen: Vec::new(),
            layouts: BTreeMap::new(),
            config_folder: None,
            sync: None,
        }
    }
}

/// A set of screens is identified by its sorted device paths.
pub fn signature<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let mut v: Vec<&str> = ids.collect();
    v.sort_unstable();
    v.join("|")
}

/// Apply a JSON merge patch (RFC 7386): objects merge, null removes.
pub fn merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let Some(fields) = patch.as_object() else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = serde_json::json!({});
    }
    if let Some(object) = target.as_object_mut() {
        for (key, value) in fields {
            if value.is_null() {
                object.remove(key);
            } else {
                merge_patch(object.entry(key.clone()).or_insert(serde_json::Value::Null), value);
            }
        }
    }
}

/// The merge patch that turns `old` into `new`.
pub fn diff_patch(old: &serde_json::Value, new: &serde_json::Value) -> serde_json::Value {
    let (Some(before), Some(after)) = (old.as_object(), new.as_object()) else { return new.clone() };
    let mut patch = serde_json::Map::new();
    for key in before.keys().filter(|k| !after.contains_key(*k)) {
        patch.insert(key.clone(), serde_json::Value::Null);
    }
    for (key, value) in after {
        match before.get(key) {
            Some(same) if same == value => {}
            Some(previous) => {
                patch.insert(key.clone(), diff_patch(previous, value));
            }
            None => {
                patch.insert(key.clone(), value.clone());
            }
        }
    }
    serde_json::Value::Object(patch)
}

impl Desk {
    /// What happens at sign-in on this desk. A desk seen after the migration has no
    /// choice yet, which is Nothing.
    pub fn choice(&self) -> SignIn {
        self.sign_in.clone().unwrap_or_default()
    }
}

impl Config {
    /// What each app that isn't open starts at in Update from screen.
    pub fn closed_apps(&self) -> ClosedApps {
        self.workspaces_update_closed.unwrap_or(if self.workspaces_update_removes_closed {
            ClosedApps::Remove
        } else {
            ClosedApps::Keep
        })
    }

    /// Whether the desk keeps a last session.
    pub fn keeps_session(&self, sig: &str) -> bool {
        self.desks.get(sig).is_some_and(|d| d.choice().mode == SignInMode::LastSession)
    }

    /// Whether any desk keeps a last session.
    pub fn any_keeps_session(&self) -> bool {
        self.desks.values().any(|d| d.choice().mode == SignInMode::LastSession)
    }

    /// This config with a merge patch applied. A patch's null deletes a key,
    /// but a deleted shortcut would come back as its default, so "not set"
    /// stays an explicit null.
    pub fn patched(&self, patch: &serde_json::Value) -> serde_json::Result<Config> {
        let mut merged = serde_json::to_value(self)?;
        merge_patch(&mut merged, patch);
        let hotkeys = patch.get("hotkeys").and_then(|h| h.as_object()).into_iter().flatten();
        for (action, _) in hotkeys.filter(|(_, value)| value.is_null()) {
            merged["hotkeys"][action] = serde_json::Value::Null;
        }
        let mut config: Config = serde_json::from_value(merged)?;
        config.windows.focus_color = palette::normalise(&config.windows.focus_color, palette::FOCUS_DEFAULT);
        config.windows.pin_color = palette::normalise(&config.windows.pin_color, palette::PIN_DEFAULT);
        // An older client changes the old switch: that is every desk's choice, stated outright,
        // except a desk the same patch gives its own. An unchanged value says nothing.
        let switched =
            patch.get("restore_session").and_then(|v| v.as_bool()).filter(|on| *on != self.any_keeps_session());
        if let Some(on) = switched {
            let mode = if on { SignInMode::LastSession } else { SignInMode::Nothing };
            for (sig, desk) in config.desks.iter_mut() {
                if patch.pointer(&format!("/desks/{}/sign_in", sig.replace('~', "~0").replace('/', "~1"))).is_none() {
                    desk.sign_in = Some(SignIn { mode, ..desk.choice() });
                }
            }
        }
        // A desk that stops keeping its session by a change loses the saved one.
        for (sig, desk) in config.desks.iter_mut() {
            if self.keeps_session(sig) && desk.choice().mode != SignInMode::LastSession {
                desk.last_session = None;
            }
        }
        // Only older builds read it.
        config.restore_session = config.any_keeps_session();
        crate::library::follow(&self.custom_layouts, &mut config);
        Ok(config)
    }
}
