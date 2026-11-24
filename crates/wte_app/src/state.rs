//! Application state (ARCHITECTURE.md 8.2): everything the panels, views, tools and dialogs read and write.
//!
//! Shared contract file [S]: written completely by the wave-0 scaffold and not edited by later packages. Round 4: the
//! scaffold S2 added the types of FEATURE_BRUSH 4.2 and FEATURE_TEXT 4.2 (brush type and shape settings, the side tab,
//! the loaded brush shape, the closing-openings question of ROUND4_PLAN section 7 item 5, the text effect settings, the
//! Text tab page, the selected gap, the font samples and `side_rects`); later packages change it only additively.
//!
//! Derive policy (later packages rely on it): every plain-data type derives `Clone, Debug`; `PartialEq` wherever all
//! fields support it; `OrbitCamera` and `UvViewState` are also `Copy`; the view / tool / colour settings also derive
//! `Serialize, Deserialize` because they are written into `Document::ui_state` and `Prefs`. `Dialog`, its state
//! structs, `SetChoiceUi` and `Pending` derive `Clone, Debug` only. `Session` and `AppState` derive nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use serde::{Deserialize, Serialize};
use wte_core::{
    BlobId, CloseMode, DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, Document, ExportOptions, FillScope, FontCatalog,
    ImportedModel, LayerId, PixelBuf, PressureDynamics, SampleKind, StampId, SurfaceMode, TextAlign, TextPath,
    TextureSetId, TipAlign, TipField, UvStats, Warp, WrapOptions,
};

use crate::action::AppAction;

/// Number of colour swatches in `ColorState::swatches`.
pub const SWATCH_COUNT: usize = 24;

/// Layout of the central area.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[serde(rename = "view3d")]
    View3D,
    #[serde(rename = "view2d")]
    View2D,
    Split,
    Preview,
}

/// One rendered view (each has its own offscreen target).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ViewId {
    Main3D,
    Main2D,
    Preview,
    ExportPreview,
}

/// A control or a section of the side panel; `AppState::panel_rects` maps it to its screen rect (points) of the last
/// frame, for the self-test and the headless panel tests.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum PanelHit {
    /// The whole row (click selects, double-click renames, drag reorders).
    LayerRow(LayerId),
    LayerEye(LayerId),
    LayerThumb(LayerId),
    LayerName(LayerId),
    LayerLock(LayerId),
    LayerAlphaLock(LayerId),
    /// The visible part of the Layers list's scroll area (the rows scroll inside it).
    LayersList,
    /// The Layers footer (New / Duplicate / Merge Down / Rasterize / Delete), outside the list's scroll area.
    LayersFooter,
    /// The footer's Delete button.
    LayersDelete,
    /// The whole Layers area (resize edge at its top).
    LayersArea,
    /// The visible part of the Color section's area (header included).
    ColorSection,
    /// The whole Color section content as laid out (unclipped); inside `ColorSection` = nothing is scrolled away.
    ColorContent,
    /// The visible part of the scroll area of Tool options and Texture sets.
    SettingsArea,
    /// The content field of the selected text stamp (Text tool options or Transform options).
    StampTextField,
    /// Round 4 (B5): the choices of the closing-openings dialog (`Dialog::CloseOpenings`), by their index in the
    /// dialog, and its two buttons (written by `brush_flows::show_openings_dialog`, for the self-test).
    OpeningsChoice(u8),
    OpeningsOk,
    OpeningsCancel,
    /// Round 4 (gate): the font button of the Text block (row 2), which opens the font picker (written by
    /// `font_picker::button`, for the self-test).
    FontButton,
    /// The items of an open layer row context menu: "Export this layer..." and "Export selected layers... (N)"
    /// (for the self-test).
    LayerMenuExportThis,
    LayerMenuExportSelected,
    /// objects-tabs (WP4): a button of the upper tab strip (Tool | Color | Texture sets | Objects).
    UpperTabButton(UpperTab),
    /// objects-tabs (WP5): the row of an object in the Objects tab (only rows laid out in this frame are recorded).
    ObjectRow(u32),
    /// The eye of an object row.
    ObjectEye(u32),
    /// The visible part of the Objects list's scroll area.
    ObjectsList,
    /// The footer of the Objects tab (Hide / Isolate / Show all / Frame), below the list.
    ObjectsFooter,
    ObjectsHide,
    ObjectsIsolate,
    ObjectsShowAll,
    ObjectsFrame,
}

/// What the 3D view shows on the surface. The discriminant is the value handed to the shaders.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadingMode {
    Textured = 0,
    Clay = 1,
    UvChecker = 2,
}

/// Background of the 3D views. The discriminant is the value handed to the shaders.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    Dark = 0,
    Light = 1,
    Gradient = 2,
    Checker = 3,
}

/// The tools of the toolbar.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Navigate,
    Brush,
    Eraser,
    Eyedropper,
    Fill,
    Stamp,
    Text,
    Transform,
}

/// Axis-aligned camera presets.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CameraPreset {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

/// Display toggles of the editing views.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewFlags {
    pub shading: ShadingMode,
    pub lit: bool,
    pub wireframe: bool,
    pub seams: bool,
    /// UV wireframe in the 2D view.
    pub uv_wire: bool,
    pub alpha_checker: bool,
    pub backdrop: Backdrop,
}

impl Default for ViewFlags {
    /// Textured, lit, no wireframe, no seams, UV wireframe on, alpha checker on, gradient backdrop.
    fn default() -> ViewFlags {
        ViewFlags {
            shading: ShadingMode::Textured,
            lit: true,
            wireframe: false,
            seams: false,
            uv_wire: true,
            alpha_checker: true,
            backdrop: Backdrop::Gradient,
        }
    }
}

/// Options of the Preview mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewSettings {
    pub lit: bool,
    pub turntable: bool,
    /// Radians per second.
    pub turntable_speed: f32,
    pub backdrop: Backdrop,
    /// Nearest-neighbour texture filtering instead of linear + mips.
    pub nearest: bool,
}

impl Default for PreviewSettings {
    /// Lit, no turntable (0.6 rad/s when switched on), gradient backdrop, linear filtering.
    fn default() -> PreviewSettings {
        PreviewSettings { lit: true, turntable: false, turntable_speed: 0.6, backdrop: Backdrop::Gradient, nearest: false }
    }
}

/// Brush type (FEATURE_BRUSH 1.1): `Circular` paints what the brush painted before round 4 (the surface on the
/// cursor's side inside the ring); `Sphere` paints everything inside the 3D brush object (a ball by default, or the
/// shape of `BrushSettings::shape`), on any side, hidden or not.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushType {
    #[default]
    Circular,
    Sphere,
}

/// The 3D object of the Sphere type (FEATURE_BRUSH 1.2): the ball, a built-in Cube or Cylinder, or the shape loaded
/// from a model file (`AppState::brush_shape`).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeChoice {
    #[default]
    Ball,
    Cube,
    Cylinder,
    File,
}

/// Settings of the brush or of the eraser (FEATURE_BRUSH 1.4, 4.2). Round 4 removed `same_side_only`: `kind` replaces
/// it (Circular = the old "same side only" stroke). Plan section 7 item 1: there is no "stay on connected surface"
/// setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushSettings {
    /// Diameter in texels, `1..=2048`.
    pub size_px: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub flow: f32,
    pub spacing: f32,
    /// Circular (default) or Sphere.
    pub kind: BrushType,
    /// The 3D object of the Sphere type; Ball by default.
    pub shape: ShapeChoice,
    /// false = Solid (paint where the surface passes through the shape), true = Through (the outline pressed straight
    /// through the surface). Ignored by the ball; a flat outline is always Through.
    pub through: bool,
    /// The user's rotation of the shape in its own axes, degrees, `-180..=180` each (Turn about Y, Tilt about X, Roll
    /// about Z; FEATURE_BRUSH 4.3).
    pub turn_deg: f32,
    pub tilt_deg: f32,
    pub roll_deg: f32,
    /// Shift of the tip centre along the viewer-facing normal, in brush radii, `-1..=1`.
    pub offset: f32,
    /// "Follow surface" or "Fixed in space".
    pub align: TipAlign,
    /// Branch `pen-input` (Brush tab > Pen pressure): pen pressure drives the size (default on) ...
    pub pressure_size: bool,
    /// ... from this fraction of the size at the lightest touch, `0..=1` (default 0.10);
    pub pressure_size_min: f32,
    /// pen pressure drives the flow (default off) ...
    pub pressure_flow: bool,
    /// ... from this fraction of the flow, `0..=1` (default 0). A mouse always paints at pressure 1.0, where both
    /// factors are exactly 1 (ARCHITECTURE 5.19).
    pub pressure_flow_min: f32,
}

impl Default for BrushSettings {
    /// 32 texels, hardness 0.8, opacity 1, flow 1, spacing 0.12, Circular, Ball, Solid, no rotation, no offset, Follow
    /// surface; pen pressure drives the size from 10 % and not the flow (PEN_INPUT_PLAN D3).
    fn default() -> BrushSettings {
        BrushSettings {
            size_px: 32.0,
            hardness: 0.8,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.12,
            kind: BrushType::Circular,
            shape: ShapeChoice::Ball,
            through: false,
            turn_deg: 0.0,
            tilt_deg: 0.0,
            roll_deg: 0.0,
            offset: 0.0,
            align: TipAlign::Surface,
            pressure_size: true,
            pressure_size_min: 0.10,
            pressure_flow: false,
            pressure_flow_min: 0.0,
        }
    }
}

