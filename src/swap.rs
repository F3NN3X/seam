//! Dropping a window into a zone another window holds: the windows there take
//! the dropped window's old place instead of lying under it. A child of `snap`, so
//! it reads the engine and the drag. The decision is made from what is on the
//! screen, one pass over the top-level windows on the main thread, not from the
//! snapping's records, and the moves go through window control's queue, so they
//! land one after another like any other placement and are recorded as they land.

use super::*;
use crate::wincontrol;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, mpsc};

/// A mapped rect this close to a span of zones counts as being that span.
const MATCH_SLOP: i32 = 12;

/// Where an occupant goes.
enum Dest<'a> {
    /// The zones the dropped window was snapped to, on their screen.
    Zones(&'a Placed, usize, usize, Rect),
    /// The dropped window's rect before the drag, and the DPI of the screen it was on.
    Rect(Rect, u32),
}

/// Why a drop doesn't swap, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Skip {
    /// "Swap with the window already in a zone" is off.
    Off,
    /// No place to send them: the old zones or the old frame aren't known.
    NoPlace,
    /// The window was dropped over its own zones.
    OwnSpan,
    NoOccupants,
    /// One of them can't be moved or isn't responding.
    Refused,
}

/// How far each rect is shrunk before two count as overlapping, so zones that
/// only share an edge, or round a pixel into each other, don't.
const EDGE_SLOP: i32 = 2;

fn overlaps(a: Rect, b: Rect) -> bool {
    let shrink =
        |r: Rect| Rect { x: r.x + EDGE_SLOP, y: r.y + EDGE_SLOP, w: r.w - 2 * EDGE_SLOP, h: r.h - 2 * EDGE_SLOP };
    let (a, b) = (shrink(a), shrink(b));
    a.w > 0 && a.h > 0 && b.w > 0 && b.h > 0 && a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Whether at least half of `r` lies inside `zone`. A window an app keeps a few
/// pixels off its zones, or bigger than them for its minimum size, still counts.
pub(super) fn mostly_inside(r: Rect, zone: Rect) -> bool {
    let (shared, whole) = (shared_area(r, zone), area(r));
    whole > 0 && 2 * shared >= whole
}

fn area(r: Rect) -> i64 {
    i64::from(r.w.max(0)) * i64::from(r.h.max(0))
}

/// At least 60% of `r` inside `span`, and covering at least 60% of it, so a window
/// over two zones isn't taken for one of them.
pub(super) fn fits_mutually(r: Rect, span: Rect) -> bool {
    let shared = shared_area(r, span);
    area(r) > 0 && area(span) > 0 && 5 * shared >= 3 * area(r) && 5 * shared >= 3 * area(span)
}

/// The area `a` and `b` have in common.
fn shared_area(a: Rect, b: Rect) -> i64 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    i64::from(w.max(0)) * i64::from(h.max(0))
}

/// Where `r`, the part of it inside `from`, lands when `from` is stretched over `to`.
fn map_rect(r: Rect, from: Rect, to: Rect) -> Rect {
    let (x0, y0) = (r.x.max(from.x), r.y.max(from.y));
    let (x1, y1) = ((r.x + r.w).min(from.x + from.w), (r.y + r.h).min(from.y + from.h));
    let (kx, ky) = (to.w as f64 / from.w.max(1) as f64, to.h as f64 / from.h.max(1) as f64);
    Rect {
        x: to.x + ((x0 - from.x) as f64 * kx).round() as i32,
        y: to.y + ((y0 - from.y) as f64 * ky).round() as i32,
        w: (((x1 - x0).max(0) as f64 * kx).round() as i32).max(1),
        h: (((y1 - y0).max(0) as f64 * ky).round() as i32).max(1),
    }
}

/// The first and last of `placed` whose bounding box is `r`, give or take a few pixels.
fn zones_of(placed: &[Rect], r: Rect) -> Option<(usize, usize)> {
    let near =
        |a: Rect| [(a.x, r.x), (a.y, r.y), (a.w, r.w), (a.h, r.h)].iter().all(|(p, q)| (p - q).abs() <= MATCH_SLOP);
    (0..placed.len())
        .flat_map(|i| (i..placed.len()).map(move |j| (i, j)))
        .find(|(i, j)| near(zones::span(placed[*i], placed[*j])))
}

