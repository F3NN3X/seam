//! The focus border, pins, Release all and the check for another window manager.

use super::*;

impl Wm {
    /// Move the border to the window that has focus now.
    pub(super) fn border(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.bordered == Some(key) || !borders_supported() {
            return;
        }
        if let Some(old) = self.bordered.take() {
            reset_border(hwnd_of(old));
        }
        // A pinned window keeps its pin colour (D117).
        let Some(colour) = self.focus_colour else { return };
        if !self.pin_owns(key) && ours_to_touch(h).is_ok() && set_border_colour(h, colour) {
            winutil::mark_border(h, Some(colour));
            self.bordered = Some(key);
        }
    }

    /// The pin colour paints this window: it is pinned and the pin colour isn't None.
    pub(super) fn pin_owns(&self, key: isize) -> bool {
        self.pin_colour.is_some() && self.pinned.contains(&key)
    }

    /// Give the window that has the focus border the focus colour again.
    pub(super) fn paint_focus(&self) {
        if let (Some(h), Some(colour)) = (self.bordered.map(hwnd_of), self.focus_colour)
            && set_border_colour(h, colour)
        {
            winutil::mark_border(h, Some(colour));
        }
    }

    pub(super) fn paint_pin(&self, h: HWND) {
        if let (true, Some(colour)) = (borders_supported(), self.pin_colour)
            && set_border_colour(h, colour)
        {
            winutil::mark_border(h, Some(colour));
        }
    }

    /// Paint every pinned window in the pin colour. With none, give back the border
    /// Seam painted. A pinned window that had the focus colour now wears the pin's.
    pub(super) fn paint_pins(&mut self) {
        for key in self.pinned.clone() {
            let h = hwnd_of(key);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                continue;
            }
            if self.pin_colour.is_some() {
                if self.bordered == Some(key) {
                    self.bordered = None;
                }
                self.paint_pin(h);
            } else if winutil::has_border_mark(h) && self.bordered != Some(key) {
                reset_border(h);
            }
        }
    }

    /// Turn a pin on, off, or over. Returns whether it is on now.
    pub(super) fn pin(&mut self, kind: PinKind, on: Option<bool>, h: HWND) -> Result<bool, &'static str> {
        ours_to_touch(h)?;
        let bits = winutil::pin_bits(h);
        let bit = match kind {
            PinKind::Top => winutil::PIN_TOP,
            PinKind::Sticky => winutil::PIN_STICKY,
        };
        let was = bits & bit != 0;
        let want = on.unwrap_or(!was);
        if kind == PinKind::Top && want != was {
            // Seam only ever takes back a topmost it set, so it can't set one the app has.
            if want && is_topmost(h) {
                return Err("That window is already on top");
            }
            set_topmost(h, want);
        }
        let now = if want { bits | bit } else { bits & !bit };
        winutil::set_pin_bits(h, now);
        let key = h.0 as isize;
        self.pinned.retain(|p| *p != key);
        self.known.remove(&key);
        if now != bits {
            self.tell_tray_of(WM_APP_WINDOWS_CHANGED);
        }
        if now != 0 {
            self.pinned.push(key);
            self.known.insert(key, Pinned::of(h, now));
            // The pin colour takes the border from the focus colour.
            if self.pin_colour.is_some() && self.bordered == Some(key) {
                self.bordered = None;
            }
            self.paint_pin(h);
        } else if bits != 0 {
            // With no pin colour there is nothing of the pin's to take back.
            if self.pin_colour.is_some() {
                reset_border(h);
            }
            // Still the focused window: it gets the focus colour back.
            if self.focus_border && unsafe { GetForegroundWindow() } == h {
                self.border(h);
            }
        }
        self.sync_hooks();
        Ok(want)
    }

    /// Take back every pin and border Seam set. Returns how many windows were pinned,
    /// and how many of those an app rule pinned.
    pub(super) fn release(&mut self) -> (usize, usize) {
        let (mut released, mut by_rules) = (0, 0);
        for h in winutil::top_level_windows() {
            let bits = winutil::pin_bits(h);
            if bits != 0 {
                by_rules += usize::from(winutil::rule_pin_bits(h) != 0);
                if bits & winutil::PIN_TOP != 0 {
                    set_topmost(h, false);
                }
                winutil::set_pin_bits(h, 0);
                released += 1;
            }
            if winutil::has_border_mark(h) {
                reset_border(h);
            }
        }
        self.pinned.clear();
        self.known.clear();
        self.bordered = None;
        self.sync_hooks();
        if released > 0 {
            self.tell_tray_of(WM_APP_WINDOWS_CHANGED);
        }
        (released, by_rules)
    }

    /// A pinned window closed. Seam asking it to, Release all and quitting are
    /// not news: the first is marked, and the other two leave nothing pinned.
    pub(super) fn pin_closed(&self, gone: Pinned) {
        let asked = gone.asked_to_close.is_some_and(|at| at.elapsed() < ASKED_TO_CLOSE);
        if !self.notify_close || asked {
            return;
        }
        log::info!("a pinned window closed: {}", gone.class);
        // A second close before the first notice is used replaces it.
        *CLOSED.lock().unwrap_or_else(|e| e.into_inner()) = Some(gone);
        self.tell_tray_of(WM_APP_PIN_CLOSED);
    }

    /// Look for another window manager, and tell the tray when that changes.
    pub(super) fn check(&mut self) {
        let found = find_other_wm();
        if found == self.other_wm {
            return;
        }
        self.other_wm = found;
        *DETECTED.lock().unwrap_or_else(|e| e.into_inner()) = found;
        self.tell_tray();
    }

    pub(super) fn tell_tray(&self) {
        self.tell_tray_of(WM_APP_WM);
    }

    fn tell_tray_of(&self, message: u32) {
        if self.owner != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(self.owner)), message, WPARAM(0), LPARAM(0));
            }
        }
    }
}
