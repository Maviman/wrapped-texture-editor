//! Pixel containers (ARCHITECTURE.md 4.2, 5.5): dense `PixelBuf`, cropped `CroppedBuf`, the borrowed `PixelView` and
//! the one-channel `AlphaBuf` (stickers) and the `TileBuf` trait of undo tile patches (sticker masks, P3b).
//! All pixel data is RGBA8, straight alpha, sRGB-encoded, tightly packed, row 0 = top.

use crate::math::IRect;

/// Dense RGBA8 image, straight alpha, sRGB-encoded, row 0 = top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelBuf {
    pub width: u32,
    pub height: u32,
    /// `data.len() == width * height * 4`.
    pub data: Vec<u8>,
}

impl PixelBuf {
    /// Transparent black image.
    pub fn new(width: u32, height: u32) -> PixelBuf {
        PixelBuf { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    /// Image filled with one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> PixelBuf {
        if rgba == [0; 4] {
            return PixelBuf::new(width, height);
        }
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            data.extend_from_slice(&rgba);
        }
        PixelBuf { width, height, data }
    }

    /// Wraps existing bytes; `None` unless `data.len() == width * height * 4`.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Option<PixelBuf> {
        if data.len() == width as usize * height as usize * 4 {
            Some(PixelBuf { width, height, data })
        } else {
            None
        }
    }

    #[inline]
    fn offset(&self, x: u32, y: u32) -> usize {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) outside {}x{}", self.width, self.height);
        (y as usize * self.width as usize + x as usize) * 4
    }

    /// Pixel at `(x, y)`; panics outside the image.
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.offset(x, y);
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// Writes the pixel at `(x, y)`; panics outside the image.
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let i = self.offset(x, y);
        self.data[i..i + 4].copy_from_slice(&rgba);
    }

    /// The rectangle `0,0 .. width,height`.
    pub fn rect(&self) -> IRect {
        IRect::from_size(self.width, self.height)
    }

    /// The bytes of row `y` (`width * 4` bytes); panics outside the image.
    #[inline]
    pub fn row(&self, y: u32) -> &[u8] {
        let stride = self.width as usize * 4;
        let start = y as usize * stride;
        &self.data[start..start + stride]
    }

    /// The mutable bytes of row `y` (`width * 4` bytes); panics outside the image.
    #[inline]
    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        let stride = self.width as usize * 4;
        let start = y as usize * stride;
        &mut self.data[start..start + stride]
    }

    /// Borrowed view of the whole image.
    pub fn view(&self) -> PixelView<'_> {
        PixelView { rect: self.rect(), data: &self.data }
    }

    /// Size of the pixel data in bytes.
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }
}

/// RGBA8 pixels covering only `rect` of some canvas; transparent everywhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CroppedBuf {
    pub rect: IRect,
    /// `data.len() == rect.area() * 4`.
    pub data: Vec<u8>,
}

impl CroppedBuf {
    /// Buffer that covers nothing.
    pub fn empty() -> CroppedBuf {
        CroppedBuf { rect: IRect::EMPTY, data: Vec::new() }
    }

    /// True when the buffer covers no pixel.
    pub fn is_empty(&self) -> bool {
        self.rect.is_empty()
    }

    /// Transparent buffer covering `rect` (an empty `rect` gives `CroppedBuf::empty()`).
    pub fn new(rect: IRect) -> CroppedBuf {
        if rect.is_empty() {
            return CroppedBuf::empty();
        }
        CroppedBuf { rect, data: vec![0; rect.area() as usize * 4] }
    }

    /// Pixel at canvas position `(x, y)`; `[0; 4]` outside `rect`.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        self.view().get(x, y)
    }

    /// Borrowed view of the covered rectangle.
    pub fn view(&self) -> PixelView<'_> {
        PixelView { rect: self.rect, data: &self.data }
    }

    /// Size of the pixel data in bytes.
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }

    /// Dense `width x height` canvas with this buffer's pixels at `rect` (clipped to the canvas), transparent elsewhere.
    pub fn to_dense(&self, width: u32, height: u32) -> PixelBuf {
        let mut out = PixelBuf::new(width, height);
        self.blit_into(&mut out);
        out
    }

    /// Copies these pixels into `out` at `rect` (clipped to `out`); pixels of `out` outside `rect` are kept.
    pub fn blit_into(&self, out: &mut PixelBuf) {
        let clip = self.rect.intersect(&out.rect());
        if clip.is_empty() {
            return;
        }
        let src_stride = self.rect.width() as usize * 4;
        let dst_stride = out.width as usize * 4;
        let n = clip.width() as usize * 4;
        for y in clip.y0..clip.y1 {
            let s = (y - self.rect.y0) as usize * src_stride + (clip.x0 - self.rect.x0) as usize * 4;
            let d = y as usize * dst_stride + clip.x0 as usize * 4;
            out.data[d..d + n].copy_from_slice(&self.data[s..s + n]);
        }
    }
}

