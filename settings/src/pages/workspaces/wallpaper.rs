//! The pictures each screen shows (brief K): "Wallpaper" and "When no workspace sets one" on the
//! page, and "Wallpaper for «Name»" in a workspace's card. Every choice goes through
//! `wallpaper.set`, which checks the picture, with the usual Undo.

use super::desk_sig;
use crate::panel::{Panel, error_text};
use crate::theme::Palette;
use crate::theme::tokens::{self, text_style};
use crate::ui::{self, Choice, Row, card, row, row_with, section, switch};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Icon, WindowExt};
use gpui_kit::*;
use seam::api::Event;
use seam::config::{Fit, Wallpaper};
use seam::workspace::Workspace;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SWITCH_TITLE: &str = "Change wallpaper with workspaces";
const ON_ABOUT: &str =
    "Each screen shows the picture its workspace sets. Turning this off puts your own wallpaper back";
const SPOTLIGHT_ABOUT: &str = "Turning this on replaces your Windows Spotlight pictures until you turn it off";
const SLIDESHOW_ABOUT: &str = "Turning this on replaces your slideshow until you turn it off";
const OFF_TOOLTIP: &str = "Turn on Change wallpaper with workspaces first";
const NOT_CONNECTED: &str = "Screen not connected";
/// The fits in the order the Select offers them.
const FITS: [(Fit, &str); 5] =
    [(Fit::Fill, "Fill"), (Fit::Fit, "Fit"), (Fit::Stretch, "Stretch"), (Fit::Center, "Center"), (Fit::Tile, "Tile")];
/// Wide enough for "Stretch".
const FIT_WIDTH: f32 = 120.;
/// The picture's thumbnail, 16:9.
const THUMB: (f32, f32) = (48., 27.);
/// Ids of the detail card's rows, past the page's.
const DETAIL_SLOT: usize = 100;

/// What the window knows about the pictures beyond the settings.
#[derive(Clone, Debug, Default)]
pub struct WallpaperView {
    /// Mine is Windows Spotlight, or a slideshow (`wallpaper.state`).
    pub spotlight: bool,
    pub slideshow: bool,
    /// Pictures that aren't there any more, by path.
    pub missing: BTreeSet<String>,
    /// Why a picture wasn't taken, under its row: by workspace id ("" for the desk default) and screen.
    pub errors: BTreeMap<(String, String), String>,
}

/// A screen a row is for.
struct On {
    id: String,
    number: Option<u32>,
    name: String,
    connected: bool,
}

/// The connected screens, in Line Up order.
fn connected(panel: &Panel) -> Vec<On> {
    let screens = panel.state.as_ref().map(|s| s.screens.as_slice()).unwrap_or_default();
    let on = |s: &seam::state::Screen| On {
        id: s.id.clone(),
        number: Some(s.number),
        name: s.name.clone(),
        connected: true,
    };
    screens.iter().map(on).collect()
}

/// The screen a one-screen workspace is for, connected or as the desk last saw it.
fn scope_screen(panel: &Panel, id: &str) -> On {
    if let Some(on) = connected(panel).into_iter().find(|s| s.id == id) {
        return on;
    }
    let desk = panel.config.desks.get(&desk_sig(panel));
    let seen = desk.and_then(|d| d.screens.iter().find(|s| s.id == id));
    On {
        id: id.to_string(),
        number: seen.map(|s| s.number),
        name: seen.map(|s| s.name.clone()).unwrap_or_default(),
        connected: false,
    }
}

