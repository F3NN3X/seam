//! The blocking client: the pipe, calls and subscriptions.

use super::*;
use crate::log;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;

/// `\\.\pipe\seam-<username>`. Clients derive it, so no discovery file is needed.
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default().to_lowercase();
    format!(r"\\.\pipe\seam-{user}")
}

const ERROR_PIPE_BUSY: i32 = 231;

/// The process serving this pipe runs as the same user as this one.
fn server_is_ours(pipe: &File) -> bool {
    let mut pid = 0u32;
    let got = unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) };
    got.is_ok() && process::runs_as_us(pid)
}

/// Anyone can create a pipe under a name this predictable, so the server is
/// checked, and it only gets to identify us, never to act as us.
fn open_pipe() -> io::Result<File> {
    for _ in 0..20 {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0)
            .open(pipe_name());
        match opened {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => std::thread::sleep(Duration::from_millis(50)),
            Ok(pipe) if !server_is_ours(&pipe) => {
                log::info!("api client: the pipe is served by another user's process, so it was not used");
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Something else is using Seam's pipe"));
            }
            other => return other,
        }
    }
    Err(io::ErrorKind::TimedOut.into())
}

/// Why a call didn't produce a result.
#[derive(Debug)]
pub enum CallError {
    Io(io::Error),
    Rpc(RpcError),
}

impl From<io::Error> for CallError {
    fn from(e: io::Error) -> Self {
        CallError::Io(e)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: String,
    pub data: Value,
}

fn event_of(message: &Value) -> Option<Event> {
    let params = message.get("params")?;
    Some(Event { kind: params.get("type")?.as_str()?.to_string(), data: params.get("data")?.clone() })
}

/// The tray drops a connection that sends no request for 30 s unless it
/// subscribed, so a client quiet for this long connects again before its next call.
const RECONNECT_AFTER: Duration = Duration::from_secs(25);

/// One blocking connection to the tray.
pub struct Client {
    pipe: File,
    reader: BufReader<File>,
    next_id: u64,
    name: String,
    token: Option<String>,
    last: Instant,
}

impl Client {
    pub fn connect(client: &str, token: Option<&str>) -> Result<Client, CallError> {
        let pipe = open_pipe()?;
        let reader = BufReader::new(pipe.try_clone()?);
        let (name, owned) = (client.to_string(), token.map(str::to_string));
        let mut c = Client { pipe, reader, next_id: 1, name, token: owned, last: Instant::now() };
        let version = env!("CARGO_PKG_VERSION");
        c.request("hello", json!({ "client": client, "version": version, "token": token }))?;
        Ok(c)
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        if self.last.elapsed() >= RECONNECT_AFTER {
            *self = Client::connect(&self.name, self.token.as_deref())?;
        }
        let result = self.request(method, params);
        self.last = Instant::now();
        result
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.pipe, "{request}")?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            let message: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
            if message["id"] != id {
                continue; // An event, or a late answer to something else.
            }
            if let Some(e) = message.get("error") {
                let code = e["code"].as_i64().and_then(|c| i32::try_from(c).ok()).unwrap_or(INVALID_REQUEST);
                return Err(CallError::Rpc(RpcError::new(code, e["message"].as_str().unwrap_or_default())));
            }
            return Ok(message["result"].clone());
        }
    }

    /// Events arrive on a connection of their own. A pipe handle opened for
    /// blocking I/O can't write while another thread sits in a read on it, so
    /// this keeps `call` usable on the first connection.
    pub fn subscribe(client: &str, token: Option<&str>, topics: &[&str]) -> Result<Receiver<Event>, CallError> {
        let mut events = Client::connect(client, token)?;
        events.request("events.subscribe", json!({ "topics": topics }))?;
        let (tx, rx) = channel();
        std::thread::Builder::new().name("seam-api-events".into()).spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                if !matches!(events.reader.read_line(&mut line), Ok(n) if n > 0) {
                    return;
                }
                let message = serde_json::from_str::<Value>(&line).ok();
                if let Some(event) = message.as_ref().and_then(event_of)
                    && tx.send(event).is_err()
                {
                    return;
                }
            }
        })?;
        Ok(rx)
    }
}