/// RGBA8 pixels made of disjoint cropped parts; transparent everywhere else. The storage of stamp and text layer
/// caches (6.5): a decal that reaches UV islands spread over a packed atlas keeps one small part per cluster of
/// touched 64 x 64 tiles instead of one dense rectangle over all of them.
///
/// Parts are pairwise disjoint, never empty, cropped to their non-zero pixels and ordered by `(y0, x0)`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SparseBuf {
    parts: Vec<CroppedBuf>,
    bounds: IRect,
}

impl SparseBuf {
    /// Tile side used to split a dense buffer into parts.
    pub const TILE: i32 = 64;

    /// Buffer that covers nothing.
    pub fn empty() -> SparseBuf {
        SparseBuf::default()
    }

    /// Wraps parts that are already disjoint; empty parts are dropped, the rest ordered by `(y0, x0)`.
    pub fn from_parts(parts: Vec<CroppedBuf>) -> SparseBuf {
        let mut parts: Vec<CroppedBuf> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        parts.sort_by_key(|p| (p.rect.y0, p.rect.x0));
        debug_assert!(
            parts.iter().enumerate().all(|(i, a)| parts[i + 1..].iter().all(|b| a.rect.intersect(&b.rect).is_empty())),
            "SparseBuf parts overlap"
        );
        let bounds = parts.iter().fold(IRect::EMPTY, |acc, p| acc.union(&p.rect));
        SparseBuf { parts, bounds }
    }

    /// Splits a cropped buffer into parts (see `from_dense`).
    pub fn from_cropped(buf: CroppedBuf) -> SparseBuf {
        SparseBuf::from_dense(buf.rect, buf.data)
    }

