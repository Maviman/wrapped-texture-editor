//! Picture encoding for the companion stream (COMPANION_PLAN 2.4), pure: the captured window (premultiplied RGBA,
//! opaque) to RGB with an integer box downscale, the 64-px cell diff against the last sent picture, the merge of
//! changed cells into grid-aligned rectangles (at most 512 x 512), JPEG tiles (`wte_core::encode_jpeg`, in parallel
//! when there are several), the refine pass of cells sent at motion quality, the link profiles and the
//! `RateController`.

use std::time::{Duration, Instant};

use eframe::egui;

use super::proto::{self, Frame, Tile, codec, frame_flag};

/// Side of a diff cell in picture pixels.
pub const CELL: u32 = 64;
/// Largest rectangle side in cells (512 px).
pub const MAX_RECT_CELLS: u32 = 8;
/// Idle time after the last change before low-quality cells are sent again at the refine quality.
pub const REFINE_AFTER: Duration = Duration::from_millis(400);

/// A tightly packed RGB8 picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbPic {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl RgbPic {
    /// The RGB bytes of a pixel rectangle (rows copied).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 3) as usize);
        for row in y..y + h {
            let start = ((row * self.w + x) * 3) as usize;
            out.extend_from_slice(&self.data[start..start + (w * 3) as usize]);
        }
        out
    }
}

/// The picture size of a surface at scale `den` (rounded up).
pub fn pic_size(surface: [u32; 2], den: u32) -> [u32; 2] {
    let den = den.max(1);
    [surface[0].div_ceil(den), surface[1].div_ceil(den)]
}

/// A captured `ColorImage` as RGB, box-downscaled by `den` (1..=4; blocks clipped at the right and bottom edges).
/// Alpha is ignored: the window is opaque, so premultiplied == straight.
pub fn to_rgb(image: &egui::ColorImage, den: u32) -> RgbPic {
    let [w, h] = [image.size[0] as u32, image.size[1] as u32];
    let den = den.clamp(1, 4);
    let [pw, ph] = pic_size([w, h], den);
    let mut data = Vec::with_capacity((pw * ph * 3) as usize);
    if den == 1 {
        for p in &image.pixels {
            data.extend_from_slice(&[p.r(), p.g(), p.b()]);
        }
        return RgbPic { w: pw, h: ph, data };
    }
    for py in 0..ph {
        let y0 = py * den;
        let y1 = (y0 + den).min(h);
        for px in 0..pw {
            let x0 = px * den;
            let x1 = (x0 + den).min(w);
            let mut sum = [0u32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = image.pixels[(y * w + x) as usize];
                    sum[0] += u32::from(p.r());
                    sum[1] += u32::from(p.g());
                    sum[2] += u32::from(p.b());
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            data.extend(sum.map(|s| ((s + n / 2) / n) as u8));
        }
    }
    RgbPic { w: pw, h: ph, data }
}

/// Columns and rows of cells of a picture.
pub fn grid(w: u32, h: u32) -> (u32, u32) {
    (w.div_ceil(CELL), h.div_ceil(CELL))
}

/// Which cells differ between two pictures of the same size (row-major).
pub fn changed_cells(prev: &RgbPic, cur: &RgbPic) -> Vec<bool> {
    debug_assert_eq!((prev.w, prev.h), (cur.w, cur.h));
    let (cols, rows) = grid(cur.w, cur.h);
    let mut changed = vec![false; (cols * rows) as usize];
    let row_bytes = (cur.w * 3) as usize;
    let cell_bytes = (CELL * 3) as usize;
    for y in 0..cur.h {
        let a = &prev.data[y as usize * row_bytes..(y as usize + 1) * row_bytes];
        let b = &cur.data[y as usize * row_bytes..(y as usize + 1) * row_bytes];
        let row = (y / CELL) * cols;
        for c in 0..cols {
            let i = (row + c) as usize;
            if changed[i] {
                continue;
            }
            let s = c as usize * cell_bytes;
            let e = (s + cell_bytes).min(row_bytes);
            if a[s..e] != b[s..e] {
                changed[i] = true;
            }
        }
    }
    changed
}

/// A rectangle in cell units.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub cx: u32,
    pub cy: u32,
    pub cw: u32,
    pub ch: u32,
}

