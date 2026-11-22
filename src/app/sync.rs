//! Settings in a synced folder (brief G, D751-D760): the `seam-sync` thread that watches the folder, the
//! tray's side of taking a change in, and choosing the folder. The merge itself is `config::sync`.

use super::*;
use seam::api::FolderTake;
use seam::config::folder::{self, FolderState, Problem};
use seam::config::sync::{self as shared, Attempt, Drives};
use seam::config::{Config, SyncStamp};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetDriveTypeW, OPEN_EXISTING,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects};
use windows::core::HSTRING;

/// The folder's file changed, or its state did: look at `INBOX`.
pub(super) const WM_APP_SYNC: u32 = WM_APP + 46;
/// Retries taking a change in while a switch or capture runs (D758).
pub(super) const TIMER_SYNC: usize = 16;
const SYNC_RETRY_MS: u32 = 500;
/// A change to the file waits this long for more of them, such as a sync app writing in pieces (D755).
const DEBOUNCE: Duration = Duration::from_secs(2);
/// How long a file that didn't parse waits before it is read again (D756).
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// How often the folder is looked at when it can't be watched (D755).
const POLL: Duration = Duration::from_secs(30);

/// What the folder's file was when read.
pub(super) enum Seen {
    /// There is no `config.json` there.
    Missing,
    /// A file that parsed.
    File(Box<Config>),
    /// The folder is back and holds only what this PC wrote: what changed meanwhile is written now.
    Resync,
}

/// "Keep settings in a folder…": what the folder holds, as the worker read it.
enum Found {
    Missing,
    Config(Box<Config>),
    Newer,
    Unreadable,
}

struct Pick {
    reply: Sender<Reply>,
    folder: PathBuf,
    take: Option<FolderTake>,
    found: Found,
}

#[derive(Default)]
struct Inbox {
    seen: Option<Seen>,
    pick: Option<Pick>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox { seen: None, pick: None });

fn inbox() -> std::sync::MutexGuard<'static, Inbox> {
    INBOX.lock().unwrap_or_else(|e| e.into_inner())
}

fn post(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_SYNC, WPARAM(0), LPARAM(0));
    }
}

/// What `GetDriveTypeW` says of a drive on this PC's own disks.
const DRIVE_FIXED: u32 = 3;

/// The drives Windows says are on this PC's own disks.
struct FixedDrives;

impl Drives for FixedDrives {
    fn fixed(&self, letter: char) -> bool {
        drive_type(letter) == DRIVE_FIXED
    }

    fn mounted(&self, letter: char) -> bool {
        // DRIVE_UNKNOWN and DRIVE_NO_ROOT_DIR: no volume there (now).
        drive_type(letter) > 1
    }
}

fn drive_type(letter: char) -> u32 {
    unsafe { GetDriveTypeW(&HSTRING::from(format!("{letter}:\\"))) }
}

// ---- the watcher ----

struct Watch {
    stop: isize,
    wake: isize,
    thread: JoinHandle<()>,
}

static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

/// Which watcher is the current one. A thread that is stopping, or was replaced, finds its number is
/// no longer this and drops what it read instead of handing it to the tray.
static WATCH_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn begin_watch() -> u64 {
    WATCH_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

fn current(id: u64) -> bool {
    WATCH_ID.load(std::sync::atomic::Ordering::SeqCst) == id
}

fn watching() -> std::sync::MutexGuard<'static, Option<Watch>> {
    WATCH.lock().unwrap_or_else(|e| e.into_inner())
}

/// Watch `folder` from a thread of its own. The thread reads the file first, then on each change.
fn watch(folder: PathBuf, hwnd: isize) {
    unwatch();
    let events = unsafe { (CreateEventW(None, true, false, None), CreateEventW(None, false, false, None)) };
    let (Ok(stop), Ok(wake)) = events else { return };
    let (stop_id, wake_id) = (stop.0 as isize, wake.0 as isize);
    let id = begin_watch();
    let spawned =
        std::thread::Builder::new().name("seam-sync".into()).spawn(move || run(&folder, hwnd, (stop_id, wake_id), id));
    match spawned {
        Ok(thread) => *watching() = Some(Watch { stop: stop_id, wake: wake_id, thread }),
        Err(_) => unsafe {
            let _ = CloseHandle(stop);
            let _ = CloseHandle(wake);
        },
    }
}

