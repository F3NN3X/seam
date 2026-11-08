//! The workspace editor: the selected workspace's apps in start order, where each
//! one goes, how it shows and how it starts. Every change is saved at once through
//! `workspace.update`, and the last one can be undone for 8 s.

use crate::icons::{self, Icons};
use crate::pages::app_picker::Picker;
use crate::pages::workspaces::{all, app_title, desk_sig};
use crate::panel::{error_text, refusal, Panel};
use crate::theme::{palette, Palette};
use crate::ui::{self, row, switch};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::state::Screen;
use seam::workspace::{self, Rule, Shown, Target, WinState, Workspace};
use seam::zones;
use serde_json::json;

const GRIP_WIDTH: f32 = 16.;
const GOES_TO_WIDTH: f32 = 200.;
const LAUNCH_WIDTH: f32 = 56.;
const MORE_WIDTH: f32 = 32.;
/// Launch details sit under the app's name: past the grip, the icon and their gaps.
const DETAILS_INDENT: f32 = GRIP_WIDTH + 32. + 16.;
const SHOW_AS: [(&str, Shown); 4] = [
    ("Normal", Shown::Normal),
    ("Minimized", Shown::Minimized),
    ("Maximized", Shown::Maximized),
    ("Full screen", Shown::Fullscreen),
];

/// What the editor keeps between frames.
pub struct Editing {
    /// The row whose launch details are open, by workspace id and row.
    pub expanded: Option<(String, usize)>,
    /// The delay slider of the open row, and whether it's being held.
    pub delay: Entity<SliderState>,
    holding_delay: bool,
    pub picker: Picker,
}

impl Editing {
    pub fn new(window: &mut Window, cx: &mut Context<Panel>) -> (Editing, Vec<Subscription>) {
        let delay = cx.new(|_| SliderState::new().min(0.).max(30.).step(0.5).default_value(0.));
        let (picker, picked) = Picker::new(window, cx);
        let held = cx.subscribe_in(&delay, window, |panel, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(_) => {
                panel.editing.holding_delay = true;
                cx.notify();
            }
            SliderEvent::Release(value) => {
                panel.editing.holding_delay = false;
                set_delay(panel, (value.end() * 1000.).round() as u32, window, cx);
            }
        });
        (Editing { expanded: None, delay, holding_delay: false, picker }, vec![held, picked])
    }
}

// ---- Saving

fn by_id(panel: &Panel, id: &str) -> Option<Workspace> {
    all(panel).into_iter().find(|w| w.id == id)
}

/// Show the workspace as it will be saved, before the tray answers.
fn show_locally(panel: &mut Panel, ws: &Workspace) {
    let sig = desk_sig(panel);
    let Some(desk) = panel.config.desks.get_mut(&sig) else { return };
    if let Some(slot) = desk.workspaces.iter_mut().find(|v| workspace::parse(v).is_some_and(|w| w.id == ws.id)) {
        *slot = workspace::to_value(ws);
    }
}

type Then = Box<dyn FnOnce(&mut Panel, &mut Window, &mut Context<Panel>)>;

/// Save a workspace through the tray, which checks it. `then` runs once it's saved.
fn send(panel: &mut Panel, ws: Workspace, then: Option<Then>, window: &mut Window, cx: &mut Context<Panel>) {
    show_locally(panel, &ws);
    cx.notify();
    let params = json!({ "id": ws.id, "workspace": workspace::to_value(&ws) });
    panel.call("workspace.update", params, window, cx, move |panel, result, window, cx| match (result, then) {
        (Err(e), _) => {
            window.push_notification(refusal("Workspace left as it was", &error_text(&e)), cx);
            panel.reload_config(window, cx);
        }
        (Ok(_), Some(then)) => then(panel, window, cx),
        (Ok(_), None) => panel.reload_config(window, cx),
    });
}

