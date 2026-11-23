// Owner: app-dialogs (A5). Export dialog of ARCHITECTURE.md 8.8 (R4, R5): output options, the exact list of files that
// will be written (`export::plan`, cached), the embedded live preview of the flattened result, and the
// "N files will be replaced" confirm step.

use std::path::{Path, PathBuf};

use eframe::egui;
use wte_core::{DocumentSnapshot, ExportOptions, ExportPackage, FlatFormat, LayerId, PlannedKind, PngLevel, TextureSetId};

use crate::action::AppAction;
use crate::dialogs::DialogsState;
use crate::state::{ExportDialogState, ViewId};
use crate::view::ViewCx;

/// The flattened formats in the order of `ExportDialogState::format_index`.
pub(super) const FORMAT_NAMES: [&str; 6] = ["PNG", "JPEG", "TGA", "BMP", "TIFF", "WebP (lossless)"];
/// Upper limit of the seam padding field (texels).
pub(super) const MAX_SEAM_PADDING: u32 = 64;
/// Width of the options column (points).
const LEFT_WIDTH: f32 = 400.0;
/// The preview is left out below this side length (points).
const MIN_PREVIEW_SIDE: f32 = 160.0;
/// How often the "will be replaced" markers re-check the disk while the dialog is idle (seconds).
const EXISTS_RECHECK_S: f64 = 1.0;

/// `FlatFormat` of a format index (`FORMAT_NAMES`); PNG keeps `png_level`, JPEG takes quality (clamped to 1..=100)
/// and matte.
pub(super) fn flat_format(index: usize, quality: u8, matte: [u8; 3], png_level: PngLevel) -> FlatFormat {
    match index {
        1 => FlatFormat::Jpeg { quality: quality.clamp(1, 100), matte },
        2 => FlatFormat::Tga,
        3 => FlatFormat::Bmp,
        4 => FlatFormat::Tiff,
        5 => FlatFormat::WebpLossless,
        _ => FlatFormat::Png(png_level),
    }
}

/// Index into `FORMAT_NAMES` of a format.
pub(super) fn format_index(format: &FlatFormat) -> usize {
    match format {
        FlatFormat::Png(_) => 0,
        FlatFormat::Jpeg { .. } => 1,
        FlatFormat::Tga => 2,
        FlatFormat::Bmp => 3,
        FlatFormat::Tiff => 4,
        FlatFormat::WebpLossless => 5,
    }
}

/// Makes the dialog fields agree with `options.flat` (the source of truth when it is set, e.g. after the dialog was
/// created by code): format index, and JPEG quality / matte.
pub(super) fn sync_from_options(st: &mut ExportDialogState) {
    if let Some(format) = st.options.flat {
        st.format_index = format_index(&format);
        if let FlatFormat::Jpeg { quality, matte } = format {
            st.jpeg_quality = quality.clamp(1, 100);
            st.jpeg_matte = matte;
        }
    }
    st.format_index = st.format_index.min(FORMAT_NAMES.len() - 1);
}

/// Writes the dialog fields back into `options.flat` (`None` when the flattened texture is switched off).
pub(super) fn apply_flat(st: &mut ExportDialogState, flat_on: bool) {
    let png_level = match st.options.flat {
        Some(FlatFormat::Png(level)) => level,
        _ => PngLevel::Balanced,
    };
    st.options.flat = flat_on.then(|| flat_format(st.format_index, st.jpeg_quality, st.jpeg_matte, png_level));
}

/// The matte the embedded preview shows: the JPEG matte for a format without alpha, else `None` (alpha checker).
pub(super) fn preview_matte(st: &ExportDialogState) -> Option<[u8; 3]> {
    match st.options.flat {
        Some(format) if !format.keeps_alpha() => Some(st.jpeg_matte),
        _ => None,
    }
}

/// True when the output folder is empty (or only white space).
fn dir_is_empty(dir: &Path) -> bool {
    dir.to_string_lossy().trim().is_empty()
}

/// Why *Export* is disabled for these options and this plan, or `None` when it is enabled.
pub(super) fn export_blocker(opts: &ExportOptions, plan_len: usize) -> Option<&'static str> {
    if dir_is_empty(&opts.dir) {
        return Some("Choose an output folder");
    }
    if plan_len > 0 {
        return None;
    }
    if opts.flat.is_none() && !opts.per_layer_png && !opts.ora && !opts.psd {
        Some("Choose at least one kind of file")
    } else if opts.layers.is_some() {
        Some("Select at least one layer")
    } else if opts.sets.as_ref().is_some_and(Vec::is_empty) {
        Some("Choose at least one texture set")
    } else {
        Some("Nothing to export")
    }
}

/// The dialog's Layers choice (8.8).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum LayerScope {
    /// The visible layers (`include_hidden: false`).
    Visible,
    /// Hidden layers too (`include_hidden: true`).
    All,
    /// Exactly the layers selected in the Layers list (`layers: Some(..)`).
    Selected,
}

