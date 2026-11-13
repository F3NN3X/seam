//! The library of custom layouts: named split trees shared by every desk and screen.
//! A screen's layout picked from it records the id as `custom`, and follows the
//! library layout when it's edited, until the screen's own tree changes. Raw JSON
//! in the config like app rules, so an entry this build can't read is kept and ignored.
//! An entry's `screens` lists the device paths it's offered on. None means any screen.

use crate::config::Config;
use crate::zones::{self, Node, Template, ZoneLayout};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub tree: Node,
    /// The screens it's for, by device path. Empty for any screen.
    pub screens: Vec<String>,
}

impl Entry {
    /// As stored in `custom_layouts`.
    pub fn value(&self) -> Value {
        let mut value = json!({ "id": self.id, "name": self.name, "tree": self.tree });
        if !self.screens.is_empty() {
            value["screens"] = json!(self.screens);
        }
        value
    }

    /// Whether it's offered on the screen with this device path. Paths are compared
    /// ignoring case, as Windows may report them in either.
    pub fn fits(&self, device: &str) -> bool {
        self.screens.is_empty() || self.screens.iter().any(|s| s.eq_ignore_ascii_case(device))
    }

    /// A screen's layout that follows this one.
    pub fn layout(&self) -> ZoneLayout {
        ZoneLayout { template: Template::Custom, custom: Some(self.id.clone()), tree: self.tree.clone() }
    }
}

fn id_of(saved: &Value) -> Option<&str> {
    saved.get("id").and_then(Value::as_str)
}

/// An entry Seam can read, with its ratios clamped as a screen's are. One past
/// the zone limits is never read, so it can't reach a screen through `follow`.
pub fn entry(saved: &Value) -> Option<Entry> {
    let id = id_of(saved)?.to_string();
    let name = saved.get("name")?.as_str()?.to_string();
    let layout = zones::layout_of(&json!({ "template": "custom", "tree": saved.get("tree")? }))
        .filter(|l| zones::within_limits(&l.tree))?;
    let screens = saved
        .get("screens")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |list| list.iter().filter_map(Value::as_str).map(String::from).collect());
    Some(Entry { id, name, tree: layout.tree, screens })
}

pub fn entries(saved: &[Value]) -> Vec<Entry> {
    saved.iter().filter_map(entry).collect()
}

/// The entries offered on the screen with this device path: those for any screen,
/// and those tied to it.
pub fn for_screen<'a>(entries: &'a [Entry], device: &'a str) -> impl Iterator<Item = &'a Entry> {
    entries.iter().filter(move |e| e.fits(device))
}

/// A layout the API can name: `zones.layouts` lists them, `zones.apply` takes one.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub id: String,
    pub name: String,
    /// "template", "custom" or "none".
    pub kind: &'static str,
    /// None is no zones.
    pub layout: Option<ZoneLayout>,
}

/// The id a screen's layout goes by in the API: its library id, else its template's
/// name, "none" for no zones. None for a custom tree that isn't in the library.
pub fn layout_id(layout: Option<&ZoneLayout>) -> Option<String> {
    let Some(layout) = layout else { return Some("none".into()) };
    if let Some(id) = &layout.custom {
        return Some(id.clone());
    }
    match layout.template {
        Template::Custom => None,
        template => json!(template).as_str().map(String::from),
    }
}

/// What a screen with this device path can be given: the built-in templates, the
/// library layouts offered on it (and the one it uses, wherever that's tied), then none.
pub fn offers(saved: &[Value], device: &str, now: Option<&ZoneLayout>) -> Vec<Offer> {
    let in_use = now.and_then(|l| l.custom.as_deref());
    let templates = Template::ALL.into_iter().filter_map(ZoneLayout::of).map(|layout| Offer {
        id: layout_id(Some(&layout)).unwrap_or_default(),
        name: layout.template.title().to_string(),
        kind: "template",
        layout: Some(layout),
    });
    let custom = entries(saved).into_iter().filter(|e| e.fits(device) || in_use == Some(e.id.as_str())).map(|e| {
        let layout = Some(e.layout());
        Offer { id: e.id, name: e.name, kind: "custom", layout }
    });
    let none = Offer { id: "none".into(), name: Template::None.title().to_string(), kind: "none", layout: None };
    templates.chain(custom).chain([none]).collect()
}

/// The offer with this id, else the one with this name, ignoring case. A library
/// layout wins a name over a built-in one, so a custom "Grid" is the one asked for.
pub fn find_offer<'a>(offers: &'a [Offer], wanted: &str) -> Option<&'a Offer> {
    let same = |a: &str| a.to_lowercase() == wanted.to_lowercase();
    offers.iter().find(|o| same(&o.id)).or_else(|| {
        let named = |kind| offers.iter().find(|o| o.kind == kind && same(&o.name));
        named("custom").or_else(|| offers.iter().find(|o| same(&o.name)))
    })
}

pub fn find(saved: &[Value], id: &str) -> Option<Entry> {
    saved.iter().filter(|v| id_of(v) == Some(id)).find_map(entry)
}

/// "Custom 1", "Custom 2"...: the lowest number no entry's name has.
pub fn next_name(saved: &[Value]) -> String {
    let taken: Vec<&str> = saved.iter().filter_map(|v| v.get("name").and_then(Value::as_str)).collect();
    (1..).map(|n| format!("Custom {n}")).find(|name| !taken.contains(&name.as_str())).unwrap_or_default()
}

