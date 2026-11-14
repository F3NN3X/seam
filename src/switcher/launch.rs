//! Starting apps and opening places, without starting one twice.

use super::*;

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

/// Start an app that has no window yet. A packaged app, a PWA or a Squirrel app
/// starts through its AUMID, because the exe that was running isn't the thing to start.
/// With `places`, a row with a folder or page opens on it (D330).
pub(super) fn launch(rule: &Rule, places: bool) -> Result<(), &'static str> {
    if rule.exe.to_lowercase().contains(r"\steamapps\") {
        return Err("steam game");
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

/// `GetDriveTypeW` answer for a network drive (`DRIVE_REMOTE`).
const DRIVE_REMOTE_TYPE: u32 = 4;

/// Whether the drive the path is on is a network drive (a mapped share).
fn is_network_drive(exe: &str) -> bool {
    let root: Vec<u16> = exe.chars().take(2).chain(['\\']).map(|c| c as u16).chain([0]).collect();
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == DRIVE_REMOTE_TYPE }
}
