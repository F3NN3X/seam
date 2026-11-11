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
use gpui_kit::component::{ActiveTheme, Icon, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::library;
use seam::state::Screen;
use seam::workspace::{self, RowState, Rule, Shown, Target, WinState, Workspace};
use seam::zones::{self, Template, ZoneLayout};
use serde_json::{json, Value};

const GRIP_WIDTH: f32 = 16.;
const ICON_WIDTH: f32 = 32.;
/// The name, Now and Goes to columns start at the width their usual text needs (see
/// `Cols`), and share both the room to spare and the room missing, by these weights,
/// each down to its least, so the "..." always shows.
const NAME_GROW: f32 = 2.;
const NAME_LEAST: f32 = 40.;
const STATUS_GROW: f32 = 1.;
const STATUS_LEAST: f32 = 16.;
const GOES_TO_GROW: f32 = 2.;
const GOES_TO_LEAST: f32 = 48.;
const MORE_WIDTH: f32 = 32.;
/// What the columns are measured on: a usual app name, the longest Now word that
/// should show in full, and a usual place.
const USUAL_NAME: &str = "Visual Studio Code";
const LONGEST_STATUS: &str = "Running in background";
const USUAL_PLACE: &str = "Screen 2, zone 1";
/// Around the text: the name's gap and chevron, the status dot and its gap, and the
/// Goes to button's border, padding, gap and caret.
const NAME_CHROME: f32 = 4. + 16.;
const STATUS_CHROME: f32 = 8. + 8.;
const GOES_TO_CHROME: f32 = 2. + 20. + 8. + 16.;

/// The table's measured widths: where the flexible columns start, and Launch, as wide as its head.
#[derive(Clone, Copy)]
pub struct Cols {
    name: f32,
    status: f32,
    goes: f32,
    launch: f32,
}

impl Cols {
    pub fn measure(window: &Window, cx: &App) -> Cols {
        let width = |text: &str, size: f32| text_width(text, size, window, cx).ceil();
        Cols {
            name: width(USUAL_NAME, 14.) + NAME_CHROME,
            status: width(LONGEST_STATUS, 12.) + STATUS_CHROME,
            goes: width(USUAL_PLACE, 14.) + GOES_TO_CHROME,
            launch: width("Launch", 13.).max(16.),
        }
    }
}

/// How wide `text` is at `size` in the panel's font.
fn text_width(text: &str, size: f32, window: &Window, cx: &App) -> f32 {
    let mut style = window.text_style();
    style.font_family = cx.theme().font_family.clone();
    let run = style.to_run(text.len());
    let line = window.text_system().shape_line(SharedString::from(text.to_string()), px(size), &[run], None);
    f32::from(line.width)
}
/// Launch details sit under the app's name: past the grip, the icon and their gaps.
const DETAILS_INDENT: f32 = GRIP_WIDTH + ICON_WIDTH + 16.;
/// A column that starts at `width`, grows by `grow` and shrinks down to `least`.
fn column(width: f32, grow: f32, least: f32) -> Div {
    div().flex_grow(grow).flex_shrink(1.).flex_basis(px(width)).min_w(px(least))
}

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
    /// The workspace whose Zones section is open. Closed until opened.
    zones_open: Option<String>,
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
        (Editing { expanded: None, delay, holding_delay: false, picker, zones_open: None }, vec![held, picked])
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
    // `zones` is sent even when empty, or the tray keeps the layouts it has.
    let mut sent = workspace::to_value(&ws);
    sent["zones"] = json!(ws.zones);
    let params = json!({ "id": ws.id, "workspace": sent });
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
pub struct Place {
    pub label: String,
    /// The screen it's on. `None` leaves a window's screen as it was.
    pub monitor: Option<String>,
    pub target: Target,
}

pub fn places(panel: &Panel) -> Vec<Place> {
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
pub fn goes_to(rule: &Rule, screens: &[Screen]) -> String {
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

pub fn exe_line(rule: &Rule) -> String {
    let lower = rule.exe.to_lowercase();
    if rule.aumid.is_some() && (lower.is_empty() || lower.contains(r"\windowsapps\")) {
        return "Packaged app".into();
    }
    rule.exe.rsplit('\\').next().unwrap_or(&rule.exe).to_string()
}

/// The app's icon, or its first letter while there is none.
pub fn app_icon(rule: &Rule, icons: &Icons, p: Palette) -> Div {
    let tile =
        div().flex().flex_none().items_center().justify_center().size(px(ICON_WIDTH)).rounded(px(8.)).bg(p.raised);
    match icons.get(rule) {
        Some(image) => tile.child(img(image).size(px(16.))),
        None => {
            let letter = app_title(rule).chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            tile.text_size(px(14.)).text_color(p.secondary).child(letter)
        }
    }
}

/// What the tray last said about this row's app, while it's about this workspace.
pub fn row_state(panel: &Panel, ws: &Workspace, i: usize) -> Option<RowState> {
    let (id, rows) = panel.status.as_ref()?;
    let row = rows.iter().find(|r| r.rule_index == i).filter(|_| *id == ws.id)?;
    Some(row.state)
}

/// The status cell: a dot and a word, in full on hover when it's cut short.
fn status_cell(i: usize, state: Option<RowState>, p: Palette) -> AnyElement {
    let (dot, word) = match state {
        Some(RowState::InPlace) => (ui::dot(p.positive, true), "In place"),
        Some(RowState::Moved) => (ui::dot(p.secondary, false), "Moved"),
        Some(RowState::Minimized) => (ui::dot(p.secondary, false), "Minimized"),
        Some(RowState::NotRunning) => (ui::dot(p.muted, true), "Not running"),
        Some(RowState::Background) => (ui::dot(p.muted, false), "Running in background"),
        Some(RowState::CantMove) => (ui::dot(p.negative, false), "Can't move: runs as administrator"),
        None => return div().into_any_element(),
    };
    let cell = div().id(("status", i)).flex().items_center().gap(px(8.)).min_w_0();
    cell.tooltip(move |window, cx| Tooltip::new(word).build(window, cx))
        .child(dot.flex_none())
        .child(div().min_w_0().text_size(px(12.)).text_color(p.secondary).truncate().child(word))
        .into_any_element()
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

fn goes_menu(
    panel: &Panel,
    i: usize,
    ws: &Workspace,
    screens: &[Screen],
    title: &str,
    cx: &mut Context<Panel>,
) -> AnyElement {
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
    ui::secondary(("goes-to", i), current.clone(), cx)
        .dropdown_caret(true)
        .accessibility_label(format!("Where {title} goes: {current}"))
        .w_full()
        .tooltip(current)
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
    let show = ui::segmented(("show-as", i), "Show as", &labels, at, window, cx, {
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
    let wait = switch(
        ("wait-for-window", i),
        "Wait for this app's window before opening the next",
        rule.wait_for_window,
        window,
        cx,
        move |on, window, cx| {
            let id = id.clone();
            view.update(cx, |panel, cx| {
                let note = |title: &str| match on {
                    true => format!("The next app waits for {title}"),
                    false => format!("The next app doesn't wait for {title}"),
                };
                update_row(panel, &id, i, note, window, cx, |rule| rule.wait_for_window = on);
            });
        },
    );
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
    (i, cols): (usize, Cols),
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
    let path = if rule.exe.is_empty() { exe_line(rule) } else { rule.exe.clone() };
    let full_path = SharedString::from(format!("{title}: {path}"));
    let chevron = if open { assets::IconName::ChevronDown } else { assets::IconName::ChevronRight };
    let toggle = id.clone();
    let name = div()
        .id(("app-name", i))
        .flex()
        .flex_grow(NAME_GROW)
        .flex_shrink(1.)
        .flex_basis(px(cols.name))
        .min_w(px(NAME_LEAST))
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
        .child(Icon::new(chevron).flex_none().size(px(16.)).text_color(p.muted))
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

    let state = row_state(panel, ws, i);
    // A closed app that starts with the workspace says so under where it goes.
    let opens = state == Some(RowState::NotRunning) && rule.launch && ws.launch_missing;
    let goes = column(cols.goes, GOES_TO_GROW, GOES_TO_LEAST)
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(goes_menu(panel, i, ws, screens, &title, cx))
        .when(opens, |d| d.child(div().text_size(px(12.)).text_color(p.muted).truncate().child("Will open")));
    let main = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(8.))
        .child(grip)
        .child(app_icon(rule, &panel.icons, p))
        .child(name)
        .child(column(cols.status, STATUS_GROW, STATUS_LEAST).child(status_cell(i, state, p)))
        .child(goes)
        .child(div().flex().flex_none().justify_center().w(px(cols.launch)).child(launch))
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

fn table_head(cols: Cols, p: Palette) -> AnyElement {
    let head = |text: &'static str| div().text_size(px(13.)).text_color(p.muted).truncate().child(text);
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(10.))
        .child(div().flex_none().w(px(GRIP_WIDTH)))
        // The icon's place, so the columns line up with the rows' as they shrink.
        .child(div().flex_none().w(px(ICON_WIDTH)))
        .child(column(cols.name, NAME_GROW, NAME_LEAST).child(head("App, in start order")))
        .child(column(cols.status, STATUS_GROW, STATUS_LEAST).child(head("Now")))
        .child(column(cols.goes, GOES_TO_GROW, GOES_TO_LEAST).child(head("Goes to")))
        .child(div().flex().flex_none().justify_center().w(px(cols.launch)).child(head("Launch")))
        .child(div().flex_none().w(px(MORE_WIDTH)))
        .into_any_element()
}

/// The table: a head, then one row per app in start order.
pub fn rows(panel: &mut Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let p = palette(cx);
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    icons::request(panel, &ws.windows, cx);
    let cols = Cols::measure(window, cx);
    let mut rows = vec![table_head(cols, p)];
    if ws.windows.is_empty() {
        let text = "No apps yet. Add one, or arrange your windows and choose Update from screen";
        rows.push(div().px(px(16.)).py(px(16.)).child(ui::description(text, p)).into_any_element());
    }
    for i in 0..ws.windows.len() {
        rows.push(window_row(panel, (i, cols), ws, &screens, window, cx));
    }
    rows
}

// ---- Zones: the layout a switch gives each screen

/// The built-in layouts a screen's Zones menu lists, after No zones.
const BUILT_IN: [Template; 6] =
    [Template::Columns, Template::Rows, Template::Grid, Template::Priority, Template::MainStack, Template::StackMain];
/// The thumbnail of a screen's layout.
const THUMB: (f32, f32) = (40., 24.);

/// A choice in a screen's Zones menu. No entry keeps the screen as it is.
#[derive(Clone)]
struct ZoneChoice {
    label: String,
    entry: Option<Value>,
}

/// Keep as is, No zones, the built-in layouts, then the library's.
fn zone_choices(panel: &Panel) -> Vec<ZoneChoice> {
    let keep = ZoneChoice { label: "Keep as is".into(), entry: None };
    let none = ZoneChoice { label: "No zones".into(), entry: Some(workspace::no_zones()) };
    let built_in = BUILT_IN.iter().filter_map(|t| {
        let entry = serde_json::to_value(ZoneLayout::of(*t)?).ok()?;
        Some(ZoneChoice { label: t.title().into(), entry: Some(entry) })
    });
    let custom = library::entries(&panel.config.custom_layouts).into_iter().filter_map(|e| {
        let entry = serde_json::to_value(e.layout()).ok()?;
        Some(ZoneChoice { label: e.name, entry: Some(entry) })
    });
    [keep, none].into_iter().chain(built_in).chain(custom).collect()
}

/// A screen's entry, in the words of its menu.
fn zones_label(panel: &Panel, saved: Option<&Value>) -> String {
    match saved.and_then(workspace::screen_layout) {
        None => "Keep as is".into(),
        Some(None) => "No zones".into(),
        Some(Some(layout)) => {
            let linked = layout.custom.as_deref().and_then(|id| library::find(&panel.config.custom_layouts, id));
            linked.map_or_else(|| layout.template.title().into(), |entry| entry.name)
        }
    }
}

/// A small drawing of what an entry gives its screen. Keep as is is a dashed outline.
fn zones_thumb(saved: Option<&Value>, p: Palette) -> Div {
    let frame = div().relative().flex_none().w(px(THUMB.0)).h(px(THUMB.1)).rounded(px(4.));
    match saved.and_then(workspace::screen_layout) {
        None => frame.border_1().border_dashed().border_color(p.hairline_strong),
        Some(None) => frame.bg(p.page),
        Some(Some(layout)) => {
            let area = zones::Rect { x: 0, y: 0, w: THUMB.0 as i32, h: THUMB.1 as i32 };
            frame.children(zones::rects(&layout.tree, area, 2).into_iter().map(|r| {
                let at = |v: i32| px(v as f32);
                div().absolute().left(at(r.x)).top(at(r.y)).w(at(r.w)).h(at(r.h)).rounded(px(2.)).bg(p.segment)
            }))
        }
    }
}

/// Give a screen another entry, or none to keep it as it is, with Undo.
fn set_zones(
    panel: &mut Panel,
    id: &str,
    screen: &str,
    entry: Option<Value>,
    note: String,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let screen = screen.to_string();
    update(panel, id, Some(note), window, cx, move |ws| match entry {
        Some(entry) => {
            ws.zones.insert(screen, entry);
        }
        None => {
            ws.zones.remove(&screen);
        }
    });
}

/// The screen's layout now isn't the one the workspace gives it, so a switch changes it.
fn screen_moved(panel: &Panel, ws: &Workspace, screen: &str) -> bool {
    let Some(saved) = ws.zones.get(screen) else { return false };
    let now = panel.config.desks.get(&desk_sig(panel)).and_then(|d| d.zones.get(screen)).and_then(zones::layout_of);
    !workspace::zones_match(saved, now.as_ref())
}

/// A connected screen's layout isn't the workspace's, which Put windows back changes.
pub fn zones_moved(panel: &Panel, ws: &Workspace) -> bool {
    panel.mons.iter().any(|m| screen_moved(panel, ws, &m.id))
}

/// The menu of a connected screen's layout. `number` is its place on the Zones tabs.
fn zones_menu(panel: &Panel, number: usize, screen: &str, ws: &Workspace, cx: &mut Context<Panel>) -> AnyElement {
    let current = zones_label(panel, ws.zones.get(screen));
    let weak = cx.entity().downgrade();
    let (options, shown, id, screen) = (zone_choices(panel), current.clone(), ws.id.clone(), screen.to_string());
    let menu = move |mut menu: gpui_kit::component::menu::PopupMenu, _: &mut Window, _: &mut Context<_>| {
        for (n, choice) in options.iter().enumerate() {
            let (weak, choice, id, screen) = (weak.clone(), choice.clone(), id.clone(), screen.clone());
            let checked = choice.label == shown;
            let item = PopupMenuItem::new(choice.label.clone()).checked(checked).on_click(move |_, window, cx| {
                let (entry, id) = (choice.entry.clone(), id.clone());
                let note = match (&entry, choice.label.as_str()) {
                    (None, _) => format!("Screen {number} keeps its layout"),
                    (_, "No zones") => format!("Screen {number} gets no zones"),
                    (_, label) => format!("Screen {number} gets {label}"),
                };
                let _ = weak.update(cx, |panel, cx| set_zones(panel, &id, &screen, entry, note, window, cx));
            });
            menu = menu.item(item);
            if (n == 1 || n == BUILT_IN.len() + 1) && n + 1 < options.len() {
                menu = menu.separator();
            }
        }
        menu
    };
    ui::secondary(("ws-zones", number), current.clone(), cx)
        .dropdown_caret(true)
        .accessibility_label(format!("Layout for screen {number}: {current}"))
        .dropdown_menu(menu)
        .into_any_element()
}

/// A screen's model from its device path, for an entry whose screen isn't connected.
fn model_of(screen: &str) -> String {
    screen.split('#').nth(1).filter(|m| !m.is_empty()).unwrap_or("Screen").to_string()
}

/// The Zones section: the layout a switch gives each screen, one row per connected
/// screen, then the entries of screens that aren't connected. Closed until opened.
pub fn zones_section(
    panel: &mut Panel,
    ws: &Workspace,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<AnyElement> {
    let p = palette(cx);
    let open = panel.editing.zones_open.as_deref() == Some(ws.id.as_str());
    let about = match ws.zones.len() {
        0 => "Every screen keeps its layout".to_string(),
        1 => "Sets the layout of 1 screen".to_string(),
        n => format!("Sets the layout of {n} screens"),
    };
    let chevron = if open { assets::IconName::ChevronDown } else { assets::IconName::ChevronRight };
    let focus = window.use_keyed_state("ws-zones-head", cx, |_, cx| cx.focus_handle()).read(cx).clone();
    let (click, key) = (ws.id.clone(), ws.id.clone());
    let toggle = |panel: &mut Panel, id: &str, cx: &mut Context<Panel>| {
        let this = Some(id.to_string());
        panel.editing.zones_open = if panel.editing.zones_open == this { None } else { this };
        cx.notify();
    };
    let head = div()
        .id("ws-zones-head")
        .track_focus(&focus)
        .tab_index(0)
        .role(Role::Button)
        .aria_label(format!("Zones, {about}"))
        .cursor_pointer()
        .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
        .child(row("Zones", Some(about.clone().into()), Icon::new(chevron).size(px(16.)).text_color(p.muted), p))
        .on_click(cx.listener(move |panel, _, _, cx| toggle(panel, &click, cx)))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                toggle(panel, &key, cx);
            }
        }));
    let mut rows = vec![head.into_any_element()];
    if !open {
        return rows;
    }
    let mons: Vec<(String, String)> = panel.mons.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
    for (i, (screen, name)) in mons.iter().enumerate() {
        let saved = ws.zones.get(screen);
        let moved = screen_moved(panel, ws, screen).then(|| {
            let word = div().text_size(px(12.)).text_color(p.secondary).child("Moved");
            div().flex().items_center().gap(px(8.)).child(ui::dot(p.secondary, false)).child(word).into_any_element()
        });
        let menu = zones_menu(panel, i + 1, screen, ws, cx);
        let control = div().flex().items_center().gap(px(8.)).child(zones_thumb(saved, p)).child(menu);
        rows.push(ui::row_with(format!("{}  {name}", i + 1), moved, control));
    }
    let gone = ws.zones.iter().filter(|(screen, _)| !mons.iter().any(|(id, _)| id == *screen));
    for (k, (screen, saved)) in gone.enumerate() {
        let (model, id, screen) = (model_of(screen), ws.id.clone(), screen.clone());
        let note = format!("Removed the layout for {model}");
        let remove = ui::secondary(("ws-zones-remove", k), "Remove", cx)
            .accessibility_label(format!("Remove the layout for {model}"))
            .on_click(cx.listener(move |panel, _, window, cx| {
                set_zones(panel, &id, &screen, None, note.clone(), window, cx);
            }));
        let about = div().text_size(px(13.)).text_color(p.muted).child("Not connected").into_any_element();
        let control =
            div().flex().items_center().gap(px(8.)).child(zones_thumb(Some(saved), p).opacity(0.5)).child(remove);
        rows.push(div().text_color(p.muted).child(ui::row_with(model, Some(about), control)).into_any_element());
    }
    rows
}
