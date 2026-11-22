// Owner: B4 brush panels (round 4).
//
// The Brush tab of the lower side-panel area (FEATURE_BRUSH 1.3, 1.4, 6.2, 6.3; ROUND4_PLAN R4-6 and section 7): the
// first line naming the tool and the layer it paints (with Reset for the Sphere type), Type, Shape with "Load..." and
// "Remove file shape", the preview of the shape that can be dragged to turn it, Turn / Tilt / Roll, Offset, Align and
// Use, the closing-openings control "Openings" of a loaded shape file (plan section 7 item 5) and the two note lines.
// There is no "Stay on connected surface" checkbox (plan section 7 item 1).
//
// The tab edits the tip of the Eraser while the Eraser is the active tool, otherwise the tip of the Brush. Settings are
// written to `state.tools` directly, as every tool setting; "Load...", the entry of a remembered shape file that is not
// loaded, "Remove file shape" and a new "Openings" choice are actions. The container, the tab strip and the tab rule
// are in `panels/mod.rs`; this function draws into any `egui::Ui` of the area.

use std::sync::Arc;

use eframe::egui;
use egui::{DragValue, Pos2, Rect, Sense, Stroke, vec2};
use glam::{Quat, Vec3};
use wte_core::{CloseMode, PaintTarget, TipAlign, TipField};

use crate::action::AppAction;
use crate::panels::{PanelState, percent_slider};
use crate::state::{AppState, BrushSettings, BrushType, ShapeChoice, ToolKind};

/// Width of the label column (as in Tool options, FEATURE_BRUSH 6.3).
const LABEL_WIDTH: f32 = 72.0;
/// Width of the labels next to the preview (Turn, Tilt, Roll).
const ANGLE_LABEL_WIDTH: f32 = 40.0;
/// Width of the Turn / Tilt / Roll fields.
const ANGLE_FIELD_WIDTH: f32 = 72.0;
/// Side of the preview picture.
pub(crate) const PREVIEW_SIDE: f32 = 72.0;
/// Degrees of turn per point of a drag in the preview (FEATURE_BRUSH 1.4).
pub(crate) const PREVIEW_DEG_PER_POINT: f32 = 0.5;
/// Width of the Shape list and of the Openings list.
const SHAPE_COMBO_WIDTH: f32 = 104.0;
/// Width of the Align list and of the Use list.
const ALIGN_COMBO_WIDTH: f32 = 104.0;
const USE_COMBO_WIDTH: f32 = 76.0;
/// Height of one row of the tab.
const ROW_HEIGHT: f32 = 20.0;
/// Vertical space between two rows of the tab.
const ROW_SPACING: f32 = 2.0;
/// The fixed three-quarter view of the preview: turn about the vertical, then tilt towards the viewer (radians).
const PREVIEW_YAW: f32 = -0.6;
const PREVIEW_PITCH: f32 = 0.45;

/// Hover texts of the two Type buttons (FEATURE_BRUSH 1.4).
pub(crate) const CIRCULAR_HOVER: &str =
    "Circular: the round brush on the surface you point at. It does not paint the far side of thin parts.";
pub(crate) const SPHERE_HOVER: &str = "Sphere: a ball that glides over the model. Everything inside it is painted, also the far side and hidden parts. Differs from Circular where the ball reaches a far side, a fold or another part.";
/// The second note line, always shown.
pub(crate) const NOTE_2D: &str = "The 2D view always paints a flat disc.";

/// UI state of the Brush tab that lives across frames.
#[derive(Default)]
pub(crate) struct BrushTabState {
    /// The tab's content as laid out in the last frame it was drawn (unclipped, screen points; it reaches below the
    /// visible area when the tab scrolls). For the layout tests (A12).
    pub content: Option<Rect>,
    /// The preview picture's rect in the last frame (`None` when no preview was drawn).
    pub preview: Option<Rect>,
}