impl BrushSettings {
    /// Every value forced into the range of FEATURE_BRUSH 1.4; a value that is not finite becomes the default: size
    /// `1..=2048`, hardness `0..=1`, opacity and flow `0.01..=1`, spacing `0.02..=1`, angles wrapped into `-180..=180`,
    /// offset `-1..=1`, the two pressure minimums `0..=1`.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    pub fn sanitized(self) -> BrushSettings {
        let d = BrushSettings::default();
        let clamp = |v: f32, def: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { def };
        let angle = |v: f32| {
            if !v.is_finite() {
                0.0
            } else if (-180.0..=180.0).contains(&v) {
                v
            } else {
                // A wrapped angle lands in (-180, 180] (900 -> 180, FEATURE_BRUSH 8.2 A10; B5).
                let w = (v + 180.0).rem_euclid(360.0) - 180.0;
                if w <= -180.0 { 180.0 } else { w }
            }
        };
        BrushSettings {
            size_px: clamp(self.size_px, d.size_px, 1.0, 2048.0),
            hardness: clamp(self.hardness, d.hardness, 0.0, 1.0),
            opacity: clamp(self.opacity, d.opacity, 0.01, 1.0),
            flow: clamp(self.flow, d.flow, 0.01, 1.0),
            spacing: clamp(self.spacing, d.spacing, 0.02, 1.0),
            turn_deg: angle(self.turn_deg),
            tilt_deg: angle(self.tilt_deg),
            roll_deg: angle(self.roll_deg),
            offset: clamp(self.offset, d.offset, -1.0, 1.0),
            pressure_size_min: clamp(self.pressure_size_min, d.pressure_size_min, 0.0, 1.0),
            pressure_flow_min: clamp(self.pressure_flow_min, d.pressure_flow_min, 0.0, 1.0),
            ..self
        }
    }

    /// The core's pressure dynamics of these settings (branch `pen-input`).
    pub fn dynamics(&self) -> PressureDynamics {
        PressureDynamics {
            size: self.pressure_size,
            size_min: self.pressure_size_min,
            flow: self.pressure_flow,
            flow_min: self.pressure_flow_min,
        }
    }
}

/// Branch `pen-input` (PEN_INPUT_PLAN D8): where the pen pressure comes from. Windows Ink is the default and the only
/// source unless the user picks WinTab in Help > Tablet setup. Kept in the preferences (`Prefs::tablet_input`; a value
/// that does not read is Windows Ink).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabletInput {
    #[default]
    WindowsInk,
    WinTab,
}

/// Branch `companion` (COMPANION_PLAN 3): what one finger on the tablet does over an editing view.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FingerMode {
    /// "Navigate in the view, click elsewhere" (orbit / pan / zoom in the views, a mouse over the panels).
    #[default]
    Navigate,
    /// "Act like the mouse everywhere".
    Mouse,
}

/// Branch `companion` (COMPANION_PLAN 3 "Prefs", ARCHITECTURE 7): the Tablet companion settings in `Prefs::companion`.
/// Every field falls back to its default when missing; the whole entry falls back to the default when it does not
/// read. Ignored in self-test runs (decision C13).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompanionPrefs {
    /// The links the user turned on (all off by default; they stay on across restarts).
    pub usb: bool,
    pub wifi: bool,
    pub bluetooth: bool,
    /// The TCP port of the session listener (`crate::companion::DEFAULT_PORT`).
    pub port: u16,
    /// The adb.exe the user browsed to (first in the search order, COMPANION_PLAN 6.1).
    pub adb_path: Option<PathBuf>,
    /// This desktop's id: 16 random bytes as hex, made the first time a link is turned on.
    pub desktop_id: String,
    /// The paired tablets; each token is stored as hex in plain text in this per-user file.
    pub paired: Vec<crate::companion::auth::PairedTablet>,
    /// "Let the tablet resize this window to its shape".
    pub allow_resize: bool,
    pub fingers: FingerMode,
}

impl Default for CompanionPrefs {
    fn default() -> CompanionPrefs {
        CompanionPrefs {
            usb: false,
            wifi: false,
            bluetooth: false,
            port: crate::companion::DEFAULT_PORT,
            adb_path: None,
            desktop_id: String::new(),
            paired: Vec::new(),
            allow_resize: true,
            fingers: FingerMode::Navigate,
        }
    }
}

/// The tab of the lower area of the side panel (FEATURE_BRUSH 6.2, FEATURE_TEXT 6.1, ROUND4_PLAN 2.2 item 10). Starts
/// on Layers; objects-tabs: the chosen tab is kept in the preferences (`Prefs::side_lower_tab`, written at exit). A tab
/// whose condition is not met is greyed out and the area shows Layers, whatever this says; a tool change never writes
/// it.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum SideTab {
    #[default]
    Layers,
    Brush,
    Text,
}

impl SideTab {
    /// The three tabs in the order of the strip.
    pub const ALL: [SideTab; 3] = [SideTab::Layers, SideTab::Brush, SideTab::Text];

    /// Position in the strip (index into `SideRects::tab_buttons`).
    pub fn index(self) -> usize {
        match self {
            SideTab::Layers => 0,
            SideTab::Brush => 1,
            SideTab::Text => 2,
        }
    }

    /// The label of the tab.
    pub fn label(self) -> &'static str {
        match self {
            SideTab::Layers => "Layers",
            SideTab::Brush => "Brush",
            SideTab::Text => "Text",
        }
    }

    /// The name stored in `Prefs::side_lower_tab`.
    pub fn pref_name(self) -> &'static str {
        match self {
            SideTab::Layers => "layers",
            SideTab::Brush => "brush",
            SideTab::Text => "text",
        }
    }

    /// The tab a `Prefs::side_lower_tab` value names; Layers for anything else.
    pub fn from_pref_name(name: &str) -> SideTab {
        SideTab::ALL.into_iter().find(|t| t.pref_name() == name).unwrap_or_default()
    }
}

/// objects-tabs (WP4): the tab of the upper group of the side panel. Starts on Tool; all four are always available and
/// a tool change never writes it (OBJECTS_TABS_PLAN 6.4). Kept in the preferences (`Prefs::side_upper_tab`).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpperTab {
    #[default]
    Tool,
    Color,
    TextureSets,
    Objects,
}

impl UpperTab {
    /// The four tabs in the order of the strip.
    pub const ALL: [UpperTab; 4] = [UpperTab::Tool, UpperTab::Color, UpperTab::TextureSets, UpperTab::Objects];

    /// Position in the strip (index into `SideRects::upper_tab_buttons`).
    pub fn index(self) -> usize {
        match self {
            UpperTab::Tool => 0,
            UpperTab::Color => 1,
            UpperTab::TextureSets => 2,
            UpperTab::Objects => 3,
        }
    }

    /// The label of the tab.
    pub fn label(self) -> &'static str {
        match self {
            UpperTab::Tool => "Tool",
            UpperTab::Color => "Color",
            UpperTab::TextureSets => "Texture sets",
            UpperTab::Objects => "Objects",
        }
    }
}

/// The page of the Text tab (FEATURE_TEXT 6.4).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
#[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
pub enum TextPage {
    #[default]
    Character,
    Warp,
    Path,
}

/// Rectangles of the side panel in points, written by the panels every frame and read by tests and scenarios
/// (FEATURE_BRUSH 6.1, ROUND4_PLAN 3.3). `Default` = all empty.
#[derive(Clone, Debug, PartialEq)]
pub struct SideRects {
    /// The side panel's content rect.
    pub panel: egui::Rect,
    /// objects-tabs (WP4): the whole upper group (strip and page).
    pub upper_area: egui::Rect,
    /// The buttons of the upper strip, indexed by `UpperTab::index`.
    pub upper_tab_buttons: [egui::Rect; 4],
    /// The visible page of the upper group, below its strip.
    pub upper_page: egui::Rect,
    /// The visible Tool page (= `upper_page`, = `PanelHit::SettingsArea`) in frames where the Tool page is shown, else
    /// `Rect::NOTHING`.
    pub tool_area: egui::Rect,
    /// The Tool options as laid out (unclipped; they reach below `tool_area` when the page scrolls), in the same frames
    /// as `tool_area`, else `Rect::NOTHING`.
    pub tool_options: egui::Rect,
    /// The visible Color page (= `upper_page`, = `PanelHit::ColorSection`) in frames where the Color page is shown,
    /// else `Rect::NOTHING`.
    pub color: egui::Rect,
    /// `Some` (the visible page) exactly in the frames in which the Texture sets page was drawn.
    pub sets_tab: Option<egui::Rect>,
    /// `Some` (the visible page) exactly in the frames in which the Objects page was drawn.
    pub objects_tab: Option<egui::Rect>,
    /// The lower area with the tab strip (= `PanelHit::LayersArea`).
    pub bottom_area: egui::Rect,
    /// The buttons of the tab strip, indexed by `SideTab::index`.
    pub tab_buttons: [egui::Rect; 3],
    /// `Some` (the visible content rect) exactly in the frames in which the Brush tab's content was drawn.
    pub brush_tab: Option<egui::Rect>,
    /// The Brush tab's content as laid out (unclipped; it reaches beyond `brush_tab` when the tab scrolls), in the same
    /// frames as `brush_tab` (round 4, B4: the self-test asserts that every row of the tab is inside the lower area).
    pub brush_tab_content: Option<egui::Rect>,
    /// `Some` (the visible content rect) exactly in the frames in which the Text tab's content was drawn.
    pub text_tab: Option<egui::Rect>,
}

