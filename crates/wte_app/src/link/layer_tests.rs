//! Tests of linked paint layers in the app (stickers P12; STICKER_PLAN 7 "P12", 5.9, 13.9 "P12"): "Edit Layer in
//! <editor>" makes the one-layer PSD and starts the stub, a locked and a stamp layer are not offered (and refused when
//! the command arrives anyway), an editor's save is one undo step "Update Linked Layer" and an undo is never applied
//! again by itself, the conflict, the size mismatch and a refused colour mode on the layer badge, Send, Unlink, the
//! file restored after a load, the Photoshop wording, and Paste as New Paint Layer through the in-memory clipboard.

use std::path::PathBuf;

use wte_core::link::{EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP, FileStamp, PAINT_UPDATE_LABEL, links_folder};
use wte_core::{LayerId, PixelBuf, TextureSetId};

use super::tests::{DialogName, TmpDir, linked_state, question_kind, run_queued};
use super::watch::LinkEvent;
use super::*;
use crate::state::AppState;

const S0: TextureSetId = TextureSetId(0);

/// The linked-state fixture with a new paint layer "Dirt" painted with a red rectangle (the history cleared).
fn layer_state(name: &str) -> (AppState, LayerId, TmpDir) {
    let (mut state, _stamp, dir) = linked_state(name);
    let doc = state.doc_mut().unwrap();
    let layer = doc.add_raster_layer(S0, Some("Dirt")).unwrap();
    let [w, h] = doc.layer_texture_size(layer).unwrap();
    doc.replace_layer_pixels(layer, rect(w, h, [10, 10, 40, 30], [200, 30, 30, 255]), "Paint").unwrap();
    doc.history.clear();
    (state, layer, dir)
}

fn texture(state: &AppState, layer: LayerId) -> [u32; 2] {
    state.doc().unwrap().layer_texture_size(layer).unwrap()
}

/// A `w x h` transparent picture with one rectangle.
fn rect(w: u32, h: u32, r: [u32; 4], rgba: [u8; 4]) -> PixelBuf {
    let mut p = PixelBuf::new(w, h);
    for y in r[1]..r[3].min(h) {
        for x in r[0]..r[2].min(w) {
            p.set(x, y, rgba);
        }
    }
    p
}

fn dense(state: &AppState, layer: LayerId) -> PixelBuf {
    let [w, h] = texture(state, layer);
    state.doc().unwrap().layer(layer).unwrap().to_dense(w, h)
}

fn edit_layer(state: &mut AppState, layer: LayerId) {
    flows::run_in(state, LinkAction::EditIn(EditRequest::new(LinkTarget::Layer(layer))));
    run_queued(state);
}

/// The editor saves `picture` as a one-layer PSD over the layer's file (temp file + rename).
fn editor_saves(state: &AppState, layer: LayerId, picture: &PixelBuf) -> PathBuf {
    let path = link_path(state, layer).unwrap();
    let psd = wte_core::link::paint::layer_psd("Dirt", picture, picture.width, picture.height);
    testkit::save_as_editor(&path, &psd).unwrap();
    path
}

/// Reads the layer's file as the watcher would and hands the event to the app.
fn deliver(state: &mut AppState, layer: LayerId) {
    let path = link_path(state, layer).unwrap();
    let (stamp, bytes) = FileStamp::read(&path).unwrap();
    match wte_core::psd::read(&bytes, &wte_core::psd::PsdLimits::default()) {
        Ok(psd) => on_event(state, LinkEvent::Changed { id: layer.into(), stamp, psd: Box::new(psd), from_alias: false }, false),
        Err(error) => on_event(state, LinkEvent::Refused { id: layer.into(), stamp, error }, false),
    }
}

fn badge_text(state: &mut AppState, layer: LayerId) -> String {
    badge(state, layer).map(|(_, t)| t).unwrap_or_default()
}

fn undo_len(state: &AppState) -> usize {
    state.doc().unwrap().history.undo_len()
}

