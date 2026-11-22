//! Every shortcut the page lists, with its place on the page and in "Not set yet".

use crate::pages::workspaces;
use crate::panel::Panel;
use seam::config::Hotkeys;

/// The Windows rows, in `Hotkeys::directional` order. Search lists the same titles.
const WINDOW_TITLES: [&str; 12] = [
    "Move window left",
    "Move window right",
    "Move window up",
    "Move window down",
    "Focus window to the left",
    "Focus window to the right",
    "Focus window above",
    "Focus window below",
    "Swap with window to the left",
    "Swap with window to the right",
    "Swap with window above",
    "Swap with window below",
];

/// The keys "Use suggested keys" tries first for an action, before any free one.
const PREFERRED: &[(&str, &str)] = &[
    ("pause_seam", "Ctrl+Shift+Alt+Space"),
    ("next_zone", "Ctrl+Shift+Alt+PageDown"),
    ("prev_zone", "Ctrl+Shift+Alt+PageUp"),
    ("swap_left", "Ctrl+Win+Alt+Left"),
    ("swap_right", "Ctrl+Win+Alt+Right"),
    ("swap_up", "Ctrl+Win+Alt+Up"),
    ("swap_down", "Ctrl+Win+Alt+Down"),
    ("focus_only", "Ctrl+Shift+Alt+F"),
    ("spotlight", "Ctrl+Shift+Alt+S"),
    ("tile_screen", "Ctrl+Shift+Alt+T"),
    ("quick_layout", "Ctrl+Shift+Alt+Q"),
    ("rotate", "Ctrl+Shift+Alt+R"),
    ("overview", "Ctrl+Shift+Alt+O"),
    ("capture_workspace", "Ctrl+Shift+Alt+C"),
    ("snap_back", "Ctrl+Shift+Alt+B"),
    ("quick_save", "Ctrl+Shift+Alt+W"),
];

/// The four sections of the page, which "In use" and "All" both list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Displays,
    Windows,
    Zones,
    Workspaces,
}

impl Section {
    pub const ALL: [Section; 4] = [Section::Displays, Section::Windows, Section::Zones, Section::Workspaces];

    pub fn title(self) -> &'static str {
        match self {
            Section::Displays => "Displays",
            Section::Windows => "Windows",
            Section::Zones => "Zones",
            Section::Workspaces => "Workspaces",
        }
    }
}

/// The groups "Not set yet" folds its shortcuts into.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Displays,
    SwapAndFocus,
    Zones,
    Sizes,
    Stacks,
    Scrolling,
    Pins,
    Workspaces,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Displays,
        Group::SwapAndFocus,
        Group::Zones,
        Group::Sizes,
        Group::Stacks,
        Group::Scrolling,
        Group::Pins,
        Group::Workspaces,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::Displays => "Displays",
            Group::SwapAndFocus => "Swap and focus",
            Group::Zones => "Zones",
            Group::Sizes => "Sizes and tiling",
            Group::Stacks => "Stacks and overview",
            Group::Scrolling => "Scrolling screens",
            Group::Pins => "Pins",
            Group::Workspaces => "Workspaces",
        }
    }

    /// What's inside, for the line under the group's name.
    pub fn names(self) -> &'static str {
        match self {
            Group::Displays => "Pause Seam",
            Group::SwapAndFocus => {
                "Swap with the window to the left, right, above or below, Focus on this window, Spotlight"
            }
            Group::Zones => "Move window to next zone, Move window to previous zone",
            Group::Sizes => "Grow, Shrink, Tile this screen's windows, Quick layout, Rotate windows and 6 more",
            Group::Stacks => "Next and previous window in stack, Show overview",
            Group::Scrolling => "Scroll left and right, Move column left and right, Column width",
            Group::Pins => "Pin window on top, Keep window out of workspace switches, Release pinned windows",
            Group::Workspaces => {
                "Switch to each workspace, Capture current windows, Add window to workspace 1 to 9 and 5 more"
            }
        }
    }
}

