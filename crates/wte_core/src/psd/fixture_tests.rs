//! Fixture tests (stickers P5a): the 55 Photoshop-saved files of STICKER_PLAN 13.4 (`k5_ps_*`, one per row of
//! its table), the PhotoCraft fixtures of 8.3 against PhotoCraft's own PNGs (`k5_pc_*`), the truncation sweep and
//! byte flips.

use std::path::PathBuf;

use super::read::{RESYNCS, parse};
use super::testkit::*;
use super::*;
use crate::layer::BlendMode;

pub(super) fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/psd")
}

fn ps_bytes(rel: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join("photoshop").join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn ps(rel: &str) -> PsdDoc {
    read(&ps_bytes(rel), &PsdLimits::default()).unwrap_or_else(|e| panic!("{rel}: {e:?}"))
}

/// The 55 repository fixtures (13.4, `repo_set.txt`).
pub(super) const PS_FIXTURES: [&str; 55] = [
    "layers/pixel-layer.psd",
    "layers-minimal/pixel-layer.psd",
    "layers/type-layer.psd",
    "layers-minimal/type-layer.psd",
    "layers/smartobject-layer.psd",
    "layers-minimal/smartobject-layer.psd",
    "layers/shape-layer.psd",
    "layers-minimal/shape-layer.psd",
    "stroke.psd",
    "layers/solid-color-fill.psd",
    "layers/gradient-fill.psd",
    "layers-minimal/solid-color-fill.psd",
    "layers/group.psd",
    "masks3.psd",
    "layers/levels.psd",
    "layers/curves.psd",
    "clip-adjustment.psd",
    "effects/effects-enabled.psd",
    "effects/outside-stroke.psd",
    "transparency/clip-opacity.psd",
    "transparency/fill-opacity.psd",
    "mask.psd",
    "mask-disabled.psd",
    "mask_parameters.psd",
    "masks/2.psd",
    "clipping-mask3.psd",
    "hidden-layer.psd",
    "semi-transparent-layers.psd",
    "blend-modes/multiply.psd",
    "blend-modes/screen.psd",
    "blend-modes/overlay.psd",
    "blend-modes/linear-dodge.psd",
    "blend-modes/darken.psd",
    "blend-modes/lighten.psd",
    "blend-modes/color-dodge.psd",
    "blend-modes/pass-through.psd",
    "layer-name-emoji.psd",
    "transparentbg-gimp.psd",
    "16bit5x5.psd",
    "16bit5x5.psb",
    "colormodes/4x4_16bit_rgb.psd",
    "32bit5x5.psd",
    "colormodes/4x4_8bit_lab.psd",
    "gradients/noise-gradient-cmyk.psd",
    "colormodes/4x4_8bit_index_color.psd",
    "colormodes/4x4_1bit_bitmap.psd",
    "colormodes/4x4_8bit_grayscale.psd",
    "colorprofiles/north_america_newspaper.psd",
    "slices.psd",
    "vector-mask2.psd",
    "gradient-sizes.psd",
    "colormodes/4x4_16bit_grayscale.psd",
    "colormodes/4x4_16bit_multichannel.psd",
    "colormodes/4x4_8bit_duotone.psd",
    "layers/pattern-fill.psb",
];

/// The five fixtures over 64 KB (13.4: the sweep takes 200 prefixes of each in the ordinary run).
const LARGE: [&str; 5] =
    ["stroke.psd", "mask-disabled.psd", "gradient-sizes.psd", "mask.psd", "transparency/fill-opacity.psd"];

/// The refused fixtures with the refusal each gets.
const REFUSED: [(&str, PsdRefusal); 7] = [
    ("32bit5x5.psd", PsdRefusal::Depth32),
    ("colormodes/4x4_8bit_lab.psd", PsdRefusal::Lab),
    ("gradients/noise-gradient-cmyk.psd", PsdRefusal::Cmyk),
    ("colormodes/4x4_8bit_index_color.psd", PsdRefusal::Indexed),
    ("colormodes/4x4_1bit_bitmap.psd", PsdRefusal::Bitmap),
    ("colormodes/4x4_8bit_duotone.psd", PsdRefusal::Duotone),
    ("colormodes/4x4_16bit_multichannel.psd", PsdRefusal::Multichannel),
];

fn composite_vs_merged(doc: &PsdDoc) -> u8 {
    max_diff(&composite(doc), doc.merged.as_ref().expect("a real merged picture"), 16)
}

#[test]
fn k5_ps_every_fixture_is_present_and_readable_or_refused() {
    let mut total = 0;
    for rel in PS_FIXTURES {
        let bytes = ps_bytes(rel);
        total += bytes.len();
        match REFUSED.iter().find(|(r, _)| *r == rel) {
            Some((_, refusal)) => assert_eq!(read(&bytes, &PsdLimits::default()), Err(PsdError::Unsupported(*refusal)), "{rel}"),
            None => {
                let p = parse(&bytes, &PsdLimits::default()).unwrap_or_else(|e| panic!("{rel}: {e:?}"));
                assert!(!p.doc.too_large, "{rel}");
            }
        }
    }
    assert_eq!(total, 1_970_184, "the 55 files of repo_set.txt");
    let licence = std::fs::read_to_string(fixture_dir().join("photoshop/LICENSE-psd-tools.txt")).unwrap();
    assert!(licence.starts_with("Copyright (c) 2019 Kota Yamaguchi"));
}

#[test]
fn k5_ps_pixel_layer() {
    let doc = ps("layers/pixel-layer.psd");
    assert_eq!(doc.layers.len(), 1);
    let l = &doc.layers[0];
    assert_eq!((l.lyid, l.rect), (Some(3), IRect::new(1, 1, 30, 30)));
    assert!(l.pixels.is_some() && l.features.is_empty());
    assert!(composite_vs_merged(&doc) <= 1);
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_minimal_pixel_layer_imports_without_a_merged_picture() {
    let doc = ps("layers-minimal/pixel-layer.psd");
    assert!(!doc.has_real_merged && doc.merged.is_none());
    assert!(doc.layers[0].pixels.as_ref().is_some_and(|p| !p.is_empty()));
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_type_layers() {
    for rel in ["layers/type-layer.psd", "layers-minimal/type-layer.psd"] {
        let doc = ps(rel);
        assert_eq!(doc.layers.len(), 1, "{rel}");
        let l = &doc.layers[0];
        assert_eq!(l.features, PsdFeatures::TEXT, "{rel}");
        assert!(l.pixels.as_ref().is_some_and(|p| !p.is_empty()), "{rel}");
        assert!(doc.forcing(true).is_empty() && doc.editor_native(), "{rel}");
        assert_eq!(mode(&doc, true), Mode::Layers, "{rel}");
    }
    assert_eq!(composite_vs_merged(&ps("layers/type-layer.psd")), 0);
}

#[test]
fn k5_ps_smart_objects() {
    let full = ps("layers/smartobject-layer.psd");
    let minimal = ps("layers-minimal/smartobject-layer.psd");
    for (doc, merged_mode) in [(&full, Mode::Merged), (&minimal, Mode::NeedsMerged)] {
        assert_eq!(doc.layers[0].features, PsdFeatures::SMART);
        assert!(doc.layers[0].pixels.as_ref().is_some_and(|p| !p.is_empty()), "pixels with MaxCompat on and off");
        // A Photoshop link (smart objects force).
        assert_eq!(doc.forcing(true), PsdFeatures::SMART);
        assert!(doc.forcing(true).words().contains(&"smart objects"));
        assert_eq!(mode(doc, true), merged_mode);
        // A PhotoCraft link, and the option of decision D3.
        assert!(doc.forcing(false).is_empty());
        assert_eq!(mode(doc, false), Mode::Layers);
        assert!(doc.editor_native());
    }
    assert_eq!(composite_vs_merged(&full), 0);
}

#[test]
fn k5_ps_shape_layer_with_pixels() {
    let doc = ps("layers/shape-layer.psd");
    let l = &doc.layers[0];
    assert_eq!(l.flags & 0x10, 0x10);
    assert_eq!(l.features, PsdFeatures::FILL, "a picture layer, no VECTOR_MASK");
    assert!(composite_vs_merged(&doc) <= 1);
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_minimal_shape_layer_needs_a_merged_picture() {
    let doc = ps("layers-minimal/shape-layer.psd");
    assert_eq!(doc.layers[0].features, PsdFeatures::NO_PIXELS | PsdFeatures::FILL);
    assert_eq!(doc.forcing(true), PsdFeatures::NO_PIXELS);
    assert!(!doc.has_real_merged);
    assert_eq!(mode(&doc, true), Mode::NeedsMerged);
}

#[test]
fn k5_ps_stroke_shapes_are_finished_pictures() {
    let doc = ps("stroke.psd");
    assert_eq!(doc.layers.len(), 6);
    let bg = &doc.layers[0];
    assert_eq!(bg.name, "Background");
    assert!(bg.pixels.as_ref().unwrap().data.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "no channel -1 = opaque");
    assert!(!bg.features.contains(PsdFeatures::BLEND_IF));
    for l in &doc.layers[1..] {
        assert_eq!(l.features, PsdFeatures::FILL, "{}", l.name);
        assert!(l.pixels.as_ref().is_some_and(|p| !p.is_empty()), "{}", l.name);
    }
    assert_eq!((doc.layers[1].opacity, doc.layers[2].opacity), (194, 148));
    assert!(composite_vs_merged(&doc) <= 2, "shape pixels are final");
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_fill_layers_with_masks() {
    for rel in ["layers/solid-color-fill.psd", "layers/gradient-fill.psd"] {
        let doc = ps(rel);
        let l = &doc.layers[0];
        assert_eq!(l.features, PsdFeatures::FILL, "{rel}: 5 channels with 40-byte ranges, not BLEND_IF");
        assert!(l.mask.is_some() && l.pixels.is_some(), "{rel}");
        assert!(composite_vs_merged(&doc) <= 1, "{rel}");
        assert_eq!(mode(&doc, true), Mode::Layers, "{rel}");
    }
}

#[test]
fn k5_ps_minimal_fill_layer() {
    let doc = ps("layers-minimal/solid-color-fill.psd");
    assert!(doc.layers[0].features.contains(PsdFeatures::NO_PIXELS));
    assert!(doc.layers[0].pixels.is_none());
    assert!(!doc.has_real_merged);
    assert_eq!(mode(&doc, true), Mode::NeedsMerged);
}

#[test]
fn k5_ps_pattern_fill_psb() {
    RESYNCS.with(|r| r.set(0));
    let bytes = ps_bytes("layers/pattern-fill.psb");
    let p = parse(&bytes, &PsdLimits::default()).unwrap();
    assert_eq!(RESYNCS.with(|r| r.get()), 0, "the 8B64 cinf block (8-byte length) is stepped over exactly");
    let doc = p.doc;
    assert!(doc.psb && doc.depth == 8);
    let l = &doc.layers[0];
    assert_eq!((l.rect, l.flags), (IRect::new(0, 0, 32, 32), 0x18));
    assert_eq!(l.features, PsdFeatures::FILL);
    assert!(l.pixels.is_some() && l.mask.is_some());
    assert_eq!(mode(&doc, true), Mode::Layers);
    // Measured by this builder: the composite equals the merged picture (STICKER_PLAN 13.4 asked for the number).
    assert_eq!(composite_vs_merged(&doc), 0);
}

#[test]
fn k5_ps_groups() {
    for rel in ["layers/group.psd", "masks3.psd"] {
        let doc = ps(rel);
        assert!(doc.forcing(true).contains(PsdFeatures::GROUP), "{rel}");
        assert!(doc.forcing(true).words().contains(&"groups"), "{rel}");
        let end = doc.layers.iter().find(|l| l.section == PsdSection::GroupEnd).unwrap();
        assert_eq!(end.features, PsdFeatures::GROUP, "{rel}: the end marker is not NO_PIXELS");
        assert!(doc.layers.iter().any(|l| l.section == PsdSection::GroupOpen), "{rel}");
        assert_eq!(mode(&doc, true), Mode::Merged, "{rel}");
    }
}

#[test]
fn k5_ps_artboards() {
    RESYNCS.with(|r| r.set(0));
    let doc = ps("gradient-sizes.psd");
    assert_eq!(RESYNCS.with(|r| r.get()), 0, "artd and cinf are skipped exactly");
    let boards: Vec<_> = doc.layers.iter().filter(|l| l.name.starts_with("Artboard")).collect();
    assert_eq!(boards.len(), 3);
    assert!(boards.iter().all(|l| l.section == PsdSection::GroupOpen && l.features.contains(PsdFeatures::GROUP)));
    let shapes: Vec<_> = doc.layers.iter().filter(|l| l.name.starts_with("Rectangle")).collect();
    assert_eq!(shapes.len(), 3);
    for s in &shapes {
        assert_eq!(s.features, PsdFeatures::FILL, "{}", s.name);
        assert!(s.pixels.is_some(), "{}", s.name);
        assert!(s.rect.x0 < 0 || s.rect.y0 < 0 || s.rect.x1 > 105 || s.rect.y1 > 105, "reaches outside");
    }
    let merged = doc.merged.as_ref().unwrap();
    assert!(merged.data.as_chunks::<4>().0.iter().any(|p| p[3] < 255), "negative layer count = a transparent merged picture");
    assert!(composite_vs_merged(&doc) <= 2);
    assert_eq!(mode(&doc, true), Mode::Merged);
}

#[test]
fn k5_ps_adjustment_layers() {
    for rel in ["layers/levels.psd", "layers/curves.psd", "clip-adjustment.psd"] {
        let doc = ps(rel);
        assert!(doc.forcing(true).contains(PsdFeatures::ADJUSTMENT), "{rel}");
        assert_eq!(mode(&doc, true), Mode::Merged, "{rel}");
    }
    let doc = ps("clip-adjustment.psd");
    let hue = layer(&doc, "Hue/Saturation 1");
    assert!(hue.clipped && hue.features.contains(PsdFeatures::ADJUSTMENT));
    assert_eq!(layer(&doc, "a").features, PsdFeatures::TEXT);
}

#[test]
fn k5_ps_effects_walker_per_layer() {
    let bytes = ps_bytes("effects/effects-enabled.psd");
    let p = parse(&bytes, &PsdLimits::default()).unwrap();
    let doc = &p.doc;
    // Each layer has lfx2 and lrFX; only lfx2 decides.
    assert_eq!(layer(doc, "Layer 1").features, PsdFeatures::EFFECTS, "master on, one effect enabled");
    assert_eq!(layer(doc, "Layer 2").features, PsdFeatures::empty(), "master on, every effect off");
    assert_eq!(layer(doc, "Layer 3").features, PsdFeatures::empty(), "master off");
    assert_eq!(mode(doc, true), Mode::NeedsMerged);
    // With Layer 1 hidden (flags bit 1, patched in the bytes) the file imports layer by layer.
    let i = doc.layers.iter().position(|l| l.name == "Layer 1").unwrap();
    let mut patched = bytes.clone();
    patched[p.flag_offsets[i]] |= 0x02;
    let doc = read(&patched, &PsdLimits::default()).unwrap();
    assert!(!layer(&doc, "Layer 1").visible);
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_outside_stroke() {
    let doc = ps("effects/outside-stroke.psd");
    assert_eq!(doc.forcing(true), PsdFeatures::EFFECTS);
    assert!(doc.forcing(true).words().contains(&"layer effects"));
    assert_eq!(mode(&doc, true), Mode::Merged);
    // The merged picture holds the stroke: it differs from the layer's own pixels where the stroke is.
    let own = composite(&doc);
    let merged = doc.merged.as_ref().unwrap();
    let differ = own.data.as_chunks::<4>().0.iter().zip(merged.data.as_chunks::<4>().0.iter()).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 224, "of 1,024 pixels");
}

#[test]
fn k5_ps_fill_opacity() {
    for (rel, value) in [("transparency/clip-opacity.psd", 128), ("transparency/fill-opacity.psd", 0)] {
        let doc = ps(rel);
        assert!(doc.forcing(true).contains(PsdFeatures::FILL_OPACITY), "{rel}");
        assert!(doc.layers.iter().any(|l| l.fill_opacity == value), "{rel}");
        assert_eq!(mode(&doc, true), Mode::Merged, "{rel}");
    }
}

#[test]
fn k5_ps_user_masks() {
    for (rel, disabled) in [("mask.psd", false), ("mask-disabled.psd", true)] {
        let doc = ps(rel);
        let l = layer(&doc, "Background copy");
        let m = l.mask.as_ref().unwrap();
        assert_eq!((m.rect, m.default, m.disabled), (IRect::new(23, 10, 94, 67), 0, disabled), "{rel}");
        assert!(l.features.is_empty(), "{rel}: 5 channels with 40-byte ranges, not BLEND_IF");
        assert_eq!(mode(&doc, true), Mode::Layers, "{rel}");
        assert!(composite_vs_merged(&doc) <= 1, "{rel}");
    }
}

#[test]
fn k5_ps_mask_parameters() {
    let doc = ps("mask_parameters.psd");
    let l = layer(&doc, "Rectangle 1");
    assert!(l.features.contains(PsdFeatures::MASK_EXTRA));
    assert!(doc.forcing(true).contains(PsdFeatures::MASK_EXTRA));
    assert_eq!(mode(&doc, true), Mode::NeedsMerged);
}

#[test]
fn k5_ps_vector_masks_on_pixel_layers() {
    let doc = ps("masks/2.psd");
    assert_eq!(layer(&doc, "ellipse1").features, PsdFeatures::VECTOR_MASK);
    assert_eq!(layer(&doc, "ellipse3").features, PsdFeatures::VECTOR_MASK);
    assert_eq!(layer(&doc, "ellipse2").features, PsdFeatures::NO_PIXELS | PsdFeatures::FILL, "a shape without pixels");
    assert!(doc.layers.iter().all(|l| l.opacity == 128));
    assert_eq!(mode(&doc, true), Mode::NeedsMerged);
}

#[test]
fn k5_ps_clipping() {
    let doc = ps("clipping-mask3.psd");
    assert!(layer(&doc, "Rectangle 2").clipped);
    assert!(!layer(&doc, "Rectangle 1").clipped);
    assert_eq!(mode(&doc, true), Mode::Merged, "the file has a group");
}

#[test]
fn k5_ps_hidden_layer_and_rects_outside_the_canvas() {
    let doc = ps("hidden-layer.psd");
    assert!(!layer(&doc, "Shape 2").visible && layer(&doc, "Shape 1").visible);
    assert!(layer(&doc, "Background").pixels.as_ref().unwrap().data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    assert_eq!(mode(&doc, true), Mode::Layers);
    assert!(composite_vs_merged(&doc) <= 1);
    let doc = ps("semi-transparent-layers.psd");
    assert_eq!(layer(&doc, "Rectangle 1").rect, IRect::new(-7, 50, 108, 102), "reaches outside 100 x 100");
    assert!(composite_vs_merged(&doc) <= 1, "clipped to the canvas when used");
}

#[test]
fn k5_ps_blend_modes() {
    for (rel, m) in [
        ("blend-modes/multiply.psd", BlendMode::Multiply),
        ("blend-modes/screen.psd", BlendMode::Screen),
        ("blend-modes/overlay.psd", BlendMode::Overlay),
        ("blend-modes/linear-dodge.psd", BlendMode::Add),
        ("blend-modes/darken.psd", BlendMode::Darken),
        ("blend-modes/lighten.psd", BlendMode::Lighten),
    ] {
        let doc = ps(rel);
        assert!(doc.layers.iter().any(|l| l.blend == PsdBlend::Known(m)), "{rel}");
        assert_eq!(mode(&doc, true), Mode::Layers, "{rel}");
        // WTE's blend kernel against Photoshop's merged picture (measured: 2 to 3 levels).
        assert!(composite_vs_merged(&doc) <= 3, "{rel}");
    }
}

#[test]
fn k5_ps_color_dodge_and_pass_through() {
    let doc = ps("blend-modes/color-dodge.psd");
    assert!(doc.layers.iter().any(|l| l.blend == PsdBlend::Other(*b"div ")));
    assert_eq!(doc.forcing(true), PsdFeatures::BLEND_OTHER);
    assert_eq!(mode(&doc, true), Mode::Merged);
    let doc = ps("blend-modes/pass-through.psd");
    assert!(doc.layers.iter().any(|l| l.blend == PsdBlend::Other(*b"pass") && l.section == PsdSection::GroupOpen));
    assert_eq!(mode(&doc, true), Mode::Merged);
}

#[test]
fn k5_ps_emoji_name_raw_channels() {
    let doc = ps("layer-name-emoji.psd");
    let l = &doc.layers[0];
    assert_eq!(l.name, "\u{1F47D}");
    assert_eq!((l.blend, l.opacity), (PsdBlend::Known(BlendMode::Add), 128));
    assert!(l.pixels.as_ref().is_some_and(|p| p.rect == IRect::new(0, 0, 4, 4)));
    assert!(!doc.has_real_merged);
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_gimp_file() {
    let doc = ps("transparentbg-gimp.psd");
    assert!(doc.has_real_merged, "no resource 1057 = real");
    let l = &doc.layers[0];
    assert_eq!(l.lyid, None);
    assert_eq!(l.name, "\u{0424}\u{043E}\u{043D}", "a UTF-8 Pascal name without luni");
    assert_eq!(composite_vs_merged(&doc), 0);
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_16bit_psd_and_psb() {
    let psd = ps("16bit5x5.psd");
    let psb = ps("16bit5x5.psb");
    assert!(!psd.psb && psb.psb);
    assert_eq!(psd.depth, 16);
    assert!(!psd.is_flat(), "main layer count 0, three layers in Lr16");
    assert_eq!(psd.layers.len(), 3);
    assert_eq!(layer(&psd, "Background copy 2").rect, IRect::new(4, 1, 5, 4));
    assert_eq!(PsdDoc { psb: false, ..psb.clone() }, psd, "PSD and PSB give the same document");
    assert!(composite_vs_merged(&psd) <= 1);
    assert_eq!(mode(&psd, true), Mode::Layers);
}

#[test]
fn k5_ps_16bit_rgb_fill_without_pixels() {
    let doc = ps("colormodes/4x4_16bit_rgb.psd");
    assert_eq!(doc.depth, 16);
    let empty = layer(&doc, "Layer 1");
    assert_eq!(empty.pixels, Some(CroppedBuf::empty()), "an ordinary empty layer");
    assert!(empty.features.is_empty());
    let fill = layer(&doc, "Gradient Fill 1");
    assert!(fill.features.contains(PsdFeatures::NO_PIXELS));
    assert!(fill.mask.as_ref().is_some_and(|m| m.rect == IRect::new(0, 0, 4, 4)), "ZIP-prediction mask channel");
    assert!(doc.has_real_merged);
    assert_eq!(mode(&doc, true), Mode::Merged);
}

#[test]
fn k5_ps_16bit_grayscale() {
    let doc = ps("colormodes/4x4_16bit_grayscale.psd");
    assert_eq!((doc.depth, doc.mode), (16, PsdColorMode::Grayscale));
    assert!(layer(&doc, "Gradient Fill 1").features.contains(PsdFeatures::NO_PIXELS));
    let merged = doc.merged.as_ref().unwrap();
    assert!(merged.data.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
    assert_eq!(doc.profile_note(), None);
    assert_eq!(mode(&doc, true), Mode::Merged);
}

#[test]
fn k5_ps_8bit_grayscale() {
    let doc = ps("colormodes/4x4_8bit_grayscale.psd");
    assert_eq!(doc.mode, PsdColorMode::Grayscale);
    assert!(doc.icc_description.is_some() && doc.profile_note().is_none(), "the grey profile gives no note");
    assert_eq!(layer(&doc, "Layer 1").pixels, Some(CroppedBuf::empty()));
    let fill = layer(&doc, "Gradient Fill 1");
    assert_eq!(fill.features, PsdFeatures::FILL, "2 and 3 channels with 40-byte ranges are not BLEND_IF");
    assert!(fill.mask.is_some());
    assert!(fill.pixels.as_ref().unwrap().data.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(mode(&doc, true), Mode::Layers);
    assert!(composite_vs_merged(&doc) <= 1);
}

#[test]
fn k5_ps_profile_note() {
    let doc = ps("colorprofiles/north_america_newspaper.psd");
    assert_eq!(doc.profile_note(), Some("Adobe RGB (1998)"));
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(mode(&doc, true), Mode::Layers);
}

#[test]
fn k5_ps_refused_modes_and_depths() {
    for (rel, refusal) in REFUSED {
        assert_eq!(read(&ps_bytes(rel), &PsdLimits::default()), Err(PsdError::Unsupported(refusal)), "{rel}");
        let s = refusal.sentence("x.psd");
        assert!(s.contains("In Photoshop choose Image > Mode >"), "{rel}: {s}");
    }
}

#[test]
fn k5_ps_flat_file() {
    let doc = ps("slices.psd");
    assert!(doc.is_flat() && doc.has_real_merged);
    let m = doc.merged.as_ref().unwrap();
    assert_eq!((m.width, m.height), (240, 180));
    assert_eq!(mode(&doc, true), Mode::Merged, "one picture from the merged data");
}

#[test]
fn k5_ps_inverted_rect() {
    let doc = ps("vector-mask2.psd");
    assert_eq!(doc.layers.len(), 10);
    let l = layer(&doc, "Gradient Fill 1");
    assert_eq!(l.rect, IRect::EMPTY, "(0, 0, -1, 0) is an empty layer, not an error");
    assert!(l.features.contains(PsdFeatures::NO_PIXELS));
}

// --- PhotoCraft fixtures (STICKER_PLAN 8.3, made in P0 with the real v0.2.0 CLI) ---

fn pc(name: &str) -> PsdDoc {
    let bytes = std::fs::read(fixture_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    read(&bytes, &PsdLimits::default()).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

pub(super) fn png(path: PathBuf) -> PixelBuf {
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
    PixelBuf::from_raw(img.width(), img.height(), img.into_raw()).unwrap()
}

/// PhotoCraft's per-layer PNGs, bottom to top (the files count from the top layer).
pub(super) fn pc_layer_pngs(stem: &str) -> Vec<(String, PixelBuf)> {
    let dir = fixture_dir().join(format!("{stem}_layers"));
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names.reverse();
    names.into_iter().map(|n| (n.clone(), png(dir.join(n)))).collect()
}

/// A layer as PhotoCraft's layer export shows it: its pixels on the canvas with the mask and the layer opacity
/// applied.
fn visible_layer(doc: &PsdDoc, l: &PsdLayer) -> PixelBuf {
    let mut out = l.pixels.as_ref().unwrap().to_dense(doc.size[0], doc.size[1]);
    let mask = l.mask.as_ref().filter(|m| !m.disabled);
    for y in 0..doc.size[1] {
        for x in 0..doc.size[0] {
            let i = ((y * doc.size[0] + x) * 4 + 3) as usize;
            let m = mask.map_or(255, |m| m.value(x as i32, y as i32)) as u32;
            let a = (out.data[i] as u32 * m + 127) / 255;
            out.data[i] = ((a * l.opacity as u32 + 127) / 255) as u8;
        }
    }
    out
}

/// Alpha equal everywhere, colour equal where alpha > 0, within `tol`.
fn same_visible(a: &PixelBuf, b: &PixelBuf, tol: u8) -> bool {
    a.data.as_chunks::<4>().0.iter().zip(b.data.as_chunks::<4>().0.iter()).all(|(p, q)| {
        p[3].abs_diff(q[3]) <= tol && (p[3] == 0 || (0..3).all(|k| p[k].abs_diff(q[k]) <= tol))
    })
}

#[test]
fn k5_pc_layer_pixels_equal_photocrafts_pngs() {
    // (stem, tolerance): 0 = exactly; f7's PNGs are 16-bit (the image crate's reduction rounds like ours within 1).
    // f4 (the stroke effect is in its PNG) and f6 (a group exports as one picture) are compared through the merged
    // picture only.
    for (stem, tol) in [
        ("f1_pc", 0),
        ("f2_feathered", 0),
        ("f3_opaque", 0),
        ("f4b_mask", 0),
        ("f5_f1_plus_text", 0),
        ("f7_16bit", 1),
        ("f8_hidden_opacity_multiply", 0),
        ("f8b_color_dodge", 0),
        ("f9_alpha_channel", 0),
        ("f11_clipped", 0),
        ("f12_lyid_reuse", 0),
    ] {
        let doc = pc(&format!("{stem}.psd"));
        let pngs = pc_layer_pngs(stem);
        assert_eq!(pngs.len(), doc.layers.len(), "{stem}");
        for ((file, want), l) in pngs.iter().zip(&doc.layers) {
            assert!(file.ends_with(&format!("_{}.png", l.name)), "{stem}: {file} vs {}", l.name);
            assert!(same_visible(&visible_layer(&doc, l), want, tol), "{stem}: layer {}", l.name);
        }
    }
    // The PSB of f1 has the same layers as f1.
    let (psd, psb) = (pc("f1_pc.psd"), pc("f10_f1.psb"));
    assert_eq!(PsdDoc { psb: false, ..psb }, psd);
}

#[test]
fn k5_pc_merged_equals_photocrafts_convert_png() {
    for stem in [
        "f1_pc", "f2_feathered", "f3_opaque", "f4_mask_stroke", "f4b_mask", "f5_f1_plus_text", "f6_group", "f7_16bit",
        "f8_hidden_opacity_multiply", "f8b_color_dodge", "f9_alpha_channel", "f11_clipped", "f12_lyid_reuse",
    ] {
        let doc = pc(&format!("{stem}.psd"));
        let want = png(fixture_dir().join(format!("{stem}_merged.png")));
        let got = doc.merged.as_ref().unwrap();
        // Alpha exact. Colour for alpha >= 16 within the un-matting bound: PhotoCraft stores the merged colours
        // matted on white in 8 bits, so a stored level hides 255 / a levels of the straight colour; the bound is
        // 1 level at alpha 255 and 1 + ceil(127.5 / a) below it (9 at alpha 16).
        for (p, q) in got.data.as_chunks::<4>().0.iter().zip(want.data.as_chunks::<4>().0.iter()) {
            assert_eq!(p[3], q[3], "{stem}: alpha");
            let a = p[3] as u32;
            if a < 16 {
                continue;
            }
            let bound = if a == 255 { 1 } else { 1 + 255_u32.div_ceil(2 * a) };
            for k in 0..3 {
                let d = p[k].abs_diff(q[k]) as u32;
                assert!(d <= bound, "{stem}: colour {d} levels off at alpha {a} (bound {bound})");
            }
        }
    }
}

#[test]
fn k5_pc_fixture_features() {
    let ids = |d: &PsdDoc| d.layers.iter().map(|l| l.lyid).collect::<Vec<_>>();
    let f1 = pc("f1_pc.psd");
    assert_eq!(ids(&f1), [Some(1), Some(2), Some(3)]);
    assert_eq!(mode(&f1, false), Mode::Layers);
    let f4 = pc("f4_mask_stroke.psd");
    assert_eq!(f4.forcing(false), PsdFeatures::EFFECTS);
    assert_eq!(mode(&f4, false), Mode::Merged);
    let f4b = pc("f4b_mask.psd");
    let m = f4b.layers[0].mask.as_ref().unwrap();
    assert_eq!((m.rect, m.default), (IRect::new(11, 7, 53, 41), 0));
    assert_eq!(mode(&f4b, false), Mode::Layers);
    let f5 = pc("f5_f1_plus_text.psd");
    assert_eq!(ids(&f5), [Some(1), Some(2), Some(3), Some(4)]);
    assert_eq!(f5.layers[3].features, PsdFeatures::TEXT);
    assert!(f5.editor_native() && f5.forcing(false).is_empty());
    let f6 = pc("f6_group.psd");
    assert!(f6.forcing(false).contains(PsdFeatures::GROUP));
    assert_eq!(mode(&f6, false), Mode::Merged);
    let f7 = pc("f7_16bit.psd");
    assert_eq!(f7.depth, 16);
    assert!(!f7.is_flat(), "layers in Lr16");
    let f8 = pc("f8_hidden_opacity_multiply.psd");
    assert!(!layer(&f8, "hidden").visible);
    assert_eq!(layer(&f8, "half").opacity, 128);
    assert_eq!(layer(&f8, "multiply").blend, PsdBlend::Known(BlendMode::Multiply));
    assert_eq!(mode(&f8, false), Mode::Layers);
    let f8b = pc("f8b_color_dodge.psd");
    assert_eq!(f8b.forcing(false), PsdFeatures::BLEND_OTHER);
    let f9 = pc("f9_alpha_channel.psd");
    assert!(f9.merged.as_ref().unwrap().data.as_chunks::<4>().0.iter().any(|p| p[3] == 0), "the 4th channel is transparency");
    let f11 = pc("f11_clipped.psd");
    assert!(layer(&f11, "clipped").clipped);
    assert_eq!(mode(&f11, false), Mode::Layers);
    let f12 = pc("f12_lyid_reuse.psd");
    assert_eq!(ids(&f12), [Some(1), Some(3), Some(2)], "the new layer took the freed id 2");
    // The WTE-written input of f1 (no lyid, no 1057, empty blending ranges).
    let f1in = pc("f1_wte_in.psd");
    assert!(f1in.has_real_merged && f1in.layers.iter().all(|l| l.lyid.is_none() && l.features.is_empty()));
}

// --- Truncation sweep and byte flips (13.4) ---

/// Every repository PSD plus the PhotoCraft fixtures, with their bytes.
fn all_readable() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = PS_FIXTURES
        .iter()
        .filter(|r| !REFUSED.iter().any(|(x, _)| x == *r))
        .map(|r| (r.to_string(), ps_bytes(r)))
        .collect();
    for e in std::fs::read_dir(fixture_dir()).unwrap() {
        let p = e.unwrap().path();
        if matches!(p.extension().and_then(|x| x.to_str()), Some("psd" | "psb")) {
            out.push((p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap()));
        }
    }
    out
}

/// `count` evenly spaced prefix lengths below `end` (all of them when there are fewer); none may give a document.
fn sweep(name: &str, bytes: &[u8], count: usize) {
    let end = parse(bytes, &PsdLimits::default()).unwrap().end;
    let lens: Vec<usize> = if end <= count { (0..end).collect() } else { (0..count).map(|i| i * end / count).collect() };
    for len in lens {
        assert!(read(&bytes[..len], &PsdLimits::default()).is_err(), "{name}: prefix {len} of {end} gave a document");
    }
}

#[test]
fn k5_ps_truncation_sweep() {
    let started = std::time::Instant::now();
    let mut smallest: Option<(String, Vec<u8>)> = None;
    for (name, bytes) in all_readable() {
        // Every file's data reaches its end (no trailing bytes), so every shorter prefix is half-written.
        assert_eq!(parse(&bytes, &PsdLimits::default()).unwrap().end, bytes.len(), "{name}");
        let count = if LARGE.contains(&name.as_str()) { 200 } else { 2000 };
        sweep(&name, &bytes, count);
        if smallest.as_ref().is_none_or(|(_, b)| bytes.len() < b.len()) {
            smallest = Some((name, bytes));
        }
    }
    // Every prefix length of the smallest fixture.
    let (name, bytes) = smallest.unwrap();
    for len in 0..bytes.len() {
        assert!(read(&bytes[..len], &PsdLimits::default()).is_err(), "{name}: prefix {len}");
    }
    eprintln!("k5_ps_truncation_sweep: {:.1} s", started.elapsed().as_secs_f64());
}

/// The full 2,000 prefixes of the five large fixtures (13.4; run by the checker and the gate with `--release`).
#[test]
#[ignore]
fn k5_ps_sweep_large() {
    for rel in LARGE {
        sweep(rel, &ps_bytes(rel), 2000);
    }
}

#[test]
fn k5_ps_byte_flips_never_panic() {
    let started = std::time::Instant::now();
    let files: Vec<(String, Vec<u8>)> = all_readable().into_iter().filter(|(_, b)| b.len() <= 64 * 1024).collect();
    let mut rng = Rng(0x05EE_D0FF_11E5);
    let (mut oks, mut errs) = (0, 0);
    for i in 0..5000 {
        let (_, bytes) = &files[i % files.len()];
        let mut b = bytes.clone();
        // One to three flipped bytes, half of them in the first 512 bytes (header, resources, record fields).
        for _ in 0..1 + rng.below(3) {
            let at = if rng.below(2) == 0 { rng.below(b.len().min(512)) } else { rng.below(b.len()) };
            b[at] ^= 1 << rng.below(8);
        }
        match read(&b, &PsdLimits::default()) {
            Ok(_) => oks += 1,
            Err(_) => errs += 1,
        }
    }
    assert_eq!(oks + errs, 5000);
    eprintln!("k5_ps_byte_flips_never_panic: {oks} ok, {errs} refused, {:.1} s", started.elapsed().as_secs_f64());
}



/// P5a checker: one more PhotoCraft v0.2.0 CLI fixture that the builder did not make (`make_f13_check.sh`): a 16-bit
/// **PSB** (layers in the `Lr16` block with an 8-byte length, PackBits rows with 4-byte counts at 16 bits), a soft
/// user mask on the lower layer, and an upper layer named "Слой ü" (only `luni` holds it; the Pascal name is
/// `???? ?`) with opacity 153 and Screen. Its layers must equal PhotoCraft's own per-layer PNGs and its merged
/// picture PhotoCraft's `convert` PNG.
#[test]
fn k5_check_pc_16bit_psb_against_photocrafts_pngs() {
    let bytes = std::fs::read(fixture_dir().join("f13_check_16bit.psb")).unwrap();
    RESYNCS.with(|r| r.set(0));
    let p = parse(&bytes, &PsdLimits::default()).unwrap();
    assert_eq!(RESYNCS.with(|r| r.get()), 0);
    assert_eq!(p.end, bytes.len());
    let doc = p.doc;
    assert!(doc.psb && doc.depth == 16 && !doc.is_flat() && doc.has_real_merged);
    assert_eq!(doc.layers.len(), 2);
    let (lower, upper) = (&doc.layers[0], &doc.layers[1]);
    assert_eq!((lower.name.as_str(), lower.lyid), ("Layer 1", Some(1)));
    assert_eq!(upper.name, "\u{0421}\u{043B}\u{043E}\u{0439} \u{00FC}", "luni wins over the Pascal name");
    assert_eq!((upper.opacity, upper.blend), (153, PsdBlend::Known(BlendMode::Screen)));
    let m = lower.mask.as_ref().expect("the user mask");
    assert_eq!((m.rect, m.default, m.disabled), (IRect::new(6, 2, 50, 40), 0, false));
    assert!(doc.forcing(true).is_empty() && !doc.editor_native());
    assert_eq!(mode(&doc, true), Mode::Layers);
    // Layer pixels against PhotoCraft's 16-bit layer PNGs (reduced to 8 bits by the image crate: within 1 level).
    let pngs = pc_layer_pngs("f13_check_16bit");
    assert_eq!(pngs.len(), 2);
    for ((file, want), l) in pngs.iter().zip(&doc.layers) {
        assert!(file.ends_with(&format!("_{}.png", l.name)), "{file} vs {}", l.name);
        assert!(same_visible(&visible_layer(&doc, l), want, 1), "layer {}", l.name);
    }
    // The merged picture (16-bit RLE, un-matted at 16 bits) against the convert PNG: alpha within 1 (the PNG is
    // 8-bit, reduced by PhotoCraft), colour within the un-matting bound of k5_pc_merged_equals_photocrafts_convert_png.
    let want = png(fixture_dir().join("f13_check_16bit_merged.png"));
    let got = doc.merged.as_ref().unwrap();
    assert_eq!((got.width, got.height), (want.width, want.height));
    for (p, q) in got.data.as_chunks::<4>().0.iter().zip(want.data.as_chunks::<4>().0.iter()) {
        assert!(p[3].abs_diff(q[3]) <= 1, "alpha {} vs {}", p[3], q[3]);
        let a = p[3].min(q[3]) as u32;
        if a < 16 {
            continue;
        }
        let bound = if a >= 254 { 1 } else { 1 + 255_u32.div_ceil(2 * a) };
        for k in 0..3 {
            assert!(p[k].abs_diff(q[k]) as u32 <= bound, "colour {} vs {} at alpha {a}", p[k], q[k]);
        }
    }
    // And the merged picture equals the composite of the layers (WTE's Screen kernel against PhotoCraft's).
    let d = composite_vs_merged(&doc);
    assert!(d <= 2, "{d}");
}


/// P5a checker: the 8 samples of 13.4 that are not repository fixtures (two of unclear origin, six left out for size),
/// read from the planner's download when it is on this machine (`G:\Claude_Resources\ref\psd-samples\psd-tools`, or
/// the folder in `WTE_PSD_SAMPLES`). Ignored: it needs files outside the repository. Results are in
/// `docs/notes/sticker/check-P5a.md`.
#[test]
#[ignore]
fn k5_check_outside_samples() {
    let dir = std::env::var("WTE_PSD_SAMPLES").unwrap_or_else(|_| "G:/Claude_Resources/ref/psd-samples/psd-tools".into());
    let dir = PathBuf::from(dir);
    if !dir.is_dir() {
        eprintln!("k5_check_outside_samples: {} is missing; nothing checked", dir.display());
        return;
    }
    let read_at = |rel: &str| {
        let bytes = std::fs::read(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        RESYNCS.with(|r| r.set(0));
        let p = parse(&bytes, &PsdLimits::default());
        let resyncs = RESYNCS.with(|r| r.get());
        p.map(|p| {
            assert_eq!(p.end, bytes.len(), "{rel}: the walk ends at the last byte");
            assert_eq!(resyncs, 0, "{rel}: no re-sync");
            p.doc
        })
    };
    // 2layers.psd (unknown old writer): Cyrillic UTF-8 Pascal names, no lyid, no luni, no 1057 (= real).
    let d = read_at("2layers.psd").unwrap();
    assert_eq!(d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["\u{0424}\u{043E}\u{043D}", "\u{0421}\u{043B}\u{043E}\u{0439}"]);
    assert!(d.layers.iter().all(|l| l.lyid.is_none() && l.features.is_empty()));
    assert!(d.has_real_merged && mode(&d, true) == Mode::Layers);
    assert!(composite_vs_merged(&d) <= 1);
    // cactus_top.psd (third-party): channel -1 last, lsct type 0 on plain layers, a hidden Background. Layers mode.
    // Its header has 4 channels but a positive layer count, so the merged picture is read as opaque (5.2: transparency
    // only with a negative count); Layers mode does not use it.
    let d = read_at("third-party-psds/cactus_top.psd").unwrap();
    assert_eq!(d.layers.len(), 2);
    assert!(d.layers.iter().all(|l| l.section == PsdSection::Layer && l.features.is_empty() && l.pixels.is_some()));
    assert!(!d.layers[0].visible && d.layers[1].visible);
    assert_eq!(mode(&d, true), Mode::Layers);
    // empty-layer.psd: an ordinary empty layer, a shape (with a rendered mask, dropped) inside a pass-through group.
    let d = read_at("empty-layer.psd").unwrap();
    assert_eq!(layer(&d, "empty").pixels, Some(CroppedBuf::empty()));
    let shape = layer(&d, "normal");
    assert!(shape.features == PsdFeatures::FILL && shape.mask.is_none() && shape.pixels.is_some());
    assert_eq!(d.forcing(true), PsdFeatures::GROUP | PsdFeatures::BLEND_OTHER);
    assert_eq!(mode(&d, true), Mode::Merged);
    assert_eq!(composite_vs_merged(&d), 0);
    // blend-modes/normal.psd: three shape layers with pixels (vmsk, flags 0x18) at opacity 128: Layers mode.
    let d = read_at("blend-modes/normal.psd").unwrap();
    assert!(d.layers.iter().all(|l| l.features == PsdFeatures::FILL && l.opacity == 128));
    assert_eq!(mode(&d, true), Mode::Layers);
    assert!(composite_vs_merged(&d) <= 2);
    // 32-bit grayscale: the 32-bit sentence.
    assert_eq!(read_at("colormodes/4x4_32bit_grayscale.psd"), Err(PsdError::Unsupported(PsdRefusal::Depth32)));
    // An adjustment layer: ADJUSTMENT and NO_PIXELS, Merged.
    let d = read_at("layers/hue-saturation.psd").unwrap();
    assert_eq!(d.forcing(true), PsdFeatures::ADJUSTMENT | PsdFeatures::NO_PIXELS);
    assert_eq!(mode(&d, true), Mode::Merged);
    // Gradient and pattern fill with MaxCompat off: NO_PIXELS, no real merged picture.
    for rel in ["layers-minimal/gradient-fill.psd", "layers-minimal/pattern-fill.psd"] {
        let d = read_at(rel).unwrap();
        assert_eq!(d.forcing(true), PsdFeatures::NO_PIXELS, "{rel}");
        assert!(!d.has_real_merged, "{rel}");
        assert_eq!(mode(&d, true), Mode::NeedsMerged, "{rel}");
    }
}
