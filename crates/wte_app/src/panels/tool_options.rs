// Owner: app-panels (A4); round 4: B4 brush panels. The Tool options section of the side panel (ARCHITECTURE.md
// 8.7): the settings of `state.tool` (changed directly in `AppState`) and, with the Transform tool, the editable
// properties of the selected stamp (as `UpdateStamp*` / `RenameLayer` / `ReplaceStampImage` / `DeleteStamp` actions;
// `Commit::Preview` while a control is dragged or typed in, `Commit::Final` when it is released or loses the focus).
//
// Round 4 (scaffold S2, ROUND4_PLAN 2.2 item 12): the text options live in `text_block.rs` (T6). This file calls
// `text_block::target(state)` and `text_block::show(ui, state, ps, target, actions)` only; the order is the Text block
// first, then the wrong-view hint, then the "Selected: ... Edit" row; under the Transform tool the Text block comes
// before Name, Tint and the placement fields. The "Same side only" checkbox is gone (the Brush tab's Type replaces it).

use eframe::egui;
use egui::color_picker::{Alpha, color_edit_button_srgba};
use egui::{DragValue, Sense, Slider, SliderClamping, TextEdit, vec2};
use wte_core::{FillScope, Placement, Projector, StampObject, StampSource, SurfaceMode, WrapOptions, WrapStats};

use crate::action::AppAction;
use crate::panels::text_block;
use crate::panels::toolbar::tool_name;
use crate::panels::{PanelState, SOURCE_THUMB_SIDE, continuous_edit, from_color32, paint_thumb, percent_slider, to_color32};
use crate::shortcuts::shortcut_text;
use crate::state::{AppState, BrushSettings, StampSettings, ToolKind, ViewMode};

// Round 6 (R6-2, ROUND6_PLAN 5.1): the hover texts of the "On the model: Project | Wrap" selector and of the two Wrap
// options.
pub(crate) const PROJECT_HOVER: &str = "Like a projector from the front: sharp from this side, stretched at the sides";
pub(crate) const WRAP_HOVER: &str = "Lies on the surface like a sticker: goes round the object and over bumps";
pub(crate) const WRAP_ONLY_HOVER: &str = "Used in Wrap mode";
pub(crate) const JUMP_HOVER: &str =
    "Continue onto separate parts closer than 1 % of the model size; off: stop at the edge of a part";
pub(crate) const ENDS_MEET_HOVER: &str = "Ends meet: the middle part shows, the ends meet at the back";
pub(crate) const OVERLAP_HOVER: &str = "Overlap: keeps winding like tape, the end on top";
/// Hover text of "Project through" while it is greyed out (Wrap).
const THROUGH_WRAP_HOVER: &str = "Used in Project mode: a wrapped stamp lies on the surface itself";

/// The selector "On the model: Project | Wrap" (round 6). Returns the mode clicked when it differs from `mode`.
pub(crate) fn mode_selector(ui: &mut egui::Ui, mode: SurfaceMode) -> Option<SurfaceMode> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label("On the model:");
        for (m, text, hover) in [(SurfaceMode::Project, "Project", PROJECT_HOVER), (SurfaceMode::Wrap, "Wrap", WRAP_HOVER)] {
            if ui.add(egui::Button::selectable(mode == m, text)).on_hover_text(hover).clicked() && mode != m {
                picked = Some(m);
            }
        }
    });
    picked
}

/// The two options of Wrap (round 6): the checkbox "Jump small gaps" and the selector "Longer than the way round:
/// Ends meet | Overlap". Shown in Project mode too, greyed out ("Used in Wrap mode"). `probe` records whether they
/// could be used in this frame (`ToolProbe::wrap_controls`). Returns the options when a control changed them.
pub(crate) fn wrap_option_controls(
    ui: &mut egui::Ui,
    mode: SurfaceMode,
    opts: WrapOptions,
    probe: &mut Option<bool>,
) -> Option<WrapOptions> {
    let wrap = mode == SurfaceMode::Wrap;
    *probe = Some(wrap && ui.is_enabled());
    let mut out = opts;
    ui.add_enabled_ui(wrap, |ui| {
        ui.checkbox(&mut out.jump_gaps, "Jump small gaps").on_hover_text(JUMP_HOVER).on_disabled_hover_text(WRAP_ONLY_HOVER);
        // Check:B: the label on its own row; side by side the row was wider than the panel and "Overlap" wrapped alone
        // onto the next line.
        ui.label("Longer than the way round:").on_disabled_hover_text(WRAP_ONLY_HOVER);
        ui.horizontal(|ui| {
            let choices = [(false, "Ends meet", ENDS_MEET_HOVER), (true, "Overlap", OVERLAP_HOVER)];
            for (overlap, text, hover) in choices {
                let resp = ui.add(egui::Button::selectable(opts.overlap == overlap, text));
                if resp.on_hover_text(hover).on_disabled_hover_text(WRAP_ONLY_HOVER).clicked() {
                    out.overlap = overlap;
                }
            }
        });
    });
    (out != opts).then_some(out)
}

/// The surface options of the Stamp and the Text tool (shared, `StampSettings`, round 6): the mode selector, the two
/// Wrap options and "Project through" (greyed out in Wrap; `through_hover` is its hover text).
pub(crate) fn tool_surface_options(ui: &mut egui::Ui, st: &mut StampSettings, probe: &mut Option<bool>, through_hover: &str) {
    if let Some(mode) = mode_selector(ui, st.mode) {
        st.mode = mode;
    }
    if let Some(wrap) = wrap_option_controls(ui, st.mode, st.wrap, probe) {
        st.wrap = wrap;
    }
    let project = st.mode == SurfaceMode::Project;
    ui.add_enabled(project, egui::Checkbox::new(&mut st.project_through, "Project through"))
        .on_hover_text(through_hover)
        .on_disabled_hover_text(THROUGH_WRAP_HOVER);
}

