//! "Add an app": a popover on the editor's button with the app windows open now
//! and the apps installed, searchable. A click adds the app and closes it. The
//! workspace editor and the Zones page's "Where apps open" both use it. The workspace
//! editor's has a third tab, Other, for a folder, web links, a terminal or an editor
//! folder (D731), which open a dialog.

use crate::icons;
use crate::pages::workspaces::app_title;
use crate::panel::Panel;
use crate::theme::{Palette, palette, tokens};
use crate::ui;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use seam::apps;
use seam::workspace::Rule;
use std::rc::Rc;
use std::sync::Arc;

pub mod items;
pub mod words;
use items::Extras;

const WIDTH: f32 = 320.;
const LIST_HEIGHT: f32 = 320.;

pub struct Picker {
    pub open: bool,
    /// 0 is Open now, 1 is Installed, 2 is Other.
    tab: usize,
    search: Entity<InputState>,
    query: SharedString,
    /// `None` while they're being read. Each window open now comes with its handle.
    open_now: Option<Vec<(Rule, isize)>>,
    installed: Option<Vec<Rule>>,
    /// What Other needs to know about this PC, read with the Start menu's list.
    pub extras: Option<Extras>,
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
            extras: None,
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
    read_installed(panel, cx);
    cx.notify();
}

/// Read the Start menu's list, and with it what Other offers, once per panel, off the UI thread.
pub fn read_installed(panel: &mut Panel, cx: &mut Context<Panel>) {
    if std::mem::replace(&mut panel.editing.picker.asked_installed, true) {
        return;
    }
    cx.spawn(async move |this, cx| {
        let found = cx
            .background_executor()
            .spawn(async {
                let installed = apps::installed();
                let extras = Extras::read(&installed);
                (installed, extras)
            })
            .await;
        let _ = this.update(cx, |panel, cx| {
            panel.editing.picker.installed = Some(found.0);
            panel.editing.picker.extras = Some(found.1);
            cx.notify();
        });
    })
    .detach();
}

/// What a list entry says: the Start menu's name, or for a window its app.
fn label(rule: &Rule, installed: bool) -> String {
    if installed { rule.title_hint.clone() } else { app_title(rule) }
}

/// What the picker hands an app to once it's picked, with the window it was picked
/// from on Open now.
pub type Add = Rc<dyn Fn(&mut Panel, Rule, Option<isize>, &mut Window, &mut Context<Panel>)>;

/// The apps already there already have a row or a rule for this app. An item doesn't make its
/// app taken (D731).
fn already(taken: &[Rule], rule: &Rule) -> bool {
    let same_exe = |r: &Rule| !rule.exe.is_empty() && r.exe.eq_ignore_ascii_case(&rule.exe);
    let same_aumid = |r: &Rule| rule.aumid.is_some() && r.aumid == rule.aumid;
    taken.iter().filter(|r| r.item.is_none()).any(|r| same_exe(r) || same_aumid(r))
}

/// One entry: the app as it would be added, whether it's there already, its icon, and
/// on Open now its window.
type Entry = (Rule, bool, Option<Arc<Image>>, Option<isize>);

/// The entries of the tab on show that match the search.
fn entries(panel: &mut Panel, taken: &[Rule], cx: &mut Context<Panel>) -> Option<Vec<Entry>> {
    let picker = &panel.editing.picker;
    let installed = picker.tab == 1;
    let list: Vec<(Rule, Option<isize>)> = if installed {
        picker.installed.clone()?.into_iter().map(|r| (r, None)).collect()
    } else {
        picker.open_now.clone()?.into_iter().map(|(r, h)| (r, Some(h))).collect()
    };
    let query = picker.query.trim().to_lowercase();
    let matches = |r: &Rule| {
        query.is_empty()
            || label(r, installed).to_lowercase().contains(&query)
            || r.title_hint.to_lowercase().contains(&query)
            || r.exe.rsplit('\\').next().unwrap_or_default().to_lowercase().contains(&query)
    };
    let (shown, hwnds): (Vec<Rule>, Vec<Option<isize>>) = list.into_iter().filter(|(r, _)| matches(r)).unzip();
    icons::request(panel, &shown, cx);
    let entry = |(r, h): (Rule, Option<isize>)| (already(taken, &r), panel.icons.get(&r), r, h);
    Some(shown.into_iter().zip(hwnds).map(entry).map(|(a, i, r, h)| (r, a, i, h)).collect())
}

/// What the Other tab adds to: the workspace's name, and what gets the row a dialog made.
pub type Other = (String, items::Done);

