//! Screen margins: a strip along each side of a screen's work area that zones, tiles and
//! groups stay out of. A margin is in pixels at 96 DPI and scales with the screen.

use crate::zones::Rect;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The most one side may take, as a share of the work area's size on that axis.
const MAX_SHARE: f32 = 0.25;
/// The settings' slider end, in pixels at 96 DPI. A stored value past it is still honoured up to the share.
pub const MAX_PX: u32 = 400;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Margins {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl Margins {
    pub fn is_zero(&self) -> bool {
        *self == Margins::default()
    }

    /// Top, right, bottom and left, in that order.
    pub fn sides(&self) -> [u32; 4] {
        [self.top, self.right, self.bottom, self.left]
    }

    /// These with side `i` (in the order of `sides`) set to `px`, cut to the slider's end.
    pub fn with(self, i: usize, px: u32) -> Margins {
        let px = px.min(MAX_PX);
        match i {
            0 => Margins { top: px, ..self },
            1 => Margins { right: px, ..self },
            2 => Margins { bottom: px, ..self },
            _ => Margins { left: px, ..self },
        }
    }
}

/// `work` with each margin taken off its side: scaled to `dpi`, and no side more than a quarter of
/// the axis. No margins gives `work` back unchanged.
pub fn shrink(work: Rect, dpi: u32, m: &Margins) -> Rect {
    if m.is_zero() {
        return work;
    }
    let side = |px: u32, axis: i32| {
        let scaled = (px as f32 * dpi as f32 / 96.0).round();
        scaled.min(axis.max(0) as f32 * MAX_SHARE) as i32
    };
    let (left, right) = (side(m.left, work.w), side(m.right, work.w));
    let (top, bottom) = (side(m.top, work.h), side(m.bottom, work.h));
    Rect { x: work.x + left, y: work.y + top, w: work.w - left - right, h: work.h - top - bottom }
}

/// Each screen's margins on the desk in front, by the screen's rect in pixels. The tray sets it
/// whenever the desk, its screens or the config change, and every work-area lookup reads it.
static BY_SCREEN: Mutex<Vec<(Rect, Margins)>> = Mutex::new(Vec::new());

pub fn set(by_screen: Vec<(Rect, Margins)>) {
    *BY_SCREEN.lock().unwrap_or_else(|e| e.into_inner()) = by_screen;
}

/// The margins of the screen whose rect is `screen`, none when it has none.
pub fn of(screen: Rect) -> Margins {
    let by_screen = BY_SCREEN.lock().unwrap_or_else(|e| e.into_inner());
    by_screen.iter().find(|(r, _)| *r == screen).map(|(_, m)| *m).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 };

    #[test]
    fn no_margins_give_the_same_area() {
        assert_eq!(shrink(WORK, 96, &Margins::default()), WORK);
        assert_eq!(shrink(WORK, 192, &Margins::default()), WORK);
    }

    #[test]
    fn each_side_takes_its_own_strip() {
        let m = Margins { top: 10, right: 20, bottom: 30, left: 40 };
        assert_eq!(shrink(WORK, 96, &m), Rect { x: 40, y: 10, w: 1860, h: 1000 });
    }

    #[test]
    fn margins_scale_with_the_dpi() {
        let m = Margins { top: 10, right: 0, bottom: 0, left: 20 };
        assert_eq!(shrink(WORK, 144, &m), Rect { x: 30, y: 15, w: 1890, h: 1025 });
        assert_eq!(shrink(WORK, 192, &m), Rect { x: 40, y: 20, w: 1880, h: 1020 });
    }

    #[test]
    fn a_side_stops_at_a_quarter_of_its_axis() {
        let m = Margins { top: 5000, right: 5000, bottom: 0, left: 0 };
        assert_eq!(shrink(WORK, 96, &m), Rect { x: 0, y: 260, w: 1440, h: 780 });
        // On the other axis too, and offset work areas keep their origin.
        let off = Rect { x: -1920, y: 100, w: 1920, h: 1000 };
        let all = Margins { top: 9999, right: 9999, bottom: 9999, left: 9999 };
        assert_eq!(shrink(off, 96, &all), Rect { x: -1440, y: 350, w: 960, h: 500 });
    }

    #[test]
    fn the_table_answers_by_screen() {
        let (a, b) = (Rect { x: 0, y: 0, w: 1920, h: 1080 }, Rect { x: 1920, y: 0, w: 1920, h: 1080 });
        let m = Margins { top: 1, right: 2, bottom: 3, left: 4 };
        set(vec![(a, m)]);
        assert_eq!(of(a), m);
        assert_eq!(of(b), Margins::default());
        set(Vec::new());
        assert_eq!(of(a), Margins::default());
    }

    #[test]
    fn a_side_is_set_by_its_place_and_cut_to_the_slider() {
        let m = Margins::default().with(1, 30).with(3, 9999);
        assert_eq!(m.sides(), [0, 30, 0, MAX_PX]);
    }

    #[test]
    fn an_older_config_without_margins_reads_as_none() {
        let m: Margins = serde_json::from_str(r#"{"left":12}"#).unwrap();
        assert_eq!(m, Margins { left: 12, ..Margins::default() });
    }
}
