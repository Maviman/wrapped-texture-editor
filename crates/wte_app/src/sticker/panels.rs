// Owner: stickers P6b (STICKER_PLAN 3.7, 3.8, 3.9; ARCHITECTURE 8.7, 8.15). The Sticker tab, the fifth tab of the side
// panel's upper group, always available (also outside the Sticker view). Top to bottom: (1) the selected stamp
// (thumbnail, name, kind, Edit Sticker, Export Picture...), (2) Sticker style: the preset row, Copy / Paste / Clear and
// the stamp-level style editor for image, text and sticker stamps without opening the editor (E3); with several stamp
// layers selected it edits all of them (3.7), (3) the tray (`tray.rs`), (4) the library (P8). Stickers P9a: the
// editor link's block under the selected stamp (`link/ui.rs`). Stickers P11 (6.4): the thumbnail and the tray items
// are drag sources (a drag out of the window hands over their finished picture), and the "PSD" handle under the
// thumbnail hands over the stamp's linked PSD (caption "drag to your editor: stays linked"). Every command is an `AppAction`; every control has a button (tablet rule of 3.10) and a rect in
// `state.panel_rects`.

use eframe::egui;
use egui::{Rect, Sense, vec2};
use wte_core::{StampSource, StyleStack};

use crate::action::AppAction;
use crate::panels::{PanelState, paint_thumb};
use crate::state::{AppState, PanelHit};
use crate::sticker::styles_ui::{self, EditorCx};
use crate::sticker::{PictureOf, StickerAction, StyleTarget, selected_stamps, tray};

/// Side of the selected stamp's thumbnail.
const STAMP_THUMB: f32 = 56.0;

fn sticker(action: StickerAction) -> AppAction {
    AppAction::Sticker(action)
}

/// A bold section heading.
fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).strong());
}

/// The Sticker page. Returns the rect of its content.
pub(crate) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) -> Rect {
    let generation = state.session.as_ref().map(|s| s.scene_generation);
    state.sticker_ui.pictures.begin(generation);
    let mut actions = Vec::new();
    let mut hits = Vec::new();
    let out = ui.vertical(|ui| {
        if state.session.is_none() {
            ui.weak("Open a model first.");
            return;
        }
        selected_block(ui, state, ps, &mut actions, &mut hits);
        ui.separator();
        heading(ui, "Sticker tray");
        tray::show(ui, state, &mut actions, &mut hits);
        // Stickers P8 (STICKER_PLAN 3.8, 3.9 item 4): the library.
        ui.separator();
        heading(ui, "Library");
        crate::sticker::library::show(ui, state, &mut actions, &mut hits);
    });
    state.actions.extend(actions);
    state.panel_rects.extend(hits);
    out.response.rect
}

