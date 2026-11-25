# Build note: P0 - Baseline and housekeeping (STICKER_PLAN 7 "P0")

Builder, 2026-11-25, branch `stickers` (from 7969fcb). No source code under `crates/*/src` changed.

## What was built

| Item (plan P0) | Result |
|---|---|
| Notes to LF, commit the notes and the plan | already done: the five notes and the plan were committed LF in 7969fcb (`git ls-files --eol`: `i/lf w/lf`); nothing to convert |
| Baseline | `docs/notes/sticker/baseline.md`: tests 1,411 / 1 (WA20) / 48 ignored, clippy 0, exe SHA-256 `9509252b…c3a3`; the 21 jobs of 8.5 all `"ok": true`, outputs in `target\sticker\baseline\sweep\` (git-ignored), exe kept as `target\sticker\baseline\wte_p0_baseline.exe` |
| Identity tooling (8.5) | `tools/sticker/jobs_identity.txt` (the regression set, for `tools/objtabs/sweep.sh`) and `tools/sticker/compare_baseline.py` (exports byte for byte, `.wtep` entries + manifest, screenshots in the view rectangle, "ok" regressions; volatile areas masked) |
| `.gitattributes` | created with exactly `*.psd binary`, `*.psb binary`, `*.wtes binary`, `*.wtep binary`; `git ls-files --eol` of every tracked file is identical with and without it (diffed) |
| PSD fixtures (8.3) | `crates/wte_core/tests/fixtures/psd/`: f1 (WTE-written input + PhotoCraft re-save), f2, f3, f4 + f4b, f5, f6, f7, f8 + f8b, f9, f10 (PSB), f11; each with PhotoCraft's merged PNG and per-layer PNGs; `README.md` (exe hash, every command line, what each file holds), `make_fixtures.sh` (the exact script), `gen_f1/` (f1's input generator, its own workspace). 179,185 bytes in all (limit 400 KB) |
| CLI save = GUI save? (B16) | `docs/notes/sticker/pc-save-paths.md`: **the same** in v0.2.0 (same importer, same `photocraft_io::export` with `ExportOptions::default()`, same `std::fs::write`); f1 stands for a GUI save of an unedited document; hand test 1 stays the proof for a real edit + Ctrl+S |
| `licenses/photocraft-MIT.txt` | the MIT text of PhotoCraft v0.2.0's `LICENSE-MIT` (identical at HEAD), CRLF -> LF, nothing else changed |
| ARCHITECTURE | 9.1: a paragraph on the fixtures and the identity baseline; 10: new "stickers" work-package table with the P0 row |
| `tools/sticker/psd_dump.py` | small read-only PSD structure printer (header, resource 1057, layer records, `lyid`, tagged keys, compression); used to write the fixture README; later packages may use it |

HANDOFF.md and `dist/HOW_TO_TEST.txt`: no change (plan section 10 gives P0 no HANDOFF / HOW_TO_TEST edit; the first
ones are P6b / P6a).

## Fixture facts later packages need (all measured with `psd_dump.py`)

- PhotoCraft v0.2.0 assigns `lyid` 1..n bottom to top to a file without ids and keeps them on re-save; a layer it adds
  gets the next id (f5: the type layer is `lyid` 4, WTE's three keep 1..3). It keeps names, order and layer rects.
- Every PhotoCraft layer record has `luni`, `lyid`, `iOpa = 255` and the default blending ranges
  (`(channels + 1) x 00 00 FF FF 00 00 FF FF`, 32 bytes for 3 colour channels). A group divider has no `iOpa`. A
  `Background` layer has flags 0x01 and `lspf`. All channels and merged images are RLE (no ZIP from v0.2.0's CLI).
- Image resources written: 1005, 1037, 1049, 1057 (`hasRealMergedData = 1`); f9 also 1006, 1045, 1077.
- 16-bit (f7): the plain layer section is empty and the layers sit in the global `Lr16` block.
- f9: header has 5 channels; merged = RGB, transparency, then the saved alpha.
- Masks (f4/f4b): user mask channel -2, `default 0`, mask rect [11,7,53,41]; `layersToFiles` PNGs have the mask
  (and in f4 the stroke) applied, so they are the *visible* layer, not the raw channels.
- f1's input from WTE's texture writer has flags 0x08, empty blending ranges, no resource 1057 and no `lyid`.
- Latest PhotoCraft (HEAD, read only): it asks before overwriting its own file when another app changed it (#6596),
  see `pc-save-paths.md` section 4; relevant for P9's "Send to PhotoCraft" and hand tests 4 and 6.

## Deviations from the plan, with reasons

1. **f4 uses an elliptical `select.rect` (feather 2) instead of `select.subject`**: an exact, reproducible mask; the
   subject heuristic would make the mask depend on PhotoCraft's algorithm. Said in the fixture README.
2. **f1's input is written by a small separate program** (`gen_f1/`, own `[workspace]`, never compiled by the WTE
   workspace) because P0 may not change any existing file under `crates/` and `write_psd` has no other entry point that
   writes a three-layer file. The plan's checker rule ("under `crates/` only `crates/wte_core/tests/fixtures/psd/**` was
   added") holds: the generator lives inside that folder. Cargo does not auto-discover it (it is two levels below
   `tests/`); the final workspace test run has the same test binaries as the baseline.
3. **Things the plan lists for P0 that were already true**: the notes were LF and committed.
4. **Extra**: `compare_baseline.py` and `jobs_identity.txt` are written now (8.5 names the tool but no package); later
   builders and checkers need them from P1 on. `frame_spans.py` (8.4) is not written: it needs the `sticker.` spans that
   P6a introduces.
5. **Volatile screenshot areas**: the plan says screenshots are pixel-identical inside the view rectangle. A second run
   of the same exe showed two non-drawing differences (the companion dialog's random port, pairing code and date; the
   export dialog's folder path, which contains the sweep folder's name). They are masked in `compare_baseline.py`; with
   them masked, two runs give 216 / 216 identical files.

## Numbers measured

- Baseline tests: 1,411 passed / 1 failed (WA20) / 48 ignored; clippy 0; cold release build 12 min 05 s; exe 28,050,944
  bytes, SHA-256 `9509252b128145af28278d3b0b1affb56110b0534522a4ddf604cb491a28c3a3`.
- Sweep: 21 / 21 ok; 216 compared files; workflow_path 71 s, total about 3 min; second run identical after masking.
- Fixtures: 14 PSD / PSB files (3.4 KB - 22.2 KB; 52 files with the PNGs), 13 merged PNGs, 25 layer PNGs; all reproduced byte for byte by a
  second run of `make_fixtures.sh` (52 / 52 SHA-256 equal); `gen_f1` output identical on a second run.
- `photocraft-cli` runs: each under 0.2 s; no window opened; `photocraft.exe` never started.

## End-of-package checks (after the P0 changes)

- `cargo test --workspace --no-fail-fast`: `wte_core` 831 passed / 1 failed (**WA20**) / 24 ignored; integration tests
  15 + 9 + 8 + 24 passed; `wte_app` 523 passed / **1 failed** / 24 ignored. The second red is
  `companion::server::tests::a_code_attempt_does_not_hold_the_auth_lock` (`server.rs:1196`, "An established connection
  was aborted ... (os error 10053)" instead of the expected "code 2" refusal). It is **not caused by P0** (no source file
  changed; the release exe built from this tree has the same SHA-256 as the baseline exe) and it **passes alone 5 / 5**
  (11-15 s each). It is load-sensitive: the test runs 300,000 PBKDF2 iterations in a debug build while the server's
  hello / idle deadlines are 4-5 s; during the final run the machine was at about 80 % CPU from another program the
  user had open (the `wte_core` suite took 403 s instead of 100 s at the baseline), so the server gave up before the
  client read its refusal. Listed as an open issue (a pre-existing timing test; widening its deadline is a code change
  outside P0).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe SHA-256 unchanged (`9509252b…c3a3`).
- Scenarios: P0 adds none; the 21 regression jobs are the baseline itself (all ok; pictures looked at: `stamp`,
  `workflow_path`, `companion` dialog; fixture contact sheet checked by eye).
