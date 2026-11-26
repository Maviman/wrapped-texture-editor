//! Synthetic reader tests (stickers P5a; STICKER_PLAN 13.4 "Cases with no file of clear origin" and the reader
//! bullets of the P5 test list). Every file here is made by `testkit`.

use super::read::{RESYNCS, parse};
use super::testkit::*;
use super::*;

fn ok(bytes: &[u8]) -> PsdDoc {
    read(bytes, &PsdLimits::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn err(bytes: &[u8]) -> PsdError {
    read(bytes, &PsdLimits::default()).expect_err("the file should be refused")
}

/// A `w x h` straight RGBA test picture with soft alpha and some fully transparent pixels.
fn picture(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let a = ((x * 37 + y * 11 + seed * 5) % 256) as u8;
            let a = if (x + y + seed).is_multiple_of(7) { 0 } else { a.max(1) };
            out.extend_from_slice(&[(x * 13 + seed) as u8, (y * 29) as u8, (x * y + seed * 3) as u8, a]);
        }
    }
    out
}

#[test]
fn k5_synth_layer_compressions_8bit() {
    let rgba = picture(7, 5, 1);
    for compression in 0..=3u16 {
        let mut f = File::rgb(9, 6).version_info(true);
        f.layers.push(Layer::rgba_with("L", [1, 2, 6, 9], &rgba, compression, &[-1, 0, 1, 2]).lyid(7));
        let doc = ok(&f.build());
        let l = &doc.layers[0];
        assert_eq!(l.rect, IRect::new(2, 1, 9, 6), "compression {compression}");
        assert_eq!(l.pixels.as_ref().unwrap().data, rgba, "compression {compression}");
        assert_eq!(l.lyid, Some(7));
        assert_eq!(l.name, "L");
        assert!(l.features.is_empty());
        assert_eq!(mode(&doc, true), Mode::Layers);
    }
}

/// 16-bit samples of an RGBA picture: value * 257 plus a small offset that the reduction rounds away.
fn to16(rgba: &[u8]) -> Vec<u16> {
    rgba.iter().map(|&v| (v as u16) * 257 + (v as u16 % 3) * 20).collect()
}

fn plane16(samples: &[u16], k: usize) -> Vec<u8> {
    samples.iter().skip(k).step_by(4).flat_map(|v| v.to_be_bytes()).collect()
}

fn matte16(c: u16, a: u16) -> u16 {
    ((c as u64 * a as u64 + 65535 * (65535 - a as u64) + 32767) / 65535) as u16
}

#[test]
fn k5_synth_16bit_zip_channels_layers_and_merged() {
    let rgba = picture(6, 4, 2);
    let s16 = to16(&rgba);
    for compression in [0u16, 1, 2, 3] {
        let mut f = File::rgb(6, 4);
        f.depth = 16;
        let mut layer = Layer::rgba("L", [0, 0, 4, 6], &rgba);
        layer.channels = [(-1i16, 3usize), (0, 0), (1, 1), (2, 2)]
            .iter()
            .map(|&(id, k)| Chan { id, compression, samples: plane16(&s16, k), raw_override: None })
            .collect();
        f.layers.push(layer);
        f.in_lr16 = true;
        f.negative = true;
        f.channels = 4;
        let mut planes = vec![Vec::new(); 4];
        for p in s16.as_chunks::<4>().0.iter() {
            for k in 0..3 {
                planes[k].extend_from_slice(&matte16(p[k], p[3]).to_be_bytes());
            }
            planes[3].extend_from_slice(&p[3].to_be_bytes());
        }
        f.merged = planes;
        f.merged_compression = compression;
        let doc = ok(&f.build());
        assert_eq!(doc.depth, 16);
        assert!(!doc.is_flat(), "layers in Lr16 count (compression {compression})");
        let reduced: Vec<u8> = s16.iter().map(|&v| ((v as u32 + 128) / 257) as u8).collect();
        assert_eq!(doc.layers[0].pixels.as_ref().unwrap().data, reduced, "layer, compression {compression}");
        let truth = PixelBuf::from_raw(6, 4, reduced.clone()).unwrap();
        assert!(max_diff(doc.merged.as_ref().unwrap(), &truth, 16) <= 1, "merged, compression {compression}");
    }
}

#[test]
fn k5_synth_merged_zip_8bit_unmatted() {
    let rgba = picture(5, 5, 3);
    for compression in [2u16, 3] {
        let mut f = File::rgb(5, 5).merged_rgba(&rgba);
        f.merged_compression = compression;
        f.layers.push(Layer::rgba("L", [0, 0, 5, 5], &rgba));
        let bytes = f.build();
        let doc = ok(&bytes);
        let merged = doc.merged.as_ref().unwrap();
        let truth = PixelBuf::from_raw(5, 5, rgba.clone()).unwrap();
        // Matting on white and un-matting lose precision at low alpha only.
        assert!(max_diff(merged, &truth, 64) <= 2, "compression {compression}");
        for (m, t) in merged.data.as_chunks::<4>().0.iter().zip(truth.data.as_chunks::<4>().0.iter()) {
            assert_eq!(m[3], t[3]);
            if t[3] == 0 {
                assert_eq!(&m[..3], &[0, 0, 0], "a = 0 gives 0");
            }
        }
        // A truncated ZIP stream is never a picture.
        assert_eq!(parse(&bytes, &PsdLimits::default()).unwrap().end, bytes.len());
        for cut in 0..bytes.len() {
            assert!(read(&bytes[..cut], &PsdLimits::default()).is_err(), "prefix {cut} of {}", bytes.len());
        }
    }
}

