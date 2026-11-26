# Build note: P3a - The sticker document: types, render, styles pipeline, jobs (STICKER_PLAN 7 "P3a")

Builder, 2026-11-25, branch `stickers` (from 9b0d9be). Core logic (kind K) plus the minimum in `wte_app` to compile.
Nothing user-visible yet: the app cannot create a sticker or a style (P6a / P6b), so the identity sweep of 8.5 is the
app-level check; no new scenario (plan 8.2 lists none for P3a).

## What was built

| Item (plan P3a) | Where | Notes |
|---|---|---|
| Ids (2.1) | `ids.rs` | `StickerId(u64)` from `alloc_id`, `StickerLayerId(u32)` |
| Errors | `error.rs` | `UnknownSticker`, `UnknownStickerLayer`, `StickerLimit(&'static str)` |
| Sticker document (2.2, picture versions) | new `sticker/model.rs` | `StickerDoc`, `StickerLayer`, `StickerContent`, `PicturePixels`, `LayerMask`, `ArtboardShape`; `PartialEq` of owned pictures and masks: version first, bytes only when versions differ; `check_limits`, `sanitize`, `trivial_picture`, `scale_one_is_enough`, `from_picture` |
| Stamps (2.5) | `stamp.rs` | `StampSource::Sticker { sticker }`, `StampObject::styles` |
| Render (4.1, clipping, shape crop, fast path) | new `sticker/render.rs` | `render` / `render_with`, `StickerRenderInput` (`Send + Sync`), `LayerKey` (whole input by value, pixels by version / blob id), `LayerCache` (64 entries, 256 MiB) |
| Pipeline (2.6: frame, bake placement, working size, render scale) | new `sticker/pipeline.rs` | `SourceFrame`, `bake_placement_of`, `Styling`, `work_level`, `styled_mips`, `styled_straight`, `sticker_scale` |
| Placement refit (2.9) | `sticker/mod.rs` `refit_placement` | shares `text::frame::HostFrame` (now `pub(crate)`) |
| Counters | `sticker/mod.rs` `counters` | thread-local: renders, `apply_styles` calls (bumped in `fx::apply_styles`), largest styled side, largest Preview output |
| Document API (2.8, the P3a list) | new `doc_stickers.rs` | `sticker`, `stickers`, `sticker_users`, `blob_users`, `add_sticker`, `remove_sticker`, `promote_stamp` (+ `promote_no_history` for P3b), `detach_sticker`, `set_sticker`, `set_stamp_styles`, `set_stamps_styles`, `bake_placement`, `bake_wrapped_projector`, `stamp_source_frame2`, `stamp_picture`, `sticker_picture`; `StickerRef`, `ShareScope`, `StickerCaches` |
| `source_mips` pipeline, call sites of the two placements, `job_inputs_current` | `doc_stamps.rs` | `bake_picture` (UI thread), `derived_job_picture` (Full), `mark_stamp_edited` (body of `preview_edit`), `refit_auto_depth_framed` |
| Jobs (2.7, A1, A27) | `stamp_job.rs` | `JobPicture::{Ready, Derived}`, `JobBase`, `DrawnPicture`; `sticker: Option<Arc<StickerDoc>>` on job and output; the job bakes on the bake placement with the frame's tint |
| History | `history.rs` | `StructureSnapshot::stickers` (Arc clones), restore swaps it; history cost and live buffers count owned sticker pictures / masks |
| Document fields | `document.rs` | one boxed `sticker_state: Box<StickerState { map, pixel_version, cache }>`; `next_pixel_version()`; invariants (stamp layer of a sticker stamp, sticker exists) |
| Every `match` on `StampSource` | core (`project/{save,load,manifest}.rs`, tests) and app (`tool_options`, `text_block`, `stamp_place`, `tools/mod`, scenarios, tests) | the app shows a sticker stamp like an image stamp; the "Image: Choose..." row is hidden for it |
| Tests | new `doc_stickers_tests.rs` (+ in-file tests in `model.rs`, `pipeline.rs`), `sticker/bench.rs` | 18 `k3a_*` tests, 1 ignored perf test |
| ARCHITECTURE | 1.12, 2.1, 4.4, 5.1, 5.2, 5.12, 5.14, 5.18, 5.20 "Stickers", 5.27 part 3, 6.5 (invariants and "Stickers"), new 6.10, 9.1, 10 | |

HANDOFF.md and `dist/HOW_TO_TEST.txt`: no change (plan section 10 gives P3a none; nothing is user-visible).
No new crate; `Cargo.lock` unchanged. No ArtCraft name. New files are LF; the four CRLF files this package edits
(`error.rs`, `project/manifest.rs`, `project/save.rs`, `text/frame.rs`) keep CRLF (baseline note: no whole-file switch).

