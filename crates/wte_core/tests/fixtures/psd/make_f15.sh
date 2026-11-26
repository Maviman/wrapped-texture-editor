#!/bin/sh
# Makes fixture f15 (stickers P5c: a link export with a picture, a text, a shape and a masked picture, re-saved by
# PhotoCraft) with PhotoCraft v0.2.0's console exe, headless (no window). Usage: sh make_f15.sh   (from a scratch folder
# that holds f15_link_wte.psd, written by the ignored test link::fixture_tests::k5_link_make_f15_input; writes ./out)
set -e
PC=/g/Self-made_products/photocraft-0.2.0-windows-x86-portable/photocraft-cli.exe
rm -rf out && mkdir -p out
"$PC" convert f15_link_wte.psd out/f15_link_pc.psd < /dev/null
"$PC" convert out/f15_link_pc.psd out/f15_link_pc_merged.png < /dev/null
echo done
