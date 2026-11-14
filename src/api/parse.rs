//! Turning a request line into a command, and who may send it.

use super::*;

fn str_param(params: &Value, key: &str) -> Result<Option<String>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(RpcError::new(INVALID_PARAMS, format!("{key} must be a string"))),
    }
}

/// An optional `number`, a screen's Line Up number.
fn screen_param(params: &Value) -> Result<Option<u32>, RpcError> {
    match params.get("number") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "number must be a screen number")),
    }
}

/// An optional `hwnd`, a window handle.
fn hwnd_param(params: &Value) -> Result<Option<isize>, RpcError> {
    match params.get("hwnd") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .and_then(|n| isize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "hwnd must be a window handle")),
    }
}

fn on_param(params: &Value) -> Result<bool, RpcError> {
    params.get("on").and_then(Value::as_bool).ok_or_else(|| RpcError::new(INVALID_PARAMS, "on must be true or false"))
}

fn command(method: &str, p: &Value) -> Result<Command, RpcError> {
    Ok(match method {
        "hello" => hello(p)?,
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
        "workspace.switch" => workspace_switch(p)?,
        "workspace.capture" => workspace_capture(p)?,
        "workspace.update" => workspace_update(p)?,
        "workspace.status" => Command::WorkspaceStatus {
            id: str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?,
        },
        "workspace.snap_back" => Command::WorkspaceSnapBack,
        "workspace.switch_back" => Command::WorkspaceSwitchBack,
        "workspace.quick_save" => Command::WorkspaceQuickSave,
        "workspace.set_default" => Command::WorkspaceSetDefault(str_param(p, "id")?),
        "autoapply.set" => Command::AutoApplySet(on_param(p)?),
        "cursor_easing.set" => Command::CursorEasingSet(on_param(p)?),
        "remap.set" => Command::RemapSet(on_param(p)?),
        "zones.set" => Command::ZonesSet(on_param(p)?),
        "zone.move" => match p.get("dir").and_then(Value::as_str) {
            Some("next") => Command::ZoneMove(Dir::Next),
            Some("prev") => Command::ZoneMove(Dir::Prev),
            _ => return Err(RpcError::new(INVALID_PARAMS, "dir must be next or prev")),
        },
        "window.focus" | "window.move" | "window.swap" => window_verb(method, p)?,
        "window.resize" => window_resize(p)?,
        "window.priority" => Command::WindowGroup(GroupVerb::Priority),
        "window.share" => window_share(p)?,
        "window.ungroup" => Command::WindowUngroup { hwnd: hwnd_param(p)? },
        "window.fit" => Command::WindowFit { hwnd: hwnd_param(p)? },
        "window.pin" => window_pin(p)?,
        "window.focus_only" => window_focus_only(p)?,
        "window.spotlight" => Command::WindowSpotlight(match p.get("on") {
            None | Some(Value::Null) => None,
            Some(_) => Some(on_param(p)?),
        }),
        "seam.pause" => Command::SeamPause(match p.get("on") {
            None | Some(Value::Null) => None,
            Some(_) => Some(on_param(p)?),
        }),
        "wm.release" => Command::WmRelease,
        "session.restore" => Command::SessionRestore,
        "lineup.start" => Command::LineupStart,
        "arrangement.align" => Command::ArrangementAlign,
        "screen.identify" => Command::ScreenIdentify(
            p.get("number")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "number must be a screen number"))?,
        ),
        "screen.tile" => Command::ScreenTile(screen_param(p)?),
        "zones.from_windows" => Command::ZonesFromWindows(screen_param(p)?),
        "zones.quick" => Command::ZonesQuick(screen_param(p)?),
        "zones.pick" => zones_pick(p)?,
        "zones.apply" => zones_apply(p)?,
        "zones.layouts" => Command::ZonesLayouts(screen_param(p)?),
        "layout.show" => match p.get("view").and_then(Value::as_str) {
            Some("seam") => Command::LayoutShow(LayoutView::Seam),
            Some("windows") => Command::LayoutShow(LayoutView::Windows),
            _ => return Err(RpcError::new(INVALID_PARAMS, "view must be seam or windows")),
        },
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

fn hello(p: &Value) -> Result<Command, RpcError> {
    Ok(Command::Hello {
        client: str_param(p, "client")?.unwrap_or_else(|| "anonymous".into()),
        version: str_param(p, "version")?.unwrap_or_default(),
        token: str_param(p, "token")?,
    })
}

fn workspace_switch(p: &Value) -> Result<Command, RpcError> {
    let (id, name) = (str_param(p, "id")?, str_param(p, "name")?);
    if id.is_none() && name.is_none() {
        return Err(RpcError::new(INVALID_PARAMS, "id or name is required"));
    }
    let alongside = p.get("alongside").and_then(Value::as_bool).unwrap_or(false);
    Ok(Command::WorkspaceSwitch { id, name, alongside })
}

fn workspace_capture(p: &Value) -> Result<Command, RpcError> {
    let row = match p.get("row") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "row must be a row number"))?,
        ),
    };
    let into = str_param(p, "into")?;
    if row.is_some() && into.is_none() {
        return Err(RpcError::new(INVALID_PARAMS, "row needs into"));
    }
    let hwnd = match p.get("hwnd") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_i64()
                .and_then(|n| isize::try_from(n).ok())
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "hwnd must be a window handle"))?,
        ),
    };
    let pick = match (hwnd, row) {
        (None, _) => None,
        (Some(_), None) => return Err(RpcError::new(INVALID_PARAMS, "hwnd needs into and row")),
        (Some(hwnd), Some(_)) => {
            let need =
                |key| str_param(p, key)?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "hwnd needs exe and class"));
            Some(Pick { hwnd, exe: need("exe")?, class: need("class")? })
        }
    };
    Ok(Command::WorkspaceCapture { name: str_param(p, "name")?, into, row, pick })
}