/// The picture a workspace, or the desk default with `None`, sets for a screen.
fn picture(panel: &Panel, ws: Option<&str>, screen: &str) -> Option<Wallpaper> {
    let desk = panel.config.desks.get(&desk_sig(panel))?;
    let set = match ws {
        Some(id) => desk.workspace_wallpapers.get(id)?,
        None => &desk.wallpaper,
    };
    set.screens.get(screen).cloned()
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The row's second line after the screen's name: the picture's file, or what shows without one.
fn state_words(picture: Option<&Wallpaper>, desk_default: bool, missing: bool, connected: bool) -> String {
    match picture {
        _ if !connected => NOT_CONNECTED.to_string(),
        None if desk_default => "Your own wallpaper".to_string(),
        None => "Desk default".to_string(),
        Some(w) if missing => format!("{} isn't there any more", file_name(&w.path)),
        Some(w) => file_name(&w.path),
    }
}

fn second_line(name: &str, words: String) -> String {
    if name.is_empty() { words } else { format!("{name}. {words}") }
}

fn screen_title(on: &On) -> String {
    on.number.map_or_else(|| "Screen".to_string(), |n| format!("Screen {n}"))
}

/// Ask the tray what each screen shows and what mine is, and look off the UI thread for pictures
/// that are gone.
pub fn refresh(panel: &Panel, cx: &mut Context<Panel>) {
    let Some(client) = panel.client.clone() else { return };
    let desk = panel.config.desks.get(&desk_sig(panel));
    let sets = desk.into_iter().flat_map(|d| std::iter::once(&d.wallpaper).chain(d.workspace_wallpapers.values()));
    let paths: BTreeSet<String> = sets.flat_map(|s| s.screens.values().map(|w| w.path.clone())).collect();
    let answer = cx.background_executor().spawn(async move {
        let state = client.lock().ok()?.call("wallpaper.state", json!({})).ok();
        let missing: BTreeSet<String> = paths.into_iter().filter(|p| !Path::new(p).exists()).collect();
        Some((state, missing))
    });
    cx.spawn(async move |this, cx| {
        let Some((state, missing)) = answer.await else { return };
        let _ = this.update(cx, |panel, cx| {
            let view = &mut panel.wallpaper;
            if let Some(state) = state {
                view.spotlight = state["original"]["spotlight"].as_bool().unwrap_or(false);
                view.slideshow = state["original"]["slideshow"].as_bool().unwrap_or(false);
            }
            view.missing = missing;
            cx.notify();
        });
    })
    .detach();
}

/// `wallpaper.changed` and `wallpaper.failed`: mine back with Spotlight in it says how to turn it on
/// again, and a picture the worker couldn't open says so under its rows.
pub fn event(panel: &mut Panel, event: &Event, window: &mut Window, cx: &mut Context<Panel>) {
    match event.kind.as_str() {
        "wallpaper.changed" if event.data["restored"] == true && event.data["original"]["spotlight"] == true => {
            window.push_notification(spotlight_notice(), cx);
        }
        "wallpaper.failed" => {
            let (screen, path) = (event.data["screen"].as_str(), event.data["path"].as_str());
            let message = event.data["message"].as_str().filter(|m| !m.is_empty());
            if let (Some(screen), Some(path), Some(message)) = (screen, path, message) {
                let desk = panel.config.desks.get(&desk_sig(panel));
                let sets = desk.into_iter().flat_map(|d| {
                    std::iter::once((String::new(), &d.wallpaper))
                        .chain(d.workspace_wallpapers.iter().map(|(id, set)| (id.clone(), set)))
                });
                let rows: Vec<String> = sets
                    .filter(|(_, set)| set.screens.get(screen).is_some_and(|w| w.path == path))
                    .map(|(id, _)| id)
                    .collect();
                for id in rows {
                    panel.wallpaper.errors.insert((id, screen.to_string()), message.to_string());
                }
            }
        }
        _ => {}
    }
    refresh(panel, cx);
}

fn spotlight_notice() -> Notification {
    Notification::new().title("Your wallpaper is back").message("Turn Windows Spotlight on again in Settings").action(
        |_, _, cx| {
            ui::secondary("open-background", "Open background settings", cx)
                .on_click(|_, _, cx| cx.open_url("ms-settings:personalization-background"))
        },
    )
}

/// "Change wallpaper with workspaces". Off, with Spotlight or a slideshow on, it says what turning it on replaces.
fn switch_row(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let view = cx.entity();
    let on = panel.config.wallpapers_enabled;
    let toggle = switch("wallpapers", SWITCH_TITLE, on, window, cx, move |on, window, cx| {
        view.update(cx, |panel, cx| panel.patch(json!({ "wallpapers_enabled": on }), window, cx));
    });
    let about = match (on, panel.wallpaper.spotlight, panel.wallpaper.slideshow) {
        (false, true, _) => SPOTLIGHT_ABOUT,
        (false, _, true) => SLIDESHOW_ABOUT,
        _ => ON_ABOUT,
    };
    row(SWITCH_TITLE, Some(about.into()), toggle, p)
}

/// The page's two sections: the switch, and the desk default for each connected screen.
pub fn page_sections(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> [AnyElement; 2] {
    let switching = section("Wallpaper", card(vec![switch_row(panel, p, window, cx)], p), p);
    let rows = connected(panel).iter().enumerate().map(|(i, on)| picture_row(panel, None, on, i, p, cx)).collect();
    [switching.into_any_element(), section("When no workspace sets one", card(rows, p), p).into_any_element()]
}

/// "Wallpaper for «Name»": every connected screen for an All-screens workspace, only its own for a
/// one-screen one (D793a), even while that one isn't connected.
pub fn detail_section(panel: &Panel, ws: &Workspace, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let screens = match &ws.scope {
        Some(id) => vec![scope_screen(panel, id)],
        None => connected(panel),
    };
    let rows = screens.iter().enumerate().map(|(i, on)| picture_row(panel, Some(ws), on, DETAIL_SLOT + i, p, cx));
    section(format!("Wallpaper for {}", ws.name), card(rows.collect(), p), p).into_any_element()
}

/// One screen's row: its picture or what shows without one, and the control to choose one.
fn picture_row(
    panel: &Panel,
    ws: Option<&Workspace>,
    on: &On,
    slot: usize,
    p: Palette,
    cx: &mut Context<Panel>,
) -> Row {
    let id = ws.map(|w| w.id.clone());
    let chosen = picture(panel, id.as_deref(), &on.id);
    let missing = chosen.as_ref().is_some_and(|w| panel.wallpaper.missing.contains(&w.path));
    let words = state_words(chosen.as_ref(), ws.is_none(), missing, on.connected);
    let error = panel.wallpaper.errors.get(&(id.clone().unwrap_or_default(), on.id.clone())).cloned();
    let about = div()
        .flex()
        .flex_col()
        .child(ui::description(second_line(&on.name, words), p))
        .children(error.map(|e| text_style(div(), tokens::LABEL).text_color(p.negative).child(e)));
    let title = screen_title(on);
    let control = match &chosen {
        None => ui::secondary(("wallpaper-choose", slot), "Choose picture", cx)
            .on_click({
                let (id, screen, title) = (id.clone(), on.id.clone(), title.clone());
                cx.listener(move |_, _, window, cx| choose(id.clone(), screen.clone(), title.clone(), window, cx))
            })
            .into_any_element(),
        Some(w) => chosen_controls(w, id.clone(), on, slot, missing, cx),
    };
    let row = row_with(title, Some(about.into_any_element()), control);
    let why = if !panel.config.wallpapers_enabled {
        Some(OFF_TOOLTIP)
    } else if !on.connected {
        Some(NOT_CONNECTED)
    } else {
        None
    };
    ui::dimmed(row, ("wallpaper-row", slot), why)
}

/// A chosen picture: its thumbnail, the `…` and the fit.
fn chosen_controls(
    picture: &Wallpaper,
    ws: Option<String>,
    on: &On,
    slot: usize,
    missing: bool,
    cx: &mut Context<Panel>,
) -> AnyElement {
    let title = screen_title(on);
    let thumb = (!missing).then(|| {
        img(PathBuf::from(&picture.path))
            .w(px(THUMB.0))
            .h(px(THUMB.1))
            .rounded(px(tokens::RADIUS_4))
            .object_fit(ObjectFit::Cover)
    });
    let weak = cx.entity().downgrade();
    let (menu_ws, menu_screen, menu_title) = (ws.clone(), on.id.clone(), title.clone());
    let more = ui::secondary(("wallpaper-more", slot), "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for {title}"))
        .dropdown_menu(move |menu, _, _| {
            let (ws, screen, title) = (menu_ws.clone(), menu_screen.clone(), menu_title.clone());
            let (change_ws, change_screen, change_title) = (ws.clone(), screen.clone(), title.clone());
            let weak_change = weak.clone();
            let weak_remove = weak.clone();
            menu.item(PopupMenuItem::new("Change picture…").on_click(move |_, window, cx| {
                let (ws, screen, title) = (change_ws.clone(), change_screen.clone(), change_title.clone());
                let _ = weak_change.update(cx, |_, cx| choose(ws, screen, title, window, cx));
            }))
            .item(PopupMenuItem::new("Remove picture").on_click(move |_, window, cx| {
                let (ws, screen) = (ws.clone(), screen.clone());
                let note = format!("Removed the picture for {title}");
                let _ = weak_remove.update(cx, |panel, cx| set(panel, ws, screen, None, None, note, window, cx));
            }))
        })
        .into_any_element();
    let choices: Vec<Choice> = FITS.iter().map(|(fit, label)| Choice::new(fit_key(*fit), *label)).collect();
    let view = cx.entity();
    let (path, screen) = (picture.path.clone(), on.id.clone());
    let pick = move |key: &str, window: &mut Window, cx: &mut App| {
        let Some((fit, label)) = FITS.iter().find(|(fit, _)| fit_key(*fit) == key) else { return };
        let note = format!("{title} fits its picture with {label}");
        let (ws, screen, path, fit) = (ws.clone(), screen.clone(), path.clone(), *fit);
        view.update(cx, |panel, cx| set(panel, ws, screen, Some(path), Some(fit), note, window, cx));
    };
    let fit = ui::select(("wallpaper-fit", slot), "Fit", choices, &fit_key(picture.fit), FIT_WIDTH, cx, pick);
    div().flex().items_center().gap(px(tokens::SPACE_8)).children(thumb).child(more).child(fit).into_any_element()
}

fn fit_key(fit: Fit) -> String {
    serde_json::to_value(fit).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The file picker, then `wallpaper.set` with what was picked. `title` names the screen in the Undo.
fn choose(ws: Option<String>, screen: String, title: String, window: &mut Window, cx: &mut Context<Panel>) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Choose picture".into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        let Some(path) = paths.first().map(|p| p.display().to_string()) else { return };
        let _ = this.update_in(cx, move |panel, window, cx| {
            let note = format!("Chose a picture for {title}");
            set(panel, ws, screen, Some(path), None, note, window, cx);
        });
    })
    .detach();
}

