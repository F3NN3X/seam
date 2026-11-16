//! The command bar: the title bar's search field, and the actions and settings that open under it.

use super::*;
use crate::search::Entry;
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, IndexPath};

/// Results shown at most, in each group.
const MAX_ACTIONS: usize = 4;
const MAX_SETTINGS: usize = 5;

/// What stands at the right of a result.
#[derive(Clone)]
enum Hint {
    Word(String),
    /// A shortcut, as the config holds it.
    Keys(String),
}

enum Kind {
    Switch(String),
    PutBack,
    Capture,
    LineUp,
    PauseRemap,
    PauseSeam,
    ShowChanged,
    CopyDiagnostics,
}

struct Action {
    kind: Kind,
    name: String,
    /// Where it lives, which a search matches as well as the name.
    page: &'static str,
    hint: Hint,
}

/// A shortcut's keys when it's set, otherwise `word`.
fn hint(combo: Option<&String>, word: &str) -> Hint {
    combo.map_or_else(|| Hint::Word(word.into()), |combo| Hint::Keys(combo.clone()))
}

/// Everything the bar can do now, in the order it lists them.
fn actions(panel: &Panel) -> Vec<Action> {
    let keys = &panel.config.hotkeys;
    let mut all: Vec<Action> = pages::workspaces::on_desk(panel)
        .into_iter()
        .map(|ws| Action {
            kind: Kind::Switch(ws.id),
            name: format!("Switch to {}", ws.name),
            page: "Workspaces",
            hint: hint(ws.hotkey.as_ref(), "Workspace"),
        })
        .collect();
    if let Some(active) = panel.state.as_ref().and_then(|s| s.workspace.as_ref()) {
        let (name, hint) = ("Put windows back", Hint::Word(active.name.clone()));
        all.push(Action { kind: Kind::PutBack, name: name.into(), page: "Workspaces", hint });
    }
    let paused = panel.state.as_ref().is_some_and(|s| s.paused);
    all.push(Action {
        kind: Kind::Capture,
        name: "Capture current windows".into(),
        page: "Workspaces",
        hint: Hint::Word("Workspaces".into()),
    });
    all.push(Action {
        kind: Kind::LineUp,
        name: "Line up screens".into(),
        page: "Displays",
        hint: hint(keys.line_up.as_ref(), "Displays"),
    });
    all.push(Action {
        kind: Kind::PauseRemap,
        name: if panel.config.enabled { "Pause pointer remap" } else { "Resume pointer remap" }.into(),
        page: "Displays",
        hint: hint(keys.pause.as_ref(), "Displays"),
    });
    all.push(Action {
        kind: Kind::PauseSeam,
        name: if paused { "Resume Seam" } else { "Pause Seam" }.into(),
        page: "General",
        hint: Hint::Word("Seam".into()),
    });
    all.push(Action {
        kind: Kind::ShowChanged,
        name: "Show changed settings".into(),
        page: "Settings",
        hint: Hint::Word("Settings".into()),
    });
    all.push(Action {
        kind: Kind::CopyDiagnostics,
        name: "Copy diagnostics".into(),
        page: "About",
        hint: Hint::Word("About".into()),
    });
    all
}

/// Whether a name or the page it lives on holds `query`, ignoring case.
fn matches(query: &str, name: &str, page: &str) -> bool {
    let query = query.trim().to_lowercase();
    name.to_lowercase().contains(&query) || page.to_lowercase().contains(&query)
}

/// What a query found: the actions, then the settings, each group capped.
struct Results {
    actions: Vec<Action>,
    settings: Vec<&'static Entry>,
}

impl Results {
    fn of(panel: &Panel, query: &str) -> Results {
        Results {
            actions: actions(panel).into_iter().filter(|a| matches(query, &a.name, a.page)).take(MAX_ACTIONS).collect(),
            settings: search::find(query).into_iter().take(MAX_SETTINGS).collect(),
        }
    }

    fn len(&self) -> usize {
        self.actions.len() + self.settings.len()
    }
}

/// Where result number `index` stands in the list, when the groups are `actions` and `settings`
/// long. A group with nothing in it isn't there, so the settings are the first section then.
fn path(actions: usize, index: usize) -> IndexPath {
    let (section, row) = match (actions, index) {
        (0, _) => (0, index),
        (n, i) if i < n => (0, i),
        (n, i) => (1, i - n),
    };
    IndexPath { section, row, column: 0 }
}

