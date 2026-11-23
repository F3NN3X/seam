//! "Where apps open" on the Windows page: apps whose windows go to a place, or stay on
//! top, whenever they open, workspace or not, and the apps Seam leaves alone. Every change is
//! saved at once through `config.patch`, and the last one can be undone for 8 s.

use crate::icons;
use crate::pages::app_picker;
use crate::pages::workspace_editor::{Place, app_icon, places};
use crate::pages::workspaces::{app_title, on_desk};
use crate::panel::Panel;
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui::{self, Choice, Row, card, section, switch};
use gpui_kit::component::Icon;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use seam::state::Screen;
use seam::workspace::{Pin, Rule, Target, WinState, Workspace};
use serde_json::{Value, json};
use std::rc::Rc;

const GOES_TO_WIDTH: f32 = 200.;
/// The app icon's box, as the workspace editor draws it.
const ICON_BOX: f32 = 24.;
/// What taking an app out of the list does.
type Removing = Rc<dyn Fn(&mut Panel, &mut Window, &mut Context<Panel>)>;
/// What the place menu calls a rule that only pins.
const STAYS: &str = "Doesn't move";
/// `Pin::bits`: on top, and out of workspace switches.
const TOP: usize = 1;
const STICKY: usize = 2;

/// Where a rule sends its window, in the words of the menu.
fn goes(rule: &Rule, screens: &[Screen]) -> String {
    let number = screens.iter().find(|s| s.id == rule.monitor).map(|s| s.number);
    match (&rule.target, number) {
        (Target::State { state: WinState::Keep }, _) => STAYS.into(),
        (_, None) => "Screen not connected".into(),
        (Target::Zone { zone, span_to: None }, Some(n)) => format!("Screen {n}, zone {zone}"),
        (Target::Zone { zone, span_to: Some(to) }, Some(n)) => format!("Screen {n}, zones {zone} to {to}"),
        (Target::State { .. }, Some(n)) => format!("Screen {n}, full"),
        (Target::Rect { .. }, Some(n)) => format!("Screen {n}, own size"),
    }
}

/// What the rule does, as the line under its app: "Opens on screen 4, zone 1", "Kept on top",
/// "Sent to Work". A rule with a workspace doesn't use its place, so the line leaves it out.
fn rule_line(rule: &Rule, screens: &[Screen], workspaces: &[Workspace]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(id) = &rule.send_to {
        match workspaces.iter().find(|w| w.id == *id) {
            Some(ws) => parts.push(format!("sent to {}", ws.name)),
            None => parts.push("workspace on another desk".into()),
        }
    } else if goes(rule, screens) != STAYS {
        parts.push(
            format!("opens on {}", goes(rule, screens).to_lowercase())
                .replace("screen not connected", "a screen that isn't connected"),
        );
    }
    if rule.pin.is_some_and(|pin| pin.bits() & TOP != 0) {
        parts.push("kept on top".into());
    }
    if rule.pin.is_some_and(|pin| pin.bits() & STICKY != 0) {
        parts.push("workspaces leave it alone".into());
    }
    if rule.float {
        parts.push("left out of smart tiling".into());
    }
    let line = parts.join(", ");
    let mut chars = line.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or(line)
}

/// The saved rules this build can read, with their place in the saved list.
fn readable(saved: &[Value]) -> Vec<(usize, Rule)> {
    let rule = |v: &Value| serde_json::from_value::<Rule>(v.clone()).ok();
    saved.iter().enumerate().filter_map(|(i, v)| Some((i, rule(v)?))).collect()
}

/// Save the whole list. Undo puts the list from before back.
fn save(panel: &mut Panel, after: Vec<Value>, note: String, window: &mut Window, cx: &mut Context<Panel>) {
    let before = panel.config.app_rules.clone();
    if after == before {
        return;
    }
    panel.patch(json!({ "app_rules": after }), window, cx);
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        panel.patch(json!({ "app_rules": before.clone() }), window, cx)
    };
    panel.offer_undo(note, undo, window, cx);
}