/// (1) the selected stamp and (2) its style.
fn selected_block(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    let stamps = selected_stamps(state);
    let Some(first) = stamps.first().copied() else {
        heading(ui, "Selected stamp");
        ui.weak("Select a stamp or a text (Transform tool, or its layer) to style it or edit it as a sticker.");
        return;
    };
    let Some(object) = state.doc().and_then(|d| d.stamp(first)).cloned() else { return };
    let locked = stamps.iter().any(|id| state.doc().is_some_and(|d| d.stamp_locked(*id)));
    heading(ui, if stamps.len() > 1 { "Selected stamps" } else { "Selected stamp" });
    let (kind, users) = match &object.source {
        StampSource::Image { blob } => ("Image stamp".to_owned(), state.doc().map_or(0, |d| d.blob_users(*blob).len())),
        StampSource::Text(_) => ("Text stamp".to_owned(), 1),
        StampSource::Sticker { sticker } => {
            let layers = state.doc().and_then(|d| d.sticker(*sticker)).map_or(0, |d| d.layers.len());
            let s = if layers == 1 { "" } else { "s" };
            (format!("Sticker, {layers} layer{s}"), state.doc().map_or(0, |d| d.sticker_users(*sticker).len()))
        }
    };
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(STAMP_THUMB, STAMP_THUMB), Sense::drag());
        // Stickers P10a (6.2): a picture dropped here replaces the stamp's picture or becomes a layer of its sticker.
        hits.push((PanelHit::StickerTabThumb, rect));
        // Stickers P11 (6.4): dragged out of the window it hands over the finished picture (with its styles).
        if resp.drag_started_by(egui::PointerButton::Primary) {
            actions.push(AppAction::DragOut(crate::dragout::DragSource::Stamp(first)));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let thumb = match &object.source {
            StampSource::Image { blob } => tray::blob_thumb(ui.ctx(), state, *blob),
            StampSource::Sticker { sticker } => tray::sticker_thumb(ui.ctx(), state, *sticker),
            StampSource::Text(_) => None,
        };
        let aspect = thumb.map_or(1.0, |(_, [w, h])| w as f32 / h.max(1) as f32);
        paint_thumb(ui.painter(), rect, thumb.map(|(t, _)| t), aspect);
        if matches!(object.source, StampSource::Text(_)) {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "T", egui::FontId::proportional(28.0), egui::Color32::from_gray(60));
        }
        ui.vertical(|ui| {
            ui.label(object.name.clone());
            ui.weak(kind);
            if users > 1 {
                ui.weak(format!("Its picture is shown by {users} stamps"));
            }
            if stamps.len() > 1 {
                ui.weak(format!("{} stamps selected: the style below goes to all of them", stamps.len()));
            }
        });
    });
    if psd_handle(ui, first, !matches!(object.source, StampSource::Text(_)) && !locked, actions, hits) {
        state.status_hint = crate::dragout::HANDLE_CLICK_HINT.to_owned();
    }
    let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
    ui.horizontal_wrapped(|ui| {
        let key = crate::shortcuts::shortcut_text(ui.ctx(), "Sticker");
        let resp = ui.button("Edit Sticker").on_hover_text(format!("Open the stamp in the Sticker view ({key})"));
        hits.push((PanelHit::StickerTabEdit, resp.rect));
        if resp.clicked() {
            actions.push(sticker(StickerAction::Enter(Some(first))));
        }
        let resp = ui
            .button(format!("Export Picture...{on_pc}"))
            .on_hover_text("Save the finished picture of the stamp (with its border and shadow) as a PNG");
        hits.push((PanelHit::StickerTabExport, resp.rect));
        if resp.clicked() {
            actions.push(sticker(StickerAction::ExportPicture(PictureOf::Stamp(first), None)));
        }
        // Stickers P8: a text stamp becomes a layered sticker here too (its text stays editable as a text layer).
        if matches!(object.source, StampSource::Text(_)) {
            let resp = ui
                .add_enabled(!locked, egui::Button::new("Convert to layered sticker"))
                .on_hover_text("The text becomes a text layer of a sticker; it stays editable in the Text tab");
            hits.push((PanelHit::StickerTabConvertText, resp.rect));
            if resp.clicked() {
                actions.push(sticker(StickerAction::ConvertTextStamp(first)));
            }
        }
    });

    // Stickers P9a (STICKER_PLAN 3.9, 5.1, 5.5): Edit in <editor> and the link's badge and commands.
    crate::link::ui::tab_block(ui, state, crate::link::LinkTarget::Stamp(first), actions, hits);

    heading(ui, "Sticker style");
    if locked {
        ui.colored_label(ui.visuals().warn_fg_color, "A selected stamp's layer is locked: unlock it to change its style.");
    }
    if let Some(preset) = styles_ui::preset_row(ui, hits, !locked) {
        actions.push(sticker(StickerAction::ApplyPreset(preset)));
    }
    copy_paste_row(ui, state, StyleTarget::Stamps, !locked, actions, hits);
    let mixed = stamps.iter().any(|id| state.doc().and_then(|d| d.stamp(*id)).is_some_and(|s| s.styles != object.styles));
    let k = reference_k(state, first);
    let make_stamps = stamps.clone();
    let make = move |styles: StyleStack, commit: wte_core::Commit| sticker(StickerAction::SetStampStyles(make_stamps.clone(), Box::new(styles), commit));
    let text_note = matches!(object.source, StampSource::Text(_));
    let mut cx = EditorCx { ps, out: actions, hits, make: &make, k, text_note, mixed };
    styles_ui::editor(ui, "stamp", &object.styles, !locked, &mut cx);
}

