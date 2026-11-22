// Owner: T1 text geometry (round 4). The coverage rasterizer of the styled painter (FEATURE_TEXT 5.7): signed-area
// accumulation on closed polylines, the method `fontdue` uses; non-zero winding.
//
// Every edge is clipped to the rows of the bitmap and cut at every pixel row and column it crosses. Each piece lies
// inside one cell; it adds its exact trapezoid area to that cell and the rest of its signed height to the next cell,
// so that a running sum along the row gives the signed area covered up to each pixel. Coverage =
// `min(|sum|, 1)`: an area with winding 1 or more is full, opposite windings cancel.

/// A coverage bitmap: one byte per pixel, row 0 on top.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub width: u32,
    pub height: u32,
    /// `width * height` bytes, `round(255 * min(|winding area|, 1))`.
    pub data: Vec<u8>,
}

impl Coverage {
    /// An empty (all zero) bitmap of this size.
    pub fn new(width: u32, height: u32) -> Coverage {
        Coverage { width, height, data: vec![0; width as usize * height as usize] }
    }
}

/// The accumulation buffer: `(w + 2) * h` cells.
struct Accumulator {
    w: usize,
    h: usize,
    stride: usize,
    cells: Vec<f32>,
}

impl Accumulator {
    fn new(w: usize, h: usize) -> Accumulator {
        let stride = w + 2;
        Accumulator { w, h, stride, cells: vec![0.0; stride * h] }
    }

    /// One edge from `a` to `b` in pixel coordinates.
    fn edge(&mut self, a: [f32; 2], b: [f32; 2]) {
        if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) || a[1] == b[1] {
            return;
        }
        let h = self.h as f32;
        // Clip to the rows 0..h (the part above or below adds nothing to any pixel).
        let (top, bottom) = if a[1] < b[1] { (a, b) } else { (b, a) };
        if bottom[1] <= 0.0 || top[1] >= h {
            return;
        }
        let dir = if a[1] < b[1] { 1.0 } else { -1.0 };
        let dxdy = (bottom[0] - top[0]) / (bottom[1] - top[1]);
        let x_at = |y: f32| if y <= top[1] { top[0] } else if y >= bottom[1] { bottom[0] } else { top[0] + (y - top[1]) * dxdy };
        let y0 = top[1].max(0.0);
        let y1 = bottom[1].min(h);
        let mut row = y0.floor() as usize;
        let mut ya = y0;
        while ya < y1 && row < self.h {
            let yb = ((row + 1) as f32).min(y1);
            if yb > ya {
                self.row_piece(row, x_at(ya), x_at(yb), (yb - ya) * dir);
            }
            ya = yb;
            row += 1;
        }
    }

    /// The part of an edge inside one pixel row: from `xa` to `xb` (any order), signed height `dy`.
    fn row_piece(&mut self, row: usize, xa: f32, xb: f32, dy: f32) {
        let w = self.w as f32;
        let base = row * self.stride;
        let (lo, hi) = if xa <= xb { (xa, xb) } else { (xb, xa) };
        if !(lo.is_finite() && hi.is_finite()) {
            // Only an edge whose ends overflow f32 when subtracted gets here; it has no defined position.
            return;
        }
        if hi <= lo || (hi - lo) < 1e-6 {
            // Vertical inside the row.
            self.cell_piece(base, lo.clamp(0.0, w), dy);
            return;
        }
        // The height of each piece is proportional to its width. The part left of column 0 lands whole in cell 0 and
        // the part right of column w in cell w (both as one piece each), so the loop below only walks the columns of
        // the bitmap: its work does not grow with how far an edge reaches outside, and it always advances.
        let span = hi - lo;
        if lo < 0.0 {
            self.cell_piece(base, 0.0, dy * ((hi.min(0.0) - lo) / span));
        }
        if hi > w {
            self.cell_piece(base, w, dy * ((hi - lo.max(w)) / span));
        }
        // Cut at every pixel column between lo and hi inside the bitmap.
        let mut x = lo.max(0.0);
        let hi = hi.min(w);
        while x < hi {
            let next = (x.floor() + 1.0).min(hi);
            let share = (next - x) / span;
            let mid = 0.5 * (x + next);
            self.cell_piece(base, mid.clamp(0.0, w), dy * share);
            x = next;
        }
    }

    /// A piece of an edge inside one cell, represented by its middle `x` (clamped to `0..=w`) and signed height.
    fn cell_piece(&mut self, base: usize, x: f32, dy: f32) {
        let cell = (x.floor() as usize).min(self.w);
        let frac = x - cell as f32;
        // The pixel of the piece gets the area to the right of the edge, every pixel after it the full height.
        self.cells[base + cell] += dy * (1.0 - frac);
        self.cells[base + cell + 1] += dy * frac;
    }

    fn finish(self) -> Coverage {
        let mut out = Coverage::new(self.w as u32, self.h as u32);
        for (row, cells) in self.cells.chunks_exact(self.stride).enumerate() {
            let mut sum = 0.0f32;
            let dst = &mut out.data[row * self.w..(row + 1) * self.w];
            for (d, c) in dst.iter_mut().zip(cells) {
                sum += c;
                *d = (255.0 * sum.abs().min(1.0)).round() as u8;
            }
        }
        out
    }
}

