//! Tests of `clip.rs` (STICKER_PLAN 7 "P10b"): the routing of the chords on recorded inputs (a)-(d), the text copied
//! in an egui field, copies of a stamp, a paint layer and the Sticker view's pixels (in-memory clipboard), Paste as
//! the Stamp tool's source, Paste at View Centre, the "Nothing WTE can paste" sentence and Help > Clipboard contents.

use super::*;
use crate::ole::clipboard::MemClipboard;
use crate::state::{Prefs, StampToolSource, ToolKind};
use crate::tools::Tool;
use crate::tools::test_support::{at, pointer, state_with, xf_3d};
use wte_core::{Commit, SampleKind, StylePreset};

fn mem_with_text(text: &str) -> MemClipboard {
    let mut cb = MemClipboard::default();
    clipboard::write_text(&mut cb, text).unwrap();
    cb
}

/// (a) `Event::Copy` / `Event::Cut` without a text field: removed, Copy / Cut queued; with one: left for egui.
#[test]
fn copy_and_cut_events_go_to_wte_without_a_text_field() {
    let cb = MemClipboard::default();
    let mut events = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)), egui::Event::Copy, egui::Event::Cut];
    let out = route(&mut events, &[], false, false, &cb);
    assert_eq!(out, [ClipAction::Copy, ClipAction::Cut]);
    assert_eq!(events, [egui::Event::PointerMoved(egui::pos2(1.0, 2.0))]);
    let mut events = vec![egui::Event::Copy, egui::Event::Cut];
    assert!(route(&mut events, &[], true, false, &cb).is_empty());
    assert_eq!(events, [egui::Event::Copy, egui::Event::Cut], "a text field keeps its copy and cut");
    // A dialog in front: the events are taken out of the way of nothing, and no command runs.
    let mut events = vec![egui::Event::Copy];
    assert!(route(&mut events, &[], false, true, &cb).is_empty());
}

/// (b) chord + text field + egui-winit's in-app `Event::Paste("old")`: exactly one `Event::Paste(clipboard text)`.
#[test]
fn the_paste_chord_in_a_text_field_pastes_the_clipboards_text() {
    let cb = mem_with_text("new\r\nline");
    let mut events = vec![egui::Event::Paste("old".into()), egui::Event::Text("t".into()), egui::Event::Paste("old".into())];
    let out = route(&mut events, &[Chord::Paste], true, false, &cb);
    assert!(out.is_empty());
    let pastes: Vec<&egui::Event> = events.iter().filter(|e| matches!(e, egui::Event::Paste(_))).collect();
    assert_eq!(pastes, [&egui::Event::Paste("new\nline".into())]);
    assert!(events.contains(&egui::Event::Text("t".into())));
    // Also when a dialog is in front (its text fields paste).
    let mut events = Vec::new();
    route(&mut events, &[Chord::Paste], true, true, &cb);
    assert_eq!(events, [egui::Event::Paste("new\nline".into())]);
}

/// (c) chord without a text field: `Paste` is queued and no paste event is left.
#[test]
fn the_paste_chord_without_a_text_field_is_wtes_paste() {
    let cb = mem_with_text("text");
    let mut events = vec![egui::Event::Paste("old".into())];
    assert_eq!(route(&mut events, &[Chord::Paste], false, false, &cb), [ClipAction::Paste]);
    assert!(events.is_empty());
    let mut events = Vec::new();
    assert_eq!(route(&mut events, &[Chord::Paste], false, false, &cb), [ClipAction::Paste]);
    // Blocked (a dialog): no command.
    let mut events = Vec::new();
    assert!(route(&mut events, &[Chord::Paste], false, true, &cb).is_empty());
    // No chord: egui-winit's own paste event (its in-app string) stays as it is.
    let mut events = vec![egui::Event::Paste("old".into())];
    assert!(route(&mut events, &[], false, false, &cb).is_empty());
    assert_eq!(events.len(), 1);
}

/// (d) chord with an empty in-memory clipboard and a text field: egui-winit's event is kept.
#[test]
fn without_text_on_the_clipboard_egui_winits_paste_is_kept() {
    let cb = MemClipboard::default();
    let mut events = vec![egui::Event::Paste("in-app".into())];
    assert!(route(&mut events, &[Chord::Paste], true, false, &cb).is_empty());
    assert_eq!(events, [egui::Event::Paste("in-app".into())]);
    // A picture but no text: the same.
    let mut cb = MemClipboard::default();
    cb.write(vec![(ClipFormat::Png, b"\x89PNG".to_vec())]).unwrap();
    let mut events = vec![egui::Event::Paste("in-app".into())];
    route(&mut events, &[Chord::Paste], true, false, &cb);
    assert_eq!(events, [egui::Event::Paste("in-app".into())]);
}

