//! Tools (ARCHITECTURE.md 8.6): the screen <-> scene mapping of a view (`ViewXform`), the pointer resolved to a
//! surface (`ViewPointer`), the `Tool` trait and the `ToolController` that routes view input to navigation or to the
//! active tool.

use eframe::egui;
use glam::{Vec2, Vec3};
use wte_core::{Document, EditError, Hit, LayerId, PaintTarget, TextureSetId};

use crate::action::AppAction;
use crate::gpu::BrushRing;
use crate::state::{AppState, OrbitCamera, Session, ToolKind, UvViewState, ViewId};

pub mod brush;
pub mod eyedropper;
pub mod fill;
pub mod stamp_place;
pub mod text_path;
pub mod transform;

/// Toast of every tool that meets a triangle without UVs.
pub(crate) const NO_UV_TEXT: &str = "This part of the model has no UV mapping and cannot be painted";

/// objects-tabs (8.6): pointer travel in points below which a press of the Navigate tool is a click (an object pick),
/// not an orbit.
pub(crate) const NAV_CLICK_TRAVEL: f32 = 4.0;

/// objects-tabs (8.6): a primary press of the Navigate tool in the 3D view of a model with two or more objects, kept
/// while the drag may still turn out to be a click.
#[derive(Copy, Clone, Debug)]
struct NavClick {
    origin: egui::Pos2,
    mods: egui::Modifiers,
    /// The camera before the press: a click puts back the few points of orbit it made.
    camera: OrbitCamera,
    /// Pointer travel so far (points).
    travel: f32,
}

/// What is under the pointer.
#[derive(Copy, Clone, Debug)]
pub enum Surface {
    Model(Hit),
    Uv { set: TextureSetId, uv: Vec2, px: Vec2 },
    Miss,
}

impl Surface {
    /// The texture set and the (unwrapped) UV under the pointer, when there is one.
    pub(crate) fn set_uv(&self) -> Option<(TextureSetId, Vec2)> {
        match self {
            Surface::Model(hit) => hit.texture_set.zip(hit.uv),
            Surface::Uv { set, uv, .. } => Some((*set, *uv)),
            Surface::Miss => None,
        }
    }
}

/// The pointer in a view.
#[derive(Clone, Debug)]
pub struct ViewPointer {
    pub view: ViewId,
    pub pos: egui::Pos2,
    pub surface: Surface,
    pub pressure: f32,
    pub mods: egui::Modifiers,
}

/// Screen <-> scene mapping of the view the pointer is in.
#[derive(Copy, Clone, Debug)]
pub struct ViewXform {
    pub view: ViewId,
    pub rect: egui::Rect,
    pub is_3d: bool,
    /// 3D views.
    pub camera: OrbitCamera,
    /// 3D views: `camera.scene_radius(&mesh.bounds)`.
    pub scene_radius: f32,
    /// 2D views.
    pub uv: UvViewState,
    /// 2D views: size of the shown texture.
    pub tex_size: [u32; 2],
}

/// The camera a 3D view uses.
pub(crate) fn camera_of(session: &Session, view: ViewId) -> &OrbitCamera {
    match view {
        ViewId::Preview | ViewId::ExportPreview => &session.preview_camera,
        ViewId::Main3D | ViewId::Main2D => &session.camera,
    }
}

/// The camera a 3D view uses, mutable.
pub(crate) fn camera_of_mut(session: &mut Session, view: ViewId) -> &mut OrbitCamera {
    match view {
        ViewId::Preview | ViewId::ExportPreview => &mut session.preview_camera,
        ViewId::Main3D | ViewId::Main2D => &mut session.camera,
    }
}

/// Size of a texture set (`[1, 1]` for an unknown set, so the 2D mapping never divides by zero).
pub(crate) fn set_size(doc: &Document, set: TextureSetId) -> [u32; 2] {
    doc.set(set).map_or([1, 1], |s| [s.width.max(1), s.height.max(1)])
}

impl ViewXform {
    /// The transform of `view` as the views use it this frame (rect from `state.view_rects`, camera / uv view from the
    /// session). `None` without a session or before the view was laid out. Used by the views and by self-test `Dyn`
    /// steps.
    pub fn for_view(state: &AppState, view: ViewId) -> Option<ViewXform> {
        let session = state.session.as_ref()?;
        let rect = *state.view_rects.get(&view)?;
        Some(ViewXform::of_session(session, view, rect))
    }

    /// The transform of `view` laid out at `rect`.
    pub(crate) fn of_session(session: &Session, view: ViewId, rect: egui::Rect) -> ViewXform {
        let camera = *camera_of(session, view);
        ViewXform {
            view,
            rect,
            is_3d: view != ViewId::Main2D,
            camera,
            scene_radius: camera.scene_radius(&session.doc.mesh.bounds),
            uv: session.uv_view,
            tex_size: set_size(&session.doc, session.active_set),
        }
    }

    /// Screen position of a world point (3D views); `None` behind the camera.
    pub fn world_to_screen(&self, p: Vec3) -> Option<egui::Pos2> {
        if !self.is_3d {
            return None;
        }
        self.camera.project(p, self.rect, self.scene_radius)
    }

    /// Texel coordinates -> screen (2D views).
    pub fn px_to_screen(&self, px: Vec2) -> egui::Pos2 {
        let tex = Vec2::new(self.tex_size[0].max(1) as f32, self.tex_size[1].max(1) as f32);
        self.uv.uv_to_screen(px / tex, self.tex_size, self.rect)
    }

    /// Screen -> continuous texel coordinates (2D views).
    pub(crate) fn screen_to_px(&self, pos: egui::Pos2) -> Vec2 {
        let tex = Vec2::new(self.tex_size[0].max(1) as f32, self.tex_size[1].max(1) as f32);
        self.uv.screen_to_uv(pos, self.tex_size, self.rect) * tex
    }

    /// Screen length of a world-space length at a world point, measured along the camera's right axis (3D views).
    pub(crate) fn world_len_on_screen(&self, at: Vec3, len: f32) -> Option<f32> {
        let (right, _, _) = self.camera.basis();
        let a = self.world_to_screen(at)?;
        let b = self.world_to_screen(at + right * len)?;
        Some((b - a).length())
    }

