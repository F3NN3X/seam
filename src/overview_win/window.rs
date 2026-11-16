//! The overview's windows, one per screen: keys, mouse, a drag to a zone or screen,
//! and the thread that runs them.

use super::*;

/// The state, unless a call made while it is borrowed came back into the window procedure.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

fn redraw(s: &State) {
    announce(s);
    for p in &s.panes {
        let _ = unsafe { InvalidateRect(Some(p.hwnd), None, false) };
    }
}

/// Only these windows' tiles, in each screen that shows them.
fn redraw_tiles(s: &State, wins: &[usize]) {
    for p in &s.panes {
        for (_, r) in p.tiles.iter().filter(|(w, _)| wins.contains(w)) {
            let _ = unsafe { InvalidateRect(Some(p.hwnd), Some(&p.local(*r)), false) };
        }
    }
}

/// Tell a screen reader what is selected, or how many windows are shown: the focused
/// screen's window takes it as its name, and a name-change event has Narrator read it.
fn announce(s: &State) {
    let selected = s.selected.map(|w| (s.wins[w].app.as_str(), s.wins[w].title.as_str()));
    let said = overview::spoken(selected, s.shown.len());
    if *s.spoken.borrow() == said {
        return;
    }
    unsafe {
        let focus = GetFocus();
        let Some(p) = s.panes.iter().find(|p| p.hwnd == focus).or(s.panes.first()) else { return };
        let _ = SetWindowTextW(p.hwnd, &HSTRING::from(&said));
        NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, p.hwnd, OBJID_WINDOW.0, CHILDID_SELF as i32);
    }
    *s.spoken.borrow_mut() = said;
}

fn pane_of(s: &State, hwnd: HWND) -> Option<usize> {
    s.panes.iter().position(|p| p.hwnd == hwnd)
}

/// A point in a pane's own pixels, in screen pixels.
fn on_screen(s: &State, hwnd: HWND, lparam: LPARAM) -> Option<(i32, i32)> {
    let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
    let p = &s.panes[pane_of(s, hwnd)?];
    Some((p.rect.x + x, p.rect.y + y))
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(i32::from(vk.0)) < 0 }
}

/// Go to a window: restore it if it is minimized and give it the focus through the
/// window manager's input credit, as a stack's cycle does. One on another virtual
/// desktop takes Windows there. The overview closes whatever happens, also when the
/// window closed meanwhile.
fn go(s: &State, w: usize) {
    let h = s.wins[w].hwnd;
    if !unsafe { IsWindow(Some(h)) }.as_bool() {
        log::info!("overview: the window closed meanwhile");
        quit();
        return;
    }
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindowAsync(h, SW_RESTORE);
        }
    }
    wincontrol::focus(h, true);
    quit();
}

/// Esc, by `overview::escape`: end a drag, or close, giving the focus back to the window
/// that was in front while it still exists.
fn escape(s: &mut State) {
    let before = s.wins.iter().position(|w| w.hwnd.0 as isize == s.before);
    let alive = |w: usize| unsafe { IsWindow(Some(s.wins[w].hwnd)) }.as_bool();
    match overview::escape(s.drag.is_some(), before, alive) {
        Escape::CancelDrag => cancel_drag(s),
        Escape::Back(w) => go(s, w),
        Escape::Close => quit(),
    }
}

/// The search changed: lay the tiles out again for what it keeps.
fn searched(s: &mut State) {
    s.search();
}

fn key(s: &mut State, vk: u16) -> bool {
    s.notice = None;
    match VIRTUAL_KEY(vk) {
        VK_ESCAPE => escape(s),
        VK_RETURN => match s.selected {
            Some(w) => go(s, w),
            None => return false,
        },
        VK_LEFT => s.arrow(Dir::Left),
        VK_RIGHT => s.arrow(Dir::Right),
        VK_UP => s.arrow(Dir::Up),
        VK_DOWN => s.arrow(Dir::Down),
        VK_TAB if held(VK_CONTROL) => s.jump(if held(VK_SHIFT) { -1 } else { 1 }),
        VK_NEXT => s.jump(1),
        VK_PRIOR => s.jump(-1),
        VK_TAB => s.advance(if held(VK_SHIFT) { -1 } else { 1 }),
        VK_HOME => s.advance(isize::MIN),
        VK_END => s.advance(isize::MAX),
        VK_BACK if held(VK_CONTROL) => {
            s.query.clear();
            searched(s);
        }
        VK_BACK => {
            s.query.pop();
            searched(s);
        }
        _ => return false,
    }
    true
}

