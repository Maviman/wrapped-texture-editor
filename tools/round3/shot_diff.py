"""Per run folder: mean absolute difference of the 3D half and the 2D half of shot.png between two sweeps.
python -I shot_diff.py <runs A> <runs B> [threshold]   (prints all, sorted, flags > threshold)"""
import os
import sys

import numpy as np
from PIL import Image

a, b = sys.argv[1], sys.argv[2]
thr = float(sys.argv[3]) if len(sys.argv) > 3 else 8.0
rows = []
for d in sorted(set(os.listdir(a)) & set(os.listdir(b))):
    pa, pb = os.path.join(a, d, "shot.png"), os.path.join(b, d, "shot.png")
    if not (os.path.exists(pa) and os.path.exists(pb)):
        continue
    ia = np.asarray(Image.open(pa).convert("RGB"), np.int16)
    ib = np.asarray(Image.open(pb).convert("RGB"), np.int16)
    if ia.shape != ib.shape:
        rows.append((999, 999, d))
        continue
    # 1280x800: 3D half x 44..510, 2D half x 515..978, rows 50..770.
    d3 = float(np.abs(ia[50:770, 44:510] - ib[50:770, 44:510]).mean())
    d2 = float(np.abs(ia[80:690, 515:978] - ib[80:690, 515:978]).mean())
    rows.append((d3, d2, d))
for d3, d2, d in sorted(rows, reverse=True):
    flag = "CHECK" if max(d3, d2) > thr else "     "
    print(f"{flag} 3D {d3:6.2f}  2D {d2:6.2f}  {d}")
