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
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{Icon, WindowExt};
use gpui_kit::*;
use seam::api::Event;
use seam::config::{Fit, MAX_SCREEN_GAP_MM, Wallpaper, WallpaperSet};
use seam::wallpaper::span::{self, Layout};
use seam::wallpaper::{Origin, choices};
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
/// How many of `FITS` a picture across all screens has: Fill, Fit and Stretch.
const ACROSS_FITS: usize = 3;
/// Wide enough for "Stretch".
const FIT_WIDTH: f32 = 120.;
/// The picture's thumbnail, 16:9.
const THUMB: (f32, f32) = (48., 27.);
/// Ids of the detail card's rows, past the page's.
const DETAIL_SLOT: usize = 100;
/// The "screen" of the row for the picture across all screens: no real screen has this id.
const ACROSS: &str = "";
/// Its place among the rows' ids, past the screens'.
const ACROSS_SLOT: usize = 50;
const ACROSS_TITLE: &str = "Across all screens";
const ACROSS_NONE: &str = "One picture, cut to where each screen stands";
const ON_THE_DESK: &str = "Cut to where each screen stands on the desk";
const FOLLOWS_WINDOWS: &str = "Follows Windows' arrangement, so it may not line up at the edges";
const PART_OF_IT: &str = "Part of the picture across all screens";
const LEFT_OUT: &str = "Not next to the other screens, so it shows the whole picture";
const NOTE_LINE_UP: &str =
    "Your picture across the screens follows Windows' arrangement. Line up your screens so it matches where they stand";
const NEEDS_LINE_UP: &str = "Needs your screens lined up";
const GAP_TITLE: &str = "Space between screens";
const GAP_ABOUT: &str = "The picture skips this much where two screens meet, for the bezels. 0 is none";
/// The preview of the picture over the screens.
const PREVIEW: (f32, f32) = (360., 120.);

/// What the window knows about the pictures beyond the settings.
#[derive(Clone, Debug, Default)]
pub struct WallpaperView {
    /// Mine is Windows Spotlight, or a slideshow (`wallpaper.state`).
    pub spotlight: bool,
    pub slideshow: bool,
    /// Pictures that aren't there any more, by path.
    pub missing: BTreeSet<String>,
    /// The size of the pictures across all screens, by path, for the preview. Not for a kind of
    /// file whose size isn't in its first bytes.
    pub sizes: BTreeMap<String, (u32, u32)>,
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
    match screen {
        ACROSS => set.span.clone(),
        _ => set.screens.get(screen).cloned(),
    }
}

/// How the screens are cut for a picture across all screens (D804).
fn layout(panel: &Panel) -> Layout {
    let gap = panel.config.desks.get(&desk_sig(panel)).map_or(0, |d| d.screen_gap_mm);
    span::span_rects(&panel.mons, &panel.links, gap as f64).1
}

