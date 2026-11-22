//! The apps the workspace editor's "Add an app" offers: the real app windows open
//! now, by the capture's own rules, and the Start menu's list from
//! `shell:AppsFolder`. Both block, so the panel calls them off its UI thread.

use crate::classify::{Verdict, classify};
use crate::winutil;
use crate::workspace::{
    BROWSERS, EDITORS, Profile, Rule, TERMINAL_AUMIDS, default_profile, profiles, safe_aumid, safe_exe,
};
use std::collections::HashMap;
use windows::Win32::Storage::EnhancedStorage::PKEY_Link_TargetParsingPath;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2, KF_FLAG_DEFAULT,
    SHGetKnownFolderItem, SHGetKnownFolderPath, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};
use windows::core::{GUID, Interface, PWSTR, w};

/// Run `work` with COM up on this thread, for the property store and the shell.
fn with_com<T>(work: impl FnOnce() -> T) -> T {
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let out = work();
    if init.is_ok() {
        unsafe { CoUninitialize() };
    }
    out
}

/// Each real app window open now, topmost first, as a rule that keeps it where
/// it is, with its window. `exclude` holds lower-case exe names that are never managed.
pub fn open_now(exclude: &[String]) -> Vec<(Rule, isize)> {
    with_com(|| {
        let own = std::process::id();
        let excluded = |name: &str| exclude.iter().any(|x| x == name);
        let mut paths: HashMap<u32, Option<String>> = HashMap::new();
        let mut open = Vec::new();
        for h in winutil::top_level_windows() {
            let mut facts = winutil::window_facts(h, own);
            if classify(&facts, &excluded) != Verdict::Capture {
                continue;
            }
            let pid = winutil::process_id(h);
            let path = paths.entry(pid).or_insert_with(|| winutil::exe_path_of(pid)).clone();
            let Some(exe) = path.and_then(|path| winutil::exe_path_from(h, path)) else { continue };
            facts.exe_name = winutil::file_name(&exe);
            if classify(&facts, &excluded) != Verdict::Capture {
                continue;
            }
            let aumid = winutil::aumid(h);
            let rule = Rule { exe, aumid, class: facts.class, title_hint: winutil::title(h), ..Rule::default() };
            open.push((rule, h.0 as isize));
        }
        open
    })
}

fn take_string(text: PWSTR) -> Option<String> {
    let owned = unsafe { text.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    owned.filter(|s| !s.is_empty())
}

/// `{known folder id}\rest` as a path, which is how the Start menu names a
/// desktop app that has no AppUserModelID of its own.
fn known_folder_path(parsing: &str) -> Option<String> {
    let (id, rest) = parsing.strip_prefix('{')?.split_once("}\\")?;
    let id = GUID::try_from(id).ok()?;
    let folder = take_string(unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) }.ok()?)?;
    Some(format!(r"{folder}\{rest}"))
}

/// One Start menu entry as a rule Seam can start: a Store app by its AUMID, a
/// desktop app by the program its shortcut starts. Documents, links and anything
/// that isn't plainly safe to start are left out.
fn entry(item: &IShellItem) -> Option<Rule> {
    let name = take_string(unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.ok()?)?;
    let parsing = take_string(unsafe { item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING) }.ok()?)?;
    let app = |exe: String, aumid: Option<String>| Rule { exe, aumid, title_hint: name.clone(), ..Rule::default() };
    if parsing.contains('!') && safe_aumid(&parsing) {
        return Some(app(String::new(), Some(parsing)));
    }
    let target =
        item.cast::<IShellItem2>().ok().and_then(|item| unsafe { item.GetString(&PKEY_Link_TargetParsingPath) }.ok());
    let exe = target.and_then(take_string).or_else(|| known_folder_path(&parsing));
    match exe {
        Some(exe) if safe_exe(&exe) => {
            let seam = matches!(winutil::file_name(&exe).as_str(), "seam.exe" | "seam-settings.exe");
            (!seam).then(|| app(exe, None))
        }
        // An app that set its own AUMID, such as Office, starts by it.
        _ if safe_aumid(&parsing) => Some(app(String::new(), Some(parsing))),
        _ => None,
    }
}

/// The Start menu's apps, by name. Store and desktop apps alike, and no services
/// or helpers, because only what has a Start menu entry is there.
pub fn installed() -> Vec<Rule> {
    with_com(|| {
        let mut apps = Vec::new();
        let folder: Option<IShellItem> =
            unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) }.ok();
        let items: Option<IEnumShellItems> =
            folder.and_then(|f| unsafe { f.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) }.ok());
        let Some(items) = items else { return apps };
        loop {
            let (mut one, mut fetched) = ([None], 0u32);
            if unsafe { items.Next(&mut one, Some(&mut fetched)) }.is_err() || fetched == 0 {
                break;
            }
            if let Some(rule) = one[0].take().as_ref().and_then(entry) {
                apps.push(rule);
            }
        }
        apps.sort_by_key(|r| r.title_hint.to_lowercase());
        apps
    })
}

/// Windows Terminal's launcher, the App Execution Alias in `WindowsApps`, as the one path a
/// Terminal item starts (D734). It isn't a normal file, so `exists` can't tell if it's there.
pub fn terminal_alias() -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(format!(r"{local}\Microsoft\WindowsApps\wt.exe")).filter(|p| safe_exe(p))
}

