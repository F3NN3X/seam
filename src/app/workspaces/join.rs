//! An app rule that sends new windows to a workspace (brief I): the windows the live worker
//! handed over join it one at a time, as a row with launch off, quietly. The decisions are
//! the pure functions here and in `workspace::join`.

use super::add::start_capture;
use super::capture::{check_rows, write_checked};
use super::*;
use std::collections::VecDeque;

/// Windows waiting to join, oldest first. Older ones are dropped past this many.
const MAX_QUEUED: usize = 32;

/// Put newly sent windows at the end of the queue. Returns how many old ones it dropped to
/// keep to `MAX_QUEUED`. A window already queued isn't queued twice.
fn enqueue(queue: &mut VecDeque<(isize, String)>, sent: Vec<(isize, String)>) -> usize {
    for item in sent {
        if !queue.iter().any(|(h, _)| *h == item.0) {
            queue.push_back(item);
        }
    }
    let dropped = queue.len().saturating_sub(MAX_QUEUED);
    queue.drain(..dropped);
    dropped
}

/// The first queued window that is still open, dropping those that closed before it.
fn next_open(queue: &mut VecDeque<(isize, String)>, is_open: impl Fn(isize) -> bool) -> Option<(isize, String)> {
    while let Some((hwnd, id)) = queue.pop_front() {
        if is_open(hwnd) {
            return Some((hwnd, id));
        }
    }
    None
}

/// The worker handed over windows: queue them and start the first.
pub(in crate::app) fn sent(app: &mut App) {
    let dropped = enqueue(&mut app.joining, live::take_sent());
    if dropped > 0 {
        log::info!("app rule: {dropped} windows waiting to join were dropped, the queue holds {MAX_QUEUED}");
    }
    next_join(app);
}

/// Start the next queued window, if nothing else is being captured or closed. A window whose
/// workspace isn't on this desk, or can't be read, is dropped and the one after it is tried.
pub(super) fn next_join(app: &mut App) {
    while app.capturing.is_none() && app.closing.is_none() {
        let open = |hwnd: isize| unsafe { IsWindow(Some(HWND(hwnd as *mut _))) }.as_bool();
        let Some((hwnd, id)) = next_open(&mut app.joining, open) else { return };
        let ids: Vec<String> = desk_workspaces(app).into_iter().map(|w| w.id).collect();
        if workspace::join_target(&id, &ids, app.workspace.as_deref()) == workspace::Join::Skip {
            log::info!("app rule: the workspace isn't on this desk, so a window doesn't join it");
            continue;
        }
        match start_capture(app, &id, hwnd, None, None, true) {
            Ok(()) => return,
            Err(e) => log::info!("app rule: a window didn't join ({})", e.message),
        }
    }
}

/// The worker has read a window that is joining. Only logs: a window opens too often for a notice.
pub(super) fn joined(app: &mut App, job: &Capturing, read: Result<(Rule, isize), String>) {
    match join(app, job, read) {
        Ok(what) => log::info!("app rule: {what}"),
        Err(why) => log::info!("app rule: a window didn't join ({why})"),
    }
}

fn join(app: &mut App, job: &Capturing, read: Result<(Rule, isize), String>) -> Result<String, String> {
    let (rule, hwnd) = read?;
    let Some(id) = job.into.as_deref() else { return Err("no workspace".into()) };
    if job.sig != app.sig {
        return Err("the desk changed".into());
    }
    let old = desk_workspaces(app).into_iter().find(|w| w.id == id).ok_or("the workspace is gone")?;
    let name = super::add::label(&rule);
    let ids: Vec<String> = desk_workspaces(app).into_iter().map(|w| w.id).collect();
    let placing = workspace::join_target(id, &ids, app.workspace.as_deref()) == workspace::Join::SaveAndPlace;
    let joined = workspace::join_window(&old, rule);
    if joined.new {
        // The editor's checks, then one write, as any add.
        let edit = check_rows(app, id, workspace::to_value(&joined.workspace), Some(joined.row), false)
            .map_err(|e| e.message)?;
        write_checked(app, &[edit]).map_err(|e| e.message)?;
        if let Some(place) = joined.place {
            super::add::store_place(app, id, joined.row, place);
        }
        let saved = desk_workspaces(app).into_iter().find(|w| w.id == id).ok_or("the workspace is gone")?;
        if app.cfg.workspaces_remember_places {
            switcher::bind_rows(&saved, &[(hwnd, joined.row)]);
        }
        live::stamp(workspace::stamp_key(id), vec![(hwnd, joined.row)], false);
        publish(app);
    }
    if placing {
        if joined.new {
            sync_live(app);
        }
        live::join(hwnd);
        live::check_drift();
    }
    let verb = if joined.new { "joined" } else { "is already in" };
    Ok(format!("{name} {verb} {}", old.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(from: isize, to: isize) -> Vec<(isize, String)> {
        (from..to).map(|h| (h, format!("w{h}"))).collect()
    }

    #[test]
    fn queue_keeps_order_drops_closed_windows_and_caps_at_32() {
        let mut queue = VecDeque::new();
        assert_eq!(enqueue(&mut queue, items(1, 4)), 0);
        // A window queued twice stays once, in its first place.
        assert_eq!(enqueue(&mut queue, items(3, 5)), 0);
        let order: Vec<isize> = queue.iter().map(|(h, _)| *h).collect();
        assert_eq!(order, [1, 2, 3, 4]);
        // Window 1 and 2 are closed, so the first open one is 3, and the others wait.
        assert_eq!(next_open(&mut queue, |h| h > 2), Some((3, "w3".into())));
        assert_eq!(queue.len(), 1);
        assert_eq!(next_open(&mut queue, |_| false), None);
        assert!(queue.is_empty());
        // Past 32 the oldest go, and the newest are kept in order.
        assert_eq!(enqueue(&mut queue, items(100, 135)), 3);
        assert_eq!(queue.len(), MAX_QUEUED);
        assert_eq!(queue.front().map(|(h, _)| *h), Some(103));
        assert_eq!(queue.back().map(|(h, _)| *h), Some(134));
    }
}
