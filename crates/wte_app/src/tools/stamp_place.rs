//! Stamp and Text placement (ARCHITECTURE.md 8.6): a click puts a new image or text decal on the model (projected) or
//! on the texture (UV placement) as its own layer; the hover shows its outline first. Round 4 (FEATURE_TEXT 5.11,
//! 5.14): the Text tool places the anchor of the text at the click, and the hover shows a see-through picture of the
//! text inside the outline.

use std::sync::{Arc, Mutex};

use eframe::egui;
use glam::{Vec2, Vec3};
use wte_core::{
    Outline, Placement, Projector, StampId, StampSource, SurfaceMode, TEXT_PLACEMENT_PX, TextFrame, TextFx, TextRaster,
    TextSpec, TextureSetId, UvPlacement,
};

use wte_core::text::place_anchor_at_centre;

use crate::action::AppAction;
use crate::state::{AppState, StampToolSource, TextSettings, ToolKind, ViewId};
use crate::tools::{NO_UV_TEXT, Surface, Tool, ViewPointer, ViewXform, error_text, set_size};
use crate::view::draw_quad;

/// Wheel step of the size (x1.1 per notch) and the rotation (5 degrees per notch).
const SIZE_STEP: f32 = 1.1;
const ROTATE_STEP_DEG: f32 = 5.0;
/// Limits of `size_frac` / `em_frac`.
const MIN_FRAC: f32 = 0.002;
const MAX_FRAC: f32 = 4.0;
/// Largest hover picture (FEATURE_TEXT 5.14): `1 << 18` pixels, unless the full picture is smaller.
pub(crate) const HOVER_PREVIEW_PIXELS: u32 = 1 << 18;
/// Opacity of the hover picture.
const HOVER_OPACITY: f32 = 0.7;
/// Cells per side of the textured mesh of the hover picture in the 3D view.
const HOVER_CELLS: usize = 8;

/// The outline of the decal a click would place.
#[derive(Copy, Clone, Debug)]
enum PreviewOutline {
    World([Vec3; 4]),
    Px([Vec2; 4]),
}

/// What the measured source size depends on.
#[derive(Clone, Debug, PartialEq)]
enum SourceKey {
    Image(wte_core::BlobId),
    Text(TextSettings),
    /// Stickers P6b: a tray sticker, with its artboard size (an edit before placing can change it).
    Sticker(wte_core::StickerId, [u32; 2]),
}

/// The measured source: its pixel size and, for a text, its frame (anchor and pixels per em).
#[derive(Copy, Clone, Debug)]
struct Measured {
    size: [u32; 2],
    frame: Option<TextFrame>,
}

/// Stamp and Text: places a new image or text stamp at the cursor.
#[derive(Default)]
pub struct StampPlaceTool {
    preview: Option<(ViewId, PreviewOutline)>,
    /// Source pixel size of the last measured source.
    size_cache: Option<(SourceKey, Measured)>,
    /// The Text tool's hover picture (FEATURE_TEXT 5.14): the settings it was made for, its spec and the picture.
    hover_pic: Option<(SourceKey, TextSpec, Arc<TextRaster>)>,
    /// The egui texture of the hover picture, uploaded from `overlay` (which has `&self` only) when the picture
    /// changed; the `Arc` keeps the picture it was made from.
    hover_texture: Mutex<Option<(Arc<TextRaster>, egui::TextureHandle)>>,
    /// The text draft whose frame could not be measured, and the one whose hover picture could not be made: not tried
    /// again on every hover frame, only when the draft changes (a text too large for every size costs a layout per try).
    size_failed: Option<SourceKey>,
    picture_failed: Option<SourceKey>,
}

/// Builds the text source of the Text tool: a `TextSpec` from `tools.text` at `TEXT_PLACEMENT_PX`, with the new
/// settings of round 4 (thickening, slant, warp, line; no gap values: those are edited on a placed text). The font
/// is looked up in the open project first (`Document::find_font`, FEATURE_TEXT 5.13: a font that only the project
/// holds can be used again, and the project's copy wins), then in the catalog; when neither has it the bundled
/// default is used and, if `warn`, a toast says so.
pub(crate) fn text_source(state: &mut AppState, warn: bool) -> Option<StampSource> {
    let t = state.tools.text.clone();
    let known = state.doc().and_then(|d| d.find_font(&t.family, &t.style));
    let face = match known {
        Some(_) => None,
        None => state.fonts.find(&t.family, &t.style).and_then(|f| state.fonts.face_bytes(f.id)),
    };
    let doc = state.session.as_mut().map(|s| &mut s.doc)?;
    let font = match (known, face) {
        (Some(font), _) => font,
        (None, Some((bytes, face_index))) => doc.register_font(&t.family, &t.style, face_index, bytes),
        (None, None) => {
            if warn {
                state.toasts.warn(format!("Font {} not found, using Ubuntu Light", t.family));
            }
            doc.default_font()
        }
    };
    let outline = t.outline.then_some(Outline { width_em: t.outline_width_em, color: t.outline_color });
    let fx = TextFx { gaps: Vec::new(), bold_em: t.bold_em, slant_deg: t.slant_deg, warp: t.warp, path: t.path.clone() };
    Some(StampSource::Text(TextSpec {
        content: t.content.clone(),
        font,
        size_px: TEXT_PLACEMENT_PX,
        color: t.color,
        align: t.align,
        line_spacing: t.line_spacing,
        letter_spacing: t.letter_spacing,
        outline,
        fx,
    }))
}

/// A placement built around the click, moved so that the text's anchor lies at the click (FEATURE_TEXT 5.11): for a
/// plain text the anchor is the picture centre and nothing changes.
fn anchored(mut placement: Placement, frame: Option<&TextFrame>) -> Placement {
    if let Some(frame) = frame {
        place_anchor_at_centre(&mut placement, frame);
    }
    placement
}

/// The name of a new image stamp (and of its layers) from the chosen image's file name: the file stem, so the layer
/// reads "logo" and its per-layer export "..._stamp_logo.png" (accept_fixes item D). The blob keeps the full file name
/// (the project's embedded file, Extract sources). "Stamp" for an empty name; a name that is all extension
/// (".png") stays as it is.
pub(crate) fn image_stamp_name(file_name: &str) -> String {
    let name = file_name.trim();
    if name.is_empty() {
        return "Stamp".to_owned();
    }
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().trim().to_owned()).unwrap_or_default();
    if stem.is_empty() { name.to_owned() } else { stem }
}

/// The warning for a stamp or text (`what`) whose bake covers no texel (also shown by `stamp_bakes` when the first
/// bake of a new stamp lands on a worker, round 5).
pub(crate) fn no_texels_text(what: &str) -> String {
    format!(
        "The {what} was added as a layer but covers no texels here (the surface faces away, is hidden, or has no room in the texture)"
    )
}

/// True when no linked layer of stamp `id` received a single texel.
fn bake_is_empty(doc: &wte_core::Document, id: StampId) -> bool {
    doc.stamp_layers(id).into_iter().filter_map(|(_, layer)| doc.layer(layer)).all(|layer| match &layer.kind {
        wte_core::LayerKind::Stamp { cache, .. } | wte_core::LayerKind::Text { cache, .. } => cache.is_empty(),
        wte_core::LayerKind::Raster(_) => true,
    })
}

