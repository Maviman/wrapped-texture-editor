// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Layer styles (STICKER_PLAN 2.3, ARCHITECTURE 5.27 / 6.11): the `StyleStack` of a sticker layer or a stamp, its
//! margin, its limits and the presets. The pixels are drawn by `fx::apply_styles`.
//!
//! Lengths are pixels at the **reference scale** of what the stack is applied to (layer-space pixels for a sticker
//! layer; the base picture at reference size for a stamp). `apply_styles` multiplies them by its `scale`.

use crate::sticker::{MAX_GRADIENT_STOPS, MAX_STYLE_LENGTH};

/// A colour gradient: 2..=8 stops `(position 0..=1, straight sRGB RGBA8)`, positions ascending.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub stops: Vec<(f32, [u8; 4])>,
}

impl Gradient {
    /// Two stops: `a` at 0, `b` at 1.
    pub fn two(a: [u8; 4], b: [u8; 4]) -> Gradient {
        Gradient { stops: vec![(0.0, a), (1.0, b)] }
    }

    /// Colour at `t` (clamped to 0..=1): linear interpolation of the two neighbouring stops in straight sRGB bytes,
    /// rounded half up (4.3a). Before the first stop its colour, after the last stop its colour; a stop exactly at
    /// `t` gives its own colour. An empty gradient is transparent.
    pub fn sample(&self, t: f32) -> [u8; 4] {
        let stops = &self.stops;
        let Some(first) = stops.first() else { return [0; 4] };
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        if t <= first.0 {
            return first.1;
        }
        let last = stops[stops.len() - 1];
        if t >= last.0 {
            return last.1;
        }
        // The last stop at or before t, and the one after it.
        let k = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0);
        let (p0, c0) = stops[k];
        let (p1, c1) = stops[(k + 1).min(stops.len() - 1)];
        if p1 <= p0 {
            return c1;
        }
        let f = (t - p0) / (p1 - p0);
        std::array::from_fn(|i| {
            let (a, b) = (c0[i] as f32, c1[i] as f32);
            (a + (b - a) * f + 0.5).floor().clamp(0.0, 255.0) as u8
        })
    }

    /// Limits of 2.3: 2..=8 stops, finite positions in 0..=1, ascending (stable order for equal positions). A
    /// gradient with fewer than two stops gets the missing ends (none: black to white; one: that colour at 0 and 1);
    /// stops past the eighth are dropped.
    pub fn sanitize(&mut self) {
        for s in &mut self.stops {
            s.0 = if s.0.is_finite() { s.0.clamp(0.0, 1.0) } else { 0.0 };
        }
        self.stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.stops.truncate(MAX_GRADIENT_STOPS);
        match self.stops.len() {
            0 => *self = Gradient::two([0, 0, 0, 255], [255, 255, 255, 255]),
            1 => {
                let c = self.stops[0].1;
                *self = Gradient::two(c, c);
            }
            _ => {}
        }
    }
}

/// Where a stroke sits relative to the outline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrokePos {
    Outside,
    Inside,
    Center,
}

/// Die-cut backing shape (E1): the artwork's outline grown by `width`, smoothed, holes optionally filled.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    pub on: bool,
    pub width: f32,
    /// 0..=1: how much of `width` is used to round the shape (closes dents narrower than about `smooth * width`).
    pub smooth: f32,
    pub fill_holes: bool,
    pub color: [u8; 4],
    pub opacity: f32,
}

/// Stroke along the artwork's outline (around the contour when one is on).
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub on: bool,
    pub width: f32,
    pub position: StrokePos,
    pub color: [u8; 4],
    pub opacity: f32,
}

/// Drop shadow or inner shadow.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub on: bool,
    pub color: [u8; 4],
    pub opacity: f32,
    /// Where the light comes from, degrees, counter-clockwise from the +x axis (y down: 90 = light from the top).
    pub angle_deg: f32,
    pub distance: f32,
    /// 0..=1: the hard part of `size` (4.2 spread split).
    pub spread: f32,
    pub size: f32,
}

/// Outer or inner glow.
#[derive(Clone, Debug, PartialEq)]
pub struct Glow {
    pub on: bool,
    pub color: [u8; 4],
    pub opacity: f32,
    pub spread: f32,
    pub size: f32,
}

/// Recolours the artwork (alpha kept).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorOverlay {
    pub on: bool,
    pub color: [u8; 4],
    pub opacity: f32,
}