/// A typed character goes into the search. One outside the BMP comes as two UTF-16
/// units, the first kept until the second arrives.
fn typed(s: &mut State, unit: u32) -> bool {
    let unit = unit as u16;
    if (0xD800..0xDC00).contains(&unit) {
        s.surrogate = Some(unit);
        return false;
    }
    let units: Vec<u16> = s.surrogate.take().into_iter().chain([unit]).collect();
    let Some(Ok(c)) = char::decode_utf16(units).last() else { return false };
    if c.is_control() {
        return false;
    }
    s.query.push(c);
    searched(s);
    true
}

/// The screens and their zones, for what a drop lands on.
fn drop_targets(s: &State) -> Vec<(Rect, Vec<Rect>)> {
    s.panes.iter().map(|p| (p.rect, p.zones.clone())).collect()
}

/// What the pointer is over during a drag. The window's own screen, when it has no
/// zones, is no move.
fn over(s: &State, win: usize, at: (i32, i32)) -> Option<Landing> {
    match overview::drop_at(&drop_targets(s), at) {
        Some(Landing::Screen(i)) if i == s.wins[win].screen => None,
        other => other,
    }
}

fn start_drag(s: &mut State, hwnd: HWND, win: usize, at: (i32, i32)) {
    s.press = None;
    s.drag = Some(Drag { win, at, over: over(s, win, at) });
    draw::sync_thumbs(s);
    unsafe { SetCapture(hwnd) };
    redraw(s);
}

fn cancel_drag(s: &mut State) {
    if s.drag.take().is_some() {
        let _ = unsafe { ReleaseCapture() };
        draw::sync_thumbs(s);
        redraw(s);
    }
}

/// The drop: the window goes to its zone or screen through the snapping on the tray's
/// thread, gets the focus, and the overview closes. A minimized one is restored onto
/// its new place first, here, since that waits on its app.
fn drop_window(s: &mut State) {
    let Some(drag) = s.drag.take() else { return };
    let _ = unsafe { ReleaseCapture() };
    let win = &s.wins[drag.win];
    let (screen, zone, free) = match drag.over {
        Some(Landing::Zone(i, z)) => (i, Some(z), s.panes[i].zones[z]),
        Some(Landing::Screen(i)) => (i, None, overview::centred(s.panes[i].work, (win.frame.w, win.frame.h))),
        None => {
            draw::sync_thumbs(s);
            redraw(s);
            return;
        }
    };
    let p = &s.panes[screen];
    // A move to a screen keeps the window's real size: its pixels scale with the screens' DPI.
    let free = match zone {
        None => {
            let from = winutil::monitor_dpi(win.hwnd).max(96) as i32;
            let scale = |v: i32| v * p.dpi as i32 / from;
            overview::centred(p.work, (scale(win.frame.w), scale(win.frame.h)))
        }
        Some(_) => free,
    };
    if win.minimized && !winutil::is_hung(win.hwnd) {
        winutil::place_restored(win.hwnd, free, p.dpi);
    }
    locked(&MOVES).push(Move { hwnd: win.hwnd.0 as isize, screen: p.id.clone(), zone, free });
    unsafe {
        let _ = PostMessageW(Some(HWND(s.tray as *mut _)), WM_APP_OVERVIEW, WPARAM(0), LPARAM(0));
    }
    go(s, drag.win);
}

/// The wheel over a desktop's label moves between desktops. Its position is the screen's,
/// not the window's.
fn on_wheel(wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let delta = (wparam.0 >> 16) as u16 as i16;
    let at = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
    with_state(|s| {
        if delta != 0 && s.drag.is_none() && s.label_at(at) {
            // A touchpad sends many small turns: one jump per full notch.
            s.wheel += i32::from(delta);
            let notches = s.wheel / 120;
            s.wheel -= notches * 120;
            if notches != 0 {
                s.jump(if notches < 0 { 1 } else { -1 });
                redraw(s);
            }
        } else {
            s.wheel = 0;
        }
    });
    LRESULT(0)
}

