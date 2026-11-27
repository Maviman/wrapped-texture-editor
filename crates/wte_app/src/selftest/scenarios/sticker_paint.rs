// Stickers P7a (STICKER_PLAN 7 "P7a", 8.2; ARCHITECTURE.md 8.11, 8.15): the scenarios `sticker_edit` and `sticker_pen`
// - painting in the Sticker view of the real window, every stroke through pointer (or pen) steps on
// `Target::StickerCanvas`.
//
// `sticker_edit`: the sphere sample with the sample logo placed as an image stamp; the view opened with 4; a new picture
// layer; a brush stroke and an eraser stroke, each one undo step (undone and redone); ClassicSticker on the layer, then
// a stroke on it: the header's "styles update when you lift" note is on during the stroke (counted per frame) and gone
// after it, the canvas's restyle lands, and the finished sticker and the model show the white contour around the new
// paint; Fill on the logo's transparent outside; a gradient on a third layer inside a rectangle selection made by a
// command; undoing everything brings the start back.
//
// `sticker_pen`: recorded pen samples on the canvas: a light and a full-pressure stroke (the full one is wider), the
// eraser end erases across both (the sticker Eraser while the pen touches, the Brush back after), a barrel drag pans
// the canvas and changes nothing; undoing everything brings the start back.

use super::sticker::{canvas_norm, doc_state_hash, edit, same_composite, sticker_doc};
use super::*;
use crate::selftest::{PenPoint, Target};
use crate::state::StickerTool;
use crate::sticker::StickerAction;

/// Brush size of the scenarios (layer-space pixels; the logo's artboard is 256 x 256).
const SIZE: f32 = 16.0;
const YELLOW: [u8; 4] = [250, 210, 30, 255];

/// The steps that place the logo as an image stamp, open the Sticker view on it and add an empty picture layer.
fn open_with_layer() -> Vec<Step> {
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
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            Ok(())
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Num4),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Do(AppAction::Sticker(StickerAction::AddPictureLayer(None))),
        Step::WaitIdle,
        Step::Assert("the Sticker view with two layers, the new one active", |s, _| {
            let doc = sticker_doc(s)?;
            if s.view_mode != ViewMode::Sticker || doc.layers.len() != 2 || edit(s)?.active_layer != Some(doc.layers[1].id) {
                return Err(format!("view {:?}, {} layers", s.view_mode, doc.layers.len()));
            }
            Ok(())
        }),
        Step::Setup("remember after the add", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "added")
        }),
    ]
}

/// A mouse drag on the canvas through layer-space points.
fn canvas_drag(s: &AppState, points: &[[f32; 2]]) -> Result<Vec<Step>, String> {
    let mut path = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (Vec2::from_array(w[0]), Vec2::from_array(w[1]));
        let n = ((b - a).length() / 6.0).ceil().max(1.0) as usize;
        for k in 0..n {
            path.push(canvas_norm(s, a + (b - a) * (k as f32 / n as f32))?);
        }
    }
    path.push(canvas_norm(s, Vec2::from_array(*points.last().ok_or("no points")?))?);
    Ok(vec![Step::Pointer { view: Target::StickerCanvas, path, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE, points_per_frame: 2 }])
}

/// The pixel of sticker layer `index` (bottom = 0) at content pixel `(x, y)` (owned pictures only).
fn layer_px(s: &AppState, index: usize, x: u32, y: u32) -> Result<[u8; 4], String> {
    let doc = sticker_doc(s)?;
    let l = doc.layers.get(index).ok_or_else(|| format!("no layer {index}"))?;
    let p = l.owned_picture().ok_or_else(|| format!("layer {index} has no edited pixels"))?;
    if x >= p.width || y >= p.height {
        return Err(format!("({x}, {y}) is outside the {} x {} layer", p.width, p.height));
    }
    Ok(p.get(x, y))
}

/// Rows of column `x` of layer `index` with alpha above 0 within `rows` (the width of a horizontal stroke there).
fn column_width(s: &AppState, index: usize, x: u32, rows: std::ops::Range<u32>) -> Result<u64, String> {
    let mut n = 0;
    for y in rows {
        if layer_px(s, index, x, y)?[3] > 0 {
            n += 1;
        }
    }
    Ok(n)
}

