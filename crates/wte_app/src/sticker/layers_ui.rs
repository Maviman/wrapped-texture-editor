// Owner: stickers P6a (STICKER_PLAN 3.2; ARCHITECTURE 8.7, 8.15). The side panel in the Sticker view: the Layers tab
// shows the sticker's layers (eye, thumbnail, name with kind / opacity / blend / "clipped", lock; click selects, drag
// reorders; the active layer's name, opacity and blend above the list; the footer New / Add picture... / Duplicate /
// Up / Down / Delete), and the Tool tab shows the options of the active sticker tool and the active layer's position.
// Every change is a `StickerAction`; every feature has a button (tablet rule of 3.10: nothing only by drag, hover,
// right click, double click or a key).
//
// Stickers P8: the Tool tab also has the Crop, Text and Shape tools' options, the Move tool's Keep aspect and the
// active layer's W / H / Angle, Flip H / V and Reset, and the Adjustments block; the Layers footer gets Text, Shape,
// Clip and Merge, the rows a context menu (Duplicate, Delete, Merge Down, Rasterize, Clip to Layer Below, Copy /
// Paste Style: each also a button or menu item, the tablet rule).

use eframe::egui;
use egui::{Color32, Pos2, Rect, Sense, Stroke, vec2};
use wte_core::sticker::render::LayerKey;
use wte_core::{BlendMode, BlobId, StickerContent, StickerDoc, StickerId, StickerLayer, StickerLayerId};

use crate::action::AppAction;
use crate::panels::{LAYER_THUMB_SIDE, PanelState, continuous_edit, paint_thumb, percent_slider};
use crate::state::{AppState, PanelHit, StickerTool};
use crate::sticker::{Shown, StickerAction, ThumbTex, edit_of, shown, tool_of};

/// Height of one layer row.
const ROW_HEIGHT: f32 = LAYER_THUMB_SIDE + 6.0;
/// Side of the eye and lock icons.
const ICON_SIDE: f32 = 18.0;
/// Layer thumbnails drawn per frame at most (8.7 policy, as for texture layers).
const THUMBS_PER_FRAME: usize = 2;

/// What a layer thumbnail was made from: the layer alone, compared by value (pixels by version or blob id).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ThumbKey {
    Layer(StickerId, Box<LayerKey>),
    Blob(BlobId),
}

fn sticker(action: StickerAction) -> AppAction {
    AppAction::Sticker(action)
}

/// The Layers tab in the Sticker view. Records the list's visible rect, the footer and the row controls in
/// `state.panel_rects` (`LayersList` / `LayersFooter` as the texture layers' tab does, so the layout checks hold).
pub(crate) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    let out = ui.scope(|ui| body(ui, state, ps));
    ps.layers_content_bottom = Some(out.response.rect.bottom());
}

fn body(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    let Some(edit) = edit_of(state).cloned() else {
        ui.weak("No sticker is open");
        return;
    };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let mut actions = Vec::new();
    let mut hits = Vec::new();
    let Some(doc) = shown.layers().cloned() else {
        ui.label("This text stamp is not a layered sticker yet.");
        ui.weak("Its text stays editable as a text layer after the conversion.");
        let resp = ui.button("Convert to layered sticker");
        hits.push((PanelHit::StickerConvertText, resp.rect));
        if resp.clicked() {
            actions.push(sticker(StickerAction::ConvertText));
        }
        state.actions.extend(actions);
        state.panel_rects.extend(hits);
        return;
    };

    // The active layer: name, opacity, blend.
    if let Some(layer) = edit.active_layer.and_then(|id| doc.layer(id)) {
        let id = layer.id;
        ui.horizontal(|ui| {
            ui.label("Name");
            let editing = state.sticker_ui.layer_name_edit.as_ref().filter(|(l, _)| *l == id).map(|(_, t)| t.clone());
            let mut text = editing.unwrap_or_else(|| layer.name.clone());
            let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
            if resp.lost_focus() {
                actions.push(sticker(StickerAction::RenameLayer(id, text)));
                state.sticker_ui.layer_name_edit = None;
            } else if resp.has_focus() || resp.changed() {
                state.sticker_ui.layer_name_edit = Some((id, text));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).clamp(50.0, 120.0);
            let mut opacity = layer.opacity;
            let resp = ui.add(percent_slider(&mut opacity, 0.0, 100.0).max_decimals(0));
            continuous_edit(ps, &mut actions, ui, &resp, |commit| sticker(StickerAction::SetLayerOpacity(id, opacity, commit)));
            egui::ComboBox::from_id_salt("sticker_layer_blend")
                .selected_text(layer.blend.label())
                .width(76.0)
                .show_ui(ui, |ui| {
                    for mode in BlendMode::ALL {
                        if ui.selectable_label(layer.blend == mode, mode.label()).clicked() && layer.blend != mode {
                            actions.push(sticker(StickerAction::SetLayerBlend(id, mode)));
                        }
                    }
                })
                .response
                .on_hover_text("Blend mode of the active sticker layer");
        });
    }
    ui.add_space(2.0);

    let list_height = (ui.available_height() - ps.layers_footer_height - ui.spacing().item_spacing.y).max(ROW_HEIGHT);
    let mut budget = THUMBS_PER_FRAME;
    state.sticker_ui.thumbs.retain(|id, _| doc.layers.iter().any(|l| l.id.0 == *id));
    let list = egui::ScrollArea::vertical()
        .id_salt("sticker_layers_scroll")
        .max_height(list_height)
        .auto_shrink([false, true])
        .show(ui, |ui| rows(ui, state, &shown, &doc, edit.active_layer, &mut budget, &mut actions, &mut hits));
    let shown_height = list.content_size.y.min(list.inner_rect.height());
    ps.layers_list_overflow = (list.content_size.y - shown_height).max(0.0);
    hits.push((PanelHit::LayersList, Rect::from_min_size(list.inner_rect.min, vec2(list.inner_rect.width(), shown_height))));

    let footer_rect = footer(ui, state, &doc, edit.active_layer, &mut actions, &mut hits);
    ps.layers_footer_height = footer_rect.height().max(0.0);
    hits.push((PanelHit::LayersFooter, footer_rect));
    state.actions.extend(actions);
    state.panel_rects.extend(hits);
}

