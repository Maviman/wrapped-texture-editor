// Fixture tests of package P5c (STICKER_PLAN 7 "P5c", 8.3, 13.4): the comparison rule of 5.4 on files the real
// PhotoCraft v0.2.0 CLI re-saved (f1, f1w and the new f15, a link export with a text, a shape and a mask), the rows of
// the mapping table that PhotoCraft fixtures show (f5 text with pixels, f12 `lyid` reuse, f11 clipping, f4 / f6 / f8b
// merged, f4b mask, f8 properties), and the Photoshop-saved files that decide a link's path (smart object per editor,
// type layer, effects, adjustment, group, MaxCompat off, a refused colour mode, a profile note).

use std::path::PathBuf;

use super::map::LayerOutcome;
use super::tests::*;
use super::*;
use crate::doc_links::LinkApplied;
use crate::document::Document;
use crate::ids::StickerId;
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::psd::{PsdDoc, PsdError, PsdLimits, PsdRefusal};
use crate::sticker::{PicturePixels, StickerContent, StickerDoc};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/psd")
}

fn bytes(rel: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn fixture(rel: &str) -> PsdDoc {
    crate::psd::read(&bytes(rel), &PsdLimits::default()).unwrap_or_else(|e| panic!("{rel}: {e:?}"))
}

/// A document with an empty sticker linked to a file with an empty map (what "Import PSD as sticker" or a link to a
/// foreign file starts from: every layer of the first file is new).
fn fresh(editor: &str) -> (Document, StickerId) {
    let mut doc = base_doc();
    let sid = doc.add_sticker(StickerDoc::new("Fresh", 8, 8)).unwrap();
    let link = StickerLink {
        file: LinkFile::InFolder("Fresh_k1.psd".into()),
        editor: editor.to_owned(),
        mode: LinkMode::Layers,
        scale: 1.0,
        synced: FileStamp::default(),
        synced_sticker: 0,
        layer_map: Vec::new(),
        next_lyid: 1,
        editor_native: false,
        problem: None,
    };
    doc.put_link(sid, Some(link));
    (doc, sid)
}

/// Applies fixture `rel` to the link of `sid`.
fn apply(doc: &mut Document, sid: StickerId, rel: &str, editor: (&str, &str)) -> LinkApplied {
    let b = bytes(rel);
    let psd = crate::psd::read(&b, &PsdLimits::default()).unwrap();
    doc.apply_link_update(sid, &psd, FileStamp::of_bytes(&b, None), editor.1, rel).unwrap()
}

fn sticker(doc: &Document, sid: StickerId) -> StickerDoc {
    (**doc.sticker(sid).unwrap()).clone()
}

fn owned_pixels(l: &crate::sticker::StickerLayer) -> &crate::pixel::PixelBuf {
    match &l.content {
        StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => pixels,
        _ => panic!("{} is not an owned picture", l.name),
    }
}

// ----- f15: a link export re-saved by the real PhotoCraft (the comparison rule with text, shape and mask) -----

/// Writes the export of the test sticker (`tests::linked_doc`, PhotoCraft link) to `target\sticker\p5c\` (or
/// `WTE_F15_OUT`): the input of `make_f15.sh`.
#[test]
#[ignore = "writes the f15 input file; run by hand to remake the fixture"]
fn k5_link_make_f15_input() {
    let out = std::env::var_os("WTE_F15_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sticker/p5c/f15_link_wte.psd"));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let (mut doc, sid, _) = linked_doc();
    std::fs::write(&out, make_link(&mut doc, sid, EDITOR_PHOTOCRAFT)).unwrap();
}

