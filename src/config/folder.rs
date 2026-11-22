//! The folder the shared part of the config lives in (brief G): what Seam knows about it, the file in
//! it, and the writes to it. The tray's save thread and its `seam-sync` thread both use this.

use super::sync::{self, Attempt};
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

pub const SHARED_FILE: &str = "config.json";
const BASE_FILE: &str = "config.sync-base.json";
const ACCEPTED_FILE: &str = "config.sync-accepted.json";
/// The most copies by a sync app that are listed, and the longest name shown.
const MAX_CONFLICTS: usize = 20;
const MAX_NAME: usize = 100;

/// Whether the settings live on this PC only, in a folder Seam can reach, or in one it can't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderState {
    #[default]
    Local,
    Synced,
    Unavailable,
}

/// Why the folder's file isn't being used, other than the folder being gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    /// Saved by a newer Seam.
    Newer,
    /// It would not parse after every retry, or is too big.
    Unreadable,
    /// This PC's own settings are over what a file in the folder may hold, so none is written.
    TooBig,
}

/// What `config.folder` answers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Info {
    pub path: Option<String>,
    pub state: FolderState,
    /// Copies a sync app made beside `config.json`.
    pub conflict_files: Vec<String>,
    /// Unix seconds the folder's file was last read or written.
    pub last_sync: Option<u64>,
    /// The file in the local settings folder that kept this PC's side of a clash.
    pub kept: Option<String>,
    pub problem: Option<Problem>,
}

#[derive(Default)]
struct Status {
    folder: Option<PathBuf>,
    info: Info,
    /// The shared part as last read from or written to the folder.
    base: Option<Value>,
    /// What this PC holds of it after taking in the folder's file, which is `base` unless taking it in cleared something.
    held: Option<Value>,
    base_dirty: bool,
    /// What this PC accepted of the shared part: see `sync::accepted_of`. Only ever in a file on this PC.
    accepted: BTreeSet<String>,
    accepted_dirty: bool,
    /// Hash of the bytes Seam last wrote to the folder, so the watcher can tell its own writes.
    own: Option<u64>,
}

static STATUS: Mutex<Status> = Mutex::new(Status {
    folder: None,
    info: Info {
        path: None,
        state: FolderState::Local,
        conflict_files: Vec::new(),
        last_sync: None,
        kept: None,
        problem: None,
    },
    base: None,
    held: None,
    base_dirty: false,
    accepted: BTreeSet::new(),
    accepted_dirty: false,
    own: None,
});

/// Bumped each time the tray takes in the folder's file, so a save that was queued before it
/// doesn't write the folder from what it knew then.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn lock() -> MutexGuard<'static, Status> {
    STATUS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

pub fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub fn info() -> Info {
    lock().info.clone()
}

pub fn state() -> FolderState {
    lock().info.state
}

pub fn folder() -> Option<PathBuf> {
    lock().folder.clone()
}

