// Owner: stickers P6a (STICKER_PLAN 3.1, 3.2, 3.3, 3.10; ARCHITECTURE 8.15). The central area of the Sticker view:
// `[sticker canvas | divider | 3D view]`. The canvas is an egui-painted area (not a `ViewId`, review A17): a header
// (name, Done, Revert, zoom, Fit, 100 %), the sticker on a checkerboard with its artboard outline, a pixel grid from
// 800 %, the active layer's outline, and its own pointer routing: the Move tool (one history entry per drag), Hand /
// Space / Secondary / Middle drags pan, the wheel (and the companion's pinch) zooms about the pointer. The 3D half is
// the ordinary `ViewId::Main3D` with the Navigate behaviour only (`tools::active_tool`).
//
// Stickers P7a: the paint tools on the canvas (Brush and Eraser strokes with the pen's pressure, Fill, Gradient,
// Eyedropper; `sticker/tools/`), the live canvas of a stroke (`live.rs`: dirty rectangles, stroke mode, the restyle
// and Preview jobs), the stroke-mode note and the Rasterize offer in the header, the selection's outline, the
// gradient's line and the brush ring.
//
// Stickers P7b: the selection tools on the canvas (the Marquee's box and the Lasso's line while they are drawn, the
// Magic Wand's click), the Magic Eraser, the Move tool's drag of the selected pixels, the selection's dashed outline
// (`outline.rs`), and the popover of a command (`popover.rs`: its live preview in the canvas texture, a click on the
// canvas that picks its colour).

use eframe::egui;
use egui::{Color32, Pos2, Rect, Sense, Stroke, vec2};
use glam::Vec2;
use wte_core::sticker::render::LayerKey;
use wte_core::{
    ArtboardShape, BlendMode, BlobId, Commit, Document, IRect, PicturePixels, StampSource, StickerContent, StickerId,
    StickerLayerId, StyleStack,
};

use crate::app::frame_profile;
use crate::state::{AppState, Canvas2D, PanelHit, StickerEdit, StickerTarget, StickerTool};
use crate::sticker::{CanvasDrag, Shown, StickerAction, edit_mut, edit_of, shown};
use crate::view::{ViewCx, view3d};

/// Height of the canvas header (points).
pub(crate) const HEADER_HEIGHT: f32 = 28.0;
/// Side of a checkerboard cell behind the sticker (points).
const CHECKER_CELL: f32 = 8.0;
/// The pixel grid is drawn from this zoom on (points per sticker pixel: 800 %).
const GRID_ZOOM: f32 = 8.0;
/// Background of the canvas around the artboard.
const CANVAS_BG: Color32 = Color32::from_gray(38);
/// How the canvas texture is sampled: sticker pixels stay sharp when zoomed in.
pub(crate) const TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

/// What one layer looks like on the canvas, compared by value; pixels only by version or blob id (`LayerKey`), so
/// building or comparing it never reads pixel bytes and holds no buffer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerSig {
    id: StickerLayerId,
    visible: bool,
    opacity: u32,
    blend: BlendMode,
    clip: bool,
    key: LayerKey,
}

/// What the canvas texture was made from (`StickerUi::canvas`): the picture is drawn again only when this changes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CanvasKey {
    Sticker { id: StickerId, artboard: IRect, shape: ArtboardShape, layers: Vec<LayerSig> },
    Image(BlobId),
    Text(Box<StampSource>, Box<StyleStack>, [u8; 4]),
    /// Stickers P7a: what the texture shows is not a finished picture of anything (a cancelled stroke, a new texture of
    /// the live canvas): the next frame draws it again.
    Stale,
}

/// The canvas's frame of reference: the artboard's top-left corner (layer space) and its size in pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Geom {
    pub(crate) origin: Vec2,
    pub(crate) size: [u32; 2],
}

impl Geom {
    fn size_vec(self) -> Vec2 {
        Vec2::new(self.size[0].max(1) as f32, self.size[1].max(1) as f32)
    }

    /// Layer space -> screen.
    pub(crate) fn to_screen(self, cam: &Canvas2D, rect: Rect, p: Vec2) -> Pos2 {
        cam.uv_to_screen((p - self.origin) / self.size_vec(), self.size, rect)
    }

    /// Screen -> layer space.
    pub(crate) fn to_layer(self, cam: &Canvas2D, rect: Rect, pos: Pos2) -> Vec2 {
        self.origin + cam.screen_to_uv(pos, self.size, rect) * self.size_vec()
    }
}

/// How a click of the Move tool finds its layer.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Hit {
    /// A sticker of the document: `Document::sticker_layer_at`.
    Sticker(StickerId),
    /// An image stamp's virtual sticker: its one layer (id 1) wherever the picture is.
    Image,
    /// A text stamp: nothing on the canvas can be moved.
    ReadOnly,
}

