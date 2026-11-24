//! The 3D editing view (ARCHITECTURE.md 8.5): the model with the live composite, orbit camera, painting and stamping
//! on the surface, brush ring, wireframe / seams and the gizmo overlays.

use eframe::egui;

use crate::gpu::View3dParams;
use crate::state::ViewId;
use crate::tools::ViewXform;
use crate::view::{ViewCx, route_pointer, selected_outline, sync_scene, target_px};

/// The 3D editing view in `rect`: input routing, hover raycast, rendering and overlays.
pub fn show_3d(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect) {
    input(ui, cx, rect);
    if sync_scene(cx, false) {
        render(ui, cx, rect);
    }
}

/// Phase A of the 3D view: layout and pointer routing.
pub(crate) fn input(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    cx.state.view_rects.insert(ViewId::Main3D, rect);
    let response = ui.allocate_rect(rect, egui::Sense::DRAG);
    route_pointer(ui, cx, ViewId::Main3D, rect, &response);
}

/// objects-tabs (8.5): the selection tint is drawn when objects are selected, no stroke is open and the Navigate tool is
/// active (a tint would falsify the colours while painting). The plan's "or the Objects tab is shown" joins with the
/// side panel tabs (WP4).
pub(crate) fn highlight_shown(state: &crate::state::AppState) -> bool {
    state.tool == crate::state::ToolKind::Navigate
        && state.session.as_ref().is_some_and(|s| !s.selected_objects.is_empty() && !s.doc.stroke_active())
}

/// Phase B of the 3D view: render the model with the brush ring, paint it, then the overlays.
pub(crate) fn render(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let Some(session) = cx.state.session.as_ref() else { return };
    let Some(scene) = cx.scene.as_ref() else { return };
    let xf = ViewXform::of_session(session, ViewId::Main3D, rect);
    let flags = &cx.state.flags;
    let params = View3dParams {
        view_proj: xf.camera.view_proj(rect.aspect_ratio(), xf.scene_radius),
        eye: xf.camera.eye(),
        shading: flags.shading,
        lit: flags.lit,
        alpha_checker: flags.alpha_checker,
        matte: None,
        wireframe: flags.wireframe,
        seams: flags.seams,
        nearest: false,
        backdrop: flags.backdrop,
        brush: cx.tools.brush_ring(cx.state),
        all_objects: false,
        highlight: highlight_shown(cx.state),
    };
    let target = &mut cx.targets.main3d;
    target.ensure_size(cx.gpu, target_px(ui.ctx(), rect));
    let p = crate::app::frame_profile::scope("render 3d");
    cx.renderer.render_3d(cx.gpu, scene, target, &params);
    drop(p);
    let painter = ui.painter_at(rect);
    painter.image(target.egui_id, rect, target.uv_rect(), egui::Color32::WHITE);
    let _p = crate::app::frame_profile::scope("overlays");
    selected_outline(cx.state, &xf, &painter);
    cx.tools.overlay(cx.state, &xf, &painter);
}
