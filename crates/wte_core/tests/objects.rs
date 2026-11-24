// Objects-tabs WP2 (docs/plans/OBJECTS_TABS_PLAN.md 3.5): hidden objects through the public API of `Document`, on the
// "Three Objects" sample (Floor, Cube, Ball; `samples::object_trio`). Hiding is view state (no history, no revision,
// not modified); rays pass through hidden objects; brushes, projected stamps and wraps neither paint nor see them; a
// bake job keeps the visibility it was made with.
//
// UV regions of the sample (one 1024 x 1024 texture): Floor u 0.5..0.99, Cube u 0..0.5 / v 0..0.5, Ball u 0..0.5 /
// v 0.5..1.

use std::sync::Arc;

use wte_core::glam::{Vec2, Vec3};
use wte_core::testutil::layer_dense;
use wte_core::*;

const S0: TextureSetId = TextureSetId(0);
const FLOOR: u32 = 0;
const CUBE: u32 = 1;
const BALL: u32 = 2;

fn trio() -> Document {
    let doc = Document::sample(SampleKind::Objects);
    let names: Vec<&str> = doc.mesh.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["Floor", "Cube", "Ball"]);
    doc
}

/// The hidden flags with only `objects` hidden.
fn hiding(doc: &Document, objects: &[u32]) -> Vec<bool> {
    (0..doc.mesh.object_count()).map(|o| objects.contains(&o)).collect()
}

fn hide(doc: &mut Document, objects: &[u32]) {
    let flags = hiding(doc, objects);
    doc.set_hidden_objects(&flags).unwrap();
}

/// A top-down ray at (x, z).
fn down(x: f32, z: f32) -> Ray {
    Ray::new(Vec3::new(x, 5.0, z), Vec3::NEG_Y)
}

fn object_at(doc: &Document, x: f32, z: f32) -> Option<u32> {
    doc.raycast(&down(x, z)).map(|h| doc.mesh.tri_object_of(h.tri))
}

/// Per pixel: 0 floor region, 1 cube region, 2 ball region (the regions of `object_trio`).
fn region(x: u32, y: u32, side: u32) -> u32 {
    if x >= side / 2 {
        FLOOR
    } else if y < side / 2 {
        CUBE
    } else {
        BALL
    }
}

/// The pixels of a layer split by region: (changed-from-`before` counts per region).
fn changed_per_region(before: &PixelBuf, after: &PixelBuf) -> [usize; 3] {
    let mut out = [0usize; 3];
    for y in 0..after.height {
        for x in 0..after.width {
            if before.get(x, y) != after.get(x, y) {
                out[region(x, y, after.width) as usize] += 1;
            }
        }
    }
    out
}

/// Ink (alpha > 0) per region of a layer.
fn ink_per_region(img: &PixelBuf) -> [usize; 3] {
    let mut out = [0usize; 3];
    for y in 0..img.height {
        for x in 0..img.width {
            if img.get(x, y)[3] > 0 {
                out[region(x, y, img.width) as usize] += 1;
            }
        }
    }
    out
}

/// The active paint layer of the sample ("Paint 1").
fn paint_layer(doc: &Document) -> LayerId {
    doc.set(S0).unwrap().active.expect("an active layer")
}

/// A stroke along the cube's footprint (x -0.4..0.4 at z = 0), from top-down hits.
fn stroke_across_the_cube(doc: &mut Document) {
    let hits: Vec<Hit> = (0..=16).map(|i| doc.raycast(&down(-0.4 + 0.05 * i as f32, 0.0)).expect("a hit")).collect();
    let brush = BrushParams { radius: 0.25, hardness: 0.8, ..BrushParams::default() };
    doc.stroke_begin_with(&brush, &BrushTip::legacy(false)).unwrap();
    for h in &hits {
        doc.stroke_to_3d(h, 1.0);
    }
    assert!(doc.stroke_end(), "the stroke painted something");
}

fn solid_png(side: u32) -> Arc<[u8]> {
    let img = PixelBuf::filled(side, side, [255, 255, 255, 255]);
    Arc::from(encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap())
}

