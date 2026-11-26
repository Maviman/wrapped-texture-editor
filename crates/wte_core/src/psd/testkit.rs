//! Test-only PSD / PSB byte builder and helpers (STICKER_PLAN 13.9 P5a `psd/testkit.rs`).
//!
//! The builder writes exactly what it is told, including things a good writer never does (channel -1 last, odd
//! padding, ZIP channels, PSB long lengths, refused colour modes), so the reader's rules can be tested on bytes that
//! no file of clear origin shows. It is not a PSD writer for the app (that is `psd::write_sticker`, P5b).

use miniz_oxide::deflate::compress_to_vec_zlib;

use crate::blend::blend_pixel;
use crate::export::packbits::encode_row;
use crate::layer::BlendMode;
use crate::pixel::PixelBuf;

use super::{PsdBlend, PsdDoc, PsdLayer};

/// Photoshop's default blending-range pair.
pub const DEFAULT_PAIR: [u8; 8] = [0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF];

/// One channel: its id, compression and the raw samples (rows of `w * bps` bytes, big-endian for 16 bits).
#[derive(Clone, Debug)]
pub struct Chan {
    pub id: i16,
    pub compression: u16,
    pub samples: Vec<u8>,
    /// Bytes written instead of the encoded samples (after the compression word), for damaged files.
    pub raw_override: Option<Vec<u8>>,
}

/// One layer record.
#[derive(Clone, Debug)]
pub struct Layer {
    /// top, left, bottom, right
    pub rect: [i32; 4],
    pub channels: Vec<Chan>,
    pub blend: [u8; 4],
    pub opacity: u8,
    pub clipping: u8,
    pub flags: u8,
    /// The layer mask data bytes (after the length).
    pub mask_data: Vec<u8>,
    /// The blending ranges bytes (after the length).
    pub ranges: Vec<u8>,
    /// The Pascal name bytes.
    pub name: Vec<u8>,
    /// Padding after the Pascal name: `None` = to a multiple of 4.
    pub name_pad: Option<usize>,
    /// Tagged blocks (signature `8BIM`), each padded to even; `(key, data, extra padding bytes)`.
    pub blocks: Vec<([u8; 4], Vec<u8>, Vec<u8>)>,
}

impl Layer {
    /// A plain layer: channels -1, 0, 1, 2 from straight RGBA over `rect` (`[top, left, bottom, right]`), RLE.
    pub fn rgba(name: &str, rect: [i32; 4], rgba: &[u8]) -> Layer {
        Layer::rgba_with(name, rect, rgba, 1, &[-1, 0, 1, 2])
    }

    /// A layer with the given compression and channel order (ids -1, 0, 1, 2 in any order) from 8-bit RGBA.
    pub fn rgba_with(name: &str, rect: [i32; 4], rgba: &[u8], compression: u16, order: &[i16]) -> Layer {
        let channels = order
            .iter()
            .map(|&id| {
                let k = if id == -1 { 3 } else { id as usize };
                Chan { id, compression, samples: rgba.iter().skip(k).step_by(4).copied().collect(), raw_override: None }
            })
            .collect();
        Layer {
            rect,
            channels,
            blend: *b"norm",
            opacity: 255,
            clipping: 0,
            flags: 0x08,
            mask_data: Vec::new(),
            ranges: DEFAULT_PAIR.repeat(5),
            name: name.as_bytes().to_vec(),
            name_pad: None,
            blocks: Vec::new(),
        }
    }

    /// An empty layer (rect 0, 0, 0, 0; four channels of 2 bytes).
    pub fn empty(name: &str) -> Layer {
        Layer::rgba_with(name, [0, 0, 0, 0], &[], 0, &[-1, 0, 1, 2])
    }

    pub fn block(mut self, key: &[u8; 4], data: Vec<u8>) -> Layer {
        self.blocks.push((*key, data, Vec::new()));
        self
    }

    pub fn lyid(self, id: u32) -> Layer {
        self.block(b"lyid", id.to_be_bytes().to_vec())
    }

