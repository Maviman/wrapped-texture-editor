"""Identity check of STICKER_PLAN 8.5: compares a sweep of tools/sticker/jobs_identity.txt against the P0 baseline.

Usage: python -E -P tools/sticker/compare_baseline.py BASE NEW [--report FILE]
  BASE, NEW: sweep folders written by tools/objtabs/sweep.sh (one folder per job: shot.png, shot*.png, export folders,
  saved .wtep files, shot.json / shot.log / stdout.txt).

Rules (one line per file that is not identical, then a summary; exit code 1 when anything FAILs):
  1. Exports (every file inside a sub-folder of a job, and every file that is not a screenshot, a .wtep or a log):
     byte for byte.
  2. .wtep: every ZIP entry except manifest.json byte for byte (layer PNGs, blobs, source files, flattened and thumbnail
     pictures); manifest.json equal after removing "format_version", "min_reader_version", "app_version" and
     "saved_unix" (the save time; it differs between any two runs). A difference in "stamps" or "sets" (layers) FAILs;
     any other manifest difference is listed as NOTE.
  3. Screenshots (*.png directly in a job folder): 1280x800 pictures are compared inside the central view rectangle
     x 44..980, y 51..775 (right of the tool strip, below the menu bar and the mode strip, left of the side panel, above
     the status bar), pixel for pixel. Excluded on purpose (the sticker work changes them): the mode strip (a new button),
     the side panel (a fifth upper tab, possibly a second tab row), the menu bar and the status bar. Pictures of another
     size are compared whole. Areas that differ between two runs of the same exe (measured in P0 by running the
     baseline twice) are masked in both pictures first: see VOLATILE.
  4. .json, .log, .txt are not compared (they hold times). The scenario's "ok" is checked: a job that was ok in BASE
     and is not ok in NEW FAILs.
"""
import json
import os
import sys
import zipfile

import numpy as np
from PIL import Image

VIEW_BOX = (44, 51, 980, 775)
SKIP_EXT = {'.json', '.log', '.txt'}
MANIFEST_IGNORED = ('format_version', 'min_reader_version', 'app_version', 'saved_unix')
# Screenshot areas (x0, y0, x1, y1 in window pixels at 1280x800) that change between two runs of the same exe:
# the companion dialog's random test port, random pairing code and "since <today>" date; the export dialog's folder
# field, which shows the sweep's own output path.
VOLATILE = {
    'companion/shot_companion_dialog.png': [(500, 428, 625, 446), (545, 474, 670, 502), (520, 528, 640, 547)],
    'export/shot.png': [(320, 148, 505, 170)],
}

results = []  # (status, rel, detail)


def add(status, rel, detail=''):
    results.append((status, rel, detail))


def job_ok(job_dir):
    p = os.path.join(job_dir, 'shot.json')
    try:
        return json.load(open(p, encoding='utf-8')).get('ok') is True
    except (OSError, ValueError):
        return None


def compare_png_view(rel, pa, pb):
    a, b = Image.open(pa).convert('RGBA'), Image.open(pb).convert('RGBA')
    if a.size != b.size:
        add('FAIL', rel, f'screenshot size {a.size} vs {b.size}')
        return
    for box in VOLATILE.get(rel, []) if a.size == (1280, 800) else []:
        a.paste((0, 0, 0, 255), box)
        b.paste((0, 0, 0, 255), box)
    if a.size == (1280, 800):
        x = np.asarray(a.crop(VIEW_BOX)).astype(np.int16)
        y = np.asarray(b.crop(VIEW_BOX)).astype(np.int16)
        where = 'view rect'
    else:
        x, y = np.asarray(a).astype(np.int16), np.asarray(b).astype(np.int16)
        where = 'whole picture'
    d = np.any(x != y, axis=2)
    if not d.any():
        add('same', rel, where + (' (volatile areas masked)' if rel in VOLATILE else ''))
        return
    ys, xs = np.nonzero(d)
    ox, oy = (VIEW_BOX[0], VIEW_BOX[1]) if where == 'view rect' else (0, 0)
    box = (int(xs.min()) + ox, int(ys.min()) + oy, int(xs.max()) + 1 + ox, int(ys.max()) + 1 + oy)
    mad = float(np.abs(x - y).max())
    add('FAIL', rel, f'{where}: {int(d.sum())} px differ in box {box}, max channel difference {mad:.0f}')


