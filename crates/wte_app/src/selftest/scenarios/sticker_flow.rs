// Stickers P13 (STICKER_PLAN 7 "P13", 8.2, 13.9 "P13"; ARCHITECTURE.md 8.11): the scenario `sticker_flow_path` - the
// user's whole sticker workflow on a real model given as PATH (the gate: ABeautifulGame.glb, DamagedHelmet.glb), in the
// real window, with the stub launcher (nothing is started) and files the scenario writes the way an editor's save lands.
//
// (1) A photo (an orange disc on white) dropped from Explorer onto the model: an image stamp at the drop point, one undo
// step. (2) The Sticker view (key 4): Remove Background (the white goes, the stamp becomes a sticker), a text layer
// "GO!"; Done. (3) The Classic sticker preset (die-cut contour + drop shadow) from the Sticker tab. (4) Wrap on surface.
// (5) Save As, then "Edit in <default editor>" from the Sticker tab (the default editor's wording: PhotoCraft is the
// default here) -> the PSD in `<run>_links\`, the stub saw it. (6) The sticker moved and scaled on the model in WTE: the
// link stays "linked" and the file is not touched (the user: "I would be able to manuver the sticker in the wrapped
// app"). (7) The editor saves the file with one more layer: the sticker updates in place (one step "Update Linked
// Sticker"), keeps the placement of (6), its styles, and WTE's text stays text. (8) A second picture dropped on the
// model; Copy Style on the first sticker, Paste Style on the second: the same styles, one entry. (9) A paint stroke,
// then "Edit Layer in <default editor>" on that paint layer; the editor paints a band and saves: one step "Update Linked
// Layer". (10) Save: neither linked file is rewritten. (11) Reopen: the stickers, styles, the wrap, both links
// ("linked") and every composite are back. (12) Export one layer, a selection of two and everything as one zip, each
// checked file by file against the document (the `export_flow` checks). No Error toast at any point.
// Each shot says what it must show.

use super::*;
use crate::link::{EditRequest, LinkAction, LinkTarget, testkit};
use crate::ole::DropItem;
use crate::selftest::Target;
use crate::sticker::{StickerAction, StyleTarget};
use wte_core::{StickerContent, StickerId, StylePreset};

/// The generated photo: an orange disc with a soft edge on opaque white.
const PHOTO: [u32; 2] = [256, 192];
const DISC_RADIUS: f32 = 70.0;
const ORANGE: [u8; 4] = [235, 120, 30, 255];
/// The second picture: a blue square on white.
const BADGE: [u32; 2] = [160, 160];

/// The paint layers' pixel hashes before the brush stroke of (9) (a scenario runs once per process).
static RASTERS: Mutex<Vec<(LayerId, u64)>> = Mutex::new(Vec::new());

/// Every paint layer of the document with a hash of its pixels.
fn raster_hashes(s: &AppState) -> Result<Vec<(LayerId, u64)>, String> {
    let doc = doc_of(s)?;
    Ok(doc
        .sets
        .iter()
        .flat_map(|set| set.layers.iter())
        .filter_map(|l| match &l.kind {
            LayerKind::Raster(buf) => Some((l.id, wte_core::testutil::hash_pixels(buf))),
            _ => None,
        })
        .collect())
}

fn project(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, ".wtep")
}

fn assets(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, "_assets")
}

/// The photo: the disc's edge falls off over about 1.5 pixels.
fn photo_png() -> Result<Vec<u8>, String> {
    let [w, h] = PHOTO;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut img = PixelBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let t = ((DISC_RADIUS + 0.75 - d) / 1.5).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (f32::from(a) * t + f32::from(b) * (1.0 - t)).round() as u8;
            img.set(x, y, [mix(ORANGE[0], 255), mix(ORANGE[1], 255), mix(ORANGE[2], 255), 255]);
        }
    }
    wte_core::encode_png(w, h, &img.data, PngLevel::Fast).map_err(|e| e.to_string())
}

fn badge_png() -> Result<Vec<u8>, String> {
    let [w, h] = BADGE;
    let mut img = PixelBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let inside = (24..w - 24).contains(&x) && (24..h - 24).contains(&y);
            img.set(x, y, if inside { [30, 90, 230, 255] } else { [255, 255, 255, 255] });
        }
    }
    wte_core::encode_png(w, h, &img.data, PngLevel::Fast).map_err(|e| e.to_string())
}

fn stamp_key(s: &AppState, sc: &Scratch, key: &str) -> Result<StampId, String> {
    let id = StampId(remembered(sc, key)?);
    doc_of(s)?.stamp(id).ok_or_else(|| format!("the stamp \"{key}\" is gone"))?;
    Ok(id)
}

fn layer_key(s: &AppState, sc: &Scratch, key: &str) -> Result<LayerId, String> {
    let id = LayerId(remembered(sc, key)?);
    doc_of(s)?.layer(id).ok_or_else(|| format!("the layer \"{key}\" is gone"))?;
    Ok(id)
}