/// The rows, top row = top layer. A drag on a row reorders (the drop goes between the rows under the pointer).
#[expect(clippy::too_many_arguments, reason = "one list, drawn from the body")]
fn rows(
    ui: &mut egui::Ui,
    state: &mut AppState,
    shown: &Shown,
    doc: &StickerDoc,
    active: Option<StickerLayerId>,
    budget: &mut usize,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    ui.set_min_width(ui.available_width());
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut rects: Vec<(usize, Rect)> = Vec::with_capacity(doc.layers.len());
    let mut dragged: Option<(usize, StickerLayerId)> = None;
    let mut dropped: Option<(usize, StickerLayerId)> = None;
    for (index, layer) in doc.layers.iter().enumerate().rev() {
        let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), ROW_HEIGHT));
        let resp = ui.interact(rect, egui::Id::unique(("sticker_layer_row", layer.id.0)), Sense::click_and_drag());
        if resp.dragged_by(egui::PointerButton::Primary) {
            dragged = Some((index, layer.id));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.drag_stopped() {
            dropped = Some((index, layer.id));
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("sticker_layer_body", layer.id.0)));
        row(&mut child, state, shown, layer, active == Some(layer.id), budget, actions, hits);
        ui.advance_cursor_after_rect(rect);
        if resp.clicked() && active != Some(layer.id) {
            actions.push(sticker(StickerAction::SelectLayer(layer.id)));
        }
        // Stickers P8: the row's context menu.
        let id = layer.id;
        let (below, vector, clip) = (index > 0, !matches!(layer.content, StickerContent::Picture(_)), layer.clip);
        resp.context_menu(|ui| {
            let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, action: StickerAction| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    actions.push(sticker(StickerAction::SelectLayer(id)));
                    actions.push(sticker(action));
                    ui.close();
                }
            };
            item(ui, true, "Duplicate", StickerAction::DuplicateLayer(Some(id)));
            item(ui, doc.layers.len() > 1, "Delete", StickerAction::DeleteLayer(Some(id)));
            item(ui, below, "Merge Down", StickerAction::MergeDown(Some(id)));
            item(ui, vector, "Rasterize", StickerAction::Rasterize);
            item(ui, below || clip, if clip { "Release Clipping Mask" } else { "Clip to Layer Below" }, StickerAction::ToggleClip(Some(id)));
            item(ui, true, "Copy Style", StickerAction::CopyStyle(crate::sticker::StyleTarget::Layers));
            item(ui, state.style_clipboard.is_some(), "Paste Style", StickerAction::PasteStyle(crate::sticker::StyleTarget::Layers));
        });
        hits.push((PanelHit::StickerLayerRow(layer.id.0), rect));
        rects.push((index, rect));
    }
    let pointer = ui.ctx().pointer_interact_pos();
    let target = pointer.and_then(|p| drop_target(&rects, p));
    if let (Some(_), Some((_, _, y))) = (dragged, target) {
        let x = rects.first().map_or(ui.max_rect().x_range(), |(_, r)| r.x_range());
        ui.painter().hline(x, y, Stroke::new(2.0, crate::theme::ACCENT));
    }
    if let (Some((from, id)), Some((to, above, _))) = (dropped, target) {
        let to_index = crate::panels::layers::drop_index(from, to, above);
        if to_index != from {
            actions.push(sticker(StickerAction::MoveLayerTo(id, to_index)));
        }
    }
}

/// The row under `pointer`: (layer index, upper half?, y of the insertion line).
fn drop_target(rows: &[(usize, Rect)], pointer: Pos2) -> Option<(usize, bool, f32)> {
    let (index, rect) = rows
        .iter()
        .min_by(|a, b| (a.1.center().y - pointer.y).abs().total_cmp(&(b.1.center().y - pointer.y).abs()))
        .copied()?;
    let above = pointer.y < rect.center().y;
    Some((index, above, if above { rect.top() } else { rect.bottom() }))
}