/// The lines under the selector of a selected wrapped stamp (round 6): how much surface it covers and what happened
/// (from `Document::stamp_wrap_stats`; `None` = not computed yet: its bake or the parameterization is on its way).
fn wrap_stats_lines(ui: &mut egui::Ui, stats: Option<WrapStats>, opts: WrapOptions) {
    let Some(s) = stats else {
        help(ui, "Wrapping...");
        return;
    };
    help(ui, &format!("Wrapped over {} triangles", s.tris));
    for line in wrap_stats_texts(&s, opts) {
        match line {
            WrapLine::Help(text) => help(ui, &text),
            WrapLine::Warn(text) => {
                ui.add(egui::Label::new(egui::RichText::new(text).color(ui.visuals().warn_fg_color)).wrap());
            }
        }
    }
}

/// Least `WrapStats::stretch` (95th percentile of |area ratio - 1|) for which a relaxed wrap says how much it stretched.
const STRETCH_LINE_MIN: f32 = 0.05;

/// One line under "Wrapped over N triangles": plain help text or a warning.
#[derive(Clone, Debug, PartialEq)]
enum WrapLine {
    Help(String),
    Warn(String),
}

/// The lines under "Wrapped over N triangles" of `wrap_stats_lines`, in order. Round 8 (R8-1, plan 4.1): a relaxed wrap
/// that stretched by 5 % or more says so (help text: it is the intended behaviour on a curved surface), and seams
/// inside the picture are named; the round-6 warnings follow unchanged.
fn wrap_stats_texts(s: &WrapStats, opts: WrapOptions) -> Vec<WrapLine> {
    let mut out = Vec::new();
    if s.relaxed && s.stretch >= STRETCH_LINE_MIN {
        let pct = (s.stretch * 100.0).round().max(1.0);
        out.push(WrapLine::Help(format!("Fitted to the curved surface: stretched up to {pct:.0} %")));
    }
    if s.seams > 0 {
        out.push(WrapLine::Help("Cut where it cannot lie flat (a corner, a handle, or where its edges meet)".to_owned()));
    }
    if s.covered < 0.98 {
        let off = ((1.0 - s.covered) * 100.0).round().max(1.0);
        out.push(WrapLine::Warn(format!("{off:.0} % of the picture runs off the surface: an edge, a hole or the end of the part")));
    }
    if s.met_itself && !opts.overlap {
        out.push(WrapLine::Warn("The ends meet at the back".to_owned()));
    }
    if s.turns > 1 {
        out.push(WrapLine::Warn(format!("Wound {} times", s.turns)));
    }
    if s.jumps > 0 {
        out.push(WrapLine::Warn(format!("Continued onto {} separate part{}", s.jumps, if s.jumps == 1 { "" } else { "s" })));
    }
    if s.limited {
        out.push(WrapLine::Warn("Too large for this model: the wrap stops at 1 million vertices / 8 turns".to_owned()));
    }
    out
}

/// Width of the label column of the option grids.
const LABEL_WIDTH: f32 = 72.0;
/// Upper end of the Depth front / back fields (% of the model size). Corner and wheel scaling of a projected decal
/// scale its depth too (transform.rs), so the value may also lie above it; the field shows it unchanged.
const MAX_DEPTH_PCT: f32 = 1000.0;

/// A `DragValue` bound to a document value: the range limits what the user can drag or type, but an existing value
/// outside it (a UV stamp turned past 180 degrees, a depth scaled past the limit) is shown as it is. egui's default
/// `clamp_existing_to_range(true)` would clamp it on every redraw and report that as a change, which
/// `continuous_edit` would turn into an edit of the document (and, after an Undo of it, into a new one).
fn doc_value(value: &mut f32, range: std::ops::RangeInclusive<f32>) -> DragValue<'_> {
    DragValue::new(value).range(range).clamp_existing_to_range(false)
}

