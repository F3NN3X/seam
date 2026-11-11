//! Workspaces: save which apps are where on a desk, and switch between them.

use crate::icons::{self, Icons};
use crate::pages::{app_picker, workspace_editor};
use crate::panel::{error_text, refusal, Panel};
use crate::theme::{palette, Palette};
use crate::ui::{self, card, keycaps, row, segmented, switch};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::state::Screen;
use seam::workspace::{self, Export, OtherWindows, RowState, Rule, ScreenInfo, Target, Workspace};
use seam::zones;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

const LIST_WIDTH: f32 = 200.;

// ---- What is saved. The config keeps workspaces as raw JSON. Creating, deleting and
// importing send the whole list; the editor sends one workspace.

pub fn desk_sig(panel: &Panel) -> String {
    panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default()
}

/// The workspace this desk takes as in use at start when none is remembered.
fn default_id(panel: &Panel) -> Option<String> {
    panel.config.desks.get(&desk_sig(panel)).and_then(|d| d.default_workspace.clone())
}

fn saved(panel: &Panel) -> Vec<Value> {
    panel.config.desks.get(&desk_sig(panel)).map(|d| d.workspaces.clone()).unwrap_or_default()
}

pub fn all(panel: &Panel) -> Vec<Workspace> {
    workspace::list(&saved(panel))
}

/// The workspace on show: the one picked, or else the first.
fn selected(panel: &Panel) -> Option<Workspace> {
    let all = all(panel);
    let picked = panel.workspace.as_ref().and_then(|id| all.iter().find(|w| w.id == *id));
    picked.or(all.first()).cloned()
}

/// The id of the workspace on show, for its live status.
pub fn shown_id(panel: &Panel) -> Option<String> {
    selected(panel).map(|w| w.id)
}

fn save(panel: &mut Panel, list: Vec<Value>, window: &mut Window, cx: &mut Context<Panel>) {
    let sig = desk_sig(panel);
    panel.patch(json!({ "desks": { sig: { "workspaces": list } } }), window, cx);
}

/// Change one workspace's settings, with no Undo: its name, shortcut and switches.
fn edit(
    panel: &mut Panel,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Panel>,
    change: impl FnOnce(&mut Workspace),
) {
    workspace_editor::update(panel, id, None, window, cx, change);
}

/// Set or clear a workspace's shortcut. The tray registers it.
pub fn set_hotkey(panel: &mut Panel, id: &str, combo: Option<String>, window: &mut Window, cx: &mut Context<Panel>) {
    edit(panel, id, window, cx, |ws| ws.hotkey = combo);
}

/// The workspaces on this desk, for the Shortcuts page.
pub fn on_desk(panel: &Panel) -> Vec<Workspace> {
    all(panel)
}

fn start_naming(panel: &mut Panel, cx: &mut Context<Panel>) {
    panel.naming = true;
    panel.focus_name = true;
    cx.notify();
}

/// Enter or leaving the field keeps the name. A name already in use gets " 2".
pub fn finish_naming(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    // Until the field has been filled in and focused it hasn't been used.
    if !panel.naming || panel.focus_name {
        return;
    }
    panel.naming = false;
    let name = panel.name_input.read(cx).value().trim().to_string();
    if let (Some(ws), false) = (selected(panel), name.is_empty()) {
        let others: Vec<String> = all(panel).into_iter().filter(|w| w.id != ws.id).map(|w| w.name).collect();
        let name = workspace::unique_name(&name, &others);
        if name != ws.name {
            edit(panel, &ws.id, window, cx, |ws| ws.name = name);
        }
    }
    cx.notify();
}

// ---- Actions

fn capture(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    panel.call("workspace.capture", json!({}), window, cx, |panel, result, window, cx| match result {
        Ok(done) => {
            panel.workspace = done["id"].as_str().map(String::from);
            start_naming(panel, cx);
            panel.reload_config(window, cx);
        }
        Err(e) => window.push_notification(refusal("Nothing captured", &error_text(&e)), cx),
    });
}