/// Change one workspace of this desk. It shows at once and the tray has the last
/// word. With a `note`, that change can be undone for 8 s.
pub fn update(
    panel: &mut Panel,
    id: &str,
    note: Option<String>,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Workspace),
) {
    let Some(before) = by_id(panel, id) else { return };
    let mut after = before.clone();
    change(&mut after);
    if after == before {
        return;
    }
    send(panel, after, None, window, cx);
    if let Some(note) = note {
        panel.offer_undo(note, move |panel, window, cx| send(panel, before.clone(), None, window, cx), window, cx);
    }
}

/// Change one row, with Undo.
fn update_row(
    panel: &mut Panel,
    id: &str,
    i: usize,
    note: impl FnOnce(&str) -> String,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Rule),
) {
    let Some(title) = by_id(panel, id).and_then(|ws| ws.windows.get(i).map(app_title)) else { return };
    update(panel, id, Some(note(&title)), window, cx, |ws| {
        if let Some(rule) = ws.windows.get_mut(i) {
            change(rule);
        }
    });
}

fn seconds(ms: u32) -> String {
    match ms % 1000 {
        0 => format!("{} s", ms / 1000),
        _ => format!("{:.1} s", ms as f32 / 1000.),
    }
}

fn set_delay(panel: &mut Panel, ms: u32, window: &mut Window, cx: &mut Context<Panel>) {
    let Some((id, i)) = panel.editing.expanded.clone() else { return };
    let ms = workspace::tame_delay(ms);
    let note = |title: &str| match ms {
        0 => format!("{title} opens at once"),
        ms => format!("{title} opens after {}", seconds(ms)),
    };
    update_row(panel, &id, i, note, window, cx, |rule| rule.delay_ms = ms);
}

/// Move a row to another place in the start order.
fn move_row(panel: &mut Panel, id: &str, from: usize, to: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(ws) = by_id(panel, id) else { return };
    if from == to || from >= ws.windows.len() || to >= ws.windows.len() {
        return;
    }
    let title = app_title(&ws.windows[from]);
    if panel.editing.expanded.as_ref().is_some_and(|(e, row)| e == id && *row == from) {
        panel.editing.expanded = Some((id.to_string(), to));
    }
    update(panel, id, Some(format!("Moved {title} to place {}", to + 1)), window, cx, |ws| {
        let rule = ws.windows.remove(from);
        ws.windows.insert(to, rule);
    });
}

fn remove_row(panel: &mut Panel, id: &str, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(title) = by_id(panel, id).and_then(|ws| ws.windows.get(i).map(app_title)) else { return };
    panel.editing.expanded = None;
    update(panel, id, Some(format!("Removed {title}")), window, cx, |ws| {
        ws.windows.remove(i);
    });
}

/// Add an app the picker offered, at the end of the start order.
pub fn add_app(panel: &mut Panel, id: &str, rule: Rule, window: &mut Window, cx: &mut Context<Panel>) {
    let title = app_title(&rule);
    update(panel, id, Some(format!("Added {title}")), window, cx, |ws| ws.windows.push(rule));
}

/// The exe name the "Never manage" list holds for this row, if it has one. A Store
/// app added from Installed has no exe yet.
fn exe_name(rule: &Rule) -> Option<String> {
    let name = rule.exe.rsplit('\\').next().unwrap_or(&rule.exe).to_lowercase();
    (!name.is_empty()).then_some(name)
}

/// Remove the row and add its app to the list the zones and capture leave alone.
/// Undo puts both back.
fn never_include(panel: &mut Panel, id: &str, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(before) = by_id(panel, id) else { return };
    let Some((rule, name)) = before.windows.get(i).and_then(|r| Some((r, exe_name(r)?))) else { return };
    let title = app_title(rule);
    let mut after = before.clone();
    after.windows.remove(i);
    let old_list = panel.config.zones.exclude.clone();
    let mut list = old_list.clone();
    if !list.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
        list.push(name);
    }
    panel.editing.expanded = None;
    let exclude = move |list: Vec<String>| -> Then {
        Box::new(move |panel, window, cx| panel.patch(json!({ "zones": { "exclude": list } }), window, cx))
    };
    send(panel, after, Some(exclude(list)), window, cx);
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        send(panel, before.clone(), Some(exclude(old_list.clone())), window, cx);
    };
    panel.offer_undo(format!("Never including {title}"), undo, window, cx);
}

