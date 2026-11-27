// Stickers P10a (STICKER_PLAN 7 "P10a", 6.2, 8.2; ARCHITECTURE.md 8.11, 8.17): the scenario `sticker_drop` - drops
// from another program, fed through a drop target object and WTE's own data object (`Step::DragHover`, `Step::Drop`,
// `ole::TestDrag`), never the OS. The sphere in the Split view: a PNG hovered over the model (the marker) and let go
// there (a stamp centred at the hit point, the Stamp tool's size, one undo step); a PNG on the 2D view (a UV stamp
// at that texel, +-2); a miss (the corner of the 3D view: nothing placed, the picture is the Stamp tool's); a `.wtes`
// file on the model (a sticker stamp, one undo step); three picture files at once (one stamp, two tray stickers, one
// undo step); a DIB dropped on the first stamp's row in the Layers list (its picture replaced); a virtual PNG file
// dropped on the sticker canvas (a layer at the point); a `.wtep` hover (the full "Drop to open" overlay) and
// DragLeave. Undoing everything brings the start back. Run it once more with WTE_OLE=1 (the window's own drop target
// installed as well): the result must be the same.

use super::sticker::doc_state_hash;
use super::*;
use crate::ole::{DataKind, DropItem};
use crate::selftest::Target;

/// The drop points on the model, in units of the model's screen disc.
const ON_MODEL: [f32; 2] = [0.15, -0.1];
const ON_MODEL_2: [f32; 2] = [-0.35, 0.25];
const ON_MODEL_3: [f32; 2] = [0.3, 0.4];
/// The drop point on the 2D view (normalized in the view).
const ON_TEXTURE: [f32; 2] = [0.35, 0.62];