fn on_mouse_move(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    with_state(|s| {
        let Some(at) = on_screen(s, hwnd, lparam) else { return };
        if let Some(drag) = &s.drag {
            let now = over(s, drag.win, at);
            if let Some(drag) = s.drag.as_mut() {
                drag.at = at;
                if drag.over != now {
                    drag.over = now;
                    redraw(s);
                }
            }
            return;
        }
        if let Some(press) = &s.press {
            let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            let far = (at.0 - press.at.0).abs() > dx || (at.1 - press.at.1).abs() > dy;
            if !far {
                return;
            }
            let (win, w) = (press.win, &s.wins[press.win]);
            // A window on another desktop can only be gone to (D572), and Windows doesn't let
            // Seam move one running as administrator.
            if w.hung {
                s.notice = Some(format!("{} isn't responding, so Seam can't move it", w.app));
                s.press = None;
                redraw(s);
            } else if w.elevated {
                s.notice = Some(format!("{} runs as administrator, so Seam can't move it", w.app));
                s.press = None;
                redraw(s);
            } else if w.desktop != 0 {
                s.notice = Some("A window on another desktop can't be moved from here".into());
                s.press = None;
                redraw(s);
            } else {
                start_drag(s, hwnd, win, at);
            }
            return;
        }
        // The pointer picks the tile under it once it moves, not when a tile appears under it.
        if s.pointer.is_some_and(|p| p != at)
            && let Some(w) = s.tile_at(at)
            && s.selected != Some(w)
        {
            s.selected = Some(w);
            s.notice = None;
            redraw(s);
        }
        s.pointer = Some(at);
    });
    LRESULT(0)
}

fn on_button_down(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    with_state(|s| {
        let Some(at) = on_screen(s, hwnd, lparam) else { return };
        s.notice = None;
        match s.tile_at(at) {
            Some(win) => {
                s.press = Some(Press { win, at });
                s.selected = Some(win);
                redraw(s);
            }
            // The header with its search field and the desktops' labels take no click.
            None if draw::in_header(s, hwnd, at) => {}
            // A click away from every tile closes, as one outside quick layout does.
            None => quit(),
        }
    });
    LRESULT(0)
}

fn on_button_up(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    with_state(|s| {
        if s.drag.is_some() {
            drop_window(s);
            return;
        }
        let Some(press) = s.press.take() else { return };
        if on_screen(s, hwnd, lparam).and_then(|at| s.tile_at(at)) == Some(press.win) {
            go(s, press.win);
        }
    });
    LRESULT(0)
}

unsafe fn on_paint(hwnd: HWND) -> LRESULT {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        // Drawn off screen and copied over, so a change doesn't flicker.
        let back = CreateCompatibleDC(Some(dc));
        let bitmap = CreateCompatibleBitmap(dc, client.right, client.bottom);
        let old = SelectObject(back, bitmap.into());
        with_state(|s| {
            let Some(i) = pane_of(s, hwnd) else { return };
            draw::paint(back, s, i, client);
            s.painted += 1;
            if s.painted == s.panes.len() {
                if log::enabled() {
                    let previews = s.wins.iter().filter(|w| w.thumb != 0).count();
                    log::info!(
                        "overview: first paint {} ms after the key (list {} ms), {} windows, {previews} previews, {} screens",
                        s.pressed.elapsed().as_millis(),
                        s.listed_ms,
                        s.wins.len(),
                        s.panes.len()
                    );
                }
                let _ = SetTimer(Some(hwnd), TIMER_DETAILS, 1, None);
            }
        });
        let _ = BitBlt(dc, 0, 0, client.right, client.bottom, Some(back), 0, 0, SRCCOPY);
        SelectObject(back, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(back);
        let _ = EndPaint(hwnd, &ps);
        LRESULT(0)
    }
}

