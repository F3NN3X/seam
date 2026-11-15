//! What the overview needs from the snapping: the zones it can drop a window on, and
//! the move once it is dropped. A drop is a placement by hand, so for smart tiling the
//! window floats, as after a drag out (D562, D569).

use super::*;

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

/// Put a window the overview dropped: in a zone of `screen`, or at `free` there. None
/// while the snapping is busy, to be tried again.
pub fn overview_place(hwnd: isize, screen: &str, zone: Option<usize>, free: Rect) -> Option<Result<(), String>> {
    with_engine(|e| e.overview_place(hwnd, screen, zone, free))
}

impl Engine {
    fn overview_place(&mut self, key: isize, screen: &str, zone: Option<usize>, free: Rect) -> Result<(), String> {
        let h = hwnd_of(key);
        if !unsafe { IsWindow(Some(h)) }.as_bool() {
            return Err("The window closed".into());
        }
        // A tiled window leaves its tile and floats, and its neighbours fill the gap.
        if self.tiled_window(key) {
            let _ = self.tiling_float(key);
        }
        let geometry = if self.active() { self.geometry() } else { Vec::new() };
        let g = geometry.iter().find(|g| g.id == screen);
        let crosses = |dpi: u32| winutil::monitor_dpi(h) != dpi;
        match (g, zone) {
            (Some(g), Some(z)) if z < g.zones.len() && !self.tiling_on(screen) => {
                self.snap_to(h, g, (z, z), g.zones[z], None);
            }
            (Some(g), Some(z)) if z < g.zones.len() => {
                // A tiled screen's zones are its tree, which only tiling fills: the window floats over the tile.
                winutil::set_floated(h, true);
                self.place(h, g.zones[z], crosses(g.dpi), false);
                self.unsnap(h, winutil::has_zone_stamp(h));
            }
            (_, Some(_)) => return Err("That zone is gone".into()),
            (_, None) => {
                let dpi = work_area(free).map_or(96, |(_, dpi)| dpi);
                self.place(h, free, crosses(dpi), false);
                self.unsnap(h, winutil::has_zone_stamp(h));
            }
        }
        Ok(())
    }
}
