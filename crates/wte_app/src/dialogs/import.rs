// Owner: app-dialogs (A5). Import dialog of ARCHITECTURE.md 8.8 (R1, R2): model summary, warnings, one row per texture
// set with its UV-layout thumbnail, UV statistics and base-texture choice, the "not paintable" materials, the
// explanation and on/off switch of the generated texture layout of models without UVs, and Create / Cancel.

use std::path::{Path, PathBuf};

use eframe::egui;
use wte_core::{DEFAULT_BLANK_SIDE, ImportedModel, MAX_TEXTURE_SIDE, MeshData, UvStats};

use crate::action::AppAction;
use crate::dialogs::DialogsState;
use crate::state::{ImportDialogState, SetChoiceUi};
use crate::view::ViewCx;

/// Sides offered for a blank base texture (clamped to `MAX_TEXTURE_SIDE`, 8.13 pitfall 4).
pub(super) const BLANK_SIDES: [u32; 5] = [512, 1024, 2048, 4096, 8192];
/// Fill colour of a freshly chosen blank texture.
const BLANK_FILL: [u8; 4] = [255, 255, 255, 255];
/// Displayed size of a UV thumbnail (points; the image itself is 256 px).
const THUMB_POINTS: f32 = 128.0;

/// Identifies the thumbnails an `ImportDialogState` carries: a reload (other `fallback_uvs`, other file) changes it.
#[derive(Clone, Debug, PartialEq)]
struct ThumbKey {
    path: PathBuf,
    uv_hash: Option<u64>,
    fallback_uvs: bool,
    sizes: Vec<[u32; 2]>,
}

impl ThumbKey {
    fn of(st: &ImportDialogState) -> ThumbKey {
        ThumbKey {
            path: st.path.clone(),
            uv_hash: st.model.as_ref().map(|m| m.mesh.uv_hash),
            fallback_uvs: st.fallback_uvs,
            sizes: st.uv_thumbs.iter().map(|t| [t.width, t.height]).collect(),
        }
    }
}

/// The UV thumbnails of the open import dialog, uploaded once (`ctx.load_texture`) and freed with the handles.
pub(super) struct ThumbCache {
    key: ThumbKey,
    /// One entry per set; `None` for a thumbnail with inconsistent pixel data.
    textures: Vec<Option<egui::TextureHandle>>,
}

impl ThumbCache {
    fn upload(ctx: &egui::Context, st: &ImportDialogState) -> ThumbCache {
        let textures = st
            .uv_thumbs
            .iter()
            .enumerate()
            .map(|(i, thumb)| {
                let (w, h) = (thumb.width as usize, thumb.height as usize);
                if w == 0 || h == 0 || thumb.data.len() != w * h * 4 {
                    log::warn!("UV thumbnail {i} has inconsistent pixel data; not shown");
                    return None;
                }
                // Straight alpha (8.13 pitfall 3); the thumbnail is opaque anyway.
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &thumb.data);
                Some(ctx.load_texture(format!("wte_uv_thumb_{i}"), image, egui::TextureOptions::LINEAR))
            })
            .collect();
        ThumbCache { key: ThumbKey::of(st), textures }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The "Detected" choice text of one set: *Detected: name WxH*, or that the texture is missing.
pub(super) fn detected_text(model: &ImportedModel, set: usize) -> String {
    let texture = model.mesh.sets.get(set).and_then(|info| info.texture).and_then(|t| model.textures.get(t as usize));
    match texture {
        Some(tex) => match &tex.image {
            Some(img) => format!("Detected: {} {}x{}", tex.name, img.width, img.height),
            None => format!("Detected: {} - missing, a blank texture will be created", tex.name),
        },
        None => "No texture referenced - a blank texture will be created".to_owned(),
    }
}

/// The UV statistics lines of one set; `true` marks a line that deserves attention.
pub(super) fn uv_stat_lines(stats: &UvStats) -> Vec<(String, bool)> {
    let mut lines = vec![(format!("UV coverage {:.1}%", stats.coverage * 100.0), false)];
    if stats.overlapped_texels > 0 {
        lines.push(("Overlapping UVs".to_owned(), true));
    }
    if stats.mirrored_tris > 0 {
        let s = if stats.mirrored_tris == 1 { "" } else { "s" };
        lines.push((format!("{} mirrored triangle{s}", stats.mirrored_tris), true));
    }
    lines
}

/// Materials without a texture set ("Not paintable (no UVs)"), with their triangle counts.
pub(super) fn unpaintable_materials(mesh: &MeshData) -> Vec<String> {
    mesh.materials
        .iter()
        .enumerate()
        .filter(|(_, m)| m.texture_set.is_none())
        .map(|(i, m)| {
            let tris = mesh.material_ranges.get(i).map_or(0, |r| r.end.saturating_sub(r.start));
            let name = if m.name.is_empty() { "(unnamed)" } else { m.name.as_str() };
            format!("{name} ({tris} triangles)")
        })
        .collect()
}

/// The explanation shown for a model without UVs: what the generated texture layout is, or why the model cannot
/// be painted without it. `None` for a model with UVs.
pub(super) fn generated_layout_text(model: &ImportedModel) -> Option<&'static str> {
    if model.report.fallback_uvs_generated {
        Some(
            "This model has no texture layout (UV mapping) of its own, so one was generated: the surface is cut into \
             flat islands that are packed side by side, without overlap, into a new texture. Painting, stamps and \
             text work everywhere; a texture made for this model in another program will not fit this layout.",
        )
    } else if model.report.no_uvs {
        Some(
            "This model has no texture layout (UV mapping), so a texture cannot be wrapped around it. Generate a \
             layout to paint it.",
        )
    } else {
        None
    }
}

