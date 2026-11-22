//! What a switch tells this window (D711 to D716): a notice while it starts apps, with a button
//! that stops it, and after it, what went wrong. The tray keeps the last result of each workspace.

use super::*;
use gpui_kit::component::button::Button;
use seam::workspace::report;
use serde_json::Value;

/// How long a switch starts apps before the notice says so.
const STARTING_AFTER: Duration = Duration::from_secs(2);

/// An app a switch left alone or couldn't move, as `workspace.last_switch` tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub row: Option<usize>,
    pub app: String,
    pub reason: String,
}

/// What the last switch of a workspace did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastSwitch {
    pub at: u64,
    pub cancelled: bool,
    pub skipped: Vec<Skipped>,
}

impl LastSwitch {
    /// Read what `workspace.last_switch` and `workspace.switched` tell. `None` for null.
    pub fn from_value(value: &Value) -> Option<LastSwitch> {
        let skipped = value["skipped"].as_array()?.iter().filter_map(|s| {
            let row = s["row"].as_u64().map(|r| r as usize);
            Some(Skipped { row, app: s["app"].as_str()?.to_string(), reason: s["reason"].as_str()?.to_string() })
        });
        Some(LastSwitch {
            at: value["at"].as_u64().unwrap_or(0),
            cancelled: value["cancelled"].as_bool().unwrap_or(false),
            skipped: skipped.collect(),
        })
    }

    /// Each app that had a problem once, with the words for it.
    pub fn problems(&self) -> Vec<(String, &'static str)> {
        report::problems(self.skipped.iter().map(|s| (s.app.as_str(), s.reason.as_str())))
    }

    /// The words for the problem one of these rules had, if any.
    pub fn problem_of(&self, rules: &[usize]) -> Option<&'static str> {
        let on_row = |s: &&Skipped| s.row.is_some_and(|r| rules.contains(&r));
        self.skipped.iter().filter(on_row).find_map(|s| report::words(&s.reason))
    }

    /// How many apps a cancel left unstarted.
    fn left(&self) -> Option<usize> {
        let mut rows: Vec<Option<usize>> =
            self.skipped.iter().filter(|s| s.reason == report::CANCELLED).map(|s| s.row).collect();
        rows.sort_unstable();
        rows.dedup();
        self.cancelled.then_some(rows.len())
    }
}

/// A switch this window asked for, until its result arrives.
pub struct Asked {
    pub name: String,
    /// Undo goes back to the workspace that was in use.
    pub undoable: bool,
}

struct Starting;

impl Panel {
    /// A switch is about to be asked for, so its result gets the notice with Undo.
    pub fn expect_switch(&mut self, name: String, undoable: bool) {
        self.asked = Some(Asked { name, undoable });
    }

    /// `workspace.switching`: a switch began starting apps. Say so if it takes a while.
    pub fn switching(&mut self, data: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(name), Some(apps)) = (data["name"].as_str(), data["starting"].as_u64()) else { return };
        let (title, message) = report::starting(name, apps as usize, false);
        self.starting_generation += 1;
        let generation = self.starting_generation;
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(STARTING_AFTER).await;
            let _ = this.update_in(cx, |panel, window, cx| {
                if panel.starting_generation == generation {
                    window.push_notification(starting_notice(title, message, view), cx);
                }
            });
        })
        .detach();
    }

    /// `workspace.switched`: the switch ended. The starting notice goes, and a switch this window
    /// asked for says how it went, with Undo.
    pub fn switched(&mut self, data: &Value, window: &mut Window, cx: &mut Context<Self>) {
        self.starting_generation += 1;
        window.remove_notification::<Starting>(cx);
        if data["from_panel"].as_bool() != Some(true) {
            return;
        }
        let asked = self.asked.take();
        let name = data["name"].as_str().unwrap_or_default().to_string();
        let last = LastSwitch::from_value(data);
        let summary = last.as_ref().and_then(|l| report::panel_summary(&name, &l.problems(), l.left()));
        let undo = |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
            panel.command("workspace.switch_back", json!({}), window, cx)
        };
        match (asked, summary) {
            (Some(asked), summary) if asked.undoable => {
                let (title, message) =
                    summary.map_or((None, format!("Switched to {}", asked.name)), |(t, m)| (Some(t), m));
                self.offer_undo_titled(title, message, undo, window, cx);
            }
            (_, Some((title, message))) => {
                window.push_notification(Notification::new().title(title).message(message), cx);
            }
            (Some(asked), None) => {
                window.push_notification(Notification::new().message(format!("Switched to {}", asked.name)), cx);
            }
            (None, None) => {}
        }
    }
}

/// "Switching to «Name»" with a button that stops the switch. It stays until the switch ends.
fn starting_notice(title: String, message: String, view: WeakEntity<Panel>) -> Notification {
    Notification::new()
        .id::<Starting>()
        .title(title)
        .message(message)
        .autohide(false)
        .action(move |_, _, cx| cancel_button(view.clone(), cx))
}

fn cancel_button(view: WeakEntity<Panel>, cx: &mut Context<Notification>) -> Button {
    ui::secondary("cancel-switch", "Cancel switch", cx).on_click(cx.listener(
        move |note: &mut Notification, _, window, cx| {
            // The tray ends the switch, which takes this notice away.
            let _ = view.update(cx, |panel, cx| panel.command("workspace.cancel", json!({}), window, cx));
            note.dismiss(window, cx);
        },
    ))
}
