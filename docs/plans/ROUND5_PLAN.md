# Round 5 plan — Wrapped Texture Editor ("smoothness")

Date: 2026-11-20. Source = the main tree `E:\claude_programs\wrapped_texture_editor` (round 4's final source,
exe `target\round4\wte_round4.exe`). `ARCHITECTURE.md` is the binding contract; this plan says which of its
paragraphs change. All file:line references are to the main tree as it is today.

Three items, two packages, one gate:

| Item | What | Package |
|---|---|---|
| R5-1 | Final stamp / text bakes off the UI thread | A |
| R5-2 | Brush lag on very heavy models | B |
| R5-3 | The font picker's slow first open | B (small item) |

Left out (as ordered): classic text painter 3-5 %, P1 (c), P13 jitter, F20, wording, FBX IMPORTER_VERSION.

---

## 0. Facts every agent needs

- Build from the root of your copy: `cmd /c "E:\claude_programs\wrapped_texture_editor\cargo.cmd <cmd> --offline 2>&1"`
  (`test --workspace`, `clippy --workspace --all-targets`, `build --release -p wte_app`). Never edit `cargo.cmd` or
  `.cargo\config.toml`. No new crates (offline): everything below uses what is already in `Cargo.lock`.
- The machine is slow (6 cores, 7 GB RAM, USB disk; a cold build of one copy is about 20 minutes). Run one
  measurement at a time, on an idle machine, and few of them. Do not scan `target\` with recursive finds: it
  stalls the disk for minutes and ruins any timing that runs meanwhile (it happened while this plan was written).
- Never use OS-level mouse or keyboard injection; drive the app only through the headless self-test runner:
  `<exe> [PATH] --screenshot <out.png> --scenario <name> --size 1280x800`. Never kill a running
  `wrapped_texture_editor.exe`. Never run downloaded binaries. Large outputs go to E: only
  (`target\round5\...` in the main tree; tests use `wte_core::testutil::temp_dir`).
- `WTE_FRAME_LOG=1` gives one `frame timing:` line per frame (interval, ui cpu, every segment that ran).
- Measuring tools that still exist (the `runs_*` folders are gone): `target\round4\review\gate3\` (`btp.py`,
  `frames.py`, `wf_steps.py`, `perf_driver.ps1`, `core_benches.ps1`), `target\round4\perf\p0_tools\`
  (`run_one.ps1`, `analyze.py`), `target\round3_audit\` (`sweep.ps1`, `slow.py`, `quarters.py`),
  `target\round3\shot_diff.py`, `target\round3\check_exports.py`. Reference exes: `target\round4\wte_round4.exe`,
  `target\round4\perf\wte_p0_instrumented.exe` (round-3 code with stroke counters), `target\round3\wte_round3.exe`.
- Models: `E:\claude_downloads\glb_samples\ABeautifulGame.glb` (1.5 M triangles, 15 sets),
  `target\compat\harness\shapes\ngons_tiling.obj`, `target\compat\topologies\models\tex_8192\tex_8192.obj`,
  `...\tex_4096\tex_4096.obj`.
- Lean process: one builder, then one check-and-fix agent per package. Each package works in its own copy
  (`target\round5\work\pkg_a`, `pkg_b`), made from the main tree without `target\`; the gate merges with a
  three-way merge against that base. Notes go to `target\round5\notes\pkg_a.md` / `pkg_b.md`.

---

## 1. R5-1 — Final stamp bakes off the UI thread (package A)

### 1.1 Cause

Every path that ends a stamp edit bakes at Full quality on the calling thread, which is the UI thread:

- `Document::add_stamp` renders the text in full and calls `bake_and_apply(id, BakeQuality::Full)`
  (`crates\wte_core\src\doc_stamps.rs:325`, `:342`); called by the Stamp / Text tool's click
  (`crates\wte_app\src\tools\stamp_place.rs:477`).
- `Commit::Final` of `set_stamp_placement` / `set_stamp_source` / `set_stamp_tint` ends in `preview_edit`
  (`doc_stamps.rs:887-905`), which calls `flush_pending_edit()` (`crates\wte_core\src\history.rs:616-628`), which
  calls `finalize_stamp_bake(id)` for every preview-baked stamp (`doc_stamps.rs:656-666`): `retune_text`, then
  `bake_and_apply(id, Full)` (`doc_stamps.rs:1030-1047`: text picture in full through `text_source`
  `:952-963`, then `bake_projected` / `bake_uv`), then `remove_empty_linked`. Callers: the Transform tool's
  release (`tools\transform.rs:235`, `:452`, `:496`) and the panel actions (`dispatch.rs:772-779`).
- Every other command, undo / redo (`history.rs:524`, `:541`), save and export (`io_flows.rs:451`, `:476`),
  the export dialog, Preview mode (`dispatch.rs:460`, `:472`, `:618`) flush a pending edit the same way.

Round 3 removed the doubled bake (R3-4); the one Full bake that is left is the freeze: 270-285 ms per placement
or release on ngons_tiling, 0.9-1.4 s on ABeautifulGame (`ROUND3_REPORT.md` "Unresolved" 1), warp Finals
641-2,618 ms (`GATE_REPORT.md` part 2), a whole-model Circle text 3.4-4.9 s in one frame (part 9 item 5).

What makes the move possible: a Full bake is a pure function of data that is already shared by `Arc` —
`mesh`, `bvh` (`document.rs:35-36`), the texel maps (`Document::texel_maps`, `document.rs:159`), the decoded
image mips (`image_source`, `doc_stamps.rs:938`), parsed fonts (`FontCache`, `text\font_catalog.rs:489-514`:
`Arc<fontdue::Font>`) and font bytes (`Arc<[u8]>` in the blob store, `with_text_fonts` `doc_stamps.rs:784`).
`bake_projected` is documented as independent of the thread count (`stamp_bake.rs:1205-1210`).

### 1.2 Design in one paragraph

A new document mode, **deferred Final bakes** (off by default, as `set_defer_preview_bakes` is; the app turns it
on). While on, ending a stamp edit does the cheap, history-visible part at once (text raster size and anchor,
`retune_text`: frames only) and marks the stamp **unbaked** instead of baking it. The caches keep what they
show (the Preview). The app asks the document for a **bake job** (owned inputs), runs it on a worker, and hands
the **output** back; the document applies it only if the stamp is still the same object and still unbaked,
otherwise the output is dropped. "Unbaked" is part of the structure state: a history snapshot carries the set
of unbaked stamps, so undo / redo never wait and never bake on the UI thread — a restored state whose stamp
never got its Full bake is simply baked in the background after the restore. Only the commands that turn stamp
pixels into something permanent (save, export, merge down, rasterize) **settle** first: wait for the bake in
flight and bake what else is waiting. The synchronous path and the worker run the same code, so pixels are
bit-identical.

### 1.3 Core changes (`wte_core`)

New public items (exported from `lib.rs`; types in `doc_stamps.rs` or a new `stamp_job.rs`, the builder's choice):

```rust
/// Owned inputs of one Full bake of one stamp. Send + 'static.
pub struct StampBakeJob { /* id, object: StampObject (source, placement, tint, home_set), mesh: Arc<MeshData>,
    bvh: Arc<Bvh>, maps: Vec<Arc<TexelMap>>, picture: Image(Arc<PremulMips>) | TextCached(Arc<PremulMips>)
    | TextDraw { spec, own: Option<(Arc<fontdue::Font>, Arc<[u8]>, u32)>, fallback: Arc<fontdue::Font> } */ }
