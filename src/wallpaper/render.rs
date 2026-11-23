//! Rendering one screen's picture with WIC (D796): the first frame, turned upright from its EXIF
//! orientation, into sRGB and premultiplied BGRA, scaled with high quality cubic, placed on a
//! canvas of the screen's size by `fit_rect` and `compose`, and saved as a PNG. Runs on the
//! `seam-wallpaper` worker only.

use super::span::{Cut, part};
use super::{Area, Fit, NO_CODEC, Piece, Placement, TOO_BIG, UNREADABLE, compose, fit_rect, too_big};
use std::path::Path;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, WINCODEC_ERR_COMPONENTNOTFOUND};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPVARIANT, PropVariantToUInt16};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::{HSTRING, w};

/// The EXIF colour space that is sRGB.
const EXIF_SRGB: u32 = 1;
/// A picture of up to this many pixels is kept decoded for the parts of a span (D806).
const KEEP_PIXELS: u64 = 64_000_000;

fn size(picture: &IWICBitmapSource) -> std::result::Result<(u32, u32), Failure> {
    let (mut w, mut h) = (0, 0);
    unsafe { picture.GetSize(&mut w, &mut h) }.map_err(Failure::of)?;
    Ok((w, h))
}

/// Why a picture couldn't be rendered: one of the strings the window shows, and for the log, the HRESULT.
#[derive(Debug)]
pub struct Failure {
    pub message: &'static str,
    pub hresult: Option<i32>,
}

impl Failure {
    fn of(e: windows::core::Error) -> Failure {
        Failure { message: UNREADABLE, hresult: Some(e.code().0) }
    }

    /// Opening `source` failed. No codec means the missing extension only for HEIC, HEIF and AVIF:
    /// for anything else it is content WIC doesn't know under a name it does.
    fn opening(e: windows::core::Error, source: &str) -> Failure {
        let ext = Path::new(source).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let store = ext.is_some_and(|ext| matches!(ext.as_str(), "heic" | "heif" | "avif"));
        let message = if e.code() == WINCODEC_ERR_COMPONENTNOTFOUND && store { NO_CODEC } else { UNREADABLE };
        Failure { message, hresult: Some(e.code().0) }
    }
}

/// WIC's factory, made once by the worker.
pub struct Wic(IWICImagingFactory);

/// What EXIF orientation 1 to 8 asks for, as WIC's flips and turns. WIC flips before it turns, so
/// a transpose (5) is a flip and a quarter turn left (`every_exif_orientation_comes_out_upright`).
fn upright(orientation: u16) -> WICBitmapTransformOptions {
    let turn = match orientation {
        2 => WICBitmapTransformFlipHorizontal.0,
        3 => WICBitmapTransformRotate180.0,
        4 => WICBitmapTransformFlipVertical.0,
        5 => WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0,
        6 => WICBitmapTransformRotate90.0,
        7 => WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0,
        8 => WICBitmapTransformRotate270.0,
        _ => WICBitmapTransformRotate0.0,
    };
    WICBitmapTransformOptions(turn)
}

impl Wic {
    pub fn open() -> windows::core::Result<Wic> {
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.map(Wic)
    }