impl Default for SideRects {
    fn default() -> SideRects {
        SideRects {
            panel: egui::Rect::NOTHING,
            upper_area: egui::Rect::NOTHING,
            upper_tab_buttons: [egui::Rect::NOTHING; 4],
            upper_page: egui::Rect::NOTHING,
            tool_area: egui::Rect::NOTHING,
            tool_options: egui::Rect::NOTHING,
            color: egui::Rect::NOTHING,
            sets_tab: None,
            objects_tab: None,
            bottom_area: egui::Rect::NOTHING,
            tab_buttons: [egui::Rect::NOTHING; 3],
            brush_tab: None,
            brush_tab_content: None,
            text_tab: None,
        }
    }
}

/// A brush shape loaded from a model file, ready for strokes.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
pub struct ReadyShape {
    pub path: PathBuf,
    /// The file name, for the Shape list and the messages.
    pub name: String,
    pub field: Arc<TipField>,
    /// How its openings were closed for this field (`CloseMode::Refuse` = it had none to close).
    pub close: CloseMode,
    /// The file has openings and also closed parts, so the Brush tab's "Openings" control offers "Use only the closed
    /// parts" (plan section 7 item 5); false for a file without openings. B5 fills it from
    /// `TipError::HasOpenings::closed_parts` or, for a bake with a stored choice, from `TipReport::closed_parts`.
    pub closed_parts: bool,
}

/// A load of a brush shape that is running (a background job).
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
pub struct PendingShape {
    /// `BrushShapeState::next_request` when it was started; only the result with this number is used.
    pub request: u64,
    pub path: PathBuf,
    /// "Load..." (modal, with messages) rather than a quiet reload.
    pub by_user: bool,
    /// The Eraser was the active tool when "Load..." was chosen: the result is given to the eraser.
    pub for_eraser: bool,
    /// How the openings are closed in this bake (`Refuse` = ask when the file has openings).
    pub close: CloseMode,
}

/// The one loaded model shape, shared by brush and eraser (FEATURE_BRUSH 4.2, 4.4). Three independent parts, so that a
/// new load or a failed load never takes away the shape that strokes are using.
#[derive(Clone, Debug, Default)]
#[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
pub struct BrushShapeState {
    /// What strokes use.
    pub current: Option<ReadyShape>,
    /// A load that is running.
    pub pending: Option<PendingShape>,
    /// The last load that failed (path, message), for the "!" marker.
    pub last_error: Option<(PathBuf, String)>,
    /// Counts up with every load that is started.
    pub next_request: u64,
    /// The "no shape loaded" toast was shown in this program run.
    pub warned_missing: bool,
}

/// The question of plan section 7 item 5: a shape file has openings; how should they be closed? Shown as
/// `Dialog::CloseOpenings`; answered with `AppAction::AnswerOpenings`.
#[derive(Clone, Debug)]
pub struct OpeningsDialogState {
    pub path: PathBuf,
    /// Number of openings (boundary loops).
    pub count: u32,
    /// Size of the largest opening as a share of the model's size, `0..=1`.
    pub largest: f32,
    /// The model also has closed parts ("Use only the closed parts" is offered).
    pub closed_parts: bool,
    /// The choice the dialog shows as selected (`CloseMode::FlatCaps` at first).
    pub choice: CloseMode,
    /// The load that asked: started by the user's "Load..." and for which tool.
    pub by_user: bool,
    pub for_eraser: bool,
}

/// The gap selected in the gap strip of the Text tab (FEATURE_TEXT 6.2): the gap after character `after` of the text
/// stamp `stamp`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct TextGapSel {
    pub stamp: StampId,
    pub after: u32,
}

/// What the active tool showed in the last frame, for the self-test: scenarios see `AppState` only, not the tools
/// (round 4). Written by the tools in `hover` / `drag` (FEATURE_BRUSH 5.11, FEATURE_TEXT 5.14); cleared by nobody else.
#[derive(Clone, Debug, Default, PartialEq)]
#[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
pub struct ToolProbe {
    /// The shape cursor of the brush in the 3D view (`cursor_segments`, test A18 / scenario `brush_tab`): number of
    /// footprint and of wire segments of the last frame; 0 for Circular and without a shape.
    pub cursor_footprint: usize,
    pub cursor_wires: usize,
    /// The Text tool's see-through picture under the pointer (`StampPlaceTool::hover_preview`, scenario `text_tab`):
    /// the spec it was made for and its size in pixels; `None` without one.
    pub text_hover: Option<(wte_core::TextSpec, [u32; 2])>,
    /// Round 6 (R6-2): the two Wrap option controls ("Jump small gaps", "Ends meet | Overlap") as the tool options drew
    /// them in the last frame: `Some(enabled)`, `None` when they were not shown (scenario `wrap_surface`, step 4d).
    pub wrap_controls: Option<bool>,
    /// accept_fixes (items A, E2): the export dialog's preview caption and its file list lines as drawn in the last
    /// frame the dialog showed them ("<name>" + " (hidden)" / " separate layer" / " will be replaced"). Left as they
    /// were when the dialog closes.
    pub export_caption: Option<String>,
    pub export_files: Vec<String>,
}

/// Where the sample of one font family lies in the sample atlases.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct FontSampleCell {
    /// Index into `FontSampleState::atlases`.
    pub atlas: usize,
    /// The sample's rectangle in the atlas, in pixels.
    pub rect: egui::Rect,
}

/// The samples of the font picker (FEATURE_TEXT 5.13, 6.3): one per family, made by a non-modal job the first time the
/// picker opens (`AppAction::LoadFontSamples`) and packed into atlas textures by `text_ops.rs`. (No `Debug`: egui's
/// `TextureHandle` has none.)
#[derive(Clone, Default)]
#[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
pub struct FontSampleState {
    /// Number of catalog families the samples were requested for (the catalog grows once, when the system scan
    /// arrives); `None` = never requested.
    pub requested_for: Option<usize>,
    /// The sample job is running.
    pub loading: bool,
    /// Family name -> its cell; a family without a sample has no entry.
    pub cells: HashMap<String, FontSampleCell>,
    pub atlases: Vec<egui::TextureHandle>,
}

/// Settings of the Stamp tool (image decals).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StampSettings {
    /// The image the tool places. Bound to the open document, therefore never serialized (a `BlobId` of one document
    /// must never be used with another; `install_document` resets it, 8.9).
    #[serde(skip)]
    pub source: Option<BlobId>,
    /// Display name of `source`; not serialized either.
    #[serde(skip)]
    pub source_name: String,
    /// Larger side of the decal as a fraction of the model diagonal (3D) or of the texture width (2D).
    pub size_frac: f32,
    pub rotation_deg: f32,
    pub project_through: bool,
    /// Round 6 (R6-2): how new 3D stamps and texts reach the surface (Project | Wrap), shared by the Stamp and the Text
    /// tool like `project_through`. Not serialized: new texts and images start in Project in every session and the
    /// tools keep the chosen mode for the session (ROUND6_PLAN 11, Q1). `WTE_STAMP_MODE=wrap` sets Wrap at start.
    #[serde(skip)]
    pub mode: SurfaceMode,
    /// Round 6 (R6-2): the two Wrap options new 3D stamps and texts get (Jump small gaps, Ends meet | Overlap); kept
    /// for the session like `mode`, used only in Wrap.
    #[serde(skip)]
    pub wrap: WrapOptions,
}

impl Default for StampSettings {
    /// No source, 15% size, no rotation, no project-through, Project mode with the default Wrap options.
    fn default() -> StampSettings {
        StampSettings {
            source: None,
            source_name: String::new(),
            size_frac: 0.15,
            rotation_deg: 0.0,
            project_through: false,
            mode: SurfaceMode::Project,
            wrap: WrapOptions::default(),
        }
    }
}

/// Round 6 (R6-2): the start mode of new 3D stamps from the value of `WTE_STAMP_MODE` (a measuring aid): "wrap" (any
/// case, spaces ignored) gives Wrap, anything else or no value gives Project.
pub(crate) fn stamp_mode_from_env(value: Option<&str>) -> SurfaceMode {
    match value {
        Some(v) if v.trim().eq_ignore_ascii_case("wrap") => SurfaceMode::Wrap,
        _ => SurfaceMode::Project,
    }
}

