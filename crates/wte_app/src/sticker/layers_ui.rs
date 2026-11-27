// Owner: stickers P6a (STICKER_PLAN 3.2; ARCHITECTURE 8.7, 8.15). The side panel in the Sticker view: the Layers tab
// shows the sticker's layers (eye, thumbnail, name with kind / opacity / blend / "clipped", lock; click selects, drag
// reorders; the active layer's name, opacity and blend above the list; the footer New / Add picture... / Duplicate /
// Up / Down / Delete), and the Tool tab shows the options of the active sticker tool and the active layer's position.
// Every change is a `StickerAction`; every feature has a button (tablet rule of 3.10: nothing only by drag, hover,
// right click, double click or a key).

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
        let (dup, up, down) = (key("Duplicate Sticker Layer"), key("Move Sticker Layer Up"), key("Move Sticker Layer Down"));
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
                    help(ui, "Drag a layer to move it. Arrow keys: 1 pixel, Shift+arrow: 10 pixels.");
                }
                StickerTool::Hand => help(ui, "Drag to pan, wheel to zoom. A right or middle drag pans with every tool."),
                _ => help(ui, "This tool is not available yet."),
            }
            help(ui, "Right or middle drag: pan  |  Wheel: zoom  |  Esc or Done: back");
            position_fields(ui, state, ps, &mut actions);
            layer_styles(ui, state, ps, &mut actions);
        });
    });
    state.actions.extend(actions);
    out.response.rect
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
