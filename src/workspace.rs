//! Workspaces: which apps go where on a desk. Pure logic, so the tray's capture
//! and switch and the panel's editor, export and import agree.
//!
//! Zone numbers here count from 1, as the panel shows them.

use crate::zones::{self, Rect};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{hash_map::RandomState, BTreeMap};
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, Prefix};

/// A window this close to a zone, in pixels, counts as being in it.
const ZONE_TOLERANCE: i32 = 8;

/// What a switch does with windows the workspace doesn't mention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtherWindows {
    #[default]
    Leave,
    Minimize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WinState {
    Max,
    Keep,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Zone {
        zone: u32,
        span_to: Option<u32>,
    },
    /// Fractions of the work area: x, y, width, height.
    Rect {
        rect: [f32; 4],
    },
    State {
        state: WinState,
    },
}

/// How a window is shown. A minimized one stays minimized when switched to,
/// and restores into its place later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shown {
    #[default]
    Normal,
    Minimized,
    Maximized,
    /// Borderless and covering the whole screen.
    Fullscreen,
}

/// How a window was pinned, for the last session (D120). No pin isn't stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pin {
    Top,
    Sticky,
    Both,
}

impl Pin {
    /// From `Seam_pin` bits: 1 is on top, 2 is sticky.
    pub fn of_bits(bits: usize) -> Option<Pin> {
        match bits & 3 {
            1 => Some(Pin::Top),
            2 => Some(Pin::Sticky),
            3 => Some(Pin::Both),
            _ => None,
        }
    }

    pub fn bits(self) -> usize {
        match self {
            Pin::Top => 1,
            Pin::Sticky => 2,
            Pin::Both => 3,
        }
    }
}

// Deserialize is written out below, so older files load into the new fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self", default)]
pub struct Rule {
    /// Full path, matched case-insensitively.
    pub exe: String,
    pub aumid: Option<String>,
    pub class: String,
    /// For display and tiebreaks only.
    pub title_hint: String,
    /// Device path of the screen.
    pub monitor: String,
    pub target: Target,
    /// `WINDOWPLACEMENT.rcNormalPosition` as fractions of the work area.
    pub normal_rect: Option<[f32; 4]>,
    pub launch: bool,
    /// Only the last session has pins. A switch pins the window it matches again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<Pin>,
    pub shown: Shown,
    /// Stacking at capture, 0 is the topmost. `u32::MAX` when it isn't known.
    pub z: u32,
    /// The window that had the focus at capture. At most one per workspace.
    pub focused: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            exe: String::new(),
            aumid: None,
            class: String::new(),
            title_hint: String::new(),
            monitor: String::new(),
            target: Target::State { state: WinState::Keep },
            normal_rect: None,
            launch: true,
            pin: None,
            shown: Shown::Normal,
            z: u32::MAX,
            focused: false,
        }
    }
}

impl Serialize for Rule {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Rule::serialize(self, s)
    }
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut rule = Rule::deserialize(d)?;
        // Before `shown`, a maximized window was only its target. Nothing is
        // written back until the next save.
        if rule.target == (Target::State { state: WinState::Max }) && rule.shown == Shown::Normal {
            rule.shown = Shown::Maximized;
        }
        Ok(rule)
    }
}

impl Rule {
    /// Send the window somewhere else. Full is maximized, and another place
    /// isn't, but a minimized window stays minimized.
    pub fn set_target(&mut self, target: Target) {
        match target {
            Target::State { state: WinState::Max } => self.shown = Shown::Maximized,
            Target::State { state: WinState::Keep } => {}
            _ if matches!(self.shown, Shown::Maximized | Shown::Fullscreen) => self.shown = Shown::Normal,
            _ => {}
        }
        self.target = target;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub hotkey: Option<String>,
    pub launch_missing: bool,
    pub other_windows: OtherWindows,
    pub windows: Vec<Rule>,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace {
            id: String::new(),
            name: String::new(),
            hotkey: None,
            launch_missing: true,
            other_windows: OtherWindows::Leave,
            windows: Vec::new(),
        }
    }
}

