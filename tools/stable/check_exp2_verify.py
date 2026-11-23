"""check_exp2 (stable-fix-export): independent verification of export_flow outputs (stable-fix-export).

Usage: python -E -P verify.py RUN_DIR [RUN_DIR ...] --json OUT.json
RUN_DIR holds <stem>_all, <stem>_loose, <stem>_one, <stem>_sel folders (export_flow output).
"""
import io
import json
import os
import re
import struct
import sys
import zipfile

import numpy as np
from PIL import Image

LAYER_RE = re.compile(r"^(?P<prefix>.*)_L(?P<idx>\d\d)_(?P<kind>paint|stamp|text|base|raster)_(?P<name>.*)\.png$")


def load(data):
    im = Image.open(io.BytesIO(data))
    im.load()
    return np.asarray(im.convert("RGBA"), dtype=np.float64) / 255.0


def over(dst, src):
    sa = src[..., 3:4]
    da = dst[..., 3:4]
    oa = sa + da * (1 - sa)
    rgb = src[..., :3] * sa + dst[..., :3] * da * (1 - sa)
    out = np.zeros_like(dst)
    out[..., 3:4] = oa
    nz = oa[..., 0] > 0
    out[..., :3][nz] = rgb[nz] / oa[nz]
    return out


def premul(a):
    return np.concatenate([a[..., :3] * a[..., 3:4], a[..., 3:4]], axis=-1)


def flows(run_dir):
    for name in sorted(os.listdir(run_dir)):
        p = os.path.join(run_dir, name)
        if os.path.isdir(p) and name.endswith("_all"):
            yield p[: -len("_all")]