/// "Where it is now": the tray captures the row's window and saves its place.
fn where_it_is(panel: &mut Panel, ws: &Workspace, i: usize, window: &mut Window, cx: &mut Context<Panel>) {
    let (before, title) = (ws.clone(), ws.windows.get(i).map(app_title).unwrap_or_default());
    let params = json!({ "into": ws.id, "row": i });
    panel.call("workspace.capture", params, window, cx, move |panel, result, window, cx| match result {
        Ok(_) => {
            panel.reload_config(window, cx);
            let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                send(panel, before.clone(), None, window, cx)
            };
            panel.offer_undo(format!("{title} goes where it is now"), undo, window, cx);
        }
        Err(e) => {
            window.push_notification(refusal(&format!("Where {title} goes stays as it was"), &error_text(&e)), cx)
        }
    });
}

// ---- What is drawn

/// A place a window can be sent to, as the Goes to menu lists it.
#[derive(Clone)]
struct Place {
    label: String,
    /// The screen it's on. `None` leaves a window's screen as it was.
    monitor: Option<String>,
    target: Target,
}

fn places(panel: &Panel) -> Vec<Place> {
    let keep =
        Place { label: "Keep where it is".into(), monitor: None, target: Target::State { state: WinState::Keep } };
    let mut places = vec![keep];
    let sig = desk_sig(panel);
    let desk = panel.config.desks.get(&sig);
    for s in panel.state.iter().flat_map(|s| s.screens.iter()) {
        let layout = desk.and_then(|d| d.zones.get(&s.id)).and_then(zones::layout_of);
        let count = layout.map_or(0, |l| zones::leaf_count(&l.tree));
        for zone in 1..=count as u32 {
            let target = Target::Zone { zone, span_to: None };
            places.push(Place {
                label: format!("Screen {}, zone {zone}", s.number),
                monitor: Some(s.id.clone()),
                target,
            });
        }
        let full = Target::State { state: WinState::Max };
        places.push(Place { label: format!("Screen {}, full", s.number), monitor: Some(s.id.clone()), target: full });
    }
    places
}

/// Where a rule sends its window, in the words of the menu.
fn goes_to(rule: &Rule, screens: &[Screen]) -> String {
    let number = screens.iter().find(|s| s.id == rule.monitor).map(|s| s.number);
    match (&rule.target, number) {
        (Target::State { state: WinState::Keep }, _) => "Keep where it is".into(),
        (_, None) => "Screen not connected".into(),
        (Target::Zone { zone, span_to: None }, Some(n)) => format!("Screen {n}, zone {zone}"),
        (Target::Zone { zone, span_to: Some(to) }, Some(n)) => format!("Screen {n}, zones {zone} to {to}"),
        (Target::State { .. }, Some(n)) => format!("Screen {n}, full"),
        (Target::Rect { .. }, Some(n)) => format!("Screen {n}, own size"),
    }
}

fn exe_line(rule: &Rule) -> String {
    let lower = rule.exe.to_lowercase();
    if rule.aumid.is_some() && (lower.is_empty() || lower.contains(r"\windowsapps\")) {
        return "Packaged app".into();
    }
    rule.exe.rsplit('\\').next().unwrap_or(&rule.exe).to_string()
}

/// The app's icon, or its first letter while there is none.
fn app_icon(rule: &Rule, icons: &Icons, p: Palette) -> Div {
    let tile = div().flex().flex_none().items_center().justify_center().size(px(32.)).rounded(px(8.)).bg(p.raised);
    match icons.get(rule) {
        Some(image) => tile.child(img(image).size(px(16.))),
        None => {
            let letter = app_title(rule).chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            tile.text_size(px(14.)).text_color(p.secondary).child(letter)
        }
    }
}

/// A row being dragged to another place in the start order.
#[derive(Clone)]
struct DragRow {
    workspace: String,
    from: usize,
    title: SharedString,
}

impl Render for DragRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .px(px(12.))
            .py(px(4.))
            .rounded(px(8.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hairline_strong)
            .text_size(px(13.))
            .text_color(p.text)
            .child(self.title.clone())
    }
}

