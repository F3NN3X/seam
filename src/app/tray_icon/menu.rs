//! The tray menu: a native menu with the state, the workspaces and the things done every day.

use super::*;

/// One text item. A tab puts what follows at the right edge, and && is how a menu spells &.
unsafe fn add_text(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, label: &str) {
    let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = AppendMenuW(menu, flags, id, PCWSTR(label.as_ptr()));
    }
}

/// A shortcut as plain text for the right edge: "Ctrl+Shift+Alt+L" reads "Ctrl Shift Alt L".
fn hint(combo: &Option<String>) -> Option<String> {
    combo.as_ref().map(|c| c.replace('+', " "))
}

/// A label with its hint at the right edge, when it has one.
fn with_hint(label: &str, hint: Option<String>) -> String {
    match hint {
        Some(hint) => format!("{label}\t{hint}"),
        None => label.to_string(),
    }
}

/// The workspaces of the desk as radio items, the one in use marked and hinted when its windows
/// have moved. Ids run on from `ID_WORKSPACE`.
unsafe fn add_workspaces(app: &App, menu: HMENU, workspaces: &[Workspace]) {
    let active = app.workspace.as_deref();
    unsafe {
        add_text(menu, MF_STRING | MF_GRAYED, 0, "Workspace");
        for (id, ws) in (ID_WORKSPACE..).zip(workspaces) {
            let name = ws.name.replace('&', "&&");
            let moved = (active == Some(ws.id.as_str()) && app.drifted).then(|| "moved".to_string());
            // The workspace's own shortcut stays, ahead of the moved hint.
            let hints: Vec<String> = hint(&ws.hotkey).into_iter().chain(moved).collect();
            let label = with_hint(&name, (!hints.is_empty()).then(|| hints.join(", ")));
            add_text(menu, MF_STRING, id, &label);
        }
        if let Some(at) = workspaces.iter().position(|w| active == Some(w.id.as_str())) {
            let last = ID_WORKSPACE + workspaces.len() - 1;
            let _ =
                CheckMenuRadioItem(menu, ID_WORKSPACE as u32, last as u32, (ID_WORKSPACE + at) as u32, MF_BYCOMMAND.0);
        }
        let previous = app.cfg.desks.get(&app.sig).and_then(|d| d.previous_workspace.as_deref());
        if let Some(ws) = workspaces.iter().find(|w| Some(w.id.as_str()) == previous) {
            add_text(menu, MF_STRING, ID_SWITCH_BACK, &format!("Switch back to {}", ws.name.replace('&', "&&")));
        }
        let back = if active.is_some() { MF_STRING } else { MF_STRING | MF_GRAYED };
        add_text(menu, back, ID_PUT_BACK, "Put windows back");
    }
}

/// Several screens get a submenu naming them as Line Up does; one is tiled from the item itself.
unsafe fn add_tile_menu(app: &App, menu: HMENU) {
    unsafe {
        let tile_menu = if app.mons.len() > 1 { CreatePopupMenu().ok() } else { None };
        match tile_menu {
            Some(sub) => {
                for n in 1..=app.mons.len().min(ID_WORKSPACE - ID_TILE_SCREEN) {
                    add_text(sub, MF_STRING, ID_TILE_SCREEN + n - 1, &format!("Screen {n}"));
                }
                if AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Tile windows")).is_err() {
                    let _ = DestroyMenu(sub);
                }
            }
            None => {
                let _ = AppendMenuW(menu, MF_STRING, ID_TILE, w!("Tile windows"));
            }
        }
    }
}

/// Free all windows, for each screen tiled on the desktop in front: one item, or a
/// submenu naming them when there are several. None while nothing is tiled.
unsafe fn add_free_menu(app: &App, menu: HMENU, tiled: &[usize]) {
    let tiled: Vec<usize> = tiled.iter().copied().filter(|n| *n <= FREE_TILING_MAX).collect();
    unsafe {
        match tiled.as_slice() {
            [] => {}
            [one] if app.mons.len() == 1 => {
                let _ = AppendMenuW(menu, MF_STRING, ID_FREE_TILING + one - 1, w!("Free all windows"));
            }
            several => {
                let Ok(sub) = CreatePopupMenu() else { return };
                for n in several {
                    add_text(sub, MF_STRING, ID_FREE_TILING + n - 1, &format!("Screen {n}"));
                }
                if AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Free all windows")).is_err() {
                    let _ = DestroyMenu(sub);
                }
            }
        }
    }
}

