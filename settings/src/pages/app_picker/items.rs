//! The four dialogs of the picker's Other tab (D734): a folder, web links, a terminal and an
//! editor folder. The same dialog changes an item that's in a workspace already. Every check is
//! `workspace`'s; the dialog only says what it found, under the field.

use super::*;
use crate::theme::tokens::text_style;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::InputState;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use seam::workspace::{self, Item, Profile, Shell, Tab};
use std::cell::{Cell, RefCell};
use std::path::Path;

/// The tallest the list of addresses or tabs grows before it scrolls.
const LIST_HEIGHT: f32 = 360.;

/// What the Other tab needs to know about this PC, read once off the UI thread (D739).
#[derive(Clone, Default)]
pub struct Extras {
    pub terminal: bool,
    /// The row a Terminal item starts from: Windows Terminal's AUMID, and its launcher.
    pub terminal_rule: Rule,
    pub profiles: Vec<Profile>,
    pub default_profile: Option<String>,
    pub editors: Vec<Rule>,
    pub browsers: Vec<Rule>,
}

impl Extras {
    /// Blocks on the disk and the registry.
    pub fn read(installed: &[Rule]) -> Extras {
        let (profiles, default_profile) = apps::terminal_profiles();
        Extras {
            terminal: apps::terminal_installed(installed),
            terminal_rule: apps::terminal_rule(installed),
            profiles,
            default_profile,
            editors: apps::editors(installed),
            browsers: apps::browsers(installed),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Web,
    Terminal,
    Editor,
}

pub const KINDS: [Kind; 4] = [Kind::Folder, Kind::Web, Kind::Terminal, Kind::Editor];

impl Kind {
    pub fn of(item: &Item) -> Kind {
        match item {
            Item::Folder { .. } => Kind::Folder,
            Item::Web { .. } => Kind::Web,
            Item::Terminal { .. } => Kind::Terminal,
            Item::Editor { .. } => Kind::Editor,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::Web => "Web links",
            Kind::Terminal => "Terminal",
            Kind::Editor => "Editor folder",
        }
    }

    pub fn about(self, extras: &Extras) -> String {
        match self {
            Kind::Folder => "Opens a folder in Explorer".into(),
            Kind::Web => "Opens pages as tabs in one browser window".into(),
            Kind::Terminal => "Opens Windows Terminal tabs in a folder, with a command if you like".into(),
            Kind::Editor => {
                let editor = extras.editors.first().map_or("VS Code or another editor", |e| e.title_hint.as_str());
                format!("Opens a folder in {editor}")
            }
        }
    }

    /// Why this kind can't be added on this PC, for the tooltip of its dimmed entry.
    pub fn unavailable(self, extras: &Extras) -> Option<&'static str> {
        match self {
            Kind::Terminal if !extras.terminal => Some("Windows Terminal isn't installed"),
            Kind::Editor if extras.editors.is_empty() => {
                Some("No editor found. Pick one from Installed and type a folder in Open with")
            }
            _ => None,
        }
    }

    fn dialog_title(self, adding: bool) -> &'static str {
        match (self, adding) {
            (Kind::Folder, true) => "Add a folder",
            (Kind::Web, true) => "Add web links",
            (Kind::Terminal, true) => "Add a terminal",
            (Kind::Editor, true) => "Add an editor folder",
            (Kind::Folder, false) => "Change a folder",
            (Kind::Web, false) => "Change web links",
            (Kind::Terminal, false) => "Change a terminal",
            (Kind::Editor, false) => "Change an editor folder",
        }
    }
}

/// What a finished dialog hands on: the row as it should be, to add or to change a row into.
pub type Done = Rc<dyn Fn(&mut Panel, Rule, &mut Window, &mut Context<Panel>)>;

struct TabForm {
    /// `None` is Windows Terminal's default profile.
    profile: Option<String>,
    folder: Entity<InputState>,
    command: Entity<InputState>,
    /// The saved command came from another PC or a file (D736).
    unreadable: bool,
}