/// A white image stamp on the given projector; returns its id and its (only) layer.
fn add_white_stamp(doc: &mut Document, p: Projector) -> (StampId, LayerId) {
    let blob = doc.add_image_blob("white.png", solid_png(32)).unwrap();
    let id = doc.add_stamp("White", StampSource::Image { blob }, Placement::Projected(p), S0).unwrap();
    let layers = doc.stamp_layers(id);
    assert_eq!(layers.len(), 1);
    (id, layers[0].1)
}

/// A projector from above over the cube's footprint: the box reaches the cube's top (y = 1) and the floor (y = 0).
fn over_the_cube() -> Projector {
    let p = Projector::from_surface(Vec3::new(0.0, 0.0, 0.0), Vec3::Y, Vec3::NEG_Z, Vec2::new(0.8, 0.8), 2.0);
    assert!(p.occlusion, "occlusion on by default");
    p
}

/// A wrapped picture on the floor beside the cube (its reach covers the cube), Jump small gaps as given.
fn beside_the_cube(jump: bool) -> Projector {
    let p = Projector::from_surface(Vec3::new(-0.9, 0.0, 0.0), Vec3::Y, Vec3::NEG_Z, Vec2::new(1.2, 0.6), 0.6);
    Projector { mode: SurfaceMode::Wrap, wrap: WrapOptions { jump_gaps: jump, overlap: false }, ..p }
}

fn cube_tris(doc: &Document) -> Vec<u32> {
    (0..doc.mesh.tri_count()).filter(|&t| doc.mesh.tri_object_of(t) == CUBE).collect()
}

#[test]
fn hiding_is_view_state() {
    let mut doc = trio();
    let layer = paint_layer(&doc);
    stroke_across_the_cube(&mut doc);
    let (revision, modified, label) = (doc.revision(), doc.is_modified(), doc.undo_label().map(str::to_owned));
    let (pixels, composite) = (layer_dense(&doc, layer), doc.set(S0).unwrap().composite().clone());
    assert_eq!(doc.visibility_generation(), 0);
    assert_eq!(doc.hidden_objects(), [false, false, false]);

    assert!(matches!(doc.set_hidden_objects(&[false, true, false]), Ok(true)));
    assert_eq!(doc.visibility_generation(), 1);
    assert!(doc.object_hidden(CUBE) && !doc.object_hidden(FLOOR) && !doc.object_hidden(99));
    assert_eq!(doc.hidden_object_count(), 1);
    assert_eq!((doc.revision(), doc.is_modified()), (revision, modified));
    assert_eq!(doc.undo_label().map(str::to_owned), label);
    assert!(layer_dense(&doc, layer) == pixels && *doc.set(S0).unwrap().composite() == composite, "no pixel changes");
    // The same flags again: nothing changes. A short slice is padded with `false`, a long one cut.
    assert!(matches!(doc.set_hidden_objects(&[false, true, false]), Ok(false)));
    assert!(matches!(doc.set_hidden_objects(&[false, true]), Ok(false)));
    assert!(matches!(doc.set_hidden_objects(&[false, true, false, true, true]), Ok(false)));
    assert_eq!(doc.visibility_generation(), 1);
    // Undo works on the stroke, not on the visibility.
    assert!(doc.undo().is_some());
    assert!(doc.object_hidden(CUBE));
    assert!(doc.redo().is_some());
    // Refused while a stroke is open.
    let hit = doc.raycast(&down(-2.0, 1.0)).unwrap();
    doc.stroke_begin_with(&BrushParams::default(), &BrushTip::legacy(false)).unwrap();
    doc.stroke_to_3d(&hit, 1.0);
    assert!(matches!(doc.set_hidden_objects(&[false; 3]), Err(EditError::StrokeInProgress)));
    doc.stroke_end();
    assert!(matches!(doc.set_hidden_objects(&[]), Ok(true)), "an empty slice shows everything");
    assert_eq!(doc.hidden_object_count(), 0);
    assert_eq!(doc.visibility_generation(), 2);
}

