//! My own wallpaper, as Seam found it before its first change (D797), and when it goes back (D798).
//! The record is a journal: written before Seam sets a picture, deleted once mine is back. Pure.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// `BackgroundType` under `HKCU\...\Explorer\Wallpapers` for Windows Spotlight. Not documented.
pub const SPOTLIGHT: u32 = 3;

/// Everything Windows showed, to put it back as it was.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    /// Each monitor's picture, by Windows' id for it.
    pub monitors: Vec<Picture>,
    /// `DESKTOP_WALLPAPER_POSITION`.
    pub position: i32,
    /// `COLORREF`: 0x00BBGGRR.
    pub background: u32,
    /// `DESKTOP_SLIDESHOW_STATE` flags.
    pub status: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slideshow: Option<Slideshow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_type: Option<u32>,
}

/// One monitor's picture: the path Windows had, empty for a solid colour, and Seam's copy of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Picture {
    pub id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<String>,
}

/// A slideshow's pictures and options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Slideshow {
    pub items: Vec<String>,
    /// `DESKTOP_SLIDESHOW_OPTIONS`.
    pub options: i32,
    /// Milliseconds between pictures.
    pub tick: u32,
}

impl Record {
    pub fn spotlight(&self) -> bool {
        self.background_type == Some(SPOTLIGHT)
    }

    pub fn slideshow_on(&self) -> bool {
        self.slideshow.is_some()
    }

    /// What goes back on a monitor: the picture where it was, else Seam's copy of it, or "" for a
    /// solid colour. `exists` says whether a file is still there.
    pub fn own(&self, id: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
        let picture = self.monitors.iter().find(|p| p.id.eq_ignore_ascii_case(id))?;
        let back = match &picture.copy {
            Some(copy) if !picture.path.is_empty() && !exists(&picture.path) => copy.clone(),
            _ => picture.path.clone(),
        };
        Some(back)
    }

    /// What goes back on a monitor the record has no entry for, one first seen after it was
    /// written: the first recorded monitor's picture, else "" (a solid colour).
    pub fn fallback(&self, exists: &dyn Fn(&str) -> bool) -> String {
        let first = self.monitors.first().and_then(|p| self.own(&p.id, exists));
        first.unwrap_or_default()
    }

    /// `own`, or for a monitor the record lacks, `fallback`.
    pub fn own_or_fallback(&self, id: &str, exists: &dyn Fn(&str) -> bool) -> String {
        self.own(id, exists).unwrap_or_else(|| self.fallback(exists))
    }

    /// Whether the record names this file, as a monitor's picture or Seam's copy of one.
    pub fn names(&self, path: &Path) -> bool {
        let named = |s: &str| !s.is_empty() && Path::new(s) == path;
        self.monitors.iter().any(|m| named(&m.path) || m.copy.as_deref().is_some_and(named))
    }
}

/// Whether `path` is a file directly in `dir`, in any case: one of Seam's renders when `dir` is the cache.
pub fn in_dir(path: &str, dir: &Path) -> bool {
    Path::new(path).parent().is_some_and(|p| p.as_os_str().eq_ignore_ascii_case(dir.as_os_str()))
}

/// What a restore sets, monitor by monitor (D798): every recorded monitor's own, then every monitor
/// Seam set that the record lacks, with the fallback. `set_by_seam` are the monitors showing one of
/// Seam's pictures. With `only_seams` (a start after a crash), only those: a monitor showing
/// something else was changed since, and keeps it.
pub fn restore_plan(
    record: &Record,
    set_by_seam: &[String],
    only_seams: bool,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let ours = |id: &str| set_by_seam.iter().any(|s| s.eq_ignore_ascii_case(id));
    let recorded = record.monitors.iter().map(|p| p.id.clone());
    let unrecorded = set_by_seam.iter().filter(|id| !record.monitors.iter().any(|p| p.id.eq_ignore_ascii_case(id)));
    let ids = recorded.chain(unrecorded.cloned()).filter(|id| !only_seams || ours(id));
    ids.map(|id| {
        let back = record.own_or_fallback(&id, exists);
        (id, back)
    })
    .collect()
}

/// Whether a restore went far enough for the journal to go: nothing was to be set, or at least one
/// monitor took its picture. A monitor Windows no longer takes doesn't keep it alive.
pub fn restored(planned: usize, took: usize) -> bool {
    planned == 0 || took > 0
}

