#!/bin/sh
set -e
PC=/g/Self-made_products/photocraft-0.2.0-windows-x86-portable/photocraft-cli.exe
OUT="$(pwd -W 2>/dev/null || pwd)/out"
NEW16='{"width":64,"height":48,"background":"transparent","depth":16,"name":"fx"}'
"$PC" run --new "$NEW16" \
  --cmd select.rect --params '{"x":2,"y":2,"width":50,"height":40}' --cmd edit.fill --params '{"color":"#20a0c0"}' \
  --cmd select.rect --params '{"x":10,"y":6,"width":36,"height":30,"ellipse":true,"feather":3}' \
  --cmd layer.layerMask.revealSelection --cmd select.deselect \
  --cmd layer.new.layer --params '{"name":"Слой ü"}' \
  --cmd select.rect --params '{"x":30,"y":20,"width":30,"height":26,"ellipse":true,"feather":5}' --cmd edit.fill --params '{"color":"#e04080"}' \
  --cmd layer.setProps --params '{"opacity":0.6,"blend":"Screen"}' \
  --cmd select.deselect --out out/f13_check_16bit.psb < /dev/null
"$PC" convert out/f13_check_16bit.psb out/f13_check_16bit_merged.png < /dev/null
"$PC" run out/f13_check_16bit.psb --cmd file.export.layersToFiles --params "{\"dir\":\"$OUT/f13_check_16bit_layers\",\"format\":\"png\",\"visibleOnly\":false}" < /dev/null
echo done
