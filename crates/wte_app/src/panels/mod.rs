// Owner: app-panels (A4); round 4: B4 brush panels. Menu bar, toolbar, side panel and status bar of ARCHITECTURE.md
// 8.7. Round 4 (scaffold S2): the lower area of the side panel has the tab strip Layers | Brush | Text (FEATURE_BRUSH
// 6.1, 6.2; FEATURE_TEXT 6.1; ROUND4_PLAN 2.2 items 1, 3, 10) and the panel records `AppState::side_rects`.
//
// Panels only read `AppState` / `Document`, change settings in `AppState` directly and express every command as an
// `AppAction`. They never call `rfd` and never read keys. The thumbnail caches follow the policy of 8.7: a layer
// thumbnail is keyed by `(LayerId, Layer::version())`, at most `THUMBS_PER_FRAME` are regenerated per frame and none
// while a stroke, a gizmo drag or a slider drag is running.

use std::collections::HashMap;
use std::time::Duration;

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, vec2};
use wte_core::{BlobId, Commit, Document, LayerId, PixelBuf, StampId, TextureSetId};

use crate::action::AppAction;
use crate::state::{AppState, PanelHit, SideRects, SideTab, ToolKind};

pub mod brush_tab;
pub mod color;
pub mod font_picker;
pub mod layers;
pub mod menu_bar;
pub mod status_bar;
pub mod text_block;
pub mod text_tab;
pub mod texture_sets;
pub mod tool_options;
pub mod toolbar;

/// Layer thumbnails regenerated per frame at most (8.7).
const THUMBS_PER_FRAME: usize = 2;
/// Side of a layer thumbnail in points (8.7: "40-px thumbnail").
const LAYER_THUMB_SIDE: f32 = 40.0;
/// Side of a texture-set thumbnail in points.
const SET_THUMB_SIDE: f32 = 36.0;
/// Side of the Stamp tool's source preview in points.
const SOURCE_THUMB_SIDE: f32 = 56.0;
/// While an interaction blocks thumbnail updates, the panel looks again after this delay (never a busy loop).
const INTERACTION_RECHECK: Duration = Duration::from_millis(200);

/// A cached egui texture of a thumbnail and the content version it shows (`None` = the source had no pixels).
struct Thumb {
    version: u64,
    tex: Option<egui::TextureHandle>,
}

/// An in-place layer rename (double-click on the name).
struct Rename {
    layer: LayerId,
    text: String,
    /// The field still has to take the keyboard focus.
    focus: bool,
}

/// UI-only state of the panels: layer thumbnails keyed by `(LayerId, Layer::version)`, set thumbnails,
/// rename-in-progress, drag state.
pub struct PanelState {
    /// `Session::scene_generation` the caches belong to (`None` = no document).
    generation: Option<u64>,
    layer_thumbs: HashMap<LayerId, Thumb>,
    /// Version = `Document::revision()` when the thumbnail was made.
    set_thumbs: HashMap<TextureSetId, Thumb>,
    /// Preview of the Stamp tool's source image.
    source_thumb: Option<(BlobId, Option<egui::TextureHandle>)>,
    /// Thumbnails that may still be regenerated in this frame.
    thumb_budget: usize,
    /// Whether a set thumbnail was already refreshed in this frame (one per frame).
    set_thumb_done: bool,
    rename: Option<Rename>,
    /// The hex field's text while it has the focus.
    hex: Option<String>,
    /// Edit buffer of the selected stamp's name.
    stamp_name: Option<(StampId, String)>,
    /// The `Commit::Final` twin of the last `Commit::Preview` action that has not been finalized yet.
    owed_final: Option<AppAction>,
    /// A `Commit::Preview` action was queued in this frame.
    preview_sent: bool,
    /// UI state of the Text block, the Text tab and the font picker (round 4; the type belongs to `text_block.rs`).
    pub(crate) text: text_block::TextPanelState,
    /// UI state of the Brush tab (round 4, B4).
    pub(crate) brush: brush_tab::BrushTabState,
    /// Height of the open Color section without its saturation/value square, measured in the last frame (points).
    color_fixed_height: f32,
    /// Height of the Color section's content in the last frame (points).
    color_content_height: f32,
    /// Height of the Layers footer in the last frame (points).
    layers_footer_height: f32,
    /// Outer height the Layers area needed in the last frame to show all its rows and the footer (points); `None`
    /// until measured.
    layers_natural_height: Option<f32>,
    /// Bottom of the Layers section's content (header, or body when open) in the last frame (screen y).
    layers_content_bottom: Option<f32>,
    /// Height of the layer rows scrolled out of the list's view in the last frame (points).
    layers_list_overflow: f32,
    /// The Layers area's rect in the last frame (for the resize-edge press).
    layers_area_rect: Option<Rect>,
    /// The user dragged the Layers area's edge: from then on the area keeps the size they chose (kept across
    /// documents); before that it hugs its content.
    layers_user_sized: bool,
}

impl Default for PanelState {
    fn default() -> PanelState {
        PanelState::new()
    }
}

impl PanelState {
    /// Empty caches.
    pub fn new() -> PanelState {
        PanelState {
            generation: None,
            layer_thumbs: HashMap::new(),
            set_thumbs: HashMap::new(),
            source_thumb: None,
            thumb_budget: THUMBS_PER_FRAME,
            set_thumb_done: false,
            rename: None,
            hex: None,
            stamp_name: None,
            owed_final: None,
            preview_sent: false,
            text: text_block::TextPanelState::default(),
            brush: brush_tab::BrushTabState::default(),
            color_fixed_height: COLOR_FIXED_HEIGHT_GUESS,
            color_content_height: COLOR_FIXED_HEIGHT_GUESS + COLOR_SQUARE_MAX,
            layers_footer_height: LAYERS_FOOTER_HEIGHT_GUESS,
            layers_natural_height: None,
            layers_content_bottom: None,
            layers_list_overflow: 0.0,
            layers_area_rect: None,
            layers_user_sized: false,
        }
    }