/// What Windows shows now, kept true to mine (D797, D799): a monitor showing one of Seam's pictures
/// (`ours`) keeps the old record's entry, else the fallback, and with an old record, its position
/// and colour stay, since Seam set Fill over them.
pub fn rerecord(mut now: Record, old: Option<&Record>, ours: &dyn Fn(&str) -> bool) -> Record {
    let exists = |_: &str| true;
    for picture in now.monitors.iter_mut().filter(|p| ours(&p.path)) {
        let before = old.and_then(|r| r.monitors.iter().find(|p| p.id.eq_ignore_ascii_case(&picture.id)));
        *picture = match (before, old) {
            (Some(before), _) => before.clone(),
            (None, Some(old)) => Picture { id: picture.id.clone(), path: old.fallback(&exists), copy: None },
            (None, None) => Picture { id: picture.id.clone(), ..Picture::default() },
        };
    }
    if let Some(old) = old {
        now.position = old.position;
        now.background = old.background;
    }
    now
}

/// What a start does with a journal an earlier run left (D798).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtStart {
    /// No journal: Windows shows mine.
    Nothing,
    /// The feature is off now: an earlier run crashed, or an older Seam turned it off. Mine goes back.
    Restore,
    /// The feature is on: the journal stays mine, and Seam puts its pictures up over what is there.
    Keep,
}

pub fn at_start(journal: bool, enabled: bool) -> AtStart {
    match (journal, enabled) {
        (false, _) => AtStart::Nothing,
        (true, false) => AtStart::Restore,
        (true, true) => AtStart::Keep,
    }
}

/// Whether Seam records mine before it sets a picture: only while there is no journal, so mine is
/// never replaced by one of Seam's own pictures.
pub fn records(journal: bool) -> bool {
    !journal
}

/// Whether someone else changed the wallpaper (D799): a monitor Seam set shows another picture now.
/// `read_back` is what Windows said each monitor showed right after Seam set it, `now` what it says
/// now, both by Windows' id. A detached monitor, one Windows gives no answer for and one showing
/// one of Seam's pictures (`ours`) count as unchanged.
pub fn outside_change(
    read_back: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
    attached: &dyn Fn(&str) -> bool,
    ours: &dyn Fn(&str) -> bool,
) -> bool {
    let differs = |(id, was): (&String, &String)| {
        let Some(shown) = now.get(id).filter(|_| attached(id)) else { return false };
        !ours(shown) && !shown.eq_ignore_ascii_case(was)
    };
    read_back.iter().any(differs)
}

