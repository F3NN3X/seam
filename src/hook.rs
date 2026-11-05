//! Low-level mouse hook: when the pointer crosses a calibrated border, put it
//! where the screens physically meet instead of at the same pixel row.

use crate::geom::{exit_of, remap_crossing, EdgeMap, Rect, Side};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

/// A calibrated border between two monitors (indices into `Live::rects`).
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Side of `a` that `b` sits on.
    pub side: Side,
    pub map: EdgeMap,
}

#[derive(Default)]
pub struct Live {
    pub rects: Vec<Rect>,
    pub links: Vec<Link>,
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live::default());
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
}

pub fn set_live(live: Live) {
    LIVE.with(|l| *l.borrow_mut() = live);
}

/// Install or remove the hook. Must be called from the thread running the
/// message loop; the hook callback runs on that thread.
pub fn enable(on: bool) {
    HOOK.with(|h| unsafe {
        match (on, h.get()) {
            (true, None) => {
                if let Ok(hh) = SetWindowsHookExW(WH_MOUSE_LL, Some(ll_proc), None, 0) {
                    h.set(Some(hh));
                }
            }
            (false, Some(hh)) => {
                let _ = UnhookWindowsHookEx(hh);
                h.set(None);
            }
            _ => {}
        }
    });
}

/// Pure decision logic, kept separate from the Win32 plumbing.
fn decide(live: &Live, prev: (i32, i32), pt: (i32, i32)) -> Option<(i32, i32)> {
    let src = live.rects.iter().position(|r| r.contains(prev.0, prev.1))?;
    let a = live.rects[src];
    let (side, along, over) = exit_of(&a, prev, pt)?;
    for l in &live.links {
        let (dst, map) = if l.a == src && l.side == side {
            (l.b, l.map)
        } else if l.b == src && l.side.opposite() == side {
            (l.a, l.map.inverse())
        } else {
            continue;
        };
        if let Some(p) = remap_crossing(&a, &live.rects[dst], side, &map, along, over) {
            return Some(p);
        }
    }
    None
}

/// Games and some apps confine the pointer with ClipCursor; leave those alone.
fn cursor_is_confined(live: &Live) -> bool {
    let mut clip = RECT::default();
    if unsafe { GetClipCursor(&mut clip) }.is_err() {
        return false;
    }
    let (mut l, mut t, mut r, mut b) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for rc in &live.rects {
        l = l.min(rc.left);
        t = t.min(rc.top);
        r = r.max(rc.right);
        b = b.max(rc.bottom);
    }
    clip.left > l || clip.top > t || clip.right < r || clip.bottom < b
}

unsafe extern "system" fn ll_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEMOVE {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        // Leave injected input (remote desktop, automation, our own moves) alone.
        if info.flags & LLMHF_INJECTED == 0 {
            let mut prev = POINT::default();
            if GetCursorPos(&mut prev).is_ok() {
                let dest = LIVE.with(|l| {
                    let live = l.try_borrow().ok()?;
                    if live.links.is_empty() || cursor_is_confined(&live) {
                        return None;
                    }
                    decide(&live, (prev.x, prev.y), (info.pt.x, info.pt.y))
                });
                if let Some((x, y)) = dest {
                    if (x, y) != (info.pt.x, info.pt.y) && SetCursorPos(x, y).is_ok() {
                        return LRESULT(1); // swallow the original move
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}
