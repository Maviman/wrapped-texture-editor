// Stickers S1 (STICKER_PLAN 7 "S1", 8.2; ARCHITECTURE.md 8.11, 8.15): the scenario `sticker_grabcut` - the Cut Out Box
// tool (GrabCut) in the Sticker view of the real window, through a pointer drag on `Target::StickerCanvas`.
//
// A generated photo (an orange ball with shading on textured grass, 320 x 240) placed as an image stamp on the sphere;
// the view opened with 4. (1) Shift+W chooses the Cut Out Box. (2) A box dragged around the ball: the cut runs on a
// job (the idle check waits for it) and lands as one entry "Cut Out Box", the image stamp promoted inside it: the
// grass is gone (alpha 0 outside the box and between the box and the ball), the ball stays opaque, and the kept
// pixels match the ball with IoU >= 0.9. (3) Undo: the image stamp again. (4) "Into mask (keep pixels)" clicked in the
// Tool tab and the same box again: one entry, the pixels unchanged, a mask that hides the grass. (5) The ClassicSticker
// style on the stamp: the sphere shows the die-cut ball. Every step one undo step; undoing everything brings the start
// back; Esc leaves the view.

use super::sticker::{doc_state_hash, same_composite};
use super::*;
use crate::selftest::Target;
use crate::sticker::StickerAction;

/// Size of the generated photo; the ball's centre and radius.
const W: u32 = 320;
const H: u32 = 240;
const CENTRE: [f32; 2] = [160.0, 125.0];
const RADIUS: f32 = 70.0;
/// The box the scenario drags (artboard pixels): the ball plus a margin.
const BOX: [f32; 4] = [72.0, 38.0, 248.0, 212.0];

/// A hash of `(x, y, k)` in 0..1 (the photo's grain and blotches; deterministic).
fn hash(x: u32, y: u32, k: u32) -> f32 {
    let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ k.wrapping_mul(0xC2B2_AE3D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    v = v.wrapping_mul(0x297A_2D39);
    v ^= v >> 15;
    (v >> 8) as f32 / (1u32 << 24) as f32
}

/// Smooth blotches: the hash on a 24-pixel grid, interpolated.
fn blotch(x: u32, y: u32) -> f32 {
    let (fx, fy) = (x as f32 / 24.0, y as f32 / 24.0);
    let (ix, iy) = (fx as u32, fy as u32);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let top = hash(ix, iy, 7) * (1.0 - tx) + hash(ix + 1, iy, 7) * tx;
    let bottom = hash(ix, iy + 1, 7) * (1.0 - tx) + hash(ix + 1, iy + 1, 7) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// The ball's coverage of pixel `(x, y)` (a one-pixel soft edge).
fn coverage(x: u32, y: u32) -> f32 {
    let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::from_array(CENTRE));
    (RADIUS + 0.5 - d).clamp(0.0, 1.0)
}

/// The generated photo: the ball (shaded orange) on grass (green with brown blotches and grain), opaque.
fn photo() -> PixelBuf {
    let mut p = PixelBuf::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let b = blotch(x, y);
            let g = (hash(x, y, 1) - 0.5) * 36.0;
            let grass = if b > 0.7 {
                [125.0 + 20.0 * b + g, 92.0 + g * 0.7, 52.0 + g * 0.5]
            } else {
                [45.0 + 40.0 * b + g, 105.0 + 70.0 * b + g, 35.0 + 20.0 * b + g * 0.5]
            };
            let shade = 1.0 - 0.3 * ((x as f32 - CENTRE[0]) + (y as f32 - CENTRE[1])) / (2.0 * RADIUS);
            let fine = (hash(x, y, 2) - 0.5) * 12.0;
            let ball = [235.0 * shade + fine, 125.0 * shade + fine, 30.0 * shade + fine * 0.5];
            let k = coverage(x, y);
            let c = |i: usize| (grass[i] * (1.0 - k) + ball[i] * k).round().clamp(0.0, 255.0) as u8;
            p.set(x, y, [c(0), c(1), c(2), 255]);
        }
    }
    p
}

fn setup_picture(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let img = photo();
    let png = wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).map_err(|e| e.to_string())?;
    let doc = state.doc_mut().ok_or("no session")?;
    let blob = doc.add_image_blob("ball.png", Arc::from(png)).map_err(|e| e.to_string())?;
    state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
    state.tools.stamp.source_name = "Ball".to_owned();
    sc.values.insert("blob", u64::from(blob.0));
    Ok(())
}

/// The stamp of the open view.
fn the_stamp(s: &AppState) -> Result<&wte_core::StampObject, String> {
    super::sticker::stamp(s)
}

/// The sticker of the stamp (`None` while it is an image stamp).
fn the_sticker(s: &AppState) -> Result<Option<&wte_core::StickerDoc>, String> {
    Ok(match the_stamp(s)?.source {
        StampSource::Sticker { sticker } => Some(&**doc_of(s)?.sticker(sticker).ok_or("no sticker")?),
        _ => None,
    })
}

