//! Move and resize a window from anywhere in it, as WinScreen and AltSnap do: hold
//! the key and drag with the left button to move, or with the right to resize.
//! A low-level mouse hook on a thread of its own (the D204 pattern), in only while
//! the setting is on and Seam isn't standing by. The hook decides a press with a
//! few cheap checks and what the worker found about the app before, swallows it and
//! its release, and only posts. A worker thread asks the slow questions and does
//! the moving. The tray hears when a drag starts and ends, so the snapping treats
//! it as a normal drag: zones with the show key, neighbours following an edge, a
//! window moved away leaving its group.

use crate::hook::SEAM_INPUT_TAG;
use crate::{log, minsize, wincontrol};
use seam::winutil;
use seam::zones::Rect;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
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
/// Look at the app of the window in wparam, for the next press on it.
const MSG_LOOK: u32 = WM_APP + 6;
/// The hook's thread looks whether Windows took the hook out.
const MSG_CHECK: u32 = WM_APP + 7;
/// How often the hook's thread looks, while it is in.
const CHECK_MS: u32 = 60_000;
/// Look at the apps of every window on screen, ahead of a press on one.
const MSG_SWEEP: u32 = WM_APP + 8;
/// How often the worker looks at every window's app, while the hook is in.
const SWEEP_MS: u32 = 60_000;
/// Processes remembered before the oldest goes.
const MAX_VERDICTS: usize = 512;
/// The longest quitting waits for the release of a drag still going.
const QUIT_WAIT: Duration = Duration::from_secs(2);
/// How long the app has to say how big it may get.
const ASK_MS: u32 = 100;

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
/// The newest hook's worker, or 0.
static WORKER: AtomicU32 = AtomicU32::new(0);
/// A hook owes a release, so quitting waits a moment for it.
static OWING: AtomicBool = AtomicBool::new(false);
/// By process id and creation time, oldest first: whether Seam may move its windows
/// (not Seam's, not an administrator's, not excluded). Neither changes in a process's
/// life, so nothing expires: an entry goes when its process does. The worker fills it
/// ahead of use, and the hook only tries the lock.
static VERDICTS: Mutex<Vec<Verdict>> = Mutex::new(Vec::new());
/// Counts changes of the exclude list, so a look begun before one isn't kept.
static EXCLUDED_AT: AtomicU64 = AtomicU64::new(0);

struct Verdict {
    pid: u32,
    created: u64,
    ok: bool,
}

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
    /// The other button, pressed during the drag: its press and release are the drag's too.
    other: Option<Button>,
}