/// Every window a drop could swap with, topmost first, and where each is. Win32 reads
/// only, so it runs on the main thread or a worker. The time it took is logged once.
fn look(dragged: isize, landed: &[(isize, Rect)], exclude: &[String], how: &str) -> Vec<(isize, Rect)> {
    let started = log::enabled().then(Instant::now);
    let own = std::process::id();
    let list: Vec<(isize, Rect)> = winutil::top_level_windows()
        .into_iter()
        .filter(|h| h.0 as isize != dragged && winutil::is_snappable(*h, own, exclude))
        .filter_map(|h| {
            let key = h.0 as isize;
            let at = landed.iter().find(|(k, _)| *k == key).map(|(_, r)| *r);
            Some((key, at.or_else(|| winutil::visible_rect(h))?))
        })
        .collect();
    if let Some(started) = started {
        let us = started.elapsed().as_secs_f64() * 1e6;
        log::info!("zone swap: looked at {} windows {how} in {us:.0}us", list.len());
    }
    list
}

/// The worker's answer to a look.
pub(super) type Ahead = mpsc::Receiver<Vec<(isize, Rect)>>;

/// The drag the worker is looking for now. An older job is for a drag that is over.
static CURRENT: AtomicU64 = AtomicU64::new(0);
/// The look is ready, for the drag whose generation is in `wparam`.
pub const WM_APP_SWAP_LOOK: u32 = WM_APP + 35;

/// A look at the screen for a drag, and where to send it.
struct Look {
    generation: u64,
    owner: isize,
    dragged: isize,
    landed: Vec<(isize, Rect)>,
    exclude: Arc<[String]>,
    reply: mpsc::Sender<Vec<(isize, Rect)>>,
}

/// The tray's handler of `WM_APP_SWAP_LOOK`.
pub fn look_ready(generation: u64) {
    with_engine(|e| e.look_posted(generation));
}

/// Look for the newest drag only: older jobs waiting, and one whose drag is over, are skipped.
fn serve(rx: mpsc::Receiver<Look>) {
    while let Ok(mut job) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        if job.generation != CURRENT.load(Ordering::SeqCst) {
            continue;
        }
        let list = look(job.dragged, &job.landed, &job.exclude, "on a worker");
        // A reply nobody is waiting for means the drag ended.
        if job.reply.send(list).is_ok() {
            unsafe {
                let _ = PostMessageW(
                    Some(hwnd_of(job.owner)),
                    WM_APP_SWAP_LOOK,
                    WPARAM(job.generation as usize),
                    LPARAM(0),
                );
            }
        }
    }
}

/// The one worker that makes the looks, started the first time one is asked for.
fn worker() -> Option<&'static mpsc::Sender<Look>> {
    static WORKER: OnceLock<Option<mpsc::Sender<Look>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Look>();
            std::thread::Builder::new().name("zone-swap-look".into()).spawn(move || serve(rx)).ok().map(|_| tx)
        })
        .as_ref()
}

impl Engine {
    /// The windows on the screen in the zones `target` covers on `g`, and where each
    /// is. None when one of them can't be moved or is hung, which leaves the drop as
    /// it always was.
    pub(super) fn occupants(&self, drag: &Drag, g: &Placed, target: Rect) -> Option<Vec<(HWND, Rect)>> {
        match self.held_in(drag, g, target) {
            Ok(list) => Some(list),
            Err(Skip::NoOccupants) => Some(Vec::new()),
            Err(_) => None,
        }
    }

