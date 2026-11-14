//! Where a window goes, and moving it there.

use super::*;

fn close_to(a: Rect, b: Rect, sized: bool) -> bool {
    let size = [(a.w, b.w), (a.h, b.h)];
    let sides = [(a.x, b.x), (a.y, b.y)].into_iter().chain(size.into_iter().filter(|_| sized));
    sides.into_iter().all(|(p, q)| (p - q).abs() <= CLOSE_ENOUGH)
}

pub(super) fn now_of(h: HWND, dpi: u32) -> Option<Now> {
    let window = winutil::window_rect(h)?;
    let shown = shown_of(h);
    let frame = match shown {
        Shown::Minimized => winutil::normal_frame(h, dpi)?,
        _ => winutil::visible_rect(h).unwrap_or(window),
    };
    Some(Now { window, frame, shown, resizable: winutil::is_resizable(h) })
}

/// What a rule asks for on its screen, or `None` for a window to keep where it is.
fn plan(rule: &Rule, geo: &Geo) -> Option<Kind> {
    if rule.target == (Target::State { state: WinState::Keep }) {
        return None;
    }
    let rect = workspace::rect_for(&rule.target, &geo.zones, geo.work);
    let normal = rule.normal_rect.map(|f| workspace::from_fractions(f, geo.work));
    Some(match (rule.shown, rect) {
        (Shown::Minimized, _) => Kind::Min(rect.or(normal).unwrap_or(geo.work)),
        (Shown::Fullscreen, _) => Kind::Full(geo.bounds),
        (Shown::Normal, Some(r)) => Kind::Rect(r),
        (Shown::Maximized, _) | (Shown::Normal, None) => Kind::Max { work: geo.work, normal: normal.or(rect) },
    })
}

/// Where a rule puts its window, and on which screen. With "Keep its pixel size" or
/// "Keep its real size", a free place restored on a screen other than the one it was
/// captured on (`captured_on`, or without it, one where the saved fractions no longer
/// give that size) keeps the pixels or the millimetres it had at capture (§11.4): on
/// its own screen, or, with that screen gone, on the one the window is on now. Then it comes with the line to log once
/// it's moved. `tell` logs a window Windows scales because a screen doesn't report
/// its size. `None` leaves the window where it is.
pub(super) fn destination<'g>(
    rule: &Rule,
    geos: &'g [Geo],
    h: HWND,
    mode: WindowSize,
    tell: bool,
) -> Option<Planned<'g>> {
    let saved = geos.iter().find(|g| g.id == rule.monitor);
    let px = rule.size_px.map(|p| [p[0] as f32, p[1] as f32]);
    let kept = match mode {
        WindowSize::Windows => None,
        WindowSize::Pixels => px,
        WindowSize::Real => rule.size_mm,
    };
    let kept = kept.filter(|_| matches!(rule.target, Target::Rect { .. }));
    let Some(kept) = kept else { return saved.and_then(|geo| Some((geo, plan(rule, geo)?, None))) };
    let geo = match saved {
        Some(geo) => geo,
        None => {
            let now = winutil::visible_rect(h)?;
            geos.iter().find(|g| g.bounds.contains(centre(now).0, centre(now).1))?
        }
    };
    let kind = plan(rule, geo)?;
    let (Kind::Rect(at) | Kind::Min(at)) = kind else { return Some((geo, kind, None)) };
    // Restored on the screen it was captured on. Without that record, the screen
    // it was captured on is the one whose fractions still give the size it had:
    // in millimetres where both screens report one, else in pixels.
    let same = match &rule.captured_on {
        Some(id) => *id == geo.id,
        None => {
            let near = |now: [f32; 2], then: [f32; 2]| (0..2).all(|i| (now[i] - then[i]).abs() <= then[i] * SAME_SIZE);
            let size = match (rule.size_mm, desk::size_mm(at.w, at.h, geo.raw_dpi)) {
                (Some(then), Some(now)) => near(now, then),
                _ => px.is_none_or(|then| near([at.w as f32, at.h as f32], then)),
            };
            saved.is_some() && size
        }
    };
    if same {
        return Some((geo, kind, None));
    }
    let size = (f64::from(kept[0]), f64::from(kept[1]));
    let resized = match mode {
        WindowSize::Pixels => Some(desk::centred(size, at, geo.work)),
        _ => desk::physical_rect(size, desk::MM_PER_INCH, geo.raw_dpi, at, geo.work),
    };
    let Some(r) = resized else {
        if tell {
            log::info!(
                "window size: {} scaled by Windows, screen {} doesn't report its size",
                app_name(rule),
                geo.number
            );
        }
        // On its own screen it keeps the saved fractions. A screen that's gone leaves it where it is.
        return saved.map(|geo| (geo, kind, None));
    };
    let (app, to) = (app_name(rule), geo.number);
    let note =
        format!("window size ({mode:?}): {app} screen at capture → {to}, {}×{} px → {}×{} px", at.w, at.h, r.w, r.h);
    Some((geo, if matches!(kind, Kind::Min(_)) { Kind::Min(r) } else { Kind::Rect(r) }, Some(note)))
}

/// Whether the window already is where, and as, the plan wants it. Such a
/// window takes no call at all.
pub(super) fn in_place(kind: &Kind, now: &Now, screen: Rect) -> bool {
    match *kind {
        Kind::Rect(r) => now.shown == Shown::Normal && close_to(now.frame, r, now.resizable),
        Kind::Min(r) => now.shown == Shown::Minimized && close_to(now.frame, r, now.resizable),
        Kind::Max { .. } => {
            now.shown == Shown::Maximized && screen.contains(centre(now.window).0, centre(now.window).1)
        }
        Kind::Full(r) => now.shown == Shown::Fullscreen && now.window == r,
    }
}