/// The material names of one set, comma separated.
fn set_materials(mesh: &MeshData, set: usize) -> String {
    let Some(info) = mesh.sets.get(set) else { return String::new() };
    let names: Vec<&str> =
        info.materials.iter().filter_map(|&m| mesh.materials.get(m as usize)).map(|m| m.name.as_str()).collect();
    names.join(", ")
}

/// Why *Create* is disabled right now, or `None` when it is enabled.
pub(super) fn create_blocker(st: &ImportDialogState) -> Option<String> {
    if st.loading {
        return Some("Loading the model...".to_owned());
    }
    let Some(model) = &st.model else { return Some("No model is loaded".to_owned()) };
    if model.mesh.sets.is_empty() {
        return Some("The model has no UV mapping".to_owned());
    }
    if st.choices.len() != model.mesh.sets.len() {
        return Some("The texture choices do not match the model; reload it".to_owned());
    }
    st.choices.iter().enumerate().find_map(|(i, choice)| match choice {
        SetChoiceUi::File { image: None, .. } => {
            let name = model.mesh.sets.get(i).map_or_else(|| format!("{}", i + 1), |s| s.name.clone());
            Some(format!("The image chosen for \"{name}\" cannot be used"))
        }
        _ => None,
    })
}

/// The text of a chosen image file: file name and size, or the error.
pub(super) fn file_choice_text(path: &Path, image: Option<&wte_core::PixelBuf>, error: Option<&str>) -> (String, bool) {
    match (image, error) {
        (Some(img), _) => (format!("{} {}x{}", file_name(path), img.width, img.height), false),
        (None, Some(e)) => (format!("{}: {e}", file_name(path)), true),
        (None, None) => (format!("{}: not loaded", file_name(path)), true),
    }
}

