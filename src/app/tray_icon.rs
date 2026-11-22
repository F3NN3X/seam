//! The tray icon, its tooltip and balloons; the menu is in `tray_icon/menu.rs`.

use super::*;
use windows::Win32::System::Registry::*;

mod menu;

pub(super) use menu::show_menu;

/// What the tray icon shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Look {
    On,
    /// Seam paused, or stepped aside for a full-screen app.
    Paused,
    /// A set of screens Seam would offer to line up.
    NeedsYou,
}

/// The icon for the state, decided in the order the status card in the sidebar does: paused
/// first, then a new screen, then stepped aside.
pub(super) fn look(paused: bool, new_screen: bool, stepped_aside: bool) -> Look {
    match () {
        _ if paused => Look::Paused,
        _ if new_screen => Look::NeedsYou,
        _ if stepped_aside => Look::Paused,
        _ => Look::On,
    }
}

/// The state in words, the same as the status card.
pub(super) fn words(paused: bool, new_screen: bool, stepped_aside: bool, remap: Remap) -> &'static str {
    match remap {
        _ if paused => "Remap paused",
        _ if new_screen => "New screen to line up",
        _ if stepped_aside => "Stepped aside",
        Remap::On => "Remap on",
        Remap::Paused => "Remap paused",
        Remap::OffNoBorders => "Not lined up",
    }
}

/// The icons built so far, one per state and taskbar theme, and the one the tray shows.
#[derive(Default)]
pub(super) struct TrayIcons {
    shown: Option<(Look, bool)>,
    current: HICON,
    built: Vec<((Look, bool), HICON)>,
    /// The taskbar's theme as last read, so the state changing doesn't read it again.
    light: bool,
    /// The tooltip last sent to the shell.
    tip: String,
}

/// Seam's state now, for the icon and for the words.
fn state_now(app: &App) -> (Look, &'static str) {
    let paused = standby::paused();
    let stepped_aside = standby::reason() == Some(standby::Standby::FullScreen);
    let new_screen = !paused && report::new_screen(app);
    let remap = Remap::of(app.cfg.enabled, app.links.len());
    (look(paused, new_screen, stepped_aside), words(paused, new_screen, stepped_aside, remap))
}

/// Whether the taskbar is light. Read only for the icon that rings its dot in the taskbar's
/// colour, when the state changes or Windows says the theme did.
fn taskbar_is_light() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    code == ERROR_SUCCESS && value != 0
}

/// Windows says the colours changed: the ring of the dot may need the other taskbar colour.
pub(super) fn theme_changed(app: &mut App) {
    app.tray.light = taskbar_is_light();
    if app.tray.shown.is_some_and(|(look, _)| look == Look::NeedsYou) {
        update_tip(app);
    }
}

const NEGATIVE_ON_DARK: u32 = 0xFFF07A7D;
const NEGATIVE_ON_LIGHT: u32 = 0xFFB91C1C;
const TASKBAR_DARK: u32 = 0xFF202020;
const TASKBAR_LIGHT: u32 = 0xFFF3F3F3;

/// The 32 x 32 pixels of the icon: two screens of different sizes with a line running straight
/// across. Paused is the same drawing in greys, with a slash from bottom left to top right;
/// needs you is the on icon with a `negative` dot at the top right, ringed in the taskbar's colour.
fn icon_pixels(look: Look, light: bool) -> Vec<u32> {
    const N: usize = 32;
    let mut px = vec![0u32; N * N];
    let mut rect = |x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                px[y * N + x] = c;
            }
        }
    };
    let (white, amber) = if look == Look::Paused { (0xFF949494, 0xFF5C5C5C) } else { (0xFFF2F4F8, 0xFFFFB020) };
    // Left (smaller) screen outline.
    rect(1, 10, 13, 11, white);
    rect(1, 22, 13, 23, white);
    rect(1, 10, 2, 23, white);
    rect(12, 10, 13, 23, white);
    // Right (bigger) screen outline.
    rect(16, 5, 30, 6, white);
    rect(16, 26, 30, 27, white);
    rect(16, 5, 17, 27, white);
    rect(29, 5, 30, 27, white);
    // The seam line, straight across both.
    rect(3, 16, 28, 17, amber);
    match look {
        Look::On => {}
        Look::Paused => {
            // The slash, three pixels across, in the outline's grey.
            for i in 0..N {
                for x in i.saturating_sub(1)..=(i + 1).min(N - 1) {
                    px[(N - 1 - i) * N + x] = white;
                }
            }
        }
        Look::NeedsYou => {
            let (dot, ring) = if light { (NEGATIVE_ON_LIGHT, TASKBAR_LIGHT) } else { (NEGATIVE_ON_DARK, TASKBAR_DARK) };
            // 9 px across, in a ring 2 px wide.
            let (cx, cy) = (25.5f32, 6.5f32);
            for y in 0..N {
                for x in 0..N {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    if d <= 4.5 {
                        px[y * N + x] = dot;
                    } else if d <= 6.5 {
                        px[y * N + x] = ring;
                    }
                }
            }
        }
    }
    px
}

