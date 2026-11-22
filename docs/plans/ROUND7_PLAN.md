# Round 7 plan — Wrapped Texture Editor ("brush speed")

Date: 2026-11-21. Source = `target\round6\work\base` (round 5's final source; exe `target\round5\wte_round5.exe`, SHA
5DA4E165...6568). `ARCHITECTURE.md` is the binding contract; this plan says which of its sections change. File:line
references are to `target\round6\work\base`.

One item, one package (P), one gate. **Round 6 is being built at the same time** in `target\round6\work`; round 7 owns a
file set disjoint from round 6's (section 5) and lands after round 6 (section 11).

| Item | What | Package |
|---|---|---|
| R7-1 | Cheaper and properly parallel 3D dabs on very heavy models (no pixel changes) | P |

---

## 0. Facts every agent needs

- **Build lock (shared with round 6).** Before any heavy cargo command (`build`, `test`, `clippy`, a `--no-run` bench
  build) look at `E:\claude_programs\wrapped_texture_editor\target\BUILD_LOCK`. If it exists and is younger than 90
  minutes, wait and look again every 2 minutes. Otherwise create it with `New-Item -ItemType File` (no `-Force`: if
  that fails, another agent just took it, so wait) and write one line `<your agent label> <local time>`, e.g.
  `round7-build 2026-11-21 14:05`. Run the command, and delete the file right after it ends, also when it failed. A
  lock older than 90 minutes is stale: replace it and say so in your notes. Never hold the lock while not compiling
  (reading, measuring, writing notes).
- Build from the root of your copy: `cmd /c "E:\claude_programs\wrapped_texture_editor\cargo.cmd <cmd> --offline 2>&1"`
  (`test --workspace`, `clippy --workspace --all-targets`, `build --release -p wte_app`; the release core bench is
  `test --release -p wte_core --lib --no-run`). Never edit `cargo.cmd` or `.cargo\config.toml`. **No new crates**:
  everything below uses `rayon` (1.12, already used by `brush.rs`), `glam`, `std`. The core has no `unsafe` code; this
  round adds none (no atomic views of `u8` buffers).
