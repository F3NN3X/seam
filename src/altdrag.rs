//! Move and resize a window from anywhere in it, as WinScreen and AltSnap do: hold
//! the key and drag with the left button to move, or with the right to resize.
//! A low-level mouse hook on a thread of its own (the D204 pattern), in only while
//! the setting is on and Seam isn't standing by. The hook decides a press with a
//! few checks, swallows it and its release, and only posts. A worker thread does
//! the moving. The tray hears when a drag starts and ends, so the snapping treats
//! it as a normal drag: zones with the show key, neighbours following an edge, a
//! window moved away leaving its group.

use crate::hook::SEAM_INPUT_TAG;
use crate::{log, minsize, wincontrol};
use seam::winutil;
use seam::zones::Rect;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A drag started (wparam 1) or ended (0), with the window in lparam. The start is
/// sent, not posted, so the snapping sees the window before it moves.
pub const WM_APP_ALT_DRAG: u32 = WM_APP + 34;
/// How long the worker waits for the tray to hear of a start.
const TELL_MS: u32 = 500;

/// The worker's messages: a move or resize drag from the point in lparam, the
/// pointer moved, the button came up. The window is in wparam.
const MSG_MOVE: u32 = WM_APP + 1;
const MSG_RESIZE: u32 = WM_APP + 2;
const MSG_MOVED: u32 = WM_APP + 3;
const MSG_END: u32 = WM_APP + 4;
/// Wakes the hook's thread so it looks whether it is still wanted.
const MSG_WAKE: u32 = WM_APP + 5;

/// The key held for a drag.
const KEY_ALT: u8 = 0;
const KEY_WIN: u8 = 1;
const KEY_CTRL_ALT: u8 = 2;

static KEY: AtomicU8 = AtomicU8::new(KEY_ALT);
/// Exe names the zones leave alone, lower case.
static EXCLUDE: RwLock<Vec<String>> = RwLock::new(Vec::new());
/// The tray window, told of each drag.
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// The setting is on and Seam isn't standing by.
static ON: AtomicBool = AtomicBool::new(false);
/// Counts starts and stops: a hook whose generation is gone takes no new drag.
static WANTED: AtomicU64 = AtomicU64::new(0);
/// The newest hook's thread, or 0.
static OPEN: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Left,
    Right,
}

/// What the hook on this thread holds. The callback runs on the thread that put it in.
#[derive(Default)]
struct Hooked {
    generation: u64,
    worker: u32,
    /// A move was posted and the worker hasn't taken it yet, so the next isn't.
    pending: Arc<AtomicBool>,
    /// The button whose press was swallowed, so its release is too.
    held: Option<Button>,
}

thread_local! {
    static HOOKED: RefCell<Hooked> = RefCell::new(Hooked::default());
}

/// Put the hook in or take it out, and take the key and the excluded apps.
/// `on` is the setting, with Seam not standing by. Cheap when nothing changed.
pub fn configure(owner: isize, on: bool, key: &str, exclude: &[String]) {
    OWNER.store(owner, Ordering::SeqCst);
    KEY.store(
        match key {
            "win" => KEY_WIN,
            "ctrl_alt" => KEY_CTRL_ALT,
            _ => KEY_ALT,
        },
        Ordering::SeqCst,
    );
    *EXCLUDE.write().unwrap_or_else(|e| e.into_inner()) = exclude.iter().map(|x| x.to_lowercase()).collect();
    if ON.swap(on, Ordering::SeqCst) == on {
        return;
    }
    if on {
        let generation = WANTED.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(e) = std::thread::Builder::new().name("seam-altdrag".into()).spawn(move || run(generation)) {
            log::info!("alt-drag: couldn't start: {e}");
        }
    } else {
        stop();
    }
}