#[test]
fn k5_synth_no_real_merged_still_imports_layers() {
    let rgba = picture(4, 4, 4);
    let mut f = File::rgb(4, 4).merged_rgba(&[255; 64]).version_info(false);
    f.layers.push(Layer::rgba("L", [0, 0, 4, 4], &rgba));
    let doc = ok(&f.build());
    assert!(!doc.has_real_merged);
    assert!(doc.merged.is_none());
    assert_eq!(doc.layers[0].pixels.as_ref().unwrap().data, rgba);
    assert_eq!(mode(&doc, true), Mode::Layers);
    // Resource missing = real; present and 1 = real.
    assert!(ok(&File::rgb(4, 4).version_info(true).build()).has_real_merged);
    assert!(ok(&File::rgb(4, 4).build()).has_real_merged);
    // A file with 0 layers is always real, whatever 1057 says (its picture is the document).
    let doc = ok(&File::rgb(4, 4).version_info(false).build());
    assert!(doc.is_flat() && doc.has_real_merged && doc.merged.is_some());
}

#[test]
fn k5_synth_psb_long_keys() {
    let keys: [&[u8; 4]; 14] = [
        b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid",
        b"PxSD", b"cinf",
    ];
    let rgba = picture(3, 3, 5);
    for psb in [false, true] {
        for key in keys {
            let mut f = File::rgb(3, 3);
            f.psb = psb;
            f.layers.push(Layer::rgba("L", [0, 0, 3, 3], &rgba).lyid(1));
            // `Lr16` / `Lr32` / `Layr` are read as a layer info only when the section has no layers.
            f.global_blocks.push((*b"8B64", *key, vec![0xAB; 6]));
            f.global_blocks.push((*b"8BIM", *b"Patt", vec![1, 2, 3, 4]));
            RESYNCS.with(|r| r.set(0));
            let name = std::str::from_utf8(key).unwrap();
            let doc = read(&f.build(), &PsdLimits::default()).unwrap_or_else(|e| panic!("psb {psb} key {name}: {e:?}"));
            assert_eq!(doc.psb, psb);
            assert_eq!(doc.layers.len(), 1);
            assert_eq!(doc.layers[0].pixels.as_ref().unwrap().data, rgba);
            assert_eq!(RESYNCS.with(|r| r.get()), 0, "psb {psb} key {name} is stepped over exactly");
        }
    }
    // PSB lengths in the layer section: 8-byte section, layer info and channel lengths, 4-byte RLE row counts.
    let pic = picture(5, 4, 6);
    let mut f = File::rgb(5, 4).merged_rgba(&pic);
    f.psb = true;
    f.layers.push(Layer::rgba("P", [0, 0, 4, 5], &pic));
    let doc = ok(&f.build());
    assert!(doc.psb);
    assert_eq!(doc.layers[0].pixels.as_ref().unwrap().data, pic);
    assert!(max_diff(doc.merged.as_ref().unwrap(), &composite(&doc), 64) <= 2);
}

#[test]
fn k5_synth_blending_ranges() {
    let rgba = picture(2, 2, 7);
    let gray: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[3]]).collect();
    // (channels in the record, colour mode, with a mask, with a second mask)
    let shapes: [(usize, u16, bool, bool); 5] =
        [(2, 1, false, false), (3, 1, true, false), (4, 3, false, false), (5, 3, true, false), (6, 3, true, true)];
    for (n, mode_number, with_mask, with_real) in shapes {
        let build = |ranges: Vec<u8>| {
            let mut f = File::rgb(2, 2);
            if mode_number == 1 {
                f.mode = 1;
                f.channels = 1;
                f.merged = vec![vec![255; 4]];
            }
            let order: &[i16] = if mode_number == 1 { &[-1, 0] } else { &[-1, 0, 1, 2] };
            let src = if mode_number == 1 { &gray } else { &rgba };
            let mut l = Layer::rgba_with("L", [0, 0, 2, 2], src, 1, order);
            if with_mask {
                l = l.mask([0, 0, 2, 2], &[255, 128, 0, 255], 0, 0);
            }
            if with_real {
                l.channels.push(Chan { id: -3, compression: 0, samples: vec![], raw_override: Some(vec![]) });
            }
            assert_eq!(l.channels.len(), n);
            l.ranges = ranges;
            f.layers.push(l);
            ok(&f.build())
        };
        // Photoshop's 40-byte form, PhotoCraft's (channels + 1) pairs, one pair, none: never BLEND_IF.
        for ranges in [DEFAULT_PAIR.repeat(5), DEFAULT_PAIR.repeat(n + 1), Vec::new(), DEFAULT_PAIR.to_vec()] {
            let doc = build(ranges.clone());
            assert!(!doc.layers[0].features.contains(PsdFeatures::BLEND_IF), "{n} channels, {} bytes", ranges.len());
        }
        // A changed pair, and a length that is not a multiple of 8.
        let mut changed = DEFAULT_PAIR.repeat(5);
        changed[17] = 0x80;
        assert!(build(changed).layers[0].features.contains(PsdFeatures::BLEND_IF), "{n} channels, changed pair");
        let mut odd = DEFAULT_PAIR.to_vec();
        odd.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
        assert!(build(odd).layers[0].features.contains(PsdFeatures::BLEND_IF), "{n} channels, 12 bytes");
    }
}