/// Where the local copy and the side files live.
pub fn local_dir() -> PathBuf {
    Config::path().parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Start using `folder`, or none. `base` is the shared part as this PC last saw it, if it kept one.
pub fn point_at(folder: Option<PathBuf>, base: Option<Value>) {
    let mut status = lock();
    let state = if folder.is_some() { FolderState::Synced } else { FolderState::Local };
    let path = folder.as_ref().map(|f| f.display().to_string());
    let accepted = std::mem::take(&mut status.accepted);
    *status = Status {
        folder,
        info: Info { path, state, ..Info::default() },
        base: base.clone(),
        held: base,
        accepted,
        ..Status::default()
    };
}

/// The shared part Seam kept from last time.
pub fn load_base() -> Option<Value> {
    serde_json::from_slice(&std::fs::read(local_dir().join(BASE_FILE)).ok()?).ok()
}

pub fn base() -> Option<Value> {
    lock().base.clone()
}

/// What this PC holds of the shared part: `base` as taking it in left it.
pub fn held() -> Option<Value> {
    lock().held.clone()
}

/// The shared part is now `base` in the folder, and this PC holds `held` of it.
pub fn set_base(base: Value, held: Value) {
    let mut status = lock();
    status.base = Some(base);
    status.held = Some(held);
    status.base_dirty = true;
}

pub fn accepted() -> BTreeSet<String> {
    lock().accepted.clone()
}

/// Take what this PC accepts from its own config: every row that launches and every workspace that
/// closes other windows in it. A row the folder brought with its launch off is not in it, until the
/// user turns it on. Nothing is read back from the file the set is written to.
pub fn accept(config: &Config) {
    let now = sync::accepted_of(&shared_of(config));
    let mut status = lock();
    if status.accepted != now {
        status.accepted = now;
        status.accepted_dirty = true;
    }
}

/// Write the base and what was accepted to their files if they changed. Called from the save thread.
pub fn flush_base() {
    let accepted = {
        let mut status = lock();
        std::mem::take(&mut status.accepted_dirty).then(|| status.accepted.clone())
    };
    if let Some(Ok(text)) = accepted.map(|a| serde_json::to_string_pretty(&a)) {
        let _ = write_atomic(&local_dir().join(ACCEPTED_FILE), &text);
    }
    let base = {
        let mut status = lock();
        if !std::mem::take(&mut status.base_dirty) {
            return;
        }
        status.base.clone()
    };
    let Some(Ok(text)) = base.map(|b| serde_json::to_string_pretty(&b)) else { return };
    let _ = write_atomic(&local_dir().join(BASE_FILE), &text);
}

/// Change the state. True if it was another.
pub fn set_state(state: FolderState) -> bool {
    std::mem::replace(&mut lock().info.state, state) != state
}

pub fn set_problem(problem: Option<Problem>) -> bool {
    std::mem::replace(&mut lock().info.problem, problem) != problem
}

pub fn set_kept(file: Option<String>) {
    lock().info.kept = file;
}

pub fn set_conflicts(files: Vec<String>) -> bool {
    let mut status = lock();
    status.info.conflict_files != files && {
        status.info.conflict_files = files;
        true
    }
}

/// The folder's file was read or written fine just now.
pub fn synced_at(unix: u64) {
    let mut status = lock();
    status.info.state = FolderState::Synced;
    // A config too big to write stays so until it shrinks.
    if status.info.problem != Some(Problem::TooBig) {
        status.info.problem = None;
    }
    status.info.last_sync = Some(unix);
}

pub fn hash(bytes: &[u8]) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// Whether these bytes are what Seam last wrote to the folder itself.
pub fn is_own_write(bytes: &[u8]) -> bool {
    lock().own == Some(hash(bytes))
}

/// The folder's `config.json`: None when it isn't there, an error when the folder can't be read.
/// Never more than the limit, so a hostile file can't fill memory.
pub fn read_theirs(folder: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let path = folder.join(SHARED_FILE);
    // Never a link: it could lead anywhere, such as a slow share or a device.
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other("config.json is a link"));
    }
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && folder.is_dir() => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(sync::MAX_READ as u64 + 1).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// The names in the folder that a sync app made as copies of `config.json`.
pub fn conflict_files(folder: &Path) -> Vec<String> {
    let names = std::fs::read_dir(folder).into_iter().flatten().flatten();
    let mut found: Vec<String> =
        names.filter_map(|e| e.file_name().into_string().ok()).filter(|n| sync::is_conflict_copy(n)).collect();
    found.sort();
    found.truncate(MAX_CONFLICTS);
    found.iter_mut().for_each(|n| n.truncate(MAX_NAME));
    found
}

/// The shared part of a config, as the file holds it and without the stamp.
pub fn shared_of(config: &Config) -> Value {
    let mut shared = sync::split(&serde_json::to_value(config).unwrap_or_default()).0;
    if let Some(map) = shared.as_object_mut() {
        map.remove("sync");
    }
    shared
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// Write the shared part to the folder with a stamp of who wrote it, through a temp file that
/// replaces `config.json` in one step (`ReplaceFileW` keeps the file's identity, which a sync app
/// prefers to a delete and create). A sync conflict never leaves half a file.
fn write_shared(folder: &Path, shared: &Value, unix: u64) -> std::io::Result<()> {
    let mut value = shared.clone();
    value["sync"] = json!({ "pc": computer_name(), "at": unix });
    let text = serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?;
    lock().own = Some(hash(text.as_bytes()));
    // A name nobody can have planted: `create_new` never writes through a link or into a file that is there.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let (dest, tmp) = (folder.join(SHARED_FILE), folder.join(format!("config.{}-{nanos}.tmp", std::process::id())));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    drop(file);
    replace(&tmp, &dest).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Write a file in `dir` the way the shared file is: through a new temp file with a name nobody can have
/// planted, onto the name, never through a link that is already there.
pub fn write_safe(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let dest = dir.join(name);
    if std::fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other("the file is a link"));
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let tmp = dir.join(format!("{name}.{}-{nanos}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    std::io::Write::write_all(&mut file, bytes)?;
    drop(file);
    std::fs::rename(&tmp, &dest).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(windows)]
fn replace(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};
    use windows::core::HSTRING;
    if dest.exists() {
        let (to, from) = (HSTRING::from(&*dest.to_string_lossy()), HSTRING::from(&*tmp.to_string_lossy()));
        if unsafe { ReplaceFileW(&to, &from, None, REPLACEFILE_IGNORE_MERGE_ERRORS, None, None) }.is_ok() {
            return Ok(());
        }
    }
    std::fs::rename(tmp, dest)
}

#[cfg(not(windows))]
fn replace(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, dest)
}

/// Write the shared part of a config to the folder, for the first time or over what is there. For
/// "Keep this PC's settings", which has copied theirs aside first.
pub fn write_over(folder: &Path, config: &Config, unix: u64) -> std::io::Result<()> {
    let shared = shared_of(config);
    write_shared(folder, &shared, unix)?;
    set_base(shared.clone(), shared);
    synced_at(unix);
    Ok(())
}

/// What a save did about the folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    /// Nothing to tell the tray.
    Quiet,
    /// The state or the problem changed, so the tray should say so.
    Changed,
    /// The folder's file isn't what this PC last saw: the watcher should read it, and the tray merge.
    Wake,
}

