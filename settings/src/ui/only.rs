//! "Show changed settings": a page drawn with only the rows that differ from their default.
//!
//! Each row is known by its title, so it decides for itself. A card leaves itself out when none of
//! its rows show, and a section when its card does.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

struct Only {
    titles: Rc<HashSet<&'static str>>,
    /// Rows drawn on the page, in the cards and sections built so far.
    total: usize,
}

thread_local! {
    static ONLY: RefCell<Option<Only>> = const { RefCell::new(None) };
}

/// Draw the next page with only the rows titled `titles`, or with all of them for `None`.
pub fn show_only(titles: Option<Rc<HashSet<&'static str>>>) {
    ONLY.with(|o| *o.borrow_mut() = titles.map(|titles| Only { titles, total: 0 }));
}

/// Whether the row titled `title` is drawn. A row with no title isn't known to have changed.
pub fn row_shows(title: Option<&str>) -> bool {
    ONLY.with(|o| o.borrow().as_ref().is_none_or(|only| title.is_some_and(|t| only.titles.contains(t))))
}

/// Whether rows are being left out.
pub fn active() -> bool {
    ONLY.with(|o| o.borrow().is_some())
}

/// `count` rows were drawn.
pub fn drawn(count: usize) {
    ONLY.with(|o| {
        if let Some(only) = o.borrow_mut().as_mut() {
            only.total += count;
        }
    });
}

/// Whether the page drew no row at all. Never while all of them are drawn.
pub fn nothing_shown() -> bool {
    ONLY.with(|o| o.borrow().as_ref().is_some_and(|only| only.total == 0))
}
