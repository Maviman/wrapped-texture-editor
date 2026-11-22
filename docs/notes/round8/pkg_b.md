# Round 8, package B (app, `wte_app`) â€” builder notes

Copy: `target\round8\work\pkg_b` = full copy of the checked `work\pkg_a` including its `target` folder (robocopy /E,
2026-11-22 02:14-02:26, 10,730 files, 11.07 GB, 0 failed). Builder: build:B (Opus). Logs `target\round8\logs_b\`,
runs `target\round8\runs_b\`.

## What was built (plan 4.1)

Files changed (app only): `jobs.rs`, `stamp_bakes.rs`, `io_flows.rs`, `app.rs`, `state.rs`, `tools\transform.rs`,
`panels\tool_options.rs`, `selftest\mod.rs`, `selftest\scenarios.rs`; `ARCHITECTURE.md` 8.2, 8.6, 8.7, 8.9, 8.11, 9.2.
No core file, no Cargo file changed (no new crate); hashes of the app sources before the edits in
`logs_b\app_src_before.md5`.

- `jobs.rs`: `JobOutput::WrapPreview { generation, output }` and `WrapParam { generation, output }`;
  `Jobs::spawn_silent` (non-modal, `busy()` skips it, so the status bar never shows it; start/end logged at debug).
- `stamp_bakes.rs`: `WrapJobs` (one wrap job in flight: Preview or param-only, a Preview output held during a stroke,
  the param-only request of the Transform tool, `param_failed`, counters). `pump` first runs `pump_wrap`: a held output
  is applied once no stroke is open; with no wrap job in flight and no stroke open it spawns
  `doc.next_wrap_preview_job()` (latest wins: the job is made from the stamp as it is when it starts), else
  `doc.wrap_param_job(requested id)`; `run_isolated` on the worker. A Final does not start while the wrap job in flight
  belongs to a stamp waiting for its Final (`final_waits_for_wrap_job`): it starts after that job lands.
  `on_wrap_output` (generation check as Finals; held during a stroke): `apply_wrap_preview` -> Applied { due_again }
  (the last finished picture; with `due_again` the same logic pass's `pump` starts the next job) or Stale;
  `apply_wrap_param` (stats and outline, no pixels); a param-only job that brings nothing for an unchanged placement ->
  `param_failed` (not asked again). `after_tools` (called in `App::logic` right after `ensure_wrap_param`) starts the
  param-only job just asked for in the same pass. `after_dispatch` asks for a frame when a wrapped stamp waits for its
  Preview job (a wheel notch, typing). `settle` never waits for wrap jobs. `reset_for_new_document` drops the held
  output, the request and `param_failed`. `FinalBakes::last_applied()` for the scenario's log. Log lines:
  `wrap preview: stamp N job started`, `... applied after T ms (worker W ms, due again: b)`, `... result dropped
  (stale|other document)`, `wrap fit: stamp N param-only job started`, `... parameterization landed after T ms`.
- `io_flows::install_document`: `doc.set_defer_wrap_previews(state.async_final_bakes)` (exactly when async Finals are
  on; `WTE_SYNC_FINAL_BAKES=1` keeps wrapped Previews on the UI thread).
- `state.rs`: `AppState::wrap_jobs`, `wrap_jobs_pending`.
- `tools\transform.rs` `ensure_wrap_param`: when the document defers wrapped Previews it asks for a param-only job
  (`request_param`) instead of building on the UI thread; `ui_params` counts the synchronous builds.
- `panels\tool_options.rs`: `wrap_stats_texts` adds "Fitted to the curved surface: stretched up to P %" (relaxed and
  stretch >= 5 %) and "Cut where it cannot lie flat (a corner, a handle, or where its edges meet)" (seams > 0) as weak
  help lines under "Wrapped over N triangles"; the round-6 warnings follow unchanged.
- `selftest\mod.rs`: `BakeWait` (input / Do waits) and `WaitIdle` also wait for `wrap_jobs_pending`.
- `selftest\scenarios.rs` `wrap_surface`: step 2b (relaxed, seams 0, dup <= 1 %, scatter <= 2 %, logged), step 6b (jobs
  in async mode: >= 1 started and >= 1 landed during the drag, no UI-thread parameterization by the app, nothing left;
  sync mode: no job; composite after the Final logged in both), step 10 (`... quality:` line with ink / picture,
  dup, scatter, bins, param ms, relaxed, seams per rule, stretch, rejected, jumps, the Final's applied ms; one assert
  over Jump off and on: ink <= 1.3, dup <= 3 %, scatter <= 5 %). Scenario count unchanged (39).

### Not built / deviations
1. **Warm-up job** (`wrap_mesh_prep`): not built. Package A did not build step 6 (welded graph, `WrapMeshCache`,
   `wrap_mesh_prep`), so the core has no API for it (cut order item 3 applies by necessity).
2. "No parameterization built on the UI thread (core counter)": the core has no such counter. The app tests check it
   through what the core shows instead: after each Preview move the stamp's pixels are unchanged and
   `stamp_wrap_stats(id)` is `None` for the moved placement until a job lands (a build on the UI thread would have cached
   one), and `counters.ui_params` (the app's only UI-thread build, `ensure_wrap_param`) stays 0. Request 2 below adds
   the counter.
3. "A Final waits for its stamp's Preview job in flight and reuses its parameterization": the wait is built; the reuse
   is not possible with the core as it is: after the release (flush) `apply_wrap_preview` returns `Stale` and drops the
   output's parameterization, so the Final builds it again. Request 1 below fixes it in the core (no app change needed).
4. One wrap job slot for both kinds (Preview first, then the param-only request), not two slots: the plan's "one
   wrap Preview job in flight" holds, and a param job never runs beside a Preview job (CPU contention with the
   software renderer, plan 9).

### Requests for core files (package A's; for check:B / the gate to apply, then rerun the core tests)
**Request 1** (`doc_stamps.rs`, `Document::apply_wrap_preview`): keep a Stale Preview output's parameterization when it
serves the stamp's current placement on the same inputs (no pixel changes), so the Final that waited for the job
reuses it (plan 2.5-3). Replace the `if !live { return WrapPreview::Stale; }` block with:
```rust
        if !live {
            // Round 8 (request of package B): a Preview job that lands after the release (a Final / flush) was usually
            // made for the final placement: its parameterization is cached when it serves the stamp now (no pixels
            // change), so the Final job that waited for it reuses it instead of building it again.
            if out.kind == StampJobKind::Preview
                && self.stamp(id).is_some()
                && self.same_inputs(&out)
                && let Some(used) = out.wrap
                && self.wrapped_projector(id).is_some_and(|p| used.param.serves(&self.mesh, &p))
            {
                self.store_wrap_param(id, used.param, used.fresh);
            }
            return WrapPreview::Stale;
        }
