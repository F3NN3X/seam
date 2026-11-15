//! `seam-mcp.exe`: an MCP server over stdio that lets an AI assistant use Seam through its
//! local API. Nothing goes to stdout but protocol lines. Seam decides what it may do, from
//! the two switches on General, so this only turns tool calls into API calls.

mod link;
mod rpc;
mod tools;

use std::io::{BufRead, Write};

fn main() {
    let mut link = link::Link::new();
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = rpc::handle(line.trim(), &mut link)
            && (writeln!(out, "{answer}").is_err() || out.flush().is_err())
        {
            break;
        }
    }
}