/// One row: eye, thumbnail, name with its kind and details, lock.
#[expect(clippy::too_many_arguments, reason = "one row of the list")]
fn row(
    ui: &mut egui::Ui,
    state: &mut AppState,
    shown: &Shown,
    layer: &StickerLayer,
    active: bool,
    budget: &mut usize,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    let width = ui.available_width();
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = ui.horizontal(|ui| {
        ui.set_min_size(vec2(width, ROW_HEIGHT));
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(ICON_SIDE, ROW_HEIGHT), Sense::click());
        hits.push((PanelHit::StickerLayerEye(layer.id.0), rect));
        crate::panels::layers::paint_eye(ui.painter(), rect, layer.visible, crate::panels::layers::icon_color(ui, &resp));
        if resp.on_hover_text(if layer.visible { "Hide layer" } else { "Show layer" }).clicked() {
            actions.push(sticker(StickerAction::SetLayerVisible(layer.id, !layer.visible)));
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(LAYER_THUMB_SIDE, LAYER_THUMB_SIDE), Sense::click());
        let tex = thumb(ui.ctx(), state, shown, layer, budget);
        let aspect = shown.layers().map_or(1.0, |d| d.size()[0] as f32 / d.size()[1].max(1) as f32);
        paint_thumb(ui.painter(), rect, tex, aspect);
        if !layer.visible {
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(120));
        }
        if resp.clicked() && !active {
            actions.push(sticker(StickerAction::SelectLayer(layer.id)));
        }
        let name_width = (ui.available_width() - ICON_SIDE - 8.0).max(40.0);
        ui.allocate_ui_with_layout(vec2(name_width, ROW_HEIGHT), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(3.0);
            let text = egui::RichText::new(&layer.name);
            let resp = ui.add(egui::Label::new(if active { text.strong() } else { text }).truncate().selectable(false).sense(Sense::click()));
            if resp.clicked() && !active {
                actions.push(sticker(StickerAction::SelectLayer(layer.id)));
            }
            resp.on_hover_text("Click: select  |  Drag: reorder (or Up / Down below)");
            let kind = match layer.content {
                StickerContent::Picture(_) => "Picture",
                StickerContent::Text(_) => "Text",
                StickerContent::Shape(_) => "Shape",
            };
            let mut details = String::new();
            if layer.clip {
                details.push_str("  \u{21b3} clipped");
            }
            if layer.opacity < 1.0 {
                details.push_str(&format!("  {:.0} %", layer.opacity * 100.0));
            }
            if layer.blend != BlendMode::Normal {
                details.push_str("  ");
                details.push_str(layer.blend.label());
            }
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(kind).small().color(Color32::from_rgb(200, 140, 60))).selectable(false));
                if !details.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(details).small().weak()).selectable(false));
                }
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(ICON_SIDE, ROW_HEIGHT), Sense::click());
            hits.push((PanelHit::StickerLayerLock(layer.id.0), rect));
            crate::panels::layers::paint_lock(ui.painter(), rect, layer.locked, crate::panels::layers::icon_color(ui, &resp));
            let tip = if layer.locked { "Locked: click to unlock" } else { "Click to lock the layer (it cannot be moved or deleted)" };
            if resp.on_hover_text(tip).clicked() {
                actions.push(sticker(StickerAction::SetLayerLocked(layer.id, !layer.locked)));
            }
        });
    });
    let rect = inner.response.rect;
    let visuals = ui.visuals();
    let fill = if active { visuals.selection.bg_fill.linear_multiply(0.6) } else { visuals.extreme_bg_color };
    ui.painter().set(bg, egui::Shape::rect_filled(rect, 3.0, fill));
}