/// Recolours the artwork with a gradient across its box (alpha kept).
#[derive(Clone, Debug, PartialEq)]
pub struct GradientOverlay {
    pub on: bool,
    pub gradient: Gradient,
    /// Direction of the gradient, degrees counter-clockwise from +x (y down: 90 = first stop at the bottom).
    pub angle_deg: f32,
    pub opacity: f32,
}

/// Smooth inner bevel.
#[derive(Clone, Debug, PartialEq)]
pub struct Bevel {
    pub on: bool,
    pub size: f32,
    /// 0..=10 (1 = a slope of about 45 degrees at the edge, Photoshop's 100 %).
    pub depth: f32,
    pub angle_deg: f32,
    /// 0..=90 degrees above the picture plane.
    pub altitude_deg: f32,
    pub highlight: [u8; 4],
    pub shadow: [u8; 4],
    pub opacity: f32,
}

/// Every layer style of one sticker layer or stamp (2.3). An entry with `on: false` keeps the user's numbers.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StyleStack {
    pub contour: Option<Contour>,
    pub stroke: Option<Stroke>,
    pub drop_shadow: Option<Shadow>,
    pub inner_shadow: Option<Shadow>,
    pub outer_glow: Option<Glow>,
    pub inner_glow: Option<Glow>,
    pub color_overlay: Option<ColorOverlay>,
    pub gradient_overlay: Option<GradientOverlay>,
    pub bevel: Option<Bevel>,
}

/// A length of 2.3: finite, 0..=256.
fn length(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, MAX_STYLE_LENGTH) } else { 0.0 }
}

/// A fraction: finite, 0..=1 (`fallback` when not finite).
fn unit(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { fallback }
}

/// An angle: finite (0 when not), any value.
fn angle(v: f32) -> f32 {
    if v.is_finite() { v.clamp(-3600.0, 3600.0) } else { 0.0 }
}

/// Light offset of a shadow: `round(distance * (-cos a, sin a))` (4.3 step 4; y down, the angle says where the light
/// comes from, so the shadow falls the other way).
pub(crate) fn light_offset(angle_deg: f32, distance: f32) -> (i32, i32) {
    let a = angle_deg.to_radians();
    let dx = (-(a.cos()) * distance).round();
    let dy = (a.sin() * distance).round();
    (dx as i32, dy as i32)
}

impl StyleStack {
    /// True when every member is `None` (an entry with `on: false` counts as used).
    pub fn is_default(&self) -> bool {
        *self == StyleStack::default()
    }

    /// True when at least one entry is on.
    pub fn any_on(&self) -> bool {
        self.contour.as_ref().is_some_and(|c| c.on)
            || self.stroke.as_ref().is_some_and(|s| s.on)
            || self.drop_shadow.as_ref().is_some_and(|s| s.on)
            || self.inner_shadow.as_ref().is_some_and(|s| s.on)
            || self.outer_glow.as_ref().is_some_and(|g| g.on)
            || self.inner_glow.as_ref().is_some_and(|g| g.on)
            || self.color_overlay.as_ref().is_some_and(|o| o.on)
            || self.gradient_overlay.as_ref().is_some_and(|o| o.on)
            || self.bevel.as_ref().is_some_and(|b| b.on)
    }

    /// Clamps every number into its range and replaces non-finite values (2.3): lengths 0..=256, opacities, spreads
    /// and smoothing 0..=1, bevel depth 0..=10 and altitude 0..=90, gradients as `Gradient::sanitize`.
    pub fn sanitize(&mut self) {
        if let Some(c) = &mut self.contour {
            c.width = length(c.width);
            c.smooth = unit(c.smooth, 0.0);
            c.opacity = unit(c.opacity, 1.0);
        }
        if let Some(s) = &mut self.stroke {
            s.width = length(s.width);
            s.opacity = unit(s.opacity, 1.0);
        }
        for s in [&mut self.drop_shadow, &mut self.inner_shadow].into_iter().flatten() {
            s.opacity = unit(s.opacity, 1.0);
            s.angle_deg = angle(s.angle_deg);
            s.distance = length(s.distance);
            s.spread = unit(s.spread, 0.0);
            s.size = length(s.size);
        }
        for g in [&mut self.outer_glow, &mut self.inner_glow].into_iter().flatten() {
            g.opacity = unit(g.opacity, 1.0);
            g.spread = unit(g.spread, 0.0);
            g.size = length(g.size);
        }
        if let Some(o) = &mut self.color_overlay {
            o.opacity = unit(o.opacity, 1.0);
        }
        if let Some(o) = &mut self.gradient_overlay {
            o.gradient.sanitize();
            o.angle_deg = angle(o.angle_deg);
            o.opacity = unit(o.opacity, 1.0);
        }
        if let Some(b) = &mut self.bevel {
            b.size = length(b.size);
            b.depth = if b.depth.is_finite() { b.depth.clamp(0.0, 10.0) } else { 1.0 };
            b.angle_deg = angle(b.angle_deg);
            b.altitude_deg = if b.altitude_deg.is_finite() { b.altitude_deg.clamp(0.0, 90.0) } else { 30.0 };
            b.opacity = unit(b.opacity, 1.0);
        }
    }

