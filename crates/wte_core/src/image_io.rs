// Owner: core-import (C5). Image decoding and encoding of ARCHITECTURE.md 5.22 (codec notes: 7.4). The only place
// of the workspace that talks to the `image` and `png` crates.

use std::io::Cursor;

use image::error::{ImageError, LimitErrorKind};
use image::{ExtendedColorType, ImageFormat, ImageReader, Limits};

use crate::error::ImageIoError;
use crate::pixel::PixelBuf;
use crate::pixel_ops::resize;

/// Largest image side `decode_image` accepts at all (larger files are `ImageIoError::TooLarge`).
const MAX_DECODE_SIDE: u32 = 16384;
/// Allocation budget of one decode.
const MAX_DECODE_ALLOC: u64 = 1 << 30;

/// Result of `decode_image`.
#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub pixels: PixelBuf,
    pub original_size: [u32; 2],
    pub resized: bool,
}

/// Extension of a file name or path, without the dot; `None` when there is none.
fn extension_of(name: &str) -> Option<&str> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = file.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() { None } else { Some(ext) }
}

/// A reader over `bytes` with its format decided: from the content, else from the extension of `name_hint`.
fn reader_for<'a>(bytes: &'a [u8], name_hint: Option<&str>) -> Result<ImageReader<Cursor<&'a [u8]>>, ImageIoError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImageIoError::Decode(e.to_string()))?;
    if reader.format().is_none() {
        let extension = name_hint.and_then(extension_of);
        match extension.and_then(ImageFormat::from_extension) {
            Some(format) => reader.set_format(format),
            None => {
                return Err(ImageIoError::UnsupportedFormat(match (name_hint, extension) {
                    (_, Some(ext)) => format!("unrecognized image data (.{ext})"),
                    (Some(name), None) if !name.is_empty() => format!("unrecognized image data ({name})"),
                    _ => "unrecognized image data".to_owned(),
                }));
            }
        }
    }
    Ok(reader)
}

fn decode_error(error: ImageError, size: Option<[u32; 2]>) -> ImageIoError {
    match error {
        ImageError::Unsupported(e) => ImageIoError::UnsupportedFormat(e.to_string()),
        ImageError::Limits(e) => match (e.kind(), size) {
            (LimitErrorKind::DimensionError | LimitErrorKind::InsufficientMemory, Some([width, height])) => {
                ImageIoError::TooLarge { width, height }
            }
            _ => ImageIoError::Decode(e.to_string()),
        },
        other => ImageIoError::Decode(other.to_string()),
    }
}

/// Decodes any supported format to straight RGBA8. Format is guessed from content, then from the extension of
/// `name_hint` (needed for TGA). Images larger than `max_dim` on a side are downscaled (aspect preserved).
///
/// Supported: PNG, JPEG, TGA, BMP, TIFF, WebP and DDS (DXT1/3/5 only). Errors: `Empty` for a zero-size image,
/// `TooLarge` beyond 16384 px per side or 1 GiB of decoded data, `UnsupportedFormat` for unknown or disabled
/// formats and DDS variants, `Decode` for everything else. `max_dim == 0` is treated as 1. After a downscale the
/// longer side is exactly `max_dim`, `resized` is set and `original_size` holds the size found in the file.
pub fn decode_image(bytes: &[u8], name_hint: Option<&str>, max_dim: u32) -> Result<DecodedImage, ImageIoError> {
    if bytes.is_empty() {
        return Err(ImageIoError::Decode("the file is empty".to_owned()));
    }
    // Pass 1: the header only, so that size problems are reported with the real dimensions.
    let mut probe = reader_for(bytes, name_hint)?;
    let mut probe_limits = Limits::default();
    probe_limits.max_alloc = Some(MAX_DECODE_ALLOC);
    probe.limits(probe_limits);
    let (width, height) = probe.into_dimensions().map_err(|e| decode_error(e, None))?;
    if width == 0 || height == 0 {
        return Err(ImageIoError::Empty);
    }
    if width > MAX_DECODE_SIDE || height > MAX_DECODE_SIDE {
        return Err(ImageIoError::TooLarge { width, height });
    }

    // Pass 2: the pixels.
    let mut reader = reader_for(bytes, name_hint)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let rgba = reader.decode().map_err(|e| decode_error(e, Some([width, height])))?.into_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(ImageIoError::Empty);
    }
    let pixels = PixelBuf::from_raw(width, height, rgba.into_raw())
        .ok_or_else(|| ImageIoError::Decode("decoder returned a buffer of the wrong size".to_owned()))?;

    let max_dim = max_dim.max(1);
    if width <= max_dim && height <= max_dim {
        return Ok(DecodedImage { pixels, original_size: [width, height], resized: false });
    }
    let scaled = |side: u32, longest: u32| -> u32 {
        ((side as u64 * max_dim as u64 + longest as u64 / 2) / longest as u64).clamp(1, max_dim as u64) as u32
    };
    let longest = width.max(height);
    let (new_w, new_h) = (scaled(width, longest), scaled(height, longest));
    Ok(DecodedImage { pixels: resize(&pixels, new_w, new_h), original_size: [width, height], resized: true })
}