/// Settings of the Text tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    pub content: String,
    pub family: String,
    pub style: String,
    pub color: [u8; 4],
    pub align: TextAlign,
    pub line_spacing: f32,
    /// Fraction of the em.
    pub letter_spacing: f32,
    pub outline: bool,
    /// Fraction of the em.
    pub outline_width_em: f32,
    pub outline_color: [u8; 4],
    /// The UI "Size": on-model height of one em (one text line) as a fraction of the model diagonal (3D placement)
    /// or of the texture height (2D placement). There is no raster-size setting: the core picks `TextSpec.size_px`
    /// (sections 4.3, 6.5).
    pub em_frac: f32,
    /// Thickening, fraction of the em, `0..=0.08` (`TextFx::bold_em`; FEATURE_TEXT 4.2). 0 = off.
    pub bold_em: f32,
    /// Slant in degrees, `-45..=45` (`TextFx::slant_deg`). 0 = off.
    pub slant_deg: f32,
    pub warp: Option<Warp>,
    pub path: Option<TextPath>,
}

impl Default for TextSettings {
    /// "Text" in the bundled default font, black, left aligned, no outline (6% white when switched on), 6% size.
    fn default() -> TextSettings {
        TextSettings {
            content: "Text".to_owned(),
            family: DEFAULT_FONT_FAMILY.to_owned(),
            style: DEFAULT_FONT_STYLE.to_owned(),
            color: [0, 0, 0, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: false,
            outline_width_em: 0.06,
            outline_color: [255, 255, 255, 255],
            em_frac: 0.06,
            bold_em: 0.0,
            slant_deg: 0.0,
            warp: None,
            path: None,
        }
    }
}

/// Settings of all tools.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSettings {
    pub brush: BrushSettings,
    pub eraser: BrushSettings,
    #[serde(with = "fill_scope_serde")]
    pub fill_scope: FillScope,
    pub stamp: StampSettings,
    pub text: TextSettings,
}

impl Default for ToolSettings {
    /// Default brush and eraser, island fill, default stamp and text settings.
    fn default() -> ToolSettings {
        ToolSettings {
            brush: BrushSettings::default(),
            eraser: BrushSettings::default(),
            fill_scope: FillScope::Island,
            stamp: StampSettings::default(),
            text: TextSettings::default(),
        }
    }
}

/// Paint colours (straight sRGB RGBA) and the swatch palette.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorState {
    pub primary: [u8; 4],
    pub secondary: [u8; 4],
    /// `SWATCH_COUNT` entries.
    pub swatches: Vec<[u8; 4]>,
}

/// The built-in palette: a grey ramp, the primaries and a row of muted tones.
const DEFAULT_SWATCHES: [[u8; 4]; SWATCH_COUNT] = [
    [0, 0, 0, 255],
    [64, 64, 64, 255],
    [128, 128, 128, 255],
    [192, 192, 192, 255],
    [255, 255, 255, 255],
    [230, 25, 40, 255],
    [245, 130, 30, 255],
    [255, 225, 25, 255],
    [60, 180, 75, 255],
    [0, 130, 200, 255],
    [70, 60, 200, 255],
    [145, 30, 180, 255],
    [240, 50, 230, 255],
    [70, 240, 240, 255],
    [170, 255, 195, 255],
    [255, 215, 180, 255],
    [128, 0, 0, 255],
    [154, 99, 36, 255],
    [128, 128, 0, 255],
    [0, 100, 60, 255],
    [0, 0, 128, 255],
    [90, 60, 40, 255],
    [220, 190, 255, 255],
    [255, 250, 200, 255],
];

impl Default for ColorState {
    /// Black primary, white secondary, the built-in palette.
    fn default() -> ColorState {
        ColorState { primary: [0, 0, 0, 255], secondary: [255, 255, 255, 255], swatches: DEFAULT_SWATCHES.to_vec() }
    }
}

/// What is under the pointer; rewritten by the views every frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverInfo {
    pub view: Option<ViewId>,
    pub set: Option<TextureSetId>,
    pub uv: Option<[f32; 2]>,
    pub texel: Option<[u32; 2]>,
    pub rgba: Option<[u8; 4]>,
    pub world: Option<[f32; 3]>,
}

/// Orbit camera of a 3D view. Its methods live in `view/camera.rs`.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrbitCamera {
    #[serde(with = "vec3_serde")]
    pub target: glam::Vec3,
    /// Radians around +Y; 0 looks from +Z.
    pub yaw: f32,
    /// Radians above the horizon, clamped to +-89 degrees.
    pub pitch: f32,
    pub distance: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub ortho: bool,
}

impl Default for OrbitCamera {
    /// Three-quarter view of the origin from distance 3 with a 45 degree field of view, perspective.
    fn default() -> OrbitCamera {
        OrbitCamera {
            target: glam::Vec3::ZERO,
            yaw: 0.6,
            pitch: 0.35,
            distance: 3.0,
            fov_y: std::f32::consts::FRAC_PI_4,
            ortho: false,
        }
    }
}

/// Pan / zoom of the 2D (UV) view. Its methods live in `view/view2d.rs`.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UvViewState {
    #[serde(with = "vec2_serde")]
    pub center_uv: glam::Vec2,
    /// Points per texel, `0.02..=64`.
    pub zoom: f32,
}

impl Default for UvViewState {
    /// Centred on the texture at 1 point per texel. A session starts with this value; the 2D view fits the texture
    /// on its first draw (8.9), which it can recognise by the state still being equal to this default.
    fn default() -> UvViewState {
        UvViewState { center_uv: glam::Vec2::splat(0.5), zoom: 1.0 }
    }
}

/// One open document plus the app-side selection and view state that belongs to it.
pub struct Session {
    pub doc: Document,
    /// The `.wtep` path once saved or opened.
    pub path: Option<PathBuf>,
    pub force_save_as: bool,
    pub active_set: TextureSetId,
    pub selected_stamp: Option<StampId>,
    pub camera: OrbitCamera,
    pub preview_camera: OrbitCamera,
    pub uv_view: UvViewState,
    /// Process-wide unique; changes whenever the document object is replaced (=> `GpuScene` rebuild). It also
    /// identifies the session a Save job belongs to.
    pub scene_generation: u64,
    /// The gap selected in the gap strip (FEATURE_TEXT 4.2, 6.2). Here, not in `AppState`: stamp ids are counted per
    /// document, and a new session clears it.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    pub text_gap: Option<TextGapSel>,
    /// The layers selected in the Layers list (Ctrl / Shift-click), raw and possibly stale: read it through
    /// `layer_selection`, which normalises it. App state only: not in the document, not in history, not saved.
    pub selected_layers: Vec<LayerId>,
    /// objects-tabs: the selected objects (indices into `doc.mesh.objects`). App state only: not in the document, not
    /// in history, not saved.
    pub selected_objects: std::collections::BTreeSet<u32>,
    /// The object a Shift-click in the Objects list extends from (the last plain / Ctrl-clicked object).
    pub object_anchor: Option<u32>,
    /// +1 at every change of `selected_objects` (the GPU highlight buffer follows it).
    pub object_selection_rev: u64,
    /// Request: the Objects list scrolls this row into view once (set by a pick in the 3D view).
    pub object_reveal: Option<u32>,
}

impl Session {
    /// Re-validates the app-side selection after any document change (called by dispatch, section 8.3): clears
    /// `selected_stamp` if `doc.stamp(id)` is `None`, clamps `active_set` to an existing set and drops selected objects
    /// that the mesh does not have.
    pub fn validate(&mut self) {
        if let Some(id) = self.selected_stamp
            && self.doc.stamp(id).is_none()
        {
            self.selected_stamp = None;
        }
        let set_count = self.doc.sets.len();
        if self.active_set.index() >= set_count {
            self.active_set = TextureSetId(set_count.saturating_sub(1) as u32);
        }
        // objects-tabs: selected objects beyond the mesh's objects are dropped, a stale anchor / reveal cleared.
        let objects = self.doc.mesh.object_count();
        if self.selected_objects.last().is_some_and(|&o| o >= objects) {
            self.selected_objects.retain(|&o| o < objects);
            self.object_selection_rev += 1;
        }
        if self.object_anchor.is_some_and(|o| o >= objects) {
            self.object_anchor = None;
        }
        if self.object_reveal.is_some_and(|o| o >= objects) {
            self.object_reveal = None;
        }
    }

    /// The selected layers of the active set, bottom to top. Normalised on every read: ids that are gone or belong to
    /// another set are dropped; when the active layer is not among them the selection is just the active layer; no
    /// active layer => empty. So whatever changes the active layer (a click on a stamp, New, Delete, Undo, another
    /// set) collapses a multi-selection by itself.
    pub fn layer_selection(&self) -> Vec<LayerId> {
        let Some(set) = self.doc.set(self.active_set) else { return Vec::new() };
        let Some(active) = set.active.filter(|id| set.layer_index(*id).is_some()) else { return Vec::new() };
        if !self.selected_layers.contains(&active) {
            return vec![active];
        }
        set.layers.iter().map(|l| l.id).filter(|id| self.selected_layers.contains(id)).collect()
    }
}

/// A file-level operation waiting behind the "unsaved changes" prompt or behind a running save.
#[derive(Clone, Debug)]
pub enum Pending {
    NewFromModel(Option<PathBuf>),
    OpenProject(Option<PathBuf>),
    OpenSample(SampleKind),
    Quit,
}

/// The base-texture choice of one set in the import dialog.
#[derive(Clone, Debug)]
pub enum SetChoiceUi {
    Detected,
    /// A manually chosen image: `bytes` = the original file (kept for R7), `image` = the decoded pixels.
    File { path: PathBuf, bytes: Option<Arc<[u8]>>, image: Option<PixelBuf>, error: Option<String> },
    Blank { side: u32, fill: [u8; 4] },
}