/// The sticker stamp `key` shows.
fn sticker_of(s: &AppState, sc: &Scratch, key: &str) -> Result<StickerId, String> {
    match doc_of(s)?.stamp(stamp_key(s, sc, key)?).ok_or("no stamp")?.source {
        StampSource::Sticker { sticker } => Ok(sticker),
        ref other => Err(format!("the stamp \"{key}\" is not a sticker stamp: {other:?}")),
    }
}

fn sticker_doc<'a>(s: &'a AppState, sc: &Scratch, key: &str) -> Result<&'a wte_core::StickerDoc, String> {
    let id = sticker_of(s, sc, key)?;
    doc_of(s)?.sticker(id).map(|d| &**d).ok_or_else(|| "the sticker is gone".to_owned())
}

/// The kinds of a sticker's layers, bottom first.
fn kinds(d: &wte_core::StickerDoc) -> Vec<&'static str> {
    d.layers
        .iter()
        .map(|l| match l.content {
            StickerContent::Picture(_) => "picture",
            StickerContent::Text(_) => "text",
            StickerContent::Shape(_) => "shape",
        })
        .collect()
}

/// A hash of a stamp's stored placement without the depth range (an automatic depth refits to the styled frame).
fn placement_key(s: &AppState, id: StampId) -> Result<u64, String> {
    let p = match doc_of(s)?.stamp(id).ok_or("no stamp")?.placement {
        Placement::Projected(p) => Placement::Projected(Projector { depth_front: 0.0, depth_back: 0.0, ..p }),
        uv => uv,
    };
    Ok(wte_core::fnv1a64(0, format!("{p:?}").as_bytes()))
}

fn entry(s: &AppState, sc: &Scratch, label: &str, extra: u64) -> Result<(), String> {
    let got = doc_of(s)?.undo_label();
    if got != Some(label) {
        return Err(format!("the last entry is {got:?}, expected {label:?}"));
    }
    undo_since(s, sc, extra)
}

fn no_error_toast(s: &AppState) -> Result<(), String> {
    if s.toasts.error_count() == 0 { Ok(()) } else { Err(format!("error toasts: {:?}", s.toasts.error_texts())) }
}

/// The selected stamp is new (not one of the remembered ones) and named `name`; remembered under `key`.
fn remember_new_stamp(s: &AppState, sc: &mut Scratch, key: &'static str, name: &str) -> Result<(), String> {
    let id = session_of(s)?.selected_stamp.ok_or("no stamp is selected")?;
    if ["photo", "badge"].iter().any(|k| sc.values.get(*k) == Some(&id.0)) {
        return Err("no new stamp was placed".into());
    }
    let stamp = doc_of(s)?.stamp(id).ok_or("the selected stamp is gone")?;
    if stamp.name != name {
        return Err(format!("the stamp is named {:?}, expected {name:?}", stamp.name));
    }
    sc.values.insert(key, id.0);
    Ok(())
}

/// A hash of a file's bytes and its modification time.
fn file_key(path: &Path) -> Result<u64, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let time = std::fs::metadata(path).and_then(|m| m.modified()).map_err(|e| e.to_string())?;
    let nanos = time.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    Ok(wte_core::fnv1a64(wte_core::fnv1a64(0, &bytes), &nanos.to_le_bytes()))
}

fn sticker_file(s: &AppState, sc: &Scratch) -> Result<PathBuf, String> {
    crate::link::link_path(s, sticker_of(s, sc, "photo")?).ok_or_else(|| "the sticker has no link file".to_owned())
}

fn layer_file(s: &AppState, sc: &Scratch) -> Result<PathBuf, String> {
    crate::link::link_path(s, layer_key(s, sc, "paint")?).ok_or_else(|| "the paint layer has no link file".to_owned())
}

/// The badge of a link reads `want` (a Setup: the badge needs the document mutably for its caches).
fn badge_is(s: &mut AppState, id: crate::link::LinkId, want: &str) -> Result<(), String> {
    let got = crate::link::badge(s, id).map(|(_, t)| t).ok_or("not linked")?;
    if got == want { Ok(()) } else { Err(format!("the badge reads \"{got}\", expected \"{want}\"")) }
}

fn both_linked(s: &mut AppState, sc: &Scratch) -> Result<(), String> {
    let sticker = sticker_of(s, sc, "photo")?;
    let layer = layer_key(s, sc, "paint")?;
    badge_is(s, sticker.into(), "linked")?;
    badge_is(s, layer.into(), "linked")
}

fn remember_link(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    sc.values.insert("applied", s.link_ui.counters.applied);
    remember_undo(s, sc)
}

fn one_more_update(s: &AppState, sc: &Scratch) -> Result<bool, String> {
    Ok(s.link_ui.counters.applied == remembered(sc, "applied")? + 1)
}

fn texture(s: &AppState, id: LayerId) -> Result<[u32; 2], String> {
    doc_of(s)?.layer_texture_size(id).ok_or_else(|| "no texture".to_owned())
}

