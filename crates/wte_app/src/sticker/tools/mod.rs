// Owner: stickers P7a (STICKER_PLAN 3.3, 3.4, 3.10; ARCHITECTURE 8.15). The paint tools of the Sticker view: Brush and
// Eraser (`brush.rs`: strokes through `Document::sticker_stroke_*`), Fill (`fill.rs`), Gradient (`gradient.rs`) and
// Eyedropper (`eyedropper.rs`). The canvas (`canvas.rs`) routes the pointer to them; Fill and Gradient end in a
// `StickerAction` (one entry each through `Document::replace_sticker_pixels`).
//
// `paint_target` holds the rules every paint tool shares (3.3 "Layer-kind rules"): a picture layer (a text or shape
// layer offers Rasterize, never a silent rasterize), not locked, shown; the buffer is the picture or, with "Paint on
// mask" and a mask, the mask; the selection limits the change (3.4), seen from the layer's content pixels
// (`selection_in_content`, so a moved, scaled or turned layer is limited where the selection is on the artboard).
//
// Stickers P7b: the selection tools Marquee (`marquee.rs`), Lasso (`lasso.rs`) and Magic Wand (`wand.rs`, also Color
// Range), the Magic Eraser (`magic_eraser.rs`), Move Selection and Layer via Copy / Cut (`move_selection.rs`), and the
// helpers they share here: the artboard of the open editor, `set_selection` (a new mask combined by a mode), and
// `put` / `put_new_mask` (one entry for a changed buffer, nothing for an unchanged one).

use std::sync::Arc;

use glam::Vec2;
use wte_core::{
    AlphaBuf, Commit, IRect, LayerMask, LayerTransform, PixelData, PixelTarget, SelectMode, Selection, StickerContent, StickerLayerId,
    StickerRef,
};

use crate::state::{AppState, StickerTool};
use crate::sticker::{Shown, edit_of, edit_ref, shown};

pub mod brush;
pub mod crop;
pub mod eyedropper;
pub mod fill;
pub mod gradient;
pub mod lasso;
pub mod magic_eraser;
pub mod marquee;
pub mod move_selection;
pub mod shape;
pub mod text;
pub mod transform;
pub mod wand;

/// The words of the Rasterize offer (status line, toast and the header).
pub(crate) const RASTERIZE_HINT: &str = "Rasterize this layer to paint on it";
/// Status line of a paint tool on a hidden layer.
pub(crate) const HIDDEN_HINT: &str = "The layer is hidden: show it first";
/// Status line of a paint tool on a locked layer.
pub(crate) const LOCKED_HINT: &str = "The layer is locked: unlock it first";
/// Status line of a paint tool on a text stamp.
pub(crate) const TEXT_STAMP_HINT: &str = "A text stamp is edited in the Text tab; convert it to a layered sticker to paint on it";

/// The buffer a paint tool changes, resolved before anything changes.
#[derive(Clone, Debug)]
pub(crate) struct PaintTarget {
    /// What the document command gets (an image stamp is promoted inside it).
    pub(crate) on: StickerRef,
    pub(crate) layer: StickerLayerId,
    /// The picture, or its mask ("Paint on mask" with a mask).
    pub(crate) target: PixelTarget,
    /// The buffer's size (content pixels).
    pub(crate) size: [u32; 2],
    pub(crate) transform: LayerTransform,
    pub(crate) artboard: IRect,
    /// The layer has an enabled style (a stroke runs in stroke mode).
    pub(crate) styled: bool,
}

impl PaintTarget {
    /// Layer space -> content pixels of the buffer (continuous).
    pub(crate) fn to_content(&self, p: Vec2) -> Vec2 {
        self.transform.invert(self.size, p)
    }
}

/// The paint target of the active layer for `tool`, or `None` after saying why (status line; a text or shape layer
/// also gets the warning toast and the Rasterize offer). `ask`: a content edit of a shared sticker asks the E6
/// question first (strokes; Fill and Gradient asked in `run_in` already).
pub(crate) fn paint_target(state: &mut AppState, tool: StickerTool, ask: bool) -> Option<PaintTarget> {
    let edit = edit_of(state)?.clone();
    let on = edit_ref(&edit)?;
    let session = state.session.as_mut()?;
    let shown = shown(&mut session.doc, &edit)?;
    if let Shown::Text { .. } = shown {
        state.status_hint = TEXT_STAMP_HINT.to_owned();
        return None;
    }
    let doc = shown.layers()?;
    let Some(layer) = edit.active_layer.and_then(|id| doc.layer(id)).cloned() else {
        state.status_hint = "Pick a layer in the Layers tab first".to_owned();
        return None;
    };
    let artboard = doc.artboard;
    if !matches!(layer.content, StickerContent::Picture(_)) {
        if tool.paints() {
            if state.sticker_ui.rasterize_offer != Some(layer.id) {
                state.toasts.warn(RASTERIZE_HINT);
            }
            state.sticker_ui.rasterize_offer = Some(layer.id);
            state.status_hint = RASTERIZE_HINT.to_owned();
        }
        return None;
    }
    if layer.locked {
        state.status_hint = LOCKED_HINT.to_owned();
        return None;
    }
    if !layer.visible {
        state.status_hint = HIDDEN_HINT.to_owned();
        return None;
    }
    if ask && let Some(users) = crate::sticker::shared_users(state) {
        crate::sticker::ask_shared_for(state, None, users);
        return None;
    }
    let mask = state.sticker_ui.paint_on_mask && layer.mask.is_some();
    let target = if mask { PixelTarget::Mask } else { PixelTarget::Pixels };
    let doc = state.doc_mut()?;
    let pixels = match doc.sticker_layer_pixels(on, layer.id) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return None;
        }
    };
    Some(PaintTarget {
        on,
        layer: layer.id,
        target,
        size: [pixels.width, pixels.height],
        transform: layer.transform,
        artboard,
        styled: layer.styles.any_on(),
    })
}

