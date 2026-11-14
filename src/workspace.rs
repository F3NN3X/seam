//! Workspaces: which apps go where on a desk. Pure logic, so the tray's capture
//! and switch and the panel's editor, export and import agree.
//!
//! Zone numbers here count from 1, as the panel shows them.

use crate::zones::{self, Rect, Template, ZoneLayout};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet, hash_map::RandomState};
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, Prefix};

mod capture;
mod export;
mod geometry;
mod matching;
#[cfg(test)]
mod tests;

pub use capture::{checked_update, merge_capture, moved_rows, recapture_row, stages, take_window_place, tame_delay};
pub use export::{
    Export, ScreenInfo, explorer_arg, has_places, import, page_of, safe_aumid, safe_exe, safe_folder, safe_url,
    without_places,
};
pub use geometry::{from_fractions, rect_for, target_of, to_fractions};
pub use matching::{Bound, Candidate, assign, assign_bound};

/// A window this close to a zone, in pixels, counts as being in it.
const ZONE_TOLERANCE: i32 = 8;
/// The longest delay before an app starts, and the step it moves in.
pub const MAX_DELAY_MS: u32 = 30_000;
pub const DELAY_STEP_MS: u32 = 500;

/// What a switch does with windows the workspace doesn't mention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtherWindows {
    #[default]
    Leave,
    Minimize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WinState {
    Max,
    Keep,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Zone {
        zone: u32,
        span_to: Option<u32>,
    },
    /// Fractions of the work area: x, y, width, height.
    Rect {
        rect: [f32; 4],
    },
    State {
        state: WinState,
    },
}

/// How a window is shown. A minimized one stays minimized when switched to,
/// and restores into its place later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shown {
    #[default]
    Normal,
    Minimized,
    Maximized,
    /// Borderless and covering the whole screen.
    Fullscreen,
}

/// How a window was pinned, for the last session (D120). No pin isn't stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pin {
    Top,
    Sticky,
    Both,
}

impl Pin {
    /// From `Seam_pin` bits: 1 is on top, 2 is sticky.
    pub fn of_bits(bits: usize) -> Option<Pin> {
        match bits & 3 {
            1 => Some(Pin::Top),
            2 => Some(Pin::Sticky),
            3 => Some(Pin::Both),
            _ => None,
        }
    }

    pub fn bits(self) -> usize {
        match self {
            Pin::Top => 1,
            Pin::Sticky => 2,
            Pin::Both => 3,
        }
    }
}

/// What a window showed at capture, so a switch that starts its app opens it there
/// again (D330): an Explorer window's folder, or a browser's page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Folder(String),
    Url(String),
}

/// The longest page address kept.
const MAX_URL: usize = 2048;

// Deserialize is written out below, so older files load into the new fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self", default)]
pub struct Rule {
    /// Full path, matched case-insensitively.
    pub exe: String,
    pub aumid: Option<String>,
    pub class: String,
    /// For display and tiebreaks only.
    pub title_hint: String,
    /// Device path of the screen.
    pub monitor: String,
    pub target: Target,
    /// `WINDOWPLACEMENT.rcNormalPosition` as fractions of the work area.
    pub normal_rect: Option<[f32; 4]>,
    pub launch: bool,
    /// The last session's pins, which a switch pins the window it matches again, and
    /// an app rule's "Keep on top".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<Pin>,
    pub shown: Shown,
    /// Stacking at capture, 0 is the topmost. `u32::MAX` when it isn't known.
    pub z: u32,
    /// The window that had the focus at capture. At most one per workspace.
    pub focused: bool,
    /// How long a switch waits before starting this app, within its stage.
    pub delay_ms: u32,
    /// The next stage starts only once this app's window is up, or took too long.
    pub wait_for_window: bool,
    /// The window's width and height in pixels at capture. A free place restored on
    /// another screen keeps it with "Keep its pixel size" (§11.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_px: Option<[i32; 2]>,
    /// The same in millimetres, when its screen reports a size, for "Keep its real size".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_mm: Option<[f32; 2]>,
    /// Device path of the screen the window was on at capture. `monitor` can be
    /// retargeted later, this stays. A restore on any other screen counts as another
    /// screen. Without it, the saved size decides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_on: Option<String>,
    /// The folder or page the window showed, with "Remember folders and pages" on.
    /// Used only when a switch starts the app, and never exported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            exe: String::new(),
            aumid: None,
            class: String::new(),
            title_hint: String::new(),
            monitor: String::new(),
            target: Target::State { state: WinState::Keep },
            normal_rect: None,
            launch: true,
            pin: None,
            shown: Shown::Normal,
            z: u32::MAX,
            focused: false,
            delay_ms: 0,
            wait_for_window: false,
            size_px: None,
            size_mm: None,
            captured_on: None,
            place: None,
        }
    }
}