    /// The windows from the drag's look at the screen that fit the target, snapped or
    /// not, or one of the target's zones: the topmost for each, as the ones under it
    /// can't be seen. One that can't be moved is passed over, unless the snapping put
    /// it there, which leaves the drop as it always was.
    fn held_in(&self, drag: &Drag, g: &Placed, target: Rect) -> Result<Vec<(HWND, Rect)>, Skip> {
        let inner = g.zones.iter().copied().filter(|z| shared_area(*z, target) == area(*z));
        let spans: Vec<Rect> = std::iter::once(target).chain(inner).collect();
        let mut taken = vec![false; spans.len()];
        let mut occupants = Vec::new();
        for &(key, r) in &drag.others {
            let Some(i) = spans.iter().position(|s| fits_mutually(r, *s)) else { continue };
            if std::mem::replace(&mut taken[i], true) {
                continue;
            }
            let h = hwnd_of(key);
            if wincontrol::movable(h, &self.exclude).is_err() {
                let placed = self.snapped.get(&key).is_some_and(|s| {
                    s.screen == g.id
                        && s.from.max(s.to) < g.zones.len()
                        && overlaps(zones::span(g.zones[s.from], g.zones[s.to]), target)
                });
                if placed {
                    return Err(Skip::Refused);
                }
                continue;
            }
            occupants.push((h, r));
        }
        if occupants.is_empty() {
            return Err(Skip::NoOccupants);
        }
        Ok(occupants)
    }

    /// The look at the screen a drag starts with: every window a drop could swap
    /// with, topmost first, and where each is. `landed` were just sent elsewhere.
    pub(super) fn snapshot(&self, dragged: isize, landed: &[(isize, Rect)]) -> Vec<(isize, Rect)> {
        look(dragged, landed, &self.exclude, "on the spot")
    }

    /// Ask the worker for the look at drag start, so it is ready before zones first show.
    /// None when the worker can't be had: the look is then read on the spot.
    pub(super) fn look_ahead(&self, dragged: isize, landed: &[(isize, Rect)]) -> Option<(u64, Ahead)> {
        let (reply, rx) = mpsc::channel();
        let generation = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;
        let (landed, exclude) = (landed.to_vec(), Arc::clone(&self.exclude));
        let job = Look { generation, owner: self.owner, dragged, landed, exclude, reply };
        worker()?.send(job).ok()?;
        Some((generation, rx))
    }

    /// The drag's look at the screen, if it is there. The tray never waits for the
    /// worker: until it answers there is no look, and the worker posts when it has. Read on
    /// the spot only when there is no worker, or it is gone.
    pub(super) fn others(&self, drag: &mut Drag) -> Option<Vec<(isize, Rect)>> {
        let Some((_, rx)) = &drag.ahead else { return Some(self.snapshot(drag.hwnd, &drag.landed)) };
        match rx.try_recv() {
            Ok(list) => {
                drag.ahead = None;
                Some(list)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                drag.ahead = None;
                Some(self.snapshot(drag.hwnd, &drag.landed))
            }
        }
    }

    /// The worker's look is ready: if its drag is still the one running and zones show,
    /// draw them again once, now with the windows they swap with.
    fn look_posted(&mut self, generation: u64) {
        let Some(mut drag) = self.drag.take() else { return };
        let mine = drag.ahead.as_ref().is_some_and(|(g, _)| *g == generation);
        let (view, beside) = (drag.shown, drag.beside);
        if mine && (view.is_some() || beside.is_some()) {
            self.present(&mut drag, view, beside);
        }
        self.drag = Some(drag);
    }

    /// At the drop: where the windows on the screen it was dropped on are now, as
    /// they may have moved during the drag. Only those few are read again.
    pub(super) fn refresh(&self, drag: &mut Drag) {
        let Some(view) = drag.shown else { return };
        let bounds = drag.geometry[view.screen].bounds;
        // Dropped before the worker's look came: read it now.
        if !drag.looked {
            drag.others = self.snapshot(drag.hwnd, &drag.landed);
            drag.looked = true;
        }
        // One closed, minimized or hidden since is no longer there to swap with.
        drag.others.retain_mut(|(key, r)| {
            if !overlaps(*r, bounds) {
                return true;
            }
            let h = hwnd_of(*key);
            let there = unsafe { IsWindow(Some(h)) }.as_bool() && !winutil::is_minimized(h) && !winutil::is_cloaked(h);
            match winutil::visible_rect(h).filter(|_| there) {
                Some(now) => {
                    *r = now;
                    true
                }
                None => false,
            }
        });
    }

