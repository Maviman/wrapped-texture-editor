// Owner: app-panels (A4). The Layers section of the side panel (ARCHITECTURE.md 8.7): opacity and blend of the active
// layer, the layer list of the active set (top row = top layer: eye, thumbnail, kind badge, name, lock, alpha lock;
// double-click renames; drag-reorder) and the footer buttons. Every change is an `AppAction`.

use eframe::egui;
use egui::{Color32, Pos2, Rect, Sense, Stroke, vec2};
use wte_core::{BlendMode, Document, Layer, LayerId, LayerKind, Placement, StampId, TextureSet};

use crate::action::AppAction;
use crate::panels::{LAYER_THUMB_SIDE, PanelState, Rename, continuous_edit, paint_thumb, percent_slider};
use crate::state::{AppState, PanelHit};

/// Height of one layer row.
const ROW_HEIGHT: f32 = LAYER_THUMB_SIDE + 6.0;
/// Side of the small toggle icons (eye, lock, alpha lock).
const ICON_SIDE: f32 = 18.0;

/// `MoveLayer::to_index` for dropping the layer at index `from` on the row of the layer at index `target`
/// (indices bottom -> top, as in `TextureSet::layers`): `above` = in the upper half of that row, i.e. the dragged
/// layer goes directly above `target`, else directly below it. `move_layer` removes the layer first and then inserts
/// it at `to_index`.
pub(crate) fn drop_index(from: usize, target: usize, above: bool) -> usize {
    let gap = if above { target + 1 } else { target };
    if gap > from { gap - 1 } else { gap }
}

/// What a primary click on a layer row (its empty part, thumbnail or name) does (8.7): Ctrl toggles the layer in the
/// selection, Shift selects the range from the active layer, a plain click makes it the active and only selected
/// layer — nothing for a plain click on the active layer when it is the only selected one (`multi` = several layers
/// are selected).
pub(crate) fn row_click_action(mods: egui::Modifiers, layer: LayerId, active: bool, multi: bool) -> Option<AppAction> {
    if mods.command {
        Some(AppAction::ToggleLayerSelected(layer))
    } else if mods.shift {
        Some(AppAction::SelectLayerRange(layer))
    } else if !active || multi {
        Some(AppAction::SelectLayer(layer))
    } else {
        None
    }
}

/// The context menu of a layer row (right-click on the row, its thumbnail or its name): "Export selected layers... (N)"
/// when the row is part of a selection of several layers, and "Export this layer...". Opening it changes neither the
/// selection nor the active layer. The items' rects go to `hits` for the self-test.
fn row_menu(ui: &mut egui::Ui, selection: &[LayerId], layer: LayerId, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    if selection.len() > 1 && selection.contains(&layer) {
        let resp = ui.button(format!("Export selected layers... ({})", selection.len()));
        hits.push((PanelHit::LayerMenuExportSelected, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::ShowExportLayers(selection.to_vec()));
            ui.close();
        }
    }
    let resp = ui.button("Export this layer...");
    hits.push((PanelHit::LayerMenuExportThis, resp.rect));
    if resp.clicked() {
        actions.push(AppAction::ShowExportLayers(vec![layer]));
        ui.close();
    }
}

/// Whether a stamp / text layer belongs to a projected (3D) or a UV-placed (2D) stamp: `Some(true)` = 3D,
/// `Some(false)` = 2D, `None` for paint layers (and a link to a missing stamp).
pub(crate) fn stamp_is_3d(doc: &Document, layer: &Layer) -> Option<bool> {
    match &layer.kind {
        LayerKind::Stamp { stamp, .. } | LayerKind::Text { stamp, .. } => {
            doc.stamp(*stamp).map(|s| matches!(s.placement, Placement::Projected(_)))
        }
        LayerKind::Raster(_) => None,
    }
}

/// The badge text of a layer kind: "Paint", or "3D stamp" / "2D stamp" / "3D text" / "2D text" (the two stamp kinds
/// are separate and edited in their own view, so the list says which one a layer is).
pub(crate) fn kind_badge(doc: &Document, layer: &Layer) -> &'static str {
    let dim = stamp_is_3d(doc, layer);
    match (layer.kind_name(), dim) {
        ("stamp", Some(true)) => "3D stamp",
        ("stamp", Some(false)) => "2D stamp",
        ("stamp", None) => "Stamp",
        ("text", Some(true)) => "3D text",
        ("text", Some(false)) => "2D text",
        ("text", None) => "Text",
        _ => "Paint",
    }
}

/// Round 6 (R6-2): whether a stamp / text layer belongs to a 3D stamp wrapped on the surface (`SurfaceMode::Wrap`).
pub(crate) fn stamp_is_wrapped(doc: &Document, layer: &Layer) -> bool {
    match &layer.kind {
        LayerKind::Stamp { stamp, .. } | LayerKind::Text { stamp, .. } => doc
            .stamp(*stamp)
            .is_some_and(|s| matches!(s.placement, Placement::Projected(p) if p.mode == wte_core::SurfaceMode::Wrap)),
        LayerKind::Raster(_) => false,
    }
}

/// The hover text of the kind badge.
pub(crate) fn kind_tip(doc: &Document, layer: &Layer) -> &'static str {
    match stamp_is_3d(doc, layer) {
        Some(true) if stamp_is_wrapped(doc, layer) => {
            "Wrapped on the surface of the model: move, scale and rotate it in the 3D view"
        }
        Some(true) => "Projected onto the model: move, scale and rotate it in the 3D view",
        Some(false) => "Placed on the texture: move, scale and rotate it in the 2D view",
        None => "Paint layer",
    }
}

