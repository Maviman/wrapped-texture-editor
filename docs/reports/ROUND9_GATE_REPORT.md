# Round 9 gate report — Wrapped Texture Editor ("wrap: once, nothing lost — then fast")

Date 2026-11-22. Branch `round9-wip` at `42996df` (= `round9-b`: package A + check:A + package B + check:B). Plan:
`docs/plans/ROUND9_PLAN.md` in the main tree (untracked there). Notes: `docs/notes/round9/pkg_a.md`, `check_a.md`,
`pkg_b.md`, `check_b.md`, `gate.md` (this gate's method, paths and raw numbers). Raw outputs:
`target\round9\gate\` (git-ignored). No product source was changed by the gate.

## Verdict

**Round 9 does not pass its gate.** The wrap is clearly better than round 8 where round 8 failed: a picture no longer
appears twice on the Avocado, the Lantern, the Klein bottle or a cube standing on a floor; on the torus knot it now
goes once round the tube instead of collapsing into a sliver; the bumpy ball gives the same picture at any texture
size; Project mode, the brush, the exports and old files are untouched. Heavy wraps are about twice as fast as round 8.
But five core tests stay red (three more than the plan allows), four sweep models miss a limit, a few new defects
showed up in the shots, the WaterBottle label is not exact again, and most speed lines miss their absolute limits.

- **Better than round 8.** Duplicates are gone from the sweep: Avocado (Jump on) dup 52.6 % -> 0.79 %, Lantern 26.9 % ->
  1.7 %, klein_bottle 4.1 % -> 0, nonmanifold 84 % -> 1.5 %, ABeautifulGame (Jump on) 10.5 % -> 2.7 %; the ink /
  dup / scatter limits now hold on all 13 models with both settings (round 8: 8 of 13). torus_knot shows the picture
  once round the tube over its full height (round 8: a dark sliver). high_poly_sphere has 0 seams and 0 rejected at
  256 px and at 2,048 px (round 8: 176 seams and 6.4 % at 256 px).
- **Unchanged.** Every non-wrap scenario's `shot.png`, every brush scenario and all exports are identical to round 6+7
  (`wte_r67.exe`); `check_exports.py` PASS; the Project hash of `wrap_surface` step 1 is equal. Projects saved by round
  6+7 and by round 8 open with identical pixels and are not re-wrapped until edited. No format change.
- **Not met: core tests.** `cargo test`: 1,170 passed, **5 failed**. The plan accepts only `wa4_r8` (60 deg) and `wa20`'s
  cells. Also red: `wa20`'s ink (1.189 > 1.15), `wa17` / `wa24` (ink 0.915: the picture partly beyond the floor's edge,
  a known decision for the user) and `wa35` (an exact-region case: the cylinder next to a dome is 92.4 % exact, not
  99.9 %).
- **Not met: sweep.** Lantern (lost_cut 11.2 % > 10 %, 30 seams > 6, bare slivers inside the faces), ABeautifulGame Jump
  off and many_islands (the new coverage assert fails because the picture is larger than the part it lands on),
  DamagedHelmet Jump on (ink 0.84 < 0.90). New in the shots: on ABeautifulGame with Jump on only 59 % of the picture's
  length is laid (the spine "passes itself" round the pawn under the anchor), and on nonmanifold the page facing the
  viewer is bare where the picture's middle should be. The WaterBottle label re-wraps 68 % exact on the cylinder body
  after a tint change (limit 99.9 %).
- **Speed: about 2 x round 8 on heavy wraps, but over the limits.** high_poly_sphere Final 5.6-6.5 s (limit 5 s; round 8
  11.1-13.0 s); ABeautifulGame Final Jump off 1.84-1.86 s (limit 1.0 s; round 8 3.4-3.5 s), Jump on 8.9-9.3 s (limit 6 s;
  round 8 16.9-17.2 s); the WA15 decal lines without Jump are met (board 14 ms, piece 142-144 ms, graph 0.7 s); Jump on
  costs 4-60 x the Jump-off time there (limit 1.5 x); text moves on the chess set 0.2-0.3 s (limit 150 ms or 1.5 x
  Project); on the sphere sample a wrapped Final is no faster than round 8 (41-71 ms; limit 40 ms). The time is now the
  relaxation solve (79-91 %) on large pictures and the owner grid (70-90 %) on small ones. The fuzz run and the memory
  line pass.
- **Measuring conditions.** The machine was **not idle**: the user's game ran during every measurement and took about 3
  of the 8 cores (median 36 %, `gate.md`). Round 8 was re-measured under the same load, alternating with round 9, so the
  ratios are comparable; the absolute times are pessimistic and should be re-measured on an idle machine before the
  user decides the speed lines.

## Exe and tests

- Exe `target\round9\gate\bin\wte_r9.exe` (release build of `42996df` in this worktree), 26,677,760 bytes, **SHA-256
  4F9E14222A17C9E1CF096FB7281822EC8E4E70E40DD6E9D8744CB999F0B358B2**. Every scenario, sweep and timing run used this copy.
  References: `wte_r67.exe` (main, rounds 6+7, 9F4F93CB...) and `wte_r8.exe` (round 8, DB61E168...), copied to the same
  folder.
- `cargo test --workspace --no-fail-fast`: **1,170 passed, 5 failed, 35 ignored** (wte_app 363 / 16; wte_core lib 760
  passed, 5 failed / 19; brush_tips 15; end_to_end 9; project_roundtrip 23). Round 8: 1,147 / 4 / 33. Log
  `target\round9\gate\logs\test1.log`.
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok. No `todo!` /
  `unimplemented!` in `crates\`.
- Line endings: the six files named for conversion (`uv_share.rs`, `wrap_mesh.rs`, the four round-9 notes) are already LF
  (0 CR bytes; Git Bash's grep misreports them), so nothing was converted and no commit was made for it. Instead, three
  existing files that are LF in `main` and `round8-wip` were converted to CRLF in round 9 (check:A, commits `ce7f656`,
  `591617d`): `wrap_param.rs`, `wrap_relax.rs`, `wrap_tests.rs`. Their diffs against round 8 therefore show every line
  changed (real change with `--ignore-cr-at-eol`: +1,058 / -114, +922 / -176, +1,090 / -9). Left as is (outside the
  gate's remit); see "Decisions".

## Acceptance lines

Two values = run 1 / run 2. "P" = Project, "W" = Wrap (`WTE_STAMP_MODE=wrap`). Sources under `target\round9\gate\`.

### Core tests (plan 5 / 6 / 7.1; `wte_core::wrap_tests`; `logs\test1.log`, `logs\wa_tests_nocapture.log`)

| # | Test | Limit | Measured | Verdict |
|---|---|---|---|---|
| WA1-WA3, WA5, WA16, WA18 | exact cases (round-6 limits) | 99.9 % within 2/255, max 8 | pass | **met** |
| WA4 30 / 45 deg | bumpy plate, 2 periods | ink 0.90-1.10; cells 0.75-1.30; dup <= 1 %; seams 0 | 30 deg ink 1.0203, cells 0.953..1.119; 45 deg ink 1.0414, cells 0.898..1.279; dup 0, seams 0 | **met** |
| WA4 60 deg | same | same | **ink 1.1177, cells 0.769..1.432**; smear 0.001 %, covered 1.0, dup 0, scatter 0, seams 0, flipped 0 (round 8: 1.136, 0.785..1.599) | **not met** (allowed red by (d); user decides) |
| WA4b | 4 periods, 60 deg | ink 0.85-1.15; cells 0.65-1.50 | ink 1.1149, cells 0.799..1.481 | **met** |
| WA20 | bumpy ball, 40 % | ink 0.85-1.15; cells 0.70-1.40; dup <= 1 %; scatter <= 2 %; seams 0; covered >= 0.99 | **ink 1.1890, cells 0.665..2.886**; dup 0.976 %, scatter 0.564 %, seams 0, covered 1.0, flipped 15 (round 8: ink 1.637, dup 10.7 %) | **not met**: cells (allowed by (d)) **and ink** (not covered by (d)) |
| WA21 | closed lathe piece | share >= 0.95; dup <= 1 %; S1/S2 only; flipped <= 1 % | share 0.9966, dup 0.315 %, S1 130, flipped 0.044 % | **met** |
| WA22 | torus taller / larger | dup <= 1 %; scatter <= 3 % | taller dup 0.006 %, scatter 0.029 %; larger dup 0.026 %, scatter 0 (round 8: 2.27 %) | **met** |
| WA23 | cone tip; cube corner | one S4 cut; smear <= 0.5 % | cone: 1 cut, smear 0, covered 0.461 (geometric bound as round 8); cube: S4 1, smear 0, dup 0.926 % | **met** (cone coverage as round 8) |
| WA17 / WA24 | cube on floor, Jump on | ink 0.95-1.10, dup <= 1 % | **ink 0.9146 (0.915)**, dup 0, cube 13,561 texels; the picture's last 10 % lies beyond the floor's edge, so at most 0.90 can lie once (check:A section 2). WA24's grid half not reached | **not met** (user decision) |
| WA25 | owner inert where exact | 0 rejected | 0 in all 8 cases (WA1 x2, WA3, WA5, WA16 x2, WA18 x2), relaxed false | **met** |
| WA26 / WA27 | determinism, reuse = fresh (incl. WA24, WA31, WA32, WA35) | byte-identical | pass | **met** |
| WA28 | growth: handed <= 1.25 x (laid + halo); graph path = sphere path | as stated | 2 handed / 2 laid + halo; every comparison 0 differ (127 printed here; check:B: also all 26 sweep placements) | **met** |
| WA29, WA30 | Preview / param jobs; old tests | pass | pass | **met** |
| WA31 | owner vs floor density 2 / 128 / 8,192 | dup <= 1 %, ink 0.95-1.10, inks within 2 % | ink 1.0142 / 1.0187 / 1.0152, dup 0, cube 23,001-23,021 texels | **met** |
| WA32 | helix (cylinder sample, 64-row cylinder, thin torus) | passed; ends within 5 % of analytic; covered >= 0.9 x ends; dup <= 1 %; scatter <= 3 % | ends 0.6183 / 0.6187 (analytic 0.6196), 0.5588 (0.5715); covered 0.664 / 0.790 / 0.720; dup 0; Overlap turns 3 | **met** |
| WA33 | rivets; cube corner | seams 0, covered >= 0.99, ink 0.95-1.10, dup 0; one S4 | plate seams 0, covered 1.0, ink 1.0142, dup 0; corner S4 1 | **met** |
| WA34 | saddle corner | dup <= 1 %, scatter <= 3 % | dup 0.132 %, scatter 0.073 %, lost_cut 0 | **met** |
| WA35 | exact region | cylinder / plate texels 99.9 % within 2/255, max 8; bump relaxed, seams 0, scatter <= 2 %; cone falls back | **capped cylinder side 92.37 %, max 51** (strip pins only: 82.02 %, max 255); bump plate 100 %, max 1, seams 0; cone fell back (bit-equal) | **not met** (capped cylinder) |
| WA36 | 256 vs 2,048 texture | layers equal; stats equal except time | equal (bumpy ball and sphere sample) | **met** |
| WA37 | covered >= 0.95 x ends; lost_cut <= 2 % | as stated | all cases met (lost_cut 0 %) | **met** |
| WA38 / WA39 | graph once per mesh; uv_share 0.5 +- 0.02 | as stated | pass | **met** |
| app (B) | Final-wait rule, warm-up races, T9 line | pass | 363 app tests pass | **met** |
| clippy / todo / release | | 0 / 0 / ok | 0 / 0 / ok | **met** |
| 7.1 count | >= 1,147 + new, 0 failed except (d) | | 1,170 passed; 5 failed (3 beyond (d)) | **not met** |

### Identity and compatibility (plan 7.1; `gate.md` sections 2-3)

| Line | Measured | Verdict |
|---|---|---|
| All 39 scenarios ok at 1280 x 800 | 39 / 39 ok (r9 and r67), `logs\scen_r9.txt` | **met** |
| Non-wrap `shot.png` identical to `wte_r67.exe` | 38 of 38 identical except `export` (folder text in the dialog, 870 px, known exception). Named extra shot `brush_shape\shot_dialog.png`: path text and a toast fade only. `final_bake_async`, `workflow_path`: 0 px this time | **met** |
| Every brush scenario identical | all identical (`paint3d`, `paint2d`, `brush_*`, `pan2d`, `fuzz_path`) | **met** |
| `export_run` exports byte-identical, `check_exports.py` | 7 / 7 files identical; PASS on both | **met** |
| `wrap_surface` step 1 Project hash | `cd272dc06ef4e935` in both | **met** |
| `WTE_SYNC_FINAL_BAKES=1` composites equal (`wrap_surface`, `final_bake_async`) | equal lists, identical shots; `final_bake_async` differs in the toast box only | **met** |
| FORMAT_VERSION 3, no new manifest key | 3; key set of an r9 save = r67 save = r8 save | **met** |
| r67- and r8-saved wrapped projects open identical, no `wrap:` line until edited | 4 projects (sphere and WaterBottle from each exe) open with 0 px difference to their saving exe; no wrap line | **met** |
| WaterBottle: after a tint change of a round-6 project, label texels on the cylinder body 99.9 % within 2/255 | logo: **68.14 %** of 9,187 body texels (max 255); shoulder 72.90 % of 30,175; all 71.79 %. Text "WTE": 53.26 % body. (Round-8 project: logo 83.34 %.) Differences along every colour edge: the picture moved by about a texel. `logs\compat_tint.txt`, `compat\dump\` | **not met** |

### Sweep (plan 7.2; `sweep_r9\`, `logs\sweep_r9_table.txt`)

`wrap_surface` with each model as PATH, step 10 (the `cells` picture at 40 % in Wrap, Jump small gaps off then on).
Round-8 values from today's round-8 sweep (`ref_shots\r8\<model>\shot.log`) in brackets. All quality numbers are
identical to check:B's final sweep. Times were measured under the game's load (see 7.3 for the timing runs).

| Model | Jump | verts / tris | relaxed | seams [S1..S5] | stretch | rejected | covered | ends_share | lost_cut | ink (r8) | dup (r8) | scatter (r8) | jumps | param / applied ms (r8) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| WaterBottle | off | 225 / 482 | yes | 0 [0,0,0,0,0] | 3.8 % | 0.00 % | 1.000 | 1.000 | 0.00 % | 1.0033 (1.0017) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 18 / 41 (22 / 49) |
| WaterBottle | on | 225 / 482 | yes | 0 [0,0,0,0,0] | 3.8 % | 0.00 % | 1.000 | 1.000 | 0.00 % | 1.0033 (1.0017) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 21 / 35 (23 / 41) |
| Avocado | off | 79 / 136 | yes | 0 [0,0,0,0,0] | 13.4 % | 0.00 % | 0.545 | 1.000 | 0.00 % | 0.5415 (0.5288) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 14 / 40 (10 / 40) |
| Avocado | on | 115 / 235 | yes | 0 [0,0,0,0,0] | 13.3 % | 34.79 % | 1.000 | 1.000 | 0.00 % | 0.9976 (1.5295) | 0.787 % (52.591 %) | 0.502 % (5.865 %) | 1 | 30 / 55 (21 / 49) |
| DamagedHelmet | off | 233 / 457 | yes | 11 [0,0,5,0,6] | 24.2 % | 0.26 % | 0.660 | 1.000 | 0.77 % | 0.5094 (0.5065) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 19 / 76 (24 / 81) |
| DamagedHelmet | on | 1,060 / 1,956 | yes | 69 [0,0,5,10,54] | 51.1 % | 16.95 % | 1.000 | 1.000 | 2.61 % | 0.8424 (1.0941) | 0.036 % (0.000 %) | 0.000 % (0.000 %) | 9 | 66 / 101 (60 / 98) |
| Lantern | off | 52 / 100 | yes | 30 [0,0,0,11,19] | 31.2 % | 16.19 % | 0.885 | 1.000 | 11.23 % | 0.7704 (0.8273) | 1.676 % (26.860 %) | 1.854 % (10.578 %) | 0 | 16 / 60 (13 / 54) |
| Lantern | on | 52 / 100 | yes | 30 [0,0,0,11,19] | 31.2 % | 16.19 % | 0.885 | 1.000 | 11.23 % | 0.7704 (0.8273) | 1.676 % (26.860 %) | 1.854 % (10.578 %) | 0 | 16 / 35 (14 / 35) |
| ABeautifulGame | off | 14,210 / 28,416 | yes | 353 [334,46,0,0,0] | 85.5 % | 1.13 % | 0.071 | 0.331 (passed) | 0.00 % | 0.0639 (0.0591) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 618 / 1,538 (3,443 / 3,541) |
| ABeautifulGame | on | 160,593 / 323,172 | yes | 2,172 [0,864,44,3,1261] | 98.1 % | 45.65 % | 1.000 | 0.588 (passed) | 0.02 % | 0.8463 (1.3045) | 2.674 % (10.505 %) | 2.631 % (1.130 %) | 30 | 6,299 / 7,072 (20,342 / 22,684) |
| ngons_tiling | off | 11 / 9 | no | 0 [0,0,0,0,0] | 0.0 % | 0.00 % | 0.074 | 1.000 | 0.00 % | 0.0061 (0.0061) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 12 / 45 (3 / 13) |
| ngons_tiling | on | 11 / 9 | no | 0 [0,0,0,0,0] | 0.0 % | 0.00 % | 0.074 | 1.000 | 0.00 % | 0.0061 (0.0061) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 11 / 50 (4 / 25) |
| torus_knot | off | 998 / 2,052 | yes | 48 [48,0,0,0,0] | 15.6 % | 0.08 % | 0.440 | 0.430 (passed) | 0.00 % | 0.4545 (0.4051) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 50 / 61 (78 / 88) |
| torus_knot | on | 998 / 2,052 | yes | 48 [48,0,0,0,0] | 15.6 % | 0.08 % | 0.440 | 0.430 (passed) | 0.00 % | 0.4545 (0.4051) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 57 / 69 (85 / 96) |
| mobius | off | 232 / 434 | yes | 0 [0,0,0,0,0] | 1.4 % | 0.00 % | 0.570 | 1.000 | 0.00 % | 0.5702 (0.5706) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 111 / 133 (12 / 19) |
| mobius | on | 232 / 434 | yes | 0 [0,0,0,0,0] | 1.4 % | 0.00 % | 0.570 | 1.000 | 0.00 % | 0.5702 (0.5706) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 134 / 165 (13 / 27) |
| klein_bottle | off | 1,820 / 3,911 | yes | 0 [0,0,0,0,0] | 23.7 % | 0.00 % | 1.000 | 1.000 | 0.00 % | 1.0132 (1.1050) | 0.000 % (4.132 %) | 0.000 % (0.053 %) | 0 | 493 / 502 (212 / 221) |
| klein_bottle | on | 1,820 / 3,911 | yes | 0 [0,0,0,0,0] | 23.7 % | 0.00 % | 1.000 | 1.000 | 0.00 % | 1.0132 (1.1050) | 0.000 % (4.132 %) | 0.000 % (0.053 %) | 0 | 396 / 458 (225 / 234) |
| nonmanifold | off | 16 / 14 | no | 0 [0,0,0,0,0] | 0.0 % | 70.08 % | 0.630 | 1.000 | 0.00 % | 0.1873 (0.6292) | 1.523 % (84.186 %) | 0.539 % (1.672 %) | 0 | 136 / 193 (7 / 15) |
| nonmanifold | on | 16 / 14 | no | 0 [0,0,0,0,0] | 0.0 % | 70.08 % | 0.630 | 1.000 | 0.00 % | 0.1873 (0.6292) | 1.523 % (84.186 %) | 0.539 % (1.672 %) | 0 | 238 / 310 (7 / 20) |
| many_islands | off | 8 / 12 | yes | 8 [2,0,0,6,0] | 100.0 % | 27.48 % | 0.018 | 0.184 | 0.00 % | 0.0123 (0.0148) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 28 / 42 (8 / 26) |
| many_islands | on | 24 / 36 | yes | 15 [0,0,0,13,2] | 98.1 % | 6.25 % | 0.033 | 0.332 | 1.48 % | 0.0313 (0.0345) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 2 | 23 / 49 (11 / 25) |
| flat_annulus | off | 22 / 27 | no | 0 [0,0,0,0,0] | 0.0 % | 0.00 % | 0.600 | 1.000 | 0.00 % | 0.5972 (0.5972) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 12 / 26 (16 / 141) |
| flat_annulus | on | 22 / 27 | no | 0 [0,0,0,0,0] | 0.0 % | 0.00 % | 0.600 | 1.000 | 0.00 % | 0.5972 (0.5972) | 0.000 % (0.000 %) | 0.000 % (0.000 %) | 0 | 13 / 26 (8 / 18) |
| high_poly_sphere | off | 77,721 / 156,680 | yes | 0 [0,0,0,0,0] | 33.8 % | 0.00 % | 1.000 | 1.000 | 0.00 % | 1.0977 (1.0427) | 0.000 % (0.000 %) | 0.000 % (0.103 %) | 0 | 4,329 / 4,848 (11,041 / 11,105) |
| high_poly_sphere | on | 77,721 / 156,680 | yes | 0 [0,0,0,0,0] | 33.8 % | 0.00 % | 1.000 | 1.000 | 0.00 % | 1.0977 (1.0427) | 0.000 % (0.000 %) | 0.000 % (0.103 %) | 0 | 4,346 / 4,413 (9,894 / 9,961) |

Round 8's seams for comparison: DamagedHelmet 16 / 140, Lantern 28, ABeautifulGame 213 / 2,235, torus_knot 84,
high_poly_sphere 176 [0,49,0,0,127] with 6.40 % rejected; round 6+7 ink (Jump off / on): WaterBottle 0.9992, Avocado
0.5184 / 1.9379, DamagedHelmet 1.0185 / 5.5631, Lantern 1.1828, ABeautifulGame 0.0641 / 5.2357, torus_knot 1.8677 /
2.3549, mobius 0.5731, klein_bottle 9.1089 / 9.8622, nonmanifold 0.6292, many_islands 0.0148 / 0.0345, flat_annulus
0.5965, high_poly_sphere 6.6264 / 8.0510, ngons_tiling 0.0061.

high_poly_sphere with a 2,048 x 2,048 checker (`sweep_r9_2048\`): 77,721 / 156,680, seams 0, rejected 0.00 %, ink 1.1024,
dup 0, scatter 0, stretch 33.8 %, param 4,663 / 4,723 ms.

**Scenario asserts.** "ink <= 1.3, dup <= 3 %, scatter <= 5 %": met on all 13 models, both settings. "covered >= 0.85 x
ends_share unless reached_edge, lost_cut <= 10 %": **red on 3 models**: Lantern (lost_cut 11.23 %, both settings),
ABeautifulGame Jump off (covered 0.0714 < 0.85 x 0.3314), many_islands (0.0177 < 0.85 x 0.1843; 0.0333 < 0.85 x
0.3318). One more run failed at step 6b on high_poly_sphere's first run (a timing-dependent Preview-job assert under
the game's load; the rerun and three other runs passed; `gate.md` section 4).

**Per-model lines (plan 7.2 table)** — numbers and the shot reading (readings below):

| Model | Must hold | Measured | Shot must show | Shot | Verdict |
|---|---|---|---|---|---|
| WaterBottle | ink 0.95-1.05; tint check 99.9 % on the body | ink 1.0033; body 68.14 % | one whole 8 x 4 picture | yes | **not met** (tint check) |
| Avocado | off ink >= 0.50; on ink 0.90-1.15, dup <= 3 % | 0.5415; 0.9976, dup 0.787 % | no cell on both the cut face and the seed | yes, except possibly a sliver of one cell (light blue) | **met** (with the noted sliver) |
| DamagedHelmet | off lost_cut <= 10 %, covered explained; on ink 0.90-1.3 | 0.77 %; covered 0.660 (about 0.33 lies beyond the visor part's edge, Jump off: package A's A3); on **ink 0.8424** | visor continuous; mirrored repeats named; panel line | visor centre continuous; panel line present (shared texels 100 %) | **not met** (ink on) |
| Lantern | dup <= 3 %, scatter <= 5 %, seams <= 6, stretch <= 60 % | 1.676 %, 1.854 %, **seams 30**, 31.2 %; **lost_cut 11.23 %** | faces continuous across the box edges; cuts only at box corners | faces continuous; **bare slivers inside two face cells**; plinth and cap mostly bare | **not met** |
| ABeautifulGame | on: board rows continuous, ink <= 1.3, dup <= 3 %; pieces as answer (a) | ink 0.8463, dup 2.674 %; ends_share 0.588 | panel line for shared texels | board rows continuous but only columns 4-8 on the board; panel lines present (shared 20.1 %, "59 % shown") | numbers **met**; Jump-off coverage assert red; 41 % of the picture cut (new finding) |
| torus_knot | passed or met_itself; covered and ink >= 0.85 x ends | passed; covered 0.440, ink 0.4545 >= 0.366 | once round the tube over its full height, ends meeting in one line; panel says how much | yes (the meeting line is on the hidden back); panel "43 % shown" | **met** |
| klein_bottle | dup <= 3 % | 0 % | one piece on the visible side | yes | **met** |
| nonmanifold | dup <= 3 % | 1.523 % | one piece on the visible side | **no: two pieces, the page between them bare** | **not met** (shot) |
| high_poly_sphere | ink 0.95-1.15, seams 0, rejected <= 1 % at 256 and 2,048 px; same cells | 1.0977 / 1.1024; 0 / 0; 0 / 0 %; same cells | one whole picture | yes, both | **met** |
| mobius, many_islands, flat_annulus, ngons_tiling | asserts; no change for the worse | ink 0.5702 (0.5706), 0.0123 / 0.0313 (0.0148 / 0.0345), 0.5972 (0.5972), 0.0061 (0.0061) | | shots unchanged to the eye | many_islands step-10 assert **red**; the rest **met** |

### Screenshots: round 6+7 | round 8 | round 9 (the shot is the state after Jump on)

Sheets in `target\round9\gate\sheets\`: `<model>_view.png` (the viewport, half size), `<model>_3way_zoom.png` (3-4 x),
4 x singles `<model>_r9_zoom.png` / `_r8_zoom.png` / `_r67_zoom.png`, Jump off `<model>_r9off_zoom.png`, closer crops
named below. Round 6+7 and round 8 shots: `ref_shots\r67|r8\<model>\shot.png`; round 9: `sweep_r9\<model>\shot.png`
(`shot_off.png` = Jump off). Every round-9 crop below was read at 4-12 x. The 8 x 4 picture's rows (top to bottom,
left to right): 1 black, light blue, orange, cyan, olive, navy, green, pink; 2 purple, grey, teal, mint, tan, violet,
light green, blue; 3 red, light pink, dark olive, pale cyan, green, magenta, yellow, slate; 4 crimson, teal, rosy brown,
pale yellow, sea green, pink-magenta, spring green, lilac.

| Model | Round 6+7 | Round 8 | Round 9 |
|---|---|---|---|
| high_poly_sphere (`high_poly_sphere_r9_zoom.png`, `_2048_zoom.png`) | torn scraps of colour over the whole ball | one whole picture, cells bent over the bumps | **one whole picture**: all 32 cells once, in order, rows continuous, each cell bent over its bumps, wavy outline, no cut, no bare gap. The top-left cells (black, light blue) are slightly larger than round 8's. The 2,048 px shot shows the same cells in the same places with the same outline, sharper |
| torus_knot (`torus_knot_3way_zoom.png`) | colour smeared along a long stretch | a dark sliver squeezed round the tube | **the picture once round the tube over its full height**: down the tube the visible face shows column 5's four rows in order (olive, tan, green, sea green), column 4 to its left (cyan, mint, pale cyan), a sliver of column 6 (pink-magenta) at the right edge; the outline's top and bottom edges cross the tube; the ends meet on the hidden back. No repeat, no bare gap on the visible side. Panel: "Longer than the way round: 43 % shown" and also "56 % of the picture runs off the surface: an edge, a hole or the end of the part" (the second names a wrong cause here) |
| Avocado (`Avocado_r9_zoom.png`, `Avocado_r9_topleft.png` 10 x) | strips sticking out to the left, repeats | the left columns twice (seed and flesh) | **one picture**: the seed carries columns 2-5 (light blue, orange, cyan; grey, teal, mint; light pink, dark olive, pale cyan, green; teal, rosy brown, pale yellow, sea green), column 1 only as slivers on the seed's left edge (purple, red, crimson); the flesh right of the seed carries columns 5-8 in four continuous rows. The black cell lies only on the flesh, top left. The light-blue cell shows as a thin wedge on the flesh above the seed and as a patch on the seed's upper left, with bare flesh between: either the cell split over the seed's rim or a sliver twice (dup 0.787 % is about a quarter of a cell). The flesh left of the seed is outside the outline (bare); most of column 1 went to the seed's hidden side |
| DamagedHelmet (`DamagedHelmet_r9_zoom.png`, `_r9off_zoom.png`) | shredded triangles over the whole helmet | visor mostly continuous; a fold on the left; scraps on 9 side parts | **visor centre continuous** (columns 3-6, all rows, in order). The visor's left shows the black, purple and red cells twice in a tilted pattern, as in round 8, and cells of row 1 appear on the shell above the outline: the model's mirrored UVs (panel line "This model uses the same texture area in several places ..." present; shared texels 100 %). Right of the visor: a navy part, the violet / light-green / pink / blue / slate cells on the visor's right, a yellow knob, green and magenta strips on the lower tubes, and dark bare patches between them inside the outline (ink 0.84). Fewer scraps than round 8. Jump off: the visor carries columns 1-6, the picture stops at the visor part's edge |
| ABeautifulGame (`ABeautifulGame_r9_zoom.png`, `_r9_board.png` 7 x, `_r9off_zoom.png`) | smeared over the board and every piece | most of the picture on the board in rows; blobs on pawns, a bishop, piece bases | **the board inside the outline shows only columns 4-8** (pale yellow / sea green / pink-magenta / spring green / lilac row, the green / yellow / slate row, tan / violet / light-green / blue, navy / green / pink), rows continuous and in order; the outline is narrower than round 8's and the frame's left part is bare board. All eight white pawns carry the same magenta bodies and lime heads, both knights a cyan mane, both bishops an orange band: instanced meshes sharing texels (panel line present, 20.1 %). The rest of the picture lies round the pawn under the anchor ("59 % shown") |
| Lantern (`Lantern_3way_zoom.png`, `Lantern_r9_wedge.png` 12 x) | patches on plinth and ledge | front faces continuous; smeared cells on the plinth (dup 26.9 %) | front face (teal, mint / dark olive, pale cyan / rosy brown, pale yellow) and right face (tan, violet / green, magenta / sea green, pink-magenta) **continuous across the box edge**, rows level. **Two thin dark bare slivers inside faces** (through the pale-cyan cell on the front, through the green cell on the right): cuts not at a corner. The plinth's right half and most of the cap are bare; a teal and a rosy patch on the plinth's front left, thin cyan / orange / green strips on the cap's rim. No repeated cell |
| WaterBottle (`WaterBottle_r9_zoom.png`) | one whole picture | the same | **one whole 8 x 4 picture** on the body, rows continuous, columns narrowing evenly to the sides, no cut, no repeat; differs from round 8's shot by 1,052 px (> 2 levels) in a thin band at the top (y 329-362) |
| klein_bottle (`klein_bottle_r9_zoom.png`) | pieces repeated all over | one piece on the visible side | **one piece on the visible side**: columns 1-3 on the outer tube with continuous rows, columns 4-7 curving into the ring's hole, stretched; row 1 of columns 4-8 and most of column 8 out of view behind the rim; no repeat, no bare gap |
| nonmanifold (`nonmanifold_3way_zoom.png`) | cells on several pages | the same cells on several pages (dup 84 %) | **two pieces with a bare page between**: the left front page carries light pink, dark olive, teal, rosy brown; the back right page mint, tan, green, sea green; the page facing the viewer between them shows the model's blue and white stripes where round 8 had pale cyan and pale yellow; the triangle to the right is bare (round 8: cells through overlapping UVs). Nothing twice, but a gap in the middle (ink 0.629 -> 0.187, rejected 70 %) |

## Speed (plan 7.3; `timing1\`, measured with the user's game running)

Each line: the absolute limit **and** <= 0.5 x round 8 here. Round 8 re-measured first in each pair, alternating.

| # | Line | Round 9 | Round 8 (here) | Ratio | Limit | Verdict | Where the time goes (round 9) |
|---|---|---|---|---|---|---|---|
| T-a | high_poly_sphere Final applied (Jump off; on) | 6,529 / 6,122 ms; 6,257 / 5,626 ms | 11,111 / 12,649; 11,278 / 13,013 | 0.59 / 0.48; 0.55 / 0.43 | <= 5 s | **not met** | param 5.5-6.2 s: relax 5.0-5.6 s (88-91 %), front 0.26 s, collect 0.09-0.14 s, owner 0.05 s; the first Final also waits 0.47-0.57 s for the graph. `sweep_r9_hps_*` |
| T-b | ABeautifulGame Final, Jump off | applied 1,844 / 1,864 ms (param 956 / 940) | 3,361 / 3,492 | 0.55 / 0.53 | <= 1.0 s | **not met** (param alone under 1.0 s; applied includes 0.76-0.80 s graph build inside the first Final) | relax 0.86-0.87 s (91 % of param), front 24 ms, owner 26-32 ms. `sweep_r9_abg_*` |
| T-b | ABeautifulGame Final, Jump on | 9,326 / 8,892 ms (param 8,380 / 8,008) | 16,873 / 17,165 | 0.55 / 0.52 | <= 6 s | **not met** | relax 6.3-6.7 s (79-80 %), front 0.83-0.86 s, collect 0.29 s, owner 0.23-0.27 s |
| T-c | WA15 20 % decal param, board / piece; graph build | board 13.8 / 14.6 ms; piece 144.0 / 141.6 ms; graph 728 / 706 ms | board 332 / 300; piece 988 / 1,047 | 0.04 / 0.05; 0.15 / 0.14 | <= 60 / 150 ms; graph <= 1.0 s | **met** | board: owner 95 %; piece: relax 83 %, owner 12 %. `wa15_r9_*.txt`, `wa15_table.txt` |
| T-d | WA15 Jump on vs off | board 836 / 855 ms (limit max(1.5 x 14, 150) = 150); piece 514 / 564 ms (limit 216 / 212) | board 3,052 / 2,567; piece 2,159 / 1,731 | 0.27 / 0.33; 0.24 / 0.33 | <= max(1.5 x off, 150 ms) | **not met** (5.6 x and 2.4-2.7 x the limit; ratio met) | board: relax 545 ms (70 %), collect 127 (16 %), front 63; piece: relax 260 (54 %), collect 113 (24 %), owner 75 (16 %) |
| T-e | ABeautifulGame `workflow_path`, W vs P: image | 1,443 / 1,531 vs P 1,388 / 1,611 ms (1.04 / 0.95 x) | W 2,440 / 2,644 | 0.59 / 0.58 | <= 1.5 x P or 150 ms | absolute **met**, ratio **not met** | the first wrapped Final waits 621 ms for the graph (the warm-up started 30 frames earlier); relax 624 ms |
| T-e | text placed | 71 / 67 vs 31 / 26 ms | 237 / 267 | 0.30 / 0.25 | same | **met** | |
| T-e | move | 212 / 197 vs 70 / 69 ms (3.0 / 2.9 x) | 540 / 597 | 0.39 / 0.33 | same | **not met** (absolute) | param 191 ms: relax 157 ms (82 %), owner 24 ms |
| T-e | scale | 309 / 259 vs 136 / 116 ms (2.3 / 2.2 x) | 847 / 909 | 0.36 / 0.28 | same | **not met** (absolute) | relax 230 ms of the param |
| T-e | rotate | 233 / 217 vs 120 / 110 ms (1.9 / 2.0 x) | 971 / 1,037 | 0.24 / 0.21 | same | **not met** (absolute) | |
| T-e | drag frame max; frames > 300 ms during Finals | W 10.1 / 7.2 ms vs P 50.3 / 55.7 ms; 0 | W 8.0 / 8.0 ms | | <= P + max(30 ms, 25 %); 0 | **met** | the 3-4 frames > 300 ms per run are model load and the final reopen, in both modes and exes |
| T-f | sphere sample `stamp`: W Final vs P (image, move, scale, rotate, wheel) | run 2: 41, 46, 53, 71, 12 ms (run 1: 102, 54, 68, 127, 15) vs P 11, 8, 12, 13, 12 (9, 53, 35, 15, 23) | W run 2: 38, 41, 55, 83, 13 (run 1: 58, 43, 60, 75, 13) | run 2: 1.08, 1.12, 0.96, 0.86, 0.92 | <= 1.5 x P or <= 40 ms | **not met** (only the wheel Final meets it; no faster than round 8) | the owner grid: 28-68 ms of 31-58 ms parameterizations (70-90 %) |
| T-g | fuzz seed 7, 1,000 steps, ngons_tiling, W | 0 errors; 0 frames > 500 ms (0 > 300 ms) of 11,048 / 11,193; 165.7 / 214.3 s; 33 / 30 wrap Finals | not run (absolute limit) | | 0 errors, <= 2 frames > 500 ms | **met** | |
| T-h | `workflow_path` peak working set, ABeautifulGame | W 2,651 / 2,653 MB vs P 2,549 / 2,550 MB (+102 / +103 MB); private W 3,352 / 3,400 vs P 3,268 / 3,250 MB (+84 / +150 MB) | W 2,530 / 2,531 MB | not meaningful for memory | W <= P + 300 MB | **met** | the graph is 105 MB |

Speed lines met: T-c, T-e (text, drag frames, no long frames), T-g, T-h. Not met: T-a, T-b (both), T-d, T-e (image ratio;
move, scale, rotate absolute), T-f.

## Behaviour changes in round 9

1. **A picture point lies on one surface place** (place owner). Hidden second copies are dropped: under Avocado's
   seed, on the Lantern's plinth, inside the Klein bottle, under a cube on a floor. With Jump small gaps a part standing
   in the picture takes its share (answer (a)-1). Where the owner picks a place the viewer cannot see, the visible spot
   stays bare: DamagedHelmet's lower right (ink 1.09 -> 0.84), nonmanifold's front page (ink 0.63 -> 0.19), the flesh
   left of Avocado's seed.
2. **Ends meet on tubes and helices** (`passed`): torus_knot shows the picture once round the tube. The panel says
   "Longer than the way round: P % shown". It also fires round a chess pawn with Jump on, cutting the picture to 59 %
   although the board has room.
3. **Cuts at real corners only** (net curvature): fewer cut lines on bevelled and riveted parts (helmet S4 4 -> 0 with
   Jump off); a box corner still cuts.
4. **Relaxation starts from the surface's own coordinates (no LSCM), 12 ARAP steps**: bumps fold less (WA20 dup 10.7 %
   -> 0.98 %), but the steepest test bumps still exceed their area limits.
5. **Exact parts pinned** where the seed is isometric: flat plates beside a bump stay exact; the WaterBottle body is not
   exact again (a round-6 label re-wraps 68 % within 2/255 on the body after a tint change).
6. **Same picture at any texture size**: high_poly_sphere at 256 px and 2,048 px gives the same layout (0 seams).
7. **Speed**: a welded graph per mesh (built once, 0.5-0.8 s on the chess set and the bumpy ball, on a background thread
   when Wrap is chosen or a 3D stamp is selected); growth from the anchor; a Final no longer waits for a drag Preview made
   for another placement. Heavy wraps about 2 x faster than round 8; small wraps not faster (the owner grid's fixed cost).
8. **New panel lines**: "This model uses the same texture area in several places (mirrored or repeated parts): the
   picture also shows there." (DamagedHelmet 100 %, ABeautifulGame 20.1 %, ngons_tiling 100 %); "Longer than the way
   round: P % shown" (torus_knot 43 %, ABeautifulGame 59 %, many_islands 33 %).
9. **Un-relaxed layouts (plan D7)** compared with round 6+7: flat_annulus ink 0.5965 -> 0.5972, mobius 0.5731 -> 0.5702,
   ngons_tiling 0.0061 unchanged; nonmanifold 0.6292 -> 0.1873 (owner, item 1). The exact tests (WA1-WA3, WA5, WA16,
   WA18) stay within their limits; the gate did not recount WA5's corner cuts (plan D7: 1 in round 6, 2 in round 8).
10. **Log**: `wrap:` lines add `passed`, `ends`, `lost_cut`; new `wrap phases:` line (graph, collect, front, relax, owner
    ms), `wrap mesh: graph built ...`, `shared texels found ...`, `warm-up started / ended`; step 10 of `wrap_surface`
    adds `coverage:`, `placement bits:`, `panel lines:` and `shot_off.png`.
11. **Old files**: round-6 and round-8 wrapped stamps keep their saved pixels until edited (unchanged rule); at the first
    edit they re-wrap with round 9's layout (WaterBottle logo: 72 % / 83 % of texels within 2/255 of the saved ones).

## What was not met, and why

1. **Core tests**: `wa4_r8` 60 deg and `wa20` (ink and cells): with the centre line exact, an as-rigid-as-possible
   sticker on the steepest bumps takes up more area than its own (plan D4b); the area-term solver was not in this
   round. `wa17` / `wa24`: the test's picture runs 0.2 beyond the floor, so ink 0.95 needs a second copy. `wa35`: the
   front's seed itself is not isometric on the cylinder rows next to the dome (check:A section 5).
2. **Sweep**: Lantern (coarse 52-vertex box: corner and tree-cotree cuts, cut slivers in the faces, lost_cut 11.2 %);
   ABeautifulGame Jump off and many_islands (the picture is larger than the closed part across; the new assert's
   exemption covers only `reached_edge`); DamagedHelmet Jump on (owner sends picture to hidden parts: ink 0.84);
   nonmanifold shot (bare front page); ABeautifulGame Jump on cut to 59 % (spine passes round the pawn).
3. **WaterBottle tint check**: the label lies mostly on the shoulder; the exact region does not keep the body texels
   (68.14 %).
4. **Speed**: the relaxation solve dominates large pictures (79-91 %), the owner grid small ones (70-95 %); Jump on adds
   the solve of 25k-160k laid vertices. Every speed change in round 9 kept every bit (check:B); a faster solve or a
   smaller owner grid changes results and needs a quality decision. Measured with the user's game taking ~3 of 8 cores.

## Decisions for the user

Known before the gate (reported, not resolved):
1. WA17 / WA24 ink limit 0.95 vs the geometric 0.90 (move the test's picture or lower the limit).
2. The step-10 coverage assert on ABeautifulGame Jump off and many_islands (part smaller than the picture across):
   extend the exemption ("unless reached_edge or the halves meet") or keep it red.
3. Lantern lost_cut 11.2 % vs 10 % (and seams 30 vs 6).
4. WA4 60 deg and WA20 red (answer (d)); also WA20's ink (1.189 > 1.15) and WA35 red.
5. Meeting T-a / T-b needs a faster solve that changes results (fewer ARAP steps, another preconditioner, a smaller owner
   grid: check:B's list).

New from this gate:
6. ABeautifulGame with Jump on: the picture is cut to 59 % because the spine passes itself round the pawn under the
   anchor. Intended ("the piece takes its share", the picture goes round the pawn) or should the board take the rest?
7. nonmanifold: the visible front page is bare where the middle of the picture should be (ink 0.19). Acceptable for a
   non-manifold model?
8. WaterBottle: the round-6 label is 68 % exact on the body after a tint change (limit 99.9 %).
9. Line endings: `wrap_param.rs`, `wrap_relax.rs`, `wrap_tests.rs` were turned into CRLF in round 9; convert them back
   to LF before landing (the history would then show the real diff).
10. The speed lines should be re-measured on an idle machine before deciding them (the game ran during all timing).
11. The torus_knot panel shows "56 % of the picture runs off the surface: an edge, a hole or the end of the part" next
    to "Longer than the way round: 43 % shown"; the first line's cause is wrong for a tube.

## What to try by hand

1. **A text across the torus knot** (Wrap): it should go once round the tube and meet itself at the back; the panel
   says how much is shown.
2. **A picture over Avocado's seed** (Jump on): one picture; look at the top left, where the light-blue cell meets the
   seed's rim, and at the bare flesh left of the seed.
3. **Jump small gaps on across chess pieces** (ABeautifulGame): see how much of the picture reaches the board when it
   starts on a pawn (59 % here), and that every white pawn shows the same colours (instancing; the panel says so).
4. **The bumpy ball** (`high_poly_sphere`): a picture or text should lie whole; the release takes about 5-6 s.
5. **A round-6 WaterBottle project**: open it (unchanged), change the label's tint and back: the label moves by about a
   texel.
6. **The Lantern's base**: look for thin bare slivers inside the faces and the bare plinth.
7. **A drag on ABeautifulGame** (wrapped text): frames stay smooth (7-10 ms here); the release takes 0.2-0.3 s.
