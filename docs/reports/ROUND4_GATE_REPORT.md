# Round 4 gate report — Wrapped Texture Editor

Date: 2026-11-20. Gate = package Z (plan section 3.3), stages 1-3. Paths are relative to
`E:\claude_programs\wrapped_texture_editor\target\round4\` unless they start with `round3\` (=
`...\target\round3\`) or are absolute. Stage notes: `notes\gate1.md` (merge, scenarios, built-ins), `notes\gate2.md`
(nothing got worse: sweep, fuzz, exports, projects), `notes\gate3.md` (performance, final confirmation).

**Final exe: `wte_round4.exe`** (= `wte_gate.exe` = `work\merged\target\release\wrapped_texture_editor.exe`),
**SHA-256 7695C240609D53E65DF157A0FFDEF48EC7E3F545D9FC7B537109AF5DC188E88B**, source `work\merged`.
Round-3 reference: `round3\wte_round3.exe` (SHA-256 7D0E0B57...3E39).

**Runs with older exes.** Stages 1 and 2 used `144E3C12...0115` (gate 1's exe: the merged source before the gate-3
changes). In stage 3 the brush runs (`runs_gate3\btp_*`, `hover_*`) and the text runs `runs_gate3\text_1`, `text_2`
used `434BD322...0B7`; the text runs `text_3`..`text_5` and the round-3 comparison runs (`wf_*`, `fuzz_*`) used
`1FD43034...2ED2`. Those two differ from the final exe only by one assert of the gate's measuring scenario
`text_preview_path` and by `#[cfg(test)]` probes; 144E3C12 differs by the gate-3 changes listed in part 5 (an off-by-
default frame log, one panel rect, scenario steps). Everything in `runs_gate3\final\` used the final exe.

## Summary

Round 4 delivers what the user asked for. **Brush:** a Type (Circular as before, Sphere = a 3D brush object whose
default is a ball), the built-in Cube and Cylinder, any closed OBJ / glTF / GLB / FBX model as the brush shape with a
dialog that closes openings (flat caps, rounded caps, fill all hollows, only closed parts), Turn / Tilt / Roll, Offset,
Solid or Through, the Brush tab with a draggable preview, the shape cursor, Ctrl+wheel size and Shift+wheel turn, and
strokes that keep the window responsive. **Text:** the fonts of this machine with a picker that shows samples,
spacing for every single gap ("segmented"), bold and slant (real faces first, thickening / slant only for what a face
lacks), 15 warp styles, text along Arc / Circle / Curve with draggable points, the Text tab (Character, Warp, Path),
the four-row Text block with an Angle field, the see-through text under the pointer before placing, and project
format 2. 18 of 19 items are done; R4-3 was cut by the user's answer.

Final confirmation: 1,040 tests pass, 0 fail (32 ignored: GPU tests and timing probes); clippy clean; all 37 scenarios and the 12 size variants pass; the round-3 repros
and a 16-model sweep sample pass; the 157-model sweep (gate 2) gives round 3's 149 ok / 8 intended failures.

**Missed limits (numbers in part 2):** P7 (stroke work per walk step on the 1.5-million-triangle model, 1.8-1.9 x),
P8d (backlog there at Size 50, 782-1,623 ms vs 350 ms), P1 (c) 2048 against P0's file (1.075 x; 1.01 x of round 3 back
to back), the classic text painter B1 against P0's 2.041-ms line (2.13-2.32 ms; 1.03-1.05 x of round 3 back to back),
3 of 20 warp Finals on ABeautifulGame over 1,500 ms, and the font picker's opening frame in 1 of 5 runs (179 ms).
P13 on the two texture models sits at the 60 Hz refresh (2 of 8 medians 0.05 / 0.15 ms above the line).

---

## 1. Items R4-1 to R4-19

| Item | Status | Evidence (tests; scenario runs under `runs_gate3\final\runs\`) |
|---|---|---|
| R4-1 tabs Layers / Brush / Text | done | `panels::tests` tab-rule test (S2), B4's A11-A13 (`control_sweep::panel_tests`), scenario `text_tab` (the three-tab step of plan 3.3, five asserts), `brush_tab`; side panel 300 pt at 1280 x 800, 900 x 600, 640 x 480 (`final\s900`, `final\s640`) |
| R4-2 Type Circular / Sphere, Offset | done | C1 (legacy and shared stroke hashes identical, `stroke_bench_tips` and `stroke::tests`), `tests\brush_tips.rs` (15 tests, incl. `strokes_save_identically_through_both_entry_points`), `tools::brush::tests`; scenario `brush_types` |
| R4-3 "Stay on connected surface" | **cut** (plan section 7 item 1, the user's answer) | not built; no flood code, no topology field |
| R4-4 Cube, Cylinder, orientation, Solid / Through | done | `tip_field::tests`, `brush_tips.rs` (field tips, Through), `tools::brush::tests` (rotation, wheel gestures); scenarios `brush_types`, `brush_tab`; P5 / P6 met |
| R4-5 shape from a model file (+ closing openings, section 7 item 5) | done | `tip_bake::tests` (F2-F25, `open_box` with each mode, cup rounded > flat, non-planar hole), `brush_flows::tests` (A10, A16 four formats, A20), scenario `brush_shape` (OBJ / GLB / FBX cube, the open box closed with Flat caps through the dialog; `_dialog.png`); P10 met. Deviations: F20 on baked shapes 2.27 / 3.70 cells vs 1.5 (contract request B1-1, part 5), F8 within 3 % (B1-2) |
| R4-6 the Brush tab | done | B4's panel tests (A11-A13, first line, notes), scenario `brush_tab` (`shot.png`: preview, Turn 30 / Tilt -20, Openings "Flat caps", the "!" marker) |
| R4-7 cursor and hints | done | `tools::brush::tests` (cursor segments, `only_the_ball_shows_the_inner_hardness_ring`), B6's A14 renderer test; A15 (GPU) is `#[ignore]` without a GPU in the test run; scenarios `brush_types`, `brush_tab`, `brush_hover` (Cube cursor 108 footprint / 876 wire segments) |
| R4-8 paced strokes | done; **P8d missed on ABeautifulGame** | `tools::brush::tests` A4, A5 (replaceable clock), A17, A19; P8a / P8b / P8c met everywhere; P8d met on the texture models (5-52 ms), missed on ABeautifulGame (782-1,623 ms) |
| R4-9 one search per dab | done; **P7 missed**, P1 (c) 2048 against the file | C2, C4; P2, P3, P13 on ABeautifulGame met (part 2, part 3) |
| R4-10 per-gap spacing | done; classic painter B1 line missed (part 2) | `text::gaps`, `layout` tests (8 round-3 hashes bit-identical, `plain_text_is_bit_identical`), T6 gap-control panel tests, scenario `text_gaps` (`_gap3d.png`) |
| R4-11 font picker | done; first open slow in 1 of 5 runs | `font_catalog::tests` (machine fonts, samples), `text_ops::tests` (sample job 15.1 ms), `the_font_picker_filters_and_takes_a_font`, `the_font_button_rect_opens_the_picker`; scenario `text_fonts` (78 families, Arial present; `_picker.png`) |
| R4-12 bold and slant | done | `face_traits` tests, `styled` tests, `bold_formats_the_whole_text`; scenarios `text_styles`, `text_fonts` (Arial gets "Bold", Ubuntu is thickened) |
| R4-13 warp | done; 3 of 20 ABeautifulGame Finals over 1,500 ms | `text::warp` tests (15 styles), `styled` warp tests, Warp page tests; scenario `text_warp` (`_warp3d.png`) |
| R4-14 text along Arc / Circle / Curve | done | `text::path` tests, `a_gap_on_a_circle_turns_the_next_letter`, `tools::text_path::tests`, Path page tests; scenario `text_path` (point drag; `_circle3d.png`); Circle Final on ABeautifulGame 716-841 ms (1,500) |
| R4-15 four-row Text block, Text tab, Angle | done | T6's `text_panel_tests` (Angle wraps 270 -> -90, rows, pages), scenario `text_tab` (`_page_character|_page_warp|_page_path.png`) |
| R4-16 the text stays; bounded cost | done | "Frame" and "Document" tests of TEXT 8.1 incl. `editing_a_plain_text_gives_the_round_3_placement`; Preview 4.38 ms (8), `set_stamp_source` Preview 4.60 ms (16), twenty edits on 8192 longest 42-46 ms (105-109), peak memory 116.6 MB (250) |
| R4-17 text visible before placing | done | `hover_with_the_text_tool_caches_a_preview_of_the_draft`, `a_changed_draft_replaces_the_hover_preview`, `add_stamp_reuses_only_a_full_preview`; scenario `text_tab` hover assert + `_hover.png` |
| R4-18 project format 2 | done | `tests\project_roundtrip.rs` (21 tests, frozen v1 fixture, unknown warp / path warnings, `the_effects_change_the_saved_pixels`), scenarios `roundtrip`, `export_run` (two gaps + Arc saved, reopened, exported; check_exports PASS); gate 2 section 5: round-3 projects open in round 4, round-4 projects without new features open in round 3 with its "newer version" warning |
| R4-19 merge, scenarios, measurements, report | done | `notes\gate1.md`, `gate2.md`, `gate3.md`, this report |