/// One shortcut: what it's called, which setting holds it, and where it sits.
#[derive(Clone)]
pub struct Item {
    pub title: String,
    /// The key of `hotkeys` in the config, or `workspace:` and the workspace's id.
    pub action: String,
    pub combo: Option<String>,
    /// Numbers the row, so its buttons get ids of their own.
    pub slot: usize,
    pub section: Section,
    pub group: Group,
    /// The keys "Use suggested keys" tries first.
    pub preferred: Option<String>,
}

impl Item {
    /// Whether its name holds `query`, or every word of it names one of its keys ("alt l").
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() || self.title.to_lowercase().contains(&query) {
            return true;
        }
        let keys: Vec<String> = self.combo.iter().flat_map(|c| c.split('+')).map(str::to_lowercase).collect();
        let words: Vec<&str> = query.split(|c: char| c.is_whitespace() || c == '+').filter(|w| !w.is_empty()).collect();
        !keys.is_empty() && !words.is_empty() && words.iter().all(|w| keys.iter().any(|k| k == w))
    }
}

/// The page's shortcuts in the order it lists them, workspaces last.
pub fn items(panel: &Panel) -> Vec<Item> {
    let keys = &panel.config.hotkeys;
    let mut out = Vec::new();
    let mut add =
        |title: &str, action: &str, combo: Option<&String>, slot: usize, (section, group): (Section, Group)| {
            let preferred = PREFERRED.iter().find(|(a, _)| *a == action).map(|(_, combo)| combo.to_string());
            out.push(Item {
                title: title.to_string(),
                action: action.to_string(),
                combo: combo.cloned(),
                slot,
                section,
                group,
                preferred,
            });
        };
    let displays = (Section::Displays, Group::Displays);
    add("Line up screens", "line_up", keys.line_up.as_ref(), 0, displays);
    add("Pause or resume pointer remap", "pause", keys.pause.as_ref(), 1, displays);
    add("Pause Seam", "pause_seam", keys.pause_seam.as_ref(), 950, displays);
    windows(keys, &mut add);
    let zones = (Section::Zones, Group::Zones);
    add("Move window to next zone", "next_zone", keys.next_zone.as_ref(), 2, zones);
    add("Move window to previous zone", "prev_zone", keys.prev_zone.as_ref(), 3, zones);
    // One row for each workspace on this desk, then the capture shortcut.
    let on_desk = workspaces::on_desk(panel);
    let section = (Section::Workspaces, Group::Workspaces);
    for (i, ws) in on_desk.iter().enumerate() {
        add(&ws.name, &format!("workspace:{}", ws.id), ws.hotkey.as_ref(), 20 + i, section);
    }
    add("Capture current windows", "capture_workspace", keys.capture_workspace.as_ref(), 4, section);
    // Slots past the other workspace rows, which stop before 100.
    for (n, combo) in keys.add_to_workspace().into_iter().enumerate() {
        add(&format!("Add window to workspace {}", n + 1), &format!("add_to_{}", n + 1), combo, 1000 + n, section);
    }
    add("Snap back to this workspace", "snap_back", keys.snap_back.as_ref(), 20 + on_desk.len(), section);
    add("Save windows to the active workspace", "quick_save", keys.quick_save.as_ref(), 21 + on_desk.len(), section);
    add("Switch back to the previous workspace", "switch_back", keys.switch_back.as_ref(), 22 + on_desk.len(), section);
    add("Minimize this workspace", "minimize_workspace", keys.minimize_workspace.as_ref(), 23 + on_desk.len(), section);
    add("Close this workspace", "close_workspace", keys.close_workspace.as_ref(), 24 + on_desk.len(), section);
    // Workspaces and "Add window to workspace" are numbered, so their keys are too.
    for (n, item) in out.iter_mut().filter(|i| i.action.starts_with("workspace:")).enumerate() {
        item.preferred = (n < 9).then(|| format!("Ctrl+Shift+Alt+{}", n + 1));
    }
    for item in &mut out {
        if let Some(n) = item.action.strip_prefix("add_to_") {
            item.preferred = Some(format!("Shift+Win+Alt+{n}"));
        }
    }
    out
}

