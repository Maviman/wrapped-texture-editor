//! Tests of package P6b (STICKER_PLAN 7 "P6b"): the stamp styles commands of the Sticker tab (no promotion, one entry
//! per drag, the automatic depth), presets, Copy / Paste / Clear Style (E5), the shared-picture question (E6), the
//! tray (E7) and "Edit before placing" (E4).

use glam::{Vec2, Vec3};
use wte_core::{Commit, Placement, Projector, ShareScope, StampId, StampSource, StylePreset, StyleStack, TextureSetId};

use super::tests::image_stamp_state;
use super::*;
use crate::action::AppAction;
use crate::state::{AppState, Dialog, StampToolSource, StickerTarget, ToolKind, ViewMode};

fn doc(state: &AppState) -> &Document {
    state.doc().unwrap()
}

fn undo_len(state: &AppState) -> usize {
    doc(state).history.undo_len()
}

fn shadow(size: f32) -> StyleStack {
    StyleStack {
        drop_shadow: Some(wte_core::Shadow { on: true, color: [0, 0, 0, 255], opacity: 0.6, angle_deg: 120.0, distance: 4.0, spread: 0.0, size }),
        ..StyleStack::default()
    }
}

/// Another image stamp of `blob` at `x` on the sphere's front.
fn another_stamp(state: &mut AppState, blob: BlobId, x: f32) -> StampId {
    let p = Projector::from_surface(Vec3::new(x, 0.0, 0.5), Vec3::Z, Vec3::Y, Vec2::splat(0.2), 0.6);
    state.doc_mut().unwrap().add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap()
}

/// Selects the home layers of `stamps` in the Layers list (the first one active), as Ctrl-clicks do.
fn select_layers(state: &mut AppState, stamps: &[StampId]) {
    let session = state.session.as_mut().unwrap();
    let layers: Vec<_> = stamps.iter().map(|s| session.doc.stamp_layers(*s)[0].1).collect();
    session.doc.set_active_layer(TextureSetId(0), Some(layers[0])).unwrap();
    session.selected_layers = layers;
    session.selected_stamp = Some(stamps[0]);
}

#[test]
fn a_style_edit_on_an_image_stamp_does_not_promote_and_a_drag_is_one_entry() {
    let (mut state, stamp, blob) = image_stamp_state(ViewMode::View3D);
    let undo = undo_len(&state);
    for size in [2.0, 5.0, 8.0] {
        run_in(&mut state, StickerAction::SetStampStyles(vec![stamp], Box::new(shadow(size)), Commit::Preview));
    }
    run_in(&mut state, StickerAction::SetStampStyles(vec![stamp], Box::new(shadow(8.0)), Commit::Final));
    assert_eq!(undo_len(&state), undo + 1, "one entry for the drag");
    let object = doc(&state).stamp(stamp).unwrap();
    assert_eq!(object.source, StampSource::Image { blob }, "a style edit never promotes");
    assert_eq!(object.styles, shadow(8.0).sanitized());
    assert!(doc(&state).stickers().next().is_none(), "no sticker was made");
    // A preset is one entry too, scaled to the picture (64 px).
    run_in(&mut state, StickerAction::ApplyPreset(StylePreset::ClassicSticker));
    assert_eq!(undo_len(&state), undo + 2);
    assert_eq!(doc(&state).stamp(stamp).unwrap().styles, StylePreset::ClassicSticker.styles([64, 64]));
    assert_eq!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { blob });
}

