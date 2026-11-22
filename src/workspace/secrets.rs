//! Where startup commands may and may not go (D736, D737): out of exports, imports and
//! what another client reads, sealed before anything is saved, and in clear only to the
//! panel. The sealing itself is the caller's, so this stays pure.
//!
//! A clear `command` is only ever what the panel was handed or typed, on its way to be sealed.
//! Stored data never has one (`without_clear` takes it off on load), and a clear command that
//! reaches `seal_patch` is sealed only if it's one the panel could have been given.

use super::*;

/// The sealing and unsealing the tray does with `CryptProtectData`.
pub type Seal<'a> = &'a dyn Fn(&str) -> Option<String>;

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
    without_keys(saved, &["command_sealed", "command", "unreadable"])
}

/// The saved workspaces without the clear command and the flag, which are never stored. A
/// file that has them, written by hand or by another program, never gets them sealed or shown.
pub fn without_clear(saved: &[Value]) -> Vec<Value> {
    without_keys(saved, &["command", "unreadable"])
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
            match unseal(&sealed) {
                Some(command) => {
                    tab.insert("command".into(), Value::String(command));
                }
                None => {
                    tab.insert("unreadable".into(), Value::Bool(true));
                }
            }
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

/// Every command these saved workspaces hold, opened: the ones the panel was handed.
pub fn unsealed_in(saved: &[Value], unseal: Seal) -> HashSet<String> {
    sealed_in(saved).iter().filter_map(|s| unseal(s)).collect()
}

/// Every tab of every Terminal item these saved workspaces hold.
pub fn tabs_in(saved: &[Value]) -> Vec<Tab> {
    let mut saved = saved.to_vec();
    let mut tabs = Vec::new();
    for row in rows(&mut saved) {
        if let Some(Item::Terminal { tabs: found }) =
            row.get("item").and_then(|i| serde_json::from_value(i.clone()).ok())
        {
            tabs.extend(found);
        }
    }
    tabs
}

/// A tab as it may be saved. A command in clear is the one to keep, and a sealed one is kept
/// only when it's one the workspace already held, so a client can't plant a value from elsewhere.
/// A tab the panel marked `unreadable` and sent back with no command gets back the sealed value
/// of the stored tab with the same profile and folder: saving never destroys another PC's command.
pub(super) fn settle(item: &mut Item, known: &dyn Fn(&str) -> bool, stored: &[Tab]) {
    if let Item::Terminal { tabs } = item {
        for tab in tabs {
            if tab.unreadable && tab.command.is_none() && tab.command_sealed.is_none() {
                let same = |s: &&Tab| s.profile == tab.profile && s.folder == tab.folder;
                tab.command_sealed = stored.iter().filter(same).find_map(|s| s.command_sealed.clone());
            }
            tab.unreadable = false;
            if tab.command.is_some() || !tab.command_sealed.as_deref().is_some_and(known) {
                tab.command_sealed = None;
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

/// What a patch's items may be sealed against: the sealed commands and the opened ones the
/// saved config holds, and its tabs.
pub struct Held<'a> {
    pub sealed: &'a HashSet<String>,
    pub clear: &'a HashSet<String>,
    pub stored: &'a [Tab],
}

/// A patch's workspaces as they may be saved: each item read, checked as `check_item` says and
/// settled, its commands sealed, and `open_with` and `place` taken off the row (D732). A clear
/// command is sealed only if it's one the saved config holds, opened, which is what the panel was
/// handed; one anywhere else is dropped, and a new command only arrives through `workspace.update`.
/// An item that fails is dropped, and the row starts its app with nothing. Fails when a command
/// can't be sealed, so nothing is saved.
pub fn seal_patch(saved: &mut [Value], held: &Held, seal: Seal) -> Result<(), &'static str> {
    for (n, ws) in saved.iter_mut().enumerate() {
        let name = ws.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(list) = ws.get_mut("windows").and_then(Value::as_array_mut) else { continue };
        for (row_n, row) in list.iter_mut().enumerate() {
            let Some(sent) = row.get("item").cloned() else { continue };
            let exe = row.get("exe").and_then(Value::as_str).unwrap_or_default().to_string();
            let aumid = row.get("aumid").and_then(Value::as_str).map(str::to_string);
            let checked = serde_json::from_value::<Item>(sent).ok().and_then(|mut item| {
                if let Item::Terminal { tabs } = &mut item {
                    tabs.iter_mut().for_each(|t| t.command = t.command.take().filter(|c| held.clear.contains(c)));
                }
                check_item(&item, &exe, aumid.as_deref()).ok()
            });
            let sealed = checked.map(|mut item| {
                settle(&mut item, &|s| held.sealed.contains(s), held.stored);
                if let Item::Terminal { tabs } = &mut item {
                    for tab in tabs {
                        if let Some(command) = tab.command.take() {
                            tab.command_sealed =
                                Some(seal(&command).ok_or("Seam couldn't keep the command on this PC")?);
                        }
                    }
                }
                Ok::<_, &'static str>(item)
            });
            let item = sealed.transpose()?;
            let Some(fields) = row.as_object_mut() else { continue };
            match item.and_then(|item| serde_json::to_value(item).ok()) {
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
    Ok(())
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
