//! The apps the workspace editor's "Add an app" offers: the real app windows open
//! now, by the capture's own rules, and the Start menu's list from
//! `shell:AppsFolder`. Both block, so the panel calls them off its UI thread.

use crate::classify::{classify, Verdict};
use crate::winutil;
use crate::workspace::{safe_aumid, safe_exe, Rule};
use std::collections::HashMap;
use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_Link_TargetParsingPath;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2, SHGetKnownFolderItem,
    SHGetKnownFolderPath, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

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