#[test]
fn edit_layer_in_makes_the_one_layer_psd_and_starts_the_stub() {
    let (mut state, layer, dir) = layer_state("layer_edit");
    assert_eq!(ui::edit_layer_label(&state, layer), "Edit Layer in PhotoCraft", "the only known editor");
    edit_layer(&mut state, layer);
    let link = state.doc().unwrap().layer_link(layer).cloned().expect("linked");
    let path = links_folder(&dir.join("proj.wtep")).join(format!("Dirt_l{}.psd", layer.0));
    assert_eq!(link_path(&state, layer).as_deref(), Some(path.as_path()));
    assert_eq!(link.editor, EDITOR_PHOTOCRAFT);
    let psd = testkit::read(&path).unwrap();
    assert_eq!(psd.size, texture(&state, layer));
    assert_eq!(psd.layers.len(), 1);
    let calls = state.link_ui.launcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].editor.as_str(), calls[0].file.as_path()), ("photocraft", path.as_path()));
    assert!(state.link_ui.made_since_save.contains(&LinkId::Layer(layer)));
    assert!(state.toasts.visible_labels().iter().any(|t| t.contains("opacity and blend mode stay in WTE")));
    assert_eq!(badge_text(&mut state, layer), "linked");
    assert_eq!(undo_len(&state), 0, "making the link changes no pixel");
    assert!(state.doc().unwrap().is_modified(), "the link asks for a save");
    // The watcher follows it; the close prompt names it.
    assert!(super::wanted(&state).iter().any(|w| w.id == LinkId::Layer(layer) && w.path == path));
    assert!(unsaved_line(&state).unwrap().contains("Dirt (paint layer)"));
    // Edit Layer in again with the file untouched: no question, the stub again.
    edit_layer(&mut state, layer);
    assert_eq!(state.dialog_name(), "None");
    assert_eq!(state.link_ui.launcher.calls().len(), 2);
}

#[test]
fn locked_and_stamp_layers_are_not_offered_and_refused() {
    let (mut state, layer, _dir) = layer_state("layer_refused");
    assert!(ui::layer_editable(&state, layer));
    state.doc_mut().unwrap().set_layer_locked(layer, true).unwrap();
    assert!(!ui::layer_editable(&state, layer), "a locked layer is not offered");
    edit_layer(&mut state, layer);
    assert!(state.doc().unwrap().layer_link(layer).is_none());
    assert!(state.link_ui.launcher.calls().is_empty());
    assert!(state.status_hint.contains("locked"), "{}", state.status_hint);
    // The stamp's own layer (an image stamp of the fixture).
    let stamp_layer = state.doc().unwrap().set(S0).unwrap().layers.iter().find(|l| !l.is_raster()).map(|l| l.id).expect("a stamp layer");
    assert!(!ui::layer_editable(&state, stamp_layer), "a stamp layer is not offered (edit its sticker)");
    edit_layer(&mut state, stamp_layer);
    assert!(state.doc().unwrap().layer_link(stamp_layer).is_none());
    assert!(state.link_ui.launcher.calls().is_empty());
}

#[test]
fn an_editors_save_is_one_update_and_an_undo_stays_changed_in_wte() {
    let (mut state, layer, _dir) = layer_state("layer_update");
    edit_layer(&mut state, layer);
    let [w, h] = texture(&state, layer);
    let green = rect(w, h, [50, 60, 120, 90], [20, 220, 40, 255]);
    editor_saves(&state, layer, &green);
    deliver(&mut state, layer);
    assert_eq!(undo_len(&state), 1);
    assert_eq!(state.doc().unwrap().undo_label(), Some(PAINT_UPDATE_LABEL));
    assert_eq!(dense(&state, layer), green);
    assert_eq!(badge_text(&mut state, layer), "linked");
    assert_eq!(state.link_ui.counters.applied, 1);
    assert!(state.toasts.visible_labels().iter().any(|t| t == "Updated from PhotoCraft"));
    // Undo: "changed in WTE"; the same file again is nothing (the stamp did not change, R6).
    state.doc_mut().unwrap().undo().unwrap();
    assert_eq!(badge_text(&mut state, layer), "changed in WTE");
    deliver(&mut state, layer);
    assert_eq!(state.link_ui.counters.applied, 1, "nothing is applied by itself");
    assert_ne!(dense(&state, layer), green);
    // Update now applies it again.
    flows::run_in(&mut state, LinkAction::UpdateNow(layer.into()));
    assert_eq!(dense(&state, layer), green);
    assert_eq!(badge_text(&mut state, layer), "linked");
}