    /// Raycast (3D) or UV lookup (2D) at a screen position. In a 2D view only positions on the texture
    /// (`uv` in `[0, 1]^2`) are a surface.
    pub fn surface_at(&self, doc: &Document, set: TextureSetId, pos: egui::Pos2) -> Surface {
        if !pos.is_finite() {
            return Surface::Miss;
        }
        if self.is_3d {
            let ray = self.camera.ray(pos, self.rect, self.scene_radius);
            return doc.raycast(&ray).map_or(Surface::Miss, Surface::Model);
        }
        if doc.set(set).is_none() {
            return Surface::Miss;
        }
        let size = set_size(doc, set);
        let uv = self.uv.screen_to_uv(pos, size, self.rect);
        if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
            return Surface::Miss;
        }
        Surface::Uv { set, uv, px: uv * Vec2::new(size[0] as f32, size[1] as f32) }
    }
}

/// What the pointer is currently doing in the views.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Interaction {
    Idle,
    Orbit(ViewId),
    Pan(ViewId),
    Tool(ViewId),
}

impl Interaction {
    /// The view the interaction belongs to.
    pub(crate) fn view(&self) -> Option<ViewId> {
        match *self {
            Interaction::Idle => None,
            Interaction::Orbit(v) | Interaction::Pan(v) | Interaction::Tool(v) => Some(v),
        }
    }
}

/// One tool of the toolbar. The controller calls it with the pointer already resolved to a surface.
pub trait Tool {
    /// The pointer moved over a view without a captured drag.
    fn hover(&mut self, _state: &mut AppState, _xf: &ViewXform, _p: &ViewPointer) {}
    /// Left button pressed. `true` = capture the drag.
    fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) -> bool;
    /// Pointer motion during a captured drag.
    fn drag(&mut self, _state: &mut AppState, _xf: &ViewXform, _p: &ViewPointer) {}
    /// The captured drag ended.
    fn release(&mut self, _state: &mut AppState, _xf: &ViewXform, _p: &ViewPointer) {}
    /// Ctrl+wheel / Shift+wheel in notches (`p.mods` = the modifiers of the wheel event). `true` = consumed.
    fn scroll(&mut self, _state: &mut AppState, _xf: &ViewXform, _p: &ViewPointer, _notches: f32) -> bool {
        false
    }
    /// Ends the current interaction without committing it.
    fn cancel(&mut self, _state: &mut AppState) {}
    /// Ends the current interaction and commits what it did so far, as a release would (an open stroke is
    /// `stroke_end`ed, a gizmo drag gets its `Commit::Final`). Tools without anything to commit just cancel.
    fn finish(&mut self, state: &mut AppState) {
        self.cancel(state);
    }
    /// egui-painter overlay of the tool in a view.
    fn overlay(&self, _state: &AppState, _xf: &ViewXform, _painter: &egui::Painter) {}
    /// The brush cursor to draw on the model.
    fn brush_ring(&self, _state: &AppState) -> Option<BrushRing> {
        None
    }
}

/// Which tool instance serves the current tool kind.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Slot {
    Brush,
    Eyedropper,
    Fill,
    Place,
    Transform,
}

/// The paint tool the editing views use: `AppState::tool`, except in the Sticker view, whose 3D half only navigates
/// (stickers P6a, STICKER_PLAN 3.1: orbit, pan, zoom, frame; the paint tool comes back when the view is left).
pub(crate) fn active_tool(state: &AppState) -> ToolKind {
    if state.view_mode == crate::state::ViewMode::Sticker { ToolKind::Navigate } else { state.tool }
}

/// The tool instance for a tool kind; Alt turns Brush / Eraser / Fill into the eyedropper (8.6).
fn slot_for(tool: ToolKind, alt: bool) -> Option<Slot> {
    match tool {
        ToolKind::Navigate => None,
        ToolKind::Brush | ToolKind::Eraser | ToolKind::Fill if alt => Some(Slot::Eyedropper),
        ToolKind::Brush | ToolKind::Eraser => Some(Slot::Brush),
        ToolKind::Fill => Some(Slot::Fill),
        ToolKind::Eyedropper => Some(Slot::Eyedropper),
        ToolKind::Stamp | ToolKind::Text => Some(Slot::Place),
        ToolKind::Transform => Some(Slot::Transform),
    }
}

/// Owns the interaction state and one instance of each tool; routes view input to the active tool.
pub struct ToolController {
    interaction: Interaction,
    brush: brush::BrushTool,
    eyedropper: eyedropper::EyedropperTool,
    fill: fill::FillTool,
    stamp_place: stamp_place::StampPlaceTool,
    transform: transform::TransformTool,
    /// The tool that captured the running `Interaction::Tool`.
    captured: Option<Slot>,
    /// The physical button that holds the running interaction.
    drag_button: egui::PointerButton,
    /// Last pointer position of an orbit / pan.
    nav_last: egui::Pos2,
    /// Alt state of the last hover (for the overlay / ring of the temporary eyedropper).
    alt: bool,
    /// The button of a brush stroke went up while its paint still trails (FEATURE_BRUSH 5.10): the interaction stays
    /// until the brush has painted everything and ended the stroke.
    brush_released: bool,
    /// objects-tabs: the Navigate press that may still be a click (object pick).
    nav_click: Option<NavClick>,
}

impl Default for ToolController {
    fn default() -> ToolController {
        ToolController::new()
    }
}

impl ToolController {
    /// Idle controller with fresh tools.
    pub fn new() -> ToolController {
        ToolController {
            interaction: Interaction::Idle,
            brush: brush::BrushTool::default(),
            eyedropper: eyedropper::EyedropperTool::default(),
            fill: fill::FillTool::default(),
            stamp_place: stamp_place::StampPlaceTool::default(),
            transform: transform::TransformTool::default(),
            captured: None,
            drag_button: egui::PointerButton::Primary,
            nav_last: egui::Pos2::ZERO,
            alt: false,
            brush_released: false,
            nav_click: None,
        }
    }