#[test]
fn k5_synth_16bit_unmatting_beats_unmatting_after_reduction() {
    // A soft-edged colour ramp at 16 bits: un-matting at 16 bits and then reducing is closer to the truth than
    // reducing the matted picture first and un-matting at 8 bits (STICKER_PLAN 5.2 "16-bit").
    let (w, h) = (64u32, 4u32);
    let mut truth16 = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let a = (x * 1024 + 300 + y * 7) as u16; // from very low to almost opaque
            truth16.extend_from_slice(&[40000u16 + (y as u16) * 101, 12345, (x as u16) * 900 + 7, a]);
        }
    }
    let mut f = File::rgb(w, h);
    f.depth = 16;
    f.channels = 4;
    f.negative = true;
    f.in_lr16 = true;
    f.layers.push(Layer::empty("E"));
    let mut planes = vec![Vec::new(); 4];
    let mut matted16 = Vec::new();
    for p in truth16.as_chunks::<4>().0.iter() {
        for k in 0..3 {
            let m = matte16(p[k], p[3]);
            planes[k].extend_from_slice(&m.to_be_bytes());
            matted16.push(m);
        }
        planes[3].extend_from_slice(&p[3].to_be_bytes());
        matted16.push(p[3]);
    }
    f.merged = planes;
    f.merged_compression = 0;
    let doc = ok(&f.build());
    let merged = doc.merged.as_ref().unwrap();
    let r8 = |v: u16| ((v as u32 + 128) / 257) as u8;
    let (mut ours, mut late, mut ours_max) = (0u64, 0u64, 0u8);
    for (i, p) in truth16.as_chunks::<4>().0.iter().enumerate() {
        let a8 = r8(p[3]);
        for k in 0..3 {
            let want = r8(p[k]);
            let got = merged.data[i * 4 + k];
            // Un-matting after the reduction (what the plan rules out).
            let m8 = r8(matted16[i * 4 + k]) as u32;
            let late_v = if a8 == 0 {
                0
            } else {
                ((m8.saturating_sub(255 - a8 as u32) * 255 + a8 as u32 / 2) / a8 as u32).min(255) as u8
            };
            ours += want.abs_diff(got) as u64;
            late += want.abs_diff(late_v) as u64;
            if a8 >= 16 {
                ours_max = ours_max.max(want.abs_diff(got));
            }
        }
    }
    assert!(ours * 4 < late, "16-bit un-matting error {ours} should be far below {late}");
    assert!(ours_max <= 1, "largest error at alpha >= 16 is {ours_max}");
}

#[test]
fn k5_synth_grayscale_is_expanded() {
    let mut f = File::rgb(3, 2);
    f.mode = 1;
    f.channels = 2;
    f.negative = true;
    f.merged = vec![vec![10, 20, 30, 40, 50, 255], vec![255; 6]];
    let gray_rgba: Vec<u8> = [10u8, 20, 30, 40, 50, 60].iter().flat_map(|&g| [g, g, g, 200]).collect();
    f.layers.push(Layer::rgba_with("G", [0, 0, 2, 3], &gray_rgba, 1, &[-1, 0]));
    let doc = ok(&f.build());
    assert_eq!(doc.mode, PsdColorMode::Grayscale);
    assert_eq!(doc.layers[0].pixels.as_ref().unwrap().data, gray_rgba);
    let m = doc.merged.as_ref().unwrap();
    assert_eq!(m.get(1, 0), [20, 20, 20, 255]);
    assert_eq!(m.get(2, 1), [255, 255, 255, 255]);
}

#[test]
fn k5_synth_refusals_have_their_sentences() {
    let header = |mode: u16, depth: u16| {
        let mut f = File::rgb(2, 2);
        f.mode = mode;
        f.depth = depth;
        f.build()
    };
    let cases: [(u16, u16, PsdRefusal, &str); 9] = [
        (4, 8, PsdRefusal::Cmyk, "is in CMYK Color mode. In Photoshop choose Image > Mode > RGB Color and save again."),
        (9, 8, PsdRefusal::Lab, "is in Lab Color mode."),
        (2, 8, PsdRefusal::Indexed, "is in Indexed Color mode."),
        (8, 8, PsdRefusal::Duotone, "is in Duotone mode."),
        (7, 16, PsdRefusal::Multichannel, "is in Multichannel mode."),
        (0, 1, PsdRefusal::Bitmap, "is in Bitmap mode."),
        (3, 32, PsdRefusal::Depth32, "has 32 bits per channel. In Photoshop choose Image > Mode > 8 Bits/Channel and save again."),
        (1, 32, PsdRefusal::Depth32, "has 32 bits per channel."),
        (3, 4, PsdRefusal::OtherDepth(4), "has 4 bits per channel."),
    ];
    for (mode_number, depth, refusal, words) in cases {
        assert_eq!(err(&header(mode_number, depth)), PsdError::Unsupported(refusal), "mode {mode_number} depth {depth}");
        let sentence = refusal.sentence("cat.psd");
        assert!(sentence.starts_with("cat.psd ") && sentence.contains(words), "{sentence}");
    }
    assert_eq!(err(&header(5, 8)), PsdError::Unsupported(PsdRefusal::OtherMode(5)));
    // An unknown channel compression.
    let mut f = File::rgb(2, 2);
    let mut l = Layer::rgba("L", [0, 0, 2, 2], &picture(2, 2, 1));
    l.channels[1].compression = 4;
    l.channels[1].raw_override = Some(vec![0; 4]);
    f.layers.push(l);
    assert_eq!(err(&f.build()), PsdError::Unsupported(PsdRefusal::Compression(4)));
    assert!(PsdRefusal::Compression(4).sentence("x.psd").contains("compression"));
    // Not a PSD, an unknown version, too large.
    assert_eq!(err(b"GIF89a\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"), PsdError::Corrupt("not a PSD file"));
    let mut b = File::rgb(2, 2).build();
    b[5] = 3;
    assert_eq!(err(&b), PsdError::Corrupt("unknown PSD version"));
    let mut f = File::rgb(30_001, 1);
    f.merged = vec![vec![]; 3];
    assert_eq!(err(&f.build()), PsdError::TooLarge { width: 30_001, height: 1 });
}