#[test]
fn a_style_slider_drag_on_a_stamp_with_an_automatic_depth_ends_with_a_refitted_depth() {
    let (mut state, _, blob) = image_stamp_state(ViewMode::View3D);
    let fitted = state.doc().unwrap().fit_stamp_depth(&Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::Y, Vec2::splat(0.3), 0.1));
    let stamp = state.doc_mut().unwrap().add_stamp("auto", StampSource::Image { blob }, Placement::Projected(fitted), TextureSetId(0)).unwrap();
    for size in [4.0, 10.0, 16.0] {
        run_in(&mut state, StickerAction::SetStampStyles(vec![stamp], Box::new(shadow(size)), Commit::Preview));
    }
    run_in(&mut state, StickerAction::SetStampStyles(vec![stamp], Box::new(shadow(16.0)), Commit::Final));
    let Placement::Projected(stored) = doc(&state).stamp(stamp).unwrap().placement else { panic!() };
    assert!(stored.depth_back > fitted.depth_back, "the depth grew with the shadow: {} -> {}", fitted.depth_back, stored.depth_back);
    let Some(Placement::Projected(bake)) = doc(&state).bake_placement(stamp) else { panic!() };
    assert!(doc(&state).stamp_depth_is_auto(&bake), "the stored depth is the automatic depth of the bake footprint");
    assert_eq!((stored.origin, stored.half_size), (fitted.origin, fitted.half_size), "the artwork box is unchanged");
}

#[test]
fn paste_style_onto_three_selected_stamps_is_one_entry() {
    let (mut state, first, blob) = image_stamp_state(ViewMode::View3D);
    let second = another_stamp(&mut state, blob, -0.3);
    let third = another_stamp(&mut state, blob, 0.3);
    let source = another_stamp(&mut state, blob, 0.0);
    // Copy from one stamp ...
    state.session.as_mut().unwrap().selected_stamp = Some(source);
    state.session.as_mut().unwrap().selected_layers.clear();
    run_in(&mut state, StickerAction::SetStampStyles(vec![source], Box::new(shadow(6.0)), Commit::Final));
    run_in(&mut state, StickerAction::CopyStyle(StyleTarget::Stamps));
    assert_eq!(state.style_clipboard, Some(shadow(6.0).sanitized()));
    // ... paste onto three selected layers' stamps: one entry.
    select_layers(&mut state, &[first, second, third]);
    assert_eq!(selected_stamps(&state), vec![first, second, third]);
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::PasteStyle(StyleTarget::Stamps));
    assert_eq!(undo_len(&state), undo + 1, "one entry");
    for s in [first, second, third] {
        assert_eq!(doc(&state).stamp(s).unwrap().styles, shadow(6.0).sanitized());
        assert!(matches!(doc(&state).stamp(s).unwrap().source, StampSource::Image { .. }));
    }
    // One undo takes it off all three.
    state.doc_mut().unwrap().undo();
    assert!([first, second, third].iter().all(|s| doc(&state).stamp(*s).unwrap().styles.is_default()));
    // A slider on the three: one entry, the same styles on each ("apply to all selected").
    let undo = undo_len(&state);
    let all = selected_stamps(&state);
    run_in(&mut state, StickerAction::SetStampStyles(all.clone(), Box::new(shadow(3.0)), Commit::Preview));
    run_in(&mut state, StickerAction::SetStampStyles(all, Box::new(shadow(4.0)), Commit::Final));
    assert_eq!(undo_len(&state), undo + 1);
    assert!([first, second, third].iter().all(|s| doc(&state).stamp(*s).unwrap().styles == shadow(4.0).sanitized()));
    // Clear Style: one entry, no styles.
    run_in(&mut state, StickerAction::ClearStyle(StyleTarget::Stamps));
    assert_eq!(undo_len(&state), undo + 2);
    assert!([first, second, third].iter().all(|s| doc(&state).stamp(*s).unwrap().styles.is_default()));
    // Paste without a copy says so.
    state.style_clipboard = None;
    run_in(&mut state, StickerAction::PasteStyle(StyleTarget::Stamps));
    assert_eq!(undo_len(&state), undo + 2);
    assert_eq!(state.status_hint, "Copy a style first");
}