    /// Adds a user mask (channel -2) over `rect` with `values`, default colour and flags.
    pub fn mask(mut self, rect: [i32; 4], values: &[u8], default: u8, flags: u8) -> Layer {
        let mut d = Vec::new();
        for v in rect {
            d.extend_from_slice(&v.to_be_bytes());
        }
        d.push(default);
        d.push(flags);
        d.extend_from_slice(&[0, 0]);
        self.mask_data = d;
        self.channels.push(Chan { id: -2, compression: 1, samples: values.to_vec(), raw_override: None });
        self
    }
}

/// A whole file.
#[derive(Clone, Debug)]
pub struct File {
    pub psb: bool,
    pub channels: u16,
    pub width: u32,
    pub height: u32,
    pub depth: u16,
    pub mode: u16,
    /// `(id, data)`; written with signature `8BIM` and an empty name.
    pub resources: Vec<(u16, Vec<u8>)>,
    pub layers: Vec<Layer>,
    /// Write the layer count negative (the merged picture's first extra channel is transparency).
    pub negative: bool,
    /// Put the layer info into a global `Lr16` block (and leave the section's layer info empty).
    pub in_lr16: bool,
    /// Global tagged blocks after the global layer mask info: `(signature, key, data)`, padded to 4.
    pub global_blocks: Vec<([u8; 4], [u8; 4], Vec<u8>)>,
    /// The merged planes (raw samples, one per header channel).
    pub merged: Vec<Vec<u8>>,
    pub merged_compression: u16,
}

impl File {
    /// An 8-bit RGB file of `w x h` with an opaque white merged picture and no layers.
    pub fn rgb(w: u32, h: u32) -> File {
        let n = (w * h) as usize;
        File {
            psb: false,
            channels: 3,
            width: w,
            height: h,
            depth: 8,
            mode: 3,
            resources: Vec::new(),
            layers: Vec::new(),
            negative: false,
            in_lr16: false,
            global_blocks: Vec::new(),
            merged: vec![vec![255; n]; 3],
            merged_compression: 1,
        }
    }

    /// Sets the merged picture from straight RGBA: matted on white, 4 channels with a negative layer count when
    /// some alpha is below 255, 3 channels otherwise.
    pub fn merged_rgba(mut self, rgba: &[u8]) -> File {
        let opaque = rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
        let mut planes = vec![Vec::new(), Vec::new(), Vec::new()];
        let mut alpha = Vec::new();
        for p in rgba.as_chunks::<4>().0.iter() {
            let a = p[3] as u32;
            for k in 0..3 {
                planes[k].push(((p[k] as u32 * a + 255 * (255 - a) + 127) / 255) as u8);
            }
            alpha.push(p[3]);
        }
        if !opaque {
            planes.push(alpha);
            self.negative = true;
        }
        self.channels = planes.len() as u16;
        self.merged = planes;
        self
    }

