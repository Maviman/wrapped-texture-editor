# Stable: independent check of the wrap branch (pass 1)

Checker (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable-wrap` at ad8e268
(w1 + w2 on df93777). Read-only for sources. Raw outputs (git-ignored) under `target\stable\check_wrap_1\`: `build.log`,
`test.log`, `test_nff.log`, `clippy.log`, sweeps `sweep_sw\` / `sweep_r9\` with `table_sw.txt` / `table_r9.txt`,
identity runs `ident_sw\` / `ident_r9c\` with `ident_cmp.txt`, determinism runs `det\`, crops `crops\*.png`, scripts
`tools\` (`sweep.sh`, `run1.sh`, `sweep_parse.py`, `crop.py`, `ident.sh`, `ident_cmp.py`).

Exes: `bin\wte_sw.exe` = `target\release\wrapped_texture_editor.exe` built from ad8e268 (SHA-256 269D98FE...,
identical to w2's `target\w2\wte_w2.exe`); `bin\wte_r9c.exe` = copy of `target\round9\gate\bin\wte_r9.exe` (4F9E1422...).

## Verdict

The branch is a clear improvement over round 9 and I could not break the build, determinism or non-wrap identity.
I refute one claim: the panel lines are not yet accurate everywhere. On ABeautifulGame with Jump small gaps on, the
panel says "The ends meet at the back: 74 % of the picture's length shown", but all 8 columns and 4 rows of the picture
are on the board in the shot (covered 1.0000). One wrap_surface model (nonmanifold) is red, the documented side effect
of w2 item 4, and WA20 is still red.

## Build, tests, clippy

- Release build: ok (already up to date; the exe hash equals w2's final exe).
- `cargo test --workspace --no-fail-fast`: **1,186 passed, 1 failed, 36 ignored** (wte_app 364/0/16, wte_core
  775/1/20, brush_tips 15, end_to_end 9, project_roundtrip 23). Failing:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`: `cells 0.690..1.986` against the
  0.70-1.40 limit (ink 1.0871, dup 0.899 %, scatter 0.482 %, seams 0, flipped 29). This matches w2's report exactly.
  Note: without `--no-fail-fast` cargo stops after wte_core, so the three integration test binaries do not run.
- `w1_the_chess_board_keeps_the_rows_under_a_pawn` really runs on this machine (not skipped): rows -2.11 deg over 872
  board triangles, board share 0.7995.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- Line endings: every file changed since round9-wip is `i/lf`.

## Diff review: tests, asserts, races, panics

`git diff round9-wip --ignore-cr-at-eol -- '*test*' crates/wte_app/src/selftest` plus a grep for removed asserts and
limits in `crates/`:
- No assert or limit was deleted. WA4, WA20 and WA35 limits are unchanged.
- WA17 / WA24: the test setup moved (picture centre z 1.2 -> 0.95), limits unchanged. The geometric reason is given
  with numbers: from 1.2, the last 0.2 of the 2.0-long picture lay beyond the floor's edge, so at most 0.90 could lie
  once. This is allowed by the rules.
- Step-10 scenario assert loosened in two ways, each with a geometric reason:
  1. `covered >= 0.85 x ends_share` becomes `... x room`. `room` is the largest 3D extent of the entered parts
     along the projector's across axis, over the picture's across size (`wrap_param.rs:2601`, `:3622`). The reason
     holds (a pawn is narrower than the picture). The notes' claim that "the exemption errs on the side of the
     assert" is backwards, though: the 3D extent underestimates the surface's room on a curved part, which lowers
     the threshold and makes the assert more lenient. The exemption is also measured by the code under test. Margin
     on the sweep: ABeautifulGame off 0.0731 vs 0.0683; many_islands on 0.0338 vs 0.0323. **Minor.**
  2. `lost_cut <= 10 %` gets no limit at all when `wraps_closed`. The reason holds for the Lantern's coarse box
     (720 deg total curvature), but the exemption has no bound: any closed part passes with any `lost_cut`. A bound
     such as 15 % would have covered the Lantern (12.11 %). ABeautifulGame Jump on is also `wraps_closed: true`
     (`cargo test ... w1_the_chess_board -- --nocapture`), though its lost_cut is only 0.12 %. **Minor.**
- `tool_options` tests: two round-9 expectations changed with the intended new wording ("runs off" now requires
  `reached_edge`; `ends_share` 0.95 with `passed` now gives the plain ends line). These are behaviour changes, not
  weakening.
- Concurrency and panics: the new `WrapParam::memo` is a `Mutex` with poisoned-lock fallbacks. Test knobs
  (`relax::knob`) read the environment only under `cfg(test)`; release builds return the default. The new code
  indexes `Vec`s by ids it built itself, and `closes_on` / `hidden_at` handle missing entries and non-finite input.
  I found no race and no new panic path. Determinism (below) also holds.

## Sweep: wrap_surface, 13 models + noisy400, tiles300, parts200, trim_strip, both exes

Tables: `table_sw.txt`, `table_r9.txt`. Exit codes, stable vs r9:

| model | stable (w2) | r9 | note |
|---|---|---|---|
| WaterBottle, Avocado, DamagedHelmet, ngons_tiling, torus_knot, mobius, klein_bottle, flat_annulus, high_poly_sphere, trim_strip | ok | ok | |
| Lantern | ok | red | lost_cut 12.11 % now exempt (`wraps_closed`) |
| ABeautifulGame | ok | red | coverage via `room`; Jump on dup 2.674 -> 1.886 % |
| many_islands | ok | red | coverage via `room` |
| parts200 | ok | red | lost_cut 27.69 % -> 0 (w1 item 4) |
| nonmanifold | **red** | ok | dup 1.523 -> 3.414 % > 3 % (w2 item 4 side effect) |
| noisy400 | red | red | dup 9.97 -> 5.12 %, ink 0.30 -> 0.47; still a shattered picture |
| tiles300 | red | red | Jump on ink 1.3286 > 1.3 in both; param 5,396 -> 1,768 ms; lost_cut 16.85 -> 0 |

My w2 rows match w2's own table (`target\w2\table_w2a.txt`) number for number on all 13 models, so the sweep is
deterministic across runs and sessions. Other changes against r9: WaterBottle stretch 3.8 -> 2.8 %, high_poly_sphere
ink 1.0977 -> 1.0643, klein ink 1.0132 -> 0.9824 (area push); ABeautifulGame Jump on passed -> met_itself, ends
0.588 -> 0.738, ink 0.846 -> 0.906, rejected 45.6 -> 50.3 %, flipped 69,120 -> **70,583**. This flipped count is
above round 9's; w1 had 58,604. w2's notes do not mention it.

Speed: small models' param times rose (Avocado Jump on 22 -> 59 ms, nonmanifold 9 -> 28-35 ms, DamagedHelmet Jump on
58 -> 78 ms, WaterBottle 14 -> 19 ms). Heavy models are faster or unchanged (ABeautifulGame Jump on 4,079 -> 3,442 ms,
tiles300 5,396 -> 1,768 ms, high_poly_sphere 2.93-2.96 s -> 3.01-3.40 s, within noise on one run each). None of this is
user-visible. No idle protocol was followed.