/// A saved workspace, or `None` for an entry that isn't one. The config keeps
/// them as raw JSON so one bad entry can't stop the file loading (D63).
pub fn parse(saved: &Value) -> Option<Workspace> {
    serde_json::from_value(saved.clone()).ok().filter(|w: &Workspace| !w.id.is_empty())
}

pub fn list(saved: &[Value]) -> Vec<Workspace> {
    saved.iter().filter_map(parse).collect()
}

pub fn to_value(workspace: &Workspace) -> Value {
    serde_json::to_value(workspace).unwrap_or_default()
}

/// The id the last session switches under. No saved workspace has it.
pub const SESSION_ID: &str = "last_session";

/// The hidden "Last session" workspace a desk keeps (D120): what was where, and
/// what was pinned, when Seam last looked.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// Unix seconds.
    pub saved_at: u64,
    pub windows: Vec<Rule>,
}

impl Session {
    /// A saved session with at least one window, or `None`.
    pub fn parse(saved: &Value) -> Option<Session> {
        serde_json::from_value(saved.clone()).ok().filter(|s: &Session| !s.windows.is_empty())
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// The workspace a restore switches to. Other windows are always left alone.
    pub fn workspace(self) -> Workspace {
        Workspace {
            id: SESSION_ID.into(),
            name: "Last session".into(),
            hotkey: None,
            launch_missing: true,
            other_windows: OtherWindows::Leave,
            windows: self.windows,
        }
    }
}

/// A random id that isn't in `taken`.
pub fn new_id(taken: &[String]) -> String {
    loop {
        let bits = RandomState::new().build_hasher().finish();
        let id = format!("ws_{:04x}", bits & 0xffff);
        if !taken.contains(&id) {
            return id;
        }
    }
}

/// "Workspace N", the first number not in use.
pub fn next_name(taken: &[String]) -> String {
    (taken.len() + 1..).map(|n| format!("Workspace {n}")).find(|name| !taken.contains(name)).unwrap_or_default()
}

/// `name`, with " 2" added until it is no longer in use.
pub fn unique_name(name: &str, taken: &[String]) -> String {
    let mut name = name.to_string();
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name.push_str(" 2");
    }
    name
}

/// A window on screen, as the matcher sees it.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub hwnd: isize,
    pub exe: String,
    pub aumid: Option<String>,
    pub class: String,
    pub title: String,
    /// 0 is the topmost.
    pub z: usize,
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// For each rule, in order, the window it gets, or `None` (launch it or skip it).
/// An AUMID beats an exe. Ties go to the same class, then the longest title
/// prefix, then z-order. A window is claimed once.
pub fn assign(rules: &[Rule], wins: &[Candidate]) -> Vec<Option<isize>> {
    let mut taken = vec![false; wins.len()];
    rules
        .iter()
        .map(|r| {
            let best = wins
                .iter()
                .enumerate()
                .filter(|(i, _)| !taken[*i])
                .filter(|(_, w)| match (&r.aumid, &w.aumid) {
                    (Some(a), Some(b)) => a == b,
                    (Some(_), None) => false,
                    (None, _) => r.exe.eq_ignore_ascii_case(&w.exe),
                })
                .max_by_key(|(_, w)| (w.class == r.class, common_prefix(&r.title_hint, &w.title), usize::MAX - w.z))
                .map(|(i, w)| (i, w.hwnd));
            best.map(|(i, h)| {
                taken[i] = true;
                h
            })
        })
        .collect()
}

/// `r` as fractions of `work`.
pub fn to_fractions(r: Rect, work: Rect) -> [f32; 4] {
    let (w, h) = (work.w.max(1) as f32, work.h.max(1) as f32);
    [(r.x - work.x) as f32 / w, (r.y - work.y) as f32 / h, r.w as f32 / w, r.h as f32 / h]
}