/// The Windows section's rows. Slots far past the workspace rows, which count up from 20.
fn windows(keys: &Hotkeys, add: &mut impl FnMut(&str, &str, Option<&String>, usize, (Section, Group))) {
    let at = |group| (Section::Windows, group);
    for (i, ((verb, dir, combo), title)) in keys.directional().into_iter().zip(WINDOW_TITLES).enumerate() {
        add(title, &format!("{verb}_{dir}"), combo, 5 + i, at(Group::SwapAndFocus));
    }
    add("Focus on this window", "focus_only", keys.focus_only.as_ref(), 910, at(Group::SwapAndFocus));
    add("Spotlight", "spotlight", keys.spotlight.as_ref(), 911, at(Group::SwapAndFocus));
    let sizes = at(Group::Sizes);
    add("Grow window into its neighbour", "grow", keys.grow.as_ref(), 900, sizes);
    add("Shrink window", "shrink", keys.shrink.as_ref(), 901, sizes);
    add("Give window two thirds of its group", "priority", keys.priority.as_ref(), 902, sizes);
    add("Tile this screen's windows", "tile_screen", keys.tile_screen.as_ref(), 920, sizes);
    add("Quick layout", "quick_layout", keys.quick_layout.as_ref(), 930, sizes);
    add("Size menu", "divider_menu", keys.divider_menu.as_ref(), 940, sizes);
    add("Fit group to screen", "fit_group", keys.fit_group.as_ref(), 960, sizes);
    let stacks = at(Group::Stacks);
    add("Next window in stack", "stack_next", keys.stack_next.as_ref(), 970, stacks);
    add("Previous window in stack", "stack_prev", keys.stack_prev.as_ref(), 971, stacks);
    add("Float or tile this window", "float_tile", keys.float_tile.as_ref(), 980, sizes);
    add("Free all windows", "free_tiling", keys.free_tiling.as_ref(), 981, sizes);
    add("Show overview", "overview", keys.overview.as_ref(), 990, stacks);
    add("Rotate windows", "rotate", keys.rotate.as_ref(), 991, sizes);
    add("Reset sizes", "reset_sizes", keys.reset_sizes.as_ref(), 992, sizes);
    let scrolling = at(Group::Scrolling);
    add("Scroll left", "scroll_left", keys.scroll_left.as_ref(), 993, scrolling);
    add("Scroll right", "scroll_right", keys.scroll_right.as_ref(), 994, scrolling);
    add("Move column left", "column_left", keys.column_left.as_ref(), 995, scrolling);
    add("Move column right", "column_right", keys.column_right.as_ref(), 996, scrolling);
    add("Column width", "column_width", keys.column_width.as_ref(), 997, scrolling);
    let pins = at(Group::Pins);
    add("Pin window on top", "pin_top", keys.pin_top.as_ref(), 17, pins);
    add("Keep window out of workspace switches", "pin_sticky", keys.pin_sticky.as_ref(), 18, pins);
    add("Release pinned windows", "release_all", keys.release_all.as_ref(), 19, pins);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, combo: Option<&str>) -> Item {
        Item {
            title: title.into(),
            action: "x".into(),
            combo: combo.map(String::from),
            slot: 0,
            section: Section::Windows,
            group: Group::Sizes,
            preferred: None,
        }
    }

    #[test]
    fn a_filter_matches_the_name_or_the_key_names() {
        let it = item("Spotlight", Some("Ctrl+Shift+Alt+L"));
        assert!(it.matches("spot"));
        assert!(it.matches("alt l"));
        assert!(it.matches("  CTRL+shift "));
        assert!(!it.matches("alt k"));
        assert!(!item("Spotlight", None).matches("alt"));
        assert!(it.matches(""));
    }

    #[test]
    fn a_key_name_has_to_match_whole() {
        assert!(!item("Move", Some("Win+Left")).matches("lef"));
        assert!(item("Move", Some("Win+Left")).matches("win left"));
    }
}