/// Whether a cache file stays (D800): one on screen now, or one the record names.
pub fn kept(path: &Path, on_screen: &[&str], record: Option<&Record>) -> bool {
    on_screen.iter().any(|f| Path::new(f) == path) || record.is_some_and(|r| r.names(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_once_while_journal_exists() {
        assert!(records(false));
        // Once written, the next change finds the journal and leaves mine as recorded.
        assert!(!records(true));
    }

    #[test]
    fn start_with_journal_and_feature_off_restores() {
        assert_eq!(at_start(true, false), AtStart::Restore);
        assert_eq!(at_start(false, false), AtStart::Nothing);
    }

    #[test]
    fn start_with_journal_and_feature_on_keeps_it() {
        assert_eq!(at_start(true, true), AtStart::Keep);
        assert_eq!(at_start(false, true), AtStart::Nothing);
    }

    #[test]
    fn outside_change_rerecords() {
        let set = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        };
        let cache = Path::new(r"C:\cache");
        let ours = |p: &str| in_dir(p, cache);
        let all = |_: &str| true;
        // Windows read back something else than the file Seam sent: that is what counts.
        let read_back = set(&[("m1", r"C:\Themes\Transcoded_000")]);
        let check = |now: &BTreeMap<String, String>, attached: &dyn Fn(&str) -> bool| {
            outside_change(&read_back, now, attached, &ours)
        };
        // The same again, in any case, or one of Seam's renders: nothing to record.
        assert!(!check(&set(&[("m1", r"c:\themes\TRANSCODED_000"), ("m2", r"D:\mine.jpg")]), &all));
        assert!(!check(&set(&[("m1", r"C:\cache\b.png")]), &all));
        // Another picture on a screen Seam set: someone else changed it.
        assert!(check(&set(&[("m1", r"D:\theirs.jpg")]), &all));
        // No answer, or a detached monitor, isn't a change.
        assert!(!check(&set(&[]), &all));
        assert!(!check(&set(&[("m1", "")]), &|id: &str| id != "m1"));
        // A screen Seam never set is never compared.
        assert!(!outside_change(&set(&[]), &set(&[("m2", r"D:\x.jpg")]), &all, &ours));
    }

    #[test]
    fn own_picture_is_the_original_while_it_is_there_else_the_copy() {
        let record = Record {
            monitors: vec![
                Picture { id: "M1".into(), path: r"D:\a.jpg".into(), copy: Some(r"C:\orig\a.jpg".into()) },
                Picture { id: "m2".into(), path: String::new(), copy: None },
            ],
            ..Record::default()
        };
        assert_eq!(record.own("m1", &|_| true).as_deref(), Some(r"D:\a.jpg"));
        assert_eq!(record.own("m1", &|_| false).as_deref(), Some(r"C:\orig\a.jpg"));
        // A solid colour goes back as one.
        assert_eq!(record.own("m2", &|_| false).as_deref(), Some(""));
        assert_eq!(record.own("m3", &|_| true), None);
        // A monitor first seen later gets the first recorded picture.
        assert_eq!(record.own_or_fallback("m3", &|_| true), r"D:\a.jpg");
        assert_eq!(Record::default().own_or_fallback("m3", &|_| true), "");
    }

    fn two_screens() -> Record {
        Record {
            monitors: vec![
                Picture { id: "m1".into(), path: r"D:\a.jpg".into(), copy: None },
                Picture { id: "m2".into(), path: r"D:\b.jpg".into(), copy: None },
            ],
            position: 2,
            background: 0x00ff_0000,
            ..Record::default()
        }
    }

    fn plan_words(plan: Vec<(String, String)>) -> Vec<String> {
        plan.into_iter().map(|(id, path)| format!("{id}={path}")).collect()
    }

    #[test]
    fn restore_visits_every_monitor_seam_set_and_only_those_after_a_crash() {
        let record = two_screens();
        let set_by_seam = ["m2".to_string(), "m3".to_string()];
        // Turning off or quitting: every recorded monitor, then m3, which came later.
        let plan = restore_plan(&record, &set_by_seam, false, &|_| true);
        assert_eq!(plan_words(plan), [r"m1=D:\a.jpg", r"m2=D:\b.jpg", r"m3=D:\a.jpg"]);
        // A start after a crash leaves m1, which shows something of mine now.
        let plan = restore_plan(&record, &set_by_seam, true, &|_| true);
        assert_eq!(plan_words(plan), [r"m2=D:\b.jpg", r"m3=D:\a.jpg"]);
        assert!(restore_plan(&record, &[], true, &|_| true).is_empty());
    }

    #[test]
    fn a_stale_monitor_doesnt_keep_the_journal() {
        assert!(restored(2, 1));
        assert!(restored(0, 0));
        assert!(!restored(2, 0));
    }

    #[test]
    fn rerecord_keeps_mine_where_seams_picture_shows() {
        let cache = Path::new(r"C:\cache");
        let ours = |p: &str| in_dir(p, cache);
        let now = Record {
            monitors: vec![
                Picture { id: "m1".into(), path: r"C:\Cache\x.png".into(), copy: None },
                Picture { id: "m2".into(), path: r"E:\theirs.jpg".into(), copy: None },
                Picture { id: "m3".into(), path: r"C:\cache\y.png".into(), copy: None },
            ],
            position: 4,
            background: 0,
            ..Record::default()
        };
        let old = two_screens();
        let got = rerecord(now.clone(), Some(&old), &ours);
        let paths: Vec<&str> = got.monitors.iter().map(|p| p.path.as_str()).collect();
        // m1 keeps mine, m2 is the new one, m3 (not in the old record) gets the fallback, never a render.
        assert_eq!(paths, [r"D:\a.jpg", r"E:\theirs.jpg", r"D:\a.jpg"]);
        assert_eq!((got.position, got.background), (2, 0x00ff_0000));
        // The first record: a render a lost journal left up is recorded as a solid colour, not as mine.
        let first = rerecord(now, None, &ours);
        assert_eq!(first.monitors[0].path, "");
        assert_eq!(first.position, 4);
    }

    #[test]
    fn in_dir_is_the_cache_folder_itself_in_any_case() {
        let cache = Path::new(r"C:\Users\a\Seam\wallpapers\cache");
        assert!(in_dir(r"c:\users\A\seam\wallpapers\CACHE\k.png", cache));
        assert!(!in_dir(r"C:\Users\a\Seam\wallpapers\cache\sub\k.png", cache));
        assert!(!in_dir(r"C:\Users\a\Seam\wallpapers\original\k.png", cache));
        assert!(!in_dir("", cache));
    }

    #[test]
    fn eviction_keeps_what_is_on_screen_and_what_the_record_names() {
        let record = Record {
            monitors: vec![Picture {
                id: "m1".into(),
                path: r"C:\cache\r.png".into(),
                copy: Some(r"C:\o\c.jpg".into()),
            }],
            ..Record::default()
        };
        let on_screen = [r"C:\cache\s.png"];
        assert!(kept(Path::new(r"C:\cache\s.png"), &on_screen, None));
        assert!(kept(Path::new(r"C:\cache\r.png"), &on_screen, Some(&record)));
        assert!(kept(Path::new(r"C:\o\c.jpg"), &[], Some(&record)));
        assert!(!kept(Path::new(r"C:\cache\old.png"), &on_screen, Some(&record)));
        // A solid colour names no file.
        let solid = Record { monitors: vec![Picture { id: "m1".into(), ..Picture::default() }], ..Record::default() };
        assert!(!kept(Path::new(""), &[], Some(&solid)));
    }
}
