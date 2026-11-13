//! The zone overlay: click-through layered windows, painted into premultiplied
//! bitmaps. No blur and no animation.
//!
//! All of it lives on one overlay thread of its own: the windows, the bitmaps
//! and every window call. The tray's main thread only posts commands to it
//! (`build`, `show`, `hide`, `clear`), each a queue push and one wake-up, so a
//! drag never waits on pixels or on the window system. When zones are set up
//! the thread draws, ahead of time and one piece per turn, a base window per
//! screen with every zone and number, and a small lit window per zone. Showing
//! a zone is then a show of windows that already exist. A zone or span that
//! wasn't drawn yet is drawn when asked for, and a screen whose base isn't
//! ready yet shows as soon as it is, with nobody waiting for it. A preview adds
//! outlines where other windows will land and, while the drag's keys are live, a
//! strip naming them, drawn when asked for and gone when the drag ends.

use crate::{log, overlay};
use seam::zones::Rect;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::channel;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const CLASS: PCWSTR = w!("SeamZones");
const RADIUS: f32 = 6.;
const OUTLINE: f32 = 1.5;
/// Zone outlines and numbers, `#F3F3F3` at 40%, on a `#202020` fill at 35%.
const INK: (u8, u8, u8) = (0xF3, 0xF3, 0xF3);
const SHADE: (u8, u8, u8) = (0x20, 0x20, 0x20);
/// The seam colour, `#FFB020`.
const AMBER: (u8, u8, u8) = (0xFF, 0xB0, 0x20);
/// Where other windows will land, in the muted text colour `#949494`.
const MUTED: (u8, u8, u8) = (0x94, 0x94, 0x94);
/// The key strip: its text size, height, padding and distance from the bottom of the work area, at 96 DPI.
const HINT_TEXT: f32 = 13.;
const HINT_HEIGHT: f32 = 32.;
const HINT_PAD: f32 = 16.;
const HINT_BOTTOM: f32 = 24.;
const WM_APP_WAKE: u32 = WM_APP + 1;

type Key = (i32, i32, i32, i32);
type MaskKey = (String, i32, i32, i32);

fn key(r: &Rect) -> Key {
    (r.x, r.y, r.w, r.h)
}

/// What the overlay needs to know about one screen, in screen pixels.
#[derive(Clone, PartialEq)]
pub struct ScreenSpec {
    pub id: String,
    pub work: Rect,
    /// The screen's DPI over 96.
    pub scale: f32,
    pub zones: Vec<Rect>,
}

enum Command {
    Build(u64, Vec<ScreenSpec>),
    /// A screen, its lit rect, the share of it the window takes, outlines where other
    /// windows land, and the key strip's text.
    Preview(String, Option<Rect>, Option<Rect>, Vec<Rect>, Option<String>),
    Hide(String),
    HideAll,
    Clear,
}

struct Shared {
    queue: Mutex<VecDeque<Command>>,
    thread: u32,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static GENERATION: Mutex<u64> = Mutex::new(0);

/// The overlay thread's queue, starting the thread on first use. `None` if it couldn't start.
fn shared() -> Option<&'static Shared> {
    let shared = SHARED.get_or_init(|| {
        let (ready, wait) = channel();
        let spawned = std::thread::Builder::new().name("seam-zones".into()).spawn(move || {
            unsafe {
                let mut msg = MSG::default();
                // Creates this thread's queue, so posting to it can't fail once `ready` is sent.
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                let _ = ready.send(GetCurrentThreadId());
            }
            Overlay::default().run();
        });
        let thread = spawned.ok().and_then(|_| wait.recv().ok()).unwrap_or(0);
        Shared { queue: Mutex::new(VecDeque::new()), thread }
    });
    (shared.thread != 0).then_some(shared)
}