impl CellRect {
    /// The pixel rectangle inside a `w x h` picture: (x, y, w, h).
    pub fn pixels(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let x = self.cx * CELL;
        let y = self.cy * CELL;
        (x, y, (self.cw * CELL).min(w - x), (self.ch * CELL).min(h - y))
    }
}

/// Merges marked cells into rectangles: runs along each row (split at `MAX_RECT_CELLS`), then a run grows down while
/// the next row has a run with the same start and width (up to `MAX_RECT_CELLS` rows). The rectangles cover exactly the
/// marked cells and never overlap.
pub fn merge_cells(marked: &[bool], cols: u32, rows: u32) -> Vec<CellRect> {
    let mut done = Vec::new();
    let mut open: Vec<CellRect> = Vec::new();
    for cy in 0..rows {
        let mut runs = Vec::new();
        let mut cx = 0;
        while cx < cols {
            if marked[(cy * cols + cx) as usize] {
                let start = cx;
                while cx < cols && marked[(cy * cols + cx) as usize] && cx - start < MAX_RECT_CELLS {
                    cx += 1;
                }
                runs.push((start, cx - start));
            } else {
                cx += 1;
            }
        }
        let mut next_open = Vec::with_capacity(runs.len());
        for (start, len) in runs {
            if let Some(i) = open.iter().position(|r| r.cx == start && r.cw == len && r.ch < MAX_RECT_CELLS) {
                let mut r = open.swap_remove(i);
                r.ch += 1;
                next_open.push(r);
            } else {
                next_open.push(CellRect { cx: start, cy, cw: len, ch: 1 });
            }
        }
        done.append(&mut open);
        open = next_open;
    }
    done.append(&mut open);
    done.sort_by_key(|r| (r.cy, r.cx));
    done
}

/// Threads for the JPEG tiles of one frame: `min(4, cores - 2)`, at least 1.
pub fn encode_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    cores.saturating_sub(2).clamp(1, 4)
}

/// JPEG tiles of `rects` of `pic` at `quality`, in parallel on up to `threads` threads when there are two or more.
pub fn encode_tiles(pic: &RgbPic, rects: &[CellRect], quality: u8, threads: usize) -> Vec<Tile> {
    let one = |r: &CellRect| -> Tile {
        let (x, y, w, h) = r.pixels(pic.w, pic.h);
        let data = wte_core::encode_jpeg(w, h, &pic.crop(x, y, w, h), quality).unwrap_or_default();
        Tile { x: x as u16, y: y as u16, w: w as u16, h: h as u16, codec: codec::JPEG, data }
    };
    if rects.len() < 2 || threads < 2 {
        return rects.iter().map(one).collect();
    }
    let mut out: Vec<Option<Tile>> = vec![None; rects.len()];
    let per = rects.len().div_ceil(threads);
    std::thread::scope(|s| {
        for (slots, chunk) in out.chunks_mut(per).zip(rects.chunks(per)) {
            s.spawn(move || {
                for (slot, r) in slots.iter_mut().zip(chunk) {
                    *slot = Some(one(r));
                }
            });
        }
    });
    out.into_iter().map(|t| t.expect("every tile encoded")).collect()
}

/// The encoder's settings for one frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Params {
    pub den: u32,
    pub quality: u8,
    pub refine_quality: u8,
}

/// The per-session encoder state: the last sent picture (what the tablet shows, in source pixels), the quality each
/// cell was last sent at, the frame counter.
pub struct Encoder {
    last: Option<RgbPic>,
    cell_quality: Vec<u8>,
    den: u32,
    surface: [u32; 2],
    frame_seq: u32,
    last_change: Option<Instant>,
    sent_input_seq: u32,
    /// The next frame is a KEY frame (first frame, reconnect, restore after minimize).
    pub need_key: bool,
    pub threads: usize,
}