    /// Render `source` for a screen of `size` with `fit`, bars in `bars` (0x00BBGGRR), into `out`,
    /// through a `.tmp` file so a half-written render is never used.
    pub fn render(
        &self,
        source: &str,
        size: (u32, u32),
        fit: Fit,
        bars: u32,
        out: &Path,
    ) -> std::result::Result<(), Failure> {
        let picture = self.upright_bgra(source)?;
        let (mut w, mut h) = (0, 0);
        unsafe { picture.GetSize(&mut w, &mut h) }.map_err(Failure::of)?;
        if too_big((w, h)) {
            return Err(Failure { message: TOO_BIG, hresult: None });
        }
        let placement = fit_rect((w, h), size, fit);
        let scaled: IWICBitmapSource = if placement.scaled == (w, h) {
            picture
        } else {
            let scaler = unsafe { self.0.CreateBitmapScaler() }.map_err(Failure::of)?;
            let (sw, sh) = placement.scaled;
            unsafe { scaler.Initialize(&picture, sw, sh, WICBitmapInterpolationModeHighQualityCubic) }
                .map_err(Failure::of)?;
            scaler.into()
        };
        let from = placement.piece.from;
        let rect = WICRect { X: from.x as i32, Y: from.y as i32, Width: from.w as i32, Height: from.h as i32 };
        let mut pixels = vec![0u8; from.w as usize * from.h as usize * 4];
        unsafe { scaled.CopyPixels(&rect, from.w * 4, &mut pixels) }.map_err(Failure::of)?;
        let [r, g, b, _] = bars.to_le_bytes();
        let canvas = compose(&placement, &pixels, [b, g, r]);
        drop(pixels);
        let tmp = out.with_extension("png.tmp");
        self.save_png(&canvas, size, &tmp).map_err(Failure::of)?;
        std::fs::rename(&tmp, out).map_err(|_| Failure { message: UNREADABLE, hresult: None })
    }

    /// A picture's size once upright, without decoding its pixels.
    pub fn size_of(&self, source: &str) -> std::result::Result<(u32, u32), Failure> {
        size(&self.upright_bgra(source)?)
    }

    /// `source` ready to be cut into parts (D806), and whether it is kept in memory: up to
    /// `KEEP_PIXELS` it is decoded once, and above that it is read again for each part, so memory
    /// stays bounded.
    pub fn decode(&self, source: &str) -> std::result::Result<(IWICBitmapSource, bool), Failure> {
        let picture = self.upright_bgra(source)?;
        let (w, h) = size(&picture)?;
        if too_big((w, h)) {
            return Err(Failure { message: TOO_BIG, hresult: None });
        }
        if w as u64 * h as u64 > KEEP_PIXELS {
            return Ok((picture, false));
        }
        let bitmap = unsafe { self.0.CreateBitmapFromSource(&picture, WICBitmapCacheOnLoad) }.map_err(Failure::of)?;
        Ok((bitmap.into(), true))
    }

    /// Render the part `cut` of `picture` for a screen of `screen` pixels (D805, D806), saved as a
    /// PNG through a `.tmp` file.
    pub fn render_part(
        &self,
        picture: &IWICBitmapSource,
        screen: (u32, u32),
        cut: Cut,
        bars: u32,
        out: &Path,
    ) -> std::result::Result<(), Failure> {
        let canvas = self.part_canvas(picture, screen, cut, bars)?;
        let tmp = out.with_extension("png.tmp");
        self.save_png(&canvas, screen, &tmp).map_err(Failure::of)?;
        std::fs::rename(&tmp, out).map_err(|_| Failure { message: UNREADABLE, hresult: None })
    }

    /// The screen's pixels for the part `cut` of `picture`: clipped, scaled to the screen with high
    /// quality cubic, and on bars where the cut leaves the picture.
    fn part_canvas(
        &self,
        picture: &IWICBitmapSource,
        screen: (u32, u32),
        cut: Cut,
        bars: u32,
    ) -> std::result::Result<Vec<u8>, Failure> {
        let (placement, pixels) = match part(size(picture)?, cut, screen) {
            Some((clip, placed)) => {
                let rect = WICRect { X: clip.x as i32, Y: clip.y as i32, Width: clip.w as i32, Height: clip.h as i32 };
                let clipper = unsafe { self.0.CreateBitmapClipper() }.map_err(Failure::of)?;
                unsafe { clipper.Initialize(picture, &rect) }.map_err(Failure::of)?;
                let clipped: IWICBitmapSource = clipper.into();
                let scaler = unsafe { self.0.CreateBitmapScaler() }.map_err(Failure::of)?;
                let (w, h) = placed.scaled;
                unsafe { scaler.Initialize(&clipped, w, h, WICBitmapInterpolationModeHighQualityCubic) }
                    .map_err(Failure::of)?;
                let mut pixels = vec![0u8; w as usize * h as usize * 4];
                unsafe { scaler.CopyPixels(std::ptr::null(), w * 4, &mut pixels) }.map_err(Failure::of)?;
                (placed, pixels)
            }
            // The cut is all bars.
            None => {
                let nothing = Piece { from: Area::default(), to: (0, 0) };
                (Placement { scaled: (0, 0), piece: nothing, tiled: false, screen }, Vec::new())
            }
        };
        let [r, g, b, _] = bars.to_le_bytes();
        Ok(compose(&placement, &pixels, [b, g, r]))
    }

