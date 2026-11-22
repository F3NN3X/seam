//! Starting apps and opening places, without starting one twice.

use super::*;
use workspace::report::{COMMAND_NOT_RUN, FOREIGN_COMMAND};

/// Apps a switch has started and is still waiting on: key, when, and which switch. A
/// newer switch that replaces it waits for these instead of starting a second copy.
static LAUNCHES: Mutex<Vec<(String, Instant, u64)>> = Mutex::new(Vec::new());

pub(super) fn launch_key(rule: &Rule) -> String {
    match rule.aumid.as_deref().filter(|a| workspace::safe_aumid(a)) {
        Some(aumid) => aumid.to_string(),
        None => rule.exe.to_lowercase(),
    }
}

/// An earlier switch's launch of this app that no rule of this switch has taken over yet.
pub(super) fn earlier_launch(key: &str, mine: u64, taken: &mut Vec<usize>) -> Option<Instant> {
    let mut launches = LAUNCHES.lock().unwrap_or_else(|e| e.into_inner());
    launches.retain(|(_, at, _)| at.elapsed() < LAUNCH_WAIT);
    let i =
        launches.iter().enumerate().position(|(i, (k, _, r#gen))| k == key && *r#gen != mine && !taken.contains(&i))?;
    taken.push(i);
    Some(launches[i].1)
}

pub(super) fn note_launch(key: String, mine: u64) {
    LAUNCHES.lock().unwrap_or_else(|e| e.into_inner()).push((key, Instant::now(), mine));
}

/// Explorer by its full path, so a copy of it in the app folder never wins.
fn explorer_path() -> String {
    let mut buf = [0u16; 260];
    let len = unsafe { GetWindowsDirectoryW(Some(&mut buf)) } as usize;
    match len {
        1..=259 => format!(r"{}\explorer.exe", String::from_utf16_lossy(&buf[..len])),
        _ => r"C:\Windows\explorer.exe".to_string(),
    }
}

fn shell_open(file: &str, parameters: Option<&str>) -> bool {
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (file, parameters) = (wide(file), parameters.map(wide));
    let parameters = parameters.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
    // A value above 32 means the shell took it.
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(file.as_ptr()), parameters, PCWSTR::null(), SW_SHOWNORMAL).0 as isize
            > 32
    }
}

fn steam_game(rule: &Rule) -> bool {
    rule.exe.to_lowercase().contains(r"\steamapps\")
}

/// Start what a row opens when it has no window yet: its item (D731), or else its app. A
/// `Some` answer is a reason to list the row as skipped though it started.
pub(super) fn launch(rule: &Rule, places: bool, other_app: bool) -> Result<Option<&'static str>, &'static str> {
    match &rule.item {
        Some(item) if !steam_game(rule) => open_item(rule, item, other_app),
        _ => start_app(rule, places).map(|()| None),
    }
}

/// Start an app that has no window yet. A packaged app, a PWA or a Squirrel app
/// starts through its AUMID, because the exe that was running isn't the thing to start.
/// With `places`, a row with a folder or page opens on it (D330).
fn start_app(rule: &Rule, places: bool) -> Result<(), &'static str> {
    if steam_game(rule) {
        return Err("steam game");
    }
    // What I typed wins over a saved place, and a refusal never falls back to a plain start.
    if let Some(target) = &rule.open_with {
        return open_with(rule, target);
    }
    if let Some(place) = &rule.place {
        let (app, kind) = (app_name(rule), place_kind(place));
        match open_place(rule, place, places) {
            Ok(()) => {
                log::info!("{app}: started on its saved {kind}");
                return Ok(());
            }
            Err(why) => log::info!("{app}: saved {kind} not used, {why}"),
        }
    }
    let aumid = rule.aumid.as_deref().filter(|a| workspace::safe_aumid(a));
    if let Some(aumid) = aumid
        && shell_open(&explorer_path(), Some(&format!(r"shell:AppsFolder\{aumid}")))
    {
        return Ok(());
    }
    // Any workspace, imported or not, only starts an .exe on a drive letter, never a network drive.
    if !workspace::safe_exe(&rule.exe) || is_network_drive(&rule.exe) {
        return Err("refused");
    }
    if !Path::new(&rule.exe).is_file() {
        return Err("not installed");
    }
    shell_open(&rule.exe, None).then_some(()).ok_or("not installed")
}

/// What the log calls a place: never the folder or page itself.
pub(super) fn place_kind(place: &Place) -> &'static str {
    match place {
        Place::Folder(_) => "folder",
        Place::Url(_) => "page",
    }
}