/// The open editor's selection seen from the target's content pixels (`None` without a selection).
pub(crate) fn selection_for(state: &AppState, t: &PaintTarget) -> Option<Arc<AlphaBuf>> {
    let sel = edit_of(state)?.selection.as_ref()?;
    Some(Arc::new(wte_core::sticker::select::selection_in_content(&sel.mask, t.artboard, &t.transform, t.size)))
}

/// Stickers P7b: the open editor's artboard as its top-left corner (layer space) and its size; `None` for a text stamp
/// or without an editor.
pub(crate) fn artboard_of(state: &mut AppState) -> Option<(Vec2, [u32; 2])> {
    let edit = edit_of(state)?.clone();
    let session = state.session.as_mut()?;
    let shown = shown(&mut session.doc, &edit)?;
    let Some(doc) = shown.layers() else {
        state.status_hint = TEXT_STAMP_HINT.to_owned();
        return None;
    };
    Some((Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32), doc.size()))
}

/// Stickers P7b: a new selection mask (artboard size) combined with the selection there is by `mode` (New replaces;
/// Add, Subtract and Intersect without a selection start from nothing). A result without any selected pixel drops the
/// selection. App state: no entry.
pub(crate) fn set_selection(state: &mut AppState, mask: AlphaBuf, mode: SelectMode) {
    let Some(edit) = crate::sticker::edit_mut(state) else { return };
    let size = [mask.width, mask.height];
    let current = edit.selection.as_ref().filter(|s| s.size() == size);
    let result = match (mode, current) {
        (SelectMode::Replace, _) | (SelectMode::Add, None) => Selection::from_mask(mask),
        (SelectMode::Subtract | SelectMode::Intersect, None) => Selection::none(size[0], size[1]),
        (mode, Some(cur)) => {
            let mut s = (**cur).clone();
            s.combine(&mask, mode);
            s
        }
    };
    edit.selection = (!result.is_empty()).then(|| Arc::new(result));
}

/// Stickers P7b: `coverage` multiplied by `sel` (same size; another size leaves nothing).
pub(crate) fn limit(coverage: &mut AlphaBuf, sel: &AlphaBuf) {
    if coverage.data.len() != sel.data.len() {
        coverage.data.fill(0);
        return;
    }
    for (c, &s) in coverage.data.iter_mut().zip(&sel.data) {
        *c = wte_core::mul_u8(*c, s);
    }
}

/// Stickers P7b: puts `new` into the target's picture or mask as one entry `label` (`replace_sticker_pixels`); an
/// unchanged buffer makes no entry (and does not promote an image stamp) and says so. True when applied.
pub(crate) fn put(state: &mut AppState, t: &PaintTarget, new: PixelData, label: &str) -> bool {
    let Some(doc) = state.doc_mut() else { return false };
    let same = match &new {
        PixelData::Rgba(p) => doc.sticker_layer_pixels(t.on, t.layer).is_ok_and(|old| *old == *p),
        PixelData::Alpha(m) => doc.sticker_layer_mask(t.on, t.layer).is_ok_and(|old| old.is_some_and(|o| *o == *m)),
    };
    if same {
        state.status_hint = format!("{label}: nothing changed");
        return false;
    }
    let target = new.target();
    let r = doc.replace_sticker_pixels(t.on, t.layer, target, new, label);
    let done = crate::dispatch::edited(state, r).is_some();
    if done {
        state.status_hint.clear();
    }
    done
}

/// Stickers P7b: gives the target's layer the mask `mask` (it has none yet) as one entry `label`.
pub(crate) fn put_new_mask(state: &mut AppState, t: &PaintTarget, mask: AlphaBuf, label: &str) -> bool {
    let layer = t.layer;
    let Some(doc) = state.doc_mut() else { return false };
    let r = doc.set_sticker(
        t.on,
        move |s| {
            if let Some(l) = s.layer_mut(layer) {
                l.mask = Some(LayerMask { pixels: Arc::new(mask), version: 0 });
            }
        },
        label,
        Commit::Final,
    );
    let done = crate::dispatch::edited(state, r).is_some();
    if done {
        state.status_hint.clear();
    }
    done
}

/// Rec. 601 grey level of a colour (the value a paint tool puts into a mask: white shows, black hides).
pub(crate) fn grey(c: [u8; 4]) -> u8 {
    ((299 * c[0] as u32 + 587 * c[1] as u32 + 114 * c[2] as u32 + 500) / 1000) as u8
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_p7b;
