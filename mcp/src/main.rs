//! `seam-mcp.exe`: an MCP server over stdio that lets an AI assistant use Seam through its
//! local API. Nothing goes to stdout but protocol lines. Seam decides what it may do, from
//! the two switches on General, so this only turns tool calls into API calls.

mod link;
mod rpc;
mod tools;

use std::io::{BufRead, Read, Write};

/// The longest line read, as in Seam's own API.
const MAX_LINE: u64 = 1 << 20;

/// One line, or `None` at the end of input. A line over the cap is read to its end and
/// thrown away, and comes back as `Err`, so the loop answers it and carries on.
fn read_line(input: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<Option<Result<(), ()>>> {
    line.clear();
    let n = input.by_ref().take(MAX_LINE + 1).read_until(b'\n', line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() as u64 > MAX_LINE && line.last() != Some(&b'\n') {
        line.clear();
        // Skip what is left of it, a buffer at a time.
        loop {
            let (used, done) = {
                let buffer = input.fill_buf()?;
                match buffer.iter().position(|&b| b == b'\n') {
                    Some(end) => (end + 1, true),
                    None => (buffer.len(), buffer.is_empty()),
                }
            };
            input.consume(used);
            if done {
                break;
            }
        }
        return Ok(Some(Err(())));
    }
    Ok(Some(Ok(())))
}

fn main() {
    let mut link = link::Link::new();
    let (mut input, mut out) = (std::io::stdin().lock(), std::io::stdout().lock());
    let mut bytes = Vec::new();
    loop {
        let answer = match read_line(&mut input, &mut bytes) {
            Ok(Some(Ok(()))) => {
                // A line that isn't UTF-8 reads lossily, and fails as bad JSON if it matters.
                let text = String::from_utf8_lossy(&bytes);
                if text.trim().is_empty() {
                    continue;
                }
                rpc::handle(text.trim(), &mut link)
            }
            Ok(Some(Err(()))) => Some(rpc::parse_error("The line is over 1 MiB")),
            Ok(None) | Err(_) => break,
        };
        if let Some(answer) = answer
            && (writeln!(out, "{answer}").is_err() || out.flush().is_err())
        {
            break;
        }
    }
}
