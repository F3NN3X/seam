//! API commands and the config patch, moved out of `app.rs`.

use super::*;

pub(super) fn refused(message: impl Into<String>) -> Reply {
    Err(RpcError::new(api::REFUSED, message))
}

pub(super) fn busy(message: &str) -> Reply {
    Err(RpcError::new(api::BUSY, message))
}

/// Merge a patch into the settings, keep it only if the result is a valid
/// config, and apply it live.
pub(super) fn patch_config(app: &mut App, patch: &Value) -> Reply {
    let mut next = match app.cfg.patched(patch) {
        Ok(c) => c,
        Err(e) => return refused(format!("That isn't a valid setting: {e}")),
    };
    if next.version != app.cfg.version {
        return refused("That config is from a different version of Seam");
    }
    next.layouts.clear();
    // What Windows' Restart apps was before Seam changed it is the tray's to keep.
    next.restore_apps_saved = app.cfg.restore_apps_saved;
    let remap_turned_on = next.enabled && !app.cfg.enabled;
    app.cfg = next;
    forget_deleted(app);
    sync_restart_apps(app);
    persist(app);
    log::apply(app.cfg.debug_log);
    apply_hotkeys(app);
    relink(app);
    if remap_turned_on {
        warn_about_ease(app);
    }
    publish(app);
    arm_auto_install(app);
    Ok(json!({ "ok": true }))
}

/// Mode sets can stall input for seconds, so this runs off the main thread with
/// the hook removed. Windows may silently remove a hook that stalls.
pub(super) fn start_align(app: &mut App, reply: Sender<Reply>) -> Option<Reply> {
    if overlay::is_active() {
        return Some(busy("Line Up is open"));
    }
    let mut slot = ALIGNING.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Some(busy("Already aligning the arrangement"));
    }
    *slot = Some(Aligning { reply, result: None });
    drop(slot);
    hook::enable(false);
    let (mons, links) = (app.mons.clone(), app.links.clone());
    let hwnd = app.hwnd.0 as isize;
    let spawned = std::thread::Builder::new().name("seam-align".into()).spawn(move || {
        let result = arrange::align(&mons, &links);
        if let Some(job) = ALIGNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            job.result = Some(result);
        }
        let hwnd = HWND(hwnd as *mut _);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_ALIGNED, WPARAM(0), LPARAM(0));
        }
    });
    if spawned.is_err() {
        ALIGNING.lock().unwrap_or_else(|e| e.into_inner()).take();
        arm_hook(app);
        return Some(busy("Couldn't start the arrangement change"));
    }
    None
}

pub(super) fn align_finished(app: &mut App) {
    let Some(job) = ALIGNING.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let result = job.result.unwrap_or_else(|| Err("the arrangement change stopped".into()));
    log::info!("arrangement result: {result:?}");
    let message = result.as_ref().err().cloned().unwrap_or_default();
    app.last_align = Some(AlignResult { ok: result.is_ok(), message: message.clone() });
    refresh(app);
    let _ = job.reply.send(if result.is_ok() { Ok(json!({ "ok": true })) } else { refused(message) });
}

/// Run one command and answer it, unless a worker will answer it later.
pub(super) fn run_command(app: &mut App, queued: Queued) {
    let ok = || Ok(json!({ "ok": true }));
    let Queued { command, reply, panel } = queued;
    let result = match command {
        Command::ConfigPatch(patch) => patch_config(app, &patch),
        Command::RemapSet(on) => {
            set_remap(app, on);
            Ok(json!({ "on": on }))
        }
        Command::ZonesSet(on) => {
            app.cfg.zones.enabled = on;
            persist(app);
            snap::update(app.hwnd, &app.cfg, &app.sig, &app.mons, aside(app).as_deref());
            publish(app);
            Ok(json!({ "on": on }))
        }
        Command::LineupStart if overlay::is_active() => busy("Line Up is already open"),
        Command::LineupStart => {
            start_lineup(app);
            ok()
        }
        Command::ArrangementAlign => {
            if let Some(refusal) = start_align(app, reply.clone()) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::ScreenIdentify(number) => match number.checked_sub(1).and_then(|i| app.mons.get(i as usize)) {
            Some(mon) => {
                identify::show(number, mon.rect);
                ok()
            }
            None => Err(RpcError::new(api::NOT_FOUND, "no such screen")),
        },
        Command::PanelOpen(page) if open_panel(app, page.as_deref()) => ok(),
        Command::PanelOpen(_) => refused("seam-settings.exe needs to be next to seam.exe"),
        Command::AppQuit => {
            // The updater and the Store copy ask without the panel's token.
            app.release_on_exit = panel && app.install_started.is_none();
            unsafe {
                let _ = PostMessageW(Some(app.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            ok()
        }
        Command::UpdateReady if app.packaged => refused("The Store updates Seam"),
        Command::UpdateReady => {
            read_update_state(app);
            ok()
        }
        Command::ZoneMove(dir) => match snap::move_foreground(dir == api::Dir::Next) {
            Ok(()) => ok(),
            Err(why) => refused(why),
        },
        Command::Window(verb, dir) => {
            start_window(app, verb, dir, Some(reply));
            return;
        }
        Command::WindowPin { kind, on } => {
            wm::pin(kind, on, unsafe { GetForegroundWindow() }.0 as isize, Some(reply));
            return;
        }
        Command::WmRelease => {
            wm::release(Some(reply));
            return;
        }
        Command::SessionRestore => match saved_session(app) {
            Some(session) => restore_session(app, session.workspace()),
            None => Err(RpcError::new(api::NOT_FOUND, "No last session is saved for this desk")),
        },
        Command::WorkspaceSwitch { id, name } => start_switch(app, id.as_deref(), name.as_deref()),
        Command::WorkspaceUpdate { id, workspace } => update_workspace(app, &id, workspace),
        Command::WorkspaceStatus { id } => match desk_workspaces(app).into_iter().find(|w| w.id == id) {
            Some(ws) => {
                live::answer_status(live_of(app, ws), reply);
                return;
            }
            None => Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")),
        },
        Command::WorkspaceSnapBack => snap_back(app),
        Command::WorkspaceSetDefault(id) => set_default(app, id),
        Command::WorkspaceQuickSave => {
            if let Some(refusal) = quick_save(app, Some(reply.clone())) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::CursorEasingSet(on) => set_cursor_easing(app, on),
        Command::AutoApplySet(on) => {
            app.cfg.auto_apply = on;
            persist(app);
            sync_live(app);
            publish(app);
            Ok(json!({ "on": on }))
        }
        Command::WorkspaceCapture { name, into, row } => {
            if let Some(refusal) = start_capture(app, name, into, row, Some(reply.clone())) {
                let _ = reply.send(refusal);
            }
            return;
        }
        // Answered on the client's own thread.
        _ => Err(RpcError::new(api::METHOD_NOT_FOUND, "not a tray command")),
    };
    let _ = reply.send(result);
}

pub(super) fn drain_api(app: &mut App) {
    if let Some(names) = api_server::take_client_names() {
        app.clients = names;
        publish(app);
    }
    for queued in api_server::take_commands() {
        run_command(app, queued);
    }
}