/// The key of sticker `id`'s picture, by value (no buffer kept; stickers P6b: also the tray thumbnails' key).
pub(crate) fn sticker_key(id: StickerId, sticker: &wte_core::StickerDoc) -> CanvasKey {
    CanvasKey::Sticker {
        id,
        artboard: sticker.artboard,
        shape: sticker.shape,
        layers: sticker
            .layers
            .iter()
            .map(|l| LayerSig {
                id: l.id,
                visible: l.visible,
                opacity: l.opacity.to_bits(),
                blend: l.blend,
                clip: l.clip,
                key: LayerKey::of(l, sticker.artboard, 1.0, wte_core::RasterQuality::Full),
            })
            .collect(),
    }
}

/// The canvas key of what is shown.
fn key_of(shown: &Shown, doc: &Document, edit: &StickerEdit) -> Option<CanvasKey> {
    let _ = edit;
    key_of_shown(shown, doc)
}

/// The canvas key of what is shown (stickers P7a: the brush asks whether the texture is the finished picture).
pub(crate) fn key_of_shown(shown: &Shown, doc: &Document) -> Option<CanvasKey> {
    match shown {
        Shown::Sticker { id, doc: sticker } => Some(sticker_key(*id, sticker)),
        Shown::Image { blob, .. } => Some(CanvasKey::Image(*blob)),
        Shown::Text { stamp } => {
            let object = doc.stamp(*stamp)?;
            Some(CanvasKey::Text(Box::new(object.source.clone()), Box::new(object.styles.clone()), object.tint))
        }
    }
}

/// The geometry of what is shown (a text stamp: its picture's size at the origin).
fn geom_of(shown: &Shown, picture: [u32; 2]) -> Geom {
    match shown.layers() {
        Some(doc) => Geom { origin: Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32), size: doc.size() },
        None => Geom { origin: Vec2::ZERO, size: picture },
    }
}

/// Brings the canvas texture up to date (draws the sticker again only when its key changed; never while the live
/// canvas of a stroke on that sticker owns the texture, `live.rs`). Returns the texture and its size.
fn sync_texture(ctx: &egui::Context, state: &mut crate::state::AppState, shown: &Shown, edit: &StickerEdit) -> Option<(egui::TextureId, [u32; 2])> {
    let live_owns = match (&state.sticker_ui.live, shown) {
        (Some(live), Shown::Sticker { id, .. }) => live.sticker == *id,
        _ => false,
    };
    // Stickers P7b: an open popover's preview picture owns the texture until it closes.
    let live_owns = live_owns || state.sticker_ui.popover.as_ref().is_some_and(|p| p.shows_picture);
    let session = state.session.as_mut()?;
    let key = key_of(shown, &session.doc, edit)?;
    if !live_owns && state.sticker_ui.canvas.as_ref().is_none_or(|c| c.key != key) {
        let _p = frame_profile::scope("sticker.render");
        let picture = match (shown, edit.target) {
            (Shown::Sticker { id, .. }, _) => session.doc.sticker_picture(*id, None, 1.0),
            (Shown::Image { .. }, StickerTarget::Stamp(stamp)) => session.doc.stamp_picture(stamp, false),
            (Shown::Text { stamp }, _) => session.doc.stamp_picture(*stamp, true),
            (Shown::Image { .. }, _) => return None,
        };
        let picture = match picture {
            Ok(p) if p.width > 0 && p.height > 0 => p,
            Ok(_) => return None,
            Err(e) => {
                log::warn!("the sticker canvas cannot draw the sticker: {e}");
                return None;
            }
        };
        let size = [picture.width, picture.height];
        crate::sticker::live::upload(ctx, state, &IRect::from_size(size[0], size[1]), &picture.data, size, true);
        let ui_state = &mut state.sticker_ui;
        ui_state.canvas_uploads += 1;
        if let Some(c) = ui_state.canvas.as_mut() {
            c.key = key;
            c.exact = true;
        }
    }
    state.sticker_ui.canvas.as_ref().map(|c| (c.tex.id(), c.size))
}

/// Stickers P7a: what `canvas_side` does to the texture in a frame, without drawing (tests): the key-driven upload,
/// then the live canvas's work (`live::frame`).
#[cfg(test)]
pub(crate) fn refresh(ctx: &egui::Context, state: &mut AppState) {
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    sync_texture(ctx, state, &shown, &edit);
    crate::sticker::live::frame(ctx, state);
    // P7b check: and the open popover's preview (its job), as `canvas_side`.
    crate::sticker::popover::frame(ctx, state);
}

/// The central area in the Sticker view: canvas, divider, 3D view. Without an open editor (a document replaced
/// underneath it) the 3D view alone.
pub fn show(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, area: Rect) {
    if edit_of(cx.state).is_none() {
        crate::view::show_3d(ui, cx, area);
        return;
    }
    let (left, divider, right) = crate::view::split_rects(area, cx.state.prefs.sticker_split);
    canvas_side(ui, cx.state, left);
    crate::view::split_divider(ui, &mut cx.state.prefs.sticker_split, area, divider);
    view3d::input(ui, cx, right);
    if crate::view::sync_scene(cx, false) {
        view3d::render(ui, cx, right);
    }
}