/// Draws the import dialog (model summary, one row per texture set, Create / Cancel).
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, ds: &mut DialogsState, st: &mut ImportDialogState) {
    if ds.import_thumbs.as_ref().is_none_or(|c| c.key != ThumbKey::of(st)) {
        ds.import_thumbs = Some(ThumbCache::upload(ctx, st));
    }
    let thumbs = ds.import_thumbs.as_ref().map(|c| c.textures.as_slice()).unwrap_or(&[]);
    let max = super::max_dialog_size(ctx);
    let mut actions: Vec<AppAction> = Vec::new();

    super::modal("import").show(ctx, |ui| {
        ui.set_width(max.x.min(720.0));
        ui.heading("New from Model");
        ui.label(format!("File: {}", file_name(&st.path)));

        if st.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading the model...");
            });
        }
        if let Some(error) = &st.error {
            ui.colored_label(super::error_color(ui), error.as_str());
        }

        let Some(model) = st.model.as_ref() else {
            ui.separator();
            super::button_row(ui, |ui| {
                if ui.button("Cancel").clicked() {
                    actions.push(AppAction::CloseDialog);
                }
            });
            return;
        };
        let mesh = &model.mesh;
        let set_count = mesh.sets.len();
        ui.label(format!(
            "{} triangles, {} material{}, {} texture set{}",
            mesh.tri_count(),
            mesh.materials.len(),
            if mesh.materials.len() == 1 { "" } else { "s" },
            set_count,
            if set_count == 1 { "" } else { "s" },
        ));

        if !model.warnings.is_empty() {
            egui::CollapsingHeader::new(format!("Warnings ({})", model.warnings.len()))
                .id_salt("wte_import_warnings")
                .default_open(model.warnings.len() <= 3)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("wte_import_warning_list").max_height(90.0).show(ui, |ui| {
                        for w in &model.warnings {
                            ui.add(egui::Label::new(egui::RichText::new(w.to_string()).color(super::warn_color(ui))).wrap());
                        }
                    });
                });
        }

        // A model without any UVs: the generated texture layout is applied on import; explain it and let the user
        // switch it off again (=> ImportReload).
        if model.report.no_uvs || st.fallback_uvs {
            ui.separator();
            if let Some(text) = generated_layout_text(model) {
                ui.add(egui::Label::new(text).wrap());
            }
            let mut fallback = st.fallback_uvs;
            let enabled = !st.loading;
            let checkbox = egui::Checkbox::new(&mut fallback, "Generate a texture layout (automatic UVs)");
            if ui.add_enabled(enabled, checkbox).changed() {
                actions.push(AppAction::ImportReload { fallback_uvs: fallback });
            }
        }

        ui.separator();
        let list_height = (max.y - 300.0).max(160.0);
        egui::ScrollArea::vertical().id_salt("wte_import_sets").max_height(list_height).auto_shrink([false, true]).show(
            ui,
            |ui| {
                for set in 0..set_count {
                    let info = &mesh.sets[set];
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            // UV-layout thumbnail: visible proof that the wrapping was recognised.
                            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(THUMB_POINTS), egui::Sense::hover());
                            match thumbs.get(set).and_then(Option::as_ref) {
                                Some(tex) => {
                                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                    ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
                                }
                                None => {
                                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(0x20, 0x24, 0x28));
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "no preview",
                                        egui::FontId::proportional(12.0),
                                        egui::Color32::GRAY,
                                    );
                                }
                            }
                            ui.vertical(|ui| {
                                ui.strong(info.name.as_str());
                                let materials = set_materials(mesh, set);
                                if !materials.is_empty() {
                                    ui.add(egui::Label::new(format!("Materials: {materials}")).wrap());
                                }
                                ui.label(format!("{} triangles", info.tri_count));
                                if let Some(stats) = st.uv_stats.get(set) {
                                    ui.horizontal_wrapped(|ui| {
                                        for (line, attention) in uv_stat_lines(stats) {
                                            if attention {
                                                ui.colored_label(super::warn_color(ui), line);
                                            } else {
                                                ui.label(line);
                                            }
                                        }
                                    });
                                }
                                ui.add_space(4.0);
                                if let Some(choice) = st.choices.get_mut(set) {
                                    texture_choice_ui(ui, model, set, choice, st.loading, &mut actions);
                                }
                            });
                        });
                    });
                }

                let unpaintable = unpaintable_materials(mesh);
                if !unpaintable.is_empty() {
                    ui.add_space(4.0);
                    ui.strong("Not paintable (no UVs)");
                    ui.weak("These materials are shown in their base colour and cannot be painted.");
                    for line in unpaintable {
                        ui.label(format!("  {line}"));
                    }
                }
            },
        );

        ui.separator();
        let blocker = create_blocker(st);
        super::button_row(ui, |ui| {
            if ui.button("Cancel").clicked() {
                actions.push(AppAction::CloseDialog);
            }
            if ui.add_enabled(blocker.is_none(), egui::Button::new("Create")).clicked() {
                actions.push(AppAction::ImportCreate);
            }
            if let Some(reason) = &blocker {
                ui.colored_label(super::warn_color(ui), reason.as_str());
            }
        });
    });

    for action in actions {
        cx.state.push(action);
    }
}

