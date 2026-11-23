//! Wallpapers on the tray's side (brief K): what each screen is to show, worked out here (D793)
//! and handed to the `seam-wallpaper` worker, which does every file and Explorer call (D795),
//! and what comes back from it.

use super::*;
use seam::wallpaper::worker::{self, Job, Original, Shown, Want, Worker};
use seam::wallpaper::{self, Fit, Wallpaper};
use std::sync::mpsc::Receiver;

/// The tray's side of the wallpapers.
pub(super) struct Wallpapers {
    worker: Option<Worker>,
    /// What was last handed to the worker, so the same again is skipped.
    sent: Option<Vec<Want>>,
    /// The switch that is running, `(id, scope, away)`: its pictures are up while its windows move
    /// (D794). `away` are the screens it leaves to their own workspace.
    switching: Option<(String, Option<String>, Vec<String>)>,
    /// What each screen shows, from the worker's last Apply.
    shown: Vec<Shown>,
    original: Original,
    /// Someone else changed the wallpaper: Seam leaves it until the next switch or screen change (D799).
    held: bool,
    /// A quit asked for mine back: nothing is put up after it.
    quitting: bool,
}

impl Wallpapers {
    /// Start the worker. It looks at the journal an earlier run left before anything else (D798).
    pub(super) fn start(owner: isize, enabled: bool) -> Wallpapers {
        let worker = Worker::start(owner, enabled);
        if worker.is_none() {
            log::info!("wallpaper: the worker didn't start");
        }
        Wallpapers {
            worker,
            sent: None,
            switching: None,
            shown: Vec::new(),
            original: Original::default(),
            held: false,
            quitting: false,
        }
    }

    fn send(&self, job: Job) {
        if let Some(worker) = &self.worker {
            worker.send(job);
        }
    }
}

/// What each connected screen can show, best first, on `desk`.
fn wants(app: &App, desk: &seam::config::Desk) -> Vec<Want> {
    let each = app.mons.iter().enumerate().map(|(i, m)| Want {
        screen: worker::Screen { id: m.id.clone(), name: m.name.clone(), number: i + 1, rect: m.rect },
        choices: wallpaper::choices(desk, &m.id),
    });
    each.collect()
}

/// Hand the worker what each screen is to show, unless it is what it already has. With `switch`,
/// as if that switch had ended, so the pictures change while its windows move: a one-screen
/// switch changes its screen only, and one that leaves `away` screens leaves their pictures (D794).
/// Off, nothing is touched.
pub(super) fn wallpaper_sync(app: &mut App, switch: Option<(&Workspace, &[String])>) {
    if let Some((ws, away)) = switch.filter(|(ws, _)| ws.id != SESSION_ID) {
        app.wallpaper.switching = Some((ws.id.clone(), ws.scope.clone(), away.to_vec()));
        app.wallpaper.held = false;
    }
    if !app.cfg.wallpapers_enabled || app.wallpaper.held || app.wallpaper.quitting {
        return;
    }
    let Some(desk) = app.cfg.desks.get(&app.sig) else { return };
    let ids: Vec<&str> = app.mons.iter().map(|m| m.id.as_str()).collect();
    let wants = match &app.wallpaper.switching {
        Some((id, scope, away)) => wants(app, &wallpaper::after_switch(desk, id, scope.as_deref(), &ids, away)),
        None => wants(app, desk),
    };
    if app.wallpaper.sent.as_ref() == Some(&wants) {
        return;
    }
    app.wallpaper.sent = Some(wants.clone());
    app.wallpaper.send(Job::Apply(wants));
}

/// The switch to `id` ended, done or cancelled: what is in use now decides. The late end of a
/// switch a newer one replaced leaves the newer one's pictures up.
pub(super) fn wallpaper_switched(app: &mut App, id: &str) {
    if app.wallpaper.switching.as_ref().is_some_and(|(running, ..)| running == id) {
        app.wallpaper.switching = None;
    }
    wallpaper_sync(app, None);
}

/// The screens changed, or Explorer restarted: every screen is looked at again, and a change
/// someone else made is Seam's to replace now (D799).
pub(super) fn wallpaper_screens(app: &mut App, forget: bool) {
    if forget {
        app.wallpaper.send(Job::Forget);
    }
    // What failed on the screens before may work now.
    app.wallpaper.send(Job::Retry(None));
    app.wallpaper.sent = None;
    app.wallpaper.held = false;
    wallpaper_sync(app, None);
}

/// "Change wallpaper with workspaces" changed: off puts mine back, on puts the pictures up.
pub(super) fn wallpaper_turned(app: &mut App, was: bool) {
    match (was, app.cfg.wallpapers_enabled) {
        (true, false) => {
            log::info!("wallpaper: turned off");
            app.wallpaper.send(Job::Restore(None));
            app.wallpaper.sent = None;
            app.wallpaper.shown.clear();
        }
        (false, true) => {
            log::info!("wallpaper: turned on");
            wallpaper_screens(app, false);
        }
        _ => wallpaper_sync(app, None),
    }
}

/// Windows says the wallpaper changed: the worker looks whether it was someone else (D799).
pub(super) fn wallpaper_changed_outside(app: &App) {
    if app.cfg.wallpapers_enabled {
        app.wallpaper.send(Job::Check);
    }
}

/// A quit I asked for puts mine back, within the quit's budget (D798). The answer is waited for
/// later; without one in time the journal stays for the next start.
pub(super) fn wallpaper_quit(app: &mut App) -> Option<Receiver<bool>> {
    if !app.cfg.wallpapers_enabled {
        return None;
    }
    let (tx, rx) = channel();
    app.wallpaper.quitting = true;
    app.wallpaper.send(Job::Restore(Some(tx)));
    Some(rx)
}

