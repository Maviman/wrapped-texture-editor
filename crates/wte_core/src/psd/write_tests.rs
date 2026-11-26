//! Writer tests (stickers P5b; STICKER_PLAN 5.2, 13.3, 13.9 "P5b"): writer -> reader round trips, every output
//! through the strict walker, the walker calibrated on the Photoshop-saved fixtures and rejecting a seeded fault of
//! each kind, fixture f1 rebuilt with this writer (`f1w_*`, re-saved by the real PhotoCraft v0.2.0 CLI), bytes, limits.

use std::path::PathBuf;

use super::fixture_tests::{PS_FIXTURES, fixture_dir, pc_layer_pngs, png};
use super::strict::walk;
use super::testkit::composite as read_composite;
use super::write::{composite, normalise};
use super::*;
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};

const BLENDS: [BlendMode; 7] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Add,
    BlendMode::Darken,
    BlendMode::Lighten,
];

/// A layer of `rect` whose pixels come from `f(x, y)` (canvas coordinates).
fn layer_from(name: &str, lyid: u32, rect: IRect, f: impl Fn(i32, i32) -> [u8; 4]) -> StickerPsdLayer {
    let mut pixels = CroppedBuf::new(rect);
    let w = rect.width() as usize;
    for (i, px) in pixels.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *px = f(rect.x0 + (i % w) as i32, rect.y0 + (i / w) as i32);
    }
    StickerPsdLayer {
        name: name.to_owned(),
        lyid,
        pixels,
        mask: None,
        clipped: false,
        visible: true,
        opacity: 255,
        blend: BlendMode::Normal,
    }
}

/// A soft disc of `rgb` around `(cx, cy)`: alpha 255 inside `r0`, 0 from `r1`; RGB kept under alpha 0 inside `rect`.
fn disc(name: &str, lyid: u32, rect: IRect, c: (f32, f32), r: (f32, f32), rgb: [u8; 3]) -> StickerPsdLayer {
    layer_from(name, lyid, rect, |x, y| {
        let d = ((x as f32 + 0.5 - c.0).powi(2) + (y as f32 + 0.5 - c.1).powi(2)).sqrt();
        let a = ((r.1 - d) / (r.1 - r.0)).clamp(0.0, 1.0);
        [rgb[0].wrapping_add(x as u8), rgb[1], rgb[2].wrapping_add(y as u8), (a * 255.0).round() as u8]
    })
}

fn mask_from(rect: IRect, default: u8, f: impl Fn(i32, i32) -> u8) -> PsdMask {
    let mut pixels = AlphaBuf::new(rect.width(), rect.height());
    for y in 0..rect.height() {
        for x in 0..rect.width() {
            pixels.set(x, y, f(rect.x0 + x as i32, rect.y0 + y as i32));
        }
    }
    PsdMask { rect, default, pixels, disabled: false, rendered: false }
}

/// Every feature of the round-trip bullet of the P5 list on a 64 x 48 canvas: names (non-ASCII, longer than 31
/// bytes), hidden, opacity, all seven blends, a mask, a clipped layer, a layer partly outside the canvas, an empty
/// layer; the merged picture is transparent.
fn rich_sticker() -> StickerPsd {
    let mut layers = vec![disc("photo", 1, IRect::new(-6, -4, 40, 44), (18.0, 20.0), (10.0, 22.0), [40, 160, 90])];
    // A layer with a soft mask (and a mask default of 0).
    let mut masked = layer_from("Слой ü mit Maske", 2, IRect::new(8, 6, 56, 42), |x, y| [200, (x * 4) as u8, (y * 5) as u8, 220]);
    masked.mask = Some(mask_from(IRect::new(4, 4, 60, 44), 0, |x, y| {
        let d = ((x - 32) * (x - 32) + (y - 24) * (y - 24)) as f32;
        (255.0 * (1.0 - d / 300.0)).clamp(0.0, 255.0) as u8
    }));
    layers.push(masked);
    // Clipped to the masked layer.
    let mut clipped = layer_from("clipped band", 3, IRect::new(0, 18, 64, 30), |_, _| [250, 230, 20, 255]);
    clipped.clipped = true;
    layers.push(clipped);
    // The seven blends, each on a small square, varying opacity; one is hidden.
    for (i, &blend) in BLENDS.iter().enumerate() {
        let x0 = 2 + i as i32 * 9;
        let mut l = layer_from(&format!("blend {i}"), 10 + i as u32, IRect::new(x0, 30, x0 + 8, 46), |x, y| {
            [(x * 3) as u8, 90, (y * 4) as u8, 255]
        });
        l.blend = blend;
        l.opacity = 255 - 20 * i as u8;
        l.visible = i != 3;
        layers.push(l);
    }
    // A name longer than 31 bytes with characters outside Windows-1252 (emoji: a surrogate pair in luni).
    layers.push(layer_from(
        "A very long layer name that is longer than thirty-one bytes 🙂 Ж",
        30,
        IRect::new(50, -10, 80, 12), // partly outside the canvas
        |x, _| [10, 20, (x * 2) as u8, 128],
    ));
    // An empty layer.
    layers.push(StickerPsdLayer { pixels: CroppedBuf::empty(), ..layer_from("empty", 31, IRect::EMPTY, |_, _| [0; 4]) });
    StickerPsd { size: [64, 48], layers, id_seed: 40 }
}

/// Writes, walks strictly and reads back.
fn roundtrip(psd: &StickerPsd) -> (Vec<u8>, strict::Walk, PsdDoc) {
    let bytes = write_sticker_to_vec(psd).expect("write");
    let walked = walk(&bytes).unwrap_or_else(|e| panic!("strict walker: {e}"));
    read::RESYNCS.with(|r| r.set(0));
    let doc = read(&bytes, &PsdLimits::default()).expect("read");
    assert_eq!(read::RESYNCS.with(|r| r.get()), 0, "the reader had to re-synchronise on a WTE file");
    (bytes, walked, doc)
}

