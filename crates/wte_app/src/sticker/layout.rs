// Owner: stickers P8 (STICKER_PLAN 3.3 "Sticker" and "Layer" commands, 3.8 "Export as PSD"; ARCHITECTURE 8.15). The
// Sticker view's whole-sticker commands (Trim, Add Padding, Canvas Size, Resize, Rotate 90 CW / CCW / 180, Rotate...,
// Flip Horizontal / Vertical, Flatten) and the layer commands P8 adds (Merge Down, Clip to Layer Below, Flip and Reset
// of one layer). Each is one `Document` command (`doc_sticker_layout.rs` in the core), so one history entry; an image
// stamp is promoted inside it. The commands with numbers open a popover (`popover.rs`: Trim..., Add Padding...,
// Canvas Size..., Resize..., Rotate...). Export as PSD writes the sticker's layers with the link writer (5.2) without
// making a link.

use std::path::{Path, PathBuf};

use crate::app::WteApp;
use crate::state::AppState;
use crate::sticker::tools::transform::{self, TRANSFORM_LABEL};
use crate::sticker::{edit_of, edit_ref, layered};

/// History labels of the layer commands here.
pub(crate) const CLIP_LABEL: &str = "Clip to Layer Below";
pub(crate) const UNCLIP_LABEL: &str = "Release Clipping Mask";

/// One whole-sticker command.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Layout {
    Trim(u32),
    Padding(u32),
    CanvasSize([u32; 2], [f32; 2]),
    Resize([u32; 2]),
    Rotate(f32),
    Flip(bool),
    Flatten,
}

/// Runs a whole-sticker command on the open editor's sticker (one entry; an error becomes a toast). A crop box in
/// progress is dropped (the artboard it was drawn on changed).
pub(crate) fn run(state: &mut AppState, cmd: Layout) {
    let Some(on) = edit_of(state).and_then(edit_ref) else { return };
    if layered(state, None).is_none() && crate::sticker::tools::artboard_of(state).is_none() {
        return;
    }
    let Some(doc) = state.doc_mut() else { return };
    let r = match cmd {
        Layout::Trim(p) => doc.trim_sticker(on, p),
        Layout::Padding(p) => doc.pad_sticker(on, p),
        Layout::CanvasSize(size, anchor) => doc.canvas_size_sticker(on, size, anchor),
        Layout::Resize(size) => doc.resize_sticker(on, size),
        Layout::Rotate(deg) => doc.rotate_sticker(on, deg),
        Layout::Flip(h) => doc.flip_sticker(on, h),
        Layout::Flatten => doc.flatten_sticker(on),
    };
    if crate::dispatch::edited(state, r).is_some() {
        state.sticker_ui.crop = None;
        state.status_hint.clear();
    }
}

/// Merge Down of `layer` (`None`: the active layer) onto the layer below (one entry).
pub(crate) fn merge_down(state: &mut AppState, layer: Option<wte_core::StickerLayerId>) {
    let Some((doc, id)) = layered(state, layer) else { return };
    let Some(i) = doc.layers.iter().position(|l| l.id == id) else { return };
    if i == 0 {
        state.status_hint = "The bottom layer has no layer below it".to_owned();
        return;
    }
    let below = doc.layers[i - 1].id;
    let Some(on) = edit_of(state).and_then(edit_ref) else { return };
    let r = state.doc_mut().map(|d| d.merge_down_sticker_layer(on, id));
    if let Some(r) = r
        && crate::dispatch::edited(state, r).is_some()
    {
        crate::sticker::set_active(state, below);
    }
}

/// Clip to Layer Below of `layer` (`None`: the active layer): a toggle; the bottom layer cannot be clipped.
pub(crate) fn toggle_clip(state: &mut AppState, layer: Option<wte_core::StickerLayerId>) {
    let Some((doc, id)) = layered(state, layer) else { return };
    let Some(i) = doc.layers.iter().position(|l| l.id == id) else { return };
    let clip = doc.layers[i].clip;
    if i == 0 && !clip {
        state.status_hint = "The bottom layer has no layer below it to clip to".to_owned();
        return;
    }
    if doc.layers[i].locked {
        state.status_hint = "The layer is locked: unlock it first".to_owned();
        return;
    }
    let label = if clip { UNCLIP_LABEL } else { CLIP_LABEL };
    crate::sticker::layer_edit_pub(state, id, label, wte_core::Commit::Final, move |l| l.clip = !clip);
}

/// Flip Horizontal / Vertical or Reset of the active layer (about its box centre; one entry each).
pub(crate) fn layer_transform(state: &mut AppState, what: LayerTransformCmd) {
    let Some((id, b, t, locked)) = transform::active_box(state) else { return };
    if locked {
        state.status_hint = "The layer is locked: unlock it first".to_owned();
        return;
    }
    let t2 = match what {
        LayerTransformCmd::FlipH => b.flipped(&t, true),
        LayerTransformCmd::FlipV => b.flipped(&t, false),
        LayerTransformCmd::Reset => transform::reset(&b, &t),
    };
    crate::sticker::layer_edit_pub(state, id, TRANSFORM_LABEL, wte_core::Commit::Final, move |l| l.transform = t2);
}

/// What `layer_transform` does.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayerTransformCmd {
    FlipH,
    FlipV,
    Reset,
}

/// "Export as PSD..." (`path` `None`: the save dialog, never in a self-test).
pub(crate) fn export_psd(app: &mut WteApp, path: Option<PathBuf>, frame: &eframe::Frame) {
    let Some(on) = edit_of(&app.state).and_then(edit_ref) else { return };
    let name = layered(&mut app.state, None).map(|(d, _)| d.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Sticker".to_owned());
    let Some(path) = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_save_psd(frame, prefs, &name)) else { return };
    write_psd(&mut app.state, on, &path);
}

/// Writes the PSD of the sticker `on` names to `path`; a toast says what happened.
pub(crate) fn write_psd(state: &mut AppState, on: wte_core::StickerRef, path: &Path) -> bool {
    let Some(doc) = state.doc_mut() else { return false };
    let bytes = doc.sticker_psd_bytes(on);
    let Some(bytes) = crate::dispatch::edited(state, bytes) else { return false };
    let file = path.file_name().map_or_else(|| path.display().to_string(), |f| f.to_string_lossy().into_owned());
    match std::fs::write(path, &bytes) {
        Ok(()) => {
            state.toasts.info(format!("Exported {file} (the sticker's layers, no link)"));
            true
        }
        Err(e) => {
            state.toasts.error(format!("Could not write {file}: {e}"));
            false
        }
    }
}
