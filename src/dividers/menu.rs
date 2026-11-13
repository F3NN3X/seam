//! The divider menus: a handle's right-click menu, and the size menu key's at the
//! focused window's centre. A choice goes to the tray's main thread.

use super::*;

/// A menu line and what it does, or None for a separator.
type Item = Option<(String, Choice)>;
/// The size menu key's window, its zones, and the zones then.
type Asked = (isize, (String, usize, usize), Scene);

/// The size menu key's ask, waiting for the thread.
static ASKED: Mutex<Option<Asked>> = Mutex::new(None);
static CHOICE: Mutex<Option<Choice>> = Mutex::new(None);

/// Open the size menu for `hwnd`, in zones `zone`, at its centre.
pub fn menu(hwnd: isize, zone: (String, usize, usize), scene: Scene) {
    *ASKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((hwnd, zone, scene));
    post(true, WM_APP_MENU);
}

/// The menu choice waiting for the main thread, once.
pub fn take_choice() -> Option<Choice> {
    CHOICE.lock().unwrap_or_else(|e| e.into_inner()).take()
}

impl State {
    /// A handle's menu: shares of the window before the divider, and Take out for each.
    fn handle_items(&self, hwnd: HWND) -> Option<(Vec<Item>, isize)> {
        let h = self.handles.get(self.handle(hwnd).filter(|_| self.drag.is_none())?)?;
        let scene = self.scene.as_ref()?;
        let (a, b) = (h.spot.a.0, h.spot.b.0);
        let (axis, side, sides) = match h.spot.dir {
            Dir::Vertical => (Axis::Width, Side::Right, ("Left", "right", "left")),
            Dir::Horizontal => (Axis::Height, Side::Bottom, ("Top", "bottom", "top")),
        };
        // An app's name reads better than a side, unless both sides are the same app.
        let names = match (winutil::app_name(hwnd_of(a)), winutil::app_name(hwnd_of(b))) {
            (Some(x), Some(y)) if x != y => (x.clone(), x, y),
            _ => (sides.0.to_string(), format!("the {} window", sides.2), format!("the {} window", sides.1)),
        };
        let share = |(text, fraction): (&str, f32)| {
            let verb = GroupVerb::Share { axis, fraction, side: Some(side) };
            Some((format!("{} gets {text}", names.0), Choice::Share(a, verb)))
        };
        let mut items: Vec<_> = SHARES.into_iter().map(share).collect();
        items.push(None);
        items.push(Some((format!("Take out {}", names.1), Choice::TakeOut(a))));
        items.push(Some((format!("Take out {}", names.2), Choice::TakeOut(b))));
        Some((items, scene.owner))
    }
}

/// The size menu for a window in zones `from` to `to`: a share along each axis
/// where a zone lies beside it, and Take out.
fn key_items(hwnd: isize, (screen, from, to): &(String, usize, usize), scene: &Scene) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(s) = scene.screens.iter().find(|s| s.id == *screen) {
        let cells = zones::rects(&s.layout.tree, s.work, 0);
        if let (Some(a), Some(b)) = (cells.get(*from), cells.get(*to)) {
            let r = zones::span(*a, *b);
            let inside = |c: &Rect| c.x >= r.x && c.y >= r.y && c.x + c.w <= r.x + r.w && c.y + c.h <= r.y + r.h;
            let leaves: Vec<usize> = (0..cells.len()).filter(|i| inside(&cells[*i])).collect();
            let axes = [
                (Axis::Width, [Side::Left, Side::Right], "Width"),
                (Axis::Height, [Side::Top, Side::Bottom], "Height"),
            ];
            for (axis, sides, word) in axes {
                if sides.iter().all(|side| groups::edge_of(&s.layout.tree, s.work, &leaves, *side).is_none()) {
                    continue;
                }
                for (text, fraction) in SHARES {
                    let verb = GroupVerb::Share { axis, fraction, side: None };
                    items.push(Some((format!("{word}: {text}"), Choice::Share(hwnd, verb))));
                }
                items.push(None);
            }
        }
    }
    let name = winutil::app_name(hwnd_of(hwnd)).unwrap_or_else(|| "this window".into());
    items.push(Some((format!("Take out {name}"), Choice::TakeOut(hwnd))));
    items
}

/// Show a menu at `(x, y)` and hand its choice to the main thread `main`. The menu
/// takes focus while it is open, as a menu must to close on a click elsewhere, and
/// gives it back.
fn pick(items: &[Item], (x, y): (i32, i32), align: TRACK_POPUP_MENU_FLAGS, main: isize) {
    let Some(owner) = with_state(|s| s.owner) else { return };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for (i, item) in items.iter().enumerate() {
            let _ = match item {
                Some((text, _)) => {
                    let label: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                    AppendMenuW(menu, MF_STRING, i + 1, PCWSTR(label.as_ptr()))
                }
                None => AppendMenuW(menu, MF_SEPARATOR, 0, None),
            };
        }
        let before = GetForegroundWindow();
        let _ = SetForegroundWindow(owner);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | align, x, y, None, owner, None).0 as usize;
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if GetForegroundWindow() == owner && !before.is_invalid() {
            let _ = SetForegroundWindow(before);
        }
        let Some(Some((_, choice))) = id.checked_sub(1).and_then(|i| items.get(i)) else { return };
        *CHOICE.lock().unwrap_or_else(|e| e.into_inner()) = Some(*choice);
        let _ = PostMessageW(Some(hwnd_of(main)), WM_APP_DIVIDER, WPARAM(0), LPARAM(0));
    }
}

/// Right-click on a handle.
pub(super) fn handle_menu(hwnd: HWND) {
    let Some(Some((items, main))) = with_state(|s| s.handle_items(hwnd)) else { return };
    let pt = cursor();
    pick(&items, (pt.x, pt.y), TPM_LEFTALIGN, main);
}

/// The size menu key, at the window's centre.
pub(super) fn asked_menu() {
    let Some((hwnd, zone, scene)) = ASKED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let Some(r) = winutil::visible_rect(hwnd_of(hwnd)) else { return };
    let items = key_items(hwnd, &zone, &scene);
    pick(&items, (r.x + r.w / 2, r.y + r.h / 2), TPM_CENTERALIGN | TPM_VCENTERALIGN, scene.owner);
}