/// The row's focus, keyed by its app rather than its place, so it follows the row
/// when Alt+arrow moves it.
fn row_focus(ws: &Workspace, i: usize, window: &mut Window, cx: &mut App) -> FocusHandle {
    let key = icons::identity(&ws.windows[i]);
    let nth = ws.windows[..i].iter().filter(|r| icons::identity(r) == key).count();
    let id = ElementId::Name(format!("ws-row-{}-{key}-{nth}", ws.id).into());
    window.use_keyed_state(id, cx, |_, cx| cx.focus_handle()).read(cx).clone()
}

/// Where a wait ends a stage: a thin line with "then wait" at its left.
fn stage_line(p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .pb(px(8.))
        .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child("then wait"))
        .child(div().flex_1().h(px(1.)).bg(p.hairline_strong))
}

fn goes_menu(panel: &Panel, i: usize, ws: &Workspace, screens: &[Screen], cx: &mut Context<Panel>) -> AnyElement {
    let rule = &ws.windows[i];
    let current = goes_to(rule, screens);
    let weak = cx.entity().downgrade();
    let (options, shown, id, here) = (places(panel), current.clone(), ws.id.clone(), ws.clone());
    let menu = move |mut menu: gpui_kit::component::menu::PopupMenu, _: &mut Window, _: &mut Context<_>| {
        for (n, place) in options.iter().enumerate() {
            let (weak, place, id) = (weak.clone(), place.clone(), id.clone());
            let checked = place.label == shown;
            let item = PopupMenuItem::new(place.label.clone()).checked(checked).on_click(move |_, window, cx| {
                let (place, id) = (place.clone(), id.clone());
                let _ = weak.update(cx, |panel, cx| {
                    let note = |title: &str| format!("{title} goes to {}", place.label);
                    let (target, monitor) = (place.target.clone(), place.monitor.clone());
                    update_row(panel, &id, i, note, window, cx, |rule| {
                        rule.set_target(target);
                        if let Some(monitor) = monitor {
                            rule.monitor = monitor;
                        }
                    });
                });
            });
            menu = menu.item(item);
            if n == 0 {
                menu = menu.separator();
            }
        }
        let (weak, here) = (weak.clone(), here.clone());
        menu.separator().item(PopupMenuItem::new("Where it is now").on_click(move |_, window, cx| {
            let here = here.clone();
            let _ = weak.update(cx, |panel, cx| where_it_is(panel, &here, i, window, cx));
        }))
    };
    ui::secondary(("goes-to", i), current, cx)
        .dropdown_caret(true)
        .w(px(GOES_TO_WIDTH))
        .dropdown_menu(menu)
        .into_any_element()
}

fn more_menu(i: usize, ws: &Workspace, title: &str, cx: &mut Context<Panel>) -> AnyElement {
    let weak = cx.entity().downgrade();
    let (id, excludable) = (ws.id.clone(), exe_name(&ws.windows[i]).is_some());
    ui::secondary(("row-more", i), "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {title}"))
        .dropdown_menu(move |menu, _, _| {
            let (removing, never, id_r, id_n) = (weak.clone(), weak.clone(), id.clone(), id.clone());
            let menu = menu.item(PopupMenuItem::new("Remove").on_click(move |_, window, cx| {
                let _ = removing.update(cx, |panel, cx| remove_row(panel, &id_r, i, window, cx));
            }));
            if !excludable {
                return menu;
            }
            menu.item(PopupMenuItem::new("Never include this app").on_click(move |_, window, cx| {
                let _ = never.update(cx, |panel, cx| never_include(panel, &id_n, i, window, cx));
            }))
        })
        .into_any_element()
}

