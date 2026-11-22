//! The desk menu under the page title: the desks Seam knows, rename, and forget the old ones.

use super::*;

/// How long a desk can go unseen before "Forget desks not seen in 30 days" takes it.
const STALE_DAYS: u64 = 30;

/// A short local date in the user's format, such as "9 Nov".
pub(super) fn short_date(unix: u64) -> Option<String> {
    winutil::local_date(unix).map(seam::locale::month_day_local)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn count(screens: usize) -> String {
    format!("{screens} screen{}", if screens == 1 { "" } else { "s" })
}

fn rename(sig: String, current: String, window: &mut Window, cx: &mut Context<Panel>) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Desk name").default_value(current));
    let view = cx.entity().downgrade();
    let first = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        let (name, view, sig) = (input.clone(), view.clone(), sig.clone());
        dialog
            .title("Rename desk")
            .child(Input::new(&input))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.label("Cancel")))
                    .child(DialogAction::new().child(ui::primary("rename-ok", "Rename"))),
            )
            .on_ok(move |_, window, cx| {
                let text = name.read(cx).value();
                if text.trim().is_empty() {
                    return false;
                }
                let patch = json!({ "desks": { sig.clone(): { "name": text.trim() } } });
                let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
                true
            })
    });
}

/// The desks not seen for `STALE_DAYS`, other than the one in use. A desk the tray never saw
/// connected has no date, so it stays: with the settings in a synced folder that is every desk of
/// another PC, since `last_seen` stays on each PC. Without the desk in use there's nothing safe to forget.
fn stale(panel: &Panel) -> Vec<(String, Desk)> {
    let Some(active) = panel.state.as_ref().map(|s| s.desk.sig.as_str()) else { return vec![] };
    stale_desks(&panel.config.desks, active, now())
}

fn stale_desks(desks: &std::collections::BTreeMap<String, Desk>, active: &str, now: u64) -> Vec<(String, Desk)> {
    let before = now.saturating_sub(STALE_DAYS * 24 * 60 * 60);
    let old = |d: &Desk| d.last_seen != 0 && d.last_seen < before;
    desks.iter().filter(|(sig, d)| sig.as_str() != active && old(d)).map(|(s, d)| (s.clone(), d.clone())).collect()
}

/// "Home", "Home and Office" or "Home, Office and Den".
fn named(desks: &[(String, Desk)]) -> String {
    let names: Vec<&str> =
        desks.iter().map(|(_, d)| if d.name.is_empty() { "Unnamed desk" } else { d.name.as_str() }).collect();
    match names.as_slice() {
        [] => String::new(),
        [only] => (*only).to_string(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

/// Ask first, then forget them. Undo puts the desks back exactly as they were.
fn forget_old(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let old = stale(panel);
    if old.is_empty() {
        window.push_notification(Notification::new().message("No desk has gone unseen for 30 days"), cx);
        return;
    }
    let view = cx.entity().downgrade();
    let names = named(&old);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, old) = (view.clone(), old.clone());
        alert
            .title(format!("Forget {names}?"))
            .description("Seam stops recognising them, and their borders and zones go with them")
            .ok_text("Forget desks")
            .cancel_text("Cancel")
            .on_ok(move |_, window, cx| {
                let old = old.clone();
                let _ = view.update(cx, |panel, cx| {
                    let gone: serde_json::Map<String, serde_json::Value> =
                        old.iter().map(|(sig, _)| (sig.clone(), serde_json::Value::Null)).collect();
                    panel.patch(json!({ "desks": gone }), window, cx);
                    let message = format!("Forgot {}", named(&old));
                    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                        // A desk that reconnected since keeps its new data.
                        let back: serde_json::Map<String, serde_json::Value> = old
                            .iter()
                            .filter(|(sig, _)| !panel.config.desks.contains_key(sig))
                            .map(|(sig, desk)| (sig.clone(), json!(desk)))
                            .collect();
                        if !back.is_empty() {
                            panel.patch(json!({ "desks": back }), window, cx);
                        }
                    };
                    panel.offer_undo(message, undo, window, cx);
                });
                true
            })
    });
}

