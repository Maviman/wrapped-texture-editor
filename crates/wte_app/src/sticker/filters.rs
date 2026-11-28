// Owner: stickers P7b (STICKER_PLAN 3.3 "Commands", 3.4, 4.5, 4.9; ARCHITECTURE 8.15). The Cut-out commands (Remove
// Background with the Tool tab's options, Color to Alpha, Defringe, Remove White / Black Matte) and the Filter commands
// (Blur, Sharpen) on the active picture layer, limited by the selection (seen from the layer's pixels), each one entry
// through `Document::replace_sticker_pixels` (a tile patch of the changed tiles; an image stamp is promoted inside it).
// `compute` is shared by the command and by its popover's live preview (`popover.rs`), so the preview is exactly what
// Apply gives.
//
// Also here: `COMMANDS`, the one list of the Selection, Cut-out and Filter commands that the Sticker menu and the Tool
// tab's command blocks draw (label, shortcut, what it needs, the action).
//
// Stickers P8: the list also has the Sticker group (Trim..., Add Padding..., Canvas Size..., Resize..., Rotate 90 CW
// / CCW / 180, Rotate..., Flip Horizontal / Vertical, Flatten, Export as PSD..., Save to Library) and the Layer group
// (New Picture Layer, Add Picture..., Text, Shape, Duplicate, Delete, Merge Down, Rasterize, Clip to Layer Below,
// Bring Forward / Send Backward, Copy / Paste / Clear Layer Style), appended after the P7b entries (their indexes
// stay).

use wte_core::sticker::{cutout, filter, select};
use std::sync::Arc;

use wte_core::{AlphaBuf, BackgroundCut, Matte, PixelBuf, PixelData, PixelTarget, RemoveBgOptions, Selection, StickerContent};

use crate::state::{AppState, StickerTool};
use crate::sticker::popover::PopKind;
use crate::sticker::tools::{self, PaintTarget};
use crate::sticker::layout::Layout;
use crate::sticker::{StickerAction, StyleTarget, commands::Modify, edit_of, layered};

/// The largest picture side Blur and Sharpen work on (their intermediates are 8 bytes a pixel, three planes).
pub(crate) const MAX_FILTER_SIDE: u32 = 4096;

/// Remove Background's options in the Tool tab (`StickerUi::remove_bg`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct RemoveBgUi {
    pub opts: RemoveBgOptions,
    /// Hide the background on the layer mask; the pixels are kept.
    pub into_mask: bool,
}

/// One Cut-out or Filter command with its numbers.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum CutOut {
    RemoveBackground(RemoveBgUi),
    ColorToAlpha([u8; 3]),
    RemoveMatte(Matte),
    Defringe(u32),
    Blur(f32),
    /// Amount 0..=3 (300 %), radius.
    Sharpen(f32, f32),
}

impl CutOut {
    /// The history label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            CutOut::RemoveBackground(_) => "Remove Background",
            CutOut::ColorToAlpha(_) => "Color to Alpha",
            CutOut::RemoveMatte(Matte::White) => "Remove White Matte",
            CutOut::RemoveMatte(Matte::Black) => "Remove Black Matte",
            CutOut::Defringe(_) => "Defringe",
            CutOut::Blur(_) => "Blur",
            CutOut::Sharpen(..) => "Sharpen",
        }
    }
}

/// What a command changes.
pub(crate) enum Change {
    /// New pixels or a new mask of the same layer.
    Data(PixelData),
    /// A mask for a layer that had none (Remove Background "Into mask").
    NewMask(AlphaBuf),
}

/// P7b check: the inputs of a Cut-out or Filter command, read on the UI thread without copying a buffer (`Arc`s);
/// `math` works on any thread (the popovers' live preview runs it on a job, `popover.rs`).
pub(crate) struct Prepared {
    pub(crate) t: PaintTarget,
    cut: CutOut,
    pixels: Arc<PixelBuf>,
    mask: Option<Arc<AlphaBuf>>,
    /// The editor's selection (artboard pixels); `math` sees it from the layer's content pixels.
    selection: Option<Arc<Selection>>,
}

const _: fn() = || {
    fn send_static<T: Send + 'static>() {}
    send_static::<Prepared>();
};