    /// Resource 1057 with the given `hasRealMergedData`.
    pub fn version_info(mut self, real: bool) -> File {
        let mut d = vec![0, 0, 0, 1, real as u8];
        for _ in 0..2 {
            d.extend_from_slice(&3u32.to_be_bytes());
            for u in "WTE".encode_utf16() {
                d.extend_from_slice(&u.to_be_bytes());
            }
        }
        d.extend_from_slice(&1u32.to_be_bytes());
        self.resources.push((1057, d));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(b"8BPS");
        o.extend_from_slice(&(if self.psb { 2u16 } else { 1u16 }).to_be_bytes());
        o.extend_from_slice(&[0; 6]);
        o.extend_from_slice(&self.channels.to_be_bytes());
        o.extend_from_slice(&self.height.to_be_bytes());
        o.extend_from_slice(&self.width.to_be_bytes());
        o.extend_from_slice(&self.depth.to_be_bytes());
        o.extend_from_slice(&self.mode.to_be_bytes());
        o.extend_from_slice(&0u32.to_be_bytes());

        let mut res = Vec::new();
        for (id, data) in &self.resources {
            res.extend_from_slice(b"8BIM");
            res.extend_from_slice(&id.to_be_bytes());
            res.extend_from_slice(&[0, 0]);
            res.extend_from_slice(&(data.len() as u32).to_be_bytes());
            res.extend_from_slice(data);
            if !data.len().is_multiple_of(2) {
                res.push(0);
            }
        }
        o.extend_from_slice(&(res.len() as u32).to_be_bytes());
        o.extend_from_slice(&res);

        let bps = (self.depth / 8).max(1) as usize;
        let mut section = Vec::new();
        let info = if self.layers.is_empty() && !self.negative { Vec::new() } else { self.layer_info(bps) };
        if self.in_lr16 {
            put_len(&mut section, 0, self.psb);
        } else {
            put_len(&mut section, info.len() as u64, self.psb);
            section.extend_from_slice(&info);
        }
        section.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info
        let mut globals = self.global_blocks.clone();
        if self.in_lr16 {
            globals.insert(0, (*b"8BIM", *b"Lr16", info));
        }
        for (sig, key, data) in &globals {
            section.extend_from_slice(sig);
            section.extend_from_slice(key);
            let long = self.psb && super::read::psb_long_key(key);
            let padded = data.len().div_ceil(4) * 4;
            put_len_sized(&mut section, padded as u64, long);
            section.extend_from_slice(data);
            section.resize(section.len() + padded - data.len(), 0);
        }
        put_len(&mut o, section.len() as u64, self.psb);
        o.extend_from_slice(&section);

        // Merged picture.
        o.extend_from_slice(&self.merged_compression.to_be_bytes());
        let row = self.width as usize * bps;
        match self.merged_compression {
            0 => {
                for p in &self.merged {
                    o.extend_from_slice(p);
                }
            }
            1 => {
                let mut counts = Vec::new();
                let mut rows = Vec::new();
                for p in &self.merged {
                    for r in p.chunks(row.max(1)) {
                        let mut e = Vec::new();
                        encode_row(r, &mut e);
                        if self.psb {
                            counts.extend_from_slice(&(e.len() as u32).to_be_bytes());
                        } else {
                            counts.extend_from_slice(&(e.len() as u16).to_be_bytes());
                        }
                        rows.extend_from_slice(&e);
                    }
                }
                o.extend_from_slice(&counts);
                o.extend_from_slice(&rows);
            }
            c => {
                let mut all: Vec<u8> = self.merged.concat();
                if c == 3 {
                    predict(&mut all, row, bps);
                }
                o.extend_from_slice(&compress_to_vec_zlib(&all, 6));
            }
        }
        o
    }

    fn layer_info(&self, bps: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let n = self.layers.len() as i16;
        out.extend_from_slice(&(if self.negative { -n } else { n }).to_be_bytes());
        let mut data = Vec::new();
        for l in &self.layers {
            let w = (l.rect[3] - l.rect[1]).max(0) as usize;
            let h = (l.rect[2] - l.rect[0]).max(0) as usize;
            for v in l.rect {
                out.extend_from_slice(&v.to_be_bytes());
            }
            out.extend_from_slice(&(l.channels.len() as u16).to_be_bytes());
            let mut encoded = Vec::new();
            for ch in &l.channels {
                let (cw, chh) = if ch.id == -2 && l.mask_data.len() >= 16 {
                    let r = |i: usize| i32::from_be_bytes(l.mask_data[i * 4..i * 4 + 4].try_into().unwrap());
                    ((r(3) - r(1)).max(0) as usize, (r(2) - r(0)).max(0) as usize)
                } else {
                    (w, h)
                };
                let mut e = ch.compression.to_be_bytes().to_vec();
                match &ch.raw_override {
                    Some(raw) => e.extend_from_slice(raw),
                    None => e.extend_from_slice(&encode(&ch.samples, ch.compression, cw * bps, chh, bps, self.psb)),
                }
                out.extend_from_slice(&ch.id.to_be_bytes());
                put_len(&mut out, e.len() as u64, self.psb);
                encoded.push(e);
            }
            out.extend_from_slice(b"8BIM");
            out.extend_from_slice(&l.blend);
            out.extend_from_slice(&[l.opacity, l.clipping, l.flags, 0]);
            let mut extra = Vec::new();
            extra.extend_from_slice(&(l.mask_data.len() as u32).to_be_bytes());
            extra.extend_from_slice(&l.mask_data);
            extra.extend_from_slice(&(l.ranges.len() as u32).to_be_bytes());
            extra.extend_from_slice(&l.ranges);
            extra.push(l.name.len() as u8);
            extra.extend_from_slice(&l.name);
            let pad = l.name_pad.unwrap_or((4 - (l.name.len() + 1) % 4) % 4);
            extra.resize(extra.len() + pad, 0);
            for (key, data, pad) in &l.blocks {
                extra.extend_from_slice(b"8BIM");
                extra.extend_from_slice(key);
                let long = self.psb && super::read::psb_long_key(key);
                let padded = data.len().div_ceil(2) * 2;
                put_len_sized(&mut extra, padded as u64, long);
                extra.extend_from_slice(data);
                extra.resize(extra.len() + padded - data.len(), 0);
                extra.extend_from_slice(pad);
            }
            out.extend_from_slice(&(extra.len() as u32).to_be_bytes());
            out.extend_from_slice(&extra);
            for e in encoded {
                data.extend_from_slice(&e);
            }
        }
        out.extend_from_slice(&data);
        let padded = out.len().div_ceil(4) * 4;
        out.resize(padded, 0);
        out
    }
}