/// The left side: header and canvas.
fn canvas_side(ui: &mut egui::Ui, state: &mut AppState, left: Rect) {
    let _p = frame_profile::scope("sticker.canvas");
    let header = Rect::from_min_size(left.min, vec2(left.width(), HEADER_HEIGHT.min(left.height())));
    let rect = Rect::from_min_max(egui::pos2(left.min.x, header.max.y), left.max);
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    state.sticker_canvas_rect = Some(rect);
    state.panel_rects.insert(PanelHit::StickerCanvas, rect);
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let texture = sync_texture(ui.ctx(), state, &shown, &edit);
    let geom = geom_of(&shown, texture.map_or([1, 1], |t| t.1));
    header_bar(ui, state, header, &shown, &edit);

    // The camera: fitted on the first draw (and by Fit, which resets it), zoomed by "100 %".
    if let Some(e) = edit_mut(state)
        && e.camera == Canvas2D::default()
    {
        e.camera.fit(geom.size, rect);
    }
    if let Some(zoom) = state.sticker_ui.zoom_request.take()
        && let Some(e) = edit_mut(state)
    {
        e.camera.zoom_about(rect.center(), zoom / e.camera.zoom.max(1e-6), geom.size, rect);
    }

    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let hit = match &shown {
        Shown::Sticker { id, .. } => Hit::Sticker(*id),
        Shown::Image { .. } => Hit::Image,
        Shown::Text { .. } => Hit::ReadOnly,
    };
    handle_input(ui, state, rect, geom, hit, response.contains_pointer());
    // Stickers P7a: the dabs of this frame reach the texture in this frame.
    crate::sticker::live::frame(ui.ctx(), state);
    // Stickers P7b: the open popover's live preview.
    crate::sticker::popover::frame(ui.ctx(), state);
    let texture = state.sticker_ui.canvas.as_ref().map(|c| c.tex.id()).or(texture.map(|t| t.0));
    draw(ui, state, rect, geom, texture, &shown);
    crate::sticker::popover::show(ui, state, rect);
}

/// The canvas header (3.2): the sticker's name, Done, Revert, the zoom, Fit, 100 %; a text stamp's "Convert to layered
/// sticker".
fn header_bar(ui: &mut egui::Ui, state: &mut AppState, header: Rect, shown: &Shown, edit: &StickerEdit) {
    ui.painter().rect_filled(header, 0.0, ui.visuals().panel_fill);
    let differs = state.session.as_ref().is_some_and(|s| crate::sticker::differs_from_entry(&s.doc, edit));
    let stamp_name = match edit.target {
        StickerTarget::Stamp(id) => state.doc().and_then(|d| d.stamp(id)).map(|s| s.name.clone()).unwrap_or_default(),
        _ => String::new(),
    };
    let mut actions = Vec::new();
    let mut hits = Vec::new();
    let builder = egui::UiBuilder::new()
        .max_rect(header.shrink2(vec2(6.0, 2.0)))
        .layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        // A narrow canvas (a small window, a wide 3D half) keeps Done and Revert; the zoom read-out, Fit and 100 % go
        // first (Fit is also the F key).
        let narrow = ui.available_width() < 460.0;
        match shown {
            Shown::Sticker { doc, .. } => {
                let mut text = state.sticker_ui.name_edit.clone().unwrap_or_else(|| doc.name.clone());
                let width = if narrow { 80.0 } else { 130.0 };
                let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width)).on_hover_text("The sticker's name");
                if resp.lost_focus() {
                    actions.push(crate::action::AppAction::Sticker(StickerAction::Rename(text)));
                    state.sticker_ui.name_edit = None;
                } else if resp.has_focus() || resp.changed() {
                    state.sticker_ui.name_edit = Some(text);
                }
            }
            Shown::Image { .. } => {
                ui.add(egui::Label::new(egui::RichText::new(&stamp_name).strong()).truncate());
                if !narrow {
                    ui.weak("picture");
                }
            }
            Shown::Text { .. } => {
                ui.add(egui::Label::new(egui::RichText::new(&stamp_name).strong()).truncate());
                if !narrow {
                    ui.weak("text (read-only here)");
                }
            }
        }
        let done = ui.button("Done").on_hover_text("Back to the previous view (Esc)");
        hits.push((PanelHit::StickerDone, done.rect));
        if done.clicked() {
            actions.push(crate::action::AppAction::Sticker(StickerAction::Leave));
        }
        let revert = ui
            .add_enabled(differs, egui::Button::new("Revert"))
            .on_hover_text("Put the sticker back to how it was when the editor opened (one undo step)")
            .on_disabled_hover_text("Nothing changed since the editor opened");
        hits.push((PanelHit::StickerRevert, revert.rect));
        if revert.clicked() {
            actions.push(crate::action::AppAction::Sticker(StickerAction::Revert));
        }
        if let Shown::Text { .. } = shown {
            let convert = ui
                .button("Convert to layered sticker")
                .on_hover_text("The text becomes a text layer of a sticker; it stays editable in the Text tab");
            hits.push((PanelHit::StickerConvertText, convert.rect));
            if convert.clicked() {
                actions.push(crate::action::AppAction::Sticker(StickerAction::ConvertText));
            }
        }
        // Stickers P7a: stroke mode (3.5) and the Rasterize offer of the paint tools (3.3 "Layer-kind rules").
        if state.sticker_ui.live.as_ref().is_some_and(|l| l.stroke_mode()) {
            ui.colored_label(crate::theme::ACCENT, crate::sticker::live::STROKE_MODE_NOTE);
            state.sticker_ui.counters.note_frames += 1;
        } else if state.sticker_ui.rasterize_offer.is_some() {
            if !narrow {
                ui.weak(crate::sticker::tools::RASTERIZE_HINT);
            }
            let r = ui.button("Rasterize").on_hover_text("Make this text or shape layer a picture layer, so it can be painted");
            hits.push((PanelHit::StickerHeaderRasterize, r.rect));
            if r.clicked() {
                actions.push(crate::action::AppAction::Sticker(StickerAction::Rasterize));
            }
        }
        let room = ui.available_width();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let fit_key = crate::shortcuts::sticker_shortcut_text(ui.ctx(), "Fit Sticker");
            if room >= 60.0 {
                let zoom100 = ui.button("100 %").on_hover_text("One sticker pixel per screen point");
                hits.push((PanelHit::StickerZoom100, zoom100.rect));
                if zoom100.clicked() {
                    actions.push(crate::action::AppAction::Sticker(StickerAction::Zoom100));
                }
            }
            if room >= 100.0 {
                let fit = ui.button("Fit").on_hover_text(format!("Fit the sticker in the canvas ({fit_key})"));
                hits.push((PanelHit::StickerFit, fit.rect));
                if fit.clicked() {
                    actions.push(crate::action::AppAction::Sticker(StickerAction::Fit));
                }
            }
            if room >= 150.0 {
                ui.weak(format!("{:.0} %", edit.camera.zoom * 100.0));
            }
        });
    });
    state.actions.extend(actions);
    state.panel_rects.extend(hits);
}