/// Stop watching. The thread may sit in a read of a file that is still downloading, so another
/// thread waits for it, and the tray never does.
fn unwatch() {
    begin_watch();
    let Some(watch) = watching().take() else { return };
    unsafe {
        let _ = SetEvent(HANDLE(watch.stop as *mut _));
    }
    let _ = std::thread::Builder::new().name("seam-sync-stop".into()).spawn(move || {
        let _ = watch.thread.join();
        unsafe {
            let _ = CloseHandle(HANDLE(watch.stop as *mut _));
            let _ = CloseHandle(HANDLE(watch.wake as *mut _));
        }
    });
}

/// Ask the watcher to read the folder's file now: a save found it isn't what this PC last saw.
fn wake() {
    if let Some(watch) = watching().as_ref() {
        unsafe {
            let _ = SetEvent(HANDLE(watch.wake as *mut _));
        }
    }
}

/// The names `ReadDirectoryChangesW` put in its buffer.
fn changed_names(buf: &[u8]) -> Vec<String> {
    let (mut names, mut at) = (Vec::new(), 0usize);
    while at + 12 <= buf.len() {
        let word = |n: usize| u32::from_le_bytes([buf[n], buf[n + 1], buf[n + 2], buf[n + 3]]) as usize;
        let (next, len) = (word(at), word(at + 8));
        let Some(raw) = buf.get(at + 12..at + 12 + len) else { break };
        let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        names.push(String::from_utf16_lossy(&units));
        if next == 0 {
            break;
        }
        at += next;
    }
    names
}

fn open_dir(folder: &Path) -> Option<HANDLE> {
    unsafe {
        CreateFileW(
            &HSTRING::from(&*folder.to_string_lossy()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )
        .ok()
    }
}

/// The size and write time of the file, for the checks made while the folder can't be watched.
fn file_mark(folder: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(folder.join(folder::SHARED_FILE)).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn now() -> Instant {
    Instant::now()
}

fn run(folder: &Path, hwnd: isize, (stop, wake): (isize, isize), id: u64) {
    let (stop, wake) = (HANDLE(stop as *mut _), HANDLE(wake as *mut _));
    let Ok(io) = (unsafe { CreateEventW(None, true, false, None) }) else { return };
    let mut overlapped = OVERLAPPED { hEvent: io, ..Default::default() };
    // `u32`s, so the notifications in it are aligned.
    let mut buffer = vec![0u32; 4096];
    let mut dir = open_dir(folder);
    let mut armed = false;
    // When to read the file next, and how many reads of this change have failed to parse.
    let mut due = Some(now());
    let mut tries = 0u8;
    let mut mark = file_mark(folder);
    if folder::set_conflicts(folder::conflict_files(folder)) {
        post(hwnd);
    }
    loop {
        if let (Some(handle), false) = (dir, armed) {
            let size = (buffer.len() * 4) as u32;
            let filter = FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE;
            unsafe {
                let _ = ResetEvent(io);
                let started = windows::Win32::Storage::FileSystem::ReadDirectoryChangesW(
                    handle,
                    buffer.as_mut_ptr().cast(),
                    size,
                    false,
                    filter,
                    None,
                    Some(&mut overlapped),
                    None,
                );
                if started.is_ok() {
                    armed = true;
                } else {
                    let _ = CloseHandle(handle);
                    dir = None;
                }
            }
        }
        let wait = match due {
            Some(at) => at.saturating_duration_since(now()),
            None if dir.is_some() && folder::state() != FolderState::Unavailable => {
                Duration::from_millis(u64::from(INFINITE))
            }
            None => POLL,
        };
        let handles = [stop, wake, io];
        let count = if armed { 3 } else { 2 };
        let woken =
            unsafe { WaitForMultipleObjects(&handles[..count], false, wait.as_millis().min(0xFFFF_FFFE) as u32) };
        if !current(id) {
            break;
        }
        match woken.0.wrapping_sub(WAIT_OBJECT_0.0) {
            0 => break,
            1 => {
                // A save that found the folder down waits before it looks again.
                let wait = if folder::state() == FolderState::Unavailable { POLL } else { Duration::ZERO };
                due = Some(now() + wait);
                tries = 0;
            }
            2 => {
                armed = false;
                let mut got = 0u32;
                let finished = unsafe { GetOverlappedResult(dir.unwrap_or_default(), &overlapped, &mut got, false) };
                if finished.is_err() {
                    // The folder went away under the watch.
                    if let Some(handle) = dir.take() {
                        unsafe {
                            let _ = CloseHandle(handle);
                        }
                    }
                    folder::set_state(FolderState::Unavailable);
                    post(hwnd);
                    continue;
                }
                let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast(), buffer.len() * 4) };
                // Nothing returned means the buffer overflowed: anything may have changed.
                let names = if got == 0 {
                    vec![folder::SHARED_FILE.to_string()]
                } else {
                    changed_names(&bytes[..got as usize])
                };
                if folder::set_conflicts(folder::conflict_files(folder)) {
                    post(hwnd);
                }
                if names.iter().any(|n| n.eq_ignore_ascii_case(folder::SHARED_FILE)) {
                    due = Some(now() + DEBOUNCE);
                    tries = 0;
                }
            }
            _ if woken == WAIT_TIMEOUT => {
                if due.is_some_and(|at| at <= now()) {
                    due = read_once(folder, hwnd, &mut tries, id);
                    mark = file_mark(folder);
                    if folder::set_conflicts(folder::conflict_files(folder)) {
                        post(hwnd);
                    }
                    if dir.is_none() {
                        dir = open_dir(folder);
                    }
                    continue;
                }
                // A write that failed, or a file that was locked, with the watch still on: look again.
                if dir.is_some() && folder::state() == FolderState::Unavailable {
                    due = Some(now());
                    continue;
                }
                // Not watched: see whether the folder is back, or the file changed.
                if dir.is_none() {
                    let seen = file_mark(folder);
                    dir = open_dir(folder);
                    if dir.is_some() || seen != mark {
                        mark = seen;
                        due = Some(now());
                        tries = 0;
                    }
                    if folder::set_conflicts(folder::conflict_files(folder)) {
                        post(hwnd);
                    }
                }
            }
            _ => break,
        }
    }
    unsafe {
        if let Some(handle) = dir {
            if armed {
                let _ = CancelIoEx(handle, Some(&overlapped));
                let mut got = 0u32;
                let _ = GetOverlappedResult(handle, &overlapped, &mut got, true);
            }
            let _ = CloseHandle(handle);
        }
        let _ = CloseHandle(io);
    }
}

