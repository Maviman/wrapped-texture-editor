// Owner: app-shell (A1). File drag-and-drop of ARCHITECTURE.md 8.9: a full-window "Drop to open" overlay while files
// hover over the window; dropped paths become `AppAction::OpenPath` (dispatch routes them by extension).

use eframe::egui;

use crate::action::{AppAction, ToastLevel};
use crate::state::AppState;

/// Draws the "Drop to open" overlay while files hover over the window and turns dropped paths into
/// `AppAction::OpenPath`.
pub fn handle(ctx: &egui::Context, state: &mut AppState) {
    let (hovering, dropped) = ctx.input(|i| {
        let dropped: Vec<_> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect();
        (!i.raw.hovered_files.is_empty(), dropped)
    });
    // While a modal job runs nothing may replace the session (8.9): drops are ignored.
    let blocked = state.busy.as_ref().is_some_and(|b| b.modal);

    if hovering {
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("wte_drop_overlay")));
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(170));
        painter.rect_stroke(
            rect.shrink(12.0),
            8.0,
            egui::Stroke::new(2.0, crate::theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        let text = if blocked { "Please wait for the running operation to finish" } else { "Drop to open" };
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(28.0), egui::Color32::WHITE);
        if !blocked {
            let hint = "Models (.obj, .gltf, .glb, .fbx), projects (.wtep), images (stamp) or PSD files (sticker)";
            let pos = rect.center() + egui::vec2(0.0, 32.0);
            painter.text(pos, egui::Align2::CENTER_CENTER, hint, egui::FontId::proportional(15.0), egui::Color32::LIGHT_GRAY);
        }
    }

    if dropped.is_empty() {
        return;
    }
    if blocked {
        state.toasts.warn("Please wait for the running operation to finish");
        return;
    }
    let plan = plan_drop(&dropped);
    for path in &dropped {
        log::info!("dropped {}", path.display());
    }
    if !plan.ignored.is_empty() {
        let names: Vec<String> = plan.ignored.iter().map(|p| file_name(p)).collect();
        state.push(AppAction::Toast(ToastLevel::Info, format!("Not opened: {} ({})", names.join(", "), plan.reason)));
    }
    if let Some(doc) = plan.document {
        state.push(AppAction::OpenPath(doc));
    }
    for image in plan.stamps {
        state.push(AppAction::OpenPath(image));
    }
}

/// What a drop of several files does (8.9): one project or model is opened (`.wtep` first, else the first
/// `.obj` / `.gltf` / `.glb` / `.fbx`); images become stamp sources only when no project or model is part of the drop (a
/// model's texture dropped together with it is found through the model itself: its MTL or glTF); every other file
/// (`.mtl`, `.bin`, folders, unknown types) is ignored with one info toast instead of taking the document's place.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct DropPlan {
    pub document: Option<std::path::PathBuf>,
    pub stamps: Vec<std::path::PathBuf>,
    pub ignored: Vec<std::path::PathBuf>,
    /// Why the `ignored` files are not opened (for the toast).
    pub reason: &'static str,
}

pub(crate) fn plan_drop(paths: &[std::path::PathBuf]) -> DropPlan {
    let is_project = |p: &std::path::PathBuf| extension_of(p) == wte_core::PROJECT_EXTENSION;
    let is_model = |p: &std::path::PathBuf| MODEL_EXTENSIONS.contains(&extension_of(p).as_str());
    let document = paths.iter().find(|p| is_project(p)).or_else(|| paths.iter().find(|p| is_model(p))).cloned();
    let mut plan = DropPlan { document, ..DropPlan::default() };
    for path in paths {
        if plan.document.as_ref() == Some(path) {
            continue;
        }
        if (is_image_path(path) || is_psd_path(path)) && plan.document.is_none() {
            plan.stamps.push(path.clone());
        } else {
            plan.ignored.push(path.clone());
        }
    }
    plan.reason = match &plan.document {
        Some(_) => "one project or model is opened per drop; a model finds its textures and material files itself",
        None => "not a model, project, image or PSD file",
    };
    plan
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Image extensions accepted as stamp sources (same list as the decoder: png, jpeg, tga, bmp, tiff, webp, dds).
pub const IMAGE_EXTENSIONS: [&str; 9] = ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "webp", "dds"];
/// Model extensions of the import flow (file dialog filter, drag and drop, `OpenPath` / command line).
pub const MODEL_EXTENSIONS: [&str; 4] = ["obj", "gltf", "glb", "fbx"];