/// Show as, the delay and the wait, under an app's row.
fn details(panel: &mut Panel, i: usize, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let rule = &ws.windows[i];
    let view = cx.entity();
    let id = ws.id.clone();
    let at = SHOW_AS.iter().position(|(_, s)| *s == rule.shown).unwrap_or(0);
    let labels: Vec<&'static str> = SHOW_AS.iter().map(|(label, _)| *label).collect();
    let show = ui::segmented(("show-as", i), &labels, at, window, cx, {
        let (view, id) = (view.clone(), id.clone());
        move |k, window, cx| {
            let (label, shown) = SHOW_AS[k.min(SHOW_AS.len() - 1)];
            let id = id.clone();
            view.update(cx, |panel, cx| {
                let note = |title: &str| format!("{title} shows {}", label.to_lowercase());
                update_row(panel, &id, i, note, window, cx, |rule| rule.set_shown(shown));
            });
        }
    });

    // The slider follows the saved delay unless it's being held.
    let saved = workspace::tame_delay(rule.delay_ms) as f32 / 1000.;
    if !panel.editing.holding_delay && panel.editing.delay.read(cx).value().end() != saved {
        panel.editing.delay.update(cx, |slider, cx| slider.set_value(saved, window, cx));
    }
    let value = (panel.editing.delay.read(cx).value().end() * 1000.).round() as u32;
    let ends = |text: &'static str| div().text_size(px(12.)).text_color(p.muted).child(text);
    let slider = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ends("0 s"))
        .child(Slider::new(&panel.editing.delay).horizontal().w(px(160.)))
        .child(ends("30 s"));
    let wait = switch(("wait-for-window", i), rule.wait_for_window, window, cx, move |on, window, cx| {
        let id = id.clone();
        view.update(cx, |panel, cx| {
            let note = |title: &str| match on {
                true => format!("The next app waits for {title}"),
                false => format!("The next app doesn't wait for {title}"),
            };
            update_row(panel, &id, i, note, window, cx, |rule| rule.wait_for_window = on);
        });
    });
    let delay_about = match value {
        0 => "Opens as soon as its stage starts".to_string(),
        ms => format!("{} after its stage starts", seconds(ms)),
    };
    div()
        .flex()
        .flex_col()
        .pl(px(DETAILS_INDENT))
        .pb(px(8.))
        .child(row("Show as", None, show, p))
        .child(row("Delay before opening", Some(delay_about.into()), slider, p))
        .child(row(
            "Wait for this app's window before opening the next",
            Some("The apps below start once it's open, or after 15 s".into()),
            wait,
            p,
        ))
}

