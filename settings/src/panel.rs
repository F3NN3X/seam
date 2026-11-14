//! The window: the link to the tray, the sidebar, and which page is showing.

use crate::pages::updates::UpdateUi;
use crate::{pages, search, ui};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Theme, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::api::{CallError, Client};
use seam::config::{Config, MAX_EDGE_RESISTANCE_PX, MAX_GLUE_PUSH_PX};
use seam::desk::{self, Mon, Placed};
use seam::layout::Link;
use seam::package::{self, Startup};
use seam::state::State;
use seam::updates::{self, UpdateFile};
use seam::workspace::RowStatus;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod connection;
mod view;

actions!(seam_panel, [FocusSearch, ClearSearch]);

const CONTEXT: &str = "SeamPanel";
/// How long Undo stays offered after a change.
const UNDO_FOR: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Displays,
    Zones,
    Workspaces,
    Shortcuts,
    General,
    Help,
    About,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Displays => "Displays",
            Page::Zones => "Zones",
            Page::Workspaces => "Workspaces",
            Page::Shortcuts => "Shortcuts",
            Page::General => "General",
            Page::Help => "Help",
            Page::About => "About",
        }
    }

    fn icon(self) -> assets::IconName {
        match self {
            Page::Displays => assets::IconName::Monitor,
            Page::Zones => assets::IconName::LayoutGrid,
            Page::Workspaces => assets::IconName::Layers,
            Page::Shortcuts => assets::IconName::Keyboard,
            Page::General => assets::IconName::SlidersHorizontal,
            Page::Help => assets::IconName::CircleQuestionMark,
            Page::About => assets::IconName::Info,
        }
    }

    pub fn from_arg(arg: &str) -> Option<Page> {
        Page::top_group().into_iter().chain(Page::app_group()).find(|p| p.title().eq_ignore_ascii_case(arg))
    }

    fn top_group() -> Vec<Page> {
        vec![Page::Displays, Page::Zones, Page::Workspaces]
    }

    fn app_group() -> Vec<Page> {
        vec![Page::Shortcuts, Page::General, Page::Help, Page::About]
    }
}

fn last_page_file() -> PathBuf {
    Config::path().with_file_name("panel-page.txt")
}

