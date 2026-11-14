//! Drawing the panel: the sidebar, the status, banners and search results.

use super::*;
use crate::theme::palette;
use gpui_kit::component::Icon;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
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

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query = SharedString::default();
        self.search.update(cx, |input, cx| input.set_value("", window, cx));
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

    fn status_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
            _ if state.paused => ("Paused", p.muted, true),
            _ if state.stepped_aside => ("Stepped aside", p.muted, false),
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
            .accessibility_label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.command("remap.set", json!({ "on": !enabled }), window, cx);
            }));
        base.child(ui::dot(color, filled)).child(text(title.into(), detail)).child(toggle)
    }

    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
        for page in Page::app_group() {
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
            // The app's own pages sit at the bottom, above the version.
            .child(div().flex_1())
            .child(div().mt(px(24.)).mb(px(8.)).px(px(8.)).text_size(px(12.)).text_color(p.muted).child("App"))
            .child(app)
            .child(
                div()
                    .mt(px(12.))
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
        banners
    }

    fn results(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
            let (page, anchor, topic) = (hit.page, hit.anchor(), hit.topic);
            div()
                .id(i)
                .cursor_pointer()
                .hover(|d| d.bg(p.raised))
                .child(ui::row(hit.title, Some(page.title().into()), div(), p))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(topic) = topic {
                        this.help_topic = topic;
                    }
                    this.show(page, Some(anchor), window, cx);
                }))
                .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        if !rows.is_empty() {
            list = list.child(ui::card(rows, p));
        }
        list
    }
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
                Page::Help => pages::help::render(self, cx).into_any_element(),
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