/// PNG compression effort.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PngLevel {
    Fast,
    Balanced,
    High,
}

/// `Err(Empty)` for a zero-size image, `Err(Encode)` when `len` is not `width * height * 4`.
fn check_rgba(width: u32, height: u32, len: usize) -> Result<(), ImageIoError> {
    if width == 0 || height == 0 {
        return Err(ImageIoError::Empty);
    }
    let expected = width as u64 * height as u64 * 4;
    if len as u64 != expected {
        return Err(ImageIoError::Encode(format!(
            "pixel data has {len} bytes, expected {expected} for {width}x{height} RGBA"
        )));
    }
    Ok(())
}

/// RGBA8, non-interlaced, straight alpha. `rgba.len() == width*height*4`.
///
/// The bytes are stored as given (RGB under alpha 0 is kept). `Err(Empty)` for a zero-size image, `Err(Encode)`
/// for a wrong buffer length or an encoder failure.
pub fn encode_png(width: u32, height: u32, rgba: &[u8], level: PngLevel) -> Result<Vec<u8>, ImageIoError> {
    check_rgba(width, height, rgba.len())?;
    let encode_error = |e: png::EncodingError| ImageIoError::Encode(e.to_string());
    let mut out = Vec::with_capacity(rgba.len() / 4 + 1024);
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(match level {
        PngLevel::Fast => png::Compression::Fast,
        PngLevel::Balanced => png::Compression::Balanced,
        PngLevel::High => png::Compression::High,
    });
    let mut writer = encoder.write_header().map_err(encode_error)?;
    writer.write_image_data(rgba).map_err(encode_error)?;
    writer.finish().map_err(encode_error)?;
    Ok(out)
}

/// 8-bit greyscale (`gray.len() == width*height`), non-interlaced (STICKER_PLAN 2.11: sticker layer masks). `Err(Empty)`
/// for a zero-size image, `Err(Encode)` for a wrong buffer length or an encoder failure (as `encode_png`).
pub fn encode_png_gray(width: u32, height: u32, gray: &[u8], level: PngLevel) -> Result<Vec<u8>, ImageIoError> {
    if width == 0 || height == 0 {
        return Err(ImageIoError::Empty);
    }
    let expected = width as u64 * height as u64;
    if gray.len() as u64 != expected {
        return Err(ImageIoError::Encode(format!(
            "pixel data has {} bytes, expected {expected} for {width}x{height} grey",
            gray.len()
        )));
    }
    let encode_error = |e: png::EncodingError| ImageIoError::Encode(e.to_string());
    let mut out = Vec::with_capacity(gray.len() / 4 + 1024);
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(match level {
        PngLevel::Fast => png::Compression::Fast,
        PngLevel::Balanced => png::Compression::Balanced,
        PngLevel::High => png::Compression::High,
    });
    let mut writer = encoder.write_header().map_err(encode_error)?;
    writer.write_image_data(gray).map_err(encode_error)?;
    writer.finish().map_err(encode_error)?;
    Ok(out)
}

