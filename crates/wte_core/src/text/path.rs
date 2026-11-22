// Owner: T1 text geometry (round 4). Placing glyphs on a line: straight, Arc, Circle, Curve (FEATURE_TEXT 5.5). All
// lengths in raster pixels of text space (origin at the anchor, x to the right, y down).
//
// Every text line gets its own walk: a function from the arc length `s` (0 = the start of that text line on the
// line, alignment and Start already applied) to a point and a unit tangent. Glyphs are rigid: `place` puts the
// middle of a glyph on the walk and turns the glyph with the tangent. `new_circle`, `fit_circle` and `new_curve` are
// the creation formulas of 5.5.

use std::f32::consts::PI;
use std::sync::Arc;

use crate::text::{PathShape, TextAlign, TextFrame, TextPath};

/// Bends (and Arc sizes) below this are laid out straight.
const MIN_ARC_BEND: f32 = 0.001;
/// Parameter steps per Bezier piece in the length table of a Curve.
const CURVE_STEPS: usize = 32;
/// Two neighbouring curve points closer than this (px) count as this far apart for the knot spacing.
const MIN_KNOT_PX: f32 = 0.001;

/// One text line as the path layout needs it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TextLine {
    /// Unrounded baseline of the line in text space (`b0 + l * line advance`).
    pub baseline: f32,
    /// Width of the line (pen after the last glyph minus the spacing after it).
    pub width: f32,
}

/// A point of a line and the unit tangent there.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinePoint {
    pub point: [f32; 2],
    pub tangent: [f32; 2],
}

/// Which half of a circle the reference point is on.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Side {
    /// `P = C + r (sin phi, -cos phi)`, `T = (cos phi, sin phi)`: the text stands outside, on top, clockwise.
    Top,
    /// `P = C + r (sin phi, cos phi)`, `T = (cos phi, -sin phi)`: the text stands inside, at the bottom.
    Bottom,
}

/// The walk of one text line.
#[derive(Clone, Debug, PartialEq)]
enum Walk {
    /// A straight line from `origin` along `dir`.
    Straight { origin: [f32; 2], dir: [f32; 2] },
    /// A circle. `s0` = arc length of the line start from the reference point; `flip` negates the angle and
    /// reverses the tangent (Arc "Other side"); `span`: half the arc length of an open Arc, beyond which the walk
    /// continues straight (`None` for a full Circle).
    Circle { center: [f32; 2], radius: f32, side: Side, s0: f32, flip: bool, span: Option<f32> },
    /// A sampled curve; `s0` = start of the line on the curve; `flip` walks it backwards.
    Curve { table: Arc<CurveTable>, s0: f32, flip: bool },
}

/// A line laid out for one text: one walk per text line, with the start of each line on it.
#[derive(Clone, Debug, PartialEq)]
pub struct PathLayout {
    lines: Vec<TextLine>,
    walks: Vec<Walk>,
    /// Circle only, else 0: the largest share of its own circle that a text line covers (`TextFrame::circle_fill`).
    pub circle_fill: f32,
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: [f32; 2], k: f32) -> [f32; 2] {
    [a[0] * k, a[1] * k]
}

fn length(a: [f32; 2]) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

fn normalize(a: [f32; 2], fallback: [f32; 2]) -> [f32; 2] {
    let l = length(a);
    if l > 1e-12 && l.is_finite() { scale(a, 1.0 / l) } else { fallback }
}

impl PathLayout {
    /// The layout of `path` for text lines `lines` at `px` raster pixels per em; `align` decides where each line
    /// starts on the line (5.5); `start` is applied (clamped to one line length on open lines).
    pub fn new(path: &TextPath, px: f32, align: TextAlign, lines: &[TextLine]) -> PathLayout {
        let px = if px.is_finite() && px > 0.0 { px } else { 1.0 };
        let lines: Vec<TextLine> = lines
            .iter()
            .map(|l| TextLine { baseline: finite_or(l.baseline, 0.0), width: finite_or(l.width, 0.0).max(0.0) })
            .collect();
        let b0 = lines.first().map_or(0.0, |l| l.baseline);
        let widest = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
        let start = finite_or(path.start, 0.0) * px;
        let quarter_em = px / 4.0;
        let mut circle_fill = 0.0f32;
        let walks = match &path.shape {
            PathShape::Arc { bend } => {
                let bend = finite_or(*bend, 0.0).clamp(-1.0, 1.0);
                let theta = bend.abs() * 2.0 * PI;
                let shift = start.clamp(-widest, widest);
                if bend.abs() < MIN_ARC_BEND || widest <= 0.0 {
                    lines.iter().map(|l| straight_walk(l, b0, widest, align, shift, path.flip)).collect()
                } else {
                    let r = widest / theta;
                    let (center, side) = if bend > 0.0 { ([0.0, b0 + r], Side::Top) } else { ([0.0, b0 - r], Side::Bottom) };
                    lines
                        .iter()
                        .map(|l| {
                            // With flip the letters' "down" points away from the centre for bend > 0 (towards it
                            // for bend < 0), so the following lines go the other way, as on a flipped Circle.
                            let dy = if path.flip { -(l.baseline - b0) } else { l.baseline - b0 };
                            let radius = if bend > 0.0 { r - dy } else { r + dy }.max(quarter_em);
                            let half = theta / 2.0 * radius;
                            let s0 = align_start(align, half, l.width) + shift;
                            Walk::Circle { center, radius, side, s0, flip: path.flip, span: Some(half) }
                        })
                        .collect()
                }
            }
            PathShape::Circle { diameter, center } => {
                let r = (finite_or(*diameter, 1.0) * px / 2.0).max(1e-3);
                let center = [finite_or(center[0], 0.0) * px, finite_or(center[1], 0.0) * px];
                let angle = start / r;
                let side = if path.flip { Side::Bottom } else { Side::Top };
                lines
                    .iter()
                    .map(|l| {
                        let dy = l.baseline - b0;
                        let radius = if path.flip { r + dy } else { r - dy }.max(quarter_em);
                        circle_fill = circle_fill.max(l.width / (2.0 * PI * radius));
                        let s0 = align_start(align, PI / 2.0 * radius, l.width) + angle * radius;
                        Walk::Circle { center, radius, side, s0, flip: false, span: None }
                    })
                    .collect()
            }
            PathShape::Curve { points } => {
                let pts: Vec<[f32; 2]> = points
                    .iter()
                    .map(|p| [finite_or(p[0], 0.0) * px, finite_or(p[1], 0.0) * px])
                    .collect();
                let base = Arc::new(CurveTable::catmull_rom(&pts));
                // Finite but huge points can overflow the lengths; such a walk has no usable length.
                let total = if base.length().is_finite() { base.length() } else { 0.0 };
                let shift = start.clamp(-total, total);
                lines
                    .iter()
                    .map(|l| {
                        // A flipped walk has the reversed tangent, so its "down" is the curve's "up".
                        let dy = if path.flip { -(l.baseline - b0) } else { l.baseline - b0 };
                        let table = if dy.abs() > 1e-6 { Arc::new(base.offset(dy)) } else { base.clone() };
                        let len = table.length();
                        let s0 = match align {
                            TextAlign::Left => 0.0,
                            TextAlign::Center => (len - l.width) / 2.0,
                            TextAlign::Right => len - l.width,
                        } + shift;
                        Walk::Curve { table, s0, flip: path.flip }
                    })
                    .collect()
            }
        };
        PathLayout { lines, walks, circle_fill }
    }

