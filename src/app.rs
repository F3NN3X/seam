//! Tray app: owns config, tracks monitor changes, runs the hook and the overlay.

use crate::config::{signature, Config, SavedBorder};
use crate::geom::touching;
use crate::hook::{self, Link, Live};
use crate::monitors::{self, Mon};
use crate::{arrange, autostart, overlay};
use std::cell::RefCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("SeamTray");
const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_LINEUP: u32 = WM_APP + 3;
const TIMER_REFRESH: usize = 1;
/// Windows silently removes low-level hooks that ever time out, so re-arm periodically.
const TIMER_REARM: usize = 2;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const ID_LINEUP: usize = 1;
const ID_ENABLED: usize = 2;
const ID_ALIGN: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_FOLDER: usize = 5;
const ID_QUIT: usize = 9;

struct App {
    hwnd: HWND,
    icon: HICON,
    cfg: Config,
    mons: Vec<Mon>,
    sig: String,
    links: Vec<Link>,
    taskbar_created: u32,
    /// Only the "new screen setup" balloon should open Line Up when clicked.
    balloon_opens_lineup: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        out[i] = c;
    }
    out
}

/// Resolve saved borders against the monitors that are connected right now.
/// A border only applies if the two screens still touch on the same side.
fn resolve(mons: &[Mon], saved: Option<&Vec<SavedBorder>>) -> Vec<Link> {
    let Some(saved) = saved else { return Vec::new() };
    saved
        .iter()
        .filter_map(|s| {
            let a = mons.iter().position(|m| m.id == s.a)?;
            let b = mons.iter().position(|m| m.id == s.b)?;
            (touching(&mons[a].rect, &mons[b].rect) == Some(s.side)).then_some(Link { a, b, side: s.side, map: s.map })
        })
        .collect()
}

fn has_neighbours(mons: &[Mon]) -> bool {
    (0..mons.len()).any(|i| (i + 1..mons.len()).any(|j| touching(&mons[i].rect, &mons[j].rect).is_some()))
}

/// Draw the tray icon: two screens of different sizes with a line running straight across.
fn make_icon() -> HICON {
    const N: usize = 32;
    let mut px = vec![0u32; N * N];
    let mut rect = |x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                px[y * N + x] = c;
            }
        }
    };
    let white = 0xFFF2F4F8;
    let cyan = 0xFF00C8FF;
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
    rect(3, 16, 28, 17, cyan);
    unsafe {
        let color = CreateBitmap(N as i32, N as i32, 1, 32, Some(px.as_ptr() as *const _));
        let mask_bits = vec![0u8; N * N / 8];
        let mask = CreateBitmap(N as i32, N as i32, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn tray_data(app: &App) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: app.hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: app.icon,
        ..Default::default()
    };
    let tip = if app.links.is_empty() {
        "Seam — click to line up your screens".to_string()
    } else if !app.cfg.enabled {
        "Seam — paused".to_string()
    } else {
        format!("Seam — {} border(s) lined up", app.links.len())
    };
    nid.szTip = wide(&tip);
    nid
}