/// Start an app on the folder or page its window showed: an Explorer row on a folder
/// of a fixed or removable drive that still exists, or a browser row on an http or
/// https page, in a window of its own. Otherwise why not, and the app starts as usual.
fn open_place(rule: &Rule, place: &Place, remember: bool) -> Result<(), &'static str> {
    // In order, so nothing touches the disk once a cheaper check has said no.
    let need = |ok: bool, why: &'static str| if ok { Ok(()) } else { Err(why) };
    need(remember, "the setting is off")?;
    let kind = places::kind_of(rule);
    match place {
        Place::Folder(folder) => {
            need(kind == Some(places::Kind::Explorer), "not an Explorer row")?;
            need(workspace::safe_folder(folder), "not a folder Seam opens")?;
            need(places::local_drive(folder), "the folder isn't on a drive of this PC")?;
            need(Path::new(folder).is_dir(), "the folder is gone")?;
            need(shell_open(&explorer_path(), Some(&workspace::explorer_arg(folder))), "it didn't start")
        }
        Place::Url(url) => {
            need(kind == Some(places::Kind::Browser), "not a browser row")?;
            need(workspace::safe_url(url), "not a page Seam opens")?;
            need(workspace::safe_exe(&rule.exe), "the browser can't be started from there")?;
            need(!is_network_drive(&rule.exe), "the browser can't be started from there")?;
            need(Path::new(&rule.exe).is_file(), "the browser isn't installed")?;
            // A browser that runs already would open the page as a tab of a window it has.
            need(shell_open(&rule.exe, Some(&places::new_window_args(rule, url))), "it didn't start")
        }
    }
}

/// Start an app on what I typed in "Open with". It's checked again here, whatever
/// saved it: a path goes to the row's program as one quoted argument, from a drive of
/// this PC, and a link goes to the shell. Only an app with no window is started, so
/// a window that is open is never navigated.
fn open_with(rule: &Rule, target: &str) -> Result<(), &'static str> {
    let app = app_name(rule);
    let refused = |why: &'static str| {
        log::info!("{app}: open with not used, {why}");
        Err("open with refused")
    };
    match workspace::open_with_for(rule, target) {
        Err(why) => refused(why),
        Ok(OpenWith::Link(link)) => match shell_open(&link, None) {
            true => Ok(()),
            false => refused("it didn't start"),
        },
        Ok(OpenWith::Path(path)) => {
            if !places::local_drive(&path) {
                return refused("the path isn't on a drive of this PC");
            }
            if !Path::new(&path).exists() {
                return refused("the path is gone");
            }
            let explorer = Path::new(&rule.exe).file_stem().is_some_and(|s| s.eq_ignore_ascii_case("explorer"));
            if explorer && !Path::new(&path).is_dir() {
                return refused("Explorer opens a folder here");
            }
            if !workspace::safe_exe(&rule.exe) || is_network_drive(&rule.exe) || !Path::new(&rule.exe).is_file() {
                return refused("the program can't be started from there");
            }
            match shell_open(&rule.exe, Some(&workspace::explorer_arg(&path))) {
                true => Ok(()),
                false => refused("it didn't start"),
            }
        }
    }
}

/// Start an item (D734), checked again here whatever saved it. Each kind's arguments come from
/// `workspace`'s pure builders. Only a row with no window gets here, so an open window is never
/// sent anywhere new.
fn open_item(rule: &Rule, item: &Item, other_app: bool) -> Result<Option<&'static str>, &'static str> {
    let app = app_name(rule);
    let refused = |why: &str| {
        log::info!("{app}: item not used, {why}");
        Err("item refused")
    };
    let item = match workspace::check_item(item, &rule.exe, rule.aumid.as_deref()) {
        Ok(item) => item,
        Err(why) => return refused(why),
    };
    let started = |ok: bool| if ok { Ok(None) } else { refused("it didn't start") };
    // The program a Folder item or an editor starts, from a drive of this PC.
    let program = |rule: &Rule| {
        let there = workspace::safe_exe(&rule.exe) && !is_network_drive(&rule.exe) && places::local_drive(&rule.exe);
        there && Path::new(&rule.exe).is_file()
    };
    let folder_there = |folder: &str| places::local_drive(folder) && Path::new(folder).is_dir();
    match &item {
        Item::Folder { folder } => match folder_there(folder) {
            true => started(shell_open(&explorer_path(), Some(&workspace::explorer_arg(folder)))),
            false => refused("the folder isn't on this PC or is gone"),
        },
        Item::Editor { folder } => match (program(rule), folder_there(folder)) {
            (true, true) => started(shell_open(&rule.exe, Some(&workspace::editor_args(folder)))),
            (false, _) => refused("the editor can't be started from there"),
            (_, false) => refused("the folder isn't on this PC or is gone"),
        },
        Item::Web { urls } => match (program(rule), workspace::web_args(&rule.exe, urls)) {
            (true, Ok(args)) => started(shell_open(&rule.exe, Some(&args))),
            (false, _) => refused("the browser can't be started from there"),
            (_, Err(why)) => refused(why),
        },
        Item::Terminal { tabs } => {
            let wt = seam::apps::terminal_alias().filter(|wt| std::fs::symlink_metadata(wt).is_ok());
            let Some(wt) = wt else { return refused("Windows Terminal isn't installed") };
            if tabs.iter().filter_map(|t| t.folder.as_deref()).any(|f| !folder_there(f)) {
                return refused("a tab's folder isn't on this PC or is gone");
            }
            // Only a command sealed for this account runs. One that doesn't open is dropped, and
            // its tab opens without it.
            let (commands, held_back) = commands_for(other_app, tabs, &seam::seal::unseal);
            let foreign = !held_back && foreign_command(tabs, &commands);
            let (profiles, _) = seam::apps::terminal_profiles();
            let unused = workspace::unused_commands(tabs, &profiles, &commands);
            match workspace::terminal_args(tabs, &profiles, &commands) {
                Ok(args) if shell_open(&wt, Some(&args)) => {
                    let note = command_note(held_back, foreign, unused);
                    if unused && !foreign {
                        log::info!(
                            "{app}: a startup command wasn't used, its profile isn't there or runs no shell Seam knows"
                        );
                    }
                    Ok(note)
                }
                Ok(_) => refused("it didn't start"),
                Err(why) => refused(why),
            }
        }
    }
}

