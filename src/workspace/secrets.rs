//! Where startup commands may and may not go (D736, D737): out of exports, imports and
//! what another client reads, sealed before anything is saved, and in clear only to the
//! panel. The sealing itself is the caller's, so this stays pure.

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

/// The saved workspaces as stored, without any startup command, sealed or in clear: they stay
/// on this PC, so both exports leave them out.
pub fn without_commands(saved: &[Value]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for row in rows(&mut saved) {
        for tab in tabs_of(row).filter_map(Value::as_object_mut) {
            for key in ["command_sealed", "command", "unreadable"] {
                tab.remove(key);
            }
        }
    }
    saved
}

/// The saved workspaces with each item reduced to its kind, for a client that isn't the panel.
pub fn kind_only(saved: &[Value]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for row in rows(&mut saved) {
        let kind = row.pointer("/item/kind").cloned();
        if let (Some(kind), Some(fields)) = (kind, row.as_object_mut()) {
            fields.insert("item".into(), json!({ "kind": kind }));
        }
    }
    saved
}

/// The saved workspaces for the panel: each sealed command opened and in clear as `command`, or,
/// when it won't open here, marked `unreadable`.
pub fn for_panel(saved: &[Value], unseal: Seal) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for row in rows(&mut saved) {
        for tab in tabs_of(row).filter_map(Value::as_object_mut) {
            let Some(sealed) = tab.get("command_sealed").and_then(Value::as_str).map(str::to_string) else { continue };
            match unseal(&sealed) {
                Some(command) => {
                    tab.remove("command_sealed");
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

/// A tab as it may be saved: a command in clear is the one to keep, and a sealed one is kept
/// only when it's one the workspace already held, so a client can't plant a value from elsewhere.
pub(super) fn settle(item: &mut Item, known: &dyn Fn(&str) -> bool) {
    if let Item::Terminal { tabs } = item {
        for tab in tabs {
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

/// A patch's workspaces as they may be saved: each item read, checked as `check_item` says and
/// settled, its commands sealed, and `open_with` and `place` taken off the row (D732). An item
/// that fails is dropped, and the row starts its app with nothing.
pub fn seal_patch(saved: &mut [Value], known: &HashSet<String>, seal: Seal) {
    for row in rows(saved) {
        let Some(sent) = row.get("item").cloned() else { continue };
        let exe = row.get("exe").and_then(Value::as_str).unwrap_or_default().to_string();
        let aumid = row.get("aumid").and_then(Value::as_str).map(str::to_string);
        let checked = serde_json::from_value::<Item>(sent)
            .ok()
            .and_then(|item| check_item(&item, &exe, aumid.as_deref()).ok())
            .and_then(|mut item| {
                settle(&mut item, &|s| known.contains(s));
                if let Item::Terminal { tabs } = &mut item {
                    for tab in tabs {
                        if let Some(command) = tab.command.take() {
                            tab.command_sealed = Some(seal(&command)?);
                        }
                    }
                }
                Some(item)
            });
        let Some(fields) = row.as_object_mut() else { continue };
        match checked.and_then(|item| serde_json::to_value(item).ok()) {
            Some(item) => {
                fields.insert("item".into(), item);
                fields.remove("open_with");
                fields.remove("place");
            }
            None => {
                fields.remove("item");
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
