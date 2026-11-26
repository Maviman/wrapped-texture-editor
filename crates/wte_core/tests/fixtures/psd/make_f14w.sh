#!/bin/sh
# Makes fixture f14w (P5b checker): the writer's rich test sticker re-saved by PhotoCraft v0.2.0's console exe,
# headless (no window). Usage: sh make_f14w.sh   (from a scratch folder that holds rich.psd, written by the ignored
# test psd::write_tests::k5_write_dump_for_psd_check; writes ./out)
set -e
PC=/g/Self-made_products/photocraft-0.2.0-windows-x86-portable/photocraft-cli.exe
rm -rf out && mkdir -p out
"$PC" convert rich.psd out/f14w_rich_pc.psd < /dev/null
echo done
