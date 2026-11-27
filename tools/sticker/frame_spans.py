"""Worst frames of the sticker, link and OLE spans in a scenario's frame log (STICKER_PLAN 8.4).

Usage: python -E -P tools/sticker/frame_spans.py OUT.log [OUT2.log ...] [--limit 50]

A scenario run with WTE_FRAME_LOG=1 writes one "frame timing:" line per frame with "segments ms: name value, ...".
Spans whose name starts with "sticker.", "link." or "ole." are the ones 8.4 limits. Spans nest ("sticker.live" and
"sticker.region" run inside "sticker.canvas"), so the limit is checked per span, not on a sum: the worst value of each
span and the frame of the worst one are printed, and the exit code is 1 when any span exceeds the limit.
"""
import re
import sys

PREFIXES = ("sticker.", "link.", "ole.")


def main(argv):
    limit = 50.0
    paths = []
    it = iter(argv)
    for a in it:
        if a == "--limit":
            limit = float(next(it))
        else:
            paths.append(a)
    over = False
    for path in paths:
        worst = {}
        frames = 0
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if "frame timing:" not in line or "segments ms:" not in line:
                    continue
                frames += 1
                segs = line.split("segments ms:", 1)[1]
                for name, value in re.findall(r"([A-Za-z0-9_./ -]+?) (\d+(?:\.\d+)?)(?:,|$)", segs.strip()):
                    name = name.strip()
                    if not name.startswith(PREFIXES):
                        continue
                    v = float(value)
                    if v > worst.get(name, (0.0, 0))[0]:
                        worst[name] = (v, frames)
        print(f"{path}: {frames} frames")
        for name, (v, frame) in sorted(worst.items()):
            flag = "  OVER" if v > limit else ""
            print(f"  {name:<20} worst {v:8.2f} ms (frame {frame}){flag}")
            over |= v > limit
    return 1 if over else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