/// A solid picture with a transparent border, as PNG bytes.
fn picture_png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut img = PixelBuf::new(w, h);
    for y in 2..h.saturating_sub(2) {
        for x in 2..w.saturating_sub(2) {
            img.set(x, y, [rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    wte_core::encode_png(w, h, &img.data, PngLevel::Fast).unwrap_or_default()
}

fn png_item(name: &str, rgb: [u8; 3]) -> DropItem {
    DropItem::Data { name: name.to_owned(), kind: DataKind::Png, bytes: picture_png(64, 40, rgb) }
}

/// The folder of the scenario's files (`<out_stem>_assets`).
fn assets(sc: &Scratch) -> PathBuf {
    let mut p = sc.out_stem.clone().into_os_string();
    p.push("_assets");
    PathBuf::from(p)
}

/// Normalized 3D-view coordinates of a disc point of the model.
fn model_at(s: &AppState, p: [f32; 2]) -> Result<[f32; 2], String> {
    let (xf, c, r) = model_disc(s, ViewId::Main3D)?;
    Ok(norm(&xf, disc_point(c, r, p)))
}

fn f32_value(sc: &Scratch, key: &str) -> Result<f32, String> {
    remembered(sc, key).map(|v| f32::from_bits(v as u32))
}

/// The hit under normalized 3D-view coordinates.
fn hit_at(s: &AppState, at: [f32; 2]) -> Result<wte_core::Hit, String> {
    let xf = view_xf(s, ViewId::Main3D)?;
    let pos = xf.rect.min + egui::vec2(at[0] * xf.rect.width(), at[1] * xf.rect.height());
    let session = session_of(s)?;
    match xf.surface_at(&session.doc, session.active_set, pos) {
        Surface::Model(hit) => Ok(hit),
        _ => Err(format!("nothing of the model at {at:?}")),
    }
}

/// The selected stamp, which must be new (not `old`) and come from a drop with `name`.
fn new_selected(s: &AppState, sc: &Scratch, name: &str) -> Result<StampId, String> {
    let id = session_of(s)?.selected_stamp.ok_or("no stamp is selected")?;
    let stamp = doc_of(s)?.stamp(id).ok_or("the selected stamp is gone")?;
    if sc.values.get("last_stamp") == Some(&id.0) {
        return Err("no new stamp was placed".into());
    }
    if stamp.name != name {
        return Err(format!("the stamp is named {:?}, expected {name:?}", stamp.name));
    }
    Ok(id)
}

fn remember_stamp(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    if let Some(id) = session_of(s)?.selected_stamp {
        sc.values.insert("last_stamp", id.0);
    }
    remember_undo(s, sc)
}

/// The `sticker_drop` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_drop() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::Split)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember the start; write the dropped files", |s, sc| {
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            remember_undo(s, sc)?;
            let dir = assets(sc);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            for (name, rgb) in [("one.png", [220, 40, 40]), ("two.png", [40, 200, 70]), ("three.png", [40, 90, 230])] {
                std::fs::write(dir.join(name), picture_png(48, 48, rgb)).map_err(|e| e.to_string())?;
            }
            let mut badge = PixelBuf::new(32, 32);
            for y in 4..28 {
                for x in 4..28 {
                    badge.set(x, y, [250, 200, 30, 255]);
                }
            }
            let pixels = wte_core::PicturePixels::Owned { pixels: Arc::new(badge), version: 0 };
            let sticker = wte_core::StickerDoc::from_picture("Badge", pixels, [32, 32]);
            let bytes = wte_core::write_wtes(&sticker, None, &wte_core::BlobStore::default(), None).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("Badge.wtes"), bytes).map_err(|e| e.to_string())
        }),
        // (1) A PNG hovered over the model: the marker; let go there: a stamp centred at the hit point.
        Step::Dyn("hover a picture over the model", |s, _| {
            let at = model_at(s, ON_MODEL)?;
            Ok(vec![Step::DragHover { payload: vec![png_item("Dropped picture.png", [230, 60, 40])], target: ViewId::Main3D.into(), at }])
        }),
        Step::WaitFrames(3),
        Step::Assert("the hover is known with its point and shows the marker", |s, _| {
            let hover = s.drop_ui.hover.as_ref().ok_or("no hover")?;
            if hover.pos.is_none() || !hover.kinds.picture {
                return Err(format!("{hover:?}"));
            }
            match crate::dnd::overlay_of(&hover.kinds, false) {
                crate::dnd::Overlay::Place => Ok(()),
                other => Err(format!("overlay {other:?}")),
            }
        }),
        Step::Shot {
            name: "drop_hover_picture",
            expect: "Split view: the 3D half framed in the accent colour with a small ring at the pointer on the sphere and \
                     the sentence \"Drop to place the picture here\" in a dark box beside it; the 2D half and the panels \
                     unchanged",
        },
        // The drag still hovering is the one let go (its payload).
        Step::Dyn("let the picture go on the model", |s, _| {
            let at = model_at(s, ON_MODEL)?;
            Ok(vec![Step::Drop { payload: Vec::new(), target: ViewId::Main3D.into(), at }])
        }),
        Step::WaitIdle,
        Step::Assert("a stamp centred at the drop point, the Stamp tool's size; one entry", |s, sc| {
            let id = new_selected(s, sc, "Dropped picture")?;
            let at = model_at(s, ON_MODEL)?;
            let hit = hit_at(s, at)?;
            let doc = doc_of(s)?;
            let Placement::Projected(pr) = doc.stamp(id).ok_or("gone")?.placement else { return Err("not projected".into()) };
            let diag = doc.mesh.bounds.diagonal();
            let off = (pr.origin - hit.position).length();
            if off > 1e-3 * diag {
                return Err(format!("the stamp is centred {off} away from the drop point"));
            }
            if (pr.half_size.max_element() - s.tools.stamp.size_frac * diag / 2.0).abs() > 1e-3 * diag {
                return Err(format!("half size {:?}", pr.half_size));
            }
            if let (Some(uv), Some(set)) = (hit.uv, hit.texture_set) {
                let size = doc.set(set).map_or([1, 1], |t| [t.width, t.height]);
                log::info!("sticker_drop: the stamp is centred at texel {:?} of set {}", uv * Vec2::new(size[0] as f32, size[1] as f32), set.0);
            }
            tool_is(s, ToolKind::Transform)?;
            if s.drop_ui.hover.is_some() {
                return Err("the hover is still shown after the drop".into());
            }
            undo_since(s, sc, 1)
        }),
        Step::Shot {
            name: "drop_model",
            expect: "Split view: a red rectangle with a transparent border on the sphere at the drop point with the \
                     Transform gizmo around it; the same picture on the texture in the 2D half; the Layers tab lists the \
                     new stamp layer \"Dropped picture\", selected",
        },
        Step::Setup("remember after the model drop", |s, sc| {
            let id = session_of(s)?.selected_stamp.ok_or("no stamp")?;
            sc.values.insert("first_stamp", id.0);
            remember_stamp(s, sc)
        }),
        // (2) A PNG on the 2D view: a UV stamp centred at the texel under the point.
        Step::Drop { payload: vec![png_item("flat.png", [40, 160, 230])], target: ViewId::Main2D.into(), at: ON_TEXTURE },
        Step::WaitIdle,
        Step::Assert("a UV stamp centred at the texel under the point (+-2); one entry", |s, sc| {
            // Picture data has no name of its own: the drop target names it "Dropped picture.png".
            let id = new_selected(s, sc, "Dropped picture")?;
            let xf = view_xf(s, ViewId::Main2D)?;
            let pos = xf.rect.min + egui::vec2(ON_TEXTURE[0] * xf.rect.width(), ON_TEXTURE[1] * xf.rect.height());
            let session = session_of(s)?;
            let Surface::Uv { px, .. } = xf.surface_at(&session.doc, session.active_set, pos) else {
                return Err("the drop point is not on the texture".into());
            };
            let Placement::Uv(u) = doc_of(s)?.stamp(id).ok_or("gone")?.placement else { return Err("not a UV stamp".into()) };
            if (u.center_px - px).abs().max_element() > 2.0 {
                return Err(format!("centred at {:?}, the drop texel is {px:?}", u.center_px));
            }
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember after the 2D drop", remember_stamp),
        // (3) A miss: the corner of the 3D view; nothing is placed, the picture is the Stamp tool's.
        Step::Drop { payload: vec![png_item("missed.png", [200, 200, 200])], target: ViewId::Main3D.into(), at: [0.03, 0.04] },
        Step::WaitIdle,
        Step::Assert("a miss places nothing; the picture is the Stamp tool's source", |s, sc| {
            if !matches!(s.tools.stamp.source, crate::state::StampToolSource::Blob(_)) || s.tools.stamp.source_name != "Dropped picture.png" {
                return Err(format!("source {:?} {:?}", s.tools.stamp.source, s.tools.stamp.source_name));
            }
            tool_is(s, ToolKind::Stamp)?;
            undo_since(s, sc, 0)
        }),
        // (4) A `.wtes` file on the model: a sticker stamp (the tray sticker and the stamp: one entry).
        Step::Dyn("drop the sticker file on the model", |s, sc| {
            let at = model_at(s, ON_MODEL_2)?;
            Ok(vec![Step::Drop { payload: vec![DropItem::File(assets(sc).join("Badge.wtes"))], target: ViewId::Main3D.into(), at }])
        }),
        Step::WaitIdle,
        Step::Assert("a sticker stamp showing the dropped sticker; one entry", |s, sc| {
            let id = new_selected(s, sc, "Badge")?;
            let doc = doc_of(s)?;
            let StampSource::Sticker { sticker } = doc.stamp(id).ok_or("gone")?.source else { return Err("not a sticker stamp".into()) };
            if doc.sticker(sticker).map(|d| d.layers.len()) != Some(1) {
                return Err("the sticker has not one layer".into());
            }
            if doc.undo_label() != Some("Drop") {
                return Err(format!("undo label {:?}", doc.undo_label()));
            }
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember after the sticker drop", remember_stamp),
        // (5) Three picture files at once on the model: the first is placed, two go to the tray.
        Step::Dyn("drop three files on the model", |s, sc| {
            let at = model_at(s, ON_MODEL_3)?;
            let dir = assets(sc);
            let files = ["one.png", "two.png", "three.png"].iter().map(|n| DropItem::File(dir.join(n))).collect();
            Ok(vec![Step::Drop { payload: files, target: ViewId::Main3D.into(), at }])
        }),
        Step::WaitIdle,
        Step::Assert("one stamp \"one\", \"two\" and \"three\" in the tray; one entry; the toast", |s, sc| {
            new_selected(s, sc, "one")?;
            let doc = doc_of(s)?;
            let tray: Vec<String> = doc.stickers().filter(|(_, d)| d.keep).map(|(_, d)| d.name.clone()).collect();
            if !(tray.contains(&"two".to_owned()) && tray.contains(&"three".to_owned())) {
                return Err(format!("tray {tray:?}"));
            }
            let toasts = s.toasts.visible_labels();
            if !toasts.iter().any(|t| t.starts_with("2 more added to the tray")) {
                return Err(format!("toasts {toasts:?}"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Shot {
            name: "drop_three",
            expect: "Split view: on the sphere the red dropped picture, the blue one is only in the 2D half's texture, the \
                     yellow Badge sticker and the new red-bordered \"one\" picture with the gizmo; a toast \"2 more added to \
                     the tray\"",
        },
        Step::Setup("remember after the three", remember_stamp),
        // (6) A DIB dropped on the first stamp's row of the Layers list: its picture is replaced.
        Step::Dyn("drop a DIB on the first stamp's row", |s, sc| {
            let doc = doc_of(s)?;
            let first = doc.stamp(StampId(remembered(sc, "first_stamp")?)).ok_or("the first stamp is gone")?;
            let layer = doc.stamp_layers(first.id).first().map(|(_, l)| *l).ok_or("the stamp has no layer")?;
            let row = panel_rect(s, PanelHit::LayerRow(layer))?;
            let mut dib = PixelBuf::new(30, 30);
            for px in dib.data.as_chunks_mut::<4>().0 {
                px.copy_from_slice(&[120, 20, 200, 255]);
            }
            let dib = crate::ole::formats::pixels_to_dibv5(&dib);
            let payload = vec![DropItem::Data { name: "Dropped picture.png".into(), kind: DataKind::Dib, bytes: dib }];
            Ok(vec![Step::Drop { payload, target: Target::Screen, at: [row.center().x, row.center().y] }])
        }),
        Step::WaitIdle,
        Step::Assert("the first stamp shows the new picture; one entry", |s, sc| {
            let doc = doc_of(s)?;
            let first = doc.stamp(StampId(remembered(sc, "first_stamp")?)).ok_or("the first stamp is gone")?;
            let StampSource::Image { blob } = first.source else { return Err("not an image stamp".into()) };
            let decoded = wte_core::decode_image(&doc.blobs.get(blob).ok_or("no blob")?.bytes, Some("x.png"), 64).map_err(|e| e.to_string())?;
            if decoded.pixels.width != 30 || decoded.pixels.data[..4] != [120, 20, 200, 255] {
                return Err(format!("the stamp's picture is {}x{}", decoded.pixels.width, decoded.pixels.height));
            }
            undo_since(s, sc, 1)
        }),
        Step::Setup("select the first stamp", |s, sc| {
            s.push(AppAction::SelectStamp(Some(StampId(remembered(sc, "first_stamp")?))));
            Ok(())
        }),
        Step::WaitIdle,
        // (7) A virtual PNG file dropped on the sticker canvas: a picture layer at the point.
        Step::Key(egui::Modifiers::NONE, egui::Key::Num4),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember in the Sticker view", |s, sc| {
            remember_undo(s, sc)?;
            let rect = s.sticker_canvas_rect.ok_or("the canvas is not laid out")?;
            let pos = rect.min + egui::vec2(rect.width() * 0.6, rect.height() * 0.4);
            let p = crate::sticker::canvas::layer_point(s, pos).ok_or("no canvas point")?;
            sc.values.insert("canvas_x", p.x.to_bits() as u64);
            sc.values.insert("canvas_y", p.y.to_bits() as u64);
            Ok(())
        }),
        Step::Drop {
            payload: vec![DropItem::Data { name: "badge.png".into(), kind: DataKind::File, bytes: picture_png(24, 24, [30, 220, 120]) }],
            target: Target::StickerCanvas,
            at: [0.6, 0.4],
        },
        Step::WaitIdle,
        Step::Assert("a second layer \"badge\" centred at the point; one entry (promotion included)", |s, sc| {
            let doc = super::sticker::sticker_doc(s)?;
            let top = doc.layers.last().ok_or("no layers")?;
            if doc.layers.len() != 2 || top.name != "badge" {
                return Err(format!("{} layers, top {:?}", doc.layers.len(), top.name));
            }
            let want = Vec2::new(f32_value(sc, "canvas_x")?, f32_value(sc, "canvas_y")?);
            let centre = top.transform.translate + Vec2::splat(12.0);
            if (centre - want).length() > 1.5 {
                return Err(format!("centred at {centre:?}, the point is {want:?}"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Shot {
            name: "drop_canvas",
            expect: "Sticker view: the canvas zoomed onto the 30 x 30 purple picture with a green square almost as large \
                     on it, its centre right of and above the picture's centre (reaching over the top edge), with the \
                     Move handles; the Layers tab lists two sticker layers, \"badge\" on top and active; the sphere on \
                     the right shows both",
        },
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
        // (8) A project hovered over the window: the full overlay; DragLeave clears it.
        Step::DragHover { payload: vec![DropItem::File(PathBuf::from(r"C:\Projects\other.wtep"))], target: ViewId::Main3D.into(), at: [0.5, 0.5] },
        Step::WaitFrames(3),
        Step::Assert("a project hover shows \"Drop to open\"", |s, _| {
            let hover = s.drop_ui.hover.as_ref().ok_or("no hover")?;
            match crate::dnd::overlay_of(&hover.kinds, false) {
                crate::dnd::Overlay::Open => Ok(()),
                other => Err(format!("overlay {other:?}")),
            }
        }),
        Step::Shot {
            name: "drop_hover_project",
            expect: "the whole window darkened with an accent frame, \"Drop to open\" in large letters in the middle and the \
                     list of file kinds below it",
        },
        Step::DragLeave,
        Step::WaitFrames(2),
        Step::Assert("the hover is gone after DragLeave; nothing was opened", |s, _| {
            if s.drop_ui.hover.is_some() || s.view_mode == ViewMode::Sticker {
                return Err("the hover is still there".into());
            }
            doc_of(s).map(|_| ())
        }),
        // Undo everything: the start comes back.
        Step::Dyn("undo every drop", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            if n != 6 {
                return Err(format!("{n} entries since the start, expected 6 (one per drop that changed the document)"));
            }
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).collect())
        }),
        Step::WaitIdle,
        Step::Assert("after undoing every drop the document is as at the start", |s, sc| {
            if doc_state_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            if !doc_of(s)?.stamps.is_empty() {
                return Err("stamps are left".into());
            }
            Ok(())
        }),
    ]
}