impl StampBakeJob {
    pub fn stamp(&self) -> StampId;
    pub fn is_text(&self) -> bool;
    pub fn run(self) -> StampBakeOutput;            // on the calling thread's rayon pool (the synchronous path)
    pub fn run_isolated(self) -> StampBakeOutput;   // inside the core's bake pool (below); what the app's worker calls
}
/// Result of a job. Send + 'static. Carries the job's identity (id, object, map Arcs), the baked buffers
/// `Vec<(TextureSetId, SparseBuf)>` or the error, and for a drawn text its picture (spec, mips, frame, full = true).
pub struct StampBakeOutput { /* ... */ }
pub enum FinalBake { Applied { id: StampId, empty: bool }, Failed { id: StampId }, Stale, Busy(Box<StampBakeOutput>) }

impl Document {
    pub fn set_defer_final_bakes(&mut self, on: bool);          // turning it off calls settle_final_bakes()
    pub fn has_unbaked_stamps(&self) -> bool;                   // an id in `unbaked` that is not in `preview_baked`
    pub fn stamp_is_unbaked(&self, id: StampId) -> bool;
    /// The job of the lowest unbaked id that is not in `preview_baked`; None when there is none or a stroke is open.
    /// A stamp whose inputs cannot be prepared is logged, taken out of `unbaked` and skipped.
    pub fn next_final_bake(&mut self) -> Option<StampBakeJob>;
    pub fn apply_final_bake(&mut self, out: StampBakeOutput) -> FinalBake;
    /// Bakes every unbaked stamp on the calling thread (next_final_bake + run + apply until none is left).
    pub fn settle_final_bakes(&mut self);
}
```

Rules:

1. **One code path.** `bake_and_apply(id, Full)` becomes "make the job, `run()`, apply the output without the
   staleness check". `finalize_stamp_bake` (mode off), `add_stamp` (mode off), `rebake_stamp`, the project load
   and `settle_final_bakes` all go through it. The Preview-quality path keeps its direct code or takes a quality
   field; either way no pixel changes.
2. **Job making (UI thread, cheap).** Image: `image_source(blob)` (cached decode). Text: if `text_mips[id]`
   holds the full picture of the spec (`text_source`'s test, `doc_stamps.rs:954-959`) the job carries those
   mips; otherwise it carries the spec and the fonts (`FontCache::get` / `fallback`, as `with_text_fonts`) and
   the worker draws the text (`render_text(.., RasterQuality::Full)`) and builds the mips.
   `raster_counter::bump()` runs on the thread that draws (the tests that read it run in mode off).
3. **`run_isolated`.** A `OnceLock<rayon::ThreadPool>` in the core with
   `max(1, available_parallelism - 1)` threads named `stamp-bake-N`; `pool.install(|| self.run())`. The bake's
   parallel loops then do not queue the UI thread's own loops (compositing) behind them. `run()` itself never
   creates a pool.
4. **Ending an edit, mode on.** `finalize_stamp_bake(id)` does: `retune_text(id)`; `unbaked.insert(id)`;
   `preview_baked.remove`, `preview_stale.remove`. No bake, no `remove_empty_linked`. `flush_pending_edit` then
   pushes the history entry as today. `Commit::Final`, undo, another command: all the same, none bakes.
5. **`add_stamp`, mode on.** Validation as today. Text: sanitize; reuse the hover picture when the T7 rule allows
   (`doc_stamps.rs:320-326`), else take the fitted spec from `fit_frame` (geometry only, the same too-large
   fallback) and store only its frame. Insert the stamp object, `retune_text`, insert the linked layer of
   `home_set` with an empty cache (`insert_linked_layer`), make it active, `unbaked.insert(id)`, push
   "Add Stamp" / "Add Text". Layers of other covered sets appear when the bake is applied.
6. **Applying.** `apply_final_bake` returns `Busy(out)` while a stroke is open (the app keeps it and tries again).
   The output is used only when all of this holds: the stamp exists, `*stamp == out.object`, `id` is in
   `unbaked` and not in `preview_baked`, and mesh and every texel map are the same `Arc`s (`Arc::ptr_eq`);
   if not, the result is `Stale` and nothing changes. When it is used: store the text picture
   (`store_text_picture`), `apply_bake`, `remove_empty_linked`, `unbaked.remove(id)`, `touch()` (the revision
   moves so thumbnails refresh; `history_state()` and `is_modified()` do not change), `debug_check_structure`.
   An output that carries an error: warning in the log, `unbaked.remove(id)`, `Failed` (the Preview stays, as
   the "final bake failed" branch does today). A lock does not stop it (it is not a user edit).
7. **Structure snapshots carry the flag.** `StructureSnapshot` gets `pub unbaked: BTreeSet<StampId>`.
   `snapshot_structure` stores `self.unbaked`; `restore_structure` swaps it with the document's (ids of stamps
   that no longer exist are dropped) and returns the old set in the inverse snapshot. So undo, redo and
   `cancel_pending_edit` restore "this stamp still needs its Full bake" together with the caches it belongs to.
8. **Commands that keep stamp pixels.** `merge_down` and `rasterize_layer` (`doc_layers.rs:177`, `:247`): after
   their `flush_pending_edit()`, if the layer's stamp is unbaked, bake that stamp synchronously (rule 1) before
   reading its cache. `duplicate_stamp`: the copy shares the caches, so it is unbaked exactly when the original
   is. `remove_stamp` and the removal of a stamp's last layer (`doc_layers.rs:283`): `unbaked.remove`.
9. **Invariants** (`debug_check_structure`, `document.rs:377`): `unbaked` holds only ids of existing stamps; it is
   empty while the mode is off; "outside a pending edit `preview_baked` is empty" stays.
10. `Document::snapshot()` does not change. Its callers settle first (1.4). Project load is untouched (the loaded
    document is in mode off on the load worker; `unbaked` starts empty in `doc_build.rs:354` and `from_parts`).

### 1.4 App changes (`wte_app`)

- **`state.rs`**: `AppState::async_final_bakes: bool` — `false` from `AppState::new` (so the 318 app tests and
  `WteApp::for_tests` keep today's synchronous behaviour), set `true` by the real `WteApp::new` unless the
  environment has `WTE_SYNC_FINAL_BAKES=1` (measuring and A/B aid). `AppState::final_bakes_pending: bool`
  (mirror, for the runner and the tests). `AppState::await_first_bake: Option<StampId>`.
- **`io_flows.rs` `install_document`** (`:529`): `doc.set_defer_final_bakes(app.state.async_final_bakes)`.
- **`jobs.rs`**: `JobOutput::StampBake { generation: u64, output: Box<StampBakeOutput> }`;
  `Jobs::wait_for(&mut self, title: &str) -> Option<JobOutput>` (removes the running job with that title and
  blocks on its receiver; used only by `settle`).
- **New `stamp_bakes.rs`** (one bake in flight at a time):
  - `pump(app)`, called from `App::logic` right after the job poll (`app.rs:274-277`): apply a held output if no
    stroke is open; if no bake is in flight and a session exists, `doc.next_final_bake()` and
    `app.jobs.spawn(ctx, "Finishing stamp" | "Finishing text", false, move |_| JobOutput::StampBake { generation,
    output: Box::new(job.run_isolated()) })`; set `final_bakes_pending = in flight || held ||
    doc.has_unbaked_stamps()`. Bakes start also while a pending edit is open on another stamp (only the stamps
    being edited are skipped); none starts while a stroke is open.
  - `on_output(app, generation, output)`, called from `io_flows::on_job_output`: dropped when `generation !=
    session.scene_generation`; else `apply_final_bake`: `Applied { id, empty }` → if `await_first_bake ==
    Some(id)` and `empty`, the "covers no texels" warning of `stamp_place.rs:481-485` (moved here for the async
    case); `Busy(out)` → held; `Stale` / `Failed` → log. Log lines (the gate greps them):
    `final bake: stamp N started (text|image)`, `final bake: stamp N applied after T ms (worker W ms, empty: b)`,
    `final bake: stamp N result dropped (stale)`, `final bake: settle waited T ms`.
  - `settle(app)`: if a bake is in flight, `jobs.wait_for(..)` and apply it; then `doc.settle_final_bakes()`.
    Called at the top of `start_save` and `start_export` (`io_flows.rs:442`, `:471`, before `snapshot()`), in
    the `ShowExport` arm (`dispatch.rs:457`), and before `MergeDown` / `RasterizeLayer` / `RasterizeActiveLayer`
    (`dispatch.rs:718-719`). After it `debug_assert!(!doc.has_unbaked_stamps())`.
- **`tools\stamp_place.rs:477-488`**: when `doc.stamp_is_unbaked(id)`, set `await_first_bake` instead of
  testing `bake_is_empty` now. The tool still selects the stamp and switches to Transform at once: the gizmo
  frame shows where the decal will appear, and the status bar shows "Finishing stamp" (the status bar already
  draws a non-modal job, `panels\status_bar.rs:148`).
- **`selftest\mod.rs`**: scenarios stay deterministic because the runner treats waiting bakes as part of the
  step: `Active::Input` also waits while `state.final_bakes_pending` (`:348-353`), a `Do` step is followed by
  the same wait before the next step starts, and `is_idle` (`:578`) includes `!final_bakes_pending`. New
  `Step::BakeWait(bool)` switches that wait off and on (default on) for the scenario below.
- Unchanged call sites: the Transform tool, the text panels and `dispatch` keep calling `set_stamp_*(..,
  Commit::Final)`; the mode does the rest. `view::bake_stale_previews` (Preview throttle) is unchanged.
- Optional, only if the frame log shows a mip rebuild hitch after a release: treat `has_unbaked_stamps()` like
  `has_pending_edit()` in `gpu\scene.rs:329`.

### 1.5 What happens while a bake is pending (the decisions)

| Event | Behaviour |
|---|---|
| The same stamp is edited again (drag, typing, slider) | Nothing waits. The Preview edits run as today; the bake in flight finishes and is dropped (stale). The next Final marks it unbaked again and a new bake starts. |
| Undo / Redo / Esc on a pending edit | Never wait, never bake on the UI thread. The restored snapshot brings its own unbaked set. A result that matches the now-current stamp object is used; any other is dropped. Stamps the restored state lists as unbaked are baked in the background. |
| Stamp deleted | Flag removed; the result is dropped. |
| Duplicate stamp | The copy is unbaked too and gets its own bake. |
| Save / Save As / close → Save | **Settle** (wait for the bake in flight, bake the rest on the UI thread), then snapshot. A saved project never holds a Preview cache. |
| Export dialog / Export | Settle first. |
| Merge down / Rasterize of that stamp's layer | Settle first (the core also guarantees it, rule 8). |
| Brush stroke or fill started | Not blocked. A result that arrives during an open stroke is held and applied when the stroke ends. |
| Preview view mode, tool changes, camera | Nothing waits; the picture swaps when the bake lands. |
| New / Open / Close without saving | The result is dropped (generation and `Arc` identity); the worker ends on its own; exit does not wait. |
| Bake fails (font gone, text too large to draw) | Warning in the log, the Preview (or the empty layer of a new stamp) stays, flag cleared. |

Settle blocks the UI thread for at most the rest of one bake plus the Full bakes of other waiting stamps — never
more than round 4 spent on the UI thread for the same edits.

### 1.6 ARCHITECTURE.md changes (package A writes them)

- **4.6 Threading**, bullets 4 and 5 become:
  - "Long operations (import, document build, project load/save, export, system-font scan, and — with deferred
    Final bakes on, 5.20 — the Full bake of a stamp or text after a placement or a Final edit) run on
    `std::thread` workers started by `wte_app::jobs`; they receive owned inputs (`DocumentSnapshot`, paths,
    `StampBakeJob`) and report through `&dyn Progress` (a stamp bake reports nothing)."
  - "The UI thread never blocks on a worker, with one exception: `stamp_bakes::settle` waits for the stamp bake
    in flight and bakes the other waiting stamps itself before save, export, the export dialog, merge down and
    rasterize, because these keep final stamp pixels. Workers never touch egui except
    `Context::request_repaint()`."
  - new bullet: "A stamp bake job runs its data-parallel loops in the core's own rayon pool
    (`available_parallelism - 1` threads, at least 1), not the global pool."
  - bullet 2 gains: "`StampBakeJob`, `StampBakeOutput`: `Send`."
- **5.14**: the `unbaked` and `defer_final_bakes` fields. **5.18**: `StructureSnapshot::unbaked`; the doc
  comments of `flush_pending_edit` / `cancel_pending_edit`; **6.7**: one paragraph "unbaked stamps in snapshots"
  (rule 7).
- **5.20**: the API block of 1.3 and a paragraph "Deferred Final bakes" (rules 1-10). The sentence "**every**
  path that ends a pending edit … leaves Full-quality caches; a Preview-quality cache can never reach history, a
  saved project or an export" becomes: "With deferred Final bakes off, every path that ends a pending edit
  leaves Full-quality caches. With them on, it leaves the stamp unbaked: its cache may be in a history snapshot
  only together with its flag, and never reaches a saved project, an export, a merged or a rasterized layer
  (settle, rule 8 and 8.9)." The `add_stamp` comment gains the mode-on behaviour (rule 5).
- **8.6** Stamp / Text row: the "covers no texels" toast comes when the first bake lands. **8.9**:
  `JobOutput::StampBake`, `Jobs::wait_for`, `stamp_bakes.rs`. **8.11**: the runner's bake wait and
  `Step::BakeWait`. **1.x**: a short "Round 5" paragraph.

### 1.7 Tests package A adds

Core (`doc_stamps.rs` / `history.rs` tests; a helper `pump(doc)` = `next_final_bake` → `run()` → `apply` until none):
1. `deferred_final_bakes_equal_the_synchronous_ones`: the same edit list on two documents (mode off / mode on +
   pump): image stamp projected on a two-set model, UV stamp, plain text, styled text (Arc warp, Circle path,
   thickening), placement / source / tint Finals, duplicate, undo, redo. Equal: stamps, layer order, names and
   kinds, every cache byte for byte (layer ids may differ).
2. Stale results: job taken, then the stamp is moved / deleted / the edit undone → `Stale`, document unchanged.
3. Undo restores the flag: Final (not pumped), second Final, undo → the stamp is unbaked with the first
   placement; pump → caches equal the synchronous document after the first Final.
4. `merge_down` / `rasterize_layer` of an unbaked stamp give the pixels of the synchronous document.
5. `apply_final_bake` during an open stroke returns `Busy`; after `stroke_end` it applies.
6. `add_stamp` mode on: one history entry, home layer exists and is active, other sets' layers appear on apply,
   the empty ones never; `undo` before the apply removes everything and the late result is `Stale`.
7. The round-4 hash tests (8 layout hashes, `plain_text_is_bit_identical`, stamp_bake tests) pass unchanged.

App: `stamp_bakes` tests with a state in async mode (start, apply, drop on new session, settle before save and
merge, the empty-bake toast); the runner's wait (`is_idle`).

Scenario `final_bake_async` (PATH or built-in sample; `scenario_names().len()` becomes 38): with the wait on,
place an image stamp, drag it, record the composite hash H1; undo. Then with `BakeWait(false)`:
(a) the same drag, `Undo` in the next frame, `Redo`, wait → hash == H1; (b) drag, at once a second drag, wait,
undo, wait → hash == H1; (c) a text stamp gets Arc warp (Final) and `Save` in the next frame, reopen → the
reopened composite equals the in-memory one after the wait; (d) Final then `MergeDown` at once → the merged
layer equals the one of the same steps with the wait on; (e) Final then delete at once → no layer comes back.
The run must log at least one `result dropped (stale)` and one `settle waited`.

### 1.8 Acceptance (measured by the gate; release build, idle machine, 1280 x 800, `WTE_FRAME_LOG=1`)

| # | Line | How |
|---|---|---|
| A1 | Tests of 1.7 pass; all round-4 tests pass unchanged; clippy clean | `cargo test --workspace`, clippy |
| A2 | Pixels identical to round 4 | `stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run` on the built-in sample: `shot.png` equal to `wte_round4.exe`'s (`shot_diff.py`; toast area excepted), exported files byte-identical; `final_bake_async` passes with and without `WTE_SYNC_FINAL_BAKES=1` |
| A3 | ABeautifulGame `workflow_path`, steps "image stamp", "text stamp", "transform the stamp": between each `final bake: … started` and its `applied` line no frame interval over **300 ms** (round 4: one frame of 1,400-2,600 ms); the release / edit frame itself has `dispatch` + `central` ≤ **60 ms** (a throttled Preview bake that was due in the same frame is reported separately). The click frame of the first image placement is reported separately (image decode, depth fit) | `frames.py` on the log, 2 runs |
| A4 | Same on ngons_tiling: no frame over **150 ms** in those steps (round 4: 270-285 ms bake frames) | 2 runs |
| A5 | Time from Final to swap (`applied after T ms`) ≤ **1.3 x** the Final frame of `wte_round4.exe` in the same step, back to back (the bake has one thread fewer) | same runs + 2 runs of the round-4 exe |
| A6 | `text_warp` on ABeautifulGame, 2 runs: 0 frames over 300 ms around the 20 Finals (round 4: up to 2,618 ms) | `frames.py` |
| A7 | Fuzz seed 7, 1,000 steps, ngons_tiling: `central` / `dispatch` frames over 500 ms ≤ **2**, none in a stamp or text step (round 4: 5 / 8, worst 1,011 / 2,149 ms); 0 "Invalid parameter" / "Unknown layer" / "panicked" | `slow.py <log> 500`, 2 runs |
| A8 | Workflow step totals for the three stamp steps ≤ 1.25 x round 4 (the runner now waits for the swap) | `wf_steps.py` |

If A3 / A6 fail only because compositing queues behind the bake, lower the bake pool by one thread before
anything else and report both measurements.

### 1.9 Files (package A)

Core: `doc_stamps.rs`, `history.rs` (not the `TileBackup` block, lines 147-260), `document.rs`, `doc_build.rs`,
`doc_layers.rs`, `lib.rs`, `project\load.rs` only if `from_parts` needs the new field, optional new
`stamp_job.rs`. App: `jobs.rs`, new `stamp_bakes.rs`, `main.rs` (mod line), `app.rs`, `io_flows.rs`,
`dispatch.rs`, `state.rs`, `tools\stamp_place.rs`, `selftest\mod.rs`, `selftest\scenarios.rs`, optional
`gpu\scene.rs`. `ARCHITECTURE.md` sections 1.x, 4.6, 5.14, 5.18, 5.20, 6.7, 8.6 (Stamp / Text row), 8.9, 8.11.
`stamp_bake.rs` is not edited (the bake itself does not change).

---

## 2. R5-2 — Brush lag on very heavy models (package B)

### 2.1 What the numbers really say (found while planning)

**The P7 miss ("1.8-1.9 x per walk step") is not a per-step slowdown. It compares two different step sets.**

- The walk is the same code in both rounds: steps of `max(1 pt, ring / 4)`, one raycast and one `stroke_to_3d`
  per step — round 3 `walk_3d` (`target\round3\work\merged\crates\wte_app\src\tools\brush.rs:150-180`), round 4
  `resolve` + `paint` (`crates\wte_app\src\tools\brush.rs:487-556`). Round 4 only queues the hits and paints
  them under the time rule.
- Checked today with the existing exes on ABeautifulGame `workflow_path` (Size 78, the same three strokes: grid
  row 16 with 27 hit points and sets {0, 2, 4, 7, 8, 9, 11, 13}, and the two after it): the round-3 code
  (`wte_p0_instrumented.exe`) walks **234 / 126 / 131** steps after the press, round 4 logs **235 / 127 / 132**
  samples (the press included). Identical.
- The gate's P7 row divided round 4's `brush_types_path` stroke (235 samples, 1,508 / 1,620 ms) by P0's Size-50
  baseline stroke (377 steps, 1,322 / 1,409 ms; `perf\baseline.txt` section 6). 377 / 235 = 1.60 of the
  "1.8-1.9 x" is the step count; what is left is the **1.14 x per stroke** the gate also printed — from
  different runs, not back to back, and not the same steps. Open point: the round-3 code walks 377 steps at
  Size 50 and 234 at Size 78 (ratio = the size ratio), but round 4's `brush_types_path` stroke logged 235
  samples at Size 50. Either that scenario's view differs from `workflow_path`'s, or the round-4 walk does not
  follow the size there; B-1 settles it (the new log fields `size` and `ring` show it at once).
- Per dab the core is not slower (P3: 0.5697 ms against 0.6061 ms, `GATE_REPORT.md` part 2).
- Per stroke, like for like (one run pair today, Size 78, stroke work in ms; another build was running on the
  machine, so these are indicative only): round-3 code 4,708 / 415 / 2,317, round 4 2,462 / 385 / 1,951.
  No sign that round 4 is slower per stroke. (Stroke 1 of a run includes the first use of 8 sets: one sample of
  471 ms in round 4. Stroke 3 at Size 78 ended 1,110 ms after the release.)

**Where the time of a stroke on this model is** (`target\round4\runs_gate3\btp_analysis.txt`, lines 19-60): of 39
drag frames about 27 are cheap (stroke work 13-15 ms: the 10-ms soft budget plus one sample) and about 12, in two
bursts, cost 100-213 ms each; the burst frames carry about three quarters of the 1.5-1.7 s of stroke work.
The likely reason (to be confirmed by the probe of B-2; it was not measured): the ball's world radius is fixed
at the press from the texel size of the pressed set (`tools\brush.rs:661`), so on a set with finer texels every
dab covers many more texels, and a one-point screen step that jumps between surfaces is bridged with dabs
along up to 6 radii (`stroke.rs:36`, `:246-275`). That is the backlog (P8d): work arrives faster than one
thread paints it. While it is behind,
round 4 already spends about 93 % of the time painting (frames of 100 ms + one sample), so **pacing cannot shrink
the backlog; only cheaper dabs can**. And throttling the input is not something a real mouse does — in the
self-test it would only make the measured stroke slower. The plan therefore does not fake P8d through the
runner.

**What can be improved without changing a pixel:**

1. Frames while catching up are "100 ms + one sample", and one sample can be 88-132 ms (207 ms at Size 78
   today; a walk step paints every dab of its segment in one call). The unit of pacing should be the **dab**,
   not the walk step.
2. Cheaper dabs, but only where a measurement shows the time is. The phase counters exist only in test builds
   (`brush::probe`, `brush.rs:1255-1285`).

### 2.2 Design

**B-1 Like-for-like check first (no code, about 5 minutes).** Idle machine (no build running), one process at
a time, alternating, 2 runs each: `run_one.ps1 -Exe wte_p0_instrumented.exe -Scenario workflow_path -EnvList
"WTE_P0_FRAMES=1"` and `-Exe wte_round4.exe … -EnvList "WTE_FRAME_LOG=1"` on ABeautifulGame. Compare per
stroke: P0 `analyze.py` "totals ms: … stroke X" and "steps N" against round 4's `brush stroke: N samples,
stroke work X ms`; judge by strokes 2 and 3 (stroke 1 pays the first use of its sets). Write the table into
`notes\pkg_b.md`. If round 4 is within 1.10 x per stroke, P7 is closed as a measuring artifact and B-4 is
optional. Then settle the open point of 2.1: once B-2's log fields exist, one `brush_types_path` run on
ABeautifulGame shows `size` and `ring` of its Size-50 stroke; if the ring is not about 50 / 78 of the
`workflow_path` stroke's ring at the same camera distance, find out why before anything else.

**B-2 A stroke probe that exists in release builds.** Turn `brush::probe` into a runtime switch: a
`static ON: AtomicBool` (off by default) instead of `#[cfg(test)]`; `now()` returns `None` when off (one relaxed
load). Add slots for "first touch of a set" (`stroke_target`, `stroke.rs:435-453`: `allocate_raster`,
`StrokeScratch::new`) and "layer buffer copied" (`Arc::make_mut` at `stroke.rs:594` when the buffer is shared),
and a dab counter in `StrokeSession`. `Document::stroke_counters() -> StrokeCounters { dabs, sets_started,
buffer_copies, nanos: [u64; N] }` (reset by `stroke_begin*`). The app turns the switch on when
`WTE_STROKE_LOG` is set and then logs one line per sample of 20 ms or more (dabs, sets, phase split) and the
totals per stroke. Always: the stroke's log line (`tools\brush.rs:565-574`) gets `, size S px, ring R pt,
D dabs, longest dab L ms` appended at the end (`btp.py`'s regex keeps matching). `stroke_bench_tips` must stay
within 1.02 x with the switch off.

**B-3 Pace by dab.** Core (`stroke.rs`):

```rust
pub enum StrokeStep { Done, Partial }
/// `stroke_to_3d` that asks `more()` after every dab and stops with `Partial` when it says no (at least one dab
/// per call). The rest of the segment is kept in the session.
pub fn stroke_to_3d_paced(&mut self, hit: &Hit, pressure: f32, more: &mut dyn FnMut() -> bool) -> StrokeStep;
/// Continues a `Partial` segment.
pub fn stroke_resume(&mut self, more: &mut dyn FnMut() -> bool) -> StrokeStep;
```

The session stores the open segment (endpoints, normals, pressures, `step`, `count`, next `k`, the residual to
set at the end). `stroke_to_3d` = paced with `|| true`. Every other stroke call (`stroke_to_3d*` with a new
sample, `stroke_break`, `stroke_end`, `stroke_to_2d`) first finishes an open segment, `stroke_cancel` drops it —
so the dabs and their order never depend on the pacing. App (`BrushTool::paint`, `tools\brush.rs:521-556`): the
sample at the front of `pending` is painted with `more = spent in this frame < the frame's limit` (the same
`soft` / `frame_cap` rule as today, checked per dab); a `Partial` sample stays at the front and the frame ends.
`note_sample` books a sample when it is done (its time summed over frames); `longest_frame` is unchanged; the
backlog estimate counts a partial sample as one. `complete_stroke` (no time rule) and tests with the fixed
clock never go partial, so A4, A5, A17 and A19 run unchanged; a new test paints a stroke with one dab per frame
and gets the unpaced hash.

