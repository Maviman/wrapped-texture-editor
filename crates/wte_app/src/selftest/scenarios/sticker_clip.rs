// Stickers P10b (STICKER_PLAN 7 "P10b", 6.3, 8.2; ARCHITECTURE.md 8.11, 8.17): the scenario `sticker_clip` - Copy and
// Paste through the in-memory clipboard of a self-test run, with **only `Step::Chord`** for the chords (Copy as the
// `Event::Copy` egui-winit makes, Paste as the key subclass's chord), never `Step::Key`. The sphere in the 3D view: a
// sticker file with stamp styles dropped on the model; Ctrl+C on that stamp (PNG + CF_DIBV5 + WTE.Sticker.v1); Ctrl+V
// (the sticker is the Stamp tool's source, nothing placed, the styles come along); a click places it; Paste at View
// Centre places one more at once; in the Sticker view a Marquee selection is copied (only its pixels), cut (a hole in
// the layer) and pasted as a new layer centred in the canvas; the clipboard's text pasted into the Text tool's content field with the chord, and
// text copied out of the field with the chord. Undoing everything brings the start back.

use super::sticker::doc_state_hash;
use super::*;
use crate::clip::ClipAction;
use crate::ole::clipboard::{self, ClipFormat};
use crate::ole::{Chord, DataKind, DropItem};
use crate::selftest::Target;

/// Drop and click points on the model, in units of the model's screen disc.
const ON_MODEL: [f32; 2] = [-0.25, -0.15];
const ON_MODEL_2: [f32; 2] = [0.35, 0.3];
/// The sticker's side, and the square of it the Marquee selects (artboard pixels).
const SIDE: u32 = 64;
const SEL: [f32; 4] = [20.0, 16.0, 44.0, 40.0];
/// The text put on the clipboard, and the text copied out of the field.
const CLIP_TEXT: &str = "Hello from the clipboard";
const OUT_TEXT: &str = "Copied out of WTE";

/// A 64 x 64 sticker: an orange disc on a transparent ground, with the Outline style as its stamp styles.
fn badge_wtes() -> Result<Vec<u8>, String> {
    let mut img = PixelBuf::new(SIDE, SIDE);
    let c = (SIDE as f32 - 1.0) / 2.0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            if d < 28.0 {
                img.set(x, y, [240, 140, 30, 255]);
            }
        }
    }
    let pixels = wte_core::PicturePixels::Owned { pixels: Arc::new(img), version: 0 };
    let sticker = wte_core::StickerDoc::from_picture("Badge", pixels, [SIDE, SIDE]);
    let styles = wte_core::StylePreset::Outline.styles([SIDE, SIDE]);
    wte_core::write_wtes(&sticker, Some(&styles), &wte_core::BlobStore::default(), None).map_err(|e| e.to_string())
}

/// Normalized 3D-view coordinates of a disc point of the model.
fn model_at(s: &AppState, p: [f32; 2]) -> Result<[f32; 2], String> {
    let (xf, c, r) = model_disc(s, ViewId::Main3D)?;
    Ok(norm(&xf, disc_point(c, r, p)))
}

/// The clipboard's formats, as a list.
fn formats(s: &AppState) -> Result<Vec<String>, String> {
    if !s.clipboard.is_memory() {
        return Err("a self-test run must use the in-memory clipboard".into());
    }
    s.clipboard.format_names()
}

/// The clipboard's PNG, decoded; its CF_DIBV5 must hold the same pixels.
fn clip_picture(s: &AppState) -> Result<PixelBuf, String> {
    let png = s.clipboard.read(ClipFormat::Png)?.ok_or("no PNG on the clipboard")?;
    let pixels = wte_core::decode_image(&png, Some("clip.png"), 16384).map_err(|e| e.to_string())?.pixels;
    let dib = s.clipboard.read(ClipFormat::DibV5)?.ok_or("no CF_DIBV5 on the clipboard")?;
    if crate::ole::formats::dib_to_pixels(&dib)? != pixels {
        return Err("the CF_DIBV5 does not hold the PNG's pixels".into());
    }
    Ok(pixels)
}