/// Read the file once and hand what came of it to the tray. The time to read again, if it should.
fn read_once(folder: &Path, hwnd: isize, tries: &mut u8, id: u64) -> Option<Instant> {
    let was_down = folder::state() == FolderState::Unavailable;
    // The rules for a folder hold each time it is read: it may have become a link to somewhere else.
    let refused = shared::folder_ok(&folder.to_string_lossy(), &package::settings_dir(), &FixedDrives)
        .err()
        .filter(|why| *why != shared::NOT_THERE);
    let read = if refused.is_some() {
        Err(std::io::Error::other("not a folder Seam may use"))
    } else {
        folder::read_theirs(folder)
    };
    // A watcher that was replaced or stopped while it read says nothing.
    if !current(id) {
        return None;
    }
    let bytes = match read {
        Err(_) => {
            // Gone, or locked: the copy on this PC carries on, and the folder is looked at every 30 s.
            if folder::set_state(FolderState::Unavailable) {
                log::info!("sync: the settings folder isn't available");
            }
            post(hwnd);
            return Some(now() + POLL);
        }
        Ok(bytes) => bytes,
    };
    let Some(bytes) = bytes else {
        folder::synced_at(unix_now());
        inbox().seen = Some(Seen::Missing);
        post(hwnd);
        return None;
    };
    // What Seam wrote itself, which the tray already holds.
    if folder::is_own_write(&bytes) {
        folder::synced_at(unix_now());
        if was_down {
            inbox().seen = Some(Seen::Resync);
        }
        post(hwnd);
        return None;
    }
    match shared::attempt_as(&bytes, *tries, Some(&folder::computer_name())) {
        Attempt::Take(config) => {
            folder::synced_at(unix_now());
            inbox().seen = Some(Seen::File(config));
            *tries = 0;
            post(hwnd);
            None
        }
        Attempt::Newer => {
            log::info!("sync: config.json in the folder is from a newer Seam");
            folder::set_problem(Some(Problem::Newer));
            *tries = 0;
            post(hwnd);
            None
        }
        Attempt::Retry => {
            *tries += 1;
            Some(now() + RETRY_AFTER)
        }
        Attempt::GiveUp => {
            log::info!("sync: config.json in the folder can't be read");
            folder::set_problem(Some(Problem::Unreadable));
            *tries = 0;
            post(hwnd);
            None
        }
    }
}

