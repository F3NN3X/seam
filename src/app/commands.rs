//! API commands and the config patch, moved out of `app.rs`.

use super::*;

pub(super) fn refused(message: impl Into<String>) -> Reply {
    Err(RpcError::new(api::REFUSED, message))
}

pub(super) fn busy(message: &str) -> Reply {
    Err(RpcError::new(api::BUSY, message))
}

/// The screen a request names, or the tray's rule when it names none (D785).
fn on(screen: Option<u32>) -> OnScreen {
    screen.map_or(OnScreen::Unnamed, OnScreen::Number)
}

/// The panel's patch with its items checked and each startup command in it sealed, so none
/// reaches config.json in clear and none is one the panel didn't have (D736).
fn seal_items(patch: &mut Value) {
    let desks = patch.get_mut("desks").and_then(Value::as_object_mut);
    for desk in desks.into_iter().flat_map(|d| d.values_mut()) {
        if let Some(list) = desk.get_mut("workspaces").and_then(Value::as_array_mut) {
            workspace::seal_patch(list);
        }
        // The last session is a capture's, which never has an item.
        if let Some(session) = desk.get_mut("last_session").filter(|s| s.is_object()) {
            *session = workspace::without_items(std::slice::from_ref(session)).remove(0);
        }
    }
}

/// A patch that changes a row's item, compared with what is saved, leaves that row's window behind:
/// the next switch starts the new item (D733). Rows are compared in the order the list holds them.
fn forget_edited_items(app: &App, patch: &Value) {
    let desks = patch.get("desks").and_then(Value::as_object).into_iter().flatten();
    for (sig, desk) in desks {
        let (Some(sent), Some(saved)) = (desk.get("workspaces").and_then(Value::as_array), app.cfg.desks.get(sig))
        else {
            continue;
        };
        let old = workspace::list(&saved.workspaces);
        for new in workspace::list(sent) {
            let Some(before) = old.iter().find(|w| w.id == new.id) else { continue };
            let moves: Vec<Option<usize>> =
                (0..before.windows.len()).map(|i| (i < new.windows.len()).then_some(i)).collect();
            switcher::forget_edited(&new.id, &workspace::edited_items(&before.windows, &new.windows, &moves));
        }
    }
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
    standby_configure(app);
    apply_hotkeys(app);
    relink(app);
    arm(app);
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
    let result = job.result.unwrap_or_else(|| Err("The arrangement change stopped".into()));
    log::info!("arrangement result: {result:?}");
    let message = result.as_ref().err().cloned().unwrap_or_default();
    app.last_align = Some(AlignResult { ok: result.is_ok(), message: message.clone() });
    refresh(app);
    let _ = job.reply.send(if result.is_ok() { Ok(json!({ "ok": true })) } else { refused(message) });
}

