//! The connection to the tray: made when first needed, and again when it drops.

use crate::rpc::Backend;
use seam::api::{CallError, Client, DISABLED, Event, MCP_CLIENT, MCP_OFF};
use serde_json::Value;
use std::io::ErrorKind;
use std::sync::mpsc::{Receiver, TryRecvError};

pub struct Link {
    name: String,
    client: Option<Client>,
    /// A second connection that only listens, so Seam can show an assistant is connected
    /// even while it is between calls. Nothing is ever sent on it.
    watch: Option<Receiver<Event>>,
}

impl Link {
    pub fn new() -> Link {
        Link { name: MCP_CLIENT.to_string(), client: None, watch: None }
    }

    /// Keep the listening connection up, if Seam lets it. A refusal just means a later call tries again.
    fn watching(&mut self) {
        let gone = matches!(self.watch.as_ref().map(Receiver::try_recv), Some(Err(TryRecvError::Disconnected)));
        if gone {
            self.watch = None;
        }
        if self.watch.is_none() {
            self.watch = Client::subscribe(&self.name, None, &["mcp"]).ok();
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
                // The tray hung up on an idle connection, or restarted: connect once more.
                Err(CallError::Io(_)) if !retried => {
                    retried = true;
                    self.client = None;
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
