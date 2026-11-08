//! The updater's side of the tray: reading its state file and installing when idle.

use super::*;

/// What a worker found in the updater's state file.
struct UpdateRead {
    update: Option<UpdateState>,
    failed: bool,
    /// A new version, so say so.
    announce: bool,
}

/// Hands the state file's contents from a worker to the main thread.
static UPDATE_READ: Mutex<Option<UpdateRead>> = Mutex::new(None);

/// Start `seam-settings.exe` with no window, for the updater.
pub(super) fn run_settings(args: &[&str]) -> Option<std::process::Child> {
    let panel = std::env::current_exe().ok()?.with_file_name("seam-settings.exe");
    std::process::Command::new(panel).args(args).creation_flags(CREATE_NO_WINDOW).spawn().ok()
}

/// Read the updater's state file, then tell the main thread. Runs on a worker.
pub(super) fn read_update_file(hwnd: isize) {
    let file = updates::read();
    let current = env!("CARGO_PKG_VERSION");
    let failed = file.error.is_some();
    let available = file.available.filter(|v| v != current && updates::is_plain_version(v));
    let update = available.map(|available| UpdateState { available, ready: file.ready });
    let new = update.as_ref().filter(|u| u.ready && updates::notified().as_deref() != Some(u.available.as_str()));
    let announce = new.is_some();
    if let Some(new) = new {
        updates::set_notified(&new.available);
    }
    // Two workers can finish before the main thread looks. Keep an announcement either found.
    let mut slot = UPDATE_READ.lock().unwrap_or_else(|e| e.into_inner());
    let announce = announce || slot.as_ref().is_some_and(|earlier| earlier.announce);
    *slot = Some(UpdateRead { update, failed, announce });
    drop(slot);
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_UPDATE, WPARAM(0), LPARAM(0));
    }
}

pub(super) fn read_update_state(app: &App) {
    let hwnd = app.hwnd.0 as isize;
    let _ = std::thread::Builder::new().name("seam-update-state".into()).spawn(move || read_update_file(hwnd));
}

/// Run the check as its own process, so the tray never touches the network.
pub(super) fn check_for_updates(hwnd: HWND) {
    let hwnd = hwnd.0 as isize;
    let _ = std::thread::Builder::new().name("seam-update-check".into()).spawn(move || {
        if let Some(mut check) = run_settings(&["--check-updates"]) {
            let _ = check.wait();
        }
        read_update_file(hwnd);
    });
}

/// The updater has news, or the file was read at start.
pub(super) fn update_read(app: &mut App) {
    let Some(read) = UPDATE_READ.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    app.update = read.update;
    app.update_failed = read.failed;
    publish(app);
    if let (true, Some(available)) = (read.announce, app.update.as_ref().map(|u| u.available.clone())) {
        let body = format!("Seam {available} is ready to install");
        notify(app, "Update ready", &body, BalloonAction::OpenPanel("about"));
        api_server::broadcast("update.ready", &json!({ "version": available }));
    }
    arm_auto_install(app);
}

/// An automatic install is wanted, and the PC may not be idle yet.
pub(super) fn arm_auto_install(app: &App) {
    let wanted = app.cfg.updates.auto_install && !app.packaged && !app.update_failed;
    let ready = app.update.as_ref().filter(|u| u.ready && app.install_started.as_deref() != Some(&u.available));
    if wanted && ready.is_some() {
        unsafe { SetTimer(Some(app.hwnd), TIMER_AUTO_INSTALL, AUTO_INSTALL_RETRY_MS, None) };
    }
}

/// No Line Up, no button held, and no input for 2 minutes.
pub(super) fn is_idle() -> bool {
    if overlay::is_active() {
        return false;
    }
    unsafe {
        let held = |key: VIRTUAL_KEY| GetAsyncKeyState(i32::from(key.0)) < 0;
        if held(VK_LBUTTON) || held(VK_RBUTTON) {
            return false;
        }
        let mut last = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        GetLastInputInfo(&mut last).as_bool() && GetTickCount().wrapping_sub(last.dwTime) >= IDLE_BEFORE_INSTALL_MS
    }
}

/// Install a ready update once the PC is idle. The new tray says it updated.
pub(super) fn auto_install(app: &mut App) {
    let version = app.update.as_ref().filter(|u| u.ready).map(|u| u.available.clone());
    let Some(version) = version.filter(|v| app.install_started.as_ref() != Some(v) && !app.update_failed) else {
        unsafe {
            let _ = KillTimer(Some(app.hwnd), TIMER_AUTO_INSTALL);
        }
        return;
    };
    if !app.cfg.updates.auto_install || !is_idle() {
        return;
    }
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_AUTO_INSTALL);
    }
    log::info!("installing {version} while idle");
    app.install_started = Some(version.clone());
    let _ = std::thread::Builder::new().name("seam-update-install".into()).spawn(move || {
        run_settings(&["--install", &version]);
    });
}

/// Delete what an update left behind. Best effort and silent.
pub(super) fn clean_up_after_update() {
    if let Ok(exe) = std::env::current_exe() {
        for name in ["seam.exe.old", "seam-settings.exe.old"] {
            let _ = std::fs::remove_file(exe.with_file_name(name));
        }
    }
    updates::clear();
}
