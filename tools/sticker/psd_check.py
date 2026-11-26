"""Independent check of PSD files with Pillow (STICKER_PLAN 13.3, P5b).

A second reader, next to WTE's own reader and the strict walker of `psd/strict.rs`, for the files the sticker
writer produces. Run with `python -E -P tools/sticker/psd_check.py PATH...` (Pillow is in the user site-packages;
`-I` would hide it). A PATH is a .psd file or a folder (searched recursively for .psd files).

For `X.psd` with an expectation `X.json` beside it (written by the ignored test
`psd::write_tests::k5_write_dump_for_psd_check`) it checks:
  - Pillow opens the file as an 8-bit RGB / RGBA PSD of the expected canvas size;
  - image resources 1005, 1044 and 1057 are present and 1057 says the merged picture is real;
  - the layers Pillow lists, in order, have the expected Pascal names (Windows-1252, `?` for other characters) and
    rectangles. Pillow 11 skips layers with more than 4 channels (a layer with a mask), so only the layers without a
    mask are compared, and their count must match;
  - the merged picture equals WTE's own composite (`X_merged.png`, straight RGBA) exactly: colours matted on white with
    the writer's formula when the file has a transparency channel, the alpha as is; RGB when it has 3 channels.
Pillow's layer pixels are not compared: Pillow maps a layer's channels in sorted band order (R, G, B, A) instead of
the file order (-1, 0, 1, 2), so its layer pictures of any Photoshop-style file are not meaningful.

For any other .psd it only opens the file and its merged picture and prints what Pillow sees (fixtures); a
16- or 32-bit file, which Pillow cannot read, is reported as SKIP.
Exit code 1 when a check fails.
"""

import json
import os
import sys

from PIL import Image

CP1252_HIGH = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0,
    0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E,
    0x0178,
]


def pascal_bytes(name):
    """The Pascal name the writer writes: Windows-1252, '?' for other characters, at most 255 bytes."""
    out = bytearray()
    for ch in name[:255]:
        u = ord(ch)
        if 0x20 <= u <= 0x7E or 0xA0 <= u <= 0xFF:
            out.append(u)
        elif u in CP1252_HIGH and u != 0:
            out.append(0x80 + CP1252_HIGH.index(u))
        else:
            out.append(ord('?'))
    return bytes(out)


def matte(px):
    r, g, b, a = px
    return tuple((c * a + 255 * (255 - a) + 127) // 255 for c in (r, g, b)) + (a,)


def check_expected(path, expect_path):
    errors = []
    with open(expect_path, encoding='utf-8') as f:
        expect = json.load(f)
    im = Image.open(path)
    w, h = expect['size']
    if im.size != (w, h):
        errors.append(f'size {im.size} != {(w, h)}')
    if im.mode not in ('RGB', 'RGBA'):
        errors.append(f'mode {im.mode}')
    ids = [r[0] for r in im.resources]
    for rid in (1005, 1044, 1057):
        if rid not in ids:
            errors.append(f'resource {rid} missing')
    v = [r[2] for r in im.resources if r[0] == 1057]
    if v and (len(v[0]) < 5 or v[0][4] != 1):
        errors.append('resource 1057 does not say the merged picture is real')

    listed = im.layers
    want = [l for l in expect['layers'] if not l.get('mask')]
    if len(listed) != len(want):
        errors.append(f'Pillow lists {len(listed)} layers, expected {len(want)} (layers without a mask)')
    for got, exp in zip(listed, want):
        name, _mode, bbox = got[0], got[1], got[2]
        if name.encode('latin-1') != pascal_bytes(exp['name']):
            errors.append(f'layer name {name!r} != {exp["name"]!r}')
        if list(bbox) != list(exp['rect']):
            errors.append(f'layer {exp["name"]!r}: rect {bbox} != {exp["rect"]}')

    merged = Image.open(os.path.join(os.path.dirname(path), expect['merged'])).convert('RGBA')
    im.load()
    got = im.convert('RGBA') if im.mode == 'RGB' else im
    want_px = list(merged.getdata())
    got_px = list(got.getdata())
    transparent = im.mode == 'RGBA'
    worst = 0
    for p, q in zip(got_px, want_px):
        q2 = matte(q) if transparent else q
        if not transparent and q[3] != 255:
            errors.append('a 3-channel file for a picture that is not opaque')
            break
        worst = max(worst, max(abs(a - b) for a, b in zip(p, q2)))
    if worst:
        errors.append(f'merged picture differs from WTE\'s composite by up to {worst} levels')
    summary = f'{im.size[0]}x{im.size[1]} {im.mode}, {len(listed)} of {len(expect["layers"])} layers listed'
    return errors, summary


def pillow_can_read(path):
    """Pillow 11 reads version-1 PSDs of 1 or 8 bits per channel only."""
    with open(path, 'rb') as f:
        head = f.read(26)
    return len(head) == 26 and head[4:6] == bytes([0, 1]) and head[22:24] in (bytes([0, 1]), bytes([0, 8]))


def check_open(path):
    if not pillow_can_read(path):
        return None, 'skipped: Pillow reads only 8-bit version-1 PSDs'
    im = Image.open(path)
    im.load()
    try:
        n = len(im.layers)
    except Exception as e:  # noqa: BLE001 - report what Pillow says
        n = f'layers unreadable ({e})'
    return [], f'{im.size[0]}x{im.size[1]} {im.mode}, {n} layers listed (no expectation: opened only)'


def files(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _dirs, names in os.walk(p):
                for n in sorted(names):
                    if n.lower().endswith('.psd'):
                        yield os.path.join(root, n)
        else:
            yield p


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    failed = 0
    skipped = 0
    count = 0
    for path in files(argv):
        count += 1
        expect = os.path.splitext(path)[0] + '.json'
        try:
            errors, summary = check_expected(path, expect) if os.path.exists(expect) else check_open(path)
        except Exception as e:  # noqa: BLE001 - a file Pillow cannot read is a result
            errors, summary = [f'{type(e).__name__}: {e}'], 'not read'
        if errors is None:
            skipped += 1
            print(f'SKIP {path}: {summary}')
        elif errors:
            failed += 1
            print(f'FAIL {path}: {summary}')
            for e in errors:
                print(f'     {e}')
        else:
            print(f'PASS {path}: {summary}')
    print(f'{count - failed - skipped} passed, {skipped} skipped, {failed} failed of {count}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
