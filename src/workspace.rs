//! Workspaces: which apps go where on a desk. Pure logic, so the tray's capture
//! and switch and the panel's editor, export and import agree.
//!
//! Zone numbers here count from 1, as the panel shows them.

use crate::zones::{self, Rect, Template, ZoneLayout};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet, hash_map::RandomState};
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, Prefix};

/// A window this close to a zone, in pixels, counts as being in it.
const ZONE_TOLERANCE: i32 = 8;
/// The longest delay before an app starts, and the step it moves in.
pub const MAX_DELAY_MS: u32 = 30_000;
pub const DELAY_STEP_MS: u32 = 500;

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

/// What a window showed at capture, so a switch that starts its app opens it there
/// again (D330): an Explorer window's folder, or a browser's page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Folder(String),
    Url(String),
}

/// The longest page address kept.
const MAX_URL: usize = 2048;

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
    /// How long a switch waits before starting this app, within its stage.
    pub delay_ms: u32,
    /// The next stage starts only once this app's window is up, or took too long.
    pub wait_for_window: bool,
    /// The window's width and height in pixels at capture. A free place restored on
    /// another screen keeps it with "Keep its pixel size" (§11.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_px: Option<[i32; 2]>,
    /// The same in millimetres, when its screen reports a size, for "Keep its real size".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_mm: Option<[f32; 2]>,
    /// Device path of the screen the window was on at capture. `monitor` can be
    /// retargeted later, this stays. A restore on any other screen counts as another
    /// screen. Without it, the saved size decides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_on: Option<String>,
    /// The folder or page the window showed, with "Remember folders and pages" on.
    /// Used only when a switch starts the app, and never exported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
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
            delay_ms: 0,
            wait_for_window: false,
            size_px: None,
            size_mm: None,
            captured_on: None,
            place: None,
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

    /// Show the window another way. Maximized is the screen's full target, and
    /// leaving it puts the window back where it was when it wasn't maximized.
    pub fn set_shown(&mut self, shown: Shown) {
        let full = self.target == (Target::State { state: WinState::Max });
        match shown {
            Shown::Maximized => self.target = Target::State { state: WinState::Max },
            _ if full => {
                self.target = match self.normal_rect {
                    Some(rect) => Target::Rect { rect },
                    None => Target::State { state: WinState::Keep },
                }
            }
            _ => {}
        }
        self.shown = shown;
    }

    /// Where and how a capture found the window, taken into this row. The row
    /// keeps its launch details and its place in the start order.
    fn take_place(&mut self, from: &Rule) {
        self.title_hint = from.title_hint.clone();
        self.class = from.class.clone();
        self.monitor = from.monitor.clone();
        self.target = from.target.clone();
        self.normal_rect = from.normal_rect;
        self.shown = from.shown;
        self.z = from.z;
        self.focused = from.focused;
        self.size_px = from.size_px;
        self.size_mm = from.size_mm;
        self.captured_on = from.captured_on.clone();
        // A capture that read no folder or page keeps the one saved.
        if from.place.is_some() {
            self.place = from.place.clone();
        }
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
    /// Screen device path -> the layout a switch gives that screen, as a desk saves
    /// one, or `no_zones()`. A screen with no entry is kept as it is. Raw, so an
    /// entry this build can't read is kept and keeps its screen as it is.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub zones: BTreeMap<String, Value>,
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
            zones: BTreeMap::new(),
        }
    }
}

/// A workspace's entry for a screen that has no zones. A desk leaves such a screen
/// out, but here no entry means "Keep as is".
pub fn no_zones() -> Value {
    json!({ "template": "none", "tree": "leaf" })
}

/// The layout a workspace's entry gives its screen: `Some(None)` for no zones, and
/// `None` for an entry this build can't read, or with a tree bigger than the editor
/// can make, which keeps the screen as it is.
pub fn screen_layout(saved: &Value) -> Option<Option<ZoneLayout>> {
    let layout = zones::layout_of(saved).filter(|l| zones::within_limits(&l.tree))?;
    Some((layout.template != Template::None).then_some(layout))
}

/// An entry from outside the tray, as it is stored: read and written again, so
/// fields this build doesn't know are gone. `None` for one `screen_layout` refuses.
fn clean_entry(saved: &Value) -> Option<Value> {
    Some(zones_entry(screen_layout(saved)?.as_ref()))
}

/// A screen's layout as a capture records it, with no layout recorded as no zones.
pub fn zones_entry(layout: Option<&ZoneLayout>) -> Value {
    layout.and_then(|l| serde_json::to_value(l).ok()).unwrap_or_else(no_zones)
}

/// Capture and Update from screen: every screen of the desk with its layout now, or
/// no zones. Entries for screens not in `now` stay.
pub fn capture_zones(old: &BTreeMap<String, Value>, now: &[(String, Option<ZoneLayout>)]) -> BTreeMap<String, Value> {
    let mut zones = old.clone();
    for (screen, layout) in now {
        zones.insert(screen.clone(), zones_entry(layout.as_ref()));
    }
    zones
}

