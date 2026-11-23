// Owner: stable (w1). Regression tests of the wrap correctness fixes after round 9 (docs/notes/stable/w1.md): the
// ends-meet detection on open surfaces, the board's orientation under a part with Jump small gaps. A child module of
// `wrap_tests` (it uses that module's helpers).

use super::*;
use crate::ids::TriId;

/// A square height field of `n x n` quads of side 1 centred on the origin in the XY plane (facing +Z), each vertex
/// lifted by a deterministic noise in `[-amp, amp]` (`seed`). One UV island over the whole texture.
pub(super) fn rough_field(n: u32, amp: f32, seed: u64) -> RawMesh {
    let noise = |i: u32, j: u32| -> f32 {
        let mut h = seed ^ ((i as u64) << 32 | j as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        amp * (2.0 * (h >> 11) as f32 / (1u64 << 53) as f32 - 1.0)
    };
    let half = n as f32 * 0.5;
    let at = |i: u32, j: u32| Vec3::new(i as f32 - half, j as f32 - half, noise(i, j));
    let uv = |i: u32, j: u32| Vec2::new(0.01 + 0.98 * i as f32 / n as f32, 0.99 - 0.98 * j as f32 / n as f32);
    let mut t = Vec::new();
    for j in 0..n {
        for i in 0..n {
            t.push(raw_tri([at(i, j), at(i + 1, j), at(i + 1, j + 1)], [uv(i, j), uv(i + 1, j), uv(i + 1, j + 1)]));
            t.push(raw_tri([at(i, j), at(i + 1, j + 1), at(i, j + 1)], [uv(i, j), uv(i + 1, j + 1), uv(i, j + 1)]));
        }
    }
    textured("Rough", t)
}

/// Stable (w1), item 1: on an open rough height field (no tube, no hole) the spine's traces wander and cross each
/// other's paths, but the ends never meet: `passed` and `met_itself` stay false and the picture is not cut
/// (`ends_share` 1). Round 9: `passed` fired (the noisy 400 field: 36 % shown, 173 S1 seams; here seeds 1 and 9 of the
/// 200 field: ends 0.598 / 0.593, 59 / 9 S1 seams; 5 of 12 seeds fired).
#[test]
fn w1_an_open_rough_field_never_closes() {
    for seed in [1u64, 9] {
        let (n, amp, half) = (200u32, 0.6f32, Vec2::new(56.0, 28.0));
        let raw = rough_field(n, amp, seed);
        let p = wrapped(Vec3::new(0.3, 0.2, 0.0), Vec3::Z, Vec3::Y, half);
        let model = Model::new(raw, 256);
        let param = model.param(&cells(), &p).expect("a parameterization");
        let s = param.stats();
        println!("W1 rough field {n} amp {amp} seed {seed}: {s:?}");
        assert!(!s.passed && !s.met_itself, "seed {seed}: the ends met on an open field: {s:?}");
        assert_eq!(s.ends_share, 1.0, "seed {seed}: the picture was cut: {s:?}");
    }
}

/// Copies of `samples::lathe_piece` (UVs squeezed into the left half, one strip per copy) at `at` standing on a floor:
/// an `n x n` grid over x, z in -3..3 at `y = -gap` facing +Y (UVs in the right half), every copy and the floor a
/// separate part. Returns the mesh and the pieces' triangle count (they come first).
pub(super) fn pieces_on_floor(at: &[Vec3], n: u32, gap: f32) -> (RawMesh, u32) {
    let one = samples::lathe_piece();
    let mut raw = RawMesh { triangles: Vec::new(), materials: one.materials.clone() };
    let k = at.len() as f32;
    for (i, &o) in at.iter().enumerate() {
        for t in &one.triangles {
            let mut t = t.clone();
            for p in t.positions.iter_mut() {
                *p += o;
            }
            if let Some(uvs) = t.uvs.as_mut() {
                for uv in uvs.iter_mut() {
                    uv.x = (i as f32 + uv.x) * 0.49 / k;
                }
            }
            raw.triangles.push(t);
        }
    }
    let piece = raw.triangles.len() as u32;
    let at = |i: u32, j: u32| Vec3::new(-3.0 + 6.0 * i as f32 / n as f32, -gap, 3.0 - 6.0 * j as f32 / n as f32);
    let mut floor = Vec::new();
    a9_grid(&mut floor, n, n, [0.51, 0.99, 0.01, 0.99], &at);
    raw.triangles.extend(floor);
    (raw, piece)
}

/// The direction (degrees, -180..180) of the picture's rows (its x axis) on the floor triangles `floor` of the top
/// layer whose layout lies inside the picture, against the projector's right projected onto the floor (y = const), and
/// how many triangles were measured. Per triangle the gradient of the layout's x over the triangle's (x, z), summed as
/// unit vectors.
pub(super) fn rows_on_floor(param: &WrapParam, mesh: &MeshData, floor: &dyn Fn(TriId) -> bool, p: &Projector) -> (f64, usize) {
    let layer = param.layers.last().unwrap();
    let (right, _, _) = p.axes();
    let r = glam::DVec2::new(right.x as f64, right.z as f64).normalize();
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    let mut sum = glam::DVec2::ZERO;
    let mut n = 0;
    for (k, &t) in layer.tris.iter().enumerate() {
        if !floor(t) {
            continue;
        }
        let q = layer.corners[k];
        if !q.iter().all(|c| c[0].abs() <= hw && c[1].abs() <= hh) {
            continue;
        }
        let pos = mesh.tri_positions(t);
        let (a, b) = (pos[1] - pos[0], pos[2] - pos[0]);
        let (a, b) = (glam::DVec2::new(a.x as f64, a.z as f64), glam::DVec2::new(b.x as f64, b.z as f64));
        let (du, dv) = ((q[1][0] - q[0][0]) as f64, (q[2][0] - q[0][0]) as f64);
        // grad . a = du, grad . b = dv.
        let det = a.x * b.y - a.y * b.x;
        if det.abs() < 1e-12 {
            continue;
        }
        let g = glam::DVec2::new((du * b.y - dv * a.y) / det, (dv * a.x - du * b.x) / det);
        if g.length() > 0.0 {
            sum += g.normalize();
            n += 1;
        }
    }
    let angle = (r.x * sum.y - r.y * sum.x).atan2(r.dot(sum)).to_degrees();
    (angle, n)
}

/// Stable (w1), item 1: a part standing on a plane, the picture's anchor on the part's base, Jump small gaps on (the
/// chess set's pawn on the board): the picture's rows on the plane run parallel to the frame's top edge, and the plane
/// receives the picture the part does not take. Round 9: the exact region's fallback dropped the plane's exact pins
/// because the part's failed, and the free solve turned the plane's picture (the chess board: about 90 degrees).
#[test]
fn w1_a_part_on_a_plane_keeps_the_rows_on_the_plane() {
    let n = Vec3::new(0.0, 0.7, 0.714).normalize();
    let up = Vec3::new(0.0, 0.714, -0.7).normalize();
    let cases: Vec<(&str, Vec<Vec3>, Vec3, Vec2)> = vec![
        ("one piece, base rim", vec![Vec3::ZERO], Vec3::new(0.0, 0.04, 0.5), Vec2::new(1.5, 0.75)),
        ("crowded", vec![Vec3::ZERO, Vec3::new(0.7, 0.0, 0.5), Vec3::new(-0.8, 0.0, 0.3), Vec3::new(1.3, 0.0, -0.6), Vec3::new(-1.4, 0.0, -0.6)], Vec3::new(0.0, 0.04, 0.5), Vec2::new(2.0, 1.0)),
        ("two pieces, small", vec![Vec3::ZERO, Vec3::new(1.2, 0.0, 0.45)], Vec3::new(0.0, 0.04, 0.5), Vec2::new(0.9, 0.45)),
    ];
    for (what, at, anchor, half) in cases {
        let (raw, piece) = pieces_on_floor(&at, 48, 0.005);
        let total = raw.triangles.len() as u32;
        let p = with_options(wrapped(anchor, n, up, half), true, false);
        let model = Model::new(raw.clone(), SIDE);
        let param = model.param(&cells(), &p).expect("a parameterization");
        let s = param.stats();
        let outcome = crate::wrap_param::relax_exact_for_tests().1.with(|o| o.get());
        println!("W1 {what}: exact region (wedges, fell back) {outcome:?}");
        let (angle, count) = rows_on_floor(&param, &model.mesh, &|t| t >= piece && t < total, &p);
        let share = layout_share(&param, &|t| t >= piece && t < total, &p);
        let (q, s2, ink, doc, id) = r8_measure(raw.clone(), SIDE, p);
        let (on_piece, on_floor) = ink_in(&doc, id, 0..piece);
        println!("W1 {what}: rows on the floor {angle:.2} deg over {count} triangles, floor share {share:.4}, piece texels {on_piece}, floor texels {on_floor}, {}, {s:?}", r8_line(what, &q, &s2, ink));
        assert!(count > 100, "{what}: the floor was not laid ({count} triangles)");
        assert!(angle.abs() <= 5.0, "{what}: the picture is turned by {angle:.2} degrees on the floor");
        assert!(share >= 0.7, "{what}: the floor receives only {share:.4} of the picture");
    }
}

/// A sample model file: `WTE_GLB_SAMPLES/<name>`, else `samples/<name>` in an ancestor of this crate or of a sibling
/// `texture_wrapper_ap` tree (this machine's layout); `None` when absent (the test is then skipped).
pub(super) fn sample_model(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("WTE_GLB_SAMPLES") {
        let p = std::path::Path::new(&dir).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    here.ancestors().flat_map(|a| [a.join("samples").join(name), a.join("texture_wrapper_ap").join("samples").join(name)]).find(|p| p.exists())
}

/// The picture's share (layout area of the triangles `tris` lying wholly inside the picture, over the picture's area).
pub(super) fn layout_share(param: &WrapParam, tris: &dyn Fn(TriId) -> bool, p: &Projector) -> f64 {
    let layer = param.layers.last().unwrap();
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    let mut area = 0.0f64;
    for (k, &t) in layer.tris.iter().enumerate() {
        let q = layer.corners[k];
        if !tris(t) || !q.iter().all(|c| c[0].abs() <= hw && c[1].abs() <= hh) {
            continue;
        }
        area += 0.5 * ((q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[1][1] - q[0][1]) * (q[2][0] - q[0][0])).abs() as f64;
    }
    area / (4.0 * hw as f64 * hh as f64)
}

/// Stable (w1), item 1 on the chess set (`ABeautifulGame.glb`, skipped without it): `wrap_surface` step 10's
/// placement (the anchor on a white pawn's base, Jump small gaps on). The picture's rows on the board run parallel
/// to the frame's top edge (round 9: turned about 60-90 degrees), the board receives the picture round the pawns,
/// and the spine's ends do not "pass" (round 9: passed, 59 % shown).
#[test]
fn w1_the_chess_board_keeps_the_rows_under_a_pawn() {
    use crate::doc_build::TextureChoice;
    let Some(path) = sample_model("ABeautifulGame.glb") else {
        println!("W1 chess board: ABeautifulGame.glb not found, skipped");
        return;
    };
    let model = crate::import::import_from_path(&path, &Default::default(), &crate::progress::NoProgress).unwrap();
    let n = model.mesh.sets.len();
    let doc = Document::from_import(model, vec![TextureChoice::Detected; n], &crate::progress::NoProgress).unwrap();
    let bits = |b: &[u32]| b.iter().map(|&v| f32::from_bits(v)).collect::<Vec<_>>();
    let (o, r, h) = (bits(&[3183435816, 1019565072, 3188448736]), bits(&[3201021214, 1029003424, 1037318073, 1063844668]), bits(&[1045379227, 1036990619]));
    let mut p = wrapped(Vec3::new(o[0], o[1], o[2]), Vec3::Y, Vec3::X, Vec2::new(h[0], h[1]));
    p.rotation = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
    p.wrap = WrapOptions { jump_gaps: true, overlap: false };
    let src = cells();
    let maps = doc.texel_maps();
    let param = crate::wrap_param::wrap_param(&doc.mesh, &doc.bvh, &maps, src.size(), &p).expect("a parameterization");
    let s = param.stats();
    // The board's top: the triangles facing +Y at the board's height.
    let mesh = &doc.mesh;
    let board = |t: TriId| {
        let q = mesh.tri_positions(t);
        q.iter().all(|v| (v.y - 0.016947).abs() < 2e-4) && (q[1] - q[0]).cross(q[2] - q[0]).normalize_or_zero().y > 0.999
    };
    let (angle, count) = rows_on_floor(&param, mesh, &board, &p);
    let share = layout_share(&param, &board, &p);
    println!("W1 chess board: rows on the board {angle:.2} deg over {count} triangles board share of the picture {share:.4}, {s:?}");
    assert!(count > 300, "the board was not laid ({count} triangles)");
    assert!(angle.abs() <= 5.0, "the picture is turned by {angle:.2} degrees on the board");
    assert!(share >= 0.7, "the board receives only {share:.4} of the picture");
    assert!(!s.passed, "{s:?}");
}

/// Stable (w1), item 3: a parameterization reused for another size in its reach bucket bakes byte for byte like a
/// fresh one of that size, and reports the same statistics (every field but the build time). Cases where the exact
/// region decides (WA35's capped cylinder with the picture over the dome's shoulder, the plate with a 45 degree bump),
/// a closed helix (WA32's torus: `passed`, `ends_share`), the cube on the floor with Jump on (the owner grid's
/// `rejected`, `lost_cut`). Round 9 scaled by 1.0001 only; the exact pins' check used the unbucketed picture and the
/// stats of a reused parameterization were those of the size it was built for (`covered` aside).
#[test]
fn w1_reuse_equals_fresh_within_the_bucket() {
    let torus = wa32_cases().pop().expect("the torus case");
    let cases: Vec<(&str, RawMesh, Projector)> = vec![
        ("WA35 capped cylinder", samples::capped_cylinder(), wrapped(cylinder_anchor(Some(20), 0.9).0, cylinder_anchor(Some(20), 0.9).1, Vec3::Y, Vec2::new(0.3, 0.6))),
        ("WA35 bump plate", a9_bump_plate(Vec2::new(0.1, 0.12), 0.04, 45.0), wrapped(Vec3::ZERO, Vec3::Z, Vec3::Y, Vec2::new(0.4, 0.2))),
        ("WA32 torus", torus.1, torus.2),
        ("WA24 cube on the floor", samples::uv_cube_on_floor(), with_options(wrapped(Vec3::new(0.0, 0.0, 1.2), Vec3::Y, Vec3::NEG_X, Vec2::new(1.0, 0.25)), true, false)),
    ];
    let no_time = |s: WrapStatsAlias| WrapStatsAlias { micros: 0, ..s };
    for (what, raw, p) in cases {
        let mut reused_sizes = 0;
        for f in [0.84f32, 0.9, 0.95, 0.98, 0.995, 1.005, 1.02, 1.05, 1.1, 1.18] {
            let (mut doc, id) = doc_with_stamp(raw.clone(), 512, p);
            let (first, _) = doc.wrap_cache_probe(id);
            let first = first.expect("a cached parameterization");
            let scaled = Projector { half_size: p.half_size * f, ..p };
            doc.set_stamp_placement(id, Placement::Projected(scaled), Commit::Final).unwrap();
            let (now, _) = doc.wrap_cache_probe(id);
            if !now.is_some_and(|w| Arc::ptr_eq(&w, &first)) {
                continue;
            }
            reused_sizes += 1;
            let reused_stats = doc.stamp_wrap_stats(id).expect("stats");
            let reused_bake = doc_state(&mut doc);
            let (mut fresh_doc, fresh_id) = doc_with_stamp(raw.clone(), 512, scaled);
            let fresh_stats = fresh_doc.stamp_wrap_stats(fresh_id).expect("stats");
            println!("W1 reuse {what} x {f}: reused {reused_stats:?}");
            assert_eq!(doc_state(&mut fresh_doc).sets, reused_bake.sets, "{what} x {f}: the reused and the fresh parameterization bake differently");
            assert_eq!(no_time(reused_stats), no_time(fresh_stats), "{what} x {f}: the reused parameterization reports other stats");
        }
        println!("W1 reuse {what}: {reused_sizes} sizes reused");
        assert!(reused_sizes >= 2, "{what}: only {reused_sizes} sizes stay in the bucket");
    }
}

/// `n x n` separate square tiles of side `side` one unit apart in the XY plane (facing +Z), each its own part and UV
/// cell (the code skeptic's tiles300, smaller).
pub(super) fn tiles(n: u32, side: f32) -> RawMesh {
    let mut t = Vec::new();
    let half = n as f32 * 0.5;
    for j in 0..n {
        for i in 0..n {
            let o = Vec3::new(i as f32 - half, j as f32 - half, 0.0);
            let p = [o, o + Vec3::new(side, 0.0, 0.0), o + Vec3::new(side, side, 0.0), o + Vec3::new(0.0, side, 0.0)];
            let isl = a9_island(j * n + i, n, n);
            raw_quad(&mut t, [p[3], p[2], p[1], p[0]].map(|q| q), [isl[0], isl[1], isl[2], isl[3]]);
        }
    }
    textured("Tiles", t)
}

/// Stable (w1), item 4: `lost_cut` counts picture lost to cuts, not the gaps between jumped parts. A tiled floor of
/// 40 x 40 separate tiles (gaps of 0.1, Jump small gaps on, no cut): lost_cut stays near 0 (round 9: the gap share,
/// about 1 - 0.9^2 = 19 %; tiles300 16.85 %). One part with a cut keeps counting (WA37 and the sweep's Lantern).
#[test]
fn w1_lost_cut_does_not_count_the_gaps_between_parts() {
    let p = with_options(wrapped(Vec3::new(0.45, 0.45, 0.0), Vec3::Z, Vec3::Y, Vec2::new(8.0, 4.0)), true, false);
    let model = Model::new(tiles(40, 0.9), 256);
    let s = model.param(&cells(), &p).expect("a parameterization").stats();
    println!("W1 tiles 40 x 40, Jump on: {s:?}");
    assert!(s.jumps > 100 && s.seams == 0, "{s:?}");
    assert!(s.lost_cut <= 0.01, "lost_cut {:.4} counts the gaps between the tiles: {s:?}", s.lost_cut);
}

// ----- stable (w2) -----

/// Stable (w2, item 4): `test-assets/shapes/nonmanifold.obj`'s book (seven pages on one spine edge, one part welded
/// at the spine) under the `wrap_surface` step-10 placement (Jump off): round 9 never asked which page is hidden, so
/// the picture went to pages behind the one the stamp faces (owner hidden along `out` at 2,253 of 5,186 picture points,
/// 1,551 of them with a visible place dropped). A cell with candidates on two sheets (separated by the non-manifold
/// edge) at two places now goes to a place not hidden along the projector's `out` when one is laid there.
#[test]
fn w2_a_book_shows_the_picture_on_the_page_it_faces() {
    use crate::doc_build::TextureChoice;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-assets/shapes/nonmanifold.obj");
    let model = crate::import::import_from_path(&path, &Default::default(), &crate::progress::NoProgress).expect("nonmanifold.obj");
    let n = model.mesh.sets.len();
    let doc = Document::from_import(model, vec![TextureChoice::Detected; n], &crate::progress::NoProgress).unwrap();
    let mut p = wrapped(Vec3::ZERO, Vec3::Y, Vec3::X, Vec2::ONE);
    p.origin = Vec3::from_array([1057008808u32, 1061764292, 3206634056].map(f32::from_bits));
    p.rotation = Quat::from_array([3166529817u32, 1054725428, 3175393369, 1063670273].map(f32::from_bits));
    p.half_size = Vec2::from_array([1075237229u32, 1066848621].map(f32::from_bits));
    let (points, hidden, hidden_over_visible) = super::owner_probe(&doc, &p);
    let mut doc = doc;
    let blob = doc.add_image_blob("cells.png", png_of(&samples::cells_picture(PIC[0], PIC[1]))).unwrap();
    let id = doc.add_stamp("Cells", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
    let st = doc.ensure_wrap_param(id).expect("a parameterization");
    let q = doc.stamp_wrap_quality(id).expect("a quality");
    println!("W2 book: {q:?}, rejected {}", st.rejected);
    println!("W2 book: {points} picture points on the surface, owner hidden at {hidden}, hidden while a visible place is dropped at {hidden_over_visible}");
    assert!(points > 4000, "{points}");
    assert!(hidden_over_visible == 0 && (hidden as f64) < 0.05 * points as f64, "{hidden} / {hidden_over_visible} of {points}");
}

/// Stable (w2, item 2): `room` and `wraps_closed`. A closed cube of side 1 under a picture 3 high covers the whole
/// cube (the picture loses its corners' 720 degrees to cuts: `wraps_closed`) and the cube holds a third of the
/// picture's height (`room` 1 / 3); a picture on a large plate has room 1 and covers no closed part.
#[test]
fn w2_room_and_a_whole_closed_part() {
    let src = cells();
    let model = Model::new(samples::seam_cube(1.0), SIDE);
    let p = wrapped(Vec3::new(0.0, 0.0, 0.5), Vec3::Z, Vec3::Y, Vec2::new(3.0, 1.5));
    let s = model.param(&src, &p).expect("a parameterization").stats();
    println!("W2 cube under a large picture: {s:?}");
    assert!(s.wraps_closed, "{s:?}");
    assert!((s.room - 1.0 / 3.0).abs() < 0.02, "room {}", s.room);
    let model = Model::new(samples::bumpy_plate(16, 0.0), SIDE);
    let p = wrapped(Vec3::ZERO, Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1));
    let s = model.param(&src, &p).expect("a parameterization").stats();
    println!("W2 plate: {s:?}");
    assert!(!s.wraps_closed && s.room == 1.0, "{s:?}");
}

/// Stable (w2, item 1): the developed exact region pins only flat surface: on WA4's bumpy plates (every vertex curved
/// by 1.5e-5 rad or more) nothing is added to round 9's exact region, and the capped cylinder's side rows next to
/// the dome are pinned at the exact unrolling (WA35).
#[test]
fn w2_the_developed_region_stays_off_curved_surface() {
    use crate::wrap_param::relax_exact_for_tests as exact;
    let src = cells();
    let model = Model::new(samples::bumpy_plate_periods(48, 0.12, 2.0), SIDE);
    let p = wrapped(Vec3::new(0.0, 0.0, 0.0), Vec3::Z, Vec3::Y, Vec2::new(0.4, 0.2));
    let with = model.param(&src, &p).expect("a parameterization");
    let (with_pins, _) = exact().1.with(|o| o.get());
    crate::wrap_param::relax_develop_for_tests().with(|e| e.set(false));
    let without = model.param(&src, &p);
    let (without_pins, _) = exact().1.with(|o| o.get());
    crate::wrap_param::relax_develop_for_tests().with(|e| e.set(true));
    let without = without.expect("a parameterization");
    println!("W2 bumpy plate: exact wedges {with_pins} with the developed region, {without_pins} without");
    assert_eq!(with_pins, without_pins);
    let bits = |l: Vec<testkit::Layer>| l.into_iter().map(|(t, c)| (t, c.iter().flatten().flatten().map(|v| v.to_bits()).collect::<Vec<_>>())).collect::<Vec<_>>();
    assert!(bits(testkit::layers(&with)) == bits(testkit::layers(&without)), "the developed region changed a curved plate");
}
