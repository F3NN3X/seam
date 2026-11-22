//! Capture current windows lists the windows before it saves (D721), and Update from
//! screen asks what to do with the apps that aren't open (D726).

use super::close::label_within;
use super::sections::CLOSED_CHOICES;
use super::*;
use crate::icons::Icons;
use gpui_kit::component::Disableable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{InputEvent, InputState, SelectAll};
use gpui_kit::component::progress::Progress;
use seam::api::CallError;
use seam::config::ClosedApps;
use std::cell::RefCell;

/// The longest "App: title, reason" a row of the list shows, in characters.
const LABEL_MAX: usize = 76;

/// One window the tray would save.
struct Item {
    app: String,
    title: String,
    screen: Option<u64>,
    /// Why it starts unticked.
    why: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Waiting for the tray to read the windows.
    Reading,
    Ready,
    /// The preview is too old to save.
    Expired,
}

/// What the dialog shows, and what it will send.
struct Review {
    stage: Stage,
    token: String,
    items: Vec<Item>,
    ticked: Vec<bool>,
}

impl Review {
    fn reading() -> Review {
        Review { stage: Stage::Reading, token: String::new(), items: Vec::new(), ticked: Vec::new() }
    }

    fn read(&mut self, answer: &Value) {
        let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_string();
        let windows = answer["windows"].as_array().into_iter().flatten();
        self.items = windows
            .map(|w| Item {
                app: text(w, "app"),
                title: text(w, "title"),
                screen: w["screen"].as_u64(),
                why: w["why"].as_str().map(String::from),
            })
            .collect();
        self.ticked =
            answer["windows"].as_array().into_iter().flatten().map(|w| w["ticked"].as_bool() == Some(true)).collect();
        self.token = text(answer, "token");
        self.stage = Stage::Ready;
    }
}

/// Whether the tray refused a save because its preview is gone.
fn expired(e: &CallError) -> bool {
    matches!(e, CallError::Rpc(e) if e.code == seam::api::REFUSED && e.message.contains("expired"))
}

/// "6 of 8 windows ticked".
fn ticked_note(ticked: usize, total: usize) -> String {
    format!("{ticked} of {total} window{} ticked", if total == 1 { "" } else { "s" })
}

/// Ask the tray for a preview, and fill `state` from its answer. The dialog shows the
/// list when the answer comes, or closes with the reason when there is none.
fn ask(
    panel: &Panel,
    state: &Rc<RefCell<Review>>,
    name: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    *state.borrow_mut() = Review::reading();
    let (state, name) = (state.clone(), name.clone());
    panel.call("workspace.capture_preview", json!({}), window, cx, move |_, result, window, cx| match result {
        Ok(answer) => {
            state.borrow_mut().read(&answer);
            let text = answer["name"].as_str().unwrap_or_default().to_string();
            name.update(cx, |input, cx| {
                input.set_value(text, window, cx);
                input.focus(window, cx);
            });
            window.defer(cx, |window, cx| window.dispatch_action(Box::new(SelectAll), cx));
            window.refresh();
        }
        Err(e) => {
            window.close_dialog(cx);
            window.push_notification(refusal("Nothing captured", &error_text(&e)), cx);
        }
    });
}

