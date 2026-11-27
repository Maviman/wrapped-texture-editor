// Owner: stickers P6b (STICKER_PLAN 3.6, 3.8; ARCHITECTURE 8.15). The sticker tray of the Sticker tab and its commands:
// thumbnails of every sticker of the project that is saved with it (a stamp shows it, or it is kept in the tray), a
// click makes one the Stamp tool's source; New Sticker (blank), New from picture..., Edit, Duplicate, Delete (only
// without stamps) and Export Picture...; the Stamp tool's "Edit before placing" (E4). The thumbnails of the Sticker tab
// (`PictureThumbs`) are made at most two per frame and never while an edit is being dragged.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use egui::{Rect, Sense, vec2};
use wte_core::{BlobId, PicturePixels, PixelBuf, StickerDoc, StickerId, StickerLayer};

use crate::action::AppAction;
use crate::app::WteApp;
use crate::state::{AppState, PanelHit, StampToolSource, StickerTarget, ToolKind, ViewMode};
use crate::sticker::{PictureOf, StickerAction, enter_target};

/// Side of a blank sticker (STICKER_PLAN 3.8).
pub(crate) const BLANK_SIDE: u32 = 512;
/// Side of a tray thumbnail in points.
pub(crate) const TRAY_THUMB: f32 = 52.0;
/// Thumbnails made per frame at most.
const THUMBS_PER_FRAME: usize = 2;

/// What a thumbnail was made from, by value (pitfall 23 of ARCHITECTURE 8.13: no `Arc` of a sticker is kept across
/// frames). The keys hold picture versions, numbered per document: the caches are dropped at a document change.
enum PictureKey {
    Sticker(crate::sticker::canvas::CanvasKey),
    Blob(BlobId),
}

/// One thumbnail.
struct PictureThumb {
    key: PictureKey,
    tex: Option<egui::TextureHandle>,
}

/// The thumbnails of the Sticker tab, for one document (`Session::scene_generation`).
#[derive(Default)]
pub(crate) struct PictureThumbs {
    generation: Option<u64>,
    stickers: HashMap<StickerId, PictureThumb>,
    blobs: HashMap<BlobId, PictureThumb>,
    budget: usize,
    /// Thumbnails made so far (tests: a thumbnail is made once per change).
    pub(crate) made: u64,
}

impl PictureThumbs {
    /// Start of a frame of the Sticker tab: the caches of another document are dropped; the budget is reset.
    pub(crate) fn begin(&mut self, generation: Option<u64>) {
        if self.generation != generation {
            self.stickers.clear();
            self.blobs.clear();
            self.generation = generation;
        }
        self.budget = THUMBS_PER_FRAME;
    }
}

