#!/bin/sh
# Makes the STICKER_PLAN 8.3 PSD fixtures with PhotoCraft v0.2.0's console exe (headless, no window).
# Usage: sh make_fixtures.sh   (from a scratch folder that holds f1_wte_in.psd, made by gen_f1; writes ./out)
set -e
PC=/g/Self-made_products/photocraft-0.2.0-windows-x86-portable/photocraft-cli.exe
# Absolute Windows-style path of ./out (the per-layer export needs an absolute folder; it must be the same ./out).
OUT="$(pwd -W 2>/dev/null || pwd)/out"
rm -rf out && mkdir -p out
NEW8='{"width":64,"height":48,"background":"transparent","name":"fx"}'
NEW16='{"width":64,"height":48,"background":"transparent","depth":16,"name":"fx"}'
NEWW='{"width":64,"height":48,"background":"white","name":"fx"}'

# expected outputs: merged PNG (PhotoCraft's own composite) and per-layer PNGs
expect() {
  "$PC" convert "out/$1" "out/${1%.*}_merged.png" < /dev/null
  "$PC" run "out/$1" --cmd file.export.layersToFiles --params "{\"dir\":\"$OUT/${1%.*}_layers\",\"format\":\"png\",\"visibleOnly\":false}" < /dev/null
}

# f1: WTE-written three-layer sticker, opened and re-saved by PhotoCraft
cp f1_wte_in.psd out/f1_wte_in.psd
"$PC" convert out/f1_wte_in.psd out/f1_pc.psd < /dev/null
expect f1_pc.psd

# f2: soft-edged transparency (feathered disc)
"$PC" run --new "$NEW8" \
  --cmd select.rect --params '{"x":12,"y":4,"width":40,"height":40,"ellipse":true,"feather":6}' \
  --cmd edit.fill --params '{"color":"#3080e0"}' --cmd select.deselect \
  --out out/f2_feathered.psd < /dev/null
expect f2_feathered.psd

# f3: fully opaque picture
"$PC" run --new "$NEWW" \
  --cmd select.rect --params '{"x":4,"y":4,"width":30,"height":20}' --cmd edit.fill --params '{"color":"#d02020"}' \
  --cmd select.rect --params '{"x":24,"y":14,"width":34,"height":30,"ellipse":true}' --cmd edit.fill --params '{"color":"#20a040"}' \
  --cmd select.deselect --out out/f3_opaque.psd < /dev/null
expect f3_opaque.psd

# f4: a layer with a soft mask and a live Stroke effect; f4b the same without the effect
F4='--cmd select.rect --params {"x":8,"y":6,"width":48,"height":36} --cmd edit.fill --params {"color":"#30a060"}
 --cmd select.rect --params {"x":14,"y":10,"width":36,"height":28,"ellipse":true,"feather":2}
 --cmd layer.layerMask.revealSelection --cmd select.deselect'
# shellcheck disable=SC2086
"$PC" run --new "$NEW8" $F4 --cmd layer.layerStyle.stroke --params '{"size":3,"position":"outside","color":"#ff0000"}' \
  --out out/f4_mask_stroke.psd < /dev/null
expect f4_mask_stroke.psd
# shellcheck disable=SC2086
"$PC" run --new "$NEW8" $F4 --out out/f4b_mask.psd < /dev/null
expect f4b_mask.psd

# f5: f1 plus a type layer added in PhotoCraft
"$PC" run out/f1_pc.psd --cmd type.create --params '{"x":3,"y":44,"text":"PC","size":14,"color":"#102080","name":"pc text"}' \
  --out out/f5_f1_plus_text.psd < /dev/null
expect f5_f1_plus_text.psd

# f6: a layer group (two pixel layers grouped)
"$PC" run --new "$NEW8" \
  --cmd select.rect --params '{"x":4,"y":4,"width":30,"height":30}' --cmd edit.fill --params '{"color":"#c03030"}' \
  --cmd layer.new.layer --params '{"name":"upper"}' \
  --cmd select.rect --params '{"x":24,"y":14,"width":30,"height":30}' --cmd edit.fill --params '{"color":"#3030c0"}' \
  --cmd select.deselect --cmd layer.select --params '{"layer":2,"mode":"add"}' \
  --cmd layer.groupLayers --params '{"name":"group"}' \
  --out out/f6_group.psd < /dev/null
expect f6_group.psd

