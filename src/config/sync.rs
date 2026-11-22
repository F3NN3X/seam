//! The part of the config that lives in a synced folder (brief G, D751-D760): what is shared and what
//! stays on this PC, the merge for when both sides changed, and what a folder may be. No I/O here.

use super::{CURRENT_VERSION, Config};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// Top-level fields that stay on this PC. `config_folder` is the pointer to the folder itself, and
/// `layouts` is the legacy v1 list, read and never written. A field not listed here is shared (D752).
pub const LOCAL_TOP: &[&str] = &[
    "setup_done",
    "tray_click",
    "debug_log",
    "hotkeys",
    "restore_apps",
    "restore_apps_saved",
    "cursor_easing_asked",
    "updates",
    "api",
    "seen",
    "layouts",
    "config_folder",
];

/// Fields of a desk the tray rewrites on every switch, sign-in or desk change, so they stay on this PC.
pub const LOCAL_DESK: &[&str] = &["last_seen", "last_session", "active_workspace", "previous_workspace"];

/// The stamp only the shared file has.
const STAMP: &str = "sync";

/// The longest file read from the folder. A bigger one is refused.
pub const MAX_READ: usize = 8 * 1024 * 1024;
/// Reads of a file that doesn't parse before Seam gives up and says so (D756).
pub const READ_TRIES: u8 = 5;

pub const NOT_LOCAL: &str = "Pick a folder on this PC's own drive";
pub const OWN_FOLDER: &str = "Pick a folder other than Seam's own";
pub const NOT_THERE: &str = "That folder isn't there";

fn fields(v: &Value) -> impl Iterator<Item = (&String, &Value)> {
    v.as_object().into_iter().flatten()
}

