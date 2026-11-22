"""Contact sheets of run screenshots: python -I make_sheets.py <runs dir> <out dir> <prefix> [per_sheet=9]
3 columns of 640x400 tiles, the run folder name printed on each tile."""
import os
import sys

from PIL import Image, ImageDraw

runs, out, prefix = sys.argv[1], sys.argv[2], sys.argv[3]
per = int(sys.argv[4]) if len(sys.argv) > 4 else 9
os.makedirs(out, exist_ok=True)
dirs = [d for d in sorted(os.listdir(runs)) if os.path.exists(os.path.join(runs, d, "shot.png"))]
cols, tw, th = 3, 640, 400
for s in range(0, len(dirs), per):
    chunk = dirs[s:s + per]
    rows = (len(chunk) + cols - 1) // cols
    sheet = Image.new("RGB", (tw * cols, th * rows), (40, 0, 40))
    for i, d in enumerate(chunk):
        im = Image.open(os.path.join(runs, d, "shot.png")).convert("RGB").resize((tw, th), Image.LANCZOS)
        dr = ImageDraw.Draw(im)
        dr.rectangle((0, 0, tw, 14), fill=(0, 0, 0))
        dr.text((4, 1), d, fill=(255, 255, 0))
        sheet.paste(im, (tw * (i % cols), th * (i // cols)))
    path = os.path.join(out, f"{prefix}_{s // per:02d}.png")
    sheet.save(path)
    print(path, len(chunk))
