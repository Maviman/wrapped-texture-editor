//! The central area (ARCHITECTURE.md 8.5): view-mode strip, layout of the 3D / 2D / Split / Preview views, pointer
//! routing into navigation and tools, rendering through the GPU layer and the egui-painter overlays.

use eframe::egui;
use glam::{Vec2, Vec3};
use wte_core::{Document, Placement, Projector, SurfaceMode, TextureSetId, UvPlacement};

use crate::action::AppAction;
use crate::gpu::{Gpu, GpuScene, Renderer, ViewTargets};
use crate::state::{AppState, Dialog, ShadingMode, ToolKind, ViewId, ViewMode};
use crate::tools::{Surface, ToolController, ViewPointer, ViewXform, set_size, wrap_uv};

pub mod camera;
pub mod gizmo;
pub mod preview;
pub mod view2d;
pub mod view3d;

pub use preview::{preview_widget, show_preview_mode};
pub use view2d::show_2d;
pub use view3d::show_3d;

/// Height of the view-mode strip above the views (points).
const MODE_STRIP_HEIGHT: f32 = 26.0;
/// Width of the Split divider (points).
const DIVIDER_WIDTH: f32 = 6.0;
/// Limits of `split_ratio`.
const SPLIT_LIMITS: (f32, f32) = (0.15, 0.85);
/// Text of the empty central area.
const EMPTY_HINT: &str = "File > New from Model, or drop a model / .wtep file here";

/// Everything a view needs for one frame.
pub struct ViewCx<'a> {
    pub state: &'a mut AppState,
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
    pub scene: &'a mut Option<GpuScene>,
    pub targets: &'a mut ViewTargets,
    pub tools: &'a mut ToolController,
}

/// Tab strip (3D | 2D | Split | Preview), layout, input, rendering and overlays of the central area.
pub fn central(ui: &mut egui::Ui, cx: &mut ViewCx<'_>) {
    central_views(ui, cx);
    // A wheel gesture of the Transform tool ends once the wheel has been idle for a moment (one history entry); a
    // brush stroke with queued work asks for its next frame at once (stroke pacing, FEATURE_BRUSH 5.10).
    let now = ui.input(|i| i.time);
    if let Some(wait) = cx.tools.end_frame(cx.state, now) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.0)));
    }
    // A deferred stamp Preview bake that the throttle held back runs in a later frame, even without input.
    if let Some(wake) = cx.state.preview_bakes.wake.take() {
        ui.ctx().request_repaint_after(wake);
    }
}

fn central_views(ui: &mut egui::Ui, cx: &mut ViewCx<'_>) {
    let full = ui.available_rect_before_wrap();
    cx.state.view_rects.clear();
    cx.state.sticker_canvas_rect = None;
    cx.state.hover = Default::default();
    cx.tools.begin_frame();
    cx.tools.set_clock(ui.input(|i| i.time));

    let strip = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), MODE_STRIP_HEIGHT));
    let area = egui::Rect::from_min_max(egui::pos2(full.min.x, strip.max.y), full.max);
    mode_strip(ui, cx.state, strip);
    if area.width() < 1.0 || area.height() < 1.0 {
        return;
    }

    if cx.state.session.is_none() {
        empty_area(ui, cx, area);
        return;
    }
    // Input gate (8.5): a modal job or an open dialog keeps the editing views from seeing the pointer.
    if input_gated(cx.state) && cx.tools.is_busy() {
        cx.tools.cancel(cx.state);
    }

    match cx.state.view_mode {
        ViewMode::Preview => show_preview_mode(ui, cx, area),
        ViewMode::View3D => show_3d(ui, cx, area),
        ViewMode::View2D => show_2d(ui, cx, area),
        ViewMode::Sticker => crate::sticker::canvas::show(ui, cx, area),
        ViewMode::Split => {
            let (left, divider, right) = split_rects(area, cx.state.split_ratio);
            // Phase A: input of both views, so neither lags the other by a frame.
            view3d::input(ui, cx, left);
            let image = view2d::input(ui, cx, right);
            split_divider(ui, &mut cx.state.split_ratio, area, divider);
            // Phase B: one scene sync, then both renders.
            if sync_scene(cx, false) {
                view3d::render(ui, cx, left);
                view2d::render(ui, cx, image);
            }
        }
    }
}

/// While a modal job runs or a dialog is open, the editing views take no pointer input (8.5).
pub(crate) fn input_gated(state: &AppState) -> bool {
    state.busy.as_ref().is_some_and(|b| b.modal) || !matches!(state.dialog, Dialog::None)
}

/// `(3D rect, divider rect, 2D rect)` of the Split layout (stickers P6a: also `(canvas, divider, 3D)` of the Sticker
/// view).
pub(crate) fn split_rects(area: egui::Rect, ratio: f32) -> (egui::Rect, egui::Rect, egui::Rect) {
    let ratio = if ratio.is_finite() { ratio.clamp(SPLIT_LIMITS.0, SPLIT_LIMITS.1) } else { 0.5 };
    let x = (area.min.x + area.width() * ratio).round();
    let half = DIVIDER_WIDTH * 0.5;
    let left = egui::Rect::from_min_max(area.min, egui::pos2(x - half, area.max.y));
    let divider = egui::Rect::from_min_max(egui::pos2(x - half, area.min.y), egui::pos2(x + half, area.max.y));
    let right = egui::Rect::from_min_max(egui::pos2(x + half, area.min.y), area.max);
    (left, divider, right)
}

/// The draggable divider of the Split layout (stickers P6a: also of the Sticker view); `ratio` is the left share.
pub(crate) fn split_divider(ui: &mut egui::Ui, ratio: &mut f32, area: egui::Rect, divider: egui::Rect) {
    let response = ui.allocate_rect(divider, egui::Sense::DRAG).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        let delta = response.drag_delta().x / area.width().max(1.0);
        *ratio = (*ratio + delta).clamp(SPLIT_LIMITS.0, SPLIT_LIMITS.1);
    }
    let visuals = ui.visuals();
    let color = if response.hovered() || response.dragged() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.noninteractive.bg_stroke.color
    };
    ui.painter().rect_filled(divider, 0.0, visuals.panel_fill);
    ui.painter().rect_filled(divider.shrink2(egui::vec2(2.0, 0.0)), 0.0, color);
}

