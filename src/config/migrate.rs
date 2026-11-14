//! Moving an older file forward: the sign-in choice and the v1 layouts.

use super::*;

impl Config {
    /// Once, for a file from before "When I sign in": each desk takes its choice from
    /// the old `restore_session` switch. A file where some desk already has one is
    /// left alone, so a desk seen later never inherits anything.
    pub fn migrate_sign_in(&mut self) {
        if self.desks.values().any(|d| d.sign_in.is_some()) {
            return;
        }
        let mode = if self.restore_session { SignInMode::LastSession } else { SignInMode::Nothing };
        for desk in self.desks.values_mut() {
            desk.sign_in = Some(SignIn { mode, ..SignIn::default() });
            if mode == SignInMode::Nothing {
                desk.last_session = None;
            }
        }
    }

    /// Each v1 layout becomes a desk. Tray left-click stays on Line Up, so
    /// nobody's habit breaks.
    pub(super) fn migrate_v1(&mut self) {
        for (sig, borders) in std::mem::take(&mut self.layouts) {
            let name = Desk::placeholder(sig.split('|').count());
            self.desks.entry(sig).or_insert_with(|| Desk { name, borders, ..Desk::default() });
        }
        self.tray_click = TrayClick::LineUp;
        self.version = CURRENT_VERSION;
    }
}

/// A v0.1 build can't read a v2 file, so keep the original to go back to. Only
/// the first copy counts: a later one could already hold another run's changes.
pub(super) fn keep_v1_copy(config: &Path, bytes: &[u8]) {
    let backup = config.with_file_name("config.v1.json.bak");
    if !backup.exists()
        && let Err(e) = std::fs::write(&backup, bytes)
    {
        log::info!("config.v1.json.bak not written: {e}");
    }
}
