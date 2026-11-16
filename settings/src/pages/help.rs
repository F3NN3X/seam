//! Help: Getting started, the topics grouped by page, and the text of the one that's open.

use crate::help::{self, Block, CARDS, GETTING_STARTED, TOPICS, Topic};
use crate::pages::workspaces;
use crate::panel::{Page, Panel};
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui::{self, Row, card, section};
use gpui_kit::component::Icon;
use gpui_kit::component::breadcrumb::{Breadcrumb, BreadcrumbItem};
use gpui_kit::*;

/// How wide the text of a topic runs.
const TEXT_WIDTH: f32 = 680.;
/// The narrowest a card of topics gets before the grid puts it on a row of its own.
const CARD_WIDTH: f32 = 320.;

/// The text with its `**` labels in bold.
fn rich(text: &str, p: Palette) -> StyledText {
    let mut plain = String::new();
    let mut bold = Vec::new();
    for (i, part) in text.split("**").enumerate() {
        let start = plain.len();
        plain.push_str(part);
        if i % 2 == 1 {
            let style = HighlightStyle {
                font_weight: Some(FontWeight::SEMIBOLD),
                color: Some(p.text_primary),
                ..Default::default()
            };
            bold.push((start..plain.len(), style));
        }
    }
    StyledText::new(plain).with_highlights(bold)
}

fn paragraph(text: &str, p: Palette) -> Div {
    text_style(div(), tokens::BODY).text_color(p.text_secondary).child(rich(text, p))
}

fn bullets(items: &[&str], p: Palette) -> Div {
    let item = |text: &&str| {
        div()
            .flex()
            .items_start()
            .gap(px(tokens::SPACE_8))
            .child(div().flex_none().mt(px(8.)).size(px(4.)).rounded_full().bg(p.text_muted))
            .child(paragraph(text, p).flex_1().min_w_0())
    };
    div().flex().flex_col().gap(px(tokens::SPACE_4)).children(items.iter().map(item))
}

fn open_button(id: impl Into<ElementId>, page: Page, cx: &mut Context<Panel>) -> impl IntoElement {
    ui::secondary(id, format!("Open {}", page.title()), cx)
        .on_click(cx.listener(move |panel, _, window, cx| panel.show(page, None, window, cx)))
}

/// Whether the desk has what the step makes: borders, zones or a workspace.
fn done(panel: &Panel, page: Page) -> bool {
    panel.config.desks.get(&workspaces::desk_sig(panel)).is_some_and(|desk| match page {
        Page::Displays => !desk.borders.is_empty(),
        // A layout chosen for a screen, not the zones a workspace saved.
        Page::Zones => desk.zones.values().any(|saved| seam::zones::layout_of(saved).is_some()),
        _ => !desk.workspaces.is_empty(),
    })
}

/// The three steps, each with where it stands. Gone once they're all done.
fn getting_started(panel: &Panel, p: Palette, cx: &mut Context<Panel>) -> Option<AnyElement> {
    let steps: Vec<_> =
        GETTING_STARTED.iter().map(|&(title, about, page)| (title, about, page, done(panel, page))).collect();
    if steps.iter().all(|(.., finished)| *finished) {
        return None;
    }
    let rows: Vec<Row> = steps
        .into_iter()
        .enumerate()
        .map(|(i, (title, about, page, finished))| {
            let status = finished.then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(tokens::SPACE_8))
                    .child(ui::dot(p.positive, true))
                    .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child("Done"))
            });
            let control = div().flex().items_center().gap(px(tokens::SPACE_16)).children(status).child(open_button(
                ("help-step", i),
                page,
                cx,
            ));
            ui::row(title, Some(about.into()), control, p)
        })
        .collect();
    Some(section("Getting started", card(rows, p), p))
}

/// A topic as a row of 48 px: its name and a chevron. It opens in place.
fn topic_row(
    at: usize,
    (before, after): (Option<usize>, Option<usize>),
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let title = TOPICS[at].title;
    let view = cx.entity();
    let neighbour = |to: Option<usize>, window: &mut Window, cx: &mut Context<Panel>| {
        to.map(|to| ui::item_focus(("help-topic", to), window, cx))
    };
    let (before, after) = (neighbour(before, window, cx), neighbour(after, window, cx));
    let topic = ui::selectable(("help-topic", at), title, false, window, cx, move |_, cx| {
        view.update(cx, |panel, cx| panel.open_help_topic(at, cx));
    });
    ui::arrow_keys(topic, before, after)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(tokens::SPACE_16))
        .min_h(px(tokens::SIZE_ROW))
        .px(px(tokens::SPACE_16))
        .hover(|d| ui::lit(d, p.surface_raised, p))
        .active(|d| ui::lit(d, p.surface_sidebar, p))
        .child(text_style(div(), tokens::BODY).child(title))
        .child(Icon::new(assets::IconName::ChevronRight).size(px(tokens::SIZE_ICON)).text_color(p.text_muted))
        .into_any_element()
}

