# Round 9 gate — skeptic notes, lens: speed and robustness (7.3)

Skeptic (Opus), 2026-11-23 00:45-01:05 local. Worktree `G:\software_projects\texture_wrapper_ap_r9`, `round9-wip` @ a8beea6
(gate commit on top of 42996df). Nothing tracked was edited; nothing committed. All outputs in this folder
(`target\round9\verify_timing\`): `driver.log`, `load_samples.txt`, `sweep_*`, `wa15_*`, `stamp_*`, `fuzz_r9_W_1`,
parsed tables `sweep_finals.txt`, `wa15_table.txt`, `stamp_analysis.txt`, scripts in `tools\`.

## Conditions: idle machine (checked before every measurement)

- `BlackOps3.exe` (the game that ran during the gate) was **not running**. Steam was open, idle.
- Before each of the 19 runs `tools\idle.sh` checked `tasklist` for cargo / rustc / wrapped_texture_editor / wte_* /
  wa15_* / BlackOps3 and took a 2 s CPU sample: **no competing process in any check**; CPU 1.5-9.5 % (`driver.log`).
- Background sampler every ~5 s (`load_samples.txt`, 171 samples): game never present; the only busy processes were
  my own measured runs, one at a time.
- Same binaries as the gate: `gate\bin\wte_r9.exe` SHA-256 4F9E1422... (= `target\release\wrapped_texture_editor.exe`),
  `wte_r8.exe` DB61E168... (= the scratchpad reference `sweep\bin\wte_r8.exe`). WA15 probe: I ran
  `cargo test --release -p wte_core --lib --no-run` (the only cargo call; no-op "Finished", nothing rebuilt); the
  resulting `wte_core-e6751041a6709cfa.exe` has the same SHA-256 (22587BAF...) as `gate\bin\wa15_r9.exe`, so the gate's
  probe exe is the current source. `wa15_r8.exe` reused from the gate (built from `round8-wip` f9076124).
- Order as the plan says: r8 first in each pair, alternating; 2 runs per line; T-g once.
- Quality numbers in my runs equal the gate's (e.g. high_poly_sphere 77,721 verts; ABeautifulGame Jump off covered
  0.0714, same step-10 assert failure; r8 ABeautifulGame Jump on ink 1.3045 dup 10.5 %). Timing does not change results.

## Results vs the report

### T-a high_poly_sphere Final applied (limit <= 5 s and <= 0.5 x r8)

| | r9 run 1 / 2 (idle) | r8 run 1 / 2 (idle) | ratio | gate r9 (loaded) | gate r8 (loaded) |
|---|---|---|---|---|---|
| Jump off | 3,419 / 3,408 ms (param 3,036 / 3,037; graph wait 303 / 299) | 8,173 / 7,981 | 0.42 / 0.43 | 6,529 / 6,122 | 11,111 / 12,649 |
| Jump on | 3,110 / 3,149 ms (param 3,044 / 3,070) | 8,341 / 8,171 | 0.37 / 0.39 | 6,257 / 5,626 | 11,278 / 13,013 |

Phases (idle, off): relax 2,693-2,707 ms (89 % of param), front 132-143, collect 68, owner 32-33. Shares match the
report; absolute times are about half.
**Verdict on idle: MET** (absolute and ratio). The report's "not met" does not hold on an idle machine.

### T-b ABeautifulGame Final (limits 1.0 s off / 6 s on, and <= 0.5 x r8)

| | r9 run 1 / 2 (idle) | r8 run 1 / 2 (idle) | ratio | gate r9 | gate r8 |
|---|---|---|---|---|---|
| Jump off applied | 1,002 / 1,030 ms | 2,351 / 2,356 | 0.43 / 0.44 | 1,844 / 1,864 | 3,361 / 3,492 |
| of which graph wait | 506 / 524 ms (graph built in 570 / 587 ms) | - | | 764 / 796 | |
| param | 407 / 412 ms (relax 346 / 352 = 85 %) | 2,259 / 2,263 | | 956 / 940 | |
| Jump on applied | 4,832 / 4,855 ms (param 4,157 / 4,172) | 10,945 / 10,967 | 0.44 / 0.44 | 9,326 / 8,892 | 16,873 / 17,165 |

Jump on phases (idle): relax 3,041-3,051 ms (73 %), front 475-477, collect 218, owner 179-183.
**Jump on on idle: MET** (4.8 s <= 6 s; 0.44 <= 0.5). Report's "not met" refuted.
**Jump off on idle: missed by 2 ms and 30 ms (0.2 % / 3 %)**, and only because of the one-off graph build: the
scenario's warm-up starts 2 frames before the Final (log: "warm-up started (graph: true)" at frame ~731, Final at 733),
so the first Final waits 506-524 ms for the 104.9 MB graph. Without that wait the Final is ~480-510 ms. The plan
budgets the graph build separately (T-c: "graph build <= 1.0 s once"), so whether T-b off counts it is a question for
the user. The report attributes the graph wait correctly ("applied includes 0.76-0.80 s graph build inside the first
Final") — confirmed in the gate's logs (graph phase 764 / 796 ms).
Also running during the first Final (both models): the "shared texels" pass of the warm-up (913 ms on ABeautifulGame,
905-924 ms on the sphere, ends after the Final's param). On the idle sphere it had no measurable effect (param off 3,036
vs on 3,044 ms); under the gate's load it competed for cores. Not mentioned in the report.

### Load was not "fair to both" (the report's ratio claim)

Load factor gate / idle: r9 1.80-1.93 x (hps off 1.91 / 1.80, abg off 1.84 / 1.81, abg on 1.93 / 1.83),
r8 1.36-1.59 x (hps 1.36 / 1.58, abg off 1.43 / 1.48, abg on 1.54 / 1.57). Round 9's parallel solve lost more to the
game than round 8's, so the gate's ratios (0.43-0.59) were pessimistic for round 9; on idle all six T-a / T-b ratios
are 0.37-0.44 (gate: three of them above 0.5). The report's sentence "the ratios are comparable" / gate.md "the ratios
are fair to each other" does not hold. Heavy wraps are 2.3-2.7 x faster than round 8 on idle (report: "about twice").

Plan baselines for r8 "here" (10-11 s hps; 3.4 s abg off) are also too high: idle r8 is 8.0-8.3 s and 2.35 s.

### T-c / T-d WA15 probe (release)

| line | r9 idle run 1 / 2 | r8 idle | ratio | gate r9 | verdict idle |
|---|---|---|---|---|---|
| board 20 % off | 10.4 / 10.6 ms (owner 94 %, relax 0: flat, not relaxed) | 268.4 / 268.2 | 0.04 | 13.8 / 14.6 | met (<= 60) |
| piece 20 % off | 119.4 / 119.5 ms (relax 84 %, owner 11 %) | 910.4 / 910.5 | 0.13 | 144.0 / 141.6 | met (<= 150) |
| graph build | 549.4 / 549.6 ms | - | | 727.6 / 706.1 | met (<= 1.0 s) |
| board 20 % on | 671.1 / 671.2 ms; limit max(1.5 x 10.5, 150) = 150 -> 4.5 x | 2,131 / 2,128 | 0.31 | 836 / 855 (5.6 x) | NOT met |
| piece 20 % on | 466.7 / 477.1 ms; limit 1.5 x 119.5 = 179 -> 2.6-2.7 x | 1,582 / 1,583 | 0.30 | 514 / 564 | NOT met |

T-c met, T-d not met: both reproduce. Absolutes on idle are 17-32 % below the gate's (outside 15 %, explained by load).
T-d phase shares match (board on: relax 67 %, collect 18 %; piece on: relax 56 %, collect 23 %, owner 15 %).
Side note: the r9 probe's total run is longer than r8's (112 vs 90 s) because it also runs the "sphere path" for each
case to check equality; not a product cost.

### T-f sphere sample `stamp`, W Final vs P (limit <= 1.5 x P or <= 40 ms; and <= 0.5 x r8)

Finals image / move / scale / rotate / wheel, applied ms:
- r9 W idle: run 1 31, 33, 49, 51, 12; run 2 31, 32, 40, 48, 12.
- r9 P idle: 11, 12, 12, 12, 13 (both runs).
- r8 W idle: 32, 33, 47, 69, 13; 32, 31, 47, 68, 13.
- gate r9 W run 2: 41, 46, 53, 71, 12 (run 1: 102, 54, 68, 127, 15).

Absolute limit on idle: image (31) and move (32-33) and wheel (12) **meet** 40 ms; scale 49 / 40 (miss / at the limit);
rotate 51 / 48 miss. The report's "only the wheel Final meets it" is refuted for the absolute part. The ratio part
fails for every Final (r9 / r8 = 0.97, 1.0, 0.85-1.04, 0.71-0.74, 0.92), the wheel included, so the line is still
**not met**, and the report's statement that the wheel meets it is wrong under the plan's "and <= 0.5 x r8" rule even
with its own numbers (12 / 13 = 0.92). Owner grid dominates (owner 19.7-23.9 ms of 23-35 ms params): confirmed.
The move Final's worker is 2 ms (param reused) but it lands after 32-33 ms because it waits for the drag's in-flight
param job (~23 ms): correct behaviour, not a misattribution. Gate's numbers reproduce within 15 % only for the wheel.

### T-g fuzz seed 7, 1,000 steps, ngons_tiling, Wrap (once)

`fuzz_r9_W_1`: exit 0, ok true, `errors: []`, 10,976 frame-timing lines (json frames 10,974), **0 frames > 500 ms,
0 > 300 ms** (max 150.6 ms), 0 ERROR lines, no panic, 33 wrap Final lines, 171.6 s. Warnings are the same 43 toasts
as the gate's run (locked / hidden layers). **Met; confirmed.**

### Robustness

- high_poly_sphere step 6b ("the drag's wrapped Previews ran as jobs and landed"): passed in both idle runs (jobs
  started 9 / 8, applied 8 / 8). Consistent with the gate's reading that its run-1 failure was a load-dependent flake;
  the assert is still timing-dependent (it can fail when the machine is loaded) — I did not reproduce the failure.
- r9 peak working set in wrap_surface on ABeautifulGame: 2,011-2,014 MB vs r8 1,791 MB (+220 MB; graph 104.9 MB).
  Not a 7.3 line (T-h is `workflow_path`), information only.

## Summary of verdict changes on an idle machine

| line | report (loaded) | idle |
|---|---|---|
| T-a | not met | **met** (3.1-3.4 s, ratio 0.37-0.43) |
| T-b off | not met | missed by 2 / 30 ms, only through the one-off graph wait (506-524 ms); met without it |
| T-b on | not met | **met** (4.83-4.86 s, ratio 0.44) |
| T-c | met | met |
| T-d | not met | not met (4.5 x / 2.6-2.7 x the limit) |
| T-f | not met ("only wheel meets") | not met (ratio fails on all five; absolute met for image, move, wheel) |
| T-g | met | met |

Consequence for the report's open decision 5 ("Meeting T-a / T-b needs a faster solve that changes results"): on an
idle machine T-a and T-b-on already meet their limits with the current solve; only T-b off (by the graph wait), T-d,
T-e and T-f remain. The report's speed paragraph and "Speed lines met / not met" list should be re-stated with idle
numbers before the user decides.
