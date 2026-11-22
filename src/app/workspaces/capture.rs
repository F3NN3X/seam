//! Capturing the windows on screen, quick save and its undo, and saving an edited workspace.

use super::*;

/// Start capturing the windows on screen. Returns an answer only if it can't start.
pub(in crate::app) fn start_capture(
    app: &mut App,
    name: Option<String>,
    into: Option<String>,
    row: Option<usize>,
    pick: Option<api::Pick>,
    reply: Option<Sender<Reply>>,
) -> Option<Reply> {
    begin_capture(app, (name, into, row, pick), reply, false)
}

/// `start_capture`; with `preview` the result is kept for the review instead of saved.
pub(super) fn begin_capture(
    app: &mut App,
    (name, into, row, pick): (Option<String>, Option<String>, Option<usize>, Option<api::Pick>),
    reply: Option<Sender<Reply>>,
    preview: bool,
) -> Option<Reply> {
    if app.capturing.is_some() {
        return Some(busy("Already capturing"));
    }
    if app.closing.is_some() {
        return Some(busy("Windows of a workspace are still being closed or minimized"));
    }
    if pick.is_some() && !app.cfg.workspaces_remember_places {
        return Some(refused("Remember folders and pages is off"));
    }
    let target = into.as_ref().map(|into| desk_workspaces(app).into_iter().find(|w| w.id == *into));
    if matches!(target, Some(None)) {
        return Some(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
    }
    if row.is_some_and(|row| target.flatten().is_none_or(|w| row >= w.windows.len())) {
        return Some(Err(RpcError::new(api::NOT_FOUND, "no such row in that workspace")));
    }
    let hwnd = pick.as_ref().map(|p| p.hwnd);
    // Telling the apps that aren't open from the capture needs the windows of the rest of the desk.
    let mut job = capture_job(app);
    job.removes |= preview && into.is_some();
    app.capturing = Some(Capturing {
        reply,
        name,
        into,
        row,
        pick,
        sig: app.sig.clone(),
        quick: false,
        add: None,
        move_from: None,
        preview,
        closed: BTreeMap::new(),
        from_panel: false,
    });
    let started = match hwnd {
        Some(hwnd) => switcher::capture_window(job, hwnd),
        None => switcher::capture(job),
    };
    if started {
        return None;
    }
    app.capturing = None;
    Some(refused("Couldn't start the capture"))
}

/// Quick save (§7): the same capture as "Update from screen", into the workspace in
/// use, or with none in use into a new "Workspace N" that becomes the one in use.
/// Returns an answer only if it can't start.
pub(in crate::app) fn quick_save(app: &mut App, reply: Option<Sender<Reply>>) -> Option<Reply> {
    let into = active_workspace(app).map(|w| w.id);
    let refusal = start_capture(app, None, into, None, None, reply);
    if let Some(job) = app.capturing.as_mut().filter(|_| refusal.is_none()) {
        job.quick = true;
    }
    refusal
}

/// A quick save went through: say so, and keep what Undo puts back. Undo is a
/// click on the notification, because a tray balloon can't carry a button.
pub(super) fn quick_saved(app: &mut App, saved: &Workspace, before: Option<Value>) {
    let created = before.is_none();
    let in_use = app.workspace.clone();
    if created {
        set_in_use(app, Some(saved.id.clone()));
    }
    sync_live(app);
    let until = Instant::now() + QUICK_UNDO_FOR;
    let after = workspace::to_value(saved);
    app.quick_undo = Some(QuickUndo {
        sig: app.sig.clone(),
        id: saved.id.clone(),
        before,
        after,
        from: None,
        in_use,
        until,
        source: None,
    });
    let n = saved.windows.len();
    let body = if created {
        format!("{n} windows saved in a new workspace, now in use. Click to undo")
    } else {
        format!("{n} windows saved. Click to undo")
    };
    log::info!("quick save: {n} windows into {}{}", saved.name, if created { ", created" } else { "" });
    notify(app, &format!("Saved {}", saved.name), &body, BalloonAction::UndoQuickSave);
}

/// Update from screen removed or turned off rows of apps that weren't open: say so, and
/// keep the workspace as it was for Undo, which is a click on the notification as with a
/// quick save. `from` says which old row each row now is.
pub(super) fn update_removed(
    app: &mut App,
    saved: &Workspace,
    before: Option<Value>,
    from: Vec<Option<usize>>,
    (removed, turned_off): (usize, usize),
) {
    let until = Instant::now() + QUICK_UNDO_FOR;
    let in_use = app.workspace.clone();
    let after = workspace::to_value(saved);
    app.quick_undo = Some(QuickUndo {
        sig: app.sig.clone(),
        id: saved.id.clone(),
        before,
        after,
        from: Some(from),
        in_use,
        until,
        source: None,
    });
    let body = format!("{}. Click to undo", workspace::review::closed_note(removed, turned_off));
    log::info!("update from screen: {removed} removed and {turned_off} turned off in {}", saved.name);
    notify(app, &format!("Updated {}", saved.name), &body, BalloonAction::UndoQuickSave);
}

/// The quick save's notification was clicked: put the workspace back as it was, or
/// remove the one it created, while that's still within 8 s.
pub(in crate::app) fn undo_quick_save(app: &mut App) {
    let Some(undo) = app.quick_undo.take() else { return };
    if Instant::now() > undo.until {
        log::info!("quick save: too late to undo");
        return;
    }
    let Some(desk) = app.cfg.desks.get_mut(&undo.sig) else { return };
    let same = |v: &Value| workspace::parse(v).is_some_and(|w| w.id == undo.id);
    // Changed since the save, by an edit or another save: putting the old one back would lose that.
    if !desk.workspaces.iter().any(|v| same(v) && *v == undo.after) {
        log::info!("quick save: not undone, the workspace changed since");
        return;
    }
    // A move's other workspace comes back too, unless it changed since.
    let source = undo.source.filter(|s| desk.workspaces.contains(&s.after));
    if let Some(s) = &source
        && let Some(slot) = desk.workspaces.iter_mut().find(|v| **v == s.after)
    {
        *slot = s.before.clone();
    }
    match undo.before {
        Some(before) => {
            if let Some(slot) = desk.workspaces.iter_mut().find(|v| same(v)) {
                *slot = before;
            }
        }
        None => {
            desk.workspaces.retain(|v| !same(v));
            if app.workspace.as_deref() == Some(undo.id.as_str()) {
                set_in_use(app, undo.in_use);
            }
        }
    }
    log::info!("quick save: undone");
    persist(app);
    // Rows an Update from screen removed come back, so the others move again.
    if let Some(from) = undo.from {
        switcher::rebind(&undo.id, &from);
        live::restamp(workspace::stamp_key(&undo.id), from);
    }
    if let Some(s) = source {
        switcher::rebind(&s.id, &s.from);
        live::restamp(workspace::stamp_key(&s.id), s.from);
    }
    sync_live(app);
    publish(app);
}

/// Where Update from screen sent the rows it kept, which old row each new one was,
/// and how many it removed and turned off.
type Removal = (Vec<Option<usize>>, Vec<Option<usize>>, (usize, usize));

/// What saving a capture leaves for the steps after it.
#[derive(Default)]
struct Kept {
    /// The workspace as it was, for a quick save's Undo.
    before: Option<Value>,
    /// The row each captured window has now, when the save stamps them.
    rows: Option<Vec<usize>>,
    /// "Where it is now" stamps its one window.
    stamps: Option<Vec<(isize, usize)>>,
    /// Rows Update from screen removed or turned off: where the others went, and how many.
    removal: Option<Removal>,
}

/// A finished capture and what saving it reads.
struct Saving<'a> {
    job: &'a Capturing,
    hwnds: &'a [isize],
    others: &'a [workspace::Candidate],
    layouts: &'a [(String, Option<zones::ZoneLayout>)],
    remember: bool,
    /// What happens to the row of an app that isn't open, by row of the workspace.
    closed: &'a dyn Fn(usize) -> ClosedApps,
}