/// After the first paint: the app names and icons that wait on other apps, a slice per
/// timer tick, so keys, clicks and paints are taken between slices.
fn on_details(hwnd: HWND) -> LRESULT {
    let done = with_state(|s| {
        let facts = |s: &State| -> Vec<_> {
            s.wins.iter().map(|w| (w.app.clone(), w.icon.map(|i| i.0 as isize), w.hung)).collect()
        };
        let before = facts(s);
        s.details_next = list::details(&mut s.wins, &mut s.names, s.details_next, DETAILS_SLICE);
        let changed = overview::changed(&before, &facts(s));
        if !changed.is_empty() {
            let shown = std::mem::take(&mut s.shown);
            // A new app name can change what the search keeps, and where.
            searched(s);
            match overview::repaint(&shown, &s.shown, &changed) {
                Repaint::Nothing => {}
                Repaint::Tiles(wins) => redraw_tiles(s, &wins),
                Repaint::All => redraw(s),
            }
            announce(s);
        }
        s.details_next >= s.wins.len()
    });
    if done != Some(false) {
        let _ = unsafe { KillTimer(Some(hwnd), TIMER_DETAILS) };
    }
    LRESULT(0)
}

unsafe fn on_focus_timer(hwnd: HWND, id: usize) -> LRESULT {
    unsafe {
        let _ = KillTimer(Some(hwnd), id);
        let ours = with_state(|s| s.panes.iter().any(|p| p.hwnd == GetForegroundWindow())).unwrap_or(true);
        match (ours, id) {
            (true, _) => {}
            (false, TIMER_FOCUS) => {
                let _ = SetForegroundWindow(hwnd);
                let _ = SetTimer(Some(hwnd), TIMER_FOCUS_LAST, FOCUS_LAST_MS, None);
            }
            (false, _) => {
                log::info!("overview: Windows kept the focus elsewhere, so it closes");
                quit();
            }
        }
        LRESULT(0)
    }
}