    /// Splits the dense pixels `data` covering `rect` (stride `rect.width() * 4`) into parts: the `TILE` x `TILE`
    /// tiles that hold a non-zero byte are grouped into rectangles (runs of tiles per tile row, a run joining the
    /// rectangle above it when that wastes at most a quarter of the joined area), and each rectangle is cropped to its
    /// non-zero pixels. All-zero buffers give `empty()`.
    pub fn from_dense(rect: IRect, data: Vec<u8>) -> SparseBuf {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        if w == 0 || h == 0 || data.len() < w * h * 4 {
            return SparseBuf::empty();
        }
        let t = Self::TILE as usize;
        let (tw, th) = (w.div_ceil(t), h.div_ceil(t));
        let stride = w * 4;
        // Occupied tiles, one tile row per task.
        let occupied: Vec<Vec<bool>> = {
            use rayon::prelude::*;
            (0..th)
                .into_par_iter()
                .map(|ty| {
                    let mut row = vec![false; tw];
                    for y in ty * t..((ty + 1) * t).min(h) {
                        let line = &data[y * stride..(y + 1) * stride];
                        for (tx, hit) in row.iter_mut().enumerate() {
                            if !*hit {
                                let (a, b) = (tx * t * 4, ((tx + 1) * t).min(w) * 4);
                                *hit = line[a..b].iter().any(|&v| v != 0);
                            }
                        }
                    }
                    row
                })
                .collect()
        };
        // Rectangles in tile units: (x0, x1, y0, y1).
        let mut rects: Vec<(usize, usize, usize, usize)> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for (ty, row) in occupied.iter().enumerate() {
            let mut next_open = Vec::new();
            let mut tx = 0;
            while tx < tw {
                if !row[tx] {
                    tx += 1;
                    continue;
                }
                let a = tx;
                while tx < tw && row[tx] {
                    tx += 1;
                }
                let b = tx;
                // Join the open rectangle above that overlaps most, if the join wastes little.
                let joined = open
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| {
                        let r = rects[**i];
                        let overlap = r.1.min(b) as isize - r.0.max(a) as isize;
                        let merged = (r.1.max(b) - r.0.min(a)) * (r.3 + 1 - r.2);
                        let own = (r.1 - r.0) * (r.3 - r.2) + (b - a);
                        overlap > 0 && merged * 4 <= own * 5
                    })
                    .max_by_key(|(_, i)| rects[**i].1.min(b) - rects[**i].0.max(a))
                    .map(|(k, _)| k);
                match joined {
                    Some(k) => {
                        let i = open.swap_remove(k);
                        let r = &mut rects[i];
                        *r = (r.0.min(a), r.1.max(b), r.2, ty + 1);
                        next_open.push(i);
                    }
                    None => {
                        rects.push((a, b, ty, ty + 1));
                        next_open.push(rects.len() - 1);
                    }
                }
            }
            open = next_open;
        }
        // A wider join can make two rectangles of the same rows overlap: merge those (rare).
        let mut merged = true;
        while merged {
            merged = false;
            'outer: for i in 0..rects.len() {
                for j in i + 1..rects.len() {
                    let (a, b) = (rects[i], rects[j]);
                    if a.0 < b.1 && b.0 < a.1 && a.2 < b.3 && b.2 < a.3 {
                        rects[i] = (a.0.min(b.0), a.1.max(b.1), a.2.min(b.2), a.3.max(b.3));
                        rects.swap_remove(j);
                        merged = true;
                        break 'outer;
                    }
                }
            }
        }
        if rects.len() == 1 && rects[0] == (0, tw, 0, th) {
            // One rectangle over everything: crop the buffer itself.
            return SparseBuf::from_parts(vec![crop_nonzero(rect, &data, 0, w, 0, h)]);
        }
        let parts = {
            use rayon::prelude::*;
            rects
                .par_iter()
                .map(|&(x0, x1, y0, y1)| crop_nonzero(rect, &data, x0 * t, (x1 * t).min(w), y0 * t, (y1 * t).min(h)))
                .collect()
        };
        SparseBuf::from_parts(parts)
    }

    /// True when the buffer covers no pixel.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Bounding box of all parts (`IRect::EMPTY` when empty).
    pub fn bounds(&self) -> IRect {
        self.bounds
    }

    /// The parts, ordered by `(y0, x0)`.
    pub fn parts(&self) -> &[CroppedBuf] {
        &self.parts
    }

    /// The rectangles of the parts.
    pub fn rects(&self) -> impl Iterator<Item = IRect> + '_ {
        self.parts.iter().map(|p| p.rect)
    }

    /// Size of the pixel data in bytes.
    pub fn byte_len(&self) -> usize {
        self.parts.iter().map(CroppedBuf::byte_len).sum()
    }

    /// Pixel at canvas position `(x, y)`; `[0; 4]` outside every part.
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        if !self.bounds.contains(x, y) {
            return [0; 4];
        }
        for part in &self.parts {
            if part.rect.y0 > y {
                break;
            }
            if part.rect.contains(x, y) {
                return part.get(x, y);
            }
        }
        [0; 4]
    }

    /// Dense `width x height` canvas with these pixels (clipped to the canvas), transparent elsewhere.
    pub fn to_dense(&self, width: u32, height: u32) -> PixelBuf {
        let mut out = PixelBuf::new(width, height);
        for part in &self.parts {
            part.blit_into(&mut out);
        }
        out
    }

    /// One cropped buffer over `bounds()` with these pixels (for writers that store a single rectangle).
    pub fn to_cropped(&self) -> CroppedBuf {
        if let [only] = &self.parts[..] {
            return only.clone();
        }
        let mut out = CroppedBuf::new(self.bounds);
        let stride = self.bounds.width() as usize * 4;
        for part in &self.parts {
            let n = part.rect.width() as usize * 4;
            for y in part.rect.y0..part.rect.y1 {
                let Some(src) = part.view().row(y) else { continue };
                let d = (y - self.bounds.y0) as usize * stride + (part.rect.x0 - self.bounds.x0) as usize * 4;
                out.data[d..d + n].copy_from_slice(src);
            }
        }
        out
    }
}

