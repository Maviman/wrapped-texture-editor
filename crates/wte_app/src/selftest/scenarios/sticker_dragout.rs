// Stickers P11 (STICKER_PLAN 7 "P11", 6.4, 13.5, 13.9, 8.2; ARCHITECTURE.md 8.11, 8.17): the scenario `sticker_dragout` -
// drag out of WTE with the self-test's port: real pointer steps start an egui drag on a Layers row, a Sticker tab
// thumbnail, a tray item and the PSD handle and leave the window with the button held (`Step::ScreenHold`); the drag's
// job prepares the file; the "post" reaches `ole::drag_source::Recorder` in the next frame (where the subclass would run
// DoDragDrop; the OS is never involved), which reads the data object like a drop target; the app then injects the
// release OLE ate. Checked: the recorded formats and bytes of each kind, the next pointer move drags nothing, an
// in-window reorder still works and drags nothing out, an Esc'd drag removes its file, the PSD handle asks "Save first"
// on an unsaved project and afterwards hands out the linked PSD in the default editor (Photoshop: the tab-bar toast
// once per session).

use super::sticker::doc_state_hash;
use super::*;
use crate::ole::drag_source::{self, RecordedDrag};
use crate::ole::{DataKind, DropItem};
use crate::selftest::Target;
use crate::state::UpperTab;
use wte_core::StickerId;

const ON_MODEL: [f32; 2] = [-0.2, -0.1];
const SIDE: u32 = 64;

/// A 64 x 64 sticker: an orange disc, with the Outline style as its stamp styles (as in `sticker_clip`).
fn badge_wtes() -> Result<Vec<u8>, String> {
    let mut img = PixelBuf::new(SIDE, SIDE);
    let c = (SIDE as f32 - 1.0) / 2.0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            if ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() < 28.0 {
                img.set(x, y, [240, 140, 30, 255]);
            }
        }
    }
    let pixels = wte_core::PicturePixels::Owned { pixels: Arc::new(img), version: 0 };
    let sticker = wte_core::StickerDoc::from_picture("Badge", pixels, [SIDE, SIDE]);
    let styles = wte_core::StylePreset::Outline.styles([SIDE, SIDE]);
    wte_core::write_wtes(&sticker, Some(&styles), &wte_core::BlobStore::default(), None).map_err(|e| e.to_string())
}

fn project(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, ".wtep")
}

fn dragout_folder(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, "_dragout")
}

/// A press on `from`, a few moves inside, then out of the window past its right edge, the button held.
fn hold_out(s: &AppState, from: egui::Pos2) -> Result<Vec<Step>, String> {
    let right = s.side_panel_rect.ok_or("the side panel is not laid out")?.right();
    let path = vec![
        from,
        from + egui::vec2(6.0, 0.0),
        from + egui::vec2(14.0, 2.0),
        from + egui::vec2(30.0, 2.0),
        egui::pos2(right - 3.0, from.y + 2.0),
        egui::pos2(right + 40.0, from.y + 4.0),
        egui::pos2(right + 90.0, from.y + 4.0),
    ];
    Ok(vec![Step::ScreenHold { path }])
}

/// The control's rect, which must lie inside the window (above the status bar).
fn visible_rect(s: &AppState, hit: PanelHit) -> Result<egui::Rect, String> {
    let r = panel_rect(s, hit)?;
    let bottom = s.view_rects.get(&ViewId::Main3D).map_or(f32::INFINITY, |v| v.bottom());
    if r.top() < 0.0 || r.bottom() > bottom + 1.0 {
        return Err(format!("{hit:?} is at {r:?}, outside the visible panel"));
    }
    Ok(r)
}

/// A control of the upper tab page (the Sticker tab), which must lie inside the page's visible part.
fn upper_rect(s: &AppState, hit: PanelHit) -> Result<egui::Rect, String> {
    let r = visible_rect(s, hit)?;
    let page = s.side_rects.upper_page;
    if page.is_positive() && (r.top() < page.top() || r.bottom() > page.bottom()) {
        return Err(format!("{hit:?} is at {r:?}, outside the upper page {page:?}"));
    }
    Ok(r)
}

