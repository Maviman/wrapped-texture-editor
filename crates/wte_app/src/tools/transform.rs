//! Transform (ARCHITECTURE.md 8.6): selects stamps and moves, scales and rotates the selected one with its gizmo, in
//! the view that matches its placement kind. Drags are live `Commit::Preview` edits finished by one `Commit::Final`.

use eframe::egui;
use glam::Vec2;
use wte_core::{Commit, Placement, StampId, StampSource, SurfaceMode};

use crate::state::{AppState, ViewId};
use crate::tools::text_path::{self, PathDrag, PathPoints};
use crate::tools::{Surface, Tool, ViewPointer, ViewXform, error_text};
use crate::view::gizmo::{self, GizmoHandle, GizmoShape};

/// Wheel step of the scale (x1.1 per notch) and the rotation (5 degrees per notch).
const SCALE_STEP: f32 = 1.1;
const ROTATE_STEP: f32 = 5.0 * std::f32::consts::PI / 180.0;
/// Limits of one drag's scale ratio.
const MIN_RATIO: f32 = 0.01;
const MAX_RATIO: f32 = 100.0;
/// Below this cursor distance (points) from the centre a corner drag does not scale along that axis.
const MIN_LEVER: f32 = 2.0;

/// A gizmo drag in progress.
#[derive(Copy, Clone, Debug)]
struct Drag {
    id: StampId,
    handle: GizmoHandle,
    view: ViewId,
    start: Placement,
    shape: GizmoShape,
    press: egui::Pos2,
    /// Body drag: offset from the cursor to the decal centre on screen.
    grab: egui::Vec2,
    current: Placement,
    moved: bool,
    /// Rotate drag: cursor angle of the last frame and the accumulated turn (radians, clockwise on screen).
    angle_last: f32,
    angle_total: f32,
    /// +1 when the viewer sees the projected decal from its front (+Z) side, else -1.
    view_sign: f32,
    /// The projected decal's depth range was automatic when the drag started: it is re-fitted to the surface under
    /// the new footprint (6.5); a depth typed in by the user is kept (and scaled along).
    auto_depth: bool,
}

/// True when `placement` is projected and its depth range is the automatic one (`Document::stamp_depth_is_auto`).
fn depth_is_auto(state: &AppState, placement: &Placement) -> bool {
    match placement {
        Placement::Projected(p) => state.doc().is_some_and(|d| d.stamp_depth_is_auto(p)),
        Placement::Uv(_) => false,
    }
}

/// `placement` with its depth range re-fitted to the surface when `auto` (6.5).
fn refit_depth(state: &AppState, placement: Placement, auto: bool) -> Placement {
    match (placement, state.doc()) {
        (Placement::Projected(p), Some(doc)) if auto => Placement::Projected(doc.fit_stamp_depth(&p)),
        _ => placement,
    }
}

/// Transform: selects stamps and moves, scales and rotates the selected one with its gizmo.
#[derive(Default)]
pub struct TransformTool {
    drag: Option<Drag>,
    /// A curve point drag of a text on a Curve (FEATURE_TEXT 6.5), next to the gizmo drag.
    path_drag: Option<PathDrag>,
    /// The handle under the cursor.
    hot: Option<(ViewId, GizmoHandle)>,
    /// The curve point under the cursor.
    point_hot: Option<(ViewId, u8)>,
    /// An open wheel gesture: input time of its last notch. Its notches are `Commit::Preview` edits of one pending
    /// edit, finished by `end_wheel` once the wheel has been idle for `WHEEL_IDLE` seconds (one history entry per
    /// gesture, and no full-quality bake per notch).
    wheel_last: Option<f64>,
    /// Input time of the current frame (`ToolController::set_clock`).
    now: f64,
    /// Round 6: the selected wrapped stamp and placement whose parameterization `ensure_wrap_param` could not make
    /// (no surface within reach): not tried again every frame. Objects-tabs: with the document's visibility generation
    /// of the try (showing or hiding objects changes what the stamp can wrap onto, so it is tried again).
    wrap_failed: Option<(StampId, Placement, u64)>,
}

/// Seconds without a wheel event after which a wheel gesture is committed.
pub(crate) const WHEEL_IDLE: f64 = 0.3;

/// The selected stamp and its placement; a stale selection counts as none.
fn selected(state: &AppState) -> Option<(StampId, Placement)> {
    let session = state.session.as_ref()?;
    let id = session.selected_stamp?;
    session.doc.stamp(id).map(|s| (id, s.placement))
}

/// True when a locked linked layer protects the stamp (it can be selected, not moved).
fn locked(state: &AppState, id: StampId) -> bool {
    state.doc().is_some_and(|d| d.stamp_locked(id))
}

/// Warns that a locked stamp cannot be moved, scaled or turned, naming the set that holds the lock (the Layers list
/// shows one set at a time, and the lock may sit on the stamp's layer in another set).
fn warn_locked(state: &mut AppState, id: StampId) {
    let text = state.doc().map(|d| crate::panels::layers::stamp_lock_message(d, id)).unwrap_or_default();
    state.toasts.warn(text);
}

/// The gizmo of a placement in this view (a UV placement only in the 2D view that shows its set).
fn shape_for(state: &AppState, xf: &ViewXform, placement: &Placement) -> Option<GizmoShape> {
    if let Placement::Uv(u) = placement
        && state.session.as_ref().is_none_or(|s| s.active_set != u.set)
    {
        return None;
    }
    gizmo::gizmo_shape(placement, xf)
}

/// Selects a stamp: its home-set layer becomes the active layer (the rule of `SelectStamp`, 8.2).
fn select(state: &mut AppState, id: Option<StampId>) {
    let Some(session) = state.session.as_mut() else { return };
    session.selected_stamp = id;
    let Some(id) = id else { return };
    let Some(home) = session.doc.stamp(id).map(|s| s.home_set) else { return };
    if let Some((_, layer)) = session.doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home) {
        session.active_set = home;
        if let Err(e) = session.doc.set_active_layer(home, Some(layer)) {
            state.toasts.warn(error_text(&e));
        }
    }
}

/// +1 when the camera of a 3D view looks at the front of the decal, -1 from behind (rotation sense, 4.1).
fn view_sign(xf: &ViewXform, placement: &Placement) -> f32 {
    match placement {
        Placement::Projected(p) if xf.is_3d => {
            let (_, _, out) = p.axes();
            if out.dot(xf.camera.eye() - p.origin) < 0.0 { -1.0 } else { 1.0 }
        }
        _ => 1.0,
    }
}

/// Wraps an angle difference into (-pi, pi].
fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI { w + TAU } else { w }
}

