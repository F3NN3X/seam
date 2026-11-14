//! The workspace editor: the selected workspace's apps in start order, where each
//! one goes, how it shows and how it starts. Every change is saved at once through
//! `workspace.update`, and the last one can be undone for 8 s.

use crate::icons::{self, Icons};
use crate::pages::app_picker::Picker;
use crate::pages::workspaces::{app_title, desk_sig};
use crate::panel::Panel;
use crate::theme::{Palette, palette};
use crate::ui::{self, row, switch};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Icon, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::state::Screen;
use seam::workspace::{self, RowState, Rule, Shown, Target, WinState, Workspace};
use seam::zones;

mod edits;
mod screen_zones;
mod table;

pub use edits::{add_app, update};
use edits::{exe_name, move_row, never_include, remove_row, seconds, set_delay, update_row, where_it_is};
pub use screen_zones::{zones_moved, zones_section};
pub use table::rows;

const GRIP_WIDTH: f32 = 16.;
const ICON_WIDTH: f32 = 32.;
/// The name, Now and Goes to columns start at the width their usual text needs (see
/// `Cols`), and share both the room to spare and the room missing, by these weights,
/// each down to its least, so the "..." always shows.
const NAME_GROW: f32 = 2.;
const NAME_LEAST: f32 = 40.;
const STATUS_GROW: f32 = 1.;
const STATUS_LEAST: f32 = 16.;
const GOES_TO_GROW: f32 = 2.;
const GOES_TO_LEAST: f32 = 48.;
const MORE_WIDTH: f32 = 32.;
/// What the columns are measured on: a usual app name, the longest Now word that
/// should show in full, and a usual place.
const USUAL_NAME: &str = "Visual Studio Code";
const LONGEST_STATUS: &str = "Running in background";
const USUAL_PLACE: &str = "Screen 2, zone 1";
/// Around the text: the name's gap and chevron, the status dot and its gap, and the
/// Goes to button's border, padding, gap and caret.
const NAME_CHROME: f32 = 4. + 16.;
const STATUS_CHROME: f32 = 8. + 8.;
const GOES_TO_CHROME: f32 = 2. + 20. + 8. + 16.;

/// The table's measured widths: where the flexible columns start, and Launch, as wide as its head.
#[derive(Clone, Copy)]
pub struct Cols {
    name: f32,
    status: f32,
    goes: f32,
    launch: f32,
}

impl Cols {
    pub fn measure(window: &Window, cx: &App) -> Cols {
        let width = |text: &str, size: f32| text_width(text, size, window, cx).ceil();
        Cols {
            name: width(USUAL_NAME, 14.) + NAME_CHROME,
            status: width(LONGEST_STATUS, 12.) + STATUS_CHROME,
            goes: width(USUAL_PLACE, 14.) + GOES_TO_CHROME,
            launch: width("Launch", 13.).max(16.),
        }
    }
}

/// How wide `text` is at `size` in the panel's font.
fn text_width(text: &str, size: f32, window: &Window, cx: &App) -> f32 {
    let mut style = window.text_style();
    style.font_family = cx.theme().font_family.clone();
    let run = style.to_run(text.len());
    let line = window.text_system().shape_line(SharedString::from(text.to_string()), px(size), &[run], None);
    f32::from(line.width)
}
/// Launch details sit under the app's name: past the grip, the icon and their gaps.
const DETAILS_INDENT: f32 = GRIP_WIDTH + ICON_WIDTH + 16.;
/// A column that starts at `width`, grows by `grow` and shrinks down to `least`.
fn column(width: f32, grow: f32, least: f32) -> Div {
    div().flex_grow(grow).flex_shrink(1.).flex_basis(px(width)).min_w(px(least))
}

const SHOW_AS: [(&str, Shown); 4] = [
    ("Normal", Shown::Normal),
    ("Minimized", Shown::Minimized),
    ("Maximized", Shown::Maximized),
    ("Full screen", Shown::Fullscreen),
];

/// What the editor keeps between frames.
pub struct Editing {
    /// The row whose launch details are open, by workspace id and row.
    pub expanded: Option<(String, usize)>,
    /// The delay slider of the open row, and whether it's being held.
    pub delay: Entity<SliderState>,
    holding_delay: bool,
    pub picker: Picker,
    /// The workspace whose Zones section is open. Closed until opened.
    zones_open: Option<String>,
}