    /// The zones the dragged window was in when the drag began, from where it was:
    /// the span on its screen its frame fits best, at least half of it inside and
    /// covering at least half of the span. None for a maximized window, or one that
    /// fits none, which is loose.
    /// When none fits, the snapping's record still counts while at least half of the
    /// window is inside its zones, as for one bigger than them for its minimum size.
    pub(super) fn zones_held(
        &self,
        hwnd: HWND,
        geometry: &[Placed],
        frame: Option<Rect>,
        was_maximized: bool,
    ) -> Option<(String, usize, usize)> {
        let frame = frame.filter(|_| !was_maximized)?;
        let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
        let g = geometry.iter().find(|g| g.bounds.contains(x, y))?;
        let n = g.zones.len();
        let span = |(i, j): (usize, usize)| zones::span(g.zones[i], g.zones[j]);
        let best = (0..n)
            .flat_map(|i| (i..n).map(move |j| (i, j)))
            .filter(|ij| fits_mutually(frame, span(*ij)))
            .min_by_key(|ij| (area(span(*ij)) - area(frame)).abs());
        if let Some((from, to)) = best {
            return Some((g.id.clone(), from, to));
        }
        let s = self.snapped.get(&(hwnd.0 as isize))?;
        let g = geometry.iter().find(|g| g.id == s.screen && s.from.max(s.to) < g.zones.len())?;
        mostly_inside(frame, zones::span(g.zones[s.from], g.zones[s.to])).then(|| (g.id.clone(), s.from, s.to))
    }

    /// At the drop, with `--log`: where the snapping and the screen disagree about the
    /// dragged window and the windows in the target, and how long the look took.
    pub(super) fn audit(&self, drag: &Drag, screen: usize, (a, b): (usize, usize)) {
        let g = &drag.geometry[screen];
        let target = zones::span(g.zones[a], g.zones[b]);
        let found = self.held_in(drag, g, target);
        let record = |key: isize| self.snapped.get(&key).map(|s| (s.screen == g.id, s.from, s.to, s.at));
        let had = record(drag.hwnd).map(|(_, f, t, at)| (f, t, at));
        let shows = drag.old_zone.as_ref().map(|(_, f, t)| (*f, *t));
        if had.map(|(f, t, _)| (f, t)) != shows {
            log::info!(
                "zone swap: the snapping had the dragged window in {had:?}, the screen showed zones {shows:?} at {:?}",
                drag.frame
            );
        }
        for (h, r) in found.unwrap_or_default() {
            let had = record(h.0 as isize);
            let inside = had.is_some_and(|(same, f, t, _)| {
                same && f.max(t) < g.zones.len() && mostly_inside(r, zones::span(g.zones[f], g.zones[t]))
            });
            if !inside {
                log::info!(
                    "zone swap: the snapping had a window in the target as {had:?}, the screen shows it at {r:?}"
                );
            }
        }
    }

    /// The moves that give the windows in the zones `(a, b)` of screen `screen` the
    /// dropped window's old place, and whether it was snapped. None when nothing
    /// swaps, and the drop is as it always was.
    pub(super) fn swap_plan(&self, drag: &Drag, screen: usize, ab: (usize, usize)) -> Option<(Vec<Placement>, bool)> {
        self.swap_why(drag, screen, ab).ok()
    }