/// A wheel step over the upper page that brings `hit` inside it (as `sticker_tab` does).
fn reveal(s: &AppState, hit: PanelHit) -> Result<Vec<Step>, String> {
    let rect = panel_rect(s, hit)?;
    let page = s.side_rects.upper_page;
    if !page.is_positive() {
        return Ok(Vec::new());
    }
    let dy = if rect.top() < page.top() + 2.0 {
        rect.top() - page.top() - 12.0
    } else if rect.bottom() > page.bottom() - 2.0 {
        rect.bottom() - page.bottom() + 12.0
    } else {
        return Ok(Vec::new());
    };
    Ok(vec![Step::ScreenScroll { at: page.center(), notches: (-dy / 40.0).clamp(-30.0, 30.0) }])
}

fn last_recorded(sc: &Scratch) -> Result<RecordedDrag, String> {
    let all = drag_source::recorded();
    let before = remembered(sc, "recorded")? as usize;
    if all.len() != before + 1 {
        return Err(format!("{} drag(s) recorded since the last check, not 1", all.len() as i64 - before as i64));
    }
    all.last().cloned().ok_or_else(|| "no drag recorded".into())
}

fn remember_recorded(sc: &mut Scratch) {
    sc.values.insert("recorded", drag_source::recorded().len() as u64);
}

fn names_are(rec: &RecordedDrag, want: &[&str]) -> Result<(), String> {
    if rec.names() == want { Ok(()) } else { Err(format!("formats {:?}, not {want:?}", rec.names())) }
}

fn dropped_file(rec: &RecordedDrag) -> Result<PathBuf, String> {
    let bytes = rec.bytes("CF_HDROP").ok_or("no CF_HDROP")?;
    let files = crate::ole::formats::parse_dropfiles(bytes, 8);
    match files.as_slice() {
        [one] => Ok(one.clone()),
        other => Err(format!("CF_HDROP holds {other:?}")),
    }
}

fn png_of(rec: &RecordedDrag) -> Result<PixelBuf, String> {
    let png = rec.bytes("PNG").ok_or("no PNG")?;
    let pixels = wte_core::decode_image(png, Some("drag.png"), 16384).map_err(|e| e.to_string())?.pixels;
    let dib = rec.bytes("CF_DIBV5").ok_or("no CF_DIBV5")?;
    if crate::ole::formats::dib_to_pixels(dib)? != pixels {
        return Err("the CF_DIBV5 does not hold the PNG's pixels".into());
    }
    Ok(pixels)
}

/// The PNG file of a recorded drag lies in the run's drag-out folder and holds the offered PNG.
fn file_in_folder(rec: &RecordedDrag, sc: &Scratch) -> Result<PathBuf, String> {
    let path = dropped_file(rec)?;
    if path.parent() != Some(dragout_folder(sc).as_path()) {
        return Err(format!("{} is not in the run's drag-out folder", path.display()));
    }
    if std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))? != rec.bytes("PNG").unwrap_or_default() {
        return Err("the file is not the PNG offered".into());
    }
    Ok(path)
}

fn ended_at_least(n: u64) -> fn(&AppState, &Scratch) -> Result<bool, String> {
    match n {
        1 => |s, _| Ok(s.drag_out.ended >= 1 && !s.drag_out.busy()),
        2 => |s, _| Ok(s.drag_out.ended >= 2 && !s.drag_out.busy()),
        3 => |s, _| Ok(s.drag_out.ended >= 3 && !s.drag_out.busy()),
        4 => |s, _| Ok(s.drag_out.ended >= 4 && !s.drag_out.busy()),
        5 => |s, _| Ok(s.drag_out.ended >= 5 && !s.drag_out.busy()),
        _ => |s, _| Ok(s.drag_out.ended >= 6 && !s.drag_out.busy()),
    }
}

fn layer_order(s: &AppState) -> Result<Vec<u64>, String> {
    Ok(doc_of(s)?.sets[0].layers.iter().map(|l| l.id.0).collect())
}

fn psd_toasts(s: &AppState) -> Vec<String> {
    s.toasts.visible_labels().into_iter().filter(|l| l.starts_with("In Photoshop the sticker stays linked")).collect()
}