/// Set a picture, or with no `path` take it away, with Undo back to what it was.
#[allow(clippy::too_many_arguments)]
fn set(
    panel: &mut Panel,
    ws: Option<String>,
    screen: String,
    path: Option<String>,
    fit: Option<Fit>,
    note: String,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let before = picture(panel, ws.as_deref(), &screen);
    send(panel, ws, screen, path, fit, Some((note, before)), window, cx);
}

#[allow(clippy::too_many_arguments)]
fn send(
    panel: &mut Panel,
    ws: Option<String>,
    screen: String,
    path: Option<String>,
    fit: Option<Fit>,
    undo: Option<(String, Option<Wallpaper>)>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let key = (ws.clone().unwrap_or_default(), screen.clone());
    panel.wallpaper.errors.remove(&key);
    let params = json!({ "workspace": ws, "screen": screen, "path": path, "fit": fit });
    panel.call("wallpaper.set", params, window, cx, move |panel, result, window, cx| {
        match (result, undo) {
            (Ok(_), Some((note, before))) => {
                let back = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                    let (path, fit) = before.clone().map(|w| (w.path, w.fit)).unzip();
                    send(panel, ws.clone(), screen.clone(), path, fit, None, window, cx);
                };
                panel.offer_undo(note, back, window, cx);
            }
            (Ok(_), None) => {}
            (Err(e), _) => {
                panel.wallpaper.errors.insert(key, error_text(&e));
            }
        }
        panel.reload_config(window, cx);
        refresh(panel, cx);
        cx.notify();
    });
}

