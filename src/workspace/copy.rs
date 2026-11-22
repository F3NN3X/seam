//! Copying a desk's workspaces to another desk with the same number of screens (D741-D745):
//! each screen's apps go to the screen in the same place.

use super::*;
use crate::config::{Config, DeskScreen};

/// Why a copy can't be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// No desk has that signature, or it has no workspaces.
    NoSource,
    /// The connected desk has workspaces already.
    HasWorkspaces,
    DifferentCounts,
    /// One of the desks was never connected by a Seam that remembers its screens.
    ScreensUnknown,
}

impl CopyError {
    pub fn message(self) -> &'static str {
        match self {
            CopyError::NoSource => "No such desk, or it has no workspaces",
            CopyError::HasWorkspaces => "This desk already has workspaces",
            CopyError::DifferentCounts => "The desks have different numbers of screens",
            CopyError::ScreensUnknown => "That desk's screens aren't known yet",
        }
    }
}

/// A screen's middle and height, in the positions both desks have: millimetres when every
/// screen of both is lined up, else Windows' pixels.
fn centres(screens: &[DeskScreen], physical: bool) -> Vec<(f64, f64, f64)> {
    screens
        .iter()
        .map(|s| match (physical, s.mm) {
            (true, Some([x, y, w, h])) => {
                (f64::from(x) + f64::from(w) / 2.0, f64::from(y) + f64::from(h) / 2.0, f64::from(h))
            }
            _ => {
                let [x, y, w, h] = s.rect.map(f64::from);
                (x + w / 2.0, y + h / 2.0, h)
            }
        })
        .collect()
}

/// `of` left to right by centre, and top to bottom among screens whose centres are within 10 %
/// of a height of each other across.
fn reading_order(of: &[usize], centres: &[(f64, f64, f64)]) -> Vec<usize> {
    let mut by_x = of.to_vec();
    by_x.sort_by(|a, b| centres[*a].0.total_cmp(&centres[*b].0));
    let mut out: Vec<usize> = Vec::with_capacity(by_x.len());
    let mut column: Vec<usize> = Vec::new();
    for i in by_x {
        let same = column
            .first()
            .is_some_and(|first| (centres[i].0 - centres[*first].0).abs() <= 0.1 * centres[i].2.min(centres[*first].2));
        if !same {
            column.sort_by(|a, b| centres[*a].1.total_cmp(&centres[*b].1));
            out.append(&mut column);
        }
        column.push(i);
    }
    column.sort_by(|a, b| centres[*a].1.total_cmp(&centres[*b].1));
    out.append(&mut column);
    out
}

/// Which screen of `to` takes the place of each screen of `from`, by device path, `from`'s id to
/// `to`'s: the same device path first, then main to main, then the rest left to right and top to
/// bottom. `None` when the counts differ or there are no screens.
pub fn map_screens(from: &[DeskScreen], to: &[DeskScreen]) -> Option<BTreeMap<String, String>> {
    if from.len() != to.len() || from.is_empty() {
        return None;
    }
    let physical = from.iter().chain(to).all(|s| s.mm.is_some());
    let (from_at, to_at) = (centres(from, physical), centres(to, physical));
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (i, f) in from.iter().enumerate() {
        if let Some(j) = to.iter().position(|t| t.id == f.id) {
            pairs.push((i, j));
        }
    }
    let left = |pairs: &[(usize, usize)], n: usize, to_side: bool| -> Vec<usize> {
        (0..n).filter(|i| !pairs.iter().any(|p| if to_side { p.1 == *i } else { p.0 == *i })).collect()
    };
    let main_of = |list: &[DeskScreen], rest: Vec<usize>| rest.into_iter().find(|i| list[*i].primary);
    let mains = (main_of(from, left(&pairs, from.len(), false)), main_of(to, left(&pairs, to.len(), true)));
    if let (Some(i), Some(j)) = mains {
        pairs.push((i, j));
    }
    let rest_from = reading_order(&left(&pairs, from.len(), false), &from_at);
    let rest_to = reading_order(&left(&pairs, to.len(), true), &to_at);
    pairs.extend(rest_from.into_iter().zip(rest_to));
    Some(pairs.into_iter().map(|(i, j)| (from[i].id.clone(), to[j].id.clone())).collect())
}