/// The two lines at the top, which do nothing: the state, and the desk or what it needs.
unsafe fn add_status(app: &App, menu: HMENU) {
    let paused = standby::paused();
    let stepped_aside = standby::reason() == Some(standby::Standby::FullScreen);
    let new_screen = !paused && report::new_screen(app);
    let remap = Remap::of(app.cfg.enabled, app.links.len());
    let detail = if new_screen {
        "The pointer may jump at its edges".to_string()
    } else if stepped_aside {
        "An app is full screen".to_string()
    } else {
        let saved = app.cfg.desks.get(&app.sig).map(|d| d.name.clone()).filter(|n| !n.is_empty());
        let name = saved.unwrap_or_else(|| desk::default_name(&app.mons));
        let screens = app.mons.len();
        format!("{name} desk, {screens} screen{}", if screens == 1 { "" } else { "s" })
    };
    unsafe {
        add_text(menu, MF_STRING | MF_GRAYED, 0, words(paused, new_screen, stepped_aside, remap));
        add_text(menu, MF_STRING | MF_GRAYED, 0, &detail.replace('&', "&&"));
    }
}

pub(in crate::app) fn show_menu(app: &mut App) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        add_status(app, menu);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let workspaces = desk_workspaces(app);
        if !workspaces.is_empty() {
            add_workspaces(app, menu, &workspaces);
        }
        // Tile and free put windows where they belong, so they sit with the workspaces.
        add_tile_menu(app, menu);
        let tiled = tiling::tiled_screens(app);
        add_free_menu(app, menu, &tiled);
        let _ = AppendMenuW(menu, MF_STRING, ID_RELEASE, w!("Release pinned windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let keys = &app.cfg.hotkeys;
        add_text(menu, MF_STRING, ID_LINEUP, &with_hint("Line up screens", hint(&keys.line_up)));
        let remap = if app.cfg.enabled { "Pause pointer remap" } else { "Resume pointer remap" };
        add_text(menu, MF_STRING, ID_ENABLED, &with_hint(remap, hint(&keys.pause)));
        let pause = if standby::paused() { "Resume Seam" } else { "Pause Seam" };
        add_text(menu, MF_STRING, ID_PAUSE_SEAM, &with_hint(pause, hint(&keys.pause_seam)));
        add_text(menu, MF_STRING, ID_OVERVIEW, &with_hint("Overview", hint(&keys.overview)));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open Seam"));
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit Seam"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(app.hwnd);
        let hwnd = app.hwnd;
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 as usize {
            ID_LINEUP => start_lineup(app),
            ID_OVERVIEW => overview::open_overview(app, false, false),
            ID_OPEN => {
                open_panel(app, None);
            }
            ID_ENABLED => set_remap(app, !app.cfg.enabled),
            ID_PAUSE_SEAM => {
                set_paused(app, None);
            }
            ID_RELEASE => wm::release(None),
            ID_PUT_BACK => {
                if let Err(e) = snap_back(app, false) {
                    log::info!("workspace menu: {}", e.message);
                    notify(app, "Put windows back", &e.message, BalloonAction::None);
                }
            }
            ID_SWITCH_BACK => {
                if let Err(e) = switch_back(app, false) {
                    log::info!("workspace menu: {}", e.message);
                    notify(app, "Switch back", &e.message, BalloonAction::None);
                }
            }
            // The screen the pointer was on when the menu opened.
            ID_TILE => start_tile(app, At::Point(pt.x, pt.y), None),
            id if (ID_FREE_TILING..ID_FREE_TILING + FREE_TILING_MAX).contains(&id) => {
                tiling::free_tiling(app, Some((id - ID_FREE_TILING + 1) as u32), None);
            }
            id if (ID_TILE_SCREEN..ID_WORKSPACE).contains(&id) => {
                start_tile(app, At::Screen(id - ID_TILE_SCREEN + 1), None);
            }
            id if id >= ID_WORKSPACE => {
                let chosen = workspaces.get(id - ID_WORKSPACE).map(|w| w.id.clone());
                if let Some(Err(e)) = chosen.map(|id| start_switch(app, Some(&id), None, false)) {
                    log::info!("workspace menu: {}", e.message);
                }
            }
            ID_QUIT => {
                app.release_on_exit = true;
                // Close after this borrow ends so WM_DESTROY can clean up the icon.
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            _ => {}
        }
    }
}