/// After a duplicate is saved: the copy gets the source's pictures, one `wallpaper.set` after
/// another, with no Undo of their own (the duplicate's covers them).
pub fn copy_pictures(panel: &mut Panel, from: &str, to: String, window: &mut Window, cx: &mut Context<Panel>) {
    let pictures = pictures_of(panel, from);
    put_pictures(panel, to, pictures, window, cx);
}

/// The pictures a workspace sets, by screen, to give back later with `put_pictures`.
pub fn pictures_of(panel: &Panel, id: &str) -> Vec<(String, Wallpaper)> {
    let desk = panel.config.desks.get(&desk_sig(panel));
    let set = desk.and_then(|d| d.workspace_wallpapers.get(id));
    set.map(|s| s.screens.clone().into_iter().collect()).unwrap_or_default()
}

/// `wallpaper.set` for each picture, one after another, for workspace `to`.
pub fn put_pictures(
    panel: &mut Panel,
    to: String,
    pictures: Vec<(String, Wallpaper)>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    copy_next(panel, to, pictures, window, cx);
}

fn copy_next(
    panel: &mut Panel,
    to: String,
    mut left: Vec<(String, Wallpaper)>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let Some((screen, w)) = left.pop() else {
        panel.reload_config(window, cx);
        return;
    };
    let params = json!({ "workspace": to, "screen": screen, "path": w.path, "fit": w.fit });
    panel.call("wallpaper.set", params, window, cx, move |panel, _, window, cx| copy_next(panel, to, left, window, cx));
}

#[cfg(test)]
mod tests {
    use super::{FITS, Fit, Wallpaper, fit_key, second_line, state_words};

    fn picture(path: &str) -> Wallpaper {
        Wallpaper { path: path.into(), fit: Fit::Fill }
    }

    #[test]
    fn rows_say_what_shows_without_a_picture_and_which_file_with_one() {
        let w = picture(r"D:\Pictures\Lake.jpg");
        assert_eq!(second_line("PG32UCDM", state_words(None, true, false, true)), "PG32UCDM. Your own wallpaper");
        assert_eq!(second_line("PG32UCDM", state_words(None, false, false, true)), "PG32UCDM. Desk default");
        assert_eq!(state_words(Some(&w), true, false, true), "Lake.jpg");
        assert_eq!(state_words(Some(&w), false, true, true), "Lake.jpg isn't there any more");
        assert_eq!(state_words(Some(&w), false, false, false), "Screen not connected");
        assert_eq!(second_line("", state_words(None, true, false, true)), "Your own wallpaper");
    }

    #[test]
    fn fits_are_offered_in_order_with_their_file_names() {
        let keys: Vec<String> = FITS.iter().map(|(fit, _)| fit_key(*fit)).collect();
        assert_eq!(keys, ["fill", "fit", "stretch", "center", "tile"]);
    }
}