fn add_icon(app: &App) {
    unsafe {
        let mut nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

fn update_tip(app: &App) {
    unsafe {
        let nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Show a balloon/toast. Returns whether the shell accepted it.
fn notify(app: &mut App, title: &str, body: &str, opens_lineup: bool) -> bool {
    app.balloon_opens_lineup = opens_lineup;
    unsafe {
        let mut nid = tray_data(app);
        nid.uFlags |= NIF_INFO;
        nid.szInfoTitle = wide(title);
        nid.szInfo = wide(body);
        nid.dwInfoFlags = NIIF_INFO;
        Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
    }
}

/// (Re)install the hook if it should be running. Unhooking first guarantees a
/// hook Windows dropped behind our back comes back.
fn arm_hook(app: &App) {
    hook::enable(false);
    hook::enable(app.cfg.enabled && !app.links.is_empty());
}

/// Re-read monitors, resolve the saved layout, (re)arm the hook.
fn refresh(app: &mut App) {
    if overlay::is_active() {
        return;
    }
    app.mons = monitors::enumerate();
    app.sig = signature(app.mons.iter().map(|m| m.id.as_str()));
    app.links = resolve(&app.mons, app.cfg.layouts.get(&app.sig));
    hook::set_live(Live { rects: app.mons.iter().map(|m| m.rect).collect(), links: app.links.clone() });
    arm_hook(app);
    update_tip(app);
    // New set of screens that's never been lined up: ask once. Only mark it
    // seen if the shell actually showed the prompt (it may not be up at login).
    if app.links.is_empty() && has_neighbours(&app.mons) && !app.cfg.seen.contains(&app.sig) {
        if notify(app, "New screen setup", "Click here to line up your screens.", true) {
            app.cfg.seen.push(app.sig.clone());
            app.cfg.save();
        }
    }
}

fn start_lineup(app: &mut App) {
    let mons = monitors::enumerate();
    let sig = signature(mons.iter().map(|m| m.id.as_str()));
    let saved = app.cfg.layouts.get(&sig).cloned().unwrap_or_default();
    if !overlay::start(app.hwnd, mons, &saved) {
        notify(
            app,
            "Nothing to line up",
            "Seam needs two or more screens that touch in the Windows arrangement.",
            false,
        );
    }
}

fn lineup_done(app: &mut App, accepted: bool) {
    if let (true, Some(res)) = (accepted, overlay::take_result()) {
        let mons = monitors::enumerate();
        let sig = signature(mons.iter().map(|m| m.id.as_str()));
        let links = resolve(&mons, Some(&res));
        app.cfg.layouts.insert(sig.clone(), res);
        if !app.cfg.seen.contains(&sig) {
            app.cfg.seen.push(sig);
        }
        app.cfg.enabled = true;
        app.cfg.save();
        // A mode set can stall input for seconds; never let it run under the
        // hook, or Windows may silently remove it.
        hook::enable(false);
        let msg = if app.cfg.align_arrangement {
            match arrange::align(&mons, &links) {
                Ok(_) => "The pointer now crosses where your screens meet.".to_string(),
                Err(e) => format!("Pointer remap is on. Windows arrangement unchanged: {e}."),
            }
        } else {
            "The pointer now crosses where your screens meet.".to_string()
        };
        refresh(app);
        notify(app, "Screens lined up", &msg, false);
    } else {
        refresh(app);
    }
}

fn show_menu(app: &mut App) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let flag = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
        let status = match (app.mons.len(), app.links.len()) {
            (n, 0) => format!("{n} screen(s), not lined up yet"),
            (n, k) => format!("{n} screens, {k} border(s) lined up"),
        };
        let status_w: Vec<u16> = status.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status_w.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_LINEUP, w!("Line Up Screens…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, flag(app.cfg.enabled), ID_ENABLED, w!("Remap pointer at borders"));
        let _ = AppendMenuW(menu, flag(app.cfg.align_arrangement), ID_ALIGN, w!("Also align Windows arrangement"));
        let _ = AppendMenuW(menu, flag(autostart::is_enabled()), ID_AUTOSTART, w!("Launch at login"));
        let _ = AppendMenuW(menu, MF_STRING, ID_FOLDER, w!("Open settings folder"));
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
            ID_ENABLED => {
                app.cfg.enabled = !app.cfg.enabled;
                app.cfg.save();
                refresh(app);
            }
            ID_ALIGN => {
                app.cfg.align_arrangement = !app.cfg.align_arrangement;
                app.cfg.save();
            }
            ID_AUTOSTART => {
                autostart::set(!autostart::is_enabled());
            }
            ID_FOLDER => {
                if let Some(dir) = Config::path().parent() {
                    let _ = std::fs::create_dir_all(dir);
                    let d: Vec<u16> = dir.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
                    ShellExecuteW(None, w!("open"), PCWSTR(d.as_ptr()), None, None, SW_SHOWNORMAL);
                }
            }
            ID_QUIT => {
                // Close after this borrow ends so WM_DESTROY can clean up the icon.
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xffff) as u32;
            match event {
                WM_CONTEXTMENU | WM_RBUTTONUP => {
                    // The menu runs a nested loop; take the app out of the cell
                    // only through with_app, and do it after this message returns.
                    let _ = PostMessageW(Some(hwnd), WM_APP + 10, WPARAM(0), LPARAM(0));
                }
                NIN_SELECT | NIN_KEYSELECT => {
                    let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
                }
                NIN_BALLOONUSERCLICK => {
                    if with_app(|a| std::mem::take(&mut a.balloon_opens_lineup)).unwrap_or(false) {
                        let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        m if m == WM_APP + 10 => {
            with_app(show_menu);
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
            // Screens changed under us: a half-finished line-up no longer applies.
            if overlay::is_active() {
                overlay::cancel();
            }
            SetTimer(Some(hwnd), TIMER_REFRESH, 800, None);
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
                SetTimer(Some(hwnd), TIMER_REFRESH, 1500, None);
            }
            LRESULT(1)
        }
        WM_TIMER if wparam.0 == TIMER_REARM => {
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
            hook::enable(false);
            with_app(|a| {
                let nid = tray_data(a);
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            });
            PostQuitMessage(0);
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

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // One instance only. Launching again opens Line Up in the running one.
        let _mutex = CreateMutexW(None, true, w!("Local\\Seam.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(other) = FindWindowW(CLASS, None) {
                // Hand over foreground rights so the overlay gets the keyboard.
                let mut pid = 0u32;
                GetWindowThreadProcessId(other, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(other), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
            }
            return;
        }

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
            w!("Seam"),
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
            icon: make_icon(),
            cfg: Config::load(),
            mons: Vec::new(),
            sig: String::new(),
            links: Vec::new(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            balloon_opens_lineup: false,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            add_icon(a);
            refresh(a);
        });
        SetTimer(Some(hwnd), TIMER_REARM, 60_000, None);
        if std::env::args().any(|a| a == "--line-up") {
            let _ = PostMessageW(Some(hwnd), WM_APP_LINEUP, WPARAM(0), LPARAM(0));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
