//! Talking to the tray: the link, calls, config patches and undo.

use super::*;
use seam::api::Event;
use seam::config::diff_patch;
use seam::monitors;
use serde_json::Value;
use std::sync::mpsc::Receiver;

impl Panel {
    /// Ask Windows about the startup task. It blocks, so it runs off the UI thread.
    pub(super) fn refresh_startup(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx.background_executor().spawn(async { package::startup_state() }).await;
            let _ = this.update(cx, |this, cx| {
                this.startup = state;
                cx.notify();
            });
        })
        .detach();
    }

    /// Turn the Store copy's startup task on or off.
    pub fn set_startup(&self, on: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx.background_executor().spawn(async move { package::set_startup(on) }).await;
            let _ = this.update(cx, |this, cx| {
                this.startup = state.or(this.startup);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn bind_keys(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("ctrl-f", FocusSearch, Some(CONTEXT)),
            KeyBinding::new("escape", ClearSearch, Some(CONTEXT)),
        ]);
        pages::zone_editor::bind_keys(cx);
    }

    /// Stay connected, and redraw from events. There's no polling.
    pub(super) fn link(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let token = self.token.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let session = {
                    let token = token.clone();
                    cx.background_executor().spawn(async move { connect(&token) }).await
                };
                let Ok((client, events, state, config)) = session else {
                    let linked = this.update(cx, |this, cx| {
                        this.client = None;
                        this.state = None;
                        cx.notify();
                    });
                    if linked.is_err() {
                        return;
                    }
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    continue;
                };
                let client = Arc::new(Mutex::new(client));
                let accepted = this.update(cx, |this, cx| {
                    // A new tray has a new token, so this panel can't change settings.
                    // Ask for a fresh one.
                    if this.was_linked {
                        crate::restart_panel(cx);
                        return false;
                    }
                    this.was_linked = true;
                    this.client = Some(client.clone());
                    this.apply(state, config);
                    cx.notify();
                    true
                });
                if !matches!(accepted, Ok(true)) {
                    return;
                }
                let events = Arc::new(Mutex::new(events));
                loop {
                    let next = {
                        let events = events.clone();
                        cx.background_executor().spawn(async move { next_event(&events) }).await
                    };
                    let Some(event) = next else { break };
                    let fresh = {
                        let client = client.clone();
                        cx.background_executor()
                            .spawn(async move { client.lock().ok()?.call("config.get", json!({})).ok() })
                            .await
                    };
                    let updated = this.update_in(cx, |this, window, cx| {
                        if event.kind == "panel.show"
                            && let Some(page) = event.data["page"].as_str().and_then(Page::from_arg)
                        {
                            this.show(page, None, window, cx);
                        }
                        // A moved window of another workspace changes nothing on show.
                        let moved_here = event.kind == "workspace.moved"
                            && this.status.as_ref().is_some_and(|(id, _)| event.data["id"].as_str() == Some(id));
                        let restatus =
                            moved_here || matches!(event.kind.as_str(), "state.changed" | "workspace.switched");
                        if event.kind == "state.changed"
                            && let (Ok(state), Some(config)) = (serde_json::from_value(event.data), fresh)
                        {
                            this.apply(state, config);
                        }
                        if restatus {
                            this.ask_status(cx);
                        }
                        cx.notify();
                    });
                    if updated.is_err() {
                        return;
                    }
                }
            }
        })
    }

    fn apply(&mut self, state: State, config: Value) {
        let screens_changed = self.state.as_ref().map(|s| &s.desk.sig) != Some(&state.desk.sig);
        if let Ok(config) = serde_json::from_value(config) {
            self.config = config;
        }
        // The tray reads the updater's file and says so in the state.
        if self.state.as_ref().map(|s| &s.update) != Some(&state.update) {
            self.update_file = updates::read();
        }
        self.state = Some(state);
        // A screen that came, went or changed size leaves the editor's tree meaning
        // something else, so it closes and nothing is saved.
        if self.editor.is_some() && !screens_changed {
            let now = monitors::enumerate();
            let same = now.len() == self.mons.len()
                && now.iter().zip(&self.mons).all(|(a, b)| a.id == b.id && a.rect == b.rect);
            if !same {
                self.editor = None;
            }
        }
        if screens_changed {
            self.editor = None;
            self.read_screens();
        }
        pages::zone_editor::reload(self);
        self.relink();
    }

    pub(super) fn read_screens(&mut self) {
        self.mons = monitors::enumerate();
    }

    /// The tray's answer to what's lined up, worked out here for the map.
    fn relink(&mut self) {
        let sig = self.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default();
        let saved = self.config.desks.get(&sig).map(|d| d.borders.as_slice());
        self.links = desk::resolve(&self.mons, saved);
        self.layout = desk::physical_layout(&self.mons, &self.links);
    }

    pub fn desk_name(&self) -> String {
        self.state.as_ref().map(|s| s.desk.name.clone()).unwrap_or_default()
    }

    /// Call the tray off the UI thread, then carry on here with the answer.
    pub fn call(
        &self,
        method: &'static str,
        params: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Panel, Result<Value, CallError>, &mut Window, &mut Context<Panel>) + 'static,
    ) {
        let Some(client) = self.client.clone() else {
            window.push_notification(refusal("Seam isn't running", "Start it from the sidebar, then try again"), cx);
            return;
        };
        let answer = cx.background_executor().spawn(async move {
            match client.lock() {
                Ok(mut c) => c.call(method, params),
                Err(_) => Err(CallError::Io(std::io::ErrorKind::BrokenPipe.into())),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| then(this, result, window, cx));
        })
        .detach();
    }

    /// Run a command whose only feedback is a refusal.
    pub fn command(&self, method: &'static str, params: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.call(method, params, window, cx, |_, result, window, cx| {
            if let Err(e) = result {
                window.push_notification(refusal("Seam can't do that", &error_text(&e)), cx);
            }
        });
    }

    /// Change settings. They show at once, and the tray has the last word.
    pub fn patch(&mut self, patch: Value, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(config) = self.config.patched(&patch) {
            self.config = config;
        }
        self.relink();
        cx.notify();
        self.call("config.patch", json!({ "patch": patch }), window, cx, |this, result, window, cx| {
            if let Err(e) = result {
                window.push_notification(refusal("Setting left as it was", &error_text(&e)), cx);
            }
            this.reload_config(window, cx);
        });
    }

    pub fn reload_config(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.call("config.get", json!({}), window, cx, |this, result, _, cx| {
            if let Some(config) = result.ok().and_then(|c| serde_json::from_value(c).ok()) {
                this.config = config;
                this.relink();
                cx.notify();
            }
        });
    }

    /// Ask the tray how the shown workspace's apps are doing. Only while the
    /// Workspaces page is on screen, and one question at a time.
    pub fn ask_status(&mut self, cx: &mut Context<Self>) {
        let Some(id) = pages::workspaces::shown_id(self).filter(|_| self.page == Page::Workspaces) else { return };
        let Some(client) = self.client.clone() else { return };
        // Something changed while the last answer was on its way, so ask again once it's in.
        if self.asking_status.is_some() {
            self.status_again = true;
            return;
        }
        self.asking_status = Some(id.clone());
        let params = json!({ "id": id });
        let answer = cx.background_executor().spawn(async move {
            let mut client = client.lock().ok()?;
            client.call("workspace.status", params).ok().and_then(|rows| serde_json::from_value(rows).ok())
        });
        cx.spawn(async move |this, cx| {
            let rows: Option<Vec<RowStatus>> = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.asking_status = None;
                this.status = Some((id, rows.unwrap_or_default()));
                if std::mem::take(&mut this.status_again) {
                    this.ask_status(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Say what just happened, with Undo for 8 s. A newer one replaces it.
    pub fn offer_undo(
        &mut self,
        message: String,
        undo: impl Fn(&mut Panel, &mut Window, &mut Context<Panel>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        struct Undo;
        let view = cx.entity().downgrade();
        let undo = std::rc::Rc::new(undo);
        let note = Notification::new().id::<Undo>().message(message).action(move |_, _, cx| {
            let (view, undo) = (view.clone(), undo.clone());
            ui::primary("undo", "Undo").on_click(cx.listener(move |note: &mut Notification, _, window, cx| {
                let undo = undo.clone();
                let _ = view.update(cx, |panel, cx| undo(panel, window, cx));
                note.dismiss(window, cx);
            }))
        });
        window.push_notification(note, cx);
        self.toast_generation += 1;
        let generation = self.toast_generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(UNDO_FOR).await;
            let _ = this.update_in(cx, |panel, window, cx| {
                if panel.toast_generation == generation {
                    window.remove_notification::<Undo>(cx);
                }
            });
        })
        .detach();
    }

    /// Replace every setting with `next`.
    pub fn replace_config(&mut self, next: &Config, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(old), Ok(new)) = (serde_json::to_value(&self.config), serde_json::to_value(next)) else { return };
        self.patch(diff_patch(&old, &new), window, cx);
    }
}

type Session = (Client, Receiver<Event>, State, Value);

fn connect(token: &str) -> Result<Session, CallError> {
    let mut client = Client::connect("panel", Some(token))?;
    let events = Client::subscribe("panel", Some(token), &["state", "desk", "workspace", "update", "lineup", "panel"])?;
    let state = client.call("state.get", json!({}))?;
    let state = serde_json::from_value(state).map_err(std::io::Error::other)?;
    let config = client.call("config.get", json!({}))?;
    Ok((client, events, state, config))
}

fn next_event(events: &Mutex<Receiver<Event>>) -> Option<Event> {
    events.lock().ok()?.recv().ok()
}