pub struct Panel {
    pub page: Page,
    /// Requests go here. `None` while the tray isn't reachable.
    pub client: Option<Arc<Mutex<Client>>>,
    pub token: String,
    pub state: Option<State>,
    pub config: Config,
    pub mons: Vec<Mon>,
    pub links: Vec<Link>,
    pub layout: Vec<Placed>,
    /// The layout the Displays map shows. It is the panel's memory only, not a setting.
    pub layout_view: seam::api::LayoutView,
    pub ease_on: bool,
    /// The screen whose zones the Zones page shows, as an index into `mons`.
    pub zone_screen: usize,
    /// "Show all screens" under Your layouts: every custom layout, not only those for
    /// the screen. Kept while the panel is open, not saved.
    pub all_layouts: bool,
    /// The zone editor's unsaved copy of a layout, while it's open.
    pub editor: Option<pages::zone_editor::Editor>,
    /// The editor's window on the real screen, while it's open.
    pub screen_window: Option<pages::screen_editor::Shown>,
    /// The workspace the Workspaces page shows, by id. `None` shows the first.
    pub workspace: Option<String>,
    /// App icons for the workspace list, loaded as they are first needed.
    pub icons: crate::icons::Icons,
    /// The name field is open on the selected workspace.
    pub naming: bool,
    /// The name field should take focus when it next draws.
    pub focus_name: bool,
    pub name_input: Entity<InputState>,
    /// The workspace editor's open row, delay slider and app picker.
    pub editing: pages::workspace_editor::Editing,
    /// How the shown workspace's apps are doing, by workspace id, and the id being asked about.
    pub status: Option<(String, Vec<RowStatus>)>,
    pub asking_status: Option<String>,
    status_again: bool,
    /// The topic the Help page shows, as an index into `help::TOPICS`.
    pub help_topic: usize,
    /// The pickers behind the Custom swatch of the focus and pin colour rows, and a
    /// counter so only the last colour picked in a burst is sent.
    pub focus_picker: Entity<ColorPickerState>,
    pub pin_picker: Entity<ColorPickerState>,
    pub colour_generation: u64,
    /// The Gap slider. It sends the new gap when it's let go.
    pub gap: Entity<SliderState>,
    /// The Edge resistance and Glued edge push sliders, sent the same way.
    pub edge_resistance: Entity<SliderState>,
    pub glue_push: Entity<SliderState>,
    pub search: Entity<InputState>,
    /// The page's scroll, which opening a search result moves to the row it found.
    page_scroll: ScrollHandle,
    /// Holds focus when no control does, so Tab and Ctrl+F always reach a binding.
    focus: FocusHandle,
    pub query: SharedString,
    /// The shortcut being recorded, and what stops recording it.
    pub recording: Option<String>,
    pub recorder: Option<Subscription>,
    /// A shortcut that works but types characters on some keyboards.
    pub hotkey_warning: Option<String>,
    pub toast_generation: u64,
    /// What the updater left in its state file, read when the tray says it changed.
    pub update_file: UpdateFile,
    pub update_ui: UpdateUi,
    /// The Store copy's "Start with Windows", which Windows keeps. `None` until it answers.
    pub startup: Option<Startup>,
    /// A GitHub copy is still installed next to this Store copy.
    pub dual_install: bool,
    /// Settings from an earlier GitHub install, offered once.
    pub import_offer: Option<Config>,
    /// The tray was reachable earlier, so a reconnect means a new tray.
    was_linked: bool,
    _link: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    pub fn new(page: Option<Page>, token: String, window: &mut Window, cx: &mut Context<Self>) -> Panel {
        let saved = std::fs::read_to_string(last_page_file()).ok();
        let page = page.or(saved.and_then(|s| Page::from_arg(s.trim()))).unwrap_or(Page::Displays);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
        let focus_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let pin_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let gap =
            cx.new(|_| SliderState::new().min(0.).max(32.).step(2.).default_value(Config::default().zones.gap as f32));
        let pointer = Config::default().pointer;
        let edge_resistance = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(MAX_EDGE_RESISTANCE_PX as f32)
                .step(2.)
                .default_value(pointer.edge_resistance_px as f32)
        });
        let glue_push = cx.new(|_| {
            SliderState::new().min(0.).max(MAX_GLUE_PUSH_PX as f32).step(10.).default_value(pointer.glue_push_px as f32)
        });
        let subscriptions = vec![
            cx.subscribe_in(&edge_resistance, window, |this, _, event: &SliderEvent, window, cx| match event {
                SliderEvent::Change(value) => {
                    this.config.pointer.edge_resistance_px = value.end() as u32;
                    cx.notify();
                }
                SliderEvent::Release(value) => {
                    this.patch(json!({ "pointer": { "edge_resistance_px": value.end() as u32 } }), window, cx);
                }
            }),
            cx.subscribe_in(&glue_push, window, |this, _, event: &SliderEvent, window, cx| match event {
                SliderEvent::Change(value) => {
                    this.config.pointer.glue_push_px = value.end() as u32;
                    cx.notify();
                }
                SliderEvent::Release(value) => {
                    this.patch(json!({ "pointer": { "glue_push_px": value.end() as u32 } }), window, cx);
                }
            }),
            cx.subscribe_in(&gap, window, |this, _, event: &SliderEvent, window, cx| match event {
                SliderEvent::Change(value) => {
                    this.config.zones.gap = value.end() as u32;
                    cx.notify();
                }
                SliderEvent::Release(value) => {
                    this.patch(json!({ "zones": { "gap": value.end() as u32 } }), window, cx);
                }
            }),
            cx.subscribe_in(&name_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    pages::workspaces::finish_naming(this, window, cx);
                }
            }),
            cx.subscribe_in(&focus_picker, window, |this, _, event: &ColorPickerEvent, window, cx| {
                let ColorPickerEvent::Change(colour) = event;
                pages::zones::picked("focus_color", *colour, this, window, cx);
            }),
            cx.subscribe_in(&pin_picker, window, |this, _, event: &ColorPickerEvent, window, cx| {
                let ColorPickerEvent::Change(colour) = event;
                pages::zones::picked("pin_color", *colour, this, window, cx);
            }),
            cx.subscribe_in(&search, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = input.read(cx).value();
                    cx.notify();
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx)),
            // The Ease setting has no change event, so look again on coming back.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.ease_on = seam::ease::is_on();
                    if this.screen_window.is_some() {
                        pages::screen_editor::restack();
                    }
                    cx.notify();
                }
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // The editor's window is the panel's, so it goes with it.
        let release = cx.on_release(|panel, cx| pages::screen_editor::close_all(panel.screen_window.as_ref(), cx));
        let (editing, editing_subscriptions) = pages::workspace_editor::Editing::new(window, cx);
        let mut subscriptions = subscriptions;
        subscriptions.push(release);
        subscriptions.extend(editing_subscriptions);
        let mut panel = Panel {
            page,
            client: None,
            token,
            state: None,
            config: Config::default(),
            mons: Vec::new(),
            links: Vec::new(),
            layout: Vec::new(),
            layout_view: seam::api::LayoutView::Seam,
            ease_on: seam::ease::is_on(),
            zone_screen: 0,
            all_layouts: false,
            editor: None,
            screen_window: None,
            workspace: None,
            icons: Default::default(),
            naming: false,
            focus_name: false,
            name_input,
            editing,
            status: None,
            asking_status: None,
            status_again: false,
            help_topic: 0,
            focus_picker,
            pin_picker,
            colour_generation: 0,
            gap,
            edge_resistance,
            glue_push,
            search,
            page_scroll: ScrollHandle::new(),
            focus,
            query: SharedString::default(),
            recording: None,
            recorder: None,
            hotkey_warning: None,
            toast_generation: 0,
            update_file: updates::read(),
            update_ui: UpdateUi::Idle,
            startup: None,
            dual_install: package::is_packaged() && package::unpackaged_copy_installed(),
            import_offer: package::take_import_offer(),
            was_linked: false,
            _link: Task::ready(()),
            _subscriptions: subscriptions,
        };
        panel.read_screens();
        panel._link = panel.link(window, cx);
        if package::is_packaged() {
            panel.refresh_startup(cx);
        }
        panel
    }
}

/// One line for a refused command, saying what happened and why.
pub fn refusal(title: &str, reason: &str) -> Notification {
    struct Feedback;
    Notification::new().id::<Feedback>().title(title.to_string()).message(reason.to_string())
}

pub fn error_text(e: &CallError) -> String {
    match e {
        CallError::Rpc(e) => e.message.clone(),
        CallError::Io(_) => "Seam stopped answering".to_string(),
    }
}