/// The result at `at`, which `path` made.
fn index_of(actions: usize, at: IndexPath) -> usize {
    if at.section == 0 || actions == 0 { at.row } else { actions + at.row }
}

fn hint_element(hint: &Hint, p: Palette) -> AnyElement {
    match hint {
        Hint::Word(word) => text_style(div().flex_none(), tokens::CAPTION)
            .text_color(p.text_muted)
            .child(word.clone())
            .into_any_element(),
        Hint::Keys(combo) => ui::keycaps(combo, false, p).flex_none().text_color(p.text_secondary).into_any_element(),
    }
}

/// One result: its name, and its hint at the right.
fn item(name: &str, hint: Hint, p: Palette) -> CommandItem {
    let name = SharedString::from(name.to_string());
    let label = name.clone();
    CommandItem::new().label(name).child(move |_, _| {
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_between()
            .gap(px(tokens::SPACE_8))
            .child(text_style(div().min_w_0().truncate(), tokens::BODY).child(label.clone()))
            .child(hint_element(&hint, p))
    })
}

impl Panel {
    /// The search field, with the results under it while there's something typed.
    pub(super) fn command_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let search = Input::new(&self.search)
            .appearance(false)
            .aria_label("Search settings and actions")
            .prefix(Icon::new(assets::IconName::Search).size(px(tokens::SIZE_ICON)).text_color(p.text_muted))
            .suffix(ui::keycaps("Ctrl F", false, p).text_color(p.text_secondary));
        // The field takes the mouse, so only the bar around it drags the window.
        let field = div()
            .occlude()
            .flex()
            .items_center()
            .size_full()
            .px(px(tokens::SPACE_8))
            .rounded(px(tokens::RADIUS_8))
            .bg(p.surface_input)
            .border_1()
            .border_color(p.hairline)
            .child(search);
        let bounds = self.field_bounds.clone();
        div()
            .flex_shrink(1.)
            .w(px(tokens::SIZE_COMMAND_BAR))
            .min_w(px(200.))
            .h(px(32.))
            .on_children_prepainted(move |at, _, _| bounds.set(at[0]))
            // The field's own Up and Down go nowhere in one line, so the bar takes them first.
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let step = match e.keystroke.key.as_str() {
                    "down" => 1,
                    "up" => -1,
                    _ => return,
                };
                if !this.query.trim().is_empty() {
                    this.step(step, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(field)
            .when(!self.query.trim().is_empty(), |d| d.child(self.results(p, cx)))
    }

    /// The results, under the field and as wide as it.
    fn results(&self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let found = Results::of(self, &self.query);
        let actions = found.actions.len();
        let at = self.field_bounds.get();
        let mut list = Command::new(&self.command_list).searchable(false).filterable(false).bordered(false);
        if !found.actions.is_empty() {
            let rows = found.actions.iter().map(|a| item(&a.name, a.hint.clone(), p));
            list = list.group(CommandGroup::new().label("Actions").items(rows));
        }
        if !found.settings.is_empty() {
            let rows = found.settings.iter().map(|e| item(e.title, Hint::Word(e.page.title().into()), p));
            list = list.group(CommandGroup::new().label("Settings").items(rows));
        }
        let (select, confirm) = (cx.entity().downgrade(), cx.entity().downgrade());
        let list = list
            .on_select(move |at, _, cx| {
                let _ = select.update(cx, |panel, cx| {
                    panel.selected = index_of(actions, at);
                    cx.notify();
                });
            })
            .on_confirm(move |at, window, cx| {
                let _ = confirm.update(cx, |panel, cx| panel.run(index_of(actions, at), window, cx));
            })
            .max_h(px(360.))
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.hairline)
            .rounded(px(tokens::RADIUS_8))
            .shadow_lg()
            .empty(move |_, _, _| {
                text_style(div().p(px(tokens::SPACE_12)), tokens::LABEL)
                    .text_color(p.text_secondary)
                    .child("Nothing matches. Try a shorter word")
            })
            .footer(move |_, _, _| {
                text_style(div().px(px(tokens::SPACE_12)).py(px(tokens::SPACE_8)), tokens::CAPTION)
                    .border_t_1()
                    .border_color(p.hairline)
                    .text_color(p.text_muted)
                    .child("↑ ↓ to move, Enter to run or open, Esc to close")
            });
        let outside = cx.entity().downgrade();
        let field = self.field_bounds.clone();
        deferred(
            anchored()
                .position(point(at.left(), at.bottom() + px(tokens::SPACE_4)))
                .anchor(Anchor::TopLeft)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .occlude()
                        .w(at.size.width)
                        // A click that isn't on the field or the results puts the bar away.
                        .on_mouse_down_out(move |e, window, cx| {
                            if !field.get().contains(&e.position) {
                                let _ = outside.update(cx, |panel, cx| panel.clear_search(window, cx));
                            }
                        })
                        .child(list),
                ),
        )
        .with_priority(gpui_kit::base::POPUP_PRIORITY)
    }

    /// Move the highlight, round the ends.
    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let found = Results::of(self, &self.query);
        let len = found.len() as isize;
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + by).rem_euclid(len) as usize;
        let at = path(found.actions.len(), self.selected);
        self.command_list.update(cx, |list, cx| list.set_selected_index(Some(at), window, cx));
        cx.notify();
    }

    /// Ctrl+F: the field takes focus, and Esc gives it back to what had it.
    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.search.focus_handle(cx);
        if !field.is_focused(window) {
            self.return_focus = window.focused(cx);
        }
        field.focus(window, cx);
    }

    /// Esc: clear the field, close the results and give focus back.
    pub(super) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let in_bar = !self.query.is_empty() || self.search.focus_handle(cx).is_focused(window);
        self.clear_search(window, cx);
        if in_bar {
            self.give_back_focus(window, cx);
        }
    }

    fn give_back_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.return_focus.take().unwrap_or_else(|| self.focus.clone());
        back.focus(window, cx);
    }

    /// Enter in the field.
    pub(super) fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.trim().is_empty() {
            self.run(self.selected, window, cx);
        }
    }

    /// Do result number `index`: an action at once, a setting by opening its page and row.
    fn run(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let found = Results::of(self, &self.query);
        self.clear_search(window, cx);
        if index < found.actions.len() {
            self.act(&found.actions[index].kind, window, cx);
        } else if let Some(entry) = found.settings.get(index - found.actions.len()) {
            if let Some(topic) = entry.topic {
                self.help_topic = topic;
            }
            self.show(entry.page, Some(entry.anchor()), window, cx);
        }
        self.give_back_focus(window, cx);
    }

    /// An action, through what the pages and the sidebar already call.
    fn act(&mut self, kind: &Kind, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            Kind::Switch(id) => self.command("workspace.switch", json!({ "id": id }), window, cx),
            Kind::PutBack => self.command("workspace.snap_back", json!({}), window, cx),
            Kind::Capture => {
                self.show(Page::Workspaces, None, window, cx);
                pages::workspaces::capture(self, window, cx);
            }
            Kind::LineUp => self.command("lineup.start", json!({}), window, cx),
            Kind::PauseRemap => {
                let on = !self.config.enabled;
                self.command("remap.set", json!({ "on": on }), window, cx);
            }
            Kind::PauseSeam => {
                let on = self.state.as_ref().is_some_and(|s| s.paused);
                self.command("seam.pause", json!({ "on": on }), window, cx);
            }
            Kind::ShowChanged => {
                self.changed_only = true;
                cx.notify();
            }
            Kind::CopyDiagnostics => pages::about::copy_diagnostics(self, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IndexPath, index_of, matches, path};

    #[test]
    fn a_query_matches_the_name_or_the_page_whatever_its_case() {
        assert!(matches("LINE", "Line up screens", "Displays"));
        assert!(matches("  displays ", "Line up screens", "Displays"));
        assert!(!matches("zones", "Line up screens", "Displays"));
    }

    #[test]
    fn a_result_keeps_its_place_through_its_path() {
        for actions in [0, 1, 4] {
            for index in 0..9 {
                assert_eq!(index_of(actions, path(actions, index)), index);
            }
        }
    }

    #[test]
    fn settings_alone_are_the_first_section() {
        assert_eq!(path(0, 2), IndexPath { section: 0, row: 2, column: 0 });
        assert_eq!(path(2, 2), IndexPath { section: 1, row: 0, column: 0 });
    }
}