/// A 4-byte (PSD) or 8-byte (PSB) section length.
fn put_len(out: &mut Vec<u8>, n: u64, psb: bool) {
    put_len_sized(out, n, psb);
}

fn put_len_sized(out: &mut Vec<u8>, n: u64, long: bool) {
    if long {
        out.extend_from_slice(&n.to_be_bytes());
    } else {
        out.extend_from_slice(&(n as u32).to_be_bytes());
    }
}

/// Encodes `h` rows of `row` bytes with `compression` (the bytes after the compression word).
pub fn encode(samples: &[u8], compression: u16, row: usize, h: usize, bps: usize, psb: bool) -> Vec<u8> {
    match compression {
        0 => samples.to_vec(),
        1 => {
            let mut counts = Vec::new();
            let mut rows = Vec::new();
            if row > 0 {
                for r in samples.chunks(row).take(h) {
                    let mut e = Vec::new();
                    encode_row(r, &mut e);
                    if psb {
                        counts.extend_from_slice(&(e.len() as u32).to_be_bytes());
                    } else {
                        counts.extend_from_slice(&(e.len() as u16).to_be_bytes());
                    }
                    rows.extend_from_slice(&e);
                }
            }
            counts.extend_from_slice(&rows);
            counts
        }
        c => {
            if samples.is_empty() {
                return Vec::new();
            }
            let mut v = samples.to_vec();
            if c == 3 {
                predict(&mut v, row, bps);
            }
            compress_to_vec_zlib(&v, 6)
        }
    }
}

/// ZIP prediction: per row, each sample becomes its difference to the one before it.
pub fn predict(buf: &mut [u8], row: usize, bps: usize) {
    if row == 0 {
        return;
    }
    for r in buf.chunks_exact_mut(row) {
        if bps == 1 {
            for i in (1..r.len()).rev() {
                r[i] = r[i].wrapping_sub(r[i - 1]);
            }
        } else {
            let words: Vec<u16> = r.as_chunks::<2>().0.iter().map(|w| u16::from_be_bytes([w[0], w[1]])).collect();
            for (i, w) in r.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let v = if i == 0 { words[0] } else { words[i].wrapping_sub(words[i - 1]) };
                w.copy_from_slice(&v.to_be_bytes());
            }
        }
    }
}

/// A test descriptor builder (unicode name, class id, items).
pub struct Desc(pub Vec<u8>);

impl Desc {
    pub fn new(class: &[u8], items: u32) -> Desc {
        let mut d = Vec::new();
        d.extend_from_slice(&1u32.to_be_bytes());
        d.extend_from_slice(&0u16.to_be_bytes());
        key(&mut d, class);
        d.extend_from_slice(&items.to_be_bytes());
        Desc(d)
    }

    pub fn item(mut self, k: &[u8], ty: &[u8; 4], value: &[u8]) -> Desc {
        key(&mut self.0, k);
        self.0.extend_from_slice(ty);
        self.0.extend_from_slice(value);
        self
    }

    pub fn bool(self, k: &[u8], v: bool) -> Desc {
        self.item(k, b"bool", &[v as u8])
    }

    /// The body of an `lfx2` block: versions 0 and 16, then this descriptor.
    pub fn effects_block(self) -> Vec<u8> {
        let mut d = vec![0, 0, 0, 0, 0, 0, 0, 16];
        d.extend_from_slice(&self.0);
        d
    }
}