    /// Per-frame bookkeeping: drops every cache when the document object changed, prunes thumbnails of layers and
    /// sets that no longer exist, resets the regeneration budget.
    fn begin_frame(&mut self, state: &AppState) {
        let generation = state.session.as_ref().map(|s| s.scene_generation);
        if generation != self.generation {
            *self = PanelState { generation, layers_user_sized: self.layers_user_sized, ..PanelState::new() };
        }
        self.thumb_budget = THUMBS_PER_FRAME;
        self.set_thumb_done = false;
        self.preview_sent = false;
        if let Some(doc) = state.doc() {
            self.layer_thumbs.retain(|id, _| doc.layer(*id).is_some());
            self.set_thumbs.retain(|id, _| doc.set(*id).is_some());
            self.drop_stale_owed_final(doc);
        }
    }

    /// Forgets the owed `Commit::Final` when the layer or stamp it acts on no longer exists (deleted, or removed by an
    /// undo): sending it would only be rejected with "Unknown layer" / "Unknown stamp".
    fn drop_stale_owed_final(&mut self, doc: &Document) {
        if self.owed_final.as_ref().is_some_and(|a| !action_target_exists(doc, a)) {
            self.owed_final = None;
        }
    }

    /// Sends the `Commit::Final` still owed for a preview edit once nothing is being dragged or typed any more
    /// (colour popups report no "drag stopped"). Nothing is sent when the pending edit has meanwhile ended
    /// (cancelled with Esc, or flushed by another command).
    fn end_frame(&mut self, ctx: &egui::Context, state: &mut AppState) {
        if let Some(doc) = state.doc() {
            self.drop_stale_owed_final(doc);
        }
        let Some(action) = self.owed_final.take() else { return };
        let still_editing = self.preview_sent || ctx.input(|i| i.pointer.any_down()) || ctx.text_edit_focused();
        if still_editing {
            self.owed_final = Some(action);
        } else if state.doc().is_some_and(Document::has_pending_edit) {
            state.push(action);
        }
    }

    /// The thumbnail of a layer, regenerated only when its version changed (8.7 thumbnail policy).
    fn layer_thumb(&mut self, ctx: &egui::Context, doc: &Document, id: LayerId) -> Option<egui::TextureId> {
        let version = doc.layer_version(id)?;
        let stale = self.layer_thumbs.get(&id).is_none_or(|t| t.version != version);
        if stale {
            if interacting(doc) {
                ctx.request_repaint_after(INTERACTION_RECHECK);
            } else if self.thumb_budget > 0 {
                self.thumb_budget -= 1;
                let side = thumb_pixels(ctx, LAYER_THUMB_SIDE);
                let image = doc.layer_thumbnail(id, side);
                let name = format!("layer-thumb-{}", id.0);
                let old = self.layer_thumbs.remove(&id).and_then(|t| t.tex);
                self.layer_thumbs.insert(id, Thumb { version, tex: upload(ctx, &name, old, image.as_ref()) });
            } else {
                ctx.request_repaint();
            }
        }
        self.layer_thumbs.get(&id).and_then(|t| t.tex.as_ref()).map(egui::TextureHandle::id)
    }

    /// The thumbnail of a texture set, refreshed (one set per frame) when `doc.revision()` changed and no
    /// interaction is running.
    fn set_thumb(&mut self, ctx: &egui::Context, doc: &Document, set: TextureSetId) -> Option<egui::TextureId> {
        let revision = doc.revision();
        let stale = self.set_thumbs.get(&set).is_none_or(|t| t.version != revision);
        if stale {
            if interacting(doc) || doc.has_pending_composite() {
                ctx.request_repaint_after(INTERACTION_RECHECK);
            } else if !self.set_thumb_done {
                self.set_thumb_done = true;
                let side = thumb_pixels(ctx, SET_THUMB_SIDE);
                let image = doc.set_thumbnail(set, side);
                let name = format!("set-thumb-{}", set.0);
                let old = self.set_thumbs.remove(&set).and_then(|t| t.tex);
                self.set_thumbs.insert(set, Thumb { version: revision, tex: upload(ctx, &name, old, image.as_ref()) });
            } else {
                ctx.request_repaint();
            }
        }
        self.set_thumbs.get(&set).and_then(|t| t.tex.as_ref()).map(egui::TextureHandle::id)
    }

    /// The preview of a stamp source image (made once per blob). `blob_thumbnail` only reads the blob through the
    /// document's decode cache.
    /// Returns the texture and its size in pixels.
    fn source_thumb(
        &mut self,
        ctx: &egui::Context,
        state: &mut AppState,
        blob: BlobId,
    ) -> Option<(egui::TextureId, [usize; 2])> {
        if self.source_thumb.as_ref().is_none_or(|(b, _)| *b != blob) {
            let side = thumb_pixels(ctx, SOURCE_THUMB_SIDE);
            let image = state.doc_mut().and_then(|d| d.blob_thumbnail(blob, side).ok());
            let tex = upload(ctx, &format!("stamp-source-{}", blob.0), None, image.as_ref());
            self.source_thumb = Some((blob, tex));
        }
        self.source_thumb.as_ref().and_then(|(_, t)| t.as_ref()).map(|t| (t.id(), t.size()))
    }
}

/// Whether the layer or stamp a continuous control's action acts on still exists in `doc` (true for other actions).
pub(crate) fn action_target_exists(doc: &Document, action: &AppAction) -> bool {
    match action {
        AppAction::SetLayerOpacity(layer, ..) => doc.layer(*layer).is_some(),
        AppAction::UpdateStampSource(id, ..) | AppAction::UpdateStampPlacement(id, ..) | AppAction::UpdateStampTint(id, ..) => {
            doc.stamp(*id).is_some()
        }
        _ => true,
    }
}

/// True while a stroke, a gizmo drag or a slider drag is running: thumbnails keep their old image (8.7).
fn interacting(doc: &Document) -> bool {
    doc.stroke_active() || doc.has_pending_edit()
}

/// Thumbnail side in physical pixels for `points` (crisp on high-DPI screens, bounded).
fn thumb_pixels(ctx: &egui::Context, points: f32) -> u32 {
    (points * ctx.pixels_per_point()).round().clamp(16.0, 160.0) as u32
}