/// A pointer event of this pass that the canvas needs, with its pen pressure (1.0 for a mouse; stickers P7a).
enum CanvasEvent {
    Moved(Pos2, f32),
    /// Stickers P7b: with the keys held at the press (Shift adds to a selection, Alt subtracts).
    Pressed(Pos2, egui::PointerButton, f32, egui::Modifiers),
    Released(Pos2, egui::PointerButton, f32),
}

/// The canvas's pointer routing (3.3, 3.10): presses inside the canvas start a Move or a pan, motion continues it,
/// the release (also one not seen: the button is up) ends it; the wheel zooms about the pointer. Gated like the views
/// while a modal job runs or a dialog is open. Pure egui input + state: the unit tests drive it.
pub(crate) fn handle_input(ui: &egui::Ui, state: &mut AppState, rect: Rect, geom: Geom, hit: Hit, contains: bool) {
    // Stickers P7a: every pointer event takes its pressure from this frame's pen notes, in order (as `route_pointer`).
    let pen = &state.pen.frame;
    let held = pen.held_pressure();
    let (events, space, latest, down, double) = ui.input(|i| {
        let mut walk = pen.cursor();
        let events: Vec<CanvasEvent> = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerMoved(pos) => Some(CanvasEvent::Moved(*pos, walk.take(*pos))),
                egui::Event::PointerButton { pos, button, pressed: true, modifiers } => {
                    Some(CanvasEvent::Pressed(*pos, *button, walk.take(*pos), *modifiers))
                }
                egui::Event::PointerButton { pos, button, pressed: false, .. } => {
                    Some(CanvasEvent::Released(*pos, *button, walk.take(*pos)))
                }
                _ => None,
            })
            .collect();
        let down = [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle]
            .map(|b| (b, i.pointer.button_down(b)));
        let double = i.pointer.button_double_clicked(egui::PointerButton::Primary);
        (events, i.key_down(egui::Key::Space), i.pointer.latest_pos(), down, double)
    });
    state.sticker_ui.hover = latest.filter(|p| contains && rect.contains(*p));
    if crate::view::input_gated(state) {
        return;
    }
    for event in events {
        match event {
            CanvasEvent::Pressed(pos, button, pressure, mods) if contains && rect.contains(pos) && state.sticker_ui.drag.is_none() => {
                press(state, rect, geom, hit, pos, button, space, pressure, mods);
            }
            CanvasEvent::Pressed(..) => {}
            CanvasEvent::Moved(pos, pressure) => motion(state, rect, geom, pos, pressure),
            CanvasEvent::Released(pos, button, pressure) => {
                if drag_button(state) == Some(button) {
                    release(state, rect, geom, pos, pressure);
                }
            }
        }
    }
    // Stickers P7b: a double-click closes the polygon lasso.
    if double && contains && state.sticker_ui.lasso.as_ref().is_some_and(|l| !l.freehand) {
        crate::sticker::tools::lasso::close(state);
    }
    // The release happened where the canvas did not see it (outside the window).
    if let (Some(button), Some(pos)) = (drag_button(state), latest)
        && !down.iter().any(|(b, d)| *b == button && *d)
    {
        release(state, rect, geom, pos, held);
    }
    // Wheel (and the companion's pinch): zoom about the pointer.
    if let Some(pos) = latest.filter(|p| rect.contains(*p))
        && contains
    {
        for (notches, _) in crate::view::wheel_events(ui) {
            if let Some(e) = edit_mut(state) {
                e.camera.zoom_about(pos, (0.15 * notches).exp(), geom.size, rect);
            }
        }
    }
    // Cursor.
    let tool = crate::sticker::tool_of(state);
    let icon = match &state.sticker_ui.drag {
        Some(CanvasDrag::Pan { .. }) => Some(egui::CursorIcon::Grabbing),
        Some(CanvasDrag::Move { .. }) => Some(egui::CursorIcon::Move),
        Some(CanvasDrag::MoveSelection { .. }) => Some(egui::CursorIcon::Move),
        Some(
            CanvasDrag::Stroke { .. } | CanvasDrag::Gradient { .. } | CanvasDrag::Pick { .. } | CanvasDrag::Marquee { .. } | CanvasDrag::Lasso { .. },
        ) => Some(egui::CursorIcon::Crosshair),
        None if contains && (tool == StickerTool::Hand || space) => Some(egui::CursorIcon::Grab),
        None if contains && tool == StickerTool::Move && hit != Hit::ReadOnly => Some(egui::CursorIcon::Move),
        None if contains => Some(egui::CursorIcon::Crosshair),
        None => None,
    };
    if let Some(icon) = icon {
        ui.ctx().set_cursor_icon(icon);
    }
}

