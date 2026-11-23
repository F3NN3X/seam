//! Windows' side of the wallpaper, through `IDesktopWallpaper`: its ids for the monitors and which
//! of Seam's screens each one is (D792), what it shows (D797), setting a picture, and putting mine
//! back (D798). Every call is made on the `seam-wallpaper` worker, in its COM apartment.

use super::journal::{Picture, Record, Slideshow};
use crate::geom::Rect;
use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, RECT, S_OK};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DESKTOP_SLIDESHOW_OPTIONS, DESKTOP_WALLPAPER_POSITION, DSS_SLIDESHOW, DWPOS_FILL, DesktopWallpaper,
    IDesktopWallpaper, ILCreateFromPathW, ILFree, SHCreateShellItemArrayFromIDLists, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, Result, w};

/// `BackgroundType` lives here. Not documented (Microsoft Q&A points at it for Spotlight).
const WALLPAPERS_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Wallpapers");

/// Which of Windows' monitor ids is which screen (D792): the same device path in any case, else,
/// for the GDI fallback id or another string, the same rect. A detached id (`None` rect) is never
/// matched. Returns `(screen, Windows id)` for each screen that has one.
pub fn match_ids(screens: &[(&str, Rect)], windows: &[(String, Option<Rect>)]) -> Vec<(String, String)> {
    let attached: Vec<(&String, Rect)> = windows.iter().filter_map(|(id, r)| Some((id, (*r)?))).collect();
    let mut taken: Vec<&String> = Vec::new();
    let mut out = Vec::new();
    // By path first, so a rect never takes an id that names another screen.
    let by_path: Vec<Option<&String>> = screens
        .iter()
        .map(|(screen, _)| attached.iter().find(|(id, _)| id.eq_ignore_ascii_case(screen)).map(|(id, _)| *id))
        .collect();
    taken.extend(by_path.iter().flatten());
    for ((screen, rect), found) in screens.iter().zip(by_path) {
        let found = found.or_else(|| {
            let by_rect = attached.iter().find(|(id, r)| r == rect && !taken.contains(id)).map(|(id, _)| *id);
            taken.extend(by_rect);
            by_rect
        });
        if let Some(id) = found {
            out.push((screen.to_string(), id.clone()));
        }
    }
    out
}

fn take(text: PWSTR) -> String {
    let owned = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    owned
}