#[test]
fn visible_bounds_and_object_bounds() {
    let mut doc = trio();
    let all = doc.mesh.bounds;
    assert_eq!(doc.visible_bounds(), all);
    let cube = doc.mesh.objects[CUBE as usize].bounds;
    let ball = doc.mesh.objects[BALL as usize].bounds;
    assert_eq!(doc.objects_bounds(&[CUBE]), Some(cube));
    assert_eq!(doc.objects_bounds(&[CUBE, BALL, 77]), Some(cube.union(&ball)));
    assert_eq!(doc.objects_bounds(&[]), None);
    hide(&mut doc, &[FLOOR]);
    assert_eq!(doc.visible_bounds(), cube.union(&ball));
    hide(&mut doc, &[FLOOR, BALL]);
    assert_eq!(doc.visible_bounds(), cube);
    assert_eq!(doc.objects_bounds(&[BALL]), None, "hidden objects are not framed");
    assert_eq!(doc.objects_bounds(&[BALL, CUBE]), Some(cube));
    hide(&mut doc, &[FLOOR, CUBE, BALL]);
    assert_eq!(doc.visible_bounds(), all, "everything hidden: the whole model");
}

#[test]
fn rays_pass_through_a_hidden_object() {
    let mut doc = trio();
    assert_eq!(object_at(&doc, 0.1, 0.1), Some(CUBE));
    hide(&mut doc, &[CUBE]);
    assert_eq!(object_at(&doc, 0.1, 0.1), Some(FLOOR));
    let hit = doc.raycast(&down(0.1, 0.1)).unwrap();
    assert!(hit.position.y.abs() < 1e-5);
    hide(&mut doc, &[CUBE, FLOOR]);
    assert_eq!(object_at(&doc, 0.1, 0.1), None, "nothing behind the cube but the hidden floor");
    assert_eq!(object_at(&doc, 1.75, 0.0), Some(BALL));
    hide(&mut doc, &[]);
    assert_eq!(object_at(&doc, 0.1, 0.1), Some(CUBE));
}

#[test]
fn a_brush_stroke_paints_under_a_hidden_object_and_not_on_it() {
    // Cube hidden: the stroke lands on the floor under the cube; no cube texel changes.
    let mut doc = trio();
    let layer = paint_layer(&doc);
    let before = layer_dense(&doc, layer);
    hide(&mut doc, &[CUBE]);
    stroke_across_the_cube(&mut doc);
    let hidden = changed_per_region(&before, &layer_dense(&doc, layer));
    assert!(hidden[FLOOR as usize] > 500, "{hidden:?}");
    assert_eq!(hidden[CUBE as usize], 0, "{hidden:?}");
    assert_eq!(hidden[BALL as usize], 0, "{hidden:?}");
    assert_eq!(doc.undo_label(), Some("Brush Stroke"));

    // Everything visible: the same stroke paints the cube's top and leaves the floor (one unit below) clean.
    let mut doc = trio();
    let before = layer_dense(&doc, layer);
    stroke_across_the_cube(&mut doc);
    let shown = changed_per_region(&before, &layer_dense(&doc, layer));
    assert!(shown[CUBE as usize] > 500, "{shown:?}");
    assert_eq!(shown[FLOOR as usize], 0, "{shown:?}");
}