/// Uploads a straight-alpha thumbnail (`ColorImage::from_rgba_unmultiplied`, pitfall 3), reusing `old` when given.
fn upload(
    ctx: &egui::Context,
    name: &str,
    old: Option<egui::TextureHandle>,
    image: Option<&PixelBuf>,
) -> Option<egui::TextureHandle> {
    let image = image.filter(|p| p.width > 0 && p.height > 0)?;
    let color = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.data);
    match old {
        Some(mut tex) => {
            tex.set(color, egui::TextureOptions::LINEAR);
            Some(tex)
        }
        None => Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR)),
    }
}

/// The menu bar (File, Edit, View, Layer, Help).
pub fn show_menu_bar(ui: &mut egui::Ui, state: &mut AppState) {
    menu_bar::show(ui, state);
}

/// The tool buttons and the primary / secondary colour chip.
pub fn show_toolbar(ui: &mut egui::Ui, state: &mut AppState) {
    toolbar::show(ui, state);
}

/// The right side panel, three areas from top to bottom (decision 2 of round 3):
/// - Tool options and Texture sets in one scroll area with a visible scroll bar; it gets the room the other two leave;
/// - the Color section, pinned below them: as tall as its content, so at 1280x800 the whole picker is on screen
///   whatever the tool options show; when the room is short its square shrinks (down to `COLOR_SQUARE_MIN`), then
///   the section scrolls, leaving the tool options about 96 pt (`SETTINGS_MIN_HEIGHT`);
/// - the Layers section at the bottom in its own resizable area: the layer list scrolls inside it, the footer buttons
///   stay below the list (`layers::show`).
///
/// The areas are records of `state.panel_rects` (`PanelHit::SettingsArea`, `ColorSection`, `ColorContent`,
/// `LayersList`, `LayersFooter`, `LayersDelete`) for the self-test.
///
/// Round 4: the lower area starts with the tab strip Layers | Brush | Text in place of the "Layers" header and shows
/// the tab `shown_tab` gives (the one tab rule of ROUND4_PLAN 2.2 item 10). With the Layers tab the area hugs its rows
/// as in round 3; with the Brush or the Text tab it has the computed height of FEATURE_BRUSH 6.1 (`tab_area_height`);
/// a size the user dragged is kept for every tab. Every frame records `state.side_rects`.
pub fn show_side_panel(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    ps.begin_frame(state);
    state.panel_rects.clear();
    let mut rects = SideRects { panel: ui.max_rect(), ..SideRects::default() };
    let shown = shown_tab(state);
    if shown != SideTab::Text {
        text_tab::clear_gap_status(state); // T6 (round 4): the status line of a selected gap goes with the Text tab
    }
    let height = ui.available_height().max(1.0);
    // A press on the Layers area's resize edge: the user takes over its size.
    if !ps.layers_user_sized
        && let Some(area) = ps.layers_area_rect
    {
        let grab = ui.style().interaction.resize_grab_radius_side + 3.0;
        ps.layers_user_sized = ui.input(|i| {
            i.pointer.primary_pressed()
                && i.pointer.press_origin().is_some_and(|p| (p.y - area.top()).abs() <= grab && area.x_range().contains(p.x))
        });
    }
    let min = LAYERS_MIN_HEIGHT.min(height * 0.5);
    let panel = egui::Panel::bottom("side_layers_area")
        .resizable(true)
        .frame(egui::Frame::new().inner_margin(egui::Margin { top: 4, ..egui::Margin::ZERO }));
    let panel = if ps.layers_user_sized {
        panel.default_size(layers_area_height(height)).size_range(min..=layers_area_max(height))
    } else if shown == SideTab::Layers {
        // Until the user sizes it, the area hugs its rows and footer, at most its default (40 %, R3-10 review): the
        // room a short list does not need goes to the tool options and the Color section.
        panel.exact_size(layers_area_fit(height, ps.layers_natural_height))
    } else {
        // The Brush and the Text tab: the computed height of FEATURE_BRUSH 6.1 (the five brush slider rows stay
        // visible above the Color section, the tab gets the rest).
        panel.exact_size(tab_area_height(height, ps.color_content_height + COLOR_AREA_MARGIN))
    };
    ps.layers_list_overflow = 0.0;
    ps.layers_content_bottom = None;
    let layers_area = panel.show(ui, |ui| {
        // A panel's size is the size of its content: the area keeps its full height even when the list (which
        // shrinks to its rows) and the footer need less, or it would shrink a little every frame.
        ui.set_min_height(ui.available_height());
        with_visible_scroll_bars(ui);
        rects.tab_buttons = tab_strip(ui, state, shown);
        match shown {
            SideTab::Layers => layers::show(ui, state, ps),
            SideTab::Brush => {
                rects.brush_tab = Some(tab_content(ui, "side_brush_tab_scroll", |ui| brush_tab::show(ui, state, ps)));
                rects.brush_tab_content = ps.brush.content;
            }
            SideTab::Text => rects.text_tab = Some(tab_content(ui, "side_text_tab_scroll", |ui| text_tab::show(ui, state, ps))),
        }
    });
    let area = layers_area.response.rect;
    rects.bottom_area = area;
    state.panel_rects.insert(PanelHit::LayersArea, area);
    ps.layers_area_rect = Some(area);
    if let Some(bottom) = ps.layers_content_bottom {
        // 2 pt of slack: an area exactly as tall as its rows could flip the list's scroll bar on a rounding difference.
        let natural = (bottom - area.top() + ps.layers_list_overflow + 2.0).ceil();
        if ps.layers_natural_height.is_none_or(|n| (n - natural).abs() > 0.5) {
            ui.ctx().request_repaint();
        }
        ps.layers_natural_height = Some(natural);
    }

    // The Color section's room: what the Layers area left, minus the minimum of the tool options. The area is as tall as
    // its content was in the last frame (a scroll area cannot grow a panel by itself), at most the room.
    let room = (ui.available_height() - SETTINGS_MIN_HEIGHT).max(COLOR_MIN_HEIGHT);
    let square = color_square_side(ui.available_width(), room - COLOR_AREA_MARGIN, ps.color_fixed_height);
    let color = egui::Panel::bottom("side_color_area")
        .resizable(false)
        .show_separator_line(true)
        .exact_size((ps.color_content_height + COLOR_AREA_MARGIN).min(room))
        .frame(egui::Frame::new().inner_margin(egui::Margin { top: 4, bottom: 2, ..egui::Margin::ZERO }))
        .show(ui, |ui| {
            with_visible_scroll_bars(ui);
            let out = egui::ScrollArea::vertical()
                .id_salt("side_color_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| color::show(ui, state, ps, square));
            let content = Rect::from_min_size(out.inner_rect.min - out.state.clamped_offset(), out.content_size);
            if (out.content_size.y - ps.color_content_height).abs() > 0.5 {
                ui.ctx().request_repaint();
            }
            ps.color_content_height = out.content_size.y;
            if let Some(square) = out.inner {
                // Open: remember the height of everything but the square for the next frame's fit.
                ps.color_fixed_height = (out.content_size.y - square).max(0.0);
            }
            content
        });
    state.panel_rects.insert(PanelHit::ColorSection, color.response.rect);
    state.panel_rects.insert(PanelHit::ColorContent, color.inner);
    rects.color = color.response.rect;

    with_visible_scroll_bars(ui);
    let settings = egui::ScrollArea::vertical().id_salt("side_settings_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let options = tool_options::show(ui, state, ps);
        texture_sets::show(ui, state, ps);
        options
    });
    state.panel_rects.insert(PanelHit::SettingsArea, settings.inner_rect);
    rects.tool_area = settings.inner_rect;
    rects.tool_options = settings.inner;
    state.side_rects = rects;
    ps.end_frame(ui.ctx(), state);
}

