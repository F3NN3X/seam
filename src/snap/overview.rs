//! What the overview needs from the snapping: the zones it can drop a window on, and
//! the move once it is dropped. A drop is a placement by hand, so for smart tiling the
//! window floats, as after a drag out (D562, D569).

use super::*;
use seam::overview;

/// A screen the overview can drop windows on, by device path.
pub struct Target {
    pub screen: String,
    /// Its zones with the gap, empty when it has none or zones are off.
    pub zones: Vec<Rect>,
    /// Smart tiling arranges it on the desktop in front, so a drop there floats.
    pub tiled: bool,
}

/// The zones of each screen that has them, while zones are on and Seam doesn't stand aside.
pub fn overview_targets() -> Vec<Target> {
    let targets = |e: &mut Engine| {
        if !e.active() {
            return Vec::new();
        }
        let tiled = |id: &str| e.tiling_on(id);
        e.geometry().into_iter().map(|g| Target { tiled: tiled(&g.id), screen: g.id, zones: g.zones }).collect()
    };
    with_engine(targets).unwrap_or_default()
}

/// Put a window the overview dropped: in zone `zone` of `screen`, whose rect it showed
/// as `free`, or at `free` there. None while the snapping is busy, to be tried again.
pub fn overview_place(hwnd: isize, screen: &str, zone: Option<usize>, free: Rect) -> Option<Result<(), String>> {
    with_engine(|e| e.overview_place(hwnd, screen, zone, free))
}

impl Engine {
    /// Every check runs on the zones as they are before anything moves, and only then is
    /// a tiled window floated, so a refused drop leaves the window and tiling as they were.
    fn overview_place(&mut self, key: isize, screen: &str, zone: Option<usize>, free: Rect) -> Result<(), String> {
        let h = hwnd_of(key);
        let alive = unsafe { IsWindow(Some(h)) }.as_bool();
        let geometry = if self.active() { self.geometry() } else { Vec::new() };
        let g = geometry.iter().find(|g| g.id == screen);
        let facts = overview::DropFacts {
            alive,
            elevated: alive && winutil::is_elevated_over_us(h),
            tiled_window: self.tiled_window(key),
            zone,
            zones_now: g.map_or(&[][..], |g| &g.zones),
            shown: free,
            target_tiled: self.tiling_on(screen),
        };
        let (float_first, snap, float_over) = match overview::plan_drop(&facts) {
            overview::DropPlan::Refuse(why) => return Err(why.into()),
            overview::DropPlan::Move { float_first, snap, float_over } => (float_first, snap, float_over),
        };
        // A tiled window leaves its tile and floats, and its neighbours fill the gap.
        if float_first {
            let _ = self.tiling_float(key);
        }
        let crosses = |dpi: u32| winutil::monitor_dpi(h) != dpi;
        match (g, snap) {
            // Not a tiled screen, so its zones are as checked.
            (Some(g), Some(z)) => self.snap_to(h, g, (z, z), g.zones[z], None),
            _ => {
                // A tiled screen's zones are its tree, which only tiling fills: the window
                // floats over the tile as it was shown, even if the float reflowed the tree.
                if float_over {
                    winutil::set_floated(h, true);
                }
                let dpi = work_area(free).map_or(96, |(_, dpi)| dpi);
                self.place(h, free, crosses(dpi), false);
                self.unsnap(h, winutil::has_zone_stamp(h));
            }
        }
        Ok(())
    }
}