/// Draws the small "3D" / "2D" tag over the lower left corner of a stamp layer's thumbnail.
fn paint_dim_tag(painter: &egui::Painter, thumb: Rect, is_3d: bool, color: Color32) {
    let text = if is_3d { "3D" } else { "2D" };
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(9.0), Color32::WHITE);
    let size = galley.size() + vec2(4.0, 1.0);
    let rect = Rect::from_min_size(Pos2::new(thumb.left(), thumb.bottom() - size.y), size);
    painter.rect_filled(rect, 2.0, color.gamma_multiply(0.9));
    painter.galley(rect.min + vec2(2.0, 0.5), galley, Color32::WHITE);
}

fn badge_color(layer: &Layer) -> Color32 {
    match layer.kind_name() {
        "stamp" => Color32::from_rgb(200, 140, 60),
        "text" => Color32::from_rgb(150, 110, 210),
        _ => Color32::from_rgb(80, 150, 110),
    }
}

/// Draws the Layers tab into the lower area of the side panel, below the tab strip (round 4: the strip replaced the
/// "Layers" collapsing header, FEATURE_BRUSH 6.2): the active layer's opacity and blend, the layer list of the active
/// set in its own scroll area, and below it, outside the scroll area, the footer buttons. Records the row controls,
/// the list's visible rect and the footer in `state.panel_rects`.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    let out = ui.scope(|ui| {
        let mut actions = Vec::new();
        let mut hits = Vec::new();
        body(ui, state, ps, &mut actions, &mut hits);
        state.actions.extend(actions);
        state.panel_rects.extend(hits);
    });
    // Where the tab's content ends (the Layers area hugs it until the user sizes the area).
    ps.layers_content_bottom = Some(out.response.rect.bottom());
}

fn body(ui: &mut egui::Ui, state: &AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    let Some(session) = state.session.as_ref() else {
        ui.weak("No document open");
        return;
    };
    let doc = &session.doc;
    let Some(set) = doc.set(session.active_set) else { return };
    if doc.sets.len() > 1 {
        ui.label(egui::RichText::new(format!("Texture set: {}", set.name)).weak());
    }

    // Opacity and blend of the active layer.
    if let Some(layer) = set.active_layer() {
        let id = layer.id;
        ui.horizontal(|ui| {
            ui.label("Opacity");
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).clamp(50.0, 120.0);
            let mut opacity = layer.opacity;
            let resp = ui.add(percent_slider(&mut opacity, 0.0, 100.0).max_decimals(0));
            continuous_edit(ps, actions, ui, &resp, |commit| AppAction::SetLayerOpacity(id, opacity, commit));
            egui::ComboBox::from_id_salt("active_layer_blend")
                .selected_text(layer.blend.label())
                .width(76.0)
                .show_ui(ui, |ui| {
                    for mode in BlendMode::ALL {
                        if ui.selectable_label(layer.blend == mode, mode.label()).clicked() && layer.blend != mode {
                            actions.push(AppAction::SetLayerBlend(id, mode));
                        }
                    }
                })
                .response
                .on_hover_text("Blend mode of the active layer");
        });
    } else {
        ui.weak("No active layer");
    }
    ui.add_space(2.0);

    // The list scrolls in the room the footer (measured in the last frame) leaves; the footer follows it directly.
    let list_height = (ui.available_height() - ps.layers_footer_height - ui.spacing().item_spacing.y).max(ROW_HEIGHT);
    let list = egui::ScrollArea::vertical()
        .id_salt("side_layers_scroll")
        .max_height(list_height)
        .auto_shrink([false, true])
        .show(ui, |ui| layer_list(ui, state, ps, set, actions, hits));
    // `inner_rect` is the largest viewport; the list shrinks to its rows when they need less.
    let shown = list.content_size.y.min(list.inner_rect.height());
    ps.layers_list_overflow = (list.content_size.y - shown).max(0.0);
    hits.push((PanelHit::LayersList, Rect::from_min_size(list.inner_rect.min, vec2(list.inner_rect.width(), shown))));

    let footer_rect = footer(ui, state, actions, hits);
    ps.layers_footer_height = footer_rect.height().max(0.0);
    hits.push((PanelHit::LayersFooter, footer_rect));
}

/// The rows of the active set, top row = top layer, inside a drop zone for drag-reordering.
fn layer_list(
    ui: &mut egui::Ui,
    state: &AppState,
    ps: &mut PanelState,
    set: &TextureSet,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    let Some(session) = state.session.as_ref() else { return };
    let selection = session.layer_selection();
    let mut rows: Vec<(usize, Rect)> = Vec::with_capacity(set.layers.len());
    let frame = egui::Frame::new().inner_margin(egui::Margin::same(2)).corner_radius(4.0);
    let (zone, dropped) = ui.dnd_drop_zone::<LayerId, ()>(frame, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 1.0;
        for (index, layer) in set.layers.iter().enumerate().rev() {
            let active = set.active == Some(layer.id);
            let drag_id = egui::Id::unique(("wte_layer_row", session.scene_generation, layer.id.0));
            let rect = drag_row(ui, drag_id, ps, state, layer, active, &selection, actions, hits);
            rows.push((index, rect));
        }
    });

    // Insertion marker while dragging, and the drop itself.
    let pointer = ui.ctx().pointer_interact_pos();
    let dragging = egui::DragAndDrop::payload::<LayerId>(ui.ctx());
    let target = pointer.filter(|p| zone.response.rect.contains(*p)).and_then(|p| drop_target(&rows, p));
    if let (Some(dragged), Some((_, _, line_y))) = (&dragging, target)
        && set.layer_index(**dragged).is_some()
    {
        ui.painter().hline(zone.response.rect.x_range(), line_y, Stroke::new(2.0, crate::theme::ACCENT));
    }
    if let Some(dropped) = dropped
        && let Some(from) = set.layer_index(*dropped)
        && let Some((target_index, above, _)) = target
    {
        let to_index = drop_index(from, target_index, above);
        if to_index != from {
            actions.push(AppAction::MoveLayer { layer: *dropped, to_index });
        }
    }
}

