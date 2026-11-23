//! The `seam-wallpaper` worker (D795): one thread with its own COM apartment, the WIC factory and
//! `IDesktopWallpaper`. The tray never decodes, encodes or calls into Explorer. Jobs come on a
//! channel and the latest Apply wins; each job that has news for the tray posts `WM_APP_WALLPAPER`.

use super::desktop::{Desktop, match_ids};
use super::journal::{self, AtStart, Record};
use super::render::{Failure, Wic};
use super::span::{self, Place};
use super::{
    CACHE_CAP, Cached, Failures, Fit, Origin, Show, UNREADABLE, Wallpaper, cache_key, cache_prefix, evict, retry,
    to_apply,
};
use crate::geom::Rect;
use crate::log;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Imaging::IWICBitmapSource;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// The worker has a report for the tray (`take_reports`).
pub const WM_APP_WALLPAPER: u32 = WM_APP + 48;

/// A connected screen, as the worker needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// Monitor device path.
    pub id: String,
    pub name: String,
    /// Line Up number, from 1.
    pub number: usize,
    /// Physical pixels, the size its picture is rendered at.
    pub rect: Rect,
}

/// What a screen can show, best first (`wallpaper::choices`). Past the last, my own.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub screen: Screen,
    pub choices: Vec<(Wallpaper, Origin)>,
    /// Where the screen stands in the picture across all screens, `None` when it is left out (D804).
    pub place: Option<Place>,
}

pub enum Job {
    /// Show these pictures, changing only the screens that differ (D794). With `true`, a switch, a
    /// screen change or a choice for a screen asked for it: it ends a hold, so it replaces a picture
    /// someone else set (D799). Without, it leaves such a picture where it is.
    Apply(Vec<Want>, bool),
    /// Render a picture into the cache now, so the first switch finds it there (D795): the whole
    /// picture, or with a place the part of a span that stands there (D809).
    Render(Screen, Wallpaper, Option<Place>),
    /// Windows says the wallpaper changed: was it someone else (D799)?
    Check,
    /// Put mine back (D798), and say whether it went back.
    Restore(Option<Sender<bool>>),
    /// Explorer restarted: set every screen again at the next Apply.
    Forget,
    /// Try the pictures that failed again: on one screen, after a choice for it, or with `None`
    /// on every screen, after a screen change.
    Retry(Option<String>),
}

/// What a screen shows after an Apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub screen: String,
    /// The picture's own file, "" for my own wallpaper.
    pub path: String,
    pub fit: Fit,
    pub from: Origin,
    /// A part of the picture across all screens, cut to where the screen stands.
    pub part: bool,
    /// A picture it was to show is gone: its last render is shown, or the next choice.
    pub missing: bool,
}

/// A picture that couldn't be shown, with one of the strings the window shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Failed {
    pub screen: String,
    pub number: usize,
    pub path: String,
    pub message: String,
    /// While setting what screens show, not when a picture was chosen.
    pub at_apply: bool,
}

/// Mine, as far as the window needs to know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Original {
    pub spotlight: bool,
    pub slideshow: bool,
}

/// What a job did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// After an Apply: every screen it was given.
    pub shown: Option<Vec<Shown>>,
    pub failed: Vec<Failed>,
    pub original: Original,
    /// Mine went back.
    pub restored: bool,
    /// Someone else changed the wallpaper, and that is mine now.
    pub outside: bool,
    /// An outside change was noticed and Seam leaves it: the app holds until a switch, screen
    /// change or choice.
    pub hold: bool,
}

static REPORTS: Mutex<Vec<Report>> = Mutex::new(Vec::new());

/// A picture's size once upright, with the file's length and write time it was read at.
type Sized = ((u64, u64), (u32, u32));

/// The reports since the last call.
pub fn take_reports() -> Vec<Report> {
    std::mem::take(&mut *REPORTS.lock().unwrap_or_else(|e| e.into_inner()))
}

pub struct Worker {
    queue: Sender<Job>,
}