**B-4 Cheaper dabs — only what the probe supports, each one kept only if it pays.** Candidates in the order to
try, each behind the rule "keep it if stroke work on ABeautifulGame drops by 5 % or more in a back-to-back pair
and every hash test is identical; otherwise take it out and say so":

1. The per-dab `Vec<DabTri>` in `dab_tip` (`brush.rs:1236`): reuse a buffer held by the scratch (dense
   pieces give thousands of candidates per dab).
2. The per-set loop of one dab (`stroke.rs:495-518`): sets do not share scratch, layer buffer or backup, so the
   sets a dab reaches can be painted in parallel (`rayon`), dirty marks applied afterwards in set order. Only
   when at least two sets have candidates.
3. Whatever "first touch" or "buffer copied" shows, if they are not near zero after the warm-up stroke.

No change to the bridge rule, the dab spacing, the query radius or the coverage maths: those change pixels.

### 2.3 Acceptance

| # | Line | How |
|---|---|---|
| B1 | C1 legacy hashes, the shared-query test C2, `tests\brush_tips.rs`, `stroke::tests`, `tools::brush::tests` A1-A19: all pass with unchanged expected values; new test "one dab per frame = unpaced hash" | `cargo test --workspace` |
| B2 | P7 restated like for like: ABeautifulGame `workflow_path` strokes 1-3, stroke work per stroke ≤ **1.10 x** the P0 exe, back to back, 2 pairs | B-1's method, repeated by the gate with the new exe |
| B3 | Not slower than round 4: `brush_types_path` on ABeautifulGame, tex_8192, tex_4096, stroke work per stroke ≤ **1.05 x** `wte_round4.exe` back to back; core `stroke_bench_tips` ≤ 1.02 x | `btp.py`, `core_benches.ps1` |
| B4 | Longest frame of stroke work on ABeautifulGame, Size 50, after the warm-up stroke: ≤ **100 ms + the longest dab**, and ≤ **140 ms** (round 4: 152-206 ms); frames over 140 ms in those strokes: 0 (round 4: 11-13 over 100 ms, up to 213) | `btp.py` on 2 `brush_types_path` runs |
| B5 | P8a-P8c stay met (release to end ≤ 400 ms at Size 50; pointer step ≤ 1.10 x round 4) | same runs |
| B6 | P8d (largest backlog, ABeautifulGame, Size 50; limit 350 ms, round 4: 782-1,623 ms): **reported, with the phase split of the two bursts from the probe.** It is met only if B-4 makes the burst dabs about 3 x cheaper; if it is not met, the report says what the probe shows a later round would have to change (the expected answer: parallel rasterization inside one dab) | `WTE_STROKE_LOG=1`, 1 run |
| B7 | Fuzz seed 7 on ngons_tiling: no `pointer/tools/hover` frame over 500 ms unless one dab alone is longer (then the longest dab is reported) | with A7's runs, at the gate |