/// Fractions come from files, so a wild or NaN one is pulled back to something a screen can hold.
pub fn from_fractions(f: [f32; 4], work: Rect) -> Rect {
    let tame = |v: f32, lo: f32, hi: f32| if v.is_nan() { lo.max(0.) } else { v.clamp(lo, hi) };
    let (w, h) = (work.w as f32, work.h as f32);
    Rect {
        x: work.x.saturating_add((tame(f[0], -1., 2.) * w).round() as i32),
        y: work.y.saturating_add((tame(f[1], -1., 2.) * h).round() as i32),
        w: ((tame(f[2], 0.05, 2.) * w).round() as i32).max(1),
        h: ((tame(f[3], 0.05, 2.) * h).round() as i32).max(1),
    }
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x, b.x), (a.y, b.y), (a.w, b.w), (a.h, b.h)].iter().all(|(p, q)| (p - q).abs() <= ZONE_TOLERANCE)
}

/// The zone, or the span of two zones, that `visible` is within a few pixels of.
pub fn target_of(visible: Rect, zones: &[Rect]) -> Option<Target> {
    if let Some(i) = zones.iter().position(|z| near(visible, *z)) {
        return Some(Target::Zone { zone: i as u32 + 1, span_to: None });
    }
    (0..zones.len()).find_map(|a| {
        let b = (a + 1..zones.len()).find(|b| near(visible, zones::span(zones[a], zones[*b])))?;
        Some(Target::Zone { zone: a as u32 + 1, span_to: Some(b as u32 + 1) })
    })
}

/// Where a window with this target goes, in pixels. A zone that is gone means
/// the whole work area. `None` for max and keep, which aren't a place.
pub fn rect_for(target: &Target, zones: &[Rect], work: Rect) -> Option<Rect> {
    let zone = |n: u32| n.checked_sub(1).and_then(|i| zones.get(i as usize)).copied();
    match target {
        Target::Zone { zone: first, span_to } => Some(match (zone(*first), span_to.and_then(zone)) {
            (Some(a), Some(b)) => zones::span(a, b),
            (Some(a), None) => a,
            (None, _) => work,
        }),
        Target::Rect { rect } => Some(from_fractions(*rect, work)),
        Target::State { .. } => None,
    }
}

/// The old rule a new capture of the same app replaces, so its Launch box survives.
pub fn keep_launch_flags(old: &Workspace, mut fresh: Vec<Rule>) -> Vec<Rule> {
    let mut used = vec![false; old.windows.len()];
    for rule in &mut fresh {
        let same = |o: &Rule| match (&rule.aumid, &o.aumid) {
            (Some(a), Some(b)) => a == b,
            (None, None) => rule.exe.eq_ignore_ascii_case(&o.exe),
            _ => false,
        };
        if let Some(i) = old.windows.iter().enumerate().position(|(i, o)| !used[i] && same(o)) {
            used[i] = true;
            rule.launch = old.windows[i].launch;
        }
    }
    fresh
}

/// A screen as an export file describes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenInfo {
    /// Line Up number.
    pub number: u32,
    pub name: String,
    pub width: i32,
    pub height: i32,
}

/// What Export writes: the workspace and the screens its windows refer to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Export {
    pub seam_workspace: u32,
    pub workspace: Workspace,
    /// Device path -> screen.
    pub screens: BTreeMap<String, ScreenInfo>,
}

impl Export {
    pub const VERSION: u32 = 1;
}

/// A program Seam may start: an absolute path on a drive letter (`C:\...`) that ends
/// in `.exe`. UNC, verbatim, relative, drive-relative and bare names are all refused,
/// and so is a `/` or a `..` anywhere. Whether the drive is a network drive is
/// checked where the program is started.
pub fn safe_exe(exe: &str) -> bool {
    let path = Path::new(exe);
    let mut parts = path.components();
    let on_drive = matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)));
    on_drive
        && matches!(parts.next(), Some(Component::RootDir))
        && !exe.contains('/')
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && exe.to_lowercase().ends_with(".exe")
}

/// An AppUserModelID is letters, digits, `.`, `_` and `-`, with an optional `!` and a second such part.
pub fn safe_aumid(aumid: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    match aumid.split_once('!') {
        Some((a, b)) => part(a) && part(b),
        None => part(aumid),
    }
}