#[test]
fn the_shared_picture_question_and_its_three_answers() {
    // Two image stamps show the same picture.
    let (mut state, stamp, blob) = image_stamp_state(ViewMode::View3D);
    let other = another_stamp(&mut state, blob, 0.3);
    enter_in(&mut state, Some(stamp));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }), "{:?}", state.dialog);
    assert_eq!(undo_len(&state), undo, "nothing changed before the answer");
    // Cancel: the edit is dropped; the question comes again next time.
    run_in(&mut state, StickerAction::AnswerShared(None));
    assert!(matches!(state.dialog, Dialog::None));
    assert_eq!(undo_len(&state), undo);
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }));
    // Only this one: this stamp alone is promoted, the waiting edit runs (one entry); no second question.
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }));
    run_in(&mut state, StickerAction::AnswerShared(Some(ShareScope::ThisStamp)));
    assert!(matches!(state.dialog, Dialog::None));
    assert_eq!(undo_len(&state), undo + 1);
    let StampSource::Sticker { sticker } = doc(&state).stamp(stamp).unwrap().source else { panic!("promoted") };
    assert_eq!(doc(&state).sticker(sticker).unwrap().layers.len(), 2, "the waiting add ran");
    assert_eq!(doc(&state).stamp(other).unwrap().source, StampSource::Image { blob }, "the other stamp is unchanged");
    run_in(&mut state, StickerAction::Nudge(1, 0));
    assert!(matches!(state.dialog, Dialog::None), "answered for this opening of the editor");
    leave_in(&mut state, None);

    // Edit all on a picture shown by two image stamps: both share the new sticker.
    let (mut state, stamp, blob) = image_stamp_state(ViewMode::View3D);
    let other = another_stamp(&mut state, blob, 0.3);
    enter_in(&mut state, Some(stamp));
    run_in(&mut state, StickerAction::SetLayerOpacity(StickerLayerId(1), 0.5, Commit::Preview));
    assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }));
    run_in(&mut state, StickerAction::AnswerShared(Some(ShareScope::AllUsers)));
    let a = doc(&state).stamp(stamp).unwrap().source.clone();
    assert!(matches!(a, StampSource::Sticker { .. }));
    assert_eq!(doc(&state).stamp(other).unwrap().source, a, "Edit all: both stamps show the edited sticker");
    let StampSource::Sticker { sticker } = a else { unreachable!() };
    assert_eq!(doc(&state).sticker(sticker).unwrap().layers[0].opacity, 0.5, "the waiting Preview ran as a Final");
    assert!(!doc(&state).has_pending_edit());
    leave_in(&mut state, None);

    // Only this one on a sticker two stamps share: the stamp gets its own copy first; the other keeps the old one.
    let mut state = state;
    let users = doc(&state).sticker_users(sticker);
    assert_eq!(users.len(), 2);
    enter_in(&mut state, Some(users[0]));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::SetLayerVisible(StickerLayerId(1), false));
    assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }));
    run_in(&mut state, StickerAction::AnswerShared(Some(ShareScope::ThisStamp)));
    let StampSource::Sticker { sticker: own } = doc(&state).stamp(users[0]).unwrap().source else { panic!() };
    assert_ne!(own, sticker, "detached");
    assert!(!doc(&state).sticker(own).unwrap().layers[0].visible);
    assert!(doc(&state).sticker(sticker).unwrap().layers[0].visible, "the other stamp's sticker is unchanged");
    assert_eq!(undo_len(&state), undo + 2, "Detach Sticker and the edit");
    // An unchanged name in a focused field never asks.
    leave_in(&mut state, None);
    let (mut state, stamp, blob) = image_stamp_state(ViewMode::View3D);
    another_stamp(&mut state, blob, 0.3);
    enter_in(&mut state, Some(stamp));
    run_in(&mut state, StickerAction::RenameLayer(StickerLayerId(1), "Logo".to_owned()));
    assert!(matches!(state.dialog, Dialog::None));
}