/// The screen already has the layout the workspace's entry gives it. The trees are
/// compared, so a screen whose link to a library layout was dropped still matches.
/// An entry this build can't read keeps the screen as it is, so it always matches.
pub fn zones_match(saved: &Value, now: Option<&ZoneLayout>) -> bool {
    screen_layout(saved).is_none_or(|want| want.map(|l| l.tree) == now.map(|l| l.tree.clone()))
}

/// A saved workspace, or `None` for an entry that isn't one. The config keeps
/// them as raw JSON so one bad entry can't stop the file loading (D63).
pub fn parse(saved: &Value) -> Option<Workspace> {
    serde_json::from_value(saved.clone()).ok().filter(|w: &Workspace| !w.id.is_empty())
}

pub fn list(saved: &[Value]) -> Vec<Workspace> {
    saved.iter().filter_map(parse).collect()
}

/// The app rules this build can use: an app, a screen and a place to go to. Others
/// stay in the config and are ignored.
pub fn app_rules(saved: &[Value]) -> Vec<Rule> {
    let usable = |r: &Rule| {
        let named = !r.exe.is_empty() || r.aumid.is_some();
        named && !r.monitor.is_empty() && r.target != (Target::State { state: WinState::Keep })
    };
    saved.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).filter(usable).collect()
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
            // A restore never changes a screen's layout.
            zones: BTreeMap::new(),
        }
    }
}

/// What the `Seam_ws` stamp keeps of a workspace's id: the same number for the
/// same id in every run, so a stamp outlives a tray restart. FNV-1a, never 0.
pub fn stamp_key(id: &str) -> u32 {
    let hash = id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    hash.max(1)
}

/// How a workspace's app is doing right now, as the editor shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    InPlace,
    /// Running, but not where the workspace puts it.
    Moved,
    /// Minimized, and the workspace wants it shown.
    Minimized,
    NotRunning,
    /// Its program runs with no window, as an app closed to the tray does.
    Background,
    /// Runs as administrator, so Seam can't move it.
    CantMove,
}

/// One row of `workspace.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowStatus {
    pub rule_index: usize,
    pub state: RowState,
}

/// Something a switch would put back: Snap back has work to do.
pub fn drifted(rows: &[RowStatus]) -> bool {
    rows.iter().any(|r| matches!(r.state, RowState::Moved | RowState::Minimized))
}

/// The workspace looks like the one in use: it has an app with a window, and every
/// app with a window is where it puts it. One in the background counts as neither.
pub fn all_in_place(rows: &[RowStatus]) -> bool {
    let mut running =
        rows.iter().filter(|r| !matches!(r.state, RowState::NotRunning | RowState::Background)).peekable();
    running.peek().is_some() && running.all(|r| r.state == RowState::InPlace)
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
    rules.iter().map(|r| pick(r, wins, &mut taken, |_| true)).collect()
}

/// Whether the window is one of the rule's app: by AUMID when the rule has one, else by exe.
fn same_app(r: &Rule, w: &Candidate) -> bool {
    match (&r.aumid, &w.aumid) {
        (Some(a), Some(b)) => a == b,
        (Some(_), None) => false,
        (None, _) => r.exe.eq_ignore_ascii_case(&w.exe),
    }
}

/// The best window not taken yet, among those `allowed`, for the rule, which then takes it.
fn pick(r: &Rule, wins: &[Candidate], taken: &mut [bool], allowed: impl Fn(usize) -> bool) -> Option<isize> {
    let (i, w) = wins
        .iter()
        .enumerate()
        .filter(|(i, w)| !taken[*i] && allowed(*i) && same_app(r, w))
        .max_by_key(|(_, w)| (w.class == r.class, common_prefix(&r.title_hint, &w.title), usize::MAX - w.z))?;
    taken[i] = true;
    Some(w.hwnd)
}

/// The window the tray remembers for a row with a saved folder or page, with
/// "Remember folders and pages" on (D336).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// None yet: the tray started since, or the row is older.
    Unbound,
    /// This window, which is still open.
    Alive(isize),
    /// Its window was closed.
    Gone,
}