/// Lower-case extension of a path ("" without one).
pub fn extension_of(path: &std::path::Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Stickers P9a (STICKER_PLAN 5.8): a PSD / PSB file, imported as a sticker.
pub fn is_psd_path(path: &std::path::Path) -> bool {
    matches!(extension_of(path).as_str(), "psd" | "psb")
}

/// True for the image extensions of `IMAGE_EXTENSIONS`.
pub fn is_image_path(path: &std::path::Path) -> bool {
    IMAGE_EXTENSIONS.contains(&extension_of(path).as_str())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::state::Prefs;

    #[derive(Debug)]
    struct FakeDrop(PathBuf);
    impl egui::DroppedFile for FakeDrop {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("not read".into())
        }
    }

    #[test]
    fn dropped_files_become_open_path_actions() {
        let ctx = egui::Context::default();
        let mut state = AppState::new(Prefs::default(), true);
        let files: Vec<egui::DroppedFileHandle> = vec![
            Arc::new(FakeDrop(PathBuf::from(r"E:\m\a.OBJ"))),
            Arc::new(FakeDrop(PathBuf::from(r"E:\m\b.wtep"))),
            Arc::new(FakeDrop(PathBuf::from(r"E:\m\logo.png"))),
        ];
        let input = egui::RawInput { dropped_files: files, ..Default::default() };
        crate::theme::run_test_pass(&ctx, input, |ui| handle(ui.ctx(), &mut state));
        let mut toasts = Vec::new();
        let paths: Vec<_> = state
            .actions
            .iter()
            .filter_map(|a| match a {
                AppAction::OpenPath(p) => Some(p.clone()),
                AppAction::Toast(ToastLevel::Info, text) => {
                    toasts.push(text.clone());
                    None
                }
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        // The project wins over the model; the image is not a stamp when a document comes with the drop.
        assert_eq!(paths, [PathBuf::from(r"E:\m\b.wtep")]);
        assert_eq!(toasts.len(), 1, "one toast names the files not opened");
        assert!(toasts[0].contains("a.OBJ") && toasts[0].contains("logo.png"), "{toasts:?}");
    }

    #[test]
    fn a_model_dropped_with_its_material_and_texture_opens_the_model() {
        let p = |s: &str| PathBuf::from(s);
        // Explorer lists the grabbed file first: here the .mtl.
        let plan = plan_drop(&[p(r"E:\m\model.mtl"), p(r"E:\m\model.obj"), p(r"E:\m\texture.png")]);
        assert_eq!(plan.document, Some(p(r"E:\m\model.obj")));
        assert!(plan.stamps.is_empty(), "the model's texture does not become a stamp");
        assert_eq!(plan.ignored, [p(r"E:\m\model.mtl"), p(r"E:\m\texture.png")]);
        // Only images: stamp sources, in order; a folder or unknown file is ignored.
        let plan = plan_drop(&[p(r"E:\img\folder"), p(r"E:\img\a.png"), p(r"E:\img\b.JPG"), p(r"E:\img\notes.txt")]);
        assert_eq!(plan.document, None);
        assert_eq!(plan.stamps, [p(r"E:\img\a.png"), p(r"E:\img\b.JPG")]);
        assert_eq!(plan.ignored, [p(r"E:\img\folder"), p(r"E:\img\notes.txt")]);
        // Two models: the first one; a glTF's .bin is ignored.
        let plan = plan_drop(&[p("x.bin"), p("x.gltf"), p("y.glb")]);
        assert_eq!(plan.document, Some(p("x.gltf")));
        assert_eq!(plan.ignored, [p("x.bin"), p("y.glb")]);
        assert_eq!(plan_drop(&[]), DropPlan { reason: "not a model, project, image or PSD file", ..DropPlan::default() });
        // Stickers P9a (5.8): a PSD file without a model or project is opened as a sticker, in order with the images.
        let plan = plan_drop(&[p(r"E:\img.png"), p(r"E:\img\Logo.PSD"), p(r"E:\imgig.psb")]);
        assert_eq!(plan.stamps, [p(r"E:\img.png"), p(r"E:\img\Logo.PSD"), p(r"E:\imgig.psb")]);
        let plan = plan_drop(&[p(r"E:\m\model.obj"), p(r"E:\img\Logo.psd")]);
        assert_eq!(plan.ignored, [p(r"E:\img\Logo.psd")]);
        assert!(is_psd_path(Path::new("x.PSD")) && is_psd_path(Path::new("x.psb")) && !is_psd_path(Path::new("x.png")));
    }

    #[test]
    fn extensions() {
        assert!(is_image_path(Path::new("x.JPG")));
        assert!(!is_image_path(Path::new("x.obj")));
        assert_eq!(extension_of(Path::new("dir.v2/model")), "");
        assert_eq!(extension_of(Path::new("a.GLB")), "glb");
        // FBX models open like the other formats (drop, file dialog filter, OpenPath).
        assert!(MODEL_EXTENSIONS.contains(&extension_of(Path::new(r"E:\m\Spider.FBX")).as_str()));
        let plan = plan_drop(&[PathBuf::from(r"E:\m\tex.png"), PathBuf::from(r"E:\m\Spider.FBX")]);
        assert_eq!(plan.document, Some(PathBuf::from(r"E:\m\Spider.FBX")));
    }
}