#[test]
fn k5_synth_third_party_shapes() {
    // What cactus_top.psd and 2layers.psd showed: channel -1 last, `lsct` type 0 on a plain layer, no `lyid`, no
    // resource 1057, Pascal names in UTF-8 and in Windows-1252 without `luni`.
    let rgba = picture(4, 3, 8);
    let mut f = File::rgb(4, 3).merged_rgba(&rgba);
    let mut a = Layer::rgba_with("x", [0, 0, 3, 4], &rgba, 1, &[0, 1, 2, -1]);
    a.name = vec![0xD0, 0xA4, 0xD0, 0xBE, 0xD0, 0xBD];
    a = a.block(b"lsct", 0u32.to_be_bytes().to_vec());
    let mut b = Layer::rgba_with("y", [0, 0, 3, 4], &rgba, 0, &[0, 1, 2, -1]);
    b.name = vec![b'C', b'a', b'f', 0xE9, b' ', 0x80, b' ', 0x9F];
    b.flags = 0;
    f.layers = vec![a, b];
    let doc = ok(&f.build());
    assert!(doc.has_real_merged, "no 1057 = real");
    let (a, b) = (&doc.layers[0], &doc.layers[1]);
    assert_eq!(a.name, "\u{0424}\u{043E}\u{043D}");
    assert_eq!(b.name, "Caf\u{e9} \u{20AC} \u{178}");
    assert_eq!(a.section, PsdSection::Layer);
    assert!(a.features.is_empty() && b.features.is_empty(), "lsct type 0 is no group");
    assert_eq!(a.lyid, None);
    assert_eq!(a.pixels.as_ref().unwrap().data, rgba, "channel -1 last");
    assert_eq!(b.pixels.as_ref().unwrap().data, rgba);
    assert_eq!(mode(&doc, true), Mode::Layers);
    // `luni` wins over the Pascal name, also with characters outside the BMP.
    let mut f = File::rgb(1, 1);
    let mut luni = 4u32.to_be_bytes().to_vec();
    for u in "a\u{1F47D}b".encode_utf16() {
        luni.extend_from_slice(&u.to_be_bytes());
    }
    f.layers.push(Layer::rgba("ascii", [0, 0, 1, 1], &[1, 2, 3, 4]).block(b"luni", luni));
    assert_eq!(ok(&f.build()).layers[0].name, "a\u{1F47D}b");
}

#[test]
fn k5_synth_flat_16bit_vs_layers_in_lr16() {
    let mut flat = File::rgb(2, 2);
    flat.depth = 16;
    flat.merged = vec![vec![0x12, 0x34, 0, 0, 0xFF, 0xFF, 0x80, 0x00]; 3];
    flat.merged_compression = 0;
    let doc = ok(&flat.build());
    assert!(doc.is_flat() && doc.has_real_merged);
    assert_eq!(doc.merged.as_ref().unwrap().get(0, 0), [0x12, 0x12, 0x12, 255]);
    assert_eq!(doc.merged.as_ref().unwrap().get(1, 1), [0x80, 0x80, 0x80, 255]);
    let mut with = flat.clone().version_info(false);
    with.in_lr16 = true;
    with.layers.push(Layer::empty("E"));
    let doc = ok(&with.build());
    assert!(!doc.is_flat(), "the layers of Lr16 count");
    assert!(!doc.has_real_merged);
    assert_eq!(doc.layers[0].pixels, Some(CroppedBuf::empty()));
}

#[test]
fn k5_synth_effects_rules() {
    let fx_on = Desc::new(b"null", 3)
        .item(b"Scl ", b"UntF", &[b'#', b'P', b'r', b'c', 0x40, 0x59, 0, 0, 0, 0, 0, 0])
        .bool(b"masterFXSwitch", true)
        .item(b"DrSh", b"Objc", &Desc::new(b"DrSh", 2).bool(b"enab", true).bool(b"present", true).0)
        .effects_block();
    let fx_master_off = Desc::new(b"null", 2)
        .bool(b"masterFXSwitch", false)
        .item(b"DrSh", b"Objc", &Desc::new(b"DrSh", 1).bool(b"enab", true).0)
        .effects_block();
    let fx_all_off = Desc::new(b"null", 2)
        .bool(b"masterFXSwitch", true)
        .item(b"OrGl", b"Objc", &Desc::new(b"OrGl", 1).bool(b"enab", false).0)
        .effects_block();
    let mut list = 1u32.to_be_bytes().to_vec();
    list.extend_from_slice(b"Objc");
    list.extend_from_slice(&Desc::new(b"DrSh", 1).bool(b"enab", true).0);
    let lmfx_list =
        Desc::new(b"null", 2).bool(b"masterFXSwitch", true).item(b"dropShadowMulti", b"VlLs", &list).effects_block();
    let mut unreadable = vec![0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0];
    unreadable.extend_from_slice(b"null\0\0\0\x01\0\0\0\0XYZW????");
    type Case<'a> = (&'a str, Vec<([u8; 4], Vec<u8>)>, bool);
    let cases: Vec<Case> = vec![
        ("lrFX alone", vec![(*b"lrFX", vec![0, 0, 0, 0])], true),
        ("lfx2 on", vec![(*b"lfx2", fx_on), (*b"lrFX", vec![0; 4])], true),
        ("lfx2 master off + lrFX", vec![(*b"lfx2", fx_master_off), (*b"lrFX", vec![0; 4])], false),
        ("lfx2 every effect off", vec![(*b"lfx2", fx_all_off), (*b"lrFX", vec![0; 4])], false),
        ("lmfx with a list", vec![(*b"lmfx", lmfx_list)], true),
        ("unreadable lfx2", vec![(*b"lfx2", unreadable)], true),
        ("none", vec![], false),
    ];
    for (what, blocks, on) in cases {
        let mut f = File::rgb(2, 2);
        let mut l = Layer::rgba("L", [0, 0, 2, 2], &picture(2, 2, 2));
        for (k, d) in blocks {
            l = l.block(&k, d);
        }
        f.layers.push(l);
        let doc = ok(&f.build());
        assert_eq!(doc.layers[0].features.contains(PsdFeatures::EFFECTS), on, "{what}");
        assert_eq!(doc.forcing(false).contains(PsdFeatures::EFFECTS), on, "{what}");
    }
}

/// A descriptor holding one item of type `ty` with `value`, then `masterFXSwitch` false: the walker reads `false`
/// (effects off) only when it stepped over the first item exactly.
fn after(ty: &[u8; 4], value: &[u8]) -> bool {
    let block = Desc::new(b"null", 2).item(b"Itm ", ty, value).bool(b"masterFXSwitch", false).effects_block();
    descriptor::effects_enabled(&block)
}

fn class(k: &[u8]) -> Vec<u8> {
    let mut d = Vec::new();
    key(&mut d, k);
    d
}