/// `assign`, with the windows the tray remembers for place rows (D336). `bound[i]` is
/// `None` for a row that isn't one, which matches as `assign` does but never takes a
/// window bound to a place row here. A place row bound to an open window takes that
/// window only, and one whose window was closed takes none, so it's started on its
/// place. An unbound one matches by app, but never takes a window bound to a place
/// row here or in `elsewhere` (other workspaces, or windows a launch must not take).
pub fn assign_bound(
    rules: &[Rule],
    wins: &[Candidate],
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
) -> Vec<Option<isize>> {
    let bound_of = |i: usize| bound.get(i).copied().flatten();
    let mine: HashSet<isize> = (0..rules.len())
        .filter_map(|i| match bound_of(i) {
            Some(Bound::Alive(h)) => Some(h),
            _ => None,
        })
        .collect();
    let mut taken = vec![false; wins.len()];
    let mut found = vec![None; rules.len()];
    // Bound windows first, so no row before them in the order can take them.
    for (i, r) in rules.iter().enumerate() {
        if let Some(Bound::Alive(h)) = bound_of(i) {
            found[i] = pick(r, wins, &mut taken, |w| wins[w].hwnd == h);
        }
    }
    let free = |w: usize| !mine.contains(&wins[w].hwnd);
    for (i, r) in rules.iter().enumerate() {
        found[i] = match bound_of(i) {
            Some(Bound::Alive(_)) => found[i],
            Some(Bound::Gone) => None,
            Some(Bound::Unbound) => pick(r, wins, &mut taken, |w| free(w) && !elsewhere.contains(&wins[w].hwnd)),
            None => pick(r, wins, &mut taken, free),
        };
    }
    found
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

/// A capture's rules as the matcher sees windows, so a recapture finds the rows a
/// switch would.
fn as_candidates(fresh: &[Rule]) -> Vec<Candidate> {
    let cand = |(i, r): (usize, &Rule)| Candidate {
        hwnd: i as isize,
        exe: r.exe.clone(),
        aumid: r.aumid.clone(),
        class: r.class.clone(),
        title: r.title_hint.clone(),
        z: i,
    };
    fresh.iter().enumerate().map(cand).collect()
}

/// The capture's window, by index, for each row of `old`, paired as a switch matches.
/// `bound` and `elsewhere` are the tray's bindings (D336) with windows as capture
/// indices, so a place row pairs with its own window. Empty for plain matching.
fn pair(old: &[Rule], fresh: &[Rule], bound: &[Option<Bound>], elsewhere: &HashSet<isize>) -> Vec<Option<isize>> {
    assign_bound(old, &as_candidates(fresh), bound, elsewhere)
}

/// What "Update from screen" makes of a workspace's rows.
pub struct Merged {
    pub windows: Vec<Rule>,
    /// For each row of `windows`, the row of the old workspace it kept, or `None` for
    /// one the capture added.
    pub from: Vec<Option<usize>>,
    /// For each window of the capture, its row in `windows`.
    pub rows: Vec<usize>,
    /// How many rows were removed.
    pub removed: usize,
}

/// Whether some window in `wins` is of the row's app: the same program, or the same AUMID.
fn app_open(row: &Rule, wins: &[&Candidate]) -> bool {
    wins.iter().any(|w| {
        (!row.exe.is_empty() && w.exe.eq_ignore_ascii_case(&row.exe))
            || row.aumid.as_ref().is_some_and(|a| w.aumid.as_ref() == Some(a))
    })
}

/// "Update from screen": each row the capture has a window for takes where and how
/// that window is now, and keeps its launch details and its place in the start
/// order. A row with no window right now stays. With `removes` (the setting "Update
/// from screen removes apps that aren't open") it goes instead, unless its app still
/// has a window the capture didn't give a row: bound elsewhere, or in `others` (windows
/// of the desk it made no rule of, such as on another desktop). Nothing is removed
/// when that would leave no rows of `old`. New windows go at the end, topmost first.
/// `bound` and `elsewhere` as `pair`.
pub fn merge_capture(
    old: &Workspace,
    fresh: Vec<Rule>,
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
    others: &[Candidate],
    removes: bool,
) -> Merged {
    let found = pair(&old.windows, &fresh, bound, elsewhere);
    let mut taken = vec![false; fresh.len()];
    for i in found.iter().flatten() {
        taken[*i as usize] = true;
    }
    let cands = as_candidates(&fresh);
    let unpaired = || cands.iter().zip(&taken).filter(|(_, t)| !**t).map(|(c, _)| c);
    // A row whose own window was closed isn't kept by another window of its app, unless
    // that one is held by another workspace's row or is one the capture made no row of.
    let held: Vec<&Candidate> = unpaired().filter(|c| elsewhere.contains(&c.hwnd)).chain(others).collect();
    let open: Vec<&Candidate> = unpaired().chain(others).collect();
    let gone: Vec<bool> = old
        .windows
        .iter()
        .zip(&found)
        .enumerate()
        .map(|(o, (row, hit))| {
            let wins = match bound.get(o).copied().flatten() {
                Some(Bound::Gone) => &held,
                // Its own window is open, though the capture took none of it.
                Some(Bound::Alive(_)) => return false,
                _ => &open,
            };
            removes && hit.is_none() && !app_open(row, wins)
        })
        .collect();
    // A capture that finds nothing of the workspace would only empty it.
    let gone = if gone.iter().all(|g| *g) { vec![false; gone.len()] } else { gone };
    let mut windows = Vec::with_capacity(old.windows.len());
    let mut from = Vec::with_capacity(old.windows.len());
    let mut rows = vec![0; fresh.len()];
    for (o, (row, hit)) in old.windows.iter().zip(&found).enumerate() {
        if gone[o] {
            continue;
        }
        let mut row = row.clone();
        match hit {
            Some(i) => {
                rows[*i as usize] = windows.len();
                row.take_place(&fresh[*i as usize]);
            }
            // With no window, nothing says how it stacks or whether it has the focus.
            None => {
                row.z = u32::MAX;
                row.focused = false;
            }
        }
        windows.push(row);
        from.push(Some(o));
    }
    for (i, rule) in fresh.into_iter().enumerate().filter(|(i, _)| !taken[*i]) {
        rows[i] = windows.len();
        windows.push(rule);
        from.push(None);
    }
    let removed = gone.iter().filter(|g| **g).count();
    Merged { windows, from, rows, removed }
}

/// Where each row of `old` went in `new` after an edit reordered, removed or added
/// rows: the same rule first, then the same app the way a switch matches, for a
/// row that also changed. `None` for a row that was removed.
pub fn moved_rows(old: &[Rule], new: &[Rule]) -> Vec<Option<usize>> {
    let mut taken = vec![false; new.len()];
    let mut moves: Vec<Option<usize>> = old
        .iter()
        .map(|rule| {
            let j = (0..new.len()).find(|j| !taken[*j] && new[*j] == *rule)?;
            taken[j] = true;
            Some(j)
        })
        .collect();
    let left: Vec<usize> = (0..new.len()).filter(|j| !taken[*j]).collect();
    let changed: Vec<usize> = (0..old.len()).filter(|i| moves[*i].is_none()).collect();
    let rules: Vec<Rule> = changed.iter().map(|i| old[*i].clone()).collect();
    let cands = as_candidates(&left.iter().map(|j| new[*j].clone()).collect::<Vec<_>>());
    for (i, hit) in changed.into_iter().zip(assign(&rules, &cands)) {
        moves[i] = hit.map(|k| left[k as usize]);
    }
    moves
}

/// "Where it is now" for one row: the place of the window a capture has for it,
/// and which of the capture's windows that is, or `None` when the app has no window open.
/// `bound` and `elsewhere` as `pair`, for every row of `old`.
pub fn recapture_row(
    old: &Workspace,
    row: usize,
    fresh: &[Rule],
    bound: &[Option<Bound>],
    elsewhere: &HashSet<isize>,
) -> Option<(Workspace, usize)> {
    let rule = old.windows.get(row)?;
    let bound = [bound.get(row).copied().flatten()];
    let hit = pair(std::slice::from_ref(rule), fresh, &bound, elsewhere).into_iter().next().flatten()?;
    let from = &fresh[hit as usize];
    let mut updated = old.clone();
    if from.focused {
        updated.windows.iter_mut().for_each(|r| r.focused = false);
    }
    updated.windows[row].take_place(from);
    Some((updated, hit as usize))
}

/// "Add an app" from the windows open now: the row takes the folder or page the tray
/// read from the one window picked, `picked`. Only a row that is still the one the
/// panel added (its `exe` and `class`), has no place yet, and is the picked window's
/// app and class. `None` otherwise, or when the window has none.
pub fn take_window_place(old: &Workspace, row: usize, exe: &str, class: &str, picked: &[Rule]) -> Option<Workspace> {
    let rule = old.windows.get(row).filter(|r| r.exe == exe && r.class == class && r.place.is_none())?;
    let [from] = picked else { return None };
    if from.class != rule.class {
        return None;
    }
    assign(std::slice::from_ref(rule), &as_candidates(picked)).into_iter().next().flatten()?;
    let mut updated = old.clone();
    updated.windows[row].place = Some(from.place.clone()?);
    Some(updated)
}

/// The rows a switch starts, in start order, cut into stages. A stage ends at a
/// row that waits for its window. Apps in a stage start together, each after its
/// own delay, and the next stage starts once the waiting ones are up.
pub fn stages(rules: &[Rule], starting: &[usize]) -> Vec<Vec<usize>> {
    let mut stages: Vec<Vec<usize>> = Vec::new();
    let mut open = false;
    for &i in starting {
        match stages.last_mut() {
            Some(stage) if open => stage.push(i),
            _ => stages.push(vec![i]),
        }
        open = !rules.get(i).is_some_and(|r| r.wait_for_window);
    }
    stages
}

/// A delay as a switch uses it: 0 to 30 s, in half seconds.
pub fn tame_delay(ms: u32) -> u32 {
    (ms.min(MAX_DELAY_MS) + DELAY_STEP_MS / 2) / DELAY_STEP_MS * DELAY_STEP_MS
}

/// What the editor sends, made safe to save over `old`. The id stays, a zone it
/// picks must be on its screen (`zones` says how many a screen has), and a program or
/// AUMID that wasn't in the workspace already must pass the same checks as an
/// import. A row with nothing safe to start can't be ticked to start, which keeps
/// an imported workspace's cleared rows from launching. Errors say why, in the
/// panel's words.
pub fn checked_update(old: &Workspace, mut new: Workspace, zones: &dyn Fn(&str) -> usize) -> Result<Workspace, String> {
    if new.id != old.id {
        return Err("That's a different workspace".into());
    }
    let name = new.name.trim().to_string();
    new.name = if name.is_empty() { old.name.clone() } else { name };
    let mut focused = false;
    for rule in &mut new.windows {
        let known = old.windows.iter().any(|o| o.exe == rule.exe && o.aumid == rule.aumid);
        let exe_ok = rule.exe.is_empty() || safe_exe(&rule.exe);
        let aumid_ok = rule.aumid.as_deref().is_none_or(safe_aumid);
        if !(known || exe_ok && aumid_ok) {
            return Err(format!("{} wasn't added, because Seam won't start that program", rule.title_hint));
        }
        // Only a place the editor chose is checked, so a layout changed since keeps the
        // workspace editable.
        let placed = old.windows.iter().any(|o| o.monitor == rule.monitor && o.target == rule.target);
        if let (Target::Zone { zone, span_to }, false) = (&rule.target, placed) {
            let (zone, span_to, count) = (*zone, *span_to, zones(&rule.monitor) as u32);
            if !(1..=count).contains(&zone) || span_to.is_some_and(|to| !(1..=count).contains(&to)) {
                return Err("That zone isn't on its screen any more".into());
            }
        }
        let startable = (exe_ok && !rule.exe.is_empty()) || rule.aumid.as_deref().is_some_and(safe_aumid);
        rule.launch &= startable;
        rule.delay_ms = tame_delay(rule.delay_ms);
        // Pins belong to the last session.
        rule.pin = None;
        // Only a capture finds a folder or page, so the editor can keep or drop one but not
        // set one, nor move one to another app's row.
        let place = rule.place.take();
        let same_app = |o: &Rule| o.exe == rule.exe && (rule.class.is_empty() || o.class == rule.class);
        let kept = place.filter(|p| old.windows.iter().any(|o| same_app(o) && o.place.as_ref() == Some(p)));
        rule.place = kept;
        // At most one window has the focus.
        rule.focused &= !focused;
        focused |= rule.focused;
    }
    // A screen's entry the editor changed must be a layout within the editor's limits,
    // and is stored as read. One it didn't change stays as it was.
    let mut zones = BTreeMap::new();
    for (screen, saved) in std::mem::take(&mut new.zones) {
        let entry = if old.zones.get(&screen) == Some(&saved) { Some(saved) } else { clean_entry(&saved) };
        let Some(entry) = entry else { return Err("That layout can't be used for a screen".into()) };
        zones.insert(screen, entry);
    }
    new.zones = zones;
    Ok(new)
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

/// A folder Seam may open in Explorer: an absolute path on a drive letter, with no
/// `/`, `..`, `"` or control character. Whether it exists and isn't on a network
/// drive is checked where it's read and where it's opened.
pub fn safe_folder(folder: &str) -> bool {
    let path = Path::new(folder);
    let mut parts = path.components();
    let on_drive = matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)));
    on_drive
        && matches!(parts.next(), Some(Component::RootDir))
        && !folder.contains(['/', '"'])
        && !folder.chars().any(char::is_control)
        && !path.components().any(|c| matches!(c, Component::ParentDir))
}

