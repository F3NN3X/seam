//! Version, updates, support links and diagnostics.

use crate::pages::updates;
use crate::panel::Panel;
use crate::theme::{palette, Palette};
use crate::ui;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use seam::package;
use seam::state::{BorderState, Remap, Zones};
use seam::winver;

const REPO: &str = "https://github.com/F3NN3X/seam";

/// Two screen outlines with the amber seam between them.
fn mark(p: Palette) -> Div {
    let outline = |w: f32, h: f32| div().w(px(w)).h(px(h)).border_1().border_color(p.text).rounded(px(2.));
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .size(px(48.))
        .bg(p.card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(12.))
        .child(outline(12., 16.))
        .child(div().flex_none().w(px(2.)).h(px(12.)).bg(p.accent))
        .child(outline(14., 20.))
}

/// No window titles, exe paths, user name or profile path.
fn diagnostics(panel: &Panel) -> String {
    let mut text = format!(
        "Seam {}, build {}\nWindows {} ({})\nInstall: {}\n\nScreens\n",
        env!("CARGO_PKG_VERSION"),
        option_env!("SEAM_BUILD").unwrap_or("dev"),
        winver::display_version(),
        winver::build(),
        seam::install::kind(),
    );
    for (i, m) in panel.mons.iter().enumerate() {
        text += &format!(
            "{}: {}, {}x{}, {:.0} dpi{}\n",
            i + 1,
            m.name,
            m.rect.width(),
            m.rect.height(),
            m.raw_dpi,
            if m.primary { ", primary" } else { "" },
        );
    }
    text += &format!("\nDesks: {}\n", panel.config.desks.len());
    if let Some(state) = &panel.state {
        text += "\nBorders\n";
        for b in &state.borders {
            let word = if b.state == BorderState::LinedUp { "lined up" } else { "not lined up" };
            let link = panel.links.iter().find(|l| (l.a as u32 + 1, l.b as u32 + 1) == (b.a, b.b));
            let map = link.map_or(String::new(), |l| format!(", s={} o={}", l.map.s, l.map.o));
            text += &format!("{} to {}, {:?}, {word}{map}\n", b.a, b.b, b.side);
        }
        let remap = match state.remap {
            Remap::On => "on",
            Remap::Paused => "paused",
            Remap::OffNoBorders => "off, no borders",
        };
        let zones = if state.zones == Zones::On { "on" } else { "off" };
        text += &format!("\nRemap {remap}, align {}, zones {zones}\n", panel.config.align_arrangement);
        for e in &state.hotkey_errors {
            text += &format!("Shortcut not registered: {} {}\n", e.action, e.combo);
        }
    }
    text
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement {
    let p = palette(cx);
    let header = div().flex().items_center().gap(px(16.)).child(mark(p)).child(ui::page_title(
        format!("Seam {}", env!("CARGO_PKG_VERSION")),
        format!("Build {} on Windows 11 {}", option_env!("SEAM_BUILD").unwrap_or("dev"), winver::display_version()),
        p,
    ));
    let copy =
        ui::secondary("copy-diagnostics", "Copy diagnostics", cx).on_click(cx.listener(|panel, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(diagnostics(panel)));
            window.push_notification(Notification::new().message("Copied diagnostics"), cx);
        }));
    let report = ui::secondary("report", "Report a problem", cx).on_click(cx.listener(|panel, _, window, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(diagnostics(panel)));
        cx.open_url(&format!("{REPO}/issues/new"));
        window.push_notification(Notification::new().message("Diagnostics copied. Paste them into the issue"), cx);
    }));
    let source = ui::secondary("source", "Source on GitHub", cx).on_click(|_, _, cx| cx.open_url(REPO));
    let licenses = ui::secondary("licenses", "Licenses", cx)
        .on_click(|_, _, cx| cx.open_url(&format!("{REPO}/blob/main/LICENSE")));
    let mut page = div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header);
    if let Some(note) = updates::unsigned_note(cx) {
        page = page.child(note);
    }
    if !package::is_packaged() {
        if let Some(card) = updates::update_card(panel, cx) {
            page = page.child(card);
        }
    }
    page.child(updates::group(panel, window, cx)).child(ui::section(
        "Support",
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().gap(px(8.)).child(copy).child(report).child(source).child(licenses))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child("Diagnostics holds your version, Windows build, screens and saved borders. No window titles or file paths"),
            ),
        p,
    ))
}