/// The setting went off, Seam stands by or the tray is quitting: no new drag. A
/// drag still going keeps the hook until its release comes, so no app gets a lone one.
pub fn stop() {
    ON.store(false, Ordering::SeqCst);
    WANTED.fetch_add(1, Ordering::SeqCst);
    let open = OPEN.swap(0, Ordering::SeqCst);
    if open != 0 {
        unsafe {
            let _ = PostThreadMessageW(open, MSG_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

fn run(generation: u64) {
    unsafe {
        let id = GetCurrentThreadId();
        let mut msg = MSG::default();
        // Creates this thread's queue, so a stop from now on reaches it.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        OPEN.store(id, Ordering::SeqCst);
        let pending = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let shared = pending.clone();
        let spawned = std::thread::Builder::new().name("seam-altdrag-worker".into()).spawn(move || work(shared, tx));
        let worker = match (spawned, rx.recv()) {
            (Ok(_), Ok(worker)) => worker,
            _ => {
                log::info!("alt-drag: couldn't start the worker");
                let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
                return;
            }
        };
        HOOKED.with(|h| *h.borrow_mut() = Hooked { generation, worker, pending, held: None });
        if WANTED.load(Ordering::SeqCst) == generation {
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
                Ok(hook) => {
                    log::info!("alt-drag: hook installed");
                    while (WANTED.load(Ordering::SeqCst) == generation || owes())
                        && GetMessageW(&mut msg, None, 0, 0).as_bool()
                    {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                    log::info!("alt-drag: hook uninstalled");
                }
                Err(e) => log::info!("alt-drag: no mouse hook: {e}"),
            }
        }
        let _ = PostThreadMessageW(worker, WM_QUIT, WPARAM(0), LPARAM(0));
        let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

fn owes() -> bool {
    HOOKED.with(|h| h.try_borrow().map_or(true, |h| h.held.is_some()))
}

/// Swallow a press with the key held over a window Seam may move, and its release.
/// Moves always pass, and during a drag only tell the worker the pointer moved.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let swallow = info.dwExtraInfo != SEAM_INPUT_TAG
                && HOOKED.with(|h| h.try_borrow_mut().is_ok_and(|mut h| h.on_mouse(wparam.0 as u32, info.pt)));
            if swallow {
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

impl Hooked {
    /// Whether to swallow this mouse message.
    unsafe fn on_mouse(&mut self, msg: u32, pt: POINT) -> bool {
        unsafe {
            let worker = self.worker;
            let post = |message: u32, wparam: usize, lparam: isize| {
                let _ = PostThreadMessageW(worker, message, WPARAM(wparam), LPARAM(lparam));
            };
            let button = match msg {
                WM_MOUSEMOVE => {
                    if self.held.is_some() && !self.pending.swap(true, Ordering::Relaxed) {
                        post(MSG_MOVED, 0, 0);
                    }
                    return false;
                }
                WM_LBUTTONDOWN | WM_LBUTTONUP => Button::Left,
                WM_RBUTTONDOWN | WM_RBUTTONUP => Button::Right,
                _ => return false,
            };
            if matches!(msg, WM_LBUTTONUP | WM_RBUTTONUP) {
                if self.held != Some(button) {
                    return false;
                }
                self.held = None;
                post(MSG_END, 0, 0);
                // Stopped while this drag went on: the release was the last thing owed.
                if WANTED.load(Ordering::Relaxed) != self.generation {
                    PostQuitMessage(0);
                }
                return true;
            }
            // A second press of the drag's button: its release never came (a secure
            // desktop took it), so that drag is over and this press is a new one.
            if self.held == Some(button) {
                self.held = None;
                post(MSG_END, 0, 0);
            }
            if self.held.is_some() || WANTED.load(Ordering::Relaxed) != self.generation || !key_held() {
                return false;
            }
            let Some(window) = target(pt, button) else { return false };
            self.held = Some(button);
            let message = if button == Button::Left { MSG_MOVE } else { MSG_RESIZE };
            post(message, window.0 as usize, pack(pt));
            true
        }
    }
}

/// The chosen key is held, and no other modifier.
fn key_held() -> bool {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0;
    let (ctrl, alt, shift, win) = (down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT), down(VK_LWIN) || down(VK_RWIN));
    match KEY.load(Ordering::Relaxed) {
        KEY_WIN => win && !ctrl && !alt && !shift,
        KEY_CTRL_ALT => ctrl && alt && !shift && !win,
        _ => alt && !ctrl && !shift && !win,
    }
}

/// The top-level window under the point, when Seam may move it: not Seam's own, an
/// administrator's, a full-screen app's, a hung or an excluded app's. A resize also
/// needs a sizing border, and a maximized window isn't resized.
unsafe fn target(pt: POINT, button: Button) -> Option<HWND> {
    unsafe {
        let under = WindowFromPoint(pt);
        if under.is_invalid() {
            return None;
        }
        let window = GetAncestor(under, GA_ROOT);
        if button == Button::Right && (!winutil::is_resizable(window) || winutil::is_maximized(window)) {
            return None;
        }
        let exclude = EXCLUDE.try_read().ok()?;
        wincontrol::movable(window, &exclude).ok().map(|_| window)
    }
}

/// A point in an lparam, each half a signed 16-bit coordinate.
fn pack(pt: POINT) -> isize {
    ((pt.x as u16 as u32) | ((pt.y as u16 as u32) << 16)) as isize
}

fn unpack(l: isize) -> POINT {
    POINT { x: (l as u32 & 0xFFFF) as u16 as i16 as i32, y: ((l as u32) >> 16) as u16 as i16 as i32 }
}

/// Which edges a resize moves, from where in the window it was pressed: -1 the left or
/// top, 1 the right or bottom, 0 neither. The window is cut in thirds each way: a
/// corner moves both, the middle of a side one, and the centre the bottom-right corner.
fn sides_at(r: Rect, (x, y): (i32, i32)) -> (i32, i32) {
    let third = |at: i32, start: i32, len: i32| {
        if at < start + len / 3 {
            -1
        } else if at >= start + len - len / 3 {
            1
        } else {
            0
        }
    };
    match (third(x, r.x, r.w), third(y, r.y, r.h)) {
        (0, 0) => (1, 1),
        sides => sides,
    }
}

/// One drag on the worker.
struct Drag {
    hwnd: HWND,
    started: Instant,
    /// The window rect when the drag began, or after a maximized one was restored.
    start: Rect,
    /// Where the button went down.
    from: POINT,
    /// Edges a resize moves, None for a move.
    sides: Option<(i32, i32)>,
    /// The smallest window rect a resize may make.
    min: (i32, i32),
}

const FLAGS: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_ASYNCWINDOWPOS.0);

impl Drag {
    fn begin(hwnd: HWND, from: POINT, resize: bool) -> Option<Drag> {
        let started = Instant::now();
        let mut start = winutil::window_rect(hwnd)?;
        tell(hwnd, true);
        tap();
        let mut sides = None;
        let mut min = (0, 0);
        if resize {
            sides = Some(sides_at(start, (from.x, from.y)));
            let visible = winutil::visible_rect(hwnd).unwrap_or(start);
            let (w, h) = minsize::of(hwnd, winutil::monitor_dpi(hwnd));
            min = (w + start.w - visible.w, h + start.h - visible.h);
        } else if winutil::is_maximized(hwnd) {
            // As dragging a maximized title bar does: its normal size, centred under the pointer.
            let dpi = winutil::monitor_dpi(hwnd);
            if let Some(normal) = winutil::normal_frame(hwnd, dpi) {
                let frame = Rect { x: from.x - normal.w / 2, y: from.y - normal.h / 2, w: normal.w, h: normal.h };
                winutil::place_restored(hwnd, frame, dpi);
                start = winutil::window_rect(hwnd).unwrap_or(start);
            }
        }
        Some(Drag { hwnd, started, start, from, sides, min })
    }

    /// Put the window where the pointer says. The last step waits for the app, so
    /// the snapping reads where the drag really ended.
    fn follow(&self, last: bool) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let (dx, dy) = (pt.x - self.from.x, pt.y - self.from.y);
        let s = self.start;
        let flags = if last { FLAGS & !SWP_ASYNCWINDOWPOS } else { FLAGS };
        let (x, y, w, h, flags) = match self.sides {
            None => (s.x + dx, s.y + dy, 0, 0, flags | SWP_NOSIZE),
            Some((sx, sy)) => {
                let (x, w) = edge(s.x, s.w, sx, dx, self.min.0);
                let (y, h) = edge(s.y, s.h, sy, dy, self.min.1);
                (x, y, w, h, flags)
            }
        };
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, flags);
        }
    }

    fn end(self) {
        self.follow(true);
        tell(self.hwnd, false);
        let what = if self.sides.is_some() { "resize" } else { "move" };
        log::info!("alt-drag: {what}, {} ms", self.started.elapsed().as_millis());
    }
}

/// One axis of a resize: the start and length after the pointer moved `d`, keeping
/// at least `min`. Moving the left or top edge leaves the far one where it was.
fn edge(start: i32, len: i32, side: i32, d: i32, min: i32) -> (i32, i32) {
    match side {
        -1 => {
            let to = (len - d).max(min);
            (start + len - to, to)
        }
        1 => (start, (len + d).max(min)),
        _ => (start, len),
    }
}

/// Tell the tray a drag started or ended, so the snapping follows it.
fn tell(hwnd: HWND, started: bool) {
    let owner = HWND(OWNER.load(Ordering::SeqCst) as *mut _);
    let (wparam, lparam) = (WPARAM(usize::from(started)), LPARAM(hwnd.0 as isize));
    unsafe {
        if started {
            let _ = SendMessageTimeoutW(owner, WM_APP_ALT_DRAG, wparam, lparam, SMTO_ABORTIFHUNG, TELL_MS, None);
        } else {
            let _ = PostMessageW(Some(owner), WM_APP_ALT_DRAG, wparam, lparam);
        }
    }
}

/// A key no app uses, pressed and released while the drag's key is held, so letting
/// go of Alt opens no menu bar and letting go of Win opens no Start (as AltSnap does).
fn tap() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0xFF),
                dwFlags: flags,
                dwExtraInfo: SEAM_INPUT_TAG,
                ..Default::default()
            },
        },
    };
    unsafe {
        SendInput(&[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)], std::mem::size_of::<INPUT>() as i32);
    }
}

