//! Newline-delimited JSON-RPC 2.0 for MCP: parsing a line and answering it. Pure,
//! so the tests need no Seam. What a tool call does is the `Backend`'s.

use crate::tools;
use serde_json::{Value, json};

/// Protocol versions this server speaks, newest first.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;

const INSTRUCTIONS: &str = "Seam arranges windows on a Windows desk and saves layouts as workspaces. \
Switching a workspace never closes windows. Changing workspaces needs the user to allow it in Seam, \
and each change shows them an Undo notice. If a call says assistant access is off, ask the user to \
turn on \"Let assistants use Seam (MCP)\" on Seam's General page.";

/// What the tools talk to: Seam's local API, or a stand-in in the tests.
pub trait Backend {
    /// An API call: the result, or the words to give the assistant.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// The assistant that connected, from its `initialize`.
    fn client(&mut self, name: &str);
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The answer to a line that couldn't be read at all.
pub fn parse_error(message: &str) -> Value {
    failure(Value::Null, PARSE_ERROR, message)
}

/// A short, single-line name for the log, from whatever the client called itself.
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string()
}

fn initialize(params: &Value, backend: &mut dyn Backend) -> Result<Value, (i32, String)> {
    let Some(asked) = params.get("protocolVersion").and_then(Value::as_str) else {
        return Err((INVALID_PARAMS, "protocolVersion is required".into()));
    };
    // The client's version when this server speaks it, else the newest, for the client to accept or leave.
    let version = VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[0]);
    if let Some(name) = params.pointer("/clientInfo/name").and_then(Value::as_str) {
        backend.client(&clean_name(name));
    }
    Ok(json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "seam-mcp", "title": "Seam", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    }))
}

fn call_tool(params: &Value, backend: &mut dyn Backend) -> Result<Value, (i32, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let Some(tool) = tools::find(name) else { return Err((INVALID_PARAMS, format!("Unknown tool: {name}"))) };
    let empty = serde_json::Map::new();
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(map)) => map,
        Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".into())),
    };
    let (text, is_error) = match backend.call(tool.method, tool.params(arguments)) {
        Ok(result) => (result.to_string(), false),
        Err(message) => (message, true),
    };
    Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

fn dispatch(method: &str, params: &Value, backend: &mut dyn Backend) -> Result<Value, (i32, String)> {
    match method {
        "initialize" => initialize(params, backend),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::TOOLS.iter().map(tools::Tool::listed).collect::<Vec<_>>() })),
        "tools/call" => call_tool(params, backend),
        _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
    }
}