/// RGB8 (tightly packed, `rgb.len() == width*height*3`) as baseline JPEG with `quality` clamped to 1..=100 (branch
/// `companion`: the picture tiles of the tablet companion, COMPANION_PLAN 2.4). `Err(Empty)` for a zero-size image,
/// `Err(Encode)` for a wrong buffer length or an encoder failure (as `encode_png`).
pub fn encode_jpeg(width: u32, height: u32, rgb: &[u8], quality: u8) -> Result<Vec<u8>, ImageIoError> {
    if width == 0 || height == 0 {
        return Err(ImageIoError::Empty);
    }
    let expected = width as u64 * height as u64 * 3;
    if rgb.len() as u64 != expected {
        return Err(ImageIoError::Encode(format!(
            "pixel data has {} bytes, expected {expected} for {width}x{height} RGB",
            rgb.len()
        )));
    }
    let mut out = Vec::with_capacity(rgb.len() / 8 + 1024);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode(rgb, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| ImageIoError::Encode(e.to_string()))?;
    Ok(out)
}

/// File format of a flattened texture export.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum FlatFormat {
    Png(PngLevel),
    Jpeg { quality: u8, matte: [u8; 3] },
    Tga,
    Bmp,
    Tiff,
    WebpLossless,
}

impl FlatFormat {
    /// File extension without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            FlatFormat::Png(_) => "png",
            FlatFormat::Jpeg { .. } => "jpg",
            FlatFormat::Tga => "tga",
            FlatFormat::Bmp => "bmp",
            FlatFormat::Tiff => "tif",
            FlatFormat::WebpLossless => "webp",
        }
    }

    /// True when the format stores the alpha channel.
    pub fn keeps_alpha(&self) -> bool {
        !matches!(self, FlatFormat::Jpeg { .. })
    }
}

/// `rgba` composited over the opaque `matte` colour: `c' = (c*a + m*(255 - a) + 127) / 255`, tightly packed RGB8.
fn matte_rgb(rgba: &[u8], matte: [u8; 3]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.as_chunks::<4>().0 {
        let a = px[3] as u32;
        for c in 0..3 {
            rgb.push(((px[c] as u32 * a + matte[c] as u32 * (255 - a) + 127) / 255) as u8);
        }
    }
    rgb
}