impl Default for Encoder {
    fn default() -> Encoder {
        Encoder::new()
    }
}

impl Encoder {
    pub fn new() -> Encoder {
        Encoder {
            last: None,
            cell_quality: Vec::new(),
            den: 1,
            surface: [0, 0],
            frame_seq: 0,
            last_change: None,
            sent_input_seq: 0,
            need_key: true,
            threads: encode_threads(),
        }
    }

    /// The surface size of the last encoded frame.
    pub fn surface(&self) -> [u32; 2] {
        self.surface
    }

    /// The FRAME for a captured image, or `None` when nothing changed and no input is to be confirmed.
    pub fn frame(&mut self, image: &egui::ColorImage, p: Params, last_input_seq: u32, capture_ms: u32, now: Instant) -> Option<Frame> {
        let surface = [image.size[0] as u32, image.size[1] as u32];
        if surface[0] == 0 || surface[1] == 0 {
            return None;
        }
        let pic = to_rgb(image, p.den);
        let (cols, rows) = grid(pic.w, pic.h);
        let key = self.need_key
            || self.den != p.den
            || self.surface != surface
            || self.last.as_ref().is_none_or(|l| (l.w, l.h) != (pic.w, pic.h));
        let marked = if key { vec![true; (cols * rows) as usize] } else { changed_cells(self.last.as_ref().expect("not key"), &pic) };
        let any = marked.iter().any(|&m| m);
        if !any && last_input_seq <= self.sent_input_seq {
            return None;
        }
        let rects = merge_cells(&marked, cols, rows);
        let tiles = encode_tiles(&pic, &rects, p.quality, self.threads);
        if key {
            self.cell_quality = vec![p.quality; marked.len()];
        } else {
            for (q, m) in self.cell_quality.iter_mut().zip(&marked) {
                if *m {
                    *q = p.quality;
                }
            }
        }
        if any {
            self.last_change = Some(now);
        }
        self.frame_seq += 1;
        self.sent_input_seq = self.sent_input_seq.max(last_input_seq);
        self.need_key = false;
        self.den = p.den;
        self.surface = surface;
        let frame = Frame {
            frame_seq: self.frame_seq,
            flags: if key { frame_flag::KEY } else { 0 },
            den: p.den as u8,
            pic_w: pic.w as u16,
            pic_h: pic.h as u16,
            last_input_seq,
            capture_ms,
            tiles,
        };
        self.last = Some(pic);
        Some(frame)
    }

    /// True when a refine frame is due at `now` (idle long enough and some cell below the refine quality).
    pub fn refine_due(&self, p: Params, now: Instant) -> bool {
        self.last.is_some()
            && self.den == p.den
            && self.last_change.is_some_and(|t| now.saturating_duration_since(t) >= REFINE_AFTER)
            && self.cell_quality.iter().any(|&q| q < p.refine_quality)
    }

    /// When the next refine is due (for the writer's wait), `None` when nothing waits for one.
    pub fn refine_at(&self, p: Params) -> Option<Instant> {
        (self.last.is_some() && self.cell_quality.iter().any(|&q| q < p.refine_quality))
            .then(|| self.last_change.map(|t| t + REFINE_AFTER))
            .flatten()
    }

    /// The REFINE frame: the cells sent below `refine_quality`, again at that quality, from the last sent picture.
    pub fn refine(&mut self, p: Params, now: Instant) -> Option<Frame> {
        if !self.refine_due(p, now) {
            return None;
        }
        let pic = self.last.as_ref().expect("refine_due");
        let (cols, rows) = grid(pic.w, pic.h);
        let marked: Vec<bool> = self.cell_quality.iter().map(|&q| q < p.refine_quality).collect();
        let rects = merge_cells(&marked, cols, rows);
        let tiles = encode_tiles(pic, &rects, p.refine_quality, self.threads);
        for (q, m) in self.cell_quality.iter_mut().zip(&marked) {
            if *m {
                *q = p.refine_quality;
            }
        }
        self.frame_seq += 1;
        Some(Frame {
            frame_seq: self.frame_seq,
            flags: frame_flag::REFINE,
            den: p.den as u8,
            pic_w: pic.w as u16,
            pic_h: pic.h as u16,
            last_input_seq: self.sent_input_seq,
            capture_ms: 0,
            tiles,
        })
    }
}