/// Height of the tab strip of the lower area (FEATURE_BRUSH 6.2).
const TAB_STRIP_HEIGHT: f32 = 24.0;
/// Height of the Brush options in the tool area (header and five slider rows, FEATURE_BRUSH 6.1).
const BRUSH_OPTIONS_HEIGHT: f32 = 144.0;
/// Limits of the computed height of the lower area while the Brush or the Text tab is shown (FEATURE_BRUSH 6.1).
const TAB_AREA_MIN: f32 = 160.0;
const TAB_AREA_MAX: f32 = 420.0;

/// The Brush tab can be shown: the tool is the Brush or the Eraser (ROUND4_PLAN 2.2 item 10).
pub(crate) fn brush_tab_enabled(state: &AppState) -> bool {
    matches!(state.tool, ToolKind::Brush | ToolKind::Eraser)
}

/// The Text tab can be shown: the Text block has a target (the tool is Text, or the tool is Transform and the selected
/// stamp is a text; ROUND4_PLAN 2.2 item 10, FEATURE_TEXT 6.1).
pub(crate) fn text_tab_enabled(state: &AppState) -> bool {
    text_block::target(state).is_some()
}

/// The tab the lower area shows: `state.side_tab` when its condition is met, else Layers (the one tab rule). A tool
/// change never writes `side_tab`, so the chosen tab comes back by itself when its tool does.
pub(crate) fn shown_tab(state: &AppState) -> SideTab {
    match state.side_tab {
        SideTab::Brush if brush_tab_enabled(state) => SideTab::Brush,
        SideTab::Text if text_tab_enabled(state) => SideTab::Text,
        _ => SideTab::Layers,
    }
}

/// Height of the lower area while the Brush or the Text tab is shown, for a side panel of `height` points and a Color
/// section of `color` points (FEATURE_BRUSH 6.1): `height - color - 144 - 4`, limited to `160..=420` and to what the
/// tool options and the Color area must keep.
fn tab_area_height(height: f32, color: f32) -> f32 {
    let wanted = if color.is_finite() { height - color - BRUSH_OPTIONS_HEIGHT - 4.0 } else { layers_area_height(height) };
    wanted.clamp(TAB_AREA_MIN.min(height * 0.5), TAB_AREA_MAX).min(layers_area_max(height))
}

/// The tab strip: one row of three selectable buttons in place of the "Layers" header. A tab that cannot be used is
/// greyed out with a hover text that says why. A click writes `state.side_tab` directly (a setting, not an action).
/// Returns the buttons' rects (indexed by `SideTab::index`).
fn tab_strip(ui: &mut egui::Ui, state: &mut AppState, shown: SideTab) -> [Rect; 3] {
    let mut rects = [Rect::NOTHING; 3];
    ui.horizontal(|ui| {
        ui.set_min_height(TAB_STRIP_HEIGHT);
        for tab in SideTab::ALL {
            let (enabled, why) = match tab {
                SideTab::Layers => (true, ""),
                SideTab::Brush => (brush_tab_enabled(state), "Select the Brush or the Eraser"),
                SideTab::Text => (text_tab_enabled(state), "Choose the Text tool, or select a text with the Transform tool"),
            };
            let button = egui::Button::selectable(shown == tab, tab.label()).min_size(vec2(0.0, TAB_STRIP_HEIGHT - 4.0));
            let resp = ui.add_enabled(enabled, button).on_disabled_hover_text(why);
            if resp.clicked() {
                state.side_tab = tab;
            }
            rects[tab.index()] = resp.rect;
        }
    });
    rects
}

/// The content of the Brush or the Text tab in its own scroll area (the tab is taller than the area in a small
/// window); returns the visible rect of the content.
fn tab_content(ui: &mut egui::Ui, salt: &'static str, add: impl FnOnce(&mut egui::Ui)) -> Rect {
    let out = egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(ui, add);
    out.inner_rect
}

/// Smallest height of the Layers area: header, opacity row, two whole rows and the two-line footer (at 640x480 the
/// Layers area gets this; 160 pt left 1.6 rows once the footer moved out of the list's scroll area).
const LAYERS_MIN_HEIGHT: f32 = 200.0;
/// Height kept for the Tool options / Texture sets area while the Color section fits itself in (about 96 pt of it stay
/// visible once the spacing and separator lines of the areas are taken off).
const SETTINGS_MIN_HEIGHT: f32 = 112.0;
/// Smallest height of the Color section's area (it scrolls below that).
const COLOR_MIN_HEIGHT: f32 = 60.0;
/// Top and bottom margin of the Color area's frame (6 pt) plus 2 pt of slack: an area exactly as tall as its content
/// could show the scroll bar on a rounding difference, which narrows the swatches, which shortens the content, which
/// hides the bar again — a layout that flips every frame.
const COLOR_AREA_MARGIN: f32 = 8.0;
/// Largest and smallest side of the colour picker's saturation/value square (= the width of its bars).
const COLOR_SQUARE_MAX: f32 = 140.0;
const COLOR_SQUARE_MIN: f32 = 72.0;
/// First-frame guesses of measured heights (corrected after one frame).
const COLOR_FIXED_HEIGHT_GUESS: f32 = 180.0;
const LAYERS_FOOTER_HEIGHT_GUESS: f32 = 52.0;

