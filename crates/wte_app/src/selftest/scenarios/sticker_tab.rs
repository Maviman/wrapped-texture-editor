// Stickers P6b (STICKER_PLAN 7 "P6b", 8.2; ARCHITECTURE.md 8.11): the scenarios of the Sticker tab in the real window.
//
// - `sticker_styles`: an image stamp and a text stamp on the sphere; Classic sticker applied to both from the Sticker
//   tab (outside the Sticker view), a stroke ticked on the text; the linked layers grow, the stored placements stay,
//   no stamp is promoted, one entry each; undoing everything gives the start back.
// - `sticker_tray`: a tray sticker from a picture file, "Edit before placing" (a second layer, no question), placed
//   twice; an edit of one of them asks "Edit all / Only this one": Edit all moves both, Only this one parts them; one
//   entry per action, undo all / redo all; Save As, reopen, the same document.
// - `sticker_controls`: every new control of P6b clicked once through `ScreenClick` (the shared-picture question's
//   Cancel included).
// Each shot says what it must show.

use super::*;
use crate::state::{StampToolSource, StickerTarget};
use crate::sticker::{StickerAction, tray};

/// Brings control `hit` of the upper page into view (two wheel steps at most) and clicks it.
macro_rules! tap {
    ($name:literal, $hit:expr) => {
        [
            Step::Dyn(concat!("show ", $name), |s, _| reveal(s, $hit)),
            Step::WaitFrames(3),
            Step::Dyn(concat!("show ", $name), |s, _| reveal(s, $hit)),
            Step::WaitFrames(3),
            Step::Dyn(concat!("click ", $name), |s, _| hit_click(s, $hit)),
            Step::WaitIdle,
        ]
    };
}

/// A wheel step over the upper page that brings `hit` inside it (nothing when it is inside already).
pub(super) fn reveal(s: &AppState, hit: PanelHit) -> Result<Vec<Step>, String> {
    let rect = panel_rect(s, hit)?;
    let page = s.side_rects.upper_page;
    if !page.is_positive() {
        return Ok(Vec::new());
    }
    // Positive notches scroll up (the content moves down); egui moves 40 pt per notch (Line unit).
    let dy = if rect.top() < page.top() + 2.0 {
        rect.top() - page.top() - 12.0
    } else if rect.bottom() > page.bottom() - 2.0 {
        rect.bottom() - page.bottom() + 12.0
    } else {
        return Ok(Vec::new());
    };
    log::info!("reveal {hit:?}: {rect:?} in the page {page:?}: scroll {dy:.1} pt");
    Ok(vec![Step::ScreenScroll { at: page.center(), notches: (-dy / 40.0).clamp(-30.0, 30.0) }])
}

/// A click on the recorded rect of `hit`.
pub(super) fn hit_click(s: &AppState, hit: PanelHit) -> Result<Vec<Step>, String> {
    let rect = panel_rect(s, hit)?;
    Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
}

/// The stamp remembered under `key`.
fn stamp_at(s: &AppState, sc: &Scratch, key: &str) -> Result<StampId, String> {
    let id = StampId(remembered(sc, key)?);
    doc_of(s)?.stamp(id).ok_or_else(|| format!("the stamp \"{key}\" is gone"))?;
    Ok(id)
}

/// Texels with alpha above 0 in every linked layer of `stamp`.
fn ink(s: &AppState, stamp: StampId) -> Result<u64, String> {
    let doc = doc_of(s)?;
    let mut n = 0u64;
    for (_, l) in doc.stamp_layers(stamp) {
        if let Some(LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. }) = doc.layer(l).map(|l| &l.kind) {
            n += cache.parts().iter().map(|p| p.data.as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count() as u64).sum::<u64>();
        }
    }
    Ok(n)
}

/// A hash of the artwork box of `stamp`'s stored placement: everything but the depth range, which an automatic depth
/// refits to the bake footprint when styles draw outside the artwork (STICKER_PLAN 2.6 "Auto depth").
fn placement_hash(s: &AppState, stamp: StampId) -> Result<u64, String> {
    let p = match doc_of(s)?.stamp(stamp).ok_or("no stamp")?.placement {
        Placement::Projected(p) => Placement::Projected(Projector { depth_front: 0.0, depth_back: 0.0, ..p }),
        uv => uv,
    };
    Ok(wte_core::fnv1a64(0, format!("{p:?}").as_bytes()))
}

