//! My own wallpaper, as Seam found it before its first change (D797), and when it goes back (D798).
//! The record is a journal: written before Seam sets a picture, deleted once mine is back. Pure.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
/// `applied` is what Seam set, `now` what Windows says, both by Windows' id.
pub fn outside_change(applied: &BTreeMap<String, String>, now: &BTreeMap<String, String>) -> bool {
    applied.iter().any(|(id, file)| now.get(id).is_none_or(|shown| !shown.eq_ignore_ascii_case(file)))
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
        let applied = set(&[("m1", r"C:\cache\a.png")]);
        // Seam's own change comes back as the same file, in any case: nothing to record.
        assert!(!outside_change(&applied, &set(&[("m1", r"c:\CACHE\a.png"), ("m2", r"D:\mine.jpg")])));
        // Another picture on a screen Seam set, or none: someone else changed it.
        assert!(outside_change(&applied, &set(&[("m1", r"D:\theirs.jpg")])));
        assert!(outside_change(&applied, &set(&[])));
        // A screen Seam never set is never compared.
        assert!(!outside_change(&set(&[]), &set(&[("m2", r"D:\x.jpg")])));
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
    }
}