/// Default height of the Layers area for a side panel of `height` points: about 40 %, at least room for three rows.
fn layers_area_height(height: f32) -> f32 {
    (height * 0.4).clamp(LAYERS_MIN_HEIGHT.min(height * 0.5), 420.0)
}

/// Height of the Layers area before the user sizes it: what its rows and footer need (`natural`, measured in the last
/// frame), at least `LAYERS_MIN_HEIGHT` and at most the default `layers_area_height`; the default until measured.
fn layers_area_fit(height: f32, natural: Option<f32>) -> f32 {
    let max = layers_area_height(height);
    natural.map_or(max, |n| n.clamp(LAYERS_MIN_HEIGHT.min(height * 0.5).min(max), max))
}

/// Largest height of the Layers area: 85 % of the panel, and never so much that the tool options and the Color area
/// lose their minimum (a Layers area sized in a large window keeps its size when the window shrinks, clamped to this).
fn layers_area_max(height: f32) -> f32 {
    (height * 0.85).min(height - SETTINGS_MIN_HEIGHT - COLOR_MIN_HEIGHT).max(LAYERS_MIN_HEIGHT)
}

/// Side of the colour picker's square: as large as the panel width allows (at most `COLOR_SQUARE_MAX`) and as the
/// section's `room` allows given the rest of the section (`fixed`), never below `COLOR_SQUARE_MIN` (then it scrolls).
fn color_square_side(width: f32, room: f32, fixed: f32) -> f32 {
    let by_width = (width - 24.0).clamp(COLOR_SQUARE_MIN, COLOR_SQUARE_MAX);
    (room - fixed).floor().clamp(COLOR_SQUARE_MIN, by_width)
}

/// Thin scroll bars that stay visible while the content overflows (egui's default floating bars are invisible until
/// hovered, so a cut-off section gave no cue that it scrolls).
fn with_visible_scroll_bars(ui: &mut egui::Ui) {
    ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
}

/// The status bar: hint, hover read-out, texture size, zoom, undo label, job progress.
pub fn show_status_bar(ui: &mut egui::Ui, state: &mut AppState) {
    status_bar::show(ui, state);
}

// ---- shared helpers of the panel files ----

/// Queues the action of a continuous control (slider, drag value, text field, colour popup):
/// `Commit::Preview` while it is being dragged or typed in, `Commit::Final` when the drag stops or the field loses
/// the focus (8.7); a one-shot change (a click, a key on a focused slider is still a preview) is `Final` at once.
pub(crate) fn continuous_edit(
    ps: &mut PanelState,
    out: &mut Vec<AppAction>,
    ui: &egui::Ui,
    resp: &egui::Response,
    make: impl Fn(Commit) -> AppAction,
) {
    let ending = resp.drag_stopped() || resp.lost_focus();
    if ending && resp.changed() {
        out.push(make(Commit::Final));
        ps.owed_final = None;
    } else if ending && let Some(owed) = ps.owed_final.take() {
        // The edit ended without a change in this frame: finish it with the owed action itself, i.e. for the target
        // and value of its last preview. `make` would aim at whatever this control shows now, which can be another
        // layer or stamp than the one being edited (the selection or the active layer changed meanwhile).
        out.push(owed);
    } else if resp.changed() {
        // A focused text field counts as live too: typing into the R/G/B fields of a colour popup changes the popup's
        // button, which itself has no focus; each keystroke would otherwise be its own Final (one undo entry and, for
        // text, one re-rasterization per key). `end_frame` sends the owed Final once nothing is focused any more.
        let live = resp.dragged()
            || resp.has_focus()
            || resp.is_pointer_button_down_on()
            || ui.input(|i| i.pointer.any_down())
            || ui.ctx().text_edit_focused();
        if live {
            out.push(make(Commit::Preview));
            ps.owed_final = Some(make(Commit::Final));
            ps.preview_sent = true;
        } else {
            out.push(make(Commit::Final));
            ps.owed_final = None;
        }
    }
}

/// Straight sRGB RGBA -> egui colour.
pub(crate) fn to_color32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// egui colour -> straight sRGB RGBA.
pub(crate) fn from_color32(c: Color32) -> [u8; 4] {
    c.to_srgba_unmultiplied()
}

/// A slider that edits a `0..=1`-style fraction as a percentage.
///
/// It never rewrites the value it is given unless the user edits it (`SliderClamping::Edits`): with egui's default
/// (`Always`) the slider stores the *existing* value rounded to its decimals in every frame and reports a change, so an
/// idle slider bound to a document value (layer opacity 21.33 % -> 21 %) queued a `Commit::Final` edit by itself
/// (finding F2 of round 3: a stray "Layer Opacity" undo entry after Duplicate, and "Unknown layer" when the frame's
/// undo had removed the layer the slider still showed).
pub(crate) fn percent_slider(value: &mut f32, min_pct: f32, max_pct: f32) -> egui::Slider<'_> {
    egui::Slider::from_get_set(f64::from(min_pct)..=f64::from(max_pct), move |v| {
        if let Some(v) = v {
            *value = v as f32 / 100.0;
        }
        f64::from(*value) * 100.0
    })
    .suffix(" %")
    .max_decimals(1)
    .clamping(egui::SliderClamping::Edits)
}

/// A grey/white checkerboard behind (partly) transparent thumbnails and swatches.
pub(crate) fn paint_checker(painter: &egui::Painter, rect: Rect) {
    let cell = (rect.width().min(rect.height()) / 4.0).max(3.0);
    painter.rect_filled(rect, 0.0, Color32::from_gray(200));
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                continue;
            }
            let min = rect.min + vec2(x as f32 * cell, y as f32 * cell);
            let r = Rect::from_min_size(min, vec2(cell, cell)).intersect(rect);
            painter.rect_filled(r, 0.0, Color32::from_gray(150));
        }
    }
}