/// The thumbnail of a layer, drawn again only when the layer changed, at most `THUMBS_PER_FRAME` per frame and never
/// while an edit is being dragged (8.7 policy).
fn thumb(ctx: &egui::Context, state: &mut AppState, shown: &Shown, layer: &StickerLayer, budget: &mut usize) -> Option<egui::TextureId> {
    let key = match shown {
        Shown::Sticker { id, doc } => ThumbKey::Layer(*id, Box::new(LayerKey::of(layer, doc.artboard, 1.0, wte_core::RasterQuality::Full))),
        Shown::Image { blob, .. } => ThumbKey::Blob(*blob),
        Shown::Text { .. } => return None,
    };
    let session = state.session.as_mut()?;
    let ui_state = &mut state.sticker_ui;
    let stale = ui_state.thumbs.get(&layer.id.0).is_none_or(|t| t.key != key);
    if stale {
        if crate::panels::interacting(&session.doc) {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else if *budget > 0 {
            *budget -= 1;
            let side = crate::panels::thumb_pixels(ctx, LAYER_THUMB_SIDE);
            let image = match (&key, shown) {
                (ThumbKey::Layer(id, _), _) => session.doc.sticker_layer_thumbnail(*id, layer.id, side),
                (ThumbKey::Blob(blob), _) => session.doc.blob_thumbnail(*blob, side).ok(),
            };
            let old = ui_state.thumbs.remove(&layer.id.0).and_then(|t| t.tex);
            let tex = crate::panels::upload(ctx, &format!("sticker-layer-thumb-{}", layer.id.0), old, image.as_ref());
            ui_state.thumbs.insert(layer.id.0, ThumbTex { key, tex });
        } else {
            ctx.request_repaint();
        }
    }
    ui_state.thumbs.get(&layer.id.0).and_then(|t| t.tex.as_ref()).map(egui::TextureHandle::id)
}

/// The footer: New / Add picture... / Duplicate / Up / Down / Delete. Returns its rect.
fn footer(
    ui: &mut egui::Ui,
    state: &AppState,
    doc: &StickerDoc,
    active: Option<StickerLayerId>,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) -> Rect {
    let top = ui.cursor().min.y;
    ui.add_space(2.0);
    let index = active.and_then(|id| doc.layers.iter().position(|l| l.id == id));
    let layer = index.map(|i| &doc.layers[i]);
    let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
    let row = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let mut button = |ui: &mut egui::Ui, hit: PanelHit, enabled: bool, label: String, tip: &str, action: StickerAction| {
            let resp = ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip);
            hits.push((hit, resp.rect));
            if resp.clicked() {
                actions.push(sticker(action));
            }
        };
        button(ui, PanelHit::StickerLayerNew, true, "New".into(), "New empty picture layer above the active layer", StickerAction::AddPictureLayer(None));
        button(
            ui,
            PanelHit::StickerLayerAddPicture,
            true,
            format!("Add picture...{on_pc}"),
            "A picture from a file as a new layer above the active layer",
            StickerAction::AddPictureFile(None),
        );
        let key = |label: &str| crate::shortcuts::sticker_shortcut_text(ui.ctx(), label);
        let (dup, up, down, merge) = (key("Duplicate Sticker Layer"), key("Move Sticker Layer Up"), key("Move Sticker Layer Down"), key("Merge Sticker Layer Down"));
        button(ui, PanelHit::StickerLayerDuplicate, layer.is_some(), "Duplicate".into(), &format!("Duplicate the active layer ({dup})"), StickerAction::DuplicateLayer(None));
        let can_up = index.is_some_and(|i| i + 1 < doc.layers.len());
        button(ui, PanelHit::StickerLayerUp, can_up, "Up".into(), &format!("One step up ({up})"), StickerAction::MoveLayer(None, 1));
        let can_down = index.is_some_and(|i| i > 0);
        button(ui, PanelHit::StickerLayerDown, can_down, "Down".into(), &format!("One step down ({down})"), StickerAction::MoveLayer(None, -1));
        let can_delete = layer.is_some_and(|l| !l.locked) && doc.layers.len() > 1;
        button(
            ui,
            PanelHit::StickerLayerDelete,
            can_delete,
            "Delete".into(),
            "Delete the active layer (a sticker keeps at least one; a locked layer stays)",
            StickerAction::DeleteLayer(None),
        );
        // Stickers P8.
        button(ui, PanelHit::StickerLayerText, true, "Text".into(), "A new text layer in the middle of the artboard (or click with the Text tool, X)", StickerAction::AddTextLayer(None));
        button(ui, PanelHit::StickerLayerShape, true, "Shape".into(), "A new shape layer with the Shape tool's options (or drag with the Shape tool, U)", StickerAction::AddShapeLayer(None));
        let clipped = layer.is_some_and(|l| l.clip);
        let can_clip = index.is_some_and(|i| i > 0 || clipped) && layer.is_some_and(|l| !l.locked);
        let clip_label = if clipped { "Unclip" } else { "Clip" };
        button(ui, PanelHit::StickerLayerClip, can_clip, clip_label.into(), "Clip to Layer Below: the layer shows only where the layer below shows (pattern inside text)", StickerAction::ToggleClip(None));
        let can_merge = index.is_some_and(|i| i > 0) && layer.is_some_and(|l| !l.locked);
        button(ui, PanelHit::StickerLayerMerge, can_merge, "Merge".into(), &format!("Merge Down: the layer and the one below become one picture ({merge})"), StickerAction::MergeDown(None));
    });
    Rect::from_min_max(egui::pos2(row.response.rect.left(), top), row.response.rect.max)
}

