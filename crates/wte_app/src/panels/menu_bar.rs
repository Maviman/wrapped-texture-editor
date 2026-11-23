// Owner: app-panels (A4). The menu bar of ARCHITECTURE.md 8.7: File, Edit, View, Layer, Help. Every entry pushes an
// `AppAction` (or changes a view setting); shortcut texts come from `shortcuts::shortcut_text`.

use std::path::PathBuf;

use eframe::egui;
use egui::Button;
use wte_core::SampleKind;

use crate::action::AppAction;
use crate::shortcuts::shortcut_text;
use crate::state::{AppState, Backdrop, CameraPreset, Dialog, ShadingMode, ViewMode};

/// A menu entry: label, the `shortcut_text` label it shows ("" = none), enabled, selected, action.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, selected: bool) -> bool {
    let key = if shortcut.is_empty() { String::new() } else { shortcut_text(ui.ctx(), shortcut) };
    let button = Button::new(label).shortcut_text(key).selected(selected);
    ui.add_enabled(enabled, button).clicked()
}

/// Draws the menu bar: File, Edit, View, Layer, Help.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let mut actions: Vec<AppAction> = Vec::new();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, state, &mut actions));
        ui.menu_button("Edit", |ui| edit_menu(ui, state, &mut actions));
        ui.menu_button("View", |ui| view_menu(ui, state, &mut actions));
        ui.menu_button("Layer", |ui| layer_menu(ui, state, &mut actions));
        ui.menu_button("Help", |ui| {
            if item(ui, "Shortcuts", "", true, false) {
                actions.push(AppAction::ShowDialog(Dialog::Shortcuts));
            }
            if item(ui, "About", "", true, false) {
                actions.push(AppAction::ShowDialog(Dialog::About));
            }
        });
    });
    state.actions.extend(actions);
}

fn file_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let has_doc = state.session.is_some();
    if item(ui, "New from Model...", "New from Model...", true, false) {
        actions.push(AppAction::NewFromModel(None));
    }
    if item(ui, "Open Project...", "Open Project...", true, false) {
        actions.push(AppAction::OpenProject(None));
    }
    ui.menu_button("Open Recent", |ui| recent_menu(ui, state, actions));
    ui.menu_button("Open Sample", |ui| {
        for (label, kind) in [("Sphere", SampleKind::Sphere), ("Cube", SampleKind::Cube), ("Two Materials", SampleKind::TwoMaterial)] {
            if item(ui, label, "", true, false) {
                actions.push(AppAction::OpenSample(kind));
            }
        }
    });
    ui.separator();
    if item(ui, "Save", "Save", has_doc, false) {
        actions.push(AppAction::Save);
    }
    if item(ui, "Save As...", "Save As...", has_doc, false) {
        actions.push(AppAction::SaveAs(None));
    }
    ui.separator();
    if item(ui, "Import Image as Stamp...", "", has_doc, false) {
        actions.push(AppAction::ImportStampImage(None));
    }
    if item(ui, "Export...", "Export...", has_doc, false) {
        actions.push(AppAction::ShowExport);
    }
    if item(ui, "Extract Original Files...", "", has_doc, false) {
        actions.push(AppAction::ExtractSources(None));
    }
    ui.separator();
    if item(ui, "Exit", "", true, false) {
        actions.push(AppAction::Quit);
    }
}

fn recent_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    if state.prefs.recent.is_empty() {
        ui.add_enabled(false, Button::new("No recent files"));
        return;
    }
    let mut open: Option<PathBuf> = None;
    for path in &state.prefs.recent {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        if ui.add(Button::new(name)).on_hover_text(path.display().to_string()).clicked() {
            open = Some(path.clone());
        }
    }
    if let Some(path) = open {
        actions.push(AppAction::OpenPath(path));
    }
    ui.separator();
    if item(ui, "Clear Recent", "", true, false) {
        state.prefs.recent.clear();
    }
}

fn edit_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let doc = state.doc();
    let undo = doc.and_then(|d| d.undo_label()).map(|l| format!("Undo {l}"));
    let redo = doc.and_then(|d| d.redo_label()).map(|l| format!("Redo {l}"));
    // Delete removes the selected stamp, else the active layer (dispatch `DeleteSelection`): enabled when that target
    // is not protected by a lock.
    let can_delete = state.session.as_ref().is_some_and(|s| {
        match s.selected_stamp.filter(|id| s.doc.stamp(*id).is_some()) {
            Some(stamp) => !s.doc.stamp_locked(stamp),
            None => s.doc.set(s.active_set).and_then(|set| set.active_layer()).is_some_and(|l| {
                !l.locked && l.stamp_id().is_none_or(|stamp| !s.doc.stamp_locked(stamp))
            }),
        }
    });
    if item(ui, undo.as_deref().unwrap_or("Undo"), "Undo", undo.is_some(), false) {
        actions.push(AppAction::Undo);
    }
    if item(ui, redo.as_deref().unwrap_or("Redo"), "Redo", redo.is_some(), false) {
        actions.push(AppAction::Redo);
    }
    ui.separator();
    if item(ui, "Delete", "Delete", can_delete, false) {
        actions.push(AppAction::DeleteSelection);
    }
}

