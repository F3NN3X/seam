//! The window table: rows, drag handle, menus and details.

use super::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::tooltip::Tooltip;

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

/// The handle a row is dragged by.
fn row_grip(ws: &Workspace, i: usize, title: &str, p: Palette) -> Stateful<Div> {
    let drag = DragRow { workspace: ws.id.clone(), from: i, title: title.to_string().into() };
    div()
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
        })
}

/// The app's name and where it runs from. A click opens or closes the launch details.
fn name_cell(ws: &Workspace, i: usize, title: &str, open: bool, cols: Cols, cx: &mut Context<Panel>) -> Stateful<Div> {
    let p = palette(cx);
    let rule = &ws.windows[i];
    // What will run is shown in full on hover. A click opens the launch details.
    let path = if rule.exe.is_empty() { exe_line(rule) } else { rule.exe.clone() };
    let full_path = SharedString::from(format!("{title}: {path}"));
    let chevron = if open { assets::IconName::ChevronDown } else { assets::IconName::ChevronRight };
    let toggle = ws.id.clone();
    div()
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
                .child(div().text_size(px(14.)).truncate().child(title.to_string()))
                .child(div().text_size(px(12.)).text_color(p.muted).truncate().child(exe_line(rule))),
        )
        .child(Icon::new(chevron).flex_none().size(px(16.)).text_color(p.muted))
        .on_click(cx.listener(move |panel, _, _, cx| {
            let this = Some((toggle.clone(), i));
            panel.editing.expanded = if panel.editing.expanded == this { None } else { this };
            cx.notify();
        }))
}

/// Whether the app starts with the workspace.
fn launch_checkbox(ws: &Workspace, i: usize, title: &str, cx: &mut Context<Panel>) -> Checkbox {
    let view = cx.entity();
    let launch_id = ws.id.clone();
    Checkbox::new(("launch", i)).checked(ws.windows[i].launch).accessibility_label(format!("Start {title}")).on_click(
        move |on, window, cx| {
            let (on, id) = (*on, launch_id.clone());
            view.update(cx, |panel, cx| {
                let note = |title: &str| match on {
                    true => format!("{title} starts with the workspace"),
                    false => format!("{title} doesn't start with the workspace"),
                };
                update_row(panel, &id, i, note, window, cx, |rule| rule.launch = on);
            });
        },
    )
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

    let grip = row_grip(ws, i, &title, p);
    let name = name_cell(ws, i, &title, open, cols, cx);
    let launch = launch_checkbox(ws, i, &title, cx);

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