/// The Tool tab in the Sticker view: the options of the active sticker tool and the active layer's position (X / Y,
/// whole pixels; one entry per edit). Returns the page's content rect.
pub(crate) fn tool_page(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) -> Rect {
    let tool = tool_of(state);
    let mut actions = Vec::new();
    let out = ui.vertical(|ui| {
        ui.label(egui::RichText::new(format!("Sticker {} options", tool.label())).strong());
        ui.indent("sticker_tool_options", |ui| {
            let help = |ui: &mut egui::Ui, text: &str| {
                ui.label(egui::RichText::new(text).weak());
            };
            match tool {
                StickerTool::Move => {
                    ui.checkbox(&mut state.sticker_ui.auto_select, "Auto-select layer")
                        .on_hover_text("A click picks the top layer under the pointer; off: always the active layer");
                    // Stickers P8: the corner handles keep the aspect (Shift frees it).
                    ui.checkbox(&mut state.sticker_ui.keep_aspect, "Keep aspect")
                        .on_hover_text("The corner handles keep the layer's proportions; Shift while dragging does the opposite");
                    help(ui, "Drag a layer to move it; drag a handle to scale it, the round one to turn it (Shift: 15 degree steps). Arrow keys: 1 pixel, Shift+arrow: 10 pixels.");
                }
                // Stickers P8: the Crop, Text and Shape tools.
                StickerTool::Crop => {
                    let mut hits = Vec::new();
                    crate::sticker::tools::crop::options(ui, state, &mut actions, &mut hits);
                    state.panel_rects.extend(hits);
                    help(ui, "Drag the box's handles, drag inside to move it, or drag outside for a new box. Enter applies.");
                }
                StickerTool::Text => {
                    if let Some(target) = crate::panels::text_block::target(state) {
                        crate::panels::text_block::show(ui, state, ps, target, &mut actions);
                    }
                    help(ui, "Click to add a text layer; click a text layer to edit it. Warp and path: the Text tab.");
                }
                StickerTool::Shape => {
                    let active = crate::sticker::layered(state, None).and_then(|(d, id)| {
                        let l = d.layer(id)?;
                        match &l.content {
                            StickerContent::Shape(s) => Some((id, s.clone(), l.locked)),
                            _ => None,
                        }
                    });
                    if active.is_some() {
                        help(ui, "These options edit the active shape layer.");
                    }
                    crate::sticker::tools::shape::options(ui, state, ps, active, &mut actions);
                    help(ui, "Drag to add a shape layer (a click: a 64-pixel shape). Fill: primary colour; outline: secondary colour.");
                }
                StickerTool::Hand => help(ui, "Drag to pan, wheel to zoom. A right or middle drag pans with every tool."),
                // Stickers P7a: the paint tools.
                StickerTool::Brush | StickerTool::Eraser => {
                    let pen_line = state.pen.status_line();
                    let b = if tool == StickerTool::Eraser { &mut state.tools.eraser } else { &mut state.tools.brush };
                    crate::panels::tool_options::brush_options(ui, if tool == StickerTool::Eraser { "sticker_eraser" } else { "sticker_brush" }, b, &pen_line);
                    help(ui, if tool == StickerTool::Eraser {
                        "Drag to erase. On a mask the eraser hides."
                    } else {
                        "Drag to paint the primary colour. On a mask: white shows, black hides."
                    });
                }
                StickerTool::Fill => {
                    fill_options(ui, state);
                    help(ui, "Click to fill the colour region under the pointer (with a selection: the selection).");
                }
                StickerTool::Gradient => {
                    gradient_options(ui, state, &mut actions);
                    help(ui, "Drag from where the gradient starts to where it ends (limited by the selection).");
                }
                StickerTool::Eyedropper => help(ui, "Click or drag to pick a colour of the sticker into the primary colour."),
                // Stickers P7b: the selection tools and the Magic Eraser.
                StickerTool::Marquee => {
                    select_mode_row(ui, state);
                    marquee_options(ui, state);
                    help(ui, "Drag a box. Shift adds, Alt subtracts, both intersect; a click drops the selection.");
                }
                StickerTool::Lasso => {
                    select_mode_row(ui, state);
                    lasso_options(ui, state, &mut actions);
                    help(ui, "Drag to draw freehand, or click corners and close with a double-click, Enter or Close.");
                }
                StickerTool::Wand => {
                    select_mode_row(ui, state);
                    region_options(ui, "sticker_wand", &mut state.sticker_ui.wand.region);
                    ui.checkbox(&mut state.sticker_ui.wand.all_layers, "Sample all layers")
                        .on_hover_text("Look at every visible layer (without styles); off: the active layer alone");
                    help(ui, "Click a colour to select it. Shift adds, Alt subtracts, both intersect.");
                }
                StickerTool::MagicEraser => {
                    region_options(ui, "sticker_magic_eraser", &mut state.sticker_ui.magic_eraser.region);
                    ui.checkbox(&mut state.sticker_ui.magic_eraser.into_mask, "Into mask (keep pixels)")
                        .on_hover_text("Hide the colour on the layer mask instead of erasing it (a mask is added when needed)");
                    help(ui, "Click a colour to make it transparent (limited by the selection).");
                }
            }
            help(ui, "Right or middle drag: pan  |  Wheel: zoom  |  Esc or Done: back");
            position_fields(ui, state, ps, &mut actions);
            mask_block(ui, state, &mut actions);
            layer_styles(ui, state, ps, &mut actions);
            // Stickers P8: the active layer's adjustments.
            crate::sticker::adjust_ui::block(ui, state, ps, &mut actions);
            // Stickers P7b: the Selection, Cut-out and Filter commands at the end of the page (the controls above keep
            // their places).
            command_blocks(ui, state, &mut actions);
        });
    });
    state.actions.extend(actions);
    out.response.rect
}

/// Stickers P7b: the mode of a new selection (New, Add, Subtract, Intersect), for a pen or a tablet without keys.
fn select_mode_row(ui: &mut egui::Ui, state: &mut AppState) {
    let mut hits = Vec::new();
    crate::sticker::popover::mode_row(ui, &mut state.sticker_ui.select_mode, &mut hits);
    state.panel_rects.extend(hits);
}

