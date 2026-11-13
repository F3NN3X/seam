//! The API's pipe server. One acceptor thread plus one thread per client.
//!
//! Nothing here touches the mouse hook or a window except to post
//! `WM_APP_API`. Reads come from a snapshot the main thread replaces, and
//! commands wait in a queue for it. The main thread only ever holds these locks
//! for a moment, so a client can't hold it up, however slow.
//!
//! Each thread uses overlapped I/O and waits for it, because a synchronous
//! handle can't write while another call is reading, and events must reach a
//! client that's sitting idle.

use crate::log;
use crate::tray::WM_APP_API;
use seam::api::{
    API_VERSION, BUSY, Command, DISABLED, MAX_LINE, MAX_QUEUED_EVENTS, Peer, Reply, RpcError, desks_list, error_line,
    event_line, parse_request, pipe_name, result_line, topic_of, workspaces_list, zones_layouts,
};
use seam::process;
use seam::state::Snapshot;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, PostMessageW};
use windows::core::PCWSTR;

const MAX_CLIENTS: u32 = 8;
const PIPE_BUFFER: u32 = 64 * 1024;
const WRITE_TIMEOUT_MS: u32 = 2000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
/// A client with no `events.subscribe` that sends no request line for this long is dropped.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// A command waiting for the main thread.
pub struct Queued {
    pub command: Command,
    pub reply: Sender<Reply>,
    /// Sent by the panel the tray started, which proved it with the token.
    pub panel: bool,
}

/// Lines waiting for a client, which its own thread writes.
struct Outbox {
    lines: Mutex<VecDeque<String>>,
    /// Signalled when there's something to write.
    wake: isize,
    /// It fell too far behind, so its thread should hang up.
    overflowed: AtomicBool,
}

struct Conn {
    id: u64,
    name: Option<String>,
    topics: Vec<String>,
    /// It proved it is the panel with the token.
    panel: bool,
    outbox: Arc<Outbox>,
}

struct Shared {
    hwnd: isize,
    token: String,
    snapshot: RwLock<Arc<Snapshot>>,
    api_enabled: AtomicBool,
    queue: Mutex<VecDeque<Queued>>,
    conns: Mutex<Vec<Conn>>,
    clients_changed: AtomicBool,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn user_sid() -> Option<String> {
    process::user_sid(unsafe { GetCurrentProcess() })
}

/// A descriptor only the current user can open. It lives as long as the process.
fn current_user_only() -> Option<isize> {
    let sddl = wide(&format!("D:P(A;;GA;;;{})", user_sid()?));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .ok()?;
    }
    Some(descriptor.0 as isize)
}

fn create_pipe(descriptor: isize, first: bool) -> Option<isize> {
    let name = wide(&pipe_name());
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor as *mut c_void,
        bInheritHandle: false.into(),
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let pipe = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            MAX_CLIENTS,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            Some(&attributes),
        )
    };
    (!pipe.is_invalid()).then_some(pipe.0 as isize)
}

fn new_event(manual_reset: bool) -> Option<HANDLE> {
    unsafe { CreateEventW(None, manual_reset, false, None).ok() }
}

/// A fresh token each run, so only a panel the tray started can use panel calls.
pub fn new_token() -> Option<String> {
    let mut bytes = [0u8; 16];
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.ok().ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Start serving. The tray keeps working without the API if this fails.
pub fn start(hwnd: isize, token: String, first: Snapshot, api_enabled: bool) {
    let Some(descriptor) = current_user_only() else {
        log::info!("api off: no security descriptor");
        return;
    };
    let Some(pipe) = create_pipe(descriptor, true) else {
        log::info!("api off: the pipe name is taken");
        return;
    };
    let _ = SHARED.set(Shared {
        hwnd,
        token,
        snapshot: RwLock::new(Arc::new(first)),
        api_enabled: AtomicBool::new(api_enabled),
        queue: Mutex::new(VecDeque::new()),
        conns: Mutex::new(Vec::new()),
        clients_changed: AtomicBool::new(false),
    });
    let spawned = std::thread::Builder::new().name("seam-api".into()).spawn(move || accept_loop(pipe, descriptor));
    if let Err(e) = spawned {
        log::info!("api off: {e}");
    }
}

/// Wait for a client on this pipe instance. Overlapped handles need the event
/// even though nothing else happens on this thread meanwhile.
fn wait_for_client(pipe: HANDLE, event: HANDLE) -> bool {
    let mut overlapped = OVERLAPPED { hEvent: event, ..Default::default() };
    unsafe {
        match ConnectNamedPipe(pipe, Some(&mut overlapped)) {
            Ok(()) => true,
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => true,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                let mut n = 0;
                GetOverlappedResult(pipe, &overlapped, &mut n, true).is_ok()
            }
            Err(_) => false,
        }
    }
}

