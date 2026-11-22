import re, sys, collections
log = sys.argv[1]; thr = float(sys.argv[2]) if len(sys.argv) > 2 else 400
last = []; agg = collections.defaultdict(lambda: [0, 0.0, 0.0])
for line in open(log, encoding='utf-8', errors='replace'):
    m = re.search(r'fuzz (\d+)/(\d+): (.*)', line)
    if m: last.append(m.group(3).strip()); last = last[-6:]; continue
    m = re.search(r'frame profile: (\d+) frames.*ui cpu avg ([\d.]+) ms max ([\d.]+) ms; segments avg/max ms: (.*)', line)
    if m and float(m.group(3)) >= thr:
        segs = [(s.rsplit(' ', 1)[0], float(s.rsplit('/', 1)[1])) for s in m.group(4).split(', ')]
        top = max(segs, key=lambda s: s[1])
        kinds = sorted(set(re.sub(r'[^A-Za-z_ ].*', '', a).strip()[:40] for a in last))
        print(f"{m.group(3):>8} ms  seg={top[0]:<22} recent={' | '.join(a[:70] for a in last[-4:])}")