thread_local! {
    static HOOKED: RefCell<Hooked> = RefCell::new(Hooked::default());
    /// Calls of the hook on this thread, to tell whether Windows took it out.
    static CALLS: Cell<u64> = const { Cell::new(0) };
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
    let exclude: Vec<String> = exclude.iter().map(|x| x.to_lowercase()).collect();
    let mut kept = EXCLUDE.write().unwrap_or_else(|e| e.into_inner());
    if *kept != exclude {
        *kept = exclude;
        EXCLUDED_AT.fetch_add(1, Ordering::SeqCst);
        VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    drop(kept);
    if ON.swap(on, Ordering::SeqCst) == on {
        // Still on: the hook's thread looks whether Windows took the hook out.
        let open = OPEN.load(Ordering::SeqCst);
        if on && open != 0 {
            unsafe {
                let _ = PostThreadMessageW(open, MSG_CHECK, WPARAM(0), LPARAM(0));
            }
        }
        return;
    }
    if on {
        let generation = WANTED.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(e) = std::thread::Builder::new().name("seam-altdrag".into()).spawn(move || run(generation)) {
            log::info!("alt-drag: couldn't start: {e}");
            // The next change of settings tries again.
            ON.store(false, Ordering::SeqCst);
        }
    } else {
        stop();
    }
}

/// The setting went off or Seam stands by: no new drag. A drag still going keeps
/// the hook until its release comes, so no app gets a lone one.
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

/// The tray is quitting. The worker ends a drag still going, and this keeps the hook
/// until the button is released so the hook can swallow it, for at most 2 s. A release
/// after that reaches the app, as the hook goes with the process.
pub fn shutdown() {
    stop();
    if !OWING.load(Ordering::SeqCst) {
        return;
    }
    let worker = WORKER.load(Ordering::SeqCst);
    unsafe {
        let _ = PostThreadMessageW(worker, MSG_END, WPARAM(0), LPARAM(0));
    }
    let until = Instant::now() + QUIT_WAIT;
    while OWING.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The hook couldn't go in, or was lost: the next change of settings tries again.
fn failed(generation: u64) {
    if WANTED.load(Ordering::SeqCst) == generation {
        ON.store(false, Ordering::SeqCst);
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
                failed(generation);
                let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
                return;
            }
        };
        WORKER.store(worker, Ordering::SeqCst);
        HOOKED.with(|h| *h.borrow_mut() = Hooked { generation, worker, pending, held: None, other: None });
        let install = || SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
        if WANTED.load(Ordering::SeqCst) == generation {
            match install() {
                Ok(mut hook) => {
                    log::info!("alt-drag: hook installed");
                    SetTimer(None, 0, CHECK_MS, None);
                    // The apps are looked at ahead of a press: those on screen now, then
                    // each that comes to the foreground.
                    let _ = PostThreadMessageW(worker, MSG_SWEEP, WPARAM(0), LPARAM(0));
                    let foreground = EVENT_SYSTEM_FOREGROUND;
                    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                    let watch = SetWinEventHook(foreground, foreground, None, Some(on_foreground), 0, 0, flags);
                    let mut seen = (CALLS.get(), cursor());
                    while (WANTED.load(Ordering::SeqCst) == generation || owes())
                        && GetMessageW(&mut msg, None, 0, 0).as_bool()
                    {
                        if msg.message == MSG_CHECK || (msg.message == WM_TIMER && msg.hwnd.is_invalid()) {
                            // Windows takes out a hook that was too slow, and says nothing. The
                            // pointer moved since the last look, and the hook heard nothing of it.
                            let now = (CALLS.get(), cursor());
                            if now.0 == seen.0 && now.1 != seen.1 {
                                let _ = UnhookWindowsHookEx(hook);
                                match install() {
                                    Ok(again) => {
                                        hook = again;
                                        log::info!("alt-drag: hook re-armed, Windows had taken it out");
                                    }
                                    Err(e) => {
                                        log::info!("alt-drag: hook lost and not put back: {e}");
                                        failed(generation);
                                        break;
                                    }
                                }
                            }
                            seen = (CALLS.get(), cursor());
                            continue;
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                    if !watch.0.is_null() {
                        let _ = UnhookWinEvent(watch);
                    }
                    log::info!("alt-drag: hook uninstalled");
                }
                Err(e) => {
                    log::info!("alt-drag: no mouse hook: {e}");
                    failed(generation);
                }
            }
        }
        OWING.store(false, Ordering::SeqCst);
        let _ = PostThreadMessageW(worker, WM_QUIT, WPARAM(0), LPARAM(0));
        let _ = OPEN.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

fn owes() -> bool {
    HOOKED.with(|h| h.try_borrow().map_or(true, |h| h.owes()))
}

fn cursor() -> POINT {
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    pt
}

/// Swallow a press with the key held over a window Seam may move, and its release.
/// Moves always pass, and during a drag only tell the worker the pointer moved.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        CALLS.set(CALLS.get() + 1);
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
    fn owes(&self) -> bool {
        self.held.is_some() || self.other.is_some()
    }

    /// A swallowed release came, or a press whose release never will. A hook that was
    /// stopped during the drag goes once it owes nothing.
    unsafe fn settle(&self) {
        let owes = self.owes();
        OWING.store(owes, Ordering::Relaxed);
        if !owes && WANTED.load(Ordering::Relaxed) != self.generation {
            unsafe { PostQuitMessage(0) };
        }
    }

    /// Whether to swallow this mouse message.
    unsafe fn on_mouse(&mut self, msg: u32, pt: POINT) -> bool {
        unsafe {
            let worker = self.worker;
            let post = |message: u32, wparam: usize, lparam: isize| {
                PostThreadMessageW(worker, message, WPARAM(wparam), LPARAM(lparam)).is_ok()
            };
            let button = match msg {
                WM_MOUSEMOVE => {
                    // A move the worker never gets mustn't hold back the next.
                    if self.held.is_some() && !self.pending.swap(true, Ordering::Relaxed) && !post(MSG_MOVED, 0, 0) {
                        self.pending.store(false, Ordering::Relaxed);
                    }
                    return false;
                }
                WM_LBUTTONDOWN | WM_LBUTTONUP => Button::Left,
                WM_RBUTTONDOWN | WM_RBUTTONUP => Button::Right,
                _ => return false,
            };
            if matches!(msg, WM_LBUTTONUP | WM_RBUTTONUP) {
                if self.other == Some(button) {
                    self.other = None;
                } else if self.held == Some(button) {
                    self.held = None;
                    post(MSG_END, 0, 0);
                } else {
                    return false;
                }
                self.settle();
                return true;
            }
            // A second press of a button whose press was swallowed: its release never
            // came (a secure desktop took it), so that is over and this press is new.
            if self.other == Some(button) {
                self.other = None;
            }
            if self.held == Some(button) {
                self.held = None;
                post(MSG_END, 0, 0);
            }
            // The other button during a drag is the drag's, so no stray click reaches the app.
            if self.held.is_some() {
                self.other = Some(button);
                return true;
            }
            self.settle();
            if WANTED.load(Ordering::Relaxed) != self.generation || !key_held() {
                return false;
            }
            let Some(window) = target(pt, button, |h| post(MSG_LOOK, h.0 as usize, 0)) else { return false };
            self.held = Some(button);
            OWING.store(true, Ordering::Relaxed);
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
        // AltGr is Ctrl+Alt to Windows, and types characters on many layouts.
        KEY_CTRL_ALT => ctrl && alt && !shift && !win && !down(VK_RMENU),
        _ => alt && !ctrl && !shift && !win,
    }
}

/// The top-level window under the point, when Seam may move it, from cheap checks
/// only: a real window, not Seam's, not full screen. A resize also needs a sizing
/// border, and a maximized window isn't resized. Whether its app is Seam's, an
/// administrator's or excluded is what the worker found before: an app it hasn't
/// looked at yet gets the press, and `look` has the worker find out for next time.
unsafe fn target(pt: POINT, button: Button, look: impl Fn(HWND) -> bool) -> Option<HWND> {
    unsafe {
        let under = WindowFromPoint(pt);
        if under.is_invalid() {
            return None;
        }
        let window = GetAncestor(under, GA_ROOT);
        if button == Button::Right && (!winutil::is_resizable(window) || winutil::is_maximized(window)) {
            return None;
        }
        // With no exclude list it opens no process.
        if !winutil::is_snappable(window, std::process::id(), &[]) {
            return None;
        }
        let pid = winutil::process_id(window);
        let known = VERDICTS.try_lock().ok().and_then(|list| list.iter().find(|v| v.pid == pid).map(|v| v.ok));
        match known {
            Some(ok) => ok.then_some(window),
            None => {
                look(window);
                None
            }
        }
    }
}

/// Find out whether Seam may move the windows of this window's app, for the hook,
/// unless that process is known already. Worker only: it opens the process.
fn look(h: HWND) {
    let pid = winutil::process_id(h);
    if pid == 0 {
        return;
    }
    let created = created(pid).unwrap_or(0);
    let known = |list: &Vec<Verdict>| list.iter().any(|v| v.pid == pid && v.created == created);
    if known(&VERDICTS.lock().unwrap_or_else(|e| e.into_inner())) {
        return;
    }
    // A copy, so no lock is held while the process is opened.
    let at = EXCLUDED_AT.load(Ordering::SeqCst);
    let exclude = EXCLUDE.read().unwrap_or_else(|e| e.into_inner()).clone();
    let ok = !wincontrol::is_seam(h)
        && !winutil::is_elevated_over_us(h)
        && (exclude.is_empty() || winutil::exe_name(h).is_none_or(|name| !exclude.contains(&name)));
    let mut list = VERDICTS.lock().unwrap_or_else(|e| e.into_inner());
    // The exclude list changed meanwhile, so this answer is for the old one.
    if EXCLUDED_AT.load(Ordering::SeqCst) != at {
        return;
    }
    // A process gone, its id now another's.
    list.retain(|v| v.pid != pid);
    list.push(Verdict { pid, created, ok });
    if list.len() > MAX_VERDICTS {
        list.remove(0);
    }
}

/// Forget processes that are gone, then look at the apps of every window on screen.
fn sweep() {
    let pids: Vec<(u32, u64)> =
        VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|v| (v.pid, v.created)).collect();
    let gone: Vec<u32> = pids.into_iter().filter(|(pid, at)| created(*pid) != Some(*at)).map(|(pid, _)| pid).collect();
    if !gone.is_empty() {
        VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|v| !gone.contains(&v.pid));
    }
    // What is left is alive and checked just now, so a known pid needs no second look.
    let mut seen: Vec<u32> = VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|v| v.pid).collect();
    for h in winutil::top_level_windows() {
        let pid = winutil::process_id(h);
        if !seen.contains(&pid) && unsafe { IsWindowVisible(h) }.as_bool() {
            seen.push(pid);
            look(h);
        }
    }
}

/// When the process started, which tells it from a later one given the same id.
fn created(pid: u32) -> Option<u64> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut start, mut exit, mut kernel, mut user) = Default::default();
        let ok = GetProcessTimes(process, &mut start, &mut exit, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(process);
        let start: FILETIME = start;
        ok.then(|| (u64::from(start.dwHighDateTime) << 32) | u64::from(start.dwLowDateTime))
    }
}