fn accept_loop(first: isize, descriptor: isize) {
    let Some(event) = new_event(true) else { return };
    let mut pipe = first;
    loop {
        if wait_for_client(HANDLE(pipe as *mut c_void), event) {
            let serving = pipe;
            let spawned = std::thread::Builder::new().name("seam-api-client".into()).spawn(move || serve(serving));
            if let Err(e) = spawned {
                log::info!("api client refused: {e}");
                unsafe {
                    let _ = CloseHandle(HANDLE(serving as *mut c_void));
                }
            }
        } else {
            // Keep this instance rather than closing it and creating another, so
            // the name is never free for someone else to create first.
            unsafe {
                let _ = DisconnectNamedPipe(HANDLE(pipe as *mut c_void));
            }
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        // With every instance in use this fails until a client leaves.
        pipe = loop {
            match create_pipe(descriptor, false) {
                Some(next) => break next,
                None => std::thread::sleep(Duration::from_millis(250)),
            }
        };
    }
}

/// The client is `seam.exe` or `seam-settings.exe` from the folder this one runs from.
fn client_is_seam(pipe: HANDLE) -> bool {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) }.is_err() {
        return false;
    }
    let Some(image) = process::image_path(pid) else { return false };
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) else {
        return false;
    };
    let image = image.to_string_lossy().to_lowercase();
    ["seam.exe", "seam-settings.exe"].iter().any(|name| dir.join(name).to_string_lossy().to_lowercase() == image)
}

/// Write one line, giving up on a client that has stopped reading.
fn write_line(pipe: HANDLE, event: HANDLE, line: &str) -> bool {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    let mut overlapped = OVERLAPPED { hEvent: event, ..Default::default() };
    let mut written = 0u32;
    unsafe {
        let _ = ResetEvent(event);
        if let Err(e) = WriteFile(pipe, Some(&bytes), None, Some(&mut overlapped)) {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return false;
            }
            if WaitForSingleObject(event, WRITE_TIMEOUT_MS) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(pipe, Some(&overlapped));
                let _ = GetOverlappedResult(pipe, &overlapped, &mut written, true);
                return false;
            }
        }
        GetOverlappedResult(pipe, &overlapped, &mut written, true).is_ok() && written as usize == bytes.len()
    }
}

/// Everything one connection knows about itself.
struct Session {
    pipe: HANDLE,
    write_event: HANDLE,
    conn_id: u64,
    outbox: Arc<Outbox>,
    panel: bool,
    own_image: bool,
}

impl Session {
    fn flush(&self) -> bool {
        if self.outbox.overflowed.load(Ordering::Relaxed) {
            return false;
        }
        loop {
            let Some(line) = guard(&self.outbox.lines).pop_front() else { return true };
            if !write_line(self.pipe, self.write_event, &line) {
                return false;
            }
        }
    }

    fn peer(&self, shared: &Shared) -> Peer {
        Peer { panel: self.panel, own_image: self.own_image, api_enabled: shared.api_enabled.load(Ordering::Relaxed) }
    }

    fn hello(&mut self, shared: &Shared, client: String, token: Option<String>) -> Reply {
        // An empty token (the RNG failed) never matches, so nobody is the panel.
        self.panel = !shared.token.is_empty() && token.is_some_and(|t| t == shared.token);
        let peer = self.peer(shared);
        if !peer.api_enabled && !peer.panel && !peer.own_image {
            return Err(RpcError::new(DISABLED, "Other apps can't control Seam right now"));
        }
        let name = if self.panel { "panel".to_string() } else { client };
        if let Some(conn) = guard(&shared.conns).iter_mut().find(|c| c.id == self.conn_id) {
            conn.name = Some(name);
            conn.panel = self.panel;
        }
        notify_clients_changed(shared);
        let packaged = read_snapshot(shared).state.packaged;
        Ok(json!({ "api_version": API_VERSION, "seam_version": env!("CARGO_PKG_VERSION"), "packaged": packaged }))
    }