/// Run one command and answer it, unless a worker will answer it later.
pub(super) fn run_command(app: &mut App, queued: Queued) {
    let Queued { command, reply, panel, client, assistant } = queued;
    let result = match command {
        Command::WorkspaceCreate { name } => create_workspace(app, &client, assistant, &name),
        Command::WorkspaceEdit { id, edit } => edit_workspace(app, &client, &id, &edit),
        Command::ArrangementAlign => {
            if let Some(refusal) = start_align(app, reply.clone()) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::ScreenTile(Some(number)) if number.checked_sub(1).and_then(|i| app.mons.get(i as usize)).is_none() => {
            Err(RpcError::new(api::NOT_FOUND, "no such screen"))
        }
        // Answered once the setting is made, which may wait to know the desktop in front.
        Command::TilingSet { number, on, family } => {
            tiling::set_tiling(app, number, on, family, Some(reply));
            return;
        }
        Command::TilingFree(number) => {
            tiling::free_tiling(app, number, Some(reply));
            return;
        }
        Command::TilingScroll { number, left } => {
            tiling::scroll(app, number, left, Some(reply));
            return;
        }
        Command::ScreenRotate { number, back } => {
            turn::start_turn(app, Verb::Rotate, number, !back, Some(reply));
            return;
        }
        Command::ScreenResetSizes(number) => {
            turn::start_turn(app, Verb::Reset, number, true, Some(reply));
            return;
        }
        Command::ScreenTile(number) => {
            let at = number.map_or_else(tile::pointer, |n| At::Screen(n as usize));
            start_tile(app, at, Some(reply));
            return;
        }
        Command::ZonesFromWindows(Some(number))
            if number.checked_sub(1).and_then(|i| app.mons.get(i as usize)).is_none() =>
        {
            Err(RpcError::new(api::NOT_FOUND, "no such screen"))
        }
        Command::ZonesPick { number, layout } => {
            tile::pick_layout(app, number, layout, None, reply);
            return;
        }
        Command::ZonesApply { number, layout } => {
            tile::apply_layout(app, number, layout, reply);
            return;
        }
        Command::ZonesFromWindows(number) => {
            let at = number.map_or_else(tile::pointer, |n| At::Screen(n as usize));
            tile::start_from_windows(app, at, reply);
            return;
        }
        Command::Window(verb, dir) => {
            start_window(app, verb, dir, Some(reply));
            return;
        }
        Command::WindowFocusOnly(scope) => {
            start_focus_only(app, scope, Some(reply));
            return;
        }
        Command::WindowGroup(verb) => {
            start_group(app, verb, Some(reply));
            return;
        }
        Command::WindowShare { hwnd, axis, fraction } => {
            share::start_share(app, hwnd, GroupVerb::Share { axis, fraction, side: None }, Some(reply));
            return;
        }
        Command::WindowFit { hwnd } => {
            fit_group(app, hwnd, Some(reply));
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
        Command::WorkspaceStatus { id } => match desk_workspaces(app).into_iter().find(|w| w.id == id) {
            Some(ws) => {
                live::answer_status(live_of(app, ws), reply);
                return;
            }
            None => Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")),
        },
        Command::WorkspaceCancel => Ok(json!({ "cancelled": switcher::cancel() })),
        Command::WorkspaceLastSwitch { id } => Ok(last_switch(app, &id)),
        Command::WorkspaceQuickSave { screen } => {
            if let Some(refusal) = quick_save(app, on(screen), Some(reply.clone())) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::WorkspaceWindows { id } => {
            let from = (Some(reply.clone()), client.as_str());
            if let Some(refusal) = start_close(app, (Some(&id), OnScreen::Unnamed), (Ask::List, Vec::new()), from) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::WorkspaceClose { id, hwnds } => {
            let from = (Some(reply.clone()), client.as_str());
            if let Some(refusal) = start_close(app, (Some(&id), OnScreen::Unnamed), (Ask::Close, hwnds), from) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::WorkspaceMinimize { id, screen } => {
            let from = (Some(reply.clone()), client.as_str());
            if let Some(refusal) = start_close(app, (id.as_deref(), on(screen)), (Ask::Minimize, Vec::new()), from) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::WorkspaceMinimizeUndo if undo_minimize(app) => Ok(json!({ "ok": true })),
        Command::WorkspaceMinimizeUndo => refused("Nothing to undo"),
        Command::WorkspaceAddWindow { id, number, hwnd } => {
            let which = match id {
                Some(id) => Which::Id(id),
                None => Which::Number(number.unwrap_or(0) as usize),
            };
            start_add_window(app, which, hwnd, Some(reply.clone()));
            return;
        }
        Command::WorkspaceCapture { name, into, save: Some(save), .. } => {
            if let Some(refusal) = save_preview(app, (name, into), save, (reply.clone(), panel)) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::WorkspaceCapture { name, into, row, pick, save: None } => {
            let refusal = start_capture(app, name, into, row, pick, Some(reply.clone()));
            if let Some(job) = app.capturing.as_mut().filter(|_| refusal.is_none()) {
                job.from_panel = panel;
            }
            if let Some(refusal) = refusal {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::WorkspaceCapturePreview { into } => {
            if let Some(refusal) = start_preview(app, into, reply.clone()) {
                let _ = reply.send(refusal);
            }
            return;
        }
        Command::ConfigSetFolder { path, take } => {
            sync::set_folder(app, path, take, reply.clone());
            return;
        }
        other => command_result(app, other, panel),
    };
    let _ = reply.send(result);
}

/// The commands the tray thread answers itself, without handing the reply on.
fn command_result(app: &mut App, command: Command, panel: bool) -> Reply {
    let ok = || Ok(json!({ "ok": true }));
    match command {
        Command::ConfigPatch(mut patch) => {
            seal_items(&mut patch);
            forget_edited_items(app, &patch);
            patch_config(app, &patch)
        }
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
        Command::ZonesQuick(Some(number)) if number.checked_sub(1).and_then(|i| app.mons.get(i as usize)).is_none() => {
            Err(RpcError::new(api::NOT_FOUND, "no such screen"))
        }
        Command::ZonesQuick(number) => match tile::open_quick_layout(app, number, true) {
            Ok(()) => ok(),
            Err(why) => refused(why),
        },
        Command::OverviewShow => {
            overview::open_overview(app, false, true);
            ok()
        }
        Command::ScreenIdentify(number) => match number.checked_sub(1).and_then(|i| app.mons.get(i as usize)) {
            Some(mon) => {
                identify::show(number, mon.rect);
                ok()
            }
            None => Err(RpcError::new(api::NOT_FOUND, "no such screen")),
        },
        Command::LayoutShow(view) => {
            showlayout::show(view, &app.mons, &app.links);
            ok()
        }
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
        Command::WindowSpotlight(on) => {
            let now = set_spotlight(app, on);
            Ok(json!({ "on": now, "resumes": app.spotlight_resume }))
        }
        Command::SeamPause(on) => Ok(json!({ "paused": set_paused(app, on) })),
        Command::WindowUngroup { hwnd } => share::take_out(hwnd),
        Command::WindowStack { hwnd, on } => {
            let h = hwnd.map_or_else(|| unsafe { GetForegroundWindow() }, |h| HWND(h as *mut _));
            match snap::stack_set(h, on) {
                Ok(windows) => Ok(json!({ "on": on, "windows": windows })),
                Err(why) => refused(why),
            }
        }
        Command::WindowStackCycle(next) => match snap::stack_cycle(unsafe { GetForegroundWindow() }, next, true) {
            Ok((hwnd, position, windows)) => Ok(json!({ "hwnd": hwnd, "position": position, "windows": windows })),
            Err(why) => refused(why),
        },
        Command::SessionRestore => match saved_session(app) {
            Some(session) => restore_session(app, session.workspace()),
            None => Err(RpcError::new(api::NOT_FOUND, "No last session is saved for this desk")),
        },
        Command::WorkspaceSwitch { id, name, alongside } => {
            start_switch(app, id.as_deref(), name.as_deref(), alongside, Via::api(panel))
        }
        Command::WorkspaceMove { id, to } => move_workspace(app, &id, to),
        Command::WorkspaceCopyFromDesk { from } => copy_from_desk(app, &from),
        Command::WorkspaceUpdate { id, workspace } => update_from_panel(app, &id, workspace),
        Command::WorkspaceSnapBack { alongside, screen } => snap_back(app, on(screen), alongside, Via::api(panel)),
        Command::WorkspaceSwitchBack { alongside, screen } => switch_back(app, on(screen), alongside, Via::api(panel)),
        Command::WorkspaceStep { direction, alongside } => {
            step(app, direction, OnScreen::Unnamed, alongside, Via::api(panel))
        }
        Command::WorkspaceSetDefault(id) => set_default(app, id),
        Command::CursorEasingSet(on) => set_cursor_easing(app, on),
        Command::AutoApplySet(on) => {
            app.cfg.auto_apply = on;
            persist(app);
            sync_live(app);
            publish(app);
            Ok(json!({ "on": on }))
        }
        // Answered on the client's own thread.
        _ => Err(RpcError::new(api::METHOD_NOT_FOUND, "not a tray command")),
    }
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
