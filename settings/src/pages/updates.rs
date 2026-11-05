//! The update card and the Updates group on About. The Store copy shows one
//! row instead, because the Store does the updating there.

use crate::panel::Panel;
use crate::theme::{palette, Palette};
use crate::ui::{self, card, row, section, switch};
use crate::update::{self, InstallError, Progress};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress as Bar;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable, WindowExt};
use gpui_kit::*;
use seam::package;
use seam::updates;
use serde_json::json;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The Store listing's product id, for "Get it from the Store". The note
/// stays hidden until the listing is live and this is filled in.
pub const STORE_PRODUCT_ID: Option<&str> = None;
const STORE_UPDATES: &str = "ms-windows-store://downloadsandupdates";
const MAX_NOTES: usize = 6;

/// What the update card is doing now. When it's `Idle` the card shows what the
/// state file says.
pub enum UpdateUi {
    Idle,
    Checking,
    Downloading { version: String, done: u64, total: u64 },
    Installing(String),
    CantInstall(String),
    Failed(String),
}

impl UpdateUi {
    fn busy(&self) -> bool {
        matches!(self, UpdateUi::Checking | UpdateUi::Downloading { .. } | UpdateUi::Installing(_))
    }
}

enum Msg {
    Progress(Progress),
    Done(Result<(), String>),
}

/// Look for an update off the UI thread, and show how far it is.
pub fn check_now(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
    if panel.update_ui.busy() {
        return;
    }
    panel.update_ui = UpdateUi::Checking;
    cx.notify();
    let prerelease = panel.config.updates.prerelease;
    let (tx, rx) = channel();
    cx.background_executor()
        .spawn(async move {
            let result = update::check(prerelease, &mut |progress| {
                let _ = tx.send(Msg::Progress(progress));
            });
            let _ = tx.send(Msg::Done(result));
        })
        .detach();
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn_in(window, async move |this, cx| loop {
        let next = {
            let rx = rx.clone();
            cx.background_executor().spawn(async move { rx.lock().ok()?.recv().ok() }).await
        };
        let Some(msg) = next else { return };
        let finished = matches!(msg, Msg::Done(_));
        let updated = this.update_in(cx, |this, window, cx| {
            match msg {
                Msg::Progress(Progress::Checking) => this.update_ui = UpdateUi::Checking,
                Msg::Progress(Progress::Downloading { version, done, total }) => {
                    this.update_ui = UpdateUi::Downloading { version, done, total }
                }
                Msg::Done(result) => {
                    this.update_file = updates::read();
                    this.update_ui = match result {
                        Ok(()) => UpdateUi::Idle,
                        Err(e) => UpdateUi::Failed(e),
                    };
                    if this.update_file.available.is_none() && matches!(this.update_ui, UpdateUi::Idle) {
                        window.push_notification(Notification::new().message("Seam is up to date"), cx);
                    }
                }
            }
            cx.notify();
        });
        if updated.is_err() || finished {
            return;
        }
    })
    .detach();
}

/// Swap in the downloaded version. The new tray starts, so this window ends.
fn install_now(panel: &mut Panel, version: String, window: &mut Window, cx: &mut Context<Panel>) {
    if panel.update_ui.busy() {
        return;
    }
    panel.update_ui = UpdateUi::Installing(version.clone());
    cx.notify();
    let target = version.clone();
    let answer = cx.background_executor().spawn(async move { update::install(&target) });
    cx.spawn_in(window, async move |this, cx| {
        let result = answer.await;
        let _ = this.update_in(cx, |this, _, cx| match result {
            Ok(()) => cx.quit(),
            Err(InstallError::NotWritable) => {
                this.update_ui = UpdateUi::CantInstall(version);
                cx.notify();
            }
            Err(InstallError::Failed(message)) => {
                this.update_ui = UpdateUi::Failed(message);
                cx.notify();
            }
        });
    })
    .detach();
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.)
}

