//! "Remember folders and pages" (D330): the folder an Explorer window shows, read
//! through the shell's window list, and the page a browser shows, read from its
//! address bar through UI Automation. Only the capture worker reads them, and each
//! browser window gets its own thread and at most `PAGE_WAIT`, so a hung browser
//! can't hold up a capture.

use crate::log;
use seam::winutil;
use seam::workspace::{self, Place, Rule};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use windows::core::{w, Interface, BSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IServiceProvider, CLSCTX_ALL,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
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

/// The longest a browser window's address bar may take to read.
const PAGE_WAIT: Duration = Duration::from_millis(300);

/// Browsers whose address bar is read, by exe name. Chromium's first.
const BROWSERS: [&str; 7] =
    ["chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "helium.exe", "firefox.exe"];

/// Words a private window's title carries, in Chromium, Edge, Firefox and Brave.
const PRIVATE: [&str; 3] = ["incognito", "inprivate", "private browsing"];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Explorer,
    Browser,
}

fn kind_of(rule: &Rule) -> Option<Kind> {
    let name = winutil::file_name(&rule.exe);
    if name == "explorer.exe" && rule.class == "CabinetWClass" {
        return Some(Kind::Explorer);
    }
    // An installed web app runs the browser's exe too, but starts by its own AUMID.
    let web_app = rule.aumid.as_deref().is_some_and(|a| a.contains("_crx_"));
    (BROWSERS.contains(&name.as_str()) && !web_app).then_some(Kind::Browser)
}

/// The folder or page of each captured window that has one, in the rules' order.
pub fn read(rules: &[Rule], hwnds: &[isize]) -> Vec<Option<Place>> {
    let started = Instant::now();
    let kinds: Vec<Option<Kind>> = rules.iter().map(kind_of).collect();
    let folders = if kinds.contains(&Some(Kind::Explorer)) { explorer_folders() } else { HashMap::new() };
    // Every browser window at once, each on its own thread.
    let mut pages: HashMap<usize, Receiver<Option<String>>> = HashMap::new();
    for (i, rule) in rules.iter().enumerate().filter(|(i, _)| kinds[*i] == Some(Kind::Browser)) {
        let title = rule.title_hint.to_lowercase();
        if PRIVATE.iter().any(|w| title.contains(w)) {
            log::info!("places: {} skipped, a private window", winutil::file_name(&rule.exe));
            continue;
        }
        pages.extend(ask_page(hwnds[i]).map(|rx| (i, rx)));
    }
    let deadline = Instant::now() + PAGE_WAIT;
    let (mut found, mut late) = (0, 0);
    let places = (0..rules.len()).map(|i| {
        let place = match kinds[i] {
            Some(Kind::Explorer) => folders.get(&hwnds[i]).cloned().map(Place::Folder),
            Some(Kind::Browser) => pages.get(&i).and_then(|rx| {
                let page = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                late += usize::from(page.is_err());
                page.ok().flatten().map(Place::Url)
            }),
            None => None,
        };
        found += usize::from(place.is_some());
        place
    });
    let places: Vec<Option<Place>> = places.collect();
    let ms = started.elapsed().as_millis();
    log::info!("places: {found} folders and pages in {ms} ms, {late} browsers too slow");
    places
}

/// Each Explorer window's folder, by its top-level window. With tabs, the shown tab's.
/// A folder that isn't on a local drive, or no longer exists, isn't kept.
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
        let local = folder.filter(|f| workspace::safe_folder(f) && Path::new(f).is_dir());
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

/// Start reading a browser window's page on its own thread.
fn ask_page(hwnd: isize) -> Option<Receiver<Option<String>>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("seam-page".into()).spawn(move || {
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