fn workspace_update(p: &Value) -> Result<Command, RpcError> {
    Ok(Command::WorkspaceUpdate {
        id: str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?,
        workspace: p
            .get("workspace")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "workspace must be an object"))?,
    })
}

/// `window.focus`, `window.move` and `window.swap`: the verb is the method's suffix.
fn window_verb(method: &str, p: &Value) -> Result<Command, RpcError> {
    let verb = method.strip_prefix("window.").and_then(WindowVerb::parse);
    match (verb, p.get("dir").and_then(Value::as_str).and_then(direction::Dir::parse)) {
        (Some(verb), Some(dir)) => Ok(Command::Window(verb, dir)),
        _ => Err(RpcError::new(INVALID_PARAMS, "dir must be left, right, up or down")),
    }
}

fn window_resize(p: &Value) -> Result<Command, RpcError> {
    let step = match p.get("step") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| i32::try_from(n).ok())
                .filter(|n| (1..=MAX_STEP_PX).contains(n))
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "step must be 1 to 10000 pixels"))?,
        ),
    };
    match p.get("by").and_then(Value::as_str) {
        Some("grow") => Ok(Command::WindowGroup(GroupVerb::Grow(step))),
        Some("shrink") => Ok(Command::WindowGroup(GroupVerb::Shrink(step))),
        _ => Err(RpcError::new(INVALID_PARAMS, "by must be grow or shrink")),
    }
}

fn window_share(p: &Value) -> Result<Command, RpcError> {
    Ok(Command::WindowShare {
        hwnd: hwnd_param(p)?,
        axis: match p.get("axis").and_then(Value::as_str) {
            Some("width") => Axis::Width,
            Some("height") => Axis::Height,
            _ => return Err(RpcError::new(INVALID_PARAMS, "axis must be width or height")),
        },
        fraction: p
            .get("fraction")
            .and_then(Value::as_f64)
            .filter(|f| (MIN_SHARE..=MAX_SHARE).contains(f))
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "fraction must be 0.1 to 0.9"))? as f32,
    })
}