#[test]
fn a_style_on_the_selected_sticker_layers_is_a_content_edit() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, Some(stamp));
    let undo = undo_len(&state);
    for size in [2.0, 6.0] {
        run_in(&mut state, StickerAction::SetLayerStyles(StickerLayerId(1), Box::new(shadow(size)), Commit::Preview));
    }
    run_in(&mut state, StickerAction::SetLayerStyles(StickerLayerId(1), Box::new(shadow(6.0)), Commit::Final));
    assert_eq!(undo_len(&state), undo + 1, "promotion and the drag: one entry");
    let StampSource::Sticker { sticker } = doc(&state).stamp(stamp).unwrap().source else { panic!("promoted") };
    assert_eq!(doc(&state).sticker(sticker).unwrap().layers[0].styles, shadow(6.0).sanitized());
    assert!(doc(&state).stamp(stamp).unwrap().styles.is_default(), "the stamp's own styles are untouched");
    // Copy from the layer, a new layer, paste onto it; Clear.
    run_in(&mut state, StickerAction::CopyStyle(StyleTarget::Layers));
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    run_in(&mut state, StickerAction::PasteStyle(StyleTarget::Layers));
    let d = doc(&state).sticker(sticker).unwrap();
    assert_eq!(d.layers[1].styles, shadow(6.0).sanitized());
    run_in(&mut state, StickerAction::ClearStyle(StyleTarget::Layers));
    assert!(doc(&state).sticker(sticker).unwrap().layers[1].styles.is_default());
    run_in(&mut state, StickerAction::ApplyLayerPreset(StylePreset::Outline));
    let d = doc(&state).sticker(sticker).unwrap();
    assert_eq!(d.layers[1].styles, StylePreset::Outline.styles(d.size()));
}

#[test]
fn the_tray_makes_places_duplicates_and_deletes_stickers() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    assert!(tray::tray_stickers(&state).is_empty(), "an image stamp is no tray sticker");
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::NewSticker);
    assert_eq!(undo_len(&state), undo + 1);
    let items = tray::tray_stickers(&state);
    assert_eq!(items.len(), 1);
    let id = items[0];
    let d = doc(&state).sticker(id).unwrap();
    assert!(d.keep && d.size() == [tray::BLANK_SIDE, tray::BLANK_SIDE] && d.layers.len() == 1, "{d:?}");
    assert_eq!(state.tools.stamp.source, StampToolSource::Sticker(id), "the new sticker is the Stamp tool's source");
    assert_eq!(state.sticker_ui.tray_selected, Some(id));
    // A click on a tray item: the source, and outside the view the Stamp tool.
    state.tools.stamp.source = StampToolSource::None;
    run_in(&mut state, StickerAction::TrayUse(id));
    assert_eq!(state.tools.stamp.source, StampToolSource::Sticker(id));
    assert!(matches!(state.actions.as_slice(), [AppAction::SetTool(ToolKind::Stamp)]), "{:?}", state.actions);
    state.actions.clear();
    // Duplicate: a second item.
    run_in(&mut state, StickerAction::TrayDuplicate(id));
    let items = tray::tray_stickers(&state);
    assert_eq!(items.len(), 2);
    let copy = items[1];
    assert_eq!(doc(&state).sticker(copy).unwrap().name, format!("{} copy", doc(&state).sticker(id).unwrap().name));
    // A promoted stamp's sticker is in the tray while the stamp shows it.
    state.doc_mut().unwrap().promote_stamp(stamp, ShareScope::ThisStamp).unwrap();
    let StampSource::Sticker { sticker: promoted } = doc(&state).stamp(stamp).unwrap().source else { panic!() };
    assert!(tray::tray_stickers(&state).contains(&promoted));
    // Delete: refused while a stamp shows it; works for an unplaced one.
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::TrayDelete(promoted));
    assert_eq!(undo_len(&state), undo, "a sticker a stamp shows stays");
    assert!(doc(&state).sticker(promoted).is_some());
    state.tools.stamp.source = StampToolSource::Sticker(copy);
    run_in(&mut state, StickerAction::TrayDelete(copy));
    assert_eq!(undo_len(&state), undo + 1);
    assert!(doc(&state).sticker(copy).is_none());
    assert_eq!(state.tools.stamp.source, StampToolSource::None, "the deleted source is forgotten");
}

