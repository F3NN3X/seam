//! "Where apps open" on the Zones page: apps whose windows go to a place whenever
//! they open, workspace or not, and the apps Seam leaves alone. Every change is
//! saved at once through `config.patch`, and the last one can be undone for 8 s.

use crate::icons;
use crate::pages::app_picker;
use crate::pages::workspace_editor::{app_icon, exe_line, goes_to, places};
use crate::pages::workspaces::app_title;
use crate::panel::Panel;
use crate::theme::{palette, Palette};
use crate::ui::{self, card, section};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::Icon;
use gpui_kit::*;
use seam::state::Screen;
use seam::workspace::Rule;
use serde_json::{json, Value};
use std::rc::Rc;

const GOES_TO_WIDTH: f32 = 200.;
const MORE_WIDTH: f32 = 32.;

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

fn goes_menu(panel: &Panel, at: usize, rule: &Rule, screens: &[Screen], cx: &mut Context<Panel>) -> AnyElement {
    let (title, current) = (app_title(rule), goes_to(rule, screens));
    let weak = cx.entity().downgrade();
    let options: Vec<_> = places(panel).into_iter().filter(|p| p.monitor.is_some()).collect();
    let shown = current.clone();
    let menu = move |mut menu: gpui_kit::component::menu::PopupMenu, _: &mut Window, _: &mut Context<_>| {
        for place in &options {
            let (weak, place) = (weak.clone(), place.clone());
            let checked = place.label == shown;
            let item = PopupMenuItem::new(place.label.clone()).checked(checked).on_click(move |_, window, cx| {
                let place = place.clone();
                let _ = weak.update(cx, |panel, cx| {
                    let note = |title: &str| format!("{title} opens in {}", place.label);
                    change(panel, at, note, window, cx, |rule| {
                        rule.set_target(place.target.clone());
                        if let Some(monitor) = place.monitor.clone() {
                            rule.monitor = monitor;
                        }
                    });
                });
            });
            menu = menu.item(item);
        }
        menu
    };
    ui::secondary(("rule-goes-to", at), current.clone(), cx)
        .dropdown_caret(true)
        .accessibility_label(format!("Where {title} opens: {current}"))
        .w_full()
        .tooltip(current)
        .dropdown_menu(menu)
        .into_any_element()
}

fn more_menu(at: usize, title: &str, cx: &mut Context<Panel>) -> AnyElement {
    let weak = cx.entity().downgrade();
    ui::secondary(("rule-more", at), "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {title}"))
        .dropdown_menu(move |menu, _, _| {
            let weak = weak.clone();
            menu.item(PopupMenuItem::new("Remove").on_click(move |_, window, cx| {
                let _ = weak.update(cx, |panel, cx| remove(panel, at, window, cx));
            }))
        })
        .into_any_element()
}

fn rule_row(
    panel: &Panel,
    at: usize,
    rule: &Rule,
    screens: &[Screen],
    p: Palette,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let title = app_title(rule);
    let name = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(div().text_size(px(14.)).truncate().child(title.clone()))
        .child(div().text_size(px(12.)).text_color(p.muted).truncate().child(exe_line(rule)));
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(8.))
        .child(app_icon(rule, &panel.icons, p))
        .child(name)
        .child(div().flex_none().w(px(GOES_TO_WIDTH)).child(goes_menu(panel, at, rule, screens, cx)))
        .child(div().flex().flex_none().justify_end().w(px(MORE_WIDTH)).child(more_menu(at, &title, cx)))
        .into_any_element()
}

fn group_head(text: &'static str, p: Palette) -> AnyElement {
    div().px(px(16.)).py(px(8.)).text_size(px(12.)).text_color(p.muted).child(text).into_any_element()
}

/// An app Seam leaves alone, with Remove. Undo puts the list back.
fn left_alone_row(i: usize, name: String, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let remove = ui::secondary(("never-manage", i), "Remove", cx)
        .accessibility_label(format!("Remove {name}"))
        .on_click(cx.listener({
            let name = name.clone();
            move |panel, _, window, cx| {
                let before = panel.config.zones.exclude.clone();
                let rest: Vec<String> = before.iter().filter(|x| **x != name).cloned().collect();
                panel.patch(json!({ "zones": { "exclude": rest } }), window, cx);
                let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                    panel.patch(json!({ "zones": { "exclude": before.clone() } }), window, cx)
                };
                panel.offer_undo(format!("Removed {name}"), undo, window, cx);
            }
        }));
    ui::row(name, None, remove, p)
}

/// The card: a row for each rule, "Add an app", then the apps left alone.
pub fn view(panel: &mut Panel, cx: &mut Context<Panel>) -> AnyElement {
    let p = palette(cx);
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    let known = readable(&panel.config.app_rules);
    let apps: Vec<Rule> = known.iter().map(|(_, r)| r.clone()).collect();
    icons::request(panel, &apps, cx);

    let mut rows: Vec<AnyElement> =
        known.iter().map(|(at, rule)| rule_row(panel, *at, rule, &screens, p, cx)).collect();
    if rows.is_empty() {
        let about = "Add an app to have its windows open in a place you choose";
        rows.push(ui::row("No apps yet", Some(about.into()), div(), p));
    }
    let adding: app_picker::Add = Rc::new(|panel, rule, _, window, cx| add(panel, rule, window, cx));
    let button = app_picker::button(panel, &apps, "Already has a place", adding, cx);
    rows.push(div().flex().px(px(16.)).py(px(12.)).child(button).into_any_element());

    rows.push(group_head("Left alone", p));
    let excluded = panel.config.zones.exclude.clone();
    if excluded.is_empty() {
        let about = "Choose Never include this app on a workspace's app to add one";
        rows.push(ui::row("No apps yet", Some(about.into()), div(), p));
    }
    rows.extend(excluded.into_iter().enumerate().map(|(i, name)| left_alone_row(i, name, p, cx)));

    let caption = ui::description("Seam puts these apps in place whenever they open, outside workspaces too", p);
    let body = div().flex().flex_col().gap(px(8.)).child(caption).child(card(rows, p));
    section("Where apps open", body, p)
}