impl Serialize for Rule {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Rule::serialize(self, s)
    }
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut rule = Rule::deserialize(d)?;
        // Before `shown`, a maximized window was only its target. Nothing is
        // written back until the next save.
        if rule.target == (Target::State { state: WinState::Max }) && rule.shown == Shown::Normal {
            rule.shown = Shown::Maximized;
        }
        Ok(rule)
    }
}

impl Rule {
    /// Whether the rule sends its window somewhere: it has a screen and a place.
    pub fn moves(&self) -> bool {
        !self.monitor.is_empty() && self.target != (Target::State { state: WinState::Keep })
    }

    /// Send the window somewhere else. Full is maximized, and another place
    /// isn't, but a minimized window stays minimized.
    pub fn set_target(&mut self, target: Target) {
        match target {
            Target::State { state: WinState::Max } => self.shown = Shown::Maximized,
            Target::State { state: WinState::Keep } => {}
            _ if matches!(self.shown, Shown::Maximized | Shown::Fullscreen) => self.shown = Shown::Normal,
            _ => {}
        }
        self.target = target;
    }

    /// Show the window another way. Maximized is the screen's full target, and
    /// leaving it puts the window back where it was when it wasn't maximized.
    pub fn set_shown(&mut self, shown: Shown) {
        let full = self.target == (Target::State { state: WinState::Max });
        match shown {
            Shown::Maximized => self.target = Target::State { state: WinState::Max },
            _ if full => {
                self.target = match self.normal_rect {
                    Some(rect) => Target::Rect { rect },
                    None => Target::State { state: WinState::Keep },
                }
            }
            _ => {}
        }
        self.shown = shown;
    }

    /// Where and how a capture found the window, taken into this row. The row
    /// keeps its launch details and its place in the start order.
    fn take_place(&mut self, from: &Rule) {
        self.title_hint = from.title_hint.clone();
        self.class = from.class.clone();
        self.monitor = from.monitor.clone();
        self.target = from.target.clone();
        self.normal_rect = from.normal_rect;
        self.shown = from.shown;
        self.z = from.z;
        self.focused = from.focused;
        self.size_px = from.size_px;
        self.size_mm = from.size_mm;
        self.captured_on = from.captured_on.clone();
        // A capture that read no folder or page keeps the one saved.
        if from.place.is_some() {
            self.place = from.place.clone();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub hotkey: Option<String>,
    pub launch_missing: bool,
    pub other_windows: OtherWindows,
    pub windows: Vec<Rule>,
    /// Screen device path -> the layout a switch gives that screen, as a desk saves
    /// one, or `no_zones()`. A screen with no entry is kept as it is. Raw, so an
    /// entry this build can't read is kept and keeps its screen as it is.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub zones: BTreeMap<String, Value>,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace {
            id: String::new(),
            name: String::new(),
            hotkey: None,
            launch_missing: true,
            other_windows: OtherWindows::Leave,
            windows: Vec::new(),
            zones: BTreeMap::new(),
        }
    }
}

/// A workspace's entry for a screen that has no zones. A desk leaves such a screen
/// out, but here no entry means "Keep as is".
pub fn no_zones() -> Value {
    json!({ "template": "none", "tree": "leaf" })
}

/// The layout a workspace's entry gives its screen: `Some(None)` for no zones, and
/// `None` for an entry this build can't read, or with a tree bigger than the editor
/// can make, which keeps the screen as it is.
pub fn screen_layout(saved: &Value) -> Option<Option<ZoneLayout>> {
    let layout = zones::layout_of(saved).filter(|l| zones::within_limits(&l.tree))?;
    Some((layout.template != Template::None).then_some(layout))
}

/// An entry from outside the tray, as it is stored: read and written again, so
/// fields this build doesn't know are gone. `None` for one `screen_layout` refuses.
fn clean_entry(saved: &Value) -> Option<Value> {
    Some(zones_entry(screen_layout(saved)?.as_ref()))
}

/// A screen's layout as a capture records it, with no layout recorded as no zones.
pub fn zones_entry(layout: Option<&ZoneLayout>) -> Value {
    layout.and_then(|l| serde_json::to_value(l).ok()).unwrap_or_else(no_zones)
}

/// Capture and Update from screen: every screen of the desk with its layout now, or
/// no zones. Entries for screens not in `now` stay.
pub fn capture_zones(old: &BTreeMap<String, Value>, now: &[(String, Option<ZoneLayout>)]) -> BTreeMap<String, Value> {
    let mut zones = old.clone();
    for (screen, layout) in now {
        zones.insert(screen.clone(), zones_entry(layout.as_ref()));
    }
    zones
}