/// Stickers P7b: the Marquee's options (shape, feather, fixed aspect).
fn marquee_options(ui: &mut egui::Ui, state: &mut AppState) {
    let m = &mut state.sticker_ui.marquee;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut m.ellipse, false, "Rectangle");
        ui.selectable_value(&mut m.ellipse, true, "Ellipse");
    });
    ui.horizontal(|ui| {
        ui.label("Feather");
        ui.add(egui::DragValue::new(&mut m.feather).range(0.0..=wte_core::sticker::select::MAX_MODIFY).speed(0.2).max_decimals(1).suffix(" px"));
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut m.fixed_aspect, "Fixed aspect");
        ui.add_enabled(m.fixed_aspect, egui::DragValue::new(&mut m.aspect[0]).range(0.01..=100.0).speed(0.05).max_decimals(2));
        ui.label(":");
        ui.add_enabled(m.fixed_aspect, egui::DragValue::new(&mut m.aspect[1]).range(0.01..=100.0).speed(0.05).max_decimals(2));
    });
}

/// Stickers P7b: the Lasso's feather and the buttons that close or drop an open polygon (also Enter / Esc).
fn lasso_options(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let mut hits = Vec::new();
    ui.horizontal(|ui| {
        ui.label("Feather");
        ui.add(egui::DragValue::new(&mut state.sticker_ui.lasso_feather).range(0.0..=wte_core::sticker::select::MAX_MODIFY).speed(0.2).max_decimals(1).suffix(" px"));
    });
    let open = state.sticker_ui.lasso.as_ref().map_or(0, |l| l.points.len());
    ui.horizontal(|ui| {
        let close = ui.add_enabled(open >= 3, egui::Button::new("Close lasso")).on_hover_text("Close the polygon (Enter or a double-click)");
        hits.push((PanelHit::StickerLassoClose, close.rect));
        if close.clicked() {
            actions.push(sticker(StickerAction::CloseLasso));
        }
        let cancel = ui.add_enabled(open > 0, egui::Button::new("Cancel")).on_hover_text("Drop the open polygon (Esc)");
        hits.push((PanelHit::StickerLassoCancel, cancel.rect));
        if cancel.clicked() {
            actions.push(sticker(StickerAction::CancelLasso));
        }
    });
    state.panel_rects.extend(hits);
}

/// Stickers P7b: Tolerance, Contiguous and Anti-alias of the Magic Wand and the Magic Eraser.
fn region_options(ui: &mut egui::Ui, id: &str, opts: &mut wte_core::RegionOptions) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.label("Tolerance");
            ui.add(egui::Slider::new(&mut opts.tolerance, 0..=255)).on_hover_text("How different a colour may be and still count");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut opts.contiguous, "Contiguous").on_hover_text("Only the connected region; off: every matching pixel");
            ui.checkbox(&mut opts.anti_alias, "Anti-alias").on_hover_text("A soft edge around the region");
        });
    });
}

/// Stickers P7b (STICKER_PLAN 3.3 "Commands ... also in the Tool tab"): the Selection, Cut-out and Filter commands as
/// buttons (the same list as the Sticker menu), with Remove Background's options above its button.
fn command_blocks(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    use crate::sticker::filters::{COMMANDS, Group, available, available_more, enabled_more};
    let have = available(state);
    let more = available_more(state);
    if !have.0 {
        return;
    }
    let mut hits = Vec::new();
    for group in Group::ALL {
        ui.separator();
        egui::CollapsingHeader::new(egui::RichText::new(group.label()).strong()).id_salt(("sticker_commands", group.label())).default_open(true).show(ui, |ui| {
            if group == Group::CutOut {
                let bg = &mut state.sticker_ui.remove_bg;
                ui.horizontal(|ui| {
                    ui.label("Tolerance");
                    ui.add(egui::Slider::new(&mut bg.opts.tolerance, 0..=255)).on_hover_text("How far from the background colour a pixel may be");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut bg.opts.follow_gradients, "Follow gradients").on_hover_text("Also a background that changes slowly");
                    ui.checkbox(&mut bg.opts.enclosed, "Also enclosed areas").on_hover_text("Also background colour inside the object (letter holes)");
                    ui.checkbox(&mut bg.into_mask, "Into mask").on_hover_text("Hide the background on the layer mask; the pixels are kept");
                });
            }
            ui.horizontal_wrapped(|ui| {
                for (i, def) in COMMANDS.iter().enumerate().filter(|(_, d)| d.group == group) {
                    let r = ui.add_enabled(enabled_more(def, have, more), egui::Button::new(def.label)).on_hover_text(def.hover);
                    hits.push((PanelHit::StickerCommand(i as u8), r.rect));
                    if r.clicked() {
                        actions.push(sticker((def.make)(state)));
                    }
                }
            });
        });
    }
    state.panel_rects.extend(hits);
}

/// Stickers P7a: the Fill tool's options (Tolerance, Contiguous, Anti-alias).
fn fill_options(ui: &mut egui::Ui, state: &mut AppState) {
    let opts = &mut state.sticker_ui.fill;
    ui.horizontal(|ui| {
        ui.label("Tolerance");
        ui.add(egui::Slider::new(&mut opts.tolerance, 0..=255)).on_hover_text("How different a colour may be and still be filled");
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut opts.contiguous, "Contiguous").on_hover_text("Only the connected region; off: every matching pixel");
        ui.checkbox(&mut opts.anti_alias, "Anti-alias").on_hover_text("A soft edge around the region");
    });
}