    /// `swap_plan`, or why there is no swap.
    pub(super) fn swap_why(
        &self,
        drag: &Drag,
        screen: usize,
        (a, b): (usize, usize),
    ) -> Result<(Vec<Placement>, bool), Skip> {
        if !self.settings.swap_on_drop {
            return Err(Skip::Off);
        }
        let g = &drag.geometry[screen];
        let zone = |z: &Placed, i: usize| z.zones.get(i).copied().ok_or(Skip::NoPlace);
        let target = zones::span(zone(g, a)?, zone(g, b)?);
        let dest = match &drag.old_zone {
            Some((id, from, to)) => {
                let old = drag.geometry.iter().find(|x| x.id == *id).ok_or(Skip::NoPlace)?;
                let span = zones::span(zone(old, *from)?, zone(old, *to)?);
                // Growing or shrinking over its own area isn't a swap.
                if old.id == g.id && overlaps(span, target) {
                    return Err(Skip::OwnSpan);
                }
                Dest::Zones(old, *from, *to, span)
            }
            None => {
                let frame = drag.frame.filter(|_| !drag.was_maximized).ok_or(Skip::NoPlace)?;
                let (x, y) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
                let on = self.screens.iter().find(|s| s.bounds.contains(x, y)).ok_or(Skip::NoPlace)?;
                Dest::Rect(frame, work_area(on.bounds).ok_or(Skip::NoPlace)?.1)
            }
        };
        let occupants = self.held_in(drag, g, target)?;
        let alone = occupants.len() == 1;
        let place = |(h, at): &(HWND, Rect)| {
            let hwnd = h.0 as isize;
            match &dest {
                Dest::Zones(old, from, to, span) => {
                    let zone = |f: usize, t: usize| Some((old.id.clone(), f, t));
                    if alone {
                        return Placement { hwnd, target: *span, zone: zone(*from, *to), dpi: old.dpi };
                    }
                    let mut mapped = map_rect(*at, target, *span);
                    match zones_of(&old.zones, mapped) {
                        Some((f, t)) => {
                            let target = zones::span(old.zones[f], old.zones[t]);
                            Placement { hwnd, target, zone: zone(f, t), dpi: old.dpi }
                        }
                        None => {
                            let (w, ht) = minsize::cached(*h, old.dpi);
                            mapped.w = mapped.w.max(w);
                            mapped.h = mapped.h.max(ht);
                            Placement { hwnd, target: mapped, zone: None, dpi: old.dpi }
                        }
                    }
                }
                Dest::Rect(frame, dpi) => {
                    let mut mapped = if alone { *frame } else { map_rect(*at, target, *frame) };
                    let (w, ht) = minsize::cached(*h, *dpi);
                    mapped.w = mapped.w.max(w);
                    mapped.h = mapped.h.max(ht);
                    Placement { hwnd, target: mapped, zone: None, dpi: *dpi }
                }
            }
        };
        Ok((occupants.iter().map(place).collect(), drag.old_zone.is_some()))
    }

    /// Start the moves `swap_plan` made, once the dropped window has been sent to its zones.
    pub(super) fn swap_start(&mut self, plan: Option<(Vec<Placement>, bool)>) {
        let Some((list, snapped)) = plan else { return };
        let was = if snapped { "snapped" } else { "loose" };
        log::info!("zone swap: moved {} window(s), the dropped window was {was}", list.len());
        self.queued.extend(list);
        self.window_pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn occupants_keep_their_place_in_the_old_span() {
        // The left half of a 1000x500 span lands on the left half of a 400x200 one.
        let m = map_rect(r(0, 0, 500, 500), r(0, 0, 1000, 500), r(100, 50, 400, 200));
        assert_eq!(m, r(100, 50, 200, 200));
        // What sticks out of the span is cut to it.
        let m = map_rect(r(-100, 0, 600, 500), r(0, 0, 1000, 500), r(0, 0, 400, 200));
        assert_eq!(m, r(0, 0, 200, 200));
    }

    #[test]
    fn zones_that_share_an_edge_dont_overlap() {
        // Two rows at gap 0, and a pixel of rounding into each other.
        assert!(!overlaps(r(0, 0, 1000, 500), r(0, 500, 1000, 500)));
        assert!(!overlaps(r(0, 0, 1000, 501), r(0, 500, 1000, 500)));
        assert!(overlaps(r(0, 0, 1000, 600), r(0, 500, 1000, 500)));
        // A window a few pixels off, or taller for its minimum, is still in its zone.
        assert!(mostly_inside(r(3, -7, 1000, 520), r(0, 0, 1000, 500)));
        assert!(!mostly_inside(r(0, 400, 1000, 500), r(0, 0, 1000, 500)));
    }

    #[test]
    fn a_mapped_rect_finds_its_zones() {
        let zones = [r(0, 0, 100, 100), r(100, 0, 100, 100), r(0, 100, 200, 100)];
        assert_eq!(zones_of(&zones, r(3, 0, 98, 101)), Some((0, 0)));
        assert_eq!(zones_of(&zones, r(0, 0, 200, 100)), Some((0, 1)));
        assert_eq!(zones_of(&zones, r(0, 0, 150, 70)), None);
    }
}
