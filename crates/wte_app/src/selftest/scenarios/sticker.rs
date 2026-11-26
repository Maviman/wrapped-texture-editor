// Stickers P6a (STICKER_PLAN 7 "P6a", 8.2; ARCHITECTURE.md 8.11, 8.15): the scenario `sticker_view` - the Sticker view
// in the real window. The sphere sample with the sample logo placed as an image stamp; the view opened with the key 4
// (the stamp stays an image stamp until the first content edit); a second picture layer added (the promotion and the
// add are one undo step); that layer moved with a pointer drag on the canvas (`Target::StickerCanvas`), its opacity
// lowered, the layers reordered with the Layers tab's Down button; the 3D cache hash changes after each step and each
// step is one undo step (checked by undoing them one by one and redoing them); Delete in the view leaves the stamp;
// Revert and its undo; leaving with Esc back to the 3D view; undoing everything brings the start back. Shots at
// 1280x800 and 900x600, each with what it must show.

use super::*;
use crate::selftest::Target;
use crate::sticker::StickerAction;

/// The second picture: a red disc with a soft edge (it shows clearly on the logo).
const DISC_SIDE: u32 = 96;
/// How far the drag moves the disc layer (layer-space pixels).
const DRAG: [f32; 2] = [40.0, 24.0];

