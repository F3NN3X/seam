//! Drawing the panel: the title bar, the sidebar, the status and banners.

use super::*;
use crate::theme::tokens::text_style;
use crate::theme::{palette, tokens};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, TitleBar};
use seam::state::Remap;

impl Panel {
    /// Show a page. A search result also names the row to scroll to on it.
    pub fn show(&mut self, page: Page, reveal: Option<&'static str>, window: &mut Window, cx: &mut Context<Self>) {
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
        self.changed_only = false;
        if page == Page::Workspaces {
            self.icons.retry_failed();
        }
        if page == Page::Displays {
            self.ease_on = seam::ease::is_on();
        }
        let _ = std::fs::write(last_page_file(), page.title());
        self.ask_status(cx);
        if let Some(title) = reveal {
            // From the top, so a result that isn't a row of its own lands at the page's start.
            self.page_scroll.set_offset(Point::default());
            ui::reveal(title, &self.page_scroll);
        }
        self.clear_search(window, cx);
        cx.notify();
    }

    pub(super) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query = SharedString::default();
        self.search.update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// The rows of the page that differ from their default, worked out again only when the
    /// page or the settings change.
    fn changed_titles(&mut self) -> Rc<HashSet<&'static str>> {
        match &self.changed {
            Some((page, revision, titles)) if *page == self.page && *revision == self.revision => titles.clone(),
            _ => {
                let titles = Rc::new(search::changed::titles(self.page, &self.config));
                self.changed = Some((self.page, self.revision, titles.clone()));
                titles
            }
        }
    }

    fn nav_row(&self, page: Page, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
            .role(Role::Tab)
            .aria_label(page.title())
            .aria_selected(active)
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(32.))
            .px(px(8.))
            .rounded(px(6.))
            .text_size(px(14.))
            .cursor_pointer()
            .when(active, |d| d.bg(p.surface_raised).text_color(p.text_primary))
            .when(!active, |d| d.text_color(p.text_secondary).hover(|d| d.bg(p.surface_card)))
            .when(ui::focused(&focus, window), |d| ui::focus_ring(d, tokens::RADIUS_6, 0., p))
            .child(Icon::new(page.icon()).size(px(16.)).text_color(if active { p.text_primary } else { p.text_muted }))
            .child(div().flex_1().child(page.title()))
            .when(update_ready, |d| d.child(ui::dot(p.accent, true)))
            .on_click(cx.listener(move |this, _, window, cx| this.show(page, None, window, cx)))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                    this.show(page, None, window, cx);
                }
            }))
    }

    fn status_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let base = div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_8))
            .p(px(tokens::SPACE_8))
            .rounded(px(tokens::RADIUS_8))
            .bg(p.surface_card);
        let Some(state) = &self.state else {
            return base
                .child(ui::dot(p.negative, false))
                .child(text_style(div().flex_1().min_w_0(), tokens::LABEL_STRONG).child("Seam isn't running"))
                .child(
                    ui::primary("start-seam", "Start").on_click(cx.listener(|_, _, _, cx| crate::restart_panel(cx))),
                );
        };
        // A screen Seam hasn't lined up yet ("New screen to line up", a hollow negative dot,
        // linking to Displays) joins this match with the Note on Displays.
        let (title, color, filled) = match state.remap {
            _ if state.paused => ("Remap paused", p.text_muted, true),
            _ if state.stepped_aside => ("Stepped aside", p.text_muted, false),
            Remap::On => ("Remap on", p.positive, true),
            Remap::Paused => ("Remap paused", p.text_muted, true),
            Remap::OffNoBorders => ("Not lined up", p.text_muted, false),
        };
        let screens = state.screens.len();
        let (page, detail) = if state.stepped_aside && !state.paused {
            (Page::Windows, "An app is full screen".to_string())
        } else {
            let plural = if screens == 1 { "" } else { "s" };
            (Page::Displays, format!("{} desk, {screens} screen{plural}", state.desk.name))
        };
        let id = ElementId::Name("status-link".into());
        let focus = window.use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle()).read(cx).clone();
        let link = div()
            .id(id)
            .track_focus(&focus)
            .tab_index(0)
            .role(Role::Link)
            .aria_label(format!("{title}. {detail}"))
            .flex()
            .items_center()
            .flex_1()
            .min_w_0()
            .gap(px(tokens::SPACE_8))
            .rounded(px(tokens::RADIUS_6))
            .cursor_pointer()
            .when(ui::focused(&focus, window), |d| ui::focus_ring(d, tokens::RADIUS_6, 0., p))
            .child(ui::dot(color, filled))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text_style(div(), tokens::LABEL_STRONG).child(title))
                    .child(text_style(div(), tokens::CAPTION).text_color(p.text_secondary).truncate().child(detail)),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.show(page, None, window, cx)))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "space" | "enter") {
                    this.show(page, None, window, cx);
                }
            }));
        let enabled = self.config.enabled;
        let (icon, label) =
            if enabled { (assets::IconName::Pause, "Pause remap") } else { (assets::IconName::Play, "Resume remap") };
        let toggle = Button::new("toggle-remap")
            .ghost()
            .icon(Icon::new(icon).size(px(16.)).text_color(p.text_secondary))
            .tooltip(label)
            .accessibility_label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.command("remap.set", json!({ "on": !enabled }), window, cx);
            }));
        base.child(link).child(toggle)
    }

    /// The window's own title bar: Back, the mark, and the command bar slot between the kit's controls.
    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        // Back closes a help topic or a workspace's detail. Neither is a view of its own yet
        // (Help always shows a topic, Workspaces always its editor), so it stays disabled until
        // the Help and Workspaces pages bring them.
        let can_go_back = false;
        let back = Button::new("back")
            .ghost()
            .icon(Icon::new(assets::IconName::ChevronLeft).size(px(tokens::SIZE_ICON)).text_color(p.text_muted))
            .w(px(32.))
            .h(px(32.))
            .tooltip(if can_go_back { "Back" } else { "Back works inside a help topic or a workspace" })
            .accessibility_label("Back")
            .disabled(!can_go_back);
        let left = div()
            .flex()
            .items_center()
            .flex_none()
            .gap(px(tokens::SPACE_8))
            .child(div().occlude().child(back))
            .child(ui::mark(16., p))
            .child(text_style(div(), tokens::CAPTION).text_color(p.text_primary).child("Seam"));
        TitleBar::new()
            .h(px(tokens::SIZE_TITLE_BAR))
            .pl(px(tokens::SPACE_8))
            .when(p.mica, |bar| bar.bg(transparent_black()).border_0())
            .child(
                div()
                    .flex()
                    .items_center()
                    .size_full()
                    .gap(px(tokens::SPACE_8))
                    .child(left)
                    .child(div().flex_1())
                    .child(self.command_bar(cx))
                    .child(div().flex_1()),
            )
    }

    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let mut top = div().flex().flex_col().gap(px(tokens::SPACE_2)).mt(px(tokens::SPACE_16));
        for page in Page::top_group() {
            top = top.child(self.nav_row(page, window, cx));
        }
        let mut app = div().flex().flex_col().gap(px(tokens::SPACE_2));
        for page in Page::app_group() {
            app = app.child(self.nav_row(page, window, cx));
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(tokens::SIZE_SIDEBAR))
            .h_full()
            .p(px(tokens::SPACE_12))
            .bg(p.sidebar_ground())
            // Over Mica the sidebar has no edge of its own.
            .when(!p.mica, |d| d.border_r_1().border_color(p.hairline))
            .child(self.status_row(window, cx))
            .child(top)
            // The app's own pages sit at the bottom, above the version.
            .child(div().flex_1())
            .child(
                text_style(div(), tokens::CAPTION)
                    .mt(px(tokens::SPACE_24))
                    .mb(px(tokens::SPACE_8))
                    .px(px(tokens::SPACE_8))
                    .text_color(p.text_muted)
                    .child("App"),
            )
            .child(app)
            .child(
                text_style(div(), tokens::CAPTION)
                    .mt(px(tokens::SPACE_12))
                    .px(px(tokens::SPACE_8))
                    .text_color(p.text_muted)
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
        // Asked once. Displays has its own alert with the same button.
        if self.state.is_some() && !self.ease_on && !self.config.cursor_easing_asked && self.page != Page::Displays {
            let text = "Seam needs Windows' Ease cursor movement between displays, so the pointer can always leave a screen toward its neighbour. Turn it on?";
            let turn_on =
                ui::primary("ask-turn-on-easing", "Turn it on").on_click(cx.listener(|panel, _, window, cx| {
                    pages::displays::turn_on_easing(panel, window, cx);
                }));
            let not_now =
                ui::secondary("ask-not-now-easing", "Not now", cx).on_click(cx.listener(|panel, _, window, cx| {
                    panel.patch(json!({ "cursor_easing_asked": true }), window, cx);
                }));
            let buttons = div().flex().flex_none().gap(px(8.)).child(not_now).child(turn_on);
            banners.push(ui::note(text, None, p).child(buttons));
        }
        if self.changed_only {
            let text = if ui::nothing_shown() {
                "Nothing on this page differs from its default"
            } else {
                "Showing settings you've changed"
            };
            let show_all =
                ui::secondary("show-all-settings", "Show all", cx).on_click(cx.listener(|panel, _, _, cx| {
                    panel.changed_only = false;
                    cx.notify();
                }));
            banners.push(ui::note(text, Some(show_all), p));
        }
        banners
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let changed = self.changed_only.then(|| self.changed_titles());
        ui::show_only(changed);
        if !self.query.trim().is_empty() {
            self.found();
        }
        let body = match self.page {
            Page::Displays => pages::displays::render(self, window, cx).into_any_element(),
            Page::Zones => pages::zones::render(self, window, cx).into_any_element(),
            Page::Windows => pages::windows::render(cx).into_any_element(),
            Page::Workspaces => pages::workspaces::render(self, window, cx).into_any_element(),
            Page::Shortcuts => pages::shortcuts::render(self, window, cx).into_any_element(),
            Page::General => pages::general::render(self, window, cx).into_any_element(),
            Page::Help => pages::help::render(self, cx).into_any_element(),
            Page::About => pages::about::render(self, window, cx).into_any_element(),
        };
        ui::reveal_built();
        let banners = self.banners(cx);
        pages::screen_editor::sync(self, cx);
        div()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &ClearSearch, window, cx| this.dismiss(window, cx)))
            .flex()
            .flex_col()
            .size_full()
            .bg(p.page_ground())
            .text_color(p.text_primary)
            .font_family(cx.theme().font_family.clone())
            .child(self.title_bar(cx))
            .child(
                div().flex().flex_1().min_h_0().child(self.sidebar(window, cx)).child(
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
                                    .px(px(tokens::SPACE_32))
                                    .py(px(tokens::SPACE_24))
                                    .children(banners)
                                    .child(body),
                            ),
                        )
                        .vertical_scrollbar(&self.page_scroll),
                ),
            )
    }
}
