//! The window: the link to the tray, the sidebar, and which page is showing.

use crate::pages::updates::UpdateUi;
use crate::{chrome, pages, search, ui};
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, WindowExt};
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
use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod command_bar;
mod connection;
mod new_screen;
mod regions;
mod setup;
mod view;

actions!(seam_panel, [FocusSearch, ClearSearch, GoBack, NextRegion, PreviousRegion]);

const CONTEXT: &str = "SeamPanel";
/// How long Undo stays offered after a change.
const UNDO_FOR: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Displays,
    Zones,
    Windows,
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
            Page::Windows => "Windows",
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
            Page::Windows => assets::IconName::AppWindow,
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
        vec![Page::Displays, Page::Zones, Page::Windows, Page::Workspaces]
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
    /// The border the Displays map and its strip show, by the numbers of its two screens.
    pub border_pick: Option<(u32, u32)>,
    /// The "More ... options" rows that are open, by title. The panel's memory only.
    pub disclosed: HashSet<&'static str>,
    /// The screen whose zones the Zones page shows, as an index into `mons`.
    pub zone_screen: usize,
    /// The Where apps open rule whose details are open, by the app's identity.
    pub rule_open: Option<String>,
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
    /// The topic the Help page shows, as an index into `help::TOPICS`. `None` shows the topic grid.
    pub help_topic: Option<usize>,
    /// The pickers behind the Custom swatch of the focus and pin colour rows, and a
    /// counter so only the last colour picked in a burst is sent.
    pub focus_picker: Entity<ColorPickerState>,
    pub pin_picker: Entity<ColorPickerState>,
    pub colour_generation: u64,
    /// The Gap slider. It sends the new gap when it's let go.
    pub gap: Entity<SliderState>,
    /// The Screen margins sliders, top, right, bottom and left, sent the same way.
    pub margins: [Entity<SliderState>; 4],
    /// The Edge resistance and Glued edge push sliders, sent the same way.
    pub edge_resistance: Entity<SliderState>,
    pub glue_push: Entity<SliderState>,
    pub search: Entity<InputState>,
    /// The command bar's results list, driven from the search field.
    command_list: Entity<CommandState>,
    /// The result Enter would run. `None`, or one a newer query dropped, is the first.
    selected: Option<command_bar::Key>,
    /// What the query found, kept until the query, the config or the state changes.
    found: Option<Rc<command_bar::Results>>,
    /// Counts changes to the config and the state, for what is worked out from them.
    pub revision: u64,
    /// The rows "Show changed settings" keeps, for the page and revision they were worked out for.
    changed: Option<(Page, u64, Rc<HashSet<&'static str>>)>,
    /// Where focus was when Ctrl+F took it, so Esc can give it back.
    return_focus: Option<FocusHandle>,
    /// Where the search field was last drawn, for the results to sit under.
    field_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// "Show changed settings": the page shows only the rows that differ from their default.
    changed_only: bool,
    /// The page's scroll, which opening a search result moves to the row it found.
    page_scroll: ScrollHandle,
    /// Holds focus when no control does, so Tab and Ctrl+F always reach a binding.
    focus: FocusHandle,
    /// The page and the sidebar, so F6 knows where focus is and where to put it.
    page_focus: FocusHandle,
    sidebar_focus: FocusHandle,
    /// The sidebar's items, which keep their focus between frames.
    nav_focus: Vec<(Page, FocusHandle)>,
    pub query: SharedString,
    /// The shortcut being recorded, and what stops recording it.
    pub recording: Option<String>,
    pub recorder: Option<Subscription>,
    /// Keys pressed while recording that something already holds, and what holds them. Not saved.
    pub clash: Option<pages::shortcuts::Clash>,
    /// The Shortcuts page lists every shortcut, not only those with keys.
    pub all_shortcuts: bool,
    /// The Shortcuts page's filter.
    pub shortcut_filter: Entity<InputState>,
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
    /// First-run setup, while its dialog is the one the window opened.
    setup: Option<Rc<setup::Setup>>,
    _link: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    /// Every write to the config goes through here, so what is worked out from it is found again.
    pub fn config_mut(&mut self) -> &mut Config {
        self.revision += 1;
        &mut self.config
    }

    pub fn set_config(&mut self, config: Config) {
        *self.config_mut() = config;
    }

    /// The same for the state.
    pub fn set_state(&mut self, state: Option<State>) {
        self.revision += 1;
        self.state = state;
    }

    pub fn new(page: Option<Page>, token: String, window: &mut Window, cx: &mut Context<Self>) -> Panel {
        let saved = std::fs::read_to_string(last_page_file()).ok();
        let page = page.or(saved.and_then(|s| Page::from_arg(s.trim()))).unwrap_or(Page::Displays);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings and actions"));
        let command_list = cx.new(|cx| CommandState::new(window, cx));
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
        let shortcut_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Find a shortcut or a key"));
        let focus_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let pin_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let gap =
            cx.new(|_| SliderState::new().min(0.).max(32.).step(2.).default_value(Config::default().zones.gap as f32));
        let margins = std::array::from_fn(|_| {
            cx.new(|_| SliderState::new().min(0.).max(seam::margins::MAX_PX as f32).step(4.).default_value(0.))
        });
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
        let mut subscriptions = slider_subscriptions(&edge_resistance, &glue_push, &gap, window, cx);
        subscriptions.extend(pages::zones::margins::subscriptions(&margins, window, cx));
        subscriptions.extend(field_subscriptions(
            &name_input,
            &focus_picker,
            &pin_picker,
            &search,
            &shortcut_filter,
            window,
            cx,
        ));
        subscriptions.extend(window_subscriptions(window, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let nav_focus =
            Page::top_group().into_iter().chain(Page::app_group()).map(|p| (p, cx.focus_handle())).collect();
        // The editor's window is the panel's, so it goes with it.
        let release = cx.on_release(|panel, cx| pages::screen_editor::close_all(panel.screen_window.as_ref(), cx));
        let (editing, editing_subscriptions) = pages::workspace_editor::Editing::new(window, cx);
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
            border_pick: None,
            disclosed: HashSet::new(),
            zone_screen: 0,
            rule_open: None,
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
            help_topic: None,
            focus_picker,
            pin_picker,
            colour_generation: 0,
            gap,
            margins,
            edge_resistance,
            glue_push,
            search,
            command_list,
            selected: None,
            found: None,
            revision: 0,
            changed: None,
            return_focus: None,
            field_bounds: Rc::default(),
            changed_only: false,
            page_scroll: ScrollHandle::new(),
            focus,
            page_focus: cx.focus_handle().tab_stop(false),
            sidebar_focus: cx.focus_handle(),
            nav_focus,
            query: SharedString::default(),
            recording: None,
            recorder: None,
            clash: None,
            all_shortcuts: false,
            shortcut_filter,
            hotkey_warning: None,
            toast_generation: 0,
            update_file: updates::read(),
            update_ui: UpdateUi::Idle,
            startup: None,
            dual_install: package::is_packaged() && package::unpackaged_copy_installed(),
            import_offer: package::take_import_offer(),
            was_linked: false,
            setup: None,
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

/// The pointer and gap sliders: they show the value as it moves and send it when let go.
fn slider_subscriptions(
    edge_resistance: &Entity<SliderState>,
    glue_push: &Entity<SliderState>,
    gap: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe_in(edge_resistance, window, |this, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(value) => {
                this.config_mut().pointer.edge_resistance_px = value.end() as u32;
                cx.notify();
            }
            SliderEvent::Release(value) => {
                this.patch(json!({ "pointer": { "edge_resistance_px": value.end() as u32 } }), window, cx);
            }
        }),
        cx.subscribe_in(glue_push, window, |this, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(value) => {
                this.config_mut().pointer.glue_push_px = value.end() as u32;
                cx.notify();
            }
            SliderEvent::Release(value) => {
                this.patch(json!({ "pointer": { "glue_push_px": value.end() as u32 } }), window, cx);
            }
        }),
        cx.subscribe_in(gap, window, |this, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(value) => {
                this.config_mut().zones.gap = value.end() as u32;
                cx.notify();
            }
            SliderEvent::Release(value) => {
                this.patch(json!({ "zones": { "gap": value.end() as u32 } }), window, cx);
            }
        }),
    ]
}

