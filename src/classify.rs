//! Which windows a workspace captures: only real app windows, the ones with a
//! taskbar button. Pure, so the rules are in one place and read in order. The
//! facts come from `winutil::window_facts`, cheap ones first.

use crate::zones::Rect;

// Window styles, as the rules need them.
const WS_CHILD: u32 = 0x4000_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;

/// Smaller than this, in pixels, is a helper, not a window someone works in.
const MIN_W: i32 = 64;
const MIN_H: i32 = 32;

/// The shell's own windows.
const SHELL_CLASSES: [&str; 8] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "XamlExplorerHostIslandWindow",
];

/// Processes whose windows are never an app's, whatever their styles say.
const NOISE: [&str; 7] = [
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "lockapp.exe",
    "gamebar.exe",
    "gamebarftserver.exe",
];

/// What the rules look at, read from a top-level window.
#[derive(Clone, Debug, Default)]
pub struct WindowFacts {
    pub style: u32,
    pub ex_style: u32,
    pub class: String,
    pub title_len: usize,
    pub visible: bool,
    pub cloaked: bool,
    pub minimized: bool,
    /// It has an owner window.
    pub owned: bool,
    /// The Alt+Tab walk from its root owner ends on this window.
    pub root_owner_is_self: bool,
    /// The visible frame, or the normal rect of a minimized window.
    pub rect: Rect,
    /// `rect` touches a screen.
    pub on_any_screen: bool,
    /// The alpha of a layered window, when it sets one.
    pub layered_alpha: Option<u8>,
    /// Seam's own process.
    pub own_process: bool,
    /// The exe's file name in lower case. Empty until the window passes the
    /// rules that need only the window, so most windows never open their process.
    pub exe_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Capture,
    Skip(&'static str),
}

/// Whether to capture the window, or why not. With `exe_name` empty only the
/// window's own rules can skip it.
pub fn classify(f: &WindowFacts, user_excluded: &dyn Fn(&str) -> bool) -> Verdict {
    let ex = f.ex_style;
    let skip = if !f.visible {
        Some("hidden")
    } else if f.cloaked {
        Some("cloaked")
    } else if f.style & WS_CHILD != 0 {
        Some("child")
    } else if ex & WS_EX_APPWINDOW == 0 && ex & WS_EX_TOOLWINDOW != 0 {
        Some("tool window")
    } else if ex & WS_EX_APPWINDOW == 0 && f.owned {
        Some("owned")
    } else if ex & WS_EX_APPWINDOW == 0 && !f.root_owner_is_self {
        Some("not in Alt+Tab")
    } else if ex & WS_EX_NOACTIVATE != 0 {
        Some("no activate")
    } else if ex & WS_EX_LAYERED != 0 && (f.layered_alpha == Some(0) || ex & WS_EX_TRANSPARENT != 0) {
        Some("transparent")
    } else if f.title_len == 0 {
        Some("no title")
    } else if f.rect.w < MIN_W || f.rect.h < MIN_H {
        Some("too small")
    } else if !f.on_any_screen {
        Some("off screen")
    } else if SHELL_CLASSES.contains(&f.class.as_str()) || (f.exe_name == "explorer.exe" && f.class != "CabinetWClass")
    {
        // Of Explorer's windows, only File Explorer's folders are an app.
        Some("shell")
    } else if NOISE.contains(&f.exe_name.as_str()) {
        Some("system")
    } else if !f.exe_name.is_empty() && user_excluded(&f.exe_name) {
        Some("never include")
    } else if f.own_process || matches!(f.exe_name.as_str(), "seam.exe" | "seam-settings.exe") {
        Some("seam")
    } else {
        None
    };
    skip.map_or(Verdict::Capture, Verdict::Skip)
}