/// The part of the dense `data` (covering `rect`) at columns `x0..x1`, rows `y0..y1` (buffer coordinates), cropped to
/// its non-zero pixels.
fn crop_nonzero(rect: IRect, data: &[u8], x0: usize, x1: usize, y0: usize, y1: usize) -> CroppedBuf {
    let stride = rect.width() as usize * 4;
    let nonzero = |y: usize, a: usize, b: usize| data[y * stride + a * 4..y * stride + b * 4].iter().any(|&v| v != 0);
    let Some(top) = (y0..y1).find(|&y| nonzero(y, x0, x1)) else { return CroppedBuf::empty() };
    let bottom = (top..y1).rev().find(|&y| nonzero(y, x0, x1)).unwrap_or(top) + 1;
    let (mut left, mut right) = (x1, x0);
    for y in top..bottom {
        let line = &data[y * stride..(y + 1) * stride];
        let px = |x: usize| line[x * 4..x * 4 + 4] != [0, 0, 0, 0];
        if let Some(l) = (x0..left).find(|&x| px(x)) {
            left = l;
        }
        if let Some(r) = (right.max(left)..x1).rev().find(|&x| px(x)) {
            right = r + 1;
        }
    }
    if left >= right {
        return CroppedBuf::empty();
    }
    let n = (right - left) * 4;
    let mut out = Vec::with_capacity(n * (bottom - top));
    for y in top..bottom {
        out.extend_from_slice(&data[y * stride + left * 4..y * stride + left * 4 + n]);
    }
    let r = IRect::new(rect.x0 + left as i32, rect.y0 + top as i32, rect.x0 + right as i32, rect.y0 + bottom as i32);
    CroppedBuf { rect: r, data: out }
}

/// Borrowed pixels covering `rect` of a canvas; stride = `rect.width() * 4`.
#[derive(Copy, Clone, Debug)]
pub struct PixelView<'a> {
    pub rect: IRect,
    pub data: &'a [u8],
}

impl<'a> PixelView<'a> {
    /// The bytes of canvas row `y` inside `rect` (`rect.width() * 4` bytes); `None` when `y` is outside `rect`.
    #[inline]
    pub fn row(&self, y: i32) -> Option<&'a [u8]> {
        if self.rect.is_empty() || y < self.rect.y0 || y >= self.rect.y1 {
            return None;
        }
        let stride = self.rect.width() as usize * 4;
        let start = (y - self.rect.y0) as usize * stride;
        self.data.get(start..start + stride)
    }

    /// Pixel at canvas position `(x, y)`; `[0; 4]` outside `rect`.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        if !self.rect.contains(x, y) {
            return [0; 4];
        }
        let stride = self.rect.width() as usize * 4;
        let i = (y - self.rect.y0) as usize * stride + (x - self.rect.x0) as usize * 4;
        match self.data.get(i..i + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0; 4],
        }
    }
}

/// Dense 8-bit single-channel image (masks, selections, coverage), row 0 = top; 255 = fully on (STICKER_PLAN 2.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlphaBuf {
    pub width: u32,
    pub height: u32,
    /// `data.len() == width * height`.
    pub data: Vec<u8>,
}

impl AlphaBuf {
    /// All-zero image.
    pub fn new(width: u32, height: u32) -> AlphaBuf {
        AlphaBuf { width, height, data: vec![0; width as usize * height as usize] }
    }

    /// Image filled with one value.
    pub fn filled(width: u32, height: u32, value: u8) -> AlphaBuf {
        AlphaBuf { width, height, data: vec![value; width as usize * height as usize] }
    }

    /// Wraps existing bytes; `None` unless `data.len() == width * height`.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Option<AlphaBuf> {
        (data.len() == width as usize * height as usize).then_some(AlphaBuf { width, height, data })
    }

    /// The alpha channel of an RGBA image.
    pub fn from_alpha(buf: &PixelBuf) -> AlphaBuf {
        AlphaBuf { width: buf.width, height: buf.height, data: buf.data.iter().skip(3).step_by(4).copied().collect() }
    }

    /// Value at `(x, y)`; panics outside the image.
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> u8 {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) outside {}x{}", self.width, self.height);
        self.data[y as usize * self.width as usize + x as usize]
    }

    /// Writes the value at `(x, y)`; panics outside the image.
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, value: u8) {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) outside {}x{}", self.width, self.height);
        self.data[y as usize * self.width as usize + x as usize] = value;
    }

    /// The rectangle `0,0 .. width,height`.
    pub fn rect(&self) -> IRect {
        IRect::from_size(self.width, self.height)
    }

    /// Size of the data in bytes.
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }
}