/// State of the import dialog.
#[derive(Clone, Debug)]
pub struct ImportDialogState {
    pub path: PathBuf,
    pub model: Option<ImportedModel>,
    /// One entry per texture set.
    pub choices: Vec<SetChoiceUi>,
    /// One `uv_layout_image(.., 256)` per set, from the import job.
    pub uv_thumbs: Vec<PixelBuf>,
    /// One per set (`TexelMap::build` at 256x256, gutter 0), from the import job.
    pub uv_stats: Vec<UvStats>,
    pub fallback_uvs: bool,
    pub loading: bool,
    pub error: Option<String>,
}

/// State of the export dialog.
#[derive(Clone, Debug)]
pub struct ExportDialogState {
    pub options: ExportOptions,
    pub format_index: usize,
    pub jpeg_quality: u8,
    pub jpeg_matte: [u8; 3],
    pub show_preview: bool,
    /// `Some` = the "N files will be replaced" step is showing.
    pub confirm_replace: Option<Vec<PathBuf>>,
    /// The layer selection captured when the dialog opened, bottom to top: what the Layers choice "Selected (N)"
    /// exports.
    pub selected_layers: Vec<LayerId>,
    /// fix_export item 1: the texture sets and file kinds of the *other* side of the Layers choice, when the dialog
    /// was opened for layers ("Export this layer...", "Export selected layers..."): the dialog's normal ticks while the
    /// choice is Selected, the layer preset (per-layer PNGs only) while it is Visible / All. Switching sides swaps
    /// them (`dialogs::export::switch_scope`), so Visible / All export everything again. `None` for a dialog opened
    /// with Export...: its ticks stay as they are.
    pub other_ticks: Option<ExportTicks>,
}

/// The texture sets and file kinds of the export options (one side of the Layers choice, fix_export item 1).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportTicks {
    pub sets: Option<Vec<TextureSetId>>,
    pub flat: Option<wte_core::FlatFormat>,
    pub per_layer_png: bool,
    pub ora: bool,
    pub psd: bool,
}

impl ExportTicks {
    /// The ticks of `opts`.
    pub fn of(opts: &ExportOptions) -> ExportTicks {
        ExportTicks { sets: opts.sets.clone(), flat: opts.flat, per_layer_png: opts.per_layer_png, ora: opts.ora, psd: opts.psd }
    }

    /// Writes the ticks into `opts`.
    pub fn apply(self, opts: &mut ExportOptions) {
        opts.sets = self.sets;
        opts.flat = self.flat;
        opts.per_layer_png = self.per_layer_png;
        opts.ora = self.ora;
        opts.psd = self.psd;
    }
}

/// fix_export item 6: the longest UI-thread work of one frame (`App::logic` start to `App::ui` end) since the last
/// `reset`. The self-test asserts on it ("the UI never froze": an export right after a wrapped stamp's placement must
/// not wait for its Final bake on the UI thread).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameWork {
    start: Option<std::time::Instant>,
    pub max: std::time::Duration,
    pub frames: u64,
}

impl FrameWork {
    pub fn begin(&mut self) {
        self.start = Some(std::time::Instant::now());
    }

    pub fn end(&mut self) {
        if let Some(start) = self.start.take() {
            self.max = self.max.max(start.elapsed());
            self.frames += 1;
        }
    }

    pub fn reset(&mut self) {
        self.max = std::time::Duration::ZERO;
        self.frames = 0;
    }
}

/// The modal dialog that is open, if any.
#[derive(Clone, Debug)]
pub enum Dialog {
    None,
    Import(Box<ImportDialogState>),
    Unsaved { then: Pending },
    Export(Box<ExportDialogState>),
    Message { title: String, lines: Vec<String> },
    About,
    Shortcuts,
    /// Branch `pen-input`: Help > Tablet setup (`dialogs/tablet.rs`).
    TabletHelp,
    /// "How should the openings of this brush shape be closed?" (plan section 7 item 5; drawn by `brush_flows.rs`).
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    CloseOpenings(Box<OpeningsDialogState>),
}

/// User preferences, stored as JSON by `prefs.rs`. Every field falls back to its default when missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Most recent first, at most 10.
    pub recent: Vec<PathBuf>,
    pub last_dir: Option<PathBuf>,
    pub last_export_dir: Option<PathBuf>,
    /// Inner window size in points (the normal, not maximized, size).
    pub window_size: [f32; 2],
    /// The window was maximized when it closed.
    pub window_maximized: bool,
    pub flags_wireframe: bool,
    /// Keep a `.bak` of the previous project file when saving.
    pub keep_backup: bool,
    /// The user's swatch palette; empty = the built-in palette.
    pub swatches: Vec<[u8; 4]>,
    /// Brush and eraser settings, written at every write of the file (`brush_flows::sync_prefs`, FEATURE_BRUSH 4.4).
    /// Read leniently (FEATURE_BRUSH 4.5): a key that does not read keeps its default, the rest is kept.
    #[serde(deserialize_with = "lenient_prefs_serde::brush")]
    pub brush: BrushSettings,
    #[serde(deserialize_with = "lenient_prefs_serde::brush")]
    pub eraser: BrushSettings,
    /// The remembered brush shape file. Changed only by a successful "Load..." and by "Remove file shape".
    pub brush_shape_path: Option<PathBuf>,
    /// How the openings of a shape file are closed, as the user answered (plan section 7 item 5), per file: a reload of
    /// that file closes them the same way without asking again. Read leniently: an entry that does not read is
    /// dropped (that file asks again), the others are kept.
    #[serde(deserialize_with = "lenient_prefs_serde::close_map")]
    pub brush_shape_close: std::collections::BTreeMap<PathBuf, CloseMode>,
    /// objects-tabs (WP4): the tab of the side panel's upper group at exit. Read leniently: a value that does not read
    /// is Tool.
    #[serde(deserialize_with = "lenient_prefs_serde::upper_tab")]
    pub side_upper_tab: UpperTab,
    /// The chosen tab of the lower group at exit (`SideTab::pref_name`: "layers", "brush" or "text"); anything else
    /// reads as "layers".
    #[serde(deserialize_with = "lenient_prefs_serde::lower_tab")]
    pub side_lower_tab: String,
    /// The height of the lower group in points when the user dragged the split; `None` = automatic. A value that is
    /// not a number reads as `None`; `prefs::from_json` keeps only a finite value in `100..=2000`.
    #[serde(deserialize_with = "lenient_prefs_serde::split")]
    pub side_split: Option<f32>,
    /// Branch `pen-input`: the pressure source (Help > Tablet setup). Read leniently: a value that does not read is
    /// Windows Ink.
    #[serde(deserialize_with = "lenient_prefs_serde::tablet_input")]
    pub tablet_input: TabletInput,
    /// Branch `companion`: the Tablet companion settings. Read leniently: an entry that does not read is the default.
    #[serde(deserialize_with = "lenient_prefs_serde::companion")]
    pub companion: CompanionPrefs,
}

impl Default for Prefs {
    /// No recent files, no remembered folders, a 1280x800 window, wireframe off, no backups, the built-in palette.
    fn default() -> Prefs {
        Prefs {
            recent: Vec::new(),
            last_dir: None,
            last_export_dir: None,
            window_size: [1280.0, 800.0],
            window_maximized: false,
            flags_wireframe: false,
            keep_backup: false,
            swatches: Vec::new(),
            brush: BrushSettings::default(),
            eraser: BrushSettings::default(),
            brush_shape_path: None,
            brush_shape_close: std::collections::BTreeMap::new(),
            side_upper_tab: UpperTab::Tool,
            side_lower_tab: SideTab::Layers.pref_name().to_owned(),
            side_split: None,
            tablet_input: TabletInput::WindowsInk,
            companion: CompanionPrefs::default(),
        }
    }
}

/// The running job shown in the status bar or, when `modal`, in the progress dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct BusyInfo {
    pub title: String,
    pub stage: String,
    /// `0..=1`.
    pub fraction: f32,
    pub modal: bool,
}

