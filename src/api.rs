//! The local API: JSON-RPC over JSON Lines on a per-user named pipe. This is
//! the protocol and the blocking client; the server is in the tray.

use crate::direction;
use crate::log;
use crate::process;
use crate::state::Snapshot;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;

pub const API_VERSION: u32 = 1;
/// Longest accepted line, in bytes.
pub const MAX_LINE: usize = 1 << 20;
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

/// Which pin `window.pin` sets (D106).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    Top,
    Sticky,
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
    },
    WorkspaceCapture {
        name: Option<String>,
        into: Option<String>,
    },
    RemapSet(bool),
    ZonesSet(bool),
    ZoneMove(Dir),
    Window(WindowVerb, direction::Dir),
    /// `on` is None to toggle.
    WindowPin {
        kind: PinKind,
        on: Option<bool>,
    },
    WmRelease,
    LineupStart,
    ArrangementAlign,
    ScreenIdentify(u32),
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

fn str_param(params: &Value, key: &str) -> Result<Option<String>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(RpcError::new(INVALID_PARAMS, format!("{key} must be a string"))),
    }
}

fn on_param(params: &Value) -> Result<bool, RpcError> {
    params.get("on").and_then(Value::as_bool).ok_or_else(|| RpcError::new(INVALID_PARAMS, "on must be true or false"))
}

fn command(method: &str, p: &Value) -> Result<Command, RpcError> {
    Ok(match method {
        "hello" => Command::Hello {
            client: str_param(p, "client")?.unwrap_or_else(|| "anonymous".into()),
            version: str_param(p, "version")?.unwrap_or_default(),
            token: str_param(p, "token")?,
        },
        "app.info" => Command::AppInfo,
        "state.get" => Command::StateGet,
        "config.get" => Command::ConfigGet,
        "config.patch" => Command::ConfigPatch(
            p.get("patch")
                .filter(|v| v.is_object())
                .cloned()
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "patch must be an object"))?,
        ),
        "desks.list" => Command::DesksList,
        "workspaces.list" => Command::WorkspacesList { desk: str_param(p, "desk")? },
        "workspace.switch" => {
            let (id, name) = (str_param(p, "id")?, str_param(p, "name")?);
            if id.is_none() && name.is_none() {
                return Err(RpcError::new(INVALID_PARAMS, "id or name is required"));
            }
            Command::WorkspaceSwitch { id, name }
        }
        "workspace.capture" => Command::WorkspaceCapture { name: str_param(p, "name")?, into: str_param(p, "into")? },
        "remap.set" => Command::RemapSet(on_param(p)?),
        "zones.set" => Command::ZonesSet(on_param(p)?),
        "zone.move" => match p.get("dir").and_then(Value::as_str) {
            Some("next") => Command::ZoneMove(Dir::Next),
            Some("prev") => Command::ZoneMove(Dir::Prev),
            _ => return Err(RpcError::new(INVALID_PARAMS, "dir must be next or prev")),
        },
        "window.focus" | "window.move" | "window.swap" => {
            let verb = method.strip_prefix("window.").and_then(WindowVerb::parse);
            match (verb, p.get("dir").and_then(Value::as_str).and_then(direction::Dir::parse)) {
                (Some(verb), Some(dir)) => Command::Window(verb, dir),
                _ => return Err(RpcError::new(INVALID_PARAMS, "dir must be left, right, up or down")),
            }
        }
        "window.pin" => {
            let kind = match p.get("kind").and_then(Value::as_str) {
                Some("top") => PinKind::Top,
                Some("sticky") => PinKind::Sticky,
                _ => return Err(RpcError::new(INVALID_PARAMS, "kind must be top or sticky")),
            };
            let on = match p.get("on") {
                None | Some(Value::Null) => None,
                Some(_) => Some(on_param(p)?),
            };
            Command::WindowPin { kind, on }
        }
        "wm.release" => Command::WmRelease,
        "lineup.start" => Command::LineupStart,
        "arrangement.align" => Command::ArrangementAlign,
        "screen.identify" => Command::ScreenIdentify(
            p.get("number")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "number must be a screen number"))?,
        ),
        "panel.open" => Command::PanelOpen(str_param(p, "page")?),
        "events.subscribe" => Command::EventsSubscribe(
            p.get("topics")
                .and_then(Value::as_array)
                .and_then(|t| t.iter().map(|v| v.as_str().map(String::from)).collect())
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "topics must be a list of strings"))?,
        ),
        "update.ready" => Command::UpdateReady,
        "app.quit" => Command::AppQuit,
        _ => return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}"))),
    })
}