#[test]
fn edit_before_placing_makes_the_picture_a_tray_sticker_and_opens_it() {
    let (mut state, _, blob) = image_stamp_state(ViewMode::View3D);
    state.tool = ToolKind::Stamp;
    state.tools.stamp.source = StampToolSource::Blob(blob);
    state.tools.stamp.source_name = "logo.png".to_owned();
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::EditBeforePlacing);
    assert_eq!(undo_len(&state), undo + 1, "Add Sticker");
    let StampToolSource::Sticker(id) = state.tools.stamp.source else { panic!("{:?}", state.tools.stamp.source) };
    let d = doc(&state).sticker(id).unwrap();
    assert!(d.keep && d.name == "logo" && d.size() == [64, 64]);
    assert_eq!(state.view_mode, ViewMode::Sticker);
    let edit = edit_of(&state).unwrap();
    assert_eq!(edit.target, StickerTarget::Tray(id));
    assert_eq!(edit.active_layer, Some(d.layers[0].id));
    assert_eq!(state.status_hint, NOT_PLACED_HINT);
    // An edit of the tray sticker is one entry, never asks (no stamp shows it), and Revert sees it.
    run_in(&mut state, StickerAction::Nudge(3, 0));
    assert!(matches!(state.dialog, Dialog::None));
    assert_eq!(undo_len(&state), undo + 2);
    assert!(differs_from_entry(doc(&state), edit_of(&state).unwrap()));
    run_in(&mut state, StickerAction::Revert);
    assert!(!differs_from_entry(doc(&state), edit_of(&state).unwrap()));
    // Done: back to the 3D view with the Stamp tool and the sticker as its source.
    leave_in(&mut state, None);
    assert_eq!((state.view_mode, state.tool), (ViewMode::View3D, ToolKind::Stamp));
    assert_eq!(state.tools.stamp.source, StampToolSource::Sticker(id));
    // With the sticker as the source the same button opens it again, without a new entry.
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::EditBeforePlacing);
    assert_eq!(undo_len(&state), undo);
    assert_eq!(edit_of(&state).unwrap().target, StickerTarget::Tray(id));
    leave_in(&mut state, None);
    // Without a source: a hint.
    state.tools.stamp.source = StampToolSource::None;
    run_in(&mut state, StickerAction::EditBeforePlacing);
    assert_ne!(state.view_mode, ViewMode::Sticker);
    assert!(!state.status_hint.is_empty());
}