/// The foreground changed: have the worker look at its app before a press on it.
unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object == OBJID_WINDOW.0 && !hwnd.is_invalid() {
        let worker = WORKER.load(Ordering::Relaxed);
        unsafe {
            let _ = PostThreadMessageW(worker, MSG_LOOK, WPARAM(hwnd.0 as usize), LPARAM(0));
        }
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
    /// The smallest and largest window rect a resize may make.
    min: (i32, i32),
    max: (i32, i32),
}

const FLAGS: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_ASYNCWINDOWPOS.0);

impl Drag {
    fn begin(hwnd: HWND, from: POINT, resize: bool) -> Option<Drag> {
        let started = Instant::now();
        // The hook only looked at what is cheap. A window that stopped answering is
        // left alone, and its release is still swallowed with the press.
        let visible = winutil::visible_rect(hwnd).filter(|_| !winutil::is_hung(hwnd))?;
        let mut start = winutil::window_rect(hwnd)?;
        tell(hwnd, true);
        tap();
        let mut sides = None;
        let (mut min, mut max) = ((0, 0), (i32::MAX, i32::MAX));
        if resize {
            sides = Some(sides_at(start, (from.x, from.y)));
            let (w, h) = minsize::of(hwnd, winutil::monitor_dpi(hwnd));
            min = (w + start.w - visible.w, h + start.h - visible.h);
            max = largest(hwnd);
        } else if winutil::is_maximized(hwnd) {
            // As dragging a maximized title bar does: its normal size, centred under the pointer.
            let dpi = winutil::monitor_dpi(hwnd);
            if let Some(normal) = winutil::normal_frame(hwnd, dpi) {
                let frame = Rect { x: from.x - normal.w / 2, y: from.y - normal.h / 2, w: normal.w, h: normal.h };
                winutil::place_restored(hwnd, frame, dpi);
                start = winutil::window_rect(hwnd).unwrap_or(start);
            }
        }
        Some(Drag { hwnd, started, start, from, sides, min, max })
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
                let (x, w) = edge(s.x, s.w, sx, dx, (self.min.0, self.max.0));
                let (y, h) = edge(s.y, s.h, sy, dy, (self.min.1, self.max.1));
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

/// One axis of a resize: the start and length after the pointer moved `d`, between
/// `min` and `max`. Moving the left or top edge leaves the far one where it was, so
/// a window held at its largest or smallest doesn't drift.
fn edge(start: i32, len: i32, side: i32, d: i32, (min, max): (i32, i32)) -> (i32, i32) {
    let fit = |to: i32| to.min(max).max(min);
    match side {
        -1 => {
            let to = fit(len - d);
            (start + len - to, to)
        }
        1 => (start, fit(len + d)),
        _ => (start, len),
    }
}

/// The largest window rect the app allows, in screen pixels, or no limit when it
/// doesn't say. Worker only: asking can wait on the app.
fn largest(h: HWND) -> (i32, i32) {
    let mut info = MINMAXINFO::default();
    let sent = unsafe {
        SendMessageTimeoutW(
            h,
            WM_GETMINMAXINFO,
            WPARAM(0),
            LPARAM(&mut info as *mut _ as isize),
            SMTO_ABORTIFHUNG,
            ASK_MS,
            None,
        )
    };
    let (w, ht) = (info.ptMaxTrackSize.x, info.ptMaxTrackSize.y);
    if sent.0 == 0 || w <= 0 || ht <= 0 {
        return (i32::MAX, i32::MAX);
    }
    // A window that ignores DPI answers in its own pixels, which the screen scales.
    let (screen, own) = (winutil::monitor_dpi(h) as i64, unsafe { GetDpiForWindow(h) }.max(96) as i64);
    ((w as i64 * screen / own) as i32, (ht as i64 * screen / own) as i32)
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

/// The worker: one drag at a time, from the hook's messages, and the looks at apps.
fn work(pending: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<u32>) {
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        if ready.send(GetCurrentThreadId()).is_err() {
            return;
        }
        SetTimer(None, 0, SWEEP_MS, None);
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
                MSG_LOOK => look(HWND(msg.wParam.0 as *mut _)),
                MSG_SWEEP => sweep(),
                WM_TIMER if msg.hwnd.is_invalid() => sweep(),
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
