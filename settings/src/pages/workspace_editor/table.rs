//! The apps of a workspace: a group for each screen, and a row for each app in it.

use super::details::details;
use super::groups::{self, AppRow, Group};
use super::screen_zones::model_of;
use super::*;
use crate::pages::workspaces::all;
use crate::theme::tokens::text_style;
use gpui_kit::component::tooltip::Tooltip;
use seam::workspace::Pin;

/// `Pin::bits`: on top, and out of workspace switches.
const TOP: usize = 1;
const STICKY: usize = 2;
/// What a row shows as its place while its windows are in different ones.
const SEVERAL: &str = "Several places";

/// How a row's windows are doing, which sets the dot.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Good,
    Waiting,
    Off,
    Problem,
}

/// A row's windows now, as a kind and words: one word for one window, a count of each for more.
fn status_of(states: &[Option<RowState>]) -> Option<(Kind, String)> {
    let states: Vec<RowState> = states.iter().flatten().copied().collect();
    if states.is_empty() {
        return None;
    }
    let count = |wanted: RowState| states.iter().filter(|s| **s == wanted).count();
    let words = [
        (RowState::InPlace, "in place", "In place"),
        (RowState::Moved, "moved", "Moved"),
        (RowState::Minimized, "minimized", "Minimized"),
        (RowState::Background, "hidden", "Hidden"),
        (RowState::NotRunning, "not running", "Not running"),
        (RowState::CantMove, "can't move", "Can't move: runs as administrator"),
        (RowState::Off, "off", "Off"),
    ];
    let word = match states.as_slice() {
        [one] => words.iter().find(|(s, ..)| s == one).map(|(.., word)| word.to_string())?,
        _ => {
            let parts =
                words.iter().filter(|(s, ..)| count(*s) > 0).map(|(s, word, _)| format!("{} {word}", count(*s)));
            parts.collect::<Vec<_>>().join(", ")
        }
    };
    let kind = match () {
        _ if count(RowState::CantMove) > 0 => Kind::Problem,
        _ if count(RowState::InPlace) == states.len() => Kind::Good,
        _ if count(RowState::NotRunning) + count(RowState::Off) == states.len() => Kind::Off,
        _ => Kind::Waiting,
    };
    Some((kind, word))
}

/// The status cell: a dot and a word, in full on hover when it's cut short.
fn status_cell(i: usize, states: &[Option<RowState>], p: Palette) -> AnyElement {
    let Some((kind, word)) = status_of(states) else { return div().into_any_element() };
    let dot = match kind {
        Kind::Good => ui::dot(p.positive, true),
        Kind::Waiting => ui::dot(p.text_muted, false),
        Kind::Off => ui::dot(p.text_muted, true),
        Kind::Problem => ui::dot(p.negative, false),
    };
    let tip = SharedString::from(word.clone());
    div()
        .id(("status", i))
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .min_w_0()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(dot.flex_none())
        .child(ui::description(word, p).min_w_0().truncate())
        .into_any_element()
}

/// A row being dragged to another place in the start order, or another screen.
#[derive(Clone)]
struct DragRow {
    workspace: String,
    windows: Vec<usize>,
    title: SharedString,
}

impl Render for DragRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        text_style(div(), tokens::LABEL)
            .px(px(tokens::SPACE_12))
            .py(px(tokens::SPACE_4))
            .rounded(px(tokens::RADIUS_8))
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.hairline_strong)
            .text_color(p.text_primary)
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
        .gap(px(tokens::SPACE_8))
        .px(px(tokens::SPACE_16))
        .pb(px(tokens::SPACE_8))
        .child(text_style(div().flex_none(), tokens::CAPTION).text_color(p.text_muted).child("then wait"))
        .child(div().flex_1().h(px(1.)).bg(p.hairline_strong))
}

/// A place in the words of its own screen's group: "Zone 2", "Full screen".
fn short(place: &Place, same_screen: bool) -> String {
    match (&place.target, same_screen) {
        (Target::Zone { zone, .. }, true) => format!("Zone {zone}"),
        (Target::State { state: WinState::Max }, true) => "Full screen".into(),
        _ => place.label.clone(),
    }
}

/// What a rule's place is called under its screen's header: "Zones 3 and 2", "Own size".
fn short_goes(rule: &Rule, screens: &[Screen]) -> String {
    match (&rule.target, screens.iter().any(|s| s.id == rule.monitor)) {
        (_, false) => "Screen not connected".into(),
        (Target::Zone { zone, span_to: None }, _) => format!("Zone {zone}"),
        (Target::Zone { zone, span_to: Some(to) }, _) => format!("Zones {zone} and {to}"),
        (Target::State { state: WinState::Keep }, _) => "Keep where it is".into(),
        (Target::State { .. }, _) => "Full screen".into(),
        (Target::Rect { .. }, _) => "Own size".into(),
    }
}