### Panel lines (step 10 log, Jump on)

- ABeautifulGame: "stretched up to 92 %", cut line, **"The ends meet at the back: 74 % of the picture's length
  shown"**, "50 % of the laid picture lies on surface hidden ...", "Continued onto 30 separate parts", shared texels.
  The ends line is **wrong**. `crops\abg_on_board5.png` (5x) shows all 8 columns and all 4 rows of the cells picture
  on the board (row 2 red ... slate, row 3 teal ... purple), and `covered` is 1.0000. `wrap_stats_texts`
  (`tool_options.rs`) shows the ends line whenever `met_itself && ends_share < 0.95`, with no check against
  `covered`. Since w1's per-component exact fallback, the board's layout no longer follows the spine that looped
  round a pawn, so `ends_share` no longer describes what is shown.
- torus_knot: "Longer than the way round: 43 % shown, the ends meet at the back" (covered 0.444): consistent.
- Lantern: "12 % of the picture is lost where it is cut open round the closed part's corners" + "18 % ... hidden or
  folded": matches the pixels (two bare slivers on the front faces, plinth wedge).
- many_islands: ends 37 %, 6 % hidden, "The part is narrower than the picture: only 3 % ...", 2 parts: consistent.
- Avocado: "35 % of the laid picture lies on surface hidden under another part ..., and is not shown there" while ink
  is 0.996 (the picture shows on the seed instead). Defensible because of "there", but it reads like a 35 % loss.
  DamagedHelmet: 18 % hidden, plus the shared-texel line (86.7 % mirrored UVs, model). **Minor wording.**
- nonmanifold and noisy400 stop at the ink/dup assert before the panel step, so their lines are not logged.

### Shots read (crops in `crops\`, 4-8x, stable top/left, r9 bottom/right)

- ABeautifulGame Jump on (`abg_on.png`, `abg_on_board5.png`): the rows are parallel to the frame's top edge (r9
  turned about 35-40 deg) and the whole picture is on the board: **the rotation fix holds**. A dark bare streak runs
  across rows 1-2 in front of the pawn row (`abg_on_gap8.png`). It is also present in r9 (pre-existing; rejected
  cells). Jump off (`abg_off4.png`): identical to r9. The picture shows on every pawn through instanced shared UVs
  (model-caused; panel line shown).
- torus_knot (`torus4.png`): same as r9, one column of cells along the tube.
- Lantern (`lantern6.png`): two bare slivers on the front faces and a bare wedge on the plinth, as in r9.
  Topology-caused (coarse closed box); the panel names it.