/// The base-texture choice of one set: Detected / image file / blank.
fn texture_choice_ui(
    ui: &mut egui::Ui,
    model: &ImportedModel,
    set: usize,
    choice: &mut SetChoiceUi,
    loading: bool,
    actions: &mut Vec<AppAction>,
) {
    ui.add_enabled_ui(!loading, |ui| {
        if ui.radio(matches!(choice, SetChoiceUi::Detected), detected_text(model, set)).clicked() {
            *choice = SetChoiceUi::Detected;
        }

        ui.horizontal(|ui| {
            let is_file = matches!(choice, SetChoiceUi::File { .. });
            if ui.radio(is_file, "Image file").clicked() && !is_file {
                actions.push(AppAction::ImportPickTexture { set, path: None });
            }
            if ui.button("Choose image...").clicked() {
                actions.push(AppAction::ImportPickTexture { set, path: None });
            }
            if let SetChoiceUi::File { path, image, error, .. } = choice {
                let (text, is_error) = file_choice_text(path, image.as_ref(), error.as_deref());
                if is_error {
                    ui.colored_label(super::error_color(ui), text);
                } else {
                    ui.label(text);
                }
            }
        });

        ui.horizontal(|ui| {
            let is_blank = matches!(choice, SetChoiceUi::Blank { .. });
            if ui.radio(is_blank, "Blank").clicked() && !is_blank {
                *choice = SetChoiceUi::Blank { side: DEFAULT_BLANK_SIDE, fill: BLANK_FILL };
            }
            if let SetChoiceUi::Blank { side, fill } = choice {
                *side = (*side).clamp(1, MAX_TEXTURE_SIDE);
                egui::ComboBox::from_id_salt(("wte_import_blank_side", set))
                    .selected_text(format!("{side} x {side}"))
                    .show_ui(ui, |ui| {
                        for s in BLANK_SIDES {
                            ui.selectable_value(side, s.min(MAX_TEXTURE_SIDE), format!("{s} x {s}"));
                        }
                    });
                ui.label("Fill");
                ui.color_edit_button_srgba_unmultiplied(fill);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wte_core::{PixelBuf, SampleKind, TexelMap, TextureSetId};

    fn sample_model() -> ImportedModel {
        // The sphere sample's model, as an import would deliver it (texture detected and decoded).
        let doc = wte_core::Document::sample(SampleKind::Sphere);
        wte_core::import::import_from_bundle(&doc.source, &doc.import_options, true, &wte_core::NoProgress)
            .expect("re-import of the sample")
    }

    fn dialog_state(model: ImportedModel) -> ImportDialogState {
        let sets = model.mesh.sets.len();
        let uv_thumbs = (0..sets).map(|s| wte_core::uv_layout_image(&model.mesh, TextureSetId(s as u32), 64)).collect();
        let uv_stats =
            (0..sets).map(|s| TexelMap::build(&model.mesh, TextureSetId(s as u32), 64, 64, 0).stats).collect();
        ImportDialogState {
            path: PathBuf::from(r"E:\models\sphere.obj"),
            model: Some(model),
            choices: vec![SetChoiceUi::Detected; sets],
            uv_thumbs,
            uv_stats,
            fallback_uvs: false,
            loading: false,
            error: None,
        }
    }

    #[test]
    fn detected_text_names_the_texture_and_its_size() {
        let model = sample_model();
        let text = detected_text(&model, 0);
        assert!(text.starts_with("Detected: "), "{text}");
        let img = model.textures[model.mesh.sets[0].texture.unwrap() as usize].image.as_ref().unwrap();
        assert!(text.ends_with(&format!("{}x{}", img.width, img.height)), "{text}");

        let mut missing = model.clone();
        for t in &mut missing.textures {
            t.image = None;
        }
        assert!(detected_text(&missing, 0).contains("missing"));
        assert!(detected_text(&missing, 99).contains("blank texture"));
    }

    #[test]
    fn create_is_blocked_with_a_reason() {
        let mut st = dialog_state(sample_model());
        assert_eq!(create_blocker(&st), None);

        st.choices[0] = SetChoiceUi::File {
            path: PathBuf::from(r"E:\t\broken.png"),
            bytes: None,
            image: None,
            error: Some("Not an image".to_owned()),
        };
        assert!(create_blocker(&st).unwrap().contains("cannot be used"));

        st.choices[0] = SetChoiceUi::File {
            path: PathBuf::from(r"E:\t\ok.png"),
            bytes: Some(Arc::from(&b"x"[..])),
            image: Some(PixelBuf::new(4, 4)),
            error: None,
        };
        assert_eq!(create_blocker(&st), None);

        st.choices[0] = SetChoiceUi::Blank { side: 2048, fill: [1, 2, 3, 255] };
        assert_eq!(create_blocker(&st), None);

        st.loading = true;
        assert!(create_blocker(&st).is_some());
        st.loading = false;

        st.choices.clear();
        assert!(create_blocker(&st).is_some());

        let mut no_sets = dialog_state(sample_model());
        if let Some(m) = no_sets.model.as_mut() {
            m.mesh.sets.clear();
        }
        no_sets.choices.clear();
        assert_eq!(create_blocker(&no_sets).as_deref(), Some("The model has no UV mapping"));

        no_sets.model = None;
        assert!(create_blocker(&no_sets).is_some());
    }

    #[test]
    fn uv_stats_lines_flag_overlaps_and_mirrors() {
        let clean = UvStats { covered_texels: 10, overlapped_texels: 0, mirrored_tris: 0, coverage: 0.4567, texel_world: 1.0 };
        assert_eq!(uv_stat_lines(&clean), vec![("UV coverage 45.7%".to_owned(), false)]);
        let messy = UvStats { overlapped_texels: 5, mirrored_tris: 3, ..clean };
        let lines = uv_stat_lines(&messy);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], ("Overlapping UVs".to_owned(), true));
        assert_eq!(lines[2], ("3 mirrored triangles".to_owned(), true));
        let one = UvStats { mirrored_tris: 1, ..clean };
        assert_eq!(uv_stat_lines(&one)[1].0, "1 mirrored triangle");
    }

    #[test]
    fn materials_without_a_set_are_listed_as_not_paintable() {
        let mut model = sample_model();
        assert!(unpaintable_materials(&model.mesh).is_empty());
        assert!(!set_materials(&model.mesh, 0).is_empty());
        model.mesh.materials[0].texture_set = None;
        let list = unpaintable_materials(&model.mesh);
        assert_eq!(list.len(), 1);
        assert!(list[0].ends_with(&format!("({} triangles)", model.mesh.tri_count())), "{}", list[0]);
    }

    #[test]
    fn file_choice_text_shows_size_or_error() {
        let path = PathBuf::from(r"E:\t\wood.png");
        assert_eq!(file_choice_text(&path, Some(&PixelBuf::new(16, 8)), None), ("wood.png 16x8".to_owned(), false));
        assert_eq!(file_choice_text(&path, None, Some("bad header")), ("wood.png: bad header".to_owned(), true));
        assert!(file_choice_text(&path, None, None).1);
    }

    #[test]
    fn thumb_key_changes_with_a_reload() {
        let st = dialog_state(sample_model());
        let key = ThumbKey::of(&st);
        assert_eq!(key, ThumbKey::of(&st.clone()));
        let mut reloaded = st.clone();
        reloaded.fallback_uvs = true;
        assert_ne!(key, ThumbKey::of(&reloaded));
        let mut other = st;
        other.path = PathBuf::from(r"E:\models\other.obj");
        assert_ne!(key, ThumbKey::of(&other));
    }

    #[test]
    fn a_model_without_uvs_explains_the_generated_layout() {
        let (obj, _) = wte_core::samples::to_obj(&wte_core::samples::no_uv_tetra(), "", &[]);
        let bundle = wte_core::SourceBundle {
            format: wte_core::ModelFormat::Obj,
            main: wte_core::SourceFile { key: "tetra.obj".into(), original_name: "tetra.obj".into(), bytes: Arc::from(obj.as_bytes()) },
            sidecars: Vec::new(),
            extras: Vec::new(),
            original_path: None,
        };
        let plain = wte_core::import::import_from_bundle(&bundle, &wte_core::ImportOptions::default(), true, &wte_core::NoProgress).unwrap();
        assert!(generated_layout_text(&plain).unwrap().contains("cannot be wrapped"));
        let opts = wte_core::ImportOptions { generate_fallback_uvs: true, ..Default::default() };
        let generated = wte_core::import::import_from_bundle(&bundle, &opts, true, &wte_core::NoProgress).unwrap();
        let text = generated_layout_text(&generated).unwrap();
        assert!(text.contains("generated") && text.contains("without overlap"), "{text}");
        let mut st = dialog_state(generated);
        st.fallback_uvs = true;
        assert_eq!(create_blocker(&st), None, "the generated layout can be created");
        assert_eq!(generated_layout_text(&sample_model()), None);
    }

    #[test]
    fn blank_sides_respect_the_texture_limit() {
        assert!(BLANK_SIDES.iter().all(|&s| s <= MAX_TEXTURE_SIDE));
        assert!(BLANK_SIDES.contains(&DEFAULT_BLANK_SIDE));
    }
}
