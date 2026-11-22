// Owner: core-persist (C6). OpenRaster writer of ARCHITECTURE.md 7.6: a ZIP with `mimetype`, `stack.xml`, one PNG
// per layer cropped to its content, the merged image and a thumbnail.

use std::fmt::Write as _;
use std::io::Write as _;

use rayon::prelude::*;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::error::{ExportError, ImageIoError};
use crate::export::{ExportLayer, ExportSet};
use crate::image_io::{PngLevel, encode_png};
use crate::math::IRect;
use crate::pixel_ops::{content_bounds, thumbnail};
use crate::project::save::{parallel_budget, thumbnail_side, zip_deflated, zip_stored};

/// Content of the `mimetype` entry.
const ORA_MIMETYPE: &str = "image/openraster";

fn zip_error(e: zip::result::ZipError) -> ExportError {
    match e {
        zip::result::ZipError::Io(io) => ExportError::Io(io),
        other => ExportError::Zip(other.to_string()),
    }
}

/// Escapes a string for use inside a double- or single-quoted XML attribute. Characters XML 1.0 cannot represent
/// (most control characters) are dropped; tab, line feed and carriage return become character references.
fn xml_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => {
                let _ = write!(out, "&#{};", c as u32);
            }
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

/// Layer opacity with three decimals; NaN counts as 0.
fn opacity_attr(opacity: f32) -> String {
    let clamped = if opacity.is_nan() { 0.0 } else { opacity.clamp(0.0, 1.0) };
    format!("{clamped:.3}")
}

/// Name of the PNG entry of the layer with this index from the bottom.
fn layer_entry_name(index: usize) -> String {
    format!("data/layer{index:03}.png")
}

/// Content bounding box of a layer inside the canvas; `None` for an empty layer.
fn layer_rect(layer: &ExportLayer<'_>, canvas: IRect) -> Option<IRect> {
    content_bounds(layer.pixels).map(|r| r.intersect(&canvas)).filter(|r| !r.is_empty())
}

/// `stack.xml` for the set; `rects[i]` is the content rectangle of layer `i` (bottom -> top).
fn stack_xml(set: &ExportSet<'_>, rects: &[Option<IRect>]) -> String {
    let mut xml = String::from("<?xml version='1.0' encoding='UTF-8'?>\n");
    let _ = writeln!(xml, "<image version=\"0.0.3\" w=\"{}\" h=\"{}\" xres=\"72\" yres=\"72\">", set.width, set.height);
    xml.push_str(" <stack>\n");
    // The top-most layer comes first.
    for (index, (layer, rect)) in set.layers.iter().zip(rects).enumerate().rev() {
        let (x, y) = rect.map_or((0, 0), |r| (r.x0, r.y0));
        let _ = writeln!(
            xml,
            "  <layer name=\"{}\" src=\"{}\" x=\"{x}\" y=\"{y}\" opacity=\"{}\" visibility=\"{}\" composite-op=\"{}\"/>",
            xml_attr(layer.name),
            layer_entry_name(index),
            opacity_attr(layer.opacity),
            if layer.visible { "visible" } else { "hidden" },
            layer.blend.ora_op(),
        );
    }
    xml.push_str(" </stack>\n</image>\n");
    xml
}

/// The PNG of one layer: its content rectangle, or a 1x1 transparent image for an empty layer.
fn encode_layer(layer: &ExportLayer<'_>, rect: Option<IRect>) -> Result<Vec<u8>, ImageIoError> {
    let Some(rect) = rect else {
        return encode_png(1, 1, &[0; 4], PngLevel::Balanced);
    };
    let view = layer.pixels;
    if rect == view.rect {
        return encode_png(rect.width(), rect.height(), view.data, PngLevel::Balanced);
    }
    let (start, end) = ((rect.x0 - view.rect.x0) as usize * 4, (rect.x1 - view.rect.x0) as usize * 4);
    let mut data = Vec::with_capacity(rect.area() as usize * 4);
    for y in rect.y0..rect.y1 {
        // `rect` lies inside the view, so every row exists and is long enough.
        let row = view.row(y).and_then(|row| row.get(start..end)).unwrap_or(&[]);
        data.extend_from_slice(row);
    }
    encode_png(rect.width(), rect.height(), &data, PngLevel::Balanced)
}

/// PNGs the caller already encoded from the same pixels with `PngLevel::Balanced`, which `write_ora_reusing` stores
/// instead of encoding them again. The PNG encoder is deterministic, so the archive is byte-identical either way.
#[derive(Default)]
pub struct OraReuse<'a> {
    /// `encode_png(width, height, &flattened.data, Balanced)`.
    pub merged: Option<&'a [u8]>,
    /// Per entry of `ExportSet::layers`: the full-canvas PNG of that layer (`width x height`, Balanced). Used only for
    /// a layer whose content rectangle is the whole canvas: its entry then encodes exactly these pixels.
    pub full_canvas_layers: Vec<Option<&'a [u8]>>,
}