/// The un-matting bound of 8-bit merged colours (as in `k5_pc_merged_equals_photocrafts_convert_png`): `at_255`
/// levels for opaque pixels (0 for WTE's own composite, 1 against PhotoCraft's compositor), 1 + ceil(127.5 / a)
/// below. Alpha exact.
fn assert_unmatted_close(got: &PixelBuf, want: &PixelBuf, at_255: u32, what: &str) {
    assert_eq!((got.width, got.height), (want.width, want.height), "{what}");
    for (p, q) in got.data.as_chunks::<4>().0.iter().zip(want.data.as_chunks::<4>().0.iter()) {
        assert_eq!(p[3], q[3], "{what}: alpha");
        let a = q[3] as u32;
        if a == 0 {
            assert_eq!(&p[..3], &[0, 0, 0], "{what}: colour under alpha 0");
            continue;
        }
        let bound = if a == 255 { at_255 } else { 1 + 255_u32.div_ceil(2 * a) };
        for k in 0..3 {
            let d = p[k].abs_diff(q[k]) as u32;
            assert!(d <= bound, "{what}: colour {d} levels off at alpha {a} (bound {bound})");
        }
    }
}

#[test]
fn k5_write_roundtrip_every_layer_feature() {
    let psd = rich_sticker();
    let (_, walked, doc) = roundtrip(&psd);
    assert_eq!(doc.size, [64, 48]);
    assert_eq!((doc.depth, doc.psb, doc.mode), (8, false, PsdColorMode::Rgb));
    assert_eq!(doc.layers.len(), psd.layers.len());
    assert!(walked.negative && walked.channels == 4, "transparent merged picture: negative count, 4 channels");
    assert_eq!(walked.resources.iter().map(|r| r.0).collect::<Vec<_>>(), [1005, 1044, 1057]);
    assert!(doc.has_real_merged);
    assert_eq!(doc.forcing(true), PsdFeatures::empty());
    assert!(!doc.editor_native());
    assert_eq!(testkit::mode(&doc, true), testkit::Mode::Layers);
    for (i, (w, r)) in psd.layers.iter().zip(&doc.layers).enumerate() {
        let normal = normalise(&w.pixels);
        assert_eq!(r.name, w.name, "layer {i}: the full name comes back through luni");
        assert_eq!(r.lyid, Some(w.lyid), "layer {i}");
        assert_eq!(r.rect, normal.rect, "layer {i}: the trimmed rectangle");
        assert_eq!(r.pixels.as_ref(), Some(&normal), "layer {i}: normalised pixels");
        assert_eq!(r.visible, w.visible, "layer {i}");
        assert_eq!(r.opacity, w.opacity, "layer {i}");
        assert_eq!(r.blend, PsdBlend::Known(w.blend), "layer {i}");
        assert_eq!(r.clipped, w.clipped && i > 0, "layer {i}");
        assert_eq!(r.features, PsdFeatures::empty(), "layer {i}: no feature (not BLEND_IF, not NO_PIXELS)");
        assert_eq!((r.section, r.flags, r.fill_opacity), (PsdSection::Layer, 0x08 | if w.visible { 0 } else { 2 }, 255));
        match (&w.mask, &r.mask) {
            (None, None) => {}
            (Some(m), Some(rm)) => {
                assert_eq!((rm.default, rm.disabled, rm.rendered), (m.default, false, false), "layer {i}");
                // Equal as functions on the canvas (the writer trims to the values that differ from the default).
                for y in -8..56 {
                    for x in -8..72 {
                        assert_eq!(rm.value(x, y), m.value(x, y), "layer {i}: mask at {x}, {y}");
                    }
                }
            }
            other => panic!("layer {i}: mask {other:?}"),
        }
    }
    // The layer partly outside the canvas keeps its outside part; the empty layer is an empty picture layer.
    let outside = doc.layers.iter().find(|l| l.lyid == Some(30)).unwrap();
    assert_eq!(outside.rect, IRect::new(50, -10, 80, 12));
    let empty = doc.layers.last().unwrap();
    assert_eq!((empty.rect, empty.pixels.as_ref()), (IRect::EMPTY, Some(&CroppedBuf::empty())));
    // The merged picture: the writer's composite = the test compositor's composite of what was read, exactly; the
    // stored (matted) picture un-matted by the reader is within the un-matting bound of it.
    let want = composite(&psd).unwrap();
    assert_eq!(read_composite(&doc), want, "composite of the read layers");
    assert_unmatted_close(doc.merged.as_ref().unwrap(), &want, 0, "merged");
}

#[test]
fn k5_write_empty_layer_reads_as_an_empty_picture_layer_in_layers_mode() {
    let psd = StickerPsd {
        size: [16, 16],
        layers: vec![StickerPsdLayer {
            pixels: CroppedBuf::new(IRect::new(2, 2, 10, 10)), // all transparent: trimmed to nothing
            ..layer_from("nothing yet", 1, IRect::EMPTY, |_, _| [0; 4])
        }],
        id_seed: 1,
    };
    let (bytes, walked, doc) = roundtrip(&psd);
    let l = &doc.layers[0];
    assert_eq!((l.rect, l.pixels.as_ref(), l.features), (IRect::EMPTY, Some(&CroppedBuf::empty()), PsdFeatures::empty()));
    assert_eq!(l.flags, 0x08);
    assert_eq!(testkit::mode(&doc, true), testkit::Mode::Layers, "not NO_PIXELS, not Merged");
    // The record says rect 0, 0, 0, 0 and every channel is compression 0 with no bytes.
    let at = walked.channel_len_at[0][0] - 2 - 2 - 16; // length <- id <- channel count <- rect
    assert_eq!(&bytes[at..at + 16], &[0; 16]);
    for &len_at in &walked.channel_len_at[0] {
        assert_eq!(u32::from_be_bytes(bytes[len_at..len_at + 4].try_into().unwrap()), 2);
    }
    // A transparent canvas: 4 channels, fully transparent merged picture.
    assert!(doc.merged.as_ref().unwrap().data.iter().all(|&b| b == 0));
}