#[derive(Default)]
struct Errors {
    folder: Option<String>,
    program: Option<String>,
    list: Option<String>,
    addresses: Vec<Option<String>>,
    /// Each tab's folder and command.
    tabs: Vec<(Option<String>, Option<String>)>,
}

struct Form {
    kind: Kind,
    extras: Extras,
    /// The browsers or editors to choose from, and the one chosen.
    programs: Vec<Rule>,
    program: usize,
    folder: Entity<InputState>,
    addresses: Vec<Entity<InputState>>,
    tabs: Vec<TabForm>,
    errors: Errors,
}

const EXPLORER: &str = "explorer.exe";

/// Explorer by its full path.
fn explorer_exe() -> String {
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{windows}\{EXPLORER}")
}

fn field(placeholder: &'static str, value: String, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
}

fn new_form(kind: Kind, extras: Extras, existing: Option<&Rule>, window: &mut Window, cx: &mut App) -> Form {
    let mut programs = match kind {
        Kind::Web => extras.browsers.clone(),
        Kind::Editor => extras.editors.clone(),
        _ => Vec::new(),
    };
    let mut program = 0;
    if let Some(row) = existing.filter(|_| !programs.is_empty() || kind == Kind::Web || kind == Kind::Editor) {
        match programs.iter().position(|r| r.exe.eq_ignore_ascii_case(&row.exe)) {
            Some(at) => program = at,
            None => {
                programs.push(row.clone());
                program = programs.len() - 1;
            }
        }
    }
    let item = existing.and_then(|r| r.item.as_ref());
    let folder = match item {
        Some(Item::Folder { folder } | Item::Editor { folder }) => folder.clone(),
        _ => String::new(),
    };
    let folder = field(r"C:\Projects\seam", folder, window, cx);
    let urls = match item {
        Some(Item::Web { urls }) => urls.clone(),
        _ => vec![String::new()],
    };
    let addresses = urls.into_iter().map(|u| field("https://", u, window, cx)).collect();
    let tabs = match item {
        Some(Item::Terminal { tabs }) => tabs.iter().map(|t| tab_form(t, window, cx)).collect(),
        _ => vec![tab_form(&Tab { profile: extras.default_profile.clone(), ..Tab::default() }, window, cx)],
    };
    Form { kind, extras, programs, program, folder, addresses, tabs, errors: Errors::default() }
}

fn tab_form(tab: &Tab, window: &mut Window, cx: &mut App) -> TabForm {
    TabForm {
        profile: tab.profile.clone(),
        folder: field(r"C:\Projects\seam", tab.folder.clone().unwrap_or_default(), window, cx),
        command: field("", tab.command.clone().unwrap_or_default(), window, cx),
        unreadable: tab.unreadable,
    }
}

/// The shell of a tab's profile, `Other` for a profile Windows Terminal doesn't have (any more).
fn shell_of(form: &Form, profile: &Option<String>) -> Shell {
    let found = profile.as_ref().and_then(|name| form.extras.profiles.iter().find(|p| p.name == *name));
    found.map_or(Shell::Other, workspace::shell_of)
}

/// `Some(why)` for a folder that can't be used, in the words of Open with.
fn folder_problem(text: &str) -> Option<String> {
    let folder = match workspace::folder_ok(text) {
        Ok(folder) => folder,
        Err(why) => return Some(why.to_string()),
    };
    if !seam::winutil::local_drive(&folder) {
        return Some(r"Use a full path on this PC's own drive, like C:\Projects".into());
    }
    (!Path::new(&folder).is_dir()).then(|| "That folder doesn't exist".to_string())
}

fn text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}