/// The physical button of the running drag.
fn drag_button(state: &AppState) -> Option<egui::PointerButton> {
    match state.sticker_ui.drag.as_ref()? {
        CanvasDrag::Move { button, .. }
        | CanvasDrag::Pan { button, .. }
        | CanvasDrag::Stroke { button, .. }
        | CanvasDrag::Gradient { button, .. }
        | CanvasDrag::Pick { button }
        | CanvasDrag::Marquee { button, .. }
        | CanvasDrag::Lasso { button, .. }
        | CanvasDrag::MoveSelection { button, .. } => Some(*button),
    }
}

/// A press in the canvas.
#[allow(clippy::too_many_arguments)]
fn press(
    state: &mut AppState,
    rect: Rect,
    geom: Geom,
    hit: Hit,
    pos: Pos2,
    button: egui::PointerButton,
    space: bool,
    pressure: f32,
    mods: egui::Modifiers,
) {
    let tool = crate::sticker::tool_of(state);
    let pan = match button {
        egui::PointerButton::Secondary | egui::PointerButton::Middle => true,
        egui::PointerButton::Primary => tool == StickerTool::Hand || space,
        _ => return,
    };
    if pan {
        state.sticker_ui.drag = Some(CanvasDrag::Pan { last: pos, button });
        return;
    }
    // Stickers P7b: an open Color to Alpha / Color Range popover takes the click as its colour.
    if let Some(cam) = edit_of(state).map(|e| e.camera)
        && crate::sticker::popover::pick(state, geom.to_layer(&cam, rect, pos))
    {
        return;
    }
    // Stickers P7b: the selection tools and the Magic Eraser.
    if matches!(tool, StickerTool::Marquee | StickerTool::Lasso | StickerTool::Wand | StickerTool::MagicEraser) {
        let Some(cam) = edit_of(state).map(|e| e.camera) else { return };
        let p = geom.to_layer(&cam, rect, pos);
        let mode = crate::sticker::tools::marquee::mode_of(mods, state.sticker_ui.select_mode);
        if hit == Hit::ReadOnly {
            state.status_hint = crate::sticker::tools::TEXT_STAMP_HINT.to_owned();
            return;
        }
        match tool {
            StickerTool::Marquee => state.sticker_ui.drag = Some(CanvasDrag::Marquee { start: p, to: p, mode, button }),
            StickerTool::Lasso => {
                let near_first = state
                    .sticker_ui
                    .lasso
                    .as_ref()
                    .and_then(|l| l.points.first())
                    .is_some_and(|f| geom.to_screen(&cam, rect, *f).distance(pos) <= crate::sticker::tools::lasso::CLOSE_DISTANCE);
                crate::sticker::tools::lasso::press(state, p, pos, mode, near_first, button);
            }
            StickerTool::Wand => state.push(crate::action::AppAction::Sticker(StickerAction::WandAt(p.to_array(), mode))),
            _ => state.push(crate::action::AppAction::Sticker(StickerAction::MagicErase(p.to_array()))),
        }
        return;
    }
    // Stickers P7a: the paint tools.
    if matches!(tool, StickerTool::Brush | StickerTool::Eraser | StickerTool::Fill | StickerTool::Gradient | StickerTool::Eyedropper) {
        let Some(cam) = edit_of(state).map(|e| e.camera) else { return };
        let p = geom.to_layer(&cam, rect, pos);
        match tool {
            StickerTool::Brush | StickerTool::Eraser => {
                crate::sticker::tools::brush::press(state, tool, p, pressure, button);
            }
            StickerTool::Fill => state.push(crate::action::AppAction::Sticker(StickerAction::Fill(p.to_array()))),
            StickerTool::Gradient => {
                if crate::sticker::tools::paint_target(state, tool, false).is_some() {
                    state.sticker_ui.drag = Some(CanvasDrag::Gradient { from: p, to: p, button });
                }
            }
            _ => {
                crate::sticker::tools::eyedropper::pick(state, p);
                state.sticker_ui.drag = Some(CanvasDrag::Pick { button });
            }
        }
        return;
    }
    if tool != StickerTool::Move {
        state.status_hint = format!("The sticker {} tool is not available yet", tool.label());
        return;
    }
    if hit == Hit::ReadOnly {
        state.status_hint = "A text stamp is edited in the Text tab; convert it to a layered sticker to move its parts".to_owned();
        return;
    }
    let Some(cam) = edit_of(state).map(|e| e.camera) else { return };
    let p = geom.to_layer(&cam, rect, pos);
    // Stickers P7b: a press inside the selection on a picture layer drags the selected pixels (cut into a new layer at
    // the first motion; one entry "Move Selection").
    if let Some(source) = crate::sticker::tools::move_selection::applies(state, p) {
        state.sticker_ui.drag = Some(CanvasDrag::MoveSelection { source, start: p, sent: None, button });
        return;
    }
    // Auto-select: the top layer under the pointer becomes the active one (nothing under it: the active layer stays).
    if state.sticker_ui.auto_select {
        let found = match hit {
            Hit::Sticker(id) => state.doc_mut().and_then(|d| d.sticker_layer_at(id, p)),
            Hit::Image => {
                let inside = p.x >= geom.origin.x
                    && p.y >= geom.origin.y
                    && p.x < geom.origin.x + geom.size[0] as f32
                    && p.y < geom.origin.y + geom.size[1] as f32;
                inside.then_some(StickerLayerId(1))
            }
            Hit::ReadOnly => None,
        };
        if let Some(found) = found
            && let Some(e) = edit_mut(state)
        {
            e.active_layer = Some(found);
            e.selected_layers = vec![found];
        }
    }
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(layer_id) = edit.active_layer else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let Some(layer) = shown.layers().and_then(|d| d.layer(layer_id)) else { return };
    if layer.locked {
        state.status_hint = "The layer is locked: unlock it first".to_owned();
        return;
    }
    let from = layer.transform.translate;
    state.sticker_ui.drag = Some(CanvasDrag::Move { layer: layer_id, start: p, from, sent: None, button });
}