/// The thumbnail of sticker `id` (its layers, no stamp styles) and its size in pixels; made again when the sticker
/// changed.
pub(crate) fn sticker_thumb(ctx: &egui::Context, state: &mut AppState, id: StickerId) -> Option<(egui::TextureId, [usize; 2])> {
    let session = state.session.as_mut()?;
    let doc = session.doc.sticker(id)?;
    let [w, h] = doc.size();
    let key = crate::sticker::canvas::sticker_key(id, doc);
    let thumbs = &mut state.sticker_ui.pictures;
    let stale = thumbs.stickers.get(&id).is_none_or(|t| !matches!(&t.key, PictureKey::Sticker(k) if *k == key));
    if stale {
        if crate::panels::interacting(&session.doc) {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else if thumbs.budget > 0 {
            thumbs.budget -= 1;
            thumbs.made += 1;
            let side = crate::panels::thumb_pixels(ctx, TRAY_THUMB);
            let scale = side as f32 / w.max(h).max(1) as f32;
            let image = session.doc.sticker_picture(id, None, scale.min(1.0)).ok();
            let old = thumbs.stickers.remove(&id).and_then(|t| t.tex);
            let tex = crate::panels::upload(ctx, &format!("sticker-thumb-{}", id.0), old, image.as_ref());
            thumbs.stickers.insert(id, PictureThumb { key: PictureKey::Sticker(key), tex });
        } else {
            ctx.request_repaint();
        }
    }
    thumbs.stickers.get(&id).and_then(|t| t.tex.as_ref()).map(|t| (t.id(), t.size()))
}

/// The thumbnail of image `blob`.
pub(crate) fn blob_thumb(ctx: &egui::Context, state: &mut AppState, blob: BlobId) -> Option<(egui::TextureId, [usize; 2])> {
    let session = state.session.as_mut()?;
    let thumbs = &mut state.sticker_ui.pictures;
    let stale = thumbs.blobs.get(&blob).is_none_or(|t| !matches!(t.key, PictureKey::Blob(b) if b == blob));
    if stale && thumbs.budget > 0 {
        thumbs.budget -= 1;
        thumbs.made += 1;
        let side = crate::panels::thumb_pixels(ctx, TRAY_THUMB);
        let image = session.doc.blob_thumbnail(blob, side).ok();
        let tex = crate::panels::upload(ctx, &format!("sticker-blob-thumb-{}", blob.0), None, image.as_ref());
        thumbs.blobs.insert(blob, PictureThumb { key: PictureKey::Blob(blob), tex });
    } else if stale {
        ctx.request_repaint();
    }
    thumbs.blobs.get(&blob).and_then(|t| t.tex.as_ref()).map(|t| (t.id(), t.size()))
}

/// The stickers the tray lists: those saved with the project (a stamp shows them, or they are kept), ascending id.
pub(crate) fn tray_stickers(state: &AppState) -> Vec<StickerId> {
    let Some(doc) = state.doc() else { return Vec::new() };
    doc.stickers().filter(|(id, d)| d.keep || !doc.sticker_users(*id).is_empty()).map(|(id, _)| id).collect()
}

/// The tray block of the Sticker tab. Records the item and button rects.
pub(crate) fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    let items = tray_stickers(state);
    if state.sticker_ui.tray_selected.is_some_and(|id| !items.contains(&id)) {
        state.sticker_ui.tray_selected = None;
    }
    let selected = state.sticker_ui.tray_selected;
    if items.is_empty() {
        ui.weak("No stickers yet. Make one here, or edit a stamp (Edit Sticker).");
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for id in &items {
            let thumb = sticker_thumb(ui.ctx(), state, *id);
            let (name, users) = state.doc().map_or((String::new(), 0), |d| {
                (d.sticker(*id).map_or_else(String::new, |s| s.name.clone()), d.sticker_users(*id).len())
            });
            let (rect, resp) = ui.allocate_exact_size(vec2(TRAY_THUMB, TRAY_THUMB), Sense::click());
            let aspect = thumb.map_or(1.0, |(_, [w, h])| w as f32 / h.max(1) as f32);
            crate::panels::paint_thumb(ui.painter(), rect.shrink(3.0), thumb.map(|(t, _)| t), aspect);
            if selected == Some(*id) {
                ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(2.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
            }
            let placed = match users {
                0 => "not placed".to_owned(),
                1 => "placed once".to_owned(),
                n => format!("placed {n} times"),
            };
            let resp = resp.on_hover_text(format!("{name} ({placed}). Click: place it with the Stamp tool"));
            hits.push((PanelHit::TrayItem(id.0), rect));
            if resp.clicked() {
                actions.push(AppAction::Sticker(StickerAction::TrayUse(*id)));
            }
        }
    });
    if let Some(name) = selected.and_then(|id| state.doc().and_then(|d| d.sticker(id)).map(|s| s.name.clone())) {
        ui.label(format!("Selected: {name}"));
    }
    let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
    let has_doc = state.session.is_some();
    let can_delete = selected.is_some_and(|id| state.doc().is_some_and(|d| d.sticker_users(id).is_empty()));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let mut button = |ui: &mut egui::Ui, hit: PanelHit, enabled: bool, label: String, tip: &str, action: Option<StickerAction>| {
            let resp = ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip);
            hits.push((hit, resp.rect));
            if resp.clicked()
                && let Some(a) = action
            {
                actions.push(AppAction::Sticker(a));
            }
        };
        button(ui, PanelHit::TrayNew, has_doc, "New Sticker".into(), "A blank 512 x 512 sticker in the tray", Some(StickerAction::NewSticker));
        button(
            ui,
            PanelHit::TrayNewFromPicture,
            has_doc,
            format!("From picture...{on_pc}"),
            "A picture file as a new sticker in the tray",
            Some(StickerAction::NewStickerFromPicture(None)),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let mut button = |ui: &mut egui::Ui, hit: PanelHit, enabled: bool, label: String, tip: &str, action: Option<StickerAction>| {
            let resp = ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip);
            hits.push((hit, resp.rect));
            if resp.clicked()
                && let Some(a) = action
            {
                actions.push(AppAction::Sticker(a));
            }
        };
        let sel = selected.is_some();
        button(ui, PanelHit::TrayEdit, sel, "Edit".into(), "Open the selected sticker in the Sticker view", selected.map(StickerAction::EnterTray));
        button(ui, PanelHit::TrayDuplicate, sel, "Duplicate".into(), "A copy of the selected sticker in the tray", selected.map(StickerAction::TrayDuplicate));
        let delete_tip = if can_delete || !sel { "Remove the selected sticker from the tray" } else { "A stamp shows this sticker: delete the stamp first" };
        button(ui, PanelHit::TrayDelete, can_delete, "Delete".into(), delete_tip, selected.map(StickerAction::TrayDelete));
        button(
            ui,
            PanelHit::TrayExport,
            sel,
            format!("Export Picture...{on_pc}"),
            "Save the selected sticker as a PNG picture",
            selected.map(|id| StickerAction::ExportPicture(PictureOf::Sticker(id), None)),
        );
    });
}

