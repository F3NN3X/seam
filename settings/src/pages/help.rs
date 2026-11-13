//! Help: a list of topics, and the text of the one picked with a button to its page.

use crate::help::{self, Block, TOPICS, Topic};
use crate::panel::{Page, Panel};
use crate::theme::{Palette, palette};
use crate::ui::{self, card};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const LIST_WIDTH: f32 = 200.;

/// The text with its `**` labels in bold.
fn rich(text: &str) -> StyledText {
    let mut plain = String::new();
    let mut bold = Vec::new();
    for (i, part) in text.split("**").enumerate() {
        let start = plain.len();
        plain.push_str(part);
        if i % 2 == 1 {
            let style = HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
            bold.push((start..plain.len(), style));
        }
    }
    StyledText::new(plain).with_highlights(bold)
}

fn paragraph(text: &str) -> Div {
    div().text_size(px(14.)).line_height(px(20.)).child(rich(text))
}

fn bullets(items: &[&str], p: Palette) -> Div {
    let item = |text: &&str| {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().flex_none().mt(px(8.)).size(px(4.)).rounded_full().bg(p.muted))
            .child(paragraph(text).flex_1().min_w_0())
    };
    div().flex().flex_col().gap(px(4.)).children(items.iter().map(item))
}

fn open_button(id: impl Into<ElementId>, page: Page, cx: &mut Context<Panel>) -> impl IntoElement {
    ui::secondary(id, format!("Open {}", page.title()), cx)
        .on_click(cx.listener(move |panel, _, window, cx| panel.show(page, None, window, cx)))
}

fn list_card(i: usize, topic: &Topic, picked: bool, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    div()
        .id(("help-topic", i))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .cursor_pointer()
        .text_size(px(14.))
        .when(picked, |d| d.bg(p.raised).border_color(p.hairline_strong))
        .when(!picked, |d| d.bg(p.card).border_color(p.hairline).hover(|d| d.bg(p.raised)))
        .child(topic.title)
        .on_click(cx.listener(move |panel, _, _, cx| {
            panel.help_topic = i;
            cx.notify();
        }))
        .into_any_element()
}

/// The topic's text, then its steps, then the button to its page.
fn detail(topic: &Topic, p: Palette, cx: &mut Context<Panel>) -> Div {
    let title = div().text_size(px(16.)).font_weight(FontWeight::MEDIUM).child(topic.title);
    let mut text = div().flex().flex_col().gap(px(12.)).p(px(16.)).child(title);
    let mut steps = Vec::new();
    for (i, block) in topic.blocks.iter().enumerate() {
        match block {
            Block::Text(t) => text = text.child(paragraph(t)),
            Block::List(items) => text = text.child(bullets(items, p)),
            Block::Step { title, text: about, page } => {
                let open = open_button(("help-step", i), *page, cx);
                steps.push(ui::row(*title, Some(help::plain(about).into()), open, p));
            }
        }
    }
    let mut rows = vec![text.into_any_element()];
    rows.extend(steps);
    if let Some(page) = topic.opens {
        rows.push(div().flex().px(px(16.)).py(px(12.)).child(open_button("help-open", page, cx)).into_any_element());
    }
    div().flex_1().min_w_0().child(card(rows, p))
}

pub fn render(panel: &mut Panel, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let p = palette(cx);
    let header = ui::page_header("Help", "What each part of Seam does, and where to find it", p);
    let picked = panel.help_topic.min(TOPICS.len() - 1);
    let cards: Vec<AnyElement> =
        TOPICS.iter().enumerate().map(|(i, topic)| list_card(i, topic, i == picked, p, cx)).collect();
    let list = div().flex().flex_col().flex_none().gap(px(8.)).w(px(LIST_WIDTH)).children(cards);
    div()
        .flex()
        .flex_col()
        .gap(px(ui::SECTION_GAP))
        .child(header)
        .child(div().flex().items_start().gap(px(16.)).child(list).child(detail(&TOPICS[picked], p, cx)))
}
