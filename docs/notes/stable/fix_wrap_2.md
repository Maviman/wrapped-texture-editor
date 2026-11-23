# Stable: fix-wrap (pass 2)

Builder (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable-fix-wrap` off `stable`
(b064fee). Inputs: `docs/notes/stable/critic.md`, `docs/reports/STABLE_GATE_REPORT.md`, `docs/notes/stable/check_exp_2.md`.
Raw outputs (git-ignored) under `target\stable\fixwrap\`: `bin\` (exe copies: `wte_stbase.exe` = the gate's stable exe
9EB3F988, `wte_fw1..7.exe` = this branch's builds, `wte_fw7.exe` final), `sweep_fw*\` with `table_fw*.txt`, `det\` with
`det_cmp.txt`, `timing\` (`driver.log`), `tint_fw3*.txt`, `perf.log`, `test.log`, `clippy.log`, `build.log`, scripts in
`tools\`. No export code and no layer-panel code was touched. No new crate; `Cargo.toml` / `Cargo.lock` unchanged.

## 1. ABeautifulGame: the cut at the flush marble frame

**What actually cuts the picture** (instrumented `w1_the_chess_board_keeps_the_rows_under_a_pawn`, debug prints since
removed):
- The board is 5 welded components; the frame is 4 separate parts (one per side). The frame's left side (part 20 of the
  patch) *is* entered by Jump small gaps: from a slab vertex 7.8 mm inside the gold inlay ridge (the gap is 10.1 mm), at
  depth 0.155. So the frame is not "not jumpable at gap 0": it is reached across the 1-2 mm ridge.
- The spine closed round the pawn under the anchor (`met_itself`, spine -0.138..+0.162 along the pawn), so the depth
  (`key`, geodesic distance from the spine) on the board is about the radial distance from that pawn: 0.164-0.184 along
  the slab's left edge, against the front's stop of 0.185 (`WRAP_STOP_SLACK` 1.7 x the across reach 0.109). The frame
  strip the picture should cover lies 0.025-0.066 beyond the edge, so beyond the stop. Only 78 frame triangles were laid
  (1.1 % of the picture).
- A larger stop (x 1.4, an experiment knob) laid the frame but turned the board's rows by 31 degrees and dropped
  `shown` to 0.827: not a fix.

**Attempt 1** (a part whose box touches another part's box, within 1e-4 x the patch diagonal, is entered again wherever
its own front has not reached): the chess pieces' boxes overlap the board's, so every piece vertex asked the BVH:
the front phase went from 0.7 s to 131 s (debug). Rejected.

**Attempt 2** (touching only face to face: apart, or overlapping by at most the tolerance, along an axis; a landing
within 1e-4 x diag, facing the same way, on a triangle the part's own front had not reached; no jump counted): the
synthetic test passes (plate B 33.6 % -> 100 % of its texels right). The sweep: every model bit-identical except
ABeautifulGame Jump on, which got 5 landings between a chess piece's touching ring parts (parts 81 / 82 / 85, not the
frame): seams 2,147 -> 2,269 (S2 52 -> 173), rejected 50.33 -> 50.53 %, flipped 70,583 -> 68,889, ink 0.9055 -> 0.9052,
dup 1.886 -> 1.879 %, scatter 1.151 -> 1.148 %, `shown` unchanged (0.9127), shot: a few pixel spots on two pieces
(`abg_shot_cmp.png`). Speed was fine (front phase +0.06 s debug; release within 2 %). Under the rule "no regression in
any metric beyond noise" the seam count (+5.7 %) is a regression, so **attempt 2 was not kept** (reverted in the code).
With a frame-strip gain of 0.05 % of the picture it would not have fixed the board's cut anyway.

**Result: the cut stays** (it is the front's reach from a spine that closed round a pawn, not a gap). The synthetic test
`fix_jump_continues_across_flush_coplanar_plates` (two flush coplanar plates, the seam parallel to the spine, Jump on)
is in the tree as `#[ignore]` with the reason "known limit": a flush touching part is entered once, so a long flush
seam is laid only near the first landing (plate B 33.6 % within 2/255). The user may want attempt 2 anyway (decision
below).

## 2. Panel lines

New `WrapStats` fields from the owner grid (core, `OwnerGrid::classify_bare` / `bare_cells`, ARCHITECTURE 6.5a
"Stable (fix-wrap)"): `gaps` (bare runs of at most the jump gap between cells of two different parts) and `at_part`
(bare cells next to a laid cell where the picture stops at a neighbouring part: the owner is on a continued part that
goes on where the front stopped, or another part's unlaid surface is within the jump gap; the bare cells go with their
nearest laid cell). The panel (ARCHITECTURE 8.7):
- "P % of the picture falls into the gaps between separate parts" (help text);
- "P % of the picture stops where it meets a neighbouring part";
- "runs off the surface: an edge, a hole or the end of the part" now gives only the rest
  (`1 - shown - beyond - lost_cut - gaps - at_part`) and shows from 2 %.

Sweep (final exe, Jump on), before -> after:

| Model | gate line | now |
|---|---|---|
| ABeautifulGame | 9 % ... runs off the surface: an edge, a hole or the end of the part | **9 % of the picture stops where it meets a neighbouring part** (`at_part` 8.60 % of 8.73 % bare) |
| parts200 | 28 % ... runs off the surface | **28 % of the picture falls into the gaps between separate parts** (`gaps` 27.69 %) |
| tiles300 (red, newly logged) | not logged | 17 % falls into the gaps (16.85 %); 5 % overlap; continued onto 21,860 parts |
| nonmanifold (red, newly logged) | not logged | 82 % runs off the surface (edge; no gaps, no neighbour); 70 % overlap |
| noisy400 (red, newly logged) | from one screenshot | 3 % lost to cuts; 45 % overlap; Only 47 % lies on the surface (as the screenshot) |
| DamagedHelmet | 3 % ... runs off the surface | **Only 97 % of the picture lies on the surface**: of its 2.6 % bare, 1.96 % is lost to cuts inside the laid outline and 0.52 % in gaps between plates, 0.1 % is left at an edge |
| all others | | unchanged (many_islands, Lantern, torus_knot, mobius, flat_annulus, ngons_tiling, trim_strip, ...) |

Checked against the numbers: every line's share is the grid's own count; the shares add up with `shown`. The ABG strip
is on the frame (check_wrap_2's ray cast and the shot); parts200 and tiles300 look whole at screen size.

`wrap_surface` step 10 now logs the panel lines before its asserts (the three red models log them), adds `gaps` /
`at_part` to the `coverage:` and `panel lines:` lines, and logs a `composite hash` per Jump setting.

## 3. DamagedHelmet shared texels: 100 % vs 86.7 %

Two measures, both right. 86.7 % is the whole helmet's surface on triangles that share texels (`UvShare::mesh_share`,
the warm-up's log line "shared texels found ... (86.7 % of the surface)"). 100 % is the stamp's share
(`Document::stamp_shared_texels`: of the 16 x 16 picture points on the surface, those on such triangles), which decides
whether the panel shows its line (from 5 %); the line itself has no number. The visor under the picture is mirrored.
New test `fix_damaged_helmet_shared_texels_are_two_measures`: an independent 4,096 count raster (per picture point, is
the texel under it covered by two footprints) gives 1.0000 for 251 points; mesh share 0.8675. No fix needed; the
scenario's log now says "shared texels under the picture".

## 4. Determinism (final exe)

`WTE_SYNC_FINAL_BAKES=1`, `RAYON_NUM_THREADS=1` against the default, `wrap_surface` on DamagedHelmet, ABeautifulGame
and WaterBottle with `wte_fw7.exe` (`det\`, `det_cmp.txt`): the new `composite hash` lines (every texture set's
composite with the wrapped stamp baked in, Jump off and on) are **byte-equal** in all 6 pairs (DamagedHelmet
e3a5c6a2fda4a30e / a2c8870c3f424c6d, ABeautifulGame fe87556ade49ca2b / 5d3e1b968ada7a32, WaterBottle bf04fc5cd9aad83f
twice); every `WrapStats` line (build time removed), quality line and panel line is equal; all 21 shots (7 per model)
are pixel-identical. The only differing log line is the frame number at which a step ran.

## 5. WaterBottle exact-label tint check

The round-9 gate probe (`compat_probe`, rebuilt against this branch): a round-6 project's label re-baked after a tint
change and back, compared with the round-6 bake: **body 67.81 %** within 2/255 (round 9 68.14 %; round-8 project
86.52 %, round 9 87.69 %). The same probe with `--rebase` (re-baked by this code first): **100.00 %** everywhere (max 0).
New test `fix_the_water_bottle_label_is_exact_on_the_body`: on the WaterBottle sample with step 10's placement, the 31
body triangles meeting the picture keep their 3D edge lengths in the layout within 0.00014 texels (mean 0.00002): the
body is laid exactly. So the 68 % is round 6's layout against this exact one (the label moves by about a texel), not a
code fault of this build. Not changed.

## 6. Wall-clock unit tests

Choice: **moved the 200 ms limit to serial perf tests** (`perf_wa7_parameterizations_within_200_ms`,
`perf_check_a_own_meshes_within_200_ms`, `#[ignore]`, run by the gate with
`cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`). The functional tests keep every other assert and
print their slowest time. The limit (200 ms, best of up to five runs) and the cases are unchanged.

Why not the thread's CPU time: I measured it (`GetThreadTimes` via a kernel32 declaration, no crate, on a one-thread
rayon pool so the parallel parts count): the whole work on one thread is 3-5 x the parallel wall time (doubled_cube
500 ms CPU against 115 ms wall; book x0.05 328 ms against 66 ms), and `GetThreadTimes` advances in 15.6 ms ticks. The
same 200 ms bound could not stay. (windows-sys 0.52 / 0.61 are in `Cargo.lock`, but a direct dependency was not needed
for that probe and the approach was dropped.)

Serial run on this machine: check:A slowest 85.7 ms, WA7 slowest 108.6 ms (`perf.log`); in the parallel suite the same
cases printed 136-198 ms.

## Final results (`wte_fw7.exe`, SHA-256 97A70242D70767483F84062BA6A1B52BA2CEAAE507065A675BA16815B42041C7)

- `cargo test --workspace --no-fail-fast`: wte_app 372 passed / 0 failed / 17 ignored; wte_core lib 792 passed /
  **1 failed** (`wa20`, `cells 0.689751..1.986138`, unchanged) / 23 ignored (the 2 new perf tests and the known-limit
  test among them); brush_tips 15, end_to_end 9, project_roundtrip 23. **1,211 passed, 1 failed, 40 ignored.**
- `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`: 2 passed (85.7 / 108.6 ms of 200).
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok.
- Sweep (17 models, `sweep_fw7\`, `table_fw7.txt`): every metric field of every model equal to the gate's
  (`target\stable\gate\sweep_st`); exit codes as the gate (red: nonmanifold, noisy400, tiles300, the same asserts);
  panel lines changed only for ABeautifulGame, parts200, DamagedHelmet (above) and newly logged for the three reds.
- Speed: measured on an idle machine (CPU 1-17 %, no other cargo / app process; `timing\` from the run with
  `wte_fw3.exe`, which had the same bare-cell classification plus attempt 2), two alternating runs each, param ms:
  high_poly_sphere Jump on 3,345 / 3,329 against 3,334 / 3,378 (stable), off 3,444 / 3,327 against 3,276 / 3,318;
  ABeautifulGame on 3,425 / 3,453 against 3,460 / 3,511, off 529 / 533 against 525 / 527; tiles300 on 1,834 / 1,856
  against 1,820 / 1,819 (+1 %), off 16.4 / 16.9 against 12.2 / 12.6. All within 2 % except the tiny Jump-off wrap (+4
  ms: the grid's passes). The repeat with the final exe ran while another worktree's build held the CPU at 100 %
  (`driver.log`), so it is not a speed measure (it shows no outlier either way).

## Decisions for the user

1. Keep the ABeautifulGame cut at the marble frame as a known limit (the front's reach from a spine closed round the
   pawn under the anchor; the panel says "stops where it meets a neighbouring part"), or a later change to the stop
   for a spine that closed round a small part.
2. Take attempt 2 (a flush touching part entered again; ABeautifulGame seams +5.7 %, visually the same, flipped -2.4 %)
   or keep the known limit (`fix_jump_continues_across_flush_coplanar_plates` ignored).
3. The gate runs the perf tests serially (`cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`).
