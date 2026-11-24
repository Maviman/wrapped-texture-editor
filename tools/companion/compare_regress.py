"""Compares two regression output folders of tools/companion/regress.sh (companion off = app unchanged).

Usage (repo root):  python -E -P tools/companion/compare_regress.py BASE NEW

For every PNG in BASE (final screenshots and named shots) it reports whether NEW has the same pixels, else the
number of differing pixels and their bounding box. Also compares the "ok" of every scenario JSON.
Exit code 0 when every picture is identical, 1 otherwise (differences can be expected: say which in the notes).
"""

import json
import os
import sys

import numpy as np
from PIL import Image


def load(path):
    with Image.open(path) as im:
        return np.asarray(im.convert("RGBA"))


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    base, new = sys.argv[1], sys.argv[2]
    same = True
    for name in sorted(os.listdir(base)):
        if name.endswith(".json"):
            b = json.load(open(os.path.join(base, name), encoding="utf-8")).get("ok")
            p = os.path.join(new, name)
            n = json.load(open(p, encoding="utf-8")).get("ok") if os.path.exists(p) else None
            print(f"{name:40s} ok {b} -> {n}")
            same &= b == n
        if not name.endswith(".png"):
            continue
        p = os.path.join(new, name)
        if not os.path.exists(p):
            print(f"{name:40s} MISSING in {new}")
            same = False
            continue
        a, b = load(os.path.join(base, name)), load(p)
        if a.shape != b.shape:
            print(f"{name:40s} size {a.shape[1]}x{a.shape[0]} -> {b.shape[1]}x{b.shape[0]}")
            same = False
            continue
        diff = np.any(a != b, axis=2)
        count = int(diff.sum())
        if count == 0:
            print(f"{name:40s} identical")
        else:
            ys, xs = np.nonzero(diff)
            print(f"{name:40s} {count} pixels differ in x {xs.min()}..{xs.max()}, y {ys.min()}..{ys.max()}")
            same = False
    return 0 if same else 1


if __name__ == "__main__":
    sys.exit(main())