    fn tool_mut(&mut self, slot: Slot) -> &mut dyn Tool {
        match slot {
            Slot::Brush => &mut self.brush,
            Slot::Eyedropper => &mut self.eyedropper,
            Slot::Fill => &mut self.fill,
            Slot::Place => &mut self.stamp_place,
            Slot::Transform => &mut self.transform,
        }
    }

    fn tool_ref(&self, slot: Slot) -> &dyn Tool {
        match slot {
            Slot::Brush => &self.brush,
            Slot::Eyedropper => &self.eyedropper,
            Slot::Fill => &self.fill,
            Slot::Place => &self.stamp_place,
            Slot::Transform => &self.transform,
        }
    }

    /// The tool that is shown right now (the captured one during a drag).
    fn current_slot(&self, state: &AppState) -> Option<Slot> {
        self.captured.or_else(|| slot_for(active_tool(state), self.alt))
    }

    /// The current interaction.
    pub fn interaction(&self) -> &Interaction {
        &self.interaction
    }

    /// True while an interaction is running (not `Idle`); also while a released brush stroke is still being painted.
    pub fn is_busy(&self) -> bool {
        self.interaction != Interaction::Idle
    }

    /// True while a brush stroke whose button is already up is still being painted (FEATURE_BRUSH 5.10). The status
    /// bar says "Finishing the stroke... (Esc discards it)".
    pub fn finishing(&self) -> bool {
        self.brush_released && self.captured == Some(Slot::Brush) && matches!(self.interaction, Interaction::Tool(_))
    }

    /// Completes a finishing brush stroke at once (no time limit) and returns to Idle; nothing otherwise. A new press,
    /// a command and Save use it, so that the stroke the user made is never lost.
    pub(crate) fn complete_finishing(&mut self, state: &mut AppState) {
        if self.finishing() {
            self.brush.complete(state);
            self.go_idle();
        }
    }

    /// Back to Idle (no tool holds the pointer).
    fn go_idle(&mut self) {
        self.interaction = Interaction::Idle;
        self.captured = None;
        self.brush_released = false;
        self.nav_click = None;
    }

    /// Start of a views pass: resets the per-frame raycast budget and the hover feedback of the tools (it is set again
    /// by `hover` / `drag` while the pointer is over a view).
    pub(crate) fn begin_frame(&mut self) {
        self.brush.begin_frame();
        self.eyedropper.begin_frame();
        self.stamp_place.begin_frame();
        self.transform.begin_frame();
    }

    /// The input time of the current frame (seconds), for the tools that end a gesture after a pause (the Transform
    /// tool's wheel gesture).
    pub(crate) fn set_clock(&mut self, now: f64) {
        self.transform.set_clock(now);
    }

    /// Round 6 (R6-2): the parameterization of the selected wrapped stamp when the cache has none
    /// (`TransformTool::ensure_wrap_param`); called once per frame before the panels and views are drawn.
    pub(crate) fn ensure_wrap_param(&mut self, state: &mut AppState) -> bool {
        self.transform.ensure_wrap_param(state)
    }

    /// End of a views pass at input time `now`: commits a wheel gesture of the Transform tool that has been idle long
    /// enough, and pumps the brush's stroke pacing (FEATURE_BRUSH 5.10): the brush paints what waits when no drag ran
    /// in this frame; a released stroke that the brush has ended returns the controller to Idle. Returns the seconds
    /// after which to look again: `0` while the brush still has queued work, so an open stroke always gets its next
    /// frame (also with a still pointer or the pointer outside the window).
    pub(crate) fn end_frame(&mut self, state: &mut AppState, now: f64) -> Option<f64> {
        let mut wake = self.transform.end_wheel(state, now, false);
        if self.brush.pump(state) {
            wake = Some(0.0);
        }
        if self.brush_released && !self.brush.finishing() {
            self.go_idle();
        }
        wake
    }

    /// The physical button that holds the running interaction.
    pub(crate) fn drag_button(&self) -> egui::PointerButton {
        self.drag_button
    }

    /// Records the physical button of an interaction that `press` started (Space+LMB is routed as the middle button).
    pub(crate) fn set_drag_button(&mut self, button: egui::PointerButton) {
        self.drag_button = button;
    }

    /// The pointer is over a view (no captured drag).
    pub fn hover(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        if self.is_busy() {
            return;
        }
        self.alt = p.mods.alt;
        if let Some(slot) = slot_for(active_tool(state), p.mods.alt) {
            self.tool_mut(slot).hover(state, xf, p);
        }
    }

