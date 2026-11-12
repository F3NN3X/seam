//! "Remember folders and pages" (D330): the folder an Explorer window shows, read
//! through the shell's window list, and the page a browser shows, read from its
//! address bar through UI Automation. Only a capture asks for them. The Explorer
//! windows and each browser window are read on threads of their own, given at most
//! `FOLDER_WAIT` and `PAGE_WAIT`, so a hung window can't hold up a capture.

use crate::log;
use seam::winutil;
use seam::workspace::{self, Place, Rule};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use windows::core::{w, Interface, BSTR, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IServiceProvider, CLSCTX_ALL,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, TreeScope_Descendants, UIA_AutomationIdPropertyId,
    UIA_ClassNamePropertyId, UIA_ControlTypePropertyId, UIA_EditControlTypeId, UIA_ValueValuePropertyId,
};
use windows::Win32::UI::Shell::{
    IFolderView, IPersistFolder2, IShellBrowser, IShellWindows, IWebBrowserApp, SHGetPathFromIDListW,
    SID_STopLevelBrowser, ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;

/// The longest each browser window may take to read.
const PAGE_WAIT: Duration = Duration::from_millis(300);
/// The longest the Explorer windows may take to read, all together.
const FOLDER_WAIT: Duration = Duration::from_secs(1);

/// Browsers whose address bar is read, by exe name. Chromium's first.
const BROWSERS: [&str; 7] =
    ["chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "helium.exe", "firefox.exe"];

/// Bits of a private window's title, in lower case, in several languages. A false match
/// only means its page isn't saved. `privat` covers private, (private), privata and privaten.
const PRIVATE: [&str; 8] = ["incognito", "inkognito", "inprivate", "privat", "privad", "privé", "privée", "prywatn"];

/// Folder and page reads still running, ones that never ended included. At
/// `MAX_READING` a capture starts no more.
static READING: AtomicUsize = AtomicUsize::new(0);
const MAX_READING: usize = 8;

/// `GetDriveTypeW` answers for a removable and a fixed drive.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Explorer,
    Browser,
}

/// Whether a row's window is an Explorer window or a browser's, the only ones with a place.
pub(crate) fn kind_of(rule: &Rule) -> Option<Kind> {
    let name = winutil::file_name(&rule.exe);
    if name == "explorer.exe" && rule.class == "CabinetWClass" {
        return Some(Kind::Explorer);
    }
    // An installed web app runs the browser's exe too, but starts by its own AUMID.
    let web_app = rule.aumid.as_deref().is_some_and(|a| a.contains("_crx_"));
    (BROWSERS.contains(&name.as_str()) && !web_app).then_some(Kind::Browser)
}

/// The browser's arguments for a page in a new window of its own, so a running browser
/// doesn't add it as a tab to a window it has, which another row may own. `url` has
/// passed `safe_url`, so it's one argument.
pub(crate) fn new_window_args(rule: &Rule, url: &str) -> String {
    match winutil::file_name(&rule.exe).as_str() {
        "firefox.exe" => format!("-new-window {url}"),
        _ => format!("--new-window {url}"),
    }
}

/// Whether the path is on a fixed or removable drive of this PC, not a mapped one.
pub(crate) fn local_drive(path: &str) -> bool {
    let root: Vec<u16> = path.chars().take(2).chain(['\\']).map(|c| c as u16).chain([0]).collect();
    matches!(unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) }, DRIVE_FIXED | DRIVE_REMOVABLE)
}

/// Counts a read as running for as long as its thread lives.
struct Reading;

