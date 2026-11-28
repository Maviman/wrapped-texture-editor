# Check note: S1 - GrabCut box cut-out (STICKER_PLAN 7 "S1", 4.5 "GrabCut box", 8.4)

Independent checker, 2026-11-27, branch `stickers` at 4e3fbea (builder commits 2bb6166 + 4e3fbea). Verdict:
**pass after fixes**. No blocker or major found. The fixes are two test gaps closed with new tests (tests only: the
product code is unchanged).

## Plan items, one by one

| Plan item (7 "S1", 4.5, 8.2, 8.4) | Found | Verdict |
|---|---|---|
| `sticker/grabcut.rs` | `wte_core/src/sticker/grabcut.rs` (`cut_out_box`, `BoxCutRefusal`), MIT notice of plan 4.0 on line 1 | ok |
| 5-component full-covariance GMMs, deterministic k-means start | `Gmm::fit` (k-means++ with a seed taken from the sample count, 10 Lloyd steps), `refit` = GrabCut's hard assignment, ridge 2e-5 | ok |
| 8-neighbour graph, contrast-sensitive weights, `gamma = 50` | `grid_cut`, `NB`, `contrast_beta` (`beta = 1 / (2 <dz^2>)`), weight `GAMMA exp(-beta dz^2) / dist` | ok |
| Boykov-Kolmogorov max-flow | `Graph` (two trees, augment, adopt with the distance / timestamp heuristic) | ok, read against the BK paper / reference code line by line, and cross-checked (below) |
| 5 iterations | `ITERATIONS = 5`, early stop when no label changed (same result) | ok |
| working copy of at most 512 px | box + margin, box-filtered by a whole step (deviation 1 of the build note: a window, not the whole picture; pixels farther out are background anyway) | ok, deviation accepted |
| one re-cut at full size in a band of 6 px | `BAND = 6.0`, distance planes to the kept / cleared sides, pixels outside the band fixed | ok |
| soft edge as step 4 | `cutout::soft_edge` (Remove Background's step 4 moved; diff read: identical code, only the key chooser is a closure) | ok |
| tool "Cut Out Box", drag a rectangle | `StickerTool::CutOutBox` (Shift+W), `CanvasDrag::CutBox`, `cut_box.rs` | ok |
| option "Into mask" | Tool tab checkbox, `filters::apply_cut` (shared with Remove Background) | ok |
| on a job with a progress note (8.4) | thread + `AtomicU32` progress, header note + Cancel, Tool tab progress bar, Esc | ok |
| tests: three textured photo-like pictures IoU >= 0.9 | `k_s1_ball_on_grass...` 0.999, `k_s1_mug_on_wood...` 0.997, `k_s1_star_on_carpet...` 0.997 (the last through the re-cut) | ok |
| determinism | `k_s1_the_same_for_1_and_8_threads_and_for_two_runs` | ok |
| 2048 x 2048 within budget (3 s) | `perf_s1_grabcut_box_2048` | ok (re-measured: 518 ms / 746 ms whole picture) |
| scenario `sticker_grabcut` | `selftest/scenarios/sticker_grabcut.rs`, `NAMES` (73) | ok, ran it |
| one undo per action, undo to the start, no Error toast (8.2) | asserted in the scenario; "Cut Out Box" one entry (app tests) | ok |

## What I re-ran (this machine, release where it matters)

- `cargo test --workspace --no-fail-fast` (builder's tree 4e3fbea): **1,944 passed, 1 failed (WA20, the known red),
  69 ignored** (`target\sticker\s1check\test_full.log`) - equal to the builder's numbers. After my additions:
  the new core test passes (`check_s1_*` 1/1); the S1 tests unchanged.
- `cargo clippy --workspace --all-targets`: **0 warnings** (after fixing two in my own new test helper).
- `cargo build --release -p wte_app`: OK; exe copy `target\sticker\s1check\wte_s1c.exe` (SHA-256 2782f77ba8c9f11d...;
  my test-only edits do not reach the exe).
- Release perf (serial): `perf_s1_grabcut_box_2048` **518 ms**, whole picture **746 ms**;
  `perf_check_s1_grabcut_fragmented_worst_case_2048` (new) **534 ms** - all within 3 s.
- Scenarios (`target\sticker\s1check\runs\`, `--screenshot` / `--scenario`, 1280x800, off-screen, no focus taken):
  `sticker_grabcut`, `sticker_cutout`, `sticker_keys`, `sticker_view`, `smoke`: all `"ok": true`.
- Shots looked at: `grabcut_cut` (the ball alone on the checkerboard, clean round edge, the sphere shows the ball
  without grass, status bar "Undo: Cut Out Box", Tool tab "Into mask (keep pixels)" unticked); `grabcut_mask` (same
  picture, Into mask ticked, the Mask block offers Invert / Apply / Delete, "Add mask" greyed = the mask exists);
  `grabcut_die_cut` (the ball with a white die-cut border on the sphere, "Undo: Edit Sticker Style"). Each matches its
  expect text. No "Cutting out..." note is left in the status bar after the cut lands.
- Identity sweep (plan 8.5, `tools/sticker/jobs_identity.txt` -> `target\sticker\s1check\sweep\`): 21 / 21 `"ok": true`;
  against the builder's S1 sweep 216 / 217 equal, the one difference is `export_checks/shot.png` in the box of the
  fading "Exported ..." toast, and that file is pixel-identical to the builder's re-run (`sweep_rerun`): 217 / 217.
  Old projects and exports are unchanged.

## Hunting bugs - what I checked and found

1. **Max-flow correctness (test gap, fixed).** The only proof of the BK solver was a brute-force check on 4 x 3 grids
   (12 nodes): BK's tree reuse / orphan adoption bugs typically show up only on larger graphs. New test
   `check_s1_the_max_flow_equals_an_independent_max_flow_on_larger_graphs` (`grabcut/tests.rs`): 60 random
   8-neighbour grids up to 30 x 30, asymmetric arc capacities (10 % near zero, around the solver's `EPS`), terminal
   capacities of both signs and zero, plus random long-range arcs; the cost of the cut BK labels must equal the
   max-flow value of an independent `f64` Dinic. It passes. Mutation check: with `in_source` ignoring the tree side,
   it fails at case 0 (11,176 vs 8,533), so the test has teeth.
2. **Worst-case re-cut band (perf gap, fixed with a test).** The band re-cut builds a graph of every band pixel; a
   fragmented working result could make the band the whole window (memory / time trap). New ignored release test
   `perf_check_s1_grabcut_fragmented_worst_case_2048` (`sticker/bench.rs`): red / green cells of the working step
   inside the box on a green picture (2.7 M pixels kept as salt and pepper): 534 ms. No trap at the plan's size.
3. **Negative data costs.** `neg_log` can be negative (densities above 1); `add_tweights` keeps only the difference,
   which is all the cut depends on; the brute-force and Dinic tests use negative and mixed costs. ok.
4. **Threading.** At most one cut thread (`AliveGuard` counts it, also on a panic; a spawn failure drops the guard
   with the closure); a cancelled thread's late result goes to a dropped receiver; the result is applied only when the
   layer still holds the same pixels and mask (`Arc::ptr_eq` or equal), on the job's own layer; leaving the view or
   another sticker stops it; a result waits for a drag or stroke. The progress callback runs only on the cut's thread
   (not inside rayon). ok.
5. **Undo / history.** One entry "Cut Out Box" (image stamp promotion inside it); `S::CutOutBox` is in the content-edit
   list so the shared-picture question comes first, like Remove Background. ok.
6. **Rules.** No new crate, `Cargo.lock` unchanged; no forbidden name in the diff; MIT notice line present; sources LF;
   no clipboard / registry / OS-input use; the scenario and tests use no external program.

## Minor points not fixed (for the record)

- **Soft-edge key next to transparent pixels.** `soft_edge` keys a band pixel on the colour of the closest region
  pixel; for the GrabCut box that pixel can be a transparent one (its RGB is meaningless, often black). The
  normalisation by the nearest inside pixel's opacity keeps the effect small (the transparent-hole test keeps IoU
  >= 0.88), and only a 2 px band next to transparency is touched. Remove Background had and keeps the same behaviour.
- **Partly transparent pixels.** Pixels with alpha below 128 count as fixed background, so re-cutting a picture whose
  edge is already anti-aliased by alpha clears those edge pixels. Photos are opaque; documented as a known limit here.
- **Cancel granularity.** Cancel is seen between steps, not inside a max-flow (about 0.15 s at 512 px; the full-size
  re-cut of a 4,096 px picture can take longer). A new box waits for the old thread, so nothing piles up.
- The builder's open issues stand: the progress note is not caught in a shot (covered by held-thread unit tests);
  the Cut Out Box has no number in the hand-test list 1-25 (HOW_TO_TEST part 3 step 13 only); S2 not started.

## Commits of this check

Tests and docs only (no product code changed): `grabcut/tests.rs` (the Dinic cross-check), `sticker/bench.rs` (the
fragmented worst-case perf test), ARCHITECTURE 9.1 (the two tests), this note.