// ---- the tray's side ----

/// Start watching the folder the config points at, if it does.
pub(super) fn start(app: &mut App) {
    let Some(path) = app.cfg.config_folder.clone() else { return };
    folder::point_at(Some(PathBuf::from(&path)), folder::load_base());
    // What this PC launches now is what it accepts. The file of it is only ever written.
    folder::accept(&app.cfg);
    // The same rules as when it was chosen. A folder that isn't there yet, or on a drive that isn't
    // mounted yet, is looked for every 30 s.
    let own = package::settings_dir();
    if let Err(why) = shared::folder_ok(&path, &own, &FixedDrives)
        && !shared::may_appear(&path, &own, &FixedDrives)
    {
        log::info!("sync: the settings folder in config.json isn't a folder Seam may use: {why}");
        folder::set_state(FolderState::Unavailable);
        return;
    }
    watch(PathBuf::from(path), app.hwnd.0 as isize);
}

/// After a save on the save thread: write the folder when it needs it, and tell the tray what came of it.
pub(super) fn saved(hwnd: isize, config: &Config, generation: u64) {
    if generation == folder::generation() {
        match folder::push(config, unix_now()) {
            folder::Push::Quiet => {}
            folder::Push::Changed => {
                post(hwnd);
                // Down: look again in 30 s, and write what changed meanwhile.
                if folder::state() == FolderState::Unavailable {
                    wake();
                }
            }
            folder::Push::Wake => wake(),
        }
    }
    folder::flush_base();
}

fn busy(app: &App) -> bool {
    switcher::switching() || app.capturing.is_some() || app.closing.is_some()
}

/// `WM_APP_SYNC`: the folder's file or its state changed.
pub(super) fn incoming(app: &mut App) {
    if (inbox().seen.is_some() || inbox().pick.is_some()) && busy(app) {
        unsafe { SetTimer(Some(app.hwnd), TIMER_SYNC, SYNC_RETRY_MS, None) };
        return;
    }
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_SYNC);
    }
    let (seen, pick) = {
        let mut inbox = inbox();
        (inbox.seen.take(), inbox.pick.take())
    };
    // What a watcher read of a folder this PC no longer uses is dropped.
    let seen = seen.filter(|_| folder::state() != FolderState::Local);
    if let Some(pick) = pick {
        picked(app, pick);
    }
    match seen {
        Some(Seen::File(theirs)) => absorb(app, &theirs),
        Some(Seen::Resync) => bump_and_persist(app),
        Some(Seen::Missing) => {
            // The file is gone or was never there: this PC's copy is written back.
            bump_and_persist(app);
        }
        None => {}
    }
    warn_unavailable(app);
    publish(app);
}

fn bump_and_persist(app: &mut App) {
    folder::bump_generation();
    persist(app);
}

/// The one balloon of a run for a folder that can't be reached, unless it would take the place of an offer.
fn warn_unavailable(app: &mut App) {
    if folder::state() != FolderState::Unavailable || app.sync_warned {
        return;
    }
    app.sync_warned = true;
    let offered = matches!(
        app.balloon_action,
        BalloonAction::LineUp
            | BalloonAction::RestoreAfterScreens
            | BalloonAction::OpenWorkspace
            | BalloonAction::RestoreSession
    );
    if !offered {
        notify(
            app,
            "Settings folder not available",
            "Seam is using the copy on this PC. Click to open General",
            BalloonAction::OpenPanel("general"),
        );
    }
}