/// Make the icon for a state and taskbar theme.
fn make_icon(look: Look, light: bool) -> HICON {
    const N: usize = 32;
    let px = icon_pixels(look, light);
    unsafe {
        let color = CreateBitmap(N as i32, N as i32, 1, 32, Some(px.as_ptr() as *const _));
        let mask_bits = [0u8; N * N / 8];
        let mask = CreateBitmap(N as i32, N as i32, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// Point the tray at the icon for Seam's state now, building it the first time. Returns
/// whether the icon changed.
fn sync_icon(app: &mut App) -> bool {
    let (look, _) = state_now(app);
    // The taskbar's colour only matters to the dot's ring.
    let key = (look, look == Look::NeedsYou && app.tray.light);
    if app.tray.shown == Some(key) {
        return false;
    }
    let icon = match app.tray.built.iter().find(|(k, _)| *k == key) {
        Some((_, icon)) => *icon,
        None => {
            let icon = make_icon(key.0, key.1);
            app.tray.built.push((key, icon));
            icon
        }
    };
    app.tray.current = icon;
    app.tray.shown = Some(key);
    true
}

fn tip_now(app: &App) -> String {
    format!("Seam — {}", state_now(app).1)
}

pub(super) fn tray_data(app: &App) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: app.hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: app.tray.current,
        ..Default::default()
    };
    nid.szTip = wide(&tip_now(app));
    nid
}

pub(super) fn add_icon(app: &mut App) {
    app.tray.light = taskbar_is_light();
    app.tray.shown = None;
    sync_icon(app);
    app.tray.tip = tip_now(app);
    unsafe {
        let mut nid = tray_data(app);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

/// The state may have changed: say it in the tooltip, and swap the icon only if it differs.
pub(super) fn update_tip(app: &mut App) {
    let changed = sync_icon(app);
    let tip = tip_now(app);
    if !changed && tip == app.tray.tip {
        return;
    }
    app.tray.tip = tip;
    unsafe {
        let mut nid = tray_data(app);
        if !changed {
            nid.uFlags &= !NIF_ICON;
        }
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Show a balloon/toast. Returns whether the shell accepted it.
pub(super) fn notify(app: &mut App, title: &str, body: &str, click: BalloonAction) -> bool {
    app.balloon_action = click;
    unsafe {
        let mut nid = tray_data(app);
        nid.uFlags |= NIF_INFO;
        nid.szInfoTitle = wide(title);
        nid.szInfo = wide(body);
        nid.dwInfoFlags = NIIF_INFO;
        Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
    }
}

/// Take down the balloon that is up, if the shell still shows it.
pub(super) fn clear_balloon(app: &mut App) {
    if app.balloon_action == BalloonAction::CancelSwitch {
        app.balloon_action = BalloonAction::None;
    }
    unsafe {
        let mut nid = tray_data(app);
        nid.uFlags |= NIF_INFO;
        nid.szInfoTitle = wide("");
        nid.szInfo = wide("");
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_beats_a_new_screen_which_beats_stepping_aside() {
        assert_eq!(look(true, true, true), Look::Paused);
        assert_eq!(look(false, true, true), Look::NeedsYou);
        assert_eq!(look(false, false, true), Look::Paused);
        assert_eq!(look(false, false, false), Look::On);
    }

    #[test]
    fn the_words_are_the_status_cards() {
        assert_eq!(words(false, false, false, Remap::On), "Remap on");
        assert_eq!(words(false, false, false, Remap::Paused), "Remap paused");
        assert_eq!(words(true, true, false, Remap::On), "Remap paused");
        assert_eq!(words(false, true, true, Remap::OffNoBorders), "New screen to line up");
        assert_eq!(words(false, false, true, Remap::On), "Stepped aside");
        assert_eq!(words(false, false, false, Remap::OffNoBorders), "Not lined up");
    }

    #[test]
    fn paused_is_slashed_and_grey() {
        let on = icon_pixels(Look::On, false);
        let paused = icon_pixels(Look::Paused, false);
        // The slash crosses the middle of the left screen's empty inside.
        assert_eq!(on[20 * 32 + 11], 0);
        assert_eq!(paused[20 * 32 + 11], 0xFF949494);
        assert_eq!(paused[16 * 32 + 3], 0xFF5C5C5C);
    }

    #[test]
    fn needs_you_has_a_dot_ringed_in_the_taskbar_colour() {
        let dark = icon_pixels(Look::NeedsYou, false);
        let light = icon_pixels(Look::NeedsYou, true);
        assert_eq!(dark[6 * 32 + 25], NEGATIVE_ON_DARK);
        assert_eq!(dark[6 * 32 + 20], TASKBAR_DARK);
        assert_eq!(light[6 * 32 + 25], NEGATIVE_ON_LIGHT);
        assert_eq!(light[6 * 32 + 20], TASKBAR_LIGHT);
        assert_eq!(dark[16 * 32 + 3], 0xFFFFB020);
    }
}
