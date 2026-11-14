//! The local API: JSON-RPC over JSON Lines on a per-user named pipe. This is
//! the protocol and the blocking client; the server is in the tray.

use crate::direction;
use crate::groups::Side;
use crate::library;
use crate::process;
use crate::state::Snapshot;
use crate::zones;
use serde::Serialize;
use serde_json::{Value, json};

pub const API_VERSION: u32 = 1;
/// Longest accepted line, in bytes.
pub const MAX_LINE: usize = 1 << 20;
/// Largest `step` `window.resize` takes, in pixels.
pub const MAX_STEP_PX: i32 = 10_000;
/// The share `window.share` takes, as in the layout's ratio clamp.
pub const MIN_SHARE: f64 = 0.1;
pub const MAX_SHARE: f64 = 0.9;
/// Events a client may fall behind by before it's dropped.
pub const MAX_QUEUED_EVENTS: usize = 256;

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const NOT_FOUND: i32 = 1;
pub const BUSY: i32 = 2;
pub const REFUSED: i32 = 3;
pub const FORBIDDEN: i32 = 4;
pub const DISABLED: i32 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> RpcError {
        RpcError { code, message: message.into() }
    }
}

pub type Reply = Result<Value, RpcError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Next,
    Prev,
}

/// What `window.*` does to the focused window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowVerb {
    Focus,
    Move,
    Swap,
}

impl WindowVerb {
    pub fn parse(text: &str) -> Option<WindowVerb> {
        match text {
            "focus" => Some(WindowVerb::Focus),
            "move" => Some(WindowVerb::Move),
            "swap" => Some(WindowVerb::Swap),
            _ => None,
        }
    }
}

/// What `window.resize` and `window.priority` do to the focused window's group of zones.
/// Grow and shrink carry a step in pixels, or None for the default. Share gives the
/// window `fraction` of the span of itself and the zones beside it along `axis`,
/// across the divider on `side`, or the one towards the bigger neighbour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupVerb {
    Grow(Option<i32>),
    Shrink(Option<i32>),
    Priority,
    Share { axis: Axis, fraction: f32, side: Option<Side> },
}

/// Which way `window.share` divides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Width,
    Height,
}

/// Which pin `window.pin` sets (D106).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    Top,
    Sticky,
}

/// What `window.focus_only` minimizes: the others on the focused window's screen,
/// or everything on the desk outside the workspace in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusScope {
    Screen,
    Workspace,
}

/// Which layout `layout.show` draws on the screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutView {
    Seam,
    Windows,
}

/// The window "Add an app" picked from the windows open now, and the row it added
/// for it, as the panel saw that row.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub hwnd: isize,
    pub exe: String,
    pub class: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Hello {
        client: String,
        version: String,
        token: Option<String>,
    },
    AppInfo,
    StateGet,
    ConfigGet,
    ConfigPatch(Value),
    DesksList,
    WorkspacesList {
        desk: Option<String>,
    },
    WorkspaceSwitch {
        id: Option<String>,
        name: Option<String>,
        /// Leave the other windows alone for this one switch.
        alongside: bool,
    },
    /// `row` with `into` takes only that row's window ("Where it is now"). With `pick`
    /// too, the row only takes the folder or page of that one window ("Add an app").
    WorkspaceCapture {
        name: Option<String>,
        into: Option<String>,
        row: Option<usize>,
        pick: Option<Pick>,
    },
    /// The editor saving one workspace of the current desk.
    WorkspaceUpdate {
        id: String,
        workspace: Value,
    },
    /// How each app of a workspace on this desk is doing right now.
    WorkspaceStatus {
        id: String,
    },
    /// Switch to the active workspace again, so drifted windows go back.
    WorkspaceSnapBack,
    WorkspaceSwitchBack,
    /// Save the windows on screen into the workspace in use, or a new one (§7).
    WorkspaceQuickSave,
    /// The workspace taken as in use at start when none is remembered, or none.
    WorkspaceSetDefault(Option<String>),
    /// "Put apps in place when they open".
    AutoApplySet(bool),
    /// Windows' "Ease cursor movement between displays". Only on is carried out.
    CursorEasingSet(bool),
    RemapSet(bool),
    ZonesSet(bool),
    ZoneMove(Dir),
    Window(WindowVerb, direction::Dir),
    WindowGroup(GroupVerb),
    /// `window.share`: a window, or the focused one, gets a share of it and its neighbour.
    WindowShare {
        hwnd: Option<isize>,
        axis: Axis,
        fraction: f32,
    },
    /// `window.ungroup`: a window, or the focused one, leaves its zones.
    WindowUngroup {
        hwnd: Option<isize>,
    },
    /// `window.fit`: a window's group, or the focused one's, fills its screen.
    WindowFit {
        hwnd: Option<isize>,
    },
    /// `on` is None to toggle.
    WindowPin {
        kind: PinKind,
        on: Option<bool>,
    },
    WindowFocusOnly(FocusScope),
    /// `on` is None to toggle.
    WindowSpotlight(Option<bool>),
    /// `seam.pause`: pause or resume everything Seam does. `on` is None to toggle.
    SeamPause(Option<bool>),
    WmRelease,
    SessionRestore,
    LineupStart,
    ArrangementAlign,
    ScreenIdentify(u32),
    /// Tile the loose windows of a screen: its Line Up number, or the one under the pointer.
    ScreenTile(Option<u32>),
    /// Save the windows on a screen as a new custom layout: its Line Up number, or the one under the pointer.
    ZonesFromWindows(Option<u32>),
    /// Open the quick layout overlay on a screen: its Line Up number, or the one under the pointer.
    ZonesQuick(Option<u32>),
    /// The panel picking a screen's layout (None for no zones): the screen's snapped
    /// windows follow it, with Undo.
    ZonesPick {
        number: u32,
        layout: Option<Value>,
    },
    /// `zones.apply`: give a screen a layout from the library or a template, by id or
    /// name (None for no zones), as a pick does. The screen is a Line Up number, or the
    /// one under the pointer.
    ZonesApply {
        number: Option<u32>,
        layout: Option<String>,
    },
    /// `zones.layouts`: what a screen can be given, and what it has. Read from the snapshot.
    ZonesLayouts(Option<u32>),
    LayoutShow(LayoutView),
    PanelOpen(Option<String>),
    EventsSubscribe(Vec<String>),
    UpdateReady,
    AppQuit,
}