/// The whole UI-thread state of the application.
pub struct AppState {
    pub session: Option<Session>,
    pub prefs: Prefs,
    pub view_mode: ViewMode,
    /// Width share of the 3D view in Split mode.
    pub split_ratio: f32,
    /// The non-Preview mode to return to when leaving Preview.
    pub last_edit_mode: ViewMode,
    pub flags: ViewFlags,
    pub preview: PreviewSettings,
    pub tool: ToolKind,
    pub tools: ToolSettings,
    pub colors: ColorState,
    pub hover: HoverInfo,
    /// Written by `view::central` every frame (points); read by the self-test.
    pub view_rects: HashMap<ViewId, egui::Rect>,
    /// Written by the Layers panel every frame (points): the rects of the row controls; read by the self-test.
    pub panel_rects: HashMap<PanelHit, egui::Rect>,
    /// Written by `App::ui` every frame (points): the rect of the right side panel; read by the self-test.
    pub side_panel_rect: Option<egui::Rect>,
    /// Written by `App::ui` every frame (points): the rect of the central view area (mode strip included), where the
    /// toasts are anchored.
    pub central_rect: Option<egui::Rect>,
    pub dialog: Dialog,
    pub toasts: crate::toasts::Toasts,
    /// Command queue; drained by dispatch at the end of `App::ui`.
    pub actions: Vec<AppAction>,
    pub fonts: FontCatalog,
    /// True while the system-font scan is running; set by whoever starts the Fonts job (8.9).
    pub fonts_loading: bool,
    /// Mirrored from `Jobs` each frame.
    pub busy: Option<BusyInfo>,
    /// Set by the active tool.
    pub status_hint: String,
    /// Written by the status bar every frame (points): the rects of the items it drew, left to right then right to
    /// left; read by the self-test and the panel tests (items never overlap).
    pub status_rects: Vec<egui::Rect>,
    pub allow_close: bool,
    /// The last window title sent (`update_title` sends only on change).
    pub last_title: String,
    /// Written by `view::central` after `GpuScene::sync` (8.4); read by the self-test's `WaitIdle`.
    pub gpu_mips_stale: bool,
    /// Request: the Text tool options focus their content field next frame.
    pub focus_text_content: bool,
    /// `ctx.text_edit_focused()` at the end of the previous `shortcuts::handle` (8.10).
    pub typing_prev: bool,
    /// True while the scenario runner drives the app (no native dialogs, no prefs writes).
    pub selftest: bool,
    /// Throttle of the deferred stamp Preview bakes (`view::bake_stale_previews`, 8.4).
    pub preview_bakes: PreviewBakeClock,
    /// The chosen tab of the lower side-panel area (see `SideTab` for when it is shown); from `Prefs::side_lower_tab`
    /// at start, written back at exit (`panels::sync_prefs`).
    pub side_tab: SideTab,
    /// objects-tabs (WP4): the shown tab of the side panel's upper group; from `Prefs::side_upper_tab` at start,
    /// written back at exit.
    pub upper_tab: UpperTab,
    /// The page of the Text tab.
    pub text_page: TextPage,
    /// The brush shape loaded from a model file (FEATURE_BRUSH 4.4).
    pub brush_shape: BrushShapeState,
    /// The built-in Cube and Cylinder fields (in this order), made on first use.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    pub builtin_tips: [Option<Arc<TipField>>; 2],
    /// Rectangles of the side panel of the last frame.
    pub side_rects: SideRects,
    /// The font picker's samples.
    pub font_samples: FontSampleState,
    /// What the active tool showed in the last frame (for the self-test).
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    pub tool_probe: ToolProbe,
    /// Round 5 (R5-1): the Full bakes after a placement or a Final stamp edit run on a worker (`stamp_bakes`; the
    /// document's deferred Final bakes, set by `install_document`). False from `AppState::new`, so the app tests keep
    /// the synchronous behaviour; the real `WteApp::new` turns it on unless `WTE_SYNC_FINAL_BAKES=1`.
    pub async_final_bakes: bool,
    /// Mirror for the self-test runner and the tests: a Final bake is in flight, held or still to start
    /// (`stamp_bakes::pump`).
    pub final_bakes_pending: bool,
    /// The stamp just placed whose first bake is awaited: its "covers no texels" warning comes when that bake lands.
    pub await_first_bake: Option<StampId>,
    /// The bake in flight and an output held while a stroke is open (`stamp_bakes`).
    pub final_bakes: crate::stamp_bakes::FinalBakes,
    /// Round 8 (R8-2): the wrapped stamps' Preview and param-only jobs (`stamp_bakes`; on when `async_final_bakes`).
    pub wrap_jobs: crate::stamp_bakes::WrapJobs,
    /// Mirror for the self-test runner and the tests: a wrap job is in flight, an output held, a wrapped stamp waits
    /// for its Preview job or a param-only job was asked for (`stamp_bakes`).
    pub wrap_jobs_pending: bool,
    /// fix_export item 6: the longest frame of UI-thread work (the self-test's freeze check).
    pub frame_work: FrameWork,
    /// Branch `pen-input` (ARCHITECTURE 8.2): the pen's records, router, this frame's pen input and the eraser override.
    pub pen: crate::pen::PenState,
}

/// When the deferred stamp Preview bakes (`Document::bake_stale_previews`) may run next: after a bake that took `t`,
/// the next one waits `t` (capped), so a slow bake of a large decal on a large texture takes at most about half of
/// the frames of a drag, a wheel gesture or typing, and the gizmo and the UI stay responsive in between.
#[derive(Clone, Debug, Default)]
pub struct PreviewBakeClock {
    pub last_end: Option<std::time::Instant>,
    pub last_cost: std::time::Duration,
    /// Set when a bake is waiting: the frame should repaint after this delay.
    pub wake: Option<std::time::Duration>,
}

impl PreviewBakeClock {
    /// Longest wait between two deferred bakes.
    pub const MAX_WAIT: std::time::Duration = std::time::Duration::from_millis(400);

    /// True when a deferred bake may run at `now`; otherwise `wake` is set to the remaining wait.
    pub fn due(&mut self, now: std::time::Instant) -> bool {
        let Some(end) = self.last_end else { return true };
        let at = end + self.last_cost.min(Self::MAX_WAIT);
        if now >= at {
            true
        } else {
            self.wake = Some(at - now);
            false
        }
    }

    /// Records a bake that ran from `start` to `end`.
    pub fn record(&mut self, start: std::time::Instant, end: std::time::Instant) {
        self.last_cost = end.saturating_duration_since(start);
        self.last_end = Some(end);
    }
}

impl AppState {
    /// The state of a freshly started app: no document, 3D view, Brush tool, the bundled font catalog. `prefs`
    /// supplies the wireframe flag and (when not empty) the swatch palette.
    pub fn new(prefs: Prefs, selftest: bool) -> AppState {
        let flags = ViewFlags { wireframe: prefs.flags_wireframe, ..ViewFlags::default() };
        let mut colors = ColorState::default();
        if !prefs.swatches.is_empty() {
            colors.swatches = prefs.swatches.clone();
            colors.swatches.resize(SWATCH_COUNT, [255, 255, 255, 255]);
        }
        let side_tab = SideTab::from_pref_name(&prefs.side_lower_tab);
        let upper_tab = prefs.side_upper_tab;
        AppState {
            session: None,
            prefs,
            view_mode: ViewMode::View3D,
            split_ratio: 0.5,
            last_edit_mode: ViewMode::View3D,
            flags,
            preview: PreviewSettings::default(),
            tool: ToolKind::Brush,
            tools: ToolSettings::default(),
            colors,
            hover: HoverInfo::default(),
            view_rects: HashMap::new(),
            panel_rects: HashMap::new(),
            side_panel_rect: None,
            central_rect: None,
            dialog: Dialog::None,
            toasts: crate::toasts::Toasts::default(),
            actions: Vec::new(),
            fonts: FontCatalog::bundled(),
            fonts_loading: false,
            busy: None,
            status_hint: String::new(),
            status_rects: Vec::new(),
            allow_close: false,
            last_title: String::new(),
            gpu_mips_stale: false,
            focus_text_content: false,
            typing_prev: false,
            selftest,
            preview_bakes: PreviewBakeClock::default(),
            side_tab,
            upper_tab,
            text_page: TextPage::Character,
            brush_shape: BrushShapeState::default(),
            builtin_tips: [None, None],
            side_rects: SideRects::default(),
            font_samples: FontSampleState::default(),
            tool_probe: ToolProbe::default(),
            async_final_bakes: false,
            final_bakes_pending: false,
            await_first_bake: None,
            final_bakes: crate::stamp_bakes::FinalBakes::default(),
            wrap_jobs: crate::stamp_bakes::WrapJobs::default(),
            wrap_jobs_pending: false,
            frame_work: FrameWork::default(),
            pen: crate::pen::PenState::default(),
        }
    }

    /// The open document.
    pub fn doc(&self) -> Option<&Document> {
        self.session.as_ref().map(|s| &s.doc)
    }

    /// The open document, mutable.
    pub fn doc_mut(&mut self) -> Option<&mut Document> {
        self.session.as_mut().map(|s| &mut s.doc)
    }

    /// Queues a command for dispatch at the end of the frame.
    pub fn push(&mut self, a: AppAction) {
        self.actions.push(a);
    }
}

