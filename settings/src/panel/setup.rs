//! First-run setup: a dialog of three short steps, shown once while `setup_done` is false.

use super::*;
use crate::pages::workspaces::app_title;
use crate::theme::palette;
use crate::theme::tokens::{self, text_style};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::dialog::{Dialog, DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::stepper::{Stepper, StepperItem};
use seam::apps;
use seam::zones::{self, Template, ZoneLayout};
use serde_json::Value;
use std::cell::RefCell;

/// The names of the steps, and the step after them all.
const STEPS: [&str; 3] = ["Line up", "Zones", "Workspace"];
const DONE: usize = STEPS.len();
/// Apps named under the windows count before "and K more".
const APPS_NAMED: usize = 4;

/// Where the dialog is and what it has been told, shared by each draw of it.
struct Setup {
    step: Cell<usize>,
    /// The layout picked for each screen, by index into `mons`. `None` leaves the screen as it is.
    picked: RefCell<Vec<Option<Template>>>,
    /// The title of each window open when the last step was reached. `None` while they're read.
    windows: RefCell<Option<Vec<String>>>,
}

impl Panel {
    /// Offer first-run setup, once, when the first config says it hasn't been done.
    pub(super) fn open_setup(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.setup_done {
            return;
        }
        let setup = Rc::new(Setup {
            step: Cell::new(0),
            picked: RefCell::new(vec![None; self.mons.len()]),
            windows: RefCell::new(None),
        });
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| draw(dialog, &setup, &view, window, cx));
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The first apps by name, then how many more: "Code, Chrome and Slack" or "Code, Chrome, Slack, Notes and 3 more".
fn named(windows: &[String]) -> String {
    let mut apps: Vec<&str> = Vec::new();
    for title in windows {
        if !apps.contains(&title.as_str()) {
            apps.push(title);
        }
    }
    match apps.as_slice() {
        [] => String::new(),
        [only] => (*only).to_string(),
        all if all.len() <= APPS_NAMED => format!("{} and {}", all[..all.len() - 1].join(", "), all[all.len() - 1]),
        all => format!("{} and {} more", all[..APPS_NAMED].join(", "), all.len() - APPS_NAMED),
    }
}

/// The apps open now, read off the UI thread, then the dialog draws again.
fn read_windows(setup: &Rc<Setup>, exclude: Vec<String>, cx: &mut App) {
    *setup.windows.borrow_mut() = None;
    let setup = setup.clone();
    cx.spawn(async move |cx| {
        let open = cx.background_executor().spawn(async move { apps::open_now(&exclude) }).await;
        *setup.windows.borrow_mut() = Some(open.iter().map(|(rule, _)| app_title(rule)).collect());
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

/// Move to a step. The last one reads the windows it will capture.
fn go(setup: &Rc<Setup>, view: &WeakEntity<Panel>, step: usize, window: &mut Window, cx: &mut App) {
    setup.step.set(step);
    if step == 2 {
        let exclude = view
            .upgrade()
            .map(|panel| panel.read(cx).config.zones.exclude.iter().map(|x| x.to_lowercase()).collect())
            .unwrap_or_default();
        read_windows(setup, exclude, cx);
    }
    window.refresh();
}

/// The step's own action, which Enter runs too. Returns whether the dialog closes.
fn act(setup: &Rc<Setup>, view: &WeakEntity<Panel>, window: &mut Window, cx: &mut App) -> bool {
    match setup.step.get() {
        0 => {
            let _ = view.update(cx, |panel, cx| panel.command("lineup.start", json!({}), window, cx));
            go(setup, view, 1, window, cx);
        }
        1 => {
            let picked = setup.picked.borrow().clone();
            let _ = view.update(cx, |panel, cx| use_layouts(panel, &picked, window, cx));
            go(setup, view, 2, window, cx);
        }
        2 => {
            let (next, again) = (setup.clone(), view.clone());
            let _ = view.update(cx, |panel, cx| {
                panel.call("workspace.capture", json!({}), window, cx, move |panel, result, window, cx| match result {
                    Ok(done) => {
                        panel.workspace = done["id"].as_str().map(String::from);
                        panel.reload_config(window, cx);
                        go(&next, &again, DONE, window, cx);
                    }
                    Err(e) => window.push_notification(refusal("Nothing captured", &error_text(&e)), cx),
                })
            });
        }
        _ => return true,
    }
    false
}

/// Give each screen the layout picked for it, where that is a change.
fn use_layouts(panel: &mut Panel, picked: &[Option<Template>], window: &mut Window, cx: &mut Context<Panel>) {
    let sig = panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default();
    for (i, (mon, template)) in panel.mons.clone().iter().zip(picked).enumerate() {
        let Some(template) = *template else { continue };
        if template == current(panel, &sig, &mon.id) {
            continue;
        }
        let layout = ZoneLayout::of(template).and_then(|l| serde_json::to_value(l).ok()).unwrap_or(Value::Null);
        panel.call(
            "zones.pick",
            json!({ "number": i + 1, "layout": layout }),
            window,
            cx,
            |panel, result, window, cx| {
                if let Err(e) = result {
                    window.push_notification(refusal("Layout left as it was", &error_text(&e)), cx);
                }
                panel.reload_config(window, cx);
            },
        );
    }
}

/// The template a screen has now.
fn current(panel: &Panel, sig: &str, screen: &str) -> Template {
    let saved = panel.config.desks.get(sig).and_then(|d| d.zones.get(screen)).and_then(zones::layout_of);
    saved.map_or(Template::None, |l| l.template)
}

fn draw(dialog: Dialog, setup: &Rc<Setup>, view: &WeakEntity<Panel>, _: &mut Window, cx: &mut App) -> Dialog {
    let Some(panel) = view.upgrade() else { return dialog };
    let p = palette(cx);
    let step = setup.step.get();
    let (mons, sig) = {
        let panel = panel.read(cx);
        (panel.mons.clone(), panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default())
    };
    let now: Vec<Template> = mons.iter().map(|m| current(panel.read(cx), &sig, &m.id)).collect();

    let (heading, line, card, primary) = match step {
        0 => (
            "Line up your screens",
            format!(
                "Seam found {}. In Line Up you drag two lines across each border until the stripes run straight over the bezel. It takes about a minute",
                plural(mons.len(), "screen", "screens")
            ),
            Some(ui::card(
                mons.iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let size = format!("{} \u{d7} {}", m.rect.width(), m.rect.height());
                        ui::row(
                            format!("{}  {}", i + 1, m.name),
                            None,
                            text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(size),
                            p,
                        )
                    })
                    .collect(),
                p,
            )),
            "Start Line Up",
        ),
        1 => (
            "Pick zone layouts",
            "Hold Shift while you drag a window to drop it into a zone. Pick a layout for each screen, or leave it as None".to_string(),
            Some(ui::card(
                mons.iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let chosen = setup.picked.borrow().get(i).copied().flatten().unwrap_or(now[i]);
                        // A screen with a layout of its own shows it as Custom, which isn't offered otherwise.
                        let options = Template::ALL
                            .into_iter()
                            .filter(|t| *t != Template::Custom || now[i] == Template::Custom)
                            .map(|t| ui::Choice::new(t.title(), t.title()))
                            .collect();
                        let pick = {
                            let setup = setup.clone();
                            move |key: &str, window: &mut Window, _: &mut App| {
                                let template = Template::ALL.into_iter().find(|t| t.title() == key);
                                if let Some(slot) = setup.picked.borrow_mut().get_mut(i) {
                                    *slot = template;
                                }
                                window.refresh();
                            }
                        };
                        let layout = ui::select(
                            ("setup-layout", i),
                            format!("Layout for screen {}", i + 1),
                            options,
                            chosen.title(),
                            160.,
                            cx,
                            pick,
                        );
                        ui::row(format!("{}  {}", i + 1, m.name), None, layout, p)
                    })
                    .collect(),
                p,
            )),
            "Use these layouts",
        ),
        2 => {
            let windows = setup.windows.borrow().clone();
            let row = match windows {
                None => ui::row("Looking at your windows", None, div(), p),
                Some(open) if open.is_empty() => ui::row("No windows open", None, div(), p),
                Some(open) => ui::row(plural(open.len(), "window open", "windows open"), Some(named(&open).into()), div(), p),
            };
            (
                "Capture a workspace",
                "Open the apps you use together and put them where you want them. Seam saves which app is where, so one shortcut puts them all back".to_string(),
                Some(ui::card(vec![row], p)),
                "Capture current windows",
            )
        }
        _ => (
            "Seam is ready",
            "It waits in the tray. Right-click the icon to switch workspaces or pause it, and press Ctrl F in this window to find any setting".to_string(),
            None,
            "Close setup",
        ),
    };

    let items = STEPS.iter().map(|name| StepperItem::new().child(text_style(div(), tokens::LABEL).child(*name)));
    let body = div()
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_16))
        .child(
            text_style(div(), tokens::LABEL)
                .text_color(p.text_secondary)
                .child("Three short steps. You can do any of them later from Help"),
        )
        .child(Stepper::new("setup-steps").selected_index(step).items(items))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(tokens::SPACE_4))
                .child(text_style(div(), tokens::CARD_TITLE_STRONG).child(heading))
                .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(line)),
        )
        .children(card);

    let primary_button = if step == DONE {
        DialogAction::new().child(ui::primary("setup-primary", primary)).into_any_element()
    } else {
        let (setup, view) = (setup.clone(), view.clone());
        ui::primary("setup-primary", primary)
            .on_click(move |_, window, cx| {
                act(&setup, &view, window, cx);
            })
            .into_any_element()
    };
    let later = (step < DONE).then(|| {
        let (setup, view) = (setup.clone(), view.clone());
        ui::secondary("setup-later", "Do it later", cx)
            .on_click(move |_, window, cx| go(&setup, &view, step + 1, window, cx))
    });
    let skip = (step < DONE).then(|| DialogClose::new().trigger(|b| b.label("Skip setup").ghost()));
    let footer = DialogFooter::new()
        .justify_between()
        .child(div().children(skip))
        .child(div().flex().gap(px(tokens::SPACE_8)).children(later).child(primary_button));

    let (ok_setup, ok_view, done_view) = (setup.clone(), view.clone(), view.clone());
    dialog
        .title(text_style(div(), tokens::CARD_TITLE).child("Set up Seam"))
        .w(px(tokens::SIZE_DIALOG_WIDE))
        .p(px(tokens::SPACE_24))
        .child(body)
        .footer(footer)
        .on_ok(move |_, window, cx| act(&ok_setup, &ok_view, window, cx))
        .on_close(move |_, window, cx| {
            let _ = done_view.update(cx, |panel, cx| panel.patch(json!({ "setup_done": true }), window, cx));
        })
}