def check(stem):
    res = {"stem": stem, "problems": [], "notes": []}
    pr = res["problems"]
    alld, loosed, oned, seld = (stem + s for s in ("_all", "_loose", "_one", "_sel"))
    zips = sorted(n for n in os.listdir(alld) if n.endswith(".zip"))
    full = [n for n in zips if not n.endswith("_selection.zip")]
    sel = [n for n in zips if n.endswith("_selection.zip")]
    res["zips"] = zips
    if len(full) != 1 or len(sel) != 1:
        pr.append("zips in _all: %r" % zips)
        return res
    if sel[0] != full[0][:-4] + "_selection.zip":
        pr.append("selection zip name %s vs %s" % (sel[0], full[0]))
    zf = zipfile.ZipFile(os.path.join(alld, full[0]))
    bad = zf.testzip()
    if bad:
        pr.append("testzip " + bad)
    entries = {i.filename: zf.read(i) for i in zf.infolist()}
    order = [i.filename for i in zf.infolist()]
    for i in zf.infolist():
        if i.date_time[:3] != (1980, 1, 1):
            pr.append("date " + i.filename)
        if "/" in i.filename or "\\" in i.filename:
            pr.append("folder in entry " + i.filename)
        if any(ord(c) > 127 for c in i.filename) and not (i.flag_bits & 0x800):
            pr.append("no utf8 flag " + i.filename)
    res["full_entries"] = len(entries)
    # zip vs loose
    if os.path.isdir(loosed):
        loose = sorted(os.listdir(loosed))
        if sorted(entries) != loose:
            pr.append("loose names != zip names: only zip %r only loose %r" % (sorted(set(entries) - set(loose))[:5], sorted(set(loose) - set(entries))[:5]))
        for n in loose:
            if n in entries:
                with open(os.path.join(loosed, n), "rb") as f:
                    if f.read() != entries[n]:
                        pr.append("loose differs " + n)
    else:
        res["notes"].append("no loose folder")
    # selection zip vs full
    sz = zipfile.ZipFile(os.path.join(alld, sel[0]))
    if sz.testzip():
        pr.append("sel testzip")
    sentries = {i.filename: sz.read(i) for i in sz.infolist()}
    res["selection_entries"] = sorted(sentries)
    for n, b in sentries.items():
        if n not in entries:
            pr.append("selection entry not in full: " + n)
        elif entries[n] != b:
            pr.append("selection entry differs: " + n)
    # loose one / sel exports vs full
    for d in (oned, seld):
        if not os.path.isdir(d):
            continue
        names = sorted(os.listdir(d))
        res[os.path.basename(d)] = names
        for n in names:
            if ".tmp-" in n:
                pr.append("temp left " + n)
                continue
            with open(os.path.join(d, n), "rb") as f:
                b = f.read()
            if n not in entries:
                pr.append("%s: %s not a full-export name" % (os.path.basename(d), n))
            elif entries[n] != b:
                pr.append("%s: %s differs from full" % (os.path.basename(d), n))
    # sel loose vs selection zip: per-layer png names of the selection loose export should equal the zip entries
    if os.path.isdir(seld):
        sel_pngs = sorted(n for n in os.listdir(seld) if LAYER_RE.match(n))
        if sel_pngs != sorted(sentries):
            res["notes"].append("sel folder layer files %r vs selection zip %r" % (sel_pngs, sorted(sentries)))
    for d in (alld, loosed, oned, seld):
        if os.path.isdir(d) and any(".tmp-" in n for n in os.listdir(d)):
            pr.append("temp left in " + d)
    # flattened vs recomposite, ORA, PSD, per set
    layers_by_prefix = {}
    flats = {}
    for n in order:
        m = LAYER_RE.match(n)
        if m:
            layers_by_prefix.setdefault(m.group("prefix"), []).append((int(m.group("idx")), n))
    for n in order:
        if n.endswith(".ora") or n.endswith(".psd") or LAYER_RE.match(n):
            continue
        stem_n = os.path.splitext(n)[0]
        flats[stem_n] = n
    worst = 0.0
    recomposed = 0
    for prefix, items in layers_by_prefix.items():
        items.sort()
        flat = flats.get(prefix)
        if flat is None:
            continue
        canvas = None
        for _, n in items:
            img = load(entries[n])
            canvas = img.copy() if canvas is None else over(canvas, img)
        if not flat.lower().endswith(".png"):
            continue
        f = load(entries[flat])
        d = np.abs(premul(f) - premul(canvas)).max() * 255
        worst = max(worst, d)
        recomposed += 1
        if d > 2.0:
            pr.append("flat %s differs from recomposite by %.2f/255" % (flat, d))
        ora = prefix + ".ora"
        if ora in entries:
            oz = zipfile.ZipFile(io.BytesIO(entries[ora]))
            first = oz.infolist()[0]
            if first.filename != "mimetype" or first.compress_type != zipfile.ZIP_STORED:
                pr.append("ora mimetype " + ora)
            stack = oz.read("stack.xml").decode()
            nl = stack.count("<layer ")
            if nl != len(items):
                pr.append("ora %s has %d layers, %d pngs" % (ora, nl, len(items)))
            merged = load(oz.read("mergedimage.png"))
            if merged.shape != f.shape or np.abs(merged - f).max() > 0:
                pr.append("ora merged != flat " + ora)
        psd = prefix + ".psd"
        if psd in entries:
            b = entries[psd]
            sig, ver = b[:4], struct.unpack(">H", b[4:6])[0]
            ch, h, w, depth, mode = struct.unpack(">HIIHH", b[12:26])
            if sig != b"8BPS" or ver != 1 or depth != 8 or mode != 3 or (h, w) != f.shape[:2]:
                pr.append("psd header " + psd)
    res["recomposed_sets"] = recomposed
    res["worst_flat_diff_255"] = round(worst, 3)
    return res


def main():
    args = sys.argv[1:]
    out = None
    if "--json" in args:
        i = args.index("--json")
        out = args[i + 1]
        del args[i : i + 2]
    results = []
    for run in args:
        for stem in flows(run):
            results.append(check(stem))
    ok = all(not r["problems"] for r in results)
    for r in results:
        print(("OK  " if not r["problems"] else "BAD ") + r["stem"], r.get("full_entries"), "entries,", r.get("recomposed_sets"), "sets recomposed, worst", r.get("worst_flat_diff_255"))
        for p in r["problems"][:20]:
            print("   ", p)
        for n in r["notes"]:
            print("   note:", n)
    if out:
        with open(out, "w", encoding="utf-8") as f:
            json.dump({"ok": ok, "results": results}, f, indent=1, ensure_ascii=False)
    print("ALL OK" if ok else "PROBLEMS")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
