# Stable, acceptance fixes A-F: independent check

Opus (checker), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable`, builder head c4da6fc
(`docs/notes/stable/accept_fixes.md`). Raw outputs (git-ignored) are in `target\accfix\chk\`: `bin\` (exes), `runs\`
(scenario runs), `zip_lantern\`, `zip_helmet\`, `mutations.diff` (my mutations, not committed), and the logs `test.txt`,
`clippy.txt`, `gpu.txt`, `f_mut.txt`, `test_app2.txt`, `clippy2.txt`, `build2.txt`. Every cargo command and app run used
`start /b /low /wait`, one app at a time.

**Verdict: the six items are fixed. Every new check fails without its fix (my own mutations). No test limit was loosened.**
F is a real fitting bug, not the model; my own ray casting reproduces the builder's numbers. I made two small fixes (below).

| Exe (`target\accfix\chk\bin\`) | What | SHA-256 (first 16) |
|---|---|---|
| `wte_chk.exe` | release build of c4da6fc; cargo found it up to date, and it is byte-identical to the builder's `wte_final.exe` | 577fd91e16c99372 |
| `wte_chkmut.exe` | c4da6fc plus my env-gated mutations `WTE_CHK_A/B/D/E1/E4` (`mutations.diff`, sources restored afterwards) | 1e4229cd1300c5b0 |
| `wte_chk2.exe` | c4da6fc plus the checker's two small fixes (this commit) | 27b3067568b13a23 |

## Gate commands

| Command | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | 1232 passed, 1 failed, 43 ignored. The failure is WA20, `cells 0.689751281166193..1.9861376786437033`, which is the known red with the same numbers as the builder. |
| `cargo clippy --workspace --all-targets` | 0 warnings (also after my fixes) |
| `cargo test -p wte_app -- --ignored --skip perf_` (GPU app tests) | 18 passed |
| `cargo build --release -p wte_app` | OK (also after my fixes); `cargo test -p wte_app` after my fixes: 382 passed |

## Repro runs on `wte_chk.exe`

All `"ok": true` unless noted:
- `wrap_surface`: Avocado at 1280x800 and 1024x640, and Lantern at 1280x800. The first Lantern 1280 run failed (6b),
  "the drag started 1 Preview jobs, 0 landed (1 dropped)". It ran while clippy and the GPU tests were running, and the
  builder saw the same failure under load. The rerun on an idle machine passed. Lantern at 1024x640 fails the (10) quality
  assert with "dup 3.401 %". The builder's `wte_before` run gives the same numbers, so that red was there before.
- `export_checks`: 1280x800 and 1024x640.
- `final_bake_async`: sphere, Avocado and WaterBottle.
- `stamp_edge_path`: WaterBottle, Avocado and Lantern.
- `workflow_path`: BoxTextured, and the old project `target\accfix\oldproj\old_box.wtep`, saved by 8328B74C.
- `export_flow`: Lantern and DamagedHelmet.
- `export_selected`.

`my_zip_check.py`:
- `zip_lantern`: RESULT PASS (8 entries, composite worst 0/255, loose twin).
- `zip_helmet`: RESULT PASS (worst 1/255).

The zips list the stamp files as `..._stamp_Logo.png`.

## Per item

**A. Caption.** `preview_caption` covers four cases, and each one has a unit test. The screenshot
`runs\ec_1280_hidden.png` shows the dialog in Selected (3) mode with Flattened off. Its caption reads "The preview shows
the whole texture; only the selected layers are exported, as separate files". The Logo line has the "(hidden)" marker.
- *Mutation* `WTE_CHK_A` (the old caption logic) fails `export_checks` (E2) with "the caption is Some("Shows the flattened
  texture exactly as exported ...")".
- *All + flattened.* The builder keeps "exactly as exported" for All with the flattened texture on. That is correct.
  "flattened always = visible only" was already in ARCHITECTURE 5.x (`ExportOptions::include_hidden`) and in
  `export/mod.rs` before this work. My own compositor confirms it: the Visible and All flattened PNGs are byte-identical.
  The All button's tooltip still said only "Hidden layers too", so I made it say where they go (fix 2 below).

**B. Side panel.** The cause the builder found, egui's 64 pt `min_scrolled_height` on the Color scroll area, matches
the logged rects:
- with the fix: Color 490..518, lower area from 518;
- with the mutation: 533..561.

I looked at the screenshots myself:
- `ws_avocado_1280.png` (Texture sets card) and `ws_lantern_1024.png`: the tool options end above the "Color" header, and
  the header and the Layers | Brush | Text strip are clear.
- `ws_avocado_1024_off.png` (Color open): normal.
- The pre-fix zoom `target\accept\layer_save\lantern_ws_overlap_zoom.png` showed the overlap.

*Mutation* `WTE_CHK_B` (Color min height back to 64) fails `wrap_surface` (10) on Lantern and Avocado at 1280x800: "the
Color section ([[989 533]-[1272 561]]) runs 43.0 pt into the lower area". Its screenshot `runs\mut_B_ws_lantern.png`
shows the old overlap: "Flip X" over "Color", "Delete stamp" over the tab strip.

Limit: the new assert runs only in the PATH variant of `wrap_surface` (`ws_path_steps`), where the panel is long. The
sphere run does not check it.

**C. final_bake_async (k).** On Avocado the log says "(k) the dialog opened; bake still pending: true", so the new step
does see a dialog that opened before the bake landed. The Avocado run also shows that the old step failed by design,
not because of timing. The step passes on the sphere, Avocado and WaterBottle. I did not make my own mutation for C (not
asked for); the builder's `WTE_MUT_C` result stands as reported.

**D. Stamp names.** The checks:
- `workflow_path` BoxTextured logs `stamp "wf_stamp" placed`.
- *Mutation* `WTE_CHK_D` (full file name) fails it with 'the stamp is "wf_stamp.png", its layers ["wf_stamp.png"]'.
- The old project opens with `"wf_stamp.png (stamp)"` unchanged and passes `workflow_path`.
- `project_roundtrip::an_image_stamp_named_with_its_extension_loads_unchanged` passes.

Replacing the image of an existing stamp ("Choose...", `set_stamp_source`) does not rename the stamp, as before.

**E. export_checks.** I checked the written files a second time with my own numpy compositor
(`target\accfix\chk\ec.py`, ARCHITECTURE 6.6 with the alpha-0 rule). It gives the builder's numbers:
- Visible and All flattened PNGs against the reference: max 1/255, 0 texels over 2/255.
- Against Normal 100 %: 18644 texels differ by more than 8/255.
- All 2611 of Logo's opaque texels are left out of the flattened PNG.
- E3 against Base + WTE: max 0.
- The E2 Logo file is byte-equal to All's Logo file (8298 inked texels).
- The Visible and All per-layer files are identical.

The in-test `reference_composite` matches 6.6: straight alpha, opacity quantized once, the alpha-0 rule.

*Mutations:*

| Mutation | Result |
|---|---|
| `WTE_CHK_E1` (All drops hidden layers' per-layer files) | E1 fails: "per-layer files: Visible [0, 1, 3], All [0, 1, 3]" |
| `WTE_CHK_E4` (`blend_row` ignores layer opacity) | E4 fails: "Visible 18849 texels (max 93)" |
| no variable set (`wte_chkmut.exe` as is) | `export_checks` passes |

**F. stamp_edge_path on WaterBottle.** I did my own ray casting in Python (`target\accfix\chk\fit.py`). It reads
WaterBottle.glb with its node transforms, casts the 17 x 17 grid of `fit_projector_depth` at the placement the builder
logged, culls back faces, and grows the surface the same way. Results:
- Old rule: front 0.004657, back 0.045779. New rule: front 0.010045. These match the app logs, 0.0047 -> 0.0100.
- 83 cells, the bottom seven rows, have a first hit up to 4.4 mm in front of the grown surface. For example, the
  lower-centre cell hits z = +4.37 mm (triangle 3456, the sleeve) and then z = -0.07 mm (triangle 40, the body).

So a visible sleeve stands in front of the body, and the grown surface follows the hidden body. The bake paints the
visible sleeve, and the old default (front = half size) covered it. This is a fitting bug, not a UV or topology limit,
and the fix is right. The 0.95 limit in `stamp_edge_path` is unchanged.

*Mutation*: with the old `near_parts` line, `auto_fit_covers_a_visible_part_in_front_of_the_grown_surface` panics at
`stamp_bake.rs:2213`; the other two auto-fit tests pass (`f_mut.txt`). The fix can only raise `hi`, because the first
hit of a cell is never behind its surface hit. It stays within one half size, so it cannot reach farther than the old
default did.

## Test limits

`git diff c82bff3..c4da6fc` on the tests and scenarios. Removed lines:
- the old (k) assert ("opened before the Final bake landed"), replaced as item C asked;
- the scenario count 46 -> 47;
- the old caption asserts, replaced by exact-string asserts per case;
- `stamp.name == "ring.png"` -> `"ring"` (item D).

No numeric limit changed.

## Small defects fixed here

1. `tools/stamp_place.rs`: `image_stamp_name` had been inserted between `no_texels_text` and its doc comment, so
   `image_stamp_name`'s doc began with "The warning for a stamp or text ...". Moved the comment back to its function.
2. `dialogs/export.rs`: the All button's hover said only "Hidden layers too". It now says "Hidden layers too, in the
   per-layer, ORA and PSD files (the flattened texture is the visible layers)", so the dialog states the contract that
   E1 checks. ARCHITECTURE 8.x (Layers row) records the text.

After these: `cargo test -p wte_app` 382 passed, clippy 0 warnings, release build OK. `wte_chk2.exe` passes
`export_checks`, `export_selected`, `workflow_path` BoxTextured and `wrap_surface` Avocado.

## Notes (not defects of this commit)

- `wrap_surface` (6b) can fail under heavy CPU load: one Preview job is dropped as stale, so none lands. Rerun when the
  machine is idle.
- `wrap_surface` on Lantern at 1024x640: the (10) quality red was there before this commit (dup 3.401 %).