/// A link's ceiling (2.4 table).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub den: u32,
    pub quality: u8,
    pub refine: u8,
    pub fps: u32,
    pub window: u32,
}

/// The profile of a HELLO `link`.
pub fn profile(link: u8) -> Profile {
    match link {
        proto::link::BLUETOOTH => Profile { den: 2, quality: 50, refine: 80, fps: 8, window: 1 },
        proto::link::WIFI => Profile { den: 1, quality: 75, refine: 92, fps: 30, window: 2 },
        _ => Profile { den: 1, quality: 80, refine: 92, fps: 30, window: 2 },
    }
}

/// The ceiling after the tablet's CONFIG: `quality_pref` 1 low / 2 medium lowers the qualities, `max_fps` > 0 caps the
/// frame rate.
pub fn apply_config(p: Profile, quality_pref: u8, max_fps: u8) -> Profile {
    let (q, r) = match quality_pref {
        1 => (p.quality.min(50), p.refine.min(75)),
        2 => (p.quality.min(65), p.refine.min(85)),
        _ => (p.quality, p.refine),
    };
    let fps = if max_fps > 0 { p.fps.min(u32::from(max_fps)).max(1) } else { p.fps };
    Profile { quality: q, refine: r, fps, ..p }
}

/// Quality floor of the rate controller.
pub const MIN_QUALITY: u8 = 40;
/// Largest scale-down.
pub const MAX_DEN: u32 = 4;
/// Lowest frame rate.
pub const MIN_FPS: u32 = 4;
/// Send-to-ack time that counts as slow / fast.
pub const SLOW: Duration = Duration::from_millis(150);
pub const FAST: Duration = Duration::from_millis(50);
/// Slow acks in a row before a step down; fast time before a step up.
pub const SLOW_STREAK: u32 = 3;
pub const FAST_FOR: Duration = Duration::from_secs(2);

/// Moves the encoder settings below the link's ceiling by the send-to-ack time (2.4 "RateController").
#[derive(Clone, Debug)]
pub struct RateController {
    ceiling: Profile,
    quality: u8,
    den: u32,
    fps: u32,
    slow: u32,
    fast_since: Option<Instant>,
}

impl RateController {
    pub fn new(ceiling: Profile) -> RateController {
        RateController { ceiling, quality: ceiling.quality, den: ceiling.den, fps: ceiling.fps, slow: 0, fast_since: None }
    }

    /// A new ceiling (CONFIG): the current level is clamped under it.
    pub fn set_ceiling(&mut self, ceiling: Profile) {
        self.ceiling = ceiling;
        self.quality = self.quality.min(ceiling.quality);
        self.den = self.den.max(ceiling.den);
        self.fps = self.fps.min(ceiling.fps);
    }

    pub fn ceiling(&self) -> Profile {
        self.ceiling
    }