// ---- commands ----

/// A new sticker in the tray (kept with the project); it becomes the selected tray item and the Stamp tool's source.
fn add_to_tray(state: &mut AppState, mut doc: StickerDoc) -> Option<StickerId> {
    doc.keep = true;
    let name = doc.name.clone();
    let r = state.doc_mut()?.add_sticker(doc);
    let id = crate::dispatch::edited(state, r)?;
    state.sticker_ui.tray_selected = Some(id);
    state.tools.stamp.source = StampToolSource::Sticker(id);
    state.tools.stamp.source_name = name;
    Some(id)
}

/// The next free "Sticker N" name.
fn next_name(state: &AppState) -> String {
    let names: Vec<String> = state.doc().map(|d| d.stickers().map(|(_, s)| s.name.clone()).collect()).unwrap_or_default();
    (1..).map(|n| format!("Sticker {n}")).find(|n| !names.contains(n)).unwrap_or_else(|| "Sticker".to_owned())
}

/// "New Sticker": blank, 512 x 512, one empty picture layer.
pub(crate) fn new_blank(state: &mut AppState) {
    if state.session.is_none() {
        return;
    }
    let name = next_name(state);
    let mut doc = StickerDoc::new(&name, BLANK_SIDE, BLANK_SIDE);
    let layer = doc.alloc_layer();
    let pixels = PicturePixels::Owned { pixels: Arc::new(PixelBuf::new(BLANK_SIDE, BLANK_SIDE)), version: 0 };
    doc.layers.push(StickerLayer::new(layer, "Layer 1", wte_core::StickerContent::Picture(pixels)));
    if add_to_tray(state, doc).is_some() {
        state.status_hint = format!("{name} is in the tray: Edit it, or click the model to place it");
    }
}

/// "New from picture..." with a registered image: a one-layer sticker of the picture.
pub(crate) fn new_from_blob(state: &mut AppState, blob: BlobId, name: &str) -> Option<StickerId> {
    let size = match state.doc_mut()?.image_size(blob) {
        Ok(s) => s,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return None;
        }
    };
    add_to_tray(state, StickerDoc::from_picture(name, PicturePixels::Blob(blob), size))
}

/// A click on a tray item: the selected item and the Stamp tool's source; outside the Sticker view the Stamp tool
/// becomes the tool (one click on the model places it).
pub(crate) fn use_as_source(state: &mut AppState, id: StickerId) {
    let Some(name) = state.doc().and_then(|d| d.sticker(id)).map(|s| s.name.clone()) else { return };
    state.sticker_ui.tray_selected = Some(id);
    state.tools.stamp.source = StampToolSource::Sticker(id);
    state.tools.stamp.source_name = name;
    if state.view_mode == ViewMode::Sticker {
        state.status_hint = "Chosen for the Stamp tool: leave the Sticker view (Done) to place it".to_owned();
    } else {
        state.push(AppAction::SetTool(ToolKind::Stamp));
        state.status_hint = "Click on the model to place the sticker".to_owned();
    }
}