fn window_row(
    panel: &mut Panel,
    i: usize,
    ws: &Workspace,
    screens: &[Screen],
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    let rule = &ws.windows[i];
    let title = app_title(rule);
    let id = ws.id.clone();
    let open = panel.editing.expanded.as_ref().is_some_and(|(e, row)| *e == id && *row == i);
    let focus = row_focus(ws, i, window, cx);

    let drag = DragRow { workspace: id.clone(), from: i, title: title.clone().into() };
    let grip = div()
        .id(("row-grip", i))
        .flex()
        .flex_none()
        .w(px(GRIP_WIDTH))
        .cursor_grab()
        .child(Icon::new(assets::IconName::GripVertical).size(px(16.)).text_color(p.muted))
        .tooltip(|window, cx| Tooltip::new("Drag to change the start order, or Alt+Up and Alt+Down").build(window, cx))
        .on_drag(drag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        });

    // What will run is shown in full on hover. A click opens the launch details.
    let full_path = SharedString::from(if rule.exe.is_empty() { exe_line(rule) } else { rule.exe.clone() });
    let chevron = if open { assets::IconName::ChevronDown } else { assets::IconName::ChevronRight };
    let toggle = id.clone();
    let name = div()
        .id(("app-name", i))
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(full_path.clone()).build(window, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_size(px(14.)).truncate().child(title.clone()))
                .child(div().text_size(px(12.)).text_color(p.muted).truncate().child(exe_line(rule))),
        )
        .child(Icon::new(chevron).size(px(16.)).text_color(p.muted))
        .on_click(cx.listener(move |panel, _, _, cx| {
            let this = Some((toggle.clone(), i));
            panel.editing.expanded = if panel.editing.expanded == this { None } else { this };
            cx.notify();
        }));

    let view = cx.entity();
    let launch_id = id.clone();
    let launch = Checkbox::new(("launch", i))
        .checked(rule.launch)
        .accessibility_label(format!("Start {title}"))
        .on_click(move |on, window, cx| {
            let (on, id) = (*on, launch_id.clone());
            view.update(cx, |panel, cx| {
                let note = |title: &str| match on {
                    true => format!("{title} starts with the workspace"),
                    false => format!("{title} doesn't start with the workspace"),
                };
                update_row(panel, &id, i, note, window, cx, |rule| rule.launch = on);
            });
        });

    let main = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(8.))
        .child(grip)
        .child(app_icon(rule, &panel.icons, p))
        .child(name)
        .child(div().flex_none().w(px(GOES_TO_WIDTH)).child(goes_menu(panel, i, ws, screens, cx)))
        .child(div().flex().flex_none().justify_center().w(px(LAUNCH_WIDTH)).child(launch))
        .child(div().flex().flex_none().justify_end().w(px(MORE_WIDTH)).child(more_menu(i, ws, &title, cx)));

    let (keys, dropped, last) = (id.clone(), id.clone(), ws.windows.len() - 1);
    let mut body = div()
        .id(("ws-row", i))
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .flex_col()
        .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
            if !e.keystroke.modifiers.alt {
                return;
            }
            let to = match e.keystroke.key.as_str() {
                "up" => i.checked_sub(1),
                "down" => (i < last).then_some(i + 1),
                _ => None,
            };
            if let Some(to) = to {
                cx.stop_propagation();
                move_row(panel, &keys, i, to, window, cx);
            }
        }))
        .drag_over::<DragRow>(move |d, _, _, _| d.bg(p.raised))
        .on_drop(cx.listener(move |panel, drag: &DragRow, window, cx| {
            if drag.workspace == dropped {
                move_row(panel, &dropped, drag.from, i, window, cx);
            }
        }))
        .child(main);
    if open {
        body = body.child(details(panel, i, ws, window, cx));
    }
    if rule.wait_for_window && i < last {
        body = body.child(stage_line(p));
    }
    body.into_any_element()
}

fn table_head(p: Palette) -> AnyElement {
    let head = |text: &'static str| div().text_size(px(13.)).text_color(p.muted).child(text);
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(10.))
        .child(div().flex_none().w(px(GRIP_WIDTH)))
        .child(div().flex_1().child(head("App, in start order")))
        .child(div().flex_none().w(px(GOES_TO_WIDTH)).child(head("Goes to")))
        .child(div().flex().flex_none().justify_center().w(px(LAUNCH_WIDTH)).child(head("Launch")))
        .child(div().flex_none().w(px(MORE_WIDTH)))
        .into_any_element()
}

/// The table: a head, then one row per app in start order.
pub fn rows(panel: &mut Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let p = palette(cx);
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    icons::request(panel, &ws.windows, cx);
    let mut rows = vec![table_head(p)];
    if ws.windows.is_empty() {
        let text = "No apps yet. Add one, or arrange your windows and choose Update from screen";
        rows.push(div().px(px(16.)).py(px(16.)).child(ui::description(text, p)).into_any_element());
    }
    for i in 0..ws.windows.len() {
        rows.push(window_row(panel, i, ws, &screens, window, cx));
    }
    rows
}
