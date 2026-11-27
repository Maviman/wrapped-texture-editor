// Stickers P7b (STICKER_PLAN 7 "P7b", 8.2; ARCHITECTURE.md 8.11, 8.15): the scenario `sticker_cutout` - selections and
// cut-out in the Sticker view of the real window, the tools through pointer steps and keys on `Target::StickerCanvas`.
//
// A generated picture (an orange disc with a soft edge on white, 256 x 256) placed as an image stamp on the sphere; the
// view opened with 4. (1) A Marquee drag over the white corner and the Delete key: the corner's pixels go (one entry,
// the image stamp promoted inside it), the stamp stays. (2) Ctrl+D; the Magic Wand on the white, Fill with yellow (the
// selection is filled), undo. (3) Ctrl+D; the Magic Eraser (Shift+E) on the white, undo. (4) Remove Background: the
// white goes, the disc stays opaque, the soft edge has no white halo. (5) The original picture as a second layer and
// Color to Alpha through its popover (the live preview, the Apply button). (6) Defringe on the first layer. (7) Blur
// inside a Marquee selection through the popover: only the selected half changes. (8) A Marquee selection inside the
// disc dragged with the Move tool: "Move Selection" cuts it into a new layer. (9) The ClassicSticker style on the
// stamp: the model shows the die-cut sticker. Asserts: corner alpha 0, object alpha 255, the halo measure below its
// bound; every step one undo step; undoing everything brings the start back.

use super::sticker::{doc_state_hash, same_composite};
use super::*;
use crate::selftest::Target;
use crate::sticker::StickerAction;
use crate::sticker::popover::PopKind;

/// Side of the generated picture; the disc's centre and radius.
const SIDE: u32 = 256;
const CENTRE: f32 = 128.0;
const RADIUS: f32 = 80.0;
/// The disc's colour.
const ORANGE: [u8; 4] = [230, 120, 30, 255];
const YELLOW: [u8; 4] = [250, 220, 40, 255];
/// The halo bound: the mean blue excess (over the disc's blue) of the soft edge after the cut-out.
const HALO_BOUND: f64 = 40.0;

/// The generated picture: the orange disc with a soft edge of about 1.5 pixels on opaque white.
fn picture() -> PixelBuf {
    let mut p = PixelBuf::new(SIDE, SIDE);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::splat(CENTRE));
            let k = ((RADIUS + 0.75 - d) / 1.5).clamp(0.0, 1.0);
            let c = |a: u8| (255.0 + (a as f32 - 255.0) * k).round() as u8;
            p.set(x, y, [c(ORANGE[0]), c(ORANGE[1]), c(ORANGE[2]), 255]);
        }
    }
    p
}

fn setup_picture(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let img = picture();
    let png = wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).map_err(|e| e.to_string())?;
    let doc = state.doc_mut().ok_or("no session")?;
    let blob = doc.add_image_blob("badge.png", Arc::from(png)).map_err(|e| e.to_string())?;
    state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
    state.tools.stamp.source_name = "Badge".to_owned();
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
        None => crate::sticker::canvas::Geom { origin: Vec2::ZERO, size: [SIDE, SIDE] },
    };
    let edit = s.session.as_ref().and_then(|x| x.sticker_edit.as_ref()).ok_or("the Sticker view is not open")?;
    let pos = geom.to_screen(&edit.camera, rect, p);
    Ok([(pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height()])
}

fn canvas_drag(s: &AppState, points: &[[f32; 2]]) -> Result<Vec<Step>, String> {
    let mut path = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (Vec2::from_array(w[0]), Vec2::from_array(w[1]));
        let n = ((b - a).length() / 8.0).ceil().max(1.0) as usize;
        for k in 0..n {
            path.push(cnorm(s, a + (b - a) * (k as f32 / n as f32))?);
        }
    }
    path.push(cnorm(s, Vec2::from_array(*points.last().ok_or("no points")?))?);
    Ok(vec![Step::Pointer { view: Target::StickerCanvas, path, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE, points_per_frame: 2 }])
}

