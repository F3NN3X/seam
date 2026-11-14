//! The saved desks list, with rename and forget.

use super::*;

/// "today", "3 days ago" or "12 Oct", by the local calendar.
pub(super) fn relative(unix: u64) -> Option<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(relative_day(winutil::local_date(now)?, winutil::local_date(unix)?))
}

fn forget(panel: &mut Panel, sig: String, desk: Desk, window: &mut Window, cx: &mut Context<Panel>) {
    struct Undo;
    let name = desk.name.clone();
    panel.patch(json!({ "desks": { sig.clone(): null } }), window, cx);
    let view = cx.entity().downgrade();
    let note = Notification::new().id::<Undo>().message(format!("Forgot {name}")).action(move |_, _, cx| {
        let (view, sig, desk) = (view.clone(), sig.clone(), desk.clone());
        ui::primary("undo", "Undo").on_click(cx.listener(move |note: &mut Notification, _, window, cx| {
            let patch = json!({ "desks": { sig.clone(): desk } });
            let _ = view.update(cx, |panel, cx| panel.patch(patch, window, cx));
            note.dismiss(window, cx);
        }))
    });
    window.push_notification(note, cx);
    panel.toast_generation += 1;
    let generation = panel.toast_generation;
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(UNDO_FOR).await;
        let _ = this.update_in(cx, |panel, window, cx| {
            if panel.toast_generation == generation {
                window.remove_notification::<Undo>(cx);
            }
        });
    })
    .detach();
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

pub(super) fn desk_rows(panel: &Panel, window: &Window, cx: &mut Context<Panel>) -> Vec<AnyElement> {
    let p = palette(cx);
    let _ = window;
    let active = panel.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default();
    let mut desks: Vec<(String, Desk)> = panel.config.desks.iter().map(|(s, d)| (s.clone(), d.clone())).collect();
    desks.sort_by_key(|(sig, d)| (*sig != active, std::cmp::Reverse(d.last_seen)));
    let rows = desks.into_iter().enumerate().map(|(i, (sig, desk))| {
        let in_use = sig == active;
        let screens = sig.split('|').count();
        let count = format!("{screens} screen{}", if screens == 1 { "" } else { "s" });
        let about = match (in_use, desk.last_seen) {
            (true, _) if !desk.borders.is_empty() => match desk.lined_up_at.and_then(relative) {
                Some(when) => format!("{count}, lined up {when}"),
                None => format!("{count}, lined up"),
            },
            (true, _) | (false, 0) => count,
            (false, seen) => match winutil::local_date(seen) {
                Some(date) => format!("{count}, last connected {}", month_day(date)),
                None => count,
            },
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(14.))
            .child(desk.name.clone())
            .when(in_use, |d| d.child(div().text_size(px(12.)).text_color(p.muted).child("in use")));
        let view = cx.entity().downgrade();
        let more = ui::secondary(("more", i), "", cx)
            .ghost()
            .icon(Icon::new(assets::IconName::Ellipsis))
            .accessibility_label(format!("More for {}", desk.name))
            .dropdown_menu(move |menu, _, _| {
                let (rename_sig, name) = (sig.clone(), desk.name.clone());
                let (forget_sig, forget_desk) = (sig.clone(), desk.clone());
                let (first, second) = (view.clone(), view.clone());
                menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                    let (sig, name) = (rename_sig.clone(), name.clone());
                    let _ = first.update(cx, |_, cx| rename(sig, name, window, cx));
                }))
                .item(PopupMenuItem::new("Forget").on_click(move |_, window, cx| {
                    let (sig, desk) = (forget_sig.clone(), forget_desk.clone());
                    let _ = second.update(cx, |panel, cx| forget(panel, sig, desk, window, cx));
                }))
            });
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .min_h(px(56.))
            .px(px(16.))
            .py(px(8.))
            .child(div().flex().flex_col().gap(px(2.)).flex_1().child(title).child(ui::description(about, p)))
            .child(more)
            .into_any_element()
    });
    rows.collect()
}
