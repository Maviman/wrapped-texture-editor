"""Compares two sweep folders file by file (run with python -E -P compare.py BASE NEW [MENU_HEIGHT_PX]).

PNG: byte-identical, else pixel-identical, else the diff box and the number of differing pixels; a diff whose box lies
inside the top `MENU_HEIGHT_PX` rows (the menu bar) is classed "menu bar only". Other files: byte-identical or not
(.json / .log / stdout.txt / summary.txt are skipped; .wtep are reported separately, they hold the save time).
"""
import os
import sys

import numpy as np
from PIL import Image

base, new = sys.argv[1], sys.argv[2]
menu_h = int(sys.argv[3]) if len(sys.argv) > 3 else 24
SKIP = {'.json', '.log', '.txt'}

counts = {}
lines = []


def note(kind, rel, extra=''):
    counts[kind] = counts.get(kind, 0) + 1
    if kind not in ('identical', 'pixel-identical'):
        lines.append(f"{kind:22s} {rel} {extra}")


for root, _, files in os.walk(base):
    for f in sorted(files):
        p = os.path.join(root, f)
        rel = os.path.relpath(p, base)
        ext = os.path.splitext(f)[1].lower()
        if ext in SKIP:
            continue
        q = os.path.join(new, rel)
        if not os.path.exists(q):
            note('missing in new', rel)
            continue
        a, b = open(p, 'rb').read(), open(q, 'rb').read()
        if a == b:
            note('identical', rel)
            continue
        if ext == '.wtep':
            note('wtep differs', rel)
            continue
        if ext != '.png':
            note('EXPORT DIFFERS', rel)
            continue
        ia = np.asarray(Image.open(p).convert('RGBA'))
        ib = np.asarray(Image.open(q).convert('RGBA'))
        if ia.shape != ib.shape:
            note('SIZE DIFFERS', rel, f"{ia.shape} vs {ib.shape}")
            continue
        d = np.any(ia != ib, axis=2)
        if not d.any():
            note('pixel-identical', rel)
            continue
        ys, xs = np.nonzero(d)
        box = (int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1)
        kind = 'menu bar only' if box[3] <= menu_h else 'PIXELS DIFFER'
        note(kind, rel, f"{int(d.sum())} px in box {box}")

for root, _, files in os.walk(new):
    for f in files:
        rel = os.path.relpath(os.path.join(root, f), new)
        if os.path.splitext(f)[1].lower() not in SKIP and not os.path.exists(os.path.join(base, rel)):
            note('only in new', rel)

for k in sorted(counts):
    print(f"{k}: {counts[k]}")
print()
for line in sorted(lines):
    print(line)
