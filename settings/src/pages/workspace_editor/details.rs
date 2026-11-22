//! An app's start order details: how it shows, the delay, what it opens and whether the next waits.

use super::*;
use crate::pages::app_picker;
use crate::theme::tokens::text_style;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::slider::Slider;
use seam::workspace::{Item, Tab};

/// Show as, the delay and the wait, under an app's row.
pub(super) fn details(
    panel: &mut Panel,
    i: usize,
    ws: &Workspace,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Div {
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
    let ends = |text: &'static str| div().text_size(crate::theme::scaled(12.)).text_color(p.text_muted).child(text);
    let slider = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ends("0 s"))
        .child(
            Slider::new(&panel.editing.delay).horizontal().w(px(160.)).accessibility_label("Delay before the next app"),
        )
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
    // What the row opens: its item, or else the "Open with" field, which follows the open row and
    // starts from what is saved for it. Open with is hidden on an item row (D732).
    let opens = match &rule.item {
        Some(item) => {
            // The profiles a Terminal item's tabs are checked against are read once.
            app_picker::read_installed(panel, cx);
            item_row(panel, &ws.id, i, item, p, cx)
        }
        None => {
            let shown_for = Some((ws.id.clone(), i));
            if panel.editing.open_with_for != shown_for {
                panel.editing.open_with_for = shown_for;
                panel.editing.open_with_error = None;
                let saved = rule.open_with.clone().unwrap_or_default();
                panel.editing.open_with.update(cx, |input, cx| input.set_value(saved, window, cx));
            }
            let open_with = div()
                .w(px(320.))
                .font_family(cx.theme().mono_font_family.clone())
                .child(Input::new(&panel.editing.open_with));
            let about = match &panel.editing.open_with_error {
                Some(why) => ui::description(why.clone(), p).text_color(p.negative).into_any_element(),
                None => {
                    ui::description("A full path on this PC for the app to open, or a link, when a switch starts it", p)
                        .into_any_element()
                }
            };
            ui::row_with("Open with", Some(about), open_with)
        }
    };
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
        .child(opens)
        .child(row(
            "Wait for this app's window before opening the next",
            Some("The apps below start once it's open, or after 15 s".into()),
            wait,
            p,
        ))
}

/// What an item row opens, as the dialog would hold it, and a button to change it (D731, D738).
/// Each startup command is shown in the mono font, so nothing a row will run is out of sight.
fn item_row(panel: &Panel, id: &str, i: usize, item: &Item, p: Palette, cx: &mut Context<Panel>) -> ui::Row {
    let mono =
        |text: String| text_style(div(), tokens::LABEL).font_family(cx.theme().mono_font_family.clone()).child(text);
    let known: Vec<String> =
        panel.editing.picker.extras.iter().flat_map(|e| e.profiles.iter().map(|p| p.name.clone())).collect();
    let mut lines = div().flex().flex_col().gap(px(tokens::SPACE_4));
    let title = match item {
        Item::Folder { folder } => {
            lines = lines.child(mono(folder.clone()));
            "Folder"
        }
        Item::Editor { folder } => {
            lines = lines.child(mono(folder.clone()));
            "Editor folder"
        }
        Item::Web { urls } => {
            for url in urls {
                lines = lines.child(mono(url.clone()));
            }
            "Web links"
        }
        Item::Terminal { tabs } => {
            for (n, tab) in tabs.iter().enumerate() {
                lines = lines.child(tab_lines(n, tab, &known, p, &mono));
            }
            "Terminal"
        }
    };
    let (view, id) = (cx.entity(), id.to_string());
    let change = ui::secondary(("change-item", i), "Change what it opens", cx).on_click(move |_, window, cx| {
        let id = id.clone();
        view.update(cx, |panel, cx| change_item(panel, &id, i, window, cx));
    });
    ui::row_with(title, Some(lines.into_any_element()), change)
}

/// One tab of a Terminal item: its profile and folder, its command if it has one, and what's wrong.
fn tab_lines(n: usize, tab: &Tab, known: &[String], p: Palette, mono: &dyn Fn(String) -> Div) -> Div {
    let profile = tab.profile.as_deref().unwrap_or("Default profile");
    let head = match &tab.folder {
        Some(folder) => format!("Tab {}: {profile}, {folder}", n + 1),
        None => format!("Tab {}: {profile}", n + 1),
    };
    let mut lines = div().flex().flex_col().gap(px(tokens::SPACE_2)).child(ui::description(head, p));
    if let Some(command) = &tab.command {
        let label = ui::description("Startup command", p);
        lines =
            lines.child(div().flex().items_center().gap(px(tokens::SPACE_8)).child(label).child(mono(command.clone())));
    }
    // A profile Windows Terminal doesn't have any more opens its default one, with no command.
    let gone = tab.profile.as_ref().filter(|name| !known.is_empty() && !known.contains(name));
    if let Some(name) = gone {
        let why = format!("Profile {name} isn't in Windows Terminal any more");
        lines = lines.child(ui::description(why, p).text_color(p.negative));
    }
    if tab.unreadable {
        let why = "This command came from another PC or a file, so Seam won't run it. Type it again to use it here";
        lines = lines.child(ui::description(why, p).text_color(p.negative));
    }
    lines
}