/// The view-mode tabs and the quick display toggles above the views.
fn mode_strip(ui: &mut egui::Ui, state: &mut AppState, strip: egui::Rect) {
    ui.painter().rect_filled(strip, 0.0, ui.visuals().panel_fill);
    let has_doc = state.session.is_some();
    let builder = egui::UiBuilder::new()
        .max_rect(strip.shrink2(egui::vec2(6.0, 0.0)))
        .layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let tabs = [
            (ViewMode::View3D, "3D", "3D view"),
            (ViewMode::View2D, "2D", "2D (UV) view of the texture"),
            (ViewMode::Split, "Split", "3D and 2D side by side"),
            (ViewMode::Preview, "Preview", "Clean preview of the result before export"),
            (ViewMode::Sticker, "Sticker", "Edit the selected stamp as a layered sticker, with the model beside it"),
        ];
        // Stickers P6a: the Sticker tab needs a selected stamp (or the open editor).
        let can_sticker = state.view_mode == ViewMode::Sticker
            || state.session.as_ref().is_some_and(|s| s.selected_stamp.is_some_and(|id| s.doc.stamp(id).is_some()));
        for (mode, label, tip) in tabs {
            let shortcut = crate::shortcuts::shortcut_text(ui.ctx(), label);
            let tip = if shortcut.is_empty() { tip.to_owned() } else { format!("{tip} ({shortcut})") };
            let enabled = match mode {
                ViewMode::Preview => has_doc,
                ViewMode::Sticker => can_sticker,
                _ => true,
            };
            let response = ui
                .add_enabled(enabled, egui::Button::selectable(state.view_mode == mode, label))
                .on_hover_text(tip)
                .on_disabled_hover_text(crate::sticker::NO_STAMP_HINT);
            if response.clicked() && state.view_mode != mode {
                state.push(AppAction::SetViewMode(mode));
            }
        }
        if state.view_mode == ViewMode::Preview || !has_doc {
            return;
        }
        ui.separator();
        let toggles = display_toggles(ui.ctx(), state);
        // The toggles stay one row of buttons while they fit; in a narrow central area (small window, wide side panel)
        // they move into a "Display" drop-down so none of them ends up under the side panel.
        let spacing = ui.spacing().item_spacing.x;
        let padding = ui.spacing().button_padding.x * 2.0;
        let needed: f32 = toggles
            .iter()
            .map(|t| {
                let galley = egui::WidgetText::from(t.label).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
                galley.size().x + padding + spacing
            })
            .sum();
        if needed <= ui.available_width() {
            for t in toggles {
                let button = match t.selected {
                    Some(on) => egui::Button::selectable(on, t.label),
                    None => egui::Button::new(t.label),
                };
                if ui.add(button).on_hover_text(t.tip).clicked() {
                    state.push(t.action);
                }
            }
        } else {
            ui.menu_button("Display", |ui| {
                for t in toggles {
                    let button = match t.selected {
                        Some(on) => egui::Button::selectable(on, t.label),
                        None => egui::Button::new(t.label),
                    };
                    if ui.add(button).on_hover_text(t.tip).clicked() {
                        state.push(t.action);
                    }
                }
            })
            .response
            .on_hover_text("Display options of the views");
        }
    });
}

/// One quick display toggle of the view-mode strip.
struct DisplayToggle {
    label: &'static str,
    /// `None` for a plain button (Frame).
    selected: Option<bool>,
    tip: String,
    action: AppAction,
}

/// The display toggles of the current view mode, in strip order.
fn display_toggles(ctx: &egui::Context, state: &AppState) -> Vec<DisplayToggle> {
    let mut out = Vec::new();
    let show_3d_toggles = matches!(state.view_mode, ViewMode::View3D | ViewMode::Split | ViewMode::Sticker);
    let show_2d_toggles = matches!(state.view_mode, ViewMode::View2D | ViewMode::Split);
    if show_3d_toggles {
        let shortcut = crate::shortcuts::shortcut_text(ctx, "Texture on Model");
        out.push(DisplayToggle {
            label: "Texture",
            selected: Some(state.flags.shading == ShadingMode::Textured),
            tip: format!("Show the texture on the model, or plain clay ({shortcut})"),
            action: AppAction::ToggleTexture,
        });
        let checker = state.flags.shading == ShadingMode::UvChecker;
        out.push(DisplayToggle {
            label: "UV checker",
            selected: Some(checker),
            tip: "Show a UV test pattern on the model to judge the wrapping".to_owned(),
            action: AppAction::SetShading(if checker { ShadingMode::Textured } else { ShadingMode::UvChecker }),
        });
        let shortcut = crate::shortcuts::shortcut_text(ctx, "Wireframe");
        out.push(DisplayToggle {
            label: "Wireframe",
            selected: Some(state.flags.wireframe),
            tip: format!("Show all mesh edges ({shortcut})"),
            action: AppAction::ToggleWireframe,
        });
        out.push(DisplayToggle {
            label: "Seams",
            selected: Some(state.flags.seams),
            tip: "Show UV seams (orange) and open borders (cyan)".to_owned(),
            action: AppAction::ToggleSeams,
        });
    }
    if show_2d_toggles {
        out.push(DisplayToggle {
            label: "UV wire",
            selected: Some(state.flags.uv_wire),
            tip: "Show the UV layout over the texture".to_owned(),
            action: AppAction::ToggleUvWire,
        });
    }
    let shortcut = crate::shortcuts::shortcut_text(ctx, "Frame Model");
    out.push(DisplayToggle {
        label: "Frame",
        selected: None,
        tip: format!("Frame the model / fit the texture ({shortcut})"),
        action: AppAction::FrameModel,
    });
    out
}

/// Test access to the view-mode strip (control sweep).
#[cfg(test)]
pub(crate) fn mode_strip_for_tests(ui: &mut egui::Ui, state: &mut AppState, strip: egui::Rect) {
    mode_strip(ui, state, strip);
}