    /// Number of text lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Point and tangent of the walk of text line `line` at arc length `s` (from the start of that text line on the
    /// path; beyond the ends of an open line it continues straight along the end tangent).
    pub fn line_point(&self, line: usize, s: f32) -> LinePoint {
        let Some(walk) = self.walks.get(line) else {
            let y = self.lines.last().map_or(0.0, |l| l.baseline);
            return LinePoint { point: [s, y], tangent: [1.0, 0.0] };
        };
        match walk {
            Walk::Straight { origin, dir } => LinePoint { point: add(*origin, scale(*dir, s)), tangent: *dir },
            Walk::Circle { center, radius, side, s0, flip, span } => {
                let sp = s0 + s;
                let (sp, beyond) = match span {
                    Some(half) if sp > *half => (*half, sp - half),
                    Some(half) if sp < -*half => (-*half, sp + half),
                    _ => (sp, 0.0),
                };
                let mut phi = sp / radius;
                if *flip {
                    phi = -phi;
                }
                let (sin, cos) = phi.sin_cos();
                let (offset, mut tangent) = match side {
                    Side::Top => ([sin, -cos], [cos, sin]),
                    Side::Bottom => ([sin, cos], [cos, -sin]),
                };
                if *flip {
                    tangent = scale(tangent, -1.0);
                }
                let mut point = add(*center, scale(offset, *radius));
                // Beyond an open end: straight along the end tangent (in reading direction).
                point = add(point, scale(tangent, beyond));
                LinePoint { point, tangent }
            }
            Walk::Curve { table, s0, flip } => {
                let sp = s0 + s;
                if *flip {
                    let p = table.at(table.length() - sp);
                    LinePoint { point: p.point, tangent: scale(p.tangent, -1.0) }
                } else {
                    table.at(sp)
                }
            }
        }
    }

    /// Where a glyph point lands: `pen` = the unrounded pen position of the glyph in its text line, `advance` its
    /// advance, `p` = the point in glyph space after slant and bold shift (`x'`, `y'` of 5.4). The glyph is rigid:
    /// its middle sits on the line and it turns with the tangent.
    ///
    /// This looks the walk up for every call; a painter that places many points of one glyph calls
    /// [`PathLayout::glyph_frame`] once per glyph and [`PathLayout::place_with`] per point (FEATURE_TEXT 5.5: one
    /// look-up per glyph, one rotation per outline point). Both give the same result as `place`.
    pub fn place(&self, line: usize, pen: f32, advance: f32, p: [f32; 2]) -> [f32; 2] {
        Self::place_with(self.glyph_frame(line, pen, advance), advance, p)
    }

    /// The point and tangent of the line under the middle of a glyph (`pen + advance / 2`): one walk look-up per
    /// glyph, to be passed to [`PathLayout::place_with`] for each of its points.
    pub fn glyph_frame(&self, line: usize, pen: f32, advance: f32) -> LinePoint {
        self.line_point(line, pen + advance / 2.0)
    }

    /// Where glyph point `p` lands, given the glyph's frame from [`PathLayout::glyph_frame`]: one rotation.
    pub fn place_with(frame: LinePoint, advance: f32, p: [f32; 2]) -> [f32; 2] {
        let LinePoint { point, tangent } = frame;
        let down = [-tangent[1], tangent[0]];
        let along = p[0] - advance / 2.0;
        [point[0] + tangent[0] * along + down[0] * p[1], point[1] + tangent[1] * along + down[1] * p[1]]
    }
}

/// The start of a text line, measured from the reference point, for a line whose span reaches `half` to either side
/// (FEATURE_TEXT 5.5 "Alignment in angle").
fn align_start(align: TextAlign, half: f32, width: f32) -> f32 {
    match align {
        TextAlign::Left => -half,
        TextAlign::Center => -width / 2.0,
        TextAlign::Right => half - width,
    }
}

/// The straight layout of a text line (an Arc with a bend below 0.001): the line box of width `widest` centred on
/// the anchor, the line at its alignment, `shift` added along the line. `flip` reverses the direction and turns the
/// letters upside down, so the following lines stack upwards from the first baseline `b0`.
fn straight_walk(line: &TextLine, b0: f32, widest: f32, align: TextAlign, shift: f32, flip: bool) -> Walk {
    let x0 = align_start(align, widest / 2.0, line.width) + shift;
    if flip {
        Walk::Straight { origin: [-x0, 2.0 * b0 - line.baseline], dir: [-1.0, 0.0] }
    } else {
        Walk::Straight { origin: [x0, line.baseline], dir: [1.0, 0.0] }
    }
}

/// One cubic Bezier piece.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Bezier([[f32; 2]; 4]);

impl Bezier {
    fn point(&self, t: f32) -> [f32; 2] {
        let [p0, p1, p2, p3] = self.0;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        [a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]
    }

    fn derivative(&self, t: f32) -> [f32; 2] {
        let [p0, p1, p2, p3] = self.0;
        let u = 1.0 - t;
        let (a, b, c) = (3.0 * u * u, 6.0 * u * t, 3.0 * t * t);
        [
            a * (p1[0] - p0[0]) + b * (p2[0] - p1[0]) + c * (p3[0] - p2[0]),
            a * (p1[1] - p0[1]) + b * (p2[1] - p1[1]) + c * (p3[1] - p2[1]),
        ]
    }
}

/// A walk sampled into a table of running chord lengths. Either exact Bezier pieces (the curve itself: a look-up is
/// a binary search, a linear step of the parameter inside the interval and the exact point and derivative there) or
/// a plain polyline with tangents (a line of a text of several lines, moved along the normals).
#[derive(Clone, Debug, PartialEq)]
struct CurveTable {
    /// Exact pieces, or empty for a polyline.
    pieces: Vec<Bezier>,
    /// Sample points with their running length and unit tangent.
    samples: Vec<([f32; 2], f32, [f32; 2])>,
}