/// Whether Windows Terminal is installed: its alias is there, or it's among `apps`, the
/// Start menu's (D739).
pub fn terminal_installed(apps: &[Rule]) -> bool {
    let alias = terminal_alias().is_some_and(|p| std::fs::symlink_metadata(p).is_ok());
    alias || apps.iter().any(|r| r.aumid.as_deref().is_some_and(|a| TERMINAL_AUMIDS.contains(&a)))
}

/// The Windows Terminal rule for a Terminal item's row: its AUMID, as among `apps` when it's
/// there, and the alias as its exe.
pub fn terminal_rule(apps: &[Rule]) -> Rule {
    let found = apps.iter().find(|r| r.aumid.as_deref().is_some_and(|a| TERMINAL_AUMIDS.contains(&a)));
    Rule {
        exe: terminal_alias().unwrap_or_default(),
        aumid: Some(found.and_then(|r| r.aumid.clone()).unwrap_or_else(|| TERMINAL_AUMIDS[0].to_string())),
        title_hint: "Windows Terminal".into(),
        class: "CASCADIA_HOSTING_WINDOW_CLASS".into(),
        ..Rule::default()
    }
}

/// The most of Windows Terminal's settings file that is read.
const MAX_SETTINGS: u64 = 1 << 20;

/// The text of Windows Terminal's `settings.json`, packaged, Preview or unpackaged, up to 1 MB.
/// Blocks on the disk, so never on the main thread.
fn terminal_settings() -> Option<String> {
    use std::io::Read;
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let places = [
        r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json",
        r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json",
        r"Microsoft\Windows Terminal\settings.json",
    ];
    places.iter().find_map(|rest| {
        let mut text = String::new();
        let file = std::fs::File::open(format!(r"{local}\{rest}")).ok()?;
        file.take(MAX_SETTINGS + 1).read_to_string(&mut text).ok()?;
        (text.len() as u64 <= MAX_SETTINGS).then_some(text)
    })
}

/// Windows Terminal's profiles to offer, and the name of the one it opens by default. Empty
/// when its settings can't be read (D739), so a tab then has the default profile and no command.
pub fn terminal_profiles() -> (Vec<Profile>, Option<String>) {
    let Some(text) = terminal_settings() else { return (Vec::new(), None) };
    let list = profiles(&text);
    let default = default_profile(&text, &list);
    (list, default)
}

/// The apps among `apps` whose exe file name passes `wanted`, in the Start menu's order.
fn with_name<'a>(apps: &'a [Rule], wanted: impl Fn(&str) -> bool + 'a) -> impl Iterator<Item = &'a Rule> {
    apps.iter().filter(move |r| safe_exe(&r.exe) && wanted(&winutil::file_name(&r.exe)))
}

/// The editors among `apps` (D739).
pub fn editors(apps: &[Rule]) -> Vec<Rule> {
    let editor = |name: &str| EDITORS.contains(&name.strip_suffix(".exe").unwrap_or(name));
    with_name(apps, editor).cloned().collect()
}

/// The browsers among `apps`, the default for http first when it can be told.
pub fn browsers(apps: &[Rule]) -> Vec<Rule> {
    let mut found: Vec<Rule> = with_name(apps, |name| BROWSERS.contains(&name)).cloned().collect();
    let default = default_browser().unwrap_or_default();
    found.sort_by_key(|r| winutil::file_name(&r.exe) != default);
    found
}

/// The exe name of the browser a ProgId belongs to, by its prefix.
pub fn browser_of_progid(progid: &str) -> Option<&'static str> {
    let progid = progid.to_lowercase();
    let known = [
        ("chromehtml", "chrome.exe"),
        ("msedgehtm", "msedge.exe"),
        ("firefoxurl", "firefox.exe"),
        ("bravehtml", "brave.exe"),
        ("vivaldihtm", "vivaldi.exe"),
        ("opera", "opera.exe"),
        ("helium", "helium.exe"),
    ];
    known.iter().find(|(prefix, _)| progid.starts_with(prefix)).map(|(_, exe)| *exe)
}

/// The browser that is the default for http, as its exe name, if it's one Seam knows.
fn default_browser() -> Option<&'static str> {
    let key = w!(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice");
    let mut buf = [0u16; 256];
    let mut size = size_of_val(&buf) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            w!("ProgId"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if read.0 != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    browser_of_progid(&String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_browsers_are_told_by_progid_prefix() {
        assert_eq!(browser_of_progid("MSEdgeHTM"), Some("msedge.exe"));
        assert_eq!(browser_of_progid("FirefoxURL-308046B0AF4A39CB"), Some("firefox.exe"));
        assert_eq!(browser_of_progid("ChromeHTML"), Some("chrome.exe"));
        assert_eq!(browser_of_progid("SomethingElse"), None);
    }

    #[test]
    fn editors_and_browsers_are_found_by_exe_stem() {
        let rule = |exe: &str| Rule { exe: exe.into(), ..Rule::default() };
        let apps = [
            rule(r"C:\a\Code.exe"),
            rule(r"C:\a\notepad.exe"),
            rule(r"C:\b\msedge.exe"),
            rule(r"C:\b\chrome.exe"),
            rule(r"\\s\x\zed.exe"),
        ];
        assert_eq!(editors(&apps).len(), 1);
        assert_eq!(browsers(&apps).len(), 2);
    }
}