fn window_pin(p: &Value) -> Result<Command, RpcError> {
    let kind = match p.get("kind").and_then(Value::as_str) {
        Some("top") => PinKind::Top,
        Some("sticky") => PinKind::Sticky,
        _ => return Err(RpcError::new(INVALID_PARAMS, "kind must be top or sticky")),
    };
    let on = match p.get("on") {
        None | Some(Value::Null) => None,
        Some(_) => Some(on_param(p)?),
    };
    Ok(Command::WindowPin { kind, on })
}

fn window_focus_only(p: &Value) -> Result<Command, RpcError> {
    Ok(Command::WindowFocusOnly(match p.get("scope") {
        None | Some(Value::Null) => FocusScope::Screen,
        Some(v) => match v.as_str() {
            Some("screen") => FocusScope::Screen,
            Some("workspace") => FocusScope::Workspace,
            _ => return Err(RpcError::new(INVALID_PARAMS, "scope must be screen or workspace")),
        },
    }))
}

fn zones_pick(p: &Value) -> Result<Command, RpcError> {
    Ok(Command::ZonesPick {
        number: screen_param(p)?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "number is required"))?,
        layout: match p.get("layout") {
            None | Some(Value::Null) => None,
            Some(v) if v.is_object() => Some(v.clone()),
            Some(_) => return Err(RpcError::new(INVALID_PARAMS, "layout must be a layout or null")),
        },
    })
}