/// Pixels per em of a measured text picture: its frame's (below `TEXT_PLACEMENT_PX` after the too-large fallback),
/// else the placement size of round 3.
fn em_px_of(m: &Measured) -> f32 {
    m.frame.map_or(TEXT_PLACEMENT_PX, |f| f.em_px)
}

/// Half size (world) of a projected decal from the source size (8.6).
fn projected_half_size(state: &AppState, diag: f32, m: &Measured) -> Vec2 {
    let [w, h] = m.size;
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    if state.tool == ToolKind::Text {
        let hy = state.tools.text.em_frac * diag * h / em_px_of(m) / 2.0;
        Vec2::new(hy * w / h, hy)
    } else {
        let m = state.tools.stamp.size_frac * diag / 2.0;
        if w >= h { Vec2::new(m, m * h / w) } else { Vec2::new(m * w / h, m) }
    }
}

/// Size (texels) of a UV decal from the source size (8.6).
fn uv_size(state: &AppState, tex: [u32; 2], m: &Measured) -> Vec2 {
    let [w, h] = m.size;
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    if state.tool == ToolKind::Text {
        let sy = state.tools.text.em_frac * tex[1] as f32 * h / em_px_of(m);
        Vec2::new(sy * w / h, sy)
    } else {
        let m = state.tools.stamp.size_frac * tex[0] as f32;
        if w >= h { Vec2::new(m, m * h / w) } else { Vec2::new(m * w / h, m) }
    }
}

impl StampPlaceTool {
    /// Start of a frame: clears the hover outline.
    pub(crate) fn begin_frame(&mut self) {
        self.preview = None;
    }

    /// Key of the current source; `None` when there is nothing to place (no image chosen, empty text).
    fn source_key(state: &AppState) -> Option<SourceKey> {
        match state.tool {
            ToolKind::Text if !state.tools.text.content.trim().is_empty() => Some(SourceKey::Text(state.tools.text.clone())),
            ToolKind::Stamp => match state.tools.stamp.source {
                StampToolSource::None => None,
                StampToolSource::Blob(blob) => Some(SourceKey::Image(blob)),
                StampToolSource::Sticker(id) => Some(SourceKey::Sticker(id, state.doc()?.sticker(id)?.size())),
            },
            _ => None,
        }
    }

    /// The source to place and its stamp name.
    fn source(state: &mut AppState, warn: bool) -> Option<(StampSource, String)> {
        match state.tool {
            ToolKind::Text => {
                // The automatic name; `set_stamp_source` keeps it following the text while nobody renames it (R3-9).
                let name = wte_core::text_stamp_name(&state.tools.text.content);
                text_source(state, warn).map(|s| (s, name))
            }
            ToolKind::Stamp => match state.tools.stamp.source {
                StampToolSource::None => None,
                StampToolSource::Blob(blob) => Some((StampSource::Image { blob }, image_stamp_name(&state.tools.stamp.source_name))),
                // Stickers P6b (3.6): a tray sticker; the stamp is named after the sticker.
                StampToolSource::Sticker(sticker) => {
                    let name = state.doc()?.sticker(sticker)?.name.trim().to_owned();
                    let name = if name.is_empty() { "Sticker".to_owned() } else { name };
                    Some((StampSource::Sticker { sticker }, name))
                }
            },
            _ => None,
        }
    }

    /// Pixel size (and, for a text, frame) of the current source, measured once per source. With `with_picture` the
    /// Text tool's hover picture is made too when the draft changed (FEATURE_TEXT 5.14), from the same source.
    fn source_size(&mut self, state: &mut AppState, source: Option<&StampSource>, with_picture: bool) -> Option<Measured> {
        if state.tool != ToolKind::Text && self.hover_pic.is_some() {
            self.forget_picture(state);
        }
        let Some(key) = Self::source_key(state) else {
            self.forget_picture(state);
            return None;
        };
        if self.size_failed.as_ref() == Some(&key) {
            return None;
        }
        let cached = self.size_cache.as_ref().filter(|(k, _)| *k == key).map(|(_, m)| *m);
        let wants_picture = with_picture && state.tool == ToolKind::Text;
        let picture_current = !wants_picture
            || self.hover_pic.as_ref().is_some_and(|(k, _, _)| *k == key)
            || self.picture_failed.as_ref() == Some(&key);
        if let Some(m) = cached
            && picture_current
        {
            return Some(m);
        }
        let built;
        let source = match source {
            Some(s) => s,
            None => {
                built = Self::source(state, false)?.0;
                &built
            }
        };
        if !picture_current {
            self.make_picture(state, key.clone(), source);
        }
        if let Some(m) = cached {
            return Some(m);
        }
        let doc = state.doc_mut()?;
        let measured = match source {
            // The frame `add_stamp` will draw (with the too-large fallback): its size, anchor and pixels per em.
            StampSource::Text(_) => {
                let frame = doc.stamp_source_frame(source).ok();
                if frame.is_none() {
                    self.size_failed = Some(key.clone());
                }
                frame.map(|f| Measured { size: f.size, frame: Some(f) })
            }
            StampSource::Image { .. } | StampSource::Sticker { .. } => {
                doc.stamp_source_size(source).ok().map(|size| Measured { size, frame: None })
            }
        }?;
        if measured.size[0] == 0 || measured.size[1] == 0 {
            return None;
        }
        self.size_cache = Some((key, measured));
        Some(measured)
    }

    /// Makes the hover picture of the draft `source` (`Document::text_preview`, at most `HOVER_PREVIEW_PIXELS` pixels
    /// unless the full picture is smaller) and publishes its spec and size in `tool_probe.text_hover`.
    fn make_picture(&mut self, state: &mut AppState, key: SourceKey, source: &StampSource) {
        let StampSource::Text(spec) = source else { return };
        let Some(doc) = state.doc_mut() else { return };
        match doc.text_preview(spec, HOVER_PREVIEW_PIXELS) {
            Ok(raster) => {
                state.tool_probe.text_hover = Some((spec.clone(), [raster.image.width, raster.image.height]));
                self.hover_pic = Some((key, spec.clone(), raster));
                self.picture_failed = None;
            }
            Err(e) => {
                log::debug!("no hover picture for the draft: {e}");
                self.forget_picture(state);
                self.picture_failed = Some(key);
            }
        }
    }

    /// Drops the hover picture (another tool, an empty draft, a picture that failed).
    fn forget_picture(&mut self, state: &mut AppState) {
        self.hover_pic = None;
        if state.tool_probe.text_hover.is_some() {
            state.tool_probe.text_hover = None;
        }
    }

    /// The hover picture the Text tool holds (FEATURE_TEXT 5.14): the spec it was made for and the picture.
    pub(crate) fn hover_preview(&self) -> Option<(&TextSpec, &Arc<TextRaster>)> {
        self.hover_pic.as_ref().map(|(_, spec, raster)| (spec, raster))
    }

