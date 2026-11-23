# Stable, fix pass 1 on the wrap branch

Builder (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable-wrap`, on top of
check_wrap_1 (5fdf323). Input: the six defects of `check_wrap_1.md` handed to this pass (1 major, 5 minor). Raw outputs
(git-ignored) under `target\stable\fix_wrap_1\`: `test.log`, `clippy.log`, `build.log`, sweeps `sweep_a\` (first exe)
and `sweep_f\` (final exe) with `table_a.txt` / `table_f.txt` (the checker's parser plus `shown`, `beyond` and the
logged panel lines), identity runs `ident_f1\` with `ident_cmp.txt` (against the checker's `ident_r9c\`), panel crops
`panel_abg.png` (top: this pass, bottom: w2) and `panel_lantern.png` (Lantern, Avocado), scripts in `tools\`. Exes:
`bin\wte_f1.exe` (final, SHA-256 6CCB7FBAEE69E941C12B3CA188C2B173DF4756A7C6CC61B0662ACACEAC9A8568), `bin\wte_f1a.exe`
(first build, before the last panel-rule edit; same step-10 numbers and lines on all 17 models).

## Summary

| # | Defect (check_wrap_1) | Result |
|---|---|---|
| 1 | major: ABeautifulGame Jump on "The ends meet at the back: 74 % ..." with the whole picture on the board | **fixed** (new `WrapStats::shown`, `beyond`; panel rule); regression tests |
| 2 | minor: WA20 cells 0.690..1.986 | not attempted: w2 made two attempts (rule: stop); numbers unchanged |
| 3 | minor: step-10 exemptions (room lenient, lost_cut unbounded when closed) | (b) **fixed**: 15 % bound; (a) classified and documented, assert unchanged |
| 4 | minor: ABeautifulGame flipped 70,583 | **attributed**: w2's area push, +11,979 exactly; on the jumped pieces, not the board; not changed |
| 5 | minor: "hidden ... not shown there" reads like a loss | **fixed**: new wording; also fed by the honest `shown` |
| 6 | minor: small-model param times up | **attributed**: w2's owner visibility rays (Avocado owner phase 22 -> 54 ms); not changed |

One commit: `ecb5553` (code, tests, ARCHITECTURE); these notes in a second commit.

## 1. The ends line on the chess board (major, code)

**Cause.** The ends line trusted the spine's `ends_share` (the traces' arc lengths up to the meeting) whenever
`met_itself` and `ends_share < 0.95`. On ABeautifulGame with Jump on, each trace jumps onto a pawn, loops round it and
meets its own path (ends 73.8 %), but since w1's per-component exact fallback the board is laid from its own exact
layout and holds the picture beyond the spine's ends. The checker's other observation also holds: `covered` (1.0) is
not "the whole picture is shown" either, because it sums the clipped laid areas of every stored triangle: with Jump on
the board and the pawns are laid over the same picture points, so it counts them twice and clamps at 1.

**Probe** (temporary ASCII map of the top layer's owner grid over the picture, removed after use): 91.3 % of the
picture's cells have an owner, almost all on the board; the bare 8.7 % is a slanted strip along the picture's left
edge and its bottom-right corner, open to the grid border (the frame's corner past the board's front edge, `reached_edge`
true). No bare cell touches a closure triangle (S1 / S2 seam): the closure round the pawn cost no picture.

**Fix** (`wrap_param.rs`, `wrap_relax.rs`, `tool_options.rs`):
- `WrapStats::shown`: the share of the owner grid's picture cells with an owner (`OwnerGrid::owned_cells`), each cell
  once; `covered` when no layer has an owner grid.
- `WrapStats::beyond`: the share of the picture's cells without an owner that `lost_cells` already left out of
  `lost_cut` as beyond a closure (4-connected through bare cells to a cell a closure triangle covers); `lost_cells`
  now returns that count too.
- Both are size-dependent and computed in `size_stats` (fresh and reused parameterizations alike;
  `w1_reuse_equals_fresh_within_the_bucket` still passes with the new fields).
- Panel: the ends line with a share needs `ends_share < 0.95` and `beyond >= 2 %` (`ENDS_BEYOND_MIN`,
  `ends_cut_shown`); with `ends_share < 0.95` and nothing bare beyond the closure there is no ends line; the plain "The
  ends meet at the back" stays at `ends_share >= 0.95`. "runs off" and "Only P %" use `shown` instead of `covered`;
  the "Only" line's expected share is `ends_share` whenever an ends line shows (also the plain one).

**Checks of the rule.** WA32 (helices): `beyond` 0.339 / 0.164 / 0.281 and `shown + beyond + lost_cut = 1` (every bare
cell lies beyond the meeting line): the ends line stays. torus_knot: beyond 0.558, line unchanged ("Longer than the
way round: 43 % shown ..."). ABeautifulGame Jump on: beyond 0, shown 0.9127.

**Panel lines now (step 10, Jump on, `table_f.txt`), against w2 (`check_wrap_1\table_sw.txt`):**

| model | w2 | this pass |
|---|---|---|
| ABeautifulGame | "The ends meet at the back: 74 % of the picture's length shown", "50 % ... hidden ... not shown there" | "9 % of the picture runs off the surface: an edge ...", "50 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only" (fitted, cut, parts and shared-texel lines as before) |
| many_islands | "The ends meet at the back: 37 % ...", room line 3 % | room line 3 % only: the spine met itself without a closure seam (seams S4 / S5 only, beyond 0); the islands' room explains the share. Jump off (beyond 0.99) keeps the ends line (18 %) |
| Lantern | cut 12 %, "18 % ... hidden", (no covered line: covered 0.903) | cut 12 %, overlap 18 %, "The part is narrower than the picture: only 73 % of the picture lies on it" (`shown` 0.731: the folded layout's areas summed made `covered` 0.903) |
| DamagedHelmet | "18 % ... hidden" | "3 % of the picture runs off the surface ..." (shown 0.974), overlap 18 % |
| Avocado | "35 % ... hidden ... not shown there" | "35 % ... each spot of the picture is shown in one place only" (shown 1.0, ink 0.996) |
| parts200, ngons_tiling, mobius, flat_annulus, trim_strip | runs off 27 / 93 / 43 / 40 / 68 % | 28 / 93 / 43 / 40 / 68 % |
| torus_knot, WaterBottle, klein_bottle, high_poly_sphere | | unchanged |
| nonmanifold, noisy400 (stop at the dup assert before the panel step) | | would read 82 % runs off (shown 0.178, ink 0.188; covered said 0.63) and "Only 47 %" (shown 0.473, ink 0.472; covered said 0.862) |

`shown` agrees with the ink measure where the model does not share texels (nonmanifold 0.178 / 0.188, noisy400
0.473 / 0.472, ABeautifulGame 0.913 / 0.906); `covered` did not. Read in the shot (`panel_abg.png`): the new lines;
the 3D view is pixel-identical to w2 on every model (all differences lie in the panel, x >= 995, plus the known 2-10
px UI edge at x 1270-1271).

**Tests.**
- `w1_the_chess_board_keeps_the_rows_under_a_pawn`: also `beyond < 2 %` and `shown >=` the board's share (0.7995;
  shown 0.9127). This pins the measures on the real model; the panel test below carries the line itself (w2's rule
  fails it: it gives the "74 %" line).
- `wa32_a_helix_round_a_tube_has_its_ends_meet`: `beyond >= 2 %` and `|shown + beyond + lost_cut - 1| <= 0.01` on
  all three helices (the ends line must stay where the picture really is cut).
- `tool_options::the_wrap_lines_name_the_right_cause_and_never_contradict`: the sweep's numbers for ABeautifulGame (no
  ends line, "9 % ... runs off"), many_islands off / on, Lantern, DamagedHelmet, nonmanifold, noisy400, torus knot; the
  grid now runs over `beyond` (an ends line iff met or passed and `beyond >= 2 %`). The w2 expectations for the
  Lantern, many_islands Jump on, DamagedHelmet, nonmanifold and noisy changed with the measured `shown` / `beyond`
  (listed above); none of their asserts was removed.
- `a_picture_longer_than_the_way_round_says_how_much_is_shown`: data made consistent (`shown` 0.43, `beyond` 0.57; the
  95 % case `shown` 0.95, `beyond` 0.05).

## 2. WA20 (minor, code): not attempted

w2 made two attempts (item 3: the same-aspect target, then the grow-only area push kept); the rule is to stop after
two. Unchanged: cells 0.690..1.986 against 0.70-1.40, ink 1.0871, dup 0.899 %, scatter 0.482 %, seams 0, flipped 29.

## 3. Step-10 exemptions (minor, test strength)

- (b) `lost_cut` when `wraps_closed`: now bounded at **15 %** (10 % otherwise). Reason: the Lantern's closed block
  measures 12.11 %, round 8's cone rule on it 14.7 % (w2's notes); w2 had no bound. ABeautifulGame Jump on is also
  `wraps_closed` with 0.12 %. The sweep stays green on both.
- (a) `covered >= 0.85 x ends_share x room`: the checker is right that `room` (the parts' 3D extent across over the
  picture's height) is a lower bound of the surface's room on a curved part, so the factor makes the assert more
  lenient than the surface requires; w2's notes said the opposite. Not changed: an upper bound of a part's surface
  room across (a geodesic width) is not available without the layout under test. Margins as measured by the checker
  (ABeautifulGame off 0.0731 vs 0.0683, many_islands on 0.0338 vs 0.0323). ARCHITECTURE 8 (step 10) now says this.

## 4. ABeautifulGame Jump on flipped 70,583 (minor): attributed, not changed

`w1_the_chess_board_...` with test knobs (debug build, same placement): `WTE_TEST_AREA_PUSH=0` gives **58,604** (w1's
number exactly), `WTE_TEST_EXACT_DEVELOP=false` and `WTE_TEST_OWNER_VISIBLE=false` leave 70,583. So all +11,979 come
from w2's grow-only area push. The board is exact (its rows -2.11 degrees and its share 0.7995 are identical with and
without the push), so the flips are on the jumped pieces (pawn heads and bases: small closed curved parts under a
picture as wide as the board, where growing compressed triangles has no room and folds; the same effect as WA21's 3 ->
34 in w2). Those places lose the owner test to the board (`rejected` 50 %); the checker found no visible effect and
neither did I (the 3D view is unchanged by this pass). Not changed: the push is what turned WA4 green and WA20's ink.

## 5. The overlap line (minor, wording): fixed

A cell's dropped pairs (`rejected`) always belong to a cell that has an owner, so the dropped spot is shown at the
owner's place: no loss. New text: "P % of the laid picture fell where the surface overlaps another part or folds over
itself: each spot of the picture is shown in one place only". Real losses are now named by `shown` (runs off / only).

## 6. Small-model param times (minor): attributed, not changed

Release test probe (`check_b_probe`, step 10's placement bits, 5 runs each, `target\release\deps\wte_core-*.exe`):

| case | default | `OWNER_VISIBLE=false` | `OWNER_SHEETS=false` | `AREA_PUSH=0` | `EXACT_DEVELOP=false` |
|---|---|---|---|---|---|
| Avocado Jump on: total / owner ms | 58-61 / 53-57 | 25-27 / 21-23 | 57-58 / 52-53 | 58-60 / 54-56 | 60-64 / 55-60 |
| nonmanifold Jump off | 30-31 / 26-27 | 15-18 / 12-14 | 12-13 / 9-10 | 30-31 / 26-27 | 30 / 26-27 |

The rise is w2's owner visibility test: every mixed cell asks a ray along `out` (Avocado: 275,825 mixed cells of the
1,024 x 525 grid, pass 2 48 ms already on the pool). Pass 2 is already lazy (candidates in g order, stop at the first
not buried). Not user-visible (Final applied 89 ms on Avocado Jump on); left for a speed pass.

## Behaviour changes (intended)

- `WrapStats` has two new fields (`shown`, `beyond`); not saved in projects (FORMAT_VERSION 3 unchanged).
- Panel wrap lines as in the table of section 1; no pixel of any bake, Project-mode stamp, brush stroke or export changed.
- Step 10: `lost_cut <= 15 %` for a whole closed part; its `coverage:` line also logs `shown` and `beyond`; its
  `panel lines:` "line shown" for the ends share follows the panel's rule (ABeautifulGame now "no line").

## Verification

- `cargo test --workspace --no-fail-fast`: **1,186 passed, 1 failed, 36 ignored** (wte_app 364 / 16 ignored; wte_core
  775 passed, 1 failed, 20 ignored; brush_tips 15; end_to_end 9; project_roundtrip 23). Failing:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` (cells 0.690..1.986, as before). No test
  was added or removed (asserts were added to existing tests), so the counts equal w2's.
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok (`bin\wte_f1.exe`).
- Line endings of every changed file: `i/lf w/lf`.
- Sweep (`wrap_surface`, 13 models + noisy400, tiles300, parts200, trim_strip, final exe): exit codes as w2's: 14 ok,
  nonmanifold, noisy400 and tiles300 red at the ink/dup assert (unchanged numbers: dup 3.414 %, 5.124 %, ink 1.3286). Every
  step-10 stat equal to the checker's w2 table (param times aside); the first and the final exe give the same table
  and the same panel lines.
- Non-wrap identity: the checker's 15 scenarios (stamp, text, text_warp, paint3d, paint2d, export_run, export,
  roundtrip, brush_types, final_bake_async, multi, layers, stamp_edge_path, workflow_path, text_preview_path) with
  `wte_f1.exe` against the checker's `wte_r9c` runs: 15 / 15 ok, 46 files identical including every export file;
  differences of the known kinds only (`shot.json` frames, `export\shot.png` folder text 920 px,
  `final_bake_async\shot.png` toast, `.wtep` `manifest.json`).

## Open / not done

- WA20 red (cells), two attempts made by w2.
- nonmanifold step 10 red (dup 3.414 %, w2 item 4 side effect): an orchestrator decision, not in this pass's list.
- The step-10 coverage assert still uses `covered` (round 9's plan measure); `shown` would be stricter where layouts
  overlap. Not switched: it would change which models are red without a geometric reason being worked out.
- Area-push flips on the chess pieces and the owner-ray cost: attributed, not changed (sections 4 and 6).
