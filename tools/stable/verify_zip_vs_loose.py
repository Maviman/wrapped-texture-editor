"""fix_export item 10: checks the zips of export_flow runs against the loose files, byte for byte.

For every `<stem>_all` folder under the given run folders (export_flow writes `<out>_<model>_all\<base>.zip`,
`<out>_<model>_all\<base>_selection.zip` and the same export as loose files in `<out>_<model>_loose\`):

- `<base>.zip`: `testzip()` is clean; its entry names are exactly the loose file names; every entry's bytes equal the
  loose file's bytes; no entry has a folder; every entry is dated 1980-01-01; PNG/ORA entries are stored, the rest
  deflated; non-ASCII names carry the UTF-8 flag.
- `<base>_selection.zip`: clean; every entry exists among the loose files with the same bytes (a selection uses the
  full export's names).
- no `.tmp-` file or folder is left in any of the run's folders.

Usage: python -E -P verify_zip_vs_loose.py RUN_DIR [RUN_DIR ...] [--json OUT.json]
Exit code 0 when everything holds, 1 otherwise.
"""

import json
import os
import sys
import zipfile

STORED_EXT = {"png", "jpg", "jpeg", "webp", "ora"}


def check_flow(all_dir):
    stem = all_dir[: -len("_all")]
    loose_dir = stem + "_loose"
    out = {"all": all_dir, "loose": loose_dir, "problems": [], "entries": 0, "selection_entries": 0}
    problems = out["problems"]
    if not os.path.isdir(loose_dir):
        problems.append("no loose folder " + loose_dir)
        return out
    loose = sorted(os.listdir(loose_dir))
    zips = sorted(n for n in os.listdir(all_dir) if n.endswith(".zip"))
    full = [n for n in zips if not n.endswith("_selection.zip")]
    sel = [n for n in zips if n.endswith("_selection.zip")]
    if len(full) != 1:
        problems.append("expected one full zip, found %r" % zips)
        return out
    out["zip"] = full[0]
    with zipfile.ZipFile(os.path.join(all_dir, full[0])) as z:
        bad = z.testzip()
        if bad is not None:
            problems.append("testzip: bad entry " + bad)
        infos = z.infolist()
        names = [i.filename for i in infos]
        out["entries"] = len(names)
        if sorted(names) != loose:
            problems.append("entry names differ from the loose files: only in zip %r, only loose %r"
                            % (sorted(set(names) - set(loose)), sorted(set(loose) - set(names))))
        for info in infos:
            name = info.filename
            if "/" in name or "\\" in name:
                problems.append("entry with a folder: " + name)
            if info.date_time != (1980, 1, 1, 0, 0, 0):
                problems.append("%s dated %r" % (name, info.date_time))
            ext = name.rsplit(".", 1)[-1].lower()
            want = zipfile.ZIP_STORED if ext in STORED_EXT else zipfile.ZIP_DEFLATED
            if info.compress_type != want:
                problems.append("%s: method %d, expected %d" % (name, info.compress_type, want))
            if any(ord(c) > 127 for c in name) and not info.flag_bits & 0x800:
                problems.append(name + ": non-ASCII name without the UTF-8 flag")
            path = os.path.join(loose_dir, name)
            if os.path.isfile(path):
                with open(path, "rb") as f:
                    if f.read() != z.read(info):
                        problems.append(name + ": zip entry differs from the loose file")
    for s in sel:
        with zipfile.ZipFile(os.path.join(all_dir, s)) as z:
            bad = z.testzip()
            if bad is not None:
                problems.append(s + " testzip: bad entry " + bad)
            out["selection_entries"] = len(z.infolist())
            for info in z.infolist():
                path = os.path.join(loose_dir, info.filename)
                if not os.path.isfile(path):
                    problems.append("%s: %s is not a name of the full export" % (s, info.filename))
                    continue
                with open(path, "rb") as f:
                    if f.read() != z.read(info):
                        problems.append("%s: %s differs from the full export's file" % (s, info.filename))
    out["selection_zip"] = sel
    return out


def leftovers(run_dir):
    found = []
    for root, dirs, files in os.walk(run_dir):
        for n in dirs + files:
            if ".tmp-" in n:
                found.append(os.path.join(root, n))
    return found


def main(argv):
    out_json = None
    if "--json" in argv:
        i = argv.index("--json")
        out_json = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    results = []
    ok = True
    for run_dir in argv:
        for n in sorted(os.listdir(run_dir)):
            p = os.path.join(run_dir, n)
            if n.endswith("_all") and os.path.isdir(p):
                r = check_flow(p)
                results.append(r)
                ok &= not r["problems"]
                print("%-70s %3d entries, selection %d: %s" % (n, r["entries"], r["selection_entries"],
                                                             "OK" if not r["problems"] else r["problems"]))
        left = leftovers(run_dir)
        if left:
            ok = False
            print("temp files left:", left)
            results.append({"leftovers": left})
    if not results:
        ok = False
        print("no export_flow output found")
    if out_json:
        with open(out_json, "w", encoding="utf-8") as f:
            json.dump({"ok": ok, "results": results}, f, indent=1)
    print("ALL OK" if ok else "PROBLEMS")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