/// Explorer's argument for a folder: quoted, without a trailing `\` that would
/// escape the quote. A drive's root has no space or comma, so it goes as it is.
pub fn explorer_arg(folder: &str) -> String {
    match folder.trim_end_matches('\\') {
        root if root.len() == 2 => format!(r"{root}\"),
        folder => format!("\"{folder}\""),
    }
}

/// A page Seam may open in a browser: an http or https address with a host, not too
/// long, and with no space, quote or control character, so it reaches the browser
/// as one argument. A user name or password before the host (`user@host`) is refused.
pub fn safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    let authority = rest.and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or_default();
    !authority.is_empty()
        && !authority.contains('@')
        && url.len() <= MAX_URL
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
}

/// Bits of a query parameter's name that say it can carry a sign-in or a secret. A
/// page with any such parameter is kept without its query.
const SECRET_PARTS: [&str; 13] =
    ["token", "secret", "key", "pass", "sig", "auth", "code", "sess", "cred", "otp", "jwt", "sid", "state"];

/// `%XX` escapes turned back into the characters they stand for.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()).filter(|_| bytes[i] == b'%') {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A page as it's stored: without its query when a parameter's name, decoded, has a
/// bit of `SECRET_PARTS` in it, and without its fragment when it holds a value.
/// Otherwise as it is.
fn without_secrets(page: &str) -> String {
    let (rest, fragment) = page.split_once('#').map_or((page, None), |(r, f)| (r, Some(f)));
    let (base, query) = rest.split_once('?').map_or((rest, None), |(b, q)| (b, Some(q)));
    let name = |param: &str| percent_decoded(param.split('=').next().unwrap_or_default()).to_lowercase();
    let secret = |param: &str| {
        let name = name(param);
        SECRET_PARTS.iter().any(|s| name.contains(s))
    };
    let mut kept = base.to_string();
    if let Some(query) = query.filter(|q| !q.split('&').any(secret)) {
        kept = format!("{kept}?{query}");
    }
    if let Some(fragment) = fragment.filter(|f| !f.contains('=')) {
        kept = format!("{kept}#{fragment}");
    }
    kept
}

