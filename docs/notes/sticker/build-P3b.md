# Build note: P3b - Pixel edits, undo, limits, conversion (STICKER_PLAN 7 "P3b")

Builder, 2026-11-25, branch `stickers` (from 95108e9, the P3a check). Core logic (kind K), headless; nothing in
`wte_app` changes (it compiles unchanged: no `match` on `Edit` there). Nothing is user-visible yet (the sticker editor
UI is P6a-P8), so plan 8.2 lists no scenario for P3b; the app-level check is the identity sweep of 8.5, which includes
`undo`, `paint2d` and `paint3d` (the scenarios the P3b checker is told to compare).

## What was built

| Item (plan P3b) | Where | Notes |
|---|---|---|
| `TileBuf` (2.10, review A4) | `pixel.rs` | `const BPP`, `size`, `bytes`, `bytes_mut`; for `PixelBuf` (4), `AlphaBuf` (1) and `Arc<T>` (reads as `T`, writes copy-on-write) |
| Generic `TilePatch` / `TileBackup` | `history.rs` | `TilePatchOf<const BPP>` / `TileBackupOf<const BPP>`; `TilePatch = TilePatchOf<4>`, `MaskPatch = TilePatchOf<1>`, `TileBackup`, `MaskBackup`; `swap_with<B: TileBuf>` refuses a buffer of another kind (changes nothing, returns `EMPTY`); `fits`, `bpp`, `empty`, `diff` (changed tiles of two same-size buffers), `original_span` (any kind) |
| `Edit::StickerPixels` | `history.rs`, `doc_sticker_edits.rs` | `{ sticker, layer, patch: StickerPatch::{Pixels(TilePatch), Mask(MaskPatch)} }`; swap through `Arc::make_mut` on sticker and buffer, new version, users re-baked after the whole entry (`settle_sticker_pixel_edits`); history cost = patch bytes |
| Strokes | `doc_sticker_edits.rs` | `sticker_stroke_begin / dab / break / end / cancel / active / target`, `StickerBrush` (radius in layer px, hardness, opacity, flow, spacing, colour, Paint / Erase, `PressureDynamics`, selection) |
| `replace_sticker_pixels` | `doc_sticker_edits.rs` | same size on an owned picture or a mask: `StickerPixels` of the changed tiles; other size or a blob picture: `Structure`; unchanged: no entry |
| `StickerRef::OfStamp` with promotion inside the edit (2.10, review A3) | `doc_stickers.rs` (`set_sticker`), `doc_sticker_edits.rs` | `promotion_of`, `begin_pending_with`; strokes push `Multi([Structure(before), StickerPixels])` |
| `convert_text_stamp` (review A24) | `doc_sticker_edits.rs` | em = `text_raster_px`, artboard = the text frame, anchor at `translate`, placement unchanged, styles x `em / 128`, linked layers `Text` -> `Stamp` |
| Revert (review B12) | `doc_sticker_edits.rs` | `sticker_checkpoint`, `restore_sticker_state` (one entry; none when nothing differs) |
| `rebake_sticker_users` | `doc_sticker_edits.rs` | live preview during a stroke; Final (or unbaked) otherwise |
| Limits (2.7) | `doc_sticker_edits.rs` (+ `StickerDoc::check_limits` of P3a) | 32 layers, 4096 artboard, 512 MiB owned (also a blob made owned by a stroke), 4096 for a new size made in the editor |
| Strokes exclude each other | `document.rs` (`any_stroke`, `guard_no_stroke`), `history.rs`, `doc_stamps.rs` | an open sticker stroke blocks every command, undo / redo, texture strokes and new Final bake jobs, as a texture stroke does |
| Copy counter | `sticker/mod.rs` | `counters::buffer_copies()` (shared picture / mask buffers copied by a write) |
| `LayerTransform::invert`, `mean_scale` | `sticker/mod.rs` | layer space -> content pixels for dabs |
| `next_pixel_version` | `document.rs` | existed since P3a (per document; plan 2.2 says "process"); unchanged |
| Tests | new `doc_sticker_edits_tests.rs` | 14 `k3b_*` tests, 1 ignored `perf_k3b_*` |
| ARCHITECTURE | 2.1, 5.5, 5.18, 5.20 ("Sticker pixel edits"), 5.27 (part 4), 6.7, 9.1, 10 | |

HANDOFF.md and `dist/HOW_TO_TEST.txt`: no change (plan section 10 gives P3b none; nothing is user-visible). No new
crate; `Cargo.lock` unchanged. Names and marks as plan 0.4 rule 9 (checked with grep). New files LF; every edited file keeps its line endings (all LF).

## Deviations from the plan, with reasons

1. **`TileData` of 2.8 is named `PixelData`.** `history::TileData` (the tile payload, `Solid` / `Raw`) is re-exported at
   the crate root, so a second public `TileData` would clash. `PixelData::{Rgba(PixelBuf), Alpha(AlphaBuf)}` as planned.
2. **Generic patches by a const parameter, not a stored `bpp` field.** `TilePatchOf<const BPP>` keeps the struct literal
   `TilePatch { tiles }` and the `TileData` enum unchanged, so the existing history tests compile and run **untouched**
   (a stored field would have needed an edit in the history test helper). The kind is part of the type; `swap_with`
   on another kind is refused at run time (test k3b-2).
