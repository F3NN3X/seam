//! What the pointer hook decides, with no Win32 in it: which corners of the desk are
//! outer ones, whether the pointer is in one, and when a rest there opens the overview.
//! The hook and its thread keep the state and ask these.

use seam::geom::Rect;

/// How far from the corner the pointer may rest, in pixels at 100% scale.
pub const REACH_PX: i32 = 4;
/// How long the pointer rests in the corner before the overview opens.
pub const REST_MS: u32 = 400;

/// The corner of the desk that opens the overview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    /// From the config's name. Anything else reads as top left.
    pub fn from_name(name: &str) -> Corner {
        match name {
            "top_right" => Corner::TopRight,
            "bottom_left" => Corner::BottomLeft,
            "bottom_right" => Corner::BottomRight,
            _ => Corner::TopLeft,
        }
    }

    /// The way out of the screen at this corner, -1 or 1 on each axis.
    fn out(self) -> (i32, i32) {
        match self {
            Corner::TopLeft => (-1, -1),
            Corner::TopRight => (1, -1),
            Corner::BottomLeft => (-1, 1),
            Corner::BottomRight => (1, 1),
        }
    }
}

/// The square the pointer rests in at one corner, in physical pixels: left and top
/// inside, right and bottom outside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spot {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Spot {
    pub fn contains(&self, (x, y): (i32, i32)) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The chosen corner of each screen, where it is an outer corner of the desk: no screen
/// holds the pixel past it sideways, up or down, or diagonally. A corner another screen
/// touches is where the pointer crosses, so it never counts. `screens` are the monitor
/// rects in physical pixels, each with its DPI, which sizes its spot.
pub fn outer_corners(screens: &[(Rect, u32)], corner: Corner) -> Vec<Spot> {
    let (dx, dy) = corner.out();
    let held = |x: i32, y: i32| screens.iter().any(|(r, _)| r.contains(x, y));
    screens
        .iter()
        .filter(|(r, _)| r.width() > 0 && r.height() > 0)
        .filter_map(|&(r, dpi)| {
            let x = if dx < 0 { r.left } else { r.right - 1 };
            let y = if dy < 0 { r.top } else { r.bottom - 1 };
            if held(x + dx, y) || held(x, y + dy) || held(x + dx, y + dy) {
                return None;
            }
            let reach = (REACH_PX * dpi.max(96) as i32 / 96).min(r.width()).min(r.height());
            let (left, right) = if dx < 0 { (r.left, r.left + reach) } else { (r.right - reach, r.right) };
            let (top, bottom) = if dy < 0 { (r.top, r.top + reach) } else { (r.bottom - reach, r.bottom) };
            Some(Spot { left, top, right, bottom })
        })
        .collect()
}

/// Whether the point is in any of the spots.
pub fn in_corner(spots: &[Spot], pt: (i32, i32)) -> bool {
    spots.iter().any(|s| s.contains(pt))
}

/// Where the pointer is with the corner, between moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rest {
    /// Out of the corner: the next time in, it waits again.
    #[default]
    Away,
    /// In the corner since this tick, by the hook's millisecond clock.
    Waiting(u32),
    /// The rest opened the overview, or a button went down in the corner: nothing more
    /// until the pointer leaves.
    Spent,
}

/// What the hook's thread does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// Look again in this many milliseconds.
    Wait(u32),
    /// Tell the tray to open the overview.
    Open,
}

impl Rest {
    /// The pointer moved, or a button went down, at `now`. `inside` says whether it is in
    /// the corner and `button` whether a button is down. Leaving the corner arms it again.
    pub fn moved(self, inside: bool, button: bool, now: u32) -> (Rest, Step) {
        match self {
            _ if !inside => (Rest::Away, Step::Nothing),
            // A drag or a click into the corner isn't a rest.
            _ if button => (Rest::Spent, Step::Nothing),
            Rest::Away => (Rest::Waiting(now), Step::Wait(REST_MS)),
            rest => (rest, Step::Nothing),
        }
    }

