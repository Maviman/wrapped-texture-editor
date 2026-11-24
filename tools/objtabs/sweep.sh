#!/bin/sh
# objects-tabs gate sweep (docs/reports/OBJECTS_TABS_GATE_REPORT.md). Runs the jobs of JOBSFILE one after the other
# with EXE (the app opens off-screen in the background and never takes the focus) and writes
# OUTDIR/<scenario>[_<tag>]/shot.png|json|log plus OUTDIR/summary.txt ("<name> exit=<code> ok=<true|false> secs=<n>").
#
# Usage: sh tools/objtabs/sweep.sh EXE OUTDIR JOBSFILE
# Jobs: one per line, "scenario|model|tag|size": model "-" = none; "@SAMPLES@" in a model path is replaced by
# $WTE_SAMPLES (default G:/Claude_Resources/samples/khronos-gltf), "@REPO@" by the repo root; size defaults to 1280x800.
# A job whose model file is missing is reported as MISSING-MODEL and not started (a missing path pops a dialog).
EXE="$1"; OUT="$2"; JOBS="$3"
REPO=$(cd "$(dirname "$0")/../.." && pwd)
SAMPLES="${WTE_SAMPLES:-G:/Claude_Resources/samples/khronos-gltf}"
[ -f "$EXE" ] || { echo "missing exe $EXE"; exit 1; }
mkdir -p "$OUT"
: > "$OUT/summary.txt"
while IFS="|" read -r sc model tag size; do
  [ -z "$sc" ] && continue
  case "$sc" in \#*) continue;; esac
  SIZE="${size:-1280x800}"
  model=$(printf '%s' "$model" | sed "s#@SAMPLES@#$SAMPLES#; s#@REPO@#$REPO#")
  name="$sc"; [ -n "$tag" ] && name="${sc}_$tag"
  dir="$OUT/$name"; rm -rf "$dir"; mkdir -p "$dir"
  if [ "$model" != "-" ] && [ ! -f "$model" ]; then echo "$name MISSING-MODEL" >> "$OUT/summary.txt"; continue; fi
  t0=$(date +%s)
  if [ "$model" = "-" ]; then
    timeout 900 "$EXE" --screenshot "$dir/shot.png" --scenario "$sc" --size "$SIZE" < /dev/null > "$dir/stdout.txt" 2>&1
  else
    timeout 900 "$EXE" "$model" --screenshot "$dir/shot.png" --scenario "$sc" --size "$SIZE" < /dev/null > "$dir/stdout.txt" 2>&1
  fi
  code=$?
  t1=$(date +%s)
  ok=$(grep -o '"ok": *[a-z]*' "$dir/shot.json" 2>/dev/null | head -1 | sed 's/.*: *//')
  echo "$name exit=$code ok=${ok:-none} secs=$((t1-t0))" >> "$OUT/summary.txt"
done < "$JOBS"
echo done