/// Check what was typed. The row it makes, or `None` with each problem said under its field.
fn submit(form: &mut Form, cx: &App) -> Option<Rule> {
    form.errors = Errors::default();
    let kind = form.kind;
    let mut ok = true;
    let mut rule = match kind {
        Kind::Folder => Rule {
            exe: explorer_exe(),
            class: "CabinetWClass".into(),
            title_hint: "Explorer".into(),
            ..Rule::default()
        },
        Kind::Terminal => form.extras.terminal_rule.clone(),
        Kind::Web | Kind::Editor => match form.programs.get(form.program) {
            Some(program) => Rule { class: String::new(), ..program.clone() },
            None => {
                let what = if kind == Kind::Web { "browser" } else { "editor" };
                form.errors.program = Some(format!("Seam found no {what} to use"));
                return None;
            }
        },
    };
    let item = match kind {
        Kind::Folder | Kind::Editor => {
            let folder = text(&form.folder, cx);
            form.errors.folder = folder_problem(&folder);
            ok &= form.errors.folder.is_none();
            match kind {
                Kind::Folder => Item::Folder { folder },
                _ => Item::Editor { folder },
            }
        }
        Kind::Web => {
            let mut urls = Vec::new();
            for input in &form.addresses {
                let typed = text(input, cx);
                match workspace::address_ok(&typed) {
                    Ok(url) => {
                        urls.push(url);
                        form.errors.addresses.push(None);
                    }
                    Err(_) if typed.is_empty() && form.addresses.len() > 1 => form.errors.addresses.push(None),
                    Err(why) => {
                        ok = false;
                        form.errors.addresses.push(Some(why.to_string()));
                    }
                }
            }
            if urls.is_empty() && ok {
                ok = false;
                form.errors.addresses = vec![Some("Use a web address starting with http:// or https://".into())];
            }
            Item::Web { urls }
        }
        Kind::Terminal => {
            let mut tabs = Vec::new();
            for tab in &form.tabs {
                let (folder, command) = (text(&tab.folder, cx), text(&tab.command, cx));
                let folder_error = if folder.is_empty() { None } else { folder_problem(&folder) };
                let shell = shell_of(form, &tab.profile);
                let command_error = match (command.is_empty(), shell) {
                    (true, _) | (false, Shell::Other) => None,
                    (false, shell) => workspace::command_ok(&command)
                        .and_then(|c| match shell {
                            Shell::Cmd => workspace::cmd_escape(&c.unwrap_or_default()).map(Some),
                            _ => Ok(c),
                        })
                        .err()
                        .map(str::to_string),
                };
                ok &= folder_error.is_none() && command_error.is_none();
                form.errors.tabs.push((folder_error, command_error));
                let runs = !matches!(shell, Shell::Other) && !command.is_empty();
                tabs.push(Tab {
                    profile: tab.profile.clone(),
                    folder: Some(folder).filter(|f| !f.is_empty()),
                    command: runs.then_some(command),
                    ..Tab::default()
                });
            }
            Item::Terminal { tabs }
        }
    };
    if !ok {
        return None;
    }
    match workspace::check_item(&item, &rule.exe, rule.aumid.as_deref()) {
        Ok(item) => rule.item = Some(item),
        Err(why) => {
            form.errors.list = Some(why.to_string());
            return None;
        }
    }
    rule.launch = true;
    Some(rule)
}

// ---- What is drawn

fn labelled(label: &'static str, control: impl IntoElement, error: Option<&String>, p: Palette) -> Div {
    let error = error.map(|e| text_style(div(), tokens::LABEL).text_color(p.negative).child(e.clone()));
    div()
        .flex()
        .flex_col()
        .gap(px(tokens::SPACE_4))
        .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(label))
        .child(control)
        .children(error)
}

/// A field that holds a path, a link or a command, in the mono font.
fn mono(input: &Entity<InputState>, cx: &App) -> Div {
    div().flex_1().min_w_0().font_family(cx.theme().mono_font_family.clone()).child(Input::new(input))
}

