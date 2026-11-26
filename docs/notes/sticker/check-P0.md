# Check note: P0 - Baseline and housekeeping (STICKER_PLAN 7 "P0")

Independent checker, 2026-11-25, branch `stickers`, builder commit c86ba23. Verdict: **pass after fixes**.

## Item by item (plan 7 "P0" and its checker line)

| Plan item | Checked how | Result |
|---|---|---|
| Notes LF, the five notes and the plan committed | `git ls-files --eol docs/notes/sticker docs/plans/STICKER_PLAN.md` | all `i/lf w/lf`; committed in 7969fcb. Every file added by c86ba23 is `i/lf w/lf` or binary |
| Baseline note, counts, clippy, exe hash, outputs of 8.5 | read `baseline.md`; `target\sticker\baseline\sweep\` has the 21 jobs of 8.5 (list equal to 8.5 word for word), shots, exports, four `.wtep` | present. The exe hash claim needed a correction (finding 3) |
| `.gitattributes` exactly the four rules, no `text=` | `od -c`; `git ls-tree -r 7969fcb` has no `.psd/.psb/.wtes/.wtep` file, so no existing file's `i/ w/ attr/` state can change | OK |
| PSD fixtures of 8.3 with README | f1-f11 present with merged + per-layer PNGs; README has every command line | OK; all f1-f11 rows match their "Content" column (dumped with `psd_dump.py` and with my own small decoder) |
| Fixtures < 400 KB | `du -cb` without `gen_f1/` | 181,005 bytes (194,749 with f12 added) |
| Each README line reproduces its fixture | ran `make_fixtures.sh` **twice from my own scratch folder** and `gen_f1` (copied to scratch, own target dir, `--offline --release`) | `f1_wte_in.psd` SHA-256 `86edd684...` reproduced; all 52 committed outputs byte-identical to both of my runs. Found the script only worked from the builder's folder (finding 4) |
| Under `crates/` only `crates/wte_core/tests/fixtures/psd/**` added, no existing file changed | `git diff --name-status 7969fcb c86ba23 -- crates` | OK for the builder's commit. The checker changed one test (finding 2, a deliberate, recorded exception) |
| CLI save = GUI save, read independently | read pc020 `apps/photocraft-cli/src/lib.rs:109,139-167`, `crates/automation/src/files.rs:19-69`, `crates/io/src/lib.rs:110-149`, `crates/ui-egui/src/{file_open.rs:29-40, lib.rs:418-532, menus.rs:220-227,402-408}`, `apps/photocraft/src/services.rs:99-125`, `crates/engine/src/color_cmds.rs:236-276,917-920` (`ask_on_missing` defaults to false, `:142`), `crates/io/src/psd_export.rs:80-110,140-151,545-590`, `crates/doc/src/lib.rs:424,505-515` | the verdict "same exporter, same options" is right. One statement about layer ids was wrong (finding 1) |
| `licenses/photocraft-MIT.txt` | `diff` against `PCHEAD:LICENSE-MIT` | identical (LF); the copyright line is the one plan 4.0 asks for. No "ArtCraft" anywhere else outside licences, notes and the plan (`git grep`) |

## Findings

1. **Major (fixed in docs and fixtures; the design consequence is for P5).** PhotoCraft does **not** give a new layer an
   id "above the largest" (plan section 1, `pc-save-paths.md` table, `build-P0.md`). Both v0.2.0 (`psd_export.rs:577-590`)
   and HEAD (`psd_export/document.rs` `index_layer_ids_and_text`) keep unique stored ids and give every other layer the
   **lowest free** id counting from 1. Proved with the real CLI: new fixture **f12** = `f1_pc.psd` with the WTE layer
   `text` (`lyid` 2) deleted and a layer `pc new` added in the same run; `pc new` comes back with **`lyid` 2**. Under the
   5.4 table that reads as "`lyid` in `layer_map`, the pixels differ", so P5 as planned would turn the deleted text
   layer into the new picture and keep the text's WTE layer styles on it. Fixed: section 1 corrected; 5.4 has a new
   "`lyid` reuse" bullet with a suggested rule (known id + different pixels + different name = deleted + new), P5's test
   list names f12, 8.3 has an f12 row, review log line added; `pc-save-paths.md` and `build-P0.md` corrected; f12 with
   its merged and per-layer PNGs is in the fixtures folder and in `make_fixtures.sh` (reproduced twice, byte-identical).
   The exact rule is P5's to choose and test.
2. **Major (fixed, test-only).** `companion::server::tests::a_code_attempt_does_not_hold_the_auth_lock` failed in the
   builder's final run and again in my first full run (os error 10053), so every later package would show a second red
   (rule 0.4.7). Cause: 300,000 PBKDF2 iterations in a debug build take 5-7 s on each side, alone the test took 10-15 s;
   the client's PBKDF2 must finish inside the server's 10 s `HANDSHAKE_TIMEOUT`, and under load (cargo runs the app's
   tests in parallel) it does not. Fix: 50,000 iterations (`server.rs`, inside `#[cfg(test)]`; about 0.8 s per side).
   The test still catches the bug it guards: a mutation that holds `hub.auth` around `hs.check` fails with "the auth lock
   was held for 1.18 s" (limit 30 ms). Alone: 3/3 pass in about 2 s; full workspace run: passes. This touches an existing
   file under `crates/`, which P0's own rule forbids; done anyway because the change is test-only and rule 0.4.7 is the
   stronger rule. The main session may revert it.
3. **Minor (fixed in the note).** "The release exe built after P0 has the same SHA-256 as the baseline" proves nothing:
   the builder's build did not recompile anything. Release builds here are not bit-reproducible: the same source gave
   `3004da74...` and `0cfd37bb...`, and the untouched 7969fcb source rebuilt gave `f9a8abee...` (all 28,050,944 bytes).
   `baseline.md` now says the hash only names the baseline exe; identity is proved by the sweep.
4. **Minor (fixed).** `make_fixtures.sh` wrote the PSDs to `./out` but the per-layer PNGs to the builder's absolute
   scratch path, and its usage line said "from this folder" (the repository) while the README said the scratch folder.
   Run anywhere else, it would have mixed outputs or written into the repository. Now `OUT` is the absolute path of
   `./out`; usage and README say: copy the script and `f1_wte_in.psd` into an empty scratch folder and run it there.
5. **Minor (fixed).** `compare_baseline.py` was weaker than plan 8.5: it ignored `min_reader_version` and reported any
   manifest difference other than `stamps` / `sets` as a NOTE only. Plan 2.11 requires a project with no new feature to
   keep exactly today's reader version, and 8.5 allows the manifest to differ only in format and app version. Now every
   manifest key except `format_version`, `app_version`, `saved_unix` must be equal, and a key present on one side only
   FAILs. Negative checks: `min_reader_version` 2 -> 4 FAILs; an added `"stickers": []` FAILs; base vs the builder's rerun
   still PASSes 216/216.
6. **Minor (fixed).** A third volatile screenshot area: my sweep showed the toast "Info Undo: Erase" in
   `companion/shot.png` (box 846,734-977,775) which the baseline lacks. Toasts fade after 4 s of wall-clock time
   (`toasts.rs`), so on a slower run the last toast is still there. Masked in `VOLATILE` (760,725-980,775) for that shot.
   Checkers of later packages: a FAIL box in the bottom-right toast corner of another shot is probably the same thing;
   look at the picture before calling it a regression.
7. **Note (not changed).** `gen_f1/Cargo.toml` has a `[dependencies]` line (`wte_core` by path, `fontdue = "0.9.5"`, the
   version already in `Cargo.lock`): no new crate, but the gate's "no new `[dependencies]` line anywhere" grep (8.6 item
   5) will list it; it is fixture tooling outside the workspace.
8. **Note.** The CRLF list in `baseline.md` is right (23 files under `crates/`, 29 `i/crlf` in the repo, 5 `i/mixed`).
   The builder's open point on CRLF sources stands for the main session.

What PhotoCraft v0.2.0 does with RGB under alpha 0 (asked by 8.3 f1): my own channel decoder shows `f1_wte_in` and
`f1_pc` have byte-identical layer channels (rects equal, the 252 coloured pixels under alpha 0 in `photo` kept), so a
CLI re-save does not change an untouched layer at all; f12's untouched `photo` and `shape` are identical too.

## Runs (after the fixes)

- `cargo test --workspace --no-fail-fast`: **1,411 passed, 1 failed (WA20), 48 ignored** (`wte_app` 524/0/24, `wte_core`
  831/1/24, integration 15 + 9 + 8 + 24). First run before the fix: 1,410 / 2 (WA20 + the companion test) / 48.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK (exe `0cfd37bb...`, see finding 3).
- Identity sweep (8.5, 21 jobs, `tools/objtabs/sweep.sh` with that exe into `target\sticker\check\sweep\`): 21/21
  `"ok": true`; `compare_baseline.py` against the P0 baseline: **216/216 same, PASS** (report
  `target\sticker\check\compare.txt`). Looked at: `workflow_path` (Split view, chess set with strokes and stamps, UV view
  of the board), the companion and export shots with the masks drawn, the stamp shot with the view rectangle drawn (the
  rectangle is exactly the central 3D view).
- `photocraft-cli.exe` (console) only; `photocraft.exe` never started; nothing took the focus.

## Open for the main session

- Finding 1: P5 must choose and test the `lyid`-reuse rule (5.4); the suggested one costs a renamed-and-repainted
  layer its WTE styles.
- Finding 2: the test change in `server.rs` breaks P0's "no existing file under `crates/`" rule on purpose; keep or
  revert.
- The builder's CRLF point (keep line endings of the 23 CRLF sources; the LF audit looks at new files and added lines).