/// Change the rule saved at `at`.
fn change(
    panel: &mut Panel,
    at: usize,
    note: impl FnOnce(&str) -> String,
    window: &mut Window,
    cx: &mut Context<Panel>,
    edit: impl FnOnce(&mut Rule),
) {
    let mut after = panel.config.app_rules.clone();
    let Some(mut rule) = after.get(at).and_then(|v| serde_json::from_value::<Rule>(v.clone()).ok()) else { return };
    let title = app_title(&rule);
    edit(&mut rule);
    after[at] = serde_json::to_value(&rule).unwrap_or_default();
    save(panel, after, note(&title), window, cx);
}

fn remove(panel: &mut Panel, at: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let mut after = panel.config.app_rules.clone();
    let Some(title) = after.get(at).and_then(|v| serde_json::from_value::<Rule>(v.clone()).ok()).map(|r| app_title(&r))
    else {
        return;
    };
    after.remove(at);
    panel.rule_open = None;
    save(panel, after, format!("Removed {title}"), window, cx);
}

/// An app the picker offered. It starts on the first place the screens offer, to be
/// changed from there. An app that has a rule already is left as it is.
fn add(panel: &mut Panel, rule: Rule, window: &mut Window, cx: &mut Context<Panel>) {
    let known = readable(&panel.config.app_rules);
    let same = |r: &Rule| icons::identity(r) == icons::identity(&rule);
    let Some(place) = places(panel).into_iter().find(|p| p.monitor.is_some()) else { return };
    if known.iter().any(|(_, r)| same(r)) {
        return;
    }
    let title = app_title(&rule);
    let mut rule = Rule { monitor: place.monitor.unwrap_or_default(), ..rule };
    rule.set_target(place.target);
    let mut after = panel.config.app_rules.clone();
    after.push(serde_json::to_value(&rule).unwrap_or_default());
    save(panel, after, format!("{title} opens in {}", place.label), window, cx);
}

/// The Select of where a rule's windows open. It keeps its value while a workspace decides.
fn goes_select(panel: &Panel, at: usize, rule: &Rule, screens: &[Screen], cx: &mut Context<Panel>) -> AnyElement {
    let (title, current, locked) = (app_title(rule), goes(rule, screens), rule.send_to.is_some());
    let view = cx.entity();
    // "Doesn't move" is for a rule that only pins, so it comes first.
    let stays = Place { label: STAYS.into(), monitor: None, target: Target::State { state: WinState::Keep } };
    let placed = places(panel).into_iter().filter(|p| p.monitor.is_some());
    let offered: Vec<Place> = std::iter::once(stays).chain(placed).collect();
    // A screen that isn't connected anymore is still what the rule says, until another is picked.
    let kept =
        offered.iter().all(|place| place.label != current).then(|| Choice::new(current.clone(), current.clone()));
    let choices = kept
        .into_iter()
        .chain(offered.iter().map(|place| Choice::new(place.label.clone(), place.label.clone())))
        .collect();
    let pick = move |key: &str, window: &mut Window, cx: &mut App| {
        let Some(place) = offered.iter().find(|place| place.label == key).cloned().filter(|_| !locked) else { return };
        view.update(cx, |panel, cx| {
            let stays = place.monitor.is_none();
            let note = |title: &str| match stays {
                true => format!("{title} doesn't move"),
                false => format!("{title} opens in {}", place.label),
            };
            change(panel, at, note, window, cx, |rule| {
                rule.set_target(place.target.clone());
                if let Some(monitor) = place.monitor.clone() {
                    rule.monitor = monitor;
                }
                // A rule that moves nothing and pins nothing would do nothing.
                if stays && rule.pin.is_none() {
                    rule.pin = Some(Pin::Top);
                }
            });
        });
    };
    ui::select(("rule-goes-to", at), format!("Where {title} opens"), choices, &current, GOES_TO_WIDTH, cx, pick)
}

/// Turn one pin bit of the rule saved at `at` on or off.
fn set_pin(panel: &mut Panel, at: usize, bit: usize, on: bool, window: &mut Window, cx: &mut Context<Panel>) {
    let what = if bit == TOP { "on top" } else { "out of workspace switches" };
    let note = |title: &str| match on {
        true => format!("{title} stays {what}"),
        false => format!("{title} no longer stays {what}"),
    };
    change(panel, at, note, window, cx, |rule| {
        let bits = rule.pin.map_or(0, Pin::bits);
        rule.pin = Pin::of_bits(if on { bits | bit } else { bits & !bit });
    });
}

