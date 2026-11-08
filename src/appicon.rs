//! An app's icon as RGBA pixels, for the panel's workspace list. Called off the
//! panel's UI thread.

use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY};

/// Straight (not premultiplied) RGBA, top row first.
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The packaged app's logo for a Store app, else the exe's icon. `None` when the exe
/// is gone or the shell has nothing to show.
pub fn load(exe: &str, aumid: Option<&str>, size: i32) -> Option<Icon> {
    let packaged = exe.is_empty() || exe.to_lowercase().contains(r"\windowsapps\");
    let path = match aumid {
        Some(id) if packaged => format!(r"shell:AppsFolder\{id}"),
        _ if exe.is_empty() => return None,
        _ => exe.to_string(),
    };
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let icon = unsafe { shell_image(&path, size) };
    if init.is_ok() {
        unsafe { CoUninitialize() };
    }
    icon
}

unsafe fn shell_image(path: &str, size: i32) -> Option<Icon> {
    let item: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None::<&IBindCtx>).ok()?;
    let bitmap = item.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?;
    let icon = read_bitmap(bitmap);
    let _ = DeleteObject(HGDIOBJ(bitmap.0));
    icon
}

unsafe fn read_bitmap(bitmap: HBITMAP) -> Option<Icon> {
    let mut info = BITMAP::default();
    let got = GetObjectW(HGDIOBJ(bitmap.0), size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut _));
    if got == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
        return None;
    }
    let (width, height) = (info.bmWidth as u32, info.bmHeight as u32);
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative: top row first.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    let dc = GetDC(None);
    let lines = GetDIBits(dc, bitmap, 0, height, Some(bgra.as_mut_ptr() as *mut _), &mut header, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    if lines == 0 {
        return None;
    }
    Some(Icon { width, height, rgba: straight_rgba(bgra) })
}

/// BGRA from the shell is premultiplied; undo that and put red first. A bitmap with
/// no alpha at all is opaque.
fn straight_rgba(mut px: Vec<u8>) -> Vec<u8> {
    let opaque = px.chunks_exact(4).all(|p| p[3] == 0);
    for p in px.chunks_exact_mut(4) {
        let a = if opaque { 255 } else { p[3] as u32 };
        let un = |c: u8| if a == 0 || a == 255 { c } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
        let (b, g, r) = (un(p[0]), un(p[1]), un(p[2]));
        p.copy_from_slice(&[r, g, b, a as u8]);
    }
    px
}