    /// A pointer button went down in a view (routing rules of section 8.6).
    pub fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer, button: egui::PointerButton) {
        // A press while a released stroke is still being painted completes that stroke first (FEATURE_BRUSH 5.10), so
        // a quick second click is not lost.
        self.complete_finishing(state);
        if self.is_busy() || state.session.is_none() {
            return;
        }
        let navigate = match button {
            egui::PointerButton::Secondary => Some(xf.is_3d),
            egui::PointerButton::Middle => Some(false),
            egui::PointerButton::Primary if active_tool(state) == ToolKind::Navigate => Some(xf.is_3d),
            egui::PointerButton::Primary => None,
            egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => return,
        };
        self.drag_button = button;
        if let Some(orbit) = navigate {
            self.interaction = if orbit { Interaction::Orbit(xf.view) } else { Interaction::Pan(xf.view) };
            self.nav_last = p.pos;
            // objects-tabs (8.6): the left button of the Navigate tool (the logical button: Space+LMB pans) in the 3D
            // view of a model with two or more objects may be a click that picks an object.
            if let Some(session) = state.session.as_ref()
                && orbit
                && button == egui::PointerButton::Primary
                && active_tool(state) == ToolKind::Navigate
                && xf.view == ViewId::Main3D
                && session.doc.mesh.object_count() >= 2
            {
                self.nav_click = Some(NavClick { origin: p.pos, mods: p.mods, camera: session.camera, travel: 0.0 });
            }
            return;
        }
        self.alt = p.mods.alt;
        // Round 4 (FEATURE_TEXT 6.2): the Text tool always places the text of its options (the draft), also while a
        // placed text is selected; the selected text is edited with the Transform tool.
        let Some(slot) = slot_for(active_tool(state), p.mods.alt) else { return };
        if self.tool_mut(slot).press(state, xf, p) {
            self.interaction = Interaction::Tool(xf.view);
            self.captured = Some(slot);
        }
    }

    /// Pointer motion during an interaction.
    pub fn drag(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        match self.interaction {
            Interaction::Idle => {}
            Interaction::Orbit(view) => {
                let delta = p.pos - self.nav_last;
                self.nav_last = p.pos;
                if let Some(click) = self.nav_click.as_mut() {
                    click.travel += delta.length();
                }
                if let Some(session) = state.session.as_mut() {
                    camera_of_mut(session, view).orbit(delta);
                }
            }
            Interaction::Pan(view) => {
                let delta = p.pos - self.nav_last;
                self.nav_last = p.pos;
                let Some(session) = state.session.as_mut() else { return };
                if view == ViewId::Main2D {
                    // Clamped so that the texture stays reachable (R3-11).
                    let tex = set_size(&session.doc, session.active_set);
                    session.uv_view.pan(delta, tex, xf.rect);
                } else {
                    let bounds = session.doc.mesh.bounds;
                    camera_of_mut(session, view).pan(delta, xf.rect.height(), &bounds);
                }
            }
            Interaction::Tool(_) => {
                if let Some(slot) = self.captured {
                    self.tool_mut(slot).drag(state, xf, p);
                }
            }
        }
    }

    /// The pointer button of the interaction went up. A brush stroke whose paint still trails keeps the interaction
    /// (it is "finishing", FEATURE_BRUSH 5.10) until `end_frame` sees that the brush has ended it; the view calls this
    /// again in every frame in which the button is up, which the brush ignores for a released stroke.
    pub fn release(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        if let (Interaction::Tool(_), Some(slot)) = (self.interaction, self.captured) {
            self.tool_mut(slot).release(state, xf, p);
            if slot == Slot::Brush && self.brush.finishing() {
                self.brush_released = true;
                return;
            }
        }
        if let Some(click) = self.nav_click.take()
            && click.travel < NAV_CLICK_TRAVEL
            && self.interaction == Interaction::Orbit(ViewId::Main3D)
        {
            pick_object(state, xf, click);
        }
        self.go_idle();
    }

    /// Ctrl+wheel / Shift+wheel in notches; `true` = consumed by the tool.
    pub fn scroll(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer, notches: f32) -> bool {
        if self.is_busy() || !notches.is_finite() || notches == 0.0 {
            return false;
        }
        match slot_for(active_tool(state), p.mods.alt) {
            Some(slot) => self.tool_mut(slot).scroll(state, xf, p, notches),
            None => false,
        }
    }

    /// Ends the current interaction and returns to Idle: a held stroke is `stroke_cancel`led, a gizmo drag
    /// `cancel_pending_edit`ed. A released brush stroke that is still being painted is the user's completed stroke:
    /// it is completed, not thrown away (FEATURE_BRUSH 5.10). Called by dispatch (the interaction guard, tool / mode /
    /// document change) and by the views (input gate).
    pub fn cancel(&mut self, state: &mut AppState) {
        if self.finishing() {
            self.complete_finishing(state);
            return;
        }
        if let (Interaction::Tool(_), Some(slot)) = (self.interaction, self.captured) {
            self.tool_mut(slot).cancel(state);
        }
        self.go_idle();
    }

    /// Ends the current interaction without committing anything, also a released brush stroke that is still being
    /// painted (its queues are dropped and the stroke is `stroke_cancel`led). Only Undo and Esc do this
    /// (FEATURE_BRUSH 5.10).
    pub fn discard(&mut self, state: &mut AppState) {
        if let (Interaction::Tool(_), Some(slot)) = (self.interaction, self.captured) {
            self.tool_mut(slot).cancel(state);
        }
        self.go_idle();
    }

    /// Ends the current interaction, committing it (`Tool::finish`), and returns to Idle. Called by dispatch for
    /// Save / Save As / Export: the saved or exported state is what the user sees, including a stroke or a gizmo
    /// drag that is still held, and a released stroke that is still being painted (completed first).
    pub fn finish(&mut self, state: &mut AppState) {
        if self.finishing() {
            self.complete_finishing(state);
            return;
        }
        if let (Interaction::Tool(_), Some(slot)) = (self.interaction, self.captured) {
            self.tool_mut(slot).finish(state);
        }
        self.go_idle();
    }

    /// Overlay of the active tool in a view.
    pub fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter) {
        if let Some(slot) = self.current_slot(state) {
            self.tool_ref(slot).overlay(state, xf, painter);
        }
    }

    /// Brush cursor of the active tool.
    pub fn brush_ring(&self, state: &AppState) -> Option<BrushRing> {
        self.current_slot(state).and_then(|slot| self.tool_ref(slot).brush_ring(state))
    }
}

/// objects-tabs (8.6): a Navigate click in the 3D view. The camera goes back to where the press found it, a ray is cast
/// at the press position: on an object a plain click selects it alone, Ctrl or Shift adds or removes it (and the
/// Objects list is asked to show its row); a plain click on nothing deselects. Actions go through `state.push`.
fn pick_object(state: &mut AppState, xf: &ViewXform, click: NavClick) {
    let Some(session) = state.session.as_mut() else { return };
    session.camera = click.camera;
    let xf = ViewXform { camera: click.camera, scene_radius: click.camera.scene_radius(&session.doc.mesh.bounds), ..*xf };
    let add = click.mods.ctrl || click.mods.command || click.mods.shift;
    let action = match xf.surface_at(&session.doc, session.active_set, click.origin) {
        Surface::Model(hit) => {
            let object = session.doc.mesh.tri_object_of(hit.tri);
            session.object_reveal = Some(object);
            if add { AppAction::ToggleObjectSelected(object) } else { AppAction::SelectObject(object) }
        }
        _ if !add => AppAction::DeselectObjects,
        _ => return,
    };
    state.push(action);
}