/// "Capture current windows": read the windows, then show them with the likely noise unticked.
pub fn open(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let state = Rc::new(RefCell::new(Review::reading()));
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
    ask(panel, &state, &name, window, cx);
    if panel.client.is_none() {
        return;
    }
    let screens: Vec<(u64, String)> = panel
        .state
        .as_ref()
        .map(|s| s.screens.iter().map(|s| (u64::from(s.number), s.name.clone())).collect())
        .unwrap_or_default();
    // The Save button follows what is typed.
    let typed = cx.subscribe_in(&name, window, |_, _, event: &InputEvent, window, _| {
        if matches!(event, InputEvent::Change) {
            window.refresh();
        }
    });
    let view = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, _, cx| {
        let _typed = &typed;
        let p = palette(cx);
        let stage = state.borrow().stage;
        let text = name.read(cx).value().trim().to_string();
        let ticked = state.borrow().ticked.iter().filter(|t| **t).count();
        let body = match stage {
            Stage::Reading => div()
                .py(px(tokens::SPACE_16))
                .child(Progress::new("review-reading").loading(true).accessibility_label("Reading the windows"))
                .into_any_element(),
            Stage::Expired => text_style(div(), tokens::BODY)
                .text_color(p.text_secondary)
                .child("The windows changed. Capture again")
                .into_any_element(),
            Stage::Ready => ready_body(&state, &name, &screens, p),
        };
        let label = if stage == Stage::Expired { "Capture again" } else { "Save workspace" };
        let mut ok = ui::primary("review-ok", label);
        match stage {
            Stage::Reading => ok = ok.disabled(true),
            Stage::Ready if ticked == 0 => ok = ok.disabled(true).tooltip("Tick a window to save it"),
            Stage::Ready if text.is_empty() => ok = ok.disabled(true).tooltip("Give the workspace a name"),
            _ => {}
        }
        let (view, state, name) = (view.clone(), state.clone(), name.clone());
        dialog
            .w(px(600.))
            .title("Capture current windows")
            .child(body)
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Don't save")))
                    .child(DialogAction::new().child(ok)),
            )
            .on_ok(move |_, window, cx| {
                let stage = state.borrow().stage;
                let _ = view.update(cx, |panel, cx| match stage {
                    Stage::Reading => {}
                    Stage::Expired => {
                        ask(panel, &state, &name, window, cx);
                        window.refresh();
                    }
                    Stage::Ready => save_ticked(panel, &state, &name, window, cx),
                });
                // Saving closes the dialog when the tray has answered.
                false
            })
    });
}

/// The name, the windows grouped by screen with a tick each, and how many are ticked.
fn ready_body(
    state: &Rc<RefCell<Review>>,
    name: &Entity<InputState>,
    screens: &[(u64, String)],
    p: Palette,
) -> AnyElement {
    let review = state.borrow();
    let mut list =
        div().id("review-list").flex().flex_col().gap(px(tokens::SPACE_8)).max_h(px(360.)).overflow_y_scroll();
    let mut order: Vec<usize> = (0..review.items.len()).collect();
    order.sort_by_key(|i| review.items[*i].screen.unwrap_or(u64::MAX));
    let mut shown = None;
    for i in order {
        let item = &review.items[i];
        if shown != Some(item.screen) {
            shown = Some(item.screen);
            let (head, model) = match item.screen {
                Some(n) => (
                    format!("Screen {n}"),
                    screens.iter().find(|(s, _)| *s == n).map(|(_, m)| m.clone()).unwrap_or_default(),
                ),
                None => ("Other screens".to_string(), String::new()),
            };
            list = list.child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(tokens::SPACE_8))
                    .pt(px(tokens::SPACE_4))
                    .child(text_style(div(), tokens::BODY).font_weight(FontWeight::MEDIUM).child(head))
                    .child(ui::description(model, p)),
            );
        }
        let reason = item.why.as_ref().map(|why| format!(", {why}"));
        let room = LABEL_MAX.saturating_sub(reason.as_ref().map_or(0, |r| r.chars().count()));
        let (on, state) = (review.ticked[i], state.clone());
        let check = Checkbox::new(("review-window", i))
            .label(label_within(&item.app, &item.title, room))
            .checked(on)
            .on_click(move |on, window, _| {
                if let Some(slot) = state.borrow_mut().ticked.get_mut(i) {
                    *slot = *on;
                }
                window.refresh();
            });
        list = list.child(
            div()
                .flex()
                .items_center()
                .child(check)
                .children(reason.map(|r| text_style(div(), tokens::BODY).text_color(p.text_muted).child(r))),
        );
    }
    let count = review.ticked.iter().filter(|t| **t).count();
    div()
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_12))
        .child(
            text_style(div(), tokens::BODY)
                .text_color(p.text_secondary)
                .child("Seam saves which app is where. Untick a window to leave it out"),
        )
        .child(
            div().flex().flex_col().gap(px(tokens::SPACE_4)).child(ui::description("Name", p)).child(Input::new(name)),
        )
        .child(list)
        .child(ui::description(ticked_note(count, review.items.len()), p))
        .into_any_element()
}

