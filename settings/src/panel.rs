//! The window: the link to the tray, the sidebar, and which page is showing.

use crate::pages::updates::UpdateUi;
use crate::theme::palette;
use crate::{pages, search, ui};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Icon, Theme, ThemeStyled, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seam::api::{CallError, Client, Event};
use seam::config::{diff_patch, Config};
use seam::desk::{self, Mon, Placed};
use seam::layout::Link;
use seam::monitors;
use seam::package::{self, Startup};
use seam::state::{Remap, State};
use seam::updates::{self, UpdateFile};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
            Page::About => assets::IconName::Info,
        }
    }

    pub fn from_arg(arg: &str) -> Option<Page> {
        [Page::Displays, Page::Zones, Page::Workspaces, Page::Shortcuts, Page::General, Page::About]
            .into_iter()
            .find(|p| p.title().eq_ignore_ascii_case(arg))
    }

    fn top_group() -> Vec<Page> {
        vec![Page::Displays, Page::Zones, Page::Workspaces]
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
    pub ease_on: bool,
    /// The screen whose zones the Zones page shows, as an index into `mons`.
    pub zone_screen: usize,
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
    /// The pickers behind the Custom swatch of the focus and pin colour rows, and a
    /// counter so only the last colour picked in a burst is sent.
    pub focus_picker: Entity<ColorPickerState>,
    pub pin_picker: Entity<ColorPickerState>,
    pub colour_generation: u64,
    /// The Gap slider. It sends the new gap when it's let go.
    pub gap: Entity<SliderState>,
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
        let subscriptions = vec![
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
            ease_on: seam::ease::is_on(),
            zone_screen: 0,
            editor: None,
            screen_window: None,
            workspace: None,
            icons: Default::default(),
            naming: false,
            focus_name: false,
            name_input,
            editing,
            focus_picker,
            pin_picker,
            colour_generation: 0,
            gap,
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
        panel._link = panel.link(cx);
        if package::is_packaged() {
            panel.refresh_startup(cx);
        }
        panel
    }

    /// Ask Windows about the startup task. It blocks, so it runs off the UI thread.
    fn refresh_startup(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx.background_executor().spawn(async { package::startup_state() }).await;
            let _ = this.update(cx, |this, cx| {
                this.startup = state;
                cx.notify();
            });
        })
        .detach();
    }

    /// Turn the Store copy's startup task on or off.
    pub fn set_startup(&self, on: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx.background_executor().spawn(async move { package::set_startup(on) }).await;
            let _ = this.update(cx, |this, cx| {
                this.startup = state.or(this.startup);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn bind_keys(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("ctrl-f", FocusSearch, Some(CONTEXT)),
            KeyBinding::new("escape", ClearSearch, Some(CONTEXT)),
        ]);
        pages::zone_editor::bind_keys(cx);
    }

    /// Stay connected, and redraw from events. There's no polling.
    fn link(&self, cx: &mut Context<Self>) -> Task<()> {
        let token = self.token.clone();
        cx.spawn(async move |this, cx| loop {
            let session = {
                let token = token.clone();
                cx.background_executor().spawn(async move { connect(&token) }).await
            };
            let Ok((client, events, state, config)) = session else {
                let linked = this.update(cx, |this, cx| {
                    this.client = None;
                    this.state = None;
                    cx.notify();
                });
                if linked.is_err() {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
                continue;
            };
            let client = Arc::new(Mutex::new(client));
            let accepted = this.update(cx, |this, cx| {
                // A new tray has a new token, so this panel can't change settings.
                // Ask for a fresh one.
                if this.was_linked {
                    crate::restart_panel(cx);
                    return false;
                }
                this.was_linked = true;
                this.client = Some(client.clone());
                this.apply(state, config);
                cx.notify();
                true
            });
            if !matches!(accepted, Ok(true)) {
                return;
            }
            let events = Arc::new(Mutex::new(events));
            loop {
                let next = {
                    let events = events.clone();
                    cx.background_executor().spawn(async move { next_event(&events) }).await
                };
                let Some(event) = next else { break };
                let fresh = {
                    let client = client.clone();
                    cx.background_executor()
                        .spawn(async move { client.lock().ok()?.call("config.get", json!({})).ok() })
                        .await
                };
                let updated = this.update(cx, |this, cx| {
                    if event.kind == "state.changed" {
                        if let (Ok(state), Some(config)) = (serde_json::from_value(event.data), fresh) {
                            this.apply(state, config);
                        }
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
        })
    }

    fn apply(&mut self, state: State, config: Value) {
        let screens_changed = self.state.as_ref().map(|s| &s.desk.sig) != Some(&state.desk.sig);
        if let Ok(config) = serde_json::from_value(config) {
            self.config = config;
        }
        // The tray reads the updater's file and says so in the state.
        if self.state.as_ref().map(|s| &s.update) != Some(&state.update) {
            self.update_file = updates::read();
        }
        self.state = Some(state);
        // A screen that came, went or changed size leaves the editor's tree meaning
        // something else, so it closes and nothing is saved.
        if self.editor.is_some() && !screens_changed {
            let now = monitors::enumerate();
            let same = now.len() == self.mons.len()
                && now.iter().zip(&self.mons).all(|(a, b)| a.id == b.id && a.rect == b.rect);
            if !same {
                self.editor = None;
            }
        }
        if screens_changed {
            self.editor = None;
            self.read_screens();
        }
        self.relink();
    }

    fn read_screens(&mut self) {
        self.mons = monitors::enumerate();
    }

    /// The tray's answer to what's lined up, worked out here for the map.
    fn relink(&mut self) {
        let sig = self.state.as_ref().map(|s| s.desk.sig.clone()).unwrap_or_default();
        let saved = self.config.desks.get(&sig).map(|d| d.borders.as_slice());
        self.links = desk::resolve(&self.mons, saved);
        self.layout = desk::physical_layout(&self.mons, &self.links);
    }

    pub fn desk_name(&self) -> String {
        self.state.as_ref().map(|s| s.desk.name.clone()).unwrap_or_default()
    }

    /// Call the tray off the UI thread, then carry on here with the answer.
    pub fn call(
        &self,
        method: &'static str,
        params: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Panel, Result<Value, CallError>, &mut Window, &mut Context<Panel>) + 'static,
    ) {
        let Some(client) = self.client.clone() else {
            window.push_notification(refusal("Seam isn't running", "Start it from the sidebar, then try again"), cx);
            return;
        };
        let answer = cx.background_executor().spawn(async move {
            match client.lock() {
                Ok(mut c) => c.call(method, params),
                Err(_) => Err(CallError::Io(std::io::ErrorKind::BrokenPipe.into())),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| then(this, result, window, cx));
        })
        .detach();
    }

    /// Run a command whose only feedback is a refusal.
    pub fn command(&self, method: &'static str, params: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.call(method, params, window, cx, |_, result, window, cx| {
            if let Err(e) = result {
                window.push_notification(refusal("Seam can't do that", &error_text(&e)), cx);
            }
        });
    }

    /// Change settings. They show at once, and the tray has the last word.
    pub fn patch(&mut self, patch: Value, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(config) = self.config.patched(&patch) {
            self.config = config;
        }
        self.relink();
        cx.notify();
        self.call("config.patch", json!({ "patch": patch }), window, cx, |this, result, window, cx| {
            if let Err(e) = result {
                window.push_notification(refusal("Setting left as it was", &error_text(&e)), cx);
            }
            this.reload_config(window, cx);
        });
    }

    pub fn reload_config(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.call("config.get", json!({}), window, cx, |this, result, _, cx| {
            if let Some(config) = result.ok().and_then(|c| serde_json::from_value(c).ok()) {
                this.config = config;
                this.relink();
                cx.notify();
            }
        });
    }

    /// Say what just happened, with Undo for 8 s. A newer one replaces it.
    pub fn offer_undo(
        &mut self,
        message: String,
        undo: impl Fn(&mut Panel, &mut Window, &mut Context<Panel>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        struct Undo;
        let view = cx.entity().downgrade();
        let undo = std::rc::Rc::new(undo);
        let note = Notification::new().id::<Undo>().message(message).action(move |_, _, cx| {
            let (view, undo) = (view.clone(), undo.clone());
            ui::primary("undo", "Undo").on_click(cx.listener(move |note: &mut Notification, _, window, cx| {
                let undo = undo.clone();
                let _ = view.update(cx, |panel, cx| undo(panel, window, cx));
                note.dismiss(window, cx);
            }))
        });
        window.push_notification(note, cx);
        self.toast_generation += 1;
        let generation = self.toast_generation;
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

    /// Replace every setting with `next`.
    pub fn replace_config(&mut self, next: &Config, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(old), Ok(new)) = (serde_json::to_value(&self.config), serde_json::to_value(next)) else { return };
        self.patch(diff_patch(&old, &new), window, cx);
    }

    /// Show a page. A search result also names the row to scroll to on it.
    fn show(&mut self, page: Page, reveal: Option<&'static str>, window: &mut Window, cx: &mut Context<Self>) {
        // Unsaved zone edits ask before they're lost.
        if self.editor.is_some() {
            let then: pages::zone_editor::Then =
                std::rc::Rc::new(move |panel, window, cx| panel.show_now(page, reveal, window, cx));
            pages::zone_editor::guard(self, window, cx, then);
            return;
        }
        self.show_now(page, reveal, window, cx);
    }

    fn show_now(&mut self, page: Page, reveal: Option<&'static str>, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        let _ = std::fs::write(last_page_file(), page.title());
        if let Some(title) = reveal {
            // From the top, so a result that isn't a row of its own lands at the page's start.
            self.page_scroll.set_offset(Point::default());
            ui::reveal(title, &self.page_scroll);
        }
        self.clear_search(window, cx);
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query = SharedString::default();
        self.search.update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn nav_row(&self, page: Page, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let active = self.page == page;
        let id = ElementId::Name(format!("nav-{}", page.title()).into());
        let focus = window.use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle()).read(cx).clone();
        let update_ready =
            page == Page::About && self.state.as_ref().is_some_and(|s| s.update.as_ref().is_some_and(|u| u.ready));
        div()
            .id(id)
            .track_focus(&focus)
            .tab_index(0)
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(32.))
            .px(px(8.))
            .rounded(px(6.))
            .text_size(px(14.))
            .cursor_pointer()
            .when(active, |d| d.bg(p.raised).text_color(p.text))
            .when(!active, |d| d.text_color(p.secondary).hover(|d| d.bg(p.card)))
            .when(ui::focused(&focus, window), |d| d.focus_ring_style(window, cx))
            .child(Icon::new(page.icon()).size(px(16.)).text_color(if active { p.text } else { p.muted }))
            .child(div().flex_1().child(page.title()))
            .when(update_ready, |d| d.child(ui::dot(p.accent, true)))
            .on_click(cx.listener(move |this, _, window, cx| this.show(page, None, window, cx)))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                    this.show(page, None, window, cx);
                }
            }))
    }

    fn status_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let base = div().flex().items_center().gap(px(8.)).mt(px(12.)).p(px(8.)).rounded(px(8.)).bg(p.card);
        let text = |title: String, detail: String| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(title))
                .child(div().text_size(px(12.)).text_color(p.secondary).truncate().child(detail))
        };
        let Some(state) = &self.state else {
            return base
                .child(ui::dot(p.negative, false))
                .child(text("Seam isn't running".into(), "Remap is off".into()))
                .child(
                    ui::primary("start-seam", "Start").on_click(cx.listener(|_, _, _, cx| crate::restart_panel(cx))),
                );
        };
        let (title, color, filled) = match state.remap {
            Remap::On => ("Remap on", p.positive, true),
            Remap::Paused => ("Remap paused", p.muted, true),
            Remap::OffNoBorders => ("Not lined up", p.muted, false),
        };
        let screens = state.screens.len();
        let detail = format!("{}, {screens} screen{}", state.desk.name, if screens == 1 { "" } else { "s" });
        let enabled = self.config.enabled;
        let (icon, label) =
            if enabled { (assets::IconName::Pause, "Pause remap") } else { (assets::IconName::Play, "Resume remap") };
        let toggle = Button::new("toggle-remap")
            .ghost()
            .icon(Icon::new(icon).size(px(16.)).text_color(p.secondary))
            .tooltip(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.command("remap.set", json!({ "on": !enabled }), window, cx);
            }));
        base.child(ui::dot(color, filled)).child(text(title.into(), detail)).child(toggle)
    }

    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let search = Input::new(&self.search)
            .appearance(false)
            .prefix(Icon::new(assets::IconName::Search).size(px(16.)).text_color(p.muted))
            .suffix(ui::keycaps("Ctrl F", false, p).text_color(p.secondary));
        let mut top = div().flex().flex_col().gap(px(2.)).mt(px(16.));
        for page in Page::top_group() {
            top = top.child(self.nav_row(page, window, cx));
        }
        let mut app = div().flex().flex_col().gap(px(2.));
        for page in [Page::Shortcuts, Page::General, Page::About] {
            app = app.child(self.nav_row(page, window, cx));
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(240.))
            .h_full()
            .p(px(12.))
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.hairline)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(8.))
                    .rounded(px(8.))
                    .bg(p.input)
                    .border_1()
                    .border_color(p.hairline)
                    .child(search),
            )
            .child(self.status_row(cx))
            .child(top)
            .child(div().mt(px(24.)).mb(px(8.)).px(px(8.)).text_size(px(12.)).text_color(p.muted).child("App"))
            .child(app)
            .child(div().flex_1())
            .child(
                div()
                    .px(px(8.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(format!("Seam {}", env!("CARGO_PKG_VERSION"))),
            )
    }

    /// Dashed notes at the top of every page.
    fn banners(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let p = palette(cx);
        let mut banners = Vec::new();
        if self.dual_install {
            let text = "Seam is installed twice. Uninstall the GitHub version to keep the Store one";
            banners.push(ui::note(text, None, p));
        }
        if self.import_offer.is_some() {
            let import = ui::secondary("import-earlier", "Import", cx).on_click(cx.listener(|panel, _, window, cx| {
                if let Some(earlier) = panel.import_offer.take() {
                    panel.replace_config(&earlier, window, cx);
                    window.push_notification(Notification::new().message("Imported your settings"), cx);
                }
                cx.notify();
            }));
            banners.push(ui::note("Import settings from your earlier Seam", Some(import), p));
        }
        banners
    }

    fn results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let matches = search::find(&self.query);
        let mut list = div().flex().flex_col().gap(px(24.)).child(ui::page_title(
            "Search",
            match matches.len() {
                0 => format!("Nothing matches {}", self.query),
                1 => "1 setting".to_string(),
                n => format!("{n} settings"),
            },
            p,
        ));
        let rows = matches.iter().enumerate().map(|(i, hit)| {
            let (page, anchor) = (hit.page, hit.anchor());
            div()
                .id(i)
                .cursor_pointer()
                .hover(|d| d.bg(p.raised))
                .child(ui::row(hit.title, Some(page.title().into()), div(), p))
                .on_click(cx.listener(move |this, _, window, cx| this.show(page, Some(anchor), window, cx)))
                .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        if !rows.is_empty() {
            list = list.child(ui::card(rows, p));
        }
        list
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

type Session = (Client, Receiver<Event>, State, Value);

fn connect(token: &str) -> Result<Session, CallError> {
    let mut client = Client::connect("panel", Some(token))?;
    let events = Client::subscribe("panel", Some(token), &["state", "desk", "workspace", "update", "lineup"])?;
    let state = client.call("state.get", json!({}))?;
    let state = serde_json::from_value(state).map_err(std::io::Error::other)?;
    let config = client.call("config.get", json!({}))?;
    Ok((client, events, state, config))
}

fn next_event(events: &Mutex<Receiver<Event>>) -> Option<Event> {
    events.lock().ok()?.recv().ok()
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let body = if !self.query.is_empty() {
            self.results(cx).into_any_element()
        } else {
            match self.page {
                Page::Displays => pages::displays::render(self, window, cx).into_any_element(),
                Page::Zones => pages::zones::render(self, window, cx).into_any_element(),
                Page::Workspaces => pages::workspaces::render(self, window, cx).into_any_element(),
                Page::Shortcuts => pages::shortcuts::render(self, window, cx).into_any_element(),
                Page::General => pages::general::render(self, window, cx).into_any_element(),
                Page::About => pages::about::render(self, window, cx).into_any_element(),
            }
        };
        ui::reveal_built();
        let banners = self.banners(cx);
        pages::screen_editor::sync(self, cx);
        div()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.search.update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, window, cx| this.clear_search(window, cx)))
            .flex()
            .size_full()
            .bg(p.page)
            .text_color(p.text)
            .font_family(cx.theme().font_family.clone())
            .child(self.sidebar(window, cx))
            .child(
                // The page keeps its own scroll handle, so a search result can scroll it.
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div().id("page").size_full().overflow_y_scroll().track_scroll(&self.page_scroll).child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(ui::SECTION_GAP))
                                .px(px(32.))
                                .py(px(24.))
                                .children(banners)
                                .child(body),
                        ),
                    )
                    .vertical_scrollbar(&self.page_scroll),
            )
    }
}
