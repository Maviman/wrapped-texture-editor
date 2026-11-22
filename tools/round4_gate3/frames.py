"""Gate 3: per-frame lines (WTE_FRAME_LOG=1) between two marker lines of a self-test log.

python -I frames.py <shot.log> <start marker> <end marker> [segment ...]
Prints the number of frames, median / max of the interval, the UI CPU time and each named segment (a segment absent
in a frame counts 0), the sum "overlays+side panel", and every frame whose interval or CPU time is over 100 ms.
"""
import re
import statistics as st
import sys

FT = re.compile(r"frame timing: interval ([\d.]+) ms, ui cpu ([\d.]+) ms; segments ms: (.*)$")


def segs(text):
    d = {}
    for part in text.split(", "):
        name, _, v = part.rpartition(" ")
        try:
            d[name] = float(v)
        except ValueError:
            pass
    return d


def main():
    path, start, end = sys.argv[1:4]
    names = sys.argv[4:] or ["side panel", "overlays", "dispatch", "scene sync", "central", "pointer/tools/hover", "jobs"]
    inside, frames = False, []
    for l in open(path, encoding="utf-8", errors="replace"):
        if start in l:
            inside, frames = True, []
            continue
        if inside and end in l:
            break
        if inside:
            m = FT.search(l)
            if m:
                frames.append((float(m.group(1)), float(m.group(2)), segs(m.group(3))))
    if not frames:
        print(f"{path}: no frames between {start!r} and {end!r}")
        return
    def stat(v):
        return f"median {st.median(v):.2f} max {max(v):.2f}"
    print(f"{path} [{start} .. {end}]: {len(frames)} frames")
    print(f"  interval {stat([f[0] for f in frames])}; ui cpu {stat([f[1] for f in frames])}")
    for n in names:
        print(f"  {n}: {stat([f[2].get(n, 0.0) for f in frames])}")
    print(f"  overlays+side panel: {stat([f[2].get('overlays', 0.0) + f[2].get('side panel', 0.0) for f in frames])}")
    for i, (a, c, s) in enumerate(frames):
        if a > 100 or c > 100:
            top = sorted(s.items(), key=lambda kv: -kv[1])[:5]
            print(f"  frame #{i}: interval {a:.1f}, cpu {c:.1f}; " + ", ".join(f"{k} {v:.1f}" for k, v in top))


main()
