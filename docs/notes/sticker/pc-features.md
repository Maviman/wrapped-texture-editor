# PhotoCraft editing features, judged for WTE "sticker editing"

Date: 2026-11-25. Author: research agent (read-only study). Topic: which PhotoCraft (PC) editing features matter for
editing a *sticker* in WTE (a smallish RGBA image with transparency that WTE projects as a stamp onto a model texture),
where they live, how they work, and what reusing PC code would cost.

Sources:
- `G:\Claude_Resources\ref\photocraft` = PC git clone, HEAD b5c762017 (2026-11-25, workspace version 0.23.1; tags
  v0.2.0 = 2026-10-05 and v0.23.1 = 2026-11-11). **16,416 commits between v0.2.0 and HEAD.**
- v0.2.0 tree extracted with `git archive v0.2.0` to `G:\Claude_Resources\ref\scratch\pc020\` (paths below that start
  with `pc020\` are v0.2.0; paths that start with `HEAD:` are the latest clone).
- The user's install: `G:\Self-made_products\photocraft-0.2.0-windows-x86-portable\` (photocraft.exe GUI, not started;
  photocraft-cli.exe run headless for the checks marked RAN). CLI dumps in `G:\Claude_Resources\ref\scratch\pcfeat\`
  (`commands.txt` 748 commands, `commands.json` with parameter schemas; test images `in.png`, `out_sticker.png`,
  `out_subject.psd`).
- WTE: `G:\software_projects\texture_wrapper_ap_cmp\ARCHITECTURE.md` section 1 and the stamp/text API (lines cited).

Marks: **VERIFIED** = read in the code or ran; **INFERRED** = reasoning from what was read, not checked directly.

Note: running `git` commands in the PC clone triggered git's automatic repack once ("Auto packing the repository"). That
only re-packs objects; no tracked file changed. Later commands used `git -c gc.auto=0`.

---

## 0. Summary for the decision

1. PC v0.2.0 already has almost everything a sticker editor needs (VERIFIED, command registry of the installed exe):
   layer styles (Stroke, Drop/Inner Shadow, Outer/Inner Glow, Bevel & Emboss, Satin, Color/Gradient/Pattern Overlay),
   Magic Wand, Quick Selection, Object Selection, Select Subject, Select and Mask (refine edge), Magic Eraser,
   Background Eraser, Defringe / Remove White/Black Matte / Color Decontaminate, Trim, Crop, Free Transform, Warp,
   Puppet Warp, Liquify, 16+ adjustments, 27 blend modes, ~120 filters, type, shapes, masks, full history.
2. Latest adds sticker-relevant items not in v0.2.0: **Remove Background** quick action (Select Subject + light refine
   -> layer mask, one step; `HEAD:crates/engine-core/src/cmd/cutout_cmds.rs`), **Color to Alpha**
   (`HEAD:crates/algo/src/color_to_alpha.rs`), a better Select Subject (a second GrabCut pass and hole rules,
   `HEAD:crates/algo/src/segment/subject.rs:1-34`), and many fidelity fixes (VERIFIED by reading).
3. Background removal in PC is **classical, not AI** (SLIC superpixels + saliency + GrabCut; VERIFIED
   `pc020\crates\algo\src\segment\mod.rs:4-20`, `subject.rs:1-20`). So it is compatible with WTE's R9 "no AI" rule. It
   works for a distinct object on a plain background and is not a semantic segmenter (PC's own words, same file).
   Latest has one learned model elsewhere (face detection CNN weights, `HEAD:crates/algo/data/libfacedetection.bin`,
   BSD-3) that a sticker port would not need.
4. A full headless sticker pipeline runs on the installed v0.2.0 CLI today (RAN, section 9): magic-erase the white
   background, add an outside white stroke and a drop shadow, trim to the transparent bounds, write a PNG. This is a
   no-code way to prototype the feature set and an interop path (photocraft-cli is a separate program, not a crate).
5. Reuse as crates: v0.2.0's engine crates are self-contained (path deps only; external deps serde, serde_json, rayon,
   all already in WTE's Cargo.lock). The latest crates are **not** usable offline: they depend on `storytold/craft-libs`
   at a git rev (not cloned here) and patch egui/eframe/winit to PC's own forks (`HEAD:Cargo.toml` `[patch.crates-io]`).
   Neither version is on crates.io as far as the manifests show (INFERRED: no `publish` key, path/git deps only).
   Reuse therefore means vendoring source (MIT notice) or a clean reimplementation. Section 11.
6. Recommendation (section 12): reimplement a small, WTE-native sticker editor (alpha-based layer styles + wand/eraser
   + trim/defringe + a few adjustments), using PC's code as an algorithm reference with an MIT notice where code is
   ported; keep the photocraft-cli round trip as the "advanced editing" escape hatch.

---

## 1. What a "sticker" needs, and what WTE already has

WTE stamp model (VERIFIED, `texture_wrapper_ap_cmp\ARCHITECTURE.md:1485-1491`): `StampObject { source, placement,
tint: [u8; 4] /* multiplies the source */, home_set }`; layer blend modes are only Normal, Multiply, Screen, Overlay,
Add, Darken, Lighten (`:1371`). Text stamps already have outline (EDT-based, `text/edt.rs`, `:368`), faux bold, slant,
15 Photoshop-style warp styles and text on a path (`:1611-1642`). Non-goals of v1 include "colour-tolerance flood fill,
selections" (`:65`, also D18 `:60`) and "no AI" (`:37`). Every layer is an RGBA8 CPU buffer (`:20-23`).

So for an *image* sticker WTE today can only: import, tint, place/scale/rotate/project/wrap. Everything below is new.

Sticker editing jobs, in the order a user meets them:
1. Cut it out: remove a plain background (wand / magic eraser / color to alpha / select subject), clean the fringe.
2. Frame it: trim to content, crop, pad.
3. Style it: die-cut white border (outside stroke), drop shadow, glow, bevel, color/gradient overlay.
4. Fix colours: hue/sat, brightness/contrast, levels/curves, invert, posterize, desaturate.
5. Touch up: brush, eraser, fill, simple shapes, text on the sticker.
6. Shape it: flip, rotate, warp (WTE's projection already handles placement scale/rotation).
7. Undo everything; re-edit later (non-destructive styles are the big win).

---

## 2. Layer styles (effects) — value for stickers: HIGH

Where (VERIFIED):
- Data model: `pc020\crates\doc\src\effects.rs` (341 lines non-test; `enum Effect` at :244 = DropShadow, InnerShadow,
  OuterGlow, InnerGlow, Stroke, ColorOverlay, GradientOverlay, PatternOverlay, Satin, BevelEmboss; structs `Shadow`
  :92, `Glow` :123, `StrokeFx` :147, `Satin` :155, `Bevel` :185, `GlobalLight` :332, `Contour` :57).
  Latest: `HEAD:crates\doc\src\effects.rs` (422 lines).
- CPU renderer: `pc020\crates\compose\src\effects.rs` (1,511 lines). Latest split into `HEAD:crates\compose\src\effects.rs`
  + `effects\{bevel,blur,composite,distance,paint}.rs` (3,248 lines non-test, +~2,600 lines of tests).
- GPU renderer: `pc020\crates\gpu\` (WGSL; same maps) — not needed for stickers (INFERRED: sticker sizes <= 2k px are
  fine on the CPU).
- UI: `pc020\crates\ui-egui\src\layer_style.rs` (556 lines; tied to PC's engine/session).
- Commands (VERIFIED, CLI registry): `layer.layerStyle.{dropShadow, innerShadow, outerGlow, innerGlow, stroke,
  colorOverlay, gradientOverlay, patternOverlay, bevelEmboss, satin, clear, copyLayerStyle, pasteLayerStyle,
  hideAllEffects, showAllEffects, blendingOptions, globalLight, createLayer, scaleEffects}`, `style.presets.*`.

Algorithms (VERIFIED, `pc020\crates\compose\src\effects.rs`), all driven by the layer's **alpha** only — exactly what a
sticker has:
- Render order (:1-14): drop shadows, outer glows, outer bevel, then the layer (content at fill opacity) with
  pattern/gradient/colour overlays, satin, inner glows, inner shadows, strokes and inner bevel. Exterior effects blend
  onto the backdrop with their own blend mode; layer opacity scales the whole stack, fill opacity only the pixels.
- Distance fields: exact Euclidean distance transform (Felzenszwalb & Huttenlocher 1D passes, `dt1` :104, `edt` :147,
  `edt_nearest` :152). Strokes use a 5x5 chamfer metric seeded at sub-pixel edge offsets (`chamfer_nearest` :186,
  `chamfer_from` :195), because Photoshop's stroke corners match that metric (comment :181-185). Edge position inside a
  partly covered pixel is placed by its alpha (`dist_outside` :234-238).
- Soft falloff = two box passes of width `size` (a tent), running-sum O(1) per pixel (`blur` :320-328, `tent_kernel`
  :610-623, `box_line` :634-652). Spread/choke: whole-pixel dilation by `round(size*spread)`, blur width
  `size*(1-spread)` (`spread_split` :489-495).
- Drop/inner shadow (`shadow_map` :496-511): offset by angle/distance (rounded to whole px, :407-412), dilate, tent
  blur, contour LUT; inner = same on `1 - alpha`, then multiplied by alpha.
- Outer/inner glow (`glow_map` :513-556): Softer = dilate + tent blur; Precise = linear ramp of the exact distance
  (`solid = size*spread`, then `1 - (d-solid)/(size-solid)`); contour applied over Range (`glow_lut` :362).
- Stroke (:1113-1210): inside part = `clamp(in_w + 0.5 - d_inside, 0, 1)` painted over the layer; outside part slid
  beneath the layer onto the backdrop; several strokes stack top over bottom with per-stroke blend modes.
  Position Outside/Inside/Center = (0,size)/(size,0)/(size/2,size/2) (:1120-1124). Fill can be colour, gradient or
  pattern (`FxPaint`, `paint_fx` :577).
- Bevel & Emboss (`bevel_maps` :881-921): height map = tent blur of alpha (smooth) or ramps of the exact distance
  (chisel, `bevel_chisel_h`); normal from central differences scaled by depth; Lambert shade against the light vector
  from angle/altitude; `shade - sin(altitude)` > 0 -> highlight map, < 0 -> shadow map; gloss contour; painted with the
  highlight/shadow colours and blend modes (default Screen/Multiply).
- Colour overlay keeps alpha and replaces colour (:309-312); gradient overlay uses `gradient_t` (:416) and
  `sample_gradient` (:448) over the layer bounds.
- Maps depend only on the shape and settings, never the backdrop, so they are cached per layer state (`build_maps`
  :988-1015, `FxMaps` :164-186).

Sticker value: the die-cut white border (Stroke Outside) plus Drop Shadow are *the* sticker look; Outer Glow, Color
Overlay (single-colour silhouette), Gradient Overlay and a simple Bevel are next. Satin, Pattern Overlay, Inner Shadow,
Contours, Noise, Knockout: low.

Clean reimplementation size (INFERRED): Stroke (outside/inside/center, colour fill) + Drop Shadow + Outer Glow (softer)
+ Color Overlay on RGBA8 with an EDT ~600-900 lines incl. tests. WTE already has an exact EDT (`text/edt.rs`) used for
text outlines, so the stroke is a generalisation of existing code. Bevel adds ~250 lines.

Interaction with projection (INFERRED): effects enlarge the sticker (shadow/glow/stroke margin, PC's `margin()` :37-56),
so the baked sticker image must be padded by that margin before WTE projects it, or the effect is cut at the image edge.

---

## 3. Background removal and selections — value: HIGH (wand/eraser/subject), MEDIUM (refine), LOW (lassos)

Where (VERIFIED):
- `pc020\crates\algo\src\selection.rs` (890 lines; latest split into `HEAD:crates\algo\src\selection\` 2,477 lines):
  `magic_wand` :86 (per-channel difference <= tolerance incl. alpha; contiguous = 4-connected flood; optional AA),
  `wand_region` :174 (scanline flood on 8-bit, byte marks, AA only on edges in the touched box), `color_range` :360,
  `color_range_samples` :386, `hue_range` :430, `tone_range` :447, `expand` :530 / `contract` :539 / `border` :545 /
  `smooth` :580 / `feather` :585 (Gaussian, sigma = radius/2) — all via EDT; `polygon` :625 (AA lasso).
- `pc020\crates\algo\src\segment\` (2,428 lines non-test): `maxflow.rs` (Boykov–Kolmogorov), `gmm.rs` (full-covariance
  GMM, k-means++ + EM), `slic.rs` (SLIC superpixels), `grabcut.rs` (GrabCut; `object_select` :77 = rectangle Object
  Selection), `quick.rs` (`quick_select` :41: stroke seeds -> fg GMM, geodesic distance over a colour-gradient barrier
  picks bg samples, contrast-sensitive min cut, keep components touching the stroke), `subject.rs` (`select_subject`
  :37: SLIC -> Zhu et al. 2014 boundary-connectivity saliency + Achanta 2009 frequency-tuned saliency -> Otsu ->
  GrabCut trimap -> island/hole cleanup -> full-res re-cut in a narrow band), `sky.rs`, `focus.rs`.
  Latest: 3,968 lines; `subject.rs` adds a second GrabCut with a backdrop-only model and colour-aware hole filling
  (`HEAD:crates\algo\src\segment\subject.rs:18-30`).
- `pc020\crates\algo\src\matting.rs` (746 lines): Select and Mask: guided filter (He et al.) on the binary mask with
  the image as guide inside a band (`guided_filter_color` :141, `refine_buffer` :316, `refine_mask` :373), "smart
  radius" from `edge_width` :297, then Smooth, Feather, Contrast, Shift Edge (`morph` :222), and **Decontaminate
  colours** (normalised-convolution average of nearby fully selected pixels). Latest `matting\` 2,239 lines.
- `pc020\crates\algo\src\erase.rs` (164 lines): Magic Eraser = wand region -> alpha (or toward a colour when
  transparency is locked). `pc020\crates\paint\src\bg_erase.rs` (270 lines): Background Eraser brush (erase colours
  near the hotspot sample; "protect foreground colour").
- Matting cleanup (VERIFIED, `pc020\crates\engine\src\layer_menu_cmds.rs:225-272`): `defringe` (edge pixels within
  `width` of transparency take the colour of nearby solid pixels), Remove Black/White Matte, Color Decontaminate
  (commands `layer.matting.*`).
- Latest only: `layer.removeBackground` (`HEAD:crates\engine-core\src\cmd\cutout_cmds.rs:1-20`: Select Subject with a
  narrow smart radius and a little smoothing, applied as a *layer mask*, one history step, non-destructive) and Color to
  Alpha (`HEAD:crates\algo\src\color_to_alpha.rs:1-30`, 236 lines: per channel `a_i = (P_i - C_i)/(1 - C_i)` if
  `P_i > C_i`, `(C_i - P_i)/C_i` if `P_i < C_i`, alpha = max over channels, then un-mix the colour; GIMP-style).
  Neither is in the v0.2.0 command registry (VERIFIED: `commands.txt` has no `removeBackground`/`colorToAlpha`).

Sticker value and reimplementation notes:
- Magic Eraser / wand with tolerance + contiguous + AA: HIGH, small (~250 lines with flood fill + AA edge pass).
  Note: WTE explicitly listed colour-tolerance flood fill as a v1 non-goal for the *texture* fill tool; on the sticker
  image it is a new, separate feature (INFERRED: needs the user's OK as a scope change).
- Color to Alpha: HIGH for logos on white/black (keeps anti-aliased edges and soft shadows), tiny (~60 lines core).
- Defringe / Remove White Matte: HIGH after any cut-out (white halos show badly on a dark model), small (~80 lines).
  Remove White Matte = un-premultiply against white: `C = (P - (1 - a)) / a` (INFERRED from the Photoshop definition;
  PC code not read line by line).
- Select Subject / Remove Background: MEDIUM-HIGH value but large (~2,400 lines v0.2.0: maxflow + GMM + SLIC + GrabCut
  + saliency) and heuristic. A cheaper middle ground is GrabCut from a user rectangle (Object Selection): maxflow +
  GMM + grabcut ~900 lines (INFERRED from file sizes above).
- Refine edge (guided filter): MEDIUM, ~300 lines for the core guided filter + feather.
- Marquee/lasso/selection modify: LOW for stickers (a sticker is the whole image; trim/crop covers framing).

---

## 4. Crop / trim / canvas — value: HIGH (trim, pad), MEDIUM (crop)

Where (VERIFIED): `pc020\crates\engine\src\image_cmds.rs:271` (`trim`, based on transparent / top-left / bottom-right
colour, per side), `image.crop`, `image.canvasSize`; commands `image.trim`, `image.crop`, `image.canvasSize`.
Algorithm: bounding box of pixels whose alpha > 0 (or that differ from the corner colour), then crop. Trivial (~50 lines).
Sticker use: trim after a cut-out, then *pad* by the effect margin (section 2) so shadows/strokes are not clipped.

---

## 5. Transform / warp — value: MEDIUM (flip/rotate 90), LOW-MEDIUM (warp), LOW (puppet, liquify, perspective)

Where (VERIFIED): `pc020\crates\algo\src\transform.rs` (356 lines: Free Transform as a quad -> homography; inverse
mapping with premultiplied nearest/bilinear/Catmull-Rom sampling; pre-filtered resize for big reductions),
`warp.rs` (357: mesh warp through a forward map, grid cells rasterised as triangles), `puppet.rs` (904: ARAP on a
grid-clipped triangle mesh of the opaque region), `liquify.rs` (843: displacement field edited by replayable strokes),
`perspective.rs` (322), `resample.rs` (289). Latest: transform 631, warp 1,279, puppet 1,515, liquify 1,209.
Text warp styles: `pc020\crates\text\src\warp.rs` (119) using `photocraft_geom::warp` (15 styles).

Sticker value (INFERRED): WTE's placement already scales/rotates/projects/wraps the sticker, so Free Transform is mostly
redundant. Flip H/V and rotate 90 on the source image are cheap and useful. WTE already implements the same 15 warp
styles for text (`ARCHITECTURE.md:1638-1640`); applying them to image stickers (warp the image through WTE's existing
style math) is a cheap "Warp" for stickers. Puppet/Liquify/Perspective: later.

---

## 6. Adjustments — value: MEDIUM (hue/sat, brightness/contrast, invert, desaturate), LOW-MEDIUM (levels, curves)

Where (VERIFIED): data `pc020\crates\doc\src\adjust.rs` (296; `enum Adjustment` :124 = BrightnessContrast, Levels,
Curves, Exposure, Vibrance, HueSaturation, ColorBalance, BlackWhite, PhotoFilter, ChannelMixer, ColorLookup, Invert,
Posterize, Threshold, GradientMap, SelectiveColor); evaluation `pc020\crates\compose\src\adjust.rs` (733 lines;
latest `adjust.rs` + `adjust\` 1,885). Destructive extras `pc020\crates\algo\src\tone.rs` (445: Shadows/Highlights,
Replace Color, Match Color, HDR Toning). Commands `image.adjustments.*` (23, VERIFIED registry).

Algorithm notes (VERIFIED, compose/adjust.rs):
- Curves: natural cubic spline through the points, constant outside, clamped, sampled to a LUT (`curve_lut` :486).
- Levels per channel then master (`tone_luts` :205, `levels` :460).
- Hue/Saturation: hue shift degrees, sat/light -1..1, Colorize sets hue+sat (`hue_saturation` :268), per-range tables
  (`hue_range_tables` :254).
- "Modern" Brightness: line of slope `1.375^(b/50)` rolled off to (1,1); Contrast: symmetric cubic-Hermite S-curve
  about 0.5 with end slope `1 - c/128` and centre slope `1 + c/128` (:424-452).
- Posterize: n bins, levels `floor(k*255/(n-1))` (:422-458). Black & White weights (:286-310).
All are per-pixel on straight RGBA; reimplementing Hue/Sat + Brightness/Contrast + Invert + Desaturate + Posterize is
~200 lines. Sticker value: recolouring a sticker to match a model; WTE's `tint` already covers "multiply by a colour".

---

## 7. Filters — value: LOW-MEDIUM (blur, sharpen, pixelate, noise), LOW (the rest)

Where (VERIFIED): `pc020\crates\algo\src\lib.rs` (`FilterParams` :176, `apply` :852 tile-parallel with halo),
blur.rs/blur2.rs, sharpen.rs, noise.rs, pixelate.rs, distort*.rs, stylize*.rs, render*.rs, artistic_fx.rs (Filter
Gallery), oil.rs, lens.rs; ~120 `filter.*` commands in v0.2.0 (VERIFIED registry). The filter API takes PC's tiled
`photocraft_raster::Surface` (`pc020\crates\raster\src\lib.rs:42`), not a plain slice.
Sticker value: a slight Gaussian blur, Unsharp Mask, Pixelate/Mosaic (retro stickers), Add Noise, Posterize-like
Cutout (Filter Gallery) are nice-to-have. Most filters (lens, camera raw, panorama, HDR, vanishing point) are photo
tools with no sticker value.

---

## 8. Painting, type, shapes, masks, blend modes, history

- Brush engine `pc020\crates\paint\` (4,247 lines non-test; latest 9,523): computed/sampled tips, Shape Dynamics,
  Scattering, Texture, Dual Brush, Color Dynamics, Transfer, Wet Edges, Build-up, Smoothing; deterministic strokes
  (`paint\src\lib.rs:1-8`). Value for stickers: LOW-MEDIUM. WTE has its own brush/eraser (with pen pressure since the
  pen-input branch); a 2D sticker canvas can reuse WTE's brush on a plain RGBA buffer (INFERRED).
- Type `pc020\crates\text\` (3,433 lines; latest 14,784): parley shaping, point/paragraph text, warp, PSD text.
  Value: LOW — WTE already has text stamps with outline, warp, path; "text on an image sticker" could be done by
  compositing a WTE text raster into the sticker (WTE lists per-letter rotation and text on an image stamp as not
  built, `ARCHITECTURE.md:87`). Adds the `parley` crate (not in WTE) if reused.
- Shapes `pc020\crates\vector\` (1,933 lines; latest 5,543): rect/rounded rect/ellipse/polygon-star/line
  (`shapes.rs:15-122`), exact-area scanline rasteriser, polygon stroking with dashes (`stroke.rs`), mask -> Bezier
  tracing (`trace.rs`, marching squares + Schneider curve fit). Value: MEDIUM-LOW (badge backgrounds behind a
  sticker; a rounded-rect/circle backing plate is the useful 10 %).
- Masks: pixel mask + vector mask per layer, density, `layer.layerMask.*`, Reveal/Hide Selection, Apply, From
  Transparency (VERIFIED registry). Value: MEDIUM — a non-destructive mask makes "remove background" undoable/
  re-editable; in WTE terms an optional alpha mask on the sticker source (INFERRED).
- Blend modes: 27 (`pc020\crates\color\src\blend.rs:13-40`), Photoshop-exact variants in
  `pc020\crates\compose\src\psblend.rs` (236). Value: LOW for stickers (WTE has 7 layer modes; effects need only
  Normal/Multiply/Screen).
- History: whole-document snapshots over `Arc`-shared copy-on-write tiles, bounded by count and bytes
  (`pc020\crates\ops\src\lib.rs:1-10`; History Brush, Toggle Last State in the engine). WTE already has `Pixels`
  (64x64 tile diffs) + `Structure` snapshots (`ARCHITECTURE.md` D6). Value: ESSENTIAL as a requirement, nothing to
  reuse — sticker edits should become WTE `Structure` edits on the stamp (INFERRED).
- UI: `pc020\crates\ui-egui\src\state.rs:7-50` lists 42 tools (README says "34 tools"). The UI crate is egui 0.36 in
  v0.2.0 / a patched egui 0.37 fork in latest and is bound to PC's Session — not reusable; useful only as UX reference.

---

## 9. Real v0.2.0 behaviour (RAN, photocraft-cli.exe, headless, no window)

Scratch: `G:\Claude_Resources\ref\scratch\pcfeat\`. Input `in.png` 200x160 white background, red ellipse, blue bar.

```
photocraft-cli run in.png --cmd paint.magicEraser --params '{"x":2,"y":2,"tolerance":32}'
  --cmd layer.layerStyle.stroke --params '{"size":6,"position":"outside","color":"#ffffff"}'
  --cmd layer.layerStyle.dropShadow --params '{"distance":6,"size":8,"color":"#000000"}'
  --cmd image.trim --params '{"basedOn":"transparent"}' --out out_sticker.png
