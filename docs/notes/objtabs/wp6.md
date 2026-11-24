# objects-tabs WP6 — scenarios, docs, gate (builder notes)

Plan: `docs/plans/OBJECTS_TABS_PLAN.md` sections 8-10. Gate report: `docs/reports/OBJECTS_TABS_GATE_REPORT.md`.

## What was added

- `crates/wte_app/src/selftest/scenarios/objects.rs`: the scenarios `objects` (plan 8.1) and `objects_path` (plan 8.2),
  registered in `NAMES` (now 50) and the dispatcher; `objects_path` gets the long timeouts (300 s / 3600 s) of the
  other path scenarios. `side_tabs` (plan 8.3) was already added by the WP4/5 builder.
- `tools/objtabs/`: `sweep.sh` (sequential gate sweep with a size column, `@SAMPLES@` / `@REPO@` placeholders, a
  missing model is skipped instead of popping a dialog), `jobs_gate.txt` (every scenario, the path scenarios on the
  Khronos samples, the small-window runs, the three new scenarios), `compare_views.py` (view-rect identity at 1280x800)
  and `compare.py` (file by file), copied from the WP4/5 work folders.
- ARCHITECTURE.md: new 1.9; 7.3 note (no version change); 8.11 count line, timeouts line and the `objects` /
  `objects_path` rows; 9.2 WP6 paragraph; section 10 "objects-tabs" package list. HANDOFF.md 4.1 / 5 / 6.
  `dist/HOW_TO_TEST.txt`: "Objects" and "Side panel tabs" checklists (including the Alt+H beep question).

## Deviations from the plan (and why)

1. **`objects_path` step 3 Ctrl-clicks the last object, not "a second" one.** The list is first scrolled to the last
   row (`object_reveal`), the row Ctrl-clicked, and after Isolate / Frame the list is scrolled back to the top
   (`object_reveal = Some(0)`), the scenario asserts the last row is out of view, and the Navigate click of step 4 is
   aimed at a grid point whose ray hits the last object. That makes step 4's "its row is revealed in the list" a real
   test (with the first two objects the row was in view anyway).
2. **`objects_path` step 5 makes two strokes.** The first is a warm-up (texel maps and caches of the touched set are
   built on first use); the second is timed. "The brush budget of the report" is taken as ROUND5_GATE_REPORT B4: no
   frame of stroke work over 140 ms (`BRUSH_FRAME_MS`). Each stroke must make exactly one undo entry.
3. **`objects` has one extra shot, `hidden_2d`** (the 2D view with the Cube hidden and UV wires on), for the plan 8.5
   item "2D view of the trio with the Cube hidden: texture whole, the Cube's UV wires absent".
4. **`objects` step 5 checks more than the plan**: besides the Floor / Cube regions it checks that the 7x7 texels around
   the Floor UV under the Cube's axis (a ray straight down through the hidden Cube) changed, i.e. the floor *under* the
   cube was painted.
5. **The reopen toast** is checked as "exactly one visible toast mentions hidden objects and it names the Cube"
   (the Save toast is also visible at that moment).
6. **Sweep job files**: the repo had no committed job files (`tools/round3_audit/sweep.ps1` takes a file per run and
   holds old-machine paths), so `tools/objtabs/jobs_gate.txt` with its own `sweep.sh` is the committed job list.
7. Notes stay in `docs/notes/objtabs/` (the plan says `docs/notes/objects_tabs/`; WP0-5 already used `objtabs`).
