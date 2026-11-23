//! Rendering one screen's picture with WIC (D796): the first frame, turned upright from its EXIF
//! orientation, into sRGB and premultiplied BGRA, scaled with high quality cubic, placed on a
//! canvas of the screen's size by `fit_rect` and `compose`, and saved as a PNG. Runs on the
//! `seam-wallpaper` worker only.

use super::{Fit, NO_CODEC, TOO_BIG, UNREADABLE, compose, fit_rect, too_big};
use std::path::Path;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, WINCODEC_ERR_COMPONENTNOTFOUND};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPVARIANT, PropVariantToUInt16};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::{HSTRING, w};

/// The EXIF colour space that is sRGB.
const EXIF_SRGB: u32 = 1;

/// Why a picture couldn't be rendered: one of the strings the window shows, and for the log, the HRESULT.
#[derive(Debug)]
pub struct Failure {
    pub message: &'static str,
    pub hresult: Option<i32>,
}

impl Failure {
    fn of(e: windows::core::Error) -> Failure {
        let message = if e.code() == WINCODEC_ERR_COMPONENTNOTFOUND { NO_CODEC } else { UNREADABLE };
        Failure { message, hresult: Some(e.code().0) }
    }
}

/// WIC's factory, made once by the worker.
pub struct Wic(IWICImagingFactory);

/// What EXIF orientation 1 to 8 asks for, as WIC's flips and turns.
fn upright(orientation: u16) -> WICBitmapTransformOptions {
    let turn = match orientation {
        2 => WICBitmapTransformFlipHorizontal.0,
        3 => WICBitmapTransformRotate180.0,
        4 => WICBitmapTransformFlipVertical.0,
        5 => WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0,
        6 => WICBitmapTransformRotate90.0,
        7 => WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0,
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

    /// The first frame, upright, in sRGB, as premultiplied BGRA. GIF's first frame is its picture.
    fn upright_bgra(&self, source: &str) -> std::result::Result<IWICBitmapSource, Failure> {
        let decoder = unsafe {
            self.0.CreateDecoderFromFilename(&HSTRING::from(source), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
        }
        .map_err(Failure::of)?;
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