/// Side of the PSD handle in points (6.4: a visible 24-pt handle).
pub(crate) const PSD_HANDLE: f32 = 24.0;

/// Stickers P11 (STICKER_PLAN 6.4, 13.9): the "PSD" handle and its caption under it (plain text, not a tooltip). A drag
/// on it makes the stamp's link (save first and the shared-picture question at the press) and hands the linked PSD
/// to the drag out; a click only says how it is used. Disabled for a text stamp (it must become a sticker first) and a
/// locked one. True when it was clicked (the caller shows how it is used).
fn psd_handle(ui: &mut egui::Ui, stamp: wte_core::StampId, enabled: bool, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) -> bool {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let sense = if enabled { Sense::click_and_drag() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(vec2(PSD_HANDLE * 2.0, PSD_HANDLE), sense);
        hits.push((PanelHit::StickerTabPsdHandle, rect));
        let v = ui.visuals();
        let (fill, stroke, text) = if enabled {
            let w = v.widgets.style(&resp);
            (w.weak_bg_fill, w.bg_stroke, w.fg_stroke.color)
        } else {
            (v.widgets.noninteractive.weak_bg_fill, v.widgets.noninteractive.bg_stroke, v.weak_text_color())
        };
        ui.painter().rect(rect, 4.0, fill, stroke, egui::StrokeKind::Inside);
        // Grip dots left of the label: it is dragged, not clicked.
        for dy in [-4.0, 0.0, 4.0] {
            for dx in [0.0, 4.0] {
                ui.painter().circle_filled(egui::pos2(rect.left() + 6.0 + dx, rect.center().y + dy), 1.2, text);
            }
        }
        ui.painter().text(rect.center() + vec2(6.0, 0.0), egui::Align2::CENTER_CENTER, "PSD", egui::FontId::proportional(12.0), text);
        if enabled && (resp.hovered() || resp.dragged()) {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let resp = if enabled {
            resp.on_hover_text("Drag this onto your editor (Photoshop: onto its tab bar) to open the linked PSD there; Ctrl+S there updates the sticker")
        } else {
            resp.on_hover_text("A text stamp becomes a layered sticker first (Convert to layered sticker); a locked stamp cannot be linked")
        };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            actions.push(AppAction::DragOut(crate::dragout::DragSource::PsdHandle(stamp)));
        }
        ui.weak(crate::dragout::HANDLE_CAPTION);
        resp.clicked()
    })
    .inner
}

/// Copy / Paste / Clear Style for `target`.
pub(crate) fn copy_paste_row(
    ui: &mut egui::Ui,
    state: &AppState,
    target: StyleTarget,
    enabled: bool,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    let has_copy = state.style_clipboard.is_some();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let resp = ui.button("Copy Style").on_hover_text("Remember these styles to paste them onto other stamps or layers");
        hits.push((PanelHit::StyleCopy, resp.rect));
        if resp.clicked() {
            actions.push(sticker(StickerAction::CopyStyle(target)));
        }
        let resp = ui
            .add_enabled(enabled && has_copy, egui::Button::new("Paste Style"))
            .on_hover_text("Give the selection the copied styles")
            .on_disabled_hover_text("Copy a style first");
        hits.push((PanelHit::StylePaste, resp.rect));
        if resp.clicked() {
            actions.push(sticker(StickerAction::PasteStyle(target)));
        }
        let resp = ui.add_enabled(enabled, egui::Button::new("Clear Style")).on_hover_text("Remove every style");
        hits.push((PanelHit::StyleClear, resp.rect));
        if resp.clicked() {
            actions.push(sticker(StickerAction::ClearStyle(target)));
        }
    });
}

/// The scale of the default numbers of a newly ticked style block for stamp `id` (reference side / 512, as the
/// presets).
fn reference_k(state: &mut AppState, id: wte_core::StampId) -> f32 {
    let size = state.doc_mut().and_then(|d| d.stamp_style_reference_size(id).ok()).unwrap_or([512, 512]);
    size[0].max(size[1]).max(1) as f32 / 512.0
}
