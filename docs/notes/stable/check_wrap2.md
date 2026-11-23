# Stable: independent check of fix-wrap pass 2 (`stable-fix-wrap` @ 251ebcb)

Checker (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`. Inputs: `critic.md`,
`STABLE_GATE_REPORT.md`, `check_exp_2.md`, the builder's `fix_wrap_2.md`, and `git diff stable..HEAD` (8 files: wrap
core, panel lines, the `wrap_surface` scenario, tests, ARCHITECTURE, notes). Raw outputs (git-ignored) are under
`target\stable\checkwrap2\`: `test.log`, `fix_tests.log`, `perf.log`, `clippy.log`, `build.log`, `bin\wte_cw2.exe`,
`sweep\` + `table_cw2.txt` (17 models), `table_gate.txt` (the gate's sweep re-parsed), `flow\` (export_flow),
`vzip.txt`, `det\`, `timing\` + `timing.txt`, `flush.log`, `crops\`, my scripts in `tools\` (`vzip.py` is my own).
I changed no code. The only commit is this file.

## Verdict

**I could not refute the done claims.** The new panel lines match the counts and what the shots show, the wrap
metrics equal the gate's for all 17 models, the export workflow still works (also on ABeautifulGame, which the critic
said never ran on the merged build), and the run is deterministic. There is **no major defect**. The minor points
are listed below. None of them blocks the user's definition of done.

## Build, tests, clippy

- `cargo test --workspace --no-fail-fast`: wte_app 372 / 0 / 17 ignored; wte_core lib 792 passed / **1 failed
  (wa20, `cells 0.689751..1.986138`, the known red, unchanged)** / 23 ignored; brush_tips 15, end_to_end 9,
  project_roundtrip 23. **1,211 passed, 1 failed, 40 ignored**, as the builder says. The count adds up: the gate's
  1,208 plus 3 new passing tests. The 3 new ignored tests are 2 perf tests and the known-limit test.
- The new tests with `--nocapture`: `fix_bare_cells_by_cause` (gaps 0.0100, at a part 0.4250 against a target of
  0.4125 ± 0.02; with the neighbour test off, 0); helmet: stamp share 1.0000, independent 1.0000 (251 points), mesh
  share 0.8675; bottle: 31 body triangles, edge error mean 0.00002 and max 0.00014 texels.
- The serial perf tests (`cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`) passed 3 times out of 3.
  check:A slowest 104.4 / 90.6 / 88.8 ms; WA7 slowest **194.9** / 125.7 / 126.2 ms. Another agent's build, a fuzz
  and a python job were running at the same time, so this was not an idle machine.
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok. The exe SHA-256 is
  97A70242D70767483F84062BA6A1B52BA2CEAAE507065A675BA16815B42041C7, the same as the builder's `wte_fw7.exe`.
- All 8 changed files are `i/lf w/lf`. No change to `Cargo.toml` or `Cargo.lock`.

## The claims, one by one

1. **ABeautifulGame cut (not fixed, explained).** The ignored test `fix_jump_continues_across_flush_coplanar_plates`
   fails as stated: plate B 0.33588 within 2/255, `shown` 0.8336. Attempt 2 is not in the tree, so its sweep numbers
   (seams 2,147 -> 2,269) are single source; I could not re-run them. The cause analysis (a spine closed round the
   pawn, so the front's stop lands at the slab edge) is single source too. Its visible result I did confirm: in
   `sweep\ABeautifulGame\shot.png` at 8x (`crops\abg_left8.png`), the leftmost column is cut by a slanted line and the
   dark marble frame lies bare beyond it, as check_wrap_2 found.
2. **Panel lines.** My sweep with `wte_cw2.exe` gives exactly the builder's table (`table_fw7.txt`; only times
   differ). Without the panel lines, it is field for field the gate's `sweep_st` table. The panel lines changed only
   for parts200 ("28 % of the picture falls into the gaps between separate parts"), DamagedHelmet ("Only 97 %"
   instead of "3 % runs off") and ABeautifulGame ("9 % of the picture stops where it meets a neighbouring part").
   The three red models are now logged. I read the shots: parts200's panel shows the grey help line, and the picture
   on the quad field looks whole. tiles300 looks whole too. On ABeautifulGame the bare strip is on the frame, plus
   the notch round a pawn base, which is also a neighbouring part. Every share is the grid's count, and
   `1 - shown - beyond - lost_cut - gaps - at_part` is about 0 for parts200 and tiles300 and 0.1 % for
   DamagedHelmet. Code check: the owner index passed to `at_part` is the index into `stored` / `tris`, which map
   1:1 in `make_layer`. `laid`, `tv` and `part` are all patch-local. `turn_jumper` is not used after the `take()`.
3. **DamagedHelmet 100 % / 86.7 %.** Confirmed: these are two measures, and the new test agrees with an independent
   count of the texels.
4. **Determinism.** I ran `wrap_surface` on DamagedHelmet with `WTE_SYNC_FINAL_BAKES=1`, once with the default thread
   count and once with `RAYON_NUM_THREADS=1`. The composite hashes are e3a5c6a2fda4a30e (off) and a2c8870c3f424c6d
   (on), the same as the builder's. The coverage and panel lines are identical and all 7 shots are pixel-identical.
   I did not repeat the other 2 models.
5. **WaterBottle tint (67.81 %).** The new test passes: the body is laid isometrically. I did not re-run the
   round-6 compat probe. Note that matching edge lengths rule out a stretched body. They do not rule out a rigid
   shift or rotation of the label against round 6. So "round 6's layout drift, not a fault" is plausible but not
   proven. This is minor, and it does not affect new work.
6. **Perf tests.** The functional tests keep every functional assert, and the 200 ms limit and the cases are
   unchanged in the `perf_*` tests. Moving the limit out of the default run is the critic's option 9. See the
   minors.
- **Exports** (no export code changed). I ran `export_flow` on ABeautifulGame (15 sets, 84 files, a 227 MB zip) and
   on DamagedHelmet: ok. My own `tools\vzip.py` (`python -E -P`) checked both zips. `testzip` is clean. For every set,
   compositing the per-layer PNGs with straight-alpha "over" reproduces the flattened PNG within **1.33/255** (alpha
   exact). The one-layer file and the selection files are in the zip under the same names with identical bytes
   (5 of 5). The ABeautifulGame shot shows the toast "Exported ABeautifulGame.zip (84 files)" and "2 layers selected".
- **Speed.** Not confirmed, because the machine was busy (another agent's tests ran during my alternating runs):
   ABeautifulGame Jump on was 4,210 / 5,424 ms with the stable exe and 6,059 / 4,176 ms with this branch, and
   tiles300 Jump on was 2,256 / 1,938 against 2,064 / 1,943. The noise is larger than any difference, and there is no
   consistent outlier. The builder's idle numbers stay single source.

## Findings

Major: none.

Minor:
- **m1. DamagedHelmet now reads "Only 97 % of the picture lies on the surface" with no cause.** Of the 2.6 % bare,
  1.96 % is lost to cuts, just under the 2 % cut line. The old "3 % runs off the surface" named the wrong cause, so
  the new line is more accurate. It still does not explain anything, and the user's standard is "explain". One
  option: when no cause line shows, name the largest share. This is a wording or design choice for the user, so I did
  not change it.
- **m2. `at_part` counts any place where the front stopped on a continued part that goes on**
  (`OwnerCtx::at_part`, first branch), whether or not another part is next to it. In the flush-plates known-limit
  case the panel would say "16 % of the picture stops where it meets a neighbouring part" (`flush.log`: at_part
  0.1605), while the picture actually stops part-way across plate B. That is roughly right, because it met a
  neighbouring part it could not fully continue onto. No sweep model shows a wrong line from this branch.
- **m3. The 200 ms limits no longer run in the default `cargo test`.** The gate and HANDOFF must add
  `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`, which so far is only in ARCHITECTURE section 9.
  With other work on the machine, the serial WA7 run reached 194.9 ms of 200 once, so it can still go red under load.
  The limit itself was not loosened.
- **m4. Single-source claims I could not re-check:** attempt 2's ABeautifulGame numbers (that code is reverted), the
  idle speed numbers, the round-6 tint probe, and determinism on ABeautifulGame and WaterBottle.

Model-caused (not bugs, and the panel names them): parts200 and tiles300 gaps; the ABeautifulGame frame strip (the
model's frame is a separate flush part); noisy400 "Only 47 %" (a synthetic stress model: 50 % `beyond` with neither
`met_itself` nor `passed`, so no ends line; this was there before this branch); nonmanifold 82 % runs off.

Decisions still open for the user (unchanged from fix_wrap_2): whether to accept the ABeautifulGame frame cut, whether
to take attempt 2, and whether to accept the serial perf step. Add to them: whether a lone "Only P %" should name its
largest cause (m1).