/// The inputs of `cut` on the active picture layer (`None` after saying why).
pub(crate) fn prepare(state: &mut AppState, cut: CutOut) -> Option<Prepared> {
    let t = tools::paint_target(state, StickerTool::Brush, false)?;
    let selection = edit_of(state).and_then(|e| e.selection.clone());
    if matches!(cut, CutOut::Blur(_) | CutOut::Sharpen(..)) && t.size.iter().any(|&s| s > MAX_FILTER_SIDE) {
        state.status_hint = format!("{} works on pictures up to {MAX_FILTER_SIDE} pixels a side", cut.label());
        return None;
    }
    let doc = state.doc_mut()?;
    let pixels = match doc.sticker_layer_pixels(t.on, t.layer) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return None;
        }
    };
    let mask = doc.sticker_layer_mask(t.on, t.layer).ok().flatten();
    Some(Prepared { t, cut, pixels, mask, selection })
}

impl Prepared {
    /// What the command changes (Remove Background can refuse).
    pub(crate) fn math(&self) -> Result<Change, cutout::RemoveBgRefusal> {
        let t = &self.t;
        let selection = self.selection.as_ref().map(|s| select::selection_in_content(&s.mask, t.artboard, &t.transform, t.size));
        let sel = selection.as_ref();
        let (pixels, mask) = (&self.pixels, &self.mask);
        Ok(match self.cut {
            CutOut::RemoveBackground(ui) => {
                let found = cutout::remove_background(pixels, ui.opts)?;
                apply_cut(&found, pixels, mask.as_deref(), t.size, sel, ui.into_mask)
            }
            CutOut::ColorToAlpha(key) => {
                let mut out = (**pixels).clone();
                cutout::color_to_alpha(&mut out, key, 0.0, 1.0, sel);
                Change::Data(PixelData::Rgba(out))
            }
            CutOut::RemoveMatte(m) => {
                let mut out = (**pixels).clone();
                cutout::remove_matte(&mut out, m, sel);
                Change::Data(PixelData::Rgba(out))
            }
            CutOut::Defringe(width) => {
                let mut out = (**pixels).clone();
                cutout::defringe(&mut out, width.clamp(1, cutout::MAX_DEFRINGE), sel);
                Change::Data(PixelData::Rgba(out))
            }
            CutOut::Blur(radius) => {
                let radius = clean(radius, 0.5, filter::MAX_BLUR_RADIUS);
                match (t.target, mask) {
                    (PixelTarget::Mask, Some(m)) => Change::Data(PixelData::Alpha(filter::blur_mask(m, radius, sel))),
                    _ => Change::Data(PixelData::Rgba(filter::blur(pixels, radius, sel))),
                }
            }
            CutOut::Sharpen(amount, radius) => {
                let amount = clean(amount, 0.0, filter::MAX_SHARPEN_AMOUNT);
                let radius = clean(radius, 0.5, filter::MAX_SHARPEN_RADIUS);
                Change::Data(PixelData::Rgba(filter::sharpen(pixels, amount, radius, sel)))
            }
        })
    }
}

/// What a found cut (Remove Background, stickers S1: the Cut Out Box) changes: the pixels' alpha (and the soft
/// edge's colours), or with `into_mask` the layer mask (a new one, all shown, when the layer has none); limited by
/// `sel` (content pixels).
pub(crate) fn apply_cut(found: &BackgroundCut, pixels: &PixelBuf, mask: Option<&AlphaBuf>, size: [u32; 2], sel: Option<&AlphaBuf>, into_mask: bool) -> Change {
    if into_mask {
        match mask {
            Some(m) => {
                let mut out = m.clone();
                found.apply_to_mask(&mut out, sel);
                Change::Data(PixelData::Alpha(out))
            }
            None => {
                let mut out = AlphaBuf::filled(size[0], size[1], 255);
                found.apply_to_mask(&mut out, sel);
                Change::NewMask(out)
            }
        }
    } else {
        let mut out = pixels.clone();
        found.apply(&mut out, sel);
        Change::Data(PixelData::Rgba(out))
    }
}

/// The result of `cut` on the active picture layer, without changing anything (`None` after saying why).
pub(crate) fn compute(state: &mut AppState, cut: CutOut) -> Option<(PaintTarget, Change)> {
    let prepared = prepare(state, cut)?;
    match prepared.math() {
        Ok(change) => Some((prepared.t, change)),
        Err(refusal) => {
            state.status_hint = refusal.message().to_owned();
            state.toasts.info(refusal.message());
            None
        }
    }
}

/// A finite value clamped to `lo..=hi` (`lo` for NaN).
pub(crate) fn clean(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}