/// The text an egui field copies (`OutputCommand::CopyText`) reaches the clipboard at the end of `App::ui`, and the
/// command stays for egui-winit.
#[test]
fn copy_text_at_the_end_of_ui_reaches_the_clipboard() {
    let ctx = egui::Context::default();
    let mut state = AppState::new(Prefs::default(), true);
    assert!(state.clipboard.is_memory());
    crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| {
        ui.ctx().copy_text("copied out".into());
        after_ui(ui.ctx(), &mut state);
        let kept = ui.ctx().output(|o| o.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "copied out")));
        assert!(kept, "the command stays");
    });
    assert_eq!(clipboard::read_text(&*state.clipboard).unwrap().as_deref(), Some("copied out"));
    // A frame without a copy leaves the clipboard alone.
    crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| after_ui(ui.ctx(), &mut state));
    assert_eq!(clipboard::read_text(&*state.clipboard).unwrap().as_deref(), Some("copied out"));
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut p = PixelBuf::new(w, h);
    for px in p.data.as_chunks_mut::<4>().0 {
        px.copy_from_slice(&rgba);
    }
    wte_core::encode_png(w, h, &p.data, wte_core::PngLevel::Fast).unwrap()
}

/// Waits for the paste's drop worker and applies what it prepared (as `dnd::handle` + dispatch do).
fn settle(ctx: &egui::Context, state: &mut AppState) {
    for _ in 0..500 {
        crate::dnd::handle_events(ctx, state, Vec::new());
        let ready: Vec<_> = state.actions.iter().filter_map(|a| if let AppAction::DropReady(p) = a { Some(Arc::clone(p)) } else { None }).collect();
        if !ready.is_empty() {
            state.actions.retain(|a| !matches!(a, AppAction::DropReady(_)));
            for p in ready {
                crate::dnd::apply(state, &p);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the paste was not prepared");
}

fn undo_len(state: &AppState) -> usize {
    state.doc().unwrap().history.undo_len()
}

/// Paste outside the Sticker view: the picture becomes the Stamp tool's source and the Stamp tool is chosen; nothing
/// is placed until a click, and the click places it.
#[test]
fn paste_makes_the_stamp_tools_source_and_a_click_places_it() {
    let ctx = egui::Context::default();
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    state.clipboard.write(vec![(ClipFormat::Png, png(30, 20, [10, 200, 30, 255]))]).unwrap();
    run_with(&ctx, &mut state, ClipAction::Paste);
    settle(&ctx, &mut state);
    assert!(matches!(state.tools.stamp.source, StampToolSource::Blob(_)), "{:?}", state.tools.stamp.source);
    assert_eq!(state.tools.stamp.source_name, clipboard::PASTED_PICTURE);
    assert!(state.actions.iter().any(|a| matches!(a, AppAction::SetTool(ToolKind::Stamp))), "the Stamp tool is chosen");
    assert!(state.doc().unwrap().stamps.is_empty(), "nothing is placed by the paste");
    assert_eq!(undo_len(&state), 0);
    // The click.
    state.tool = ToolKind::Stamp;
    let p = pointer(&state, &xf, at(&xf, 0.5, 0.5), egui::Modifiers::NONE);
    crate::tools::stamp_place::StampPlaceTool::default().press(&mut state, &xf, &p);
    assert_eq!(state.doc().unwrap().stamps.len(), 1);
    assert_eq!(state.doc().unwrap().stamps[0].name, "Pasted picture");
}

/// Paste at View Centre places one stamp at once (one entry); in the corner of nowhere (no view) it is a plain Paste.
#[test]
fn paste_at_view_centre_places_one_stamp() {
    let ctx = egui::Context::default();
    let mut state = state_with(SampleKind::Sphere);
    xf_3d(&mut state);
    state.view_mode = ViewMode::View3D;
    state.clipboard.write(vec![(ClipFormat::Png, png(16, 16, [200, 0, 0, 255]))]).unwrap();
    assert!(matches!(paste_spot(&state, true), Spot::View(ViewId::Main3D, _)));
    assert_eq!(paste_spot(&state, false), Spot::Elsewhere);
    run_with(&ctx, &mut state, ClipAction::PasteAtViewCentre);
    settle(&ctx, &mut state);
    assert_eq!(state.doc().unwrap().stamps.len(), 1, "one stamp placed");
    assert_eq!(undo_len(&state), 1, "one undo step");
    assert_eq!(state.doc().unwrap().undo_label(), Some("Add Stamp"), "one edit keeps its own label");
    // Check P10b: a sticker pasted at the centre is two edits (the tray sticker, the stamp) in one step, labelled
    // "Paste" (it said "Drop").
    let sticker = wte_core::StickerDoc::from_picture(
        "Pasted",
        wte_core::PicturePixels::Owned { pixels: Arc::new(PixelBuf::new(8, 8)), version: 0 },
        [8, 8],
    );
    let wtes = wte_core::write_wtes(&sticker, None, &wte_core::BlobStore::default(), None).unwrap();
    state.clipboard.write(vec![(ClipFormat::Sticker, crate::ole::formats::wrap_sticker(1, &wtes))]).unwrap();
    run_with(&ctx, &mut state, ClipAction::PasteAtViewCentre);
    settle(&ctx, &mut state);
    assert_eq!(state.doc().unwrap().stamps.len(), 2);
    assert_eq!(undo_len(&state), 2, "one more undo step");
    assert_eq!(state.doc().unwrap().undo_label(), Some(PASTE_LABEL));
    // No 3D view laid out: a plain Paste.
    state.view_rects.clear();
    assert_eq!(paste_spot(&state, true), Spot::Elsewhere);
    // Preview: a plain Paste too.
    state.view_mode = ViewMode::Preview;
    assert_eq!(paste_spot(&state, true), Spot::Elsewhere);
}

/// Copy of a selected sticker stamp: PNG + CF_DIBV5 of the finished picture with its styles and WTE.Sticker.v1 with
/// the stamp styles; pasted back it is a tray sticker and the Stamp tool's source, with those styles.
#[test]
fn copy_of_a_sticker_stamp_round_trips_through_the_clipboard() {
    let ctx = egui::Context::default();
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    // A sticker stamp: a picture placed and promoted, then given the Outline preset.
    let item = crate::ole::DropItem::Data { name: "Badge.png".into(), kind: crate::ole::DataKind::Png, bytes: png(32, 24, [30, 60, 220, 255]) };
    let prepared = crate::dnd::prepare(Spot::View(ViewId::Main3D, at(&xf, 0.5, 0.5)), vec![item]);
    crate::dnd::apply(&mut state, &prepared);
    let stamp = state.session.as_ref().unwrap().selected_stamp.expect("a stamp");
    let doc = state.doc_mut().unwrap();
    doc.promote_stamp(stamp, wte_core::ShareScope::ThisStamp).unwrap();
    let styles = StylePreset::Outline.styles([32, 24]);
    doc.set_stamp_styles(stamp, styles.clone(), Commit::Final).unwrap();
    let expected = doc.stamp_picture(stamp, true).unwrap();
    run_with(&ctx, &mut state, ClipAction::Copy);
    finish_copies(&mut state);
    assert_eq!(state.clip_ui.copies, 1);
    assert_eq!(state.clipboard.format_names().unwrap(), ["PNG", "CF_DIBV5", "WTE.Sticker.v1"]);
    let png_bytes = state.clipboard.read(ClipFormat::Png).unwrap().unwrap();
    assert_eq!(wte_core::decode_image(&png_bytes, Some("x.png"), 16384).unwrap().pixels, expected, "what you see, with its styles");
    let dib = state.clipboard.read(ClipFormat::DibV5).unwrap().unwrap();
    assert_eq!(crate::ole::formats::dib_to_pixels(&dib).unwrap(), expected, "alpha kept");
    // Pasted back (WTE.Sticker.v1 first): a tray sticker, the Stamp tool's source, the stamp styles to the tool.
    state.tools.stamp.styles = wte_core::StyleStack::default();
    let before = state.doc().unwrap().stickers().count();
    run_with(&ctx, &mut state, ClipAction::Paste);
    settle(&ctx, &mut state);
    let StampToolSource::Sticker(id) = state.tools.stamp.source else { panic!("{:?}", state.tools.stamp.source) };
    assert_eq!(state.doc().unwrap().stickers().count(), before + 1);
    assert!(state.doc().unwrap().sticker(id).unwrap().keep, "a tray sticker");
    assert_eq!(state.tools.stamp.styles, styles);
    assert_eq!(state.doc().unwrap().stamps.len(), 1, "nothing placed");
}

/// Copy of an image stamp without styles offers no sticker (a paste gives an image stamp again); Cut outside the
/// Sticker view copies nothing; nothing selected and an empty layer: a status line.
#[test]
fn copy_of_an_image_stamp_a_paint_layer_and_nothing() {
    let ctx = egui::Context::default();
    let mut state = state_with(SampleKind::Sphere);
    run_with(&ctx, &mut state, ClipAction::Copy);
    finish_copies(&mut state);
    assert_eq!(state.status_hint, NOTHING_TO_COPY, "an unpainted layer has nothing to copy");
    assert!(state.clipboard.format_names().unwrap().is_empty());
    // A painted paint layer: its picture at texture size.
    let session = state.session.as_mut().unwrap();
    let set = session.active_set;
    let layer = session.doc.set(set).unwrap().active_layer().unwrap().id;
    session.doc.fill(set, layer, glam::Vec2::new(0.5, 0.5), [250, 10, 10, 255], wte_core::FillScope::Layer).unwrap();
    let (w, h) = { let s = state.doc().unwrap().set(set).unwrap(); (s.width, s.height) };
    run_with(&ctx, &mut state, ClipAction::Copy);
    finish_copies(&mut state);
    assert_eq!(state.clipboard.format_names().unwrap(), ["PNG", "CF_DIBV5"]);
    let got = wte_core::decode_image(&state.clipboard.read(ClipFormat::Png).unwrap().unwrap(), Some("x.png"), 16384).unwrap().pixels;
    assert_eq!((got.width, got.height), (w, h), "texture size");
    assert_eq!(&got.data[..4], &[250, 10, 10, 255]);
    // An image stamp without styles: no WTE.Sticker.v1.
    let xf = xf_3d(&mut state);
    let item = crate::ole::DropItem::Data { name: "Plain.png".into(), kind: crate::ole::DataKind::Png, bytes: png(8, 8, [1, 2, 3, 255]) };
    crate::dnd::apply(&mut state, &crate::dnd::prepare(Spot::View(ViewId::Main3D, at(&xf, 0.5, 0.5)), vec![item]));
    run_with(&ctx, &mut state, ClipAction::Copy);
    finish_copies(&mut state);
    assert_eq!(state.clipboard.format_names().unwrap(), ["PNG", "CF_DIBV5"]);
    // Cut outside the Sticker view: a status line, the clipboard unchanged.
    let copies = state.clip_ui.copies;
    run_with(&ctx, &mut state, ClipAction::Cut);
    finish_copies(&mut state);
    assert_eq!(state.status_hint, CUT_HINT);
    assert_eq!(state.clip_ui.copies, copies);
}

/// The Sticker view's copy: the layer's pixels with its mask applied, cut to the selection's pixels.
#[test]
fn a_sticker_layer_copy_takes_the_selected_pixels_with_the_mask() {
    let mut pixels = PixelBuf::new(8, 6);
    for (i, px) in pixels.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *px = [i as u8, 50, 60, 255];
    }
    let mut mask = AlphaBuf::new(8, 6);
    mask.data.fill(255);
    mask.data[8 * 2 + 3] = 0; // pixel (3, 2) masked out
    let identity = wte_core::LayerTransform::default();
    let artboard = IRect::new(0, 0, 8, 6);
    // No selection: the whole layer, mask applied.
    let all = layer_pixels(&pixels, Some(&mask), None, artboard, &identity).unwrap();
    assert_eq!((all.width, all.height), (8, 6));
    assert_eq!(all.data[(2 * 8 + 3) * 4 + 3], 0);
    assert_eq!(all.data[(2 * 8 + 4) * 4 + 3], 255);
    // A selection of x 2..5, y 1..4: cropped to it.
    let mut sel = AlphaBuf::new(8, 6);
    for y in 1..4 {
        for x in 2..5 {
            sel.data[y * 8 + x] = 255;
        }
    }
    let part = layer_pixels(&pixels, Some(&mask), Some(&sel), artboard, &identity).unwrap();
    assert_eq!((part.width, part.height), (3, 3));
    assert_eq!(part.data[0], (8 + 2) as u8, "the selection's first pixel");
    assert_eq!(part.data[(3 + 1) * 4 + 3], 0, "the masked pixel stays transparent");
    // A selection beside the pixels holds none of them.
    let empty = AlphaBuf::new(8, 6);
    assert_eq!(layer_pixels(&pixels, None, Some(&empty), artboard, &identity).unwrap_err(), crate::sticker::tools::move_selection::NOTHING_SELECTED_HINT);
}

/// 13.5: a paste that finds nothing usable names what the clipboard offers; Help > Clipboard contents... lists the
/// in-memory formats.
#[test]
fn nothing_to_paste_and_the_contents_dialog() {
    let ctx = egui::Context::default();
    let mut state = state_with(SampleKind::Sphere);
    let mut cb = mem_with_text("words");
    cb.put_named("Photoshop Clipboard", vec![0; 4]);
    state.clipboard = Box::new(cb);
    run_with(&ctx, &mut state, ClipAction::Paste);
    let toasts = state.toasts.visible_labels();
    assert!(
        toasts.iter().any(|t| t == "Nothing WTE can paste. The clipboard offers: CF_UNICODETEXT, Photoshop Clipboard."),
        "{toasts:?}"
    );
    assert!(!state.drop_ui.busy(), "nothing is prepared");
    assert_eq!(nothing_text(&[]), "Nothing WTE can paste. The clipboard offers: nothing.");
    run_with(&ctx, &mut state, ClipAction::ShowContents);
    let Some(AppAction::ShowDialog(Dialog::Message { title, lines })) = state.actions.last() else { panic!("{:?}", state.actions) };
    assert_eq!(title, "Clipboard contents");
    assert!(lines.iter().any(|l| l.trim() == "CF_UNICODETEXT") && lines.iter().any(|l| l.trim() == "Photoshop Clipboard"), "{lines:?}");
    state.clipboard = Box::new(MemClipboard::default());
    run_with(&ctx, &mut state, ClipAction::ShowContents);
    let Some(AppAction::ShowDialog(Dialog::Message { lines, .. })) = state.actions.last() else { panic!() };
    assert_eq!(lines, &["The clipboard is empty."]);
}

/// Check P10b: a later Copy wins over an earlier one still being encoded. Before the fix `poll` wrote every finished
/// copy in the order the workers finished, so a slow first copy (a 4096 x 4096 paint layer) that finished after a quick
/// second one (a stamp) put the older picture on the clipboard; and every repeat of a held Ctrl+C encoded at once.
#[test]
fn check_p10b_a_later_copy_wins_over_a_slower_earlier_one() {
    let ctx = egui::Context::default();
    let mut state = AppState::new(Prefs::default(), true);
    let solid = |w: u32, h: u32, rgba: [u8; 4]| {
        let mut p = PixelBuf::new(w, h);
        for px in p.data.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&rgba);
        }
        p
    };
    // The first copy's worker waits until the test lets it go.
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let slow = solid(6, 4, [255, 0, 0, 255]);
    start_copy(
        &ctx,
        &mut state,
        "slow".into(),
        Box::new(move || {
            let _ = started_tx.send(());
            let _ = release_rx.recv();
            Ok((Arc::new(slow), None))
        }),
    );
    started_rx.recv().unwrap();
    // The second copy is quick and is written first.
    let fast = solid(2, 3, [0, 255, 0, 255]);
    let expected = fast.clone();
    start_copy(&ctx, &mut state, "fast".into(), Box::new(move || Ok((Arc::new(fast), None))));
    let pasted = |state: &AppState| {
        let png = state.clipboard.read(ClipFormat::Png).unwrap().unwrap();
        wte_core::decode_image(&png, Some("x.png"), 64).unwrap().pixels
    };
    for _ in 0..400 {
        poll(&mut state);
        if state.clip_ui.copies > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(state.clip_ui.copies, 1, "the quick copy is written");
    assert_eq!(pasted(&state), expected);
    assert!(!state.clip_ui.busy(), "the superseded copy is not waited for");
    // The first worker finishes now: nothing of it reaches the clipboard.
    release_tx.send(()).unwrap();
    for _ in 0..20 {
        poll(&mut state);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    finish_copies(&mut state);
    assert_eq!(state.clip_ui.copies, 1, "the older copy is never written");
    assert_eq!(pasted(&state), expected, "the clipboard holds the latest copy");
}