/// The Layers choice the options express.
pub(super) fn scope_of(opts: &ExportOptions) -> LayerScope {
    match (&opts.layers, opts.include_hidden) {
        (Some(_), _) => LayerScope::Selected,
        (None, true) => LayerScope::All,
        (None, false) => LayerScope::Visible,
    }
}

/// Writes a Layers choice into the options: Visible / All = `include_hidden` false / true and no layer list; Selected
/// = these layers (their set decides the texture set, so the set filter is cleared).
pub(super) fn apply_scope(opts: &mut ExportOptions, scope: LayerScope, selected: &[LayerId]) {
    match scope {
        LayerScope::Visible | LayerScope::All => {
            opts.layers = None;
            opts.include_hidden = scope == LayerScope::All;
        }
        LayerScope::Selected => {
            if opts.layers.as_deref() != Some(selected) {
                opts.sets = None;
            }
            opts.layers = Some(selected.to_vec());
        }
    }
}

/// The names of the selected layers for the line under the Layers choice: "A, B, C" or "A, B, C and 2 more".
pub(super) fn selection_names(names: &[String]) -> String {
    const SHOWN: usize = 3;
    if names.len() <= SHOWN {
        names.join(", ")
    } else {
        format!("{} and {} more", names[..SHOWN].join(", "), names.len() - SHOWN)
    }
}

/// Whether set `id` is selected (`None` = all sets).
pub(super) fn is_set_selected(sets: &Option<Vec<TextureSetId>>, id: TextureSetId) -> bool {
    sets.as_ref().is_none_or(|v| v.contains(&id))
}

/// Selects or deselects one set; selecting every set of the document collapses to `None` (= all).
pub(super) fn set_set_selected(sets: &mut Option<Vec<TextureSetId>>, all: &[TextureSetId], id: TextureSetId, on: bool) {
    let mut current: Vec<TextureSetId> = sets.clone().unwrap_or_else(|| all.to_vec());
    current.retain(|s| *s != id);
    if on {
        current.push(id);
    }
    current.sort_unstable();
    current.dedup();
    *sets = if all.iter().all(|s| current.contains(s)) { None } else { Some(current) };
}

/// What one planned file is, for the markers of the file list.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum FileKind {
    Flat,
    /// A per-layer PNG; `separate` = a stamp or text layer ("separate layer", R4).
    Layer { separate: bool },
    Layered,
}

/// The list marker kind of a planned file (`export::plan_contents`, the core's own classification).
pub(super) fn file_kind(kind: PlannedKind) -> FileKind {
    match kind {
        PlannedKind::Flat => FileKind::Flat,
        PlannedKind::Layer { raster, .. } => FileKind::Layer { separate: !raster },
        PlannedKind::Ora | PlannedKind::Psd => FileKind::Layered,
    }
}

/// The planned files that already exist on disk (nothing while the folder is empty: relative paths would point at
/// the working directory).
pub(super) fn existing_files(dir: &Path, plan: &[PathBuf]) -> Vec<PathBuf> {
    if dir_is_empty(dir) {
        return Vec::new();
    }
    plan.iter().filter(|p| p.exists()).cloned().collect()
}

/// The cached file list of the open export dialog: recomputed when the options or the document change.
pub(super) struct PlanCache {
    options: ExportOptions,
    revision: u64,
    generation: u64,
    /// What is written to disk (`export::plan`): the loose files, or the one zip.
    pub(super) plan: Vec<PathBuf>,
    /// The files of the export (`export::plan_contents`): the loose files, or the zip's entries.
    pub(super) names: Vec<String>,
    pub(super) kinds: Vec<FileKind>,
    /// Per `plan` path: whether it exists on disk.
    pub(super) exists: Vec<bool>,
    checked_at: f64,
}

impl PlanCache {
    pub(super) fn build(snapshot: &DocumentSnapshot, options: &ExportOptions, generation: u64, now: f64) -> PlanCache {
        let plan = wte_core::export::plan(snapshot, options);
        let contents = wte_core::export::plan_contents(snapshot, options);
        let mut cache = PlanCache {
            options: options.clone(),
            revision: snapshot.revision,
            generation,
            plan,
            names: contents.iter().map(|f| f.name.clone()).collect(),
            kinds: contents.iter().map(|f| file_kind(f.kind)).collect(),
            exists: Vec::new(),
            checked_at: now,
        };
        cache.check_exists(now);
        cache
    }

    fn check_exists(&mut self, now: f64) {
        let existing = existing_files(&self.options.dir, &self.plan);
        self.exists = self.plan.iter().map(|p| existing.contains(p)).collect();
        self.checked_at = now;
    }
}

