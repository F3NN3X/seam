//! The connection to the tray: made when first needed, and again when it drops.

use crate::rpc::Backend;
use seam::api::{CallError, Client, DISABLED, Event, MCP_CLIENT, MCP_OFF};
use serde_json::Value;
use std::io::ErrorKind;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

pub struct Link {
    name: String,
    client: Option<Client>,
    /// A second connection that only listens, so Seam can show an assistant is connected
    /// even while it is between calls. Nothing is ever sent on it.
    watch: Option<Receiver<Event>>,
    /// Not before this, after the listening connection failed.
    retry_at: Option<Instant>,
}

const WATCH_BACKOFF: Duration = Duration::from_secs(30);

impl Link {
    pub fn new() -> Link {
        Link { name: MCP_CLIENT.to_string(), client: None, watch: None, retry_at: None }
    }

    /// Keep the listening connection up, if Seam lets it. A refusal just means a later call tries again.
    fn watching(&mut self) {
        let gone = matches!(self.watch.as_ref().map(Receiver::try_recv), Some(Err(TryRecvError::Disconnected)));
        if gone {
            self.watch = None;
        }
        // A refused or failed subscribe waits a while: a full pipe makes each try cost about a second.
        let waiting = self.retry_at.is_some_and(|at| Instant::now() < at);
        if self.watch.is_none() && !waiting {
            self.watch = Client::subscribe(&self.name, None, &["mcp"]).ok();
            self.retry_at = self.watch.is_none().then(|| Instant::now() + WATCH_BACKOFF);
        }
    }
}

/// What to tell the assistant when a call didn't get an answer.
fn describe(error: CallError) -> String {
    match error {
        CallError::Rpc(e) if e.code == DISABLED && e.message == MCP_OFF => e.message,
        CallError::Rpc(e) if e.code == DISABLED => {
            format!("{}. The user can turn on \"Let other apps control Seam\" in Seam, General", e.message)
        }
        CallError::Rpc(e) => e.message,
        CallError::Io(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::TimedOut) => {
            "Seam isn't running. The user needs to start Seam from the Start menu".into()
        }
        CallError::Io(e) => format!("Couldn't reach Seam: {e}"),
    }
}

impl Backend for Link {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let mut retried = false;
        loop {
            let client = match self.client.as_mut() {
                Some(client) => client,
                None => self.client.insert(Client::connect(&self.name, None).map_err(describe)?),
            };
            match client.call(method, params.clone()) {
                Ok(result) => {
                    self.watching();
                    return Ok(result);
                }
                // The tray hung up on an idle connection, or restarted: connect once more. Only
                // when the request never went out, or it might run twice (a removed row, say).
                Err(CallError::Io(_)) if !retried && !client.unanswered() => {
                    retried = true;
                    self.client = None;
                }
                Err(CallError::Io(_)) if client.unanswered() => {
                    self.client = None;
                    return Err("Seam stopped answering. The change may or may not have been made, so check before trying again".into());
                }
                Err(e) => return Err(describe(e)),
            }
        }
    }

    fn client(&mut self, name: &str) {
        if !name.is_empty() {
            self.name = format!("{MCP_CLIENT} ({name})");
        }
        // Connect now, so Seam's General shows the assistant before its first call.
        self.watching();
    }
}
