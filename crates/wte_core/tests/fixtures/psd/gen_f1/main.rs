// Writes `f1_wte_in.psd`: a three-layer 64x48 sticker PSD made with WTE's texture PSD writer
// (`wte_core::export::write_psd`, unchanged since v0.3.0). Fixture input of STICKER_PLAN 8.3 f1;
// see README.md next to this folder for the command lines. Not part of any build.
//
// Layers (bottom to top, all Normal, 100 %):
// 1. "photo": a soft-edged picture (colour gradient, alpha 255 inside radius 12, falling to 0 at
//    radius 20 around (22, 24)). The RGB of the gradient is kept under alpha 0 inside the layer's
//    content box (a cut-out photo keeps its colours), so the fixture shows what PhotoCraft does with
//    RGB under alpha 0.
// 2. "text": the raster of the word "WTE" in WTE's bundled default font at 18 px, near black.
// 3. "shape": an anti-aliased red disc of radius 9 around (50, 36) (a shape layer's raster).

use std::fs::File;
use std::io::BufWriter;

use wte_core::export::write_psd;
use wte_core::text::default_font_bytes;
use wte_core::{BlendMode, ExportLayer, ExportSet, PixelBuf};

const W: u32 = 64;
const H: u32 = 48;

fn put(buf: &mut PixelBuf, x: u32, y: u32, px: [u8; 4]) {
    let i = (y as usize * buf.width as usize + x as usize) * 4;
    buf.data[i..i + 4].copy_from_slice(&px);
}

fn photo() -> PixelBuf {
    let mut b = PixelBuf::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let (dx, dy) = (x as f32 + 0.5 - 22.0, y as f32 + 0.5 - 24.0);
            let d = (dx * dx + dy * dy).sqrt();
            let a = ((20.0 - d) / 8.0).clamp(0.0, 1.0);
            let r = (40 + x * 3) as u8;
            let g = (200 - y * 3) as u8;
            let bl = 160u8;
            // Colour everywhere within radius 24 (also where alpha is 0), nothing outside.
            if d <= 24.0 {
                put(&mut b, x, y, [r, g, bl, (a * 255.0).round() as u8]);
            }
        }
    }
    b
}

fn text() -> PixelBuf {
    let font = fontdue::Font::from_bytes(default_font_bytes(), fontdue::FontSettings::default()).expect("font");
    let mut b = PixelBuf::new(W, H);
    let mut pen_x = 30.0f32;
    let baseline = 20.0f32;
    for ch in "WTE".chars() {
        let (m, bitmap) = font.rasterize(ch, 18.0);
        let x0 = pen_x.round() as i32 + m.xmin;
        let y0 = baseline.round() as i32 - m.height as i32 - m.ymin;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let cov = bitmap[gy * m.width + gx];
                let (x, y) = (x0 + gx as i32, y0 + gy as i32);
                if cov == 0 || x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
                    continue;
                }
                put(&mut b, x as u32, y as u32, [20, 20, 30, cov]);
            }
        }
        pen_x += m.advance_width;
    }
    b
}

fn shape() -> PixelBuf {
    let mut b = PixelBuf::new(W, H);
    for y in 0..H {
        for x in 0..W {
            // 4x4 supersampled coverage of a disc.
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
            if n > 0 {
                put(&mut b, x, y, [220, 30, 40, ((n * 255 + 8) / 16) as u8]);
            }
        }
    }
    b
}

/// Straight-alpha source-over of `layers` (bottom to top).
fn flatten(layers: &[&PixelBuf]) -> PixelBuf {
    let mut out = PixelBuf::new(W, H);
    for (i, o) in out.data.chunks_exact_mut(4).enumerate() {
        let mut acc = [0f32; 4];
        for l in layers {
            let s = &l.data[i * 4..i * 4 + 4];
            let sa = s[3] as f32 / 255.0;
            let da = acc[3];
            let oa = sa + da * (1.0 - sa);
            for c in 0..3 {
                let sc = s[c] as f32 / 255.0;
                acc[c] = if oa > 0.0 { (sc * sa + acc[c] * da * (1.0 - sa)) / oa } else { 0.0 };
            }
            acc[3] = oa;
        }
        for c in 0..4 {
            o[c] = (acc[c] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "f1_wte_in.psd".to_owned());
    let (p, t, s) = (photo(), text(), shape());
    let flat = flatten(&[&p, &t, &s]);
    let layer = |name, buf: &'static PixelBuf| ExportLayer {
        name,
        kind: "paint",
        visible: true,
        opacity: 1.0,
        blend: BlendMode::Normal,
        pixels: buf.view(),
    };
    let (p, t, s): (&'static PixelBuf, &'static PixelBuf, &'static PixelBuf) =
        (Box::leak(Box::new(p)), Box::leak(Box::new(t)), Box::leak(Box::new(s)));
    let set = ExportSet {
        name: "f1",
        width: W,
        height: H,
        flattened: &flat,
        layers: vec![layer("photo", p), layer("text", t), layer("shape", s)],
    };
    let f = BufWriter::new(File::create(&out).expect("create"));
    write_psd(f, &set).expect("write_psd");
    println!("wrote {out}");
}