/// Hand the overlay thread one command. A queue push and, if it was idle, one post.
fn send(command: Command) {
    let Some(shared) = shared() else { return };
    let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
    let was_empty = queue.is_empty();
    queue.push_back(command);
    drop(queue);
    if was_empty {
        unsafe {
            let _ = PostThreadMessageW(shared.thread, WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Tell the overlay thread which screens have zones and where. Drawing starts
/// at once on that thread, and screens that haven't changed keep what they have.
pub fn build(screens: Vec<ScreenSpec>) {
    let generation = {
        let mut g = GENERATION.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        *g
    };
    send(Command::Build(generation, screens));
}

/// Show the overlay of screen `id`, with `lit` lit and `part` of it lit once more,
/// muted outlines on `others`, and `hint` in a strip at its bottom. If the screen's
/// drawing isn't ready it shows when it is. Returns at once.
pub fn preview(id: &str, lit: Option<Rect>, part: Option<Rect>, others: Vec<Rect>, hint: Option<String>) {
    send(Command::Preview(id.to_string(), lit, part, others, hint));
}

pub fn hide(id: &str) {
    send(Command::Hide(id.to_string()));
}

pub fn hide_all() {
    send(Command::HideAll);
}

/// Zones are off or there is nothing to show: free every window and bitmap.
pub fn clear() {
    if SHARED.get().is_some() {
        send(Command::Clear);
    }
}

/// One screen on the overlay thread.
struct Screen {
    spec: ScreenSpec,
    base: Option<HWND>,
    base_shown: bool,
    lit: HashMap<Key, HWND>,
    shown: Option<Key>,
    /// What was asked for before the base was drawn: show this once it is.
    want: Option<Option<Rect>>,
}

enum Unit {
    Base(String),
    Lit(String, usize),
}

#[derive(Default)]
struct Overlay {
    generation: u64,
    screens: HashMap<String, Screen>,
    /// Drawing still to do, one piece per turn so commands are answered in between.
    todo: VecDeque<Unit>,
    /// Digit coverage by label, glyph size and mask size.
    masks: HashMap<MaskKey, Vec<u8>>,
    /// The outlines shown now, by rect.
    others: Vec<(Key, HWND)>,
    /// The share of the lit zone shown now.
    part: Option<(Key, HWND)>,
    /// The key strip shown now: its screen, text and window.
    hint: Option<(String, String, HWND)>,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A hidden overlay window.
fn create() -> Option<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let styles = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        CreateWindowExW(styles, CLASS, w!("Seam zones"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst.into()), None)
            .ok()
    }
}

fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

fn show_on_top(hwnd: HWND) {
    unsafe {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
    }
}

impl Screen {
    fn destroy(self) {
        unsafe {
            for window in self.lit.into_values().chain(self.base) {
                let _ = DestroyWindow(window);
            }
        }
    }

    fn hide(&mut self) {
        self.want = None;
        if let Some(lit) = self.shown.take().and_then(|k| self.lit.get(&k)) {
            hide_window(*lit);
        }
        if let Some(base) = self.base.filter(|_| self.base_shown) {
            hide_window(base);
        }
        self.base_shown = false;
    }
}

impl Overlay {
    fn run(&mut self) {
        let mut msg = MSG::default();
        loop {
            if !self.drain() {
                return;
            }
            unsafe {
                // Windows messages first, so nothing is left waiting while pieces are drawn.
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                // A wake-up posted after `drain` took the queue was eaten by the loop
                // above, and the commands it was for would wait for the next window
                // message, while every later `send` saw a full queue and posted none.
                // So look at the queue before sleeping: a post after this look stays queued.
                let waiting =
                    SHARED.get().is_some_and(|s| !s.queue.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
                if waiting {
                    continue;
                }
                if self.todo.is_empty() {
                    // Asleep until the main thread posts, or a window message comes.
                    if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                } else {
                    self.draw_next();
                }
            }
        }
    }

    /// Carry out every queued command. False once the thread should end.
    fn drain(&mut self) -> bool {
        let Some(shared) = SHARED.get() else { return true };
        let commands = std::mem::take(&mut *shared.queue.lock().unwrap_or_else(|e| e.into_inner()));
        for command in commands {
            match command {
                Command::Build(generation, specs) => self.build(generation, specs),
                Command::Preview(id, lit, part, others, hint) => {
                    let raised = self.show(&id, lit);
                    self.share(&id, part, raised);
                    self.outline(&id, others);
                    self.strip(&id, hint);
                }
                Command::Hide(id) => {
                    if let Some(screen) = self.screens.get_mut(&id) {
                        screen.hide();
                    }
                    // Outlines can sit on any screen, and belong to the preview this hides.
                    self.outline("", Vec::new());
                    self.share("", None, false);
                    if self.hint.as_ref().is_some_and(|(on, ..)| *on == id) {
                        self.strip(&id, None);
                    }
                }
                Command::HideAll => {
                    self.screens.values_mut().for_each(Screen::hide);
                    self.outline("", Vec::new());
                    self.share("", None, false);
                    self.strip("", None);
                }
                Command::Clear => self.clear(),
            }
        }
        true
    }

    fn clear(&mut self) {
        self.outline("", Vec::new());
        self.share("", None, false);
        self.strip("", None);
        self.todo.clear();
        for (_, screen) in self.screens.drain() {
            screen.destroy();
        }
        self.masks = HashMap::new();
    }

    /// A newer description of the screens. What is already drawn for an
    /// unchanged screen stays, and drawing that was still to do is replaced.
    fn build(&mut self, generation: u64, specs: Vec<ScreenSpec>) {
        self.generation = generation;
        self.todo.clear();
        let wanted: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        let gone: Vec<String> = self.screens.keys().filter(|id| !wanted.contains(&id.as_str())).cloned().collect();
        for id in gone {
            if let Some(screen) = self.screens.remove(&id) {
                screen.destroy();
            }
        }
        for spec in specs {
            if self.screens.get(&spec.id).is_some_and(|s| s.spec == spec) {
                continue;
            }
            if let Some(old) = self.screens.remove(&spec.id) {
                old.destroy();
            }
            let blank = Screen {
                spec: spec.clone(),
                base: None,
                base_shown: false,
                lit: HashMap::new(),
                shown: None,
                want: None,
            };
            self.screens.insert(spec.id.clone(), blank);
        }
        // Bases first, because they decide when a drag can show anything.
        let ids: Vec<String> = self.screens.keys().cloned().collect();
        self.todo.extend(ids.iter().filter(|id| self.screens[*id].base.is_none()).map(|id| Unit::Base(id.clone())));
        for id in &ids {
            let zones = self.screens[id].spec.zones.len();
            self.todo.extend((0..zones).map(|i| Unit::Lit(id.clone(), i)));
        }
    }

    /// Draw one piece of what is still to do.
    fn draw_next(&mut self) {
        let Some(unit) = self.todo.pop_front() else { return };
        let started = log::enabled().then(Instant::now);
        match unit {
            Unit::Base(id) => {
                let Some(screen) = self.screens.get_mut(&id).filter(|s| s.base.is_none()) else { return };
                let Some(base) = create() else { return };
                let spec = screen.spec.clone();
                render(base, spec.work, |canvas| draw_base(canvas, &mut self.masks, &spec));
                let screen = self.screens.get_mut(&id).map(|s| s.base = Some(base));
                if screen.is_some() {
                    if let Some(started) = started {
                        let ms = started.elapsed().as_secs_f64() * 1e3;
                        log::info!(
                            "zone overlay: worker drew a {}x{} base in {ms:.1} ms (generation {})",
                            spec.work.w,
                            spec.work.h,
                            self.generation
                        );
                    }
                    // Something asked for it while it was being drawn.
                    if let Some(lit) = self.screens.get_mut(&id).and_then(|s| s.want.take()) {
                        self.show(&id, lit);
                    }
                }
            }
            Unit::Lit(id, zone) => {
                let Some(screen) = self.screens.get(&id) else { return };
                let Some(rect) = screen.spec.zones.get(zone).copied() else { return };
                if !screen.lit.contains_key(&key(&rect)) {
                    let scale = screen.spec.scale;
                    if let Some(window) = lit_window(rect, scale)
                        && let Some(screen) = self.screens.get_mut(&id)
                    {
                        screen.lit.insert(key(&rect), window);
                    }
                }
            }
        }
    }

    /// Whether it raised a lit window, which the share then goes back over.
    fn show(&mut self, id: &str, lit: Option<Rect>) -> bool {
        let started = log::enabled().then(Instant::now);
        let Some(screen) = self.screens.get_mut(id) else { return false };
        let Some(base) = screen.base else {
            // Not drawn yet: show it when it is.
            screen.want = Some(lit);
            return false;
        };
        if !screen.base_shown {
            unsafe {
                let _ = ShowWindow(base, SW_SHOWNOACTIVATE);
            }
            screen.base_shown = true;
        }
        let mut raised = false;
        let wanted = lit.as_ref().map(key);
        let mut made = false;
        if screen.shown != wanted {
            if let Some(previous) = screen.shown.and_then(|k| screen.lit.get(&k)) {
                hide_window(*previous);
            }
            if let (Some(zone), Some(k)) = (lit, wanted) {
                // A span's box isn't one of the zones, and any zone may still be waiting its turn.
                if let Entry::Vacant(slot) = screen.lit.entry(k)
                    && let Some(window) = lit_window(zone, screen.spec.scale)
                {
                    slot.insert(window);
                    made = true;
                }
                if let Some(window) = screen.lit.get(&k) {
                    show_on_top(*window);
                    raised = true;
                }
            }
            screen.shown = wanted;
        }
        if let Some(started) = started {
            let us = started.elapsed().as_secs_f64() * 1e6;
            log::info!("zone overlay: worker show {us:.0}us, lit window drawn: {made}");
        }
        raised
    }

    /// The share of the lit zone the window takes, in the lit style over it, so it
    /// shows stronger inside the whole zone. Kept while it is the same rect.
    /// A new rect redraws the same window there rather than making another, and it
    /// is raised only when it is new or `raised` put a lit zone over it.
    fn share(&mut self, id: &str, part: Option<Rect>, raised: bool) {
        let Some(rect) = part else {
            if let Some((_, window)) = self.part.take() {
                unsafe {
                    let _ = DestroyWindow(window);
                }
            }
            return;
        };
        let s = self.screens.get(id).map_or(1., |screen| screen.spec.scale);
        match self.part.as_mut() {
            Some((k, window)) => {
                if *k != key(&rect) {
                    paint_lit(*window, rect, s);
                    *k = key(&rect);
                }
                if raised {
                    show_on_top(*window);
                }
            }
            None => {
                if let Some(window) = lit_window(rect, s) {
                    show_on_top(window);
                    self.part = Some((key(&rect), window));
                }
            }
        }
    }

    /// Muted outlines on `rects`, drawn at the scale of screen `id`. The ones
    /// already up stay when the list is the same.
    fn outline(&mut self, id: &str, rects: Vec<Rect>) {
        let keys: Vec<Key> = rects.iter().map(key).collect();
        if self.others.iter().map(|(k, _)| *k).eq(keys.iter().copied()) {
            return;
        }
        for (_, window) in self.others.drain(..) {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        let s = self.screens.get(id).map_or(1., |screen| screen.spec.scale);
        for (rect, k) in rects.into_iter().zip(keys) {
            if let Some(window) = outline_window(rect, s) {
                show_on_top(window);
                self.others.push((k, window));
            }
        }
    }

    /// The key strip at the bottom of screen `id`, or none.
    fn strip(&mut self, id: &str, text: Option<String>) {
        if self.hint.as_ref().map(|(on, t, _)| (on.as_str(), t)) == text.as_ref().map(|t| (id, t)) {
            return;
        }
        if let Some((_, _, window)) = self.hint.take() {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        let Some(text) = text else { return };
        let Some(spec) = self.screens.get(id).map(|screen| &screen.spec) else { return };
        if let Some(window) = hint_window(spec.work, &text, spec.scale) {
            show_on_top(window);
            self.hint = Some((id.to_string(), text, window));
        }
    }
}

/// Premultiplied ARGB.
fn pm(rgb: (u8, u8, u8), alpha: f32) -> u32 {
    let c = |v: u8| (v as f32 * alpha).round() as u32;
    ((alpha * 255.).round() as u32) << 24 | c(rgb.0) << 16 | c(rgb.1) << 8 | c(rgb.2)
}

fn scale(px: u32, k: f32) -> u32 {
    let c = |shift: u32| (((px >> shift) & 0xFF) as f32 * k).round() as u32;
    c(24) << 24 | c(16) << 16 | c(8) << 8 | c(0)
}

/// `src` over `dst`, both premultiplied.
fn over(dst: u32, src: u32) -> u32 {
    let inverse = 255 - (src >> 24);
    let c = |shift: u32| {
        let (s, d) = ((src >> shift) & 0xFF, (dst >> shift) & 0xFF);
        (s + (d * inverse + 127) / 255).min(255)
    };
    c(24) << 24 | c(16) << 16 | c(8) << 8 | c(0)
}

/// An empty bitmap to draw on. Zones never overlap, so fills are stored, not blended.
struct Canvas<'a> {
    px: &'a mut [u32],
    w: i32,
    h: i32,
}

impl Canvas<'_> {
    fn blend(&mut self, x: i32, y: i32, src: u32) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let at = (y * self.w + x) as usize;
            self.px[at] = over(self.px[at], src);
        }
    }

    /// A rounded rectangle with an optional outline on its inside edge. Only
    /// the rim is measured pixel by pixel. The middle is stored in whole runs.
    fn rounded(&mut self, r: Rect, radius: f32, fill: u32, outline: Option<(u32, f32)>, s: f32) {
        let (radius, width) = (radius * s, outline.map_or(0., |o| o.1 * s));
        let (half_w, half_h) = (r.w as f32 / 2., r.h as f32 / 2.);
        let radius = radius.min(half_w).min(half_h);
        let (cx, cy) = (r.x as f32 + half_w, r.y as f32 + half_h);
        let distance = |x: i32, y: i32| {
            let qx = (x as f32 + 0.5 - cx).abs() - (half_w - radius);
            let qy = (y as f32 + 0.5 - cy).abs() - (half_h - radius);
            qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
        };
        let rim = (radius + width).ceil() as i32 + 2;
        let (left, right) = (r.x.max(0), (r.x + r.w).min(self.w));
        let (inner_left, inner_right) = ((r.x + rim).clamp(left, right), (r.x + r.w - rim).clamp(left, right));
        for y in r.y.max(0)..(r.y + r.h).min(self.h) {
            let near_y = y < r.y + rim || y >= r.y + r.h - rim;
            let row = (y * self.w) as usize;
            let spans = if !near_y && inner_left < inner_right {
                self.px[row + inner_left as usize..row + inner_right as usize].fill(fill);
                [left..inner_left, inner_right..right]
            } else {
                [left..right, 0..0]
            };
            for x in spans.into_iter().flatten() {
                let d = distance(x, y);
                let inside = (0.5 - d).clamp(0., 1.);
                if inside <= 0. {
                    continue;
                }
                self.blend(x, y, scale(fill, inside));
                if let Some((color, _)) = outline {
                    let ring = inside - (0.5 - (d + width)).clamp(0., 1.);
                    self.blend(x, y, scale(color, ring));
                }
            }
        }
    }

    /// Lay `label` in `ink` over the middle of `r`, using GDI only for the glyph shapes.
    fn label(&mut self, masks: &mut HashMap<MaskKey, Vec<u8>>, r: Rect, label: &str, ink: u32, s: f32) {
        let (mw, mh, size) = ((80. * s) as i32, (56. * s) as i32, (28. * s) as i32);
        let (left, top) = (r.x + (r.w - mw) / 2, r.y + (r.h - mh) / 2);
        let mask = masks
            .entry((label.to_string(), size, mw, mh))
            .or_insert_with(|| glyph_mask(label, size, mw, mh).unwrap_or_default());
        for (i, cover) in mask.iter().enumerate().filter(|(_, c)| **c > 0) {
            let (x, y) = (i as i32 % mw, i as i32 / mw);
            self.blend(left + x, top + y, scale(ink, *cover as f32 / 255.));
        }
    }
}

fn dib_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// How much of each pixel `label` covers, drawn white on black.
fn glyph_mask(label: &str, size: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bits = std::ptr::null_mut();
        let mut mask = None;
        if let Ok(bitmap) = CreateDIBSection(Some(dc), &dib_info(w, h), DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                let old = SelectObject(dc, bitmap.into());
                let flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
                let area = RECT { left: 0, top: 0, right: w, bottom: h };
                overlay::text(dc, label, area, size, false, (255, 255, 255), flags, false);
                let _ = GdiFlush();
                let pixels = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
                let cover = |p: &u32| ((((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF)) / 3) as u8;
                mask = Some(pixels.iter().map(cover).collect());
                SelectObject(dc, old);
            }
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        mask
    }
}

/// Draw into a bitmap the size of `area`, and make it `hwnd`'s picture there.
/// This neither shows nor hides the window.
fn render(hwnd: HWND, area: Rect, draw: impl FnOnce(&mut Canvas)) {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bits = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(dc), &dib_info(area.w, area.h), DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                let old = SelectObject(dc, bitmap.into());
                let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (area.w * area.h) as usize);
                draw(&mut Canvas { px: pixels, w: area.w, h: area.h });
                let _ = GdiFlush();
                let (at, size) = (POINT { x: area.x, y: area.y }, SIZE { cx: area.w, cy: area.h });
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    hwnd,
                    Some(screen),
                    Some(&at),
                    Some(&size),
                    Some(dc),
                    Some(&POINT::default()),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(dc, old);
            }
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}