/// The workspaces of `list` as copies for another desk, raw so keys this build doesn't know
/// survive: each row's `monitor` and each key of `zones` go through `map` (one that isn't in it
/// stays as it was), each gets a new id, and a name that clashes gets " 2". Entries this build
/// can't read are left out.
pub fn copy_workspaces(
    list: &[Value],
    map: &BTreeMap<String, String>,
    taken_names: &[String],
    mut new_id: impl FnMut() -> String,
) -> Vec<Value> {
    let mut names = taken_names.to_vec();
    let mut out = Vec::new();
    for saved in list.iter().filter(|v| parse(v).is_some()) {
        let mut copy = saved.clone();
        let Some(fields) = copy.as_object_mut() else { continue };
        fields.insert("id".into(), Value::String(new_id()));
        let name = fields.get("name").and_then(Value::as_str).unwrap_or_default();
        let name = unique_name(name, &names);
        names.push(name.clone());
        fields.insert("name".into(), Value::String(name));
        if let Some(rows) = fields.get_mut("windows").and_then(Value::as_array_mut) {
            for row in rows.iter_mut().filter_map(Value::as_object_mut) {
                let moved = row.get("monitor").and_then(Value::as_str).and_then(|m| map.get(m)).cloned();
                if let Some(id) = moved {
                    row.insert("monitor".into(), Value::String(id));
                }
            }
        }
        if let Some(Value::Object(zones)) = fields.get_mut("zones") {
            *zones = std::mem::take(zones)
                .into_iter()
                .map(|(screen, layout)| (map.get(&screen).cloned().unwrap_or(screen), layout))
                .collect();
        }
        out.push(copy);
    }
    out
}

/// The desks whose workspaces could be copied to the connected one (D742), most recently seen
/// first: the connected desk has none, and the other has some, the same number of screens
/// and its screens recorded.
pub fn copy_sources(cfg: &Config, here_sig: &str, here_count: usize) -> Vec<String> {
    if here_count == 0 || cfg.desks.get(here_sig).is_some_and(|d| !list(&d.workspaces).is_empty()) {
        return Vec::new();
    }
    let mut found: Vec<(&String, u64)> = cfg
        .desks
        .iter()
        .filter(|(sig, d)| sig.as_str() != here_sig && d.screens.len() == here_count && !list(&d.workspaces).is_empty())
        .map(|(sig, d)| (sig, d.last_seen))
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    found.into_iter().map(|(sig, _)| sig.clone()).collect()
}

/// Copy the workspaces of desk `from` onto the empty desk `to`, with the ids of the copies.
/// Which one is in use, the default, the previous one and the sign-in choice stay behind.
pub fn copy_between(
    cfg: &mut Config,
    from: &str,
    to: &str,
    mut new_id: impl FnMut(&[String]) -> String,
) -> Result<Vec<String>, CopyError> {
    let source = cfg.desks.get(from).filter(|d| !list(&d.workspaces).is_empty()).ok_or(CopyError::NoSource)?;
    let here = cfg.desks.get(to);
    if here.is_some_and(|d| !list(&d.workspaces).is_empty()) {
        return Err(CopyError::HasWorkspaces);
    }
    let here_screens = here.map(|d| d.screens.as_slice()).unwrap_or_default();
    if source.screens.is_empty() || here_screens.is_empty() {
        return Err(CopyError::ScreensUnknown);
    }
    let map = map_screens(&source.screens, here_screens).ok_or(CopyError::DifferentCounts)?;
    let mut ids: Vec<String> = cfg.desks.values().flat_map(|d| list(&d.workspaces)).map(|w| w.id).collect();
    let taken: Vec<String> =
        here.map(|d| list(&d.workspaces).into_iter().map(|w| w.name).collect()).unwrap_or_default();
    let copies = copy_workspaces(&source.workspaces, &map, &taken, || {
        let id = new_id(&ids);
        ids.push(id.clone());
        id
    });
    let copied: Vec<String> = copies.iter().filter_map(|c| c["id"].as_str().map(String::from)).collect();
    cfg.desks.entry(to.to_string()).or_default().workspaces.extend(copies);
    Ok(copied)
}