/// Answer one line, or `None` when none is due: a notification, or a reply to nothing.
pub fn handle(line: &str, backend: &mut dyn Backend) -> Option<Value> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(failure(Value::Null, PARSE_ERROR, "Parse error"));
    };
    // Batches left the protocol in 2025-06-18, so an array is as wrong as any other non-object.
    let Some(object) = message.as_object() else {
        return Some(failure(Value::Null, INVALID_REQUEST, "A request is a JSON object"));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        // The client answering a request this server never makes.
        if object.contains_key("result") || object.contains_key("error") {
            return None;
        }
        return Some(failure(id.unwrap_or(Value::Null), INVALID_REQUEST, "method is missing"));
    };
    let id = id?; // `notifications/initialized` and the like have none.
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(failure(id, INVALID_REQUEST, "not a JSON-RPC 2.0 request"));
    }
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    Some(match dispatch(method, &params, backend) {
        Ok(result) => success(id, result),
        Err((code, message)) => failure(id, code, &message),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        calls: Vec<(String, Value)>,
        client: Option<String>,
        answer: Option<Result<Value, String>>,
    }

    impl Backend for Fake {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.calls.push((method.to_string(), params));
            self.answer.clone().unwrap_or_else(|| Ok(json!({ "ok": true })))
        }

        fn client(&mut self, name: &str) {
            self.client = Some(name.to_string());
        }
    }

    fn ask(fake: &mut Fake, line: &str) -> Value {
        handle(line, fake).expect("an answer")
    }

    #[test]
    fn initialize_negotiates_the_version() {
        let mut fake = Fake::default();
        let line = |v: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{v}","capabilities":{{}},"clientInfo":{{"name":"Claude\nDesktop"}}}}}}"#
            )
        };
        let old = ask(&mut fake, &line("2025-03-26"));
        assert_eq!(old["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(old["result"]["serverInfo"]["name"], "seam-mcp");
        assert!(old["result"]["capabilities"]["tools"].is_object());
        assert_eq!(old["id"], 1);
        let future = ask(&mut fake, &line("2099-01-01"));
        assert_eq!(future["result"]["protocolVersion"], VERSIONS[0]);
        assert_eq!(fake.client.as_deref(), Some("ClaudeDesktop"));
        let missing = ask(&mut fake, r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{}}"#);
        assert_eq!(missing["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn notifications_and_replies_get_no_answer() {
        let mut fake = Fake::default();
        assert!(handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &mut fake).is_none());
        assert!(
            handle(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}"#, &mut fake)
                .is_none()
        );
        assert!(handle(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#, &mut fake).is_none());
    }

    #[test]
    fn bad_lines_get_json_rpc_errors() {
        let mut fake = Fake::default();
        assert_eq!(ask(&mut fake, "nope")["error"]["code"], PARSE_ERROR);
        assert_eq!(ask(&mut fake, "[]")["error"]["code"], INVALID_REQUEST);
        assert_eq!(ask(&mut fake, r#"{"jsonrpc":"2.0","id":4}"#)["error"]["code"], INVALID_REQUEST);
        assert_eq!(ask(&mut fake, r#"{"id":4,"method":"ping"}"#)["error"]["code"], INVALID_REQUEST);
        let unknown = ask(&mut fake, r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#);
        assert_eq!((unknown["error"]["code"].as_i64(), &unknown["id"]), (Some(-32601), &json!(5)));
    }

    #[test]
    fn ping_and_list() {
        let mut fake = Fake::default();
        assert_eq!(
            ask(&mut fake, r#"{"jsonrpc":"2.0","id":"a","method":"ping"}"#),
            json!({"jsonrpc":"2.0","id":"a","result":{}})
        );
        let listed = ask(&mut fake, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), tools::TOOLS.len());
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["description"].as_str().is_some_and(|d| !d.is_empty()));
        }
        // Nothing here closes a window, sets a launch program, or opens something.
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.iter().all(|n| !n.contains("close") && !n.contains("open") && !n.contains("config")));
    }

    #[test]
    fn a_call_returns_content_and_is_error() {
        let mut fake = Fake::default();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"seam_snap_back"}}"#;
        let done = ask(&mut fake, line);
        assert_eq!(done["result"]["isError"], false);
        assert_eq!(done["result"]["content"][0]["type"], "text");
        assert_eq!(fake.calls[0].0, "workspace.snap_back");

        fake.answer = Some(Err("Assistant access is off in Seam, General".into()));
        let off = ask(&mut fake, line);
        assert_eq!(off["result"]["isError"], true);
        assert_eq!(off["result"]["content"][0]["text"], "Assistant access is off in Seam, General");
    }

    #[test]
    fn a_switch_is_always_alongside_and_extra_arguments_are_dropped() {
        let mut fake = Fake::default();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"seam_switch_workspace","arguments":{"name":"Mail","alongside":false,"token":"x","exe":"c:\\a.exe"}}}"#;
        ask(&mut fake, line);
        assert_eq!(fake.calls, vec![("workspace.switch".to_string(), json!({ "name": "Mail", "alongside": true }))]);
    }

    #[test]
    fn bad_calls_are_protocol_errors() {
        let mut fake = Fake::default();
        let unknown =
            ask(&mut fake, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"seam_close_window"}}"#);
        assert_eq!(unknown["error"]["code"], INVALID_PARAMS);
        let arguments = ask(
            &mut fake,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"seam_state","arguments":[]}}"#,
        );
        assert_eq!(arguments["error"]["code"], INVALID_PARAMS);
        assert!(fake.calls.is_empty());
    }
}
