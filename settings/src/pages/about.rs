//! Version, updates, support links and diagnostics.

use crate::pages::updates::{self, UpdateUi};
use crate::panel::Panel;
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui::{self, card, row, row_with};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use seam::package;
use seam::state::{BorderState, Remap, Zones};
use seam::winver;

const REPO: &str = "https://github.com/F3NN3X/seam";

/// Two screen outlines with the amber seam between them.
fn mark(p: Palette) -> Div {
    let outline = |w: f32, h: f32| div().w(px(w)).h(px(h)).border_1().border_color(p.text_primary).rounded(px(2.));
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .size(px(48.))
        .bg(p.surface_card)
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

/// Put the diagnostics on the clipboard and say so.
pub fn copy_diagnostics(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) {
    cx.write_to_clipboard(ClipboardItem::new_string(diagnostics(panel)));
    window.push_notification(Notification::new().message("Copied diagnostics"), cx);
}

/// How the update check stands, at the right of the header. Nothing while an update is on its way
/// or has failed, as the card under it says so, and nothing for the Store copy, which the Store updates.
fn update_status(panel: &Panel, p: Palette) -> Option<Div> {
    if package::is_packaged() {
        return None;
    }
    let (word, color, filled) = match &panel.update_ui {
        UpdateUi::Checking => ("Checking…", p.text_muted, false),
        UpdateUi::Idle => match (&panel.update_file.available, panel.update_file.ready, &panel.update_file.error) {
            (_, _, Some(_)) => return None,
            (Some(version), true, _) if version != env!("CARGO_PKG_VERSION") => ("Update ready", p.accent, true),
            (Some(version), false, _) if version != env!("CARGO_PKG_VERSION") => ("Update available", p.accent, false),
            _ => ("Up to date", p.positive, true),
        },
        _ => return None,
    };
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(tokens::SPACE_8))
            .child(ui::dot(color, filled))
            .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(word)),
    )
}

/// The Support and Seam sections: diagnostics, the issue form, the source and the licences.
fn links_sections(p: Palette, cx: &mut Context<Panel>) -> [AnyElement; 2] {
    let copy = ui::secondary("copy-diagnostics", "Copy diagnostics", cx)
        .on_click(cx.listener(|panel, _, window, cx| copy_diagnostics(panel, window, cx)));
    let report = ui::secondary("report", "Report a problem", cx).on_click(cx.listener(|panel, _, window, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(diagnostics(panel)));
        cx.open_url(&format!("{REPO}/issues/new"));
        window.push_notification(Notification::new().message("Diagnostics copied. Paste them into the issue"), cx);
    }));
    let source = ui::secondary("source", "Open on GitHub", cx).on_click(|_, _, cx| cx.open_url(REPO));
    let licenses = ui::secondary("licenses", "View licenses", cx)
        .on_click(|_, _, cx| cx.open_url(&format!("{REPO}/blob/main/LICENSE")));
    let repo_name = text_style(div(), tokens::MONO)
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(p.text_secondary)
        .child(REPO.trim_start_matches("https://"));
    let support = vec![
        row(
            "Copy diagnostics",
            Some("Your version, Windows build, screens and saved borders. No window titles or file paths".into()),
            copy,
            p,
        ),
        row("Report a problem", Some("Opens a new issue on GitHub. Paste your diagnostics into it".into()), report, p),
    ];
    let seam = vec![
        row_with("Source code", Some(repo_name.into_any_element()), source),
        row("Licenses", Some("Seam's licence and the open source parts it uses".into()), licenses, p),
    ];
    [ui::section("Support", card(support, p), p), ui::section("Seam", card(seam, p), p)]
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let title = ui::page_title(
        format!("Seam {}", env!("CARGO_PKG_VERSION")),
        format!("Build {} on Windows 11 {}", option_env!("SEAM_BUILD").unwrap_or("dev"), winver::display_version()),
        Some("Updates"),
        p,
        cx,
    );
    let header = div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_16))
        .child(mark(p))
        .child(title)
        .child(div().flex_1())
        .children(update_status(panel, p));
    let mut page = div().flex().flex_col().gap(px(ui::SECTION_GAP)).child(header);
    if let Some(note) = updates::unsigned_note(cx) {
        page = page.child(note);
    }
    if !package::is_packaged()
        && let Some(card) = updates::update_card(panel, cx)
    {
        page = page.child(card);
    }
    page.child(updates::group(panel, window, cx)).children(links_sections(p, cx))
}