- Avocado (`avo4.png`): better than r9. The left column (black, purple, red) now lies on the flesh. A bare flesh
  wedge stays between it and the seed; the picture is continuous on the seed's steep flank, so nothing is lost.
- DamagedHelmet (`dh4.png`, `dh4r.png`): nearly identical to r9. Kinked columns and a cut crack in the red cell on
  the left; overlapping plates on the right (model: overlapping shell plates, 86.7 % shared UVs).
- nonmanifold (`nm5_off.png`): the front page now carries cells over about two thirds of its width (r9: stripes). The
  triple-duplicated triangle (OBJ faces 175-177) next to the book now shows a rosy-brown stripe. Its UVs (0.1..0.3)
  overlap pages 1-2 (u 0..0.27), so it is texel sharing (model-caused). The panel shows no shared-texel line because
  the pages' footprints are less than half shared (round 9's majority rule).
- high_poly_sphere (`hps4.png`): same placement; the cells on the bumps are slightly more even.
- many_islands (`mi8.png`): identical to r9.
- noisy400 (`noisy4.png`): still a shattered star of strips in both exes. Stable lays more of it, but fragments land
  outside the frame (top right). The model is a synthetic random field (amplitude 0.6 per step: curvature everywhere,
  2,623 seams), so I class it as model-caused. It is still garbage output, and the panel line cannot be checked
  because the run stops at the assert first.

## Non-wrap identity vs wte_r9.exe

15 scenarios with both exes (`ident_cmp.txt`): stamp, text, text_warp, paint3d, paint2d, export_run, export,
roundtrip, brush_types, final_bake_async, multi, layers, stamp_edge_path (Avocado), workflow_path (DamagedHelmet),
text_preview_path (Avocado). All 30 runs ok. 41 files byte-identical, **including every export file** (export_run,
final_bake_async exports and ref, workflow_path's two DamagedHelmet PNGs). Differences of the known kinds only:
`shot.json` (`frames` timing), `export\shot.png` (932 px at x 313-449, y 153-164: folder text),
`final_bake_async\shot.png` (toast region x 686-977, y 660-702), and `.wtep` (`manifest.json` `saved_unix` only).

## Determinism

`WTE_SYNC_FINAL_BAKES=1`, default threads vs `RAYON_NUM_THREADS=1`, wrap_surface on DamagedHelmet and
ABeautifulGame (`det\`): every `WrapStats` line (9 per run, `micros` removed) is identical, as are the hash/ink lines
and every shot. The exceptions are ABeautifulGame `shot_off.png` (7,693 px, all in the toast region x 784-976,
y 734-774) and `.wtep` `manifest.json` (save time). The default run's shots also equal the sweep run's
(`shot.png` byte-identical).

## Defects

1. **Major, code**: ABeautifulGame Jump on: the panel says "The ends meet at the back: 74 % of the picture's length
   shown", but the whole picture is on the board (`crops\abg_on_board5.png`, covered 1.0000; `sweep_sw\ABeautifulGame\shot.log`
   "panel wrap lines"). The ends line ignores `covered`. This refutes w1 item 6's "accurate lines" for this model.
2. **Minor, model**: nonmanifold wrap_surface step 10 red: dup 3.414 % > 3 % (r9 1.523 %), the documented side effect
   of the w2 owner-visibility rule on a seven-page book. The front page now shows the picture; it needs an
   orchestrator decision.
3. **Minor, code**: WA20 still red on cells (0.690..1.986 vs 0.70-1.40); disclosed, after two attempts.
4. **Minor, code (test strength)**: step-10 `room` exemption: the stated reason ("errs on the side of the assert") is
   inverted. `room` underestimates the surface's room, so the assert is more lenient than geometry requires. The
   `wraps_closed` exemption removes the lost_cut limit with no bound. Both have a geometric reason, so neither is
   cheating.
5. **Minor, code**: ABeautifulGame Jump on flipped triangles 70,583 (w1 58,604, r9 69,120) after w2. Not in w2's notes,
   and no visible effect found in the shot.
6. **Minor, model**: noisy400 still a shattered picture with fragments outside the frame (both exes); synthetic
   pathological field.
7. **Minor, model**: tiles300 Jump on ink 1.3286 > 1.3 (unchanged from r9; synthetic tiles).
8. **Minor, code (wording)**: the "hidden ... not shown there" line reports `rejected` as a share of the laid picture
   even when the picture shows elsewhere (Avocado 35 % with ink 0.996), which reads like a loss.
9. **Minor, model**: nonmanifold's duplicated triangle shows a stripe of the front page's texture through overlapping
   UVs, with no shared-texel line (majority rule of uv_share).
10. **Minor, code**: small-model param times are up 1.3-3x (Avocado Jump on 22 -> 59 ms). Not user-visible.