#[test]
fn k5_write_opaque_merged_has_three_channels_and_a_positive_count() {
    let psd = StickerPsd {
        size: [20, 10],
        layers: vec![
            layer_from("ground", 1, IRect::new(0, 0, 20, 10), |x, y| [(x * 10) as u8, (y * 20) as u8, 77, 255]),
            disc("spot", 2, IRect::new(4, 0, 16, 10), (10.0, 5.0), (2.0, 5.0), [250, 10, 10]),
        ],
        id_seed: 2,
    };
    let (_, walked, doc) = roundtrip(&psd);
    assert_eq!((walked.channels, walked.negative), (3, false));
    let want = composite(&psd).unwrap();
    assert!(want.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    assert_eq!(doc.merged.as_ref(), Some(&want), "an opaque merged picture comes back exactly");
    assert_eq!(read_composite(&doc), want);
}

#[test]
fn k5_write_scales_1_2_4() {
    // The same sticker written at link scales 1, 2 and 4 (the caller scales the canvas and the layers, 5.4).
    let base = rich_sticker();
    for s in [1i32, 2, 4] {
        let scale = |l: &StickerPsdLayer| {
            let r = l.pixels.rect;
            let rect = if r.is_empty() { IRect::EMPTY } else { IRect::new(r.x0 * s, r.y0 * s, r.x1 * s, r.y1 * s) };
            StickerPsdLayer {
                pixels: layer_from("", 1, rect, |x, y| l.pixels.get(x.div_euclid(s), y.div_euclid(s))).pixels,
                mask: l.mask.as_ref().map(|m| {
                    let r = m.rect;
                    mask_from(IRect::new(r.x0 * s, r.y0 * s, r.x1 * s, r.y1 * s), m.default, |x, y| {
                        m.value(x.div_euclid(s), y.div_euclid(s))
                    })
                }),
                ..l.clone()
            }
        };
        let psd = StickerPsd {
            size: [64 * s as u32, 48 * s as u32],
            layers: base.layers.iter().map(scale).collect(),
            id_seed: base.id_seed,
        };
        let (_, _, doc) = roundtrip(&psd);
        assert_eq!(doc.size, psd.size, "scale {s}");
        for (w, r) in psd.layers.iter().zip(&doc.layers) {
            assert_eq!(r.pixels.as_ref(), Some(&normalise(&w.pixels)), "scale {s}: {}", w.name);
        }
        let want = composite(&psd).unwrap();
        assert_eq!(read_composite(&doc), want, "scale {s}");
        assert_unmatted_close(doc.merged.as_ref().unwrap(), &want, 0, &format!("scale {s}"));
    }
}

#[test]
fn k5_write_normalises_layer_pixels_and_masks() {
    // RGB under alpha 0 is cleared and the rectangle trimmed to alpha > 0 (review A11).
    let l = layer_from("p", 1, IRect::new(-3, 2, 9, 12), |x, y| {
        let inside = (2..6).contains(&x) && (5..8).contains(&y);
        [200, 100, 50, if inside { 10 + x as u8 } else { 0 }]
    });
    let n = normalise(&l.pixels);
    assert_eq!(n.rect, IRect::new(2, 5, 6, 8));
    assert!(n.data.as_chunks::<4>().0.iter().all(|p| p[3] > 0));
    let mut holes = l.clone();
    holes.pixels = layer_from("", 1, IRect::new(0, 0, 4, 1), |x, _| [9, 9, 9, if x == 1 { 0 } else { 255 }]).pixels;
    let n = normalise(&holes.pixels);
    assert_eq!(n.get(1, 0), [0, 0, 0, 0], "RGB under alpha 0 inside the trimmed rectangle is cleared");
    assert_eq!(normalise(&CroppedBuf::new(IRect::new(0, 0, 5, 5))), CroppedBuf::empty());
    // Two layers that differ only in RGB under alpha 0 and in their rectangle are written with the same bytes.
    let a = StickerPsd { size: [16, 16], layers: vec![l.clone()], id_seed: 1 };
    let mut b = a.clone();
    b.layers[0] = layer_from("p", 1, IRect::new(0, 0, 16, 16), |x, y| {
        let inside = (2..6).contains(&x) && (5..8).contains(&y);
        if inside { [200, 100, 50, 10 + x as u8] } else { [1, 2, 3, 0] }
    });
    assert_eq!(write_sticker_to_vec(&a).unwrap(), write_sticker_to_vec(&b).unwrap());

    // A mask is trimmed to the values that differ from its default; a mask of only its default has an empty rect.
    let mut m = l.clone();
    m.pixels = layer_from("", 1, IRect::new(0, 0, 16, 16), |_, _| [1, 2, 3, 255]).pixels;
    m.mask = Some(mask_from(IRect::new(0, 0, 16, 16), 255, |x, y| if x == 4 && y == 9 { 0 } else { 255 }));
    let (_, _, doc) = roundtrip(&StickerPsd { size: [16, 16], layers: vec![m.clone()], id_seed: 1 });
    assert_eq!(doc.layers[0].mask.as_ref().unwrap().rect, IRect::new(4, 9, 5, 10));
    m.mask = Some(PsdMask { disabled: true, ..mask_from(IRect::new(0, 0, 16, 16), 255, |_, _| 255) });
    let psd = StickerPsd { size: [16, 16], layers: vec![m], id_seed: 1 };
    let (_, _, doc) = roundtrip(&psd);
    let rm = doc.layers[0].mask.as_ref().unwrap();
    assert_eq!((rm.rect, rm.default, rm.disabled), (IRect::EMPTY, 255, true), "an all-default disabled mask");
    assert_eq!(read_composite(&doc), composite(&psd).unwrap());
}

#[test]
fn k5_write_bytes_are_deterministic() {
    let psd = rich_sticker();
    let a = write_sticker_to_vec(&psd).unwrap();
    let b = write_sticker_to_vec(&psd).unwrap();
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let c = one.install(|| write_sticker_to_vec(&psd).unwrap());
    assert_eq!(a, b);
    assert_eq!(a, c, "independent of the thread count");
    // `write_sticker` writes exactly the same bytes.
    let mut d = Vec::new();
    write_sticker(&mut d, &psd).unwrap();
    assert_eq!(a, d);
}

#[test]
fn k5_write_byte_rules_of_13_3() {
    let psd = rich_sticker();
    let (bytes, walked, _) = roundtrip(&psd);
    let be32 = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    // Header: version 1, 6 zero bytes, depth 8, RGB; no colour mode data.
    assert_eq!(&bytes[..6], b"8BPS\x00\x01");
    assert_eq!(&bytes[6..12], &[0; 6]);
    assert_eq!(&bytes[22..26], &[0, 8, 0, 3]);
    assert_eq!(be32(26), 0);
    // Resources: 1005 = 72 dpi, 1044 = the id seed (40 > the largest lyid 31), 1057 = version 1, real merged data,
    // the writer's name twice, file version 1.
    let data = |id: u16| {
        let at = walked.resources.iter().find(|r| r.0 == id).unwrap().1;
        &bytes[at + 4..at + 4 + be32(at) as usize]
    };
    assert_eq!(data(1005), &[0, 0x48, 0, 0, 0, 1, 0, 1, 0, 0x48, 0, 0, 0, 1, 0, 1]);
    assert_eq!(data(1044), &40u32.to_be_bytes());
    let v = data(1057);
    assert_eq!(&v[..5], &[0, 0, 0, 1, 1]);
    assert_eq!(v.len(), 4 + 1 + 2 * (4 + 2 * write::WRITER_NAME.len()) + 4);
    assert_eq!(&v[v.len() - 4..], &[0, 0, 0, 1]);
    // A seed below the largest lyid is raised to it.
    let low = StickerPsd { id_seed: 0, ..psd.clone() };
    let (b2, w2, _) = roundtrip(&low);
    let at = w2.resources.iter().find(|r| r.0 == 1044).unwrap().1;
    assert_eq!(&b2[at + 4..at + 8], &31u32.to_be_bytes());
    // Lengths: the section and the layer info are multiples of 4.
    assert_eq!(be32(walked.section_len_at) % 4, 0);
    assert_eq!(be32(walked.layer_info_len_at) % 4, 0);
    assert_eq!(walked.section_pad, 0);
    // Every record: flags 0x08 (+ 0x02 hidden), filler 0, Photoshop's 40-byte default blending ranges, channels
    // -1, 0, 1, 2 (-2), mask data 0 or 20 bytes.
    for (i, (l, &extra_at)) in psd.layers.iter().zip(&walked.extra_len_at).enumerate() {
        assert_eq!(bytes[extra_at - 2], 0x08 | if l.visible { 0 } else { 2 }, "layer {i} flags");
        assert_eq!(bytes[extra_at - 1], 0, "layer {i} filler");
        let mask_len = be32(extra_at + 4);
        assert_eq!(mask_len, if l.mask.is_some() { 20 } else { 0 }, "layer {i}");
        let ranges_at = extra_at + 8 + mask_len as usize;
        assert_eq!(be32(ranges_at), 40, "layer {i}");
        for pair in bytes[ranges_at + 4..ranges_at + 44].as_chunks::<8>().0 {
            assert_eq!(pair, &[0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF], "layer {i}");
        }
        let ids: Vec<i16> = walked.channel_len_at[i]
            .iter()
            .map(|&at| i16::from_be_bytes([bytes[at - 2], bytes[at - 1]]))
            .collect();
        let want: &[i16] = if l.mask.is_some() { &[-1, 0, 1, 2, -2] } else { &[-1, 0, 1, 2] };
        assert_eq!(ids, want, "layer {i}");
    }
    // The global layer mask info is empty and no global block follows; the merged picture is RLE.
    assert!(walked.global_blocks.is_empty());
    assert_eq!(walked.merged_compression, 1);
}

#[test]
fn k5_write_pascal_names_are_windows_1252() {
    let names = [
        ("Café € – “quoted”", b"Caf\xe9 \x80 \x96 \x93quoted\x94".to_vec()),
        ("Слой", b"????".to_vec()),
        ("tab\there", b"tab?here".to_vec()),
    ];
    for (name, want) in names {
        let mut l = layer_from(name, 1, IRect::new(0, 0, 2, 2), |_, _| [1, 2, 3, 255]);
        l.name = name.to_owned();
        let psd = StickerPsd { size: [4, 4], layers: vec![l], id_seed: 1 };
        let (bytes, walked, doc) = roundtrip(&psd);
        let at = walked.extra_len_at[0] + 4 + 4 + 4 + 40;
        assert_eq!(bytes[at] as usize, want.len(), "{name}");
        assert_eq!(&bytes[at + 1..at + 1 + want.len()], &want[..], "{name}");
        assert_eq!((1 + want.len()).div_ceil(4) * 4 - (1 + want.len()), {
            let pad_start = at + 1 + want.len();
            bytes[pad_start..].iter().take_while(|&&b| b == 0).count().min(3)
        });
        assert_eq!(doc.layers[0].name, name, "luni wins");
    }
    // At most 255 bytes in the Pascal name; luni keeps the whole name.
    let long: String = "x".repeat(300);
    let psd = StickerPsd {
        size: [4, 4],
        layers: vec![StickerPsdLayer { name: long.clone(), ..layer_from("", 1, IRect::new(0, 0, 1, 1), |_, _| [0, 0, 0, 255]) }],
        id_seed: 1,
    };
    let (bytes, walked, doc) = roundtrip(&psd);
    assert_eq!(bytes[walked.extra_len_at[0] + 4 + 4 + 4 + 40], 255);
    assert_eq!(doc.layers[0].name, long);
}

#[test]
fn k5_write_clipping_and_hidden_bases() {
    // A clipped bottom layer is written unclipped (it has nothing to clip to); a hidden base hides its clipped layers.
    let base = || layer_from("base", 1, IRect::new(2, 2, 10, 10), |_, _| [0, 0, 255, 255]);
    let mut top = layer_from("top", 2, IRect::new(0, 0, 12, 12), |_, _| [255, 0, 0, 255]);
    top.clipped = true;
    let mut bottom = base();
    bottom.clipped = true;
    let (_, _, doc) = roundtrip(&StickerPsd { size: [12, 12], layers: vec![bottom, top.clone()], id_seed: 2 });
    assert_eq!((doc.layers[0].clipped, doc.layers[1].clipped), (false, true));
    let merged = doc.merged.as_ref().unwrap();
    assert_eq!(merged.get(5, 5), [255, 0, 0, 255], "clipped to the base");
    assert_eq!(merged.get(0, 0), [0, 0, 0, 0], "nothing outside the base");
    let mut hidden = base();
    hidden.visible = false;
    let psd = StickerPsd { size: [12, 12], layers: vec![hidden, top], id_seed: 2 };
    let (_, _, doc) = roundtrip(&psd);
    assert!(doc.merged.as_ref().unwrap().data.iter().all(|&b| b == 0), "a hidden base hides its clipped layer");
    assert_eq!(read_composite(&doc), composite(&psd).unwrap());
}

#[test]
fn k5_write_refuses_what_a_psd_cannot_hold_and_writes_nothing() {
    let ok = || StickerPsd { size: [8, 8], layers: vec![layer_from("a", 1, IRect::new(0, 0, 2, 2), |_, _| [1; 4])], id_seed: 1 };
    let mut cases: Vec<(StickerPsd, PsdError)> = Vec::new();
    cases.push((StickerPsd { size: [0, 8], ..ok() }, PsdError::Invalid("the canvas has no pixels")));
    cases.push((StickerPsd { size: [30_001, 8], ..ok() }, PsdError::TooLarge { width: 30_001, height: 8 }));
    let mut dup = ok();
    dup.layers.push(dup.layers[0].clone());
    cases.push((dup, PsdError::Invalid("layer ids must be unique and at least 1")));
    let mut zero = ok();
    zero.layers[0].lyid = 0;
    cases.push((zero, PsdError::Invalid("layer ids must be unique and at least 1")));
    let mut bad = ok();
    bad.layers[0].pixels.data.pop();
    cases.push((bad, PsdError::Invalid("layer pixels do not match their rectangle")));
    let mut bad_mask = ok();
    bad_mask.layers[0].mask = Some(PsdMask { rect: IRect::new(0, 0, 3, 3), ..mask_from(IRect::new(0, 0, 2, 2), 0, |_, _| 9) });
    cases.push((bad_mask, PsdError::Invalid("mask values do not match their rectangle")));
    let mut grey = ok();
    grey.layers[0].mask = Some(mask_from(IRect::new(0, 0, 2, 2), 128, |_, _| 9));
    cases.push((grey, PsdError::Invalid("a mask's default must be 0 or 255")));
    let mut wide = ok();
    wide.layers[0] = layer_from("wide", 1, IRect::new(-20_000, 0, 10_001, 1), |_, _| [1, 1, 1, 255]);
    cases.push((wide, PsdError::Invalid("a layer or mask is wider than 30000 pixels")));
    let mut many = ok();
    many.layers = (1..=32_768).map(|i| StickerPsdLayer { lyid: i, ..layer_from("", i, IRect::EMPTY, |_, _| [0; 4]) }).collect();
    cases.push((many, PsdError::Invalid("too many layers for a PSD file")));
    for (psd, want) in cases {
        let mut out = Vec::new();
        assert_eq!(write_sticker(&mut out, &psd), Err(want.clone()));
        assert!(out.is_empty(), "{want:?}: nothing written");
    }
    // A failing output is `Io` with its kind.
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    match write_sticker(Broken, &ok()) {
        Err(PsdError::Io { kind, .. }) => assert_eq!(kind, std::io::ErrorKind::PermissionDenied),
        other => panic!("{other:?}"),
    }
}

#[test]
fn k5_write_without_layers_is_a_flat_transparent_file() {
    let psd = StickerPsd { size: [5, 3], layers: Vec::new(), id_seed: 0 };
    let (_, walked, doc) = roundtrip(&psd);
    assert_eq!((walked.layers, walked.channels), (0, 4));
    assert!(doc.is_flat() && doc.has_real_merged);
    assert_eq!(doc.merged, Some(PixelBuf::new(5, 3)));
}

// ---- The strict walker ----------------------------------------------------------------------------------------

#[test]
fn k5_strict_accepts_every_photoshop_fixture() {
    // Calibration (13.3): every fixture whose resource 1057 names Photoshop passes, with the 0-to-3 pad allowance.
    let mut pads = Vec::new();
    for rel in PS_FIXTURES {
        let bytes = std::fs::read(fixture_dir().join("photoshop").join(rel)).unwrap();
        let result = walk(&bytes);
        if rel == "transparentbg-gimp.psd" {
            // The one GIMP file is not part of the calibration (it has a 65,538-byte global mask record).
            continue;
        }
        let w = result.unwrap_or_else(|e| panic!("{rel}: {e}"));
        pads.push((rel, w.layer_info_pad, w.section_pad));
    }
    assert_eq!(pads.len(), 54);
    // The allowance is used: 27 of Photoshop's own files have 1 to 3 pad bytes after the channel data of the layer
    // info; none has any at the end of the section (their global blocks are padded to 4 each).
    assert_eq!(pads.iter().filter(|p| p.1 > 0).count(), 27, "{pads:?}");
    assert_eq!(pads.iter().filter(|p| p.2 > 0).count(), 0, "{pads:?}");
}

#[test]
fn k5_strict_on_the_photocraft_fixtures() {
    // Not part of the calibration: PhotoCraft v0.2.0 writes lengths that are not rounded (a layer-and-mask section
    // of any length, `lfx2` blocks of odd length). Its own reader and WTE's reader accept them; the strict walker
    // states Photoshop's stricter layout, so these files fail it. Recorded so that a change is noticed.
    let passes = [
        "f1_pc.psd", "f2_feathered.psd", "f3_opaque.psd", "f5_f1_plus_text.psd", "f7_16bit.psd", "f9_alpha_channel.psd",
        "f10_f1.psb", "f1w_pc.psd",
    ];
    let fails = [
        ("f4_mask_stroke.psd", "block lfx2 length 369 is not a multiple of 4"),
        ("f4b_mask.psd", "layer and mask section length 1642 is not a multiple of 4"),
        ("f6_group.psd", "layer and mask section length 1718 is not a multiple of 4"),
        ("f8_hidden_opacity_multiply.psd", "layer and mask section length 3230 is not a multiple of 4"),
        ("f8b_color_dodge.psd", "layer and mask section length 2038 is not a multiple of 4"),
        ("f11_clipped.psd", "layer and mask section length 1814 is not a multiple of 4"),
        ("f12_lyid_reuse.psd", "layer and mask section length 4494 is not a multiple of 4"),
        ("f13_check_16bit.psb", "layer and mask section length 6602 is not a multiple of 4"),
    ];
    for name in passes {
        walk(&std::fs::read(fixture_dir().join(name)).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    for (name, why) in fails {
        let e = walk(&std::fs::read(fixture_dir().join(name)).unwrap()).unwrap_err();
        assert!(e.contains(why), "{name}: {e}");
    }
}

/// Bytes of a WTE file whose layer info ends with 1 to 3 pad bytes.
fn padded_file() -> (Vec<u8>, strict::Walk) {
    for extra in 0..8 {
        let mut psd = rich_sticker();
        psd.layers[0].name.push_str(&"p".repeat(extra));
        let (bytes, walked, _) = roundtrip(&psd);
        if walked.layer_info_pad > 0 {
            return (bytes, walked);
        }
    }
    panic!("no padded file");
}

fn put32(bytes: &mut [u8], at: usize, v: u32) {
    bytes[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn k5_strict_rejects_a_seeded_fault_of_each_kind() {
    let (good, w) = padded_file();
    walk(&good).unwrap();

    // 1. A wrong length: one channel's length field one too large.
    let mut bad = good.clone();
    let at = w.channel_len_at[0][1];
    put32(&mut bad, at, get32(&good, at) + 1);
    assert!(walk(&bad).is_err(), "wrong channel length");
    // ... and one record's extra-data length one too small.
    let mut bad = good.clone();
    let at = w.extra_len_at[1];
    put32(&mut bad, at, get32(&good, at) - 4);
    assert!(walk(&bad).is_err(), "wrong extra length");

    // 2. Odd padding: resource 1057 (odd length) without its pad byte.
    let at = w.resources.iter().find(|r| r.0 == 1057).unwrap().1;
    let len = get32(&good, at) as usize;
    assert_eq!(len % 2, 1);
    let mut bad = good.clone();
    bad.remove(at + 4 + len);
    put32(&mut bad, w.resources_len_at, get32(&good, w.resources_len_at) - 1);
    let e = walk(&bad).unwrap_err();
    assert!(e.contains("not padded to even"), "{e}");
    // ... and a Pascal name padded to 2 instead of 4 (the lengths kept consistent otherwise).
    let psd = StickerPsd {
        size: [4, 4],
        layers: vec![layer_from("ab", 1, IRect::new(0, 0, 1, 1), |_, _| [1, 2, 3, 255])],
        id_seed: 1,
    };
    let (b, wn) = {
        let (b, wn, _) = roundtrip(&psd);
        (b, wn)
    };
    let name_at = wn.extra_len_at[0] + 4 + 4 + 4 + 40;
    assert_eq!(&b[name_at..name_at + 4], b"\x02ab\x00");
    let mut bad = b.clone();
    bad.remove(name_at + 3);
    bad.insert(wn.layer_info_data_end - 1, 0); // keep every outer length: one more pad byte at the end
    put32(&mut bad, wn.extra_len_at[0], get32(&b, wn.extra_len_at[0]) - 1);
    let e = walk(&bad).unwrap_err();
    assert!(e.contains("not a multiple of 4") || e.contains("signature") || e.contains("name"), "{e}");

    // 3. A non-zero pad byte at the end of the layer info.
    let mut bad = good.clone();
    bad[w.layer_info_data_end] = 1;
    let e = walk(&bad).unwrap_err();
    assert!(e.contains("non-zero pad byte"), "{e}");

    // 4. Four pad bytes (the layer info and the section each 4 longer).
    let mut bad = good.clone();
    let end = w.layer_info_data_end + w.layer_info_pad;
    bad.splice(end..end, [0u8; 4]);
    put32(&mut bad, w.layer_info_len_at, get32(&good, w.layer_info_len_at) + 4);
    put32(&mut bad, w.section_len_at, get32(&good, w.section_len_at) + 4);
    let e = walk(&bad).unwrap_err();
    assert!(e.contains("at most 3 pad bytes"), "{e}");

    // Also: a byte after the merged picture, and a truncated merged picture.
    let mut bad = good.clone();
    bad.push(0);
    assert!(walk(&bad).is_err(), "trailing byte");
    assert!(walk(&good[..good.len() - 1]).is_err(), "truncated");
}

// ---- Fixture f1 rebuilt with this writer (8.3) ----------------------------------------------------------------

/// The three layers of fixture f1 (the same pictures as `gen_f1`): a soft-edged picture with RGB under alpha 0,
/// the word "WTE" in WTE's default font at 18 px, an anti-aliased red disc; `lyid` 1, 2, 3.
fn f1w_sticker() -> StickerPsd {
    const W: u32 = 64;
    const H: u32 = 48;
    let full = IRect::from_size(W, H);
    let photo = layer_from("photo", 1, full, |x, y| {
        let d = ((x as f32 + 0.5 - 22.0).powi(2) + (y as f32 + 0.5 - 24.0).powi(2)).sqrt();
        let a = ((20.0 - d) / 8.0).clamp(0.0, 1.0);
        if d <= 24.0 { [(40 + x * 3) as u8, (200 - y * 3) as u8, 160, (a * 255.0).round() as u8] } else { [0; 4] }
    });
    let font = fontdue::Font::from_bytes(crate::text::default_font_bytes(), fontdue::FontSettings::default()).unwrap();
    let mut text = layer_from("text", 2, full, |_, _| [0; 4]);
    let mut pen_x = 30.0f32;
    for ch in "WTE".chars() {
        let (m, bitmap) = font.rasterize(ch, 18.0);
        let x0 = pen_x.round() as i32 + m.xmin;
        let y0 = 20 - m.height as i32 - m.ymin;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let cov = bitmap[gy * m.width + gx];
                let (x, y) = (x0 + gx as i32, y0 + gy as i32);
                if cov > 0 && full.contains(x, y) {
                    let i = (y as usize * W as usize + x as usize) * 4;
                    text.pixels.data[i..i + 4].copy_from_slice(&[20, 20, 30, cov]);
                }
            }
        }
        pen_x += m.advance_width;
    }
    let shape = layer_from("shape", 3, full, |x, y| {
        let mut n = 0;
        for sy in 0..4 {
            for sx in 0..4 {
                let px = x as f32 + (sx as f32 + 0.5) / 4.0 - 50.0;
                let py = y as f32 + (sy as f32 + 0.5) / 4.0 - 36.0;
                if px * px + py * py <= 81.0 {
                    n += 1;
                }
            }
        }
        if n > 0 { [220, 30, 40, ((n * 255 + 8) / 16) as u8] } else { [0; 4] }
    });
    StickerPsd { size: [W, H], layers: vec![photo, text, shape], id_seed: 3 }
}

#[test]
fn k5_write_f1w_input_is_the_committed_fixture() {
    // `f1w_wte_in.psd` is this writer's output for f1 (made by `k5_write_make_f1w_input`); PhotoCraft's re-save of
    // it is `f1w_pc.psd`. A change of the writer's bytes shows here first.
    let committed = std::fs::read(fixture_dir().join("f1w_wte_in.psd")).unwrap();
    assert!(write_sticker_to_vec(&f1w_sticker()).unwrap() == committed, "the writer's bytes changed");
    walk(&committed).unwrap();
}

#[test]
#[ignore = "writes the f1w input fixture; run once by hand (see tests/fixtures/psd/README.md)"]
fn k5_write_make_f1w_input() {
    let out = std::env::var_os("WTE_F1W_OUT").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sticker/p5b/f1w_wte_in.psd")
    });
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(&out, write_sticker_to_vec(&f1w_sticker()).unwrap()).unwrap();
}

#[test]
fn k5_write_f1w_photocraft_resave_keeps_ids_and_normalised_pixels() {
    // PhotoCraft v0.2.0 (`convert f1w_wte_in.psd f1w_pc.psd`) keeps the lyids, the names and the order, and its
    // re-trim of a layer this writer normalised gives the same rectangle and the same bytes (review A11): the
    // comparison rule of 5.4 sees every untouched layer as unchanged.
    let written = f1w_sticker();
    let doc = read(&std::fs::read(fixture_dir().join("f1w_pc.psd")).unwrap(), &PsdLimits::default()).unwrap();
    assert_eq!(doc.layers.iter().map(|l| l.lyid).collect::<Vec<_>>(), [Some(1), Some(2), Some(3)]);
    assert_eq!(doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["photo", "text", "shape"]);
    assert_eq!(testkit::mode(&doc, false), testkit::Mode::Layers);
    for (w, r) in written.layers.iter().zip(&doc.layers) {
        let n = normalise(&w.pixels);
        assert_eq!(r.rect, n.rect, "{}", w.name);
        assert_eq!(r.pixels.as_ref(), Some(&n), "{}: PhotoCraft's bytes equal the written ones", w.name);
    }
    // PhotoCraft's merged picture against this writer's composite, and its per-layer PNGs against the layers.
    let want = composite(&written).unwrap();
    assert_unmatted_close(doc.merged.as_ref().unwrap(), &want, 1, "f1w_pc merged");
    let pngs = pc_layer_pngs("f1w_pc");
    assert_eq!(pngs.len(), 3);
    for ((file, want), w) in pngs.iter().zip(&written.layers) {
        assert!(file.ends_with(&format!("_{}.png", w.name)), "{file}");
        assert_eq!(*want, normalise(&w.pixels).to_dense(64, 48), "{file}");
    }
    let merged_png = png(fixture_dir().join("f1w_pc_merged.png"));
    assert_unmatted_close(&merged_png, &want, 1, "f1w_pc_merged.png");
}

#[test]
fn k5_check_rich_sticker_survives_a_photocraft_resave() {
    // P5b checker: `f14w_rich_pc.psd` is `rich_sticker()` as this writer wrote it (SHA-256 dba2d8ae...17d5, the
    // `rich.psd` of `k5_write_dump_for_psd_check`), re-saved headless by the real PhotoCraft v0.2.0 CLI
    // (`convert rich.psd f14w_rich_pc.psd`, `make_f14w.sh`). A second editor reads every feature the writer writes
    // the way WTE means it: names (luni over a lossy Pascal name, an emoji), `lyid`s, order, hidden, opacity, the
    // seven blend keys, the clipping byte, a soft mask with default 0, the layer partly outside the canvas and the
    // empty layer; every layer's pixels and mask on the canvas are the written (normalised) ones.
    let written = rich_sticker();
    let doc = read(&std::fs::read(fixture_dir().join("f14w_rich_pc.psd")).unwrap(), &PsdLimits::default()).unwrap();
    assert_eq!(doc.layers.len(), written.layers.len());
    assert_eq!(testkit::mode(&doc, false), testkit::Mode::Layers);
    let canvas = IRect::from_size(64, 48);
    for (i, (w, r)) in written.layers.iter().zip(&doc.layers).enumerate() {
        assert_eq!((r.name.as_str(), r.lyid), (w.name.as_str(), Some(w.lyid)), "layer {i}");
        assert_eq!((r.visible, r.opacity, r.blend), (w.visible, w.opacity, PsdBlend::Known(w.blend)), "layer {i}");
        assert_eq!(r.clipped, w.clipped && i > 0, "layer {i}");
        assert_eq!(r.features, PsdFeatures::empty(), "layer {i}");
        let want = normalise(&w.pixels);
        let got = r.pixels.as_ref().unwrap();
        for y in canvas.y0..canvas.y1 {
            for x in canvas.x0..canvas.x1 {
                assert_eq!(got.get(x, y), want.get(x, y), "layer {i} ({}) pixel {x}, {y}", w.name);
            }
        }
        match (&w.mask, &r.mask) {
            (None, None) => {}
            (Some(m), Some(rm)) => {
                assert!(!rm.disabled, "layer {i}");
                for y in canvas.y0..canvas.y1 {
                    for x in canvas.x0..canvas.x1 {
                        assert_eq!(rm.value(x, y), m.value(x, y), "layer {i}: mask at {x}, {y}");
                    }
                }
            }
            other => panic!("layer {i}: mask {other:?}"),
        }
    }
}

#[test]
fn k5_check_random_stickers_round_trip() {
    // P5b checker: 300 random stickers (fixed seed): canvases 1..40 a side, layers anywhere (also wholly outside the
    // canvas, 1 x 1, empty), random alpha with holes and RGB under alpha 0, masks of either default (also wholly
    // outside, all default, disabled), clipping anywhere (also on the bottom layer and on chains), hidden bases,
    // every blend and opacity. Each file passes the strict walker and reads back with no re-sync; layers and masks
    // come back as written (normalised; masks as functions on the plane), and the reader's composite of what was
    // read is the writer's composite exactly; the stored merged picture is within the un-matting bound of it.
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move |n: u32| -> u32 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s % n.max(1) as u64) as u32
    };
    for case in 0..300 {
        let (cw, ch) = (1 + rnd(40), 1 + rnd(40));
        let n = rnd(6) as usize;
        let mut layers = Vec::new();
        for k in 0..n {
            let x0 = rnd(cw + 20) as i32 - 10;
            let y0 = rnd(ch + 20) as i32 - 10;
            let rect = if rnd(8) == 0 { IRect::EMPTY } else { IRect::new(x0, y0, x0 + 1 + rnd(30) as i32, y0 + 1 + rnd(30) as i32) };
            let seed = rnd(1 << 30);
            let holes = rnd(3);
            let mut l = layer_from(&format!("l{case}.{k}"), 1 + k as u32 * 3, rect, |x, y| {
                let h = (x as u32).wrapping_mul(2_654_435_761) ^ (y as u32).wrapping_mul(40_503) ^ seed;
                let a = match holes {
                    0 => 255,
                    1 => (h >> 8) as u8,
                    _ => if h & 3 == 0 { 0 } else { (h >> 16) as u8 },
                };
                [h as u8, (h >> 4) as u8, (h >> 12) as u8, a]
            });
            if rnd(3) == 0 {
                let mx = rnd(cw + 20) as i32 - 10;
                let my = rnd(ch + 20) as i32 - 10;
                let mrect = IRect::new(mx, my, mx + 1 + rnd(30) as i32, my + 1 + rnd(30) as i32);
                let default = if rnd(2) == 0 { 0 } else { 255 };
                let flat = rnd(4) == 0;
                let mut m = mask_from(mrect, default, |x, y| {
                    if flat { default } else { ((x * 37 + y * 11) as u32 ^ seed) as u8 }
                });
                m.disabled = rnd(5) == 0;
                l.mask = Some(m);
            }
            l.clipped = rnd(3) == 0;
            l.visible = rnd(5) != 0;
            l.opacity = if rnd(2) == 0 { 255 } else { rnd(256) as u8 };
            l.blend = BLENDS[rnd(7) as usize];
            layers.push(l);
        }
        let psd = StickerPsd { size: [cw, ch], layers, id_seed: rnd(40) };
        let (_, _, doc) = roundtrip(&psd);
        assert_eq!(doc.layers.len(), psd.layers.len(), "case {case}");
        for (i, (w, r)) in psd.layers.iter().zip(&doc.layers).enumerate() {
            assert_eq!(r.pixels.as_ref(), Some(&normalise(&w.pixels)), "case {case} layer {i}");
            assert_eq!((r.visible, r.opacity, r.clipped), (w.visible, w.opacity, w.clipped && i > 0), "case {case} layer {i}");
            assert_eq!(r.features, PsdFeatures::empty(), "case {case} layer {i}");
            match (&w.mask, &r.mask) {
                (None, None) => {}
                (Some(m), Some(rm)) => {
                    assert_eq!((rm.default, rm.disabled), (m.default, m.disabled), "case {case} layer {i}");
                    for y in -12..ch as i32 + 42 {
                        for x in -12..cw as i32 + 42 {
                            assert_eq!(rm.value(x, y), m.value(x, y), "case {case} layer {i}: mask at {x}, {y}");
                        }
                    }
                }
                other => panic!("case {case} layer {i}: mask {other:?}"),
            }
        }
        let want = composite(&psd).unwrap();
        assert_eq!(read_composite(&doc), want, "case {case}");
        assert_unmatted_close(doc.merged.as_ref().unwrap(), &want, 0, &format!("case {case}"));
    }
}