/// Draws the Brush tab (settings of the Brush or the Eraser, whichever is active). Settings change `state` directly;
/// commands are pushed to `state.actions`.
pub fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    let eraser = state.tool == ToolKind::Eraser;
    let old = if eraser { state.tools.eraser.clone() } else { state.tools.brush.clone() };
    let mut b = old.clone();
    let mut actions = Vec::new();
    ps.brush.preview = None;
    // Tighter rows than the default (4 pt between rows): the tallest state (a shape file with closed openings, nine
    // rows) then fits the computed lower area at 1280 x 800 with the real window's fonts too (FEATURE_BRUSH 6.1).
    ui.spacing_mut().item_spacing.y = ROW_SPACING;

    let sphere = b.kind == BrushType::Sphere;
    let field = if sphere { shape_field(state, b.shape) } else { None };
    let flat = field.as_ref().is_some_and(|f| f.is_flat());
    let shaped = sphere && b.shape != ShapeChoice::Ball;

    // First line: the tool and the layer it paints; Reset for the Sphere type.
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        let text = format!("{} tip - paints on: {}", if eraser { "Eraser" } else { "Brush" }, paint_layer_name(state));
        let reset_room = if sphere { 60.0 } else { 0.0 };
        let width = (ui.available_width() - reset_room).max(40.0);
        ui.allocate_ui_with_layout(vec2(width, ROW_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(text).truncate());
        });
        if sphere
            && ui
                .button("Reset")
                .on_hover_text("Reset: sets Turn, Tilt, Roll and Offset to 0 and Align to Follow surface")
                .clicked()
        {
            reset_pose(&mut b);
        }
    });

    // Type.
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        row_label(ui, "Type", LABEL_WIDTH);
        if ui.add(egui::Button::selectable(b.kind == BrushType::Circular, "Circular")).on_hover_text(CIRCULAR_HOVER).clicked() {
            b.kind = BrushType::Circular;
        }
        if ui.add(egui::Button::selectable(b.kind == BrushType::Sphere, "Sphere")).on_hover_text(SPHERE_HOVER).clicked() {
            b.kind = BrushType::Sphere;
        }
    });

    if sphere {
        shape_row(ui, state, &mut b, &mut actions);
        if shaped {
            preview_block(ui, ps, field.as_deref(), &mut b);
        }
        ui.horizontal(|ui| {
            ui.set_min_height(ROW_HEIGHT);
            row_label(ui, "Offset", LABEL_WIDTH);
            ui.add(percent_slider(&mut b.offset, -100.0, 100.0).max_decimals(0)).on_hover_text(
                "Offset: moves the tip along the surface normal towards the viewer, in % of its radius (0 = centre on the surface)",
            );
        });
        if shaped {
            align_use_row(ui, &mut b, flat);
        }
        if b.shape == ShapeChoice::File {
            openings_row(ui, state, &mut actions);
        }
    }

    // Note lines (each at most 44 characters, FEATURE_BRUSH 6.3).
    ui.label(egui::RichText::new(note_line(&b, flat)).weak());
    ui.label(egui::RichText::new(NOTE_2D).weak());

    ps.brush.content = Some(ui.min_rect());
    if b != old {
        let b = b.sanitized();
        if eraser {
            state.tools.eraser = b;
        } else {
            state.tools.brush = b;
        }
    }
    state.actions.extend(actions);
}

/// The first note line by state (FEATURE_BRUSH 6.3).
pub(crate) fn note_line(b: &BrushSettings, flat: bool) -> &'static str {
    match (b.kind, b.shape) {
        (BrushType::Circular, _) => "Paints the surface you point at.",
        (BrushType::Sphere, ShapeChoice::Ball) => "Also paints far sides and hidden parts.",
        _ if b.through || flat => "Presses the outline straight through.",
        _ => "Paints everything inside the shape.",
    }
}

/// Reset: Turn, Tilt, Roll and Offset to 0, Align to Follow surface.
fn reset_pose(b: &mut BrushSettings) {
    b.turn_deg = 0.0;
    b.tilt_deg = 0.0;
    b.roll_deg = 0.0;
    b.offset = 0.0;
    b.align = TipAlign::Surface;
}

/// The name of the layer a stroke paints in the active set: "Paint 1", "Paint 1 (locked)", or "no paint layer" (the
/// brush makes one at the first stroke).
fn paint_layer_name(state: &AppState) -> String {
    let Some(session) = state.session.as_ref() else { return "no paint layer".to_owned() };
    let doc = &session.doc;
    let name = |id| doc.layer(id).map_or_else(|| "no paint layer".to_owned(), |l| l.name.clone());
    match doc.paint_target(session.active_set) {
        PaintTarget::Ready(id) => name(id),
        PaintTarget::Locked(id) => format!("{} (locked)", name(id)),
        PaintTarget::Hidden(id) => format!("{} (hidden)", name(id)),
        PaintTarget::NoActiveLayer | PaintTarget::NotRaster(_) => "no paint layer".to_owned(),
    }
}

