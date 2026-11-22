//! Which rows differ from their default, for "Show changed settings".

use crate::panel::Page;
use seam::config::Config;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::LazyLock;

/// A row by page and title, and the config values it shows, as JSON pointers into the config.
/// A row with no line here (layouts, app rules, a workspace's own rows) can't say it changed.
const ROWS: &[(Page, &str, &[&str])] = &[
    (Page::Displays, "Remap pointer at borders", &["/enabled"]),
    (Page::Displays, "Also align Windows arrangement", &["/align_arrangement"]),
    (Page::Displays, "Type measurements in Line Up", &["/lineup_measure"]),
    (Page::Displays, "Edge resistance", &["/pointer/edge_resistance_px"]),
    (Page::Displays, "Glued edge push", &["/pointer/glue_push_px"]),
    (Page::Zones, "Zones on", &["/zones/enabled"]),
    (Page::Zones, "Show zones while dragging", &["/zones/show_key", "/zones/pause_to_snap"]),
    (Page::Zones, "Span zones", &["/zones/span_key"]),
    (Page::Zones, "Gap between zones", &["/zones/gap"]),
    (Page::Zones, "Keep windows in their zones", &["/zones/refit"]),
    (Page::Zones, "Restore size when dragged out", &["/zones/restore_size"]),
    (Page::Zones, "Resize neighbours with a window", &["/zones/resize_neighbours"]),
    (Page::Zones, "Handles between snapped windows", &["/zones/divider_handles"]),
    (Page::Zones, "Swap with the window already in a zone", &["/zones/swap_on_drop"]),
    (Page::Windows, "Put windows back when Seam quits", &["/zones/tiling_free_on_quit"]),
    (Page::Zones, "Snap beside a window", &["/zones/snap_beside"]),
    (Page::Zones, "Show keys while dragging", &["/zones/drag_hints"]),
    (Page::Windows, "Focus border", &["/windows/focus_border", "/windows/focus_color"]),
    (Page::Windows, "Pin border", &["/windows/pin_color"]),
    (Page::Windows, "Tell me when a pinned window closes", &["/windows/notify_pinned_close"]),
    (Page::Windows, "Pinned windows stay when I switch workspaces", &["/windows/pins_stay_on_switch"]),
    (Page::Windows, "Move and resize from anywhere in a window", &["/windows/alt_drag"]),
    (Page::Windows, "Open the overview from a corner", &["/windows/corner_overview"]),
    (Page::Windows, "When a window moves to another screen", &["/window_size_on_move"]),
    (Page::Windows, "Also when I drag a window to another screen", &["/window_size_on_drag"]),
    (Page::Windows, "Another window manager handles my windows", &["/windows/other_wm"]),
    (Page::Workspaces, "Put apps in place when they open", &["/auto_apply"]),
    (Page::Workspaces, "Apps that aren't open", &["/workspaces_update_removes_closed"]),
    (Page::Shortcuts, "Line up screens", &["/hotkeys/line_up"]),
    (Page::Shortcuts, "Pause or resume pointer remap", &["/hotkeys/pause"]),
    (Page::Shortcuts, "Pause Seam", &["/hotkeys/pause_seam"]),
    (Page::Shortcuts, "Move window left", &["/hotkeys/move_left"]),
    (Page::Shortcuts, "Move window right", &["/hotkeys/move_right"]),
    (Page::Shortcuts, "Move window up", &["/hotkeys/move_up"]),
    (Page::Shortcuts, "Move window down", &["/hotkeys/move_down"]),
    (Page::Shortcuts, "Focus window to the left", &["/hotkeys/focus_left"]),
    (Page::Shortcuts, "Focus window to the right", &["/hotkeys/focus_right"]),
    (Page::Shortcuts, "Focus window above", &["/hotkeys/focus_up"]),
    (Page::Shortcuts, "Focus window below", &["/hotkeys/focus_down"]),
    (Page::Shortcuts, "Swap with window to the left", &["/hotkeys/swap_left"]),
    (Page::Shortcuts, "Swap with window to the right", &["/hotkeys/swap_right"]),
    (Page::Shortcuts, "Swap with window above", &["/hotkeys/swap_up"]),
    (Page::Shortcuts, "Swap with window below", &["/hotkeys/swap_down"]),
    (Page::Shortcuts, "Focus on this window", &["/hotkeys/focus_only"]),
    (Page::Shortcuts, "Spotlight", &["/hotkeys/spotlight"]),
    (Page::Shortcuts, "Grow window into its neighbour", &["/hotkeys/grow"]),
    (Page::Shortcuts, "Shrink window", &["/hotkeys/shrink"]),
    (Page::Shortcuts, "Give window two thirds of its group", &["/hotkeys/priority"]),
    (Page::Shortcuts, "Tile this screen's windows", &["/hotkeys/tile_screen"]),
    (Page::Shortcuts, "Quick layout", &["/hotkeys/quick_layout"]),
    (Page::Shortcuts, "Size menu", &["/hotkeys/divider_menu"]),
    (Page::Shortcuts, "Fit group to screen", &["/hotkeys/fit_group"]),
    (Page::Shortcuts, "Next window in stack", &["/hotkeys/stack_next"]),
    (Page::Shortcuts, "Previous window in stack", &["/hotkeys/stack_prev"]),
    (Page::Shortcuts, "Float or tile this window", &["/hotkeys/float_tile"]),
    (Page::Shortcuts, "Free all windows", &["/hotkeys/free_tiling"]),
    (Page::Shortcuts, "Show overview", &["/hotkeys/overview"]),
    (Page::Shortcuts, "Rotate windows", &["/hotkeys/rotate"]),
    (Page::Shortcuts, "Reset sizes", &["/hotkeys/reset_sizes"]),
    (Page::Shortcuts, "Scroll left", &["/hotkeys/scroll_left"]),
    (Page::Shortcuts, "Scroll right", &["/hotkeys/scroll_right"]),
    (Page::Shortcuts, "Move column left", &["/hotkeys/column_left"]),
    (Page::Shortcuts, "Move column right", &["/hotkeys/column_right"]),
    (Page::Shortcuts, "Column width", &["/hotkeys/column_width"]),
    (Page::Shortcuts, "Pin window on top", &["/hotkeys/pin_top"]),
    (Page::Shortcuts, "Keep window out of workspace switches", &["/hotkeys/pin_sticky"]),
    (Page::Shortcuts, "Release pinned windows", &["/hotkeys/release_all"]),
    (Page::Shortcuts, "Move window to next zone", &["/hotkeys/next_zone"]),
    (Page::Shortcuts, "Move window to previous zone", &["/hotkeys/prev_zone"]),
    (Page::Shortcuts, "Capture current windows", &["/hotkeys/capture_workspace"]),
    (Page::Shortcuts, "Switch back to the previous workspace", &["/hotkeys/switch_back"]),
    (Page::Shortcuts, "Snap back to this workspace", &["/hotkeys/snap_back"]),
    (Page::Shortcuts, "Save windows to the active workspace", &["/hotkeys/quick_save"]),
    (Page::Shortcuts, "Minimize this workspace", &["/hotkeys/minimize_workspace"]),
    (Page::Shortcuts, "Close this workspace", &["/hotkeys/close_workspace"]),
    (Page::General, "Offer to line up new screens", &["/ask_new_desks"]),
    (Page::General, "Who reopens the apps", &["/restore_apps"]),
    (Page::General, "Clicking the tray icon", &["/tray_click"]),
    (Page::Windows, "Step aside for games and full-screen apps", &["/step_aside_fullscreen"]),
    (Page::General, "Let other apps control Seam", &["/api/enabled"]),
    (Page::General, "Let assistants use Seam (MCP)", &["/api/mcp"]),
    (Page::General, "Let assistants change workspaces", &["/api/assistants_edit"]),
    (Page::General, "Debug log", &["/debug_log"]),
    (Page::About, "Check for updates every day", &["/updates/auto_check"]),
    (Page::About, "Install updates on their own", &["/updates/auto_install"]),
    (Page::About, "Include pre-releases", &["/updates/prerelease"]),
];