    /// A sanitized copy.
    pub fn sanitized(&self) -> StyleStack {
        let mut s = self.clone();
        s.sanitize();
        s
    }

    /// Reach of the outline shape beyond the artwork, in reference pixels (contour, then an outside stroke around
    /// it), and whether an enabled style draws outside the artwork at all.
    fn outline_reach(&self) -> (f32, bool) {
        let contour = self.contour.as_ref().filter(|c| c.on).map(|c| c.width * (1.0 + c.smooth));
        let stroke = self.stroke.as_ref().filter(|s| s.on && s.position != StrokePos::Inside).map(|s| match s.position {
            StrokePos::Center => s.width / 2.0,
            _ => s.width,
        });
        let reach = contour.unwrap_or(0.0) + stroke.unwrap_or(0.0);
        (reach, contour.is_some() || stroke.is_some())
    }

    /// The pixels the styles need beyond the content on any side at `scale` (2.3), from the numbers alone (the stack
    /// is read sanitized). `ceil(scale * (outline reach + max(shadow reach, glow reach))) + 2`, where the outline
    /// reach is the contour's `width * (1 + smooth)` plus the outside part of the stroke (which runs around the
    /// contour), shadow reach = `distance + size`, glow reach = `size`. Zero when no enabled style draws outside
    /// (Inside / Center-inner stroke parts, inner shadow, inner glow, overlays and bevel never add to it).
    pub fn margin(&self, scale: f32) -> u32 {
        let s = self.sanitized();
        let scale = if scale.is_finite() { scale.max(0.0) } else { 0.0 };
        let (outline, mut outside) = s.outline_reach();
        let shadow = s.drop_shadow.as_ref().filter(|d| d.on).map(|d| d.distance + d.size);
        let glow = s.outer_glow.as_ref().filter(|g| g.on).map(|g| g.size);
        outside |= shadow.is_some() || glow.is_some();
        if !outside {
            return 0;
        }
        let soft = shadow.unwrap_or(0.0).max(glow.unwrap_or(0.0));
        let px = (scale * (outline + soft)).ceil();
        // Bounded: the lengths are at most 256 each; a huge scale is the caller's error, not an overflow.
        px.min(1.0e6) as u32 + 2
    }

    /// The stack with every length multiplied by `scale` (no clamping: the lengths of `apply_styles`'s working size).
    pub(crate) fn scaled(&self, scale: f32) -> StyleStack {
        let mut s = self.clone();
        if let Some(c) = &mut s.contour {
            c.width *= scale;
        }
        if let Some(st) = &mut s.stroke {
            st.width *= scale;
        }
        for sh in [&mut s.drop_shadow, &mut s.inner_shadow].into_iter().flatten() {
            sh.distance *= scale;
            sh.size *= scale;
        }
        for g in [&mut s.outer_glow, &mut s.inner_glow].into_iter().flatten() {
            g.size *= scale;
        }
        if let Some(b) = &mut s.bevel {
            b.size *= scale;
        }
        s
    }
}

/// The style presets of 2.3. Preset lengths are for a 512-px picture and are scaled by `max(content side) / 512`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StylePreset {
    /// White die-cut contour 12 px, smooth 0.5, and a black drop shadow at 40 %.
    ClassicSticker,
    /// Black outside stroke.
    Outline,
    /// Soft outer glow.
    Glow,
    /// White contour, dark stroke around it, bevel and a shadow.
    Badge,
    /// A drop shadow only.
    ShadowOnly,
    /// No styles.
    Clear,
}

impl StylePreset {
    /// Every preset in menu order.
    pub const ALL: [StylePreset; 6] = [
        StylePreset::ClassicSticker,
        StylePreset::Outline,
        StylePreset::Glow,
        StylePreset::Badge,
        StylePreset::ShadowOnly,
        StylePreset::Clear,
    ];