/// The field of a shape choice: the built-in Cube or Cylinder (made on first use and kept in
/// `AppState::builtin_tips`, as the brush tool does), the loaded file shape, or `None` (Ball, nothing loaded).
fn shape_field(state: &mut AppState, shape: ShapeChoice) -> Option<Arc<TipField>> {
    match shape {
        ShapeChoice::Ball => None,
        ShapeChoice::Cube => Some(state.builtin_tips[0].get_or_insert_with(|| Arc::new(TipField::cube())).clone()),
        ShapeChoice::Cylinder => Some(state.builtin_tips[1].get_or_insert_with(|| Arc::new(TipField::cylinder())).clone()),
        ShapeChoice::File => state.brush_shape.current.as_ref().map(|s| s.field.clone()),
    }
}

/// A label of a fixed width at the start of a row (found by the control tests as the row's label).
fn row_label(ui: &mut egui::Ui, text: &str, width: f32) {
    // The whole width is taken (set_min_width), so the controls of all rows start at the same x.
    ui.allocate_ui_with_layout(vec2(width, ROW_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        ui.add(egui::Label::new(text).truncate());
    });
}

/// The text of the file entry of the Shape list, whether it can be chosen, and its hover text.
fn file_entry(state: &AppState) -> (String, bool, String) {
    let remembered = state.prefs.brush_shape_path.as_ref();
    let shape = &state.brush_shape;
    let file_name = |p: &std::path::Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    if let Some(p) = shape.pending.as_ref() {
        return (format!("Loading... {}", file_name(&p.path)), true, p.path.display().to_string());
    }
    if let Some(cur) = shape.current.as_ref() {
        let report = cur.field.report();
        let mut hover = format!("{}\n{} triangles", cur.path.display(), report.triangles);
        match report.up_axis {
            0 => hover.push_str("\nUp axis: X"),
            2 => hover.push_str("\nUp axis: Z"),
            _ => {}
        }
        return (cur.name.clone(), true, hover);
    }
    match remembered {
        Some(p) => (file_name(p), true, format!("{}\nNot loaded yet: choosing it loads it", p.display())),
        None => ("File (none loaded)".to_owned(), false, "No shape file is loaded: use Load...".to_owned()),
    }
}

/// Shape list, "Load..." and the "!" marker.
fn shape_row(ui: &mut egui::Ui, state: &mut AppState, b: &mut BrushSettings, actions: &mut Vec<AppAction>) {
    let (file_text, file_enabled, file_hover) = file_entry(state);
    let remembered = state.prefs.brush_shape_path.clone();
    let loaded = state.brush_shape.current.as_ref().is_some_and(|c| Some(&c.path) == remembered.as_ref());
    let pending = state.brush_shape.pending.is_some();
    let selected = match b.shape {
        ShapeChoice::Ball => "Ball".to_owned(),
        ShapeChoice::Cube => "Cube".to_owned(),
        ShapeChoice::Cylinder => "Cylinder".to_owned(),
        ShapeChoice::File => shorten(&file_text, SHAPE_TEXT_CHARS),
    };
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        row_label(ui, "Shape", LABEL_WIDTH);
        egui::ComboBox::from_id_salt("brush_tab_shape")
            .width(SHAPE_COMBO_WIDTH)
            .truncate()
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (choice, text) in [(ShapeChoice::Ball, "Ball"), (ShapeChoice::Cube, "Cube"), (ShapeChoice::Cylinder, "Cylinder")] {
                    if ui.selectable_label(b.shape == choice, text).clicked() {
                        b.shape = choice;
                    }
                }
                let file = ui
                    .add_enabled(file_enabled, egui::Button::selectable(b.shape == ShapeChoice::File, file_text.as_str()))
                    .on_hover_text(file_hover.as_str());
                if file.clicked() {
                    b.shape = ShapeChoice::File;
                    if !loaded && !pending {
                        actions.push(AppAction::ReloadBrushShape);
                    }
                }
                ui.separator();
                let remove = ui
                    .add_enabled(remembered.is_some(), egui::Button::selectable(false, "Remove file shape"))
                    .on_hover_text("Forgets the shape file; tools that use it go back to the Ball");
                if remove.clicked() {
                    actions.push(AppAction::RemoveBrushShape);
                }
            })
            .response
            .on_hover_text("Shape: the 3D object that glides over the model");
        if ui
            .button("Load...")
            .on_hover_text("Load a brush shape from an OBJ, glTF, GLB or FBX file")
            .clicked()
        {
            actions.push(AppAction::LoadBrushShape(None));
        }
        if let Some(why) = shape_warning(state) {
            ui.colored_label(ui.visuals().warn_fg_color, "!").on_hover_text(why);
        }
    });
}

/// Characters of a file name the closed Shape list shows (a combo box grows with its text; 104 pt hold about 16).
const SHAPE_TEXT_CHARS: usize = 15;

