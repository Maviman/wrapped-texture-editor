#!/bin/sh
# Makes fixture f1w (STICKER_PLAN 8.3 f1, rebuilt with the sticker writer in P5b) with PhotoCraft v0.2.0's console
# exe, headless (no window). Usage: sh make_f1w.sh   (from a scratch folder that holds f1w_wte_in.psd, written by
# the ignored test psd::write_tests::k5_write_make_f1w_input; writes ./out)
set -e
PC=/g/Self-made_products/photocraft-0.2.0-windows-x86-portable/photocraft-cli.exe
OUT="$(pwd -W 2>/dev/null || pwd)/out"
rm -rf out && mkdir -p out
"$PC" convert f1w_wte_in.psd out/f1w_pc.psd < /dev/null
"$PC" convert out/f1w_pc.psd out/f1w_pc_merged.png < /dev/null
"$PC" run out/f1w_pc.psd --cmd file.export.layersToFiles --params "{\"dir\":\"$OUT/f1w_pc_layers\",\"format\":\"png\",\"visibleOnly\":false}" < /dev/null
echo done
