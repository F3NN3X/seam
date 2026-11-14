//! The panel window: its keys, mouse and timers, and the thread that runs it.

use super::*;

/// The thumbnail under a point in the window's own pixels.
fn slot_at(o: &Overlay, hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let scale = scale_of(hwnd);
    let t = |v: i32| (v as f64 * scale).round() as i32;
    let (_, _, top) = frame(o, &t);
    (0..o.shown()).find(|slot| {
        let r = cell(*slot, top, &t);
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
}

fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn key(o: &mut Overlay, vk: u32) -> bool {
    let digit = |vk: u32| match vk {
        0x31..=0x39 => Some(vk as usize - 0x31),
        0x61..=0x69 => Some(vk as usize - 0x61),
        _ => None,
    };
    match vk {
        v if v == VK_ESCAPE.0 as u32 => quit(),
        v if v == VK_LEFT.0 as u32 => o.step(-1),
        v if v == VK_RIGHT.0 as u32 => o.step(1),
        v if v == VK_UP.0 as u32 => o.step(-(COLUMNS as isize)),
        v if v == VK_DOWN.0 as u32 => o.step(COLUMNS as isize),
        v if v == VK_RETURN.0 as u32 => o.pick(o.selected),
        v if v == u32::from(b'Z') && ctrl_down() => o.undo(),
        v => match digit(v) {
            Some(n) if o.first + n < o.choices.len() => o.pick(o.first + n),
            _ => return false,
        },
    }
    true
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let redraw = |changed: bool| {
            if changed {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        };
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                // Drawn off screen and copied over, so a change doesn't flicker.
                let back = CreateCompatibleDC(Some(dc));
                let bitmap = CreateCompatibleBitmap(dc, client.right, client.bottom);
                let old = SelectObject(back, bitmap.into());
                STATE.with(|s| {
                    if let Some(o) = s.borrow().as_ref() {
                        draw(back, o, client, scale_of(hwnd));
                    }
                });
                let _ = BitBlt(dc, 0, 0, client.right, client.bottom, Some(back), 0, 0, SRCCOPY);
                SelectObject(back, old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(back);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let used = STATE.with(|s| s.borrow_mut().as_mut().is_some_and(|o| key(o, wparam.0 as u32)));
                redraw(used);
                if used { LRESULT(0) } else { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_MOUSEMOVE => {
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                let moved = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    let Some(o) = s.as_mut() else { return false };
                    match slot_at(o, hwnd, x, y).map(|slot| o.first + slot) {
                        Some(at) if at != o.selected => {
                            o.selected = at;
                            true
                        }
                        _ => false,
                    }
                });
                redraw(moved);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                STATE.with(|s| {
                    if let Some(o) = s.borrow_mut().as_mut()
                        && let Some(slot) = slot_at(o, hwnd, x, y)
                    {
                        o.pick(o.first + slot);
                    }
                });
                redraw(true);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                quit();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_FOCUS => {
                let _ = KillTimer(Some(hwnd), TIMER_FOCUS);
                if GetForegroundWindow() != hwnd {
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetTimer(Some(hwnd), TIMER_FOCUS_LAST, FOCUS_LAST_MS, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_FOCUS_LAST => {
                let _ = KillTimer(Some(hwnd), TIMER_FOCUS_LAST);
                if GetForegroundWindow() != hwnd {
                    log::info!("quick layout: Windows kept the focus elsewhere, so it closes");
                    quit();
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_SETTLE => {
                STATE.with(|s| {
                    if let Some(o) = s.borrow_mut().as_mut() {
                        o.give_up();
                    }
                });
                redraw(true);
                LRESULT(0)
            }
            WM_ACTIVATE => {
                let inactive = (wparam.0 & 0xffff) as u32 == WA_INACTIVE;
                let was_active = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    let Some(o) = s.as_mut() else { return false };
                    let was = o.active;
                    o.active |= !inactive;
                    was
                });
                if inactive && was_active {
                    quit();
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// A click outside the panel closes it, and goes on to what it was for.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && matches!(wparam.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
            let at = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
            let hwnd = WINDOW.with(|w| *w.borrow());
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_ok()
                && !(at.x >= r.left && at.x < r.right && at.y >= r.top && at.y < r.bottom)
            {
                quit();
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Clears `OPEN` when the overlay's thread ends, however it ends.
struct Closing(u32);

impl Drop for Closing {
    fn drop(&mut self) {
        let _ = OPEN.compare_exchange(self.0, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// What the tray found: take it into the overlay.
fn take_settled() {
    for (screen, stamp, actual) in std::mem::take(&mut *locked(&SETTLED)) {
        STATE.with(|s| {
            if let Some(o) = s.borrow_mut().as_mut().filter(|o| o.screen == screen) {
                o.settle(stamp, actual);
            }
        });
    }
    let hwnd = WINDOW.with(|w| *w.borrow());
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn run(spec: Spec, generation: u32, ready: std::sync::mpsc::Sender<()>) {
    // Whatever an earlier overlay was left with isn't this one's.
    locked(&SETTLED).clear();
    locked(&DONE).clear();
    unsafe {
        let id = GetCurrentThreadId();
        let _closing = Closing(id);
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
        let created = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Seam quick layout"),
            WS_POPUP,
            spec.rect.left,
            spec.rect.top,
            1,
            1,
            None,
            None,
            Some(hinst.into()),
            None,
        );
        let Ok(hwnd) = created else {
            log::info!("quick layout: couldn't make the window");
            return;
        };
        let current = index_of(&spec.choices, spec.now.as_ref());
        let selected = current.unwrap_or(0);
        let mut overlay = Overlay {
            number: spec.number,
            shape: (spec.rect.width(), spec.rect.height()),
            note: spec.note,
            choices: spec.choices,
            now: spec.now,
            history: Vec::new(),
            selected,
            first: 0,
            tray: spec.tray,
            active: false,
            screen: spec.screen,
            generation,
            seq: 0,
            pending: false,
            record: None,
            undoing: false,
            queued: None,
        };
        overlay.reveal();
        // Sized after creation, because the window now knows its screen's DPI.
        let scale = scale_of(hwnd);
        let t = |v: i32| (v as f64 * scale).round() as i32;
        let (width, height, _) = frame(&overlay, &t);
        let x = spec.rect.left + (spec.rect.width() - width) / 2;
        let y = spec.rect.top + (spec.rect.height() - height) / 2;
        STATE.with(|s| *s.borrow_mut() = Some(overlay));
        WINDOW.with(|w| *w.borrow_mut() = hwnd);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_SHOWWINDOW);
        // The tray can use the overlay now, so it hears of it before focus is asked for.
        OPEN.store(id, Ordering::SeqCst);
        let _ = ready.send(());
        wincontrol::focus(hwnd, spec.from_api);
        let _ = SetTimer(Some(hwnd), TIMER_FOCUS, FOCUS_GRACE_MS, None);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).ok();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_APP_SETTLED {
                take_settled();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(hook) = hook {
            let _ = UnhookWindowsHookEx(hook);
        }
        let _ = DestroyWindow(hwnd);
    }
}

/// Open the overlay, replacing one that's up. It runs on a thread of its own, and
/// this returns once that thread has its window.
pub fn show(spec: Spec) {
    let _launching = locked(&LAUNCH);
    close();
    let (ready, opened) = channel();
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned =
        std::thread::Builder::new().name("seam-quicklayout".into()).spawn(move || run(spec, generation, ready));
    if let Err(e) = spawned {
        log::info!("quick layout: couldn't start: {e}");
        return;
    }
    let _ = opened.recv_timeout(Duration::from_millis(500));
}