/// A folder's field with Browse… beside it. The folder picker fills the field.
fn folder_row(id: impl Into<ElementId>, input: &Entity<InputState>, view: &WeakEntity<Panel>, cx: &App) -> Div {
    let (input_for, view) = (input.clone(), view.clone());
    let browse = ui::secondary(id, "Browse…", cx).small().on_click(move |_, window, cx| {
        let (input, view) = (input_for.clone(), view.clone());
        let _ = view.update(cx, |_, cx| browse(input, window, cx));
    });
    div().flex().items_center().gap(px(tokens::SPACE_8)).child(mono(input, cx)).child(browse)
}

fn browse(input: Entity<InputState>, window: &mut Window, cx: &mut Context<Panel>) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Choose".into()),
    });
    cx.spawn_in(window, async move |_, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        let Some(path) = paths.first().map(|p| p.to_string_lossy().to_string()) else { return };
        let _ = cx.update(|window, cx| input.update(cx, |state, cx| state.set_value(path, window, cx)));
    })
    .detach();
}

fn remove_button(id: impl Into<ElementId>, label: &'static str, cx: &App) -> Button {
    ui::secondary(id, "", cx).ghost().icon(Icon::new(assets::IconName::X)).tooltip(label).accessibility_label(label)
}

/// Browser or editor: the one the item opens with.
fn program_select(form: &Form, label: &'static str, state: &Rc<RefCell<Form>>, cx: &App) -> AnyElement {
    let choices: Vec<ui::Choice> =
        form.programs.iter().enumerate().map(|(i, r)| ui::Choice::new(i.to_string(), label_of(r))).collect();
    let state = state.clone();
    ui::select("item-program", label, choices, &form.program.to_string(), 240., cx, move |key, window, _| {
        state.borrow_mut().program = key.parse().unwrap_or(0);
        window.refresh();
    })
}

fn label_of(rule: &Rule) -> String {
    if rule.title_hint.is_empty() { app_title(rule) } else { rule.title_hint.clone() }
}

fn web_body(form: &Form, state: &Rc<RefCell<Form>>, view: &WeakEntity<Panel>, p: Palette, cx: &App) -> Vec<AnyElement> {
    let _ = view;
    let mut list =
        div().id("item-list").flex().flex_col().gap(px(tokens::SPACE_8)).max_h(px(LIST_HEIGHT)).overflow_y_scroll();
    for (n, input) in form.addresses.iter().enumerate() {
        let error = form.errors.addresses.get(n).and_then(Option::as_ref);
        let (keys, removing) = (state.clone(), state.clone());
        let line = div()
            .id(("address", n))
            .flex()
            .items_start()
            .gap(px(tokens::SPACE_8))
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                if !e.keystroke.modifiers.alt {
                    return;
                }
                let len = keys.borrow().addresses.len();
                let to = match e.keystroke.key.as_str() {
                    "up" if n > 0 => n - 1,
                    "down" if n + 1 < len => n + 1,
                    _ => return,
                };
                cx.stop_propagation();
                keys.borrow_mut().addresses.swap(n, to);
                window.refresh();
            })
            .child(labelled("Address", mono(input, cx), error, p).flex_1().min_w_0())
            .child(div().pt(px(tokens::SPACE_16)).child(
                remove_button(("remove-address", n), "Remove address", cx).on_click(move |_, window, _| {
                    let mut form = removing.borrow_mut();
                    if form.addresses.len() > 1 {
                        form.addresses.remove(n);
                        form.errors = Errors::default();
                    }
                    window.refresh();
                }),
            ));
        list = list.child(line);
    }
    let adding = state.clone();
    let add = ui::secondary("add-address", "Add address", cx).small().on_click(move |_, window, cx| {
        let mut form = adding.borrow_mut();
        if form.addresses.len() >= workspace::MAX_ADDRESSES {
            form.errors.list = Some("Up to 20 addresses".into());
        } else {
            let input = field("https://", String::new(), window, cx);
            form.addresses.push(input);
            form.errors.list = None;
        }
        window.refresh();
    });
    let mut body = vec![
        labelled("Browser", program_select(form, "Browser", state, cx), form.errors.program.as_ref(), p)
            .into_any_element(),
    ];
    body.push(list.into_any_element());
    body.push(
        div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_12))
            .child(add)
            .children(
                form.errors
                    .list
                    .as_ref()
                    .map(|e| text_style(div(), tokens::LABEL).text_color(p.negative).child(e.clone())),
            )
            .into_any_element(),
    );
    body
}