/// The central area without a document: backdrop and hint.
fn empty_area(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, area: egui::Rect) {
    cx.state.view_rects.insert(ViewId::Main3D, area);
    ui.allocate_rect(area, egui::Sense::HOVER);
    let target = &mut cx.targets.main3d;
    target.ensure_size(cx.gpu, target_px(ui.ctx(), area));
    cx.renderer.render_backdrop(cx.gpu, target, cx.state.flags.backdrop);
    ui.painter().image(target.egui_id, area, target.uv_rect(), egui::Color32::WHITE);
    let font = egui::FontId::proportional(16.0);
    ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, EMPTY_HINT, font, egui::Color32::GRAY);
}

/// Physical pixel size of a view rect.
pub(crate) fn target_px(ctx: &egui::Context, rect: egui::Rect) -> [u32; 2] {
    let size = (rect.size() * ctx.pixels_per_point()).round();
    [size.x.max(1.0) as u32, size.y.max(1.0) as u32]
}

/// Ensures the `GpuScene` of the current document and syncs it (8.4); stores `state.gpu_mips_stale`. `false` without
/// a document.
pub(crate) fn sync_scene(cx: &mut ViewCx<'_>, force_mips: bool) -> bool {
    let _p = crate::app::frame_profile::scope("scene sync");
    let AppState { session, preview_bakes, .. } = &mut *cx.state;
    let Some(session) = session.as_mut() else { return false };
    bake_stale_previews(&mut session.doc, preview_bakes);
    if cx.scene.as_ref().is_none_or(|s| s.generation != session.scene_generation) {
        *cx.scene = Some(GpuScene::new(cx.gpu, cx.renderer, &mut session.doc, session.scene_generation));
    }
    let Some(scene) = cx.scene.as_mut() else { return false };
    scene.sync(cx.gpu, cx.renderer, &mut session.doc, force_mips);
    // objects-tabs (8.4): the visible index buffer / wire ranges and the highlight follow visibility and selection.
    scene.sync_objects(cx.gpu, &session.doc, &session.selected_objects, session.object_selection_rev);
    cx.state.gpu_mips_stale = scene.mips_stale();
    true
}

/// Runs the deferred stamp Preview bakes of the document (`Document::set_defer_preview_bakes`, finding SP-1) when
/// the throttle allows it; otherwise the clock's `wake` asks for a repaint once it does.
pub(crate) fn bake_stale_previews(doc: &mut wte_core::Document, clock: &mut crate::state::PreviewBakeClock) {
    if !doc.has_stale_previews() {
        return;
    }
    let start = std::time::Instant::now();
    if clock.due(start) {
        doc.bake_stale_previews();
        clock.record(start, std::time::Instant::now());
    }
}

/// Notches of one raw wheel event (8.6).
pub(crate) fn wheel_notches(unit: egui::MouseWheelUnit, delta: egui::Vec2) -> f32 {
    // Some platforms turn Shift+wheel into a horizontal delta; the notch count is the same.
    let d = if delta.y != 0.0 { delta.y } else { delta.x };
    match unit {
        egui::MouseWheelUnit::Line => d,
        egui::MouseWheelUnit::Point => d / 50.0,
        egui::MouseWheelUnit::Page => d * 10.0,
    }
}

/// Raw wheel events of this pass: `(notches, modifiers)`.
pub(crate) fn wheel_events(ui: &egui::Ui) -> Vec<(f32, egui::Modifiers)> {
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some((wheel_notches(*unit, *delta), *modifiers)),
                _ => None,
            })
            .filter(|(n, _)| n.is_finite() && *n != 0.0)
            .collect()
    })
}

/// The pointer events of this pass that the routing needs, in order, each with its pen pressure (1.0 for a mouse;
/// branch `pen-input`, `PenFrame::cursor`).
enum PointerEvent {
    Moved(egui::Pos2, f32),
    Pressed(egui::Pos2, egui::PointerButton, egui::Modifiers, f32),
    Released(egui::Pos2, egui::PointerButton, f32),
}