    fn subscribe(&self, shared: &Shared, topics: Vec<String>) -> Reply {
        if let Some(conn) = guard(&shared.conns).iter_mut().find(|c| c.id == self.conn_id) {
            conn.topics = topics;
        }
        Ok(json!({ "ok": true }))
    }

    fn answer(&mut self, shared: &Shared, command: Command) -> Reply {
        match command {
            Command::Hello { client, token, .. } => self.hello(shared, client, token),
            Command::EventsSubscribe(topics) => self.subscribe(shared, topics),
            Command::AppInfo => Ok(json!({
                "seam_version": env!("CARGO_PKG_VERSION"),
                "build": option_env!("SEAM_BUILD").unwrap_or("dev"),
                "packaged": read_snapshot(shared).state.packaged,
                "pid": std::process::id(),
            })),
            Command::StateGet => {
                serde_json::to_value(&read_snapshot(shared).state).map_err(|e| RpcError::new(BUSY, e.to_string()))
            }
            Command::ConfigGet => Ok(read_snapshot(shared).config.clone()),
            Command::DesksList => Ok(desks_list(&read_snapshot(shared))),
            Command::WorkspacesList { desk } => workspaces_list(&read_snapshot(shared), desk.as_deref()),
            Command::ZonesLayouts(number) => {
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                }
                zones_layouts(&read_snapshot(shared), number, (pt.x, pt.y))
            }
            command => run_on_main_thread(shared, command, self.panel),
        }
    }

    fn handle_line(&mut self, shared: &Shared, line: &str) -> bool {
        let text = match parse_request(line, self.peer(shared)) {
            Ok((id, command)) => match self.answer(shared, command) {
                Ok(result) => result_line(&id, &result),
                Err(e) => error_line(&id, &e),
            },
            Err((id, e)) => error_line(&id, &e),
        };
        write_line(self.pipe, self.write_event, &text)
    }

    /// Answer every complete line in `input`, keeping what's left for later.
    fn handle_input(&mut self, shared: &Shared, input: &mut Vec<u8>) -> bool {
        while let Some(end) = input.iter().position(|&b| b == b'\n') {
            let rest = input.split_off(end + 1);
            let line = std::mem::replace(input, rest);
            let text = String::from_utf8_lossy(&line);
            if !text.trim().is_empty() && !self.handle_line(shared, text.trim()) {
                return false;
            }
        }
        input.len() <= MAX_LINE
    }
}

fn run_on_main_thread(shared: &Shared, command: Command, panel: bool) -> Reply {
    let (reply, answer) = channel();
    guard(&shared.queue).push_back(Queued { command, reply, panel });
    wake_main_thread(shared);
    answer.recv_timeout(COMMAND_TIMEOUT).map_err(|_| RpcError::new(BUSY, "Seam didn't answer in time"))?
}

fn wake_main_thread(shared: &Shared) {
    let hwnd = HWND(shared.hwnd as *mut c_void);
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_API, WPARAM(0), LPARAM(0));
    }
}

fn notify_clients_changed(shared: &Shared) {
    shared.clients_changed.store(true, Ordering::Relaxed);
    wake_main_thread(shared);
}

