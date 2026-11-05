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
    entry(Page::Shortcuts, "Line up screens", "Global shortcut", "hotkey keyboard"),
    entry(Page::Shortcuts, "Pause or resume pointer remap", "Global shortcut", "hotkey keyboard"),
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