fn tab_group(form: &Form, n: usize, state: &Rc<RefCell<Form>>, view: &WeakEntity<Panel>, p: Palette, cx: &App) -> Div {
    let tab = &form.tabs[n];
    let (folder_error, command_error) = form.errors.tabs.get(n).cloned().unwrap_or_default();
    let names: Vec<&String> = form.extras.profiles.iter().map(|p| &p.name).collect();
    let mut choices: Vec<ui::Choice> = names.iter().map(|name| ui::Choice::new(name.as_str(), name.as_str())).collect();
    let mut gone = None;
    match &tab.profile {
        Some(name) if !names.contains(&name) => {
            choices.insert(0, ui::Choice::new(name.as_str(), name.as_str()));
            gone = Some(format!("Profile {name} isn't in Windows Terminal any more"));
        }
        None if choices.is_empty() => choices.push(ui::Choice::new("", "Default")),
        _ => {}
    }
    let picking = state.clone();
    let selected = tab.profile.clone().unwrap_or_default();
    let profile = ui::select(("tab-profile", n), "Profile", choices, &selected, 240., cx, move |key, window, _| {
        picking.borrow_mut().tabs[n].profile = (!key.is_empty()).then(|| key.to_string());
        window.refresh();
    });
    let shell = shell_of(form, &tab.profile);
    let runs = !matches!(shell, Shell::Other);
    let command =
        div().id(("tab-command", n)).child(mono(&tab.command, cx).font_family(cx.theme().mono_font_family.clone()));
    let command = match runs {
        true => command,
        false => command.opacity(0.5).tooltip(|window, cx| {
            Tooltip::new("Startup commands work in PowerShell and Command Prompt tabs").build(window, cx)
        }),
    };
    let command_error = command_error.or_else(|| {
        tab.unreadable.then(|| {
            "This command came from another PC or a file, so Seam won't run it. Type it again to use it here"
                .to_string()
        })
    });
    let removing = state.clone();
    let remove = remove_button(("remove-tab", n), "Remove tab", cx).on_click(move |_, window, _| {
        let mut form = removing.borrow_mut();
        if form.tabs.len() > 1 {
            form.tabs.remove(n);
            form.errors = Errors::default();
        }
        window.refresh();
    });
    let note = text_style(div(), tokens::LABEL)
        .text_color(p.text_secondary)
        .child("Runs each time Seam starts this terminal. Seam keeps it on this PC only");
    div()
        .flex()
        .items_start()
        .gap(px(tokens::SPACE_8))
        .p(px(tokens::SPACE_12))
        .border_1()
        .border_color(p.hairline)
        .rounded(px(tokens::RADIUS_8))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(tokens::SPACE_8))
                .child(labelled("Profile", profile, gone.as_ref(), p))
                .child(labelled(
                    "Starting folder",
                    folder_row(("tab-browse", n), &tab.folder, view, cx),
                    folder_error.as_ref(),
                    p,
                ))
                .child(labelled("Startup command", command, command_error.as_ref(), p).when(runs, |d| d.child(note))),
        )
        .child(remove)
}