/// Update the workspace `into` from the capture, a row of it or all of it. The error
/// is the answer when that can't be done.
fn update_existing(
    saving: &Saving,
    quick_undo: &mut Option<QuickUndo>,
    workspaces: &mut [Value],
    old: &Workspace,
    into: &str,
    rules: Vec<Rule>,
    kept: &mut Kept,
) -> Result<Workspace, Reply> {
    let &Saving { job, hwnds, others, layouts, remember, closed } = saving;
    // A place row pairs with the window it remembers, as in a switch (D336).
    let (bound, elsewhere) = switcher::capture_bindings(old, hwnds, remember);
    let updated = match (job.row, &job.pick) {
        // "Add an app": only the picked window's folder or page, read here.
        (Some(row), Some(pick)) => match workspace::take_window_place(old, row, &pick.exe, &pick.class, &rules) {
            Some(updated) => updated,
            None => return Err(refused("That window has no folder or page to remember")),
        },
        (Some(row), None) => match workspace::recapture_row(old, row, &rules, &bound, &elsewhere) {
            Some((updated, hit)) => {
                kept.stamps = Some(vec![(hwnds[hit], row)]);
                updated
            }
            None => return Err(refused("That app isn't open, so where it goes stays as it was")),
        },
        (None, _) => {
            // An earlier save's Undo would put back rows this one may have changed.
            if quick_undo.as_ref().is_some_and(|u| u.id == *into) {
                *quick_undo = None;
            }
            let zones = workspace::capture_zones(&old.zones, layouts);
            let merged = workspace::merge_capture(old, rules, &bound, &elsewhere, others, closed);
            kept.rows = Some(merged.rows);
            if merged.removed + merged.turned_off > 0 {
                // Where each old row went, exactly: the rows it kept, in order.
                let mut moves = vec![None; old.windows.len()];
                for (new, old) in merged.from.iter().enumerate() {
                    if let Some(old) = old {
                        moves[*old] = Some(new);
                    }
                }
                kept.removal = Some((moves, merged.from, (merged.removed, merged.turned_off)));
            }
            Workspace { windows: merged.windows, zones, ..old.clone() }
        }
    };
    let same = |v: &&mut Value| workspace::parse(v).is_some_and(|w| w.id == *into);
    if let Some(slot) = workspaces.iter_mut().find(same) {
        kept.before = Some(std::mem::replace(slot, workspace::to_value(&updated)));
    }
    Ok(updated)
}