#[test]
fn export_picture_writes_the_finished_picture() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    run_in(&mut state, StickerAction::SetStampStyles(vec![stamp], Box::new(StylePreset::ClassicSticker.styles([64, 64])), Commit::Final));
    // P6b check: under target\, never %TEMP% (inside the real %LOCALAPPDATA%; plan 0.4 rule 5).
    let dir = wte_core::testutil::temp_dir("p6b_export");
    let path = dir.join("logo.png");
    assert!(tray::write_picture(&mut state, PictureOf::Stamp(stamp), &path));
    let bytes = std::fs::read(&path).unwrap();
    let img = wte_core::decode_image(&bytes, None, 8192).unwrap().pixels;
    assert!(img.width > 64 && img.height > 64, "the picture has room for the border and shadow: {}x{}", img.width, img.height);
    run_in(&mut state, StickerAction::NewSticker);
    let id = tray::tray_stickers(&state)[0];
    assert!(tray::write_picture(&mut state, PictureOf::Sticker(id), &path));
    let img = wte_core::decode_image(&std::fs::read(&path).unwrap(), None, 8192).unwrap().pixels;
    assert_eq!([img.width, img.height], [tray::BLANK_SIDE, tray::BLANK_SIDE]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn guarded_and_content_commands_of_p6b() {
    use StickerAction as S;
    assert!(S::SetStampStyles(vec![], Box::default(), Commit::Final).changes_document());
    assert!(!S::SetStampStyles(vec![], Box::default(), Commit::Preview).changes_document());
    assert!(!S::SetStampStyles(vec![], Box::default(), Commit::Final).is_content(), "stamp styles never ask E6");
    assert!(S::SetLayerStyles(StickerLayerId(1), Box::default(), Commit::Preview).is_content());
    assert!(S::PasteStyle(StyleTarget::Layers).is_content() && !S::PasteStyle(StyleTarget::Stamps).is_content());
    assert!(!S::CopyStyle(StyleTarget::Layers).changes_document());
    assert!(!S::TrayUse(StickerId(1)).changes_document() && S::TrayDelete(StickerId(1)).changes_document());
    assert!(!S::AnswerShared(None).changes_document() && S::AnswerShared(Some(ShareScope::AllUsers)).changes_document());
}

/// P6b check: "Only this one" whose detach is refused (the edited stamp is locked) leaves the question unanswered: the
/// next edit asks again instead of reaching the sticker the other stamp still shares.
#[test]
fn a_refused_detach_leaves_the_shared_picture_question_open() {
    let (mut state, stamp, blob) = image_stamp_state(ViewMode::View3D);
    let other = another_stamp(&mut state, blob, 0.3);
    // Both stamps show one sticker ("Edit all" on the shared picture).
    enter_in(&mut state, Some(stamp));
    run_in(&mut state, StickerAction::SetLayerOpacity(StickerLayerId(1), 0.5, Commit::Final));
    run_in(&mut state, StickerAction::AnswerShared(Some(ShareScope::AllUsers)));
    let StampSource::Sticker { sticker } = doc(&state).stamp(stamp).unwrap().source else { panic!("promoted") };
    assert_eq!(doc(&state).sticker_users(sticker), vec![stamp, other]);
    leave_in(&mut state, None);

    enter_in(&mut state, Some(stamp));
    let layer = doc(&state).stamp_layers(stamp)[0].1;
    state.doc_mut().unwrap().set_layer_locked(layer, true).unwrap();
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::Nudge(5, 0));
    assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }));
    run_in(&mut state, StickerAction::AnswerShared(Some(ShareScope::ThisStamp)));
    assert_eq!(undo_len(&state), undo, "the detach was refused: nothing changed");
    assert_eq!(edit_of(&state).unwrap().share_answer, None, "the question is still open");
    // Unlocked again, the next edit asks again; the shared sticker is untouched.
    state.doc_mut().unwrap().set_layer_locked(layer, false).unwrap();
    let before = doc(&state).sticker(sticker).unwrap().layers[0].transform;
    run_in(&mut state, StickerAction::Nudge(5, 0));
    assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }), "{:?}", state.dialog);
    assert_eq!(doc(&state).sticker(sticker).unwrap().layers[0].transform, before);
    run_in(&mut state, StickerAction::AnswerShared(Some(ShareScope::ThisStamp)));
    let StampSource::Sticker { sticker: own } = doc(&state).stamp(stamp).unwrap().source else { panic!() };
    assert_ne!(own, sticker, "detached now");
    assert_eq!(doc(&state).sticker(sticker).unwrap().layers[0].transform, before, "the other stamp's sticker is unchanged");
}

/// P6b check: Clear Style on the selected stamps is one entry named as a style edit (it was named "Paste Style").
#[test]
fn clear_style_on_stamps_is_one_style_entry() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    run_in(&mut state, StickerAction::ApplyPreset(StylePreset::ClassicSticker));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::ClearStyle(StyleTarget::Stamps));
    assert_eq!(undo_len(&state), undo + 1);
    assert_eq!(doc(&state).undo_label(), Some(wte_core::doc_stickers::STYLE_LABEL));
    assert!(doc(&state).stamp(stamp).unwrap().styles.is_default());
    // Nothing left to clear: no entry.
    run_in(&mut state, StickerAction::ClearStyle(StyleTarget::Stamps));
    assert_eq!(undo_len(&state), undo + 1);
    // Paste keeps its own name.
    state.style_clipboard = Some(shadow(5.0));
    run_in(&mut state, StickerAction::PasteStyle(StyleTarget::Stamps));
    assert_eq!(doc(&state).undo_label(), Some("Paste Style"));
}
