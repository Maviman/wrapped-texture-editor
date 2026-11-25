#!/bin/sh
# Companion off = app unchanged: runs the regression scenarios with one exe, sequentially, off-screen (builds of this
# tree open in the background without focus). Usage (Git Bash, repo root):
#   sh tools/companion/regress.sh EXE OUTDIR
# Writes OUTDIR/<scenario>.png/.json/.log (+ named shots, export folders). Compare two OUTDIRs with
# python -E -P tools/companion/compare_regress.py BASE NEW.
exe="$1"
out="$2"
if [ ! -f "$exe" ]; then echo "no exe: $exe"; exit 1; fi
mkdir -p "$out"
for s in smoke default paint3d paint2d pen side_tabs export export_run; do
  rm -rf "$out/${s}_export" "$out/$s.png" "$out/$s.json"
  "$exe" --screenshot "$out/$s.png" --scenario "$s" --size 1280x800 --log "$out/$s.log" < /dev/null
  printf '%s exit=%s ' "$s" "$?"
  grep -o '"ok": [a-z]*' "$out/$s.json" 2>/dev/null || echo "(no json)"
done
