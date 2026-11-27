// Owner: app-panels (A4). The menu bar of ARCHITECTURE.md 8.7: File, Edit, View, Layer, Object (objects-tabs), Help. Every entry pushes an
// `AppAction` (or changes a view setting); shortcut texts come from `shortcuts::shortcut_text`.

use std::path::PathBuf;

use eframe::egui;
use egui::Button;
use wte_core::SampleKind;

use crate::action::AppAction;
use crate::shortcuts::shortcut_text;
use crate::state::{AppState, Backdrop, CameraPreset, Dialog, ShadingMode, UpperTab, ViewMode};
use crate::sticker::{StickerAction, StyleTarget};

/// A menu entry: label, the `shortcut_text` label it shows ("" = none), enabled, selected, action.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, selected: bool) -> bool {
    let key = if shortcut.is_empty() { String::new() } else { shortcut_text(ui.ctx(), shortcut) };
    let button = Button::new(label).shortcut_text(key).selected(selected);
    ui.add_enabled(enabled, button).clicked()
}

/// Stickers P10b: a menu entry whose keys are not in a shortcut table (the clipboard chords egui-winit turns into
/// events, STICKER_PLAN 6.3): the key text is given as it is.
fn item_keys(ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, Button::new(label).shortcut_text(keys)).clicked()
}