/// The row under `pointer`: (layer index, upper half?, y of the insertion line).
fn drop_target(rows: &[(usize, Rect)], pointer: Pos2) -> Option<(usize, bool, f32)> {
    let (index, rect) = rows
        .iter()
        .min_by(|a, b| {
            let da = (a.1.center().y - pointer.y).abs();
            let db = (b.1.center().y - pointer.y).abs();
            da.total_cmp(&db)
        })
        .copied()?;
    let above = pointer.y < rect.center().y;
    Some((index, above, if above { rect.top() } else { rect.bottom() }))
}

/// One layer row as a drag source for reordering, with its click controls on top.
///
/// The row's own `click_and_drag` widget is registered *before* the controls are drawn, so every control lies above
/// it: egui's hit test then gives a click on the eye / lock / alpha lock / thumbnail / name to that control and still
/// lets a drag that starts anywhere on the row reorder it (the same layering as a button on a scroll area). Wrapping
/// the row in `Ui::dnd_drag_source` would do it the other way round: its drag-only widget is registered last, on top
/// of the whole row, and swallows every click (hit_test.rs: a drag-only widget on top hides the click widgets below).
/// A click on the empty part of the row selects the layer, a double-click there renames it.
#[expect(clippy::too_many_arguments, reason = "one row of the list, drawn from the body's loop")]
fn drag_row(
    ui: &mut egui::Ui,
    drag_id: egui::Id,
    ps: &mut PanelState,
    state: &AppState,
    layer: &Layer,
    active: bool,
    selection: &[LayerId],
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) -> Rect {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), ROW_HEIGHT));
    let row_resp = ui.interact(rect, drag_id, Sense::click_and_drag());
    // Only the primary button reorders: a right-button press that moves a little opens the menu instead.
    let dragging = row_resp.dragged_by(egui::PointerButton::Primary);
    if dragging {
        egui::DragAndDrop::set_payload(ui.ctx(), layer.id);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if row_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    // While dragged, the row is painted on a floating layer that follows the pointer (as `dnd_drag_source` does).
    let float_layer = egui::LayerId::new(egui::Order::Tooltip, drag_id);
    let mut builder = egui::UiBuilder::new().max_rect(rect).id_salt(("layer_row_body", layer.id.0));
    if dragging {
        builder = builder.layer_id(float_layer);
    }
    let mut child = ui.new_child(builder);
    row(&mut child, ps, state, layer, active, selection, actions, hits);
    ui.advance_cursor_after_rect(rect);
    if dragging && let Some(pointer) = ui.ctx().pointer_interact_pos() {
        ui.ctx().transform_layer_shapes(float_layer, egui::emath::TSTransform::from_translation(pointer - rect.center()));
    }

    if row_resp.double_clicked() && ps.rename.is_none() {
        ps.rename = Some(Rename { layer: layer.id, text: layer.name.clone(), focus: true });
    } else if row_resp.clicked()
        && let Some(action) = row_click_action(ui.input(|i| i.modifiers), layer.id, active, selection.len() > 1)
    {
        actions.push(action);
    }
    row_resp.context_menu(|ui| row_menu(ui, selection, layer.id, actions, hits));
    hits.push((PanelHit::LayerRow(layer.id), rect));
    rect
}

