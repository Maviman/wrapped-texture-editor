"""Gate 3: workflow_path step times, round 4 against round 3, per model (the round-3 gate's workflow_steps.csv form).

python -I wf_steps.py <runs_gate3 dir>
Reads wf_r4_<n>\workflow_path_*\shot.log and wf_r3_<n>\...; prints one row per model and step with every run's ms and
the "job Exporting" seconds. Also the fuzz seed-7 frame summary of fuzz_r4_<n> / fuzz_r3_<n> (central/dispatch frames
over 500 ms, worst frame) from the frame profile lines.
"""
import glob
import os
import re
import sys

STEP = re.compile(r'workflow step "([^"]+)" done in (\d+) ms')
JOB = re.compile(r'job "Exporting[^"]*" (?:done|finished)[^\n]*?in ([\d.]+) s')

root = sys.argv[1]
runs = {}
for kind in ("r4", "r3"):
    for d in sorted(glob.glob(os.path.join(root, f"wf_{kind}_*"))):
        n = d.rsplit("_", 1)[1]
        for m in sorted(glob.glob(os.path.join(d, "workflow_path_*"))):
            model = os.path.basename(m)[len("workflow_path_"):]
            log = os.path.join(m, "shot.log")
            steps = {}
            for line in open(log, encoding="utf-8", errors="replace"):
                s = STEP.search(line)
                if s:
                    steps[s.group(1)] = int(s.group(2))
                if "Exporting" in line and " in " in line and "job" in line:
                    j = re.search(r"in ([\d.]+) s", line)
                    if j:
                        steps["job Exporting (s)"] = float(j.group(1))
            runs[(model, kind, n)] = steps
models = sorted({k[0] for k in runs})
for model in models:
    print(f"== {model}")
    keys = []
    for k, v in runs.items():
        if k[0] == model:
            keys += [s for s in v if s not in keys]
    for step in keys:
        r4 = [runs[k].get(step) for k in sorted(runs) if k[0] == model and k[1] == "r4"]
        r3 = [runs[k].get(step) for k in sorted(runs) if k[0] == model and k[1] == "r3"]
        print(f"  {step:28s} R4 {r4}  R3 {r3}")

FP = re.compile(r"frame profile: .*")
SEG = re.compile(r"(central|dispatch) ([\d.]+)/([\d.]+)")
for kind in ("r4", "r3"):
    for d in sorted(glob.glob(os.path.join(root, f"fuzz_{kind}_*", "fuzz_path_*"))):
        log = os.path.join(d, "shot.log")
        if not os.path.exists(log):
            continue
        worst, over = 0.0, 0
        for line in open(log, encoding="utf-8", errors="replace"):
            if "frame profile:" in line:
                for name, avg, mx in SEG.findall(line):
                    worst = max(worst, float(mx))
        print(f"fuzz {kind} {d}: worst central/dispatch window max {worst:.0f} ms")