/// Bring an exported workspace onto this desk. A window's screen is found by
/// device path, then by number and model, then by number. A window with no
/// screen here stays where it is. The shortcut is dropped, so it can't clash. A file
/// from someone else is untrusted: nothing launches until the user ticks it, other
/// windows are left alone, and a
/// program path or AUMID that isn't plainly safe is cleared.
pub fn import(file: Export, here: &BTreeMap<String, ScreenInfo>, names: &[String], ids: &[String]) -> Workspace {
    let find = |old: &str| -> Option<String> {
        let was = file.screens.get(old);
        if here.contains_key(old) {
            return Some(old.to_string());
        }
        let was = was?;
        let by = |same: &dyn Fn(&ScreenInfo) -> bool| here.iter().find(|(_, s)| same(s)).map(|(id, _)| id.clone());
        by(&|s| s.number == was.number && s.name == was.name).or_else(|| by(&|s| s.number == was.number))
    };
    let mut workspace = file.workspace;
    for rule in &mut workspace.windows {
        match find(&rule.monitor) {
            Some(id) => rule.monitor = id,
            None => rule.target = Target::State { state: WinState::Keep },
        }
    }
    workspace.id = new_id(ids);
    workspace.name = unique_name(&workspace.name, names);
    for rule in &mut workspace.windows {
        rule.launch = false;
        if !safe_exe(&rule.exe) {
            rule.exe.clear();
        }
        rule.aumid = rule.aumid.take().filter(|a| safe_aumid(a));
    }
    workspace.launch_missing = false;
    // Close and Minimize are the user's opt-in, so a file can't arm them.
    workspace.other_windows = OtherWindows::Leave;
    workspace.hotkey = None;
    workspace
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(exe: &str, aumid: Option<&str>, class: &str, title: &str) -> Rule {
        Rule {
            exe: exe.into(),
            aumid: aumid.map(Into::into),
            class: class.into(),
            title_hint: title.into(),
            monitor: "m".into(),
            ..Rule::default()
        }
    }

    fn win(hwnd: isize, exe: &str, aumid: Option<&str>, class: &str, title: &str, z: usize) -> Candidate {
        Candidate { hwnd, exe: exe.into(), aumid: aumid.map(Into::into), class: class.into(), title: title.into(), z }
    }

    #[test]
    fn rules_get_the_right_windows() {
        let ff = r"C:\Program Files\Mozilla Firefox\firefox.exe";
        let terminal = Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App");
        let rules = [
            rule(ff, None, "MozillaWindowClass", "GitHub"),
            rule(ff, None, "MozillaWindowClass", "Docs"),
            rule("x", terminal, "CASCADIA_HOSTING_WINDOW_CLASS", ""),
            rule(r"C:\Spotify.exe", None, "Chrome_WidgetWin_1", ""),
        ];
        let wins = [
            win(1, &ff.to_uppercase(), None, "MozillaWindowClass", "Docs — Mozilla Firefox", 0),
            win(2, ff, None, "MozillaWindowClass", "GitHub — Mozilla Firefox", 1),
            win(3, "other.exe", terminal, "CASCADIA_HOSTING_WINDOW_CLASS", "pwsh", 2),
        ];
        // The title picks which Firefox window each rule gets, the AUMID beats the
        // exe, and a rule with no window gets none.
        assert_eq!(assign(&rules, &wins), vec![Some(2), Some(1), Some(3), None]);

        // A window is claimed once: three rules, two windows.
        let same = [rule(ff, None, "", ""), rule(ff, None, "", ""), rule(ff, None, "", "")];
        let two = [win(1, ff, None, "", "a", 0), win(2, ff, None, "", "b", 1)];
        assert_eq!(assign(&same, &two), vec![Some(1), Some(2), None]);

        // The class breaks a tie before the title or the z-order does.
        let rules = [rule(ff, None, "Right", "")];
        let wins = [win(1, ff, None, "Wrong", "", 0), win(2, ff, None, "Right", "", 5)];
        assert_eq!(assign(&rules, &wins), vec![Some(2)]);

        // A rule with an AUMID never takes a window that has none.
        let rules = [rule(ff, terminal, "", "")];
        assert_eq!(assign(&rules, &[win(1, ff, None, "", "", 0)]), vec![None]);
    }
}