/// Save the capture as a new workspace of the desk.
fn create_workspace(
    saving: &Saving,
    existing: &[Workspace],
    workspaces: &mut Vec<Value>,
    rules: Vec<Rule>,
    kept: &mut Kept,
) -> Workspace {
    let &Saving { job, layouts, .. } = saving;
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let name = match job.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => workspace::unique_name(name, &names),
        _ => workspace::next_name(&names),
    };
    if job.quick {
        kept.rows = Some((0..rules.len()).collect());
    }
    let zones = workspace::capture_zones(&Default::default(), layouts);
    let created = Workspace { id: workspace::new_id(&ids), name, windows: rules, zones, ..Workspace::default() };
    workspaces.push(workspace::to_value(&created));
    created
}

/// The saved windows hold their rows, as after a switch, so drift and auto-apply see them at once.
fn stamp_saved(
    job: &Capturing,
    saved: &Workspace,
    remember: bool,
    hwnds: &[isize],
    rows: Option<Vec<usize>>,
    stamps: Option<Vec<(isize, usize)>>,
) {
    // A row with a place remembers the window it was read from (D336).
    if remember {
        let binds: Vec<(isize, usize)> = match (&job.pick, job.row, &rows, &stamps) {
            (Some(pick), Some(row), _, _) => vec![(pick.hwnd, row)],
            (_, _, Some(rows), _) => hwnds.iter().copied().zip(rows.iter().copied()).collect(),
            (_, _, _, Some(windows)) => windows.clone(),
            _ if job.into.is_none() => hwnds.iter().copied().zip(0..).collect(),
            _ => Vec::new(),
        };
        switcher::bind_rows(saved, &binds);
    }
    let key = workspace::stamp_key(&saved.id);
    if let Some(rows) = rows {
        live::stamp(key, hwnds.iter().copied().zip(rows).collect(), true);
    } else if let Some(windows) = stamps {
        // The drift check comes below, once the worker has the updated workspace.
        live::stamp(key, windows, false);
    }
}