fn view_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let has_doc = state.session.is_some();
    for (label, mode) in
        [("3D", ViewMode::View3D), ("2D", ViewMode::View2D), ("Split", ViewMode::Split), ("Preview", ViewMode::Preview)]
    {
        let enabled = mode != ViewMode::Preview || has_doc;
        if item(ui, label, label, enabled, state.view_mode == mode) {
            actions.push(AppAction::SetViewMode(mode));
        }
    }
    ui.separator();
    let flags = &state.flags;
    if item(ui, "Texture on Model", "Texture on Model", true, flags.shading == ShadingMode::Textured) {
        actions.push(AppAction::ToggleTexture);
    }
    ui.menu_button("Shading", |ui| {
        for (label, mode) in
            [("Textured", ShadingMode::Textured), ("Clay", ShadingMode::Clay), ("UV Checker", ShadingMode::UvChecker)]
        {
            if item(ui, label, "", true, state.flags.shading == mode) {
                actions.push(AppAction::SetShading(mode));
            }
        }
    });
    let flags = &state.flags;
    let toggles = [
        ("Wireframe", "Wireframe", flags.wireframe, AppAction::ToggleWireframe),
        ("UV Seams", "", flags.seams, AppAction::ToggleSeams),
        ("UV Wireframe in 2D", "", flags.uv_wire, AppAction::ToggleUvWire),
        ("Alpha Checker", "", flags.alpha_checker, AppAction::ToggleAlphaChecker),
        ("Lit", "", flags.lit, AppAction::ToggleLit),
    ];
    for (label, shortcut, on, action) in toggles {
        if item(ui, label, shortcut, true, on) {
            actions.push(action);
        }
    }
    ui.menu_button("Background", |ui| {
        for (label, backdrop) in
            [("Gradient", Backdrop::Gradient), ("Dark", Backdrop::Dark), ("Light", Backdrop::Light), ("Checker", Backdrop::Checker)]
        {
            if item(ui, label, "", true, state.flags.backdrop == backdrop) {
                state.flags.backdrop = backdrop;
            }
        }
    });
    ui.separator();
    if item(ui, "Frame Model", "Frame Model", has_doc, false) {
        actions.push(AppAction::FrameModel);
    }
    if item(ui, "Reset Camera", "", has_doc, false) {
        actions.push(AppAction::ResetCamera);
    }
    ui.menu_button("Camera", |ui| {
        for (label, preset) in [
            ("Front", CameraPreset::Front),
            ("Back", CameraPreset::Back),
            ("Left", CameraPreset::Left),
            ("Right", CameraPreset::Right),
            ("Top", CameraPreset::Top),
            ("Bottom", CameraPreset::Bottom),
        ] {
            if item(ui, label, "", has_doc, false) {
                actions.push(AppAction::SetCameraPreset(preset));
            }
        }
    });
}

fn layer_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let session = state.session.as_ref();
    let set = session.and_then(|s| s.doc.set(s.active_set));
    let active = set.and_then(|s| s.active_layer());
    let index = set.zip(active).and_then(|(s, l)| s.layer_index(l.id));
    let count = set.map_or(0, |s| s.layers.len());
    let has_doc = session.is_some();
    let has_active = active.is_some();
    let can_merge = set.zip(active).is_some_and(|(s, l)| crate::panels::layers::can_merge_down(s, l));
    let can_rasterize = active.is_some_and(|l| !l.is_raster() && !l.locked);
    let can_delete = session.zip(active).is_some_and(|(s, l)| !l.locked && crate::panels::layers::locked_elsewhere(&s.doc, l).is_empty());
    let can_up = index.is_some_and(|i| i + 1 < count);
    let can_down = index.is_some_and(|i| i > 0);
    let active_id = active.map(|l| l.id);

    if item(ui, "New Paint Layer", "New Paint Layer", has_doc, false) {
        actions.push(AppAction::AddPaintLayer);
    }
    if item(ui, "Duplicate", "Duplicate Layer", has_active, false) {
        actions.push(AppAction::DuplicateActiveLayer);
    }
    if item(ui, "Merge Down", "Merge Down", can_merge, false) {
        actions.push(AppAction::MergeActiveDown);
    }
    if item(ui, "Delete", "", can_delete, false)
        && let Some(id) = active_id
    {
        actions.push(AppAction::DeleteLayer(id));
    }
    if item(ui, "Rasterize", "", can_rasterize, false) {
        actions.push(AppAction::RasterizeActiveLayer);
    }
    ui.separator();
    if item(ui, "Move Up", "Move Layer Up", can_up, false) {
        actions.push(AppAction::MoveActiveLayer(1));
    }
    if item(ui, "Move Down", "Move Layer Down", can_down, false) {
        actions.push(AppAction::MoveActiveLayer(-1));
    }
    ui.separator();
    // The Layers list's selection (Ctrl / Shift-click): one layer or several.
    let selection = session.map(|s| s.layer_selection()).unwrap_or_default();
    let label = if selection.len() > 1 { format!("Export Selected Layers... ({})", selection.len()) } else { "Export Layer...".to_owned() };
    if item(ui, &label, "", has_active && !selection.is_empty(), false) {
        actions.push(AppAction::ShowExportLayers(selection));
    }
}