/// The Select of where the row's windows go. Picking sets every window of the row.
fn goes_select(
    panel: &Panel,
    ws: &Workspace,
    row: &AppRow,
    screens: &[Screen],
    title: &str,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let first = row.windows[0];
    let rule = &ws.windows[first];
    let here: Vec<String> = row.windows.iter().map(|i| goes_to(&ws.windows[*i], screens)).collect();
    let several = here.iter().any(|g| *g != here[0]);
    let current = if several { SEVERAL.to_string() } else { here[0].clone() };
    let options = places(panel);
    let mut choices: Vec<ui::Choice> = options
        .iter()
        .map(|place| ui::Choice::new(place.label.clone(), short(place, place.monitor.as_ref() == Some(&rule.monitor))))
        .collect();
    // What the rule says, when the menu doesn't offer it: its own size, a span, a screen that's gone.
    if !options.iter().any(|place| place.label == current) {
        let label = if several { SEVERAL.to_string() } else { short_goes(rule, screens) };
        choices.insert(0, ui::Choice::new(current.clone(), label));
    }
    let (view, id, windows) = (cx.entity(), ws.id.clone(), row.windows.clone());
    let pick = move |key: &str, window: &mut Window, cx: &mut App| {
        let Some(place) = options.iter().find(|place| place.label == key).cloned() else { return };
        view.update(cx, |panel, cx| {
            let note = |title: &str| format!("{title} goes to {}", place.label);
            update_rows(panel, &id, &windows, note, window, cx, |rule| {
                rule.set_target(place.target.clone());
                if let Some(monitor) = place.monitor.clone() {
                    rule.monitor = monitor;
                }
            });
        });
    };
    ui::select(("goes-to", first), format!("Where {title} goes"), choices, &current, GOES_TO_WIDTH, cx, pick)
}

/// Whether every window of the row has `bit` of the pin.
fn pinned(ws: &Workspace, row: &AppRow, bit: usize) -> bool {
    row.windows.iter().all(|i| ws.windows[*i].pin.is_some_and(|pin| pin.bits() & bit != 0))
}