#[test]
fn k5_desc_walker_steps_over_every_item_type() {
    let double = 1.5f64.to_be_bytes().to_vec();
    let mut unfl = b"#Pxl".to_vec();
    unfl.extend_from_slice(&2u32.to_be_bytes());
    unfl.extend_from_slice(&[0; 16]);
    let en = [class(b"Type"), class(b"a longer enum value")].concat();
    let ty = [unicode("name"), class(b"Clss")].concat();
    let raw = |n: u32| {
        let mut d = n.to_be_bytes().to_vec();
        d.extend(std::iter::repeat_n(7u8, n as usize));
        d
    };
    let mut vlls = 2u32.to_be_bytes().to_vec();
    vlls.extend_from_slice(b"long");
    vlls.extend_from_slice(&5u32.to_be_bytes());
    vlls.extend_from_slice(b"TEXT");
    vlls.extend_from_slice(&unicode("hi"));
    let mut obar = 16u32.to_be_bytes().to_vec();
    obar.extend_from_slice(&unicode(""));
    obar.extend_from_slice(&class(b"rationalPoint"));
    obar.extend_from_slice(&1u32.to_be_bytes());
    obar.extend_from_slice(&class(b"Hrzn"));
    obar.extend_from_slice(b"UnFl");
    obar.extend_from_slice(&unfl);
    let items: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"doub", double.clone()),
        (b"UntF", [b"#Pxl".to_vec(), double.clone()].concat()),
        (b"UnFl", unfl.clone()),
        (b"TEXT", unicode("text \u{1F47D}")),
        (b"enum", en),
        (b"long", vec![0, 0, 0, 9]),
        (b"comp", vec![0; 8]),
        (b"bool", vec![1]),
        (b"type", ty.clone()),
        (b"GlbC", ty),
        (b"alis", raw(5)),
        (b"tdta", raw(3)),
        (b"Pth ", raw(9)),
        (b"VlLs", vlls),
        (b"ObAr", obar),
        (b"Objc", Desc::new(b"Clr ", 1).item(b"Rd  ", b"doub", &double).0),
        (b"GlbO", Desc::new(b"Clr ", 0).0),
    ];
    for (ty, value) in &items {
        assert!(!after(ty, value), "item type {:?}", std::str::from_utf8(*ty));
    }
    // Every reference item kind inside `obj `, alone and all seven in one reference.
    let refs: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"prop", [unicode("p"), class(b"Lyr "), class(b"Ordn")].concat()),
        (b"Clss", [unicode("c"), class(b"Lyr ")].concat()),
        (b"Enmr", [unicode("e"), class(b"Lyr "), class(b"Ordn"), class(b"Trgt")].concat()),
        (b"rele", [unicode("r"), class(b"Lyr "), vec![0, 0, 0, 1]].concat()),
        (b"Idnt", vec![0, 0, 0, 4]),
        (b"indx", vec![0, 0, 0, 2]),
        (b"name", [unicode("n"), class(b"Lyr "), unicode("Layer 1")].concat()),
    ];
    let mut all = (refs.len() as u32).to_be_bytes().to_vec();
    for (kind, body) in &refs {
        let mut r = 1u32.to_be_bytes().to_vec();
        r.extend_from_slice(*kind);
        r.extend_from_slice(body);
        assert!(!after(b"obj ", &r), "reference item {:?}", std::str::from_utf8(*kind));
        all.extend_from_slice(*kind);
        all.extend_from_slice(body);
    }
    assert!(!after(b"obj ", &all));
    // Unknown item types, unknown reference kinds, cut data and deep nesting are unreadable = on.
    assert!(after(b"XXXX", &[0; 8]));
    let mut bad = 1u32.to_be_bytes().to_vec();
    bad.extend_from_slice(b"Zzzz");
    assert!(after(b"obj ", &bad));
    assert!(after(b"doub", &[0; 3]));
    let mut deep = Desc::new(b"leaf", 0).0;
    for _ in 0..40 {
        deep = Desc::new(b"node", 1).item(b"in  ", b"Objc", &deep).0;
    }
    assert!(after(b"Objc", &deep));
    assert!(descriptor::effects_enabled(&[0, 0, 0, 0]), "too short");
    assert!(descriptor::effects_enabled(&[0, 0, 0, 0, 0, 0, 0, 15]), "unknown descriptor version");
}

#[test]
fn k5_synth_empty_layers_and_no_pixels() {
    let rgba = picture(2, 2, 3);
    let mut f = File::rgb(2, 2);
    // Layer > New before painting: empty rect, flags 0x08, no key -> an empty picture layer.
    f.layers.push(Layer::empty("new"));
    let mut l = Layer::empty("bits");
    l.flags = 0x18;
    f.layers.push(l);
    f.layers.push(Layer::empty("fill").block(b"SoCo", vec![0; 8]));
    let mut l = Layer::empty("levels").block(b"levl", vec![0; 8]);
    l.flags = 0x18;
    f.layers.push(l);
    // Group markers never get NO_PIXELS.
    let mut end = Layer::empty("</Layer group>").block(b"lsct", 3u32.to_be_bytes().to_vec());
    end.flags = 0x18;
    f.layers.push(end);
    let mut open = Layer::empty("grp").block(b"lsct", [1u32.to_be_bytes(), *b"8BIM", *b"pass"].concat());
    open.flags = 0x18;
    f.layers.push(open);
    // An inverted rect (Photoshop CC 2019 writes (0, 0, -1, 0)) and right < left.
    let mut inv = Layer::empty("inverted");
    inv.rect = [0, 0, -1, 0];
    f.layers.push(inv);
    let mut inv = Layer::rgba("inverted2", [0, 5, 2, 3], &[]);
    inv.channels.iter_mut().for_each(|c| c.compression = 0);
    f.layers.push(inv);
    f.layers.push(Layer::rgba("pixels", [0, 0, 2, 2], &rgba));
    let doc = ok(&f.build());
    let get = |n: &str| layer(&doc, n);
    assert_eq!(get("new").pixels, Some(CroppedBuf::empty()));
    assert!(get("new").features.is_empty());
    assert_eq!(get("bits").features, PsdFeatures::NO_PIXELS);
    assert!(get("bits").pixels.is_none());
    assert_eq!(get("fill").features, PsdFeatures::NO_PIXELS | PsdFeatures::FILL);
    assert_eq!(get("levels").features, PsdFeatures::NO_PIXELS | PsdFeatures::ADJUSTMENT);
    assert_eq!(get("</Layer group>").features, PsdFeatures::GROUP);
    assert_eq!(get("</Layer group>").section, PsdSection::GroupEnd);
    assert_eq!(get("grp").features, PsdFeatures::GROUP | PsdFeatures::BLEND_OTHER);
    assert_eq!(get("grp").blend, PsdBlend::Other(*b"pass"));
    assert_eq!(get("grp").section, PsdSection::GroupOpen);
    assert_eq!(get("inverted").rect, IRect::EMPTY);
    assert_eq!(get("inverted").pixels, Some(CroppedBuf::empty()));
    assert_eq!(get("inverted2").rect, IRect::EMPTY);
    assert_eq!(get("pixels").pixels.as_ref().unwrap().data, rgba);
    // An ordinary empty layer next to a picture layer: Layers mode.
    let mut only = File::rgb(2, 2);
    only.layers = vec![Layer::empty("a"), Layer::rgba("b", [0, 0, 2, 2], &rgba)];
    assert_eq!(mode(&ok(&only.build()), true), Mode::Layers);
}