/// Turn the rule's "Leave out of smart tiling" on or off.
fn set_float(panel: &mut Panel, at: usize, on: bool, window: &mut Window, cx: &mut Context<Panel>) {
    let note = |title: &str| match on {
        true => format!("{title} floats on tiled screens"),
        false => format!("{title} is tiled like any other app"),
    };
    change(panel, at, note, window, cx, |rule| rule.float = on);
}

/// The Select of the workspace a rule's new windows join. A workspace that isn't on this desk
/// stays what the rule says, in muted text, until another is picked.
fn send_select(at: usize, rule: &Rule, workspaces: &[Workspace], cx: &mut Context<Panel>) -> AnyElement {
    let view = cx.entity();
    let current = rule.send_to.clone().unwrap_or_default();
    let gone = (!current.is_empty() && workspaces.iter().all(|w| w.id != current))
        .then(|| Choice::new(current.clone(), "Not on this desk").muted());
    let choices: Vec<Choice> = std::iter::once(Choice::new("", "None"))
        .chain(workspaces.iter().map(|w| Choice::new(w.id.clone(), w.name.clone())))
        .chain(gone)
        .collect();
    let names: Vec<(String, String)> = workspaces.iter().map(|w| (w.id.clone(), w.name.clone())).collect();
    let pick = move |key: &str, window: &mut Window, cx: &mut App| {
        let to = names.iter().find(|(id, _)| id == key).cloned();
        view.update(cx, |panel, cx| set_send_to(panel, at, to, window, cx));
    };
    let label = format!("Send {}'s new windows to", app_title(rule));
    ui::select(("rule-send-to", at), label, choices, &current, GOES_TO_WIDTH, cx, pick)
}

/// Send the rule's new windows to the workspace `to` (its id and name), or to none.
fn set_send_to(
    panel: &mut Panel,
    at: usize,
    to: Option<(String, String)>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let name = to.as_ref().map(|(_, name)| name.clone());
    let note = |title: &str| match &name {
        Some(name) => format!("{title} goes to {name}"),
        None => format!("{title} no longer goes to a workspace"),
    };
    change(panel, at, note, window, cx, |rule| {
        rule.send_to = to.map(|(id, _)| id);
    });
}

/// The `…` of a row: Edit opens or closes the rule's details, and Remove takes the rule away.
fn more_menu(
    id: (&'static str, usize),
    title: &str,
    edit: Option<String>,
    remove: Removing,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let weak = cx.entity().downgrade();
    ui::secondary(id, "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {title}"))
        .dropdown_menu(move |mut menu, _, _| {
            if let Some(app) = edit.clone() {
                let weak = weak.clone();
                menu = menu.item(PopupMenuItem::new("Edit").on_click(move |_, _, cx| {
                    let app = app.clone();
                    let _ = weak.update(cx, |panel, cx| {
                        panel.rule_open = if panel.rule_open.as_ref() == Some(&app) { None } else { Some(app) };
                        cx.notify();
                    });
                }));
            }
            let (weak, remove) = (weak.clone(), remove.clone());
            menu.item(PopupMenuItem::new("Remove").on_click(move |_, window, cx| {
                let remove = remove.clone();
                let _ = weak.update(cx, |panel, cx| remove(panel, window, cx));
            }))
        })
        .into_any_element()
}

/// One app and what it does, with the `…` after it.
fn app_row(icon: Div, title: String, line: String, more: AnyElement, p: Palette) -> Div {
    let name = div()
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_2))
        .flex_1()
        .min_w_0()
        .child(text_style(div(), tokens::BODY).truncate().child(title))
        .child(ui::description(line, p).truncate());
    div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .min_h(px(tokens::SIZE_ROW_2))
        .px(px(tokens::SPACE_16))
        .py(px(tokens::SPACE_8))
        .child(icon)
        .child(name)
        .child(more)
}

