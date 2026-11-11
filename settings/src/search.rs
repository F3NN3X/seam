//! Search across the rows of every page.

use crate::help;
use crate::panel::Page;
use std::sync::LazyLock;

pub struct Entry {
    pub page: Page,
    pub title: &'static str,
    about: &'static str,
    keywords: &'static str,
    /// The row or section that holds it, when it's a button rather than a row of its own.
    within: Option<&'static str>,
    /// The Help topic it opens, by index into `help::TOPICS`. Its text is searched too.
    pub topic: Option<usize>,
}

impl Entry {
    /// The row or section opening this result scrolls to.
    pub fn anchor(&self) -> &'static str {
        self.within.unwrap_or(self.title)
    }
}

const fn entry(page: Page, title: &'static str, about: &'static str, keywords: &'static str) -> Entry {
    Entry { page, title, about, keywords, within: None, topic: None }
}

/// A button in About's Support section.
const fn support(title: &'static str, about: &'static str, keywords: &'static str) -> Entry {
    Entry { page: Page::About, title, about, keywords, within: Some("Support"), topic: None }
}

const ENTRIES: &[Entry] = &[
    entry(Page::Displays, "Line up screens", "Line up where your screens physically meet", "calibrate border"),
    entry(Page::Displays, "Align arrangement", "Move screens in Display settings to match", "windows"),
    entry(Page::Displays, "Physical layout", "Real sizes from each screen's EDID", "map identify"),
    entry(Page::Displays, "Show on screens", "Draw the layout on each screen for a few seconds", "windows seam view"),
    entry(Page::Displays, "Saved desks", "Rename or forget a desk", "layouts"),
    entry(Page::Displays, "Remap pointer at borders", "Cross where the screens physically meet", "pause mouse"),
    entry(Page::Displays, "Also align Windows arrangement", "Move screens in Display settings to match", ""),
    entry(
        Page::Displays,
        "Type measurements in Line Up",
        "Adds a field to type how far the screens are offset, in cm",
        "measure offset cm type",
    ),
    entry(
        Page::Displays,
        "Ease cursor movement between displays",
        "Windows' setting Seam needs. Displays offers to turn it on while it's off",
        "ease cursor easing deadzone jump pointer stuck",
    ),
    entry(Page::Zones, "Zones on", "Hold a key while you drag a window to drop it into a zone", "snap enable"),
    entry(
        Page::Zones,
        "Layout",
        "Columns, rows, grid, priority, main + stack or stack + main for each screen",
        "template tiles arrange organise",
    ),
    entry(Page::Zones, "Edit layout", "Split, merge and resize the zones of a screen", "editor custom divider"),
    entry(
        Page::Zones,
        "Where apps open",
        "Seam puts these apps in place whenever they open, outside workspaces too",
        "app rules always open place add an app",
    ),
    entry(
        Page::Zones,
        "Left alone",
        "Seam doesn't snap, move or capture their windows",
        "exclude never manage never include ignore",
    ),
    entry(Page::Zones, "Show zones", "Hold while dragging", "shift ctrl alt key"),
    entry(Page::Zones, "Span zones", "Add this key to cover more", "shift ctrl alt key"),
    entry(Page::Zones, "Gap", "Space between zones and the screen edge", "padding"),
    entry(
        Page::Zones,
        "Keep windows in their zones",
        "Experimental. Moves snapped windows back after a screen change",
        "refit resolution scaling unplug",
    ),
    entry(Page::Zones, "Restore size when dragged out", "The window gets its old size back", "snap"),
    entry(
        Page::Zones,
        "Resize neighbours with a window",
        "Windows in the zones next to it follow its edge",
        "resize neighbour edge follow",
    ),
    entry(Page::Zones, "Focus border", "Colour the border of the window that has focus", "windows highlight active"),
    entry(
        Page::Zones,
        "Focus border colour",
        "Windows accent follows Settings > Personalisation > Colours",
        "swatch none custom hex amber blue teal green red pink white",
    ),
    entry(
        Page::Zones,
        "Pin border colour",
        "Border of a window pinned on top or kept out of workspace switches",
        "swatch none custom hex amber blue teal green red pink white sticky topmost",
    ),
    entry(
        Page::Zones,
        "Tell me when a pinned window closes",
        "Click the notification to start the app again and pin its window",
        "notify notification reopen closed sticky topmost",
    ),
    entry(
        Page::Zones,
        "Pinned windows stay when I switch workspaces",
        "A window pinned on top isn't moved, minimized or closed",
        "pin topmost workspace switch sticky",
    ),
    entry(
        Page::Zones,
        "When a window moves to another screen",
        "Let Windows scale it, keep its pixel size or keep its real size",
        "size pixels scale scaling dpi real move screen",
    ),
    entry(
        Page::Zones,
        "Also when I drag a window to another screen",
        "The same size after you drop a window on another screen",
        "size pixels scale dpi drag mouse",
    ),
    entry(
        Page::Zones,
        "Another window manager handles my windows",
        "Snapping and window moves step aside",
        "komorebi glazewm leopardwm tiling coexistence",
    ),
    entry(Page::Workspaces, "Capture current windows", "Save which apps are where on this desk", "workspace save new"),
    entry(Page::Workspaces, "Update from screen", "Save the windows as they are now into a workspace", "capture"),
    entry(Page::Workspaces, "Switch now", "Put the apps of a workspace where they go", "workspace launch"),
    entry(
        Page::Workspaces,
        "Open alongside",
        "Switch without touching the other windows, just this once",
        "keep open leave alongside workspace",
    ),
    entry(Page::Workspaces, "Start missing apps", "Ticked apps open if they're closed", "launch"),
    entry(Page::Workspaces, "Other windows", "Leave, minimize or politely close the rest", "close minimize leave"),
    entry(Page::Workspaces, "Export workspace", "Save a workspace to a file", "import share backup"),
    entry(Page::Workspaces, "Add an app", "Add an open window or an installed app", "picker store start menu"),
    entry(
        Page::Workspaces,
        "Put apps in place when they open",
        "A window that opens goes where the workspace in use puts it, once",
        "auto apply rules automatic open",
    ),
    entry(
        Page::Workspaces,
        "Put windows back",
        "Moved or minimized windows go back where the workspace puts them",
        "snap back drift status moved",
    ),
    entry(
        Page::Workspaces,
        "Delay before opening",
        "Start order, a delay and waiting for an app's window",
        "launch stage order wait minimized",
    ),
    entry(Page::Shortcuts, "Line up screens", "Global shortcut", "hotkey keyboard"),
    entry(Page::Shortcuts, "Pause or resume pointer remap", "Global shortcut", "hotkey keyboard"),
    entry(Page::Shortcuts, "Move window left", "Global shortcut", "hotkey keyboard direction desk"),
    entry(Page::Shortcuts, "Move window right", "Global shortcut", "hotkey keyboard direction desk"),
    entry(Page::Shortcuts, "Move window up", "Global shortcut", "hotkey keyboard direction desk"),
    entry(Page::Shortcuts, "Move window down", "Global shortcut", "hotkey keyboard direction desk"),
    entry(Page::Shortcuts, "Focus window to the left", "Global shortcut", "hotkey keyboard direction activate"),
    entry(Page::Shortcuts, "Focus window to the right", "Global shortcut", "hotkey keyboard direction activate"),
    entry(Page::Shortcuts, "Focus window above", "Global shortcut", "hotkey keyboard direction activate up"),
    entry(Page::Shortcuts, "Focus window below", "Global shortcut", "hotkey keyboard direction activate down"),
    entry(Page::Shortcuts, "Swap with window to the left", "Global shortcut", "hotkey keyboard direction exchange"),
    entry(Page::Shortcuts, "Swap with window to the right", "Global shortcut", "hotkey keyboard direction exchange"),
    entry(Page::Shortcuts, "Swap with window above", "Global shortcut", "hotkey keyboard direction exchange up"),
    entry(Page::Shortcuts, "Swap with window below", "Global shortcut", "hotkey keyboard direction exchange down"),
    entry(
        Page::Shortcuts,
        "Focus on this window",
        "Minimize the others on its screen, and bring them back",
        "hotkey keyboard focus minimize others hide declutter",
    ),
    entry(
        Page::Shortcuts,
        "Spotlight",
        "Dim everything but the window you're using",
        "hotkey keyboard dim focus darken highlight",
    ),
    entry(Page::Shortcuts, "Grow window into its neighbour", "Global shortcut", "hotkey keyboard resize zone bigger"),
    entry(Page::Shortcuts, "Shrink window", "Global shortcut", "hotkey keyboard resize zone smaller"),
    entry(
        Page::Shortcuts,
        "Give window two thirds of its group",
        "Global shortcut",
        "hotkey keyboard priority main focus zone resize",
    ),
    entry(
        Page::Shortcuts,
        "Tile this screen's windows",
        "Global shortcut",
        "hotkey keyboard tile arrange organise layout zones screen",
    ),
    entry(Page::Shortcuts, "Pin window on top", "Global shortcut", "hotkey keyboard topmost always"),
    entry(Page::Shortcuts, "Keep window out of workspace switches", "Global shortcut", "hotkey keyboard sticky pin"),
    entry(Page::Shortcuts, "Release pinned windows", "Global shortcut", "hotkey keyboard unpin reset border"),
    entry(Page::Shortcuts, "Move window to next zone", "Global shortcut", "hotkey keyboard snap"),
    entry(Page::Shortcuts, "Move window to previous zone", "Global shortcut", "hotkey keyboard snap"),
    entry(Page::Shortcuts, "Capture current windows", "Global shortcut", "hotkey keyboard workspace"),
    entry(
        Page::Shortcuts,
        "Switch back to the previous workspace",
        "Global shortcut",
        "hotkey keyboard workspace back previous last",
    ),
    entry(
        Page::Shortcuts,
        "Snap back to this workspace",
        "Global shortcut",
        "hotkey keyboard workspace drift put back",
    ),
    entry(
        Page::Shortcuts,
        "Save windows to the active workspace",
        "Global shortcut",
        "hotkey keyboard workspace quick save capture update",
    ),
    entry(Page::General, "Start with Windows", "Seam waits in the tray after you sign in", "login autostart"),
    entry(Page::General, "Offer to line up new screens", "Asks once when screens Seam hasn't seen connect", ""),
    entry(
        Page::General,
        "Offer to restore my windows after I sign in",
        "Seam remembers which apps were where and asks before it starts any",
        "session restore reboot sign out login",
    ),
    entry(
        Page::General,
        "Who reopens the apps",
        "Windows' Restart apps, or Seam, after you sign in",
        "session restore restart apps sign-in options twice",
    ),
    entry(Page::General, "Left-click", "Right-click always shows the menu", "tray icon"),
    entry(Page::General, "Let other apps control Seam", "Stream Deck and tools on this PC", "api"),
    entry(Page::General, "Settings folder", "Export, import or open your settings", "config files"),
    entry(Page::General, "Debug log", "Writes remaps and screen changes to seam.log", "troubleshooting"),
    entry(Page::About, "Check for updates every day", "Last checked, and check now", "update version"),
    entry(Page::About, "Install updates on their own", "Only while you're away from the computer", "update"),
    entry(Page::About, "Include pre-releases", "Get new features early", "update beta"),
    support("Copy diagnostics", "Your version, Windows build, screens and saved borders", "support"),
    support("Report a problem", "Opens a new GitHub issue", "bug"),
    support("Source on GitHub", "", "code"),
    support("Licenses", "", "open source"),
];

/// One entry for each Help topic, found by any word of its text.
static TOPICS: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    let topic = |(i, t): (usize, &help::Topic)| Entry {
        page: Page::Help,
        title: t.title,
        about: "",
        keywords: "",
        within: None,
        topic: Some(i),
    };
    help::TOPICS.iter().enumerate().map(topic).collect()
});

pub fn find(query: &str) -> Vec<&'static Entry> {
    let query = query.trim().to_lowercase();
    let hit = |e: &&Entry| {
        [e.title, e.about, e.keywords, e.page.title()].iter().any(|t| t.to_lowercase().contains(&query))
            || e.topic.is_some_and(|i| help::TOPICS[i].mentions(&query))
    };
    ENTRIES.iter().chain(TOPICS.iter()).filter(hit).collect()
}