/// The editor paints rows `band` (fractions of the height) of the linked paint layer's file and saves it.
fn editor_paints_band(s: &AppState, sc: &mut Scratch, band: [f32; 2], rgba: [u8; 4]) -> Result<(), String> {
    let id = layer_key(s, sc, "paint")?;
    let path = layer_file(s, sc)?;
    let [w, h] = texture(s, id)?;
    let mut picture = wte_core::link::paint_source(&testkit::read(&path)?, [w, h], "photocraft").map_err(|e| format!("{e:?}"))?.pixels;
    for y in (band[0] * h as f32) as u32..(band[1] * h as f32) as u32 {
        for x in 0..w {
            picture.set(x, y, rgba);
        }
    }
    sc.values.insert("file_crc", u64::from(wte_core::link::paint::normalised_crc(&picture, w, h)));
    testkit::save_as_editor(&path, &wte_core::link::paint::layer_psd("Paint", &picture, w, h))
}

/// The two layers of the selection export: the photo sticker's layer and the badge's when they share a set (a
/// selection lies in one set), else the photo sticker's layer and the linked paint layer, else its set's first paint
/// layer.
fn flow_pair(s: &AppState, sc: &Scratch) -> Result<(LayerId, LayerId), String> {
    let doc = doc_of(s)?;
    let (a, b, paint) = (layer_key(s, sc, "photo_layer")?, layer_key(s, sc, "badge_layer")?, layer_key(s, sc, "paint")?);
    let set_of = |l: LayerId| doc.find_layer(l).map(|(set, _)| set);
    for (x, y) in [(a, b), (a, paint)] {
        if set_of(x).is_some() && set_of(x) == set_of(y) {
            return Ok((x, y));
        }
    }
    let set = doc.set(set_of(a).ok_or("the sticker's layer is gone")?).ok_or("no set")?;
    let raster = set.layers.iter().find(|l| l.is_raster()).ok_or("no paint layer in the sticker's set")?.id;
    log::warn!("sticker_flow_path: the stickers lie in different sets; the selection is the sticker and a paint layer");
    Ok((raster, a))
}

/// The layer of stamp `stamp` in its home set.
fn home_layer(s: &AppState, stamp: StampId) -> Result<LayerId, String> {
    let doc = doc_of(s)?;
    let home = doc.stamp(stamp).ok_or("the stamp is gone")?.home_set;
    doc.stamp_layers(stamp).into_iter().find(|(set, _)| *set == home).map(|(_, l)| l).ok_or_else(|| "the stamp has no layer in its home set".to_owned())
}

/// A hash of what the reopened project must give back: every composite, every stamp (source kind, placement, styles)
/// with its sticker's layers (name, kind, transform, styles) and the links.
fn flow_key(s: &AppState) -> Result<u64, String> {
    let doc = doc_of(s)?;
    let mut h = composite_hash(doc);
    for stamp in &doc.stamps {
        h = wte_core::fnv1a64(h, format!("{}|{:?}|{:?}", stamp.name, stamp.placement, stamp.styles).as_bytes());
        if let StampSource::Sticker { sticker } = stamp.source
            && let Some(d) = doc.sticker(sticker)
        {
            h = wte_core::fnv1a64(h, format!("{:?}|{:?}", d.artboard, kinds(d)).as_bytes());
            for l in &d.layers {
                h = wte_core::fnv1a64(h, format!("{}|{}|{:?}|{:?}|{:?}", l.name, l.visible, l.opacity, l.transform, l.styles).as_bytes());
            }
            h = wte_core::fnv1a64(h, format!("{:?}", doc.link(sticker).map(|l| (&l.file, &l.editor))).as_bytes());
        }
    }
    Ok(h)
}

/// Two drop points on large, well-facing areas of one texture set (a chess board, a helmet's shell), apart from each
/// other: the grid hit with the most neighbours (+-4 cells) on its own set that face the viewer (cos > 0.25), then by facing; the
/// second at least a fifth of the view's width away.
fn flat_spots(s: &AppState) -> Result<(ViewXform, egui::Pos2, egui::Pos2), String> {
    let (xf, hits) = model_hits(s)?;
    let at: std::collections::HashMap<(usize, usize), &ScreenHit> = hits.iter().map(|h| ((h.col, h.row), h)).collect();
    let score = |h: &ScreenHit| {
        let mut n = 0u32;
        for dy in -4i64..=4 {
            for dx in -4i64..=4 {
                let key = ((h.col as i64 + dx).max(0) as usize, (h.row as i64 + dy).max(0) as usize);
                if at.get(&key).is_some_and(|o| o.set == h.set && o.facing > 0.25) {
                    n += 1;
                }
            }
        }
        n as f32 + h.facing
    };
    let mut ranked: Vec<(f32, &ScreenHit)> = hits.iter().map(|h| (score(h), h)).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let first = ranked.first().ok_or("no hit on the model")?.1;
    let second = ranked
        .iter()
        .map(|(_, h)| *h)
        .find(|h| (h.pos - first.pos).length() >= xf.rect.width() * 0.2)
        .unwrap_or(ranked.last().ok_or("no hit")?.1);
    log::info!("sticker_flow_path: drop points {:?} (set {}) and {:?} (set {})", first.pos, first.set.0, second.pos, second.set.0);
    Ok((xf, first.pos, second.pos))
}