/// Pointer routing of an editing view (8.6), identical for `Main3D` and `Main2D`: presses, every motion event of a
/// drag, releases, wheel and hover. `set` is the texture set a 2D view shows.
pub(crate) fn route_pointer(ui: &egui::Ui, cx: &mut ViewCx<'_>, view: ViewId, rect: egui::Rect, response: &egui::Response) {
    let _p = crate::app::frame_profile::scope("pointer/tools/hover");
    // Branch `pen-input`: every pointer event takes its pressure from this frame's pen notes, in order (one walker per
    // view, so Split works); calls without an event of their own take the held contact pressure.
    let pen = &cx.state.pen.frame;
    let held = pen.held_pressure();
    let (events, space, mods, latest, down) = ui.input(|i| {
        let mut walk = pen.cursor();
        let events: Vec<PointerEvent> = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerMoved(pos) => Some(PointerEvent::Moved(*pos, walk.take(*pos))),
                egui::Event::PointerButton { pos, button, pressed: true, modifiers } => {
                    Some(PointerEvent::Pressed(*pos, *button, *modifiers, walk.take(*pos)))
                }
                egui::Event::PointerButton { pos, button, pressed: false, .. } => {
                    Some(PointerEvent::Released(*pos, *button, walk.take(*pos)))
                }
                _ => None,
            })
            .collect();
        let down = [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle]
            .map(|b| (b, i.pointer.button_down(b)));
        (events, i.key_down(egui::Key::Space), i.modifiers, i.pointer.latest_pos(), down)
    });
    let gated = input_gated(cx.state);
    let contains = response.contains_pointer();
    let mine = |tools: &ToolController| tools.interaction().view() == Some(view);

    let mut moved = false;
    if !gated {
        for event in events {
            match event {
                PointerEvent::Moved(pos, pressure) if mine(cx.tools) => {
                    if let Some((xf, p)) = pointer_at(cx.state, view, pos, mods, pressure) {
                        cx.tools.drag(cx.state, &xf, &p);
                        moved = true;
                    }
                }
                PointerEvent::Moved(..) => {}
                PointerEvent::Pressed(pos, button, modifiers, pressure) => {
                    if contains && rect.contains(pos) {
                        // A released brush stroke that is still being painted is completed first, so the press is not
                        // lost to the busy check (FEATURE_BRUSH 5.10).
                        cx.tools.complete_finishing(cx.state);
                    }
                    if cx.tools.is_busy() || !contains || !rect.contains(pos) {
                        continue;
                    }
                    let logical =
                        if button == egui::PointerButton::Primary && space { egui::PointerButton::Middle } else { button };
                    if let Some((xf, p)) = pointer_at(cx.state, view, pos, modifiers, pressure) {
                        cx.tools.press(cx.state, &xf, &p, logical);
                        if mine(cx.tools) {
                            cx.tools.set_drag_button(button);
                        }
                    }
                }
                PointerEvent::Released(pos, button, pressure) => {
                    if mine(cx.tools)
                        && cx.tools.drag_button() == button
                        && let Some((xf, p)) = pointer_at(cx.state, view, pos, mods, pressure)
                    {
                        cx.tools.release(cx.state, &xf, &p);
                    }
                }
            }
        }
        if mine(cx.tools)
            && let Some(pos) = latest
            && let Some((xf, p)) = pointer_at(cx.state, view, pos, mods, held)
        {
            // A drag without motion this frame still continues (the brush carries its raycast budget over).
            if !moved {
                cx.tools.drag(cx.state, &xf, &p);
            }
            // The release happened where we did not see it (outside the window).
            let button = cx.tools.drag_button();
            if !down.iter().any(|(b, d)| *b == button && *d) {
                cx.tools.release(cx.state, &xf, &p);
            }
        }

        // Wheel: Ctrl / Shift go to the tool first; everything else zooms the view.
        if contains {
            for (notches, modifiers) in wheel_events(ui) {
                let Some(pos) = latest else { break };
                let Some((xf, p)) = pointer_at(cx.state, view, pos, modifiers, held) else { break };
                let tool_first = modifiers.ctrl || modifiers.command || modifiers.shift;
                if tool_first && cx.tools.scroll(cx.state, &xf, &p, notches) {
                    continue;
                }
                zoom_view(cx.state, &xf, pos, notches);
            }
        }
    }

    // Hover: one surface lookup per frame, fills `state.hover` and feeds the tool.
    if let Some(pos) = latest
        && rect.contains(pos)
        && (contains || mine(cx.tools))
        && let Some((xf, p)) = pointer_at(cx.state, view, pos, mods, held)
    {
        fill_hover(cx.state, &xf, &p);
        if !gated && !cx.tools.is_busy() {
            cx.tools.hover(cx.state, &xf, &p);
            hover_cursor(ui, cx.state);
        }
    }
    if mine(cx.tools) {
        let icon = match cx.tools.interaction() {
            crate::tools::Interaction::Orbit(_) | crate::tools::Interaction::Pan(_) => Some(egui::CursorIcon::Grabbing),
            _ => None,
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
    }
}

/// Cursor shape over an editing view for the active tool.
fn hover_cursor(ui: &egui::Ui, state: &AppState) {
    let icon = match crate::tools::active_tool(state) {
        ToolKind::Navigate => egui::CursorIcon::Grab,
        ToolKind::Transform => return, // the transform tool sets its own cursor over the gizmo
        _ => egui::CursorIcon::Crosshair,
    };
    ui.ctx().set_cursor_icon(icon);
}

/// The view transform and the resolved pointer at `pos` with its pen pressure (1.0 for a mouse).
fn pointer_at(
    state: &AppState,
    view: ViewId,
    pos: egui::Pos2,
    mods: egui::Modifiers,
    pressure: f32,
) -> Option<(ViewXform, ViewPointer)> {
    let xf = ViewXform::for_view(state, view)?;
    let session = state.session.as_ref()?;
    let surface = xf.surface_at(&session.doc, session.active_set, pos);
    Some((xf, ViewPointer { view, pos, surface, pressure, mods }))
}

/// Wheel zoom of an editing view: the 3D camera distance, or the 2D zoom about the cursor.
fn zoom_view(state: &mut AppState, xf: &ViewXform, pos: egui::Pos2, notches: f32) {
    let Some(session) = state.session.as_mut() else { return };
    if xf.is_3d {
        let radius = session.doc.mesh.bounds.diagonal() * 0.5;
        crate::tools::camera_of_mut(session, xf.view).zoom(notches, radius);
    } else {
        let tex = set_size(&session.doc, session.active_set);
        session.uv_view.zoom_about(pos, (0.15 * notches).exp(), tex, xf.rect);
    }
}

/// `state.hover` for the pointer over a view.
fn fill_hover(state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
    let Some(session) = state.session.as_ref() else { return };
    let doc = &session.doc;
    let mut hover = crate::state::HoverInfo { view: Some(xf.view), ..Default::default() };
    match &p.surface {
        Surface::Model(hit) => {
            hover.world = Some(hit.position.to_array());
            hover.set = hit.texture_set;
        }
        Surface::Uv { set, px, .. } => {
            hover.set = Some(*set);
            hover.world = locate_px(doc, *set, *px).map(|v| v.to_array());
        }
        Surface::Miss => {}
    }
    if let Some((set, uv)) = p.surface.set_uv() {
        let uv = wrap_uv(uv);
        let [w, h] = set_size(doc, set);
        hover.uv = Some(uv.to_array());
        hover.texel = Some([((uv.x * w as f32) as u32).min(w - 1), ((uv.y * h as f32) as u32).min(h - 1)]);
        hover.rgba = doc.sample_color(set, uv, wte_core::SampleSource::Composite);
    }
    state.hover = hover;
}