/// What the restore offer looks at (D747).
#[derive(Clone, Copy, Debug, Default)]
pub struct OfferFacts {
    /// "Offer to restore after screens change" is on.
    pub on: bool,
    /// A workspace is in use on the connected desk.
    pub in_use: bool,
    /// At least one of its windows isn't in place.
    pub moved: bool,
    /// A switch, capture, Line Up or sign-in offer is going on.
    pub busy: bool,
    /// Seam is paused or standing aside.
    pub standing_by: bool,
    /// The new-desk balloon was just shown, and has the notice slot.
    pub new_desk_balloon: bool,
}

/// About how long a balloon stays up, so how long an "Open <name>?" notice that nobody clicked
/// can hold the restore offer back, whether or not the shell told us it faded.
pub const SIGN_IN_OFFER_LIFE: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether the sign-in action still holds the restore offer back: its switch is waiting or
/// running, or its "Open <name>?" notice, shown `offer_age` ago, is still the one showing.
pub fn sign_in_pending(run_waiting: bool, offer_age: Option<std::time::Duration>, notice_is_showing: bool) -> bool {
    run_waiting || (notice_is_showing && offer_age.is_some_and(|age| age < SIGN_IN_OFFER_LIFE))
}

/// Whether to ask "Restore «Name»?".
pub fn should_offer(f: OfferFacts) -> bool {
    f.on && f.in_use && f.moved && !f.busy && !f.standing_by && !f.new_desk_balloon
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn screen(id: &str, rect: [i32; 4], primary: bool) -> DeskScreen {
        DeskScreen { id: id.into(), number: 1, name: id.into(), rect, primary, mm: None }
    }

    fn lined_up(id: &str, mm: [f32; 4], primary: bool) -> DeskScreen {
        DeskScreen { mm: Some(mm), ..screen(id, [0, 0, 1920, 1080], primary) }
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn same_device_path_wins() {
        let from = [screen("a", [0, 0, 1920, 1080], true), screen("b", [1920, 0, 1920, 1080], false)];
        // "b" is the main one now, but it is the same monitor.
        let to = [screen("b", [0, 0, 1920, 1080], true), screen("c", [1920, 0, 1920, 1080], false)];
        assert_eq!(map_screens(&from, &to), Some(map(&[("a", "c"), ("b", "b")])));
    }

    #[test]
    fn main_maps_to_main() {
        let from = [screen("a", [-1920, 0, 1920, 1080], false), screen("b", [0, 0, 1920, 1080], true)];
        let to = [screen("x", [0, 0, 1920, 1080], true), screen("y", [1920, 0, 1920, 1080], false)];
        assert_eq!(map_screens(&from, &to), Some(map(&[("a", "y"), ("b", "x")])));
    }

    #[test]
    fn the_rest_go_left_to_right() {
        let from = [
            screen("c", [3840, 0, 1920, 1080], false),
            screen("a", [0, 0, 1920, 1080], false),
            screen("b", [1920, 0, 1920, 1080], false),
        ];
        let to = [
            screen("z", [2560, 0, 2560, 1440], false),
            screen("x", [0, 0, 2560, 1440], false),
            screen("y", [-2560, 0, 2560, 1440], false),
        ];
        assert_eq!(map_screens(&from, &to), Some(map(&[("a", "y"), ("b", "x"), ("c", "z")])));
    }

    #[test]
    fn stacked_screens_go_top_to_bottom() {
        let from = [screen("low", [0, 1080, 1920, 1080], false), screen("high", [0, 0, 1920, 1080], false)];
        // Nearly the same x, within a tenth of a height.
        let to = [screen("under", [40, 1080, 1920, 1080], false), screen("over", [0, 0, 1920, 1080], false)];
        assert_eq!(map_screens(&from, &to), Some(map(&[("high", "over"), ("low", "under")])));
    }

    #[test]
    fn physical_positions_are_used_when_both_are_lined_up() {
        // Windows has all four at the same place, so only the desk's millimetres can tell them apart.
        let from = [lined_up("a", [0.0, 0.0, 500.0, 300.0], false), lined_up("b", [-600.0, 0.0, 500.0, 300.0], false)];
        let to = [lined_up("x", [0.0, 0.0, 500.0, 300.0], false), lined_up("y", [600.0, 0.0, 500.0, 300.0], false)];
        assert_eq!(map_screens(&from, &to), Some(map(&[("a", "y"), ("b", "x")])));
        // With only one side lined up, Windows' arrangement decides, and these tie on x and y.
        let half = [lined_up("a", [0.0, 0.0, 500.0, 300.0], false), screen("b", [-1920, 0, 1920, 1080], false)];
        let px = [screen("x", [0, 0, 1920, 1080], false), screen("y", [1920, 0, 1920, 1080], false)];
        let by_rect = map_screens(&half, &px).unwrap();
        assert_eq!((by_rect["b"].as_str(), by_rect["a"].as_str()), ("x", "y"));
    }

    #[test]
    fn different_counts_map_nothing() {
        let one = [screen("a", [0, 0, 1920, 1080], true)];
        let two = [screen("a", [0, 0, 1920, 1080], true), screen("b", [1920, 0, 1920, 1080], false)];
        assert_eq!(map_screens(&one, &two), None);
        assert_eq!(map_screens(&[], &[]), None);
    }

    fn sample() -> Vec<Value> {
        vec![
            json!({
                "id": "ws_1", "name": "Dev", "future": { "keep": true },
                "windows": [
                    { "exe": "a.exe", "monitor": "a", "later": 1 },
                    { "exe": "b.exe", "monitor": "gone" },
                ],
                "zones": { "a": { "template": "halves" }, "b": { "template": "thirds" } },
            }),
            json!({ "id": "ws_2", "name": "Chat", "windows": [] }),
            json!({ "not": "a workspace" }),
        ]
    }

    #[test]
    fn copy_rewrites_monitors_and_zone_keys_and_keeps_unknown_keys() {
        let copies = copy_workspaces(&sample(), &map(&[("a", "y"), ("b", "x")]), &[], || "ws_n".into());
        assert_eq!(copies.len(), 2);
        let dev = &copies[0];
        assert_eq!(dev["windows"][0]["monitor"], "y");
        assert_eq!(dev["windows"][0]["later"], 1);
        // A screen the source doesn't list keeps its old id.
        assert_eq!(dev["windows"][1]["monitor"], "gone");
        assert_eq!(dev["zones"]["y"]["template"], "halves");
        assert_eq!(dev["zones"]["x"]["template"], "thirds");
        assert!(dev["zones"].get("a").is_none());
        assert_eq!(dev["future"], json!({ "keep": true }));
    }

    #[test]
    fn copy_gives_new_ids_and_keeps_names() {
        let mut n = 0;
        let copies = copy_workspaces(&sample(), &BTreeMap::new(), &[], || {
            n += 1;
            format!("ws_new{n}")
        });
        assert_eq!(copies.iter().map(|c| c["id"].as_str().unwrap()).collect::<Vec<_>>(), ["ws_new1", "ws_new2"]);
        assert_eq!(copies.iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Dev", "Chat"]);
        let clash = copy_workspaces(&sample(), &BTreeMap::new(), &["dev".to_string()], || "ws_n".into());
        assert_eq!(clash[0]["name"], "Dev 2");
    }

    fn desk(screens: Vec<DeskScreen>, workspaces: Vec<Value>, last_seen: u64) -> crate::config::Desk {
        crate::config::Desk { screens, workspaces, last_seen, ..Default::default() }
    }

    fn two(a: &str, b: &str) -> Vec<DeskScreen> {
        vec![screen(a, [0, 0, 1920, 1080], true), screen(b, [1920, 0, 1920, 1080], false)]
    }

    #[test]
    fn copy_leaves_in_use_default_and_sign_in_behind() {
        let mut cfg = Config::default();
        let mut old = desk(two("a", "b"), sample(), 5);
        old.active_workspace = Some("ws_1".into());
        old.previous_workspace = Some("ws_2".into());
        old.default_workspace = Some("ws_1".into());
        old.sign_in = Some(crate::config::SignIn { mode: crate::config::SignInMode::Workspace, ..Default::default() });
        cfg.desks.insert("old".into(), old);
        cfg.desks.insert("new".into(), desk(two("x", "y"), Vec::new(), 9));
        let ids = copy_between(&mut cfg, "old", "new", |taken| format!("ws_c{}", taken.len())).unwrap();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        let new = &cfg.desks["new"];
        assert_eq!(new.workspaces.len(), 2);
        assert_eq!(new.workspaces[0]["windows"][0]["monitor"], "x");
        assert_eq!((&new.active_workspace, &new.previous_workspace, &new.default_workspace), (&None, &None, &None));
        assert!(new.sign_in.is_none());
        // The source is untouched.
        assert_eq!(cfg.desks["old"].workspaces.len(), 3);
        assert_eq!(cfg.desks["old"].active_workspace.as_deref(), Some("ws_1"));
        // A second copy is refused, and so is a missing source.
        assert_eq!(copy_between(&mut cfg, "old", "new", |_| "z".into()), Err(CopyError::HasWorkspaces));
        assert_eq!(copy_between(&mut cfg, "nope", "new", |_| "z".into()), Err(CopyError::NoSource));
    }

    #[test]
    fn copy_between_refuses_unknown_screens_and_different_counts() {
        let mut cfg = Config::default();
        cfg.desks.insert("old".into(), desk(two("a", "b"), sample(), 5));
        cfg.desks.insert("one".into(), desk(vec![screen("x", [0, 0, 1920, 1080], true)], Vec::new(), 9));
        cfg.desks.insert("blank".into(), desk(Vec::new(), Vec::new(), 9));
        assert_eq!(copy_between(&mut cfg, "old", "one", |_| "z".into()), Err(CopyError::DifferentCounts));
        assert_eq!(copy_between(&mut cfg, "old", "blank", |_| "z".into()), Err(CopyError::ScreensUnknown));
    }

    #[test]
    fn copy_sources_need_an_empty_desk_the_same_count_and_known_screens() {
        let mut cfg = Config::default();
        cfg.desks.insert("here".into(), desk(two("x", "y"), Vec::new(), 9));
        cfg.desks.insert("older".into(), desk(two("a", "b"), sample(), 3));
        cfg.desks.insert("newer".into(), desk(two("c", "d"), sample(), 7));
        cfg.desks.insert("no-screens".into(), desk(Vec::new(), sample(), 8));
        cfg.desks.insert("one-screen".into(), desk(vec![screen("e", [0, 0, 1, 1], true)], sample(), 8));
        cfg.desks.insert("no-workspaces".into(), desk(two("f", "g"), Vec::new(), 8));
        assert_eq!(copy_sources(&cfg, "here", 2), ["newer", "older"]);
        // A desk that has workspaces is offered nothing, and neither is one with no screens.
        assert!(copy_sources(&cfg, "older", 2).is_empty());
        assert!(copy_sources(&cfg, "here", 0).is_empty());
        // The connected desk isn't in the config yet.
        assert_eq!(copy_sources(&cfg, "brand-new", 2), ["newer", "older"]);
    }

    #[test]
    fn an_ignored_sign_in_offer_stops_holding_the_restore_offer_back() {
        // Waiting or running holds it, and so does a notice that is still the one showing.
        use std::time::Duration;
        let fresh = Some(Duration::from_secs(2));
        assert!(sign_in_pending(true, None, false));
        assert!(sign_in_pending(false, fresh, true));
        // The notice was replaced, or never made: nothing holds it.
        assert!(!sign_in_pending(false, fresh, false));
        assert!(!sign_in_pending(false, None, true));
        // Ignored for longer than a balloon lives, the shell's word or not.
        assert!(sign_in_pending(false, Some(SIGN_IN_OFFER_LIFE - Duration::from_millis(1)), true));
        assert!(!sign_in_pending(false, Some(SIGN_IN_OFFER_LIFE), true));
        assert!(!sign_in_pending(false, Some(Duration::from_secs(3600)), true));
    }

    #[test]
    fn should_offer_needs_everything_in_its_place() {
        let go = OfferFacts { on: true, in_use: true, moved: true, ..OfferFacts::default() };
        assert!(should_offer(go));
        assert!(!should_offer(OfferFacts { on: false, ..go }));
        assert!(!should_offer(OfferFacts { in_use: false, ..go }));
        assert!(!should_offer(OfferFacts { moved: false, ..go }));
        assert!(!should_offer(OfferFacts { busy: true, ..go }));
        assert!(!should_offer(OfferFacts { standing_by: true, ..go }));
        assert!(!should_offer(OfferFacts { new_desk_balloon: true, ..go }));
    }
}