    /// Menu name.
    pub fn name(self) -> &'static str {
        match self {
            StylePreset::ClassicSticker => "Classic sticker",
            StylePreset::Outline => "Outline",
            StylePreset::Glow => "Glow",
            StylePreset::Badge => "Badge",
            StylePreset::ShadowOnly => "Shadow only",
            StylePreset::Clear => "Clear",
        }
    }

    /// The preset for content of size `content` (pixels at reference scale): lengths multiplied by
    /// `max(content) / 512`, then sanitized.
    pub fn styles(self, content: [u32; 2]) -> StyleStack {
        let k = content[0].max(content[1]).max(1) as f32 / 512.0;
        let shadow = |opacity: f32, distance: f32, size: f32| Shadow {
            on: true,
            color: [0, 0, 0, 255],
            opacity,
            angle_deg: 120.0,
            distance: distance * k,
            spread: 0.0,
            size: size * k,
        };
        let mut s = match self {
            StylePreset::ClassicSticker => StyleStack {
                contour: Some(Contour {
                    on: true,
                    width: 12.0 * k,
                    smooth: 0.5,
                    fill_holes: true,
                    color: [255, 255, 255, 255],
                    opacity: 1.0,
                }),
                drop_shadow: Some(shadow(0.4, 6.0, 10.0)),
                ..StyleStack::default()
            },
            StylePreset::Outline => StyleStack {
                stroke: Some(Stroke {
                    on: true,
                    width: 6.0 * k,
                    position: StrokePos::Outside,
                    color: [0, 0, 0, 255],
                    opacity: 1.0,
                }),
                ..StyleStack::default()
            },
            StylePreset::Glow => StyleStack {
                outer_glow: Some(Glow { on: true, color: [255, 240, 160, 255], opacity: 0.8, spread: 0.1, size: 24.0 * k }),
                ..StyleStack::default()
            },
            StylePreset::Badge => StyleStack {
                contour: Some(Contour {
                    on: true,
                    width: 16.0 * k,
                    smooth: 0.7,
                    fill_holes: true,
                    color: [255, 255, 255, 255],
                    opacity: 1.0,
                }),
                stroke: Some(Stroke {
                    on: true,
                    width: 4.0 * k,
                    position: StrokePos::Outside,
                    color: [40, 40, 40, 255],
                    opacity: 1.0,
                }),
                drop_shadow: Some(shadow(0.35, 4.0, 8.0)),
                bevel: Some(Bevel {
                    on: true,
                    size: 6.0 * k,
                    depth: 1.0,
                    angle_deg: 120.0,
                    altitude_deg: 30.0,
                    highlight: [255, 255, 255, 255],
                    shadow: [0, 0, 0, 255],
                    opacity: 0.5,
                }),
                ..StyleStack::default()
            },
            StylePreset::ShadowOnly => StyleStack { drop_shadow: Some(shadow(0.5, 8.0, 12.0)), ..StyleStack::default() },
            StylePreset::Clear => StyleStack::default(),
        };
        s.sanitize();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k1_gradient_endpoints_and_interpolation() {
        let g = Gradient::two([0, 10, 200, 255], [255, 20, 100, 0]);
        assert_eq!(g.sample(0.0), [0, 10, 200, 255]);
        assert_eq!(g.sample(1.0), [255, 20, 100, 0]);
        assert_eq!(g.sample(-3.0), [0, 10, 200, 255]);
        assert_eq!(g.sample(7.0), [255, 20, 100, 0]);
        assert_eq!(g.sample(f32::NAN), [0, 10, 200, 255]);
        // Halfway: 127.5 -> 128 (half up), 15, 150, 127.5 -> 128.
        assert_eq!(g.sample(0.5), [128, 15, 150, 128]);
        let three = Gradient { stops: vec![(0.0, [0; 4]), (0.25, [100, 100, 100, 100]), (1.0, [250, 250, 250, 250])] };
        assert_eq!(three.sample(0.25), [100, 100, 100, 100]);
        assert_eq!(three.sample(0.125), [50, 50, 50, 50]);
        assert_eq!(three.sample(0.625), [175, 175, 175, 175]);
        assert_eq!(Gradient { stops: vec![] }.sample(0.3), [0; 4]);
    }

    #[test]
    fn k1_gradient_and_stack_sanitize() {
        let mut g = Gradient { stops: vec![(f32::NAN, [1; 4]), (2.0, [2; 4]), (0.5, [3; 4])] };
        g.sanitize();
        assert_eq!(g.stops, vec![(0.0, [1; 4]), (0.5, [3; 4]), (1.0, [2; 4])]);
        let mut many = Gradient { stops: (0..12).map(|i| (i as f32 / 11.0, [i as u8; 4])).collect() };
        many.sanitize();
        assert_eq!(many.stops.len(), MAX_GRADIENT_STOPS);
        let mut one = Gradient { stops: vec![(0.3, [9; 4])] };
        one.sanitize();
        assert_eq!(one, Gradient::two([9; 4], [9; 4]));

        let mut s = StylePreset::Badge.styles([512, 512]);
        s.contour.as_mut().unwrap().width = f32::INFINITY;
        s.stroke.as_mut().unwrap().width = 1.0e9;
        s.drop_shadow.as_mut().unwrap().opacity = f32::NAN;
        s.drop_shadow.as_mut().unwrap().spread = -2.0;
        s.bevel.as_mut().unwrap().depth = 99.0;
        s.bevel.as_mut().unwrap().altitude_deg = 300.0;
        s.sanitize();
        assert_eq!(s.contour.as_ref().unwrap().width, 0.0);
        assert_eq!(s.stroke.as_ref().unwrap().width, MAX_STYLE_LENGTH);
        assert_eq!(s.drop_shadow.as_ref().unwrap().opacity, 1.0);
        assert_eq!(s.drop_shadow.as_ref().unwrap().spread, 0.0);
        assert_eq!(s.bevel.as_ref().unwrap().depth, 10.0);
        assert_eq!(s.bevel.as_ref().unwrap().altitude_deg, 90.0);
    }

    #[test]
    fn k1_default_and_margin_rules() {
        let d = StyleStack::default();
        assert!(d.is_default() && !d.any_on());
        assert_eq!(d.margin(1.0), 0);
        // An entry that is off is "used" but draws nothing and needs no margin.
        let mut off = StylePreset::ClassicSticker.styles([512, 512]);
        off.contour.as_mut().unwrap().on = false;
        off.drop_shadow.as_mut().unwrap().on = false;
        assert!(!off.is_default() && !off.any_on());
        assert_eq!(off.margin(1.0), 0);
        // Inside styles never add to the margin.
        let inside = StyleStack {
            stroke: Some(Stroke { on: true, width: 30.0, position: StrokePos::Inside, color: [0; 4], opacity: 1.0 }),
            inner_shadow: StylePreset::ShadowOnly.styles([512, 512]).drop_shadow,
            inner_glow: Some(Glow { on: true, color: [0; 4], opacity: 1.0, spread: 0.0, size: 40.0 }),
            color_overlay: Some(ColorOverlay { on: true, color: [1; 4], opacity: 1.0 }),
            bevel: StylePreset::Badge.styles([512, 512]).bevel,
            ..StyleStack::default()
        };
        assert_eq!(inside.margin(3.0), 0);
        // Contour 12 * 1.5 = 18, shadow 6 + 10 = 16: ceil(34) + 2.
        let classic = StylePreset::ClassicSticker.styles([512, 512]);
        assert_eq!(classic.margin(1.0), 36);
        assert_eq!(classic.margin(0.5), 19);
        // Preset lengths follow the content size.
        assert_eq!(StylePreset::ClassicSticker.styles([1024, 300]).contour.unwrap().width, 24.0);
        // The outside stroke runs around the contour: the two reaches add up.
        let mut both = classic.clone();
        both.drop_shadow = None;
        both.stroke = Some(Stroke { on: true, width: 4.0, position: StrokePos::Center, color: [0; 4], opacity: 1.0 });
        assert_eq!(both.margin(1.0), 18 + 2 + 2);
        assert_eq!(both.margin(f32::NAN), 2);
        assert!(StylePreset::Clear.styles([10, 10]).is_default());
        for p in StylePreset::ALL {
            assert!(!p.name().is_empty());
            let s = p.styles([300, 200]);
            assert_eq!(s, s.sanitized());
        }
    }

    #[test]
    fn k1_light_offsets() {
        assert_eq!(light_offset(0.0, 10.0), (-10, 0));
        assert_eq!(light_offset(90.0, 10.0), (0, 10));
        assert_eq!(light_offset(120.0, 10.0), (5, 9));
        assert_eq!(light_offset(270.0, 10.0), (0, -10));
    }
}