def manifest_core(m):
    return {k: v for k, v in m.items() if k not in MANIFEST_IGNORED}


def compare_wtep(rel, pa, pb):
    za, zb = zipfile.ZipFile(pa), zipfile.ZipFile(pb)
    na, nb = set(za.namelist()), set(zb.namelist())
    bad = []
    if na != nb:
        bad.append(f'entries only in base {sorted(na - nb)}, only in new {sorted(nb - na)}')
    notes = []
    for n in sorted(na & nb):
        if n == 'manifest.json':
            ma = json.loads(za.read(n))
            mb = json.loads(zb.read(n))
            ca, cb = manifest_core(ma), manifest_core(mb)
            for k in sorted(set(ca) | set(cb)):
                if ca.get(k) != cb.get(k):
                    (bad if k in ('stamps', 'sets') else notes).append(f'manifest "{k}" differs')
            continue
        if za.read(n) != zb.read(n):
            bad.append(f'entry {n} differs')
    if bad:
        add('FAIL', rel, '; '.join(bad + notes))
    elif notes:
        add('NOTE', rel, '; '.join(notes))
    else:
        add('same', rel, 'wtep entries and manifest (save time, versions ignored)')


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    report = None
    if '--report' in sys.argv:
        report = sys.argv[sys.argv.index('--report') + 1]
        args = [a for a in args if a != report]
    base, new = args[0], args[1]
    jobs = sorted(d for d in os.listdir(base) if os.path.isdir(os.path.join(base, d)))
    for job in jobs:
        jb, jn = os.path.join(base, job), os.path.join(new, job)
        if not os.path.isdir(jn):
            add('FAIL', job, 'job missing in NEW')
            continue
        okb, okn = job_ok(jb), job_ok(jn)
        if okb and not okn:
            add('FAIL', job + '/shot.json', f'"ok" was true, now {okn}')
        for root, _, files in os.walk(jb):
            for f in sorted(files):
                pa = os.path.join(root, f)
                rel = os.path.relpath(pa, base).replace(os.sep, '/')
                ext = os.path.splitext(f)[1].lower()
                if ext in SKIP_EXT:
                    continue
                pb = os.path.join(new, os.path.relpath(pa, base))
                if not os.path.exists(pb):
                    add('FAIL', rel, 'missing in NEW')
                    continue
                if ext == '.wtep':
                    compare_wtep(rel, pa, pb)
                elif ext == '.png' and root == jb:
                    compare_png_view(rel, pa, pb)
                else:
                    same = open(pa, 'rb').read() == open(pb, 'rb').read()
                    add('same' if same else 'FAIL', rel, 'bytes' if same else 'export bytes differ')
        for root, _, files in os.walk(jn):
            for f in files:
                pb = os.path.join(root, f)
                if os.path.splitext(f)[1].lower() in SKIP_EXT:
                    continue
                if not os.path.exists(os.path.join(base, os.path.relpath(pb, new))):
                    add('NOTE', os.path.relpath(pb, new).replace(os.sep, '/'), 'only in NEW')
    out = []
    for status, rel, detail in results:
        if status != 'same':
            out.append(f'{status:4s} {rel}  {detail}')
    counts = {}
    for status, _, _ in results:
        counts[status] = counts.get(status, 0) + 1
    out.append(f'jobs {len(jobs)}; files: ' + ', '.join(f'{k} {v}' for k, v in sorted(counts.items())))
    out.append('RESULT: ' + ('FAIL' if counts.get('FAIL') else 'PASS'))
    text = '\n'.join(out)
    print(text)
    if report:
        with open(report, 'w', encoding='utf-8', newline='\n') as fh:
            fh.write(text + '\n')
    return 1 if counts.get('FAIL') else 0


if __name__ == '__main__':
    sys.exit(main())