/// Writes one texture set as an OpenRaster archive (section 7.6).
///
/// Entry order: `mimetype` (stored), `stack.xml` (deflated), `data/layerNNN.png` bottom to top, `mergedimage.png`,
/// `Thumbnails/thumbnail.png` (all PNGs stored). Layer PNGs are cropped to the pixels with alpha > 0. Errors: `Io`
/// for a zero-size set or a `flattened` image of the wrong size and for write failures, `Image` / `Zip` otherwise.
pub fn write_ora<W: std::io::Write + std::io::Seek>(w: W, set: &ExportSet<'_>) -> Result<(), ExportError> {
    write_ora_reusing(w, set, &OraReuse::default())
}

/// `write_ora`, storing the PNGs of `reuse` where they encode the same pixels (see `OraReuse`).
pub fn write_ora_reusing<W: std::io::Write + std::io::Seek>(
    w: W,
    set: &ExportSet<'_>,
    reuse: &OraReuse<'_>,
) -> Result<(), ExportError> {
    let (width, height) = (set.width, set.height);
    let flat = set.flattened;
    if width == 0 || height == 0 {
        return Err(ExportError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the texture set has no pixels")));
    }
    if flat.width != width || flat.height != height || flat.data.len() != width as usize * height as usize * 4 {
        return Err(ExportError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the flattened image does not have the size of the texture set",
        )));
    }
    let canvas = IRect::from_size(width, height);
    let rects: Vec<Option<IRect>> = set.layers.par_iter().map(|layer| layer_rect(layer, canvas)).collect();

    let mut zip = ZipWriter::new(w);
    let mut add = |name: String, options: SimpleFileOptions, bytes: &[u8]| -> Result<(), ExportError> {
        zip.start_file(name, options).map_err(zip_error)?;
        zip.write_all(bytes)?;
        Ok(())
    };
    add("mimetype".to_owned(), zip_stored(), ORA_MIMETYPE.as_bytes())?;
    add("stack.xml".to_owned(), zip_deflated(), stack_xml(set, &rects).as_bytes())?;

    // A bounded number of full-size crops at a time.
    let budget = parallel_budget(width, height);
    let indexed: Vec<(usize, &ExportLayer<'_>)> = set.layers.iter().enumerate().collect();
    for chunk in indexed.chunks(budget) {
        let encoded: Vec<_> = chunk
            .par_iter()
            .map(|&(index, layer)| match reuse.full_canvas_layers.get(index).copied().flatten() {
                Some(png) if rects[index] == Some(canvas) => Ok(std::borrow::Cow::Borrowed(png)),
                _ => encode_layer(layer, rects[index]).map(std::borrow::Cow::Owned),
            })
            .collect();
        for (&(index, _), png) in chunk.iter().zip(encoded) {
            add(layer_entry_name(index), zip_stored(), &png?)?;
        }
    }

    let merged = match reuse.merged {
        Some(png) => std::borrow::Cow::Borrowed(png),
        None => std::borrow::Cow::Owned(encode_png(width, height, &flat.data, PngLevel::Balanced)?),
    };
    add("mergedimage.png".to_owned(), zip_stored(), &merged)?;
    drop(merged);
    let thumb = thumbnail(flat.view(), width, height, thumbnail_side(width, height));
    let thumb_png = encode_png(thumb.width, thumb.height, &thumb.data, PngLevel::Balanced)?;
    add("Thumbnails/thumbnail.png".to_owned(), zip_stored(), &thumb_png)?;

    let mut inner = zip.finish().map_err(zip_error)?;
    inner.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use zip::{CompressionMethod, ZipArchive};

    use super::*;
    use crate::image_io::decode_image;
    use crate::layer::BlendMode;
    use crate::pixel::{CroppedBuf, PixelBuf};
    use crate::testutil::XorShift;

    fn entry(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap_or_else(|e| panic!("{name}: {e}")).read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn png(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> PixelBuf {
        decode_image(&entry(zip, name), None, 16384).unwrap().pixels
    }

    struct Fixture {
        bottom: PixelBuf,
        patch: CroppedBuf,
        empty: PixelBuf,
        flat: PixelBuf,
    }

    fn fixture() -> Fixture {
        let (w, h) = (40u32, 24u32);
        let mut rng = XorShift::new(77);
        let mut bottom = PixelBuf::new(w, h);
        for px in bottom.data.as_chunks_mut::<4>().0 {
            *px = [rng.next_u32() as u8, rng.next_u32() as u8, 9, 255];
        }
        let mut patch = CroppedBuf::new(IRect::new(10, 4, 30, 20));
        for (i, px) in patch.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 20, i / 20);
            // The content (alpha > 0) is the inner 14 x 10 block at canvas 13,7.
            let inside = (3..17).contains(&x) && (3..13).contains(&y);
            *px = if inside { [200, rng.next_u32() as u8, 30, 1 + (rng.next_u32() % 255) as u8] } else { [0; 4] };
        }
        let mut flat = bottom.clone();
        flat.set(1, 1, [4, 5, 6, 0]);
        Fixture { bottom, patch, empty: PixelBuf::new(w, h), flat }
    }

    fn layers(f: &Fixture) -> Vec<ExportLayer<'_>> {
        vec![
            ExportLayer { name: "Base", kind: "paint", visible: true, opacity: 1.0, blend: BlendMode::Normal, pixels: f.bottom.view() },
            ExportLayer {
                name: "Logo <\"A&B\"> 'x'",
                kind: "stamp",
                visible: false,
                opacity: 0.75,
                blend: BlendMode::Multiply,
                pixels: f.patch.view(),
            },
            ExportLayer { name: "Empty", kind: "paint", visible: true, opacity: 0.5, blend: BlendMode::Screen, pixels: f.empty.view() },
        ]
    }

    fn write(set: &ExportSet<'_>) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        write_ora(&mut out, set).unwrap();
        out.into_inner()
    }

    #[test]
    fn reused_pngs_give_the_same_archive_and_only_full_canvas_layers_use_them() {
        let f = fixture();
        let set = ExportSet { name: "set", width: 40, height: 24, flattened: &f.flat, layers: layers(&f) };
        let plain = write(&set);
        // What the exporter keeps: the flattened PNG and the full-canvas PNG of every layer.
        let merged = encode_png(40, 24, &f.flat.data, PngLevel::Balanced).unwrap();
        let full: Vec<Vec<u8>> = [f.bottom.clone(), f.patch.to_dense(40, 24), f.empty.clone()]
            .iter()
            .map(|buf| encode_png(40, 24, &buf.data, PngLevel::Balanced).unwrap())
            .collect();
        let reuse = OraReuse { merged: Some(&merged), full_canvas_layers: full.iter().map(|png| Some(png.as_slice())).collect() };
        let mut out = Cursor::new(Vec::new());
        write_ora_reusing(&mut out, &set, &reuse).unwrap();
        assert_eq!(out.into_inner(), plain, "reuse must not change a single byte");

        // The cropped and the empty layer never take the offered PNG; the full-canvas one does.
        let bogus = b"offered".to_vec();
        let reuse = OraReuse { merged: None, full_canvas_layers: vec![Some(&bogus[..]); 3] };
        let mut out = Cursor::new(Vec::new());
        write_ora_reusing(&mut out, &set, &reuse).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(out.into_inner())).unwrap();
        assert_eq!(entry(&mut zip, "data/layer000.png"), bogus);
        assert_eq!(png(&mut zip, "data/layer001.png").width, 14);
        assert_eq!(png(&mut zip, "data/layer002.png").width, 1);
        assert_eq!(png(&mut zip, "mergedimage.png"), f.flat);
    }

    #[test]
    fn mimetype_is_the_first_entry_stored_with_exact_content() {
        let f = fixture();
        let bytes = write(&ExportSet { name: "set", width: 40, height: 24, flattened: &f.flat, layers: layers(&f) });
        // Local file header: signature, ..., method 0 at offset 8, name length 8 at 26, no extra field, then the
        // name and directly the uncompressed content.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(&bytes[8..10], [0, 0]);
        assert_eq!(&bytes[26..30], [8, 0, 0, 0]);
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + 16], b"image/openraster");

        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned()).collect();
        assert_eq!(
            names,
            [
                "mimetype",
                "stack.xml",
                "data/layer000.png",
                "data/layer001.png",
                "data/layer002.png",
                "mergedimage.png",
                "Thumbnails/thumbnail.png"
            ]
        );
        assert_eq!(zip.by_index(0).unwrap().compression(), CompressionMethod::Stored);
        assert_eq!(zip.by_name("stack.xml").unwrap().compression(), CompressionMethod::Deflated);
        assert_eq!(zip.by_name("data/layer000.png").unwrap().compression(), CompressionMethod::Stored);
        assert_eq!(entry(&mut zip, "mimetype"), b"image/openraster");
    }

    #[test]
    fn stack_xml_order_and_attributes() {
        let f = fixture();
        let bytes = write(&ExportSet { name: "set", width: 40, height: 24, flattened: &f.flat, layers: layers(&f) });
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let xml = String::from_utf8(entry(&mut zip, "stack.xml")).unwrap();
        let expected = concat!(
            "<?xml version='1.0' encoding='UTF-8'?>\n",
            "<image version=\"0.0.3\" w=\"40\" h=\"24\" xres=\"72\" yres=\"72\">\n",
            " <stack>\n",
            "  <layer name=\"Empty\" src=\"data/layer002.png\" x=\"0\" y=\"0\" opacity=\"0.500\" visibility=\"visible\" composite-op=\"svg:screen\"/>\n",
            "  <layer name=\"Logo &lt;&quot;A&amp;B&quot;&gt; &apos;x&apos;\" src=\"data/layer001.png\" x=\"13\" y=\"7\" opacity=\"0.750\" visibility=\"hidden\" composite-op=\"svg:multiply\"/>\n",
            "  <layer name=\"Base\" src=\"data/layer000.png\" x=\"0\" y=\"0\" opacity=\"1.000\" visibility=\"visible\" composite-op=\"svg:src-over\"/>\n",
            " </stack>\n",
            "</image>\n",
        );
        assert_eq!(xml, expected);
    }

    #[test]
    fn pngs_decode_to_the_layer_pixels() {
        let f = fixture();
        let bytes = write(&ExportSet { name: "set", width: 40, height: 24, flattened: &f.flat, layers: layers(&f) });
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        // Full-canvas layer: byte for byte.
        assert_eq!(png(&mut zip, "data/layer000.png"), f.bottom);
        // Cropped layer: the content block at its offset.
        let patch = png(&mut zip, "data/layer001.png");
        assert_eq!((patch.width, patch.height), (14, 10));
        for y in 0..10 {
            for x in 0..14 {
                assert_eq!(patch.get(x, y), f.patch.get(13 + x as i32, 7 + y as i32), "pixel {x},{y}");
            }
        }
        // Empty layer: 1x1 transparent.
        assert_eq!(png(&mut zip, "data/layer002.png"), PixelBuf::new(1, 1));
        // Merged image: the flattened pixels unchanged, RGB under alpha 0 included.
        assert_eq!(png(&mut zip, "mergedimage.png"), f.flat);
        let thumb = png(&mut zip, "Thumbnails/thumbnail.png");
        assert_eq!((thumb.width, thumb.height), (40, 24));
    }

    #[test]
    fn thumbnail_is_at_most_256_px_with_the_aspect_kept() {
        let flat = PixelBuf::filled(1024, 512, [30, 60, 90, 255]);
        let bytes = write(&ExportSet { name: "set", width: 1024, height: 512, flattened: &flat, layers: Vec::new() });
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let thumb = png(&mut zip, "Thumbnails/thumbnail.png");
        assert_eq!((thumb.width, thumb.height), (256, 128));
        assert_eq!(thumb.get(100, 50), [30, 60, 90, 255]);
        let xml = String::from_utf8(entry(&mut zip, "stack.xml")).unwrap();
        assert!(xml.contains(" <stack>\n </stack>\n"), "{xml}");
    }

    #[test]
    fn identical_input_gives_identical_bytes() {
        let f = fixture();
        let a = write(&ExportSet { name: "set", width: 40, height: 24, flattened: &f.flat, layers: layers(&f) });
        let b = write(&ExportSet { name: "set", width: 40, height: 24, flattened: &f.flat, layers: layers(&f) });
        assert_eq!(a, b);
    }

    #[test]
    fn attribute_escaping_and_opacity_format() {
        assert_eq!(xml_attr("a&b<c>d\"e'f"), "a&amp;b&lt;c&gt;d&quot;e&apos;f");
        assert_eq!(xml_attr("line\nbreak\ttab\u{1}bell"), "line&#10;break&#9;tabbell");
        assert_eq!(xml_attr("\u{6728} \u{fc}"), "\u{6728} \u{fc}");
        assert_eq!(opacity_attr(0.75), "0.750");
        assert_eq!(opacity_attr(1.0 / 3.0), "0.333");
        assert_eq!(opacity_attr(7.0), "1.000");
        assert_eq!(opacity_attr(-1.0), "0.000");
        assert_eq!(opacity_attr(f32::NAN), "0.000");
    }

    #[test]
    fn wrong_input_is_refused() {
        let flat = PixelBuf::new(2, 2);
        let wrong = ExportSet { name: "set", width: 4, height: 4, flattened: &flat, layers: Vec::new() };
        assert!(matches!(write_ora(Cursor::new(Vec::new()), &wrong), Err(ExportError::Io(_))));
        let zero = ExportSet { name: "set", width: 0, height: 0, flattened: &PixelBuf::new(0, 0), layers: Vec::new() };
        assert!(matches!(write_ora(Cursor::new(Vec::new()), &zero), Err(ExportError::Io(_))));
    }
}