/// The `sticker_view` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_view() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("register the sample stamp image", setup_stamp_image),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.0, -0.05])),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| placed_stamp(s, false).map(|_| ())),
        Step::Setup("remember the start", |s, sc| {
            remember_composite(s, sc, "start")?;
            remember_undo(s, sc)?;
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            Ok(())
        }),
        // (1) Enter with the key 4: nothing in the document changes.
        Step::Key(egui::Modifiers::NONE, egui::Key::Num4),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the Sticker view is open on the image stamp; nothing changed", |s, sc| {
            let edit = edit(s)?;
            if s.view_mode != ViewMode::Sticker || edit.return_mode != ViewMode::View3D {
                return Err(format!("view {:?}, return {:?}", s.view_mode, edit.return_mode));
            }
            if edit.active_layer != Some(wte_core::StickerLayerId(1)) || edit.tool != crate::state::StickerTool::Move {
                return Err(format!("active {:?}, tool {:?}", edit.active_layer, edit.tool));
            }
            if !matches!(stamp(s)?.source, StampSource::Image { .. }) {
                return Err("the stamp is no longer an image stamp".into());
            }
            s.sticker_canvas_rect.filter(|r| r.width() > 100.0).ok_or("the canvas is not laid out")?;
            panel_rect(s, PanelHit::StickerLayerRow(1))?;
            panel_rect(s, PanelHit::StickerToolButton(crate::state::StickerTool::Move))?;
            undo_since(s, sc, 0)?;
            if composite_hash(doc_of(s)?) != remembered(sc, "start")? { Err("the composite changed".into()) } else { Ok(()) }
        }),
        Step::Shot {
            name: "sticker_enter",
            expect: "left: the logo on a checkerboard with a grey artboard outline and a blue layer outline, header with the \
                     stamp name, Done, greyed Revert, zoom, Fit and 100 %; right: the sphere with the logo; tool strip: the \
                     sticker tools with Move highlighted; Layers tab: one picture row",
        },
        // (2) A second picture layer: promotion and add in one entry.
        Step::Setup("register the disc picture", |s, sc| {
            let png = disc_png()?;
            let blob = s.doc_mut().ok_or("no document")?.add_image_blob("disc.png", Arc::from(png)).map_err(|e| e.to_string())?;
            sc.values.insert("disc_blob", u64::from(blob.0));
            Ok(())
        }),
        Step::Dyn("add the disc as a picture layer", |_, sc| {
            let blob = wte_core::BlobId(remembered(sc, "disc_blob")? as u32);
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::AddPictureLayer(Some(blob))))])
        }),
        Step::WaitIdle,
        Step::Assert("a sticker stamp with two layers, the disc active; one entry; the model changed", |s, sc| {
            let doc = sticker_doc(s)?;
            if doc.layers.len() != 2 || edit(s)?.active_layer != Some(doc.layers[1].id) || doc.layers[1].name != "disc" {
                return Err(format!("{} layers, active {:?}", doc.layers.len(), edit(s)?.active_layer));
            }
            undo_since(s, sc, 1)?;
            changed_since(s, sc, "start")
        }),
        Step::Setup("remember after the add", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "added")?;
            let t = sticker_doc(s)?.layers[1].transform.translate;
            sc.values.insert("disc_x", t.x.to_bits() as u64);
            sc.values.insert("disc_y", t.y.to_bits() as u64);
            Ok(())
        }),
        // (3) Move the disc with a drag on the canvas.
        Step::Dyn("drag the disc on the canvas", |s, _| {
            let doc = sticker_doc(s)?;
            let layer = &doc.layers[1];
            let c = layer.transform.translate + Vec2::splat(DISC_SIDE as f32 * 0.5);
            let path: Vec<[f32; 2]> =
                (0..=12).map(|k| canvas_norm(s, c + Vec2::from_array(DRAG) * (k as f32 / 12.0))).collect::<Result<_, _>>()?;
            Ok(vec![Step::Pointer {
                view: Target::StickerCanvas,
                path,
                button: egui::PointerButton::Primary,
                mods: egui::Modifiers::NONE,
                points_per_frame: 2,
            }])
        }),
        Step::WaitIdle,
        Step::Assert("the disc moved by the drag; one entry; the model changed", |s, sc| {
            let t = sticker_doc(s)?.layers[1].transform.translate;
            let from = Vec2::new(f32::from_bits(remembered(sc, "disc_x")? as u32), f32::from_bits(remembered(sc, "disc_y")? as u32));
            let moved = t - from;
            if (moved - Vec2::from_array(DRAG)).abs().max_element() > 1.0 {
                return Err(format!("moved by {moved:?}, expected about {DRAG:?}"));
            }
            undo_since(s, sc, 1)?;
            changed_since(s, sc, "added")
        }),
        Step::Setup("remember after the move", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "moved")
        }),
        Step::Shot {
            name: "sticker_moved",
            expect: "canvas: the red disc below right of the logo's centre with a blue outline around the disc; the sphere \
                     shows the logo with the disc; Layers tab: two rows, disc on top and highlighted, Revert enabled",
        },
        // (4) Lower its opacity.
        Step::Dyn("disc opacity 50 %", |s, _| {
            let id = sticker_doc(s)?.layers[1].id;
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::SetLayerOpacity(id, 0.5, Commit::Final)))])
        }),
        Step::WaitIdle,
        Step::Assert("opacity 0.5; one entry; the model changed", |s, sc| {
            let o = sticker_doc(s)?.layers[1].opacity;
            if o != 0.5 {
                return Err(format!("opacity {o}"));
            }
            undo_since(s, sc, 1)?;
            changed_since(s, sc, "moved")
        }),
        Step::Setup("remember after the opacity", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "faded")
        }),
        // (5) Reorder with the Layers tab's Down button.
        Step::Dyn("click Down in the Layers tab", |s, _| {
            let at = panel_rect(s, PanelHit::StickerLayerDown)?.center();
            Ok(vec![Step::ScreenClick { at, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("the disc is the bottom layer; one entry; the model changed", |s, sc| {
            let doc = sticker_doc(s)?;
            if doc.layers[0].name != "disc" {
                return Err(format!("bottom layer {:?}", doc.layers[0].name));
            }
            undo_since(s, sc, 1)?;
            changed_since(s, sc, "faded")
        }),
        Step::Setup("remember after the reorder", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "reordered")
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "sticker_reordered",
            expect: "canvas: the half-transparent disc now behind the logo; Layers tab: the logo row above the disc row, the \
                     disc row highlighted with 50 % in its details",
        },
        // (6) Delete in the Sticker view never deletes the stamp.
        Step::Key(egui::Modifiers::NONE, egui::Key::Delete),
        Step::WaitIdle,
        Step::Assert("Delete left the stamp and the sticker", |s, sc| {
            stamp(s)?;
            if sticker_doc(s)?.layers.len() != 2 || s.status_hint != crate::sticker::DELETE_HINT {
                return Err(format!("status \"{}\"", s.status_hint));
            }
            undo_since(s, sc, 0)
        }),
        // (7) 900 x 600.
        Step::Resize([900, 600]),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("900x600: canvas, divider and 3D half laid out; the tool strip fits", |s, _| {
            let canvas = s.sticker_canvas_rect.ok_or("no canvas")?;
            let view = s.view_rects.get(&ViewId::Main3D).copied().ok_or("no 3D half")?;
            if canvas.right() > view.left() || canvas.width() < 100.0 || view.width() < 100.0 {
                return Err(format!("canvas {canvas:?}, 3D {view:?}"));
            }
            let last = panel_rect(s, PanelHit::StickerToolButton(crate::state::StickerTool::Shape))?;
            let central = s.central_rect.ok_or("no central rect")?;
            if last.bottom() > central.bottom() + 0.5 { Err(format!("the last tool button {last:?} is cut off")) } else { Ok(()) }
        }),
        Step::Shot {
            name: "sticker_small",
            expect: "900x600: the same Sticker view, smaller: canvas left with the sticker, sphere right, all 14 sticker \
                     tools and the colour chip in the tool strip",
        },
        Step::Resize([1280, 800]),
        Step::WaitIdle,
        // (8) One undo per step, then redo.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo 1: the opacity state", |s, sc| same_composite(s, sc, "faded")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo 2: the moved state", |s, sc| same_composite(s, sc, "moved")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo 3: the added state", |s, sc| same_composite(s, sc, "added")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo 4: the start; the stamp is an image stamp again; the view stays open", |s, sc| {
            if !matches!(stamp(s)?.source, StampSource::Image { .. }) || s.view_mode != ViewMode::Sticker {
                return Err(format!("view {:?}", s.view_mode));
            }
            same_composite(s, sc, "start")
        }),
        Step::Do(AppAction::Redo),
        Step::Do(AppAction::Redo),
        Step::Do(AppAction::Redo),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("redo 4: the reordered state", |s, sc| same_composite(s, sc, "reordered")),
        // (9) Revert and its undo.
        Step::Dyn("click Revert", |s, _| {
            let at = panel_rect(s, PanelHit::StickerRevert)?.center();
            Ok(vec![Step::ScreenClick { at, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("Revert: the image stamp and the start picture; one entry", |s, sc| {
            if !matches!(stamp(s)?.source, StampSource::Image { .. }) {
                return Err("the stamp is not an image stamp after Revert".into());
            }
            if doc_of(s)?.undo_label() != Some(wte_core::REVERT_LABEL) {
                return Err(format!("undo label {:?}", doc_of(s)?.undo_label()));
            }
            undo_since(s, sc, 1)?;
            same_composite(s, sc, "start")
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("the undo of Revert: the reordered state", |s, sc| same_composite(s, sc, "reordered")),
        // (10) Esc leaves the view to the 3D view; the paint tool is the one before.
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
        Step::Assert("Esc: back in the 3D view, the editor closed, the Transform tool kept", |s, _| {
            if s.view_mode != ViewMode::View3D || session_of(s)?.sticker_edit.is_some() || s.sticker_canvas_rect.is_some() {
                return Err(format!("view {:?}", s.view_mode));
            }
            tool_is(s, ToolKind::Transform)
        }),
        Step::Shot {
            name: "sticker_left",
            expect: "the 3D view of the sphere with the logo and the faded disc behind it inside the Transform gizmo (white \
                     box with handles); the paint tool strip is back with Transform highlighted; the Tool tab shows the \
                     Transform options with Edit Sticker; the Layers tab lists Logo, Paint 1, Base",
        },
        // (11) Undoing everything brings the start back.
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("after undoing everything: the start", |s, sc| {
            if doc_of(s)?.history.undo_len() as u64 != remembered(sc, "start_undo")? {
                return Err(format!("{} undo entries", doc_of(s)?.history.undo_len()));
            }
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            same_composite(s, sc, "start")
        }),
    ]
}

/// The open Sticker view.
fn edit(s: &AppState) -> Result<&crate::state::StickerEdit, String> {
    session_of(s)?.sticker_edit.as_ref().ok_or_else(|| "the Sticker view is not open".to_owned())
}

/// The stamp of the open Sticker view (or the selected stamp).
fn stamp(s: &AppState) -> Result<&wte_core::StampObject, String> {
    let session = session_of(s)?;
    let id = match session.sticker_edit.as_ref().map(|e| e.target) {
        Some(crate::state::StickerTarget::Stamp(id)) => id,
        _ => session.selected_stamp.ok_or("no stamp")?,
    };
    session.doc.stamp(id).ok_or_else(|| "the stamp is gone".to_owned())
}

/// The sticker of the edited stamp.
fn sticker_doc(s: &AppState) -> Result<&wte_core::StickerDoc, String> {
    match stamp(s)?.source {
        StampSource::Sticker { sticker } => doc_of(s)?.sticker(sticker).map(|d| &**d).ok_or_else(|| "no sticker".to_owned()),
        _ => Err("the stamp is not a sticker stamp".into()),
    }
}

/// Layer-space point -> normalized canvas coordinates (the canvas camera of the last frame).
fn canvas_norm(s: &AppState, p: Vec2) -> Result<[f32; 2], String> {
    let rect = s.sticker_canvas_rect.ok_or("the canvas is not laid out")?;
    let doc = sticker_doc(s)?;
    let geom = crate::sticker::canvas::Geom { origin: Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32), size: doc.size() };
    let pos = geom.to_screen(&edit(s)?.camera, rect, p);
    Ok([(pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height()])
}

/// The composite equals the one remembered under `key`.
fn same_composite(s: &AppState, sc: &Scratch, key: &str) -> Result<(), String> {
    if composite_hash(doc_of(s)?) == remembered(sc, key)? { Ok(()) } else { Err(format!("the composite differs from \"{key}\"")) }
}

/// A hash of what the user can see change in the document: the composite, every stamp (source kind, placement, tint,
/// styles) and the stickers the stamps show.
fn doc_state_hash(doc: &Document) -> u64 {
    let mut h = composite_hash(doc);
    for stamp in &doc.stamps {
        h = wte_core::fnv1a64(h, format!("{:?}|{:?}|{:?}|{:?}", stamp.source, stamp.placement, stamp.tint, stamp.styles).as_bytes());
        if let StampSource::Sticker { sticker } = stamp.source
            && let Some(d) = doc.sticker(sticker)
        {
            for l in &d.layers {
                h = wte_core::fnv1a64(h, format!("{}|{}|{:?}|{:?}", l.name, l.opacity, l.transform, l.blend).as_bytes());
            }
        }
    }
    h
}

/// The disc picture as PNG bytes.
fn disc_png() -> Result<Vec<u8>, String> {
    let mut img = PixelBuf::new(DISC_SIDE, DISC_SIDE);
    let c = DISC_SIDE as f32 * 0.5;
    for y in 0..DISC_SIDE {
        for x in 0..DISC_SIDE {
            let d = Vec2::new(x as f32 + 0.5 - c, y as f32 + 0.5 - c).length();
            let a = (c - 2.0 - d).clamp(0.0, 1.0);
            if a > 0.0 {
                img.set(x, y, [230, 40, 50, (255.0 * a + 0.5) as u8]);
            }
        }
    }
    wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).map_err(|e| e.to_string())
}