impl Editing {
    pub fn new(window: &mut Window, cx: &mut Context<Panel>) -> (Editing, Vec<Subscription>) {
        let delay = cx.new(|_| SliderState::new().min(0.).max(30.).step(0.5).default_value(0.));
        let (picker, picked) = Picker::new(window, cx);
        let held = cx.subscribe_in(&delay, window, |panel, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(_) => {
                panel.editing.holding_delay = true;
                cx.notify();
            }
            SliderEvent::Release(value) => {
                panel.editing.holding_delay = false;
                set_delay(panel, (value.end() * 1000.).round() as u32, window, cx);
            }
        });
        (Editing { expanded: None, delay, holding_delay: false, picker, zones_open: None }, vec![held, picked])
    }
}

// ---- What is drawn

/// A place a window can be sent to, as the Goes to menu lists it.
#[derive(Clone)]
pub struct Place {
    pub label: String,
    /// The screen it's on. `None` leaves a window's screen as it was.
    pub monitor: Option<String>,
    pub target: Target,
}

pub fn places(panel: &Panel) -> Vec<Place> {
    let keep =
        Place { label: "Keep where it is".into(), monitor: None, target: Target::State { state: WinState::Keep } };
    let mut places = vec![keep];
    let sig = desk_sig(panel);
    let desk = panel.config.desks.get(&sig);
    for s in panel.state.iter().flat_map(|s| s.screens.iter()) {
        let layout = desk.and_then(|d| d.zones.get(&s.id)).and_then(zones::layout_of);
        let count = layout.map_or(0, |l| zones::leaf_count(&l.tree));
        for zone in 1..=count as u32 {
            let target = Target::Zone { zone, span_to: None };
            places.push(Place {
                label: format!("Screen {}, zone {zone}", s.number),
                monitor: Some(s.id.clone()),
                target,
            });
        }
        let full = Target::State { state: WinState::Max };
        places.push(Place { label: format!("Screen {}, full", s.number), monitor: Some(s.id.clone()), target: full });
    }
    places
}

/// Where a rule sends its window, in the words of the menu.
pub fn goes_to(rule: &Rule, screens: &[Screen]) -> String {
    let number = screens.iter().find(|s| s.id == rule.monitor).map(|s| s.number);
    match (&rule.target, number) {
        (Target::State { state: WinState::Keep }, _) => "Keep where it is".into(),
        (_, None) => "Screen not connected".into(),
        (Target::Zone { zone, span_to: None }, Some(n)) => format!("Screen {n}, zone {zone}"),
        (Target::Zone { zone, span_to: Some(to) }, Some(n)) => format!("Screen {n}, zones {zone} to {to}"),
        (Target::State { .. }, Some(n)) => format!("Screen {n}, full"),
        (Target::Rect { .. }, Some(n)) => format!("Screen {n}, own size"),
    }
}

pub fn exe_line(rule: &Rule) -> String {
    let lower = rule.exe.to_lowercase();
    if rule.aumid.is_some() && (lower.is_empty() || lower.contains(r"\windowsapps\")) {
        return "Packaged app".into();
    }
    rule.exe.rsplit('\\').next().unwrap_or(&rule.exe).to_string()
}

/// The app's icon, or its first letter while there is none.
pub fn app_icon(rule: &Rule, icons: &Icons, p: Palette) -> Div {
    let tile =
        div().flex().flex_none().items_center().justify_center().size(px(ICON_WIDTH)).rounded(px(8.)).bg(p.raised);
    match icons.get(rule) {
        Some(image) => tile.child(img(image).size(px(16.))),
        None => {
            let letter = app_title(rule).chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            tile.text_size(px(14.)).text_color(p.secondary).child(letter)
        }
    }
}

/// What the tray last said about this row's app, while it's about this workspace.
pub fn row_state(panel: &Panel, ws: &Workspace, i: usize) -> Option<RowState> {
    let (id, rows) = panel.status.as_ref()?;
    let row = rows.iter().find(|r| r.rule_index == i).filter(|_| *id == ws.id)?;
    Some(row.state)
}