/// A short id no entry has: the time in milliseconds, in hex.
pub fn new_id(saved: &[Value]) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    (now..).map(|n| format!("{n:x}")).find(|id| !saved.iter().any(|v| id_of(v) == Some(id))).unwrap_or_default()
}

/// The library changed from `before` to what `config` has now. Every screen, on
/// every desk and in every workspace, that follows a library layout whose tree
/// changed takes the new tree, and one whose library layout was deleted keeps its
/// tree and drops the link.
pub fn follow(before: &[Value], config: &mut Config) {
    if before == config.custom_layouts.as_slice() {
        return;
    }
    let now = &config.custom_layouts;
    let gone: Vec<String> = before
        .iter()
        .filter_map(id_of)
        .filter(|id| !now.iter().any(|v| id_of(v) == Some(id)))
        .map(String::from)
        .collect();
    let changed: Vec<Entry> =
        entries(now).into_iter().filter(|e| find(before, &e.id).is_none_or(|was| was.tree != e.tree)).collect();
    fn workspace_zones(ws: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
        ws.get_mut("zones").and_then(Value::as_object_mut)
    }
    let screens = config.desks.values_mut().flat_map(|desk| {
        let in_workspaces = desk.workspaces.iter_mut().filter_map(workspace_zones).flat_map(|z| z.values_mut());
        desk.zones.values_mut().chain(in_workspaces)
    });
    for saved in screens {
        let Some(id) = saved.get("custom").and_then(Value::as_str).map(String::from) else { continue };
        if gone.contains(&id) {
            if let Some(fields) = saved.as_object_mut() {
                fields.remove("custom");
            }
        } else if let Some(entry) = changed.iter().find(|e| e.id == id) {
            saved["tree"] = json!(entry.tree);
        }
    }
}

/// A layout the tray is about to save for a screen. A `fresh` one, from Save from
/// windows on the screen with that device path, adds its tree to the library under
/// its id as "Custom N", for that screen. Any other
/// keeps its link only while the library still has that id with the same tree,
/// so Priority's second press after the library layout was edited or deleted
/// gives the screen its tree back on its own.
pub fn link(saved: &mut Vec<Value>, layout: &mut ZoneLayout, fresh: Option<&str>) {
    let Some(id) = layout.custom.clone() else { return };
    let known = saved.iter().any(|v| id_of(v) == Some(id.as_str()));
    if let Some(screen) = fresh.filter(|_| !known) {
        let name = next_name(saved);
        saved.push(Entry { id, name, tree: layout.tree.clone(), screens: vec![screen.to_string()] }.value());
    } else if find(saved, &id).is_none_or(|entry| entry.tree != layout.tree) {
        layout.custom = None;
    }
}

/// A screen's layout as a merge patch value. A layout with no link says so with
/// `"custom": null`, or the link it replaces would stay.
pub fn screen_patch(layout: &ZoneLayout) -> Option<Value> {
    let mut value = serde_json::to_value(layout).ok()?;
    if layout.custom.is_none() {
        value["custom"] = Value::Null;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A patch is a merge, so a screen given a template has to lose its link to a library layout.
    #[test]
    fn a_linked_screen_given_a_template_follows_no_library_layout() {
        let tree = zones::template_tree(Template::Rows).unwrap();
        let entry = Entry { id: "a1".into(), name: "Custom 1".into(), tree, screens: Vec::new() };
        let linked = serde_json::to_value(entry.layout()).unwrap();
        let config: Config = serde_json::from_value(json!({
            "custom_layouts": [entry.value()],
            "desks": { "S": { "name": "Desk", "zones": { "M": linked } } },
        }))
        .unwrap();
        let columns = screen_patch(&ZoneLayout::of(Template::Columns).unwrap()).unwrap();
        let config = config.patched(&json!({ "desks": { "S": { "zones": { "M": columns } } } })).unwrap();
        let now = zones::layout_of(&config.desks["S"].zones["M"]).unwrap();
        assert_eq!(now, ZoneLayout::of(Template::Columns).unwrap());
        // A later edit of the library layout leaves the screen alone.
        let mut edited = entry.value();
        edited["tree"] = json!("leaf");
        let config = config.patched(&json!({ "custom_layouts": [edited] })).unwrap();
        assert_eq!(zones::layout_of(&config.desks["S"].zones["M"]).unwrap(), now);
    }

    #[test]
    fn a_layout_is_found_by_id_or_name_ignoring_case() {
        let tree = zones::template_tree(Template::Rows).unwrap();
        let mine = Entry { id: "a1".into(), name: "Main + stack".into(), tree, screens: vec!["M".into()] };
        let offers = offers(&[mine.value()], "m", None);
        let kind = |wanted| find_offer(&offers, wanted).map(|o| o.kind);
        assert_eq!(kind("GRID"), Some("template"), "an id beats a name");
        assert_eq!(kind("A1"), Some("custom"));
        assert_eq!(kind("main + STACK"), Some("custom"), "a library layout wins a name");
        assert_eq!(kind("mainstack"), Some("template"));
        assert_eq!(kind("none"), Some("none"));
        assert_eq!(kind("nope"), None);
        assert_eq!(layout_id(Some(&ZoneLayout::of(Template::MainStack).unwrap())).as_deref(), Some("mainstack"));
    }
}
