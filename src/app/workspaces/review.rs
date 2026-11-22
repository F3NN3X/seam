//! The capture review: a capture kept under a token for a short while, then saved as the
//! panel ticked it (D721).

use super::capture::{begin_capture, save_capture};
use super::*;

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
        let running = workspace::review::running(old, &got.rules, &bound, &elsewhere, &rows);
        answer["not_open"] = rows
            .into_iter()
            .zip(running)
            .map(|(row, running)| {
                let rule = &old.windows[row];
                let app = workspace::report::app_name(&rule.exe, &rule.title_hint);
                json!({ "row": row, "app": app, "screen": number(&rule.monitor), "running": running })
            })
            .collect();
    }
    let Some(token) = crate::api_server::new_token() else {
        let _ = reply.send(refused("Couldn't make a token for the preview"));
        return;
    };
    answer["token"] = json!(token);
    let (sig, into, tx) = (job.sig.clone(), job.into.clone(), reply.clone());
    let rows = old.map_or_else(Vec::new, row_keys);
    // A new preview replaces the one before it.
    app.preview = Some(Preview { token, sig, into, rows, captured: got, expires: Instant::now() + PREVIEW_FOR });
    let _ = tx.send(Ok(answer));
}

/// What makes a row the same row: its app and the window it was captured from.
pub(in crate::app) type RowKey = (String, Option<String>, String, String);

fn row_keys(ws: &Workspace) -> Vec<RowKey> {
    let key = |r: &Rule| (r.exe.clone(), r.aumid.clone(), r.class.clone(), r.title_hint.clone());
    ws.windows.iter().map(key).collect()
}

/// Whether `p` is the preview a save names: the same token, desk and workspace, not
/// past its time, and the workspace still has the rows it had.
fn live(p: &Preview, (token, sig, into): (&str, &str, &Option<String>), rows: &[RowKey], now: Instant) -> bool {
    p.token == token && p.sig == sig && p.into == *into && p.rows == rows && p.expires > now
}

/// The windows of a preview that `take` ticks, numbered from the top again, with the
/// ones left out added to `others`: they are still open, so they keep their app from
/// counting as closed. Saving nothing is only for an update, which keeps every row.
fn ticked_subset(got: &switcher::Captured, take: &[usize], updating: bool) -> Result<switcher::Captured, RpcError> {
    let mut take = take.to_vec();
    take.sort_unstable();
    take.dedup();
    if take.iter().any(|i| *i >= got.rules.len()) {
        return Err(RpcError::new(api::INVALID_PARAMS, "take has a window the preview doesn't"));
    }
    if take.is_empty() && !updating {
        return Err(RpcError::new(api::REFUSED, "No window is ticked"));
    }
    let left = (0..got.rules.len()).filter(|i| !take.contains(i)).map(|i| {
        let rule = &got.rules[i];
        workspace::Candidate {
            hwnd: got.hwnds[i],
            exe: rule.exe.clone(),
            aumid: rule.aumid.clone(),
            class: rule.class.clone(),
            title: rule.title_hint.clone(),
            z: i,
        }
    });
    Ok(switcher::Captured {
        rules: take.iter().enumerate().map(|(z, i)| Rule { z: z as u32, ..got.rules[*i].clone() }).collect(),
        hwnds: take.iter().map(|i| got.hwnds[*i]).collect(),
        others: got.others.iter().cloned().chain(left).collect(),
        elevated: Vec::new(),
    })
}