impl Worker {
    /// Start the worker for the tray window `owner`. First it looks at the journal an earlier run
    /// left: with the feature off it puts mine back (D798).
    pub fn start(owner: isize, enabled: bool) -> Option<Worker> {
        let (queue, jobs) = channel::<Job>();
        let spawned =
            std::thread::Builder::new().name("seam-wallpaper".into()).spawn(move || run(owner, enabled, jobs));
        spawned.ok().map(|_| Worker { queue })
    }

    pub fn send(&self, job: Job) {
        let _ = self.queue.send(job);
    }
}

/// COM on the worker, for as long as it runs.
struct Com;

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn run(owner: isize, enabled: bool, jobs: Receiver<Job>) {
    let _com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok().then_some(Com);
    // Without WIC no picture can be rendered, but mine can still go back.
    let wic = Wic::open().ok();
    if wic.is_none() {
        log::info!("wallpaper: WIC isn't there, so no picture is shown");
    }
    let dir = crate::package::local_dir().join("wallpapers");
    let mut state = State::new(wic, dir);
    post(owner, state.at_start(enabled));
    while let Ok(first) = jobs.recv() {
        let batch: Vec<Job> = std::iter::once(first).chain(jobs.try_iter()).collect();
        for job in coalesce(batch) {
            if let Some(report) = state.run(job) {
                post(owner, report);
            }
        }
        // The picture the parts were cut from isn't kept past the jobs that wanted it (D806).
        state.kept = None;
    }
}

/// A newer Apply replaces the older ones in a batch, but if any of them ended a hold, the one
/// that runs does too, so a switch is never swallowed by a plain sync behind it.
fn coalesce(batch: Vec<Job>) -> Vec<Job> {
    let last = batch.iter().rposition(|j| matches!(j, Job::Apply(..)));
    let ends_hold = batch.iter().any(|j| matches!(j, Job::Apply(_, true)));
    let kept = batch.into_iter().enumerate().filter_map(|(i, job)| match job {
        Job::Apply(wants, _) if Some(i) == last => Some(Job::Apply(wants, ends_hold)),
        Job::Apply(..) => None,
        other => Some(other),
    });
    kept.collect()
}