fn rect_of(r: RECT) -> Rect {
    Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

/// Windows' wallpaper, for one job.
pub struct Desktop(IDesktopWallpaper);

impl Desktop {
    /// Made again for each job, so an Explorer that restarted is found again.
    pub fn open() -> Result<Desktop> {
        unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL) }.map(Desktop)
    }

    /// Every monitor Windows has an id for, with its rect, or `None` for one that is detached
    /// (`GetMonitorRECT` answers `S_FALSE`).
    pub fn monitors(&self) -> Vec<(String, Option<Rect>)> {
        let count = unsafe { self.0.GetMonitorDevicePathCount() }.unwrap_or(0);
        let ids = (0..count).filter_map(|i| unsafe { self.0.GetMonitorDevicePathAt(i) }.ok().map(take));
        ids.filter(|id| !id.is_empty()).map(|id| (id.clone(), self.rect(&id))).collect()
    }

    fn rect(&self, id: &str) -> Option<Rect> {
        let id = HSTRING::from(id);
        let mut rect = RECT::default();
        // The wrapper reads S_FALSE as success, and S_FALSE is what says detached.
        let get = Interface::vtable(&self.0).GetMonitorRECT;
        let hr = unsafe { get(Interface::as_raw(&self.0), PCWSTR(id.as_ptr()), &mut rect) };
        (hr == S_OK).then(|| rect_of(rect))
    }

    /// The picture a monitor shows, "" for a solid colour.
    pub fn wallpaper(&self, id: &str) -> String {
        unsafe { self.0.GetWallpaper(&HSTRING::from(id)) }.map(take).unwrap_or_default()
    }

    pub fn set(&self, id: &str, path: &str) -> Result<()> {
        unsafe { self.0.SetWallpaper(&HSTRING::from(id), &HSTRING::from(path)) }
    }

    /// Fill, for every screen: each picture is already the screen's size (D791).
    pub fn fill(&self) -> Result<()> {
        unsafe { self.0.SetPosition(DWPOS_FILL) }
    }

    /// Windows' background colour, the colour of the bars (D796).
    pub fn background(&self) -> u32 {
        unsafe { self.0.GetBackgroundColor() }.map_or(0, |c| c.0)
    }

    /// What Windows shows now, to put it back later (D797). Pictures aren't copied here.
    pub fn record(&self) -> Record {
        let monitors = self.monitors().into_iter().map(|(id, _)| Picture { path: self.wallpaper(&id), id, copy: None });
        let status = unsafe { self.0.GetStatus() }.map_or(0, |s| s.0);
        let slideshow = (status & DSS_SLIDESHOW.0 != 0).then(|| self.slideshow()).flatten();
        Record {
            monitors: monitors.collect(),
            position: unsafe { self.0.GetPosition() }.map_or(DWPOS_FILL.0, |p| p.0),
            background: self.background(),
            status,
            slideshow,
            background_type: background_type(),
        }
    }

    fn slideshow(&self) -> Option<Slideshow> {
        let items = unsafe { self.0.GetSlideshow() }.ok()?;
        let count = unsafe { items.GetCount() }.ok()?;
        let path = |i| {
            let item = unsafe { items.GetItemAt(i) }.ok()?;
            Some(take(unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?))
        };
        let (mut options, mut tick) = (DESKTOP_SLIDESHOW_OPTIONS(0), 0u32);
        let _ = unsafe { self.0.GetSlideshowOptions(&mut options, &mut tick) };
        Some(Slideshow { items: (0..count).filter_map(path).collect(), options: options.0, tick })
    }

    /// Put mine back (D798): position, background colour, then the slideshow or each monitor's
    /// picture. `own` says what goes on a monitor, from the record.
    pub fn restore(&self, record: &Record, own: &dyn Fn(&str) -> Option<String>) -> Result<()> {
        unsafe {
            self.0.SetPosition(DESKTOP_WALLPAPER_POSITION(record.position))?;
            self.0.SetBackgroundColor(COLORREF(record.background))?;
        }
        if let Some(show) = record.slideshow.as_ref().filter(|s| !s.items.is_empty()) {
            return self.start_slideshow(show);
        }
        let mut last = Ok(());
        for picture in &record.monitors {
            let path = own(&picture.id).unwrap_or_default();
            if let Err(e) = self.set(&picture.id, &path) {
                last = Err(e);
            }
        }
        last
    }

    fn start_slideshow(&self, show: &Slideshow) -> Result<()> {
        let pidls: Vec<*mut ITEMIDLIST> = show
            .items
            .iter()
            .map(|p| unsafe { ILCreateFromPathW(&HSTRING::from(p.as_str())) })
            .filter(|p| !p.is_null())
            .collect();
        let list: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
        let items = unsafe { SHCreateShellItemArrayFromIDLists(&list) };
        for pidl in pidls {
            unsafe { ILFree(Some(pidl)) };
        }
        unsafe {
            self.0.SetSlideshow(&items?)?;
            self.0.SetSlideshowOptions(DESKTOP_SLIDESHOW_OPTIONS(show.options), show.tick)
        }
    }
}

/// `BackgroundType`, read for Spotlight.
fn background_type() -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            WALLPAPERS_KEY,
            w!("BackgroundType"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (code == ERROR_SUCCESS).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32) -> Rect {
        Rect::new(x, 0, 1920, 1080)
    }

    #[test]
    fn match_by_path_ignoring_case() {
        let screens = [(r"\\?\DISPLAY#AAA#1", rect(0)), (r"\\?\DISPLAY#BBB#2", rect(1920))];
        // Windows lists them the other way round, in another case.
        let windows =
            [(r"\\?\display#bbb#2".to_string(), Some(rect(1920))), (r"\\?\Display#Aaa#1".to_string(), Some(rect(0)))];
        let got = match_ids(&screens, &windows);
        assert_eq!(got[0], (screens[0].0.to_string(), windows[1].0.clone()));
        assert_eq!(got[1], (screens[1].0.to_string(), windows[0].0.clone()));
    }

    #[test]
    fn match_by_rect_when_paths_differ() {
        let screens = [(r"\\.\DISPLAY1", rect(0)), (r"\\?\DISPLAY#BBB#2", rect(1920))];
        let windows = [("other-a".to_string(), Some(rect(0))), (r"\\?\DISPLAY#BBB#2".to_string(), Some(rect(1920)))];
        let got = match_ids(&screens, &windows);
        assert_eq!(
            got,
            [(r"\\.\DISPLAY1".to_string(), "other-a".to_string()), (screens[1].0.into(), windows[1].0.clone())]
        );
        // A screen whose rect matches an id another screen has by path gets nothing.
        let same_rect = [(r"\\.\DISPLAY1", rect(1920)), (r"\\?\DISPLAY#BBB#2", rect(1920))];
        assert_eq!(match_ids(&same_rect, &windows).len(), 1);
    }

    #[test]
    fn detached_ids_are_skipped() {
        let screens = [(r"\\?\DISPLAY#AAA#1", rect(0))];
        let windows = [(r"\\?\DISPLAY#AAA#1".to_string(), None), ("unplugged".to_string(), None)];
        assert!(match_ids(&screens, &windows).is_empty());
    }
}