## 2. Performance limits

Release build, idle machine, one run at a time, 1280 x 800. B = `perf\baseline.txt` / `perf\text_baseline.txt` (P0,
2026-11-18). "R3 now" = round-3 code measured in the same session (P0's round-3 bench binary, T4's round-3 probe, or
`wte_round3.exe`). Raw: `runs_gate3\core_bench1.txt`, `classic_b2b.txt`, `p0_stroke_bench_r3.txt`,
`twenty_edits_b2b.txt`, `app_timing_tests.txt`, `btp_analysis.txt`, run folders `runs_gate3\*`.

### Brush (FEATURE_BRUSH 8.4; P4 and P9 dropped with R4-3)

| # | Limit | Measured | Baseline | Verdict |
|---|---|---|---|---|
| P1 (a) tex_8192, Ball/SameSide, ms per dab, 50 / 512 / 2048 | <= 1.05 x B: 0.2532 / 3.0140 / 41.4767 | 0.1728 / 2.3294 / 31.1305 | B 0.2411 / 2.8705 / 39.5016; R3 now 0.1537 / 2.1135 / 31.3440 | met (against R3 now 1.12 / 1.10 / 0.99 x; the in-run round-3 loop compiled in round 4 gives 0.1689 / 2.3270 / 31.1288) |
| P1 (c) tex_4096, 50 / 512 / 2048 | 0.1843 / 2.3340 / 24.5473 | 0.1802 / 2.2296 / **25.1221** | B 0.1755 / 2.2229 / 23.3784; R3 now 0.1786 / 2.2404 / 24.8653 | **2048 missed against B (1.075 x)**; 1.01 x R3 now |
| P2 (b) ABeautifulGame 50: query phase per dab | <= 0.35 x B = 0.0167 ms | 0.0137 ms | 0.0476 | met |
| P3 (b) 50: total per dab | <= 1.00 x B = 0.608 ms | 0.5697 ms | R3 now 0.6061 | met |
| P5 (a) Cube / Ball-All, 50 texels (h 0.8 / 1.0) | <= 1.5 x | 1.34 x / 1.19 x | | met |
| P5 (a) 512 / 2048 | <= 3.0 x (3.5 at h 0.8) | 1.28 / 1.22 x; 0.92 / 0.75 x | | met |
| P6 (a) Cube | <= 6 ms at 512, <= 60 ms at 2048 | 2.94 ms, 28.27 ms | | met |
| P7 (b) Size 50, Circular, stroke work per walk step | <= 1.10 x B | **6.42 / 6.89 ms per step** (235 steps, 1,508 / 1,620 ms) | R3 now (P0 exe, same path) 3.51 / 3.74 ms per step (377 steps, 1,322 / 1,409 ms) | **missed (1.8-1.9 x per step; 1.14 x per stroke)** |
| P8a every stroke: longest frame of stroke work | <= 100 ms + longest sample | (a) <= 23.3 ms; (c) <= 14.1 ms; (b) Size 50 152-206 ms with samples of 88-132 ms; (b) Size 512 = the single sample | | met |
| P8b release to end | Size 50 <= 400 ms; Size 512 <= 1,000 ms | Size 50: <= 9.6 ms; Size 512: <= 23.3 ms | | met |
| P8c pointer step: Circular vs B; Sphere / Circular; Cube / Circular | 1.10 x; 1.3 x; 1.6 x (50) / 3.0 x (512) | (b) Circular 1,770 / 1,932 ms vs B now 2,647 / 2,132 ms (0.67-0.91 x); Sphere 1.24 / 1.13 x; Cube 1.09 / 0.97 x at 50, 1.04 / 1.10 x at 512. (a) Circular 474 / 458 ms vs B (Size 50, `perf\runs_p13`) 622 / 800 ms; Sphere 1.04 / 1.15 x; Cube 0.98 / 1.04 x, 0.69 / 0.61 x. (c) Circular 463 / 463; Sphere 1.04 / 0.99 x; Cube 1.02 / 1.00 x, 0.97 / 0.99 x | | met |
| P8d largest backlog, Size 50 | <= 350 ms | (a) 17.9-51.7 ms; (c) 5.3-7.3 ms; **(b) 782-1,623 ms** | | **missed on (b)** |
| P10 load to ready | cube 1 s; 500k-face sphere 8 s; two-pass 2 s; Cancel 1 s | core: 0.40 s; 3.76 s (1.68 s import); 0.37 s; 0.20-0.21 s. App: OBJ / GLB / FBX cube 0.54 / 0.51 / 0.52 s | | met |
| P11 overlays + side panel with a shape vs without; side panel idle | +1 ms; 1.5 ms | 1.71 / 1.68 vs 0.78 / 0.78 ms (+0.93 / +0.90); idle median 0.77, max 0.97 / 1.03 ms | 0.9-1.1 ms (BRUSH 3.8) | met |
| P12 frames > 100 ms on (b) | listed, no limit | part 3 | | listed |
| P13 (b) median drag frame, Size 50 | Circular <= 27.4 ms; Sphere <= 27.0 ms | Circular 17.62 / 20.18 ms; Sphere 18.05 / 17.72 ms | B now (P0 exe, first stroke) 25.09 / 27.55 and 26.47 / 26.85 ms | met |
| P13 (a), (c) | <= 16.7 ms or the baseline (a: 16.77 / 16.65) | (a) Circular 16.82 / 16.69, Sphere 16.62 / 16.18; (c) Circular 16.66 / 16.85, Sphere 16.69 / 16.54 ms | at the 60 Hz refresh | **2 of 8 medians 0.05 / 0.15 ms above** (vsync jitter; the frames wait for the display) |