/// Draws the options of the active tool, or the properties of the selected stamp. Returns the rect of the section
/// (header and body as laid out) for `AppState::side_rects`.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) -> egui::Rect {
    if state.tool != ToolKind::Text {
        state.focus_text_content = false;
    }
    // Round 6: set again below by whichever options show the two Wrap controls in this frame.
    state.tool_probe.wrap_controls = None;
    let mut header = egui::CollapsingHeader::new(format!("{} options", tool_name(state.tool)))
        .id_salt("side_tool_options")
        .default_open(true);
    if state.focus_text_content {
        header = header.open(Some(true));
    }
    // A selected stamp that the current view cannot edit (3D stamp in the 2D view or the other way round) says so,
    // here and in the status bar, instead of the gizmo silently not appearing.
    let hint = wrong_view_hint(state);
    let stale = |h: &str| h.starts_with(WRONG_VIEW_PREFIX);
    match &hint {
        Some((text, _)) => state.status_hint.clone_from(text),
        None if stale(&state.status_hint) => state.status_hint.clear(),
        None => {}
    }
    let text_target = text_block::target(state);
    let out = header.show(ui, |ui| {
        let mut actions = Vec::new();
        // The Text block first (FEATURE_TEXT 6.1): with the Text tool the text to place, with the Transform tool the
        // selected text (above its Name, Tint and placement fields).
        if let Some(target) = text_target {
            text_block::show(ui, state, ps, target, &mut actions);
            if hint.is_some() {
                ui.separator();
            }
        }
        if let Some((text, mode)) = &hint {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, text.as_str());
                let label = if *mode == ViewMode::View3D { "Show 3D view" } else { "Show 2D view" };
                if ui.button(label).clicked() {
                    actions.push(AppAction::SetViewMode(*mode));
                }
            });
            ui.separator();
        }
        match state.tool {
            ToolKind::Navigate => {
                help(ui, "Drag: orbit (3D) or pan (2D). Right-drag: orbit. Middle-drag or Space+drag: pan. Wheel: zoom.");
            }
            ToolKind::Brush => brush_options(ui, "brush", &mut state.tools.brush),
            ToolKind::Eraser => brush_options(ui, "eraser", &mut state.tools.eraser),
            ToolKind::Eyedropper => {
                help(ui, "Click the model or the texture to pick a colour. Holding Alt with the Brush, Eraser or Fill does the same.");
            }
            ToolKind::Fill => {
                ui.horizontal(|ui| {
                    ui.label("Fill");
                    ui.radio_value(&mut state.tools.fill_scope, FillScope::Island, "UV island")
                        .on_hover_text("Fills the UV island under the cursor (and its seam padding)");
                    ui.radio_value(&mut state.tools.fill_scope, FillScope::Layer, "Whole layer");
                });
                help(ui, "Fills the active layer with the primary colour.");
            }
            ToolKind::Stamp => stamp_options(ui, state, ps, &mut actions),
            // The Text block above is the whole of the Text options.
            ToolKind::Text => {}
            ToolKind::Transform => match selected_stamp(state) {
                Some(stamp) => stamp_properties(ui, state, ps, stamp, &mut actions),
                None => help(ui, "Click a stamp or a text on the model to select it, or select its layer in the Layers list."),
            },
        }
        // Every tool but Transform, the Text tool included (FEATURE_TEXT 6.2): the selected stamp and the way to edit it.
        if state.tool != ToolKind::Transform
            && let Some(stamp) = selected_stamp(state)
        {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("Selected: {}", stamp.name));
                let key = shortcut_text(ui.ctx(), "Transform");
                if ui.button("Edit").on_hover_text(format!("Transform tool ({key})")).clicked() {
                    actions.push(AppAction::SetTool(ToolKind::Transform));
                }
            });
        }
        state.actions.extend(actions);
    });
    out.body_response.as_ref().map_or(out.header_response.rect, |b| out.header_response.rect.union(b.rect))
}

/// Start of every wrong-view hint (so a stale one in the status bar is recognised and cleared).
const WRONG_VIEW_PREFIX: &str = "This is a ";

/// The hint for a selected stamp that the current view mode cannot edit, and the view mode that can: a projected
/// (3D) stamp in the 2D view, a UV-placed (2D) stamp in the 3D view. Split shows both views, Preview edits nothing.
pub(crate) fn wrong_view_hint(state: &AppState) -> Option<(String, ViewMode)> {
    let stamp = selected_stamp(state)?;
    let text = matches!(stamp.source, StampSource::Text(_));
    let kind = |dim: &str| format!("{WRONG_VIEW_PREFIX}{dim} {}", if text { "text" } else { "stamp" });
    match (stamp.placement, state.view_mode) {
        (Placement::Projected(_), ViewMode::View2D) => Some((format!("{} - edit it in the 3D view", kind("3D")), ViewMode::View3D)),
        (Placement::Uv(_), ViewMode::View3D) => Some((format!("{} - edit it in the 2D view", kind("2D")), ViewMode::View2D)),
        _ => None,
    }
}

/// The selected stamp, if it still exists.
fn selected_stamp(state: &AppState) -> Option<StampObject> {
    let session = state.session.as_ref()?;
    session.selected_stamp.and_then(|id| session.doc.stamp(id)).cloned()
}

fn help(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

fn grid_label(ui: &mut egui::Ui, text: &str) {
    ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(text).truncate());
    });
}

fn brush_options(ui: &mut egui::Ui, salt: &str, b: &mut BrushSettings) {
    egui::Grid::new(("brush_grid", salt)).num_columns(2).show(ui, |ui| {
        grid_label(ui, "Size");
        let keys = format!("{} / {}", shortcut_text(ui.ctx(), "Smaller Brush"), shortcut_text(ui.ctx(), "Larger Brush"));
        ui.add(Slider::new(&mut b.size_px, 1.0..=2048.0).logarithmic(true).max_decimals(0).suffix(" px"))
            .on_hover_text(format!("Brush diameter in texels; a brush shape is scaled to fit inside this ring ({keys})"));
        ui.end_row();
        grid_label(ui, "Hardness");
        let keys = format!("{} / {}", shortcut_text(ui.ctx(), "Softer Brush"), shortcut_text(ui.ctx(), "Harder Brush"));
        ui.add(percent_slider(&mut b.hardness, 0.0, 100.0).max_decimals(0)).on_hover_text(keys);
        ui.end_row();
        grid_label(ui, "Opacity");
        ui.add(percent_slider(&mut b.opacity, 1.0, 100.0).max_decimals(0)).on_hover_text("Maximum coverage of one stroke");
        ui.end_row();
        grid_label(ui, "Flow");
        ui.add(percent_slider(&mut b.flow, 1.0, 100.0).max_decimals(0)).on_hover_text("Build-up per dab");
        ui.end_row();
        grid_label(ui, "Spacing");
        ui.add(percent_slider(&mut b.spacing, 2.0, 100.0).max_decimals(0))
            .on_hover_text("Distance between dabs, % of the diameter. A thin shape needs a lower value.");
        ui.end_row();
    });
}