    pub fn params(&self) -> Params {
        Params { den: self.den, quality: self.quality, refine_quality: self.ceiling.refine.max(self.quality) }
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// One acknowledged frame; returns true when `den` changed (the next frame must be a KEY frame).
    pub fn on_ack(&mut self, send_to_ack: Duration, now: Instant) -> bool {
        let den = self.den;
        if send_to_ack > SLOW {
            self.fast_since = None;
            self.slow += 1;
            if self.slow >= SLOW_STREAK {
                self.slow = 0;
                self.step_down();
            }
        } else {
            self.slow = 0;
            if send_to_ack < FAST {
                let since = *self.fast_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= FAST_FOR {
                    self.step_up();
                    self.fast_since = Some(now);
                }
            } else {
                self.fast_since = None;
            }
        }
        den != self.den
    }

    fn step_down(&mut self) {
        if self.quality > MIN_QUALITY {
            self.quality = self.quality.saturating_sub(15).max(MIN_QUALITY);
        } else if self.den < MAX_DEN {
            self.den += 1;
        } else {
            self.fps = (self.fps / 2).max(MIN_FPS.min(self.ceiling.fps));
        }
    }

    fn step_up(&mut self) {
        if self.fps < self.ceiling.fps {
            self.fps = (self.fps * 2).min(self.ceiling.fps);
        } else if self.den > self.ceiling.den {
            self.den -= 1;
        } else if self.quality < self.ceiling.quality {
            self.quality = (self.quality + 15).min(self.ceiling.quality);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> egui::ColorImage {
        let pixels = (0..w * h).map(|i| {
            let [r, g, b] = f(i % w, i / w);
            egui::Color32::from_rgb(r, g, b)
        });
        egui::ColorImage::new([w, h], pixels.collect())
    }

    #[test]
    fn rgb_and_box_downscale() {
        let img = image(5, 3, |x, y| [(x * 10) as u8, (y * 20) as u8, 7]);
        let full = to_rgb(&img, 1);
        assert_eq!((full.w, full.h), (5, 3));
        assert_eq!(&full.data[..6], &[0, 0, 7, 10, 0, 7]);
        for den in 1..=4u32 {
            let p = to_rgb(&img, den);
            assert_eq!([p.w, p.h], pic_size([5, 3], den), "den {den}");
            assert_eq!(p.data.len() as u32, p.w * p.h * 3);
        }
        let half = to_rgb(&img, 2);
        assert_eq!((half.w, half.h), (3, 2));
        // Block (0,0): x 0..2, y 0..2 -> r (0+10+0+10)/4 = 5, g (0+0+20+20)/4 = 10.
        assert_eq!(&half.data[..3], &[5, 10, 7]);
        // Right edge block: only column 4 -> r 40; bottom row: only y 2 -> g 40.
        assert_eq!(&half.data[6..9], &[40, 10, 7]);
        assert_eq!(&half.data[15..18], &[40, 40, 7]);
        assert_eq!((to_rgb(&img, 4).w, to_rgb(&img, 4).h), (2, 1));
    }

    #[test]
    fn cell_diff() {
        let a = to_rgb(&image(200, 130, |_, _| [1, 2, 3]), 1);
        assert_eq!(grid(200, 130), (4, 3));
        assert!(changed_cells(&a, &a).iter().all(|c| !c));
        // One pixel in cell (1, 0); one at the last pixel (right / bottom border cell).
        let mut b = a.clone();
        let set = |p: &mut RgbPic, x: u32, y: u32| p.data[((y * p.w + x) * 3) as usize] ^= 0xff;
        set(&mut b, 64, 0);
        set(&mut b, 199, 129);
        let c = changed_cells(&a, &b);
        let marked: Vec<usize> = c.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect();
        assert_eq!(marked, vec![1, 11]);
        // Cell borders: x 63 is cell 0, x 128 is cell 2.
        let mut d = a.clone();
        set(&mut d, 63, 70);
        set(&mut d, 128, 70);
        let c = changed_cells(&a, &d);
        assert!(c[4] && !c[5] && c[6] && !c[7]);
    }

    fn covered(rects: &[CellRect], cols: u32, rows: u32) -> Vec<u8> {
        let mut n = vec![0u8; (cols * rows) as usize];
        for r in rects {
            assert!(r.cw >= 1 && r.ch >= 1 && r.cw <= MAX_RECT_CELLS && r.ch <= MAX_RECT_CELLS, "{r:?}");
            for y in r.cy..r.cy + r.ch {
                for x in r.cx..r.cx + r.cw {
                    n[(y * cols + x) as usize] += 1;
                }
            }
        }
        n
    }

    #[test]
    fn merged_rectangles_cover_exactly_the_marked_cells() {
        let (cols, rows) = (21u32, 19u32);
        let mut seed = 12345u32;
        for density in [0u32, 10, 50, 90, 100] {
            let marked: Vec<bool> = (0..cols * rows)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) % 100 < density
                })
                .collect();
            let rects = merge_cells(&marked, cols, rows);
            let n = covered(&rects, cols, rows);
            for (i, m) in marked.iter().enumerate() {
                assert_eq!(n[i], u8::from(*m), "density {density}, cell {i}");
            }
            if density == 100 {
                // Full: 21 x 19 cells in blocks of at most 8 x 8 -> 3 x 3 rectangles.
                assert_eq!(rects.len(), 9);
            }
        }
        // A block of 2 x 3 cells is one rectangle.
        let mut m = vec![false; 25];
        for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2), (1, 3), (2, 3)] {
            m[y * 5 + x] = true;
        }
        assert_eq!(merge_cells(&m, 5, 5), vec![CellRect { cx: 1, cy: 1, cw: 2, ch: 3 }]);
        // Pixel rects are clipped at the picture edge.
        assert_eq!(CellRect { cx: 3, cy: 2, cw: 1, ch: 1 }.pixels(200, 130), (192, 128, 8, 2));
    }

    #[test]
    fn tiles_decode_back_within_tolerance() {
        let pic = to_rgb(&image(300, 200, |x, y| [(x % 256) as u8, (y % 256) as u8, 128]), 1);
        let (cols, rows) = grid(pic.w, pic.h);
        let rects = merge_cells(&vec![true; (cols * rows) as usize], cols, rows);
        for threads in [1, 3] {
            let tiles = encode_tiles(&pic, &rects, 90, threads);
            assert_eq!(tiles.len(), rects.len());
            for t in &tiles {
                let img = wte_core::decode_image(&t.data, Some("t.jpg"), 4096).unwrap();
                assert_eq!(img.original_size, [u32::from(t.w), u32::from(t.h)]);
                let src = pic.crop(t.x.into(), t.y.into(), t.w.into(), t.h.into());
                let mut sum = 0u64;
                for (a, b) in src.chunks(3).zip(img.pixels.data.chunks(4)) {
                    for c in 0..3 {
                        sum += (a[c] as i32 - b[c] as i32).unsigned_abs() as u64;
                    }
                }
                let mean = sum as f64 / src.len() as f64;
                assert!(mean < 3.0, "tile {t:?}: mean error {mean}");
            }
        }
    }

    #[test]
    fn key_then_changed_rectangle_then_nothing_then_refine() {
        let t0 = Instant::now();
        let p = Params { den: 1, quality: 60, refine_quality: 90 };
        let mut e = Encoder::new();
        let a = image(256, 192, |_, _| [10, 20, 30]);
        let f = e.frame(&a, p, 0, 3, t0).unwrap();
        assert_eq!(f.flags, frame_flag::KEY);
        assert_eq!((f.pic_w, f.pic_h, f.frame_seq), (256, 192, 1));
        assert_eq!(f.tiles.len(), 1, "4 x 3 cells fit one rectangle");
        // Unchanged, no new input: nothing. New input: an empty frame that confirms it.
        assert!(e.frame(&a, p, 0, 3, t0).is_none());
        let f = e.frame(&a, p, 5, 3, t0).unwrap();
        assert!(f.tiles.is_empty() && f.last_input_seq == 5 && f.flags == 0);
        assert!(e.frame(&a, p, 5, 3, t0).is_none());
        // One changed pixel: one 64 x 64 tile.
        let b = image(256, 192, |x, y| if (x, y) == (130, 70) { [255, 0, 0] } else { [10, 20, 30] });
        let f = e.frame(&b, p, 6, 3, t0).unwrap();
        assert_eq!(f.tiles.len(), 1);
        assert_eq!((f.tiles[0].x, f.tiles[0].y, f.tiles[0].w, f.tiles[0].h), (128, 64, 64, 64));
        // Refine: not before 400 ms; then every cell (all were sent at 60) at 90, once.
        assert!(e.refine(p, t0 + Duration::from_millis(399)).is_none());
        let r = e.refine(p, t0 + REFINE_AFTER).unwrap();
        assert_eq!(r.flags, frame_flag::REFINE);
        assert_eq!(r.tiles.iter().map(|t| u32::from(t.w) * u32::from(t.h)).sum::<u32>(), 256 * 192);
        assert!(e.refine(p, t0 + Duration::from_secs(5)).is_none(), "refined once");
        // A change sends only its cell at motion quality; the next refine picks only that cell.
        let c = image(256, 192, |x, y| if (x, y) == (10, 10) { [0, 255, 0] } else { [10, 20, 30] });
        let t1 = t0 + Duration::from_secs(6);
        e.frame(&c, p, 7, 3, t1).unwrap();
        let r = e.refine(p, t1 + REFINE_AFTER).unwrap();
        assert_eq!(r.tiles.len(), 2, "the two cells that changed back and forth: {:?}", r.tiles.iter().map(|t| (t.x, t.y)).collect::<Vec<_>>());
        // A size change or a scale change is a KEY frame.
        let big = image(300, 192, |_, _| [10, 20, 30]);
        assert_eq!(e.frame(&big, p, 7, 3, t1).unwrap().flags, frame_flag::KEY);
        let f = e.frame(&big, Params { den: 2, ..p }, 7, 3, t1).unwrap();
        assert_eq!((f.flags, f.den, f.pic_w, f.pic_h), (frame_flag::KEY, 2, 150, 96));
        e.need_key = true;
        assert_eq!(e.frame(&big, Params { den: 2, ..p }, 7, 3, t1).unwrap().flags, frame_flag::KEY);
    }

    #[test]
    fn rate_controller_steps_down_and_up_within_the_ceiling() {
        let t0 = Instant::now();
        let usb = profile(proto::link::USB);
        assert_eq!(usb, Profile { den: 1, quality: 80, refine: 92, fps: 30, window: 2 });
        assert_eq!(profile(proto::link::BLUETOOTH).window, 1);
        let mut rc = RateController::new(usb);
        let slow = Duration::from_millis(200);
        // Two slow acks: nothing; the third: quality 80 -> 65.
        assert!(!rc.on_ack(slow, t0) && !rc.on_ack(slow, t0));
        rc.on_ack(slow, t0);
        assert_eq!(rc.params().quality, 65);
        // Down to the floor, then den, then fps.
        let mut den_changes = 0;
        for _ in 0..3 * 20 {
            den_changes += u32::from(rc.on_ack(slow, t0));
        }
        assert_eq!((rc.params().quality, rc.params().den, rc.fps()), (MIN_QUALITY, MAX_DEN, MIN_FPS));
        assert_eq!(den_changes, 3, "den 1 -> 4 forces three key frames");
        // Fast acks for 2 s: one step up at a time, never above the ceiling.
        let fast = Duration::from_millis(10);
        let mut t = t0;
        for _ in 0..400 {
            t += Duration::from_millis(100);
            rc.on_ack(fast, t);
        }
        assert_eq!(rc.params(), Params { den: 1, quality: 80, refine_quality: 92 });
        assert_eq!(rc.fps(), 30);
        // A medium ack (between 50 and 150 ms) neither steps nor counts.
        rc.on_ack(Duration::from_millis(100), t);
        assert_eq!(rc.params().quality, 80);
        // CONFIG lowers the ceiling.
        rc.set_ceiling(apply_config(usb, 1, 10));
        assert_eq!((rc.params().quality, rc.fps(), rc.params().refine_quality), (50, 10, 75));
        assert_eq!(apply_config(usb, 0, 0), usb);
        assert_eq!(apply_config(usb, 3, 60).fps, 30);
    }
}