### Text (FEATURE_TEXT 8.4; median of 5)

| Measurement | Limit | Measured | Verdict |
|---|---|---|---|
| Classic painter B1 "WTE 2026" at 256 with mips | within 5 % of B1 1.944 ms = 2.041 | **2.197 / 2.324 / 2.255 ms** (R3 now 2.142 / 2.229: 1.03-1.05 x) | **missed against B1**; within 5 % of round 3 measured back to back |
| B2 60 letters at 128 with outline / B3 16 em at 512 | 7.230 / 15.501 ms | 6.964-7.136 / 14.942-15.214 ms | met |
| Classic with 3 gap values at 256 | 2.041 ms | **2.131 ms** | **missed** (same cause as B1) |
| `text_frame` 200 letters, 4 lines, Flag, 128 | 5 ms | 4.04 | met |
| Preview Arc 50 % at 512 (+ mips): plain / thickening 2 % / outline 6 % | 8 / 12 / 45 ms | 4.38 / 5.27 / 7.51 | met |
| Preview 40 letters Flag at 256 | 12 ms | 3.95 | met |
| Final Arc 50 % at 512: plain / thickening / outline | 40 / 60 / 250 ms | 3.70 / 7.32 / 18.20 | met |
| Final default Circle at 512: plain / outline | 50 / 350 ms | 3.58 / 21.89 | met |
| Final 16 em full Circle at 512: plain / outline; Preview with outline | 150 / 1,200; 60 ms | 13.07 / 89.26; 9.31 | met |
| Peak memory of that Final with / without outline | 250 / 120 MB | 116.6 / 62.5 MB | met |
| `text_frame` too-large fallback (512 refused + 256) | 1 ms | 0.080 (document 0.076) | met |
| Final 200 letters Flag at 128 | 40 ms | 7.50 | met |
| `set_stamp_source(.., Preview)` 40 styled letters, sample sphere | 16 ms | 4.60 | met |
| Font sample job, all families, warm cache | 0.5 s | 15.1 ms (app), 13.2 ms warm / 50.1 ms cold (core), 78 families | met |
| B4 first fontdue parse | reported, not slower than round 3 | Arial 22.0-22.7 ms (R3 now 22.0 / 22.1); mingliub.ttc 1,196-1,246 ms (R3 now 1,170 / 1,179: 1.02-1.06 x) | reported |
| In-app: `text_warp` on ABeautifulGame, every Final | 1,500 ms | 5 runs, 3D: Arc 2,618 / 761 / 876 / 1,503 / 760; Flag 1,754 / 775 / 974 / 820 / 731; Fisheye 1,033 / 732 / 827 / 784 / 641; Arc 1,024 / 1,011 / 838 / 846 / 808 ms (medians 876 / 820 / 784 / 846); 2D 84-107 ms | **3 of the 20 3D Finals over** (run 1, cold after the brush runs: 2,618, 1,754; run 4: 1,503) |
| In-app: Preview frames on ABeautifulGame (`text_preview_path`) | no frame > 100 ms outside the bake segment | dispatch <= 8.8-11.8 ms; the frames over 100 ms are the throttled preview bakes inside "scene sync" (390-520 ms every ~23 frames) and one 940-ms frame of the self-test runner | met |
| In-app: twenty Preview edits on the 8192 texture | dispatch <= 30 ms + round-3 preview-bake time | dispatch max 7.1 / 5.6 / 5.4 / 5.6 ms (4 runs) | met |
| In-app: `workflow_path` "text stamp" on ABeautifulGame | within 10 % of round 3 | 496 / 512 ms vs R3 now 628 / 456 ms (median 504 vs 542) | met |
| In-app: opening the picker, "side panel" segment | no frame > 16 ms | runs 2-5: max 1.88 / 2.80 / 2.30 / 2.55 ms; **run 1: 179.0 ms (+131.9 ms next frame)**; headless 78 families max 1.52 ms | **missed once in 5** |
| Character page, 1,000 characters, 20 idle + 20 scrolling frames | 16 ms | whole headless panel pass max 1.12 / 1.23 ms | met |
| In-app: `text_path` Circle Final on ABeautifulGame | 1,500 ms | 805 / 841 / 784 / 818 / 716 ms | met |
| Font scan at start | <= round 3 + 0.05 s | 13.9-15.6 ms vs R3 now 13.4 / 14.1 ms | met |