/// Pointer motion during a drag: a Move sends a `Commit::Preview` position (whole pixels) when it changed; a pan moves
/// the camera; stickers P7a: a stroke paints a sample, a gradient drag follows, the Eyedropper picks.
fn motion(state: &mut AppState, rect: Rect, geom: Geom, pos: Pos2, pressure: f32) {
    let Some(cam) = edit_of(state).map(|e| e.camera) else { return };
    match state.sticker_ui.drag.clone() {
        Some(CanvasDrag::Stroke { .. }) => crate::sticker::tools::brush::sample(state, geom.to_layer(&cam, rect, pos), pressure),
        Some(CanvasDrag::Gradient { from, button, .. }) => {
            state.sticker_ui.drag = Some(CanvasDrag::Gradient { from, to: geom.to_layer(&cam, rect, pos), button });
        }
        Some(CanvasDrag::Pick { .. }) => {
            crate::sticker::tools::eyedropper::pick(state, geom.to_layer(&cam, rect, pos));
        }
        // Stickers P7b.
        Some(CanvasDrag::Marquee { start, mode, button, .. }) => {
            state.sticker_ui.drag = Some(CanvasDrag::Marquee { start, to: geom.to_layer(&cam, rect, pos), mode, button });
        }
        Some(CanvasDrag::Lasso { .. }) => crate::sticker::tools::lasso::motion(state, geom.to_layer(&cam, rect, pos), pos, cam.zoom),
        Some(CanvasDrag::MoveSelection { source, start, sent, button }) => {
            let delta = (geom.to_layer(&cam, rect, pos) - start).round();
            if (sent.is_some() || delta != Vec2::ZERO) && sent != Some(delta) {
                state.push(crate::action::AppAction::Sticker(StickerAction::MoveSelection(source, delta.to_array(), Commit::Preview)));
                state.sticker_ui.drag = Some(CanvasDrag::MoveSelection { source, start, sent: Some(delta), button });
            }
        }
        Some(CanvasDrag::Pan { last, button }) => {
            if let Some(e) = edit_mut(state) {
                e.camera.pan(pos - last, geom.size, rect);
            }
            state.sticker_ui.drag = Some(CanvasDrag::Pan { last: pos, button });
        }
        Some(CanvasDrag::Move { layer, start, from, sent, button }) => {
            let delta = (geom.to_layer(&cam, rect, pos) - start).round();
            let to = from + delta;
            if (sent.is_some() || delta != Vec2::ZERO) && sent != Some(to) {
                state.push(crate::action::AppAction::Sticker(StickerAction::SetLayerPosition(layer, to.to_array(), Commit::Preview)));
                state.sticker_ui.drag = Some(CanvasDrag::Move { layer, start, from, sent: Some(to), button });
            }
        }
        None => {}
    }
}