#[test]
fn k5_synth_shape_rule_masks_and_mask_features() {
    let rgba = picture(3, 3, 9);
    let rendered = [9u8; 9];
    let build = |flags: u8, keys: &[&[u8; 4]], mask_flags: Option<u8>, third: bool| {
        let mut f = File::rgb(3, 3);
        let mut l = Layer::rgba("L", [0, 0, 3, 3], &rgba);
        l.flags = flags;
        for k in keys {
            l = l.block(k, vec![0; 8]);
        }
        if let Some(mf) = mask_flags {
            l = l.mask([0, 0, 3, 3], &rendered, 0, mf);
        }
        if third {
            l.channels.push(Chan { id: -3, compression: 0, samples: vec![], raw_override: Some(vec![]) });
        }
        f.layers.push(l);
        ok(&f.build())
    };
    // A shape layer with pixels: its vector mask and a rendered (bit 3) mask are already in its pixels.
    let doc = build(0x18, &[b"SoCo", b"vmsk"], Some(0x08), false);
    assert_eq!(doc.layers[0].features, PsdFeatures::FILL);
    assert!(doc.layers[0].mask.is_none());
    assert_eq!(mode(&doc, true), Mode::Layers);
    // The same with a user mask (not rendered) keeps the mask.
    let doc = build(0x18, &[b"vscg", b"vsms"], Some(0), false);
    let m = doc.layers[0].mask.as_ref().unwrap();
    assert_eq!((m.rect, m.default, m.disabled, m.rendered), (IRect::new(0, 0, 3, 3), 0, false, false));
    assert_eq!(m.pixels.data, rendered);
    // A vector mask on an ordinary pixel layer forces.
    let doc = build(0x08, &[b"vmsk"], Some(0x08), false);
    assert_eq!(doc.layers[0].features, PsdFeatures::VECTOR_MASK);
    assert!(doc.layers[0].mask.as_ref().unwrap().rendered);
    // A disabled mask; mask parameters (flags bit 4); a second mask (channel -3).
    let doc = build(0x08, &[], Some(0x02), false);
    assert!(doc.layers[0].mask.as_ref().unwrap().disabled);
    assert!(doc.layers[0].features.is_empty());
    assert_eq!(build(0x08, &[], Some(0x10), false).layers[0].features, PsdFeatures::MASK_EXTRA);
    assert_eq!(build(0x08, &[], Some(0), true).layers[0].features, PsdFeatures::MASK_EXTRA);
    // Fill opacity, another blend key, text and smart objects.
    let mut f = File::rgb(3, 3);
    f.layers.push(Layer::rgba("iopa", [0, 0, 3, 3], &rgba).block(b"iOpa", vec![254, 0, 0, 0]));
    let mut l = Layer::rgba("div", [0, 0, 3, 3], &rgba);
    l.blend = *b"div ";
    f.layers.push(l);
    f.layers.push(Layer::rgba("text", [0, 0, 3, 3], &rgba).block(b"TySh", vec![0; 8]));
    f.layers.push(Layer::rgba("smart", [0, 0, 3, 3], &rgba).block(b"SoLd", vec![0; 8]));
    let doc = ok(&f.build());
    assert_eq!(layer(&doc, "iopa").features, PsdFeatures::FILL_OPACITY);
    assert_eq!(layer(&doc, "iopa").fill_opacity, 254);
    assert_eq!(layer(&doc, "div").features, PsdFeatures::BLEND_OTHER);
    assert_eq!(layer(&doc, "text").features, PsdFeatures::TEXT);
    assert_eq!(layer(&doc, "smart").features, PsdFeatures::SMART);
    assert!(doc.editor_native());
    assert_eq!(doc.forcing(false), PsdFeatures::FILL_OPACITY | PsdFeatures::BLEND_OTHER);
    assert_eq!(doc.forcing(true), PsdFeatures::FILL_OPACITY | PsdFeatures::BLEND_OTHER | PsdFeatures::SMART);
    // Hidden layers do not force; a group does even when its marker is hidden.
    let mut f = File::rgb(3, 3);
    let mut l = Layer::rgba("hidden fx", [0, 0, 3, 3], &rgba).block(b"lrFX", vec![0; 4]);
    l.flags |= 0x02;
    f.layers.push(l);
    let doc = ok(&f.build());
    assert!(!doc.layers[0].visible);
    assert!(doc.forcing(true).is_empty());
    let mut f = File::rgb(3, 3);
    let mut l = Layer::empty("hidden group").block(b"lsct", 1u32.to_be_bytes().to_vec());
    l.flags |= 0x02;
    f.layers.push(l);
    assert_eq!(ok(&f.build()).forcing(true), PsdFeatures::GROUP);
    // Artboards.
    let mut f = File::rgb(3, 3);
    f.layers.push(Layer::empty("board").block(b"artb", vec![0; 8]));
    assert!(ok(&f.build()).forcing(false).contains(PsdFeatures::GROUP));
    // The words of the sentence.
    assert_eq!((PsdFeatures::SMART | PsdFeatures::GROUP).words(), vec!["groups", "smart objects"]);
}