fn update_from_screen(panel: &Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    let name = ws.name.clone();
    panel.call(
        "workspace.capture",
        json!({ "into": ws.id }),
        window,
        cx,
        move |panel, result, window, cx| match result {
            Ok(_) => {
                let note = Notification::new().message(format!("Updated {name} from the screen"));
                window.push_notification(note, cx);
                panel.reload_config(window, cx);
            }
            Err(e) => window.push_notification(refusal("Workspace left as it was", &error_text(&e)), cx),
        },
    );
}

fn create_empty(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let mut list = saved(panel);
    let existing = workspace::list(&list);
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let created = Workspace { id: workspace::new_id(&ids), name: workspace::next_name(&names), ..Workspace::default() };
    panel.workspace = Some(created.id.clone());
    list.push(workspace::to_value(&created));
    save(panel, list, window, cx);
    start_naming(panel, cx);
}

/// The workspace taken as in use at start, or none. The tray saves it.
fn set_default(panel: &mut Panel, id: Option<String>, window: &mut Window, cx: &mut Context<Panel>) {
    panel.call("workspace.set_default", json!({ "id": id }), window, cx, |panel, result, window, cx| {
        if let Err(e) = result {
            window.push_notification(refusal("Default left as it was", &error_text(&e)), cx);
        }
        panel.reload_config(window, cx);
    });
}

fn delete(panel: &mut Panel, id: &str, window: &mut Window, cx: &mut Context<Panel>) {
    let before = saved(panel);
    let name = workspace::list(&before).into_iter().find(|w| w.id == id).map(|w| w.name).unwrap_or_default();
    let rest = before.iter().filter(|v| workspace::parse(v).is_none_or(|w| w.id != id)).cloned().collect();
    panel.workspace = None;
    save(panel, rest, window, cx);
    panel.offer_undo(
        format!("Deleted {name}"),
        move |panel, window, cx| save(panel, before.clone(), window, cx),
        window,
        cx,
    );
}

/// The connected screens as an export file describes them.
fn screen_infos(panel: &Panel) -> BTreeMap<String, ScreenInfo> {
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    let info = |s: &Screen| {
        let size = panel.mons.iter().find(|m| m.id == s.id).map(|m| (m.rect.width(), m.rect.height()));
        let (width, height) = size.unwrap_or_default();
        (s.id.clone(), ScreenInfo { number: s.number, name: s.name.clone(), width, height })
    };
    screens.iter().map(info).collect()
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn export(panel: &Panel, ws: Workspace, window: &mut Window, cx: &mut Context<Panel>) {
    let used: BTreeMap<String, ScreenInfo> =
        screen_infos(panel).into_iter().filter(|(id, _)| ws.windows.iter().any(|r| r.monitor == *id)).collect();
    let name: String = ws.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '-' }).collect();
    let file = Export { seam_workspace: Export::VERSION, workspace: ws, screens: used };
    let chosen = cx.prompt_for_new_path(&home(), Some(&format!("{name}.seam-workspace.json")));
    cx.spawn_in(window, async move |_, cx| {
        let Ok(Ok(Some(path))) = chosen.await else { return };
        let written = serde_json::to_string_pretty(&file).map_err(std::io::Error::other);
        let written = written.and_then(|text| std::fs::write(&path, text));
        let _ = cx.update(|window, cx| match written {
            Ok(()) => window.push_notification(Notification::new().message("Exported the workspace"), cx),
            Err(e) => window.push_notification(refusal("Workspace not exported", &e.to_string()), cx),
        });
    })
    .detach();
}

fn import(window: &mut Window, cx: &mut Context<Panel>) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Import".into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        let Some(path) = paths.first() else { return };
        let read = std::fs::read(path).map_err(|e| e.to_string());
        let parsed = read.and_then(|bytes| serde_json::from_slice::<Export>(&bytes).map_err(|e| e.to_string()));
        let _ = this.update_in(cx, move |panel, window, cx| match parsed {
            Ok(file) if file.seam_workspace == Export::VERSION => add_imported(panel, file, window, cx),
            Ok(_) => window.push_notification(refusal("That file can't be imported", "It's from a newer Seam"), cx),
            Err(e) => window.push_notification(refusal("That file can't be imported", &e), cx),
        });
    })
    .detach();
}

