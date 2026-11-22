//! Where startup commands may and may not go (D736, D737): out of exports, imports and
//! what another client reads, sealed before anything is saved, and in clear only to the
//! panel. The sealing itself is the caller's, so this stays pure.
//!
//! A clear `command` is only ever what the panel was handed or typed, on its way to be sealed.
//! Stored data never has one (`without_clear` takes it off on load), and a clear command that
//! reaches `seal_patch` is sealed only if it's one the panel could have been given.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// The sealing and unsealing the tray does with `CryptProtectData`.
pub type Seal<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The seals this tray has read from its own config, for as long as it runs: reference to the
/// sealed command and, when it opened, the command in clear. `for_panel` hands the panel a
/// reference for each tab, and a save that sends it back keeps that tab's seal without sealing
/// again, so nothing is matched by value, and a reference the tray never made resolves to nothing.
#[derive(Default)]
struct Seals {
    next: u64,
    by_ref: HashMap<String, (String, Option<String>)>,
    by_seal: HashMap<String, String>,
}

static SEALS: Mutex<Option<Seals>> = Mutex::new(None);

fn register(sealed: &str, clear: Option<String>) -> String {
    let mut guard = SEALS.lock().unwrap_or_else(|e| e.into_inner());
    let seals = guard.get_or_insert_with(Seals::default);
    let known = seals.by_seal.get(sealed).cloned();
    let reference = known.unwrap_or_else(|| {
        seals.next += 1;
        // Not derived from the seal, and not a number to count up to.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(seals.next);
        format!("{:016x}", hasher.finish())
    });
    seals.by_seal.insert(sealed.to_string(), reference.clone());
    seals.by_ref.insert(reference.clone(), (sealed.to_string(), clear));
    reference
}

fn lookup(reference: &str) -> Option<(String, Option<String>)> {
    SEALS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.by_ref.get(reference).cloned()
}

/// Each row of each saved workspace, as stored.
fn rows(saved: &mut [Value]) -> impl Iterator<Item = &mut Value> {
    saved.iter_mut().filter_map(|ws| ws.get_mut("windows")?.as_array_mut()).flatten()
}

fn tabs_of(row: &mut Value) -> impl Iterator<Item = &mut Value> {
    let terminal = row.pointer("/item/kind").and_then(Value::as_str) == Some("terminal");
    let tabs = row.pointer_mut("/item/tabs").and_then(Value::as_array_mut).filter(|_| terminal);
    tabs.into_iter().flatten()
}

fn without_keys(saved: &[Value], keys: &[&str]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for row in rows(&mut saved) {
        for tab in tabs_of(row).filter_map(Value::as_object_mut) {
            for key in keys {
                tab.remove(*key);
            }
        }
    }
    saved
}

/// The saved workspaces as stored, without any startup command, sealed or in clear: they stay
/// on this PC, so both exports leave them out.
pub fn without_commands(saved: &[Value]) -> Vec<Value> {
    without_keys(saved, &["command_sealed", "command", "unreadable", "seal_ref"])
}

/// The saved workspaces without the clear command and the flag, which are never stored. A
/// file that has them, written by hand or by another program, never gets them sealed or shown.
pub fn without_clear(saved: &[Value]) -> Vec<Value> {
    without_keys(saved, &["command", "unreadable", "seal_ref"])
}

/// The saved workspaces without any item: the last session is only what a capture read.
pub fn without_items(saved: &[Value]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for row in rows(&mut saved) {
        if let Some(fields) = row.as_object_mut() {
            fields.remove("item");
        }
    }
    saved
}

/// The saved workspaces with each item reduced to its kind, for a client that isn't the panel.
/// An item whose kind isn't text is dropped.
pub fn kind_only(saved: &[Value]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for row in rows(&mut saved) {
        let kind = row.pointer("/item/kind").filter(|k| k.is_string()).cloned();
        let Some(fields) = row.as_object_mut() else { continue };
        match kind {
            Some(kind) => {
                fields.insert("item".into(), json!({ "kind": kind }));
            }
            None => {
                fields.remove("item");
            }
        }
    }
    saved
}

/// The saved workspaces for the panel: each sealed command opened and in clear as `command`, or,
/// when it won't open here, marked `unreadable`. The panel never gets a sealed value (D737).
pub fn for_panel(saved: &[Value], unseal: Seal) -> Vec<Value> {
    let mut saved = without_clear(saved);
    for row in rows(&mut saved) {
        for tab in tabs_of(row).filter_map(Value::as_object_mut) {
            let Some(sealed) = tab.remove("command_sealed").and_then(|s| s.as_str().map(str::to_string)) else {
                continue;
            };
            let clear = unseal(&sealed);
            tab.insert("seal_ref".into(), Value::String(register(&sealed, clear.clone())));
            match clear {
                Some(command) => tab.insert("command".into(), Value::String(command)),
                None => tab.insert("unreadable".into(), Value::Bool(true)),
            };
        }
    }
    saved
}