/// The end of a drag: a Move that moved ends with its `Commit::Final` (one history entry); a click moved nothing.
/// Stickers P7a: a stroke ends (one entry), a gradient drag is applied (`StickerAction::Gradient`, one entry).
fn release(state: &mut AppState, rect: Rect, geom: Geom, pos: Pos2, pressure: f32) {
    motion(state, rect, geom, pos, pressure);
    match state.sticker_ui.drag.take() {
        Some(CanvasDrag::Move { layer, sent: Some(to), .. }) => {
            state.push(crate::action::AppAction::Sticker(StickerAction::SetLayerPosition(layer, to.to_array(), Commit::Final)));
        }
        Some(CanvasDrag::Stroke { .. }) => crate::sticker::tools::brush::end(state),
        Some(CanvasDrag::Gradient { from, to, .. }) if from.distance(to) >= crate::sticker::tools::gradient::MIN_DRAG => {
            state.push(crate::action::AppAction::Sticker(StickerAction::Gradient(from.to_array(), to.to_array())));
        }
        // Stickers P7b.
        Some(CanvasDrag::Marquee { start, to, mode, .. }) => crate::sticker::tools::marquee::finish(state, start, to, mode),
        Some(CanvasDrag::Lasso { .. }) => crate::sticker::tools::lasso::release(state),
        Some(CanvasDrag::MoveSelection { source, sent: Some(d), .. }) => {
            state.push(crate::action::AppAction::Sticker(StickerAction::MoveSelection(source, d.to_array(), Commit::Final)));
        }
        _ => {}
    }
}