/// Another app took the focus: the overview closes. Moving between its own windows
/// isn't that, since they share a thread.
fn on_activate_app(wparam: WPARAM) -> LRESULT {
    let gone = with_state(|s| {
        let was = s.active;
        s.active |= wparam.0 != 0;
        wparam.0 == 0 && was
    });
    if gone == Some(true) {
        quit();
    }
    LRESULT(0)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => on_paint(hwnd),
            WM_ERASEBKGND => LRESULT(1),
            WM_KEYDOWN => {
                let used = with_state(|s| {
                    let used = key(s, wparam.0 as u16);
                    if used {
                        redraw(s);
                    }
                    used
                });
                if used == Some(true) { LRESULT(0) } else { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_CHAR => {
                with_state(|s| {
                    if typed(s, wparam.0 as u32) {
                        redraw(s);
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEWHEEL => on_wheel(wparam, lparam),
            WM_MOUSEMOVE => on_mouse_move(hwnd, lparam),
            WM_LBUTTONDOWN => on_button_down(hwnd, lparam),
            WM_LBUTTONUP => on_button_up(hwnd, lparam),
            // Lost to something else mid-drag: the drag ends where it was, with no move.
            WM_CAPTURECHANGED => {
                with_state(cancel_drag);
                LRESULT(0)
            }
            WM_TIMER if matches!(wparam.0, TIMER_FOCUS | TIMER_FOCUS_LAST) => on_focus_timer(hwnd, wparam.0),
            WM_ACTIVATEAPP => on_activate_app(wparam),
            WM_TIMER if wparam.0 == TIMER_DETAILS => on_details(hwnd),
            WM_CLOSE | WM_DISPLAYCHANGE => {
                quit();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Clears `OPEN` when the overview's thread ends, however it ends.
struct Closing(u32);

impl Drop for Closing {
    fn drop(&mut self) {
        let _ = OPEN.compare_exchange(self.0, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// One window per screen, not shown yet. None when one couldn't be made.
unsafe fn make_windows(screens: &[Screen]) -> Option<Vec<HWND>> {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let mut made = Vec::new();
        for s in screens {
            let r = rect_of(&s.rect);
            // A tool window has no taskbar button and stays out of Alt+Tab.
            let created = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS,
                w!("Seam overview"),
                WS_POPUP,
                r.x,
                r.y,
                r.w,
                r.h,
                None,
                None,
                Some(hinst.into()),
                None,
            );
            match created {
                Ok(hwnd) => made.push(hwnd),
                Err(_) => {
                    for hwnd in made {
                        let _ = DestroyWindow(hwnd);
                    }
                    return None;
                }
            }
        }
        Some(made)
    }
}

fn run(spec: Spec) {
    if spec.screens.is_empty() {
        OPENING.done();
        return;
    }
    unsafe {
        let id = GetCurrentThreadId();
        let _closing = Closing(id);
        // The queue exists before the tray hears of it, so a close is never lost.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        OPEN.store(id, Ordering::SeqCst);
        let com = list::com_start();
        let started = Instant::now();
        let rects: Vec<Rect> = spec.screens.iter().map(|s| rect_of(&s.rect)).collect();
        let (wins, desktops) = list::windows(&rects);
        let listed_ms = started.elapsed().as_millis();
        let Some(hwnds) = make_windows(&spec.screens) else {
            log::info!("overview: couldn't make its windows");
            OPENING.done();
            list::com_end(com);
            return;
        };
        let panes = spec
            .screens
            .into_iter()
            .zip(&hwnds)
            .map(|(s, &hwnd)| {
                let rect = rect_of(&s.rect);
                let dpi = dpi_of(hwnd);
                let work = winutil::work_area(rect).map_or(rect, |(work, _)| work);
                Pane {
                    hwnd,
                    number: s.number,
                    id: s.id,
                    rect,
                    work,
                    dpi,
                    zones: s.zones,
                    tiled: s.tiled,
                    labels: Vec::new(),
                    tiles: Vec::new(),
                }
            })
            .collect();
        let mut pointer = POINT::default();
        let _ = GetCursorPos(&mut pointer);
        let mut state = State {
            panes,
            wins,
            desktops,
            query: String::new(),
            shown: Vec::new(),
            selected: None,
            before: spec.before,
            tray: spec.tray,
            colours: colours(),
            press: None,
            drag: None,
            pointer: Some((pointer.x, pointer.y)),
            active: false,
            pressed: spec.pressed,
            painted: 0,
            listed_ms,
            notice: None,
            names: HashMap::new(),
            details_next: 0,
            surrogate: None,
            spoken: RefCell::default(),
            wheel: 0,
        };
        draw::register_thumbs(&mut state);
        state.search();
        // The window in front is where the selection starts.
        if let Some(w) = state.wins.iter().position(|w| w.hwnd.0 as isize == spec.before)
            && state.tiles().iter().any(|(x, _)| *x == w)
        {
            state.selected = Some(w);
        }
        let main = state.panes.iter().position(|p| p.rect.contains(pointer.x, pointer.y)).unwrap_or(0);
        let shown: Vec<(HWND, Rect)> = state.panes.iter().map(|p| (p.hwnd, p.rect)).collect();
        STATE.with(|s| *s.borrow_mut() = Some(state));
        for (i, (hwnd, r)) in shown.iter().enumerate() {
            let quiet = if i == main { SWP_SHOWWINDOW } else { SWP_SHOWWINDOW | SWP_NOACTIVATE };
            let _ = SetWindowPos(*hwnd, Some(HWND_TOPMOST), r.x, r.y, r.w, r.h, quiet);
        }
        // Its windows exist: a request for another overview is no longer dropped.
        OPENING.done();
        let main = shown[main].0;
        wincontrol::focus(main, spec.from_api);
        with_state(|s| announce(s));
        let _ = SetTimer(Some(main), TIMER_FOCUS, FOCUS_GRACE_MS, None);
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(mut state) = STATE.with(|s| s.borrow_mut().take()) {
            draw::unregister_thumbs(&mut state);
        }
        for hwnd in hwnds {
            let _ = DestroyWindow(hwnd);
        }
        list::com_end(com);
    }
}

/// Open the overview, replacing one that's up. It runs on a thread of its own and this
/// doesn't wait for it: until its windows exist `OPENING` is set, and a request made
/// meanwhile is dropped (D653).
pub fn show(spec: Spec) {
    if !OPENING.begin(now_ms()) {
        log::info!("overview: one is still opening, so this request is dropped");
        return;
    }
    close();
    let spawned = std::thread::Builder::new().name("seam-overview".into()).spawn(move || run(spec));
    if let Err(e) = spawned {
        OPENING.done();
        log::info!("overview: couldn't start: {e}");
    }
}
