"""Check the layered exports of Wrapped Texture Editor (round 3 gate, plan section 6, decision 3).

Usage: python -I check_exports.py <export folder> [<export folder> ...]

For every texture set in a folder (found by its .psd / .ora file) it checks:
  - the PSD opens with Pillow (size, layer count, layer names, layer boxes, merged image) and with a strict reader
    written here from the PSD spec (every length, padding byte, channel id, the unicode name block);
  - the ORA opens: a ZIP with `mimetype` first and stored, stack.xml, one PNG per layer, mergedimage.png, thumbnail;
  - layer count, names, order and bounding boxes of PSD and ORA match the per-layer PNG files
    (<stem>_L<NN>_<kind>_<name>.png, NN = index from the bottom, full canvas);
  - each layer's pixels in the PSD and in the ORA equal the per-layer PNG inside its bounding box, and the PNG is
    fully transparent outside it;
  - PSD and ORA agree on opacity and visibility;
  - the flattened image equals the flat file <stem>.png: ORA mergedimage.png byte for byte (RGBA), PSD merged image
    equal to it, matted on white when the flat image has any alpha below 255 (ARCHITECTURE.md 7.7).
Exit code 0 when every set passes, 1 otherwise. Prints one line per check group and a summary.
"""

import io
import os
import re
import sys
import struct
import zipfile
import unicodedata
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None

FORBIDDEN = set('<>:"/\\|?*')
LAYER_RE = r"_L(\d\d)_(paint|stamp|text)_(.*)\.png$"
FLAT_EXTS = ["png", "tga", "bmp", "tif", "tiff", "webp", "jpg", "jpeg"]


# ---------------------------------------------------------------- naming (export/naming.rs sanitize_file_stem)

def _is_reserved(stem):
    base = stem.split(".")[0].rstrip(" ").upper()
    if base in ("CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"):
        return True
    if len(base) == 4 and base[:3] in ("COM", "LPT") and base[3] in "123456789¹²³":
        return True
    return False


def sanitize_file_stem(s):
    replaced = "".join("_" if (c in FORBIDDEN or unicodedata.category(c) == "Cc") else c for c in s)[:64]
    stem = replaced.rstrip(". ")
    if not stem:
        return "untitled"
    if _is_reserved(stem):
        stem = ("_" + stem)[:64].rstrip(". ")
    return stem


def pascal_of(name):
    return "".join(c if ord(c) < 128 else "?" for c in name[:255])


# ---------------------------------------------------------------- PackBits and a strict PSD reader

def unpackbits_row(data, width):
    out = bytearray()
    i, n = 0, len(data)
    while i < n:
        h = data[i]
        i += 1
        if h < 128:
            cnt = h + 1
            if i + cnt > n:
                raise ValueError("literal run past the row end")
            out += data[i:i + cnt]
            i += cnt
        elif h > 128:
            if i >= n:
                raise ValueError("repeat run past the row end")
            out += bytes([data[i]]) * (257 - h)
            i += 1
        # 128 is a no-op
    if len(out) != width:
        raise ValueError(f"row decodes to {len(out)} bytes, want {width}")
    return bytes(out)


class Reader:
    def __init__(self, data, what):
        self.d, self.p, self.what = data, 0, what

    def left(self):
        return len(self.d) - self.p

    def take(self, n):
        if n < 0 or n > self.left():
            raise ValueError(f"{self.what}: {n} bytes wanted at {self.p}, {self.left()} left")
        b = self.d[self.p:self.p + n]
        self.p += n
        return b

    def u8(self):
        return self.take(1)[0]

    def u16(self):
        return struct.unpack(">H", self.take(2))[0]

    def i16(self):
        return struct.unpack(">h", self.take(2))[0]

    def u32(self):
        return struct.unpack(">I", self.take(4))[0]

    def i32(self):
        return struct.unpack(">i", self.take(4))[0]

    def sub(self, n, what):
        return Reader(self.take(n), what)

    def zeros(self, n):
        if any(self.take(n)):
            raise ValueError(f"{self.what}: non-zero padding")

    def finish(self):
        if self.left():
            raise ValueError(f"{self.what}: {self.left()} unread bytes")