/// A rule's row, and while it's being edited, what it does under it.
fn rule_rows(
    panel: &Panel,
    at: usize,
    rule: &Rule,
    screens: &[Screen],
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<AnyElement> {
    let p = palette(cx);
    let title = app_title(rule);
    let remove: Removing = Rc::new(move |panel, window, cx| self::remove(panel, at, window, cx));
    let more = more_menu(("rule-more", at), &title, Some(icons::identity(rule)), remove, cx);
    let line = rule_line(rule, screens, &on_desk(panel));
    let main = app_row(app_icon(rule, &panel.icons, p), title, line, more, p);
    let mut rows = vec![main.into_any_element()];
    if panel.rule_open.as_ref() == Some(&icons::identity(rule)) {
        rows.extend(details(panel, at, rule, screens, window, cx));
    }
    rows
}

/// What a rule does: where its windows open, and the pins that go with it.
fn details(
    panel: &Panel,
    at: usize,
    rule: &Rule,
    screens: &[Screen],
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<AnyElement> {
    let p = palette(cx);
    let view = cx.entity();
    // A rule that sends to a workspace does something without a place or a pin.
    let (bits, stays) = (rule.pin.map_or(0, Pin::bits), goes(rule, screens) == STAYS && rule.send_to.is_none());
    let workspaces = on_desk(panel);
    let pin = |bit: usize,
               id: &'static str,
               title: &'static str,
               about: &'static str,
               window: &mut Window,
               cx: &mut Context<Panel>|
     -> Row {
        let on = bits & bit != 0;
        // A rule that doesn't move its app needs a pin, or to float, to do anything.
        let last = stays && on && bits == bit && !rule.float;
        let view = view.clone();
        let control = switch((id, at), title, on, window, cx, move |on, window, cx| {
            if !last {
                view.update(cx, |panel, cx| set_pin(panel, at, bit, on, window, cx));
            }
        });
        let why = last.then_some("This rule only pins, so it needs one of these on");
        let row_id = ElementId::Name(format!("{id}-row-{at}").into());
        ui::dimmed(ui::row(title, Some(about.into()), control, p), row_id, why)
    };
    let float = {
        let on = rule.float;
        let last = stays && bits == 0 && on;
        let view = view.clone();
        let title = "Leave out of smart tiling";
        let control = switch(("rule-float", at), title, on, window, cx, move |on, window, cx| {
            if !last {
                view.update(cx, |panel, cx| set_float(panel, at, on, window, cx));
            }
        });
        let why = last.then_some("This rule only floats, so it needs one of these on");
        let about = "Its windows float on a screen with smart tiling";
        ui::dimmed(ui::row(title, Some(about.into()), control, p), ("rule-float-row", at), why)
    };
    let why = rule.send_to.is_some().then_some("The workspace decides where it goes");
    let opens = ui::dimmed(
        ui::row("Opens on", None, goes_select(panel, at, rule, screens, cx), p),
        ("rule-opens-row", at),
        why,
    );
    let about = "It joins the workspace as a row. When that workspace is in use it goes where the row says";
    let send = ui::row("Send new windows to", Some(about.into()), send_select(at, rule, &workspaces, cx), p);
    let keep_top = pin(TOP, "rule-top", "Keep on top", "Its windows stay above the others", window, cx);
    let keep_out = pin(
        STICKY,
        "rule-sticky",
        "Keep out of workspace switches",
        "A workspace switch leaves its windows alone",
        window,
        cx,
    );
    [opens, send, keep_top, keep_out, float]
        .into_iter()
        .map(|row| div().pl(px(tokens::SPACE_32)).child(row).into_any_element())
        .collect()
}

/// An app Seam leaves alone, with Remove. Undo puts the list back.
fn left_alone_row(i: usize, name: String, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let letter = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let icon = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(ICON_BOX))
        .rounded(px(tokens::RADIUS_4))
        .bg(p.surface_raised)
        .text_color(p.text_secondary)
        .child(letter);
    let remove: Removing = Rc::new({
        let name = name.clone();
        move |panel, window, cx| {
            let before = panel.config.zones.exclude.clone();
            let rest: Vec<String> = before.iter().filter(|x| **x != name).cloned().collect();
            panel.patch(json!({ "zones": { "exclude": rest } }), window, cx);
            let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                panel.patch(json!({ "zones": { "exclude": before.clone() } }), window, cx)
            };
            panel.offer_undo(format!("Removed {name}"), undo, window, cx);
        }
    });
    let more = more_menu(("never-manage", i), &name, None, remove, cx);
    app_row(icon, name, "Seam doesn't snap, move or capture its windows".into(), more, p).into_any_element()
}

