//! The "Physical layout" card: its header, the map, the selected border and the legend.

use super::borders::{self, Kind};
use super::physical_map::{map, sample};
use super::*;

/// Each kind of border the map draws, with its name.
const LEGEND: [(Kind, &str); 3] = [
    (Kind::LinedUp, "Lined up"),
    (Kind::Glued, "Glued, the pointer jumps the gap"),
    (Kind::NotOnDesk, "Not on your desk, the pointer stops"),
];

fn legend(p: Palette) -> Div {
    let entries = LEGEND.iter().map(|(kind, name)| {
        div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_8))
            .child(sample(*kind, p))
            .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(*name))
    });
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_x(px(tokens::SPACE_24))
        .gap_y(px(tokens::SPACE_8))
        .child(div().flex().flex_wrap().gap_x(px(tokens::SPACE_24)).gap_y(px(tokens::SPACE_8)).children(entries))
        .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child("Click a border to see it"))
}

/// The title and second line, the choice of view, and "Show on screens".
fn header(panel: &Panel, desk_view: bool, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let caption = if desk_view {
        "Real sizes from each screen's EDID, joined where you lined them up"
    } else {
        "Windows joins screens by pixels, so the pointer jumps where sizes differ"
    };
    let view = cx.entity();
    let views = ui::segmented(
        "layout-view",
        "Layout view",
        &["On your desk", "As Windows sees it"],
        usize::from(!desk_view),
        window,
        cx,
        move |at, _, cx| {
            view.update(cx, |panel, cx| {
                panel.layout_view = if at == 0 { LayoutView::Seam } else { LayoutView::Windows };
                cx.notify();
            });
        },
    );
    let show = ui::secondary("show-layout", "Show on screens", cx).on_click(cx.listener(|panel, _, window, cx| {
        let view = if panel.layout_view == LayoutView::Seam { "seam" } else { "windows" };
        panel.command("layout.show", json!({ "view": view }), window, cx);
    }));
    let one_screen = panel.mons.len() < 2;
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(tokens::SPACE_16))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(tokens::SPACE_2))
                .child(text_style(div(), tokens::CARD_TITLE).child("Physical layout"))
                .child(ui::description(caption, p)),
        )
        .when(!one_screen, |d| {
            d.child(div().flex().flex_none().items_center().gap(px(tokens::SPACE_8)).child(views).child(show))
        })
}

/// The physical layout: the map, then the selected border and the legend on the desk view.
pub(super) fn card_of(
    panel: &Panel,
    state: &State,
    width: f64,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let desk_view = panel.layout_view == LayoutView::Seam;
    let head = header(panel, desk_view, p, window, cx);
    let mut body = div().flex().flex_col().gap(px(tokens::SPACE_12)).p(px(tokens::SPACE_16)).child(head);
    if panel.mons.len() < 2 {
        let text = "Only one screen is connected, so there's nothing to line up";
        return card(vec![body.child(ui::description(text, p)).into_any_element()], p).into_any_element();
    }
    body = body.child(map(panel, state, width, p, cx));
    if desk_view {
        body = body.children(borders::strip(panel, state, p, cx)).child(legend(p));
    }
    card(vec![body.into_any_element()], p).into_any_element()
}
