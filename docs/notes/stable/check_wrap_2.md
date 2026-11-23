# Stable: independent check of the wrap branch (pass 2)

Checker (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable-wrap` at e265f0c
(w1 + w2 + check_wrap_1 + fix_wrap_1 on df93777). Read-only for sources. Raw outputs (git-ignored) under
`target\stable\check_wrap_2\`: `test.log`, `test_int.log`, `clippy.log`, `build.log`, sweeps `sweep_c2\` / `sweep_r9\`
with `table_c2.txt` / `table_r9.txt`, identity runs `ident_c2\` / `ident_r9c\` with `ident_cmp.txt`, determinism runs
`det\` (`det.log`), crops `crops\*.png` and `abg_on5.png`, `abg_on_left8.png`, scripts `tools\` (the earlier checker's
`sweep.sh`, `run1.sh`, `crop.py`, `ident_cmp.py`, fix_wrap_1's parser as `sweep_parse_f.py`, plus `ident2.sh`,
`abg_cast.py`, `abg_board_edges.py`).

Exes: `bin\wte_c2.exe` = `target\release\wrapped_texture_editor.exe` built from e265f0c (SHA-256 6CCB7FBA...,
identical to fix_wrap_1's `bin\wte_f1.exe`); `bin\wte_r9c.exe` = copy of `target\round9\gate\bin\wte_r9.exe`
(4F9E1422..., hash checked).

Python note: the Python 3.11 under `AppData\Local\Programs` has no Pillow; the Store Python 3.11.9
(`AppData\Local\Microsoft\WindowsApps\python`) has Pillow 11.3 in its user site, so I ran it with `-P` (not `-I`,
which hides the user site). Only my own scripts were run.

## Verdict

I could not refute the fix pass on the defect it was given: the false "The ends meet at the back: 74 %" line on
ABeautifulGame with Jump on is gone, and the wrap, the bakes, the exports and determinism are unchanged. Tests, clippy
and non-wrap identity hold. The reds that remain are the ones already disclosed: WA20 (cells), nonmanifold step 10 (dup
3.414 %), noisy400 and tiles300. One new minor finding: on ABeautifulGame with Jump on, the 9 % of the picture that the
panel now says "runs off the surface" lies on the flat marble frame. That frame is a separate part, flush with the
playing slab and touching it, so it is the case Jump small gaps exists for. The loss itself is not new (round 9 showed
even less of the picture: ink 0.846, now 0.906).

## Build, tests, clippy

- `cargo test --workspace`: wte_app 364 passed / 16 ignored; wte_core 775 passed / **1 failed** / 20 ignored. cargo
  stops after a failing crate, so I ran the three integration binaries on their own (`test_int.log`): brush_tips 15,
  end_to_end 9, project_roundtrip 23, all green. **Total 1,186 passed, 1 failed, 36 ignored**, the same as the fix pass reported.
  Failing: `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`, `cells 0.689751..1.986138`
  against the 0.70-1.40 limit (`wrap_tests.rs:2035`).
- `w1_the_chess_board_keeps_the_rows_under_a_pawn` ran (it is not skipped: the sample is present) and passed with the new
  `beyond < 2 %` and `shown >= board share` asserts.
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok (up to date, hash above).
- Line endings: every file changed since round9-wip is `i/lf w/lf`.

## Diff review (fix_wrap_1, plus round9-wip..HEAD test files)

- No assert or limit was deleted or loosened in fix_wrap_1. It added asserts: the chess board (`beyond < 0.02`,
  `shown >= share`); WA32 (`beyond >= 0.02`, `|shown + beyond + lost_cut - 1| <= 0.01`); and step 10, where `lost_cut`
  for `wraps_closed` is now bounded at 15 % (w2 had no bound). Panel-test expectations changed with the new wording and
  with the measured `shown` / `beyond`. The many_islands Jump-on case lost its "ends meet 33 %" line: it has beyond 0 and
  no closure seam, so this is a behaviour change, not a weaker test. Since round9-wip the only loosened limits are still
  w2's two step-10 exemptions (`room`, `wraps_closed`), each with a geometric reason. check_wrap_1 already classed them as minor.
- New code: `OwnerGrid::owned_cells` slices rows within `picture_cells` bounds (i1 < width). `lost_cells` now also
  returns the count of the `beyond` flood. `size_stats` guards both new shares against zero denominators. No
  allocation or lock was added, I found no new panic path, and nothing is shared across threads. For Overlap turns,
  `owner` is `None` on clipped layers, so `shown` falls back to `covered`. The 3-turn Overlap wraps in the sweep logs
  report covered 1.0 and shown 1.0, which is consistent.

## Sweep: wrap_surface, 13 models + noisy400, tiles300, parts200, trim_strip, both exes

`table_c2.txt` equals fix_wrap_1's `table_f.txt` field for field (param and applied times aside). `table_r9.txt` equals
check_wrap_1's r9 table. Exit codes: stable 14 ok, red on nonmanifold (dup 3.414 % > 3 %), noisy400 (dup 5.124 %) and
tiles300 (Jump on ink 1.3286 > 1.3); r9 red on Lantern, ABeautifulGame, many_islands, noisy400, tiles300 and parts200. Every
shot from `wte_c2` is byte-identical to fix_wrap_1's.

Panel lines (Jump on, step-10 log), checked against the numbers and the shots:
- ABeautifulGame: stretched 92 %, cut, "9 % of the picture runs off the surface ...", "50 % ... overlaps ... shown in
  one place only", 30 parts, shared texels. There is **no ends line** (ends 73.8 %, beyond 0, shown 0.9127, ink
  0.9055), so the major from check_wrap_1 is fixed. See the finding below for where the 9 % is.
- torus_knot: "Longer than the way round: 43 % shown, the ends meet at the back" (shown 0.442, beyond 0.558, ink 0.449).
- Lantern: cut 12 %, overlap 18 %, "The part is narrower than the picture: only 73 % ..." (`crops\lantern_panel2.png`;
  shown 0.731, room 0.909). This is consistent with the shot (`crops\Lantern_c2_vs_r9.png`: the same two bare slivers and
  plinth wedge as r9; topology, a coarse closed block).
- Avocado: "35 % ... overlaps ... one place only" with shown 1.0 and ink 0.996. The wording no longer reads as a loss
  (check_wrap_1 minor 8 is fixed).
- DamagedHelmet: "3 % runs off" (shown 0.974; the cells reach the visor's rim), overlap 18 %, 9 parts, shared texels
  (86.7 %, model).
- many_islands: overlap 6 %, "narrower than the picture: only 3 %" (room 0.103), 2 parts.
- noisy400 and nonmanifold stop at the dup assert before the panel step (unchanged).

Shots read (4x crops in `crops\`, stable left/top, r9 right/bottom):
- ABeautifulGame Jump on (`abg_on5.png` at 5x, `abg_on_left8.png` at 8x): the rows are parallel to the frame's top edge
  and all 8 columns and 4 rows show. The leftmost column is cut by a slanted line, beyond which the dark marble frame
  shows bare; the bottom-right corner is notched round a pawn's base.
- Avocado, DamagedHelmet, Lantern, high_poly_sphere, many_islands, torus_knot: the same as check_wrap_1 found (Avocado
  better than r9; DamagedHelmet's kinked left columns and overlapping right plates as in r9; the others unchanged).
- nonmanifold (`nm_book6.png`): the front page carries cells over about two thirds of its width, the rest stays
  striped; the duplicated triangle shows a stripe (shared UVs). The double cone far right now shows its own blue/white
  base texture where r9 showed teal picture through shared texels.
- noisy400: still a shattered star of strips with fragments past the frame, in both exes (synthetic, model).

Timing (one run each, no idle protocol): small models' param times are as in check_wrap_1 (Avocado Jump on 60 ms vs r9
25 ms, nonmanifold 28 vs 11 ms, DamagedHelmet Jump on 79 vs 60 ms). high_poly_sphere ran 3.65 s with r9 and 3.77-3.92 s
with stable in this session (machine noise: r9 measured 2.93 s earlier). None of this is user-visible.

## New finding: ABeautifulGame Jump on, the 9 % "runs off" is on a flush neighbouring part

- `tools\abg_cast.py` casts rays along the projector's -Z (from the logged placement bits) through a 32 x 16 grid
  over the picture. Every ray hits the Chessboard mesh or a piece standing on it, so there is no hole or drop under the
  picture.
- `tools\abg_board_edges.py`: the Chessboard mesh has 0 boundary edges after a 1e-5 weld and 5 welded components. The
  playing slab (top y 0.0169) with its gold inlay ridge (x -0.254..-0.250, y 0.0185, 1.6 mm high) is one component.
  The marble frame (top y 0.0169, flush, from x -0.256 outward) is another, separate component.
- The picture is 0.405 x 0.202 laid at true size. Its left corners land at about x -0.32 / -0.26, so the left strip
  reaches past the slab's edge onto the frame. That strip and the notch at the bottom-right pawn base are the bare 8.7 %
  (`shown` 0.9127, ink 0.9055).
- The panel says "9 % of the picture runs off the surface: an edge, a hole or the end of the part". "The end of the
  part" is literally true, but the picture would continue flat on a touching part, which is the case Jump small gaps
  is for (the same wrap jumps onto 30 pawns and pieces). The wrap does not jump onto the frame.
- This is not a regression: r9 showed less of the picture (ink 0.846, rotated). The new line is the honest
  number for this, and better than w2's false ends line.
- **Minor, code** (wrap coverage across a flush seam; not model-caused, since the gap between the parts is about
  1-2 mm and the surfaces are coplanar).

## Non-wrap identity vs wte_r9.exe

15 scenarios with both exes (`ident_cmp.txt`): stamp, text, text_warp, paint3d, paint2d, export_run, export, roundtrip,
brush_types, final_bake_async, multi, layers, stamp_edge_path (Avocado), workflow_path (DamagedHelmet),
text_preview_path (Avocado). All 30 runs are ok and **41 files are byte-identical**, including every export file: export_run's
sphere .ora/.png/.psd and its 4 layer PNGs, final_bake_async's exports and refs, and workflow_path's DamagedHelmet .ora and
PNGs. The differences are only the known ones:
- `shot.json`: `frames`.
- `export\shot.png`: 916 px at x 313-449, y 153-164 (folder text).
- `final_bake_async\shot.png`: 9,728 px at x 686-976, y 660-701 (toast).
- The three `.wtep` files: `manifest.json` `saved_unix` only.

## Determinism

`WTE_SYNC_FINAL_BAKES=1`, wrap_surface on DamagedHelmet and ABeautifulGame, default threads vs `RAYON_NUM_THREADS=1`
(`det\`):
- All 9 `WrapStats` lines per run (`micros` removed), both quality lines and the panel lines are identical.
- DamagedHelmet: all 7 shots are identical, and byte-equal to the sweep run's.
- ABeautifulGame: 5 of 7 shots are identical. `shot.png` and `shot_off.png` differ only at x 784-977, y 697-774. That is
  the toast stack ("Undo: Edit Stamp (x5)" / "Saved shot_wrap.wtep"; `crops\abg_toast.png`), so it is timing, not wrap
  output.

## Earlier defects: status

| check_wrap_1 defect | now |
|---|---|
| major: ABG Jump on false "ends meet 74 %" line | **fixed** (no ends line; beyond 0) |
| minor, model: nonmanifold step 10 dup 3.414 % | still red, unchanged (orchestrator decision) |
| minor: WA20 cells 0.690..1.986 | still red, unchanged |
| minor: step-10 exemptions | (b) fixed (15 % bound); (a) `room` lower bound still lenient, now documented |
| minor: ABG flipped 70,583 | unchanged; attributed to w2's area push (not re-verified by me) |
| minor, model: noisy400 shattered | unchanged |
| minor, model: tiles300 ink 1.3286 | unchanged |
| minor: "hidden ... not shown there" wording | **fixed** |
| minor, model: nonmanifold duplicated-triangle stripe, no shared line | unchanged |
| minor: small-model param times up | unchanged |