/// A hash of what the user sees of the document, independent of ids: the composite, every stamp (kind, placement,
/// tint, styles) with the layers of its sticker, and the tray's stickers.
pub(super) fn doc_hash(doc: &Document) -> u64 {
    let mut h = composite_hash(doc);
    let layers = |h: u64, d: &wte_core::StickerDoc| {
        d.layers.iter().fold(wte_core::fnv1a64(h, format!("{}|{:?}", d.name, d.artboard).as_bytes()), |h, l| {
            wte_core::fnv1a64(h, format!("{}|{}|{}|{:?}|{:?}|{:?}", l.name, l.visible, l.opacity, l.blend, l.transform, l.styles).as_bytes())
        })
    };
    for stamp in &doc.stamps {
        let kind = match stamp.source {
            StampSource::Image { .. } => "image",
            StampSource::Text(_) => "text",
            StampSource::Sticker { .. } => "sticker",
        };
        h = wte_core::fnv1a64(h, format!("{kind}|{:?}|{:?}|{:?}", stamp.placement, stamp.tint, stamp.styles).as_bytes());
        if let StampSource::Sticker { sticker } = stamp.source
            && let Some(d) = doc.sticker(sticker)
        {
            h = layers(h, d);
        }
    }
    for (id, d) in doc.stickers() {
        if d.keep || !doc.sticker_users(id).is_empty() {
            h = layers(wte_core::fnv1a64(h, b"tray"), d);
        }
    }
    h
}

/// The sticker a stamp shows.
fn sticker_of(s: &AppState, stamp: StampId) -> Result<wte_core::StickerId, String> {
    match doc_of(s)?.stamp(stamp).ok_or("no stamp")?.source {
        StampSource::Sticker { sticker } => Ok(sticker),
        ref other => Err(format!("not a sticker stamp: {other:?}")),
    }
}

/// The steps that place the sample logo as an image stamp at disc point `at` and remember it under `key`.
fn place_image(key: &'static str, at: [f32; 2]) -> Vec<Step> {
    let remember: fn(&mut AppState, &mut Scratch) -> Result<(), String> = match key {
        "image" => |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("image", id.0);
            Ok(())
        },
        _ => |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("image2", id.0);
            Ok(())
        },
    };
    let click: fn(&AppState, &Scratch) -> Result<Vec<Step>, String> =
        if at[0] < 0.0 { |s, _| model_click(s, ViewId::Main3D, [-0.25, -0.05]) } else if at[0] > 0.0 { |s, _| model_click(s, ViewId::Main3D, [0.25, -0.05]) } else { |s, _| model_click(s, ViewId::Main3D, [0.0, -0.05]) };
    vec![Step::Do(AppAction::SetTool(ToolKind::Stamp)), Step::Dyn("place the logo", click), Step::WaitIdle, Step::Setup("remember the placed stamp", remember)]
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_styles
// ---------------------------------------------------------------------------------------------------------------------