    /// The projector of a decal clicked at `hit` (8.6).
    fn projector(state: &AppState, xf: &ViewXform, hit: &wte_core::Hit, m: &Measured) -> Option<Projector> {
        let doc = state.doc()?;
        let diag = doc.mesh.bounds.diagonal();
        let half = projected_half_size(state, diag, m);
        let (_, up, _) = xf.camera.basis();
        let mut p = Projector::from_surface(hit.position, hit.normal, up, half, half.max_element())
            .rotated(state.tools.stamp.rotation_deg.to_radians());
        // Round 6 (R6-2): the mode and the Wrap options of the tool. A wrapped decal lies on the surface: the
        // projection settings below (back face, project through) are for Project only.
        p.mode = state.tools.stamp.mode;
        p.wrap = state.tools.stamp.wrap;
        if p.mode == SurfaceMode::Wrap {
            return Some(p);
        }
        if !hit.front_face {
            p.backface_cull = false;
        }
        if state.tools.stamp.project_through {
            p.backface_cull = false;
            p.occlusion = false;
            p.depth_back = diag;
        }
        Some(p)
    }

    /// The UV placement of a decal clicked at texel `px` of `set`.
    fn uv_placement(state: &AppState, set: TextureSetId, px: Vec2, m: &Measured) -> Option<UvPlacement> {
        let tex = set_size(state.doc()?, set);
        Some(UvPlacement {
            set,
            center_px: px,
            size_px: uv_size(state, tex, m),
            rotation: state.tools.stamp.rotation_deg.to_radians(),
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        })
    }

    /// The egui texture of the hover picture, uploaded when the picture changed.
    fn picture_texture(&self, ctx: &egui::Context) -> Option<egui::TextureId> {
        let (_, raster) = self.hover_preview()?;
        let mut slot = self.hover_texture.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((held, handle)) = slot.as_ref()
            && Arc::ptr_eq(held, raster)
        {
            return Some(handle.id());
        }
        let img = &raster.image;
        let (w, h) = (img.width as usize, img.height as usize);
        if w == 0 || h == 0 || img.data.len() != w * h * 4 {
            return None;
        }
        // The picture is straight alpha.
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &img.data);
        let handle = ctx.load_texture("text_hover_picture", image, egui::TextureOptions::LINEAR);
        let id = handle.id();
        *slot = Some((Arc::clone(raster), handle));
        Some(id)
    }

    /// The see-through picture of the text to place, inside its outline (FEATURE_TEXT 5.14): at 70 % opacity, as a
    /// textured mesh. In the 2D view the rotated rectangle; in the 3D view the placement rectangle on the plane under
    /// the pointer, cut into 8 x 8 cells whose corners are projected to the screen (a cell with a corner behind the
    /// camera is left out). It does not show how the text wraps around the surface: the bake after the click does.
    fn draw_picture(&self, xf: &ViewXform, painter: &egui::Painter, outline: &PreviewOutline) {
        let Some(texture) = self.picture_texture(painter.ctx()) else { return };
        let cells = match outline {
            PreviewOutline::World(_) => HOVER_CELLS,
            PreviewOutline::Px(_) => 1,
        };
        let point = |u: f32, v: f32| -> Option<egui::Pos2> {
            match outline {
                PreviewOutline::World([tl, tr, br, bl]) => {
                    xf.world_to_screen(tl.lerp(*tr, u).lerp(bl.lerp(*br, u), v))
                }
                PreviewOutline::Px([tl, tr, br, bl]) => Some(xf.px_to_screen(tl.lerp(*tr, u).lerp(bl.lerp(*br, u), v))),
            }
        };
        let color = egui::Color32::WHITE.gamma_multiply(HOVER_OPACITY);
        let mut mesh = egui::Mesh::with_texture(texture);
        let n = cells + 1;
        let mut ok = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let (u, v) = (i as f32 / cells as f32, j as f32 / cells as f32);
                let pos = point(u, v);
                ok.push(pos.is_some());
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: pos.unwrap_or(egui::Pos2::ZERO),
                    uv: egui::pos2(u, v),
                    color,
                });
            }
        }
        for j in 0..cells {
            for i in 0..cells {
                let [a, b, c, d] = [j * n + i, j * n + i + 1, (j + 1) * n + i + 1, (j + 1) * n + i];
                if [a, b, c, d].iter().all(|&k| ok[k]) {
                    mesh.add_triangle(a as u32, b as u32, c as u32);
                    mesh.add_triangle(a as u32, c as u32, d as u32);
                }
            }
        }
        if !mesh.indices.is_empty() {
            painter.add(egui::Shape::mesh(mesh));
        }
    }

    /// "text" or "stamp", for messages.
    fn what(state: &AppState) -> &'static str {
        if state.tool == ToolKind::Text { "text" } else { "stamp" }
    }

    /// The new stamp: selected, its home set active, and the tool switches to Transform.
    fn placed(state: &mut AppState, id: StampId, home: TextureSetId) {
        if let Some(session) = state.session.as_mut() {
            session.selected_stamp = Some(id);
            session.active_set = home;
        }
        state.tool = ToolKind::Transform;
        state.status_hint = "Drag the handles to move, scale or rotate the stamp".to_owned();
    }
}

impl Tool for StampPlaceTool {
    fn hover(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        self.preview = None;
        let Some(m) = self.source_size(state, None, true) else { return };
        let placement = match (&p.surface, xf.is_3d) {
            (Surface::Model(hit), true) if hit.texture_set.is_some() => {
                Self::projector(state, xf, hit, &m).map(Placement::Projected)
            }
            (Surface::Uv { set, px, .. }, false) => Self::uv_placement(state, *set, *px, &m).map(Placement::Uv),
            _ => None,
        };
        // The outline is the rectangle a click would place: around the anchor at the pointer.
        let outline = placement.map(|pl| match anchored(pl, m.frame.as_ref()) {
            Placement::Projected(pr) => PreviewOutline::World(pr.corners()),
            Placement::Uv(u) => PreviewOutline::Px(u.corners_px()),
        });
        self.preview = outline.map(|o| (xf.view, o));
    }

    fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) -> bool {
        if state.tool == ToolKind::Text && state.tools.text.content.trim().is_empty() {
            state.status_hint = "Type the text in the tool options first".to_owned();
            state.focus_text_content = true;
            return false;
        }
        if state.tool == ToolKind::Stamp && state.tools.stamp.source.is_none() {
            state.push(AppAction::ImportStampImage(None));
            return false;
        }
        // Where the click landed: on the model (projected decal) or on the texture (UV decal).
        let (home, hit) = match (p.surface, xf.is_3d) {
            (Surface::Model(hit), true) => match hit.texture_set {
                Some(set) => (set, Some(hit)),
                None => {
                    state.toasts.warn(NO_UV_TEXT);
                    return false;
                }
            },
            (Surface::Uv { set, .. }, false) => (set, None),
            _ => {
                log::info!("{} press at {:?} hits nothing", Self::what(state), p.pos);
                // A click between the parts of the model or beside the texture: say why nothing was placed.
                let place = if xf.is_3d { "on the model" } else { "on the texture" };
                state.toasts.info(format!("Nothing here to put the {} on: click {place}", Self::what(state)));
                return false;
            }
        };
        let Some((source, name)) = Self::source(state, true) else {
            state.toasts.warn(format!("The {} has nothing to place", Self::what(state)));
            return false;
        };
        let Some(m) = self.source_size(state, Some(&source), false) else {
            state.toasts.warn("The stamp source has no pixels");
            return false;
        };
        // A text is placed with its anchor at the click (FEATURE_TEXT 5.11; a plain text: the centre, as before).
        let placement = match (hit, p.surface) {
            // The depth range is fitted to the surface under the footprint (6.5); project-through keeps its own.
            (Some(hit), _) => Self::projector(state, xf, &hit, &m).map(|pr| {
                let pr = match anchored(Placement::Projected(pr), m.frame.as_ref()) {
                    Placement::Projected(moved) => moved,
                    Placement::Uv(_) => pr,
                };
                // Round 6: a wrapped decal gets the fitted depth too (switching it back to Project gives a fitted
                // projection); project-through is a Project setting.
                let through = state.tools.stamp.project_through && pr.mode == SurfaceMode::Project;
                let fitted = if through { pr } else { state.doc().map_or(pr, |d| d.fit_stamp_depth(&pr)) };
                Placement::Projected(fitted)
            }),
            (None, Surface::Uv { px, .. }) => {
                Self::uv_placement(state, home, px, &m).map(|u| anchored(Placement::Uv(u), m.frame.as_ref()))
            }
            (None, _) => None,
        };
        let Some(placement) = placement else {
            state.toasts.warn(format!("The {} could not be placed here", Self::what(state)));
            return false;
        };
        let what = Self::what(state);
        // Stickers P6b (3.6): new stamps of the Stamp tool get the tool's styles (none by default: exactly `add_stamp`).
        let styles = if state.tool == ToolKind::Stamp { state.tools.stamp.styles.clone() } else { wte_core::StyleStack::default() };
        let Some(doc) = state.doc_mut() else { return false };
        match doc.add_stamp_styled(&name, source, placement, home, styles) {
            Ok(id) if doc.stamp_is_unbaked(id) => {
                // Round 5 (R5-1): the bake runs on a worker; the warning below comes when it lands (`stamp_bakes`).
                // The gizmo frame shows at once where the decal will appear.
                log::info!("{what} {name:?} placed as stamp {} ({}, bake pending)", id.0, if hit.is_some() { "3D" } else { "2D" });
                state.await_first_bake = Some(id);
                Self::placed(state, id, home);
            }
            Ok(id) => {
                let empty = bake_is_empty(doc, id);
                log::info!("{what} {name:?} placed as stamp {} ({}, empty: {empty})", id.0, if hit.is_some() { "3D" } else { "2D" });
                if empty {
                    state.toasts.warn(no_texels_text(what));
                }
                Self::placed(state, id, home);
            }
            Err(e) => state.toasts.warn(error_text(&e)),
        }
        self.preview = None;
        false
    }

    fn scroll(&mut self, state: &mut AppState, _xf: &ViewXform, p: &ViewPointer, notches: f32) -> bool {
        if p.mods.ctrl || p.mods.command {
            let frac = if state.tool == ToolKind::Text { &mut state.tools.text.em_frac } else { &mut state.tools.stamp.size_frac };
            *frac = (*frac * SIZE_STEP.powf(notches)).clamp(MIN_FRAC, MAX_FRAC);
            true
        } else if p.mods.shift {
            let r = state.tools.stamp.rotation_deg + ROTATE_STEP_DEG * notches;
            // Keep the angle in (-180, 180].
            state.tools.stamp.rotation_deg = -((180.0 - r).rem_euclid(360.0) - 180.0);
            true
        } else {
            false
        }
    }

    fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter) {
        let Some((view, outline)) = &self.preview else { return };
        if *view != xf.view {
            return;
        }
        if state.tool == ToolKind::Text {
            self.draw_picture(xf, painter, outline);
        }
        let corners = match outline {
            PreviewOutline::World(w) => {
                let [a, b, c, d] = w.map(|p| xf.world_to_screen(p));
                match (a, b, c, d) {
                    (Some(a), Some(b), Some(c), Some(d)) => [a, b, c, d],
                    _ => return,
                }
            }
            PreviewOutline::Px(px) => px.map(|p| xf.px_to_screen(p)),
        };
        draw_quad(painter, corners, egui::Color32::from_rgb(255, 210, 90), 1.25);
        // Mark the top edge so the orientation is visible before placing.
        let top = corners[0].lerp(corners[1], 0.5);
        painter.circle_filled(top, 3.0, egui::Color32::from_rgb(255, 210, 90));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolController;
    use crate::tools::test_support::*;
    use std::sync::Arc;
    use wte_core::{LayerKind, SampleKind, encode_png, samples};

    fn with_image(state: &mut AppState) {
        let img = samples::sample_stamp_image(64);
        let png = encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let blob = state.doc_mut().unwrap().add_image_blob("ring.png", Arc::from(png)).unwrap();
        state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
        state.tools.stamp.source_name = "ring.png".to_owned();
    }

    #[test]
    fn an_image_stamp_is_named_after_the_file_stem() {
        assert_eq!(image_stamp_name("wf_stamp.png"), "wf_stamp");
        assert_eq!(image_stamp_name(" logo.v2.PNG "), "logo.v2");
        assert_eq!(image_stamp_name("cells"), "cells");
        assert_eq!(image_stamp_name(""), "Stamp");
        assert_eq!(image_stamp_name("   "), "Stamp");
        assert_eq!(image_stamp_name(".png"), ".png");
    }

    #[test]
    fn image_stamp_in_3d_becomes_a_layer_and_selects_it() {
        let mut state = state_with(SampleKind::Sphere);
        with_image(&mut state);
        state.tool = ToolKind::Stamp;
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert!(tools.stamp_place.preview.is_some(), "hover outline");
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy());
        let session = state.session.as_ref().unwrap();
        let id = session.selected_stamp.expect("new stamp selected");
        let stamp = session.doc.stamp(id).unwrap();
        // accept_fixes item D: the layer is named after the file stem (its export is "..._stamp_ring.png").
        assert_eq!(stamp.name, "ring");
        let Placement::Projected(pr) = stamp.placement else { panic!("projected") };
        let diag = session.doc.mesh.bounds.diagonal();
        assert!((pr.half_size.max_element() - 0.15 * diag / 2.0).abs() < 1e-4);
        assert_eq!(state.tool, ToolKind::Transform);
        let set = &session.doc.sets[0];
        assert_eq!(set.layers.len(), 3);
        assert!(matches!(set.layers[2].kind, LayerKind::Stamp { .. }));
        assert_eq!(session.doc.history.undo_len(), 1);
    }

    /// Stickers P6b (STICKER_PLAN 3.6): a `Blob` source with default styles is exactly an image stamp; with the tool's
    /// styles the stamp gets them in the same one entry; a tray sticker places a sticker stamp (twice: both show it).
    #[test]
    fn the_stamp_tool_places_images_with_their_styles_and_tray_stickers() {
        let mut state = state_with(SampleKind::Sphere);
        with_image(&mut state);
        state.tool = ToolKind::Stamp;
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let plain = state.session.as_ref().unwrap().selected_stamp.unwrap();
        let object = state.doc().unwrap().stamp(plain).unwrap().clone();
        assert!(matches!(object.source, StampSource::Image { .. }) && object.styles.is_default(), "{object:?}");

        // With the tool's styles: one entry, the styles on the stamp, the source an image.
        let styles = wte_core::StylePreset::ClassicSticker.styles([64, 64]);
        state.tools.stamp.styles = styles.clone();
        state.tool = ToolKind::Stamp;
        let undo = state.doc().unwrap().history.undo_len();
        let p = pointer(&state, &xf, at(&xf, 0.45, 0.45), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let styled = state.session.as_ref().unwrap().selected_stamp.unwrap();
        assert_ne!(styled, plain);
        let object = state.doc().unwrap().stamp(styled).unwrap().clone();
        assert!(matches!(object.source, StampSource::Image { .. }));
        assert_eq!(object.styles, styles.sanitized());
        assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1, "one entry");

        // A tray sticker, placed twice: two sticker stamps showing the same sticker.
        let blob = state.tools.stamp.source.blob().unwrap();
        let size = state.doc_mut().unwrap().image_size(blob).unwrap();
        let mut sticker = wte_core::StickerDoc::from_picture("Tray one", wte_core::PicturePixels::Blob(blob), size);
        sticker.keep = true;
        let id = state.doc_mut().unwrap().add_sticker(sticker).unwrap();
        state.tools.stamp.source = crate::state::StampToolSource::Sticker(id);
        state.tools.stamp.styles = wte_core::StyleStack::default();
        let mut placed = Vec::new();
        for x in [0.4, 0.6] {
            state.tool = ToolKind::Stamp;
            let p = pointer(&state, &xf, at(&xf, x, 0.55), egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            placed.push(state.session.as_ref().unwrap().selected_stamp.unwrap());
        }
        let doc = state.doc().unwrap();
        for s in &placed {
            let object = doc.stamp(*s).unwrap();
            assert_eq!(object.source, StampSource::Sticker { sticker: id });
            assert_eq!(object.name, "Tray one");
        }
        assert_eq!(doc.sticker_users(id), placed);
    }

    #[test]
    fn text_stamp_in_2d_and_empty_text_press() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE".to_owned();
        let xf = xf_2d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let session = state.session.as_ref().unwrap();
        let stamp = session.doc.stamp(session.selected_stamp.unwrap()).unwrap();
        assert_eq!(stamp.name, "WTE");
        let Placement::Uv(u) = stamp.placement else { panic!("uv") };
        let tex_h = session.doc.sets[0].height as f32;
        // One em is em_frac of the texture height: the 1-line text is a bit taller than that (padding).
        assert!(u.size_px.y > 0.06 * tex_h * 0.8 && u.size_px.y < 0.06 * tex_h * 3.0, "{:?}", u.size_px);
        assert!(u.size_px.x > u.size_px.y);
        assert!(matches!(session.doc.sets[0].layers.last().unwrap().kind, LayerKind::Text { .. }));
        let undo = session.doc.history.undo_len();
        let stamps = session.doc.stamps.len();

        state.tool = ToolKind::Text;
        state.tools.text.content = "  \n ".to_owned();
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let doc = state.doc().unwrap();
        assert_eq!((doc.history.undo_len(), doc.stamps.len()), (undo, stamps));
        assert!(state.focus_text_content);
        assert_eq!(state.status_hint, "Type the text in the tool options first");
    }

    #[test]
    fn stamp_without_image_asks_for_one_and_no_uv_warns() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Stamp;
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(matches!(state.actions.as_slice(), [AppAction::ImportStampImage(None)]));
        assert!(state.doc().unwrap().stamps.is_empty());
    }

    #[test]
    fn wheel_changes_size_and_rotation() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Stamp;
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::CTRL);
        assert!(tools.scroll(&mut state, &xf, &p, 2.0));
        assert!((state.tools.stamp.size_frac - 0.15 * 1.21).abs() < 1e-5);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::SHIFT);
        assert!(tools.scroll(&mut state, &xf, &p, -3.0));
        assert!((state.tools.stamp.rotation_deg + 15.0).abs() < 1e-4);
        state.tools.stamp.rotation_deg = 178.0;
        tools.scroll(&mut state, &xf, &p, 1.0);
        assert!((state.tools.stamp.rotation_deg + 177.0).abs() < 1e-4);
        state.tool = ToolKind::Text;
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::CTRL);
        tools.scroll(&mut state, &xf, &p, -1.0);
        assert!((state.tools.text.em_frac - 0.06 / 1.1).abs() < 1e-5);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        assert!(!tools.scroll(&mut state, &xf, &p, 1.0));
    }

    #[test]
    fn missing_font_falls_back_with_a_warning() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.family = "No Such Font Family".to_owned();
        let source = text_source(&mut state, true).unwrap();
        let StampSource::Text(spec) = source else { panic!() };
        assert_eq!(spec.font.family, wte_core::DEFAULT_FONT_FAMILY);
        assert_eq!(spec.size_px, TEXT_PLACEMENT_PX);
        assert_eq!(state.toasts.error_count(), 0);
    }

    #[test]
    fn a_click_that_hits_nothing_says_why_nothing_was_placed() {
        // Fix round 2, item 4: a click between the parts of a model (many_islands.obj) used to do nothing silently.
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE".to_owned();
        let mut tools = ToolController::new();
        let xf = xf_3d(&mut state);
        let p = pointer(&state, &xf, xf.rect.min + egui::vec2(3.0, 3.0), egui::Modifiers::NONE);
        assert!(matches!(p.surface, Surface::Miss));
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(state.doc().unwrap().stamps.is_empty());
        assert!(state.toasts.visible_texts().contains(&"Nothing here to put the text on: click on the model"));

        state.tool = ToolKind::Stamp;
        with_image(&mut state);
        let xf = xf_2d(&mut state);
        let p = pointer(&state, &xf, xf.rect.min + egui::vec2(1.0, 1.0), egui::Modifiers::NONE);
        assert!(matches!(p.surface, Surface::Miss));
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(state.doc().unwrap().stamps.is_empty());
        assert!(state.toasts.visible_texts().contains(&"Nothing here to put the stamp on: click on the texture"));
    }

    #[test]
    fn a_placed_3d_stamp_gets_its_depth_fitted_and_project_through_keeps_its_own() {
        for through in [false, true] {
            let mut state = state_with(SampleKind::Sphere);
            with_image(&mut state);
            state.tool = ToolKind::Stamp;
            state.tools.stamp.project_through = through;
            let xf = xf_3d(&mut state);
            let mut tools = ToolController::new();
            let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(40.0, -30.0), egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            let doc = state.doc().unwrap();
            let id = state.session.as_ref().unwrap().selected_stamp.expect("placed");
            assert!(!bake_is_empty(doc, id));
            let Placement::Projected(pr) = doc.stamp(id).unwrap().placement else { panic!() };
            if through {
                assert_eq!(pr.depth_back, doc.mesh.bounds.diagonal());
                assert!(!pr.backface_cull && !pr.occlusion);
            } else {
                assert!(doc.stamp_depth_is_auto(&pr));
                let fitted = doc.fit_stamp_depth(&pr);
                assert_eq!((pr.depth_front, pr.depth_back), (fitted.depth_front, fitted.depth_back));
                // On the gently curved sphere the fitted box is shallower than the old default (the half size)
                // but still reaches the corners of the footprint.
                assert!(pr.depth_back < pr.half_size.max_element(), "{pr:?}");
                assert!(pr.depth_back > 0.05 * pr.half_size.max_element(), "{pr:?}");
            }
        }
    }

    /// Round 6 (R6-2, ROUND6_PLAN 5.3): new 3D images and texts carry the tool's mode and Wrap options; a wrapped one
    /// still gets the fitted depth (for switching back) and ignores project-through; its bake leaves a
    /// parameterization in the cache. Back in Project the next stamp is projected again with the options kept.
    #[test]
    fn new_placements_carry_the_tools_mode_and_wrap_options() {
        use wte_core::WrapOptions;
        for text in [false, true] {
            let mut state = state_with(SampleKind::Sphere);
            if text {
                state.tool = ToolKind::Text;
                state.tools.text.content = "WRAP".to_owned();
            } else {
                with_image(&mut state);
                state.tool = ToolKind::Stamp;
            }
            let opts = WrapOptions { jump_gaps: true, overlap: true };
            state.tools.stamp.mode = SurfaceMode::Wrap;
            state.tools.stamp.wrap = opts;
            state.tools.stamp.project_through = true;
            let xf = xf_3d(&mut state);
            let mut tools = ToolController::new();
            let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(-30.0, 20.0), egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            let doc = state.doc().unwrap();
            let id = state.session.as_ref().unwrap().selected_stamp.expect("placed");
            let Placement::Projected(pr) = doc.stamp(id).unwrap().placement else { panic!("projected") };
            assert_eq!((pr.mode, pr.wrap), (SurfaceMode::Wrap, opts), "text: {text}");
            assert!(doc.stamp_depth_is_auto(&pr), "the depth is fitted, project-through ignored: {pr:?}");
            assert!(pr.depth_back < doc.mesh.bounds.diagonal());
            assert!(!bake_is_empty(doc, id));
            let stats = doc.stamp_wrap_stats(id).expect("the bake cached its parameterization");
            assert!(stats.covered > 0.99 && stats.tris > 0, "{stats:?}");

            state.tool = if text { ToolKind::Text } else { ToolKind::Stamp };
            state.tools.stamp.mode = SurfaceMode::Project;
            let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(40.0, -30.0), egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            let doc = state.doc().unwrap();
            let second = state.session.as_ref().unwrap().selected_stamp.expect("placed");
            assert_ne!(second, id);
            let Placement::Projected(pr) = doc.stamp(second).unwrap().placement else { panic!("projected") };
            assert_eq!((pr.mode, pr.wrap), (SurfaceMode::Project, opts));
            assert_eq!(pr.depth_back, doc.mesh.bounds.diagonal(), "project-through applies in Project");
        }
    }

    /// Check:B (ROUND6_PLAN 3.3, "Warps, text on a path, styles, outline, gaps, bold, slant: change the picture only;
    /// the picture is wrapped as drawn"): on a flat face of the cube sample, where the wrap is exact, a wrapped text
    /// with thickening, slant and an outline, once with an Arc warp and once on a Curve, inks the texels of the same
    /// text projected; switching back to Project gives the projected bytes again.
    #[test]
    fn a_styled_warped_or_curved_text_is_wrapped_as_drawn() {
        use wte_core::{Commit, Outline, PathShape, Warp, WarpStyle};
        let mut state = state_with(SampleKind::Cube);
        let doc = state.doc_mut().unwrap();
        let font = doc.default_font();
        // The +Z face of the 1.5 cube: z = 0.75, x and y in -0.75..0.75.
        let base = Projector::from_surface(Vec3::new(0.0, 0.0, 0.75), Vec3::Z, Vec3::Y, Vec2::new(0.40, 0.10), 0.2);
        let pixels = |doc: &wte_core::Document, id: StampId| -> Vec<u8> {
            let (set, layer) = doc.stamp_layers(id)[0];
            let s = doc.set(set).unwrap();
            doc.layer(layer).unwrap().to_dense(s.width, s.height).data
        };
        for curved in [false, true] {
            let mut spec = TextSpec::new("WRAP ME", font.clone());
            spec.outline = Some(Outline { width_em: 0.05, color: [240, 220, 20, 255] });
            spec.fx.bold_em = 0.02;
            spec.fx.slant_deg = 12.0;
            if !curved {
                spec.fx.warp = Some(Warp { bend: 0.3, ..Warp::new(WarpStyle::Arc) });
            }
            let id = doc.add_stamp("t", StampSource::Text(spec.clone()), Placement::Projected(base), TextureSetId(0)).unwrap();
            if curved {
                let frame = doc.stamp_text_frame(id).unwrap();
                let mut path = wte_core::text::path::new_curve(&frame);
                if let PathShape::Curve { points } = &mut path.shape {
                    points[1][1] += 0.8;
                }
                spec.fx.path = Some(path);
                doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final).unwrap();
            }
            let Placement::Projected(p) = doc.stamp(id).unwrap().placement else { panic!("projected") };
            let reach = (p.origin.truncate().abs() + p.half_size.abs()).max_element();
            assert!(reach < 0.74, "the text lies on the face: {p:?}");
            let projected = pixels(doc, id);
            let wrapped = Placement::Projected(Projector { mode: SurfaceMode::Wrap, ..p });
            doc.set_stamp_placement(id, wrapped, Commit::Final).unwrap();
            let stats = doc.stamp_wrap_stats(id).expect("a parameterization");
            assert!(stats.covered >= 0.9999 && stats.cut_tris == 0, "curved {curved}: {stats:?}");
            let now = pixels(doc, id);
            let (mut inked, mut differ) = (0usize, 0usize);
            for (a, b) in projected.as_chunks::<4>().0.iter().zip(now.as_chunks::<4>().0) {
                if a[3] > 0 || b[3] > 0 {
                    inked += 1;
                    differ += usize::from((0..4).any(|k| a[k].abs_diff(b[k]) > 8));
                }
            }
            eprintln!("check:B: curved {curved}: {inked} inked texels, {differ} differ by more than 8 levels");
            assert!(inked > 2000, "curved {curved}: only {inked} inked texels");
            assert!(differ * 100 <= inked, "curved {curved}: {differ} of {inked} texels differ from the projected text");
            doc.set_stamp_placement(id, Placement::Projected(p), Commit::Final).unwrap();
            assert!(pixels(doc, id) == projected, "curved {curved}: back in Project the bytes of before");
            doc.remove_stamp(id).unwrap();
        }
    }

    #[test]
    fn an_empty_bake_is_reported() {
        let mut state = state_with(SampleKind::Sphere);
        with_image(&mut state);
        let blob = state.tools.stamp.source.blob().unwrap();
        let doc = state.doc_mut().unwrap();
        let c = doc.mesh.bounds.center();
        let r = doc.mesh.bounds.diagonal() * 0.29;
        // A decal in front of the sphere that faces away from it: culled everywhere.
        let away = wte_core::Projector::from_surface(c + Vec3::Z * r, -Vec3::Z, Vec3::Y, Vec2::splat(r * 0.2), r * 0.05);
        let id = doc.add_stamp("away", StampSource::Image { blob }, Placement::Projected(away), TextureSetId(0)).unwrap();
        assert!(bake_is_empty(doc, id));
        let facing = wte_core::Projector { rotation: away.rotation * glam::Quat::from_rotation_y(std::f32::consts::PI), ..away };
        let id = doc.add_stamp("facing", StampSource::Image { blob }, Placement::Projected(facing), TextureSetId(0)).unwrap();
        assert!(!bake_is_empty(doc, id));
    }

    #[test]
    fn a_placed_text_is_named_after_its_first_line_and_follows_its_text() {
        // Fix round 3, R3-9: the tool names the stamp with the core rule, so a later text edit keeps the name current.
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "  \nHello\nWorld".to_owned();
        let xf = xf_2d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let id = state.session.as_ref().unwrap().selected_stamp.unwrap();
        assert_eq!(state.doc().unwrap().stamp(id).unwrap().name, "Hello");

        state.tools.text.content = "Bye".to_owned();
        let source = text_source(&mut state, false).unwrap();
        let doc = state.doc_mut().unwrap();
        doc.set_stamp_source(id, source, wte_core::Commit::Final).unwrap();
        assert_eq!(doc.stamp(id).unwrap().name, "Bye");
        let layers = doc.stamp_layers(id);
        assert!(!layers.is_empty());
        assert!(layers.into_iter().all(|(_, l)| doc.layer(l).unwrap().name == "Bye"));
    }

    // ----- round 4 (FEATURE_TEXT 8.2, the unit tests next to the tools) -----

    fn arc50() -> wte_core::Warp {
        wte_core::Warp { style: wte_core::WarpStyle::Arc, bend: 0.5, horizontal: 0.0, vertical: 0.0, vertical_axis: false }
    }

    fn draft_spec(state: &mut AppState) -> TextSpec {
        let StampSource::Text(spec) = text_source(state, false).unwrap() else { panic!("text source") };
        spec
    }

    /// The anchor of text stamp `id` in texels (UV) or world units (projected).
    fn anchor_of(state: &mut AppState, id: StampId) -> Vec3 {
        let doc = state.doc_mut().unwrap();
        let frame = doc.stamp_text_frame(id).unwrap();
        let placement = doc.stamp(id).unwrap().placement;
        let map = wte_core::text::EmMap::new(&placement, &frame);
        match placement {
            Placement::Projected(_) => map.to_world([0.0, 0.0]),
            Placement::Uv(_) => map.to_texel([0.0, 0.0]).extend(0.0),
        }
    }

    /// The `em_frac` that makes one em about 100 texels on the 2D placement: the core keeps 128 px per em.
    fn em_of_100_texels(state: &AppState) -> f32 {
        100.0 / state.doc().unwrap().sets[0].height as f32
    }

    #[test]
    fn text_source_copies_fx() {
        let mut state = state_with(SampleKind::Sphere);
        let path = wte_core::TextPath { shape: wte_core::PathShape::Arc { bend: 0.25 }, start: 0.5, flip: true };
        let t = &mut state.tools.text;
        t.bold_em = 0.03;
        t.slant_deg = -12.0;
        t.warp = Some(arc50());
        t.path = Some(path.clone());
        let spec = draft_spec(&mut state);
        assert_eq!(spec.fx, TextFx { gaps: Vec::new(), bold_em: 0.03, slant_deg: -12.0, warp: Some(arc50()), path: Some(path) });
        assert_eq!(spec.size_px, TEXT_PLACEMENT_PX);
        // Without the new settings the spec is the plain one of round 3.
        state.tools.text = TextSettings { content: "x".into(), ..TextSettings::default() };
        assert!(draft_spec(&mut state).fx.is_default());
    }

    #[test]
    fn the_draft_takes_a_font_the_project_already_holds() {
        // FEATURE_TEXT 5.13: a family only the project holds (not in the catalog) is found in the document first.
        let mut state = state_with(SampleKind::Sphere);
        let hack = state.fonts.find("Hack", "Regular").and_then(|f| state.fonts.face_bytes(f.id)).unwrap();
        let font = state.doc_mut().unwrap().register_font("Projectonly", "Regular", hack.1, hack.0);
        state.tools.text.family = "Projectonly".into();
        state.tools.text.style = "Regular".into();
        let blobs = state.doc().unwrap().blobs.len();
        let spec = draft_spec(&mut state);
        assert_eq!(spec.font, font);
        assert_eq!(state.doc().unwrap().blobs.len(), blobs, "no blob added");
        assert_eq!(state.toasts.visible_texts().len(), 0, "no toast");
    }

    #[test]
    fn a_plain_draft_is_placed_as_in_round_3() {
        // The round-3 rule: the rectangle centred on the click, sized from the measured picture at TEXT_PLACEMENT_PX.
        for is_3d in [false, true] {
            let mut state = state_with(SampleKind::Sphere);
            state.tool = ToolKind::Text;
            state.tools.text.content = "WTE 2026\nline two".into();
            state.tools.text.align = wte_core::TextAlign::Right;
            let xf = if is_3d { xf_3d(&mut state) } else { xf_2d(&mut state) };
            let pos = xf.rect.center() + egui::vec2(23.0, -17.0);
            let p = pointer(&state, &xf, pos, egui::Modifiers::NONE);
            // The expected placement, built the round-3 way on a second state.
            let mut other = state_with(SampleKind::Sphere);
            other.tool = ToolKind::Text;
            other.tools.text = state.tools.text.clone();
            let source = text_source(&mut other, false).unwrap();
            let (em_frac, rotation_deg) = (other.tools.text.em_frac, other.tools.stamp.rotation_deg);
            let doc = other.doc_mut().unwrap();
            let [w, h] = doc.stamp_source_size(&source).unwrap().map(|v| v as f32);
            let expected = match p.surface {
                Surface::Model(hit) => {
                    let diag = doc.mesh.bounds.diagonal();
                    let hy = em_frac * diag * h / TEXT_PLACEMENT_PX / 2.0;
                    let (_, up, _) = xf.camera.basis();
                    let half = Vec2::new(hy * w / h, hy);
                    let mut pr = Projector::from_surface(hit.position, hit.normal, up, half, half.max_element())
                        .rotated(rotation_deg.to_radians());
                    if !hit.front_face {
                        pr.backface_cull = false;
                    }
                    Placement::Projected(doc.fit_stamp_depth(&pr))
                }
                Surface::Uv { px, .. } => {
                    let sy = em_frac * doc.sets[0].height as f32 * h / TEXT_PLACEMENT_PX;
                    Placement::Uv(UvPlacement {
                        set: TextureSetId(0),
                        center_px: px,
                        size_px: Vec2::new(sy * w / h, sy),
                        rotation: 0.0,
                        flip_x: false,
                        flip_y: false,
                        clip_to_islands: false,
                    })
                }
                Surface::Miss => panic!("the click misses"),
            };
            let want = doc.add_stamp("x", source, expected, TextureSetId(0)).unwrap();
            let want = doc.stamp(want).unwrap().placement;

            let mut tools = ToolController::new();
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            let id = state.session.as_ref().unwrap().selected_stamp.expect("placed");
            assert_eq!(state.doc().unwrap().stamp(id).unwrap().placement, want, "3D: {is_3d}");
        }
    }

    #[test]
    fn a_styled_draft_is_placed_with_its_anchor_at_the_click() {
        // 2D: within 0.5 texel of the clicked texel.
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE 2026".into();
        state.tools.text.warp = Some(arc50());
        state.tools.text.em_frac = em_of_100_texels(&state);
        let xf = xf_2d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(31.0, 12.0), egui::Modifiers::NONE);
        let Surface::Uv { px, .. } = p.surface else { panic!("2D click misses") };
        let mut tools = ToolController::new();
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let id = state.session.as_ref().unwrap().selected_stamp.expect("placed");
        let frame = state.doc_mut().unwrap().stamp_text_frame(id).unwrap();
        let [w, h] = frame.size.map(|v| v as f32);
        assert!((frame.anchor_px[1] - h / 2.0).abs() > 2.0, "the Arc moves the anchor off the centre: {frame:?}");
        let anchor = anchor_of(&mut state, id).truncate();
        assert!((anchor - px).length() < 0.5, "anchor {anchor:?}, click {px:?}");
        let Placement::Uv(u) = state.doc().unwrap().stamp(id).unwrap().placement else { panic!("uv") };
        assert!((u.center_px - px).length() > 2.0, "the rectangle is not centred on the click ({w}x{h})");

        // 3D: the anchor lies at the surface point under the click.
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE 2026".into();
        state.tools.text.warp = Some(arc50());
        let xf = xf_3d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(10.0, -20.0), egui::Modifiers::NONE);
        let Surface::Model(hit) = p.surface else { panic!("3D click misses") };
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let id = state.session.as_ref().unwrap().selected_stamp.expect("placed");
        let anchor = anchor_of(&mut state, id);
        let diag = state.doc().unwrap().mesh.bounds.diagonal();
        assert!((anchor - hit.position).length() < diag * 1e-4, "anchor {anchor:?}, hit {:?}", hit.position);
    }

    #[test]
    fn hover_with_the_text_tool_caches_a_preview_of_the_draft() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE 2026".into();
        state.tools.text.warp = Some(arc50());
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        let draft = draft_spec(&mut state);
        let (spec, first) = tools.stamp_place.hover_preview().expect("a hover picture");
        assert_eq!(*spec, draft);
        let (w, h) = (first.image.width, first.image.height);
        assert!(w * h <= HOVER_PREVIEW_PIXELS, "{w}x{h}");
        assert!(first.image.data.as_chunks::<4>().0.iter().any(|px| px[3] > 0), "no ink");
        assert_eq!(state.tool_probe.text_hover, Some((draft.clone(), [w, h])));
        assert!(tools.stamp_place.preview.is_some(), "the yellow frame is there too");
        let first = Arc::clone(first);
        // Nothing is drawn twice.
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert!(Arc::ptr_eq(tools.stamp_place.hover_preview().unwrap().1, &first));

        // The image Stamp tool holds none.
        with_image(&mut state);
        state.tool = ToolKind::Stamp;
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert!(tools.stamp_place.hover_preview().is_none());
        assert_eq!(state.tool_probe.text_hover, None);
    }

    #[test]
    fn a_changed_draft_replaces_the_hover_preview() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "WTE 2026".into();
        state.tools.text.warp = Some(arc50());
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        let first = Arc::clone(tools.stamp_place.hover_preview().unwrap().1);
        state.tools.text.content = "Bob".into();
        tools.hover(&mut state, &xf, &p);
        let (spec, second) = tools.stamp_place.hover_preview().unwrap();
        assert_eq!(spec.content, "Bob");
        assert!(!Arc::ptr_eq(second, &first));
        let second = Arc::clone(second);
        // The angle is not part of the picture.
        state.tools.stamp.rotation_deg = 30.0;
        tools.hover(&mut state, &xf, &p);
        assert!(Arc::ptr_eq(tools.stamp_place.hover_preview().unwrap().1, &second));
        // Neither is the size on the model.
        state.tools.text.em_frac *= 2.0;
        tools.hover(&mut state, &xf, &p);
        assert!(Arc::ptr_eq(tools.stamp_place.hover_preview().unwrap().1, &second));
    }

    #[test]
    fn add_stamp_reuses_only_a_full_preview() {
        use wte_core::doc_stamps::raster_counter;
        // (content, full picture expected): a short styled text has a full hover picture; a long one does not.
        for (content, full) in [("Hi", true), ("A much longer text stamp line", false)] {
            let mut state = state_with(SampleKind::Sphere);
            state.tool = ToolKind::Text;
            state.tools.text.content = content.into();
            state.tools.text.warp = Some(arc50());
            state.tools.text.em_frac = em_of_100_texels(&state);
            let xf = xf_2d(&mut state);
            let mut tools = ToolController::new();
            let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
            tools.hover(&mut state, &xf, &p);
            let (_, picture) = tools.stamp_place.hover_preview().expect("a hover picture");
            assert_eq!(picture.full, full, "{content}: {}x{}", picture.image.width, picture.image.height);
            let before = raster_counter::get();
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            let made = raster_counter::get() - before;
            assert_eq!(made, if full { 0 } else { 1 }, "{content}: pictures made by the click");
            let id = state.session.as_ref().unwrap().selected_stamp.expect("placed");
            let doc = state.doc().unwrap();
            let StampSource::Text(placed) = &doc.stamp(id).unwrap().source else { panic!() };
            assert_eq!(placed.size_px, TEXT_PLACEMENT_PX, "{content}: the resolution was kept");
            assert_eq!(doc.text_picture_is_full(id), Some(true), "{content}");
        }
    }

    #[test]
    fn a_draft_too_large_for_every_size_is_not_drawn_again_each_frame() {
        // Check-and-fix of T7: a draft whose picture fails at every size (here wider than the side limit even at
        // 32 px per em) is tried once per draft, not once per hover frame; a changed draft is tried again.
        use wte_core::doc_stamps::raster_counter;
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Text;
        state.tools.text.content = "W".repeat(700);
        state.tools.text.slant_deg = 10.0;
        let xf = xf_2d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert!(tools.stamp_place.hover_preview().is_none(), "no picture of a text too large to place");
        assert!(tools.stamp_place.preview.is_none(), "no outline either");
        let before = raster_counter::get();
        for _ in 0..3 {
            tools.begin_frame();
            tools.hover(&mut state, &xf, &p);
        }
        assert_eq!(raster_counter::get(), before, "the failing draft was drawn again");
        assert!(tools.stamp_place.preview.is_none());
        // A draft that fits is drawn and outlined at once.
        state.tools.text.content = "WTE".into();
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert!(tools.stamp_place.hover_preview().is_some());
        assert!(tools.stamp_place.preview.is_some());
    }
}