/// A hidden window holding the lit look of `zone`.
fn lit_window(zone: Rect, s: f32) -> Option<HWND> {
    let hwnd = create()?;
    paint_lit(hwnd, zone, s);
    Some(hwnd)
}

/// Give `hwnd` the lit look of `zone`, there.
fn paint_lit(hwnd: HWND, zone: Rect, s: f32) {
    render(hwnd, zone, |canvas| {
        let area = Rect { x: 0, y: 0, ..zone };
        canvas.rounded(area, RADIUS, pm(AMBER, 0.1), Some((pm(AMBER, 1.), OUTLINE)), s);
    });
}

/// A hidden window with a muted outline around `rect`, where another window will land.
fn outline_window(rect: Rect, s: f32) -> Option<HWND> {
    let hwnd = create()?;
    render(hwnd, rect, |canvas| {
        let area = Rect { x: 0, y: 0, ..rect };
        canvas.rounded(area, RADIUS, 0, Some((pm(MUTED, 1.), OUTLINE)), s);
    });
    Some(hwnd)
}

/// A hidden window with `text` in a strip, centred at the bottom of `work`. The
/// text is drawn as wide as the screen allows and the strip is cut to its ink.
fn hint_window(work: Rect, text: &str, s: f32) -> Option<HWND> {
    let px = |v: f32| (v * s).round() as i32;
    let (size, h, pad) = (px(HINT_TEXT), px(HINT_HEIGHT), px(HINT_PAD));
    let room = work.w - 4 * pad;
    if room <= 0 {
        return None;
    }
    let mask = glyph_mask(text, size, room, h)?;
    let inked = |x: i32| (0..h).any(|y| mask[(y * room + x) as usize] > 0);
    let first = (0..room).find(|x| inked(*x))?;
    let last = (0..room).rev().find(|x| inked(*x))?;
    let w = last - first + 1 + 2 * pad;
    let at = Rect { x: work.x + (work.w - w) / 2, y: work.y + work.h - h - px(HINT_BOTTOM), w, h };
    let hwnd = create()?;
    render(hwnd, at, |canvas| {
        canvas.rounded(Rect { x: 0, y: 0, w, h }, RADIUS, pm(SHADE, 0.9), Some((pm(INK, 0.4), OUTLINE)), s);
        let ink = pm(INK, 1.);
        for y in 0..h {
            for x in first..=last {
                let cover = mask[(y * room + x) as usize];
                if cover > 0 {
                    canvas.blend(pad + x - first, y, scale(ink, cover as f32 / 255.));
                }
            }
        }
    });
    Some(hwnd)
}

/// The base: every zone and its number, in screen pixels turned local to the work area.
fn draw_base(canvas: &mut Canvas, masks: &mut HashMap<MaskKey, Vec<u8>>, spec: &ScreenSpec) {
    let (fill, outline, ink) = (pm(SHADE, 0.35), pm(INK, 0.4), pm(INK, 0.4));
    for (i, zone) in spec.zones.iter().enumerate() {
        let zone = Rect { x: zone.x - spec.work.x, y: zone.y - spec.work.y, ..*zone };
        canvas.rounded(zone, RADIUS, fill, Some((outline, OUTLINE)), spec.scale);
        canvas.label(masks, zone, &(i + 1).to_string(), ink, spec.scale);
    }
}