/// Say how the save went: the answer for the API client if there is one, else a notice.
fn finish_capture(
    app: &mut App,
    job: &Capturing,
    saved: &Workspace,
    before: Option<Value>,
    removal: Option<Removal>,
) -> Option<Reply> {
    if job.quick {
        let created = before.is_none();
        quick_saved(app, saved, before);
        live::check_drift();
        publish(app);
        return Some(Ok(json!({ "id": saved.id, "created": created })));
    }
    if app.workspace.as_deref() == Some(saved.id.as_str()) {
        sync_live(app);
        live::check_drift();
    }
    publish(app);
    // The Seam window offers its own Undo, so only a call from anywhere else raises the balloon (D727).
    if let (Some((_, from, counts)), false) = (removal, job.from_panel) {
        update_removed(app, saved, before, from, counts);
    }
    if job.reply.is_some() {
        return Some(Ok(json!({ "id": saved.id })));
    }
    let body = format!("{} windows saved as {}", saved.windows.len(), saved.name);
    notify(app, "Workspace captured", &body, BalloonAction::OpenPanel("workspaces"));
    None
}

/// A capture finished: save it as a new workspace, or over the one it was asked to update.
pub(in crate::app) fn captured(app: &mut App) {
    if app.capturing.as_ref().is_some_and(|job| job.add.is_some()) {
        return super::add::added(app);
    }
    let (Some(got), Some(job)) = (switcher::take_captured(), app.capturing.take()) else { return };
    if job.preview {
        return super::review::previewed(app, job, got);
    }
    save_capture(app, job, got);
}