#[test]
fn both_changed_is_a_conflict_with_both_answers() {
    let (mut state, layer, _dir) = layer_state("layer_conflict");
    edit_layer(&mut state, layer);
    let [w, h] = texture(&state, layer);
    // WTE paints, the editor saves: the question, nothing applied.
    let wte = rect(w, h, [0, 0, 20, 20], [0, 0, 255, 255]);
    state.doc_mut().unwrap().replace_layer_pixels(layer, wte.clone(), "Paint").unwrap();
    let theirs = rect(w, h, [30, 30, 60, 60], [255, 255, 0, 255]);
    editor_saves(&state, layer, &theirs);
    deliver(&mut state, layer);
    assert_eq!(question_kind(&state), Some(QuestionKind::Conflict { saved_over: false }));
    let q = open_question(&state).unwrap();
    assert_eq!(q.title, "Linked layer changed");
    assert_eq!(q.lines[0], "Both WTE and PhotoCraft changed Dirt.");
    assert_eq!(q.link, Some(LinkId::Layer(layer)));
    assert_eq!(dense(&state, layer), wte);
    assert_eq!(badge_text(&mut state, layer), "conflict");
    // Take PhotoCraft's version: applied, WTE's paint one undo away.
    press(&mut state, "Take PhotoCraft's version").unwrap();
    run_queued(&mut state);
    assert_eq!(dense(&state, layer), theirs);
    state.doc_mut().unwrap().undo().unwrap();
    assert_eq!(dense(&state, layer), wte);
    // Keep WTE's version: the file is rewritten with WTE's pixels (the editor had it open: the warning first).
    flows::run_in(&mut state, LinkAction::Send(SendRequest { answers_conflict: true, ..SendRequest::new(layer) }));
    run_queued(&mut state);
    assert_eq!(question_kind(&state), Some(QuestionKind::SendWarning));
    press(&mut state, "Send anyway").unwrap();
    run_queued(&mut state);
    let back = testkit::read(&link_path(&state, layer).unwrap()).unwrap();
    let got = wte_core::link::paint_source(&back, [w, h], EDITOR_PHOTOCRAFT).unwrap().pixels;
    assert_eq!(got, wte);
    assert_eq!(badge_text(&mut state, layer), "linked");
    assert!(state.link_ui.sessions[&LinkId::Layer(layer)].stale_in_editor, "the editor's next save is a question");
}