/// The `sticker_dragout` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn sticker_dragout() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("the test port and the run's own drag-out folder; nothing recorded yet", |s, sc| {
            if s.drag_out.port.is_none() {
                return Err("a self-test run has the test port".into());
            }
            if s.drag_out.folder.as_deref() != Some(dragout_folder(sc).as_path()) {
                return Err(format!("drag-out folder {:?}", s.drag_out.folder));
            }
            remember_recorded(sc);
            sc.values.insert("start_doc", doc_state_hash(doc_of(s)?));
            Ok(())
        }),
        // (1) A sticker with stamp styles on the model.
        Step::Dyn("drop the sticker on the model", |s, _| {
            let (xf, c, r) = model_disc(s, ViewId::Main3D)?;
            let at = norm(&xf, disc_point(c, r, ON_MODEL));
            let payload = vec![DropItem::Data { name: "Badge.wtes".into(), kind: DataKind::Sticker, bytes: badge_wtes()? }];
            Ok(vec![Step::Drop { payload, target: ViewId::Main3D.into(), at }])
        }),
        Step::WaitIdle,
        Step::Setup("remember the sticker stamp, its row and the layer order", |s, sc| {
            let id = session_of(s)?.selected_stamp.ok_or("no stamp is selected")?;
            let doc = doc_of(s)?;
            let StampSource::Sticker { sticker } = doc.stamp(id).ok_or("gone")?.source else { return Err("not a sticker stamp".into()) };
            let row = doc.stamp_layers(id).first().ok_or("no layer")?.1;
            sc.values.insert("badge", id.0);
            sc.values.insert("sticker", sticker.0);
            sc.values.insert("row", row.0);
            sc.values.insert("order", layer_order(s)?.iter().fold(17u64, |h, v| h.wrapping_mul(31).wrapping_add(*v)));
            Ok(())
        }),
        Step::WaitFrames(2),
        // (2) Its Layers row dragged out of the window: one OS drag with the stamp's finished picture.
        Step::Dyn("drag the sticker's Layers row out of the window", |s, sc| {
            let row = visible_rect(s, PanelHit::LayerRow(LayerId(remembered(sc, "row")?)))?;
            hold_out(s, row.center())
        }),
        Step::Until("the drag out was handed over and has ended", ended_at_least(1)),
        Step::WaitFrames(2),
        Step::Setup("one drag: CF_HDROP (a PNG in the run's folder), PNG = the stamp with its styles, CF_DIBV5, WTE.Sticker.v1", |s, sc| {
            let rec = last_recorded(sc)?;
            names_are(&rec, &["CF_HDROP", "PNG", "CF_DIBV5", "WTE.Sticker.v1"])?;
            if !rec.source_ok {
                return Err("the drop source answered Esc / button up / held wrongly".into());
            }
            file_in_folder(&rec, sc)?;
            let stamp = StampId(remembered(sc, "badge")?);
            let expected = s.doc_mut().ok_or("no document")?.stamp_picture(stamp, true).map_err(|e| e.to_string())?;
            if png_of(&rec)? != expected {
                return Err("the PNG is not the stamp's finished picture with its styles".into());
            }
            let (pid, _) = crate::ole::formats::unwrap_sticker(rec.bytes("WTE.Sticker.v1").unwrap_or_default()).ok_or("a bad sticker block")?;
            if pid != std::process::id() {
                return Err("the sticker block names another process".into());
            }
            remember_recorded(sc);
            Ok(())
        }),
        Step::Assert("the rows kept their order; no drag is left in egui or in WTE", |s, sc| {
            let order = layer_order(s)?.iter().fold(17u64, |h, v| h.wrapping_mul(31).wrapping_add(*v));
            if order != remembered(sc, "order")? {
                return Err("the release outside the window reordered the layers".into());
            }
            if s.drag_out.egui_dragging || s.drag_out.current().is_some() || s.drag_out.posted != 1 {
                return Err(format!("dragging {}, current {:?}, posted {}", s.drag_out.egui_dragging, s.drag_out.current(), s.drag_out.posted));
            }
            Ok(())
        }),
        Step::Shot {
            name: "dragout_after_row",
            expect: "the sphere with the orange disc sticker and its thin dark outline, selected; the Layers list in its \
                     usual order with no floating row and no insertion line; the status bar says \"Dragged out Badge.png\"",
        },
        // (3) The next pointer move drags nothing.
        Step::Dyn("move the pointer over the Layers list", |s, sc| {
            let row = visible_rect(s, PanelHit::LayerRow(LayerId(remembered(sc, "row")?)))?;
            Ok(vec![Step::Hover { view: Target::Screen, at: [row.center().x, row.center().y + 3.0] }])
        }),
        Step::WaitFrames(2),
        Step::Assert("the next move drags nothing", |s, sc| {
            if s.drag_out.egui_dragging || s.drag_out.current().is_some() || drag_source::recorded().len() as u64 != remembered(sc, "recorded")? {
                return Err("something is still being dragged".into());
            }
            Ok(())
        }),
        // (4) Inside the window the row still reorders; nothing is dragged out and its prepared file is removed.
        Step::Setup("remember the row below the sticker's and the files removed so far", |s, sc| {
            let layers = &doc_of(s)?.sets[0].layers;
            let below = layers.get(layers.len().checked_sub(2).ok_or("one layer only")?).ok_or("no row below")?.id;
            sc.values.insert("removed", s.drag_out.removed);
            sc.values.insert("below", below.0);
            Ok(())
        }),
        Step::Dyn("reorder: drag the sticker's row onto the lower half of the row below it", |s, sc| {
            let row = visible_rect(s, PanelHit::LayerRow(LayerId(remembered(sc, "row")?)))?;
            let target = visible_rect(s, PanelHit::LayerRow(LayerId(remembered(sc, "below")?)))?;
            let to = egui::pos2(target.center().x, target.bottom() - 3.0);
            Ok(vec![Step::ScreenPointer { path: vec![row.center(), row.center() + egui::vec2(0.0, 6.0), row.center() + egui::vec2(0.0, 14.0), to, to] }])
        }),
        Step::WaitIdle,
        Step::Assert("the row moved down one place; nothing was dragged out; its file was removed", |s, sc| {
            let layers = &doc_of(s)?.sets[0].layers;
            if layers.last().map(|l| l.id.0) != Some(remembered(sc, "below")?) {
                return Err(format!("order {:?}", layer_order(s)?));
            }
            if s.drag_out.posted != 1 || drag_source::recorded().len() as u64 != remembered(sc, "recorded")? {
                return Err("an in-window reorder was dragged out".into());
            }
            if s.drag_out.removed != remembered(sc, "removed")? + 1 {
                return Err(format!("files removed {} (before {})", s.drag_out.removed, remembered(sc, "removed")?));
            }
            Ok(())
        }),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        // (5) The paint layer, filled: its picture at texture size, no sticker block.
        Step::Setup("fill the paint layer red", |s, sc| {
            let session = s.session.as_mut().ok_or("no session")?;
            let set = session.active_set;
            let layer = session.doc.sets[set.index()]
                .layers
                .iter()
                .find(|l| matches!(l.kind, LayerKind::Raster(_)) && !l.locked)
                .ok_or("no paint layer")?
                .id;
            session.doc.fill(set, layer, Vec2::new(0.5, 0.5), [230, 20, 20, 255], wte_core::FillScope::Layer).map_err(|e| e.to_string())?;
            sc.values.insert("paint", layer.0);
            Ok(())
        }),
        Step::WaitIdle,
        Step::Dyn("drag the paint layer's row out of the window", |s, sc| {
            let row = visible_rect(s, PanelHit::LayerRow(LayerId(remembered(sc, "paint")?)))?;
            hold_out(s, row.center())
        }),
        Step::Until("the second drag out has ended", ended_at_least(2)),
        Step::Assert("CF_HDROP, PNG and CF_DIBV5 of the whole layer at texture size; no sticker", |s, sc| {
            let rec = last_recorded(sc)?;
            names_are(&rec, &["CF_HDROP", "PNG", "CF_DIBV5"])?;
            file_in_folder(&rec, sc)?;
            let got = png_of(&rec)?;
            let set = doc_of(s)?.set(session_of(s)?.active_set).ok_or("no set")?;
            if (got.width, got.height) != (set.width, set.height) || got.data[..4] != [230, 20, 20, 255] {
                return Err(format!("{} x {}, first pixel {:?}", got.width, got.height, &got.data[..4]));
            }
            Ok(())
        }),
        Step::Setup("remember", |_, sc| {
            remember_recorded(sc);
            Ok(())
        }),
        // (6) The Sticker tab: the thumbnail, dragged out and cancelled (Esc): its file is removed.
        Step::Setup("the Sticker tab with the sticker stamp; the recorder answers Esc", |s, sc| {
            crate::dispatch::select_stamp(s, Some(StampId(remembered(sc, "badge")?)));
            s.upper_tab = UpperTab::Sticker;
            s.drag_out.test_effect = 0;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "dragout_sticker_tab",
            expect: "the Sticker tab: the selected stamp \"Badge\" with its thumbnail, under it the small \"PSD\" handle \
                     with grip dots and the caption \"drag to your editor: stays linked\", then Edit Sticker and Export \
                     Picture...",
        },
        Step::Dyn("drag the thumbnail out of the window", |s, _| hold_out(s, upper_rect(s, PanelHit::StickerTabThumb)?.center())),
        Step::Until("the third drag out has ended", ended_at_least(3)),
        Step::Setup("an Esc'd drag: the picture was offered, nobody took it, its file is gone", |s, sc| {
            let rec = last_recorded(sc)?;
            names_are(&rec, &["CF_HDROP", "PNG", "CF_DIBV5", "WTE.Sticker.v1"])?;
            if rec.effect != 0 {
                return Err("the recorder did not cancel".into());
            }
            let file = dropped_file(&rec)?;
            if file.exists() {
                return Err(format!("{} was not removed", file.display()));
            }
            s.drag_out.test_effect = 1;
            remember_recorded(sc);
            Ok(())
        }),
        // (7) A tray item: the sticker's own picture (no stamp styles) and the sticker.
        Step::Dyn("show the tray item", |s, sc| reveal(s, PanelHit::TrayItem(remembered(sc, "sticker")?))),
        Step::WaitFrames(3),
        Step::Dyn("show the tray item", |s, sc| reveal(s, PanelHit::TrayItem(remembered(sc, "sticker")?))),
        Step::WaitFrames(3),
        Step::Dyn("drag the tray item out of the window", |s, sc| hold_out(s, upper_rect(s, PanelHit::TrayItem(remembered(sc, "sticker")?))?.center())),
        Step::Until("the fourth drag out has ended", ended_at_least(4)),
        Step::Setup("the tray sticker's picture without the stamp styles, and the sticker", |s, sc| {
            let rec = last_recorded(sc)?;
            names_are(&rec, &["CF_HDROP", "PNG", "CF_DIBV5", "WTE.Sticker.v1"])?;
            file_in_folder(&rec, sc)?;
            let id = StickerId(remembered(sc, "sticker")?);
            let expected = s.doc_mut().ok_or("no document")?.sticker_picture(id, None, 1.0).map_err(|e| e.to_string())?;
            if png_of(&rec)? != expected {
                return Err("the PNG is not the sticker's picture".into());
            }
            remember_recorded(sc);
            Ok(())
        }),
        // (8) The PSD handle on an unsaved project: "Save first", no drag.
        Step::Dyn("show the PSD handle", |s, _| reveal(s, PanelHit::StickerTabPsdHandle)),
        Step::WaitFrames(3),
        Step::Dyn("show the PSD handle", |s, _| reveal(s, PanelHit::StickerTabPsdHandle)),
        Step::WaitFrames(3),
        Step::Dyn("drag the PSD handle out of the unsaved project (released outside)", |s, _| {
            let r = upper_rect(s, PanelHit::StickerTabPsdHandle)?;
            let right = s.side_panel_rect.ok_or("no side panel")?.right();
            let c = r.center();
            Ok(vec![Step::ScreenPointer { path: vec![c, c + egui::vec2(8.0, 0.0), c + egui::vec2(20.0, 0.0), egui::pos2(right + 60.0, c.y)] }])
        }),
        Step::WaitFrames(3),
        Step::Assert("\"Save first\"; nothing was linked or dragged out", |s, sc| {
            let q = crate::link::open_question(s).ok_or("no question")?;
            if q.kind != crate::link::QuestionKind::SaveFirst {
                return Err(format!("question {:?}", q.kind));
            }
            if doc_of(s)?.links().next().is_some() || s.drag_out.posted != 4 || drag_source::recorded().len() as u64 != remembered(sc, "recorded")? {
                return Err("something was linked or dragged out".into());
            }
            Ok(())
        }),
        Step::Shot {
            name: "dragout_save_first",
            expect: "the question \"Save first\" with Cancel and Save... over the sphere with the disc sticker",
        },
        Step::Setup("Cancel", |s, _| crate::link::press(s, "Cancel")),
        Step::WaitIdle,
        // (9) Saved, Photoshop (a stub program in the run folder) as the default editor: the handle hands out the PSD.
        Step::Dyn("Save As into the run folder", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Setup("Photoshop known (a stub program) and the default editor", |s, sc| {
            let exe = with_suffix(&sc.out_stem, "_stub").join("Photoshop.exe");
            std::fs::create_dir_all(exe.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
            std::fs::write(&exe, b"").map_err(|e| e.to_string())?;
            s.prefs.photoshop_exe = Some(exe);
            s.prefs.default_editor = Some(wte_core::link::EDITOR_PHOTOSHOP.to_owned());
            s.link_ui.discovered = true;
            crate::link::editor::refresh_known(s);
            Ok(())
        }),
        Step::WaitFrames(2),
        Step::Dyn("drag the PSD handle out of the window", |s, _| hold_out(s, upper_rect(s, PanelHit::StickerTabPsdHandle)?.center())),
        Step::Until("the fifth drag out has ended", ended_at_least(5)),
        Step::WaitFrames(2),
        Step::Setup("CF_HDROP with the linked PSD only; the link in Photoshop, opened; the tab-bar toast", |s, sc| {
            let rec = last_recorded(sc)?;
            names_are(&rec, &["CF_HDROP"])?;
            let id = StickerId(remembered(sc, "sticker")?);
            let link = doc_of(s)?.link(id).cloned().ok_or("no link record")?;
            if link.editor != wte_core::link::EDITOR_PHOTOSHOP {
                return Err(format!("the link's editor is {}", link.editor));
            }
            let psd = crate::link::link_path(s, id).ok_or("no link path")?;
            if dropped_file(&rec)? != psd || !psd.is_file() {
                return Err(format!("dropped {:?}, linked {}", dropped_file(&rec), psd.display()));
            }
            let opened = s.link_ui.sessions.get(&id.into()).is_some_and(|x| x.opened && x.opened_editor.as_deref() == Some(wte_core::link::EDITOR_PHOTOSHOP));
            if !opened {
                return Err("the link does not count as opened in Photoshop".into());
            }
            if psd_toasts(s) != [crate::dragout::PHOTOSHOP_TAB_TOAST] {
                return Err(format!("toasts {:?}", s.toasts.visible_labels()));
            }
            remember_recorded(sc);
            Ok(())
        }),
        Step::Shot {
            name: "dragout_psd_toast",
            expect: "the toast \"In Photoshop the sticker stays linked only when it opened as its own tab. Drop it on the \
                     tab bar, not on a picture.\"; the Sticker tab shows the link (\"Link: linked\", \"Editor: Photoshop\")",
        },
        // (10) Again: the same PSD, no second toast.
        Step::Dyn("drag the PSD handle out again", |s, _| hold_out(s, upper_rect(s, PanelHit::StickerTabPsdHandle)?.center())),
        Step::Until("the sixth drag out has ended", ended_at_least(6)),
        Step::Assert("the same PSD; the toast was not shown again", |s, sc| {
            let rec = last_recorded(sc)?;
            let id = StickerId(remembered(sc, "sticker")?);
            if Some(dropped_file(&rec)?) != crate::link::link_path(s, id) {
                return Err("another file".into());
            }
            if psd_toasts(s) != [crate::dragout::PHOTOSHOP_TAB_TOAST] || !s.drag_out.psd_toast_shown {
                return Err(format!("toasts {:?}", s.toasts.visible_labels()));
            }
            if s.drag_out.posted != 6 || s.drag_out.egui_dragging {
                return Err(format!("posted {}, dragging {}", s.drag_out.posted, s.drag_out.egui_dragging));
            }
            Ok(())
        }),
    ]
}