/// Brings the plan cache up to date for the current options and document.
fn refresh_plan(ds: &mut DialogsState, cx: &ViewCx<'_>, options: &ExportOptions, now: f64) {
    let Some(session) = cx.state.session.as_ref() else {
        ds.export_plan = None;
        return;
    };
    let (revision, generation) = (session.doc.revision(), session.scene_generation);
    match ds.export_plan.as_mut() {
        Some(c) if c.options == *options && c.revision == revision && c.generation == generation => {
            if now - c.checked_at >= EXISTS_RECHECK_S || now < c.checked_at {
                c.check_exists(now);
            }
        }
        _ => ds.export_plan = Some(PlanCache::build(&session.doc.snapshot(), options, generation, now)),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Draws the export dialog (options, file list, embedded preview, confirm-replace step).
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, ds: &mut DialogsState, st: &mut ExportDialogState) {
    if cx.state.session.is_none() {
        // The document went away underneath the dialog; nothing can be exported.
        super::close(cx);
        return;
    }
    if st.confirm_replace.is_some() {
        show_confirm(ctx, cx, st);
        return;
    }

    sync_from_options(st);
    let now = ctx.input(|i| i.time);
    refresh_plan(ds, cx, &st.options, now);

    let max = super::max_dialog_size(ctx);
    let preview_side = (max.y - 210.0).min(max.x - LEFT_WIDTH - 64.0).min(460.0);
    let preview_fits = preview_side >= MIN_PREVIEW_SIDE;
    let mut actions: Vec<AppAction> = Vec::new();

    super::modal("export").show(ctx, |ui| {
        ui.heading("Export Textures");
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            let left = ui
                .vertical(|ui| {
                    ui.set_width(LEFT_WIDTH);
                    options_ui(ui, cx, ds, st, &mut actions);
                })
                .response
                .rect;
            if st.show_preview && preview_fits {
                // A vertical `ui.separator()` would take the whole available height of the modal's area, which makes
                // the modal grow every frame: the divider is as tall as the columns instead.
                let height = left.height().max(preview_side);
                let (divider, _) = ui.allocate_exact_size(egui::vec2(ui.spacing().item_spacing.x * 2.0, height), egui::Sense::hover());
                ui.painter().vline(divider.center().x, divider.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
                ui.vertical(|ui| {
                    ui.set_width(preview_side);
                    preview_ui(ui, cx, st, preview_side);
                });
            }
        });

        ui.separator();
        let plan_len = ds.export_plan.as_ref().map_or(0, |c| c.plan.len());
        let blocker = export_blocker(&st.options, plan_len);
        super::button_row(ui, |ui| {
            if ui.button("Cancel").clicked() {
                actions.push(AppAction::CloseDialog);
            }
            if ui.add_enabled(blocker.is_none(), egui::Button::new("Export")).clicked() {
                // Fresh check at the moment of the click: never replace a file without asking.
                let plan = ds.export_plan.as_ref().map(|c| c.plan.as_slice()).unwrap_or(&[]);
                let existing = existing_files(&st.options.dir, plan);
                if existing.is_empty() {
                    actions.push(AppAction::RunExport(st.options.clone()));
                } else {
                    st.confirm_replace = Some(existing);
                }
            }
            if let Some(reason) = blocker {
                ui.colored_label(super::warn_color(ui), reason);
            }
            if preview_fits {
                ui.checkbox(&mut st.show_preview, "Show preview");
            }
        });
    });

    for action in actions {
        cx.state.push(action);
    }
}

/// The output folder field. While it is not being edited it shows the end of the path, the middle elided to fit
/// (`elide_middle`; R3-10, finding F9: the start of a long path filled the field and hid the folder's own name), with
/// the full path on hover. A click or Tab into it edits the full path, the cursor at its end.
fn folder_field(ui: &mut egui::Ui, dir: &mut PathBuf, width: f32) {
    let id = ui.make_persistent_id("wte_export_dir");
    let full = dir.to_string_lossy().into_owned();
    if ui.memory(|m| m.has_focus(id)) {
        let mut text = full;
        let resp = ui.add(egui::TextEdit::singleline(&mut text).id(id).desired_width(width).hint_text("Output folder"));
        if resp.changed() {
            *dir = PathBuf::from(text);
        }
        return;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    // The field's text margin plus a little slack, so the elided text is never clipped at its start.
    let inner = width - 2.0 * ui.spacing().button_padding.x - 4.0;
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE).size().x <= inner;
    let mut shown = elide_middle(&full, fits);
    let mut resp = ui.add(egui::TextEdit::singleline(&mut shown).id(id).desired_width(width).hint_text("Output folder"));
    if !full.is_empty() {
        resp = resp.on_hover_text(&full);
    }
    if resp.gained_focus()
        && let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), id)
    {
        // The field showed the elided text this frame; from the next one on it edits the full path.
        let end = egui::text::CCursor::new(full.chars().count());
        edit.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        edit.store(ui.ctx(), id);
    }
}

