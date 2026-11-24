//! Commands of the application (ARCHITECTURE.md 8.2): panels, dialogs, shortcuts and the self-test express every
//! command as an `AppAction`; `dispatch` (8.3) is the single place that executes them.
//!
//! Shared contract file [S]: written completely by the wave-0 scaffold and not edited by later packages. Round 4: the
//! scaffold S2 added the brush shape actions (FEATURE_BRUSH 4.2, ROUND4_PLAN section 7 item 5) and `LoadFontSamples`
//! (FEATURE_TEXT 4.2).

use std::path::PathBuf;

use wte_core::{
    BlendMode, CloseMode, Commit, ExportOptions, LayerId, Placement, SampleKind, StampId, StampSource, TextureSetId,
};

use crate::state::{CameraPreset, Dialog, Pending, ShadingMode, ToolKind, ViewMode};

/// One command.
///
/// Convention for every `Option<PathBuf>` below: `Some(path)` = use this path; `None` = ask the user with a native
/// file dialog (done by dispatch, which has the `eframe::Frame`; a `None` action is a no-op while `state.selftest`).
#[derive(Clone, Debug)]
pub enum AppAction {
    // ---- file ----
    NewFromModel(Option<PathBuf>),
    OpenProject(Option<PathBuf>),
    OpenSample(SampleKind),
    /// Opens a path by its extension (project, model or stamp image).
    OpenPath(PathBuf),
    Save,
    SaveAs(Option<PathBuf>),
    ShowExport,
    /// Open the Export dialog for these layers (the Layers choice "Selected", per-layer PNGs only).
    ShowExportLayers(Vec<LayerId>),
    RunExport(ExportOptions),
    ExportPickFolder(Option<PathBuf>),
    /// Extract the original model files; the path is the destination folder.
    ExtractSources(Option<PathBuf>),
    Quit,
    /// The unsaved prompt was answered "Don't Save".
    ConfirmPending(Pending),
    /// The unsaved prompt was answered "Save".
    SaveThen(Pending),

    // ---- import dialog ----
    ImportReload { fallback_uvs: bool },
    ImportPickTexture { set: usize, path: Option<PathBuf> },
    ImportCreate,

    // ---- edit ----
    Undo,
    Redo,
    /// Esc: cancel the interaction / close the dialog / leave Preview (order in 8.3).
    Cancel,
    /// Delete: the selected layers when several are selected (fix_export item 5), else the selected stamp, else the
    /// active layer of the active set.
    DeleteSelection,

    // ---- view ----
    SetViewMode(ViewMode),
    /// Textured <-> Clay.
    ToggleTexture,
    SetShading(ShadingMode),
    ToggleWireframe,
    ToggleSeams,
    ToggleUvWire,
    ToggleAlphaChecker,
    ToggleLit,
    /// 3D: frame the model; 2D: fit the texture.
    FrameModel,
    ResetCamera,
    SetCameraPreset(CameraPreset),

    // ---- tools and colours ----
    SetTool(ToolKind),
    BrushSizeStep(i32),
    BrushHardnessStep(i32),
    SwapColors,
    /// Primary black, secondary white.
    ResetColors,
    SetPrimaryColor([u8; 4]),

    // ---- sets and layers ----
    SetActiveSet(TextureSetId),
    /// A plain click on a layer row: the layer becomes the active layer and the only selected one.
    SelectLayer(LayerId),
    /// Ctrl-click on a layer row: add it to the selection or take it out (the last selected layer stays).
    ToggleLayerSelected(LayerId),
    /// Shift-click on a layer row: select the range from the active layer to this one; the active layer stays.
    SelectLayerRange(LayerId),
    AddPaintLayer,
    DuplicateLayer(LayerId),
    DeleteLayer(LayerId),
    /// fix_export item 5: Duplicate on a multi-selection of the Layers list: every listed layer, one undo step.
    DuplicateLayers(Vec<LayerId>),
    /// fix_export item 5: Delete on a multi-selection of the Layers list: every listed layer, one undo step.
    DeleteLayers(Vec<LayerId>),
    MergeDown(LayerId),
    RasterizeLayer(LayerId),
    MoveLayer { layer: LayerId, to_index: usize },
    SetLayerVisible(LayerId, bool),
    SetLayerLocked(LayerId, bool),
    SetLayerAlphaLock(LayerId, bool),
    SetLayerOpacity(LayerId, f32, Commit),
    SetLayerBlend(LayerId, BlendMode),
    RenameLayer(LayerId, String),
    // The same commands on "the active layer of the active set" (shortcuts and the Layer menu; no-op without one).
    DuplicateActiveLayer,
    MergeActiveDown,
    RasterizeActiveLayer,
    /// +1 = one step up, -1 = one step down.
    MoveActiveLayer(i32),

    // ---- stamps ----
    /// Choose the image the Stamp tool places.
    ImportStampImage(Option<PathBuf>),
    /// "Choose..." of a selected image stamp.
    ReplaceStampImage(StampId, Option<PathBuf>),
    SelectStamp(Option<StampId>),
    UpdateStampSource(StampId, StampSource, Commit),
    UpdateStampPlacement(StampId, Placement, Commit),
    UpdateStampTint(StampId, [u8; 4], Commit),
    DeleteStamp(StampId),

    // ---- objects (objects-tabs; logic in `object_ops.rs`) ----
    /// Plain click: this object only.
    SelectObject(u32),
    /// Ctrl-click (list) / Ctrl- or Shift-click (3D view): add the object to the selection or take it out.
    ToggleObjectSelected(u32),
    /// Shift-click in the list: the range from `Session::object_anchor` to this row.
    #[allow(dead_code, reason = "objects-tabs contract: queued by the Objects tab (WP5)")]
    SelectObjectRange(u32),
    SelectAllObjects,
    DeselectObjects,
    /// The eye of one row: show (`true`) or hide the object.
    #[allow(dead_code, reason = "objects-tabs contract: queued by the Objects tab (WP5)")]
    SetObjectVisible(u32, bool),
    HideSelectedObjects,
    /// Hide every object that is not selected (and show the selected ones).
    IsolateSelectedObjects,
    ShowAllObjects,
    /// Frame the selected visible objects in the 3D view (the visible model when nothing visible is selected).
    FrameSelection,

    // ---- brush shape (round 4; flows in `brush_flows.rs`) ----
    /// The Brush tab's "Load...": load a model file as the brush shape (`None` = ask with the file dialog); a modal
    /// background job. The tool that was active (Brush or Eraser) gets the shape.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    LoadBrushShape(Option<PathBuf>),
    /// Load `prefs.brush_shape_path` again, quietly (non-modal, no messages but a missing-file warning), the first
    /// time it is needed after a start.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    ReloadBrushShape,
    /// "Remove file shape": forget the loaded shape and its remembered path.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    RemoveBrushShape,
    /// The answer of the closing-openings dialog (`Dialog::CloseOpenings`): how to close, or `None` = Cancel.
    AnswerOpenings(Option<CloseMode>),
    /// The Brush tab's "Openings" control: close the openings of the current shape file another way (bakes it again
    /// and remembers the choice for that file).
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    SetBrushShapeClose(CloseMode),

    // ---- text (round 4; flows in `text_ops.rs`) ----
    /// Start the job that draws one sample per font family for the font picker (FEATURE_TEXT 5.13); done once per
    /// catalog, the first time the picker opens.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    LoadFontSamples,

    // ---- ui ----
    ShowDialog(Dialog),
    CloseDialog,
    CancelJob,
    Toast(ToastLevel, String),
}

/// Severity of a toast.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ToastLevel {
    Info,
    Warning,
    Error,
}