/// The page a browser's address bar shows, as it's stored, or `None` for anything but
/// an http or https page. Chromium and Firefox hide the scheme, so a bare host gets it
/// back: `http://` for localhost, an IP address, or a name with no dot and a port, and
/// `https://` for a name with a dot. Other text with no dot, a search word, isn't a page.
pub fn page_of(bar: &str) -> Option<String> {
    let bar = bar.trim();
    let lower = bar.to_ascii_lowercase();
    let page = if lower.starts_with("http://") || lower.starts_with("https://") {
        bar.to_string()
    } else {
        let host = bar.split(['/', '?', '#']).next().unwrap_or_default();
        let (name, port) = host.split_once(':').map_or((host, None), |(n, p)| (n, Some(p)));
        let named = !name.is_empty() && !name.starts_with('.') && !name.ends_with('.');
        let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
        let port_ok = port.is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let octets: Vec<&str> = name.split('.').collect();
        let ip = octets.len() == 4 && octets.iter().all(|o| !o.is_empty() && o.chars().all(|c| c.is_ascii_digit()));
        let localhost = name.eq_ignore_ascii_case("localhost");
        let dotless = !name.contains('.');
        if !(named && plain && port_ok) || dotless && !localhost && port.is_none() {
            return None;
        }
        let local = localhost || ip || dotless;
        format!("{}://{bar}", if local { "http" } else { "https" })
    };
    let page = without_secrets(&page);
    safe_url(&page).then_some(page)
}

