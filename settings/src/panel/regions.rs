//! F6 and Shift+F6: focus moves between the title bar, the sidebar and the page.

use super::*;

/// The three parts of the window that F6 moves between, in the order it goes through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Region {
    TitleBar,
    Sidebar,
    Page,
}

impl Region {
    /// The region after this one, or before it when going `back`, round the ends.
    pub(super) fn step(self, back: bool) -> Region {
        match (self, back) {
            (Region::TitleBar, false) | (Region::Page, true) => Region::Sidebar,
            (Region::Sidebar, false) | (Region::TitleBar, true) => Region::Page,
            (Region::Page, false) | (Region::Sidebar, true) => Region::TitleBar,
        }
    }
}

impl Panel {
    /// The focus handle of the sidebar item for `page`, which keeps its focus between frames.
    pub(super) fn nav_handle(&self, page: Page) -> FocusHandle {
        self.nav_focus
            .iter()
            .find(|(p, _)| *p == page)
            .map(|(_, handle)| handle.clone())
            .unwrap_or_else(|| self.focus.clone())
    }

    /// The region that has focus. Anything outside the sidebar and the page is the title bar.
    fn region(&self, window: &Window, cx: &App) -> Region {
        if self.page_focus.contains_focused(window, cx) {
            Region::Page
        } else if self.sidebar_focus.contains_focused(window, cx) {
            Region::Sidebar
        } else {
            Region::TitleBar
        }
    }

    /// F6 moves to the next region, Shift+F6 to the one before.
    pub(super) fn cycle_region(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        // A dialog, the setup or a menu holds focus until it closes, so F6 doesn't take it away.
        let modal = self.setup.is_some() || window.has_active_dialog(cx) || window.has_active_sheet(cx);
        if modal || !self.focus.contains_focused(window, cx) {
            return;
        }
        match self.region(window, cx).step(back) {
            Region::TitleBar => self.focus_search(window, cx),
            Region::Sidebar => self.nav_handle(self.page).focus(window, cx),
            Region::Page => {
                // The page's own handle isn't a tab stop, so the next one is its first control.
                self.page_focus.focus(window, cx);
                window.focus_next(cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Region;

    #[test]
    fn f6_goes_through_the_title_bar_the_sidebar_and_the_page() {
        assert_eq!(Region::TitleBar.step(false), Region::Sidebar);
        assert_eq!(Region::Sidebar.step(false), Region::Page);
        assert_eq!(Region::Page.step(false), Region::TitleBar);
    }

    #[test]
    fn shift_f6_goes_the_other_way() {
        assert_eq!(Region::TitleBar.step(true), Region::Page);
        assert_eq!(Region::Page.step(true), Region::Sidebar);
        assert_eq!(Region::Sidebar.step(true), Region::TitleBar);
    }

    #[test]
    fn going_round_returns_to_where_it_began() {
        for start in [Region::TitleBar, Region::Sidebar, Region::Page] {
            assert_eq!(start.step(false).step(false).step(false), start);
            assert_eq!(start.step(true).step(true).step(true), start);
        }
    }
}
