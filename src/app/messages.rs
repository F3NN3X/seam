//! The tray window's message handler.

use super::*;

pub(super) unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_APP_TRAY => {
                on_tray_event(hwnd, lparam);
                LRESULT(0)
            }
            m if m == WM_APP + 10 => {
                with_app(show_menu);
                LRESULT(0)
            }
            m if m == WM_APP + 12 => {
                with_app(tray_click);
                LRESULT(0)
            }
            WM_HOTKEY => {
                on_hotkey(wparam.0 as i32);
                LRESULT(0)
            }
            WM_APP_STANDBY => {
                if with_app(standby_check).is_none() {
                    SetTimer(Some(hwnd), standby::TIMER_STANDBY, 50, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == standby::TIMER_STANDBY => {
                if with_app(standby_check).is_some() {
                    let _ = KillTimer(Some(hwnd), standby::TIMER_STANDBY);
                }
                LRESULT(0)
            }
            WM_APP_SWITCHED => {
                with_app(switched);
                LRESULT(0)
            }
            WM_APP_CAPTURED => {
                with_app(captured);
                LRESULT(0)
            }
            WM_APP_WS_MOVED => {
                with_app(|a| ws_moved(a, wparam.0 as u32));
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_DRIFT => {
                if with_app(drift_due).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_DRIFT);
                }
                LRESULT(0)
            }
            WM_APP_DRIFT => {
                with_app(drift_checked);
                LRESULT(0)
            }
            WM_APP_DETECTED => {
                with_app(detected_in_use);
                LRESULT(0)
            }
            WM_APP_SESSION => {
                with_app(session_captured);
                LRESULT(0)
            }
            WM_APP_WINDOWS_CHANGED => {
                with_app(|a| windows_changed(a));
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_SESSION => {
                if with_app(start_session_capture).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_SESSION);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_SESSION_OFFER => {
                if with_app(sign_in_tick).unwrap_or(false) {
                    let _ = KillTimer(Some(hwnd), TIMER_SESSION_OFFER);
                }
                LRESULT(0)
            }
            wm::WM_APP_PIN_CLOSED => {
                with_app(pin_closed);
                LRESULT(0)
            }
            WM_QUERYENDSESSION => {
                with_app(query_end_session);
                LRESULT(1)
            }
            WM_ENDSESSION => {
                if wparam.0 != 0 {
                    with_app(end_session);
                } else {
                    // The sign-out was called off.
                    with_app(|a| a.session_ending = false);
                }
                LRESULT(0)
            }
            WM_APP_API => {
                if with_app(drain_api).is_none() {
                    SetTimer(Some(hwnd), TIMER_API, 250, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_API => {
                if with_app(drain_api).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_API);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_STATE => {
                let _ = KillTimer(Some(hwnd), TIMER_STATE);
                with_app(send_state_event);
                LRESULT(0)
            }
            quicklayout::WM_APP_QUICK_LAYOUT => {
                if with_app(tile::apply_quick).is_none() {
                    SetTimer(Some(hwnd), TIMER_QUICK, 50, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_QUICK => {
                if with_app(tile::apply_quick).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_QUICK);
                }
                LRESULT(0)
            }
            crate::dragkeys::WM_APP_DRAG_KEY => {
                snap::drag_key(wparam.0 as u32, lparam.0 as u64);
                LRESULT(0)
            }
            altdrag::WM_APP_ALT_DRAG => {
                snap::alt_drag(HWND(lparam.0 as *mut _), wparam.0 == 1);
                LRESULT(0)
            }
            dividers::WM_APP_DIVIDER => {
                if with_app(share::divider_choice).is_none() {
                    SetTimer(Some(hwnd), TIMER_DIVIDER, 50, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_DIVIDER => {
                if with_app(share::divider_choice).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_DIVIDER);
                }
                LRESULT(0)
            }
            WM_APP_ALIGNED => {
                with_app(align_finished);
                LRESULT(0)
            }
            WM_APP_UPDATE => {
                with_app(update_read);
                LRESULT(0)
            }
            WM_APP_REFIT => {
                snap::refit_found();
                LRESULT(0)
            }
            snap::WM_APP_SWAP_LOOK => {
                snap::look_ready();
                LRESULT(0)
            }
            wincontrol::WM_APP_WINDOW => {
                // Busy elsewhere: the moves wait for nothing, but a layout can't be saved.
                if with_app(window_planned).is_none() {
                    wincontrol::planned(|_, _, _| false);
                    // Its layout wasn't saved, so there is nothing to undo.
                    wincontrol::take_split();
                }
                LRESULT(0)
            }
            wm::WM_APP_WM => {
                if with_app(other_wm_changed).is_none() {
                    SetTimer(Some(hwnd), TIMER_OTHER_WM, 250, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_OTHER_WM => {
                if with_app(other_wm_changed).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_OTHER_WM);
                }
                LRESULT(0)
            }
            // The work area changed, which is what a scaling change or a moved taskbar does.
            // Windows' accent colour changed, which the focus border follows.
            WM_DWMCOLORIZATIONCOLORCHANGED => {
                wm::accent_changed();
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                if wparam.0 == SPI_SETWORKAREA.0 as usize {
                    snap::work_area_changed();
                    snap::display_changed();
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_APP_SNAP_BLOCKED => {
                let who = snap::take_blocked_app().unwrap_or_else(|| "This app".into());
                let body = format!(
                    "{who} runs as administrator, so Windows won't let Seam move it. Click for how to manage it"
                );
                with_app(|a| notify(a, "Seam", &body, BalloonAction::OpenPanel("help")));
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_UPDATE => {
                SetTimer(Some(hwnd), TIMER_UPDATE, UPDATE_CHECK_EVERY_MS, None);
                if with_app(|a| a.cfg.updates.auto_check).unwrap_or(false) {
                    check_for_updates(hwnd);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_AUTO_INSTALL => {
                with_app(auto_install);
                LRESULT(0)
            }
            WM_APP_LINEUP => {
                with_app(start_lineup);
                LRESULT(0)
            }
            overlay::WM_APP_CALIB_DONE => {
                with_app(|a| lineup_done(a, wparam.0 != 0));
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                on_display_change(hwnd, lparam);
                LRESULT(0)
            }
            WM_POWERBROADCAST => {
                if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
                    log::info!("resumed from sleep");
                    SetTimer(Some(hwnd), TIMER_REFRESH, 1500, None);
                    snap::display_changed();
                }
                LRESULT(1)
            }
            WM_TIMER if wparam.0 == TIMER_REARM => {
                hook::log_stats();
                if !overlay::is_active() {
                    with_app(|a| arm_hook(a));
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_REFRESH => {
                // If the app is busy (menu open), keep the timer and try again.
                if with_app(refresh).is_some() {
                    let _ = KillTimer(Some(hwnd), TIMER_REFRESH);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                on_destroy();
                LRESULT(0)
            }
            m if with_app(|a| a.taskbar_created == m).unwrap_or(false) => {
                // Explorer (re)started: put the icon back, and retry a prompt
                // that couldn't be shown earlier.
                with_app(|a| {
                    add_icon(a);
                    refresh(a);
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// A tray icon event: the menu and clicks are posted on, a balloon click runs its action.
unsafe fn on_tray_event(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        let event = (lparam.0 & 0xffff) as u32;
        match event {
            WM_CONTEXTMENU | WM_RBUTTONUP => {
                // The menu runs a nested loop; take the app out of the cell
                // only through with_app, and do it after this message returns.
                let _ = PostMessageW(Some(hwnd), WM_APP + 10, WPARAM(0), LPARAM(0));
            }
            NIN_SELECT | NIN_KEYSELECT => {
                let _ = PostMessageW(Some(hwnd), WM_APP + 12, WPARAM(0), LPARAM(0));
            }
            NIN_BALLOONUSERCLICK => on_balloon_click(hwnd),
            _ => {}
        }
    }
}

unsafe fn on_balloon_click(hwnd: HWND) {
    unsafe {
        let click = with_app(|a| std::mem::replace(&mut a.balloon_action, BalloonAction::None));
        match click {
            Some(BalloonAction::LineUp) => {
                let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
            }
            Some(BalloonAction::OpenPanel(page)) => {
                with_app(|a| open_panel(a, Some(page)));
            }
            Some(BalloonAction::Reopen) => {
                with_app(reopen_closed);
            }
            Some(BalloonAction::UndoQuickSave) => {
                with_app(undo_quick_save);
            }
            Some(BalloonAction::UndoTile) => {
                with_app(undo_tile);
            }
            Some(BalloonAction::OpenWorkspace) => {
                with_app(open_offered);
            }
            Some(BalloonAction::RestoreSession) => {
                with_app(|a| {
                    if let Some(Err(e)) = a.session_offer.take().map(|s| restore_session(a, s)) {
                        log::info!("last session: {}", e.message);
                    }
                });
            }
            _ => {}
        }
    }
}

unsafe fn on_hotkey(id: i32) {
    unsafe {
        match id {
            HOTKEY_LINE_UP => {
                with_app(start_lineup);
            }
            HOTKEY_PAUSE => {
                with_app(|a| set_remap(a, !a.cfg.enabled));
            }
            HOTKEY_PAUSE_SEAM => {
                with_app(|a| set_paused(a, None));
            }
            id @ (HOTKEY_NEXT_ZONE | HOTKEY_PREV_ZONE) => {
                if let Err(why) = snap::move_foreground(id == HOTKEY_NEXT_ZONE) {
                    log::info!("zone hotkey: {why}");
                }
            }
            HOTKEY_CAPTURE => {
                with_app(|a| start_capture(a, None, None, None, None, None));
            }
            id @ (HOTKEY_PIN_TOP | HOTKEY_PIN_STICKY) => {
                let kind = if id == HOTKEY_PIN_TOP { PinKind::Top } else { PinKind::Sticky };
                wm::pin(kind, None, GetForegroundWindow().0 as isize, None);
            }
            HOTKEY_RELEASE => wm::release(None),
            HOTKEY_SNAP_BACK => {
                if let Some(Err(e)) = with_app(snap_back) {
                    log::info!("snap back hotkey: {}", e.message);
                }
            }
            HOTKEY_SWITCH_BACK => {
                with_app(|a| {
                    if let Err(e) = switch_back(a) {
                        log::info!("switch back hotkey: {}", e.message);
                        notify(a, "Switch back", &e.message, BalloonAction::None);
                    }
                });
            }
            HOTKEY_FOCUS_ONLY => {
                with_app(|a| start_focus_only(a, FocusScope::Screen, None));
            }
            HOTKEY_SPOTLIGHT => {
                with_app(|a| set_spotlight(a, None));
            }
            HOTKEY_QUICK_SAVE => {
                if let Some(Some(Err(e))) = with_app(|a| quick_save(a, None)) {
                    log::info!("quick save hotkey: {}", e.message);
                }
            }
            HOTKEY_TILE_SCREEN => {
                with_app(|a| start_tile(a, tile::pointer(), None));
            }
            HOTKEY_FIT_GROUP => {
                with_app(|a| fit_group(a, None, None));
            }
            HOTKEY_DIVIDER_MENU => {
                if let Err(why) = snap::size_menu(GetForegroundWindow()) {
                    log::info!("size menu hotkey: {why}");
                }
            }
            HOTKEY_QUICK_LAYOUT => {
                if let Some(Err(why)) = with_app(|a| tile::open_quick_layout(a, None, false)) {
                    log::info!("quick layout hotkey: {why}");
                }
            }
            id @ (HOTKEY_GROW | HOTKEY_SHRINK | HOTKEY_PRIORITY) => {
                let verb = match id {
                    HOTKEY_GROW => GroupVerb::Grow(None),
                    HOTKEY_SHRINK => GroupVerb::Shrink(None),
                    _ => GroupVerb::Priority,
                };
                with_app(|a| start_group(a, verb, None));
            }
            id if (HOTKEY_WINDOW..HOTKEY_WINDOW + WINDOW_KEYS).contains(&id) => {
                with_app(|a| {
                    let (verb, dir, _) = a.cfg.hotkeys.directional()[(id - HOTKEY_WINDOW) as usize];
                    if let (Some(verb), Some(dir)) = (WindowVerb::parse(verb), Dir::parse(dir)) {
                        start_window(a, verb, dir, None);
                    }
                });
            }
            slot if slot >= HOTKEY_WORKSPACE => {
                with_app(|a| {
                    let action = a.bound.iter().find(|(id, ..)| *id == slot).map(|(_, action, _)| action.clone());
                    let id = action.as_deref().and_then(|action| action.strip_prefix("workspace:"));
                    if let Some(Err(e)) = id.map(|id| start_switch(a, Some(id), None, false)) {
                        log::info!("workspace hotkey: {}", e.message);
                    }
                });
            }
            _ => {}
        }
    }
}

unsafe fn on_display_change(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        log::info!(
            "display change: {}x{}, overlay active: {}",
            lparam.0 & 0xffff,
            (lparam.0 >> 16) & 0xffff,
            overlay::is_active()
        );
        // Screens changed under us: a half-finished line-up no longer applies.
        if overlay::is_active() {
            overlay::cancel();
        }
        SetTimer(Some(hwnd), TIMER_REFRESH, 800, None);
        snap::display_changed();
    }
}

unsafe fn on_destroy() {
    unsafe {
        hook::enable(false);
        if with_app(|a| a.release_on_exit).unwrap_or(false) {
            wm::release_before_exit();
        }
        snap::shutdown();
        altdrag::stop();
        spotlight::stop();
        with_app(|a| {
            let nid = tray_data(a);
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            if let Some(saver) = a.saver.take() {
                saver.finish();
            }
        });
        // Last, so the tray icon doesn't linger while the hook waits for a release.
        altdrag::shutdown();
        PostQuitMessage(0);
    }
}