/// After a save: write the shared part to the folder if it changed, and only if the file there is
/// still what this PC last saw. A file that changed meanwhile is the watcher's to read and the
/// tray's to merge, never overwritten from an older base.
pub fn push(config: &Config, unix: u64, allowed: &dyn Fn(&Path) -> bool) -> Push {
    let was_big = info().problem == Some(Problem::TooBig);
    let pushed = push_checked(config, unix, allowed);
    // The note about a config that was too big goes as soon as it isn't.
    if was_big && pushed == Push::Quiet && info().problem != Some(Problem::TooBig) { Push::Changed } else { pushed }
}

fn push_checked(config: &Config, unix: u64, allowed: &dyn Fn(&Path) -> bool) -> Push {
    let Some(folder) = folder() else { return Push::Quiet };
    let ours = shared_of(config);
    // Seam never writes a file it would refuse to read: over the counts, nothing goes to the folder.
    if !sync::within_limits(&ours) {
        return if set_problem(Some(Problem::TooBig)) { Push::Changed } else { Push::Quiet };
    }
    let was_big = info().problem == Some(Problem::TooBig);
    if was_big {
        set_problem(None);
    }
    if info().problem == Some(Problem::Newer) {
        return Push::Quiet;
    }
    let (base, held) = (base(), held());
    // The folder can't be reached or written: this PC carries on with its own copy.
    let down = || if set_state(FolderState::Unavailable) { Push::Changed } else { Push::Quiet };
    // The folder is looked at again before this writes into it.
    if !allowed(&folder) {
        return down();
    }
    let mut read = read_theirs(&folder);
    // A sync app may replace the file by deleting and creating it: look once more before writing over it.
    if matches!(read, Ok(None)) && base.is_some() {
        std::thread::sleep(std::time::Duration::from_secs(1));
        read = read_theirs(&folder);
    }
    let there = match read {
        Err(_) => return down(),
        Ok(None) => None,
        Ok(Some(bytes)) => match sync::attempt(&bytes, 0) {
            Attempt::Take(theirs) => Some(shared_of(&theirs)),
            Attempt::Newer => return if set_problem(Some(Problem::Newer)) { Push::Changed } else { Push::Quiet },
            Attempt::Retry | Attempt::GiveUp => return Push::Wake,
        },
    };
    match (&there, &base) {
        (Some(theirs), Some(base)) if theirs != base => return Push::Wake,
        (Some(_), None) => return Push::Wake,
        _ => {}
    }
    if there.is_some() && held.as_ref() == Some(&ours) {
        return Push::Quiet;
    }
    // What the folder had goes back for everything this PC didn't change: what it holds is only its own view.
    let out = match (&held, &base) {
        (Some(held), Some(base)) => sync::outgoing(held, &ours, base),
        _ => ours.clone(),
    };
    match write_shared(&folder, &out, unix) {
        Ok(()) => {
            set_base(out, ours);
            let was = state();
            synced_at(unix);
            if was == FolderState::Synced { Push::Quiet } else { Push::Changed }
        }
        Err(_) => down(),
    }
}

/// Tests that use the folder status take this, since it is one for the whole process.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Keep this PC's side of a clash beside the local config. The file's name, once it is written.
pub fn keep_conflict(ours: &Value, unix: u64) -> Option<String> {
    let name = format!("config.sync-conflict-{}.json", sync::stamp(unix));
    let text = serde_json::to_string_pretty(ours).ok()?;
    write_safe(&local_dir(), &name, text.as_bytes()).ok()?;
    Some(name)
}