fn terminal_body(
    form: &Form,
    state: &Rc<RefCell<Form>>,
    view: &WeakEntity<Panel>,
    p: Palette,
    cx: &App,
) -> Vec<AnyElement> {
    let mut list =
        div().id("item-list").flex().flex_col().gap(px(tokens::SPACE_8)).max_h(px(LIST_HEIGHT)).overflow_y_scroll();
    for n in 0..form.tabs.len() {
        list = list.child(tab_group(form, n, state, view, p, cx));
    }
    let adding = state.clone();
    let add = ui::secondary("add-tab", "Add tab", cx).small().on_click(move |_, window, cx| {
        let mut form = adding.borrow_mut();
        if form.tabs.len() >= workspace::MAX_TABS {
            form.errors.list = Some("Up to 8 tabs".into());
        } else {
            let profile = form.extras.default_profile.clone();
            let tab = tab_form(&Tab { profile, ..Tab::default() }, window, cx);
            form.tabs.push(tab);
            form.errors.list = None;
        }
        window.refresh();
    });
    let error =
        form.errors.list.as_ref().map(|e| text_style(div(), tokens::LABEL).text_color(p.negative).child(e.clone()));
    vec![
        list.into_any_element(),
        div().flex().items_center().gap(px(tokens::SPACE_12)).child(add).children(error).into_any_element(),
    ]
}

fn body(form: &Form, state: &Rc<RefCell<Form>>, view: &WeakEntity<Panel>, p: Palette, cx: &App) -> Div {
    let column = div().flex().flex_col().gap(px(tokens::SPACE_12));
    match form.kind {
        Kind::Folder => column.child(labelled(
            "Folder",
            folder_row("item-browse", &form.folder, view, cx),
            form.errors.folder.as_ref(),
            p,
        )),
        Kind::Editor => column
            .child(labelled("Editor", program_select(form, "Editor", state, cx), form.errors.program.as_ref(), p))
            .child(labelled(
                "Folder",
                folder_row("item-browse", &form.folder, view, cx),
                form.errors.folder.as_ref(),
                p,
            )),
        Kind::Web => column.children(web_body(form, state, view, p, cx)),
        Kind::Terminal => column.children(terminal_body(form, state, view, p, cx)),
    }
}

/// Open the dialog. With `existing`, it changes that row's item, and `done` gets the row as
/// it should be; without, it adds to `workspace`.
pub fn open(
    kind: Kind,
    extras: Extras,
    (workspace_name, existing): (String, Option<Rule>),
    done: Done,
    window: &mut Window,
    cx: &mut Context<Panel>,
) {
    let state = Rc::new(RefCell::new(new_form(kind, extras, existing.as_ref(), window, cx)));
    let view = cx.entity().downgrade();
    let first = Cell::new(true);
    let adding = existing.is_none();
    window.open_dialog(cx, move |dialog, window, cx| {
        let p = palette(cx);
        if first.replace(false) {
            let form = state.borrow();
            let input = match kind {
                Kind::Web => form.addresses.first().cloned(),
                Kind::Terminal => form.tabs.first().map(|t| t.folder.clone()),
                _ => Some(form.folder.clone()),
            };
            if let Some(input) = input {
                input.update(cx, |state, cx| state.focus(window, cx));
            }
        }
        let shown = body(&state.borrow(), &state, &view, p, cx);
        let (ok_label, close_label) = match adding {
            true => (format!("Add to {workspace_name}"), "Don't add"),
            false => ("Save changes".to_string(), "Don't save"),
        };
        let wide = if kind == Kind::Terminal { tokens::SIZE_DIALOG_WIDE } else { tokens::SIZE_DIALOG };
        let (state, view, done) = (state.clone(), view.clone(), done.clone());
        dialog
            .w(px(wide))
            .title(kind.dialog_title(adding))
            .child(shown)
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(move |b| b.label(close_label)))
                    .child(DialogAction::new().child(ui::primary("item-ok", ok_label))),
            )
            .on_ok(move |_, window, cx| {
                let built = submit(&mut state.borrow_mut(), cx);
                let Some(rule) = built else {
                    window.refresh();
                    return false;
                };
                let done = done.clone();
                let _ = view.update(cx, |panel, cx| done(panel, rule, window, cx));
                true
            })
    });
}