/// One layer row: eye, thumbnail, kind badge + name (double-click renames), lock, alpha lock. The active layer's row
/// is highlighted and its name bold; a row that is selected but not active gets a lighter highlight and an accent bar
/// at its left edge.
#[expect(clippy::too_many_arguments, reason = "one row of the list, drawn from drag_row")]
fn row(
    ui: &mut egui::Ui,
    ps: &mut PanelState,
    state: &AppState,
    layer: &Layer,
    active: bool,
    selection: &[LayerId],
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    let Some(doc) = state.doc() else { return };
    let multi = selection.len() > 1;
    let mods = ui.input(|i| i.modifiers);
    let width = ui.available_width();
    // Background placeholder, filled once the row's rect is known.
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = ui.horizontal(|ui| {
        ui.set_min_size(vec2(width, ROW_HEIGHT));
        ui.spacing_mut().item_spacing.x = 4.0;

        // Eye.
        let (rect, resp) = ui.allocate_exact_size(vec2(ICON_SIDE, ROW_HEIGHT), Sense::click());
        hits.push((PanelHit::LayerEye(layer.id), rect));
        paint_eye(ui.painter(), rect, layer.visible, icon_color(ui, &resp));
        let tip = if layer.visible { "Hide layer" } else { "Show layer" };
        if resp.on_hover_text(tip).clicked() {
            actions.push(AppAction::SetLayerVisible(layer.id, !layer.visible));
        }

        // Thumbnail.
        let (rect, resp) = ui.allocate_exact_size(vec2(LAYER_THUMB_SIDE, LAYER_THUMB_SIDE), Sense::click());
        hits.push((PanelHit::LayerThumb(layer.id), rect));
        let tex = ps.layer_thumb(ui.ctx(), doc, layer.id);
        let set = doc.find_layer(layer.id).and_then(|(s, _)| doc.set(s));
        let aspect = set.map_or(1.0, |s| s.width as f32 / s.height.max(1) as f32);
        paint_thumb(ui.painter(), rect, tex, aspect);
        if !layer.visible {
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(120));
        }
        if let Some(is_3d) = stamp_is_3d(doc, layer) {
            paint_dim_tag(ui.painter(), rect, is_3d, badge_color(layer));
        }
        if resp.clicked()
            && let Some(action) = row_click_action(mods, layer.id, active, multi)
        {
            actions.push(action);
        }
        resp.context_menu(|ui| row_menu(ui, selection, layer.id, actions, hits));

        // Badge + name (or the rename field).
        let right_width = 2.0 * ICON_SIDE + 8.0;
        let name_width = (ui.available_width() - right_width).max(40.0);
        ui.allocate_ui_with_layout(vec2(name_width, ROW_HEIGHT), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(3.0);
            let renaming = ps.rename.as_ref().is_some_and(|r| r.layer == layer.id);
            if renaming {
                rename_field(ui, ps, layer, actions);
            } else {
                let text = egui::RichText::new(&layer.name);
                let text = if active { text.strong() } else { text };
                let resp = ui.add(egui::Label::new(text).truncate().selectable(false).sense(Sense::click()));
                hits.push((PanelHit::LayerName(layer.id), resp.rect));
                if resp.double_clicked() {
                    ps.rename = Some(Rename { layer: layer.id, text: layer.name.clone(), focus: true });
                } else if resp.clicked()
                    && let Some(action) = row_click_action(mods, layer.id, active, multi)
                {
                    actions.push(action);
                }
                resp.context_menu(|ui| row_menu(ui, selection, layer.id, actions, hits));
                resp.on_hover_text(
                    "Click: select  |  Ctrl / Shift-click: select more  |  Right-click: export  |  Double-click: rename  |  Drag: reorder",
                );
            }
            let badge = egui::RichText::new(kind_badge(doc, layer)).small().color(badge_color(layer));
            let mut extra = String::new();
            if layer.opacity < 1.0 {
                extra = format!("  {:.0} %", layer.opacity * 100.0);
            }
            if layer.blend != BlendMode::Normal {
                extra.push_str("  ");
                extra.push_str(layer.blend.label());
            }
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(badge).selectable(false)).on_hover_text(kind_tip(doc, layer));
                if !extra.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(extra).small().weak()).selectable(false));
                }
            });
        });

        // Lock and alpha lock (right-aligned).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(ICON_SIDE, ROW_HEIGHT), Sense::click());
            hits.push((PanelHit::LayerAlphaLock(layer.id), rect));
            paint_alpha_lock(ui.painter(), rect, layer.alpha_lock, icon_color(ui, &resp));
            let tip = if layer.alpha_lock { "Alpha lock on: painting keeps the transparency" } else { "Alpha lock off" };
            if resp.on_hover_text(tip).clicked() {
                actions.push(AppAction::SetLayerAlphaLock(layer.id, !layer.alpha_lock));
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(ICON_SIDE, ROW_HEIGHT), Sense::click());
            hits.push((PanelHit::LayerLock(layer.id), rect));
            // A stamp protected by its locked layer in another set shows a dimmed closed padlock here, so the lock that
            // blocks it is not invisible (its own lock is still the one this icon toggles).
            let elsewhere = locked_elsewhere(doc, layer);
            let color = icon_color(ui, &resp);
            if elsewhere.is_empty() {
                paint_lock(ui.painter(), rect, layer.locked, color);
            } else {
                paint_lock(ui.painter(), rect, true, color.linear_multiply(0.45));
            }
            let tip = if layer.locked {
                "Locked: click to unlock".to_owned()
            } else if !elsewhere.is_empty() {
                format!("The stamp is locked by its layer in {}. Click to lock this layer too", set_names_text(&elsewhere))
            } else {
                "Click to lock the layer".to_owned()
            };
            if resp.on_hover_text(tip).clicked() {
                actions.push(AppAction::SetLayerLocked(layer.id, !layer.locked));
            }
        });
    });
    let rect = inner.response.rect;
    let visuals = ui.visuals();
    let selected = !active && selection.contains(&layer.id);
    let fill = if active {
        visuals.selection.bg_fill.linear_multiply(0.6)
    } else if selected {
        visuals.selection.bg_fill.linear_multiply(0.3)
    } else {
        visuals.extreme_bg_color
    };
    if selected {
        let bar = Rect::from_min_size(rect.min, vec2(2.0, rect.height()));
        let shapes = vec![egui::Shape::rect_filled(rect, 3.0, fill), egui::Shape::rect_filled(bar, 0.0, crate::theme::ACCENT)];
        ui.painter().set(bg, egui::Shape::Vec(shapes));
    } else {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 3.0, fill));
    }
}

/// The in-place rename field: Enter, a click elsewhere or Esc commit; an empty name keeps the old one.
fn rename_field(ui: &mut egui::Ui, ps: &mut PanelState, layer: &Layer, actions: &mut Vec<AppAction>) {
    let Some(rename) = ps.rename.as_mut() else { return };
    let resp = ui.add(egui::TextEdit::singleline(&mut rename.text).id_salt(("layer_rename", layer.id.0)).desired_width(f32::INFINITY));
    if rename.focus {
        resp.request_focus();
        rename.focus = false;
    } else if resp.lost_focus() || !resp.has_focus() {
        let name = rename.text.trim().to_owned();
        if !name.is_empty() && name != layer.name {
            actions.push(AppAction::RenameLayer(layer.id, name));
        }
        ps.rename = None;
    }
}

/// True when `Document::merge_down(layer)` can succeed: the layer is unlocked, and directly below it in `set` lies an
/// unlocked paint layer (the same checks as the core, so the footer, the Layer menu and Ctrl+M agree with it).
pub(crate) fn can_merge_down(set: &TextureSet, layer: &Layer) -> bool {
    if layer.locked {
        return false;
    }
    let Some(index) = set.layer_index(layer.id) else { return false };
    index.checked_sub(1).and_then(|i| set.layers.get(i)).is_some_and(|below| below.is_raster() && !below.locked)
}