/// `FillScope` (a core type without serde support) as `"layer"` / `"island"`.
mod fill_scope_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use wte_core::FillScope;

    pub fn serialize<S: Serializer>(scope: &FillScope, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match scope {
            FillScope::Layer => "layer",
            FillScope::Island => "island",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<FillScope, D::Error> {
        let name = String::deserialize(deserializer)?;
        match name.as_str() {
            "layer" => Ok(FillScope::Layer),
            "island" => Ok(FillScope::Island),
            other => Err(serde::de::Error::unknown_variant(other, &["layer", "island"])),
        }
    }
}

/// Lenient reading of the round-4 entries of `Prefs` (FEATURE_BRUSH 4.5): a value this version cannot read (written
/// by a later version, edited by hand) is dropped on its own instead of making the whole preferences file invalid
/// (`prefs::load_from` replaces an invalid file by the defaults).
mod lenient_prefs_serde {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use serde::{Deserialize, Deserializer};
    use serde_json::Value;
    use wte_core::CloseMode;

    use super::{BrushSettings, CompanionPrefs, SideTab, TabletInput, UpperTab};

    /// Brush or eraser settings: every key is taken on its own; a key whose value does not read keeps its default.
    /// Sanitized (`BrushSettings::sanitized`).
    pub fn brush<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BrushSettings, D::Error> {
        Ok(brush_from_value(Value::deserialize(deserializer)?))
    }

    pub(super) fn brush_from_value(value: Value) -> BrushSettings {
        let Value::Object(entries) = value else { return BrushSettings::default() };
        let Ok(Value::Object(mut merged)) = serde_json::to_value(BrushSettings::default()) else {
            return BrushSettings::default();
        };
        for (key, v) in entries {
            let previous = merged.insert(key.clone(), v);
            if serde_json::from_value::<BrushSettings>(Value::Object(merged.clone())).is_err() {
                match previous {
                    Some(p) => merged.insert(key, p),
                    None => merged.remove(&key),
                };
            }
        }
        serde_json::from_value::<BrushSettings>(Value::Object(merged)).unwrap_or_default().sanitized()
    }

    /// The upper tab: a value that does not read is Tool.
    pub fn upper_tab<'de, D: Deserializer<'de>>(deserializer: D) -> Result<UpperTab, D::Error> {
        Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
    }

    /// The pressure source: a value that does not read is Windows Ink.
    pub fn tablet_input<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TabletInput, D::Error> {
        Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
    }

    /// Branch `companion`: the Tablet companion settings; an entry that does not read is the default (all links off).
    pub fn companion<'de, D: Deserializer<'de>>(deserializer: D) -> Result<CompanionPrefs, D::Error> {
        Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
    }

    /// The lower tab's name: anything but a known name reads as "layers".
    pub fn lower_tab<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        let tab = match Value::deserialize(deserializer)? {
            Value::String(name) => SideTab::from_pref_name(&name),
            _ => SideTab::Layers,
        };
        Ok(tab.pref_name().to_owned())
    }

    /// The user-sized split: a value that is not a number reads as `None` (automatic).
    pub fn split<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f32>, D::Error> {
        Ok(match Value::deserialize(deserializer)? {
            Value::Number(n) => n.as_f64().map(|v| v as f32),
            _ => None,
        })
    }

    /// The closing choice per shape file: an entry whose value does not read is dropped (that file asks again).
    pub fn close_map<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<PathBuf, CloseMode>, D::Error> {
        let Value::Object(entries) = Value::deserialize(deserializer)? else { return Ok(BTreeMap::new()) };
        Ok(entries
            .into_iter()
            .filter_map(|(path, v)| serde_json::from_value::<CloseMode>(v).ok().map(|mode| (PathBuf::from(path), mode)))
            .collect())
    }
}

/// `glam::Vec3` as `[x, y, z]` (glam is built without its serde feature).
mod vec3_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &glam::Vec3, serializer: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<glam::Vec3, D::Error> {
        <[f32; 3]>::deserialize(deserializer).map(glam::Vec3::from_array)
    }
}

/// `glam::Vec2` as `[x, y]` (glam is built without its serde feature).
mod vec2_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &glam::Vec2, serializer: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<glam::Vec2, D::Error> {
        <[f32; 2]>::deserialize(deserializer).map(glam::Vec2::from_array)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wte_core::{Placement, StampObject, StampSource, UvPlacement};

