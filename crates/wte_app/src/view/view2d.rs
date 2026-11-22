//! The 2D (UV) view (ARCHITECTURE.md 8.5): the active set's texture laid flat, pan / zoom, painting and stamping in
//! texture space, the UV wireframe and the overlays.

use eframe::egui;
use glam::Vec2;
use wte_core::TextureSetId;

use crate::action::AppAction;
use crate::gpu::ViewUvParams;
use crate::state::{UvViewState, ViewId};
use crate::tools::{ViewXform, set_size};
use crate::view::{ViewCx, route_pointer, selected_outline, sync_scene, target_px};

/// Zoom limits (points per texel).
const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 64.0;
/// Share of the view the texture fills after `fit`.
const FIT_MARGIN: f32 = 0.92;
/// Height of the set-tab strip (points).
const SET_STRIP_HEIGHT: f32 = 24.0;
/// Pan and zoom keep at least this much of the texture inside the 2D view on each axis (points; the whole texture
/// side when it is smaller on screen). R3-11.
pub(crate) const KEEP_VISIBLE: f32 = 32.0;

/// The texture size as a float vector, never zero.
fn tex_vec(tex: [u32; 2]) -> Vec2 {
    Vec2::new(tex[0].max(1) as f32, tex[1].max(1) as f32)
}

impl UvViewState {
    /// `rect.center() + (uv - center_uv) * tex * zoom`.
    pub fn uv_to_screen(&self, uv: Vec2, tex: [u32; 2], rect: egui::Rect) -> egui::Pos2 {
        let d = (uv - self.center_uv) * tex_vec(tex) * self.zoom;
        rect.center() + egui::vec2(d.x, d.y)
    }

    /// Inverse of `uv_to_screen`.
    pub fn screen_to_uv(&self, pos: egui::Pos2, tex: [u32; 2], rect: egui::Rect) -> Vec2 {
        let d = pos - rect.center();
        self.center_uv + Vec2::new(d.x, d.y) / (tex_vec(tex) * self.zoom.max(1e-9))
    }

    /// Multiplies the zoom by `factor` (clamped to `0.02..=64`) keeping the UV under `pos` fixed, then `clamp_to(rect)`
    /// (so a zoom about a point beside the texture can move the texture back into the view).
    pub fn zoom_about(&mut self, pos: egui::Pos2, factor: f32, tex: [u32; 2], rect: egui::Rect) {
        if !factor.is_finite() || factor <= 0.0 || !pos.is_finite() {
            return;
        }
        let anchor = self.screen_to_uv(pos, tex, rect);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let d = pos - rect.center();
        self.center_uv = anchor - Vec2::new(d.x, d.y) / (tex_vec(tex) * self.zoom);
        self.clamp_to(tex, rect);
    }

    /// Pans by a screen delta (the texture follows the pointer), then `clamp_to(rect)`.
    pub fn pan(&mut self, delta: egui::Vec2, tex: [u32; 2], rect: egui::Rect) {
        if !delta.is_finite() {
            return;
        }
        self.center_uv -= Vec2::new(delta.x, delta.y) / (tex_vec(tex) * self.zoom.max(1e-6));
        self.clamp_to(tex, rect);
    }

    /// Moves `center_uv` the least amount so that at least `KEEP_VISIBLE` points of the texture (or the whole texture /
    /// view side when that is smaller) stay inside `rect` on each axis. Per axis with view length `L` and texture length
    /// on screen `S = tex * zoom`, `k = min(32, S, L)`: `center ∈ [(k - L/2) / S, 1 + (L/2 - k) / S]`; the centred
    /// texture (`center = 0.5`) is always inside that range. Non-finite values are reset (centre 0.5, zoom 1).
    /// Called by pan, wheel zoom and every frame of the 2D view (window resize, set switch, a loaded project's view).
    pub fn clamp_to(&mut self, tex: [u32; 2], rect: egui::Rect) {
        if !self.zoom.is_finite() || self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if !self.center_uv.is_finite() {
            self.center_uv = Vec2::splat(0.5);
        }
        if !(rect.width() >= 1.0 && rect.height() >= 1.0) {
            return;
        }
        let size = tex_vec(tex) * self.zoom;
        let axis = |c: f32, s: f32, l: f32| {
            let k = KEEP_VISIBLE.min(s).min(l);
            c.clamp((k - l * 0.5) / s, 1.0 + (l * 0.5 - k) / s)
        };
        self.center_uv = Vec2::new(axis(self.center_uv.x, size.x, rect.width()), axis(self.center_uv.y, size.y, rect.height()));
    }

