//! "Close this workspace" and "Minimize this workspace", on a worker: a snapshot can be
//! slow with hung windows. Closing is a close request only, the call Other windows: Close
//! makes (D703); Seam never ends a process.

use super::*;
use seam::workspace::{Fact, Why};

/// A window of the workspace, with the row that matched it.
#[derive(Clone, Debug)]
pub struct Entry {
    pub hwnd: isize,
    pub row: usize,
    /// The app, as the workspace table names it.
    pub app: String,
    pub title: String,
    /// Where it is in the stacking, 0 is the topmost.
    pub z: usize,
}

/// The windows a workspace has open: the ones a close or minimize takes, and the ones it keeps.
#[derive(Clone, Debug, Default)]
pub struct Owned {
    pub take: Vec<Entry>,
    pub kept: Vec<(Entry, Why)>,
}

/// What the worker does once it has looked.
pub enum Verb {
    /// Only say which windows there are.
    List,
    /// Ask these windows to close, the ones of them the fresh look still matches.
    Close(Vec<isize>),
    /// Minimize every window taken.
    Minimize,
}

pub struct CloseJob {
    pub owner: isize,
    pub ws: Workspace,
    /// Lower-case exe names never taken: the zones' exclude list.
    pub exclude: Vec<String>,
    pub places: bool,
    pub verb: Verb,
}

pub enum Done {
    Listed(Owned),
    Closed {
        asked: usize,
        changed: usize,
    },
    /// The windows minimized, topmost first.
    Minimized(Vec<isize>),
}

static DONE: Mutex<Option<Done>> = Mutex::new(None);

/// Start a job. The main thread gets `WM_APP_CLOSED` when it's done.
pub fn start(job: CloseJob) -> bool {
    let spawned = std::thread::Builder::new().name("seam-close".into()).spawn(move || {
        let _com = Com::enter();
        let owned = windows_of(&job.ws, &job.exclude, job.places);
        let done = match job.verb {
            Verb::List => Done::Listed(owned),
            Verb::Close(asked) => {
                let fresh: Vec<isize> = owned.take.iter().map(|e| e.hwnd).collect();
                let (ours, changed) = workspace::still_ours(&asked, &fresh);
                close(&ours);
                Done::Closed { asked: ours.len(), changed }
            }
            Verb::Minimize => {
                let mut by_z: Vec<&Entry> = owned.take.iter().collect();
                by_z.sort_by_key(|e| e.z);
                let handles: Vec<isize> = by_z.iter().map(|e| e.hwnd).collect();
                Done::Minimized(minimize(&handles))
            }
        };
        *DONE.lock().unwrap_or_else(|e| e.into_inner()) = Some(done);
        // A full message queue refuses the post, so try again a few times.
        for _ in 0..20 {
            if unsafe { PostMessageW(Some(hwnd_of(job.owner)), WM_APP_CLOSED, WPARAM(0), LPARAM(0)) }.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    spawned.is_ok()
}

pub fn take_done() -> Option<Done> {
    DONE.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The windows matched to the workspace's rows, one per row, sorted into the ones taken and
/// the ones kept (D701, D702). A second window of the same app that no row describes isn't here.
pub fn windows_of(ws: &Workspace, exclude: &[String], remember: bool) -> Owned {
    let seen = snapshot_with(exclude, &mut Procs::new(), &mut |_, _| {});
    let known: Vec<Candidate> = seen.iter().map(|s| s.cand.clone()).collect();
    let matched = match_rows(ws, &known, remember);
    let facts: Vec<Fact> = seen.iter().map(|s| (s.cand.hwnd, s.pin, s.elevated)).collect();
    let (take, kept) = workspace::sort_out(&ws.windows, &matched, &facts);
    let entry = |row: usize, hwnd: isize| {
        let cand = known.iter().find(|c| c.hwnd == hwnd);
        Entry {
            hwnd,
            row,
            app: app_name(&ws.windows[row]),
            title: cand.map(|c| c.title.clone()).unwrap_or_default(),
            z: cand.map_or(usize::MAX, |c| c.z),
        }
    };
    Owned {
        take: take.into_iter().map(|(row, h)| entry(row, h)).collect(),
        kept: kept.into_iter().map(|(row, h, why)| (entry(row, h), why)).collect(),
    }
}

/// Ask each window to close, the way clicking its X does. The app decides, so one with
/// unsaved work asks first. Doesn't wait, so a hung window can't stall it.
pub fn close(handles: &[isize]) {
    for &key in handles {
        let h = hwnd_of(key);
        unsafe {
            if !IsWindow(Some(h)).as_bool() {
                continue;
            }
            // Seam closing a pinned window isn't the news D119 tells about.
            if winutil::pin_bits(h) != 0 {
                crate::wm::closing(key);
            }
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Minimize the windows that aren't minimized, and return those, in the order given, which
/// is topmost first, for `restore`.
pub fn minimize(handles: &[isize]) -> Vec<isize> {
    let open: Vec<isize> = handles
        .iter()
        .copied()
        .filter(|k| unsafe { IsWindow(Some(hwnd_of(*k))) }.as_bool() && !winutil::is_minimized(hwnd_of(*k)))
        .collect();
    for &key in &open {
        unsafe {
            let _ = ShowWindowAsync(hwnd_of(key), SW_MINIMIZE);
        }
    }
    open
}

/// Undo `minimize`: restore the windows still minimized, bottom of the stack first so the
/// order comes back. A window that is gone, or that someone restored by hand, is skipped.
pub fn restore(handles: &[isize]) {
    for &key in handles.iter().rev() {
        let h = hwnd_of(key);
        unsafe {
            if IsWindow(Some(h)).as_bool() && winutil::is_minimized(h) {
                let _ = ShowWindowAsync(h, SW_RESTORE);
            }
        }
    }
}
