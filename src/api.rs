//! The local API: JSON-RPC over JSON Lines on a per-user named pipe. This is
//! the protocol and the blocking client; the server is in the tray.

use crate::config::ClosedApps;
use crate::direction;
use crate::groups::Side;
use crate::library;
use crate::process;
use crate::state::Snapshot;
use crate::workspace;
use crate::zones;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
/// What `workspace.capture` says when its `token` is no longer good.
pub const PREVIEW_EXPIRED: &str = "preview expired";

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

/// What the capture review sends back to save a preview: the windows ticked, by their
/// `i`, and what Update from screen does with the rows of apps that aren't open.
#[derive(Clone, Debug, PartialEq)]
pub struct Save {
    pub token: String,
    pub take: Vec<usize>,
    pub closed: BTreeMap<usize, ClosedApps>,
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
    /// With `save`, the preview it names is saved as ticked instead of a new capture.
    WorkspaceCapture {
        name: Option<String>,
        into: Option<String>,
        row: Option<usize>,
        pick: Option<Pick>,
        save: Option<Save>,
    },
    /// `workspace.capture_preview`: capture as `workspace.capture` would, and keep the
    /// result under a token for the review instead of saving it.
    WorkspaceCapturePreview {
        into: Option<String>,
    },
    /// `workspace.add_window`: the window in front, or `hwnd`, joins the workspace with this
    /// id, or the one with this number in the desk's list. Panel or any client, never an assistant.
    WorkspaceAddWindow {
        id: Option<String>,
        number: Option<u32>,
        hwnd: Option<isize>,
    },
    /// The editor saving one workspace of the current desk.
    WorkspaceUpdate {
        id: String,
        workspace: Value,
    },
    /// `workspace.create`: an empty workspace on this desk. Panel, or an assistant allowed to edit.
    WorkspaceCreate {
        name: String,
    },
    /// `workspace.rename`, `edit_row`, `remove_row` and `set_show_all`, which an
    /// assistant allowed to edit may send. The tray saves them as the editor would.
    WorkspaceEdit {
        id: String,
        edit: workspace::Edit,
    },
    /// `workspace.move`: the workspace to number `to` of this desk's list, from 1.
    WorkspaceMove {
        id: String,
        to: usize,
    },
    /// `workspace.get`: the rows of one workspace on this desk, read from the snapshot.
    WorkspaceGet {
        id: String,
    },
    /// How each app of a workspace on this desk is doing right now.
    WorkspaceStatus {
        id: String,
    },
    /// Stop the switch that is starting apps. Assistants may: it only stops things.
    WorkspaceCancel,
    /// What the workspace's last switch did, from this run of Seam.
    WorkspaceLastSwitch {
        id: String,
    },
    /// Switch to the active workspace again, so drifted windows go back.
    WorkspaceSnapBack {
        /// Leave the other windows alone, which an assistant always does.
        alongside: bool,
    },
    WorkspaceSwitchBack {
        alongside: bool,
    },
    /// Save the windows on screen into the workspace in use, or a new one (§7).
    WorkspaceQuickSave,
    /// `workspace.windows`: the windows a close or minimize would take, and the ones it
    /// would keep. Never for an assistant.
    WorkspaceWindows {
        id: String,
    },
    /// `workspace.close`: ask these windows to close, the ones a fresh look still matches
    /// to the workspace (D704). Never for an assistant.
    WorkspaceClose {
        id: String,
        hwnds: Vec<isize>,
    },
    /// `workspace.minimize`: the workspace in use when `id` is left out. Never for an assistant.
    WorkspaceMinimize {
        id: Option<String>,
    },
    /// Put back the windows the last minimize took (D705).
    WorkspaceMinimizeUndo,
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
    /// `window.stack`: on stacks a window, or the focused one, with the others in its
    /// zones; off takes it out of its stack.
    WindowStack {
        hwnd: Option<isize>,
        on: bool,
    },
    /// `window.stack_cycle`: the next window of the focused window's stack goes on top,
    /// or the previous one with `false`.
    WindowStackCycle(bool),
    /// `tiling.set`: smart tiling on or off on a screen for the virtual desktop in front,
    /// and its family. The screen's Line Up number, or the one under the pointer.
    TilingSet {
        number: Option<u32>,
        on: Option<bool>,
        family: Option<crate::tiling::Family>,
    },
    /// `tiling.free`: Free all windows on a screen, by Line Up number or under the pointer.
    TilingFree(Option<u32>),
    /// `tiling.scroll`: on a Scrolling screen, the column left or right of the focused one
    /// takes the focus and comes into view. The screen's Line Up number, or else the
    /// focused window's strip, or the screen under the pointer.
    TilingScroll {
        number: Option<u32>,
        left: bool,
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
    /// `screen.rotate`: every window snapped on a screen moves to the next zone, or with
    /// `back` the previous one. Its Line Up number, or the one under the pointer.
    ScreenRotate {
        number: Option<u32>,
        back: bool,
    },
    /// `screen.reset_sizes`: a screen whose layout was changed goes back to the layout it
    /// came from, or a tiled one evens out its splits. Its Line Up number, or the one under the pointer.
    ScreenResetSizes(Option<u32>),
    /// Save the windows on a screen as a new custom layout: its Line Up number, or the one under the pointer.
    ZonesFromWindows(Option<u32>),
    /// Open the quick layout overlay on a screen: its Line Up number, or the one under the pointer.
    ZonesQuick(Option<u32>),
    /// `overview.show`: open the overview of every window on every screen.
    OverviewShow,
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
    /// The client process is a `seam-mcp.exe`, whatever folder it sits in.
    pub mcp: bool,
    /// "Let assistants use Seam (MCP)".
    pub mcp_on: bool,
    /// "Let assistants change workspaces", while `mcp_on`.
    pub assistants_edit: bool,
}

/// The name `seam-mcp.exe` gives in `hello`, which General counts as an assistant.
pub const MCP_CLIENT: &str = "seam-mcp";

/// What `seam-mcp` tells an assistant while "Let assistants use Seam (MCP)" is off.
pub const MCP_OFF: &str = "Assistant access is off in Seam, General";

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

/// `config.get` for a client that isn't the panel: the config without what I typed in
/// "Open with", which only the panel sees, and with each item reduced to its kind, so no
/// folder, address, profile or startup command leaves the panel (D737).
pub fn config_for_others(config: &Value) -> Value {
    let mut config = config.clone();
    let desks = config.get_mut("desks").and_then(Value::as_object_mut);
    for desk in desks.into_iter().flat_map(|d| d.values_mut()) {
        if let Some(list) = desk.get_mut("workspaces").and_then(Value::as_array_mut) {
            *list = crate::workspace::kind_only(&crate::workspace::without_open_with(list));
        }
    }
    config
}

/// `config.get` for the panel: the startup commands opened for this account and in clear, the
/// panel being the one place they're shown and typed (D737).
pub fn config_for_panel(config: &Value, unseal: crate::workspace::Seal) -> Value {
    let mut config = config.clone();
    let desks = config.get_mut("desks").and_then(Value::as_object_mut);
    for desk in desks.into_iter().flat_map(|d| d.values_mut()) {
        if let Some(list) = desk.get_mut("workspaces").and_then(Value::as_array_mut) {
            *list = crate::workspace::for_panel(list, unseal);
        }
    }
    config
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

/// `workspace.get`: one workspace's rows, with the app's file name and what a client
/// may edit. The program's path, its AUMID and the folder or page it showed stay out, and
/// so does an item but for its kind, unless the panel asks.
pub fn workspace_get(snapshot: &Snapshot, id: &str, panel: Option<crate::workspace::Seal>) -> Reply {
    let desk = snapshot.config["desks"].get(&snapshot.state.desk.sig);
    let found = desk.and_then(|d| d["workspaces"].as_array()).and_then(|list| list.iter().find(|w| w["id"] == id));
    let Some(ws) = found else { return Err(RpcError::new(NOT_FOUND, "no such workspace on this desk")) };
    // The panel's rows carry their items, commands opened; anyone else's only the item's kind.
    let shown = {
        let one = [json!({ "windows": ws["windows"] })];
        let list = match panel {
            Some(unseal) => crate::workspace::for_panel(&one, unseal),
            None => crate::workspace::kind_only(&one),
        };
        list.first().and_then(|w| w["windows"].as_array().cloned()).unwrap_or_default()
    };
    let rows = shown.iter().enumerate().map(|(row, r)| {
        let exe = r["exe"].as_str().unwrap_or_default();
        let app = exe.rsplit('\\').next().filter(|n| !n.is_empty()).or_else(|| r["aumid"].as_str());
        let mut entry = json!({
            "row": row,
            "app": app,
            "title": r["title_hint"],
            "screen": r["monitor"],
            "target": r["target"],
            "shown": r["shown"],
            "launch": r["launch"],
            "off": r["off"].as_bool().unwrap_or(false),
        });
        // What I typed in "Open with" is the panel's alone, as in `config.get`.
        if panel.is_some() {
            entry["open_with"] = r["open_with"].clone();
        }
        if !r["item"].is_null() {
            entry["item"] = r["item"].clone();
        }
        entry
    });
    Ok(json!({
        "id": ws["id"],
        "name": ws["name"],
        "show_all": ws["show_all"],
        "other_windows": ws["other_windows"],
        "open_windows": ws.get("open_windows").cloned().unwrap_or_else(|| json!("move")),
        "rows": rows.collect::<Vec<_>>(),
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_get_shows_open_with_only_to_the_panel() {
        let mut snapshot = Snapshot::default();
        snapshot.state.desk.sig = "d".into();
        snapshot.config = json!({ "desks": { "d": { "workspaces": [
            { "id": "ws_1", "name": "A", "windows": [{ "exe": "c:\\a\\code.exe", "open_with": "c:\\proj" }] }
        ] } } });
        let others = workspace_get(&snapshot, "ws_1", None).unwrap().to_string();
        assert!(!others.contains("open_with") && !others.contains("proj"));
        assert!(others.contains("code.exe"));
        let panel = workspace_get(&snapshot, "ws_1", Some(&|_| None)).unwrap();
        assert_eq!(panel["rows"][0]["open_with"], "c:\\proj");
    }
    #[test]
    fn workspace_get_shows_only_kind_to_other_clients() {
        let mut snapshot = Snapshot::default();
        snapshot.state.desk.sig = "d".into();
        snapshot.config = json!({ "desks": { "d": { "workspaces": [
            { "id": "ws_1", "name": "A", "windows": [{ "exe": "c:\\wt.exe", "item": { "kind": "terminal", "tabs": [
                { "profile": "PowerShell", "folder": "c:\\proj", "command_sealed": "c2VhbGVk" }] } }] }
        ] } } });
        let others = workspace_get(&snapshot, "ws_1", None).unwrap();
        assert_eq!(others["rows"][0]["item"], json!({ "kind": "terminal" }));
        assert!(!others.to_string().contains("proj") && !others.to_string().contains("c2VhbGVk"));
        let panel = workspace_get(&snapshot, "ws_1", Some(&|_| Some("npm run dev".into()))).unwrap();
        assert_eq!(panel["rows"][0]["item"]["tabs"][0]["command"], "npm run dev");
        assert!(panel["rows"][0]["item"]["tabs"][0].get("command_sealed").is_none());
    }
}
