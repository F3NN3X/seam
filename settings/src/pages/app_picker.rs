//! "Add an app": a popover on the editor's button with the app windows open now
//! and the apps installed, searchable. A click adds the app and closes it.

use crate::icons;
use crate::pages::workspace_editor::add_app;
use crate::pages::workspaces::app_title;
use crate::panel::Panel;
use crate::theme::palette;
use crate::ui;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::*;
use seam::apps;
use seam::workspace::{Rule, Workspace};
use std::sync::Arc;

const WIDTH: f32 = 320.;
const LIST_HEIGHT: f32 = 320.;

pub struct Picker {
    pub open: bool,
    /// 0 is Open now, 1 is Installed.
    tab: usize,
    search: Entity<InputState>,
    query: SharedString,
    /// `None` while they're being read.
    open_now: Option<Vec<Rule>>,
    installed: Option<Vec<Rule>>,
    /// The Start menu's list is read once per panel.
    asked_installed: bool,
}

impl Picker {
    pub fn new(window: &mut Window, cx: &mut Context<Panel>) -> (Picker, Subscription) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search apps"));
        let typed = cx.subscribe_in(&search, window, |panel, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                panel.editing.picker.query = input.read(cx).value();
                cx.notify();
            }
        });
        let picker = Picker {
            open: false,
            tab: 0,
            search,
            query: SharedString::default(),
            open_now: None,
            installed: None,
            asked_installed: false,
        };
        (picker, typed)
    }
}

/// Read what's open each time it opens, and the Start menu's list the first time.
fn opened(panel: &mut Panel, open: bool, window: &mut Window, cx: &mut Context<Panel>) {
    let picker = &mut panel.editing.picker;
    picker.open = open;
    if !open {
        cx.notify();
        return;
    }
    picker.query = SharedString::default();
    picker.search.update(cx, |input, cx| input.set_value("", window, cx));
    picker.open_now = None;
    let exclude: Vec<String> = panel.config.zones.exclude.iter().map(|x| x.to_lowercase()).collect();
    cx.spawn(async move |this, cx| {
        let found = cx.background_executor().spawn(async move { apps::open_now(&exclude) }).await;
        let _ = this.update(cx, |panel, cx| {
            panel.editing.picker.open_now = Some(found);
            cx.notify();
        });
    })
    .detach();
    if !std::mem::replace(&mut panel.editing.picker.asked_installed, true) {
        cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async { apps::installed() }).await;
            let _ = this.update(cx, |panel, cx| {
                panel.editing.picker.installed = Some(found);
                cx.notify();
            });
        })
        .detach();
    }
    cx.notify();
}

/// What a list entry says: the Start menu's name, or for a window its app.
fn label(rule: &Rule, installed: bool) -> String {
    if installed {
        rule.title_hint.clone()
    } else {
        app_title(rule)
    }
}

/// The workspace already has a row for this app.
fn already(ws: &Workspace, rule: &Rule) -> bool {
    let same_exe = |r: &Rule| !rule.exe.is_empty() && r.exe.eq_ignore_ascii_case(&rule.exe);
    let same_aumid = |r: &Rule| rule.aumid.is_some() && r.aumid == rule.aumid;
    ws.windows.iter().any(|r| same_exe(r) || same_aumid(r))
}

/// One entry: the app as it would be added, whether it's in the workspace, and its icon.
type Entry = (Rule, bool, Option<Arc<Image>>);

/// The entries of the tab on show that match the search.
fn entries(panel: &mut Panel, ws: &Workspace, cx: &mut Context<Panel>) -> Option<Vec<Entry>> {
    let picker = &panel.editing.picker;
    let installed = picker.tab == 1;
    let list = if installed { picker.installed.clone() } else { picker.open_now.clone() }?;
    let query = picker.query.trim().to_lowercase();
    let matches = |r: &Rule| {
        query.is_empty()
            || label(r, installed).to_lowercase().contains(&query)
            || r.title_hint.to_lowercase().contains(&query)
            || r.exe.rsplit('\\').next().unwrap_or_default().to_lowercase().contains(&query)
    };
    let shown: Vec<Rule> = list.into_iter().filter(|r| matches(r)).collect();
    icons::request(panel, &shown, cx);
    Some(shown.into_iter().map(|r| (already(ws, &r), panel.icons.get(&r), r)).map(|(a, i, r)| (r, a, i)).collect())
}