fn stamp_options(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>) {
    let has_doc = state.session.is_some();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(SOURCE_THUMB_SIDE, SOURCE_THUMB_SIDE), Sense::hover());
        let thumb = state.tools.stamp.source.and_then(|blob| ps.source_thumb(ui.ctx(), state, blob));
        let aspect = thumb.map_or(1.0, |(_, [w, h])| w as f32 / h.max(1) as f32);
        paint_thumb(ui.painter(), rect, thumb.map(|(t, _)| t), aspect);
        ui.vertical(|ui| {
            match state.tools.stamp.source {
                Some(_) => ui.label(&state.tools.stamp.source_name),
                None => ui.weak("No image chosen"),
            };
            let choose = ui.add_enabled(has_doc, egui::Button::new("Choose..."));
            if choose.on_disabled_hover_text("Open a model first").on_hover_text("Choose the image to stamp").clicked() {
                actions.push(AppAction::ImportStampImage(None));
            }
        });
    });
    let st = &mut state.tools.stamp;
    egui::Grid::new("stamp_grid").num_columns(2).show(ui, |ui| {
        grid_label(ui, "Size");
        // `SliderClamping::Edits`: a Ctrl+wheel size beyond the slider range (stamp_place.rs) is kept, not clamped back.
        ui.add(percent_slider(&mut st.size_frac, 1.0, 100.0).logarithmic(true).clamping(SliderClamping::Edits))
            .on_hover_text("Larger side of the stamp, % of the model size (2D: of the texture width). Ctrl+wheel");
        ui.end_row();
        grid_label(ui, "Rotation");
        ui.add(Slider::new(&mut st.rotation_deg, -180.0..=180.0).suffix(" °").max_decimals(0)).on_hover_text("Shift+wheel");
        ui.end_row();
    });
    // Round 6: Project | Wrap and the two Wrap options for new 3D stamps (shared with the Text tool).
    tool_surface_options(
        ui,
        st,
        &mut state.tool_probe.wrap_controls,
        "Also print on surfaces behind the first one (the far side of the model)",
    );
    help(ui, "Click on the model or the texture to place the stamp. It becomes its own layer.");
}

// ---- the selected stamp ----

/// Larger side of a placement in percent of the model diagonal (projected) or of its set's texture width (UV).
pub(crate) fn placement_size_pct(p: &Placement, diag: f32, tex_width: f32) -> f32 {
    match p {
        Placement::Projected(pr) => 2.0 * pr.half_size.max_element() / diag.max(1e-9) * 100.0,
        Placement::Uv(uv) => uv.size_px.max_element() / tex_width.max(1.0) * 100.0,
    }
}

/// The placement scaled uniformly about its centre.
pub(crate) fn scale_placement(p: &Placement, factor: f32) -> Placement {
    let mut p = *p;
    match &mut p {
        Placement::Projected(pr) => pr.half_size *= factor,
        Placement::Uv(uv) => uv.size_px *= factor,
    }
    p
}

/// Rotation of a projector in radians, clockwise as seen by the viewer (+Z), the sense of `Projector::rotated`:
/// the angle of its up axis from the up axis of the unrotated frame `Projector::from_surface(.., +Y, ..)` at the
/// same point and normal.
pub(crate) fn projector_angle(p: &Projector) -> f32 {
    let (_, up, out) = p.axes();
    let reference = Projector::from_surface(p.origin, out, glam::Vec3::Y, p.half_size, 1.0);
    let (ref_right, ref_up, _) = reference.axes();
    up.dot(ref_right).atan2(up.dot(ref_up))
}

/// Rotation of a placement in degrees (clockwise positive, 4.1).
pub(crate) fn placement_rotation_deg(p: &Placement) -> f32 {
    match p {
        Placement::Projected(pr) => projector_angle(pr).to_degrees(),
        Placement::Uv(uv) => uv.rotation.to_degrees(),
    }
}

/// The placement turned to an absolute rotation in degrees.
pub(crate) fn rotate_placement_to(p: &Placement, deg: f32) -> Placement {
    match p {
        Placement::Projected(pr) => Placement::Projected(pr.rotated(deg.to_radians() - projector_angle(pr))),
        Placement::Uv(uv) => {
            let mut uv = *uv;
            uv.rotation = deg.to_radians();
            Placement::Uv(uv)
        }
    }
}

