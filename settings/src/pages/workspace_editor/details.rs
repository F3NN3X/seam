//! An app's start order details: how it shows, the delay, what it opens and whether the next waits.

use super::*;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::slider::Slider;

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
    let ends = |text: &'static str| div().text_size(px(12.)).text_color(p.text_muted).child(text);
    let slider = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ends("0 s"))
        .child(Slider::new(&panel.editing.delay).horizontal().w(px(160.)))
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
    // The field follows the open row, and starts from what is saved for it.
    let shown_for = Some((ws.id.clone(), i));
    if panel.editing.open_with_for != shown_for {
        panel.editing.open_with_for = shown_for;
        panel.editing.open_with_error = None;
        let saved = rule.open_with.clone().unwrap_or_default();
        panel.editing.open_with.update(cx, |input, cx| input.set_value(saved, window, cx));
    }
    let open_with =
        div().w(px(320.)).font_family(cx.theme().mono_font_family.clone()).child(Input::new(&panel.editing.open_with));
    let open_with_about = match &panel.editing.open_with_error {
        Some(why) => ui::description(why.clone(), p).text_color(p.negative).into_any_element(),
        None => ui::description("A full path on this PC for the app to open, or a link, when a switch starts it", p)
            .into_any_element(),
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
        .child(ui::row_with("Open with", Some(open_with_about), open_with))
        .child(row(
            "Wait for this app's window before opening the next",
            Some("The apps below start once it's open, or after 15 s".into()),
            wait,
            p,
        ))
}