/// The surface point (world) of continuous texel coordinates of a set, if a visible triangle owns that texel.
pub(crate) fn locate_px(doc: &Document, set: TextureSetId, px: Vec2) -> Option<Vec3> {
    let s = doc.set(set)?;
    if !px.is_finite() || s.width == 0 || s.height == 0 {
        return None;
    }
    let x = (px.x.floor() as i64).rem_euclid(s.width as i64) as u32;
    let y = (px.y.floor() as i64).rem_euclid(s.height as i64) as u32;
    // objects-tabs: a texel owned by a hidden object has no visible surface point (outline traces, the brush ring of a
    // 2D cursor and the hover position skip it).
    s.texel_map.locate(&doc.mesh, x, y).filter(|sp| !doc.object_hidden(doc.mesh.tri_object_of(sp.tri))).map(|sp| sp.position)
}

/// Strokes of overlay outlines: a dark shadow under a light line.
pub(crate) fn outline_strokes(color: egui::Color32, width: f32) -> (egui::Stroke, egui::Stroke) {
    (egui::Stroke::new(width + 2.0, egui::Color32::from_black_alpha(150)), egui::Stroke::new(width, color))
}

/// Draws polylines with the outline strokes.
pub(crate) fn draw_polylines(painter: &egui::Painter, lines: &[Vec<egui::Pos2>], color: egui::Color32, width: f32) {
    let (shadow, line) = outline_strokes(color, width);
    for stroke in [shadow, line] {
        for points in lines.iter().filter(|l| l.len() >= 2) {
            painter.add(egui::Shape::line(points.clone(), stroke));
        }
    }
}

/// Draws a closed quad outline.
pub(crate) fn draw_quad(painter: &egui::Painter, corners: [egui::Pos2; 4], color: egui::Color32, width: f32) {
    let (shadow, line) = outline_strokes(color, width);
    for stroke in [shadow, line] {
        painter.add(egui::Shape::closed_line(corners.to_vec(), stroke));
    }
}

/// Points per edge when an outline is traced across the surface.
const OUTLINE_SAMPLES: usize = 24;

/// Outline of a UV-placed stamp on the model (3D view): the placement rectangle is sampled in texel space and every
/// sample is located on the surface through the texel map; consecutive samples that both land on the model are joined.
pub(crate) fn uv_outline_in_3d(doc: &Document, p: &UvPlacement, xf: &ViewXform) -> Vec<Vec<egui::Pos2>> {
    let corners = p.corners_px();
    let max_jump = xf.rect.size().max_elem() * 0.25;
    let mut lines = Vec::new();
    let mut current: Vec<egui::Pos2> = Vec::new();
    for edge in 0..4 {
        let (a, b) = (corners[edge], corners[(edge + 1) % 4]);
        for k in 0..=OUTLINE_SAMPLES {
            let px = a.lerp(b, k as f32 / OUTLINE_SAMPLES as f32);
            let screen = locate_px(doc, p.set, px).and_then(|w| xf.world_to_screen(w));
            push_outline_point(&mut lines, &mut current, screen, max_jump);
        }
    }
    if current.len() >= 2 {
        lines.push(current);
    }
    lines
}

/// Outline of a projected stamp on the texture (2D view of `set`): the projector rectangle is sampled in world space,
/// each sample is projected onto the surface along the projector axis, and the hit's UV gives the texture position.
/// Samples landing in other sets or across a UV seam break the line.
pub(crate) fn projected_outline_in_2d(doc: &Document, p: &Projector, set: TextureSetId, xf: &ViewXform) -> Vec<Vec<egui::Pos2>> {
    let corners = p.corners();
    let (_, _, out) = p.axes();
    let reach = p.depth_front.abs().max(p.half_size.max_element()).max(1e-6);
    let size = set_size(doc, set);
    let tex = Vec2::new(size[0] as f32, size[1] as f32);
    let max_jump = xf.rect.size().max_elem() * 0.25;
    let mut lines = Vec::new();
    let mut current: Vec<egui::Pos2> = Vec::new();
    for edge in 0..4 {
        let (a, b) = (corners[edge], corners[(edge + 1) % 4]);
        for k in 0..=OUTLINE_SAMPLES {
            let world = a.lerp(b, k as f32 / OUTLINE_SAMPLES as f32);
            let ray = wte_core::Ray::new(world + out * reach, -out);
            let screen = doc
                .raycast(&ray)
                .filter(|hit| hit.texture_set == Some(set) && hit.t <= 2.0 * reach + p.depth_back.abs())
                .and_then(|hit| hit.uv)
                .map(|uv| xf.px_to_screen(uv * tex));
            push_outline_point(&mut lines, &mut current, screen, max_jump);
        }
    }
    if current.len() >= 2 {
        lines.push(current);
    }
    lines
}

/// Appends a traced outline sample; a missing sample or a jump (seam) starts a new polyline.
fn push_outline_point(lines: &mut Vec<Vec<egui::Pos2>>, current: &mut Vec<egui::Pos2>, p: Option<egui::Pos2>, max_jump: f32) {
    match p {
        Some(p) if current.last().is_none_or(|last| last.distance(p) <= max_jump) => current.push(p),
        Some(p) => {
            if current.len() >= 2 {
                lines.push(std::mem::take(current));
            }
            current.clear();
            current.push(p);
        }
        None => {
            if current.len() >= 2 {
                lines.push(std::mem::take(current));
            }
            current.clear();
        }
    }
}

/// Round 6 (R6-2): outline of wrapped stamp `id` on the texture (2D view of `set`): the picture's border on the
/// surface (`Document::stamp_wrap_outline`) mapped through each point's triangle UVs to texels; points on triangles of
/// other sets and jumps across a UV seam break the line. Empty without a cached parameterization.
pub(crate) fn wrapped_outline_in_2d(doc: &Document, id: wte_core::StampId, set: TextureSetId, xf: &ViewXform) -> Vec<Vec<egui::Pos2>> {
    let size = set_size(doc, set);
    let tex = Vec2::new(size[0] as f32, size[1] as f32);
    let max_jump = xf.rect.size().max_elem() * 0.25;
    let mut lines = Vec::new();
    for run in doc.stamp_wrap_outline(id, gizmo::WRAP_OUTLINE_SAMPLES) {
        let mut current: Vec<egui::Pos2> = Vec::new();
        for sp in run {
            let screen = (doc.mesh.tri_set(sp.tri) == Some(set)).then(|| {
                let uv = doc.mesh.tri_uvs(sp.tri);
                let at = uv[0] * sp.bary[0] + uv[1] * sp.bary[1] + uv[2] * sp.bary[2];
                xf.px_to_screen(at * tex)
            });
            push_outline_point(&mut lines, &mut current, screen.filter(|p| p.is_finite()), max_jump);
        }
        if current.len() >= 2 {
            lines.push(current);
        }
    }
    lines
}