/// The placement scaled by `(sx, sy)` along its own axes (projected decals also scale their depth).
fn scaled(placement: &Placement, sx: f32, sy: f32) -> Placement {
    match *placement {
        Placement::Projected(mut p) => {
            p.half_size = Vec2::new(p.half_size.x * sx, p.half_size.y * sy);
            let s = sx.max(sy);
            p.depth_front *= s;
            p.depth_back *= s;
            Placement::Projected(p)
        }
        Placement::Uv(mut u) => {
            u.size_px = Vec2::new((u.size_px.x * sx).max(1.0), (u.size_px.y * sy).max(1.0));
            Placement::Uv(u)
        }
    }
}

/// The placement turned by `radians` clockwise as seen by the viewer (`sign` from `view_sign`).
fn rotated(placement: &Placement, radians: f32, sign: f32) -> Placement {
    match *placement {
        Placement::Projected(p) => Placement::Projected(p.rotated(radians * sign)),
        Placement::Uv(mut u) => {
            // Kept in (-pi, pi]: the Rotation field of the tool options shows -180..=180 degrees.
            u.rotation = wrap_angle(u.rotation + radians);
            Placement::Uv(u)
        }
    }
}

impl TransformTool {
    /// Start of a frame: clears the hover highlight.
    pub(crate) fn begin_frame(&mut self) {
        self.hot = None;
        self.point_hot = None;
    }

    /// The curve points of the selected text on a Curve in this view, with the frame taken (and cached) from the
    /// document (FEATURE_TEXT 6.5: `stamp_text_frame` in `hover`, `press` and `drag`).
    fn points_now(state: &mut AppState, xf: &ViewXform, id: StampId) -> Option<PathPoints> {
        let doc = state.doc_mut()?;
        let (placement, spec) = text_path::curve_stamp(doc, id)?;
        let frame = doc.stamp_text_frame(id).ok()?;
        // Round 6: a wrapped text's points lie on the surface.
        let points = text_path::path_points_on(doc, id, &placement, &frame, spec.fx.path.as_ref()?, xf)?;
        // A UV text only in the 2D view that shows its set (as the gizmo).
        shape_for(state, xf, &placement)?;
        Some(points)
    }

    /// The curve point under the pointer, unless the rotate knob is there (hit order of FEATURE_TEXT 6.5: the knob,
    /// then the curve points, then the corners, then the body).
    fn point_under(state: &mut AppState, xf: &ViewXform, id: StampId, placement: &Placement, pos: egui::Pos2) -> Option<(u8, egui::Pos2)> {
        if shape_for(state, xf, placement).and_then(|shape| gizmo::hit_test(&shape, pos)) == Some(GizmoHandle::Rotate) {
            return None;
        }
        let points = Self::points_now(state, xf, id)?;
        let i = text_path::hit(&points, pos)?;
        Some((i, points.points[i as usize]))
    }

    /// One frame of a curve point drag: the point under the pointer (clamped to the coordinate range) as a `Preview`.
    /// A refused `Preview` keeps the last accepted point; its reason is shown once per drag.
    fn drag_point(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        let Some(mut d) = self.path_drag.take() else { return };
        if d.view == xf.view
            && let Some(doc) = state.doc_mut()
            && let Some((placement, spec)) = text_path::curve_stamp(doc, d.id)
            && let Ok(frame) = doc.stamp_text_frame(d.id)
            && let Some(em) = text_path::screen_to_em_on(doc, d.id, &placement, &frame, xf, p.pos + d.grab)
            && let Some(next) = text_path::with_point(&spec, d.point, em)
            && next != spec
        {
            match doc.set_stamp_source(d.id, StampSource::Text(next), Commit::Preview) {
                Ok(()) => d.moved = true,
                Err(e) => {
                    if !d.refused_said {
                        d.refused_said = true;
                        state.status_hint = error_text(&e);
                    }
                }
            }
        }
        self.point_hot = Some((d.view, d.point));
        self.path_drag = Some(d);
    }

    /// Commits a curve point drag that moved: the last accepted point, one history entry.
    fn commit_point(state: &mut AppState, d: &PathDrag) {
        if !d.moved {
            return;
        }
        let Some(doc) = state.doc_mut() else { return };
        let Some(StampSource::Text(spec)) = doc.stamp(d.id).map(|s| s.source.clone()) else { return };
        if let Err(e) = doc.set_stamp_source(d.id, StampSource::Text(spec), Commit::Final) {
            state.toasts.warn(error_text(&e));
        }
    }

    /// The input time of the current frame (seconds).
    pub(crate) fn set_clock(&mut self, now: f64) {
        self.now = now;
    }

    /// Round 6 (R6-2): makes the parameterization of the selected wrapped stamp on the UI thread when the cache holds
    /// none that serves its placement (after opening a project, an undo or a redo the caches come back, the
    /// parameterizations do not), so that its outline, its curve handles and its stats line can be drawn. Not while
    /// its bake is pending (the bake brings one), not during a pending edit (check:B: its throttled Preview bakes bring
    /// one) and not again for a placement that had no surface within reach.
    /// Logged with its time. Returns true when it made one.
    ///
    /// Round 8 (R8-2): while the document's wrapped Previews go to jobs (the asynchronous mode), it makes nothing here:
    /// it asks `stamp_bakes` for a param-only job (`WrapJobs::request_param`) and the stats line says "Wrapping..." until
    /// that lands; a placement for which that job found no surface is not asked for again.
    pub(crate) fn ensure_wrap_param(&mut self, state: &mut AppState) -> bool {
        // check:B (round 9, plan T8: "when Wrap is chosen"): the Stamp or Text tool set to Wrap warms up the mesh graph
        // (and the shared texels) before the first wrapped stamp is placed, so its first Final does not pay the build.
        if matches!(state.tool, crate::state::ToolKind::Stamp | crate::state::ToolKind::Text)
            && state.tools.stamp.mode == SurfaceMode::Wrap
            && state.session.is_some()
        {
            state.wrap_jobs.request_warmup(true);
        }
        let Some((id, placement)) = selected(state) else { return false };
        let Placement::Projected(p) = placement else { return false };
        // Round 9 (package B, T8 / T9): a selected 3D stamp has the mesh's shared texels found, a wrapped one also the
        // mesh graph built, off the UI thread.
        state.wrap_jobs.request_warmup(p.mode == SurfaceMode::Wrap);
        let visibility = state.doc().map_or(0, |d| d.visibility_generation());
        if p.mode != SurfaceMode::Wrap
            || self.wrap_failed == Some((id, placement, visibility))
            || state.wrap_jobs.param_failed == Some((id, placement, visibility))
        {
            return false;
        }
        let Some(doc) = state.doc_mut() else { return false };
        // Check:B: not during a pending edit either (a drag, a wheel gesture, typing): its throttled Preview bakes
        // (`view::bake_stale_previews`) bring the parameterization; making it here in every frame of a drag would put
        // the parameterization of each new placement on the UI thread past the throttle.
        if doc.stamp_wrap_stats(id).is_some() || doc.stamp_is_unbaked(id) || doc.has_pending_edit() {
            return false;
        }
        if doc.defers_wrap_previews() {
            state.wrap_jobs.request_param(id);
            return false;
        }
        let start = std::time::Instant::now();
        let stats = doc.ensure_wrap_param(id);
        state.wrap_jobs.counters.ui_params += 1;
        let ms = start.elapsed().as_secs_f64() * 1e3;
        match stats {
            Some(s) => {
                self.wrap_failed = None;
                log::info!("wrap: selected stamp {} got its parameterization in {ms:.1} ms ({} tris, covered {:.3})", id.0, s.tris, s.covered);
                true
            }
            None => {
                self.wrap_failed = Some((id, placement, visibility));
                log::info!("wrap: selected stamp {} has no surface to wrap onto ({ms:.1} ms)", id.0);
                false
            }
        }
    }