/// "Add an app" and its popover.
pub fn button(panel: &mut Panel, ws: &Workspace, cx: &mut Context<Panel>) -> AnyElement {
    let open = panel.editing.picker.open;
    let found = if open { entries(panel, ws, cx) } else { None };
    let (tab, query, search) =
        (panel.editing.picker.tab, panel.editing.picker.query.clone(), panel.editing.picker.search.clone());
    let focus = search.read(cx).focus_handle(cx);
    let (view, id) = (cx.entity(), ws.id.clone());
    let p = palette(cx);
    let content = move |_: &mut _, _: &mut Window, _: &mut Context<_>| {
        let installed = tab == 1;
        let tabs = TabBar::new("picker-tabs")
            .child(Tab::new().label("Open now"))
            .child(Tab::new().label("Installed"))
            .selected_index(tab)
            .on_click({
                let view = view.clone();
                move |ix: &usize, _, cx| {
                    view.update(cx, |panel, cx| {
                        panel.editing.picker.tab = *ix;
                        cx.notify();
                    })
                }
            });
        let note = |text: String| div().px(px(8.)).py(px(12.)).child(ui::description(text, p));
        let list: AnyElement = match &found {
            None => note("Finding apps".into()).into_any_element(),
            Some(found) if found.is_empty() && !query.is_empty() => {
                note(format!("Nothing matches {query}")).into_any_element()
            }
            Some(found) if found.is_empty() && !installed => note("No app windows are open".into()).into_any_element(),
            Some(found) if found.is_empty() => note("No apps found in the Start menu".into()).into_any_element(),
            Some(found) => {
                let items = found.iter().enumerate().map(|(n, (rule, already, icon))| {
                    let (view, id, rule) = (view.clone(), id.clone(), rule.clone());
                    let name = label(&rule, installed);
                    // A window says which one it is. An app already there can be added again, for a second window.
                    let detail = match (already, installed) {
                        (true, _) => Some("In this workspace".to_string()),
                        (false, false) => Some(rule.title_hint.clone()),
                        (false, true) => None,
                    };
                    let icon = match icon {
                        Some(image) => div().flex_none().size(px(16.)).child(img(image.clone()).size(px(16.))),
                        None => div().flex_none().size(px(16.)).rounded(px(4.)).bg(p.muted),
                    };
                    div()
                        .id(("pick", n))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|d| d.bg(p.raised))
                        .child(icon)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().text_size(px(14.)).truncate().child(name))
                                .children(
                                    detail.map(|d| div().text_size(px(12.)).text_color(p.muted).truncate().child(d)),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            let rule = rule.clone();
                            view.update(cx, |panel, cx| {
                                panel.editing.picker.open = false;
                                add_app(panel, &id, Rule { launch: true, ..rule }, window, cx);
                            });
                        })
                        .into_any_element()
                });
                div()
                    .id("picker-list")
                    .flex()
                    .flex_col()
                    .max_h(px(LIST_HEIGHT))
                    .overflow_y_scroll()
                    .children(items)
                    .into_any_element()
            }
        };
        div().flex().flex_col().gap(px(8.)).w(px(WIDTH)).child(Input::new(&search)).child(tabs).child(list)
    };
    let changed = cx.entity();
    Popover::new("add-app")
        .open(open)
        .track_focus(&focus)
        .on_open_change(move |open, window, cx| {
            changed.update(cx, |panel, cx| opened(panel, *open, window, cx));
        })
        .trigger(ui::secondary("add-app-button", "Add an app", cx))
        .content(content)
        .into_any_element()
}