/// Save a capture, or the ticked part of a preview, as a new workspace or over `job.into`.
pub(super) fn save_capture(app: &mut App, job: Capturing, got: switcher::Captured) {
    let switcher::Captured { rules, hwnds, others, .. } = got;
    let answer = |r: Reply| {
        if let Some(tx) = &job.reply {
            let _ = tx.send(r);
        }
    };
    let remember = app.cfg.workspaces_remember_places;
    // Only Update from screen removes or turns off rows. A quick save goes through the
    // same capture, so it is told apart here.
    let default = if job.quick { ClosedApps::Keep } else { app.cfg.closed_apps() };
    let closed = |row: usize| job.closed.get(&row).copied().unwrap_or(default);
    let Some(desk) = app.cfg.desks.get_mut(&job.sig) else { return answer(refused("That desk is gone")) };
    // Every screen of the desk, with the layout it has now.
    let layouts: Vec<(String, Option<zones::ZoneLayout>)> =
        job.sig.split('|').map(|id| (id.to_string(), desk.zones.get(id).and_then(zones::layout_of))).collect();
    let existing = workspace::list(&desk.workspaces);
    let mut kept = Kept::default();
    let saving = Saving { job: &job, hwnds: &hwnds, others: &others, layouts: &layouts, remember, closed: &closed };
    let saved = match &job.into {
        Some(into) => {
            let Some(old) = existing.iter().find(|w| w.id == *into) else {
                return answer(Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")));
            };
            match update_existing(&saving, &mut app.quick_undo, &mut desk.workspaces, old, into, rules, &mut kept) {
                Ok(updated) => updated,
                Err(reply) => return answer(reply),
            }
        }
        None => create_workspace(&saving, &existing, &mut desk.workspaces, rules, &mut kept),
    };
    persist(app);
    // The rows left follow their places in the list, as after an edit (D336).
    if let Some((moves, ..)) = &kept.removal {
        switcher::rebind(&saved.id, moves);
    }
    stamp_saved(&job, &saved, app.cfg.workspaces_remember_places, &hwnds, kept.rows, kept.stamps);
    if let Some(reply) = finish_capture(app, &job, &saved, kept.before, kept.removal) {
        answer(reply);
    }
}

/// The editor saved a workspace of this desk. It's checked as `checked_update`
/// says, then saved and applied through the same path as `config.patch`.
pub(in crate::app) fn update_workspace(app: &mut App, id: &str, sent: Value) -> Reply {
    update_rows(app, id, sent, None, false)
}

/// `update_workspace` for the Seam window's own save, the one way a startup command is typed: a
/// command in clear is sealed (D736). Anyone else's save, and an add, seals none.
pub(in crate::app) fn update_from_panel(app: &mut App, id: &str, sent: Value) -> Reply {
    update_rows(app, id, sent, None, true)
}

/// `update_workspace`; with `only` set, the Open with of the other rows isn't checked again.
pub(super) fn update_rows(app: &mut App, id: &str, sent: Value, only: Option<usize>, sealing: bool) -> Reply {
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return refused("This desk has no workspaces") };
    let Some(old) = workspace::list(&desk.workspaces).into_iter().find(|w| w.id == id) else {
        return Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk"));
    };
    // A client that doesn't know `zones` leaves the screen layouts as they were.
    let keeps_zones = sent.get("zones").is_none();
    let mut sent: Workspace = match serde_json::from_value(sent) {
        Ok(ws) => ws,
        Err(e) => return refused(format!("That isn't a workspace: {e}")),
    };
    if keeps_zones {
        sent.zones = old.zones.clone();
    }
    // A clear command is only the panel's own, so no other save seals one (D736).
    if !sealing {
        workspace::without_clear_commands(&mut sent);
    }
    let zones = |monitor: &str| {
        let layout = desk.zones.get(monitor).and_then(zones::layout_of);
        layout.map_or(0, |l| zones::leaf_count(&l.tree))
    };
    let checked = match workspace::checked_rows(&old, sent, &zones, only) {
        Ok(ws) => ws,
        Err(why) => return refused(why),
    };
    let others: Vec<String> =
        workspace::list(&desk.workspaces).into_iter().filter(|w| w.id != id).map(|w| w.name).collect();
    let mut checked = Workspace { name: workspace::unique_name(&checked.name, &others), ..checked };
    // A startup command is saved sealed for this account, and only ever from the panel (D736).
    if let Err(why) = workspace::seal_commands(&mut checked, &seam::seal::seal) {
        return refused(why);
    }
    let list: Vec<Value> = desk
        .workspaces
        .iter()
        .map(
            |v| if workspace::parse(v).is_some_and(|w| w.id == id) { workspace::to_value(&checked) } else { v.clone() },
        )
        .collect();
    let patch = json!({ "desks": { app.sig.clone(): { "workspaces": list } } });
    let moves = workspace::moved_rows(&old.windows, &checked.windows);
    let reply = patch_config(app, &patch);
    // Windows a switch or auto-apply stamped keep the app they were stamped for. The
    // worker already has the new rows by now, so a window it stamps in between is
    // remapped from the wrong row. That needs a window to open in those microseconds,
    // and the next switch stamps everything again.
    let unmoved = moves.len() == checked.windows.len() && moves.iter().enumerate().all(|(i, m)| *m == Some(i));
    if reply.is_ok() && !unmoved {
        // The windows place rows remember follow their rows the same way (D336).
        switcher::rebind(id, &moves);
        live::restamp(workspace::stamp_key(id), moves.clone());
    }
    // An item that was edited isn't what its window opened, so the next switch starts the new one (D733).
    if reply.is_ok() {
        switcher::forget_edited(id, &workspace::edited_items(&old.windows, &checked.windows, &moves));
    }
    reply
}