/// Send the ticked windows and the name. The dialog closes once the tray has saved them.
fn save_ticked(
    panel: &mut Panel,
    state: &Rc<RefCell<Review>>,
    name: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let (token, take) = {
        let review = state.borrow();
        let take: Vec<usize> = review.ticked.iter().enumerate().filter(|(_, t)| **t).map(|(i, _)| i).collect();
        (review.token.clone(), take)
    };
    let text = name.read(cx).value().trim().to_string();
    if take.is_empty() || text.is_empty() {
        return;
    }
    let state = state.clone();
    let params = json!({ "token": token, "take": take, "name": text });
    panel.call("workspace.capture", params, window, cx, move |panel, result, window, cx| match result {
        Ok(done) => {
            window.close_dialog(cx);
            shortcuts::stop_recording(panel);
            // The tray gives a name already in use a number, the way `unique_name` does.
            let taken: Vec<String> = all(panel).into_iter().map(|w| w.name).collect();
            let saved_name = workspace::unique_name(&text, &taken);
            let id = done["id"].as_str().map(String::from);
            panel.workspace = id.clone();
            panel.reload_config(window, cx);
            if let Some(id) = id {
                let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                    let rest =
                        saved(panel).into_iter().filter(|v| workspace::parse(v).is_none_or(|w| w.id != id)).collect();
                    shortcuts::stop_recording(panel);
                    panel.workspace = None;
                    save(panel, rest, window, cx);
                };
                panel.offer_undo(format!("Saved {saved_name}"), undo, window, cx);
            }
        }
        Err(e) if expired(&e) => {
            state.borrow_mut().stage = Stage::Expired;
            window.refresh();
        }
        Err(e) => {
            window.close_dialog(cx);
            window.push_notification(refusal("Nothing captured", &error_text(&e)), cx);
        }
    });
}

/// An app of the workspace that isn't open, as the tray listed it.
struct Closed {
    row: usize,
    screen: Option<u64>,
}

/// What Update from screen did, for the notification: "Turned off 1 app and removed 2",
/// or only the part that applies.
fn changes_note(turned_off: usize, removed: usize) -> Option<String> {
    let apps = |n: usize| if n == 1 { "1 app".to_string() } else { format!("{n} apps") };
    match (turned_off, removed) {
        (0, 0) => None,
        (t, 0) => Some(format!("Turned off {}", apps(t))),
        (0, r) => Some(format!("Removed {}", apps(r))),
        (t, r) => Some(format!("Turned off {} and removed {r}", apps(t))),
    }
}

/// "Update from screen": read the windows, ask what to do with each app that isn't open
/// when there is one, then save with Undo.
pub fn update(panel: &Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    let before = ws.clone();
    let default = panel.config.closed_apps();
    panel.call("workspace.capture_preview", json!({ "into": ws.id }), window, cx, move |panel, result, window, cx| {
        match result {
            Ok(answer) => {
                let token = answer["token"].as_str().unwrap_or_default().to_string();
                let windows = answer["windows"].as_array().map_or(0, Vec::len);
                let take: Vec<usize> = (0..windows).collect();
                let rows: Vec<Closed> = answer["not_open"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| Some(Closed { row: r["row"].as_u64()? as usize, screen: r["screen"].as_u64() }))
                    .filter(|r| r.row < before.windows.len())
                    .collect();
                if rows.is_empty() {
                    send(panel, before, (token, take), Vec::new(), window, cx);
                } else {
                    ask_closed(before, (token, take), rows, default, window, cx);
                }
            }
            Err(e) => window.push_notification(refusal("Workspace left as it was", &error_text(&e)), cx),
        }
    });
}

