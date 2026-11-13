//! Shares and Take out on the main thread: `window.share`, `window.ungroup`, the
//! size menu key, and a choice from a divider handle's menu or the size menu,
//! which the handles' own thread hands over here.

use super::*;
use crate::dividers::{self, Choice};

/// Queue a share for a window, or the focused one. The worker answers.
pub(super) fn start_share(app: &App, hwnd: Option<isize>, verb: GroupVerb, reply: Option<Sender<Reply>>) {
    if let Some(why) = aside(app) {
        log::info!("window {verb:?}: {why}");
        if let Some(tx) = reply {
            let _ = tx.send(refused(why));
        }
        return;
    }
    let hwnd = hwnd.map_or_else(|| unsafe { GetForegroundWindow() }, |h| HWND(h as *mut _));
    let desk = app.cfg.desks.get(&app.sig).filter(|_| app.cfg.zones.enabled);
    let layout = |m: &Mon| desk.and_then(|d| d.zones.get(&m.id)).and_then(zones::layout_of);
    wincontrol::start(wincontrol::Job {
        owner: app.hwnd.0 as isize,
        task: wincontrol::Task::Group(verb),
        hwnd: hwnd.0 as isize,
        snapped: snap::zone_of(hwnd),
        members: snap::zoned(),
        mons: app.mons.clone(),
        links: app.links.clone(),
        layouts: app.mons.iter().map(layout).collect(),
        gap: app.cfg.zones.gap,
        exclude: app.cfg.zones.exclude.iter().map(|x| x.to_lowercase()).collect(),
        size_mode: app.cfg.window_size_on_move,
        from_api: reply.is_some(),
        reply,
    });
}

/// Take a window, or the focused one, out of its zones.
pub(super) fn take_out(hwnd: Option<isize>) -> Reply {
    let hwnd = hwnd.map_or_else(|| unsafe { GetForegroundWindow() }, |h| HWND(h as *mut _));
    match snap::take_out(hwnd) {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(why) => {
            log::info!("take out: {why}");
            refused(why)
        }
    }
}

/// A choice from a divider handle's menu or the size menu.
pub(super) fn divider_choice(app: &mut App) {
    match dividers::take_choice() {
        Some(Choice::Share(hwnd, verb)) => start_share(app, Some(hwnd), verb, None),
        Some(Choice::TakeOut(hwnd)) => {
            let _ = take_out(Some(hwnd));
        }
        None => {}
    }
}