fn read_snapshot(shared: &Shared) -> Arc<Snapshot> {
    shared.snapshot.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn serve(pipe: isize) {
    let Some(shared) = SHARED.get() else { return };
    let pipe = HANDLE(pipe as *mut c_void);
    let events = (new_event(true), new_event(true), new_event(false));
    if let (Some(read_event), Some(write_event), Some(wake)) = events {
        pump(shared, pipe, read_event, write_event, wake);
        unsafe {
            let _ = CloseHandle(read_event);
            let _ = CloseHandle(write_event);
            let _ = CloseHandle(wake);
        }
    }
    unsafe {
        let _ = DisconnectNamedPipe(pipe);
        let _ = CloseHandle(pipe);
    }
}

/// The connection has asked for events, so it's idle by design.
fn subscribed(shared: &Shared, conn_id: u64) -> bool {
    guard(&shared.conns).iter().any(|c| c.id == conn_id && !c.topics.is_empty())
}

fn pump(shared: &Shared, pipe: HANDLE, read_event: HANDLE, write_event: HANDLE, wake: HANDLE) {
    let outbox = Arc::new(Outbox {
        lines: Mutex::new(VecDeque::new()),
        wake: wake.0 as isize,
        overflowed: AtomicBool::new(false),
    });
    let conn_id = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    guard(&shared.conns).push(Conn {
        id: conn_id,
        name: None,
        topics: Vec::new(),
        panel: false,
        outbox: outbox.clone(),
    });
    notify_clients_changed(shared);

    let own_image = client_is_seam(pipe);
    let mut session = Session { pipe, write_event, conn_id, outbox, panel: false, own_image };
    let mut buffer = vec![0u8; 8192];
    let mut input = Vec::new();
    let mut deadline = Instant::now() + IDLE_TIMEOUT;

    'connection: loop {
        let mut overlapped = OVERLAPPED { hEvent: read_event, ..Default::default() };
        unsafe {
            let _ = ResetEvent(read_event);
            if let Err(e) = ReadFile(pipe, Some(&mut buffer), None, Some(&mut overlapped))
                && e.code() != ERROR_IO_PENDING.to_hresult()
            {
                break;
            }
        }
        // The read stays pending while events go out.
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            let waited = unsafe { WaitForMultipleObjects(&[read_event, wake], false, left) };
            if waited == WAIT_OBJECT_0 {
                break;
            }
            if waited == WAIT_TIMEOUT {
                if subscribed(shared, conn_id) {
                    deadline = Instant::now() + IDLE_TIMEOUT;
                    continue;
                }
                log::info!("api client dropped: no request in {} s", IDLE_TIMEOUT.as_secs());
                unsafe {
                    let mut n = 0;
                    let _ = CancelIoEx(pipe, Some(&overlapped));
                    let _ = GetOverlappedResult(pipe, &overlapped, &mut n, true);
                }
                break 'connection;
            }
            if waited.0 != WAIT_OBJECT_0.0 + 1 || !session.flush() {
                unsafe {
                    let mut n = 0;
                    let _ = CancelIoEx(pipe, Some(&overlapped));
                    let _ = GetOverlappedResult(pipe, &overlapped, &mut n, true);
                }
                break 'connection;
            }
        }
        let mut read = 0u32;
        if unsafe { GetOverlappedResult(pipe, &overlapped, &mut read, false) }.is_err() {
            break;
        }
        input.extend_from_slice(&buffer[..read as usize]);
        if input.contains(&b'\n') {
            deadline = Instant::now() + IDLE_TIMEOUT;
        }
        if !session.handle_input(shared, &mut input) || !session.flush() {
            break;
        }
    }

    guard(&shared.conns).retain(|c| c.id != conn_id);
    notify_clients_changed(shared);
}

/// Replace what clients read. Returns whether the state changed.
pub fn publish(next: Snapshot, api_enabled: bool) -> bool {
    let Some(shared) = SHARED.get() else { return false };
    shared.api_enabled.store(api_enabled, Ordering::Relaxed);
    let mut slot = shared.snapshot.write().unwrap_or_else(|e| e.into_inner());
    let changed = slot.state != next.state;
    *slot = Arc::new(next);
    changed
}

/// Queue an event for every subscribed client. Their threads do the writing.
pub fn broadcast(kind: &str, data: &Value) {
    queue_event(kind, data, false);
}

/// Like `broadcast`, but only the panel gets it.
pub fn broadcast_to_panels(kind: &str, data: &Value) {
    queue_event(kind, data, true);
}

fn queue_event(kind: &str, data: &Value, panels_only: bool) {
    let Some(shared) = SHARED.get() else { return };
    let line = event_line(kind, data);
    let topic = topic_of(kind);
    let wanted = |c: &&Conn| (c.panel || !panels_only) && c.topics.iter().any(|t| t == topic);
    for conn in guard(&shared.conns).iter().filter(wanted) {
        let mut lines = guard(&conn.outbox.lines);
        if lines.len() >= MAX_QUEUED_EVENTS {
            conn.outbox.overflowed.store(true, Ordering::Relaxed);
        } else {
            lines.push_back(line.clone());
        }
        unsafe {
            let _ = SetEvent(HANDLE(conn.outbox.wake as *mut c_void));
        }
    }
}

pub fn take_commands() -> Vec<Queued> {
    SHARED.get().map_or_else(Vec::new, |shared| guard(&shared.queue).drain(..).collect())
}

/// The names of connected clients, if they changed since the last call.
pub fn take_client_names() -> Option<Vec<String>> {
    let shared = SHARED.get()?;
    shared.clients_changed.swap(false, Ordering::Relaxed).then(|| {
        let conns = guard(&shared.conns);
        conns.iter().map(|c| c.name.clone().unwrap_or_else(|| "anonymous".into())).collect()
    })
}