fn allowed(command: &Command, peer: Peer) -> Result<(), RpcError> {
    // The token in `hello` is what makes a client the panel, so it always gets through.
    if matches!(command, Command::Hello { .. }) {
        return Ok(());
    }
    if !peer.api_enabled && !peer.panel && !peer.own_image {
        return Err(RpcError::new(DISABLED, "Other apps can't control Seam right now"));
    }
    let permitted = match command {
        Command::ConfigPatch(_) => peer.panel,
        Command::AppQuit | Command::WorkspaceCapture { .. } => peer.panel || peer.own_image,
        Command::UpdateReady => peer.own_image,
        _ => true,
    };
    permitted.then_some(()).ok_or_else(|| RpcError::new(FORBIDDEN, "Only Seam can do that"))
}

/// Turn one request line into the command it asks for.
pub fn parse_request(line: &str, peer: Peer) -> Result<(Value, Command), Rejected> {
    let request: Value =
        serde_json::from_str(line).map_err(|e| (Value::Null, RpcError::new(PARSE_ERROR, e.to_string())))?;
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let reject = |code, message: &str| (id.clone(), RpcError::new(code, message));
    if !request.is_object() || request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(reject(INVALID_REQUEST, "not a JSON-RPC 2.0 request"));
    }
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Err(reject(INVALID_REQUEST, "method is missing"));
    };
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let parsed = command(method, &params).and_then(|c| allowed(&c, peer).map(|()| c));
    match parsed {
        Ok(command) => Ok((id, command)),
        Err(e) => Err((id, e)),
    }
}

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
    let workspaces = desk["workspaces"].as_array().into_iter().flatten().map(|w| {
        json!({
            "id": w["id"],
            "name": w["name"],
            "hotkey": w["hotkey"],
            "windows": w["windows"].as_array().map_or(0, Vec::len),
            "active": w["id"].as_str() == active,
        })
    });
    Ok(Value::Array(workspaces.collect()))
}

/// `\\.\pipe\seam-<username>`. Clients derive it, so no discovery file is needed.
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default().to_lowercase();
    format!(r"\\.\pipe\seam-{user}")
}

const ERROR_PIPE_BUSY: i32 = 231;

/// The process serving this pipe runs as the same user as this one.
fn server_is_ours(pipe: &File) -> bool {
    let mut pid = 0u32;
    let got = unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) };
    got.is_ok() && process::runs_as_us(pid)
}

/// Anyone can create a pipe under a name this predictable, so the server is
/// checked, and it only gets to identify us, never to act as us.
fn open_pipe() -> io::Result<File> {
    for _ in 0..20 {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0)
            .open(pipe_name());
        match opened {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => std::thread::sleep(Duration::from_millis(50)),
            Ok(pipe) if !server_is_ours(&pipe) => {
                log::info!("api client: the pipe is served by another user's process, so it was not used");
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Something else is using Seam's pipe"));
            }
            other => return other,
        }
    }
    Err(io::ErrorKind::TimedOut.into())
}

/// Why a call didn't produce a result.
#[derive(Debug)]
pub enum CallError {
    Io(io::Error),
    Rpc(RpcError),
}

