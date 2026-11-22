import re, sys, statistics as st
log = sys.argv[1]
step = 0; total = 1000; rows = []
for line in open(log, encoding='utf-8', errors='replace'):
    m = re.search(r'fuzz (\d+)/(\d+):', line)
    if m: step, total = int(m.group(1)), int(m.group(2)); continue
    m = re.search(r'frame profile: (\d+) frames, ([\d.]+) fps, interval avg ([\d.]+) ms max ([\d.]+) ms, ui cpu avg ([\d.]+) ms max ([\d.]+)', line)
    if m and step > 0: rows.append((step, int(m.group(1)), float(m.group(2)), float(m.group(5)), float(m.group(6))))
for q in range(4):
    r = [x for x in rows if q * total / 4 < x[0] <= (q + 1) * total / 4]
    if not r: continue
    fps = sorted(x[2] for x in r); cpu = sorted(x[3] for x in r)
    print(f"Q{q+1}: windows {len(r):3d}  fps median {st.median(fps):5.1f}  mean {st.mean(fps):5.1f}  p10 {fps[len(fps)//10]:5.1f} | ui cpu avg: median {st.median(cpu):6.2f} ms  p90 {cpu[int(len(cpu)*0.9)]:7.2f} ms | worst frame {max(x[4] for x in r):7.1f} ms | windows <30 fps: {sum(1 for x in r if x[2] < 30)}")