#[test]
fn k5_synth_block_padding_and_resync() {
    let rgba = picture(2, 2, 4);
    for pad in [vec![], vec![0], vec![0, 0], vec![0, 0, 0], b"junk".to_vec(), b"junk junk!".to_vec()] {
        let mut f = File::rgb(2, 2);
        let mut l = Layer::rgba("L", [0, 0, 2, 2], &rgba);
        l.blocks.push((*b"sn2P", vec![0, 0, 0, 1], pad.clone()));
        l.blocks.push((*b"lyid", 42u32.to_be_bytes().to_vec(), pad.clone()));
        l.blocks.push((*b"TySh", vec![1, 2], Vec::new()));
        f.layers.push(l);
        f.global_blocks.push((*b"8BIM", *b"Patt", vec![9; 3]));
        RESYNCS.with(|r| r.set(0));
        let doc = ok(&f.build());
        assert_eq!(doc.layers[0].lyid, Some(42), "padding {pad:?}");
        assert_eq!(doc.layers[0].features, PsdFeatures::TEXT, "padding {pad:?}");
        let junk = pad.iter().any(|&b| b != 0);
        assert_eq!(RESYNCS.with(|r| r.get()) > 0, junk, "padding {pad:?}");
    }
    // A name padded to 2 instead of 4.
    let mut f = File::rgb(2, 2);
    let mut l = Layer::rgba("Name", [0, 0, 2, 2], &rgba).lyid(5);
    l.name_pad = Some(1);
    f.layers.push(l);
    assert_eq!(ok(&f.build()).layers[0].lyid, Some(5));
}

fn icc_profile(tag: Vec<u8>) -> Vec<u8> {
    let mut p = vec![0u8; 128];
    p.extend_from_slice(&1u32.to_be_bytes());
    p.extend_from_slice(b"desc");
    p.extend_from_slice(&144u32.to_be_bytes());
    p.extend_from_slice(&(tag.len() as u32).to_be_bytes());
    p.extend_from_slice(&tag);
    p
}

fn icc_v2(desc: &str) -> Vec<u8> {
    let mut tag = b"desc\0\0\0\0".to_vec();
    tag.extend_from_slice(&((desc.len() + 1) as u32).to_be_bytes());
    tag.extend_from_slice(desc.as_bytes());
    tag.push(0);
    icc_profile(tag)
}

fn icc_v4(desc: &str) -> Vec<u8> {
    let units: Vec<u8> = desc.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    let mut tag = b"mluc\0\0\0\0".to_vec();
    tag.extend_from_slice(&1u32.to_be_bytes());
    tag.extend_from_slice(&12u32.to_be_bytes());
    tag.extend_from_slice(b"enUS");
    tag.extend_from_slice(&(units.len() as u32).to_be_bytes());
    tag.extend_from_slice(&28u32.to_be_bytes());
    tag.extend_from_slice(&units);
    icc_profile(tag)
}

#[test]
fn k5_synth_icc_description_and_note() {
    let with = |p: Vec<u8>, gray: bool| {
        let mut f = File::rgb(1, 1);
        if gray {
            f.mode = 1;
            f.channels = 1;
            f.merged = vec![vec![0]];
        }
        f.resources.push((1039, p));
        ok(&f.build())
    };
    let doc = with(icc_v2("sRGB IEC61966-2.1"), false);
    assert_eq!(doc.icc_description.as_deref(), Some("sRGB IEC61966-2.1"));
    assert_eq!(doc.profile_note(), None);
    assert_eq!(with(icc_v2("Adobe RGB (1998)"), false).profile_note(), Some("Adobe RGB (1998)"));
    assert_eq!(with(icc_v4("Display P3 \u{e9}"), false).profile_note(), Some("Display P3 \u{e9}"));
    let doc = with(icc_v2("Dot Gain 15%"), true);
    assert_eq!(doc.icc_description.as_deref(), Some("Dot Gain 15%"));
    assert_eq!(doc.profile_note(), None, "grayscale files are exempt");
    assert_eq!(with(vec![1, 2, 3], false).icc_description, None, "a damaged profile has no description");
}

#[test]
fn k5_synth_limits() {
    let rgba = picture(4, 4, 1);
    let mut f = File::rgb(4, 4).merged_rgba(&rgba);
    f.layers.push(Layer::rgba("L", [0, 0, 4, 4], &rgba).mask([0, 0, 4, 4], &[200; 16], 0, 0));
    let bytes = f.build();
    let small = PsdLimits { max_side: 30_000, max_decode_side: 3, max_decode_pixels: 1 << 20 };
    let doc = read(&bytes, &small).unwrap();
    assert!(doc.too_large && doc.merged.is_none() && doc.layers[0].pixels.is_none() && doc.layers[0].mask.is_none());
    let few = PsdLimits { max_side: 30_000, max_decode_side: 8192, max_decode_pixels: 15 };
    assert!(read(&bytes, &few).unwrap().too_large);
    assert!(!ok(&bytes).too_large);
    // A truncated file over the limits is still an error.
    assert!(read(&bytes[..bytes.len() - 1], &small).is_err());
    let tiny = PsdLimits { max_side: 3, ..PsdLimits::default() };
    assert_eq!(read(&bytes, &tiny), Err(PsdError::TooLarge { width: 4, height: 4 }));
    // A layer far larger than the canvas but within the pixel limit is read; one above it is skipped.
    let mut f = File::rgb(2, 2);
    f.layers.push(Layer::rgba("big", [-3, -3, 5, 5], &picture(8, 8, 2)));
    assert_eq!(ok(&f.build()).layers[0].pixels.as_ref().unwrap().rect, IRect::new(-3, -3, 5, 5));
    let doc = read(&f.build(), &PsdLimits { max_decode_pixels: 32, ..PsdLimits::default() }).unwrap();
    assert!(doc.too_large && doc.layers[0].pixels.is_none());
}