/// The card: a row for each rule and each app left alone, then "Add an app".
pub fn section_view(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> AnyElement {
    let p = palette(cx);
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    let known = readable(&panel.config.app_rules);
    let apps: Vec<Rule> = known.iter().map(|(_, r)| r.clone()).collect();
    icons::request(panel, &apps, cx);

    let mut rows: Vec<AnyElement> = Vec::new();
    for (at, rule) in &known {
        rows.extend(rule_rows(panel, *at, rule, &screens, window, cx));
    }
    let excluded = panel.config.zones.exclude.clone();
    rows.extend(excluded.into_iter().enumerate().map(|(i, name)| left_alone_row(i, name, p, cx)));
    let adding: app_picker::Add = Rc::new(|panel, rule, _, window, cx| add(panel, rule, window, cx));
    let button = app_picker::button(panel, &apps, "Already has a place", adding, None, cx);
    if rows.is_empty() {
        let about = "Pick where an app's windows open, keep them on top, or keep workspaces from touching them";
        rows.push(ui::row("No apps yet", Some(about.into()), button, p).into_any_element());
    } else {
        rows.push(div().flex().px(px(tokens::SPACE_16)).py(px(tokens::SPACE_12)).child(button).into_any_element());
    }
    let hint = ui::description(
        "Choose Never include this app on a workspace's app to add one to the apps Seam leaves alone",
        p,
    );
    let lonely = panel.config.zones.exclude.is_empty();
    let body = card(rows, p).within(false, move |group| {
        div().flex().flex_col().gap(px(tokens::SPACE_8)).child(group).children(lonely.then_some(hint))
    });
    section("Where apps open", body, p)
}

#[cfg(test)]
mod tests {
    use super::{Pin, Rule, Screen, Target, Workspace, rule_line};

    fn screen() -> Screen {
        Screen {
            number: 4,
            id: "m".into(),
            name: String::new(),
            primary: true,
            rect: Default::default(),
            layout: None,
            workspace: None,
        }
    }

    fn work() -> Workspace {
        Workspace { id: "w1".into(), name: "Work".into(), ..Workspace::default() }
    }

    fn placed() -> Rule {
        Rule { monitor: "m".into(), target: Target::Zone { zone: 1, span_to: None }, ..Rule::default() }
    }

    #[test]
    fn the_line_of_a_rule_without_a_workspace_is_as_before() {
        let pinned = Rule { pin: Some(Pin::Top), ..placed() };
        assert_eq!(rule_line(&placed(), &[screen()], &[work()]), "Opens on screen 4, zone 1");
        assert_eq!(rule_line(&pinned, &[screen()], &[work()]), "Opens on screen 4, zone 1, kept on top");
    }

    #[test]
    fn the_line_of_a_rule_with_a_workspace_names_it_and_leaves_the_place_out() {
        let sent = Rule { send_to: Some("w1".into()), ..placed() };
        assert_eq!(rule_line(&sent, &[screen()], &[work()]), "Sent to Work");
        let pinned = Rule { pin: Some(Pin::Top), ..sent };
        assert_eq!(rule_line(&pinned, &[screen()], &[work()]), "Sent to Work, kept on top");
        let only = Rule { send_to: Some("w1".into()), ..Rule::default() };
        assert_eq!(rule_line(&only, &[], &[work()]), "Sent to Work");
    }

    #[test]
    fn the_line_of_a_rule_for_a_workspace_on_another_desk_says_so() {
        let sent = Rule { send_to: Some("other".into()), pin: Some(Pin::Top), ..placed() };
        assert_eq!(rule_line(&sent, &[screen()], &[work()]), "Workspace on another desk, kept on top");
    }
}
