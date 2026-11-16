//! "Show changed settings": a page drawn with only the rows that differ from their default.
//!
//! Rows are built before the card and section that hold them, so each row notes whether it
//! shows, and the card that comes next takes its rows' notes in the order they were made.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};

struct Only {
    titles: HashSet<&'static str>,
    /// Whether each row built since the last card shows.
    rows: VecDeque<bool>,
    /// Whether the last card has a row that shows, until the section around it reads it.
    card: Option<bool>,
}

thread_local! {
    static ONLY: RefCell<Option<Only>> = const { RefCell::new(None) };
}

/// Draw the next page with only the rows titled `titles`, or with all of them for `None`.
pub fn show_only(titles: Option<HashSet<&'static str>>) {
    ONLY.with(|o| *o.borrow_mut() = titles.map(|titles| Only { titles, rows: VecDeque::new(), card: None }));
}

/// Whether the row titled `title` is drawn.
pub fn row_shows(title: &str) -> bool {
    ONLY.with(|o| {
        let mut o = o.borrow_mut();
        let Some(only) = o.as_mut() else { return true };
        let shows = only.titles.contains(title);
        only.rows.push_back(shows);
        shows
    })
}

/// Which of a card's `count` rows are drawn. `None` draws them all: nothing is hidden, or the
/// card holds rows this didn't see being made.
pub fn card_shows(count: usize) -> Option<Vec<bool>> {
    ONLY.with(|o| {
        let mut o = o.borrow_mut();
        let only = o.as_mut()?;
        if only.rows.len() < count {
            only.rows.clear();
            return None;
        }
        let shown: Vec<bool> = only.rows.drain(..count).collect();
        only.card = Some(shown.contains(&true));
        Some(shown)
    })
}

/// Whether the section around the card just made is drawn. A section with no card always is.
pub fn section_shows() -> bool {
    ONLY.with(|o| o.borrow_mut().as_mut().and_then(|only| only.card.take()).unwrap_or(true))
}