    /// Centres the texture and zooms so that it fits `rect`.
    pub fn fit(&mut self, tex: [u32; 2], rect: egui::Rect) {
        let t = tex_vec(tex);
        let zoom = (rect.width() / t.x).min(rect.height() / t.y) * FIT_MARGIN;
        self.center_uv = Vec2::splat(0.5);
        self.zoom = if zoom.is_finite() && zoom > 0.0 { zoom.clamp(MIN_ZOOM, MAX_ZOOM) } else { 1.0 };
    }
}

/// The 2D (UV) editing view in `rect`: pan/zoom, cursor -> UV, rendering and overlays.
pub fn show_2d(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect) {
    let image = input(ui, cx, rect);
    if sync_scene(cx, false) {
        render(ui, cx, image);
    }
}

/// Phase A of the 2D view: set tabs, layout, first-draw fit and pointer routing. Returns the image rect.
pub(crate) fn input(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect) -> egui::Rect {
    let mut image = rect;
    let Some(session) = cx.state.session.as_ref() else { return image };
    if session.doc.sets.len() > 1 {
        let strip = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), SET_STRIP_HEIGHT));
        image = egui::Rect::from_min_max(egui::pos2(rect.min.x, strip.max.y), rect.max);
        let tabs: Vec<(TextureSetId, String)> = session.doc.sets.iter().map(|s| (s.id, s.name.clone())).collect();
        let active = session.active_set;
        ui.painter().rect_filled(strip, 0.0, ui.visuals().faint_bg_color);
        let builder = egui::UiBuilder::new()
            .max_rect(strip.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center));
        ui.scope_builder(builder, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (id, name) in tabs {
                let response = ui.add(egui::Button::selectable(id == active, name)).on_hover_text("Texture set shown in the 2D view");
                if response.clicked() && id != active {
                    cx.state.push(AppAction::SetActiveSet(id));
                }
            }
        });
    }
    if image.width() < 1.0 || image.height() < 1.0 {
        return image;
    }
    cx.state.view_rects.insert(ViewId::Main2D, image);
    if let Some(session) = cx.state.session.as_mut()
        && session.uv_view == UvViewState::default()
    {
        // A fresh session fits the texture on its first draw (8.9).
        let tex = set_size(&session.doc, session.active_set);
        session.uv_view.fit(tex, image);
    }
    if let Some(session) = cx.state.session.as_mut() {
        // The texture stays reachable after a window resize, a Split divider move, a set switch or a loaded view (R3-11).
        let tex = set_size(&session.doc, session.active_set);
        session.uv_view.clamp_to(tex, image);
    }
    let response = ui.allocate_rect(image, egui::Sense::DRAG);
    route_pointer(ui, cx, ViewId::Main2D, image, &response);
    image
}