/// Stickers P7a: the Gradient tool's options (what it draws, Opacity, Reverse, the stops of the custom gradient).
fn gradient_options(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    use crate::sticker::tools::gradient::GradientMode;
    let _ = actions;
    let mut hits = Vec::new();
    let g = &mut state.sticker_ui.gradient;
    let combo = egui::ComboBox::from_id_salt("sticker_gradient_mode").selected_text(g.mode.label()).show_ui(ui, |ui| {
        for m in GradientMode::ALL {
            ui.selectable_value(&mut g.mode, m, m.label());
        }
    });
    hits.push((PanelHit::StickerGradientMode, combo.response.rect));
    ui.horizontal(|ui| {
        ui.label("Opacity");
        ui.add(crate::panels::percent_slider(&mut g.opacity, 1.0, 100.0).max_decimals(0));
    });
    let rev = ui.checkbox(&mut g.reverse, "Reverse").on_hover_text("The end colour where the drag starts");
    hits.push((PanelHit::StickerGradientReverse, rev.rect));
    if g.mode == GradientMode::Custom {
        let mut remove = None;
        let n = g.stops.len();
        for (i, (t, c)) in g.stops.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.color_edit_button_srgba_unmultiplied(c);
                ui.add(egui::Slider::new(t, 0.0..=1.0).max_decimals(2).text("at"));
                if n > 2 && ui.small_button("x").on_hover_text("Remove this stop").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            g.remove_stop(i);
        }
        let add = ui
            .add_enabled(g.stops.len() < wte_core::MAX_GRADIENT_STOPS, egui::Button::new("Add stop"))
            .on_hover_text("A new stop in the widest gap (at most 8)");
        hits.push((PanelHit::StickerGradientAddStop, add.rect));
        if add.clicked() {
            g.add_stop();
        }
    }
    state.panel_rects.extend(hits);
}

/// Stickers P7a (STICKER_PLAN 3.2 "Mask"): the active picture layer's mask: Add (from the selection when there is
/// one), Invert, Apply, Delete and "Paint on mask"; a text or shape layer gets the Rasterize button instead.
fn mask_block(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let Some(layer) = shown.layers().and_then(|d| edit.active_layer.and_then(|id| d.layer(id))).cloned() else { return };
    let mut hits = Vec::new();
    ui.separator();
    if !matches!(layer.content, wte_core::StickerContent::Picture(_)) {
        ui.label(egui::RichText::new("Text or shape layer").strong());
        ui.label(egui::RichText::new("The paint tools need a picture layer.").weak());
        let r = ui.add_enabled(!layer.locked, egui::Button::new("Rasterize")).on_hover_text("Make this layer a picture layer, so it can be painted");
        hits.push((PanelHit::StickerRasterize, r.rect));
        if r.clicked() {
            actions.push(sticker(StickerAction::Rasterize));
        }
        state.panel_rects.extend(hits);
        return;
    }
    let has = layer.mask.is_some();
    let selection = edit.selection.is_some();
    ui.label(egui::RichText::new("Mask").strong());
    ui.add_enabled_ui(!layer.locked, |ui| {
        ui.horizontal_wrapped(|ui| {
            let add_label = if selection { "Add from selection" } else { "Add mask" };
            let add = ui
                .add_enabled(!has, egui::Button::new(add_label))
                .on_hover_text("A mask that shows the selected area (without a selection: everything)")
                .on_disabled_hover_text("The layer has a mask already");
            hits.push((PanelHit::StickerMaskAdd, add.rect));
            if add.clicked() {
                actions.push(sticker(StickerAction::AddMask));
            }
            let buttons = [
                (PanelHit::StickerMaskInvert, "Invert", "What was hidden shows and what showed is hidden", StickerAction::InvertMask),
                (PanelHit::StickerMaskApply, "Apply", "The mask becomes the picture's transparency", StickerAction::ApplyMask),
                (PanelHit::StickerMaskDelete, "Delete", "Remove the mask (the picture is unchanged)", StickerAction::DeleteMask),
            ];
            for (hit, label, hover, action) in buttons {
                let r = ui.add_enabled(has, egui::Button::new(label)).on_hover_text(hover).on_disabled_hover_text("Add a mask first");
                hits.push((hit, r.rect));
                if r.clicked() {
                    actions.push(sticker(action));
                }
            }
        });
        let mut on = state.sticker_ui.paint_on_mask && has;
        let r = ui
            .add_enabled(has, egui::Checkbox::new(&mut on, "Paint on mask"))
            .on_hover_text("Brush, Eraser, Fill and Gradient change the mask: white shows, black hides")
            .on_disabled_hover_text("Add a mask first");
        hits.push((PanelHit::StickerPaintOnMask, r.rect));
        if r.changed() {
            actions.push(sticker(StickerAction::SetPaintOnMask(on)));
        }
    });
    state.panel_rects.extend(hits);
}

