"""Branch pen-input: compare two scenario sweeps (tools/objtabs/sweep.sh outputs) file by file.

Usage: python -E -P tools/pen/compare_runs.py BASE_DIR NEW_DIR

- Every non-image file (exports: PNG layers are compared as bytes too, PSD, ORA, ZIP, OBJ, ...) under each scenario
  folder: byte-identical or not.
- Every PNG screenshot (shot*.png): identical, or the bounding box of the changed pixels, split into the central view
  area (x < side panel) and the rest, so a changed side panel (the Pen pressure rows) is told apart from changed
  painting. The side panel starts at the first column of the 1280-wide shot whose pixels match the panel colour; we use
  a fixed split at x = 980 for 1280-wide shots (the side panel's left edge at default width) and the status bar
  (bottom 26 rows) is reported separately.
"""
import hashlib
import os
import sys

import numpy as np
from PIL import Image

SHOT_PREFIX = 'shot'
SIDE_X = 980
STATUS_ROWS = 26


def files(root):
    out = {}
    for d, _, names in os.walk(root):
        for n in names:
            p = os.path.join(d, n)
            out[os.path.relpath(p, root)] = p
    return out


def digest(p):
    return hashlib.sha256(open(p, 'rb').read()).hexdigest()


def bbox(mask):
    ys, xs = np.nonzero(mask)
    if len(xs) == 0:
        return None
    return (int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1, int(mask.sum()))


def main(base, new):
    a, b = files(base), files(new)
    report = []
    same_files = diff_files = shots_same = 0
    for rel in sorted(set(a) | set(b)):
        if rel.endswith(('.log', '.json', 'stdout.txt', 'summary.txt')):
            continue
        if rel not in a or rel not in b:
            report.append(f'ONLY IN {"BASE" if rel in a else "NEW"}: {rel}')
            continue
        name = os.path.basename(rel)
        is_shot = name.startswith(SHOT_PREFIX) and name.endswith('.png') and os.path.dirname(rel).count(os.sep) == 0
        if not is_shot:
            if digest(a[rel]) == digest(b[rel]):
                same_files += 1
            else:
                diff_files += 1
                report.append(f'BYTES DIFFER: {rel}')
            continue
        ia = np.asarray(Image.open(a[rel]).convert('RGBA'))
        ib = np.asarray(Image.open(b[rel]).convert('RGBA'))
        if ia.shape != ib.shape:
            report.append(f'SHOT SIZE DIFFERS: {rel} {ia.shape} vs {ib.shape}')
            continue
        mask = np.any(ia != ib, axis=2)
        if not mask.any():
            shots_same += 1
            continue
        h, w = mask.shape
        side_x = SIDE_X if w == 1280 else int(w * SIDE_X / 1280)
        central = mask.copy()
        central[:, side_x:] = False
        central[h - STATUS_ROWS:, :] = False
        side = mask.copy()
        side[:, :side_x] = False
        status = np.zeros_like(mask)
        status[h - STATUS_ROWS:, :side_x] = mask[h - STATUS_ROWS:, :side_x]
        report.append(f'SHOT DIFFERS: {rel} central={bbox(central)} side={bbox(side)} status={bbox(status)}')
    print(f'{same_files} files byte-identical, {diff_files} differ; {shots_same} screenshots pixel-identical')
    for line in report:
        print(line)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