# f7: 16-bit document with soft transparency
"$PC" run --new "$NEW16" \
  --cmd select.rect --params '{"x":12,"y":4,"width":40,"height":40,"ellipse":true,"feather":6}' \
  --cmd edit.fill --params '{"color":"#e06020"}' --cmd select.deselect \
  --out out/f7_16bit.psd < /dev/null
expect f7_16bit.psd

# f8: hidden layer, 50 % opacity, Multiply; f8b: Color Dodge
"$PC" run --new "$NEW8" \
  --cmd select.rect --params '{"x":2,"y":2,"width":60,"height":44}' --cmd edit.fill --params '{"color":"#e0c040"}' \
  --cmd layer.new.layer --params '{"name":"half"}' \
  --cmd select.rect --params '{"x":6,"y":6,"width":26,"height":36}' --cmd edit.fill --params '{"color":"#2040c0"}' \
  --cmd layer.setProps --params '{"opacity":0.5}' \
  --cmd layer.new.layer --params '{"name":"multiply"}' \
  --cmd select.rect --params '{"x":20,"y":10,"width":30,"height":30,"ellipse":true}' --cmd edit.fill --params '{"color":"#c02020"}' \
  --cmd layer.setProps --params '{"blend":"Multiply"}' \
  --cmd layer.new.layer --params '{"name":"hidden"}' \
  --cmd select.rect --params '{"x":40,"y":4,"width":20,"height":20}' --cmd edit.fill --params '{"color":"#20c040"}' \
  --cmd layer.setProps --params '{"visible":false}' \
  --cmd select.deselect --out out/f8_hidden_opacity_multiply.psd < /dev/null
expect f8_hidden_opacity_multiply.psd
"$PC" run --new "$NEW8" \
  --cmd select.rect --params '{"x":2,"y":2,"width":60,"height":44}' --cmd edit.fill --params '{"color":"#806040"}' \
  --cmd layer.new.layer --params '{"name":"dodge"}' \
  --cmd select.rect --params '{"x":16,"y":8,"width":32,"height":32,"ellipse":true}' --cmd edit.fill --params '{"color":"#4080c0"}' \
  --cmd layer.setProps --params '{"blend":"Color Dodge"}' \
  --cmd select.deselect --out out/f8b_color_dodge.psd < /dev/null
expect f8b_color_dodge.psd

# f9: transparency plus a saved alpha channel
"$PC" run --new "$NEW8" \
  --cmd select.rect --params '{"x":12,"y":4,"width":40,"height":40,"ellipse":true,"feather":4}' \
  --cmd edit.fill --params '{"color":"#a040c0"}' \
  --cmd select.rect --params '{"x":0,"y":0,"width":32,"height":24}' \
  --cmd select.saveSelection --params '{"name":"Alpha 1"}' --cmd select.deselect \
  --out out/f9_alpha_channel.psd < /dev/null
expect f9_alpha_channel.psd

# f10: PSB of f1
"$PC" convert out/f1_pc.psd out/f10_f1.psb < /dev/null
expect f10_f1.psb

# f11: a layer clipped to the layer below
"$PC" run --new "$NEW8" \
  --cmd select.rect --params '{"x":8,"y":4,"width":40,"height":40,"ellipse":true}' --cmd edit.fill --params '{"color":"#3060c0"}' \
  --cmd layer.new.layer --params '{"name":"clipped"}' \
  --cmd select.rect --params '{"x":0,"y":20,"width":64,"height":12}' --cmd edit.fill --params '{"color":"#f0e020"}' \
  --cmd select.deselect --cmd layer.createClippingMask \
  --out out/f11_clipped.psd < /dev/null
expect f11_clipped.psd
# f12: f1 with the WTE layer "text" (lyid 2) deleted and a new layer added in PhotoCraft: PhotoCraft gives the new
# layer the lowest free id, 2, the deleted layer's id (not an id above the largest). Runtime layer 3 = "text" here.
"$PC" run out/f1_pc.psd --cmd layer.select --params '{"layer":3}' --cmd layer.delete \
  --cmd layer.new.layer --params '{"name":"pc new"}' \
  --cmd select.rect --params '{"x":2,"y":30,"width":20,"height":14}' --cmd edit.fill --params '{"color":"#20c0c0"}' \
  --cmd select.deselect --out out/f12_lyid_reuse.psd < /dev/null
expect f12_lyid_reuse.psd
echo done
