//! What the drag-keys hook decides, with no Win32 in it: what to do with a key, and
//! how the hook's thread lingers, is taken over and goes. The hook and its thread
//! keep the state in atomics and ask these.

use super::Key;

/// A press of a key whose press was swallowed is a repeat when it comes this soon
/// after the last one: Windows' longest repeat delay is 1 s, with room here, and
/// repeats come faster. A later press is a new one, so its release was lost (a
/// secure desktop took it), and it is looked at afresh.
pub const REPEAT_MS: u32 = 1_500;

/// Set in the idle word once the hook's thread decided to go.
pub const GONE: u64 = 1 << 63;

/// One key event as the hook sees it.
#[derive(Clone, Copy)]
pub struct KeyIn {
    pub up: bool,
    /// Its press was swallowed and its release hasn't come.
    pub held: bool,
    /// Since the last press of this key, in ms.
    pub since: u32,
    /// The drag this hook serves shows zones, and the key is one it takes.
    pub ours: bool,
    /// Ctrl, Alt or Win held makes it the app's.
    pub foreign: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Swallow,
    /// Swallow it and post it to the drag.
    Post,
}

/// What to do with a key, and whether its press is swallowed after it.
pub fn key(e: KeyIn) -> (Verdict, bool) {
    if e.up {
        return (if e.held { Verdict::Swallow } else { Verdict::Pass }, false);
    }
    if e.held && e.since <= REPEAT_MS {
        // It steered once; its repeats are swallowed, even once the drag is over.
        return (Verdict::Swallow, true);
    }
    if e.ours && !e.foreign { (Verdict::Post, true) } else { (Verdict::Pass, false) }
}

/// Whether the drag takes this key: every steering key, or Esc only for a divider handle's drag.
pub fn ours(key: Option<Key>, esc_only: bool) -> bool {
    key.is_some_and(|k| k == Key::Cancel || !esc_only)
}

/// Ctrl, Alt or Win is held and isn't the drag's own key, so what comes with it is
/// the app's: Ctrl+wheel zoom. Tab with Alt held is Windows' Alt+Tab, even when Alt
/// is the drag's own key. Each modifier is (held, the drag's own).
pub fn foreign(tab: bool, ctrl: (bool, bool), alt: (bool, bool), win: (bool, bool)) -> bool {
    let theirs = |(held, own): (bool, bool)| held && !own;
    theirs(ctrl) || theirs(win) || (alt.0 && (!alt.1 || tab))
}

/// The drag of the hook for `thread` ended: the idle word says it may be taken over.
/// None when the thread already decided to go, so a new drag gets a new hook.
pub fn ended(word: u64, thread: u64) -> Option<u64> {
    (word != thread | GONE).then_some(thread)
}

/// A new drag takes over the hook idle for `thread`, the one wanted now.
pub fn take_over(word: u64, thread: u64) -> Option<u64> {
    (thread != 0 && word == thread).then_some(0)
}

/// The hook's thread goes, unless a new drag took it over meanwhile.
pub fn go(word: u64, thread: u64) -> Option<u64> {
    (word == thread).then_some(thread | GONE)
}

