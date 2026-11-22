"""Summarise fuzz logs: python -I fuzz_check.py <runs dir>
Per run: WARN / ERROR line counts, the distinct WARN / ERROR messages (timestamps cut, numbers kept), the distinct
toast texts by level, panics, and the worst frame interval of the frame profile lines."""
import collections
import os
import re
import sys

LINE = re.compile(r"^\[\S+ (WARN|ERROR|INFO|DEBUG)\s+(\S+)\] (.*)$")
FRAME = re.compile(r"interval avg [\d.]+ ms max ([\d.]+) ms")

runs = sys.argv[1]
all_toasts = collections.Counter()
for d in sorted(os.listdir(runs)):
    log = os.path.join(runs, d, "shot.log")
    if not os.path.exists(log):
        continue
    counts = collections.Counter()
    msgs = collections.Counter()
    toasts = collections.Counter()
    worst = 0.0
    panics = 0
    steps = None
    for line in open(log, encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if "panicked" in line:
            panics += 1
        m = FRAME.search(line)
        if m:
            worst = max(worst, float(m.group(1)))
        m = LINE.match(line)
        if not m:
            continue
        level, target, text = m.groups()
        if "fuzz" in text and "steps" in text and steps is None and "done" in text:
            steps = text
        if target.endswith("toasts") and text.startswith("toast: "):
            toasts[(level, text[7:])] += 1
        if level in ("WARN", "ERROR"):
            counts[level] += 1
            if not target.endswith("toasts"):
                msgs[(level, target, text)] += 1
    print(f"== {d}: WARN {counts['WARN']}, ERROR {counts['ERROR']}, panics {panics}, worst frame interval {worst:.0f} ms")
    for (level, target, text), n in sorted(msgs.items()):
        print(f"   {level} x{n} {target}: {text[:200]}")
    for (level, text), n in sorted(toasts.items()):
        if level != "INFO" or not re.match(r"(Undo|Redo): ", text):
            print(f"   toast {level} x{n}: {text[:200]}")
        all_toasts[(level, re.sub(r"\d+", "N", text))] += n
print("== distinct toast texts over all runs (numbers as N), Undo/Redo labels folded:")
folded = collections.Counter()
for (level, text), n in all_toasts.items():
    key = (level, re.sub(r"^(Undo|Redo): .*", r"\1: <label>", text))
    folded[key] += n
for (level, text), n in sorted(folded.items()):
    print(f"   {level} x{n}: {text[:200]}")