/// Merge the folder's file into what this PC has (D754) and apply the result.
fn absorb(app: &mut App, theirs: &Config) {
    let raw = folder::shared_of(theirs);
    // What this PC holds is `held`, the file as it last read it and guarded: the merge starts from that,
    // so a setting this PC changed isn't mistaken for a clash with a row the guard changed.
    let base = folder::held();
    let mut cleaned = raw.clone();
    shared::guard(&mut cleaned, &folder::accepted());
    let ours_config = serde_json::to_value(&app.cfg).unwrap_or_default();
    let ours = folder::shared_of(&app.cfg);
    // No base is the first time the folder is read here: its shared settings are taken (D753).
    let merged = match &base {
        Some(base) => shared::merge3(base, &ours, &cleaned),
        None => shared::Merged { value: cleaned.clone(), clashed: Vec::new() },
    };
    let joined = shared::join(&merged.value, &ours_config);
    let Ok(mut next) = serde_json::from_value::<Config>(joined) else { return };
    next.layouts.clear();
    next.sync = None;
    folder::bump_generation();
    folder::set_base(raw, cleaned);
    if merged.clashed.is_empty() {
        folder::set_kept(None);
    } else {
        // Theirs won. This PC's side is kept in a file, off the tray thread.
        log::info!("sync: both sides changed {}, the folder's was taken", merged.clashed.join(", "));
        let (unix, hwnd) = (unix_now(), app.hwnd.0 as isize);
        let _ = std::thread::Builder::new().name("seam-sync-keep".into()).spawn(move || {
            folder::set_kept(folder::keep_conflict(&ours, unix));
            post(hwnd);
        });
    }
    if serde_json::to_value(&next).ok() != serde_json::to_value(&app.cfg).ok() {
        apply(app, next);
    } else {
        persist(app);
    }
}

/// Take a merged config the way a patch does, without an Undo: the parts that need it are refreshed
/// and the Seam window is told (D758).
fn apply(app: &mut App, next: Config) {
    app.cfg = next;
    forget_deleted(app);
    persist(app);
    apply_hotkeys(app);
    relink(app);
    publish(app);
    api_server::broadcast_to_panels("config.reloaded", &json!({ "source": "folder" }));
}

// ---- choosing the folder ----

/// `config.set_folder`: a path, or none to keep the settings on this PC only.
pub(super) fn set_folder(app: &mut App, path: Option<String>, take: Option<FolderTake>, reply: Sender<Reply>) {
    let Some(path) = path else {
        turn_off(app);
        let _ = reply.send(Ok(json!({ "ok": true, "state": FolderState::Local })));
        return;
    };
    let own = package::settings_dir();
    let chosen = shared::folder_ok(&path, &own, &FixedDrives)
        .and_then(|f| if shared::same_folder(&f, &folder::local_dir()) { Err(shared::OWN_FOLDER) } else { Ok(f) });
    let chosen = match chosen {
        Ok(chosen) => chosen,
        Err(why) => {
            let _ = reply.send(refused(why));
            return;
        }
    };
    let ours = serde_json::to_string_pretty(&app.cfg).unwrap_or_default();
    let hwnd = app.hwnd.0 as isize;
    let worker = {
        let (chosen, reply) = (chosen.clone(), reply.clone());
        std::thread::Builder::new()
            .name("seam-sync-pick".into())
            .spawn(move || read_choice(chosen, take, ours, hwnd, reply))
    };
    if worker.is_err() {
        let _ = reply.send(refused("Seam couldn't read that folder"));
    }
}

/// What a chosen folder holds, read where a file that is still downloading can block.
fn read_choice(chosen: PathBuf, take: Option<FolderTake>, ours: String, hwnd: isize, reply: Sender<Reply>) {
    let bytes = folder::read_theirs(&chosen);
    let found = match &bytes {
        Err(_) => Found::Unreadable,
        Ok(None) => Found::Missing,
        Ok(Some(bytes)) => match shared::attempt(bytes, shared::READ_TRIES - 1) {
            Attempt::Take(config) => Found::Config(config),
            Attempt::Newer => Found::Newer,
            Attempt::Retry | Attempt::GiveUp => Found::Unreadable,
        },
    };
    // The side the choice replaces is kept first.
    let kept = match (&found, take, &bytes) {
        (Found::Config(_), Some(FolderTake::Here), Ok(Some(bytes))) => {
            folder::write_safe(&chosen, "config.before-sync.json", bytes)
        }
        (Found::Config(_), Some(FolderTake::Folder), _) => {
            folder::write_safe(&folder::local_dir(), "config.before-sync.json", ours.as_bytes())
        }
        _ => Ok(()),
    };
    if kept.is_err() {
        let _ = reply.send(refused("Nothing changed, because Seam couldn't keep a copy of the settings it replaces"));
        return;
    }
    inbox().pick = Some(Pick { reply, folder: chosen, take, found });
    post(hwnd);
}

