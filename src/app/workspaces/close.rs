//! Close this workspace and Minimize this workspace (D701 to D709): the refusals, the
//! worker, and what each way of asking hears back. Closing is a close request only, and
//! never reaches the assistants.

use super::*;
use crate::switcher::{CloseJob, Done, Owned, Verb};

/// What a job running on the worker is for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Ask {
    /// Read the list. With no client waiting, it is the tray or a shortcut starting Close,
    /// which opens the Seam window with the dialog.
    List,
    Close,
    Minimize,
}

/// A job running on the worker.
pub(in crate::app) struct Closing {
    /// The API client waiting for the answer. A tray item or a shortcut has none.
    reply: Option<Sender<Reply>>,
    /// Who asked, for the log.
    client: String,
    ask: Ask,
    ws: Workspace,
}

/// The windows the last minimize took, topmost first, while Undo can still put them back.
pub(in crate::app) struct Minimized {
    windows: Vec<isize>,
    until: Instant,
}

/// Start listing, closing or minimizing the windows of a workspace of this desk, the one in
/// use on `on`'s screen when `id` is `None` (D785). Returns an answer only if it can't start;
/// the worker's end is `closed`.
pub(in crate::app) fn start_close(
    app: &mut App,
    (id, on): (Option<&str>, OnScreen),
    (ask, handles): (Ask, Vec<isize>),
    (reply, client): (Option<Sender<Reply>>, &str),
) -> Option<Reply> {
    if app.closing.is_some() || app.capturing.is_some() || crate::switcher::switching() {
        return Some(busy("A workspace switch or capture is still running"));
    }
    let screen = match screen_meant(app, on) {
        Ok(screen) => screen,
        Err(e) => return Some(Err(e)),
    };
    let wanted = id.map(str::to_string).or_else(|| meant(app, screen.as_deref()));
    let found = wanted.and_then(|wanted| desk_workspaces(app).into_iter().find(|w| w.id == wanted));
    let Some(ws) = found else {
        return Some(match id {
            Some(_) => Err(RpcError::new(api::NOT_FOUND, "no such workspace on this desk")),
            None => refused("No workspace in use"),
        });
    };
    let verb = match ask {
        Ask::List => Verb::List,
        Ask::Close => Verb::Close(handles),
        Ask::Minimize => Verb::Minimize,
    };
    let exclude = app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    let (screens, held) = reach_of(app, &ws);
    let job = CloseJob {
        owner: app.hwnd.0 as isize,
        ws: ws.clone(),
        exclude,
        places: app.cfg.workspaces_remember_places,
        verb,
        screens,
        held,
    };
    app.closing = Some(Closing { reply, client: client.to_string(), ask, ws });
    if crate::switcher::start_close(job) {
        return None;
    }
    app.closing = None;
    Some(refused("Couldn't start"))
}

/// Where a close or minimize of `ws` reaches (D786): the screens where it is in use, or while it
/// isn't, its screen or every screen (empty, as before one-screen workspaces). Never a window
/// another workspace in use holds.
fn reach_of(app: &App, ws: &Workspace) -> (Vec<zones::Rect>, std::collections::HashSet<isize>) {
    let ids = screen_ids(app);
    let held = app.cfg.desks.get(&app.sig).map(|d| workspace::holds(d, &ids)).unwrap_or_default();
    let mine: Vec<&str> = ids.iter().copied().filter(|s| held.get(*s) == Some(&ws.id)).collect();
    let reach =
        |s: &str| if mine.is_empty() { ws.scope.as_deref().is_none_or(|own| own == s) } else { mine.contains(&s) };
    let rect = |m: &Mon| zones::Rect { x: m.rect.left, y: m.rect.top, w: m.rect.width(), h: m.rect.height() };
    let screens = if ids.iter().all(|s| reach(s)) {
        Vec::new()
    } else {
        app.mons.iter().filter(|m| reach(&m.id)).map(rect).collect()
    };
    (screens, switcher::held_windows(&holders(app, &ws.id, &|_| false)))
}

/// A tray item or a shortcut: say why it did nothing, in a notice.
pub(in crate::app) fn start_close_here(app: &mut App, title: &str, ask: Ask, on: OnScreen) {
    if let Some(Err(e)) = start_close(app, (None, on), (ask, Vec::new()), (None, "the tray")) {
        log::info!("{}: {}", title.to_lowercase(), e.message);
        notify(app, title, &e.message, BalloonAction::None);
    }
}

/// The worker finished: answer the client, or tell the user.
/// Then a window an app rule sent to a workspace may have its turn.
pub(in crate::app) fn closed(app: &mut App) {
    ended(app);
    super::join::next_join(app);
}

fn ended(app: &mut App) {
    let (Some(done), Some(job)) = (crate::switcher::take_done(), app.closing.take()) else { return };
    let Closing { reply, client, ask, ws } = job;
    let name = &ws.name;
    let answer = match done {
        Done::Listed(owned) => {
            if let Some(tx) = &reply {
                let _ = tx.send(Ok(listed(app, &ws, &owned)));
            } else if ask == Ask::List {
                if owned.take.is_empty() {
                    notify(app, "Close this workspace", &format!("Nothing of {name} is open"), BalloonAction::None);
                } else {
                    open_panel_closing(app, &ws.id);
                }
            }
            return;
        }
        Done::Closed { asked, changed } => {
            log::info!("workspace close: {asked} of {} by {}", ws.id, client);
            api_server::broadcast("workspace.closed", &json!({ "id": ws.id, "asked": asked }));
            json!({ "asked": asked, "changed": changed })
        }
        Done::Minimized(windows) => {
            let n = windows.len();
            log::info!("workspace minimize: {n} of {} by {}", ws.id, client);
            api_server::broadcast("workspace.minimized", &json!({ "id": ws.id, "minimized": n }));
            if reply.is_none() {
                let body = match n {
                    0 => format!("Nothing of {name} to minimize"),
                    1 => format!("Minimized 1 window of {name}. Click to undo"),
                    n => format!("Minimized {n} windows of {name}. Click to undo"),
                };
                let click = if n == 0 { BalloonAction::None } else { BalloonAction::UndoMinimize };
                notify(app, "Minimize this workspace", &body, click);
            }
            if n > 0 {
                app.minimized = Some(Minimized { windows, until: Instant::now() + QUICK_UNDO_FOR });
            }
            json!({ "minimized": n })
        }
    };
    if let Some(tx) = reply {
        let _ = tx.send(Ok(answer));
    }
}

/// `workspace.windows`: what a close would take and what it would leave.
fn listed(app: &App, ws: &Workspace, owned: &Owned) -> Value {
    let screen = |row: usize| app.mons.iter().position(|m| m.id == ws.windows[row].monitor).map(|i| i + 1);
    let windows: Vec<Value> = owned
        .take
        .iter()
        .map(|e| json!({ "hwnd": e.hwnd, "row": e.row, "app": e.app, "title": e.title, "screen": screen(e.row) }))
        .collect();
    let kept: Vec<Value> =
        owned.kept.iter().map(|(e, why)| json!({ "app": e.app, "title": e.title, "why": why.word() })).collect();
    json!({ "windows": windows, "kept": kept })
}

/// The minimize's notification was clicked, or the Seam window's Undo: restore what it
/// minimized, while that's still within 8 s.
pub(in crate::app) fn undo_minimize(app: &mut App) -> bool {
    let Some(undo) = app.minimized.take() else { return false };
    if Instant::now() > undo.until {
        log::info!("workspace minimize: too late to undo");
        return false;
    }
    crate::switcher::restore(&undo.windows);
    log::info!("workspace minimize: undone");
    true
}