/// Runs `cut` on the active picture layer: one entry, or nothing (and the status line says why).
pub(crate) fn run(state: &mut AppState, cut: CutOut) {
    let Some((t, change)) = compute(state, cut) else { return };
    match change {
        Change::Data(data) => tools::put(state, &t, data, cut.label()),
        Change::NewMask(mask) => tools::put_new_mask(state, &t, mask, cut.label()),
    };
}

// ---------------------------------------------------------------------------------------------------------------
// The command list of the Sticker menu and the Tool tab
// ---------------------------------------------------------------------------------------------------------------

/// The group of a command (a submenu of the Sticker menu, a block of the Tool tab).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Selection,
    CutOut,
    Filter,
    /// Stickers P8.
    Sticker,
    Layer,
}

impl Group {
    /// The Tool tab's blocks in order (the P7b blocks first: their places stay).
    pub(crate) const ALL: [Group; 5] = [Group::Selection, Group::CutOut, Group::Filter, Group::Sticker, Group::Layer];
    /// The Sticker menu's submenus in order.
    pub(crate) const MENU: [Group; 5] = [Group::Sticker, Group::Layer, Group::Selection, Group::CutOut, Group::Filter];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Group::Selection => "Selection",
            Group::CutOut => "Cut-out",
            Group::Filter => "Filter",
            Group::Sticker => "Sticker",
            Group::Layer => "Layer",
        }
    }
}

/// What a command needs to be enabled.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Needs {
    /// A layered sticker in the editor.
    Layers,
    /// ... and a selection.
    Selection,
    /// ... and a picture layer as the active layer.
    Picture,
    /// ... and both.
    PictureAndSelection,
    /// Stickers P8: ... and a layer below the active one (Merge Down, Clip to Layer Below).
    Below,
    /// ... and a text or shape layer as the active layer (Rasterize).
    Vector,
}

/// One entry of the Sticker menu / the Tool tab's command blocks.
pub(crate) struct CommandDef {
    pub(crate) group: Group,
    pub(crate) label: &'static str,
    /// The `shortcuts::sticker_shortcut_text` label of its key ("" = none).
    pub(crate) shortcut: &'static str,
    pub(crate) needs: Needs,
    pub(crate) hover: &'static str,
    pub(crate) make: fn(&AppState) -> StickerAction,
}

const fn cmd(group: Group, label: &'static str, shortcut: &'static str, needs: Needs, hover: &'static str, make: fn(&AppState) -> StickerAction) -> CommandDef {
    CommandDef { group, label, shortcut, needs, hover, make }
}