/// `text` as it is when it `fits`, else its start up to the first path separator (`E:\`), an ellipsis and as much of
/// its end as fits (`E:\…\target\shot_export`); without the start when even that is too long.
pub(crate) fn elide_middle(text: &str, fits: impl Fn(&str) -> bool) -> String {
    const ELLIPSIS: char = '\u{2026}';
    const MIN_TAIL: usize = 8;
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let head_len = chars.iter().position(|c| *c == '\\' || *c == '/').map_or(0, |i| (i + 1).min(12));
    let candidate = |head: usize, start: usize| -> String {
        let mut s: String = chars[..head].iter().collect();
        s.push(ELLIPSIS);
        s.extend(&chars[start..]);
        s
    };
    for head in [head_len, 0] {
        // The smallest start of the tail that still fits (a later start gives a shorter, narrower text).
        let (mut lo, mut hi) = (head.max(1), chars.len());
        // The start is kept only with at least a few characters of the end; else the end gets all the room.
        let min_tail = if head > 0 { MIN_TAIL.min(hi) } else { 0 };
        if !fits(&candidate(head, (hi - min_tail).max(lo))) {
            continue;
        }
        while lo < hi {
            let mid = (lo + hi) / 2;
            if fits(&candidate(head, mid)) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        return candidate(head, lo);
    }
    ELLIPSIS.to_string()
}

/// The left column: folder, name, sets, formats, options and the file list.
fn options_ui(
    ui: &mut egui::Ui,
    cx: &mut ViewCx<'_>,
    ds: &mut DialogsState,
    st: &mut ExportDialogState,
    actions: &mut Vec<AppAction>,
) {
    let set_names: Vec<(TextureSetId, String)> =
        cx.state.doc().map(|d| d.sets.iter().map(|s| (s.id, s.name.clone())).collect()).unwrap_or_default();

    egui::Grid::new("wte_export_options").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Folder");
        ui.horizontal(|ui| {
            folder_field(ui, &mut st.options.dir, LEFT_WIDTH - 170.0);
            if ui.button("Browse...").clicked() {
                actions.push(AppAction::ExportPickFolder(None));
            }
        });
        ui.end_row();

        ui.label("Base name");
        ui.add(egui::TextEdit::singleline(&mut st.options.base_name).desired_width(LEFT_WIDTH - 110.0));
        ui.end_row();

        ui.label("Layers");
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let mut scope = scope_of(&st.options);
                let before = scope;
                ui.selectable_value(&mut scope, LayerScope::Visible, "Visible").on_hover_text("The visible layers");
                ui.selectable_value(&mut scope, LayerScope::All, "All").on_hover_text("Hidden layers too");
                let n = st.selected_layers.len();
                ui.add_enabled_ui(n > 0, |ui| {
                    ui.selectable_value(&mut scope, LayerScope::Selected, format!("Selected ({n})"))
                        .on_hover_text("Only the layers selected in the Layers list (Ctrl / Shift-click), hidden ones too")
                        .on_disabled_hover_text("Select layers in the Layers list first");
                });
                if scope != before {
                    apply_scope(&mut st.options, scope, &st.selected_layers);
                }
            });
            if scope_of(&st.options) == LayerScope::Selected {
                let names: Vec<String> = st
                    .options
                    .layers
                    .iter()
                    .flatten()
                    .filter_map(|id| cx.state.doc().and_then(|d| d.layer(*id)).map(|l| l.name.clone()))
                    .collect();
                ui.add(egui::Label::new(egui::RichText::new(selection_names(&names)).small().weak()).truncate());
            }
        });
        ui.end_row();

        if set_names.len() > 1 && scope_of(&st.options) != LayerScope::Selected {
            ui.label("Texture sets");
            ui.vertical(|ui| {
                let all: Vec<TextureSetId> = set_names.iter().map(|(id, _)| *id).collect();
                for (id, name) in &set_names {
                    let mut on = is_set_selected(&st.options.sets, *id);
                    if ui.checkbox(&mut on, name.as_str()).changed() {
                        set_set_selected(&mut st.options.sets, &all, *id, on);
                    }
                }
            });
            ui.end_row();
        }
    });

    ui.add_space(6.0);
    let mut flat_on = st.options.flat.is_some();
    ui.horizontal(|ui| {
        ui.checkbox(&mut flat_on, "Flattened texture");
        ui.add_enabled_ui(flat_on, |ui| {
            egui::ComboBox::from_id_salt("wte_export_format")
                .selected_text(FORMAT_NAMES[st.format_index.min(FORMAT_NAMES.len() - 1)])
                .show_ui(ui, |ui| {
                    for (i, name) in FORMAT_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut st.format_index, i, *name);
                    }
                });
        });
    });
    if flat_on && st.format_index == 1 {
        ui.indent("wte_export_jpeg", |ui| {
            ui.horizontal(|ui| {
                ui.label("Quality");
                ui.add(egui::Slider::new(&mut st.jpeg_quality, 1..=100));
            });
            ui.horizontal(|ui| {
                ui.label("Matte");
                ui.color_edit_button_srgb(&mut st.jpeg_matte);
                ui.weak("JPEG has no transparency");
            });
        });
    }
    apply_flat(st, flat_on);

    ui.checkbox(&mut st.options.per_layer_png, "Per-layer PNGs (stamps and text as separate files)");
    ui.checkbox(&mut st.options.ora, "OpenRaster (.ora)");
    ui.checkbox(&mut st.options.psd, "Photoshop (.psd)");
    let mut zip = st.options.package == ExportPackage::Zip;
    if ui
        .checkbox(&mut zip, "Pack into one .zip file")
        .on_hover_text("One <base name>.zip in the folder, holding the files listed below")
        .changed()
    {
        st.options.package = if zip { ExportPackage::Zip } else { ExportPackage::Files };
    }
    ui.horizontal(|ui| {
        ui.label("Seam padding");
        ui.add(egui::DragValue::new(&mut st.options.dilate_px).range(0..=MAX_SEAM_PADDING).suffix(" px"));
    })
    .response
    .on_hover_text("Extends the painted areas of the flattened texture beyond the UV island borders");

    // The options may have changed above: the list shows the plan of what is visible now.
    let now = ui.ctx().input(|i| i.time);
    refresh_plan(ds, cx, &st.options, now);

    ui.add_space(6.0);
    let Some(cache) = ds.export_plan.as_ref() else { return };
    let packed = st.options.package == ExportPackage::Zip && !cache.plan.is_empty();
    if packed {
        ui.horizontal(|ui| {
            let n = cache.names.len();
            let inside = if n == 1 { "1 file inside".to_owned() } else { format!("{n} files inside") };
            ui.strong(format!("1 file will be written: {} ({inside})", file_name(&cache.plan[0])));
            if cache.exists.first().copied().unwrap_or(false) {
                ui.colored_label(super::warn_color(ui), "will be replaced");
            }
        });
    } else {
        let replaced = cache.exists.iter().filter(|e| **e).count();
        let header = match (cache.plan.len(), replaced) {
            (1, 0) => "1 file will be written".to_owned(),
            (n, 0) => format!("{n} files will be written"),
            (n, r) => format!("{n} files will be written, {r} replaced"),
        };
        ui.strong(header);
    }
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("wte_export_files").max_height(150.0).auto_shrink([false, true]).show(
            ui,
            |ui| {
                if cache.names.is_empty() {
                    ui.weak("No files");
                }
                for (i, name) in cache.names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(name).monospace()).truncate());
                        if cache.kinds.get(i) == Some(&FileKind::Layer { separate: true }) {
                            ui.colored_label(crate::theme::ACCENT, "separate layer");
                        }
                        if !packed && cache.exists.get(i).copied().unwrap_or(false) {
                            ui.colored_label(super::warn_color(ui), "will be replaced");
                        }
                    });
                }
            },
        );
    });
}

