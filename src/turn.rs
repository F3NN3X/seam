//! Rotate windows: which zone each window of a screen goes to. Pure, so the window
//! worker's zones and a tiled screen's tiles share it, and it is tested here.

/// A window in one zone of a screen, or one tile of a tiled screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub hwnd: isize,
    pub zone: usize,
    /// Seam may not move it: pinned, sticky, on top, or an administrator's.
    pub stuck: bool,
}

/// Where each window that moves goes, as (window, zone). Every window goes to the next
/// zone in zone order, as the zone keys step, or the previous one, and the last wraps
/// to the first. Windows in one zone move together, as a stack does. A zone `blocked`
/// (a window over several zones, a tile nobody holds) or holding a stuck window is
/// left out, its windows stay, and the others step over it. Fewer than two zones
/// left to rotate through means nothing moves.
pub fn rotate(zones: usize, held: &[Held], blocked: &[usize], forward: bool) -> Vec<(isize, usize)> {
    let open = |z: &usize| !blocked.contains(z) && !held.iter().any(|h| h.zone == *z && h.stuck);
    let through: Vec<usize> = (0..zones).filter(open).collect();
    let n = through.len();
    if n < 2 {
        return Vec::new();
    }
    let step = |h: &Held| {
        let at = through.iter().position(|z| *z == h.zone)?;
        Some((h.hwnd, through[if forward { (at + 1) % n } else { (at + n - 1) % n }]))
    };
    held.iter().filter_map(step).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hwnd: isize, zone: usize) -> Held {
        Held { hwnd, zone, stuck: false }
    }

    #[test]
    fn every_window_takes_the_next_zone_and_the_last_wraps() {
        let held = [at(1, 0), at(2, 1), at(3, 2)];
        assert_eq!(rotate(4, &held, &[], true), vec![(1, 1), (2, 2), (3, 3)]);
        assert_eq!(rotate(4, &held, &[], false), vec![(1, 3), (2, 0), (3, 1)]);
        // With every zone held, the last goes round to the first.
        assert_eq!(rotate(3, &held, &[], true), vec![(1, 1), (2, 2), (3, 0)]);
    }

    #[test]
    fn a_stack_moves_as_one() {
        let held = [at(1, 0), at(2, 1), at(3, 1), at(4, 1)];
        assert_eq!(rotate(3, &held, &[], true), vec![(1, 1), (2, 2), (3, 2), (4, 2)]);
    }

    #[test]
    fn a_stuck_window_keeps_its_zone_and_the_others_step_over_it() {
        // Zone 1 holds a pinned window, and a window beside it that stays too.
        let held = [at(1, 0), Held { hwnd: 2, zone: 1, stuck: true }, at(3, 1), at(4, 2)];
        assert_eq!(rotate(4, &held, &[], true), vec![(1, 2), (4, 3)]);
        assert_eq!(rotate(4, &held, &[], false), vec![(1, 3), (4, 0)]);
    }

    #[test]
    fn a_blocked_zone_is_left_out() {
        // A window over zones 1 and 2, and a tile nobody holds.
        let held = [at(1, 0), at(2, 3)];
        assert_eq!(rotate(5, &held, &[1, 2], true), vec![(1, 3), (2, 4)]);
        assert_eq!(rotate(5, &held, &[1, 2, 4], true), vec![(1, 3), (2, 0)]);
    }

    #[test]
    fn nothing_moves_with_fewer_than_two_zones_to_rotate_through() {
        assert!(rotate(1, &[at(1, 0)], &[], true).is_empty());
        assert!(rotate(0, &[], &[], true).is_empty());
        assert!(rotate(3, &[at(1, 0), at(2, 1)], &[1, 2], true).is_empty());
        let stuck = [Held { hwnd: 1, zone: 0, stuck: true }, at(2, 1)];
        assert!(rotate(2, &stuck, &[], true).is_empty());
    }
}