/// "Length and key": a 4-byte key is written with length 0.
pub fn key(out: &mut Vec<u8>, k: &[u8]) {
    if k.len() == 4 {
        out.extend_from_slice(&0u32.to_be_bytes());
    } else {
        out.extend_from_slice(&(k.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(k);
}

/// A unicode string value.
pub fn unicode(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut d = (units.len() as u32).to_be_bytes().to_vec();
    for u in units {
        d.extend_from_slice(&u.to_be_bytes());
    }
    d
}

/// Straight-alpha composite of the visible picture layers of `doc` on a transparent canvas: WTE's own blend kernel,
/// layer opacity, the user mask (unless disabled) and clipping to the nearest unclipped layer below. Layers without
/// pixels are skipped. The test oracle for "composite = merged".
pub fn composite(doc: &PsdDoc) -> PixelBuf {
    let [w, h] = doc.size;
    let mut out = PixelBuf::new(w, h);
    let mut base_alpha: Option<Vec<u8>> = None;
    for layer in &doc.layers {
        let Some(px) = layer.pixels.as_ref() else {
            if !layer.clipped {
                base_alpha = None;
            }
            continue;
        };
        // The layer's own coverage on the canvas (alpha times mask), before opacity.
        let mut cov = vec![0u8; (w * h) as usize];
        let mut colour = vec![[0u8; 4]; (w * h) as usize];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let p = px.get(x, y);
                let m = mask_value(layer, x, y);
                let i = (y as u32 * w + x as u32) as usize;
                cov[i] = ((p[3] as u32 * m as u32 + 127) / 255) as u8;
                colour[i] = p;
            }
        }
        if layer.clipped {
            if let Some(base) = &base_alpha {
                for (c, b) in cov.iter_mut().zip(base) {
                    *c = ((*c as u32 * *b as u32 + 127) / 255) as u8;
                }
            }
        } else {
            base_alpha = Some(cov.clone());
        }
        if !layer.visible {
            continue;
        }
        let mode = match layer.blend {
            PsdBlend::Known(m) => m,
            PsdBlend::Other(_) => BlendMode::Normal,
        };
        for i in 0..(w * h) as usize {
            let mut s = colour[i];
            s[3] = cov[i];
            let d = &mut out.data[i * 4..i * 4 + 4];
            let r = blend_pixel([d[0], d[1], d[2], d[3]], s, layer.opacity, mode);
            d.copy_from_slice(&r);
        }
    }
    out
}

fn mask_value(layer: &PsdLayer, x: i32, y: i32) -> u8 {
    match &layer.mask {
        Some(m) if !m.disabled => m.value(x, y),
        _ => 255,
    }
}

/// Largest per-channel difference between two pictures over pixels where both alphas are at least `min_alpha`
/// (colours) plus the largest alpha difference everywhere.
pub fn max_diff(a: &PixelBuf, b: &PixelBuf, min_alpha: u8) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height), "picture sizes differ");
    let mut worst = 0u8;
    for (p, q) in a.data.as_chunks::<4>().0.iter().zip(b.data.as_chunks::<4>().0.iter()) {
        worst = worst.max(p[3].abs_diff(q[3]));
        if p[3] >= min_alpha && q[3] >= min_alpha {
            for k in 0..3 {
                worst = worst.max(p[k].abs_diff(q[k]));
            }
        }
    }
    worst
}

/// The layer named `name` (panics when missing).
pub fn layer<'a>(doc: &'a PsdDoc, name: &str) -> &'a PsdLayer {
    doc.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no layer {name:?}"))
}

/// How a link would take the file (the rule P5c implements, 5.4 / 13.2): `Layers` when nothing forces, `Merged`
/// when something forces and the merged picture is real, `NeedsMerged` otherwise.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Layers,
    Merged,
    NeedsMerged,
}

pub fn mode(doc: &PsdDoc, smart_forces: bool) -> Mode {
    if doc.forcing(smart_forces).is_empty() && !doc.is_flat() {
        Mode::Layers
    } else if doc.merged.is_some() {
        Mode::Merged
    } else {
        Mode::NeedsMerged
    }
}

/// A small deterministic random generator (xorshift64*), for byte flips.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