/// The `…` of a row.
fn more_menu(ws: &Workspace, row: &AppRow, merged: Merge, title: &str, cx: &mut Context<Panel>) -> AnyElement {
    let first = row.windows[0];
    let weak = cx.entity().downgrade();
    let (id, windows, excludable) = (ws.id.clone(), row.windows.clone(), exe_name(&ws.windows[first]).is_some());
    let (starts, top, sticky) =
        (row.windows.iter().all(|i| ws.windows[*i].launch), pinned(ws, row, TOP), pinned(ws, row, STICKY));
    let (off, ws_name) = (row.windows.iter().all(|i| ws.windows[*i].off), ws.name.clone());
    let (here, key) = (ws.clone(), groups::split_key(&ws.id, &ws.windows[first]));
    ui::secondary(("row-more", first), "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {title}"))
        .dropdown_menu(move |menu, _, _| {
            let toggle_pin = |bit: usize, label: &'static str, on: bool| {
                let (weak, id, windows) = (weak.clone(), id.clone(), windows.clone());
                PopupMenuItem::new(label).checked(on).on_click(move |_, window, cx| {
                    let (id, windows) = (id.clone(), windows.clone());
                    let note = |title: &str| match on {
                        true => format!("{title} no longer stays {}", pin_words(bit)),
                        false => format!("{title} stays {}", pin_words(bit)),
                    };
                    let _ = weak.update(cx, |panel, cx| {
                        update_rows(panel, &id, &windows, note, window, cx, |rule| {
                            let bits = rule.pin.map_or(0, Pin::bits);
                            rule.pin = Pin::of_bits(if on { bits & !bit } else { bits | bit });
                        });
                    });
                })
            };
            let (start, details, split) = (weak.clone(), weak.clone(), weak.clone());
            let (start_id, start_windows, details_id, split_key) =
                (id.clone(), windows.clone(), id.clone(), key.clone());
            let mut menu = menu
                .item(PopupMenuItem::new("Start if it's closed").checked(starts).on_click(move |_, window, cx| {
                    let (id, windows) = (start_id.clone(), start_windows.clone());
                    let note = |title: &str| match starts {
                        true => format!("{title} doesn't start with the workspace"),
                        false => format!("{title} starts with the workspace"),
                    };
                    let _ = start.update(cx, |panel, cx| {
                        update_rows(panel, &id, &windows, note, window, cx, |rule| rule.launch = !starts);
                    });
                }))
                .item(PopupMenuItem::new(if off { "Turn on this app" } else { "Turn off this app" }).on_click({
                    let (turn, id, windows, ws_name) = (weak.clone(), id.clone(), windows.clone(), ws_name.clone());
                    move |_, window, cx| {
                        let (id, windows, ws_name) = (id.clone(), windows.clone(), ws_name.clone());
                        let note = |title: &str| match off {
                            true => format!("Turned on {title} in {ws_name}"),
                            false => format!("Turned off {title} in {ws_name}"),
                        };
                        let _ = turn.update(cx, |panel, cx| {
                            update_rows(panel, &id, &windows, note, window, cx, |rule| rule.off = !off);
                        });
                    }
                }))
                .item(
                    PopupMenuItem::new(one_window("Start order details…", merged))
                        .disabled(merged == Merge::Merged)
                        .on_click(move |_, _, cx| {
                            let this = Some((details_id.clone(), first));
                            let _ = details.update(cx, |panel, cx| {
                                panel.editing.expanded = if panel.editing.expanded == this { None } else { this };
                                cx.notify();
                            });
                        }),
                );
            // Several windows can be shown one by one, and shown as one again.
            let label = match merged {
                Merge::Merged => Some("Edit each window"),
                Merge::Split => Some("Merge windows"),
                Merge::Single => None,
            };
            if let Some(label) = label {
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let key = split_key.clone();
                    let _ = split.update(cx, |panel, cx| {
                        if !panel.editing.split.remove(&key) {
                            panel.editing.split.insert(key);
                        }
                        panel.editing.expanded = None;
                        cx.notify();
                    });
                }));
            }
            menu = menu.item(toggle_pin(TOP, "Keep on top", top)).item(toggle_pin(
                STICKY,
                "Keep out of workspace switches",
                sticky,
            ));
            let (now, now_here) = (weak.clone(), here.clone());
            menu = menu.item(
                PopupMenuItem::new(one_window("Use where it is now", merged))
                    .disabled(merged == Merge::Merged)
                    .on_click(move |_, window, cx| {
                        let ws = now_here.clone();
                        let _ = now.update(cx, |panel, cx| where_it_is(panel, &ws, first, window, cx));
                    }),
            );
            if excludable {
                let (never, id, windows) = (weak.clone(), id.clone(), windows.clone());
                menu = menu.item(PopupMenuItem::new("Never include this app").on_click(move |_, window, cx| {
                    let (id, windows) = (id.clone(), windows.clone());
                    let _ = never.update(cx, |panel, cx| never_include(panel, &id, &windows, window, cx));
                }));
            }
            let (removing, id, windows) = (weak.clone(), id.clone(), windows.clone());
            menu.item(PopupMenuItem::new("Remove").on_click(move |_, window, cx| {
                let (id, windows) = (id.clone(), windows.clone());
                let _ = removing.update(cx, |panel, cx| remove_rows(panel, &id, &windows, window, cx));
            }))
        })
        .into_any_element()
}

/// A menu entry that acts on one window, which a merged row can't say: it names the way out.
fn one_window(label: &str, merge: Merge) -> String {
    match merge {
        Merge::Merged => format!("{label} (use Edit each window first)"),
        _ => label.to_string(),
    }
}

fn pin_words(bit: usize) -> &'static str {
    if bit == TOP { "on top" } else { "out of workspace switches" }
}

/// Whether a row stands for one window, several, or several shown one by one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Merge {
    Single,
    Merged,
    Split,
}

