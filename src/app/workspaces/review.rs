//! The capture review: a capture kept under a token for a short while, then saved as the
//! panel ticked it (D721).

use super::capture::{begin_capture, save_capture};
use super::*;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

fn new_token() -> String {
    format!("{:016x}", RandomState::new().build_hasher().finish())
}

/// Start a capture the review will show. The answer comes when it is read, as
/// `workspace.capture_preview` says. Returns an answer only if it can't start.
pub(in crate::app) fn start_preview(app: &mut App, into: Option<String>, reply: Sender<Reply>) -> Option<Reply> {
    begin_capture(app, (None, into, None, None), Some(reply), true)
}

/// A preview was read: keep it under a token and answer with what the review lists.
pub(super) fn previewed(app: &mut App, job: Capturing, got: switcher::Captured) {
    let Some(reply) = &job.reply else { return };
    // Updating a workspace with no window open keeps every row, so it still gets its token.
    if got.rules.is_empty() && job.into.is_none() {
        let _ = reply.send(refused("No windows to capture"));
        return;
    }
    let Some(desk) = app.cfg.desks.get(&job.sig) else {
        let _ = reply.send(refused("That desk is gone"));
        return;
    };
    let existing = workspace::list(&desk.workspaces);
    let old = job.into.as_ref().and_then(|id| existing.iter().find(|w| w.id == *id));
    if job.into.is_some() && old.is_none() {
        let _ = reply.send(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
        return;
    }
    let screens = job_screens(app);
    let number = |monitor: &str| screens.iter().find(|s| s.id == monitor).map(|s| s.number);
    let ticks = workspace::review::ticks(&got.rules, &got.elevated);
    let windows: Vec<Value> = got
        .rules
        .iter()
        .zip(&ticks)
        .enumerate()
        .map(|(i, (rule, (ticked, why)))| {
            let app = workspace::report::app_name(&rule.exe, &rule.title_hint);
            json!({ "i": i, "app": app, "title": rule.title_hint, "screen": number(&rule.monitor), "ticked": ticked, "why": why })
        })
        .collect();
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let name = old.map_or_else(|| workspace::next_name(&names), |w| w.name.clone());
    let mut answer = json!({ "name": name, "windows": windows });
    if let Some(old) = old {
        let (bound, elsewhere) = switcher::capture_bindings(old, &got.hwnds, app.cfg.workspaces_remember_places);
        let rows = workspace::review::not_open(old, &got.rules, &bound, &elsewhere, &got.others);
        answer["not_open"] = rows
            .into_iter()
            .map(|row| {
                let rule = &old.windows[row];
                let app = workspace::report::app_name(&rule.exe, &rule.title_hint);
                json!({ "row": row, "app": app, "screen": number(&rule.monitor) })
            })
            .collect();
    }
    let token = new_token();
    answer["token"] = json!(token);
    let (sig, into, tx) = (job.sig.clone(), job.into.clone(), reply.clone());
    // A new preview replaces the one before it.
    app.preview = Some(Preview { token, sig, into, captured: got, expires: Instant::now() + PREVIEW_FOR });
    let _ = tx.send(Ok(answer));
}

/// Save the ticked windows of the preview `save` names. Returns an answer only when it
/// can't go ahead; otherwise the save answers.
pub(in crate::app) fn save_preview(
    app: &mut App,
    name: Option<String>,
    into: Option<String>,
    save: api::Save,
    reply: Sender<Reply>,
) -> Option<Reply> {
    if app.capturing.is_some() {
        return Some(busy("Already capturing"));
    }
    if app.closing.is_some() {
        return Some(busy("Windows of a workspace are still being closed or minimized"));
    }
    let live = |p: &&Preview| p.token == save.token && p.expires > Instant::now() && p.sig == app.sig && p.into == into;
    let Some(preview) = app.preview.as_ref().filter(live) else {
        return Some(refused("preview expired"));
    };
    let got = &preview.captured;
    let mut take = save.take;
    take.sort_unstable();
    take.dedup();
    if take.iter().any(|i| *i >= got.rules.len()) {
        return Some(Err(RpcError::new(api::INVALID_PARAMS, "take has a window the preview doesn't")));
    }
    if take.is_empty() && into.is_none() {
        return Some(refused("No window is ticked"));
    }
    // What isn't ticked is still open, so it keeps its app from counting as closed.
    let left: Vec<workspace::Candidate> = (0..got.rules.len())
        .filter(|i| !take.contains(i))
        .map(|i| {
            let rule = &got.rules[i];
            workspace::Candidate {
                hwnd: got.hwnds[i],
                exe: rule.exe.clone(),
                aumid: rule.aumid.clone(),
                class: rule.class.clone(),
                title: rule.title_hint.clone(),
                z: i,
            }
        })
        .collect();
    let rules = take.iter().enumerate().map(|(z, i)| Rule { z: z as u32, ..got.rules[*i].clone() }).collect();
    let ticked = switcher::Captured {
        rules,
        hwnds: take.iter().map(|i| got.hwnds[*i]).collect(),
        others: got.others.iter().cloned().chain(left).collect(),
        elevated: Vec::new(),
    };
    let sig = preview.sig.clone();
    app.preview = None;
    let job = Capturing {
        reply: Some(reply),
        name,
        into,
        row: None,
        pick: None,
        sig,
        quick: false,
        add: None,
        preview: false,
        closed: save.closed,
    };
    save_capture(app, job, ticked);
    None
}