#[test]
fn a_size_change_and_a_cmyk_file_are_refused_on_the_layer_badge() {
    let (mut state, layer, dir) = layer_state("layer_problems");
    state.prefs.photoshop_exe = Some({
        let exe = dir.join("ps").join("Photoshop.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"").unwrap();
        exe
    });
    state.prefs.default_editor = Some(EDITOR_PHOTOSHOP.to_owned());
    editor::refresh_known(&mut state);
    assert_eq!(ui::edit_layer_label(&state, layer), "Edit Layer in Photoshop");
    edit_layer(&mut state, layer);
    assert_eq!(state.doc().unwrap().layer_link(layer).unwrap().editor, EDITOR_PHOTOSHOP);
    let before = dense(&state, layer);
    let [w, h] = texture(&state, layer);
    // Another canvas size: nothing changes; the sentence on the badge and in a toast.
    editor_saves(&state, layer, &rect(w + 10, h, [0, 0, 5, 5], [1, 2, 3, 255]));
    deliver(&mut state, layer);
    let want = format!("Dirt is {} x {h} in Photoshop; the texture is {w} x {h}. Undo the size change in Photoshop.", w + 10);
    assert_eq!(badge_text(&mut state, layer), want);
    assert!(state.toasts.visible_labels().contains(&want));
    assert_eq!(dense(&state, layer), before);
    assert_eq!(undo_len(&state), 0);
    // A CMYK file: its sentence on the badge.
    let path = link_path(&state, layer).unwrap();
    let mut cmyk = Vec::new();
    cmyk.extend_from_slice(b"8BPS\0\x01\0\0\0\0\0\0\0\x04");
    cmyk.extend_from_slice(&h.to_be_bytes());
    cmyk.extend_from_slice(&w.to_be_bytes());
    cmyk.extend_from_slice(&[0, 8, 0, 4]);
    cmyk.extend_from_slice(&[0; 12]);
    cmyk.extend_from_slice(&[0, 0]);
    cmyk.extend(std::iter::repeat_n(0u8, (w * h * 4) as usize));
    testkit::save_bytes(&path, &cmyk).unwrap();
    deliver(&mut state, layer);
    let text = badge_text(&mut state, layer);
    assert!(text.contains("CMYK"), "{text}");
    assert_eq!(dense(&state, layer), before);
}

#[test]
fn unlink_and_restore_after_a_load() {
    let (mut state, layer, dir) = layer_state("layer_restore");
    edit_layer(&mut state, layer);
    let path = link_path(&state, layer).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    // Save the project for real, delete the file, then what `load` reports restores it.
    let project = dir.join("proj.wtep");
    let opts = wte_core::project::SaveOptions {
        png: wte_core::PngLevel::Fast,
        write_flattened: false,
        keep_backup: false,
        now_unix: 1,
        app_version: "test".to_owned(),
    };
    wte_core::project::save(&state.doc().unwrap().snapshot(), &project, &opts, &wte_core::NoProgress).unwrap();
    std::fs::remove_file(&path).unwrap();
    let report = wte_core::project::load(&project, &wte_core::NoProgress).unwrap();
    assert_eq!(report.layer_links.len(), 1);
    state.session.as_mut().unwrap().doc = report.document;
    state.link_ui.reset();
    flows::after_load_layers(&mut state, &project, &report.layer_links);
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "restored from the project");
    assert!(state.toasts.visible_labels().iter().any(|t| t.starts_with("Restored Dirt_l")));
    // Changed while closed: the question, Update applies it.
    let [w, h] = texture(&state, layer);
    let blue = rect(w, h, [5, 5, 25, 25], [0, 0, 250, 255]);
    editor_saves(&state, layer, &blue);
    let report = wte_core::project::load(&project, &wte_core::NoProgress).unwrap();
    state.session.as_mut().unwrap().doc = report.document;
    state.link_ui.reset();
    flows::after_load_layers(&mut state, &project, &report.layer_links);
    assert_eq!(question_kind(&state), Some(QuestionKind::ChangedWhileClosed));
    assert_eq!(open_question(&state).unwrap().lines[0], "Linked layer Dirt changed while the project was closed.");
    press(&mut state, "Update").unwrap();
    run_queued(&mut state);
    assert_eq!(dense(&state, layer), blue);
    // Unlink: the pixels stay, the file stays.
    flows::run_in(&mut state, LinkAction::Unlink(layer.into()));
    assert!(state.doc().unwrap().layer_link(layer).is_none());
    assert_eq!(dense(&state, layer), blue);
    assert!(path.is_file());
    assert!(!super::wanted(&state).iter().any(|w| w.id == LinkId::Layer(layer)));
}