fn post(owner: isize, report: Report) {
    REPORTS.lock().unwrap_or_else(|e| e.into_inner()).push(report);
    unsafe {
        let _ = PostMessageW(Some(HWND(owner as *mut _)), WM_APP_WALLPAPER, WPARAM(0), LPARAM(0));
    }
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

struct State {
    wic: Option<Wic>,
    /// `wallpapers\`: `original.json`, `original\` and `cache\`.
    dir: PathBuf,
    /// The file Seam last set on each monitor, by Windows' id. A monitor not here shows mine.
    applied: BTreeMap<String, String>,
    /// What Windows said each of those monitors showed right after Seam set it (D799).
    read_back: BTreeMap<String, String>,
    /// Renders that failed, by cache key, so a broken picture isn't opened again until it changes.
    failures: Failures,
    /// The journal (D797), while there is one.
    record: Option<Record>,
    /// A picture's size once upright, by path, with the file's length and write time it was read at (D806).
    sizes: BTreeMap<String, Sized>,
    /// The picture the parts of a span are cut from, kept decoded while an Apply renders them
    /// (D806): its path, length and write time. Only up to 64 megapixels; a bigger one is read for each part.
    kept: Option<((String, u64, u64), IWICBitmapSource)>,
}

impl State {
    fn new(wic: Option<Wic>, dir: PathBuf) -> State {
        let path = dir.join("original.json");
        let journal = std::fs::read(&path).ok();
        let record = journal.as_ref().and_then(|bytes| serde_json::from_slice(bytes).ok());
        // A journal that doesn't read is kept aside, never written over: it may be all there is of mine.
        if journal.is_some() && record.is_none() {
            log::info!("wallpaper: original.json didn't read, so it is kept as original.json.corrupt");
            let _ = std::fs::rename(&path, path.with_extension("json.corrupt"));
        }
        let (applied, read_back, failures) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        State { wic, dir, applied, read_back, failures, record, sizes: BTreeMap::new(), kept: None }
    }

    fn cache(&self) -> PathBuf {
        self.dir.join("cache")
    }

    fn in_cache(&self, path: &str) -> bool {
        journal::in_dir(path, &self.cache())
    }

    /// Seam set `id` to `sent`, and Windows says it shows `shown`.
    fn note_set(&mut self, id: &str, sent: String, shown: String) {
        let shown = if shown.is_empty() { sent.clone() } else { shown };
        self.applied.insert(id.to_string(), sent);
        self.read_back.insert(id.to_string(), shown);
    }

    fn forget_all(&mut self) {
        self.applied.clear();
        self.read_back.clear();
    }

    fn run(&mut self, job: Job) -> Option<Report> {
        match job {
            Job::Apply(wants, ends_hold) => Some(self.apply(&wants, ends_hold)),
            Job::Render(screen, picture, place) => self.render_only(&screen, &picture, place.as_ref()),
            Job::Check => self.check(),
            Job::Restore(reply) => {
                let report = self.restore(false);
                if let Some(reply) = reply {
                    let _ = reply.send(report.restored || self.record.is_none());
                }
                Some(report)
            }
            Job::Forget => {
                self.forget_all();
                None
            }
            Job::Retry(screen) => {
                retry(&mut self.failures, screen.as_deref());
                None
            }
        }
    }

    /// What a start does with the journal it found (D798).
    fn at_start(&mut self, enabled: bool) -> Report {
        let at_start = journal::at_start(self.record.is_some(), enabled);
        // Whatever of Seam's an earlier run left up is Seam's to change, or to take down.
        if at_start != AtStart::Nothing
            && let Ok(desk) = Desktop::open()
        {
            for (id, _) in desk.monitors() {
                let shown = desk.wallpaper(&id);
                if self.in_cache(&shown) {
                    self.note_set(&id, shown.clone(), shown);
                }
            }
        }
        let report = match at_start {
            AtStart::Restore => {
                log::info!("wallpaper: putting your own back, left from an earlier run");
                self.restore(true)
            }
            AtStart::Keep => Report { original: self.original(None), ..Report::default() },
            AtStart::Nothing => Report { original: self.original(Desktop::open().ok().as_ref()), ..Report::default() },
        };
        self.evict();
        report
    }

    /// Spotlight and slideshow, from the record, else from what Windows shows now.
    fn original(&self, desk: Option<&Desktop>) -> Original {
        let of = |r: &Record| Original { spotlight: r.spotlight(), slideshow: r.slideshow_on() };
        match (&self.record, desk) {
            (Some(record), _) => of(record),
            (None, Some(desk)) => of(&desk.record()),
            (None, None) => Original::default(),
        }
    }

    fn apply(&mut self, wants: &[Want], ends_hold: bool) -> Report {
        let mut report = Report::default();
        let desk = match Desktop::open() {
            Ok(desk) => desk,
            Err(e) => {
                log::info!("wallpaper: Windows' wallpaper isn't there ({:#x})", e.code().0);
                return report;
            }
        };
        // A change that sent Seam no message (Explorer's "Set as desktop background") is seen now (D799).
        report.outside = self.notice_outside(&desk);
        if !journal::apply_goes_on(report.outside, ends_hold) {
            report.hold = true;
            report.original = self.original(Some(&desk));
            return report;
        }
        let screens: Vec<(&str, Rect)> = wants.iter().map(|w| (w.screen.id.as_str(), w.screen.rect)).collect();
        let ids: BTreeMap<String, String> = match_ids(&screens, &desk.monitors()).into_iter().collect();
        let bars = self.record.as_ref().map_or_else(|| desk.background(), |r| r.background);
        let mut shows: Vec<(String, Show)> = Vec::new();
        let mut shown = Vec::new();
        for want in wants {
            let Some(id) = ids.get(&want.screen.id) else {
                log::info!("wallpaper: no Windows id for «{}»", want.screen.name);
                continue;
            };
            let (show, seen) = self.pick(want, bars, &mut report.failed);
            shows.push((id.clone(), show));
            shown.push(seen);
        }
        let changes: Vec<(String, Show)> = to_apply(&self.applied, &shows).into_iter().cloned().collect();
        if !changes.is_empty() && self.set(&desk, &changes, wants, &ids, &mut report.failed).is_err() {
            report.failed.extend(wants.iter().map(|w| Failed {
                screen: w.screen.id.clone(),
                number: w.screen.number,
                path: String::new(),
                message: String::new(),
                at_apply: true,
            }));
        }
        report.shown = Some(shown);
        report.original = self.original(Some(&desk));
        self.touch_and_evict();
        report
    }

    /// Record mine if there is no journal yet, then set each changed screen. An error is one that
    /// stops them all: the record couldn't be written, so nothing is changed.
    fn set(
        &mut self,
        desk: &Desktop,
        changes: &[(String, Show)],
        wants: &[Want],
        ids: &BTreeMap<String, String>,
        failed: &mut Vec<Failed>,
    ) -> Result<(), ()> {
        if journal::records(self.record.is_some()) {
            let record = self.record_now(desk);
            if let Err(e) = self.write_journal(&record) {
                log::info!("wallpaper: couldn't write original.json, so nothing changes: {e}");
                return Err(());
            }
            self.record = Some(record);
        }
        if let Err(e) = desk.fill() {
            log::info!("wallpaper: Fill wasn't taken ({:#x})", e.code().0);
        }
        let record = self.record.clone().unwrap_or_default();
        let exists = |p: &str| Path::new(p).exists();
        for (id, show) in changes {
            let path = match show {
                Show::Picture(file) => file.clone(),
                Show::Own => record.own_or_fallback(id, &exists),
            };
            match desk.set(id, &path) {
                // What Windows reads back is what a later check compares with (D799).
                Ok(()) => match show {
                    Show::Picture(file) => self.note_set(id, file.clone(), desk.wallpaper(id)),
                    Show::Own => {
                        self.applied.remove(id);
                        self.read_back.remove(id);
                    }
                },
                Err(e) => {
                    let screen = ids.iter().find(|(_, w)| *w == id).map(|(s, _)| s.clone()).unwrap_or_default();
                    let number = wants.iter().find(|w| w.screen.id == screen).map_or(0, |w| w.screen.number);
                    log::info!("wallpaper: screen {number} wasn't changed ({:#x})", e.code().0);
                    failed.push(Failed { screen, number, path, message: String::new(), at_apply: true });
                }
            }
        }
        Ok(())
    }

    /// The first of a screen's choices that can be shown: its render, made now if it isn't in the
    /// cache, or for a picture that is gone, its last render (D800). None of them: mine.
    fn pick(&mut self, want: &Want, bars: u32, failed: &mut Vec<Failed>) -> (Show, Shown) {
        let mut missing = false;
        for (picture, from) in &want.choices {
            // A part of the span is cut to where the screen stands; one left out shows the whole picture
            // with Fill (D804).
            let (drawn, place) = match (from, &want.place) {
                (Origin::Span, Some(place)) => (picture.clone(), Some(place)),
                (Origin::Span, None) => (Wallpaper { fit: Fit::Fill, ..picture.clone() }, None),
                _ => (picture.clone(), None),
            };
            let file = match self.render(&want.screen, &drawn, bars, place) {
                Ok(Some(file)) => file,
                Ok(None) => {
                    missing = true;
                    continue;
                }
                // Said once, not at every Apply, until the file changes.
                Err((message, fresh)) => {
                    if fresh {
                        failed.push(self.failure(&want.screen, &drawn, message, true));
                    }
                    continue;
                }
            };
            let seen = Shown {
                screen: want.screen.id.clone(),
                path: picture.path.clone(),
                fit: drawn.fit,
                from: *from,
                part: place.is_some(),
                missing,
            };
            return (Show::Picture(file), seen);
        }
        let own = Shown {
            screen: want.screen.id.clone(),
            path: String::new(),
            fit: Fit::Fill,
            from: Origin::Own,
            part: false,
            missing,
        };
        (Show::Own, own)
    }

    fn failure(&self, screen: &Screen, picture: &Wallpaper, message: &str, at_apply: bool) -> Failed {
        Failed {
            screen: screen.id.clone(),
            number: screen.number,
            path: picture.path.clone(),
            message: message.into(),
            at_apply,
        }
    }

    /// The render of `picture` for `screen`, made if it isn't in the cache: the whole picture, or
    /// with a `place` the part of it that stands there (D806). `None`: the picture is gone and was
    /// never rendered at this size. A failure says whether it is new: one already known for this
    /// very file isn't tried again.
    fn render(
        &mut self,
        screen: &Screen,
        picture: &Wallpaper,
        bars: u32,
        place: Option<&Place>,
    ) -> Result<Option<String>, (&'static str, bool)> {
        let size = (screen.rect.width().max(1) as u32, screen.rect.height().max(1) as u32);
        let cache = self.cache();
        let Ok(meta) = std::fs::metadata(&picture.path) else {
            // A part is found by its cut, which needs the size the picture had when it was last read.
            let known = self.sizes.get(&picture.path).map(|(_, src)| *src);
            let cut = place.zip(known).map(|(p, src)| span::cut(src, p, picture.fit));
            return Ok(match (place, cut) {
                (Some(_), None) => None,
                _ => self.last_render(&cache_prefix(&picture.path, picture.fit, bars, size, cut)),
            });
        };
        let written = meta.modified().map(unix).unwrap_or(0);
        let base = cache_key(&picture.path, meta.len(), written, picture.fit, bars, size, None);
        if let Some((_, message)) = self.failures.get(&base) {
            return Err((message, false));
        }
        let failed = |this: &mut State, e: Failure| {
            log::info!("wallpaper: couldn't render a picture for screen {} ({:?})", screen.number, e.hresult);
            this.failures.insert(base.clone(), (screen.id.clone(), e.message));
            (e.message, true)
        };
        let cut = match place {
            None => None,
            Some(p) => match self.source_size(&picture.path, meta.len(), written) {
                Ok(src) => Some(span::cut(src, p, picture.fit)),
                Err(e) => return Err(failed(self, e)),
            },
        };
        let key = match cut {
            Some(_) => cache_key(&picture.path, meta.len(), written, picture.fit, bars, size, cut),
            None => base.clone(),
        };
        let file = cache.join(format!("{key}.png"));
        if !file.exists() {
            let _ = std::fs::create_dir_all(&cache);
            let made = match (cut, &self.wic) {
                (None, Some(wic)) => wic.render(&picture.path, size, picture.fit, bars, &file),
                (None, None) => Err(Failure { message: UNREADABLE, hresult: None }),
                (Some(cut), _) => self.render_part(&picture.path, (meta.len(), written), size, cut, bars, &file),
            };
            if let Err(e) = made {
                return Err(failed(self, e));
            }
        }
        Ok(Some(file.to_string_lossy().into_owned()))
    }

    /// A picture's size once upright, read from the file only when it changed (D806).
    fn source_size(&mut self, path: &str, len: u64, written: u64) -> Result<(u32, u32), Failure> {
        if let Some((_, size)) = self.sizes.get(path).filter(|(stamp, _)| *stamp == (len, written)) {
            return Ok(*size);
        }
        let wic = self.wic.as_ref().ok_or(Failure { message: UNREADABLE, hresult: None })?;
        let size = wic.size_of(path)?;
        self.sizes.insert(path.to_string(), ((len, written), size));
        Ok(size)
    }

    /// Render the part `cut` of a picture for a screen of `size`. The picture is decoded once for
    /// every part of an Apply, up to 64 megapixels; a bigger one is read again for each part (D806).
    fn render_part(
        &mut self,
        path: &str,
        (len, written): (u64, u64),
        size: (u32, u32),
        cut: span::Cut,
        bars: u32,
        out: &Path,
    ) -> Result<(), Failure> {
        let stamp = (path.to_string(), len, written);
        let wic = self.wic.as_ref().ok_or(Failure { message: UNREADABLE, hresult: None })?;
        let source = match self.kept.as_ref().filter(|(s, _)| *s == stamp) {
            Some((_, source)) => source.clone(),
            None => {
                let (source, kept) = wic.decode(path)?;
                self.kept = kept.then(|| (stamp, source.clone()));
                source
            }
        };
        let wic = self.wic.as_ref().ok_or(Failure { message: UNREADABLE, hresult: None })?;
        wic.render_part(&source, size, cut, bars, out)
    }

    /// The newest render whose name starts with `prefix`.
    fn last_render(&self, prefix: &str) -> Option<String> {
        let found = std::fs::read_dir(self.cache()).ok()?.flatten().filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(prefix) && name.ends_with(".png")
        });
        let newest = found.max_by_key(|e| e.metadata().and_then(|m| m.modified()).map(unix).unwrap_or(0))?;
        Some(newest.path().to_string_lossy().into_owned())
    }

    /// `wallpaper.set` for a connected screen: render now, show nothing (D795).
    fn render_only(&mut self, screen: &Screen, picture: &Wallpaper, place: Option<&Place>) -> Option<Report> {
        let desk = Desktop::open().ok();
        let bars = match (&self.record, &desk) {
            (Some(record), _) => record.background,
            (None, desk) => desk.as_ref().map_or(0, Desktop::background),
        };
        let failed = match self.render(screen, picture, bars, place) {
            Err((message, _)) => vec![self.failure(screen, picture, message, false)],
            Ok(_) => Vec::new(),
        };
        self.evict();
        (!failed.is_empty()).then(|| Report { failed, original: self.original(desk.as_ref()), ..Report::default() })
    }

    /// D799: a monitor Seam set shows something else now, so that is mine, recorded once, and
    /// Seam leaves it until the next switch or screen change.
    fn check(&mut self) -> Option<Report> {
        let desk = Desktop::open().ok()?;
        self.notice_outside(&desk).then(|| Report {
            outside: true,
            hold: true,
            original: self.original(Some(&desk)),
            ..Report::default()
        })
    }

    /// Whether someone else changed the wallpaper since Seam set it (D799): if so, what shows now
    /// is recorded as mine and every screen is forgotten, so the next Apply sets them all again.
    /// Run by `check`, and before an Apply or a Restore acts: not every way to set a wallpaper
    /// tells Seam.
    fn notice_outside(&mut self, desk: &Desktop) -> bool {
        if self.read_back.is_empty() {
            return false;
        }
        let monitors = desk.monitors();
        let attached = |id: &str| monitors.iter().any(|(m, rect)| m == id && rect.is_some());
        let now: BTreeMap<String, String> =
            self.read_back.keys().map(|id| (id.clone(), desk.wallpaper(id))).filter(|(_, s)| !s.is_empty()).collect();
        // A picture of Seam's own is never someone else's change.
        if !journal::outside_change(&self.read_back, &now, &attached, &|p| self.in_cache(p)) {
            return false;
        }
        log::info!("wallpaper: someone else changed it, so that is your own now");
        let record = self.record_now(desk);
        if self.write_journal(&record).is_ok() {
            self.record = Some(record);
        }
        self.forget_all();
        true
    }

    /// Put mine back (D798), monitor by monitor, on every monitor Seam set too. The journal goes
    /// once anything is back, so a monitor Windows no longer takes can't keep it alive. With
    /// `only_seams` (a start after a crash), a monitor that shows something else now keeps it.
    fn restore(&mut self, only_seams: bool) -> Report {
        if self.record.is_none() {
            return Report { original: self.original(Desktop::open().ok().as_ref()), ..Report::default() };
        }
        let desk = Desktop::open();
        let set_by_seam: Vec<String> = self.applied.keys().cloned().collect();
        // What someone else set since is mine now, and goes back (D799). Not after a crash: there
        // a monitor showing something else keeps it anyway.
        let outside = !only_seams && desk.as_ref().is_ok_and(|desk| self.notice_outside(desk));
        let record = self.record.clone().unwrap_or_default();
        let original = self.original(None);
        let exists = |p: &str| Path::new(p).exists();
        let plan = journal::restore_plan(&record, &set_by_seam, only_seams, &exists);
        let took = match (only_seams && plan.is_empty(), desk) {
            // Nothing of Seam's is up: what shows is mine, whoever chose it.
            (true, _) => 0,
            (false, Ok(desk)) => desk.restore(&record, &plan),
            (false, Err(e)) => {
                log::info!("wallpaper: your own didn't go back ({:#x}), the record stays", e.code().0);
                return Report { original, outside, ..Report::default() };
            }
        };
        if !journal::restored(plan.len(), took) {
            log::info!("wallpaper: no monitor took your own back, the record stays");
            return Report { original, outside, ..Report::default() };
        }
        let _ = std::fs::remove_file(self.dir.join("original.json"));
        self.record = None;
        self.forget_all();
        log::info!("wallpaper: your own is back ({took} of {} monitors)", plan.len());
        Report { restored: true, original, outside, ..Report::default() }
    }

    /// What Windows shows now, with a copy of each picture in `original\`, so a moved or deleted
    /// one can still come back (D797). A monitor showing one of Seam's renders keeps what the
    /// last record had for it.
    fn record_now(&self, desk: &Desktop) -> Record {
        let mut record = journal::rerecord(desk.record(), self.record.as_ref(), &|p| self.in_cache(p));
        let copies = self.dir.join("original");
        let _ = std::fs::create_dir_all(&copies);
        for (i, picture) in record.monitors.iter_mut().enumerate() {
            let source = Path::new(&picture.path);
            if picture.path.is_empty() || picture.copy.is_some() || source.starts_with(&copies) {
                continue;
            }
            let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let copy = copies.join(format!("{i}-{name}"));
            if std::fs::copy(source, &copy).is_ok() {
                picture.copy = Some(copy.to_string_lossy().into_owned());
            }
        }
        // Copies the new record doesn't name are an older record's.
        let named = |p: &Path| record.monitors.iter().any(|m| m.copy.as_deref().is_some_and(|c| Path::new(c) == p));
        let named = |p: &Path| named(p) || record.monitors.iter().any(|m| Path::new(&m.path) == p);
        for entry in std::fs::read_dir(&copies).into_iter().flatten().flatten() {
            if !named(&entry.path()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        record
    }

    /// The journal, written and flushed before any picture is set.
    fn write_journal(&self, record: &Record) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("original.json");
        let tmp = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(serde_json::to_string_pretty(record).map_err(std::io::Error::other)?.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &path)
    }

    /// The renders on screen now count as used now, then the cache is trimmed.
    fn touch_and_evict(&self) {
        for file in self.applied.values() {
            let opened = std::fs::File::options().write(true).open(file);
            if let Ok(file) = opened {
                let _ = file.set_modified(SystemTime::now());
            }
        }
        self.evict();
    }

    /// Keep the cache under its cap, oldest first, never a render on screen or the record's (D800).
    fn evict(&self) {
        let Ok(entries) = std::fs::read_dir(self.cache()) else { return };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // A render that never finished.
            if path.extension().is_some_and(|e| e == "tmp") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            files.push(Cached { path, len: meta.len(), used: meta.modified().map(unix).unwrap_or(0) });
        }
        let on_screen: Vec<&str> = self.applied.values().map(String::as_str).collect();
        let kept = |p: &Path| journal::kept(p, &on_screen, self.record.as_ref());
        for path in evict(&files, CACHE_CAP, &kept) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(ends_hold: bool) -> Job {
        Job::Apply(Vec::new(), ends_hold)
    }

    #[test]
    fn the_last_apply_runs_and_ends_a_hold_if_any_did() {
        let kept = coalesce(vec![apply(true), apply(false)]);
        assert_eq!(kept.len(), 1);
        assert!(matches!(kept[0], Job::Apply(_, true)));
        let kept = coalesce(vec![apply(false), apply(false)]);
        assert!(matches!(kept[..], [Job::Apply(_, false)]));
    }

    #[test]
    fn other_jobs_keep_their_order_around_the_apply() {
        let kept = coalesce(vec![Job::Check, apply(true), Job::Forget, apply(false), Job::Retry(None)]);
        assert!(matches!(kept[..], [Job::Check, Job::Forget, Job::Apply(_, true), Job::Retry(None)]));
    }

    #[test]
    fn a_single_apply_is_unchanged() {
        assert!(matches!(coalesce(vec![apply(false)])[..], [Job::Apply(_, false)]));
        assert!(matches!(coalesce(vec![Job::Check, apply(true)])[..], [Job::Check, Job::Apply(_, true)]));
    }
}