/// "Add an app" and its popover. `taken` are the apps already there, which the list
/// says `there` of, and `add` gets the one picked. With `other`, a third tab offers items.
pub fn button(
    panel: &mut Panel,
    taken: &[Rule],
    there: &'static str,
    add: Add,
    other: Option<Other>,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let open = panel.editing.picker.open;
    let tab = panel.editing.picker.tab.min(if other.is_some() { 2 } else { 1 });
    panel.editing.picker.tab = tab;
    let found = if open && tab != 2 { entries(panel, taken, cx) } else { None };
    let (query, search) = (panel.editing.picker.query.clone(), panel.editing.picker.search.clone());
    let extras = panel.editing.picker.extras.clone();
    let focus = search.read(cx).focus_handle(cx);
    let view = cx.entity();
    let p = palette(cx);
    let content = move |_: &mut _, window: &mut Window, cx: &mut Context<_>| {
        let installed = tab == 1;
        let tabs = TabBar::new("picker-tabs")
            .child(Tab::new().label("Open now"))
            .child(Tab::new().label("Installed"))
            .children(other.as_ref().map(|_| Tab::new().label("Other")))
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
            _ if tab == 2 => match (&extras, &other) {
                (Some(extras), Some(other)) => other_list(extras, other, &query, &view, p, window, cx),
                _ => note("Finding apps".into()).into_any_element(),
            },
            None => note("Finding apps".into()).into_any_element(),
            Some(found) if found.is_empty() && !query.is_empty() => {
                note(format!("Nothing matches {query}")).into_any_element()
            }
            Some(found) if found.is_empty() && !installed => note("No app windows are open".into()).into_any_element(),
            Some(found) if found.is_empty() => note("No apps found in the Start menu".into()).into_any_element(),
            Some(found) => {
                let last = found.len() - 1;
                let items = found.iter().enumerate().map(|(n, (rule, already, icon, hwnd))| {
                    let (view, add, rule, hwnd) = (view.clone(), add.clone(), rule.clone(), *hwnd);
                    let name = label(&rule, installed);
                    let before = (n > 0).then(|| ui::item_focus(("pick", n - 1), window, cx));
                    let after = (n < last).then(|| ui::item_focus(("pick", n + 1), window, cx));
                    let target = rule.clone();
                    let pick = move |window: &mut Window, cx: &mut App| {
                        let (rule, add) = (target.clone(), add.clone());
                        view.update(cx, |panel, cx| {
                            panel.editing.picker.open = false;
                            add(panel, Rule { launch: true, ..rule }, hwnd, window, cx);
                        });
                    };
                    // A window says which one it is. An app already there can be added again, for a second window.
                    let detail = match (already, installed) {
                        (true, _) => Some(there.to_string()),
                        (false, false) => Some(rule.title_hint.clone()),
                        (false, true) => None,
                    };
                    let icon = match icon {
                        Some(image) => div().flex_none().size(px(16.)).child(img(image.clone()).size(px(16.))),
                        None => div().flex_none().size(px(16.)).rounded(px(tokens::RADIUS_4)).bg(p.surface_zone),
                    };
                    let row = ui::selectable_ringed(("pick", n), name.clone(), false, (6., 0.), window, cx, pick);
                    ui::arrow_keys(row, before, after)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|d| ui::lit(d, p.surface_raised, p))
                        .child(icon)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().text_size(crate::theme::scaled(14.)).truncate().child(name))
                                .children(detail.map(|d| {
                                    div()
                                        .text_size(crate::theme::scaled(12.))
                                        .text_color(p.text_muted)
                                        .truncate()
                                        .child(d)
                                })),
                        )
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

/// The Other tab: the four kinds of item, a kind dimmed with its reason when this PC can't have it.
fn other_list(
    extras: &Extras,
    (workspace, done): &Other,
    query: &str,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let query = query.trim().to_lowercase();
    let shown = items::KINDS.iter().filter(|k| query.is_empty() || k.name().to_lowercase().contains(&query));
    let mut list = div().id("picker-list").flex().flex_col();
    for (n, kind) in shown.enumerate() {
        let icon = match kind {
            items::Kind::Folder => assets::IconName::Folder,
            items::Kind::Web => assets::IconName::Globe,
            items::Kind::Terminal => assets::IconName::SquareTerminal,
            items::Kind::Editor => assets::IconName::FileText,
        };
        let texts = div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(div().text_size(crate::theme::scaled(14.)).truncate().child(kind.name()))
            .child(div().text_size(crate::theme::scaled(12.)).text_color(p.text_muted).child(kind.about(extras)));
        let icon = Icon::new(icon).size(px(tokens::SIZE_ICON)).text_color(p.text_secondary);
        let entry = div().flex().items_center().gap(px(8.)).px(px(8.)).py(px(4.)).rounded(px(6.));
        if let Some(why) = kind.unavailable(extras) {
            let dimmed = entry.id(("other-off", n)).opacity(0.5).child(icon).child(texts);
            list = list.child(dimmed.tooltip(move |window, cx| Tooltip::new(why).build(window, cx)));
            continue;
        }
        let (view, extras, done, name, kind) = (view.clone(), extras.clone(), done.clone(), workspace.clone(), *kind);
        let pick = move |window: &mut Window, cx: &mut App| {
            let (extras, done, name) = (extras.clone(), done.clone(), name.clone());
            view.update(cx, |panel, cx| {
                panel.editing.picker.open = false;
                items::open(kind, extras, (name, None), done, window, cx);
            });
        };
        let row = ui::selectable_ringed(("pick-other", n), kind.name(), false, (6., 0.), window, cx, pick);
        list = list.child(
            row.flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|d| ui::lit(d, p.surface_raised, p))
                .child(icon)
                .child(texts),
        );
    }
    list.into_any_element()
}