/// Put an imported workspace on this desk, with its windows on the screens that match.
fn add_imported(panel: &mut Panel, file: Export, window: &mut Window, cx: &mut Context<Panel>) {
    let mut list = saved(panel);
    let existing = workspace::list(&list);
    let names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    let ids: Vec<String> = existing.iter().map(|w| w.id.clone()).collect();
    let imported = workspace::import(file, &screen_infos(panel), &names, &ids);
    let note = Notification::new().message(format!("Imported {}", imported.name));
    panel.workspace = Some(imported.id.clone());
    list.push(workspace::to_value(&imported));
    save(panel, list, window, cx);
    window.push_notification(note, cx);
}

// ---- What is drawn

/// "Windows Terminal" from `WindowsTerminal.exe`, "Firefox" from `firefox.exe`.
pub fn app_title(rule: &Rule) -> String {
    let stem = rule.exe.rsplit('\\').next().unwrap_or(&rule.exe);
    let stem = stem.strip_suffix(".exe").or_else(|| stem.strip_suffix(".EXE")).unwrap_or(stem);
    let mut title = String::new();
    let mut before = ' ';
    for c in stem.chars() {
        if c.is_uppercase() && before.is_lowercase() {
            title.push(' ');
        }
        title.push(if title.is_empty() { c.to_ascii_uppercase() } else { c });
        before = c;
    }
    if title.is_empty() {
        rule.title_hint.clone()
    } else {
        title
    }
}

/// The line under the name: which layout the windows use, or how many there are.
fn summary(panel: &Panel, ws: &Workspace) -> String {
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    let mut used: Vec<&Screen> = Vec::new();
    for rule in ws.windows.iter().filter(|r| matches!(r.target, Target::Zone { .. })) {
        if let Some(s) = screens.iter().find(|s| s.id == rule.monitor) {
            if !used.iter().any(|u| u.id == s.id) {
                used.push(s);
            }
        }
    }
    let sig = desk_sig(panel);
    let template = |s: &Screen| {
        let saved = panel.config.desks.get(&sig).and_then(|d| d.zones.get(&s.id));
        saved.and_then(zones::layout_of).map(|l| l.template.title())
    };
    match used.as_slice() {
        [only] => match template(only) {
            Some(title) => format!("{title} layout on screen {}", only.number),
            None => format!("Zones on screen {}", only.number),
        },
        [] if ws.windows.is_empty() => "No windows yet".into(),
        [] => format!("{} window{}", ws.windows.len(), if ws.windows.len() == 1 { "" } else { "s" }),
        many => {
            let numbers: Vec<String> = many.iter().map(|s| s.number.to_string()).collect();
            format!("Zones on screens {}", numbers.join(", "))
        }
    }
}

/// How many icons the list card shows before "+n".
/// As many as fit in a 200 px card with the "+n" after them.
const ICONS_SHOWN: usize = 6;

/// The workspace's apps as a row of icons in rule order, each app once.
fn icon_strip(i: usize, ws: &Workspace, icons: &Icons, p: Palette) -> Div {
    let mut seen: Vec<String> = Vec::new();
    let mut apps: Vec<&Rule> = Vec::new();
    for rule in &ws.windows {
        let key = icons::identity(rule);
        if !seen.contains(&key) {
            seen.push(key);
            apps.push(rule);
        }
    }
    let more = apps.len().saturating_sub(ICONS_SHOWN);
    let shown = apps.iter().take(ICONS_SHOWN).enumerate().map(|(n, rule)| {
        let slot = div().id(("app-icon", i * 100 + n)).flex_none().size(px(16.));
        let name = SharedString::from(app_title(rule));
        let slot = match icons.get(rule) {
            Some(image) => slot.child(img(image).size(px(16.))),
            None => slot.child(div().size(px(16.)).rounded(px(4.)).bg(p.muted)),
        };
        slot.tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
    });
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .children(shown)
        .when(apps.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(p.muted).child("No windows yet")))
        .when(more > 0, |d| {
            d.child(
                div().flex_none().whitespace_nowrap().text_size(px(12.)).text_color(p.muted).child(format!("+{more}")),
            )
        })
}