/// Phase B of the 2D view: render the texture and the UV wireframe, paint them, then the overlays.
pub(crate) fn render(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, image: egui::Rect) {
    if image.width() < 1.0 || image.height() < 1.0 {
        return;
    }
    let Some(session) = cx.state.session.as_ref() else { return };
    let Some(scene) = cx.scene.as_ref() else { return };
    let ppp = ui.ctx().pixels_per_point();
    let params = ViewUvParams {
        set: session.active_set,
        center_uv: session.uv_view.center_uv,
        px_per_texel: session.uv_view.zoom * ppp,
        uv_wire: cx.state.flags.uv_wire,
        alpha_checker: cx.state.flags.alpha_checker,
    };
    let target = &mut cx.targets.main2d;
    target.ensure_size(cx.gpu, target_px(ui.ctx(), image));
    let p = crate::app::frame_profile::scope("render 2d");
    cx.renderer.render_uv(cx.gpu, scene, target, &params);
    drop(p);
    let painter = ui.painter_at(image);
    painter.image(target.egui_id, image, target.uv_rect(), egui::Color32::WHITE);

    let xf = ViewXform::of_session(session, ViewId::Main2D, image);
    // The [0, 1] border of the texture.
    let border = egui::Rect::from_two_pos(xf.px_to_screen(Vec2::ZERO), xf.px_to_screen(tex_vec(xf.tex_size)));
    painter.rect_stroke(border, 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(150)), egui::StrokeKind::Outside);
    let _p = crate::app::frame_profile::scope("overlays");
    selected_outline(cx.state, &xf, &painter);
    cx.tools.overlay(cx.state, &xf, &painter);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(40.0, 70.0), egui::vec2(640.0, 480.0))
    }

    #[test]
    fn screen_to_uv_inverts_uv_to_screen() {
        let view = UvViewState { center_uv: Vec2::new(0.3, 0.7), zoom: 0.37 };
        for tex in [[1024u32, 1024u32], [512, 2048], [3, 5]] {
            for uv in [Vec2::ZERO, Vec2::ONE, Vec2::new(0.25, 0.8), Vec2::new(-1.5, 2.25)] {
                let back = view.screen_to_uv(view.uv_to_screen(uv, tex, rect()), tex, rect());
                assert!((back - uv).length() < 1e-4, "{uv:?} -> {back:?} at {tex:?}");
            }
        }
        // The view centre shows `center_uv`.
        assert!((view.screen_to_uv(rect().center(), [64, 64], rect()) - view.center_uv).length() < 1e-6);
    }

    #[test]
    fn zoom_about_keeps_the_point_fixed() {
        let tex = [1024, 512];
        let mut view = UvViewState { center_uv: Vec2::new(0.5, 0.5), zoom: 0.5 };
        // A point on the texture (R3-11: about a point beside it the clamp may move the texture, see below).
        let pos = egui::pos2(300.0, 400.0);
        let uv = view.screen_to_uv(pos, tex, rect());
        for factor in [1.5f32, 0.25, 10.0] {
            view.zoom_about(pos, factor, tex, rect());
            assert!((view.screen_to_uv(pos, tex, rect()) - uv).length() < 1e-5);
        }
        view.zoom_about(pos, 1e9, tex, rect());
        assert_eq!(view.zoom, MAX_ZOOM);
        assert!((view.screen_to_uv(pos, tex, rect()) - uv).length() < 1e-4);
        view.zoom_about(pos, 1e-12, tex, rect());
        assert_eq!(view.zoom, MIN_ZOOM);
    }

    /// The texture's screen rect and how much of it lies inside `r` on each axis.
    fn overlap(view: &UvViewState, tex: [u32; 2], r: egui::Rect) -> (egui::Rect, f32, f32) {
        let t = egui::Rect::from_two_pos(view.uv_to_screen(Vec2::ZERO, tex, r), view.uv_to_screen(Vec2::ONE, tex, r));
        let o = t.intersect(r);
        (t, o.width().max(0.0), o.height().max(0.0))
    }

    /// At least `KEEP_VISIBLE` points (or the whole smaller side) of the texture inside `r` on both axes.
    fn assert_reachable(view: &UvViewState, tex: [u32; 2], r: egui::Rect, what: &str) {
        let (t, w, h) = overlap(view, tex, r);
        let need_w = KEEP_VISIBLE.min(t.width()).min(r.width()) - 1e-2;
        let need_h = KEEP_VISIBLE.min(t.height()).min(r.height()) - 1e-2;
        assert!(w >= need_w && h >= need_h, "{what}: {w} x {h} of {t:?} inside {r:?} (need {need_w} x {need_h})");
    }

    #[test]
    fn pan_of_ten_view_widths_keeps_32_points_of_the_texture_in_the_view() {
        let r = rect();
        for tex in [[1024u32, 1024u32], [4096, 512], [3, 5]] {
            for zoom in [MIN_ZOOM, 0.13, 1.3, 20.0, MAX_ZOOM] {
                for dir in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(-1.0, -1.0), egui::vec2(0.7, -0.3)] {
                    let mut view = UvViewState { center_uv: Vec2::splat(0.5), zoom };
                    // Ten view widths in 100 motion events, as a drag delivers them.
                    for _ in 0..100 {
                        view.pan(dir * r.width() * 0.1, tex, r);
                    }
                    let what = format!("tex {tex:?} zoom {zoom} dir {dir:?}");
                    assert_reachable(&view, tex, r, &what);
                    // Panning back moves the texture again (the view is not stuck at the limit).
                    let before = view.center_uv;
                    view.pan(-dir * 10.0, tex, r);
                    assert_ne!(view.center_uv, before, "{what}");
                }
            }
        }
        // The clamp stops exactly at the limit: after ten view widths to the left (zoom 130 %, as in the audit's
        // BrainStem shot) the texture's right edge sits 32 points inside the view's left edge, its rows unchanged.
        let tex = [1024, 1024];
        let mut view = UvViewState { center_uv: Vec2::splat(0.5), zoom: 1.3 };
        view.pan(egui::vec2(-10.0 * r.width(), 0.0), tex, r);
        let (t, _, h) = overlap(&view, tex, r);
        assert!((t.max.x - (r.min.x + KEEP_VISIBLE)).abs() < 1e-2, "{t:?}");
        assert!((view.center_uv.y - 0.5).abs() < 1e-6 && h == r.height());
    }

    #[test]
    fn clamp_leaves_a_reachable_view_alone() {
        let r = rect();
        let tex = [1024, 512];
        let mut view = UvViewState::default();
        view.fit(tex, r);
        let fitted = view;
        view.clamp_to(tex, r);
        assert_eq!(view, fitted);
        // The texture partly out of the view, with more than 32 points still inside: unchanged.
        let mut view = UvViewState { center_uv: Vec2::new(0.9, 0.1), zoom: 2.0 };
        let before = view;
        view.clamp_to(tex, r);
        assert_eq!(view, before);
        // A small pan of a reachable view is the plain pan.
        let mut panned = before;
        panned.pan(egui::vec2(10.0, -4.0), tex, r);
        let expect = before.center_uv - Vec2::new(10.0 / (1024.0 * 2.0), -4.0 / (512.0 * 2.0));
        assert!((panned.center_uv - expect).length() < 1e-6);
    }

    #[test]
    fn zoom_beside_the_texture_and_window_resize_keep_it_reachable() {
        let r = rect();
        let tex = [1024, 1024];
        // Zoom in a lot about the view corner while the texture sits in the middle: the texture would leave the view.
        let mut view = UvViewState::default();
        view.fit(tex, r);
        for _ in 0..30 {
            view.zoom_about(r.left_top() + egui::vec2(1.0, 1.0), 1.5, tex, r);
            assert_reachable(&view, tex, r, "zoom in at the corner");
        }
        assert_eq!(view.zoom, MAX_ZOOM);
        for _ in 0..60 {
            view.zoom_about(r.right_bottom() - egui::vec2(1.0, 1.0), 0.7, tex, r);
            assert_reachable(&view, tex, r, "zoom out at the corner");
        }
        assert_eq!(view.zoom, MIN_ZOOM);
        // At the limit for a large view, then the window (or the Split half) shrinks: the next clamp keeps it reachable.
        let big = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 1000.0));
        let small = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 200.0));
        let mut view = UvViewState { center_uv: Vec2::splat(0.5), zoom: 0.5 };
        view.pan(egui::vec2(-20_000.0, 9_000.0), tex, big);
        assert_reachable(&view, tex, big, "big view");
        let (_, w_small, _) = overlap(&view, tex, small);
        assert!(w_small < 1.0, "the shrunk view no longer shows the texture before the clamp");
        view.clamp_to(tex, small);
        assert_reachable(&view, tex, small, "after the resize");
    }

    #[test]
    fn clamp_repairs_non_finite_views() {
        let r = rect();
        let mut view = UvViewState { center_uv: Vec2::new(f32::NAN, 2.0), zoom: f32::INFINITY };
        view.clamp_to([256, 256], r);
        assert_eq!(view.zoom, 1.0);
        assert_reachable(&view, [256, 256], r, "repaired");
        // A degenerate view rect changes nothing but the repair.
        let mut view = UvViewState { center_uv: Vec2::new(50.0, -50.0), zoom: 1.0 };
        view.clamp_to([256, 256], egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(0.0, 300.0)));
        assert_eq!(view.center_uv, Vec2::new(50.0, -50.0));
    }

    #[test]
    fn fit_shows_the_whole_texture() {
        let mut view = UvViewState::default();
        for tex in [[1024u32, 1024u32], [4096, 512], [256, 2048]] {
            view.fit(tex, rect());
            let a = view.uv_to_screen(Vec2::ZERO, tex, rect());
            let b = view.uv_to_screen(Vec2::ONE, tex, rect());
            assert!(rect().contains(a) && rect().contains(b));
            let r = egui::Rect::from_two_pos(a, b);
            assert!(r.width() > rect().width() * 0.85 || r.height() > rect().height() * 0.85);
        }
    }
}