/// The screen already has the layout the workspace's entry gives it. The trees are
/// compared, so a screen whose link to a library layout was dropped still matches.
/// An entry this build can't read keeps the screen as it is, so it always matches.
pub fn zones_match(saved: &Value, now: Option<&ZoneLayout>) -> bool {
    screen_layout(saved).is_none_or(|want| want.map(|l| l.tree) == now.map(|l| l.tree.clone()))
}

/// A saved workspace, or `None` for an entry that isn't one. The config keeps
/// them as raw JSON so one bad entry can't stop the file loading (D63).
pub fn parse(saved: &Value) -> Option<Workspace> {
    serde_json::from_value(saved.clone()).ok().filter(|w: &Workspace| !w.id.is_empty())
}

pub fn list(saved: &[Value]) -> Vec<Workspace> {
    saved.iter().filter_map(parse).collect()
}

/// The app rules this build can use: an app, a screen and a place to go to. Others
/// stay in the config and are ignored.
pub fn app_rules(saved: &[Value]) -> Vec<Rule> {
    // A rule needs a place to send the window to, or a pin to keep it with.
    let usable = |r: &Rule| (!r.exe.is_empty() || r.aumid.is_some()) && (r.moves() || r.pin.is_some());
    saved.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).filter(usable).collect()
}

pub fn to_value(workspace: &Workspace) -> Value {
    serde_json::to_value(workspace).unwrap_or_default()
}

/// The id the last session switches under. No saved workspace has it.
pub const SESSION_ID: &str = "last_session";

/// The hidden "Last session" workspace a desk keeps (D120): what was where, and
/// what was pinned, when Seam last looked.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// Unix seconds.
    pub saved_at: u64,
    pub windows: Vec<Rule>,
}

impl Session {
    /// A saved session with at least one window, or `None`.
    pub fn parse(saved: &Value) -> Option<Session> {
        serde_json::from_value(saved.clone()).ok().filter(|s: &Session| !s.windows.is_empty())
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// The workspace a restore switches to. Other windows are always left alone.
    pub fn workspace(self) -> Workspace {
        Workspace {
            id: SESSION_ID.into(),
            name: "Last session".into(),
            hotkey: None,
            launch_missing: true,
            other_windows: OtherWindows::Leave,
            windows: self.windows,
            // A restore never changes a screen's layout.
            zones: BTreeMap::new(),
        }
    }
}

/// What the `Seam_ws` stamp keeps of a workspace's id: the same number for the
/// same id in every run, so a stamp outlives a tray restart. FNV-1a, never 0.
pub fn stamp_key(id: &str) -> u32 {
    let hash = id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    hash.max(1)
}

/// How a workspace's app is doing right now, as the editor shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    InPlace,
    /// Running, but not where the workspace puts it.
    Moved,
    /// Minimized, and the workspace wants it shown.
    Minimized,
    NotRunning,
    /// Its program runs with no window, as an app closed to the tray does.
    Background,
    /// Runs as administrator, so Seam can't move it.
    CantMove,
}

/// One row of `workspace.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowStatus {
    pub rule_index: usize,
    pub state: RowState,
}

/// Something a switch would put back: Snap back has work to do.
pub fn drifted(rows: &[RowStatus]) -> bool {
    rows.iter().any(|r| matches!(r.state, RowState::Moved | RowState::Minimized))
}

/// The workspace looks like the one in use: it has an app with a window, and every
/// app with a window is where it puts it. One in the background counts as neither.
pub fn all_in_place(rows: &[RowStatus]) -> bool {
    let mut running =
        rows.iter().filter(|r| !matches!(r.state, RowState::NotRunning | RowState::Background)).peekable();
    running.peek().is_some() && running.all(|r| r.state == RowState::InPlace)
}

/// A random id that isn't in `taken`.
pub fn new_id(taken: &[String]) -> String {
    loop {
        let bits = RandomState::new().build_hasher().finish();
        let id = format!("ws_{:04x}", bits & 0xffff);
        if !taken.contains(&id) {
            return id;
        }
    }
}

/// "Workspace N", the first number not in use.
pub fn next_name(taken: &[String]) -> String {
    (taken.len() + 1..).map(|n| format!("Workspace {n}")).find(|name| !taken.contains(name)).unwrap_or_default()
}

/// `name`, with " 2" added until it is no longer in use.
pub fn unique_name(name: &str, taken: &[String]) -> String {
    let mut name = name.to_string();
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name.push_str(" 2");
    }
    name
}