impl Reading {
    /// `None` while `MAX_READING` reads still run.
    fn enter() -> Option<Reading> {
        let mut n = READING.load(Ordering::SeqCst);
        while n < MAX_READING {
            match READING.compare_exchange_weak(n, n + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Reading),
                Err(now) => n = now,
            }
        }
        None
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        READING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The folder or page of each captured window that has one, in the rules' order.
pub fn read(rules: &[Rule], hwnds: &[isize]) -> Vec<Option<Place>> {
    let started = Instant::now();
    let left = |wait: Duration| (started + wait).saturating_duration_since(Instant::now());
    let kinds: Vec<Option<Kind>> = rules.iter().map(kind_of).collect();
    let explorers = kinds.contains(&Some(Kind::Explorer));
    let mut full = 0;
    let folders = if explorers { ask_folders() } else { None };
    full += usize::from(explorers && folders.is_none());
    // Every browser window at once, each on its own thread, while earlier reads don't hang.
    let mut asked: HashMap<usize, Receiver<Option<String>>> = HashMap::new();
    let browsers = rules.iter().enumerate().filter(|(i, _)| kinds[*i] == Some(Kind::Browser));
    for (i, rule) in browsers {
        let title = rule.title_hint.to_lowercase();
        if PRIVATE.iter().any(|w| title.contains(w)) {
            log::info!("places: {} skipped, a private window", winutil::file_name(&rule.exe));
            continue;
        }
        match ask_page(hwnds[i]) {
            Some(rx) => {
                asked.insert(i, rx);
            }
            None => full += 1,
        }
    }
    if full > 0 {
        log::info!("places: {} reads still running, {full} windows not read", READING.load(Ordering::SeqCst));
    }
    let mut late = 0;
    let pages: HashMap<usize, String> = asked
        .into_iter()
        .filter_map(|(i, rx)| {
            let page = rx.recv_timeout(left(PAGE_WAIT));
            late += usize::from(page.is_err());
            page.ok().flatten().map(|p| (i, p))
        })
        .collect();
    let folders = folders.and_then(|rx| rx.recv_timeout(left(FOLDER_WAIT)).ok());
    let places = (0..rules.len()).map(|i| match kinds[i] {
        Some(Kind::Explorer) => folders.as_ref().and_then(|f| f.get(&hwnds[i])).cloned().map(Place::Folder),
        Some(Kind::Browser) => pages.get(&i).cloned().map(Place::Url),
        None => None,
    });
    let places: Vec<Option<Place>> = places.collect();
    let (found, ms) = (places.iter().flatten().count(), started.elapsed().as_millis());
    let explorer = if explorers && folders.is_none() { ", Explorer too slow" } else { "" };
    log::info!("places: {found} folders and pages in {ms} ms, {late} browsers too slow{explorer}");
    places
}

/// Start reading the Explorer windows' folders on their own thread, unless too many
/// reads still run.
fn ask_folders() -> Option<Receiver<HashMap<isize, String>>> {
    let (tx, rx) = mpsc::channel();
    // Counted until the thread ends, or at once if it never starts.
    let reading = Reading::enter()?;
    let spawned = std::thread::Builder::new().name("seam-folders".into()).spawn(move || {
        let _reading = reading;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let folders = explorer_folders();
        unsafe { CoUninitialize() };
        let _ = tx.send(folders);
    });
    spawned.ok().map(|_| rx)
}

/// Each Explorer window's folder, by its top-level window. With tabs, the shown tab's.
/// A folder that isn't on a fixed or removable drive, or no longer exists, isn't kept.
fn explorer_folders() -> HashMap<isize, String> {
    let mut folders = HashMap::new();
    let Ok(windows) = (unsafe { CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_ALL) }) else {
        return folders;
    };
    for i in 0..unsafe { windows.Count() }.unwrap_or(0) {
        let Ok(browser) = (unsafe { windows.Item(&VARIANT::from(i)) }).and_then(|d| d.cast::<IWebBrowserApp>()) else {
            continue;
        };
        let Ok(top) = (unsafe { browser.HWND() }) else { continue };
        let Some((tab, folder)) = folder_of(&browser) else { continue };
        // A window with tabs has one of these per tab. The first tab window is the one on show.
        let shown = unsafe { FindWindowExW(Some(HWND(top.0 as *mut _)), None, w!("ShellTabWindowClass"), None) };
        if shown.is_ok_and(|s| s != tab) {
            continue;
        }
        // The drive first, so a mapped drive that's gone is never waited on.
        let local = folder.filter(|f| workspace::safe_folder(f) && local_drive(f) && Path::new(f).is_dir());
        folders.extend(local.map(|f| (top.0, f)));
    }
    folders
}

/// The window of one Explorer view (a tab's, with tabs) and its folder, `None` for a
/// folder with no path: This PC, the Recycle Bin, Control Panel, Home and the like.
fn folder_of(browser: &IWebBrowserApp) -> Option<(HWND, Option<String>)> {
    unsafe {
        let shell: IShellBrowser = browser.cast::<IServiceProvider>().ok()?.QueryService(&SID_STopLevelBrowser).ok()?;
        let tab = shell.GetWindow().ok()?;
        let view = shell.QueryActiveShellView().ok()?;
        let folder: IPersistFolder2 = view.cast::<IFolderView>().ok()?.GetFolder().ok()?;
        let pidl = folder.GetCurFolder().ok()?;
        let mut buf = [0u16; 260];
        let has_path = SHGetPathFromIDListW(pidl, &mut buf).as_bool();
        CoTaskMemFree(Some(pidl as *const _));
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some((tab, has_path.then(|| String::from_utf16_lossy(&buf[..len]))))
    }
}

/// Start reading a browser window's page on its own thread, unless too many reads still run.
fn ask_page(hwnd: isize) -> Option<Receiver<Option<String>>> {
    let (tx, rx) = mpsc::channel();
    // Counted until the thread ends, or at once if it never starts.
    let reading = Reading::enter()?;
    let spawned = std::thread::Builder::new().name("seam-page".into()).spawn(move || {
        let _reading = reading;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let page = address_bar(hwnd).and_then(|bar| workspace::page_of(&bar));
        unsafe { CoUninitialize() };
        let _ = tx.send(page);
    });
    spawned.ok().map(|_| rx)
}

/// What the browser's address bar holds: Chromium's omnibox by its class, Firefox's
/// by its automation id. Found the first one in the tree, the toolbar comes before the page.
fn address_bar(hwnd: isize) -> Option<String> {
    unsafe {
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        // UI Automation's own wait on a browser that stopped answering, so the thread ends soon too.
        if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
            let _ = uia2.SetTransactionTimeout(PAGE_WAIT.as_millis() as u32);
        }
        let root = uia.ElementFromHandle(HWND(hwnd as *mut _)).ok()?;
        let edit =
            uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0)).ok()?;
        let chromium = uia.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from("OmniboxViewViews")).ok()?;
        let firefox = uia.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from("urlbar-input")).ok()?;
        let bar = uia.CreateAndCondition(&edit, &uia.CreateOrCondition(&chromium, &firefox).ok()?).ok()?;
        let found = root.FindFirst(TreeScope_Descendants, &bar).ok()?;
        let value = found.GetCurrentPropertyValue(UIA_ValueValuePropertyId).ok()?;
        BSTR::try_from(&value).ok().map(|b| b.to_string())
    }
}