/// Names of the texture sets in which a linked layer of the stamp behind `layer` is locked, when that lock is not
/// visible on `layer`'s own row (the layer itself is unlocked, but the stamp is protected by a layer of another set:
/// `Document::stamp_locked`). Empty for paint layers and for stamps that are not locked elsewhere.
pub(crate) fn locked_elsewhere(doc: &Document, layer: &Layer) -> Vec<String> {
    let Some(stamp) = layer.stamp_id() else { return Vec::new() };
    if layer.locked || !doc.stamp_locked(stamp) {
        return Vec::new();
    }
    stamp_lock_sets(doc, stamp)
}

/// Names of the texture sets that hold a locked layer linked to `stamp`, ascending set id.
pub(crate) fn stamp_lock_sets(doc: &Document, stamp: StampId) -> Vec<String> {
    doc.stamp_layers(stamp)
        .into_iter()
        .filter(|(_, id)| doc.layer(*id).is_some_and(|l| l.locked))
        .filter_map(|(set, _)| doc.set(set).map(|s| s.name.clone()))
        .collect()
}

/// Why a locked stamp cannot be edited, naming the texture set(s) whose Layers list shows the lock when the document has
/// more than one set (the Layers list shows only the active set).
pub(crate) fn stamp_lock_message(doc: &Document, stamp: StampId) -> String {
    let sets = stamp_lock_sets(doc, stamp);
    if doc.sets.len() <= 1 || sets.is_empty() {
        "The stamp's layer is locked: unlock it in the Layers list first".to_owned()
    } else {
        format!("The stamp's layer is locked in {}: unlock it there in the Layers list first", set_names_text(&sets))
    }
}

/// `"set \"A\""` / `"sets \"A\" and \"B\""` for messages.
pub(crate) fn set_names_text(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    match quoted.len() {
        0 => String::new(),
        1 => format!("set {}", quoted[0]),
        _ => format!("sets {} and {}", quoted[..quoted.len() - 1].join(", "), quoted[quoted.len() - 1]),
    }
}

/// The footer buttons: New / Duplicate / Merge Down / Rasterize / Delete. Returns the footer's rect.
fn footer(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) -> Rect {
    let Some(session) = state.session.as_ref() else { return Rect::NOTHING };
    let set = session.doc.set(session.active_set);
    let active = set.and_then(|s| s.active_layer());
    let top = ui.cursor().min.y;
    ui.add_space(2.0);
    let selection = session.layer_selection();
    if selection.len() > 1
        && let Some(layer) = active
    {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{} layers selected \u{b7} tools paint on {}", selection.len(), layer.name)).small().weak(),
            )
            .truncate(),
        );
    }
    let row = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if ui.button("New").on_hover_text("New paint layer above the active layer").clicked() {
            actions.push(AppAction::AddPaintLayer);
        }
        let Some(layer) = active else { return };
        // fix_export item 5: with several layers selected, Duplicate and Delete act on all of them (one undo step
        // each); Merge Down stays on the active layer. The labels and tooltips say so.
        let n = selection.len();
        let multi = n > 1;
        let duplicate = ui.button(if multi { format!("Duplicate ({n})") } else { "Duplicate".to_owned() });
        let duplicate = if multi {
            duplicate.on_hover_text(format!("Duplicate the {n} selected layers (one undo step)"))
        } else {
            duplicate
        };
        if duplicate.clicked() {
            actions.push(if multi { AppAction::DuplicateLayers(selection.clone()) } else { AppAction::DuplicateLayer(layer.id) });
        }
        let can_merge = set.is_some_and(|s| can_merge_down(s, layer));
        let merge = ui.add_enabled(can_merge, egui::Button::new("Merge Down")).on_disabled_hover_text("Needs an unlocked layer below");
        let merge = if multi {
            merge.on_hover_text(format!("Merges the active layer, {}, into the layer below (not the other selected layers)", layer.name))
        } else {
            merge
        };
        if merge.clicked() {
            actions.push(AppAction::MergeDown(layer.id));
        }
        let rasterize = ui.add_enabled(!layer.is_raster() && !layer.locked, egui::Button::new("Rasterize"));
        // Polish item 5: like Merge Down, Rasterize acts on the active layer only; with several layers selected the
        // tooltip says so, enabled or not.
        let rasterize = if multi {
            rasterize.on_hover_text(rasterize_multi_hover(&layer.name, None)).on_disabled_hover_text(rasterize_multi_hover(
                &layer.name,
                // check_polish: a paint layer is never rasterized, locked or not (the locked Base must not suggest that
                // unlocking it would help).
                Some(if layer.is_raster() { "it is already a paint layer" } else { "it is locked" }),
            ))
        } else {
            rasterize.on_hover_text("Turn the stamp or text layer into an ordinary paint layer")
        };
        if rasterize.clicked() {
            actions.push(AppAction::RasterizeLayer(layer.id));
        }
        let elsewhere = locked_elsewhere(&session.doc, layer);
        let why = if elsewhere.is_empty() {
            "The layer is locked".to_owned()
        } else {
            format!("The stamp's layer is locked in {}: unlock it there", set_names_text(&elsewhere))
        };
        if multi {
            let deletable = selection.iter().filter_map(|id| session.doc.layer(*id)).any(|l| {
                !l.locked && locked_elsewhere(&session.doc, l).is_empty()
            });
            let delete = ui.add_enabled(deletable, egui::Button::new(format!("Delete ({n})")));
            hits.push((PanelHit::LayersDelete, delete.rect));
            if delete
                .on_hover_text(format!("Delete the {n} selected layers (one undo step); locked layers stay"))
                .on_disabled_hover_text("Every selected layer is locked")
                .clicked()
            {
                actions.push(AppAction::DeleteLayers(selection.clone()));
            }
            return;
        }
        let delete = ui.add_enabled(!layer.locked && elsewhere.is_empty(), egui::Button::new("Delete"));
        hits.push((PanelHit::LayersDelete, delete.rect));
        if delete.on_disabled_hover_text(why).clicked() {
            actions.push(AppAction::DeleteLayer(layer.id));
        }
    });
    Rect::from_min_max(Pos2::new(row.response.rect.left(), top), row.response.rect.max)
}

