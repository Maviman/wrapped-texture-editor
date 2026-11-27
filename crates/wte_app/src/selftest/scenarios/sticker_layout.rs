// Stickers P8 (STICKER_PLAN 7 "P8", 8.2; ARCHITECTURE.md 8.11, 8.15): the scenarios `sticker_text`, `sticker_layout`
// and `sticker_library`, in the real window with the tools on `Target::StickerCanvas`.
//
// - `sticker_text`: a transparent 512 x 256 picture placed as an image stamp; in the Sticker view the Text tool (X)
//   clicks a text layer "H I" (wide letter spacing), the Shape tool (U) drags a yellow star to the left and Ctrl+[
//   puts it behind the text, the text gets a stroke, a striped picture layer is added above the text and the footer's
//   Clip clips it to the text ("pattern inside text": a pixel between the two letters is transparent, one inside a
//   letter has the stripes' colour), the text is changed and warped (the actions the Text tab sends), then a text
//   stamp placed on the model is converted to a layered sticker from the Sticker tab.
// - `sticker_layout`: a 200 x 120 picture (an orange box with a transparent margin and a green mark) placed as an
//   image stamp; Crop (C, the Ellipse shape, Enter), Trim, Add Padding... (its popover's Apply), Rotate 90 CW and
//   back, Resize... 200 % (its popover), Flatten: the green mark stays on its texel through every step but the turn
//   (which moves it, and the turn back brings it back), each step is one undo step, and undoing everything gives the
//   start.
// - `sticker_library`: the library folder inside the run's output folder; a blank tray sticker with a text layer is
//   saved to the library, listed (its preview), a new document is opened, the library entry is added to its tray and
//   placed on the model with the Stamp tool.

use super::sticker::doc_state_hash;
use super::*;
use crate::selftest::Target;
use crate::sticker::StickerAction;
use crate::sticker::layout::Layout;
use crate::sticker::popover::PopKind;