/// A dense image that undo tile patches and tile backups work on (STICKER_PLAN 2.10, review A4): `BPP` bytes per
/// pixel, rows of `width * BPP` bytes, row 0 = top. `PixelBuf` (4: RGBA paint layers and sticker pictures) and
/// `AlphaBuf` (1: sticker layer masks).
pub trait TileBuf {
    /// Bytes per pixel.
    const BPP: usize;
    /// `[width, height]`.
    fn size(&self) -> [u32; 2];
    /// All bytes, `width * height * BPP`.
    fn bytes(&self) -> &[u8];
    /// All bytes, mutable.
    fn bytes_mut(&mut self) -> &mut [u8];
}

impl TileBuf for PixelBuf {
    const BPP: usize = 4;
    #[inline]
    fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }
    #[inline]
    fn bytes(&self) -> &[u8] {
        &self.data
    }
    #[inline]
    fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

impl TileBuf for AlphaBuf {
    const BPP: usize = 1;
    #[inline]
    fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }
    #[inline]
    fn bytes(&self) -> &[u8] {
        &self.data
    }
    #[inline]
    fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

/// A shared buffer reads as the buffer; writing goes through `Arc::make_mut` (copy-on-write when shared), so the
/// read-only uses (`touch_rect`, `into_patch`, `diff`) take a layer's `Arc` as they took `&PixelBuf` by deref before.
impl<T: TileBuf + Clone> TileBuf for std::sync::Arc<T> {
    const BPP: usize = T::BPP;
    #[inline]
    fn size(&self) -> [u32; 2] {
        (**self).size()
    }
    #[inline]
    fn bytes(&self) -> &[u8] {
        (**self).bytes()
    }
    #[inline]
    fn bytes_mut(&mut self) -> &mut [u8] {
        std::sync::Arc::make_mut(self).bytes_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k1_alpha_buf_basics() {
        let mut a = AlphaBuf::new(3, 2);
        assert_eq!(a.byte_len(), 6);
        a.set(2, 1, 77);
        assert_eq!(a.get(2, 1), 77);
        assert_eq!(a.rect(), IRect::new(0, 0, 3, 2));
        assert!(AlphaBuf::from_raw(3, 2, vec![0; 5]).is_none());
        assert_eq!(AlphaBuf::filled(2, 2, 9).data, vec![9; 4]);
        let rgba = PixelBuf::from_raw(2, 1, vec![1, 2, 3, 40, 5, 6, 7, 80]).unwrap();
        assert_eq!(AlphaBuf::from_alpha(&rgba).data, vec![40, 80]);
    }

    #[test]
    fn pixel_buf_accessors() {
        let mut b = PixelBuf::new(3, 2);
        assert_eq!(b.byte_len(), 24);
        assert!(b.data.iter().all(|&v| v == 0));
        b.set(2, 1, [1, 2, 3, 4]);
        assert_eq!(b.get(2, 1), [1, 2, 3, 4]);
        assert_eq!(b.get(1, 1), [0; 4]);
        assert_eq!(b.row(1), &[0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4]);
        b.row_mut(0)[4..8].copy_from_slice(&[9, 8, 7, 6]);
        assert_eq!(b.get(1, 0), [9, 8, 7, 6]);
        assert_eq!(b.rect(), IRect::new(0, 0, 3, 2));
        assert_eq!(b.view().get(2, 1), [1, 2, 3, 4]);
        assert_eq!(b.view().get(3, 1), [0; 4]);
        assert_eq!(b.view().get(-1, 0), [0; 4]);

        let f = PixelBuf::filled(2, 2, [5, 6, 7, 8]);
        assert_eq!(f.data, [5, 6, 7, 8].repeat(4));
        assert_eq!(PixelBuf::filled(2, 2, [0; 4]), PixelBuf::new(2, 2));

        assert!(PixelBuf::from_raw(2, 2, vec![0; 16]).is_some());
        assert!(PixelBuf::from_raw(2, 2, vec![0; 15]).is_none());
        assert_eq!(PixelBuf::new(0, 5).byte_len(), 0);
    }

    #[test]
    fn cropped_buf_and_view() {
        let e = CroppedBuf::empty();
        assert!(e.is_empty());
        assert_eq!(e.get(0, 0), [0; 4]);
        assert_eq!(e.view().row(0), None);
        assert_eq!(e.to_dense(2, 2), PixelBuf::new(2, 2));
        assert!(CroppedBuf::new(IRect::new(4, 4, 2, 9)).is_empty());

        let mut c = CroppedBuf::new(IRect::new(1, 2, 3, 4));
        assert_eq!(c.byte_len(), 2 * 2 * 4);
        c.data.copy_from_slice(&[1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4]);
        assert_eq!(c.get(1, 2), [1; 4]);
        assert_eq!(c.get(2, 2), [2; 4]);
        assert_eq!(c.get(1, 3), [3; 4]);
        assert_eq!(c.get(2, 3), [4; 4]);
        assert_eq!(c.get(0, 2), [0; 4]);
        assert_eq!(c.get(3, 3), [0; 4]);
        assert_eq!(c.view().row(3), Some(&[3u8, 3, 3, 3, 4, 4, 4, 4][..]));
        assert_eq!(c.view().row(4), None);

        let d = c.to_dense(4, 4);
        assert_eq!(d.get(1, 2), [1; 4]);
        assert_eq!(d.get(2, 3), [4; 4]);
        assert_eq!(d.get(0, 0), [0; 4]);
        assert_eq!(d.data.iter().filter(|&&v| v != 0).count(), 16);

        // A rect reaching outside the canvas is clipped.
        let clipped = c.to_dense(2, 3);
        assert_eq!(clipped.get(1, 2), [1; 4]);
        assert_eq!(clipped.data.iter().filter(|&&v| v != 0).count(), 4);
    }

    /// Finding SP-2 (review round 2): content spread over a large rectangle is stored as small parts.
    #[test]
    fn sparse_buf_keeps_only_the_touched_tiles() {
        // A 1000 x 800 canvas region with three small blobs far apart (and RGB under alpha 0 in one of them).
        let rect = IRect::new(100, 50, 1100, 850);
        let mut dense = CroppedBuf::new(rect);
        let mut put = |x: i32, y: i32, p: [u8; 4]| {
            let i = ((y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize) * 4;
            dense.data[i..i + 4].copy_from_slice(&p);
        };
        for (cx, cy) in [(110, 60), (700, 400), (1090, 840)] {
            for y in cy - 5..cy + 5 {
                for x in cx - 8..cx + 8 {
                    if rect.contains(x, y) {
                        put(x, y, [x as u8, y as u8, 7, if x == cx { 0 } else { 200 }]);
                    }
                }
            }
        }
        let sparse = SparseBuf::from_cropped(dense.clone());
        assert_eq!(sparse.parts().len(), 3, "{:?}", sparse.rects().collect::<Vec<_>>());
        assert!(sparse.byte_len() < 3 * 16 * 10 * 4 + 1, "{}", sparse.byte_len());
        assert!(sparse.byte_len() * 100 < dense.byte_len());
        // Same pixels everywhere, RGB under alpha 0 included.
        for y in rect.y0 - 2..rect.y1 + 2 {
            for x in rect.x0 - 2..rect.x1 + 2 {
                assert_eq!(sparse.get(x, y), dense.get(x, y), "({x}, {y})");
            }
        }
        assert_eq!(sparse.to_dense(1200, 900), dense.to_dense(1200, 900));
        let joined = sparse.to_cropped();
        assert_eq!(joined.rect, sparse.bounds());
        assert_eq!(joined.to_dense(1200, 900), dense.to_dense(1200, 900));
        // Parts are disjoint, ordered, cropped to their content.
        let rects: Vec<IRect> = sparse.rects().collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(rects[i + 1..].iter().all(|b| a.intersect(b).is_empty()));
        }
        assert!(rects.windows(2).all(|w| (w[0].y0, w[0].x0) <= (w[1].y0, w[1].x0)));
        assert_eq!(rects[0], IRect::new(102, 55, 118, 65));

        // One solid blob over everything stays one part; nothing at all is empty.
        let mut full = CroppedBuf::new(IRect::new(0, 0, 200, 150));
        full.data.fill(9);
        let one = SparseBuf::from_cropped(full.clone());
        assert_eq!(one.parts(), &[full]);
        assert!(SparseBuf::from_cropped(CroppedBuf::new(IRect::new(0, 0, 300, 300))).is_empty());
        assert!(SparseBuf::empty().is_empty() && SparseBuf::empty().to_cropped().is_empty());
        assert_eq!(SparseBuf::empty().bounds(), IRect::EMPTY);
    }
}