fn stamp_properties(
    ui: &mut egui::Ui,
    state: &mut AppState,
    ps: &mut PanelState,
    stamp: StampObject,
    actions: &mut Vec<AppAction>,
) {
    let id = stamp.id;
    let Some(doc) = state.doc() else { return };
    let diag = doc.mesh.bounds.diagonal();
    let home_layer = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == stamp.home_set).map(|(_, l)| l);
    let tex_width = match &stamp.placement {
        Placement::Uv(uv) => doc.set(uv.set).map_or(1.0, |s| s.width as f32),
        Placement::Projected(_) => 1.0,
    };
    let source_name = match &stamp.source {
        StampSource::Image { blob } => doc.blobs.get(*blob).map(|b| b.name.clone()),
        StampSource::Text(_) => None,
    };
    // A locked linked layer protects the stamp (core `stamp_locked`): only the name stays editable.
    let locked = doc.stamp_locked(id);
    if locked {
        ui.colored_label(ui.visuals().warn_fg_color, crate::panels::layers::stamp_lock_message(doc, id));
        // The lock may sit in a set the Layers list does not show right now: offer to go there.
        let active_set = state.session.as_ref().map(|s| s.active_set);
        let locking: Vec<_> = doc
            .stamp_layers(id)
            .into_iter()
            .filter(|(set, l)| Some(*set) != active_set && doc.layer(*l).is_some_and(|l| l.locked))
            .collect();
        if let Some((set, _)) = locking.first() {
            let name = doc.set(*set).map_or_else(String::new, |s| s.name.clone());
            if ui.button(format!("Show set \"{name}\"")).on_hover_text("Show that set's layers to unlock the layer").clicked() {
                actions.push(AppAction::SetActiveSet(*set));
            }
        }
    }

    // Name (renames the stamp and its linked layers).
    egui::Grid::new(("stamp_props", id.0)).num_columns(2).show(ui, |ui| {
        grid_label(ui, "Name");
        let mut name = match &ps.stamp_name {
            Some((sid, text)) if *sid == id => text.clone(),
            _ => stamp.name.clone(),
        };
        let resp = ui.add(TextEdit::singleline(&mut name).id_salt(("stamp_name", id.0)).desired_width(f32::INFINITY));
        if resp.has_focus() {
            ps.stamp_name = Some((id, name));
        } else {
            let trimmed = name.trim();
            if resp.lost_focus()
                && !trimmed.is_empty()
                && trimmed != stamp.name
                && let Some(layer) = home_layer
            {
                actions.push(AppAction::RenameLayer(layer, trimmed.to_owned()));
            }
            ps.stamp_name = None;
        }
        ui.end_row();

        grid_label(ui, "Tint");
        let mut c = to_color32(stamp.tint);
        let resp = ui.add_enabled_ui(!locked, |ui| color_edit_button_srgba(ui, &mut c, Alpha::OnlyBlend)).inner;
        let tint = from_color32(c);
        continuous_edit(ps, actions, ui, &resp, |commit| AppAction::UpdateStampTint(id, tint, commit));
        ui.end_row();

        if let StampSource::Image { .. } = &stamp.source {
            grid_label(ui, "Image");
            ui.horizontal(|ui| {
                ui.label(source_name.as_deref().unwrap_or("(missing)"));
                if ui.add_enabled(!locked, egui::Button::new("Choose...")).on_hover_text("Replace the image of this stamp").clicked() {
                    actions.push(AppAction::ReplaceStampImage(id, None));
                }
            });
            ui.end_row();
        }
    });

    // The text fields of a text stamp are the Text block above (`text_block::show`, round 4).
    ui.separator();
    let mut probe = None;
    ui.add_enabled_ui(!locked, |ui| placement_fields(ui, ps, state.doc(), &stamp, diag, tex_width, actions, &mut probe));
    if probe.is_some() {
        state.tool_probe.wrap_controls = probe;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!locked, egui::Button::new("Delete stamp")).on_disabled_hover_text("The layer is locked").clicked() {
            actions.push(AppAction::DeleteStamp(id));
        }
        if ui.button("Deselect").on_hover_text(shortcut_text(ui.ctx(), "Cancel")).clicked() {
            actions.push(AppAction::SelectStamp(None));
        }
    });
}

/// `new` (an edit of the projected placement `old`) with its depth range re-fitted to the surface when the depth of
/// `old` is the automatic one (6.5); unchanged otherwise, for UV placements and when nothing changed.
fn refit_auto_depth(doc: Option<&wte_core::Document>, old: &Placement, new: Placement) -> Placement {
    match (new, old, doc) {
        (Placement::Projected(p), Placement::Projected(o), Some(doc)) if new != *old && doc.stamp_depth_is_auto(o) => {
            Placement::Projected(doc.fit_stamp_depth(&p))
        }
        _ => new,
    }
}