impl CurveTable {
    /// The centripetal Catmull-Rom spline through `points` as Bezier pieces (FEATURE_TEXT 5.5). Two points give a
    /// straight piece; one or none give a walk of length 0 at that point (or the origin).
    fn catmull_rom(points: &[[f32; 2]]) -> CurveTable {
        let n = points.len();
        if n < 2 {
            let p = points.first().copied().unwrap_or([0.0, 0.0]);
            return CurveTable { pieces: Vec::new(), samples: vec![(p, 0.0, [1.0, 0.0])] };
        }
        let at = |i: isize| -> [f32; 2] {
            if i < 0 {
                sub(scale(points[0], 2.0), points[1])
            } else if i as usize >= n {
                sub(scale(points[n - 1], 2.0), points[n - 2])
            } else {
                points[i as usize]
            }
        };
        let mut pieces = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            let i = i as isize;
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            if n == 2 {
                let d = sub(p2, p1);
                pieces.push(Bezier([p1, add(p1, scale(d, 1.0 / 3.0)), sub(p2, scale(d, 1.0 / 3.0)), p2]));
                continue;
            }
            let knot = |a: [f32; 2], b: [f32; 2]| length(sub(b, a)).max(MIN_KNOT_PX).sqrt();
            let (d01, d12, d23) = (knot(p0, p1), knot(p1, p2), knot(p2, p3));
            let (t0, t1) = (0.0, d01);
            let t2 = t1 + d12;
            let t3 = t2 + d23;
            let m1 = scale(
                add(sub(scale(sub(p1, p0), 1.0 / (t1 - t0)), scale(sub(p2, p0), 1.0 / (t2 - t0))), scale(sub(p2, p1), 1.0 / (t2 - t1))),
                t2 - t1,
            );
            let m2 = scale(
                add(sub(scale(sub(p2, p1), 1.0 / (t2 - t1)), scale(sub(p3, p1), 1.0 / (t3 - t1))), scale(sub(p3, p2), 1.0 / (t3 - t2))),
                t2 - t1,
            );
            pieces.push(Bezier([p1, add(p1, scale(m1, 1.0 / 3.0)), sub(p2, scale(m2, 1.0 / 3.0)), p2]));
        }
        CurveTable::from_pieces(pieces)
    }

    fn from_pieces(pieces: Vec<Bezier>) -> CurveTable {
        let mut samples = Vec::with_capacity(pieces.len() * CURVE_STEPS + 1);
        let mut total = 0.0f32;
        let mut prev: Option<[f32; 2]> = None;
        for (k, piece) in pieces.iter().enumerate() {
            let first = if k == 0 { 0 } else { 1 };
            for j in first..=CURVE_STEPS {
                let t = j as f32 / CURVE_STEPS as f32;
                let p = piece.point(t);
                if let Some(q) = prev {
                    total += length(sub(p, q));
                }
                prev = Some(p);
                samples.push((p, total, [0.0, 0.0]));
            }
        }
        let mut table = CurveTable { pieces, samples };
        // Tangents of the samples (exact derivatives, or the chord where the derivative vanishes).
        let count = table.samples.len();
        for i in 0..count {
            let (piece, t) = table.param_of_sample(i);
            let chord = sub(table.samples[(i + 1).min(count - 1)].0, table.samples[i.saturating_sub(1)].0);
            let d = table.pieces[piece].derivative(t);
            let tangent = normalize(d, normalize(chord, [1.0, 0.0]));
            table.samples[i].2 = tangent;
        }
        table.fill_degenerate_tangents();
        table
    }

    /// A walk of length 0 has no direction; samples on a part of length 0 take the direction of the next part.
    fn fill_degenerate_tangents(&mut self) {
        if self.length() <= 0.0 {
            for s in &mut self.samples {
                s.2 = [1.0, 0.0];
            }
        }
    }

    /// Piece and parameter of sample `i` (only for exact tables).
    fn param_of_sample(&self, i: usize) -> (usize, f32) {
        let piece = (i / CURVE_STEPS).min(self.pieces.len().saturating_sub(1));
        let t = (i - piece * CURVE_STEPS) as f32 / CURVE_STEPS as f32;
        (piece, t)
    }

    fn length(&self) -> f32 {
        self.samples.last().map_or(0.0, |s| s.1)
    }

    /// The polyline of this walk moved by `dy` along the "down" direction `D = (-T.y, T.x)`, with its own lengths.
    fn offset(&self, dy: f32) -> CurveTable {
        let mut samples = Vec::with_capacity(self.samples.len());
        let mut total = 0.0f32;
        let mut prev: Option<[f32; 2]> = None;
        for &(p, _, t) in &self.samples {
            let q = add(p, scale([-t[1], t[0]], dy));
            if let Some(r) = prev {
                total += length(sub(q, r));
            }
            prev = Some(q);
            samples.push((q, total, t));
        }
        let mut table = CurveTable { pieces: Vec::new(), samples };
        table.fill_degenerate_tangents();
        table
    }

    /// Point and unit tangent at arc length `s`; beyond the ends the walk continues straight along the end tangent.
    fn at(&self, s: f32) -> LinePoint {
        let first = self.samples[0];
        let last = *self.samples.last().unwrap_or(&first);
        if s.is_nan() || s <= 0.0 {
            let s = if s.is_finite() { s } else { 0.0 };
            return LinePoint { point: add(first.0, scale(first.2, s)), tangent: first.2 };
        }
        if s >= last.1 {
            return LinePoint { point: add(last.0, scale(last.2, s - last.1)), tangent: last.2 };
        }
        // The last sample whose running length is at most s.
        let i = self.samples.partition_point(|x| x.1 <= s).saturating_sub(1).min(self.samples.len() - 2);
        let (a, b) = (self.samples[i], self.samples[i + 1]);
        let span = b.1 - a.1;
        let f = if span > 0.0 { ((s - a.1) / span).clamp(0.0, 1.0) } else { 0.0 };
        if self.pieces.is_empty() {
            let point = add(a.0, scale(sub(b.0, a.0), f));
            let tangent = normalize(add(scale(a.2, 1.0 - f), scale(b.2, f)), normalize(sub(b.0, a.0), a.2));
            return LinePoint { point, tangent };
        }
        let (piece, ta) = self.param_of_sample(i);
        // Sample i + 1 may be the first of the next piece (t = 0 there); in the piece of i it is t = ta + 1 / steps.
        let t = ta + f / CURVE_STEPS as f32;
        let bez = &self.pieces[piece];
        let point = bez.point(t);
        let tangent = normalize(bez.derivative(t), normalize(sub(b.0, a.0), a.2));
        LinePoint { point, tangent }
    }
}

/// The Circle the panel creates for a text whose straight frame is `frame` (FEATURE_TEXT 5.5 "Creation"): diameter
/// `max(1, 2 W / (pi px))` em, the top of the circle on the middle of the first baseline, centred on its top point
/// whatever the alignment (with `start` doing the centring when the smallest diameter applies).
pub fn new_circle(frame: &TextFrame, align: TextAlign) -> TextPath {
    let px = frame.em_px.max(1e-6);
    let widest = frame.box_px[0].max(0.0);
    let diameter = (2.0 * widest / (PI * px)).max(1.0);
    let b0 = frame.baseline0_px / px;
    let r = diameter * px / 2.0;
    let centring = ((PI / 2.0) * r - widest / 2.0) / px;
    let start = match align {
        TextAlign::Left => centring,
        TextAlign::Center => 0.0,
        TextAlign::Right => -centring,
    };
    TextPath { shape: PathShape::Circle { diameter, center: [0.0, b0 + diameter / 2.0] }, start, flip: false }
}

/// The Fit button (FEATURE_TEXT 6.4): the diameter of the creation formula for the text as it is now, keeping the top
/// point of the circle (with `flip`, its bottom point). Other shapes are returned unchanged.
pub fn fit_circle(frame: &TextFrame, align: TextAlign, path: &TextPath) -> TextPath {
    let PathShape::Circle { diameter, center } = path.shape else { return path.clone() };
    let fitted = new_circle(frame, align);
    let PathShape::Circle { diameter: new_diameter, .. } = fitted.shape else { return path.clone() };
    // The kept point is the top (bottom with flip): the centre moves by half the change of the diameter.
    let shift = (new_diameter - diameter) / 2.0;
    let center = if path.flip { [center[0], center[1] - shift] } else { [center[0], center[1] + shift] };
    TextPath { shape: PathShape::Circle { diameter: new_diameter, center }, start: fitted.start, flip: path.flip }
}

