//! Search across the rows of every page.

use crate::help;
use crate::panel::Page;
use std::sync::LazyLock;

pub mod changed;

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

const ENTRIES: &[Entry] = &[
    entry(Page::Displays, "Line up screens", "Line up where your screens physically meet", "calibrate border"),
    entry(
        Page::Displays,
        "Match Windows to this layout",
        "Move screens in Display settings to match",
        "align arrangement windows",
    ),
    entry(Page::Displays, "Physical layout", "Real sizes from each screen's EDID", "map identify"),
    entry(Page::Displays, "Show on screens", "Draw the layout on each screen for a few seconds", "windows seam view"),
    entry(
        Page::Displays,
        "Desks",
        "Rename this desk, or forget the ones not seen in 30 days",
        "saved desks layouts rename forget",
    ),
    entry(
        Page::Displays,
        "Borders",
        "Which screens meet where, and which of them the pointer crosses",
        "border glued unglue lined up not on desk stops pointer",
    ),
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
        "Edge resistance",
        "Push this far past a screen's edge before the pointer crosses",
        "resistance edge grab corner push",
    ),
    entry(
        Page::Displays,
        "Glued edge push",
        "How far to push past a glued edge before the pointer crosses",
        "push taskbar glue edge",
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
        "Custom layouts",
        "Your own layouts, shared by every desk and screen",
        "custom layout library new layout rename duplicate delete save as layout changed",
    ),
    entry(
        Page::Zones,
        "Used on",
        "The screens a layout of your own is offered on",
        "screen tie filter show all screens your layouts",
    ),
    entry(
        Page::Zones,
        "Save from windows",
        "Make a layout from where the windows on a screen are now",
        "custom layout library save windows arrangement",
    ),
    entry(
        Page::Windows,
        "Where apps open",
        "Seam puts these apps in place, or keeps them on top, whenever they open, outside workspaces too",
        "app rules always open place add an app",
    ),
    entry(
        Page::Windows,
        "Keep an app on top",
        "An app rule can pin its windows on top, so Release pinned windows takes them back",
        "pin always on top app rules keep on top sticky doesn't move",
    ),
    entry(
        Page::Windows,
        "Left alone",
        "Seam doesn't snap, move or capture their windows",
        "exclude never manage never include ignore",
    ),
    entry(
        Page::Zones,
        "Show zones while dragging",
        "Hold a key, or pause with the window still",
        "shift ctrl alt key pause",
    ),
    entry(
        Page::Zones,
        "Span zones",
        "Add this key to cover more than one zone",
        "shift ctrl alt key pull out take out of layout close gap",
    ),
    entry(Page::Zones, "Gap between zones", "Space between zones and the screen edge", "gap padding"),
    entry(
        Page::Zones,
        "Screen margins",
        "Room kept free around the zones on this screen",
        "top right bottom left edge border inset ultrawide bezel",
    ),
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
    entry(
        Page::Zones,
        "Handles between snapped windows",
        "Drag one to resize both sides. Right-click for sizes",
        "divider handle gap drag share half third quarter take out",
    ),
    entry(
        Page::Zones,
        "Swap with the window already in a zone",
        "Off lets windows overlap in a zone",
        "swap exchange trade overlap drop occupied",
    ),
    // A drag key, not a row: it opens the swap row, which S turns into a stack.
    Entry {
        page: Page::Zones,
        title: "Stacks",
        about: "Press S in a drag to stack a window on the one in the zone",
        keywords: "stack tabs several windows one zone cycle behind top overlap",
        within: Some("Swap with the window already in a zone"),
        topic: None,
    },
    entry(
        Page::Windows,
        "Arrange new windows automatically",
        "Each new window takes a share of the screen it opens on, on this desktop",
        "smart tiling tile tiler auto arrange automatic dwindle i3 virtual desktop new windows",
    ),
    entry(
        Page::Windows,
        "How windows are arranged",
        "Smart, Columns, Main + stack, Grid or Scrolling, while smart tiling is on",
        "smart tiling family columns main stack grid scrolling dwindle layout shape",
    ),
    entry(
        Page::Windows,
        "Scrolling",
        "Each new window is a column on a strip wider than the screen, scrolled into view",
        "smart tiling scrolling scroll strip columns niri paperwm wheel win parked off screen",
    ),
    entry(
        Page::Windows,
        "Put windows back when Seam quits",
        "Quitting Seam frees every tiled screen first, as Free all windows does",
        "smart tiling quit exit free all windows put back restore arrangement",
    ),
    Entry {
        page: Page::Windows,
        title: "Leave out of smart tiling",
        about: "An app's windows float on a tiled screen",
        keywords: "smart tiling float never tile exclude app rule",
        within: Some("Where apps open"),
        topic: None,
    },
    entry(
        Page::Zones,
        "Snap beside a window",
        "Drop on a window's edge to put the two side by side",
        "beside next to edge pair group split half side by side ad-hoc",
    ),
    // The fourth choice of "Show zones while dragging".
    Entry {
        page: Page::Zones,
        title: "Pause to snap",
        about: "Hold still while dragging to show zones, no key needed",
        keywords: "pause rest hold still wait drag show zones without key",
        within: Some("Show zones while dragging"),
        topic: None,
    },
    entry(
        Page::Zones,
        "Show keys while dragging",
        "A strip on the screen names the wheel, right-click and the keys",
        "hints keys strip wheel scroll right-click tab half third split esc drag keyboard",
    ),
    entry(
        Page::Windows,
        "Focus border",
        "Windows accent follows Settings › Personalisation › Colours. None turns it off",
        "colour color highlight active swatch none custom hex amber blue teal green red pink white",
    ),
    entry(
        Page::Windows,
        "Pin border",
        "Border of a window pinned on top or kept out of workspace switches",
        "colour color swatch none custom hex amber blue teal green red pink white sticky topmost",
    ),
    entry(
        Page::Windows,
        "Tell me when a pinned window closes",
        "Click the notification to start the app again and pin its window",
        "notify notification reopen closed sticky topmost",
    ),
    entry(
        Page::Windows,
        "Pinned windows stay when I switch workspaces",
        "A window pinned on top isn't moved, minimized or closed",
        "pin topmost workspace switch sticky",
    ),
    entry(
        Page::Windows,
        "Move and resize from anywhere in a window",
        "Hold a key and drag to move, or right-drag to resize. Win leaves Alt+click to apps",
        "alt drag altsnap winscreen move resize anywhere win key right-drag",
    ),
    entry(
        Page::Windows,
        "When a window moves to another screen",
        "Let Windows scale it, keep pixel size or keep real size",
        "size pixels scale scaling dpi real move screen",
    ),
    entry(
        Page::Windows,
        "Also when I drag a window to another screen",
        "The same size after you drop a window on another screen",
        "size pixels scale dpi drag mouse",
    ),
    entry(
        Page::Windows,
        "Another window manager handles my windows",
        "Snapping and window moves step aside. The pointer remap keeps working",
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
    entry(Page::Workspaces, "Start missing apps", "Apps that aren't open start, in the order above", "launch"),
    entry(
        Page::Workspaces,
        "Open with",
        "A path or link the app opens when a switch starts it",
        "folder file project vault url link argument start",
    ),
    entry(
        Page::Workspaces,
        "Bring back minimized windows",
        "A window saved minimized comes back, in its place",
        "minimized restore show all show every window",
    ),
    entry(
        Page::Workspaces,
        "Windows already open",
        "New windows always go to their place. Put windows back moves every window",
        "move leave stay where they are open",
    ),
    entry(Page::Workspaces, "Other windows", "Close asks each app first, so it can save", "close minimize leave"),
    entry(
        Page::Workspaces,
        "Turn off this app",
        "The app stays in the workspace, and a switch leaves it alone",
        "disable skip off on row",
    ),
    entry(Page::Workspaces, "Export workspace", "Save a workspace to a file", "import share backup"),
    entry(Page::Workspaces, "Add an app", "Add an open window or an installed app", "picker store start menu"),
    entry(
        Page::Workspaces,
        "Put apps in place when they open",
        "A window that opens goes where the active workspace puts it, once",
        "auto apply rules automatic open",
    ),
    entry(
        Page::Workspaces,
        "Apps that aren't open",
        "What Update from screen picks for each one first. You choose for each app when you update",
        "update capture remove closed delete keep turn off",
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
    entry(
        Page::Shortcuts,
        "Use suggested keys",
        "Give every shortcut with no keys one that doesn't clash",
        "hotkey keyboard suggest default fill not set record all",
    ),
    entry(Page::Shortcuts, "Line up screens", "Global shortcut", "hotkey keyboard"),
    entry(Page::Shortcuts, "Pause or resume pointer remap", "Global shortcut", "hotkey keyboard"),
    entry(
        Page::Shortcuts,
        "Pause Seam",
        "Pause or resume everything Seam does",
        "hotkey keyboard stop resume wait suspend",
    ),
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
    entry(
        Page::Shortcuts,
        "Quick layout",
        "Pick this screen's layout from the keyboard",
        "hotkey keyboard layout zones overlay switch screen",
    ),
    entry(
        Page::Shortcuts,
        "Size menu",
        "Share or take out the focused snapped window",
        "hotkey keyboard divider share half third quarter take out zone resize",
    ),
    entry(
        Page::Shortcuts,
        "Fit group to screen",
        "Stretch the focused window's group to fill its screen",
        "hotkey keyboard fit fill stretch group beside work area full screen",
    ),
    entry(
        Page::Shortcuts,
        "Next window in stack",
        "Put the next window of the focused window's stack on top",
        "hotkey keyboard stack cycle tab next zone behind",
    ),
    entry(
        Page::Shortcuts,
        "Previous window in stack",
        "Put the previous window of the focused window's stack on top",
        "hotkey keyboard stack cycle tab previous back zone behind",
    ),
    entry(
        Page::Shortcuts,
        "Float or tile this window",
        "Take the focused window out of smart tiling, or put it back in",
        "hotkey keyboard smart tiling float tile toggle",
    ),
    entry(
        Page::Shortcuts,
        "Free all windows",
        "Stop smart tiling on the screen under the pointer, and put its windows back",
        "hotkey keyboard smart tiling free release untile restore",
    ),
    entry(
        Page::Shortcuts,
        "Show overview",
        "Every window on every screen and desktop, to search and go to",
        "hotkey keyboard overview expose mission control all windows virtual desktop search switch preview",
    ),
    entry(
        Page::Windows,
        "Open the overview from a corner",
        "Rest the pointer in a corner of the whole desk, with no button down. Off, or pick the corner",
        "hot corner screen corner overview expose mission control pointer mouse rest",
    ),
    entry(
        Page::Shortcuts,
        "Rotate windows",
        "Move every snapped window on the screen under the pointer to the next zone. Shift added goes back",
        "hotkey keyboard rotate cycle shift swap zones windows next previous tiling",
    ),
    entry(
        Page::Shortcuts,
        "Scroll left",
        "On a Scrolling screen, focus the column on the left and scroll to it",
        "hotkey keyboard scrolling scroll strip column focus left previous",
    ),
    entry(
        Page::Shortcuts,
        "Scroll right",
        "On a Scrolling screen, focus the column on the right and scroll to it",
        "hotkey keyboard scrolling scroll strip column focus right next",
    ),
    entry(
        Page::Shortcuts,
        "Move column left",
        "Swap the focused window's column with the one on its left",
        "hotkey keyboard scrolling strip column move swap left",
    ),
    entry(
        Page::Shortcuts,
        "Move column right",
        "Swap the focused window's column with the one on its right",
        "hotkey keyboard scrolling strip column move swap right",
    ),
    entry(
        Page::Shortcuts,
        "Column width",
        "Give the focused window's column the next width: a third, a half, two thirds or full",
        "hotkey keyboard scrolling strip column width wider narrower size third half full",
    ),
    entry(
        Page::Shortcuts,
        "Reset sizes",
        "Put the screen under the pointer back to the layout it came from, or even out its tiles",
        "hotkey keyboard reset sizes proportions changed layout template restore even tiling",
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
    entry(
        Page::Shortcuts,
        "Minimize this workspace",
        "Minimize the windows of the workspace in use",
        "hotkey keyboard workspace minimize put away hide",
    ),
    entry(
        Page::Shortcuts,
        "Close this workspace",
        "Lists the windows of the workspace in use, then asks each to close",
        "hotkey keyboard workspace close quit exit shut",
    ),
    entry(
        Page::Shortcuts,
        "Add window to workspace",
        "Add the focused window to workspace 1 to 9, counted in the order of the Workspaces page",
        "hotkey keyboard workspace add send move window row numbered one two three",
    ),
    entry(Page::General, "Start with Windows", "Seam waits in the tray after you sign in", "login autostart"),
    entry(Page::General, "Offer to line up new screens", "Asks once when a screen Seam hasn't seen connects", ""),
    entry(
        Page::General,
        "When I sign in",
        "Nothing, offer my last session, or open a workspace",
        "session restore reboot sign out login startup workspace ask first",
    ),
    entry(
        Page::General,
        "Who reopens the apps",
        "Windows' Restart apps, or Seam, after you sign in",
        "session restore restart apps sign-in options twice",
    ),
    entry(
        Page::General,
        "Clicking the tray icon",
        "Right-click always shows the menu",
        "tray icon left-click open line up",
    ),
    entry(
        Page::Windows,
        "Step aside for games and full-screen apps",
        "Shortcuts, snapping and Spotlight wait until you leave it",
        "game fullscreen video presentation pause hotkeys",
    ),
    entry(Page::General, "Let other apps control Seam", "Stream Deck and tools on this PC", "api"),
    entry(
        Page::General,
        "Let assistants use Seam (MCP)",
        "Claude and other assistants can read status and switch workspaces",
        "mcp ai assistant claude chatgpt model context protocol",
    ),
    entry(
        Page::General,
        "Let assistants change workspaces",
        "Rename, edit and remove rows, each with an Undo notice",
        "mcp ai assistant edit undo",
    ),
    entry(Page::General, "seam-mcp.exe", "Copy the path to paste into an assistant", "mcp ai assistant path copy"),
    entry(Page::General, "Settings folder", "Export, import or open your settings", "config files"),
    entry(Page::General, "Debug log", "Writes remaps and screen changes to seam.log", "troubleshooting"),
    entry(Page::About, "Check for updates every day", "Last checked, and check now", "update version"),
    entry(Page::About, "Install updates on their own", "Only while you're away from the computer", "update"),
    entry(Page::About, "Include pre-releases", "Get new features early", "update beta"),
    entry(Page::About, "Copy diagnostics", "Your version, Windows build, screens and saved borders", "support"),
    entry(Page::About, "Report a problem", "Opens a new GitHub issue", "support bug"),
    entry(Page::About, "Source code", "github.com/F3NN3X/seam", "github repository open"),
    entry(Page::About, "Licenses", "Seam's licence and the open source parts it uses", "license open source"),
    entry(
        Page::Help,
        "Getting started",
        "Line up your screens, pick zone layouts, capture a workspace",
        "first steps begin tutorial",
    ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_is_on_exactly_one_card() {
        for topic in help::TOPICS {
            let cards = help::CARDS.iter().filter(|(_, titles)| titles.contains(&topic.title)).count();
            assert_eq!(cards, 1, "{}", topic.title);
        }
        for title in help::CARDS.iter().flat_map(|(_, titles)| titles.iter()) {
            assert!(help::position(title).is_some(), "{title} isn't a topic");
        }
    }

    #[test]
    fn a_topic_names_settings_and_topics_that_exist() {
        for topic in help::TOPICS {
            for setting in topic.settings {
                let found = ENTRIES.iter().any(|e| e.page == setting.page && e.title == setting.title);
                assert!(found, "{}: {} isn't a search entry of {}", topic.title, setting.title, setting.page.title());
            }
            for title in topic.related {
                assert!(help::position(title).is_some(), "{}: {title} isn't a topic", topic.title);
            }
        }
    }
}
