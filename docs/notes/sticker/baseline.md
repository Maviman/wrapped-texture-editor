# Sticker work: P0 baseline (2026-11-25)

Branch `stickers` @ 7969fcb (= `main` b2fb8f5, v0.3.0, plus the plan and the research notes; no code change). Machine:
the user's desktop (NVIDIA RTX 5070, Vulkan), toolchain rustc 1.99.1 `x86_64-pc-windows-gnu` through
`G:\Claude_Resources\rust\cargo-gnu.cmd`. Raw logs and outputs: `target\sticker\baseline\` (git-ignored).

## Tests, clippy, build

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | **1,411 passed, 1 failed (WA20), 48 ignored** (first build of this worktree's target: 6 min 40 s) |
| per binary | `wte_app` unit 524 / 0 / 24 ignored; `wte_core` unit 831 / **1** / 24 ignored; `tests\brush_tips.rs` 15; `tests\end_to_end.rs` 9; `tests\objects.rs` 8; `tests\project_roundtrip.rs` 24; doc tests 0 |
| the red test | `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` (`crates\wte_core\src\wrap_tests.rs:2070`), the known WA20 |
| `cargo clippy --workspace --all-targets` | **0 warnings** |
| `cargo build --release -p wte_app` | OK (12 min 05 s from a cold release target) |
| exe | `target\release\wrapped_texture_editor.exe`, 28,050,944 bytes, SHA-256 `9509252b128145af28278d3b0b1affb56110b0534522a4ddf604cb491a28c3a3`; kept as `target\sticker\baseline\wte_p0_baseline.exe` |

The numbers equal the plan's expectation (section 1: "near 1,411 / 1 WA20 / 48").

**The exe hash is not reproducible (P0 check).** Two release builds of the same source gave different SHA-256s
(`3004da74...` and `0cfd37bb...`, both 28,050,944 bytes; the unchanged 7969fcb source rebuilt gave `f9a8abee...`).
The hash only names the exe that made the baseline; "same behaviour" is proved by the identity sweep below, never by
comparing exe hashes.

## Identity baseline (plan 8.5)

- Jobs: `tools/sticker/jobs_identity.txt` = `smoke default paint3d paint2d stamp text preview export export_run roundtrip
  undo layers workflow_path wrap_surface text_warp export_flow export_checks side_tabs objects pen companion`
  (`workflow_path` on `G:\Claude_Resources\samples\khronos-gltf\ABeautifulGame.glb`), all at 1280x800.
- Run: `sh tools/objtabs/sweep.sh target/release/wrapped_texture_editor.exe target/sticker/baseline/sweep
  tools/sticker/jobs_identity.txt` (off-screen, no focus; the exe was checked to exist first).
- Result: **21 / 21 `"ok": true`**, exit 0 each. Seconds: smoke 3, default 3, paint3d 2, paint2d 2, stamp 3, text 3,
  preview 4, export 3, export_run 4, roundtrip 3, undo 2, layers 4, workflow_path 71, wrap_surface 9, text_warp 6,
  export_flow 17, export_checks 5, side_tabs 3, objects 3, pen 6, companion 6.
- Outputs: `target\sticker\baseline\sweep\<job>\` (439 MB; 216 compared files: screenshots, export files, four saved
  projects `objects/shot.wtep`, `roundtrip/shot.wtep`, `workflow_path/shot.wtep`, `wrap_surface/shot_wrap.wtep`);
  SHA-256 of every file in `target\sticker\baseline\sweep_hashes.txt`.
- Compare later builds with: `python -E -P tools/sticker/compare_baseline.py target/sticker/baseline/sweep <NEW>`
  (rules in the script's docstring: exports byte for byte; `.wtep` entries byte for byte and the manifest without
  `format_version`, `min_reader_version`, `app_version`, `saved_unix`; screenshots inside the view rectangle x 44..980,
  y 51..775; a job ok before and not ok now fails).

### Is the baseline reproducible? (measured)

The same sweep was run a second time with the kept copy `wte_p0_baseline.exe` into `target\sticker\baseline\sweep_rerun\`.
Before masking, exactly two screenshots differed, both for reasons that have nothing to do with drawing:

- `companion/shot_companion_dialog.png`: the self-test's random port ("listening on 127.0.0.1:59463" vs ":59560"), the
  random pairing code, and (on another day) the "since <date>" of the paired tablet;
- `export/shot.png`: the export dialog's folder field shows the sweep's own output path.

These areas are masked in `compare_baseline.py` (`VOLATILE`). After that: **216 / 216 files identical** (every export
byte for byte, every `.wtep` entry and manifest, every screenshot's view rectangle). So any difference a later package
sees is real. A negative check (one changed byte in an `.ora` export, one changed pixel in a view rectangle, one extra
stamp in a manifest) gave three FAILs, as it should.

## Line endings at the start (for later packages)

`git ls-files --eol` on this commit: 344 files `i/lf`, 29 `i/crlf`, 5 `i/mixed`, 13 binary, 1 empty. The four research
notes the plan called CRLF (`pc-features`, `pc-interop`, `win-dnd`, `wte-model`) were already LF in the plan commit
7969fcb (index and working tree), so P0 had nothing to convert.

**Existing CRLF sources** (in the index on `main`; `core.autocrlf=false` keeps them CRLF on disk): `crates/wte_core/Cargo.toml`;
`crates/wte_core/src/{brush_tip,error,samples,tip_field}.rs`, `import/{obj,resolver}.rs`,
`project/{manifest,migrate,mod,save}.rs`, `text/{frame,gaps,layout,mod,raster,styled}.rs`;
`crates/wte_app/src/{cli,crash}.rs`, `gpu/scene.rs`, `view/{view2d,view3d}.rs`. Several of them are files later packages
must edit (`project/*` in P4, `text/mod.rs`, `error.rs`). P0 changes none of them (its rule: nothing existing under
`crates/` changes). A builder who edits one should keep its line endings as they are (a whole-file CRLF -> LF switch would
hide the real diff); the gate's LF audit (plan 8.6 item 7) should therefore look at **new** files and at lines the branch
adds, not at these pre-existing files. Noted as an open point for the main session.

`.gitattributes` (new) holds exactly `*.psd binary`, `*.psb binary`, `*.wtes binary`, `*.wtep binary`; no tracked file
has one of these extensions on `main`, and `git ls-files --eol` reports the same `i/` and `w/` state for every existing
file after it was added (only the new fixtures show `attr/binary`).