```
  Check after applying: the app test `stamp_bakes::tests::a_final_waits_for_its_stamps_wrap_preview_job` then sees the
  Final's `wrap:` line say "(reused)" when the Preview was made for the released placement (add
  `assert!(doc.stamp_wrap_stats(id).is_some())` right after the stale delivery: the cached parameterization serves).
  Undo / Esc are unaffected (the restored placement is not served by it).
**Request 2** (`wrap_param.rs`, optional): a thread-local build counter like `doc_stamps::raster_counter`, for the plan's
"no parameterization built on the UI thread (core counter)":
```rust
/// The number of parameterizations this thread has built (`wrap_param`), for the app tests.
pub mod param_counter {
    use std::cell::Cell;
    thread_local! { static COUNT: Cell<u64> = const { Cell::new(0) }; }
    pub(crate) fn bump() { COUNT.with(|c| c.set(c.get() + 1)); }
    pub fn get() -> u64 { COUNT.with(Cell::get) }
}
```
  and `param_counter::bump();` as the first line of `pub fn wrap_param(..)`. The app test
  `a_drag_of_a_wrapped_stamp_bakes_its_previews_on_a_worker_and_the_latest_wins` can then assert
  `wte_core::param_counter::get()` unchanged on the test thread over the ten moves.
5. Found by the scenario with `WTE_SYNC_FINAL_BAKES=1` and fixed: the first build let `pump` take wrapped stamps waiting in
   the deferred set for the views' throttle (`set_defer_preview_bakes` is always on in the app) although the document
   did not defer wrapped Previews, so the synchronous mode used jobs (6b failed: "8 jobs"). `pump_wrap` now runs only
   while `doc.defers_wrap_previews()`; test `the_synchronous_mode_keeps_wrapped_previews_on_the_ui_thread` added.
6. The step-10 (PATH) part of `wrap_surface` now collapses the Color section before the screenshot (as the run without a
   PATH always did), so the PATH shots show the stats lines; only the side panel of those shots changes.

## Tests (plan 4.2) and build (2026-11-22; logs in `logs_b\`)
- New app tests, all pass: `stamp_bakes::tests::{a_drag_of_a_wrapped_stamp_bakes_its_previews_on_a_worker_and_the_latest_wins,
  esc_or_undo_while_a_wrap_preview_job_runs_keeps_the_old_pixels, a_wrap_preview_of_another_document_is_dropped,
  a_selected_wrapped_stamp_without_parameterization_gets_it_from_a_param_only_job, a_final_waits_for_its_stamps_wrap_preview_job,
  the_synchronous_mode_keeps_wrapped_previews_on_the_ui_thread, projected_stamps_keep_their_previews_on_the_ui_thread}`,
  `jobs::tests::a_silent_job_runs_and_delivers_but_is_not_shown_in_the_status_bar`,
  `panels::tool_options::tests::a_relaxed_or_cut_wrap_says_so_under_the_stats_line`.
  The drag test: 10 Preview moves, one job in flight (`running_count` 1, no status-bar title), pixels and stats unchanged
  on the UI thread until a job lands, first job lands with `due_again`, the second (made for the 10th placement) equals
  the synchronous Preview bytes, the Final equals the synchronous Final.
- `cargo test --workspace --no-fail-fast` (debug, 03:01-03:08, `logs_b\test_ws.log`): wte_app **351 passed** / 16 ignored
  (base 342 + 9 new); wte_core lib 743 passed, **4 failed** (wa4_r8, wa20, wa22, wa24: package A's unmet plan limits,
  unchanged, same as check:A) / 17 ignored; brush_tips 15; end_to_end 9; project_roundtrip 23. **Total 1,141 passed,
  4 failed** (all 4 in package A's core tests; no core file was touched here).
- `cargo clippy --workspace --all-targets`: 0 warnings (`logs_b\clippy1.log`; rerun on the final source below).
- `cargo build --release -p wte_app`: ok (3 m 02 s incremental). Final exe
  `target\round8\work\pkg_b\target\release\wrapped_texture_editor.exe`, SHA-256
  `97002409bb112b2c8728a2b0df5418a33ddd8db4f97c032edfca689ec432c1e9`.
- Cold build of the copy: the copied `target` was reused; `test --workspace --no-run` recompiled wte_core and wte_app
  only (15 m 00 s, nothing else of ours running, free memory ~1.1 GB at the start).

## Scenarios (final exe, built-in sample, 1280x800; `runs_b\new`, `runs_b\sync`)
Reference `wte_round6_7.exe` shots: check:A's `runs_ca\ref` (same machine, this round; deterministic scenarios).
| scenario | ok | shot.png vs wte_round6_7.exe |
|---|---|---|
| stamp, text_warp, text_path, brush_types | true | identical (0 differing pixels) |
| final_bake_async | true | 6,674 pixels in x 794-977, y 623-664 (the toast area only: round 7's known exception) |
| wrap_surface | true | 3,517 pixels in x 375-1107, y 227-608 (the wrapped stamps; the same shot as package A's, 0 px vs `runs_ca\new`) |
| wrap_surface, `WTE_SYNC_FINAL_BAKES=1` | true | 0 px vs the asynchronous run; every logged composite equal (H0 cd272dc0..., H1 f3e5a387..., Overlap e4947f9d..., 6b 424bc2ed..., H8 / saved df0ab0b7...) |
- (2b) sphere, S 30 %: relaxed true, seams 0, stretch 4.2 %, rejected 0, flipped 0, cg_iters 161, **dup 0.000 %**
  (<= 1 %), **scatter 0.000 %** (<= 2 %), 10,952 bins.
- (6b) asynchronous: 8 Preview jobs started during the drag, 7 applied (all but the last `due_again`), 1 dropped (it
  landed after the release; the Final waited for it), 0 parameterizations on the UI thread by the app, nothing left;
  composite after the Final 424bc2ed5b3c53eb. Synchronous: 0 jobs, the same composite.
- Screenshots read (async run): `shot.png`: the reopened project, S selected on the sphere's right edge in Overlap +
  Jump small gaps, its 8 x 4 cells bent round the limb as one piece with the wrapped outline along them; the panel shows
  "Wrapped over 461 triangles" and no fitted / cut line (stretch 4.2 % < 5 %, seams 0); "WRAP AROUND" lies along the
  sphere below in one piece. `shot_wrapped.png` (step 2): one whole 8 x 4 cells picture on the sphere, rows straight
  and continuous, cells slightly larger towards the left and right edges (the sphere's curvature), no cut, no repeated
  cell. `shot_band.png` (step 4): the band round the sphere, ends meeting at the back, columns continuous across the
  front. `shot_text.png` (step 7): the wrapped Arc text in one piece along the surface.

## PATH runs (final exe, `runs_b\path`)
| model | Jump | relaxed | seams (S1..S5) | stretch | rejected | ink / picture | dup | scatter | param | Final applied |
|---|---|---|---|---|---|---|---|---|---|---|
| WaterBottle.glb | off | yes | 0 | 2.1 % | 0 | 1.0017 | 0.000 % | 0.000 % | 22.3 ms | 65 ms |
| WaterBottle.glb | on | yes | 0 | 2.1 % | 0 | 1.0017 | 0.000 % | 0.000 % | 21.3 ms | 33 ms |
| high_poly_sphere.obj | off | yes | 176 (0, 49, 0, 0, 127) | 34.9 % | 6.40 % | 1.0427 | 0.000 % | 0.103 % | 11,383 ms | 11,476 ms |
| high_poly_sphere.obj | on | yes | same | 34.9 % | 6.40 % | 1.0427 | 0.000 % | 0.103 % | 11,633 ms | 11,730 ms |
Step 10's assert (ink <= 1.3, dup <= 3 %, scatter <= 5 %) passes on both. high_poly_sphere's Final is over G6's 6 s
(package A's parameterization time, step 6 not built). Screenshots read:
- WaterBottle: one whole 8 x 4 cells picture on the bottle's body above the black holder, rows straight and continuous,
  columns narrowing evenly towards the sides as the body turns away; no cut, no repeated cell; the panel says
  "Wrapped over 482 triangles" (stretch 2.1 %: no fitted line).
- high_poly_sphere: one whole picture over the spiky ball, each cell once, rows continuous, every cell bent over its
  bumps (the cells' edges wave with them), the wrapped outline wavy around it; no cut line or scrap visible; the panel
  shows "Wrapped over 246559 triangles", "Fitted to the curved surface: stretched up to 35 %" and "Cut where it cannot
  lie flat (a corner, a handle, or where its edges meet)" (176 seam edges, not visible in the picture).

## ABeautifulGame `workflow_path`, indicative (final exe, `runs_b\abg`, 2026-11-22 03:13-03:16)
W = `WTE_STAMP_MODE=wrap WTE_FRAME_LOG=1`, P = `WTE_FRAME_LOG=1` (Project), one run each, run one after the other;
nothing else of ours ran (no cargo, no other scenario); other processes on the machine were not checked. Both
`"ok": true`. Frames from `round4\review\gate3\frames.py` and the per-frame lines between each drag's `Dyn(...)` step and
its checkpoint (frame #0 is the step's start: 242-246 ms in W and P alike, the runner's step gap).
| drag (the text stamp) | W frames | W interval median / max (excl. #0) | W ui cpu max | P frames | P interval median / max (excl. #0) | P ui cpu max |
|---|---|---|---|---|---|---|
| move | 49 | 16.6 / 23.2 ms | 6.5 ms | 14 | 16.9 / 22.9 ms | 21.1 ms |
| scale | 68 | 16.7 / 21.3 ms | 5.2 ms | 16 | 16.0 / 58.4 ms | 56.8 ms |
| rotate | 62 | 16.6 / 21.6 ms; one 873 ms gap (*) | 4.3 ms | 20 | 15.5 / 50.6 ms | 49.0 ms |
(*) The 873 ms interval in W rotate is not a stalled frame (ui cpu 4.3 ms): after the release no input came and no
frame was asked for while the Final ran on its worker; the frame came when it landed ("final bake: applied" in the same
frame). During the drags the W frames' UI work stays at 1.8-6.5 ms (P: up to 57 ms, its Preview bakes are on the UI
thread): **drag frame max W <= P** on all three drags. Per drag W started 1 Preview job (the drags last 5-8 frames,
shorter than one job), which landed after the release as stale; the picture stayed the last finished one meanwhile
(user answer Q4).
Finals (the core's `wrap:` / `final bake:` lines):
| Final | W applied (worker; param) | P applied (worker) |
|---|---|---|
| image stamp (placement) | 3,147 ms (3,102; param 2,936 ms, 14,210 verts, covered 13.4 %, seams 599, stretch 94.9 %) | 1,902 ms (1,851) |
| text stamp (placement) | 322 ms (313; param 302 ms) | 36 ms (17) |
| move | 633 ms (392; param 379 ms) | 58 ms (48) |
| scale | 964 ms (516; param 500 ms) | 103 ms (97) |
| rotate | 1,674 ms (1,156; param 511 ms) | 98 ms (87) |
G5c "Finals applied <= 1.5 x P" is **not met** in this indicative run (W 1.65 x P for the image, 6-17 x for the text's
move / scale / rotate): the wrapped Final's parameterization alone (300-510 ms on 2.5-3.6k laid vertices, relaxed,
cg_iters up to 1,800) is 3-10 x P's whole bake; step 6 (growth) is not built in the core. Of the W "applied" times
289 / 467 / 595 ms are the wait for the drag's stale Preview job (plan 4.1: the Final starts after its stamp's Preview
job lands). That wait only pays when the job's parameterization serves the released placement, which with the core as it
is never happens (request 1); here the jobs were made for the first drag frame, so even with request 1 they would not
serve. **For the planner / gate:** starting the Final at once when the job in flight was made for another placement would
cut those 0.3-0.6 s (at the price of two jobs running together for that time); not done, the plan's rule is binding.
Also for package A / the gate: the W image stamp at the first `stamp_spots` hit covered 13.4 % with 599 seam edges and
stretch 94.9 % (a piece's top, picture larger than the part, Jump off).
Screenshot `runs_b\abg\wf_wrap\shot.png` (read): the final Split view of the reopened project; in the 3D half the chess
set with the strokes and the fill, the wrapped image stamp as small magenta-pink patches on the red pieces at the left
front, the "WTE" text not distinguishable at this zoom; the 2D half shows the Chessboard set with the UV wire.


## Final state (after the last source change, the PATH step's Color collapse)
- `cargo clippy --workspace --all-targets`: **0 warnings** (`logs_b\clippy2.log`). `cargo test --workspace --no-run`: ok
  (`logs_b\build2.log`); the app test binary rerun: **351 passed**, 0 failed, 16 ignored (`logs_b\app_tests_final.log`);
  brush_tips 15, end_to_end 9, project_roundtrip 23 passed; the core test binary is unchanged (no core edit): its
  result of the workspace run stands (743 passed, the 4 package-A failures).
- Release exe SHA-256 `97002409bb112b2c8728a2b0df5418a33ddd8db4f97c032edfca689ec432c1e9`; every scenario above was run with it.
- Build lock taken for every heavy cargo command (label `round8-build-B ...`), never found held, never stale.

## Not done / not met (for check:B and the gate)
1. `cargo test --workspace` is not green: 4 core tests fail (wa4_r8, wa20, wa22, wa24), package A's unmet plan limits;
   no core file is mine to change. The app part is green.
2. Warm-up job not built (no core API: package A step 6 not built).
3. The Final waits for its stamp's Preview job but cannot reuse its parameterization (core request 1); on
   ABeautifulGame the wait costs 0.29-0.60 s per Final (see the ABeautifulGame section, a question for the planner).
4. No core build counter for "no parameterization on the UI thread" (core request 2); the tests use the proxies above.
5. G5c Finals on ABeautifulGame are 6-17 x P (core parameterization speed); G6 high_poly_sphere Final 11.5 s (> 6 s).
6. Not run here: `text_styles`, `roundtrip`, `export_run` (not in this package's done list; app changes do not touch
   their paths), `wrap_surface` on other sweep models; check:B races (Esc / undo / Final / open / a stroke during a Preview
   job) are covered by the unit tests named above except the stroke case (the held output path is not unit-tested).

## Check and fix (check:B, Opus, 2026-11-22 03:24-04:05)

Read: plan sections 0-2, 4, 6, 12; the builder's diff against `work\pkg_a` (9 app files + ARCHITECTURE 8.x / 9.2) and the
core job API it uses (`doc_stamps.rs` 3.4 functions, `history.rs` flush / cancel, `stamp_job.rs`). Logs
`target\round8\logs_cb\`, runs `target\round8\runs_cb\` (`runall.sh`).

### Races of plan 6 row 4: how each one behaves (all now covered by a test)
| race | behaviour | test |
|---|---|---|
| Esc while a Preview job runs | output Stale (cancel clears `preview_baked`), old pixels | builder's `esc_or_undo_while_...` |
| Esc after a Preview landed | the cancel restores the caches from before the drag | new `esc_after_a_landed_wrap_preview_restores_the_old_pixels` |
| undo while a Preview job runs | flush + undo, output Stale, old pixels; nothing left | builder's test |
| Final (release) during a Preview job | the Final waits for the job, then starts; now reuses its parameterization when it serves (request 1) | builder's `a_final_waits_...` + new reuse and no-frame asserts |
| open / New during a job | the generation check drops it | builder's `a_wrap_preview_of_another_document_is_dropped` |
| save during a Preview job | flush + `settle` bakes the Final on the UI thread without waiting for the job; the late output is Stale | new `save_during_a_wrap_preview_job_settles_without_waiting_for_it` |
| a brush stroke during a Preview job | `stroke_begin` flushes; the output is held while the stroke is open (nothing starts, nothing lands, no Final); after `stroke_end` it is dropped (Stale) with its parameterization cached, the Final starts and reuses it, pixels = synchronous Final | new `a_stroke_during_a_wrap_preview_job_holds_its_output_until_it_ends` (the builder's open item 6) |
| a stale Preview landing after a newer edit | release + a new drag before the job lands: the serial differs -> Stale, nothing changes; drag 2's own job then lands; its Final and the undo equal the synchronous run | new `a_stale_preview_landing_after_a_newer_edit_is_dropped_and_the_newer_edit_wins` |
| the stamp deleted while its job runs | `remove_stamp` flushes and drops it from `unbaked`: no Final waits; the output is Stale; undo of the delete brings the stamp back waiting for its Final, which lands with the synchronous pixels | new `a_stamp_deleted_while_its_preview_job_runs_is_left_alone_and_undo_brings_it_back` |
| a turn to Project inside the same pending edit while a wrap job runs | **bug found**: `apply_wrap_preview` applied the late wrapped picture over the projected Preview (it only checked that the stamp exists) and put the stamp back in `preview_stale`; fixed (core, below) -> Stale | new `a_wrap_preview_landing_after_a_turn_to_project_in_the_same_edit_is_dropped` |

### Fixes (this copy only)
1. **Core request 1 applied** (`doc_stamps.rs` `apply_wrap_preview`) as the builder wrote it, minus the redundant
   `stamp(id).is_some()` (covered by `wrapped_projector`): a Stale Preview output of the same mesh and maps leaves its
   parameterization in the cache when it serves the stamp's current placement (no pixel change; a serving
   parameterization equals a fresh one, WA27). The Final that waited for that job reuses it: the app tests see the same
   `WrapStats` (build time included) before and after the Final, and the core counter shows no build on the test
   thread. Safety: the Full job re-tests a cached parameterization with `serves_inputs` (current source size and texel
   size) before using it, so a stored entry can never bake wrong pixels.
2. **Core: a stamp no longer wrapped never takes a wrapped Preview** (same function): `live` now needs
   `wrapped_projector(id).is_some()` instead of `stamp(id).is_some()` (the last race above).
3. **Core request 2 applied** (`wrap_param.rs`): `pub mod param_counter` (thread-local; `bump()` is the first line of
   `wrap_param`), reachable as `wte_core::param_counter` through `pub use wrap_param::*`. The drag test now asserts it
   unchanged on the UI (test) thread over the ten Preview moves (plan 4.2 "core counter").
4. **App: no frame loop while a Final waits for its stamp's wrap job** (`stamp_bakes::needs_frame_after_dispatch`):
   before, every frame of the wait asked for the next one (`bakes.in_flight.is_none() && waiting`), so the CPU software
   renderer drew at the display rate beside the worker for the whole wait (0.27-0.58 s per Final on ABeautifulGame, up
   to a whole Preview job, ~11 s, on high_poly_sphere). Now the wait asks for no frame; the finishing job's
   `request_repaint` (`jobs.rs`) brings the frame whose `pump` starts the Final. Asserted in `a_final_waits_...`.
5. ARCHITECTURE.md: 5.20 (a Stale output keeps a serving parameterization, the not-wrapped rule, `param_counter`), 8.9
   (reuse; no frame while waiting), 9.2 (the new tests).
No other app change. Core files touched: `doc_stamps.rs` (one function) and `wrap_param.rs` (the counter) only.

### Tests and build (pkg_b, after the fixes)
- `cargo test --workspace --no-run` 03:29-03:47 (18 min; wte_core + wte_app rebuilt; nothing else of ours ran; free
  memory 0.03-1.1 GB, memory compression ~3 GB). `cargo test --workspace --no-fail-fast` 03:47-03:50
  (`logs_cb\test_ws1.log`): **wte_app 357 passed** / 16 ignored (351 + 6 new); **wte_core lib 743 passed, 4 failed**
  (wa4_r8, wa20, wa22, wa24: package A's unmet limits; messages identical to the builder's run apart from the timing:
  WA4 60 deg ink 1.1360, cells 0.785..1.599; WA20 ink 1.358, dup 14.8 %; WA22 dup 2.27 % "larger than the torus";
  WA24 ink 1.637) / 17 ignored; brush_tips 15, end_to_end 9, project_roundtrip 23. **Total 1,147 passed, 4 failed**
  (all 4 package A's).
- `cargo clippy --workspace --all-targets`: **0 warnings** (`logs_cb\clippy1.log`). `cargo build --release -p wte_app`:
  ok, 4 m 29 s (`logs_cb\release1.log`). Exe `work\pkg_b\target\release\wrapped_texture_editor.exe`, SHA-256
  `19d7844b4bdcd73ece7327fb3f677cd58695a8314ea80f42cb194cda97e54b2a`.
- Build lock taken for each heavy cargo command (label `round8-check-B`), never found held.

### Scenarios (new exe, 1280x800, `runs_cb\new`, `runs_cb\sync`; reference shots `runs_ca\ref` = wte_round6_7.exe)
| scenario | ok | shot.png vs wte_round6_7.exe |
|---|---|---|
| stamp, text_warp, text_path, text_styles, roundtrip, export_run, brush_types | true | identical (0 px) |
| export_run exports | - | `check_exports.py` PASS (sphere, 4 layers); all 7 files byte-identical to the reference's |
| final_bake_async | true | 6,674 px in x 794-977, y 623-664 (the toast area only: round 7's known exception) |
| wrap_surface | true | 3,517 px in x 375-1107, y 227-608 (the wrapped stamps); 0 px vs build:B's shot |
| wrap_surface, `WTE_SYNC_FINAL_BAKES=1` | true | 0 px vs the asynchronous run; composites equal (H0 cd272dc0..., H1 f3e5a387..., Overlap e4947f9d..., 6b 424bc2ed..., H8 df0ab0b7...) |
- (2b) relaxed true, seams 0, stretch 4.2 %, dup 0.000 % (<= 1 %), scatter 0.000 % (<= 2 %), 10,952 bins.
- (6b) asynchronous: 7 Preview jobs started, 7 applied (the last before the release, due again false), 0 dropped; 0
  UI-thread parameterizations; the Final's `wrap:` line says "(reused)"; composite 424bc2ed5b3c53eb. Synchronous: 0
  jobs, the same composite.
- Screenshots read: `shot.png`: the reopened project, S on the sphere's right limb (Overlap, Jump on), its cells bent
  round the limb in one piece with the outline along them; "WRAP AROUND" in one piece along the lower sphere; panel
  "Wrapped over 461 triangles", no fitted / cut line (stretch 4.2 % < 5 %, seams 0). `shot_wrapped.png`: one whole
  8 x 4 cells picture on the sphere, rows straight and continuous, columns slightly wider towards the edges, no cut, no
  repeated cell.

### PATH runs (`runs_cb\path`; step 10 does Jump off and on)
| model | Jump | ok | relaxed | seams (S1..S5) | stretch | ink / picture | dup | scatter | param | Final applied |
|---|---|---|---|---|---|---|---|---|---|---|
| WaterBottle.glb | off | true | yes | 0 | 2.1 % | 1.0017 | 0.000 % | 0.000 % | 25.9 ms | 64 ms |
| WaterBottle.glb | on | true | yes | 0 | 2.1 % | 1.0017 | 0.000 % | 0.000 % | 22.8 ms | 34 ms |
| ABeautifulGame.glb | off | - | yes | 213 (213,0,0,0,0) | 130.3 % | 0.0591 (covered 5.3 %, 0 inked bins) | 0.000 % | 0.000 % | 3,443 ms | 3,634 ms |
| ABeautifulGame.glb | on | **false** | yes | 2,235 (0,823,52,7,1353) | 97.2 % | **1.3045** (> 1.3) | **10.505 %** (> 3 %) | 1.130 % | 12,977 ms (160,593 verts, 30 jumps, rejected 26.3 %, flipped 49,438) | 14,107 ms |
| high_poly_sphere.obj | off / on | true | yes | 176 (0,49,0,0,127) | 34.9 % | 1.0427 | 0.000 % | 0.103 % | 11,325 / 11,521 ms | 11,426 / 11,609 ms |
- The WaterBottle and high_poly_sphere shots are pixel-identical to build:B's. Read: WaterBottle: one whole 8 x 4 cells
  picture on the body above the holder, rows straight and continuous, columns narrowing evenly to the sides, no cut,
  no repeat; panel "Wrapped over 482 triangles". high_poly_sphere: one whole picture over the spiky ball, each cell
  once, cell edges waving with the bumps, wavy outline, no visible cut or scrap; the panel shows "Fitted to the curved
  surface: stretched up to 35 %" and "Cut where it cannot lie flat ...".
- **ABeautifulGame (Jump on) fails step 10's plan limits: ink 1.3045 > 1.3, dup 10.5 % > 3 %** (`shot.json` ok false,
  only this assert). Shot read (`runs_cb\path\abg\shot.png`, enlarged `zoom_cb.png`): the cells picture lies on the
  board inside the outline, rows continuous between the pieces, but **the pawns and the bases of pieces standing in and
  beside the picture carry colour blobs from the picture** (purple / lime / cyan / orange, also on pieces right of the
  footprint): picture spots appear twice. Panel: "Wrapped over 323172 triangles", "Fitted ... 97 %", "Cut where ...",
  "The ends meet at the back", "Continued onto 30 separate parts". This is the core's Jump footprint / owner rule (plan
  2.2-5; WA24 fails the same way in the core tests), package A's; no app path is involved (the Final's
  parameterization is fresh). With Jump off the stamp at that spot covers only 5.3 % of the picture (0 inked bins at
  alpha >= 128). For package A and the gate's sweep (G6 limits).
- G6: high_poly_sphere Final 11.4-11.6 s (limit 6 s); ABeautifulGame Jump on 14.1 s: core parameterization speed.

### ABeautifulGame `workflow_path` in Wrap after the fixes (one run, `runs_cb\abg_wf_wrap`, ok true; indicative; nothing
else of ours ran)
Finals applied: image 2,695 ms (param 2,528 ms, fresh); text 302 ms; move 583 ms (waited 267 ms for the drag's stale
Preview job); scale 948 ms (waited 432 ms); rotate 1,049 ms (waited 580 ms). Every Final's parameterization is fresh:
the drag's one job was made for the first drag frame and does not serve the released placement, so request 1 cannot
help here (it does in the sphere sample's 6b and in the tests, where the job was made for the released placement).

### Not done / open (for the gate and the planner)
1. `cargo test --workspace` is not green: the same 4 core tests of package A (wa4_r8, wa20, wa22, wa24).
2. wrap_surface on ABeautifulGame (Jump on) fails the plan's sweep limits (ink 1.3045, dup 10.5 %): core, package A.
3. **For the planner**: the binding rule "a Final waits for its stamp's Preview job in flight" costs the rest of that job
   when it was made for another placement (ABeautifulGame 0.27-0.58 s per Final here; on high_poly_sphere a drag that
   starts an ~11 s job and is released after 1 s would wait ~10 s, then build another ~11 s). "Wait only when the job
   was made for the stamp's current placement (`WrapFlight::placement`), else start the Final at once" would keep the
   reuse and remove these waits, at the price of two jobs running together for the rest of the stale one. Not changed:
   the plan's rule is binding. Also `final_waits_for_wrap_job` holds every Final, not only that stamp's (one Final runs
   at a time anyway; a multi-stamp edit is rare).
4. Warm-up job: not built (no core API; package A step 6 not built). G5c (Finals <= 1.5 x P) and G6 not met (core
   speed).
5. Not run by me: the other sweep models, gate 2 timing lines.
