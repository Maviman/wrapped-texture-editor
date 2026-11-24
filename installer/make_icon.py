"""Turns the app's window icon (a 64x64 PNG written by the ignored test theme::tests::write_icon_png) into
installer/app.ico with the sizes Windows asks for (16, 24, 32, 48, 64).

    set WTE_ICON_OUT=target\icon.png
    cargo test -p wte_app --bin wrapped_texture_editor write_icon_png -- --ignored
    python installer/make_icon.py target/icon.png installer/app.ico
"""
import sys
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
img = Image.open(src).convert("RGBA")
img.save(dst, format="ICO", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64)])
print(f"wrote {dst}")