/// Layer-space point -> normalized canvas coordinates (the edited sticker's artboard).
fn cnorm(s: &AppState, p: Vec2) -> Result<[f32; 2], String> {
    let rect = s.sticker_canvas_rect.ok_or("the canvas is not laid out")?;
    let d = super::sticker::sticker_doc(s)?;
    let geom = crate::sticker::canvas::Geom { origin: Vec2::new(d.artboard.x0 as f32, d.artboard.y0 as f32), size: d.size() };
    let edit = session_of(s)?.sticker_edit.as_ref().ok_or("the Sticker view is not open")?;
    let pos = geom.to_screen(&edit.camera, rect, p);
    Ok([(pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height()])
}

fn stamps(s: &AppState) -> Result<u64, String> {
    Ok(doc_of(s)?.stamps.len() as u64)
}

/// The `sticker_clip` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_clip() -> Vec<Step> {
    let none = egui::Modifiers::NONE;
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember the start", |s, sc| {
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            if !formats(s)?.is_empty() {
                return Err("the in-memory clipboard is not empty at the start".into());
            }
            Ok(())
        }),
        // (1) A sticker with stamp styles, dropped on the model.
        Step::Dyn("drop the sticker on the model", |s, _| {
            let at = model_at(s, ON_MODEL)?;
            let payload = vec![DropItem::Data { name: "Badge.wtes".into(), kind: DataKind::Sticker, bytes: badge_wtes()? }];
            Ok(vec![Step::Drop { payload, target: ViewId::Main3D.into(), at }])
        }),
        Step::WaitIdle,
        Step::Setup("the sticker stamp is selected and styled; remember it", |s, sc| {
            let id = session_of(s)?.selected_stamp.ok_or("no stamp is selected")?;
            let stamp = doc_of(s)?.stamp(id).ok_or("gone")?;
            if stamp.name != "Badge" || !matches!(stamp.source, StampSource::Sticker { .. }) || stamp.styles.is_default() {
                return Err(format!("stamp {:?}, styled {}", stamp.name, !stamp.styles.is_default()));
            }
            sc.values.insert("badge", id.0);
            sc.values.insert("stamps", stamps(s)?);
            remember_undo(s, sc)
        }),
        // (2) Ctrl+C on the stamp: its finished picture with the styles, and the sticker itself.
        Step::Chord(Chord::Copy),
        Step::WaitIdle,
        Step::Assert("the clipboard holds PNG, CF_DIBV5 and WTE.Sticker.v1 of the styled stamp; no entry", |s, sc| {
            let names = formats(s)?;
            if names != ["PNG", "CF_DIBV5", "WTE.Sticker.v1"] {
                return Err(format!("formats {names:?}"));
            }
            let p = clip_picture(s)?;
            // The Outline style draws around the disc: the picture is larger than the 64-pixel sticker's disc needs
            // and has both transparent and opaque pixels.
            let (clear, solid) = p.data.as_chunks::<4>().0.iter().fold((0, 0), |(c, o), px| (c + usize::from(px[3] == 0), o + usize::from(px[3] == 255)));
            if p.width < SIDE / 2 || clear == 0 || solid == 0 {
                return Err(format!("{} x {} picture, {clear} clear, {solid} solid pixels", p.width, p.height));
            }
            let wtes = s.clipboard.read(ClipFormat::Sticker)?.ok_or("no sticker")?;
            let (pid, bytes) = crate::ole::formats::unwrap_sticker(&wtes).ok_or("not a WTE.Sticker.v1 block")?;
            let file = wte_core::read_wtes(bytes).map_err(|e| e.to_string())?;
            if pid != std::process::id() || file.sticker.name != "Badge" || file.styles.is_none() {
                return Err(format!("process {pid}, sticker {:?}, styles {}", file.sticker.name, file.styles.is_some()));
            }
            undo_since(s, sc, 0)
        }),
        // (3) Ctrl+V outside the Sticker view: the Stamp tool's source; nothing placed until a click.
        Step::Setup("forget the Stamp tool's styles", |s, _| {
            s.tools.stamp.styles = wte_core::StyleStack::default();
            Ok(())
        }),
        Step::Chord(Chord::Paste),
        Step::WaitIdle,
        Step::Assert("the pasted sticker is the Stamp tool's source with the stamp styles; nothing placed", |s, sc| {
            tool_is(s, ToolKind::Stamp)?;
            let crate::state::StampToolSource::Sticker(id) = s.tools.stamp.source else {
                return Err(format!("source {:?}", s.tools.stamp.source));
            };
            let d = doc_of(s)?.sticker(id).ok_or("no tray sticker")?;
            if d.name != "Badge" || !d.keep || s.tools.stamp.source_name != "Badge" {
                return Err(format!("tray sticker {:?} (kept {})", d.name, d.keep));
            }
            let badge = doc_of(s)?.stamp(StampId(remembered(sc, "badge")?)).ok_or("the first stamp is gone")?;
            if s.tools.stamp.styles != badge.styles {
                return Err("the Stamp tool did not get the copied stamp's styles".into());
            }
            if stamps(s)? != remembered(sc, "stamps")? {
                return Err("the paste placed a stamp".into());
            }
            undo_since(s, sc, 1)
        }),
        Step::Dyn("hover the model (the Stamp tool's outline)", |s, _| Ok(vec![Step::Hover { view: ViewId::Main3D.into(), at: model_at(s, ON_MODEL_2)? }])),
        Step::WaitFrames(2),
        Step::Shot {
            name: "clip_paste_source",
            expect: "3D view: the sphere with the orange disc sticker (a thin dark Outline style around it, still selected) \
                     on the upper left; the Stamp tool is active with \"Sticker: Badge\" and Style Outline in its options, \
                     its yellow placement outline drawn under the pointer on the lower right of the sphere; nothing new is \
                     placed yet",
        },
        // A click places it.
        Step::Dyn("click the model", |s, _| {
            Ok(vec![Step::Click { view: ViewId::Main3D.into(), at: model_at(s, ON_MODEL_2)?, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("the click placed one sticker stamp with the styles", |s, sc| {
            if stamps(s)? != remembered(sc, "stamps")? + 1 {
                return Err(format!("{} stamps", stamps(s)?));
            }
            let id = session_of(s)?.selected_stamp.ok_or("no stamp selected")?;
            let stamp = doc_of(s)?.stamp(id).ok_or("gone")?;
            let badge = doc_of(s)?.stamp(StampId(remembered(sc, "badge")?)).ok_or("gone")?;
            if !matches!(stamp.source, StampSource::Sticker { .. }) || stamp.styles != badge.styles {
                return Err("the placed stamp is not the styled sticker".into());
            }
            undo_since(s, sc, 2)
        }),
        // (4) Paste at View Centre: placed at once in the middle of the view.
        Step::Setup("remember before Paste at View Centre", |s, sc| {
            sc.values.insert("stamps", stamps(s)?);
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::Clip(ClipAction::PasteAtViewCentre)),
        Step::WaitIdle,
        Step::Assert("one more stamp, centred where the view's centre meets the model; one entry", |s, sc| {
            if stamps(s)? != remembered(sc, "stamps")? + 1 {
                return Err(format!("{} stamps", stamps(s)?));
            }
            let xf = view_xf(s, ViewId::Main3D)?;
            let session = session_of(s)?;
            let Surface::Model(hit) = xf.surface_at(&session.doc, session.active_set, xf.rect.center()) else {
                return Err("the view's centre is not on the model".into());
            };
            let id = session.selected_stamp.ok_or("no stamp selected")?;
            let Placement::Projected(pr) = doc_of(s)?.stamp(id).ok_or("gone")?.placement else { return Err("not projected".into()) };
            let off = (pr.origin - hit.position).length();
            if off > 1e-3 * doc_of(s)?.mesh.bounds.diagonal() {
                return Err(format!("centred {off} away from the view's centre"));
            }
            if doc_of(s)?.undo_label() != Some(crate::clip::PASTE_LABEL) {
                return Err(format!("undo label {:?}", doc_of(s)?.undo_label()));
            }
            undo_since(s, sc, 1)
        }),
        Step::Shot {
            name: "clip_view_centre",
            expect: "3D view: three orange disc stickers with thin dark outlines on the sphere - upper left, lower right and \
                     one in the middle of the view (overlapping the other two) with the Transform gizmo; three \"Badge\" 3D \
                     stamp rows in the Layers tab",
        },
        // (5) The Sticker view of the first stamp: a Marquee selection copied and pasted as a layer.
        Step::Setup("select the first stamp", |s, sc| {
            s.push(AppAction::SelectStamp(Some(StampId(remembered(sc, "badge")?))));
            Ok(())
        }),
        Step::WaitIdle,
        Step::Key(none, egui::Key::Num4),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Key(none, egui::Key::M),
        Step::Dyn("marquee over the middle of the disc", |s, _| {
            let (a, b) = (Vec2::new(SEL[0], SEL[1]), Vec2::new(SEL[2], SEL[3]));
            let path: Result<Vec<[f32; 2]>, String> = (0..=8).map(|k| cnorm(s, a + (b - a) * (k as f32 / 8.0))).collect();
            Ok(vec![Step::Pointer { view: Target::StickerCanvas, path: path?, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE, points_per_frame: 2 }])
        }),
        Step::WaitIdle,
        Step::Setup("a selection; remember the layers and the canvas centre", |s, sc| {
            let e = session_of(s)?.sticker_edit.as_ref().ok_or("no Sticker view")?;
            let b = e.selection.as_ref().ok_or("nothing is selected")?.bounds;
            sc.values.insert("sel_w", b.width() as u64);
            sc.values.insert("sel_h", b.height() as u64);
            sc.values.insert("layers", super::sticker::sticker_doc(s)?.layers.len() as u64);
            let centre = s.sticker_canvas_rect.ok_or("no canvas")?.center();
            let p = crate::sticker::canvas::layer_point(s, centre).ok_or("no canvas point")?;
            sc.values.insert("centre_x", p.x.to_bits() as u64);
            sc.values.insert("centre_y", p.y.to_bits() as u64);
            remember_undo(s, sc)
        }),
        Step::Chord(Chord::Copy),
        Step::WaitIdle,
        Step::Assert("the clipboard holds only the selected pixels (no sticker, no styles)", |s, sc| {
            if formats(s)? != ["PNG", "CF_DIBV5"] {
                return Err(format!("formats {:?}", formats(s)?));
            }
            let p = clip_picture(s)?;
            if (u64::from(p.width), u64::from(p.height)) != (remembered(sc, "sel_w")?, remembered(sc, "sel_h")?) {
                return Err(format!("{} x {} copied, the selection is {} x {}", p.width, p.height, remembered(sc, "sel_w")?, remembered(sc, "sel_h")?));
            }
            let centre = p.get(p.width / 2, p.height / 2);
            if centre != [240, 140, 30, 255] || p.data.as_chunks::<4>().0.iter().any(|px| px[3] > 0 && px[0] != 240) {
                return Err(format!("the copied pixels are not the disc's orange (centre {centre:?})"));
            }
            undo_since(s, sc, 0)
        }),
        // Ctrl+X: the same pixels again, and they leave the disc layer (one entry).
        Step::Chord(Chord::Cut),
        Step::WaitIdle,
        Step::Assert("Cut: the selected pixels on the clipboard, a hole in the disc; one entry", |s, sc| {
            let p = clip_picture(s)?;
            if (u64::from(p.width), u64::from(p.height)) != (remembered(sc, "sel_w")?, remembered(sc, "sel_h")?) {
                return Err(format!("{} x {} cut", p.width, p.height));
            }
            let d = super::sticker::sticker_doc(s)?;
            let pic = d.layers.first().and_then(|l| l.owned_picture()).ok_or("the disc layer has no edited pixels")?;
            let (x, y) = (((SEL[0] + SEL[2]) / 2.0) as u32, ((SEL[1] + SEL[3]) / 2.0) as u32);
            if pic.get(x, y)[3] != 0 || pic.get(4, 4) != [0, 0, 0, 0] || pic.get(10, 32)[3] != 255 {
                return Err(format!("inside the selection {:?}, outside {:?}", pic.get(x, y), pic.get(10, 32)));
            }
            if doc_of(s)?.undo_label() != Some(crate::sticker::commands::DELETE_SELECTED_LABEL) {
                return Err(format!("undo label {:?}", doc_of(s)?.undo_label()));
            }
            undo_since(s, sc, 1)
        }),
        Step::Chord(Chord::Paste),
        Step::WaitIdle,
        Step::Assert("a new top layer \"Pasted picture\" centred in the visible canvas; one entry", |s, sc| {
            let d = super::sticker::sticker_doc(s)?;
            let top = d.layers.last().ok_or("no layers")?;
            if d.layers.len() as u64 != remembered(sc, "layers")? + 1 || top.name != "Pasted picture" {
                return Err(format!("{} layers, top {:?}", d.layers.len(), top.name));
            }
            let (w, h) = (remembered(sc, "sel_w")? as f32, remembered(sc, "sel_h")? as f32);
            let want = Vec2::new(f32::from_bits(remembered(sc, "centre_x")? as u32), f32::from_bits(remembered(sc, "centre_y")? as u32));
            let centre = top.transform.translate + Vec2::new(w, h) * 0.5;
            if (centre - want).length() > 1.5 {
                return Err(format!("centred at {centre:?}, the canvas centre is {want:?}"));
            }
            undo_since(s, sc, 2)
        }),
        Step::Shot {
            name: "clip_sticker_layer",
            expect: "Sticker view: the orange disc with the cut square (dashed Marquee outline) and the pasted orange square \
                     (a new layer, framed, at X 20 Y 20, 24 x 24) in the middle of the canvas, covering the hole but for a \
                     transparent strip along its top; the Layers tab lists \"Pasted picture\" on top and active over \
                     \"Badge\"; the first sticker on the sphere on the right shows the same strip",
        },
        Step::Do(AppAction::Sticker(crate::sticker::StickerAction::Leave)),
        Step::WaitIdle,
        // (6) Text: the clipboard's text pasted into the Text tool's content field with the chord.
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Setup("text on the clipboard; the content field empty and focused", |s, sc| {
            clipboard::write_text(&mut *s.clipboard, CLIP_TEXT)?;
            s.tools.text.content.clear();
            s.focus_text_content = true;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
        Step::Chord(Chord::Paste),
        Step::WaitFrames(2),
        Step::Assert("the field holds the clipboard's text; nothing else happened", |s, sc| {
            if s.tools.text.content != CLIP_TEXT {
                return Err(format!("content {:?}", s.tools.text.content));
            }
            if s.drop_ui.busy() || s.tool != ToolKind::Text {
                return Err("the chord was taken as WTE's Paste".into());
            }
            undo_since(s, sc, 0)
        }),
        // Text copied out of the field with the chord.
        Step::Setup("new text in the field", |s, _| {
            s.tools.text.content = OUT_TEXT.to_owned();
            Ok(())
        }),
        Step::WaitFrames(2),
        Step::Key(egui::Modifiers::COMMAND, egui::Key::A),
        Step::WaitFrames(1),
        Step::Chord(Chord::Copy),
        Step::WaitFrames(2),
        Step::Assert("the field's text is on the clipboard (CF_UNICODETEXT)", |s, _| {
            let text = clipboard::read_text(&*s.clipboard)?;
            if text.as_deref() != Some(OUT_TEXT) || formats(s)? != ["CF_UNICODETEXT"] {
                return Err(format!("clipboard text {text:?}, formats {:?}", formats(s)?));
            }
            Ok(())
        }),
        Step::Shot {
            name: "clip_text",
            expect: "3D view with the three stickers on the sphere and the Text tool's yellow preview of \"Copied out of WTE\" \
                     beside it; the Text tool is active and its content field in the side panel shows \"Copied out of WTE\" \
                     selected (highlighted)",
        },
        Step::Key(none, egui::Key::Escape),
        Step::WaitIdle,
        // Undo everything: the start comes back.
        Step::Dyn("undo every change", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            if n != 6 {
                return Err(format!("{n} entries since the start, expected 6 (drop, paste, click, paste at centre, cut, layer)"));
            }
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undoing everything the document is as at the start", |s, sc| {
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? || !doc_of(s)?.stamps.is_empty() {
                return Err("the document differs from the start".into());
            }
            if doc_of(s)?.stickers().count() != 0 {
                return Err("tray stickers are left".into());
            }
            Ok(())
        }),
    ]
}