/// Size, rotation and the projection options of the selected stamp (`UpdateStampPlacement`). Round 6: a 3D stamp
/// starts with the "On the model: Project | Wrap" selector, the two Wrap options (each change one Final edit) and, in
/// Wrap, the lines of `wrap_stats_lines`; in Wrap the depth, angle, back-face and occlusion rows are hidden (kept in
/// the placement for switching back). `probe` records the Wrap option controls (`ToolProbe::wrap_controls`).
#[expect(clippy::too_many_arguments, reason = "the fields' whole context")]
fn placement_fields(
    ui: &mut egui::Ui,
    ps: &mut PanelState,
    doc: Option<&wte_core::Document>,
    stamp: &StampObject,
    diag: f32,
    tex_width: f32,
    actions: &mut Vec<AppAction>,
    probe: &mut Option<bool>,
) {
    let id = stamp.id;
    let placement = stamp.placement;
    let place = |p: Placement| move |commit| AppAction::UpdateStampPlacement(id, p, commit);
    let wrapped = matches!(placement, Placement::Projected(pr) if pr.mode == SurfaceMode::Wrap);
    if let Placement::Projected(pr) = placement {
        let final_edit = |p: Projector| AppAction::UpdateStampPlacement(id, Placement::Projected(p), wte_core::Commit::Final);
        if let Some(mode) = mode_selector(ui, pr.mode) {
            actions.push(final_edit(Projector { mode, ..pr }));
        }
        if let Some(wrap) = wrap_option_controls(ui, pr.mode, pr.wrap, probe) {
            actions.push(final_edit(Projector { wrap, ..pr }));
        }
        if wrapped {
            wrap_stats_lines(ui, doc.and_then(|d| d.stamp_wrap_stats(id)), pr.wrap);
        }
    }
    // Fix round 2, item 2: a new size or rotation of a projected stamp whose depth range is automatic re-fits it to the
    // surface under the new footprint, like the Transform tool does; a hand-set depth is kept. Only evaluated when the
    // field changed (the auto test itself is a fit).
    let refit = |p: Placement| refit_auto_depth(doc, &placement, p);
    egui::Grid::new(("stamp_placement", id.0)).num_columns(2).show(ui, |ui| {
        grid_label(ui, "Size");
        let size = placement_size_pct(&placement, diag, tex_width);
        let mut new_size = size;
        let what = if matches!(placement, Placement::Projected(_)) { "model size" } else { "texture width" };
        let resp = ui
            .add(doc_value(&mut new_size, 0.1..=500.0).speed(0.2).max_decimals(1).suffix(" %"))
            .on_hover_text(format!("Larger side of the stamp, % of the {what}"));
        let scaled = if size > 0.0 && new_size > 0.0 { scale_placement(&placement, new_size / size) } else { placement };
        continuous_edit(ps, actions, ui, &resp, place(refit(scaled)));
        ui.end_row();

        grid_label(ui, "Rotation");
        let mut deg = placement_rotation_deg(&placement);
        let resp = ui.add(doc_value(&mut deg, -180.0..=180.0).speed(0.5).max_decimals(1).suffix(" °"));
        let rotated = rotate_placement_to(&placement, deg);
        continuous_edit(ps, actions, ui, &resp, place(refit(rotated)));
        ui.end_row();

        if let Placement::Projected(pr) = placement
            && !wrapped
        {
            let pct = |v: f32| v / diag.max(1e-9) * 100.0;
            let world = |v: f32| v * diag.max(1e-9) / 100.0;

            grid_label(ui, "Depth front");
            let mut front = pct(pr.depth_front);
            let resp = ui
                .add(doc_value(&mut front, 0.05..=MAX_DEPTH_PCT).speed(0.2).max_decimals(1).suffix(" %"))
                .on_hover_text("Reach of the projection toward the viewer, % of the model size");
            let p = Projector { depth_front: world(front), ..pr };
            continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
            ui.end_row();

            grid_label(ui, "Depth back");
            let mut back = pct(pr.depth_back);
            let resp = ui
                .add(doc_value(&mut back, 0.05..=MAX_DEPTH_PCT).speed(0.2).max_decimals(1).suffix(" %"))
                .on_hover_text("Reach of the projection into the model, % of the model size");
            let p = Projector { depth_back: world(back), ..pr };
            continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
            ui.end_row();

            grid_label(ui, "");
            let fit = ui
                .add_enabled(doc.is_some(), egui::Button::new("Fit depth to surface"))
                .on_hover_text("Set the depth range to the surface under the stamp (as when it was placed). Moving or scaling it then keeps fitting it");
            if fit.clicked()
                && let Some(doc) = doc
            {
                actions.push(AppAction::UpdateStampPlacement(id, Placement::Projected(doc.fit_stamp_depth(&pr)), wte_core::Commit::Final));
            }
            ui.end_row();

            grid_label(ui, "Angle limits");
            ui.horizontal(|ui| {
                let mut start = pr.angle_start_deg;
                let resp = ui
                    .add(doc_value(&mut start, 0.0..=pr.angle_end_deg.min(90.0)).speed(0.5).max_decimals(0).suffix(" °"))
                    .on_hover_text("Full strength while the surface faces the projector within this angle");
                let p = Projector { angle_start_deg: start, ..pr };
                continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
                let mut end = pr.angle_end_deg;
                let resp = ui
                    .add(doc_value(&mut end, pr.angle_start_deg.max(0.0)..=90.0).speed(0.5).max_decimals(0).suffix(" °"))
                    .on_hover_text("No paint beyond this angle");
                let p = Projector { angle_end_deg: end, ..pr };
                continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
            });
            ui.end_row();
        }
    });

    ui.horizontal_wrapped(|ui| {
        let (mut fx, mut fy) = match placement {
            Placement::Projected(pr) => (pr.flip_x, pr.flip_y),
            Placement::Uv(uv) => (uv.flip_x, uv.flip_y),
        };
        let fx_changed = ui.checkbox(&mut fx, "Flip X").changed();
        let fy_changed = ui.checkbox(&mut fy, "Flip Y").changed();
        if fx_changed || fy_changed {
            let mut p = placement;
            match &mut p {
                Placement::Projected(pr) => (pr.flip_x, pr.flip_y) = (fx, fy),
                Placement::Uv(uv) => (uv.flip_x, uv.flip_y) = (fx, fy),
            }
            actions.push(AppAction::UpdateStampPlacement(id, p, wte_core::Commit::Final));
        }
        match placement {
            // Wrap ignores the back-face and occlusion settings (kept for switching back).
            Placement::Projected(_) if wrapped => {}
            Placement::Projected(pr) => {
                let mut cull = pr.backface_cull;
                if ui.checkbox(&mut cull, "Backface cull").on_hover_text("Surfaces facing away receive nothing").changed() {
                    let p = Projector { backface_cull: cull, ..pr };
                    actions.push(AppAction::UpdateStampPlacement(id, Placement::Projected(p), wte_core::Commit::Final));
                }
                let mut occlusion = pr.occlusion;
                if ui.checkbox(&mut occlusion, "Occlusion").on_hover_text("Surfaces hidden behind others receive nothing").changed() {
                    let p = Projector { occlusion, ..pr };
                    actions.push(AppAction::UpdateStampPlacement(id, Placement::Projected(p), wte_core::Commit::Final));
                }
            }
            Placement::Uv(uv) => {
                let mut clip = uv.clip_to_islands;
                if ui.checkbox(&mut clip, "Clip to UV islands").changed() {
                    let mut uv = uv;
                    uv.clip_to_islands = clip;
                    actions.push(AppAction::UpdateStampPlacement(id, Placement::Uv(uv), wte_core::Commit::Final));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::tests::{run_panels, sample_state};
    use glam::{Vec2, Vec3};
    use wte_core::{Document, Outline, SampleKind, StampId, TextAlign, TextSpec, TextureSetId, UvPlacement};

    fn projector() -> Projector {
        Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1), 0.2)
    }

    /// Round 8 (plan 4.1 / 4.2): the two new lines under "Wrapped over N triangles".
    #[test]
    fn a_relaxed_or_cut_wrap_says_so_under_the_stats_line() {
        let base = wte_core::WrapStats {
            verts: 100,
            tris: 180,
            cut_tris: 0,
            covered: 1.0,
            met_itself: false,
            reached_edge: false,
            limited: false,
            jumps: 0,
            turns: 1,
            micros: 1000,
            relaxed: false,
            seams: 0,
            seams_by_rule: [0; 5],
            flipped: 0,
            stretch: 0.0,
            rejected: 0.0,
            cg_iters: 0,
            passed: false,
            ends_share: 1.0,
            lost_cut: 0.0,
        };
        let opts = WrapOptions::default();
        assert!(wrap_stats_texts(&base, opts).is_empty(), "an exact (seed) wrap adds no line");
        let fitted = "Fitted to the curved surface: stretched up to 35 %";
        let cut = "Cut where it cannot lie flat (a corner, a handle, or where its edges meet)";
        let relaxed = wte_core::WrapStats { relaxed: true, stretch: 0.349, ..base };
        assert_eq!(wrap_stats_texts(&relaxed, opts), vec![WrapLine::Help(fitted.to_owned())]);
        // Below 5 % the stretch is not worth a line; a seed layout never says "fitted".
        assert!(wrap_stats_texts(&wte_core::WrapStats { stretch: 0.049, ..relaxed }, opts).is_empty());
        assert!(wrap_stats_texts(&wte_core::WrapStats { relaxed: false, ..relaxed }, opts).is_empty());
        let both = wte_core::WrapStats { seams: 3, seams_by_rule: [0, 3, 0, 0, 0], met_itself: true, ..relaxed };
        assert_eq!(
            wrap_stats_texts(&both, opts),
            vec![
                WrapLine::Help(fitted.to_owned()),
                WrapLine::Help(cut.to_owned()),
                WrapLine::Warn("The ends meet at the back".to_owned())
            ]
        );
        // The round-6 warnings stay as they were, after the new lines.
        let off = wte_core::WrapStats { covered: 0.5, jumps: 2, ..base };
        assert_eq!(
            wrap_stats_texts(&off, opts),
            vec![
                WrapLine::Warn("50 % of the picture runs off the surface: an edge, a hole or the end of the part".to_owned()),
                WrapLine::Warn("Continued onto 2 separate parts".to_owned())
            ]
        );
    }

    #[test]
    fn projector_angle_matches_rotated() {
        let p = projector();
        assert!(projector_angle(&p).abs() < 1e-5);
        for a in [0.3f32, -1.2, 2.5] {
            assert!((projector_angle(&p.rotated(a)) - a).abs() < 1e-4, "{a}");
        }
        // A tilted surface normal with a non-vertical up hint.
        let q = Projector::from_surface(Vec3::ZERO, Vec3::new(0.3, 0.5, 0.8).normalize(), Vec3::X, Vec2::splat(0.1), 0.1);
        let a0 = projector_angle(&q);
        let turned = rotate_placement_to(&Placement::Projected(q), 33.0);
        assert!((placement_rotation_deg(&turned) - 33.0).abs() < 1e-2);
        let back = rotate_placement_to(&turned, a0.to_degrees());
        let Placement::Projected(b) = back else { unreachable!() };
        assert!(b.rotation.angle_between(q.rotation) < 1e-3);
    }

    #[test]
    fn size_percent_and_scaling() {
        let p = Placement::Projected(projector());
        assert!((placement_size_pct(&p, 2.0, 1.0) - 20.0).abs() < 1e-4); // larger side 0.4 of diagonal 2
        let s = scale_placement(&p, 1.5);
        assert!((placement_size_pct(&s, 2.0, 1.0) - 30.0).abs() < 1e-4);
        let uv = Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(100.0, 100.0),
            size_px: Vec2::new(64.0, 32.0),
            rotation: 0.5,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        assert!((placement_size_pct(&uv, 9.0, 256.0) - 25.0).abs() < 1e-4);
        assert!((placement_rotation_deg(&uv) - 0.5f32.to_degrees()).abs() < 1e-4);
        let Placement::Uv(r) = rotate_placement_to(&uv, -90.0) else { unreachable!() };
        assert!((r.rotation + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(r.center_px, Vec2::new(100.0, 100.0));
    }

    /// Fix round 2, item 2: the Size / Rotation fields re-fit an automatic depth to the new footprint, keep a hand-set
    /// one, and leave UV placements and unchanged values alone.
    #[test]
    fn size_and_rotation_fields_refit_an_automatic_depth() {
        let state = sample_state(SampleKind::Sphere);
        let doc = state.doc().unwrap();
        let r = doc.mesh.bounds.diagonal() * 0.29;
        let c = doc.mesh.bounds.center();
        let p = Projector::from_surface(c + Vec3::Z * r, Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1) * r, 0.0);
        let auto = Placement::Projected(doc.fit_stamp_depth(&p));
        assert_eq!(refit_auto_depth(Some(doc), &auto, auto), auto, "nothing changed");
        let bigger = scale_placement(&auto, 3.0);
        let Placement::Projected(b) = refit_auto_depth(Some(doc), &auto, bigger) else { unreachable!() };
        assert!(doc.stamp_depth_is_auto(&b));
        assert_eq!(b, doc.fit_stamp_depth(&b));
        let Placement::Projected(plain) = bigger else { unreachable!() };
        assert!(b.depth_back > plain.depth_back, "the 3x footprint reaches deeper into the sphere: {b:?}");
        let turned = rotate_placement_to(&auto, 90.0);
        let Placement::Projected(t) = refit_auto_depth(Some(doc), &auto, turned) else { unreachable!() };
        assert_eq!(t, doc.fit_stamp_depth(&t));

        let Placement::Projected(a) = auto else { unreachable!() };
        let typed = Placement::Projected(Projector { depth_back: a.depth_back * 5.0, ..a });
        let typed_bigger = scale_placement(&typed, 3.0);
        assert_eq!(refit_auto_depth(Some(doc), &typed, typed_bigger), typed_bigger, "a hand-set depth is kept");
        assert_eq!(refit_auto_depth(None, &auto, bigger), bigger);
    }

    #[test]
    fn text_tool_focus_request_is_served_and_cleared() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        state.tool = ToolKind::Text;
        state.focus_text_content = true;
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        };
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(!state.focus_text_content);
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(ctx.text_edit_focused(), "the text content field has the keyboard focus");
        // With another tool a stale request is dropped.
        state.tool = ToolKind::Brush;
        state.focus_text_content = true;
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(!state.focus_text_content);
    }

    fn screen_1280() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        }
    }

    /// Sphere with a selected image stamp of `placement` (Transform tool).
    fn with_selected_stamp(placement: impl FnOnce(&Document) -> Placement) -> (AppState, StampId) {
        let mut state = sample_state(SampleKind::Sphere);
        let img = wte_core::samples::sample_stamp_image(32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("ring.png", std::sync::Arc::from(png)).unwrap();
        let p = placement(doc);
        let id = doc.add_stamp("ring", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        (state, id)
    }

    #[test]
    fn out_of_range_placements_are_shown_without_being_rewritten() {
        // A UV stamp turned past 180 degrees (the knob and Shift+wheel do that in the 2D view).
        let (mut state, _) = with_selected_stamp(|doc| {
            let w = doc.sets[0].width as f32;
            Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::splat(w * 0.5),
                size_px: Vec2::splat(w * 0.1),
                rotation: 190f32.to_radians(),
                flip_x: false,
                flip_y: false,
                clip_to_islands: false,
            })
        });
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen_1280());
        }
        assert!(state.actions.is_empty(), "display alone edits nothing: {:?}", state.actions);

        // A project-through decal scaled up: depth far beyond 200 % of the model size, size beyond 500 %.
        let (mut state, _) = with_selected_stamp(|doc| {
            let d = doc.mesh.bounds.diagonal();
            let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
            let mut p = Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::splat(d * 3.0), d * 3.0);
            p.depth_back = d * 12.0;
            Placement::Projected(p)
        });
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen_1280());
        }
        assert!(state.actions.is_empty(), "display alone edits nothing: {:?}", state.actions);
    }

    #[test]
    fn wheel_sizes_beyond_the_slider_range_survive_a_redraw() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        state.tool = ToolKind::Stamp;
        state.tools.stamp.size_frac = 1.1; // one Ctrl+wheel notch past 100 %
        run_panels(&ctx, &mut state, &mut ps, screen_1280());
        assert_eq!(state.tools.stamp.size_frac, 1.1);
        state.tools.stamp.size_frac = 0.004;
        run_panels(&ctx, &mut state, &mut ps, screen_1280());
        assert_eq!(state.tools.stamp.size_frac, 0.004);
        state.tool = ToolKind::Text;
        state.tools.text.em_frac = 0.8;
        run_panels(&ctx, &mut state, &mut ps, screen_1280());
        assert_eq!(state.tools.text.em_frac, 0.8);
    }

    #[test]
    fn a_locked_stamp_renders_quietly() {
        let (mut state, id) = with_selected_stamp(|doc| {
            let d = doc.mesh.bounds.diagonal();
            let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
            Placement::Projected(Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::splat(d * 0.1), d * 0.1))
        });
        let layer = state.doc().unwrap().stamp_layers(id)[0].1;
        state.doc_mut().unwrap().set_layer_locked(layer, true).unwrap();
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen_1280());
        }
        assert!(state.actions.is_empty(), "{:?}", state.actions);
    }

    #[test]
    fn selected_stamp_properties_render_and_stay_quiet() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let doc = state.doc_mut().unwrap();
        let font = doc.default_font();
        let spec = TextSpec {
            content: "WTE".into(),
            font,
            size_px: wte_core::TEXT_PLACEMENT_PX,
            color: [0, 0, 0, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: Some(Outline { width_em: 0.05, color: [255; 4] }),
            fx: wte_core::TextFx::default(),
        };
        let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
        let p = Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1), 0.2);
        let id = doc.add_stamp("WTE", StampSource::Text(spec), Placement::Projected(p), TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        };
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, input());
        }
        // (round 5 R5-3: the font button asks for the samples once per catalog.)
        state.actions.retain(|a| !matches!(a, AppAction::LoadFontSamples));
        assert!(state.actions.is_empty(), "{:?}", state.actions);
        // Another tool offers the jump back to Transform.
        state.tool = ToolKind::Brush;
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(state.actions.is_empty(), "{:?}", state.actions);
    }
}