/// Whether the screen with Line Up number `number` is left out of a picture across all screens.
fn left_out(panel: &Panel, number: u32) -> bool {
    let (_, reached) = seam::desk::walk(&panel.mons, &panel.links, 0.0);
    number.checked_sub(1).is_some_and(|i| reached.get(i as usize) == Some(&false))
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The row's second line after the screen's name: the picture's file, or what shows without one.
/// `span` says whether the set has a picture across all screens, and if so whether this screen is
/// in it: without a picture of its own, that is what it shows.
fn state_words(
    picture: Option<&Wallpaper>,
    desk_default: bool,
    missing: bool,
    connected: bool,
    span: Option<bool>,
) -> String {
    match picture {
        _ if !connected => NOT_CONNECTED.to_string(),
        None if span == Some(true) => PART_OF_IT.to_string(),
        None if span == Some(false) => LEFT_OUT.to_string(),
        None if desk_default => "Your own wallpaper".to_string(),
        None => "Desk default".to_string(),
        Some(w) if missing => format!("{} isn't there any more", file_name(&w.path)),
        Some(w) => file_name(&w.path),
    }
}

/// The second line of the row for the picture across all screens.
fn across_words(picture: Option<&Wallpaper>, missing: bool, on_the_desk: bool, several: bool) -> String {
    match picture {
        None => ACROSS_NONE.to_string(),
        Some(w) if missing => format!("{} isn't there any more", file_name(&w.path)),
        // One screen has no edges to line up.
        Some(w) if !several => file_name(&w.path),
        Some(w) if on_the_desk => format!("{}. {ON_THE_DESK}", file_name(&w.path)),
        Some(w) => format!("{}. {FOLLOWS_WINDOWS}", file_name(&w.path)),
    }
}

fn second_line(name: &str, words: String) -> String {
    if name.is_empty() { words } else { format!("{name}. {words}") }
}

fn screen_title(on: &On) -> String {
    on.number.map_or_else(|| "Screen".to_string(), |n| format!("Screen {n}"))
}

/// What the row is called in an Undo note: "Chose a picture for all screens".
fn noted(on: &On) -> String {
    if on.id == ACROSS { "all screens".to_string() } else { screen_title(on) }
}

fn fit_note(across: bool, title: &str, label: &str) -> String {
    match across {
        true => format!("The picture across all screens fits with {label}"),
        false => format!("{title} fits its picture with {label}"),
    }
}

/// Ask the tray what each screen shows and what mine is, and look off the UI thread for pictures
/// that are gone.
pub fn refresh(panel: &Panel, cx: &mut Context<Panel>) {
    let Some(client) = panel.client.clone() else { return };
    let desk = panel.config.desks.get(&desk_sig(panel));
    let sets = desk.into_iter().flat_map(|d| std::iter::once(&d.wallpaper).chain(d.workspace_wallpapers.values()));
    let pictures = sets.flat_map(|s| s.screens.values().chain(&s.span).map(|w| w.path.clone()));
    let paths: BTreeSet<String> = pictures.collect();
    let desk = panel.config.desks.get(&desk_sig(panel));
    let spans: BTreeSet<String> = desk
        .into_iter()
        .flat_map(|d| std::iter::once(&d.wallpaper).chain(d.workspace_wallpapers.values()))
        .filter_map(|s| s.span.as_ref().map(|w| w.path.clone()))
        .collect();
    let answer = cx.background_executor().spawn(async move {
        let state = client.lock().ok()?.call("wallpaper.state", json!({})).ok();
        let missing: BTreeSet<String> = paths.into_iter().filter(|p| !Path::new(p).exists()).collect();
        let sizes: BTreeMap<String, (u32, u32)> =
            spans.into_iter().filter_map(|p| seam::wallpaper::file_size(&p).map(|size| (p, size))).collect();
        Some((state, missing, sizes))
    });
    cx.spawn(async move |this, cx| {
        let Some((state, missing, sizes)) = answer.await else { return };
        let _ = this.update(cx, |panel, cx| {
            let view = &mut panel.wallpaper;
            if let Some(state) = state {
                view.spotlight = state["original"]["spotlight"].as_bool().unwrap_or(false);
                view.slideshow = state["original"]["slideshow"].as_bool().unwrap_or(false);
            }
            view.missing = missing;
            view.sizes = sizes;
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
                let rows: Vec<(String, String)> = sets
                    .filter_map(|(id, set)| failed_on(set, screen, path).map(|row| (id, row.to_string())))
                    .collect();
                for key in rows {
                    panel.wallpaper.errors.insert(key, message.to_string());
                }
            }
        }
        _ => {}
    }
    refresh(panel, cx);
}

/// The row a picture that failed on `screen` is reported under in `set`: the screen's own, else the
/// Across all screens row when the picture is the set's span.
fn failed_on<'a>(set: &WallpaperSet, screen: &'a str, path: &str) -> Option<&'a str> {
    if set.screens.get(screen).is_some_and(|w| w.path == path) {
        return Some(screen);
    }
    set.span.as_ref().is_some_and(|w| w.path == path).then_some(ACROSS)
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

/// "Space between screens": how much of a picture across all screens the bezels skip (brief L, Q1).
fn gap_row(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> Row {
    let gap = panel.config.desks.get(&desk_sig(panel)).map_or(0, |d| d.screen_gap_mm).min(MAX_SCREEN_GAP_MM);
    // The slider follows the setting when something else changed it.
    if panel.screen_gap.read(cx).value().end() != gap as f32 {
        panel.screen_gap.update(cx, |state, cx| state.set_value(gap as f32, window, cx));
    }
    let value = if gap == 0 { "None".to_string() } else { format!("{gap} mm") };
    let on = panel.config.wallpapers_enabled;
    // The gap is for the physical layout only, so it needs the screens lined up.
    let lined_up = layout(panel) == Layout::Desk;
    let slider = ui::slider_with_value(&panel.screen_gap, value, on && lined_up, GAP_TITLE, p);
    let why = if !on { Some(OFF_TOOLTIP) } else { (!lined_up).then_some(NEEDS_LINE_UP) };
    ui::dimmed(row(GAP_TITLE, Some(GAP_ABOUT.into()), slider, p), "screen-gap-off", why)
}

/// The Space between screens slider shows the value as it moves and sends it when let go.
pub fn gap_subscription(slider: &Entity<SliderState>, window: &mut Window, cx: &mut Context<Panel>) -> Subscription {
    cx.subscribe_in(slider, window, |this, _, event: &SliderEvent, window, cx| {
        let sig = desk_sig(this);
        match event {
            SliderEvent::Change(value) => {
                this.config_mut().desks.entry(sig).or_default().screen_gap_mm = value.end() as u32;
                cx.notify();
            }
            SliderEvent::Release(value) => {
                this.patch(json!({ "desks": { sig: { "screen_gap_mm": value.end() as u32 } } }), window, cx);
            }
        }
    })
}

/// A Note while a picture across all screens is in use on this desk and doesn't follow where the
/// screens stand: "Line up screens" when that would help, and no button when a screen doesn't
/// report its size.
fn span_note(panel: &Panel, p: Palette, cx: &mut Context<Panel>) -> Option<Div> {
    let desk = panel.config.desks.get(&desk_sig(panel))?;
    let in_use = panel.mons.iter().any(|m| choices(desk, &m.id).first().is_some_and(|c| c.1 == Origin::Span));
    if !panel.config.wallpapers_enabled || panel.mons.len() < 2 || !in_use || layout(panel) == Layout::Desk {
        return None;
    }
    if let Some(i) = span::unsized_screens(&panel.mons, &panel.links).first() {
        let text = format!(
            "Screen {} doesn't report its size, so the picture across the screens follows Windows' arrangement",
            i + 1
        );
        return Some(ui::note(text, None, p));
    }
    let line_up = ui::secondary("line-up-span", "Line up screens", cx)
        .on_click(cx.listener(|panel, _, window, cx| panel.command("lineup.start", json!({}), window, cx)));
    Some(ui::note(NOTE_LINE_UP, Some(line_up), p))
}

/// The page's two sections: the switch, and the desk default for each connected screen.
pub fn page_sections(panel: &Panel, p: Palette, window: &mut Window, cx: &mut Context<Panel>) -> [AnyElement; 2] {
    let top = card(vec![switch_row(panel, p, window, cx), gap_row(panel, p, window, cx)], p);
    let switching = match span_note(panel, p, cx) {
        Some(note) => section("Wallpaper", div().flex().flex_col().gap(px(tokens::SPACE_12)).child(note).child(top), p),
        None => section("Wallpaper", top, p),
    };
    let screens = connected(panel);
    let mut rows = across_rows(panel, None, &screens, ACROSS_SLOT, p, cx);
    rows.extend(screens.iter().enumerate().map(|(i, on)| picture_row(panel, None, on, i, p, cx)));
    [switching, section("When no workspace sets one", card(rows, p), p).into_any_element()]
}

/// The row for the picture across all screens, then its preview over the screens, when there are
/// several screens to cut it for.
fn across_rows(
    panel: &Panel,
    ws: Option<&Workspace>,
    screens: &[On],
    slot: usize,
    p: Palette,
    cx: &mut Context<Panel>,
) -> Vec<Row> {
    if screens.is_empty() {
        return Vec::new();
    }
    let on = On { id: ACROSS.to_string(), number: None, name: String::new(), connected: true };
    let mut rows = vec![picture_row(panel, ws, &on, slot, p, cx)];
    let chosen = picture(panel, ws.map(|w| w.id.as_str()), ACROSS);
    let preview = chosen.filter(|w| !panel.wallpaper.missing.contains(&w.path)).and_then(|w| preview(panel, &w, p));
    rows.extend(preview.filter(|_| screens.len() > 1).map(Row::from));
    rows
}

/// The picture over the screens, each showing the part it is cut to, as the screens stand (brief L,
/// the preview over the screen map). Not for a picture whose size isn't known.
fn preview(panel: &Panel, picture: &Wallpaper, p: Palette) -> Option<AnyElement> {
    let src = panel.wallpaper.sizes.get(&picture.path)?;
    let gap = panel.config.desks.get(&desk_sig(panel)).map_or(0, |d| d.screen_gap_mm);
    let (rects, _) = span::span_rects(&panel.mons, &panel.links, gap as f64);
    let bounds = span::bounds(&rects)?;
    let scale = (PREVIEW.0 as f64 / bounds.w).min(PREVIEW.1 as f64 / bounds.h);
    let shown = span::fitted(*src, bounds, picture.fit.across());
    let at = |v: f64| px((v * scale) as f32);
    let tiles = rects.iter().flatten().map(|r| {
        let (x, y) = (shown.x - r.x, shown.y - r.y);
        let image = img(PathBuf::from(&picture.path))
            .absolute()
            .left(at(x))
            .top(at(y))
            .w(at(shown.w))
            .h(at(shown.h))
            .object_fit(ObjectFit::Fill);
        div()
            .absolute()
            .left(at(r.x - bounds.x))
            .top(at(r.y - bounds.y))
            .w(at(r.w))
            .h(at(r.h))
            .overflow_hidden()
            .bg(p.well())
            .border_1()
            .border_color(p.hairline_strong)
            .rounded(px(tokens::RADIUS_2))
            .child(image)
    });
    let map = div().relative().w(at(bounds.w)).h(at(bounds.h)).children(tiles);
    let centred = div().flex().justify_center().px(px(tokens::SPACE_16)).py(px(tokens::SPACE_12)).child(map);
    Some(centred.into_any_element())
}

/// "Wallpaper for «Name»": every connected screen for an All-screens workspace, only its own for a
/// one-screen one (D793a), even while that one isn't connected.
pub fn detail_section(panel: &Panel, ws: &Workspace, p: Palette, cx: &mut Context<Panel>) -> AnyElement {
    let screens = match &ws.scope {
        Some(id) => vec![scope_screen(panel, id)],
        None => connected(panel),
    };
    // A one-screen workspace has no picture across all screens (D801).
    let mut rows = match &ws.scope {
        None => across_rows(panel, Some(ws), &screens, DETAIL_SLOT + ACROSS_SLOT, p, cx),
        Some(_) => Vec::new(),
    };
    rows.extend(screens.iter().enumerate().map(|(i, on)| picture_row(panel, Some(ws), on, DETAIL_SLOT + i, p, cx)));
    section(format!("Wallpaper for {}", ws.name), card(rows, p), p).into_any_element()
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
    let across = on.id == ACROSS;
    // Without a picture of its own, a screen shows its part of the set's picture across all screens.
    let set_span = picture(panel, id.as_deref(), ACROSS).is_some();
    let in_span = (set_span && !across && ws.is_none_or(|w| w.scope.is_none()))
        .then(|| on.number.is_some_and(|n| !left_out(panel, n)));
    let words = match across {
        true => across_words(chosen.as_ref(), missing, layout(panel) == Layout::Desk, panel.mons.len() > 1),
        false => state_words(chosen.as_ref(), ws.is_none(), missing, on.connected, in_span),
    };
    let error = panel.wallpaper.errors.get(&(id.clone().unwrap_or_default(), on.id.clone())).cloned();
    let about = div()
        .flex()
        .flex_col()
        .child(ui::description(second_line(&on.name, words), p))
        .children(error.map(|e| text_style(div(), tokens::LABEL).text_color(p.negative).child(e)));
    let title = if across { ACROSS_TITLE.to_string() } else { screen_title(on) };
    let control = match &chosen {
        None => ui::secondary(("wallpaper-choose", slot), "Choose picture", cx)
            .on_click({
                let (id, screen) = (id.clone(), on.id.clone());
                let named = noted(on);
                cx.listener(move |_, _, window, cx| choose(id.clone(), screen.clone(), named.clone(), window, cx))
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
    let title = noted(on);
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
    // Across all screens, Center and Tile aren't offered (D801).
    let across = on.id == ACROSS;
    let fits = &FITS[..if across { ACROSS_FITS } else { FITS.len() }];
    let choices: Vec<Choice> = fits.iter().map(|(fit, label)| Choice::new(fit_key(*fit), *label)).collect();
    let view = cx.entity();
    let (path, screen) = (picture.path.clone(), on.id.clone());
    let pick = move |key: &str, window: &mut Window, cx: &mut App| {
        let Some((fit, label)) = fits.iter().find(|(fit, _)| fit_key(*fit) == key) else { return };
        let note = fit_note(across, &title, label);
        let (ws, screen, path, fit) = (ws.clone(), screen.clone(), path.clone(), *fit);
        view.update(cx, |panel, cx| set(panel, ws, screen, Some(path), Some(fit), note, window, cx));
    };
    let shown = if across { picture.fit.across() } else { picture.fit };
    let fit = ui::select(("wallpaper-fit", slot), "Fit", choices, &fit_key(shown), FIT_WIDTH, cx, pick);
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
    if screen == ACROSS {
        // A new picture across all screens ends what was said under the screens about the old one.
        let desk = panel.config.desks.get(&desk_sig(panel));
        let set = desk.and_then(|d| match &ws {
            Some(id) => d.workspace_wallpapers.get(id),
            None => Some(&d.wallpaper),
        });
        let own: Vec<String> = set.map(|s| s.screens.keys().cloned().collect()).unwrap_or_default();
        panel.wallpaper.errors.retain(|(w, screen), _| *w != key.0 || own.contains(screen));
    }
    let params = match screen.as_str() {
        ACROSS => json!({ "workspace": ws, "across": true, "path": path, "fit": fit }),
        _ => json!({ "workspace": ws, "screen": screen, "path": path, "fit": fit }),
    };
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
    desk.and_then(|d| d.workspace_wallpapers.get(id)).map(pictures_in).unwrap_or_default()
}

/// A set's pictures by screen, and the one across all screens under `ACROSS`.
fn pictures_in(set: &WallpaperSet) -> Vec<(String, Wallpaper)> {
    let span = set.span.iter().map(|w| (ACROSS.to_string(), w.clone()));
    set.screens.iter().map(|(s, w)| (s.clone(), w.clone())).chain(span).collect()
}

/// The `wallpaper.set` params that give workspace `to` a picture back.
fn put_params(to: &str, screen: &str, w: &Wallpaper) -> serde_json::Value {
    match screen {
        ACROSS => json!({ "workspace": to, "across": true, "path": w.path, "fit": w.fit }),
        _ => json!({ "workspace": to, "screen": screen, "path": w.path, "fit": w.fit }),
    }
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
    let params = put_params(&to, &screen, &w);
    panel.call("wallpaper.set", params, window, cx, move |panel, _, window, cx| copy_next(panel, to, left, window, cx));
}

#[cfg(test)]
mod tests {
    use super::{
        ACROSS, FITS, Fit, Wallpaper, WallpaperSet, across_words, failed_on, fit_key, json, pictures_in, put_params,
        second_line, state_words,
    };

    fn picture(path: &str) -> Wallpaper {
        Wallpaper { path: path.into(), fit: Fit::Fill }
    }

    #[test]
    fn rows_say_what_shows_without_a_picture_and_which_file_with_one() {
        let w = picture(r"D:\Pictures\Lake.jpg");
        assert_eq!(second_line("PG32UCDM", state_words(None, true, false, true, None)), "PG32UCDM. Your own wallpaper");
        assert_eq!(second_line("PG32UCDM", state_words(None, false, false, true, None)), "PG32UCDM. Desk default");
        assert_eq!(state_words(Some(&w), true, false, true, None), "Lake.jpg");
        assert_eq!(state_words(Some(&w), false, true, true, None), "Lake.jpg isn't there any more");
        assert_eq!(state_words(Some(&w), false, false, false, None), "Screen not connected");
        assert_eq!(second_line("", state_words(None, true, false, true, None)), "Your own wallpaper");
    }

    #[test]
    fn rows_for_the_picture_across_all_screens_say_how_it_is_cut() {
        let w = picture(r"D:\Pictures\Wide.jpg");
        assert_eq!(across_words(None, false, true, true), "One picture, cut to where each screen stands");
        assert_eq!(across_words(Some(&w), false, true, true), "Wide.jpg. Cut to where each screen stands on the desk");
        assert_eq!(
            across_words(Some(&w), false, false, true),
            "Wide.jpg. Follows Windows' arrangement, so it may not line up at the edges"
        );
        assert_eq!(across_words(Some(&w), true, true, true), "Wide.jpg isn't there any more");
        assert_eq!(across_words(Some(&w), false, false, false), "Wide.jpg", "one screen has no edges");
        // A screen without a picture of its own shows its part, or the whole picture when it is left out.
        let part = state_words(None, true, false, true, Some(true));
        assert_eq!(second_line("PG32UCDM", part), "PG32UCDM. Part of the picture across all screens");
        let alone = state_words(None, true, false, true, Some(false));
        assert_eq!(
            second_line("PG32UCDM", alone),
            "PG32UCDM. Not next to the other screens, so it shows the whole picture"
        );
        assert_eq!(state_words(Some(&w), true, false, true, Some(true)), "Wide.jpg");
    }

    #[test]
    fn a_set_gives_back_its_span_with_its_screens_and_a_failure_shows_on_the_right_row() {
        let mut set = WallpaperSet::default();
        set.screens.insert("s1".into(), picture("a.jpg"));
        set.span = Some(picture("wide.jpg"));
        let back = pictures_in(&set);
        assert_eq!(back, [("s1".to_string(), picture("a.jpg")), (ACROSS.to_string(), picture("wide.jpg"))]);
        let sent = put_params("w2", ACROSS, &picture("wide.jpg"));
        assert_eq!(sent, json!({ "workspace": "w2", "across": true, "path": "wide.jpg", "fit": "fill" }));
        assert_eq!(put_params("w2", "s1", &picture("a.jpg"))["screen"], "s1");
        assert_eq!(failed_on(&set, "s1", "a.jpg"), Some("s1"));
        assert_eq!(failed_on(&set, "s2", "wide.jpg"), Some(ACROSS));
        assert_eq!(failed_on(&set, "s2", "other.jpg"), None);
    }

    #[test]
    fn fits_are_offered_in_order_with_their_file_names() {
        let keys: Vec<String> = FITS.iter().map(|(fit, _)| fit_key(*fit)).collect();
        assert_eq!(keys, ["fill", "fit", "stretch", "center", "tile"]);
        // Across all screens only the first three.
        assert_eq!(&keys[..super::ACROSS_FITS], ["fill", "fit", "stretch"]);
    }
}