/// Outline of the selected stamp in a view whose gizmo the Transform tool does not draw (other tools, or the view that
/// does not match the placement kind, 8.6).
pub(crate) fn selected_outline(state: &AppState, xf: &ViewXform, painter: &egui::Painter) {
    let Some(session) = state.session.as_ref() else { return };
    let Some(stamp) = session.selected_stamp.and_then(|id| session.doc.stamp(id)) else { return };
    let gizmo_here = crate::tools::active_tool(state) == ToolKind::Transform
        && match &stamp.placement {
            Placement::Projected(_) => xf.is_3d,
            Placement::Uv(u) => !xf.is_3d && u.set == session.active_set,
        };
    if gizmo_here {
        return;
    }
    let color = crate::theme::ACCENT;
    match (&stamp.placement, xf.is_3d) {
        // Round 6: a wrapped stamp's outline lies on the surface (3D) or is mapped to the active set's texture (2D).
        (Placement::Projected(p), true) if p.mode == SurfaceMode::Wrap => {
            draw_polylines(painter, &gizmo::wrapped_outline(&session.doc, stamp.id, xf), color, 1.5);
        }
        (Placement::Projected(p), false) if p.mode == SurfaceMode::Wrap => {
            draw_polylines(painter, &wrapped_outline_in_2d(&session.doc, stamp.id, session.active_set, xf), color, 1.5);
        }
        (Placement::Projected(p), true) => {
            let corners = p.corners().map(|c| xf.world_to_screen(c));
            if let [Some(a), Some(b), Some(c), Some(d)] = corners {
                draw_quad(painter, [a, b, c, d], color, 1.5);
            }
        }
        (Placement::Uv(u), false) if u.set == session.active_set => {
            draw_quad(painter, u.corners_px().map(|c| xf.px_to_screen(c)), color, 1.5);
        }
        (Placement::Uv(u), true) => draw_polylines(painter, &uv_outline_in_3d(&session.doc, u, xf), color, 1.5),
        (Placement::Projected(p), false) => {
            draw_polylines(painter, &projected_outline_in_2d(&session.doc, p, session.active_set, xf), color, 1.5);
        }
        (Placement::Uv(_), false) => {}
    }
}

/// Real-GPU checks of the views: a headless device, real egui passes with injected pointer events through `central`,
/// and the view targets read back. Ignored by default (they need a GPU); run with
/// `cargo test -p wte_app --offline -- --ignored view::gpu_view_tests`. Images go to `target/selftest/views_*.png`.
#[cfg(test)]
mod gpu_view_tests {
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::gpu::ViewTarget;
    use eframe::{egui_wgpu, wgpu};
    use crate::tools::test_support::state_with;
    use wte_core::SampleKind;

    fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }

    fn headless_gpu() -> Gpu {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("no wgpu adapter");
        let base = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        };
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("view_probe"),
            required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..base },
            ..Default::default()
        }))
        .expect("no wgpu device");
        let egui_renderer = egui_wgpu::Renderer::new(&device, Gpu::COLOR_FORMAT, egui_wgpu::RendererOptions::default());
        Gpu {
            max_side: device.limits().max_texture_dimension_2d.min(8192),
            msaa: crate::gpu::msaa_for(adapter.get_info().backend),
            device,
            queue,
            egui_renderer: std::sync::Arc::new(egui::mutex::RwLock::new(egui_renderer)),
        }
    }



    fn read_back(gpu: &Gpu, target: &ViewTarget) -> (u32, u32, Vec<u8>) {

        let [w, h] = target.size_px();
        let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read_back"),
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            target.resolve_texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.expect("map failed"));
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll failed");
        let mapped = buffer.get_mapped_range(..).expect("mapped range");
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in mapped.chunks(padded as usize).take(h as usize) {
            out.extend_from_slice(&row[..(w * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();
        (w, h, out)
    }

    fn save(gpu: &Gpu, target: &ViewTarget, name: &str) -> Vec<u8> {
        let (w, h, rgba) = read_back(gpu, target);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/selftest");
        std::fs::create_dir_all(&dir).expect("create target/selftest");
        let png = wte_core::encode_png(w, h, &rgba, wte_core::PngLevel::Fast).expect("encode png");
        std::fs::write(dir.join(format!("views_{name}.png")), png).expect("write png");
        rgba
    }

    struct Harness {
        ctx: egui::Context,
        gpu: Gpu,
        renderer: Renderer,
        scene: Option<GpuScene>,
        targets: ViewTargets,
        tools: ToolController,
        state: AppState,
        frame: u64,
    }

    impl Harness {
        fn new() -> Harness {
            let gpu = headless_gpu();
            let renderer = Renderer::new(&gpu);
            let targets = ViewTargets::new(&gpu);
            Harness {
                ctx: egui::Context::default(),
                gpu,
                renderer,
                scene: None,
                targets,
                tools: ToolController::new(),
                state: state_with(SampleKind::Sphere),
                frame: 0,
            }
        }

        /// One egui pass of the central area with these input events.
        fn frame(&mut self, events: Vec<egui::Event>) {
            self.frame += 1;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0))),
                time: Some(self.frame as f64 / 60.0),
                predicted_dt: 1.0 / 60.0,
                events,
                ..Default::default()
            };
            let Harness { ctx, gpu, renderer, scene, targets, tools, state, .. } = self;
            let mut output = ctx.run_ui(raw, |ui| {
                let mut cx = ViewCx { state: &mut *state, gpu: &*gpu, renderer: &mut *renderer, scene: &mut *scene, targets: &mut *targets, tools: &mut *tools };
                egui::CentralPanel::no_frame().show(ui, |ui| central(ui, &mut cx));
            });
            output.textures_delta.clear();
            // The tools queue colour picks etc.; the test applies only the primary colour.
            for a in std::mem::take(&mut self.state.actions) {
                if let AppAction::SetPrimaryColor(c) = a {
                    self.state.colors.primary = c;
                }
            }
        }

        fn at(&self, view: ViewId, x: f32, y: f32) -> egui::Pos2 {
            let r = self.state.view_rects[&view];
            r.min + egui::vec2(x * r.width(), y * r.height())
        }

        /// Hover, press, moves, release (one event per frame, like the self-test runner).
        fn stroke(&mut self, points: &[egui::Pos2]) {
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(points[0])]);
            self.frame(vec![egui::Event::PointerMoved(points[0]), button(points[0], true)]);
            for p in &points[1..] {
                self.frame(vec![egui::Event::PointerMoved(*p)]);
            }
            self.frame(vec![button(*points.last().unwrap(), false)]);
            self.frame(vec![]);
        }

        fn undo_len(&self) -> usize {
            self.state.doc().unwrap().history.undo_len()
        }
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn views_paint_stamp_and_render_in_every_mode() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.frame(vec![]);
        let before = save(&h.gpu, &h.targets.main3d, "3d_before");
        assert!(h.state.view_rects.contains_key(&ViewId::Main3D));
        assert!(!h.state.gpu_mips_stale);

        // 3D stroke across the sphere in red.
        h.state.colors.primary = [230, 20, 20, 255];
        h.state.tools.brush.size_px = 48.0;
        let path: Vec<egui::Pos2> = (0..=12).map(|k| h.at(ViewId::Main3D, 0.42 + 0.16 * k as f32 / 12.0, 0.5)).collect();
        h.stroke(&path);
        assert!(!h.tools.is_busy());
        assert_eq!(h.undo_len(), 1, "one stroke entry");
        h.frame(vec![]);
        let after = save(&h.gpu, &h.targets.main3d, "3d_after_stroke");
        assert_ne!(before, after);

        // Split: a blue 2D stroke, both views rendered.
        h.state.view_mode = ViewMode::Split;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.state.view_rects.contains_key(&ViewId::Main2D) && h.state.view_rects.contains_key(&ViewId::Main3D));
        h.state.colors.primary = [20, 40, 230, 255];
        let path: Vec<egui::Pos2> = (0..=8).map(|k| h.at(ViewId::Main2D, 0.3 + 0.4 * k as f32 / 8.0, 0.3)).collect();
        h.stroke(&path);
        assert_eq!(h.undo_len(), 2);
        h.frame(vec![]);
        save(&h.gpu, &h.targets.main3d, "split_3d");
        save(&h.gpu, &h.targets.main2d, "split_2d");

        // Text stamp clicked onto the model in the 3D view, then wheel-scaled with Ctrl in Transform.
        h.state.tool = ToolKind::Text;
        h.state.tools.text.content = "WTE".to_owned();
        h.state.tools.text.em_frac = 0.12;
        let p = h.at(ViewId::Main3D, 0.5, 0.42);
        h.stroke(&[p]);
        assert_eq!(h.state.doc().unwrap().stamps.len(), 1);
        assert_eq!(h.state.tool, ToolKind::Transform);
        let undo = h.undo_len();
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::CTRL,
        }]);
        // One wheel gesture: a pending edit, committed once the wheel has been idle for a moment.
        assert!(h.state.doc().unwrap().has_pending_edit(), "Ctrl+wheel scaled the selected stamp");
        for _ in 0..(crate::tools::transform::WHEEL_IDLE * 60.0) as usize + 2 {
            h.frame(vec![]);
        }
        assert_eq!(h.undo_len(), undo + 1, "the wheel gesture is one entry");
        // A plain wheel zooms the camera instead.
        let distance = h.state.session.as_ref().unwrap().camera.distance;
        h.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(h.state.session.as_ref().unwrap().camera.distance < distance);
        h.frame(vec![]);
        save(&h.gpu, &h.targets.main3d, "split_3d_text");
        save(&h.gpu, &h.targets.main2d, "split_2d_text");

        // Right-drag orbits.
        let yaw = h.state.session.as_ref().unwrap().camera.yaw;
        let a = h.at(ViewId::Main3D, 0.5, 0.5);
        let b = h.at(ViewId::Main3D, 0.6, 0.5);
        let rb = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(vec![egui::Event::PointerMoved(a)]);
        h.frame(vec![rb(a, true)]);
        h.frame(vec![egui::Event::PointerMoved(b)]);
        h.frame(vec![rb(b, false)]);
        assert!((h.state.session.as_ref().unwrap().camera.yaw - yaw).abs() > 0.1);
        assert!(!h.tools.is_busy());

        // Clay and UV checker shading.
        h.state.flags.shading = ShadingMode::UvChecker;
        h.state.flags.seams = true;
        h.frame(vec![]);
        save(&h.gpu, &h.targets.main3d, "3d_uvchecker_seams");

        // Preview mode.
        h.state.view_mode = ViewMode::Preview;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.state.view_rects.contains_key(&ViewId::Preview));
        save(&h.gpu, &h.targets.preview, "preview");

        // Dialog gate: no tool input while a dialog is open.
        h.state.view_mode = ViewMode::View3D;
        h.state.tool = ToolKind::Brush;
        h.state.dialog = Dialog::About;
        h.frame(vec![]);
        let undo = h.undo_len();
        let path: Vec<egui::Pos2> = (0..=4).map(|k| h.at(ViewId::Main3D, 0.45 + 0.02 * k as f32, 0.55)).collect();
        h.stroke(&path);
        assert_eq!(h.undo_len(), undo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::*;
    use wte_core::SampleKind;

    #[test]
    fn wheel_units_become_notches() {
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Line, egui::vec2(0.0, 2.0)), 2.0);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Point, egui::vec2(0.0, -100.0)), -2.0);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Page, egui::vec2(0.0, 1.0)), 10.0);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Line, egui::vec2(3.0, 0.0)), 3.0);
    }

    #[test]
    fn split_rects_partition_the_area() {
        let area = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(1000.0, 500.0));
        let (l, d, r) = split_rects(area, 0.4);
        assert_eq!(l.min, area.min);
        assert_eq!(r.max, area.max);
        assert_eq!(l.max.x, d.min.x);
        assert_eq!(d.max.x, r.min.x);
        assert!((d.center().x - 410.0).abs() <= 1.0);
        let (l, _, _) = split_rects(area, 5.0);
        assert!(l.width() <= area.width() * SPLIT_LIMITS.1);
    }

    #[test]
    fn locate_px_finds_the_surface() {
        let state = state_with(SampleKind::Sphere);
        let doc = state.doc().unwrap();
        let set = &doc.sets[0];
        let p = locate_px(doc, TextureSetId(0), Vec2::new(set.width as f32 * 0.5, set.height as f32 * 0.5)).unwrap();
        // On the sphere surface.
        let r = doc.mesh.bounds.diagonal() * 0.5 / 3f32.sqrt();
        assert!(((p - doc.mesh.bounds.center()).length() - r).abs() < r * 0.05);
    }

    #[test]
    fn outlines_are_traced_in_the_other_view() {
        let mut state = state_with(SampleKind::Sphere);
        let xf3 = xf_3d(&mut state);
        let xf2 = xf_2d(&mut state);
        let doc = state.doc().unwrap();
        let set = &doc.sets[0];
        let uv = UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(set.width as f32 * 0.5, set.height as f32 * 0.5),
            size_px: Vec2::splat(set.width as f32 * 0.1),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let lines = uv_outline_in_3d(doc, &uv, &xf3);
        assert!(lines.iter().map(Vec::len).sum::<usize>() > 10);
        let Surface::Model(hit) = xf3.surface_at(doc, TextureSetId(0), xf3.rect.center()) else { panic!() };
        let d = doc.mesh.bounds.diagonal();
        let proj = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::splat(d * 0.05), d * 0.05);
        let lines = projected_outline_in_2d(doc, &proj, TextureSetId(0), &xf2);
        assert!(lines.iter().map(Vec::len).sum::<usize>() > 10);
        for p in lines.iter().flatten() {
            assert!(xf2.rect.expand(1.0).contains(*p));
        }
    }

    /// Check:B (ROUND6_PLAN 3.3 "2D view", 5.1 `view\mod.rs`): the 2D outline of a selected wrapped stamp
    /// (`wrapped_outline_in_2d`) runs along the edge of the texels the stamp inks: every point has inked and blank
    /// texels within 2 texels (an opaque picture, so its rectangle's border is the edge of the ink); without a cached
    /// parameterization it is empty.
    #[test]
    fn a_wrapped_stamps_2d_outline_runs_along_its_inked_texels() {
        use wte_core::{Commit, Projector, StampSource, SurfaceMode, samples};
        let mut state = state_with(SampleKind::Sphere);
        let xf2 = xf_2d(&mut state);
        let img = samples::cells_picture(64, 32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("cells.png", std::sync::Arc::from(png)).unwrap();
        let (center, d) = (doc.mesh.bounds.center(), doc.mesh.bounds.diagonal());
        let r = d * 0.5 / 3f32.sqrt();
        // Front of the sphere (the UV seam is at the back), the picture over a curved patch.
        let at = center + Vec3::new(0.3, 0.2, 1.0).normalize() * r;
        let flat = Projector::from_surface(at, (at - center).normalize(), Vec3::Y, Vec2::new(0.45, 0.22) * r, 0.2 * r);
        let wrapped = Projector { mode: SurfaceMode::Wrap, ..flat };
        let id = doc.add_stamp("cells", StampSource::Image { blob }, Placement::Projected(wrapped), TextureSetId(0)).unwrap();
        let doc = state.doc().unwrap();
        let lines = wrapped_outline_in_2d(doc, id, TextureSetId(0), &xf2);
        let points: Vec<egui::Pos2> = lines.iter().flatten().copied().collect();
        assert!(points.len() >= 4 * gizmo::WRAP_OUTLINE_SAMPLES, "{} points in {} lines", points.len(), lines.len());
        let (set, layer) = doc.stamp_layers(id)[0];
        let set = doc.set(set).unwrap();
        let ink = doc.layer(layer).unwrap().to_dense(set.width, set.height);
        let alpha = |x: i32, y: i32| -> Option<u8> {
            (x >= 0 && y >= 0 && (x as u32) < set.width && (y as u32) < set.height).then(|| ink.get(x as u32, y as u32)[3])
        };
        let mut off = 0;
        for p in &points {
            let px = xf2.screen_to_px(*p);
            let (cx, cy) = (px.x.floor() as i32, px.y.floor() as i32);
            // The nearest inked and the nearest blank texel (Chebyshev distance, up to 12).
            let (mut inked, mut blank) = (None, None);
            for r in 0..=12i32 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx.abs().max(dy.abs()) != r {
                            continue;
                        }
                        match alpha(cx + dx, cy + dy) {
                            Some(0) => blank = blank.or(Some(r)),
                            Some(_) => inked = inked.or(Some(r)),
                            None => {}
                        }
                    }
                }
            }
            let at_edge = inked.is_some_and(|r| r <= 3) && blank.is_some_and(|r| r <= 3);
            if !at_edge {
                eprintln!("check:B 2D outline point {px:?}: nearest ink {inked:?}, nearest blank {blank:?}");
            }
            off += usize::from(!at_edge);
        }
        assert!(off * 50 <= points.len(), "{off} of {} outline points are not at the edge of the ink", points.len());
        // A moved placement has no parameterization until it is baked: no outline.
        let moved = wrapped.moved_to(center + Vec3::new(-0.3, 0.1, 1.0).normalize() * r, Vec3::new(-0.3, 0.1, 1.0).normalize());
        let doc = state.doc_mut().unwrap();
        doc.set_defer_preview_bakes(true);
        doc.set_stamp_placement(id, Placement::Projected(moved), Commit::Preview).unwrap();
        assert!(wrapped_outline_in_2d(doc, id, TextureSetId(0), &xf2).is_empty());
    }
}
