//! The Tiling section: smart tiling on the screens, for the virtual desktop in front.

use super::toggle;
use crate::panel::Panel;
use crate::theme::Palette;
use crate::ui::{self, card, row, section, switch};
use gpui_kit::*;
use seam::tiling::Family;
use serde_json::json;

/// Smart tiling on one screen, for the virtual desktop in front. The tray keeps it for each
/// screen, so the screen is picked first, the same one the Zones page shows.
pub(super) fn tiling_section(
    panel: &mut Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let screens = panel.mons.len().max(1);
    panel.zone_screen = panel.zone_screen.min(screens - 1);
    let number = panel.zone_screen as u32 + 1;
    let now = panel.state.as_ref().and_then(|s| s.tiling.iter().find(|t| t.screen == number).cloned());
    let (on, family) = now.map_or((false, Family::Smart), |t| (t.on, t.family));
    let mut rows = Vec::new();
    if screens > 1 {
        let labels: Vec<SharedString> = (1..=screens).map(|n| format!("Screen {n}").into()).collect();
        let tabs = ui::choice("tiling-screen", "Screen", labels, None, panel.zone_screen, window, cx, {
            let view = view.clone();
            move |i, _, cx| {
                view.update(cx, |panel, cx| {
                    panel.zone_screen = i;
                    cx.notify();
                })
            }
        });
        rows.push(row("Screen", Some("Smart tiling is set for each screen".into()), tabs, p));
    }
    let title = "Arrange new windows automatically";
    let arrange = switch("zone-tiling", title, on, window, cx, {
        let view = view.clone();
        move |on, window, cx| {
            view.update(cx, |panel, cx| {
                panel.command("tiling.set", json!({ "number": number, "on": on }), window, cx);
            })
        }
    });
    let about = if screens > 1 {
        format!("Each new window takes a share of screen {number}, on this desktop")
    } else {
        "Each new window takes a share of the screen it opens on, on this desktop".to_string()
    };
    rows.push(row(title, Some(about.into()), arrange, p));
    if on {
        let titles: Vec<&'static str> = Family::ALL.iter().map(|f| f.title()).collect();
        let at = Family::ALL.iter().position(|f| *f == family).unwrap_or(0);
        let choice = ui::segmented("zone-tiling-family", "How windows are arranged", &titles, at, window, cx, {
            let view = view.clone();
            move |i, window, cx| {
                view.update(cx, |panel, cx| {
                    let family = Family::ALL[i].name();
                    panel.command("tiling.set", json!({ "number": number, "family": family }), window, cx);
                })
            }
        });
        let about = "Smart splits the largest window's space. Scrolling puts windows side by side on a strip you scroll. The others share the screen evenly";
        rows.push(row("How windows are arranged", Some(about.into()), choice, p));
    }
    rows.push(toggle(
        (
            "zone-tiling-quit",
            "Put windows back when Seam quits",
            "Quit from the tray frees every tiled screen first. An update or a restart keeps the arrangement",
        ),
        (panel.config.zones.tiling_free_on_quit, |on| json!({ "zones": { "tiling_free_on_quit": on } })),
        view,
        p,
        window,
        cx,
    ));
    section("Tiling", card(rows, p), p)
}