/// Layer-space point -> normalized canvas coordinates (an image stamp's virtual sticker is its picture at the origin).
fn cnorm(s: &AppState, p: Vec2) -> Result<[f32; 2], String> {
    let rect = s.sticker_canvas_rect.ok_or("the canvas is not laid out")?;
    let geom = match the_sticker(s)? {
        Some(d) => crate::sticker::canvas::Geom { origin: Vec2::new(d.artboard.x0 as f32, d.artboard.y0 as f32), size: d.size() },
        None => crate::sticker::canvas::Geom { origin: Vec2::ZERO, size: [W, H] },
    };
    let edit = s.session.as_ref().and_then(|x| x.sticker_edit.as_ref()).ok_or("the Sticker view is not open")?;
    let pos = geom.to_screen(&edit.camera, rect, p);
    Ok([(pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height()])
}

/// The box drag: a press at one corner, through the middle, a release at the other.
fn box_drag(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    let (a, b) = (Vec2::new(BOX[0], BOX[1]), Vec2::new(BOX[2], BOX[3]));
    let mut path = Vec::new();
    for k in 0..=16 {
        path.push(cnorm(s, a + (b - a) * (k as f32 / 16.0))?);
    }
    Ok(vec![Step::Pointer { view: Target::StickerCanvas, path, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE, points_per_frame: 2 }])
}

/// Layer 0's pixels (the blob's while the cut went into the mask) and its mask.
fn layer0(s: &mut AppState) -> Result<(Arc<PixelBuf>, Option<Arc<wte_core::AlphaBuf>>), String> {
    let StampSource::Sticker { sticker } = the_stamp(s)?.source else { return Err("the stamp is not a sticker yet".into()) };
    let id = the_sticker(s)?.ok_or("no sticker")?.layers.first().ok_or("no layer")?.id;
    let on = wte_core::StickerRef::Id(sticker);
    let doc = s.doc_mut().ok_or("no document")?;
    let pixels = doc.sticker_layer_pixels(on, id).map_err(|e| e.to_string())?;
    let mask = doc.sticker_layer_mask(on, id).map_err(|e| e.to_string())?;
    Ok((pixels, mask))
}

/// IoU of "kept" (`value(i) >= 128`) against the ball (coverage >= 1/2).
fn iou(value: impl Fn(usize) -> u8) -> f64 {
    let (mut inter, mut union) = (0u64, 0u64);
    for y in 0..H {
        for x in 0..W {
            let kept = value((y * W + x) as usize) >= 128;
            let ball = coverage(x, y) >= 0.5;
            inter += u64::from(kept && ball);
            union += u64::from(kept || ball);
        }
    }
    inter as f64 / union.max(1) as f64
}

/// The last entry is `label` and there are `extra` more entries than remembered.
fn entry(s: &AppState, sc: &Scratch, label: &str, extra: u64) -> Result<(), String> {
    let got = doc_of(s)?.undo_label();
    if got != Some(label) {
        return Err(format!("the last entry is {got:?}, expected {label:?}"));
    }
    undo_since(s, sc, extra)
}

fn the_cut(s: &mut AppState, sc: &Scratch) -> Result<(), String> {
    entry(s, sc, crate::sticker::tools::cut_box::CUT_BOX_LABEL, 1)?;
    let (pixels, mask) = layer0(s)?;
    if mask.is_some() {
        return Err("a mask was added (Into mask is off)".into());
    }
    let a = |x: u32, y: u32| pixels.get(x, y)[3];
    let (corner, between, centre, top) = (a(5, 5), a(85, 50), a(160, 125), a(160, 60));
    if corner != 0 || between != 0 || centre != 255 || top != 255 {
        return Err(format!("alpha: corner {corner}, between box and ball {between}, centre {centre}, top of the ball {top}"));
    }
    let v = iou(|i| pixels.data[i * 4 + 3]);
    if v < 0.9 {
        return Err(format!("IoU {v:.4} below 0.9"));
    }
    if s.status_hint.contains("Cutting") {
        return Err(format!("the progress note stayed: {}", s.status_hint));
    }
    Ok(())
}

/// `sticker_grabcut` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_grabcut() -> Vec<Step> {
    let n = egui::Modifiers::NONE;
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("register the generated photo", setup_picture),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the photo as an image stamp", |s, _| model_click(s, ViewId::Main3D, [0.0, -0.05])),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| placed_stamp(s, false).map(|_| ())),
        Step::Setup("remember the start", |s, sc| {
            remember_composite(s, sc, "start")?;
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            Ok(())
        }),
        Step::Key(n, egui::Key::Num4),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the Sticker view on the image stamp", |s, _| {
            if s.view_mode != ViewMode::Sticker || the_sticker(s)?.is_some() {
                return Err(format!("view {:?}", s.view_mode));
            }
            Ok(())
        }),
        // (1) Shift+W: the Cut Out Box.
        Step::Key(egui::Modifiers::SHIFT, egui::Key::W),
        Step::WaitFrames(2),
        Step::Assert("Shift+W chose the Cut Out Box; the Tool tab shows its option", |s, _| {
            let tool = crate::sticker::tool_of(s);
            if tool != crate::state::StickerTool::CutOutBox {
                return Err(format!("the tool is {tool:?}"));
            }
            panel_rect(s, PanelHit::StickerCutBoxInto).map(|_| ())
        }),
        // (2) The box around the ball.
        Step::Setup("remember the undo count", remember_undo),
        Step::Dyn("drag a box around the ball", box_drag),
        Step::WaitIdle,
        Step::Assert("one \"Cut Out Box\" entry: the grass is gone, the ball opaque, IoU >= 0.9; promoted", |s, sc| {
            if the_sticker(s)?.is_none() {
                return Err("the image stamp was not promoted".into());
            }
            entry(s, sc, crate::sticker::tools::cut_box::CUT_BOX_LABEL, 1)
        }),
        // The pixels need the document mutably (an `Assert` reads the state only).
        Step::Setup("check the cut's pixels", |s, sc| the_cut(s, sc)),
        Step::Shot {
            name: "grabcut_cut",
            expect: "canvas: the orange ball alone on the checkerboard (no grass around it, a clean round edge); the sphere \
                     shows the ball without its grass square; the Tool tab shows the Cut Out Box options (Into mask)",
        },
        // (3) Undo: the image stamp again.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo: the image stamp and the start composite", |s, sc| {
            if the_sticker(s)?.is_some() {
                return Err("still a sticker".into());
            }
            same_composite(s, sc, "start")
        }),
        // (4) Into mask, clicked in the Tool tab, and the same box.
        Step::Dyn("click Into mask", |s, _| {
            let at = panel_rect(s, PanelHit::StickerCutBoxInto)?.left_center() + egui::vec2(8.0, 0.0);
            Ok(vec![Step::ScreenClick { at, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitFrames(2),
        Step::Assert("Into mask is on", |s, _| if s.sticker_ui.cut_box.into_mask { Ok(()) } else { Err("the option is off".into()) }),
        Step::Setup("remember the undo count", remember_undo),
        Step::Dyn("the same box again", box_drag),
        Step::WaitIdle,
        Step::Setup("one entry: the pixels kept, a mask hides the grass", |s, sc| {
            entry(s, sc, crate::sticker::tools::cut_box::CUT_BOX_LABEL, 1)?;
            let (pixels, mask) = layer0(s)?;
            if *pixels != photo() {
                return Err("the pixels changed".into());
            }
            let mask = mask.ok_or("no mask")?;
            let (corner, centre) = (mask.get(5, 5), mask.get(160, 125));
            if corner != 0 || centre != 255 {
                return Err(format!("mask: corner {corner}, centre {centre}"));
            }
            let v = iou(|i| mask.data[i]);
            if v < 0.9 {
                return Err(format!("mask IoU {v:.4}"));
            }
            Ok(())
        }),
        Step::Shot {
            name: "grabcut_mask",
            expect: "canvas: the ball alone on the checkerboard as before (the grass hidden by the mask); the Tool tab's Into \
                     mask box is ticked and its Mask block offers Invert / Apply / Delete with Add mask greyed (the layer \
                     has a mask now)",
        },
        // (5) The die-cut style on the stamp.
        Step::Setup("remember before the style", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::ApplyPreset(wte_core::StylePreset::ClassicSticker))),
        Step::WaitIdle,
        Step::Setup("the stamp has the die-cut style; the finished sticker: corner alpha 0, ball alpha 255", |s, sc| {
            undo_since(s, sc, 1)?;
            let StampSource::Sticker { sticker } = the_stamp(s)?.source else { return Err("not a sticker".into()) };
            let pic = s.doc_mut().ok_or("no doc")?.sticker_picture(sticker, None, 1.0).map_err(|e| e.to_string())?;
            let (corner, ball) = (pic.get(2, 2), pic.get(160, 125));
            if corner[3] != 0 || ball[3] != 255 {
                return Err(format!("corner {corner:?}, ball {ball:?}"));
            }
            Ok(())
        }),
        Step::Shot {
            name: "grabcut_die_cut",
            expect: "3D half: the sphere with the orange ball as a die-cut sticker (a white border following the ball's round \
                     edge and a soft shadow, no grass); canvas: the ball on the checkerboard",
        },
        // Undo everything: the start.
        Step::Dyn("undo every step", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undoing everything: the start, an image stamp again", |s, sc| {
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            if !matches!(the_stamp(s)?.source, StampSource::Image { .. }) {
                return Err("the stamp is not an image stamp".into());
            }
            same_composite(s, sc, "start")
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
        Step::Assert("Esc left the Sticker view", |s, _| {
            if s.view_mode == ViewMode::Sticker { Err("still in the Sticker view".into()) } else { Ok(()) }
        }),
    ]
}
