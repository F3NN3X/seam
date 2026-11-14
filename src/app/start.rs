//! Starting up: taking over from a running copy, then running the tray.

use super::*;

/// Seam is already running, so ask it to do what this launch was for. The one
/// exception is the Store copy meeting the GitHub copy: the Store copy wins, so
/// it asks the other to quit and starts in its place. Returns whether to start.
fn take_over(mut client: Client, line_up: bool) -> bool {
    let info = client.call("app.info", Value::Null).ok();
    let pid = info.as_ref().and_then(|i| i["pid"].as_u64()).and_then(|pid| u32::try_from(pid).ok());
    let github_copy = info.as_ref().is_some_and(|i| i["packaged"] == false);
    if package::is_packaged() && github_copy && client.call("app.quit", Value::Null).is_ok() {
        // It was told to close, so it will, even if slowly. Wait for it, then
        // start either way, so there's never no tray at all.
        if let Some(pid) = pid {
            process::wait_for_exit(pid, TAKE_OVER_WAIT_MS);
        }
        return true;
    }
    // Hand over foreground rights so the window that opens gets the keyboard.
    if let Some(pid) = pid {
        let _ = unsafe { AllowSetForegroundWindow(pid) };
    }
    let method = if line_up { "lineup.start" } else { "panel.open" };
    let _ = client.call(method, json!({}));
    false
}

fn arg_after(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // One instance only. Launching again opens Seam in the running one.
        let has_flag = |flag: &str| std::env::args().any(|a| a == flag);
        let line_up = has_flag("--line-up");
        let updated_from = arg_after("--updated-from");
        let _mutex = CreateMutexW(None, true, w!("Local\\Seam.SingleInstance"));
        let mutex_taken = GetLastError() == ERROR_ALREADY_EXISTS;
        // The pipe name is shared by the Store and GitHub copies, so it also
        // finds a Seam whose mutex this one can't see. A mutex alone means v0.1.
        let carry_on = match Client::connect("seam", None) {
            Ok(client) => take_over(client, line_up),
            Err(_) => !mutex_taken,
        };
        if !carry_on {
            return;
        }

        // A Store copy's first run starts clean and offers the earlier settings in the panel.
        let cfg = if package::first_run() {
            package::stash_earlier_config();
            Config::default()
        } else {
            Config::load()
        };
        // After the instance check: only the running instance owns the log.
        log::apply(cfg.debug_log);
        log::info!("seam {} started", env!("CARGO_PKG_VERSION"));

        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // A hidden top-level window (not message-only) so it receives
        // WM_DISPLAYCHANGE and the TaskbarCreated broadcast.
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!("Seam tray"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            return;
        };

        let app = App {
            hwnd,
            icon: make_icon(false),
            quiet_icon: make_icon(true),
            cfg,
            mons: Vec::new(),
            sig: String::new(),
            links: Vec::new(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            balloon_action: BalloonAction::None,
            ease_warned: false,
            cursor_easing: ease::read(),
            last_align: None,
            saver: Saver::start(),
            token: api_server::new_token().unwrap_or_default(),
            hotkey_errors: Vec::new(),
            bound: Vec::new(),
            workspace: None,
            drifted: false,
            moved: Vec::new(),
            recheck: false,
            capturing: None,
            quick_undo: None,
            tile_undo: None,
            clients: Vec::new(),
            last_state_event: None,
            packaged: package::is_packaged(),
            update: None,
            install_started: None,
            update_failed: false,
            other_wm: None,
            release_on_exit: false,
            reopen: None,
            session_offer: None,
            session_offer_tries: 0,
            sign_in_offer: None,
            sign_in_run: None,
            session_capture: None,
            session_ending: false,
            restart_apps: false,
            spotlight_resume: false,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            add_icon(a);
            migrate_hotkeys(a);
            // Before the shortcuts, so a game already in front keeps its keys.
            standby_configure(a);
            apply_hotkeys(a);
            check_restart_apps(a);
            refresh(a);
            api_server::start(a.hwnd.0 as isize, a.token.clone(), snapshot(a), a.cfg.api.enabled);
        });
        SetTimer(Some(hwnd), TIMER_REARM, 60_000, None);
        if with_app(|a| a.packaged) == Some(false) {
            SetTimer(Some(hwnd), TIMER_UPDATE, FIRST_UPDATE_CHECK_MS, None);
            package::note_unpackaged_install();
            if let Ok(tray) = std::env::current_exe() {
                autostart::add_flag(&tray);
                install::take_over_autostart(&tray);
            }
            with_app(|a| read_update_state(a));
        }
        if let Some(from) = &updated_from {
            log::info!("updated from {from}");
            let _ = std::thread::Builder::new().name("seam-update-clean".into()).spawn(clean_up_after_update);
            let body = format!("Updated to {}", env!("CARGO_PKG_VERSION"));
            with_app(|a| notify(a, "Seam", &body, BalloonAction::None));
        }
        // A launch from the Start menu or by hand opens the window too. A
        // sign-in launch, a restart after an update and a `--line-up` start don't.
        let login = package::login_launch();
        let quiet = line_up || updated_from.is_some() || has_flag("--tray-only") || login;
        if !quiet {
            with_app(|a| open_panel(a, None));
        }
        // After a sign-in, do what I chose once the desktop has settled (D120).
        if login {
            with_app(begin);
            SetTimer(Some(hwnd), TIMER_SESSION_OFFER, SESSION_OFFER_AFTER_MS, None);
        }
        if line_up {
            let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        log::info!("seam exiting");
        log::flush();
    }
}