// ---- psd_check.py input and performance -------------------------------------------------------------------------

#[test]
#[ignore = "writes WTE PSDs and their expectations for tools/sticker/psd_check.py (WTE_PSD_CHECK_OUT or target/sticker/p5b/psd_check)"]
fn k5_write_dump_for_psd_check() {
    let dir = std::env::var_os("WTE_PSD_CHECK_OUT").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sticker/p5b/psd_check")
    });
    std::fs::create_dir_all(&dir).unwrap();
    let mut cases = vec![("rich", rich_sticker()), ("f1w", f1w_sticker())];
    let mut opaque = f1w_sticker();
    opaque.layers.insert(0, layer_from("ground", 9, IRect::from_size(64, 48), |x, y| [x as u8 * 4, y as u8 * 5, 90, 255]));
    cases.push(("opaque", opaque));
    let mut big = rich_sticker();
    big.size = [300, 200];
    big.layers.push(disc("big disc", 99, IRect::new(-20, -20, 320, 220), (150.0, 100.0), (60.0, 110.0), [30, 120, 220]));
    cases.push(("big", big));
    for (name, psd) in cases {
        let bytes = write_sticker_to_vec(&psd).unwrap();
        walk(&bytes).unwrap();
        std::fs::write(dir.join(format!("{name}.psd")), &bytes).unwrap();
        let merged = composite(&psd).unwrap();
        let png = crate::image_io::encode_png(merged.width, merged.height, &merged.data, crate::image_io::PngLevel::Fast).unwrap();
        std::fs::write(dir.join(format!("{name}_merged.png")), png).unwrap();
        let layers: Vec<serde_json::Value> = psd
            .layers
            .iter()
            .map(|l| {
                let r = normalise(&l.pixels).rect;
                serde_json::json!({ "name": l.name, "rect": [r.x0, r.y0, r.x1, r.y1], "visible": l.visible,
                                    "opacity": l.opacity, "mask": l.mask.is_some() })
            })
            .collect();
        let expect = serde_json::json!({ "size": psd.size, "layers": layers, "merged": format!("{name}_merged.png") });
        std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(&expect).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "performance budget of STICKER_PLAN 8.4: run with --release"]
fn perf_psd_write_and_read_2048_five_layers() {
    let n = 2048;
    let full = IRect::from_size(n, n);
    let layers: Vec<StickerPsdLayer> = (0..5u32)
        .map(|i| {
            let c = (400.0 + 300.0 * i as f32, 1024.0);
            disc(&format!("layer {i}"), i + 1, full, c, (200.0, 700.0), [30 * i as u8, 90, 200])
        })
        .collect();
    let psd = StickerPsd { size: [n, n], layers, id_seed: 5 };
    let t = std::time::Instant::now();
    let bytes = write_sticker_to_vec(&psd).unwrap();
    let write_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = std::time::Instant::now();
    let doc = read(&bytes, &PsdLimits::default()).unwrap();
    let read_ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(doc.layers.len(), 5);
    println!("psd 2048x2048, 5 layers, {} bytes: write {write_ms:.1} ms, read {read_ms:.1} ms (budget 150 / 150)", bytes.len());
    assert!(write_ms < 150.0 * 1.5 && read_ms < 150.0 * 1.5, "write {write_ms:.1} ms, read {read_ms:.1} ms");
}
