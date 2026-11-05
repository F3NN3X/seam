//! Low-level mouse hook: when the pointer crosses a calibrated border, put it
//! where the screens physically meet instead of at the same pixel row.

use crate::layout::{decide, Live};
use crate::log;
use std::cell::{Cell, RefCell};
use std::time::Instant;
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Hook timing in ns since the last `log_stats`. Only updated while logging.
#[derive(Clone, Copy, Default)]
struct Stats {
    events: u64,
    total: u64,
    max: u64,
    own_max: u64,
    next_max: u64,
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live::default());
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    static STATS: Cell<Stats> = Cell::new(Stats::default());
    /// Time the current event spent in hooks after ours.
    static NEXT_NS: Cell<u64> = const { Cell::new(0) };
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
    let next = NEXT_NS.take();
    STATS.with(|s| {
        let t = s.get();
        s.set(Stats {
            events: t.events + 1,
            total: t.total + ns,
            max: t.max.max(ns),
            own_max: t.own_max.max(ns - next),
            next_max: t.next_max.max(next),
        });
    });
    res
}

/// Log the hook's timing since the last call. Covers every event, not just
/// remaps, and separates our own time from hooks later in the chain.
pub fn log_stats() {
    let t = STATS.with(Cell::take);
    if t.events > 0 {
        let us = |ns: u64| ns as f64 / 1e3;
        log::info!(
            "hook stats: {} events, avg {:.1}us, max {:.1}us, own max {:.1}us, next hooks max {:.1}us",
            t.events,
            t.total as f64 / t.events as f64 / 1e3,
            us(t.max),
            us(t.own_max),
            us(t.next_max)
        );
    }
}

unsafe fn call_next(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if !log::enabled() {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let started = Instant::now();
    let res = CallNextHookEx(None, code, wparam, lparam);
    NEXT_NS.set(started.elapsed().as_nanos() as u64);
    res
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
    call_next(code, wparam, lparam)
}