/// The rows of the topics titled `titles`, with Up and Down between them.
fn topic_rows(titles: &[&str], p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let ats: Vec<usize> = titles.iter().filter_map(|title| help::position(title)).collect();
    (0..ats.len())
        .map(|k| {
            let around = (k.checked_sub(1).map(|k| ats[k]), ats.get(k + 1).copied());
            topic_row(ats[k], around, p, window, cx)
        })
        .collect()
}

/// The topics of one page, under its name.
fn topic_card(name: &str, titles: &[&str], p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let heading = text_style(div(), tokens::CARD_TITLE_STRONG)
        .px(px(tokens::SPACE_16))
        .pt(px(tokens::SPACE_12))
        .pb(px(tokens::SPACE_8))
        .child(name.to_string());
    let rows = topic_rows(titles, p, window, cx);
    let list = rows.into_iter().map(|row| div().border_t_1().border_color(p.hairline).child(row));
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(CARD_WIDTH))
        .bg(p.surface_card)
        .border_1()
        .border_color(p.hairline)
        .rounded(px(tokens::RADIUS_12))
        .child(heading)
        .children(list)
}

/// "Something not working?", with About as a link.
fn footnote(p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let view = cx.entity();
    let about = ui::selectable("help-about", "About", false, window, cx, move |window, cx| {
        view.update(cx, |panel, cx| panel.show(Page::About, None, window, cx));
    })
    .text_color(p.text_secondary)
    .underline()
    .child("About");
    text_style(div(), tokens::LABEL)
        .flex()
        .flex_wrap()
        .gap_x(px(tokens::SPACE_4))
        .text_color(p.text_muted)
        .child("Something not working? Copy diagnostics or report a problem from")
        .child(about)
}

/// Every topic, grouped by the page it's about.
fn topics(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let header = ui::page_header("Help", "What each part of Seam does, and where to find it", None, p, cx);
    let cards: Vec<Div> = CARDS.iter().map(|(name, titles)| topic_card(name, titles, p, window, cx)).collect();
    let grid = div().flex().flex_wrap().gap(px(tokens::SPACE_16)).children(cards);
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(header)
        .children(getting_started(panel, p, cx))
        .child(section("Topics", grid, p))
        .child(footnote(p, window, cx))
}

/// "Help / Displays": the way back, and the page the topic is under.
fn breadcrumb(topic: &Topic, cx: &mut Context<Panel>) -> Breadcrumb {
    let page = CARDS.iter().find(|(_, titles)| titles.contains(&topic.title)).map_or("Topics", |(name, _)| name);
    let back = cx.listener(|panel, _, _, cx| panel.go_back(cx));
    Breadcrumb::new().child(BreadcrumbItem::new("Help").on_click(back)).child(page)
}

/// "Settings in this topic": each with where it is, and a button that opens its page at its row.
fn settings(topic: &Topic, p: Palette, cx: &mut Context<Panel>) -> Option<AnyElement> {
    if topic.settings.is_empty() {
        return None;
    }
    let rows: Vec<AnyElement> = topic
        .settings
        .iter()
        .enumerate()
        .map(|(i, setting)| {
            let (page, title) = (setting.page, setting.title);
            let go = ui::secondary(("help-setting", i), "Go to setting", cx)
                .on_click(cx.listener(move |panel, _, window, cx| panel.show(page, Some(title), window, cx)));
            ui::row(title, Some(format!("{}, {}", page.title(), setting.section).into()), go, p).into_any_element()
        })
        .collect();
    Some(section("Settings in this topic", card(rows, p), p))
}

/// "Related": the topics to read next.
fn related(topic: &Topic, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Option<AnyElement> {
    let rows = topic_rows(topic.related, p, window, cx);
    (!rows.is_empty()).then(|| section("Related", card(rows, p), p))
}

/// One topic in place of the page: its text, the settings it's about and the topics next to it.
fn topic_view(topic: &Topic, window: &mut Window, cx: &mut Context<Panel>) -> Div {
    let p = palette(cx);
    let mut text = div().flex().flex_col().gap(px(tokens::SPACE_12)).max_w(px(TEXT_WIDTH));
    for block in topic.blocks {
        text = match block {
            Block::Text(t) => text.child(paragraph(t, p)),
            Block::List(items) => text.child(bullets(items, p)),
        };
    }
    let header = ui::page_header(topic.title, topic.about, None, p, cx)
        .children(topic.opens.map(|page| open_button("help-open", page, cx)));
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(breadcrumb(topic, cx))
        .child(header)
        .child(text)
        .children(settings(topic, p, cx))
        .children(related(topic, p, window, cx))
}

pub fn render(panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    match panel.help_topic.and_then(|at| TOPICS.get(at)) {
        Some(topic) => topic_view(topic, window, cx),
        None => topics(panel, window, cx),
    }
}