/// The last entry's label is `label` and there are `extra` more entries than remembered.
fn entry(s: &AppState, sc: &Scratch, label: &str, extra: u64) -> Result<(), String> {
    let got = doc_of(s)?.undo_label();
    if got != Some(label) {
        return Err(format!("the last entry is {got:?}, expected {label:?}"));
    }
    undo_since(s, sc, extra)
}

fn near(c: [u8; 4], want: [u8; 4], tol: u8) -> bool {
    c.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= tol)
}

/// `sticker_edit` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_edit() -> Vec<Step> {
    let mut steps = open_with_layer();
    steps.extend([
        // (1) A brush stroke: one entry.
        Step::Do(AppAction::SetStickerTool(StickerTool::Brush)),
        Step::Setup("brush: size 16, hard, full flow, no pressure; red", |s, _| {
            s.tools.brush = BrushSettings { size_px: SIZE, hardness: 1.0, flow: 1.0, opacity: 1.0, pressure_size: false, ..BrushSettings::default() };
            s.colors.primary = RED;
            Ok(())
        }),
        Step::Dyn("brush stroke along y = 60", |s, _| canvas_drag(s, &[[30.0, 60.0], [226.0, 60.0]])),
        Step::WaitIdle,
        Step::Assert("one \"Sticker Brush\" entry; red paint on the new layer; the model changed", |s, sc| {
            entry(s, sc, wte_core::STICKER_BRUSH_LABEL, 1)?;
            let (on, off) = (layer_px(s, 1, 128, 60)?, layer_px(s, 1, 128, 100)?);
            if !near(on, RED, 2) || off[3] != 0 {
                return Err(format!("on the line {on:?}, beside it {off:?}"));
            }
            changed_since(s, sc, "added")
        }),
        Step::Setup("remember the brush stroke", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "brushed")
        }),
        Step::Shot {
            name: "edit_brush",
            expect: "canvas: the logo with a straight red line across its upper part, the new layer active in the Layers tab; \
                     the sphere shows the logo with the red line; the Tool tab shows the sticker Brush options",
        },
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo: the line is gone", |s, sc| same_composite(s, sc, "added")),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("redo: the line is back", |s, sc| same_composite(s, sc, "brushed")),
        // (2) An eraser stroke across the line: one entry.
        Step::Do(AppAction::SetStickerTool(StickerTool::Eraser)),
        Step::Setup("eraser: size 24, hard, no pressure", |s, _| {
            s.tools.eraser = BrushSettings { size_px: 24.0, hardness: 1.0, flow: 1.0, pressure_size: false, ..BrushSettings::default() };
            Ok(())
        }),
        Step::Dyn("eraser stroke across the line at x = 128", |s, _| canvas_drag(s, &[[128.0, 30.0], [128.0, 90.0]])),
        Step::WaitIdle,
        Step::Assert("one \"Sticker Eraser\" entry; a gap in the line", |s, sc| {
            entry(s, sc, wte_core::STICKER_ERASER_LABEL, 1)?;
            let (gap, kept) = (layer_px(s, 1, 128, 60)?, layer_px(s, 1, 60, 60)?);
            if gap[3] != 0 || !near(kept, RED, 2) {
                return Err(format!("in the gap {gap:?}, beside it {kept:?}"));
            }
            changed_since(s, sc, "brushed")
        }),
        Step::Setup("remember the eraser stroke", |s, sc| remember_composite(s, sc, "erased")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo: the line is whole again", |s, sc| same_composite(s, sc, "brushed")),
        Step::Do(AppAction::Redo),
        Step::WaitIdle,
        Step::Assert("redo: the gap is back", |s, sc| same_composite(s, sc, "erased")),
        // (3) ClassicSticker on the layer, then a stroke in stroke mode.
        Step::Dyn("ClassicSticker on the new layer", |s, _| {
            let id = sticker_doc(s)?.layers[1].id;
            let styles = wte_core::StylePreset::ClassicSticker.styles([256, 256]);
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::SetLayerStyles(id, Box::new(styles), Commit::Final)))])
        }),
        Step::WaitIdle,
        Step::Setup("remember the styled layer and the live counters", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "styled")?;
            let c = s.sticker_ui.counters;
            sc.values.insert("note_frames", c.note_frames);
            sc.values.insert("full_composites", c.full_composites);
            sc.values.insert("restyles_applied", c.restyles_applied);
            Ok(())
        }),
        Step::Do(AppAction::SetStickerTool(StickerTool::Brush)),
        Step::Dyn("brush stroke on the styled layer along y = 180", |s, _| canvas_drag(s, &[[40.0, 180.0], [216.0, 180.0]])),
        Step::WaitIdle,
        Step::Assert("stroke mode: the note was on during the stroke and is gone; one entry; the restyle landed", |s, sc| {
            entry(s, sc, wte_core::STICKER_BRUSH_LABEL, 1)?;
            let c = s.sticker_ui.counters;
            if c.note_frames <= remembered(sc, "note_frames")? {
                return Err("the \"styles update when you lift\" note was never shown".into());
            }
            if s.sticker_ui.live.is_some() {
                return Err("the canvas is still in stroke mode".into());
            }
            if c.full_composites != remembered(sc, "full_composites")? + 1 || c.restyles_applied != remembered(sc, "restyles_applied")? + 1 {
                return Err(format!("counters {c:?}"));
            }
            changed_since(s, sc, "styled")
        }),
        Step::Setup("the finished sticker shows the white contour just below the new paint", |s, _| {
            let id = match edit(s)?.target {
                crate::state::StickerTarget::Stamp(stamp) => match doc_of(s)?.stamp(stamp).ok_or("no stamp")?.source {
                    StampSource::Sticker { sticker } => sticker,
                    _ => return Err("not a sticker stamp".into()),
                },
                _ => return Err("not a stamp target".into()),
            };
            // The paint ends at y = 188 (radius 8); the contour (6 px at 256) lies below it.
            let mut found = None;
            for y in 189..195u32 {
                if layer_px(s, 1, 128, y)?[3] != 0 {
                    continue;
                }
                let c = s.doc_mut().ok_or("no document")?.sticker_picture(id, None, 1.0).map_err(|e| e.to_string())?.get(128, y);
                if c[3] == 255 && c[0] > 230 && c[1] > 230 && c[2] > 230 {
                    found = Some(y);
                    break;
                }
            }
            found.map(|_| ()).ok_or_else(|| "no white contour pixel below the new paint".to_owned())
        }),
        Step::Setup("remember the styled stroke", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "styled_stroke")
        }),
        Step::Shot {
            name: "edit_styled",
            expect: "canvas: the red line with its gap and a second red line lower down, both with a white die-cut border and \
                     a soft shadow (ClassicSticker); the header without the stroke-mode note; the sphere shows the same white \
                     borders around both lines",
        },
        // (4) Fill on the logo's transparent outside.
        Step::Dyn("select the logo layer", |s, _| {
            let id = sticker_doc(s)?.layers[0].id;
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::SelectLayer(id)))])
        }),
        Step::Do(AppAction::SetStickerTool(StickerTool::Fill)),
        Step::Do(AppAction::SetPrimaryColor(YELLOW)),
        Step::Dyn("click the Fill tool on the logo's corner", |s, _| {
            Ok(vec![Step::Click { view: Target::StickerCanvas, at: canvas_norm(s, Vec2::new(8.5, 8.5))?, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("one \"Fill\" entry; the logo's outside is yellow, its ring kept", |s, sc| {
            entry(s, sc, "Fill", 1)?;
            let (corner, far) = (layer_px(s, 0, 8, 8)?, layer_px(s, 0, 250, 250)?);
            if !near(corner, YELLOW, 2) || !near(far, YELLOW, 2) {
                return Err(format!("corners {corner:?} {far:?}"));
            }
            let centre = layer_px(s, 0, 128, 128)?;
            if near(centre, YELLOW, 8) {
                return Err(format!("the logo's inside was filled too: {centre:?}"));
            }
            changed_since(s, sc, "styled_stroke")
        }),
        Step::Setup("remember the fill", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "filled")
        }),
        // (5) A gradient on a new layer inside a rectangle selection made by a command.
        Step::Do(AppAction::Sticker(StickerAction::AddPictureLayer(None))),
        Step::Do(AppAction::Sticker(StickerAction::SelectRect([40.0, 204.0, 216.0, 240.0]))),
        Step::Do(AppAction::SetStickerTool(StickerTool::Gradient)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Setup("secondary blue", |s, _| {
            s.colors.secondary = BLUE;
            Ok(())
        }),
        Step::WaitIdle,
        Step::Dyn("gradient drag along y = 222", |s, _| canvas_drag(s, &[[40.0, 222.0], [216.0, 222.0]])),
        Step::WaitIdle,
        Step::Assert("one \"Gradient\" entry after the layer's; red to blue inside the selection only", |s, sc| {
            entry(s, sc, "Gradient", 2)?;
            let (a, b, out) = (layer_px(s, 1, 41, 222)?, layer_px(s, 1, 214, 222)?, layer_px(s, 1, 128, 196)?);
            if !near(a, RED, 12) || !near(b, BLUE, 12) || out[3] != 0 {
                return Err(format!("start {a:?}, end {b:?}, outside {out:?}"));
            }
            changed_since(s, sc, "filled")
        }),
        Step::Do(AppAction::Sticker(StickerAction::SelectNone)),
        Step::WaitIdle,
        Step::Shot {
            name: "edit_final",
            expect: "canvas: the logo on a yellow square, the two red lines with white borders, a red-to-blue band at the \
                     bottom; the sphere shows the same sticker; Layers tab: three rows with the gradient layer above the logo",
        },
        // (6) Undo everything: the start.
        Step::Dyn("undo every step", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undoing everything: the start, an image stamp again", |s, sc| {
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            if !matches!(super::sticker::stamp(s)?.source, StampSource::Image { .. }) {
                return Err("the stamp is not an image stamp".into());
            }
            same_composite(s, sc, "start")
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
    ]);
    steps
}

/// Pen samples along layer-space points on the canvas: two hover samples, then contact at `pressure`.
fn canvas_pen(s: &AppState, from: Vec2, to: Vec2, pressure: f32, inverted: bool, barrel: bool) -> Result<Vec<Step>, String> {
    let a = canvas_norm(s, from)?;
    let mut samples = vec![PenPoint { at: [a[0] - 0.01, a[1]], pressure: 0.0 }, PenPoint { at: a, pressure: 0.0 }];
    let n = ((to - from).length() / 6.0).ceil().max(1.0) as usize;
    for k in 0..=n {
        samples.push(PenPoint { at: canvas_norm(s, from + (to - from) * (k as f32 / n as f32))?, pressure });
    }
    Ok(vec![Step::Pen { view: Target::StickerCanvas, samples, inverted, barrel, points_per_frame: 2 }])
}

/// `sticker_pen` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_pen() -> Vec<Step> {
    let mut steps = open_with_layer();
    steps.extend([
        Step::Do(AppAction::SetStickerTool(StickerTool::Brush)),
        Step::Setup("brush: size 24, hard; pressure drives the size from 0; blue", |s, _| {
            s.tools.brush = BrushSettings {
                size_px: 24.0,
                hardness: 1.0,
                flow: 1.0,
                pressure_size: true,
                pressure_size_min: 0.0,
                pressure_flow: false,
                ..BrushSettings::default()
            };
            s.colors.primary = BLUE;
            Ok(())
        }),
        // (1) Pressure changes the width.
        Step::Dyn("a light pen stroke (pressure 0.2) along y = 70", |s, _| canvas_pen(s, Vec2::new(30.0, 70.0), Vec2::new(226.0, 70.0), 0.2, false, false)),
        Step::WaitIdle,
        Step::Setup("measure the light stroke", |s, sc| {
            let w = column_width(s, 1, 128, 40..100)?;
            sc.values.insert("light", w);
            if w == 0 { Err("the light stroke painted nothing".into()) } else { Ok(()) }
        }),
        Step::Dyn("a full pen stroke (pressure 1.0) along y = 170", |s, _| canvas_pen(s, Vec2::new(30.0, 170.0), Vec2::new(226.0, 170.0), 1.0, false, false)),
        Step::WaitIdle,
        Step::Assert("two \"Sticker Brush\" entries; the full stroke is much wider", |s, sc| {
            entry(s, sc, wte_core::STICKER_BRUSH_LABEL, 2)?;
            let (light, full) = (remembered(sc, "light")?, column_width(s, 1, 128, 140..200)?);
            if full < 20 || full < light * 3 { Err(format!("width {light} at 0.2, {full} at 1.0")) } else { Ok(()) }
        }),
        Step::Setup("remember the strokes", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "pen_strokes")
        }),
        // (2) The eraser end erases across both lines.
        Step::Setup("reset the pen probes", super::pen::reset_probes),
        Step::Dyn("the eraser end across both lines at x = 128", |s, _| canvas_pen(s, Vec2::new(128.0, 40.0), Vec2::new(128.0, 200.0), 1.0, true, false)),
        Step::WaitIdle,
        Step::Assert("one \"Sticker Eraser\" entry; gaps in both lines; the sticker Eraser while touching", |s, sc| {
            entry(s, sc, wte_core::STICKER_ERASER_LABEL, 1)?;
            let (a, b) = (layer_px(s, 1, 128, 70)?, layer_px(s, 1, 128, 170)?);
            if a[3] != 0 || b[3] != 0 {
                return Err(format!("not erased: {a:?} {b:?}"));
            }
            let (contact, eraser) =
                (sc.values.get("pen_contact_frames").copied().unwrap_or(0), sc.values.get("pen_eraser_frames").copied().unwrap_or(0));
            if contact == 0 || eraser != contact {
                return Err(format!("the sticker Eraser in {eraser} of {contact} contact frames"));
            }
            changed_since(s, sc, "pen_strokes")
        }),
        Step::WaitFrames(2),
        Step::Assert("the pen left: the sticker Brush is back", |s, _| {
            let t = edit(s)?.tool;
            if t == StickerTool::Brush && s.pen.sticker_eraser_restore.is_none() { Ok(()) } else { Err(format!("sticker tool {t:?}")) }
        }),
        Step::Shot {
            name: "pen_canvas",
            expect: "canvas: a thin blue line at the top and a wide blue line lower down, both cut by a vertical gap in the \
                     middle; the sphere shows the same; the sticker Brush is highlighted in the tool strip",
        },
        // (3) A barrel drag pans the canvas and changes nothing.
        Step::Setup("remember the canvas camera", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "before_barrel")?;
            let c = edit(s)?.camera.center_uv;
            sc.values.insert("cam_x", u64::from(c.x.to_bits()));
            sc.values.insert("cam_y", u64::from(c.y.to_bits()));
            Ok(())
        }),
        Step::Dyn("a barrel drag over the canvas", |s, _| canvas_pen(s, Vec2::new(60.0, 120.0), Vec2::new(180.0, 140.0), 0.8, false, true)),
        Step::WaitIdle,
        Step::Assert("the canvas panned; nothing painted", |s, sc| {
            let c = edit(s)?.camera.center_uv;
            if u64::from(c.x.to_bits()) == remembered(sc, "cam_x")? && u64::from(c.y.to_bits()) == remembered(sc, "cam_y")? {
                return Err("the canvas did not pan".into());
            }
            undo_since(s, sc, 0)?;
            same_composite(s, sc, "before_barrel")
        }),
        Step::Do(AppAction::Sticker(StickerAction::Fit)),
        // (4) Undo everything: the start.
        Step::Dyn("undo every step", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undoing everything: the start", |s, sc| {
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            same_composite(s, sc, "start")
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
    ]);
    steps
}
