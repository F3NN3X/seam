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
    /// Rows can be edited after a switch, so the app's name has to match too.
    pub fn problem_of(&self, rules: &[(usize, String)]) -> Option<&'static str> {
        let on_row = |s: &&Skipped| s.row.is_some_and(|r| rules.iter().any(|(i, app)| *i == r && *app == s.app));
        self.skipped.iter().filter(on_row).find_map(|s| report::words(&s.reason))
    }

    /// How many apps a cancel left unstarted.
    fn left(&self) -> Option<usize> {
        self.cancelled.then(|| report::left_unstarted(self.skipped.iter().map(|s| (s.row, s.reason.as_str()))))
    }
}

/// A switch this window asked for, until its result arrives.
pub struct Asked {
    pub id: String,
    pub name: String,
    /// Undo goes back to the workspace that was in use.
    pub undoable: bool,
}

struct Starting;

impl Panel {
    /// A switch is about to be asked for, so its result gets the notice with Undo.
    pub fn expect_switch(&mut self, id: String, name: String, undoable: bool) {
        self.asked = Some(Asked { id, name, undoable });
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
        // Only for the switch it was asked for: a replaced one leaves nothing behind.
        let asked = self.asked.take().filter(|a| data["id"].as_str() == Some(a.id.as_str()));
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

#[cfg(test)]
mod tests {
    use super::LastSwitch;
    use serde_json::{Value, json};

    fn last() -> LastSwitch {
        let skipped = vec![
            json!({ "row": 1, "app": "slack", "reason": "not installed", "why": "x" }),
            json!({ "row": null, "app": "Other windows", "reason": "nothing matched", "why": null }),
            json!({ "row": 2, "app": "figma", "reason": "pinned", "why": null }),
            json!({ "row": 3, "app": "zoom", "reason": "cancelled", "why": null }),
            json!({ "app": "no row", "reason": "timeout" }),
        ];
        let value = json!({ "at": 5, "cancelled": true, "skipped": skipped });
        LastSwitch::from_value(&value).unwrap()
    }

    #[test]
    fn a_result_is_read_with_or_without_a_row() {
        let last = last();
        assert_eq!((last.at, last.cancelled, last.skipped.len()), (5, true, 5));
        assert_eq!(last.skipped[1].row, None);
        assert_eq!(last.skipped[4].row, None);
        assert_eq!(LastSwitch::from_value(&Value::Null), None);
        assert_eq!(last.left(), Some(1));
    }

    #[test]
    fn a_row_gets_words_only_for_its_own_app_and_a_problem() {
        let last = last();
        let rows = |i: usize, app: &str| vec![(i, app.to_string())];
        assert_eq!(last.problem_of(&rows(1, "slack")), Some("isn't installed, or Seam can't find it"));
        // A merged row names any of its windows.
        assert!(last.problem_of(&[(0, "chrome".into()), (1, "slack".into())]).is_some());
        // The row moved to another app since the switch.
        assert_eq!(last.problem_of(&rows(1, "figma")), None);
        // On purpose, and no row.
        assert_eq!(last.problem_of(&rows(2, "figma")), None);
        assert_eq!(last.problem_of(&rows(0, "no row")), None);
        assert_eq!(last.problems().len(), 2);
    }
}