def read_plane(r, w, h):
    comp = r.u16()
    if comp == 0:
        return np.frombuffer(r.take(w * h), np.uint8).reshape(h, w)
    if comp != 1:
        raise ValueError(f"compression {comp}")
    lens = [r.u16() for _ in range(h)]
    rows = [unpackbits_row(r.take(n), w) for n in lens]
    return np.frombuffer(b"".join(rows), np.uint8).reshape(h, w)


def read_psd(data):
    r = Reader(data, "file")
    if r.take(4) != b"8BPS" or r.u16() != 1:
        raise ValueError("not a version-1 PSD")
    r.zeros(6)
    channels = r.u16()
    height, width = r.u32(), r.u32()
    depth, mode = r.u16(), r.u16()
    if (channels, depth, mode) != (4, 8, 3):
        raise ValueError(f"channels/depth/mode {channels}/{depth}/{mode}, want 4/8/3")
    if r.u32() != 0:
        raise ValueError("colour mode data not empty")
    res = r.sub(r.u32(), "image resources")
    while res.left():
        if res.take(4) != b"8BIM":
            raise ValueError("resource signature")
        res.u16()
        nl = res.u8()
        res.take(nl)
        if (1 + nl) % 2:
            res.zeros(1)
        sz = res.u32()
        res.take(sz)
        if sz % 2:
            res.zeros(1)
    sec_len = r.u32()
    sec = r.sub(sec_len, "layer and mask section")
    info_len = sec.u32()
    if info_len % 4 or sec_len != 4 + info_len + 4:
        raise ValueError(f"section length {sec_len} / layer info length {info_len}")
    info = sec.sub(info_len, "layer info")
    count = abs(info.i16())
    recs = []
    for _ in range(count):
        top, left, bottom, right = info.i32(), info.i32(), info.i32(), info.i32()
        if bottom < top or right < left:
            raise ValueError("inverted layer rect")
        nch = info.u16()
        chans = [(info.i16(), info.u32()) for _ in range(nch)]
        if sorted(c for c, _ in chans) != [-1, 0, 1, 2]:
            raise ValueError(f"channel ids {chans}")
        if info.take(4) != b"8BIM":
            raise ValueError("blend signature")
        blend = info.take(4).decode("latin-1")
        opacity, clipping, flags, filler = info.u8(), info.u8(), info.u8(), info.u8()
        extra = info.sub(info.u32(), "layer extra data")
        extra.take(extra.u32())  # mask data
        extra.take(extra.u32())  # blending ranges
        nl = extra.u8()
        pascal = extra.take(nl).decode("latin-1")
        extra.zeros((4 - (1 + nl) % 4) % 4)
        uname = None
        while extra.left():
            if extra.take(4) != b"8BIM":
                raise ValueError("additional info signature")
            key = extra.take(4)
            blk = extra.sub(extra.u32(), "additional info")
            if key == b"luni":
                n = blk.u32()
                uname = blk.take(2 * n).decode("utf-16-be")
                blk.zeros(blk.left())
        recs.append(dict(rect=(left, top, right, bottom), chans=chans, blend=blend, opacity=opacity,
                         clipping=clipping, hidden=bool(flags & 2), pascal=pascal, uname=uname))
    for rec in recs:
        l, t, rr, b = rec["rect"]
        w, h = rr - l, b - t
        planes = {}
        for cid, ln in rec["chans"]:
            c = info.sub(ln, "channel data")
            if w == 0 or h == 0:
                if c.u16() not in (0, 1):
                    raise ValueError("compression of an empty channel")
                c.finish()
                planes[cid] = np.zeros((0, 0), np.uint8)
                continue
            planes[cid] = read_plane(c, w, h)
            c.finish()
        if w and h:
            rec["rgba"] = np.dstack([planes[0], planes[1], planes[2], planes[-1]])
        else:
            rec["rgba"] = np.zeros((0, 0, 4), np.uint8)
    pad = info.left()
    if pad >= 4:
        raise ValueError(f"layer info: {pad} unread bytes")
    info.zeros(pad)
    if sec.u32() != 0:
        raise ValueError("global layer mask info not empty")
    sec.finish()
    comp = r.u16()
    if comp == 1:
        lens = [r.u16() for _ in range(4 * height)]
        planes = []
        for c in range(4):
            rows = [unpackbits_row(r.take(n), width) for n in lens[c * height:(c + 1) * height]]
            planes.append(np.frombuffer(b"".join(rows), np.uint8).reshape(height, width))
    elif comp == 0:
        planes = [np.frombuffer(r.take(width * height), np.uint8).reshape(height, width) for _ in range(4)]
    else:
        raise ValueError(f"merged compression {comp}")
    r.finish()
    return dict(width=width, height=height, layers=recs, merged=np.dstack(planes))