/// `text` cut to `max` characters with "..." at the end when it is longer.
pub(crate) fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut s: String = text.chars().take(max.saturating_sub(3)).collect();
        s.push_str("...");
        s
    }
}

/// The text behind the "!" after the Shape list: the last failed load, or a note of the loaded shape's report.
fn shape_warning(state: &AppState) -> Option<String> {
    if let Some((_, message)) = state.brush_shape.last_error.as_ref() {
        return Some(message.clone());
    }
    let report = state.brush_shape.current.as_ref()?.field.report();
    if report.partial {
        Some("Only the closed parts of this file are used".to_owned())
    } else if report.thin {
        Some("Parts of this shape are thinner than the brush can show".to_owned())
    } else {
        None
    }
}

/// The preview picture (dragging turns the shape) and the Turn / Tilt / Roll fields next to it.
fn preview_block(ui: &mut egui::Ui, ps: &mut PanelState, field: Option<&TipField>, b: &mut BrushSettings) {
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(PREVIEW_SIDE, PREVIEW_SIDE), Sense::drag());
        let resp = resp.on_hover_text("Shape preview: drag left/right to change Turn, up/down to change Tilt");
        if resp.dragged() {
            let d = resp.drag_delta();
            if d != egui::Vec2::ZERO {
                b.turn_deg = wrap_deg(b.turn_deg + d.x * PREVIEW_DEG_PER_POINT);
                b.tilt_deg = wrap_deg(b.tilt_deg + d.y * PREVIEW_DEG_PER_POINT);
            }
        }
        ps.brush.preview = Some(rect);
        if ui.is_rect_visible(rect) {
            paint_preview(ui, rect, field, b);
        }
        ui.add_space(8.0 - ui.spacing().item_spacing.x);
        ui.vertical(|ui| {
            let fields: [(&str, &str, &mut f32); 3] = [
                ("Turn", "Turn: spins the shape about the axis out of the surface (also Shift+wheel)", &mut b.turn_deg),
                ("Tilt", "Tilt: leans the shape forward or back", &mut b.tilt_deg),
                ("Roll", "Roll: leans the shape sideways", &mut b.roll_deg),
            ];
            for (label, hover, value) in fields {
                ui.horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    row_label(ui, label, ANGLE_LABEL_WIDTH);
                    ui.add_sized(
                        vec2(ANGLE_FIELD_WIDTH, ROW_HEIGHT),
                        DragValue::new(value).range(-180.0..=180.0).speed(1.0).max_decimals(1).suffix(" deg"),
                    )
                    .on_hover_text(hover);
                });
            }
        });
    });
}

/// An angle wrapped into `-180..=180` degrees.
pub(crate) fn wrap_deg(v: f32) -> f32 {
    if (-180.0..=180.0).contains(&v) { v } else { (v + 180.0).rem_euclid(360.0) - 180.0 }
}

/// The rotation of the shape in its tip frame from the settings: `Ry(-turn) Rx(tilt) Rz(roll)` (FEATURE_BRUSH 4.3, as
/// the brush tool turns it).
fn tip_rotation(b: &BrushSettings) -> Quat {
    Quat::from_rotation_y(-b.turn_deg.to_radians()) * Quat::from_rotation_x(b.tilt_deg.to_radians()) * Quat::from_rotation_z(b.roll_deg.to_radians())
}

/// Draws the preview: the three principal sections of the shape, turned by the angles and moved by the offset, in a
/// fixed three-quarter view, with the surface (the plane under the tip centre) as a thin ellipse; what lies below the
/// surface is drawn dimmer. Without a field (a file shape that is not loaded) only the frame and the surface.
fn paint_preview(ui: &egui::Ui, rect: Rect, field: Option<&TipField>, b: &BrushSettings) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    painter.rect_stroke(rect, 2.0, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let view = Quat::from_rotation_x(PREVIEW_PITCH) * Quat::from_rotation_y(PREVIEW_YAW);
    let scale = rect.width() * 0.42;
    let centre = rect.center();
    let project = |p: Vec3| -> Pos2 {
        let v = view * p;
        Pos2::new(centre.x + v.x * scale, centre.y - v.y * scale)
    };
    // The surface: the plane y = -offset of the tip frame (the tip centre sits `offset` radii above it).
    let surface_y = -b.offset;
    let surface = Stroke::new(1.0, visuals.weak_text_color());
    let ring: Vec<Pos2> = (0..=32)
        .map(|i| {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            project(Vec3::new(a.cos(), surface_y, a.sin()))
        })
        .collect();
    painter.add(egui::Shape::line(ring, surface));
    let Some(field) = field else {
        painter.text(centre, egui::Align2::CENTER_CENTER, "no shape", egui::FontId::proportional(10.0), visuals.weak_text_color());
        return;
    };
    let rot = tip_rotation(b);
    let bright = Stroke::new(1.2, visuals.strong_text_color());
    let dim = Stroke::new(1.0, visuals.strong_text_color().gamma_multiply(0.35));
    let [sx, sy, sz] = field.principal_sections();
    let lift = |p: Vec3| rot * p;
    let draw =|a: Vec3, c: Vec3| {
        let (a, c) = (lift(a), lift(c));
        let below = (a.y + c.y) * 0.5 < surface_y;
        painter.line_segment([project(a), project(c)], if below { dim } else { bright });
    };
    for s in sx {
        draw(Vec3::new(0.0, s[0].x, s[0].y), Vec3::new(0.0, s[1].x, s[1].y));
    }
    for s in sy {
        draw(Vec3::new(s[0].x, 0.0, s[0].y), Vec3::new(s[1].x, 0.0, s[1].y));
    }
    for s in sz {
        draw(Vec3::new(s[0].x, s[0].y, 0.0), Vec3::new(s[1].x, s[1].y, 0.0));
    }
}