#[test]
fn k5_link_f15_photocraft_resave_comes_back_as_the_same_sticker() {
    // `f15_link_wte.psd` is this build's export of the test sticker; `f15_link_pc.psd` is it re-saved headless by the
    // real PhotoCraft v0.2.0 CLI (`convert`, `make_f15.sh`). Every layer is recognised as untouched (the text stays
    // text, the shape a shape, the mask the same), so importing gives back the sticker that was exported.
    let (mut doc, sid, _) = linked_doc();
    let written = make_link(&mut doc, sid, EDITOR_PHOTOCRAFT);
    assert!(written == bytes("f15_link_wte.psd"), "the export still writes the committed input");
    let before = sticker(&doc, sid);
    let pc = fixture("f15_link_pc.psd");
    assert_eq!(pc.layers.iter().map(|l| l.lyid).collect::<Vec<_>>(), [Some(1), Some(2), Some(3), Some(4)]);
    assert!(pc.layers[3].mask.is_some(), "PhotoCraft keeps the mask");
    let made = import_now(&doc, sid, &pc, PC).unwrap();
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.ids_lost, None);
    assert_eq!(made.sticker, before);
    let applied = doc.apply_link_update(sid, &pc, FileStamp::of_bytes(&bytes("f15_link_pc.psd"), None), "PhotoCraft", "f15.psd").unwrap();
    assert!(matches!(applied, LinkApplied::Updated { mode: LinkMode::Layers, .. }));
    assert_eq!(sticker(&doc, sid), before);
    assert!(matches!(by_name(&before, "text").content, StickerContent::Text(_)));
}

// ----- f1 / f1w: untouched although PhotoCraft re-trimmed or cleared the hidden RGB -----

#[test]
fn k5_link_f1_and_f1w_untouched_layers_survive_photocrafts_resave() {
    for (input, resaved) in [("f1_wte_in.psd", "f1_pc.psd"), ("f1w_wte_in.psd", "f1w_pc.psd")] {
        let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
        apply(&mut doc, sid, input, PC);
        let first = sticker(&doc, sid);
        assert_eq!(names(&first), ["photo", "text", "shape"], "{input}");
        assert_eq!(first.artboard, IRect::new(0, 0, 64, 48));
        assert_eq!(doc.link(sid).unwrap().layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 2, 3], "{input}");
        let pc = fixture(resaved);
        assert_ne!(bytes(input), bytes(resaved), "{resaved} is PhotoCraft's own file (its resources, blocks, ranges)");
        let made = import_now(&doc, sid, &pc, PC).unwrap();
        assert!(all_untouched(&made.outcomes), "{resaved}: {:?}", made.outcomes);
        assert_eq!(made.ids_lost, None, "{resaved}: PhotoCraft's ids 1..3 are the map's");
        assert_eq!(made.sticker, first, "{resaved}");
    }
}

