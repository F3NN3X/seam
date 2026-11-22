//! The part of the config that lives in a synced folder (brief G, D751-D760): what is shared and what
//! stays on this PC, the merge for when both sides changed, and what a folder may be. No I/O here.

use super::{CURRENT_VERSION, Config};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
            let kept = sealed_by_id(before.map(Vec::as_slice).unwrap_or_default());
            list.iter_mut().for_each(|ws| restore_commands(ws, &kept));
        }
        desks.insert(sig.clone(), Value::Object(joined));
    }
    out.insert("desks".into(), Value::Object(desks));
    Value::Object(out)
}

/// A tab as a sealed command is tied to it: the row's app and kind of item, and the tab's profile and
/// folder. A command only goes back onto a tab with the same of all of these (D757).
fn tab_key(row: &Value, tab: &Value) -> String {
    let part = |v: Option<&Value>| v.map_or_else(String::new, Value::to_string);
    [
        part(row.get("exe")),
        part(row.get("aumid")),
        part(row.pointer("/item/kind")),
        part(tab.get("profile")),
        part(tab.get("folder")),
    ]
    .join("\u{1}")
}

/// This PC's sealed commands by workspace id, then by tab key, in the order they stand in the workspace.
type Sealed<'a> = HashMap<&'a str, HashMap<String, Vec<&'a Value>>>;

fn sealed_by_id(workspaces: &[Value]) -> Sealed<'_> {
    let mut all: Sealed = HashMap::new();
    for ws in workspaces {
        let Some(id) = ws.get("id").and_then(Value::as_str) else { continue };
        let rows = ws.get("windows").and_then(Value::as_array).into_iter().flatten();
        for row in rows {
            for tab in row.pointer("/item/tabs").and_then(Value::as_array).into_iter().flatten() {
                if let Some(sealed) = tab.get("command_sealed") {
                    all.entry(id).or_default().entry(tab_key(row, tab)).or_default().push(sealed);
                }
            }
        }
    }
    all
}

/// Put back the sealed commands this PC kept for a workspace, onto the tabs that are still the same
/// by `tab_key`, the nth such tab getting the nth command. A tab that moved keeps its command, and a
/// tab whose folder, profile or app changed in the folder gets none.
fn restore_commands(ws: &mut Value, kept: &Sealed) {
    let Some(mine) = ws.get("id").and_then(Value::as_str).and_then(|id| kept.get(id)) else { return };
    let mut used: HashMap<String, usize> = HashMap::new();
    let rows = ws.get_mut("windows").and_then(Value::as_array_mut).into_iter().flatten();
    for row in rows {
        let snapshot = row.clone();
        let tabs = row.pointer_mut("/item/tabs").and_then(Value::as_array_mut).into_iter().flatten();
        for tab in tabs {
            let key = tab_key(&snapshot, tab);
            let n = used.entry(key.clone()).or_default();
            if let (Some(sealed), Some(fields)) = (mine.get(&key).and_then(|c| c.get(*n)), tab.as_object_mut()) {
                fields.insert("command_sealed".into(), (*sealed).clone());
            }
            *n += 1;
        }
    }
}

/// The most the file in the folder may hold. A bigger one is refused as unreadable, so a file under
/// the byte cap can't make the merge slow or huge.
const MAX_DESKS: usize = 64;
const MAX_WORKSPACES: usize = 200;
const MAX_ROWS: usize = 100;
const MAX_TABS: usize = 8;
const MAX_TOTAL_ROWS: usize = 2_000;

/// Whether the counts of a config in JSON are within the limits above.
pub fn within_limits(cfg: &Value) -> bool {
    let desks = cfg.get("desks").and_then(Value::as_object);
    if desks.is_some_and(|d| d.len() > MAX_DESKS) {
        return false;
    }
    let mut total = 0;
    for desk in desks.into_iter().flat_map(|d| d.values()) {
        let list = desk.get("workspaces").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        if list.len() > MAX_WORKSPACES {
            return false;
        }
        for ws in list {
            let rows = ws.get("windows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            total += rows.len();
            let tabs_ok =
                |row: &Value| row.pointer("/item/tabs").and_then(Value::as_array).is_none_or(|t| t.len() <= MAX_TABS);
            if rows.len() > MAX_ROWS || total > MAX_TOTAL_ROWS || !rows.iter().all(tabs_ok) {
                return false;
            }
        }
    }
    true
}

/// FNV-1a, so what is remembered across runs doesn't change with the compiler.
fn fnv(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    format!("{hash:016x}")
}

/// What a row can start or open, with its workspace: the app, the item, "Open with" and the folder or page.
/// This is what this PC accepts, or doesn't, before a row from the folder may launch.
fn identity(ws_id: &str, row: &Value) -> String {
    let part = |key: &str| row.get(key).map_or_else(String::new, Value::to_string);
    fnv(&[ws_id.to_string(), part("exe"), part("aumid"), part("item"), part("open_with"), part("place")].join("\u{1}"))
}

fn close_mark(ws_id: &str) -> String {
    format!("close:{ws_id}")
}

/// What this PC has accepted of a config's shared part: each row that launches (by `identity`) and each
/// workspace that closes other windows. A row from the folder that isn't one of these doesn't launch here.
/// It is kept in a file on this PC and never in the shared one.
pub fn accepted_of(shared: &Value) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for desk in shared.get("desks").into_iter().flat_map(fields).map(|(_, d)| d) {
        for ws in desk.get("workspaces").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = ws.get("id").and_then(Value::as_str) else { continue };
            if ws.get("other_windows").and_then(Value::as_str) == Some("close") {
                set.insert(close_mark(id));
            }
            for row in ws.get("windows").and_then(Value::as_array).into_iter().flatten() {
                if row.get("launch").and_then(Value::as_bool).unwrap_or(true) {
                    set.insert(identity(id, row));
                }
            }
        }
    }
    set
}