### 2.4 Files (package B, brush part)

Core: `stroke.rs`, `brush.rs`, `layer.rs` (only if B-4.3 needs it), `lib.rs` (exports), `history.rs` **only**
the `TileBackup` block (lines 147-260) and only if the probe points there. App: `tools\brush.rs`,
`tools\mod.rs` (brush pump only). `ARCHITECTURE.md` 5.19 (the two functions, `StrokeCounters`) and the brush
row of 8.6 (pacing by dab). No scenario changes (the existing `brush_types_path` and `workflow_path` measure it).

---

## 3. R5-3 — The font picker's first open (package B, small item)

### 3.1 What is known

`text_fonts`, between the log lines `gate: font picker open` and `gate: font picker closed`
(`selftest\scenarios.rs:5219-5246`): side-panel frames of 179.0 and 131.9 ms in run 1 of 5, at most 2.8 ms in
runs 2-5, 1.52 ms headless. Run 1 was the first after other runs. Two consecutive slow frames = the popup's
sizing frame and its first drawn frame; from the third frame on it is fast. Nothing in the picker's frame reads
a file: `list()` and `font_row()` (`panels\font_picker.rs:118-262`) use the in-memory catalog.

### 3.2 Candidates, most likely first

1. **Cold pages, not code**: the first open happens while the sample job starts (`text_ops.rs:39-48`) and
   memory-maps every font file; the exe sits on the USB disk and the popup / text-field / scroll-area code runs
   for the first time. Fits "once, after a pause, two frames, never headless".