/// The handle a row is dragged by.
fn row_grip(ws: &Workspace, row: &AppRow, title: &str, p: Palette) -> Stateful<Div> {
    let drag = DragRow { workspace: ws.id.clone(), windows: row.windows.clone(), title: title.to_string().into() };
    div()
        .id(("row-grip", row.windows[0]))
        .flex()
        .flex_none()
        .w(px(GRIP_WIDTH))
        .cursor_grab()
        .child(Icon::new(assets::IconName::GripVertical).size(px(tokens::SIZE_ICON)).text_color(p.text_muted))
        .tooltip(|window, cx| {
            Tooltip::new("Drag to change the start order or the screen, or Alt+Up and Alt+Down").build(window, cx)
        })
        .on_drag(drag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
}

/// The app's name, and under it where it runs from, how many windows, and whether a switch starts it.
fn name_cell(
    ws: &Workspace,
    row: &AppRow,
    (title, state, problem): (&str, Option<RowState>, Option<&str>),
    p: Palette,
) -> Stateful<Div> {
    let first = row.windows[0];
    let rule = &ws.windows[first];
    // What will run is shown in full on hover.
    let path = if rule.exe.is_empty() { exe_line(rule) } else { rule.exe.clone() };
    let full_path = SharedString::from(format!("{title}: {path}"));
    let mut meta = exe_line(rule);
    if row.windows.len() > 1 {
        meta = format!("{meta}, {} windows", row.windows.len());
    }
    // A closed app that starts with the workspace says so.
    if state == Some(RowState::NotRunning) && rule.launch && ws.launch_missing && !rule.off {
        meta = format!("{meta}, starts when you switch");
    }
    // A merged row is off when all its windows are.
    let off = row.windows.iter().all(|i| ws.windows[*i].off);
    if off {
        meta = format!("{meta}, off");
    }
    // What the last switch couldn't do with it.
    if let Some(problem) = problem {
        meta = format!("{meta}, {problem}");
    }
    div()
        .id(("app-name", first))
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .tooltip(move |window, cx| Tooltip::new(full_path.clone()).build(window, cx))
        .child(
            text_style(div(), tokens::BODY)
                .truncate()
                .when(off, |d| d.text_color(p.text_muted))
                .child(title.to_string()),
        )
        .child(text_style(div(), tokens::CAPTION).text_color(p.text_muted).truncate().child(meta))
}

/// What dropping a row on another row, or a group's header, does: reorder within a screen, or
/// send the app to another.
fn dropped(panel: &mut Panel, drag: &DragRow, target: (&[usize], &str), window: &mut Window, cx: &mut Context<Panel>) {
    let Some(ws) = all(panel).into_iter().find(|w| w.id == drag.workspace) else { return };
    let Some(moving) = drag.windows.first().and_then(|i| ws.windows.get(*i)) else { return };
    let (row, screen) = target;
    if moving.monitor == screen {
        // Going down, it lands after the target row's last window, so never inside a merged row.
        let after = drag.windows.first().zip(row.first()).is_some_and(|(from, to)| from < to);
        let Some(at) = (if after { row.last() } else { row.first() }).copied() else { return };
        move_rows(panel, &drag.workspace, &drag.windows, (at, after), window, cx);
        return;
    }
    let Some(to) = panel.state.as_ref().and_then(|s| s.screens.iter().find(|s| s.id == screen).cloned()) else {
        return;
    };
    send_to_screen(panel, &drag.workspace, &drag.windows, &to, window, cx);
}

/// A group's header: "Screen 1" and its model. Dropping an app on it sends the app there.
fn group_head(group: &Group, screens: &[Screen], ws: &Workspace, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let (name, model) = match screens.iter().find(|s| s.id == group.monitor) {
        Some(s) => (format!("Screen {}", s.number), s.name.clone()),
        None => ("Screen not connected".to_string(), model_of(&group.monitor)),
    };
    let (monitor, dropping) = (group.monitor.clone(), ws.id.clone());
    let at = group.rows.first().map(|r| r.windows.clone()).unwrap_or_default();
    div()
        .id(ElementId::Name(format!("group-{}", group.monitor).into()))
        .flex()
        .items_baseline()
        .gap(px(tokens::SPACE_8))
        .px(px(tokens::SPACE_16))
        .pt(px(tokens::SPACE_12))
        .pb(px(tokens::SPACE_4))
        .child(text_style(div(), tokens::BODY).font_weight(FontWeight::MEDIUM).child(name))
        .child(ui::description(model, p))
        .drag_over::<DragRow>(move |d, _, _, _| d.bg(p.surface_raised))
        .on_drop(cx.listener(move |panel, drag: &DragRow, window, cx| {
            if drag.workspace == dropping {
                dropped(panel, drag, (&at, monitor.as_str()), window, cx);
            }
        }))
        .into_any_element()
}

/// Where the Alt+arrow keys move a row: next to the row before it or after it in its group.
struct Neighbours {
    before: Option<usize>,
    after: Option<usize>,
}

fn app_row(
    panel: &mut Panel,
    ws: &Workspace,
    (group, at): (&Group, usize),
    screens: &[Screen],
    merge: Merge,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let p = palette(cx);
    let row = &group.rows[at];
    let first = row.windows[0];
    let rule = &ws.windows[first];
    let title = app_title(rule);
    let id = ws.id.clone();
    let open = panel.editing.expanded.as_ref().is_some_and(|(e, i)| *e == id && *i == first);
    let focus = row_focus(ws, first, window, cx);
    // A row that is off says so at once, before the tray has been asked.
    let state_of = |i: &usize| if ws.windows[*i].off { Some(RowState::Off) } else { row_state(panel, ws, *i) };
    let states: Vec<Option<RowState>> = row.windows.iter().map(state_of).collect();
    let problem = panel.last_switch.as_ref().filter(|(id, _)| *id == ws.id).and_then(|(_, last)| {
        let apps: Vec<(usize, String)> = row
            .windows
            .iter()
            .map(|i| (*i, seam::workspace::report::app_name(&ws.windows[*i].exe, &ws.windows[*i].title_hint)))
            .collect();
        last.problem_of(&apps)
    });
    let near = Neighbours {
        before: at.checked_sub(1).map(|n| group.rows[n].windows[0]),
        after: group.rows.get(at + 1).and_then(|r| r.windows.last().copied()),
    };

    let main = div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .min_h(px(tokens::SIZE_ROW_2))
        .px(px(tokens::SPACE_16))
        .py(px(tokens::SPACE_8))
        .child(row_grip(ws, row, &title, p))
        .child(app_icon(rule, &panel.icons, p))
        .child(name_cell(
            ws,
            row,
            (&title, states.first().copied().flatten().filter(|_| states.len() == 1), problem),
            p,
        ))
        .child(div().flex_none().w(px(GOES_TO_WIDTH)).child(goes_select(panel, ws, row, screens, &title, cx)))
        .child(div().flex().flex_none().w(px(STATUS_WIDTH)).child(status_cell(first, &states, p)))
        .child(div().flex().flex_none().justify_end().w(px(MORE_WIDTH)).child(more_menu(ws, row, merge, &title, cx)));

    let (keys, dropped_on, monitor) = (id.clone(), id, group.monitor.clone());
    let (moving, target) = (row.windows.clone(), row.windows.clone());
    let last = ws.windows.len() - 1;
    let mut body = div()
        .id(("ws-row", first))
        .track_focus(&focus)
        .tab_index(0)
        .flex()
        .flex_col()
        .when(ui::focused(&focus, window), |d| ui::focus_ring(d, 0., 0., p))
        .on_key_down(cx.listener(move |panel, e: &KeyDownEvent, window, cx| {
            if !e.keystroke.modifiers.alt {
                return;
            }
            let to = match e.keystroke.key.as_str() {
                "up" => near.before.map(|n| (n, false)),
                "down" => near.after.map(|n| (n, true)),
                _ => None,
            };
            if let Some(to) = to {
                cx.stop_propagation();
                move_rows(panel, &keys, &moving, to, window, cx);
            }
        }))
        .drag_over::<DragRow>(move |d, _, _, _| d.bg(p.surface_raised))
        .on_drop(cx.listener(move |panel, drag: &DragRow, window, cx| {
            if drag.workspace == dropped_on {
                dropped(panel, drag, (&target, monitor.as_str()), window, cx);
            }
        }))
        .child(main);
    if open {
        body = body.child(details(panel, first, ws, window, cx));
    }
    let lasts = row.windows.last().copied().unwrap_or(first);
    if rule.wait_for_window && lasts < last {
        body = body.child(stage_line(p));
    }
    body.into_any_element()
}

/// The groups of a workspace's apps, a header for each screen and a row for each app in it.
pub fn rows(panel: &mut Panel, ws: &Workspace, window: &mut Window, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let p = palette(cx);
    let screens = panel.state.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
    icons::request(panel, &ws.windows, cx);
    let split = panel.editing.split.clone();
    let shown = groups::group(&ws.windows, &screens, |rule| split.contains(&groups::split_key(&ws.id, rule)));
    let mut rows = Vec::new();
    if ws.windows.is_empty() {
        let text = "No apps yet. Add one, or arrange your windows and choose Update from screen";
        rows.push(
            div().px(px(tokens::SPACE_16)).py(px(tokens::SPACE_16)).child(ui::description(text, p)).into_any_element(),
        );
    }
    for group in &shown {
        rows.push(group_head(group, &screens, ws, p, cx));
        for at in 0..group.rows.len() {
            let first = &ws.windows[group.rows[at].windows[0]];
            let merge = match (group.rows[at].windows.len(), split.contains(&groups::split_key(&ws.id, first))) {
                (1, true) => Merge::Split,
                (1, false) => Merge::Single,
                _ => Merge::Merged,
            };
            rows.push(app_row(panel, ws, (group, at), &screens, merge, window, cx));
        }
    }
    rows
}