/// Paste as New Paint Layer through the in-memory clipboard: a picture of the texture's size as it is, another size
/// fitted and centred; one undo step each.
#[test]
fn paste_as_new_paint_layer_through_the_clipboard() {
    use crate::clip::{ClipAction, run_with};
    use crate::ole::clipboard::ClipFormat;
    let ctx = eframe::egui::Context::default();
    let (mut state, layer, _dir) = layer_state("layer_paste");
    let [w, h] = texture(&state, layer);
    let settle = |state: &mut AppState| {
        for _ in 0..1000 {
            crate::clip::poll(state);
            if !state.clip_ui.busy() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the paste was not prepared");
    };
    let png = |p: &PixelBuf| wte_core::encode_png(p.width, p.height, &p.data, wte_core::PngLevel::Fast).unwrap();
    let same = rect(w, h, [3, 3, 30, 20], [9, 99, 199, 255]);
    state.clipboard.write(vec![(ClipFormat::Png, png(&same))]).unwrap();
    let layers = state.doc().unwrap().set(S0).unwrap().layers.len();
    run_with(&ctx, &mut state, ClipAction::PasteAsLayer);
    settle(&mut state);
    let set = state.doc().unwrap().set(S0).unwrap();
    assert_eq!(set.layers.len(), layers + 1);
    let new = set.active.unwrap();
    assert_eq!(state.doc().unwrap().layer(new).unwrap().name, "Pasted");
    assert_eq!(dense(&state, new), same);
    assert_eq!(undo_len(&state), 1);
    assert_eq!(state.doc().unwrap().undo_label(), Some(crate::clip::PASTE_LAYER_LABEL));
    // A square picture of another size: fitted with its aspect kept, centred.
    let square = PixelBuf::filled(10, 10, [255, 0, 0, 255]);
    state.clipboard.write(vec![(ClipFormat::DibV5, crate::ole::formats::pixels_to_dibv5(&square))]).unwrap();
    run_with(&ctx, &mut state, ClipAction::PasteAsLayer);
    settle(&mut state);
    let new2 = state.doc().unwrap().set(S0).unwrap().active.unwrap();
    assert_ne!(new2, new);
    assert_eq!(dense(&state, new2), wte_core::doc_layer_links::fit_centred(&square, w, h));
    assert_eq!(undo_len(&state), 2);
    // Only sticker data: nothing WTE can paste as a layer.
    state.clipboard = Box::new(crate::ole::clipboard::MemClipboard::default());
    run_with(&ctx, &mut state, ClipAction::PasteAsLayer);
    assert!(state.toasts.visible_labels().iter().any(|t| t.starts_with("Nothing WTE can paste.")));
    assert!(!state.clip_ui.busy());
}

/// check-P12: an editor's save that arrives during an open texture stroke (the button may already be up while the
/// stroke finishes) is held, not refused with an error toast and lost; it is applied once the stroke ends.
#[test]
fn check_p12_an_update_waits_for_an_open_texture_stroke() {
    let (mut state, layer, _dir) = layer_state("layer_stroke_wait");
    edit_layer(&mut state, layer);
    let [w, h] = texture(&state, layer);
    let doc = state.doc_mut().unwrap();
    let base = doc.add_raster_layer(S0, Some("Other")).unwrap();
    doc.set_active_layer(S0, Some(base)).unwrap();
    doc.stroke_begin(&wte_core::BrushParams::default()).unwrap();
    doc.stroke_to_2d(S0, glam::Vec2::new(5.0, 5.0), 1.0);
    assert!(doc.stroke_active());
    let green = rect(w, h, [50, 60, 120, 90], [20, 220, 40, 255]);
    editor_saves(&state, layer, &green);
    deliver(&mut state, layer);
    assert!(busy_for(&state, LinkId::Layer(layer), false), "an open texture stroke holds a layer's update");
    assert_ne!(dense(&state, layer), green, "not applied during the stroke");
    assert!(state.link_ui.sessions.get(&LinkId::Layer(layer)).is_some_and(|s| s.held.is_some()), "held, not lost");
    assert!(!state.toasts.visible_labels().iter().any(|t| t.contains("stroke")), "no error toast");
    assert!(state.doc_mut().unwrap().stroke_end());
    super::retry_held(&mut state, false);
    assert_eq!(dense(&state, layer), green, "applied after the stroke");
    assert_eq!(state.doc().unwrap().undo_label(), Some(PAINT_UPDATE_LABEL));
}

/// check-P12: the chooser of "Edit Layer in..." (two editors, no default) speaks of the layer, not of a sticker.
#[test]
fn check_p12_the_layer_chooser_speaks_of_the_layer() {
    let (mut state, layer, dir) = layer_state("layer_chooser");
    let ps = dir.join("ps").join("Photoshop.exe");
    std::fs::create_dir_all(ps.parent().unwrap()).unwrap();
    std::fs::write(&ps, b"").unwrap();
    state.prefs.photoshop_exe = Some(ps);
    state.prefs.default_editor = None;
    edit_layer(&mut state, layer);
    assert_eq!(question_kind(&state), Some(QuestionKind::ChooseEditor));
    let q = open_question(&state).unwrap();
    assert_eq!(q.link, Some(LinkId::Layer(layer)));
    assert_eq!(q.lines, ["WTE opens the layer's pixels in it; Ctrl+S there updates the layer on the model."]);
    assert!(state.link_ui.launcher.calls().is_empty());
}

/// check-P12: Paste as New Paint Layer whose picture is ready during an open stroke waits for the stroke's end (the
/// new layer would be refused and the picture lost).
#[test]
fn check_p12_paste_as_a_layer_waits_for_an_open_stroke() {
    use crate::clip::{ClipAction, run_with};
    use crate::ole::clipboard::ClipFormat;
    let ctx = eframe::egui::Context::default();
    let (mut state, layer, _dir) = layer_state("layer_paste_stroke");
    let [w, h] = texture(&state, layer);
    let pic = rect(w, h, [3, 3, 30, 20], [9, 99, 199, 255]);
    let png = wte_core::encode_png(w, h, &pic.data, wte_core::PngLevel::Fast).unwrap();
    state.clipboard.write(vec![(ClipFormat::Png, png)]).unwrap();
    let layers = state.doc().unwrap().set(S0).unwrap().layers.len();
    run_with(&ctx, &mut state, ClipAction::PasteAsLayer);
    let doc = state.doc_mut().unwrap();
    doc.set_active_layer(S0, Some(layer)).unwrap();
    doc.stroke_begin(&wte_core::BrushParams::default()).unwrap();
    doc.stroke_to_2d(S0, glam::Vec2::new(5.0, 5.0), 1.0);
    // Long enough for the worker to finish; the job stays while the stroke is open.
    for _ in 0..40 {
        crate::clip::poll(&mut state);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.clip_ui.busy(), "the paste waits");
    assert_eq!(state.doc().unwrap().set(S0).unwrap().layers.len(), layers);
    assert!(state.doc_mut().unwrap().stroke_end());
    for _ in 0..1000 {
        crate::clip::poll(&mut state);
        if !state.clip_ui.busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let set = state.doc().unwrap().set(S0).unwrap();
    assert_eq!(set.layers.len(), layers + 1, "pasted after the stroke");
    assert_eq!(dense(&state, set.active.unwrap()), pic);
}

/// check-P12: the close prompt's "linked since the last save" line does not name a paint layer deleted since (its
/// link stays for an undo, a save leaves it out); an undo of the delete names it again.
#[test]
fn check_p12_a_deleted_linked_layer_is_not_named_by_the_close_prompt() {
    let (mut state, layer, _dir) = layer_state("layer_deleted_prompt");
    edit_layer(&mut state, layer);
    assert!(unsaved_line(&state).unwrap().contains("Dirt (paint layer)"));
    state.doc_mut().unwrap().remove_layer(layer).unwrap();
    assert!(unsaved_line(&state).is_none_or(|l| !l.contains("(paint layer)")), "{:?}", unsaved_line(&state));
    state.doc_mut().unwrap().undo().unwrap();
    assert!(unsaved_line(&state).unwrap().contains("Dirt (paint layer)"));
}