/// The right column: the live preview of the flattened texture on the model, Lit / Unlit and the caption.
fn preview_ui(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, st: &ExportDialogState, side: f32) {
    let matte = preview_matte(st);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    // The widget allocates `rect` itself (orbit / pan / zoom inside the modal); it gets a child Ui over that space.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt("wte_export_preview"));
    crate::view::preview_widget(&mut child, cx, ViewId::ExportPreview, rect, matte);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut cx.state.preview.lit, true, "Lit");
        ui.selectable_value(&mut cx.state.preview.lit, false, "Unlit");
        ui.weak("Drag to orbit, wheel to zoom");
    });
    ui.add(egui::Label::new(egui::RichText::new(preview_caption(&st.options)).small().weak()).wrap());
}

/// The caption under the preview: under a layer selection with a flattened texture, the preview (the whole texture)
/// is not what the flattened file holds.
pub(super) fn preview_caption(opts: &ExportOptions) -> &'static str {
    if opts.layers.is_some() && opts.flat.is_some() {
        "The preview shows the whole texture; the flattened file holds only the selected layers"
    } else {
        "Shows the flattened texture exactly as exported (seam padding and JPEG compression not shown)"
    }
}

/// The confirm step: "N files will be replaced" + list, Replace / Cancel.
fn show_confirm(ctx: &egui::Context, cx: &mut ViewCx<'_>, st: &mut ExportDialogState) {
    let existing = st.confirm_replace.clone().unwrap_or_default();
    let max = super::max_dialog_size(ctx);
    let mut run = false;
    super::modal("export_confirm").show(ctx, |ui| {
        ui.set_width(max.x.min(520.0));
        ui.heading("Replace existing files?");
        let n = existing.len();
        ui.label(if n == 1 { "1 file will be replaced:".to_owned() } else { format!("{n} files will be replaced:") });
        if let Some(dir) = existing.first().and_then(|p| p.parent()) {
            ui.weak(format!("in {}", dir.display()));
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("wte_export_replace").max_height((max.y - 220.0).clamp(80.0, 260.0)).show(
                ui,
                |ui| {
                    for path in &existing {
                        ui.monospace(file_name(path));
                    }
                },
            );
        });
        ui.add_space(6.0);
        super::button_row(ui, |ui| {
            if ui.button("Cancel").clicked() {
                st.confirm_replace = None; // back to the options
            }
            if ui.button("Replace").clicked() {
                run = true;
            }
        });
    });
    if run {
        st.confirm_replace = None;
        cx.state.push(AppAction::RunExport(st.options.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wte_core::{CroppedBuf, Document, IRect, Layer, LayerId, LayerKind, SampleKind, SparseBuf, StampId};

    fn options(dir: &str) -> ExportOptions {
        ExportOptions {
            dir: PathBuf::from(dir),
            base_name: "model".to_owned(),
            sets: None,
            flat: Some(FlatFormat::Png(PngLevel::Balanced)),
            per_layer_png: true,
            ora: false,
            psd: false,
            include_hidden: false,
            dilate_px: 0,
            layers: None,
            package: wte_core::ExportPackage::Files,
        }
    }

    fn dialog(opts: ExportOptions) -> ExportDialogState {
        ExportDialogState {
            options: opts,
            format_index: 0,
            jpeg_quality: 90,
            jpeg_matte: [255, 255, 255],
            show_preview: true,
            confirm_replace: None,
            selected_layers: Vec::new(),
        }
    }

    /// A snapshot of the sphere sample with an extra stamp layer and a hidden text layer on top.
    fn snapshot_with_decals() -> DocumentSnapshot {
        let doc = Document::sample(SampleKind::Sphere);
        let mut snap = doc.snapshot();
        let cache = Arc::new(SparseBuf::from_parts(vec![CroppedBuf { rect: IRect { x0: 0, y0: 0, x1: 1, y1: 1 }, data: vec![1; 4] }]));
        snap.sets[0].layers.push(Layer::new(LayerId(900), "Logo", LayerKind::Stamp { stamp: StampId(901), cache: cache.clone() }));
        let mut text = Layer::new(LayerId(902), "Title", LayerKind::Text { stamp: StampId(903), cache });
        text.visible = false;
        snap.sets[0].layers.push(text);
        snap
    }

    #[test]
    fn format_index_round_trips() {
        for i in 0..FORMAT_NAMES.len() {
            let f = flat_format(i, 80, [1, 2, 3], PngLevel::High);
            assert_eq!(format_index(&f), i);
        }
        assert_eq!(flat_format(0, 80, [0; 3], PngLevel::High), FlatFormat::Png(PngLevel::High));
        assert_eq!(flat_format(1, 0, [9, 8, 7], PngLevel::Fast), FlatFormat::Jpeg { quality: 1, matte: [9, 8, 7] });
        assert_eq!(flat_format(1, 255, [0; 3], PngLevel::Fast), FlatFormat::Jpeg { quality: 100, matte: [0; 3] });
        assert_eq!(flat_format(99, 80, [0; 3], PngLevel::Fast), FlatFormat::Png(PngLevel::Fast));
    }

    #[test]
    fn flat_options_follow_the_dialog_fields() {
        let mut st = dialog(options(r"E:\out"));
        sync_from_options(&mut st);
        assert_eq!(st.format_index, 0);
        assert_eq!(preview_matte(&st), None);

        st.format_index = 1;
        st.jpeg_quality = 70;
        st.jpeg_matte = [10, 20, 30];
        apply_flat(&mut st, true);
        assert_eq!(st.options.flat, Some(FlatFormat::Jpeg { quality: 70, matte: [10, 20, 30] }));
        assert_eq!(preview_matte(&st), Some([10, 20, 30]));

        apply_flat(&mut st, false);
        assert_eq!(st.options.flat, None);
        assert_eq!(preview_matte(&st), None);

        // Options set by code win over stale dialog fields.
        st.options.flat = Some(FlatFormat::Jpeg { quality: 55, matte: [1, 1, 1] });
        st.format_index = 4;
        sync_from_options(&mut st);
        assert_eq!((st.format_index, st.jpeg_quality, st.jpeg_matte), (1, 55, [1, 1, 1]));

        // The PNG compression level of the options is kept.
        st.options.flat = Some(FlatFormat::Png(PngLevel::High));
        sync_from_options(&mut st);
        apply_flat(&mut st, true);
        assert_eq!(st.options.flat, Some(FlatFormat::Png(PngLevel::High)));
        st.format_index = 5;
        apply_flat(&mut st, true);
        assert_eq!(st.options.flat, Some(FlatFormat::WebpLossless));
        assert_eq!(preview_matte(&st), None);
    }

    #[test]
    fn export_is_blocked_with_a_reason() {
        let mut opts = options("");
        assert_eq!(export_blocker(&opts, 3), Some("Choose an output folder"));
        opts.dir = PathBuf::from("   ");
        assert_eq!(export_blocker(&opts, 3), Some("Choose an output folder"));
        opts.dir = PathBuf::from(r"E:\out");
        assert_eq!(export_blocker(&opts, 3), None);
        opts.sets = Some(Vec::new());
        assert_eq!(export_blocker(&opts, 0), Some("Choose at least one texture set"));
        opts.sets = None;
        opts.flat = None;
        opts.per_layer_png = false;
        assert_eq!(export_blocker(&opts, 0), Some("Choose at least one kind of file"));
        opts.per_layer_png = true;
        assert_eq!(export_blocker(&opts, 0), Some("Nothing to export"));
        // A layer selection that matches nothing.
        opts.layers = Some(Vec::new());
        assert_eq!(export_blocker(&opts, 0), Some("Select at least one layer"));
        assert_eq!(export_blocker(&opts, 2), None);
        opts.per_layer_png = false;
        assert_eq!(export_blocker(&opts, 0), Some("Choose at least one kind of file"));
    }

    #[test]
    fn set_selection_collapses_to_all() {
        let all = [TextureSetId(0), TextureSetId(1), TextureSetId(2)];
        let mut sets = None;
        assert!(is_set_selected(&sets, TextureSetId(1)));
        set_set_selected(&mut sets, &all, TextureSetId(1), false);
        assert_eq!(sets, Some(vec![TextureSetId(0), TextureSetId(2)]));
        assert!(!is_set_selected(&sets, TextureSetId(1)));
        set_set_selected(&mut sets, &all, TextureSetId(0), false);
        set_set_selected(&mut sets, &all, TextureSetId(2), false);
        assert_eq!(sets, Some(Vec::new()));
        set_set_selected(&mut sets, &all, TextureSetId(2), true);
        set_set_selected(&mut sets, &all, TextureSetId(2), true);
        assert_eq!(sets, Some(vec![TextureSetId(2)]));
        set_set_selected(&mut sets, &all, TextureSetId(0), true);
        set_set_selected(&mut sets, &all, TextureSetId(1), true);
        assert_eq!(sets, None);
    }

    #[test]
    fn the_list_kinds_come_from_the_core_plan() {
        let snap = snapshot_with_decals();
        let mut opts = options(r"E:\out");
        for (per_layer, ora, psd, hidden, flat) in [
            (true, false, false, false, true),
            (true, true, true, true, true),
            (false, true, false, false, false),
            (true, false, true, true, false),
        ] {
            opts.per_layer_png = per_layer;
            opts.ora = ora;
            opts.psd = psd;
            opts.include_hidden = hidden;
            opts.flat = flat.then_some(FlatFormat::Tga);
            let cache = PlanCache::build(&snap, &opts, 0, 0.0);
            let plan = wte_core::export::plan(&snap, &opts);
            assert_eq!(cache.plan, plan);
            assert_eq!(cache.names, plan.iter().map(|p| file_name(p)).collect::<Vec<_>>());
            assert_eq!(cache.kinds.len(), plan.len());
        }

        opts = options(r"E:\out");
        opts.include_hidden = true;
        // Flat, Base, Paint 1, Logo (stamp), Title (hidden text).
        let kinds = PlanCache::build(&snap, &opts, 0, 0.0).kinds;
        assert_eq!(
            kinds,
            vec![
                FileKind::Flat,
                FileKind::Layer { separate: false },
                FileKind::Layer { separate: false },
                FileKind::Layer { separate: true },
                FileKind::Layer { separate: true },
            ]
        );
        opts.include_hidden = false;
        assert_eq!(PlanCache::build(&snap, &opts, 0, 0.0).kinds.len(), 4);
        opts.sets = Some(vec![TextureSetId(7)]);
        assert!(PlanCache::build(&snap, &opts, 0, 0.0).kinds.is_empty());
        // ORA and PSD are layered files.
        assert_eq!(file_kind(PlannedKind::Ora), FileKind::Layered);
        assert_eq!(file_kind(PlannedKind::Psd), FileKind::Layered);
        assert_eq!(file_kind(PlannedKind::Layer { layer: LayerId(1), index: 0, raster: false }), FileKind::Layer { separate: true });
    }

    #[test]
    fn the_list_for_a_selection_and_for_a_zip_package() {
        let snap = snapshot_with_decals();
        // The stamp and the hidden text layer, selected: two separate-layer files.
        let mut opts = options(r"E:\out");
        opts.flat = None;
        opts.layers = Some(vec![LayerId(902), LayerId(900)]);
        let cache = PlanCache::build(&snap, &opts, 0, 0.0);
        assert_eq!(cache.names, ["model_L02_stamp_Logo.png", "model_L03_text_Title.png"]);
        assert_eq!(cache.kinds, [FileKind::Layer { separate: true }; 2]);
        assert_eq!(cache.plan.len(), 2);
        // Packed: one file on disk, the same names inside.
        opts.package = ExportPackage::Zip;
        let cache = PlanCache::build(&snap, &opts, 0, 0.0);
        assert_eq!(cache.plan, [PathBuf::from(r"E:\out").join("model.zip")]);
        assert_eq!(cache.names, ["model_L02_stamp_Logo.png", "model_L03_text_Title.png"]);
        assert_eq!(cache.exists, [false]);
        // A selection of nothing that exists: nothing to write, not even the zip.
        opts.layers = Some(vec![LayerId(4242)]);
        let cache = PlanCache::build(&snap, &opts, 0, 0.0);
        assert!(cache.plan.is_empty() && cache.names.is_empty());
        assert_eq!(export_blocker(&opts, 0), Some("Select at least one layer"));
    }

    #[test]
    fn the_layers_choice_round_trips() {
        let selected = [LayerId(5), LayerId(7)];
        let mut opts = options(r"E:\out");
        assert_eq!(scope_of(&opts), LayerScope::Visible);
        for scope in [LayerScope::All, LayerScope::Selected, LayerScope::Visible, LayerScope::Selected, LayerScope::All] {
            apply_scope(&mut opts, scope, &selected);
            assert_eq!(scope_of(&opts), scope);
        }
        assert_eq!((opts.layers.clone(), opts.include_hidden), (None, true));
        apply_scope(&mut opts, LayerScope::Visible, &selected);
        assert_eq!((opts.layers.clone(), opts.include_hidden), (None, false));
        // Choosing Selected clears the set filter (the layers decide the set) and lists the layers.
        opts.sets = Some(vec![TextureSetId(1)]);
        apply_scope(&mut opts, LayerScope::Selected, &selected);
        assert_eq!(opts.layers.as_deref(), Some(&selected[..]));
        assert_eq!(opts.sets, None);
        // Choosing it again with the same layers keeps a set filter that came with them (ShowExportLayers).
        opts.sets = Some(vec![TextureSetId(1)]);
        apply_scope(&mut opts, LayerScope::Selected, &selected);
        assert_eq!(opts.sets, Some(vec![TextureSetId(1)]));
        // Leaving Selected keeps include_hidden as it was before.
        apply_scope(&mut opts, LayerScope::All, &selected);
        assert_eq!(opts.layers, None);
        assert_eq!(
            selection_names(&["Logo".to_owned(), "WTE".to_owned()]),
            "Logo, WTE"
        );
        let many: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
        assert_eq!(selection_names(&many), "a, b, c and 2 more");
        assert_eq!(selection_names(&[]), "");
        // The preview caption names what the flattened file holds under a selection.
        let mut opts = options(r"E:\out");
        assert!(preview_caption(&opts).starts_with("Shows the flattened texture"));
        opts.layers = Some(selected.to_vec());
        assert!(preview_caption(&opts).contains("only the selected layers"));
        opts.flat = None;
        assert!(preview_caption(&opts).starts_with("Shows the flattened texture"));
    }

    #[test]
    fn the_list_matches_for_several_sets() {
        let snap = Document::sample(SampleKind::TwoMaterial).snapshot();
        assert!(snap.sets.len() >= 2);
        let mut opts = options(r"E:\out");
        opts.ora = true;
        for sets in [None, Some(vec![TextureSetId(1)]), Some(vec![TextureSetId(1), TextureSetId(0), TextureSetId(1)])] {
            opts.sets = sets;
            let cache = PlanCache::build(&snap, &opts, 0, 0.0);
            assert_eq!(cache.names.len(), wte_core::export::plan(&snap, &opts).len());
            assert_eq!(cache.kinds.len(), cache.names.len());
        }
    }

    #[test]
    fn long_folders_keep_their_end_and_the_middle_is_elided() {
        let fits = |n: usize| move |s: &str| s.chars().count() <= n;
        let path = r"E:\claude_programs\wrapped_texture_editor\target\round3\runs\shot_export";
        assert_eq!(elide_middle(path, fits(200)), path);
        let short = elide_middle(path, fits(30));
        assert_eq!(short.chars().count(), 30, "{short}");
        assert!(short.starts_with("E:\\\u{2026}") && short.ends_with(r"runs\shot_export"), "{short}");
        // Too narrow even for the drive: only the end.
        let tiny = elide_middle(path, fits(8));
        assert_eq!(tiny, "\u{2026}_export");
        assert_eq!(elide_middle(path, fits(0)), "\u{2026}");
        // Forward slashes and paths without a separator.
        assert_eq!(elide_middle("/home/user/projects/out", fits(12)), "/\u{2026}ojects/out");
        assert_eq!(elide_middle("abcdefghij", fits(5)), "\u{2026}ghij");
        assert_eq!(elide_middle("", fits(5)), "");
    }

    #[test]
    fn existing_files_are_found_and_an_empty_folder_finds_nothing() {
        let dir = wte_core::testutil::temp_dir("dialogs_export_existing");
        let present = dir.join("present.png");
        std::fs::write(&present, b"x").unwrap();
        let plan = vec![present.clone(), dir.join("absent.png")];
        assert_eq!(existing_files(&dir, &plan), vec![present.clone()]);
        assert!(existing_files(Path::new(""), &plan).is_empty());

        let snap = snapshot_with_decals();
        let opts = ExportOptions { dir: dir.clone(), ..options("") };
        let plan = wte_core::export::plan(&snap, &opts);
        std::fs::write(&plan[0], b"old").unwrap();
        let cache = PlanCache::build(&snap, &opts, 1, 0.0);
        assert_eq!(cache.exists.iter().filter(|e| **e).count(), 1);
        assert!(cache.exists[0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plan_cache_tracks_the_options() {
        let snap = snapshot_with_decals();
        let opts = options(r"E:\wte_export_dialog_test_dir_that_does_not_exist");
        let cache = PlanCache::build(&snap, &opts, 3, 0.0);
        assert_eq!(cache.plan, wte_core::export::plan(&snap, &opts));
        assert_eq!(cache.kinds.len(), cache.plan.len());
        assert!(cache.exists.iter().all(|e| !e));
        assert_eq!(cache.revision, snap.revision);
        assert_eq!(cache.generation, 3);
    }
}