/// Save the ticked windows of the preview `save` names. Returns an answer only when it
/// can't go ahead; otherwise the save answers.
pub(in crate::app) fn save_preview(
    app: &mut App,
    (name, into): (Option<String>, Option<String>),
    save: api::Save,
    (reply, from_panel): (Sender<Reply>, bool),
) -> Option<Reply> {
    if app.capturing.is_some() {
        return Some(busy("Already capturing"));
    }
    if app.closing.is_some() {
        return Some(busy("Windows of a workspace are still being closed or minimized"));
    }
    let now = Instant::now();
    // A preview past its time holds folders and pages for nothing.
    if app.preview.as_ref().is_some_and(|p| p.token == save.token && p.expires <= now) {
        app.preview = None;
    }
    let rows = into
        .as_ref()
        .and_then(|id| desk_workspaces(app).iter().find(|w| w.id == *id).map(row_keys))
        .unwrap_or_default();
    let Some(preview) = app.preview.as_ref().filter(|p| live(p, (&save.token, &app.sig, &into), &rows, now)) else {
        return Some(refused(api::PREVIEW_EXPIRED));
    };
    let ticked = match ticked_subset(&preview.captured, &save.take, into.is_some()) {
        Ok(ticked) => ticked,
        Err(e) => return Some(Err(e)),
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
        move_from: None,
        preview: false,
        closed: save.closed,
        from_panel,
    };
    save_capture(app, job, ticked);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use seam::workspace::review::not_open;
    use std::time::Duration;

    fn rule(exe: &str) -> Rule {
        Rule { exe: exe.into(), z: 9, ..Rule::default() }
    }

    fn captured(exes: &[&str]) -> switcher::Captured {
        switcher::Captured {
            rules: exes.iter().map(|e| rule(e)).collect(),
            hwnds: (0..exes.len() as isize).map(|h| h + 100).collect(),
            ..Default::default()
        }
    }

    fn keys(names: &[&str]) -> Vec<RowKey> {
        names.iter().map(|n| (n.to_string(), None, String::new(), String::new())).collect()
    }

    fn preview(now: Instant) -> Preview {
        Preview {
            token: "t".into(),
            sig: "desk".into(),
            into: Some("ws".into()),
            rows: keys(&["a", "b", "c"]),
            captured: captured(&["a.exe"]),
            expires: now + PREVIEW_FOR,
        }
    }

    #[test]
    fn a_preview_is_live_until_its_time_is_up() {
        let now = Instant::now();
        let p = preview(now);
        let into = Some("ws".to_string());
        let key = ("t", "desk", &into);
        assert!(live(&p, key, &keys(&["a", "b", "c"]), now));
        assert!(live(&p, key, &keys(&["a", "b", "c"]), now + PREVIEW_FOR - Duration::from_millis(1)));
        assert!(!live(&p, key, &keys(&["a", "b", "c"]), now + PREVIEW_FOR));
    }

    #[test]
    fn another_token_desk_workspace_or_row_count_isnt_the_preview() {
        let now = Instant::now();
        let p = preview(now);
        let (ws, other, none) = (Some("ws".to_string()), Some("other".to_string()), None);
        // A token already used is gone from the slot, so only a different one is left to refuse.
        assert!(!live(&p, ("used", "desk", &ws), &keys(&["a", "b", "c"]), now));
        assert!(!live(&p, ("t", "elsewhere", &ws), &keys(&["a", "b", "c"]), now));
        assert!(!live(&p, ("t", "desk", &other), &keys(&["a", "b", "c"]), now));
        assert!(!live(&p, ("t", "desk", &none), &keys(&["a", "b", "c"]), now));
        // Rows added or removed by another client since the preview.
        // Removed, reordered, and one removed with another added.
        for changed in [["a", "b"].as_slice(), &["b", "a", "c"], &["a", "b", "d"], &["a", "c", "d", "b"]] {
            assert!(!live(&p, ("t", "desk", &ws), &keys(changed), now), "{changed:?}");
        }
    }

    #[test]
    fn take_picks_renumbers_and_collapses_duplicates() {
        let got = captured(&["a.exe", "b.exe", "c.exe"]);
        let subset = ticked_subset(&got, &[2, 0, 2], false).unwrap();
        let rows: Vec<(&str, u32)> = subset.rules.iter().map(|r| (r.exe.as_str(), r.z)).collect();
        assert_eq!(rows, [("a.exe", 0), ("c.exe", 1)]);
        assert_eq!(subset.hwnds, [100, 102]);
    }

    #[test]
    fn a_bad_take_is_refused() {
        let got = captured(&["a.exe"]);
        let out_of_range = ticked_subset(&got, &[1], true).err().map(|e| e.code);
        assert_eq!(out_of_range, Some(api::INVALID_PARAMS));
        // Nothing ticked saves nothing for a new workspace, and only keeps every row of an update.
        assert_eq!(ticked_subset(&got, &[], false).err().map(|e| e.code), Some(api::REFUSED));
        assert!(ticked_subset(&got, &[], true).unwrap().rules.is_empty());
    }

    #[test]
    fn unticked_windows_keep_their_app_open() {
        let got = captured(&["a.exe", "b.exe"]);
        let subset = ticked_subset(&got, &[0], true).unwrap();
        assert_eq!(subset.others.len(), 1);
        assert_eq!((subset.others[0].exe.as_str(), subset.others[0].hwnd), ("b.exe", 101));
        let old = Workspace { windows: vec![rule("a.exe"), rule("b.exe")], ..Workspace::default() };
        let none = std::collections::HashSet::new();
        assert!(not_open(&old, &subset.rules, &[], &none, &subset.others).is_empty());
        // Without them in `others` the second row would count as closed.
        assert_eq!(not_open(&old, &subset.rules, &[], &none, &[]), [1]);
    }
}
