//! Zones: a layout for each screen, and how dragging a window snaps to it.

use crate::pages::zone_editor::{self, Opened};
use crate::panel::{Panel, error_text, refusal};
use crate::theme::palette;
use crate::theme::tokens::{self, text_style};
use crate::ui::{self, card, row};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::desk::Mon;
use seam::library;
use seam::state;
use seam::zones::{self, Node, Rect, Template, ZoneLayout};
use serde_json::{Value, json};
use std::rc::Rc;

mod layouts;
pub mod margins;
mod page;
mod picker;
mod screen_preview;
mod snapping;

use layouts::{for_screens, save_as_layout, screen_names, tile};
pub use page::render;
use screen_preview::{preview, preview_height};
use snapping::snapping_section;

/// Config value and label of each key a drag can use.
const KEYS: [(&str, &str); 3] = [("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];
const SPAN_ORDER: [usize; 3] = [1, 0, 2];
/// A tile's width and the size of its drawing: your layouts, and the narrower built-in ones.
const TILE: (f32, (f32, f32)) = (104., (86., 44.));
const BUILT_IN_TILE: (f32, (f32, f32)) = (88., (70., 44.));
const TILE_GAP: f32 = 8.;
/// The preview is as tall as the screen's shape asks, up to this.
const PREVIEW_MAX_HEIGHT: f32 = 320.;
/// The preview card's padding and border around the drawn screen.
const PREVIEW_INSET: f64 = 34.;

fn key_label(key: &str) -> &'static str {
    KEYS.iter().find(|(k, _)| *k == key).map_or("Shift", |(_, label)| label)
}

/// A tile in the layout picker.
#[derive(Clone)]
enum Pick {
    Template(Template),
    Library(library::Entry),
    /// The screen's own layout, which is no template's or library layout's.
    Changed(Node),
    New,
}

impl Pick {
    fn key(&self) -> String {
        match self {
            Pick::Template(t) => format!("zone-tile-{}", t.title()),
            Pick::Library(entry) => format!("zone-tile-library-{}", entry.id),
            Pick::Changed(_) => "zone-tile-changed".into(),
            Pick::New => "zone-tile-new".into(),
        }
    }

    fn title(&self) -> SharedString {
        match self {
            Pick::Template(t) => t.title().into(),
            Pick::Library(entry) => entry.name.clone().into(),
            Pick::Changed(_) => "Changed".into(),
            Pick::New => "New layout".into(),
        }
    }

    /// The tree the tile draws.
    fn art(&self) -> Option<Node> {
        match self {
            Pick::Template(t) => zones::template_tree(*t),
            Pick::Library(entry) => Some(entry.tree.clone()),
            Pick::Changed(tree) => Some(tree.clone()),
            Pick::New => None,
        }
    }

    fn built_in(&self) -> bool {
        matches!(self, Pick::Template(_))
    }

    /// Whether this is the layout the screen has now.
    fn selected(&self, saved: Option<&ZoneLayout>) -> bool {
        match self {
            Pick::Template(t) => saved.map_or(Template::None, |l| l.template) == *t,
            Pick::Library(entry) => saved.is_some_and(|l| l.custom.as_ref() == Some(&entry.id)),
            Pick::Changed(_) => true,
            Pick::New => false,
        }
    }
}

/// The screen the page is showing, with the layout saved for it on this desk.
pub fn chosen(panel: &Panel) -> Option<(&Mon, Option<ZoneLayout>)> {
    let mon = panel.mons.get(panel.zone_screen.min(panel.mons.len().checked_sub(1)?))?;
    let sig = panel.state.as_ref().map(|s| s.desk.sig.as_str())?;
    let saved = panel.config.desks.get(sig).and_then(|d| d.zones.get(&mon.id)).and_then(zones::layout_of);
    Some((mon, saved))
}

/// Give the chosen screen a layout, or no zones. It shows at once, and the tray
/// saves it and moves the screen's snapped windows into the new zones, with Undo.
fn set_layout(panel: &mut Panel, layout: Option<ZoneLayout>, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(sig) = panel.state.as_ref().map(|s| s.desk.sig.clone()) else { return };
    let Some((mon, _)) = chosen(panel) else { return };
    let number = panel.zone_screen.min(panel.mons.len() - 1) + 1;
    // No entry means no zones, and a patch's null removes the entry.
    let shown = layout.as_ref().and_then(library::screen_patch).unwrap_or(Value::Null);
    if let Ok(config) = panel.config.patched(&json!({ "desks": { sig: { "zones": { mon.id.clone(): shown } } } })) {
        panel.set_config(config);
    }
    cx.notify();
    let sent = layout.and_then(|l| serde_json::to_value(l).ok()).unwrap_or(Value::Null);
    panel.call("zones.pick", json!({ "number": number, "layout": sent }), window, cx, |panel, result, window, cx| {
        if let Err(e) = result {
            window.push_notification(refusal("Layout left as it was", &error_text(&e)), cx);
        }
        panel.reload_config(window, cx);
    });
}