static DEFAULT: LazyLock<Value> = LazyLock::new(|| serde_json::to_value(Config::default()).unwrap_or_default());

/// The titles of `page`'s rows whose value in `config` isn't the default.
pub fn titles(page: Page, config: &Config) -> HashSet<&'static str> {
    let Ok(now) = serde_json::to_value(config) else { return HashSet::new() };
    differing(page, &now, &DEFAULT)
}

fn differing(page: Page, now: &Value, default: &Value) -> HashSet<&'static str> {
    ROWS.iter()
        .filter(|(p, _, pointers)| *p == page && pointers.iter().any(|at| now.pointer(at) != default.pointer(at)))
        .map(|(_, title, _)| *title)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ENTRIES;

    #[test]
    fn a_default_config_has_nothing_changed() {
        for page in
            [Page::Displays, Page::Zones, Page::Windows, Page::Workspaces, Page::Shortcuts, Page::General, Page::About]
        {
            assert!(titles(page, &Config::default()).is_empty());
        }
    }

    #[test]
    fn a_changed_value_names_its_row_on_its_page_only() {
        let mut config = Config::default();
        config.zones.gap += 2;
        config.hotkeys.line_up = None;
        assert_eq!(titles(Page::Zones, &config), HashSet::from(["Gap between zones"]));
        assert_eq!(titles(Page::Shortcuts, &config), HashSet::from(["Line up screens"]));
        assert!(titles(Page::Displays, &config).is_empty());
    }

    #[test]
    fn every_row_is_a_search_entry_of_its_page() {
        for (page, title, _) in ROWS {
            let found = ENTRIES.iter().any(|e| e.page == *page && e.title == *title);
            assert!(found, "{} {title} isn't a search entry", page.title());
        }
    }

    #[test]
    fn every_pointer_reaches_a_value() {
        let default = serde_json::to_value(Config::default()).unwrap();
        for (page, title, pointers) in ROWS {
            for at in *pointers {
                assert!(default.pointer(at).is_some(), "{} {title}: {at}", page.title());
            }
        }
    }
}