The P0 identity hash of test k3a-1 (`0x63d26d88cd3f46ee`) was recorded with the code before any P3a change (also with a
local pool of 1 thread: the same).

## Bake vs stored placement (the table of 2.6, as built)

| Call site | Placement | Why |
|---|---|---|
| `StampBakeJob::run` (every Full, wrap Preview and ParamOnly job) | bake, from the job's own frame | the picture it places has the margin |
| `bake_and_apply` (Preview) | bake, from `bake_picture`'s frame | same |
| `ensure_wrap_param` | bake (Preview picture's frame) | the parameterization is made for what is drawn |
| `make_wrap_job` | bake (in the job) | same |
| `serving_wrap_param`, `apply_wrap_preview` (both caching branches), `apply_wrap_param` | bake (`bake_wrapped_projector`) | "does the cached parameterization serve" |
| `stamp_shared_texels` | bake (`bake_projector`) | samples the drawn footprint |
| `stamp_ink_area` | none (reads the caches) | - |
| auto depth: `set_stamp_source`, `retune_text`, `set_sticker` (artboard), `set_stamp_styles`, `set_stamps_styles` | bake footprints, depth written into the stored placement (`refit_auto_depth_framed`) | the depth covers the shadow too |
| `wrapped_projector` and its readers `stamp_wrap_outline`, `stamp_wrap_stats`, `stamp_wrap_local`, `stamp_wrap_surface`, `stamp_wrap_quality`, `wrap_preview_deferred`, `next_wrap_preview_job`, `wrap_param_job` (only "is it wrapped") | stored | gizmo outline and handles keep the artwork's box |
| `text_raster_px`, `stamp_text_frame`, `set_stamp_placement` / `set_stamp_tint` validation, the panels and the gizmo | stored | as before |

## Deviations from the plan, with reasons

1. **"Styled" means `styles.any_on()`, not `!is_default()`.** A stack whose entries are all off draws exactly what no
   stack draws (margin 0), so it keeps the bit-identical path. (`required_reader` in P4 still counts an off entry as
   used, as 2.3 says.)
2. **`bake_placement(id)` depends on the current picture.** The margin of a styled picture is measured at its working
   level `L` (`ceil(scale / 2^L * reach) + 2` level pixels), so the bake placement of a Preview (small `L`) and of a Final
   can differ by a pixel's worth. The bake always uses the frame of the picture it places (exact). `bake_placement(id)`
   uses the frame of the stamp's current cached styled picture (else the reference frame at `L = 0` from geometry), so it
   is the placement the cached parameterization was made for. Consequence: a styled wrapped stamp may build its
   parameterization again at the Final after a Preview drag (cost only; plain stamps unchanged).
3. **Working size details.** The 4096 limit is on the working picture (input of `apply_styles`, test k3a-5), plus an
   output limit of 6144 a side (check P1 minor 4: large lengths at a text's reference scale of 4 could ask for hundreds of
   MB). The Final density rule uses the denser axis ("the denser axis may stay up to 2 pixels per texel") so that the
   other axis never drops below the texels. The working picture is mip level `L` un-premultiplied (also at `L = 0`, from
   the premultiplied level 0: alpha < 255 edge colours are rounded once; opaque photos are exact). `styled_straight`
   (finished pictures) uses the straight base at `L = 0`.
4. **Sticker render scale is computed at every bake** (a pure function of the placement and the sticker, buckets 1 / 2 /
   4), not stored at Final placement edits: no extra state, the same result at the Final; a Preview reuses any cached
   render of the same sticker value, so a drag does not re-render.
5. **Promoted images use their own mips for every quality.** Plan 4.1's fast path is at scale 1; a Preview of a large
   promoted image would halve the scale and render. Built: the trivial sticker always takes `image_mips[blob]` (the bake
   samples the mip level it needs), so promotion changes no pixel in Previews either (k3a-1 checks a Preview too) and
   nothing is rendered.
6. **`LayerKey` has no sticker id.** The key is the layer's whole input (content by blob id / version / value, mask
   version, transform, adjustments, styles, artboard, scale, quality), so two stickers with the same input share a
   raster correctly (a detached copy hits). The layer id is not needed either.
7. **Render cache: 8 entries and also 512 MiB** (picture and its mips); a 4096^2 render with mips is about 155 MB.
8. **Text layers** draw within the text engine's 8..=512 px per em; a larger em (or a picture over the engine's limits)
   is drawn smaller and scaled up by the resampler. A text layer's scale and rotation turn about its anchor (which sits at
   `translate`), pictures and shapes about the content centre. Adjustments apply to text and shape content too.