/// Every Selection, Cut-out and Filter command, in menu order (`PanelHit::StickerCommand` takes the index).
pub(crate) static COMMANDS: [CommandDef; 46] = [
    cmd(Group::Selection, "All", "Select All", Needs::Layers, "Select the whole artboard", |_| StickerAction::SelectAll),
    cmd(Group::Selection, "None", "Select None", Needs::Selection, "Drop the selection", |_| StickerAction::SelectNone),
    cmd(Group::Selection, "Invert", "Invert Selection", Needs::Layers, "Select what is not selected", |_| StickerAction::InvertSelection),
    cmd(Group::Selection, "Grow...", "", Needs::Selection, "Make the selection larger by some pixels", |_| {
        StickerAction::OpenPopover(PopKind::Modify { how: Modify::Grow, px: 4.0 })
    }),
    cmd(Group::Selection, "Shrink...", "", Needs::Selection, "Make the selection smaller by some pixels", |_| {
        StickerAction::OpenPopover(PopKind::Modify { how: Modify::Shrink, px: 4.0 })
    }),
    cmd(Group::Selection, "Feather...", "", Needs::Selection, "Soften the selection's edge", |_| {
        StickerAction::OpenPopover(PopKind::Modify { how: Modify::Feather, px: 4.0 })
    }),
    cmd(Group::Selection, "Color Range...", "", Needs::Layers, "Select every pixel of a colour (click the canvas to pick it)", |s| {
        StickerAction::OpenPopover(PopKind::ColorRange {
            color: s.colors.primary,
            tolerance: select::DEFAULT_TOLERANCE,
            all_layers: false,
            mode: s.sticker_ui.select_mode,
        })
    }),
    cmd(Group::Selection, "From Layer Alpha", "", Needs::Layers, "Select the visible pixels of the active layer", |_| {
        StickerAction::SelectionFromAlpha
    }),
    cmd(Group::Selection, "Delete", "Sticker Delete", Needs::PictureAndSelection, "Clear the selected pixels of the active layer", |_| {
        StickerAction::DeleteSelected
    }),
    cmd(Group::Selection, "Crop to Selection", "", Needs::Selection, "Make the artboard the selection's bounds", |_| {
        StickerAction::CropToSelection
    }),
    cmd(Group::Selection, "Layer via Copy", "", Needs::PictureAndSelection, "The selected pixels as a new layer", |_| StickerAction::LayerViaCopy),
    cmd(Group::Selection, "Layer via Cut", "", Needs::PictureAndSelection, "The selected pixels moved to a new layer", |_| StickerAction::LayerViaCut),
    cmd(Group::CutOut, "Remove Background", "", Needs::Picture, "Clear a plain background (the options above)", |_| StickerAction::RemoveBackground),
    cmd(Group::CutOut, "Color to Alpha...", "", Needs::Picture, "Make one colour transparent, keeping soft edges", |_| {
        StickerAction::OpenPopover(PopKind::ColorToAlpha { key: [255, 255, 255] })
    }),
    cmd(Group::CutOut, "Defringe...", "", Needs::Picture, "Recolour the edge pixels from the inside", |_| {
        StickerAction::OpenPopover(PopKind::Defringe { width: 2 })
    }),
    cmd(Group::CutOut, "Remove White Matte", "", Needs::Picture, "Undo a white background mixed into soft edges", |_| {
        StickerAction::RemoveMatte(Matte::White)
    }),
    cmd(Group::CutOut, "Remove Black Matte", "", Needs::Picture, "Undo a black background mixed into soft edges", |_| {
        StickerAction::RemoveMatte(Matte::Black)
    }),
    cmd(Group::Filter, "Blur...", "", Needs::Picture, "Gaussian blur (of the selection)", |_| StickerAction::OpenPopover(PopKind::Blur { radius: 4.0 })),
    cmd(Group::Filter, "Sharpen...", "", Needs::Picture, "Unsharp mask (of the selection)", |_| {
        StickerAction::OpenPopover(PopKind::Sharpen { amount: 1.0, radius: 2.0 })
    }),
    // ---- stickers P8: the Sticker group ----
    cmd(Group::Sticker, "Trim...", "", Needs::Layers, "Make the artboard the bounds of what is visible (with padding)", |_| {
        StickerAction::OpenPopover(PopKind::Trim { padding: 0 })
    }),
    cmd(Group::Sticker, "Add Padding...", "", Needs::Layers, "More room on every side of the artboard", |_| {
        StickerAction::OpenPopover(PopKind::Padding { px: 16 })
    }),
    cmd(Group::Sticker, "Canvas Size...", "", Needs::Layers, "Another artboard size, the layers kept where they are", |_| {
        StickerAction::OpenPopover(PopKind::CanvasSize { w: 0, h: 0, anchor: [1, 1] })
    }),
    cmd(Group::Sticker, "Resize...", "", Needs::Layers, "Scale the whole sticker (it keeps its size on the model)", |_| {
        StickerAction::OpenPopover(PopKind::Resize { w: 0, h: 0, keep: true })
    }),
    cmd(Group::Sticker, "Rotate 90 CW", "", Needs::Layers, "Turn the sticker a quarter clockwise", |_| StickerAction::Layout(Layout::Rotate(90.0))),
    cmd(Group::Sticker, "Rotate 90 CCW", "", Needs::Layers, "Turn the sticker a quarter counter-clockwise", |_| {
        StickerAction::Layout(Layout::Rotate(-90.0))
    }),
    cmd(Group::Sticker, "Rotate 180", "", Needs::Layers, "Turn the sticker upside down", |_| StickerAction::Layout(Layout::Rotate(180.0))),
    cmd(Group::Sticker, "Rotate...", "", Needs::Layers, "Turn the sticker by any angle (the artboard grows to fit)", |_| {
        StickerAction::OpenPopover(PopKind::Rotate { deg: 15.0 })
    }),
    cmd(Group::Sticker, "Flip Horizontal", "", Needs::Layers, "Mirror the whole sticker left to right", |_| StickerAction::Layout(Layout::Flip(true))),
    cmd(Group::Sticker, "Flip Vertical", "", Needs::Layers, "Mirror the whole sticker top to bottom", |_| StickerAction::Layout(Layout::Flip(false))),
    cmd(Group::Sticker, "Flatten", "", Needs::Layers, "All layers become one picture (it looks the same)", |_| StickerAction::Layout(Layout::Flatten)),
    cmd(Group::Sticker, "Export as PSD...", "", Needs::Layers, "Save the sticker's layers as a PSD file (no link)", |_| StickerAction::ExportPsd(None)),
    cmd(Group::Sticker, "Save to Library", "", Needs::Layers, "Save the sticker into the sticker library folder", |_| StickerAction::SaveToLibrary(None)),
    // ---- the Layer group ----
    cmd(Group::Layer, "New Picture Layer", "", Needs::Layers, "An empty picture layer above the active layer", |_| StickerAction::AddPictureLayer(None)),
    cmd(Group::Layer, "Add Picture...", "", Needs::Layers, "A picture file as a new layer", |_| StickerAction::AddPictureFile(None)),
    cmd(Group::Layer, "Text", "", Needs::Layers, "A new text layer in the middle of the artboard", |_| StickerAction::AddTextLayer(None)),
    cmd(Group::Layer, "Shape", "", Needs::Layers, "A new shape layer (the Shape tool's options)", |_| StickerAction::AddShapeLayer(None)),
    cmd(Group::Layer, "Duplicate", "Duplicate Sticker Layer", Needs::Layers, "A copy of the active layer", |_| StickerAction::DuplicateLayer(None)),
    cmd(Group::Layer, "Delete", "", Needs::Layers, "Delete the active layer", |_| StickerAction::DeleteLayer(None)),
    cmd(Group::Layer, "Merge Down", "Merge Sticker Layer Down", Needs::Below, "The active layer and the one below become one picture", |_| {
        StickerAction::MergeDown(None)
    }),
    cmd(Group::Layer, "Rasterize", "", Needs::Vector, "Make the text or shape layer a picture layer", |_| StickerAction::Rasterize),
    cmd(Group::Layer, "Clip to Layer Below", "", Needs::Below, "Show the layer only where the layer below shows (a toggle)", |_| {
        StickerAction::ToggleClip(None)
    }),
    cmd(Group::Layer, "Bring Forward", "Move Sticker Layer Up", Needs::Layers, "One step up", |_| StickerAction::MoveLayer(None, 1)),
    cmd(Group::Layer, "Send Backward", "Move Sticker Layer Down", Needs::Layers, "One step down", |_| StickerAction::MoveLayer(None, -1)),
    cmd(Group::Layer, "Copy Layer Style", "", Needs::Layers, "Remember the active layer's styles", |_| StickerAction::CopyStyle(StyleTarget::Layers)),
    cmd(Group::Layer, "Paste Layer Style", "", Needs::Layers, "The remembered styles on the selected layers", |_| StickerAction::PasteStyle(StyleTarget::Layers)),
    cmd(Group::Layer, "Clear Layer Style", "", Needs::Layers, "No styles on the selected layers", |_| StickerAction::ClearStyle(StyleTarget::Layers)),
];