/// X / Y of the active layer (its translation in layer-space pixels).
fn position_fields(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>) {
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let Some(layer) = shown.layers().and_then(|d| edit.active_layer.and_then(|id| d.layer(id))) else { return };
    let id = layer.id;
    let t = layer.transform.translate;
    ui.separator();
    ui.label(egui::RichText::new(format!("Layer: {}", layer.name)).strong());
    ui.add_enabled_ui(!layer.locked, |ui| {
        ui.horizontal(|ui| {
            let mut x = t.x;
            ui.label("X");
            let rx = ui.add(egui::DragValue::new(&mut x).speed(1.0).max_decimals(1).suffix(" px"));
            continuous_edit(ps, actions, ui, &rx, |commit| sticker(StickerAction::SetLayerPosition(id, [x, t.y], commit)));
            let mut y = t.y;
            ui.label("Y");
            let ry = ui.add(egui::DragValue::new(&mut y).speed(1.0).max_decimals(1).suffix(" px"));
            continuous_edit(ps, actions, ui, &ry, |commit| sticker(StickerAction::SetLayerPosition(id, [t.x, y], commit)));
        });
    });
    // Stickers P8: W, H, Angle, Flip and Reset (the transform box's numbers).
    transform_fields(ui, state, ps, actions);
}

/// Stickers P8: the active layer's width and height (layer-space pixels), angle (degrees, clockwise), Flip H / V and
/// Reset; each change one entry "Transform Sticker Layer" (the box centre kept).
fn transform_fields(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>) {
    use crate::sticker::tools::transform;
    let Some((id, b, t, locked)) = transform::active_box(state) else { return };
    let keep = state.sticker_ui.keep_aspect;
    let mut hits = Vec::new();
    ui.add_enabled_ui(!locked, |ui| {
        let size = transform::size_of(&b, &t);
        ui.horizontal(|ui| {
            let (mut w, mut h) = (size.x, size.y);
            ui.label("W");
            let rw = ui.add(egui::DragValue::new(&mut w).range(0.01..=100_000.0).speed(1.0).max_decimals(1).suffix(" px"));
            ui.label("H");
            let rh = ui.add(egui::DragValue::new(&mut h).range(0.01..=100_000.0).speed(1.0).max_decimals(1).suffix(" px"));
            let want = if w != size.x {
                if keep && size.x > 0.0 { glam::Vec2::new(w, size.y * w / size.x) } else { glam::Vec2::new(w, size.y) }
            } else if h != size.y {
                if keep && size.y > 0.0 { glam::Vec2::new(size.x * h / size.y, h) } else { glam::Vec2::new(size.x, h) }
            } else {
                size
            };
            let t2 = transform::with_size(&b, &t, want);
            crate::panels::text_block::value_edit(ps, actions, ui, &rw.union(rh), t2 != t, |commit| sticker(StickerAction::SetLayerTransform(id, t2, commit)));
        });
        ui.horizontal(|ui| {
            let deg = crate::panels::text_block::wrap_deg(t.rotation.to_degrees());
            let mut new_deg = deg;
            ui.label("Angle");
            let r = ui.add(egui::DragValue::new(&mut new_deg).speed(0.5).max_decimals(1).suffix(" deg"));
            let t2 = if new_deg != deg { b.with_rotation(&t, crate::panels::text_block::wrap_deg(new_deg).to_radians()) } else { t };
            crate::panels::text_block::value_edit(ps, actions, ui, &r, t2 != t, |commit| sticker(StickerAction::SetLayerTransform(id, t2, commit)));
            use crate::sticker::layout::LayerTransformCmd as C;
            for (hit, label, tip, cmd) in [
                (PanelHit::StickerFlipH, "Flip H", "Mirror the layer left to right", C::FlipH),
                (PanelHit::StickerFlipV, "Flip V", "Mirror the layer top to bottom", C::FlipV),
                (PanelHit::StickerTransformReset, "Reset", "Scale 100 %, no turn, no flip (the centre stays)", C::Reset),
            ] {
                let r = ui.button(label).on_hover_text(tip);
                hits.push((hit, r.rect));
                if r.clicked() {
                    actions.push(sticker(StickerAction::LayerTransform(cmd)));
                }
            }
        });
    });
    state.panel_rects.extend(hits);
}

/// Stickers P6b (STICKER_PLAN 3.2): the active layer's styles (the preset row scaled to the artboard, Copy / Paste /
/// Clear for the selected layers, and the style editor); each change is a content edit of the sticker.
fn layer_styles(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>) {
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let Some(doc) = shown.layers() else { return };
    let Some(layer) = edit.active_layer.and_then(|id| doc.layer(id)).cloned() else { return };
    let [w, h] = doc.size();
    let k = w.max(h).max(1) as f32 / 512.0;
    let mut hits = Vec::new();
    ui.separator();
    let head = ui.label(egui::RichText::new("Layer styles").strong());
    hits.push((PanelHit::StickerLayerStyles, head.rect));
    if let Some(preset) = crate::sticker::styles_ui::preset_row(ui, &mut hits, !layer.locked) {
        actions.push(sticker(StickerAction::ApplyLayerPreset(preset)));
    }
    crate::sticker::panels::copy_paste_row(ui, state, crate::sticker::StyleTarget::Layers, !layer.locked, actions, &mut hits);
    let id = layer.id;
    let make = move |styles: wte_core::StyleStack, commit: wte_core::Commit| sticker(StickerAction::SetLayerStyles(id, Box::new(styles), commit));
    let mut cx = crate::sticker::styles_ui::EditorCx { ps, out: actions, hits: &mut hits, make: &make, k, text_note: false, mixed: false };
    crate::sticker::styles_ui::editor(ui, "layer", &layer.styles, !layer.locked, &mut cx);
    state.panel_rects.extend(hits);
}
