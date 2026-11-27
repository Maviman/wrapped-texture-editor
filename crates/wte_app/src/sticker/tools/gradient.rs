// Owner: stickers P7a (STICKER_PLAN 3.3 row Gradient, 4.9). The Gradient tool: a drag draws a linear gradient on the
// active picture layer (or its mask) from the press point to the release point, limited by the selection; one entry
// "Gradient". Options (`StickerUi::gradient`): Primary to Secondary, Primary to Transparent, or the stops of the
// gradient editor (2..=8); Opacity; Reverse.

use glam::Vec2;
use wte_core::sticker::filter::{draw_gradient, draw_gradient_mask};
use wte_core::{Gradient, MAX_GRADIENT_STOPS, PixelData, PixelTarget};

use crate::state::{AppState, StickerTool};

/// History label of the Gradient tool.
pub(crate) const GRADIENT_LABEL: &str = "Gradient";
/// A drag shorter than this (layer-space pixels) draws nothing.
pub(crate) const MIN_DRAG: f32 = 0.5;

/// What the Gradient tool draws.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GradientMode {
    PrimaryToSecondary,
    PrimaryToTransparent,
    /// The stops of the gradient editor.
    Custom,
}

impl GradientMode {
    pub const ALL: [GradientMode; 3] = [GradientMode::PrimaryToSecondary, GradientMode::PrimaryToTransparent, GradientMode::Custom];

    pub fn label(self) -> &'static str {
        match self {
            GradientMode::PrimaryToSecondary => "Primary to secondary",
            GradientMode::PrimaryToTransparent => "Primary to transparent",
            GradientMode::Custom => "Custom stops",
        }
    }
}

/// The Gradient tool's options.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientOptions {
    pub mode: GradientMode,
    /// 0..=1.
    pub opacity: f32,
    pub reverse: bool,
    /// The editor's stops (position 0..=1, straight RGBA), 2..=`MAX_GRADIENT_STOPS`, kept sorted by position.
    pub stops: Vec<(f32, [u8; 4])>,
}

impl Default for GradientOptions {
    /// Primary to secondary, opacity 1, not reversed; the editor starts with black to white.
    fn default() -> GradientOptions {
        GradientOptions {
            mode: GradientMode::PrimaryToSecondary,
            opacity: 1.0,
            reverse: false,
            stops: vec![(0.0, [0, 0, 0, 255]), (1.0, [255, 255, 255, 255])],
        }
    }
}

impl GradientOptions {
    /// The gradient drawn with the primary and secondary colours `primary`, `secondary`: reversed when asked, the stops
    /// sorted and clamped, at least two, at most `MAX_GRADIENT_STOPS`.
    pub fn gradient(&self, primary: [u8; 4], secondary: [u8; 4]) -> Gradient {
        let mut stops = match self.mode {
            GradientMode::PrimaryToSecondary => vec![(0.0, primary), (1.0, secondary)],
            GradientMode::PrimaryToTransparent => vec![(0.0, primary), (1.0, [primary[0], primary[1], primary[2], 0])],
            GradientMode::Custom => self.stops.clone(),
        };
        for s in &mut stops {
            s.0 = if s.0.is_finite() { s.0.clamp(0.0, 1.0) } else { 0.0 };
        }
        stops.truncate(MAX_GRADIENT_STOPS);
        if stops.len() < 2 {
            stops = vec![(0.0, primary), (1.0, secondary)];
        }
        if self.reverse {
            for s in &mut stops {
                s.0 = 1.0 - s.0;
            }
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Gradient { stops }
    }

    /// Adds a stop half way into the widest gap (at most `MAX_GRADIENT_STOPS`), with the colour there.
    pub fn add_stop(&mut self) {
        if self.stops.len() >= MAX_GRADIENT_STOPS {
            return;
        }
        self.stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let Some((i, _)) = self.stops.windows(2).enumerate().max_by(|a, b| (a.1[1].0 - a.1[0].0).total_cmp(&(b.1[1].0 - b.1[0].0))) else {
            return;
        };
        let t = (self.stops[i].0 + self.stops[i + 1].0) * 0.5;
        let c = Gradient { stops: self.stops.clone() }.sample(t);
        self.stops.insert(i + 1, (t, c));
    }

    /// Removes stop `i` while more than two are left.
    pub fn remove_stop(&mut self, i: usize) {
        if self.stops.len() > 2 && i < self.stops.len() {
            self.stops.remove(i);
        }
    }
}

/// The Gradient tool's drag from `from` to `to` (layer space; `StickerAction::Gradient`).
pub(crate) fn apply(state: &mut AppState, from: Vec2, to: Vec2) {
    if !(from.is_finite() && to.is_finite()) || from.distance(to) < MIN_DRAG {
        return;
    }
    let Some(t) = super::paint_target(state, StickerTool::Gradient, false) else { return };
    let selection = super::selection_for(state, &t);
    let options = state.sticker_ui.gradient.clone();
    let gradient = options.gradient(state.colors.primary, state.colors.secondary);
    let (a, b) = (t.to_content(from), t.to_content(to));
    let Some(doc) = state.doc_mut() else { return };
    let new = match t.target {
        PixelTarget::Pixels => match doc.sticker_layer_pixels(t.on, t.layer) {
            Ok(p) => {
                let mut out = (*p).clone();
                draw_gradient(&mut out, a, b, &gradient, options.opacity, selection.as_deref());
                PixelData::Rgba(out)
            }
            Err(e) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
        },
        PixelTarget::Mask => match doc.sticker_layer_mask(t.on, t.layer) {
            Ok(Some(m)) => {
                let mut out = (*m).clone();
                draw_gradient_mask(&mut out, a, b, &gradient, options.opacity, selection.as_deref());
                PixelData::Alpha(out)
            }
            Ok(None) => return,
            Err(e) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
        },
    };
    let Some(doc) = state.doc_mut() else { return };
    let r = doc.replace_sticker_pixels(t.on, t.layer, t.target, new, GRADIENT_LABEL);
    if crate::dispatch::edited(state, r).is_some() {
        state.status_hint.clear();
    }
}