/// Whether a tab's command didn't open here: it was sealed elsewhere, or was typed into the file in clear.
fn foreign_command(tabs: &[workspace::Tab], commands: &[Option<String>]) -> bool {
    tabs.iter().zip(commands).any(|(t, c)| c.is_none() && (t.command_sealed.is_some() || t.command.is_some()))
}

/// The commands a Terminal item starts with, and whether any was held back. Only a command
/// sealed for this account opens, and none runs when another app asked for the switch (D737).
fn commands_for(
    other_app: bool,
    tabs: &[workspace::Tab],
    unseal: &dyn Fn(&str) -> Option<String>,
) -> (Vec<Option<String>>, bool) {
    let held_back = other_app && tabs.iter().any(|t| t.command_sealed.is_some());
    let open = |t: &workspace::Tab| t.command_sealed.as_deref().and_then(unseal).filter(|_| !other_app);
    (tabs.iter().map(open).collect(), held_back)
}

/// What a started Terminal item says it didn't do: a command another app's switch held back, one
/// from elsewhere that never runs, and one whose profile is missing or runs no shell Seam knows.
fn command_note(held_back: bool, foreign: bool, unused: bool) -> Option<&'static str> {
    match (held_back, foreign, unused) {
        (true, ..) => Some(COMMAND_NOT_RUN),
        (false, true, _) => Some(FOREIGN_COMMAND),
        (false, false, true) => Some("item refused"),
        (false, false, false) => None,
    }
}

/// `GetDriveTypeW` answer for a network drive (`DRIVE_REMOTE`).
const DRIVE_REMOTE_TYPE: u32 = 4;

/// Whether the drive the path is on is a network drive (a mapped share).
fn is_network_drive(exe: &str) -> bool {
    let root: Vec<u16> = exe.chars().take(2).chain(['\\']).map(|c| c as u16).chain([0]).collect();
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == DRIVE_REMOTE_TYPE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workspace::Tab;

    #[test]
    fn a_command_that_didnt_open_here_is_foreign() {
        let sealed = Tab { command_sealed: Some("x".into()), ..Tab::default() };
        let typed = Tab { command: Some("calc".into()), ..Tab::default() };
        let none = Tab::default();
        assert!(foreign_command(std::slice::from_ref(&sealed), &[None]));
        assert!(foreign_command(&[typed], &[None]));
        assert!(!foreign_command(&[sealed], &[Some("ls".into())]));
        assert!(!foreign_command(&[none], &[None]));
    }

    #[test]
    fn a_started_terminal_says_what_it_left_out() {
        assert_eq!(command_note(false, true, true), Some(FOREIGN_COMMAND));
        assert_eq!(command_note(false, true, false), Some(FOREIGN_COMMAND));
        assert_eq!(command_note(false, false, true), Some("item refused"));
        assert_eq!(command_note(false, false, false), None);
        assert_eq!(command_note(true, true, true), Some(COMMAND_NOT_RUN));
    }

    #[test]
    fn another_apps_switch_runs_no_startup_command() {
        let tab = |sealed: Option<&str>| Tab { command_sealed: sealed.map(Into::into), ..Tab::default() };
        let open = |s: &str| Some(format!("open:{s}"));
        let tabs = [tab(Some("a")), tab(None)];
        // The Seam window, the tray, a shortcut and sign-in run what opens here.
        assert_eq!(commands_for(false, &tabs, &open), (vec![Some("open:a".to_string()), None], false));
        // Any other app gets the terminal without any command, and the row says so.
        assert_eq!(commands_for(true, &tabs, &open), (vec![None, None], true));
        // A terminal with no command has nothing held back.
        assert_eq!(commands_for(true, &[tab(None)], &open), (vec![None], false));
    }
}