    fn round_trip<T: Serialize + for<'de> Deserialize<'de>>(value: &T) -> T {
        let json = serde_json::to_string(value).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    /// Round 6 (R6-2): `WTE_STAMP_MODE=wrap` (any case) starts in Wrap, anything else in Project; the mode and the Wrap
    /// options of the tools are kept for the session only (not in the preferences), so a new session starts in
    /// Project (ROUND6_PLAN 11, Q1).
    #[test]
    fn stamp_mode_env_switch_and_session_only_mode() {
        assert_eq!(stamp_mode_from_env(None), SurfaceMode::Project);
        assert_eq!(stamp_mode_from_env(Some("wrap")), SurfaceMode::Wrap);
        assert_eq!(stamp_mode_from_env(Some(" Wrap ")), SurfaceMode::Wrap);
        assert_eq!(stamp_mode_from_env(Some("project")), SurfaceMode::Project);
        assert_eq!(stamp_mode_from_env(Some("1")), SurfaceMode::Project);
        assert_eq!(stamp_mode_from_env(Some("")), SurfaceMode::Project);
        let fresh = ToolSettings::default();
        assert_eq!((fresh.stamp.mode, fresh.stamp.wrap), (SurfaceMode::Project, WrapOptions::default()));
        let mut tools = ToolSettings::default();
        tools.stamp.mode = SurfaceMode::Wrap;
        tools.stamp.wrap = WrapOptions { jump_gaps: true, overlap: true };
        let json = serde_json::to_value(&tools).unwrap();
        assert!(json["stamp"].get("mode").is_none() && json["stamp"].get("wrap").is_none(), "{json}");
        let back = round_trip(&tools);
        assert_eq!((back.stamp.mode, back.stamp.wrap), (SurfaceMode::Project, WrapOptions::default()));
    }

    #[test]
    fn defaults_match_the_contract() {
        let flags = ViewFlags::default();
        assert_eq!(flags.shading, ShadingMode::Textured);
        assert!(flags.lit && flags.uv_wire && flags.alpha_checker && !flags.wireframe && !flags.seams);
        assert_eq!(flags.backdrop, Backdrop::Gradient);

        assert_eq!(PreviewSettings::default().turntable_speed, 0.6);

        let tools = ToolSettings::default();
        assert!((1.0..=2048.0).contains(&tools.brush.size_px));
        assert_eq!(tools.stamp.size_frac, 0.15);
        assert_eq!(tools.stamp.source, None);
        assert_eq!(tools.text.content, "Text");
        assert_eq!(tools.text.family, DEFAULT_FONT_FAMILY);
        assert_eq!(tools.text.style, DEFAULT_FONT_STYLE);
        assert_eq!(tools.text.line_spacing, 1.0);
        assert_eq!(tools.text.letter_spacing, 0.0);
        assert_eq!(tools.text.outline_width_em, 0.06);
        assert_eq!(tools.text.em_frac, 0.06);

        let colors = ColorState::default();
        assert_eq!(colors.primary, [0, 0, 0, 255]);
        assert_eq!(colors.secondary, [255, 255, 255, 255]);
        assert_eq!(colors.swatches.len(), SWATCH_COUNT);

        let camera = OrbitCamera::default();
        assert_eq!(camera.fov_y, 45f32.to_radians());
        assert!(!camera.ortho && camera.distance > 0.0);
        assert!(camera.pitch.abs() < 89f32.to_radians());

        let uv = UvViewState::default();
        assert!((0.02..=64.0).contains(&uv.zoom));

        assert_eq!(HoverInfo::default().view, None);
    }

    #[test]
    fn shader_discriminants_are_stable() {
        assert_eq!(ShadingMode::Textured as u32, 0);
        assert_eq!(ShadingMode::Clay as u32, 1);
        assert_eq!(ShadingMode::UvChecker as u32, 2);
        assert_eq!(Backdrop::Dark as u32, 0);
        assert_eq!(Backdrop::Light as u32, 1);
        assert_eq!(Backdrop::Gradient as u32, 2);
        assert_eq!(Backdrop::Checker as u32, 3);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let mut tools = ToolSettings::default();
        tools.brush.size_px = 77.5;
        tools.eraser.hardness = 0.25;
        tools.fill_scope = FillScope::Layer;
        tools.stamp.rotation_deg = 30.0;
        tools.stamp.project_through = true;
        tools.text.content = "Line 1\nLine 2".to_owned();
        tools.text.align = TextAlign::Right;
        tools.text.outline = true;
        assert_eq!(round_trip(&tools), tools);

        let flags = ViewFlags { shading: ShadingMode::UvChecker, wireframe: true, backdrop: Backdrop::Checker, ..ViewFlags::default() };
        assert_eq!(round_trip(&flags), flags);

        let preview = PreviewSettings { lit: false, turntable: true, nearest: true, ..PreviewSettings::default() };
        assert_eq!(round_trip(&preview), preview);

        let colors = ColorState { primary: [1, 2, 3, 4], ..ColorState::default() };
        assert_eq!(round_trip(&colors), colors);

        let camera = OrbitCamera { target: glam::Vec3::new(1.0, -2.0, 3.5), yaw: 1.25, ortho: true, ..OrbitCamera::default() };
        assert_eq!(round_trip(&camera), camera);

        let uv = UvViewState { center_uv: glam::Vec2::new(0.25, 0.75), zoom: 3.0 };
        assert_eq!(round_trip(&uv), uv);

        assert_eq!(round_trip(&ViewMode::Split), ViewMode::Split);
        assert_eq!(round_trip(&ToolKind::Transform), ToolKind::Transform);
        assert_eq!(serde_json::to_string(&ViewMode::View3D).unwrap(), "\"view3d\"");
        assert_eq!(serde_json::to_string(&ViewMode::View2D).unwrap(), "\"view2d\"");
        assert_eq!(serde_json::to_string(&ShadingMode::UvChecker).unwrap(), "\"uv_checker\"");
    }

    #[test]
    fn stamp_source_is_never_serialized() {
        let stamp = StampSettings { source: Some(BlobId(7)), source_name: "logo.png".to_owned(), ..StampSettings::default() };
        let back = round_trip(&stamp);
        assert_eq!(back.source, None);
        assert_eq!(back.source_name, "");
        assert_eq!(back.size_frac, stamp.size_frac);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let tools: ToolSettings = serde_json::from_str(r#"{ "brush": { "size_px": 9.0 } }"#).unwrap();
        assert_eq!(tools.brush.size_px, 9.0);
        assert_eq!(tools.brush.hardness, BrushSettings::default().hardness);
        assert_eq!(tools.fill_scope, ToolSettings::default().fill_scope);
        assert_eq!(tools.text, TextSettings::default());

        let camera: OrbitCamera = serde_json::from_str(r#"{ "yaw": 2.0 }"#).unwrap();
        assert_eq!(camera, OrbitCamera { yaw: 2.0, ..OrbitCamera::default() });

        assert!(serde_json::from_str::<ToolSettings>(r#"{ "fill_scope": "bucket" }"#).is_err());
    }

    #[test]
    fn prefs_json_round_trip_and_defaults() {
        let prefs = Prefs {
            recent: vec![PathBuf::from(r"E:\models\a.wtep"), PathBuf::from(r"E:\models\b.wtep")],
            last_dir: Some(PathBuf::from(r"E:\models")),
            last_export_dir: None,
            window_size: [1600.0, 900.0],
            window_maximized: true,
            flags_wireframe: true,
            keep_backup: true,
            swatches: vec![[1, 2, 3, 255]],
            ..Prefs::default()
        };
        assert_eq!(round_trip(&prefs), prefs);

        assert_eq!(serde_json::from_str::<Prefs>("{}").unwrap(), Prefs::default());
        let partial: Prefs = serde_json::from_str(r#"{ "flags_wireframe": true, "unknown_key": 1 }"#).unwrap();
        assert_eq!(partial, Prefs { flags_wireframe: true, ..Prefs::default() });
        assert_eq!(Prefs::default().window_size, [1280.0, 800.0]);
    }

    #[test]
    fn round_4_prefs_entries_are_read_leniently() {
        let mut close = std::collections::BTreeMap::new();
        close.insert(PathBuf::from(r"E:\shapes\cup.obj"), CloseMode::RoundedCaps);
        let prefs = Prefs {
            brush: BrushSettings { size_px: 33.0, kind: BrushType::Sphere, ..BrushSettings::default() },
            eraser: BrushSettings { shape: ShapeChoice::Cube, ..BrushSettings::default() },
            brush_shape_path: Some(PathBuf::from(r"E:\shapes\cup.obj")),
            brush_shape_close: close.clone(),
            flags_wireframe: true,
            ..Prefs::default()
        };
        assert_eq!(round_trip(&prefs), prefs);

        // Values of a later version: an unknown brush type, a broken angle, an unknown closing mode. Only those are
        // dropped; the rest of the file (and of each entry) is kept.
        let text = r#"{
            "flags_wireframe": true,
            "brush": { "size_px": 33.0, "kind": "cone", "tilt_deg": "steep", "shape": "cube" },
            "eraser": "not an object",
            "brush_shape_close": { "E:\\shapes\\cup.obj": "rounded_caps", "E:\\shapes\\pot.obj": "weld_everything", "E:\\shapes\\x.obj": 4 }
        }"#;
        let p: Prefs = serde_json::from_str(text).unwrap();
        assert!(p.flags_wireframe);
        assert_eq!(p.brush, BrushSettings { size_px: 33.0, shape: ShapeChoice::Cube, ..BrushSettings::default() });
        assert_eq!(p.eraser, BrushSettings::default());
        assert_eq!(p.brush_shape_close, close);
        // Read values are sanitized.
        let p: Prefs = serde_json::from_str(r#"{ "brush": { "size_px": 1e9, "turn_deg": 270.0 } }"#).unwrap();
        assert_eq!((p.brush.size_px, p.brush.turn_deg), (2048.0, -90.0));
        let p: Prefs = serde_json::from_str(r#"{ "brush": null, "brush_shape_close": [1, 2] }"#).unwrap();
        assert_eq!(p, Prefs::default());
    }

    #[test]
    fn brush_settings_are_sanitized_and_round_trip() {
        let d = BrushSettings::default();
        assert_eq!((d.kind, d.shape, d.through, d.align), (BrushType::Circular, ShapeChoice::Ball, false, TipAlign::Surface));
        assert_eq!(d.clone().sanitized(), d);
        let wild = BrushSettings {
            size_px: 1e9,
            hardness: f32::NAN,
            opacity: 0.0,
            flow: -3.0,
            spacing: 5.0,
            turn_deg: 270.0,
            tilt_deg: -190.0,
            roll_deg: f32::INFINITY,
            offset: 7.0,
            kind: BrushType::Sphere,
            shape: ShapeChoice::File,
            ..BrushSettings::default()
        }
        .sanitized();
        assert_eq!((wild.size_px, wild.hardness, wild.opacity, wild.flow, wild.spacing), (2048.0, d.hardness, 0.01, 0.01, 1.0));
        assert_eq!((wild.turn_deg, wild.tilt_deg, wild.roll_deg, wild.offset), (-90.0, 170.0, 0.0, 1.0));
        assert_eq!((wild.kind, wild.shape), (BrushType::Sphere, ShapeChoice::File), "choices are kept");
        assert_eq!(round_trip(&wild), wild);
        assert_eq!(serde_json::to_value(BrushType::Sphere).unwrap(), "sphere");
        assert_eq!(serde_json::to_value(ShapeChoice::Cylinder).unwrap(), "cylinder");
        // A settings file of round 3 (with the removed key) still reads.
        let old: BrushSettings = serde_json::from_str(r#"{ "size_px": 10.0, "same_side_only": false }"#).unwrap();
        assert_eq!(old, BrushSettings { size_px: 10.0, ..BrushSettings::default() });
        assert_eq!(SideTab::ALL.map(SideTab::index), [0, 1, 2]);
        assert_eq!(SideTab::default(), SideTab::Layers);
    }

    // The two tests below need a working core (`FontCatalog::bundled`, `Document::sample`).

    #[test]
    fn app_state_new_applies_prefs() {
        let state = AppState::new(Prefs::default(), true);
        assert!(state.session.is_none() && state.doc().is_none());
        assert!(state.selftest && !state.allow_close && !state.flags.wireframe);
        assert_eq!(state.view_mode, ViewMode::View3D);
        assert_eq!(state.last_edit_mode, ViewMode::View3D);
        assert_eq!(state.split_ratio, 0.5);
        assert_eq!(state.colors.swatches.len(), SWATCH_COUNT);
        assert!(matches!(state.dialog, Dialog::None));
        assert_eq!(state.toasts.error_count(), 0);

        let prefs = Prefs { flags_wireframe: true, swatches: vec![[9, 8, 7, 255]], ..Prefs::default() };
        let mut state = AppState::new(prefs, false);
        assert!(state.flags.wireframe && !state.selftest);
        assert_eq!(state.colors.swatches.len(), SWATCH_COUNT);
        assert_eq!(state.colors.swatches[0], [9, 8, 7, 255]);

        state.push(AppAction::Undo);
        state.push(AppAction::Redo);
        assert_eq!(state.actions.len(), 2);
        assert!(state.doc_mut().is_none());
    }

    #[test]
    fn session_validate_clears_stale_selection_and_clamps_the_set() {
        let doc = Document::sample(SampleKind::Sphere);
        let set_count = doc.sets.len();
        assert!(set_count >= 1);
        let mut session = Session {
            doc,
            path: None,
            force_save_as: false,
            active_set: TextureSetId(0),
            selected_stamp: None,
            camera: OrbitCamera::default(),
            preview_camera: OrbitCamera::default(),
            uv_view: UvViewState::default(),
            scene_generation: 1,
            text_gap: None,
            selected_layers: Vec::new(),
            selected_objects: Default::default(),
            object_anchor: None,
            object_selection_rev: 0,
            object_reveal: None,
        };

        // Nothing to fix.
        session.validate();
        assert_eq!(session.active_set, TextureSetId(0));
        assert_eq!(session.selected_stamp, None);

        // A stamp id the document does not know is cleared; an out-of-range set is clamped to the last one.
        session.selected_stamp = Some(StampId(u64::MAX));
        session.active_set = TextureSetId(99);
        session.validate();
        assert_eq!(session.selected_stamp, None);
        assert_eq!(session.active_set, TextureSetId(set_count as u32 - 1));

        // A stamp the document knows stays selected (`validate` only looks the id up).
        let id = StampId(1_000_000);
        session.doc.stamps.push(StampObject {
            id,
            name: "probe".to_owned(),
            source: StampSource::Image { blob: BlobId(0) },
            placement: Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: glam::Vec2::new(8.0, 8.0),
                size_px: glam::Vec2::new(4.0, 4.0),
                rotation: 0.0,
                flip_x: false,
                flip_y: false,
                clip_to_islands: false,
            }),
            tint: [255; 4],
            home_set: TextureSetId(0),
        });
        session.selected_stamp = Some(id);
        session.validate();
        assert_eq!(session.selected_stamp, Some(id));

        session.doc.stamps.pop();
        session.validate();
        assert_eq!(session.selected_stamp, None);
    }
}