    /// Commits an open wheel gesture once the wheel has been idle for `WHEEL_IDLE` seconds at `now` (or at once with
    /// `force`). Returns the seconds still to wait while the gesture stays open.
    pub(crate) fn end_wheel(&mut self, state: &mut AppState, now: f64, force: bool) -> Option<f64> {
        let last = self.wheel_last?;
        let still_open = state.doc().is_some_and(|d| d.has_pending_edit());
        let wait = last + WHEEL_IDLE - now;
        if still_open && !force && wait > 0.0 {
            return Some(wait);
        }
        self.wheel_last = None;
        if still_open && let Some(doc) = state.doc_mut() {
            doc.flush_pending_edit();
        }
        None
    }

    /// Starts a drag of `handle` on the stamp `id`.
    #[expect(clippy::too_many_arguments, reason = "the drag's whole starting state")]
    fn start(
        &mut self,
        state: &AppState,
        xf: &ViewXform,
        p: &ViewPointer,
        id: StampId,
        placement: Placement,
        shape: GizmoShape,
        handle: GizmoHandle,
    ) {
        let angle = (p.pos - shape.center).angle();
        self.drag = Some(Drag {
            auto_depth: depth_is_auto(state, &placement),
            id,
            handle,
            view: xf.view,
            start: placement,
            shape,
            press: p.pos,
            grab: shape.center - p.pos,
            current: placement,
            moved: false,
            angle_last: angle,
            angle_total: 0.0,
            view_sign: view_sign(xf, &placement),
        });
        self.hot = Some((xf.view, handle));
    }

    /// The placement for the cursor at `pos` during drag `d`.
    fn dragged(d: &mut Drag, state: &AppState, xf: &ViewXform, p: &ViewPointer) -> Option<Placement> {
        let doc = state.doc()?;
        match d.handle {
            GizmoHandle::Body => match d.start {
                Placement::Projected(start) => {
                    let ray = xf.camera.ray(p.pos + d.grab, xf.rect, xf.scene_radius);
                    let hit = doc.raycast(&ray)?;
                    let mut moved = start.moved_to(hit.position, hit.normal);
                    // Round 6: a wrapped decal lies on the surface; the back-face setting is a Project setting.
                    if !hit.front_face && moved.mode == SurfaceMode::Project {
                        moved.backface_cull = false;
                    }
                    Some(Placement::Projected(moved))
                }
                Placement::Uv(mut u) => {
                    u.center_px += xf.screen_to_px(p.pos) - xf.screen_to_px(d.press);
                    Some(Placement::Uv(u))
                }
            },
            GizmoHandle::Corner(_) => {
                let v0 = d.press - d.shape.center;
                let v1 = p.pos - d.shape.center;
                let uniform = if v0.length() < MIN_LEVER { 1.0 } else { v1.length() / v0.length() };
                let (sx, sy) = if p.mods.shift {
                    let axis_x = (d.shape.corners[1] - d.shape.corners[0]).normalized();
                    let axis_y = (d.shape.corners[0] - d.shape.corners[3]).normalized();
                    let along = |axis: egui::Vec2| {
                        let a0 = v0.dot(axis).abs();
                        if a0 < MIN_LEVER || !axis.is_finite() { 1.0 } else { v1.dot(axis).abs() / a0 }
                    };
                    (along(axis_x), along(axis_y))
                } else {
                    (uniform, uniform)
                };
                Some(scaled(&d.start, sx.clamp(MIN_RATIO, MAX_RATIO), sy.clamp(MIN_RATIO, MAX_RATIO)))
            }
            GizmoHandle::Rotate => {
                let v = p.pos - d.shape.center;
                if v.length() < MIN_LEVER {
                    return Some(d.current);
                }
                let angle = v.angle();
                d.angle_total += wrap_angle(angle - d.angle_last);
                d.angle_last = angle;
                Some(rotated(&d.start, d.angle_total, d.view_sign))
            }
        }
    }
}

impl Tool for TransformTool {
    fn hover(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        self.point_hot = selected(state)
            .and_then(|(id, placement)| Self::point_under(state, xf, id, &placement, p.pos))
            .map(|(i, _)| (xf.view, i));
        self.hot = if self.point_hot.is_some() {
            None
        } else {
            selected(state)
                .and_then(|(_, placement)| shape_for(state, xf, &placement))
                .and_then(|shape| gizmo::hit_test(&shape, p.pos))
                .map(|h| (xf.view, h))
        };
    }

    fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) -> bool {
        self.drag = None;
        self.path_drag = None;
        // A wheel gesture still open is its own history entry.
        self.end_wheel(state, self.now, true);
        // A curve point of the selected text (after the knob, before the corners and the body).
        if let Some((id, placement)) = selected(state)
            && let Some((point, at)) = Self::point_under(state, xf, id, &placement, p.pos)
        {
            if locked(state, id) {
                warn_locked(state, id);
                return false;
            }
            self.path_drag =
                Some(PathDrag { id, view: xf.view, point, grab: at - p.pos, moved: false, refused_said: false });
            self.point_hot = Some((xf.view, point));
            return true;
        }
        // A handle of the selected stamp's gizmo.
        if let Some((id, placement)) = selected(state)
            && let Some(shape) = shape_for(state, xf, &placement)
            && let Some(handle) = gizmo::hit_test(&shape, p.pos)
        {
            if locked(state, id) {
                warn_locked(state, id);
                return false;
            }
            self.start(state, xf, p, id, placement, shape, handle);
            return true;
        }
        // Otherwise pick the stamp under the cursor (a body drag starts right away), or deselect.
        let picked = match (p.surface, state.doc()) {
            (Surface::Model(hit), Some(doc)) => hit.texture_set.zip(hit.uv).and_then(|(set, uv)| doc.stamp_at(set, uv)),
            (Surface::Uv { set, uv, .. }, Some(doc)) => doc.stamp_at(set, uv),
            _ => None,
        };
        select(state, picked);
        // The "drag the handles" hint of a placed stamp is stale once the selection is gone.
        if picked.is_none() {
            state.status_hint.clear();
        }
        if let Some((id, placement)) = selected(state)
            && !locked(state, id)
            && let Some(shape) = shape_for(state, xf, &placement)
        {
            self.start(state, xf, p, id, placement, shape, GizmoHandle::Body);
            return true;
        }
        false
    }

    fn drag(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        if self.path_drag.is_some() {
            self.drag_point(state, xf, p);
            return;
        }
        let Some(mut d) = self.drag else { return };
        if d.view != xf.view {
            return;
        }
        // A placement the drag did not change (a click without motion) is not re-fitted either: no edit.
        let start = d.start;
        if let Some(next) =
            Self::dragged(&mut d, state, xf, p).map(|next| if next == start { next } else { refit_depth(state, next, d.auto_depth) })
            && next != d.current
        {
            let Some(doc) = state.doc_mut() else { return };
            match doc.set_stamp_placement(d.id, next, Commit::Preview) {
                Ok(()) => {
                    d.current = next;
                    d.moved = true;
                }
                Err(e) => state.toasts.warn(error_text(&e)),
            }
        }
        self.hot = Some((d.view, d.handle));
        self.drag = Some(d);
    }

    fn release(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        if self.path_drag.as_ref().is_some_and(|d| d.view == xf.view) {
            self.drag_point(state, xf, p);
        }
        if let Some(d) = self.path_drag.take() {
            Self::commit_point(state, &d);
            return;
        }
        if self.drag.is_some_and(|d| d.view == xf.view) {
            self.drag(state, xf, p);
        }
        let Some(d) = self.drag.take() else { return };
        if !d.moved {
            return;
        }
        let Some(doc) = state.doc_mut() else { return };
        if let Err(e) = doc.set_stamp_placement(d.id, d.current, Commit::Final) {
            state.toasts.warn(error_text(&e));
        }
    }

    fn scroll(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer, notches: f32) -> bool {
        let Some((id, placement)) = selected(state) else { return false };
        let scale = p.mods.ctrl || p.mods.command;
        if !scale && !p.mods.shift {
            return false;
        }
        if locked(state, id) {
            warn_locked(state, id);
            return true;
        }
        // One wheel gesture = one pending Preview edit (8.6): every event (whole notches and a touchpad's fractions
        // alike) is applied at once and only Preview-baked; `end_wheel` commits the gesture with one Final bake and
        // one history entry after the wheel has been idle for a moment.
        let sign = view_sign(xf, &placement);
        let next = if scale {
            scaled(&placement, SCALE_STEP.powf(notches), SCALE_STEP.powf(notches))
        } else {
            rotated(&placement, ROTATE_STEP * notches, sign)
        };
        let next = refit_depth(state, next, depth_is_auto(state, &placement));
        let Some(doc) = state.doc_mut() else { return true };
        match doc.set_stamp_placement(id, next, Commit::Preview) {
            Ok(()) => self.wheel_last = Some(self.now),
            Err(e) => state.toasts.warn(error_text(&e)),
        }
        true
    }

    fn finish(&mut self, state: &mut AppState) {
        // As `release`, at the point or placement reached so far.
        if let Some(d) = self.path_drag.take() {
            Self::commit_point(state, &d);
            return;
        }
        let Some(d) = self.drag.take() else { return };
        if !d.moved {
            return;
        }
        let Some(doc) = state.doc_mut() else { return };
        if let Err(e) = doc.set_stamp_placement(d.id, d.current, Commit::Final) {
            state.toasts.warn(error_text(&e));
        }
    }

    fn cancel(&mut self, state: &mut AppState) {
        // Esc during a curve point drag restores the curve (no history entry).
        if let Some(d) = self.path_drag.take()
            && d.moved
            && let Some(doc) = state.doc_mut()
        {
            doc.cancel_pending_edit();
        }
        if let Some(d) = self.drag.take()
            && d.moved
            && let Some(doc) = state.doc_mut()
        {
            doc.cancel_pending_edit();
        }
    }

    fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter) {
        let Some((id, placement)) = selected(state) else { return };
        let Some(shape) = shape_for(state, xf, &placement) else { return };
        let hot = self.hot.filter(|(v, _)| *v == xf.view).map(|(_, h)| h);
        // Round 6: a wrapped decal's edge is its outline on the surface; the gizmo draws its flat frame faintly.
        if shape.wrapped
            && let Some(doc) = state.doc()
        {
            crate::view::draw_polylines(painter, &gizmo::wrapped_outline(doc, id, xf), egui::Color32::WHITE, 1.5);
        }
        gizmo::draw(painter, &shape, hot);
        // The curve of a text on a Curve, from the cached frame (`hover` / `press` / `drag` fill it).
        let point_hot = self.point_hot.filter(|(v, _)| *v == xf.view).map(|(_, i)| i);
        if let Some(doc) = state.doc()
            && let Some((placement, spec)) = text_path::curve_stamp(doc, id)
            && let Some(frame) = doc.cached_text_frame(id)
            && let Some(path) = spec.fx.path.as_ref()
            && let Some(points) = text_path::path_points_on(doc, id, &placement, &frame, path, xf)
        {
            text_path::draw(painter, &points, point_hot);
        }
        if point_hot.is_some() {
            painter.ctx().set_cursor_icon(egui::CursorIcon::Move);
            return;
        }
        let icon = match hot {
            Some(GizmoHandle::Body) => egui::CursorIcon::Move,
            Some(GizmoHandle::Corner(0 | 2)) => egui::CursorIcon::ResizeNwSe,
            Some(GizmoHandle::Corner(_)) => egui::CursorIcon::ResizeNeSw,
            Some(GizmoHandle::Rotate) => egui::CursorIcon::Alias,
            None => return,
        };
        painter.ctx().set_cursor_icon(icon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ToolKind;
    use crate::tools::ToolController;
    use crate::tools::test_support::*;
    use glam::Vec3;
    use std::sync::Arc;
    use wte_core::{Projector, SampleKind, StampSource, TextureSetId, UvPlacement, encode_png, samples};

    /// Sphere with an image stamp projected at the front, selected, Transform tool.
    fn stamped() -> (AppState, StampId) {
        let mut state = state_with(SampleKind::Sphere);
        let img = samples::sample_stamp_image(64);
        let png = encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("ring.png", Arc::from(png)).unwrap();
        let xf = xf_3d(&mut state);
        let doc = state.doc_mut().unwrap();
        let Surface::Model(hit) = xf.surface_at(doc, TextureSetId(0), xf.rect.center()) else { panic!() };
        let d = doc.mesh.bounds.diagonal();
        let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::splat(d * 0.08), d * 0.08);
        let id = doc.add_stamp("ring", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        (state, id)
    }

    fn placement(state: &AppState, id: StampId) -> Placement {
        state.doc().unwrap().stamp(id).unwrap().placement
    }

    fn drag(tools: &mut ToolController, state: &mut AppState, xf: &ViewXform, from: egui::Pos2, to: egui::Pos2, mods: egui::Modifiers) {
        let p = pointer(state, xf, from, mods);
        tools.press(state, xf, &p, egui::PointerButton::Primary);
        assert!(tools.is_busy(), "press at {from:?} did not grab");
        for k in 1..=4 {
            let pos = from + (to - from) * (k as f32 / 4.0);
            let p = pointer(state, xf, pos, mods);
            tools.drag(state, xf, &p);
        }
        let p = pointer(state, xf, to, mods);
        tools.release(state, xf, &p);
    }

    #[test]
    fn body_corner_and_knob_drags_make_one_entry_each() {
        let (mut state, id) = stamped();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let undo0 = state.doc().unwrap().history.undo_len();

        // Body: the decal follows the cursor across the surface.
        let shape = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        drag(&mut tools, &mut state, &xf, shape.center, shape.center + egui::vec2(60.0, 20.0), egui::Modifiers::NONE);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 1);
        assert!(!state.doc().unwrap().has_pending_edit());
        let shape2 = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        assert!((shape2.center - (shape.center + egui::vec2(60.0, 20.0))).length() < 3.0, "{:?}", shape2.center);

        // Corner: scale up.
        let Placement::Projected(before) = placement(&state, id) else { panic!() };
        let c = shape2.corners[2];
        drag(&mut tools, &mut state, &xf, c, shape2.center + (c - shape2.center) * 1.5, egui::Modifiers::NONE);
        let Placement::Projected(after) = placement(&state, id) else { panic!() };
        assert!((after.half_size.x / before.half_size.x - 1.5).abs() < 0.05);
        assert!((after.half_size.y / before.half_size.y - 1.5).abs() < 0.05);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 2);

        // Shift+corner: only along x when dragging horizontally.
        let shape3 = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        let c = shape3.corners[1];
        let dx = (shape3.corners[1] - shape3.corners[0]).normalized() * (c - shape3.center).x * 0.5;
        drag(&mut tools, &mut state, &xf, c, c + dx, egui::Modifiers::SHIFT);
        let Placement::Projected(after2) = placement(&state, id) else { panic!() };
        assert!(after2.half_size.x > after.half_size.x * 1.2);
        assert!((after2.half_size.y / after.half_size.y - 1.0).abs() < 0.05);

        // Knob: a quarter turn clockwise on screen turns the decal's up axis to the screen right.
        let shape4 = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        let knob = shape4.rotate_knob;
        let r = (knob - shape4.center).length();
        let p = pointer(&state, &xf, knob, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        for k in 1..=8 {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_2 * (k as f32 / 8.0);
            let pos = shape4.center + egui::vec2(a.cos(), a.sin()) * r;
            let p = pointer(&state, &xf, pos, egui::Modifiers::NONE);
            tools.drag(&mut state, &xf, &p);
        }
        let p = pointer(&state, &xf, shape4.center + egui::vec2(r, 0.0), egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        let Placement::Projected(turned) = placement(&state, id) else { panic!() };
        let (_, up, _) = turned.axes();
        let (cam_right, _, _) = xf.camera.basis();
        assert!(up.dot(cam_right) > 0.9, "up {up:?}");
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 4);
    }

    /// Round 6 (R6-2, ROUND6_PLAN 5.3): a body drag of a wrapped stamp is Previews and one Final (one history
    /// entry), the mode and the Wrap options survive, the gizmo has no projection box; after an undo the controller
    /// makes the parameterization of the selected stamp again, once.
    #[test]
    fn a_wrapped_stamp_drags_like_a_projected_one_and_keeps_its_mode() {
        use wte_core::{SurfaceMode, WrapOptions};
        let (mut state, id) = stamped();
        let opts = WrapOptions { jump_gaps: false, overlap: true };
        let Placement::Projected(start) = placement(&state, id) else { panic!() };
        let wrapped = Placement::Projected(Projector { mode: SurfaceMode::Wrap, wrap: opts, ..start });
        state.doc_mut().unwrap().set_stamp_placement(id, wrapped, Commit::Final).unwrap();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        // As the app runs it (io_flows::install_document): Preview bakes deferred to the views' throttle.
        state.doc_mut().unwrap().set_defer_preview_bakes(true);
        let undo0 = state.doc().unwrap().history.undo_len();
        let shape = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        assert!(shape.wrapped && shape.depth_box.is_none());
        let target = shape.center + egui::vec2(60.0, 20.0);
        let p0 = pointer(&state, &xf, shape.center, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p0, egui::PointerButton::Primary);
        for k in 1..=4 {
            let p = pointer(&state, &xf, shape.center + (target - shape.center) * (k as f32 / 4.0), egui::Modifiers::NONE);
            tools.drag(&mut state, &xf, &p);
            assert!(state.doc().unwrap().has_pending_edit(), "frame {k} of the drag is a Preview");
            // Check:B: with the app's deferred Preview bakes the moved placement has no parameterization until its
            // throttled Preview bake runs; none is made on the side in a drag frame.
            assert!(!tools.ensure_wrap_param(&mut state), "frame {k}: no parameterization outside the Preview bakes");
        }
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_none(), "the moved placement's Preview waits");
        state.doc_mut().unwrap().bake_stale_previews();
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some(), "the Preview bake brings the parameterization");
        let p = pointer(&state, &xf, target, egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        let doc = state.doc().unwrap();
        assert_eq!(doc.history.undo_len(), undo0 + 1);
        assert!(!doc.has_pending_edit());
        let Placement::Projected(moved) = placement(&state, id) else { panic!() };
        assert_eq!((moved.mode, moved.wrap), (SurfaceMode::Wrap, opts));
        assert_ne!(moved.origin, start.origin);
        let shape2 = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        assert!((shape2.center - target).length() < 3.0, "{:?}", shape2.center);
        assert!(doc.stamp_wrap_stats(id).is_some(), "the Final bake cached the parameterization");
        assert!(doc.stamp_ink_area(id) > 0.0);
        // Nothing to make while the cache serves the placement.
        assert!(!tools.ensure_wrap_param(&mut state));

        state.doc_mut().unwrap().undo().expect("the drag");
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_none(), "undo restores the caches, not the parameterization");
        assert!(tools.ensure_wrap_param(&mut state));
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some());
        assert!(!tools.ensure_wrap_param(&mut state));
    }

    #[test]
    fn moving_refits_an_automatic_depth_and_keeps_a_typed_one() {
        // The fixture's depth is the placement default of earlier versions, which counts as automatic.
        let (mut state, id) = stamped();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let shape = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        drag(&mut tools, &mut state, &xf, shape.center, shape.center + egui::vec2(70.0, 40.0), egui::Modifiers::NONE);
        let Placement::Projected(moved) = placement(&state, id) else { panic!() };
        let doc = state.doc().unwrap();
        let fitted = doc.fit_stamp_depth(&moved);
        assert_eq!((moved.depth_front, moved.depth_back), (fitted.depth_front, fitted.depth_back));
        assert!(doc.stamp_depth_is_auto(&moved));

        // Corner scaling of an automatic depth: fitted to the larger footprint.
        let shape = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        let c = shape.corners[2];
        drag(&mut tools, &mut state, &xf, c, shape.center + (c - shape.center) * 1.6, egui::Modifiers::NONE);
        let Placement::Projected(scaled_up) = placement(&state, id) else { panic!() };
        let doc = state.doc().unwrap();
        assert!(doc.stamp_depth_is_auto(&scaled_up));
        assert!(scaled_up.depth_back > moved.depth_back, "{} -> {}", moved.depth_back, scaled_up.depth_back);

        // A depth typed in by hand is kept by a move.
        let mut typed = scaled_up;
        typed.depth_back = scaled_up.depth_back * 3.0;
        typed.depth_front = scaled_up.depth_front * 0.5;
        state.doc_mut().unwrap().set_stamp_placement(id, Placement::Projected(typed), Commit::Final).unwrap();
        assert!(!state.doc().unwrap().stamp_depth_is_auto(&typed));
        let shape = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        drag(&mut tools, &mut state, &xf, shape.center, shape.center + egui::vec2(-50.0, 0.0), egui::Modifiers::NONE);
        let Placement::Projected(after) = placement(&state, id) else { panic!() };
        assert_ne!(after.origin, typed.origin);
        assert_eq!((after.depth_front, after.depth_back), (typed.depth_front, typed.depth_back));
    }

    #[test]
    fn cancel_restores_and_a_click_without_motion_adds_nothing() {
        let (mut state, id) = stamped();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let undo0 = state.doc().unwrap().history.undo_len();
        let start = placement(&state, id);
        let shape = gizmo::gizmo_shape(&start, &xf).unwrap();
        // Click on the body without moving.
        let p = pointer(&state, &xf, shape.center, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        tools.release(&mut state, &xf, &p);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0);
        // Drag, then cancel (Esc).
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let p2 = pointer(&state, &xf, shape.center + egui::vec2(50.0, 0.0), egui::Modifiers::NONE);
        tools.drag(&mut state, &xf, &p2);
        assert_ne!(placement(&state, id), start);
        tools.cancel(&mut state);
        assert_eq!(placement(&state, id), start);
        assert!(!state.doc().unwrap().has_pending_edit());
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0);
    }

    #[test]
    fn finish_commits_a_held_drag_and_a_locked_stamp_does_not_move() {
        let (mut state, id) = stamped();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let undo0 = state.doc().unwrap().history.undo_len();
        let start = placement(&state, id);
        let shape = gizmo::gizmo_shape(&start, &xf).unwrap();
        let p = pointer(&state, &xf, shape.center, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let p2 = pointer(&state, &xf, shape.center + egui::vec2(40.0, 0.0), egui::Modifiers::NONE);
        tools.drag(&mut state, &xf, &p2);
        let dragged = placement(&state, id);
        assert_ne!(dragged, start);
        tools.finish(&mut state);
        assert!(!tools.is_busy() && !state.doc().unwrap().has_pending_edit());
        assert_eq!(placement(&state, id), dragged, "the held drag is kept");
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 1);

        // Locked: the handles and the wheel only warn.
        let layer = state.doc().unwrap().stamp_layers(id)[0].1;
        state.doc_mut().unwrap().set_layer_locked(layer, true).unwrap();
        let undo1 = state.doc().unwrap().history.undo_len();
        let shape = gizmo::gizmo_shape(&dragged, &xf).unwrap();
        let p = pointer(&state, &xf, shape.center, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy(), "no drag starts on a locked stamp");
        let p = pointer(&state, &xf, shape.center, egui::Modifiers::CTRL);
        assert!(tools.scroll(&mut state, &xf, &p, 1.0), "the wheel is consumed (no camera zoom)");
        assert_eq!(placement(&state, id), dragged);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo1);
        assert!(state.toasts.visible_texts().contains(&"The stamp's layer is locked: unlock it in the Layers list first"));
        // Clicking it still selects it.
        state.session.as_mut().unwrap().selected_stamp = None;
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy());
        assert_eq!(state.session.as_ref().unwrap().selected_stamp, Some(id));
    }

    #[test]
    fn uv_rotation_stays_within_half_a_turn() {
        let u = UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::ZERO,
            size_px: Vec2::ONE,
            rotation: 175f32.to_radians(),
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let Placement::Uv(r) = rotated(&Placement::Uv(u), 15f32.to_radians(), 1.0) else { panic!() };
        assert!((r.rotation.to_degrees() + 170.0).abs() < 1e-3, "{}", r.rotation.to_degrees());
        let Placement::Uv(r) = rotated(&Placement::Uv(u), -400f32.to_radians(), 1.0) else { panic!() };
        assert!(r.rotation.abs() <= std::f32::consts::PI);
    }

    #[test]
    fn clicking_elsewhere_deselects_and_clicking_a_stamp_selects_it() {
        let (mut state, id) = stamped();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, at(&xf, 0.5, 0.15), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy());
        assert_eq!(state.session.as_ref().unwrap().selected_stamp, None);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        tools.release(&mut state, &xf, &p);
        assert_eq!(state.session.as_ref().unwrap().selected_stamp, Some(id));
        let set = &state.doc().unwrap().sets[0];
        assert_eq!(set.active, state.doc().unwrap().stamp_layers(id).first().map(|(_, l)| *l));
    }

    /// Finding SP-1: a wheel gesture is one pending Preview edit (no full-quality bake per notch), committed as one
    /// history entry once the wheel has been idle for `WHEEL_IDLE` seconds.
    #[test]
    fn a_wheel_gesture_scales_and_rotates_as_one_entry() {
        let (mut state, id) = stamped();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let undo0 = state.doc().unwrap().history.undo_len();
        let Placement::Projected(before) = placement(&state, id) else { panic!() };
        tools.set_clock(10.0);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::CTRL);
        assert!(tools.scroll(&mut state, &xf, &p, 2.0));
        tools.set_clock(10.1);
        assert!(tools.scroll(&mut state, &xf, &p, 0.5), "a touchpad fraction");
        let Placement::Projected(after) = placement(&state, id) else { panic!() };
        assert!((after.half_size.x / before.half_size.x - 1.1f32.powf(2.5)).abs() < 1e-3);
        assert!(state.doc().unwrap().has_pending_edit(), "the gesture is still open");
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0);
        // Not idle long enough yet: still open, look again later.
        let wait = tools.end_frame(&mut state, 10.2).expect("still open");
        assert!(wait > 0.0 && wait <= WHEEL_IDLE);
        // Idle: one entry for the whole gesture, baked at full quality.
        assert_eq!(tools.end_frame(&mut state, 10.1 + WHEEL_IDLE + 0.01), None);
        assert!(!state.doc().unwrap().has_pending_edit());
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 1);
        // A rotation gesture is the next entry; a press on the gizmo commits an open gesture at once.
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::SHIFT);
        assert!(tools.scroll(&mut state, &xf, &p, 1.0));
        let shape = gizmo::gizmo_shape(&placement(&state, id), &xf).unwrap();
        let press = pointer(&state, &xf, shape.center, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &press, egui::PointerButton::Primary);
        tools.release(&mut state, &xf, &press);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 2);
        assert!(!state.doc().unwrap().has_pending_edit());
        // Undo brings back the size before the first gesture.
        state.doc_mut().unwrap().undo();
        state.doc_mut().unwrap().undo();
        assert_eq!(placement(&state, id), Placement::Projected(before));
        // Without a selection the wheel is not consumed (the camera zooms).
        state.session.as_mut().unwrap().selected_stamp = None;
        assert!(!tools.scroll(&mut state, &xf, &p, 1.0));
    }

    #[test]
    fn uv_stamp_is_edited_in_the_2d_view() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Transform;
        let img = samples::sample_stamp_image(32);
        let png = encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("ring.png", Arc::from(png)).unwrap();
        let [w, h] = [doc.sets[0].width as f32, doc.sets[0].height as f32];
        let u = UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(w * 0.5, h * 0.5),
            size_px: Vec2::splat(w * 0.1),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let id = doc.add_stamp("uv", StampSource::Image { blob }, Placement::Uv(u), TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        let xf3 = xf_3d(&mut state);
        let xf = xf_2d(&mut state);
        assert!(shape_for(&state, &xf3, &placement(&state, id)).is_none());
        let shape = shape_for(&state, &xf, &placement(&state, id)).unwrap();
        let mut tools = ToolController::new();
        drag(&mut tools, &mut state, &xf, shape.center, shape.center + egui::vec2(30.0, -10.0), egui::Modifiers::NONE);
        let Placement::Uv(moved) = placement(&state, id) else { panic!() };
        let expected = u.center_px + Vec2::new(30.0, -10.0) / xf.uv.zoom;
        assert!((moved.center_px - expected).length() < 0.01, "{:?} vs {expected:?}", moved.center_px);
        // Knob: clockwise quarter turn = +90 degrees.
        let shape = shape_for(&state, &xf, &placement(&state, id)).unwrap();
        let r = (shape.rotate_knob - shape.center).length();
        drag(&mut tools, &mut state, &xf, shape.rotate_knob, shape.center + egui::vec2(r, 0.0), egui::Modifiers::NONE);
        let Placement::Uv(turned) = placement(&state, id) else { panic!() };
        // The straight drag from the knob to the right passes the quarter turn in 4 steps.
        assert!((turned.rotation - std::f32::consts::FRAC_PI_2).abs() < 0.05, "{}", turned.rotation);
    }

    // ----- round 4: curve points of a text on a Curve (FEATURE_TEXT 6.5, 8.2 "text_path.rs") -----

    /// Sphere, 2D view, a selected UV text with a Curve through `points` (em), one em about 100 texels; Transform.
    fn curved(content: &str, points: &[[f32; 2]], align: wte_core::TextAlign) -> (AppState, StampId, ViewXform) {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_2d(&mut state);
        let doc = state.doc_mut().unwrap();
        let mut spec = wte_core::TextSpec::new(content, doc.default_font());
        spec.align = align;
        spec.fx.path = Some(wte_core::TextPath {
            shape: wte_core::PathShape::Curve { points: points.to_vec() },
            start: 0.0,
            flip: false,
        });
        let source = StampSource::Text(spec);
        let frame = doc.stamp_source_frame(&source).unwrap();
        let k = 100.0 / frame.em_px;
        let [w, h] = [doc.sets[0].width as f32, doc.sets[0].height as f32];
        let u = UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(w * 0.5, h * 0.45),
            size_px: Vec2::new(frame.size[0] as f32 * k, frame.size[1] as f32 * k),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let id = doc.add_stamp("curve", source, Placement::Uv(u), TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        (state, id, xf)
    }

    fn curve_of(state: &AppState, id: StampId) -> Vec<[f32; 2]> {
        let (_, spec) = text_path::curve_stamp(state.doc().unwrap(), id).expect("a text on a Curve");
        text_path::curve_points(spec.fx.path.as_ref().unwrap()).unwrap().to_vec()
    }

    fn screen_points(state: &mut AppState, xf: &ViewXform, id: StampId) -> Vec<egui::Pos2> {
        TransformTool::points_now(state, xf, id).expect("curve points in this view").points
    }

    /// Sets curve point `i` of stamp `id` to the em point under `pos` (one Final).
    fn put_point(state: &mut AppState, xf: &ViewXform, id: StampId, i: u8, pos: egui::Pos2) {
        let doc = state.doc_mut().unwrap();
        let (placement, spec) = text_path::curve_stamp(doc, id).unwrap();
        let frame = doc.stamp_text_frame(id).unwrap();
        let em = text_path::screen_to_em(&placement, &frame, xf, pos).unwrap();
        let next = text_path::with_point(&spec, i, em).unwrap();
        doc.set_stamp_source(id, StampSource::Text(next), Commit::Final).unwrap();
    }

    const FIVE: [[f32; 2]; 5] = [[0.0, 0.0], [2.0, 0.0], [6.0, 0.0], [10.0, 0.0], [14.0, 0.0]];

    #[test]
    fn points_win_over_corners_and_lose_to_the_knob() {
        // "Hi" walks the first piece only, so moving the last two points changes neither the picture nor the gizmo.
        let (mut state, id, xf) = curved("Hi", &FIVE, wte_core::TextAlign::Left);
        let shape = shape_for(&state, &xf, &placement(&state, id)).unwrap();
        put_point(&mut state, &xf, id, 3, shape.corners[1]);
        put_point(&mut state, &xf, id, 4, shape.rotate_knob);
        let after = shape_for(&state, &xf, &placement(&state, id)).unwrap();
        assert!((after.corners[1] - shape.corners[1]).length() < 0.5 && (after.rotate_knob - shape.rotate_knob).length() < 0.5);
        let pts = screen_points(&mut state, &xf, id);
        assert!(pts[3].distance(after.corners[1]) < 0.5 && pts[4].distance(after.rotate_knob) < 0.5, "{pts:?}");
        let mut tools = ToolController::new();
        // The corner: the point wins.
        let p = pointer(&state, &xf, after.corners[1], egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        assert_eq!(tools.transform.point_hot, Some((xf.view, 3)));
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(tools.transform.path_drag.as_ref().is_some_and(|d| d.point == 3) && tools.transform.drag.is_none());
        tools.release(&mut state, &xf, &p);
        // The knob: the gizmo wins.
        let p = pointer(&state, &xf, after.rotate_knob, egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        assert_eq!(tools.transform.point_hot, None);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(tools.transform.path_drag.is_none());
        assert!(tools.transform.drag.is_some_and(|d| d.handle == GizmoHandle::Rotate));
        tools.release(&mut state, &xf, &p);
    }

    #[test]
    fn dragging_a_point_sends_previews_and_one_final() {
        let (mut state, id, xf) = curved("WTE 2026", &[[-2.0, 0.0], [0.0, 0.0], [2.0, 0.0]], wte_core::TextAlign::Center);
        let undo0 = state.doc().unwrap().history.undo_len();
        let before = curve_of(&state, id);
        let pts = screen_points(&mut state, &xf, id);
        let mut tools = ToolController::new();
        let from = pts[1];
        let to = from + egui::vec2(0.0, -40.0);
        let p = pointer(&state, &xf, from, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(tools.is_busy());
        for k in 1..=4 {
            let p = pointer(&state, &xf, from + (to - from) * (k as f32 / 4.0), egui::Modifiers::NONE);
            tools.drag(&mut state, &xf, &p);
            assert!(state.doc().unwrap().has_pending_edit(), "Previews while the button is down");
        }
        let p = pointer(&state, &xf, to, egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        let after = curve_of(&state, id);
        assert_eq!((after[0], after[2]), (before[0], before[2]), "only the dragged point moves");
        assert!(after[1][1] < before[1][1] - 0.5, "{:?} -> {:?}", before[1], after[1]);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 1);
        assert!(!state.doc().unwrap().has_pending_edit());
        // The point is under the pointer, and the others stayed under their marks (the anchor and the em are kept).
        let now = screen_points(&mut state, &xf, id);
        assert!(now[1].distance(to) < 1.0, "{:?} vs {to:?}", now[1]);
        assert!(now[0].distance(pts[0]) < 1.5 && now[2].distance(pts[2]) < 1.5, "{now:?} vs {pts:?}");
    }

    #[test]
    fn esc_restores_the_curve() {
        let (mut state, id, xf) = curved("WTE 2026", &[[-2.0, 0.0], [0.0, 0.0], [2.0, 0.0]], wte_core::TextAlign::Center);
        let undo0 = state.doc().unwrap().history.undo_len();
        let StampSource::Text(spec0) = state.doc().unwrap().stamp(id).unwrap().source.clone() else { panic!() };
        let placement0 = placement(&state, id);
        let pts = screen_points(&mut state, &xf, id);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, pts[2], egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let p = pointer(&state, &xf, pts[2] + egui::vec2(25.0, 30.0), egui::Modifiers::NONE);
        tools.drag(&mut state, &xf, &p);
        assert_ne!(curve_of(&state, id), text_path::curve_points(spec0.fx.path.as_ref().unwrap()).unwrap());
        // Esc while the button is down.
        tools.cancel(&mut state);
        let StampSource::Text(spec) = state.doc().unwrap().stamp(id).unwrap().source.clone() else { panic!() };
        assert_eq!(spec, spec0);
        assert_eq!(placement(&state, id), placement0);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0);
        assert!(!state.doc().unwrap().has_pending_edit() && !tools.is_busy());
    }

    #[test]
    fn a_locked_text_refuses_the_drag() {
        let (mut state, id, xf) = curved("WTE 2026", &[[-2.0, 0.0], [0.0, 0.0], [2.0, 0.0]], wte_core::TextAlign::Center);
        let layer = state.doc().unwrap().stamp_layers(id)[0].1;
        state.doc_mut().unwrap().set_layer_locked(layer, true).unwrap();
        let undo0 = state.doc().unwrap().history.undo_len();
        let before = curve_of(&state, id);
        let pts = screen_points(&mut state, &xf, id);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, pts[1], egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy(), "no drag on a locked text");
        let p = pointer(&state, &xf, pts[1] + egui::vec2(0.0, -40.0), egui::Modifiers::NONE);
        tools.drag(&mut state, &xf, &p);
        tools.release(&mut state, &xf, &p);
        assert_eq!(curve_of(&state, id), before);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0);
        assert!(state.toasts.visible_texts().contains(&"The stamp's layer is locked: unlock it in the Layers list first"));
    }

    #[test]
    fn a_drag_far_out_stops_at_the_range() {
        let (mut state, id, xf) = curved("WTE", &[[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]], wte_core::TextAlign::Center);
        let pts = screen_points(&mut state, &xf, id);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, pts[2], egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        for k in 1..=4 {
            let p = pointer(&state, &xf, pts[2] + egui::vec2(3000.0 * k as f32, 0.0), egui::Modifiers::NONE);
            tools.drag(&mut state, &xf, &p);
        }
        let p = pointer(&state, &xf, pts[2] + egui::vec2(12000.0, 0.0), egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        let after = curve_of(&state, id);
        assert_eq!(after[2][0], wte_core::PATH_COORD_MAX_EM, "{after:?}");
        assert!(after[2][1].abs() < 0.05, "{after:?}");
        assert_eq!(after[0], [-1.0, 0.0]);
    }

    #[test]
    fn a_refused_preview_keeps_the_last_point() {
        // 200 short right-aligned lines stacked along the normal at the end of the curve: turning the end tangent turns
        // the whole block, and past about 20 degrees its picture exceeds the area limit at every resolution.
        let content = vec!["Hi"; 200].join("\n");
        let (mut state, id, xf) = curved(&content, &[[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]], wte_core::TextAlign::Right);
        let undo0 = state.doc().unwrap().history.undo_len();
        let pts = screen_points(&mut state, &xf, id);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, pts[2], egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let em_pt = pts[2].distance(pts[1]);
        let mut accepted = curve_of(&state, id);
        let mut refused = 0;
        for k in 1..=12 {
            let target = pts[2] + egui::vec2(0.0, -em_pt * 0.1 * k as f32);
            let p = pointer(&state, &xf, target, egui::Modifiers::NONE);
            let hint = state.status_hint.clone();
            tools.drag(&mut state, &xf, &p);
            let now = curve_of(&state, id);
            if now == accepted {
                refused += 1;
                assert!(!state.status_hint.is_empty());
                if refused > 1 {
                    assert_eq!(state.status_hint, hint, "the reason is shown once");
                }
            } else {
                assert_eq!(refused, 0, "accepted after a refusal at step {k}");
                accepted = now;
            }
        }
        assert!(refused > 0, "no Preview was refused; last point {:?}", accepted[2]);
        assert!(accepted[2][1] < 0.0, "a Preview was accepted before the refusal");
        let p = pointer(&state, &xf, pts[2] + egui::vec2(0.0, -em_pt * 1.2), egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        assert_eq!(curve_of(&state, id), accepted, "release commits the last accepted point");
        assert_eq!(state.doc().unwrap().history.undo_len(), undo0 + 1);
        assert!(!state.doc().unwrap().has_pending_edit());
    }

    #[test]
    fn wrap_angle_stays_in_range() {
        use std::f32::consts::PI;
        assert!((wrap_angle(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-5);
        assert!((wrap_angle(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-5);
        assert!((wrap_angle(PI) - PI).abs() < 1e-5);
    }
}
