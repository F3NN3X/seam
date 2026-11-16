//! A set of screens Seam hasn't seen, worked out from what the panel already has (D673).

use super::*;

/// The tray's own rule for asking about a new desk (`src/app/report.rs`): nothing lined up yet,
/// asking is on, the screens touch and this set hasn't been seen.
fn needs_lining_up(lined_up: bool, ask: bool, touching: bool, seen: &[String], sig: &str) -> bool {
    !lined_up && ask && touching && !seen.iter().any(|s| s == sig)
}

impl Panel {
    /// The desk now is a set of screens Seam hasn't seen, and it would ask about it.
    pub fn new_screen(&self) -> bool {
        let Some(state) = &self.state else { return false };
        let touching = desk::has_neighbours(&self.mons);
        needs_lining_up(!self.links.is_empty(), self.config.ask_new_desks, touching, &self.config.seen, &state.desk.sig)
    }
}

#[cfg(test)]
mod tests {
    use super::needs_lining_up;

    #[test]
    fn a_new_touching_desk_with_nothing_lined_up_asks() {
        assert!(needs_lining_up(false, true, true, &["a|b".into()], "c|d"));
    }

    #[test]
    fn a_desk_that_was_seen_lined_up_or_not_asked_about_doesnt() {
        assert!(!needs_lining_up(false, true, true, &["c|d".into()], "c|d"));
        assert!(!needs_lining_up(true, true, true, &[], "c|d"));
        assert!(!needs_lining_up(false, false, true, &[], "c|d"));
        assert!(!needs_lining_up(false, true, false, &[], "c|d"));
    }
}