/// "Duplicate": a copy in the tray (pictures shared, nothing drawn again).
pub(crate) fn duplicate(state: &mut AppState, id: StickerId) {
    let Some(doc) = state.doc().and_then(|d| d.sticker(id)).map(|d| (**d).clone()) else { return };
    let name = format!("{} copy", doc.name);
    add_to_tray(state, StickerDoc { name, ..doc });
}

/// "Delete": only a sticker no stamp shows (the core refuses the others).
pub(crate) fn delete(state: &mut AppState, id: StickerId) {
    let Some(doc) = state.doc_mut() else { return };
    let r = doc.remove_sticker(id);
    if crate::dispatch::edited(state, r).is_some() {
        if state.sticker_ui.tray_selected == Some(id) {
            state.sticker_ui.tray_selected = None;
        }
        if state.tools.stamp.source == StampToolSource::Sticker(id) {
            state.tools.stamp.source = StampToolSource::None;
            state.tools.stamp.source_name.clear();
        }
    }
}

/// The Stamp tool's "Edit before placing" (E4): an image source becomes a tray sticker (kept, one entry "Add
/// Sticker"); the Sticker view opens on the tool's sticker. The tool keeps it as its source.
pub(crate) fn edit_before_placing(state: &mut AppState) {
    let id = match state.tools.stamp.source {
        StampToolSource::None => {
            state.status_hint = "Choose a picture or a sticker for the Stamp tool first".to_owned();
            return;
        }
        StampToolSource::Sticker(id) if state.doc().is_some_and(|d| d.sticker(id).is_some()) => id,
        StampToolSource::Sticker(_) => {
            state.tools.stamp.source = StampToolSource::None;
            return;
        }
        StampToolSource::Blob(blob) => {
            let name = crate::tools::stamp_place::image_stamp_name(&state.tools.stamp.source_name);
            let Some(id) = new_from_blob(state, blob, &name) else { return };
            id
        }
    };
    enter_target(state, StickerTarget::Tray(id));
}

/// "Export Picture...": the finished picture as a PNG (`path` `None`: the save dialog; never in a self-test).
pub(crate) fn export_picture(app: &mut WteApp, what: PictureOf, path: Option<PathBuf>, frame: &eframe::Frame) {
    let name = match what {
        PictureOf::Stamp(id) => app.state.doc().and_then(|d| d.stamp(id)).map(|s| s.name.clone()),
        PictureOf::Sticker(id) => app.state.doc().and_then(|d| d.sticker(id)).map(|s| s.name.clone()),
    };
    let Some(name) = name else { return };
    let Some(path) = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_save_png(frame, prefs, &name)) else { return };
    write_picture(&mut app.state, what, &path);
}

/// Writes the picture of `what` to `path` (PNG); a toast says what happened.
pub(crate) fn write_picture(state: &mut AppState, what: PictureOf, path: &std::path::Path) -> bool {
    let Some(doc) = state.doc_mut() else { return false };
    let picture = match what {
        PictureOf::Stamp(id) => doc.stamp_picture(id, true),
        PictureOf::Sticker(id) => doc.sticker_picture(id, None, 1.0),
    };
    let Some(picture) = crate::dispatch::edited(state, picture) else { return false };
    let file = path.file_name().map_or_else(|| path.display().to_string(), |f| f.to_string_lossy().into_owned());
    let written = wte_core::encode_png(picture.width, picture.height, &picture.data, wte_core::PngLevel::Balanced)
        .map_err(|e| e.to_string())
        .and_then(|png| std::fs::write(path, png).map_err(|e| e.to_string()));
    match written {
        Ok(()) => {
            state.toasts.info(format!("Exported {file} ({} x {} pixels)", picture.width, picture.height));
            true
        }
        Err(e) => {
            state.toasts.error(format!("Could not write {file}: {e}"));
            false
        }
    }
}