/// `wallpaper.set`: the picture a workspace, or with none the desk default, sets for one screen,
/// or none. The file's type and size are read from its header here, and it is decoded on the worker.
pub(super) fn wallpaper_set(
    app: &mut App,
    workspace: Option<String>,
    screen: String,
    path: Option<String>,
    fit: Option<Fit>,
) -> Reply {
    let list = desk_workspaces(app);
    let ws = match &workspace {
        Some(id) => Some(
            list.iter()
                .find(|w| w.id == *id)
                .ok_or_else(|| RpcError::new(api::NOT_FOUND, "no such workspace on this desk"))?,
        ),
        None => None,
    };
    let connected = app.mons.iter().position(|m| m.id == screen);
    // A one-screen workspace whose screen isn't connected keeps its choices (J D787).
    if connected.is_none() && ws.and_then(|w| w.scope.as_deref()) != Some(screen.as_str()) {
        return Err(RpcError::new(api::NOT_FOUND, "no such screen"));
    }
    wallpaper::may_set(ws, &screen).map_err(|why| RpcError::new(api::REFUSED, why))?;
    if let Some(path) = &path {
        if !std::path::Path::new(path).is_absolute() {
            return refused(wallpaper::UNREADABLE);
        }
        wallpaper::read_header(path).map_err(|why| RpcError::new(api::REFUSED, why))?;
    }
    // Only a choice the screen shows now ends a hold after a change made elsewhere (D799).
    let shows_now = app.cfg.desks.get(&app.sig).is_some_and(|d| wallpaper::shows_now(d, workspace.as_deref(), &screen));
    let Some(desk) = app.cfg.desks.get_mut(&app.sig) else { return refused("This desk isn't saved yet") };
    let set = match &workspace {
        Some(id) => desk.workspace_wallpapers.entry(id.clone()).or_default(),
        None => &mut desk.wallpaper,
    };
    let picture = path.map(|path| {
        let fit = fit.or_else(|| set.screens.get(&screen).map(|w| w.fit)).unwrap_or_default();
        Wallpaper { path, fit }
    });
    match &picture {
        Some(picture) => set.screens.insert(screen.clone(), picture.clone()),
        None => set.screens.remove(&screen),
    };
    desk.workspace_wallpapers.retain(|_, set| !set.is_empty());
    persist(app);
    if shows_now {
        app.wallpaper.held = false;
    }
    // A new choice for the screen tries again what failed there.
    app.wallpaper.send(Job::Retry(Some(screen.clone())));
    // Rendered now, so the first switch to it finds it in the cache (D795).
    if let (Some(picture), Some(i)) = (picture, connected) {
        let m = &app.mons[i];
        let on = worker::Screen { id: m.id.clone(), name: m.name.clone(), number: i + 1, rect: m.rect };
        app.wallpaper.send(Job::Render(on, picture));
    }
    wallpaper_sync(app, None);
    publish(app);
    Ok(json!({ "ok": true }))
}

fn origin(from: wallpaper::Origin) -> Value {
    serde_json::to_value(from).unwrap_or_default()
}

fn screens_json(shown: &[Shown]) -> Value {
    let each = shown
        .iter()
        .map(|s| json!({ "id": s.screen, "path": s.path, "fit": s.fit, "from": origin(s.from), "missing": s.missing }));
    Value::Array(each.collect())
}

/// `wallpaper.state`: what each connected screen shows and where it comes from, and what mine is.
pub(super) fn wallpaper_state(app: &App) -> Value {
    let on = app.cfg.wallpapers_enabled;
    let shown: Vec<Shown> = app
        .mons
        .iter()
        .map(|m| {
            let found = app.wallpaper.shown.iter().find(|s| s.screen == m.id).filter(|_| on);
            found.cloned().unwrap_or(Shown {
                screen: m.id.clone(),
                path: String::new(),
                fit: Fit::Fill,
                from: wallpaper::Origin::Own,
                missing: false,
            })
        })
        .collect();
    let Original { spotlight, slideshow } = app.wallpaper.original;
    json!({
        "enabled": on,
        "screens": screens_json(&shown),
        "original": { "spotlight": spotlight, "slideshow": slideshow },
    })
}

/// The worker has news: what the screens show, what failed, whether mine went back.
pub(super) fn wallpaper_reported(app: &mut App) {
    for report in worker::take_reports() {
        app.wallpaper.original = report.original;
        if report.outside {
            app.wallpaper.held = true;
            app.wallpaper.sent = None;
        }
        let Original { spotlight, slideshow } = report.original;
        let original = json!({ "spotlight": spotlight, "slideshow": slideshow });
        for failed in &report.failed {
            let data = json!({ "screen": failed.screen, "message": failed.message, "path": failed.path });
            api_server::broadcast("wallpaper.failed", &data);
        }
        if let Some(failed) = report.failed.iter().find(|f| f.at_apply) {
            let body = format!("Couldn't change the wallpaper on Screen {}", failed.number);
            notify(app, "Seam", &body, BalloonAction::None);
        }
        let applied = report.shown.is_some();
        if let Some(shown) = report.shown {
            app.wallpaper.shown = shown;
        }
        if applied || report.restored || report.outside {
            let data = json!({
                "screens": screens_json(&app.wallpaper.shown),
                "restored": report.restored,
                "outside": report.outside,
                "original": original,
            });
            api_server::broadcast("wallpaper.changed", &data);
        }
    }
}