/// "2026-11-04" as "4 Nov".
fn short_date(iso: &str) -> String {
    let parts: Vec<&str> = iso.split('-').collect();
    let month = parts.get(1).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    let day = parts.get(2).and_then(|d| d.parse::<u32>().ok());
    let names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    match (day, month) {
        (Some(day), Some(month)) => format!("{day} {}", names[month - 1]),
        _ => iso.to_string(),
    }
}

/// "10 minutes ago".
fn ago(unix: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let unit = |n: u64, word: &str| format!("{n} {word}{} ago", if n == 1 { "" } else { "s" });
    match now.saturating_sub(unix) {
        _ if unix == 0 => "never".to_string(),
        s if s < 60 => "just now".to_string(),
        s if s < 3600 => unit(s / 60, "minute"),
        s if s < 86_400 => unit(s / 3600, "hour"),
        s => unit(s / 86_400, "day"),
    }
}

fn shell(p: Palette) -> Div {
    div().flex().flex_col().gap(px(12.)).p(px(16.)).bg(p.card).border_1().border_color(p.hairline).rounded(px(12.))
}

fn title(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).child(text.into())
}

fn busy_card(label: String, p: Palette) -> Div {
    shell(p)
        .flex_row()
        .items_center()
        .gap(px(12.))
        .child(Spinner::new().color(p.muted))
        .child(div().text_size(px(14.)).child(label))
}

fn progress_card(version: &str, done: u64, total: u64, p: Palette) -> Div {
    let percent = if total == 0 { 0. } else { (done as f32 / total as f32) * 100. };
    let range = |text: String| div().text_size(px(12.)).text_color(p.muted).child(text);
    let spread = |left: Div, right: Div| div().flex().justify_between().child(left).child(right);
    shell(p)
        .child(spread(
            title(format!("Downloading Seam {version}")),
            ui::tabular(div().text_size(px(16.)).text_color(p.secondary)).child(format!(
                "{:.1} of {}",
                done as f64 / 1_048_576.,
                megabytes(total)
            )),
        ))
        .child(Bar::new("download").value(percent).color(p.text).h(px(4.)))
        .child(ui::tabular(spread(range("0 MB".into()), range(megabytes(total)))))
}

fn problem_card(message: String, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    shell(p)
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(ui::dot(p.negative, false))
                .child(div().text_size(px(14.)).child(message)),
        )
        .child(ui::secondary("update-retry", "Try again", cx).on_click(cx.listener(|panel, _, window, cx| {
            check_now(panel, window, cx);
        })))
}

fn cant_install_card(version: String, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    shell(p)
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(title("Can't install here"))
                .child(ui::description("Seam is in a folder that needs admin rights", p)),
        )
        .child(ui::secondary("update-download", "Download", cx).on_click(move |_, _, cx| {
            cx.open_url(&update::release_page(&version));
        }))
}

