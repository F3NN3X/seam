//! Keys, pointer drags and typed values on the Line Up screens.

use super::*;

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// Screen coordinate of the pointer along the current border's axis.
fn pointer_along(side: Side) -> i32 {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    if side.along_is_y() { p.y } else { p.x }
}

fn set_b(step: &mut Step, which: Which, base: (f64, f64), delta: f64) {
    match which {
        Which::Both => step.b_pts = (base.0 + delta, base.1 + delta),
        Which::One(0) => step.b_pts.0 = (base.0 + delta).min(step.b_pts.1 - MIN_GAP),
        Which::One(_) => step.b_pts.1 = (base.1 + delta).max(step.b_pts.0 + MIN_GAP),
    }
}

/// The map a typed offset gives, `None` if it isn't a number or a screen has no size.
fn typed_map(mons: &[Mon], step: &Step, typing: &Typing) -> Option<EdgeMap> {
    let cm: f64 = typing.text.replace(',', ".").parse().ok()?;
    let (mm_a, mm_b) = sizes(mons, step)?;
    map_from_offset(mm_a, mm_b, if typing.after { cm * 10.0 } else { -cm * 10.0 })
}

/// Put B's lines where `m` says, with A's lines inside the overlap it gives.
fn set_map(mons: &[Mon], step: &mut Step, m: EdgeMap) {
    let (la, lb) = (mons[step.a].rect.along_len(step.side), mons[step.b].rect.along_len(step.side));
    step.a_pts = anchor_points(&m, la, lb);
    step.b_pts = (m.apply(step.a_pts.0), m.apply(step.a_pts.1));
}

pub(super) fn handle(hwnd: HWND, idx: usize, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<(LRESULT, After)> {
    CALIB.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        let c = guard.as_mut()?;
        let cur = c.cur;
        let n_steps = c.steps.len();
        let step = &mut c.steps[cur];
        let is_b = idx == step.b;
        let side = step.side;
        // An unglued gap, or a border that isn't on the desk, has nothing to adjust.
        let live = step.adjustable();
        let mut after = After::Nothing;
        let mut changed = false;
        match msg {
            WM_LBUTTONDOWN if is_b && live => {
                let x = (lparam.0 & 0xffff) as i16 as f64;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as f64;
                let pos = if side.along_is_y() { y } else { x };
                let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f64;
                let reach = 24.0 * dpi / 96.0;
                let (d0, d1) = ((pos - step.b_pts.0).abs(), (pos - step.b_pts.1).abs());
                let which = if key_down(VK_CONTROL.0) {
                    Which::Both
                } else if d0.min(d1) <= reach {
                    Which::One(if d0 <= d1 { 0 } else { 1 })
                } else {
                    Which::Both
                };
                c.sel = match which {
                    Which::One(i) => i,
                    Which::Both => 2,
                };
                c.drag = Some(Drag { which, start: pointer_along(side), b0: step.b_pts });
                after = After::Capture(hwnd);
                changed = true;
            }
            WM_MOUSEMOVE => {
                if let Some(d) = &c.drag {
                    let delta = (pointer_along(side) - d.start) as f64;
                    let (which, b0) = (d.which, d.b0);
                    set_b(step, which, b0, delta);
                    changed = true;
                }
            }
            WM_LBUTTONUP => {
                c.drag = None;
                if unsafe { GetCapture() } == hwnd {
                    after = After::Release;
                }
            }
            WM_CAPTURECHANGED => {
                c.drag = None;
            }
            WM_CHAR if c.typing.is_some() => {
                let ch = char::from_u32(wparam.0 as u32).unwrap_or('\0');
                if let Some(t) = c.typing.as_mut() {
                    let separator = (ch == ',' || ch == '.') && !t.text.contains([',', '.']);
                    if (ch.is_ascii_digit() || separator) && t.text.len() < 8 {
                        t.text.push(ch);
                        changed = true;
                    }
                }
            }
            // Enter and Backspace mean next and back everywhere but in the field.
            WM_KEYDOWN if c.typing.is_some() => match VIRTUAL_KEY(wparam.0 as u16) {
                VK_ESCAPE => {
                    c.typing = None;
                    changed = true;
                }
                VK_TAB => {
                    if let Some(t) = c.typing.as_mut() {
                        t.after = !t.after;
                    }
                    changed = true;
                }
                VK_BACK => {
                    if let Some(t) = c.typing.as_mut() {
                        t.text.pop();
                    }
                    changed = true;
                }
                VK_RETURN => {
                    if let Some(m) = c.typing.as_ref().and_then(|t| typed_map(&c.mons, step, t)) {
                        set_map(&c.mons, step, m);
                        c.typing = None;
                        changed = true;
                    }
                }
                _ => return None,
            },
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wparam.0 as u16);
                let amount = if key_down(VK_SHIFT.0) { 10.0 } else { 1.0 };
                let nudge = match vk {
                    VK_UP | VK_LEFT => Some(-amount),
                    VK_DOWN | VK_RIGHT => Some(amount),
                    _ => None,
                };
                if let (Some(delta), true) = (nudge, live) {
                    let which = if key_down(VK_CONTROL.0) || c.sel == 2 { Which::Both } else { Which::One(c.sel) };
                    let base = step.b_pts;
                    set_b(step, which, base, delta);
                    changed = true;
                } else {
                    match vk {
                        VK_ESCAPE => after = After::Finish(false),
                        VK_RETURN => {
                            if cur + 1 < n_steps {
                                c.cur += 1;
                                c.sel = 0;
                                c.drag = None;
                                after = After::Focus(c.wins[c.steps[c.cur].b]);
                                changed = true;
                            } else {
                                after = After::Finish(true);
                            }
                        }
                        VK_BACK if cur > 0 => {
                            c.cur -= 1;
                            c.sel = 0;
                            c.drag = None;
                            after = After::Focus(c.wins[c.steps[c.cur].b]);
                            changed = true;
                        }
                        VK_G if step.glue.is_some() => {
                            step.glue = step.glue.map(|on| !on);
                            c.drag = None;
                            changed = true;
                        }
                        VK_N if step.glue.is_none() => {
                            step.not_on_desk = !step.not_on_desk;
                            c.drag = None;
                            changed = true;
                        }
                        VK_TAB if live => {
                            c.sel = (c.sel + 1) % 3;
                            changed = true;
                        }
                        VK_R if live => {
                            let m = step.guess;
                            set_map(&c.mons, step, m);
                            changed = true;
                        }
                        VK_M if c.measure && live && sizes(&c.mons, step).is_some() => {
                            c.typing = Some(Typing { text: String::new(), after: true });
                            c.drag = None;
                            changed = true;
                        }
                        _ => return None,
                    }
                }
            }
            WM_SETCURSOR => {
                let id = if is_b && live { if side.along_is_y() { IDC_SIZENS } else { IDC_SIZEWE } } else { IDC_ARROW };
                unsafe {
                    SetCursor(LoadCursorW(None, id).ok());
                }
                return Some((LRESULT(1), After::Nothing));
            }
            _ => return None,
        }
        if changed {
            push_live(c);
            redraw_all(c);
        }
        Some((LRESULT(0), after))
    })
}