/// What a workspace card says about itself besides its name.
struct Marks {
    picked: bool,
    /// Some while it's the workspace in use, saying whether its windows drifted.
    active: Option<bool>,
    default: bool,
}

fn list_card(i: usize, ws: &Workspace, icons: &Icons, marks: Marks, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let id = ws.id.clone();
    let picked = marks.picked;
    let default = marks.default.then(|| div().text_size(px(12.)).text_color(p.secondary).child("Default"));
    // The marks get a line of their own, so the name keeps the card's full width.
    let tags = (marks.default || marks.active.is_some()).then(|| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(default)
            .children(marks.active.map(|drifted| active_status(drifted, p)))
    });
    let mut label = vec![ws.name.clone()];
    if marks.default {
        label.push("default".into());
    }
    if let Some(drifted) = marks.active {
        label.push(if drifted { "active, moved" } else { "active" }.into());
    }
    let name = div().min_w_0().text_size(px(14.)).font_weight(FontWeight::MEDIUM).truncate().child(ws.name.clone());
    let detail = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(icon_strip(i, ws, icons, p))
        .children(ws.hotkey.as_ref().map(|combo| keycaps(combo, false, p)));
    div()
        .id(("workspace", i))
        .role(Role::ListItem)
        .aria_label(label.join(", "))
        .aria_selected(picked)
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(12.))
        .rounded(px(12.))
        .border_1()
        .cursor_pointer()
        .when(picked, |d| d.bg(p.raised).border_color(p.hairline_strong))
        .when(!picked, |d| d.bg(p.card).border_color(p.hairline).hover(|d| d.bg(p.raised)))
        .child(name)
        .children(tags)
        .child(detail)
        .on_click(cx.listener(move |panel, _, _, cx| {
            panel.workspace = Some(id.clone());
            panel.naming = false;
            panel.editing.expanded = None;
            cx.notify();
        }))
        .into_any_element()
}

/// The status on the card of the workspace in use: a dot and a word, as the editor's rows have.
fn active_status(drifted: bool, p: Palette) -> AnyElement {
    let word = if drifted { "Active, moved" } else { "Active" };
    let text = div().text_size(px(12.)).text_color(p.secondary).child(word);
    div().flex().flex_none().items_center().gap(px(8.)).child(ui::dot(p.positive, true)).child(text).into_any_element()
}

/// "Put windows back": Snap back for the workspace in use, a switch for any other.
fn snap_back_button(panel: &Panel, ws: &Workspace, cx: &mut Context<Panel>) -> Button {
    let active = panel.state.as_ref().and_then(|s| s.workspace.as_ref()).is_some_and(|w| w.id == ws.id);
    let id = ws.id.clone();
    ui::primary("snap-back", "Put windows back").on_click(cx.listener(move |panel, _, window, cx| {
        if active {
            panel.command("workspace.snap_back", json!({}), window, cx);
        } else {
            panel.command("workspace.switch", json!({ "id": id }), window, cx);
        }
    }))
}

/// The one switch for every workspace: put apps in place when they open (§11.1).
fn auto_apply_row(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) -> AnyElement {
    let p = palette(cx);
    let view = cx.entity();
    let on = panel.config.auto_apply;
    let toggle = switch("auto-apply", "Put apps in place when they open", on, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| {
            panel.config.auto_apply = on;
            cx.notify();
            panel.call("autoapply.set", json!({ "on": on }), window, cx, |panel, result, window, cx| {
                if let Err(e) = result {
                    window.push_notification(refusal("Setting left as it was", &error_text(&e)), cx);
                }
                panel.reload_config(window, cx);
            });
        });
    });
    let about = "A window that opens goes where the workspace in use puts it, once";
    row("Put apps in place when they open", Some(about.into()), toggle, p)
}

