//! Where the file lives, and reading, importing and saving it.

use super::*;

/// Write-then-rename so a crash never leaves half a file.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("Seam").join("config.json")
    }

    pub fn load() -> Config {
        Self::load_from(&Self::path())
    }

    pub(super) fn load_from(p: &Path) -> Config {
        let Ok(bytes) = std::fs::read(p) else { return Config::default() };
        match serde_json::from_slice::<Config>(&bytes) {
            Ok(mut c) => {
                if c.version < CURRENT_VERSION {
                    keep_v1_copy(p, &bytes);
                    c.migrate_v1();
                }
                c.migrate_sign_in();
                c
            }
            Err(_) => {
                // Set it aside: the next save would otherwise destroy the layouts in it.
                let _ = std::fs::rename(p, p.with_extension("json.corrupt"));
                Config::default()
            }
        }
    }

    /// Read a file the user chose to import. Old files convert, as on load. A
    /// folder or page from a file is never opened (D98, D330), so they're dropped.
    pub fn from_import(bytes: &[u8]) -> Result<Config, String> {
        let mut c: Config = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if c.version < CURRENT_VERSION {
            c.migrate_v1();
        }
        c.migrate_sign_in();
        for desk in c.desks.values_mut() {
            desk.workspaces = crate::workspace::without_places(&desk.workspaces);
            // The last session never saves them, so one in a file is dropped too.
            if let Some(session) = desk.last_session.take() {
                desk.last_session = crate::workspace::without_places(&[session]).pop();
            }
        }
        Ok(c)
    }

    pub fn save(&self) {
        self.save_to(&Self::path());
    }

    pub(super) fn save_to(&self, p: &Path) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = write_atomic(p, &s);
        }
    }
}
