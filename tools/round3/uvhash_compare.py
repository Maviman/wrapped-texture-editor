"""Compare manifest import uv_hash of the saved workflow projects of two sweeps.
python -I uvhash_compare.py <runs dir A> <runs dir B> [name filter]"""
import json
import os
import sys
import zipfile


def find_hash(obj):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == "uv_hash":
                return v
            r = find_hash(v)
            if r is not None:
                return r
    elif isinstance(obj, list):
        for v in obj:
            r = find_hash(v)
            if r is not None:
                return r
    return None


def manifest_hash(path):
    with zipfile.ZipFile(path) as z:
        for n in z.namelist():
            if n.endswith(".json") and "manifest" in n:
                return find_hash(json.loads(z.read(n)))
    return None


def collect(root, filt):
    out = {}
    for d in sorted(os.listdir(root)):
        if filt and filt not in d:
            continue
        p = os.path.join(root, d, "shot.wtep")
        if os.path.exists(p):
            try:
                out[d] = manifest_hash(p)
            except Exception as e:  # noqa: BLE001
                out[d] = f"error {e}"
    return out


a = collect(sys.argv[1], sys.argv[3] if len(sys.argv) > 3 else "")
b = collect(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "")
same = diff = 0
for k in sorted(set(a) | set(b)):
    ha, hb = a.get(k), b.get(k)
    state = "same" if ha == hb and ha is not None else "DIFF"
    same += state == "same"
    diff += state != "same"
    print(f"{state:4} {k}: {ha} | {hb}")
print(f"same {same}, different or missing {diff}")