/// Rasterizes the closed polylines `contours` into a `width x height` coverage bitmap. A contour point `p` lies at
/// pixel position `p - origin` (pixel `(x, y)` covers `x..x+1`, `y..y+1`). Edges outside the bitmap are clipped.
pub fn fill_contours(contours: &[Vec<[f32; 2]>], origin: [f32; 2], width: u32, height: u32) -> Coverage {
    if width == 0 || height == 0 {
        return Coverage::new(width, height);
    }
    let mut acc = Accumulator::new(width as usize, height as usize);
    for contour in contours {
        if contour.len() < 2 {
            continue;
        }
        let at = |p: [f32; 2]| [p[0] - origin[0], p[1] - origin[1]];
        let mut prev = at(contour[contour.len() - 1]);
        for &p in contour {
            let p = at(p);
            acc.edge(prev, p);
            prev = p;
        }
    }
    acc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
        (a1.min(b1) - a0.max(b0)).max(0.0)
    }

    /// TEXT 8.1 `a_square_has_exact_coverage`.
    #[test]
    fn a_square_has_exact_coverage() {
        for (x0, y0) in [(3.3, 2.7), (0.25, 0.75), (5.5, 5.5), (2.9, 3.01)] {
            let (x1, y1) = (x0 + 10.5, y0 + 10.5);
            for contour in [rect(x0, y0, x1, y1), rect(x0, y0, x1, y1).into_iter().rev().collect()] {
                let cov = fill_contours(&[contour], [0.0, 0.0], 20, 20);
                for y in 0..20 {
                    for x in 0..20 {
                        let area = overlap(x as f32, x as f32 + 1.0, x0, x1) * overlap(y as f32, y as f32 + 1.0, y0, y1);
                        let got = f32::from(cov.data[y * 20 + x]) / 255.0;
                        assert!((got - area).abs() <= 1.0 / 255.0 + 1e-6, "({x},{y}) {got} vs {area}");
                    }
                }
            }
        }
        // A slanted edge: a triangle covering half of a 4 x 4 square.
        let cov = fill_contours(&[vec![[0.0, 0.0], [4.0, 4.0], [0.0, 4.0]]], [0.0, 0.0], 4, 4);
        let total: u32 = cov.data.iter().map(|&v| u32::from(v)).sum();
        assert!((total as f32 / 255.0 - 8.0).abs() < 0.05, "{total}");
        assert_eq!(cov.data[0], 128); // the diagonal halves the corner pixel
        assert_eq!(cov.data[4], 255);
    }

    /// TEXT 8.1 `a_ring_has_an_empty_middle`.
    #[test]
    fn a_ring_has_an_empty_middle() {
        let outer = rect(1.0, 1.0, 19.0, 19.0);
        let inner: Vec<[f32; 2]> = rect(6.0, 6.0, 14.0, 14.0).into_iter().rev().collect();
        let cov = fill_contours(&[outer, inner], [0.0, 0.0], 20, 20);
        assert_eq!(cov.data[10 * 20 + 10], 0);
        assert_eq!(cov.data[3 * 20 + 10], 255);
        assert_eq!(cov.data[10 * 20 + 3], 255);
        assert_eq!(cov.data[0], 0);
    }

    /// TEXT 8.1 `overlapping_contours_fill_once`.
    #[test]
    fn overlapping_contours_fill_once() {
        let a = rect(1.5, 1.5, 12.0, 12.0);
        let b = rect(6.0, 6.0, 18.5, 18.5);
        let cov = fill_contours(&[a.clone(), b.clone()], [0.0, 0.0], 20, 20);
        assert_eq!(cov.data[8 * 20 + 8], 255);
        assert_eq!(cov.data[20 + 1], 64); // a quarter pixel
        // Where only one square covers a pixel, the result is that square's coverage; the overlap is full, once.
        let only_a = fill_contours(&[a], [0.0, 0.0], 20, 20);
        let only_b = fill_contours(&[b], [0.0, 0.0], 20, 20);
        for i in 0..400 {
            if only_a.data[i] == 0 {
                assert_eq!(cov.data[i], only_b.data[i]);
            } else if only_b.data[i] == 0 {
                assert_eq!(cov.data[i], only_a.data[i]);
            } else {
                assert!(cov.data[i] >= only_a.data[i].max(only_b.data[i]));
            }
        }
        // Two identical squares wound the same way: exactly one square.
        let twice = fill_contours(&[rect(2.25, 2.25, 9.75, 9.75), rect(2.25, 2.25, 9.75, 9.75)], [0.0, 0.0], 12, 12);
        let once = fill_contours(&[rect(2.25, 2.25, 9.75, 9.75)], [0.0, 0.0], 12, 12);
        assert_eq!(twice.data[5 * 12 + 5], 255);
        // At an edge the doubled winding saturates (|1.5| -> 1), as in `fontdue`.
        assert_eq!(twice.data[2 * 12 + 5], 255);
        assert_eq!(once.data[2 * 12 + 5], 191);
        assert!(twice.data.iter().zip(&once.data).all(|(t, o)| t >= o));
    }

    /// TEXT 8.1 `edges_outside_the_buffer_are_clipped`.
    #[test]
    fn edges_outside_the_buffer_are_clipped() {
        // A square much larger than the buffer, on every side.
        let cov = fill_contours(&[rect(-50.0, -30.0, 70.0, 90.0)], [0.0, 0.0], 10, 10);
        assert!(cov.data.iter().all(|&v| v == 255));
        // Only partly inside: left and top edge outside, right edge at 4.5, bottom edge at 6.25.
        let cov = fill_contours(&[rect(-7.0, -3.0, 4.5, 6.25)], [0.0, 0.0], 10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let area = overlap(x as f32, x as f32 + 1.0, -7.0, 4.5) * overlap(y as f32, y as f32 + 1.0, -3.0, 6.25);
                let got = f32::from(cov.data[y * 10 + x]) / 255.0;
                assert!((got - area).abs() <= 1.0 / 255.0 + 1e-6, "({x},{y})");
            }
        }
        // A triangle crossing the left border diagonally.
        let tri = vec![[-4.0, 0.0], [4.0, 8.0], [-4.0, 8.0]];
        let cov = fill_contours(&[tri], [0.0, 0.0], 8, 8);
        let total: u32 = cov.data.iter().map(|&v| u32::from(v)).sum();
        // Inside x >= 0: the triangle (0,4)-(4,8)-(0,8): area 8.
        assert!((total as f32 / 255.0 - 8.0).abs() < 0.05, "{total}");
        // Entirely outside: nothing.
        let cov = fill_contours(&[rect(20.0, 2.0, 30.0, 5.0), rect(2.0, -9.0, 5.0, -1.0)], [0.0, 0.0], 10, 10);
        assert!(cov.data.iter().all(|&v| v == 0));
        // The origin moves the picture.
        let cov = fill_contours(&[rect(100.0, 200.0, 101.0, 201.0)], [99.0, 199.0], 3, 3);
        assert_eq!(cov.data[4], 255);
        assert_eq!(cov.data.iter().filter(|&&v| v != 0).count(), 1);
        // An edge reaching very far outside (review of T1): no endless loop, the same coverage as a near one.
        let far = fill_contours(&[vec![[-2e30, 0.0], [5.0, 0.0], [5.0, 4.0], [-1e30, 4.0]]], [0.0, 0.0], 8, 4);
        let near = fill_contours(&[rect(-3.0, 0.0, 5.0, 4.0)], [0.0, 0.0], 8, 4);
        assert_eq!(far, near);
        assert!((0..4).all(|y| (0..8).all(|x| far.data[y * 8 + x] == if x < 5 { 255 } else { 0 })));
        // A long slanted edge right of the bitmap adds nothing visible.
        let right = fill_contours(&[vec![[1.0, 0.0], [3e30, 0.0], [1e30, 4.0], [1.0, 4.0]]], [0.0, 0.0], 8, 4);
        assert!((0..4).all(|y| (0..8).all(|x| right.data[y * 8 + x] == if x >= 1 { 255 } else { 0 })));
        // Degenerate input.
        assert_eq!(fill_contours(&[vec![[f32::NAN, 0.0], [1.0, 1.0], [0.0, 1.0]]], [0.0, 0.0], 0, 4).data.len(), 0);
    }
}