/// Paints the canvas: background, checkerboard, the sticker, the artboard outline, the pixel grid and the active
/// layer's outline.
fn draw(ui: &egui::Ui, state: &mut AppState, rect: Rect, geom: Geom, texture: Option<egui::TextureId>, shown: &Shown) {
    let Some(edit) = edit_of(state).cloned() else { return };
    let cam = edit.camera;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, CANVAS_BG);
    let board = Rect::from_two_pos(
        geom.to_screen(&cam, rect, geom.origin),
        geom.to_screen(&cam, rect, geom.origin + geom.size_vec()),
    );
    paint_checker(&painter, board.intersect(rect), board.min);
    if let Some(tex) = texture {
        painter.image(tex, board, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
    painter.rect_stroke(board, 0.0, Stroke::new(1.0, Color32::from_gray(140)), egui::StrokeKind::Outside);
    if cam.zoom >= GRID_ZOOM {
        paint_grid(&painter, board.intersect(rect), board.min, cam.zoom);
    }
    // The active layer's outline (picture layers: the transformed picture rectangle).
    if let (Some(doc), Some(active)) = (shown.layers(), edit.active_layer)
        && let Some(layer) = doc.layer(active)
        && let StickerContent::Picture(picture) = &layer.content
    {
        let size = match picture {
            PicturePixels::Owned { pixels, .. } => Some([pixels.width, pixels.height]),
            PicturePixels::Blob(b) => state.doc_mut().and_then(|d| d.image_size(*b).ok()),
        };
        if let Some(size) = size {
            let [w, h] = [size[0] as f32, size[1] as f32];
            let corners = [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)]
                .map(|c| geom.to_screen(&cam, rect, layer.transform.apply(size, c)));
            crate::view::draw_quad(&painter, corners, crate::theme::ACCENT, 1.0);
        }
    }
    // Stickers P7b: the selection's outline (a static two-colour dashed line, no animation: the frame loop stays idle),
    // or the one an open Color Range / Grow / Shrink / Feather popover would make; the Marquee's box and the Lasso's
    // line while they are drawn. (P7a: the gradient's line and the brush ring.)
    let preview = state.sticker_ui.popover.as_ref().and_then(|p| p.selection.clone());
    if let Some(sel) = preview.or_else(|| edit.selection.clone()).filter(|s| !s.is_empty()) {
        let _p = frame_profile::scope("sticker.outline");
        let outline = state.sticker_ui.outline.get(&sel);
        crate::sticker::outline::draw(&painter, &outline, geom, &cam, rect);
    }
    if let Some(CanvasDrag::Marquee { start, to, .. }) = state.sticker_ui.drag {
        let (a, b) = crate::sticker::tools::marquee::drag_box(state, start, to);
        let r = Rect::from_two_pos(geom.to_screen(&cam, rect, a), geom.to_screen(&cam, rect, b));
        if state.sticker_ui.marquee.ellipse {
            painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() * 0.5, Stroke::new(3.0, Color32::from_black_alpha(140))));
            painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() * 0.5, Stroke::new(1.0, Color32::WHITE)));
        } else {
            painter.rect_stroke(r, 0.0, Stroke::new(3.0, Color32::from_black_alpha(140)), egui::StrokeKind::Middle);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
        }
    }
    if let Some(lasso) = state.sticker_ui.lasso.as_ref() {
        let mut points: Vec<Pos2> = lasso.points.iter().map(|p| geom.to_screen(&cam, rect, *p)).collect();
        if !lasso.freehand
            && let Some(at) = state.sticker_ui.hover
        {
            points.push(at);
        }
        if points.len() >= 2 {
            painter.add(egui::Shape::line(points.clone(), Stroke::new(3.0, Color32::from_black_alpha(140))));
            painter.add(egui::Shape::line(points, Stroke::new(1.0, Color32::WHITE)));
        }
        if let Some(first) = lasso.points.first() {
            painter.circle_stroke(geom.to_screen(&cam, rect, *first), 4.0, Stroke::new(1.0, Color32::WHITE));
        }
    }
    if let Some(CanvasDrag::Gradient { from, to, .. }) = state.sticker_ui.drag {
        let (a, b) = (geom.to_screen(&cam, rect, from), geom.to_screen(&cam, rect, to));
        painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(160)));
        painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
        painter.circle_stroke(a, 4.0, Stroke::new(1.0, Color32::WHITE));
        painter.circle_filled(b, 3.0, Color32::WHITE);
    }
    if matches!(edit.tool, StickerTool::Brush | StickerTool::Eraser)
        && let Some(at) = state.sticker_ui.hover
    {
        let b = crate::sticker::tools::brush::settings_of(state, edit.tool);
        let radius = (b.size_px * 0.5 * cam.zoom).max(1.0);
        painter.circle_stroke(at, radius + 1.0, Stroke::new(1.0, Color32::from_black_alpha(140)));
        painter.circle_stroke(at, radius, Stroke::new(1.0, Color32::WHITE));
    }
}

/// A grey / white checkerboard over `area` with cells anchored at `anchor` (the artboard's corner, so the cells stay
/// put while panning).
fn paint_checker(painter: &egui::Painter, area: Rect, anchor: Pos2) {
    if !area.is_positive() {
        return;
    }
    painter.rect_filled(area, 0.0, Color32::from_gray(205));
    let x0 = ((area.min.x - anchor.x) / CHECKER_CELL).floor() as i64;
    let y0 = ((area.min.y - anchor.y) / CHECKER_CELL).floor() as i64;
    let x1 = ((area.max.x - anchor.x) / CHECKER_CELL).ceil() as i64;
    let y1 = ((area.max.y - anchor.y) / CHECKER_CELL).ceil() as i64;
    for y in y0..y1 {
        for x in x0..x1 {
            if (x + y).rem_euclid(2) == 0 {
                continue;
            }
            let min = anchor + vec2(x as f32 * CHECKER_CELL, y as f32 * CHECKER_CELL);
            let cell = Rect::from_min_size(min, vec2(CHECKER_CELL, CHECKER_CELL)).intersect(area);
            painter.rect_filled(cell, 0.0, Color32::from_gray(160));
        }
    }
}

/// The pixel grid over the visible part of the artboard (from 800 %).
fn paint_grid(painter: &egui::Painter, area: Rect, anchor: Pos2, zoom: f32) {
    if !area.is_positive() || zoom <= 0.0 {
        return;
    }
    let stroke = Stroke::new(1.0, Color32::from_black_alpha(60));
    let first = |min: f32, a: f32| a + ((min - a) / zoom).ceil() * zoom;
    let mut x = first(area.min.x, anchor.x);
    while x < area.max.x {
        painter.vline(x, area.y_range(), stroke);
        x += zoom;
    }
    let mut y = first(area.min.y, anchor.y);
    while y < area.max.y {
        painter.hline(area.x_range(), y, stroke);
        y += zoom;
    }
}