/// Stickers P8: what the Below and Vector commands need beside `available`: a layer below the active one, and a text
/// or shape layer as the active one.
pub(crate) fn available_more(state: &mut AppState) -> (bool, bool) {
    match layered(state, None) {
        Some((doc, id)) => {
            let i = doc.layers.iter().position(|l| l.id == id);
            let vector = doc.layer(id).is_some_and(|l| !matches!(l.content, StickerContent::Picture(_)));
            (i.is_some_and(|i| i > 0), vector)
        }
        None => (false, false),
    }
}

/// `enabled` with the Below and Vector needs looked at (`more` = `available_more`).
pub(crate) fn enabled_more(def: &CommandDef, have: (bool, bool, bool), more: (bool, bool)) -> bool {
    enabled(def, have)
        && match def.needs {
            Needs::Below => more.0,
            Needs::Vector => more.1,
            _ => true,
        }
}

/// What the open editor has: a layered sticker, a selection, a picture layer as the active layer.
pub(crate) fn available(state: &mut AppState) -> (bool, bool, bool) {
    let selection = edit_of(state).is_some_and(|e| e.selection.is_some());
    match layered(state, None) {
        Some((doc, id)) => {
            let picture = doc.layer(id).is_some_and(|l| matches!(l.content, StickerContent::Picture(_)));
            (true, selection, picture)
        }
        None => (false, selection, false),
    }
}

/// True when the command can run now (`have` = `available`).
pub(crate) fn enabled(def: &CommandDef, have: (bool, bool, bool)) -> bool {
    let (layers, selection, picture) = have;
    layers
        && match def.needs {
            Needs::Layers => true,
            Needs::Selection => selection,
            Needs::Picture => picture,
            Needs::PictureAndSelection => picture && selection,
            // Stickers P8: looked at by `enabled_more`.
            Needs::Below | Needs::Vector => true,
        }
}