    /// The first frame, upright, in sRGB, as premultiplied BGRA. GIF's first frame is its picture.
    fn upright_bgra(&self, source: &str) -> std::result::Result<IWICBitmapSource, Failure> {
        let decoder = unsafe {
            self.0.CreateDecoderFromFilename(&HSTRING::from(source), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
        }
        .map_err(|e| Failure::opening(e, source))?;
        let frame = unsafe { decoder.GetFrame(0) }.map_err(Failure::of)?;
        let orientation = self.orientation(&frame);
        let mut picture: IWICBitmapSource = frame.clone().into();
        if let Some(srgb) = self.to_srgb(&frame, &picture) {
            picture = srgb;
        }
        if orientation != WICBitmapTransformRotate0 {
            let turner = unsafe { self.0.CreateBitmapFlipRotator() }.map_err(Failure::of)?;
            unsafe { turner.Initialize(&picture, orientation) }.map_err(Failure::of)?;
            picture = turner.into();
        }
        let converter = unsafe { self.0.CreateFormatConverter() }.map_err(Failure::of)?;
        unsafe {
            converter.Initialize(
                &picture,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }
        .map_err(Failure::of)?;
        Ok(converter.into())
    }

    /// `System.Photo.Orientation`, as WIC's flips and turns. None in the file is upright.
    fn orientation(&self, frame: &IWICBitmapFrameDecode) -> WICBitmapTransformOptions {
        let read = || -> Option<u16> {
            let reader = unsafe { frame.GetMetadataQueryReader() }.ok()?;
            let mut value = PROPVARIANT::default();
            unsafe { reader.GetMetadataByName(w!("System.Photo.Orientation"), &mut value) }.ok()?;
            unsafe { PropVariantToUInt16(&value) }.ok()
        };
        upright(read().unwrap_or(1))
    }

    /// `picture` in sRGB, when the frame carries a colour profile that isn't sRGB already.
    fn to_srgb(&self, frame: &IWICBitmapFrameDecode, picture: &IWICBitmapSource) -> Option<IWICBitmapSource> {
        let mut count = 0u32;
        unsafe { frame.GetColorContexts(&mut [], &mut count) }.ok()?;
        if count == 0 {
            return None;
        }
        let made = (0..count).map(|_| unsafe { self.0.CreateColorContext() }.ok());
        let mut contexts: Vec<Option<IWICColorContext>> =
            made.collect::<Option<Vec<_>>>()?.into_iter().map(Some).collect();
        unsafe { frame.GetColorContexts(&mut contexts, &mut count) }.ok()?;
        let source = contexts.into_iter().next().flatten()?;
        let kind = unsafe { source.GetType() }.ok()?;
        if kind == WICColorContextExifColorSpace && unsafe { source.GetExifColorSpace() }.ok() == Some(EXIF_SRGB) {
            return None;
        }
        let srgb = unsafe { self.0.CreateColorContext() }.ok()?;
        unsafe { srgb.InitializeFromExifColorSpace(EXIF_SRGB) }.ok()?;
        let transform = unsafe { self.0.CreateColorTransformer() }.ok()?;
        unsafe { transform.Initialize(picture, &source, &srgb, &GUID_WICPixelFormat32bppBGRA) }.ok()?;
        Some(transform.into())
    }

    /// Opaque BGRA pixels of `size` as a PNG at `path`.
    fn save_png(&self, canvas: &[u8], (w, h): (u32, u32), path: &Path) -> windows::core::Result<()> {
        unsafe {
            let bitmap = self.0.CreateBitmapFromMemory(w, h, &GUID_WICPixelFormat32bppBGRA, w * 4, canvas)?;
            let stream = self.0.CreateStream()?;
            stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
            let encoder = self.0.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            let mut options: Option<IPropertyBag2> = None;
            encoder.CreateNewFrame(&mut frame, &mut options)?;
            let frame = frame.ok_or_else(windows::core::Error::empty)?;
            frame.Initialize(options.as_ref())?;
            frame.SetSize(w, h)?;
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(&bitmap, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;
            stream.Commit(windows::Win32::System::Com::STGC_DEFAULT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    /// A 2 by 3 picture whose pixels are numbered 0 to 5, turned for EXIF `orientation`: its size
    /// and its pixels' numbers, row by row.
    fn turned(orientation: u16) -> (u32, u32, Vec<u8>) {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let wic = Wic::open().expect("WIC");
        let mut pixels = vec![0u8; 24];
        for (i, pixel) in pixels.chunks_mut(4).enumerate() {
            pixel[0] = i as u8;
            pixel[3] = 255;
        }
        unsafe {
            let bitmap = wic.0.CreateBitmapFromMemory(2, 3, &GUID_WICPixelFormat32bppBGRA, 8, &pixels).unwrap();
            let turner = wic.0.CreateBitmapFlipRotator().unwrap();
            turner.Initialize(&bitmap, upright(orientation)).unwrap();
            let (mut w, mut h) = (0, 0);
            turner.GetSize(&mut w, &mut h).unwrap();
            let mut out = vec![0u8; (w * h * 4) as usize];
            turner.CopyPixels(std::ptr::null(), w * 4, &mut out).unwrap();
            (w, h, out.chunks(4).map(|p| p[0]).collect())
        }
    }

    #[test]
    fn a_part_is_clipped_scaled_and_put_on_bars() {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let wic = Wic::open().expect("WIC");
        // A 2 by 2 blue picture; the cut starts two pixels left of it, so the left half is bars.
        let blue = [255u8, 0, 0, 255].repeat(4);
        let bitmap = unsafe { wic.0.CreateBitmapFromMemory(2, 2, &GUID_WICPixelFormat32bppPBGRA, 8, &blue) }.unwrap();
        let picture: IWICBitmapSource = bitmap.into();
        let cut = Cut { x: -2, y: 0, w: 4, h: 2 };
        let canvas = wic.part_canvas(&picture, (4, 2), cut, 0x0000_00ff).expect("a canvas");
        let at = |x: usize, y: usize| <[u8; 4]>::try_from(&canvas[(y * 4 + x) * 4..][..4]).unwrap();
        // Red bars (0x00BBGGRR), then blue, as BGRA.
        assert_eq!([at(0, 0), at(1, 1)], [[0, 0, 255, 255]; 2]);
        assert_eq!([at(2, 0), at(3, 1)], [[255, 0, 0, 255]; 2]);
    }

    #[test]
    fn every_exif_orientation_comes_out_upright() {
        // The picture as stored is 0 1 / 2 3 / 4 5; each tag says how to show it.
        let want: [(u16, (u32, u32), [u8; 6]); 8] = [
            (1, (2, 3), [0, 1, 2, 3, 4, 5]),
            (2, (2, 3), [1, 0, 3, 2, 5, 4]),
            (3, (2, 3), [5, 4, 3, 2, 1, 0]),
            (4, (2, 3), [4, 5, 2, 3, 0, 1]),
            // Transpose, and transverse.
            (5, (3, 2), [0, 2, 4, 1, 3, 5]),
            (6, (3, 2), [4, 2, 0, 5, 3, 1]),
            (7, (3, 2), [5, 3, 1, 4, 2, 0]),
            (8, (3, 2), [1, 3, 5, 0, 2, 4]),
        ];
        for (orientation, size, pixels) in want {
            let (w, h, got) = turned(orientation);
            assert_eq!(((w, h), got.as_slice()), (size, pixels.as_slice()), "orientation {orientation}");
        }
    }
}