/// "4 screens, in use" or "4 screens, last here 9 Nov". With the settings in a synced folder, a
/// desk this PC never saw is "4 screens, on another PC".
fn meta(sig: &str, desk: &Desk, in_use: bool, synced: bool) -> String {
    let screens = count(sig.split('|').count());
    match (in_use, desk.last_seen) {
        (true, _) => format!("{screens}, in use"),
        (false, 0) if synced => format!("{screens}, on another PC"),
        (false, 0) => screens,
        (false, seen) => match short_date(seen) {
            Some(date) => format!("{screens}, last here {date}"),
            None => screens,
        },
    }
}

/// One two-line item of the menu: the desk's name, and what is known of it.
fn desk_item(name: String, meta: String, in_use: bool) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        let p = palette(cx);
        let bar = div().flex_none().w(px(4.)).h(px(16.)).rounded_full().when(in_use, |d| d.bg(p.accent_ink));
        div().flex().items_center().gap(px(tokens::SPACE_8)).child(bar).child(
            div()
                .flex()
                .flex_col()
                .child(text_style(div(), tokens::BODY).child(name.clone()))
                .child(text_style(div(), tokens::CAPTION).text_color(p.text_muted).child(meta.clone())),
        )
    })
}

/// The subtitle under "Displays": the desk in use, and a menu of every desk Seam knows.
pub(super) fn desk_menu(panel: &Panel, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let active = panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default();
    let label = format!("{} desk, {}", panel.desk_name(), count(panel.mons.len()));
    let accessible = label.clone();
    let mut desks: Vec<(String, Desk)> = panel.config.desks.iter().map(|(s, d)| (s.clone(), d.clone())).collect();
    desks.sort_by_key(|(sig, d)| (*sig != active, std::cmp::Reverse(d.last_seen)));
    let (current, name) = (active.clone(), panel.desk_name());
    let synced = panel.config.config_folder.is_some();
    let view = cx.entity().downgrade();
    Button::new("desk-menu")
        .ghost()
        .label(label)
        .dropdown_caret(true)
        .tooltip("Desks Seam knows")
        .accessibility_label(accessible)
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu
                .min_w(px(300.))
                .max_w(px(300.))
                .item(PopupMenuItem::label("Desks Seam knows. It picks one by the screens it finds"));
            for (sig, desk) in &desks {
                menu = menu.item(desk_item(desk.name.clone(), meta(sig, desk, *sig == active, synced), *sig == active));
            }
            let (renaming, forgetting) = (view.clone(), view.clone());
            let (sig, name) = (current.clone(), name.clone());
            menu.separator()
                .item(PopupMenuItem::new("Rename this desk").on_click(move |_, window, cx| {
                    let (sig, name) = (sig.clone(), name.clone());
                    let _ = renaming.update(cx, |_, cx| rename(sig, name, window, cx));
                }))
                .item(PopupMenuItem::new("Forget desks not seen in 30 days").on_click(move |_, window, cx| {
                    let _ = forgetting.update(cx, |panel, cx| forget_old(panel, window, cx));
                }))
        })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would bring in gpui's own `test` attribute.
    use super::{Desk, meta, stale_desks};

    #[test]
    fn forget_old_desks_keeps_desks_this_pc_never_saw() {
        let day = 24 * 60 * 60;
        let desk = |last_seen| Desk { last_seen, ..Desk::default() };
        let desks = std::collections::BTreeMap::from([
            ("here".to_string(), desk(10 * day)),
            ("old".to_string(), desk(20 * day)),
            ("recent".to_string(), desk(90 * day)),
            // The other PC's desk: its `last_seen` stays on that PC, so it is 0 here.
            ("other-pc".to_string(), desk(0)),
        ]);
        let gone: Vec<String> = stale_desks(&desks, "here", 100 * day).into_iter().map(|(sig, _)| sig).collect();
        assert_eq!(gone, ["old"]);
        assert_eq!(meta("a|b", &desk(0), false, true), "2 screens, on another PC");
        assert_eq!(meta("a|b", &desk(0), false, false), "2 screens");
    }
}
