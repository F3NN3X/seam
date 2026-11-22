//! Turning a request line into a command, and who may send it.

use super::*;
use crate::workspace::Edit;

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
        "workspace.add_window" => workspace_add_window(p)?,
        "workspace.create" => Command::WorkspaceCreate { name: name_param(p)? },
        "workspace.rename" => workspace_edit(p, Edit::Rename(name_param(p)?))?,
        "workspace.edit_row" => {
            let edit = Edit::Row { row: row_param(p)?, change: row_change(p)? };
            workspace_edit(p, edit)?
        }
        "workspace.move" => workspace_move(p)?,
        "workspace.remove_row" => workspace_edit(p, Edit::RemoveRow(row_param(p)?))?,
        "workspace.set_show_all" => workspace_edit(p, Edit::ShowAll(on_param(p)?))?,
        "workspace.get" => Command::WorkspaceGet {
            id: str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?,
        },
        "workspace.status" => Command::WorkspaceStatus {
            id: str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?,
        },
        "workspace.cancel" => Command::WorkspaceCancel,
        "workspace.last_switch" => Command::WorkspaceLastSwitch { id: id_param(p)? },
        "workspace.snap_back" => Command::WorkspaceSnapBack { alongside: false },
        "workspace.switch_back" => Command::WorkspaceSwitchBack { alongside: false },
        "workspace.quick_save" => Command::WorkspaceQuickSave,
        "workspace.windows" => Command::WorkspaceWindows { id: id_param(p)? },
        "workspace.close" => workspace_close(p)?,
        "workspace.minimize" => Command::WorkspaceMinimize { id: str_param(p, "id")? },
        "workspace.minimize_undo" => Command::WorkspaceMinimizeUndo,
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
        "window.stack" => Command::WindowStack { hwnd: hwnd_param(p)?, on: on_param(p)? },
        "window.stack_cycle" => Command::WindowStackCycle(match p.get("dir").and_then(Value::as_str) {
            Some("next") => true,
            Some("prev") => false,
            _ => return Err(RpcError::new(INVALID_PARAMS, "dir must be next or prev")),
        }),
        "tiling.set" => tiling_set(p)?,
        "tiling.free" => Command::TilingFree(screen_param(p)?),
        "tiling.scroll" => Command::TilingScroll {
            number: screen_param(p)?,
            left: match p.get("dir").and_then(Value::as_str) {
                Some("left") => true,
                Some("right") => false,
                _ => return Err(RpcError::new(INVALID_PARAMS, "dir must be left or right")),
            },
        },
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
        "screen.rotate" => Command::ScreenRotate {
            number: screen_param(p)?,
            back: match p.get("dir") {
                None | Some(Value::Null) => false,
                Some(Value::String(d)) if d == "next" => false,
                Some(Value::String(d)) if d == "back" => true,
                Some(_) => return Err(RpcError::new(INVALID_PARAMS, "dir must be next or back")),
            },
        },
        "screen.reset_sizes" => Command::ScreenResetSizes(screen_param(p)?),
        "zones.from_windows" => Command::ZonesFromWindows(screen_param(p)?),
        "zones.quick" => Command::ZonesQuick(screen_param(p)?),
        "overview.show" => Command::OverviewShow,
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

fn id_param(p: &Value) -> Result<String, RpcError> {
    str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))
}

fn workspace_close(p: &Value) -> Result<Command, RpcError> {
    let id = id_param(p)?;
    let hwnds: Vec<isize> = match p.get("hwnds") {
        Some(Value::Array(list)) if !list.is_empty() => list
            .iter()
            .map(|v| v.as_i64().and_then(|n| isize::try_from(n).ok()))
            .collect::<Option<_>>()
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "hwnds must be window handles"))?,
        _ => {
            return Err(RpcError::new(INVALID_PARAMS, "hwnds must be a list of window handles from workspace.windows"));
        }
    };
    Ok(Command::WorkspaceClose { id, hwnds })
}