/// The Curve the panel creates (FEATURE_TEXT 5.5): three points on the first baseline, `(-W/2, b0)`, `(0, b0)`,
/// `(W/2, b0)`, in em.
pub fn new_curve(frame: &TextFrame) -> TextPath {
    let px = frame.em_px.max(1e-6);
    let half = frame.box_px[0].max(0.0) / 2.0 / px;
    let b0 = frame.baseline0_px / px;
    TextPath { shape: PathShape::Curve { points: vec![[-half, b0], [0.0, b0], [half, b0]] }, start: 0.0, flip: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: f32, px: f32) -> TextFrame {
        TextFrame {
            size: [width as u32 + 8, px as u32 + 8],
            anchor_px: [width / 2.0 + 4.0, px / 2.0 + 4.0],
            em_px: px,
            box_px: [width, px * 1.2],
            baseline0_px: -0.1 * px,
            align_px: [4.0, px / 2.0 + 4.0],
            circle_fill: 0.0,
        }
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        length(sub(a, b))
    }

    fn angle_deg(v: [f32; 2]) -> f32 {
        v[1].atan2(v[0]).to_degrees()
    }

    fn path(shape: PathShape) -> TextPath {
        TextPath { shape, start: 0.0, flip: false }
    }

    #[test]
    fn creation_formulas() {
        // A long text: the half circle is as long as the widest line, so start is 0 for every alignment.
        let f = frame(1000.0, 100.0);
        let c = new_circle(&f, TextAlign::Left);
        let PathShape::Circle { diameter, center } = c.shape else { panic!() };
        assert!((diameter - 2.0 * 1000.0 / (PI * 100.0)).abs() < 1e-4);
        assert!((PI / 2.0 * diameter * 100.0 / 2.0 - 1000.0 / 2.0).abs() < 1e-2, "half circle = widest line");
        assert!(c.start.abs() < 1e-4);
        assert!((center[1] - (-0.1 + diameter / 2.0)).abs() < 1e-5, "the top touches the first baseline");
        // A short text: the smallest diameter applies and start centres it.
        let f = frame(20.0, 100.0);
        let left = new_circle(&f, TextAlign::Left);
        let right = new_circle(&f, TextAlign::Right);
        assert!(matches!(left.shape, PathShape::Circle { diameter, .. } if diameter == 1.0));
        assert!(left.start > 0.0 && (left.start + right.start).abs() < 1e-6);
        assert_eq!(new_circle(&f, TextAlign::Center).start, 0.0);
        // Fit keeps the top point.
        let grown = frame(3000.0, 100.0);
        let fitted = fit_circle(&grown, TextAlign::Center, &new_circle(&frame(1000.0, 100.0), TextAlign::Center));
        let (PathShape::Circle { diameter: d0, center: c0 }, PathShape::Circle { diameter: d1, center: c1 }) =
            (new_circle(&frame(1000.0, 100.0), TextAlign::Center).shape, fitted.shape)
        else {
            panic!()
        };
        assert!(((c0[1] - d0 / 2.0) - (c1[1] - d1 / 2.0)).abs() < 1e-4);
        // The curve: three points on the baseline.
        let PathShape::Curve { points } = new_curve(&frame(400.0, 100.0)).shape else { panic!() };
        assert_eq!(points, vec![[-2.0, -0.1], [0.0, -0.1], [2.0, -0.1]]);
    }

    // ----- Arc -----

    /// TEXT 8.1 `arc_bend_is_the_share_of_a_circle`.
    #[test]
    fn arc_bend_is_the_share_of_a_circle() {
        let lines = [TextLine { baseline: -20.0, width: 600.0 }, TextLine { baseline: 100.0, width: 400.0 }];
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            let layout = PathLayout::new(&path(PathShape::Arc { bend: 0.25 }), 100.0, align, &lines);
            let a = layout.line_point(0, 0.0).tangent;
            let b = layout.line_point(0, 600.0).tangent;
            let turn = angle_deg(b) - angle_deg(a);
            assert!((turn - 90.0).abs() < 0.1, "{align:?}: {turn}");
            // The widest line is centred on the top point.
            let mid = layout.line_point(0, 300.0);
            assert!(mid.point[0].abs() < 0.01 && (mid.point[1] + 20.0).abs() < 0.01, "{mid:?}");
        }
        // The second line runs on a concentric circle.
        let layout = PathLayout::new(&path(PathShape::Arc { bend: 0.25 }), 100.0, TextAlign::Center, &lines);
        let r = 600.0 / (0.25 * 2.0 * PI);
        let centre = [0.0, -20.0 + r];
        for s in [0.0, 200.0, 400.0] {
            assert!((dist(layout.line_point(1, s).point, centre) - (r - 120.0)).abs() < 0.05);
        }
    }

    /// TEXT 8.1 `arc_at_zero_is_the_straight_layout`.
    #[test]
    fn arc_at_zero_is_the_straight_layout() {
        let lines = [TextLine { baseline: -30.5, width: 500.0 }, TextLine { baseline: 90.0, width: 320.0 }];
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            let offset = |w: f32| match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (500.0 - w) / 2.0,
                TextAlign::Right => 500.0 - w,
            };
            for bend in [0.0f32, 0.0005, -0.0005] {
                let layout = PathLayout::new(&path(PathShape::Arc { bend }), 100.0, align, &lines);
                for (l, line) in lines.iter().enumerate() {
                    for pen in [0.0f32, 37.5, 250.0, 300.0] {
                        let p = layout.place(l, pen, 60.0, [5.0, -40.0]);
                        let straight = [-250.0 + offset(line.width) + pen + 5.0, line.baseline - 40.0];
                        assert!(dist(p, straight) < 0.01, "{align:?} {bend} {l} {pen}: {p:?} {straight:?}");
                    }
                }
            }
        }
        // A tiny bend just above the limit is as good as straight too (continuity).
        let layout = PathLayout::new(&path(PathShape::Arc { bend: 0.0011 }), 100.0, TextAlign::Left, &lines);
        assert!(dist(layout.place(0, 0.0, 60.0, [0.0, 0.0]), [-250.0 + 30.0 - 30.0, -30.5]) < 0.6);
    }

    /// TEXT 8.1 `negative_bend_keeps_letters_upright`.
    #[test]
    fn negative_bend_keeps_letters_upright() {
        let lines = [TextLine { baseline: 0.0, width: 800.0 }];
        // Up to half a circle (bend -0.5) the tangent turns by at most 90 degrees to either side; at -1 the text
        // runs round a full circle and its ends are upside down by construction.
        for bend in [-0.1f32, -0.25, -0.5] {
            let layout = PathLayout::new(&path(PathShape::Arc { bend }), 100.0, TextAlign::Left, &lines);
            for k in 0..=16 {
                let t = layout.line_point(0, 800.0 * k as f32 / 16.0).tangent;
                // Reading to the right; "up" of the letter (-D) points up on screen as long as the tangent turns by
                // less than 90 degrees.
                assert!(t[0] > -1e-3, "{bend} {k}: {t:?}");
                let up = [t[1], -t[0]];
                assert!(up[1] <= 1e-3);
            }
            // The middle is the lowest point: the circle is above (the text sags).
            let mid = layout.line_point(0, 400.0).point;
            let end = layout.line_point(0, 0.0).point;
            assert!(mid[1] > end[1] + 10.0, "{bend}: {mid:?} {end:?}");
            assert!(angle_deg(layout.line_point(0, 400.0).tangent).abs() < 0.01);
        }
        // A positive bend arches the other way.
        let layout = PathLayout::new(&path(PathShape::Arc { bend: 0.5 }), 100.0, TextAlign::Left, &lines);
        assert!(layout.line_point(0, 400.0).point[1] < layout.line_point(0, 0.0).point[1] - 10.0);
    }

    #[test]
    fn arc_flip_reads_the_other_way() {
        let lines = [TextLine { baseline: 0.0, width: 400.0 }];
        let mut p = path(PathShape::Arc { bend: 0.25 });
        let plain = PathLayout::new(&p, 100.0, TextAlign::Center, &lines);
        p.flip = true;
        let flipped = PathLayout::new(&p, 100.0, TextAlign::Center, &lines);
        for s in [0.0, 100.0, 200.0, 400.0] {
            let a = plain.line_point(0, s);
            let b = flipped.line_point(0, s);
            // Same circle, mirrored position, reversed direction.
            assert!((a.point[0] + b.point[0]).abs() < 1e-3 && (a.point[1] - b.point[1]).abs() < 1e-3);
            assert!(dist(a.tangent, [-b.tangent[0], b.tangent[1]]) < 1e-4);
        }
    }

    /// TEXT 8.1 `start_is_limited_on_open_lines`.
    #[test]
    fn start_is_limited_on_open_lines() {
        let px = 100.0;
        let lines = [TextLine { baseline: 0.0, width: 500.0 }];
        // A curve of exactly the line length (5 em).
        let curve = PathShape::Curve { points: vec![[-2.5, 0.0], [0.0, 0.0], [2.5, 0.0]] };
        for shape in [PathShape::Arc { bend: 0.3 }, curve.clone()] {
            let far = PathLayout::new(&TextPath { shape: shape.clone(), start: 100.0, flip: false }, px, TextAlign::Left, &lines);
            let at_five = PathLayout::new(&TextPath { shape: shape.clone(), start: 5.0, flip: false }, px, TextAlign::Left, &lines);
            let back = PathLayout::new(&TextPath { shape: shape.clone(), start: -100.0, flip: false }, px, TextAlign::Left, &lines);
            let back_five = PathLayout::new(&TextPath { shape: shape.clone(), start: -5.0, flip: false }, px, TextAlign::Left, &lines);
            for s in [0.0, 250.0, 500.0] {
                assert!(dist(far.line_point(0, s).point, at_five.line_point(0, s).point) < 1e-3, "{shape:?}");
                assert!(dist(back.line_point(0, s).point, back_five.line_point(0, s).point) < 1e-3, "{shape:?}");
            }
            // One line length past the end: the start of the text sits exactly at the end of the line plus nothing
            // more than the line length.
            let plain = PathLayout::new(&TextPath { shape: shape.clone(), start: 0.0, flip: false }, px, TextAlign::Left, &lines);
            let end = plain.line_point(0, 500.0);
            let shifted = far.line_point(0, 0.0);
            assert!(dist(shifted.point, end.point) < 0.5, "{shape:?}: {shifted:?} {end:?}");
        }
    }

    // ----- Circle -----

    /// TEXT 8.1 `circle_does_not_depend_on_the_text`.
    #[test]
    fn circle_does_not_depend_on_the_text() {
        let shape = PathShape::Circle { diameter: 4.0, center: [0.3, 1.9] };
        let centre = [30.0, 190.0];
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            for width in [60.0f32, 480.0] {
                let layout = PathLayout::new(&path(shape.clone()), 100.0, align, &[TextLine { baseline: -10.0, width }]);
                for s in [0.0, width / 2.0, width] {
                    assert!((dist(layout.line_point(0, s).point, centre) - 200.0).abs() < 0.01);
                }
            }
        }
    }

    /// TEXT 8.1 `circle_flip_reads_upright_at_the_bottom`.
    #[test]
    fn circle_flip_reads_upright_at_the_bottom() {
        let shape = PathShape::Circle { diameter: 4.0, center: [0.0, 0.0] };
        let mut p = path(shape);
        p.flip = true;
        let layout = PathLayout::new(&p, 100.0, TextAlign::Center, &[TextLine { baseline: 0.0, width: 100.0 }]);
        let mid = layout.line_point(0, 50.0);
        assert!(dist(mid.point, [0.0, 200.0]) < 0.01, "{mid:?}");
        assert!(dist(mid.tangent, [1.0, 0.0]) < 1e-5);
        // A letter's top (y' < 0) points towards the centre: upright, inside the circle.
        let top = layout.place(0, 25.0, 50.0, [25.0, -30.0]);
        assert!(top[1] < 200.0 - 29.0 && top[1] > 0.0);
        // Reading left to right: the start is left of the end.
        assert!(layout.line_point(0, 0.0).point[0] < layout.line_point(0, 100.0).point[0]);
        // Without flip the middle is on top and reads left to right too.
        let plain = PathLayout::new(&path(p.shape.clone()), 100.0, TextAlign::Center, &[TextLine { baseline: 0.0, width: 100.0 }]);
        assert!(dist(plain.line_point(0, 50.0).point, [0.0, -200.0]) < 0.01);
        assert!(dist(plain.line_point(0, 50.0).tangent, [1.0, 0.0]) < 1e-5);
    }

    /// Advances of the glyphs of `text` in the bundled Ubuntu Light at `px` (kerning included), as the layout gives
    /// them: (pen, advance) per glyph and the line width.
    fn ubuntu_line(text: &str, px: f32) -> (Vec<(f32, f32)>, f32) {
        let font = crate::text::layout::testkit::ubuntu(px);
        let mut pen = 0.0f32;
        let mut glyphs = Vec::new();
        let mut prev: Option<u16> = None;
        for ch in text.chars() {
            let g = font.lookup_glyph_index(ch);
            if let Some(p) = prev {
                pen += font.horizontal_kern_indexed(p, g, px).unwrap_or(0.0);
            }
            let adv = font.metrics_indexed(g, px).advance_width;
            glyphs.push((pen, adv));
            pen += adv;
            prev = Some(g);
        }
        (glyphs, pen)
    }

    fn straight_frame(width: f32, px: f32) -> TextFrame {
        let box_h = (1.2 * px).ceil();
        TextFrame {
            size: [width.ceil() as u32, box_h as u32],
            anchor_px: [width.ceil() / 2.0, box_h / 2.0],
            em_px: px,
            box_px: [width.ceil(), box_h],
            baseline0_px: (0.93 * px).round() - box_h / 2.0,
            align_px: [0.0, box_h / 2.0],
            circle_fill: 0.0,
        }
    }

    /// TEXT 8.1 `a_new_circle_is_centred_on_its_top`. "First" and "last glyph" at 9 and 3 o'clock are read as the
    /// start and the end of the line (the pen at 0 and at the line width): the middle of a wide first glyph is
    /// necessarily further round.
    #[test]
    fn a_new_circle_is_centred_on_its_top() {
        let px = 128.0;
        for text in ["WTE 2026", "Hi"] {
            let (glyphs, width) = ubuntu_line(text, px);
            let frame = straight_frame(width, px);
            let line = [TextLine { baseline: frame.baseline0_px, width }];
            for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
                let p = new_circle(&frame, align);
                let PathShape::Circle { center, .. } = p.shape else { panic!() };
                let c = [center[0] * px, center[1] * px];
                let layout = PathLayout::new(&p, px, align, &line);
                let middles: Vec<[f32; 2]> =
                    glyphs.iter().map(|&(pen, adv)| layout.line_point(0, pen + adv / 2.0).point).collect();
                for (m, (pen, adv)) in middles.iter().zip(&glyphs) {
                    assert!(m[1] < c[1], "{text} {align:?}: above the centre");
                    let t = layout.line_point(0, pen + adv / 2.0).tangent;
                    assert!(angle_deg(t).abs() <= 95.0, "{text} {align:?}: turned {}", angle_deg(t));
                }
                if text == "Hi" {
                    // The line itself is centred on the top point exactly. The centroid of the two glyph middles
                    // is off by half the difference of the two advances turned onto the circle: 0.1005 em for
                    // "H" and "i" of Ubuntu Light (the design's limit is 0.1 em; see the T1 note).
                    let line_mid = layout.line_point(0, width / 2.0).point;
                    assert!((line_mid[0] - c[0]).abs() < 0.01 * px, "{align:?}: line middle {line_mid:?}");
                    let mean = middles.iter().map(|m| m[0]).sum::<f32>() / middles.len() as f32;
                    assert!((mean - c[0]).abs() < 0.11 * px, "{align:?}: centroid {mean} vs {}", c[0]);
                } else {
                    // Symmetric about the vertical line through the centre.
                    let n = middles.len();
                    for i in 0..n {
                        let (a, b) = (middles[i], middles[n - 1 - i]);
                        let (wa, wb) = (glyphs[i].1, glyphs[n - 1 - i].1);
                        // Glyph middles mirror exactly only for glyphs of equal advance; "WTE 2026" has no such
                        // mirrored pair in Ubuntu Light (W/6, T/2, E/0, space/2 all differ), so this compares
                        // nothing for it. What is checked for every glyph instead: the walk is symmetric, the
                        // mirror of each glyph middle is the point at the mirrored arc length `width - s`.
                        if (wa - wb).abs() < 1e-3 {
                            assert!(((a[0] - c[0]) + (b[0] - c[0])).abs() < 0.05 * px, "{align:?} {i}");
                        }
                        let s = glyphs[i].0 + glyphs[i].1 / 2.0;
                        let m = layout.line_point(0, width - s).point;
                        assert!(((a[0] - c[0]) + (m[0] - c[0])).abs() < 0.05 * px && (a[1] - m[1]).abs() < 0.05 * px, "{align:?} {i}");
                    }
                    let start = layout.line_point(0, 0.0).point;
                    let end = layout.line_point(0, width).point;
                    let clock = |p: [f32; 2]| angle_deg(sub(p, c));
                    assert!((clock(start).abs() - 180.0).abs() < 3.0, "{align:?}: start at {}", clock(start));
                    assert!(clock(end).abs() < 3.0, "{align:?}: end at {}", clock(end));
                    assert!(((start[0] - c[0]) + (end[0] - c[0])).abs() < 0.05 * px);
                }
            }
        }
    }

    /// TEXT 8.1 `circle_alignment_pins_its_side`.
    #[test]
    fn circle_alignment_pins_its_side() {
        let shape = PathShape::Circle { diameter: 6.0, center: [0.0, 3.0] };
        let c = [0.0, 300.0];
        let clock = |l: &PathLayout, s: f32| angle_deg(sub(l.line_point(0, s).point, c));
        for width in [200.0f32, 400.0, 700.0] {
            let line = [TextLine { baseline: 0.0, width }];
            let left = PathLayout::new(&path(shape.clone()), 100.0, TextAlign::Left, &line);
            let right = PathLayout::new(&path(shape.clone()), 100.0, TextAlign::Right, &line);
            let centre = PathLayout::new(&path(shape.clone()), 100.0, TextAlign::Center, &line);
            assert!((clock(&left, 0.0).abs() - 180.0).abs() < 0.01, "left starts at 9 o'clock");
            assert!(clock(&right, width).abs() < 0.01, "right ends at 3 o'clock");
            assert!((clock(&centre, width / 2.0) + 90.0).abs() < 0.01, "the middle stays on top");
        }
    }

    /// TEXT 8.1 `circle_start_turns_all_lines_alike`.
    #[test]
    fn circle_start_turns_all_lines_alike() {
        let shape = PathShape::Circle { diameter: 8.0, center: [0.0, 4.0] };
        let lines = [TextLine { baseline: 0.0, width: 300.0 }, TextLine { baseline: 120.0, width: 200.0 }];
        let c = [0.0, 400.0];
        let a = PathLayout::new(&TextPath { shape: shape.clone(), start: 0.0, flip: false }, 100.0, TextAlign::Center, &lines);
        let b = PathLayout::new(&TextPath { shape, start: 1.0, flip: false }, 100.0, TextAlign::Center, &lines);
        let turn = |l: usize, s: f32| {
            angle_deg(sub(b.line_point(l, s).point, c)) - angle_deg(sub(a.line_point(l, s).point, c))
        };
        let expected = (100.0f32 / 400.0).to_degrees();
        assert!((turn(0, 50.0) - expected).abs() < 0.01, "{}", turn(0, 50.0));
        assert!((turn(1, 50.0) - expected).abs() < 0.01, "{}", turn(1, 50.0));
    }

    /// TEXT 8.1 `circle_fill_reports_overflow`.
    #[test]
    fn circle_fill_reports_overflow() {
        let shape = PathShape::Circle { diameter: 4.0, center: [0.0, 2.0] };
        let circumference = 2.0 * PI * 200.0;
        for share in [1.5f32, 0.5] {
            let layout = PathLayout::new(&path(shape.clone()), 100.0, TextAlign::Left, &[TextLine { baseline: 0.0, width: share * circumference }]);
            assert!((layout.circle_fill / share - 1.0).abs() < 0.01, "{}", layout.circle_fill);
        }
        let arc = PathLayout::new(&path(PathShape::Arc { bend: 0.5 }), 100.0, TextAlign::Left, &[TextLine { baseline: 0.0, width: 900.0 }]);
        assert_eq!(arc.circle_fill, 0.0);
        // An inner line on a smaller circle fills more.
        let two = PathLayout::new(
            &path(shape),
            100.0,
            TextAlign::Left,
            &[TextLine { baseline: 0.0, width: 600.0 }, TextLine { baseline: 100.0, width: 600.0 }],
        );
        assert!((two.circle_fill - 600.0 / (2.0 * PI * 100.0)).abs() < 1e-3);
    }

    /// TEXT 8.1 `a_gap_is_applied_before_the_line`: "AB" on a Circle of 4 em; a gap of 0.5 em after "A" (the pen of
    /// "B" moves by 0.5 em) increases the angle between the two letters by 0.25 radian.
    #[test]
    fn a_gap_is_applied_before_the_line() {
        let px = 128.0;
        let (glyphs, width) = ubuntu_line("AB", px);
        let shape = PathShape::Circle { diameter: 4.0, center: [0.0, 2.0] };
        let c = [0.0, 2.0 * px];
        let angle = |gap: f32| {
            let line = [TextLine { baseline: 0.0, width: width + gap }];
            let layout = PathLayout::new(&path(shape.clone()), px, TextAlign::Left, &line);
            let a = layout.line_point(0, glyphs[0].0 + glyphs[0].1 / 2.0).point;
            let b = layout.line_point(0, glyphs[1].0 + gap + glyphs[1].1 / 2.0).point;
            let (aa, ab) = (sub(a, c)[1].atan2(sub(a, c)[0]), sub(b, c)[1].atan2(sub(b, c)[0]));
            ab - aa
        };
        let grown = angle(0.5 * px) - angle(0.0);
        assert!((grown / 0.25 - 1.0).abs() < 0.01, "{grown}");
    }

    // ----- Curve -----

    fn curve_layout(points: &[[f32; 2]], px: f32, align: TextAlign, width: f32) -> PathLayout {
        PathLayout::new(&path(PathShape::Curve { points: points.to_vec() }), px, align, &[TextLine { baseline: 0.0, width }])
    }

    /// TEXT 8.1 `curve_passes_through_its_points`.
    #[test]
    fn curve_passes_through_its_points() {
        let pts = [[-3.0, 0.5], [-1.0, -1.0], [0.2, 0.0], [0.25, 0.1], [2.0, 2.5], [4.0, 0.0]];
        let px = 64.0;
        let layout = curve_layout(&pts, px, TextAlign::Left, 10.0);
        let Walk::Curve { table, .. } = &layout.walks[0] else { panic!() };
        let len = table.length();
        for p in pts {
            let target = [p[0] * px, p[1] * px];
            let nearest = (0..=20_000)
                .map(|i| dist(layout.line_point(0, len * i as f32 / 20_000.0).point, target))
                .fold(f32::INFINITY, f32::min);
            assert!(nearest < 0.01 * px, "{p:?}: {nearest}");
        }
        // The knots are pieces' end points exactly.
        for (k, piece) in table.pieces.iter().enumerate() {
            assert!(dist(piece.0[0], [pts[k][0] * px, pts[k][1] * px]) < 1e-3);
        }
    }

    /// TEXT 8.1 `curve_does_not_overshoot`.
    #[test]
    fn curve_does_not_overshoot() {
        let pts = [[0.0, 0.0], [4.0, 0.0], [4.2, 0.2], [4.2, 4.0]];
        let px = 100.0;
        let layout = curve_layout(&pts, px, TextAlign::Left, 10.0);
        let Walk::Curve { table, .. } = &layout.walks[0] else { panic!() };
        let grow = 0.15 * px;
        for &(p, _, _) in &table.samples {
            assert!(p[0] >= -grow && p[0] <= 4.2 * px + grow, "{p:?}");
            assert!(p[1] >= -grow && p[1] <= 4.0 * px + grow, "{p:?}");
        }
    }

    /// TEXT 8.1 `length_table_is_exact`.
    #[test]
    fn length_table_is_exact() {
        // A straight piece.
        let layout = curve_layout(&[[0.0, 0.0], [3.0, 4.0]], 100.0, TextAlign::Left, 10.0);
        let Walk::Curve { table, .. } = &layout.walks[0] else { panic!() };
        assert!((table.length() - 500.0).abs() < 0.001, "{}", table.length());
        let mid = table.at(250.0);
        assert!(dist(mid.point, [150.0, 200.0]) < 0.001);
        // A quarter circle of radius 100 px as one Bezier piece.
        let k = 0.552_284_8 * 100.0;
        let quarter = CurveTable::from_pieces(vec![Bezier([[100.0, 0.0], [100.0, k], [k, 100.0], [0.0, 100.0]])]);
        let expected = PI / 2.0 * 100.0;
        assert!((quarter.length() / expected - 1.0).abs() < 0.001, "{}", quarter.length());
        // Look-ups land on the circle with tangents along it.
        for s in [10.0, 78.0, 150.0] {
            let p = quarter.at(s);
            assert!((length(p.point) - 100.0).abs() < 0.05);
            assert!((p.point[0] * p.tangent[0] + p.point[1] * p.tangent[1]).abs() < 0.5);
        }
        // At most 480 entries for 16 points (plus the first sample).
        let pts: Vec<[f32; 2]> = (0..16).map(|i| [i as f32, (i % 3) as f32]).collect();
        let layout = curve_layout(&pts, 10.0, TextAlign::Left, 10.0);
        let Walk::Curve { table, .. } = &layout.walks[0] else { panic!() };
        assert_eq!(table.samples.len(), 15 * CURVE_STEPS + 1);
    }

    /// TEXT 8.1 `glyph_middles_are_spaced_by_their_advances`.
    #[test]
    fn glyph_middles_are_spaced_by_their_advances() {
        let px = 128.0;
        let (glyphs, width) = ubuntu_line("WTE 2026 on a curve", px);
        let pts = [[-8.0, 0.0], [-3.0, -2.0], [1.0, 1.5], [6.0, -1.0], [9.0, 0.5]];
        let layout = curve_layout(&pts, px, TextAlign::Left, width);
        // True arc length by fine integration of the exact walk.
        let arc = |s0: f32, s1: f32| {
            let n = 400;
            let mut len = 0.0;
            let mut prev = layout.line_point(0, s0).point;
            for i in 1..=n {
                let p = layout.line_point(0, s0 + (s1 - s0) * i as f32 / n as f32).point;
                len += dist(p, prev);
                prev = p;
            }
            len
        };
        for w in glyphs.windows(2) {
            let (a, b) = (w[0].0 + w[0].1 / 2.0, w[1].0 + w[1].1 / 2.0);
            assert!((arc(a, b) - (b - a)).abs() < 0.5, "{a} {b}: {}", arc(a, b));
        }
    }

    /// TEXT 8.1 `text_longer_than_the_line_continues_straight`.
    #[test]
    fn text_longer_than_the_line_continues_straight() {
        let pts = [[0.0, 0.0], [2.0, -1.0], [4.0, 0.0]];
        let layout = curve_layout(&pts, 100.0, TextAlign::Left, 900.0);
        let Walk::Curve { table, .. } = &layout.walks[0] else { panic!() };
        let len = table.length();
        let end = layout.line_point(0, len);
        for extra in [10.0, 200.0, 500.0] {
            let p = layout.line_point(0, len + extra);
            assert!(dist(p.point, add(end.point, scale(end.tangent, extra))) < 1e-3);
            assert_eq!(p.tangent, end.tangent);
        }
        let start = layout.line_point(0, 0.0);
        let before = layout.line_point(0, -50.0);
        assert!(dist(before.point, sub(start.point, scale(start.tangent, 50.0))) < 1e-3);
        // The same for an Arc.
        let arc = PathLayout::new(&path(PathShape::Arc { bend: 0.5 }), 100.0, TextAlign::Left, &[TextLine { baseline: 0.0, width: 400.0 }]);
        let end = arc.line_point(0, 400.0);
        let past = arc.line_point(0, 600.0);
        assert!(dist(past.point, add(end.point, scale(end.tangent, 200.0))) < 1e-2);
    }

    /// TEXT 8.1 `flip_reverses_the_direction`.
    #[test]
    fn flip_reverses_the_direction() {
        let pts = vec![[0.0, 0.0], [2.0, -1.0], [4.0, 0.5]];
        let line = [TextLine { baseline: 0.0, width: 200.0 }];
        let plain = PathLayout::new(&path(PathShape::Curve { points: pts.clone() }), 100.0, TextAlign::Left, &line);
        let flipped = PathLayout::new(&TextPath { shape: PathShape::Curve { points: pts }, start: 0.0, flip: true }, 100.0, TextAlign::Left, &line);
        let Walk::Curve { table, .. } = &plain.walks[0] else { panic!() };
        let len = table.length();
        for s in [0.0, 50.0, 199.0] {
            let a = plain.line_point(0, len - s);
            let b = flipped.line_point(0, s);
            assert!(dist(a.point, b.point) < 1e-3);
            assert!(dist(a.tangent, scale(b.tangent, -1.0)) < 1e-5);
        }
        // The flipped text starts at the last point.
        assert!(dist(flipped.line_point(0, 0.0).point, [400.0, 50.0]) < 1e-3);
    }

    /// With "Other side" the letters turn round, so the second text line must lie on their "down" side of the first
    /// (review of T1): Arc, a straight Arc and a Curve, each flipped, lines at baselines 0 and 120.
    #[test]
    fn flipped_lines_stack_under_the_first() {
        let lines = [TextLine { baseline: 0.0, width: 400.0 }, TextLine { baseline: 120.0, width: 300.0 }];
        let curve = PathShape::Curve { points: vec![[-2.0, 0.0], [0.0, -0.5], [2.0, 0.0]] };
        for shape in [PathShape::Arc { bend: 0.25 }, PathShape::Arc { bend: -0.25 }, PathShape::Arc { bend: 0.0005 }, curve] {
            for flip in [false, true] {
                for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
                    let layout = PathLayout::new(&TextPath { shape: shape.clone(), start: 0.0, flip }, 100.0, align, &lines);
                    let first = layout.line_point(0, 200.0);
                    let second = layout.line_point(1, 150.0);
                    let down = [-first.tangent[1], first.tangent[0]];
                    let d = sub(second.point, first.point);
                    let below = d[0] * down[0] + d[1] * down[1];
                    assert!(below > 60.0, "{shape:?} flip {flip} {align:?}: {below}");
                }
            }
        }
    }

    /// Finite but huge points (beyond what `sanitize_fx` lets through) give a defined layout, no panic.
    #[test]
    fn huge_curve_points_do_not_panic() {
        let lines = [TextLine { baseline: 0.0, width: 100.0 }];
        for pts in [vec![[-1e30, 0.0], [0.0, 1e30], [1e30, 0.0]], vec![[-3e38, 0.0], [3e38, 0.0]]] {
            for flip in [false, true] {
                let p = TextPath { shape: PathShape::Curve { points: pts.clone() }, start: 5.0, flip };
                let layout = PathLayout::new(&p, 128.0, TextAlign::Center, &lines);
                let _ = layout.place(0, 10.0, 20.0, [1.0, -5.0]);
            }
        }
    }

    /// Review of T1: the per-glyph frame plus one rotation per point gives exactly what `place` gives, on every
    /// shape, both directions, every line (so a painter can look the walk up once per glyph).
    #[test]
    fn a_glyph_frame_places_like_place() {
        let lines = [TextLine { baseline: -10.0, width: 300.0 }, TextLine { baseline: 110.0, width: 180.0 }];
        let shapes = [
            PathShape::Arc { bend: 0.0 },
            PathShape::Arc { bend: 0.4 },
            PathShape::Arc { bend: -0.7 },
            PathShape::Circle { diameter: 4.0, center: [0.3, 1.9] },
            PathShape::Curve { points: vec![[-2.0, 0.0], [-0.5, -1.0], [1.0, 0.5], [2.5, 0.0]] },
        ];
        for shape in shapes {
            for flip in [false, true] {
                let p = TextPath { shape: shape.clone(), start: 0.7, flip };
                let layout = PathLayout::new(&p, 100.0, TextAlign::Center, &lines);
                for line in 0..lines.len() {
                    for (pen, advance) in [(-40.0f32, 25.0f32), (0.0, 61.0), (130.0, 48.0), (290.0, 70.0)] {
                        let frame = layout.glyph_frame(line, pen, advance);
                        assert_eq!(frame, layout.line_point(line, pen + advance / 2.0));
                        for q in [[0.0, 0.0], [advance, -70.0], [12.5, 18.0], [-3.0, -33.0]] {
                            let want = layout.place(line, pen, advance, q);
                            let got = PathLayout::place_with(frame, advance, q);
                            assert_eq!(got, want, "{shape:?} flip {flip} line {line} pen {pen} point {q:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn curve_lines_and_alignment() {
        // A second text line runs below the curve, at the line distance, with its own length.
        let pts = vec![[-2.0, 0.0], [0.0, 0.0], [2.0, 0.0]];
        let lines = [TextLine { baseline: -10.0, width: 100.0 }, TextLine { baseline: 110.0, width: 60.0 }];
        // Points are relative to the anchor: the first line's baseline is the curve itself (here at y = 0).
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            let layout = PathLayout::new(&path(PathShape::Curve { points: pts.clone() }), 100.0, align, &lines);
            let p = layout.line_point(1, 0.0).point;
            assert!((p[1] - 120.0).abs() < 1e-3, "{p:?}");
            let x0 = match align {
                TextAlign::Left => -200.0,
                TextAlign::Center => -30.0,
                TextAlign::Right => 140.0,
            };
            assert!((p[0] - x0).abs() < 1e-2, "{align:?} {p:?}");
        }
        // Degenerate curves never panic and stay finite.
        for pts in [vec![[1.0, 1.0], [1.0, 1.0]], vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]], vec![[f32::NAN, 0.0], [1.0, 0.0]]] {
            let layout = PathLayout::new(&path(PathShape::Curve { points: pts }), 100.0, TextAlign::Center, &lines);
            for s in [-10.0, 0.0, 30.0] {
                let p = layout.line_point(1, s);
                assert!(p.point.iter().chain(&p.tangent).all(|v| v.is_finite()));
            }
        }
        // No lines: still a defined layout.
        let empty = PathLayout::new(&path(PathShape::Arc { bend: 0.5 }), 100.0, TextAlign::Left, &[]);
        assert_eq!(empty.line_count(), 0);
        assert_eq!(empty.line_point(0, 3.0).point, [3.0, 0.0]);
    }
}