/// The striped pattern's two colours.
const MAGENTA: [u8; 4] = [220, 30, 200, 255];
const CYAN: [u8; 4] = [20, 210, 230, 255];
const YELLOW: [u8; 4] = [250, 220, 30, 255];
const NAVY: [u8; 4] = [20, 30, 120, 255];
/// The layout picture and its green mark.
const LAYOUT_W: u32 = 200;
const LAYOUT_H: u32 = 120;
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// The stamp of the open view (or the selected stamp).
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
fn cnorm(s: &AppState, p: Vec2, size: [u32; 2]) -> Result<[f32; 2], String> {
    let rect = s.sticker_canvas_rect.ok_or("the canvas is not laid out")?;
    let geom = match the_sticker(s)? {
        Some(d) => crate::sticker::canvas::Geom { origin: Vec2::new(d.artboard.x0 as f32, d.artboard.y0 as f32), size: d.size() },
        None => crate::sticker::canvas::Geom { origin: Vec2::ZERO, size },
    };
    let edit = s.session.as_ref().and_then(|x| x.sticker_edit.as_ref()).ok_or("the Sticker view is not open")?;
    let pos = geom.to_screen(&edit.camera, rect, p);
    Ok([(pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height()])
}

fn canvas_drag(s: &AppState, a: [f32; 2], b: [f32; 2], size: [u32; 2]) -> Result<Vec<Step>, String> {
    let (a, b) = (Vec2::from_array(a), Vec2::from_array(b));
    let n = ((b - a).length() / 8.0).ceil().max(1.0) as usize;
    let mut path = Vec::new();
    for k in 0..=n {
        path.push(cnorm(s, a + (b - a) * (k as f32 / n as f32), size)?);
    }
    Ok(vec![Step::Pointer { view: Target::StickerCanvas, path, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE, points_per_frame: 2 }])
}

fn canvas_click(s: &AppState, p: [f32; 2], size: [u32; 2]) -> Result<Vec<Step>, String> {
    Ok(vec![Step::Click { view: Target::StickerCanvas, at: cnorm(s, Vec2::from_array(p), size)?, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
}

/// A click on a drawn panel control.
fn click_panel(s: &AppState, hit: PanelHit) -> Result<Vec<Step>, String> {
    let at = panel_rect(s, hit)?.center();
    Ok(vec![Step::ScreenClick { at, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
}

/// The last entry is `label` and there are `extra` more entries than remembered.
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

/// Registers `img` as an image blob and makes it the Stamp tool's picture.
fn register(state: &mut AppState, sc: &mut Scratch, img: &PixelBuf, name: &str) -> Result<wte_core::BlobId, String> {
    let png = wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).map_err(|e| e.to_string())?;
    let doc = state.doc_mut().ok_or("no session")?;
    let blob = doc.add_image_blob(name, Arc::from(png)).map_err(|e| e.to_string())?;
    state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
    state.tools.stamp.source_name = name.trim_end_matches(".png").to_owned();
    let _ = sc;
    Ok(blob)
}

fn start_steps(setup: fn(&mut AppState, &mut Scratch) -> Result<(), String>) -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("register the generated picture", setup),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the picture as an image stamp", |s, _| model_click(s, ViewId::Main3D, [0.0, -0.05])),
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
    ]
}

fn undo_all() -> Vec<Step> {
    vec![
        Step::Dyn("undo every step", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undoing everything: the start, an image stamp again", |s, sc| {
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            super::sticker::same_composite(s, sc, "start")
        }),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_text
// ---------------------------------------------------------------------------------------------------------------------

const TEXT_W: u32 = 512;
const TEXT_H: u32 = 256;

fn setup_text(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    // A transparent picture with a light grey 4-px frame (the middle stays transparent for the letter test).
    let mut img = PixelBuf::new(TEXT_W, TEXT_H);
    for y in 0..TEXT_H {
        for x in 0..TEXT_W {
            if x < 4 || y < 4 || x >= TEXT_W - 4 || y >= TEXT_H - 4 {
                img.set(x, y, [200, 200, 200, 255]);
            }
        }
    }
    register(state, sc, &img, "board.png")?;
    let mut stripes = PixelBuf::new(TEXT_W, TEXT_H);
    for y in 0..TEXT_H {
        for x in 0..TEXT_W {
            stripes.set(x, y, if (x / 16) % 2 == 0 { MAGENTA } else { CYAN });
        }
    }
    let png = wte_core::encode_png(TEXT_W, TEXT_H, &stripes.data, PngLevel::Fast).map_err(|e| e.to_string())?;
    let blob = state.doc_mut().ok_or("no session")?.add_image_blob("stripes.png", Arc::from(png)).map_err(|e| e.to_string())?;
    sc.values.insert("stripes", u64::from(blob.0));
    state.tools.text.content = "H I".to_owned();
    state.tools.text.letter_spacing = 0.3;
    state.colors.primary = NAVY;
    Ok(())
}

/// The text layer's id.
fn text_layer(s: &AppState) -> Result<wte_core::StickerLayerId, String> {
    let d = the_sticker(s)?.ok_or("not a sticker")?;
    d.layers.iter().find(|l| matches!(l.content, wte_core::StickerContent::Text(_))).map(|l| l.id).ok_or_else(|| "no text layer".to_owned())
}

/// Checks the clip: a pixel between the two letters is transparent and one well inside a letter has a stripe colour.
fn pattern_inside_text(s: &mut AppState) -> Result<(), String> {
    let StampSource::Sticker { sticker } = the_stamp(s)?.source else { return Err("not a sticker".into()) };
    let text = text_layer(s)?;
    let doc = s.doc_mut().ok_or("no document")?;
    let on = wte_core::StickerRef::Id(sticker);
    let alone = doc.sticker_layer_on_artboard(on, text).map_err(|e| e.to_string())?;
    let pic = doc.sticker_picture(sticker, None, 1.0).map_err(|e| e.to_string())?;
    // The row with the most ink of the text.
    let (w, h) = (alone.width, alone.height);
    let ink = |x: u32, y: u32| alone.get(x, y)[3];
    let row = (0..h).max_by_key(|&y| (0..w).filter(|&x| ink(x, y) > 0).count()).ok_or("no rows")?;
    let runs: Vec<(u32, u32)> = {
        let mut out = Vec::new();
        let mut start = None;
        for x in 0..=w {
            let on = x < w && ink(x, row) > 0;
            match (on, start) {
                (true, None) => start = Some(x),
                (false, Some(a)) => {
                    out.push((a, x));
                    start = None;
                }
                _ => {}
            }
        }
        out
    };
    let gap = runs.windows(2).map(|p| (p[0].1, p[1].0)).max_by_key(|(a, b)| b - a).ok_or(format!("one run only: {runs:?}"))?;
    if gap.1 - gap.0 < 12 {
        return Err(format!("the letters are too close: {runs:?}"));
    }
    let between = pic.get((gap.0 + gap.1) / 2, row);
    if between[3] != 0 {
        return Err(format!("between the letters {between:?} at ({}, {row})", (gap.0 + gap.1) / 2));
    }
    // Inside a letter: a pixel with a 3 x 3 neighbourhood of full ink (the letters are Ubuntu Light).
    let inside = (1..h - 1)
        .flat_map(|y| (1..w - 1).map(move |x| (x, y)))
        .find(|&(x, y)| (0..3).all(|dy| (0..3).all(|dx| ink(x + dx - 1, y + dy - 1) == 255)))
        .ok_or("no solid letter pixel")?;
    let c = pic.get(inside.0, inside.1);
    if !(near(c, MAGENTA, 4) || near(c, CYAN, 4)) {
        return Err(format!("inside a letter {c:?} at {inside:?}"));
    }
    Ok(())
}

/// `sticker_text` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_text() -> Vec<Step> {
    let n = egui::Modifiers::NONE;
    let size = [TEXT_W, TEXT_H];
    let mut steps = start_steps(setup_text);
    steps.extend([
        Step::Setup("remember the undo count", remember_undo),
        Step::Key(n, egui::Key::X),
        Step::Dyn("Text tool click in the middle", |s, _| canvas_click(s, [256.0, 150.0], [TEXT_W, TEXT_H])),
        Step::WaitIdle,
        Step::Assert("one \"Add Text Layer\" entry: the text \"H I\" in navy at the click, the active layer", |s, sc| {
            entry(s, sc, crate::sticker::tools::text::ADD_TEXT_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("not promoted")?;
            let l = d.layers.last().ok_or("no layer")?;
            let wte_core::StickerContent::Text(spec) = &l.content else { return Err("not a text layer".into()) };
            if spec.content != "H I" || spec.color != NAVY || (l.transform.translate - Vec2::new(256.0, 150.0)).abs().max_element() > 1.0 {
                return Err(format!("{:?} {:?} at {:?}", spec.content, spec.color, l.transform.translate));
            }
            if super::sticker::edit(s)?.active_layer != Some(l.id) {
                return Err("the text layer is not active".into());
            }
            Ok(())
        }),
        // The content field has the focus (typing goes into the text): Esc leaves it, so the next key is a tool key.
        Step::Assert("the new text layer's content field has the focus", |s, _| {
            if s.sticker_ui.text_edit.is_some() { Ok(()) } else { Err("the content field is not focused".into()) }
        }),
        Step::Key(n, egui::Key::Escape),
        Step::WaitIdle,
        Step::Assert("Esc left the field, the Sticker view stays", |s, _| {
            if s.view_mode == ViewMode::Sticker { Ok(()) } else { Err(format!("view {:?}", s.view_mode)) }
        }),
        // A star with the Shape tool on the left, then behind the text.
        Step::Setup("a yellow star", |s, sc| {
            s.sticker_ui.shape.kind = crate::sticker::tools::shape::ShapeChoice::Star;
            s.colors.primary = YELLOW;
            remember_undo(s, sc)
        }),
        Step::Key(n, egui::Key::U),
        Step::Dyn("Shape drag on the left", move |s, _| canvas_drag(s, [20.0, 40.0], [130.0, 150.0], [TEXT_W, TEXT_H])),
        Step::WaitIdle,
        Step::Key(egui::Modifiers::COMMAND, egui::Key::OpenBracket),
        Step::WaitIdle,
        Step::Assert("\"Add Shape Layer\" then the reorder: the star is below the text", |s, sc| {
            undo_since(s, sc, 2)?;
            let d = the_sticker(s)?.ok_or("no sticker")?;
            let kinds: Vec<&str> = d.layers.iter().map(|l| match l.content {
                wte_core::StickerContent::Picture(_) => "picture",
                wte_core::StickerContent::Text(_) => "text",
                wte_core::StickerContent::Shape(_) => "shape",
            }).collect();
            if kinds != ["picture", "shape", "text"] {
                return Err(format!("{kinds:?}"));
            }
            Ok(())
        }),
        // The text gets a stroke (the Tool tab's Layer styles send this action).
        Step::Dyn("select the text layer and give it a stroke", |s, _| {
            let id = text_layer(s)?;
            let styles = wte_core::StyleStack {
                stroke: Some(wte_core::Stroke { on: true, width: 2.0, position: wte_core::StrokePos::Outside, color: [255, 255, 255, 255], opacity: 1.0 }),
                ..wte_core::StyleStack::default()
            };
            Ok(vec![
                Step::Do(AppAction::Sticker(StickerAction::SelectLayer(id))),
                Step::Do(AppAction::Sticker(StickerAction::SetLayerStyles(id, Box::new(styles), Commit::Final))),
            ])
        }),
        Step::WaitIdle,
        // The stripes above the text, clipped to it with the footer's Clip.
        Step::Dyn("the stripes as a picture layer above the text", |_, sc| {
            let blob = wte_core::BlobId(u32::try_from(remembered(sc, "stripes")?).map_err(|e| e.to_string())?);
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::AddPictureLayer(Some(blob))))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember before the clip", remember_undo),
        Step::Dyn("click Clip in the Layers footer", |s, _| click_panel(s, PanelHit::StickerLayerClip)),
        Step::WaitIdle,
        Step::Assert("one \"Clip to Layer Below\" entry: the top layer is clipped", |s, sc| {
            entry(s, sc, crate::sticker::layout::CLIP_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("no sticker")?;
            if !d.layers.last().is_some_and(|l| l.clip) {
                return Err("the top layer is not clipped".into());
            }
            Ok(())
        }),
        Step::Setup("pattern inside text: a pixel between the letters is transparent, one inside a letter striped", |s, _| pattern_inside_text(s)),
        Step::Shot {
            name: "text_clipped",
            expect: "canvas: a yellow star on the left and the big letters \"H I\" filled with magenta / cyan stripes and a \
                     thin white outline, nothing between the letters; the sphere shows the same sticker; Layers: four rows, \
                     the top one with \"clipped\"",
        },
        // The text changed and warped (what the Text tab sends).
        Step::Setup("remember before the text edits", remember_undo),
        Step::Dyn("change the text to \"H I !\", then warp it with an arc", |s, _| {
            let id = text_layer(s)?;
            let d = the_sticker(s)?.ok_or("no sticker")?;
            let wte_core::StickerContent::Text(spec) = &d.layer(id).ok_or("no layer")?.content else { return Err("no text".into()) };
            let mut changed = (**spec).clone();
            changed.content = "H I !".to_owned();
            let mut warped = changed.clone();
            warped.fx.warp = Some(wte_core::Warp { bend: 0.5, ..wte_core::Warp::new(wte_core::WarpStyle::Arc) });
            Ok(vec![
                Step::Do(AppAction::Sticker(StickerAction::SetLayerText(id, Box::new(changed), Commit::Preview))),
                Step::Do(AppAction::Sticker(StickerAction::SetLayerText(id, Box::new(warped.clone()), Commit::Final))),
            ])
        }),
        Step::WaitIdle,
        Step::Assert("one \"Edit Sticker Text\" entry: the new text, warped, the name following; still clipped", |s, sc| {
            entry(s, sc, crate::sticker::tools::text::EDIT_TEXT_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("no sticker")?;
            let l = d.layer(text_layer(s)?).ok_or("no layer")?;
            let wte_core::StickerContent::Text(spec) = &l.content else { return Err("no text".into()) };
            if spec.content != "H I !" || spec.fx.warp.is_none() || l.name != "H I !" {
                return Err(format!("{:?} warp {:?} name {:?}", spec.content, spec.fx.warp, l.name));
            }
            if !d.layers.last().is_some_and(|l| l.clip) {
                return Err("the stripes are not clipped".into());
            }
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "text_warped",
            expect: "canvas: the striped letters \"H I !\" bent into an arc with the white outline, the yellow star on the \
                     left; the Tool tab shows the Sticker Shape options (the Shape tool is still chosen)",
        },
        // Done; a text stamp on the model, converted from the Sticker tab.
        Step::Dyn("click Done", |s, _| click_panel(s, PanelHit::StickerDone)),
        Step::WaitIdle,
        Step::Setup("the text tool's text", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            s.colors.primary = NAVY;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Dyn("place a text stamp", |s, _| model_click(s, ViewId::Main3D, [0.35, 0.3])),
        Step::WaitIdle,
        Step::Assert("the text stamp was placed and selected", |s, _| placed_stamp(s, true).map(|_| ())),
        Step::Setup("remember; the Sticker tab", |s, sc| {
            s.upper_tab = UpperTab::Sticker;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
        Step::Dyn("click Convert to layered sticker", |s, _| click_panel(s, PanelHit::StickerTabConvertText)),
        Step::WaitIdle,
        Step::Assert("one \"Convert to Sticker\" entry: a sticker stamp with one text layer \"WTE\"", |s, sc| {
            entry(s, sc, wte_core::CONVERT_LABEL, 1)?;
            let session = session_of(s)?;
            let id = session.selected_stamp.ok_or("no selected stamp")?;
            let StampSource::Sticker { sticker } = session.doc.stamp(id).ok_or("no stamp")?.source else {
                return Err("still a text stamp".into());
            };
            let d = session.doc.sticker(sticker).ok_or("no sticker")?;
            match d.layers.as_slice() {
                [l] if matches!(&l.content, wte_core::StickerContent::Text(t) if t.content == "WTE") => Ok(()),
                other => Err(format!("{} layers", other.len())),
            }
        }),
        Step::Shot {
            name: "text_converted",
            expect: "3D view: the sphere with the striped \"H I !\" sticker and the navy \"WTE\" text, unchanged by the \
                     conversion; the Sticker tab says \"Sticker, 1 layer\" for the selected stamp",
        },
        Step::Setup("back to the Tool tab", |s, _| {
            s.upper_tab = UpperTab::Tool;
            Ok(())
        }),
    ]);
    steps.extend(undo_all());
    let _ = size;
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_layout
// ---------------------------------------------------------------------------------------------------------------------

fn setup_layout(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let mut img = PixelBuf::new(LAYOUT_W, LAYOUT_H);
    for y in 20..100 {
        for x in 20..180 {
            img.set(x, y, [235, 130, 30, 255]);
        }
    }
    for y in 52..64 {
        for x in 124..136 {
            img.set(x, y, GREEN);
        }
    }
    register(state, sc, &img, "layout.png")?;
    Ok(())
}

/// The centroid (texels) of the green mark in the stamp's texture layers, and how many texels it covers.
fn mark(s: &AppState) -> Result<(Vec2, u64), String> {
    let doc = doc_of(s)?;
    let id = the_stamp(s)?.id;
    let (mut sum, mut n) = (Vec2::ZERO, 0u64);
    for (_, l) in doc.stamp_layers(id) {
        if let Some(LayerKind::Stamp { cache, .. }) = doc.layer(l).map(|l| &l.kind) {
            for part in cache.parts() {
                let w = part.rect.width() as usize;
                for (i, p) in part.data.as_chunks::<4>().0.iter().enumerate() {
                    if p[3] > 200 && p[1] > 200 && p[0] < 80 && p[2] < 80 {
                        sum += Vec2::new((part.rect.x0 + (i % w) as i32) as f32 + 0.5, (part.rect.y0 + (i / w) as i32) as f32 + 0.5);
                        n += 1;
                    }
                }
            }
        }
    }
    if n == 0 {
        return Err("the green mark is not on the model".into());
    }
    Ok((sum / n as f32, n))
}

fn remember_mark(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let (c, _) = mark(s)?;
    sc.values.insert("mark_x", u64::from(c.x.to_bits()));
    sc.values.insert("mark_y", u64::from(c.y.to_bits()));
    remember_undo(s, sc)
}

/// The mark is within 1.5 texels of where it was remembered (`stays`) or at least 4 texels away (`!stays`).
fn mark_at(s: &AppState, sc: &Scratch, stays: bool) -> Result<(), String> {
    let want = Vec2::new(f32::from_bits(remembered(sc, "mark_x")? as u32), f32::from_bits(remembered(sc, "mark_y")? as u32));
    let (c, _) = mark(s)?;
    let d = c.distance(want);
    match stays {
        true if d <= 1.5 => Ok(()),
        false if d >= 4.0 => Ok(()),
        _ => Err(format!("the mark is at {c:?}, {d:.2} texels from {want:?}")),
    }
}

/// `sticker_layout` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_layout() -> Vec<Step> {
    let n = egui::Modifiers::NONE;
    let mut steps = start_steps(setup_layout);
    steps.extend([
        Step::Setup("remember the mark", remember_mark),
        // (1) Crop to an ellipse: the Crop tool, the Ellipse shape, Enter.
        Step::Key(n, egui::Key::C),
        Step::Setup("the Ellipse shape", |s, _| {
            s.sticker_ui.crop_opts.shape = crate::sticker::tools::crop::CropShape::Ellipse;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "layout_crop_box",
            expect: "canvas: the orange box with the green mark, the crop box over the whole artboard with eight white handles \
                     and an ellipse inside it; the Tool tab shows the Crop options with Ellipse chosen and Apply crop",
        },
        Step::Key(n, egui::Key::Enter),
        Step::WaitIdle,
        Step::Assert("crop: one \"Crop Sticker\" entry (the image stamp promoted inside it), an ellipse; the mark stays", |s, sc| {
            entry(s, sc, wte_core::CROP_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("not promoted")?;
            if d.shape != wte_core::ArtboardShape::Ellipse {
                return Err(format!("{:?}", d.shape));
            }
            mark_at(s, sc, true)
        }),
        // (2) Trim, from the Sticker menu's list.
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::Layout(Layout::Trim(0)))),
        Step::WaitIdle,
        Step::Assert("trim: one entry, the artboard is the orange box; the mark stays", |s, sc| {
            entry(s, sc, wte_core::TRIM_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("no sticker")?;
            if d.size() != [160, 80] {
                return Err(format!("{:?}", d.artboard));
            }
            mark_at(s, sc, true)
        }),
        // (3) Add Padding... through its popover.
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::OpenPopover(PopKind::Padding { px: 10 }))),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("click Apply", |s, _| click_panel(s, PanelHit::StickerPopoverApply)),
        Step::WaitIdle,
        Step::Assert("padding: one entry, 10 px on every side; the mark stays", |s, sc| {
            entry(s, sc, wte_core::PADDING_LABEL, 1)?;
            let d = the_sticker(s)?.ok_or("no sticker")?;
            if d.size() != [180, 100] {
                return Err(format!("{:?}", d.artboard));
            }
            mark_at(s, sc, true)
        }),
        // (4) Rotate 90 CW, then back.
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::Layout(Layout::Rotate(90.0)))),
        Step::WaitIdle,
        Step::Assert("rotate 90: one entry, the sides swap; the mark turns with the sticker", |s, sc| {
            entry(s, sc, wte_core::ROTATE_LABEL, 1)?;
            if the_sticker(s)?.ok_or("no sticker")?.size() != [100, 180] {
                return Err("the sides did not swap".into());
            }
            mark_at(s, sc, false)
        }),
        Step::Shot {
            name: "layout_rotated",
            expect: "canvas: the orange ellipse standing upright (turned a quarter clockwise), the green mark below the middle; \
                     the sphere shows the sticker turned the same way",
        },
        Step::Do(AppAction::Sticker(StickerAction::Layout(Layout::Rotate(-90.0)))),
        Step::WaitIdle,
        Step::Assert("rotate 90 CCW: one more entry; the mark is back on its texel", |s, sc| {
            entry(s, sc, wte_core::ROTATE_LABEL, 2)?;
            mark_at(s, sc, true)
        }),
        // (5) Resize... 200 % through its popover.
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::OpenPopover(PopKind::Resize { w: 0, h: 0, keep: true }))),
        Step::WaitIdle,
        Step::Setup("200 %", |s, _| {
            let p = s.sticker_ui.popover.as_mut().ok_or("no popover")?;
            let PopKind::Resize { w, h, .. } = &mut p.kind else { return Err("not the Resize popover".into()) };
            *w *= 2;
            *h *= 2;
            Ok(())
        }),
        Step::WaitFrames(2),
        Step::Dyn("click Apply", |s, _| click_panel(s, PanelHit::StickerPopoverApply)),
        Step::WaitIdle,
        Step::Assert("resize 200 %: one entry, the artboard doubled, the placement kept; the mark stays", |s, sc| {
            entry(s, sc, wte_core::RESIZE_LABEL, 1)?;
            if the_sticker(s)?.ok_or("no sticker")?.size() != [360, 200] {
                return Err("not doubled".into());
            }
            mark_at(s, sc, true)
        }),
        // (6) Flatten.
        Step::Setup("remember before the flatten", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "before_flatten")
        }),
        Step::Do(AppAction::Sticker(StickerAction::Layout(Layout::Flatten))),
        Step::WaitIdle,
        Step::Assert("flatten: one entry, one picture layer; the model and the mark unchanged", |s, sc| {
            entry(s, sc, wte_core::FLATTEN_LABEL, 1)?;
            if the_sticker(s)?.ok_or("no sticker")?.layers.len() != 1 {
                return Err("not one layer".into());
            }
            mark_at(s, sc, true)
        }),
        Step::Shot {
            name: "layout_done",
            expect: "canvas: the orange ellipse with the green mark on a larger artboard (padding around it), one layer row; \
                     the sphere shows the orange ellipse with the green mark where it was at the start",
        },
    ]);
    steps.extend(undo_all());
    steps.extend([
        Step::Key(n, egui::Key::Escape),
        Step::WaitIdle,
        Step::Key(n, egui::Key::Escape),
        Step::WaitIdle,
    ]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_library
// ---------------------------------------------------------------------------------------------------------------------

fn library_dir(sc: &Scratch) -> PathBuf {
    let mut s = sc.out_stem.as_os_str().to_owned();
    s.push("_library");
    PathBuf::from(s)
}

fn library_files(sc: &Scratch) -> Vec<PathBuf> {
    std::fs::read_dir(library_dir(sc))
        .map(|it| it.filter_map(Result::ok).map(|e| e.path()).filter(|p| crate::dnd::extension_of(p) == "wtes").collect())
        .unwrap_or_default()
}

/// `sticker_library` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_library() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("the library folder inside the run's folder (empty); the Sticker tab", |s, sc| {
            let dir = library_dir(sc);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            s.prefs.sticker_library_dir = Some(dir);
            s.upper_tab = UpperTab::Sticker;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
        Step::Dyn("click New Sticker", |s, _| click_panel(s, PanelHit::TrayNew)),
        Step::WaitIdle,
        Step::Dyn("click Edit", |s, _| click_panel(s, PanelHit::TrayEdit)),
        Step::WaitIdle,
        Step::Setup("the text", |s, _| {
            s.tools.text.content = "LIB".to_owned();
            s.colors.primary = NAVY;
            Ok(())
        }),
        Step::Do(AppAction::Sticker(StickerAction::AddTextLayer(None))),
        Step::WaitIdle,
        Step::Dyn("click Done", |s, _| click_panel(s, PanelHit::StickerDone)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Dyn("click Save to Library", |s, _| click_panel(s, PanelHit::TraySaveLibrary)),
        Step::WaitIdle,
        Step::Assert("one .wtes file in the library folder; Save made no entry", |s, sc| {
            let files = library_files(sc);
            if files.len() != 1 {
                return Err(format!("{files:?}"));
            }
            undo_since(s, sc, 2)
        }),
        Step::Until("the library lists it", |s, _| Ok(!s.sticker_ui.library.busy() && s.sticker_ui.library.items.len() == 1)),
        Step::WaitFrames(3),
        Step::Shot {
            name: "library_saved",
            expect: "Sticker tab: the tray with the blank sticker (the navy \"LIB\" text) and, under Library, the folder path and \
                     one thumbnail of it",
        },
        // A new document (the unsaved one discarded): the library stays, the tray is empty.
        Step::Do(AppAction::ConfirmPending(Pending::OpenSample(SampleKind::Sphere))),
        Step::WaitIdle,
        Step::Setup("the Sticker tab; remember", |s, sc| {
            s.upper_tab = UpperTab::Sticker;
            remember_undo(s, sc)
        }),
        Step::Until("the library lists it again", |s, _| Ok(!s.sticker_ui.library.busy() && s.sticker_ui.library.items.len() == 1)),
        Step::Assert("a new document: no sticker in the tray", |s, _| {
            if crate::sticker::tray::tray_stickers(s).is_empty() { Ok(()) } else { Err("the tray is not empty".into()) }
        }),
        Step::WaitFrames(3),
        Step::Dyn("click the library entry", |s, _| click_panel(s, PanelHit::LibraryItem(0))),
        Step::WaitIdle,
        Step::Assert("one \"Add Sticker\" entry: a kept tray sticker with the text layer, the Stamp tool's source", |s, sc| {
            entry(s, sc, "Add Sticker", 1)?;
            let items = crate::sticker::tray::tray_stickers(s);
            let [id] = items.as_slice() else { return Err(format!("tray {items:?}")) };
            let d = doc_of(s)?.sticker(*id).ok_or("no sticker")?;
            if !d.keep || d.layers.len() != 2 || s.tools.stamp.source != crate::state::StampToolSource::Sticker(*id) {
                return Err(format!("keep {} layers {} source {:?}", d.keep, d.layers.len(), s.tools.stamp.source));
            }
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place it on the model", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.0])),
        Step::WaitIdle,
        Step::Assert("a sticker stamp of the library sticker on the model", |s, _| {
            let session = session_of(s)?;
            let id = session.selected_stamp.ok_or("no stamp placed")?;
            let StampSource::Sticker { sticker } = session.doc.stamp(id).ok_or("no stamp")?.source else { return Err("not a sticker stamp".into()) };
            if crate::sticker::tray::tray_stickers(s).first() != Some(&sticker) {
                return Err("another sticker".into());
            }
            placed_stamp_any(s)
        }),
        Step::Shot {
            name: "library_placed",
            expect: "3D view: the sphere with the white 512-px sticker with the navy \"LIB\" text placed in the middle; \
                     Sticker tab: the library entry and the tray item",
        },
    ]
}

/// The selected stamp has visible texels.
fn placed_stamp_any(s: &AppState) -> Result<(), String> {
    let session = session_of(s)?;
    let id = session.selected_stamp.ok_or("no stamp")?;
    let texels: usize = session
        .doc
        .stamp_layers(id)
        .into_iter()
        .filter_map(|(_, l)| session.doc.layer(l))
        .map(|l| match &l.kind {
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => {
                cache.parts().iter().map(|p| p.data.as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count()).sum()
            }
            _ => 0,
        })
        .sum();
    if texels == 0 { Err("the stamp shows nothing".into()) } else { Ok(()) }
}
