//! Search across the rows of every page.

use crate::panel::Page;

pub struct Entry {
    pub page: Page,
    pub title: &'static str,
    about: &'static str,
    keywords: &'static str,
}

const fn entry(page: Page, title: &'static str, about: &'static str, keywords: &'static str) -> Entry {
    Entry { page, title, about, keywords }
}

const ENTRIES: &[Entry] = &[
    entry(Page::Displays, "Line up screens", "Line up where your screens physically meet", "calibrate border"),
    entry(Page::Displays, "Align arrangement", "Move screens in Display settings to match", "windows"),
    entry(Page::Displays, "Physical layout", "Real sizes from each screen's EDID", "map identify"),
    entry(Page::Displays, "Saved desks", "Rename or forget a desk", "layouts"),
    entry(Page::Displays, "Remap pointer at borders", "Cross where the screens physically meet", "pause mouse"),
    entry(Page::Displays, "Also align Windows arrangement", "Move screens in Display settings to match", ""),
    entry(Page::Zones, "Zones on", "Hold a key while you drag a window to drop it into a zone", "snap enable"),
    entry(Page::Zones, "Layout", "Columns, rows, grid or priority for each screen", "template tiles"),
    entry(Page::Zones, "Edit layout", "Split, merge and resize the zones of a screen", "editor custom divider"),
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
    entry(Page::Shortcuts, "Line up screens", "Global shortcut", "hotkey keyboard"),
    entry(Page::Shortcuts, "Pause or resume pointer remap", "Global shortcut", "hotkey keyboard"),
    entry(Page::Shortcuts, "Move window to next zone", "Global shortcut", "hotkey keyboard snap"),
    entry(Page::Shortcuts, "Move window to previous zone", "Global shortcut", "hotkey keyboard snap"),
    entry(Page::General, "Start with Windows", "Seam waits in the tray after you sign in", "login autostart"),
    entry(Page::General, "Offer to line up new screens", "Asks once when screens Seam hasn't seen connect", ""),
    entry(Page::General, "Left-click", "Right-click always shows the menu", "tray icon"),
    entry(Page::General, "Let other apps control Seam", "Stream Deck and tools on this PC", "api"),
    entry(Page::General, "Settings folder", "Export, import or open your settings", "config files"),
    entry(Page::General, "Debug log", "Writes remaps and screen changes to seam.log", "troubleshooting"),
    entry(Page::About, "Check for updates every day", "Last checked, and check now", "update version"),
    entry(Page::About, "Install updates on their own", "Only while you're away from the computer", "update"),
    entry(Page::About, "Include pre-releases", "Get new features early", "update beta"),
    entry(Page::About, "Copy diagnostics", "Your version, Windows build, screens and saved borders", "support"),
    entry(Page::About, "Report a problem", "Opens a new GitHub issue", "bug"),
    entry(Page::About, "Source on GitHub", "", "code"),
    entry(Page::About, "Licenses", "", "open source"),
];

pub fn find(query: &str) -> Vec<&'static Entry> {
    let query = query.trim().to_lowercase();
    ENTRIES
        .iter()
        .filter(|e| [e.title, e.about, e.keywords, e.page.title()].iter().any(|t| t.to_lowercase().contains(&query)))
        .collect()
}
