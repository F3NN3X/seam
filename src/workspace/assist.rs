//! What an assistant may change in a workspace (D534): its name, whether it shows
//! every window, and a row's place or how it is shown, or turning a row's launch off.
//! Never a program, an AUMID, a hotkey or other windows, and never a launch on.
//! The tray saves the result through the editor's own check, `checked_update`.

use super::*;

/// The longest workspace name an assistant may give.
pub const MAX_NAME: usize = 64;

/// What `workspace.edit_row` changes. `launch_off` only ever turns a row's launch off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowChange {
    pub target: Option<Target>,
    pub shown: Option<Shown>,
    pub launch_off: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Rename(String),
    Row { row: usize, change: RowChange },
    RemoveRow(usize),
    ShowAll(bool),
}

/// A name as an assistant gave it: trimmed, not empty, and short.
pub fn checked_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("name must be 1 to {MAX_NAME} characters"));
    }
    Ok(name.to_string())
}

/// A rect target is fractions of the work area, inside it and not empty.
fn rect_ok(rect: &[f32; 4]) -> bool {
    let [x, y, w, h] = *rect;
    rect.iter().all(|v| v.is_finite()) && x >= 0. && y >= 0. && w > 0. && h > 0. && x + w <= 1.001 && y + h <= 1.001
}

/// `old` with the edit made. Zones and the other checks are `checked_update`'s.
pub fn edited(old: &Workspace, edit: &Edit) -> Result<Workspace, String> {
    let mut ws = old.clone();
    match edit {
        Edit::Rename(name) => ws.name = checked_name(name)?,
        Edit::ShowAll(on) => ws.show_all = *on,
        Edit::RemoveRow(row) => {
            if *row >= ws.windows.len() {
                return Err("no such row in that workspace".into());
            }
            ws.windows.remove(*row);
        }
        Edit::Row { row, change } => {
            let Some(rule) = ws.windows.get_mut(*row) else { return Err("no such row in that workspace".into()) };
            if let Some(target) = &change.target {
                if matches!(target, Target::Rect { rect } if !rect_ok(rect)) {
                    return Err("that rect isn't inside the screen".into());
                }
                rule.set_target(target.clone());
            }
            if let Some(shown) = change.shown {
                rule.set_shown(shown);
            }
            if change.launch_off {
                rule.launch = false;
            }
        }
    }
    Ok(ws)
}

/// For each row left after removing `row` of `len` rows, the row it was.
pub fn survivors(len: usize, row: usize) -> Vec<Option<usize>> {
    (0..len.saturating_sub(1)).map(|i| Some(if i < row { i } else { i + 1 })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(exe: &str) -> Rule {
        Rule { exe: exe.into(), monitor: "m".into(), launch: true, ..Rule::default() }
    }

    fn two() -> Workspace {
        Workspace {
            id: "ws_1".into(),
            name: "Coding".into(),
            windows: vec![rule("a.exe"), rule("b.exe")],
            ..Default::default()
        }
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(checked_name("  Review "), Ok("Review".into()));
        assert!(checked_name("   ").is_err());
        assert!(checked_name(&"x".repeat(MAX_NAME + 1)).is_err());
        assert!(checked_name("a\nb").is_err());
    }

    #[test]
    fn a_row_edit_never_touches_what_starts_the_app() {
        let change = RowChange {
            target: Some(Target::Zone { zone: 2, span_to: None }),
            shown: Some(Shown::Minimized),
            launch_off: true,
        };
        let ws = edited(&two(), &Edit::Row { row: 1, change }).unwrap();
        assert_eq!(ws.windows[1].target, Target::Zone { zone: 2, span_to: None });
        assert_eq!(ws.windows[1].shown, Shown::Minimized);
        assert!(!ws.windows[1].launch);
        assert_eq!((ws.windows[1].exe.as_str(), &ws.windows[1].aumid), ("b.exe", &None));
        assert_eq!(ws.windows[0], two().windows[0]);
        assert_eq!((ws.hotkey, ws.other_windows), (None, OtherWindows::Leave));
    }

    #[test]
    fn launch_is_left_alone_unless_turned_off() {
        let change = RowChange { shown: Some(Shown::Maximized), ..RowChange::default() };
        let ws = edited(&two(), &Edit::Row { row: 0, change }).unwrap();
        assert!(ws.windows[0].launch);
        assert_eq!(ws.windows[0].target, Target::State { state: WinState::Max });
    }

    #[test]
    fn bad_rows_and_rects_are_refused() {
        let rect = |rect| RowChange { target: Some(Target::Rect { rect }), ..RowChange::default() };
        assert!(edited(&two(), &Edit::Row { row: 2, change: RowChange::default() }).is_err());
        assert!(edited(&two(), &Edit::RemoveRow(2)).is_err());
        assert!(edited(&two(), &Edit::Row { row: 0, change: rect([0.5, 0., 0.8, 1.]) }).is_err());
        assert!(edited(&two(), &Edit::Row { row: 0, change: rect([0., 0., f32::NAN, 1.]) }).is_err());
        assert!(edited(&two(), &Edit::Row { row: 0, change: rect([0., 0., 0.5, 1.]) }).is_ok());
    }

    #[test]
    fn rename_remove_and_show_all() {
        assert_eq!(edited(&two(), &Edit::Rename(" Mail ".into())).unwrap().name, "Mail");
        assert!(edited(&two(), &Edit::Rename(" ".into())).is_err());
        assert!(edited(&two(), &Edit::ShowAll(true)).unwrap().show_all);
        let left = edited(&two(), &Edit::RemoveRow(0)).unwrap();
        assert_eq!(left.windows.len(), 1);
        assert_eq!(left.windows[0].exe, "b.exe");
    }

    #[test]
    fn survivors_say_which_row_each_one_was() {
        assert_eq!(survivors(3, 0), vec![Some(1), Some(2)]);
        assert_eq!(survivors(3, 1), vec![Some(0), Some(2)]);
        assert_eq!(survivors(3, 2), vec![Some(0), Some(1)]);
        assert!(survivors(1, 0).is_empty());
    }
}