fn pause(cancelled: &dyn Fn() -> bool, d: Duration) -> bool {
    std::thread::sleep(d);
    !cancelled()
}

/// The calls that wait on the app are skipped for one that has stopped answering.
fn answering(m: &Move, skipped: &mut Vec<(String, String)>) -> bool {
    let hung = winutil::is_hung(hwnd_of(m.hwnd));
    if hung {
        skipped.push((m.app.clone(), "timeout".into()));
    }
    !hung
}

/// Whether the window ended up as the move wanted.
fn landed(h: HWND, kind: &Kind) -> bool {
    match *kind {
        Kind::Rect(r) => winutil::visible_rect(h).is_some_and(|v| close_to(v, r, winutil::is_resizable(h))),
        Kind::Max { .. } => winutil::is_maximized(h),
        Kind::Min(_) => winutil::is_minimized(h),
        Kind::Full(r) => winutil::window_rect(h) == Some(r),
    }
}

/// Put each window where it goes, in one pass. Only a window that goes to a screen
/// with another DPI, or a maximized one that changes screens, takes a second step
/// after the apps have had a moment to react (D8), and only those are checked and
/// retried once. Returns how many moved.
pub(super) fn place_all(moves: &[Move], skipped: &mut Vec<(String, String)>, cancelled: &dyn Fn() -> bool) -> usize {
    let mut moved = 0;
    let mut later: Vec<&Move> = Vec::new();
    for m in moves {
        let h = hwnd_of(m.hwnd);
        // Seam is moving it, so the zone it was snapped to no longer holds (a re-fit would pull it back).
        winutil::clear_zone_stamp(h);
        let restoring = matches!(m.was, Shown::Minimized | Shown::Maximized);
        match m.kind {
            Kind::Min(r) => {
                if answering(m, skipped) && winutil::place_minimized(h, r, m.dpi) {
                    moved += 1;
                }
            }
            Kind::Max { normal, .. } if m.on_screen => {
                if answering(m, skipped) && winutil::place_maximized(h, normal) {
                    moved += 1;
                }
            }
            Kind::Max { work, .. } => {
                if !restoring {
                    winutil::move_to(h, work);
                } else if !answering(m, skipped) || !winutil::place_restored(h, work, m.dpi) {
                    continue;
                }
                later.push(m);
            }
            // A window that has its caption back isn't full screen any more.
            Kind::Full(_) if winutil::has_caption(h) => skipped.push((m.app.clone(), "not full screen".into())),
            Kind::Rect(r) | Kind::Full(r) => {
                if restoring {
                    if !answering(m, skipped) || !winutil::place_restored(h, r, m.dpi) {
                        continue;
                    }
                } else if m.crosses {
                    winutil::move_to(h, r);
                } else if matches!(m.kind, Kind::Full(_)) {
                    winutil::set_rect(h, r);
                } else {
                    winutil::fit(h, r);
                }
                if m.crosses {
                    later.push(m);
                } else {
                    moved += 1;
                }
            }
        }
    }
    if later.is_empty() || !pause(cancelled, SETTLE) {
        return moved;
    }
    // On the new screen, with its DPI, the margins and size are right this time.
    for m in &later {
        let h = hwnd_of(m.hwnd);
        match m.kind {
            Kind::Rect(r) => winutil::fit(h, r),
            Kind::Full(r) => winutil::set_rect(h, r),
            Kind::Max { normal, .. } => {
                if answering(m, skipped) {
                    winutil::place_maximized(h, normal);
                }
            }
            Kind::Min(_) => {}
        }
    }
    if !pause(cancelled, SETTLE) {
        return moved;
    }
    for m in later {
        let h = hwnd_of(m.hwnd);
        // An app that answered a DPI change late can leave a window off by its margins.
        if let (Kind::Rect(r), false) = (&m.kind, landed(h, &m.kind)) {
            winutil::fit(h, *r);
        }
        if landed(h, &m.kind) || winutil::window_rect(h).is_some_and(|now| now != m.before) {
            moved += 1;
        } else if !skipped.iter().any(|(app, _)| *app == m.app) {
            skipped.push((m.app.clone(), "didn't move".into()));
        }
    }
    moved
}

/// Stack the shown windows as they were at capture, bottom first, unless they
/// already are. Windows whose stacking wasn't captured are left where they are.
pub(super) fn restack(stack: &[Stacked], moved: bool) {
    let mut known: Vec<&Stacked> = stack.iter().filter(|s| s.z != u32::MAX).collect();
    known.sort_by_key(|s| s.z);
    let mut now = known.clone();
    now.sort_by_key(|s| s.now.unwrap_or(usize::MAX));
    // A move can change the stacking, and a window found after the first look has none to compare.
    let stacked = !moved && known.iter().zip(&now).all(|(a, b)| a.hwnd == b.hwnd && a.now.is_some());
    if stacked {
        return;
    }
    for s in known.iter().rev() {
        winutil::raise(hwnd_of(s.hwnd));
    }
}

/// The window to focus: the one that had the focus at capture, or else the
/// topmost. A minimized window is never in `stack`, so never focused.
pub(super) fn to_focus(stack: &[Stacked]) -> Option<isize> {
    let focused = stack.iter().find(|s| s.focused);
    focused.or_else(|| stack.iter().min_by_key(|s| s.z)).map(|s| s.hwnd)
}