```
Output (VERIFIED): each command printed a JSON result; trim -> `{"x":33,"y":23,"width":140,"height":122}`; warning
"1 layer(s) flattened; layers, masks and blend modes are not kept" (PNG export flattens and bakes the effects).
`out_sticker.png` is RGBA 140x122, transparent corners, an anti-aliased white border (alpha 42 at the outer pixel, then
255) and the shadow. So: background removal + die-cut border + shadow + trim works end-to-end in v0.2.0 headless.

```
photocraft-cli run in.png --cmd select.subject --cmd layer.layerMask.revealSelection
  --cmd layer.layerStyle.stroke --params '{"size":4,"position":"outside","color":"#ffffff"}' --out out_subject.psd
```
Output (VERIFIED): Select Subject found the ellipse (`bounds [39,29,123,103]`), the PSD keeps the layer, its mask and a
*live* Stroke effect (`photocraft-cli info out_subject.psd`: `"hasMask":true, "effects":{"items":[{"kind":"Stroke"}]}`).

Parameter schemas of every command are in `commands.json` (e.g. dropShadow: color, opacity 75, blend multiply, angle
120, useGlobalLight, distance 5, spread, size 5, knocksOut).

Interop note (INFERRED, for the interop topic): photocraft-cli `run`/`batch`/`serve` (JSON lines on stdio or
127.0.0.1) and the GUI's `--control` channel are a ready-made, process-level way for WTE to hand a sticker to PC and get
a PNG/PSD back without linking any PC crate. WTE already writes PSD/ORA exports; PC reads and writes PSD with layers.

---

## 10. Sizes at a glance (non-test Rust lines; VERIFIED by `wc -l`, tests excluded by file name)

| Feature group | v0.2.0 location | v0.2.0 | latest |
|---|---|---|---|
| Layer styles (renderer) | compose/src/effects.rs | 1,511 | 3,248 |
| Layer styles (data) | doc/src/effects.rs | 341 | 422 |
| Selection basics (wand, range, modify, lasso) | algo/src/selection.rs | 890 | 2,477 |
| Smart selection (subject, quick, object) | algo/src/segment/ | 2,428 | 3,968 |
| Select and Mask / decontaminate | algo/src/matting.rs | 746 | 2,239 |
| Magic Eraser | algo/src/erase.rs | 164 | 175 |
| Color to Alpha | — | absent | 236 |
| Adjustments (layer) | compose/src/adjust.rs | 733 | 1,885 |
| Adjustments (destructive extras) | algo/src/tone.rs | 445 | 1,551 |
| Free Transform / Warp / Puppet / Liquify | algo/src/{transform,warp,puppet,liquify}.rs | 356/357/904/843 | 631/1,279/1,515/1,209 |
| Brush engine | paint/src | 4,247 | 9,523 |
| Type | text/src | 3,433 | 14,784 |
| Vector shapes | vector/src | 1,933 | 5,543 |
| Blend modes (PS-exact) | compose/src/psblend.rs | 236 | 539 |
| Whole crates | algo / compose / ui-egui / engine | 27,112 / 5,642 / 39,046 / 49,318 | 92,193 / 29,706 / 283,670 / 45,311 (+engine-core 190,819) |

Whole workspace: v0.2.0 ~190 k lines in 22 crates; latest ~900 k lines in 25 crates (VERIFIED, per-crate counts).

---

## 11. Licensing and reuse cost

License (VERIFIED): `MIT OR Apache-2.0` (`HEAD:Cargo.toml` `[workspace.package]`, same in v0.2.0), copyright
"ArtCraft Team and the PhotoCraft contributors" (`LICENSE-MIT`). `NOTICE` lists third-party assets (Inter and JetBrains
Mono fonts OFL, Lucide icons ISC, SCOWL word list, libfacedetection BSD-3 in latest, craft-i18n catalogs) and states
the **ArtCraft name/logos are trademarks** not covered by the license. WTE is MIT (`texture_wrapper_ap_cmp\Cargo.toml`
`license = "MIT"`), so it is compatible.

What reuse requires (INFERRED from the license texts, standard practice):
- Choose MIT (simplest): keep PC's copyright + MIT permission notice with any copied or ported code (e.g. a
  `THIRD_PARTY_NOTICES` / `licenses\photocraft-MIT.txt` shipped with the exe and in the installer, and a header comment
  in ported files). Apache-2.0 would add: carry the NOTICE file's relevant parts and state changes — avoid by choosing MIT.
- Do not use the ArtCraft/PhotoCraft names or logos in WTE's UI beyond factual attribution ("works with PhotoCraft").
- Bundled assets (fonts, icons) keep their own licenses; a sticker port needs none of them.
- A *clean reimplementation from the papers* the PC code cites (Felzenszwalb EDT, Boykov–Kolmogorov, GrabCut, SLIC,
  guided filter, Zhu saliency) needs no notice; reading PC's code and porting it is a derivative work -> MIT notice.
  Cheapest honest path: add the notice once and port freely.

Using PC crates as dependencies:
- v0.2.0 (VERIFIED manifests in `pc020\crates\*\Cargo.toml`): edition 2024, rust-version 1.90 (WTE rustc 1.99.1 OK),
  all internal deps are `path = ...`; no crates.io publication visible. Usable dependency chains and their external crates:
  - Selection + segmentation + matting + filters: `photocraft-algo` -> geom (serde) + color -> cms (rayon) + raster
    (none) ; algo itself serde + rayon. ~34.8 k lines (geom 991 + cms 4,906 + color 999 + raster 794 + algo 27,112).
    **No new external crate** for WTE (serde, rayon already in WTE's lock). API takes `photocraft_raster::Surface`
    (tiled COW, `raster\src\lib.rs:42`) for filters; but `selection::magic_wand`, `wand_region`, `color_range`,
    `feather`, `expand`... take plain `&[[f32;4]]` / `&[f32]` slices (`selection.rs:86, 174, 360, 530-585`), and the
    segmenters read through a `Sampler` trait — easy to adapt.
  - Layer styles: `photocraft-compose` needs geom, color, cms, raster, doc (+serde_json), vector (+rayon). ~19.6 k lines
    (+5,642 compose +4,029 doc +2,282 vector). Entry points `build_maps(&Layer, alpha, rect, &GlobalLight, &TextureCtx)`
    and `composite_with_effects(&Layer, &Buffer, &mut Buffer, &FxMaps, Rect, &[Pattern])` (`effects.rs:988, 1022`) take
    PC's whole `doc::Layer` (`doc\src\lib.rs:395`, with PSD blocks etc.) — usable but heavy coupling.
  - Text: adds `parley 0.11` and `skrifa 0.44` (WTE's Cargo.lock has skrifa 0.44.0, pulled by `glifo` (VERIFIED), but not parley). Not recommended.
  - GPU (`photocraft-gpu`): wgpu 30 — same major as WTE — but not needed.
  - Vendoring is the only offline option (copy into `crates\third_party\photocraft-*`); each vendored crate is still a
    "new crate" in WTE's sense and needs the user's OK (rule since round 4). Ongoing cost: no upstream fixes (v0.2.0 is
    16 k commits behind), PC's lints (`unsafe_code = "forbid"`, which WTE could live with) and test suites.
- Latest (VERIFIED `HEAD:Cargo.toml`, `HEAD:crates\*\Cargo.toml`): rust-version 1.95 (toolchain file pins 1.100.0 for
  PC's own CI; WTE's 1.99.1 satisfies 1.95 — INFERRED that sources really build on it). Even the leaf crates now
  depend on `craft-tracked`, `craft-ui-macros`, `craft` (algo: `features = ["imaging"]`, text: font-catalog),
  `craft-develop`, `craft-metadata` from `github.com/storytold/craft-libs` at a pinned git rev (not cloned here, so not
  buildable offline), plus new crates.io crates (`rustfft`, `lz4_flex`, `crc32fast`, `sysinfo =0.40.1`, `winsafe`,
  `base64 0.23`, `skrifa 0.47`...). The workspace `[patch.crates-io]` replaces egui/eframe/egui-wgpu/egui-winit/epaint
  and winit with PC forks; any crate that pulls `craft` would likely drag that fork into WTE (INFERRED; craft-libs not
  readable here). Lock size: latest 1,018 packages vs v0.2.0 620 vs WTE 456. **Latest is not a practical dependency.**
- No engine crate is usable "standalone" in the latest tree; in v0.2.0 `photocraft-algo` (with its 4 small foundation
  crates) is the closest to standalone, and `photocraft-compose` effects are usable only with the doc model.

Cost comparison (INFERRED):
| Option | New crates | Code WTE owns | Fit | Risk |
|---|---|---|---|---|
| A. Vendor v0.2.0 algo stack (+compose for styles) | 5-8 vendored path crates, no new external | ~35-55 k lines | RGBA f32 vs WTE RGBA8, doc::Layer coupling | stale fork, big review/test surface |
| B. Port selected algorithms (MIT notice) into `wte_core` | none | ~2-4 k lines | native RGBA8, WTE undo/project | fidelity gaps vs PS, own tests |
| C. Process interop with photocraft-cli / PC GUI | none | small adapter | PC does the editing | PC must be installed; v0.2.0 CLI behaviour fixed |

---

## 12. Proposed sticker-editing feature set (ranked)

Tier 1 — essential (small, high value; all reimplementable in `wte_core` on RGBA8, ~1.5-2.5 k lines incl. tests):
1. Sticker edit stack stored with the stamp (non-destructive, re-editable, saved in `.wtep`, undo as Structure edits):
   source image -> cut-out -> adjustments -> effects -> trim/pad -> baked sticker used by the existing projection.
2. Stroke effect (Outside/Inside/Center, colour, size, opacity) — the die-cut border. EDT already in WTE.
3. Drop Shadow (colour, opacity, angle, distance, spread, size) — tent blur of the offset alpha.
4. Outer Glow (softer) and Color Overlay (silhouette recolour).
5. Magic Eraser / wand (tolerance, contiguous, anti-alias) and Color to Alpha (logo on white/black).
6. Defringe / Remove White Matte / Remove Black Matte.
7. Trim to content + automatic padding for the effect margin; Flip H/V, rotate 90.
8. Hue/Saturation (+Colorize), Brightness/Contrast, Invert, Desaturate.
9. Eraser + brush on the flat sticker (reuse WTE's brush on the source buffer), with undo.

Tier 2 — valuable (medium size):
10. Object Selection (GrabCut from a rectangle: maxflow + GMM, ~900 lines) as "Remove background" for photos;
    optionally Select Subject (saliency + GrabCut, +~1,000 lines) — classical, satisfies "no AI".
11. Refine edge: feather / smooth / shift edge + guided filter + colour decontamination.
12. Gradient Overlay, Inner Shadow, simple Bevel & Emboss (smooth, inner).
13. Levels and Curves (LUT-based), Posterize, Threshold (sticker/print looks).
14. Non-destructive alpha mask on the sticker (paint to hide/reveal, "mask from selection").
15. Backing shapes: rounded rectangle / circle / star plate behind the sticker; text composited onto an image sticker
    using WTE's existing text renderer.
16. Image warp using WTE's existing 15 warp styles; effect presets ("Classic sticker", "Glow", "Embossed badge").

Tier 3 — later:
17. Quick Selection brush, magnetic/lasso selections, Select and Mask workspace.
18. Filters: Gaussian blur, Unsharp Mask, Pixelate/Mosaic, Add Noise, Cutout/Poster Edges (stylised stickers).
19. Pattern Overlay, Satin, contours, multiple strokes, blend modes beyond WTE's 7, global light.
20. Puppet Warp, Liquify, Perspective Warp; Healing/Clone; full brush dynamics.
21. Round-trip "Edit in PhotoCraft" via photocraft-cli/GUI (belongs to the interop study; covers everything above as a
    fallback until WTE has it natively).
