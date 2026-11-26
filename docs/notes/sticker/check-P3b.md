# Check note: P3b - Pixel edits, undo, limits, conversion (STICKER_PLAN 7 "P3b")

Checker, 2026-11-25, branch `stickers`, builder commit 5714344 (from 95108e9, the P3a check). Independent and
adversarial: every plan item compared with the code, the builder's numbers re-run, extra tests written to try to break it.

**Verdict: pass.** No blocker or major found. No source change was needed; this check adds 8 `chk3b_*` tests (they pass)
and this note.

## Results re-run by the checker

| What | Result |
|---|---|
| `cargo test --workspace --no-fail-fast`, builder's source | 1,510 passed / 1 failed (WA20) / 56 ignored, as the builder reported |
| the same with the checker's tests | **1,518 passed / 1 failed (WA20 only) / 56 ignored** (`wte_core` 938 / 1 / 32, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 24) |
| `cargo clippy --workspace --all-targets` | 0 warnings |
| `cargo build --release -p wte_app` | OK; exe 28,838,400 bytes, SHA-256 `a47250059b6e5404918402cb4e72e348668619060a68ef28933b03bb321397f3`, the same as the builder's (only tests changed since) |
| texture patch hash on the code before P3b | the builder's test body run in a temporary worktree at 95108e9 (removed afterwards) prints `0xb75b1fe0532ccc75`, the constant in `k3b_2_texture_stroke_patch_bytes_are_unchanged`: the texture path keeps its bytes |
| `perf_k3b_stroke_begin_and_one_dab` (release, serial) | begin on a shared 4096 x 4096 picture 10.1 ms (budget 40); one size-64 dab on 2048 x 2048 0.20 ms (budget 2) |
| identity sweep (8.5), `tools/objtabs/sweep.sh` + `tools/sticker/jobs_identity.txt`, off-screen, no focus taken | 21 / 21 `"ok": true`; `compare_baseline.py` against the P0 baseline: **216 / 216 files the same, PASS** (`target\sticker\p3b_check\sweep\`, `compare.txt`) |
| screenshots looked at | `undo` (Split view: red and blue strokes, logo and WTE text on the checker sphere, "Redo: Add Text (x4)" toast), `paint2d` (two blue strokes in the UV view), `paint3d` (red stroke across two faces of the cube): as before |

## Plan items (7 "P3b") against the code

| Item | Found | Note |
|---|---|---|
| `TileBuf`, generic `TilePatch` / `TileBackup` | yes | const parameter instead of a stored `bpp` (deviation 2, fine: the kind is the type; `swap_with`, `touch_rect`, `restore`, `into_patch`, `diff` refuse another kind) |
| places that still assume 4 bytes | none wrong | read the whole of `history.rs`: `original` / `original_row` exist only on `TileBackupOf<4>`; `byte_len` counts `Raw` lengths; a solid mask tile keeps its value in byte 0 and `fill_tile` writes `value[..BPP]` |
| `Edit::StickerPixels` | yes | swap via `Arc::make_mut` on sticker and buffer, new version each time, users re-baked after the whole entry (`settle_sticker_pixel_edits`); history cost = patch bytes; undo / redo refused during a sticker stroke |
| strokes (begin / dab / end / cancel) | yes | plus `break`, `active`, `target` |
| `replace_sticker_pixels` | yes | same size: tile patch of changed tiles; other size or a blob: `Structure` |
| `StickerRef::OfStamp` promotion inside the edit | yes | `set_sticker` (pending "before" taken before the promotion), strokes (`Multi([Structure, StickerPixels])`), `replace_sticker_pixels`; every refusal takes the promotion back |
| `convert_text_stamp`, `sticker_checkpoint`, `restore_sticker_state`, `rebake_sticker_users`, limits | yes | |
| `next_pixel_version` | per document (P3a) | plan 2.2 says per process; every cache keyed by it is per document, so no collision is possible today (P3a check accepted it) |
| ARCHITECTURE 5.18, 5.20, 6.7, 9.1 (+ 2.1, 5.5, 5.27, 10) | yes | read the added text; it matches the code |
| tests k3b 1-8 | all present | each plan bullet has an assertion; the patch-hash claim verified independently (above) |
| rules: no new crate, `Cargo.lock` unchanged, LF, no ArtCraft name | yes | grep and `git diff` checked |

## Extra tests written by the checker (`chk3b_*`, `doc_sticker_edits_tests.rs`)

1. `chk3b_trivial_styled_sticker_preview_copies_nothing_per_dab`: a sticker on the trivial fast path (one opaque
   picture, opacity 1) with a styled stamp; dabs interleaved with live Previews, `stamp_picture` and `sticker_picture`
   never copy the picture (the renders that hold its `Arc` are dropped before each write).
2. `chk3b_a_dab_on_a_transformed_layer_lands_at_the_inverse_point`: `LayerTransform::invert` undoes `apply`
   (rotation, flip, anisotropic scale); a dab at the layer-space image of a content pixel paints that pixel.
3. `chk3b_a_final_job_made_before_a_stroke_or_an_undo_is_stale_after_it`: a deferred Final job made before a stroke,
   and one made before an undo of a stroke, come back `Stale` (no in-place write while a job holds the sticker); the
   user ends with the right picture.
4. `chk3b_a_stroke_on_a_detached_copy_leaves_the_original_alone` (shared pixels after `detach_sticker`).
5. `chk3b_mixed_history_walks_back_and_forth_consistently`: strokes and structure edits interleaved, undone and redone
   twice; picture bytes and the user's cache match each state.
6. `chk3b_revert_of_a_stamp_promoted_inside_a_stroke_brings_the_image_back` (Revert after a promotion inside a stroke,
   then undo / redo).
7. `chk3b_convert_text_redo_matches` (redo of a conversion: same object, cache and linked layer kind).
8. `chk3b_convert_text_keeps_the_ink_box_within_one_texel` at 32, 128 and 512 px per em (stronger than the mean
   difference of k3b-7: a one-pixel shift of the artwork would show here).

## Findings

Minor, not fixed (no change of behaviour needed for P3b; for the later packages):

1. **Renders of earlier sticker values pin their pictures.** The render LRU (8 entries, P3a) keeps entries of earlier
   values of a sticker (after a structural edit such as an opacity change). Each entry holds its `Arc<StickerDoc>`, and
   so that value's layer pictures, which `RENDER_CACHE_BYTES` does not count. Effect: (a) after `history.clear()` or an
   eviction, the next `sticker_stroke_begin` can still copy the picture once (found by checker test 1, which first
   expected no copy); (b) up to 8 old picture sets stay in memory beyond the history budget. Normally the history
   snapshot holds the same value, so the copy happens anyway and the memory is shared. Worth a look when P4 / P7a
   measure memory: count the doc's owned buffers in the render-cache budget, or drop renders of values that neither the
   map nor the history holds.
2. **Cancel with deferred Final bakes after a live preview** leaves the Preview picture on the model until the
   deferred Final bake has run (`rebake_users_final` marks the users unbaked). The plan says "its cancel puts the earlier
   pictures back"; it does once the job runs, as every deferred bake. P7a should make sure the job runs promptly.
3. **The app does not yet know about sticker strokes.** `wte_app` checks `Document::stroke_active()` (texture strokes
   only) for "interacting" and the bake scheduler (`gpu/scene.rs`, `panels/mod.rs`, `stamp_bakes.rs`). The core is
   safe (`next_final_bake` gives no job, `apply_final_bake` answers `Busy`), but P6a / P7a must use
   `sticker_stroke_active()` there too, besides the builder's open items (`sticker_stroke_break` on pen lifts, the
   live `rebake_sticker_users(Preview)`, mapping the dab rectangle from buffer pixels).

Deviations of the builder (`build-P3b.md`, 14 items) were read against the code: each is as described and none breaks a
user decision or the plan's intent.

## Focus / clipboard / registry

Only `--screenshot ... --scenario ...` runs of the checked exe were started (off-screen, no focus). No clipboard,
registry, `%LOCALAPPDATA%` or input injection. The temporary worktree at 95108e9 lived in the scratch directory and was
removed with `git worktree remove`.