/// What comes in from the folder, cleared as an untrusted import is and then held to what this PC
/// accepted (D757): nothing local, no startup command, every item checked. A row whose app, item,
/// "Open with" or folder or page isn't one this PC accepted arrives with its launch off, and
/// without the "Open with" and the folder or page if the version without them isn't accepted either.
/// A workspace that closes other windows arrives leaving them, unless this PC accepted that. The
/// result is what this PC holds; what is written back to the folder is never this (see `outgoing`).
pub fn guard(theirs: &mut Value, accepted: &BTreeSet<String>) {
    let Some(map) = theirs.as_object_mut() else { return };
    map.retain(|key, _| key == "desks" || key == "version" || !(LOCAL_TOP.contains(&key.as_str()) || key == STAMP));
    for desk in map.get_mut("desks").and_then(Value::as_object_mut).into_iter().flat_map(|d| d.values_mut()) {
        let Some(desk) = desk.as_object_mut() else { continue };
        desk.retain(|key, _| !LOCAL_DESK.contains(&key.as_str()));
        let Some(list) = desk.get_mut("workspaces").and_then(Value::as_array_mut) else { continue };
        *list = crate::workspace::clean_imported_items(list);
        for ws in list.iter_mut() {
            let id = ws.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            if ws.get("other_windows").and_then(Value::as_str) == Some("close") && !accepted.contains(&close_mark(&id))
            {
                ws["other_windows"] = Value::String("leave".into());
            }
            for row in ws.get_mut("windows").and_then(Value::as_array_mut).into_iter().flatten() {
                let Some(fields) = row.as_object_mut() else { continue };
                if accepted.contains(&identity(&id, &Value::Object(fields.clone()))) {
                    continue;
                }
                fields.remove("open_with");
                fields.remove("place");
                if !accepted.contains(&identity(&id, &Value::Object(fields.clone()))) {
                    fields.insert("launch".into(), Value::Bool(false));
                }
            }
        }
    }
}

/// The shared part to write to the folder: this PC's, but with the folder's own value for everything this PC
/// didn't change since it took the folder's file in. `held` is what this PC made of `base` (the file as
/// it was read) with `guard`, so a row that came in with its launch off, or without an "Open with",
/// goes back out as the other PC wrote it. A key or row this PC changed goes out as this PC has it.
pub fn outgoing(held: &Value, ours: &Value, base: &Value) -> Value {
    let (h, o, b) = (flat(held), flat(ours), flat(base));
    let mut out: BTreeMap<Key, Value> = BTreeMap::new();
    for (key, mine) in &o {
        let value = match (h.get(key), b.get(key)) {
            (Some(held), Some(base)) if held == mine => base.clone(),
            (Some(held), Some(base)) if key.1 == "workspaces" => unguard(mine, held, base),
            _ => mine.clone(),
        };
        out.insert(key.clone(), value);
    }
    unflat(out)
}

/// A workspace list this PC edited, with each workspace and row it left as held put back as the folder had it.
fn unguard(mine: &Value, held: &Value, base: &Value) -> Value {
    let index = |v: &Value| -> HashMap<String, Value> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|ws| Some((ws.get("id")?.as_str()?.to_string(), ws.clone())))
            .collect()
    };
    let (held, base) = (index(held), index(base));
    let list = mine.as_array().into_iter().flatten().map(|ws| {
        let id = ws.get("id").and_then(Value::as_str).unwrap_or_default();
        let (Some(was), Some(raw)) = (held.get(id), base.get(id)) else { return ws.clone() };
        if ws == was {
            return raw.clone();
        }
        let mut ws = ws.clone();
        if ws.get("other_windows") == was.get("other_windows") {
            match raw.get("other_windows") {
                Some(v) => ws["other_windows"] = v.clone(),
                None => {
                    ws.as_object_mut().map(|f| f.remove("other_windows"));
                }
            }
        }
        let (held_rows, raw_rows) =
            (was.get("windows").and_then(Value::as_array), raw.get("windows").and_then(Value::as_array));
        if let (Some(rows), Some(held_rows), Some(raw_rows)) =
            (ws.get_mut("windows").and_then(Value::as_array_mut), held_rows, raw_rows)
        {
            for (i, row) in rows.iter_mut().enumerate() {
                if let (Some(was), Some(raw)) = (held_rows.get(i), raw_rows.get(i))
                    && row == was
                {
                    *row = raw.clone();
                }
            }
        }
        ws
    });
    Value::Array(list.collect())
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

fn unflat(keys: BTreeMap<Key, Value>) -> Value {
    let (mut top, mut desks) = (Map::new(), Map::<String, Value>::new());
    for (key, value) in keys {
        match key {
            (Some(sig), field) => {
                if let Some(desk) = desks.entry(sig).or_insert_with(|| Value::Object(Map::new())).as_object_mut() {
                    desk.insert(field, value);
                }
            }
            (None, field) => {
                top.insert(field, value);
            }
        }
    }
    top.insert("desks".into(), Value::Object(desks));
    Value::Object(top)
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
    if !within_limits(&value) {
        return Attempt::GiveUp;
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
    // A junction or link on a fixed drive can lead to a network share: the real path has to pass too.
    if let Ok(real) = std::fs::canonicalize(&folder) {
        let text = real.to_string_lossy().replace('/', "\\");
        let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
        if plain.starts_with(r"UNC\") || plain.starts_with(r"\\") || !shape_ok(plain).is_ok_and(|l| drives.fixed(l)) {
            return Err(NOT_LOCAL);
        }
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