3. **`impl TileBuf for Arc<T>`.** `touch_rect` / `into_patch` used to take `&PixelBuf` by deref from `&Arc<PixelBuf>`; a
   generic parameter does not deref, and the testkit helper `paint_rect` and `stroke.rs` pass the layer's `Arc`. The
   impl keeps every existing call site byte-for-byte unchanged (`bytes_mut` on an `Arc` is `Arc::make_mut`).
4. **`Edit::StickerPixels { sticker, layer, patch: StickerPatch }`**: the target is the patch's variant (`Pixels` /
   `Mask`), not a separate `target` field that could disagree with it.
5. **A new picture version at every dab that changes pixels**, not only at `begin`. The layer raster cache keys owned
   pictures by version; with one version per stroke, a Preview render during the stroke (the live preview) could take
   the raster of an earlier dab. Equal versions still mean equal bytes (2.2), which is the stricter reading.
6. **Writes mutate the sticker in place when nothing shares it.** `Arc::make_mut` keeps the map's `Arc` when it is
   unique, so cache entries holding the `Arc` are dropped before every write (`forget_sticker_pictures`: renders and
   styled stamp pictures of that value). An entry that would hold it keeps the refcount above one, so a stale
   `ptr_eq` match is impossible; dropping them first is what keeps the one copy per stroke in `begin`.
7. **Stroke model.** The 2D texture kernel of 6.4 (`falloff`, `mask_increment`, 16-bit coverage, opacity caps, flow
   builds) without wrapping at the borders (`brush::dab_2d` wraps around UV borders, wrong for a picture); a dab is a
   new private function using the same `falloff` and `mask_increment` (`mask_increment` became `pub(crate)`). Constant
   step `max(spacing * diameter, 0.5)` with pressure interpolation; the pressure-sized walk of pen texture strokes
   (6.4 "VarWalk") is not ported (size dynamics still change each dab's radius). The radius is in layer-space pixels and
   divided by the layer's mean scale (an anisotropic scale paints a circle in content pixels). Masks: Paint moves a
   value towards the colour's Rec. 601 grey, Erase towards 0 (hide).
8. **Strokes that paint nothing** leave no entry and take back what `begin` changed (a promotion, a blob made owned).
   `set_sticker` and `replace_sticker_pixels` through `OfStamp` on an image stamp record the promotion even when the
   edit itself changes nothing (they return the sticker id, which must exist); a refused edit always takes the
   promotion back.
9. **Blob pictures and `replace_sticker_pixels`.** A same-size result on a `Picture(Blob)` layer is a `Structure` entry
   (the layer becomes owned: a structural change), not a tile patch; a tile patch needs an owned buffer on both sides.
10. **`sticker_stroke_dab` returns the changed rectangle in buffer (content) pixels**, not layer space (identical for
    an untransformed layer; the canvas of P7a maps it).
11. **`sticker_checkpoint` also takes the stickers of the named sticker stamps** (a Revert of a stamp needs its sticker);
    `restore_sticker_state` skips stamps deleted since the checkpoint (their layers cannot be rebuilt from a stamp
    object) and changes the linked layers' kind when a stamp goes back from sticker to text.
12. **`sticker_stroke_break`** (new, small): the next sample starts a new segment, as `stroke_break`. Used by the perf
    test; the canvas will need it for pen lifts.
13. **Bakes during a sticker stroke.** `next_final_bake` gives no job and `apply_final_bake` / `apply_wrap_preview`
    answer `Busy` / stale while a sticker stroke is open, exactly as during a texture stroke: a job made then would hold
    the live sticker's `Arc` and make the next dab copy the whole picture.
14. **The "before" snapshot of a stroke is taken only when `begin` changes the structure** (and before the change). A
    snapshot shares the sticker and its picture, so an unconditional one made every stroke copy the picture (found by
    test k3b-5).

## Numbers measured

- k3b tests (debug test profile): 14 pass in about 0.2 s; the recorded texture patch hash `0xb75b1fe0532ccc75` was
  measured on the unchanged history code first (same test, before any P3b change), then kept as the constant.
- Perf (`perf_k3b_stroke_begin_and_one_dab`, release, serial): `sticker_stroke_begin` on a 4096 x 4096 picture shared
  with a snapshot (the one copy) **14.7 ms** (budget 40); one dab of size 64 on a 2048 x 2048 picture (pixels only;
  the canvas composite is P7a's) **0.12 ms** (budget 2).
- `cargo test --workspace --no-fail-fast` (final source): **1,510 passed, 1 failed (WA20), 56 ignored** (`wte_core`
  930 / 1 / 32, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 24). Against the P3a check (1,496 / 1 / 55): +14
  `k3b_*`, +1 ignored `perf_k3b_*`. The red test is `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`
  only. Every existing `history`, `stroke` and `fill` test passes untouched on the generic patches.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 28,838,400 bytes, SHA-256 `a47250059b6e5404918402cb4e72e348668619060a68ef28933b03bb321397f3`
  (names this build only; see `baseline.md`).
- Identity sweep (8.5, `tools/objtabs/sweep.sh` with that exe and `tools/sticker/jobs_identity.txt`, off-screen, no
  focus taken) into `target\sticker\p3b\sweep\`: **21 / 21 `"ok": true`**; `compare_baseline.py` against the P0 baseline:
  **216 / 216 files the same, PASS** (`target\sticker\p3b\compare.txt`). Looked at `undo` (Split view: logo, WTE text and
  the two strokes on the checker sphere, Redo toast), `paint2d` (two blue strokes in the UV view) and `paint3d` (red
  stroke over two faces of the cube): as before.
