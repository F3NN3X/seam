//! The desk menu under the page title: the desks Seam knows, rename, and forget the old ones.

use super::*;

/// How long a desk can go unseen before "Forget desks not seen in 30 days" takes it.
const STALE_DAYS: u64 = 30;

/// A short local date, such as "9 Nov".
pub(super) fn short_date(unix: u64) -> Option<String> {
    winutil::local_date(unix).map(month_day)
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
/// connected has no date, so it stays.
fn stale(panel: &Panel) -> Vec<String> {
    let active = panel.state.as_ref().map(|s| s.desk.sig.as_str());
    let before = now().saturating_sub(STALE_DAYS * 24 * 60 * 60);
    let old = |d: &Desk| d.last_seen != 0 && d.last_seen < before;
    panel
        .config
        .desks
        .iter()
        .filter(|(sig, d)| Some(sig.as_str()) != active && old(d))
        .map(|(s, _)| s.clone())
        .collect()
}

/// Ask first, because forgetting a desk can't be undone.
fn forget_old(panel: &Panel, window: &mut Window, cx: &mut Context<Panel>) {
    let old = stale(panel);
    if old.is_empty() {
        window.push_notification(Notification::new().message("No desk has gone unseen for 30 days"), cx);
        return;
    }
    let view = cx.entity().downgrade();
    let what = if old.len() == 1 { "1 desk".to_string() } else { format!("{} desks", old.len()) };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, old) = (view.clone(), old.clone());
        alert
            .title(format!("Forget {what}?"))
            .description(
                "Seam last saw them more than 30 days ago. Their borders and zones are lost, and this can't be undone",
            )
            .ok_text(format!("Forget {what}"))
            .cancel_text("Cancel")
            .on_ok(move |_, window, cx| {
                let forgotten: serde_json::Map<String, serde_json::Value> =
                    old.iter().map(|sig| (sig.clone(), serde_json::Value::Null)).collect();
                let patch = json!({ "desks": forgotten });
                let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
                true
            })
    });
}

/// "4 screens, in use" or "4 screens, last here 9 Nov".
fn meta(sig: &str, desk: &Desk, in_use: bool) -> String {
    let screens = count(sig.split('|').count());
    match (in_use, desk.last_seen) {
        (true, _) => format!("{screens}, in use"),
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
    let mut desks: Vec<(String, Desk)> = panel.config.desks.iter().map(|(s, d)| (s.clone(), d.clone())).collect();
    desks.sort_by_key(|(sig, d)| (*sig != active, std::cmp::Reverse(d.last_seen)));
    let (current, name) = (active.clone(), panel.desk_name());
    let view = cx.entity().downgrade();
    Button::new("desk-menu")
        .ghost()
        .label(label)
        .dropdown_caret(true)
        .tooltip("Desks Seam knows")
        .accessibility_label("Desks")
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu
                .min_w(px(300.))
                .max_w(px(300.))
                .item(PopupMenuItem::label("Desks Seam knows. It picks one by the screens it finds"));
            for (sig, desk) in &desks {
                menu = menu.item(desk_item(desk.name.clone(), meta(sig, desk, *sig == active), *sig == active));
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
