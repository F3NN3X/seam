//! Spotlight: dims everything but the window in use. One black, half-clear,
//! click-through window per screen sits directly below the focused window, so
//! that window and anything above it stays bright. The windows live on a thread
//! of their own and never take focus. The `seam-wm` thread tells it when focus
//! moves or a window minimizes or restores, and it also looks again now and then,
//! for a window moved to another screen.
//!
//! The screen the focused window is on dims below it. The other screens dim above
//! everything but topmost windows. With the desktop or the taskbar in front, the
//! dim is hidden, because there is no window to leave bright.

use crate::geom::Rect;
use crate::log;
use crate::monitors::Mon;
use seam::winutil;
use std::sync::mpsc::channel;
use std::sync::Mutex;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("SeamSpotlight");
/// About 50% black.
const ALPHA: u8 = 128;
/// How often the thread looks for a focused window that changed screens.
const FOLLOW_MS: u32 = 250;
/// Focus moved, or a window minimized or restored.
const WM_APP_REORDER: u32 = WM_APP + 1;
/// The screens changed.
const WM_APP_REBUILD: u32 = WM_APP + 2;
/// The shell's windows: nothing there to leave bright.
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// The id of the thread showing the dim, or 0 while spotlight is off.
static THREAD: Mutex<u32> = Mutex::new(0);
/// The screens in pixels, for the thread to build its windows from.
static SCREENS: Mutex<Vec<Rect>> = Mutex::new(Vec::new());

pub fn is_on() -> bool {
    *THREAD.lock().unwrap_or_else(|e| e.into_inner()) != 0
}

/// Turn spotlight on or off. Returns whether it's on now.
pub fn set(on: bool, mons: &[Mon]) -> bool {
    let mut thread = THREAD.lock().unwrap_or_else(|e| e.into_inner());
    *SCREENS.lock().unwrap_or_else(|e| e.into_inner()) = mons.iter().map(|m| m.rect).collect();
    match (on, *thread) {
        (true, 0) => *thread = start(),
        (false, id) if id != 0 => {
            post(id, WM_QUIT);
            *thread = 0;
        }
        _ => {}
    }
    *thread != 0
}

/// Turn spotlight off when Seam quits.
pub fn stop() {
    set(false, &[]);
}

/// The screens may have changed: if they have, the windows are built again to fit them.
pub fn screens_changed(mons: &[Mon]) {
    let now: Vec<Rect> = mons.iter().map(|m| m.rect).collect();
    let mut screens = SCREENS.lock().unwrap_or_else(|e| e.into_inner());
    if *screens == now {
        return;
    }
    *screens = now;
    drop(screens);
    let id = *THREAD.lock().unwrap_or_else(|e| e.into_inner());
    if id != 0 {
        post(id, WM_APP_REBUILD);
    }
}

/// Focus moved, or a window minimized or restored: put the dim windows right again.
pub fn reorder() {
    let id = *THREAD.lock().unwrap_or_else(|e| e.into_inner());
    if id != 0 {
        post(id, WM_APP_REORDER);
    }
}

fn post(thread: u32, message: u32) {
    unsafe {
        let _ = PostThreadMessageW(thread, message, WPARAM(0), LPARAM(0));
    }
}

/// Start the thread and wait for its id. Zero if it couldn't start.
fn start() -> u32 {
    let (ready, wait) = channel();
    let spawned = std::thread::Builder::new().name("seam-spotlight".into()).spawn(move || run(ready));
    match spawned {
        Ok(_) => wait.recv().unwrap_or(0),
        Err(e) => {
            log::info!("spotlight: couldn't start: {e}");
            0
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// One screen's dim window.
struct Dim {
    hwnd: HWND,
    screen: Rect,
}

#[derive(Default)]
struct Dims {
    list: Vec<Dim>,
    /// The focused window and its screen at the last reorder.
    last: Option<(isize, Option<usize>)>,
}

impl Dims {
    /// Destroy the windows and make one for each screen. They stay hidden until the first reorder.
    fn rebuild(&mut self) {
        self.destroy();
        let screens = SCREENS.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.list = screens
            .into_iter()
            .filter_map(|screen| unsafe { open_window(screen) }.map(|hwnd| Dim { hwnd, screen }))
            .collect();
        self.reorder(true);
    }

    fn destroy(&mut self) {
        for dim in self.list.drain(..) {
            unsafe {
                let _ = DestroyWindow(dim.hwnd);
            }
        }
        self.last = None;
    }

    /// The index of the dim window on the focused window's screen, or `None` when
    /// there is no app window to leave bright.
    fn focused_screen(&self, fg: HWND) -> Option<usize> {
        if fg.is_invalid() || !unsafe { IsWindow(Some(fg)) }.as_bool() || !unsafe { IsWindowVisible(fg) }.as_bool() {
            return None;
        }
        if winutil::is_minimized(fg) || SHELL_CLASSES.contains(&winutil::class_name(fg).as_str()) {
            return None;
        }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let monitor = unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return None;
        }
        let r = info.rcMonitor;
        self.list.iter().position(|d| d.screen == Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
    }

    /// Put each dim window where it belongs. Unless `force`, only when the focused
    /// window or its screen changed.
    fn reorder(&mut self, force: bool) {
        let fg = unsafe { GetForegroundWindow() };
        let on = self.focused_screen(fg);
        let now = (fg.0 as isize, on);
        if !force && self.last == Some(now) {
            return;
        }
        self.last = Some(now);
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        for (i, dim) in self.list.iter().enumerate() {
            unsafe {
                match on {
                    // Directly below the focused window, which a window above it stays bright over.
                    Some(s) if s == i => {
                        let _ = SetWindowPos(dim.hwnd, Some(fg), 0, 0, 0, 0, flags | SWP_SHOWWINDOW);
                    }
                    // Above everything except topmost windows.
                    Some(_) => {
                        let _ = SetWindowPos(dim.hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags | SWP_SHOWWINDOW);
                    }
                    None => {
                        let _ = SetWindowPos(dim.hwnd, None, 0, 0, 0, 0, flags | SWP_NOZORDER | SWP_HIDEWINDOW);
                    }
                }
            }
        }
    }
}

/// Create one click-through window over a screen. It never takes focus.
unsafe fn open_window(screen: Rect) -> Option<HWND> {
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        CLASS,
        w!("Seam spotlight"),
        WS_POPUP,
        screen.left,
        screen.top,
        1,
        1,
        None,
        None,
        Some(hinst.into()),
        None,
    )
    .ok()?;
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), ALPHA, LWA_ALPHA);
    // Sized after creation, because the window now knows its screen's DPI.
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
    let _ = SetWindowPos(hwnd, None, screen.left, screen.top, screen.width(), screen.height(), flags);
    Some(hwnd)
}

fn run(ready: std::sync::mpsc::Sender<u32>) {
    unsafe {
        let mut msg = MSG::default();
        // Creates this thread's queue, so posting to it can't fail once `ready` is sent.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let _ = ready.send(GetCurrentThreadId());
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let mut dims = Dims::default();
        dims.rebuild();
        let timer = SetTimer(None, 0, FOLLOW_MS, None);
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                WM_APP_REORDER => dims.reorder(true),
                WM_APP_REBUILD => dims.rebuild(),
                WM_TIMER if msg.hwnd.is_invalid() => dims.reorder(false),
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        let _ = KillTimer(None, timer);
        dims.destroy();
    }
}