pub(super) fn sticker_styles() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("register the sample stamp image", setup_stamp_image),
    ];
    steps.extend(place_image("image", [0.0, 0.0]));
    steps.extend([
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Setup("the text WTE", |s, _| {
            s.tools.text.content = "WTE".to_owned();
            Ok(())
        }),
        Step::Dyn("place the text", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.45])),
        Step::WaitIdle,
        Step::Setup("remember the start", |s, sc| {
            let (text, _) = placed_stamp(s, true)?;
            sc.values.insert("text", text.0);
            let image = stamp_at(s, sc, "image")?;
            sc.values.insert("image_ink", ink(s, image)?);
            sc.values.insert("text_ink", ink(s, text)?);
            sc.values.insert("image_place", placement_hash(s, image)?);
            sc.values.insert("text_place", placement_hash(s, text)?);
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_hash(doc_of(s)?));
            remember_composite(s, sc, "start")?;
            remember_undo(s, sc)?;
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::Dyn("select the image stamp", |_, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(StampId(remembered(sc, "image")?))))])),
        Step::WaitFrames(3),
        Step::Assert("the Sticker tab (the fifth tab, second row) shows the image stamp", |s, _| {
            if s.upper_tab != UpperTab::Sticker || s.side_rects.sticker_tab.is_none() {
                return Err(format!("upper tab {:?}", s.upper_tab));
            }
            let b = s.side_rects.upper_tab_buttons;
            if !b[4].is_positive() || b[4].top() < b[0].bottom() {
                return Err(format!("the Sticker tab button {:?} is not in the second row ({b:?})", b[4]));
            }
            panel_rect(s, PanelHit::StylePresetButton(0)).map(|_| ())
        }),
    ]);
    // Classic sticker on the image stamp.
    steps.extend(tap!("Classic sticker", PanelHit::StylePresetButton(0)));
    steps.extend([
        Step::Assert("the image stamp has the Classic sticker style; not promoted; placement kept; it grew", |s, sc| {
            let image = stamp_at(s, sc, "image")?;
            let object = doc_of(s)?.stamp(image).ok_or("no stamp")?;
            if object.styles != wte_core::StylePreset::ClassicSticker.styles([STAMP_SIDE, STAMP_SIDE]) {
                return Err(format!("styles {:?}", object.styles));
            }
            if !matches!(object.source, StampSource::Image { .. }) || s.view_mode != ViewMode::View3D {
                return Err(format!("source {:?}, view {:?}", object.source, s.view_mode));
            }
            if placement_hash(s, image)? != remembered(sc, "image_place")? {
                return Err("the stored placement changed".into());
            }
            let (now, before) = (ink(s, image)?, remembered(sc, "image_ink")?);
            if now <= before + before / 20 {
                return Err(format!("the linked layer did not grow: {before} -> {now} texels"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember", remember_undo),
        Step::Shot {
            name: "styles_image",
            expect: "3D view: the logo on the sphere with a white die-cut border and a soft shadow, the text WTE below it \
                     still plain; side panel: the Sticker tab (second row of tabs, highlighted) with Selected stamp Logo \
                     (Image stamp), the preset row, Copy / Paste / Clear Style and the style blocks (Die-cut contour and \
                     Drop shadow ticked)",
        },
        Step::Dyn("select the text stamp", |_, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(StampId(remembered(sc, "text")?))))])),
        Step::WaitFrames(3),
    ]);
    steps.extend(tap!("Classic sticker", PanelHit::StylePresetButton(0)));
    steps.extend([
        Step::Assert("the text stamp has a contour and a shadow; still a text; placement kept; it grew", |s, sc| {
            let text = stamp_at(s, sc, "text")?;
            let object = doc_of(s)?.stamp(text).ok_or("no stamp")?;
            if !object.styles.contour.as_ref().is_some_and(|c| c.on) || !object.styles.drop_shadow.as_ref().is_some_and(|c| c.on) {
                return Err(format!("styles {:?}", object.styles));
            }
            if !matches!(object.source, StampSource::Text(_)) {
                return Err("the text stamp is no longer a text".into());
            }
            if placement_hash(s, text)? != remembered(sc, "text_place")? {
                return Err("the stored placement changed".into());
            }
            let (now, before) = (ink(s, text)?, remembered(sc, "text_ink")?);
            if now <= before + before / 20 {
                return Err(format!("the linked layer did not grow: {before} -> {now} texels"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember", remember_undo),
    ]);
    steps.extend(tap!("the Stroke box", PanelHit::StyleBlockToggle(1)));
    steps.extend([
        Step::Assert("a stroke on the text: one entry", |s, sc| {
            let text = stamp_at(s, sc, "text")?;
            let st = &doc_of(s)?.stamp(text).ok_or("no stamp")?.styles;
            if !st.stroke.as_ref().is_some_and(|x| x.on) {
                return Err(format!("no stroke: {st:?}"));
            }
            undo_since(s, sc, 1)
        }),
        Step::Shot {
            name: "styles_both",
            expect: "3D view: the logo and the text WTE both with a white die-cut border and a soft shadow, the text also \
                     with a dark stroke around the border; side panel: the Sticker tab with Selected stamp WTE (Text \
                     stamp), Die-cut contour, Stroke and Drop shadow ticked",
        },
        // Undo everything: the start.
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("after undoing the three style edits: the start, both stamps unstyled", |s, sc| {
            if doc_of(s)?.history.undo_len() as u64 != remembered(sc, "start_undo")? {
                return Err(format!("{} undo entries", doc_of(s)?.history.undo_len()));
            }
            if doc_hash(doc_of(s)?) != remembered(sc, "start_doc")? {
                return Err("the document differs from the start".into());
            }
            if composite_hash(doc_of(s)?) != remembered(sc, "start")? {
                return Err("the composite differs from the start".into());
            }
            Ok(())
        }),
    ]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_tray
// ---------------------------------------------------------------------------------------------------------------------

pub(super) fn sticker_tray() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("write the logo picture file, remember the start", |s, sc| {
            let dir = fresh_assets_dir(sc)?;
            let img = samples::sample_stamp_image(STAMP_SIDE);
            let png = wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).map_err(|e| e.to_string())?;
            write(&dir.join("badge.png"), &png)?;
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_hash(doc_of(s)?));
            remember_undo(s, sc)?;
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::Dyn("New Sticker from the picture file", |_, sc| {
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::NewStickerFromPicture(Some(assets_dir(sc).join("badge.png")))))])
        }),
        Step::WaitIdle,
        Step::Assert("one tray sticker \"badge\", the Stamp tool's source; one entry", |s, sc| {
            let items = tray::tray_stickers(s);
            let [id] = items.as_slice() else { return Err(format!("tray {items:?}")) };
            let d = doc_of(s)?.sticker(*id).ok_or("no sticker")?;
            if d.name != "badge" || !d.keep || s.tools.stamp.source != StampToolSource::Sticker(*id) {
                return Err(format!("{:?} keep {} source {:?}", d.name, d.keep, s.tools.stamp.source));
            }
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember; the Stamp tool and its options", |s, sc| {
            sc.values.insert("sticker", tray::tray_stickers(s)[0].0);
            remember_undo(s, sc)?;
            s.upper_tab = UpperTab::Tool;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitFrames(3),
    ];
    steps.extend(tap!("Edit before placing", PanelHit::StampEditBeforePlacing));
    steps.extend([
        Step::Assert("the Sticker view on the tray sticker; nothing placed; no new entry", |s, sc| {
            let edit = session_of(s)?.sticker_edit.as_ref().ok_or("the Sticker view is not open")?;
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            if s.view_mode != ViewMode::Sticker || edit.target != StickerTarget::Tray(id) || s.status_hint != crate::sticker::NOT_PLACED_HINT {
                return Err(format!("view {:?}, target {:?}, status {:?}", s.view_mode, edit.target, s.status_hint));
            }
            if !doc_of(s)?.stamps.is_empty() {
                return Err("a stamp was placed".into());
            }
            undo_since(s, sc, 0)
        }),
        Step::Setup("register the disc picture", |s, sc| {
            let png = super::sticker::disc_png()?;
            let blob = s.doc_mut().ok_or("no document")?.add_image_blob("disc.png", Arc::from(png)).map_err(|e| e.to_string())?;
            sc.values.insert("disc_blob", u64::from(blob.0));
            Ok(())
        }),
        Step::Dyn("add the disc as a second layer", |_, sc| {
            let blob = wte_core::BlobId(remembered(sc, "disc_blob")? as u32);
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::AddPictureLayer(Some(blob))))])
        }),
        Step::WaitIdle,
        Step::Assert("two layers, no question (no stamp shows it); one entry", |s, sc| {
            let d = doc_of(s)?.sticker(wte_core::StickerId(remembered(sc, "sticker")?)).ok_or("no sticker")?;
            if d.layers.len() != 2 || !matches!(s.dialog, Dialog::None) {
                return Err(format!("{} layers, dialog {:?}", d.layers.len(), s.dialog));
            }
            undo_since(s, sc, 1)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "tray_edit_before",
            expect: "Sticker view: the canvas shows the logo with the red disc in its middle, two rows (disc on top) in the \
                     Layers tab; the 3D half shows the plain sphere (nothing placed); status line: Not placed yet",
        },
        Step::Setup("remember", remember_undo),
        Step::Dyn("click Done", |s, _| hit_click(s, PanelHit::StickerDone)),
        Step::WaitIdle,
        Step::Assert("back in the 3D view with the Stamp tool and the sticker as its source", |s, sc| {
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            if s.view_mode != ViewMode::View3D || s.tools.stamp.source != StampToolSource::Sticker(id) {
                return Err(format!("view {:?}, source {:?}", s.view_mode, s.tools.stamp.source));
            }
            tool_is(s, ToolKind::Stamp)
        }),
    ]);
    steps.extend(place_image("image", [-1.0, 0.0]));
    steps.extend(place_image("image2", [1.0, 0.0]));
    steps.extend([
        Step::Assert("placed twice: two sticker stamps show the tray sticker; one entry each", |s, sc| {
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            let (a, b) = (stamp_at(s, sc, "image")?, stamp_at(s, sc, "image2")?);
            if doc_of(s)?.sticker_users(id) != vec![a, b] {
                return Err(format!("users {:?}", doc_of(s)?.sticker_users(id)));
            }
            undo_since(s, sc, 2)
        }),
        Step::Setup("remember; the Sticker tab", |s, sc| {
            remember_undo(s, sc)?;
            remember_composite(s, sc, "placed")?;
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::Shot {
            name: "tray_placed_twice",
            expect: "3D view: two copies of the logo with the red disc on the sphere, side by side (the right one selected \
                     in the Transform gizmo); side panel: the Sticker tab with Selected stamp badge (Sticker, 2 layers; \
                     its picture is shown by 2 stamps), the preset row and the style blocks, none ticked",
        },
        // Edit all.
        Step::Dyn("open the first stamp in the Sticker view", |_, sc| {
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::Enter(Some(StampId(remembered(sc, "image")?)))))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Do(AppAction::Sticker(StickerAction::Nudge(10, 0))),
        Step::WaitFrames(3),
        Step::Assert("the question: used by 2 stickers; nothing changed yet", |s, sc| {
            if !matches!(s.dialog, Dialog::StickerShared { users: 2 }) {
                return Err(format!("dialog {:?}", s.dialog));
            }
            panel_rect(s, PanelHit::SharedEditAll)?;
            undo_since(s, sc, 0)
        }),
        Step::Shot {
            name: "tray_question",
            expect: "a modal dialog Shared picture: This picture is used by 2 stickers., with the buttons Edit all, Only \
                     this one and Cancel, over the dimmed Sticker view (the dialog lies over the two stamps of the 3D \
                     half; the canvas with the logo and the disc stays visible on the left)",
        },
        Step::Dyn("click Edit all", |s, _| hit_click(s, PanelHit::SharedEditAll)),
        Step::WaitIdle,
        Step::Assert("Edit all: both stamps still share the sticker, its disc moved 10 px; one entry; the model changed", |s, sc| {
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            let (a, b) = (stamp_at(s, sc, "image")?, stamp_at(s, sc, "image2")?);
            if sticker_of(s, a)? != id || sticker_of(s, b)? != id || !matches!(s.dialog, Dialog::None) {
                return Err(format!("{:?} {:?}, dialog {:?}", sticker_of(s, a), sticker_of(s, b), s.dialog));
            }
            changed_since(s, sc, "placed")?;
            undo_since(s, sc, 1)
        }),
        Step::Setup("remember the shared state", |s, sc| {
            remember_undo(s, sc)?;
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            sc.values.insert("shared_hash", wte_core::fnv1a64(0, format!("{:?}", d.layers.iter().map(|l| l.transform).collect::<Vec<_>>()).as_bytes()));
            Ok(())
        }),
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        // Only this one.
        Step::Dyn("open the second stamp in the Sticker view", |_, sc| {
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::Enter(Some(StampId(remembered(sc, "image2")?)))))])
        }),
        Step::WaitIdle,
        Step::Do(AppAction::Sticker(StickerAction::Nudge(0, 12))),
        Step::WaitFrames(3),
        Step::Dyn("click Only this one", |s, _| hit_click(s, PanelHit::SharedOnlyThis)),
        Step::WaitIdle,
        Step::Assert("Only this one: the second stamp has its own sticker, the first one's is unchanged; two entries", |s, sc| {
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            let (a, b) = (stamp_at(s, sc, "image")?, stamp_at(s, sc, "image2")?);
            let own = sticker_of(s, b)?;
            if sticker_of(s, a)? != id || own == id {
                return Err(format!("{:?} {own:?}", sticker_of(s, a)));
            }
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            let first = wte_core::fnv1a64(0, format!("{:?}", d.layers.iter().map(|l| l.transform).collect::<Vec<_>>()).as_bytes());
            if first != remembered(sc, "shared_hash")? {
                return Err("the first stamp's sticker changed".into());
            }
            undo_since(s, sc, 2)
        }),
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        Step::WaitIdle,
        Step::Shot {
            name: "tray_only_this",
            expect: "3D view: both logos with their discs moved a little right, the right one's disc also moved down (each \
                     stamp now its own sticker); the Sticker tab shows badge (Sticker, 2 layers) without the shared line",
        },
        // Undo everything, redo everything.
        Step::Setup("remember the end", |s, sc| {
            sc.values.insert("end_doc", doc_hash(doc_of(s)?));
            sc.values.insert("end_undo", doc_of(s)?.history.undo_len() as u64);
            Ok(())
        }),
        Step::Dyn("undo everything", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).chain([Step::WaitIdle]).collect())
        }),
        Step::Assert("after undoing everything: the start", |s, sc| {
            if doc_hash(doc_of(s)?) != remembered(sc, "start_doc")? || !doc_of(s)?.stamps.is_empty() || !tray::tray_stickers(s).is_empty() {
                return Err("the document differs from the start".into());
            }
            Ok(())
        }),
        Step::Dyn("redo everything", |_, sc| {
            let n = remembered(sc, "end_undo")? - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Redo)).chain([Step::WaitIdle]).collect())
        }),
        Step::Assert("after redoing everything: the end", |s, sc| {
            if doc_hash(doc_of(s)?) != remembered(sc, "end_doc")? { Err("the document differs from the end".into()) } else { Ok(()) }
        }),
        // Save, reopen: the same.
        Step::Dyn("Save As the project", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Assert("saved", |s, sc| {
            if session_of(s)?.doc.is_modified() {
                return Err("the document is still modified".into());
            }
            file_non_empty(&with_suffix(&sc.out_stem, ".wtep"))
        }),
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Assert("the reopened project equals the saved one: two sticker stamps with their own stickers, the tray", |s, sc| {
            let doc = doc_of(s)?;
            if doc.history.undo_len() != 0 || doc.stamps.len() != 2 {
                return Err(format!("{} entries, {} stamps", doc.history.undo_len(), doc.stamps.len()));
            }
            let kinds: Vec<_> = doc.stamps.iter().map(|st| matches!(st.source, StampSource::Sticker { .. })).collect();
            if kinds != [true, true] || doc.stamps[0].source == doc.stamps[1].source {
                return Err(format!("sources {:?}", doc.stamps.iter().map(|st| &st.source).collect::<Vec<_>>()));
            }
            if tray::tray_stickers(s).len() != 2 {
                return Err(format!("tray {:?}", tray::tray_stickers(s)));
            }
            if doc_hash(doc) != remembered(sc, "end_doc")? { Err("the reopened document differs".into()) } else { Ok(()) }
        }),
        Step::Setup("the Sticker tab for the final screenshot", |s, _| {
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitFrames(3),
    ]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_controls
// ---------------------------------------------------------------------------------------------------------------------

pub(super) fn sticker_controls() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("register the sample stamp image", setup_stamp_image),
    ];
    steps.extend(place_image("image", [0.0, 0.0]));
    // The shared-picture question's Cancel: a second stamp of the same picture.
    steps.extend([
        Step::Setup("a second stamp of the same picture; the Sticker tab", |s, sc| {
            let image = stamp_at(s, sc, "image")?;
            let doc = s.doc_mut().ok_or("no document")?;
            let object = doc.stamp(image).ok_or("no stamp")?.clone();
            let Placement::Projected(p) = object.placement else { return Err("not projected".into()) };
            let moved = Projector { origin: p.origin + p.rotation * Vec3::X * (p.half_size.x * 2.5), ..p };
            let fitted = doc.fit_stamp_depth(&moved);
            let id = doc.add_stamp("Logo 2", object.source.clone(), Placement::Projected(fitted), object.home_set).map_err(|e| e.to_string())?;
            sc.values.insert("image2", id.0);
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitIdle,
        Step::Dyn("open the first stamp", |_, sc| Ok(vec![Step::Do(AppAction::Sticker(StickerAction::Enter(Some(StampId(remembered(sc, "image")?)))))])),
        Step::WaitIdle,
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::Nudge(4, 0))),
        Step::WaitFrames(3),
        Step::Dyn("click Cancel", |s, _| hit_click(s, PanelHit::SharedCancel)),
        Step::WaitIdle,
        Step::Assert("Cancel: no dialog, nothing changed, the stamp still an image stamp", |s, sc| {
            let image = stamp_at(s, sc, "image")?;
            if !matches!(s.dialog, Dialog::None) || !matches!(doc_of(s)?.stamp(image).ok_or("no stamp")?.source, StampSource::Image { .. }) {
                return Err(format!("dialog {:?}", s.dialog));
            }
            undo_since(s, sc, 0)
        }),
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        Step::Dyn("select the first stamp", |_, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(StampId(remembered(sc, "image")?))))])),
        Step::WaitFrames(3),
        Step::Setup("remember", remember_undo),
    ]);
    // The six presets, Copy / Paste / Clear: one entry each except Paste of the same styles.
    steps.extend(tap!("preset 1", PanelHit::StylePresetButton(0)));
    steps.extend(tap!("preset 2", PanelHit::StylePresetButton(1)));
    steps.extend(tap!("preset 3", PanelHit::StylePresetButton(2)));
    steps.extend(tap!("preset 4", PanelHit::StylePresetButton(3)));
    steps.extend(tap!("preset 5", PanelHit::StylePresetButton(4)));
    steps.extend(tap!("preset 6", PanelHit::StylePresetButton(5)));
    steps.push(Step::Assert("six presets: six entries, the last one Clear", |s, sc| {
        let image = stamp_at(s, sc, "image")?;
        if !doc_of(s)?.stamp(image).ok_or("no stamp")?.styles.is_default() {
            return Err("Clear left styles".into());
        }
        undo_since(s, sc, 6)
    }));
    steps.extend(tap!("preset 1 again", PanelHit::StylePresetButton(0)));
    steps.extend(tap!("Copy Style", PanelHit::StyleCopy));
    steps.extend(tap!("Paste Style", PanelHit::StylePaste));
    steps.extend(tap!("Clear Style", PanelHit::StyleClear));
    steps.push(Step::Assert("Copy remembered the styles; Paste of the same styles is no entry; Clear is one", |s, sc| {
        if s.style_clipboard.as_ref() != Some(&wte_core::StylePreset::ClassicSticker.styles([STAMP_SIDE, STAMP_SIDE])) {
            return Err(format!("clipboard {:?}", s.style_clipboard));
        }
        undo_since(s, sc, 8)
    }));
    steps.push(Step::Setup("remember", remember_undo));
    // Every style block's box.
    steps.extend(tap!("block 1", PanelHit::StyleBlockToggle(0)));
    steps.extend(tap!("block 2", PanelHit::StyleBlockToggle(1)));
    steps.extend(tap!("block 3", PanelHit::StyleBlockToggle(2)));
    steps.extend(tap!("block 4", PanelHit::StyleBlockToggle(3)));
    steps.extend(tap!("block 5", PanelHit::StyleBlockToggle(4)));
    steps.extend(tap!("block 6", PanelHit::StyleBlockToggle(5)));
    steps.extend(tap!("block 7", PanelHit::StyleBlockToggle(6)));
    steps.extend(tap!("block 8", PanelHit::StyleBlockToggle(7)));
    steps.extend(tap!("block 9", PanelHit::StyleBlockToggle(8)));
    steps.push(Step::Assert("nine boxes: every style on, nine entries, still an image stamp", |s, sc| {
        let image = stamp_at(s, sc, "image")?;
        let object = doc_of(s)?.stamp(image).ok_or("no stamp")?;
        let on = (0..9).filter(|i| crate::sticker::styles_ui::block_on(&object.styles, *i) == Some(true)).count();
        if on != 9 || !matches!(object.source, StampSource::Image { .. }) {
            return Err(format!("{on} styles on, source {:?}", object.source));
        }
        undo_since(s, sc, 9)
    }));
    steps.push(Step::Shot {
        name: "controls_tab",
        expect: "3D view: the logo with every style on (border, stroke, shadows, glows, overlays, bevel) beside a plain \
                 logo; side panel: the Sticker tab scrolled to the style blocks, all nine boxes ticked",
    });
    // Export Picture... without a path: no dialog in a self-test, nothing happens.
    steps.extend(tap!("Export Picture", PanelHit::StickerTabExport));
    steps.push(Step::Setup("remember", remember_undo));
    // Edit Sticker; in the view the Tool tab's Layer styles.
    steps.extend(tap!("Edit Sticker", PanelHit::StickerTabEdit));
    steps.extend([
        Step::Assert("the Sticker view is open on the stamp", |s, _| {
            if s.view_mode != ViewMode::Sticker { Err(format!("view {:?}", s.view_mode)) } else { Ok(()) }
        }),
        Step::Setup("the Tool tab", |s, _| {
            s.upper_tab = UpperTab::Tool;
            Ok(())
        }),
        Step::WaitFrames(3),
    ]);
    steps.extend(tap!("layer preset", PanelHit::StylePresetButton(1)));
    steps.push(Step::Dyn("Only this one", |s, _| if matches!(s.dialog, Dialog::StickerShared { .. }) { hit_click(s, PanelHit::SharedOnlyThis) } else { Ok(Vec::new()) }));
    steps.push(Step::WaitIdle);
    steps.extend(tap!("layer Stroke box", PanelHit::StyleBlockToggle(1)));
    steps.extend(tap!("layer Copy Style", PanelHit::StyleCopy));
    steps.extend([
        Step::Assert("the layer got the Outline preset (promoted, one entry) and the stroke toggled off (one entry)", |s, sc| {
            let image = stamp_at(s, sc, "image")?;
            let id = sticker_of(s, image)?;
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            if d.layers[0].styles.stroke.as_ref().map(|x| x.on) != Some(false) {
                return Err(format!("layer styles {:?}", d.layers[0].styles));
            }
            undo_since(s, sc, 2)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "controls_view",
            expect: "Sticker view: the logo on the canvas; the Tool tab shows Sticker Move options, the layer's X / Y and \
                     Layer styles with the preset row, Copy / Paste / Clear Style and the style blocks (Stroke unticked)",
        },
        Step::Dyn("click Done", |s, _| hit_click(s, PanelHit::StickerDone)),
        Step::WaitIdle,
        Step::Setup("the Sticker tab; remember", |s, sc| {
            s.upper_tab = UpperTab::Sticker;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
    ]);
    // The tray: New Sticker, its item, Duplicate, Delete, Export, From picture..., Edit.
    steps.extend(tap!("New Sticker", PanelHit::TrayNew));
    steps.extend([
        Step::Assert("a blank sticker in the tray, the Stamp tool's source", |s, sc| {
            let items = tray::tray_stickers(s);
            let blank = items.iter().find(|id| doc_of(s).is_ok_and(|d| d.sticker(**id).is_some_and(|k| k.keep))).ok_or("no kept sticker")?;
            if s.tools.stamp.source != StampToolSource::Sticker(*blank) {
                return Err(format!("source {:?}", s.tools.stamp.source));
            }
            undo_since(s, sc, 1)
        }),
        Step::Dyn("click the new tray item", |s, _| {
            let id = s.sticker_ui.tray_selected.ok_or("no selected tray item")?;
            hit_click(s, PanelHit::TrayItem(id.0))
        }),
        Step::WaitIdle,
        Step::Assert("the Stamp tool with the sticker", |s, _| tool_is(s, ToolKind::Stamp)),
    ]);
    steps.extend(tap!("Duplicate", PanelHit::TrayDuplicate));
    steps.extend(tap!("Delete", PanelHit::TrayDelete));
    steps.extend(tap!("Export Picture of the tray", PanelHit::TrayExport));
    steps.extend(tap!("From picture", PanelHit::TrayNewFromPicture));
    steps.push(Step::Assert("Duplicate and Delete: two entries; Export and From picture without a path: nothing", |s, sc| {
        if tray::tray_stickers(s).len() != 2 {
            return Err(format!("tray {:?}", tray::tray_stickers(s)));
        }
        undo_since(s, sc, 3)
    }));
    steps.push(Step::Dyn("select the blank sticker", |s, _| {
        let id = tray::tray_stickers(s).into_iter().find(|id| doc_of(s).is_ok_and(|d| d.sticker_users(*id).is_empty())).ok_or("no blank sticker")?;
        hit_click(s, PanelHit::TrayItem(id.0))
    }));
    steps.push(Step::WaitIdle);
    steps.extend(tap!("Edit", PanelHit::TrayEdit));
    steps.extend([
        Step::Assert("Edit opened the tray sticker", |s, _| {
            let edit = session_of(s)?.sticker_edit.as_ref().ok_or("not open")?;
            if !matches!(edit.target, StickerTarget::Tray(_)) { Err(format!("target {:?}", edit.target)) } else { Ok(()) }
        }),
        Step::Dyn("click Done", |s, _| hit_click(s, PanelHit::StickerDone)),
        Step::WaitIdle,
        // The Stamp tool options: Edit before placing and the style list.
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Setup("the Tool tab", |s, _| {
            s.upper_tab = UpperTab::Tool;
            Ok(())
        }),
        Step::WaitFrames(3),
    ]);
    steps.extend(tap!("Edit before placing", PanelHit::StampEditBeforePlacing));
    steps.extend([
        Step::Assert("Edit before placing opened the tool's sticker", |s, _| {
            if s.view_mode != ViewMode::Sticker { Err(format!("view {:?}", s.view_mode)) } else { Ok(()) }
        }),
        Step::Dyn("click Done", |s, _| hit_click(s, PanelHit::StickerDone)),
        Step::WaitIdle,
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitFrames(3),
    ]);
    steps.extend(tap!("the style list", PanelHit::StampToolStyle));
    steps.extend(tap!("the style list again (closes it)", PanelHit::StampToolStyle));
    steps.push(Step::Assert("the style list changed nothing in the document", |s, _| {
        if !matches!(s.dialog, Dialog::None) { Err(format!("dialog {:?}", s.dialog)) } else { Ok(()) }
    }));
    steps.push(Step::Setup("the Sticker tab for the final screenshot", |s, _| {
        s.upper_tab = UpperTab::Sticker;
        Ok(())
    }));
    steps.push(Step::WaitFrames(3));
    steps
}
