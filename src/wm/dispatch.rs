//! Carrying out the tray's commands and the timers.

use super::*;

impl Wm {
    pub(super) fn run(&mut self, command: Command) {
        match command {
            Command::Configure { owner, focus_border, focus_color, pin_color, notify_close } => {
                // A window manager found before the tray first said who it is.
                let first = self.owner == 0;
                self.owner = owner;
                self.notify_close = notify_close;
                if first && self.other_wm.is_some() {
                    self.tell_tray();
                }
                self.focus_accent = palette::colour(&focus_color, FOCUS_DEFAULT) == Colour::Accent;
                self.pin_accent = palette::colour(&pin_color, PIN_DEFAULT) == Colour::Accent;
                let focus_colour = palette::resolve(&focus_color, FOCUS_DEFAULT);
                let pin_colour = palette::resolve(&pin_color, PIN_DEFAULT);
                let (focus_changed, pin_changed) =
                    (focus_colour != self.focus_colour, first || pin_colour != self.pin_colour);
                (self.focus_colour, self.pin_colour) = (focus_colour, pin_colour);
                let was_on = self.focus_border;
                self.focus_border = focus_border;
                self.sync_hooks();
                if pin_changed {
                    // None gives the pinned windows' borders back. Otherwise this is also where
                    // a tray that starts again gives adopted pins their colour.
                    self.paint_pins();
                }
                if !focus_border {
                    if let Some(old) = self.bordered.take() {
                        reset_border(hwnd_of(old));
                    }
                } else if !was_on || focus_changed || pin_changed {
                    match self.focus_colour {
                        Some(_) if self.bordered.is_some() && !pin_changed => self.paint_focus(),
                        Some(_) => self.border(unsafe { GetForegroundWindow() }),
                        None => {
                            if let Some(old) = self.bordered.take() {
                                reset_border(hwnd_of(old));
                            }
                        }
                    }
                }
            }
            Command::Pin { kind, on, hwnd, reply } => {
                let result = self.pin(kind, on, hwnd_of(hwnd));
                match (result, reply) {
                    (Ok(on), Some(tx)) => {
                        let _ = tx.send(Ok(json!({ "on": on })));
                    }
                    (Err(why), Some(tx)) => {
                        let _ = tx.send(Err(RpcError::new(api::REFUSED, why)));
                    }
                    (Ok(on), None) => log::info!("pin {kind:?}: {}", if on { "on" } else { "off" }),
                    (Err(why), None) => log::info!("pin {kind:?}: {why}"),
                }
            }
            Command::AccentChanged => {
                if self.focus_accent {
                    self.focus_colour = palette::resolve("accent", FOCUS_DEFAULT);
                    self.paint_focus();
                }
                if self.pin_accent {
                    self.pin_colour = palette::resolve("accent", PIN_DEFAULT);
                    self.paint_pins();
                }
            }
            Command::Release { reply } => {
                let (released, by_rules) = self.release();
                log::info!("released {released} pinned windows, {by_rules} of them pinned by app rules");
                if let Some(tx) = reply {
                    let _ = tx.send(Ok(json!({ "ok": true, "released": released })));
                }
            }
            Command::Closing { hwnd } => {
                if let Some(known) = self.known.get_mut(&hwnd) {
                    known.asked_to_close = Some(Instant::now());
                }
            }
            Command::WatchShown { switch, tx } => {
                self.shown_to = Some((switch, tx));
                self.sync_hooks();
            }
            Command::UnwatchShown { switch } => {
                if self.shown_to.as_ref().is_some_and(|(s, _)| *s == switch) {
                    self.shown_to = None;
                    self.sync_hooks();
                }
            }
            Command::Workspace { active, auto_apply } => {
                if (active, auto_apply) != (self.workspace_active, self.auto_apply) {
                    (self.workspace_active, self.auto_apply) = (active, auto_apply);
                    self.sync_hooks();
                    log::info!("wm: workspace in use {active}, auto-apply {}", self.auto_applying());
                }
            }
            Command::AppRules { on } => {
                if on != self.app_rules {
                    self.app_rules = on;
                    self.sync_hooks();
                    log::info!("wm: app rules {on}");
                }
            }
            Command::Spotlight { on } => {
                if on != self.spotlight {
                    self.spotlight = on;
                    self.sync_hooks();
                }
            }
        }
    }

    /// Run `work`, and count it in the stats while logging.
    fn timed(&mut self, work: impl FnOnce(&mut Self)) {
        let started = log::enabled().then(Instant::now);
        work(self);
        if let Some(started) = started {
            let ns = started.elapsed().as_nanos() as u64;
            let s = &mut self.stats;
            (s.events, s.total_ns, s.max_ns) = (s.events + 1, s.total_ns + ns, s.max_ns.max(ns));
        }
    }

    /// A repaint or refresh timer fired.
    pub(super) fn timer(&mut self, id: usize) {
        if id == self.repaint_timer && id != 0 {
            self.timed(|wm| {
                wm.paint_again(&wm.repaint);
                if wm.repaint_second {
                    wm.stop_repaint();
                } else {
                    wm.repaint_second = true;
                    wm.repaint_timer =
                        unsafe { SetTimer(None, wm.repaint_timer, REPAINT_SECOND_MS - REPAINT_FIRST_MS, None) };
                }
            });
        } else if id == self.refresh_timer && id != 0 {
            self.timed(|wm| {
                wm.paint_again(&wm.pinned);
                wm.paint_again(&wm.bordered.into_iter().collect::<Vec<_>>());
            });
        } else if id == self.settle_timer && id != 0 {
            self.timed(Self::settled);
        } else if id == self.relocated_timer && id != 0 {
            self.timed(Self::relocated_due);
        }
    }
}