/// An edit error as a sentence for a toast.
pub(crate) fn error_text(e: &EditError) -> String {
    let text = match e {
        EditError::InvalidParam(msg) => (*msg).to_owned(),
        other => other.to_string(),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The paintable layer of `set` for a brush or fill (step 3 of the Brush row in 8.6): a locked / hidden active layer
/// is a warning and `None`; a stamp layer or no active layer gets a new paint layer (info toast "Created paint layer").
pub(crate) fn paint_layer(state: &mut AppState, set: TextureSetId) -> Option<LayerId> {
    let doc = state.session.as_mut().map(|s| &mut s.doc)?;
    match doc.paint_target(set) {
        PaintTarget::Ready(layer) => Some(layer),
        PaintTarget::Locked(_) => {
            state.toasts.warn("The active layer is locked");
            None
        }
        PaintTarget::Hidden(_) => {
            state.toasts.warn("The active layer is hidden");
            None
        }
        PaintTarget::NotRaster(_) | PaintTarget::NoActiveLayer => match doc.add_raster_layer(set, None) {
            Ok(layer) => {
                state.toasts.info("Created paint layer");
                Some(layer)
            }
            Err(e) => {
                state.toasts.warn(error_text(&e));
                None
            }
        },
    }
}

/// UV wrapped into `[0, 1)` (texture repeat).
pub(crate) fn wrap_uv(uv: Vec2) -> Vec2 {
    Vec2::new(uv.x.rem_euclid(1.0), uv.y.rem_euclid(1.0))
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Headless fixtures for the tool tests: an app state with the sphere sample and the transforms of the two
    //! editing views.

    use super::*;
    use crate::state::{Prefs, Session, UvViewState};
    use wte_core::SampleKind;

    /// App state with an open sample document (the session as `install_document` sets it up).
    pub(crate) fn state_with(kind: SampleKind) -> AppState {
        let mut state = AppState::new(Prefs::default(), true);
        let doc = Document::sample(kind);
        let mut camera = OrbitCamera { yaw: 0.0, pitch: 0.0, ..OrbitCamera::default() };
        camera.frame(doc.mesh.bounds);
        state.session = Some(Session {
            doc,
            path: None,
            force_save_as: false,
            active_set: TextureSetId(0),
            selected_stamp: None,
            camera,
            preview_camera: camera,
            uv_view: UvViewState::default(),
            scene_generation: 1,
            text_gap: None,
            selected_layers: Vec::new(),
            selected_objects: Default::default(),
            object_anchor: None,
            object_selection_rev: 0,
            object_reveal: None,
            sticker_edit: None,
        });
        state
    }

    /// The 3D view at 800x600 points.
    pub(crate) fn xf_3d(state: &mut AppState) -> ViewXform {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 30.0), egui::vec2(800.0, 600.0));
        state.view_rects.insert(ViewId::Main3D, rect);
        ViewXform::for_view(state, ViewId::Main3D).expect("3D transform")
    }

    /// The 2D view at 600x600 points, fitted to the active set.
    pub(crate) fn xf_2d(state: &mut AppState) -> ViewXform {
        let rect = egui::Rect::from_min_size(egui::pos2(820.0, 30.0), egui::vec2(600.0, 600.0));
        state.view_rects.insert(ViewId::Main2D, rect);
        let session = state.session.as_mut().expect("session");
        let tex = set_size(&session.doc, session.active_set);
        session.uv_view.fit(tex, rect);
        ViewXform::for_view(state, ViewId::Main2D).expect("2D transform")
    }

    /// The pointer at `pos` in the view of `xf`, resolved to a surface.
    pub(crate) fn pointer(state: &AppState, xf: &ViewXform, pos: egui::Pos2, mods: egui::Modifiers) -> ViewPointer {
        pointer_p(state, xf, pos, mods, 1.0)
    }

    /// `pointer` with a pen pressure (branch `pen-input`).
    pub(crate) fn pointer_p(
        state: &AppState,
        xf: &ViewXform,
        pos: egui::Pos2,
        mods: egui::Modifiers,
        pressure: f32,
    ) -> ViewPointer {
        let session = state.session.as_ref().expect("session");
        let surface = xf.surface_at(&session.doc, session.active_set, pos);
        ViewPointer { view: xf.view, pos, surface, pressure, mods }
    }

    /// The screen position of a normalized position inside the view.
    pub(crate) fn at(xf: &ViewXform, x: f32, y: f32) -> egui::Pos2 {
        xf.rect.min + egui::vec2(x * xf.rect.width(), y * xf.rect.height())
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use wte_core::SampleKind;

    #[test]
    fn for_view_needs_a_session_and_a_rect() {
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        state.view_rects.insert(ViewId::Main3D, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0)));
        assert!(ViewXform::for_view(&state, ViewId::Main3D).is_none());
        let mut state = state_with(SampleKind::Sphere);
        assert!(ViewXform::for_view(&state, ViewId::Main2D).is_none());
        let xf = xf_2d(&mut state);
        assert!(!xf.is_3d);
        let set = &state.doc().unwrap().sets[0];
        assert_eq!(xf.tex_size, [set.width, set.height]);
        let xf = xf_3d(&mut state);
        assert!(xf.is_3d && xf.scene_radius > 0.0);
    }

    #[test]
    fn surface_at_hits_the_model_and_the_texture() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let doc = state.doc().unwrap();
        let Surface::Model(hit) = xf.surface_at(doc, TextureSetId(0), xf.rect.center()) else { panic!("centre misses") };
        assert_eq!(hit.texture_set, Some(TextureSetId(0)));
        // The hit projects back to the cursor.
        let back = xf.world_to_screen(hit.position).unwrap();
        assert!((back - xf.rect.center()).length() < 0.5);
        assert!(matches!(xf.surface_at(doc, TextureSetId(0), xf.rect.min + egui::vec2(2.0, 2.0)), Surface::Miss));

        let xf = xf_2d(&mut state);
        let doc = state.doc().unwrap();
        let Surface::Uv { set, uv, px } = xf.surface_at(doc, TextureSetId(0), xf.rect.center()) else { panic!("no uv") };
        assert_eq!(set, TextureSetId(0));
        assert!((uv - Vec2::splat(0.5)).length() < 1e-4);
        let back = xf.px_to_screen(px);
        assert!((back - xf.rect.center()).length() < 1e-3);
        assert!(matches!(xf.surface_at(doc, TextureSetId(0), xf.rect.min + egui::vec2(1.0, 1.0)), Surface::Miss));
    }

    #[test]
    fn navigation_routes_by_button_and_tool() {
        let mut state = state_with(SampleKind::Sphere);
        let mut tools = ToolController::new();
        let xf = xf_3d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        let yaw = state.session.as_ref().unwrap().camera.yaw;
        tools.press(&mut state, &xf, &p, egui::PointerButton::Secondary);
        assert_eq!(*tools.interaction(), Interaction::Orbit(ViewId::Main3D));
        let p2 = ViewPointer { pos: p.pos + egui::vec2(50.0, 0.0), ..p.clone() };
        tools.drag(&mut state, &xf, &p2);
        assert!((state.session.as_ref().unwrap().camera.yaw - yaw).abs() > 0.1);
        tools.release(&mut state, &xf, &p2);
        assert!(!tools.is_busy());

        // 2D: right button pans; the UV under the cursor follows it.
        let xf = xf_2d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Secondary);
        assert_eq!(*tools.interaction(), Interaction::Pan(ViewId::Main2D));
        let p2 = ViewPointer { pos: p.pos + egui::vec2(40.0, -30.0), ..p.clone() };
        tools.drag(&mut state, &xf, &p2);
        tools.release(&mut state, &xf, &p2);
        let xf_after = ViewXform::for_view(&state, ViewId::Main2D).unwrap();
        let uv_before = xf.uv.screen_to_uv(p.pos, xf.tex_size, xf.rect);
        let uv_after = xf_after.uv.screen_to_uv(p2.pos, xf_after.tex_size, xf_after.rect);
        assert!((uv_before - uv_after).length() < 1e-5);

        // Navigate tool: the left button orbits in 3D.
        state.tool = ToolKind::Navigate;
        let xf = xf_3d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert_eq!(*tools.interaction(), Interaction::Orbit(ViewId::Main3D));
        tools.cancel(&mut state);
        assert!(!tools.is_busy());
    }

    /// Round 4 (FEATURE_TEXT 6.2, ROUND4_PLAN R4-15; replaces the R3-10 rule): the Text tool always works on the text
    /// to place, also while a placed text is selected. The hover outlines it, Ctrl / Shift+wheel change it, and a press
    /// places it as a second text; the selected text is edited with the Transform tool.
    #[test]
    fn the_text_tool_places_the_draft_while_a_text_is_selected() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE".into();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let first = state.session.as_ref().unwrap().selected_stamp.expect("placed and selected");
        assert_eq!(state.doc().unwrap().stamps.len(), 1);

        // Back to the Text tool with the first text still selected; the draft is now "Bob".
        state.tool = ToolKind::Text;
        state.tools.text.content = "Bob".into();
        let undo = state.doc().unwrap().history.undo_len();
        let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(60.0, 20.0), egui::Modifiers::NONE);
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            tools.overlay(&state, &xf, &painter);
        });
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty(), "the outline of the draft is drawn although a text is selected");
        let em = state.tools.text.em_frac;
        let ctrl = pointer(&state, &xf, p.pos, egui::Modifiers::CTRL);
        assert!(tools.scroll(&mut state, &xf, &ctrl, 2.0), "consumed, not a view zoom");
        assert!(state.tools.text.em_frac > em, "Ctrl+wheel sizes the draft");

        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.doc.stamps.len(), 2, "the press placed the draft");
        assert_eq!(session.doc.history.undo_len(), undo + 1);
        let second = session.selected_stamp.expect("the new text is selected");
        assert_ne!(second, first);
        let text = |id| match &session.doc.stamp(id).unwrap().source {
            wte_core::StampSource::Text(spec) => spec.content.clone(),
            _ => String::new(),
        };
        assert_eq!((text(first), text(second)), ("WTE".to_owned(), "Bob".to_owned()), "the first text is untouched");
        assert!(!state.focus_text_content);
    }

    /// The controller side of the stroke pacing (FEATURE_BRUSH 5.10) with the scaffold's brush (nothing is ever
    /// queued): a release ends the stroke at once, `end_frame` asks for no frame, and `discard` (Undo, Esc) throws a
    /// held stroke away like `cancel`.
    #[test]
    fn the_pacing_controller_keeps_the_round_3_stroke_with_a_brush_that_queues_nothing() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let undo = state.doc().unwrap().history.undo_len();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.begin_frame();
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(tools.is_busy() && !tools.finishing());
        assert_eq!(tools.end_frame(&mut state, 0.0), None);
        assert!(tools.is_busy(), "end_frame keeps a held stroke");
        tools.release(&mut state, &xf, &p);
        assert!(!tools.is_busy() && !tools.finishing());
        assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1);

        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(state.doc().unwrap().stroke_active());
        tools.discard(&mut state);
        assert!(!tools.is_busy() && !state.doc().unwrap().stroke_active());
        assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1, "the discarded stroke left no entry");
    }

    /// objects-tabs (8.6): a Navigate click (travel under 4 pt) in the 3D view of a model with two or more objects
    /// picks the object under the press and puts the camera back; a longer drag is an orbit; Ctrl / Shift toggle; a
    /// plain click on nothing deselects; no pick on a one-object model, in 2D or in another 3D view.
    #[test]
    fn a_navigate_click_picks_an_object_and_a_drag_orbits() {
        use crate::action::AppAction as A;
        let mut state = state_with(SampleKind::Objects);
        state.tool = ToolKind::Navigate;
        state.session.as_mut().unwrap().camera.pitch = 0.7; // from above: the floor lies behind the cube
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let cube = state.doc().unwrap().mesh.objects[1].bounds.center();
        let at = xf.world_to_screen(cube).expect("the cube is in view");
        let Surface::Model(hit) = xf.surface_at(state.doc().unwrap(), TextureSetId(0), at) else { panic!("the cube is hit") };
        assert_eq!(state.doc().unwrap().mesh.tri_object_of(hit.tri), 1);
        let camera = state.session.as_ref().unwrap().camera;

        // `dx` points of travel in two steps, then the release; returns the queued actions.
        let click = |state: &mut AppState, tools: &mut ToolController, pos: egui::Pos2, dx: f32, mods: egui::Modifiers| {
            let xf = ViewXform::for_view(state, ViewId::Main3D).unwrap();
            let p = pointer(state, &xf, pos, mods);
            tools.press(state, &xf, &p, egui::PointerButton::Primary);
            assert_eq!(*tools.interaction(), Interaction::Orbit(ViewId::Main3D));
            for k in 1..=2 {
                let q = ViewPointer { pos: pos + egui::vec2(dx * k as f32 / 2.0, 0.0), ..p.clone() };
                tools.drag(state, &xf, &q);
            }
            let end = ViewPointer { pos: pos + egui::vec2(dx, 0.0), ..p };
            tools.release(state, &xf, &end);
            assert!(!tools.is_busy());
            std::mem::take(&mut state.actions)
        };

        // 3 pt: a click; the camera is restored and the cube's row is to be revealed.
        let a = click(&mut state, &mut tools, at, 3.0, egui::Modifiers::NONE);
        assert!(matches!(a[..], [A::SelectObject(1)]), "{a:?}");
        assert_eq!(state.session.as_ref().unwrap().camera, camera, "the click left the camera where it was");
        assert_eq!(state.session.as_ref().unwrap().object_reveal, Some(1));
        // 5 pt: an orbit, nothing picked.
        let a = click(&mut state, &mut tools, at, 5.0, egui::Modifiers::NONE);
        assert!(a.is_empty(), "{a:?}");
        assert_ne!(state.session.as_ref().unwrap().camera, camera, "the drag orbited");
        state.session.as_mut().unwrap().camera = camera;
        // Ctrl and Shift toggle.
        for mods in [egui::Modifiers::CTRL, egui::Modifiers::SHIFT] {
            let a = click(&mut state, &mut tools, at, 0.0, mods);
            assert!(matches!(a[..], [A::ToggleObjectSelected(1)]), "{mods:?}: {a:?}");
        }
        // On nothing: a plain click deselects, Ctrl does nothing.
        let empty = xf.rect.min + egui::vec2(3.0, 3.0);
        assert!(matches!(xf.surface_at(state.doc().unwrap(), TextureSetId(0), empty), Surface::Miss));
        let a = click(&mut state, &mut tools, empty, 0.0, egui::Modifiers::NONE);
        assert!(matches!(a[..], [A::DeselectObjects]), "{a:?}");
        let a = click(&mut state, &mut tools, empty, 0.0, egui::Modifiers::CTRL);
        assert!(a.is_empty(), "{a:?}");
        // A hidden cube: the ray passes through it.
        state.doc_mut().unwrap().set_hidden_objects(&[false, true, false]).unwrap();
        let a = click(&mut state, &mut tools, at, 0.0, egui::Modifiers::NONE);
        assert!(matches!(a[..], [A::SelectObject(0)]), "the floor behind the hidden cube: {a:?}");
        state.doc_mut().unwrap().set_hidden_objects(&[]).unwrap();
        // Esc (cancel) during the press forgets the click.
        let p = pointer(&state, &xf, at, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        tools.cancel(&mut state);
        tools.release(&mut state, &xf, &p);
        assert!(state.actions.is_empty());

        // Another 3D view (Preview) and the 2D view: no pick.
        let preview = ViewXform { view: ViewId::Preview, ..xf };
        let p = pointer(&state, &preview, at, egui::Modifiers::NONE);
        tools.press(&mut state, &preview, &p, egui::PointerButton::Primary);
        tools.release(&mut state, &preview, &p);
        assert!(state.actions.is_empty(), "{:?}", state.actions);
        let xf2 = xf_2d(&mut state);
        let p = pointer(&state, &xf2, xf2.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf2, &p, egui::PointerButton::Primary);
        assert_eq!(*tools.interaction(), Interaction::Pan(ViewId::Main2D));
        tools.release(&mut state, &xf2, &p);
        assert!(state.actions.is_empty());
        // Space + left button arrives as the middle button: a pan, no pick.
        let p = pointer(&state, &xf, at, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Middle);
        tools.release(&mut state, &xf, &p);
        assert!(state.actions.is_empty());

        // A one-object model: a click is the old zero-length orbit, nothing is picked.
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Navigate;
        let xf = xf_3d(&mut state);
        let a = click(&mut state, &mut tools, xf.rect.center(), 0.0, egui::Modifiers::NONE);
        assert!(a.is_empty(), "{a:?}");
    }

    /// objects-tabs check (plan 3.4): through the tools as the 3D view drives them, the ray of the eyedropper, the fill
    /// seed, a brush stroke, a projected stamp and a wrapped stamp passes through a hidden object and acts on the object
    /// behind it. Three Objects seen from above: the cube stands on the floor; every case is run with the cube shown
    /// (it acts on the cube) and hidden (it acts on the floor under it and never on the cube's texels).
    #[test]
    fn every_tool_ray_passes_through_a_hidden_object() {
        use std::sync::Arc;
        use wte_core::{SampleSource, SurfaceMode, samples};

        /// Changed composite texels per owning object (Floor, Cube, Ball) between two composites of set 0.
        fn changed_per_object(doc: &Document, before: &[u8], after: &[u8]) -> [usize; 3] {
            let set = &doc.sets[0];
            let mut out = [0; 3];
            for y in 0..set.height {
                for x in 0..set.width {
                    let i = ((y * set.width + x) * 4) as usize;
                    if before[i..i + 4] == after[i..i + 4] {
                        continue;
                    }
                    if let Some((tri, _)) = set.texel_map.owner(x, y) {
                        out[doc.mesh.tri_object_of(tri) as usize] += 1;
                    }
                }
            }
            out
        }
        fn composite(state: &mut AppState) -> Vec<u8> {
            let doc = state.doc_mut().unwrap();
            doc.recomposite_all();
            doc.sets[0].composite().data.clone()
        }
        /// A fresh trio seen from above, the cube hidden or not, the tool set; the pointer on the cube's centre.
        fn setup(tool: ToolKind, hidden: bool) -> (AppState, ViewXform, egui::Pos2) {
            let mut state = state_with(SampleKind::Objects);
            state.tool = tool;
            state.session.as_mut().unwrap().camera.pitch = 0.7;
            let xf = xf_3d(&mut state);
            let cube = state.doc().unwrap().mesh.objects[1].bounds.center();
            let at = xf.world_to_screen(cube).expect("the cube is in view");
            state.doc_mut().unwrap().set_hidden_objects(&[false, hidden, false]).unwrap();
            let Surface::Model(hit) = xf.surface_at(state.doc().unwrap(), TextureSetId(0), at) else { panic!("a hit") };
            assert_eq!(state.doc().unwrap().mesh.tri_object_of(hit.tri), if hidden { 0 } else { 1 }, "hidden {hidden}");
            (state, xf, at)
        }
        fn click(state: &mut AppState, tools: &mut ToolController, xf: &ViewXform, at: egui::Pos2) {
            let p = pointer(state, xf, at, egui::Modifiers::NONE);
            tools.begin_frame();
            tools.press(state, xf, &p, egui::PointerButton::Primary);
            tools.release(state, xf, &p);
        }

        for hidden in [false, true] {
            let target = if hidden { 0 } else { 1 };

            // Eyedropper: the cube's top is painted red; hidden, the colour comes from the floor under it.
            let (mut state, xf, at) = setup(ToolKind::Eyedropper, hidden);
            let Surface::Model(cube_hit) = ({
                let doc = state.doc_mut().unwrap();
                doc.set_hidden_objects(&[false; 3]).unwrap();
                let s = xf.surface_at(doc, TextureSetId(0), at);
                doc.set_hidden_objects(&[false, hidden, false]).unwrap();
                s
            }) else {
                panic!("the cube is hit")
            };
            let layer = paint_layer(&mut state, TextureSetId(0)).unwrap();
            let red = [255, 0, 0, 255];
            state.doc_mut().unwrap().fill(TextureSetId(0), layer, cube_hit.uv.unwrap(), red, wte_core::FillScope::Island).unwrap();
            state.doc_mut().unwrap().recomposite_all();
            let mut tools = ToolController::new();
            click(&mut state, &mut tools, &xf, at);
            let p = pointer(&state, &xf, at, egui::Modifiers::NONE);
            let (set, uv) = p.surface.set_uv().unwrap();
            let floor_colour = state.doc().unwrap().sample_color(set, uv, SampleSource::Composite).unwrap();
            let picked = match state.actions.as_slice() {
                [AppAction::SetPrimaryColor(c)] => *c,
                other => panic!("eyedropper queued {other:?}"),
            };
            if hidden {
                assert_eq!(picked, floor_colour, "the floor's colour");
                assert_ne!(picked, red, "not the hidden cube's red");
            } else {
                assert_eq!(picked, red, "the cube's red");
            }

            // Fill (Island scope from the 3D view): the seed is the object hit.
            let (mut state, xf, at) = setup(ToolKind::Fill, hidden);
            state.tools.fill_scope = wte_core::FillScope::Island;
            state.colors.primary = [10, 200, 30, 255];
            let before = composite(&mut state);
            click(&mut state, &mut ToolController::new(), &xf, at);
            let after = composite(&mut state);
            let changed = changed_per_object(state.doc().unwrap(), &before, &after);
            assert!(changed[target] > 0 && changed.iter().sum::<usize>() == changed[target], "fill, hidden {hidden}: {changed:?}");

            // Brush: a short stroke over the cube's centre paints the object the ray hits.
            let (mut state, xf, at) = setup(ToolKind::Brush, hidden);
            state.colors.primary = [0, 0, 255, 255];
            let before = composite(&mut state);
            let mut tools = ToolController::new();
            let p = pointer(&state, &xf, at, egui::Modifiers::NONE);
            tools.begin_frame();
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            for dx in [2.0, 4.0] {
                tools.begin_frame();
                let q = pointer(&state, &xf, at + egui::vec2(dx, 0.0), egui::Modifiers::NONE);
                tools.drag(&mut state, &xf, &q);
            }
            let q = pointer(&state, &xf, at + egui::vec2(4.0, 0.0), egui::Modifiers::NONE);
            tools.release(&mut state, &xf, &q);
            let mut frames = 0;
            while tools.is_busy() && frames < 100_000 {
                tools.begin_frame();
                tools.end_frame(&mut state, 0.0);
                frames += 1;
            }
            let after = composite(&mut state);
            let changed = changed_per_object(state.doc().unwrap(), &before, &after);
            assert!(changed[target] > 0, "brush, hidden {hidden}: {changed:?}");
            if hidden {
                assert_eq!(changed[1], 0, "no paint on the hidden cube: {changed:?}");
            }

            // A projected and a wrapped image stamp placed at the cube's centre.
            for mode in [SurfaceMode::Project, SurfaceMode::Wrap] {
                let (mut state, xf, at) = setup(ToolKind::Stamp, hidden);
                let img = samples::sample_stamp_image(64);
                let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
                let blob = state.doc_mut().unwrap().add_image_blob("ring.png", Arc::from(png)).unwrap();
                state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
                state.tools.stamp.source_name = "ring.png".to_owned();
                state.tools.stamp.mode = mode;
                let before = composite(&mut state);
                click(&mut state, &mut ToolController::new(), &xf, at);
                assert!(state.session.as_ref().unwrap().selected_stamp.is_some(), "{mode:?} placed");
                let after = composite(&mut state);
                let changed = changed_per_object(state.doc().unwrap(), &before, &after);
                assert!(changed[target] > 0, "{mode:?}, hidden {hidden}: {changed:?}");
                if hidden {
                    assert_eq!(changed[1], 0, "{mode:?}: nothing on the hidden cube: {changed:?}");
                }
            }
        }
    }

    #[test]
    fn error_text_is_a_sentence() {
        assert_eq!(error_text(&EditError::LayerLocked), "Layer is locked");
        assert_eq!(error_text(&EditError::InvalidParam("no UV island here")), "No UV island here");
    }
}
