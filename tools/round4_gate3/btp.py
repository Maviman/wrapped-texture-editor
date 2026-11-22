"""Gate 3: brush_types_path logs with WTE_FRAME_LOG=1 -> P7, P8a-d, P12, P13 per stroke.

python -I btp.py <shot.log> [...]
Per stroke (after "brush_types_path: next stroke ..."): pointer step ms (P8c), the tool's log line (P7 stroke work,
P8a longest frame / longest sample, P8d largest backlog, P8b release to end), and the drag frames of the Pointer step:
the per-frame lines after the press frame up to but not including the release frame (P0's definition), their
median / max interval (P13) and every frame > 100 ms (P12).
Frame lines of a Pointer step: [step frame, hover, press, moves..., release, finishing...]; with 2 points per frame a
path of N points has ceil((N-1)/2) move frames.
"""
import math
import re
import statistics as st
import sys

FT = re.compile(r"frame timing: interval ([\d.]+) ms, ui cpu ([\d.]+) ms; segments ms: (.*)$")
PTR = re.compile(r"frame (\d+): step Pointer\(Main3D, (\d+) points")
DONE = re.compile(r"step done: Pointer\(Main3D, \d+ points, Primary\) \((\d+) ms\)")
NEXT = re.compile(r"brush_types_path: next stroke (\w+) / (\w+), Size (\d+)")
STROKE = re.compile(r"brush stroke: (\d+) samples, stroke work ([\d.]+) ms, longest frame ([\d.]+) ms, longest sample ([\d.]+) ms, largest backlog ([\d.]+) ms, release to end ([\d.]+) ms")


def segs(text):
    d = {}
    for part in text.split(", "):
        if not part.strip():
            continue
        name, _, v = part.rpartition(" ")
        try:
            d[name] = float(v)
        except ValueError:
            pass
    return d


def main(path):
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    label = "warm-up"
    rows = []
    cur = None
    for l in lines:
        m = NEXT.search(l)
        if m:
            label = f"{m.group(1)}/{m.group(2)} {m.group(3)}"
        m = PTR.search(l)
        if m:
            cur = {"label": label, "points": int(m.group(2)), "frames": []}
            continue
        if cur is not None:
            m = FT.search(l)
            if m:
                cur["frames"].append((float(m.group(1)), float(m.group(2)), segs(m.group(3))))
            m = STROKE.search(l)
            if m:
                cur["log"] = [float(x) for x in m.groups()]
            m = DONE.search(l)
            if m:
                cur["ms"] = int(m.group(1))
                rows.append(cur)
                cur = None
                label = "?"
    print(f"== {path}")
    for r in rows:
        moves = math.ceil((r["points"] - 1) / 2)
        f = r["frames"]
        # f[0] = the frame in which the step began (its line follows the step line), f[1] hover, f[2] press.
        drag = f[3:3 + moves]
        after = f[3 + moves:]
        iv = [x[0] for x in drag]
        hov = [x[2].get("pointer/tools/hover", 0.0) for x in drag]
        big = [(i, x[0], x[1], x[2]) for i, x in enumerate(f) if x[0] > 100 or x[1] > 100]
        lg = r.get("log", [float("nan")] * 6)
        print(f"  {r['label']:>18}: pointer step {r.get('ms')} ms; {len(f)} frames ({len(drag)} drag, {len(after)} release+after); "
              f"drag median {st.median(iv) if iv else float('nan'):.2f} ms max {max(iv or [0]):.1f}; hover-seg median {st.median(hov) if hov else 0:.2f}; "
              f"log: samples {lg[0]:.0f}, work {lg[1]:.1f} ms ({lg[1] / max(lg[0], 1):.3f} ms/sample), longest frame {lg[2]:.1f}, "
              f"longest sample {lg[3]:.1f}, backlog {lg[4]:.1f}, release-to-end {lg[5]:.1f}")
        for i, a, c, s in big:
            top = sorted(s.items(), key=lambda kv: -kv[1])[:4]
            print(f"      frame #{i}: interval {a:.1f} ms, cpu {c:.1f} ms; " + ", ".join(f"{k} {v:.1f}" for k, v in top))


for p in sys.argv[1:]:
    main(p)