/// Paints a thumbnail texture letter-boxed into `rect` over a checkerboard (or only the checkerboard).
pub(crate) fn paint_thumb(painter: &egui::Painter, rect: Rect, tex: Option<egui::TextureId>, aspect: f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let size = if aspect >= 1.0 { vec2(rect.width(), rect.width() / aspect) } else { vec2(rect.height() * aspect, rect.height()) };
    let inner = Rect::from_center_size(rect.center(), size);
    paint_checker(painter, inner);
    if let Some(tex) = tex {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(tex, inner, uv, Color32::WHITE);
    }
    painter.rect_stroke(inner, 0.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Outside);
}

/// `#RRGGBB` of an opaque colour.
pub(crate) fn hex_of(c: [u8; 4]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (case-insensitive, surrounding spaces ignored) as an opaque colour.
pub(crate) fn parse_hex(text: &str) -> Option<[u8; 4]> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&t[i..i + 1], 16).ok();
    match t.len() {
        6 => {
            let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?, 255])
        }
        3 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{OrbitCamera, Prefs, Session, UvViewState};
    use wte_core::SampleKind;

    pub(crate) fn sample_state(kind: SampleKind) -> AppState {
        let mut state = AppState::new(Prefs::default(), true);
        state.session = Some(Session {
            doc: Document::sample(kind),
            path: None,
            force_save_as: false,
            active_set: TextureSetId(0),
            selected_stamp: None,
            camera: OrbitCamera::default(),
            preview_camera: OrbitCamera::default(),
            uv_view: UvViewState::default(),
            scene_generation: 7,
            text_gap: None,
            selected_layers: Vec::new(),
        });
        state
    }

    /// Runs one headless frame of every panel in the layout of `App::ui`.
    pub(crate) fn run_panels(ctx: &egui::Context, state: &mut AppState, ps: &mut PanelState, input: egui::RawInput) {
        crate::theme::run_test_pass(ctx, input, |ui| {
            egui::Panel::top("menu").show(ui, |ui| show_menu_bar(ui, state));
            egui::Panel::bottom("status").show(ui, |ui| show_status_bar(ui, state));
            egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| show_toolbar(ui, state));
            egui::Panel::right("side").default_size(300.0).show(ui, |ui| show_side_panel(ui, state, ps));
        });
    }

    fn screen(w: f32, h: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))),
            ..Default::default()
        }
    }

    #[test]
    fn hex_round_trip_and_shorthand() {
        assert_eq!(hex_of([255, 0, 128, 255]), "#FF0080");
        assert_eq!(parse_hex("#FF0080"), Some([255, 0, 128, 255]));
        assert_eq!(parse_hex(" ff0080 "), Some([255, 0, 128, 255]));
        assert_eq!(parse_hex("#f08"), Some([255, 0, 136, 255]));
        assert_eq!(parse_hex("#FF008"), None);
        assert_eq!(parse_hex("#GG0000"), None);
        assert_eq!(parse_hex(""), None);
        for c in [[0, 0, 0, 255], [1, 2, 3, 255], [254, 128, 7, 255]] {
            assert_eq!(parse_hex(&hex_of(c)), Some(c));
        }
    }

    #[test]
    fn colour_conversion_is_exact_for_opaque_colours() {
        for c in [[0, 0, 0, 255], [12, 200, 77, 255], [255, 255, 255, 255]] {
            assert_eq!(from_color32(to_color32(c)), c);
        }
    }

    #[test]
    fn idle_panels_queue_no_actions_and_respect_the_thumbnail_budget() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        assert!(state.actions.is_empty(), "{:?}", state.actions);
        // Two layers (Base, Paint 1): both thumbnails within the per-frame budget, the set thumbnail too.
        assert!(ps.layer_thumbs.len() <= THUMBS_PER_FRAME);
        assert_eq!(ps.set_thumbs.len(), 1);
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        }
        assert_eq!(ps.layer_thumbs.len(), 2);
        assert!(state.actions.is_empty(), "{:?}", state.actions);
    }

    #[test]
    fn thumbnails_follow_layer_versions_not_revisions() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        }
        let versions = |ps: &PanelState| {
            let mut v: Vec<(u64, u64)> = ps.layer_thumbs.iter().map(|(id, t)| (id.0, t.version)).collect();
            v.sort_unstable();
            v
        };
        let before = versions(&ps);
        // A structure change (visibility) bumps the revision but no layer version: no layer thumbnail is redone.
        let doc = state.doc_mut().unwrap();
        let paint = doc.sets[0].active.unwrap();
        doc.set_layer_visible(paint, false).unwrap();
        run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        assert_eq!(versions(&ps), before);
        // A new layer gets its own thumbnail.
        let doc = state.doc_mut().unwrap();
        let added = doc.add_raster_layer(TextureSetId(0), None).unwrap();
        run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        assert!(ps.layer_thumbs.contains_key(&added));
        // A different document object drops every cache.
        state.session.as_mut().unwrap().scene_generation = 8;
        ps.begin_frame(&state);
        assert!(ps.layer_thumbs.is_empty() && ps.set_thumbs.is_empty());
    }

    #[test]
    fn no_thumbnail_is_made_during_a_pending_edit() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let doc = state.doc_mut().unwrap();
        let paint = doc.sets[0].active.unwrap();
        doc.set_layer_opacity(paint, 0.5, Commit::Preview).unwrap();
        assert!(doc.has_pending_edit());
        run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        assert!(ps.layer_thumbs.is_empty() && ps.set_thumbs.is_empty());
        state.doc_mut().unwrap().flush_pending_edit();
        run_panels(&ctx, &mut state, &mut ps, screen(1280.0, 800.0));
        assert_eq!(ps.layer_thumbs.len(), THUMBS_PER_FRAME);
    }

    #[test]
    fn owed_final_is_sent_once_the_pointer_is_up_and_only_for_a_pending_edit() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let paint = state.doc().unwrap().sets[0].active.unwrap();
        let final_action = AppAction::SetLayerOpacity(paint, 0.25, Commit::Final);

        // No pending edit any more (e.g. cancelled with Esc): nothing is sent.
        ps.owed_final = Some(final_action.clone());
        ps.end_frame(&ctx, &mut state);
        assert!(state.actions.is_empty() && ps.owed_final.is_none());

        // A pending preview edit: the final twin is queued.
        state.doc_mut().unwrap().set_layer_opacity(paint, 0.25, Commit::Preview).unwrap();
        ps.owed_final = Some(final_action.clone());
        ps.preview_sent = true; // the preview of this frame is still to be dispatched
        ps.end_frame(&ctx, &mut state);
        assert!(state.actions.is_empty() && ps.owed_final.is_some());
        ps.preview_sent = false;
        ps.end_frame(&ctx, &mut state);
        assert!(matches!(state.actions[..], [AppAction::SetLayerOpacity(id, v, Commit::Final)] if id == paint && v == 0.25));
        assert!(ps.owed_final.is_none());
    }

    #[test]
    fn owed_final_is_dropped_when_its_layer_or_stamp_is_gone() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let doc = state.doc_mut().unwrap();
        let paint = doc.sets[0].active.unwrap();
        let copy = doc.duplicate_layer(paint).unwrap();
        doc.set_layer_opacity(copy, 0.4, Commit::Preview).unwrap();
        ps.owed_final = Some(AppAction::SetLayerOpacity(copy, 0.4, Commit::Final));
        // An undo removes the copy (it also ends the pending preview edit first).
        doc.undo();
        doc.undo();
        assert!(doc.layer(copy).is_none());
        ps.begin_frame(&state);
        assert!(ps.owed_final.is_none(), "begin_frame forgets the Final of a removed layer");

        // The same in end_frame, with a pending edit of another layer still running.
        let doc = state.doc_mut().unwrap();
        doc.set_layer_opacity(paint, 0.3, Commit::Preview).unwrap();
        ps.owed_final = Some(AppAction::SetLayerOpacity(copy, 0.4, Commit::Final));
        ps.end_frame(&ctx, &mut state);
        assert!(state.actions.is_empty() && ps.owed_final.is_none(), "{:?}", state.actions);
        // A stamp target that no longer exists.
        ps.owed_final = Some(AppAction::UpdateStampTint(StampId(9999), [1, 2, 3, 255], Commit::Final));
        ps.end_frame(&ctx, &mut state);
        assert!(state.actions.is_empty() && ps.owed_final.is_none());
        // A live target is still sent.
        ps.owed_final = Some(AppAction::SetLayerOpacity(paint, 0.3, Commit::Final));
        ps.end_frame(&ctx, &mut state);
        assert!(matches!(state.actions[..], [AppAction::SetLayerOpacity(id, v, Commit::Final)] if id == paint && v == 0.3));
    }

    /// An edit that ends (drag stopped, focus lost) without a change in that frame is finished with the owed action
    /// itself, never retargeted at the layer or stamp the control shows now.
    #[test]
    fn an_ending_edit_finishes_the_owed_target_not_the_shown_one() {
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        let (a, b) = (LayerId(41), LayerId(42));
        ps.owed_final = Some(AppAction::SetLayerOpacity(a, 0.4, Commit::Final));
        let field = egui::Id::unique("ending_field");
        let mut out = Vec::new();
        for _ in 0..2 {
            crate::theme::run_test_pass(&ctx, screen(800.0, 600.0), |ui| {
                let mut text = String::from("x");
                ui.add(egui::TextEdit::singleline(&mut text).id(field)).request_focus();
            });
        }
        assert!(ctx.text_edit_focused());
        crate::theme::run_test_pass(&ctx, screen(800.0, 600.0), |ui| {
            // The focus moves away during this frame (as a click elsewhere does): the field reports `lost_focus`.
            ui.memory_mut(|m| m.surrender_focus(field));
            let mut text = String::from("x");
            let resp = ui.add(egui::TextEdit::singleline(&mut text).id(field));
            assert!(resp.lost_focus() && !resp.changed());
            continuous_edit(&mut ps, &mut out, ui, &resp, |commit| AppAction::SetLayerOpacity(b, 0.9, commit));
        });
        assert!(matches!(out[..], [AppAction::SetLayerOpacity(id, v, Commit::Final)] if id == a && v == 0.4), "{out:?}");
        assert!(ps.owed_final.is_none());
    }

    /// The opacity slider of an idle Layers panel never rewrites a value that its 0 decimals cannot show.
    #[test]
    fn an_idle_percent_slider_keeps_a_fractional_value() {
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        let mut out = Vec::new();
        let mut value = 0.213_319_3_f32;
        for _ in 0..3 {
            crate::theme::run_test_pass(&ctx, screen(800.0, 600.0), |ui| {
                let shown = value;
                let resp = ui.add(percent_slider(&mut value, 0.0, 100.0).max_decimals(0));
                continuous_edit(&mut ps, &mut out, ui, &resp, |commit| AppAction::SetLayerOpacity(LayerId(1), shown, commit));
            });
        }
        assert_eq!(value, 0.213_319_3);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn typing_in_a_popup_field_is_one_preview_edit_finished_when_the_focus_leaves() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let paint = state.doc().unwrap().sets[0].active.unwrap();
        let field = egui::Id::unique("popup_r_field");
        // The colour popup's R field has the keyboard focus; the popup's button (not focused, no pointer down) reports
        // a change for every keystroke.
        let keystroke = |ctx: &egui::Context, ps: &mut PanelState, out: &mut Vec<AppAction>, value: f32| {
            crate::theme::run_test_pass(ctx, screen(800.0, 600.0), |ui| {
                let mut text = String::from("2");
                let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                r.request_focus();
                let mut button = ui.interact(egui::Rect::from_min_size(Pos2::new(300.0, 10.0), vec2(30.0, 20.0)), egui::Id::unique("button"), egui::Sense::click());
                button.mark_changed();
                continuous_edit(ps, out, ui, &button, |commit| AppAction::SetLayerOpacity(paint, value, commit));
            });
        };
        let mut out = Vec::new();
        keystroke(&ctx, &mut ps, &mut out, 0.2); // takes the focus
        out.clear();
        ps.owed_final = None;
        for value in [0.2, 0.25, 0.3] {
            keystroke(&ctx, &mut ps, &mut out, value);
        }
        assert!(ctx.text_edit_focused());
        assert!(
            out.iter().all(|a| matches!(a, AppAction::SetLayerOpacity(_, _, Commit::Preview))),
            "keystrokes are previews: {out:?}"
        );
        assert!(matches!(ps.owed_final, Some(AppAction::SetLayerOpacity(_, v, Commit::Final)) if v == 0.3));
        // The previews were applied (a pending edit); once the focus is gone the one Final is queued.
        state.doc_mut().unwrap().set_layer_opacity(paint, 0.3, Commit::Preview).unwrap();
        ps.preview_sent = false;
        ps.end_frame(&ctx, &mut state);
        assert!(state.actions.is_empty(), "still typing");
        ctx.memory_mut(|m| m.surrender_focus(field));
        crate::theme::run_test_pass(&ctx, screen(800.0, 600.0), |_| {});
        assert!(!ctx.text_edit_focused());
        ps.end_frame(&ctx, &mut state);
        assert!(matches!(state.actions[..], [AppAction::SetLayerOpacity(_, v, Commit::Final)] if v == 0.3));
    }

    #[test]
    fn panels_render_without_a_document_and_for_every_tool() {
        let ctx = egui::Context::default();
        let mut empty = AppState::new(Prefs::default(), true);
        let mut ps = PanelState::new();
        run_panels(&ctx, &mut empty, &mut ps, screen(1280.0, 800.0));
        assert!(empty.actions.is_empty());
        // The tabs work without a document too (FEATURE_BRUSH 6.2).
        for (tool, tab) in [(ToolKind::Brush, SideTab::Brush), (ToolKind::Text, SideTab::Text)] {
            empty.tool = tool;
            empty.side_tab = tab;
            run_panels(&ctx, &mut empty, &mut ps, screen(1280.0, 800.0));
            // (round 5 R5-3: the font button asks for the samples once per catalog.)
            empty.actions.retain(|a| !matches!(a, AppAction::LoadFontSamples));
            assert!(empty.actions.is_empty(), "{tab:?}: {:?}", empty.actions);
        }

        let mut state = sample_state(SampleKind::TwoMaterial);
        for tool in [
            ToolKind::Navigate,
            ToolKind::Brush,
            ToolKind::Eraser,
            ToolKind::Eyedropper,
            ToolKind::Fill,
            ToolKind::Stamp,
            ToolKind::Text,
            ToolKind::Transform,
        ] {
            state.tool = tool;
            for tab in SideTab::ALL {
                state.side_tab = tab;
                run_panels(&ctx, &mut state, &mut ps, screen(900.0, 600.0));
                // (round 5 R5-3: the font button asks for the samples once per catalog.)
                state.actions.retain(|a| !matches!(a, AppAction::LoadFontSamples));
                assert!(state.actions.is_empty(), "{tool:?} {tab:?}: {:?}", state.actions);
            }
        }
    }

    /// The one tab rule (ROUND4_PLAN 2.2 item 10) through `side_rects`, and the computed height of the lower area
    /// (FEATURE_BRUSH 6.1) at 1280 x 800.
    #[test]
    fn the_tab_strip_follows_the_one_tab_rule() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let frames = |state: &mut AppState, ps: &mut PanelState| {
            for _ in 0..3 {
                run_panels(&ctx, state, ps, screen(1280.0, 800.0));
            }
        };
        state.tool = ToolKind::Brush;
        frames(&mut state, &mut ps);
        let layers_area = state.side_rects.bottom_area;
        assert!(state.side_rects.brush_tab.is_none() && state.side_rects.text_tab.is_none(), "Layers at start");
        assert!(state.panel_rects.contains_key(&PanelHit::LayersFooter));
        assert!(state.side_rects.tab_buttons.iter().all(|r| r.is_positive() && state.side_rects.bottom_area.contains_rect(*r)));

        state.side_tab = SideTab::Brush;
        frames(&mut state, &mut ps);
        let r = state.side_rects.clone();
        let tab = r.brush_tab.expect("the Brush tab is shown with the Brush");
        assert!(r.text_tab.is_none() && !state.panel_rects.contains_key(&PanelHit::LayersFooter), "no Layers rows");
        assert!(r.bottom_area.contains_rect(tab) && r.panel.contains_rect(r.color), "{r:?}");
        let expected = r.panel.height() - r.color.height() - BRUSH_OPTIONS_HEIGHT - 4.0;
        assert!((r.bottom_area.height() - expected.clamp(TAB_AREA_MIN, TAB_AREA_MAX)).abs() < 8.0, "computed height: {r:?}");
        assert!(r.bottom_area.height() > layers_area.height(), "two layers need less than a tab");
        assert!(r.tool_area.height() >= BRUSH_OPTIONS_HEIGHT - 8.0, "the brush sliders keep their room: {r:?}");

        // Another tool: Layers again, `side_tab` unchanged; the Brush comes back without a click.
        state.tool = ToolKind::Fill;
        frames(&mut state, &mut ps);
        assert!(state.side_rects.brush_tab.is_none() && state.side_tab == SideTab::Brush);
        state.tool = ToolKind::Eraser;
        frames(&mut state, &mut ps);
        assert!(state.side_rects.brush_tab.is_some());

        // The Text tab: Text tool yes, Transform only with a selected text, other tools no.
        state.side_tab = SideTab::Text;
        frames(&mut state, &mut ps);
        assert!(state.side_rects.text_tab.is_none(), "not with the Eraser");
        state.tool = ToolKind::Text;
        frames(&mut state, &mut ps);
        assert!(state.side_rects.text_tab.is_some() && state.side_rects.brush_tab.is_none());
        state.tool = ToolKind::Transform;
        frames(&mut state, &mut ps);
        assert!(state.side_rects.text_tab.is_none(), "Transform without a selected text");
        // (round 5 R5-3: the font button asks for the samples once per catalog.)
        state.actions.retain(|a| !matches!(a, AppAction::LoadFontSamples));
        assert!(state.actions.is_empty(), "{:?}", state.actions);
    }
}