fn detail(panel: &mut Panel, ws: Workspace, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let summary = summary(panel, &ws);

    // A new or renamed workspace opens its name for editing.
    if panel.focus_name {
        let name = ws.name.clone();
        panel.name_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        panel.focus_name = false;
    }
    let title = if panel.naming {
        div().w(px(220.)).child(Input::new(&panel.name_input)).into_any_element()
    } else {
        div().text_size(px(16.)).font_weight(FontWeight::MEDIUM).truncate().child(ws.name.clone()).into_any_element()
    };
    let update = ui::secondary("update-workspace", "Update from screen", cx).on_click({
        let ws = ws.clone();
        cx.listener(move |panel, _, window, cx| update_from_screen(panel, &ws, window, cx))
    });
    let id = ws.id.clone();
    let switch_now =
        ui::secondary("switch-workspace", "Switch now", cx).on_click(cx.listener(move |panel, _, window, cx| {
            panel.command("workspace.switch", json!({ "id": id }), window, cx);
        }));
    let view = cx.entity().downgrade();
    let more_ws = ws.clone();
    let is_default = default_id(panel).as_deref() == Some(ws.id.as_str());
    let more = ui::secondary("more-workspace", "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {}", ws.name))
        .dropdown_menu(move |menu, _, _| {
            let (rename, exporting, importing, deleting) = (view.clone(), view.clone(), view.clone(), view.clone());
            let (to_export, id) = (more_ws.clone(), more_ws.id.clone());
            let (alongside, alongside_id) = (view.clone(), more_ws.id.clone());
            let (defaulting, default_to) = (view.clone(), (!is_default).then(|| more_ws.id.clone()));
            let default_label = if is_default { "Stop being the default" } else { "Set as default" };
            menu.item(PopupMenuItem::new("Open alongside").on_click(move |_, window, cx| {
                let id = alongside_id.clone();
                let _ = alongside.update(cx, |panel, cx| {
                    panel.command("workspace.switch", json!({ "id": id, "alongside": true }), window, cx);
                });
            }))
            .item(PopupMenuItem::new("Rename").on_click(move |_, _, cx| {
                let _ = rename.update(cx, start_naming);
            }))
            .item(PopupMenuItem::new(default_label).on_click(move |_, window, cx| {
                let id = default_to.clone();
                let _ = defaulting.update(cx, |panel, cx| set_default(panel, id, window, cx));
            }))
            .item(PopupMenuItem::new("Export…").on_click(move |_, window, cx| {
                let to_export = to_export.clone();
                let _ = exporting.update(cx, |panel, cx| export(panel, to_export, window, cx));
            }))
            .item(PopupMenuItem::new("Import…").on_click(move |_, window, cx| {
                let _ = importing.update(cx, |_, cx| import(window, cx));
            }))
            .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                let id = id.clone();
                let _ = deleting.update(cx, |panel, cx| delete(panel, &id, window, cx));
            }))
        });
    let header = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(12.))
        .child(div().flex().flex_col().gap(px(2.)).flex_1().min_w_0().child(title).child(ui::description(summary, p)))
        .child(div().flex_none().child(update))
        .child(div().flex_none().child(switch_now))
        .child(div().flex_none().child(more))
        .into_any_element();

    let mut rows = vec![header];
    rows.extend(workspace_editor::rows(panel, &ws, window, cx));
    let mut states = (0..ws.windows.len()).filter_map(|i| workspace_editor::row_state(panel, &ws, i));
    if states.any(|s| matches!(s, RowState::Moved | RowState::Minimized)) {
        rows.push(div().flex().px(px(16.)).py(px(12.)).child(snap_back_button(panel, &ws, cx)).into_any_element());
    }
    let id = ws.id.clone();
    let add_app: app_picker::Add =
        Rc::new(move |panel, rule, window, cx| workspace_editor::add_app(panel, &id, rule, window, cx));
    let add = app_picker::button(panel, &ws.windows, "In this workspace", add_app, cx);
    rows.push(div().flex().px(px(16.)).py(px(12.)).child(add).into_any_element());

    let view = cx.entity();
    let id = ws.id.clone();
    let start_missing = switch("start-missing", "Start missing apps", ws.launch_missing, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            let id = id.clone();
            view.update(cx, |panel, cx| edit(panel, &id, window, cx, |ws| ws.launch_missing = on));
        }
    });
    let id = ws.id.clone();
    let selected = match ws.other_windows {
        OtherWindows::Leave => 0,
        OtherWindows::Minimize => 1,
        OtherWindows::Close => 2,
    };
    let others = segmented(
        "other-windows",
        "Other windows",
        &["Leave", "Minimize", "Close"],
        selected,
        window,
        cx,
        move |i, window, cx| {
            let id = id.clone();
            let mode = [OtherWindows::Leave, OtherWindows::Minimize, OtherWindows::Close][i.min(2)];
            view.update(cx, |panel, cx| edit(panel, &id, window, cx, |ws| ws.other_windows = mode));
        },
    );
    rows.push(row("Start missing apps", Some("Ticked apps open if they're closed".into()), start_missing, p));
    rows.push(row(
        "Other windows",
        Some("When you switch. Close asks each app first, so it can save".into()),
        others,
        p,
    ));
    div().flex_1().min_w_0().child(card(rows, p))
}

