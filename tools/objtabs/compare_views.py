"""View-rect pixel identity (OBJECTS_TABS_PLAN 6.6): compares every PNG of two sweep folders inside the central view
area of a 1280x800 window (x 44..980, y 51..775: right of the toolbar, below the menu bar and the mode strip, left of
the 300 pt side panel, above the status bar). Run: python -E -P compare_views.py BASE NEW > out.txt
"""
import os
import sys

import numpy as np
from PIL import Image

base, new = sys.argv[1], sys.argv[2]
BOX = (44, 51, 980, 775)
same = diff = skipped = 0
for root, _, files in os.walk(base):
    for f in sorted(files):
        if not f.endswith('.png') or f.endswith('_diff.png'):
            continue
        p = os.path.join(root, f)
        rel = os.path.relpath(p, base)
        q = os.path.join(new, rel)
        if not os.path.exists(q):
            print(f'missing      {rel}')
            continue
        a, b = Image.open(p).convert('RGBA'), Image.open(q).convert('RGBA')
        if a.size != (1280, 800) or b.size != (1280, 800):
            skipped += 1
            print(f'skipped      {rel} (size {a.size} / {b.size})')
            continue
        x = np.asarray(a.crop(BOX)).astype(int)
        y = np.asarray(b.crop(BOX)).astype(int)
        d = np.any(x != y, axis=2)
        n = int(d.sum())
        if n == 0:
            same += 1
            continue
        diff += 1
        ys, xs = np.nonzero(d)
        print(f'differs      {rel}: {n} px in x {xs.min() + BOX[0]}..{xs.max() + BOX[0]}, y {ys.min() + BOX[1]}..{ys.max() + BOX[1]}')
print(f'view area identical: {same}, different: {diff}, skipped (other size): {skipped}')