fn ready_card(panel: &Panel, version: String, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let file = &panel.update_file;
    let released = format!("Released {}, {}, signature checked", short_date(&file.date), megabytes(file.size));
    let notes: Vec<&str> = file.notes.lines().filter(|l| !l.trim().is_empty()).take(MAX_NOTES).collect();
    let page = version.clone();
    let install = version.clone();
    let heading = div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(div().mt(px(8.)).child(ui::dot(p.accent, true)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .child(title(format!("Seam {version} is ready to install")))
                .child(ui::description(released, p)),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(ui::secondary("release-notes", "Release notes", cx).on_click(move |_, _, cx| {
                    cx.open_url(&update::release_page(&page));
                }))
                .child(ui::primary("install-update", "Install and restart").on_click(
                    cx.listener(move |panel, _, window, cx| install_now(panel, install.clone(), window, cx)),
                )),
        );
    let mut card = shell(p).child(heading);
    if !notes.is_empty() {
        let bullets = div().text_size(px(14.)).pl(px(20.)).child(TextView::markdown("update-notes", notes.join("\n")));
        card = card.child(bullets);
    }
    card.child(
        div()
            .pl(px(20.))
            .text_size(px(13.))
            .text_color(p.muted)
            .child("Seam closes for a moment while it swaps files, then starts again with your layouts"),
    )
}

/// The card above the Updates group, if there's anything to say.
pub fn update_card(panel: &Panel, cx: &mut Context<Panel>) -> Option<Div> {
    let p = palette(cx);
    Some(match &panel.update_ui {
        UpdateUi::Checking => busy_card("Checking for updates".into(), p),
        UpdateUi::Downloading { version, done, total } => progress_card(version, *done, *total, p),
        UpdateUi::Installing(version) => busy_card(format!("Installing Seam {version}"), p),
        UpdateUi::CantInstall(version) => cant_install_card(version.clone(), cx),
        UpdateUi::Failed(message) => problem_card(message.clone(), cx),
        UpdateUi::Idle => match (&panel.update_file.available, panel.update_file.ready, &panel.update_file.error) {
            (Some(version), true, _) if version != env!("CARGO_PKG_VERSION") => ready_card(panel, version.clone(), cx),
            (_, _, Some(error)) => problem_card(error.clone(), cx),
            _ => return None,
        },
    })
}

/// "Check for updates every day" and its two neighbours, or the Store's one row.
pub fn group(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) -> AnyElement {
    let p = palette(cx);
    if package::is_packaged() {
        let open = ui::secondary("open-store", "Open in Store", cx).on_click(|_, _, cx| cx.open_url(STORE_UPDATES));
        let about = Some("Signed by Microsoft and installed in the background".into());
        return card(vec![row("Updates come from the Microsoft Store", about, open, p)], p).into_any_element();
    }
    let view = cx.entity();
    let setter = |patch: fn(bool) -> serde_json::Value| {
        let view = view.clone();
        move |on: bool, window: &mut Window, cx: &mut App| {
            view.update(cx, |panel, cx| panel.patch(patch(on), window, cx));
        }
    };
    let check = ui::secondary("check-now", "Check now", cx)
        .disabled(panel.update_ui.busy())
        .on_click(cx.listener(|panel, _, window, cx| check_now(panel, window, cx)));
    let daily = switch(
        "update-daily",
        panel.config.updates.auto_check,
        window,
        cx,
        setter(|on| json!({ "updates": { "auto_check": on } })),
    );
    let on_their_own = switch(
        "update-auto-install",
        panel.config.updates.auto_install,
        window,
        cx,
        setter(|on| json!({ "updates": { "auto_install": on } })),
    );
    let prerelease = switch(
        "update-prerelease",
        panel.config.updates.prerelease,
        window,
        cx,
        setter(|on| json!({ "updates": { "prerelease": on } })),
    );
    let checked = format!("Last checked {}", ago(panel.update_file.last_checked));
    let rows = vec![
        row(
            "Check for updates every day",
            Some(checked.into()),
            div().flex().items_center().gap(px(8.)).child(check).child(daily),
            p,
        ),
        row(
            "Install updates on their own",
            Some("Only while you're away from the computer. You get a notification after".into()),
            on_their_own,
            p,
        ),
        row("Include pre-releases", Some("Get new features early. They may have rough edges".into()), prerelease, p),
    ];
    section("Updates", card(rows, p), p).into_any_element()
}

/// Under the About header in the GitHub build, once the Store listing is live.
pub fn unsigned_note(cx: &mut Context<Panel>) -> Option<Div> {
    let id = STORE_PRODUCT_ID.filter(|_| !package::is_packaged())?;
    let p = palette(cx);
    let get = ui::secondary("get-from-store", "Get it from the Store", cx).on_click(move |_, _, cx| {
        cx.open_url(&format!("ms-windows-store://pdp/?productid={id}"));
    });
    Some(ui::note("This build isn't signed, so Windows may warn about it. The Store version is signed", Some(get), p))
}