fn empty_state(panel: &Panel, p: Palette, cx: &mut Context<Panel>) -> Div {
    let capture = ui::primary("capture-empty", "Capture current windows")
        .on_click(cx.listener(|panel, _, window, cx| capture(panel, window, cx)));
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(12.))
        .p(px(32.))
        .border_1()
        .border_dashed()
        .border_color(p.hairline_strong)
        .rounded(px(12.))
        .child(div().text_size(px(16.)).child(format!("No workspaces for {} yet", panel.desk_name())))
        .child(ui::description("Arrange your windows the way you like them, then capture them", p))
        .child(capture)
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let list = all(panel);
    let header = ui::page_header("Workspaces", "Save which apps go in which zone, then switch with one shortcut", p);
    if list.is_empty() {
        return div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header).child(empty_state(panel, p, cx));
    }
    let capture = ui::primary("capture", "Capture current windows")
        .on_click(cx.listener(|panel, _, window, cx| capture(panel, window, cx)));
    let rules: Vec<Rule> = list.iter().flat_map(|ws| ws.windows.iter().cloned()).collect();
    icons::request(panel, &rules, cx);
    let active = panel.state.as_ref().and_then(|s| s.workspace.as_ref()).map(|w| (w.id.clone(), w.drifted));
    let default = default_id(panel);
    let current = selected(panel);
    // A workspace on show for the first time asks how its apps are doing.
    let known = panel.status.as_ref().map(|(id, _)| id.as_str());
    if current.as_ref().is_some_and(|c| known != Some(c.id.as_str())) && panel.asking_status.is_none() {
        panel.ask_status(cx);
    }
    let cards: Vec<AnyElement> = list
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let picked = current.as_ref().is_some_and(|c| c.id == ws.id);
            let in_use = active.as_ref().filter(|(id, _)| *id == ws.id).map(|(_, drifted)| *drifted);
            let marks = Marks { picked, active: in_use, default: default.as_deref() == Some(ws.id.as_str()) };
            list_card(i, ws, &panel.icons, marks, p, cx)
        })
        .collect();
    // The dashed frame has nothing else on it, so the hover fill sits on the button inside.
    let button = div()
        .id("new-workspace")
        .flex()
        .items_center()
        .justify_center()
        .size_full()
        .rounded(px(11.))
        .text_size(px(14.))
        .text_color(p.secondary)
        .cursor_pointer()
        .hover(|d| d.bg(p.card))
        .child("New empty workspace")
        .on_click(cx.listener(|panel, _, window, cx| create_empty(panel, window, cx)));
    let new_empty =
        div().h(px(48.)).border_1().border_dashed().border_color(p.hairline_strong).rounded(px(12.)).child(button);
    let sidebar = div().flex().flex_col().flex_none().gap(px(8.)).w(px(LIST_WIDTH)).children(cards).child(new_empty);
    let body = div().flex().items_start().gap(px(16.)).child(sidebar);
    let body = match current {
        Some(ws) => body.child(detail(panel, ws, window, cx)),
        None => body,
    };
    let auto_apply = card(vec![auto_apply_row(panel, window, cx)], p);
    div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header.child(capture)).child(body).child(auto_apply)
}
