//! Low-level mouse hook: when the pointer crosses a calibrated border, put it
//! where the screens physically meet instead of at the same pixel row.
//!
//! The hook lives on its own high-priority thread with its own message loop, so
//! UI work like painting the overlay can never delay mouse input.

use crate::layout::{decide, Crossing, Live, Push};
use crate::log;
use std::cell::Cell;
use std::sync::mpsc::{channel, Sender};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Hook timing in ns since the last `log_stats`. Only updated while logging.
#[derive(Clone, Copy, Default)]
struct Stats {
    events: u64,
    total: u64,
    max: u64,
    own_max: u64,
    next_max: u64,
    own_hist: [u32; BUCKETS],
    next_hist: [u32; BUCKETS],
    own_over_1ms: u32,
    next_over_1ms: u32,
}

const BUCKETS: usize = 13;
const MS_NS: u64 = 1_000_000;

/// Log2 bucket of a time in microseconds: 0 is under 1us, then 1, 2-3, 4-7 and
/// so on, with the last holding 2048us and up.
fn bucket(ns: u64) -> usize {
    ((64 - (ns / 1000).leading_zeros()) as usize).min(BUCKETS - 1)
}

/// Written by the UI thread, read by the hook. The hook only ever tries the
/// lock, so at worst one move falls through to Windows during a swap.
static LIVE: RwLock<Live> = RwLock::new(Live { rects: Vec::new(), links: Vec::new() });

thread_local! {
    // These four belong to the hook thread.
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    static STATS: Cell<Stats> = Cell::new(Stats::default());
    /// How far the pointer has pushed past a glued edge.
    static PUSH: Cell<Push> = Cell::new(Push::default());
    /// Time the current event spent in hooks after ours.
    static NEXT_NS: Cell<u64> = const { Cell::new(0) };
}

/// Marks the empty mouse move window control sends so Windows lets it take focus.
pub const SEAM_INPUT_TAG: usize = 0x5EA3_F0C5;
const WM_APP_COMMAND: u32 = WM_APP + 20;
const OFF: usize = 0;
const ON: usize = 1;
const STATS_LINE: usize = 2;

pub fn set_live(live: Live) {
    let mut guard = LIVE.write().unwrap_or_else(|e| e.into_inner());
    let old = std::mem::replace(&mut *guard, live);
    drop(guard);
    drop(old); // freed outside the lock
}

/// Install or remove the hook. Waits until it's done, because callers rely on
/// the hook being gone before a mode set.
pub fn enable(on: bool) {
    command(if on { ON } else { OFF });
}

fn command(cmd: usize) {
    let (done, wait) = channel::<()>();
    let done = Box::into_raw(Box::new(done));
    unsafe {
        if PostThreadMessageW(hook_thread(), WM_APP_COMMAND, WPARAM(cmd), LPARAM(done as isize)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(1));
        } else {
            drop(Box::from_raw(done));
            log::info!("hook thread unavailable");
        }
    }
}

/// The hook thread's id, starting it on first use. Zero if it couldn't start.
fn hook_thread() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| {
        let (ready, wait) = channel();
        let spawned = std::thread::Builder::new().name("seam-hook".into()).spawn(move || unsafe {
            let mut msg = MSG::default();
            // Creates this thread's queue, so posting to it can't fail once `ready` is sent.
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            let _ = ready.send(GetCurrentThreadId());
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_APP_COMMAND {
                    run_command(msg.wParam.0);
                    let _ = Box::from_raw(msg.lParam.0 as *mut Sender<()>).send(());
                }
            }
        });
        spawned.ok().and_then(|_| wait.recv().ok()).unwrap_or(0)
    })
}

fn run_command(cmd: usize) {
    if cmd == STATS_LINE {
        write_stats();
        return;
    }
    HOOK.with(|h| unsafe {
        match (cmd == ON, h.get()) {
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

/// Where this move should land, unless the pointer is confined.
fn crossing(from: (i32, i32), pt: (i32, i32)) -> Option<Crossing> {
    let live = LIVE.try_read().ok()?;
    // Most moves cross nothing, so only then pay for the system call.
    let c = decide(&live, from, pt)?;
    (!cursor_is_confined(&live)).then_some(c)
}

/// Where this move should put the pointer, with the crossing if it crosses. A
/// glued crossing waits at the edge until the push past it adds up.
fn target(from: (i32, i32), pt: (i32, i32)) -> Option<((i32, i32), Option<Crossing>)> {
    let c = crossing(from, pt);
    let (push, go) = PUSH.get().after(c.as_ref());
    PUSH.set(push);
    let c = c?;
    if go {
        Some((c.to, Some(c)))
    } else {
        c.hold.map(|at| (at, None))
    }
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
    // Seam's own empty move before it takes focus is never a pointer move to remap.
    if (*(lparam.0 as *const MSLLHOOKSTRUCT)).dwExtraInfo == SEAM_INPUT_TAG {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    if !log::enabled() {
        return on_mouse(code, wparam, lparam);
    }
    let started = Instant::now();
    let res = on_mouse(code, wparam, lparam);
    let ns = started.elapsed().as_nanos() as u64;
    let next = NEXT_NS.take();
    STATS.with(|s| {
        let t = s.get();
        let own = ns - next;
        let (mut own_hist, mut next_hist) = (t.own_hist, t.next_hist);
        own_hist[bucket(own)] += 1;
        next_hist[bucket(next)] += 1;
        s.set(Stats {
            events: t.events + 1,
            total: t.total + ns,
            max: t.max.max(ns),
            own_max: t.own_max.max(own),
            next_max: t.next_max.max(next),
            own_hist,
            next_hist,
            own_over_1ms: t.own_over_1ms + (own >= MS_NS) as u32,
            next_over_1ms: t.next_over_1ms + (next >= MS_NS) as u32,
        });
    });
    res
}

/// Log the hook's timing since the last call.
pub fn log_stats() {
    if log::enabled() {
        command(STATS_LINE);
    }
}

/// Covers every event, not just remaps, and separates our own time from hooks
/// later in the chain.
fn write_stats() {
    let t = STATS.with(Cell::take);
    if t.events > 0 {
        let us = |ns: u64| ns as f64 / 1e3;
        log::info!(
            "hook stats: {} events, avg {:.1}us, max {:.1}us, own max {:.1}us, next hooks max {:.1}us, own_hist={:?} next_hist={:?} own_over_1ms={} next_over_1ms={}",
            t.events,
            t.total as f64 / t.events as f64 / 1e3,
            us(t.max),
            us(t.own_max),
            us(t.next_max),
            t.own_hist,
            t.next_hist,
            t.own_over_1ms,
            t.next_over_1ms
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
                if let Some((to, c)) = target(from, pt) {
                    if to != pt && SetCursorPos(to.0, to.1).is_ok() {
                        if let Some(t0) = started {
                            let us = t0.elapsed().as_secs_f64() * 1e6;
                            match c {
                                Some(c) => log::info!(
                                    "remap {from:?} -> {to:?} (raw {pt:?}) border {}->{} {:?} hook {us:.1}us",
                                    c.src,
                                    c.dst,
                                    c.side
                                ),
                                None => log::info!("hold {from:?} at {to:?} (raw {pt:?}) hook {us:.1}us"),
                            }
                        }
                        return LRESULT(1); // swallow the original move
                    }
                }
            }
        }
    }
    call_next(code, wparam, lparam)
}