#[test]
fn k5_link_photocraft_fixtures_text_reuse_clip_merged_props() {
    // The base: f1_pc linked fresh (lyid 1..3).
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    apply(&mut doc, sid, "f1_pc.psd", PC);
    let base = sticker(&doc, sid);
    let id = |n: &str| by_name(&base, n).id;
    // f5: a PhotoCraft TEXT layer with pixels -> one new picture layer, every WTE layer kept, editor_native.
    let made = import_now(&doc, sid, &fixture("f5_f1_plus_text.psd"), PC).unwrap();
    assert_eq!(made.mode, LinkMode::Layers);
    assert_eq!(
        &made.outcomes[..3],
        [LayerOutcome::Untouched(id("photo")), LayerOutcome::Untouched(id("text")), LayerOutcome::Untouched(id("shape"))]
    );
    assert!(matches!(made.outcomes[3], LayerOutcome::New(_)));
    assert_eq!(names(&made.sticker), ["photo", "text", "shape", "pc text"]);
    assert!(made.editor_native);
    // The same text layer on a Photoshop link: TEXT is never forcing.
    assert_eq!(import_now(&doc, sid, &fixture("f5_f1_plus_text.psd"), PS).unwrap().mode, LinkMode::Layers);
    // f12: "text" deleted in PhotoCraft and a new layer took its id 2 -> deleted + new.
    let made = import_now(&doc, sid, &fixture("f12_lyid_reuse.psd"), PC).unwrap();
    assert_eq!(made.ids_lost, None);
    assert!(made.outcomes.contains(&LayerOutcome::Deleted(id("text"))), "{:?}", made.outcomes);
    assert!(made.outcomes.contains(&LayerOutcome::Untouched(id("photo"))));
    assert!(made.outcomes.contains(&LayerOutcome::Untouched(id("shape"))));
    assert_eq!(names(&made.sticker), ["photo", "shape", "pc new"]);
    assert_ne!(by_name(&made.sticker, "pc new").id, id("text"));
    assert_eq!(made.layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 3, 2]);
    // f10: the same file as PSB.
    let made = import_now(&doc, sid, &fixture("f10_f1.psb"), PC).unwrap();
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);

    // f11: a clipped layer -> clip.
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    apply(&mut doc, sid, "f11_clipped.psd", PC);
    let s = sticker(&doc, sid);
    assert_eq!(s.layers.iter().map(|l| l.clip).collect::<Vec<_>>(), [false, true]);
    // f4: a live stroke effect -> merged with its sentence; f4b (the same without the effect): the mask comes back.
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    let LinkApplied::Updated { mode, toast, .. } = apply(&mut doc, sid, "f4_mask_stroke.psd", PC) else { panic!() };
    assert_eq!(mode, LinkMode::Merged);
    assert_eq!(
        toast.as_deref(),
        Some(
            "This sticker now uses layer effects, which only PhotoCraft can show. WTE shows the finished picture; keep editing its layers in PhotoCraft."
        )
    );
    assert_eq!(names(&sticker(&doc, sid)), ["PhotoCraft picture"]);
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    apply(&mut doc, sid, "f4b_mask.psd", PC);
    let s = sticker(&doc, sid);
    let l = &s.layers[0];
    let m = l.mask.as_ref().expect("the mask comes back as a WTE mask");
    let p = owned_pixels(l);
    assert_eq!((m.pixels.width, m.pixels.height), (p.width, p.height));
    assert!(m.pixels.data.contains(&0) && m.pixels.data.contains(&255));
    // f6: a group -> merged.
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    let LinkApplied::Updated { mode, toast, .. } = apply(&mut doc, sid, "f6_group.psd", PC) else { panic!() };
    assert_eq!(mode, LinkMode::Merged);
    assert!(toast.as_deref().unwrap().contains("groups"), "{toast:?}");
    // f8: hidden, 50 % opacity, Multiply; f8b: Color Dodge -> merged.
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    apply(&mut doc, sid, "f8_hidden_opacity_multiply.psd", PC);
    let s = sticker(&doc, sid);
    assert!(!by_name(&s, "hidden").visible);
    assert!((by_name(&s, "half").opacity - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(by_name(&s, "multiply").blend, BlendMode::Multiply);
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    let LinkApplied::Updated { mode, .. } = apply(&mut doc, sid, "f8b_color_dodge.psd", PC) else { panic!() };
    assert_eq!(mode, LinkMode::Merged);
    // 16 bit, an opaque Background layer, a saved alpha channel, a 16-bit PSB: layer by layer.
    for rel in ["f7_16bit.psd", "f3_opaque.psd", "f9_alpha_channel.psd", "f13_check_16bit.psb"] {
        let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
        let LinkApplied::Updated { mode, .. } = apply(&mut doc, sid, rel, PC) else { panic!("{rel}") };
        assert_eq!(mode, LinkMode::Layers, "{rel}");
        assert!(!sticker(&doc, sid).layers.is_empty());
    }
}

// ----- Photoshop-saved files -----

#[test]
fn k5_link_photoshop_files_decide_the_path_per_editor() {
    let ps = |rel: &str| format!("photoshop/{rel}");
    // A smart object: merged with its sentence on a Photoshop link (what the user was told), its own layer on a
    // PhotoCraft link.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let LinkApplied::Updated { mode, toast, .. } = apply(&mut doc, sid, &ps("layers/smartobject-layer.psd"), PS) else { panic!() };
    assert_eq!(mode, LinkMode::Merged);
    assert!(toast.unwrap().starts_with("This sticker now uses smart objects, which only Photoshop can show."));
    let (mut doc, sid) = fresh(EDITOR_PHOTOCRAFT);
    let LinkApplied::Updated { mode, .. } = apply(&mut doc, sid, &ps("layers/smartobject-layer.psd"), PC) else { panic!() };
    assert_eq!(mode, LinkMode::Layers);
    // With MaxCompat off: the smart object still forces on Photoshop -> needs a merged picture.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    assert_eq!(apply(&mut doc, sid, &ps("layers-minimal/smartobject-layer.psd"), PS), LinkApplied::Problem(LinkState::NeedsMerged));
    // A type layer: a picture layer, in both files (MaxCompat on and off).
    for rel in ["layers/type-layer.psd", "layers-minimal/type-layer.psd"] {
        let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
        let LinkApplied::Updated { mode, .. } = apply(&mut doc, sid, &ps(rel), PS) else { panic!("{rel}") };
        assert_eq!(mode, LinkMode::Layers, "{rel}");
        assert!(doc.link(sid).unwrap().editor_native, "{rel}");
        assert!(sticker(&doc, sid).layers.iter().all(|l| matches!(l.content, StickerContent::Picture(_))));
    }
    // Effects, an adjustment layer, a group: merged.
    for (rel, words) in [
        ("effects/outside-stroke.psd", "layer effects"),
        ("layers/levels.psd", "adjustment layers"),
        ("layers/group.psd", "groups"),
    ] {
        let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
        let LinkApplied::Updated { mode, toast, .. } = apply(&mut doc, sid, &ps(rel), PS) else { panic!("{rel}") };
        assert_eq!(mode, LinkMode::Merged, "{rel}");
        assert!(toast.as_deref().unwrap().contains(words), "{rel}: {toast:?}");
        assert_eq!(names(&sticker(&doc, sid)), ["Photoshop picture"]);
    }
    // A shape layer saved with MaxCompat off: "needs a merged picture" with Photoshop's sentence, word for word.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let before = sticker(&doc, sid);
    assert_eq!(apply(&mut doc, sid, &ps("layers-minimal/shape-layer.psd"), PS), LinkApplied::Problem(LinkState::NeedsMerged));
    assert_eq!(sticker(&doc, sid), before, "nothing applied");
    assert_eq!(
        texts::needs_merged(EDITOR_PHOTOSHOP, "Photoshop"),
        "Photoshop saved this file without a merged picture. In Photoshop choose Edit > Preferences > File Handling, set \"Maximize PSD and PSB File Compatibility\" to Always, and save again."
    );
    // With MaxCompat on the shape layer is a picture layer.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let LinkApplied::Updated { mode, .. } = apply(&mut doc, sid, &ps("layers/shape-layer.psd"), PS) else { panic!() };
    assert_eq!(mode, LinkMode::Layers);
    // A mask, a disabled mask, 16 bit, a flat file.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    apply(&mut doc, sid, &ps("mask.psd"), PS);
    assert!(sticker(&doc, sid).layers.iter().any(|l| l.mask.is_some()));
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    apply(&mut doc, sid, &ps("mask-disabled.psd"), PS);
    assert!(sticker(&doc, sid).layers.iter().all(|l| l.mask.is_none()), "a disabled mask is no mask");
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let LinkApplied::Updated { mode, .. } = apply(&mut doc, sid, &ps("16bit5x5.psd"), PS) else { panic!() };
    assert_eq!(mode, LinkMode::Layers);
    assert_eq!(sticker(&doc, sid).layers.len(), 3);
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let LinkApplied::Updated { mode, toast, .. } = apply(&mut doc, sid, &ps("slices.psd"), PS) else { panic!() };
    assert_eq!((mode, toast), (LinkMode::Merged, None), "a flat file is one picture, without a reason toast");
    // A non-sRGB profile: applied, with the note.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let LinkApplied::Updated { profile_note, .. } = apply(&mut doc, sid, &ps("colorprofiles/north_america_newspaper.psd"), PS) else {
        panic!()
    };
    assert_eq!(
        profile_note.as_deref(),
        Some(
            "photoshop/colorprofiles/north_america_newspaper.psd uses the colour profile \"Adobe RGB (1998)\". WTE shows its colours as sRGB; they can look different. In Photoshop: Edit > Convert to Profile > sRGB."
        )
    );
    // A refused colour mode: its sentence on the badge, nothing applied.
    let (mut doc, sid) = fresh(EDITOR_PHOTOSHOP);
    let b = bytes(&ps("colormodes/4x4_8bit_lab.psd"));
    let err = crate::psd::read(&b, &PsdLimits::default()).unwrap_err();
    assert_eq!(err, PsdError::Unsupported(PsdRefusal::Lab));
    let state = doc.link_file_refused(sid, &err, FileStamp::of_bytes(&b, None), "Logo_k1.psd").unwrap();
    assert_eq!(
        state.badge("Photoshop", "Fresh", "Logo_k1.psd"),
        "Logo_k1.psd is in Lab Color mode. In Photoshop choose Image > Mode > RGB Color and save again."
    );
}