# ---------------------------------------------------------------- helpers

def rgba_array(img):
    return np.asarray(img.convert("RGBA"))


def alpha_bbox(arr):
    """(x0, y0, x1, y1) of alpha > 0, or None."""
    a = arr[:, :, 3] > 0
    if not a.any():
        return None
    ys = np.flatnonzero(a.any(axis=1))
    xs = np.flatnonzero(a.any(axis=0))
    return (int(xs[0]), int(ys[0]), int(xs[-1]) + 1, int(ys[-1]) + 1)


def matte_white(arr):
    if (arr[:, :, 3] == 255).all():
        return arr
    a = arr[:, :, 3:4].astype(np.uint32)
    rgb = (arr[:, :, :3].astype(np.uint32) * a + 255 * (255 - a) + 127) // 255
    return np.dstack([rgb.astype(np.uint8), arr[:, :, 3]])


def diff_count(a, b):
    if a.shape != b.shape:
        return f"shape {a.shape} != {b.shape}"
    n = int(np.count_nonzero((a != b).any(axis=-1))) if a.ndim == 3 else int(np.count_nonzero(a != b))
    return n


# ---------------------------------------------------------------- one texture set

def check_set(folder, stem, files):
    problems = []
    notes = []

    def fail(msg):
        problems.append(msg)

    # Per-layer PNGs.
    rx = re.compile("^" + re.escape(stem) + LAYER_RE)
    per_layer = []
    for f in files:
        m = rx.match(f)
        if m:
            per_layer.append((int(m.group(1)), m.group(2), m.group(3), f))
    per_layer.sort()
    if [p[0] for p in per_layer] != list(range(len(per_layer))):
        fail(f"per-layer PNG indices are not 0..{len(per_layer) - 1}: {[p[0] for p in per_layer]}")
    pngs = []
    canvas = None
    for idx, kind, name, f in per_layer:
        with Image.open(os.path.join(folder, f)) as im:
            arr = rgba_array(im)
        if canvas is None:
            canvas = (arr.shape[1], arr.shape[0])
        elif (arr.shape[1], arr.shape[0]) != canvas:
            fail(f"{f}: size {arr.shape[1]}x{arr.shape[0]} != {canvas}")
        pngs.append(dict(idx=idx, kind=kind, fname_name=name, file=f, arr=arr, bbox=alpha_bbox(arr)))

    # Flat image.
    flat = None
    flat_file = None
    for ext in FLAT_EXTS:
        if stem + "." + ext in files:
            flat_file = stem + "." + ext
            break
    if flat_file is None:
        notes.append("no flat file")
    else:
        with Image.open(os.path.join(folder, flat_file)) as im:
            flat = rgba_array(im)
        if flat_file.endswith((".jpg", ".jpeg")):
            notes.append("flat file is JPEG: not compared pixel by pixel")
            flat = None
        if canvas is None and flat is not None:
            canvas = (flat.shape[1], flat.shape[0])

    # ---------------- PSD
    psd_path = os.path.join(folder, stem + ".psd")
    psd = None
    if os.path.exists(psd_path):
        data = open(psd_path, "rb").read()
        try:
            psd = read_psd(data)
        except Exception as e:  # noqa: BLE001 - every reader error is a finding
            fail(f"PSD strict reader: {e}")
        # Pillow
        try:
            with Image.open(psd_path) as im:
                if im.format != "PSD":
                    fail(f"Pillow format {im.format}")
                pil_size = im.size
                pil_layers = [(n, b) for (n, _m, b, _t) in im.layers]
                im.load()  # the merged image (frame 0 tiles until a seek)
                pil_merged = np.asarray(im.convert("RGBA"))
            if psd is not None:
                if pil_size != (psd["width"], psd["height"]):
                    fail(f"Pillow size {pil_size}")
                if len(pil_layers) != len(psd["layers"]):
                    fail(f"Pillow layer count {len(pil_layers)} != {len(psd['layers'])}")
                for (pn, pb), rec in zip(pil_layers, psd["layers"]):
                    if pn != rec["pascal"]:
                        fail(f"Pillow layer name {pn!r} != {rec['pascal']!r}")
                    if tuple(pb) != rec["rect"]:
                        fail(f"Pillow layer box {pb} != {rec['rect']}")
                d = diff_count(pil_merged, psd["merged"])
                if d:
                    fail(f"Pillow merged image differs from the strict reader: {d}")
        except Exception as e:  # noqa: BLE001
            fail(f"Pillow cannot open the PSD: {e}")
        if psd is not None:
            if canvas and (psd["width"], psd["height"]) != canvas:
                fail(f"PSD canvas {psd['width']}x{psd['height']} != {canvas}")
            if len(psd["layers"]) != len(pngs):
                fail(f"PSD has {len(psd['layers'])} layers, {len(pngs)} per-layer PNGs")
            for rec, p in zip(psd["layers"], pngs):
                tag = f"PSD layer {p['idx']}"
                if rec["uname"] is None:
                    fail(f"{tag}: no unicode name")
                else:
                    want = sanitize_file_stem(rec["uname"])
                    got = p["fname_name"]
                    if got != want and not re.fullmatch(re.escape(want) + r"_\d+", got):
                        fail(f"{tag}: name {rec['uname']!r} does not give file name part {got!r}")
                    if rec["pascal"] != pascal_of(rec["uname"]):
                        fail(f"{tag}: Pascal name {rec['pascal']!r} != {pascal_of(rec['uname'])!r}")
                if rec["clipping"] != 0:
                    fail(f"{tag}: clipping {rec['clipping']}")
                l, t, r_, b = rec["rect"]
                if p["bbox"] is None:
                    if (r_ - l) * (b - t) != 0:
                        fail(f"{tag}: PNG empty but PSD rect {rec['rect']}")
                    continue
                if rec["rect"] != p["bbox"]:
                    fail(f"{tag}: rect {rec['rect']} != PNG content box {p['bbox']}")
                    continue
                crop = p["arr"][t:b, l:r_]
                d = diff_count(rec["rgba"], crop)
                if d:
                    fail(f"{tag}: {d} pixels differ from {p['file']}")
            if flat is not None:
                want = matte_white(flat)
                d = diff_count(psd["merged"], want)
                if d:
                    fail(f"PSD merged image: {d} pixels differ from {flat_file} (matted on white)")
    else:
        notes.append("no PSD")

    # ---------------- ORA
    ora_path = os.path.join(folder, stem + ".ora")
    ora_layers = None
    if os.path.exists(ora_path):
        try:
            with zipfile.ZipFile(ora_path) as z:
                infos = z.infolist()
                if not infos or infos[0].filename != "mimetype" or infos[0].compress_type != zipfile.ZIP_STORED:
                    fail("ORA: mimetype is not the first, stored entry")
                if z.read("mimetype") != b"image/openraster":
                    fail("ORA: mimetype content")
                bad = z.testzip()
                if bad:
                    fail(f"ORA: CRC error in {bad}")
                root = ET.fromstring(z.read("stack.xml"))
                if (int(root.get("w")), int(root.get("h"))) != canvas:
                    fail(f"ORA canvas {root.get('w')}x{root.get('h')} != {canvas}")
                stack = root.find("stack")
                elems = list(stack.findall("layer"))[::-1]  # bottom first
                ora_layers = []
                for i, e in enumerate(elems):
                    src = e.get("src")
                    with Image.open(io.BytesIO(z.read(src))) as im:
                        arr = rgba_array(im)
                    ora_layers.append(dict(name=e.get("name"), src=src, x=int(e.get("x")), y=int(e.get("y")),
                                           opacity=float(e.get("opacity")), hidden=e.get("visibility") == "hidden",
                                           op=e.get("composite-op"), arr=arr))
                with Image.open(io.BytesIO(z.read("mergedimage.png"))) as im:
                    merged = rgba_array(im)
                with Image.open(io.BytesIO(z.read("Thumbnails/thumbnail.png"))) as im:
                    tw, th = im.size
                if max(tw, th) > 256:
                    fail(f"ORA thumbnail {tw}x{th}")
            if len(ora_layers) != len(pngs):
                fail(f"ORA has {len(ora_layers)} layers, {len(pngs)} per-layer PNGs")
            for i, (ol, p) in enumerate(zip(ora_layers, pngs)):
                tag = f"ORA layer {p['idx']}"
                if ol["src"] != f"data/layer{i:03d}.png":
                    fail(f"{tag}: src {ol['src']}")
                want = sanitize_file_stem(ol["name"])
                got = p["fname_name"]
                if got != want and not re.fullmatch(re.escape(want) + r"_\d+", got):
                    fail(f"{tag}: name {ol['name']!r} does not give file name part {got!r}")
                h, w = ol["arr"].shape[:2]
                if p["bbox"] is None:
                    if (w, h, ol["x"], ol["y"]) != (1, 1, 0, 0) or ol["arr"][:, :, 3].any():
                        fail(f"{tag}: empty layer is not a 1x1 transparent PNG at 0,0")
                    continue
                box = (ol["x"], ol["y"], ol["x"] + w, ol["y"] + h)
                if box != p["bbox"]:
                    fail(f"{tag}: box {box} != PNG content box {p['bbox']}")
                    continue
                crop = p["arr"][box[1]:box[3], box[0]:box[2]]
                d = diff_count(ol["arr"], crop)
                if d:
                    fail(f"{tag}: {d} pixels differ from {p['file']}")
            if flat is not None:
                d = diff_count(merged, flat)
                if d:
                    fail(f"ORA mergedimage.png: {d} pixels differ from {flat_file}")
        except Exception as e:  # noqa: BLE001
            fail(f"ORA cannot be read: {e}")
    else:
        notes.append("no ORA")

    # ---------------- PSD vs ORA (names, opacity, visibility)
    if psd is not None and ora_layers is not None and len(psd["layers"]) == len(ora_layers):
        for i, (rec, ol) in enumerate(zip(psd["layers"], ora_layers)):
            if rec["uname"] != ol["name"]:
                # ORA drops control characters it cannot represent; compare after that.
                cleaned = "".join(c for c in (rec["uname"] or "") if ord(c) >= 0x20 or c in "\t\n\r")
                if cleaned != ol["name"]:
                    fail(f"layer {i}: PSD name {rec['uname']!r} != ORA name {ol['name']!r}")
            if rec["hidden"] != ol["hidden"]:
                fail(f"layer {i}: PSD hidden {rec['hidden']} != ORA {ol['hidden']}")
            if abs(rec["opacity"] - round(ol["opacity"] * 255)) > 1:
                fail(f"layer {i}: PSD opacity {rec['opacity']} != ORA {ol['opacity']}")

    names = [p["fname_name"] for p in pngs]
    summary = (f"{stem}: canvas {canvas[0] if canvas else '?'}x{canvas[1] if canvas else '?'}, "
               f"{len(pngs)} layers {names}, flat {flat_file}")
    return problems, notes, summary