/// The dialog: each app that isn't open, with Keep, Turn off or Remove, starting at `default`.
fn ask_closed(
    before: Workspace,
    sent: (String, Vec<usize>),
    rows: Vec<Closed>,
    default: ClosedApps,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let chosen = Rc::new(RefCell::new(vec![default; rows.len()]));
    let (before, rows, view) = (Rc::new(before), Rc::new(rows), cx.entity().downgrade());
    window.open_dialog(cx, move |dialog, window, cx| {
        let p = palette(cx);
        let icons = view.upgrade();
        let now = chosen.borrow().clone();
        let mut list = div().id("update-rows").flex().flex_col().max_h(px(320.)).overflow_y_scroll();
        for (k, closed) in rows.iter().enumerate() {
            let rule = &before.windows[closed.row];
            let icon = match &icons {
                Some(view) => workspace_editor::app_icon(rule, &view.read(cx).icons, p),
                None => workspace_editor::app_icon(rule, &Icons::default(), p),
            };
            let screen = closed.screen.map_or("Screen not connected".to_string(), |n| format!("Screen {n}"));
            let app = app_title(rule);
            let labels: Vec<&'static str> = CLOSED_CHOICES.iter().map(|(_, label)| *label).collect();
            let at = CLOSED_CHOICES.iter().position(|(c, _)| *c == now[k]).unwrap_or(0);
            let chosen = chosen.clone();
            let pick = ui::segmented(("closed-app", k), format!("What happens to {app}"), &labels, at, window, cx, {
                move |i, window, _| {
                    chosen.borrow_mut()[k] = CLOSED_CHOICES[i.min(CLOSED_CHOICES.len() - 1)].0;
                    window.refresh();
                }
            });
            let mut line = div()
                .flex()
                .items_center()
                .gap(px(tokens::SPACE_12))
                .min_h(px(tokens::SIZE_ROW))
                .child(icon)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(text_style(div(), tokens::BODY).truncate().child(app))
                        .child(ui::description(screen, p)),
                )
                .child(pick);
            if k > 0 {
                line = line.border_t_1().border_color(p.hairline);
            }
            list = list.child(line);
        }
        let count = rows.len();
        let about = if count == 1 {
            "1 of its apps isn't open. Choose what happens to each".to_string()
        } else {
            format!("{count} of its apps aren't open. Choose what happens to each")
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(tokens::SPACE_12))
            .child(text_style(div(), tokens::BODY).text_color(p.text_secondary).child(about))
            .child(list)
            .child(ui::description(
                "Keep starts it when you switch. Turn off keeps it but leaves it out. Remove takes it out",
                p,
            ));
        let (view, before, rows, chosen, sent) =
            (view.clone(), before.clone(), rows.clone(), chosen.clone(), sent.clone());
        let name = before.name.clone();
        dialog
            .w(px(420.))
            .title(format!("Update {name} from the screen"))
            .child(body)
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Don't update")))
                    .child(DialogAction::new().child(ui::primary("closed-ok", format!("Update {name}")))),
            )
            .on_ok(move |_, window, cx| {
                let choices: Vec<(usize, ClosedApps)> =
                    rows.iter().map(|r| r.row).zip(chosen.borrow().iter().copied()).collect();
                let (before, sent) = ((*before).clone(), sent.clone());
                let _ = view.update(cx, |panel, cx| send(panel, before, sent, choices, window, cx));
                true
            })
    });
}

/// Save the preview into the workspace, and offer Undo with what changed.
fn send(
    panel: &Panel,
    before: Workspace,
    (token, take): (String, Vec<usize>),
    choices: Vec<(usize, ClosedApps)>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let closed: serde_json::Map<String, Value> =
        choices.iter().map(|(row, choice)| (row.to_string(), json!(choice))).collect();
    // A row already off isn't turned off again.
    let turned_off = choices.iter().filter(|(row, c)| *c == ClosedApps::TurnOff && !before.windows[*row].off).count();
    let removed = choices.iter().filter(|(_, c)| *c == ClosedApps::Remove).count();
    let params = json!({ "into": before.id, "token": token, "take": take, "closed": closed });
    panel.call("workspace.capture", params, window, cx, move |panel, result, window, cx| match result {
        Ok(_) => {
            panel.reload_config(window, cx);
            let undo = {
                let before = before.clone();
                move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                    workspace_editor::update(panel, &before.id, None, window, cx, |ws| *ws = before.clone())
                }
            };
            let mut said = format!("Updated {} from the screen", before.name);
            if let Some(changes) = changes_note(turned_off, removed) {
                said = format!("{said}. {changes}");
            }
            panel.offer_undo(said, undo, window, cx);
        }
        Err(e) if expired(&e) => {
            window.push_notification(refusal("Workspace left as it was", "The windows changed. Update again"), cx)
        }
        Err(e) => window.push_notification(refusal("Workspace left as it was", &error_text(&e)), cx),
    });
}

#[cfg(test)]
mod tests {
    use super::{changes_note, ticked_note};

    #[test]
    fn the_notification_names_only_what_changed() {
        assert_eq!(changes_note(0, 0), None);
        assert_eq!(changes_note(1, 0).as_deref(), Some("Turned off 1 app"));
        assert_eq!(changes_note(0, 2).as_deref(), Some("Removed 2 apps"));
        assert_eq!(changes_note(1, 2).as_deref(), Some("Turned off 1 app and removed 2"));
    }

    #[test]
    fn the_count_agrees_with_the_list() {
        assert_eq!(ticked_note(6, 8), "6 of 8 windows ticked");
        assert_eq!(ticked_note(1, 1), "1 of 1 window ticked");
    }
}