/// Draws the menu bar: File, Edit, View, Layer, Object, (Sticker in the Sticker view), Help.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let mut actions: Vec<AppAction> = Vec::new();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, state, &mut actions));
        ui.menu_button("Edit", |ui| edit_menu(ui, state, &mut actions));
        ui.menu_button("View", |ui| view_menu(ui, state, &mut actions));
        ui.menu_button("Layer", |ui| layer_menu(ui, state, &mut actions));
        ui.menu_button("Object", |ui| object_menu(ui, state, &mut actions));
        // Stickers P7b (STICKER_PLAN 3.3 "Commands"): the Sticker menu, only in the Sticker view (outside it the menu bar
        // is as before).
        if state.view_mode == ViewMode::Sticker {
            ui.menu_button("Sticker", |ui| sticker_menu(ui, state, &mut actions));
        }
        ui.menu_button("Help", |ui| {
            if item(ui, "Shortcuts", "", true, false) {
                actions.push(AppAction::ShowDialog(Dialog::Shortcuts));
            }
            // Branch `pen-input`: drawing tablets.
            if item(ui, "Tablet setup", "", true, false) {
                actions.push(AppAction::ShowDialog(Dialog::TabletHelp));
            }
            // Branch `companion`: the Android tablet companion.
            if item(ui, "Tablet companion...", "", true, false) {
                actions.push(AppAction::ShowDialog(Dialog::Companion));
            }
            // Stickers P9a / P9b (STICKER_PLAN 5.7, 13.1): where the outside editors' programs are, and the default one.
            if item(ui, "Editor locations...", "", true, false) {
                actions.push(AppAction::Link(crate::link::LinkAction::ShowEditorLocations));
            }
            // Stickers P10b (STICKER_PLAN 13.5): what the clipboard offers (hand test 24).
            if item(ui, "Clipboard contents...", "", true, false) {
                actions.push(AppAction::Clip(crate::clip::ClipAction::ShowContents));
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
        for (label, kind) in [
            ("Sphere", SampleKind::Sphere),
            ("Cube", SampleKind::Cube),
            ("Two Materials", SampleKind::TwoMaterial),
            ("Three Objects", SampleKind::Objects),
        ] {
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
    // Stickers P9a (STICKER_PLAN 5.8): a PSD file as a layered tray sticker, optionally kept linked to the file.
    if item(ui, "Import Sticker from PSD...", "", has_doc, false) {
        actions.push(AppAction::Link(crate::link::LinkAction::ImportPsd { path: None, keep_linked: false }));
    }
    if item(ui, "Import Sticker from PSD, Keep Linked...", "", has_doc, false) {
        actions.push(AppAction::Link(crate::link::LinkAction::ImportPsd { path: None, keep_linked: true }));
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
    // Stickers P10b (STICKER_PLAN 6.3): the clipboard. Menu items too, so the tablet and a run without the key
    // subclass can do all of it without a key.
    ui.separator();
    let open = state.session.is_some();
    let in_editor = state.view_mode == ViewMode::Sticker && crate::sticker::edit_of(state).is_some();
    use crate::clip::ClipAction;
    for (label, keys, enabled, action) in [
        ("Cut", "Ctrl+X", in_editor, ClipAction::Cut),
        ("Copy", "Ctrl+C", open, ClipAction::Copy),
        ("Paste", "Ctrl+V", open, ClipAction::Paste),
        ("Paste at View Centre", "", open, ClipAction::PasteAtViewCentre),
    ] {
        if item_keys(ui, label, keys, enabled) {
            actions.push(AppAction::Clip(action));
        }
    }
    // Stickers P6b (STICKER_PLAN 3.7): the styles of the selected stamps, or in the Sticker view of the sticker layers.
    ui.separator();
    let in_view = state.view_mode == ViewMode::Sticker;
    let target = if in_view { StyleTarget::Layers } else { StyleTarget::Stamps };
    let has_target = if in_view { crate::sticker::edit_of(state).is_some() } else { !crate::sticker::selected_stamps(state).is_empty() };
    let can_paste = has_target && state.style_clipboard.is_some();
    for (label, enabled, action) in [
        ("Copy Style", has_target, StickerAction::CopyStyle(target)),
        ("Paste Style", can_paste, StickerAction::PasteStyle(target)),
        ("Clear Style", has_target, StickerAction::ClearStyle(target)),
    ] {
        if item(ui, label, "", enabled, false) {
            actions.push(AppAction::Sticker(action));
        }
    }
}

fn view_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let has_doc = state.session.is_some();
    // Stickers P6a: the Sticker view needs a selected stamp (or is open).
    let can_sticker = state.view_mode == ViewMode::Sticker
        || state.session.as_ref().is_some_and(|s| s.selected_stamp.is_some_and(|id| s.doc.stamp(id).is_some()));
    for (label, mode) in [
        ("3D", ViewMode::View3D),
        ("2D", ViewMode::View2D),
        ("Split", ViewMode::Split),
        ("Preview", ViewMode::Preview),
        ("Sticker", ViewMode::Sticker),
    ] {
        let enabled = match mode {
            ViewMode::Preview => has_doc,
            ViewMode::Sticker => can_sticker,
            _ => true,
        };
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

/// Stickers P7b: the Sticker menu: the Selection, Cut-out and Filter commands of `sticker::filters::COMMANDS` as three
/// submenus (shortcut texts from the Sticker view's table).
fn sticker_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    use crate::sticker::filters::{COMMANDS, Group, available, available_more, enabled_more};
    let have = available(state);
    let more = available_more(state);
    for group in Group::MENU {
        ui.menu_button(group.label(), |ui| {
            for def in COMMANDS.iter().filter(|d| d.group == group) {
                let key = if def.shortcut.is_empty() { String::new() } else { crate::shortcuts::sticker_shortcut_text(ui.ctx(), def.shortcut) };
                let button = Button::new(def.label).shortcut_text(key);
                if ui.add_enabled(enabled_more(def, have, more), button).on_hover_text(def.hover).clicked() {
                    actions.push(AppAction::Sticker((def.make)(state)));
                }
            }
        });
    }
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
    // fix_export item 5: with several layers selected, Duplicate and Delete act on all of them (one undo step each);
    // Merge Down stays on the active layer.
    let selection = session.map(|s| s.layer_selection()).unwrap_or_default();
    let n = selection.len();
    let multi = n > 1;
    let can_delete = if multi {
        session.is_some_and(|s| {
            selection.iter().filter_map(|id| s.doc.layer(*id)).any(|l| !l.locked && crate::panels::layers::locked_elsewhere(&s.doc, l).is_empty())
        })
    } else {
        can_delete
    };
    let can_up = index.is_some_and(|i| i + 1 < count);
    let can_down = index.is_some_and(|i| i > 0);
    let active_id = active.map(|l| l.id);

    // Stickers P6a (STICKER_PLAN 3.1): the selected stamp opens in the Sticker view.
    let stamp_selected = session.is_some_and(|s| s.selected_stamp.is_some_and(|id| s.doc.stamp(id).is_some()));
    if item(ui, "Edit Sticker", "Edit Sticker", stamp_selected && state.view_mode != ViewMode::Sticker, false) {
        actions.push(AppAction::Sticker(crate::sticker::StickerAction::Enter(None)));
    }
    // Stickers P9a / P9b (STICKER_PLAN 5.1, 13.1): the selected stamp's sticker in the outside editor (its link's
    // editor, else the default one; "Edit in..." asks), and the submenu "Edit in" with the editor list.
    let selected = session.and_then(|s| s.selected_stamp.filter(|id| s.doc.stamp(*id).is_some()));
    let edit_in_label = crate::link::edit_in_label(state, selected.and_then(|id| crate::link::sticker_of_stamp(state, id)));
    if item(ui, &edit_in_label, "", selected.is_some(), false)
        && let Some(id) = selected
    {
        actions.push(AppAction::Link(crate::link::LinkAction::EditIn(crate::link::EditRequest::new(crate::link::LinkTarget::Stamp(id)))));
    }
    ui.add_enabled_ui(selected.is_some(), |ui| {
        ui.menu_button("Edit in", |ui| {
            if let Some(id) = selected {
                let mut hits = Vec::new();
                crate::link::ui::editor_list(ui, state, crate::link::LinkTarget::Stamp(id), actions, &mut hits);
            }
        });
    });
    ui.separator();
    if item(ui, "New Paint Layer", "New Paint Layer", has_doc, false) {
        actions.push(AppAction::AddPaintLayer);
    }
    let duplicate_label = if multi { format!("Duplicate {n} Layers") } else { "Duplicate".to_owned() };
    if item(ui, &duplicate_label, "Duplicate Layer", has_active, false) {
        actions.push(AppAction::DuplicateActiveLayer);
    }
    let merge_label = if multi { "Merge Down (active layer)" } else { "Merge Down" };
    if item(ui, merge_label, "Merge Down", can_merge, false) {
        actions.push(AppAction::MergeActiveDown);
    }
    let delete_label = if multi { format!("Delete {n} Layers") } else { "Delete".to_owned() };
    if item(ui, &delete_label, "", can_delete, false) {
        if multi {
            actions.push(AppAction::DeleteLayers(selection.clone()));
        } else if let Some(id) = active_id {
            actions.push(AppAction::DeleteLayer(id));
        }
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
    let label = if selection.len() > 1 { format!("Export Selected Layers... ({})", selection.len()) } else { "Export Layer...".to_owned() };
    if item(ui, &label, "", has_active && !selection.is_empty(), false) {
        actions.push(AppAction::ShowExportLayers(selection));
    }
}

/// objects-tabs (ARCHITECTURE.md 8.7): hide / isolate / show the objects of the model, frame and select them.
fn object_menu(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>) {
    let session = state.session.as_ref();
    let has_doc = session.is_some();
    let has_selection = session.is_some_and(|s| !s.selected_objects.is_empty());
    let any_hidden = session.is_some_and(|s| s.doc.hidden_object_count() > 0);
    if item(ui, "Hide Selected", "Hide Selected", has_selection, false) {
        actions.push(AppAction::HideSelectedObjects);
    }
    if item(ui, "Isolate Selected", "Isolate Selected", has_selection, false) {
        actions.push(AppAction::IsolateSelectedObjects);
    }
    if item(ui, "Show All", "Show All", any_hidden, false) {
        actions.push(AppAction::ShowAllObjects);
    }
    ui.separator();
    if item(ui, "Frame Selection", "Frame Selection", has_doc, false) {
        actions.push(AppAction::FrameSelection);
    }
    ui.separator();
    if item(ui, "Select All", "", has_doc, false) {
        actions.push(AppAction::SelectAllObjects);
    }
    if item(ui, "Deselect All", "", has_selection, false) {
        actions.push(AppAction::DeselectObjects);
    }
    ui.separator();
    // objects-tabs (WP4): shows the side panel's Objects tab (a setting, like the Background entries).
    if item(ui, "Objects Panel", "", true, state.upper_tab == UpperTab::Objects) {
        state.upper_tab = UpperTab::Objects;
    }
}