/// The config as JSON, cut into the part for the shared file and the part that stays on this PC.
/// The shared part never carries a startup command (D757). The local part is only the local fields.
pub fn split(cfg: &Value) -> (Value, Value) {
    let (mut shared, mut local) = (Map::new(), Map::new());
    for (key, value) in fields(cfg) {
        match key.as_str() {
            "desks" => {}
            "version" => {
                shared.insert(key.clone(), value.clone());
                local.insert(key.clone(), value.clone());
            }
            STAMP => {
                shared.insert(key.clone(), value.clone());
            }
            k if LOCAL_TOP.contains(&k) => {
                local.insert(key.clone(), value.clone());
            }
            _ => {
                shared.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(desks) = cfg.get("desks").filter(|d| d.is_object()) {
        let (mut shared_desks, mut local_desks) = (Map::new(), Map::new());
        for (sig, desk) in fields(desks) {
            let (mut s, mut l) = (Map::new(), Map::new());
            for (key, value) in fields(desk) {
                let side = if LOCAL_DESK.contains(&key.as_str()) { &mut l } else { &mut s };
                side.insert(key.clone(), value.clone());
            }
            if let Some(list) = s.get("workspaces").and_then(Value::as_array) {
                s.insert("workspaces".into(), Value::Array(crate::workspace::without_commands(list)));
            }
            shared_desks.insert(sig.clone(), Value::Object(s));
            local_desks.insert(sig.clone(), Value::Object(l));
        }
        shared.insert("desks".into(), Value::Object(shared_desks));
        local.insert("desks".into(), Value::Object(local_desks));
    }
    (Value::Object(shared), Value::Object(local))
}

/// The shared part and this PC's complete local copy made one config. Every shared field comes from
/// `shared`, every local one from `local`, and a local field in `shared` is ignored, as the stamp is.
/// A startup command comes from `local` alone, onto the row that is still the same (D757).
pub fn join(shared: &Value, local: &Value) -> Value {
    let mut out = Map::new();
    for (key, value) in fields(shared) {
        if !matches!(key.as_str(), "desks" | "version" | STAMP) && !LOCAL_TOP.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in fields(local) {
        if key == "version" || LOCAL_TOP.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
    if !out.contains_key("version")
        && let Some(version) = shared.get("version")
    {
        out.insert("version".into(), version.clone());
    }
    let mine = local.get("desks");
    let mut desks = Map::new();
    for (sig, desk) in shared.get("desks").into_iter().flat_map(fields) {
        let mine = mine.and_then(|m| m.get(sig));
        let mut joined = Map::new();
        for (key, value) in fields(desk).filter(|(k, _)| !LOCAL_DESK.contains(&k.as_str())) {
            joined.insert(key.clone(), value.clone());
        }
        for key in LOCAL_DESK {
            if let Some(value) = mine.and_then(|m| m.get(*key)) {
                joined.insert((*key).into(), value.clone());
            }
        }
        if let Some(list) = joined.get_mut("workspaces").and_then(Value::as_array_mut) {
            *list = crate::workspace::without_commands(list);
            let before = mine.and_then(|m| m.get("workspaces")).and_then(Value::as_array);
            list.iter_mut().for_each(|ws| restore_commands(ws, before.map(Vec::as_slice).unwrap_or_default()));
        }
        desks.insert(sig.clone(), Value::Object(joined));
    }
    out.insert("desks".into(), Value::Object(desks));
    Value::Object(out)
}

/// Put back the sealed commands this PC kept for a workspace, matched by workspace id, row and tab.
/// A row whose app or kind of item is no longer the same gets none.
fn restore_commands(ws: &mut Value, before: &[Value]) {
    let Some(old) =
        ws.get("id").filter(|id| id.is_string()).and_then(|id| before.iter().find(|w| w.get("id") == Some(id)))
    else {
        return;
    };
    let old_rows = old.get("windows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let rows = ws.get_mut("windows").and_then(Value::as_array_mut).into_iter().flatten();
    for (row, was) in rows.zip(old_rows) {
        if row.get("exe") != was.get("exe") || row.pointer("/item/kind") != was.pointer("/item/kind") {
            continue;
        }
        let tabs = row.pointer_mut("/item/tabs").and_then(Value::as_array_mut).into_iter().flatten();
        let old_tabs = was.pointer("/item/tabs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        for (tab, was) in tabs.zip(old_tabs) {
            if let (Some(tab), Some(sealed)) = (tab.as_object_mut(), was.get("command_sealed")) {
                tab.insert("command_sealed".into(), sealed.clone());
            }
        }
    }
}

/// What comes in from the folder, cleared as an untrusted import is (D757): nothing local, no
/// startup command, every item checked, and an "Open with" that isn't the one `base` had dropped,
/// with that row's launch turned off. `base` is the shared part as this PC last saw it.
pub fn intake(base: Option<&Value>, theirs: &mut Value) {
    let Some(map) = theirs.as_object_mut() else { return };
    map.retain(|key, _| key == "desks" || key == "version" || !(LOCAL_TOP.contains(&key.as_str()) || key == STAMP));
    for (sig, desk) in map.get_mut("desks").and_then(Value::as_object_mut).into_iter().flatten() {
        let Some(desk) = desk.as_object_mut() else { continue };
        desk.retain(|key, _| !LOCAL_DESK.contains(&key.as_str()));
        let before = base.and_then(|b| b.pointer(&format!("/desks/{}/workspaces", pointer_key(sig))));
        if let Some(list) = desk.get_mut("workspaces").and_then(Value::as_array_mut) {
            guard_open_with(list, before.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default());
            *list = crate::workspace::clean_imported_items(list);
        }
    }
}

fn pointer_key(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn guard_open_with(list: &mut [Value], before: &[Value]) {
    for ws in list {
        let old = ws.get("id").and_then(|id| before.iter().find(|w| w.get("id") == Some(id))).cloned();
        let old_rows = old.as_ref().and_then(|w| w.get("windows")).and_then(Value::as_array);
        let rows = ws.get_mut("windows").and_then(Value::as_array_mut).into_iter().flatten();
        for (n, row) in rows.enumerate() {
            let Some(fields) = row.as_object_mut() else { continue };
            let same = fields
                .get("open_with")
                .is_some_and(|now| old_rows.and_then(|rows| rows.get(n)).and_then(|r| r.get("open_with")) == Some(now));
            if fields.contains_key("open_with") && !same {
                fields.remove("open_with");
                fields.insert("launch".into(), Value::Bool(false));
            }
        }
    }
}

/// A key to merge: a top-level shared field, or one shared field of one desk.
type Key = (Option<String>, String);

fn flat(shared: &Value) -> BTreeMap<Key, Value> {
    let mut out = BTreeMap::new();
    for (key, value) in fields(shared) {
        if key == "desks" {
            for (sig, desk) in fields(value) {
                for (field, v) in fields(desk) {
                    out.insert((Some(sig.clone()), field.clone()), v.clone());
                }
            }
        } else if key != STAMP {
            out.insert((None, key.clone()), value.clone());
        }
    }
    out
}

fn name(key: &Key) -> String {
    match key {
        (Some(sig), field) => format!("desks.{sig}.{field}"),
        (None, field) => field.clone(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    pub value: Value,
    /// The keys changed on both sides to different values, where theirs was taken.
    pub clashed: Vec<String>,
}

/// Three-way merge of the shared part, per key (D754): a change on one side goes in, and a change
/// on both takes theirs and reports the key. `base` is the shared part as last read or written.
pub fn merge3(base: &Value, ours: &Value, theirs: &Value) -> Merged {
    let (b, o, t) = (flat(base), flat(ours), flat(theirs));
    let keys: BTreeSet<&Key> = b.keys().chain(o.keys()).chain(t.keys()).collect();
    let (mut top, mut desks, mut clashed) = (Map::new(), Map::<String, Value>::new(), Vec::new());
    for key in keys {
        let (before, mine, theirs) = (b.get(key), o.get(key), t.get(key));
        let picked = if mine == theirs || theirs == before {
            mine
        } else if mine == before {
            theirs
        } else {
            clashed.push(name(key));
            theirs
        };
        let Some(value) = picked else { continue };
        match key {
            (Some(sig), field) => {
                let desk = desks.entry(sig.clone()).or_insert_with(|| Value::Object(Map::new()));
                if let Some(desk) = desk.as_object_mut() {
                    desk.insert(field.clone(), value.clone());
                }
            }
            (None, field) => {
                top.insert(field.clone(), value.clone());
            }
        }
    }
    top.insert("desks".into(), Value::Object(desks));
    Merged { value: Value::Object(top), clashed }
}

/// A file a sync app made beside `config.json` when two PCs wrote it at once. Seam never reads one.
pub fn is_conflict_copy(file: &str) -> bool {
    let file = file.to_ascii_lowercase();
    let Some(stem) = file.strip_suffix(".json").and_then(|s| s.strip_prefix("config")) else { return false };
    // `config (1)`, `config-PC`, `config (Name's conflicted copy …)`, and Syncthing's `config.sync-conflict-…`.
    stem.starts_with(['-', ' ', '(']) || stem.starts_with(".sync-conflict-")
}

/// What a read of the folder's file came to.
#[derive(Debug)]
pub enum Attempt {
    /// A config that parses, migrated as a local load is.
    Take(Box<Config>),
    /// Saved by a newer Seam: not taken (the edge case).
    Newer,
    /// Empty or cut off, perhaps a sync app writing in pieces: wait and read again.
    Retry,
    /// Out of tries, or too big: say it can't be read.
    GiveUp,
}

/// Judge the bytes of the file in the folder, `tries` being the reads already made before this one.
pub fn attempt(bytes: &[u8], tries: u8) -> Attempt {
    if bytes.len() > MAX_READ {
        return Attempt::GiveUp;
    }
    let again = if tries + 1 >= READ_TRIES { Attempt::GiveUp } else { Attempt::Retry };
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else { return again };
    if value.get("version").and_then(Value::as_u64).is_some_and(|v| v > u64::from(CURRENT_VERSION)) {
        return Attempt::Newer;
    }
    let Ok(mut config) = serde_json::from_value::<Config>(value) else { return again };
    if config.version < CURRENT_VERSION {
        config.migrate_v1();
    }
    config.migrate_sign_in();
    Attempt::Take(Box::new(config))
}

/// Which drives are on this PC's own disks. A trait so a test can say.
pub trait Drives {
    fn fixed(&self, letter: char) -> bool;
}

/// The drive letter of a path that has the shape D760 asks for: absolute with a drive letter, no
/// `..`, shorter than 260 characters.
pub fn shape_ok(path: &str) -> Result<char, &'static str> {
    let mut chars = path.chars();
    let (Some(letter), Some(':'), Some('\\' | '/')) = (chars.next(), chars.next(), chars.next()) else {
        return Err(NOT_LOCAL);
    };
    let clean = path.chars().count() < 260
        && letter.is_ascii_alphabetic()
        && !path[2..].contains(|c: char| c == ':' || c.is_control())
        && !path.split(['\\', '/']).any(|part| part == "..")
        && !Path::new(path).components().any(|c| matches!(c, Component::ParentDir));
    if clean { Ok(letter.to_ascii_uppercase()) } else { Err(NOT_LOCAL) }
}

/// The folder a path names, if Seam may keep its settings there (D760): a directory that exists, on
/// a fixed drive, not UNC, not Seam's own.
pub fn folder_ok(path: &str, own: &Path, drives: &dyn Drives) -> Result<PathBuf, &'static str> {
    let letter = shape_ok(path)?;
    if !drives.fixed(letter) {
        return Err(NOT_LOCAL);
    }
    let folder = PathBuf::from(path);
    if !folder.is_dir() {
        return Err(NOT_THERE);
    }
    if same_folder(&folder, own) {
        return Err(OWN_FOLDER);
    }
    Ok(folder)
}

/// Whether two paths name the same folder, whatever the case, trailing slash or `\\?\` prefix.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    let plain = |p: &Path| {
        let full = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let text = full.to_string_lossy().replace('/', "\\");
        text.strip_prefix(r"\\?\").unwrap_or(&text).trim_end_matches('\\').to_ascii_lowercase()
    };
    plain(a) == plain(b)
}

/// `yyyymmdd-hhmmss` in UTC, for the name of a kept copy.
pub fn stamp(unix: u64) -> String {
    let (days, secs) = ((unix / 86_400) as i64, unix % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}", secs / 3_600, secs % 3_600 / 60, secs % 60)
}