impl From<io::Error> for CallError {
    fn from(e: io::Error) -> Self {
        CallError::Io(e)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: String,
    pub data: Value,
}

fn event_of(message: &Value) -> Option<Event> {
    let params = message.get("params")?;
    Some(Event { kind: params.get("type")?.as_str()?.to_string(), data: params.get("data")?.clone() })
}

/// One blocking connection to the tray.
pub struct Client {
    pipe: File,
    reader: BufReader<File>,
    next_id: u64,
}

impl Client {
    pub fn connect(client: &str, token: Option<&str>) -> Result<Client, CallError> {
        let pipe = open_pipe()?;
        let reader = BufReader::new(pipe.try_clone()?);
        let mut c = Client { pipe, reader, next_id: 1 };
        let version = env!("CARGO_PKG_VERSION");
        c.call("hello", json!({ "client": client, "version": version, "token": token }))?;
        Ok(c)
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.pipe, "{request}")?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            let message: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
            if message["id"] != id {
                continue; // An event, or a late answer to something else.
            }
            if let Some(e) = message.get("error") {
                let code = e["code"].as_i64().and_then(|c| i32::try_from(c).ok()).unwrap_or(INVALID_REQUEST);
                return Err(CallError::Rpc(RpcError::new(code, e["message"].as_str().unwrap_or_default())));
            }
            return Ok(message["result"].clone());
        }
    }

    /// Events arrive on a connection of their own. A pipe handle opened for
    /// blocking I/O can't write while another thread sits in a read on it, so
    /// this keeps `call` usable on the first connection.
    pub fn subscribe(client: &str, token: Option<&str>, topics: &[&str]) -> Result<Receiver<Event>, CallError> {
        let mut events = Client::connect(client, token)?;
        events.call("events.subscribe", json!({ "topics": topics }))?;
        let (tx, rx) = channel();
        std::thread::Builder::new().name("seam-api-events".into()).spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                if !matches!(events.reader.read_line(&mut line), Ok(n) if n > 0) {
                    return;
                }
                let message = serde_json::from_str::<Value>(&line).ok();
                if let Some(event) = message.as_ref().and_then(event_of) {
                    if tx.send(event).is_err() {
                        return;
                    }
                }
            }
        })?;
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_lines_become_commands_or_the_right_error() {
        let anyone = Peer { api_enabled: true, ..Peer::default() };
        let panel = Peer { panel: true, api_enabled: true, ..Peer::default() };
        let own = Peer { own_image: true, ..Peer::default() };
        let off = Peer::default();
        let request =
            |method: &str, params: &str| format!(r#"{{"jsonrpc":"2.0","id":7,"method":"{method}","params":{params}}}"#);

        let accepted = [
            (request("state.get", "{}"), anyone, Command::StateGet),
            (request("remap.set", r#"{"on":false}"#), anyone, Command::RemapSet(false)),
            (request("zone.move", r#"{"dir":"prev"}"#), anyone, Command::ZoneMove(Dir::Prev)),
            (
                request("window.move", r#"{"dir":"left"}"#),
                anyone,
                Command::Window(WindowVerb::Move, direction::Dir::Left),
            ),
            (
                request("window.focus", r#"{"dir":"down"}"#),
                anyone,
                Command::Window(WindowVerb::Focus, direction::Dir::Down),
            ),
            (request("screen.identify", r#"{"number":3}"#), anyone, Command::ScreenIdentify(3)),
            (request("panel.open", r#"{"page":"about"}"#), anyone, Command::PanelOpen(Some("about".into()))),
            (
                request("workspace.switch", r#"{"name":"Coding"}"#),
                anyone,
                Command::WorkspaceSwitch { id: None, name: Some("Coding".into()) },
            ),
            (
                request("events.subscribe", r#"{"topics":["state"]}"#),
                anyone,
                Command::EventsSubscribe(vec!["state".into()]),
            ),
            (request("config.patch", r#"{"patch":{"enabled":false}}"#), panel, {
                Command::ConfigPatch(json!({ "enabled": false }))
            }),
            (request("app.quit", "{}"), panel, Command::AppQuit),
            (request("app.quit", "{}"), own, Command::AppQuit),
            (request("update.ready", "{}"), own, Command::UpdateReady),
            (request("app.info", "{}"), own, Command::AppInfo),
            (
                request("hello", r#"{"client":"streamdeck","token":"t"}"#),
                off,
                Command::Hello { client: "streamdeck".into(), version: String::new(), token: Some("t".into()) },
            ),
        ];
        for (line, peer, expected) in accepted {
            assert_eq!(parse_request(&line, peer).map(|(_, c)| c), Ok(expected), "{line}");
        }

        let rejected = [
            ("not json".to_string(), anyone, PARSE_ERROR),
            ("[]".to_string(), anyone, INVALID_REQUEST),
            (r#"{"id":1,"method":"state.get"}"#.to_string(), anyone, INVALID_REQUEST),
            (r#"{"jsonrpc":"2.0","id":1}"#.to_string(), anyone, INVALID_REQUEST),
            (request("window.close", "{}"), anyone, METHOD_NOT_FOUND),
            (request("remap.set", r#"{"on":"yes"}"#), anyone, INVALID_PARAMS),
            (request("zone.move", r#"{"dir":"up"}"#), anyone, INVALID_PARAMS),
            (request("window.swap", r#"{"dir":"next"}"#), anyone, INVALID_PARAMS),
            (request("screen.identify", r#"{"number":-1}"#), anyone, INVALID_PARAMS),
            (request("workspace.switch", "{}"), anyone, INVALID_PARAMS),
            (request("config.patch", r#"{"patch":3}"#), panel, INVALID_PARAMS),
            (request("config.patch", r#"{"patch":{}}"#), anyone, FORBIDDEN),
            (request("app.quit", "{}"), anyone, FORBIDDEN),
            (request("update.ready", "{}"), panel, FORBIDDEN),
            (request("state.get", "{}"), off, DISABLED),
            (request("remap.set", r#"{"on":true}"#), off, DISABLED),
        ];
        for (line, peer, code) in rejected {
            assert_eq!(parse_request(&line, peer).map_err(|(_, e)| e.code), Err(code), "{line}");
        }

        let (id, _) = parse_request(&request("state.get", "{}"), anyone).unwrap();
        assert_eq!(id, 7);
        let (id, _) = parse_request(&request("nope", "{}"), anyone).unwrap_err();
        assert_eq!(id, 7, "errors answer under the request's id");
    }
}