9. **`StyledEntry` caches the tint** (styled pictures have it multiplied in) and the quality; a Preview takes any entry of
   the same source, styles and tint, a Full needs a Full entry of the same resolution and style scale.
10. **`apply_wrap_preview`** with a job made for an older sticker value applies the picture as the last good one and
    leaves the stamp due again (as for an older stamp object); `apply_final_bake` drops it (`Stale`). Plan k3a-9 says
    "the same for apply_wrap_preview"; this is the existing round-8 meaning of "same" for Preview jobs.
11. **`StickerRef::OfStamp`** resolves a sticker stamp; for an image stamp it is `InvalidParam` here (promotion inside
    an edit is P3b, which has its tests). `promote_no_history` is ready for it.
12. **Save refuses stickers until P4.** `project::save` returns `ManifestInvalid("stamp ... uses a sticker or stamp
    styles, which this build cannot save yet")` and writes nothing, rather than dropping them. Unreachable from the UI.
13. **Sizes.** `Document` grew with the snapshot's sticker map (the inline `Option<PendingEdit>`): clippy
    `large_enum_variant` on the app's `JobOutput::Built`. Fixed by one boxed `sticker_state` and a boxed
    `pending_edit` (`Option<Box<PendingEdit>>`).
14. **Text stroke test (k3a-4)** checks that the stroke width on the model is `width * em / 128` texels at 64, 128 and
    256 px per em (UV placements whose em is that many texels, so the raster size is 64 / 128 / 256); measuring the same
    on-model size at three raster sizes would need overriding `retune_text`.
15. **k3a-5 "styled width within 1 texel for working scales 1, 1/2, 1/4"** is run on the pipeline (`styled_mips` with
    `texels` forcing `L` = 0, 1, 2, then `bake_uv` on the bake placement) on a 1024 px picture; the 8192 x 8192 document
    part checks the worker side, the calling thread and the Preview pixel count.
16. **k3a-10** drops sticker values through `remove_sticker` plus `history.clear()` every 10 cycles, so the allocator
    can reuse addresses while the render cache is in use; every hit is checked byte for byte.

App call sites of `fit_stamp_depth` / `stamp_depth_is_auto` (`tools/transform.rs`, `panels/tool_options.rs`,
`panels/text_block.rs`, `tools/stamp_place.rs`) still measure the stored placement. Correct for every stamp today (no
styles can be set from the UI); for P6b: a styled stamp's gizmo drag should use the bake footprint
(`refit_auto_depth_framed` semantics) or let the core refit.

## Numbers measured

- k3a tests (debug test profile, `wte_core` at opt-level 1): 18 pass in 2.3-3.6 s; k3a-5 (8192 x 8192 PNG, decode,
  style on a worker) is the slowest.
- Perf (`perf_k3a_render_three_layers_1024`, release, serial; photo + ClassicSticker, text "Sticker" at 160 px, star with
  outline, 1024 x 1024): cold **40.0 ms** (budget 120), one layer changed with the layer cache **5.0 ms** (50), Preview
  quality one layer changed **4.9 ms** (33).
- Tests: see "Workspace runs" below (against P2's 1,474 / 1 / 54 of check-P2: +18 `k3a_*`, +1 ignored `perf_k3a_*`).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 28,432,896 bytes, SHA-256 `4ff60a87469dda596bc8bb408fa0c95be5f883811fc52ec3653812d35ac1b6aa`
  (names this build only; see `baseline.md`).
- Identity sweep (8.5, `tools/objtabs/sweep.sh` with that exe, `tools/sticker/jobs_identity.txt`, off-screen, no focus)
  into `target\sticker\p3a\sweep\`: **21 / 21 `"ok": true`**; `compare_baseline.py` against the P0 baseline: **216 / 216
  files the same, PASS** (`target\sticker\p3a\compare.txt`). Looked at `stamp` (logo on the checker sphere with gizmo),
  `wrap_surface/shot_wrapped.png` (wrapped cells picture) and `workflow_path` (Split view, chess set): as before.

## Workspace runs (final state)

- `cargo test --workspace --no-fail-fast` (final source): **1,492 passed, 1 failed (WA20), 55 ignored** (`wte_app`
  524 / 0 / 24; `wte_core` 912 / 1 / 31; integration 15 + 9 + 8 + 24). The red test is
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` only. (An earlier full run on this machine
  under heavy load took 487 s for the core binary, the final one 93 s; same results.)
- `cargo clippy --workspace --all-targets`: 0 warnings.
- Release build and identity sweep: as above (the exe was built from the final non-test source; the later edits were the
  k3a-1 Preview check, the ignored perf test and ARCHITECTURE 9.1).
- Line endings: new files LF; edited CRLF files kept CRLF (`git ls-files --eol` unchanged for every existing file).
