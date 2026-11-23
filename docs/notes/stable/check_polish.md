# Stable, polish pass: independent check

Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable`. Checked the builder's polish
commit 94df2ca (on top of 0542f02) against its report and `docs/notes/stable/polish.md`. The user was in a fullscreen
game: every cargo command and every app run used `start /b /low /wait`, and nothing measured speed. Raw outputs
(git-ignored) are in `target\check_polish\`: `logs\`, `runs\`, `focus\`, `bin\`, scripts in `tools\`.

## Verdict

**The six polish items hold.** Each claim survived the diff review, the test runs and the scenario runs. My own Python
checks found the saved projects and the exported files correct. I found two small defects and fixed both in 65c4668
(below). What is left is wording nits and test gaps. No major defect.

## What I ran

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` (on 94df2ca, after touching the changed sources so no stale binary is reused) | wte_app 380 / 0 / 19 ignored; wte_core lib 801 / **1 failed** (WA20, the known red) / 24 ignored; brush_tips 15, end_to_end 9, project_roundtrip 23. **1,228 passed, 1 failed, 43 ignored**, as the builder reported |
| After my fix: `cargo test -p wte_app` | 380 / 0 / 19 ignored |
| `cargo test -p wte_app -- --ignored --skip perf_` (GPU app tests) | 18 / 18 |
| `cargo test -p wte_app perf_ -- --ignored --test-threads=1` | 1 / 1 (pass or fail only: the machine was under load) |
| `cargo clippy --workspace --all-targets` | 0 warnings |
| `cargo build --release -p wte_app` | OK. `bin\wte_cp.exe`, SHA-256 8328B74CF99BD9FDB46416147EEEA519D0B4837E80010CE245DA5FFE6C92A15B (the code of 65c4668; that commit only adds docs on top) |
| Focus probe (`tools/stable/focus_probe.ps1`, smoke) | the game (BlackOps3) kept the foreground: app in front 0 of 44 samples, the foreground changed 0 times |

Scenario runs (one at a time, a copy of `wte_cp.exe`), all `"ok": true`:

- `save_unsettled` on ABeautifulGame: the save waited ("waiting for the Final bakes"), the Final landed after 2,168 ms,
  longest frame 17 ms over 795 frames, both projects equal (15 sets, 31 layers).
- `save_unsettled` on `high_poly_sphere.obj` (not run by the builder): Final landed after 7,249 ms, longest frame 42 ms
  over 1,183 frames, both projects equal. My first attempt pointed at the r9 worktree's `test-assets`, where the
  git-ignored sphere is missing ("file not found"); the rerun used the main repo's copy.
- `save_unsettled` on the sphere sample: it waited too (83 ms), longest frame 7 ms.
- `export_unsettled` on ABeautifulGame, `export_selected`, `multi`, `export_flow` on the cubes and on ABeautifulGame,
  `wrap_surface` on DamagedHelmet.

## My own file checks

- `tools\cmp_wtep.py` on each `save_unsettled` pair (`logs\cmp_wtep.txt`): **ALL OK** for all three models. Every zip
  entry except `manifest.json` is byte-identical between the project saved while the Final was pending and the one
  saved after waiting. The manifests are equal apart from `saved_unix`. Every file the manifest lists has the listed size
  and CRC-32. The stamp layer has ink (ABeautifulGame 1,849,621 texels). Each set's `flattened.png` equals my own
  normal-blend re-composite of its layers within 2/255.
- `tools/stable/verify_zip_vs_loose.py` and `tools/stable/check_exp2_verify.py` on export_flow (cubes, ABeautifulGame):
  **ALL OK** (ABeautifulGame 84 entries, worst re-composite 1.33/255).
- `tools\cmp_dirs.py`: export_flow ABeautifulGame (95 files) and cubes (58 files) are **byte-identical** to the fix
  pass 2 gate's runs (f889d69, `target\stable\gate3\flow`) and to the builder's runs. export_selected, multi and
  export_unsettled are identical to the builder's runs too. All 15 screenshots equal the builder's pixel for pixel.