    /// The wait is up, at `now`: open once the pointer has rested long enough with no
    /// button down, or wait the rest of the time.
    pub fn tick(self, button: bool, now: u32) -> (Rest, Step) {
        match self {
            Rest::Waiting(_) if button => (Rest::Spent, Step::Nothing),
            Rest::Waiting(since) => {
                let rested = now.wrapping_sub(since);
                if rested >= REST_MS { (Rest::Spent, Step::Open) } else { (self, Step::Wait(REST_MS - rested)) }
            }
            rest => (rest, Step::Nothing),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(left: i32, top: i32, w: i32, h: i32) -> (Rect, u32) {
        (Rect::new(left, top, w, h), 96)
    }

    fn spot(left: i32, top: i32, right: i32, bottom: i32) -> Spot {
        Spot { left, top, right, bottom }
    }

    #[test]
    fn side_by_side_screens_keep_only_the_corners_at_the_far_ends() {
        let desk = [screen(0, 0, 1920, 1080), screen(1920, 0, 1920, 1080)];
        assert_eq!(outer_corners(&desk, Corner::TopLeft), vec![spot(0, 0, 4, 4)]);
        assert_eq!(outer_corners(&desk, Corner::TopRight), vec![spot(3836, 0, 3840, 4)]);
        assert_eq!(outer_corners(&desk, Corner::BottomLeft), vec![spot(0, 1076, 4, 1080)]);
        assert_eq!(outer_corners(&desk, Corner::BottomRight), vec![spot(3836, 1076, 3840, 1080)]);
    }

    #[test]
    fn a_corner_past_a_shorter_neighbour_is_still_outer() {
        // The right screen sits 200 px lower, so the bottom of its left edge faces nothing.
        let desk = [screen(0, 0, 1920, 1080), screen(1920, 200, 1920, 1080)];
        assert_eq!(outer_corners(&desk, Corner::TopLeft), vec![spot(0, 0, 4, 4)]);
        assert_eq!(
            outer_corners(&desk, Corner::BottomLeft),
            vec![spot(0, 1076, 4, 1080), spot(1920, 1276, 1924, 1280)]
        );
        // The left screen's top right faces nothing either, while its bottom right
        // touches the right screen's edge.
        assert_eq!(outer_corners(&desk, Corner::TopRight), vec![spot(1916, 0, 1920, 4), spot(3836, 200, 3840, 204)]);
        assert_eq!(outer_corners(&desk, Corner::BottomRight), vec![spot(3836, 1276, 3840, 1280)]);
    }

    #[test]
    fn an_l_shape_drops_the_inner_corner() {
        // Two side by side, a third under the left one.
        let desk = [screen(0, 0, 1920, 1080), screen(1920, 0, 1920, 1080), screen(0, 1080, 1920, 1080)];
        // The left screen's top left only: the others touch a neighbour there.
        assert_eq!(outer_corners(&desk, Corner::TopLeft), vec![spot(0, 0, 4, 4)]);
        // The top-left screen's bottom right touches both others.
        assert_eq!(
            outer_corners(&desk, Corner::BottomRight),
            vec![spot(3836, 1076, 3840, 1080), spot(1916, 2156, 1920, 2160)]
        );
        assert_eq!(outer_corners(&desk, Corner::BottomLeft), vec![spot(0, 2156, 4, 2160)]);
        // The inner corner of the L, the bottom screen's top right, faces the screen above it.
        assert_eq!(outer_corners(&desk, Corner::TopRight), vec![spot(3836, 0, 3840, 4)]);
    }

    #[test]
    fn stacked_screens_share_no_corner() {
        let desk = [screen(0, 0, 2560, 1440), screen(0, 1440, 2560, 1440)];
        assert_eq!(outer_corners(&desk, Corner::TopLeft), vec![spot(0, 0, 4, 4)]);
        assert_eq!(outer_corners(&desk, Corner::BottomLeft), vec![spot(0, 2876, 4, 2880)]);
        assert_eq!(outer_corners(&desk, Corner::TopRight), vec![spot(2556, 0, 2560, 4)]);
    }

    #[test]
    fn screens_meeting_only_at_a_corner_both_lose_it() {
        let desk = [screen(0, 0, 1920, 1080), screen(1920, 1080, 1920, 1080)];
        assert!(!outer_corners(&desk, Corner::BottomRight).contains(&spot(1916, 1076, 1920, 1080)));
        assert!(!outer_corners(&desk, Corner::TopLeft).contains(&spot(1920, 1080, 1924, 1084)));
    }

    #[test]
    fn mixed_dpi_scales_each_spot_with_its_screen() {
        // A 4K screen at 200% left of a 1080p one at 100%, in physical pixels.
        let desk = [(Rect::new(-3840, 0, 3840, 2160), 192), (Rect::new(0, 0, 1920, 1080), 96)];
        assert_eq!(outer_corners(&desk, Corner::TopLeft), vec![spot(-3840, 0, -3832, 8)]);
        assert_eq!(outer_corners(&desk, Corner::BottomLeft), vec![spot(-3840, 2152, -3832, 2160)]);
        // The big screen's bottom right reaches below the small one, so both are outer.
        let bottom_right = vec![spot(-8, 2152, 0, 2160), spot(1916, 1076, 1920, 1080)];
        assert_eq!(outer_corners(&desk, Corner::BottomRight), bottom_right);
        assert_eq!(outer_corners(&desk, Corner::TopRight), vec![spot(1916, 0, 1920, 4)]);
    }

    #[test]
    fn the_corner_zone_is_a_few_pixels_from_the_corner() {
        let spots = outer_corners(&[screen(0, 0, 1920, 1080)], Corner::BottomRight);
        assert!(in_corner(&spots, (1919, 1079)));
        assert!(in_corner(&spots, (1916, 1076)));
        assert!(!in_corner(&spots, (1915, 1079)));
        assert!(!in_corner(&spots, (1919, 1075)));
        assert!(!in_corner(&spots, (1920, 1080)));
        assert!(!in_corner(&[], (0, 0)));
    }

    #[test]
    fn a_rest_opens_once_after_the_wait() {
        let (rest, step) = Rest::Away.moved(true, false, 1_000);
        assert_eq!((rest, step), (Rest::Waiting(1_000), Step::Wait(REST_MS)));
        // Small moves inside keep the wait running from the first.
        assert_eq!(rest.moved(true, false, 1_200), (Rest::Waiting(1_000), Step::Nothing));
        // An early timer waits out the rest.
        assert_eq!(rest.tick(false, 1_300), (rest, Step::Wait(100)));
        let (rest, step) = rest.tick(false, 1_400);
        assert_eq!((rest, step), (Rest::Spent, Step::Open));
        // Resting on opens nothing more.
        assert_eq!(rest.moved(true, false, 2_000), (Rest::Spent, Step::Nothing));
        assert_eq!(rest.tick(false, 3_000), (Rest::Spent, Step::Nothing));
    }

    #[test]
    fn leaving_the_corner_cancels_and_arms_it_again() {
        let (rest, _) = Rest::Away.moved(true, false, 0);
        let (rest, step) = rest.moved(false, false, 100);
        assert_eq!((rest, step), (Rest::Away, Step::Nothing));
        assert_eq!(rest.tick(false, 400), (Rest::Away, Step::Nothing));
        // After an open, leaving and coming back waits again.
        let (rest, _) = Rest::Spent.moved(false, false, 500);
        assert_eq!(rest.moved(true, false, 600), (Rest::Waiting(600), Step::Wait(REST_MS)));
    }

    #[test]
    fn a_button_down_spends_the_rest() {
        // A drag into the corner.
        assert_eq!(Rest::Away.moved(true, true, 0), (Rest::Spent, Step::Nothing));
        // A press while waiting.
        assert_eq!(Rest::Waiting(0).moved(true, true, 100), (Rest::Spent, Step::Nothing));
        // A button held when the wait is up.
        assert_eq!(Rest::Waiting(0).tick(true, 400), (Rest::Spent, Step::Nothing));
        // Letting go in the corner doesn't open it until the pointer leaves and rests again.
        assert_eq!(Rest::Spent.moved(true, false, 500), (Rest::Spent, Step::Nothing));
    }

    #[test]
    fn the_clock_wrapping_still_times_the_rest() {
        let since = u32::MAX - 100;
        assert_eq!(Rest::Waiting(since).tick(false, 299), (Rest::Spent, Step::Open));
        assert_eq!(Rest::Waiting(since).tick(false, 200), (Rest::Waiting(since), Step::Wait(99)));
    }

    #[test]
    fn corner_names_read_from_the_config() {
        assert_eq!(Corner::from_name("top_right"), Corner::TopRight);
        assert_eq!(Corner::from_name("bottom_left"), Corner::BottomLeft);
        assert_eq!(Corner::from_name("bottom_right"), Corner::BottomRight);
        assert_eq!(Corner::from_name("top_left"), Corner::TopLeft);
        assert_eq!(Corner::from_name("middle"), Corner::TopLeft);
    }
}
