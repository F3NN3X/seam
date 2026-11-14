//! The tray icon, its tooltip, balloons and menu, moved out of `app.rs`.

use super::*;

/// Draw the tray icon: two screens of different sizes with a line running straight across.
/// The quiet one, for while Seam stands by, is the same drawing in greys.
pub(super) fn make_icon(quiet: bool) -> HICON {
    const N: usize = 32;
    let mut px = vec![0u32; N * N];
    let mut rect = |x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                px[y * N + x] = c;
            }
        }
    };
    let (white, amber) = if quiet { (0xFF949494, 0xFF5C5C5C) } else { (0xFFF2F4F8, 0xFFFFB020) };
    // Left (smaller) screen outline.
    rect(1, 10, 13, 11, white);
    rect(1, 22, 13, 23, white);
    rect(1, 10, 2, 23, white);
    rect(12, 10, 13, 23, white);
    // Right (bigger) screen outline.
    rect(16, 5, 30, 6, white);
    rect(16, 26, 30, 27, white);
    rect(16, 5, 17, 27, white);
    rect(29, 5, 30, 27, white);
    // The seam line, straight across both.
    rect(3, 16, 28, 17, amber);
    unsafe {
        let color = CreateBitmap(N as i32, N as i32, 1, 32, Some(px.as_ptr() as *const _));
        let mask_bits = [0u8; N * N / 8];
        let mask = CreateBitmap(N as i32, N as i32, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

pub(super) fn tray_data(app: &App) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: app.hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: if standby::reason().is_some() { app.quiet_icon } else { app.icon },
        ..Default::default()
    };
    let tip = if let Some(why) = standby::reason() {
        match why {
            standby::Standby::Paused => "Seam — paused",
            standby::Standby::FullScreen => "Seam — waiting for the full-screen app",
        }
        .to_string()
    } else if app.links.is_empty() {
        "Seam — screens not lined up yet".to_string()
    } else if !app.cfg.enabled {
        "Seam — remap paused".to_string()
    } else {
        format!("Seam — {} border(s) lined up", app.links.len())
    };
    nid.szTip = wide(&tip);
    nid
}

pub(super) fn add_icon(app: &App) {
    unsafe {
        let mut nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

pub(super) fn update_tip(app: &App) {
    unsafe {
        let nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Show a balloon/toast. Returns whether the shell accepted it.
pub(super) fn notify(app: &mut App, title: &str, body: &str, click: BalloonAction) -> bool {
    app.balloon_action = click;
    unsafe {
        let mut nid = tray_data(app);
        nid.uFlags |= NIF_INFO;
        nid.szInfoTitle = wide(title);
        nid.szInfo = wide(body);
        nid.dwInfoFlags = NIIF_INFO;
        Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
    }
}

fn checked_flag(on: bool) -> MENU_ITEM_FLAGS {
    if on { MF_STRING | MF_CHECKED } else { MF_STRING }
}

/// The Workspaces submenu, made only when there are workspaces to put in it.
unsafe fn add_workspaces_menu(app: &App, menu: HMENU, workspaces: &[Workspace]) {
    unsafe {
        // Only made when there is something to put in it, so no menu handle is dropped unused.
        let submenu = if workspaces.is_empty() { None } else { CreatePopupMenu().ok() };
        if let Some(submenu) = submenu {
            let active = app.workspace.as_deref();
            let previous = app.cfg.desks.get(&app.sig).and_then(|d| d.previous_workspace.as_deref());
            if let Some(ws) = workspaces.iter().find(|w| Some(w.id.as_str()) == previous) {
                let label = format!("Switch back to {}", ws.name.replace('&', "&&"));
                let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(submenu, MF_STRING, ID_SWITCH_BACK, PCWSTR(label.as_ptr()));
                let _ = AppendMenuW(submenu, MF_SEPARATOR, 0, None);
            }
            for (id, ws) in (ID_WORKSPACE..).zip(workspaces) {
                // A tab puts the shortcut at the right edge, and && is how a menu spells &.
                let mut label = ws.name.replace('&', "&&");
                if let Some(combo) = &ws.hotkey {
                    label = format!("{label}\t{combo}");
                }
                let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(submenu, checked_flag(active == Some(ws.id.as_str())), id, PCWSTR(label.as_ptr()));
            }
            if AppendMenuW(menu, MF_POPUP, submenu.0 as usize, w!("Workspaces")).is_err() {
                let _ = DestroyMenu(submenu);
            }
        }
    }
}

/// Several screens get a submenu naming them as Line Up does; one is tiled from the item itself.
unsafe fn add_tile_menu(app: &App, menu: HMENU) {
    unsafe {
        let tile_menu = if app.mons.len() > 1 { CreatePopupMenu().ok() } else { None };
        match tile_menu {
            Some(sub) => {
                for n in 1..=app.mons.len().min(ID_WORKSPACE - ID_TILE_SCREEN) {
                    let label: Vec<u16> = format!("Screen {n}").encode_utf16().chain(std::iter::once(0)).collect();
                    let _ = AppendMenuW(sub, MF_STRING, ID_TILE_SCREEN + n - 1, PCWSTR(label.as_ptr()));
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

pub(super) fn show_menu(app: &mut App) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let status = match (app.mons.len(), app.links.len()) {
            (n, 0) => format!("{n} screen(s), not lined up yet"),
            (n, k) => format!("{n} screens, {k} border(s) lined up"),
        };
        let status_w: Vec<u16> = status.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status_w.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open Seam…"));
        let pause = if standby::paused() { w!("Resume Seam") } else { w!("Pause Seam") };
        let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE_SEAM, pause);
        let _ = AppendMenuW(menu, MF_STRING, ID_LINEUP, w!("Line Up Screens…"));
        let workspaces = desk_workspaces(app);
        add_workspaces_menu(app, menu, &workspaces);
        add_tile_menu(app, menu);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, checked_flag(app.cfg.enabled), ID_ENABLED, w!("Remap pointer at borders"));
        let _ = AppendMenuW(menu, MF_STRING, ID_RELEASE, w!("Release pinned windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
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
            ID_OPEN => {
                open_panel(app, None);
            }
            ID_ENABLED => set_remap(app, !app.cfg.enabled),
            ID_PAUSE_SEAM => {
                set_paused(app, None);
            }
            ID_RELEASE => wm::release(None),
            ID_SWITCH_BACK => {
                if let Err(e) = switch_back(app) {
                    log::info!("workspace menu: {}", e.message);
                    notify(app, "Switch back", &e.message, BalloonAction::None);
                }
            }
            // The screen the pointer was on when the menu opened.
            ID_TILE => start_tile(app, At::Point(pt.x, pt.y), None),
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