fn workspace_add_window(p: &Value) -> Result<Command, RpcError> {
    let id = str_param(p, "id")?;
    let number = match p.get("number") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n >= 1)
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "number must be a workspace number, from 1"))?,
        ),
    };
    if id.is_some() == number.is_some() {
        return Err(RpcError::new(INVALID_PARAMS, "give id or number, one of them"));
    }
    Ok(Command::WorkspaceAddWindow { id, number, hwnd: hwnd_param(p)? })
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

/// A workspace's `name`, trimmed and short.
fn name_param(p: &Value) -> Result<String, RpcError> {
    let name = str_param(p, "name")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "name is required"))?;
    workspace::checked_name(&name).map_err(|why| RpcError::new(INVALID_PARAMS, why))
}

fn row_param(p: &Value) -> Result<usize, RpcError> {
    p.get("row")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "row must be a row number"))
}

/// What `workspace.edit_row` may change: `target`, `shown`, `launch: false` and `off`.
/// Only an assistant is held to `off: true`, in `assistant_allowed`.
fn row_change(p: &Value) -> Result<workspace::RowChange, RpcError> {
    fn part<T: serde::de::DeserializeOwned>(p: &Value, key: &str, what: &str) -> Result<Option<T>, RpcError> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|_| RpcError::new(INVALID_PARAMS, what)),
        }
    }
    let target = part(p, "target", "target must be {zone}, {rect} or {state}")?;
    let shown = part(p, "shown", "shown must be normal, minimized, maximized or fullscreen")?;
    let launch_off = match p.get("launch") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(false)) => true,
        Some(_) => return Err(RpcError::new(INVALID_PARAMS, "launch can only be turned off")),
    };
    let off = match p.get("off") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(off)) => Some(*off),
        Some(_) => return Err(RpcError::new(INVALID_PARAMS, "off must be true or false")),
    };
    if target.is_none() && shown.is_none() && !launch_off && off.is_none() {
        return Err(RpcError::new(INVALID_PARAMS, "nothing to change"));
    }
    Ok(workspace::RowChange { target, shown, launch_off, off })
}

fn workspace_move(p: &Value) -> Result<Command, RpcError> {
    let id = str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?;
    let to = p
        .get("to")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n >= 1)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "to must be a workspace number, from 1"))?;
    Ok(Command::WorkspaceMove { id, to })
}