#[test]
fn k5_synth_merged_transparency_rules() {
    let rgba = picture(3, 2, 2);
    let alphas = |m: &PixelBuf| m.data.as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>();
    // A flat file with 4 channels: the 4th is transparency.
    let mut f = File::rgb(3, 2).merged_rgba(&rgba);
    f.negative = false;
    let doc = ok(&f.build());
    assert!(doc.is_flat());
    assert_eq!(alphas(doc.merged.as_ref().unwrap()), rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>());
    // A positive layer count with 4 channels: the 4th is a saved alpha, the picture is opaque.
    let mut f = File::rgb(3, 2).merged_rgba(&rgba);
    f.negative = false;
    f.layers.push(Layer::rgba("L", [0, 0, 2, 3], &rgba));
    assert!(alphas(ok(&f.build()).merged.as_ref().unwrap()).iter().all(|&a| a == 255));
    // Negative count, 5 channels (transparency, then a saved alpha): the 4th is transparency (fixture f9's order).
    let mut f = File::rgb(3, 2).merged_rgba(&rgba);
    f.merged.push(vec![77; 6]);
    f.channels = 5;
    f.layers.push(Layer::rgba("L", [0, 0, 2, 3], &rgba));
    assert_eq!(ok(&f.build()).merged.as_ref().unwrap().get(0, 0)[3], rgba[3]);
}

#[test]
fn k5_synth_damaged_channels_fail_cleanly() {
    let rgba = picture(4, 4, 6);
    let mut cases = Vec::new();
    // A PackBits row count below what any row needs.
    let mut l = Layer::rgba("L", [0, 0, 4, 4], &rgba);
    l.channels[0].raw_override = Some(vec![0, 1, 0, 1, 0, 1, 0, 1, 0xFD, 9, 0xFD, 9, 0xFD, 9, 0xFD]);
    cases.push(l);
    // Rows longer than the channel.
    let mut l = Layer::rgba("L", [0, 0, 4, 4], &rgba);
    l.channels[0].raw_override = Some(vec![0, 50, 0, 2, 0, 2, 0, 2, 0xFD, 9]);
    cases.push(l);
    // A large rect with almost no data: refused before any allocation.
    let mut l = Layer::rgba("L", [0, 0, 4000, 4000], &[]);
    for c in &mut l.channels {
        c.raw_override = Some(vec![0, 2]);
    }
    cases.push(l);
    // A ZIP channel whose output cannot come from its input.
    let mut l = Layer::rgba("L", [0, 0, 4000, 4000], &[]);
    for c in &mut l.channels {
        c.compression = 2;
        c.raw_override = Some(vec![0x78, 0x9C, 3, 0]);
    }
    cases.push(l);
    // Raw data too short.
    let mut l = Layer::rgba("L", [0, 0, 4, 4], &rgba);
    l.channels[2].compression = 0;
    l.channels[2].raw_override = Some(vec![1, 2, 3]);
    cases.push(l);
    for (i, l) in cases.into_iter().enumerate() {
        let mut f = File::rgb(4, 4);
        f.layers.push(l);
        assert!(matches!(read(&f.build(), &PsdLimits::default()), Err(PsdError::Corrupt(_))), "case {i}");
    }
}

#[test]
fn k5_synth_layer_order_clipping_opacity_blend() {
    let a = picture(4, 4, 1);
    let b = picture(4, 4, 2);
    let mut f = File::rgb(4, 4);
    f.layers.push(Layer::rgba("bottom", [0, 0, 4, 4], &a).lyid(1));
    let mut top = Layer::rgba("top", [0, 0, 4, 4], &b).lyid(2);
    top.clipping = 1;
    top.opacity = 77;
    top.blend = *b"scrn";
    f.layers.push(top);
    let doc = ok(&f.build());
    assert_eq!(doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["bottom", "top"]);
    let t = &doc.layers[1];
    assert!(t.clipped && t.visible);
    assert_eq!((t.opacity, t.blend), (77, PsdBlend::Known(crate::layer::BlendMode::Screen)));
    assert_eq!(mode(&doc, true), Mode::Layers);
    for (key, m) in [
        (b"norm", crate::layer::BlendMode::Normal),
        (b"mul ", crate::layer::BlendMode::Multiply),
        (b"scrn", crate::layer::BlendMode::Screen),
        (b"over", crate::layer::BlendMode::Overlay),
        (b"lddg", crate::layer::BlendMode::Add),
        (b"dark", crate::layer::BlendMode::Darken),
        (b"lite", crate::layer::BlendMode::Lighten),
    ] {
        assert_eq!(PsdBlend::from_key(*key), PsdBlend::Known(m));
        assert_eq!(m.psd_key(), *key);
    }
}

/// P5a checker: the layer-count plausibility check used 46 bytes per record, but a record without channels, with empty
/// blending ranges and an empty name padded to 2 (deviation 6 of the build note: names padded to 2 read the same) has
/// only 44, and the smallest legal record 43. Such a file was refused as damaged.
#[test]
fn k5_check_layer_count_floor_is_the_smallest_record() {
    for pad in [0usize, 1] {
        let mut f = File::rgb(2, 2);
        for _ in 0..4 {
            let mut l = Layer::empty("");
            l.channels.clear();
            l.ranges.clear();
            l.name_pad = Some(pad);
            f.layers.push(l);
        }
        let doc = ok(&f.build());
        assert_eq!(doc.layers.len(), 4, "pad {pad}");
        assert!(doc.layers.iter().all(|l| l.pixels == Some(CroppedBuf::empty()) && l.features.is_empty()), "pad {pad}");
    }
}