/// Every sealed command these saved workspaces hold.
pub fn sealed_in(saved: &[Value]) -> HashSet<String> {
    let mut saved = saved.to_vec();
    let sealed = |tab: &Value| tab.get("command_sealed").and_then(Value::as_str).map(str::to_string);
    let mut found = HashSet::new();
    for row in rows(&mut saved) {
        found.extend(tabs_of(row).filter_map(|t| sealed(t)));
    }
    found
}

/// A tab as it may be saved. The reference the panel was given for the tab keeps its seal when
/// the tab's command is what the panel was shown (none for an unreadable one), so another PC's
/// command survives a save. A different command stays in clear to be sealed, and a reference the
/// tray didn't make drops the tab's command. With no reference, a command in clear is new, and a
/// sealed one is kept only when it's one the workspace already held (`held`), which is how a
/// save the tray makes itself carries them.
pub(super) fn settle(item: &mut Item, held: &dyn Fn(&str) -> bool) {
    let Item::Terminal { tabs } = item else { return };
    for tab in tabs {
        tab.unreadable = false;
        match tab.seal_ref.take().map(|r| lookup(&r)) {
            Some(Some((sealed, clear))) if clear == tab.command => {
                tab.command_sealed = Some(sealed);
                tab.command = None;
            }
            Some(Some(_)) => tab.command_sealed = None,
            Some(None) => (tab.command_sealed, tab.command) = (None, None),
            None => {
                if tab.command.is_some() || !tab.command_sealed.as_deref().is_some_and(held) {
                    tab.command_sealed = None;
                }
            }
        }
    }
}

/// Seal each command in clear of a checked workspace, which is then the only form saved.
pub fn seal_commands(ws: &mut Workspace, seal: Seal) -> Result<(), &'static str> {
    for rule in &mut ws.windows {
        let Some(Item::Terminal { tabs }) = &mut rule.item else { continue };
        for tab in tabs {
            if let Some(command) = tab.command.take() {
                tab.command_sealed = Some(seal(&command).ok_or("Seam couldn't keep the command on this PC")?);
            }
        }
    }
    Ok(())
}

/// A workspace with no clear command, for a save that isn't the panel's own edit of an item.
pub fn without_clear_commands(ws: &mut Workspace) {
    for rule in &mut ws.windows {
        if let Some(Item::Terminal { tabs }) = &mut rule.item {
            tabs.iter_mut().for_each(|t| t.command = None);
        }
    }
}

/// A patch's workspaces as they may be saved: each item read, checked as `check_item` says and
/// settled, and `open_with` and `place` taken off the row (D732). A tab keeps its seal only through
/// the reference the panel was given for it, and a command in clear is never sealed here: a new
/// or changed one arrives through `workspace.update` alone. An item that fails is dropped, and the
/// row starts its app with nothing.
pub fn seal_patch(saved: &mut [Value]) {
    for (n, ws) in saved.iter_mut().enumerate() {
        let name = ws.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(list) = ws.get_mut("windows").and_then(Value::as_array_mut) else { continue };
        for (row_n, row) in list.iter_mut().enumerate() {
            let Some(sent) = row.get("item").cloned() else { continue };
            let exe = row.get("exe").and_then(Value::as_str).unwrap_or_default().to_string();
            let aumid = row.get("aumid").and_then(Value::as_str).map(str::to_string);
            let checked = serde_json::from_value::<Item>(sent)
                .ok()
                .and_then(|item| check_item(&item, &exe, aumid.as_deref()).ok())
                .map(|mut item| {
                    settle(&mut item, &|_| false);
                    if let Item::Terminal { tabs } = &mut item {
                        tabs.iter_mut().for_each(|t| t.command = None);
                    }
                    item
                });
            let Some(fields) = row.as_object_mut() else { continue };
            match checked.and_then(|item| serde_json::to_value(item).ok()) {
                Some(item) => {
                    fields.insert("item".into(), item);
                    fields.remove("open_with");
                    fields.remove("place");
                }
                None => {
                    crate::log::info!(
                        "workspace {} ({name}), row {row_n}: an item that fails its check is dropped",
                        n + 1
                    );
                    fields.remove("item");
                }
            }
        }
    }
}

/// An imported file's workspaces: no startup command, and an item that doesn't pass its check
/// is dropped (D737). The rest is kept.
pub fn clean_imported_items(saved: &[Value]) -> Vec<Value> {
    let mut saved = without_commands(saved);
    for row in rows(&mut saved) {
        let Some(sent) = row.get("item").cloned() else { continue };
        let exe = row.get("exe").and_then(Value::as_str).unwrap_or_default().to_string();
        let aumid = row.get("aumid").and_then(Value::as_str).map(str::to_string);
        let checked =
            serde_json::from_value::<Item>(sent).ok().and_then(|i| check_item(&i, &exe, aumid.as_deref()).ok());
        let Some(fields) = row.as_object_mut() else { continue };
        match checked.and_then(|item| serde_json::to_value(item).ok()) {
            Some(item) => {
                fields.insert("item".into(), item);
            }
            None => {
                fields.remove("item");
            }
        }
    }
    saved
}