/// The footer Rasterize's tooltip with several layers selected (polish item 5): it acts on the active layer only;
/// `blocked` = why it cannot (the disabled tooltip).
pub(crate) fn rasterize_multi_hover(active: &str, blocked: Option<&str>) -> String {
    match blocked {
        None => format!("Rasterizes the active layer, {active}, into a paint layer (not the other selected layers)"),
        Some(why) => format!("Rasterize acts on the active layer, {active}, only (not the other selected layers): {why}"),
    }
}

fn icon_color(ui: &egui::Ui, resp: &egui::Response) -> Color32 {
    if resp.hovered() { ui.visuals().widgets.hovered.fg_stroke.color } else { ui.visuals().widgets.inactive.fg_stroke.color }
}

/// An open eye, or a crossed-out one.
pub(super) fn paint_eye(painter: &egui::Painter, rect: Rect, open: bool, color: Color32) {
    let c = rect.center();
    let w = ICON_SIDE * 0.42;
    let h = ICON_SIDE * 0.22;
    let stroke = Stroke::new(1.3, if open { color } else { color.linear_multiply(0.5) });
    let n = 12;
    let upper: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::PI;
            Pos2::new(c.x - w * t.cos(), c.y - h * t.sin())
        })
        .collect();
    let lower: Vec<Pos2> = upper.iter().map(|p| Pos2::new(p.x, 2.0 * c.y - p.y)).collect();
    painter.add(egui::Shape::line(upper, stroke));
    painter.add(egui::Shape::line(lower, stroke));
    if open {
        painter.circle_filled(c, h * 0.75, color);
    } else {
        painter.line_segment([c + vec2(-w, h * 1.4), c + vec2(w, -h * 1.4)], stroke);
    }
}

/// A padlock, closed or open.
fn paint_lock(painter: &egui::Painter, rect: Rect, closed: bool, color: Color32) {
    let c = rect.center();
    let body = Rect::from_center_size(c + vec2(0.0, 2.5), vec2(ICON_SIDE * 0.55, ICON_SIDE * 0.38));
    let stroke = Stroke::new(1.3, if closed { color } else { color.linear_multiply(0.5) });
    if closed {
        painter.rect_filled(body, 1.5, color);
    } else {
        painter.rect_stroke(body, 1.5, stroke, egui::StrokeKind::Inside);
    }
    // Shackle: a half circle above the body, shifted to the right when open.
    let r = body.width() * 0.32;
    let shift = if closed { 0.0 } else { r * 1.2 };
    let base_y = body.top();
    let lift = if closed { 0.0 } else { 2.0 };
    let n = 10;
    let arc: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::PI;
            Pos2::new(c.x + shift - r * t.cos(), base_y - lift - r * 0.4 - r * t.sin())
        })
        .collect();
    let left_foot = Pos2::new(c.x + shift - r, base_y);
    let right_foot = Pos2::new(c.x + shift + r, if closed { base_y } else { base_y - lift - r * 0.4 });
    painter.line_segment([left_foot, arc[0]], stroke);
    painter.line_segment([right_foot, arc[n]], stroke);
    painter.add(egui::Shape::line(arc, stroke));
}