- The ABeautifulGame selection: 2 layers (Paint 1 and one "Logo" layer) bring 6 linked layers. The selection zip holds
  the 7 layers of the projected "Logo" design plus Paint 1. The wrapped stamp's own "Logo" layer (Knight_Black L02, a
  separate design) is correctly left out.
- DamagedHelmet `wrap_surface` step 10 reads "Only 97 % of the picture lies on the surface, mostly lost to cuts (2 %)".

## Claim by claim

1. **Export dialog note**: confirmed. `linked_extra` counts distinct sets, `linked_note` picks its / their and layer /
   layers. The unit test pins the cases.
2. **Save off the UI thread**: confirmed. `start_save` no longer calls `settle`. It shares `receive_snapshot` and
   `feed_waiting_snapshot` with export. `JobOutput::Saved` clears the feed on every path. The scenario detects the old
   behaviour (the builder's mutated exe: 1,674 ms frame), and my runs pass on three models.
3. **Wrap panel cause**: confirmed. `largest_wrap_loss` uses the same rest formula as the runs-off line. The suffix is
   only added when no cause line shows.
4. **Font timing test**: confirmed renamed, ignored, limit 0.5 s unchanged. The docs over-claimed (fixed, below).
5. **Footer Rasterize tooltip**: confirmed, with one wording defect (fixed, below).
6. **export_flow expected layers**: confirmed. `expected_selection_layers` goes through `Document::stamp_layers`, while
   `export::linked_selection` matches `Layer::stamp_id` over the snapshot, so the two are separate code paths. As the
   builder said, no mutation of `plan_contents` was made; I did not make one either.

## Fixed here (65c4668)

- **Rasterize reason for a locked paint layer** (minor). With several layers selected and the locked Base active, the
  disabled tooltip said "...: it is locked". That suggests unlocking would help, but a paint layer can never be
  rasterized. Now any paint layer reads "it is already a paint layer", and "it is locked" is kept for a locked stamp or
  text layer. The panel test now hovers the locked Base too. With the old order, it fails (checked by mutation; file
  restored and touched).
- **Docs on wall-clock tests** (minor). ARCHITECTURE.md section 9 and HANDOFF.md 4.4 said every timed test is an
  ignored `perf_*` test. Two are not: `texel_map::tests::tiled_uvs_build_in_one_pass_over_the_image` runs in the
  parallel run with a coarse 1 s guard, and the ignored `tip_bake::tests::load_times` (P10, 1 / 2 / 8 s) runs in none
  of the gate commands. The text now names both. No limit was changed.

## Remaining defects

Minor:

- m1. **Cancel while a save waits is untested.** No test cancels a save, or an export, during "Finishing stamps". The
  code path is shared and reads right: `Err(cancelled)`, then "Save cancelled", the document stays modified and `then`
  does not run.
- m2. **Item 6 has no fail-without evidence.** It has no mutation of `plan_contents` showing `export_flow` going red.
- m3. **"mostly" can overstate.** The suffix names the largest cause even when that cause is under half of the loss.
  For example, 0.6 % of 2.5 % reads "mostly ... (1 %)". DamagedHelmet's case is 1.96 of 2.6 %, which is fine.
- m4. **A share can match its threshold after rounding.** "(2 %)" is printed for shares just below the 2 % line
  threshold (DamagedHelmet 1.96 %). A reader may wonder why it has no line of its own.
- m5. **"Other" sets that hold selected layers.** In the linked note, when each design is selected in a different set,
  "+ their layers in 2 other texture sets" counts sets that already hold selected layers as "other". This is the unit
  test's intended reading, but the word "other" is loose there.

Model-caused: none new. WA20 stays the known red (cells 0.690..1.986).

## Notes for the next check

- `test-assets\shapes\high_poly_sphere.obj` exists only in the main repo (`G:\software_projects\texture_wrapper_ap`),
  not in the r9 worktree.
- `cmp_wtep.py`: a layer's `pixels.rect` is `[x, y, width, height]`.