/// `sticker_flow_path` (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_flow_path(has_path: bool) -> Vec<Step> {
    if !has_path {
        return vec![Step::Assert("a PATH argument was given", |_, _| {
            Err("the sticker_flow_path scenario needs a model as PATH (e.g. ABeautifulGame.glb or DamagedHelmet.glb)".to_owned())
        })];
    }
    let mut steps = open_and_accept(true);
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        Step::Setup("the pictures in the run folder; the stub PhotoCraft as the default editor; the Sticker tab", |s, sc| {
            let dir = assets(sc);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("Photo.png"), photo_png()?).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("Badge.png"), badge_png()?).map_err(|e| e.to_string())?;
            let exe = with_suffix(&sc.out_stem, "_stub").join("photocraft.exe");
            std::fs::create_dir_all(exe.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
            std::fs::write(&exe, b"").map_err(|e| e.to_string())?;
            s.prefs.photocraft_exe = Some(exe);
            s.prefs.default_editor = Some("photocraft".to_owned());
            let links = wte_core::link::links_folder(&project(sc));
            if links.exists() {
                std::fs::remove_dir_all(&links).map_err(|e| e.to_string())?;
            }
            s.upper_tab = UpperTab::Sticker;
            s.tools.stamp.mode = SurfaceMode::Project;
            s.tools.stamp.size_frac = 0.15;
            s.tools.stamp.rotation_deg = 0.0;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
        // (1) The photo dropped from Explorer onto the model.
        Step::Dyn("drop Photo.png on the model", |s, sc| {
            let (xf, first, _) = flat_spots(s)?;
            Ok(vec![Step::Drop { payload: vec![DropItem::File(assets(sc).join("Photo.png"))], target: Target::View(ViewId::Main3D), at: norm(&xf, first) }])
        }),
        Step::WaitIdle,
        Step::Setup("one new image stamp \"Photo\" at the drop point; one entry", |s, sc| {
            remember_new_stamp(s, sc, "photo", "Photo")?;
            undo_since(s, sc, 1)?;
            if !matches!(doc_of(s)?.stamp(stamp_key(s, sc, "photo")?).ok_or("gone")?.source, StampSource::Image { .. }) {
                return Err("the dropped picture is not an image stamp".into());
            }
            no_error_toast(s)
        }),
        Step::Shot {
            name: "flow_dropped",
            expect: "3D view: the model with the photo (an orange disc on a white rectangle) where it was dropped, the \
                     Transform gizmo around it; the Sticker tab shows the image stamp \"Photo\"; status \"Undo: Add Stamp\"",
        },
        // (2) The Sticker view: Remove Background, a text layer.
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::Enter(None))),
        Step::WaitIdle,
        Step::Assert("the Sticker view is open on the photo", |s, _| {
            if s.view_mode == ViewMode::Sticker { Ok(()) } else { Err(format!("view {:?}", s.view_mode)) }
        }),
        Step::Do(AppAction::Sticker(StickerAction::RemoveBackground)),
        Step::WaitIdle,
        Step::Assert("one \"Remove Background\" entry: a sticker now; the white is transparent, the disc opaque", |s, sc| {
            entry(s, sc, "Remove Background", 1)?;
            let d = sticker_doc(s, sc, "photo")?;
            let p = d.layers.first().and_then(|l| l.owned_picture()).ok_or("the layer has no pixels")?;
            let (corner, centre) = (p.get(2, 2), p.get(PHOTO[0] / 2, PHOTO[1] / 2));
            if corner[3] != 0 || centre[3] != 255 {
                return Err(format!("corner {corner:?}, centre {centre:?}"));
            }
            Ok(())
        }),
        // The Text tool's content and the primary colour make the new text layer (the Text tab's field; the Sticker
        // tab stays in front, so no field takes the focus).
        Step::Setup("remember; the text \"GO!\" in a dark colour", |s, sc| {
            s.tools.text.content = "GO!".to_owned();
            s.colors.primary = [20, 20, 20, 255];
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::Sticker(StickerAction::AddTextLayer(None))),
        Step::WaitIdle,
        Step::Assert("one \"Add Text Layer\" entry: the photo and the text \"GO!\" above it", |s, sc| {
            entry(s, sc, crate::sticker::tools::text::ADD_TEXT_LABEL, 1)?;
            let d = sticker_doc(s, sc, "photo")?;
            if kinds(d) != ["picture", "text"] {
                return Err(format!("layers {:?}", kinds(d)));
            }
            let StickerContent::Text(spec) = &d.layers[1].content else { return Err("no text".into()) };
            if spec.content != "GO!" {
                return Err(format!("the text reads {:?}", spec.content));
            }
            if s.view_mode != ViewMode::Sticker {
                return Err(format!("the view is {:?}", s.view_mode));
            }
            no_error_toast(s)
        }),
        Step::WaitFrames(5),
        Step::Shot {
            name: "flow_sticker_view",
            expect: "Sticker view: on the canvas the orange disc on the checkerboard (no white around it) with the dark text \
                     \"GO!\" over it; the model on the right shows the same sticker; the Layers list has the two layers",
        },
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        Step::WaitIdle,
        // (3) The Classic sticker preset: the die-cut contour and the shadow.
        Step::Dyn("select the photo sticker", |s, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(stamp_key(s, sc, "photo")?)))])),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::ApplyPreset(StylePreset::ClassicSticker))),
        Step::WaitIdle,
        Step::Assert("one entry: the stamp has the white contour and the drop shadow", |s, sc| {
            undo_since(s, sc, 1)?;
            let st = &doc_of(s)?.stamp(stamp_key(s, sc, "photo")?).ok_or("gone")?.styles;
            if st.contour.is_none() || st.drop_shadow.is_none() {
                return Err(format!("styles {st:?}"));
            }
            Ok(())
        }),
        // (4) Wrap on surface.
        Step::Setup("remember", remember_undo),
        Step::Dyn("the sticker wraps on the surface", |s, sc| {
            let id = stamp_key(s, sc, "photo")?;
            let Placement::Projected(p) = doc_of(s)?.stamp(id).ok_or("gone")?.placement else { return Err("not a 3D stamp".into()) };
            let wrapped = Placement::Projected(Projector { mode: SurfaceMode::Wrap, wrap: WrapOptions::default(), ..p });
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, wrapped, Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Assert("one entry: the sticker wraps, its layer has ink; nothing pending", |s, sc| {
            undo_since(s, sc, 1)?;
            let id = stamp_key(s, sc, "photo")?;
            let Placement::Projected(p) = doc_of(s)?.stamp(id).ok_or("gone")?.placement else { return Err("not a 3D stamp".into()) };
            if p.mode != SurfaceMode::Wrap {
                return Err(format!("mode {:?}", p.mode));
            }
            if s.final_bakes_pending || s.wrap_jobs_pending || doc_of(s)?.has_unbaked_stamps() {
                return Err("a bake is still pending".into());
            }
            no_error_toast(s)
        }),
        Step::Setup("remember the photo sticker's layer", |s, sc| {
            let l = home_layer(s, stamp_key(s, sc, "photo")?)?;
            sc.values.insert("photo_layer", l.0);
            Ok(())
        }),
        Step::Shot {
            name: "flow_styled_wrapped",
            expect: "3D view: the orange disc with the dark \"GO!\" lying on the model's surface, a white die-cut border \
                     around the disc and a soft dark shadow; the Sticker tab: \"Sticker, 2 layers\", Die-cut contour and \
                     Drop shadow ticked",
        },
        // (5) Save As, then "Edit in <default editor>" from the Sticker tab.
        Step::Dyn("Save As into the run folder", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Assert("saved", |s, sc| {
            if doc_of(s)?.is_modified() {
                return Err("still modified".into());
            }
            file_non_empty(&project(sc))
        }),
        Step::WaitFrames(3),
        Step::Assert("the button names the default editor: \"Edit in PhotoCraft\"", |s, sc| {
            let label = crate::link::edit_in_label(s, Some(sticker_of(s, sc, "photo")?));
            let want = crate::link::editor::quiet_editor(s).map(|e| format!("Edit in {}", e.name())).ok_or("no default editor")?;
            if label != want || label != "Edit in PhotoCraft" {
                return Err(format!("the button reads \"{label}\", the default editor gives \"{want}\""));
            }
            panel_rect(s, PanelHit::LinkEditIn).map(|_| ())
        }),
        Step::Setup("remember", remember_undo),
        Step::Dyn("click Edit in PhotoCraft in the Sticker tab", |s, _| {
            let rect = panel_rect(s, PanelHit::LinkEditIn)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Setup("the link: the PSD in the links folder (two layers), the stub saw it, no entry; \"linked\"", |s, sc| {
            let path = sticker_file(s, sc)?;
            if path.parent() != Some(wte_core::link::links_folder(&project(sc)).as_path()) {
                return Err(format!("the file is at {}", path.display()));
            }
            let read = testkit::read(&path)?;
            if read.layers.len() != 2 {
                return Err(format!("{} layers in the file", read.layers.len()));
            }
            let calls = s.link_ui.launcher.calls();
            if calls.len() != 1 || calls[0].file != path || calls[0].editor != "photocraft" {
                return Err(format!("stub calls {calls:?}"));
            }
            undo_since(s, sc, 0)?;
            let id = sticker_of(s, sc, "photo")?;
            badge_is(s, id.into(), "linked")?;
            sc.values.insert("psd", file_key(&path)?);
            no_error_toast(s)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "flow_linked",
            expect: "Sticker tab: the photo sticker with \"Link: linked\", \"Editor: PhotoCraft\", the file name Photo_k<id>.psd \
                     and the buttons Edit in PhotoCraft, Edit in..., Update now, Send to PhotoCraft, Show file in Explorer, \
                     Unlink; the styled sticker on the model",
        },
        // (6) The sticker moved and scaled on the model: the link and the file stay as they are.
        Step::Setup("remember", remember_undo),
        Step::Dyn("move and scale the sticker on the model", |s, sc| {
            let id = stamp_key(s, sc, "photo")?;
            let Placement::Projected(p) = doc_of(s)?.stamp(id).ok_or("gone")?.placement else { return Err("not a 3D stamp".into()) };
            let moved = Projector { origin: p.origin + p.rotation * Vec3::X * (p.half_size.x * 0.15), half_size: p.half_size * 1.2, ..p };
            Ok(vec![Step::Do(AppAction::UpdateStampPlacement(id, Placement::Projected(moved), Commit::Final))])
        }),
        Step::WaitIdle,
        Step::Setup("one entry; the badge still reads \"linked\", the PSD is untouched, nothing was started", |s, sc| {
            undo_since(s, sc, 1)?;
            let id = sticker_of(s, sc, "photo")?;
            badge_is(s, id.into(), "linked")?;
            if file_key(&sticker_file(s, sc)?)? != remembered(sc, "psd")? {
                return Err("moving the sticker rewrote its PSD".into());
            }
            if s.link_ui.launcher.calls().len() != 1 {
                return Err(format!("stub calls {:?}", s.link_ui.launcher.calls()));
            }
            let key = placement_key(s, stamp_key(s, sc, "photo")?)?;
            sc.values.insert("moved", key);
            Ok(())
        }),
        // (7) The editor saves the file with one more layer: the sticker updates in place.
        Step::Setup("remember; the editor saves a red layer", |s, sc| {
            remember_link(s, sc)?;
            let path = sticker_file(s, sc)?;
            let psd = testkit::with_new_layer(&path, "red bar", 40, wte_core::IRect::from_xywh(40, 30, 120, 60))?;
            testkit::save_as_editor(&path, &psd)
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Setup("one \"Update Linked Sticker\": three layers, the text still text; the placement and styles kept", |s, sc| {
            entry(s, sc, "Update Linked Sticker", 1)?;
            let d = sticker_doc(s, sc, "photo")?;
            if kinds(d) != ["picture", "text", "picture"] || d.layers[2].name != "red bar" {
                return Err(format!("layers {:?} {:?}", kinds(d), d.layers.iter().map(|l| &l.name).collect::<Vec<_>>()));
            }
            if placement_key(s, stamp_key(s, sc, "photo")?)? != remembered(sc, "moved")? {
                return Err("the update moved the sticker on the model".into());
            }
            let st = &doc_of(s)?.stamp(stamp_key(s, sc, "photo")?).ok_or("gone")?.styles;
            if st.contour.is_none() || st.drop_shadow.is_none() {
                return Err(format!("the update lost the styles: {st:?}"));
            }
            let id = sticker_of(s, sc, "photo")?;
            badge_is(s, id.into(), "linked")?;
            no_error_toast(s)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "flow_updated",
            expect: "3D view: the styled sticker a little larger and moved to the side, now with a red bar at its upper \
                     left (partly over the disc), the white border around it too; the Sticker tab reads \"Sticker, 3 \
                     layers\" and \"Link: linked\"; the toast \"Updated from PhotoCraft\"",
        },
        // (8) A second picture; the style copied over.
        Step::Do(AppAction::SelectStamp(None)),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::Setup("remember", remember_undo),
        Step::Dyn("drop Badge.png on the model", |s, sc| {
            let (xf, _, second) = flat_spots(s)?;
            Ok(vec![Step::Drop { payload: vec![DropItem::File(assets(sc).join("Badge.png"))], target: Target::View(ViewId::Main3D), at: norm(&xf, second) }])
        }),
        Step::WaitIdle,
        Step::Setup("one new stamp \"Badge\"", |s, sc| {
            remember_new_stamp(s, sc, "badge", "Badge")?;
            undo_since(s, sc, 1)?;
            let l = home_layer(s, stamp_key(s, sc, "badge")?)?;
            sc.values.insert("badge_layer", l.0);
            Ok(())
        }),
        Step::Dyn("Copy Style on the photo sticker", |s, sc| {
            Ok(vec![
                Step::Do(AppAction::SelectStamp(Some(stamp_key(s, sc, "photo")?))),
                Step::Do(AppAction::Sticker(StickerAction::CopyStyle(StyleTarget::Stamps))),
            ])
        }),
        Step::WaitIdle,
        Step::Setup("remember", remember_undo),
        Step::Dyn("Paste Style on the badge", |s, sc| {
            Ok(vec![
                Step::Do(AppAction::SelectStamp(Some(stamp_key(s, sc, "badge")?))),
                Step::Do(AppAction::Sticker(StickerAction::PasteStyle(StyleTarget::Stamps))),
            ])
        }),
        Step::WaitIdle,
        Step::Assert("one \"Paste Style\" entry: the badge has the photo sticker's styles", |s, sc| {
            entry(s, sc, "Paste Style", 1)?;
            let doc = doc_of(s)?;
            let (a, b) = (&doc.stamp(stamp_key(s, sc, "photo")?).ok_or("gone")?.styles, &doc.stamp(stamp_key(s, sc, "badge")?).ok_or("gone")?.styles);
            if a != b || b.is_default() {
                return Err(format!("photo {a:?}, badge {b:?}"));
            }
            no_error_toast(s)
        }),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Shot {
            name: "flow_style_pasted",
            expect: "3D view: the second sticker, the blue square of Badge.png, with the same white die-cut border and \
                     shadow as the photo sticker, selected with the Transform gizmo; the Sticker tab shows \"Badge\" with \
                     Die-cut contour and Drop shadow ticked",
        },
        // (9) A paint stroke, then "Edit Layer in <default editor>" on that paint layer; the editor paints a band.
        Step::Do(AppAction::SelectStamp(None)),
        Step::Setup("every set's active layer is a paint layer", flow_paint_layers),
        Step::Setup("remember the paint layers and the history", |s, sc| {
            *RASTERS.lock().map_err(|e| e.to_string())? = raster_hashes(s)?;
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::BrushSizeStep(2)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("a brush stroke", |s, _| hit_stroke(s, 2)),
        Step::WaitIdle,
        Step::Setup("one entry; remember the painted layer", |s, sc| {
            undo_since(s, sc, 1)?;
            let before = RASTERS.lock().map_err(|e| e.to_string())?.clone();
            let changed: Vec<LayerId> = raster_hashes(s)?.into_iter().filter(|x| !before.contains(x)).map(|(id, _)| id).collect();
            // A 3D stroke may cross several texture sets (a chess board's pieces): the first changed layer is linked.
            let painted = *changed.first().ok_or("the stroke changed no paint layer")?;
            sc.values.insert("paint", painted.0);
            log::info!("sticker_flow_path: the stroke changed {changed:?}; {painted:?} is linked");
            Ok(())
        }),
        Step::Setup("remember", remember_undo),
        Step::Dyn("Edit Layer in PhotoCraft on the painted layer", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            let label = crate::link::ui::edit_layer_label(s, id);
            if label != "Edit Layer in PhotoCraft" {
                return Err(format!("the command reads \"{label}\""));
            }
            Ok(vec![Step::Do(AppAction::Link(LinkAction::EditIn(EditRequest::new(LinkTarget::Layer(id)))))])
        }),
        Step::WaitIdle,
        Step::Setup("the layer's PSD at the texture's size; the stub saw it; no entry; \"linked\"", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            let path = layer_file(s, sc)?;
            let read = testkit::read(&path)?;
            if read.layers.len() != 1 || read.size != texture(s, id)? {
                return Err(format!("{} layers, {:?}", read.layers.len(), read.size));
            }
            let calls = s.link_ui.launcher.calls();
            if calls.len() != 2 || calls[1].file != path || calls[1].editor != "photocraft" {
                return Err(format!("stub calls {calls:?}"));
            }
            undo_since(s, sc, 0)?;
            badge_is(s, id.into(), "linked")
        }),
        Step::Setup("remember; the editor paints a blue band", |s, sc| {
            remember_link(s, sc)?;
            editor_paints_band(s, sc, [0.45, 0.55], [30, 90, 230, 255])
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Setup("one \"Update Linked Layer\": the layer holds the file's picture; both links \"linked\"", |s, sc| {
            entry(s, sc, "Update Linked Layer", 1)?;
            let id = layer_key(s, sc, "paint")?;
            let [w, h] = texture(s, id)?;
            let p = doc_of(s)?.layer(id).ok_or("no layer")?.to_dense(w, h);
            if u64::from(wte_core::link::paint::normalised_crc(&p, w, h)) != remembered(sc, "file_crc")? {
                return Err("the layer does not hold the file's picture".into());
            }
            both_linked(s, sc)?;
            no_error_toast(s)
        }),
        // The linked layer is shown in the Layers tab (its set made active, the layer selected): its link block shows.
        Step::Dyn("show the linked paint layer in the Layers tab", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            let (set, _) = doc_of(s)?.find_layer(id).ok_or("the linked layer is gone")?;
            Ok(vec![Step::Do(AppAction::SetActiveSet(set)), Step::Do(AppAction::SelectLayer(id))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        // (10) Save: neither linked file is rewritten.
        Step::Setup("remember the linked files and the document", |s, sc| {
            sc.values.insert("psd", file_key(&sticker_file(s, sc)?)?);
            sc.values.insert("layer_psd", file_key(&layer_file(s, sc)?)?);
            sc.values.insert("flow_key", flow_key(s)?);
            Ok(())
        }),
        Step::Do(AppAction::Save),
        Step::WaitIdle,
        Step::Assert("saved; the linked files are not rewritten", |s, sc| {
            if doc_of(s)?.is_modified() {
                return Err("still modified".into());
            }
            if file_key(&sticker_file(s, sc)?)? != remembered(sc, "psd")? || file_key(&layer_file(s, sc)?)? != remembered(sc, "layer_psd")? {
                return Err("the project save rewrote a linked file".into());
            }
            Ok(())
        }),
        Step::Shot {
            name: "flow_saved",
            expect: "3D view: the styled photo sticker with the red bar and \"GO!\"; the badge where it was dropped (it may \
                     lie under the blue band); a red brush stroke across the model and a wide blue band of the linked paint \
                     layer; the Layers tab shows the linked layer's block (\"Link: linked\", \"Editor: PhotoCraft \u{b7} <layer>_l<id>.psd\" \
                     and its buttons) above the layer rows; the toast \"Saved\"",
        },
        // (11) Reopen.
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("the reopened project: the same stickers, styles, wrap, composites; both links \"linked\"", |s, sc| {
            let session = session_of(s)?;
            if session.path.as_deref() != Some(project(sc).as_path()) || session.doc.is_modified() {
                return Err(format!("the open document is {:?} (modified: {})", session.path, session.doc.is_modified()));
            }
            if flow_key(s)? != remembered(sc, "flow_key")? {
                return Err("the stickers, styles, links or composites differ after the reopen".into());
            }
            let d = sticker_doc(s, sc, "photo")?;
            if kinds(d) != ["picture", "text", "picture"] {
                return Err(format!("layers {:?}", kinds(d)));
            }
            let Placement::Projected(p) = doc_of(s)?.stamp(stamp_key(s, sc, "photo")?).ok_or("gone")?.placement else {
                return Err("not a 3D stamp".into());
            };
            if p.mode != SurfaceMode::Wrap {
                return Err(format!("mode {:?}", p.mode));
            }
            both_linked(s, sc)?;
            no_error_toast(s)
        }),
        Step::Shot {
            name: "flow_reopened",
            expect: "the same picture as flow_saved without the \"Saved\" toast: the wrapped photo sticker with its border, \
                     shadow, red bar and \"GO!\", the badge with the same style, the brush stroke, the blue band, and the \
                     linked layer's block \"Link: linked\" in the Layers tab",
        },
        // (12) Export one layer, a selection and everything as one zip.
        Step::Dyn("Export this layer: the photo sticker's layer", |s, sc| Ok(vec![Step::Do(AppAction::ShowExportLayers(vec![layer_key(s, sc, "photo_layer")?]))])),
        Step::WaitIdle,
        Step::Setup("into <out>_one", |s, sc| dialog_into_dir(s, with_suffix(&sc.out_stem, "_one"))),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("exactly the sticker's PNG(s), its layer's pixels, with ink", |s, sc| {
            assert_no_dialog(s, sc)?;
            let images = expect_layer_files(s, &with_suffix(&sc.out_stem, "_one"), &[layer_key(s, sc, "photo_layer")?])?;
            let n = ink(&images[0]);
            log::info!("sticker_flow_path: the sticker's file has {n} inked texels");
            if n == 0 { Err("the sticker's file is empty".into()) } else { Ok(()) }
        }),
        Step::Dyn("select two layers (Ctrl-click) and Export selected layers", |s, sc| {
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
        Step::Setup("into <out>_sel", |s, sc| dialog_into_dir(s, with_suffix(&sc.out_stem, "_sel"))),
        Step::Dyn("Export (the dialog's options)", run_dialog_export),
        Step::WaitIdle,
        Step::Assert("exactly the two layers' PNGs, each its layer, different", |s, sc| {
            assert_no_dialog(s, sc)?;
            let (a, b) = flow_pair(s, sc)?;
            let images = expect_layer_files(s, &with_suffix(&sc.out_stem, "_sel"), &[a, b])?;
            if images[0] == images[1] { Err("the two exported layers are the same picture".into()) } else { Ok(()) }
        }),
        Step::Dyn("export everything as one zip", |s, sc| {
            let dir = fresh_out_dir(sc, "_all")?;
            Ok(vec![Step::Do(AppAction::RunExport(ExportOptions { dir, package: wte_core::ExportPackage::Zip, ..export_options(s, sc)? }))])
        }),
        Step::WaitIdle,
        Step::Assert("one zip; its entries are the planned files and match the document", |s, sc| {
            assert_no_dialog(s, sc)?;
            let doc = doc_of(s)?;
            let opts = ExportOptions { dir: with_suffix(&sc.out_stem, "_all"), package: wte_core::ExportPackage::Zip, ..export_options(s, sc)? };
            let zip = wte_core::export::package_name(&doc.name);
            let listing = dir_names(&opts.dir)?;
            if listing != [zip.clone()] {
                return Err(format!("{} holds {listing:?}", opts.dir.display()));
            }
            let bytes = std::fs::read(opts.dir.join(&zip)).map_err(|e| e.to_string())?;
            let entries = wte_core::export::read_package(&bytes)?;
            let summary = verify_export(doc, &opts, &entries)?;
            log::info!("sticker_flow_path: {zip} {} bytes; {summary}", bytes.len());
            no_error_toast(s)
        }),
        Step::Do(AppAction::SetTool(ToolKind::Navigate)),
        Step::WaitIdle,
    ]);
    steps
}