/// Who is asking, as the server knows them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    /// Sent the session's panel token in `hello`.
    pub panel: bool,
    /// The client process is `seam.exe` or `seam-settings.exe`.
    pub own_image: bool,
    /// "Let other apps control Seam".
    pub api_enabled: bool,
}

/// A request that can't be answered, with the id to answer it under.
pub type Rejected = (Value, RpcError);

mod client;
mod parse;

pub use client::{CallError, Client, Event, pipe_name};
pub use parse::parse_request;

pub fn result_line(id: &Value, result: &Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub fn error_line(id: &Value, error: &RpcError) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string()
}

pub fn event_line(kind: &str, data: &Value) -> String {
    json!({ "jsonrpc": "2.0", "method": "event", "params": { "type": kind, "data": data } }).to_string()
}

/// The `events.subscribe` topic an event belongs to.
pub fn topic_of(kind: &str) -> &str {
    kind.split('.').next().unwrap_or(kind)
}

pub fn desks_list(snapshot: &Snapshot) -> Value {
    let active = &snapshot.state.desk.sig;
    let desks = snapshot.config["desks"].as_object().into_iter().flatten().map(|(sig, desk)| {
        json!({
            "sig": sig,
            "name": desk["name"],
            "screens": sig.split('|').count(),
            "active": sig == active,
        })
    });
    Value::Array(desks.collect())
}

pub fn workspaces_list(snapshot: &Snapshot, desk: Option<&str>) -> Reply {
    let sig = desk.unwrap_or(&snapshot.state.desk.sig);
    let Some(desk) = snapshot.config["desks"].get(sig) else {
        return Err(RpcError::new(NOT_FOUND, "no such desk"));
    };
    let active = snapshot.state.workspace.as_ref().map(|w| w.id.as_str());
    let default = desk["default_workspace"].as_str();
    let workspaces = desk["workspaces"].as_array().into_iter().flatten().map(|w| {
        json!({
            "id": w["id"],
            "name": w["name"],
            "hotkey": w["hotkey"],
            "windows": w["windows"].as_array().map_or(0, Vec::len),
            "zones": w.get("zones").cloned().unwrap_or_else(|| json!({})),
            "active": w["id"].as_str() == active,
            "default": default.is_some() && w["id"].as_str() == default,
        })
    });
    Ok(Value::Array(workspaces.collect()))
}

/// `zones.layouts`: the layouts a screen can be given, with the id of the one it has.
/// Without a `number` it's the screen `pointer` is on, or the first.
pub fn zones_layouts(snapshot: &Snapshot, number: Option<u32>, pointer: (i32, i32)) -> Reply {
    let screens = &snapshot.state.screens;
    let screen = match number {
        Some(n) => n.checked_sub(1).and_then(|i| screens.get(i as usize)),
        None => screens
            .iter()
            .find(|s| {
                let (x, y) = pointer;
                x >= s.rect.x && x < s.rect.x + s.rect.w && y >= s.rect.y && y < s.rect.y + s.rect.h
            })
            .or(screens.first()),
    };
    let Some(screen) = screen else { return Err(RpcError::new(NOT_FOUND, "no such screen")) };
    let Some(desk) = snapshot.config["desks"].get(&snapshot.state.desk.sig) else {
        return Err(RpcError::new(REFUSED, "This desk isn't saved yet"));
    };
    let now = desk["zones"].get(&screen.id).and_then(zones::layout_of);
    let saved = snapshot.config["custom_layouts"].as_array().map_or(&[][..], Vec::as_slice);
    let layouts: Vec<Value> = library::offers(saved, &screen.id, now.as_ref())
        .into_iter()
        .map(|o| json!({ "id": o.id, "name": o.name, "kind": o.kind }))
        .collect();
    Ok(json!({ "number": screen.number, "current": library::layout_id(now.as_ref()), "layouts": layouts }))
}