/// The name, colour and search fields.
fn field_subscriptions(
    name_input: &Entity<InputState>,
    focus_picker: &Entity<ColorPickerState>,
    pin_picker: &Entity<ColorPickerState>,
    search: &Entity<InputState>,
    shortcut_filter: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe_in(shortcut_filter, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }),
        cx.subscribe_in(name_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                pages::workspaces::finish_naming(this, window, cx);
            }
        }),
        cx.subscribe_in(focus_picker, window, |this, _, event: &ColorPickerEvent, window, cx| {
            let ColorPickerEvent::Change(colour) = event;
            pages::windows::picked("focus_color", *colour, this, window, cx);
        }),
        cx.subscribe_in(pin_picker, window, |this, _, event: &ColorPickerEvent, window, cx| {
            let ColorPickerEvent::Change(colour) = event;
            pages::windows::picked("pin_color", *colour, this, window, cx);
        }),
        cx.subscribe_in(search, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                this.query = input.read(cx).value();
                this.selected = None;
                // A new query hands the highlight back to the first result.
                let query = this.query.clone();
                this.command_list.update(cx, |list, cx| list.set_query(query, window, cx));
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.run_selected(window, cx),
            _ => {}
        }),
    ]
}

/// The theme and backdrop follow Windows, and the Ease setting is looked at again on coming back.
fn window_subscriptions(window: &mut Window, cx: &mut Context<Panel>) -> Vec<Subscription> {
    vec![
        cx.observe_window_appearance(window, |_, window, cx| chrome::follow_system(window, cx)),
        // The Ease setting has no change event, so look again on coming back.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                chrome::recheck(window, cx);
                this.ease_on = seam::ease::is_on();
                if this.screen_window.is_some() {
                    pages::screen_editor::restack();
                }
                cx.notify();
            }
        }),
    ]
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