/// The worker: one drag at a time, from the hook's messages.
fn work(pending: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<u32>) {
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        if ready.send(GetCurrentThreadId()).is_err() {
            return;
        }
        let mut drag: Option<Drag> = None;
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                MSG_MOVE | MSG_RESIZE => {
                    if let Some(d) = drag.take() {
                        d.end();
                    }
                    let hwnd = HWND(msg.wParam.0 as *mut _);
                    drag = Drag::begin(hwnd, unpack(msg.lParam.0), msg.message == MSG_RESIZE);
                }
                MSG_MOVED => {
                    pending.store(false, Ordering::Relaxed);
                    if let Some(d) = &drag {
                        d.follow(false);
                    }
                }
                MSG_END => {
                    if let Some(d) = drag.take() {
                        d.end();
                    }
                }
                _ => {}
            }
        }
        if let Some(d) = drag.take() {
            d.end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resize_takes_the_edges_nearest_the_press() {
        let r = Rect { x: 100, y: 100, w: 300, h: 600 };
        // Corners move both edges.
        assert_eq!(sides_at(r, (110, 110)), (-1, -1));
        assert_eq!(sides_at(r, (390, 690)), (1, 1));
        assert_eq!(sides_at(r, (390, 110)), (1, -1));
        // The middle of a side moves that side only.
        assert_eq!(sides_at(r, (110, 400)), (-1, 0));
        assert_eq!(sides_at(r, (250, 690)), (0, 1));
        // The centre is the bottom-right corner.
        assert_eq!(sides_at(r, (250, 400)), (1, 1));
        // Thirds: 100 wide each.
        assert_eq!(sides_at(r, (199, 400)), (-1, 0));
        assert_eq!(sides_at(r, (200, 400)), (1, 1));
        assert_eq!(sides_at(r, (300, 400)), (1, 0));
    }
}
