import sys, os
from PIL import Image, ImageDraw
out = sys.argv[1]; dirs = sys.argv[2:]
tiles = []
for d in dirs:
    p = os.path.join('runs', d, 'shot.png')
    if not os.path.exists(p): continue
    im = Image.open(p).convert('RGB').resize((960, 600), Image.LANCZOS)
    ImageDraw.Draw(im).text((300, 2), d, fill=(255, 255, 0))
    tiles.append(im)
cols = 2; rows = (len(tiles) + 1) // 2
sheet = Image.new('RGB', (960 * cols, 600 * rows))
for i, t in enumerate(tiles): sheet.paste(t, (960 * (i % cols), 600 * (i // cols)))
sheet.save(out)