#[test]
fn a_projected_stamp_skips_a_hidden_object() {
    let mut doc = trio();
    let (_, layer) = add_white_stamp(&mut doc, over_the_cube());
    let shown = ink_per_region(&layer_dense(&doc, layer));
    assert!(shown[CUBE as usize] > 0 && shown[FLOOR as usize] > 0, "{shown:?}");

    let mut doc = trio();
    hide(&mut doc, &[CUBE]);
    let (_, layer) = add_white_stamp(&mut doc, over_the_cube());
    let hidden = ink_per_region(&layer_dense(&doc, layer));
    assert_eq!(hidden[CUBE as usize], 0, "the hidden cube receives nothing: {hidden:?}");
    // The cube no longer occludes the floor under it.
    assert!(hidden[FLOOR as usize] > shown[FLOOR as usize], "{hidden:?} vs {shown:?}");
    // The depth fit sees only visible surfaces: placed on the cube's top, it fits that top (a shallow box) while the
    // cube is shown, and reaches down to the floor one unit below once the cube is hidden.
    let on_top = Projector::from_surface(Vec3::new(0.0, 1.0, 0.0), Vec3::Y, Vec3::NEG_Z, Vec2::new(0.8, 0.8), 0.8);
    let (shown_fit, hidden_fit) = (trio().fit_stamp_depth(&on_top), doc.fit_stamp_depth(&on_top));
    assert!(shown_fit.depth_back < 0.9, "{}", shown_fit.depth_back);
    assert!(hidden_fit.depth_back > 1.0, "{}", hidden_fit.depth_back);
}

#[test]
fn a_wrap_never_enters_a_hidden_object() {
    for jump in [false, true] {
        let mut doc = trio();
        hide(&mut doc, &[CUBE]);
        let (id, layer) = add_white_stamp(&mut doc, beside_the_cube(jump));
        let ink = ink_per_region(&layer_dense(&doc, layer));
        assert!(ink[FLOOR as usize] > 0, "jump {jump}: {ink:?}");
        assert_eq!(ink[CUBE as usize], 0, "jump {jump}: {ink:?}");
        assert!(doc.ensure_wrap_param(id).is_some());
        for t in cube_tris(&doc) {
            assert_eq!(doc.stamp_wrap_local(id, t, [1.0 / 3.0; 3]), None, "jump {jump}: cube triangle {t}");
        }
    }
    // With the cube shown, Jump small gaps reaches it (so the test above means something).
    let mut doc = trio();
    let (_, layer) = add_white_stamp(&mut doc, beside_the_cube(true));
    let ink = ink_per_region(&layer_dense(&doc, layer));
    assert!(ink[CUBE as usize] > 0, "{ink:?}");
}

#[test]
fn a_bake_job_keeps_the_visibility_it_was_made_with() {
    // Reference: baked with everything visible.
    let mut reference = trio();
    let (_, ref_layer) = add_white_stamp(&mut reference, over_the_cube());
    let want = layer_dense(&reference, ref_layer);

    let mut doc = trio();
    doc.set_defer_final_bakes(true);
    let (id, layer) = add_white_stamp(&mut doc, over_the_cube());
    let job = doc.next_final_bake().expect("a deferred final bake");
    assert_eq!(job.stamp(), id);
    hide(&mut doc, &[CUBE]);
    assert!(matches!(doc.apply_final_bake(job.run()), FinalBake::Applied { .. }));
    assert!(layer_dense(&doc, layer) == want, "the job baked with the cube it was made with");
    // A bake made now sees the hidden cube: a different picture.
    doc.set_defer_final_bakes(false);
    let (_, other) = add_white_stamp(&mut doc, over_the_cube());
    assert!(layer_dense(&doc, other) != want);
    assert_eq!(ink_per_region(&layer_dense(&doc, other))[CUBE as usize], 0);
}

/// Determinism with a mask: a stroke and a wrapped bake on the trio with the cube hidden give the same pixels on one
/// thread and on several.
#[test]
fn masked_results_do_not_depend_on_the_thread_count() {
    let run = || {
        let mut doc = trio();
        hide(&mut doc, &[CUBE]);
        let layer = paint_layer(&doc);
        stroke_across_the_cube(&mut doc);
        let (_, wrapped) = add_white_stamp(&mut doc, beside_the_cube(true));
        (layer_dense(&doc, layer), layer_dense(&doc, wrapped))
    };
    let pools = [1usize, 4].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
    let one = pools[0].install(run);
    let four = pools[1].install(run);
    assert!(one.0 == four.0, "brush");
    assert!(one.1 == four.1, "wrap");
}
