// Owner: app-selftest (A6). The canned self-test scenarios of ARCHITECTURE.md 8.11: every scenario drives the real app
// through `AppAction`s and injected pointer / wheel input, asserts on the live state and ends with a screenshot.
//
// Pointer paths that depend on the camera framing and the panel layout are computed at run time (`Step::Dyn`) from
// `ViewXform::for_view` and `view::gizmo::gizmo_shape`, then converted to normalized view coordinates. Values that later
// asserts compare against (hashes, counts) are remembered in `Scratch` by `Setup` steps. The import scenarios write their
// model and texture files into `<out_stem>_assets\`; export and project files go to `<out_stem>_export\` and
// `<out_stem>.wtep`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use glam::{Vec2, Vec3};
use wte_core::{
    BlendMode, Commit, Document, ExportOptions, FillScope, FlatFormat, Layer, LayerId, LayerKind, PixelBuf, Placement,
    PngLevel, Projector, SampleKind, StampId, StampSource, SurfaceMode, TextureSetId, WrapOptions, WrapStats, samples,
};
use wte_core::CloseMode;

use crate::action::AppAction;
use crate::cli::CliArgs;
use crate::selftest::{Scratch, Step};
use crate::state::{
    AppState, BrushSettings, BrushType, CameraPreset, Dialog, PanelHit, Pending, ShadingMode, ShapeChoice, SideTab,
    TextGapSel, TextPage, ToolKind, ViewId, ViewMode,
};
use crate::tools::{Surface, ViewXform};
use crate::view::gizmo::{self, GizmoShape};

/// Every scenario `scenario` accepts, in the order of the 8.11 table.
const NAMES: &[&str] = &[
    "empty",
    "smoke",
    "default",
    "paint3d",
    "paint2d",
    "stamp",
    "text",
    "clay",
    "preview",
    "export",
    "export_run",
    "multi",
    "roundtrip",
    "undo",
    "import",
    "import_tex",
    "import_nouv",
    "unsaved",
    "open_path",
    "import_save",
    "layers",
    "workflow_path",
    "fuzz_path",
    "stamp_edge_path",
    "pan2d",
    "brush_types",
    "brush_shape",
    "brush_tab",
    "brush_types_path",
    "text_gaps",
    "text_styles",
    "text_warp",
    "text_path",
    "text_tab",
    "text_fonts",
    "text_preview_path",
    "brush_hover",
    "final_bake_async",
    "wrap_surface",
    "export_selected",
    "export_zip",
    "export_flow",
    "export_unsettled",
    "export_nouv",
    "export_errors",
];

/// Colour of the 3D stroke of the default content.
const RED: [u8; 4] = [220, 30, 40, 255];
/// Colour of the 2D stroke of the default content.
const BLUE: [u8; 4] = [30, 90, 230, 255];
/// Display name of the sample stamp image (also the name of its stamp and layer).
const STAMP_NAME: &str = "Logo";
/// Side of the sample stamp image.
const STAMP_SIDE: u32 = 256;
/// Screen distance between two injected pointer positions of a stroke or drag (points).
const PATH_STEP: f32 = 8.0;
/// Pointer positions per frame of strokes and drags.
const POINTS_PER_FRAME: usize = 2;
/// Side and cell count of the textures the import scenarios write.
const ASSET_TEXTURE_SIDE: u32 = 512;

/// The steps of a canned scenario; `None` for an unknown name. `args` gives scenarios their paths
/// (`args.screenshot` => out stem, `args.path`).
pub fn scenario(name: &str, args: &CliArgs) -> Option<Vec<Step>> {
    let mut steps = match name {
        "empty" => vec![Step::WaitIdle, Step::Assert("no session", assert_no_session)],
        "smoke" => vec![
            Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
            Step::WaitIdle,
            Step::Assert("sphere sample: 1 set with Base (locked) and Paint 1 (active)", assert_fresh_sphere),
        ],
        "default" => default_scenario(),
        "paint3d" => paint3d(),
        "paint2d" => paint2d(),
        "stamp" => stamp(),
        "text" => text(),
        "clay" => clay(),
        "preview" => preview(),
        "export" => export(),
        "export_run" => export_run(),
        "multi" => multi(),
        "roundtrip" => roundtrip(),
        "undo" => undo(),
        "import" => import(),
        "import_tex" => import_tex(),
        "import_nouv" => import_nouv(),
        "unsaved" => unsaved(),
        "open_path" => open_path(args.path.is_some()),
        "import_save" => import_save(args.path.is_some()),
        "layers" => layers(),
        "workflow_path" => workflow_path(args.path.is_some()),
        "fuzz_path" => fuzz_path(args),
        "stamp_edge_path" => stamp_edge_path(args.path.is_some()),
        "pan2d" => pan2d(),
        "brush_types" => brush_types(),
        "brush_shape" => brush_shape(),
        "brush_tab" => brush_tab(),
        "brush_types_path" => brush_types_path(args.path.is_some()),
        "text_gaps" => text_gaps(),
        "text_styles" => text_styles(),
        "text_warp" => text_warp(args.path.is_some()),
        "text_path" => text_path(args.path.is_some()),
        "text_tab" => text_tab(),
        "text_fonts" => text_fonts(),
        "text_preview_path" => text_preview_path(args.path.is_some()),
        "brush_hover" => brush_hover(),
        "final_bake_async" => final_bake_async(args.path.is_some()),
        "wrap_surface" => wrap_surface(args.path.clone()),
        "export_selected" => export_selected(),
        "export_zip" => export_zip(),
        "export_flow" => export_flow(args.path.is_some()),
        "export_unsettled" => export_unsettled(args.path.is_some()),
        "export_nouv" => export_nouv(),
        "export_errors" => export_errors(),
        _ => return None,
    };
    steps.push(Step::Screenshot);
    steps.push(Step::Exit);
    Some(steps)
}

/// The names `scenario` accepts.
pub fn scenario_names() -> &'static [&'static str] {
    NAMES
}

// ---------------------------------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------------------------------

/// "Default content" (8.11): Sphere sample, Split view, a red 3D brush stroke, a blue 2D brush stroke, an image stamp
/// and the text stamp "WTE" placed in 3D, then a Transform click on the background that deselects.
fn default_content() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Assert("sphere sample: 1 set with Base (locked) and Paint 1 (active)", assert_fresh_sphere),
        Step::Setup("remember the initial composite, Base and history", remember_initial),
        Step::Do(AppAction::SetViewMode(ViewMode::Split)),
        Step::WaitIdle,
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("red brush stroke across the sphere in the 3D view", |s, _| {
            model_stroke(s, ViewId::Main3D, &[[-0.62, -0.30], [-0.2, -0.38], [0.25, -0.30], [0.62, -0.12]])
        }),
        Step::WaitIdle,
        Step::Assert("the 3D stroke painted", |s, sc| changed_since(s, sc, "initial_composite")),
        Step::Setup("remember the composite after the 3D stroke", |s, sc| remember_composite(s, sc, "after_3d")),
        Step::Do(AppAction::SetPrimaryColor(BLUE)),
        Step::Dyn("blue brush stroke on the texture in the 2D view", |s, _| {
            uv_stroke(s, &[[0.47, 0.56], [0.56, 0.60], [0.66, 0.56], [0.75, 0.60]])
        }),
        Step::WaitIdle,
        Step::Assert("the 2D stroke painted", |s, sc| changed_since(s, sc, "after_3d")),
        Step::Setup("register the sample stamp image", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp on the sphere", |s, _| model_click(s, ViewId::Main3D, [-0.36, 0.12])),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| {
            placed_stamp(s, false).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::Setup("text content \"WTE\"", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            s.tools.text.em_frac = 0.08;
            s.tools.text.color = [20, 20, 20, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Dyn("place the text stamp on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.30, 0.22])),
        Step::WaitIdle,
        Step::Assert("the text stamp was placed and selected", |s, _| {
            placed_stamp(s, true).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::Dyn("Transform: click the background to deselect", |s, _| {
            let xf = view_xf(s, ViewId::Main3D)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, xf.rect.min + egui::vec2(24.0, 24.0)))])
        }),
        Step::WaitIdle,
        Step::Assert("nothing is selected", |s, _| {
            if session_of(s)?.selected_stamp.is_some() { Err("a stamp is still selected".to_owned()) } else { Ok(()) }
        }),
    ]
}

fn default_scenario() -> Vec<Step> {
    let mut steps = default_content();
    steps.push(Step::Assert("default content: 4 layers, Base unchanged, composite changed", assert_default_content));
    steps
}

/// Cube sample, UV checker shading, one stroke across a cube edge and a corner.
fn paint3d() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Cube)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::SetShading(ShadingMode::UvChecker)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        // A larger brush through the "]" shortcut (twice: 32 -> 40 -> 50 texels).
        Step::Key(egui::Modifiers::NONE, egui::Key::CloseBracket),
        Step::Key(egui::Modifiers::NONE, egui::Key::CloseBracket),
        Step::WaitIdle,
        Step::Assert("UV checker shading is on", |s, _| shading_is(s, ShadingMode::UvChecker)),
        Step::Assert("the \"]\" shortcut enlarged the brush", |s, _| {
            let size = s.tools.brush.size_px;
            if size > 32.0 { Ok(()) } else { Err(format!("the brush size is {size}")) }
        }),
        Step::Dyn("stroke across an edge and through the nearest corner of the cube", corner_stroke),
        Step::WaitIdle,
        Step::Assert("pixels changed in at least 2 UV islands", assert_cube_islands),
        Step::Assert("the stroke switched the texture display back on", |s, _| shading_is(s, ShadingMode::Textured)),
    ]
}

/// Sphere, 2D view, a stroke, the eyedropper on that stroke, a second stroke with the picked colour.
fn paint2d() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::Do(AppAction::SetPrimaryColor(BLUE)),
        Step::WaitIdle,
        Step::Dyn("first stroke on the texture", |s, _| uv_stroke(s, &[[0.2, 0.35], [0.5, 0.40], [0.8, 0.35]])),
        Step::WaitIdle,
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Setup("remember the primary colour", |s, sc| {
            sc.values.insert("primary", u32::from_le_bytes(s.colors.primary) as u64);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Eyedropper)),
        Step::Dyn("eyedropper on the first stroke", |s, _| {
            let xf = view_xf(s, ViewId::Main2D)?;
            Ok(vec![click(ViewId::Main2D, norm(&xf, uv_screen(&xf, [0.5, 0.40])))])
        }),
        Step::WaitIdle,
        Step::Assert("the eyedropper picked the colour of the first stroke", |s, sc| {
            let before = remembered(sc, "primary")?;
            let now = s.colors.primary;
            if u32::from_le_bytes(now) as u64 == before {
                return Err(format!("the primary colour is still {now:?}"));
            }
            if color_distance(now, BLUE) > 12 {
                return Err(format!("picked {now:?}, expected the stroke colour {BLUE:?}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Setup("remember the composite", |s, sc| remember_composite(s, sc, "before_second")),
        Step::Dyn("second stroke with the picked colour", |s, _| {
            uv_stroke(s, &[[0.25, 0.62], [0.45, 0.70], [0.62, 0.58], [0.78, 0.66]])
        }),
        Step::WaitIdle,
        Step::Assert("the second stroke painted", |s, sc| changed_since(s, sc, "before_second")),
    ]
}

/// R3-11: pan drags of ten view widths and wheel zooms at the view corners never move the texture out of the 2D view
/// (2D view and the 2D half of Split, also after a window resize); Frame fits it again.
fn pan2d() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitIdle,
        Step::Assert("the fitted texture is in the 2D view", |s, _| texture_in_view(s, "fitted")),
        long_pan([0.5, 0.5], [-9.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("2D: pan drag of ten view widths to the left", |s, _| texture_in_view(s, "2D pan left")),
        long_pan([0.5, 0.5], [10.5, 10.5]),
        Step::WaitIdle,
        Step::Assert("2D: pan drag of ten view widths down-right", |s, _| texture_in_view(s, "2D pan down-right")),
        Step::Scroll { view: ViewId::Main2D, at: [0.99, 0.99], notches: 10.0, mods: egui::Modifiers::NONE },
        Step::Scroll { view: ViewId::Main2D, at: [0.99, 0.99], notches: 10.0, mods: egui::Modifiers::NONE },
        Step::WaitIdle,
        Step::Assert("2D: zoom in at the view corner", |s, _| texture_in_view(s, "2D zoom in")),
        long_pan([0.5, 0.5], [0.5, -9.5]),
        Step::Scroll { view: ViewId::Main2D, at: [0.01, 0.01], notches: -10.0, mods: egui::Modifiers::NONE },
        Step::Scroll { view: ViewId::Main2D, at: [0.01, 0.01], notches: -10.0, mods: egui::Modifiers::NONE },
        Step::WaitIdle,
        Step::Assert("2D: pan up and zoom out at the view corner", |s, _| texture_in_view(s, "2D zoom out")),
        Step::Do(AppAction::FrameModel),
        Step::Do(AppAction::SetViewMode(ViewMode::Split)),
        Step::WaitIdle,
        long_pan([0.5, 0.5], [10.5, -9.5]),
        Step::WaitIdle,
        Step::Assert("Split: pan drag of ten view widths up-right", |s, _| texture_in_view(s, "Split pan up-right")),
        Step::Resize([900, 600]),
        Step::WaitIdle,
        Step::Assert("Split: after the window shrank", |s, _| texture_in_view(s, "Split after resize")),
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Assert("Frame fits the whole texture into the 2D half", |s, _| {
            let (texture, view) = texture_and_view(s)?;
            if !view.expand(0.5).contains_rect(texture) {
                return Err(format!("texture rect {texture:?} not inside the view {view:?} after Frame"));
            }
            Ok(())
        }),
        long_pan([0.5, 0.5], [-9.5, -9.5]),
        Step::WaitIdle,
        Step::Assert("Split: pan drag of ten view widths up-left after Frame", |s, _| texture_in_view(s, "Split pan up-left")),
    ]
}

/// A middle-button pan drag in the 2D view between two normalised view positions (they may lie far outside the view).
fn long_pan(from: [f32; 2], to: [f32; 2]) -> Step {
    let n = 100;
    let path = (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]
        })
        .collect();
    pointer(ViewId::Main2D, path, egui::PointerButton::Middle, egui::Modifiers::NONE, 4)
}

/// The screen rect of the active set's texture in the 2D view, and the view rect.
fn texture_and_view(state: &AppState) -> Result<(egui::Rect, egui::Rect), String> {
    let xf = view_xf(state, ViewId::Main2D)?;
    let tex = Vec2::new(xf.tex_size[0].max(1) as f32, xf.tex_size[1].max(1) as f32);
    Ok((egui::Rect::from_two_pos(xf.px_to_screen(Vec2::ZERO), xf.px_to_screen(tex)), xf.rect))
}

/// R3-11: at least 32 points (or the whole texture / view side when smaller) of the texture stay inside the 2D view on
/// both axes. Logs the overlap.
fn texture_in_view(state: &AppState, label: &str) -> Result<(), String> {
    let (texture, view) = texture_and_view(state)?;
    let overlap = texture.intersect(view);
    let need = |t: f32, v: f32| 32.0f32.min(t).min(v) - 0.5;
    let (w, h) = (overlap.width().max(0.0), overlap.height().max(0.0));
    log::info!("pan2d {label}: texture {texture:?}, view {view:?}, overlap {w:.1} x {h:.1} pt");
    if w < need(texture.width(), view.width()) || h < need(texture.height(), view.height()) {
        return Err(format!("{label}: only {w:.1} x {h:.1} pt of the texture {texture:?} are inside the view {view:?}"));
    }
    Ok(())
}

/// Sphere, an image stamp, gizmo drags of the body, a corner and the rotate knob, then Ctrl+wheel.
fn stamp() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Setup("register the sample stamp image", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp on the sphere", |s, _| model_click(s, ViewId::Main3D, [-0.15, 0.0])),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| {
            placed_stamp(s, false).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::Assert("the 3D stamp's depth is fitted, its box is drawn and its layer says \"3D stamp\"", assert_fitted_3d_stamp),
        Step::Setup("remember the history length", remember_undo),
        Step::Dyn("drag the gizmo body", |s, _| {
            let (xf, shape) = selected_gizmo(s)?;
            Ok(vec![drag(&xf, &[shape.center, shape.center + egui::vec2(60.0, -35.0)])])
        }),
        Step::WaitIdle,
        Step::Assert("one undo entry for the body drag", |s, sc| undo_since(s, sc, 1)),
        Step::Assert("the moved stamp's depth was re-fitted to the surface", assert_fitted_3d_stamp),
        Step::Dyn("drag a corner handle outwards", |s, _| {
            let (xf, shape) = selected_gizmo(s)?;
            let corner = shape.corners[2];
            Ok(vec![drag(&xf, &[corner, shape.center + (corner - shape.center) * 1.45])])
        }),
        Step::WaitIdle,
        Step::Assert("one undo entry for the corner drag", |s, sc| undo_since(s, sc, 2)),
        Step::Dyn("turn the rotate knob by 50 degrees", |s, _| {
            let (xf, shape) = selected_gizmo(s)?;
            let r = shape.rotate_knob - shape.center;
            let a0 = r.y.atan2(r.x);
            let points: Vec<egui::Pos2> = (0..=16)
                .map(|k| {
                    let a = a0 + 50f32.to_radians() * k as f32 / 16.0;
                    shape.center + egui::vec2(a.cos(), a.sin()) * r.length()
                })
                .collect();
            Ok(vec![drag(&xf, &points)])
        }),
        Step::WaitIdle,
        Step::Assert("one undo entry for the knob drag", |s, sc| undo_since(s, sc, 3)),
        Step::Setup("remember the stamp size", |s, sc| {
            let (_, size) = selected_size(s)?;
            sc.values.insert("stamp_size", size.to_bits() as u64);
            Ok(())
        }),
        Step::Dyn("Ctrl+wheel two notches over the stamp", |s, _| {
            let (xf, shape) = selected_gizmo(s)?;
            Ok(vec![Step::Scroll { view: ViewId::Main3D, at: norm(&xf, shape.center), notches: 2.0, mods: egui::Modifiers::CTRL }])
        }),
        Step::WaitIdle,
        Step::Assert("one undo entry for the wheel gesture", |s, sc| undo_since(s, sc, 4)),
        Step::Assert("the wheel changed the placement size", |s, sc| {
            let (_, size) = selected_size(s)?;
            let before = f32::from_bits(remembered(sc, "stamp_size")? as u32);
            let expected = before * 1.1 * 1.1;
            if (size - expected).abs() > 1e-3 * expected {
                return Err(format!("size {size} after two notches, expected {expected} (was {before})"));
            }
            Ok(())
        }),
        Step::Assert("the stamp layer is present", |s, _| {
            let doc = doc_of(s)?;
            let (id, _) = selected_size(s)?;
            if doc.stamp_layers(id).is_empty() || !doc.sets[0].layers.iter().any(|l| matches!(l.kind, LayerKind::Stamp { .. })) {
                return Err("set 0 has no stamp layer".to_owned());
            }
            Ok(())
        }),
        // Fix round 2, item 3: the 3D stamp selected in the 2D view says where it can be edited.
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the 2D view shows the \"edit it in the 3D view\" hint", |s, _| {
            let want = "This is a 3D stamp - edit it in the 3D view";
            if s.status_hint != want {
                return Err(format!("status bar hint {:?}, expected {want:?}", s.status_hint));
            }
            Ok(())
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("back in the 3D view the hint is gone", |s, _| {
            if s.status_hint.starts_with("This is a ") { Err(format!("stale hint {:?}", s.status_hint)) } else { Ok(()) }
        }),
        // Fix round 2, item 9: a burst of undo / redo steps is one toast with a count, not a pile.
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("three undo steps make one toast", |s, _| {
            let labels = s.toasts.visible_labels();
            let undo: Vec<&String> = labels.iter().filter(|l| l.starts_with("Undo: ")).collect();
            if undo.len() != 1 || !undo[0].ends_with("(x3)") || labels.len() > 4 {
                return Err(format!("visible toasts {labels:?}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::Redo),
        Step::Do(AppAction::Redo),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("three redo steps replace it with one toast", |s, _| {
            let labels = s.toasts.visible_labels();
            let history: Vec<&String> = labels.iter().filter(|l| l.starts_with("Undo: ") || l.starts_with("Redo: ")).collect();
            if history.len() != 1 || !history[0].starts_with("Redo: ") || !history[0].ends_with("(x3)") {
                return Err(format!("visible toasts {labels:?}"));
            }
            Ok(())
        }),
        // R3-10 (decision 2): with the Transform options of a selected stamp the whole picker and the Layers footer
        // are on screen.
        Step::WaitFrames(3),
        Step::Assert("the Color section and the Layers footer are inside the side panel", assert_side_panel_layout),
    ]
}

/// The selected stamp is projected, its depth range is the auto-fitted one (6.5), its gizmo carries the projection box
/// (a wrapped stamp, `WTE_STAMP_MODE=wrap`: no box, round 6) and its layer is marked "3D stamp" / "3D text" in the
/// Layers list.
fn assert_fitted_3d_stamp(s: &AppState, _: &Scratch) -> Result<(), String> {
    let session = session_of(s)?;
    let id = session.selected_stamp.ok_or("no stamp is selected")?;
    let doc = &session.doc;
    let stamp = doc.stamp(id).ok_or("the selected stamp does not exist")?;
    let Placement::Projected(p) = stamp.placement else { return Err("the stamp is not projected".to_owned()) };
    let fitted = doc.fit_stamp_depth(&p);
    if (fitted.depth_front - p.depth_front).abs() > 1e-4 * p.depth_front.abs().max(1e-6)
        || (fitted.depth_back - p.depth_back).abs() > 1e-4 * p.depth_back.abs().max(1e-6)
    {
        return Err(format!("depth {} / {}, fitted {} / {}", p.depth_front, p.depth_back, fitted.depth_front, fitted.depth_back));
    }
    log::info!("stamp depth fitted: front {:.4}, back {:.4} (half size {:?})", p.depth_front, p.depth_back, p.half_size);
    let (_, shape) = selected_gizmo_3d(s)?;
    // Check:B (round 6): with WTE_STAMP_MODE=wrap (gate 2, G5a) the stamp is wrapped: its gizmo has no projection box
    // by design (the depth is still fitted, for switching back); in Project the box is required as before.
    if p.mode == SurfaceMode::Wrap {
        if !shape.wrapped || shape.depth_box.is_some() {
            return Err(format!("the wrapped stamp's gizmo: wrapped {}, depth box {:?}", shape.wrapped, shape.depth_box));
        }
    } else if shape.depth_box.is_none() {
        return Err("the gizmo has no projection box".to_owned());
    }
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == stamp.home_set).ok_or("no home layer")?;
    let layer = doc.layer(layer).ok_or("missing layer")?;
    let badge = crate::panels::layers::kind_badge(doc, layer);
    if !badge.starts_with("3D ") {
        return Err(format!("the Layers list calls it {badge:?}"));
    }
    Ok(())
}

/// Remembers the rect of the main 3D view (`view_left` / `view_size`).
fn remember_view_rect(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let rect = view_xf(s, ViewId::Main3D)?.rect;
    sc.values.insert("view_left", pack(Vec2::new(rect.min.x, rect.min.y)));
    sc.values.insert("view_size", pack(Vec2::new(rect.width(), rect.height())));
    Ok(())
}

/// The main 3D view has the rect `remember_view_rect` saw (within half a point).
fn same_view_rect(s: &AppState, sc: &Scratch) -> Result<(), String> {
    let rect = view_xf(s, ViewId::Main3D)?.rect;
    let (left, size) = (unpack(remembered(sc, "view_left")?), unpack(remembered(sc, "view_size")?));
    let now = (Vec2::new(rect.min.x, rect.min.y), Vec2::new(rect.width(), rect.height()));
    if (now.0 - left).abs().max_element() > 0.5 || (now.1 - size).abs().max_element() > 0.5 {
        return Err(format!("the 3D view moved from {left:?} size {size:?} to {:?} size {:?}", now.0, now.1));
    }
    Ok(())
}

/// Sphere, a text stamp, a content change, a 4x scale, and a press with empty content.
fn text() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Setup("text content \"WTE\" at a small size", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            // Small enough that the 4x scaled "WTE 2026" still fits on the front of the sphere.
            s.tools.text.em_frac = 0.02;
            s.tools.text.color = [150, 20, 30, 255];
            Ok(())
        }),
        // Fix round 2, item 4: the Text options (and a selected text stamp's properties) used to widen the side panel,
        // so switching from the Stamp to the Text tool moved the 3D view and a click aimed at the model landed beside it.
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember the 3D view rect with the Stamp tool", remember_view_rect),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the Text tool options leave the 3D view where it was", same_view_rect),
        Step::Dyn("place the text stamp on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.05])),
        Step::WaitIdle,
        Step::Assert("the text stamp was placed and selected", |s, _| placed_stamp(s, true).map(|_| ())),
        Step::WaitFrames(3),
        Step::Assert("the selected text stamp's properties leave the 3D view where it was", same_view_rect),
        Step::Setup("remember the text layer", remember_text_layer),
        Step::Dyn("change the content to \"WTE 2026\"", |s, _| {
            let (id, spec) = text_stamp(s)?;
            let mut spec = spec;
            spec.content = "WTE 2026".to_owned();
            Ok(vec![Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("the content changed and the text layer's cache rect changed", |s, sc| {
            let (_, spec) = text_stamp(s)?;
            if spec.content != "WTE 2026" {
                return Err(format!("the content is {:?}", spec.content));
            }
            let rect = text_layer_rect(s)?;
            if rect_key(&rect) == remembered(sc, "text_rect")? {
                return Err(format!("the text layer's cache rect is still {rect:?}"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Assert("the longer text's depth was re-fitted to its wider footprint", assert_fitted_3d_stamp),
        // Fix round 3, R3-9: a text stamp with its automatic name follows its text (stamp and linked layers).
        Step::Assert("the text stamp and its layer are named \"WTE 2026\"", |s, _| {
            let (id, _) = text_stamp(s)?;
            let doc = doc_of(s)?;
            let mut names: Vec<&str> = vec![doc.stamp(id).map_or("", |st| st.name.as_str())];
            names.extend(doc.stamp_layers(id).into_iter().filter_map(|(_, l)| doc.layer(l)).map(|l| l.name.as_str()));
            if names.len() < 2 || names.iter().any(|n| *n != "WTE 2026") {
                return Err(format!("stamp and layer names {names:?}"));
            }
            Ok(())
        }),
        Step::Setup("remember the text raster size", |s, sc| {
            let (_, spec) = text_stamp(s)?;
            sc.values.insert("size_px", spec.size_px.to_bits() as u64);
            Ok(())
        }),
        Step::Dyn("scale the text 4x", |s, _| {
            let (id, _) = text_stamp(s)?;
            let doc = doc_of(s)?;
            let placement = doc.stamp(id).ok_or("the text stamp is gone")?.placement;
            // Like the Size field and the Transform tool: an automatic depth follows the larger footprint (6.5).
            let placement = match scaled(&placement, 4.0) {
                Placement::Projected(p) => Placement::Projected(doc.fit_stamp_depth(&p)),
                uv => uv,
            };
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, placement, Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("TextSpec.size_px grew with the 4x scale", |s, sc| {
            let (_, spec) = text_stamp(s)?;
            let before = f32::from_bits(remembered(sc, "size_px")? as u32);
            if spec.size_px <= before {
                return Err(format!("size_px is {} after the 4x scale (was {before})", spec.size_px));
            }
            undo_since(s, sc, 2)
        }),
        Step::Setup("empty text content", |s, sc| {
            s.tools.text.content.clear();
            remember_undo(s, sc)?;
            sc.values.insert("stamps", doc_of(s)?.stamps.len() as u64);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Dyn("press the Text tool on the sphere with empty content", |s, _| model_click(s, ViewId::Main3D, [0.35, -0.45])),
        Step::WaitIdle,
        Step::Assert("the empty-content press added no stamp and no undo entry", |s, sc| {
            let stamps = doc_of(s)?.stamps.len() as u64;
            if stamps != remembered(sc, "stamps")? {
                return Err(format!("{stamps} stamps after the empty press"));
            }
            undo_since(s, sc, 0)
        }),
        // Round 4 (FEATURE_TEXT 6.2, replaces the R3-10 rule): the Text tool's box is always the text to place; the
        // empty press asked for it, and the placed text stays selected without being shown in it.
        Step::WaitFrames(3),
        Step::Assert("the empty press asked for the text to place: its box is shown, not the selected text", |s, _| {
            if s.panel_rects.contains_key(&PanelHit::StampTextField) {
                return Err("the selected-text field is drawn under the Text tool".to_owned());
            }
            Ok(())
        }),
        // The selected text is edited with the Transform tool: its Text block holds the text, ready to edit.
        Step::Dyn("select the text stamp and take the Transform tool", |s, _| {
            let (id, _) = text_stamp(s)?;
            Ok(vec![Step::Do(AppAction::SelectStamp(Some(id))), Step::Do(AppAction::SetTool(ToolKind::Transform))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the Transform options show the selected text ready to edit", |s, _| {
            tool_is(s, ToolKind::Transform)?;
            let (id, spec) = text_stamp(s)?;
            if session_of(s)?.selected_stamp != Some(id) {
                return Err("the text stamp is not selected".to_owned());
            }
            let field = panel_rect(s, PanelHit::StampTextField)?;
            let area = panel_rect(s, PanelHit::SettingsArea)?;
            if !area.expand(0.5).contains_rect(field) {
                return Err(format!("the text field {field:?} is not inside the tool options area {area:?}"));
            }
            log::info!("selected text {:?} in the Transform options field {field:?}", spec.content);
            Ok(())
        }),
        Step::Assert("the Color section and the Layers footer are inside the side panel", assert_side_panel_layout),
    ]
}

/// Sphere, a stroke, then the texture display switched off.
fn clay() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Setup("remember the composite", |s, sc| remember_composite(s, sc, "initial_composite")),
        Step::Dyn("brush stroke across the sphere", |s, _| {
            model_stroke(s, ViewId::Main3D, &[[-0.6, 0.1], [-0.1, -0.2], [0.55, 0.05]])
        }),
        Step::WaitIdle,
        Step::Assert("the stroke painted", |s, sc| changed_since(s, sc, "initial_composite")),
        Step::Do(AppAction::ToggleTexture),
        Step::WaitIdle,
        Step::Assert("clay shading", |s, _| shading_is(s, ShadingMode::Clay)),
    ]
}

/// Default content in the Preview mode.
fn preview() -> Vec<Step> {
    let mut steps = default_content();
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::Preview)),
        Step::WaitIdle,
        Step::Assert("Preview mode without a pending edit", |s, _| {
            if s.view_mode != ViewMode::Preview {
                return Err(format!("the view mode is {:?}", s.view_mode));
            }
            if doc_of(s)?.has_pending_edit() {
                return Err("an edit is still pending".to_owned());
            }
            Ok(())
        }),
    ]);
    steps
}

/// Default content and the Export dialog.
fn export() -> Vec<Step> {
    let mut steps = default_content();
    steps.extend([
        // Prefs are not loaded in self-test mode: remember an earlier export folder like a user who exported before.
        Step::Setup("an earlier export folder in the prefs", |s, sc| {
            s.prefs.last_export_dir = Some(with_suffix(&sc.out_stem, "_export"));
            Ok(())
        }),
        Step::Do(AppAction::ShowExport),
        Step::WaitIdle,
        Step::Assert("the Export dialog is open with a planned file list", |s, _| {
            let Dialog::Export(st) = &s.dialog else { return Err("the Export dialog is not open".to_owned()) };
            if !st.options.dir.to_string_lossy().ends_with("export") {
                return Err(format!("the export folder {} does not end in \"export\"", st.options.dir.display()));
            }
            let plan = wte_core::export::plan(&doc_of(s)?.snapshot(), &st.options);
            if plan.is_empty() {
                return Err("export::plan is empty".to_owned());
            }
            Ok(())
        }),
    ]);
    steps
}

/// Default content exported as flattened PNG, per-layer PNGs, ORA and PSD.
fn export_run() -> Vec<Step> {
    let mut steps = default_content();
    // Round 4 (FEATURE_TEXT 8.3): the text is exported with gap values and a warp.
    steps.extend(t7_style_default_text());
    steps.extend([
        Step::Setup("start with an empty export folder", |_, sc| {
            let dir = with_suffix(&sc.out_stem, "_export");
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
            }
            Ok(())
        }),
        Step::Dyn("export everything", |s, sc| Ok(vec![Step::Do(AppAction::RunExport(export_options(s, sc)?))])),
        Step::WaitIdle,
        Step::Assert("every planned file exists, stamp and text are separate layer files", assert_exported),
    ]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// Export: layer selection, the zip package and the end-to-end export flow (EXPORT_SELECT_PLAN 6.3, 7)
// ---------------------------------------------------------------------------------------------------------------------

/// Sorted names of everything in a folder.
fn dir_names(dir: &Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Ok(names)
}

/// `<out_stem><suffix>`, removed if it exists (the export creates it).
fn fresh_out_dir(scratch: &Scratch, suffix: &str) -> Result<PathBuf, String> {
    let dir = with_suffix(&scratch.out_stem, suffix);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
    }
    Ok(dir)
}

/// A decoded image file.
fn decode_named(name: &str, bytes: &[u8]) -> Result<PixelBuf, String> {
    wte_core::image_io::decode_image(bytes, Some(name), 16384).map(|d| d.pixels).map_err(|e| format!("{name}: {e}"))
}

/// A layer's pixels on the whole canvas: what its per-layer PNG must hold.
fn layer_canvas(layer: &Layer, width: u32, height: u32) -> PixelBuf {
    match &layer.kind {
        LayerKind::Raster(buf) if buf.width == width && buf.height == height => (**buf).clone(),
        LayerKind::Raster(buf) => wte_core::CroppedBuf { rect: buf.rect(), data: buf.data.clone() }.to_dense(width, height),
        LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.to_dense(width, height),
    }
}

/// The layers of `set` an export with these options writes (per-layer PNGs, ORA, PSD), bottom to top.
fn exported_layers_of<'a>(set: &'a wte_core::TextureSet, opts: &ExportOptions) -> Vec<&'a Layer> {
    set.layers
        .iter()
        .filter(|l| match &opts.layers {
            None => opts.include_hidden || l.visible,
            Some(ids) => ids.contains(&l.id),
        })
        .collect()
}

/// The flattened picture of `set` such an export writes: the composite the app shows, or under a layer selection
/// the selected visible layers flattened on their own.
fn expected_flat(set: &wte_core::TextureSet, opts: &ExportOptions) -> PixelBuf {
    match &opts.layers {
        None => set.composite().clone(),
        Some(_) => {
            let selected: Vec<Layer> = exported_layers_of(set, opts).into_iter().cloned().collect();
            wte_core::composite::flatten_layers(&selected, set.width, set.height, false)
        }
    }
}

/// Number of texels that differ between two images of the same size (`u64::MAX` for different sizes).
fn differing(a: &PixelBuf, b: &PixelBuf) -> u64 {
    if (a.width, a.height) != (b.width, b.height) {
        return u64::MAX;
    }
    a.data.as_chunks::<4>().0.iter().zip(b.data.as_chunks::<4>().0).filter(|(x, y)| x != y).count() as u64
}

/// Checks exported files (read from a folder or from a zip, in plan order) against the document: exactly the files
/// `plan_contents` names; every per-layer PNG holds its layer's pixels; the flattened texture is the composite (under a
/// layer selection: the selected visible layers); the ORA's merged image is the same picture and its stack lists the
/// exported layers; the PSD parses and holds the exported layers by name; and when every layer of a set is exported
/// without a selection, the per-layer PNGs put back together with the document's order, visibility, opacity and blend
/// give exactly the composite. Returns a line for the log.
fn verify_export(doc: &Document, opts: &ExportOptions, files: &[(String, Vec<u8>)]) -> Result<String, String> {
    if opts.dilate_px != 0 {
        return Err("verify_export expects no seam padding".to_owned());
    }
    let contents = wte_core::export::plan_contents(&doc.snapshot(), opts);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let planned: Vec<&str> = contents.iter().map(|f| f.name.as_str()).collect();
    if names != planned {
        return Err(format!("the exported files are {names:?}, the plan is {planned:?}"));
    }
    let (mut layers, mut flats, mut layered) = (0, 0, 0);
    let mut decoded: Vec<Option<PixelBuf>> = vec![None; files.len()];
    for (k, (file, (name, bytes))) in contents.iter().zip(files).enumerate() {
        let set = doc.set(file.set).ok_or_else(|| format!("{name}: no texture set {:?}", file.set))?;
        let (w, h) = (set.width, set.height);
        let exported = exported_layers_of(set, opts);
        match file.kind {
            wte_core::PlannedKind::Flat => {
                let image = decode_named(name, bytes)?;
                if matches!(opts.flat, Some(FlatFormat::Jpeg { .. })) {
                    if (image.width, image.height) != (w, h) {
                        return Err(format!("{name}: {}x{}, the set is {w}x{h}", image.width, image.height));
                    }
                } else {
                    let n = differing(&image, &expected_flat(set, opts));
                    if n != 0 {
                        return Err(format!("{name}: {n} texels differ from the flattened layers the app shows"));
                    }
                }
                flats += 1;
            }
            wte_core::PlannedKind::Layer { layer, .. } => {
                let l = doc.layer(layer).ok_or_else(|| format!("{name}: no layer {layer:?}"))?;
                let image = decode_named(name, bytes)?;
                let n = differing(&image, &layer_canvas(l, w, h));
                if n != 0 {
                    return Err(format!("{name}: {n} texels differ from the pixels of layer {:?}", l.name));
                }
                decoded[k] = Some(image);
                layers += 1;
            }
            wte_core::PlannedKind::Ora => {
                let entries = wte_core::export::read_package(bytes).map_err(|e| format!("{name}: {e}"))?;
                let entry = |n: &str| entries.iter().find(|(e, _)| e == n).map(|(_, b)| b.as_slice()).ok_or(format!("{name}: no {n}"));
                let merged = decode_named("mergedimage.png", entry("mergedimage.png")?)?;
                let n = differing(&merged, &expected_flat(set, opts));
                if n != 0 {
                    return Err(format!("{name}: {n} texels of the merged image differ from the flattened layers"));
                }
                let stack = String::from_utf8_lossy(entry("stack.xml")?).into_owned();
                let count = stack.matches("<layer ").count();
                if count != exported.len() {
                    return Err(format!("{name}: {count} layers in stack.xml, {} exported", exported.len()));
                }
                layered += 1;
            }
            wte_core::PlannedKind::Psd => {
                let psd = wte_core::export::read_psd(bytes).map_err(|e| format!("{name}: {e}"))?;
                let got: Vec<&str> = psd.layers.iter().map(|l| l.unicode_name.as_str()).collect();
                let want: Vec<&str> = exported.iter().map(|l| l.name.as_str()).collect();
                if got != want || (psd.width, psd.height) != (w, h) {
                    return Err(format!("{name}: {}x{} with layers {got:?}, expected {w}x{h} with {want:?}", psd.width, psd.height));
                }
                layered += 1;
            }
        }
    }
    // The per-layer files of a set add up to what the app shows.
    let mut recomposed = 0;
    if opts.layers.is_none() && opts.per_layer_png {
        for set in &doc.sets {
            let mut stack = Vec::new();
            for (file, image) in contents.iter().zip(&decoded) {
                let (wte_core::PlannedKind::Layer { layer, .. }, Some(image)) = (file.kind, image) else { continue };
                if file.set != set.id {
                    continue;
                }
                let l = doc.layer(layer).ok_or("a planned layer is gone")?;
                let mut copy = Layer::new(l.id, &l.name, LayerKind::Raster(Arc::new(image.clone())));
                copy.visible = l.visible;
                copy.opacity = l.opacity;
                copy.blend = l.blend;
                stack.push(copy);
            }
            if stack.is_empty() {
                continue;
            }
            let back = wte_core::composite::flatten_layers(&stack, set.width, set.height, false);
            let n = differing(&back, set.composite());
            if n != 0 {
                return Err(format!("set {:?}: the per-layer PNGs put back together differ from the composite in {n} texels", set.name));
            }
            recomposed += 1;
        }
    }
    Ok(format!(
        "{} files verified: {flats} flattened, {layers} per-layer, {layered} layered, {recomposed} set(s) recomposed exactly",
        files.len()
    ))
}

/// The files of an export folder in plan order, after checking that the folder holds exactly those.
fn read_export_dir(doc: &Document, opts: &ExportOptions) -> Result<Vec<(String, Vec<u8>)>, String> {
    let contents = wte_core::export::plan_contents(&doc.snapshot(), opts);
    let mut planned: Vec<String> = contents.iter().map(|f| f.name.clone()).collect();
    planned.sort();
    let listing = dir_names(&opts.dir)?;
    if listing != planned {
        return Err(format!("{} holds {listing:?}, the plan is {planned:?}", opts.dir.display()));
    }
    contents
        .iter()
        .map(|f| std::fs::read(opts.dir.join(&f.name)).map(|b| (f.name.clone(), b)).map_err(|e| format!("{}: {e}", f.name)))
        .collect()
}

/// A layer of the default content by role: "paint" = Paint 1, "stamp" = the image stamp's layer, "text" = the text
/// stamp's layer (in the first set).
fn content_layer(state: &AppState, role: &str) -> Result<LayerId, String> {
    let set = doc_of(state)?.sets.first().ok_or("no texture set")?;
    set.layers
        .iter()
        .find(|l| match role {
            "paint" => l.name == "Paint 1",
            "stamp" => matches!(l.kind, LayerKind::Stamp { .. }),
            _ => matches!(l.kind, LayerKind::Text { .. }),
        })
        .map(|l| l.id)
        .ok_or_else(|| format!("no {role} layer"))
}

/// A click on the name of a Layers row (any button and modifiers), at the position the panel recorded.
fn name_click(state: &AppState, role: &str, button: egui::PointerButton, mods: egui::Modifiers) -> Result<Vec<Step>, String> {
    let id = content_layer(state, role)?;
    let rect = state.panel_rects.get(&PanelHit::LayerName(id)).ok_or_else(|| format!("the {role} row's name was not drawn"))?;
    Ok(vec![Step::ScreenClick { at: rect.center(), button, mods }])
}

/// A click on an item of the open layer row menu.
fn menu_click(state: &AppState, hit: PanelHit) -> Result<Vec<Step>, String> {
    let rect = state.panel_rects.get(&hit).ok_or_else(|| format!("the row menu item {hit:?} is not shown"))?;
    Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
}

/// The selection is exactly these roles (bottom to top), `active` is the active layer, and neither the history nor
/// the document revision changed since `remember_selection_base`.
fn expect_selection(state: &AppState, sc: &Scratch, roles: &[&str], active: &str) -> Result<(), String> {
    let session = session_of(state)?;
    let want: Vec<LayerId> = roles.iter().map(|r| content_layer(state, r)).collect::<Result<_, _>>()?;
    let got = session.layer_selection();
    if got != want {
        return Err(format!("the selection is {got:?}, expected {roles:?} = {want:?}"));
    }
    let active_id = content_layer(state, active)?;
    if session.doc.sets[0].active != Some(active_id) {
        return Err(format!("the active layer is {:?}, expected {active}", session.doc.sets[0].active));
    }
    undo_since(state, sc, 0)?;
    if session.doc.revision() != remembered(sc, "sel_rev")? {
        return Err("selecting layers changed the document revision".to_owned());
    }
    Ok(())
}

fn remember_selection_base(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    remember_undo(state, sc)?;
    let rev = doc_of(state)?.revision();
    sc.values.insert("sel_rev", rev);
    Ok(())
}

/// Points the open Export dialog at `<out_stem><suffix>` (emptied).
fn dialog_into(state: &mut AppState, sc: &Scratch, suffix: &str) -> Result<(), String> {
    dialog_into_dir(state, fresh_out_dir(sc, suffix)?)
}

/// Points the open Export dialog at `dir` (emptied).
fn dialog_into_dir(state: &mut AppState, dir: PathBuf) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
    }
    let Dialog::Export(st) = &mut state.dialog else { return Err(format!("the Export dialog is not open ({})", dialog_name(&state.dialog))) };
    st.options.dir = dir;
    Ok(())
}

/// The action the dialog's Export button pushes, with the dialog's options.
fn run_dialog_export(state: &AppState, _sc: &Scratch) -> Result<Vec<Step>, String> {
    let Dialog::Export(st) = &state.dialog else { return Err(format!("the Export dialog is not open ({})", dialog_name(&state.dialog))) };
    log::info!("export with the dialog's options: {:?}", st.options);
    Ok(vec![Step::Do(AppAction::RunExport(st.options.clone()))])
}

/// The per-layer file name of a layer exported under a layer selection (no set filter): with the set name when the
/// model has more than one texture set, the same name the full export gives it.
fn selection_layer_name(doc: &Document, layer: LayerId) -> Result<String, String> {
    let (set, index) = doc.find_layer(layer).ok_or("no such layer")?;
    let set = doc.set(set).ok_or("no such set")?;
    let l = &set.layers[index];
    Ok(wte_core::export::layer_name(&doc.name, &set.name, doc.sets.len() > 1, index, l.kind_name(), &l.name))
}

/// The per-layer files of a layer selection, as the export names them (`export::plan_contents`): every layer of the
/// selection completed by the linked layers of its designs in other sets (fix_export item 3), with its file name.
fn selection_files(doc: &Document, layers: &[LayerId]) -> Vec<(LayerId, String)> {
    let snapshot = doc.snapshot();
    let opts = ExportOptions {
        dir: PathBuf::new(),
        base_name: doc.name.clone(),
        sets: None,
        flat: None,
        per_layer_png: true,
        ora: false,
        psd: false,
        include_hidden: false,
        dilate_px: 0,
        layers: Some(layers.to_vec()),
        package: wte_core::ExportPackage::Files,
    };
    wte_core::export::plan_contents(&snapshot, &opts)
        .into_iter()
        .filter_map(|f| match f.kind {
            wte_core::PlannedKind::Layer { layer, .. } => Some((layer, f.name)),
            _ => None,
        })
        .collect()
}

/// The folder holds exactly the per-layer PNGs of these layers and of the linked layers of their designs in other
/// sets (fix_export item 3), each with its layer's pixels; returns the images of `layers`. Without a collision of set
/// names, a file's name is `selection_layer_name` (the full export's name).
fn expect_layer_files(state: &AppState, dir: &Path, layers: &[LayerId]) -> Result<Vec<PixelBuf>, String> {
    let doc = doc_of(state)?;
    let files = selection_files(doc, layers);
    let mut want: Vec<String> = files.iter().map(|(_, name)| name.clone()).collect();
    want.sort();
    let listing = dir_names(dir)?;
    if listing != want {
        return Err(format!("{} holds {listing:?}, expected exactly {want:?}", dir.display()));
    }
    // Without colliding set names (no `_2`), every name is the plain pattern the full export gives that layer.
    let stems: Vec<String> = doc.sets.iter().map(|s| wte_core::export::sanitize_file_stem(&s.name).to_lowercase()).collect();
    if !(1..stems.len()).any(|i| stems[..i].contains(&stems[i])) {
        for (id, name) in &files {
            let plain = selection_layer_name(doc, *id)?;
            if *name != plain {
                return Err(format!("the file of {id:?} is {name}, expected {plain}"));
            }
        }
    }
    if files.len() > layers.len() {
        log::info!("export: the selection of {} layer(s) brought {} linked layer(s) of other sets", layers.len(), files.len() - layers.len());
    }
    let mut images = Vec::new();
    for id in layers {
        if !files.iter().any(|(l, _)| l == id) {
            return Err(format!("no file for the selected layer {id:?}"));
        }
    }
    for (id, name) in &files {
        let image = decode_named(name, &std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?)?;
        let (set, _) = doc.find_layer(*id).ok_or("no such layer")?;
        let set = doc.set(set).ok_or("no such set")?;
        let layer = doc.layer(*id).ok_or("no such layer")?;
        let n = differing(&image, &layer_canvas(layer, set.width, set.height));
        if n != 0 {
            return Err(format!("{name}: {n} texels differ from the layer {:?}", layer.name));
        }
        if layers.contains(id) {
            images.push((layers.iter().position(|l| l == id).unwrap_or(0), image));
        }
    }
    images.sort_by_key(|(i, _)| *i);
    Ok(images.into_iter().map(|(_, image)| image).collect())
}

/// fix_export item 8: no toast says "1 files".
fn toasts_count_right(state: &AppState) -> Result<(), String> {
    let labels = state.toasts.visible_labels();
    if labels.iter().any(|l| l.contains(" 1 files") || l.contains("(1 files)")) {
        return Err(format!("a toast says \"1 files\": {labels:?}"));
    }
    Ok(())
}

/// The bytes of every file of a folder, by name.
fn dir_bytes(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    dir_names(dir)?
        .into_iter()
        .map(|n| {
            let bytes = std::fs::read(dir.join(&n)).map_err(|e| format!("{n}: {e}"))?;
            Ok((n, bytes))
        })
        .collect()
}

/// Two folders hold the same files, byte for byte.
fn same_files(a: &Path, b: &Path) -> Result<usize, String> {
    let (fa, fb) = (dir_bytes(a)?, dir_bytes(b)?);
    let names = |f: &[(String, Vec<u8>)]| f.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    if names(&fa) != names(&fb) {
        return Err(format!("{} holds {:?}, {} holds {:?}", a.display(), names(&fa), b.display(), names(&fb)));
    }
    for ((name, x), (_, y)) in fa.iter().zip(&fb) {
        if x != y {
            return Err(format!("{name} differs between {} and {}", a.display(), b.display()));
        }
    }
    Ok(fa.len())
}

/// Texels of an image that are exactly `color`.
fn count_color(image: &PixelBuf, color: [u8; 4]) -> usize {
    image.data.as_chunks::<4>().0.iter().filter(|p| **p == color).count()
}

/// Texels with any alpha.
fn ink(image: &PixelBuf) -> usize {
    image.data.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
}

/// `export_selected` (EXPORT_SELECT_PLAN 6.3): the default content with the text layer hidden; real clicks on the
/// Layers rows (plain, Ctrl, Shift) select layers without touching the document; the row's right-click menu opens the
/// Export dialog for the selection, which writes exactly the selected layers (the hidden one included); "Export this
/// layer..." writes one; a selection with the flattened texture flattens the selected visible layers only.
fn export_selected() -> Vec<Step> {
    use egui::Modifiers as M;
    use egui::PointerButton as B;
    let mut steps = default_content();
    steps.extend([
        Step::Dyn("hide the text layer", |s, _| Ok(vec![Step::Do(AppAction::SetLayerVisible(content_layer(s, "text")?, false))])),
        Step::WaitIdle,
        Step::Setup("remember the history and the revision", remember_selection_base),
        Step::Dyn("click Logo's name", |s, _| name_click(s, "stamp", B::Primary, M::NONE)),
        Step::WaitIdle,
        Step::Assert("Logo is active and the only selected layer", |s, sc| expect_selection(s, sc, &["stamp"], "stamp")),
        Step::Dyn("Ctrl-click the text row", |s, _| name_click(s, "text", B::Primary, M::COMMAND)),
        Step::WaitIdle,
        Step::Assert("Logo and WTE are selected, Logo is still active", |s, sc| expect_selection(s, sc, &["stamp", "text"], "stamp")),
        Step::Dyn("Shift-click Paint 1", |s, _| name_click(s, "paint", B::Primary, M::SHIFT)),
        Step::WaitIdle,
        Step::Assert("Shift selected Paint 1 to Logo", |s, sc| expect_selection(s, sc, &["paint", "stamp"], "stamp")),
        Step::Dyn("Ctrl-click the text row", |s, _| name_click(s, "text", B::Primary, M::COMMAND)),
        Step::WaitIdle,
        Step::Assert("three selected", |s, sc| expect_selection(s, sc, &["paint", "stamp", "text"], "stamp")),
        Step::Dyn("Ctrl-click Paint 1", |s, _| name_click(s, "paint", B::Primary, M::COMMAND)),
        Step::WaitIdle,
        Step::Assert("Logo and WTE selected again", |s, sc| expect_selection(s, sc, &["stamp", "text"], "stamp")),
        Step::Dyn("right-click Logo's name", |s, _| name_click(s, "stamp", B::Secondary, M::NONE)),
        Step::WaitFrames(3),
        Step::Assert("the row menu offers the selection; the right-click selected nothing", |s, sc| {
            if !s.panel_rects.contains_key(&PanelHit::LayerMenuExportSelected) || !s.panel_rects.contains_key(&PanelHit::LayerMenuExportThis) {
                return Err("the row menu does not show both export items".to_owned());
            }
            expect_selection(s, sc, &["stamp", "text"], "stamp")
        }),
        Step::Dyn("click \"Export selected layers... (2)\"", |s, _| menu_click(s, PanelHit::LayerMenuExportSelected)),
        Step::WaitIdle,
        Step::Assert("the Export dialog is open for Logo and WTE, per-layer PNGs only", |s, _| {
            let Dialog::Export(st) = &s.dialog else { return Err(format!("the Export dialog is not open ({})", dialog_name(&s.dialog))) };
            let want = vec![content_layer(s, "stamp")?, content_layer(s, "text")?];
            if st.options.layers.as_ref() != Some(&want) || st.selected_layers != want {
                return Err(format!("layers {:?}, captured selection {:?}, expected {want:?}", st.options.layers, st.selected_layers));
            }
            if st.options.flat.is_some() || !st.options.per_layer_png || st.options.ora || st.options.psd {
                return Err(format!("the dialog's kinds are {:?}", st.options));
            }
            let contents = wte_core::export::plan_contents(&doc_of(s)?.snapshot(), &st.options);
            let kinds: Vec<_> = contents.iter().map(|f| f.kind).collect();
            if !(kinds.len() == 2 && kinds.iter().all(|k| matches!(k, wte_core::PlannedKind::Layer { raster: false, .. }))) {
                return Err(format!("the dialog plans {contents:?}"));
            }
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Shot("dialog"),
        Step::Setup("export into <out>_export_sel", |s, sc| dialog_into(s, sc, "_export_sel")),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("exactly the Logo and WTE PNGs, each its layer's pixels; nothing else changed", |s, sc| {
            assert_no_dialog(s, sc)?;
            let (stamp, text) = (content_layer(s, "stamp")?, content_layer(s, "text")?);
            let images = expect_layer_files(s, &with_suffix(&sc.out_stem, "_export_sel"), &[stamp, text])?;
            if ink(&images[0]) == 0 || ink(&images[1]) == 0 {
                return Err("an exported stamp or text file is empty".to_owned());
            }
            if count_color(&images[0], RED) + count_color(&images[0], BLUE) > 0 {
                return Err("the stamp file holds the stroke colours".to_owned());
            }
            undo_since(s, sc, 0)
        }),
        Step::Dyn("right-click Paint 1's name", |s, _| name_click(s, "paint", B::Secondary, M::NONE)),
        Step::WaitFrames(3),
        Step::Assert("a row outside the selection offers only itself", |s, _| {
            if s.panel_rects.contains_key(&PanelHit::LayerMenuExportSelected) {
                return Err("the menu of a row outside the selection offers the selection".to_owned());
            }
            Ok(())
        }),
        Step::Dyn("click \"Export this layer...\"", |s, _| menu_click(s, PanelHit::LayerMenuExportThis)),
        Step::WaitIdle,
        Step::Setup("export into <out>_export_one", |s, sc| dialog_into(s, sc, "_export_one")),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("exactly the Paint 1 PNG with both strokes", |s, sc| {
            assert_no_dialog(s, sc)?;
            let paint = content_layer(s, "paint")?;
            let images = expect_layer_files(s, &with_suffix(&sc.out_stem, "_export_one"), &[paint])?;
            let (red, blue) = (count_color(&images[0], RED), count_color(&images[0], BLUE));
            if red == 0 || blue == 0 {
                return Err(format!("the Paint 1 file has {red} red and {blue} blue texels"));
            }
            log::info!("export_selected: Paint 1 file with {red} red and {blue} blue texels");
            undo_since(s, sc, 0)
        }),
        Step::Dyn("export Paint 1 and Logo with a flattened PNG", |s, sc| {
            let dir = fresh_out_dir(sc, "_export_selflat")?;
            let opts = ExportOptions {
                dir,
                flat: Some(FlatFormat::Png(PngLevel::Balanced)),
                per_layer_png: false,
                ora: false,
                psd: false,
                layers: Some(vec![content_layer(s, "paint")?, content_layer(s, "stamp")?]),
                ..export_options(s, sc)?
            };
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::WaitIdle,
        Step::Assert("the flattened PNG is Paint 1 and Logo flattened, transparent where only the Base shows", |s, sc| {
            let doc = doc_of(s)?;
            let dir = with_suffix(&sc.out_stem, "_export_selflat");
            let name = wte_core::export::flat_name(&doc.name, &doc.sets[0].name, false, "png");
            if dir_names(&dir)? != [name.clone()] {
                return Err(format!("{} holds {:?}", dir.display(), dir_names(&dir)?));
            }
            let image = decode_named(&name, &std::fs::read(dir.join(&name)).map_err(|e| e.to_string())?)?;
            let set = &doc.sets[0];
            let selected: Vec<Layer> = [content_layer(s, "paint")?, content_layer(s, "stamp")?]
                .iter()
                .filter_map(|id| doc.layer(*id).cloned())
                .collect();
            let want = wte_core::composite::flatten_layers(&selected, set.width, set.height, false);
            let n = differing(&image, &want);
            if n != 0 {
                return Err(format!("{n} texels differ from Paint 1 + Logo flattened"));
            }
            let clear = image.data.as_chunks::<4>().0.iter().zip(set.composite().data.as_chunks::<4>().0).filter(|(a, c)| a[3] == 0 && c[3] == 255).count();
            if clear == 0 {
                return Err("the flattened selection is opaque where only the Base shows".to_owned());
            }
            log::info!("export_selected: flattened selection transparent in {clear} texels the Base covers");
            undo_since(s, sc, 0)
        }),
        Step::WaitFrames(2),
    ]);
    steps
}

/// `export_options` into `<out_stem><suffix>` (emptied) with this package.
fn options_into(state: &AppState, sc: &Scratch, suffix: &str, package: wte_core::ExportPackage) -> Result<ExportOptions, String> {
    Ok(ExportOptions { dir: fresh_out_dir(sc, suffix)?, package, ..export_options(state, sc)? })
}

/// `export_zip` (EXPORT_SELECT_PLAN 6.3): the `export_run` content exported loose and twice as a zip: each zip folder
/// holds only the zip, its entries are the loose files byte for byte in plan order, the two zips are identical, the
/// entries match the document; the toast names the zip; the dialog sees an existing zip as "will be replaced".
fn export_zip() -> Vec<Step> {
    let mut steps = default_content();
    steps.extend(t7_style_default_text());
    steps.extend([
        Step::Dyn("export everything loose", |s, sc| {
            Ok(vec![Step::Do(AppAction::RunExport(options_into(s, sc, "_export_ref", wte_core::ExportPackage::Files)?))])
        }),
        Step::WaitIdle,
        Step::Dyn("export everything as one zip", |s, sc| {
            Ok(vec![Step::Do(AppAction::RunExport(options_into(s, sc, "_export_zip", wte_core::ExportPackage::Zip)?))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("the toast names the zip and its file count", |s, sc| {
            let doc = doc_of(s)?;
            let zip = wte_core::export::package_name(&doc.name);
            let n = wte_core::export::plan_contents(&doc.snapshot(), &export_options(s, sc)?).len();
            let folder = with_suffix(&sc.out_stem, "_export_zip");
            let folder = folder.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let want = format!("Exported {zip} ({n} files) to {folder}");
            let labels = s.toasts.visible_labels();
            if !labels.contains(&want) {
                return Err(format!("no toast {want:?} in {labels:?}"));
            }
            Ok(())
        }),
        Step::Dyn("export the zip again into a second folder", |s, sc| {
            Ok(vec![Step::Do(AppAction::RunExport(options_into(s, sc, "_export_zip2", wte_core::ExportPackage::Zip)?))])
        }),
        Step::WaitIdle,
        Step::Assert("each zip folder holds only the zip; entries = loose files; the zips are identical; entries = document", |s, sc| {
            let doc = doc_of(s)?;
            let zip = wte_core::export::package_name(&doc.name);
            let mut packages = Vec::new();
            for suffix in ["_export_zip", "_export_zip2"] {
                let dir = with_suffix(&sc.out_stem, suffix);
                if dir_names(&dir)? != [zip.clone()] {
                    return Err(format!("{} holds {:?}", dir.display(), dir_names(&dir)?));
                }
                packages.push(std::fs::read(dir.join(&zip)).map_err(|e| e.to_string())?);
            }
            if packages[0] != packages[1] {
                return Err("two zip exports of the same document differ".to_owned());
            }
            let entries = wte_core::export::read_package(&packages[0])?;
            let loose_opts = ExportOptions { dir: with_suffix(&sc.out_stem, "_export_ref"), ..export_options(s, sc)? };
            let loose = read_export_dir(doc, &loose_opts)?;
            if entries.len() != loose.len() {
                return Err(format!("{} entries, {} loose files", entries.len(), loose.len()));
            }
            for ((name, bytes), (loose_name, loose_bytes)) in entries.iter().zip(&loose) {
                if name != loose_name || bytes != loose_bytes {
                    return Err(format!("entry {name} differs from the loose file {loose_name}"));
                }
            }
            let per_layer = |kind: &str| entries.iter().filter(|(n, _)| n.ends_with(".png") && n.contains(&format!("_{kind}_"))).count();
            if per_layer("stamp") != 1 || per_layer("text") != 1 {
                return Err("expected one stamp and one text entry".to_owned());
            }
            let summary = verify_export(doc, &export_options(s, sc)?, &entries)?;
            log::info!("export_zip: {} entries, {} bytes; {summary}", entries.len(), packages[0].len());
            Ok(())
        }),
        Step::Do(AppAction::ShowExport),
        Step::WaitIdle,
        Step::Setup("the dialog packs into the first zip's folder", |s, sc| {
            let dir = with_suffix(&sc.out_stem, "_export_zip");
            let Dialog::Export(st) = &mut s.dialog else { return Err("the Export dialog is not open".to_owned()) };
            st.options.dir = dir;
            st.options.package = wte_core::ExportPackage::Zip;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the dialog plans the one zip, which exists (will be replaced)", |s, sc| {
            let Dialog::Export(st) = &s.dialog else { return Err("the Export dialog is not open".to_owned()) };
            let doc = doc_of(s)?;
            let plan = wte_core::export::plan(&doc.snapshot(), &st.options);
            let zip = with_suffix(&sc.out_stem, "_export_zip").join(wte_core::export::package_name(&doc.name));
            if plan != [zip.clone()] || !zip.is_file() {
                return Err(format!("the dialog plans {plan:?}"));
            }
            Ok(())
        }),
        Step::Shot("replace"),
        Step::Do(AppAction::CloseDialog),
        Step::WaitIdle,
    ]);
    steps
}

/// The generated models of `export_flow` without PATH, in run order (tags in `FLOW_TAGS`).
const FLOW_FILES: [&str; 4] = ["flow_cube.obj", "flow_cube.gltf", "flow_cube.glb", "flow_cube.fbx"];
/// Output folder tags of the models: `<out>_<tag>_one`, `_sel`, `_all`.
const FLOW_TAGS: [&str; 5] = ["obj", "gltf", "glb", "fbx", "path"];
/// The checker the OBJ cube's MTL references.
fn flow_texture() -> PixelBuf {
    samples::checker_texture(ASSET_TEXTURE_SIDE, ASSET_TEXTURE_SIDE, 8)
}

fn flow_write_assets(_state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let dir = fresh_assets_dir(sc)?;
    let cube = samples::seam_cube(1.0);
    let (obj, mtl) = samples::to_obj(&cube, "flow_cube.mtl", &["flow_cube.png".to_owned()]);
    write(&dir.join("flow_cube.obj"), obj.as_bytes())?;
    write(&dir.join("flow_cube.mtl"), mtl.as_bytes())?;
    write(&dir.join("flow_cube.png"), &png_of(&flow_texture())?)?;
    write(&dir.join("flow_cube.gltf"), samples::to_gltf(&cube, glam::Mat4::IDENTITY).as_bytes())?;
    write(&dir.join("flow_cube.glb"), &samples::to_glb(&[(cube.clone(), glam::Mat4::IDENTITY)]))?;
    write(&dir.join("flow_cube.fbx"), samples::to_fbx_ascii(&cube, 1.0, [1.0, 1.0, 1.0]).as_bytes())
}

/// The index of the model in work (`FLOW_TAGS`).
fn flow_model(sc: &Scratch) -> Result<usize, String> {
    remembered(sc, "flow_model").map(|i| i as usize)
}

fn flow_tag(sc: &Scratch) -> Result<&'static str, String> {
    FLOW_TAGS.get(flow_model(sc)?).copied().ok_or_else(|| "no model in work".to_owned())
}

/// `<out_stem>_<tag>_<what>`.
fn flow_dir(sc: &Scratch, what: &str) -> Result<PathBuf, String> {
    Ok(with_suffix(&sc.out_stem, &format!("_{}_{what}", flow_tag(sc)?)))
}

/// A hash of the document's pixels: every composite and every layer (properties and pixels).
fn flow_doc_key(doc: &Document) -> u64 {
    let mut h = composite_hash(doc);
    for set in &doc.sets {
        for layer in &set.layers {
            h = wte_core::fnv1a64(h, &layer.id.0.to_le_bytes());
            h = wte_core::fnv1a64(h, &[layer.visible as u8, layer.locked as u8]);
            h = wte_core::fnv1a64(h, &layer.opacity.to_le_bytes());
            h = wte_core::fnv1a64(h, layer.blend.label().as_bytes());
            match &layer.kind {
                LayerKind::Raster(buf) => h = wte_core::fnv1a64(h, &wte_core::testutil::hash_pixels(buf).to_le_bytes()),
                LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => {
                    for part in cache.parts() {
                        h = wte_core::fnv1a64(h, &rect_key(&part.rect).to_le_bytes());
                        h = wte_core::fnv1a64(h, &part.data);
                    }
                }
            }
        }
    }
    h
}

/// The layer of the selected stamp in its home set, remembered under `key`.
fn remember_stamp_layer(state: &AppState, sc: &mut Scratch, text: bool, key: &'static str) -> Result<(), String> {
    let (id, _) = placed_stamp(state, text)?;
    let doc = doc_of(state)?;
    let home = doc.stamp(id).ok_or("the stamp is gone")?.home_set;
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home).ok_or("the stamp has no layer in its home set")?;
    sc.values.insert(key, layer.0);
    Ok(())
}

/// A solid, well-facing spot as far as possible from the image stamp's and the text's spots.
fn third_spot(state: &AppState) -> Result<(ViewXform, ScreenHit), String> {
    let (xf, first, second) = stamp_spots(state)?;
    let (_, hits) = model_hits(state)?;
    let doc = doc_of(state)?;
    let solid = |h: &ScreenHit| {
        [egui::vec2(0.0, 0.0), egui::vec2(3.0, 0.0), egui::vec2(-3.0, 0.0), egui::vec2(0.0, 3.0), egui::vec2(0.0, -3.0)]
            .iter()
            .all(|d| matches!(xf.surface_at(doc, h.set, h.pos + *d), Surface::Model(hit) if hit.texture_set.is_some()))
    };
    let ranked = best_spots(&hits);
    let good: Vec<ScreenHit> = ranked.iter().take(ranked.len().clamp(1, (ranked.len() / 3).max(8))).filter(|h| solid(h)).copied().collect();
    let pool = if good.is_empty() { vec![first] } else { good };
    let away = |h: &ScreenHit| (h.pos - first.pos).length().min((h.pos - second.pos).length());
    let spot = pool.iter().max_by(|a, b| away(a).total_cmp(&away(b))).copied().ok_or("no hit")?;
    Ok((xf, spot))
}

/// The two layers of the selection export: the image stamp and the text when they share a set, else the first pair
/// of {stamp, text, wrapped stamp} that does (a selection lies in one set), else the stamp and its set's Paint 1.
fn flow_pair(state: &AppState, sc: &Scratch) -> Result<(LayerId, LayerId), String> {
    let doc = doc_of(state)?;
    let ids = ["flow_stamp", "flow_text", "flow_wrap"].map(|k| remembered(sc, k).map(LayerId));
    let [stamp, text, wrap] = [ids[0].clone()?, ids[1].clone()?, ids[2].clone()?];
    let set_of = |l: LayerId| doc.find_layer(l).map(|(s, _)| s);
    for (a, b) in [(stamp, text), (stamp, wrap), (text, wrap)] {
        if set_of(a).is_some() && set_of(a) == set_of(b) {
            return Ok((a, b));
        }
    }
    let set = doc.set(set_of(stamp).ok_or("the stamp layer is gone")?).ok_or("no set")?;
    let paint = set.layers.iter().find(|l| l.name == "Paint 1").ok_or("no Paint 1 in the stamp's set")?.id;
    log::warn!("export_flow: the stamps lie in different sets; the selection is the stamp and Paint 1");
    Ok((paint, stamp))
}

/// The options of the "everything as one package" export.
fn flow_all_options(state: &AppState, sc: &Scratch) -> Result<ExportOptions, String> {
    Ok(ExportOptions { dir: flow_dir(sc, "all")?, package: wte_core::ExportPackage::Zip, ..export_options(state, sc)? })
}

/// The import of the model in work: a document with texture sets (the OBJ cube: its Base is the checker it references).
fn flow_imported(state: &AppState, sc: &Scratch) -> Result<(), String> {
    let doc = doc_of(state).map_err(|_| format!("no document was opened (dialog: {})", dialog_name(&state.dialog)))?;
    if doc.sets.is_empty() {
        return Err("the document has no texture set".to_owned());
    }
    log_document(doc);
    if flow_tag(sc)? == "obj" {
        let base = doc.sets[0].layers.first().ok_or("no Base layer")?;
        let LayerKind::Raster(pixels) = &base.kind else { return Err("the Base is not a paint layer".to_owned()) };
        if **pixels != flow_texture() {
            return Err("the Base of the OBJ cube is not the checker its MTL references".to_owned());
        }
    }
    Ok(())
}

/// Opens the next generated model (after "Don't save" for the current document).
fn flow_open_next(_state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let next = sc.values.get("flow_next").copied().unwrap_or(0);
    sc.values.insert("flow_model", next);
    sc.values.insert("flow_next", next + 1);
    log::info!("export_flow: model {}", FLOW_FILES.get(next as usize).copied().unwrap_or("?"));
    phase_mark(Some("export_flow model"));
    Ok(())
}

fn flow_open() -> Vec<Step> {
    vec![
        Step::Setup("next generated model", flow_open_next),
        Step::Dyn("open it as a new document", |_, sc| {
            let path = assets_dir(sc).join(FLOW_FILES.get(flow_model(sc)?).ok_or("no such model")?);
            Ok(vec![Step::Do(AppAction::ConfirmPending(Pending::NewFromModel(Some(path))))])
        }),
        Step::WaitIdle,
        Step::Dyn("accept the import dialog with its defaults", accept_import),
        Step::WaitIdle,
        Step::Dyn("close an import warnings message", close_message),
        Step::WaitIdle,
    ]
}

/// Gives every set whose active layer is not a paint layer (a stamp or text layer, or none, as a saved project may
/// leave it) a new paint layer, as the brush's own steps (3) and (4) would on the first press (toast "Created paint
/// layer"), so the stroke that follows is exactly one history entry on a fresh import and on a saved `.wtep` alike
/// (check_exp_1 D6). A fresh import already has "Paint 1" active in every set: nothing changes there.
fn flow_paint_layers(state: &mut AppState, _sc: &mut Scratch) -> Result<(), String> {
    let doc = &mut state.session.as_mut().ok_or("no document")?.doc;
    for set in doc.set_ids() {
        match doc.paint_target(set) {
            wte_core::PaintTarget::Ready(_) => {}
            wte_core::PaintTarget::NotRaster(_) | wte_core::PaintTarget::NoActiveLayer => {
                let layer = doc.add_raster_layer(set, None).map_err(|e| format!("set {set:?}: {e:?}"))?;
                log::info!("export_flow: set {set:?} gets the paint layer {layer:?} before the stroke");
            }
            other => return Err(format!("set {set:?}: the active layer cannot be painted ({other:?})")),
        }
    }
    Ok(())
}

/// The flow on the open document (EXPORT_SELECT_PLAN 7): paint, stamp, text and a wrapped stamp, then export one
/// layer, a selection and everything as one zip, each checked file by file against the document, which the exports
/// must leave unchanged.
fn flow_steps() -> Vec<Step> {
    vec![
        Step::Assert("a document with texture sets (the OBJ cube's Base is its texture)", flow_imported),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Setup("every set's active layer is a paint layer (a saved project may end on a stamp layer)", flow_paint_layers),
        Step::Setup("remember the history and the composite", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "flow_before")
        }),
        // Paint.
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::BrushSizeStep(2)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("a brush stroke on Paint 1", |s, _| hit_stroke(s, 0)),
        Step::WaitIdle,
        Step::Assert("the stroke painted: one history entry, composite changed", |s, sc| {
            undo_since(s, sc, 1)?;
            changed_since(s, sc, "flow_before")
        }),
        // Image stamp (Project).
        Step::Setup("Stamp tool: the sample image, 20 %, Project", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.2;
            s.tools.stamp.rotation_deg = 0.0;
            s.tools.stamp.mode = SurfaceMode::Project;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the image stamp", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Setup("remember the image stamp's layer", |s, sc| remember_stamp_layer(s, sc, false, "flow_stamp")),
        // Text.
        Step::Setup("text content \"WTE\"", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            s.tools.text.em_frac = 0.08;
            s.tools.text.color = [20, 20, 20, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the text", |s, _| {
            let (xf, _, second) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, second.pos))])
        }),
        Step::WaitIdle,
        Step::Setup("remember the text's layer", |s, sc| remember_stamp_layer(s, sc, true, "flow_text")),
        // Wrapped stamp.
        Step::Setup("Stamp tool: Wrap", |s, _| {
            s.tools.stamp.mode = SurfaceMode::Wrap;
            s.tools.stamp.wrap = WrapOptions::default();
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the wrapped stamp", |s, _| {
            let (xf, spot) = third_spot(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, spot.pos))])
        }),
        Step::WaitIdle,
        Step::Setup("the wrapped stamp: Wrap, covers texels, Final bake landed; remember it and the document", |s, sc| {
            let (id, placement) = placed_stamp(s, false)?;
            let Placement::Projected(p) = placement else { return Err("the wrapped stamp is not a 3D stamp".to_owned()) };
            if p.mode != SurfaceMode::Wrap {
                return Err(format!("the third stamp was placed in {:?}", p.mode));
            }
            if s.final_bakes_pending || s.wrap_jobs_pending || doc_of(s)?.has_unbaked_stamps() {
                return Err("a Final bake is still pending".to_owned());
            }
            log::info!("export_flow: wrapped stamp {id:?} placed with {p:?}");
            remember_stamp_layer(s, sc, false, "flow_wrap")?;
            remember_undo(s, sc)?;
            let doc = doc_of(s)?;
            let (key, rev) = (flow_doc_key(doc), doc.revision());
            sc.values.insert("flow_key", key);
            sc.values.insert("flow_rev", rev);
            Ok(())
        }),
        // One layer.
        Step::Dyn("Export this layer: the wrapped stamp", |_, sc| {
            Ok(vec![Step::Do(AppAction::ShowExportLayers(vec![LayerId(remembered(sc, "flow_wrap")?)]))])
        }),
        Step::WaitIdle,
        Step::Setup("into <out>_<model>_one", |s, sc| dialog_into_dir(s, flow_dir(sc, "one")?)),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("exactly the wrapped stamp's PNG, its layer's pixels, with ink", |s, sc| {
            assert_no_dialog(s, sc)?;
            let images = expect_layer_files(s, &flow_dir(sc, "one")?, &[LayerId(remembered(sc, "flow_wrap")?)])?;
            let n = ink(&images[0]);
            if n == 0 {
                return Err("the wrapped stamp's file is empty".to_owned());
            }
            log::info!("export_flow: wrapped stamp file with {n} inked texels");
            Ok(())
        }),
        Step::Assert("the toast counts the files right (\"1 file\")", |s, _| toasts_count_right(s)),
        // fix_export item 1: the dialog opened for one layer, switched to Visible, exports everything again.
        Step::Dyn("Export this layer again (the wrapped stamp)", |_, sc| {
            Ok(vec![Step::Do(AppAction::ShowExportLayers(vec![LayerId(remembered(sc, "flow_wrap")?)]))])
        }),
        Step::WaitIdle,
        Step::Setup("switch the dialog's Layers choice to Visible", |s, _| {
            let Dialog::Export(st) = &mut s.dialog else { return Err(format!("the Export dialog is not open ({})", dialog_name(&s.dialog))) };
            crate::dialogs::export::switch_scope(st, crate::dialogs::export::LayerScope::Visible);
            Ok(())
        }),
        Step::Assert("Visible: every texture set, the flattened texture and the normal formats again", |s, _| {
            let Dialog::Export(st) = &s.dialog else { return Err("the Export dialog closed".to_owned()) };
            let o = &st.options;
            if o.layers.is_some() || o.sets.is_some() || o.include_hidden {
                return Err(format!("layers {:?}, sets {:?}, hidden {}", o.layers, o.sets, o.include_hidden));
            }
            if o.flat != Some(FlatFormat::Png(PngLevel::Balanced)) || !o.per_layer_png || o.ora || o.psd {
                return Err(format!("flat {:?}, per-layer {}, ora {}, psd {}", o.flat, o.per_layer_png, o.ora, o.psd));
            }
            let doc = doc_of(s)?;
            let flats = wte_core::export::plan_contents(&doc.snapshot(), o).iter().filter(|f| f.kind == wte_core::PlannedKind::Flat).count();
            if flats != doc.sets.len() {
                return Err(format!("{flats} flattened textures for {} sets", doc.sets.len()));
            }
            Ok(())
        }),
        Step::Setup("and back to Selected", |s, _| {
            let Dialog::Export(st) = &mut s.dialog else { return Err("the Export dialog closed".to_owned()) };
            crate::dialogs::export::switch_scope(st, crate::dialogs::export::LayerScope::Selected);
            Ok(())
        }),
        Step::Assert("Selected: the layer preset again (per-layer PNGs only)", |s, sc| {
            let Dialog::Export(st) = &s.dialog else { return Err("the Export dialog closed".to_owned()) };
            let o = &st.options;
            if o.layers.as_deref() != Some(&[LayerId(remembered(sc, "flow_wrap")?)][..]) || o.flat.is_some() || !o.per_layer_png {
                return Err(format!("layers {:?}, flat {:?}, per-layer {}", o.layers, o.flat, o.per_layer_png));
            }
            Ok(())
        }),
        Step::Do(AppAction::CloseDialog),
        Step::WaitIdle,
        // A selection of two.
        Step::Dyn("select the stamp, Ctrl-select the text", |s, sc| {
            let (a, b) = flow_pair(s, sc)?;
            Ok(vec![Step::Do(AppAction::SelectLayer(a)), Step::Do(AppAction::ToggleLayerSelected(b))])
        }),
        Step::WaitIdle,
        Step::Dyn("Export selected layers", |s, sc| {
            let (a, b) = flow_pair(s, sc)?;
            let selection = session_of(s)?.layer_selection();
            if selection.len() != 2 || !selection.contains(&a) || !selection.contains(&b) {
                return Err(format!("the selection is {selection:?}, expected {a:?} and {b:?}"));
            }
            Ok(vec![Step::Do(AppAction::ShowExportLayers(selection))])
        }),
        Step::WaitIdle,
        Step::Setup("into <out>_<model>_sel", |s, sc| dialog_into_dir(s, flow_dir(sc, "sel")?)),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("exactly the two PNGs, each its layer, different, without the stroke colour", |s, sc| {
            assert_no_dialog(s, sc)?;
            let (a, b) = flow_pair(s, sc)?;
            let images = expect_layer_files(s, &flow_dir(sc, "sel")?, &[a, b])?;
            if images[0] == images[1] {
                return Err("the two exported layers are the same picture".to_owned());
            }
            let doc = doc_of(s)?;
            for (id, image) in [a, b].iter().zip(&images) {
                let stamp_layer = doc.layer(*id).is_some_and(|l| !l.is_raster());
                if stamp_layer && count_color(image, RED) > 0 {
                    return Err("a stamp layer's file holds the stroke colour".to_owned());
                }
            }
            Ok(())
        }),
        // Everything, one package.
        Step::Dyn("export everything as one zip", |s, sc| {
            let opts = flow_all_options(s, sc)?;
            if opts.dir.exists() {
                std::fs::remove_dir_all(&opts.dir).map_err(|e| e.to_string())?;
            }
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::WaitIdle,
        Step::Assert("one zip; its entries are the planned files and match the document", |s, sc| {
            assert_no_dialog(s, sc)?;
            let doc = doc_of(s)?;
            let opts = flow_all_options(s, sc)?;
            let zip = wte_core::export::package_name(&doc.name);
            let listing = dir_names(&opts.dir)?;
            if listing != [zip.clone()] {
                return Err(format!("{} holds {listing:?}", opts.dir.display()));
            }
            let bytes = std::fs::read(opts.dir.join(&zip)).map_err(|e| e.to_string())?;
            let entries = wte_core::export::read_package(&bytes)?;
            let summary = verify_export(doc, &opts, &entries)?;
            log::info!("export_flow {}: {zip} {} bytes; {summary}", flow_tag(sc)?, bytes.len());
            Ok(())
        }),
        // fix_export item 10: the same export as loose files into <out>_<model>_loose: the zip's entries byte for byte
        // (checked here and by tools/stable/verify_zip_vs_loose.py).
        Step::Dyn("export everything as loose files too", |s, sc| {
            let dir = flow_dir(sc, "loose")?;
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
            Ok(vec![Step::Do(AppAction::RunExport(ExportOptions { dir, package: wte_core::ExportPackage::Files, ..flow_all_options(s, sc)? }))])
        }),
        Step::WaitIdle,
        Step::Assert("the loose files are the zip's entries, byte for byte", |s, sc| {
            let doc = doc_of(s)?;
            let zip = flow_all_options(s, sc)?.dir.join(wte_core::export::package_name(&doc.name));
            let mut entries = wte_core::export::read_package(&std::fs::read(&zip).map_err(|e| e.to_string())?)?;
            entries.sort();
            let loose = dir_bytes(&flow_dir(sc, "loose")?)?;
            if entries.len() != loose.len() || entries.iter().zip(&loose).any(|(a, b)| a.0 != b.0) {
                return Err(format!("{} entries, {} loose files", entries.len(), loose.len()));
            }
            if let Some((name, _)) = entries.iter().zip(&loose).find(|(a, b)| a.1 != b.1).map(|(a, _)| a) {
                return Err(format!("{name} differs between the zip and the loose file"));
            }
            Ok(())
        }),
        // fix_export item 2: the selection as a zip into the same folder: its own name, the full package unchanged.
        Step::Setup("remember the full package", |s, sc| {
            let opts = flow_all_options(s, sc)?;
            let bytes = std::fs::read(opts.dir.join(wte_core::export::package_name(&doc_of(s)?.name))).map_err(|e| e.to_string())?;
            sc.values.insert("flow_zip", wte_core::fnv1a64(0, &bytes));
            Ok(())
        }),
        Step::Dyn("export the selection as a zip into the same folder", |s, sc| {
            let (a, b) = flow_pair(s, sc)?;
            let opts = ExportOptions {
                layers: Some(vec![a, b]),
                flat: None,
                ora: false,
                psd: false,
                ..flow_all_options(s, sc)?
            };
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::WaitIdle,
        Step::Assert("<base>_selection.zip next to the unchanged <base>.zip; its entries are the full package's", |s, sc| {
            let doc = doc_of(s)?;
            let dir = flow_all_options(s, sc)?.dir;
            let (full, sel) = (wte_core::export::package_name(&doc.name), wte_core::export::selection_package_name(&doc.name));
            let mut want = vec![full.clone(), sel.clone()];
            want.sort();
            let listing = dir_names(&dir)?;
            if listing != want {
                return Err(format!("{} holds {listing:?}", dir.display()));
            }
            let full_bytes = std::fs::read(dir.join(&full)).map_err(|e| e.to_string())?;
            if wte_core::fnv1a64(0, &full_bytes) != remembered(sc, "flow_zip")? {
                return Err("the full package changed".to_owned());
            }
            let full_entries = wte_core::export::read_package(&full_bytes)?;
            let sel_entries = wte_core::export::read_package(&std::fs::read(dir.join(&sel)).map_err(|e| e.to_string())?)?;
            if sel_entries.len() < 2 {
                return Err(format!("the selection zip holds {} entries", sel_entries.len()));
            }
            for (name, bytes) in &sel_entries {
                match full_entries.iter().find(|(n, _)| n == name) {
                    Some((_, b)) if b == bytes => {}
                    Some(_) => return Err(format!("{name} differs from the full package's entry")),
                    None => return Err(format!("{name} is not a name of the full package")),
                }
            }
            log::info!("export_flow {}: {sel} with {} entries, all equal to the full package's", flow_tag(sc)?, sel_entries.len());
            toasts_count_right(s)
        }),
        Step::Assert("the exports left the document unchanged; no temp file anywhere", |s, sc| {
            let doc = doc_of(s)?;
            if flow_doc_key(doc) != remembered(sc, "flow_key")? {
                return Err("the layers or composites changed during the exports".to_owned());
            }
            if doc.revision() != remembered(sc, "flow_rev")? {
                return Err(format!("the revision changed during the exports ({} -> {})", remembered(sc, "flow_rev")?, doc.revision()));
            }
            undo_since(s, sc, 0)?;
            for what in ["one", "sel", "all", "loose"] {
                let names = dir_names(&flow_dir(sc, what)?)?;
                if names.iter().any(|n| n.contains(".tmp-")) {
                    return Err(format!("a temp file or folder is left in {what}: {names:?}"));
                }
            }
            Ok(())
        }),
        // fix_export item 5: Duplicate and Delete on two selected layers, one undo step each.
        Step::Dyn("select the pair again", |s, sc| {
            let (a, b) = flow_pair(s, sc)?;
            Ok(vec![Step::Do(AppAction::SelectLayer(a)), Step::Do(AppAction::ToggleLayerSelected(b))])
        }),
        Step::WaitIdle,
        Step::Setup("remember the history and the pair", |s, sc| {
            let (a, b) = flow_pair(s, sc)?;
            sc.values.insert("flow_pa", a.0);
            sc.values.insert("flow_pb", b.0);
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::DuplicateActiveLayer),
        Step::WaitIdle,
        Step::Assert("Duplicate on two selected layers: two copies, selected, one undo step", |s, sc| {
            undo_since(s, sc, 1)?;
            let (a, b) = flow_pair(s, sc)?;
            let selection = session_of(s)?.layer_selection();
            if selection.len() != 2 || selection.contains(&a) || selection.contains(&b) {
                return Err(format!("the selection after Duplicate is {selection:?} (originals {a:?}, {b:?})"));
            }
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("one Undo removes both copies", |s, sc| {
            if flow_doc_key(doc_of(s)?) != remembered(sc, "flow_key")? {
                return Err("the document differs after the undo of Duplicate".to_owned());
            }
            Ok(())
        }),
        Step::Dyn("select the pair again", |s, sc| {
            let (a, b) = flow_pair(s, sc)?;
            Ok(vec![Step::Do(AppAction::SelectLayer(a)), Step::Do(AppAction::ToggleLayerSelected(b))])
        }),
        Step::WaitIdle,
        Step::Do(AppAction::DeleteSelection),
        Step::WaitIdle,
        Step::Assert("Delete on two selected layers: both gone, one undo step", |s, sc| {
            undo_since(s, sc, 1)?;
            let doc = doc_of(s)?;
            for key in ["flow_pa", "flow_pb"] {
                let id = LayerId(remembered(sc, key)?);
                if doc.layer(id).is_some() {
                    return Err(format!("the selected layer {id:?} is still there"));
                }
            }
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("one Undo brings both back", |s, sc| {
            if flow_doc_key(doc_of(s)?) != remembered(sc, "flow_key")? {
                return Err("the document differs after the undo of Delete".to_owned());
            }
            phase_mark(None);
            Ok(())
        }),
    ]
}


/// `export_flow [PATH]` (EXPORT_SELECT_PLAN 7): the user's workflow end to end on a generated cube as OBJ (+ MTL +
/// texture), glTF, GLB and ASCII FBX, or once on PATH.
fn export_flow(has_path: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    if has_path {
        steps.push(Step::Setup("the model is PATH", |_, sc| {
            sc.values.insert("flow_model", 4);
            phase_mark(Some("export_flow PATH"));
            Ok(())
        }));
        steps.extend(open_and_accept(true));
        steps.extend(flow_steps());
    } else {
        steps.push(Step::Setup("write the cube as OBJ + MTL + PNG, glTF, GLB and ASCII FBX", flow_write_assets));
        for _ in FLOW_FILES {
            steps.extend(flow_open());
            steps.extend(flow_steps());
        }
    }
    steps.push(Step::Do(AppAction::SetViewMode(ViewMode::Split)));
    steps.push(Step::WaitIdle);
    steps
}

/// The longest UI frame an export may cause while it waits for a wrapped stamp's Final (fix_export item 6).
const EXPORT_MAX_FRAME_MS: f64 = 300.0;

/// The options of `export_unsettled`: the wrapped stamp's layers (its linked layers of other sets come along) as
/// per-layer PNGs and the flattened texture of that selection, into `<out_stem><suffix>`.
fn unsettled_options(state: &AppState, sc: &Scratch, suffix: &str) -> Result<ExportOptions, String> {
    let doc = doc_of(state)?;
    let stamp = StampId(remembered(sc, "eu_stamp")?);
    let layers: Vec<LayerId> = doc.stamp_layers(stamp).into_iter().map(|(_, l)| l).collect();
    if layers.is_empty() {
        return Err("the wrapped stamp has no layer".to_owned());
    }
    Ok(ExportOptions {
        dir: with_suffix(&sc.out_stem, suffix),
        layers: Some(layers),
        flat: Some(FlatFormat::Png(PngLevel::Balanced)),
        ora: false,
        psd: false,
        ..export_options(state, sc)?
    })
}

/// `export_unsettled [PATH]` (fix_export item 6): a wrapped stamp is placed and exported at once, before its Final bake
/// has landed (3-4 s on ABeautifulGame, seconds on high_poly_sphere). The export waits behind its progress dialog
/// instead of on the UI thread: no frame takes `EXPORT_MAX_FRAME_MS` or more, and the files are byte-identical to an
/// export made after waiting, and equal to the layers' Final pixels. The sphere sample without PATH.
fn export_unsettled(has_path: bool) -> Vec<Step> {
    let mut steps = if has_path { open_and_accept(true) } else { vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle] };
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Setup("Stamp tool: the sample image, 30 %, Wrap", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.3;
            s.tools.stamp.rotation_deg = 0.0;
            s.tools.stamp.mode = SurfaceMode::Wrap;
            s.tools.stamp.wrap = WrapOptions::default();
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::BakeWait(false),
        Step::Dyn("place the wrapped stamp", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::Setup("a Final is pending; reset the frame clock", |s, sc| {
            // The stamp as it is now (its picture may not have landed yet: no coverage check).
            let session = session_of(s)?;
            let id = session.selected_stamp.ok_or("no stamp is selected")?;
            let placement = session.doc.stamp(id).ok_or("the selected stamp does not exist")?.placement;
            let Placement::Projected(p) = placement else { return Err("the stamp is not a 3D stamp".to_owned()) };
            if p.mode != SurfaceMode::Wrap {
                return Err(format!("the stamp was placed in {:?}", p.mode));
            }
            let pending = s.final_bakes_pending || doc_of(s)?.has_unbaked_stamps();
            if !pending {
                return Err("the Final bake already landed: the export would not wait".to_owned());
            }
            sc.values.insert("eu_stamp", id.0);
            s.frame_work.reset();
            for suffix in ["_a", "_b"] {
                let dir = with_suffix(&sc.out_stem, suffix);
                if dir.exists() {
                    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }),
        Step::Dyn("Export at once, while the Final is pending", |s, sc| {
            let opts = unsettled_options(s, sc, "_a")?;
            log::info!("export_unsettled: export with the Final pending: {:?}", opts.layers);
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::BakeWait(true),
        Step::WaitIdle,
        Step::Assert("the UI kept drawing: no frame took 300 ms or more", |s, _| {
            let ms = s.frame_work.max.as_secs_f64() * 1000.0;
            log::info!("export_unsettled: longest frame {ms:.0} ms over {} frames", s.frame_work.frames);
            if ms >= EXPORT_MAX_FRAME_MS {
                return Err(format!("a frame took {ms:.0} ms (limit {EXPORT_MAX_FRAME_MS} ms)"));
            }
            if s.frame_work.frames < 3 {
                return Err(format!("only {} frames were drawn during the export", s.frame_work.frames));
            }
            Ok(())
        }),
        Step::Assert("the export finished; nothing is pending; no dialog", |s, sc| {
            assert_no_dialog(s, sc)?;
            if s.final_bakes_pending || doc_of(s)?.has_unbaked_stamps() {
                return Err("a Final is still pending".to_owned());
            }
            if dir_names(&with_suffix(&sc.out_stem, "_a"))?.is_empty() {
                return Err("nothing was exported".to_owned());
            }
            Ok(())
        }),
        Step::Dyn("Export again, after waiting", |s, sc| Ok(vec![Step::Do(AppAction::RunExport(unsettled_options(s, sc, "_b")?))])),
        Step::WaitIdle,
        Step::Assert("both exports are byte-identical, and the layer files are the Final pixels", |s, sc| {
            let (a, b) = (with_suffix(&sc.out_stem, "_a"), with_suffix(&sc.out_stem, "_b"));
            let n = same_files(&a, &b)?;
            let doc = doc_of(s)?;
            let stamp = StampId(remembered(sc, "eu_stamp")?);
            let layers: Vec<LayerId> = doc.stamp_layers(stamp).into_iter().map(|(_, l)| l).collect();
            let mut ink_total = 0;
            for (id, name) in selection_files(doc, &layers) {
                let image = decode_named(&name, &std::fs::read(a.join(&name)).map_err(|e| format!("{name}: {e}"))?)?;
                let (set, _) = doc.find_layer(id).ok_or("no such layer")?;
                let set = doc.set(set).ok_or("no such set")?;
                let layer = doc.layer(id).ok_or("no such layer")?;
                let d = differing(&image, &layer_canvas(layer, set.width, set.height));
                if d != 0 {
                    return Err(format!("{name}: {d} texels differ from the layer's Final pixels"));
                }
                ink_total += ink(&image);
            }
            if ink_total == 0 {
                return Err("the wrapped stamp's files are empty".to_owned());
            }
            log::info!("export_unsettled: {n} files byte-identical; {ink_total} inked texels");
            Ok(())
        }),
    ]);
    steps
}

/// `export_nouv` (fix_export item 9, L3): a model without UVs (generated layout) is painted and stamped, exported, saved,
/// reopened and exported again: both exports are byte-identical, file for file (flattened, per-layer, ORA, PSD), and
/// so is a zip of the reopened document to the loose files.
fn export_nouv() -> Vec<Step> {
    vec![
        Step::Setup("write the UV-less tetrahedron as OBJ", |_, sc| {
            let dir = fresh_assets_dir(sc)?;
            let (obj, _) = samples::to_obj(&samples::no_uv_tetra(), "", &[]);
            write(&dir.join("tetra.obj"), obj.as_bytes())
        }),
        Step::Dyn("New from Model", |_, sc| Ok(vec![Step::Do(AppAction::NewFromModel(Some(assets_dir(sc).join("tetra.obj"))))])),
        Step::WaitIdle,
        Step::Assert("the import dialog generated a layout", |s, _| {
            let st = import_dialog(s)?;
            let model = st.model.as_ref().ok_or("the import dialog has no model")?;
            if !model.report.no_uvs || !model.report.fallback_uvs_generated {
                return Err("no generated layout".to_owned());
            }
            Ok(())
        }),
        Step::Do(AppAction::ImportCreate),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Do(AppAction::BrushSizeStep(3)),
        Step::Dyn("brush stroke on the model", |s, _| model_stroke(s, ViewId::Main3D, &[[-0.35, 0.0], [0.0, 0.1], [0.3, -0.05]])),
        Step::WaitIdle,
        Step::Setup("register the sample stamp image (Project)", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.3;
            s.tools.stamp.mode = SurfaceMode::Project;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the stamp", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Dyn("export everything", |s, sc| {
            let dir = fresh_out_dir(sc, "_a")?;
            Ok(vec![Step::Do(AppAction::RunExport(ExportOptions { dir, ..export_options(s, sc)? }))])
        }),
        Step::WaitIdle,
        Step::Assert("the export holds the flattened texture, every layer, ORA and PSD", |s, sc| {
            assert_no_dialog(s, sc)?;
            let names = dir_names(&with_suffix(&sc.out_stem, "_a"))?;
            let doc = doc_of(s)?;
            let want = 1 + doc.sets[0].layers.iter().filter(|l| l.visible).count() + 2;
            if names.len() != want {
                return Err(format!("{} files, expected {want}: {names:?}", names.len()));
            }
            Ok(())
        }),
        Step::Dyn("Save As the project", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the reopened project is open", |s, sc| {
            assert_no_dialog(s, sc)?;
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, ".wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            Ok(())
        }),
        Step::Dyn("export everything again", |s, sc| {
            let dir = fresh_out_dir(sc, "_b")?;
            Ok(vec![Step::Do(AppAction::RunExport(ExportOptions { dir, ..export_options(s, sc)? }))])
        }),
        Step::WaitIdle,
        Step::Assert("the two exports are byte-identical", |_, sc| {
            let n = same_files(&with_suffix(&sc.out_stem, "_a"), &with_suffix(&sc.out_stem, "_b"))?;
            log::info!("export_nouv: {n} files byte-identical before and after save + reopen");
            Ok(())
        }),
        Step::Dyn("and as one zip", |s, sc| {
            let dir = fresh_out_dir(sc, "_zip")?;
            Ok(vec![Step::Do(AppAction::RunExport(ExportOptions { dir, package: wte_core::ExportPackage::Zip, ..export_options(s, sc)? }))])
        }),
        Step::WaitIdle,
        Step::Assert("the zip's entries are the loose files", |s, sc| {
            let doc = doc_of(s)?;
            let zip = with_suffix(&sc.out_stem, "_zip").join(wte_core::export::package_name(&doc.name));
            let entries = wte_core::export::read_package(&std::fs::read(&zip).map_err(|e| e.to_string())?)?;
            let loose = dir_bytes(&with_suffix(&sc.out_stem, "_b"))?;
            let mut sorted = entries.clone();
            sorted.sort();
            if sorted != loose {
                return Err("the zip's entries differ from the loose files".to_owned());
            }
            toasts_count_right(s)
        }),
    ]
}

/// A `Step::Setup` function.
type SetupFn = fn(&mut AppState, &mut Scratch) -> Result<(), String>;

/// A process id that is not running (the stale temp files of `export_errors`).
const DEAD_PID: u32 = u32::MAX - 3;

/// `export_errors` (fix_export item 9, L1 / L4): exports into folders that cannot take them, driven through the app:
/// a folder path that is a file, a planned file that is a folder (Files: nothing written, the old file kept), the zip's
/// name taken by a folder (no stage folder, no temp file); each ends in the "Export failed" message and an error toast,
/// and the app goes on. Then a crashed export's stage folder and temp files of a process that is not running are
/// cleaned up by the next export into that folder.
fn export_errors() -> Vec<Step> {
    let mut steps = vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle];
    let failing: [(&'static str, SetupFn); 3] = [
        ("the folder path is a file", |s, sc| {
            let root = fresh_out_dir(sc, "_err")?;
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let file = root.join("a file");
            std::fs::write(&file, b"x").map_err(|e| e.to_string())?;
            err_export(s, sc, file.join("out"), wte_core::ExportPackage::Files)
        }),
        ("a planned file is a folder (Files)", |s, sc| {
            let dir = fresh_out_dir(sc, "_err")?;
            let opts = ExportOptions { dir: dir.clone(), ..export_options(s, sc)? };
            let plan = wte_core::export::plan(&doc_of(s)?.snapshot(), &opts);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(&plan[0], b"old").map_err(|e| e.to_string())?;
            std::fs::create_dir(plan.last().ok_or("empty plan")?).map_err(|e| e.to_string())?;
            sc.values.insert("err_files", 2);
            err_export(s, sc, dir, wte_core::ExportPackage::Files)
        }),
        ("the zip's name is a folder", |s, sc| {
            let dir = fresh_out_dir(sc, "_err")?;
            std::fs::create_dir_all(dir.join(wte_core::export::package_name(&doc_of(s)?.name))).map_err(|e| e.to_string())?;
            sc.values.insert("err_files", 1);
            err_export(s, sc, dir, wte_core::ExportPackage::Zip)
        }),
    ];
    for (what, setup) in failing {
        steps.extend([
            Step::Setup(what, setup),
            Step::Dyn("export", |_, _| {
                let opts = ERR_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("no options")?;
                Ok(vec![Step::Do(AppAction::RunExport(opts))])
            }),
            Step::WaitIdle,
            Step::Assert("\"Export failed\": the message and an error toast; nothing new in the folder; no temp file", |s, sc| {
                match &s.dialog {
                    Dialog::Message { title, lines } if title == "Export failed" => log::info!("export_errors: {lines:?}"),
                    other => return Err(format!("the dialog is {} (expected the \"Export failed\" message)", dialog_name(other))),
                }
                if !s.toasts.visible_labels().iter().any(|l| l.starts_with("Export failed")) {
                    return Err(format!("no error toast: {:?}", s.toasts.visible_labels()));
                }
                session_of(s)?;
                let opts = ERR_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("no options")?;
                if opts.dir.exists() {
                    let names = dir_names(&opts.dir)?;
                    if names.iter().any(|n| n.contains(".tmp-")) {
                        return Err(format!("a temp file is left: {names:?}"));
                    }
                    let want = sc.values.get("err_files").copied().unwrap_or(0) as usize;
                    if names.len() != want {
                        return Err(format!("{} holds {names:?}: the export wrote files before it failed", opts.dir.display()));
                    }
                    for name in &names {
                        let path = opts.dir.join(name);
                        if path.is_file() && std::fs::read(&path).map_err(|e| e.to_string())? != b"old" {
                            return Err(format!("{name} was replaced"));
                        }
                    }
                }
                Ok(())
            }),
            Step::Do(AppAction::CloseDialog),
            Step::WaitIdle,
        ]);
    }
    steps.extend([
        Step::Setup("a crashed export's leftovers of a process that is not running", |s, sc| {
            let dir = fresh_out_dir(sc, "_stale")?;
            let doc = doc_of(s)?;
            let zip = wte_core::export::package_name(&doc.name);
            let stage = dir.join(format!("{zip}.tmp-{DEAD_PID}.d"));
            std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
            std::fs::write(stage.join("half.png"), b"x").map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{zip}.tmp-{DEAD_PID}")), b"x").map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{}.png.tmp-{DEAD_PID}", wte_core::export::sanitize_file_stem(&doc.name))), b"x").map_err(|e| e.to_string())?;
            std::fs::write(dir.join("notes.txt"), b"keep").map_err(|e| e.to_string())?;
            if sc.values.get("expected_error_toasts").copied().unwrap_or(0) != 0 {
                return Err("an announced error toast did not come".to_owned());
            }
            err_export(s, sc, dir, wte_core::ExportPackage::Zip)?;
            sc.values.insert("expected_error_toasts", 0);
            Ok(())
        }),
        Step::Dyn("export as a zip into that folder", |_, _| {
            let opts = ERR_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("no options")?;
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::WaitIdle,
        Step::Assert("the leftovers are gone, the zip is there, other files stay", |s, sc| {
            assert_no_dialog(s, sc)?;
            let doc = doc_of(s)?;
            let names = dir_names(&with_suffix(&sc.out_stem, "_stale"))?;
            let mut want = vec![wte_core::export::package_name(&doc.name), "notes.txt".to_owned()];
            want.sort();
            if names != want {
                return Err(format!("the folder holds {names:?}, expected {want:?}"));
            }
            Ok(())
        }),
    ]);
    steps
}

/// The options of the next `export_errors` export (set by its Setup steps, read by the Dyn and Assert steps).
static ERR_OPTIONS: Mutex<Option<ExportOptions>> = Mutex::new(None);

/// Stores the options of an `export_errors` export: everything into `dir`.
fn err_export(state: &mut AppState, sc: &mut Scratch, dir: PathBuf, package: wte_core::ExportPackage) -> Result<(), String> {
    let opts = ExportOptions { dir, package, ..export_options(state, sc)? };
    sc.values.entry("err_files").or_insert(0);
    *sc.values.entry("expected_error_toasts").or_insert(0) += 1;
    *ERR_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()) = Some(opts);
    Ok(())
}

/// Two-material sample: an image stamp across the material boundary, then a stroke from the other material across it.
fn multi() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::TwoMaterial)),
        Step::WaitIdle,
        Step::Assert("two texture sets", |s, _| {
            let n = doc_of(s)?.sets.len();
            if n == 2 { Ok(()) } else { Err(format!("{n} texture sets")) }
        }),
        Step::Setup("register the sample stamp image, large stamp and brush", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.45;
            s.tools.brush.size_px = 64.0;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the stamp on the body just below the head", |s, _| {
            let xf = view_xf(s, ViewId::Main3D)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, world_screen(&xf, Vec3::new(0.0, 0.42, 0.5))?))])
        }),
        Step::WaitIdle,
        Step::Assert("the stamp has a layer in both sets and is the active layer of its home set", |s, _| {
            let doc = doc_of(s)?;
            let (id, _) = placed_stamp(s, false)?;
            let layers = doc.stamp_layers(id);
            if layers.len() != 2 {
                return Err(format!("the stamp has layers in {} set(s): {layers:?}", layers.len()));
            }
            let home = doc.stamp(id).ok_or("stamp missing")?.home_set;
            let home_layer = layers.iter().find(|(set, _)| *set == home).map(|(_, l)| *l);
            if doc.sets[home.index()].active != home_layer {
                return Err("the stamp layer is not the active layer of its home set".to_owned());
            }
            Ok(())
        }),
        Step::Setup("remember both composites and the home set's paint layers", |s, sc| {
            let doc = doc_of(s)?;
            let (id, _) = placed_stamp(s, false)?;
            let home = doc.stamp(id).ok_or("stamp missing")?.home_set;
            sc.values.insert("home", home.0 as u64);
            sc.values.insert("home_rasters", raster_count(doc, home) as u64);
            sc.values.insert("set0", set_hash(doc, 0)?);
            sc.values.insert("set1", set_hash(doc, 1)?);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("stroke from the other material across the boundary", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let home = remembered(sc, "home")?;
            // The head (set 1) or the body front (set 0) is "the other material": start there, end on the home set.
            let head = world_screen(&xf, Vec3::new(0.22, 0.86, 0.2655))?;
            let body = world_screen(&xf, Vec3::new(0.3, 0.05, 0.5))?;
            let doc = doc_of(s)?;
            let head_set = surface_set(&xf, doc, head).ok_or("the head point misses the model")?;
            let (from, to) = if head_set.0 as u64 == home { (body, head) } else { (head, body) };
            Ok(vec![drag(&xf, &[from, to])])
        }),
        Step::WaitIdle,
        // fix_export item 3: "Export this layer" on the stamp's layer writes its layer of both sets.
        Step::Dyn("Export this layer: the stamp's layer in its home set", |s, _| {
            let (id, _) = placed_stamp(s, false)?;
            let doc = doc_of(s)?;
            let home = doc.stamp(id).ok_or("stamp missing")?.home_set;
            let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home).ok_or("no home layer")?;
            Ok(vec![Step::Do(AppAction::ShowExportLayers(vec![layer]))])
        }),
        Step::WaitIdle,
        Step::Setup("into <out>_one", |s, sc| dialog_into(s, sc, "_one")),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("the stamp's layers of both sets were exported, each its layer's pixels", |s, sc| {
            assert_no_dialog(s, sc)?;
            let (id, _) = placed_stamp(s, false)?;
            let doc = doc_of(s)?;
            let layers: Vec<LayerId> = doc.stamp_layers(id).into_iter().map(|(_, l)| l).collect();
            let dir = with_suffix(&sc.out_stem, "_one");
            expect_layer_files(s, &dir, &layers)?;
            let n = dir_names(&dir)?.len();
            if n != 2 { Err(format!("{n} files for a stamp across two sets")) } else { toasts_count_right(s) }
        }),
        Step::Assert("paint in both sets, a new paint layer in the stamp's home set", |s, sc| {
            let doc = doc_of(s)?;
            let home = TextureSetId(remembered(sc, "home")? as u32);
            let rasters = raster_count(doc, home) as u64;
            if rasters != remembered(sc, "home_rasters")? + 1 {
                return Err(format!("the home set has {rasters} paint layers (expected one more than before)"));
            }
            for (set, key) in [(0usize, "set0"), (1, "set1")] {
                if set_hash(doc, set)? == remembered(sc, key)? {
                    return Err(format!("the composite of set {set} did not change"));
                }
                let painted = doc.sets[set].layers.iter().any(|l| !l.locked && l.is_raster() && has_alpha(l));
                if !painted {
                    return Err(format!("no paint layer of set {set} has paint"));
                }
            }
            Ok(())
        }),
    ]
}

/// Default content saved as `<out_stem>.wtep` and opened again.
fn roundtrip() -> Vec<Step> {
    let mut steps = default_content();
    // Round 4 (FEATURE_TEXT 8.3): the saved text has two gap values and a warp.
    steps.extend(t7_style_default_text());
    steps.extend([
        Step::Dyn("Save As the project", |_, sc| {
            Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])
        }),
        Step::WaitIdle,
        Step::Assert("the project was saved", |s, sc| {
            let path = with_suffix(&sc.out_stem, ".wtep");
            let session = session_of(s)?;
            if session.doc.is_modified() || session.path.as_deref() != Some(path.as_path()) {
                return Err(format!("the document is not saved as {} (path {:?})", path.display(), session.path));
            }
            file_non_empty(&path)
        }),
        Step::Setup("remember the composites", |s, sc| remember_composite(s, sc, "saved")),
        Step::Setup("remember the styled text's spec and layer", |s, sc| {
            let (spec, layer) = t7_text_keys(s)?;
            sc.values.insert("rt_spec", spec);
            sc.values.insert("rt_layer", layer);
            Ok(())
        }),
        Step::Dyn("open the saved project", |_, sc| {
            Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])
        }),
        Step::WaitIdle,
        Step::Assert("the reopened project equals the saved one", |s, sc| {
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, ".wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            if session.doc.history.undo_len() != 0 {
                return Err("the reopened document still has the old history".to_owned());
            }
            if composite_hash(&session.doc) != remembered(sc, "saved")? {
                return Err("the composites differ after the round trip".to_owned());
            }
            assert_default_layers(&session.doc)?;
            for stamp in &session.doc.stamps {
                if session.doc.stamp_layers(stamp.id).is_empty() {
                    return Err(format!("the stamp {:?} has no layer", stamp.name));
                }
            }
            Ok(())
        }),
        Step::Assert("the reopened text spec (gaps, warp) and its layer equal the saved ones", |s, sc| {
            let (spec, layer) = t7_text_keys(s)?;
            if spec != remembered(sc, "rt_spec")? {
                return Err(format!("the reopened text spec differs: {:?}", text_stamp(s)?.1));
            }
            if layer != remembered(sc, "rt_layer")? {
                return Err("the reopened text layer's pixels differ".to_owned());
            }
            Ok(())
        }),
        Step::Setup("remember the text layer", remember_text_layer),
        Step::Dyn("re-edit the text stamp: scale it 1.5x", |s, _| {
            let (id, _) = text_stamp(s)?;
            let placement = doc_of(s)?.stamp(id).ok_or("the text stamp is gone")?.placement;
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 1.5), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("the text stamp is still re-editable", |s, sc| {
            let rect = text_layer_rect(s)?;
            if rect_key(&rect) == remembered(sc, "text_rect")? {
                return Err(format!("the text layer did not change ({rect:?})"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Assert("one more Final changed the styled text's layer", |s, sc| {
            if t7_text_keys(s)?.1 == remembered(sc, "rt_layer")? {
                return Err("the text layer's pixels did not change".to_owned());
            }
            Ok(())
        }),
    ]);
    steps
}

/// Default content, undo everything, redo everything.
fn undo() -> Vec<Step> {
    let mut steps = default_content();
    steps.extend([
        Step::Setup("remember the final composite and the history length", |s, sc| {
            remember_composite(s, sc, "final")?;
            remember_undo(s, sc)
        }),
        Step::Dyn("undo everything", |s, _| {
            let n = doc_of(s)?.history.undo_len();
            if n == 0 {
                return Err("nothing to undo".to_owned());
            }
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undo-all the composite equals the initial one", |s, sc| {
            let doc = doc_of(s)?;
            if doc.history.undo_len() != 0 {
                return Err(format!("{} entries are left to undo", doc.history.undo_len()));
            }
            if composite_hash(doc) != remembered(sc, "initial_composite")? {
                return Err("the composite after undo-all differs from the initial one".to_owned());
            }
            assert_fresh_sphere(s, sc)
        }),
        Step::Dyn("redo everything", |s, _| {
            let n = doc_of(s)?.history.redo_len();
            Ok((0..n).map(|_| Step::Do(AppAction::Redo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after redo-all the composite equals the final one", |s, sc| {
            let doc = doc_of(s)?;
            if doc.history.undo_len() as u64 != remembered(sc, "undo")? {
                return Err(format!("{} undo entries after redo-all", doc.history.undo_len()));
            }
            if composite_hash(doc) != remembered(sc, "final")? {
                return Err("the composite after redo-all differs from the final one".to_owned());
            }
            assert_default_layers(doc)
        }),
    ]);
    steps
}

/// Import of an OBJ + MTL + two textures through the import dialog.
fn import() -> Vec<Step> {
    vec![
        Step::Setup("write the two-material model as OBJ + MTL + two PNGs", write_two_material_assets),
        Step::Dyn("New from Model", |_, sc| {
            Ok(vec![Step::Do(AppAction::NewFromModel(Some(assets_dir(sc).join("two_materials.obj"))))])
        }),
        Step::WaitIdle,
        Step::Assert("the import dialog shows 2 sets with detected textures and 2 UV thumbnails", |s, _| {
            let st = import_dialog(s)?;
            let model = st.model.as_ref().ok_or("the import dialog has no model")?;
            if model.mesh.sets.len() != 2 {
                return Err(format!("{} texture sets", model.mesh.sets.len()));
            }
            for set in &model.mesh.sets {
                let detected = set.texture.and_then(|t| model.textures.get(t as usize)).is_some_and(|t| t.image.is_some());
                if !detected {
                    return Err(format!("the texture of set {:?} was not detected", set.name));
                }
            }
            if st.uv_thumbs.len() != 2 {
                return Err(format!("{} UV thumbnails", st.uv_thumbs.len()));
            }
            Ok(())
        }),
        Step::Do(AppAction::ImportCreate),
        Step::WaitIdle,
        Step::Assert("a document with 2 sets, each Base + Paint 1, textured", |s, _| {
            let doc = doc_of(s)?;
            if doc.sets.len() != 2 {
                return Err(format!("{} texture sets", doc.sets.len()));
            }
            for set in &doc.sets {
                let names: Vec<&str> = set.layers.iter().map(|l| l.name.as_str()).collect();
                if names != ["Base", "Paint 1"] {
                    return Err(format!("set {:?} has the layers {names:?}", set.name));
                }
                if is_uniform(set.composite()) {
                    return Err(format!("the composite of set {:?} is blank", set.name));
                }
            }
            Ok(())
        }),
    ]
}

/// Import of an OBJ without MTL, with a texture chosen in the import dialog.
fn import_tex() -> Vec<Step> {
    vec![
        Step::Setup("write the seam cube as OBJ without MTL plus a separate PNG", |_, sc| {
            let dir = fresh_assets_dir(sc)?;
            let (obj, _) = samples::to_obj(&samples::seam_cube(1.5), "", &[]);
            write(&dir.join("cube_nomtl.obj"), obj.as_bytes())?;
            write(&dir.join("chosen.png"), &chosen_png()?)
        }),
        Step::Dyn("New from Model", |_, sc| {
            Ok(vec![Step::Do(AppAction::NewFromModel(Some(assets_dir(sc).join("cube_nomtl.obj"))))])
        }),
        Step::WaitIdle,
        Step::Assert("the import dialog shows 1 set", |s, _| {
            let st = import_dialog(s)?;
            let n = st.model.as_ref().ok_or("the import dialog has no model")?.mesh.sets.len();
            if n == 1 { Ok(()) } else { Err(format!("{n} texture sets")) }
        }),
        Step::Dyn("choose the texture of set 1", |_, sc| {
            Ok(vec![Step::Do(AppAction::ImportPickTexture { set: 0, path: Some(assets_dir(sc).join("chosen.png")) })])
        }),
        Step::WaitFrames(2),
        Step::Assert("the chosen image is loaded", |s, _| {
            let st = import_dialog(s)?;
            match st.choices.first() {
                Some(crate::state::SetChoiceUi::File { image: Some(_), .. }) => Ok(()),
                other => Err(format!("set 1 choice is {other:?}")),
            }
        }),
        Step::Do(AppAction::ImportCreate),
        Step::WaitIdle,
        Step::Dyn("close a warnings message", close_message),
        Step::WaitIdle,
        Step::Assert("Base equals the chosen image, which is kept byte for byte", |s, sc| {
            let doc = doc_of(s)?;
            let set = doc.sets.first().ok_or("no texture set")?;
            let base = set.layers.first().ok_or("no layer")?;
            let LayerKind::Raster(pixels) = &base.kind else { return Err("Base is not a paint layer".to_owned()) };
            let expected = chosen_image();
            if pixels.width != expected.width || pixels.height != expected.height || pixels.data != expected.data {
                return Err(format!("Base ({}x{}) differs from the chosen image", pixels.width, pixels.height));
            }
            let bytes = std::fs::read(assets_dir(sc).join("chosen.png")).map_err(|e| e.to_string())?;
            match doc.source.extras.as_slice() {
                [extra] if extra.bytes[..] == bytes[..] => Ok(()),
                [extra] => Err(format!("the stored extra {:?} differs from the PNG file", extra.original_name)),
                extras => Err(format!("{} extra source files", extras.len())),
            }
        }),
    ]
}

/// Import of a model without UVs: generated UVs, then a stroke.
fn import_nouv() -> Vec<Step> {
    vec![
        Step::Setup("write the UV-less tetrahedron as OBJ", |_, sc| {
            let dir = fresh_assets_dir(sc)?;
            let (obj, _) = samples::to_obj(&samples::no_uv_tetra(), "", &[]);
            write(&dir.join("tetra.obj"), obj.as_bytes())
        }),
        Step::Dyn("New from Model", |_, sc| Ok(vec![Step::Do(AppAction::NewFromModel(Some(assets_dir(sc).join("tetra.obj"))))])),
        Step::WaitIdle,
        // Models without UVs get the generated layout (import/auto_uv.rs) right away on New from Model.
        Step::Assert("the import dialog reports a model without UVs and a generated layout", |s, _| {
            let st = import_dialog(s)?;
            let model = st.model.as_ref().ok_or("the import dialog has no model")?;
            if model.mesh.sets.len() != 1 {
                return Err(format!("{} texture sets", model.mesh.sets.len()));
            }
            if !model.report.no_uvs || !model.report.fallback_uvs_generated {
                return Err(format!("report.no_uvs {} / fallback_uvs_generated {}", model.report.no_uvs, model.report.fallback_uvs_generated));
            }
            Ok(())
        }),
        Step::Do(AppAction::ImportReload { fallback_uvs: false }),
        Step::WaitIdle,
        Step::Assert("switching the generated layout off leaves no texture set", |s, _| {
            let st = import_dialog(s)?;
            let n = st.model.as_ref().ok_or("the import dialog has no model")?.mesh.sets.len();
            if n == 0 { Ok(()) } else { Err(format!("{n} texture sets without generated UVs")) }
        }),
        Step::Do(AppAction::ImportReload { fallback_uvs: true }),
        Step::WaitIdle,
        Step::Assert("generated UVs give one texture set", |s, _| {
            let st = import_dialog(s)?;
            let n = st.model.as_ref().ok_or("the import dialog has no model")?.mesh.sets.len();
            if n == 1 { Ok(()) } else { Err(format!("{n} texture sets after generating UVs")) }
        }),
        Step::Do(AppAction::ImportCreate),
        Step::WaitIdle,
        Step::Assert("the import warnings are not shown a second time after Create", |s, _| match &s.dialog {
            Dialog::None => Ok(()),
            other => Err(format!("the dialog {:?} is open after Create", dialog_name(other))),
        }),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Do(AppAction::BrushSizeStep(3)),
        Step::Setup("remember the composite", |s, sc| remember_composite(s, sc, "initial_composite")),
        Step::Dyn("brush stroke on the model", |s, _| model_stroke(s, ViewId::Main3D, &[[-0.35, 0.0], [0.0, 0.1], [0.3, -0.05]])),
        Step::WaitIdle,
        Step::Assert("one set, and the stroke changed pixels", |s, sc| {
            let n = doc_of(s)?.sets.len();
            if n != 1 {
                return Err(format!("{n} texture sets"));
            }
            changed_since(s, sc, "initial_composite")
        }),
        // Round 3, R3-1 / R3-3 (finding F4): a saved project of a model without UVs reopens with the same layout and
        // without a "Warnings" box (the import's informational warnings are not repeated on load).
        Step::Dyn("Save As the project", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Assert("the project was saved", |s, sc| {
            if doc_of(s)?.is_modified() {
                return Err("the document is still modified after Save As".to_owned());
            }
            file_non_empty(&with_suffix(&sc.out_stem, ".wtep"))
        }),
        Step::Setup("remember the saved composite", |s, sc| remember_composite(s, sc, "saved")),
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the reopened no-UV project shows no dialog and equals the saved one", |s, sc| {
            assert_no_dialog(s, sc)?;
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, ".wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            if session.doc.is_modified() || session.doc.history.undo_len() != 0 {
                return Err("the reopened document is modified or still has the old history".to_owned());
            }
            if composite_hash(&session.doc) != remembered(sc, "saved")? {
                return Err("the composite differs after the round trip (generated UV layout changed?)".to_owned());
            }
            Ok(())
        }),
    ]
}

/// No dialog is open; a "Warnings" (or other) message names its lines in the failure (round 3, R3-3).
fn assert_no_dialog(s: &AppState, _: &Scratch) -> Result<(), String> {
    match &s.dialog {
        Dialog::None => Ok(()),
        Dialog::Message { title, lines } => Err(format!("the message \"{title}\" is open: {lines:?}")),
        other => Err(format!("the dialog {} is open", dialog_name(other))),
    }
}

/// The window title and the unsaved check (`has_unsaved_changes`) agree with `want_modified` (R3-6).
fn assert_modified_mark(s: &AppState, want_modified: bool) -> Result<(), String> {
    let doc = doc_of(s)?;
    let unsaved = crate::io_flows::has_unsaved_changes(doc);
    let title = crate::io_flows::title_text(s);
    let star = title.starts_with(&format!("{}*", doc.name));
    if unsaved != want_modified || star != want_modified || s.last_title != title {
        return Err(format!("unsaved {unsaved}, title {title:?}, window title {:?}; expected modified {want_modified}", s.last_title));
    }
    Ok(())
}

/// R3-6: Save As, stroke, undo back to the saved state (no star), redo (star), undo, Open Sample opens without the
/// prompt. Then: sphere with a stroke, then Open Sample: the unsaved-changes prompt.
fn unsaved() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Dyn("Save As the sphere", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("saved: no star", |s, _| assert_modified_mark(s, false)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("stroke after the save", |s, _| model_stroke(s, ViewId::Main3D, &[[-0.4, 0.3], [0.4, 0.35]])),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("stroke: star", |s, _| assert_modified_mark(s, true)),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("undo back to the saved state: no star", |s, _| assert_modified_mark(s, false)),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("redo: star again", |s, _| assert_modified_mark(s, true)),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("undo again: no star", |s, _| assert_modified_mark(s, false)),
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("Open Sample replaced the saved document without the prompt", |s, _| {
            if !matches!(s.dialog, Dialog::None) {
                return Err(format!("the open dialog is {:?}", dialog_name(&s.dialog)));
            }
            let session = s.session.as_ref().ok_or("no document")?;
            if session.path.is_some() || session.doc.can_undo() {
                return Err(format!("the saved document is still open (path {:?})", session.path));
            }
            assert_modified_mark(s, false)
        }),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        // Across the upper half, so the stroke stays visible above the prompt.
        Step::Dyn("brush stroke across the sphere", |s, _| {
            model_stroke(s, ViewId::Main3D, &[[-0.55, -0.5], [0.0, -0.68], [0.55, -0.5]])
        }),
        Step::WaitIdle,
        Step::Do(AppAction::OpenSample(SampleKind::Cube)),
        Step::WaitIdle,
        Step::Assert("the unsaved prompt is open and the sphere is still the document", |s, _| {
            if !matches!(s.dialog, Dialog::Unsaved { .. }) {
                return Err(format!("the open dialog is {:?}", dialog_name(&s.dialog)));
            }
            let doc = doc_of(s)?;
            if doc.name != "sphere" || !doc.is_modified() {
                return Err(format!("the document is {:?} (modified: {})", doc.name, doc.is_modified()));
            }
            Ok(())
        }),
    ]
}

/// R7 preparation: the CLI `PATH` (a model) is imported, painted, stamped with an image and a text, and saved as
/// `<out_stem>.wtep`. The verification script then deletes the model's folder and opens the project with `open_path`.
fn import_save(has_path: bool) -> Vec<Step> {
    let mut steps = open_path(has_path);
    if !has_path {
        return steps;
    }
    steps.extend([
        Step::Assert("the bundle holds the original model and its sidecars", |s, _| {
            let source = &doc_of(s)?.source;
            if source.main.bytes.is_empty() || source.sidecars.is_empty() {
                return Err(format!("the source bundle has {} sidecar(s)", source.sidecars.len()));
            }
            Ok(())
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Setup("register the sample stamp image and the text", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.3;
            s.tools.text.content = "R7".to_owned();
            s.tools.text.em_frac = 0.12;
            s.tools.text.color = [20, 20, 20, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp", |s, _| model_click(s, ViewId::Main3D, [-0.35, 0.45])),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed", |s, _| placed_stamp(s, false).map(|_| ())),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Dyn("place the text stamp", |s, _| model_click(s, ViewId::Main3D, [0.4, 0.45])),
        Step::WaitIdle,
        Step::Assert("the text stamp was placed", |s, _| placed_stamp(s, true).map(|_| ())),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("red stroke across the model", |s, _| {
            model_stroke(s, ViewId::Main3D, &[[-0.8, 0.05], [-0.2, -0.05], [0.4, 0.05], [0.8, 0.0]])
        }),
        Step::WaitIdle,
        Step::Dyn("Save As the project", |_, sc| {
            Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])
        }),
        Step::WaitIdle,
        Step::Assert("the project was saved", |s, sc| {
            let path = with_suffix(&sc.out_stem, ".wtep");
            if doc_of(s)?.is_modified() {
                return Err("the document is still modified after Save As".to_owned());
            }
            file_non_empty(&path)
        }),
    ]);
    steps
}

/// Sphere; the Layers rows are clicked with injected pointer input: Base's lock, eye (twice) and name, Paint 1's alpha
/// lock, then Paint 1 is dragged below Base. Then an image stamp is placed and duplicated with Ctrl+J: the copy is the
/// selected stamp.
fn layers() -> Vec<Step> {
    // Clicks are 0.5 s apart (WaitFrames(30) at the runner's 60 frames per second): never a double-click.
    let pause = || Step::WaitFrames(30);
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Assert("sphere sample: 1 set with Base (locked) and Paint 1 (active)", assert_fresh_sphere),
        Step::Assert("both layer rows are drawn inside the window", assert_layer_rows_shown),
        Step::Setup("remember the history length", remember_undo),
        pause(),
        Step::Dyn("click the lock of Base", |s, _| row_click(s, 0, PanelHit::LayerLock)),
        Step::WaitIdle,
        Step::Assert("Base is unlocked", |s, _| layer_is(s, 0, "unlocked", |l| !l.locked)),
        pause(),
        Step::Dyn("click the eye of Base", |s, _| row_click(s, 0, PanelHit::LayerEye)),
        Step::WaitIdle,
        Step::Assert("Base is hidden", |s, _| layer_is(s, 0, "hidden", |l| !l.visible)),
        pause(),
        Step::Dyn("click the eye of Base again", |s, _| row_click(s, 0, PanelHit::LayerEye)),
        Step::WaitIdle,
        Step::Assert("Base is visible", |s, _| layer_is(s, 0, "visible", |l| l.visible)),
        pause(),
        Step::Dyn("click the alpha lock of Paint 1", |s, _| row_click(s, 1, PanelHit::LayerAlphaLock)),
        Step::WaitIdle,
        Step::Assert("Paint 1 has the alpha lock", |s, _| layer_is(s, 1, "alpha locked", |l| l.alpha_lock)),
        pause(),
        Step::Dyn("click the name of Base", |s, _| row_click(s, 0, PanelHit::LayerName)),
        Step::WaitIdle,
        Step::Assert("Base is the active layer", |s, _| {
            let set = &doc_of(s)?.sets[0];
            if set.active == Some(set.layers[0].id) { Ok(()) } else { Err(format!("the active layer is {:?}", set.active)) }
        }),
        Step::Assert("4 undo entries: unlock, hide, show, alpha lock (selecting is none)", |s, sc| undo_since(s, sc, 4)),
        pause(),
        Step::Dyn("drag Paint 1 by its name below Base", |s, _| {
            let doc = doc_of(s)?;
            let (base, paint) = (doc.sets[0].layers[0].id, doc.sets[0].layers[1].id);
            let from = panel_rect(s, PanelHit::LayerName(paint))?.center();
            let target = panel_rect(s, PanelHit::LayerRow(base))?;
            let to = egui::pos2(from.x, target.bottom() - 4.0);
            let path: Vec<egui::Pos2> = (0..=10).map(|k| from + (to - from) * (k as f32 / 10.0)).collect();
            Ok(vec![Step::ScreenPointer { path }])
        }),
        Step::WaitIdle,
        Step::Assert("Paint 1 is now the bottom layer", |s, _| {
            let names: Vec<&str> = doc_of(s)?.sets[0].layers.iter().map(|l| l.name.as_str()).collect();
            if names == ["Paint 1", "Base"] { Ok(()) } else { Err(format!("the layers are {names:?} (bottom first)")) }
        }),
        // Duplicating a stamp layer moves the selection to the copy.
        Step::Setup("register the sample stamp image", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp on the sphere", |s, _| model_click(s, ViewId::Main3D, [-0.1, 0.0])),
        Step::WaitIdle,
        Step::Setup("remember the placed stamp", |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("stamp", id.0);
            Ok(())
        }),
        Step::Key(egui::Modifiers::CTRL.plus(egui::Modifiers::COMMAND), egui::Key::J),
        Step::WaitIdle,
        Step::Assert("Ctrl+J: the copy is selected and is the active layer", |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            if id.0 == remembered(sc, "stamp")? {
                return Err("the original stamp is still selected after Duplicate".to_owned());
            }
            let doc = doc_of(s)?;
            let active = doc.sets[0].active.ok_or("no active layer")?;
            if doc.stamp_of_layer(active) != Some(id) {
                return Err(format!("the active layer {active:?} is not the selected stamp's layer"));
            }
            if doc.stamps.len() != 2 {
                return Err(format!("{} stamps after Duplicate", doc.stamps.len()));
            }
            Ok(())
        }),
        // R3-2 (finding F2): Duplicate of a layer whose opacity is not a whole percent is one undo entry, and undoing it
        // warns about nothing (the idle opacity slider used to write the rounded value back as a Final edit).
        Step::Setup("make Paint 1 active with opacity 21.33 %", |s, sc| {
            let session = s.session.as_mut().ok_or("no document")?;
            session.selected_stamp = None;
            let doc = &mut session.doc;
            let paint = doc.sets[0].layers.iter().find(|l| l.name == "Paint 1").ok_or("no layer Paint 1")?.id;
            doc.sets[0].active = Some(paint);
            doc.set_layer_opacity(paint, 0.213_319_3, wte_core::Commit::Final).map_err(|e| e.to_string())?;
            sc.values.insert("paint", paint.0);
            remember_undo(s, sc)
        }),
        Step::WaitFrames(5),
        Step::Assert("the idle opacity slider leaves 21.33 % alone", |s, sc| {
            undo_since(s, sc, 0)?;
            let opacity = doc_of(s)?.layer(LayerId(remembered(sc, "paint")?)).ok_or("Paint 1 is gone")?.opacity;
            if opacity == 0.213_319_3 { Ok(()) } else { Err(format!("the opacity is {opacity}")) }
        }),
        Step::Dyn("duplicate Paint 1", |_, sc| Ok(vec![Step::Do(AppAction::DuplicateLayer(LayerId(remembered(sc, "paint")?)))])),
        Step::WaitIdle,
        Step::WaitFrames(5),
        Step::Assert("Duplicate adds exactly one undo entry", |s, sc| {
            undo_since(s, sc, 1)?;
            match doc_of(s)?.undo_label() {
                Some("Duplicate Layer") => Ok(()),
                other => Err(format!("the top undo entry is {other:?}")),
            }
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("undoing Duplicate warns about nothing and adds no entry", |s, sc| {
            undo_since(s, sc, 0)?;
            let labels = s.toasts.visible_labels();
            if let Some(bad) = labels.iter().find(|t| t.contains("Unknown") || t.contains("Layer Opacity")) {
                return Err(format!("toast {bad:?} after the undo (toasts: {labels:?})"));
            }
            if doc_of(s)?.can_redo() { Ok(()) } else { Err("the redo of Duplicate was lost".to_owned()) }
        }),
        // R3-10 (finding F9): with 6 layers the footer buttons stay on screen, outside the list's scroll area.
        Step::Do(AppAction::AddPaintLayer),
        Step::Do(AppAction::AddPaintLayer),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("6 layers: the Delete button is on screen below the scrolling list", |s, sc| {
            let count = doc_of(s)?.sets[0].layers.len();
            if count != 6 {
                return Err(format!("{count} layers, expected 6"));
            }
            assert_side_panel_layout(s, sc)
        }),
    ]
}

/// R3-10 (decision 2 of round 3, finding F9): the Color section's area lies inside the side panel; the Layers footer
/// and its Delete button lie inside the side panel, below the list's scroll area (not in it). In a window of 1280x800
/// or more (side panel at least 740 points high) the whole Color section is on screen, nothing of it scrolled away.
fn assert_side_panel_layout(s: &AppState, _: &Scratch) -> Result<(), String> {
    let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?.expand(0.5);
    let color = panel_rect(s, PanelHit::ColorSection)?;
    let content = panel_rect(s, PanelHit::ColorContent)?;
    let footer = panel_rect(s, PanelHit::LayersFooter)?;
    let list = panel_rect(s, PanelHit::LayersList)?;
    let settings = panel_rect(s, PanelHit::SettingsArea)?;
    log::info!("side panel {side:?}: settings {settings:?}, color {color:?} (content {content:?}), list {list:?}, footer {footer:?}");
    if !side.contains_rect(color) {
        return Err(format!("the Color section {color:?} is not inside the side panel {side:?}"));
    }
    if settings.bottom() > color.top() + 0.5 || color.bottom() > list.top() + 0.5 {
        return Err(format!("the areas overlap: settings {settings:?}, color {color:?}, layers list {list:?}"));
    }
    if !side.contains_rect(footer) || footer.top() < list.bottom() - 0.5 {
        return Err(format!("the Layers footer {footer:?} is not below the list {list:?} inside the side panel {side:?}"));
    }
    if let Some(delete) = s.panel_rects.get(&PanelHit::LayersDelete) {
        if !side.contains_rect(*delete) || delete.intersects(list) {
            return Err(format!("the Delete button {delete:?} is off the panel {side:?} or inside the list {list:?}"));
        }
    } else if doc_of(s)?.sets.iter().any(|set| set.active.is_some()) {
        return Err("the Delete button is not drawn".to_owned());
    }
    if side.height() >= 740.0 && !color.expand(0.5).contains_rect(content) {
        return Err(format!("the Color section is cut off: content {content:?} in the area {color:?}"));
    }
    // The list shows at least two whole rows (46 pt each) at 1280x800 and above.
    if side.height() >= 740.0 && list.height() < 92.0 {
        return Err(format!("the layer list is only {:.0} pt high: {list:?}", list.height()));
    }
    Ok(())
}

/// The layer at `index` (bottom first) of set 0 satisfies `check`.
fn layer_is(state: &AppState, index: usize, what: &str, check: fn(&Layer) -> bool) -> Result<(), String> {
    let layer = doc_of(state)?.sets[0].layers.get(index).ok_or_else(|| format!("no layer {index}"))?;
    if check(layer) { Ok(()) } else { Err(format!("{} is not {what}", layer.name)) }
}

/// The screen rect of a Layers control in the last frame.
fn panel_rect(state: &AppState, hit: PanelHit) -> Result<egui::Rect, String> {
    state.panel_rects.get(&hit).copied().ok_or_else(|| format!("{hit:?} is not drawn"))
}

/// A click on a control of the row of the layer at `index` (bottom first) of set 0.
fn row_click(state: &AppState, index: usize, control: fn(LayerId) -> PanelHit) -> Result<Vec<Step>, String> {
    let layer = doc_of(state)?.sets[0].layers.get(index).ok_or_else(|| format!("no layer {index}"))?.id;
    Ok(vec![Step::ScreenPointer { path: vec![panel_rect(state, control(layer))?.center()] }])
}

/// Every row of set 0 is drawn and lies above the status bar (the 3D view's bottom edge).
fn assert_layer_rows_shown(state: &AppState, _scratch: &Scratch) -> Result<(), String> {
    let bottom = state.view_rects.get(&ViewId::Main3D).ok_or("the 3D view is not shown")?.bottom();
    for layer in &doc_of(state)?.sets[0].layers {
        let row = panel_rect(state, PanelHit::LayerRow(layer.id))?;
        if row.bottom() > bottom + 1.0 || row.top() < 0.0 {
            return Err(format!("the row of {} is at {row:?}, outside the window above {bottom}", layer.name));
        }
    }
    Ok(())
}

/// The CLI `PATH` is opened by the app; an import dialog is confirmed.
fn open_path(has_path: bool) -> Vec<Step> {
    if !has_path {
        return vec![Step::Assert("a PATH argument was given", |_, _| {
            Err("the open_path scenario needs a model or .wtep PATH on the command line".to_owned())
        })];
    }
    vec![
        Step::WaitIdle,
        Step::Dyn("confirm the import dialog if it is open", |s, _| {
            Ok(if matches!(s.dialog, Dialog::Import(_)) { vec![Step::Do(AppAction::ImportCreate), Step::WaitIdle] } else { Vec::new() })
        }),
        Step::WaitIdle,
        Step::Assert("a document is open", |s, _| {
            let doc = doc_of(s).map_err(|_| "no document was opened from the PATH argument".to_owned())?;
            if doc.sets.is_empty() { Err("the document has no texture set".to_owned()) } else { Ok(()) }
        }),
        // Round 3, R3-3 (finding F4): neither an import (its warnings are in the import dialog) nor the load of a
        // saved project opens a "Warnings" box on its own.
        Step::Assert("no Warnings box after opening", assert_no_dialog),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// Asserts
// ---------------------------------------------------------------------------------------------------------------------

fn assert_no_session(state: &AppState, _scratch: &Scratch) -> Result<(), String> {
    if state.session.is_some() { Err("a document is open".to_owned()) } else { Ok(()) }
}

fn assert_fresh_sphere(state: &AppState, _scratch: &Scratch) -> Result<(), String> {
    let doc = state.doc().ok_or("no session")?;
    if doc.sets.len() != 1 {
        return Err(format!("expected 1 texture set, found {}", doc.sets.len()));
    }
    let set = &doc.sets[0];
    if set.layers.len() != 2 {
        return Err(format!("expected exactly 2 layers, found {}", set.layers.len()));
    }
    let (base, paint) = (&set.layers[0], &set.layers[1]);
    if base.name != "Base" || !base.locked || !base.is_raster() {
        return Err(format!("bottom layer is {:?} (locked: {}, kind: {}), expected a locked raster \"Base\"", base.name, base.locked, base.kind_name()));
    }
    if paint.name != "Paint 1" || !paint.is_raster() {
        return Err(format!("top layer is {:?} ({}), expected the raster \"Paint 1\"", paint.name, paint.kind_name()));
    }
    if set.active != Some(paint.id) {
        return Err("\"Paint 1\" is not the active layer".to_owned());
    }
    Ok(())
}

/// Set 0 holds Base (locked), Paint 1, one Stamp and one Text layer, bottom to top.
fn assert_default_layers(doc: &Document) -> Result<(), String> {
    let set = doc.sets.first().ok_or("no texture set")?;
    let kinds: Vec<(&str, &str, bool)> = set.layers.iter().map(|l| (l.name.as_str(), l.kind_name(), l.locked)).collect();
    let ok = matches!(
        kinds.as_slice(),
        [("Base", "paint", true), ("Paint 1", "paint", false), (_, "stamp", _), (_, "text", _)]
    );
    if !ok {
        return Err(format!("set 0 has the layers (name, kind, locked) {kinds:?}"));
    }
    Ok(())
}

fn assert_default_content(state: &AppState, scratch: &Scratch) -> Result<(), String> {
    let doc = doc_of(state)?;
    if !doc.is_modified() {
        return Err("the document is not modified".to_owned());
    }
    assert_default_layers(doc)?;
    if base_hash(doc)? != remembered(scratch, "initial_base")? {
        return Err("the Base layer's pixels changed".to_owned());
    }
    if composite_hash(doc) == remembered(scratch, "initial_composite")? {
        return Err("the composite equals the initial one".to_owned());
    }
    Ok(())
}

/// Texels that differ between the composite and Base fall into at least two of the cube's six UV islands (the 3x2
/// atlas of `samples::seam_cube`).
fn assert_cube_islands(state: &AppState, _scratch: &Scratch) -> Result<(), String> {
    let doc = doc_of(state)?;
    let set = doc.sets.first().ok_or("no texture set")?;
    let base = set.layers.first().ok_or("no layer")?;
    let LayerKind::Raster(base) = &base.kind else { return Err("Base is not a paint layer".to_owned()) };
    let composite = set.composite();
    let (w, h) = (composite.width, composite.height);
    let mut islands = [0u32; 6];
    for y in 0..h {
        let (a, b) = (composite.row(y), base.row(y));
        for x in 0..w {
            let i = 4 * x as usize;
            if a[i..i + 4] != b[i..i + 4] {
                let (col, row) = ((x * 3 / w).min(2), (y * 2 / h).min(1));
                islands[(row * 3 + col) as usize] += 1;
            }
        }
    }
    let painted = islands.iter().filter(|&&n| n >= 16).count();
    if painted < 2 {
        return Err(format!("changed texels per island (+X, -X, +Y, -Y, +Z, -Z): {islands:?}"));
    }
    log::info!("changed texels per cube island (+X, -X, +Y, -Y, +Z, -Z): {islands:?}");
    Ok(())
}

fn assert_exported(state: &AppState, scratch: &Scratch) -> Result<(), String> {
    if !matches!(state.dialog, Dialog::None) {
        return Err(format!("a dialog is open: {}", dialog_name(&state.dialog)));
    }
    let doc = doc_of(state)?;
    let opts = export_options(state, scratch)?;
    let plan = wte_core::export::plan(&doc.snapshot(), &opts);
    if plan.is_empty() {
        return Err("export::plan is empty".to_owned());
    }
    for path in &plan {
        file_non_empty(path)?;
    }
    // Exactly the planned files, each holding what the app shows (EXPORT_SELECT_PLAN 7.2: existence alone let a blank
    // file or the wrong layer pass).
    let files = read_export_dir(doc, &opts)?;
    let summary = verify_export(doc, &opts, &files)?;
    log::info!("export_run: {summary}");
    let names: Vec<String> = plan.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
    let per_layer = |kind: &str| names.iter().filter(|n| n.ends_with(".png") && n.contains(&format!("_{kind}_"))).count();
    if per_layer("stamp") != 1 || per_layer("text") != 1 {
        return Err(format!("expected one stamp and one text layer file, the files are {names:?}"));
    }
    for ext in ["ora", "psd"] {
        if !names.iter().any(|n| n.ends_with(&format!(".{ext}"))) {
            return Err(format!("no .{ext} file in {names:?}"));
        }
    }
    log::info!("exported files: {names:?}");
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Setups and remembered values
// ---------------------------------------------------------------------------------------------------------------------

fn remember_initial(state: &mut AppState, scratch: &mut Scratch) -> Result<(), String> {
    let doc = doc_of(state)?;
    let (composite, base) = (composite_hash(doc), base_hash(doc)?);
    scratch.values.insert("initial_composite", composite);
    scratch.values.insert("initial_base", base);
    Ok(())
}

fn remember_composite(state: &mut AppState, scratch: &mut Scratch, key: &'static str) -> Result<(), String> {
    let hash = composite_hash(doc_of(state)?);
    scratch.values.insert(key, hash);
    Ok(())
}

fn remember_undo(state: &mut AppState, scratch: &mut Scratch) -> Result<(), String> {
    let n = doc_of(state)?.history.undo_len();
    scratch.values.insert("undo", n as u64);
    Ok(())
}

/// Remembers the cache rect of the text stamp's layer and the history length.
fn remember_text_layer(state: &mut AppState, scratch: &mut Scratch) -> Result<(), String> {
    let rect = text_layer_rect(state)?;
    scratch.values.insert("text_rect", rect_key(&rect));
    remember_undo(state, scratch)
}

/// Registers the built-in sample stamp image as a blob and makes it the Stamp tool's source.
fn setup_stamp_image(state: &mut AppState, _scratch: &mut Scratch) -> Result<(), String> {
    let img = samples::sample_stamp_image(STAMP_SIDE);
    let png = wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).map_err(|e| e.to_string())?;
    let doc = state.doc_mut().ok_or("no session")?;
    let blob = doc.add_image_blob("logo.png", Arc::from(png)).map_err(|e| e.to_string())?;
    state.tools.stamp.source = Some(blob);
    state.tools.stamp.source_name = STAMP_NAME.to_owned();
    Ok(())
}

fn remembered(scratch: &Scratch, key: &str) -> Result<u64, String> {
    scratch.values.get(key).copied().ok_or_else(|| format!("no remembered value \"{key}\""))
}

fn changed_since(state: &AppState, scratch: &Scratch, key: &str) -> Result<(), String> {
    if composite_hash(doc_of(state)?) == remembered(scratch, key)? {
        Err(format!("the composite did not change since \"{key}\""))
    } else {
        Ok(())
    }
}

/// The history has exactly `extra` more undo entries than remembered by `remember_undo`.
fn undo_since(state: &AppState, scratch: &Scratch, extra: u64) -> Result<(), String> {
    let doc = doc_of(state)?;
    let now = doc.history.undo_len() as u64;
    let expected = remembered(scratch, "undo")? + extra;
    if doc.has_pending_edit() {
        return Err("an edit is still pending".to_owned());
    }
    if now != expected { Err(format!("{now} undo entries, expected {expected}")) } else { Ok(()) }
}

// ---------------------------------------------------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------------------------------------------------

fn session_of(state: &AppState) -> Result<&crate::state::Session, String> {
    state.session.as_ref().ok_or_else(|| "no document is open".to_owned())
}

fn doc_of(state: &AppState) -> Result<&Document, String> {
    session_of(state).map(|s| &s.doc)
}

fn tool_is(state: &AppState, tool: ToolKind) -> Result<(), String> {
    if state.tool == tool { Ok(()) } else { Err(format!("the tool is {:?}, expected {tool:?}", state.tool)) }
}

fn shading_is(state: &AppState, shading: ShadingMode) -> Result<(), String> {
    if state.flags.shading == shading { Ok(()) } else { Err(format!("the shading is {:?}, expected {shading:?}", state.flags.shading)) }
}

fn dialog_name(dialog: &Dialog) -> &'static str {
    match dialog {
        Dialog::None => "none",
        Dialog::Import(_) => "Import",
        Dialog::Unsaved { .. } => "Unsaved",
        Dialog::Export(_) => "Export",
        Dialog::Message { .. } => "Message",
        Dialog::About => "About",
        Dialog::Shortcuts => "Shortcuts",
        Dialog::CloseOpenings(_) => "CloseOpenings",
    }
}

fn import_dialog(state: &AppState) -> Result<&crate::state::ImportDialogState, String> {
    match &state.dialog {
        Dialog::Import(st) if st.loading => Err("the import dialog is still loading".to_owned()),
        Dialog::Import(st) => match &st.error {
            Some(e) => Err(format!("the import dialog shows an error: {e}")),
            None => Ok(st),
        },
        other => Err(format!("the import dialog is not open (dialog: {})", dialog_name(other))),
    }
}

/// Closes a Message dialog (import warnings) like the user would before painting.
fn close_message(state: &AppState, _scratch: &Scratch) -> Result<Vec<Step>, String> {
    Ok(match &state.dialog {
        Dialog::Message { title, lines } => {
            log::info!("closing the message \"{title}\": {lines:?}");
            vec![Step::Do(AppAction::CloseDialog)]
        }
        _ => Vec::new(),
    })
}

/// The selected stamp; `text` = it must be a text stamp, else an image stamp.
fn placed_stamp(state: &AppState, text: bool) -> Result<(StampId, Placement), String> {
    let session = session_of(state)?;
    let id = session.selected_stamp.ok_or("no stamp is selected")?;
    let stamp = session.doc.stamp(id).ok_or("the selected stamp does not exist")?;
    let is_text = matches!(stamp.source, StampSource::Text(_));
    if is_text != text {
        return Err(format!("the selected stamp {:?} is {} stamp", stamp.name, if is_text { "a text" } else { "an image" }));
    }
    if session.doc.stamp_layers(id).is_empty() {
        return Err(format!("the stamp {:?} has no layer", stamp.name));
    }
    // Fix round 2, item 4: "placed" means visible pixels, not just an (empty) layer.
    let texels: usize = session
        .doc
        .stamp_layers(id)
        .into_iter()
        .filter_map(|(_, l)| session.doc.layer(l))
        .map(|l| match &l.kind {
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => {
                cache.parts().iter().map(|p| p.data.as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count()).sum()
            }
            LayerKind::Raster(_) => 0,
        })
        .sum();
    if texels == 0 {
        return Err(format!("the stamp {:?} covers no texels", stamp.name));
    }
    log::info!("stamp {:?}: {texels} texels", stamp.name);
    Ok((id, stamp.placement))
}

/// The (only) text stamp and its spec.
fn text_stamp(state: &AppState) -> Result<(StampId, wte_core::TextSpec), String> {
    doc_of(state)?
        .stamps
        .iter()
        .find_map(|s| match &s.source {
            StampSource::Text(spec) => Some((s.id, spec.clone())),
            StampSource::Image { .. } => None,
        })
        .ok_or_else(|| "there is no text stamp".to_owned())
}

/// Cache rect of the text stamp's layer in its home set.
fn text_layer_rect(state: &AppState) -> Result<wte_core::IRect, String> {
    let (id, _) = text_stamp(state)?;
    let doc = doc_of(state)?;
    let home = doc.stamp(id).ok_or("the text stamp is gone")?.home_set;
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home).ok_or("the text stamp has no layer")?;
    match &doc.layer(layer).ok_or("the text layer is gone")?.kind {
        LayerKind::Text { cache, .. } => Ok(cache.bounds()),
        other => Err(format!("the text stamp's layer is {other:?}")),
    }
}

fn rect_key(r: &wte_core::IRect) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [r.x0, r.y0, r.x1, r.y1] {
        h = wte_core::fnv1a64(h, &v.to_le_bytes());
    }
    h
}

/// The selected stamp and the larger side of its placement (world units or texels).
fn selected_size(state: &AppState) -> Result<(StampId, f32), String> {
    let session = session_of(state)?;
    let id = session.selected_stamp.ok_or("no stamp is selected")?;
    let size = match session.doc.stamp(id).ok_or("the selected stamp does not exist")?.placement {
        Placement::Projected(p) => p.half_size.max_element(),
        Placement::Uv(u) => u.size_px.max_element(),
    };
    Ok((id, size))
}

/// The placement scaled by `f` about its centre (a projected decal also scales its depth).
fn scaled(placement: &Placement, f: f32) -> Placement {
    match *placement {
        Placement::Projected(mut p) => {
            p.half_size *= f;
            p.depth_front *= f;
            p.depth_back *= f;
            Placement::Projected(p)
        }
        Placement::Uv(mut u) => {
            u.size_px *= f;
            Placement::Uv(u)
        }
    }
}

fn composite_hash(doc: &Document) -> u64 {
    doc.sets.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, set| {
        wte_core::fnv1a64(h, &wte_core::testutil::hash_pixels(set.composite()).to_le_bytes())
    })
}

fn set_hash(doc: &Document, set: usize) -> Result<u64, String> {
    doc.sets.get(set).map(|s| wte_core::testutil::hash_pixels(s.composite())).ok_or_else(|| format!("no texture set {set}"))
}

fn base_hash(doc: &Document) -> Result<u64, String> {
    let base = doc.sets.first().and_then(|s| s.layers.first()).ok_or("no Base layer")?;
    match &base.kind {
        LayerKind::Raster(pixels) => Ok(wte_core::testutil::hash_pixels(pixels)),
        _ => Err("the bottom layer is not a paint layer".to_owned()),
    }
}

fn raster_count(doc: &Document, set: TextureSetId) -> usize {
    doc.set(set).map_or(0, |s| s.layers.iter().filter(|l| l.is_raster()).count())
}

fn has_alpha(layer: &Layer) -> bool {
    match &layer.kind {
        LayerKind::Raster(pixels) => pixels.data.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
        _ => false,
    }
}

/// True when every pixel has the same colour.
fn is_uniform(buf: &PixelBuf) -> bool {
    buf.data.as_chunks::<4>().0.iter().all(|px| px[..] == buf.data[..4])
}

fn color_distance(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(y)).max().unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------------------------------
// Pointer paths
// ---------------------------------------------------------------------------------------------------------------------

fn view_xf(state: &AppState, view: ViewId) -> Result<ViewXform, String> {
    ViewXform::for_view(state, view).ok_or_else(|| format!("the view {view:?} is not laid out"))
}

/// Screen position -> normalized view coordinates.
fn norm(xf: &ViewXform, pos: egui::Pos2) -> [f32; 2] {
    [(pos.x - xf.rect.min.x) / xf.rect.width().max(1.0), (pos.y - xf.rect.min.y) / xf.rect.height().max(1.0)]
}

fn world_screen(xf: &ViewXform, p: Vec3) -> Result<egui::Pos2, String> {
    xf.world_to_screen(p).ok_or_else(|| format!("the point {p:?} is behind the camera"))
}

/// UV (2D view) -> screen.
fn uv_screen(xf: &ViewXform, uv: [f32; 2]) -> egui::Pos2 {
    let tex = Vec2::new(xf.tex_size[0] as f32, xf.tex_size[1] as f32);
    xf.px_to_screen(Vec2::from_array(uv) * tex)
}

/// The texture set under a screen position of a 3D view.
fn surface_set(xf: &ViewXform, doc: &Document, pos: egui::Pos2) -> Option<TextureSetId> {
    match xf.surface_at(doc, TextureSetId(0), pos) {
        crate::tools::Surface::Model(hit) => hit.texture_set,
        _ => None,
    }
}

/// The model's screen disc in a 3D view: the projected bounds centre and the screen length of half the smallest
/// bounds extent (the sphere's radius for the sphere sample).
fn model_disc(state: &AppState, view: ViewId) -> Result<(ViewXform, egui::Pos2, f32), String> {
    let xf = view_xf(state, view)?;
    let bounds = doc_of(state)?.mesh.bounds;
    let center = bounds.center();
    let half = (bounds.max - bounds.min).min_element() * 0.5;
    let c = world_screen(&xf, center)?;
    let r = xf.world_len_on_screen(center, half).ok_or("the model is behind the camera")?;
    Ok((xf, c, r))
}

/// Disc coordinates (units of the disc radius, y down) -> screen.
fn disc_point(c: egui::Pos2, r: f32, p: [f32; 2]) -> egui::Pos2 {
    c + egui::vec2(p[0], p[1]) * r
}

/// Points along a polyline, at most `PATH_STEP` apart.
fn densify(points: &[egui::Pos2]) -> Vec<egui::Pos2> {
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let n = ((b - a).length() / PATH_STEP).ceil().max(1.0) as usize;
        out.extend((0..n).map(|k| a.lerp(b, k as f32 / n as f32)));
    }
    if let Some(last) = points.last() {
        out.push(*last);
    }
    out
}

/// A left-button drag along a polyline (screen positions) in the view of `xf`.
fn drag(xf: &ViewXform, points: &[egui::Pos2]) -> Step {
    Step::Pointer {
        view: xf.view,
        path: densify(points).into_iter().map(|p| norm(xf, p)).collect(),
        button: egui::PointerButton::Primary,
        mods: egui::Modifiers::NONE,
        points_per_frame: POINTS_PER_FRAME,
    }
}

fn click(view: ViewId, at: [f32; 2]) -> Step {
    Step::Click { view, at, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }
}

/// A stroke over the model along disc coordinates.
fn model_stroke(state: &AppState, view: ViewId, points: &[[f32; 2]]) -> Result<Vec<Step>, String> {
    let (xf, c, r) = model_disc(state, view)?;
    let screen: Vec<egui::Pos2> = points.iter().map(|p| disc_point(c, r, *p)).collect();
    Ok(vec![drag(&xf, &screen)])
}

/// A click on the model at disc coordinates.
fn model_click(state: &AppState, view: ViewId, p: [f32; 2]) -> Result<Vec<Step>, String> {
    let (xf, c, r) = model_disc(state, view)?;
    Ok(vec![click(view, norm(&xf, disc_point(c, r, p)))])
}

/// A stroke on the texture of the 2D view along UV coordinates.
fn uv_stroke(state: &AppState, uvs: &[[f32; 2]]) -> Result<Vec<Step>, String> {
    let xf = view_xf(state, ViewId::Main2D)?;
    let screen: Vec<egui::Pos2> = uvs.iter().map(|uv| uv_screen(&xf, *uv)).collect();
    Ok(vec![drag(&xf, &screen)])
}

/// The gizmo of the selected stamp in the 3D view.
fn selected_gizmo(state: &AppState) -> Result<(ViewXform, GizmoShape), String> {
    let (id, placement) = placed_stamp(state, false)?;
    let xf = view_xf(state, ViewId::Main3D)?;
    let shape = gizmo::gizmo_shape(&placement, &xf).ok_or_else(|| format!("the gizmo of stamp {id:?} is not visible"))?;
    Ok((xf, shape))
}

/// Stroke across the cube: horizontally across the vertical edge below the corner nearest to the camera, up through
/// that corner and onto the top face.
fn corner_stroke(state: &AppState, _scratch: &Scratch) -> Result<Vec<Step>, String> {
    let xf = view_xf(state, ViewId::Main3D)?;
    let bounds = doc_of(state)?.mesh.bounds;
    let eye = xf.camera.eye();
    let corner = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { bounds.min.x } else { bounds.max.x },
                if i & 2 == 0 { bounds.min.y } else { bounds.max.y },
                if i & 4 == 0 { bounds.min.z } else { bounds.max.z },
            )
        })
        .min_by(|a, b| a.distance(eye).total_cmp(&b.distance(eye)))
        .ok_or("no bounds corner")?;
    let c = world_screen(&xf, corner)?;
    let (_, _, r) = model_disc(state, ViewId::Main3D)?;
    // The vertical edge below the corner runs from `c` to the projection of the corner half an edge length lower.
    let below = world_screen(&xf, corner - Vec3::Y * (bounds.max.y - bounds.min.y) * 0.5)?;
    // The centre of the face at the corner's height (the top face when the camera looks down).
    let face = world_screen(&xf, Vec3::new(bounds.center().x, corner.y, bounds.center().z))?;
    let side = (r * 0.45).max(30.0);
    let points = [below + egui::vec2(-side, 0.0), below + egui::vec2(side, 0.0), c, c + (face - c) * 0.6];
    Ok(vec![drag(&xf, &points)])
}

// ---------------------------------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------------------------------

/// `path` + `suffix`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn assets_dir(scratch: &Scratch) -> PathBuf {
    with_suffix(&scratch.out_stem, "_assets")
}

/// `<out_stem>_assets`, emptied.
fn fresh_assets_dir(scratch: &Scratch) -> Result<PathBuf, String> {
    let dir = assets_dir(scratch);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn file_non_empty(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.len() > 0 => Ok(()),
        Ok(_) => Err(format!("{} is empty", path.display())),
        Err(e) => Err(format!("{} is missing: {e}", path.display())),
    }
}

fn png_of(img: &PixelBuf) -> Result<Vec<u8>, String> {
    wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Balanced).map_err(|e| e.to_string())
}

/// `two_material_model` as `two_materials.obj` + `.mtl` + `body.png` + `head.png`.
fn write_two_material_assets(_state: &mut AppState, scratch: &mut Scratch) -> Result<(), String> {
    let dir = fresh_assets_dir(scratch)?;
    let names = vec!["body.png".to_owned(), "head.png".to_owned()];
    let (obj, mtl) = samples::to_obj(&samples::two_material_model(), "two_materials.mtl", &names);
    write(&dir.join("two_materials.obj"), obj.as_bytes())?;
    write(&dir.join("two_materials.mtl"), mtl.as_bytes())?;
    write(&dir.join("body.png"), &png_of(&samples::checker_texture(ASSET_TEXTURE_SIDE, ASSET_TEXTURE_SIDE, 6))?)?;
    write(&dir.join("head.png"), &png_of(&samples::checker_texture(ASSET_TEXTURE_SIDE, ASSET_TEXTURE_SIDE, 10))?)
}

/// The texture the `import_tex` scenario chooses by hand.
fn chosen_image() -> PixelBuf {
    samples::checker_texture(ASSET_TEXTURE_SIDE, ASSET_TEXTURE_SIDE, 4)
}

fn chosen_png() -> Result<Vec<u8>, String> {
    png_of(&chosen_image())
}

/// The options of the `export_run` scenario: everything into `<out_stem>_export`.
fn export_options(state: &AppState, scratch: &Scratch) -> Result<ExportOptions, String> {
    let doc = doc_of(state)?;
    Ok(ExportOptions {
        dir: with_suffix(&scratch.out_stem, "_export"),
        base_name: doc.name.clone(),
        sets: None,
        flat: Some(FlatFormat::Png(PngLevel::Balanced)),
        per_layer_png: true,
        ora: true,
        psd: true,
        include_hidden: false,
        dilate_px: 0,
        layers: None,
        package: wte_core::ExportPackage::Files,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Compatibility harness: `workflow_path` and `fuzz_path` (any OBJ / glTF / GLB / .wtep given as PATH)
// ---------------------------------------------------------------------------------------------------------------------

/// Step and run timeouts of the scenarios that need more than the runner's defaults (30 s / 180 s): real models take
/// long to import, export and save, and a fuzz run has hundreds of actions. `None` = the runner's defaults. The
/// environment variables `WTE_SELFTEST_STEP_TIMEOUT` / `WTE_SELFTEST_RUN_TIMEOUT` (seconds) override both.
pub fn timeouts(name: &str) -> Option<(Duration, Duration)> {
    match name {
        "workflow_path" | "fuzz_path" | "brush_types_path" | "text_preview_path" | "final_bake_async" | "wrap_surface"
        | "export_flow" | "export_unsettled" => {
            Some((Duration::from_secs(300), Duration::from_secs(3600)))
        }
        _ => None,
    }
}

/// Screen grid the harness raycasts to find points on the model (columns x rows over the inset 3D view).
const GRID_COLS: usize = 40;
const GRID_ROWS: usize = 28;
/// Coarser grid of the fuzz run (one raycast pass per action).
const FUZZ_COLS: usize = 16;
const FUZZ_ROWS: usize = 12;
/// Fraction of the view rect left out on every side (view overlays, gizmo knobs).
const GRID_INSET: f32 = 0.08;
const GREEN: [u8; 4] = [40, 200, 70, 255];

/// The running workflow step and its start time; the finished steps with their durations.
static PHASE: Mutex<Option<(&'static str, Instant)>> = Mutex::new(None);
static PHASE_TIMES: Mutex<Vec<(&'static str, u128)>> = Mutex::new(Vec::new());

/// Ends the running workflow step (logging its milliseconds) and begins `next`; `None` logs the summary.
fn phase_mark(next: Option<&'static str>) {
    let mut current = PHASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut times = PHASE_TIMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((name, start)) = current.take() {
        let ms = start.elapsed().as_millis();
        log::info!("workflow step \"{name}\" done in {ms} ms");
        times.push((name, ms));
    }
    match next {
        Some(name) => {
            log::info!("workflow step \"{name}\" begins");
            *current = Some((name, Instant::now()));
        }
        None => {
            let all: Vec<String> = times.iter().map(|(n, ms)| format!("{n} {ms} ms")).collect();
            log::info!("workflow step timings: {}", all.join(", "));
        }
    }
}

/// A `Setup` that begins a timed workflow step.
macro_rules! phase {
    ($name:literal) => {
        Step::Setup(concat!("workflow step: ", $name), |_, _| {
            phase_mark(Some($name));
            Ok(())
        })
    };
}

/// A `Setup` that checks an edit of the workflow (see `checkpoint`).
macro_rules! checkpoint {
    ($what:literal, $changes:literal) => {
        Step::Setup(concat!("checkpoint: ", $what), |s, sc| checkpoint(s, sc, $what, $changes))
    };
}

/// After an edit of the workflow: nothing pending, at least one new undo entry and, with `changes`, a changed
/// composite. Counts the undo entries in `wf_ops`.
fn checkpoint(state: &mut AppState, sc: &mut Scratch, what: &str, changes: bool) -> Result<(), String> {
    let doc = doc_of(state)?;
    if doc.has_pending_edit() {
        return Err(format!("{what}: an edit is still pending"));
    }
    let undo = doc.history.undo_len() as u64;
    let before = remembered(sc, "wf_undo")?;
    if undo <= before {
        return Err(format!("{what}: no new undo entry ({undo} entries, {before} before)"));
    }
    *sc.values.entry("wf_ops").or_insert(0) += undo - before;
    sc.values.insert("wf_undo", undo);
    let hash = composite_hash(doc);
    if changes && hash == remembered(sc, "wf_hash")? {
        return Err(format!("{what}: the composite did not change"));
    }
    sc.values.insert("wf_hash", hash);
    log::info!("{what}: ok ({before} -> {undo} undo entries)");
    Ok(())
}

/// Logs what was opened: sets, sizes, layers, mesh size and bounds.
fn log_document(doc: &Document) {
    let b = doc.mesh.bounds;
    log::info!(
        "document {:?}: {} triangles, {} texture set(s), {} stamp(s), bounds {:?} .. {:?} (diagonal {})",
        doc.name,
        doc.mesh.tri_count(),
        doc.sets.len(),
        doc.stamps.len(),
        b.min,
        b.max,
        doc.mesh.diag()
    );
    for set in &doc.sets {
        let layers: Vec<String> = set.layers.iter().map(|l| format!("{} ({})", l.name, l.kind_name())).collect();
        log::info!("  set {:?}: {}x{}, layers {layers:?}", set.name, set.width, set.height);
    }
}

/// The steps that open the CLI `PATH`: the app opens it at startup; an import dialog is accepted with its defaults
/// (a refused import fails with the dialog's reason); an import warnings message is closed.
fn open_and_accept(has_path: bool) -> Vec<Step> {
    if !has_path {
        return open_path(false);
    }
    vec![
        Step::WaitIdle,
        Step::Dyn("accept the import dialog with its defaults", accept_import),
        Step::WaitIdle,
        Step::Dyn("close an import warnings message", close_message),
        Step::WaitIdle,
        Step::Assert("a document with texture sets is open", |s, _| {
            let doc = doc_of(s).map_err(|_| format!("no document was opened from the PATH argument (dialog: {})", dialog_name(&s.dialog)))?;
            if doc.sets.is_empty() { Err("the document has no texture set".to_owned()) } else { Ok(()) }
        }),
    ]
}

/// `ImportCreate` when the import dialog shows a usable model; the refusal reason otherwise.
fn accept_import(state: &AppState, _scratch: &Scratch) -> Result<Vec<Step>, String> {
    let Dialog::Import(st) = &state.dialog else { return Ok(Vec::new()) };
    if st.loading {
        // Still loading although nothing is busy: look again in a few frames (the run timeout ends a hang).
        return Ok(vec![Step::WaitFrames(5), Step::WaitIdle, Step::Dyn("accept the import dialog with its defaults", accept_import)]);
    }
    if let Some(e) = &st.error {
        return Err(format!("the import was refused: {e}"));
    }
    let model = st.model.as_ref().ok_or("the import dialog has no model")?;
    let textures: Vec<String> = model
        .textures
        .iter()
        .map(|t| format!("{} ({})", t.reference, t.image.as_ref().map_or("not loaded".to_owned(), |i| format!("{}x{}", i.width, i.height))))
        .collect();
    log::info!(
        "import dialog: {} set(s) {:?}, textures {textures:?}, report {:?}, warnings {:?}",
        model.mesh.sets.len(),
        model.mesh.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        model.report,
        model.warnings
    );
    if model.mesh.sets.is_empty() {
        return Err(format!("the import was refused: no texture set (no UV-mapped geometry); report {:?}", model.report));
    }
    Ok(vec![Step::Do(AppAction::ImportCreate)])
}

/// A screen point of a 3D view whose ray hits UV-mapped geometry.
#[derive(Copy, Clone, Debug)]
struct ScreenHit {
    pos: egui::Pos2,
    col: usize,
    row: usize,
    set: TextureSetId,
    uv: Vec2,
    /// Cosine between the (viewer-facing) shading normal and the direction to the viewer.
    facing: f32,
}

/// Raycasts a `cols` x `rows` grid over the inset 3D view (the same `surface_at` the tools use); the hits in
/// row-major order.
fn screen_hits(state: &AppState, view: ViewId, cols: usize, rows: usize) -> Result<(ViewXform, Vec<ScreenHit>), String> {
    let xf = view_xf(state, view)?;
    if !xf.is_3d {
        return Err(format!("the view {view:?} is not a 3D view"));
    }
    let doc = doc_of(state)?;
    let area = xf.rect.shrink2(xf.rect.size() * GRID_INSET);
    let eye = xf.camera.eye();
    let mut hits = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let pos = area.min
                + egui::vec2(
                    area.width() * (col as f32 + 0.5) / cols as f32,
                    area.height() * (row as f32 + 0.5) / rows as f32,
                );
            if let crate::tools::Surface::Model(hit) = xf.surface_at(doc, TextureSetId(0), pos)
                && let (Some(set), Some(uv)) = (hit.texture_set, hit.uv)
                && uv.is_finite()
                && has_uv_area(doc, hit.tri)
            {
                let to_eye = (eye - hit.position).normalize_or_zero();
                hits.push(ScreenHit { pos, col, row, set, uv, facing: to_eye.dot(hit.normal) });
            }
        }
    }
    Ok((xf, hits))
}

/// Whether triangle `tri` covers any UV area: a file's own degenerate UVs (all three corners on one point or one
/// line, e.g. parts of spider.fbx) map the triangle to no texel, so a stroke or click there cannot change pixels.
fn has_uv_area(doc: &wte_core::Document, tri: wte_core::TriId) -> bool {
    let [a, b, c] = doc.mesh.tri_uvs(tri);
    (b - a).perp_dot(c - a).abs() > 1e-12
}

/// The hits of the 3D view, failing when none hits the model. A model made of parts too small for the grid
/// (gltf_very_deep_hierarchy: thousands of boxes shrinking along a long chain) is sampled again with a 4x finer grid.
fn model_hits(state: &AppState) -> Result<(ViewXform, Vec<ScreenHit>), String> {
    let (xf, hits) = screen_hits(state, ViewId::Main3D, GRID_COLS, GRID_ROWS)?;
    if !hits.is_empty() {
        return Ok((xf, hits));
    }
    let (cols, rows) = (GRID_COLS * 4, GRID_ROWS * 4);
    let (xf, hits) = screen_hits(state, ViewId::Main3D, cols, rows)?;
    if hits.is_empty() {
        return Err(format!("no point of a {cols}x{rows} grid over the 3D view hits UV-mapped geometry"));
    }
    log::info!("the {GRID_COLS}x{GRID_ROWS} grid missed the model; {} hit(s) on a {cols}x{rows} grid", hits.len());
    Ok((xf, hits))
}

/// Runs of horizontally adjacent hits, longest first.
fn row_runs(hits: &[ScreenHit]) -> Vec<Vec<ScreenHit>> {
    let mut runs: Vec<Vec<ScreenHit>> = Vec::new();
    for h in hits {
        match runs.last_mut() {
            Some(run) if run.last().is_some_and(|p| p.row == h.row && p.col + 1 == h.col) => run.push(*h),
            _ => runs.push(vec![*h]),
        }
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.len()));
    runs
}

/// Brush stroke `k` (0, 1, 2) of the workflow: along the k-th longest run of hits, runs at least 3 grid rows apart.
fn hit_stroke(state: &AppState, k: usize) -> Result<Vec<Step>, String> {
    let (xf, hits) = model_hits(state)?;
    let runs = row_runs(&hits);
    let mut spread: Vec<&Vec<ScreenHit>> = Vec::new();
    for run in &runs {
        if spread.iter().all(|c| c[0].row.abs_diff(run[0].row) >= 3) {
            spread.push(run);
        }
    }
    let run = spread.get(k).copied().or_else(|| runs.get(k)).or_else(|| runs.first()).ok_or("no hit")?;
    let points: Vec<egui::Pos2> = if run.len() >= 2 {
        run.iter().map(|h| h.pos).collect()
    } else {
        // A lone hit (a thin part such as a leg of spider.fbx): the stroke starts on the hit itself, since a point
        // a few pixels away can already miss the part, and a stroke that begins and ends off the model paints nothing.
        let p = run[0].pos;
        vec![p, p + egui::vec2(1.0, 0.5)]
    };
    let sets: std::collections::BTreeSet<u32> = run.iter().map(|h| h.set.0).collect();
    log::info!("3D stroke {k}: grid row {}, {} hit point(s), texture set(s) {sets:?}", run[0].row, run.len());
    Ok(vec![drag(&xf, &points)])
}

/// Hits whose 8 grid neighbours all hit too, scored by how directly they face the viewer.
fn best_spots(hits: &[ScreenHit]) -> Vec<ScreenHit> {
    let at: std::collections::HashSet<(usize, usize)> = hits.iter().map(|h| (h.col, h.row)).collect();
    let interior = |h: &ScreenHit| {
        (-1i64..=1).all(|dx| {
            (-1i64..=1).all(|dy| at.contains(&((h.col as i64 + dx).max(0) as usize, (h.row as i64 + dy).max(0) as usize)))
        })
    };
    let mut scored: Vec<(f32, ScreenHit)> = hits.iter().map(|h| (h.facing + if interior(h) { 1.0 } else { 0.0 }, *h)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, h)| h).collect()
}

/// Two well-facing spots on the model for the image stamp (`first`) and the text stamp, as far apart as possible
/// among the good ones.
fn stamp_spots(state: &AppState) -> Result<(ViewXform, ScreenHit, ScreenHit), String> {
    let (xf, hits) = model_hits(state)?;
    let ranked = best_spots(&hits);
    // On a model of many small parts (many_islands.obj) a grid point can lie at the very edge of a part, where the
    // click (rounded to the pointer's precision) misses: prefer spots whose neighbourhood of a few points hits too.
    let doc = doc_of(state)?;
    let solid = |h: &ScreenHit| {
        [egui::vec2(0.0, 0.0), egui::vec2(3.0, 0.0), egui::vec2(-3.0, 0.0), egui::vec2(0.0, 3.0), egui::vec2(0.0, -3.0)]
            .iter()
            .all(|d| matches!(xf.surface_at(doc, h.set, h.pos + *d), Surface::Model(hit) if hit.texture_set.is_some()))
    };
    // Sets with a texture too small to show a stamp (tex_npot.obj's 1x1 "one_pixel" set: the text is added but
    // covers no texel, with a warning) come last.
    let roomy = |h: &ScreenHit| doc.sets.get(h.set.index()).is_some_and(|set| set.width.min(set.height) >= 32);
    let (solid_spots, rest): (Vec<ScreenHit>, Vec<ScreenHit>) = ranked.into_iter().partition(solid);
    // Both clicks stay on solid spots whenever there are any: on a thin model (BrainStem.glb's legs) the far spots
    // that are not solid lie on an edge, where the click misses.
    let mut pool = if solid_spots.is_empty() { rest } else { solid_spots };
    if pool.iter().any(roomy) {
        pool.retain(roomy);
    }
    log::info!("stamp spots: 3D view at {:?}, {} candidate(s)", xf.rect, pool.len());
    let first = pool.iter().find(|h| h.col < GRID_COLS / 2).or(pool.first()).copied().ok_or("no hit")?;
    let good = &pool[..pool.len().clamp(1, (pool.len() / 3).max(8))];
    let second = good
        .iter()
        .chain(pool.iter())
        .find(|h| (h.pos - first.pos).length() >= xf.rect.width() * 0.2)
        .or_else(|| pool.iter().max_by(|a, b| (a.pos - first.pos).length().total_cmp(&(b.pos - first.pos).length())))
        .copied()
        .ok_or("no hit")?;
    Ok((xf, first, second))
}

/// The gizmo of the selected stamp (image or text) in the 3D view.
fn selected_gizmo_3d(state: &AppState) -> Result<(ViewXform, GizmoShape), String> {
    let session = session_of(state)?;
    let id = session.selected_stamp.ok_or("no stamp is selected")?;
    let placement = session.doc.stamp(id).ok_or("the selected stamp does not exist")?.placement;
    let xf = view_xf(state, ViewId::Main3D)?;
    let shape = gizmo::gizmo_shape(&placement, &xf).ok_or_else(|| format!("the gizmo of stamp {id:?} is not visible"))?;
    Ok((xf, shape))
}

fn pack(v: Vec2) -> u64 {
    ((v.x.to_bits() as u64) << 32) | v.y.to_bits() as u64
}

fn unpack(bits: u64) -> Vec2 {
    Vec2::new(f32::from_bits((bits >> 32) as u32), f32::from_bits(bits as u32))
}

/// Picks the texture set and three UVs (wrapped into 0..1) of the 2D stroke from hits of the 3D view, preferring the
/// active set: the stroke then starts and ends on UV islands of real geometry.
fn pick_uv_stroke(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let (_, hits) = model_hits(state)?;
    let active = session_of(state)?.active_set;
    let set = if hits.iter().any(|h| h.set == active) { active } else { hits[0].set };
    let in_set: Vec<&ScreenHit> = hits.iter().filter(|h| h.set == set).collect();
    let n = in_set.len();
    let picks = [in_set[0], in_set[n / 2], in_set[n - 1]];
    for (key, h) in ["wf_uv0", "wf_uv1", "wf_uv2"].into_iter().zip(picks) {
        sc.values.insert(key, pack(Vec2::new(h.uv.x.rem_euclid(1.0), h.uv.y.rem_euclid(1.0))));
    }
    sc.values.insert("wf_uv_set", set.0 as u64);
    log::info!("2D stroke in set {}: UVs {:?}", set.0, picks.map(|h| h.uv));
    Ok(())
}

/// The workflow on the CLI `PATH` (see the 8.11 table): every editing feature once, with per-step timing in the log.
/// Fix round 2, item 2: a large image stamp placed near the silhouette of the model at PATH (Avocado.glb: its curved
/// side). Asserts that the depth was auto-fitted and the projection box is drawn, then measures the coverage of the
/// fitted placement against the old default (depth = the larger half size, angles 60 / 85 degrees) by re-placing the
/// stamp with it, logs both, and undoes back to the fitted placement. Ends with the stamp selected (Transform tool) so
/// the screenshot shows the gizmo with its projection box.
fn stamp_edge_path(has_path: bool) -> Vec<Step> {
    let mut steps = open_and_accept(has_path);
    if !has_path {
        return steps;
    }
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Setup("register the sample stamp image (30 % of the model size)", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.3;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        // The tool options of the new tool can change the side panel width and with it the 3D view: let the layout
        // settle before click positions are computed from it.
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the stamp near the silhouette", |s, _| {
            let (xf, spot) = edge_spot(s)?;
            log::info!("edge spot at {:?} (facing {:.2})", norm(&xf, spot.pos), spot.facing);
            Ok(vec![click(ViewId::Main3D, norm(&xf, spot.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| placed_stamp(s, false).map(|_| ())),
        Step::Assert("its depth is fitted and its projection box is drawn", assert_fitted_3d_stamp),
        Step::Setup("remember the coverage of the fitted placement", |s, sc| {
            let (full, any) = selected_coverage(s)?;
            sc.values.insert("edge_full", full);
            sc.values.insert("edge_any", any);
            Ok(())
        }),
        Step::Dyn("re-place it with the old default depth and angles", |s, _| {
            let id = session_of(s)?.selected_stamp.ok_or("no stamp is selected")?;
            let Placement::Projected(mut p) = doc_of(s)?.stamp(id).ok_or("no stamp")?.placement else {
                return Err("the stamp is not projected".to_owned());
            };
            let h = p.half_size.abs().max_element();
            (p.depth_front, p.depth_back, p.angle_start_deg, p.angle_end_deg) = (h, h, 60.0, 85.0);
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, Placement::Projected(p), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("the fitted placement covers the surface the old default cut off", |s, sc| {
            let (old_full, old_any) = selected_coverage(s)?;
            let (full, any) = (remembered(sc, "edge_full")?, remembered(sc, "edge_any")?);
            log::info!(
                "edge stamp coverage: fitted {full} texels at full alpha / {any} with any alpha; old default {old_full} / {old_any}"
            );
            if (any as f64) < 0.95 * old_any as f64 || (full as f64) < 0.95 * old_full as f64 {
                return Err(format!("fitted {full} / {any} texels, old default {old_full} / {old_any}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo restored the fitted placement", assert_fitted_3d_stamp),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::WaitFrames(5),
    ]);
    steps
}

/// A well-facing spot of the 3D view close to the model's silhouette: a solid hit (its few-point neighbourhood hits
/// too) with a grid point that misses the model within 3 cells, the closest such one, best facing first.
fn edge_spot(state: &AppState) -> Result<(ViewXform, ScreenHit), String> {
    let (xf, hits) = model_hits(state)?;
    let doc = doc_of(state)?;
    let at: std::collections::HashSet<(usize, usize)> = hits.iter().map(|h| (h.col, h.row)).collect();
    let miss_distance = |h: &ScreenHit| -> Option<usize> {
        (1..=3usize).find(|&r| {
            let (c, w) = (h.col as i64, h.row as i64);
            let r = r as i64;
            (-r..=r).any(|dx| (-r..=r).any(|dy| !at.contains(&((c + dx).max(0) as usize, (w + dy).max(0) as usize))))
        })
    };
    let solid = |h: &ScreenHit| {
        [egui::vec2(0.0, 0.0), egui::vec2(3.0, 0.0), egui::vec2(-3.0, 0.0), egui::vec2(0.0, 3.0), egui::vec2(0.0, -3.0)]
            .iter()
            .all(|d| matches!(xf.surface_at(doc, h.set, h.pos + *d), Surface::Model(hit) if hit.texture_set.is_some()))
    };
    let spot = hits
        .iter()
        .filter(|h| h.facing > 0.6 && solid(h))
        .filter_map(|h| miss_distance(h).map(|d| (d, h)))
        .min_by(|a, b| a.0.cmp(&b.0).then(b.1.facing.total_cmp(&a.1.facing)))
        .map(|(_, h)| *h)
        .or_else(|| best_spots(&hits).first().copied())
        .ok_or("no hit")?;
    Ok((xf, spot))
}

/// Texels of the selected stamp's layers: (alpha 255, alpha > 0).
fn selected_coverage(state: &AppState) -> Result<(u64, u64), String> {
    let session = session_of(state)?;
    let id = session.selected_stamp.ok_or("no stamp is selected")?;
    let doc = &session.doc;
    let (mut full, mut any) = (0u64, 0u64);
    for (_, layer) in doc.stamp_layers(id) {
        let Some(layer) = doc.layer(layer) else { continue };
        if let LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } = &layer.kind {
            for part in cache.parts() {
                for px in part.data.as_chunks::<4>().0 {
                    full += (px[3] == 255) as u64;
                    any += (px[3] > 0) as u64;
                }
            }
        }
    }
    Ok((full, any))
}

fn workflow_path(has_path: bool) -> Vec<Step> {
    let mut steps = vec![phase!("open / import")];
    steps.extend(open_and_accept(has_path));
    if !has_path {
        return steps;
    }
    steps.extend([
        Step::Setup("log the document and remember its initial state", |s, sc| {
            let doc = doc_of(s)?;
            log_document(doc);
            let hash = composite_hash(doc);
            let undo = doc.history.undo_len() as u64;
            sc.values.insert("wf_initial", hash);
            sc.values.insert("wf_hash", hash);
            sc.values.insert("wf_initial_undo", undo);
            sc.values.insert("wf_undo", undo);
            sc.values.insert("wf_ops", 0);
            Ok(())
        }),
        phase!("3D view and frame"),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Assert("the model is visible in the 3D view", |s, _| {
            let (_, hits) = model_hits(s)?;
            log::info!("{} of {} grid points hit the model", hits.len(), GRID_COLS * GRID_ROWS);
            Ok(())
        }),
        phase!("3D brush strokes"),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::BrushSizeStep(2)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("3D stroke 1 on the model", |s, _| hit_stroke(s, 0)),
        Step::WaitIdle,
        checkpoint!("3D stroke 1", true),
        Step::Do(AppAction::SetPrimaryColor(GREEN)),
        Step::Dyn("3D stroke 2 on the model", |s, _| hit_stroke(s, 1)),
        Step::WaitIdle,
        checkpoint!("3D stroke 2", true),
        Step::Do(AppAction::SetPrimaryColor(BLUE)),
        Step::Dyn("3D stroke 3 on the model", |s, _| hit_stroke(s, 2)),
        Step::WaitIdle,
        checkpoint!("3D stroke 3", true),
        phase!("2D view stroke"),
        Step::Setup("pick the 2D stroke's set and UVs from the 3D hits", pick_uv_stroke),
        Step::Dyn("activate the 2D stroke's set", |_, sc| {
            Ok(vec![Step::Do(AppAction::SetActiveSet(TextureSetId(remembered(sc, "wf_uv_set")? as u32)))])
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Do(AppAction::SetPrimaryColor([240, 200, 20, 255])),
        Step::Dyn("2D stroke through UVs of real geometry", |s, sc| {
            let xf = view_xf(s, ViewId::Main2D)?;
            // Kept inside the texture: a UV right at its edge (box_orphant_embedded_texture.fbx maps every face onto
            // the whole 0..1 square) would otherwise start the stroke beside the texture, where it paints nothing.
            // The inset is at most half a side, so a very narrow texture (tex_16x8192) keeps a valid range.
            let tex = egui::Rect::from_two_pos(uv_screen(&xf, [0.0, 0.0]), uv_screen(&xf, [1.0, 1.0]));
            let inset = egui::vec2(tex.width().min(4.0), tex.height().min(4.0)) * 0.5;
            let (lo, hi) = (tex.min + inset, tex.max - inset);
            let keep_inside = |q: egui::Pos2| egui::pos2(q.x.max(lo.x).min(hi.x), q.y.max(lo.y).min(hi.y));
            let mut points = Vec::new();
            for key in ["wf_uv0", "wf_uv1", "wf_uv2"] {
                let p = uv_screen(&xf, unpack(remembered(sc, key)?).to_array());
                points.extend([p - egui::vec2(5.0, 0.0), p + egui::vec2(5.0, 0.0)].map(keep_inside));
            }
            if !points.iter().all(|p| xf.rect.contains(*p)) {
                return Err(format!("the 2D stroke leaves the 2D view {:?}: {points:?}", xf.rect));
            }
            Ok(vec![drag(&xf, &points)])
        }),
        Step::WaitIdle,
        checkpoint!("2D stroke", true),
        phase!("split view"),
        Step::Do(AppAction::SetViewMode(ViewMode::Split)),
        Step::WaitIdle,
        Step::WaitFrames(10),
        Step::Assert("Split shows the 3D and the 2D view", |s, _| {
            for view in [ViewId::Main3D, ViewId::Main2D] {
                if !s.view_rects.get(&view).is_some_and(|r| r.width() > 0.0 && r.height() > 0.0) {
                    return Err(format!("the view {view:?} is not shown in Split mode"));
                }
            }
            Ok(())
        }),
        phase!("preview mode"),
        Step::Do(AppAction::SetViewMode(ViewMode::Preview)),
        Step::WaitIdle,
        Step::WaitFrames(30),
        Step::Assert("Preview mode shows the preview view", |s, _| {
            if s.view_mode != ViewMode::Preview || !s.view_rects.contains_key(&ViewId::Preview) {
                return Err(format!("view mode {:?}, views {:?}", s.view_mode, s.view_rects.keys().collect::<Vec<_>>()));
            }
            Ok(())
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
        Step::Assert("Esc left Preview", |s, _| {
            if s.view_mode == ViewMode::Preview { Err("still in Preview after Esc".to_owned()) } else { Ok(()) }
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        phase!("fill on a new layer"),
        Step::Setup("pick the fill point", |s, sc| {
            let (xf, hits) = model_hits(s)?;
            let spot = best_spots(&hits)[0];
            sc.values.insert("wf_fill_pos", pack(Vec2::from_array(norm(&xf, spot.pos))));
            sc.values.insert("wf_fill_set", spot.set.0 as u64);
            s.tools.fill_scope = FillScope::Island;
            Ok(())
        }),
        Step::Dyn("new paint layer in the fill point's set", |_, sc| {
            let set = TextureSetId(remembered(sc, "wf_fill_set")? as u32);
            Ok(vec![Step::Do(AppAction::SetActiveSet(set)), Step::Do(AppAction::AddPaintLayer)])
        }),
        Step::WaitIdle,
        checkpoint!("new paint layer", false),
        Step::Do(AppAction::SetTool(ToolKind::Fill)),
        Step::Do(AppAction::SetPrimaryColor([200, 60, 220, 255])),
        Step::Dyn("fill the island under the fill point", |_, sc| {
            Ok(vec![click(ViewId::Main3D, unpack(remembered(sc, "wf_fill_pos")?).to_array())])
        }),
        Step::WaitIdle,
        checkpoint!("fill", true),
        phase!("image stamp"),
        Step::Setup("write a generated stamp PNG", |s, sc| {
            let dir = fresh_assets_dir(sc)?;
            let img = samples::sample_stamp_image(STAMP_SIDE);
            write(&dir.join("wf_stamp.png"), &png_of(&img)?)?;
            s.tools.stamp.size_frac = 0.2;
            Ok(())
        }),
        Step::Dyn("choose the stamp image", |_, sc| {
            Ok(vec![Step::Do(AppAction::ImportStampImage(Some(assets_dir(sc).join("wf_stamp.png"))))])
        }),
        Step::WaitIdle,
        Step::Assert("the Stamp tool has the image", |s, _| {
            if s.tools.stamp.source.is_some() { Ok(()) } else { Err("no stamp image is chosen".to_owned()) }
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the image stamp on the model", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| placed_stamp(s, false).map(|_| ())),
        checkpoint!("image stamp", true),
        phase!("text stamp"),
        Step::Setup("text content", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            s.tools.text.em_frac = 0.08;
            s.tools.text.color = [20, 20, 20, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        // Settle the layout first: the Text options can change the side panel width and so the 3D view (a click
        // computed from the old layout landed beside the model).
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the text stamp on the model", |s, _| {
            let (xf, _, second) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, second.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the text stamp was placed and selected", |s, _| placed_stamp(s, true).map(|_| ())),
        checkpoint!("text stamp", true),
        phase!("transform the stamp"),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::Dyn("move: drag the gizmo body to a nearby point on the model", |s, _| {
            let (xf, shape) = selected_gizmo_3d(s)?;
            let (_, hits) = model_hits(s)?;
            let size = (shape.corners[2] - shape.corners[0]).length().max(16.0);
            let want = shape.center + egui::vec2(0.35, -0.2) * size;
            let target = hits
                .iter()
                .map(|h| h.pos)
                .filter(|p| (*p - shape.center).length() > 6.0)
                .min_by(|a, b| (*a - want).length().total_cmp(&(*b - want).length()))
                .unwrap_or(want);
            Ok(vec![drag(&xf, &[shape.center, target])])
        }),
        Step::WaitIdle,
        checkpoint!("move the stamp", true),
        Step::Dyn("scale: drag a corner handle outwards", |s, _| {
            let (xf, shape) = selected_gizmo_3d(s)?;
            let corner = shape.corners[2];
            Ok(vec![drag(&xf, &[corner, shape.center + (corner - shape.center) * 1.4])])
        }),
        Step::WaitIdle,
        checkpoint!("scale the stamp", true),
        Step::Dyn("rotate: turn the knob by 40 degrees", |s, _| {
            let (xf, shape) = selected_gizmo_3d(s)?;
            let r = shape.rotate_knob - shape.center;
            let a0 = r.y.atan2(r.x);
            let points: Vec<egui::Pos2> = (0..=12)
                .map(|k| {
                    let a = a0 + 40f32.to_radians() * k as f32 / 12.0;
                    shape.center + egui::vec2(a.cos(), a.sin()) * r.length()
                })
                .collect();
            Ok(vec![drag(&xf, &points)])
        }),
        Step::WaitIdle,
        checkpoint!("rotate the stamp", true),
        phase!("undo all / redo all"),
        Step::Setup("remember the final state", |s, sc| {
            let doc = doc_of(s)?;
            sc.values.insert("wf_final", composite_hash(doc));
            sc.values.insert("wf_final_undo", doc.history.undo_len() as u64);
            Ok(())
        }),
        Step::Dyn("undo everything", |s, _| {
            let n = doc_of(s)?.history.undo_len();
            log::info!("undo all: {n} entries");
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undo-all nothing is left to undo and the composite is the initial one", |s, sc| {
            let doc = doc_of(s)?;
            if doc.history.undo_len() != 0 {
                return Err(format!("{} entries are left to undo", doc.history.undo_len()));
            }
            let complete = remembered(sc, "wf_initial_undo")? == 0 && remembered(sc, "wf_ops")? == remembered(sc, "wf_final_undo")?;
            if !complete {
                log::warn!("the history does not reach back to the import (evicted entries): the initial composite is not compared");
            } else if composite_hash(doc) != remembered(sc, "wf_initial")? {
                return Err("the composite after undo-all differs from the imported one".to_owned());
            }
            Ok(())
        }),
        Step::Dyn("redo everything", |s, _| {
            let n = doc_of(s)?.history.redo_len();
            Ok((0..n).map(|_| Step::Do(AppAction::Redo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after redo-all the composite is the final one", |s, sc| {
            let doc = doc_of(s)?;
            if doc.history.undo_len() as u64 != remembered(sc, "wf_final_undo")? {
                return Err(format!("{} undo entries after redo-all", doc.history.undo_len()));
            }
            if composite_hash(doc) != remembered(sc, "wf_final")? {
                return Err("the composite after redo-all differs from the one before undo-all".to_owned());
            }
            Ok(())
        }),
        phase!("export every format"),
        Step::Setup("start with an empty export folder", |_, sc| {
            let dir = with_suffix(&sc.out_stem, "_export");
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
            }
            Ok(())
        }),
        Step::Dyn("export flattened PNG, per-layer PNGs, ORA and PSD", |s, sc| {
            Ok(vec![Step::Do(AppAction::RunExport(export_options(s, sc)?))])
        }),
        Step::WaitIdle,
        Step::Assert("every planned export file exists", |s, sc| {
            if !matches!(s.dialog, Dialog::None) {
                return Err(format!("a dialog is open after the export: {}", dialog_name(&s.dialog)));
            }
            let plan = wte_core::export::plan(&doc_of(s)?.snapshot(), &export_options(s, sc)?);
            if plan.is_empty() {
                return Err("export::plan is empty".to_owned());
            }
            let mut total = 0;
            for path in &plan {
                file_non_empty(path)?;
                total += std::fs::metadata(path).map_or(0, |m| m.len());
            }
            // Exactly the planned files, each holding what the app shows (pixels, layers, recomposition).
            let doc = doc_of(s)?;
            let opts = export_options(s, sc)?;
            let summary = verify_export(doc, &opts, &read_export_dir(doc, &opts)?)?;
            log::info!("workflow export: {summary}");
            let names: Vec<String> = plan.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            for ext in ["png", "ora", "psd"] {
                if !names.iter().any(|n| n.ends_with(&format!(".{ext}"))) {
                    return Err(format!("no .{ext} file in {names:?}"));
                }
            }
            log::info!("exported {} files, {total} bytes: {names:?}", names.len());
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the export toast names the folder only and stays off the side panel", |s, sc| {
            let folder = with_suffix(&sc.out_stem, "_export");
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let labels = s.toasts.visible_labels();
            let export = labels.iter().find(|l| l.starts_with("Exported ")).ok_or_else(|| format!("no export toast in {labels:?}"))?;
            if !export.ends_with(&format!(" to {name}")) || export.contains([std::path::MAIN_SEPARATOR, '/']) {
                return Err(format!("the export toast is {export:?}, expected the folder name {name:?} without its path"));
            }
            assert_toasts_off_side_panel(s, 1)
        }),
        phase!("save project"),
        Step::Dyn("Save As the project", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Assert("the project was saved", |s, sc| {
            let path = with_suffix(&sc.out_stem, ".wtep");
            let session = session_of(s)?;
            if session.doc.is_modified() || session.path.as_deref() != Some(path.as_path()) {
                return Err(format!("the document is not saved as {} (path {:?})", path.display(), session.path));
            }
            file_non_empty(&path)?;
            log::info!("saved {} ({} bytes)", path.display(), std::fs::metadata(&path).map_or(0, |m| m.len()));
            Ok(())
        }),
        Step::Setup("remember the saved composites and structure", |s, sc| {
            let doc = doc_of(s)?;
            sc.values.insert("wf_saved", composite_hash(doc));
            sc.values.insert("wf_structure", structure_key(doc));
            Ok(())
        }),
        phase!("reopen project"),
        Step::Dyn("open the saved project", |_, sc| {
            Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("the reopened project equals the saved one", |s, sc| {
            // R3-3: reopening the project just saved repeats no import warning ("Warnings" box).
            assert_no_dialog(s, sc)?;
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, ".wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            if session.doc.history.undo_len() != 0 {
                return Err("the reopened document still has the old history".to_owned());
            }
            if structure_key(&session.doc) != remembered(sc, "wf_structure")? {
                log_document(&session.doc);
                return Err("the sets, layers or stamps differ after the round trip".to_owned());
            }
            for set in 0..session.doc.sets.len() {
                log::info!("reopened set {set}: composite hash {:016x}", set_hash(&session.doc, set)?);
            }
            if composite_hash(&session.doc) != remembered(sc, "wf_saved")? {
                return Err("the composites differ after the round trip".to_owned());
            }
            Ok(())
        }),
        phase!("final split view"),
        Step::Do(AppAction::SetViewMode(ViewMode::Split)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Assert("no toast covers the side panel or the Layers controls", |s, _| assert_toasts_off_side_panel(s, 0)),
        Step::Setup("workflow timings", |_, _| {
            phase_mark(None);
            Ok(())
        }),
    ]);
    steps
}

/// R3-7: every toast drawn in the last frame lies inside the central view area, is at most `toasts::MAX_WIDTH` wide
/// and intersects neither the side panel nor any Layers control of `AppState::panel_rects`; at least `min` are drawn.
fn assert_toasts_off_side_panel(s: &AppState, min: usize) -> Result<(), String> {
    let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?;
    let central = s.central_rect.ok_or("the central view rect was not recorded")?;
    let rects = s.toasts.drawn_rects();
    if rects.len() < min {
        return Err(format!("{} toast(s) drawn, expected at least {min}: {:?}", rects.len(), s.toasts.visible_labels()));
    }
    for r in rects {
        if r.width() > crate::toasts::MAX_WIDTH + 0.5 {
            return Err(format!("a toast is {:.0} px wide: {r:?}", r.width()));
        }
        if r.intersects(side) || !central.expand(0.5).contains_rect(*r) {
            return Err(format!("a toast {r:?} leaves the central view area {central:?} (side panel {side:?})"));
        }
        if let Some((hit, _)) = s.panel_rects.iter().find(|(_, p)| p.intersects(*r)) {
            return Err(format!("a toast {r:?} covers the Layers control {hit:?}"));
        }
    }
    log::info!("{} toast(s) inside the central view area {central:?}, side panel {side:?}: {rects:?}", rects.len());
    Ok(())
}

/// Sets (name, size), layers (name, kind, visibility, lock, blend, opacity) and stamps (name, kind) as one hash.
fn structure_key(doc: &Document) -> u64 {
    let mut text = String::new();
    for set in &doc.sets {
        text.push_str(&format!("set {} {}x{}\n", set.name, set.width, set.height));
        for l in &set.layers {
            text.push_str(&format!(" {} {} {} {} {:?} {}\n", l.name, l.kind_name(), l.visible, l.locked, l.blend, l.opacity));
        }
    }
    for st in &doc.stamps {
        text.push_str(&format!("stamp {} {}\n", st.name, matches!(st.source, StampSource::Text(_))));
    }
    wte_core::fnv1a64(0xcbf2_9ce4_8422_2325, text.as_bytes())
}

// ---- fuzz_path ----

/// Seeded pseudo-random state of the fuzz run (splitmix64).
struct Fuzz {
    rng: u64,
    index: u32,
    total: u32,
}

static FUZZ: Mutex<Fuzz> = Mutex::new(Fuzz { rng: 0, index: 0, total: 0 });

impl Fuzz {
    fn next(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
    fn color(&mut self) -> [u8; 4] {
        let v = self.next().to_le_bytes();
        [v[0], v[1], v[2], if self.chance(0.2) { v[3] } else { 255 }]
    }
    /// A random point in a view, normalized.
    fn spot(&mut self) -> [f32; 2] {
        [self.range(0.03, 0.97), self.range(0.03, 0.97)]
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

const TEXT_VARIANTS: &[&str] = &["Fuzz", "WTE 2026", "two\nlines", "Ünïcödé ß €", "x", "A much longer text stamp line for wrapping"];
const RESIZES: &[[u32; 2]] = &[[1280, 800], [1024, 700], [1600, 1000], [800, 600], [1920, 1080], [640, 480], [1366, 768], [1100, 1000]];
const VIEW_MODES: &[ViewMode] = &[ViewMode::View3D, ViewMode::View2D, ViewMode::Split, ViewMode::Preview];
const TOOLS: &[ToolKind] = &[
    ToolKind::Navigate,
    ToolKind::Brush,
    ToolKind::Eraser,
    ToolKind::Eyedropper,
    ToolKind::Fill,
    ToolKind::Stamp,
    ToolKind::Text,
    ToolKind::Transform,
];

/// The fuzz run on the CLI `PATH`: `WTE_FUZZ_STEPS` (default 400) seeded random actions (`WTE_FUZZ_SEED`, default
/// 1), each logged before it runs, then undo-all / redo-all and a sanity check.
fn fuzz_path(args: &CliArgs) -> Vec<Step> {
    let mut steps = open_and_accept(args.path.is_some());
    if args.path.is_none() {
        return steps;
    }
    let seed = env_u64("WTE_FUZZ_SEED", 1);
    let total = env_u64("WTE_FUZZ_STEPS", 400).min(100_000) as u32;
    *FUZZ.lock().unwrap_or_else(|e| e.into_inner()) = Fuzz { rng: seed, index: 0, total };
    log::info!("fuzz_path: seed {seed}, {total} actions (WTE_FUZZ_SEED / WTE_FUZZ_STEPS)");
    steps.extend([
        Step::Setup("register the sample stamp image and a text", |s, sc| {
            log_document(doc_of(s)?);
            setup_stamp_image(s, sc)?;
            s.tools.stamp.size_frac = 0.2;
            s.tools.text.content = "Fuzz".to_owned();
            s.tools.text.em_frac = 0.06;
            Ok(())
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
    ]);
    steps.extend((0..total).map(|_| Step::Dyn("fuzz action", fuzz_step)));
    let [w, h] = args.size;
    steps.extend([
        Step::WaitIdle,
        Step::Dyn("close an open dialog", |s, _| {
            Ok(if matches!(s.dialog, Dialog::None) { Vec::new() } else { vec![Step::Do(AppAction::CloseDialog), Step::WaitFrames(2)] })
        }),
        Step::Resize([w, h]),
        Step::WaitIdle,
        Step::Dyn("undo everything", |s, _| {
            let n = doc_of(s)?.history.undo_len();
            log::info!("fuzz: undo all ({n} entries)");
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Dyn("redo everything", |s, _| {
            let n = doc_of(s)?.history.redo_len();
            log::info!("fuzz: redo all ({n} entries)");
            Ok((0..n).map(|_| Step::Do(AppAction::Redo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("the document survived the fuzz run", |s, _| {
            let doc = doc_of(s)?;
            if doc.has_pending_edit() {
                return Err("an edit is still pending".to_owned());
            }
            for set in &doc.sets {
                let c = set.composite();
                if c.width != set.width || c.height != set.height {
                    return Err(format!("set {:?}: composite {}x{} for a {}x{} set", set.name, c.width, c.height, set.width, set.height));
                }
                if let Some(active) = set.active
                    && set.layer(active).is_none()
                {
                    return Err(format!("set {:?}: the active layer {active:?} does not exist", set.name));
                }
            }
            for stamp in &doc.stamps {
                if doc.stamp_layers(stamp.id).is_empty() {
                    return Err(format!("the stamp {:?} has no layer", stamp.name));
                }
            }
            log_document(doc);
            Ok(())
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::Split)),
        Step::WaitIdle,
        Step::Assert("the texture is reachable in the 2D half of Split (R3-11)", |s, _| texture_in_view(s, "fuzz end")),
    ]);
    steps
}

/// One fuzz action: logged before its steps run.
fn fuzz_step(state: &AppState, _scratch: &Scratch) -> Result<Vec<Step>, String> {
    let mut f = FUZZ.lock().unwrap_or_else(|e| e.into_inner());
    f.index += 1;
    let (i, n) = (f.index, f.total);
    let (desc, mut steps) = fuzz_action(&mut f, state);
    match f.below(10) {
        0..=4 => {}
        5..=7 => steps.push(Step::WaitFrames(1 + f.below(3) as u32)),
        _ => steps.push(Step::WaitIdle),
    }
    if i % 25 == 0 {
        steps.push(Step::WaitIdle);
    }
    log::info!("fuzz {i}/{n}: {desc}");
    Ok(steps)
}

/// Random screen-space path in a view (normalized), `waypoints` corners, densified.
fn random_path(f: &mut Fuzz, waypoints: usize) -> Vec<[f32; 2]> {
    let corners: Vec<[f32; 2]> = (0..waypoints.max(2)).map(|_| f.spot()).collect();
    let mut out = Vec::new();
    for w in corners.windows(2) {
        for k in 0..8 {
            let t = k as f32 / 8.0;
            out.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
        }
    }
    out.extend(corners.last());
    out
}

/// A path for a tool in `view`: mostly through points on the model (3D) or anywhere (2D, misses included).
fn tool_path(f: &mut Fuzz, state: &AppState, view: ViewId) -> (String, Vec<[f32; 2]>) {
    if view == ViewId::Main3D
        && f.chance(0.75)
        && let Ok((xf, hits)) = screen_hits(state, view, FUZZ_COLS, FUZZ_ROWS)
        && !hits.is_empty()
    {
        let k = 2 + f.below(4);
        let pts: Vec<egui::Pos2> = (0..k).map(|_| f.pick(&hits).pos).collect();
        let path = densify(&pts).into_iter().map(|p| norm(&xf, p)).collect();
        return (format!("through {k} model points"), path);
    }
    let k = 2 + f.below(4);
    (format!("along {k} random points"), random_path(f, k))
}

/// A click spot for a tool in `view`: a model point (3D, mostly) or anywhere.
fn tool_spot(f: &mut Fuzz, state: &AppState, view: ViewId) -> [f32; 2] {
    if view == ViewId::Main3D
        && f.chance(0.85)
        && let Ok((xf, hits)) = screen_hits(state, view, FUZZ_COLS, FUZZ_ROWS)
        && !hits.is_empty()
    {
        return norm(&xf, f.pick(&hits).pos);
    }
    f.spot()
}

fn pointer(view: ViewId, path: Vec<[f32; 2]>, button: egui::PointerButton, mods: egui::Modifiers, ppf: usize) -> Step {
    Step::Pointer { view, path, button, mods, points_per_frame: ppf }
}

/// Frames to wait after an action that can change the layout, so the next action sees the new view rects.
fn settle() -> Step {
    Step::WaitFrames(3)
}

/// Picks and builds one action.
fn fuzz_action(f: &mut Fuzz, state: &AppState) -> (String, Vec<Step>) {
    use AppAction as A;
    let Some(session) = state.session.as_ref() else { return ("no document: wait".to_owned(), vec![Step::WaitFrames(2)]) };
    if !matches!(state.dialog, Dialog::None) {
        return (format!("close the {} dialog", dialog_name(&state.dialog)), vec![Step::Do(A::CloseDialog), settle()]);
    }
    let doc = &session.doc;
    let views: Vec<ViewId> = [ViewId::Main3D, ViewId::Main2D, ViewId::Preview]
        .into_iter()
        .filter(|v| state.view_rects.get(v).is_some_and(|r| r.width() > 4.0 && r.height() > 4.0))
        .collect();
    let edit_views: Vec<ViewId> = views.iter().copied().filter(|v| *v != ViewId::Preview).collect();
    if views.is_empty() {
        return ("no view is shown: 3D view".to_owned(), vec![Step::Do(A::SetViewMode(ViewMode::View3D)), settle()]);
    }
    let roll = f.below(100);
    match roll {
        // ---- navigation ----
        0..=11 => {
            let view = f.pick(&views);
            match f.below(3) {
                0 => {
                    let k = 2 + f.below(4);
                    let path = random_path(f, k);
                    let ppf = 1 + f.below(3);
                    (format!("orbit drag in {view:?}"), vec![pointer(view, path, egui::PointerButton::Secondary, egui::Modifiers::NONE, ppf)])
                }
                1 => {
                    let k = 2 + f.below(3);
                    let path = random_path(f, k);
                    (format!("pan drag in {view:?}"), vec![pointer(view, path, egui::PointerButton::Middle, egui::Modifiers::NONE, 2)])
                }
                _ => {
                    let notches = f.pick(&[-4.0, -2.0, -1.0, 1.0, 2.0, 3.0]);
                    let mods = f.pick(&[egui::Modifiers::NONE, egui::Modifiers::NONE, egui::Modifiers::CTRL, egui::Modifiers::SHIFT]);
                    let at = f.spot();
                    (format!("wheel {notches} notches ({mods:?}) in {view:?}"), vec![Step::Scroll { view, at, notches, mods }])
                }
            }
        }
        // ---- view modes ----
        12..=16 => {
            let mode = f.pick(VIEW_MODES);
            (format!("view mode {mode:?}"), vec![Step::Do(A::SetViewMode(mode)), settle()])
        }
        17..=19 => {
            let k = 3 + f.below(6);
            let modes: Vec<ViewMode> = (0..k).map(|_| f.pick(VIEW_MODES)).collect();
            let mut steps: Vec<Step> = modes.iter().map(|m| Step::Do(A::SetViewMode(*m))).collect();
            steps.push(settle());
            (format!("rapid view mode toggling {modes:?}"), steps)
        }
        20..=24 => {
            let tool = f.pick(TOOLS);
            (format!("tool {tool:?}"), vec![Step::Do(A::SetTool(tool))])
        }
        // ---- painting ----
        25..=42 => {
            if edit_views.is_empty() {
                return ("stroke: no edit view, 3D view".to_owned(), vec![Step::Do(A::SetViewMode(ViewMode::View3D)), settle()]);
            }
            let view = f.pick(&edit_views);
            let tool = if f.chance(0.8) { ToolKind::Brush } else { ToolKind::Eraser };
            let size_step = f.below(11) as i32 - 4;
            let color = f.color();
            let mods = f.pick(&[
                egui::Modifiers::NONE,
                egui::Modifiers::NONE,
                egui::Modifiers::NONE,
                egui::Modifiers::NONE,
                egui::Modifiers::SHIFT,
                egui::Modifiers::ALT,
            ]);
            let ppf = 1 + f.below(4);
            let (what, path) = tool_path(f, state, view);
            (
                format!("{tool:?} stroke in {view:?} {what}, size step {size_step}, colour {color:?}, mods {mods:?}"),
                vec![
                    Step::Do(A::SetTool(tool)),
                    Step::Do(A::BrushSizeStep(size_step)),
                    Step::Do(A::SetPrimaryColor(color)),
                    pointer(view, path, egui::PointerButton::Primary, mods, ppf),
                ],
            )
        }
        43..=47 => {
            let Some(&view) = edit_views.first() else { return ("fill: no edit view".to_owned(), vec![Step::WaitFrames(1)]) };
            let view = if edit_views.len() > 1 && f.chance(0.3) { edit_views[1] } else { view };
            let at = tool_spot(f, state, view);
            let island = f.chance(0.7);
            let color = f.color();
            let scope: fn(&mut AppState, &mut Scratch) -> Result<(), String> = if island {
                |s, _| {
                    s.tools.fill_scope = FillScope::Island;
                    Ok(())
                }
            } else {
                |s, _| {
                    s.tools.fill_scope = FillScope::Layer;
                    Ok(())
                }
            };
            (
                format!("fill ({}) in {view:?} at {at:?}, colour {color:?}", if island { "island" } else { "layer" }),
                vec![
                    Step::Setup("fuzz: fill scope", scope),
                    Step::Do(A::SetTool(ToolKind::Fill)),
                    Step::Do(A::SetPrimaryColor(color)),
                    click(view, at),
                ],
            )
        }
        // ---- stamps ----
        48..=52 => {
            let Some(&view) = edit_views.first() else { return ("stamp: no edit view".to_owned(), vec![Step::WaitFrames(1)]) };
            let view = if edit_views.len() > 1 && f.chance(0.3) { edit_views[1] } else { view };
            let at = tool_spot(f, state, view);
            (format!("image stamp in {view:?} at {at:?}"), vec![Step::Do(A::SetTool(ToolKind::Stamp)), click(view, at)])
        }
        53..=56 => {
            let Some(&view) = edit_views.first() else { return ("text: no edit view".to_owned(), vec![Step::WaitFrames(1)]) };
            let view = if edit_views.len() > 1 && f.chance(0.3) { edit_views[1] } else { view };
            let at = tool_spot(f, state, view);
            let variant = f.below(TEXT_VARIANTS.len());
            let set_text: fn(&mut AppState, &mut Scratch) -> Result<(), String> = match variant {
                0 => |s, _| set_text_variant(s, 0),
                1 => |s, _| set_text_variant(s, 1),
                2 => |s, _| set_text_variant(s, 2),
                3 => |s, _| set_text_variant(s, 3),
                4 => |s, _| set_text_variant(s, 4),
                _ => |s, _| set_text_variant(s, 5),
            };
            (
                format!("text stamp {:?} in {view:?} at {at:?}", TEXT_VARIANTS[variant.min(TEXT_VARIANTS.len() - 1)]),
                vec![Step::Setup("fuzz: text content", set_text), Step::Do(A::SetTool(ToolKind::Text)), click(view, at)],
            )
        }
        57..=63 => {
            let gizmo = session.selected_stamp.and_then(|id| {
                let placement = doc.stamp(id)?.placement;
                edit_views.iter().find_map(|v| {
                    let xf = ViewXform::for_view(state, *v)?;
                    gizmo::gizmo_shape(&placement, &xf).map(|shape| (id, xf, shape))
                })
            });
            let Some((id, xf, shape)) = gizmo else {
                let Some(&view) = edit_views.first() else { return ("transform: no edit view".to_owned(), vec![Step::WaitFrames(1)]) };
                let at = tool_spot(f, state, view);
                return (format!("transform click in {view:?} at {at:?} (no visible selection)"), vec![Step::Do(A::SetTool(ToolKind::Transform)), click(view, at)]);
            };
            let size = (shape.corners[2] - shape.corners[0]).length().max(10.0);
            let jitter = |f: &mut Fuzz| egui::vec2(f.range(-1.0, 1.0), f.range(-1.0, 1.0)) * size * 0.6;
            let (what, points): (&str, Vec<egui::Pos2>) = match f.below(5) {
                0 => ("body", vec![shape.center, shape.center + jitter(f)]),
                1 => {
                    let c = shape.corners[f.below(4)];
                    let scale = f.range(0.3, 2.0);
                    ("corner", vec![c, shape.center + (c - shape.center) * scale])
                }
                2 => {
                    let r = shape.rotate_knob - shape.center;
                    let a0 = r.y.atan2(r.x);
                    let turn = f.range(-3.0, 3.0);
                    ("rotate knob", (0..=10).map(|k| shape.center + egui::vec2((a0 + turn * k as f32 / 10.0).cos(), (a0 + turn * k as f32 / 10.0).sin()) * r.length()).collect())
                }
                3 => {
                    let notches = f.pick(&[-3.0, -1.0, 1.0, 2.0]);
                    let mods = f.pick(&[egui::Modifiers::CTRL, egui::Modifiers::SHIFT]);
                    return (
                        format!("stamp {id:?}: wheel {notches} ({mods:?}) over the gizmo in {:?}", xf.view),
                        vec![
                            Step::Do(A::SetTool(ToolKind::Transform)),
                            Step::Scroll { view: xf.view, at: norm(&xf, shape.center), notches, mods },
                        ],
                    );
                }
                _ => ("body far", vec![shape.center, xf.rect.min + egui::vec2(f.unit() * xf.rect.width(), f.unit() * xf.rect.height())]),
            };
            (format!("stamp {id:?}: drag the {what} in {:?}", xf.view), vec![Step::Do(A::SetTool(ToolKind::Transform)), drag(&xf, &points)])
        }
        // ---- layers and sets ----
        64..=75 => {
            let Some(set) = doc.set(session.active_set) else { return ("layers: no active set".to_owned(), vec![Step::WaitFrames(1)]) };
            if set.layers.is_empty() {
                return ("add a paint layer to an empty set".to_owned(), vec![Step::Do(A::AddPaintLayer)]);
            }
            let layer = &set.layers[f.below(set.layers.len())];
            let id = layer.id;
            if f.below(13) == 5 {
                // An opacity slider drag: live previews, then the final value.
                let (a, b) = (f.unit(), f.unit());
                return (
                    format!("layer {:?}: opacity drag {a:.2} -> {b:.2}", layer.name),
                    vec![
                        Step::Do(A::SetLayerOpacity(id, a, Commit::Preview)),
                        Step::Do(A::SetLayerOpacity(id, (a + b) * 0.5, Commit::Preview)),
                        Step::Do(A::SetLayerOpacity(id, b, Commit::Final)),
                    ],
                );
            }
            // fix_export item 9: multi-select, the commands on a selection, rename, export and save + reopen.
            match f.below(20) {
                13 => return (format!("layer {:?}: Ctrl-click (toggle in the selection)", layer.name), vec![Step::Do(A::ToggleLayerSelected(id))]),
                14 => return (format!("layer {:?}: Shift-click (range select)", layer.name), vec![Step::Do(A::SelectLayerRange(id))]),
                15 => {
                    let selection = session.layer_selection();
                    let action = if f.chance(0.5) { A::DuplicateLayers(selection.clone()) } else { A::DeleteLayers(selection.clone()) };
                    return (format!("selection of {} layer(s): {action:?}", selection.len()), vec![Step::Do(action)]);
                }
                16 => {
                    let name = f.pick(&["Renamed", "Gl\u{fc}hen \u{2728}", "a:b/c", "", "  ", "CON", "\u{8eab}\u{4f53}"]).to_owned();
                    return (format!("layer {:?}: rename to {name:?}", layer.name), vec![Step::Do(A::RenameLayer(id, name))]);
                }
                17 | 18 => return ("export (to a temp folder)".to_owned(), vec![Step::Dyn("fuzz: export", fuzz_export)]),
                19 => return ("save and reopen the project".to_owned(), vec![Step::Dyn("fuzz: save and reopen", fuzz_save_reopen)]),
                _ => {}
            }
            let action = match f.below(13) {
                0 => A::AddPaintLayer,
                1 => A::DuplicateLayer(id),
                2 => A::DeleteLayer(id),
                3 => A::MoveLayer { layer: id, to_index: f.below(set.layers.len() + 1) },
                4 => A::SetLayerVisible(id, !layer.visible),
                5 => A::SetLayerOpacity(id, f.unit(), Commit::Final),
                6 => A::SetLayerBlend(id, f.pick(&BlendMode::ALL)),
                7 => A::SelectLayer(id),
                8 => A::MergeDown(id),
                9 => A::RasterizeLayer(id),
                10 => A::SetLayerLocked(id, !layer.locked),
                11 => A::SetLayerAlphaLock(id, !layer.alpha_lock),
                _ => A::MoveActiveLayer(if f.chance(0.5) { 1 } else { -1 }),
            };
            (format!("layer {:?} ({}): {action:?}", layer.name, layer.kind_name()), vec![Step::Do(action)])
        }
        76..=78 => {
            let set = TextureSetId(f.below(doc.sets.len()) as u32);
            (format!("active texture set {}", set.0), vec![Step::Do(A::SetActiveSet(set)), settle()])
        }
        // ---- history ----
        79..=85 => {
            let k = 1 + f.below(10);
            let undo_bias = f.range(0.3, 0.9);
            let keys = f.chance(0.3);
            let cmd = egui::Modifiers::CTRL.plus(egui::Modifiers::COMMAND);
            let mut seq = String::new();
            let steps = (0..k)
                .map(|_| {
                    let undo = f.chance(undo_bias);
                    seq.push(if undo { 'U' } else { 'R' });
                    match (undo, keys) {
                        (true, false) => Step::Do(A::Undo),
                        (false, false) => Step::Do(A::Redo),
                        (true, true) => Step::Key(cmd, egui::Key::Z),
                        (false, true) => Step::Key(cmd, egui::Key::Y),
                    }
                })
                .collect();
            (format!("undo/redo burst {seq} ({})", if keys { "keys" } else { "actions" }), steps)
        }
        // ---- window ----
        86..=88 => {
            let size = if f.chance(0.6) { f.pick(RESIZES) } else { [640 + f.below(1300) as u32, 480 + f.below(620) as u32] };
            (format!("resize the window to {}x{}", size[0], size[1]), vec![Step::Resize(size), settle()])
        }
        89..=93 => {
            let action = match f.below(12) {
                0 => A::ToggleWireframe,
                1 => A::ToggleSeams,
                2 => A::ToggleUvWire,
                3 => A::ToggleAlphaChecker,
                4 => A::ToggleLit,
                5 => A::ToggleTexture,
                6 => A::SetShading(f.pick(&[ShadingMode::Textured, ShadingMode::Clay, ShadingMode::UvChecker])),
                7 => A::FrameModel,
                8 => A::ResetCamera,
                9 => A::SwapColors,
                10 => A::BrushHardnessStep(f.below(5) as i32 - 2),
                _ => A::SetCameraPreset(f.pick(&[
                    CameraPreset::Front,
                    CameraPreset::Back,
                    CameraPreset::Left,
                    CameraPreset::Right,
                    CameraPreset::Top,
                    CameraPreset::Bottom,
                ])),
            };
            (format!("view: {action:?}"), vec![Step::Do(action)])
        }
        94..=96 => {
            let cmd = egui::Modifiers::CTRL.plus(egui::Modifiers::COMMAND);
            let (mods, key) = f.pick(&[
                (egui::Modifiers::NONE, egui::Key::Escape),
                (egui::Modifiers::NONE, egui::Key::Delete),
                (egui::Modifiers::NONE, egui::Key::Num1),
                (egui::Modifiers::NONE, egui::Key::Num2),
                (egui::Modifiers::NONE, egui::Key::Num3),
                (egui::Modifiers::NONE, egui::Key::F5),
                (egui::Modifiers::NONE, egui::Key::F),
                (egui::Modifiers::NONE, egui::Key::T),
                (egui::Modifiers::NONE, egui::Key::W),
                (egui::Modifiers::NONE, egui::Key::OpenBracket),
                (egui::Modifiers::NONE, egui::Key::CloseBracket),
                (egui::Modifiers::NONE, egui::Key::B),
                (egui::Modifiers::NONE, egui::Key::R),
                (egui::Modifiers::NONE, egui::Key::X),
                (egui::Modifiers::NONE, egui::Key::D),
                (cmd, egui::Key::J),
                (cmd, egui::Key::M),
                (cmd, egui::Key::E),
                (cmd, egui::Key::CloseBracket),
                (cmd, egui::Key::OpenBracket),
                (cmd.plus(egui::Modifiers::SHIFT), egui::Key::N),
            ]);
            (format!("key {mods:?} {key:?}"), vec![Step::Key(mods, key), settle()])
        }
        _ => {
            if doc.stamps.is_empty() {
                return ("stamp ops: no stamp, deselect".to_owned(), vec![Step::Do(A::SelectStamp(None))]);
            }
            let stamp = &doc.stamps[f.below(doc.stamps.len())];
            let action = match (f.below(9), &stamp.source) {
                (0, _) => A::SelectStamp(Some(stamp.id)),
                (1, _) => A::SelectStamp(None),
                (2, _) => A::DeleteStamp(stamp.id),
                (3, _) => A::UpdateStampTint(stamp.id, f.color(), Commit::Final),
                // Round 4 (FEATURE_TEXT 8.3): a random gap value, warp or line of a text, or its effects cleared.
                (k @ 5..=8, StampSource::Text(spec)) => {
                    let mut spec = spec.clone();
                    fuzz_text_fx(f, &mut spec, k - 5);
                    let commit = if f.chance(0.2) { Commit::Preview } else { Commit::Final };
                    A::UpdateStampSource(stamp.id, StampSource::Text(spec), commit)
                }
                (5..=8, StampSource::Image { .. }) => A::SelectStamp(Some(stamp.id)),
                _ => A::DeleteSelection,
            };
            let mut steps = vec![Step::Do(action.clone())];
            if let A::UpdateStampSource(id, StampSource::Text(spec), Commit::Preview) = &action {
                // A Preview is always followed by its Final, as a panel control ends its drag.
                steps.push(Step::Do(A::UpdateStampSource(*id, StampSource::Text(spec.clone()), Commit::Final)));
            }
            (format!("stamp {:?}: {action:?}", stamp.name), steps)
        }
    }
}

/// fix_export item 9: a fuzz export into `<out_stem>_fuzz_export`: everything, the selection or one random layer; loose
/// files or a zip; some formats. Afterwards no temp file may be left in the folder.
fn fuzz_export(state: &AppState, sc: &Scratch) -> Result<Vec<Step>, String> {
    let mut f = FUZZ.lock().unwrap_or_else(|e| e.into_inner());
    let session = session_of(state)?;
    let doc = &session.doc;
    let layers = match f.below(3) {
        0 => None,
        1 => Some(session.layer_selection()),
        _ => {
            let all: Vec<LayerId> = doc.sets.iter().flat_map(|s| &s.layers).map(|l| l.id).collect();
            if all.is_empty() { None } else { Some(vec![f.pick(&all)]) }
        }
    };
    let mut opts = ExportOptions {
        dir: with_suffix(&sc.out_stem, "_fuzz_export"),
        base_name: doc.name.clone(),
        sets: None,
        flat: f.chance(0.5).then_some(FlatFormat::Png(PngLevel::Fast)),
        per_layer_png: f.chance(0.8),
        ora: f.chance(0.15),
        psd: f.chance(0.15),
        include_hidden: f.chance(0.5),
        dilate_px: if f.chance(0.2) { 2 } else { 0 },
        layers,
        package: if f.chance(0.5) { wte_core::ExportPackage::Zip } else { wte_core::ExportPackage::Files },
    };
    // At least one kind of file, as the dialog demands ("Choose at least one kind of file").
    if opts.flat.is_none() && !opts.per_layer_png && !opts.ora && !opts.psd {
        opts.per_layer_png = true;
    }
    log::info!("fuzz: export {:?} layers {:?} flat {:?} per-layer {} ora {} psd {}", opts.package, opts.layers, opts.flat, opts.per_layer_png, opts.ora, opts.psd);
    Ok(vec![
        Step::Do(AppAction::RunExport(opts)),
        Step::WaitIdle,
        Step::Dyn("fuzz: close a message", |s, _| {
            Ok(if matches!(s.dialog, Dialog::Message { .. }) { vec![Step::Do(AppAction::CloseDialog), Step::WaitFrames(2)] } else { Vec::new() })
        }),
        Step::Assert("fuzz: the export left no temp file", |_, sc| {
            let dir = with_suffix(&sc.out_stem, "_fuzz_export");
            if !dir.exists() {
                return Ok(());
            }
            let names = dir_names(&dir)?;
            if names.iter().any(|n| n.contains(".tmp-")) {
                return Err(format!("a temp file or folder is left: {names:?}"));
            }
            Ok(())
        }),
    ])
}

/// fix_export item 9: the fuzz saves the project as `<out_stem>_fuzz.wtep` and opens it again (the history starts
/// over); the stamp tool gets its image again.
fn fuzz_save_reopen(_state: &AppState, sc: &Scratch) -> Result<Vec<Step>, String> {
    let path = with_suffix(&sc.out_stem, "_fuzz.wtep");
    Ok(vec![
        Step::Do(AppAction::SaveAs(Some(path.clone()))),
        Step::WaitIdle,
        Step::Do(AppAction::OpenProject(Some(path))),
        Step::WaitIdle,
        Step::Setup("fuzz: the stamp image again", |s, sc| {
            if s.session.is_some() {
                setup_stamp_image(s, sc)?;
            }
            Ok(())
        }),
        Step::Assert("fuzz: the reopened project is open and unmodified", |s, sc| {
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, "_fuzz.wtep").as_path()) {
                return Err(format!("the open document is {:?} (dialog {})", session.path, dialog_name(&s.dialog)));
            }
            Ok(())
        }),
    ])
}

/// One of the four text operations of the fuzz run (round 4): `0` a random gap value (sometimes out of range, which
/// `sanitize_fx` repairs), `1` a random warp, `2` a random line (Arc, Circle or Curve, sometimes beyond the
/// coordinate range), `3` every effect cleared.
fn fuzz_text_fx(f: &mut Fuzz, spec: &mut wte_core::TextSpec, op: usize) {
    use wte_core::{PathShape, TextPath, Warp, WarpStyle};
    match op {
        0 => {
            let chars = spec.content.chars().count().max(1);
            let after = f.below(chars + 1) as u32;
            let em = if f.chance(0.1) { f.range(-3.0, 9.0) } else { f.range(-0.5, 1.5) };
            spec.fx.gaps.retain(|g| g.after != after);
            spec.fx.gaps.push(wte_core::Gap { after, em });
        }
        1 => {
            let style = f.pick(&WarpStyle::ALL);
            spec.fx.warp = Some(Warp {
                style,
                bend: f.range(-1.0, 1.0),
                horizontal: if f.chance(0.3) { f.range(-1.0, 1.0) } else { 0.0 },
                vertical: if f.chance(0.3) { f.range(-1.0, 1.0) } else { 0.0 },
                vertical_axis: f.chance(0.2),
            });
        }
        2 => {
            let shape = match f.below(3) {
                0 => PathShape::Arc { bend: f.range(-1.0, 1.0) },
                1 => PathShape::Circle { diameter: f.range(0.2, 30.0), center: [f.range(-3.0, 3.0), f.range(-3.0, 6.0)] },
                _ => {
                    let n = 2 + f.below(5);
                    let reach = if f.chance(0.1) { 90.0 } else { 8.0 };
                    PathShape::Curve { points: (0..n).map(|_| [f.range(-reach, reach), f.range(-reach / 3.0, reach / 3.0)]).collect() }
                }
            };
            spec.fx.path = Some(TextPath { shape, start: if f.chance(0.3) { f.range(-5.0, 5.0) } else { 0.0 }, flip: f.chance(0.2) });
        }
        _ => spec.fx = wte_core::TextFx::default(),
    }
}

fn set_text_variant(state: &mut AppState, variant: usize) -> Result<(), String> {
    state.tools.text.content = TEXT_VARIANTS[variant.min(TEXT_VARIANTS.len() - 1)].to_owned();
    state.tools.text.em_frac = [0.02, 0.06, 0.12][variant % 3];
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_brush_types.rs (pasted by the gate, sitting M2)
// Round 4, package B7 (author of the scenario, ROUND4_PLAN 2.2 item 11): scenario `brush_types` (FEATURE_BRUSH 8.3 S1,
// ---------------------------------------------------------------------------------------------------------------------

/// Cube sample, camera preset Front, Size 1024, hardness 1: a click at the centre of the view with Circular paints
/// every face but the back one; after Undo the composite is the one before; a click with Sphere paints all six faces
/// (the ball holds the whole cube). One history entry per click; an error toast fails the run. Then a hover with the
/// Cube shape (Turn 30): the shape cursor has a footprint and wires, and the status bar names the shape.
fn brush_types() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Cube)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::SetCameraPreset(CameraPreset::Front)),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::WaitIdle,
        Step::Setup("Circular, Size 1024, hardness 1; remember the composite and the history", |s, sc| {
            s.tools.brush = BrushSettings { size_px: 1024.0, hardness: 1.0, kind: BrushType::Circular, ..BrushSettings::default() };
            remember_composite(s, sc, "bt_before")?;
            sc.values.insert("bt_undo", doc_of(s)?.history.undo_len() as u64);
            Ok(())
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("Circular: the back face has 0 changed texels, each other face more than 100,000", |s, sc| {
            let counts = cube_face_counts(s, "Circular")?;
            let back = back_face(s)?;
            for (face, n) in counts.iter().enumerate() {
                if face == back && *n != 0 {
                    return Err(format!("the back face {} has {n} changed texels: {counts:?}", FACE_NAMES[face]));
                }
                if face != back && *n <= 100_000 {
                    return Err(format!("the face {} has only {n} changed texels: {counts:?}", FACE_NAMES[face]));
                }
            }
            history_grew(s, sc, 1)
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("after Undo the composite is the one before the click", |s, sc| {
            if composite_hash(doc_of(s)?) != remembered(sc, "bt_before")? {
                return Err("the composite differs from the one before the Circular click".to_owned());
            }
            history_grew(s, sc, 0)
        }),
        Step::Setup("Sphere", |s, _| {
            s.tools.brush.kind = BrushType::Sphere;
            Ok(())
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("Sphere: all six faces have more than 100,000 changed texels", |s, sc| {
            let counts = cube_face_counts(s, "Sphere")?;
            if let Some(face) = counts.iter().position(|&n| n <= 100_000) {
                return Err(format!("the face {} has only {} changed texels: {counts:?}", FACE_NAMES[face], counts[face]));
            }
            history_grew(s, sc, 1)
        }),
        // The shape cursor (R4-7): Sphere with the Cube, turned 30 degrees, hovering over the front face.
        Step::Setup("Sphere with the Cube, Size 600, Turn 30", |s, _| {
            s.tools.brush = BrushSettings {
                size_px: 600.0,
                kind: BrushType::Sphere,
                shape: ShapeChoice::Cube,
                turn_deg: 30.0,
                ..BrushSettings::default()
            };
            Ok(())
        }),
        Step::Hover { view: ViewId::Main3D, at: [0.5, 0.5] },
        Step::WaitFrames(3),
        Step::Assert("the shape cursor and the status bar hint are shown", |s, _| {
            let probe = &s.tool_probe;
            log::info!("brush_types: cursor footprint {} segments, wires {}", probe.cursor_footprint, probe.cursor_wires);
            if probe.cursor_footprint == 0 || probe.cursor_wires == 0 {
                return Err(format!("cursor footprint {} / wires {} segments", probe.cursor_footprint, probe.cursor_wires));
            }
            if s.status_hint != "Shape brush: Cube" {
                return Err(format!("status bar hint {:?}", s.status_hint));
            }
            Ok(())
        }),
    ]
}

/// The faces of the cube in the order `cube_face_counts` counts them.
const FACE_NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

/// The face of the cube that points away from the camera.
fn back_face(state: &AppState) -> Result<usize, String> {
    let (_, _, forward) = session_of(state)?.camera.basis();
    let normals = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    normals.iter().position(|n| n.dot(forward) > 0.9).ok_or_else(|| format!("the camera looks along {forward:?}"))
}

/// Changed texels (alpha > 0 on the active paint layer, which was empty before the click) per face of the cube sample,
/// each face's island found from the UVs of its triangles. Logs the counts.
fn cube_face_counts(state: &AppState, label: &str) -> Result<[u64; 6], String> {
    let doc = doc_of(state)?;
    let set = doc.sets.first().ok_or("no texture set")?;
    let active = set.active.ok_or("no active layer")?;
    let LayerKind::Raster(paint) = &set.layer(active).ok_or("no active layer")?.kind else {
        return Err("the active layer is not a paint layer".to_owned());
    };
    let size = Vec2::new(set.width as f32, set.height as f32);
    let normals = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    let mut rects = [(Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)); 6];
    for t in 0..doc.mesh.tri_count() {
        let [a, b, c] = doc.mesh.tri_positions(t);
        let n = (b - a).cross(c - a).normalize_or_zero();
        let face = normals.iter().position(|m| m.dot(n) > 0.9).ok_or("a triangle that is not a cube face")?;
        for uv in doc.mesh.tri_uvs(t) {
            rects[face].0 = rects[face].0.min(uv * size);
            rects[face].1 = rects[face].1.max(uv * size);
        }
    }
    let mut counts = [0u64; 6];
    for (face, (lo, hi)) in rects.iter().enumerate() {
        for y in lo.y.max(0.0).floor() as u32..(hi.y.ceil() as u32).min(set.height) {
            for x in lo.x.max(0.0).floor() as u32..(hi.x.ceil() as u32).min(set.width) {
                counts[face] += (paint.data[((y * set.width + x) * 4 + 3) as usize] > 0) as u64;
            }
        }
    }
    log::info!("brush_types {label}: changed texels per face {:?}", FACE_NAMES.iter().zip(counts).collect::<Vec<_>>());
    Ok(counts)
}

/// The history has `more` entries more than at "bt_undo" (one per click; Undo takes one back).
fn history_grew(state: &AppState, scratch: &Scratch, more: u64) -> Result<(), String> {
    let undo = doc_of(state)?.history.undo_len() as u64;
    let before = remembered(scratch, "bt_undo")?;
    if undo != before + more {
        return Err(format!("{undo} history entries, expected {}", before + more));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_brush_types_path.rs (pasted by the gate, sitting M2)
// Round 4, package B7 (author of the scenario, ROUND4_PLAN 2.2 item 11): scenario `brush_types_path` (FEATURE_BRUSH
// ---------------------------------------------------------------------------------------------------------------------

/// Needs a model path (like `workflow_path`). Opens the model, finds a stroke path with the helpers `workflow_path`
/// uses, paints one warm-up stroke and undoes it, then paints the same path with Circular, Sphere and Sphere with the
/// Cube, each at Size 50 and Size 512, with a log line naming type and size before each stroke and an Undo after it.
/// Each stroke changes pixels and adds one history entry; each Undo restores the composite.
fn brush_types_path(has_path: bool) -> Vec<Step> {
    let mut steps = vec![phase!("open / import")];
    steps.extend(open_and_accept(has_path));
    if !has_path {
        return steps;
    }
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::WaitIdle,
        phase!("warm-up stroke"),
        Step::Setup("brush_types_path: warm-up stroke (Circular, Size 50)", |s, sc| set_path_tip(s, sc, BrushType::Circular, ShapeChoice::Ball, 50.0)),
        Step::Dyn("warm-up stroke on the model", |s, _| hit_stroke(s, 0)),
        Step::WaitIdle,
        Step::Assert("the warm-up stroke painted", path_stroke_painted),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("Undo restored the composite", path_stroke_undone),
    ]);
    type SetTip = fn(&mut AppState, &mut Scratch) -> Result<(), String>;
    let runs: [(&'static str, SetTip); 6] = [
        ("brush_types_path: Circular, Size 50", |s, sc| set_path_tip(s, sc, BrushType::Circular, ShapeChoice::Ball, 50.0)),
        ("brush_types_path: Sphere, Size 50", |s, sc| set_path_tip(s, sc, BrushType::Sphere, ShapeChoice::Ball, 50.0)),
        ("brush_types_path: Sphere with the Cube, Size 50", |s, sc| set_path_tip(s, sc, BrushType::Sphere, ShapeChoice::Cube, 50.0)),
        ("brush_types_path: Circular, Size 512", |s, sc| set_path_tip(s, sc, BrushType::Circular, ShapeChoice::Ball, 512.0)),
        ("brush_types_path: Sphere, Size 512", |s, sc| set_path_tip(s, sc, BrushType::Sphere, ShapeChoice::Ball, 512.0)),
        ("brush_types_path: Sphere with the Cube, Size 512", |s, sc| set_path_tip(s, sc, BrushType::Sphere, ShapeChoice::Cube, 512.0)),
    ];
    for (label, setup) in runs {
        steps.extend([
            Step::Setup(label, setup),
            Step::Dyn("the stroke on the model", |s, _| hit_stroke(s, 0)),
            Step::WaitIdle,
            Step::Assert("the stroke changed pixels and is one history entry", path_stroke_painted),
            Step::Do(AppAction::Undo),
            Step::WaitIdle,
            Step::Assert("Undo restored the composite", path_stroke_undone),
        ]);
    }
    steps
}

/// Sets the brush for one stroke of `brush_types_path`, logs type and size, and remembers the composite and the
/// history length.
fn set_path_tip(state: &mut AppState, scratch: &mut Scratch, kind: BrushType, shape: ShapeChoice, size: f32) -> Result<(), String> {
    state.tools.brush = BrushSettings { kind, shape, size_px: size, ..BrushSettings::default() };
    log::info!("brush_types_path: next stroke {kind:?} / {shape:?}, Size {size}");
    let doc = doc_of(state)?;
    scratch.values.insert("btp_hash", composite_hash(doc));
    scratch.values.insert("btp_undo", doc.history.undo_len() as u64);
    Ok(())
}

fn path_stroke_painted(state: &AppState, scratch: &Scratch) -> Result<(), String> {
    let doc = doc_of(state)?;
    if composite_hash(doc) == remembered(scratch, "btp_hash")? {
        return Err("the stroke changed no pixel".to_owned());
    }
    let (undo, before) = (doc.history.undo_len() as u64, remembered(scratch, "btp_undo")?);
    if undo != before + 1 {
        return Err(format!("{undo} history entries after the stroke, expected {}", before + 1));
    }
    Ok(())
}

fn path_stroke_undone(state: &AppState, scratch: &Scratch) -> Result<(), String> {
    let doc = doc_of(state)?;
    if composite_hash(doc) != remembered(scratch, "btp_hash")? {
        return Err("the composite after Undo differs from the one before the stroke".to_owned());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_brush_shape.rs (pasted by the gate, sitting M2)
// Round 4, package B5 (author of the scenario, ROUND4_PLAN 2.2 item 11): scenario `brush_shape` (FEATURE_BRUSH 8.3
// ---------------------------------------------------------------------------------------------------------------------

/// Cube sample, camera preset Front, Brush tool. The cube `seam_cube(1.0)` written as OBJ is loaded as the brush shape
/// (`LoadBrushShape(Some(path))`): 12 triangles, not partial; the brush becomes Sphere / File / Solid, the eraser is
/// unchanged, the Brush tab is shown. Size 200, hardness 1: one click at the centre of the view paints a square on the
/// front face only (13,333 texels within 5 %, 90.9 by 146.8 texels within 4); Undo restores the composite; Turn 45: a
/// turned square (128.5 by 207.6 within 4), undone; a 40-point stroke is one history entry; a hover with the Brush tab open is
/// the screenshot, then Undo of the stroke. Then the same cube from a GLB and an ASCII FBX file: each load sets the prefs path, each click
/// paints the same square (count within 5 %, box within 4 texels of the OBJ click). Last (plan section 7 item 5): an
/// open box (`open_box(1.0)`) makes the app ask; "Flat caps" and "Close" are clicked in the dialog; the box is closed
/// (one opening closed), the choice is stored with the path, the preview of the Brush tab has lines, and a click
/// paints the same square as the closed cube. No error toast.
fn brush_shape() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Cube)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::SetCameraPreset(CameraPreset::Front)),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::WaitIdle,
        Step::Setup("write the cube as OBJ, GLB and ASCII FBX, and an open box as OBJ", bs_write_assets),
        Step::Dyn("load the OBJ cube as the brush shape", |_, sc| bs_load(sc, "cube_shape.obj")),
        Step::WaitIdle,
        Step::Assert("the OBJ cube is the shape; the brush is Sphere / File / Solid; the Brush tab is shown", |s, sc| {
            bs_loaded(s, sc, "cube_shape.obj")
        }),
        Step::Setup("Size 200, hardness 1; remember the composite and the history", |s, sc| {
            s.tools.brush.size_px = 200.0;
            s.tools.brush.hardness = 1.0;
            remember_composite(s, sc, "bs_before")?;
            remember_undo(s, sc)
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("first click: only the front face, 13,333 texels within 5 %, 90.9 by 146.8 within 4", |s, sc| {
            let (count, size) = bs_front_square(s, "OBJ click")?;
            bs_count_near(count, 13_333.0, "OBJ click")?;
            bs_size_near(size, [90.9, 146.8], 4.0, "OBJ click")?;
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember the box of the first click", |s, sc| {
            let (count, size) = bs_front_square(s, "OBJ click")?;
            sc.values.insert("bs_count", count);
            sc.values.insert("bs_w", (size[0] * 100.0) as u64);
            sc.values.insert("bs_h", (size[1] * 100.0) as u64);
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("Undo restores the composite", bs_restored),
        Step::Setup("Turn 45", |s, _| {
            s.tools.brush.turn_deg = 45.0;
            Ok(())
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("Turn 45: 13,333 texels within 5 %, 128.5 by 207.6 within 4", |s, sc| {
            let (count, size) = bs_front_square(s, "Turn 45 click")?;
            bs_count_near(count, 13_333.0, "Turn 45 click")?;
            bs_size_near(size, [128.5, 207.6], 4.0, "Turn 45 click")?;
            undo_since(s, sc, 1)
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("Undo restores the composite", bs_restored),
        Step::Setup("remember the history before the stroke", remember_undo),
        Step::Dyn("a 40-point stroke across the front face", bs_stroke),
        Step::WaitIdle,
        Step::Assert("the stroke is exactly one more history entry", |s, sc| undo_since(s, sc, 1)),
        Step::Hover { view: ViewId::Main3D, at: [0.5, 0.5] },
        Step::WaitFrames(3),
        Step::Assert("the Brush tab is open while hovering with the shape", |s, _| bs_tab_shown(s)),
        // Gate 1: a named capture (`<out>_hover.png`), so the final screenshot no longer overwrites this frame.
        Step::Shot("hover"),
        // The same cube from a GLB and from an FBX file: one click each, then Undo.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("Undo of the stroke restores the composite before the first click", |s, sc| {
            if composite_hash(doc_of(s)?) != remembered(sc, "bs_before")? {
                return Err("the composite differs from the one before the first click".to_owned());
            }
            Ok(())
        }),
        Step::Setup("Turn 0", |s, _| {
            s.tools.brush.turn_deg = 0.0;
            Ok(())
        }),
        Step::Dyn("load the GLB cube", |_, sc| bs_load(sc, "cube_shape.glb")),
        Step::WaitIdle,
        Step::Assert("the GLB cube is the shape; the prefs path is the GLB file", |s, sc| bs_loaded(s, sc, "cube_shape.glb")),
        Step::Setup("remember the composite and the history", |s, sc| {
            remember_composite(s, sc, "bs_before")?;
            remember_undo(s, sc)
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("GLB click: the square of the OBJ click", |s, sc| bs_same_square(s, sc, "GLB click")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("Undo restores the composite", bs_restored),
        Step::Dyn("load the FBX cube", |_, sc| bs_load(sc, "cube_shape.fbx")),
        Step::WaitIdle,
        Step::Assert("the FBX cube is the shape; the prefs path is the FBX file", |s, sc| bs_loaded(s, sc, "cube_shape.fbx")),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("FBX click: the square of the OBJ click", |s, sc| bs_same_square(s, sc, "FBX click")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("Undo restores the composite", bs_restored),
        // Plan section 7 item 5: an open model, closed with Flat caps through the dialog.
        Step::Dyn("load the open box", |_, sc| bs_load(sc, "open_box.obj")),
        Step::WaitIdle,
        Step::Assert("the app asks how to close the one opening; the FBX cube stays in use", |s, sc| {
            let Dialog::CloseOpenings(st) = &s.dialog else { return Err(format!("no question; the dialog is {:?}", s.dialog)) };
            log::info!(
                "brush_shape: {} has {} opening(s), the largest {:.1} %, closed parts {}",
                st.path.display(),
                st.count,
                st.largest * 100.0,
                st.closed_parts
            );
            if st.path != assets_dir(sc).join("open_box.obj") || st.count != 1 || st.closed_parts || st.choice != CloseMode::FlatCaps {
                return Err(format!("the question: {st:?}"));
            }
            for hit in [PanelHit::OpeningsChoice(0), PanelHit::OpeningsChoice(2), PanelHit::OpeningsOk, PanelHit::OpeningsCancel] {
                panel_rect(s, hit)?;
            }
            if s.panel_rects.contains_key(&PanelHit::OpeningsChoice(3)) {
                return Err("\"Use only the closed parts\" is offered for a file without closed parts".to_owned());
            }
            let current = s.brush_shape.current.as_ref().ok_or("no shape in use")?;
            if current.path != assets_dir(sc).join("cube_shape.fbx") {
                return Err(format!("the shape in use is {}", current.path.display()));
            }
            Ok(())
        }),
        // Gate 3 (ROUND4_PLAN 3.3 item 7): the closing dialog of an open model, `<out>_dialog.png`.
        Step::Shot("dialog"),
        Step::Dyn("click \"Flat caps\" in the dialog", |s, _| {
            Ok(vec![Step::ScreenPointer { path: vec![panel_rect(s, PanelHit::OpeningsChoice(0))?.center()] }])
        }),
        Step::WaitFrames(2),
        Step::Dyn("click the dialog's Close button", |s, _| {
            Ok(vec![Step::ScreenPointer { path: vec![panel_rect(s, PanelHit::OpeningsOk)?.center()] }])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the open box is closed with Flat caps, stored with its path; the preview is not empty", |s, sc| {
            if !matches!(s.dialog, Dialog::None) {
                return Err(format!("a dialog is still open: {:?}", s.dialog));
            }
            let open = assets_dir(sc).join("open_box.obj");
            let current = s.brush_shape.current.as_ref().ok_or("no shape")?;
            let report = current.field.report();
            log::info!("brush_shape: open box closed with {}: {report:?}", current.close.label());
            if current.path != open || current.close != CloseMode::FlatCaps || report.openings_closed != 1 {
                return Err(format!("the shape {} closed {:?}, report {report:?}", current.path.display(), current.close));
            }
            if s.prefs.brush_shape_path.as_ref() != Some(&open) || s.prefs.brush_shape_close.get(&open) != Some(&CloseMode::FlatCaps) {
                return Err(format!("prefs: {:?} / {:?}", s.prefs.brush_shape_path, s.prefs.brush_shape_close));
            }
            let lines: usize = current.field.principal_sections().iter().map(Vec::len).sum();
            log::info!("brush_shape: the preview of the closed box has {lines} section segments");
            if lines == 0 || current.field.principal_sections().iter().any(Vec::is_empty) {
                return Err("the preview sections are empty".to_owned());
            }
            bs_tab_shown(s)
        }),
        Step::Setup("remember the composite and the history", |s, sc| {
            remember_composite(s, sc, "bs_before")?;
            remember_undo(s, sc)
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::Assert("the closed box paints the square of the closed cube", |s, sc| bs_same_square(s, sc, "closed box click")),
        Step::Assert("no error toast", |s, _| {
            if s.toasts.error_count() > 0 { Err(format!("error toasts: {:?}", s.toasts.error_texts())) } else { Ok(()) }
        }),
    ]
}

/// Writes `cube_shape.obj` (+ `.mtl`), `cube_shape.glb`, `cube_shape.fbx` (`seam_cube(1.0)`) and `open_box.obj`
/// (+ `.mtl`, `open_box(1.0)`: one opening) into `<out>_assets`.
fn bs_write_assets(_state: &mut AppState, scratch: &mut Scratch) -> Result<(), String> {
    let dir = fresh_assets_dir(scratch)?;
    let write = |name: &str, bytes: &[u8]| std::fs::write(dir.join(name), bytes).map_err(|e| format!("{name}: {e}"));
    let cube = samples::seam_cube(1.0);
    for (name, raw) in [("cube_shape", &cube), ("open_box", &samples::open_box(1.0))] {
        let (obj, mtl) = samples::to_obj(raw, &format!("{name}.mtl"), &[]);
        write(&format!("{name}.obj"), obj.as_bytes())?;
        write(&format!("{name}.mtl"), mtl.as_bytes())?;
    }
    write("cube_shape.glb", &samples::to_glb(&[(cube.clone(), glam::Mat4::IDENTITY)]))?;
    write("cube_shape.fbx", samples::to_fbx_ascii(&cube, 1.0, [1.0, 1.0, 1.0]).as_bytes())
}

/// `LoadBrushShape(Some(<out>_assets\<name>))`.
fn bs_load(scratch: &Scratch, name: &str) -> Result<Vec<Step>, String> {
    Ok(vec![Step::Do(AppAction::LoadBrushShape(Some(assets_dir(scratch).join(name))))])
}

/// After a "Load..." of the cube from `name`: the shape (12 triangles, not partial, no openings), the brush on Sphere /
/// File / Solid, the eraser unchanged, the prefs path, the Brush tab shown, no error toast.
fn bs_loaded(state: &AppState, scratch: &Scratch, name: &str) -> Result<(), String> {
    let path = assets_dir(scratch).join(name);
    let current = state.brush_shape.current.as_ref().ok_or("no brush shape is loaded")?;
    let report = current.field.report();
    log::info!("brush_shape: loaded {name}: {report:?}");
    if current.path != path || report.triangles != 12 || report.partial || current.close != CloseMode::Refuse {
        return Err(format!("the shape is {} with {report:?}, closed {:?}", current.path.display(), current.close));
    }
    if state.brush_shape.pending.is_some() || state.brush_shape.last_error.is_some() {
        return Err(format!("pending {:?}, last error {:?}", state.brush_shape.pending, state.brush_shape.last_error));
    }
    let b = &state.tools.brush;
    if (b.kind, b.shape, b.through) != (BrushType::Sphere, ShapeChoice::File, false) {
        return Err(format!("the brush is {:?} / {:?} / through {}", b.kind, b.shape, b.through));
    }
    if state.tools.eraser != BrushSettings::default() {
        return Err(format!("the eraser changed: {:?}", state.tools.eraser));
    }
    if state.prefs.brush_shape_path.as_ref() != Some(&path) {
        return Err(format!("the prefs path is {:?}", state.prefs.brush_shape_path));
    }
    if state.toasts.error_count() > 0 {
        return Err(format!("error toasts: {:?}", state.toasts.error_texts()));
    }
    bs_tab_shown(state)
}

/// The Brush tab is the chosen tab and its content was drawn in the last frame.
fn bs_tab_shown(state: &AppState) -> Result<(), String> {
    if state.side_tab != SideTab::Brush || state.side_rects.brush_tab.is_none() {
        return Err(format!("side tab {:?}, Brush tab drawn: {}", state.side_tab, state.side_rects.brush_tab.is_some()));
    }
    Ok(())
}

/// The composite is the one remembered as "bs_before" and the history is back where it was.
fn bs_restored(state: &AppState, scratch: &Scratch) -> Result<(), String> {
    if composite_hash(doc_of(state)?) != remembered(scratch, "bs_before")? {
        return Err("the composite differs from the one before the click".to_owned());
    }
    undo_since(state, scratch, 0)
}

/// A 40-point stroke across the front face (disc coordinates -0.6..0.6 at a third below the centre).
fn bs_stroke(state: &AppState, _scratch: &Scratch) -> Result<Vec<Step>, String> {
    let (xf, c, r) = model_disc(state, ViewId::Main3D)?;
    let path = (0..40)
        .map(|i| {
            let t = i as f32 / 39.0;
            norm(&xf, c + egui::vec2((-0.6 + 1.2 * t) * r, 0.3 * r))
        })
        .collect();
    Ok(vec![Step::Pointer {
        view: ViewId::Main3D,
        path,
        button: egui::PointerButton::Primary,
        mods: egui::Modifiers::NONE,
        points_per_frame: POINTS_PER_FRAME,
    }])
}

/// Changed texels (alpha > 0 on the active paint layer, empty before the click) of the cube sample's face that looks at
/// the camera, and the size (width, height in texels) of their bounding box. Fails when another face has a changed
/// texel. Logs the numbers.
fn bs_front_square(state: &AppState, label: &str) -> Result<(u64, [f32; 2]), String> {
    let doc = doc_of(state)?;
    let set = doc.sets.first().ok_or("no texture set")?;
    let active = set.active.ok_or("no active layer")?;
    let LayerKind::Raster(paint) = &set.layer(active).ok_or("no active layer")?.kind else {
        return Err("the active layer is not a paint layer".to_owned());
    };
    let (_, _, forward) = session_of(state)?.camera.basis();
    let normals = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    let front = normals.iter().position(|n| n.dot(forward) < -0.9).ok_or_else(|| format!("the camera looks along {forward:?}"))?;
    let size = Vec2::new(set.width as f32, set.height as f32);
    let mut rects = [(Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)); 6];
    for t in 0..doc.mesh.tri_count() {
        let [a, b, c] = doc.mesh.tri_positions(t);
        let n = (b - a).cross(c - a).normalize_or_zero();
        let face = normals.iter().position(|m| m.dot(n) > 0.9).ok_or("a triangle that is not a cube face")?;
        for uv in doc.mesh.tri_uvs(t) {
            rects[face].0 = rects[face].0.min(uv * size);
            rects[face].1 = rects[face].1.max(uv * size);
        }
    }
    let mut counts = [0u64; 6];
    let (mut lo, mut hi) = ([u32::MAX; 2], [0u32; 2]);
    for (face, (a, b)) in rects.iter().enumerate() {
        for y in a.y.max(0.0).floor() as u32..(b.y.ceil() as u32).min(set.height) {
            for x in a.x.max(0.0).floor() as u32..(b.x.ceil() as u32).min(set.width) {
                if paint.data[((y * set.width + x) * 4 + 3) as usize] > 0 {
                    counts[face] += 1;
                    if face == front {
                        lo = [lo[0].min(x), lo[1].min(y)];
                        hi = [hi[0].max(x), hi[1].max(y)];
                    }
                }
            }
        }
    }
    let names = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    let box_size = if counts[front] > 0 { [(hi[0] - lo[0] + 1) as f32, (hi[1] - lo[1] + 1) as f32] } else { [0.0; 2] };
    log::info!(
        "brush_shape {label}: changed texels per face {:?}; front {} box {} x {}",
        names.iter().zip(counts).collect::<Vec<_>>(),
        names[front],
        box_size[0],
        box_size[1]
    );
    if let Some(other) = (0..6).find(|&f| f != front && counts[f] > 0) {
        return Err(format!("{label}: the face {} has {} changed texels", names[other], counts[other]));
    }
    Ok((counts[front], box_size))
}

fn bs_count_near(count: u64, expected: f64, label: &str) -> Result<(), String> {
    if (count as f64 - expected).abs() > 0.05 * expected {
        return Err(format!("{label}: {count} changed texels, expected {expected} within 5 %"));
    }
    Ok(())
}

fn bs_size_near(size: [f32; 2], expected: [f32; 2], within: f32, label: &str) -> Result<(), String> {
    if (size[0] - expected[0]).abs() > within || (size[1] - expected[1]).abs() > within {
        return Err(format!("{label}: the square is {} x {} texels, expected {} x {} within {within}", size[0], size[1], expected[0], expected[1]));
    }
    Ok(())
}

/// A click of the GLB, FBX or closed-box shape: only the front face, the count within 5 % of 13,333 and the box within
/// 4 texels of the first OBJ click; one history entry.
fn bs_same_square(state: &AppState, scratch: &Scratch, label: &str) -> Result<(), String> {
    let (count, size) = bs_front_square(state, label)?;
    bs_count_near(count, 13_333.0, label)?;
    let first = [remembered(scratch, "bs_w")? as f32 / 100.0, remembered(scratch, "bs_h")? as f32 / 100.0];
    bs_size_near(size, first, 4.0, label)?;
    undo_since(state, scratch, 1)
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_brush_tab.rs (pasted by the gate, sitting M2)
// Scenario `brush_tab` (FEATURE_BRUSH 8.3 S3; ROUND4_PLAN R4-1, R4-6, 2.2 item 11). Author: B4 (round 4).
// ---------------------------------------------------------------------------------------------------------------------

/// The Brush tab (FEATURE_BRUSH 8.3 S3): at 1280 x 800, the Sphere sample, the Brush tool with the Brush tab, Sphere
/// with the Cube turned by 30 degrees, the pointer on the model (no press): the Color section is inside the side panel,
/// the five brush slider rows end above the bottom of the tool area, every row of the tab is inside the lower area,
/// the side panel is 300 pt wide and the shape cursor (footprint and wires) is drawn. The same layout in the tallest
/// state of the tab (a loaded shape file whose openings were closed, with the "Openings" row of plan section 7 item 5
/// and the "!" marker). The tab rule: `SetTool(Text)` shows the Layers list and keeps `side_tab`; `SetTool(Brush)`
/// shows the Brush tab again. The end screenshot shows the tab, its preview, the footprint outline and the wires.
fn brush_tab() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::WaitIdle,
        Step::Setup("the Brush tab; Sphere with the Cube, Turn 30", |s, _| {
            s.side_tab = crate::state::SideTab::Brush;
            let b = &mut s.tools.brush;
            b.kind = crate::state::BrushType::Sphere;
            b.shape = crate::state::ShapeChoice::Cube;
            b.turn_deg = 30.0;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Hover { view: ViewId::Main3D, at: [0.5, 0.5] },
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("A12 on the window with the Cube: Color, slider rows, tab rows, 300 pt", |s, _| brush_tab_layout(s, "Cube")),
        Step::Assert("the shape cursor of the Cube: a footprint and wires", brush_tab_cursor),
        Step::Setup("the tallest state: a loaded shape file whose openings were closed with Flat caps", |s, _| {
            let path = std::path::PathBuf::from("brush_tab_shape.obj");
            s.prefs.brush_shape_path = Some(path.clone());
            s.brush_shape.current = Some(crate::state::ReadyShape {
                path,
                name: "brush_tab_shape.obj".to_owned(),
                field: std::sync::Arc::new(wte_core::TipField::cube()),
                close: wte_core::CloseMode::FlatCaps,
                closed_parts: true,
            });
            s.brush_shape.last_error =
                Some((std::path::PathBuf::from("other.obj"), "the scenario's sample message of a failed load".to_owned()));
            let b = &mut s.tools.brush;
            b.shape = crate::state::ShapeChoice::File;
            b.tilt_deg = -20.0;
            Ok(())
        }),
        Step::WaitFrames(4),
        Step::Assert("A12 in the tallest state (Reset, Shape with !, preview, Offset, Align and Use, Openings, notes)", |s, _| {
            brush_tab_layout(s, "tallest")
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("SetTool(Text): the Layers list is shown, side_tab is still Brush", |s, _| {
            if s.side_rects.brush_tab.is_some() {
                return Err("the Brush tab is still shown".to_owned());
            }
            if s.side_tab != crate::state::SideTab::Brush {
                return Err(format!("side_tab is {:?}", s.side_tab));
            }
            let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?.expand(0.5);
            let footer = panel_rect(s, PanelHit::LayersFooter)?;
            if !side.contains_rect(footer) {
                return Err(format!("the Layers footer {footer:?} is not on screen in {side:?}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("SetTool(Brush): the Brush tab is shown again", |s, _| {
            tool_is(s, ToolKind::Brush)?;
            if s.side_rects.brush_tab.is_none() {
                return Err("the Brush tab is not shown".to_owned());
            }
            brush_tab_layout(s, "back to the Brush")
        }),
        Step::Hover { view: ViewId::Main3D, at: [0.52, 0.48] },
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the shape cursor of the file shape: a footprint and wires", brush_tab_cursor),
        // P11 (FEATURE_BRUSH 8.4): the frame profile logs "side panel" and "overlays" once per 2 s. 300 idle frames
        // (about 5 s) with the Ball (no shape cursor, the tab without the preview), then 300 with the file shape again,
        // give at least one whole profile window of each; the log lines after each step are the P11 numbers. The end
        // screenshot shows the file shape.
        Step::Setup("P11: idle with the Ball, no shape (Brush tab open, the pointer on the model)", |s, _| {
            s.tools.brush.shape = crate::state::ShapeChoice::Ball;
            Ok(())
        }),
        Step::WaitFrames(300),
        Step::Setup("P11: idle with the file shape (Brush tab open, the pointer on the model)", |s, _| {
            s.tools.brush.shape = crate::state::ShapeChoice::File;
            Ok(())
        }),
        Step::WaitFrames(300),
        Step::Assert("the shape cursor of the file shape again (for the screenshot)", brush_tab_cursor),
    ]
}

/// Test A12 on the real window: the Brush tab is shown; the Color section lies inside the side panel; the Tool options
/// (header and the five brush slider rows) end above the bottom of the tool area; the tab's content (every row) lies
/// inside the visible tab area and that inside the lower area; the side panel is 300 pt wide.
fn brush_tab_layout(s: &AppState, what: &str) -> Result<(), String> {
    let r = &s.side_rects;
    let tab = r.brush_tab.ok_or_else(|| format!("{what}: the Brush tab is not shown"))?;
    let content = r.brush_tab_content.ok_or_else(|| format!("{what}: the tab's content was not recorded"))?;
    let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?;
    if !side.expand(0.5).contains_rect(r.color) {
        return Err(format!("{what}: the Color section {:?} is not inside the side panel {side:?}", r.color));
    }
    // Gate 1: the pixel budget of test A12 (300 pt wide, the slider rows inside the tool area, the tab without
    // scrolling) is defined for a window of 1280 x 800 or larger (FEATURE_BRUSH A12, ROUND4_PLAN 3.3). In a smaller
    // window (the gate also runs `brush_tab` at 900 x 600 and 640 x 480) the side panel may be narrower, the tool
    // options and the tab scroll; what must hold there is checked above and below.
    if side.right() < 1279.5 || side.height() < 749.5 {
        if side.width() > 300.5 {
            return Err(format!("{what}: the side panel is {} pt wide", side.width()));
        }
        if !r.bottom_area.expand(0.5).contains_rect(tab) {
            return Err(format!("{what}: the tab {tab:?} is not inside the lower area {:?}", r.bottom_area));
        }
        log::info!(
            "{what}: small window, A12 budget not applied: side panel {side:?}; tool area {:?} (options {:?}); Color {:?}; lower area {:?}; tab {tab:?}; content {content:?}",
            r.tool_area,
            r.tool_options,
            r.color,
            r.bottom_area
        );
        return Ok(());
    }
    if (side.width() - 300.0).abs() > 0.5 {
        return Err(format!("{what}: the side panel is {} pt wide", side.width()));
    }
    if r.tool_options.bottom() > r.tool_area.bottom() + 0.5 {
        return Err(format!("{what}: the brush sliders {:?} end below the tool area {:?}", r.tool_options, r.tool_area));
    }
    if !r.bottom_area.expand(0.5).contains_rect(tab) || !tab.expand(0.5).contains_rect(content) {
        return Err(format!("{what}: the tab content {content:?} is not inside its area {tab:?} in the lower area {:?}", r.bottom_area));
    }
    log::info!(
        "{what}: side panel {side:?}; tool area {:?} (options {:?}); Color {:?}; lower area {:?}; tab {tab:?}; content {content:?} ({:.0} of {:.0} pt)",
        r.tool_area,
        r.tool_options,
        r.color,
        r.bottom_area,
        content.height(),
        tab.height()
    );
    Ok(())
}

/// The brush tool drew the shape cursor in the last frame: a non-empty footprint and wires (`ToolProbe`, test A18).
fn brush_tab_cursor(s: &AppState, _: &Scratch) -> Result<(), String> {
    let p = &s.tool_probe;
    if p.cursor_footprint == 0 || p.cursor_wires == 0 {
        return Err(format!("the shape cursor is missing: {} footprint and {} wire segments", p.cursor_footprint, p.cursor_wires));
    }
    log::info!("shape cursor: {} footprint and {} wire segments; status bar: {:?}", p.cursor_footprint, p.cursor_wires, s.status_hint);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_t7_common.rs (pasted by the gate, sitting M2)
// T7 (round 4): helpers shared by text_gaps, text_styles, text_warp, text_path and the extended roundtrip / export_run.
// ---------------------------------------------------------------------------------------------------------------------

/// The warp styles `text_warp` applies in turn at 50 % (Arc, Flag, Fisheye, and Arc again for the screenshot).
const T7_WARPS: [wte_core::WarpStyle; 4] =
    [wte_core::WarpStyle::Arc, wte_core::WarpStyle::Flag, wte_core::WarpStyle::Fisheye, wte_core::WarpStyle::Arc];

/// The running timed text step and its start.
static T7_TIMER: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// Begins a timed text step (the `Final` steps of `text_warp` / `text_path`, FEATURE_TEXT 8.4).
fn t7_timer_start(name: String) {
    *T7_TIMER.lock().unwrap_or_else(|e| e.into_inner()) = Some((name, Instant::now()));
}

/// Ends the running timed text step and logs its milliseconds ("text step \"Arc\" final in 123 ms").
fn t7_timer_stop() {
    if let Some((name, start)) = T7_TIMER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        log::info!("text step \"{name}\" final in {} ms", start.elapsed().as_millis());
    }
}

/// The selected stamp, which must be a text, and its spec.
fn t7_selected_text(state: &AppState) -> Result<(StampId, wte_core::TextSpec), String> {
    let session = session_of(state)?;
    let id = session.selected_stamp.ok_or("no stamp is selected")?;
    match &session.doc.stamp(id).ok_or("the selected stamp does not exist")?.source {
        StampSource::Text(spec) => Ok((id, spec.clone())),
        StampSource::Image { .. } => Err("the selected stamp is an image stamp".to_owned()),
    }
}

/// The baked layer of text stamp `id` in its home set: ink bounds, inked texels, pixel hash and ink centre (texels).
struct T7Ink {
    bounds: wte_core::IRect,
    inked: u64,
    hash: u64,
    centre: Vec2,
}

fn t7_ink(state: &AppState, id: StampId) -> Result<T7Ink, String> {
    let doc = doc_of(state)?;
    let home = doc.stamp(id).ok_or("the text stamp is gone")?.home_set;
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home).ok_or("the text stamp has no layer")?;
    let LayerKind::Text { cache, .. } = &doc.layer(layer).ok_or("the text layer is gone")?.kind else {
        return Err("the text stamp's layer is not a text layer".to_owned());
    };
    let mut hash = rect_key(&cache.bounds());
    let (mut inked, mut mass, mut sum) = (0u64, 0f64, [0f64; 2]);
    for part in cache.parts() {
        hash = wte_core::fnv1a64(hash, &rect_key(&part.rect).to_le_bytes());
        hash = wte_core::fnv1a64(hash, &part.data);
        let w = (part.rect.x1 - part.rect.x0).max(0) as usize;
        for (i, px) in part.data.as_chunks::<4>().0.iter().enumerate() {
            if px[3] > 0 {
                inked += 1;
                let a = px[3] as f64;
                mass += a;
                sum[0] += a * (part.rect.x0 as f64 + (i % w.max(1)) as f64 + 0.5);
                sum[1] += a * (part.rect.y0 as f64 + (i / w.max(1)) as f64 + 0.5);
            }
        }
    }
    if inked == 0 {
        return Err("the text layer has no ink".to_owned());
    }
    let centre = Vec2::new((sum[0] / mass) as f32, (sum[1] / mass) as f32);
    Ok(T7Ink { bounds: cache.bounds(), inked, hash, centre })
}

/// The anchor of text stamp `id` (world units for a projected text, texels for a UV text), from the cached frame
/// (a `Setup` calls `t7_cache_frame` first).
fn t7_anchor(state: &AppState, id: StampId) -> Result<Vec3, String> {
    let doc = doc_of(state)?;
    let frame = doc.cached_text_frame(id).ok_or("the frame of the text is not cached")?;
    let placement = doc.stamp(id).ok_or("the text stamp is gone")?.placement;
    let map = wte_core::text::EmMap::new(&placement, &frame);
    Ok(match placement {
        Placement::Projected(_) => map.to_world([0.0, 0.0]),
        Placement::Uv(_) => map.to_texel([0.0, 0.0]).extend(0.0),
    })
}

/// Fills the frame cache of the selected text (`stamp_text_frame`, geometry only).
fn t7_cache_frame(state: &mut AppState, _sc: &mut Scratch) -> Result<(), String> {
    let (id, _) = t7_selected_text(state)?;
    state.doc_mut().ok_or("no document")?.stamp_text_frame(id).map(|_| ()).map_err(|e| e.to_string())
}

/// Sends one `UpdateStampSource(Final)` for the selected text with `edit` applied to its spec.
fn t7_edit(state: &AppState, edit: fn(&mut wte_core::TextSpec)) -> Result<Vec<Step>, String> {
    let (id, mut spec) = t7_selected_text(state)?;
    edit(&mut spec);
    Ok(vec![Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final))])
}

fn t7_store_vec3(sc: &mut Scratch, keys: [&'static str; 3], v: Vec3) {
    for (key, value) in keys.into_iter().zip(v.to_array()) {
        sc.values.insert(key, value.to_bits() as u64);
    }
}

fn t7_vec3(sc: &Scratch, keys: [&'static str; 3]) -> Result<Vec3, String> {
    let mut out = [0f32; 3];
    for (slot, key) in out.iter_mut().zip(keys) {
        *slot = f32::from_bits(remembered(sc, key)? as u32);
    }
    Ok(Vec3::from_array(out))
}

fn t7_store_i32(sc: &mut Scratch, key: &'static str, v: i32) {
    sc.values.insert(key, v as i64 as u64);
}

fn t7_i32(sc: &Scratch, key: &str) -> Result<i32, String> {
    Ok(remembered(sc, key)? as i64 as i32)
}

/// The draft "WTE 2026" in the bundled font at 6 % and the Text tool, placed on the model (the sphere: just above its
/// centre; a model from `PATH`: the first well-facing spot of the stamp scenarios).
fn t7_place_3d(has_path: bool) -> Vec<Step> {
    let place: fn(&AppState, &Scratch) -> Result<Vec<Step>, String> = if has_path {
        |s, _| {
            let (xf, spot, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, spot.pos))])
        }
    } else {
        |s, _| model_click(s, ViewId::Main3D, [0.0, 0.05])
    };
    vec![
        Step::Setup("the draft \"WTE 2026\" at 6 %", |s, _| {
            s.tools.text = crate::state::TextSettings {
                content: "WTE 2026".to_owned(),
                color: [150, 20, 30, 255],
                em_frac: 0.06,
                ..Default::default()
            };
            Ok(())
        }),
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Dyn("place the text on the model", place),
        Step::WaitIdle,
        Step::Assert("the text was placed and selected (projected)", |s, _| match placed_stamp(s, true)? {
            (_, Placement::Projected(_)) => Ok(()),
            (_, other) => Err(format!("the text is placed as {other:?}")),
        }),
    ]
}

/// The 2D view, the Text tool and a UV text placed at UV (0.3, 0.35) of the active set (no rotation: x is the
/// reading direction).
fn t7_place_2d() -> Vec<Step> {
    vec![
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Dyn("place a UV text in the 2D view", |s, _| {
            let xf = view_xf(s, ViewId::Main2D)?;
            Ok(vec![click(ViewId::Main2D, norm(&xf, uv_screen(&xf, [0.3, 0.35])))])
        }),
        Step::WaitIdle,
        Step::Assert("the UV text was placed and selected", |s, _| match placed_stamp(s, true)? {
            (_, Placement::Uv(u)) if u.rotation == 0.0 => Ok(()),
            (_, other) => Err(format!("the text is placed as {other:?}")),
        }),
    ]
}

/// The undo label of the last edit is "Edit Stamp".
fn t7_edit_label(state: &AppState) -> Result<(), String> {
    match doc_of(state)?.undo_label() {
        Some("Edit Stamp") => Ok(()),
        other => Err(format!("the undo label is {other:?}, expected \"Edit Stamp\"")),
    }
}

/// Undo gives the layer remembered under `before`, redo the one under `after`; the layer stays a text layer.
fn t7_undo_redo(before: fn(&AppState, &Scratch) -> Result<(), String>, after: fn(&AppState, &Scratch) -> Result<(), String>) -> Vec<Step> {
    vec![
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("after undo the text layer is the one before the edit", before),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("after redo the text layer is the one after the edit (still a text layer)", after),
    ]
}

fn t7_hash_is(state: &AppState, sc: &Scratch, key: &str) -> Result<(), String> {
    let (id, _) = t7_selected_text(state)?;
    let hash = t7_ink(state, id)?.hash;
    if hash != remembered(sc, key)? {
        return Err(format!("the text layer hash differs from \"{key}\""));
    }
    Ok(())
}


/// The text of the default content made styled for `roundtrip` and `export_run` (FEATURE_TEXT 8.3): two gap values
/// and a warp.
fn t7_style_default_text() -> Vec<Step> {
    vec![
        Step::Dyn("give the text two gap values and an Arc warp", |s, _| {
            let (id, mut spec) = text_stamp(s)?;
            spec.fx.gaps = vec![wte_core::Gap { after: 0, em: 0.2 }, wte_core::Gap { after: 1, em: -0.1 }];
            spec.fx.warp = Some(wte_core::Warp { style: wte_core::WarpStyle::Arc, bend: 0.3, horizontal: 0.0, vertical: 0.0, vertical_axis: false });
            Ok(vec![Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("the text has the two gap values and the warp", |s, _| {
            let (_, spec) = text_stamp(s)?;
            if spec.fx.gaps.len() != 2 || spec.fx.warp.is_none() {
                return Err(format!("fx {:?}", spec.fx));
            }
            Ok(())
        }),
    ]
}

/// A hash of the text stamp's spec (its debug text: every field) and of its layer's pixels.
fn t7_text_keys(state: &AppState) -> Result<(u64, u64), String> {
    let (id, spec) = text_stamp(state)?;
    let spec_key = wte_core::fnv1a64(0xcbf2_9ce4_8422_2325, format!("{spec:?}").as_bytes());
    Ok((spec_key, t7_ink(state, id)?.hash))
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_text_gaps.rs (pasted by the gate, sitting M2)
// T7 (round 4): scenario text_gaps (FEATURE_TEXT 8.3). Needs scenario_t7_common.rs.
// ---------------------------------------------------------------------------------------------------------------------

/// One gap edit of the selected text and its checks (FEATURE_TEXT 8.3 `text_gaps`), for a projected or a UV text.
fn t7_gap_steps() -> Vec<Step> {
    let mut steps = vec![
        Step::Setup("remember the text layer, its em in texels and the history", |s, sc| {
            let (id, _) = t7_selected_text(s)?;
            let frame = s.doc_mut().ok_or("no document")?.stamp_text_frame(id).map_err(|e| e.to_string())?;
            let ink = t7_ink(s, id)?;
            let em_texels = match doc_of(s)?.stamp(id).ok_or("gone")?.placement {
                Placement::Uv(u) => u.size_px.y / frame.size[1] as f32 * frame.em_px,
                Placement::Projected(_) => 0.0,
            };
            t7_store_i32(sc, "g_x0", ink.bounds.x0);
            t7_store_i32(sc, "g_x1", ink.bounds.x1);
            sc.values.insert("g_area", ink.bounds.area());
            sc.values.insert("g_hash0", ink.hash);
            sc.values.insert("g_em", em_texels.to_bits() as u64);
            log::info!("text_gaps: before the gap {:?}, {} inked texels, one em = {em_texels} texels", ink.bounds, ink.inked);
            remember_undo(s, sc)
        }),
        Step::Dyn("set the gap after \"E\" to 0.5 em", |s, _| {
            t7_edit(s, |spec| spec.fx.gaps = vec![wte_core::Gap { after: 2, em: 0.5 }])
        }),
        Step::WaitIdle,
        Step::Assert("the gap widened the text; one history entry \"Edit Stamp\"", |s, sc| {
            let (id, spec) = t7_selected_text(s)?;
            if spec.fx.gaps != [wte_core::Gap { after: 2, em: 0.5 }] {
                return Err(format!("the gaps are {:?}", spec.fx.gaps));
            }
            let ink = t7_ink(s, id)?;
            let (x0, x1) = (t7_i32(sc, "g_x0")?, t7_i32(sc, "g_x1")?);
            let em = f32::from_bits(remembered(sc, "g_em")? as u32);
            log::info!("text_gaps: after the gap {:?} (was x {x0}..{x1}), {} inked texels", ink.bounds, ink.inked);
            if em > 0.0 {
                // A UV text, left-aligned: the start stays, the letters after the gap move by half an em.
                let grown = (ink.bounds.x1 - ink.bounds.x0 - (x1 - x0)) as f32;
                if (grown - 0.5 * em).abs() > 2.0 {
                    return Err(format!("the bounds grew by {grown} texels, expected {} within 2", 0.5 * em));
                }
                if (ink.bounds.x0 - x0).abs() >= 1 {
                    return Err(format!("the left edge moved from {x0} to {}", ink.bounds.x0));
                }
                log::info!("text_gaps: UV bounds grew by {grown} texels (0.5 em = {} texels), left edge moved by {}", 0.5 * em, ink.bounds.x0 - x0);
            } else if ink.bounds.area() <= remembered(sc, "g_area")? {
                return Err(format!("the baked bounds {:?} are not wider than before", ink.bounds));
            }
            t7_edit_label(s)?;
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember the layer with the gap", |s, sc| {
            let (id, _) = t7_selected_text(s)?;
            sc.values.insert("g_hash1", t7_ink(s, id)?.hash);
            Ok(())
        }),
    ];
    steps.extend(t7_undo_redo(|s, sc| t7_hash_is(s, sc, "g_hash0"), |s, sc| t7_hash_is(s, sc, "g_hash1")));
    steps
}

/// `text_gaps` (FEATURE_TEXT 8.3): a gap of 0.5 em after "E" of "WTE 2026", in 3D and on a UV text.
fn text_gaps() -> Vec<Step> {
    let mut steps = vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle];
    steps.extend(t7_place_3d(false));
    steps.extend(t7_gap_steps());
    // Gate 3 (ROUND4_PLAN 3.3 item 7): the text with one widened gap on the model, `<out>_gap3d.png`.
    steps.extend([Step::WaitFrames(3), Step::Shot("gap3d")]);
    steps.extend(t7_place_2d());
    steps.extend(t7_gap_steps());
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_text_styles.rs (pasted by the gate, sitting M2)
// T7 (round 4): scenario text_styles (FEATURE_TEXT 8.3). Needs scenario_t7_common.rs.
// ---------------------------------------------------------------------------------------------------------------------

/// `text_styles` (FEATURE_TEXT 8.3): thickening 0.02 em and a slant of 12 degrees in one `Final`.
fn text_styles() -> Vec<Step> {
    let mut steps = vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle];
    steps.extend(t7_place_3d(false));
    steps.extend([
        Step::Setup("remember the text layer and the history", |s, sc| {
            let (id, _) = t7_selected_text(s)?;
            let ink = t7_ink(s, id)?;
            sc.values.insert("s_inked", ink.inked);
            sc.values.insert("s_hash0", ink.hash);
            remember_undo(s, sc)
        }),
        Step::Dyn("thicken by 0.02 em and slant by 12 degrees", |s, _| {
            t7_edit(s, |spec| {
                spec.fx.bold_em = 0.02;
                spec.fx.slant_deg = 12.0;
            })
        }),
        Step::WaitIdle,
        Step::Assert("the layer has more ink; one history entry", |s, sc| {
            let (id, spec) = t7_selected_text(s)?;
            if (spec.fx.bold_em, spec.fx.slant_deg) != (0.02, 12.0) {
                return Err(format!("thickening {} slant {}", spec.fx.bold_em, spec.fx.slant_deg));
            }
            let inked = t7_ink(s, id)?.inked;
            let before = remembered(sc, "s_inked")?;
            log::info!("text_styles: {before} -> {inked} inked texels");
            if inked <= before {
                return Err(format!("{inked} inked texels, {before} before"));
            }
            t7_edit_label(s)?;
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember the styled layer", |s, sc| {
            let (id, _) = t7_selected_text(s)?;
            sc.values.insert("s_hash1", t7_ink(s, id)?.hash);
            Ok(())
        }),
    ]);
    steps.extend(t7_undo_redo(|s, sc| t7_hash_is(s, sc, "s_hash0"), |s, sc| t7_hash_is(s, sc, "s_hash1")));
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_text_warp.rs (pasted by the gate, sitting M2)
// T7 (round 4): scenario text_warp (FEATURE_TEXT 8.3, 8.4). Needs scenario_t7_common.rs.
// ---------------------------------------------------------------------------------------------------------------------

/// Remembers the baked bounds and the anchor of the selected text as the "previous step" of `text_warp`.
fn t7_remember_step(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    t7_cache_frame(state, sc)?;
    let (id, _) = t7_selected_text(state)?;
    let ink = t7_ink(state, id)?;
    sc.values.insert("w_rect", rect_key(&ink.bounds));
    sc.values.insert("w_hash", ink.hash);
    t7_store_vec3(sc, ["w_ax", "w_ay", "w_az"], t7_anchor(state, id)?);
    Ok(())
}

/// The anchor of the selected text stayed (0.1 % of the model diagonal in 3D, half a texel in 2D).
fn t7_anchor_stayed(state: &AppState, sc: &Scratch) -> Result<f32, String> {
    let (id, _) = t7_selected_text(state)?;
    let moved = (t7_anchor(state, id)? - t7_vec3(sc, ["w_ax", "w_ay", "w_az"])?).length();
    let limit = match doc_of(state)?.stamp(id).ok_or("gone")?.placement {
        Placement::Projected(_) => doc_of(state)?.mesh.bounds.diagonal() * 1e-3,
        Placement::Uv(_) => 0.5,
    };
    if moved >= limit {
        return Err(format!("the anchor moved by {moved} (limit {limit})"));
    }
    Ok(moved)
}

/// The four warp steps of `text_warp` for the selected text.
fn t7_warp_steps() -> Vec<Step> {
    let mut steps = vec![Step::Setup("remember the straight text", |s, sc| {
        sc.values.insert("w_i", 0);
        t7_remember_step(s, sc)
    })];
    for _ in T7_WARPS {
        steps.extend([
            Step::Setup("time the next warp style", |_, sc| {
                let style = T7_WARPS[remembered(sc, "w_i")? as usize];
                t7_timer_start(format!("warp {}", style.label()));
                Ok(())
            }),
            Step::Dyn("apply the next warp style at 50 %", |s, sc| {
                let style = T7_WARPS[remembered(sc, "w_i")? as usize];
                let (id, mut spec) = t7_selected_text(s)?;
                spec.fx.warp = Some(wte_core::Warp { style, bend: 0.5, horizontal: 0.0, vertical: 0.0, vertical_axis: false });
                Ok(vec![Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final))])
            }),
            Step::WaitIdle,
            Step::Setup("log the time of the step; cache the frame", |s, sc| {
                t7_timer_stop();
                t7_cache_frame(s, sc)
            }),
            Step::Assert("the baked bounds changed and the anchor stayed", |s, sc| {
                let (id, spec) = t7_selected_text(s)?;
                let style = T7_WARPS[remembered(sc, "w_i")? as usize];
                if spec.fx.warp.map(|w| w.style) != Some(style) {
                    return Err(format!("the warp is {:?}, expected {style:?}", spec.fx.warp));
                }
                let ink = t7_ink(s, id)?;
                let bounds = ink.bounds;
                // A wrapped text (WTE_STAMP_MODE=wrap, gate 2) can touch UV islands all over the set, so its bounds
                // can stay the whole texture: there the baked pixels must change instead.
                let wrapped = matches!(doc_of(s)?.stamp(id).map(|st| st.placement), Some(Placement::Projected(p)) if p.mode == SurfaceMode::Wrap);
                if wrapped {
                    if ink.hash == remembered(sc, "w_hash")? {
                        return Err(format!("the wrapped text's pixels did not change with {}", style.label()));
                    }
                } else if rect_key(&bounds) == remembered(sc, "w_rect")? {
                    return Err(format!("the baked bounds {bounds:?} did not change with {}", style.label()));
                }
                let moved = t7_anchor_stayed(s, sc)?;
                log::info!("text_warp: {} bounds {bounds:?}, anchor moved by {moved}", style.label());
                Ok(())
            }),
            Step::Setup("remember this step", |s, sc| {
                *sc.values.entry("w_i").or_insert(0) += 1;
                t7_remember_step(s, sc)
            }),
        ]);
    }
    steps
}

/// `text_warp` (FEATURE_TEXT 8.3): Arc, Flag, Fisheye at 50 % (and Arc again), on a projected and on a UV text. On a
/// model from `PATH` it times every `Final` (FEATURE_TEXT 8.4: ABeautifulGame.glb, 1,500 ms).
fn text_warp(has_path: bool) -> Vec<Step> {
    let mut steps = if has_path {
        open_and_accept(true)
    } else {
        vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle]
    };
    steps.extend(t7_place_3d(has_path));
    steps.extend([Step::WaitFrames(3), Step::Setup("remember the 3D view rect", remember_view_rect)]);
    steps.extend(t7_warp_steps());
    steps.extend([Step::WaitFrames(3), Step::Assert("the 3D view rectangle did not move", same_view_rect)]);
    // Gate 3 (ROUND4_PLAN 3.3 item 7): the warped text (Arc 50 %) on the model, `<out>_warp3d.png`.
    steps.push(Step::Shot("warp3d"));
    steps.extend(t7_place_2d());
    steps.extend(t7_warp_steps());
    steps.extend([Step::Do(AppAction::SetViewMode(ViewMode::Split)), Step::WaitIdle, Step::WaitFrames(3)]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_text_path.rs (pasted by the gate, sitting M2)
// T7 (round 4): scenario text_path (FEATURE_TEXT 8.3, 8.4). Needs scenario_t7_common.rs.
// ---------------------------------------------------------------------------------------------------------------------

/// Sets the line of the selected text to `make(straight frame, alignment)` with one `Final` (as the Path page does:
/// the shape is made from the frame of the text without a line).
fn t7_push_path(state: &mut AppState, make: fn(&wte_core::TextFrame, wte_core::TextAlign) -> wte_core::TextPath) -> Result<(), String> {
    let (id, spec) = t7_selected_text(state)?;
    let mut straight = spec.clone();
    straight.fx.path = None;
    let frame = state.doc_mut().ok_or("no document")?.stamp_source_frame(&StampSource::Text(straight)).map_err(|e| e.to_string())?;
    let mut next = spec;
    next.fx.path = Some(make(&frame, next.align));
    state.push(AppAction::UpdateStampSource(id, StampSource::Text(next), Commit::Final));
    Ok(())
}

/// The curve points of the selected text on screen (its frame must be cached).
fn t7_curve_screen(state: &AppState) -> Result<(ViewXform, crate::tools::text_path::PathPoints), String> {
    let (id, spec) = t7_selected_text(state)?;
    let doc = doc_of(state)?;
    let placement = doc.stamp(id).ok_or("gone")?.placement;
    let view = if matches!(placement, Placement::Projected(_)) { ViewId::Main3D } else { ViewId::Main2D };
    let xf = view_xf(state, view)?;
    let frame = doc.cached_text_frame(id).ok_or("the frame of the text is not cached")?;
    let path = spec.fx.path.as_ref().ok_or("the text has no line")?;
    let points = crate::tools::text_path::path_points(&placement, &frame, path, &xf).ok_or("the curve points are not in the view")?;
    Ok((xf, points))
}

/// After a kind of line: the baked bounds changed; remembers them.
fn t7_kind_changed(state: &AppState, sc: &Scratch, kind: &str) -> Result<(), String> {
    let (id, spec) = t7_selected_text(state)?;
    let shape = spec.fx.path.as_ref().map(|p| &p.shape).ok_or("the text has no line")?;
    let found = match shape {
        wte_core::PathShape::Arc { .. } => "Arc",
        wte_core::PathShape::Circle { .. } => "Circle",
        wte_core::PathShape::Curve { .. } => "Curve",
    };
    if found != kind {
        return Err(format!("the line is {found}, expected {kind}"));
    }
    let bounds = t7_ink(state, id)?.bounds;
    if rect_key(&bounds) == remembered(sc, "p_rect")? {
        return Err(format!("the baked bounds {bounds:?} did not change with {kind}"));
    }
    log::info!("text_path: {kind} bounds {bounds:?}");
    Ok(())
}

fn t7_remember_rect(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let (id, _) = t7_selected_text(state)?;
    sc.values.insert("p_rect", rect_key(&t7_ink(state, id)?.bounds));
    Ok(())
}

/// The steps of `text_path` for the selected text: Arc 25 %, Circle, Curve, and a drag of the middle curve point.
fn t7_path_steps() -> Vec<Step> {
    t7_path_steps_shot(None)
}

/// `t7_path_steps` with a named capture of the text on its Circle (gate 3, ROUND4_PLAN 3.3 item 7).
fn t7_path_steps_shot(circle_shot: Option<&'static str>) -> Vec<Step> {
    let mut steps = vec![
        Step::Setup("remember the straight text", |s, sc| {
            let (id, _) = t7_selected_text(s)?;
            let ink = t7_ink(s, id)?;
            sc.values.insert("p_rect", rect_key(&ink.bounds));
            sc.values.insert("p_cx", ink.centre.x.to_bits() as u64);
            Ok(())
        }),
        Step::Setup("Path: Arc 25 %", |s, _| {
            t7_timer_start("path Arc".to_owned());
            t7_push_path(s, |_, _| wte_core::TextPath { shape: wte_core::PathShape::Arc { bend: 0.25 }, start: 0.0, flip: false })
        }),
        Step::WaitIdle,
        Step::Setup("log the time of the step", |_, _| {
            t7_timer_stop();
            Ok(())
        }),
        Step::Assert("Arc changed the baked bounds", |s, sc| t7_kind_changed(s, sc, "Arc")),
        Step::Setup("remember the Arc", t7_remember_rect),
        Step::Setup("Path: Circle", |s, _| {
            t7_timer_start("path Circle".to_owned());
            t7_push_path(s, wte_core::text::path::new_circle)
        }),
        Step::WaitIdle,
        Step::Setup("log the time of the step", |_, _| {
            t7_timer_stop();
            Ok(())
        }),
        Step::Assert("Circle changed the baked bounds; a UV text's ink is centred where the straight text was", |s, sc| {
            t7_kind_changed(s, sc, "Circle")?;
            let (id, _) = t7_selected_text(s)?;
            if matches!(doc_of(s)?.stamp(id).ok_or("gone")?.placement, Placement::Uv(_)) {
                let cx = t7_ink(s, id)?.centre.x;
                let before = f32::from_bits(remembered(sc, "p_cx")? as u32);
                log::info!("text_path: ink centre x {before} straight, {cx} on the Circle");
                if (cx - before).abs() > 2.0 {
                    return Err(format!("the ink centre moved across the reading direction by {} texels", cx - before));
                }
            }
            Ok(())
        }),
        Step::Setup("remember the Circle", t7_remember_rect),
    ];
    if let Some(name) = circle_shot {
        steps.extend([Step::WaitFrames(3), Step::Shot(name)]);
    }
    steps.extend([
        Step::Setup("Path: Curve", |s, _| {
            t7_timer_start("path Curve".to_owned());
            t7_push_path(s, |frame, _| wte_core::text::path::new_curve(frame))
        }),
        Step::WaitIdle,
        Step::Setup("log the time of the step", |_, _| {
            t7_timer_stop();
            Ok(())
        }),
        Step::Assert("Curve changed the baked bounds", |s, sc| t7_kind_changed(s, sc, "Curve")),
        Step::Setup("cache the frame; remember the curve points on screen and the history", |s, sc| {
            t7_cache_frame(s, sc)?;
            let (_, points) = t7_curve_screen(s)?;
            if points.points.len() != 3 {
                return Err(format!("the new Curve has {} points", points.points.len()));
            }
            let (_, spec) = t7_selected_text(s)?;
            let em = crate::tools::text_path::curve_points(spec.fx.path.as_ref().ok_or("no line")?).ok_or("not a Curve")?.to_vec();
            sc.values.insert("p_first", pack(Vec2::new(points.points[0].x, points.points[0].y)));
            sc.values.insert("p_last", pack(Vec2::new(points.points[2].x, points.points[2].y)));
            sc.values.insert("p_mid_y", em[1][1].to_bits() as u64);
            remember_undo(s, sc)
        }),
        Step::Dyn("drag the middle curve point 40 pt up", |s, _| {
            let (xf, points) = t7_curve_screen(s)?;
            let from = points.points[1];
            Ok(vec![drag(&xf, &[from, from + egui::vec2(0.0, -40.0)])])
        }),
        Step::WaitIdle,
        Step::Setup("cache the frame after the drag", t7_cache_frame),
        Step::Assert("the middle point moved up, one history entry, the first and last point stayed on screen", |s, sc| {
            let (_, spec) = t7_selected_text(s)?;
            let em = crate::tools::text_path::curve_points(spec.fx.path.as_ref().ok_or("no line")?).ok_or("not a Curve")?.to_vec();
            let before = f32::from_bits(remembered(sc, "p_mid_y")? as u32);
            if em[1][1] >= before {
                return Err(format!("points[1].y is {} (was {before})", em[1][1]));
            }
            undo_since(s, sc, 1)?;
            let (_, points) = t7_curve_screen(s)?;
            let first = points.points[0].distance({ let v = unpack(remembered(sc, "p_first")?); egui::pos2(v.x, v.y) });
            let last = points.points[2].distance({ let v = unpack(remembered(sc, "p_last")?); egui::pos2(v.x, v.y) });
            log::info!("text_path: points[1].y {before} -> {}; first point moved {first} pt, last {last} pt", em[1][1]);
            if first >= 1.5 || last >= 1.5 {
                return Err(format!("the first point moved {first} pt, the last {last} pt"));
            }
            Ok(())
        }),
    ]);
    steps
}

/// `text_path` (FEATURE_TEXT 8.3): Arc 25 %, Circle, Curve and a drag of the middle curve point, on a projected and
/// on a UV text. On a model from `PATH` it times every `Final` (FEATURE_TEXT 8.4: the Circle within 1,500 ms on
/// ABeautifulGame.glb).
fn text_path(has_path: bool) -> Vec<Step> {
    let mut steps = if has_path {
        open_and_accept(true)
    } else {
        vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle]
    };
    steps.extend(t7_place_3d(has_path));
    // Gate 3 (ROUND4_PLAN 3.3 item 7): the text on its Circle on the model, `<out>_circle3d.png`.
    steps.extend(t7_path_steps_shot(Some("circle3d")));
    steps.extend(t7_place_2d());
    steps.extend(t7_path_steps());
    steps.extend([Step::Do(AppAction::SetViewMode(ViewMode::Split)), Step::WaitIdle, Step::WaitFrames(3)]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_text_tab.rs (pasted by the gate, sitting M2)
// Scenario `text_tab` (FEATURE_TEXT 8.3; ROUND4_PLAN 3.3 with the cross-feature step). Author: T6 (round 4).
// ---------------------------------------------------------------------------------------------------------------------

/// The Text tab: the see-through text under the pointer before placing; every page with the colour picker inside the
/// panel and the 3D view in place, before and after placing; the draft does not change the placed text; the three
/// tabs together (ROUND4_PLAN 3.3); the end screenshot shows the Character page with a selected gap.
fn text_tab() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Setup("the draft \"WTE 2026\" at a small size", |s, _| {
            s.tools.text.content = "WTE 2026".to_owned();
            s.tools.text.em_frac = 0.03;
            s.tools.text.color = [150, 20, 30, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember the 3D view rect with the Stamp tool", remember_view_rect),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::Hover { view: ViewId::Main3D, at: [0.5, 0.5] },
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the Text tool holds a see-through picture of the draft", assert_text_hover),
        // Gate 1 (ROUND4_PLAN 3.3 "a screenshot" of the hover frame): `<out>_hover.png`.
        Step::Shot("hover"),
    ];
    steps.extend(text_tab_pages(false));
    steps.extend([
        Step::Dyn("place the text on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.05])),
        Step::WaitIdle,
        Step::Assert("the text was placed and selected", |s, _| placed_stamp(s, true).map(|_| ())),
        Step::Assert("the tool is Transform with the placed text", |s, _| tool_is(s, ToolKind::Transform)),
    ]);
    // Gate 3 (ROUND4_PLAN 3.3 item 7): each page of the Text tab with the placed text, `<out>_page_<name>.png`.
    steps.extend(text_tab_pages(true));
    steps.extend([
        // The draft step: the Text tool with the text still selected; the draft changes, the placed text does not.
        Step::Setup("remember the placed content", |s, sc| {
            let (_, spec) = text_stamp(s)?;
            sc.values.insert("placed_len", spec.content.chars().count() as u64);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitIdle,
        Step::Setup("one change of the draft content", |s, _| {
            s.tools.text.content = "Draft only".to_owned();
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("the draft did not change the placed text", |s, sc| {
            let (_, spec) = text_stamp(s)?;
            if spec.content != "WTE 2026" || spec.content.chars().count() as u64 != remembered(sc, "placed_len")? {
                return Err(format!("the placed text reads {:?}", spec.content));
            }
            if s.tools.text.content != "Draft only" {
                return Err(format!("the draft reads {:?}", s.tools.text.content));
            }
            Ok(())
        }),
        // ROUND4_PLAN 3.3: the three tabs together, with `side_tab = Text`, the text selected, the Transform tool.
        Step::Setup("the Text tab", |s, _| {
            s.side_tab = SideTab::Text;
            s.text_page = TextPage::Character;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the Text tab is shown under the Transform tool", |s, _| tabs_shown(s, false, true)),
        Step::Setup("remember the selected text", |s, sc| {
            let id = session_of(s)?.selected_stamp.ok_or("no selection")?;
            sc.values.insert("tabs_text", id.0);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("3.3 step 1: SetTool(Brush) shows Layers; side_tab is Text; the text is still selected", |s, sc| {
            tabs_shown(s, false, false)?;
            if s.side_tab != SideTab::Text {
                return Err(format!("side_tab is {:?}", s.side_tab));
            }
            if session_of(s)?.selected_stamp.map(|id| id.0) != Some(remembered(sc, "tabs_text")?) {
                return Err("the selection changed".to_owned());
            }
            Ok(())
        }),
        Step::Dyn("3.3 step 2: a click on the Brush tab", |s, _| tab_click(s, SideTab::Brush)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("3.3 step 2: the Brush tab is shown", |s, _| tabs_shown(s, true, false)),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("3.3 step 3: SetTool(Transform) shows Layers (side_tab is Brush)", |s, _| {
            tabs_shown(s, false, false)?;
            if s.side_tab != SideTab::Brush {
                return Err(format!("side_tab is {:?}", s.side_tab));
            }
            Ok(())
        }),
        Step::Dyn("3.3 step 4: a click on the Text tab", |s, _| tab_click(s, SideTab::Text)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("3.3 step 4: the Text tab is shown (Transform with a selected text)", |s, _| tabs_shown(s, false, true)),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("3.3 step 5: SetTool(Stamp) shows Layers; both tabs are grey", |s, _| {
            tabs_shown(s, false, false)?;
            if crate::panels::brush_tab_enabled(s) || crate::panels::text_tab_enabled(s) {
                return Err("a tab is still enabled".to_owned());
            }
            Ok(())
        }),
        // Back to Layers: the footer buttons are inside the window.
        Step::Setup("back to Layers", |s, _| {
            s.side_tab = SideTab::Layers;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("after switching back the Layers footer is inside the side panel", assert_side_panel_layout),
        // The end screenshot: the Character page with a selected gap (between "E" and the space).
        Step::Setup("the Character page with the gap after \"E\" selected", |s, _| {
            let (id, _) = text_stamp(s)?;
            s.side_tab = SideTab::Text;
            s.text_page = TextPage::Character;
            s.session.as_mut().ok_or("no session")?.text_gap = Some(TextGapSel { stamp: id, after: 2 });
            Ok(())
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the gap is selected and named in the status bar", |s, _| {
            if !s.status_hint.starts_with("Gap between E and (space)") {
                return Err(format!("the status bar says {:?}", s.status_hint));
            }
            text_page_layout(s, "Character page with a gap")
        }),
    ]);
    steps
}

/// The three pages of the Text tab in turn: on each the Color section is inside the side panel, the side panel is
/// 300 pt wide and the 3D view rect equals the one with the Stamp tool.
fn text_tab_pages(shots: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    for (page, name, shot) in [
        (TextPage::Character, "Character", "page_character"),
        (TextPage::Warp, "Warp", "page_warp"),
        (TextPage::Path, "Path", "page_path"),
    ] {
        let set: fn(&mut AppState, &mut Scratch) -> Result<(), String> = match page {
            TextPage::Character => |s, _| {
                s.side_tab = SideTab::Text;
                s.text_page = TextPage::Character;
                Ok(())
            },
            TextPage::Warp => |s, _| {
                s.side_tab = SideTab::Text;
                s.text_page = TextPage::Warp;
                Ok(())
            },
            TextPage::Path => |s, _| {
                s.side_tab = SideTab::Text;
                s.text_page = TextPage::Path;
                Ok(())
            },
        };
        steps.push(Step::Setup(name, set));
        steps.push(Step::WaitIdle);
        steps.push(Step::WaitFrames(3));
        steps.push(Step::Assert("the page: Color inside the panel, 300 pt wide, the 3D view in place", |s, sc| {
            text_page_layout(s, "page")?;
            same_view_rect(s, sc)
        }));
        if shots {
            steps.push(Step::Shot(shot));
        }
    }
    steps
}

/// The Text tab is shown, the Color section is inside the side panel and the side panel is at most 300 pt wide.
fn text_page_layout(s: &AppState, what: &str) -> Result<(), String> {
    let r = &s.side_rects;
    let tab = r.text_tab.ok_or_else(|| format!("{what}: the Text tab is not shown"))?;
    if !r.panel.expand(0.5).contains_rect(r.color) {
        return Err(format!("{what}: the Color section {:?} is not inside the side panel {:?}", r.color, r.panel));
    }
    if !r.bottom_area.expand(0.5).contains_rect(tab) {
        return Err(format!("{what}: the tab {tab:?} is not inside the lower area {:?}", r.bottom_area));
    }
    let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?;
    if side.width() > 300.5 {
        return Err(format!("{what}: the side panel is {} pt wide", side.width()));
    }
    log::info!("{what} ({:?}): tab {tab:?}, color {:?}, side panel {side:?}", s.text_page, r.color);
    Ok(())
}

/// Which of the Brush and the Text tab the lower area shows (`side_rects`).
fn tabs_shown(s: &AppState, brush: bool, text: bool) -> Result<(), String> {
    let r = &s.side_rects;
    if r.brush_tab.is_some() != brush || r.text_tab.is_some() != text {
        return Err(format!("brush tab {:?}, text tab {:?} (want {brush}, {text})", r.brush_tab, r.text_tab));
    }
    Ok(())
}

/// A click on a tab button of the strip.
fn tab_click(s: &AppState, tab: SideTab) -> Result<Vec<Step>, String> {
    let rect = s.side_rects.tab_buttons[tab.index()];
    if !rect.is_positive() {
        return Err(format!("the {tab:?} tab button was not drawn"));
    }
    Ok(vec![Step::ScreenPointer { path: vec![rect.center()] }])
}

/// After the hover step: the Text tool holds a picture of the draft (`ToolProbe::text_hover`, written by the tool,
/// T7) whose spec equals the draft and which has at most 262,144 pixels.
fn assert_text_hover(s: &AppState, _: &Scratch) -> Result<(), String> {
    let (spec, [w, h]) = s.tool_probe.text_hover.as_ref().ok_or("the Text tool holds no picture of the draft")?;
    let draft = crate::panels::text_block::draft_spec(&s.tools.text);
    if spec.content != draft.content || spec.fx != draft.fx || spec.font.family != draft.font.family {
        return Err(format!("the picture is of {:?}, the draft is {:?}", spec.content, draft.content));
    }
    if u64::from(*w) * u64::from(*h) > 1 << 18 && spec.fx.needs_outlines() {
        return Err(format!("the picture has {w} x {h} pixels"));
    }
    log::info!("hover picture {w} x {h} of {:?}", spec.content);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: delivered scenario file R4\notes\scenario_text_fonts.rs (pasted by the gate, sitting M2)
// Scenario `text_fonts` (FEATURE_TEXT 8.3; ROUND4_PLAN R4-11, R4-12). Author: T6 (round 4). Reads the fonts of this
// ---------------------------------------------------------------------------------------------------------------------

/// Fonts of the machine: the catalog really holds them (two hard asserts that are never skipped: at least 50 families,
/// and Arial, which is not a bundled family); B takes Arial's real Bold face and thickens Ubuntu; Candara lists its two
/// "Regular" faces apart; a CFF (.otf) family and Bahnschrift (a variable font) draw a thickened, Arc-warped "Ag".
/// A missing optional family logs SKIP.
fn text_fonts() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Dyn("wait for the font job of the start", wait_for_fonts),
        Step::Assert("the catalog holds the fonts of this machine (at least 50 families, Arial among them)", |s, _| {
            let families = s.fonts.families();
            let multi = families.iter().filter(|f| s.fonts.faces(f).len() > 1).count();
            log::info!("font catalog: {} families, {multi} with more than one face", families.len());
            if families.len() < 50 {
                return Err(format!("only {} families", families.len()));
            }
            let bundled = wte_core::FontCatalog::bundled();
            if bundled.families().iter().any(|f| f == "Arial") {
                return Err("Arial is a bundled family".to_owned());
            }
            if !families.iter().any(|f| f == "Arial") {
                return Err("Arial is not in the catalog".to_owned());
            }
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::Setup("place an Arial Regular text and press B", |s, sc| {
            let id = place_font_text(s, "Arial", "Regular", "Arial")?;
            sc.values.insert("arial", id.0);
            press_bold(s, id)
        }),
        Step::WaitIdle,
        Step::Assert("Arial gets the face \"Bold\" and no thickening", |s, sc| {
            let spec = stamp_spec(s, remembered(sc, "arial")?)?;
            log::info!("Arial with B: {} {}, bold_em {}", spec.font.family, spec.font.style, spec.fx.bold_em);
            if spec.font.style != "Bold" || spec.fx.bold_em != 0.0 {
                return Err(format!("Arial with B is {:?} with bold_em {}", spec.font.style, spec.fx.bold_em));
            }
            Ok(())
        }),
        Step::Setup("place an Ubuntu text and press B", |s, sc| {
            let id = place_font_text(s, "Ubuntu", "Light", "Ubuntu")?;
            sc.values.insert("ubuntu", id.0);
            press_bold(s, id)
        }),
        Step::WaitIdle,
        Step::Assert("Ubuntu (no bold face) is thickened by 0.02 em", |s, sc| {
            let spec = stamp_spec(s, remembered(sc, "ubuntu")?)?;
            log::info!("Ubuntu with B: {} {}, bold_em {}", spec.font.family, spec.font.style, spec.fx.bold_em);
            if spec.font.style != "Light" || (spec.fx.bold_em - 0.02).abs() > 1e-6 {
                return Err(format!("Ubuntu with B is {:?} with bold_em {}", spec.font.style, spec.fx.bold_em));
            }
            Ok(())
        }),
        Step::Assert("Candara lists \"Regular\" and \"Regular (Candara-Light)\"", |s, _| {
            let mut cache = std::collections::HashMap::new();
            let faces = crate::panels::text_block::candidates(&s.fonts, s.doc(), &mut cache, "Candara");
            if faces.is_empty() {
                log::info!("SKIP: Candara is not installed");
                return Ok(());
            }
            let styles: Vec<&str> = faces.iter().map(|f| f.style.as_str()).collect();
            log::info!("Candara styles: {styles:?}");
            if !styles.contains(&"Regular") || !styles.contains(&"Regular (Candara-Light)") {
                return Err(format!("Candara's style list is {styles:?}"));
            }
            Ok(())
        }),
        Step::Setup("a thickened, Arc-warped \"Ag\" in the first CFF (.otf) family and in Bahnschrift", |s, sc| {
            let otf = s
                .fonts
                .families()
                .iter()
                .find(|f| {
                    let style = crate::panels::text_block::pick_style(&s.fonts, f, "Regular");
                    s.fonts
                        .find(f, &style)
                        .and_then(|face| s.fonts.face_bytes(face.id))
                        .is_some_and(|(bytes, _)| wte_core::text::font_file_extension(&bytes) == "otf")
                })
                .cloned();
            for (key, family) in [("otf_ink", otf), ("bahnschrift_ink", Some("Bahnschrift".to_owned()))] {
                let Some(family) = family.filter(|f| !s.fonts.faces(f).is_empty()) else {
                    log::info!("SKIP: no {key} family is installed");
                    continue;
                };
                let ink = styled_ag_ink(s, &family)?;
                log::info!("{family}: thickened Arc-warped \"Ag\" has {ink} inked pixels");
                sc.values.insert(key, ink);
            }
            Ok(())
        }),
        Step::Assert("the two pictures have ink (more than 200 pixels) and no fully inked row or column", |s, sc| {
            let _ = s;
            for key in ["otf_ink", "bahnschrift_ink"] {
                if let Some(&ink) = sc.values.get(key)
                    && ink <= 200
                {
                    return Err(format!("{key}: {ink} inked pixels"));
                }
            }
            Ok(())
        }),
        // Gate 3 (FEATURE_TEXT 8.4 "opening the font picker with all families"; ROUND4_PLAN 3.3 item 7): a click on
        // the font button opens the picker; the sample job runs; 30 frames with the picker open, then
        // `<out>_picker.png`. The log lines "gate: font picker open/closed" frame the frames (WTE_FRAME_LOG).
        Step::Setup("gate: font picker open", |_, _| {
            log::info!("gate: font picker open");
            Ok(())
        }),
        Step::Dyn("click the font button", |s, _| {
            Ok(vec![Step::ScreenPointer { path: vec![panel_rect(s, PanelHit::FontButton)?.center()] }])
        }),
        Step::WaitFrames(2),
        Step::Dyn("wait for the font samples", wait_for_samples),
        Step::WaitFrames(30),
        Step::Assert("the picker drew the samples of every family", |s, _| {
            let families = s.fonts.families().len();
            if s.font_samples.requested_for != Some(families) || s.font_samples.loading {
                return Err(format!("samples for {:?} of {families} families, loading {}", s.font_samples.requested_for, s.font_samples.loading));
            }
            if s.font_samples.cells.is_empty() || s.font_samples.atlases.is_empty() {
                return Err("the sample job made no sample".to_owned());
            }
            log::info!("font picker: samples of {families} families ({} cells, {} atlases)", s.font_samples.cells.len(), s.font_samples.atlases.len());
            Ok(())
        }),
        Step::Shot("picker"),
        Step::Setup("gate: font picker closed", |_, _| {
            log::info!("gate: font picker closed");
            Ok(())
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
    ]
}

/// While the sample job of the font picker runs: wait and look again (at most about 30 s: 1,800 frames of 20).
fn wait_for_samples(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    if s.font_samples.loading || s.font_samples.requested_for.is_none() {
        Ok(vec![Step::WaitFrames(10), Step::Dyn("wait for the font samples", wait_for_samples)])
    } else {
        Ok(Vec::new())
    }
}

/// While the font job of the start runs: wait and look again.
fn wait_for_fonts(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    if s.fonts_loading {
        Ok(vec![Step::WaitFrames(20), Step::Dyn("wait for the font job of the start", wait_for_fonts)])
    } else {
        Ok(Vec::new())
    }
}

/// Places a projected text in `family` / `style` on the front of the sphere and selects it (Transform tool).
fn place_font_text(s: &mut AppState, family: &str, style: &str, content: &str) -> Result<StampId, String> {
    let font = crate::panels::text_block::resolve_font(s, family, style).ok_or("no document")?;
    if !font.family.eq_ignore_ascii_case(family) {
        return Err(format!("{family} {style} resolved to {} {}", font.family, font.style));
    }
    let doc = s.doc_mut().ok_or("no document")?;
    let d = doc.mesh.bounds.diagonal();
    let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
    let p = wte_core::Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::new(d * 0.1, d * 0.03), d * 0.1);
    let spec = wte_core::TextSpec::new(content, font);
    let id = doc
        .add_stamp(content, StampSource::Text(spec), Placement::Projected(p), TextureSetId(0))
        .map_err(|e| e.to_string())?;
    let session = s.session.as_mut().ok_or("no session")?;
    session.selected_stamp = Some(id);
    s.tool = ToolKind::Transform;
    Ok(id)
}

/// What a click on B does to the selected text (the panel's own function), sent as the action the click sends.
fn press_bold(s: &mut AppState, id: StampId) -> Result<(), String> {
    let target = crate::panels::text_block::TextTarget::Stamp(id);
    let spec = crate::panels::text_block::style_click(s, target, true).ok_or("not a text")?;
    s.push(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final));
    Ok(())
}

/// The spec of text stamp number `id`.
fn stamp_spec(s: &AppState, id: u64) -> Result<wte_core::TextSpec, String> {
    match &doc_of(s)?.stamp(StampId(id)).ok_or("the text is gone")?.source {
        StampSource::Text(spec) => Ok(spec.clone()),
        StampSource::Image { .. } => Err("not a text".to_owned()),
    }
}

/// Draws "Ag" in `family` thickened by 0.02 em with Arc 50 % (the document's own picture function) and returns its
/// inked pixels; fails when a row or a column is inked from end to end.
fn styled_ag_ink(s: &mut AppState, family: &str) -> Result<u64, String> {
    let style = crate::panels::text_block::pick_style(&s.fonts, family, "Regular");
    let font = crate::panels::text_block::resolve_font(s, family, &style).ok_or("no document")?;
    let mut spec = wte_core::TextSpec::new("Ag", font);
    spec.size_px = 64.0;
    spec.fx.bold_em = 0.02;
    spec.fx.warp = Some(wte_core::Warp { bend: 0.5, ..wte_core::Warp::new(wte_core::WarpStyle::Arc) });
    let doc = s.doc_mut().ok_or("no document")?;
    let raster = doc.text_preview(&spec, 1 << 22).map_err(|e| format!("{family}: {e}"))?;
    let img = &raster.image;
    let (w, h) = (img.width as usize, img.height as usize);
    let alpha = |x: usize, y: usize| img.data[(y * w + x) * 4 + 3];
    let ink = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| alpha(x, y) > 0).count() as u64;
    if (0..h).any(|y| (0..w).all(|x| alpha(x, y) == 255)) || (0..w).any(|x| (0..h).all(|y| alpha(x, y) == 255)) {
        return Err(format!("{family}: a row or a column of the {w} x {h} picture is fully inked"));
    }
    Ok(ink)
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4, gate stage 3: two measuring scenarios (FEATURE_TEXT 8.4 "twenty Preview edits", FEATURE_BRUSH P11). Their
// numbers are read from the per-frame lines of `WTE_FRAME_LOG=1`, between the "gate:" log lines.
// ---------------------------------------------------------------------------------------------------------------------

/// Needs a model path (FEATURE_TEXT 8.4: an 8192 texture; ABeautifulGame for the Preview frames of `text_warp`'s
/// limit). Places "WTE 2026" on the model, warps it (Arc 50 %, one `Final`, timed), then sends twenty `Preview` edits
/// of the bend, one per frame, and one `Final`. Asserts that the twenty Previews add no history entry and the Final
/// adds one with the last bend.
fn text_preview_path(has_path: bool) -> Vec<Step> {
    let mut steps = vec![phase!("open / import")];
    steps.extend(open_and_accept(has_path));
    if !has_path {
        return steps;
    }
    steps.extend(t7_place_3d(true));
    steps.extend([
        Step::Setup("time the Arc", |_, _| {
            t7_timer_start("warp Arc".to_owned());
            Ok(())
        }),
        Step::Dyn("warp the text: Arc 50 %", |s, _| t7_edit(s, |spec| spec.fx.warp = Some(gate_arc(0.5)))),
        Step::WaitIdle,
        Step::Setup("log the time; remember the history", |s, sc| {
            t7_timer_stop();
            remember_undo(s, sc)
        }),
        Step::Setup("gate: twenty preview edits start", |_, sc| {
            sc.values.insert("tp_i", 0);
            log::info!("gate: twenty preview edits start");
            Ok(())
        }),
    ]);
    for _ in 0..20 {
        steps.push(Step::Dyn("one Preview edit of the bend", |s, sc| {
            let (id, mut spec) = t7_selected_text(s)?;
            let i = remembered(sc, "tp_i")?;
            spec.fx.warp = Some(gate_arc(0.5 + 0.01 * (i as f32 + 1.0)));
            Ok(vec![
                Step::Setup("count the edit", |_, sc| {
                    *sc.values.entry("tp_i").or_insert(0) += 1;
                    Ok(())
                }),
                Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Preview)),
                Step::WaitFrames(1),
            ])
        }));
    }
    steps.extend([
        Step::Setup("gate: twenty preview edits end", |_, _| {
            log::info!("gate: twenty preview edits end");
            Ok(())
        }),
        Step::Assert("the twenty Previews are one pending edit and added no history entry", |s, sc| {
            let doc = doc_of(s)?;
            let (now, before) = (doc.history.undo_len() as u64, remembered(sc, "undo")?);
            if !doc.has_pending_edit() || now != before {
                return Err(format!("pending {}, {now} undo entries (before {before})", doc.has_pending_edit()));
            }
            Ok(())
        }),
        Step::Setup("time the Final", |_, _| {
            t7_timer_start("preview edits final".to_owned());
            Ok(())
        }),
        Step::Dyn("the Final of the last bend", |s, _| t7_edit(s, |spec| spec.fx.warp = Some(gate_arc(0.7)))),
        Step::WaitIdle,
        Step::Setup("log the time of the Final", |_, _| {
            t7_timer_stop();
            Ok(())
        }),
        Step::Assert("the Final added one history entry with the last bend", |s, sc| {
            undo_since(s, sc, 1)?;
            let (_, spec) = t7_selected_text(s)?;
            match spec.fx.warp {
                Some(w) if (w.bend - 0.7).abs() < 1e-6 => Ok(()),
                other => Err(format!("the warp is {other:?}")),
            }
        }),
    ]);
    steps
}

/// An Arc warp of `bend`.
fn gate_arc(bend: f32) -> wte_core::Warp {
    wte_core::Warp { bend, ..wte_core::Warp::new(wte_core::WarpStyle::Arc) }
}

/// Logs a "gate:" marker line (the measuring scenarios frame their frames with these).
fn gate_mark(what: &'static str) -> Step {
    Step::Dyn(what, |_, _| Ok(Vec::new()))
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 5 (R5-1): Final stamp bakes off the UI thread
// ---------------------------------------------------------------------------------------------------------------------

/// `final_bake_async` (PATH or the built-in sphere): with the runner's bake wait on, an image stamp is placed and
/// dragged (composite H1) and the drag undone. Then with `BakeWait(false)`, so that each next step starts while the
/// bake still runs: (a) the same drag, Undo in the next frame, Redo -> H1; (b) the drag, at once a second drag, Undo
/// -> H1; (d) a Final scale and Merge Down in the next frame -> the merged pixels of the same steps with the wait on;
/// (c) a text stamp gets an Arc warp (Final) and Save As in the next frame; the reopened project's composite equals
/// the in-memory one; (f) a Final scale of the text and Export in the next frame -> the same files, byte for byte, as
/// the same steps with the wait on; (e) a Final and Delete in the next frame -> no layer of the text comes back.
/// Check-and-fix, the other rows of the plan's event table (1.5), each against the same steps with the wait on: (g) a
/// Final, then the Preview view mode, a tool change and the camera at once -> the same composite; (i) a Final, then a
/// brush stroke at once (a result that lands during the stroke waits for its end) -> the same composite; (k) a Final,
/// the export dialog in the next frame -> it opens with nothing waiting; (h) a Final, Rasterize in the next frame ->
/// the same pixels; (j) a Final, another document in the next frame -> the late result is dropped (other document).
/// In the asynchronous mode the log shows "result dropped (stale)" (a, e), "result dropped (other document)" (j) and
/// "settle waited" (c, d, f, k, h); with `WTE_SYNC_FINAL_BAKES=1` the same steps give the same hashes.
fn final_bake_async(has_path: bool) -> Vec<Step> {
    let mut steps = if has_path { open_and_accept(true) } else { vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere)), Step::WaitIdle] };
    steps.extend([
        Step::Setup("register the sample stamp image", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| {
            placed_stamp(s, false).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::Setup("remember the composite of the placed stamp", |s, sc| remember_composite(s, sc, "fba_placed")),
        fba_drag("drag the stamp body", false),
        Step::WaitIdle,
        Step::Assert("the drag is baked", fba_nothing_unbaked),
        Step::Setup("remember H1, the composite after the drag", |s, sc| remember_composite(s, sc, "fba_h1")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("the undo gives the placed stamp back", |s, sc| fba_hash_is(s, sc, "fba_placed")),
        // (a) the same drag, Undo in the frame after the release, Redo.
        Step::BakeWait(false),
        fba_drag("(a) the same drag", false),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("(a) drag, Undo at once, Redo: H1", |s, sc| fba_hash_is(s, sc, "fba_h1")),
        // (b) from the placed stamp: the drag, at once a second drag, Undo of the second.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(b) start: the placed stamp", |s, sc| fba_hash_is(s, sc, "fba_placed")),
        fba_drag("(b) the drag", false),
        fba_drag("(b) at once a second drag", true),
        Step::WaitIdle,
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(b) drag, second drag at once, Undo: H1", |s, sc| fba_hash_is(s, sc, "fba_h1")),
        // (d) a Final then Merge Down: first with the wait on (the reference), then at once.
        Step::BakeWait(true),
        Step::Dyn("(d) reference: Final scale, then Merge Down after its bake", fba_final_then_merge),
        Step::WaitIdle,
        Step::Setup("remember the merged composite and layer", fba_remember_merged),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(d) two undos give H1 back", |s, sc| fba_hash_is(s, sc, "fba_h1")),
        Step::BakeWait(false),
        Step::Dyn("(d) Final scale, Merge Down in the next frame", fba_final_then_merge),
        Step::WaitIdle,
        Step::Assert("(d) the merged layer equals the reference", fba_merged_equal),
        // (c) a text stamp gets an Arc warp (Final) and is saved in the next frame; the reopened project equals it.
        Step::BakeWait(true),
        Step::Setup("text content \"WTE\"", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            s.tools.text.em_frac = 0.08;
            s.tools.text.color = [20, 20, 20, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Dyn("place the text stamp", |s, _| {
            let (xf, _, second) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, second.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the text stamp was placed and selected", |s, _| {
            placed_stamp(s, true).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::BakeWait(false),
        Step::Dyn("(c) Arc warp (Final), Save As in the next frame", |s, sc| {
            let (id, mut spec) = t7_selected_text(s)?;
            spec.fx.warp = Some(gate_arc(0.5));
            Ok(vec![
                Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final)),
                Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, "_fba.wtep")))),
            ])
        }),
        Step::WaitIdle,
        Step::Assert("(c) the project was saved with the warped text", |s, sc| {
            let path = with_suffix(&sc.out_stem, "_fba.wtep");
            let doc = doc_of(s)?;
            if doc.is_modified() || doc.has_unbaked_stamps() {
                return Err("the document is modified or has unbaked stamps after the save".to_owned());
            }
            let (_, spec) = t7_selected_text(s)?;
            if spec.fx.warp.is_none() {
                return Err("the text has no warp".to_owned());
            }
            file_non_empty(&path)
        }),
        Step::Setup("remember the in-memory composite", |s, sc| remember_composite(s, sc, "fba_saved")),
        Step::Dyn("reopen the saved project", |_, sc| {
            Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, "_fba.wtep"))))])
        }),
        Step::WaitIdle,
        Step::Assert("(c) the reopened composite equals the in-memory one", |s, sc| {
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, "_fba.wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            fba_hash_is(s, sc, "fba_saved")
        }),
        // (f) check-and-fix: a Final then Export in the next frame writes the files of the same steps with the wait on.
        Step::BakeWait(true),
        Step::Setup("(f) start with empty export folders", |_, sc| {
            for suffix in [FBA_EXPORT_REF, FBA_EXPORT] {
                let dir = with_suffix(&sc.out_stem, suffix);
                if dir.exists() {
                    std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
                }
            }
            Ok(())
        }),
        Step::Dyn("(f) reference: Final scale of the text, Export after its bake", |s, sc| fba_final_then_export(s, sc, FBA_EXPORT_REF)),
        Step::WaitIdle,
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(f) the undo gives the reopened composite back", |s, sc| fba_hash_is(s, sc, "fba_saved")),
        Step::BakeWait(false),
        Step::Dyn("(f) Final scale of the text, Export in the next frame", |s, sc| fba_final_then_export(s, sc, FBA_EXPORT)),
        Step::WaitIdle,
        Step::Assert("(f) the export equals the reference", fba_exports_equal),
        // (e) a Final then Delete in the next frame: no layer of the text comes back.
        Step::Dyn("(e) Final scale of the text, Delete in the next frame", |s, _| {
            let (id, _) = text_stamp(s)?;
            let placement = doc_of(s)?.stamp(id).ok_or("the text stamp is gone")?.placement;
            Ok(vec![
                Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 0.9), Commit::Final)),
                Step::Do(AppAction::DeleteStamp(id)),
            ])
        }),
        Step::WaitIdle,
        Step::Assert("(e) the text and its layers are gone", |s, _| {
            let doc = doc_of(s)?;
            if doc.stamps.iter().any(|st| matches!(st.source, StampSource::Text(_))) {
                return Err("a text stamp is still there".to_owned());
            }
            if doc.sets.iter().flat_map(|set| &set.layers).any(|l| matches!(l.kind, LayerKind::Text { .. })) {
                return Err("a text layer came back".to_owned());
            }
            Ok(())
        }),
        Step::Assert("(e) nothing is left to bake", fba_nothing_unbaked),
        // Check-and-fix: the other rows of the plan's event table (1.5) as steps.
        // (g) Preview view mode, tool changes and the camera right after a Final: nothing waits, the picture lands.
        Step::BakeWait(true),
        // The reopened project of (c) is a new session: the Stamp tool's image went with the old one.
        Step::Setup("register the sample stamp image for the reopened project", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place a second image stamp", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the second image stamp was placed and selected", |s, _| {
            placed_stamp(s, false).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::Setup("remember the composite of the second stamp", |s, sc| remember_composite(s, sc, "fba_second")),
        Step::Dyn("(g) reference: Final scale, then view mode, tool and camera after its bake", fba_final_then_view),
        Step::WaitIdle,
        Step::Setup("remember the composite of (g)", |s, sc| remember_composite(s, sc, "fba_view")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(g) the undo gives the second stamp back", |s, sc| fba_hash_is(s, sc, "fba_second")),
        Step::BakeWait(false),
        Step::Dyn("(g) Final scale, then view mode, tool and camera at once", fba_final_then_view),
        Step::WaitIdle,
        Step::Assert("(g) the composite equals the reference", |s, sc| {
            fba_nothing_unbaked(s, sc)?;
            fba_hash_is(s, sc, "fba_view")
        }),
        // (i) a brush stroke right after a Final: not blocked; a result that lands during the stroke waits for its end.
        Step::BakeWait(true),
        Step::Dyn("(i) make a paint layer active", fba_select_paint),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Setup("(i) remember the history length", remember_undo),
        Step::Dyn("(i) reference: Final scale, then a stroke after its bake", fba_final_then_stroke),
        Step::WaitIdle,
        Step::Assert("(i) the Final and the stroke are two entries", |s, sc| undo_since(s, sc, 2)),
        Step::Setup("remember the composite of (i)", |s, sc| remember_composite(s, sc, "fba_stroke")),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(i) two undos give (g) back", |s, sc| fba_hash_is(s, sc, "fba_view")),
        Step::Setup("(i) remember the history length again", remember_undo),
        Step::BakeWait(false),
        Step::Dyn("(i) Final scale, the stroke in the next frame", fba_final_then_stroke),
        Step::WaitIdle,
        Step::Assert("(i) the Final and the stroke are two entries", |s, sc| undo_since(s, sc, 2)),
        Step::Assert("(i) the composite equals the reference", |s, sc| {
            fba_nothing_unbaked(s, sc)?;
            fba_hash_is(s, sc, "fba_stroke")
        }),
        // (k) the export dialog right after a Final: it opens after the bake landed (settle).
        Step::Dyn("(k) Final scale, the export dialog in the next frame", |s, _| {
            let (id, placement, _) = fba_image_stamp(s)?;
            Ok(vec![
                Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 0.9), Commit::Final)),
                Step::Do(AppAction::ShowExport),
            ])
        }),
        Step::Assert("(k) the export dialog is open and nothing waits for a bake", |s, _| {
            if !matches!(s.dialog, Dialog::Export(_)) {
                return Err("the export dialog is not open".to_owned());
            }
            if doc_of(s)?.has_unbaked_stamps() {
                return Err("the export dialog opened before the Final bake landed".to_owned());
            }
            Ok(())
        }),
        Step::Do(AppAction::CloseDialog),
        Step::WaitIdle,
        // (h) a Final then Rasterize at once: the rasterized layer equals the one of the same steps with the wait on.
        Step::Setup("remember the composite before (h)", |s, sc| remember_composite(s, sc, "fba_before_raster")),
        Step::BakeWait(true),
        Step::Dyn("(h) reference: Final scale, then Rasterize after its bake", fba_final_then_rasterize),
        Step::WaitIdle,
        Step::Setup("remember the rasterized composite and layers", |s, sc| {
            let (composite, layers) = fba_merged_keys(s)?;
            sc.values.insert("fba_raster", composite);
            sc.values.insert("fba_raster_layers", layers);
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(h) two undos give the state before back", |s, sc| fba_hash_is(s, sc, "fba_before_raster")),
        Step::BakeWait(false),
        Step::Dyn("(h) Final scale, Rasterize in the next frame", fba_final_then_rasterize),
        Step::WaitIdle,
        Step::Assert("(h) the rasterized layer equals the reference", |s, sc| {
            fba_nothing_unbaked(s, sc)?;
            if doc_of(s)?.stamps.iter().any(|st| matches!(st.source, StampSource::Image { .. })) {
                return Err("the image stamp is still there after Rasterize".to_owned());
            }
            let (composite, layers) = fba_merged_keys(s)?;
            if composite != remembered(sc, "fba_raster")? || layers != remembered(sc, "fba_raster_layers")? {
                return Err("Rasterize right after the Final gave other pixels than after the bake".to_owned());
            }
            Ok(())
        }),
        // (j) a Final then another document at once: the late result is dropped, the new document is untouched.
        Step::BakeWait(true),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place a third image stamp", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the third image stamp was placed and selected", |s, _| {
            placed_stamp(s, false).map(|_| ()).and_then(|_| tool_is(s, ToolKind::Transform))
        }),
        Step::BakeWait(false),
        Step::Dyn("(j) Final scale, the sphere sample in the next frame (unsaved changes discarded)", |s, _| {
            let (id, placement, _) = fba_image_stamp(s)?;
            Ok(vec![
                Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 0.9), Commit::Final)),
                Step::Do(AppAction::ConfirmPending(Pending::OpenSample(SampleKind::Sphere))),
            ])
        }),
        Step::WaitIdle,
        Step::Assert("(j) the new document has no stamp and nothing waits", |s, sc| {
            fba_nothing_unbaked(s, sc)?;
            let session = session_of(s)?;
            if session.path.is_some() || !session.doc.stamps.is_empty() {
                return Err(format!("the open document is {:?} with {} stamp(s)", session.path, session.doc.stamps.len()));
            }
            if s.await_first_bake.is_some() {
                return Err("a first bake of the old document is still awaited".to_owned());
            }
            Ok(())
        }),
        // An image stamp for the final screenshot.
        Step::BakeWait(true),
        Step::Setup("register the sample stamp image again", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place an image stamp for the screenshot", |s, _| {
            let (xf, first, _) = stamp_spots(s)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, first.pos))])
        }),
        Step::WaitIdle,
        Step::Assert("the image stamp for the screenshot was placed", |s, _| placed_stamp(s, false).map(|_| ())),
    ]);
    steps
}

/// The (first) image stamp: its id, placement and the layer in its home set.
fn fba_image_stamp(s: &AppState) -> Result<(StampId, Placement, LayerId), String> {
    let doc = doc_of(s)?;
    let stamp = doc.stamps.iter().find(|st| matches!(st.source, StampSource::Image { .. })).ok_or("there is no image stamp")?;
    let (id, placement, home) = (stamp.id, stamp.placement, stamp.home_set);
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home).ok_or("no home layer")?;
    Ok((id, placement, layer))
}

/// (g): a Final scale (0.9) of the image stamp, then the Preview view mode, the Brush tool, the camera framed, and the
/// earlier view mode and the Transform tool again.
fn fba_final_then_view(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    let (id, placement, _) = fba_image_stamp(s)?;
    let mode = if s.view_mode == ViewMode::Preview { ViewMode::View3D } else { s.view_mode };
    Ok(vec![
        Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 0.9), Commit::Final)),
        Step::Do(AppAction::SetViewMode(ViewMode::Preview)),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::FrameModel),
        Step::Do(AppAction::SetViewMode(mode)),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
    ])
}

/// (i): in every set whose active layer is no paint layer, the topmost visible unlocked paint layer becomes active (the
/// image stamp's home set last).
fn fba_select_paint(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    let doc = doc_of(s)?;
    let home = fba_image_stamp(s)?.0;
    let home = doc.stamp(home).map(|st| st.home_set).ok_or("the image stamp is gone")?;
    let mut sets: Vec<TextureSetId> = (0..doc.sets.len() as u32).map(TextureSetId).filter(|set| *set != home).collect();
    sets.push(home);
    let mut steps = Vec::new();
    for set in sets {
        if matches!(doc.paint_target(set), wte_core::PaintTarget::Ready(_)) {
            continue;
        }
        let layers = &doc.sets[set.index()].layers;
        if let Some(layer) = layers.iter().rev().find(|l| l.is_raster() && l.visible && !l.locked) {
            steps.push(Step::Do(AppAction::SelectLayer(layer.id)));
        }
    }
    if !matches!(doc.paint_target(home), wte_core::PaintTarget::Ready(_)) && steps.is_empty() {
        return Err("the image stamp's set has no paint layer to paint on".to_owned());
    }
    Ok(steps)
}

/// (i): a Final scale (1.1) of the image stamp, then a brush stroke across the lower half of the model.
fn fba_final_then_stroke(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    let (id, placement, _) = fba_image_stamp(s)?;
    let mut steps = vec![Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 1.1), Commit::Final))];
    steps.extend(model_stroke(s, ViewId::Main3D, &[[-0.55, 0.42], [-0.1, 0.5], [0.35, 0.46], [0.6, 0.3]])?);
    Ok(steps)
}

/// (h): a Final scale (0.9) of the image stamp, then Rasterize of its layer in its home set.
fn fba_final_then_rasterize(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    let (id, placement, layer) = fba_image_stamp(s)?;
    Ok(vec![
        Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 0.9), Commit::Final)),
        Step::Do(AppAction::RasterizeLayer(layer)),
    ])
}

/// A drag of the selected stamp's gizmo body (computed when the step runs): 60 / -35 points, or -40 / 30 points for
/// the `second` drag of (b).
fn fba_drag(label: &'static str, second: bool) -> Step {
    fn body(s: &AppState, delta: egui::Vec2) -> Result<Vec<Step>, String> {
        let (xf, shape) = selected_gizmo(s)?;
        Ok(vec![drag(&xf, &[shape.center, shape.center + delta])])
    }
    if second {
        Step::Dyn(label, |s, _| body(s, egui::vec2(-40.0, 30.0)))
    } else {
        Step::Dyn(label, |s, _| body(s, egui::vec2(60.0, -35.0)))
    }
}

/// The composite equals the remembered one under `key`.
fn fba_hash_is(s: &AppState, sc: &Scratch, key: &str) -> Result<(), String> {
    let now = composite_hash(doc_of(s)?);
    let want = remembered(sc, key)?;
    if now != want { Err(format!("composite {now:016x}, expected \"{key}\" {want:016x}")) } else { Ok(()) }
}

/// No stamp waits for its Final bake and none is pending in the app.
fn fba_nothing_unbaked(s: &AppState, _: &Scratch) -> Result<(), String> {
    if doc_of(s)?.has_unbaked_stamps() || s.final_bakes_pending {
        return Err("a Final bake is still pending".to_owned());
    }
    Ok(())
}

/// Export folders of `final_bake_async` (f): the reference (wait on) and the one written right after the Final.
const FBA_EXPORT_REF: &str = "_fba_export_ref";
const FBA_EXPORT: &str = "_fba_export";

/// A Final scale (1.1) of the text stamp, then an export (flat PNG and one PNG per layer) into `suffix`.
fn fba_final_then_export(s: &AppState, sc: &Scratch, suffix: &str) -> Result<Vec<Step>, String> {
    let (id, _) = text_stamp(s)?;
    let placement = doc_of(s)?.stamp(id).ok_or("the text stamp is gone")?.placement;
    let mut opts = export_options(s, sc)?;
    opts.dir = with_suffix(&sc.out_stem, suffix);
    opts.ora = false;
    opts.psd = false;
    Ok(vec![
        Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 1.1), Commit::Final)),
        Step::Do(AppAction::RunExport(opts)),
    ])
}

/// The two export folders of (f) hold the same files, byte for byte, and nothing waits for a bake.
fn fba_exports_equal(s: &AppState, sc: &Scratch) -> Result<(), String> {
    fba_nothing_unbaked(s, sc)?;
    let files = |suffix: &str| -> Result<Vec<(String, Vec<u8>)>, String> {
        let dir = with_suffix(&sc.out_stem, suffix);
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            files.push((path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), bytes));
        }
        files.sort();
        Ok(files)
    };
    let (reference, fast) = (files(FBA_EXPORT_REF)?, files(FBA_EXPORT)?);
    let names = |f: &[(String, Vec<u8>)]| f.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    if reference.is_empty() || !names(&reference).iter().any(|n| n.contains("_text_")) {
        return Err(format!("the reference export has no text layer file: {:?}", names(&reference)));
    }
    if names(&reference) != names(&fast) {
        return Err(format!("other files: {:?} vs {:?}", names(&reference), names(&fast)));
    }
    if let Some((name, _)) = reference.iter().zip(&fast).find(|(a, b)| a.1 != b.1).map(|(a, _)| a) {
        return Err(format!("{name} differs from the reference: the export ran before the Final bake"));
    }
    Ok(())
}

/// A Final scale (0.9) of the image stamp, then Merge Down of its layer in its home set.
fn fba_final_then_merge(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    let doc = doc_of(s)?;
    let stamp = doc.stamps.iter().find(|st| matches!(st.source, StampSource::Image { .. })).ok_or("there is no image stamp")?;
    let (id, placement, home) = (stamp.id, stamp.placement, stamp.home_set);
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home).ok_or("no home layer")?;
    Ok(vec![
        Step::Do(AppAction::UpdateStampPlacement(id, scaled(&placement, 0.9), Commit::Final)),
        Step::Do(AppAction::MergeDown(layer)),
    ])
}

/// The composite and the pixels of every paint layer (the merged one among them) after Merge Down.
fn fba_merged_keys(s: &AppState) -> Result<(u64, u64), String> {
    let doc = doc_of(s)?;
    let pixels = doc.sets.iter().flat_map(|set| &set.layers).fold(0xcbf2_9ce4_8422_2325u64, |h, l| match &l.kind {
        LayerKind::Raster(buf) => wte_core::fnv1a64(h, &wte_core::testutil::hash_pixels(buf).to_le_bytes()),
        _ => h,
    });
    Ok((composite_hash(doc), pixels))
}

fn fba_remember_merged(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let (composite, layer) = fba_merged_keys(s)?;
    sc.values.insert("fba_merged", composite);
    sc.values.insert("fba_merged_layer", layer);
    Ok(())
}

fn fba_merged_equal(s: &AppState, sc: &Scratch) -> Result<(), String> {
    let (composite, layer) = fba_merged_keys(s)?;
    if composite != remembered(sc, "fba_merged")? || layer != remembered(sc, "fba_merged_layer")? {
        return Err("Merge Down right after the Final gave other pixels than after the bake".to_owned());
    }
    Ok(())
}

/// FEATURE_BRUSH P11: the sphere sample with the Brush tool and the Brush tab; 60 idle frames, then 60 hover frames
/// across the model with Circular (no shape), then 60 with Sphere and the Cube (a shape cursor). The runner logs every
/// step with its frame number, so the `gate: ...` steps mark the frames. Asserts that the Cube cursor was drawn.
fn brush_hover() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Setup("Circular, Size 64, the Brush tab", |s, _| {
            s.tools.brush = BrushSettings { kind: BrushType::Circular, shape: ShapeChoice::Ball, size_px: 64.0, ..BrushSettings::default() };
            s.side_tab = SideTab::Brush;
            Ok(())
        }),
        Step::WaitIdle,
        Step::WaitFrames(10),
        Step::Assert("the Brush tab is shown", |s, _| tabs_shown(s, true, false)),
        gate_mark("gate: idle brush tab start"),
        Step::WaitFrames(60),
        gate_mark("gate: idle brush tab end"),
    ];
    let hovers = || (0..60).map(|i| {
        let t = i as f32 / 59.0;
        Step::Hover { view: ViewId::Main3D, at: [0.42 + 0.16 * t, 0.45 + 0.1 * (t * 6.0).sin()] }
    });
    steps.push(gate_mark("gate: hover circular start"));
    steps.extend(hovers());
    steps.push(gate_mark("gate: hover circular end"));
    steps.extend([
        Step::Setup("Sphere with the Cube", |s, _| {
            s.tools.brush = BrushSettings { kind: BrushType::Sphere, shape: ShapeChoice::Cube, size_px: 64.0, ..BrushSettings::default() };
            Ok(())
        }),
        Step::Hover { view: ViewId::Main3D, at: [0.45, 0.5] },
        Step::WaitIdle,
        Step::WaitFrames(10),
        gate_mark("gate: hover cube start"),
    ]);
    steps.extend(hovers());
    steps.push(gate_mark("gate: hover cube end"));
    steps.push(Step::Assert("the Cube cursor was drawn", |s, _| {
        let p = &s.tool_probe;
        log::info!("brush cursor after the hovers: {p:?}");
        if p.cursor_footprint == 0 {
            return Err(format!("no footprint was drawn: {p:?}"));
        }
        Ok(())
    }));
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 6 (R6-2): wrap on the surface
// ---------------------------------------------------------------------------------------------------------------------

/// Size of the `cells` test picture of `wrap_surface` (ROUND6_PLAN 5.2 / 4.7).
const WS_PICTURE: [u32; 2] = [512, 256];
/// The saved project of `wrap_surface` step 9 (suffix of the out stem).
const WS_PROJECT: &str = "_wrap.wtep";
/// Least `covered` of the wrapped text of step 7 (the plan's 0.999). Package B measured 0.9979-0.9990 on the built-in
/// sphere's facets and asserted 0.995; check:B traced the gaps to halo triangles reaching into the picture on a coarse
/// mesh (their missing corners placed per triangle left wedges between them) and made them share the front's tentative
/// positions (`wrap_param.rs`, `Layout::finish`), so the plan's value holds again.
const WS_TEXT_COVERED: f32 = 0.999;

/// Logs where the picture rectangle of wrapped stamp `id` has no surface under it: `stamp_wrap_surface` on a 32 x 16
/// grid of stamp-local points (cell centres), as a character map ('#' = no surface) and a count.
fn ws_holes(doc: &Document, id: StampId, p: &Projector, what: &str) {
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    let (cols, rows) = (32, 16);
    let mut none = 0;
    let mut map = String::new();
    for j in 0..rows {
        let y = hh * (1.0 - 2.0 * (j as f32 + 0.5) / rows as f32);
        map.push('\n');
        for i in 0..cols {
            let x = hw * (2.0 * (i as f32 + 0.5) / cols as f32 - 1.0);
            let hole = doc.stamp_wrap_surface(id, Vec2::new(x, y)).is_none();
            none += usize::from(hole);
            map.push(if hole { '#' } else { '.' });
        }
    }
    log::info!("wrap_surface {what}: {none} of {} grid points of the picture have no surface:{map}", cols * rows);
}

/// `wrap_surface` (ROUND6_PLAN 5.2; PATH optional). On the built-in sphere, 3D view: (1) a 30 % image stamp S of the
/// 512 x 256 `cells` picture placed in Project left of the centre (composite H0); (2) S to Wrap: ink area / picture
/// 0.93-1.07, covered >= 0.999, no cut, H1; (3) back to Project = H0, Undo = H1, Undo = H0, Redo twice = H0; (4) Wrap
/// again and stretched along its spine to 1.2 x the sphere's circumference (the height kept, so the picture is a band
/// round the sphere): the ends meet, the band within 0.9 hh of the spine's great circle inked >= 99 %, smear <= 1 %
/// (H2); (4b) Overlap: 3 turns, one entry, the two overlap stretches show the picture's right half (the later turn on
/// top); Undo = H2; Overlap, Project and Wrap again keep Overlap; Ends meet = H2; (4c) Jump small gaps on the one-part
/// sphere = H2, jumps 0, one entry; (4d) in Project the two option controls are shown greyed out; (5) back to 30 %, then
/// moved onto the sphere's UV seam with the picture across it: seam score <= 1.25 x interior + 2 levels (WA6); Undo;
/// (6) a body drag of a quarter of the view moves the anchor >= 0.3 r along the surface, ink 0.93-1.07, one entry;
/// (7) a wrapped "WRAP AROUND" text with an Arc warp (bend 30 %): ink > 0, covered >= 0.999; (8) with the bake wait off
/// a toggle and Undo in the next frame give the composite before the toggle; (9) S set to Overlap + Jump small gaps,
/// saved and reopened: the same composite, both stamps wrapped, S with both options, the manifest's
/// `min_reader_version` 3 and the option keys on S only. (10) With a PATH: the model is opened, the picture placed at
/// 40 % in Wrap near the centre of the view, its `WrapStats` logged, then the same with Jump small gaps (jumps logged);
/// ink area > 0. Every assert logs its measured values ("wrap_surface ...").
fn wrap_surface(path: Option<PathBuf>) -> Vec<Step> {
    let mut steps = match &path {
        // The app opened PATH at start: accept it, then the sphere (PATH comes back in step 10).
        Some(_) => {
            let mut s = open_and_accept(true);
            s.push(Step::Do(AppAction::ConfirmPending(Pending::OpenSample(SampleKind::Sphere))));
            s
        }
        None => vec![Step::Do(AppAction::OpenSample(SampleKind::Sphere))],
    };
    steps.extend([
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Setup("register the 512 x 256 cells picture", ws_setup_cells),
        Step::Setup("Stamp tool: 30 %, no rotation, Project", |s, _| {
            s.tools.stamp.size_frac = 0.30;
            s.tools.stamp.rotation_deg = 0.0;
            s.tools.stamp.mode = SurfaceMode::Project;
            s.tools.stamp.wrap = WrapOptions::default();
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        // (1)
        Step::Dyn("(1) place S left of the centre", |s, _| model_click(s, ViewId::Main3D, [-0.2, 0.0])),
        Step::WaitIdle,
        Step::Setup("(1) remember S, its size and H0", |s, sc| {
            let (id, placement) = placed_stamp(s, false)?;
            tool_is(s, ToolKind::Transform)?;
            let Placement::Projected(p) = placement else { return Err("S is not a 3D stamp".to_owned()) };
            if p.mode != SurfaceMode::Project {
                return Err(format!("S was placed in {:?}", p.mode));
            }
            sc.values.insert("ws_id", id.0);
            sc.values.insert("ws_half", pack(p.half_size));
            log::info!("wrap_surface (1): S = stamp {}, half size {:?}, origin {:?}", id.0, p.half_size, p.origin);
            remember_composite(s, sc, "ws_h0")?;
            remember_undo(s, sc)
        }),
        // (2)
        Step::Dyn("(2) S to Wrap (Final)", |s, sc| ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Wrap, ..p })),
        Step::WaitIdle,
        Step::Assert("(2) wrapped: ink 0.93-1.07, covered >= 0.999, no cut, one entry, composite changed", |s, sc| {
            let ratio = ws_ink_ratio(s, sc)?;
            let st = ws_stats(s, sc)?;
            log::info!("wrap_surface (2): ink / picture {ratio:.4} (plan 0.93-1.07), {st:?}");
            let (id, p) = ws_s(s, sc)?;
            ws_holes(doc_of(s)?, id, &p, "(2) S");
            undo_since(s, sc, 1)?;
            if !(0.93..=1.07).contains(&ratio) {
                return Err(format!("ink / picture area {ratio:.4}"));
            }
            if st.covered < 0.999 || st.cut_tris != 0 {
                return Err(format!("covered {:.4}, {} cut triangles", st.covered, st.cut_tris));
            }
            changed_since(s, sc, "ws_h0")
        }),
        // (2b) Round 8 (plan 4.1): the sphere is curved, so the wrap is relaxed; once, whole, no cut.
        Step::Assert("(2b) relaxed on the sphere: seams 0, dup <= 1 %, scatter <= 2 %", |s, sc| {
            let st = ws_stats(s, sc)?;
            let q = ws_quality(s, sc)?;
            log::info!(
                "wrap_surface (2b): relaxed {}, seams {} {:?}, stretch {:.1} %, rejected {:.2} %, flipped {}, cg_iters {}; dup {:.3} % (plan <= 1 %), scatter {:.3} % (plan <= 2 %), bins {}",
                st.relaxed,
                st.seams,
                st.seams_by_rule,
                st.stretch * 100.0,
                st.rejected * 100.0,
                st.flipped,
                st.cg_iters,
                q.dup * 100.0,
                q.scatter * 100.0,
                q.bins
            );
            if !st.relaxed {
                return Err("S on the sphere was not relaxed".to_owned());
            }
            if st.seams != 0 {
                return Err(format!("{} seam edges inside the picture {:?}", st.seams, st.seams_by_rule));
            }
            if q.dup > 0.01 || q.scatter > 0.02 {
                return Err(format!("dup {:.3} %, scatter {:.3} %", q.dup * 100.0, q.scatter * 100.0));
            }
            Ok(())
        }),
        Step::Setup("(2) remember H1", |s, sc| remember_composite(s, sc, "ws_h1")),
        Step::WaitFrames(2),
        Step::Shot("wrapped"),
        // (3)
        Step::Dyn("(3) S back to Project (Final)", |s, sc| ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Project, ..p })),
        Step::WaitIdle,
        Step::Assert("(3) Project again: H0, a second entry", |s, sc| {
            undo_since(s, sc, 2)?;
            ws_hash_is(s, sc, "ws_h0")
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(3) Undo: H1", |s, sc| ws_hash_is(s, sc, "ws_h1")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(3) Undo: H0", |s, sc| ws_hash_is(s, sc, "ws_h0")),
        Step::Do(AppAction::Redo),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("(3) Redo twice: H0 in Project", |s, sc| {
            if ws_s(s, sc)?.1.mode != SurfaceMode::Project {
                return Err("S is not in Project after the two redos".to_owned());
            }
            ws_hash_is(s, sc, "ws_h0")
        }),
        // (4)
        Step::Dyn("(4) S to Wrap again (Final)", |s, sc| ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Wrap, ..p })),
        Step::WaitIdle,
        Step::Dyn("(4) stretch S along its spine to 1.2 x the circumference (Final)", |s, sc| {
            let (id, p) = ws_s(s, sc)?;
            let (_, r) = ws_sphere(doc_of(s)?);
            let hw = 1.2 * std::f32::consts::TAU * r / 2.0;
            log::info!("wrap_surface (4): half width {:.4} -> {hw:.4} (radius {r:.4}), half height {:.4}", p.half_size.x, p.half_size.y);
            let stretched = Projector { half_size: Vec2::new(hw, p.half_size.y), ..p };
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, Placement::Projected(stretched), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("(4) the ends meet; the band is inked >= 99 %, smear <= 1 %", |s, sc| {
            let st = ws_stats(s, sc)?;
            if !st.met_itself {
                return Err(format!("the spine did not meet itself: {st:?}"));
            }
            let band = ws_band(s, sc)?;
            log::info!(
                "wrap_surface (4) Ends meet: {st:?}; band {} texels, inked {:.4} (plan >= 0.99), smear {:.4} % of {} opaque (plan <= 1 %)",
                band.texels,
                band.inked as f64 / band.texels.max(1) as f64,
                100.0 * band.smear as f64 / band.opaque.max(1) as f64,
                band.opaque
            );
            if band.texels < 1000 {
                return Err(format!("only {} band texels", band.texels));
            }
            if (band.inked as f64) < 0.99 * band.texels as f64 {
                return Err(format!("the band is inked {} of {}", band.inked, band.texels));
            }
            if band.smear as f64 > 0.01 * band.opaque as f64 {
                return Err(format!("{} smeared of {} opaque band texels", band.smear, band.opaque));
            }
            Ok(())
        }),
        Step::Setup("(4) remember H2 and the history", |s, sc| {
            remember_composite(s, sc, "ws_h2")?;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(2),
        Step::Shot("band"),
        // (4b)
        Step::Dyn("(4b) Overlap (Final)", |s, sc| ws_edit(s, sc, |p| Projector { wrap: WrapOptions { overlap: true, ..p.wrap }, ..p })),
        Step::WaitIdle,
        Step::Assert("(4b) Overlap: 3 turns, one entry, the later turn on top in both overlap stretches", |s, sc| {
            undo_since(s, sc, 1)?;
            let st = ws_stats(s, sc)?;
            let band = ws_band(s, sc)?;
            let share = band.right as f64 / band.overlap_opaque.max(1) as f64;
            log::info!(
                "wrap_surface (4b) Overlap: {st:?}; overlap stretches: {} opaque texels, {:.4} show the picture's right half (plan >= 0.90)",
                band.overlap_opaque,
                share
            );
            if st.turns != 3 {
                return Err(format!("{} turns, expected 3", st.turns));
            }
            if band.overlap_opaque < 200 || share < 0.90 {
                return Err(format!("{} of {} texels of the overlap stretches show the right half", band.right, band.overlap_opaque));
            }
            Ok(())
        }),
        Step::Setup("(4b) remember the Overlap state", |s, sc| remember_composite(s, sc, "ws_hov")),
        Step::WaitFrames(2),
        Step::Shot("overlap"),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(4b) Undo: H2", |s, sc| ws_hash_is(s, sc, "ws_h2")),
        Step::Dyn("(4b) Overlap again (Final)", |s, sc| ws_edit(s, sc, |p| Projector { wrap: WrapOptions { overlap: true, ..p.wrap }, ..p })),
        Step::WaitIdle,
        Step::Assert("(4b) Overlap again: the Overlap state", |s, sc| ws_hash_is(s, sc, "ws_hov")),
        Step::Dyn("(4b) to Project (Final)", |s, sc| ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Project, ..p })),
        Step::WaitIdle,
        Step::Dyn("(4b) back to Wrap (Final)", |s, sc| ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Wrap, ..p })),
        Step::WaitIdle,
        Step::Assert("(4b) Project and back: still Overlap, the Overlap state", |s, sc| {
            if !ws_s(s, sc)?.1.wrap.overlap {
                return Err("Overlap was lost in Project".to_owned());
            }
            ws_hash_is(s, sc, "ws_hov")
        }),
        Step::Dyn("(4b) Ends meet (Final)", |s, sc| ws_edit(s, sc, |p| Projector { wrap: WrapOptions { overlap: false, ..p.wrap }, ..p })),
        Step::WaitIdle,
        Step::Assert("(4b) Ends meet: H2", |s, sc| ws_hash_is(s, sc, "ws_h2")),
        // (4c)
        Step::Setup("(4c) remember the history", remember_undo),
        Step::Dyn("(4c) Jump small gaps on (Final)", |s, sc| ws_edit(s, sc, |p| Projector { wrap: WrapOptions { jump_gaps: true, ..p.wrap }, ..p })),
        Step::WaitIdle,
        Step::Assert("(4c) one part: H2 byte for byte, no jump, one entry", |s, sc| {
            let st = ws_stats(s, sc)?;
            log::info!("wrap_surface (4c) Jump small gaps on the sphere: {st:?}");
            undo_since(s, sc, 1)?;
            if !ws_s(s, sc)?.1.wrap.jump_gaps || st.jumps != 0 {
                return Err(format!("Jump small gaps not set or jumps {}", st.jumps));
            }
            ws_hash_is(s, sc, "ws_h2")
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(4c) Undo: Jump small gaps off", |s, sc| {
            if ws_s(s, sc)?.1.wrap.jump_gaps { Err("Jump small gaps is still on".to_owned()) } else { ws_hash_is(s, sc, "ws_h2") }
        }),
        // (4d)
        Step::Dyn("(4d) S to Project (Final)", |s, sc| ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Project, ..p })),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("(4d) Project: the two Wrap option controls are shown greyed out", |s, _| {
            log::info!("wrap_surface (4d): Wrap option controls {:?} in Project", s.tool_probe.wrap_controls);
            match s.tool_probe.wrap_controls {
                Some(false) => Ok(()),
                other => Err(format!("the Wrap option controls are {other:?} (expected shown and disabled)")),
            }
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("(4d) Undo: Wrap, the controls usable, H2", |s, sc| {
            if s.tool_probe.wrap_controls != Some(true) {
                return Err(format!("the Wrap option controls are {:?} in Wrap", s.tool_probe.wrap_controls));
            }
            ws_hash_is(s, sc, "ws_h2")
        }),
        // (5)
        Step::Dyn("(5) S back to its 30 % size (Final)", |s, sc| {
            let (id, p) = ws_s(s, sc)?;
            let half = unpack(remembered(sc, "ws_half")?);
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, Placement::Projected(Projector { half_size: half, ..p }), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Setup("(5) remember the 30 % state", |s, sc| {
            let ratio = ws_ink_ratio(s, sc)?;
            log::info!("wrap_surface (5): back at 30 %, ink / picture {ratio:.4}, {:?}", ws_stats(s, sc)?);
            remember_composite(s, sc, "ws_front")
        }),
        Step::Dyn("(5) move S onto the UV seam, the picture across it (Final)", ws_to_seam),
        Step::WaitIdle,
        Step::Assert("(5) seam score <= 1.25 x interior + 2 levels", ws_seam_check),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("(5) Undo: the 30 % state", |s, sc| ws_hash_is(s, sc, "ws_front")),
        // (6)
        Step::Setup("(6) remember the anchor, the history and the wrap job counters", |s, sc| {
            let (_, p) = ws_s(s, sc)?;
            t7_store_vec3(sc, ["ws_ox", "ws_oy", "ws_oz"], p.origin);
            let c = s.wrap_jobs.counters;
            sc.values.insert("ws_pv_started", c.previews_started);
            sc.values.insert("ws_pv_applied", c.previews_applied);
            sc.values.insert("ws_pv_stale", c.previews_stale);
            sc.values.insert("ws_ui_params", c.ui_params);
            remember_undo(s, sc)
        }),
        Step::Dyn("(6) drag the body of S by a quarter of the view", |s, _| {
            let (xf, shape) = selected_gizmo(s)?;
            if !shape.wrapped || shape.depth_box.is_some() {
                return Err(format!("the gizmo of the wrapped stamp: wrapped {}, depth box {:?}", shape.wrapped, shape.depth_box));
            }
            Ok(vec![drag(&xf, &[shape.center, shape.center + egui::vec2(xf.rect.width() * 0.25, 0.0)])])
        }),
        Step::WaitIdle,
        Step::Assert("(6) moved >= 0.3 r along the surface, ink 0.93-1.07, one entry, still wrapped", |s, sc| {
            undo_since(s, sc, 1)?;
            let (_, p) = ws_s(s, sc)?;
            let (c, r) = ws_sphere(doc_of(s)?);
            let before = t7_vec3(sc, ["ws_ox", "ws_oy", "ws_oz"])?;
            let moved = r * (before - c).angle_between(p.origin - c);
            let ratio = ws_ink_ratio(s, sc)?;
            log::info!("wrap_surface (6): anchor moved {moved:.4} along the surface ({:.3} r), ink / picture {ratio:.4}", moved / r);
            if p.mode != SurfaceMode::Wrap {
                return Err("S lost its mode".to_owned());
            }
            if moved < 0.3 * r {
                return Err(format!("the anchor moved {moved:.4} (radius {r:.4})"));
            }
            if !(0.93..=1.07).contains(&ratio) {
                return Err(format!("ink / picture area {ratio:.4} after the drag"));
            }
            Ok(())
        }),
        // (6b) Round 8 (plan 4.1, R8-2): in the asynchronous mode the drag's Previews were jobs (no parameterization on
        // the UI thread), at least one landed, none is left; the composite after the Final is logged (equal with
        // `WTE_SYNC_FINAL_BAKES=1`, where no job runs).
        Step::Assert("(6b) the drag's wrapped Previews ran as jobs and landed", ws_drag_jobs),
        // (7)
        Step::Setup("(7) Text tool: \"WRAP AROUND\" in Wrap", |s, _| {
            s.tools.text.content = "WRAP AROUND".to_owned();
            s.tools.text.em_frac = crate::state::TextSettings::default().em_frac;
            s.tools.text.color = [20, 20, 20, 255];
            s.tools.stamp.mode = SurfaceMode::Wrap;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Dyn("(7) place the text below the centre", |s, _| model_click(s, ViewId::Main3D, [-0.1, 0.5])),
        Step::WaitIdle,
        Step::Dyn("(7) Arc warp, bend 30 % (Final)", |s, _| {
            let (id, mut spec) = t7_selected_text(s)?;
            spec.fx.warp = Some(gate_arc(0.30));
            Ok(vec![Step::Do(AppAction::UpdateStampSource(id, StampSource::Text(spec), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("(7) the wrapped text: ink > 0, covered >= 0.999", |s, _| {
            let (id, spec) = t7_selected_text(s)?;
            let doc = doc_of(s)?;
            let Some(Placement::Projected(p)) = doc.stamp(id).map(|st| st.placement) else { return Err("the text is not 3D".to_owned()) };
            let st = doc.stamp_wrap_stats(id).ok_or("the text has no wrap statistics")?;
            let ink = doc.stamp_ink_area(id);
            log::info!("wrap_surface (7): text mode {:?}, warp {:?}, ink area {ink:.5}, {st:?}", p.mode, spec.fx.warp.map(|w| w.bend));
            ws_holes(doc, id, &p, "(7) text");
            log::info!("wrap_surface (7): side panel {:?} pt", s.side_panel_rect.map(|r| r.width()));
            if p.mode != SurfaceMode::Wrap || spec.fx.warp.is_none() {
                return Err("the text is not wrapped or has no warp".to_owned());
            }
            if ink.is_nan() || ink <= 0.0 || st.covered < WS_TEXT_COVERED {
                return Err(format!("ink area {ink}, covered {:.5}", st.covered));
            }
            Ok(())
        }),
        Step::WaitFrames(2),
        Step::Shot("text"),
        // (8)
        Step::Setup("(8) remember the composite before the toggle", |s, sc| remember_composite(s, sc, "ws_h8")),
        Step::BakeWait(false),
        Step::Dyn("(8) toggle S to Project (Final), Undo in the next frame", |s, sc| {
            let mut steps = ws_edit(s, sc, |p| Projector { mode: SurfaceMode::Project, ..p })?;
            steps.push(Step::Do(AppAction::Undo));
            Ok(steps)
        }),
        Step::WaitIdle,
        Step::BakeWait(true),
        Step::Assert("(8) the composite before the toggle", |s, sc| {
            if ws_s(s, sc)?.1.mode != SurfaceMode::Wrap {
                return Err("S is not wrapped after the undo".to_owned());
            }
            ws_hash_is(s, sc, "ws_h8")
        }),
        // (9)
        Step::Dyn("(9) S: Overlap and Jump small gaps (Final)", |s, sc| {
            ws_edit(s, sc, |p| Projector { wrap: WrapOptions { jump_gaps: true, overlap: true }, ..p })
        }),
        Step::WaitIdle,
        Step::Dyn("(9) select S", |_, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(StampId(remembered(sc, "ws_id")?))))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("(9) remember the composite", |s, sc| {
            log::info!("wrap_surface (9): S selected before the save, side panel {:?} pt", s.side_panel_rect.map(|r| r.width()));
            remember_composite(s, sc, "ws_saved")
        }),
        Step::Shot("before_save"),
        Step::Dyn("(9) Save As", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, WS_PROJECT))))])),
        Step::WaitIdle,
        Step::Assert("(9) saved", |s, sc| {
            let doc = doc_of(s)?;
            if doc.is_modified() || doc.has_unbaked_stamps() {
                return Err("the document is modified or has unbaked stamps after the save".to_owned());
            }
            file_non_empty(&with_suffix(&sc.out_stem, WS_PROJECT))
        }),
        Step::Dyn("(9) reopen the project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, WS_PROJECT))))])),
        Step::WaitIdle,
        Step::Assert("(9) reopened: the same composite, both wrapped, S with both options, manifest version 3", ws_reopened),
        Step::WaitFrames(3),
        Step::Assert("(9) log", |s, _| {
            log::info!("wrap_surface (9): reopened, nothing selected, side panel {:?} pt", s.side_panel_rect.map(|r| r.width()));
            Ok(())
        }),
    ]);
    match path {
        Some(path) => steps.extend(ws_path_steps(path)),
        // The screenshot: S selected in the reopened project (its wrapped outline, the stats line).
        None => steps.extend([
            Step::Dyn("select the text, then S for the screenshot", |s, sc| {
                let (text, _) = text_stamp(s)?;
                Ok(vec![
                    Step::Do(AppAction::SetTool(ToolKind::Transform)),
                    Step::Do(AppAction::SelectStamp(Some(text))),
                    Step::WaitIdle,
                    Step::WaitFrames(3),
                    Step::Assert("log", |s, _| {
                        log::info!("wrap_surface (end): the text selected, side panel {:?} pt", s.side_panel_rect.map(|r| r.width()));
                        Ok(())
                    }),
                    Step::Do(AppAction::SelectStamp(Some(StampId(remembered(sc, "ws_id")?)))),
                ])
            }),
            Step::WaitIdle,
            Step::WaitFrames(3),
            Step::Assert("S is selected with its stats line", |s, sc| {
                let (id, _) = ws_s(s, sc)?;
                if session_of(s)?.selected_stamp != Some(id) {
                    return Err("S is not selected".to_owned());
                }
                let st = ws_stats(s, sc)?;
                log::info!(
                    "wrap_surface (end): S reopened and selected: {st:?}; controls {:?}; side panel {:?} pt",
                    s.tool_probe.wrap_controls,
                    s.side_panel_rect.map(|r| r.width())
                );
                Ok(())
            }),
            // The Color section is collapsed, so the screenshot shows the whole Transform options of S.
            Step::Dyn("collapse the Color section", |s, _| {
                let r = panel_rect(s, PanelHit::ColorSection)?;
                Ok(vec![Step::ScreenPointer { path: vec![r.min + egui::vec2(24.0, 9.0)] }])
            }),
            Step::WaitIdle,
            Step::WaitFrames(3),
        ]),
    }
    steps
}

/// Step 10 of `wrap_surface`: the PATH model, the `cells` picture at 40 % in Wrap near the centre of the view, then
/// with Jump small gaps.
fn ws_path_steps(path: PathBuf) -> Vec<Step> {
    let mut steps = vec![Step::Do(AppAction::OpenPath(path))];
    steps.extend(open_and_accept(true));
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Setup("(10) log the model", |s, _| {
            log_document(doc_of(s)?);
            Ok(())
        }),
        Step::Setup("(10) register the cells picture; Stamp tool 40 % in Wrap", |s, sc| {
            ws_setup_cells(s, sc)?;
            s.tools.stamp.size_frac = 0.40;
            s.tools.stamp.rotation_deg = 0.0;
            s.tools.stamp.mode = SurfaceMode::Wrap;
            s.tools.stamp.wrap = WrapOptions::default();
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("(10) place the picture at the centre of the view (or the nearest good spot)", |s, _| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let doc = doc_of(s)?;
            let centre = xf.rect.center();
            let at = match xf.surface_at(doc, TextureSetId(0), centre) {
                Surface::Model(hit) if hit.texture_set.is_some() => centre,
                _ => {
                    let (_, first, _) = stamp_spots(s)?;
                    log::info!("wrap_surface (10): the centre of the view misses the model; placing at {:?}", first.pos);
                    first.pos
                }
            };
            Ok(vec![click(ViewId::Main3D, norm(&xf, at))])
        }),
        Step::WaitIdle,
        Step::Setup("(10) log the wrap (Jump small gaps off)", |s, sc| ws_log_path_stamp(s, sc, "off")),
        // Round 9 (package B, plan 6 step 6): the picture with Jump small gaps off (`<out>_off.png`).
        Step::WaitFrames(3),
        Step::Shot("off"),
        Step::Dyn("(10) Jump small gaps on (Final)", |s, sc| {
            ws_edit(s, sc, |p| Projector { wrap: WrapOptions { jump_gaps: true, ..p.wrap }, ..p })
        }),
        Step::WaitIdle,
        Step::Setup("(10) log the wrap (Jump small gaps on)", |s, sc| ws_log_path_stamp(s, sc, "on")),
        // Round 8: the Color section collapsed, so the screenshot shows the stamp's whole Transform options with the
        // stats lines ("Wrapped over N triangles", "Fitted to the curved surface ...", "Cut where ...").
        Step::Dyn("(10) collapse the Color section", |s, _| {
            let r = panel_rect(s, PanelHit::ColorSection)?;
            Ok(vec![Step::ScreenPointer { path: vec![r.min + egui::vec2(24.0, 9.0)] }])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        // Round 9 (package B, plan 7.2): the third log state (the panel's lines: shared texels, the part of a picture
        // longer than the way round). Stable (fix-wrap, item 2): logged before the asserts, so a model that fails
        // one of them (nonmanifold, noisy400, tiles300) still logs what its panel says.
        Step::Setup("(10) log the panel lines (Jump small gaps on)", ws_log_panel_lines),
        Step::Assert("(10) ink <= 1.3, dup <= 3 %, scatter <= 5 % with Jump small gaps off and on", |_, sc| {
            let mut bad = Vec::new();
            for (jump, keys) in [("off", WS_Q_OFF), ("on", WS_Q_ON)] {
                let [ink, dup, scatter] = keys.map(|k| remembered(sc, k).map(|v| f32::from_bits(v as u32)));
                let (ink, dup, scatter) = (ink?, dup?, scatter?);
                if ink.is_nan() || ink > 1.3 || dup.is_nan() || dup > 0.03 || scatter.is_nan() || scatter > 0.05 {
                    bad.push(format!("Jump {jump}: ink {ink:.4}, dup {:.3} %, scatter {:.3} %", dup * 100.0, scatter * 100.0));
                }
            }
            if bad.is_empty() { Ok(()) } else { Err(bad.join("; ")) }
        }),
        // Round 9 (package B, plan 7.2): the new asserts.
        // Stable (w2, item 2): the coverage is asked of the room the surface has: `room` (the largest extent across
        // the picture of the parts it lies on, over the picture's height; 1 for a part as large as the picture). A
        // pawn under a picture as wide as the board can hold only `room` of its height (ABeautifulGame Jump off,
        // many_islands' small islands).
        Step::Assert("(10) covered >= 0.85 x ends_share x room unless reached_edge, lost_cut <= 10 % (15 % when wraps_closed), with Jump small gaps off and on", |_, sc| {
            let mut bad = Vec::new();
            for (jump, keys) in [("off", WS_S_OFF), ("on", WS_S_ON)] {
                let [covered, ends, edge, lost, room, closed] = keys.map(|k| remembered(sc, k).map(|v| f32::from_bits(v as u32)));
                let (covered, ends, edge, lost, room, closed) = (covered?, ends?, edge? != 0.0, lost?, room?, closed? != 0.0);
                if !edge && (covered.is_nan() || room.is_nan() || covered < 0.85 * ends * room) {
                    bad.push(format!("Jump {jump}: covered {covered:.4} < 0.85 x ends_share {ends:.4} x room {room:.4}"));
                }
                // Stable (w2, item 2): a picture covering a whole closed part (the Lantern's 52-vertex base block)
                // loses the part's total curvature to the gaps where it is cut open at the corners. Stable
                // (fix_wrap_1): bounded all the same, at 15 % (the Lantern 12.11 %; round 8's cone rule there
                // 14.7 %; w2 left closed parts unbounded).
                let limit = if closed { 0.15 } else { 0.10 };
                if lost.is_nan() || lost > limit {
                    bad.push(format!("Jump {jump}: lost_cut {:.2} % > {:.0} %", lost * 100.0, limit * 100.0));
                }
            }
            if bad.is_empty() { Ok(()) } else { Err(bad.join("; ")) }
        }),
    ]);
    steps
}

/// Round 9 (package B): scratch keys of step 10's coverage measures (covered, ends_share, reached_edge as 0 / 1,
/// lost_cut, stable w2: room and wraps_closed as 0 / 1; `f32` bits) with Jump small gaps off and on.
const WS_S_OFF: [&str; 6] = ["ws_cov_off", "ws_ends_off", "ws_edge_off", "ws_lost_off", "ws_room_off", "ws_closed_off"];
const WS_S_ON: [&str; 6] = ["ws_cov_on", "ws_ends_on", "ws_edge_on", "ws_lost_on", "ws_room_on", "ws_closed_on"];

/// Round 9 (package B, plan 7.2): logs what the panel says under stamp S besides the stats line: the share of its
/// picture on shared texels (`Document::stamp_shared_texels`, the T9 line from 5 %; stable fix-wrap: "under the
/// picture", as against the whole model's share the warm-up logs as "shared texels found ... (N % of the surface)")
/// and the part of a picture longer than the way round (`ends_share`).
fn ws_log_panel_lines(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let (id, _) = ws_s(state, sc)?;
    let doc = doc_of(state)?;
    let share = doc.stamp_shared_texels(id);
    let stats = doc.stamp_wrap_stats(id);
    log::info!(
        "wrap_surface (10) panel lines: shared texels under the picture {} ({}), ends_share {} ({}), passed {}, lost_cut {}, gaps {}, at a neighbouring part {}",
        share.map_or("-".to_owned(), |v| format!("{:.1} %", v * 100.0)),
        if share.is_some_and(|v| v >= wte_core::uv_share::UV_SHARE_WARN) { "line shown" } else { "no line" },
        stats.map_or("-".to_owned(), |s| format!("{:.1} %", s.ends_share * 100.0)),
        // Stable (fix_wrap_1): the panel's rule (a whole picture on the surface has no ends line).
        if stats.is_some_and(|s| crate::panels::tool_options::ends_cut_shown(&s)) { "line shown" } else { "no line" },
        stats.is_some_and(|s| s.passed),
        stats.map_or("-".to_owned(), |s| format!("{:.2} %", s.lost_cut * 100.0)),
        stats.map_or("-".to_owned(), |s| format!("{:.2} %", s.gaps * 100.0)),
        stats.map_or("-".to_owned(), |s| format!("{:.2} %", s.at_part * 100.0)),
    );
    // Stable (w1): the lines the panel shows under "Wrapped over N triangles".
    if let (Some(s), Some(Placement::Projected(p))) = (stats, doc.stamp(id).map(|st| st.placement)) {
        log::info!("wrap_surface (10) panel wrap lines: {:?}", crate::panels::tool_options::wrap_stats_line_texts(&s, p.wrap));
    }
    Ok(())
}

/// Scratch keys of step 10's measures (ink / picture, dup, scatter as `f32` bits) with Jump small gaps off and on.
const WS_Q_OFF: [&str; 3] = ["ws_ink_off", "ws_dup_off", "ws_scatter_off"];
const WS_Q_ON: [&str; 3] = ["ws_ink_on", "ws_dup_on", "ws_scatter_on"];

/// Registers the 512 x 256 `cells` picture as a blob and makes it the Stamp tool's image.
fn ws_setup_cells(state: &mut AppState, _: &mut Scratch) -> Result<(), String> {
    let png = png_of(&samples::cells_picture(WS_PICTURE[0], WS_PICTURE[1]))?;
    let doc = state.doc_mut().ok_or("no session")?;
    let blob = doc.add_image_blob("cells.png", Arc::from(png)).map_err(|e| e.to_string())?;
    state.tools.stamp.source = Some(blob);
    state.tools.stamp.source_name = "cells".to_owned();
    Ok(())
}

/// Stamp S of `wrap_surface` (`ws_id`) and its projector.
fn ws_s(state: &AppState, sc: &Scratch) -> Result<(StampId, Projector), String> {
    let id = StampId(remembered(sc, "ws_id")?);
    match doc_of(state)?.stamp(id).ok_or("stamp S is gone")?.placement {
        Placement::Projected(p) => Ok((id, p)),
        Placement::Uv(_) => Err("stamp S is a 2D stamp".to_owned()),
    }
}

/// A Final edit of S's projector.
fn ws_edit(state: &AppState, sc: &Scratch, edit: fn(Projector) -> Projector) -> Result<Vec<Step>, String> {
    let (id, p) = ws_s(state, sc)?;
    Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, Placement::Projected(edit(p)), Commit::Final))])
}

/// The composite equals the one remembered as `key` (both logged).
fn ws_hash_is(state: &AppState, sc: &Scratch, key: &str) -> Result<(), String> {
    let (now, want) = (composite_hash(doc_of(state)?), remembered(sc, key)?);
    let panel = state.side_panel_rect.map(|r| r.width());
    log::info!("wrap_surface: composite {now:016x}, \"{key}\" {want:016x} (tool {:?}, side panel {panel:?} pt)", state.tool);
    if now == want { Ok(()) } else { Err(format!("the composite differs from \"{key}\"")) }
}

/// S's ink area over the picture's opaque area (the `cells` picture is opaque: the whole placement rectangle).
fn ws_ink_ratio(state: &AppState, sc: &Scratch) -> Result<f32, String> {
    let (id, p) = ws_s(state, sc)?;
    let area = 4.0 * p.half_size.x.abs() * p.half_size.y.abs();
    if area.is_nan() || area <= 0.0 {
        return Err(format!("S has no area ({:?})", p.half_size));
    }
    Ok(doc_of(state)?.stamp_ink_area(id) / area)
}

/// S's wrap statistics (from the cached parameterization).
fn ws_stats(state: &AppState, sc: &Scratch) -> Result<WrapStats, String> {
    let (id, _) = ws_s(state, sc)?;
    doc_of(state)?.stamp_wrap_stats(id).ok_or_else(|| "S has no wrap statistics (no parameterization cached)".to_owned())
}

/// S's wrap quality (round 8, `Document::stamp_wrap_quality`: from its baked texels and cached parameterization).
fn ws_quality(state: &AppState, sc: &Scratch) -> Result<wte_core::WrapQuality, String> {
    let (id, _) = ws_s(state, sc)?;
    doc_of(state)?.stamp_wrap_quality(id).ok_or_else(|| "S has no wrap quality (no parameterization cached)".to_owned())
}

/// Step 6b: the wrap job counters since step 6's Setup. Asynchronous mode: the drag started Preview jobs, at least one
/// landed, no parameterization was made on the UI thread by the app, the document defers wrapped Previews and nothing
/// is left in flight or waiting. Synchronous mode (`WTE_SYNC_FINAL_BAKES=1`): no job was used. The composite is logged
/// in both modes (the gate compares them).
fn ws_drag_jobs(state: &AppState, sc: &Scratch) -> Result<(), String> {
    let c = state.wrap_jobs.counters;
    let since = |now: u64, key: &str| remembered(sc, key).map(|was| now.saturating_sub(was));
    let started = since(c.previews_started, "ws_pv_started")?;
    let applied = since(c.previews_applied, "ws_pv_applied")?;
    let stale = since(c.previews_stale, "ws_pv_stale")?;
    let ui = since(c.ui_params, "ws_ui_params")?;
    let doc = doc_of(state)?;
    log::info!(
        "wrap_surface (6b): {} mode, Preview jobs started {started}, applied {applied}, dropped {stale}; parameterizations on the UI thread by the app {ui}; composite after the Final {:016x}",
        if state.async_final_bakes { "asynchronous" } else { "synchronous" },
        composite_hash(doc)
    );
    if !state.async_final_bakes {
        return if doc.defers_wrap_previews() || started != 0 {
            Err(format!("the synchronous mode used Preview jobs ({started}) or defers them"))
        } else {
            Ok(())
        };
    }
    if !doc.defers_wrap_previews() {
        return Err("the document does not defer wrapped Previews in the asynchronous mode".to_owned());
    }
    if started == 0 || applied == 0 {
        return Err(format!("the drag started {started} Preview jobs, {applied} landed"));
    }
    if ui != 0 {
        return Err(format!("{ui} parameterizations were made on the UI thread"));
    }
    if state.wrap_jobs.in_flight().is_some() || doc.has_stale_wrap_previews() {
        return Err("a wrap job is still in flight or a Preview is waiting after the drag".to_owned());
    }
    Ok(())
}

/// The sphere sample's centre and radius (half of the smallest bounds extent).
fn ws_sphere(doc: &Document) -> (Vec3, f32) {
    let b = doc.mesh.bounds;
    (b.center(), (b.max - b.min).min_element() * 0.5)
}

/// The layer of stamp `id` in set 0.
fn ws_layer(doc: &Document, id: StampId) -> Result<&Layer, String> {
    let (_, layer) = doc.stamp_layers(id).into_iter().find(|(set, _)| set.0 == 0).ok_or("the stamp has no layer in set 0")?;
    doc.layer(layer).ok_or_else(|| "the stamp's layer is gone".to_owned())
}

/// True for a colour that is no cell colour of the `cells` picture and no mix of two neighbouring cells (within 12
/// levels plus the cell gradient): a smear (the measure of the core tests, plan 4.7).
fn ws_is_smear(px: [u8; 4]) -> bool {
    let colors = samples::cells_colors();
    let tol = 12.0 + samples::CELLS_GRADIENT as f32;
    let c = Vec3::new(px[0] as f32, px[1] as f32, px[2] as f32);
    let cell = |i: usize| Vec3::new(colors[i][0] as f32, colors[i][1] as f32, colors[i][2] as f32);
    let off = |a: Vec3, b: Vec3| {
        let ab = b - a;
        let t = if ab.length_squared() > 0.0 { ((c - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (c - (a + ab * t)).abs().max_element()
    };
    for i in 0..32 {
        if off(cell(i), cell(i)) <= tol {
            return false;
        }
        let (cx, cy) = (i % 8, i / 8);
        for j in [(cx + 1 < 8).then_some(i + 1), (cy + 1 < 4).then_some(i + 8)].into_iter().flatten() {
            if off(cell(i), cell(j)) <= tol {
                return false;
            }
        }
    }
    true
}

/// The column (0..8) of the `cells` picture whose colour is nearest to `px`.
fn ws_cell_column(px: [u8; 4]) -> usize {
    let colors = samples::cells_colors();
    let dist = |c: [u8; 3]| (0..3).map(|k| px[k].abs_diff(c[k])).max().unwrap_or(0);
    (0..32).min_by_key(|&i| dist(colors[i])).unwrap_or(0) % 8
}

/// What `ws_band` counts in set 0 for S: the texels whose surface point lies within 0.9 hh of the great circle of
/// S's spine (through its anchor along its right axis) and how many of them S inks; the opaque ones and the smeared
/// ones among them; and in the two overlap stretches of a band 1.2 x the way round (|angle from the anchor| 0.83 pi to
/// 0.97 pi, inside the 0.8 pi .. pi where two turns lie), the opaque texels and those showing the picture's right half.
struct WsBand {
    texels: u64,
    inked: u64,
    opaque: u64,
    smear: u64,
    overlap_opaque: u64,
    right: u64,
}

fn ws_band(state: &AppState, sc: &Scratch) -> Result<WsBand, String> {
    let (id, p) = ws_s(state, sc)?;
    let doc = doc_of(state)?;
    let (c, r) = ws_sphere(doc);
    let layer = ws_layer(doc, id)?;
    let set = doc.sets.first().ok_or("no texture set")?;
    let (right, _, _) = p.axes();
    let a0 = (p.origin - c).normalize();
    let e2 = (right - a0 * right.dot(a0)).normalize();
    let n = a0.cross(e2);
    let limit = 0.9 * p.half_size.y.abs();
    let mut out = WsBand { texels: 0, inked: 0, opaque: 0, smear: 0, overlap_opaque: 0, right: 0 };
    for y in 0..set.height {
        for x in 0..set.width {
            let Some((_, false)) = set.texel_map.owner(x, y) else { continue };
            let Some(sp) = set.texel_map.locate(&doc.mesh, x, y) else { continue };
            let q = (sp.position - c).normalize();
            if r * q.dot(n).clamp(-1.0, 1.0).asin().abs() > limit {
                continue;
            }
            out.texels += 1;
            let px = layer.get(x as i32, y as i32);
            if px[3] == 0 {
                continue;
            }
            out.inked += 1;
            if px[3] < 255 {
                continue;
            }
            out.opaque += 1;
            out.smear += u64::from(ws_is_smear(px));
            let angle = q.dot(e2).atan2(q.dot(a0)).abs() / std::f32::consts::PI;
            if (0.83..=0.97).contains(&angle) {
                out.overlap_opaque += 1;
                out.right += u64::from(ws_cell_column(px) >= 4);
            }
        }
    }
    Ok(out)
}

/// Step 5: S at its 30 % size, wrapped, anchored on the sphere's UV seam at the middle row of the texture (the
/// meridian where texel columns 0 and width-1 meet), its long axis horizontal across the seam.
fn ws_to_seam(state: &AppState, sc: &Scratch) -> Result<Vec<Step>, String> {
    let (id, p) = ws_s(state, sc)?;
    let doc = doc_of(state)?;
    let set = doc.sets.first().ok_or("no texture set")?;
    let row = set.height / 2;
    let a = set.texel_map.locate(&doc.mesh, 0, row).ok_or("texel column 0 has no surface")?;
    let b = set.texel_map.locate(&doc.mesh, set.width - 1, row).ok_or("the last texel column has no surface")?;
    log::info!("wrap_surface (5): seam at {:?} / {:?} ({:.5} apart)", a.position, b.position, a.position.distance(b.position));
    let half = unpack(remembered(sc, "ws_half")?);
    // The picture's centre is a cell border of the `cells` picture (8 columns): the anchor is turned half a cell
    // round the sphere's axis, so that the seam runs through the middle of a cell (as the core's WA6 does).
    let (c, r) = ws_sphere(doc);
    let turn = glam::Quat::from_rotation_y(half.x.abs() / 8.0 / r.max(1e-6));
    let (pos, normal) = (c + turn * (a.position - c), turn * a.normal);
    let at = Projector::from_surface(pos, normal, Vec3::Y, half, half.max_element());
    let fitted = doc.fit_stamp_depth(&at);
    let seam = Projector { mode: SurfaceMode::Wrap, wrap: p.wrap, ..fitted };
    Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, Placement::Projected(seam), Commit::Final))])
}

/// Step 5 (WA6 on the composite): the mean largest channel difference of the texel pairs on the two sides of the seam
/// (columns 0 and width-1 of a row, both opaque in S, their surface points within 1.25 local texel widths) is at most
/// 1.25 x the same over horizontally adjacent texels of S away from the seam + 2 levels.
fn ws_seam_check(state: &AppState, sc: &Scratch) -> Result<(), String> {
    let (id, _) = ws_s(state, sc)?;
    let doc = doc_of(state)?;
    let layer = ws_layer(doc, id)?;
    let set = doc.sets.first().ok_or("no texture set")?;
    let comp = set.composite();
    let (w, h) = (set.width, set.height);
    let opaque = |x: u32, y: u32| layer.get(x as i32, y as i32)[3] == 255;
    let diff = |a: [u8; 4], b: [u8; 4]| (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0) as f64;
    let (mut seam, mut seam_n) = (0.0f64, 0u64);
    let (mut inner, mut inner_n) = (0.0f64, 0u64);
    for y in 0..h {
        if opaque(0, y) && opaque(w - 1, y) {
            let (Some(a), Some(b), Some(next)) = (
                set.texel_map.locate(&doc.mesh, 0, y),
                set.texel_map.locate(&doc.mesh, w - 1, y),
                set.texel_map.locate(&doc.mesh, 1, y),
            ) else {
                continue;
            };
            if a.position.distance(b.position) <= 1.25 * a.position.distance(next.position) {
                seam += diff(comp.get(0, y), comp.get(w - 1, y));
                seam_n += 1;
            }
        }
        for x in 2..w.saturating_sub(3) {
            if opaque(x, y) && opaque(x + 1, y) {
                inner += diff(comp.get(x, y), comp.get(x + 1, y));
                inner_n += 1;
            }
        }
    }
    let (seam_mean, inner_mean) = (seam / seam_n.max(1) as f64, inner / inner_n.max(1) as f64);
    let limit = 1.25 * inner_mean + 2.0;
    log::info!(
        "wrap_surface (5) seam: {seam_n} pairs, mean {seam_mean:.3}; interior {inner_n} pairs, mean {inner_mean:.3}; limit {limit:.3}; {:?}",
        ws_stats(state, sc)?
    );
    if seam_n < 20 {
        return Err(format!("only {seam_n} texel pairs across the seam"));
    }
    if seam_mean > limit {
        return Err(format!("seam score {seam_mean:.3} > {limit:.3}"));
    }
    Ok(())
}

/// Step 9: the reopened project equals the saved state.
fn ws_reopened(state: &AppState, sc: &Scratch) -> Result<(), String> {
    let path = with_suffix(&sc.out_stem, WS_PROJECT);
    let session = session_of(state)?;
    if session.path.as_deref() != Some(path.as_path()) {
        return Err(format!("the open document is {:?}", session.path));
    }
    ws_hash_is(state, sc, "ws_saved")?;
    let doc = &session.doc;
    let s_id = StampId(remembered(sc, "ws_id")?);
    if doc.stamps.len() != 2 {
        return Err(format!("{} stamps after the reopen", doc.stamps.len()));
    }
    for st in &doc.stamps {
        let Placement::Projected(p) = st.placement else { return Err(format!("{:?} is not a 3D stamp", st.name)) };
        let want = if st.id == s_id { WrapOptions { jump_gaps: true, overlap: true } } else { WrapOptions::default() };
        if p.mode != SurfaceMode::Wrap || p.wrap != want {
            return Err(format!("{:?}: mode {:?}, options {:?} (expected Wrap, {want:?})", st.name, p.mode, p.wrap));
        }
    }
    let manifest = wte_core::project::read_manifest(&path).map_err(|e| e.to_string())?;
    log::info!("wrap_surface (9): manifest min_reader_version {}", manifest.min_reader_version);
    if manifest.min_reader_version != 3 {
        return Err(format!("min_reader_version {}", manifest.min_reader_version));
    }
    for entry in &manifest.stamps {
        let wte_core::project::PlacementEntry::Projected { mode, wrap_gaps, wrap_overlap, .. } = &entry.placement else {
            return Err(format!("{:?} is saved as a 2D stamp", entry.name));
        };
        let on_s = entry.id == s_id;
        log::info!("wrap_surface (9): saved {:?}: mode {mode:?}, wrap_gaps {wrap_gaps}, wrap_overlap {wrap_overlap}", entry.name);
        if mode != "wrap" || *wrap_gaps != on_s || *wrap_overlap != on_s {
            return Err(format!("{:?} is saved with mode {mode:?}, wrap_gaps {wrap_gaps}, wrap_overlap {wrap_overlap}", entry.name));
        }
    }
    Ok(())
}

/// Step 10: logs the wrap of the stamp placed on the PATH model (its statistics, round 8: its `WrapQuality`, the ink
/// ratio, the parameterization's time and the Final's "applied after" time; the core's `wrap:` and the
/// `final bake:` lines have the rest), asserts ink on the model and keeps ink / picture, dup and scatter for the
/// step's last assert (`WS_Q_OFF` / `WS_Q_ON`; logged first for both, so that a failure with Jump off still logs Jump on).
fn ws_log_path_stamp(state: &mut AppState, sc: &mut Scratch, jump: &str) -> Result<(), String> {
    let (id, placement) = placed_stamp(state, false)?;
    sc.values.insert("ws_id", id.0);
    let Placement::Projected(p) = placement else { return Err("the stamp is not 3D".to_owned()) };
    let doc = doc_of(state)?;
    let ink = doc.stamp_ink_area(id);
    let area = 4.0 * p.half_size.x.abs() * p.half_size.y.abs();
    let ratio = ink / area.max(f32::MIN_POSITIVE);
    let stats = doc.stamp_wrap_stats(id);
    let quality = doc.stamp_wrap_quality(id);
    let applied = state.final_bakes.last_applied().filter(|(s, _)| *s == id).map(|(_, ms)| ms);
    log::info!(
        "wrap_surface (10) Jump small gaps {jump}: mode {:?}, options {:?}, ink area {ink:.6} ({ratio:.4} of the picture), stats {stats:?}",
        p.mode,
        p.wrap,
    );
    // Stable (fix-wrap, item 4): the composites' hash (every texture set, the wrapped stamp baked in), for
    // byte-equality checks across runs (`WTE_SYNC_FINAL_BAKES=1`, `RAYON_NUM_THREADS=1` against the default).
    log::info!("wrap_surface (10) Jump small gaps {jump} composite hash {:016x}", composite_hash(doc));
    log::info!(
        "wrap_surface (10) Jump small gaps {jump} quality: ink / picture {ratio:.4} (plan <= 1.3), dup {} (plan <= 3 %), scatter {} (plan <= 5 %), bins {}; param {} ms, relaxed {}, seams {} {:?}, stretch {}, rejected {}, jumps {}; Final applied after {} ms",
        quality.map_or("-".to_owned(), |q| format!("{:.3} %", q.dup * 100.0)),
        quality.map_or("-".to_owned(), |q| format!("{:.3} %", q.scatter * 100.0)),
        quality.map_or(0, |q| q.bins),
        stats.map_or("-".to_owned(), |s| format!("{:.1}", s.micros as f64 / 1e3)),
        stats.is_some_and(|s| s.relaxed),
        stats.map_or(0, |s| s.seams),
        stats.map(|s| s.seams_by_rule),
        stats.map_or("-".to_owned(), |s| format!("{:.1} %", s.stretch * 100.0)),
        stats.map_or("-".to_owned(), |s| format!("{:.2} %", s.rejected * 100.0)),
        stats.map_or(0, |s| s.jumps),
        applied.map_or("-".to_owned(), |ms| format!("{ms:.0}")),
    );
    // check:B (round 9): the placement as bits, so a core probe can rebuild this parameterization exactly.
    log::info!(
        "wrap_surface (10) Jump small gaps {jump} placement bits: origin {:?} rotation {:?} half {:?} flip {} {}",
        p.origin.to_array().map(f32::to_bits),
        p.rotation.to_array().map(f32::to_bits),
        p.half_size.to_array().map(f32::to_bits),
        p.flip_x,
        p.flip_y,
    );
    let keys = if jump == "on" { WS_Q_ON } else { WS_Q_OFF };
    let (dup, scatter) = quality.map_or((f32::NAN, f32::NAN), |q| (q.dup, q.scatter));
    for (key, v) in keys.into_iter().zip([ratio, dup, scatter]) {
        sc.values.insert(key, u64::from(v.to_bits()));
    }
    // Round 9 (package B): the coverage measures of plan 7.2.
    let keys = if jump == "on" { WS_S_ON } else { WS_S_OFF };
    let measures = stats.map_or([f32::NAN; 6], |s| {
        [s.covered, s.ends_share, if s.reached_edge { 1.0 } else { 0.0 }, s.lost_cut, s.room, if s.wraps_closed { 1.0 } else { 0.0 }]
    });
    let (shown, beyond, gaps, at_part) = stats.map_or((f32::NAN, f32::NAN, f32::NAN, f32::NAN), |s| (s.shown, s.beyond, s.gaps, s.at_part));
    for (key, v) in keys.into_iter().zip(measures) {
        sc.values.insert(key, u64::from(v.to_bits()));
    }
    log::info!(
        "wrap_surface (10) Jump small gaps {jump} coverage: covered {:.4}, ends_share {:.4}, reached_edge {}, lost_cut {:.2} %, passed {}, room {:.4}, wraps_closed {}, shown {:.4}, beyond {:.4}, gaps {:.4}, at_part {:.4}",
        measures[0],
        measures[1],
        measures[2] != 0.0,
        measures[3] * 100.0,
        stats.is_some_and(|s| s.passed),
        measures[4],
        measures[5] != 0.0,
        shown,
        beyond,
        gaps,
        at_part
    );
    if p.mode != SurfaceMode::Wrap {
        return Err("the stamp is not wrapped".to_owned());
    }
    if ink.is_nan() || ink <= 0.0 {
        return Err("the wrapped stamp left no ink on the model".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_scenario_exists_and_ends_with_a_capture() {
        let args = CliArgs { path: None, screenshot: None, scenario: String::new(), size: [1280, 800], log: None };
        for name in scenario_names() {
            let steps = scenario(name, &args).unwrap_or_else(|| panic!("scenario {name} is listed but missing"));
            assert!(steps.iter().any(|s| matches!(s, Step::Screenshot)), "{name} takes no screenshot");
            assert!(matches!(steps.last(), Some(Step::Exit)), "{name} does not end with Exit");
        }
        assert!(scenario("no such scenario", &args).is_none());
        assert_eq!(scenario_names().len(), 45);
    }

    #[test]
    fn the_brush_tab_budget_applies_from_1280_by_800() {
        // Gate 1: `brush_tab` at 900 x 600 failed on the A12 budget, which is defined for 1280 x 800. The budget still
        // holds at 1280 x 800; in a smaller window only the size-independent rules are checked.
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        // The 900 x 600 frame of the failed run: the tool options reach below the tool area.
        state.side_panel_rect = Some(r(623.0, 25.0, 900.0, 575.0));
        state.side_rects.tool_area = r(609.0, 27.0, 886.0, 139.5);
        state.side_rects.tool_options = r(609.0, 27.0, 838.5, 167.0);
        state.side_rects.color = r(632.0, 140.0, 892.0, 330.0);
        state.side_rects.bottom_area = r(632.0, 340.0, 892.0, 570.0);
        state.side_rects.brush_tab = Some(r(632.0, 370.0, 892.0, 570.0));
        state.side_rects.brush_tab_content = Some(r(632.0, 370.0, 892.0, 610.0));
        assert!(brush_tab_layout(&state, "small").is_ok());
        state.side_rects.color = r(632.0, 140.0, 920.0, 330.0);
        assert!(brush_tab_layout(&state, "small").is_err(), "Color outside the panel fails at every size");
        // The same overflow at 1280 x 800 fails as before.
        state.side_panel_rect = Some(r(980.0, 25.0, 1280.0, 775.0));
        state.side_rects.tool_area = r(989.0, 27.0, 1266.0, 139.5);
        state.side_rects.tool_options = r(989.0, 27.0, 1218.5, 167.0);
        state.side_rects.color = r(989.0, 175.0, 1272.0, 493.5);
        state.side_rects.bottom_area = r(989.0, 500.0, 1272.0, 770.0);
        state.side_rects.brush_tab = Some(r(989.0, 525.0, 1272.0, 770.0));
        state.side_rects.brush_tab_content = Some(r(989.0, 525.0, 1272.0, 760.0));
        assert!(brush_tab_layout(&state, "1280").unwrap_err().contains("brush sliders"));
        state.side_rects.tool_area = r(989.0, 27.0, 1266.0, 175.0);
        assert!(brush_tab_layout(&state, "1280").is_ok());
    }

    #[test]
    fn open_path_without_a_path_fails_clearly() {
        let args = CliArgs { path: None, screenshot: None, scenario: "open_path".into(), size: [1280, 800], log: None };
        let steps = scenario("open_path", &args).unwrap();
        let state = AppState::new(crate::state::Prefs::default(), true);
        let scratch = Scratch { values: Default::default(), out_stem: PathBuf::from("x") };
        let Step::Assert(_, f) = &steps[0] else { panic!("the first step is not an assert") };
        assert!(f(&state, &scratch).unwrap_err().contains("PATH"));
        let with_path = CliArgs { path: Some(PathBuf::from("m.obj")), ..args };
        assert!(matches!(scenario("open_path", &with_path).unwrap()[0], Step::WaitIdle));
    }

    #[test]
    fn densify_keeps_the_ends_and_limits_the_spacing() {
        let pts = densify(&[egui::pos2(0.0, 0.0), egui::pos2(30.0, 0.0), egui::pos2(30.0, 5.0)]);
        assert_eq!(pts.first(), Some(&egui::pos2(0.0, 0.0)));
        assert_eq!(pts.last(), Some(&egui::pos2(30.0, 5.0)));
        assert!(pts.windows(2).all(|w| (w[1] - w[0]).length() <= PATH_STEP + 1e-4));
    }

    #[test]
    fn default_content_asserts_hold_on_a_headless_document() {
        // The layer rule of the default content, checked on a document built like the scenario does it.
        let mut state = crate::tools::test_support::state_with(SampleKind::Sphere);
        let mut scratch = Scratch { values: Default::default(), out_stem: PathBuf::from("x") };
        remember_initial(&mut state, &mut scratch).unwrap();
        assert!(assert_default_layers(state.doc().unwrap()).is_err());
        assert!(assert_default_content(&state, &scratch).is_err());
        assert!(assert_fresh_sphere(&state, &scratch).is_ok());
        assert!(changed_since(&state, &scratch, "initial_composite").is_err());
    }
}