/// Alpha lock: a small checkerboard square, framed when on.
fn paint_alpha_lock(painter: &egui::Painter, rect: Rect, on: bool, color: Color32) {
    let sq = Rect::from_center_size(rect.center(), vec2(ICON_SIDE * 0.55, ICON_SIDE * 0.55));
    let half = sq.width() / 2.0;
    let dim = if on { color } else { color.linear_multiply(0.35) };
    for (dx, dy) in [(0.0, 0.0), (half, half)] {
        painter.rect_filled(Rect::from_min_size(sq.min + vec2(dx, dy), vec2(half, half)), 0.0, dim);
    }
    painter.rect_stroke(sq, 0.0, Stroke::new(if on { 1.5 } else { 1.0 }, dim), egui::StrokeKind::Outside);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::tests::{run_panels, sample_state};
    use wte_core::SampleKind;

    fn apply(order: &[char], from: usize, to: usize) -> Vec<char> {
        let mut v = order.to_vec();
        let x = v.remove(from);
        v.insert(to.min(v.len()), x);
        v
    }

    #[test]
    fn drop_index_places_the_layer_next_to_the_target_row() {
        let layers = ['A', 'B', 'C', 'D']; // bottom -> top
        // A dropped above D: A becomes the top layer.
        assert_eq!(apply(&layers, 0, drop_index(0, 3, true)), ['B', 'C', 'D', 'A']);
        // D dropped below A: D becomes the bottom layer.
        assert_eq!(apply(&layers, 3, drop_index(3, 0, false)), ['D', 'A', 'B', 'C']);
        // C dropped above A: between A and B.
        assert_eq!(apply(&layers, 2, drop_index(2, 0, true)), ['A', 'C', 'B', 'D']);
        // B dropped below D: between C and D.
        assert_eq!(apply(&layers, 1, drop_index(1, 3, false)), ['A', 'C', 'B', 'D']);
        // Dropping on its own row or next to itself changes nothing.
        assert_eq!(drop_index(1, 1, true), 1);
        assert_eq!(drop_index(1, 1, false), 1);
        assert_eq!(drop_index(1, 2, false), 1);
        assert_eq!(drop_index(1, 0, true), 1);
    }

    #[test]
    fn row_clicks_select_toggle_or_extend() {
        use egui::Modifiers as M;
        let l = LayerId(4);
        let act = |mods, active, multi| row_click_action(mods, l, active, multi);
        for (active, multi) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(matches!(act(M::COMMAND, active, multi), Some(AppAction::ToggleLayerSelected(id)) if id == l));
            assert!(matches!(act(M::SHIFT, active, multi), Some(AppAction::SelectLayerRange(id)) if id == l));
            // Ctrl wins over Shift.
            assert!(matches!(act(M::COMMAND | M::SHIFT, active, multi), Some(AppAction::ToggleLayerSelected(_))));
        }
        assert!(matches!(act(M::NONE, false, false), Some(AppAction::SelectLayer(id)) if id == l));
        assert!(matches!(act(M::NONE, false, true), Some(AppAction::SelectLayer(id)) if id == l));
        // A plain click on the active row collapses a multi-selection and does nothing otherwise.
        assert!(matches!(act(M::NONE, true, true), Some(AppAction::SelectLayer(id)) if id == l));
        assert!(act(M::NONE, true, false).is_none());
        // Alt alone is a plain click.
        assert!(matches!(act(M::ALT, false, false), Some(AppAction::SelectLayer(_))));
    }

    #[test]
    fn drop_target_picks_the_nearest_row_and_half() {
        let rows = [
            (1, Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(100.0, 40.0))),
            (0, Rect::from_min_size(Pos2::new(0.0, 40.0), vec2(100.0, 40.0))),
        ];
        assert_eq!(drop_target(&rows, Pos2::new(10.0, 5.0)), Some((1, true, 0.0)));
        assert_eq!(drop_target(&rows, Pos2::new(10.0, 35.0)), Some((1, false, 40.0)));
        assert_eq!(drop_target(&rows, Pos2::new(10.0, 70.0)), Some((0, false, 80.0)));
        assert_eq!(drop_target(&[], Pos2::new(0.0, 0.0)), None);
    }

    #[test]
    fn badges_name_the_layer_kind() {
        let mut state = sample_state(SampleKind::Sphere);
        let doc = state.doc_mut().unwrap();
        let names: Vec<&str> = doc.sets[0].layers.iter().map(|l| kind_badge(doc, l)).collect();
        assert_eq!(names, ["Paint", "Paint"]);

        // One stamp and one text of each kind: the badge says 3D (projected) or 2D (UV-placed).
        let img = wte_core::samples::sample_stamp_image(32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let blob = doc.add_image_blob("s.png", std::sync::Arc::from(png)).unwrap();
        let font = doc.default_font();
        let text = wte_core::StampSource::Text(wte_core::TextSpec {
            content: "Hi".into(),
            font,
            size_px: wte_core::TEXT_PLACEMENT_PX,
            color: [0, 0, 0, 255],
            align: wte_core::TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: wte_core::TextFx::default(),
        });
        let set = wte_core::TextureSetId(0);
        let projected = Placement::Projected(wte_core::Projector::from_surface(
            glam::Vec3::new(0.0, 0.0, 1.0),
            glam::Vec3::Z,
            glam::Vec3::Y,
            glam::Vec2::splat(0.2),
            0.2,
        ));
        let uv = Placement::Uv(wte_core::UvPlacement {
            set,
            center_px: glam::Vec2::splat(300.0),
            size_px: glam::Vec2::splat(100.0),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        doc.add_stamp("p", wte_core::StampSource::Image { blob }, projected, set).unwrap();
        doc.add_stamp("u", wte_core::StampSource::Image { blob }, uv, set).unwrap();
        doc.add_stamp("pt", text.clone(), projected, set).unwrap();
        doc.add_stamp("ut", text, uv, set).unwrap();
        // Round 6: a wrapped 3D stamp is a 3D stamp whose hover text says it is wrapped.
        let Placement::Projected(pr) = projected else { unreachable!() };
        let wrapped = Placement::Projected(wte_core::Projector { mode: wte_core::SurfaceMode::Wrap, ..pr });
        doc.add_stamp("w", wte_core::StampSource::Image { blob }, wrapped, set).unwrap();
        let doc = state.doc().unwrap();
        let badge = |name: &str| {
            let layer = doc.sets[0].layers.iter().find(|l| l.name == name).unwrap();
            (kind_badge(doc, layer), stamp_is_3d(doc, layer))
        };
        assert_eq!(badge("p"), ("3D stamp", Some(true)));
        assert_eq!(badge("u"), ("2D stamp", Some(false)));
        assert_eq!(badge("pt"), ("3D text", Some(true)));
        assert_eq!(badge("ut"), ("2D text", Some(false)));
        assert_eq!(badge("w"), ("3D stamp", Some(true)));
        let tip = |name: &str| kind_tip(doc, doc.sets[0].layers.iter().find(|l| l.name == name).unwrap());
        assert!(tip("w").starts_with("Wrapped on the surface"), "{}", tip("w"));
        assert!(tip("p").starts_with("Projected onto the model"));
        assert!(!stamp_is_wrapped(doc, &doc.sets[0].layers[0]));
        assert_eq!((kind_badge(doc, &doc.sets[0].layers[0]), stamp_is_3d(doc, &doc.sets[0].layers[0])), ("Paint", None));
        // The rows with the tags draw without panicking.
        let mut ps = PanelState::new();
        let mut state = state;
        run_panels(&egui::Context::default(), &mut state, &mut ps, egui::RawInput::default());
    }

    /// Headless frames at 1280x800 with a running clock (double-click detection needs time stamps).
    struct Driver {
        ctx: egui::Context,
        time: f64,
    }

    impl Driver {
        fn new() -> Driver {
            Driver { ctx: egui::Context::default(), time: 0.0 }
        }

        fn frame(&mut self, state: &mut AppState, ps: &mut PanelState, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            run_panels(&self.ctx, state, ps, input);
        }

        fn button(pos: Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
        }

        /// Hover, press, release at `pos`; returns the actions queued meanwhile (the state is not dispatched).
        fn click(&mut self, state: &mut AppState, ps: &mut PanelState, pos: Pos2) -> Vec<AppAction> {
            state.actions.clear();
            self.frame(state, ps, vec![egui::Event::PointerMoved(pos)]);
            self.frame(state, ps, vec![Self::button(pos, true)]);
            self.frame(state, ps, vec![Self::button(pos, false)]);
            std::mem::take(&mut state.actions)
        }

        /// Press at the first point, move through the others (one per frame), release at the last one.
        fn drag(&mut self, state: &mut AppState, ps: &mut PanelState, points: &[Pos2]) -> Vec<AppAction> {
            state.actions.clear();
            self.frame(state, ps, vec![egui::Event::PointerMoved(points[0])]);
            self.frame(state, ps, vec![Self::button(points[0], true)]);
            for p in &points[1..] {
                self.frame(state, ps, vec![egui::Event::PointerMoved(*p)]);
            }
            self.frame(state, ps, vec![Self::button(*points.last().unwrap(), false)]);
            std::mem::take(&mut state.actions)
        }
    }

    fn hit(state: &AppState, h: PanelHit) -> Rect {
        *state.panel_rects.get(&h).unwrap_or_else(|| panic!("{h:?} was not drawn: {:?}", state.panel_rects.keys()))
    }

    #[test]
    fn every_row_control_takes_its_click_and_the_row_still_drags() {
        let mut d = Driver::new();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let doc = state.doc().unwrap();
        let (base, paint) = (doc.sets[0].layers[0].id, doc.sets[0].layers[1].id);
        assert!(doc.sets[0].layers[0].locked, "the sample's Base starts locked");
        assert_eq!(doc.sets[0].active, Some(paint));
        for _ in 0..3 {
            d.frame(&mut state, &mut ps, Vec::new());
        }
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
        for id in [base, paint] {
            assert!(screen.contains_rect(hit(&state, PanelHit::LayerRow(id))), "row of {id:?} is on screen");
        }

        let eye = hit(&state, PanelHit::LayerEye(base)).center();
        assert!(matches!(d.click(&mut state, &mut ps, eye)[..], [AppAction::SetLayerVisible(id, false)] if id == base));
        let lock = hit(&state, PanelHit::LayerLock(base)).center();
        assert!(matches!(d.click(&mut state, &mut ps, lock)[..], [AppAction::SetLayerLocked(id, false)] if id == base));
        let alpha = hit(&state, PanelHit::LayerAlphaLock(paint)).center();
        assert!(matches!(d.click(&mut state, &mut ps, alpha)[..], [AppAction::SetLayerAlphaLock(id, true)] if id == paint));
        let thumb = hit(&state, PanelHit::LayerThumb(base)).center();
        assert!(matches!(d.click(&mut state, &mut ps, thumb)[..], [AppAction::SelectLayer(id)] if id == base));
        let name = hit(&state, PanelHit::LayerName(base)).center();
        assert!(matches!(d.click(&mut state, &mut ps, name)[..], [AppAction::SelectLayer(id)] if id == base));
        // The empty part of the row selects too.
        let row = hit(&state, PanelHit::LayerRow(base));
        let empty = Pos2::new(hit(&state, PanelHit::LayerName(base)).left() + 4.0, row.bottom() - 2.0);
        assert!(matches!(d.click(&mut state, &mut ps, empty)[..], [AppAction::SelectLayer(id)] if id == base));
        // A click on the active layer's name changes nothing; a double-click opens the rename field.
        let name = hit(&state, PanelHit::LayerName(paint)).center();
        assert!(d.click(&mut state, &mut ps, name).is_empty());
        assert!(ps.rename.is_none());
        d.time += 1.0; // the next two clicks are a double-click of their own
        let first = d.click(&mut state, &mut ps, name);
        let second = d.click(&mut state, &mut ps, name);
        assert!(first.is_empty() && second.is_empty(), "{first:?} {second:?}");
        assert!(ps.rename.as_ref().is_some_and(|r| r.layer == paint), "double-click starts the rename");
        ps.rename = None;
        d.frame(&mut state, &mut ps, Vec::new());

        // A drag that starts on the top row's name and ends on the lower half of the bottom row moves it to the bottom.
        let from = hit(&state, PanelHit::LayerName(paint)).center();
        let target = hit(&state, PanelHit::LayerRow(base));
        let to = Pos2::new(from.x, target.bottom() - 4.0);
        let points: Vec<Pos2> = (0..=8).map(|k| from + (to - from) * (k as f32 / 8.0)).collect();
        let actions = d.drag(&mut state, &mut ps, &points);
        assert!(
            matches!(actions[..], [AppAction::MoveLayer { layer, to_index: 0 }] if layer == paint),
            "{actions:?}"
        );
        // Releasing a short drag over its own row moves nothing.
        let from = hit(&state, PanelHit::LayerName(base)).center();
        let actions = d.drag(&mut state, &mut ps, &[from, from + vec2(0.0, 9.0), from + vec2(0.0, 12.0)]);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn rename_commits_on_focus_loss() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let paint = state.doc().unwrap().sets[0].active.unwrap();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        };
        run_panels(&ctx, &mut state, &mut ps, input());
        ps.rename = Some(Rename { layer: paint, text: "Paint 1".into(), focus: true });
        run_panels(&ctx, &mut state, &mut ps, input()); // takes the focus
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(ctx.text_edit_focused(), "the rename field has the focus");
        assert!(state.actions.is_empty());
        ps.rename.as_mut().unwrap().text = "  Details ".into();
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(ps.rename.is_none());
        assert!(
            matches!(&state.actions[..], [AppAction::RenameLayer(id, name)] if *id == paint && name == "Details"),
            "{:?}",
            state.actions
        );
    }
}