### Round-3 limits ("not worse than" the round-3 gate, `round3\ROUND3_REPORT.md`), R4 / R3 back to back

| Step | Round-3 gate | R4 now | R3 now | Verdict |
|---|---|---|---|---|
| ngons_tiling image stamp | 614 / 585 ms | 632 / 583 | 616 / 584 | same |
| ngons_tiling transform the stamp | 1,666 / 1,682 ms | 1,686 / 1,778 | 1,615 / 1,765 | same (run spread) |
| ngons_tiling export every format (step; job) | 417 / 398 ms | 1,583 / 1,016 / 582 / 567 / 600 ms; job 0.40-1.41 s | 864 / 716 / 1,148 / 1,164 / 749 ms; job 0.22-0.99 s | same (disk; the 27 files are byte-identical to round 3's) |
| ABeautifulGame image stamp | 1,993 / 2,242 ms | 2,165 / 2,482 | 2,207 / 1,799 | same |
| ABeautifulGame transform | 1,392-2,419 ms | 1,405 / 1,614 | 1,654 / 1,470 | same |
| ABeautifulGame export every format (job) | 6.04-8.24 s (limit 12 s) | 10.05 / 9.60 s | 10.54 / 10.02 s | met; both exes slower today (disk); 85 files byte-identical |
| ABeautifulGame save / reopen | 3,525-4,993 / 4,783-10,101 ms | 4,837 / 5,542; 5,094 / 7,385 | 4,574 / 4,517; 6,440 / 4,627 | within the round-3 ranges |
| Fuzz seed 7, 1,000 steps, ngons: central/dispatch frames > 500 ms | 4-6 (worst 1,029-1,194 ms) | 5 / 8 (worst 1,011 / 2,149 ms) | 6 / 6 (worst 769 / 1,416 ms) | run 2 above the line: the 2,149-ms frame places the long **plain** text at 12 % (round-3 code); the fuzz sequence differs from round 3's after the first new text operation |
| Exported PSD / ORA with a styled text layer (Pillow) | pass | export_run PASS (gate 1, gate 2, final); 151 sweep export folders PASS (gate 2) | | met |

## 3. The 1.5-million-triangle model (ABeautifulGame.glb), in plain words

Painting at Size 50 runs at a **median of 17.6-20.2 ms per frame with Circular and 17.7-18.1 ms with Sphere** (50-57
frames per second), against **25.1-27.6 ms and 26.5-26.9 ms** for the same stroke with round 3's brush measured the
same day (27.4 / 27.0 ms in P0's baseline file). The longest drag frame was 186-213 ms (one 948-ms frame in the very
first stroke of a run, the warm-up). A stroke trails at most **5.8 ms** after the release. The frames over 100 ms (11-13
per Size-50 stroke, 100-213 ms) are all "pointer/tools/hover", that is stroke work: the pacing caps work at 100 ms per
frame plus one sample, and single samples there take 88-132 ms. At Size 512 every frame of the stroke is one dab of
0.5-8.3 s (37-41 frames per stroke, 79 dabs, 44-73 s per stroke); the brush is not interactive there, in round 3 as now
(the per-dab cost of the round-3 loop and of round 4 are equal in the core bench).

**Did the brush get faster there, or only stay responsive?** It stayed as fast per dab (P3 0.94 x) and the window is
more responsive (the median frame is 30 % shorter, because the work is spread over frames); the stroke work itself is
not smaller: per walk step the new walk costs 1.8-1.9 x round 3's (P7 missed), and the largest backlog during the drag
is 0.8-1.6 s (P8d missed), so on this model a fast stroke lags behind the pointer by up to 1.6 s and catches up before
the release (release to end <= 5.8 ms in these runs).

## 4. Re-recorded hashes

None. No scenario or app test asserts an exact hash after a drag (B7's search, `notes\pkg_b7.md`; confirmed at M2); every
scenario compares hashes it records in the same run.

## 5. Cross-package edits and contract requests

Cross-package edits (all merged, `notes\gate1.md` section 1): S1 `project\save.rs`, `panels\layers.rs`,
`tool_options.rs`, `control_sweep\panel_tests.rs`, `tools\stamp_place.rs` (the new `fx` line in every `TextSpec`
literal); S2 `dialogs\mod.rs`, `control_sweep\gpu_tests.rs`, `selftest\scenarios.rs`, `prefs.rs`, `gpu\mod.rs`,
`gpu\renderer.rs` (contracts and stubs); B4 `state.rs` (Brush tab state); B5 `state.rs` (dialog state); T6
`panels\mod.rs` (one line), `control_sweep\panel_tests.rs` (two tests); T7 `core\doc_stamps.rs` (`raster_counter`
public, hover-picture reuse in `add_stamp`, `text_picture_is_full`), `core\text\font_catalog.rs` (`impl Clone`); M1
`tip_field.rs` (B1-1, B3-1), `text\warp.rs` (T1-2), `brush.rs` (test); gate 1 `tools\brush.rs` (inner ring for the ball
only); gate 3 `app.rs` (`WTE_FRAME_LOG`), `state.rs` + `panels\font_picker.rs` (`PanelHit::FontButton`).

Contract requests (`notes\contract_requests.md`, decisions in `notes\m1.md` section 2): B1-1 step-9 local thickness on
baked shapes — applied, F20 on bakes stays about one cell above 1.5 (2.27 deep / 3.70 near the skin); B1-2 F8 wording —
accepted ("within 3 %", measured +2.48 %); B1-3 `open_box` with ClosedPartsOnly — accepted (offered only when the
file has closed parts); B1-4 steps 7 / 8 differ from the design — accepted at gate 1 (meets F2 / F9); T1-1 circle
centring and coverage wording — accepted; T1-2 `WarpMap` fields — applied; T2-1 `both_painters_cover_alike` against the
classic painter — accepted as a known limit (96-97 % against the bucketed classic painter, 99 % against a fine
reference); B3-1 `bound()` of a part cut at its grid border — applied.

## 6. Sweep against round 3 (gate 2, exe 144E3C12; `notes\gate2.md`)

| Group | Models | Round 3 | Round 4 |
|---|---|---|---|
| `round2\jobs_wf.txt`, `workflow_path` | 157 | 149 ok / 8 fail | **149 ok / 8 fail**, the same models and failure texts (6 import errors, slivers.glb / .obj) |
| mesh_3m.glb / .obj alone | 2 | ok 26.9 / 43.6 s | ok 27.9 / 44.5 s |
| final exe, every 10th model of `jobs_wf.txt` | 16 | — | 15 ok, zero_bytes.obj its intended import error (`runs_gate3\final\wf_subset.out`) |

Models slower or changed: none reproducibly more than 10 % slower in a brush or text step; the "text stamp" step is one
frame (+17 ms) longer because the Text tool now draws its hover picture (R4-17, by design). Saved projects: `uv_hash`
identical for all 149. Screenshots: the 3D half identical for every model, the 2D half differs only in the toast area.

## 7. Screenshots at 1280 x 800 (final exe, `runs_gate3\final\runs\`)

| What | File |
|---|---|
| The Brush tab with a loaded shape and its cursor | `brush_tab_builtin\shot.png` (Sphere with a loaded shape, Turn 30 / Tilt -20, Openings "Flat caps", the shape cursor on the sphere) |
| The closing dialog of an open model | `brush_shape_builtin\shot_dialog.png` ("open_box.obj has 1 opening; it is 82 % of the model's size", Flat caps / Rounded caps / Fill all hollows, Close with flat caps / Cancel) |
| The Text tab: Character, Warp, Path | `text_tab_builtin\shot_page_character.png`, `shot_page_warp.png`, `shot_page_path.png`; with a selected gap: `text_tab_builtin\shot.png` |
| The font picker | `text_fonts_builtin\shot_picker.png` (In this project: Ubuntu, Arial; All fonts (78 of 78) with samples in their own faces) |
| A warped text on the model | `text_warp_builtin\shot_warp3d.png` (Arc 50 %) |
| A text on a circle on the model | `text_path_builtin\shot_circle3d.png` |
| A text with one widened gap on the model | `text_gaps_builtin\shot_gap3d.png` (0.5 em after "E") |
| The see-through text before placing | `text_tab_builtin\shot_hover.png` |

All were read. Contact sheets of every final run: `runs_gate3\final\sheet_1..7.png` (read).

## 8. What the user should try by hand

- Whether the 15 warp shapes look right (Warp page pictures and the result on a model).
- Whether the trail of a fast stroke feels acceptable on a heavy model (ABeautifulGame-sized): it can lag up to 1.6 s
  behind the pointer and catches up by the release.
- Whether the font picker shows the fonts you use (this machine: 78 families from `C:\Windows\Fonts` and the user font
  folder; registry-only fonts and the named styles of variable fonts are not listed, plan section 7 item 6).
- Loading your own model as a brush shape, and the closing dialog with a real open model.
- Ctrl+wheel (size) and Shift+wheel (turn) with the Brush and the Eraser.

## 9. Open defects, cuts

Open (none blocks use; all have numbers in part 2):
1. P7 / P8d on the 1.5-M-triangle model (part 3). No contained fix in the designs; a later round could make one walk
   step cheaper or let the input rate follow the paint rate.
2. Classic painter B1 2.13-2.32 ms against the 2.041-ms line (1.03-1.05 x of round 3 back to back; the code path is
   bit-identical to round 3; the mip build dominates).
3. Warp Finals on ABeautifulGame: 3 of the 20 3D Finals over 1,500 ms (the Final bake runs on the UI thread, as round 3's image stamp
   did, which missed the same line at 1,993 / 2,242 ms).
4. The font picker's first open took a 179-ms side-panel frame once in 5 runs; cause not found.
5. A text stamp grown to the size of the whole model bakes slowly: on ngons_tiling a 15.29-em Circle on a line as wide
   as the model diagonal takes 3.4-3.5 s to bake (the frame of gate 2: 4.9 s), against 2.0 s for the same text without
   round-4 settings and 1.0 s for an image stamp with the same projector (`runs_gate3\ngons_circle_probe.txt`). The
   Preview stays at 4.1-4.5 ms.
6. P1 (c) 2048 against P0's file: 1.075 x (1.01 x of round 3 back to back); P13 on the texture models 0.05 / 0.15 ms
   above the 16.7-ms line in 2 of 8 medians.
7. F20 on baked shapes about one cell above 1.5 cells (contract request B1-1); A15 (GPU test of the ring filter) and
   the other 11 GPU tests are `#[ignore]` without a GPU in `cargo test`.
8. From earlier stages, unchanged: "Layer is locked" vs "The active layer is locked" wording; "locked or hidden" does
   not say which (round 3).

Cut: R4-3 "Stay on connected surface" and package B2, the flood code in B3, the TwoMaterial part of `brush_types` —
by the user's answer to question 3 (plan section 7 item 1). Nothing else was cut (plan 5.3 was not needed).

## 10. Questions of plan section 4 still unanswered

None. Questions 2-9 were answered on 2026-11-17 (plan section 7); question 1 ("segmented") was decided before.

## 11. Deliberate changes of existing behaviour

- With the Brush and the Eraser, Ctrl+wheel changes the Size and Shift+wheel turns a shape; they no longer zoom
  (BRUSH 1.5, question 9).
- A spacing change keeps the alignment side of a placed text fixed (risk 11; plain texts keep the round-3 placement).
- "Same side only" is replaced by Type (Circular = round 3's brush with "Same side only" on).
- A fast stroke may finish after the release (R4-8); only Undo and Esc throw it away.
- The lower side panel starts with the tab strip Layers | Brush | Text instead of the "Layers" header, which no longer
  collapses (S2).
- The Text tool always edits the draft; a selected text is edited with the Transform tool (S2 / T6, TEXT 6.2; R3-10's
  "selected text in the Text options" was removed).
- Choosing the Text tool costs one frame more (+17 ms) for the see-through picture (R4-17).

## The user's answers (plan section 7) and how each was built

1. Q3 "circular the way it looks is fine, the sphere is a way to have different paint brushes via a 3D object" ->
   Circular is round 3's brush bit for bit (C1 identical hashes, `BrushTip::legacy`); Sphere is the 3D-object brush
   with the ball as default; no connected-surface checkbox (R4-3 / B2 / flood code not built).
2. Q2 tabs: "agree" -> the Layers | Brush | Text strip (R4-1).
3. Q4 "whatever is inside the 3D brush object is painted" -> the shape fits inside the ring; everything inside the
   volume is painted (Solid; Through only for flat sheets).
4. Q6 "flat editing like in photoshop" -> the Path page: a line in the flat picture of the text, projected as before;
   text around the model recorded as future work (TEXT 9.2, ARCHITECTURE 1.4).
5. Q5 "the app should ask to close openings, and how" -> `CloseMode` (Flat caps, Rounded caps, Fill all hollows,
   ClosedPartsOnly, Refuse) in `core\tip_bake.rs`; `HasOpenings { count, largest, closed_parts }`; the dialog
   `Dialog::CloseOpenings` (file, path, count, size of the largest; "Use only the closed parts" only when there are
   closed parts; Cancel keeps the current shape); the choice stored per path in `prefs.json` (at most 32) and changeable
   in the Brush tab ("Openings: Flat caps"); scenario `brush_shape` closes `open_box.obj` through the dialog.
6. Q8 "the font list should just take from what the computer has in its fonts folder" -> the Windows font folders
   (system and user), no registry fonts, no named styles of variable fonts.
7. Q9 "looks good" -> Ctrl+wheel size, Shift+wheel turn with Brush / Eraser.
8. Q7 "if its already there then its already there" -> the Angle field turns the whole text; a text stays its own stamp.

## Tests

Final source, `cargo test --workspace --offline` (`runs_gate3\final_test_workspace.log`): wte_app 318 passed / 16 ignored; wte_core lib 677 passed / 16 ignored; `tests\brush_tips.rs` 15; `tests\end_to_end.rs` 9; `tests\project_roundtrip.rs` 21 — **1,040 passed, 0 failed, 32 ignored** (round 3: 725 passed, 17 ignored). The 32 ignored are the 12 GPU tests of `control_sweep` (A15 among them), the GPU tests of `gpu` / `view`, and timing or machine-file probes, all run by hand in stage 3 where they are timing probes (part 2). Clippy `--workspace --all-targets`: 0 warnings (`final_clippy3.log`). No `todo!` / `unimplemented!` in `crates`. Scenarios: 37 names (`selftest::scenarios::tests` checks the count), all pass at 1,280 x 800, and the six size variants pass at 900 x 600 and 640 x 480 (`runs_gate3\final\final_1280.csv`, `s900.csv`, `s640.csv`).

## How to repeat each run

Build (PowerShell, in `work\merged`): `cmd /c "E:\claude_programs\wrapped_texture_editor\cargo.cmd test --workspace --offline"`,
`... clippy --workspace --all-targets --offline`, `... build --release -p wte_app --offline`.

- Core probes: `powershell -ExecutionPolicy Bypass -File review\gate3\core_benches.ps1 -Exe <work\merged\target\release\deps\wte_core-*.exe> -Out <file>`
  (stroke_bench_tips, text::styled::bench, gate_classic_painter, text_document_bench, font_timings, tip_bake load_times);
  twenty edits: run T4's `round3_twenty_edits_bench` from `runs_t4\r3probe` and then
  `WTE_R3_PREVIEW_MS=<its median> <wte_core test exe> text_document_twenty_edits_bench --ignored --nocapture`;
  fuzz-circle probes: `gate_fuzz_circle_text`, `gate_fuzz_circle_text_on_ngons`; panel timings:
  `<wrapped_texture_editor test exe> gate_character_page --ignored --nocapture` and `gate_font_picker`.
- App runs: `powershell -ExecutionPolicy Bypass -File review\gate3\perf_driver.ps1 -Phase brush|text|text2|r3` (one
  process at a time, `WTE_FRAME_LOG=1`), then `python -I review\gate3\btp.py <brush_types_path logs>`,
  `python -I review\gate3\frames.py <log> "<start marker>" "<end marker>" [segments]` (markers: `gate: twenty preview
  edits start|end`, `Setup(gate: font picker open)` / `Setup(gate: font picker closed)`, `Dyn(gate: hover circular
  start)` ...), `python -I perf\p0_tools\analyze.py <P0 log>`, `python -I review\gate3\wf_steps.py runs_gate3`,
  `python -I ..\round3_audit\slow.py <fuzz log> 500`, `python -I ..\round3_audit\quarters.py <fuzz log>`.
- Final scenarios: `..\round3_audit\sweep.ps1 -Jobs runs_gate3\jobs_final.txt -Exe wte_round4.exe -Runs <dir> -Parallel 2`,
  `runs_gate1\sweep_size.ps1 -Jobs runs_gate1\jobs_small.txt -Size 900x600` (and 640x480); repros
  `sweep.ps1 -Jobs runs_gate2\jobs_fuzz.txt -Parallel 3`; sweep sample `runs_gate3\jobs_wf_subset.txt`; full sweep
  `round2\jobs_wf.txt` (gate 2); exports `python -I ..\round3\check_exports.py <shot_export>`.