fn canvas_click(s: &AppState, p: [f32; 2]) -> Result<Vec<Step>, String> {
    Ok(vec![Step::Click { view: Target::StickerCanvas, at: cnorm(s, Vec2::from_array(p))?, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
}

/// The owned pixel of sticker layer `index` (bottom = 0).
fn px(s: &AppState, index: usize, x: u32, y: u32) -> Result<[u8; 4], String> {
    let d = the_sticker(s)?.ok_or("the stamp is not a sticker yet")?;
    let l = d.layers.get(index).ok_or_else(|| format!("no layer {index}"))?;
    let p = l.owned_picture().ok_or_else(|| format!("layer {index} has no edited pixels"))?;
    if x >= p.width || y >= p.height {
        return Err(format!("({x}, {y}) is outside the {} x {} layer", p.width, p.height));
    }
    Ok(p.get(x, y))
}

/// The selection value at an artboard pixel.
fn sel(s: &AppState, x: i32, y: i32) -> Result<u8, String> {
    let e = s.session.as_ref().and_then(|x| x.sticker_edit.as_ref()).ok_or("no view")?;
    Ok(e.selection.as_ref().map_or(0, |sel| sel.at(x, y)))
}

/// The last entry is `label` and there are `extra` more entries than remembered.
fn entry(s: &AppState, sc: &Scratch, label: &str, extra: u64) -> Result<(), String> {
    let got = doc_of(s)?.undo_label();
    if got != Some(label) {
        return Err(format!("the last entry is {got:?}, expected {label:?}"));
    }
    undo_since(s, sc, extra)
}

/// The halo measure of layer 0: over its soft-edge pixels (0 < alpha < 255), the mean of `max(0, blue - 30)` (the
/// white the edge kept), and how many such pixels there are.
fn halo(s: &AppState) -> Result<(f64, u64), String> {
    let d = the_sticker(s)?.ok_or("not a sticker")?;
    let p = d.layers[0].owned_picture().ok_or("no pixels")?;
    let (mut sum, mut n) = (0.0f64, 0u64);
    for c in p.data.as_chunks::<4>().0 {
        if c[3] > 0 && c[3] < 255 {
            sum += f64::from(c[2].saturating_sub(ORANGE[2]));
            n += 1;
        }
    }
    Ok((if n == 0 { 0.0 } else { sum / n as f64 }, n))
}

fn near(c: [u8; 4], want: [u8; 4], tol: u8) -> bool {
    c.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= tol)
}

/// A click on a drawn panel control (the popover's Apply).
fn click_panel(s: &AppState, hit: PanelHit) -> Result<Vec<Step>, String> {
    let at = panel_rect(s, hit)?.center();
    Ok(vec![Step::ScreenClick { at, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
}

fn key(mods: egui::Modifiers, k: egui::Key) -> Step {
    Step::Key(mods, k)
}

/// `sticker_cutout` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_cutout() -> Vec<Step> {
    let n = egui::Modifiers::NONE;
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("register the generated picture", setup_picture),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the picture as an image stamp", |s, _| model_click(s, ViewId::Main3D, [0.0, -0.05])),
        Step::WaitIdle,
        Step::Assert("the image stamp was placed and selected", |s, _| placed_stamp(s, false).map(|_| ())),
        Step::Setup("remember the start", |s, sc| {
            remember_composite(s, sc, "start")?;
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            sc.values.insert("stamps", doc_of(s)?.stamps.len() as u64);
            Ok(())
        }),
        key(n, egui::Key::Num4),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the Sticker view on the image stamp", |s, _| {
            if s.view_mode != ViewMode::Sticker || the_sticker(s)?.is_some() {
                return Err(format!("view {:?}", s.view_mode));
            }
            Ok(())
        }),
        // (1) Marquee over the white corner, then the Delete key.
        Step::Setup("remember the undo count", remember_undo),
        key(n, egui::Key::M),
        Step::Dyn("marquee drag over the top-left corner", |s, _| canvas_drag(s, &[[-6.0, -6.0], [30.0, 30.0], [60.0, 60.0]])),
        Step::WaitIdle,
        Step::Assert("a selection of the corner; no entry, still an image stamp", |s, sc| {
            if sel(s, 30, 30)? != 255 || sel(s, 100, 100)? != 0 || the_sticker(s)?.is_some() {
                return Err("the corner is not selected (or something else is)".into());
            }
            undo_since(s, sc, 0)
        }),
        key(n, egui::Key::Delete),
        Step::WaitIdle,
        Step::Assert("Delete: the corner's pixels go; one entry; the stamp stays, selected", |s, sc| {
            entry(s, sc, crate::sticker::commands::DELETE_SELECTED_LABEL, 1)?;
            let (corner, white) = (px(s, 0, 20, 20)?, px(s, 0, 100, 20)?);
            if corner[3] != 0 || white != [255, 255, 255, 255] {
                return Err(format!("corner {corner:?}, beside it {white:?}"));
            }
            if doc_of(s)?.stamps.len() as u64 != remembered(sc, "stamps")? || s.view_mode != ViewMode::Sticker {
                return Err("the stamp or the view changed".into());
            }
            if session_of(s)?.selected_stamp != Some(the_stamp(s)?.id) {
                return Err("the stamp is no longer selected".into());
            }
            changed_since(s, sc, "start")
        }),
        Step::Shot {
            name: "cutout_delete",
            expect: "canvas: the orange disc on white with the top-left corner square transparent (checkerboard) and a dashed \
                     outline around it; the sphere shows the picture with that corner missing; the Tool tab shows the Marquee \
                     options (New / Add / Subtract / Intersect, Rectangle / Ellipse, Feather, Fixed aspect)",
        },
        // (2) Ctrl+D, the wand on the white, Fill, undo.
        key(egui::Modifiers::COMMAND, egui::Key::D),
        Step::WaitIdle,
        Step::Assert("Ctrl+D: no selection", |s, _| if sel(s, 30, 30)? == 0 { Ok(()) } else { Err("still selected".into()) }),
        key(n, egui::Key::W),
        Step::Dyn("wand click on the white", |s, _| canvas_click(s, [240.0, 128.0])),
        Step::WaitIdle,
        Step::Assert("the wand selected the white, not the disc nor the cleared corner", |s, _| {
            let (white, disc, corner) = (sel(s, 240, 128)?, sel(s, 128, 128)?, sel(s, 20, 20)?);
            if white != 255 || disc != 0 || corner != 0 {
                return Err(format!("white {white}, disc {disc}, corner {corner}"));
            }
            Ok(())
        }),
        Step::Setup("remember before the fill", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "deleted")
        }),
        key(n, egui::Key::G),
        Step::Do(AppAction::SetPrimaryColor(YELLOW)),
        Step::Dyn("Fill click on the disc (the selection is filled)", |s, _| canvas_click(s, [128.0, 128.0])),
        Step::WaitIdle,
        Step::Assert("one \"Fill\" entry: the white is yellow, the disc is not", |s, sc| {
            entry(s, sc, "Fill", 1)?;
            let (white, disc) = (px(s, 0, 240, 128)?, px(s, 0, 128, 128)?);
            if !near(white, YELLOW, 2) || !near(disc, ORANGE, 2) {
                return Err(format!("white {white:?}, disc {disc:?}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo: the white is back", |s, sc| same_composite(s, sc, "deleted")),
        // (3) Ctrl+D, the Magic Eraser on the white, undo.
        key(egui::Modifiers::COMMAND, egui::Key::D),
        key(egui::Modifiers::SHIFT, egui::Key::E),
        Step::WaitIdle,
        Step::Dyn("Magic Eraser click on the white", |s, _| canvas_click(s, [240.0, 128.0])),
        Step::WaitIdle,
        Step::Assert("one \"Magic Eraser\" entry: the white is transparent, the disc opaque", |s, sc| {
            entry(s, sc, crate::sticker::tools::magic_eraser::MAGIC_ERASER_LABEL, 1)?;
            let (white, disc) = (px(s, 0, 240, 128)?, px(s, 0, 128, 128)?);
            if white[3] != 0 || disc[3] != 255 {
                return Err(format!("white {white:?}, disc {disc:?}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo: the white is back", |s, sc| same_composite(s, sc, "deleted")),
        // (4) Remove Background.
        Step::Do(AppAction::Sticker(StickerAction::RemoveBackground)),
        Step::WaitIdle,
        Step::Assert("one \"Remove Background\" entry: corner alpha 0, disc alpha 255, no white halo", |s, sc| {
            entry(s, sc, "Remove Background", 1)?;
            let (corner, far, disc) = (px(s, 0, 2, 2)?, px(s, 0, 250, 250)?, px(s, 0, 128, 128)?);
            if corner[3] != 0 || far[3] != 0 || disc[3] != 255 {
                return Err(format!("corner {corner:?}, far corner {far:?}, disc {disc:?}"));
            }
            let (h, edge) = halo(s)?;
            if edge < 100 || h > HALO_BOUND {
                return Err(format!("halo {h:.1} over {edge} edge pixels (bound {HALO_BOUND})"));
            }
            Ok(())
        }),
        Step::Setup("remember the cut-out", |s, sc| {
            remember_undo(s, sc)?;
            let (h, _) = halo(s)?;
            sc.values.insert("halo_x10", (h * 10.0).round() as u64);
            remember_composite(s, sc, "cut")
        }),
        Step::Shot {
            name: "cutout_removed",
            expect: "canvas: the orange disc alone on the checkerboard (no white around it, no white rim); the sphere shows the \
                     disc without the white square",
        },
        // (5) The original picture as a second layer; Color to Alpha through its popover.
        Step::Dyn("the original picture as a second layer", |_, sc| {
            let blob = wte_core::BlobId(u32::try_from(remembered(sc, "blob")?).map_err(|e| e.to_string())?);
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::AddPictureLayer(Some(blob))))])
        }),
        Step::WaitIdle,
        Step::Do(AppAction::Sticker(StickerAction::OpenPopover(PopKind::ColorToAlpha { key: [255, 255, 255] }))),
        // P7b check: the preview is drawn on a job; the idle check waits for it.
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Assert("the Color to Alpha popover shows its live preview; nothing applied", |s, sc| {
            let shows = s.sticker_ui.popover.as_ref().is_some_and(|p| p.shows_picture);
            if !shows {
                return Err("no preview in the canvas".into());
            }
            undo_since(s, sc, 1)
        }),
        Step::Shot {
            name: "cutout_color_to_alpha",
            expect: "canvas: the Color to Alpha window at the canvas's top-left (colour button, White / Black / Primary, Apply, \
                     Cancel) and the preview: the second layer's white gone, its disc slightly see-through over the first",
        },
        Step::Dyn("click Apply", |s, _| click_panel(s, PanelHit::StickerPopoverApply)),
        Step::WaitIdle,
        Step::Assert("one \"Color to Alpha\" entry on the second layer: white -> alpha 0, the disc partly opaque", |s, sc| {
            entry(s, sc, "Color to Alpha", 2)?;
            if s.sticker_ui.popover.is_some() {
                return Err("the popover is still open".into());
            }
            let (white, disc) = (px(s, 1, 10, 10)?, px(s, 1, 128, 128)?);
            if white[3] != 0 || !(200..=240).contains(&disc[3]) {
                return Err(format!("white {white:?}, disc {disc:?}"));
            }
            Ok(())
        }),
        // (6) Defringe on the first layer.
        Step::Dyn("make the first layer active", |s, _| {
            let id = the_sticker(s)?.ok_or("not a sticker")?.layers[0].id;
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::SelectLayer(id)))])
        }),
        Step::Setup("remember before the defringe", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::Defringe(2))),
        Step::WaitIdle,
        Step::Assert("Defringe: one entry (or nothing to change); the halo stays below its bound", |s, sc| {
            let changed = doc_of(s)?.undo_label() == Some("Defringe");
            if changed { undo_since(s, sc, 1)? } else { undo_since(s, sc, 0)? }
            let (h, _) = halo(s)?;
            if h > HALO_BOUND {
                return Err(format!("halo {h:.1}"));
            }
            Ok(())
        }),
        // (7) Blur inside a selection of the left half, through the popover.
        Step::Setup("remember the right half of the first layer", |s, sc| {
            remember_undo(s, sc)?;
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for y in 0..SIDE {
                for x in 136..SIDE {
                    h = wte_core::fnv1a64(h, &px(s, 0, x, y)?);
                }
            }
            sc.values.insert("right_half", h);
            Ok(())
        }),
        key(n, egui::Key::M),
        Step::Dyn("marquee over the left half", |s, _| canvas_drag(s, &[[-6.0, -6.0], [64.0, 128.0], [128.0, 262.0]])),
        Step::WaitIdle,
        Step::Do(AppAction::Sticker(StickerAction::OpenPopover(PopKind::Blur { radius: 4.0 }))),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("click Apply", |s, _| click_panel(s, PanelHit::StickerPopoverApply)),
        Step::WaitIdle,
        Step::Assert("one \"Blur\" entry: the left edge is soft, the right half unchanged", |s, sc| {
            entry(s, sc, "Blur", 1)?;
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for y in 0..SIDE {
                for x in 136..SIDE {
                    h = wte_core::fnv1a64(h, &px(s, 0, x, y)?);
                }
            }
            if h != remembered(sc, "right_half")? {
                return Err("the right half changed".into());
            }
            let edge = px(s, 0, 48, 128)?;
            if edge[3] == 0 || edge[3] == 255 {
                return Err(format!("the left edge is still hard: {edge:?}"));
            }
            Ok(())
        }),
        // (8) Move a selection: a marquee inside the disc dragged with the Move tool.
        Step::Setup("remember before the move", remember_undo),
        Step::Dyn("marquee inside the disc", |s, _| canvas_drag(s, &[[100.0, 100.0], [120.0, 120.0], [140.0, 140.0]])),
        Step::WaitIdle,
        key(n, egui::Key::V),
        Step::Dyn("Move drag of the selected pixels by 30 to the right", |s, _| canvas_drag(s, &[[120.0, 120.0], [135.0, 120.0], [150.0, 120.0]])),
        Step::WaitIdle,
        Step::Assert("one \"Move Selection\" entry: a new layer with the pixels 30 to the right, a hole in the first", |s, sc| {
            entry(s, sc, crate::sticker::tools::move_selection::MOVE_SELECTION_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("not a sticker")?;
            if d.layers.len() != 3 {
                return Err(format!("{} layers", d.layers.len()));
            }
            let new = &d.layers[1];
            let p = new.owned_picture().ok_or("no pixels")?;
            let t = new.transform.translate;
            let fits = (t - Vec2::new(130.0, 100.0)).abs().max_element() <= 1.0 && (40..=42).contains(&p.width) && (40..=42).contains(&p.height);
            if !fits {
                return Err(format!("the new layer is {}x{} at {:?}", p.width, p.height, new.transform.translate));
            }
            if px(s, 0, 110, 120)?[3] != 0 {
                return Err("no hole where the pixels were".into());
            }
            if sel(s, 160, 120)? != 255 || sel(s, 110, 120)? != 0 {
                return Err("the selection did not move with the pixels".into());
            }
            Ok(())
        }),
        key(egui::Modifiers::COMMAND, egui::Key::D),
        // (9) The ClassicSticker style on the stamp.
        Step::Setup("remember before the style", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::ApplyPreset(wte_core::StylePreset::ClassicSticker))),
        Step::WaitIdle,
        Step::Setup("the stamp has the die-cut style; the finished sticker: corner alpha 0, disc alpha 255", |s, sc| {
            undo_since(s, sc, 1)?;
            if !the_stamp(s)?.styles.any_on() {
                return Err("the stamp has no style".into());
            }
            let StampSource::Sticker { sticker } = the_stamp(s)?.source else { return Err("not a sticker".into()) };
            let pic = s.doc_mut().ok_or("no doc")?.sticker_picture(sticker, None, 1.0).map_err(|e| e.to_string())?;
            let (corner, disc) = (pic.get(2, 2), pic.get(128, 190));
            if corner[3] != 0 || disc[3] != 255 {
                return Err(format!("corner {corner:?}, disc {disc:?}"));
            }
            let (h, _) = halo(s)?;
            if h > HALO_BOUND {
                return Err(format!("halo {h:.1}"));
            }
            Ok(())
        }),
        Step::Shot {
            name: "cutout_die_cut",
            expect: "3D half: the sphere with the orange disc as a die-cut sticker (a white border following the disc's edge and \
                     a soft shadow, no white square), with the moved piece and the soft left edge; canvas: the disc on the \
                     checkerboard, three layer rows in the Layers tab",
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
