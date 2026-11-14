//! The engine's state and how it reaches the hook, the overlay and the dividers.

use super::*;

impl Engine {
    /// A free drag to another screen gets its size from Seam (§11.4).
    pub(super) fn drags_sized(&self) -> bool {
        self.drag_size != WindowSize::Windows
    }

    /// Zones are on, and no other window manager has the windows.
    pub(super) fn active(&self) -> bool {
        self.enabled && self.aside.is_none() && !self.standing_by
    }

    /// A drop on a window's edge puts the two side by side.
    pub(super) fn beside_on(&self) -> bool {
        self.active() && self.settings.snap_beside
    }

    /// Have the overlay thread draw ahead of time what a drag will show, or free it all.
    pub(super) fn sync_overlay(&mut self) {
        let wanted = self.active() && self.screens.iter().any(|s| s.layout.is_some());
        if wanted {
            zone_paint::build(specs(&self.geometry()));
        } else {
            zone_paint::clear();
        }
    }

    pub(super) fn sync_hook(&mut self) {
        let zoning = self.active() && self.screens.iter().any(|s| s.layout.is_some());
        // A free drag between screens is seen at its end, zones or not, and snap
        // beside works on screens with no zones.
        let wanted = zoning || self.drags_sized() || self.beside_on();
        match (wanted, self.hook) {
            (true, None) => unsafe {
                let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                let hook = SetWinEventHook(
                    EVENT_SYSTEM_MOVESIZESTART,
                    EVENT_SYSTEM_MOVESIZEEND,
                    None,
                    Some(on_event),
                    0,
                    0,
                    flags,
                );
                if hook.0.is_null() {
                    log::info!("zone drag hook not installed");
                } else {
                    self.hook = Some(hook);
                }
            },
            (false, Some(hook)) => {
                let _ = unsafe { UnhookWinEvent(hook) };
                self.hook = None;
            }
            _ => {}
        }
    }

    pub(super) fn geometry(&self) -> Vec<Placed> {
        let gap = |dpi: u32| (self.settings.gap.min(MAX_GAP) as f32 * dpi as f32 / 96.0).round() as i32;
        let placed = self.screens.iter().filter_map(|s| {
            let layout = s.layout.as_ref()?;
            let (work, dpi) = work_area(s.bounds)?;
            Some(Placed {
                id: s.id.clone(),
                bounds: s.bounds,
                work,
                dpi,
                cells: zones::rects(&layout.tree, work, 0),
                zones: zones::rects(&layout.tree, work, gap(dpi)),
            })
        });
        placed.collect()
    }

    pub(super) fn zoned(&self) -> Vec<Zoned> {
        let moving = |key: isize| self.pending.as_ref().is_some_and(|p| p.hwnd == key);
        let entry = |(key, s): (&isize, &Snapped)| Zoned {
            hwnd: *key,
            zone: (s.screen.clone(), s.from, s.to),
            at: s.at,
            moving: moving(*key),
        };
        self.snapped.iter().map(entry).collect()
    }

    /// The snapped entry for a window that is still where it was put.
    pub(super) fn current(&self, hwnd: HWND) -> Option<&Snapped> {
        let snapped = self.snapped.get(&(hwnd.0 as isize))?;
        let here = winutil::visible_rect(hwnd)?;
        close(here, snapped.at, winutil::is_resizable(hwnd)).then_some(snapped)
    }

    /// What the divider handles and the size menu need to know about the zones now.
    fn scene(&self) -> dividers::Scene {
        let screen = |g: Placed| {
            let layout = self.screens.iter().find(|s| s.id == g.id)?.layout.clone()?;
            let gap = (self.settings.gap.min(MAX_GAP) as f32 * g.dpi as f32 / 96.0).round() as i32;
            Some(dividers::Screen { id: g.id, bounds: g.bounds, work: g.work, dpi: g.dpi, gap, layout })
        };
        let mut screens: Vec<dividers::Screen> = self.geometry().into_iter().filter_map(screen).collect();
        let mut known = self.zoned();
        self.adhoc_scene(&mut screens, &mut known);
        dividers::Scene { owner: self.owner, screens, known, exclude: self.exclude.clone() }
    }

    /// Hand the divider handles the zones as they are now, or none with zones off,
    /// stepping aside or the handles turned off. Their thread builds them a moment later.
    pub(super) fn sync_dividers(&self) {
        // The handles stay hidden until the drag ends, which hands them the zones then.
        if self.drag.is_some() {
            return;
        }
        let zoned = self.screens.iter().any(|s| s.layout.is_some()) || !self.adhoc.is_empty();
        let on = self.active() && self.settings.divider_handles && zoned;
        dividers::show(on.then(|| self.scene()));
    }

    /// Why a window can't be sized in its zones, or its visible frame.
    fn sizable(&self, hwnd: HWND) -> Result<Rect, String> {
        if let Some(why) = &self.aside {
            return Err(why.clone());
        }
        if !self.enabled {
            return Err("Zones are off".into());
        }
        wincontrol::movable(hwnd, &self.exclude)
    }

    /// The zones a window is in, and its size from before it was snapped: the
    /// snapping's own record, or after a restart of the tray the stamp on the window.
    pub(super) fn zone_here(&self, hwnd: HWND, frame: Rect) -> Option<(String, usize, usize, Option<Old>)> {
        if let Some(s) = self.current(hwnd) {
            return Some((s.screen.clone(), s.from, s.to, s.old));
        }
        let geometry = self.geometry();
        let g = geometry.iter().find(|g| g.bounds.contains(frame.x + frame.w / 2, frame.y + frame.h / 2))?;
        let (from, to, _) = wincontrol::stamped_in(hwnd, frame, &g.zones)?;
        Some((g.id.clone(), from, to, None))
    }

    pub(super) fn take_out(&mut self, hwnd: HWND) -> Result<(), String> {
        let frame = self.sizable(hwnd)?;
        let (screen, from, to, old) = self.zone_here(hwnd, frame).ok_or("That window isn't in a zone")?;
        let geometry = self.geometry();
        let g = geometry.iter().find(|g| g.id == screen && to < g.zones.len()).ok_or("That window isn't in a zone")?;
        let zone = zones::span(g.zones[from], g.zones[to]);
        let (w, h) = match old {
            _ if !winutil::is_resizable(hwnd) => (frame.w, frame.h),
            Some((w, h, dpi)) => {
                // The old size is a window rect, so its invisible borders come off.
                let k = winutil::monitor_dpi(hwnd).max(1) as f32 / dpi.max(1) as f32;
                let window = winutil::window_rect(hwnd).unwrap_or(frame);
                (
                    (w as f32 * k).round() as i32 - (window.w - frame.w),
                    (h as f32 * k).round() as i32 - (window.h - frame.h),
                )
            }
            // Maximized before it was snapped, or snapped before the tray started.
            None => (zone.w * 2 / 3, zone.h * 2 / 3),
        };
        let (w, h) = (w.max(1), h.max(1));
        let target = Rect { x: zone.x + (zone.w - w) / 2, y: zone.y + (zone.h - h) / 2, w, h };
        self.place(hwnd, target, false, false);
        self.unsnap(hwnd, winutil::has_zone_stamp(hwnd));
        Ok(())
    }

    pub(super) fn size_menu(&self, hwnd: HWND) -> Result<(), String> {
        let frame = self.sizable(hwnd)?;
        let (screen, from, to, _) = self.zone_here(hwnd, frame).ok_or("That window isn't in a zone")?;
        dividers::menu(hwnd.0 as isize, (screen, from, to), self.scene());
        Ok(())
    }
}