- Copies: `target\round7\work\pkg` = robocopy of `target\round6\work\base` (it has no `target\`: a cold build is 20-40
  min; copying a round-6 `target\` saves it only while no round-6 build runs, i.e. the lock is absent). `merged` = the
  gate's copy of the checked `pkg` (no merge in this round). Notes `target\round7\notes\pkg.md`, `check.md`, `gate.md`;
  runs under `target\round7\runs\`; report `target\round7\GATE_REPORT.md`; exe `target\round7\wte_round7.exe`.
- Machine: 6 cores, 7 GB RAM (often ~1 GB free), slow USB disk. Never scan `target\` with recursive finds or greps
  (it stalls the disk for minutes and ruins every timing that runs meanwhile). Large outputs on E: only.
- Testing is headless: `<exe> [PATH] --screenshot <out.png> --scenario <name> --size 1280x800` (writes `<out>.json`
  with `"ok"`). No OS mouse/keyboard injection. Never kill a running `wrapped_texture_editor.exe` or `wte_*.exe`.
  Never run downloaded binaries. `WTE_FRAME_LOG=1` = per-frame timing lines; `WTE_STROKE_LOG=1` = the stroke probe
  (round 5): one line per sample of 20 ms or more and a `brush stroke probe:` line per stroke.
- **Timings belong to the gate.** Builders may take indicative numbers, always as in-process A/B (section 3.5) or
  marked "loaded". The gate measures only when the lock is absent, no `cargo` / `rustc` process runs and no other
  `wte` / `wrapped_texture_editor` exe runs (round 6's agents included); it waits otherwise, and it records the load
  of each run (top-5 CPU processes before and after) as round 5 did (`load_per_run.txt`). One process at a time,
  alternating with `wte_round5.exe`.
- Models: `E:\claude_downloads\glb_samples\ABeautifulGame.glb` (1.5 M triangles, 15 sets),
  `target\compat\harness\shapes\ngons_tiling.obj`, `target\compat\topologies\models\tex_4096\tex_4096.obj`,
  `...\tex_8192\tex_8192.obj`, the built-in sample.
- Tools: `target\round4\review\gate3\` (`btp.py`, `frames.py`, `wf_steps.py`, `core_benches.ps1`),
  `target\round3_audit\` (`sweep.ps1`, `slow.py`), `target\round3\shot_diff.py`. Round 5's measurements to compare
  with: `target\round5\GATE_REPORT.md` (B4-B6, phase split), `target\round5\notes\pkg_b.md` (bench rows, probe
  table), shots of round 5's 38-scenario sweep under `target\round5\runs_gate\` (same exe, same SHA).
- Read only what you need: `brush.rs` is 119 KB and `stroke.rs` 96 KB. The ranges named below are enough.

---

## 1. What the user gets

On the chess set (ABeautifulGame, 1.5 million triangles) the brush keeps up with the mouse at the default size: the
paint trails the pointer by at most about a third of a second (round 5: 0.8-1.7 s), and no frame of a stroke takes
longer than 140 ms. A very large brush (Size 512) stops painting about 3 seconds or less after the release (round 5:
7-12 s). The pixels are exactly the same as round 5's. Light models paint as before or faster. Nothing new to set.

---

## 2. What the code shows (found while planning)

**2.1 The dab is already parallel, but it is split badly.** `rasterize_tip` (`brush.rs:879-949`) takes the band path
when its "work" reaches 65,536 texels. The work is the sum of the candidates' **clip boxes**, and a clip box is the dab's
whole footprint in UV space around each candidate (`prepare_tri`, `brush.rs:717-718`), so every heavy dab takes the
band path. The flaws:
- a. **Binning by clip box.** Every candidate of an island is listed in every 64-row band of the whole footprint
  (about 4-7 bands for a heavy Size-50 dab, about 30 at Size 512). Each of those bands sets the triangle up again
  (`TriSetup::new`, `texel_map.rs:680`) only to find that its box misses the band.
- b. **The binning loop is O(candidates x bands) and serial** (`brush.rs:907-921`), with a fresh `Vec<Vec<u32>>` and
  `extents` per dab.
- c. **The parallel width is the number of 64-row bands the footprint spans.** A Size-50 dab on one island has 4-7
  uneven bands, so at best about 3 of the 6 cores do useful work.
- d. **The commit is serial** unless the dab touched at least 65,536 texels **and an eighth of the image**
  (`brush.rs:1047`). Heavy Size-50 dabs (50-140 k texels on 2048² / 4096² sets) never qualify. This is the 13 %
  "commit" of round 5's phase split.
- e. **`prepare_tri` is serial** (`dab_tip`, `brush.rs:1257`): the 4-5 % "setup".

**2.2 The cost per candidate triangle dominates, not the cost per texel.** Round 5's bench (`notes\pkg_b.md:187-190`)
gives ABeautifulGame 0.59 ms for a 50-texel dab, about 300 ns per touched texel, while pass 1 costs about 10-20 ns per
texel (barycentrics, a 3D point, `falloff`). On a dense mesh a triangle covers a few texels. But `raster_tri`'s pass 2
(`texel_map.rs:812-897`) scans **every texel of the triangle's box grown by the gutter** to find its gutter texels, its
sliver-claimed texel and shared-gutter texels. The gutter is 8 texels at 2048² and 16 at 4096² (`default_gutter`), so a
3-texel triangle costs about 400-1,400 owner reads, and an interior triangle finds nothing. The indexed variant
(`pass2_row_candidates`, `:528-551`) starts only at boxes of 4,096 texels and costs binary searches per row.
**Estimate: pass 2 is half or more of raster on this model.** It was never measured, so step 0 measures it first.

**2.3 Consequence.** The gate's conclusion, "rasterize one dab in parallel", is right but incomplete. The parallel
scheme must split one dab into many balanced pieces, and the per-candidate work that produces nothing must go. Both
can be done without changing a pixel.

---

## 3. Technique

### 3.0 Step 0: measure where the raster time is (builder, first, about 15 min of runs)

New probe slots (appended after slot 9, `STROKE_PROBE_NAMES` in `stroke.rs:57-59`; `brush::probe::SLOTS` 10 → 15):
`10 pass-2 texels` (count: texels of the pass-2 boxes actually scanned), `11 pass-2 skipped` (count: candidate-band
calls whose pass 2 was skipped, 0 before A1), `12 bands` (count of band jobs), `13 band work` (time: band job times
summed over all threads), `14 candidates` (count of prepared candidates). The counters of worker threads are
thread-local and would be lost (round 5 noted this). So the band jobs **return** their counts and times, and the
calling thread adds them after the parallel loop. There is no per-texel probe work: a box's area is added once per
call. With the probe off, each call costs one relaxed load. App: `phase_text` (`tools\brush.rs:67-81`) prints slots
10, 11, 12 and 14 as counts.

Run once, release, on ABeautifulGame: `brush_types_path` with `WTE_STROKE_LOG=1` (round-5 code + slots), and the core
bench with `WTE_BENCH_MODELS=b WTE_BENCH_PROBE=1`. Write into `notes\pkg.md`: per stroke pass-2 texels / touched texels,
candidates per set-dab, band work / raster (= the effective parallel width), and the texture sizes of the sets.
Decision rules (the plan's order holds unless the numbers say otherwise):
- pass-2 texels ≥ 5 x touched texels → A1 is the main win (expected);
- band work / raster < 3 on the burst dabs → A2's finer bands are a main win (expected);
- query ≥ 10 % of stroke work after A1 + A2 → do A3; re-apply ≥ 15 % after A1 + A2 → do A4.

### 3.1 A1: skip pass 2 where it cannot report anything (`texel_map.rs`)

Pass 2 of triangle T reports only (i) gutter texels T owns, (ii) the texel T claimed as a sliver (build step 2,
`:401-409`), and (iii) gutter texels listed in `shared_gutter` whose source T covers. That is exactly the candidate
rule of the indexed path (`pass2_row_candidates`), which the existing tests prove equal to the full scan. Make the
emptiness test O(1):
- `TexelMap::claimed: Vec<TriId>` (sorted; the slivers that actually claimed a texel, recorded in build step 2;
  `TexelMap { .. }` is built only at `:478`).
- `pass2_tris: OnceLock<Vec<u64>>`: a bitset over `TriId` = owners of any gutter texel ∪ `claimed` (about 188 KB per set
  for 1.5 M triangles, 2.8 MB for 15 sets), built in parallel over `owner` in chunks (a set union, so the result is the
  same on any thread count).
- `shared_blocks: OnceLock<Vec<u64>>`: one bit per 8 x 8 texel block that holds a `shared_gutter` texel (8 KB at 2048²,
  32 KB at 4096²).
- `pub(crate) fn prepare_brush_index(&self)` builds both. `stroke_target` (`stroke.rs:602`) calls it on a set's first
  touch, so the cost (about 5-20 ms per 4096² set, once per map for the map's lifetime) lands in the "first touch" slot.
- `raster_tri_with` gets `lean: bool`. When it is set, T's bit is 0, and no shared block meets the pass-2 box (the
  folded `cols` x `wrapped_rows` ∩ `rows`, already computed at `:814-816`), pass 2 is skipped. New
  `pub(crate) fn raster_tri_lean` / `raster_tri_covered_lean` pass `true`, and **only `brush.rs` calls them**. The public
  `raster_tri` / `raster_tri_covered` and every stamp, wrap and fill caller stay exactly as they are (round 6's bakes use
  them).
- Same file, pixel-neutral: the folded row loops (`Folding::All` `:724-740`, `Folding::Blocks` `:749-788`) walk every
  box row and `continue` outside `rows`. Make them walk only the rows that fold into `rows`: one or two intervals for a
  wrapped box. With 32-row bands a whole-image tiled triangle (tex_8192, ngons_tiling) would otherwise loop over 8,192
  rows in each of 256 bands.

### 3.2 A2: split one dab into many balanced pieces, and commit inside them (`brush.rs`)

Replace the band path of `rasterize_tip` and the commit after it (`dab_tip` `:1263-1272`) with one fused parallel pass:
1. **Prepare in parallel**: with 4,096 or more triangles in the list, `cands.par_extend(list.par_iter().filter_map(prepare_tri))`
   into the reused `scratch.cands` (order preserved; the order does not affect any result anyway).
2. **Tight boxes**: new `pub(crate) fn raster_box(mesh, map, tri, clip) -> Option<[i64; 4]>` in `texel_map.rs` = the
   clipped unwrapped scan box exactly as `raster_tri_with` computes it (`:680-696`), so both share one function.
   Candidates whose box is empty are dropped. Work estimate = the sum of box areas (rows and columns capped at the
   image).
3. **Serial below `PAR_DAB_WORK` = 16,384** texels of work: today's serial loop + `commit_dab`, unchanged. The builder
   may move the threshold after the bench (rows a/c at 50 texels must not get slower), but never by thread count.
4. **Bands of `DAB_BAND_ROWS` = 32 rows** (16 or 64 if the bench shows ≥ 5 % better; a constant, never derived from the
   thread count). **Binning is CSR in O(candidates + entries)**: count per band from each box's wrapped row range (a box
   spanning ≥ h rows goes into every band), prefix sums, fill. The buffers are reused in `StrokeScratch`. A small
   triangle lands in 1-2 bands instead of every band of the footprint.
5. **One parallel pass over the bands that have candidates**: `mask.par_chunks_mut(B*W).zip(dab_cov.par_chunks_mut(B*W))`.
   Per band: `raster_cand` (lean) of its candidates within its rows → band-local touched list and core runs
   (`put_core_runs` per band, as today) → **commit of its own texels** (same `mask_increment`, `dab_cov` reset to 0) →
   a band-local `RectTracker`. The band returns its tracker tiles and probe counts. The calling thread merges the tiles
   **by union per tile key** in band order (32-row bands share 64-row tiles, so `extend` would be wrong) and calls
   `finish()`.
6. The invariants stay: `dab_cov` is all zero and `dab_touched` is empty between dabs, and the returned rects are those
   of the serial commit (the tracker's result does not depend on the order in which texels arrive).

**Why the pixels cannot change:** each band owns its rows of `dab_cov` and `mask`. Within a dab a texel's coverage is
the max over the fragments of its own rows (order-free). The commit of a texel depends only on that texel. Lean pass 2
visits exactly the fragments the full pass 2 visits. The partition depends only on the dab, never on the thread count.

### 3.3 A3 (only by the rule of 3.0): the shared query at large radii (`brush.rs`, optionally `bvh.rs`)

At Size 512 the query is 7-10 % of stroke work (round 5 probe table). In `shared_candidates` (`brush.rs:1157-1229`),
when `found` holds 32 k or more entries, run both loops over `found` in parallel chunks. The first loop gives per-set
maxima (order-free). The second gives per-chunk per-set lists, concatenated in chunk order, so the lists stay ascending.
In `bvh.rs` `query_sphere_dist` (`:453-457`), `par_sort_unstable_by_key` above 32 k entries (the keys are unique). The
tree walk stays serial.

### 3.4 A4 (only by the rule of 3.0): re-apply (`stroke.rs`)

`reapply_rect` (`stroke.rs:239-280`) reshades whole rects, although most of a mid-stroke dab's rect is already
saturated. Exact variant: per rect row, shade only the span from the first to the last texel whose mask changed in this
dab (from the band commits, kept per tile row). Keep it only if re-apply drops ≥ 30 % on the bench.

### 3.5 Measuring seam and what was ruled out

`#[cfg(test)] thread_local! RASTER_R5` (like `FRESH_CANDIDATES`): `dab_tip` then runs round 5's code (`rasterize_tip`
and `commit_dab` kept verbatim as `#[cfg(test)]` references, full pass 2). This seam is the reference of the
equality tests and the in-process A/B of `stroke_bench_tips` (new rows "… r5" next to each tip; ABeautifulGame also
gets a 200-texel diameter, close to the heavy app dabs). Read once per dab on the calling thread.