/// Encodes a flattened texture in the given format.
///
/// PNG, TGA, BMP, TIFF and lossless WebP store the RGBA bytes unchanged; JPEG stores the image composited over
/// `matte` with `quality` clamped to 1..=100. `Err(Empty)` for a zero-size image, `Err(Encode)` for a buffer whose
/// length does not match its size or an encoder failure.
pub fn encode_flat(buf: &PixelBuf, format: &FlatFormat) -> Result<Vec<u8>, ImageIoError> {
    let (width, height) = (buf.width, buf.height);
    check_rgba(width, height, buf.data.len())?;
    let encode_error = |e: ImageError| ImageIoError::Encode(e.to_string());
    let container = match *format {
        FlatFormat::Png(level) => return encode_png(width, height, &buf.data, level),
        FlatFormat::Jpeg { quality, matte } => {
            let rgb = matte_rgb(&buf.data, matte);
            let mut out = Vec::with_capacity(rgb.len() / 4 + 1024);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .encode(&rgb, width, height, ExtendedColorType::Rgb8)
                .map_err(encode_error)?;
            return Ok(out);
        }
        FlatFormat::Tga => ImageFormat::Tga,
        FlatFormat::Bmp => ImageFormat::Bmp,
        FlatFormat::Tiff => ImageFormat::Tiff,
        FlatFormat::WebpLossless => ImageFormat::WebP,
    };
    let mut out = Cursor::new(Vec::with_capacity(buf.data.len() / 2 + 1024));
    image::write_buffer_with_format(&mut out, &buf.data, width, height, ExtendedColorType::Rgba8, container)
        .map_err(encode_error)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;

    const ALL_FORMATS: [FlatFormat; 8] = [
        FlatFormat::Png(PngLevel::Fast),
        FlatFormat::Png(PngLevel::Balanced),
        FlatFormat::Png(PngLevel::High),
        FlatFormat::Jpeg { quality: 90, matte: [255, 255, 255] },
        FlatFormat::Tga,
        FlatFormat::Bmp,
        FlatFormat::Tiff,
        FlatFormat::WebpLossless,
    ];

    /// Random RGBA noise with every kind of alpha, including RGB under alpha 0.
    fn noise(width: u32, height: u32, seed: u64) -> PixelBuf {
        let mut rng = XorShift::new(seed);
        let mut buf = PixelBuf::new(width, height);
        for (i, px) in buf.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = rng.next_u32().to_le_bytes();
            let alpha = match i % 4 {
                0 => 0,
                1 => 255,
                _ => v[3],
            };
            *px = [v[0], v[1], v[2], alpha];
        }
        buf
    }

    /// A smooth opaque-ish gradient (JPEG-friendly).
    fn gradient(width: u32, height: u32) -> PixelBuf {
        let mut buf = PixelBuf::new(width, height);
        for y in 0..height {
            for x in 0..width {
                buf.set(x, y, [(x * 255 / width) as u8, (y * 255 / height) as u8, 128, 255]);
            }
        }
        buf
    }

    fn name_for(format: &FlatFormat) -> String {
        format!("flat.{}", format.extension())
    }

    #[test]
    fn extensions_and_alpha_support() {
        let ext: Vec<&str> = ALL_FORMATS.iter().map(|f| f.extension()).collect();
        assert_eq!(ext, ["png", "png", "png", "jpg", "tga", "bmp", "tif", "webp"]);
        for format in &ALL_FORMATS {
            assert_eq!(format.keeps_alpha(), !matches!(format, FlatFormat::Jpeg { .. }), "{format:?}");
            // Every extension is one the decoder side understands.
            assert!(ImageFormat::from_extension(format.extension()).is_some());
        }
    }

    #[test]
    fn every_flat_format_round_trips() {
        let src = noise(37, 21, 7);
        for format in &ALL_FORMATS {
            let bytes = encode_flat(&src, format).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            let name = name_for(format);
            let decoded = decode_image(&bytes, Some(&name), 8192).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!(decoded.original_size, [37, 21], "{format:?}");
            assert!(!decoded.resized);
            assert_eq!((decoded.pixels.width, decoded.pixels.height), (37, 21), "{format:?}");
            if format.keeps_alpha() {
                // Lossless: exact bytes, including RGB under alpha 0.
                assert_eq!(decoded.pixels, src, "{format:?} must be lossless");
            } else {
                let opaque = decoded.pixels.data.as_chunks::<4>().0.iter().all(|px| px[3] == 255);
                assert!(opaque, "{format:?} is opaque");
            }
        }
    }

    #[test]
    fn formats_with_magic_bytes_need_no_name_hint() {
        let src = noise(8, 8, 3);
        for format in &ALL_FORMATS {
            let bytes = encode_flat(&src, format).unwrap();
            let decoded = decode_image(&bytes, None, 8192);
            if matches!(format, FlatFormat::Tga) {
                assert!(matches!(decoded, Err(ImageIoError::UnsupportedFormat(_))), "TGA has no magic bytes");
            } else {
                assert_eq!(decoded.unwrap().original_size, [8, 8], "{format:?}");
            }
        }
    }

    #[test]
    fn tga_is_detected_by_extension() {
        let src = noise(16, 9, 11);
        let bytes = encode_flat(&src, &FlatFormat::Tga).unwrap();
        assert_eq!(decode_image(&bytes, Some("skin.tga"), 8192).unwrap().pixels, src);
        assert_eq!(decode_image(&bytes, Some("C:\\Textures.v2\\SKIN.TGA"), 8192).unwrap().pixels, src);
        // A wrong or missing extension cannot identify the data.
        assert!(matches!(decode_image(&bytes, Some("skin"), 8192), Err(ImageIoError::UnsupportedFormat(_))));
        assert!(matches!(decode_image(&bytes, Some("skin.xyz"), 8192), Err(ImageIoError::UnsupportedFormat(_))));
        // The content wins over a misleading extension.
        let png = encode_flat(&src, &FlatFormat::Png(PngLevel::Fast)).unwrap();
        assert_eq!(decode_image(&png, Some("really_a_png.tga"), 8192).unwrap().pixels, src);
    }

    #[test]
    fn jpeg_is_matted() {
        // Left half opaque red, right half fully transparent green: the matte must replace the transparent part.
        let mut src = PixelBuf::new(32, 16);
        for y in 0..16 {
            for x in 0..32 {
                src.set(x, y, if x < 16 { [200, 0, 0, 255] } else { [0, 255, 0, 0] });
            }
        }
        let bytes = encode_flat(&src, &FlatFormat::Jpeg { quality: 95, matte: [10, 20, 240] }).unwrap();
        let out = decode_image(&bytes, Some("x.jpg"), 8192).unwrap().pixels;
        let close = |a: [u8; 4], b: [u8; 3]| (0..3).all(|c| (a[c] as i32 - b[c] as i32).abs() <= 12);
        assert!(close(out.get(4, 8), [200, 0, 0]), "{:?}", out.get(4, 8));
        assert!(close(out.get(27, 8), [10, 20, 240]), "{:?}", out.get(27, 8));
        assert_eq!(out.get(27, 8)[3], 255);

        // The matte formula itself, including the half-transparent case.
        assert_eq!(matte_rgb(&[200, 100, 0, 255], [9, 9, 9]), [200, 100, 0]);
        assert_eq!(matte_rgb(&[200, 100, 0, 0], [9, 8, 7]), [9, 8, 7]);
        assert_eq!(matte_rgb(&[255, 0, 100, 128], [0, 255, 100]), [128, 127, 100]);
        // Out-of-range qualities are clamped, not rejected.
        let smooth = gradient(16, 16);
        assert!(encode_flat(&smooth, &FlatFormat::Jpeg { quality: 0, matte: [0; 3] }).is_ok());
        assert!(encode_flat(&smooth, &FlatFormat::Jpeg { quality: 255, matte: [0; 3] }).is_ok());
    }

    #[test]
    fn jpeg_quality_is_reasonable() {
        let src = gradient(64, 64);
        let bytes = encode_flat(&src, &FlatFormat::Jpeg { quality: 95, matte: [0; 3] }).unwrap();
        let out = decode_image(&bytes, None, 8192).unwrap().pixels;
        assert!(crate::testutil::psnr(&src, &out) > 35.0);
    }

    #[test]
    fn rgb_under_alpha_zero_survives_png() {
        let mut src = PixelBuf::filled(5, 3, [10, 20, 30, 0]);
        src.set(2, 1, [255, 254, 253, 0]);
        src.set(4, 2, [1, 2, 3, 1]);
        for level in [PngLevel::Fast, PngLevel::Balanced, PngLevel::High] {
            let bytes = encode_png(src.width, src.height, &src.data, level).unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            let decoded = decode_image(&bytes, None, 8192).unwrap();
            assert_eq!(decoded.pixels, src, "{level:?}");
        }
    }

    #[test]
    fn encode_png_is_deterministic_and_rgba8() {
        let src = noise(19, 13, 99);
        let a = encode_png(19, 13, &src.data, PngLevel::Balanced).unwrap();
        let b = encode_png(19, 13, &src.data, PngLevel::Balanced).unwrap();
        assert_eq!(a, b);
        // IHDR: width, height, bit depth 8, colour type 6 (RGBA), no interlace.
        assert_eq!(&a[12..16], b"IHDR");
        assert_eq!(&a[16..24], [0, 0, 0, 19, 0, 0, 0, 13]);
        assert_eq!(&a[24..29], [8, 6, 0, 0, 0]);
    }

    #[test]
    fn encoders_reject_bad_buffers() {
        assert!(matches!(encode_png(0, 4, &[], PngLevel::Fast), Err(ImageIoError::Empty)));
        assert!(matches!(encode_png(4, 0, &[], PngLevel::Fast), Err(ImageIoError::Empty)));
        assert!(matches!(encode_png(2, 2, &[0; 15], PngLevel::Fast), Err(ImageIoError::Encode(_))));
        assert!(matches!(encode_png(2, 2, &[0; 20], PngLevel::Fast), Err(ImageIoError::Encode(_))));
        let empty = PixelBuf::new(0, 0);
        let broken = PixelBuf { width: 3, height: 3, data: vec![0; 8] };
        for format in &ALL_FORMATS {
            assert!(matches!(encode_flat(&empty, format), Err(ImageIoError::Empty)), "{format:?}");
            assert!(matches!(encode_flat(&broken, format), Err(ImageIoError::Encode(_))), "{format:?}");
        }
    }

    #[test]
    fn oversize_image_is_downscaled_and_flagged() {
        let src = crate::samples::checker_texture(64, 32, 4);
        let bytes = encode_png(64, 32, &src.data, PngLevel::Fast).unwrap();
        let decoded = decode_image(&bytes, None, 16).unwrap();
        assert!(decoded.resized);
        assert_eq!(decoded.original_size, [64, 32]);
        assert_eq!((decoded.pixels.width, decoded.pixels.height), (16, 8));
        assert_eq!(decoded.pixels.data.len(), 16 * 8 * 4);
        let opaque = decoded.pixels.data.as_chunks::<4>().0.iter().all(|px| px[3] == 255);
        assert!(opaque, "an opaque image stays opaque");

        // Exactly at the limit: untouched.
        let exact = decode_image(&bytes, None, 64).unwrap();
        assert!(!exact.resized);
        assert_eq!(exact.pixels, src);
        // Portrait orientation and extreme aspect ratios keep at least one pixel per side.
        let tall = PixelBuf::filled(2, 200, [9, 9, 9, 255]);
        let tall_bytes = encode_png(2, 200, &tall.data, PngLevel::Fast).unwrap();
        let small = decode_image(&tall_bytes, None, 10).unwrap();
        assert_eq!((small.pixels.width, small.pixels.height), (1, 10));
        assert_eq!(small.pixels.get(0, 5), [9, 9, 9, 255]);
        // max_dim 0 behaves like 1.
        let tiny = decode_image(&bytes, None, 0).unwrap();
        assert_eq!((tiny.pixels.width, tiny.pixels.height), (1, 1));
        assert!(tiny.resized);
    }

    #[test]
    fn huge_dimensions_are_rejected_without_decoding() {
        // A PNG header that claims 20000 x 10 pixels, followed by an (empty) image data chunk.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend_from_slice(&20000u32.to_be_bytes());
        ihdr.extend_from_slice(&10u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(&ihdr);
        png.extend_from_slice(&crc32fast::hash(&ihdr).to_be_bytes());
        for chunk in [b"IDAT", b"IEND"] {
            png.extend_from_slice(&0u32.to_be_bytes());
            png.extend_from_slice(chunk);
            png.extend_from_slice(&crc32fast::hash(chunk).to_be_bytes());
        }
        match decode_image(&png, None, 8192) {
            Err(ImageIoError::TooLarge { width: 20000, height: 10 }) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(matches!(decode_image(&[], Some("a.png"), 8192), Err(ImageIoError::Decode(_))));
        assert!(matches!(decode_image(b"not an image at all", None, 8192), Err(ImageIoError::UnsupportedFormat(_))));
        assert!(matches!(
            decode_image(b"not an image at all", Some("readme.txt"), 8192),
            Err(ImageIoError::UnsupportedFormat(_))
        ));
        // Truncated PNG: the header is fine, the data is not.
        let src = noise(32, 32, 5);
        let bytes = encode_png(32, 32, &src.data, PngLevel::Fast).unwrap();
        assert!(matches!(decode_image(&bytes[..bytes.len() / 2], None, 8192), Err(ImageIoError::Decode(_))));
        assert!(matches!(decode_image(&bytes[..10], None, 8192), Err(ImageIoError::Decode(_))));
        // Garbage with a TGA extension must not panic either.
        assert!(decode_image(&[0xff; 64], Some("junk.tga"), 8192).is_err());
        // GIF is recognised but not compiled in.
        assert!(matches!(
            decode_image(b"GIF89a\x01\x00\x01\x00\x00\x00\x00;", None, 8192),
            Err(ImageIoError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn dds_with_an_unsupported_codec_is_reported_clearly() {
        // Minimal DDS header (124 bytes after the magic) with FourCC "ATI2", which the decoder does not support.
        let mut dds = b"DDS ".to_vec();
        let mut header = vec![0u8; 124];
        header[0..4].copy_from_slice(&124u32.to_le_bytes());
        header[4..8].copy_from_slice(&0x0008_1007u32.to_le_bytes()); // caps | height | width | pixelformat | linearsize
        header[8..12].copy_from_slice(&4u32.to_le_bytes()); // height
        header[12..16].copy_from_slice(&4u32.to_le_bytes()); // width
        header[72..76].copy_from_slice(&32u32.to_le_bytes()); // pixel format size
        header[76..80].copy_from_slice(&4u32.to_le_bytes()); // DDPF_FOURCC
        header[80..84].copy_from_slice(b"ATI2");
        header[104..108].copy_from_slice(&0x1000u32.to_le_bytes()); // DDSCAPS_TEXTURE
        dds.extend_from_slice(&header);
        dds.extend_from_slice(&[0u8; 16]);
        match decode_image(&dds, Some("normal.dds"), 8192) {
            Err(ImageIoError::UnsupportedFormat(message)) => assert!(message.contains("DDS"), "{message}"),
            other => panic!("expected UnsupportedFormat, got {other:?}"),
        }
    }

    /// Branch `companion`: `encode_jpeg` round-trips through `decode_image` within JPEG tolerance; wrong lengths and
    /// zero sizes fail like `encode_png`.
    #[test]
    fn jpeg_round_trip_and_errors() {
        let (w, h) = (67u32, 45u32);
        let src = gradient(w, h);
        let rgb: Vec<u8> = src.data.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        for quality in [40u8, 80, 92] {
            let bytes = encode_jpeg(w, h, &rgb, quality).unwrap();
            assert_eq!(&bytes[..2], &[0xff, 0xd8], "JPEG SOI");
            let back = decode_image(&bytes, Some("t.jpg"), 8192).unwrap();
            assert_eq!(back.original_size, [w, h]);
            let mut worst = 0i32;
            let mut sum = 0i64;
            for (a, b) in src.data.as_chunks::<4>().0.iter().zip(back.pixels.data.as_chunks::<4>().0) {
                assert_eq!(b[3], 255);
                for c in 0..3 {
                    let d = (a[c] as i32 - b[c] as i32).abs();
                    worst = worst.max(d);
                    sum += d as i64;
                }
            }
            let mean = sum as f64 / (w * h * 3) as f64;
            assert!(mean < 3.0 && worst < 40, "quality {quality}: mean {mean}, worst {worst}");
        }
        assert!(matches!(encode_jpeg(0, 4, &[], 80), Err(ImageIoError::Empty)));
        assert!(matches!(encode_jpeg(4, 0, &[], 80), Err(ImageIoError::Empty)));
        assert!(matches!(encode_jpeg(2, 2, &[0; 11], 80), Err(ImageIoError::Encode(_))));
        assert!(matches!(encode_jpeg(2, 2, &[0; 16], 80), Err(ImageIoError::Encode(_))), "RGBA length is wrong");
        // Quality outside 1..=100 is clamped, not an error.
        assert!(encode_jpeg(2, 2, &[7; 12], 0).is_ok() && encode_jpeg(2, 2, &[7; 12], 255).is_ok());
    }
}