def check_folder(folder):
    files = sorted(os.listdir(folder))
    stems = sorted({f[:-4] for f in files if f.lower().endswith((".psd", ".ora"))})
    total_bad = 0
    print(f"== {folder}")
    if not stems:
        print("   FAIL: no .psd or .ora file")
        return 1
    leftovers = [f for f in files if ".tmp-" in f]
    if leftovers:
        print(f"   FAIL: temp files left: {leftovers}")
        total_bad += 1
    for stem in stems:
        problems, notes, summary = check_set(folder, stem, files)
        state = "ok  " if not problems else "FAIL"
        extra = f" ({'; '.join(notes)})" if notes else ""
        print(f"   {state} {summary}{extra}")
        for p in problems:
            print(f"        - {p}")
        total_bad += bool(problems)
    return total_bad


def main(argv):
    # Layer and set names may hold any Unicode (also lone surrogates turned into U+FFFD); never fail on printing them.
    sys.stdout.reconfigure(encoding="utf-8", errors="backslashreplace")
    if len(argv) < 2:
        print(__doc__)
        return 2
    bad = 0
    for folder in argv[1:]:
        bad += check_folder(folder)
    print(f"RESULT: {'PASS' if bad == 0 else 'FAIL'} ({bad} failing set(s))")
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
