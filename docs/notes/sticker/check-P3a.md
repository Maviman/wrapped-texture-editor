# Check note: P3a - The sticker document: types, render, styles pipeline, jobs (STICKER_PLAN 7 "P3a")

Independent checker, 2026-11-25, branch `stickers`, reviewed e0987a3 + c050075 against 9b0d9be (the P2 check).
Verdict: **pass after fixes**. No blocker or major found; three minors fixed with tests (below), the rest noted.

## Re-runs

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` on the builder's code | 1,492 passed / 1 failed (WA20) / 55 ignored (as reported) |
| the same with the fixes of this check | **1,496 passed / 1 failed (WA20 only) / 55 ignored** (`wte_core` 916 / 1 / 31, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 24) |
| `cargo clippy --workspace --all-targets` | 0 warnings (before and after the fixes) |
| `cargo build --release -p wte_app` | OK (before and after the fixes) |
| identity sweep (8.5), builder's exe | 21 / 21 `"ok": true`; `compare_baseline.py`: 216 / 216 the same, PASS (`target\sticker\check-p3a\sweep\`) |
| identity sweep, exe with the fixes | see "Sweep after the fixes" below (`target\sticker\check-p3a\sweep2\`) |
| screenshots looked at | `text` (WTE 2026 on the checker sphere, gizmo, Text block), `wrap_surface/shot_wrapped.png` (cells picture wrapped on the sphere): as before |

**The identity hash of k3a-1 was verified independently.** The P2 source (`git archive 9b0d9be`) was built in a separate
folder with a probe test holding the same `identity_doc_sized` and `cache_hash` as k3a-1: size 128 gives
`0x63d26d88cd3f46ee` (the constant in k3a-1, so it really comes from the old code), size 512 gives `0x1d968db02d9a6712`.
The new test `check_p3a_identity_at_512_with_styles_switched_off_and_promoted` holds the 512 hash and also bakes every
stamp with a style stack whose entries are all off, and three promoted stamps, to the same hash.

## Plan items (7 "P3a") against the code

All listed files and functions exist: ids, errors, `StickerDoc` with picture versions and version-first `PartialEq`,
`StampSource::Sticker`, `StampObject::styles`, `sticker::render` (layer order, opacity, seven blend modes, mask,
clipping with a hidden base, shape crop, trivial fast path), `doc_stickers.rs` (queries, add, remove, promote, detach,
`set_sticker` for `Id`, `set_stamp_styles`, `set_stamps_styles`, `bake_placement`, `bake_wrapped_projector`,
`stamp_source_frame2`, `stamp_picture`, `sticker_picture`), the `source_mips` pipeline with the working size,
`job_inputs_current`, `StructureSnapshot::stickers`, `sticker` + `DrawnPicture` on job and output, refit of 2.9, every
`StampSource` match. The 11 unit-test items are present (k3a-9's "same for apply_wrap_preview" as the builder's
deviation 10, accepted: it is the round-8 meaning of a Preview job). ARCHITECTURE sections are updated.

Not applicable: "thumbnail from `stamp_picture`" in the app. The app shows no picture of a selected stamp's source
(`PanelState::source_thumb` is the Stamp tool's chosen blob only); layer thumbnails come from the baked caches. Nothing
to wire until the sticker tray (P6b).

### Bake vs stored placement (checker's grep of `.placement` and `wrapped_projector(` in core)

Every read outside `doc_stamps.rs` / `doc_stickers.rs` / project / text code: only `stamp_job.rs:284` (bake, from the
job's frame). In `doc_stamps.rs`: `set_stamp_placement` / `set_stamp_tint` / `set_stamp_source` / `retune_text` write
the stored placement (refit through `refit_auto_depth_framed`, bake footprints when styled); `text_raster_px` (775,
stored); `bake_and_apply` (bake, from `bake_picture`'s frame); `make_bake_job` (stored for the set check and the cached
parameterization, the job then uses the bake placement); `serving_wrap_param`, `ensure_wrap_param`, `apply_wrap_preview`
(both branches), `apply_wrap_param`, `stamp_shared_texels`: bake; `wrapped_projector` readers `stamp_wrap_stats`,
`stamp_wrap_outline`, `stamp_wrap_quality`, `wrap_preview_deferred`, `next_wrap_preview_job`, `wrap_param_job`, the
`live` test of `apply_wrap_preview`: stored. `WrapParam::local_of` / `surface_of` / `outline` work in world units about
the anchor, so the stored half size on a parameterization made for the bake placement gives the artwork's outline
(k3a-3 measures it). Matches the builder's table and plan 2.6.

## Findings

| # | Severity | What | Status |
|---|---|---|---|
| 1 | minor | **A styled text's Preview showed the stroke (and every style length) up to 16x too wide.** The text engine draws the Preview picture of a large styled text (bold, slant, warp, path) smaller than its frame (FEATURE_TEXT 5.8, `full == false`), but `bake_picture` styled it with the full reference scale `size_px / 128`. Seen while typing or recolouring a large styled text with a stamp style: the Preview stroke was twice as wide, the Final then jumped back. | fixed: `bake_picture` scales the reference scale by picture width / frame width. Test `check_p3a_a_styled_text_preview_keeps_its_stroke_width` (before the fix: Preview ink y 798..1279 against the Final's 829..1248) |
| 2 | minor | **A large styled text layer in a sticker was drawn at half size and in the wrong place in Previews.** `render.rs` `text_content` took the em and anchor of the frame (full-size numbers) for the reduced Preview picture. | fixed: em and anchor scaled by the picture / frame ratio. Test `check_p3a_a_large_styled_text_layer_renders_in_place_in_previews` (before: ink 0..523 x 298..465 against the Full render's 0..994 x 357..690) |
| 3 | minor (memory) | **`stamp_picture` / `sticker_picture` of a promoted photo copied the photo and built its mips again** (`sticker_render` -> `render_with` fast path clones the picture, then `PremulMips::build`): about 600 MB extra for an 8192 px photo, kept in the render cache. Not reachable from the UI yet; copy, drag out and the tray (P6b, P10b, P11) would hit it on the most common sticker. | fixed: `sticker_render` shares the blob's decoded pixels and its `image_mips` (or the owned buffer) for the trivial path at scale 1; same bytes. Test `check_p3a_a_promoted_photo_shares_its_pixels_in_finished_pictures` |
| 4 | minor (open, P6b) | The app's gizmo / panel auto-depth code (`tools/transform.rs`, `panels/tool_options.rs`, `panels/text_block.rs`, `tools/stamp_place.rs`) asks `stamp_depth_is_auto` on the **stored** placement. For a styled stamp the stored depth is the automatic depth of the **bake** footprint, so the app will treat it as hand-set and stop refitting it after a move. Latent (no UI sets styles before P6b); the builder lists it too. | open, for P6b |
| 5 | minor (open) | `StickerCaches::blob_pictures` is never trimmed (plan 2.7 gives it no limit) and holds the straight pixels next to `image_mips` (premultiplied mips) of the same blob: a large photo used by a rendering sticker costs both for the life of the document. | open, note for P6b / P10 (thumbnails, copy) |
| 6 | note | Promoting a **styled** image stamp: the styled cache entry is keyed by `BaseIdentity::Blob`, the sticker's by `Sticker`, so `bake_placement` falls back to the reference frame; at working level `L > 0` that can differ by a pixel's worth and a wrapped stamp's parameterization is built again. No pixel changes, no bake. | accepted (cost only) |

Nothing found in: job freshness (sticker `Arc` held by the job, so `ptr_eq` cannot see a reused address; undo while a
job runs gives `Stale`), history (snapshots hold `Arc` clones, `live_buffers` and `edit_cost` count sticker buffers
once), versions (`version_new_buffers` runs before the change test, so a copied buffer always gets a new number), the
render cache (holds its `Arc`), determinism (render and styles thread-count independent; k3a-11 pools of 1 and 8),
back-compat (old projects load with default styles; save refuses only documents that use stickers or styles; identity
sweep), focus / clipboard / registry rules (P3a has no app or FFI behaviour; only `--screenshot` runs were started).

## Sweep after the fixes

Release exe with the fixes: 28,428,800 bytes, SHA-256 `8da54522...` (names this build only). Identity sweep with
`tools/objtabs/sweep.sh` and `tools/sticker/jobs_identity.txt`, off-screen, no focus taken, into
`target\sticker\check-p3a\sweep2\`: **21 / 21 `"ok": true`**; `compare_baseline.py` against the P0 baseline: **216 /
216 the same, PASS**. Screenshots looked at: `stamp` (logo with its turned gizmo on the checker sphere, Image row
shown), `workflow_path` (chess set in Split view with the UV view and the painted strokes): as before.

New tests of this check (all `wte_core`, `doc_stickers_tests.rs`): `check_p3a_a_styled_text_preview_keeps_its_stroke_width`,
`check_p3a_a_large_styled_text_layer_renders_in_place_in_previews`, `check_p3a_a_promoted_photo_shares_its_pixels_in_finished_pictures`,
`check_p3a_identity_at_512_with_styles_switched_off_and_promoted`. The first two fail on the builder's code (numbers in
the table above). ARCHITECTURE 6.5 ("Stickers", step 3) and 6.10 (trivial fast path) say what the fixes do.
