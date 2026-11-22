//! The clean preview (ARCHITECTURE.md 8.5, R5): an orbit view of the composite without ring, gizmo or wireframe,
//! used by the Preview mode and embedded in the Export dialog.

use eframe::egui;

use crate::action::AppAction;
use crate::gpu::View3dParams;
use crate::state::{Backdrop, ShadingMode, ViewId, ViewMode};
use crate::tools::camera_of_mut;
use crate::view::{ViewCx, sync_scene, target_px, wheel_events};

/// Height of the option strip of the Preview mode (points).
const OPTION_STRIP_HEIGHT: f32 = 30.0;

/// Clean orbit view of the composite (no ring, gizmo, wireframe). `id` = `ViewId::Preview` or `ViewId::ExportPreview`.
/// Self-contained: allocates `rect`, handles its own orbit (LMB/RMB drag), pan (MMB) and wheel zoom, syncs the scene
/// with `force_mips = true`, renders and paints. It never looks at `state.dialog` or `state.busy`, so it stays
/// interactive inside the Export modal. `matte` is forwarded to `View3dParams.matte`.
pub fn preview_widget(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, id: ViewId, rect: egui::Rect, matte: Option<[u8; 3]>) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    cx.state.view_rects.insert(id, rect);
    let response = ui.allocate_rect(rect, egui::Sense::DRAG);

    // Navigation (egui's own layering keeps a covered widget from seeing the pointer).
    if let Some(session) = cx.state.session.as_mut() {
        let bounds = session.doc.mesh.bounds;
        let camera = camera_of_mut(session, id);
        let delta = response.drag_delta();
        if delta != egui::Vec2::ZERO {
            if response.dragged_by(egui::PointerButton::Middle) {
                camera.pan(delta, rect.height(), &bounds);
            } else if response.dragged_by(egui::PointerButton::Primary) || response.dragged_by(egui::PointerButton::Secondary) {
                camera.orbit(delta);
            }
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if response.contains_pointer() {
            let radius = bounds.diagonal() * 0.5;
            for (notches, _) in wheel_events(ui) {
                camera.zoom(notches, radius);
            }
        }
    }

    let target = cx.targets.get_mut(id);
    target.ensure_size(cx.gpu, target_px(ui.ctx(), rect));
    if sync_scene(cx, true)
        && let Some(session) = cx.state.session.as_ref()
        && let Some(scene) = cx.scene.as_ref()
    {
        let camera = crate::tools::camera_of(session, id);
        let radius = camera.scene_radius(&session.doc.mesh.bounds);
        let preview = &cx.state.preview;
        let params = View3dParams {
            view_proj: camera.view_proj(rect.aspect_ratio(), radius),
            eye: camera.eye(),
            shading: ShadingMode::Textured,
            lit: preview.lit,
            alpha_checker: true,
            matte,
            wireframe: false,
            seams: false,
            nearest: preview.nearest,
            backdrop: preview.backdrop,
            brush: None,
        };
        cx.renderer.render_3d(cx.gpu, scene, cx.targets.get_mut(id), &params);
    } else {
        cx.renderer.render_backdrop(cx.gpu, cx.targets.get_mut(id), cx.state.preview.backdrop);
    }
    let target = cx.targets.get_mut(id);
    ui.painter_at(rect).image(target.egui_id, rect, target.uv_rect(), egui::Color32::WHITE);
}

/// Preview mode of the central area: the preview widget plus its option strip.
pub fn show_preview_mode(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect) {
    let strip = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), OPTION_STRIP_HEIGHT));
    let view = egui::Rect::from_min_max(egui::pos2(rect.min.x, strip.max.y), rect.max);
    option_strip(ui, cx, strip);

    // Turntable: the preview camera turns while the option is on.
    if cx.state.preview.turntable {
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.1);
        let speed = cx.state.preview.turntable_speed;
        if let Some(session) = cx.state.session.as_mut() {
            let camera = &mut session.preview_camera;
            camera.yaw = (camera.yaw + speed * dt).rem_euclid(std::f32::consts::TAU);
        }
        ui.ctx().request_repaint();
    }
    preview_widget(ui, cx, ViewId::Preview, view, None);
}

/// Lit/Unlit, Turntable + speed, Background, Linear/Nearest, Export..., Back (Esc).
fn option_strip(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, strip: egui::Rect) {
    ui.painter().rect_filled(strip, 0.0, ui.visuals().faint_bg_color);
    let builder = egui::UiBuilder::new()
        .max_rect(strip.shrink2(egui::vec2(8.0, 0.0)))
        .layout(egui::Layout::left_to_right(egui::Align::Center));
    let state = &mut *cx.state;
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let preview = &mut state.preview;
        ui.selectable_value(&mut preview.lit, true, "Lit").on_hover_text("Shade the model with a light");
        ui.selectable_value(&mut preview.lit, false, "Unlit").on_hover_text("Show the texture colours exactly as exported");
        ui.separator();
        ui.checkbox(&mut preview.turntable, "Turntable").on_hover_text("Turn the model continuously");
        ui.add_enabled(
            preview.turntable,
            egui::Slider::new(&mut preview.turntable_speed, -3.0..=3.0).fixed_decimals(1).suffix(" rad/s"),
        );
        ui.separator();
        ui.label("Background");
        egui::ComboBox::from_id_salt("preview_backdrop")
            .selected_text(backdrop_name(preview.backdrop))
            .show_ui(ui, |ui| {
                for b in [Backdrop::Gradient, Backdrop::Dark, Backdrop::Light, Backdrop::Checker] {
                    ui.selectable_value(&mut preview.backdrop, b, backdrop_name(b));
                }
            });
        ui.separator();
        ui.selectable_value(&mut preview.nearest, false, "Linear").on_hover_text("Smooth texture filtering");
        ui.selectable_value(&mut preview.nearest, true, "Nearest").on_hover_text("Show individual texels");
        ui.separator();
        let export = crate::shortcuts::shortcut_text(ui.ctx(), "Export...");
        if ui.button("Export...").on_hover_text(format!("Export the textures ({export})")).clicked() {
            state.push(AppAction::ShowExport);
        }
        if ui.button("Back").on_hover_text("Back to editing (Esc)").clicked() {
            let back = if state.last_edit_mode == ViewMode::Preview { ViewMode::View3D } else { state.last_edit_mode };
            state.push(AppAction::SetViewMode(back));
        }
    });
}

/// Display name of a backdrop.
fn backdrop_name(b: Backdrop) -> &'static str {
    match b {
        Backdrop::Dark => "Dark",
        Backdrop::Light => "Light",
        Backdrop::Gradient => "Gradient",
        Backdrop::Checker => "Checker",
    }
}
