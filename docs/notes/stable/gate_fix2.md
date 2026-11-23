# Stable gate, fix pass 2: notes

Gate: Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable` at f889d69. The results
are in `docs/reports/STABLE_GATE_REPORT.md`, section "Fix pass 2". These notes record how I ran the gate and what to
watch for next time. Raw outputs (git-ignored) are in `target\stable\gate3\`.

## Exes

- `bin\wte_stable2.exe` is the release build of f889d69, SHA-256 21CA9303…4B98. The final rebuild after the docs edit
  gives the same hash.
- Each kind of run used its own copy of that exe, so no run touched a copy the user might have open:
  `wte_g3_scen.exe` (all scenarios), `wte_g3_rest.exe` (export_flow, fuzz and sweep) and `wte_g3_focus.exe` (the
  focus probe).
- I ran no exe older than f889d69. All comparisons are against stored outputs:
  - the stable gate's `target\stable\gate\runs_st`;
  - check_exp2's `texture_wrapper_ap_exp\target\stable\check_exp3\runs`;
  - stable-bg's `texture_wrapper_ap_bg\target\bgcheck\runs_bg`;
  - fix-wrap's `target\stable\fixwrap\sweep_fw7`;
  - check_wrap2's `target\stable\checkwrap2\flow`.

## Scripts (`target\stable\gate3\tools\`)

- `run1.sh`, `run_all.sh` and `jobs_all.txt`: all 45 scenario names. `export_unsettled` runs twice, once on the
  sphere and once on ABeautifulGame.
- `rest.sh`: export_flow, fuzz and the sweep, one process at a time.
- `shot_cmp3.py` (the gate's `shot_cmp.py` with model suffixes stripped) and `cmp_pairs.py` (compares any pair of run
  folders; PNG shots pixel for pixel, everything else byte for byte).
- `fuzz_sum.py`: action counts, errors and wrap lines.
- `focus_probe.ps1`, also committed as `tools/stable/focus_probe.ps1`.
- The export checks used the committed `tools/stable/verify_zip_vs_loose.py` and `tools/stable/check_exp2_verify.py`.
- Python: `python -E -P`. `-I` hides numpy and Pillow, which live in the user site.

## The focus check

`focus_probe.ps1 -Exe <copy> -Out <dir> -Scenario smoke` uses `Start-Process` to start the run, then samples
`GetForegroundWindow` → process id every 25 ms until the run exits. It sends no input and only reads. The result:
the foreground was the user's game (BlackOps3) before and after the run, never the app (0 of 50 samples), and it never
changed. Use it again in any later gate before running a batch, so a regression of stable-bg shows up first.

## Things that happened

- **A stale driver from the stopped earlier attempt.** When I started, `target\stable\gate2\tools\rest.sh` was still
  running (bash pid 24145). It went on to run `gate2\bin\wte_g2_flow.exe` (SHA-256 1B0C7255…), a build from before
  f889d69, from 09:15 to 09:27. It ran export_flow on ABeautifulGame and nouv_cube, 4 fuzz runs and 4 sweeps. I tried
  to `kill` the bash driver, not the exe; the permission system refused, so it finished on its own. While it ran, the
  older exe could have taken the focus from the user's game. Next time: check `ps -ef` for a leftover driver before
  starting, and ask the user to stop it if needed.
- **The machine was busy.** The user was playing a game during the whole pass (CPU 65-76 % in a check at the end of
  the sweep). Any number that depends on time is not comparable with the idle gate:
  - the sweep's param times were 1.3-3.7 x the idle gate's;
  - the font sample job failed once at 649 ms against 500 ms. It passed 3 times alone, and again in the full GPU rerun.

  A background window could also be slowed by Windows' power throttling of processes with no window in front. I did
  not separate the two causes. A speed measurement needs an idle machine. It also needs one check whether `--screenshot`
  runs are slower now that the window is in the background; if they are, timing gates would have to account for it.
- The fuzz's undo counts (3-80) are lower than the gate's (99-100) because `save and reopen` (7-16 per run) starts a
  new history. That is expected since fix_export added it.
- The fuzz's two slow frames (542 and 399 ms between frames, ngons seed 11) both followed a `Resize` step of the
  scenario. The UI's CPU time was 2.8 and 124 ms.

## ARCHITECTURE.md

Not changed: this pass changed no API, app code or test.