fn workspace_edit(p: &Value, edit: Edit) -> Result<Command, RpcError> {
    let id = str_param(p, "id")?.ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?;
    Ok(Command::WorkspaceEdit { id, edit })
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

/// `tiling.set`: `on`, `family` or both.
fn tiling_set(p: &Value) -> Result<Command, RpcError> {
    let on = match p.get("on") {
        None | Some(Value::Null) => None,
        Some(_) => Some(on_param(p)?),
    };
    let family = match str_param(p, "family")? {
        None => None,
        Some(name) => Some(crate::tiling::Family::parse(&name).ok_or_else(|| {
            RpcError::new(INVALID_PARAMS, "family must be smart, columns, main_stack, grid or scrolling")
        })?),
    };
    if on.is_none() && family.is_none() {
        return Err(RpcError::new(INVALID_PARAMS, "on or family is required"));
    }
    Ok(Command::TilingSet { number: screen_param(p)?, on, family })
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

/// What `seam-mcp.exe` may send, and only while "Let assistants use Seam (MCP)" is on.
/// Switch, Snap back and Switch back never close a window (`parse_request` makes them
/// alongside), and Quick save and edits need the second switch. Everything else, launch settings, Close and Open with included,
/// stays out of reach.
fn assistant_allowed(command: &Command, peer: Peer) -> Result<(), RpcError> {
    if !peer.mcp_on {
        return Err(RpcError::new(DISABLED, MCP_OFF));
    }
    if !peer.api_enabled {
        return Err(RpcError::new(DISABLED, "Other apps can't control Seam right now"));
    }
    match command {
        Command::AppInfo
        | Command::StateGet
        | Command::DesksList
        | Command::WorkspacesList { .. }
        | Command::WorkspaceGet { .. }
        | Command::WorkspaceStatus { .. }
        | Command::WorkspaceSwitch { .. }
        | Command::WorkspaceCancel
        | Command::WorkspaceSnapBack { .. }
        | Command::WorkspaceSwitchBack { .. }
        | Command::EventsSubscribe(_) => Ok(()),
        // Turning a row on can make a switch start a program, the same reason `launch` can only be turned off.
        Command::WorkspaceEdit {
            edit: Edit::Row { change: workspace::RowChange { off: Some(false), .. }, .. },
            ..
        } if peer.assistants_edit => {
            Err(RpcError::new(INVALID_PARAMS, "A row can only be turned off here, never back on"))
        }
        // Quick save adds a row for every window on screen, so it is an edit.
        Command::WorkspaceCreate { .. } | Command::WorkspaceEdit { .. } | Command::WorkspaceQuickSave
            if peer.assistants_edit =>
        {
            Ok(())
        }
        Command::WorkspaceCreate { .. } | Command::WorkspaceEdit { .. } | Command::WorkspaceQuickSave => {
            Err(RpcError::new(FORBIDDEN, "Changing workspaces is off for assistants in Seam, General"))
        }
        _ => Err(RpcError::new(FORBIDDEN, "Assistants can't do that")),
    }
}

fn allowed(command: &Command, peer: Peer) -> Result<(), RpcError> {
    // The token in `hello` is what makes a client the panel, so it always gets through.
    if matches!(command, Command::Hello { .. }) {
        return Ok(());
    }
    if peer.mcp && !peer.panel {
        return assistant_allowed(command, peer);
    }
    if !peer.api_enabled && !peer.panel && !peer.own_image {
        return Err(RpcError::new(DISABLED, "Other apps can't control Seam right now"));
    }
    let permitted = match command {
        Command::ConfigPatch(_)
        | Command::ZonesPick { .. }
        | Command::WorkspaceUpdate { .. }
        | Command::WorkspaceCreate { .. }
        | Command::WorkspaceEdit { .. }
        | Command::WorkspaceMove { .. }
        | Command::CursorEasingSet(_)
        | Command::LayoutShow(_) => peer.panel,
        Command::AppQuit
        | Command::WorkspaceCapture { .. }
        | Command::SessionRestore
        | Command::WorkspaceSetDefault(_)
        | Command::WorkspaceMinimizeUndo => peer.panel || peer.own_image,
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
    // An assistant's switch never closes windows, whatever it asked for.
    let parsed = parsed.map(|c| match c {
        Command::WorkspaceSwitch { id, name, .. } if peer.mcp => Command::WorkspaceSwitch { id, name, alongside: true },
        Command::WorkspaceSnapBack { .. } if peer.mcp => Command::WorkspaceSnapBack { alongside: true },
        Command::WorkspaceSwitchBack { .. } if peer.mcp => Command::WorkspaceSwitchBack { alongside: true },
        c => c,
    });
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
            (request("window.stack", r#"{"on":true}"#), anyone, Command::WindowStack { hwnd: None, on: true }),
            (
                request("window.stack", r#"{"on":false,"hwnd":1234}"#),
                anyone,
                Command::WindowStack { hwnd: Some(1234), on: false },
            ),
            (request("window.stack_cycle", r#"{"dir":"next"}"#), anyone, Command::WindowStackCycle(true)),
            (
                request("tiling.set", r#"{"number":2,"on":true,"family":"main_stack"}"#),
                anyone,
                Command::TilingSet { number: Some(2), on: Some(true), family: Some(crate::tiling::Family::MainStack) },
            ),
            (
                request("tiling.set", r#"{"family":"grid"}"#),
                anyone,
                Command::TilingSet { number: None, on: None, family: Some(crate::tiling::Family::Grid) },
            ),
            (request("tiling.free", r#"{"number":1}"#), anyone, Command::TilingFree(Some(1))),
            (
                request("tiling.set", r#"{"family":"scrolling"}"#),
                anyone,
                Command::TilingSet { number: None, on: None, family: Some(crate::tiling::Family::Scrolling) },
            ),
            (request("tiling.scroll", r#"{"dir":"left"}"#), anyone, Command::TilingScroll { number: None, left: true }),
            (
                request("tiling.scroll", r#"{"number":2,"dir":"right"}"#),
                anyone,
                Command::TilingScroll { number: Some(2), left: false },
            ),
            (request("screen.rotate", "{}"), anyone, Command::ScreenRotate { number: None, back: false }),
            (
                request("screen.rotate", r#"{"number":2,"dir":"back"}"#),
                anyone,
                Command::ScreenRotate { number: Some(2), back: true },
            ),
            (request("screen.reset_sizes", r#"{"number":3}"#), anyone, Command::ScreenResetSizes(Some(3))),
            (
                request("workspace.add_window", r#"{"number":2}"#),
                anyone,
                Command::WorkspaceAddWindow { id: None, number: Some(2), hwnd: None },
            ),
            (
                request("workspace.add_window", r#"{"id":"ws_1","hwnd":4660}"#),
                anyone,
                Command::WorkspaceAddWindow { id: Some("ws_1".into()), number: None, hwnd: Some(4660) },
            ),
            (request("overview.show", "{}"), anyone, Command::OverviewShow),
            (request("window.stack_cycle", r#"{"dir":"prev"}"#), anyone, Command::WindowStackCycle(false)),
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
            (request("workspace.snap_back", "{}"), anyone, Command::WorkspaceSnapBack { alongside: false }),
            (request("workspace.switch_back", "{}"), anyone, Command::WorkspaceSwitchBack { alongside: false }),
            (request("workspace.quick_save", "{}"), anyone, Command::WorkspaceQuickSave),
            (request("workspace.cancel", "{}"), anyone, Command::WorkspaceCancel),
            (
                request("workspace.last_switch", r#"{"id":"ws_1"}"#),
                anyone,
                Command::WorkspaceLastSwitch { id: "ws_1".into() },
            ),
            (request("workspace.windows", r#"{"id":"ws_1"}"#), anyone, Command::WorkspaceWindows { id: "ws_1".into() }),
            (
                request("workspace.close", r#"{"id":"ws_1","hwnds":[4660,22]}"#),
                anyone,
                Command::WorkspaceClose { id: "ws_1".into(), hwnds: vec![4660, 22] },
            ),
            (request("workspace.minimize", "{}"), anyone, Command::WorkspaceMinimize { id: None }),
            (
                request("workspace.minimize", r#"{"id":"ws_1"}"#),
                anyone,
                Command::WorkspaceMinimize { id: Some("ws_1".into()) },
            ),
            (request("workspace.minimize_undo", "{}"), panel, Command::WorkspaceMinimizeUndo),
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
            (request("window.stack", "{}"), anyone, INVALID_PARAMS),
            (request("window.stack_cycle", r#"{"dir":"left"}"#), anyone, INVALID_PARAMS),
            (request("tiling.set", "{}"), anyone, INVALID_PARAMS),
            (request("tiling.set", r#"{"family":"dwindle"}"#), anyone, INVALID_PARAMS),
            (request("tiling.scroll", "{}"), anyone, INVALID_PARAMS),
            (request("tiling.scroll", r#"{"dir":"up"}"#), anyone, INVALID_PARAMS),
            (request("screen.identify", r#"{"number":-1}"#), anyone, INVALID_PARAMS),
            (request("screen.rotate", r#"{"dir":"left"}"#), anyone, INVALID_PARAMS),
            (request("screen.rotate", r#"{"dir":5}"#), anyone, INVALID_PARAMS),
            (request("workspace.move", r#"{"id":"ws_1","to":0}"#), panel, INVALID_PARAMS),
            (request("workspace.move", r#"{"id":"ws_1"}"#), panel, INVALID_PARAMS),
            (request("workspace.move", r#"{"to":2}"#), panel, INVALID_PARAMS),
            (request("workspace.move", r#"{"id":"ws_1","to":2}"#), anyone, FORBIDDEN),
            (request("workspace.add_window", "{}"), anyone, INVALID_PARAMS),
            (request("workspace.add_window", r#"{"id":"ws_1","number":1}"#), anyone, INVALID_PARAMS),
            (request("workspace.add_window", r#"{"number":0}"#), anyone, INVALID_PARAMS),
            (request("workspace.add_window", r#"{"number":-1}"#), anyone, INVALID_PARAMS),
            (request("workspace.add_window", r#"{"number":1,"hwnd":"x"}"#), anyone, INVALID_PARAMS),
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
            (request("workspace.close", r#"{"id":"ws_1"}"#), anyone, INVALID_PARAMS),
            (request("workspace.close", r#"{"id":"ws_1","hwnds":[]}"#), anyone, INVALID_PARAMS),
            (request("workspace.close", r#"{"id":"ws_1","hwnds":"4660"}"#), anyone, INVALID_PARAMS),
            (request("workspace.close", r#"{"hwnds":[1]}"#), anyone, INVALID_PARAMS),
            (request("workspace.windows", "{}"), anyone, INVALID_PARAMS),
            (request("workspace.minimize_undo", "{}"), anyone, FORBIDDEN),
            (request("workspace.close", r#"{"id":"ws_1","hwnds":[1]}"#), off, DISABLED),
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

    #[test]
    fn assistants_get_a_short_list_behind_two_switches() {
        let assistant =
            |edit| Peer { mcp: true, mcp_on: true, assistants_edit: edit, api_enabled: true, ..Peer::default() };
        let off = Peer { mcp: true, api_enabled: true, ..Peer::default() };
        let panel = Peer { panel: true, api_enabled: true, ..Peer::default() };
        let request =
            |method: &str, params: &str| format!(r#"{{"jsonrpc":"2.0","id":7,"method":"{method}","params":{params}}}"#);
        let code = |line: &str, peer| parse_request(line, peer).map(|(_, c)| c).map_err(|(_, e)| (e.code, e.message));

        // A switch from an assistant never closes windows, whatever it sent.
        let switch = code(&request("workspace.switch", r#"{"name":"Mail","alongside":false}"#), assistant(false));
        assert_eq!(switch, Ok(Command::WorkspaceSwitch { id: None, name: Some("Mail".into()), alongside: true }));
        for method in ["state.get", "workspaces.list"] {
            assert!(code(&request(method, "{}"), assistant(false)).is_ok(), "{method}");
        }
        // Snap back and Switch back never close windows either.
        assert_eq!(
            code(&request("workspace.snap_back", "{}"), assistant(false)),
            Ok(Command::WorkspaceSnapBack { alongside: true })
        );
        assert_eq!(
            code(&request("workspace.switch_back", "{}"), assistant(false)),
            Ok(Command::WorkspaceSwitchBack { alongside: true })
        );
        // Cancel only stops things, so an assistant may, even with the second switch off.
        assert_eq!(code(&request("workspace.cancel", "{}"), assistant(false)), Ok(Command::WorkspaceCancel));
        assert_eq!(code(&request("workspace.cancel", "{}"), off).map_err(|e| e.0), Err(DISABLED));
        // Quick save is an edit, and needs the second switch.
        let quick = request("workspace.quick_save", "{}");
        assert_eq!(code(&quick, assistant(false)).map_err(|e| e.0), Err(FORBIDDEN));
        assert_eq!(code(&quick, assistant(true)), Ok(Command::WorkspaceQuickSave));
        // The API switch has to be on too, after hello as well as at it.
        let api_off = Peer { api_enabled: false, ..assistant(true) };
        assert_eq!(code(&request("state.get", "{}"), api_off).map_err(|e| e.0), Err(DISABLED));
        assert_eq!(code(&request("workspace.switch_back", "{}"), api_off).map_err(|e| e.0), Err(DISABLED));
        assert!(code(&request("workspace.status", r#"{"id":"ws_1"}"#), assistant(false)).is_ok());

        // With the master switch off, every call says so.
        let refused = code(&request("state.get", "{}"), off);
        assert_eq!(refused, Err((DISABLED, MCP_OFF.to_string())));
        assert_eq!(code(&request("workspace.rename", r#"{"id":"a","name":"b"}"#), off).map_err(|e| e.0), Err(DISABLED));

        // Nothing outside the list: no closing, no settings, no Open with, no full workspace write.
        let forbidden = [
            ("config.patch", r#"{"patch":{}}"#),
            ("workspace.update", r#"{"id":"ws_1","workspace":{}}"#),
            ("workspace.capture", r#"{"into":"ws_1"}"#),
            ("workspace.set_default", r#"{"id":"ws_1"}"#),
            ("window.fit", "{}"),
            ("window.stack_cycle", r#"{"dir":"next"}"#),
            ("window.stack", r#"{"on":true}"#),
            ("tiling.set", r#"{"on":true}"#),
            ("tiling.free", "{}"),
            ("tiling.scroll", r#"{"dir":"left"}"#),
            ("screen.rotate", "{}"),
            ("screen.reset_sizes", "{}"),
            ("workspace.add_window", r#"{"number":1}"#),
            ("workspace.move", r#"{"id":"ws_1","to":2}"#),
            ("overview.show", "{}"),
            ("app.quit", "{}"),
            ("session.restore", "{}"),
            ("layout.show", r#"{"view":"seam"}"#),
            ("workspace.windows", r#"{"id":"ws_1"}"#),
            ("workspace.close", r#"{"id":"ws_1","hwnds":[4660]}"#),
            ("workspace.minimize", "{}"),
            ("workspace.minimize_undo", "{}"),
            ("workspace.last_switch", r#"{"id":"ws_1"}"#),
        ];
        for (method, params) in forbidden {
            let got = code(&request(method, params), assistant(true)).map_err(|e| e.0);
            assert_eq!(got, Err(FORBIDDEN), "{method}");
        }

        // Edits wait for the second switch.
        let edits = [
            ("workspace.create", r#"{"name":"New"}"#),
            ("workspace.rename", r#"{"id":"ws_1","name":"Mail"}"#),
            ("workspace.edit_row", r#"{"id":"ws_1","row":0,"shown":"minimized","launch":false}"#),
            ("workspace.remove_row", r#"{"id":"ws_1","row":1}"#),
            ("workspace.set_show_all", r#"{"id":"ws_1","on":true}"#),
        ];
        for (method, params) in edits {
            assert_eq!(code(&request(method, params), assistant(false)).map_err(|e| e.0), Err(FORBIDDEN), "{method}");
            assert!(code(&request(method, params), assistant(true)).is_ok(), "{method}");
            assert!(code(&request(method, params), panel).is_ok(), "{method}");
        }
        let anyone = Peer { api_enabled: true, ..Peer::default() };
        assert_eq!(code(&request("workspace.create", r#"{"name":"New"}"#), anyone).map_err(|e| e.0), Err(FORBIDDEN));

        // A row edit can't turn launch on, set a program, or change nothing.
        let row = |params: &str| code(&request("workspace.edit_row", params), assistant(true)).map_err(|e| e.0);
        assert_eq!(row(r#"{"id":"a","row":0,"launch":true}"#), Err(INVALID_PARAMS));
        // It can turn a row off but never on, as with launch. The Seam window can do both.
        assert!(row(r#"{"id":"a","row":0,"off":true}"#).is_ok());
        assert_eq!(row(r#"{"id":"a","row":0,"off":false}"#), Err(INVALID_PARAMS));
        assert_eq!(row(r#"{"id":"a","row":0,"off":"yes"}"#), Err(INVALID_PARAMS));
        let turn_on = request("workspace.edit_row", r#"{"id":"a","row":0,"off":false}"#);
        assert_eq!(code(&turn_on, assistant(false)).map_err(|e| e.0), Err(FORBIDDEN));
        assert!(code(&turn_on, panel).is_ok());
        let moved = code(&request("workspace.move", r#"{"id":"a","to":2}"#), panel);
        assert_eq!(moved, Ok(Command::WorkspaceMove { id: "a".into(), to: 2 }));
        assert_eq!(row(r#"{"id":"a","row":0}"#), Err(INVALID_PARAMS));
        assert_eq!(row(r#"{"id":"a","row":0,"shown":"hidden"}"#), Err(INVALID_PARAMS));
        // Fields it doesn't know, a program among them, are ignored.
        let want = Command::WorkspaceEdit {
            id: "a".into(),
            edit: Edit::Row {
                row: 0,
                change: workspace::RowChange {
                    target: Some(workspace::Target::Zone { zone: 2, span_to: None }),
                    ..Default::default()
                },
            },
        };
        let evil = r#"{"id":"a","row":0,"target":{"zone":2},"exe":"c:\\evil.exe","open_with":"x"}"#;
        assert_eq!(code(&request("workspace.edit_row", evil), assistant(true)), Ok(want));
    }
}