/// The worker has read the folder: answer the Seam window, and switch to it if that is what was asked.
fn picked(app: &mut App, pick: Pick) {
    let Pick { reply, folder: chosen, take, found } = pick;
    let answer = match (found, take) {
        (Found::Unreadable, _) => refused("That folder has a settings file Seam can't read"),
        (Found::Newer, _) => refused("That folder has settings from a newer Seam. Update Seam on this PC to use them"),
        (Found::Missing, _) => {
            point(app, chosen, None);
            Ok(json!({ "ok": true, "state": FolderState::Synced }))
        }
        (Found::Config(theirs), None) => {
            let mut stamp = theirs.sync.clone().unwrap_or_default();
            // Only a short plain name is shown.
            stamp.pc = stamp.pc.chars().filter(|c| !c.is_control()).take(40).collect();
            let who = json!({ "pc": stamp.pc, "at": stamp.at });
            refused(if stamp == SyncStamp::default() { "has_config".to_string() } else { format!("has_config {who}") })
        }
        (Found::Config(theirs), Some(FolderTake::Here)) => {
            // The folder's file is what this PC has seen; this PC's settings are then written over it.
            let raw = folder::shared_of(&theirs);
            point(app, chosen, Some(raw));
            Ok(json!({ "ok": true, "state": FolderState::Synced }))
        }
        (Found::Config(theirs), Some(FolderTake::Folder)) => {
            point(app, chosen, None);
            absorb(app, &theirs);
            Ok(json!({ "ok": true, "state": FolderState::Synced }))
        }
    };
    let _ = reply.send(answer);
}

/// Keep the settings in `chosen` from now on. `seen` is the shared part already in it that this PC
/// writes over, if there is one.
fn point(app: &mut App, chosen: PathBuf, seen: Option<serde_json::Value>) {
    app.cfg.config_folder = Some(chosen.display().to_string());
    inbox().seen = None;
    folder::point_at(Some(chosen.clone()), None);
    if let Some(seen) = seen {
        folder::set_base(seen.clone(), seen);
    }
    watch(chosen, app.hwnd.0 as isize);
    bump_and_persist(app);
}

/// Keep the settings on this PC only. The file in the folder stays as it is.
fn turn_off(app: &mut App) {
    unwatch();
    inbox().seen = None;
    app.cfg.config_folder = None;
    folder::point_at(None, None);
    for name in ["config.sync-base.json", "config.sync-accepted.json"] {
        let _ = std::fs::remove_file(folder::local_dir().join(name));
    }
    app.sync_warned = false;
    persist(app);
    publish(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The watcher and its number are one for the process.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn names_are_read_out_of_the_change_buffer() {
        let entry = |name: &str, last: bool| {
            let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let next = if last { 0 } else { (12 + units.len() + units.len() % 4) as u32 };
            let mut bytes = [next.to_le_bytes(), 3u32.to_le_bytes(), (units.len() as u32).to_le_bytes()].concat();
            bytes.extend(&units);
            bytes.resize(if last { bytes.len() } else { next as usize }, 0);
            bytes
        };
        let buffer = [entry("config.json", false), entry("other.tmp", true)].concat();
        assert_eq!(changed_names(&buffer), ["config.json", "other.tmp"]);
        assert!(changed_names(&buffer[..10]).is_empty());
        assert!(changed_names(&[]).is_empty());
    }

    fn wait_for_file(seconds: u64) -> Option<Box<Config>> {
        let until = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < until {
            if let Some(Seen::File(config)) = inbox().seen.take() {
                return Some(config);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    #[test]
    fn the_watcher_reads_the_file_first_and_then_each_change() {
        let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("seam-test-{}-watch", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |enabled: bool| {
            let text = serde_json::to_string(&Config { enabled, ..Config::default() }).unwrap();
            std::fs::write(dir.join("config.json"), text).unwrap();
        };
        write(false);
        folder::point_at(Some(dir.clone()), None);
        watch(dir.clone(), 0);
        assert!(wait_for_file(5).is_some_and(|c| !c.enabled), "the first read");
        // A change is read after the debounce.
        write(true);
        assert!(wait_for_file(10).is_some_and(|c| c.enabled), "the change");
        std::fs::write(dir.join("config (1).json"), b"{}").unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while folder::info().conflict_files.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(folder::info().conflict_files, ["config (1).json"]);
        unwatch();
        folder::point_at(None, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_watcher_that_was_replaced_or_stopped_is_no_longer_current() {
        let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let first = begin_watch();
        assert!(current(first));
        let second = begin_watch();
        assert!(current(second) && !current(first));
        // Stopping makes the running one stale too, so what it read late is dropped.
        unwatch();
        assert!(!current(second));
    }
}