2. **The sample job starts in the frame of the click**: `list()` pushes `LoadFontSamples`
   (`font_picker.rs:120-127`) and `load_font_samples` clones the catalog (`FontCatalog::clone`,
   `text\font_catalog.rs:129-137`: the fontdb database and the size map) on the UI thread. That is the
   `dispatch` segment, not `side panel`, but it loads the machine in exactly those two frames.
3. **Per-row catalog scans**: every visible row calls `pick_style` and `find` (`font_picker.rs:248-249`), each
   a pass over all faces with allocations (`font_catalog.rs:268-316`), and the Text block does the same for the
   current family (`text_block.rs:339`, `:447`). Small when warm (about 12 rows x 2 passes x a few hundred
   faces) but all of it is first-touch memory on the first open.
4. First layout of the family names (new glyphs in egui's atlas), the first `TextEdit` focus.

### 3.3 What the builder does

1. **Find it**: add `frame_profile::scope` guards inside the picker (`"picker: setup"` for lines 118-170,
   `"picker: project rows"`, `"picker: rows"`, and `"text block"` around the Text block) — scope names are free
   text and show up in the per-frame line (`app.rs:529`, `:604-609`). Run `text_fonts` with `WTE_FRAME_LOG=1`
   three times, the first right after an ABeautifulGame `workflow_path` run (the gate's order), and read the
   frames between the two markers with `frames.py`.
2. **Fix shape, whatever step 1 shows** (both are cheap and remove candidates 2 and 3):
   - Start the samples before the first open: push `LoadFontSamples` once per catalog from
     `font_picker::button` (the font button is drawn whenever the Text block is shown) instead of from the open
     popup; keep the `asked_for` guard. Opening then never coincides with the job.
   - Keep per-family row data in `PickerState`, built once per catalog size: the "N MB" label
     (`face.file_bytes >= BIG_FONT_BYTES`) — so a row does no catalog scan.
3. If a slow frame remains and the new scopes show the time outside the picker's own scopes or spread over
   unrelated ones, record it as cold paging with the frame lines as evidence and close the item.

### 3.4 Acceptance

- `text_fonts` x 3 (first one cold as above): every `side panel` frame between the markers ≤ **16 ms**; or the
  evidence of 3.3 step 3. `gate_font_picker` (ignored timing probe) still ≤ 16 ms; the picker tests
  (`the_font_picker_filters_and_takes_a_font`, `the_font_button_rect_opens_the_picker`) pass; the samples are
  asked for once per catalog (a test).

Files: `panels\font_picker.rs`, `text_ops.rs`, `panels\text_block.rs` (`pick_style` helper only, if needed).

---

## 4. Packages and file ownership

| | Package A | Package B |
|---|---|---|
| Items | R5-1 | R5-2, R5-3 |
| Core | `doc_stamps.rs`, `history.rs` (all but `TileBackup`), `document.rs`, `doc_build.rs`, `doc_layers.rs`, `project\load.rs`, new `stamp_job.rs` | `stroke.rs`, `brush.rs`, `layer.rs`, `history.rs` lines 147-260 (`TileBackup`) only |
| App | `jobs.rs`, new `stamp_bakes.rs`, `main.rs`, `app.rs`, `io_flows.rs`, `dispatch.rs`, `state.rs`, `tools\stamp_place.rs`, `selftest\mod.rs`, `selftest\scenarios.rs`, `gpu\scene.rs` | `tools\brush.rs`, `tools\mod.rs`, `panels\font_picker.rs`, `text_ops.rs`, `panels\text_block.rs` |
| Shared | `lib.rs` (each adds its own `pub use` lines), `ARCHITECTURE.md` (A: 1.x, 4.6, 5.14, 5.18, 5.20, 6.7, 8.6 Stamp row, 8.9, 8.11; B: 5.19, 8.6 Brush row) | |

Rules: a package that needs a line in the other's file writes the request into its notes instead of editing
(the gate applies it). B does not touch `state.rs`, `app.rs`, `dispatch.rs`, `io_flows.rs` or the scenarios:
the picker state lives in `PickerState`, the brush state in `BrushTool`, the env switch is read where it is
used. Both packages are independent and can be built in parallel; with one builder at a time, build A first
(it is the larger one and the one the user will feel).

Each builder: implement, add the tests, `cargo test --workspace`, clippy, a release build, the scenario smoke
runs of its item on the built-in sample, notes with what was measured. Each check-and-fix agent: read the diff
against the plan (rules 1-10 and the table of 1.5 for A; "no pixel changes" and the probe's off-cost for B), try
to break it (A: the event table, each row as a test or a `final_bake_async` step; B: tiny budgets, a stroke
ended / cancelled / broken with a partial segment), fix what it finds, rerun tests and clippy.

---

## 5. Gate (one agent)

1. Three-way merge of `pkg_a` and `pkg_b` into `target\round5\work\merged`; apply cross-package requests.
   `cargo test --workspace --offline` (round 4: 1,040 passed, 32 ignored — the count only grows), clippy
   `--workspace --all-targets` with 0 warnings, no `todo!` / `unimplemented!`, release build; copy the exe to
   `target\round5\wte_round5.exe` and record its SHA-256.
2. Scenarios: all 38 names at 1280 x 800 (`sweep.ps1` with a jobs file like `runs_gate3\jobs_final.txt` plus
   `final_bake_async`), and `final_bake_async` once more with `WTE_SYNC_FINAL_BAKES=1`. Read `shot.png` of
   `stamp`, `text_warp`, `text_path`, `brush_types`, `final_bake_async`.
3. Measurements, one process at a time, idle machine, each at most 2 runs, the round-4 exe alternating where a
   line says "back to back": A3-A8 (`workflow_path` on ABeautifulGame and ngons_tiling, `text_warp` on
   ABeautifulGame, fuzz seed 7), B2-B7 (`workflow_path` with the P0 exe, `brush_types_path` on the three
   models, one `WTE_STROKE_LOG=1` run), R5-3 (`text_fonts` x 3). About 25 runs in all.
4. Nothing got worse, against `wte_round4.exe`: `workflow_path` on ABeautifulGame, ngons_tiling, tex_8192 and
   two small models of `target\round4\runs_gate3\jobs_wf_subset.txt` — every workflow step within 15 % (the
   three stamp steps within 25 %, A8), the 3D half of `shot.png` identical, `shot_export` byte-identical and
   `check_exports.py` PASS, the saved project's `uv_hash` and layer pixels equal. The 9 fuzz repro jobs of
   round 4 (`jobs_fuzz`) pass with 0 errors.
5. Report `target\round5\GATE_REPORT.md`: each acceptance line with its number and the file it came from, what
   was not met and why, the answer to "is the brush slower per stroke than round 3" (B2), the phase split of
   the heavy brush frames (B6), what was found for the font picker, and what the user should try by hand: drag
   a stamp and a warped text on a heavy model and keep working while the picture sharpens; undo right after a
   release; save right after a release; a fast stroke across the chess pieces.

---

## 6. Defaults chosen (no question blocks the design)

1. **A new stamp shows its frame first, the picture when the bake lands.** After the click the gizmo frame and
   "Finishing stamp" in the status bar appear at once; the decal appears when the bake is done (a frame or two
   on small models, 0.6-2.5 s on ABeautifulGame). There is no Preview bake at placement, because a Preview bake
   on that model is itself 0.4-0.5 s of freeze. Say so if a rough picture at once is wanted instead.
2. **After a release the picture can show an earlier moment of the drag until the bake lands** (the last
   throttled Preview), then it swaps. Round 4 froze for that time instead.
3. **Save, export, merge down and rasterize wait** for the pending bake (at most what round 4 spent on the UI
   thread). Queueing them behind the bake without blocking would be a larger change.
4. **Undo and redo never wait**; a state restored without its Full bake is baked in the background.
5. The bake uses all cores but one, so the swap comes up to about 1.3 x later than round 4's freeze ended.
6. **P8d is reported, not forced.** The brush backlog on the 1.5-M-triangle model is real work on one thread;
   this round bounds the frame time while catching up (pace by dab) and takes only the pixel-safe speed-ups
   that measure as worth it. Making the dabs several times cheaper (parallel rasterization) is a round of its
   own.
7. The app tests keep the synchronous mode (`async_final_bakes` off in test states); the real app and every
   self-test scenario run the asynchronous mode. `WTE_SYNC_FINAL_BAKES=1` switches it off for comparisons.