/// Whether any of these saved workspaces remembers a folder or page.
pub fn has_places(saved: &[Value]) -> bool {
    let rows = |ws: &Value| ws.get("windows").and_then(Value::as_array).cloned().unwrap_or_default();
    saved.iter().any(|ws| rows(ws).iter().any(|r| r.get("place").is_some()))
}

/// "Forget saved folders and pages": the saved workspaces as stored, without them.
pub fn without_places(saved: &[Value]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for ws in &mut saved {
        let rows = ws.get_mut("windows").and_then(Value::as_array_mut);
        for row in rows.into_iter().flatten() {
            if let Some(fields) = row.as_object_mut() {
                fields.remove("place");
            }
        }
    }
    saved
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
/// screen here stays where it is. A screen's layout goes to the screen found the
/// same way, without its link to a library layout on the other machine, and one
/// with no screen here is dropped. The shortcut is dropped, so it can't clash. A file
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
    let mut zones = BTreeMap::new();
    for (screen, saved) in std::mem::take(&mut workspace.zones) {
        let (Some(id), Some(mut layout)) = (find(&screen), clean_entry(&saved)) else { continue };
        if let Some(fields) = layout.as_object_mut() {
            fields.remove("custom");
        }
        zones.entry(id).or_insert(layout);
    }
    workspace.zones = zones;
    workspace.id = new_id(ids);
    workspace.name = unique_name(&workspace.name, names);
    for rule in &mut workspace.windows {
        rule.launch = false;
        if !safe_exe(&rule.exe) {
            rule.exe.clear();
        }
        rule.aumid = rule.aumid.take().filter(|a| safe_aumid(a));
        // A folder or page from a file is never opened (D98, D330).
        rule.place = None;
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

        // A recapture matches the same way. A matched row keeps its launch details and
        // place, a row with no window stays, and a new window goes at the end.
        let spotify = r"C:\Spotify.exe";
        let mut docs = rule(ff, None, "MozillaWindowClass", "Docs");
        (docs.launch, docs.delay_ms, docs.wait_for_window) = (false, 1500, true);
        let old = Workspace { windows: vec![docs, rule(spotify, None, "", "")], ..Workspace::default() };
        let mut moved = rule(ff, None, "MozillaWindowClass", "Docs — Mozilla Firefox");
        moved.target = Target::Zone { zone: 2, span_to: None };
        let fresh = vec![rule("notepad.exe", None, "", ""), moved];
        let none = HashSet::new();
        let merged = merge_capture(&old, fresh.clone(), &[], &none, &[], false).windows;
        let rows: Vec<(&str, bool, u32, bool)> =
            merged.iter().map(|r| (r.exe.as_str(), r.launch, r.delay_ms, r.wait_for_window)).collect();
        assert_eq!(rows, [(ff, false, 1500, true), (spotify, true, 0, false), ("notepad.exe", true, 0, false)]);
        assert_eq!(merged[0].target, Target::Zone { zone: 2, span_to: None });

        // With the setting on, the row with no window goes, and the others say where they were.
        let on = merge_capture(&old, fresh.clone(), &[], &none, &[], true);
        let exes: Vec<&str> = on.windows.iter().map(|r| r.exe.as_str()).collect();
        assert_eq!((exes, on.removed), ([ff, "notepad.exe"].to_vec(), 1));
        assert_eq!((on.from, on.rows), (vec![Some(0), None], vec![1, 0]));
        // It stays when its app has a window the capture made no row of, or nothing else is left.
        let other =
            Candidate { hwnd: 9, exe: spotify.into(), aumid: None, class: String::new(), title: String::new(), z: 0 };
        assert_eq!(merge_capture(&old, fresh.clone(), &[], &none, &[other], true).removed, 0);
        assert_eq!(merge_capture(&old, vec![rule("notepad.exe", None, "", "")], &[], &none, &[], true).removed, 0);

        // Two Explorer place rows, the first one's window closed: it goes, the second keeps its
        // binding at row 0, and Undo's map gives the old row back.
        let explorer = r"C:\Windows\explorer.exe";
        let (mut a, mut b) = (rule(explorer, None, "CabinetWClass", "A"), rule(explorer, None, "CabinetWClass", "B"));
        a.place = Some(Place::Folder(r"C:\A".into()));
        b.place = Some(Place::Folder(r"C:\B".into()));
        let old = Workspace { windows: vec![a, b], ..Workspace::default() };
        let open = rule(explorer, None, "CabinetWClass", "B");
        let bound = [Some(Bound::Gone), Some(Bound::Alive(0))];
        let merged = merge_capture(&old, vec![open], &bound, &none, &[], true);
        assert_eq!((merged.windows.len(), merged.removed, merged.from, merged.rows), (1, 1, vec![Some(1)], vec![0]));
        assert_eq!(merged.windows[0].place, old.windows[1].place);
        // Another Explorer window nobody holds doesn't keep the closed row.
        let extra = rule(explorer, None, "CabinetWClass", "C");
        let both =
            merge_capture(&old, vec![rule(explorer, None, "CabinetWClass", "B"), extra], &bound, &none, &[], true);
        assert_eq!((both.removed, both.from), (1, vec![Some(1), None]));

        // Stages end at a row that waits for its window.
        let mut waits: Vec<Rule> = (0..5).map(|_| rule(ff, None, "", "")).collect();
        (waits[1].wait_for_window, waits[4].wait_for_window) = (true, true);
        assert_eq!(stages(&waits, &[0, 1, 2, 3, 4]), vec![vec![0, 1], vec![2, 3, 4]]);
        assert_eq!(stages(&waits, &[0, 2]), vec![vec![0, 2]]);
        assert_eq!(stages(&waits, &[1, 4]), vec![vec![1], vec![4]]);
        assert_eq!((tame_delay(1200), tame_delay(99_000)), (1000, MAX_DELAY_MS));
    }

    // "No zones" and "Keep as is" must not be confused, or a switch takes a screen's zones away.
    #[test]
    fn a_capture_records_no_zones_and_keep_as_is_stays_apart() {
        let rows = ZoneLayout::of(Template::Rows).unwrap();
        let old = BTreeMap::from([("gone".to_string(), json!(rows)), ("a".to_string(), json!(rows))]);
        let captured = capture_zones(&old, &[("a".into(), None), ("b".into(), Some(rows.clone()))]);
        assert_eq!(screen_layout(&captured["a"]), Some(None));
        assert_eq!(screen_layout(&captured["b"]), Some(Some(rows.clone())));
        // A screen the capture didn't see keeps its entry.
        assert_eq!(captured["gone"], json!(rows));
        // An entry this build can't read keeps the screen as it is.
        assert_eq!(screen_layout(&json!({ "template": "spiral" })), None);
        assert!(zones_match(&json!("?"), None));
        assert!(zones_match(&no_zones(), None) && !zones_match(&no_zones(), Some(&rows)));
        assert!(!zones_match(&captured["b"], None));
        // A tree deeper than the editor can make keeps the screen as it is, and an
        // import leaves it out.
        let deep = (0..=zones::MAX_DEPTH)
            .fold(json!("leaf"), |a, _| json!({ "split": "vertical", "ratio": 0.5, "a": a, "b": "leaf" }));
        let deep = json!({ "template": "custom", "tree": deep });
        assert_eq!(screen_layout(&deep), None);
        let mut ws = Workspace { id: "w".into(), ..Workspace::default() };
        ws.zones = BTreeMap::from([("m".to_string(), deep), ("n".to_string(), json!(rows))]);
        let here = BTreeMap::from(
            [("m", 1), ("n", 2)]
                .map(|(id, number)| (id.to_string(), ScreenInfo { number, name: String::new(), width: 0, height: 0 })),
        );
        let file = Export { seam_workspace: Export::VERSION, workspace: ws, screens: BTreeMap::new() };
        let imported = import(file, &here, &[], &[]);
        assert_eq!(imported.zones.keys().collect::<Vec<_>>(), ["n"]);
    }

    // What a place row matches decides which window is moved and which app is started
    // (D336): its own window while open, none once closed, by app when not bound yet,
    // and no other row takes a window bound to one.
    #[test]
    fn place_rows_keep_their_own_windows() {
        let chrome = r"C:\Chrome\chrome.exe";
        // Window 2 is the topmost, so plain matching would give it to the first row.
        let wins: Vec<Candidate> =
            [(1, 1), (2, 0), (3, 2), (4, 3)].map(|(h, z)| win(h, chrome, None, "", "", z)).into();
        let rules: Vec<Rule> = (0..5).map(|_| rule(chrome, None, "", "")).collect();
        let bound = [Some(Bound::Unbound), Some(Bound::Alive(2)), Some(Bound::Gone), None, Some(Bound::Alive(9))];
        let elsewhere = HashSet::from([1]);
        // The unbound row goes by app but skips window 2, bound here, and window 1, bound
        // in another workspace. Window 2 stays with its bound row. The closed one gets
        // nothing, so it's started. The plain row doesn't steal window 2 either. One bound
        // to a window that isn't a candidate gets none.
        assert_eq!(assign_bound(&rules, &wins, &bound, &elsewhere), vec![Some(3), Some(2), None, Some(1), None]);
        // With nothing bound, it's plain matching.
        assert_eq!(assign_bound(&rules, &wins, &[None; 5], &HashSet::new()), assign(&rules, &wins));
    }

    // A saved place is opened when a switch starts the app, so the editor and the API
    // can keep one on its own app's row, but not make one up or move it to another app.
    #[test]
    fn an_edit_never_adds_a_place() {
        let (explorer, chrome) = (r"C:\Windows\explorer.exe", r"C:\Chrome\chrome.exe");
        let docs = Some(Place::Folder(r"C:\Docs".into()));
        let mut saved = rule(explorer, None, "CabinetWClass", "Docs");
        saved.place = docs.clone();
        let old = Workspace {
            id: "w".into(),
            windows: vec![saved.clone(), rule(chrome, None, "", "")],
            ..Workspace::default()
        };
        let mut moved = rule(chrome, None, "", "");
        moved.place = docs.clone();
        let mut other_class = rule(explorer, None, "Other", "");
        other_class.place = docs.clone();
        let mut made_up = saved.clone();
        made_up.place = Some(Place::Url("https://a.com".into()));
        let new = Workspace { windows: vec![saved, moved, other_class, made_up], ..old.clone() };
        let places: Vec<_> = checked_update(&old, new, &|_| 0).unwrap().windows.into_iter().map(|r| r.place).collect();
        assert_eq!(places, vec![docs, None, None, None]);
    }

    // A remembered folder or page is handed to Explorer or a browser, so only a local
    // folder and an http or https page may get through, each as one argument.
    #[test]
    fn only_local_folders_and_web_pages_are_opened() {
        assert!(safe_folder(r"C:\Users\me\Documents") && safe_folder(r"D:\"));
        let refused = [r"\\server\share", r"\\?\C:\x", r"C:\a\..\b", "C:relative", "::{20D04FE0}", r#"C:\a"b"#, "C:/a"];
        assert!(refused.iter().all(|f| !safe_folder(f)), "{refused:?}");
        assert_eq!(explorer_arg(r"C:\My Files\"), r#""C:\My Files""#);
        assert_eq!(explorer_arg(r"C:\"), r"C:\");

        assert!(safe_url("https://example.com/a?b=c#d") && safe_url("HTTP://localhost:8080"));
        let refused = ["file:///C:/x", "javascript:alert(1)", "https://", "https:///x", "https://a b", "https://a\"b"];
        assert!(refused.iter().all(|u| !safe_url(u)), "{refused:?}");
        assert!(!safe_url(&format!("https://a.com/{}", "x".repeat(MAX_URL))));
        assert!(!safe_url("https://me:pw@a.com/") && safe_url("https://a.com/@me"));

        assert_eq!(page_of("https://github.com/x").as_deref(), Some("https://github.com/x"));
        assert_eq!(page_of("github.com/x?q=1#top").as_deref(), Some("https://github.com/x?q=1#top"));
        assert_eq!(page_of("localhost:3000/").as_deref(), Some("http://localhost:3000/"));
        assert_eq!(page_of("192.168.1.2/a").as_deref(), Some("http://192.168.1.2/a"));
        assert_eq!(page_of("intranet:8080/wiki").as_deref(), Some("http://intranet:8080/wiki"));
        // A query that could hold a secret goes as a whole, and a fragment with a value.
        assert_eq!(page_of("https://a.com/cb?x=1&Code=9#t=2").as_deref(), Some("https://a.com/cb"));
        assert_eq!(page_of("https://a.com/?q=code#access").as_deref(), Some("https://a.com/?q=code#access"));
        for bar in [
            "about:blank",
            "edge://settings",
            "chrome://newtab",
            "rust lang",
            "rust",
            "intranet/wiki",
            "javascript:a.b",
            "",
            "x:y.z",
        ] {
            assert_eq!(page_of(bar), None, "{bar}");
        }
    }
}