fn zones_apply(p: &Value) -> Result<Command, RpcError> {
    Ok(Command::ZonesApply {
        number: screen_param(p)?,
        layout: match p.get("layout") {
            Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => return Err(RpcError::new(INVALID_PARAMS, "layout must be a layout id or name, or null")),
        },
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
        Command::ConfigPatch(_)
        | Command::ZonesPick { .. }
        | Command::WorkspaceUpdate { .. }
        | Command::CursorEasingSet(_)
        | Command::LayoutShow(_) => peer.panel,
        Command::AppQuit
        | Command::WorkspaceCapture { .. }
        | Command::SessionRestore
        | Command::WorkspaceSetDefault(_) => peer.panel || peer.own_image,
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
            (request("window.resize", r#"{"by":"grow"}"#), anyone, Command::WindowGroup(GroupVerb::Grow(None))),
            (
                request("window.resize", r#"{"by":"shrink","step":40}"#),
                anyone,
                Command::WindowGroup(GroupVerb::Shrink(Some(40))),
            ),
            (request("window.priority", "{}"), anyone, Command::WindowGroup(GroupVerb::Priority)),
            (
                request("window.share", r#"{"axis":"width","fraction":0.5}"#),
                anyone,
                Command::WindowShare { hwnd: None, axis: Axis::Width, fraction: 0.5 },
            ),
            (
                request("window.share", r#"{"hwnd":1234,"axis":"height","fraction":0.25}"#),
                anyone,
                Command::WindowShare { hwnd: Some(1234), axis: Axis::Height, fraction: 0.25 },
            ),
            (request("window.ungroup", "{}"), anyone, Command::WindowUngroup { hwnd: None }),
            (request("window.ungroup", r#"{"hwnd":1234}"#), anyone, Command::WindowUngroup { hwnd: Some(1234) }),
            (request("window.fit", r#"{"hwnd":1234}"#), anyone, Command::WindowFit { hwnd: Some(1234) }),
            (request("screen.identify", r#"{"number":3}"#), anyone, Command::ScreenIdentify(3)),
            (request("panel.open", r#"{"page":"about"}"#), anyone, Command::PanelOpen(Some("about".into()))),
            (
                request("workspace.switch", r#"{"name":"Coding"}"#),
                anyone,
                Command::WorkspaceSwitch { id: None, name: Some("Coding".into()), alongside: false },
            ),
            (request("workspace.status", r#"{"id":"ws_1"}"#), anyone, Command::WorkspaceStatus { id: "ws_1".into() }),
            (
                request("zones.apply", r#"{"number":2,"layout":"Grid"}"#),
                anyone,
                Command::ZonesApply { number: Some(2), layout: Some("Grid".into()) },
            ),
            (request("zones.apply", r#"{"layout":null}"#), anyone, Command::ZonesApply { number: None, layout: None }),
            (request("zones.layouts", "{}"), anyone, Command::ZonesLayouts(None)),
            (request("workspace.snap_back", "{}"), anyone, Command::WorkspaceSnapBack),
            (request("workspace.switch_back", "{}"), anyone, Command::WorkspaceSwitchBack),
            (request("workspace.quick_save", "{}"), anyone, Command::WorkspaceQuickSave),
            (request("autoapply.set", r#"{"on":true}"#), anyone, Command::AutoApplySet(true)),
            (
                request("events.subscribe", r#"{"topics":["state"]}"#),
                anyone,
                Command::EventsSubscribe(vec!["state".into()]),
            ),
            (request("config.patch", r#"{"patch":{"enabled":false}}"#), panel, {
                Command::ConfigPatch(json!({ "enabled": false }))
            }),
            (
                request("workspace.update", r#"{"id":"ws_1","workspace":{"id":"ws_1"}}"#),
                panel,
                Command::WorkspaceUpdate { id: "ws_1".into(), workspace: json!({ "id": "ws_1" }) },
            ),
            (
                request("workspace.capture", r#"{"into":"ws_1","row":2}"#),
                panel,
                Command::WorkspaceCapture { name: None, into: Some("ws_1".into()), row: Some(2), pick: None },
            ),
            (request("cursor_easing.set", r#"{"on":true}"#), panel, Command::CursorEasingSet(true)),
            (request("layout.show", r#"{"view":"windows"}"#), panel, Command::LayoutShow(LayoutView::Windows)),
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
            (request("window.resize", r#"{"by":"wider"}"#), anyone, INVALID_PARAMS),
            (request("window.resize", r#"{"by":"grow","step":0}"#), anyone, INVALID_PARAMS),
            (request("window.resize", r#"{"by":"grow","step":10001}"#), anyone, INVALID_PARAMS),
            (request("window.share", r#"{"axis":"depth","fraction":0.5}"#), anyone, INVALID_PARAMS),
            (request("window.share", r#"{"axis":"width","fraction":0.95}"#), anyone, INVALID_PARAMS),
            (request("window.share", r#"{"axis":"width"}"#), anyone, INVALID_PARAMS),
            (request("window.ungroup", r#"{"hwnd":"x"}"#), anyone, INVALID_PARAMS),
            (request("screen.identify", r#"{"number":-1}"#), anyone, INVALID_PARAMS),
            (request("workspace.switch", "{}"), anyone, INVALID_PARAMS),
            (request("zones.apply", "{}"), anyone, INVALID_PARAMS),
            (request("zones.apply", r#"{"layout":{"template":"grid"}}"#), anyone, INVALID_PARAMS),
            (request("zones.layouts", r#"{"number":"2"}"#), anyone, INVALID_PARAMS),
            (request("workspace.status", "{}"), anyone, INVALID_PARAMS),
            (request("autoapply.set", "{}"), anyone, INVALID_PARAMS),
            (request("config.patch", r#"{"patch":3}"#), panel, INVALID_PARAMS),
            (request("config.patch", r#"{"patch":{}}"#), anyone, FORBIDDEN),
            (request("workspace.update", r#"{"id":"ws_1","workspace":{}}"#), anyone, FORBIDDEN),
            (request("workspace.capture", r#"{"row":1}"#), panel, INVALID_PARAMS),
            (request("app.quit", "{}"), anyone, FORBIDDEN),
            (request("cursor_easing.set", r#"{"on":true}"#), anyone, FORBIDDEN),
            (request("cursor_easing.set", r#"{"on":true}"#), own, FORBIDDEN),
            (request("update.ready", "{}"), panel, FORBIDDEN),
            (request("layout.show", r#"{"view":"seam"}"#), anyone, FORBIDDEN),
            (request("layout.show", r#"{"view":"map"}"#), panel, INVALID_PARAMS),
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