/// The id of the next drag: never 0, which a key from no drag would carry.
pub fn next_drag(last: u64) -> u64 {
    last.wrapping_add(1).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterLinger {
    /// A new drag took the hook over.
    Taken,
    /// Nothing is owed: the hook goes, if it wins `go`.
    Go,
    /// A key is still held: its release ends the hook, or the hold's end does.
    Hold,
}

pub fn linger_ended(idle_is_mine: bool, owes: bool) -> AfterLinger {
    match (idle_is_mine, owes) {
        (false, _) => AfterLinger::Taken,
        (true, false) => AfterLinger::Go,
        (true, true) => AfterLinger::Hold,
    }
}

/// A takeover pushes the 60 s cap back once per hook, so a run of drags can't keep
/// it in for ever.
pub fn extend_cap(extended: bool) -> bool {
    !extended
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: (bool, bool) = (false, false);

    fn press(held: bool, since: u32, ours: bool) -> KeyIn {
        KeyIn { up: false, held, since, ours, foreign: false }
    }

    fn release(held: bool) -> KeyIn {
        KeyIn { up: true, held, since: 0, ours: true, foreign: false }
    }

    #[test]
    fn a_swallowed_press_has_its_repeats_and_release_swallowed() {
        let (v, held) = key(press(false, 0, true));
        assert_eq!((v, held), (Verdict::Post, true));
        // Repeats, even once the drag is over and the key isn't ours any more.
        assert_eq!(key(press(held, 30, false)), (Verdict::Swallow, true));
        assert_eq!(key(release(held)), (Verdict::Swallow, false));
    }

    #[test]
    fn a_release_whose_press_passed_passes() {
        assert_eq!(key(release(false)), (Verdict::Pass, false));
        let (v, held) = key(press(false, 0, false));
        assert_eq!((v, held), (Verdict::Pass, false));
    }

    #[test]
    fn a_press_long_after_a_lost_release_is_new() {
        // Ours: steers again. Not ours: the app's, and nothing is held any more.
        assert_eq!(key(press(true, REPEAT_MS + 1, true)), (Verdict::Post, true));
        assert_eq!(key(press(true, REPEAT_MS + 1, false)), (Verdict::Pass, false));
    }

    #[test]
    fn a_modifier_not_the_drags_hands_the_key_back() {
        let e = KeyIn { foreign: true, ..press(false, 0, true) };
        assert_eq!(key(e), (Verdict::Pass, false));
    }

    #[test]
    fn a_divider_drag_takes_esc_only() {
        assert!(ours(Some(Key::Cancel), true));
        assert!(!ours(Some(Key::Next), true));
        assert!(!ours(Some(Key::Scroll(true)), true));
        assert!(ours(Some(Key::Next), false));
        assert!(!ours(None, false));
    }

    #[test]
    fn alt_tab_goes_to_windows_even_when_alt_is_the_drags_key() {
        // Alt is the drag's: the wheel and 1 steer, Tab is Alt+Tab.
        assert!(!foreign(false, OFF, (true, true), OFF));
        assert!(foreign(true, OFF, (true, true), OFF));
        // Alt held and not the drag's: all of it is the app's.
        assert!(foreign(false, OFF, (true, false), OFF));
        // Nothing held: Tab steers.
        assert!(!foreign(true, OFF, OFF, OFF));
    }

    #[test]
    fn a_held_modifier_steers_only_when_it_is_the_drags_own() {
        assert!(foreign(false, (true, false), OFF, OFF));
        assert!(!foreign(false, (true, true), OFF, OFF));
        // Win as the Alt+drag's key.
        assert!(foreign(true, OFF, OFF, (true, false)));
        assert!(!foreign(true, OFF, OFF, (true, true)));
    }

    #[test]
    fn a_drag_in_the_linger_takes_the_hook_over_with_a_new_id() {
        let idle = ended(0, 7).unwrap();
        assert_eq!(take_over(idle, 7), Some(0));
        assert_ne!(next_drag(3), 3);
        assert_eq!(next_drag(u64::MAX), 1);
        // Taken over, so the linger's end and the thread's go find it busy.
        assert_eq!(linger_ended(false, false), AfterLinger::Taken);
        assert_eq!(go(0, 7), None);
        // Another thread's idle word isn't this one's to take.
        assert_eq!(take_over(idle, 8), None);
    }

    #[test]
    fn a_hook_that_went_is_not_taken_over_or_made_idle_again() {
        let gone = go(7, 7).unwrap();
        assert_eq!(take_over(gone, 7), None);
        assert_eq!(ended(gone, 7), None);
        // A later thread's drag ends as usual.
        assert_eq!(ended(gone, 8), Some(8));
    }

    #[test]
    fn the_linger_ends_in_go_or_a_hold_for_a_held_key() {
        assert_eq!(linger_ended(true, false), AfterLinger::Go);
        assert_eq!(linger_ended(true, true), AfterLinger::Hold);
    }

    #[test]
    fn a_takeover_extends_the_cap_once() {
        assert!(extend_cap(false));
        assert!(!extend_cap(true));
    }
}
