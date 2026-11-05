//! Low-level mouse hook: when the pointer crosses a calibrated border, put it
//! where the screens physically meet instead of at the same pixel row.

use crate::layout::{decide, Live};
use crate::log;
use std::cell::{Cell, RefCell};
use std::time::Instant;
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live::default());
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    /// Events, total ns and slowest ns since the last `log_stats`. Only updated while logging.
    static STATS: Cell<(u64, u64, u64)> = const { Cell::new((0, 0, 0)) };
}

pub fn set_live(live: Live) {
    LIVE.with(|l| *l.borrow_mut() = live);
}

/// Install or remove the hook. Must be called from the thread running the
/// message loop; the hook callback runs on that thread.
pub fn enable(on: bool) {
    HOOK.with(|h| unsafe {
        match (on, h.get()) {
            (true, None) => match SetWindowsHookExW(WH_MOUSE_LL, Some(ll_proc), None, 0) {
                Ok(hh) => {
                    h.set(Some(hh));
                    log::info!("hook installed");
                }
                Err(e) => log::info!("hook install failed: {e}"),
            },
            (false, Some(hh)) => {
                let res = UnhookWindowsHookEx(hh);
                h.set(None);
                log::info!("hook uninstalled: {res:?}");
            }
            _ => {}
        }
    });
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
    if !log::enabled() {
        return on_mouse(code, wparam, lparam);
    }
    let started = Instant::now();
    let res = on_mouse(code, wparam, lparam);
    let ns = started.elapsed().as_nanos() as u64;
    STATS.with(|s| {
        let (events, total, max) = s.get();
        s.set((events + 1, total + ns, max.max(ns)));
    });
    res
}

/// Log the hook's timing since the last call. Covers every event, not just remaps.
pub fn log_stats() {
    let (events, total, max) = STATS.with(Cell::take);
    if events > 0 {
        let avg = total as f64 / events as f64 / 1e3;
        log::info!("hook stats: {events} events, avg {avg:.1}us, max {:.1}us", max as f64 / 1e3);
    }
}

unsafe fn on_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEMOVE {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        // Leave injected input (remote desktop, automation, our own moves) alone.
        if info.flags & LLMHF_INJECTED == 0 {
            let started = log::enabled().then(Instant::now);
            let mut prev = POINT::default();
            if GetCursorPos(&mut prev).is_ok() {
                let (from, pt) = ((prev.x, prev.y), (info.pt.x, info.pt.y));
                let crossing = LIVE.with(|l| {
                    let live = l.try_borrow().ok()?;
                    if live.links.is_empty() || cursor_is_confined(&live) {
                        return None;
                    }
                    decide(&live, from, pt)
                });
                if let Some(c) = crossing {
                    if c.to != pt && SetCursorPos(c.to.0, c.to.1).is_ok() {
                        if let Some(t0) = started {
                            let us = t0.elapsed().as_secs_f64() * 1e6;
                            log::info!(
                                "remap {from:?} -> {:?} (raw {pt:?}) border {}->{} {:?} hook {us:.1}us",
                                c.to,
                                c.src,
                                c.dst,
                                c.side
                            );
                        }
                        return LRESULT(1); // swallow the original move
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}