/// Align ("Follow surface" / "Fixed in space") and Use ("Solid" / "Through"; fixed on Through for a flat outline).
fn align_use_row(ui: &mut egui::Ui, b: &mut BrushSettings, flat: bool) {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        row_label(ui, "Align", LABEL_WIDTH);
        let align_text = |a: TipAlign| if a == TipAlign::World { "Fixed in space" } else { "Follow surface" };
        egui::ComboBox::from_id_salt("brush_tab_align")
            .width(ALIGN_COMBO_WIDTH)
            .truncate()
            .selected_text(align_text(b.align))
            .show_ui(ui, |ui| {
                for a in [TipAlign::Surface, TipAlign::World] {
                    if ui.selectable_label(b.align == a, align_text(a)).clicked() {
                        b.align = a;
                    }
                }
            })
            .response
            .on_hover_text("Align: Follow surface stands the shape on the surface; Fixed in space keeps the axes of its file");
        let through = b.through || flat;
        ui.add_enabled_ui(!flat, |ui| {
            egui::ComboBox::from_id_salt("brush_tab_use")
                .width(USE_COMBO_WIDTH)
                .truncate()
                .selected_text(if through { "Through" } else { "Solid" })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(!through, "Solid")
                        .on_hover_text("Solid: the shape is a volume; paint lands where the surface passes through it.")
                        .clicked()
                    {
                        b.through = false;
                    }
                    if ui
                        .selectable_label(through, "Through")
                        .on_hover_text("Through: the shape's outline, seen from above, is pressed straight through the surface inside the ring. Use it for flat or thin shapes.")
                        .clicked()
                    {
                        b.through = true;
                    }
                })
                .response
                .on_hover_text("Use: Solid paints where the surface passes through the shape; Through presses its outline through")
                .on_disabled_hover_text("Use: a flat outline is always pressed Through");
        });
    });
}

/// "Openings: Flat caps": how the openings of the loaded shape file were closed, and the choice to close them another
/// way (plan section 7 item 5). Shown when the file had openings; "Use only the closed parts" only when it also has
/// closed parts. A new choice bakes the file again (`SetBrushShapeClose`), which also stores it with the file.
fn openings_row(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<AppAction>) {
    let Some(cur) = state.brush_shape.current.as_ref() else { return };
    if cur.close == CloseMode::Refuse {
        return;
    }
    let busy = state.brush_shape.pending.is_some();
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        row_label(ui, "Openings", LABEL_WIDTH);
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("brush_tab_openings")
                .width(SHAPE_COMBO_WIDTH)
                .truncate()
                .selected_text(cur.close.label())
                .show_ui(ui, |ui| {
                    for mode in CloseMode::CHOICES {
                        if mode == CloseMode::ClosedPartsOnly && !cur.closed_parts {
                            continue;
                        }
                        let item = ui.selectable_label(cur.close == mode, mode.label());
                        let item = if mode == CloseMode::FillHollows {
                            item.on_hover_text("Closes the solid from outside; this also fills deep hollows")
                        } else {
                            item
                        };
                        if item.clicked() && mode != cur.close {
                            actions.push(AppAction::SetBrushShapeClose(mode));
                        }
                    }
                })
                .response
                .on_hover_text(format!("Openings: how the openings of {} are closed; another choice loads it again", cur.name))
                .on_disabled_hover_text("Openings: the shape is loading");
        });
    });
}
