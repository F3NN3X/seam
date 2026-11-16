//! The Tiling section: smart tiling on the screens, for the virtual desktop in front.

use super::toggle;
use crate::panel::Panel;
use crate::theme::Palette;
use crate::ui::{self, card, row, section, switch};
use gpui_kit::*;
use seam::tiling::Family;
use serde_json::json;

/// The tray keeps smart tiling for each screen and desktop, so the switch and the family act on
/// every screen, and the state says what each has now.
pub(super) fn tiling_section(
    panel: &Panel,
    view: &Entity<Panel>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let tiling = panel.state.as_ref().map(|s| s.tiling.clone()).unwrap_or_default();
    let numbers: Vec<u32> = panel.state.iter().flat_map(|s| s.screens.iter().map(|s| s.number)).collect();
    let on = tiling.iter().any(|t| t.on);
    let family = tiling.iter().find(|t| t.on).map_or(Family::Smart, |t| t.family);
    let title = "Arrange new windows automatically";
    let arrange = switch("zone-tiling", title, on, window, cx, {
        let (view, numbers) = (view.clone(), numbers.clone());
        move |on, window, cx| {
            view.update(cx, |panel, cx| {
                for number in &numbers {
                    panel.command("tiling.set", json!({ "number": number, "on": on }), window, cx);
                }
            })
        }
    });
    let about = "Each new window takes a share of the screen it opens on, on this desktop";
    let mut rows = vec![row(title, Some(about.into()), arrange, p)];
    if on {
        let titles: Vec<&'static str> = Family::ALL.iter().map(|f| f.title()).collect();
        let at = Family::ALL.iter().position(|f| *f == family).unwrap_or(0);
        let on_screens: Vec<u32> = tiling.iter().filter(|t| t.on).map(|t| t.screen).collect();
        let choice = ui::segmented("zone-tiling-family", "How windows are arranged", &titles, at, window, cx, {
            let view = view.clone();
            move |i, window, cx| {
                view.update(cx, |panel, cx| {
                    let family = Family::ALL[i].name();
                    for number in &on_screens {
                        panel.command("tiling.set", json!({ "number": number, "family": family }), window, cx);
                    }
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