Ruled out:
- **Per-thread private coverage buffers merged by max.** 4-16 MB per thread and set, and a merge proportional to the
  footprint.
- **Atomic `fetch_max` on a shared buffer.** It needs `unsafe` views of `u8` buffers.
- **The sets of one dab in parallel (round 5's B-4.2).** At Size 50 a dab reaches 1.24 sets. At Size 512 each set's dab
  already fills every core with bands.
- **GPU rasterization.** A round of its own, with a risk of pixel differences.
- **Any change to spacing, bridging, query radius or coverage maths.** These change pixels.

---

## 4. Threads and determinism

- Dab work runs on the **global rayon pool** (6 threads), called from the UI thread, which waits for the dab (as round
  5's band path already does; a non-pool caller does not take part in the work). No new pool.
- **Final stamp bakes** (round 5, `stamp-bake-N` pool, 5 threads; round 6's wrap bakes use the same pool) do not start
  while a stroke is open (`stamp_bakes::pump`). A bake already in flight when a stroke starts keeps its threads until it
  lands (≤ about 1.7 s on ABeautifulGame). For that time 11 threads share 6 cores, and the stroke's first dabs run at
  about half speed. This is accepted: it is short, and it cannot deadlock (separate pools, and the UI thread waits only
  on the global pool).
- Workers of other jobs (import, save encodes, export) that use the global pool share its queue with a dab. Both make
  progress. The builder checks which of these jobs can overlap a stroke (non-modal) and writes it in the notes. Nothing
  else changes.
- **Determinism:** the partition into bands depends only on the dab. Each band owns its rows (no shared writes, no
  locks). Coverage uses the max rule. Merges go in band order, and the tracker union is order-free. `prepare_brush_index`
  builds set unions. Proof: tests T2 (pools of 1, 2 and 5 threads) and the gate's `RAYON_NUM_THREADS=1` / `=2` app runs.

---

## 5. Files and ownership (disjoint from round 6)

| | Round 7 (package P) | Round 6 (plan 4.8, 5.1) |
|---|---|---|
| Core | `brush.rs`, `stroke.rs`, `texel_map.rs`, `bvh.rs` (A3 only: `query_sphere_dist`) | new `wrap.rs`, `stamp.rs`, `stamp_bake.rs`, `stamp_job.rs`, `doc_stamps.rs`, `document.rs`, `lib.rs`, `samples.rs`, `error.rs`, `project\{manifest,save,load,migrate,mod}.rs`, `tests\project_roundtrip.rs` (uses `bvh.rs`, `blend.rs` without editing them) |
| App | `tools\brush.rs` (`phase_text` only) | `state.rs`, `app.rs`, `panels\tool_options.rs`, `panels\text_block.rs`, `panels\layers.rs`, `tools\stamp_place.rs`, `tools\transform.rs`, `tools\text_path.rs`, `view\gizmo.rs`, `view\mod.rs`, `selftest\scenarios.rs`, `selftest\mod.rs` |
| `ARCHITECTURE.md` | 4.6 (bullet 3), 5.9 (one sentence), 5.19 (probe slots), 6.3 ("Parallel pattern"), 6.4 (commit paragraph + a "Round 7" paragraph before "2D dab") | 1.6, 2.1, 5.12, 5.14, 5.20, new 6.5a, 7.2, 7.3, 8.2, 8.6 (Stamp / Text / Transform rows), 8.7, 8.11, 9.1, 9.2 |

No file appears in both columns. Round 7 adds no public item that needs `lib.rs`: `pub use stroke::*` and
`pub use texel_map::*` already export new public names, and the new texel-map functions are `pub(crate)`. Round 7 adds
no test meshes to `samples.rs` (its test meshes live in its own test modules) and no scenario. Two shared spots are
**not edited in the copy**: the new `1.7 Round 7` paragraph and the `9.1` test lines are written into `notes\pkg.md`,
and the main session pastes them at landing (round 6 inserts `1.6` and edits `9.1` there).
Round 6's bakes call `raster_tri` / `raster_tri_covered` / `TexelMap::locate`; their behaviour and signatures do not
change.

---

## 6. Tests package P adds (core, in-file; existing expected values never change)

| # | Test | Must hold |
|---|---|---|
| T1 | `r7_dabs_equal_round5_on_random_dabs`: uv_sphere, seam_cube, `tiled_strip(.., 3.0)`, stacked and mirrored quads (shared gutters), a sliver mesh (as `sliver_fallback_claims_the_centroid_texel`), a wrapped-UV quad; sets 1x1, 3x5, 64², 300x420, 1024²; gutter 0, 2, 8, 16; ball hard / soft, Cube Solid and Through; radii from under a texel to the whole image; SameSide / All. Each dab with the seam on vs off | masks, `dab_cov` all zero after, rects as sets: identical |
| T2 | `dab_raster_is_the_same_on_any_thread_count`: large dabs (all paths: serial, banded, folded, tiled) inside pools of 1, 2 and 5 threads | masks, rects and layer bytes identical |
| T3 | `lean_pass2_visits_the_same_fragments`: every triangle of T1's meshes, random clips and row ranges (also wrapping ones and empty ones) | fragment sequence of the lean call == the full call, bit for bit |
| T4 | `fused_band_commit_equals_the_serial_commit`: sparse, dense, wrapped dabs; band rows 16 / 32 / 64; image heights not a multiple of the band | masks and rects identical |
| T5 | `a_stroke_paints_round5s_pixels`: a whole multi-set stroke (cube path + seam crossing) with and without the seam; probe on vs off | layer hashes identical |
| T6 | `prepare_brush_index_is_complete`: on T1's maps, every triangle that owns a gutter texel or a claimed texel has its bit, and every block with a `shared_gutter` texel has its bit | as stated |

Unchanged and still passing: C1 / C2 hashes, `tests\brush_tips.rs`, `stroke::tests` (incl.
`one_dab_per_call_gives_the_unpaced_stroke`), `tools::brush::tests` A1-A19, `large_dab_parallel_path_matches_serial_rasterization`,
`the_parallel_commit_of_a_dense_dab_matches_the_serial_commit`, `a_large_field_dab_is_the_same_on_the_pool`, every
`texel_map` test (`raster_tri` itself is unchanged for its callers). `stroke_bench_tips` asserts r7 hash == r5 hash
per row.

---

## 7. Done means (package)

Step 0's table in the notes; A1 + A2 built (A3 / A4 by their rules); T1-T6 pass; `cargo test --workspace` (round 5:
1,067 passed, 32 ignored, 0 failed: the count only grows) and clippy `--workspace --all-targets` clean; release exe;
`brush_types`, `brush_types_path` (ABeautifulGame, ngons_tiling), `workflow_path` (ABeautifulGame) ok; `stroke_bench_tips`
models a, b, c run once with the r5 rows in the same process (indicative); one ABeautifulGame `brush_types_path` run
with `WTE_STROKE_LOG=1` (indicative); `ARCHITECTURE.md` sections of section 5 written; the 1.7 / 9.1 texts in the notes.

---

## 8. Process (lean: three agents, keep the weekly budget)

| Order | Agent | Works in | Output |
|---|---|---|---|
| 1 | build (Opus) | `work\pkg` | code, tests, `notes\pkg.md` |
| 2 | check-and-fix (Opus): reads the diff against 3-6, tries to break it (T1-T4 cases of its own: boxes wider than the image, a band at the image's last rows, gutter 0, 1 x N images, stacked UVs whose gutter is shared, field tips with folded blocks, probe on / off, `RAYON_NUM_THREADS=1`), fixes, reruns tests and clippy, rebuilds the release exe | `work\pkg` | `notes\check.md` |
| 3 | gate: copy to `merged`, tests, scenarios, measurements, report | `work\merged`, `runs\` | `notes\gate.md`, `GATE_REPORT.md`, exe |

The builder works in this order, so a context limit still leaves usable work: (1) copy, build, probe slots, step 0;
(2) the seam + A1 + T3 / T6 / T1 (all green); (3) A2 + T2 / T4 / T5; (4) bench A/B and smoke runs; (5) A3 / A4 by
their rules; (6) `ARCHITECTURE.md` and notes. Every agent follows the build lock of section 0 and writes the plan's
numbers next to its measured values.

---

## 9. Gate (one agent)

1. `merged` = robocopy of the checked `pkg` (with its `target\`). `cargo test --workspace`, clippy (0 warnings), no
   `todo!` / `unimplemented!`, release build; copy to `target\round7\wte_round7.exe`, record SHA-256.
2. Pixels: `sweep.ps1` over all 38 scenarios on the built-in sample at 1280 x 800. Each `shot.png` must be identical to
   round 5's sweep shot (`shot_diff.py`, toast area excepted; run `wte_round5.exe` only for names missing there).
   `brush_types` again with `RAYON_NUM_THREADS=1` and `=2` → identical shots. Read `shot.png` of `brush_types`.
3. Measurements, at most 2 runs per line, alternating with `wte_round5.exe`, under section 0's idle rule (about 22
   runs): `brush_types_path` on ABeautifulGame (2 pairs, `WTE_FRAME_LOG=1`) plus one round-7 run with
   `WTE_STROKE_LOG=1`; `brush_types_path` on tex_4096 and ngons_tiling (2 pairs each) and tex_8192 (1 pair);
   `workflow_path` on ABeautifulGame (1 pair); fuzz seed 7 on ngons_tiling (1 run); `stroke_bench_tips` models a, b, c
   (one run, in-process r5 rows).
4. Report: each line of section 10 with its number and the file it came from, the new phase split (round 5's table
   format plus slots 10-14), what was not met and why, what to try by hand.

---

## 10. Acceptance (release build, 1280 x 800, against `wte_round5.exe` back to back)

| # | Line | Round 5 (gate report) |
|---|---|---|
| P1 | All tests pass with unchanged expected values; T1-T6 pass; clippy 0 | 1,067 / 32 ignored |
| P2 | Pixels: the 38 sweep shots identical to round 5's; `RAYON_NUM_THREADS=1` / `2` shots identical; every `stroke_bench_tips` row r7 hash == r5 hash | |
| P3 (B4) | ABeautifulGame `brush_types_path`, Size 50, after the warm-up, both runs: longest frame of stroke work **≤ 140 ms**, **0** drag frames over 140 ms, longest dab **≤ 40 ms** | 123-156 ms; 0-4 frames over 140; dabs 47-81 ms |
| P4 (B6) | Same strokes: largest backlog **≤ 350 ms** on each, both runs | 760-1,683 ms |
| P5 | Same strokes: stroke work **≤ 0.50 x** round 5, each stroke, 2 pairs | 1,712-2,506 ms per stroke |
| P6 (B5 / P8b) | Size 512 on ABeautifulGame: release to end **≤ 3.0 s** on each stroke, both runs; stroke work ≤ 0.50 x round 5 | 6.7-11.9 s |
| P7 (B5) | Size 50: release to end ≤ 400 ms; pointer step ≤ 1.10 x round 5 | 2-14 ms |
| P8 | Light models: `brush_types_path` on tex_4096, tex_8192, ngons_tiling, stroke work per stroke ≤ 1.05 x round 5 (min of the runs); bench rows of models a and c: r7 ≤ 1.03 x the r5 row of the same run | |
| P9 | `workflow_path` ABeautifulGame: every step within 15 % of round 5 or faster; the three brush strokes reported (expected ≤ 0.6 x) | |
| P10 (B7) | Fuzz seed 7: no `pointer/tools/hover` frame over 500 ms; 0 "panicked" / "Invalid parameter" | 0 |

**Why these numbers are reachable (estimates; step 0 confirms or corrects them).** A heavy Size-50 dab in round 5 is
about 14 ms on average in a burst, of which raster is about 10 ms. A1 removes the empty pass-2 scans (estimated −40 to
−70 % of the raster CPU). A2 widens the parallel part from about 3 to 5-6 cores and moves commit and setup into it. The
burst dab becomes about 4-6 ms, and the longest (50-80 ms) about 15-30 ms, which meets P3. The backlog grows with work
per sample times the samples that queue. At 2.5-3 x cheaper dabs it falls from 760-1,683 ms to about 200-450 ms, so
**P4 is the line most at risk**. A3 / A4 exist to win the last 10-20 %.

**Size 512.** The runner feeds the drag over a fixed number of frames, and each frame paints at least one dab (a dab is
longer than the 100-ms frame budget). So release to end ≈ the dabs still queued at the release (round 5: about 4-6) x
the time of one dab (round 5: 1.4-2.1 s). At this size raster is 85-90 % of the work and is dominated by candidates:
about 30 bands per candidate in round 5's binning, and the pass-2 boxes. Expected about 3-4 x per dab, so 0.35-0.6 s per
dab and **about 1.5-3 s** release to end. P8b's 1.0 s would need about 8 x per dab and is not promised.

---

## 11. Landing after round 6 (main session)

1. Three-way merge: base `target\round6\work\base`, ours `target\round7\work\merged`, theirs = the main tree after round 6
   lands. Expected: no `.rs` file overlaps. If round 6's final tree changed any round-7 file (`git diff --no-index`
   against base for the files of section 5), merge it by hand and say so. `ARCHITECTURE.md`: disjoint sections; then
   paste the 1.7 paragraph and the 9.1 lines from `notes\pkg.md`.
2. Re-test on the merged tree: `cargo test --workspace` (round 6's WA tests run through `raster_tri`, whose file round 7
   changed), clippy, release build; scenarios `brush_types` (also with `RAYON_NUM_THREADS=1`), `brush_types_path` on
   ABeautifulGame (one run with `WTE_STROKE_LOG=1`: stroke work within 15 % of the round-7 gate's run),
   `workflow_path`, `wrap_surface`, `final_bake_async`, `stamp`; `shot.png` of `brush_types` identical to the round-7
   gate's.

---

## 12. Risks

| Risk | Answer |
|---|---|
| Step 0 shows pass 2 is not the big cost | A2 still applies. The 3.0 rules steer the rest. The report states the real split. P4 may then stay unmet: say so, do not fake it. |
| Lean pass 2 misses a fragment (an unknown owner case) | The rule is the one the indexed path already relies on. T3 compares every triangle on adversarial meshes, and T1 compares whole dabs against the seam. |
| 32-row bands cost more setups for tall triangles | Tight boxes keep a small triangle in 1-2 bands. The folded-row fix keeps tiled triangles cheap. P8 and the a/c bench rows catch a regression; then use 64 rows. |
| Small dabs pay rayon overhead | Serial below `PAR_DAB_WORK`. The bench's 50-texel rows decide. |
| Memory | Bitsets about 3 MB in all. Band buffers are reused. No per-thread image copies. |
| A stamp bake in flight shares the cores | Accepted (section 4). Not measured: the scenarios do not overlap a bake and a stroke. |
| Round 6 compiles at the same time | The build lock, and the gate's idle rule for timings. |
| Merge onto round 6 | Disjoint files. Section 11 lists the re-tests. |

---

## 13. Cut order (if the weekly limit runs short; first cut first)

1. A4 (re-apply).
2. A3 (query and sort; `bvh.rs` is then not touched).
3. Gate: the tex_8192 pair and the `workflow_path` pair (keep tex_4096 and ngons_tiling for P8).
4. Band-height tuning (keep 32) and the 200-texel bench diameter.
5. The check agent's adversarial extras beyond T1-T6 (keep its diff review and its rerun).

Never cut: step 0, A1, A2, the seam, T1-T6, the pixel sweep, P3-P6 measured back to back.

---

## 14. Defaults (no question blocks the build) and questions

1. No new setting and no visible change other than speed. The stroke probe log gets five more fields.
2. While a stamp's Final bake is still running, the brush shares the cores with it (no priority given to either).
3. If P4 (backlog ≤ 350 ms) is still missed, the round reports it with the phase split. It does **not** drop or merge
   samples to make the number: that would change pixels.
4. Question: is "Size 512 stops ≤ 3 s after release" good enough for now, or should a later round aim at 1 s (it needs
   about 8 x per dab; likely the GPU)?
5. Question: should the gate also time a stroke started while a Final bake is in flight (one extra scenario step in a
   round-6 file, so only after landing)? Default: no.
