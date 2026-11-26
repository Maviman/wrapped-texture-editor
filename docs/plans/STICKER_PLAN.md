# STICKER_PLAN - layered stickers, a sticker editor, and working together with PhotoCraft

Branch `stickers` (worktree `G:\software_projects\texture_wrapper_ap_stk`, cut from `main` @ b2fb8f5 = v0.3.0). Planned 2026-11-25;
**revision 2** (same day) after two reviews: every finding and what was done about it is in section 12 "Review log".
This plan is the contract for builders who have not seen the planning conversation. Read first: `CLAUDE.md`, `HANDOFF.md`,
`ARCHITECTURE.md` section 1 and the sections named in section 10 of this plan, then all of `docs/notes/sticker/`
(`OPTIONS.md` is the decision document; where its "## Critic" section and an earlier part disagree, the critic wins).

Words: **sticker** = a small layered picture document; **stamp** = a placed decal on the model (`StampObject`); a stamp shows an
image, a text or a sticker. **Styles** = layer styles (stroke, die-cut contour, shadows, glows, overlays, bevel).
**Link** = a PSD file both apps use. **PC** = PhotoCraft. `pc020\` = `G:\Claude_Resources\ref\scratch\pc020\` (PhotoCraft v0.2.0
source), `PCHEAD:` = `G:\Claude_Resources\ref\photocraft` at HEAD (read with `GIT_NO_LAZY_FETCH=1 git -c gc.auto=0 show HEAD:<path>`).

---

## 0. Summary

Three things are built:

1. **Layered stickers in the core.** A sticker is a small document (`StickerDoc`): an artboard and a stack of picture, text and
   shape layers, each with opacity, blend mode, transform, optional mask, adjustments and styles. Stamps get a third source kind,
   `StampSource::Sticker`, and **every** stamp (image, text, sticker) gets a stamp-level `StyleStack` (die-cut contour, stroke,
   shadow, glow, overlay, bevel). Project format 4.
2. **A Sticker view in the main window** (mode strip: 3D | 2D | Split | Preview | **Sticker**): the sticker canvas on the left,
   the live 3D result on the right, sticker tools in the tool strip, the sticker's layers in the Layers tab, a new **Sticker**
   tab (tray, styles, presets, link). Works with mouse, pen and the tablet companion (input is ordinary egui input).
3. **Working with PhotoCraft**: "Edit in PhotoCraft" for stickers **and for texture paint layers** (a linked PSD in
   `<project>_links\`, Ctrl+S in PhotoCraft updates the sticker or layer in WTE), WTE's own PSD reader and writer, WTE's own
   Windows drop target (drop point, images without files), OS clipboard for pictures and text, and drag out of WTE (layer
   row, sticker thumbnail). All Windows code is hand-written `raw-dylib` FFI. No new crate.

Key design choices (each justified in its section):

| Topic | Choice |
|---|---|
| Sticker ownership | document-level `Document::stickers: BTreeMap<StickerId, Arc<StickerDoc>>`; a stamp refers to one by id; several stamps may share one (E6) |
| Old stamps | unchanged in memory and on disk until edited. An image stamp is shown in the editor as a virtual one-layer sticker and becomes a real one ("promoted") at the first content edit; promotion changes no pixel |
| Styles | one type, `StyleStack`, used per sticker layer and per stamp. Stamp styles never change the stored placement: the bake uses an expanded copy (`Document::bake_placement`), so the gizmo box stays on the artwork |
| Margin | styles need room: the picture handed to the bake is `content + symmetric margin`; the margin is known from the style numbers alone |
| Undo | the one document history. New edit kind `Edit::StickerPixels` (tile patch on a sticker layer or mask); everything else is a `Structure` edit whose snapshot now carries the sticker map (Arc clones) |
| Rendering | CPU only, deterministic, straight RGBA8, in `wte_core::sticker`; caches hold the `Arc` they were made from (compared with `Arc::ptr_eq`) or a monotonic picture version, never a bare address |
| Link | per sticker (and per paint layer, P12), PSD 8-bit RGB written at the sticker's render scale. PSDs come back layer by layer (matched by a stable PSD layer id; masks, clipping and PhotoCraft's own text / smart layers included); only things WTE cannot draw (effects, groups, adjustment layers, other blend modes) make the file come back as one merged picture |
| Link writes | WTE writes the PSD only on an explicit "Edit in PhotoCraft" / "Send to PhotoCraft", never at project save; a link needs a saved project |
| Link state | outside the history (R6): undoing an update does not make the watcher apply the file again |
| FFI | a new `ole/` module in `wte_app`; its own second window-procedure subclass (the pen subclass is not touched) |

### 0.1 The user's request and decisions (binding)

Request (2026-11-25): "integrate sticker editing just like this program [PhotoCraft] put into this prompt. There is plenty to
include for editing. I would also like to be able to have these two products work together, where i can drag and drop between the
apps and seamlessly link photos/layers together to edit in one and move to the other."

Decisions (2026-11-25):

1. "WTE gets its OWN sticker editor (modelled on PhotoCraft's tools, clean reimplementation, no new crates) PLUS an 'Edit in
   PhotoCraft' link for anything bigger (Ctrl+S in PhotoCraft updates the sticker in WTE). The in-app editor also works from the
   Android tablet companion."
2. "A sticker is a LAYERED STICKER: a small multi-layer document inside each sticker (picture layers + text + shapes, each with
   its own effects / layer styles), like a mini PhotoCraft file. Not baked. Project format version bump is accepted (older WTE
   versions cannot open new projects)."
3. "PhotoCraft: support BOTH the user's portable v0.2.0 and the latest (v0.23.x / HEAD); the user will update when convenient."
4. "Pen + companion are landed (v0.3.0); stickers start from main (this worktree)."

Defaults taken from `OPTIONS.md` (the user did not object): Sticker view in the main window with the 3D result beside it; link
format PSD in `<project>_links\`, also stored in the `.wtep`; WTE reads and writes PSD itself; hand-written FFI; `DoDragDrop`
only from a window-procedure subclass via a posted `WM_APP` with a re-entry guard (critic C4); WTE's own drop target before
drag-out (R3); drag out only from the layer row and the sticker thumbnail; `photocraft-cli` only as an optional fallback, always
with `CREATE_NO_WINDOW`; no PhotoCraft fork; critic items E1-E7, R6, R7, R8, R10 included.

Where this plan changes a default, it says so: 3.9 (Sticker is a fifth upper tab), 5.4 (styles are not written into the PSD),
5.3 (a link needs a saved project; the PSD is never rewritten at project save), 6.4 (a visible "PSD" drag handle hands out the
linked PSD).

### 0.2 Scope

In this delivery (packages P0-P13, section 7): everything in sections 2-6 not marked *later*, **including linked texture paint
layers** (P12; the user asked to "link photos/layers"). Stretch packages S1 and S2 are built after P13 if the earlier checks
are green; nothing depends on them.

Out of scope (later; the user sees this list in section 11 and can object): layer groups inside a sticker, Curves, colour
balance, filters other than blur and sharpen (pixelate, noise), quick-selection brush, refine-edge (guided filter), satin and
pattern overlay, several strokes per layer, per-style blend modes, writing styles into the PSD (`lfx2`), perspective / warp of
pictures, puppet / liquify, healing / clone, brush presets, a history panel, a `.pcraft` reader, a PhotoCraft fork, the
PhotoCraft control channel, drag out from the 3D view, UV-space (2D) and 3D stamps becoming cross-editable (HANDOFF section 6
stays).

Not possible at all, said plainly to the user (section 11, Help): **dragging from PhotoCraft into WTE**. PhotoCraft has no drag
source in either version. The substitutes are Ctrl+S on a linked sticker or layer, Ctrl+C there and Ctrl+V in WTE, and dragging
the saved file from Explorer.

### 0.3 "Done" in the user's words

- "Sticker editing just like PhotoCraft": I select a sticker, press Edit, and can cut out its background, crop it, paint and
  erase, add text and shapes as layers, add a white die-cut border and a shadow, change colours, and the model beside it shows
  the result while I work. I can change every one of these later. It also works from my tablet.
- "Work together": I click Edit in PhotoCraft, edit there, press Ctrl+S, and the sticker on the model updates by itself. I can
  drag a sticker or layer out of WTE into PhotoCraft or Explorer, drop pictures from Explorer or a browser onto the model where
  I want them, and copy / paste pictures between the two apps.
- Old projects open and export exactly as before.

### 0.4 Hard rules (every package)

1. No new crates. `Cargo.lock` changes only by the workspace version bump of P13.
2. Sources and text fixtures LF (`core.autocrlf=false`); P0 fixes the four CRLF notes (critic R11).
3. Never take the user's focus. No test, scenario or agent starts `photocraft.exe` or any window that activates. `--screenshot`
   runs stay off-screen. `photocraft-cli.exe` (v0.2.0, console exe) may be run by an agent headless to make fixtures, always
   with `CREATE_NO_WINDOW` when started from Rust.
4. Never OS mouse / keyboard injection. A real drag between two apps is a user hand test.
5. Tests never touch the real clipboard, the real `%LOCALAPPDATA%`, the registry (write), or the user's prefs.
6. "No AI": background removal is classic image maths (flood fill, colour unmixing, GrabCut in S1).
7. `clippy --workspace --all-targets` 0 warnings; `cargo test --workspace` has exactly one failure, WA20.
8. For a project that uses no new feature: mouse, pen and companion behaviour, bakes, exports and saved layer PNGs are identical
   to the P0 baseline (section 8.5).
9. Names and marks: the UI says "PhotoCraft" as plain text only ("Edit in PhotoCraft"). No ArtCraft name, wordmark or logo
   anywhere in WTE. The one exception is the verbatim licence notice (section 4.0).

---

## 1. What was verified in the code (2026-11-25, this worktree)

- `stamp.rs:10-14` `enum StampSource { Image { blob }, Text(TextSpec) }`; `StampObject { id, name, source, placement, tint, home_set }`
  (`:91-101`). `StampSource::` is matched in 33 files (171 places in `doc_stamps.rs`, 29 in `selftest/scenarios.rs`).
- Pictures reach the bake as `Arc<PremulMips>`: `doc_stamps.rs:1028` `image_source(blob)` (cache `Document::image_mips`),
  `:1042` `text_source`, `:1056` `source_mips`. Image blobs are decoded with `decode_image(.., MAX_STAMP_IMAGE_SIDE = 8192)`.
- `set_stamp_source` (`doc_stamps.rs:443`) refuses a kind switch, refits the placement (`refit_aspect` for images,
  `text::frame::refit_placement` with a kept point for texts, `text/frame.rs:33`), folds `Commit::Preview` calls into one
  pending entry.
- Text raster size: `text_raster_px` (`doc_stamps.rs:740`) picks a power of two in 32..=512 px per em from the on-model size.
- History (`history.rs`): `Edit::{Pixels { set, layer, patch: TilePatch }, Structure(Box<StructureSnapshot>), Multi}`;
  `StructureSnapshot { sets, stamps, unbaked }`; `TilePatch::swap_with(&mut PixelBuf)` and `TileBackup` work on any `PixelBuf`;
  budget 512 MiB, 100 entries; `group_edits`, `begin_pending`, `push_structure_edit`.
- Blobs (`blob.rs`): encoded bytes, de-duplicated, never freed in memory; `save` writes only blobs a stamp references.
- Project: `FORMAT_VERSION = 3` (`project/mod.rs:22`), `required_reader` (`project/save.rs:116`), `migrate.rs`.
- PSD: `export/psd.rs` `write_psd` (8BPS v1, RGB 8-bit, RLE, `luni`, merged image matted on white, no `lyid`), a strict
  `#[doc(hidden)] read_psd` for WTE's own files (`:520`). `export/packbits.rs`. `miniz_oxide 0.8.9` is a direct dependency of
  `wte_core` (FBX inflate), so ZIP-compressed PSD channels can be read without a new crate.
- EDT: `text/edt.rs` `distance_to_ink(coverage, w, h)` (exact Felzenszwalb-Huttenlocher with the sub-pixel edge term).
- PhotoCraft keeps PSD layer ids: import `pc020\crates\io\src\psd_import.rs:190` (`l.psd_id = rec.layer_id()`), export
  `psd_export.rs:146-151` (the stored id is written again; **corrected by the P0 check:** a layer without a stored id gets the
  **lowest free** id from 1 up, `psd_export.rs:577-590`, the same at HEAD, so a new PhotoCraft layer can take the id of a layer
  deleted in PhotoCraft in the same edit: fixture f12, 5.4). Its merged image is matted on
  white and written with 3 channels when everything is opaque (`psd_export.rs:602-612`); layer channels are RLE for 8 and 16
  bit, ZIP with prediction only for 32-bit float (`:65-67`); 16-bit layer info sits in the `Lr16` block (`:755`).
- App: `ViewMode { View3D, View2D, Split, Preview }` (`state.rs:33`, serialized), `ViewId` (`state.rs:44`), central area
  `view/mod.rs:57-99`, tools `ToolKind` (`state.rs:128`), upper tabs `UpperTab::{Tool, Color, TextureSets, Objects}`, lower
  `SideTab::{Layers, Brush, Text}`, actions `action.rs:113-121`, drops `dnd.rs` (195 lines), pen subclass `pen/win.rs:198-240`
  (statics `PREV`, `HWND`; installed from `pen::install`, `pen/mod.rs:264`), process spawn pattern `companion/adb.rs`.
- Self-test: `Step` (`selftest/mod.rs:49-98`) has `Do`, `Pointer`, `Click`, `Pen`, `Until`, `Setup`, `Assert`, `Shot`;
  `raw_input_hook` drops real input and clears dropped files. `Runner::tick` (`selftest/mod.rs:327`) calls
  `ctx.request_repaint()` every frame, so a frame counter always moves in a self-test and no step waits on the wall clock.

Checked again for revision 2 (the reviewers' line references, all confirmed in this worktree):

- `apply_final_bake` (`doc_stamps.rs:1288-1300`) calls a result fresh when `*s == out.object` (plus unbaked, not preview-baked,
  same mesh and maps). A `StampSource::Sticker { sticker }` is only an id, so this test alone cannot see a sticker edit.
  `same_inputs` (`:1744`) and `apply_wrap_preview` (`:1755`) use the same idea. A job's text picture comes back as
  `DrawnText { spec, frame, full, mips }` (`stamp_job.rs:82`).
- `wrapped_projector` (`doc_stamps.rs:1503`) also feeds the gizmo outline, `stats`, `local` and `surface`; `fit_stamp_depth`
  (`:186`) and `refit_auto_depth` (`:198`) write the stored depth; `serving_wrap_param` (`:1511`).
- `group_edits` (`history.rs:782`) starts with `flush_pending_edit()`; `TilePatch` / `TileBackup` (`history.rs:31-246`) take
  RGBA `PixelBuf` only.
- eframe is built without the clipboard feature (`Cargo.toml:36`). egui-winit 0.37 (`lib.rs:1129-1151`): Ctrl+X / Ctrl+C push
  `Event::Cut` / `Event::Copy` and **return** (no `Event::Key`); Ctrl+V pushes `Event::Paste(text)` only when its in-app string
  is non-empty, and also returns without a `Key` event. So the app never sees a key event for these three chords.
- Shortcuts (`shortcuts.rs:68-101`): V Navigate, B Brush, E Eraser, I Eyedropper, G Fill, S Stamp, X Text, Shift+X Swap Colors,
  R Transform, T Texture on Model, W Wireframe, F Frame Model, Shift+F Frame Selection, H / Shift+H / Alt+H objects,
  Ctrl+J Duplicate Layer, Ctrl+M Merge Down, Ctrl+] / Ctrl+[ move layer, 1 / 2 / 3 / F5 views, D Reset Colors.
- `upper_tab_strip` (`panels/side_groups.rs:236`) returns `[Rect; 4]` from one `ui.horizontal` row; no overflow rule exists;
  the index test is at `:356`.
- `ViewId { Main3D, Main2D, Preview, ExportPreview }` means "has its own offscreen target": `gpu/targets.rs:177 get_mut` and
  `tools/mod.rs:85 camera_of` match it exhaustively.
- Companion (`companion/input.rs:375, 452-513`): in `FingerMode::Navigate` one finger over a rect of `cx.views` (filled from
  `state.view_rects`) becomes a **Secondary** drag after 6 points of travel; `input.rs` has no list of view ids.
- `pen::eraser_override` dispatches `AppAction::SetTool(ToolKind::Eraser)` from `App::logic` (`app.rs:322-325`).
- `dnd.rs:14` reads `raw.hovered_files` for the "Drop to open" overlay. There is no `.gitattributes`. The About dialog
  (`dialogs/about.rs`) has no licence list and the installer script lists no licence files. `TextSpec.outline` exists
  (`text/mod.rs:91`).

P0 records the baseline numbers (expected near 1,411 passed / 1 failed WA20 / 48 ignored; clippy 0).

---

## 2. Data model (`wte_core`)

New module tree `crates/wte_core/src/sticker/` (`mod.rs` types, `render.rs`, `style.rs`, `fx.rs`, `adjust.rs`, `cutout.rs`,
`select.rs`, `shape.rs`, `resample.rs`), `doc_stickers.rs` (the `Document` commands), `psd/` (`mod.rs`, `read.rs`, `write.rs`),
`link.rs` (link data and the PSD <-> sticker mapping), `project/` additions. All of it headless and unit-tested.

### 2.1 Ids (`ids.rs`, ARCHITECTURE 4.4, 5.1)

```rust
pub struct StickerId(pub u64);       // from Document::next_id, like LayerId / StampId
pub struct StickerLayerId(pub u32);  // unique inside one sticker (StickerDoc::next_layer), never reused
```

### 2.2 The sticker document (`sticker/mod.rs`)

```rust
pub struct StickerDoc {
    pub name: String,
    /// The artboard in layer space (pixels). Width and height 1..=MAX_ARTBOARD (4096). Layers keep their coordinates when
    /// it changes (crop, trim, padding).
    pub artboard: IRect,
    /// Shape crop (E2): what the artboard cuts out of the layer composite.
    pub shape: ArtboardShape,            // Rect | RoundedRect { radius: f32 } | Ellipse
    /// Bottom to top. At most MAX_STICKER_LAYERS (32).
    pub layers: Vec<StickerLayer>,
    pub next_layer: u32,
    /// Shown in the tray and saved even when no stamp uses it (E4, E7).
    pub keep: bool,
}

pub struct StickerLayer {
    pub id: StickerLayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,                    // 0..=1
    pub blend: BlendMode,                // the existing seven modes (layer.rs)
    pub transform: LayerTransform,       // content pixels -> layer space
    pub content: StickerContent,
    /// Clipping mask ("pattern inside text"): this layer shows only where its base shows. The base is the nearest layer
    /// below with `clip == false`; the bottom layer's flag is ignored. See 4.1.
    pub clip: bool,
    /// Picture layers only in this delivery: 8-bit, the size of the picture; 255 = shown.
    pub mask: Option<LayerMask>,
    pub adjustments: Vec<Adjustment>,    // applied in order to the content pixels, non-destructive
    pub styles: StyleStack,
}
pub struct LayerMask { pub pixels: Arc<AlphaBuf>, pub version: u64 }

pub struct LayerTransform { pub translate: Vec2, pub scale: Vec2, pub rotation: f32 /* rad, clockwise, about the content centre */ }
// flips = negative scale. Identity = translate (0,0), scale (1,1), rotation 0: content pixel (x, y) sits at layer-space (x, y).

pub enum StickerContent {
    Picture(PicturePixels),
    Text(Box<TextSpec>),                 // size_px = em in layer-space pixels at render scale 1
    Shape(ShapeSpec),
}

pub enum PicturePixels {
    /// An imported image that was never changed: the blob's decoded pixels (straight RGBA8). Saved as the blob.
    Blob(BlobId),
    /// Edited pixels. Saved as a PNG inside the project (7.1 `stickers/`).
    Owned { pixels: Arc<PixelBuf>, version: u64 },
}

pub struct ShapeSpec {
    pub kind: ShapeKind,                 // Rect | RoundedRect { radius } | Ellipse | Polygon { sides: 3..=12 } | Star { points: 3..=24, inner: 0.1..=0.95 } | Line { width }
    pub size: Vec2,                      // content size in px
    pub fill: Paint,                     // None | Solid([u8; 4]) | Linear(Gradient)
    pub outline: Option<([u8; 4], f32)>, // colour, width (centred on the edge)
}
```

`AlphaBuf { width, height, data: Vec<u8> }` is new in `pixel.rs` (one byte per pixel). All colours are straight sRGB RGBA8
(ARCHITECTURE 4.2). `StickerDoc`, `StickerLayer`, `StyleStack`, `Adjustment` derive `Clone, Debug, PartialEq`.

**Picture versions (the identity of pixels; review A8).** Every owned picture and every mask carries a `version: u64` taken
from `Document::next_pixel_version` (monotonic for the life of the process, never reused, not saved; fresh numbers are given
at load). The document gives a new version at every change of the bytes: a stroke's `begin`, `replace_sticker_pixels`, an
undo / redo of `StickerPixels` (a new number each time, also when the bytes return to an earlier state), and whenever a new
buffer is put in. Arc clones of one buffer (history snapshots, a detached copy) keep the version, and equal versions always
mean equal bytes. Caches and the save-reuse table key pictures by this version (`Blob` pictures by their `BlobId`), **never**
by `Arc::as_ptr` and never by hashing pixel content. `PartialEq` of `PicturePixels::Owned` and `LayerMask` compares the
version first and the bytes only when the versions differ (so a loaded project still compares equal to the saved one).

No groups. A sticker with zero layers is valid (renders transparent).

### 2.3 Styles (`sticker/style.rs`)

```rust
pub struct StyleStack {
    pub contour: Option<Contour>,            // die-cut backing shape (E1)
    pub stroke: Option<Stroke>,
    pub drop_shadow: Option<Shadow>,
    pub inner_shadow: Option<Shadow>,
    pub outer_glow: Option<Glow>,
    pub inner_glow: Option<Glow>,
    pub color_overlay: Option<ColorOverlay>,
    pub gradient_overlay: Option<GradientOverlay>,
    pub bevel: Option<Bevel>,
}
pub struct Contour { pub on: bool, pub width: f32, pub smooth: f32 /* 0..=1 */, pub fill_holes: bool, pub color: [u8; 4], pub opacity: f32 }
pub struct Stroke  { pub on: bool, pub width: f32, pub position: StrokePos /* Outside | Inside | Center */, pub color: [u8; 4], pub opacity: f32 }
pub struct Shadow  { pub on: bool, pub color: [u8; 4], pub opacity: f32, pub angle_deg: f32, pub distance: f32, pub spread: f32 /* 0..=1 */, pub size: f32 }
pub struct Glow    { pub on: bool, pub color: [u8; 4], pub opacity: f32, pub spread: f32, pub size: f32 }
pub struct ColorOverlay    { pub on: bool, pub color: [u8; 4], pub opacity: f32 }
pub struct GradientOverlay { pub on: bool, pub gradient: Gradient, pub angle_deg: f32, pub opacity: f32 }
pub struct Bevel   { pub on: bool, pub size: f32, pub depth: f32 /* 0..=10 */, pub angle_deg: f32, pub altitude_deg: f32, pub highlight: [u8; 4], pub shadow: [u8; 4], pub opacity: f32 }
pub struct Gradient { pub stops: Vec<(f32, [u8; 4])> }   // 2..=8 stops, positions 0..=1 ascending
```

Lengths are pixels at the **reference scale** of what they are applied to: layer-space pixels for a sticker layer; for a stamp,
the pixels of its base picture at reference size (image stamp: image pixels; sticker stamp: artboard pixels at render scale 1;
text stamp: pixels at 128 px per em, scaled by `size_px / 128` when the text is drawn at another size). Limits: every length
0..=256, `sanitize()` clamps and replaces non-finite values. `StyleStack::default()` is all `None` and `is_default()` is true
when every member is `None` (an entry with `on: false` is kept so the user's numbers survive a toggle, but counts as "used" for
`required_reader`).

`StyleStack::margin(scale: f32) -> u32`: one number, the pixels the styles need beyond the content on any side, from the numbers
alone: `ceil(scale * max(contour.width * (1 + smooth), stroke outside width) + shadow reach + glow reach) + 2`, where shadow
reach = `distance + size` and glow reach = `size`, both counted from the outline shape (4.3). Zero when no enabled style draws
outside. Inside styles (Inside / Center stroke, inner shadow, inner glow, bevel) never add to the margin: `apply_styles` pads
its working buffers by at least one transparent pixel by itself and crops back (4.3; a fully opaque photo has no transparent
pixel of its own).

**Text stamps: Stroke and the text's own Outline.** `TextSpec.outline` (Text tab) stays as it is; it is part of the text
picture, so it is inside the base alpha. A stamp-level Stroke on a text stamp therefore runs around the outlined letters. Both
may be on. In the Sticker style editor of a text stamp the Stroke block shows one line: "Text also has its own Outline (Text
tab). This stroke goes around it." Nothing is converted or hidden.

Style presets (`StylePreset`: `ClassicSticker` = white contour 12 px smooth 0.5 + black shadow 40 %, `Outline`, `Glow`, `Badge`,
`ShadowOnly`, `Clear`) are constructor functions; preset lengths are for a 512-px picture and are scaled by
`max(content side) / 512` when applied.

### 2.4 Adjustments (`sticker/adjust.rs`)

```rust
pub enum Adjustment {
    BrightnessContrast { brightness: f32 /* -100..=100 */, contrast: f32 /* -100..=100 */ },
    HueSaturation { hue_deg: f32 /* -180..=180 */, saturation: f32 /* -1..=1 */, lightness: f32 /* -1..=1 */, colorize: bool },
    Levels { in_black: u8, in_white: u8, gamma: f32 /* 0.1..=9.99 */, out_black: u8, out_white: u8 },
    Invert,
    BlackWhite,
    Posterize { levels: u8 /* 2..=32 */ },
    Threshold { level: u8 },
}
```

Per pixel on straight RGB, alpha untouched; each is a 256-entry LUT per channel except HueSaturation and BlackWhite (4.6).

### 2.5 Stamps (`stamp.rs`, ARCHITECTURE 5.12)

```rust
pub enum StampSource { Image { blob: BlobId }, Text(TextSpec), Sticker { sticker: StickerId } }
pub struct StampObject { /* existing fields */ pub styles: StyleStack }
```

- Linked layers of a sticker stamp are `LayerKind::Stamp` (image decal), so layer names, export names and the "3D stamp" /
  "2D stamp" badges need no new case.
- `tint` still multiplies the base picture before styles (a tinted sticker with a white contour keeps the contour white).
- `set_stamp_source` kind rule: Image -> Sticker and Text -> Sticker are allowed only through `promote_stamp` /
  `convert_text_stamp` (2.8); every other kind switch stays `InvalidParam`.

### 2.6 From a source to the picture the bake places

```rust
/// What a source looks like before pixels exist (geometry only).
pub struct SourceFrame { pub content: [u32; 2], pub margin: u32 }     // picture size = content + 2 * margin on both axes
```

Pipeline for stamp `id` at quality `q` (`doc_stamps.rs` `source_mips`, the only place that changes):

1. **Base picture** (straight RGBA8, size `content`): image -> the blob's decoded pixels; text -> `render_spec` as today;
   sticker -> `sticker::render(input, scale, q)` (4.1), scale from `sticker_scale(id)` below.
2. **Tint:** with default styles nothing happens here and the bake multiplies by `tint` as today. With styles, the tint is
   multiplied into the base picture here (before the styles, so a contour keeps its own colour) and the bake gets `[255; 4]`.
3. If `stamp.styles.is_default()`: mips = `PremulMips::build(base)`; for images this is exactly today's `image_mips[blob]`
   (the cache entry is reused, nothing is rebuilt). **Bit-identical path.**
4. Else: the styles are applied to a **working copy** of the base (below), `fx::apply_styles(&work, &styles, style_scale)` ->
   a picture of size `work + 2 * margin`; mips from it. Cached in
   `Document::styled_mips: HashMap<StampId, StyledEntry>`, `StyledEntry { base: BaseIdentity, styles, scale_bits, quality, mips }`
   where `BaseIdentity` = `Blob(BlobId)` | `Text(Arc<PremulMips>)` (compared with `Arc::ptr_eq`; the entry holds the Arc, so
   its address cannot be reused) | `Sticker(Arc<StickerDoc>, scale bits)` (also `Arc::ptr_eq`).

**Working size of a styled picture (review A5).** Styles are never computed on more pixels than the model can show:

- `work_scale(id, q)` = the largest `1 / 2^L` (L = 0, 1, 2, ...) such that the working picture (a) has no side over 4096 and
  (b) for `BakeQuality::Final` is not more than twice as dense as the texels it lands on (the density measure of
  `text_raster_px`), and for `BakeQuality::Preview` has, margin included, at most 524,288 pixels. The working picture is mip
  level `L` of the base (the exact box-filter chain that exists already, un-premultiplied); `L = 0` is the base itself.
- `style_scale = reference scale * work_scale`: every style length is multiplied by it, so the look does not change with `L`.
- An 8192 x 8192 photo is therefore styled at 4096 or less; `apply_styles` holds at most two `f32` distance fields at a time
  (each is reduced to an 8-bit coverage plane as soon as it has been used); peak extra memory at 4096 x 4096 is about 200 MB,
  on a worker thread.
- **Where it runs:** a Final is styled inside the `StampBakeJob` (the job carries the base as `Arc<PremulMips>` or as sticker
  render input, plus the `StyleStack`), never on the UI thread. A Preview (a slider being dragged) is styled on the UI thread
  at the Preview working size (budget 8.4).

**Two placements.** `Document::bake_placement(id) -> Placement`: the stored placement with its size multiplied per axis by
`(content + 2 * margin) / content` (`half_size` for `Projected`, `size_px` for `Uv`); centre, rotation, flips, depth, mode
untouched. With margin 0 it returns the stored placement without arithmetic (bit for bit).

| Uses the **bake** placement (the picture that is drawn, margin included) | Uses the **stored** placement (the artwork box) |
|---|---|
| `make_bake_job`, `rebake_stamp`, `bake_and_apply`, `ensure_wrap_param`, `make_wrap_job`, `stamp_ink_area`, `stamp_shared_texels`, and a **new** `bake_wrapped_projector(id)` used by exactly these | the gizmo and its handles, `wrapped_projector(id)` as it is today with all its other readers (`stamp_wrap_outline`, `stats`, `local`, `surface`: `gizmo.rs:118`, `text_path.rs:126/169`), `stamp_text_frame`, `text_raster_px`, the panels |
| `serving_wrap_param`: "does the cached parameterisation serve?" compares against the **bake** placement | - |

`wrapped_projector` is not changed (the gizmo outline keeps the artwork's size). **Auto depth:** `fit_stamp_depth` /
`refit_auto_depth` keep writing the depth into the stored placement, but they measure the **bake** footprint (so the depth
covers the shadow too); `set_stamp_styles` and every sticker edit that changes the margin call `refit_auto_depth` when the
stamp's depth is automatic. With margin 0 nothing differs from today.

**Sticker render scale** `sticker_scale(id) -> f32`: 1.0 when every layer is a picture drawn at scale 1 or larger. Otherwise
(a text or shape layer, or a picture layer whose transform shrinks it, so it has more pixels than the artboard shows) the
smallest of {1, 2, 4} that gives at least one sticker pixel per texel on the model (same density measure as `text_raster_px`,
using the artboard height), capped so the output stays within 4096 px a side and 16.8 megapixels. `BakeQuality::Preview`
additionally halves the scale (down to 1/4) until the output has at most 1,048,576 pixels. Re-evaluated where `retune_text`
runs today (Final placement edits).

### 2.7 Caches, determinism, memory

- `sticker::render` is a pure function of `StickerRenderInput { doc: Arc<StickerDoc>, pictures: Vec<(BlobId, Arc<PixelBuf>)>, fonts }`
  (everything resolved, `Send`), so a `StampBakeJob` can carry it and render on the worker (round-5 deferred Final bakes; the
  UI thread never renders a Full sticker during an edit that ends with `Commit::Final`). The job also carries the stamp's
  `StyleStack` and applies it after drawing the base picture (text, image or sticker), so a styled text stamp keeps round 5's
  rule that a Final draws nothing on the UI thread.
- **Job freshness (review A1; a blocker if missed).** `StampBakeJob` and `StampBakeOutput` get
  `sticker: Option<Arc<StickerDoc>>` (the value the job rendered). A result is fresh only when, in addition to today's tests,
  `Document::sticker(id)` of the stamp's source **is that same Arc** (`Arc::ptr_eq`; both `None` for image and text stamps).
  One helper, `Document::job_inputs_current(&out) -> bool`, is used by `apply_final_bake`, `same_inputs` and
  `apply_wrap_preview`. The styles need no extra test: they are in `StampObject`, which `*s == out.object` already compares.
  A stale result is dropped and the stamp stays in `unbaked`, so the next job bakes the newer sticker (test k3a-9).
- **A job stores what it drew (review A27).** Next to `DrawnText`, the output carries
  `DrawnPicture { base: BaseIdentity, render: Option<(Arc<StickerDoc>, scale bits, Arc<PixelBuf>)>, styled: Option<StyledEntry> }`.
  `apply_bake_output` puts the sticker render into the render cache and the styled mips into `styled_mips`, so the next
  Preview or thumbnail does not render again on the UI thread.
- `Document::sticker_cache: Box<StickerCaches>`: decoded blob pictures (`HashMap<BlobId, Arc<PixelBuf>>`, straight);
  per-layer rasters keyed by `(sticker id, layer id, LayerKey, scale bits)`, where `LayerKey` is the layer's non-pixel fields
  compared **by value** (transform, adjustments, styles, text / shape spec: small) plus the picture's and the mask's
  **version** (2.2) (LRU 64 entries, 256 MiB); finished renders in an LRU of 8 entries
  `{ doc: Arc<StickerDoc>, scale bits, quality, picture }` looked up with `Arc::ptr_eq` (the entry holds the Arc). Not
  persisted, not in history.
- Determinism (ARCHITECTURE 4.7): integer or fixed-order `f32` arithmetic; rayon only over independent rows or tiles; results
  identical for any thread count. Tests run the same call inside two **local** pools
  (`rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(..)` with n = 1 and n = 8) and compare bytes; the
  global pool is built once per process, so `RAYON_NUM_THREADS` cannot be used for this. No clock, no randomness.
- Limits (constants in `sticker/mod.rs`, each enforced with an `EditError::InvalidParam` message the UI can show): artboard side
  4096; 32 layers; picture layer side 8192 (decode limit, as today) and 4096 after any in-editor operation that creates
  pixels; total owned picture bytes per sticker 512 MiB (a new layer over the limit is refused); gradient 8 stops; text layer
  canvas as the text engine's own limits.
- History cost: `Edit::StickerPixels` counts its patch bytes; a `Structure` snapshot counts every `Arc<PixelBuf>` /
  `Arc<AlphaBuf>` of its sticker map that the live document no longer holds (extend `history_bytes` and the eviction cost the
  same way raster pins are counted).

### 2.8 Document API (`doc_stickers.rs`, ARCHITECTURE 5.14 / 5.20)

```rust
impl Document {
    pub fn sticker(&self, id: StickerId) -> Option<&Arc<StickerDoc>>;
    pub fn stickers(&self) -> impl Iterator<Item = (StickerId, &Arc<StickerDoc>)>;
    pub fn sticker_users(&self, id: StickerId) -> Vec<StampId>;
    pub fn blob_users(&self, blob: BlobId) -> Vec<StampId>;                       // image stamps showing this blob (E6)

    pub fn add_sticker(&mut self, doc: StickerDoc) -> Result<StickerId, EditError>;        // "Add Sticker"
    pub fn remove_sticker(&mut self, id: StickerId) -> Result<(), EditError>;              // "Delete Sticker"; Err when a stamp uses it
    /// Image stamp -> sticker with one Picture(Blob) layer, artboard = the image, identity transform. No pixel changes, no
    /// re-bake, layer ids kept. `scope`: this stamp only, or every stamp showing the blob (they then share the sticker).
    pub fn promote_stamp(&mut self, stamp: StampId, scope: ShareScope) -> Result<StickerId, EditError>;   // "Convert to Sticker"
    /// Text stamp -> sticker with one Text layer. The em size of the layer is the stamp's **current** `text_raster_px`
    /// (32..=512), so the artboard has exactly the pixels today's text raster has and the picture matches at every size
    /// (review A24); `sticker_scale` takes over when the stamp is resized later. The stamp's styles are kept, their lengths
    /// multiplied by `text_raster_px / 128` (2.3 reference scale). Linked layers become `LayerKind::Stamp`. Re-bakes; the
    /// picture is the same within rounding (not bit-identical, section 9).
    pub fn convert_text_stamp(&mut self, stamp: StampId) -> Result<StickerId, EditError>;
    /// "Only this sticker" (E6): the stamp gets its own copy of a shared sticker (Arc-shared pixels, new id).
    pub fn detach_sticker(&mut self, stamp: StampId) -> Result<StickerId, EditError>;

    /// The one structural edit: replaces the sticker value. Refits the placement of every user when the artboard changed
    /// (2.9), re-bakes users (`Commit` as for stamps: Preview calls fold into one pending entry with `label`).
    /// `on` names the sticker by id, or as "the sticker of this stamp": for an image stamp that is not a sticker yet the
    /// command **promotes inside itself** (2.10), so promotion and edit are one history entry also for a Preview edit.
    pub fn set_sticker(&mut self, on: StickerRef, edit: impl FnOnce(&mut StickerDoc), label: &str, commit: Commit) -> Result<StickerId, EditError>;

    /// Pixel edits on a picture layer or its mask (brush, eraser): `begin` starts a `TileBackup` and gives the buffer a
    /// new version; `dab` paints with the 2D brush kernels (`brush::dab_2d`, `apply_brush`) honouring the selection mask;
    /// `end` pushes one history entry and Final-re-bakes the users. `Picture(Blob)` becomes `Owned` at `begin`.
    /// `begin` is the one place where a buffer shared with a history snapshot is copied (`Arc::make_mut`, 2.10).
    pub fn sticker_stroke_begin(&mut self, on: StickerRef, layer: StickerLayerId, target: PixelTarget, params: StickerBrush) -> Result<StickerId, EditError>;
    pub fn sticker_stroke_dab(&mut self, at: Vec2 /* layer space */, pressure: f32) -> Result<IRect, EditError>;
    pub fn sticker_stroke_end(&mut self) -> Result<(), EditError>;
    pub fn sticker_stroke_cancel(&mut self);
    /// Whole-buffer result of a tool (magic eraser, colour to alpha, defringe, fill, gradient, blur, sharpen, remove
    /// background, delete selection). Same size as before -> a `StickerPixels` tile patch of the changed tiles; another
    /// size -> a `Structure` edit.
    pub fn replace_sticker_pixels(&mut self, on: StickerRef, layer: StickerLayerId, target: PixelTarget, new: TileData, label: &str) -> Result<StickerId, EditError>;
    /// "Revert" in the editor (3.1): one `Structure` entry that puts the stickers and stamps named in `to` back.
    pub fn restore_sticker_state(&mut self, to: &StickerCheckpoint, label: &str) -> Result<(), EditError>;
    pub fn sticker_checkpoint(&self, stamps: &[StampId], stickers: &[StickerId]) -> StickerCheckpoint;   // Arc clones

    pub fn set_stamp_styles(&mut self, stamp: StampId, styles: StyleStack, commit: Commit) -> Result<(), EditError>;   // "Edit Sticker Style"
    pub fn set_stamps_styles(&mut self, stamps: &[StampId], styles: &StyleStack) -> Result<(), EditError>;             // "Paste Style", one entry (E5)

    pub fn bake_placement(&self, stamp: StampId) -> Option<Placement>;
    pub fn stamp_source_frame2(&mut self, stamp: StampId) -> Result<SourceFrame, EditError>;
    /// The finished picture of a stamp or of a tray sticker (straight RGBA8), with or without stamp styles: export picture,
    /// copy, drag out, thumbnails. Never a history command.
    pub fn stamp_picture(&mut self, stamp: StampId, with_styles: bool) -> Result<PixelBuf, EditError>;
    pub fn sticker_picture(&mut self, id: StickerId, styles: Option<&StyleStack>, scale: f32) -> Result<PixelBuf, EditError>;
    pub fn rebake_sticker_users(&mut self, id: StickerId, quality: BakeQuality);            // live preview during a stroke
}
pub enum PixelTarget { Pixels, Mask }
pub enum ShareScope { ThisStamp, AllUsers }
pub enum StickerRef { Id(StickerId), OfStamp(StampId, ShareScope) }
pub enum TileData { Rgba(PixelBuf), Alpha(AlphaBuf) }       // must match the target
```

`set_stamp_styles` refits an automatic depth when the margin changed (2.6). `stamp_picture(.., with_styles: true)` is "what
you see": copy, drag out and Export Picture from a stamp use it (6.3, 6.4).

Guards as for stamps: `guard_no_stroke`, `LayerLocked` when a linked layer of a user is locked, a sticker stroke and a texture
stroke exclude each other. New `EditError` variants: `UnknownSticker`, `UnknownStickerLayer`, `StickerLimit(&'static str)`.

### 2.9 Placement refit when the artboard changes

A crop, trim, padding or resize changes `artboard`. Rule (the text rule of FEATURE_TEXT 5.11 generalised; implement as
`sticker::refit_placement(placement, old: &IRect, new: &IRect)` next to `text::frame::refit_placement`, sharing `HostFrame`):

- the on-model size of one layer-space pixel stays (`k = host size / old artboard size`, per axis);
- the on-model position of every layer-space point stays: the centre moves by `(new centre - old centre) * k` along the
  placement's own axes; the size becomes `new size * k`.

So the artwork never jumps on the model. Flips and rotation are honoured by `HostFrame`. A change of `shape` or of layers alone
changes no placement. For a Wrap placement the moved origin is used as the anchor as it is (same behaviour as a text edit today).

### 2.10 Undo granularity (ARCHITECTURE 5.18, 6.7)

| User action | History entry |
|---|---|
| One brush / eraser stroke on a picture or mask | one `StickerPixels` ("Sticker Brush", "Sticker Eraser") |
| Magic eraser click, fill click, colour to alpha, defringe, matte removal, remove background, delete selection | one entry each (tile patch) |
| A slider drag (style, adjustment, opacity, transform handle, text field typing burst) | one entry on release (`Commit::Preview` folding, as stamp edits today) |
| Add / delete / duplicate / reorder / rename / show-hide layer, add text, add shape | one `Structure` entry each |
| Crop, trim, padding, canvas size, resize, rotate 90, flip, shape crop | one `Structure` entry (placement refit inside it) |
| Convert to Sticker (promotion) caused by an edit | **inside** the entry of that edit: one undo removes both (rule below) |
| Paste style onto N stamps | one entry |
| Update linked sticker (PhotoCraft saved) | one entry "Update Linked Sticker" |
| Revert (editor) | one entry "Revert Sticker" |
| Selection changes, active layer, tool, canvas zoom / pan, tray selection | none (app state) |

`StructureSnapshot` gains `stickers: BTreeMap<StickerId, Arc<StickerDoc>>` (Arc clones). `restore_structure` swaps it, drops
cache entries of changed stickers, and marks the users unbaked (deferred Final bakes) or re-bakes them. `Edit::StickerPixels
{ sticker, layer, target, patch }` swaps tiles through `Arc::make_mut` on the doc and on the buffer (copy-on-write when a
snapshot shares them), gives the buffer a new version and re-bakes the users. Undo never waits for a bake (round-5 rule).

**Promotion inside an edit (review A3).** `group_edits` cannot be used: it begins with `flush_pending_edit()`, and a
`Commit::Preview` first edit (a slider, a Move drag) stays pending outside the group, which would give two undo steps.
Instead every content command that gets `StickerRef::OfStamp` for an image stamp does this, in this order:

1. take the "before" state: `begin_pending(label)` for a Preview or Final structural edit (the pending entry's `before`
   snapshot still shows the image stamp), or a `StructureSnapshot` kept in the open stroke for a pixel edit;
2. promote **without pushing an entry** (internal `promote_no_history`; the public `promote_stamp` = this + one entry
   "Convert to Sticker");
3. apply the edit. A structural edit ends as the one pending / pushed `Structure` entry. A pixel edit pushes
   `Edit::Multi([Structure(before promotion), StickerPixels])` as **one** entry, built directly.

Tests (k3b): a Preview-then-Final opacity drag on an image stamp, and a brush stroke on an image stamp, are each one undo step
that brings back `StampSource::Image`; `cancel_pending_edit` after the Preview also brings it back.

**Tile patches for masks (review A4).** `TilePatch` and `TileBackup` become generic over the bytes per pixel: a small trait
`TileBuf { const BPP: usize; fn size(&self) -> [u32; 2]; fn bytes(&self) -> &[u8]; fn bytes_mut(&mut self) -> &mut [u8]; }`
implemented for `PixelBuf` (4) and `AlphaBuf` (1); the patch stores its `bpp` and refuses a buffer of another kind. The
existing `Edit::Pixels` path keeps its bytes and its tests unchanged (P3b proves it with the existing history tests). Masks
stay `AlphaBuf` (a 4096 x 4096 mask is 16 MB, not 64).

**The first dab after a snapshot (review A25).** Every `Structure` snapshot shares the sticker's buffers, so the next pixel
edit copies the whole buffer once (`Arc::make_mut`): up to 64 MB for a 4096 x 4096 picture. This copy happens in
`sticker_stroke_begin`, never in a dab, and has its own budget (8.4: 40 ms at 4096 x 4096). Tiled sharing of picture buffers
is not built.

`Document::links` (section 5) is **not** in snapshots.

### 2.11 Project format 4 (ARCHITECTURE 7.1-7.3, 5.23)

`FORMAT_VERSION = 4`. Manifest additions, all `#[serde(default)]` and skipped when empty:

```rust
pub struct Manifest { /* ... */ pub stickers: Vec<StickerEntry>, pub links: Vec<LinkEntry> }
pub enum StampSourceEntry { Image { blob }, Text(TextEntry), Sticker { sticker: u64 } }      // tag "sticker"
pub struct StampEntry { /* ... */ pub styles: Option<StyleStackEntry> }                        // None when default
pub struct StickerEntry { id, name, artboard: [i32; 4], shape, keep, next_layer, layers: Vec<StickerLayerEntry> }
pub struct StickerLayerEntry { id, name, visible, locked, opacity, blend, transform: [f32; 5], content: StickerContentEntry,
                               mask: Option<String> /* file */, adjustments: Vec<AdjustmentEntry>, styles: Option<StyleStackEntry> }
pub enum StickerContentEntry { Picture { blob: Option<u32>, file: Option<String> }, Text(TextEntry), Shape(ShapeEntry) }
```

Entry types mirror the model types field by field with snake_case names; enum variant names are never renamed (7.3 rule).
Archive entries: `stickers/k<id:05>/p<layer:03>.png` (owned pictures, straight RGBA PNG, `PngLevel::Fast`),
`stickers/k<id:05>/m<layer:03>.png` (masks, 8-bit grey), `links/k<id:05>.psd` (5.3). Blob pictures are ordinary `blobs/` entries
(`referenced_blobs` now also walks stickers; fonts of text layers too).

- **Which stickers are saved:** those a stamp uses, and those with `keep`. Others are dropped at save (they are leftovers of undo).
- **`required_reader`:** 4 when any stamp has a `Sticker` source or a non-default `styles`, or `stickers` or `links` is
  non-empty; otherwise exactly what it returns today. An old build then refuses a project that uses stickers ("made by a newer
  version") and opens one that does not, as after every earlier bump.
- **Migration:** `migrate::upgrade` 3 -> 4 changes nothing (new fields default). Old projects load into exactly the same
  in-memory document as before (`styles` default, no stickers).
- **Load errors:** a sticker whose picture file is missing loads with that layer empty and one warning; a stamp whose sticker is
  missing loads its layers as plain raster layers (the existing rule for a missing blob).
- **Save reuse:** owned picture and mask PNGs are copied from the previous file when the buffer's **version** (2.2) is the one
  recorded for that archive entry at the last load or save (extend `PngReuse` with an `entry name -> version` table);
  otherwise encoded. Versions are never reused, so a stale copy is impossible (an address-based table could copy the wrong
  PNG after the allocator reused an address: data loss).
- `StickerLayerEntry` also has `clip: bool` (`#[serde(default)]`).
- `extract_bundle` ("Extract Original Files") also writes each saved sticker's finished picture to `stickers\<name>.png`.

### 2.12 Sticker files `.wtes` (E7; ARCHITECTURE new 7.8)

One sticker for reuse: a ZIP with `mimetype` (`application/x-wtes`), `sticker.json` (`{ format: 1, sticker: StickerEntry,
styles: Option<StyleStackEntry>, blobs: [BlobEntry] }`), the same `p*.png` / `m*.png` entries, `blobs/*` (pictures and fonts),
`preview.png` (finished picture, at most 512 px). `sticker::file::{write_wtes, read_wtes}`; reading assigns new ids. Deterministic
bytes (fixed timestamps, as the export zip).

---

## 3. The sticker editor (`wte_app`)

### 3.1 Where it lives

- `ViewMode::Sticker` (mode strip button "Sticker", after Preview; key `4`, next to 1 / 2 / 3). Central area:
  `[sticker canvas | divider | 3D view]`, ratio in `Prefs::sticker_split` (default 0.55). The 3D half is the ordinary
  `ViewId::Main3D` with the Navigate behaviour only (orbit, pan, zoom, frame); it re-renders when a bake lands.
- **The canvas is not a `ViewId`** (review A17): `ViewId` means "has its own offscreen target", and `gpu/targets.rs get_mut`
  and `tools/mod.rs camera_of` match it exhaustively. The canvas is an egui-painted area with its own id: its rect is
  `AppState::sticker_canvas_rect: Option<Rect>` (also registered as `PanelHit::StickerCanvas`), and the self-test steps that
  take a view (`Pointer`, `Click`, `Pen`, `Drop`) take `Target { View(ViewId), StickerCanvas }` with `From<ViewId>`, so
  existing scenarios compile unchanged. Wherever this plan says "the Sticker canvas" as a pointer target it means this.
- `ViewMode::Sticker` is never written to prefs or `ui_state` (the previous mode is written instead), so an older build reading
  the same prefs file is not confused.
- State: `Session::sticker_edit: Option<StickerEdit>`:

```rust
pub struct StickerEdit {
    pub target: StickerTarget,           // Stamp(StampId) | Tray(StickerId) | ToolSource (the Stamp tool's unplaced sticker, E4)
    pub active_layer: Option<StickerLayerId>,
    pub selected_layers: Vec<StickerLayerId>,
    pub selection: Option<Selection>,    // 3.4; app state, not in history, not saved
    pub tool: StickerTool,
    pub camera: Canvas2D,                // pan / zoom of the canvas; reuse view2d's camera type and its wheel / pinch / two-finger rules
    pub return_mode: ViewMode,
    pub share_answer: Option<ShareScope>,// E6: the user's answer for this session of the editor
    pub entry: StickerCheckpoint,        // the stamp(s) and sticker as they were when the editor opened (Arc clones), for Revert
    pub held_update: bool,               // a link update is waiting for the stroke / drag to end (5.6)
}
```

- **Entering:** "Edit Sticker" on the Sticker tab and the Tool page of a selected stamp, Layer menu > Edit Sticker, the row
  context menu of a stamp layer, double-click on a stamp layer's thumbnail, `Enter` with a stamp selected, "Edit" on a tray
  item, "Edit before placing" in the Stamp tool options (E4). **Leaving:** the "Done" button in the canvas header, `Esc` (after
  it has cancelled a busy tool and dropped a selection, 3.3), or picking another view mode. Leaving flushes a pending edit.
- **Revert (review B12):** next to "Done" a button **Revert** ("put the sticker back to how it was when I opened it"),
  enabled when the sticker differs from `entry`. It calls `restore_sticker_state(&entry, "Revert Sticker")`: one history
  entry, itself undoable; the editor stays open. Nothing is thrown away: the steps made in the editor stay in the history
  below it.
- **What the canvas shows for each source kind:**
  - sticker stamp / tray sticker: the sticker, fully editable;
  - image stamp: the same UI on a virtual one-layer sticker (built in the app from the blob; nothing in the document changes).
    The first content edit passes `StickerRef::OfStamp(stamp, scope)` and the command promotes inside itself (2.10): one
    undo step, also for a slider or a drag. Style-only edits never promote;
  - text stamp: the text picture with its stamp styles (read-only canvas), the Text tab edits the text as today, and a button
    "Convert to layered sticker" (`convert_text_stamp`) with a one-line note that the text keeps being editable as a text layer.
- **Shared pictures (E6):** before the first content edit of a sticker with more than one user (or of a blob shown by more than
  one image stamp), a modal `Dialog::StickerShared { users: N }` asks "This picture is used by N stickers." with **Edit all** /
  **Only this one** / Cancel. The answer is kept in `share_answer` until the editor closes. "Only this one" = `detach_sticker`
  (or `ShareScope::ThisStamp` at promotion). **The same question is asked before a link is made** (review B2): at "Edit in
  PhotoCraft" and at a drag of the PSD handle (6.4) on a shared sticker or shared blob, and the detach or promotion happens
  **before** the PSD is written, because an update from PhotoCraft is a content edit that arrives while the user is not
  looking at WTE. A link always belongs to one sticker; "Edit all" means every stamp of that sticker follows PhotoCraft.
- **Checkerboard** behind transparent parts; artboard outline; pixel grid from 800 % zoom; the canvas texture is uploaded with
  `ctx.load_texture` and updated with `TextureHandle::set_partial` for dirty rectangles.

### 3.2 Layout of the panels in Sticker mode

| Place | Content |
|---|---|
| Canvas header (top of the canvas, 28 pt) | sticker name (click to rename), "Done", "Revert", zoom read-out, Fit / 100 %, link badge and "Edit in PhotoCraft"; while a stroke is open on a styled layer the note "styles update when you lift" (3.5) |
| Left tool strip | the sticker tools (3.3) replace the paint tools while in Sticker mode; the previous paint tool comes back on leaving |
| Upper group, **Tool** tab | options of the active sticker tool; below them the active layer's properties: Opacity, Blend, transform numbers (X, Y, W, H, rotation, flips), **Layer styles** (the `StyleStack` editor, one collapsible block per style with an on/off box), **Adjustments** (list, add menu, sliders), **Mask** (Add from selection / Invert / Apply / Delete; "Paint on mask" toggle) |
| Upper group, **Color** tab | unchanged (the sticker brush, fill, text and shape tools use the primary colour) |
| Upper group, **Sticker** tab (new, always there, also outside Sticker mode) | 3.9 |
| Lower group, **Layers** tab | in Sticker mode: the sticker's layers (eye, thumbnail, name, lock, a small "clipped" arrow on a layer with `clip`, drag to reorder, multi-select, context menu: Duplicate, Delete, Merge Down, Rasterize (text / shape -> picture), Clip to Layer Below (toggle; also a button under the list), Layer from file..., Copy style / Paste style), buttons: New picture layer, Text, Shape, Add picture... Outside Sticker mode: unchanged |
| Lower group, **Brush** tab | the existing brush settings; in Sticker mode the brush type is Circular only (the 3D brush objects are greyed with a hint) |
| Lower group, **Text** tab | edits the active text layer's `TextSpec` through the existing `panels/text_block.rs` (fonts, gaps, warp, path: all of it) |

Every control gets a `PanelHit` id and a rect in `AppState::panel_rects` (for scenarios and `control_sweep`).

### 3.3 Tools (`StickerTool`, new `crates/wte_app/src/sticker/tools/*`)

All tools work in layer space through the canvas camera; pointer handling follows `tools/mod.rs` (press / drag / release,
`Esc` cancels, `Commit::Preview` while dragging). Pressure comes from the existing `PenRouter` read-out.

| Tool (key in Sticker mode) | What it does | Options |
|---|---|---|
| Move / Transform (V) | click selects the top layer under the pointer; drag moves; handles scale (Shift keeps aspect, on by default), rotate handle (Shift = 15 degree steps); arrow keys nudge 1 px (Shift 10). **With a selection on a picture layer** the drag first cuts the selected pixels into a new layer ("Layer via Cut") and moves that layer: one entry "Move Selection" | Auto-select layer; numeric fields; Flip H / V; Reset transform |
| Marquee (M) | rectangle or ellipse selection; Shift adds, Alt subtracts | Feather px; fixed aspect |
| Lasso (L) | polygon lasso (click points, double-click or Enter closes); drag = freehand | Feather |
| Magic Wand (W) | selects by colour from the clicked pixel | Tolerance 0..=255 (32), Contiguous (on), Anti-alias (on), Sample: active layer / all layers |
| Crop (C) | drag the artboard edges or draw a new artboard; Enter applies (`artboard` change, one entry); "Delete cropped pixels" off by default | Shape: Rectangle / Rounded (radius) / Ellipse (E2); aspect presets 1:1, 4:3, 16:9, free |
| Brush (B) | paints the primary colour on the active picture layer (or its mask when "Paint on mask" is on: white shows, black hides) | size, hardness, flow, opacity, pen pressure (the existing `BrushSettings`) |
| Eraser (E) | erases alpha on the picture (or hides on the mask) | as Brush |
| Fill (G) | fills the clicked region with the primary colour by colour tolerance, or the selection when there is one | Tolerance, Contiguous, Anti-alias |
| Magic Eraser (Shift+E) | makes the clicked colour region transparent | Tolerance, Contiguous, Anti-alias, "Into mask (keep pixels)" |
| Eyedropper (I) | picks a colour from the sticker (all layers) into the primary colour | - |
| Gradient (Shift+G) | drag draws a linear gradient on the active picture layer (or its mask) from the press point to the release point, limited by the selection; one entry "Gradient" | Primary to Secondary / Primary to Transparent / the stops of a gradient editor (2..=8); Opacity; Reverse |
| Text (X, the same key as WTE's Text tool) | click adds a text layer at the point (primary colour, the Text tab's settings) and focuses the content field; click on a text layer selects it | Text tab |
| Shape (U) | drag adds a shape layer | kind, fill (none / solid / gradient), outline colour and width, corner radius, sides / points |
| Hand (H, or Space held; Secondary or Middle drag with any tool) / zoom (wheel, pinch) | pans and zooms the canvas | - |

Layer-kind rules: Brush, Eraser, Fill, Gradient, Magic Eraser, Colour to Alpha, Defringe, Blur, Sharpen need a picture layer.
On a text or shape layer they show the toast "Rasterize this layer to paint on it" with a Rasterize button (never a silent
rasterize).

**Keys in Sticker mode (reviews A12, B6).** The global table of `shortcuts.rs` is static and several of these keys are taken
there. P6a adds a second static table `STICKER_SHORTCUTS` and one routing rule in the shortcut dispatcher:

> In `ViewMode::Sticker` a chord is looked up in `STICKER_SHORTCUTS` first. If it is bound there, that binding wins and the
> global one is not run. If it is not bound there, the global binding runs, **unless** the global entry is in the short
> `NOT_IN_STICKER` list. Outside Sticker mode only the global table is used, exactly as today.

| Chord | Outside Sticker mode (unchanged) | In Sticker mode |
|---|---|---|
| V | Navigate | Move / Transform |
| B, E, I, G | Brush, Eraser, Eyedropper, Fill (paint tools) | the sticker Brush, Eraser, Eyedropper, Fill |
| X | Text tool | sticker Text tool (one Text key for both modes) |
| M, L, C, U, Shift+E, Shift+G | unbound | Marquee, Lasso, Crop, Shape, Magic Eraser, Gradient |
| W | Wireframe | Magic Wand (Wireframe stays in the View menu) |
| H | Hide Selected (objects) | Hand |
| F | Frame Model | Fit the sticker in the canvas (Shift+F keeps Frame Selection for the 3D half) |
| T | Texture on Model | unchanged (falls through; it acts on the 3D half) |
| S, R | Stamp, Transform tool | blocked (`NOT_IN_STICKER`): a paint tool cannot become active in Sticker mode |
| Delete / Backspace | Delete (stamp / layer, as today) | with a selection: clears the selected pixels of the active picture layer; without one: nothing, and the status line says "Select an area first, or use Delete Layer". **Never deletes the stamp or a texture layer** |
| Esc | Cancel | cancels a busy tool or leaves a text field; else drops the selection; else leaves the Sticker view |
| Enter | (as today) | applies a crop / closes a lasso; otherwise nothing |
| Ctrl+J, Ctrl+M, Ctrl+], Ctrl+[ | texture layer: Duplicate, Merge Down, move | the same commands on the active **sticker** layer; never on a texture layer |
| Ctrl+A, Ctrl+D, Ctrl+Shift+I | (as today / unbound) | Select All, Select None, Invert Selection |
| [ ] Shift+[ Shift+] D Shift+X | brush size / hardness, colours | unchanged (the sticker brush uses the same settings) |
| 1, 2, 3, F5 | view modes | unchanged (they leave the Sticker view); 4 = Sticker in both |

Tests (P6a, extended in P7a, P7b and P8 as tools arrive): `STICKER_SHORTCUTS` has no chord twice; for **each** colliding chord of the table one dispatch test per mode asserts the action that
runs (V, W, H, F, X, B, E, G, I, S, R, Delete, Esc, Ctrl+J, Ctrl+M); Delete in Sticker mode with a stamp selected leaves the
stamp. The pen: `pen::eraser_override` gets a Sticker-mode branch (it dispatches `AppAction::SetStickerTool(Eraser)` and back
instead of `SetTool`; `pen/mod.rs` and `app.rs:322`), with its own unit test.

Commands (Sticker menu in the menu bar, also in the Tool tab where they belong; each is an `AppAction` and one history entry):

- **Cut-out:** Remove Background (dialog-less: applies with the Tool tab's options Tolerance / Follow gradients / Also enclosed
  areas / Into mask; one click, adjustable by undo + redo with other numbers), Colour to Alpha... (pick the colour with the
  eyedropper or White / Black buttons; live preview while the small popover is open), Defringe (width 1..=10),
  Remove White Matte, Remove Black Matte.
- **Filter (new in revision 2, review B11):** Blur... (Gaussian, radius 0.5..=64 px, live preview while the popover is open)
  and Sharpen... (unsharp mask: amount 0..=300 %, radius 0.5..=16 px), on the active picture layer or its selection; one
  entry each (4.9).
- **Selection:** All, None, Invert, Grow / Shrink (px), Feather (px), Color Range... (pick a colour, tolerance slider: every
  matching pixel of the layer or of all layers, i.e. the wand with Contiguous off, live outline), Delete (clears selected
  pixels of the active layer), Crop to Selection, Layer via Copy, Layer via Cut, Selection from layer alpha.
- **Sticker:** Trim (artboard = bounds of visible content; option "padding" px, default 0), Add Padding..., Canvas Size...,
  Resize... (percent or pixels; scales artboard, layer transforms, text sizes, shape sizes and style lengths; no picture is
  resampled), Rotate 90 CW / CCW / 180, Rotate... (any angle: rotates every layer's transform about the artboard centre, then
  the artboard becomes the bounding box), Flip Horizontal / Vertical, Flatten (all layers -> one picture), Export Picture...
- **Layer:** New Picture Layer, Add Picture... (file dialog), Duplicate, Delete, Merge Down, Rasterize, Clip to Layer Below,
  Bring Forward / Send Backward, Copy Layer Style / Paste Layer Style / Clear Layer Style.

### 3.4 Selection (`sticker/select.rs` in core, state in the app)

`Selection { mask: AlphaBuf /* artboard size */, bounds: IRect }`. Brush, eraser, fill, magic eraser, delete and destructive
commands are limited by it (strength multiplied by `mask / 255`). Drawn as a dashed outline (marching ants need continuous
repaint: draw a static two-colour dashed outline instead, so the reactive frame loop stays idle). The selection is cleared when
the artboard changes size.

### 3.5 Live 3D update

- Every edit goes through `Document` commands with `Commit::Preview` while a control is dragged and `Commit::Final` on release;
  the existing preview / deferred-final machinery re-bakes the users (`stamp_bakes.rs`).
- **During a brush stroke (review A6).** Contour, stroke, shadow and glow depend on the whole alpha of a layer (a distance
  transform), so they cannot be updated in a dirty rectangle. Rules:
  - The canvas is a composite of cached layer rasters. A dab recomposes only its dirty rectangle from: the cached rasters
    of the layers below and above, and the **active layer's content without its styles** (content, adjustments, mask,
    transform: all local operations).
  - If the active layer has an enabled style, `sticker_stroke_begin` switches the canvas to "stroke mode": one full
    recomposite with that layer unstyled (once, budget 8.4), and the header shows "styles update when you lift". Layers
    above that are **clipped** to the active layer are recomposited in the dirty rectangle too (clipping is local).
  - The model: at most every 100 ms a job renders the sticker **with** styles at Preview scale and the result is baked as a
    Preview (`rebake_sticker_users(id, Preview)`); a job still running is not queued twice.
  - At the stroke's end: the full restyle and the Final bake run on a job (2.7); the canvas leaves stroke mode when the
    restyled raster arrives (the stale styled raster is never shown in between: the unstyled one stays until then).
  - A layer without enabled styles needs none of this: dirty rectangles only.
- **A link update while this sticker is open (review B5):** 5.6.
- A tray sticker without users has nothing to bake: the 3D half shows the model unchanged and a hint "Not placed yet".
- Budgets in 8.4. Sticker code that runs in a frame is wrapped in `frame_profile` spans named `sticker.*`
  (`sticker.canvas`, `sticker.render`, `sticker.styles`, `sticker.link`), so the frame log can be checked (8.4).

### 3.6 Edit before placing (E4)

`StampSettings` gets `source: StampToolSource { None, Blob(BlobId), Sticker(StickerId) }` (replacing `Option<BlobId>`; all users
updated) and `styles: StyleStack` (the styles new stamps get). The Stamp tool options show the thumbnail, "Edit before
placing" (opens the Sticker view on `ToolSource`; a blob source is first turned into a tray sticker with `keep = true`), and the
style presets. Placing a `Blob` source with default styles creates exactly today's `StampSource::Image` stamp.

### 3.7 Copy / paste style, apply to all selected (E5)

- Sticker tab buttons and Edit menu: **Copy Style** (the selected stamp's `styles`, or in Sticker mode the active layer's),
  **Paste Style** (onto every stamp linked to the selected layers of the Layers list via `Session::layer_selection()`, one
  `set_stamps_styles` entry; in Sticker mode onto the selected sticker layers), **Clear Style**.
- The clipboard for styles is app state (`AppState::style_clipboard`), not the OS clipboard.
- "Apply to all selected" for one slider: while several stamp layers are selected, the Sticker tab's style editor edits all of
  them (values shown are the active one's; a mixed-values hint appears when they differ).

### 3.8 Sticker tray, library, export (E7)

- **Tray** (Sticker tab): thumbnails of every sticker of the project (`Document::stickers`), unplaced ones included. Click =
  make it the Stamp tool's source (and switch to the Stamp tool); buttons Edit, Duplicate, Delete (only without users), Save to
  Library, Export Picture. "New Sticker" (blank 512 x 512 or from a picture file / clipboard).
- **Library**: a folder of `.wtes` files, default `%APPDATA%\WrappedTextureEditor\stickers\` (`Prefs::sticker_library_dir`, can
  be changed with "Choose folder..."). The tab lists them by `preview.png` (read lazily on a job, never on the UI thread for
  more than one file per frame). Click = add to this project's tray (`read_wtes` -> `add_sticker`, `keep = true`). Self-test runs
  use a folder inside the run's output directory, never the real one.
- **Export Picture...**: PNG of the finished picture (with stamp styles when started from a stamp), file dialog, default name
  `<sticker name>.png`. **Export as PSD...**: the sticker's layers (the writer of 5.2) without making a link.

### 3.9 The Sticker tab (upper group, fifth tab: Tool | Color | Texture sets | Objects | Sticker)

Always available. Top to bottom: (1) the selected stamp (thumbnail, a small **PSD** drag handle beside it (6.4), name, kind
badge, **Edit Sticker**, **Edit in PhotoCraft**, link status line with Update now / Send to PhotoCraft / Show file / Unlink);
(2) **Sticker style** = the stamp-level `StyleStack` editor with the
preset row and Copy / Paste / Clear (works for image, text and sticker stamps without opening the editor: E3); (3) Tray;
(4) Library. This replaces nothing on the Tool page except that the image stamp row "Image: <name> [Choose...]" gains "[Edit]".

**Five tabs in the strip (review A13; decided here).** `upper_tab_strip` draws one `ui.horizontal` row and returns
`[Rect; 4]`; no overflow rule exists, and five labels do not fit the default 300-pt panel. Decision: **the strip gets a second
row when one row is too narrow.** The function measures the five labels with the strip's font; if their total width with
spacing fits the available width it draws one row, otherwise always the same two rows: `Tool | Color | Texture sets` and
`Objects | Sticker`. It returns `([Rect; 5], strip_height)`; the layout below uses the returned height instead of the
constant `TAB_STRIP_HEIGHT` (which stays the height of one row). No label is renamed, no icon-only tab, and the lower group
is not used (the Layers tab must stay visible beside the Sticker tab). `UpperTab::ALL` has five entries; the index test at
`side_groups.rs:356` becomes `[0, 1, 2, 3, 4]`; the serialized name is `"sticker"` (an older build reading the prefs falls
back to its default tab, as `from_pref_name` does for unknown names; P6b tests this against the P0 behaviour).

The four guarantees of ARCHITECTURE 8.7 (G1-G4) are re-proved with five tabs in one row and in two rows: `side_tabs` at
1280x800, 900x600 and 640x480, and `control_sweep/panel_tests.rs`, are updated keeping each test's intent (every label fully
visible and clickable, the content area never shorter than its documented minimum; if the second row breaks that minimum at
640x480 the builder reports the numbers and the checker decides, the fallback being the lower split moving down by one row).

### 3.10 Pen and tablet companion

- The canvas takes ordinary egui pointer events. Pen pressure, the eraser end (switches to the sticker Eraser while in Sticker
  mode, 3.3) and the barrel button (Secondary: pans the canvas) come from `PenRouter` exactly as in the 2D view. The
  companion's remote pen is the same `PenRecord` path (decision C4).
- **Companion fingers (review A16).** `companion/input.rs` has no list of views; it gets rects from `state.view_rects`. The
  canvas **adds its rect to `state.view_rects`** while Sticker mode is on. What the gesture mapper then sends, and what the
  canvas does with it:
  - `FingerMode::Navigate` (default): one finger over the canvas = a **Secondary** drag after 6 points of travel -> the
    canvas **pans** (Secondary and Middle drags always pan, whatever the tool); two fingers = wheel / zoom events -> the
    canvas zooms about the point. Fingers never paint in this mode; the pen does.
  - `FingerMode::Mouse`: one finger = Primary = the active tool.
- Tablet rules for the editor UI: no feature may be reachable only by hover, right click, double click or a keyboard chord
  (each has a button or menu item; the checker lists them); every hit target is at least 24 pt.
- **File dialogs.** They are native `rfd` dialogs on the PC: they are not mirrored and they block WTE's frame loop while
  open, so the tablet picture stands still (existing behaviour; there is no "dialog open" hint and none is built). Therefore:
  every sticker flow with a file dialog ("Add Picture...", "Export Picture...", "Choose folder...", locating PhotoCraft) has
  a way without one where that makes sense (tray, library, paste, drop), and while a companion client is connected each
  button that opens a file dialog carries the visible suffix "(on the PC)".
- "Edit in PhotoCraft" starts a program on the PC; from the tablet it works but the result is on the PC screen (stated in Help).

### 3.11 Feature tiers

| Tier | Content | Packages |
|---|---|---|
| A (core look, must) | layered sticker model, styles on every stamp, die-cut contour, stroke, drop shadow, glows, overlays, presets, promote, tray, format 4 | P1, P3a, P3b, P4, P6a, P6b |
| B (editing, must) | move / transform (also of selected pixels), brush, eraser, masks, **clipping masks**, wand, colour range, magic eraser, fill, **gradient tool**, **blur and sharpen**, colour to alpha, remove plain background, defringe / matte, selections, crop / trim / padding / resize / rotate / shape crop, adjustments, text and shape layers, bevel, inner shadow, gradient overlay, library, export | P2, P7a, P7b, P8 |
| C (together, must) | PSD reader / writer, linked sticker, watcher, launcher, drop target, clipboard, drag out, **linked texture paint layers** | P5, P9, P10a, P10b, P11, P12 |
| S (stretch) | GrabCut box cut-out for photos (S1); live PNG copy for PhotoCraft's Place Linked, watch folder for PhotoCraft's Image Assets (S2) | S1, S2 |
| Later | 0.2 "Out of scope" | - |

---

## 4. Algorithms

### 4.0 Reference code and licence

The algorithms are reimplemented for RGBA8 `PixelBuf`s. Builders may read these PhotoCraft files as reference (read-only):
`pc020\crates\compose\src\effects.rs` (styles: order `:1-14`, EDT `:104-152`, spread split `:489-495`, shadow `:496-511`, glow
`:513-556`, stroke `:1113-1210`, bevel `:881-921`, margin `:37-56`), `pc020\crates\algo\src\selection.rs` (wand `:86`,
`wand_region` `:174`, expand / contract / feather `:530-585`, polygon `:625`), `pc020\crates\algo\src\erase.rs` (magic eraser),
`pc020\crates\engine\src\layer_menu_cmds.rs:225-272` (defringe, matte), `pc020\crates\compose\src\adjust.rs` (brightness /
contrast `:424-452`, hue / saturation `:268`, levels `:460`, posterize), `pc020\crates\algo\src\transform.rs` (premultiplied
sampling), `pc020\crates\vector\src\shapes.rs`, `PCHEAD:crates/algo/src/color_to_alpha.rs`, and for S1
`pc020\crates\algo\src\segment\{maxflow,gmm,grabcut}.rs`.

Licence rule: because the code is written with that source open, it counts as ported. P0 adds `licenses/photocraft-MIT.txt`
(the full MIT text with the line "Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors"). Neither a licence list
in the About dialog nor licence files in the installer exist today (review A20), so P13 **creates** both: a "Licences..."
button in `dialogs/about.rs` that opens a scrolling text window with the file's text (`include_str!`), and a `[Files]` line in
`installer/WrappedTextureEditor.iss` that installs `licenses\*` into `{app}\licenses`; `README.md` gets a "Licences" section. Every file that
adapts PhotoCraft code starts with `// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).` This
notice is the only place the copyright holder's name appears. No GPL source is involved (critic R5).

Deliberate differences from PhotoCraft / Photoshop, to state in ARCHITECTURE 6.11: strokes use the Euclidean distance (round
corners) instead of the 5x5 chamfer metric; shadows and outer glows are cast by the whole outline shape (artwork + contour +
outside stroke), not by the layer alpha alone; no per-style blend modes (everything source-over); no contour curves, noise or
knockout.

### 4.1 Rendering a sticker (`sticker/render.rs`)

`render(input, scale s, quality) -> PixelBuf` of size `ceil(artboard.w * s) x ceil(artboard.h * s)`:

1. For each visible layer bottom to top, the **layer raster** (a `CroppedBuf` in output pixels):
   a. content pixels: picture (blob or owned) -> apply `adjustments` in order -> multiply alpha by the mask; text ->
      `render_spec` with `size_px * s` (anchor: the frame's `anchor_px` sits at `transform.translate * s`); shape -> 4.8 at
      size `size * s`;
   b. transform into output space (4.7) with `transform` and `s`. Identity transform at `s = 1` copies pixels (no resampling);
   c. `fx::apply_styles(raster, &layer.styles, s)`; the result may be larger than the content.
2. Composite into an output buffer cleared to `[0; 4]` with `blend::blend_row` (the existing seven kernels) and
   `opacity`, clipped to the output rectangle. **Clipping:** a layer with `clip` has its raster's alpha multiplied by the
   alpha of its base layer's raster from step 1b (the base's content shape, before the base's styles and without the
   base's opacity) and is then composited as usual above the base. A hidden base hides its clipped layers.
3. Multiply alpha by the artboard shape's coverage (4.8; `Rect` = nothing to do).

**Trivial fast path (identity contract):** a sticker with exactly one visible layer that is a picture with identity transform,
opacity 1, Normal, no clip, no mask, no adjustments, default styles, picture size = artboard size at origin, shape `Rect`, at `s = 1`
returns the picture's pixels unchanged (the same `Arc<PixelBuf>` content). With default stamp styles its mips are then equal to
`PremulMips::build` of the image, so a promoted image stamp bakes bit for bit as before (test K3-1).

Layer rasters are cached (2.7); a style slider on one layer recomputes that layer and the composite only.

### 4.2 Distances and blur (`sticker/fx.rs`, building on `text/edt.rs`)

- `dist_out(alpha) = distance_to_ink(alpha)`: for every pixel the Euclidean distance to the shape with the sub-pixel edge term
  (existing function; ink = alpha >= 128, edge term from the site's alpha).
- `dist_in(alpha) = distance_to_ink(255 - alpha)`: distance from inside pixels to the outside.
- `nearest_ink(alpha) -> Vec<u32>`: new variant of the same transform returning the index of the nearest ink pixel (needed by
  defringe). Same two passes, storing the site.
- **Tent blur** `tent(buf: &mut [u8] or [u16], w, h, b)`: two box blurs of width `b = max(1, round(width))` per axis (so four
  1-D running-sum passes), outside = 0, integer arithmetic with rounding `(sum + b/2) / b`. Rows and columns are independent
  (rayon over rows / column chunks).
- **Spread split** for shadows and glows: `solid = round(size * spread)` px of dilation (threshold on `dist_out`:
  `clamp(solid + 0.5 - d, 0, 1)`), blur width `size - solid`.

### 4.3 Styles (`fx::apply_styles(base, styles, scale) -> PixelBuf`)

Input: straight RGBA8 `base` (w x h). Output: `(w + 2m) x (h + 2m)` with `m = styles.margin(scale)`; all lengths below are
already multiplied by `scale`. Let `A` = base alpha padded by `p = max(m, 1)` transparent pixels on every side; all fields are
computed on this padded size and the result is cropped to the margin `m` at the end. **The pad is never 0** (review A7): a
fully opaque picture (the usual photo) has no transparent pixel, `dist_in(A) = distance_to_ink(255 - A)` would find no ink and
return `INFINITY` (`edt.rs:21`), and Inside stroke, inner shadow, inner glow and bevel would draw nothing. With the pad, the
outside of the picture counts as transparent, so the picture's own edge is an edge. Any remaining non-finite distance (an
empty field) is treated as "farther than every length".

1. **Contour (die-cut, E1)**, when on: `d = dist_out(A)`; `M0 = clamp(width + 0.5 - d, 0, 1)`. Fill holes (when `fill_holes`):
   flood from the border of the padded buffer through pixels with `M0 < 0.5` (4-connected); every pixel not reached becomes 1.
   Smooth: `r = round(smooth * width)`; if `r >= 1`: tent-blur `M` with width `r`, then sharpen the edge back to about one
   pixel: `M = clamp((B - 0.5) * (r + 1) + 0.5, 0, 1)`. Finally `M = max(M, A)` (the contour never cuts into the artwork).
   This gives round joins (Euclidean), closes dents narrower than about `r`, and one solid backing shape.
2. **Stroke**, when on: `(wo, wi)` = Outside `(w, 0)`, Inside `(0, w)`, Center `(w/2, w/2)`. Outside coverage
   `So = clamp(wo + 0.5 - dist_out(X), 0, 1) * (1 - X)` where `X` = the contour shape `M` when a contour is on, else `A`
   (so an outside stroke runs around the die-cut). Inside coverage `Si = clamp(wi + 0.5 - dist_in(A), 0, 1) * A`.
3. **Outline shape** `S = max(A, M, So)`: what casts shadow and outer glow.
4. **Drop shadow**: offset `(dx, dy) = round(distance * (-cos a, sin a))` (angle = where the light comes from, y down),
   dilate + tent-blur `S` by the spread split, shift by the offset, multiply by `opacity`.
5. **Outer glow**: the same without offset.
6. **Inner shadow / inner glow**: the same on `1 - A` (inner shadow shifted by the offset), multiplied by `A`.
7. **Bevel** (smooth, inner): height `H = tent(A, size)`; normal from central differences `(-dH/dx * depth, -dH/dy * depth, 1)`
   normalised; light `L` from `angle_deg`, `altitude_deg`; `shade = n . L - sin(altitude)`; highlight coverage
   `max(shade, 0) * opacity * A`, shadow coverage `max(-shade, 0) * opacity * A`.
8. **Compose** source-over, bottom to top: drop shadow colour; outer glow colour; contour colour with `M * opacity`; outside
   stroke with `So * opacity`; the artwork, where its RGB is first mixed toward the gradient overlay (4.3a) then the colour
   overlay by their opacities (alpha kept); inner glow; inner shadow; bevel highlight then shadow; inside stroke with
   `Si * opacity`.

4.3a Gradient overlay: `t` = position of the pixel along the direction `angle_deg` across the content box, 0..=1, clamped;
colour = linear interpolation of the two neighbouring stops in straight sRGB bytes.

Disabled (`on: false`) entries are skipped and do not count for the margin. An empty or fully transparent base returns a
transparent picture of the padded size.

### 4.4 Colour-tolerance regions (`sticker/select.rs`, `cutout.rs`)

- **Match:** `diff(p, q) = max(|dr|, |dg|, |db|, |da|)` on straight RGBA8, with the colour of a pixel whose alpha is 0 ignored
  (two fully transparent pixels differ by 0; a transparent and a visible one by `|da|`). A pixel matches the seed colour when
  `diff <= tolerance`.
- **Contiguous:** scanline flood fill, 4-connected, from the clicked pixel. **Not contiguous:** every matching pixel.
- **Anti-alias:** pixels that do not match but touch the region (8-neighbourhood) get the partial value
  `round(255 * clamp(1 - (diff - tolerance) / max(tolerance, 16), 0, 1))`; region pixels get 255.
- **Sample all layers:** the region is computed on the sticker's composite (without styles), else on the active layer's content
  mapped to artboard space.
- Wand = this as a `Selection`. Magic eraser: `alpha' = alpha * (255 - region) / 255` (or the same on the mask). Fill:
  source-over of the colour with strength `region / 255`.
- Selection modify: grow / shrink by `n` = thresholds on `dist_out` / `dist_in` of the mask; feather = tent blur of width `n`;
  invert = `255 - m`. Marquee ellipse and lasso polygon are rasterised with the coverage rule of 4.8.

### 4.5 Cut-out helpers (`sticker/cutout.rs`)

- **Colour to alpha** (colour `C`, thresholds `t0 = 0`, `t1 = 1`, all values 0..=1): per channel
  `a_i = (P_i - C_i) / (1 - C_i)` if `P_i > C_i`, `(C_i - P_i) / C_i` if `P_i < C_i`, else 0; `a = max_i a_i`;
  `a' = clamp((a - t0) / (t1 - t0), 0, 1)`; if `a' > 0`: `P'_i = clamp((P_i - C_i) / a' + C_i)`; new alpha = `a' * alpha`.
  A pixel equal to `C` becomes fully transparent; soft edges and shadows keep their softness.
- **Remove white matte:** `C = clamp((P - (1 - a)) / a)`; **remove black matte:** `C = clamp(P / a)`; pixels with `a = 0` are
  left; alpha is unchanged.
- **Defringe (width n):** edge pixels = pixels with alpha > 0 whose `dist_in` is `<= n`; interior = pixels with alpha >= 250
  and `dist_in > n`. Each edge pixel takes the RGB of its nearest interior pixel (`nearest_ink` on the interior mask); alpha is
  unchanged. Without any interior pixel nothing changes.
- **Remove background (plain backgrounds):**
  1. Border sample: the pixels of the 1-px frame of the layer. If at least half of them are transparent, report "This picture
     already has a transparent background" and change nothing.
  2. Background colours: quantise border colours to 4 bits per channel, take the up to 3 most frequent cells that each hold at
     least 15 % of the border, colour = the mean of the cell's pixels (ties by lower cell index).
  3. Region: flood (4-connected) from every border pixel that matches one of the colours within `tolerance` (default 40),
     comparing each pixel with the background colour (not with its neighbour). Option **Follow gradients**: a pixel is also
     accepted when it differs from the already accepted neighbour by at most `tolerance / 4`. Option **Also enclosed areas**:
     afterwards every pixel matching a background colour within `tolerance / 2` is added (letter holes).
  4. Soft edge: in the band of 2 px outside the region, apply colour to alpha against the nearest background colour, so
     anti-aliased edges lose the background tint instead of keeping a halo.
  5. Apply to alpha, or to the mask when "Into mask" is on.
  Deterministic; no model, no network.
- **GrabCut box (S1 only):** 5-component full-covariance GMMs for foreground and background (deterministic k-means start),
  8-neighbour graph with contrast-sensitive weights (`gamma = 50`), Boykov-Kolmogorov max-flow, 5 iterations, computed on a copy
  scaled to at most 512 px, then one re-cut at full size in a band of 6 px around the result; soft edge as step 4.

### 4.6 Adjustments

- Brightness / Contrast: brightness = a line of slope `1.375^(b / 50)` through the origin rolled off smoothly to (1, 1);
  contrast = a symmetric cubic-Hermite S-curve about 0.5 with centre slope `1 + c / 128` and end slope `1 - c / 128` (clamped at
  0); baked into one 256-entry LUT.
- Levels: `v = clamp((x - in_black) / (in_white - in_black))^(1 / gamma)`, then `out_black + v * (out_white - out_black)`; LUT.
- Hue / Saturation: RGB -> HSL; hue + `hue_deg`; saturation `s * (1 + sat)` for `sat <= 0`, `s + (1 - s) * sat` above;
  lightness toward black (negative) or white (positive) linearly; Colorize sets the hue to `hue_deg` (mod 360) and the
  saturation to `0.25 + 0.75 * max(sat, 0)` before the lightness step. `f32`, fixed operation order, round half up.
- Invert `255 - x`. Black & White: `y = round(0.299 r + 0.587 g + 0.114 b)`. Posterize `n`: `floor(x * n / 256) * 255 / (n - 1)`
  rounded. Threshold: luma `>= level` -> white, else black.

### 4.7 Resampling (`sticker/resample.rs`, `pixel_ops.rs`)

Affine layer transforms use inverse mapping per output pixel centre, premultiplied bilinear sampling, transparent outside.
When the scale on an axis is below 0.5, the source is first reduced with the exact box filter chain (`PremulMips`, choose the
level whose scale is closest to but not below the needed one), then sampled bilinearly. Pure translations by whole pixels and
flips copy pixels. Output is un-premultiplied with round-half-up. "Flatten", "Rasterize" and "Merge Down" bake through this
path; "Resize..." of the sticker resamples nothing (it scales transforms).

### 4.8 Shapes and coverage (`sticker/shape.rs`)

Coverage of a pixel = `clamp(0.5 - sd, 0, 1)` where `sd` is the signed distance (px) from the pixel centre to the shape edge:
rectangle and rounded rectangle exact; ellipse by the first-order estimate `(f - 1) / |grad f|` with
`f = sqrt((x/a)^2 + (y/b)^2)`; polygon and star as the signed distance to the edge list; line as a capsule; lasso polygons with
the non-zero rule and edge distance. A shape outline is `clamp(width / 2 + 0.5 - |sd|, 0, 1)`. Gradient fill as 4.3a.

### 4.9 Blur, sharpen, gradient tool (`sticker/filter.rs`; review B11)

- **Blur (Gaussian, radius r):** three box blurs per axis with the widths of the standard "three boxes for a Gaussian of
  sigma = r" rule (integer widths from sigma, fixed order), on **premultiplied** 16-bit intermediates so colour does not
  bleed out of transparent pixels; edges replicate the border pixel; un-premultiply with round-half-up. Reuses the running-sum
  passes of the tent blur (4.2). On a mask: the same on one channel.
- **Sharpen (unsharp mask):** `out = clamp(src + amount * (src - blur(src, radius)))` per colour channel on straight RGB of
  pixels with alpha > 0; alpha unchanged.
- **Gradient tool:** for each pixel, `t` = the projection of the pixel centre on the line press -> release, 0..=1, clamped;
  colour from the stops (4.3a); source-over onto the layer with `opacity`, multiplied by the selection. On a mask the luma of
  the colour is written.
- With a selection the result is mixed with the original by `mask / 255` per pixel. All three go through
  `replace_sticker_pixels` (one tile-patch entry).

---

## 5. The PhotoCraft link

### 5.1 What the user does

1. Select a sticker (or an image stamp; a text stamp is first converted, with a confirmation). Click **Edit in PhotoCraft**
   (Sticker tab, canvas header, Layer menu, row context menu).
   - The project must have a file: on a never-saved project WTE says "Save the project first, so the linked file has a place
     next to it." with **Save...** / Cancel (5.3).
   - A picture used by several stickers: the question of 3.1 (**Edit all** / **Only this one**) comes **now**, before any file
     is written.
2. First time only: if WTE cannot find PhotoCraft, it asks once "Where is photocraft.exe?" (file dialog) and remembers it.
3. PhotoCraft opens the sticker's PSD (v0.2.0 and portable copies: a new window; the installed latest: a tab in the running
   window). The sticker shows a "linked" badge.
4. Edit, press **Ctrl+S** in PhotoCraft. Within about a second (or when WTE's window becomes active) the sticker on the model
   updates: one undo step "Update Linked Sticker". Again and again. WTE does not come to the front, does not flash its taskbar
   button and makes no sound; questions (conflict, "changed while closed") are dialogs **inside** WTE's window that wait there.
5. Menu on the badge: **Update now**, **Send to PhotoCraft** (write WTE's current sticker into the file, 5.5), **Show file in
   Explorer**, **Edit in PhotoCraft**, **Unlink** (keep the picture, stop following; the PSD stays on disk).

The same five steps work for a **texture paint layer** ("Edit Layer in PhotoCraft"): 5.9.

### 5.2 PSD reader and writer in the core (`crates/wte_core/src/psd/`, ARCHITECTURE new 5.28, 7.9)

`export/psd.rs` (the texture exporter) is **not changed**: its bytes are covered by a golden test and by the identity rule.
The new module shares `export/packbits.rs` and may copy small helpers.

**Reader** `psd::read(bytes: &[u8], limits: &PsdLimits) -> Result<PsdDoc, PsdError>`:

```rust
pub struct PsdDoc {
    pub size: [u32; 2],
    pub depth: u8,                       // 8 or 16 in the file; pixels are returned as 8-bit
    pub has_real_merged: bool,           // image resource 1057 (version info), byte 4; true when the resource is missing
    pub merged: Option<PixelBuf>,        // straight RGBA8, un-matted; None when not real, absent or over the limits
    pub layers: Vec<PsdLayer>,           // bottom to top
}
pub struct PsdLayer {
    pub name: String,                    // `luni` when present, else the Pascal name (Windows-1252)
    pub lyid: Option<u32>,
    pub rect: IRect,                     // may reach outside the canvas; clipped when used
    pub pixels: Option<CroppedBuf>,      // straight RGBA8; None for layers without pixel channels
    pub mask: Option<PsdMask>,           // the user mask (channel -2)
    pub clipped: bool,                   // clipping byte != 0
    pub visible: bool,
    pub opacity: u8,
    pub blend: PsdBlend,                 // Known(BlendMode) | Other([u8; 4])
    pub features: PsdFeatures,           // bitflags, below
}
pub struct PsdMask { pub rect: IRect, pub default: u8, pub pixels: AlphaBuf, pub disabled: bool }
```

Supported subset (everything else is `PsdError::Unsupported(&'static str)` with a sentence for the user):

| Part | Reader behaviour |
|---|---|
| Header | signature `8BPS`; version 1 (PSD) and 2 (PSB); mode RGB (3) or Grayscale (1, expanded); depth 8 or 16; 32-bit float, CMYK, Lab, Indexed, Multichannel, Duotone: Unsupported |
| PSB lengths | 8-byte lengths for the layer-and-mask section, the layer info, each channel's data length, and the tagged blocks with these **13** keys (`pc020\crates\psd\src\tagged.rs:15`): `LMsk`, `Lr16`, `Lr32`, `Layr`, `Mt16`, `Mt32`, `Mtrn`, `Alph`, `FMsk`, `lnk2`, `FEid`, `FXid`, `PxSD`; 4-byte RLE row counts. Every other key keeps a 4-byte length |
| Limits | side <= 30000 to parse; pixels are decoded only when the canvas is <= 8192 a side and <= 67 megapixels, else `merged = None`, layers without pixels, and the caller reports "too large" |
| Image resources | only 1057 is read (`hasRealMergedData`). Everything else skipped by length |
| Layer info | in the layer section, or in the `Lr16` global block for 16-bit files. Negative layer count = the first alpha of the merged image is transparency |
| Layer record | rect; channels (ids -1 transparency, 0..2 colour, **-2 user mask: read into `mask`**, -3 real user mask: skipped, flagged MASK_EXTRA); blend key; opacity; clipping byte -> `clipped`; flags (bit 1 = hidden); layer mask data (rect, default colour, flags: bit 1 = disabled; a second "real" mask, or mask parameters density / feather -> MASK_EXTRA); blending ranges: **empty, or `(channel count + 1)` times the default pair `00 00 FF FF 00 00 FF FF`** (`pc020\crates\io\src\blocks.rs:36`) are the default, anything else -> BLEND_IF; Pascal name; tagged blocks |
| Tagged blocks | `luni`, `lyid` read; `lsct` / `lsdk` -> GROUP; `lfx2` / `lrFX` / `lmfx` -> EFFECTS; `TySh` -> TEXT; `SoLd` / `PlLd` / `SoLE` -> SMART; `vmsk` / `vsms` -> VECTOR_MASK; `SoCo` / `GdFl` / `PtFl` -> FILL; `levl` `curv` `hue2` `brit` `blnc` `expA` `vibA` `blwh` `phfl` `mixr` `clrL` `nvrt` `post` `thrs` `grdm` `selc` -> ADJUSTMENT; `iOpa` < 255 -> FILL_OPACITY; unknown keys skipped by length |
| Channel data | compression 0 raw, 1 PackBits, 2 ZIP, 3 ZIP with prediction (inflate with `miniz_oxide`, delta decode per row; 16-bit prediction on big-endian words) |
| Merged image | planar, compression 0, 1, **2 or 3** (HEAD has `PsdExportOptions::merged_compression`; ZIP here is one stream per file section, the builder checks the layout against `PCHEAD:crates/psd` and builds the test file with the same layout); channels: colour, then transparency only if the layer count was negative (a flat file without layers: 4th channel is treated as transparency too), further channels (saved alphas, quick mask) ignored. With transparency the colours are **un-matted from white**: `C = clamp((P - (max - a)) * max / a)` per channel for `a > 0`, 0 for `a = 0` (both PhotoCraft versions and WTE's own writer matte on white). 3 channels = opaque |
| 16-bit | every sample is kept as 16-bit through channel decoding **and through un-matting** (`max = 65535`), and only then reduced to 8-bit with `(v + 128) / 257`. Un-matting after the reduction would lose a second time |
| Robustness | every length is checked against the remaining bytes (`PsdError::Truncated`): a half-written file never panics and never yields a picture |

`PsdFeatures`: `EFFECTS, GROUP, ADJUSTMENT, VECTOR_MASK, MASK_EXTRA, FILL_OPACITY, BLEND_OTHER, BLEND_IF, NO_PIXELS, TEXT,
SMART, FILL`. Two groups:

- **Forcing** (WTE cannot draw them; one of them on any visible layer makes the whole file "Merged", 5.4): EFFECTS (only when
  the effects are enabled), GROUP, ADJUSTMENT, VECTOR_MASK, MASK_EXTRA, FILL_OPACITY, BLEND_OTHER, BLEND_IF, and NO_PIXELS on a
  layer that is not a group marker.
- **Not forcing** (review B3): TEXT, SMART and FILL on a layer **with** pixel channels. PhotoCraft renders these layers into
  their pixel channels at every save, so the pixels are current; WTE takes them as picture layers. A user mask (`mask`) and
  `clipped` are not features at all: WTE has both.

`PsdDoc::forcing() -> PsdFeatures` (the union over visible layers) and `PsdDoc::pc_native() -> bool` (some layer has TEXT,
SMART or FILL) are what `link.rs` asks.

**Writer** `psd::write_sticker(w, &StickerPsd) -> Result<(), PsdError>`: 8BPS v1, RGB, 8-bit, canvas as given; image resource
1057 with `hasRealMergedData = 1`; one record per layer in order: rect, channels -1, 0, 1, 2 (and -2 when the layer has a
mask) PackBits, blend key (`norm`, `mul `, `scrn`, `over`, `lddg` for Add, `dark`, `lite`), opacity, clipping byte, hidden
flag, layer mask data (rect, default 255), default blending ranges, `luni` + `lyid` (the id given by the caller); merged image
= the composite of exactly those layers (masks and clipping applied), matted on white when not opaque (3 channels when opaque,
as PhotoCraft does). **Layer pixels are normalised before writing** (review A11): the RGB of every pixel with alpha 0 is set
to 0 and the rect is trimmed to the bounds of alpha > 0 (an empty layer is written with an empty rect), which is exactly what
PhotoCraft's `Surface::content_bounds` trim produces at its own save (`pc020\crates\io\src\psd_export.rs:87`). Bytes are
deterministic. Round-trip test against `psd::read` and against the old strict reader.

### 5.3 Link data (`link.rs`; ARCHITECTURE new 5.28, 6.13)

```rust
pub struct StickerLink {
    pub file: LinkFile,                  // InFolder(String /* file name in the project's links folder */) | External(PathBuf)
    pub mode: LinkMode,                  // Layers | Merged
    /// PSD pixels per layer-space pixel (review B4): 1, 2 or 4 when WTE wrote the file; any value >= 1 for a PhotoCraft
    /// canvas that had to be fitted into the artboard limit (5.4). PSD canvas = artboard size * scale.
    pub scale: f32,
    /// Size, modification time and CRC-32 of the file content WTE last wrote or applied.
    pub synced: FileStamp,
    /// Hash of the sticker value that corresponds to `synced`: every non-pixel field by value, pictures and masks by the
    /// CRC-32 of their bytes (computed once per picture version and cached; this is link data, not a cache key).
    pub synced_sticker: u64,
    /// PSD layer id <-> sticker layer, with what WTE wrote or read for it (normalised as in 5.4).
    pub layer_map: Vec<LinkLayer>,       // { lyid: u32, layer: StickerLayerId, origin: [i32; 2], size: [u32; 2], pixels_crc: u32, mask_crc: Option<u32> }
    pub next_lyid: u32,                  // above every lyid ever written or read for this link
    /// The file holds PhotoCraft's own text / smart / fill layers; a rewrite from WTE would turn them into pixels (5.5).
    pub pc_native: bool,
}
```

`Document::links: BTreeMap<StickerId, StickerLink>`; view-like state: not in history snapshots, changes do set the modified
flag. Manifest `LinkEntry` mirrors it plus `archive: Option<String>` (`links/k<id:05>.psd`).

Session state that is **not** saved (`wte_app` `link/mod.rs`, `LinkSession` per link): the launched child if any, `opened_at`
(a launch happened this session), `stale_in_pc: bool` (5.5), `alias: Option<PathBuf>` (below), `held: Option<LinkEvent>` (5.6).

**A link needs a saved project (reviews A10, B8; this replaces the earlier "session folder").** There is no temporary links
folder. "Edit in PhotoCraft", the PSD drag handle and "Edit Layer in PhotoCraft" on a never-saved project ask to save first
(5.1). Reason: a link file that moves while PhotoCraft has it open (first save, or a session folder that dies with the
process) makes PhotoCraft's next Ctrl+S land where nobody looks. `External` links (5.8) need no folder and work in an unsaved
project.

**Folder.** Project `X.wtep` -> `X_links\` beside it; file names `<sanitised sticker name>_k<id>.psd` (paint layers:
`<layer name>_l<id>.psd`). Self-tests save their project into the run's output folder first.

**Who writes the PSD, and when (review B1).** Only three things write a link file: making the link, an explicit **Send to
PhotoCraft**, and "Edit in PhotoCraft" when WTE holds newer content (5.5). **A project save never rewrites a link file.**
Every write is atomic (temp file + rename in the same folder), keeps the old file as `<name>.bak.psd` (one generation), and
registers the new content's CRC-32 in the watcher's `own_writes` set **before** the rename (5.6).

**At project save** each `InFolder` link's file is stored as `links/k<id>.psd` in the `.wtep` so that moving the project keeps
it (streamed; never a blob: R6). Rule (review A26): read the file; if its CRC equals `synced.crc`, store it. If it differs
(PhotoCraft saved something WTE has not applied yet, or is writing right now), store it only when `psd::read` accepts the
bytes; otherwise keep the previous archive entry (copied from the old `.wtep`; none on a first save) and show one warning
"<name>: the linked file is being written; the project keeps the earlier copy."

**At Save As** (review A10): the link files are copied to `<new>_links\` and the links are re-pointed. For every link with
`opened_at` set, PhotoCraft may still have the **old** file open and will save there. So the old path is kept as the link's
`alias` for the rest of the session: the watcher follows both paths; a settled change at the alias is first copied over the
new file (registered in `own_writes`) and then handled as an ordinary change. The alias is dropped when the launched child is
known to have exited and no change arrived, or when WTE closes. The Save As dialog result toast names such links once:
"<names> may still be open in PhotoCraft from the old folder. WTE keeps following them until you close WTE."

**At load** (core `project::load` only reports; it writes nothing to disk): `LoadReport::links: Vec<LinkAtLoad { sticker, state:
Present | Missing | Changed }>`. The app's `io_flows` then (a) restores a `Missing` `InFolder` file from the archive entry,
(b) for `Changed` (file stamp differs from `synced`) shows once "Linked sticker <name> changed while the project was closed:
**Update** / **Ignore**" (an in-window dialog). `External` links are never copied or archived; a missing external file shows
"link broken" with Relink / Unlink.

**Closing or discarding (review B8).** With a saved project, PhotoCraft's later saves always land in `X_links\` and are found
at the next load as "changed while closed"; nothing is lost. The one gap is a link **made since the last project save** when
the user closes without saving: the `.wtep` does not know that link. So the "unsaved changes" prompt (close, New, Open) adds a
line when such links exist: "Stickers linked to PhotoCraft since the last save: <names>. If you do not save, WTE will not
follow them any more; the PSD files stay in <X_links>." WTE never deletes a file in `X_links\` by itself (no cleanup rule;
unreferenced files are left for the user).

### 5.4 Mapping rules (the round trip; implemented in `link.rs`, pure, unit-tested)

**WTE -> PSD** (`link::export(doc_input, scale) -> (StickerPsd, layer_map)`): one PSD pixel layer per sticker layer, bottom to
top.

- **Scale (review B4).** The PSD is written at the link scale `s` = the larger of the sticker's Final `sticker_scale` (2.6)
  and `ceil(1 / smallest picture-layer scale)`, from {1, 2, 4}, lowered until the canvas `artboard * s` is at most 4096 a
  side. So text and shapes arrive in PhotoCraft as sharp as the model shows them, and a picture that WTE shows shrunk up to
  4x keeps its pixels. `s` is stored in `StickerLink::scale`.
- **Pixels** = the layer raster of 4.1 steps a-b at scale `s` (content, adjustments, transform; **without layer styles and
  without the mask**), clipped to the canvas, normalised (5.2). A picture layer's **mask** is written as the PSD layer mask
  (transformed the same way); `clip` is written as the clipping byte. Name, visibility, opacity, blend mode carried over.
- **`lyid` is stable (review A11):** a layer that is already in `layer_map` keeps its `lyid` for ever; a new layer gets
  `next_lyid` (then incremented). The first write therefore gives `lyid = 1, 2, ...` bottom to top, and later writes never
  renumber. Layers PhotoCraft created keep the id PhotoCraft gave them.
- **`lyid` reuse by PhotoCraft (P0 check, fixture f12).** PhotoCraft (v0.2.0 and HEAD) numbers a layer without a stored id
  with the **lowest free** id, not one above the largest. Deleting the WTE layer `lyid 2` in PhotoCraft and adding a layer in
  the same edit gives the new layer `lyid 2`. Read by the table below alone, that is "`lyid` in `layer_map`, the pixels
  differ": the deleted WTE layer would be replaced by the new picture and keep its WTE layer styles (a text's stroke drawn
  around the new PhotoCraft picture). P5 must not map such a layer onto the old one; suggested rule (P5 decides and
  records it): a known `lyid` whose pixels differ **and** whose name differs from the name WTE wrote is treated as "a mapped
  WTE layer no longer in the PSD" plus "unknown `lyid`" (deleted + new). A k5 test on f12 proves the chosen rule.
- The stamp's styles and the layer styles are not part of the file.

What PhotoCraft therefore shows: the artwork without WTE's styles. This is deliberate: a style baked into pixels could not be
re-applied correctly after the pixels are edited (an erased part would keep its old shadow). WTE re-applies layer styles and the
stamp styles to whatever comes back. Stated in the Edit dialog the first time and in Help.

**PSD -> WTE** (`link::import(&PsdDoc, &StickerDoc, &StickerLink) -> LinkImport`).

*Comparison rule (review A11).* PhotoCraft re-trims every layer at save, and RGB under alpha 0 is not preserved. "The pixels
are what WTE wrote" therefore means: normalise the PSD layer (zero RGB where alpha is 0, trim to alpha > 0), then compare
`size` and the CRC-32 of the normalised bytes with `layer_map`. The trimmed `origin` is compared separately: equal bytes at
another origin = the layer was **moved** in PhotoCraft. Masks are compared the same way over the layer's rect.

*Layers mode* (`psd.forcing()` is empty), per PSD layer bottom to top:

| PSD layer | Result in the sticker |
|---|---|
| `lyid` in `layer_map`, pixels and mask equal, same origin | the WTE layer is kept **as it is** (a text stays text, a shape stays a shape; transform, mask and adjustments stay live); only name, visibility, opacity, blend, `clip` and order are taken from the PSD |
| the same, but another origin | kept as it is and **moved**: `transform.translate += (new origin - old origin) / s` |
| `lyid` in `layer_map`, pixels equal, the **mask** differs (added, painted, removed) | a picture layer whose transform is a whole-pixel move at `s = 1`: kept, its mask replaced by the PSD mask (mapped back into picture space). Any other layer (text, shape, transformed picture): becomes a picture layer as in the next row, **with** the PSD mask as its mask |
| `lyid` in `layer_map`, the pixels differ | the layer becomes a picture layer with the PSD pixels: content = the PSD pixels, transform = scale `1 / s` at `origin / s` (so no pixel is lost and nothing is resampled until render); its adjustments are removed (they were part of the pixels WTE sent); the PSD mask, if any, becomes its mask; its WTE layer styles are kept. **A text or shape layer loses its editability, only this layer** |
| unknown `lyid`, pixels present (a layer made in PhotoCraft: painted, pasted, **a PhotoCraft text layer, smart object or fill layer with pixels**) | a new picture layer (same placement rule), with its mask and `clip`; `pc_native = true` when it was TEXT / SMART / FILL |
| a mapped WTE layer that is no longer in the PSD | deleted |
| a mask marked disabled in the PSD | imported as "no mask" for the picture; the map remembers the mask's CRC so the layer is not seen as changed every time |

Every new or replaced layer gets `layer_map` entries from what was read. Canvas size changed in PhotoCraft -> the artboard
becomes `canvas / s` (rounded up) with its top-left at the old top-left (placement refit 2.9).

So the most common things a user does in PhotoCraft (paint on a layer, add a text, add a layer mask, clip a pattern to a
layer, move layers) come back **layer by layer**, and every WTE layer that was not touched stays what it was.

*Merged mode* (`psd.forcing()` is not empty: enabled layer effects, groups, adjustment layers, vector masks, mask density /
feather, fill opacity, blend-if, a blend mode outside WTE's seven, layers without pixels): the sticker's layers are replaced by
**one** picture layer "PhotoCraft picture" holding the un-matted merged image (transform scale `1 / s`); `mode = Merged`. The
PhotoCraft layers stay in the PSD (and so in the project). The user gets one toast that names the reason, for example: "This
sticker now uses layer effects, which only PhotoCraft can show. WTE shows the finished picture; keep editing its layers in
PhotoCraft." The stamp's styles still apply. Undo brings the WTE layers back.

Merged mode needs real merged data. `has_real_merged == false` (latest PhotoCraft with Maximize Compatibility off, critic R1):
no update; a sticky message on the link badge: "PhotoCraft saved this file without a merged picture. In PhotoCraft turn on
Preferences > File Handling > Maximize PSD and PSB File Compatibility and save again." If `photocraft-cli.exe` is known (5.7),
the message has a button "Convert with PhotoCraft's command-line tool" (fallback). Layers mode does **not** need merged data.

**A PhotoCraft canvas larger than a sticker may be (review A18).** `MAX_ARTBOARD` is 4096 layer-space pixels. If
`canvas / s` exceeds it (the user enlarged the canvas or the image in PhotoCraft): nothing is applied; the badge says
"<name> is now W x H in PhotoCraft; a sticker can be at most 4096 px." with **Fit it into the sticker** (one merged picture
layer holding the PSD's pixels, `mode = Merged`, `scale = max(W, H) / 4096`, so the artboard is the canvas divided by that
and no pixel is thrown away) and **Ignore**. Over the reader's pixel limits (8192 a side): the "too large" message only.

**What is lost, in one list** (goes into Help and `dist/HOW_TO_TEST.txt`): WTE layer styles and stamp styles are not visible in
PhotoCraft; a WTE text or shape layer whose pixels are changed in PhotoCraft (or that gets a mask there) comes back as pixels;
a WTE picture shrunk by more than 4x comes back with the pixels of 4x if it is changed in PhotoCraft; PhotoCraft's own text,
smart and fill layers come back as pictures (they stay editable in PhotoCraft as long as WTE does not send the sticker back,
5.5); PhotoCraft effects, adjustment layers, groups and unusual blend modes make the whole sticker come back as one finished
picture; 16-bit files come back as 8-bit; layer parts outside the canvas are clipped.

### 5.5 Sending again, and conflicts

State of a linked sticker, from two comparisons: `file_changed` = the file's stamp differs from `synced`;
`wte_changed` = the sticker's hash differs from `synced_sticker`. The badge texts are fixed (one wording everywhere, also in
scenarios): **"linked"**, **"changed in WTE"**, **"changed in PhotoCraft"** (waiting for Update), **"conflict"**,
**"file missing"**, **"needs a merged picture"**.

| file_changed | wte_changed | What happens |
|---|---|---|
| no | no | in sync ("linked"). "Edit in PhotoCraft" opens the existing file (PhotoCraft's own layers and effects are still in it) |
| yes | no | the watcher applies the file (5.4), unless `stale_in_pc` is set (below) |
| no | yes | "changed in WTE". **Nothing is written by itself, and nothing at project save.** The file is rewritten from the sticker only when the user clicks **Send to PhotoCraft**, or **Edit in PhotoCraft** (which says "Send WTE's changes to the file first?" **Send and open** / **Open the file as it is** / Cancel). When `mode == Merged` or `pc_native`, the question adds: "This replaces PhotoCraft's own layers (text, effects) in the file with plain picture layers." |
| yes | yes | conflict dialog (never automatic): "Both WTE and PhotoCraft changed <name>." **Take PhotoCraft's version** (apply the file; WTE's changes since the last sync are one undo away) / **Keep WTE's version** (rewrite the file, old one to `.bak.psd`) / Decide later (badge "conflict") |

**PhotoCraft still has the old picture open (review B1).** v0.2.0 does not reload a file that changed on disk. If WTE rewrites
the PSD while PhotoCraft has it open, PhotoCraft's next Ctrl+S writes its **old** canvas over WTE's changes, and by the table
that would look like row 2 and be applied silently. Therefore:

- Before any rewrite of a link with `opened_at` set, WTE warns: launched child still running -> "PhotoCraft has this file
  open. Its next save will replace what you send now. Close the file in PhotoCraft first, or reopen it there after sending."
  (**Send anyway** / Cancel); child exited or unknown (the installed build hands the file to its running window and exits) ->
  the same text starting "PhotoCraft may still have this file open."
- After such a rewrite `stale_in_pc = true`. While it is set, the **next change of the file on disk is never applied by
  itself**: it is shown as a conflict with the text "PhotoCraft saved over the changes WTE sent to <name>." and the same
  three answers.
- `stale_in_pc` is cleared by the user's answer to that dialog, or when the user starts "Edit in PhotoCraft" again **after**
  the previous child has exited (a fresh PhotoCraft window reads the new file).

While `wte_changed` is true because of an **undo** of "Update Linked Sticker", nothing is applied automatically (the file
stamp did not change: R6) and, by the rule above, nothing is written at Ctrl+S in WTE: PhotoCraft's work stays on disk. The
badge reads "changed in WTE"; "Update now" applies the file again.

Editing a linked sticker inside WTE is allowed (no lock); the badge then reads "changed in WTE" and the next sync follows the
table.

### 5.6 The watcher (`crates/wte_app/src/link/watch.rs`)

- One worker thread, started when the first link exists, stopped when none is left. It owns a list of `(link id, path, alias
  path, last seen stamp)`, replaced through a channel when links change, and shares `own_writes: Arc<Mutex<HashSet<u32>>>`
  (CRC-32 of contents WTE wrote; at most 64 entries, oldest dropped).
- Loop: `recv_timeout(1000 ms)`; a `Poke` message (sent when the window gains focus and by "Update now") polls at once. Each poll
  reads `std::fs::metadata` (length, modified time) of every file. No directory handles, no FFI.
- A file whose stamp differs from the last seen one is **settled** before it is reported: polled every 250 ms until two
  consecutive polls agree and `psd::read` succeeds on the bytes (PhotoCraft v0.2.0 writes in place, so a half-written file is
  normal; `Truncated` just waits). After 30 s without a readable file: one `Unreadable` event (toast, badge), then quiet until
  the stamp changes again.
- **WTE's own writes are recognised by content, not by timing (review A26):** a settled file whose CRC-32 is in `own_writes`
  or equals the link's `synced.crc` only updates the "last seen" stamp; no event, no repaint. (The watch list is updated
  through a channel, so there is a moment in which the list is older than the file; comparing stamps alone would report
  WTE's own write as PhotoCraft's.)
- Result: `LinkEvent::Changed { link, stamp (incl. CRC-32), psd: Box<PsdDoc> }` into an `mpsc` queue and **one**
  `ctx.request_repaint()`. Parsing and CRC happen on the worker; `App::logic` only drains the queue and calls
  `Document::apply_link_update`. Every repaint request goes through one function that also increments
  `LinkWatcher::repaints: AtomicU64`; with nothing changing the counter does not move (unit test, and the scenario check of
  8.2; a frame counter cannot show this in a self-test because the runner itself repaints every frame).
- **An update for the sticker that is open in the Sticker view (review B5):** if a stroke, a drag or a pending Preview edit is
  open on that sticker, or a text field of it has focus, the event is kept in `LinkSession::held` (`StickerEdit::held_update`)
  and applied in the first frame in which none of these is true (the pending edit is flushed first). After applying: the
  selection is dropped; the active layer stays if its id still exists, else the topmost layer becomes active;
  `selected_layers` keeps only ids that exist; the canvas caches are rebuilt; one toast "Updated from PhotoCraft". The 3D half
  re-bakes as for any `set_sticker`. A conflict (5.5) opens its dialog under the same "not while busy" rule.
- Applying never raises, focuses or flashes the window and plays no sound (no `MessageBeep`, no `FlashWindowEx`, no viewport
  focus command anywhere in `link/`; the checker greps).
- A deleted file -> `Missing` event (badge "file missing", Restore from project / Unlink).

### 5.7 Starting PhotoCraft (`link/launch.rs`)

- `Prefs::photocraft_exe: Option<PathBuf>`. Discovery when unset (review A19: **the registry only, no guessed folders**; the
  installed build's folders contain a name this plan may not write into WTE, rule 9 of 0.4), read-only with `advapi32`
  `RegGetValueW` via `raw-dylib`, `HKCU` then `HKLM`, in this order:
  1. `Software\Classes\Photocraft.PhotoshopDocument\shell\open\command` (`PCHEAD:` installer `photocraft.wxs:83`);
  2. `Software\Classes\Applications\photocraft.exe\shell\open\command` (`photocraft.wxs:109`).
  The quoted exe path is taken from the command line and must exist. If neither gives one: ask once with a file dialog (R8:
  the portable copy leaves no trace), warn (do not refuse) when the file name is not `photocraft.exe`. Help menu >
  "PhotoCraft location..." changes it later. `photocraft-cli.exe` is looked for next to it.
- `trait Launcher { fn open(&self, exe: &Path, file: &Path) -> Result<Launched, String>; }`. The command is built by a pure
  function `launch::command_for(exe, file) -> (OsString, Vec<OsString>)` (the file as **one** argument, never through a shell,
  no quoting by hand); a unit test checks it for paths with spaces, non-ASCII names, a UNC path and a path with `&` / `%`,
  and spawns nothing (review B15). `RealLauncher`: `Command::new(program).args(args)`, stdin / stdout / stderr null,
  `CREATE_NO_WINDOW` set; the `Child` is kept. `StubLauncher` records `(exe, file)` and starts nothing.
  **A self-test run and every `#[cfg(test)]` build construct only the stub** (`AppState::launcher` is chosen in `WteApp::new`
  from `cli.selftest`; a unit test asserts it); there is no environment variable that turns the real one on in a self-test.
- **Two-copies guard (R7):** if the remembered `Child` is still running (`try_wait()` is `Ok(None)`), a second "Edit in
  PhotoCraft" shows "This sticker is already open in PhotoCraft. A second copy could overwrite your changes." with **Open
  anyway** / **Show file** / Cancel. **This guard works for v0.2.0 and portable copies only** (they keep one process per
  window). The installed build's child hands the file to the running window and exits at once, so WTE cannot know the file
  is open: a second click simply starts it again (it opens a second tab or focuses the first; which one is **unverified**,
  see 6.5). Help says exactly this.
- Fallback conversions with `photocraft-cli.exe` (only on the user's click, 5.4): `convert <in.psd> <out.png>` into a temp
  folder, `CREATE_NO_WINDOW`, stdin null, 30 s timeout, on a job thread; the PNG becomes a Merged-mode update.

### 5.8 Other doors (documented, small)

- **Import PSD as sticker**: File > Import > Sticker from PSD..., dropping a `.psd`, or pasting one: `psd::read` -> a new tray
  sticker by the rules of 5.4 with an empty map (layers, or merged when something forces it). Checkbox "Keep linked to this
  file" makes an `External` link.
- **Image Assets folder and the live PNG copy** are in S2.

### 5.9 Linked texture paint layers (P12; review B10: "link photos/layers")

The user asked to link layers, not only stickers. A texture paint layer can be edited in PhotoCraft the same way.

- **What:** Layer menu and row context menu of a raster paint layer: **Edit Layer in PhotoCraft**. Not offered for stamp-linked
  layers (edit the sticker instead) and locked layers.
- **File:** a one-layer PSD at the texture set's size (`<layer name>_l<id>.psd` in `X_links\`): the layer's pixels (straight,
  normalised), opacity 255, Normal (the layer's own opacity and blend stay in WTE and are not sent), merged image = the same
  pixels. Sizes up to the reader's limits (8192 a side); a larger set gets the message "This texture is too large to link."
- **Data:** `Document::layer_links: BTreeMap<LayerId, PaintLink { file, synced, synced_pixels: u64 /* the layer's version */ }>`,
  manifest `links` entries with `"layer": id`, archive `links/l<id:05>.psd`. Same rules as 5.3 (saved project, who writes,
  save, Save As, load, close).
- **Update:** the watcher event calls `Document::replace_layer_pixels(layer, PixelBuf, "Update Linked Layer")` (new core
  command: a tile patch through the existing `Edit::Pixels`, one undo step; caches and views update as after a stroke).
  Source of the pixels: if the PSD has exactly one visible pixel layer and nothing forcing, that layer on a transparent
  canvas; otherwise the merged picture (needs real merged data, same message as 5.4). A PSD whose canvas size differs from
  the texture set is refused: "<name> is W x H in PhotoCraft; the texture is w x h. Undo the size change in PhotoCraft."
- **Conflicts, the guard, the watcher, "Send to PhotoCraft":** 5.5-5.7 unchanged (`wte_changed` = the layer's pixel version
  differs from `synced_pixels`).
- **Paste as New Paint Layer** (Edit menu, also the Layers tab "+" menu): the clipboard picture becomes a new paint layer of
  the active set: same size -> as it is; another size -> scaled to fit with the aspect kept, centred (the resampler of 4.7).
- **What the user sees in PhotoCraft:** the layer's UV texture, flat. This is the natural way to "edit a photo/layer in one
  and move to the other" for texture work; 3D-aware editing stays in WTE.

---

## 6. Drag and drop, clipboard (`crates/wte_app/src/ole/`)

### 6.1 Module layout and rules

| File | Content | Tested by |
|---|---|---|
| `ole/formats.rs` | pure codecs: `CF_DIB` / `CF_DIBV5` <-> `PixelBuf` (BI_RGB 24 / 32, BI_BITFIELDS with alpha mask, bottom-up and top-down, premultiplied-looking data left as is), `DROPFILES` build / parse (wide), `FILEGROUPDESCRIPTORW` parse, `CF_UNICODETEXT` | unit tests with byte fixtures |
| `ole/com.rs` | `raw-dylib` declarations (`ole32`: `RegisterDragDrop`, `RevokeDragDrop`, `ReleaseStgMedium`, `DoDragDrop`, `OleInitialize`; `shell32`: `DragQueryFileW`, `SHCreateStdEnumFmtEtc`; `user32`: clipboard functions, `RegisterClipboardFormatW`, `RegisterWindowMessageW`, `ScreenToClient`, `PostMessageW`, `Get/SetWindowLongPtrW`, `CallWindowProcW`, `GetKeyState`; `kernel32`: `GlobalAlloc/Lock/Unlock/Size/Free`), `#[repr(C)]` vtables for `IUnknown`, `IDataObject`, `IDropTarget`, `IDropSource`, `IEnumFORMATETC`, `IStream` (read only), GUIDs | layout tests (size / offset asserts against the SDK numbers written in comments) |
| `ole/drop_target.rs` | WTE's `IDropTarget` object | in-process test: call its vtable with a fake `IDataObject` |
| `ole/data_object.rs` | WTE's `IDataObject` (a list of `(format, bytes)`), used for drag-out and by the tests as the fake | `GetData` / `QueryGetData` / `EnumFormatEtc` tests |
| `ole/subclass.rs` | WTE's second window-procedure subclass: the drag message and the clipboard key chords (6.3) | message-routing tests on a fake previous procedure |
| `ole/drag_source.rs` | `DoDragDrop`, `IDropSource`, the `DragJob` slot | guard and state-machine tests without calling `DoDragDrop` |
| `ole/clipboard.rs` | open / read / write with retry | codec only; the OS calls are never run by tests |
| `ole/mod.rs` | `install(cc)`, the shared queue (`Mutex<VecDeque<OleEvent>>`), platform-neutral event types; non-Windows stubs | - |

Rules:

- Every callback body in `catch_unwind` (a panic must not cross FFI). Objects that Windows may hold are leaked `Box`es with an
  atomic reference count (never freed while registered). `QueryInterface` answers `IUnknown` and the object's own interface,
  `E_NOINTERFACE` otherwise (never `unimplemented!`).
- **What a callback may do (review A22).** `DragEnter` / `DragOver` / `DragLeave` and the subclass only push small events to
  the queue and call `request_repaint`. **`IDropTarget::Drop` is the exception: it must copy the data before it returns**,
  because the `IDataObject` (and every `HGLOBAL` / `IStream` it hands out) is gone afterwards. Inside `Drop`, synchronously:
  for the best format present, `GetData`, copy the bytes into a `Vec<u8>` (file paths for `CF_HDROP`: only the path strings),
  `ReleaseStgMedium`, then push `OleEvent::Drop { pos, payload }`. Caps: 64 MiB per item, 256 MiB per drop, at most 64 items;
  an `IStream` is read in 1-MiB chunks up to the cap; over a cap that item is dropped and the event carries `skipped: n` (one
  toast). No decoding (PNG, DIB, PSD) happens in the callback; that is done on a job after the event is drained.
- **Message ids.** The drag message is `RegisterWindowMessageW("WTE.OleDrag.v1")` (not a fixed `WM_APP + n`, which another
  subclass on the same window could also use). The pen subclass is not touched.
- `ole::install` runs in interactive runs only; a self-test run installs it only with `WTE_OLE=1` (proves it does no harm;
  what it queues is dropped), like `WTE_PEN_SUBCLASS`. `eframe`'s `with_drag_and_drop(true)` stays (winit has then called
  `OleInitialize`); `install` does `RevokeDragDrop(hwnd)` then `RegisterDragDrop(hwnd, ours)`. If registration fails, WTE
  re-registers nothing and logs it; file drops then do not work (toast once) - the builder checks whether winit's handler can
  be put back and documents the result.
- **The "Drop to open" overlay.** With winit's drop target revoked, egui no longer gets `raw.hovered_files` or
  `raw.dropped_files`, which `dnd.rs:14` reads today. When the OLE target is installed, `dnd.rs` takes both from the queue
  instead: `OleEvent::Hover { pos, kinds }` (what the drag carries: files with their extensions, or picture data) feeds the
  overlay, `OleEvent::Leave` clears it, `OleEvent::Drop` replaces `dropped_files`. Where the target is not installed
  (non-Windows, failed registration, a self-test without `WTE_OLE`) the old egui path stays. One function in `dnd.rs` hides
  the difference; the existing drop tests run against both sources.

### 6.2 Drop target: what WTE accepts

Formats, best first: private `WTE.Sticker.v1` (bytes of a `.wtes`; a drag that started in this WTE process is recognised by a
process id inside and ignored when dropped back on WTE: R3); `CF_HDROP` (file paths); registered `PNG`; `CF_DIBV5`; `CF_DIB`;
`FileGroupDescriptorW` + `FileContents` (virtual files from browsers and mail programs: the first entry whose name has an image
extension, read through `IStream` or `HGLOBAL`, caps of 6.1). `DragEnter` / `DragOver` answer `DROPEFFECT_COPY` when one of
them is present, else `DROPEFFECT_NONE`; they also queue `Hover { pos, kinds }` so the app can draw a drop marker. The point
arrives in screen pixels: `ScreenToClient`, divided by `pixels_per_point`.

`dnd.rs` is extended, not replaced: `plan_drop` gets the position and the payload kind. What a drop does:

| Dropped on | Picture (file or data) | `.wtes` | `.psd` | `.wtep` / model |
|---|---|---|---|---|
| 3D view, the ray hits the model | a new stamp at that point (Project mode, the Stamp tool's default size), one undo step; the Stamp tool becomes active with it selected | same, as a sticker stamp | imported as a sticker (5.8), placed there | as today |
| 2D view | a new UV stamp centred at that texel | same | same | as today |
| Sticker canvas (Sticker mode) | a new picture layer centred at the point | its layers are added | its layers are added, or one merged layer | as today |
| a stamp's row in the Layers list, or the thumbnail on the Sticker tab | image stamp: replaces the picture (`ReplaceStampImage`); sticker stamp: added as a new top layer | replaces the stamp's sticker (asks when shared) | as picture | as today |
| anywhere else, or the ray misses | today's behaviour: becomes the Stamp tool's picture (and a tray sticker) | added to the tray | added to the tray | as today |

Several pictures in one drop: the first follows the table, the others go to the tray (toast "N more added to the tray"). No
project open: pictures are refused with today's toast.

### 6.3 Clipboard

Edit menu gains **Cut** (Sticker mode only), **Copy**, **Paste**, **Paste at View Centre**, **Paste as New Paint Layer**
(5.9), **Copy Style**, **Paste Style**. Every one of them is a menu item, so the tablet and a machine where the subclass could
not be installed can do all of it without a key.

**How the keys really arrive (review A2; verified in `REG\egui-winit-0.37.0\src\lib.rs:1129-1151`, eframe without the
clipboard feature).** egui-winit swallows the three chords before the app sees a key:

| Keys | What egui-winit puts into `RawInput` | Consequence |
|---|---|---|
| Ctrl+C, Ctrl+Insert | `Event::Copy`, **no** `Event::Key` | a shortcut entry "Ctrl+C" in `shortcuts.rs` would never fire |
| Ctrl+X, Shift+Delete | `Event::Cut`, no `Event::Key` | the same |
| Ctrl+V, Shift+Insert | `Event::Paste(text)` **only if its in-app string is not empty**; otherwise nothing at all; never an `Event::Key` | with a picture on the Windows clipboard, nothing reaches the app |

So the design is:

1. **Copy / Cut** are taken from `Event::Copy` / `Event::Cut` in `raw_input_hook`: when no text field has the keyboard
   (`ctx.wants_keyboard_input()` of the last frame is false) the event is **removed** and `AppAction::Copy` / `Cut` is
   queued; when a text field has it, the event is left for egui.
2. **Paste** is caught **below winit**: WTE's window-procedure subclass (`ole/subclass.rs`) sees `WM_KEYDOWN` first. On
   `V` with Ctrl down and Alt up (`GetKeyState`; AltGr is Ctrl+Alt and is ignored), or `VK_INSERT` with Shift down, it pushes
   `OleEvent::Chord(Paste)` and then forwards the message unchanged to the previous procedure. It never eats a message.
3. In `raw_input_hook`, when a `Chord(Paste)` was drained this frame: every `Event::Paste` egui-winit may have made from its
   in-app string is removed; then, if a text field has the keyboard, the Windows clipboard's `CF_UNICODETEXT` is read and
   `Event::Paste(text)` pushed (`\r\n` -> `\n`; if the clipboard has no text or cannot be opened, egui-winit's own event is
   put back); otherwise `AppAction::Paste` is queued.
4. **Text copied in an egui field (R10):** eframe gives no hook after the frame, so at the **end of `App::ui`** WTE reads
   `ctx.output(|o| ..)` for `OutputCommand::CopyText(text)` (and the legacy `copied_text` if this egui version still fills it)
   and writes the text to the Windows clipboard as well. The command is left in place (egui-winit still stores its in-app
   copy). The builder records the exact field names of egui 0.37 in ARCHITECTURE 8.1.
5. **Scenarios inject exactly what real use produces:** `Step::Chord(Copy | Cut)` pushes `Event::Copy` / `Event::Cut` into
   the raw input; `Step::Chord(Paste)` pushes `OleEvent::Chord(Paste)` into the queue. `Step::Key(CMD, C / X / V)` is **not**
   used for the clipboard anywhere (such a key event never happens in real use and would test nothing); a unit test asserts
   that neither shortcut table binds Ctrl+C, Ctrl+X or Ctrl+V.

| Action | What is put on / taken from the Windows clipboard |
|---|---|
| Copy, a stamp selected (not in Sticker mode) | the stamp's finished picture **with its styles (what you see)** as `PNG` + `CF_DIBV5` (32-bit, alpha), and `WTE.Sticker.v1` (`.wtes` bytes incl. the stamp styles, so a paste in WTE keeps everything editable) |
| Copy, a paint layer active and no stamp selected | that layer's picture at texture size (`PNG` + `CF_DIBV5`) |
| Copy in Sticker mode | the selection of the active layer (or the whole layer), without layer styles (the pixels being edited), as `PNG` + `CF_DIBV5` |
| **Paste**, outside Sticker mode (review B14) | `WTE.Sticker.v1` first, else `PNG`, `CF_DIBV5`, `CF_DIB`, else `CF_HDROP` (first image file): the picture becomes the **Stamp tool's source** and the Stamp tool becomes active, showing its usual placement preview under the pointer; **a click places it**. Nothing is placed by the paste itself, wherever the pointer rests |
| **Paste at View Centre** | the same picture is placed at once at the point of the model (or texel) in the middle of the active view; a miss falls back to plain Paste |
| Paste in Sticker mode | a new picture layer centred in the visible canvas (a `.wtes`: its layers) |
| Text fields (R10) | steps 3 and 4 above |

`OpenClipboard` is retried 5 times, 20 ms apart; failure is a toast. Self-test runs never open the clipboard:
`AppState::clipboard` is a `trait Clipboard` with a real and an in-memory implementation (the in-memory one in self-tests and
tests).

PhotoCraft both versions read `PNG` first, then DIBV5 (arboard): copy in WTE -> paste in PhotoCraft gives a layer; copy in
PhotoCraft -> paste in WTE gives a sticker. Pixels only, no layer name or position (PhotoCraft has no richer format).

### 6.4 Drag out of WTE

- **Where a drag can start:** a row of the Layers list (texture layers and sticker layers), the sticker thumbnails (Sticker
  tab: selected stamp, tray items), and the **PSD handle** beside the selected stamp's thumbnail. Inside the window the
  Layers row drag keeps reordering as today. The drag becomes an OS drag when the pointer leaves the window rectangle with
  the primary button held (egui pointer position outside `screen_rect`, or `PointerGone` while dragging).
- **What is handed over:**
  - thumbnail or row: `CF_HDROP` with **one PNG** (`<name>.png`: a stamp's finished picture **with its styles, what you
    see**; a sticker layer's raster; a texture layer's full-canvas picture, the same pixels "Export this layer" writes)
    + `PNG` + `CF_DIBV5` + `WTE.Sticker.v1` (stickers only);
  - **the PSD handle** (review B13; a visible 24-pt handle labelled "PSD" with the caption "drag to PhotoCraft: stays
    linked" under it (plain text, not a tooltip); this replaces the earlier Alt+drag, which nobody would find and
    which a tablet cannot do):
    `CF_HDROP` with the sticker's **linked PSD**. The link is made as in 5.1 step 1 without starting PhotoCraft (saved
    project needed; the shared-picture question is asked at the press, before the drag can start). Dropped on PhotoCraft
    v0.2.0 it opens as a document whose Ctrl+S updates WTE. This is the only way a PSD leaves WTE by drag (R7); the file is
    watched from that moment and the link counts as `opened_at` (5.5).
  - Effect offered: `DROPEFFECT_COPY` only.
- **The file is prepared before the pointer leaves (review A23).** Encoding a 4096 x 4096 texture layer takes about a
  second; doing it when the pointer crosses the window edge would stall the UI. So: when egui recognises the drag (the
  press has moved past the drag threshold on a row, thumbnail or handle), a **job** renders and encodes the PNG with
  `PngLevel::Fast` into the drag-out folder. If the job is done when the pointer leaves the window, the OS drag starts at
  once; if not, it starts in the frame the job finishes, provided the button is still held and the pointer still outside;
  otherwise nothing starts. A job for a drag that never left the window is cancelled and its file removed.
- **Where the files go, and cleanup (review B7).** `%LOCALAPPDATA%\WrappedTextureEditor\dragout\` (self-tests: the run's
  folder). A PNG dragged into PhotoCraft v0.2.0 is opened **from this folder**, and its Save As may start here, so the
  user's own work can end up in it. Therefore WTE keeps `dragout\manifest.json` (name, size, modification time of every
  file it wrote) and at start removes **only** files that are in the manifest, still have exactly that size and time, and
  are older than one day. A file that is not in the manifest, or was changed since, is never touched; the folder itself is
  never removed. Linked PSDs are not in this folder (they live in `X_links\`).
- **How (critic C4, R3):** the app never calls `DoDragDrop` from `update`. It stores the prepared `DragJob` in a static slot
  and `PostMessageW(hwnd, WTE_DRAG_MSG, 0, 0)` (the registered message of 6.1). WTE's subclass (installed by `ole::install`
  above the pen subclass; it forwards every other message with `CallWindowProcW`) handles the message outside winit's event
  handler: takes the job, sets `DRAG_ACTIVE` (an `AtomicBool`; a second post while it is set is dropped), calls
  `DoDragDrop(data, source, DROPEFFECT_COPY, &mut effect)`, clears the flag, queues `OleEvent::DragEnded { effect }`.
  `IDropSource::QueryContinueDrag`: Esc -> cancel; primary button up (from `grfKeyState`) -> drop; else continue.
  `GiveFeedback` -> default cursors.
- **After the drag** `raw_input_hook` injects `PointerButton { pressed: false }` for the primary button and `PointerGone`, so
  egui does not think the button is still down (no `WM_LBUTTONUP` reaches winit), and cancels the Layers row's egui drag.
- **Pen:** WTE reads the pen through `WM_POINTER*`; OLE may not see a "button down" and end the drag at once. No workaround is
  built; it is a hand test, and Help says "drag out with the mouse" if it fails. The tablet companion cannot drag out (its input
  has no OS button state); the menu items "Export Picture...", "Copy" and "Edit in PhotoCraft" cover it.

### 6.5 What each direction does for the user

| Direction | How | Result | Checked how |
|---|---|---|---|
| Explorer / browser -> WTE | drag a picture onto the model | the sticker lands where you let go | automated in-process; hand test 7 |
| WTE -> Explorer | drag a layer row or sticker thumbnail to a folder or the desktop | a PNG file | hand test 8 |
| WTE -> PhotoCraft v0.2.0 | the same drag onto PhotoCraft | opens as a new document | read in `pc020`; hand test 8 |
| WTE -> latest PhotoCraft | the same drag | on the picture it becomes a layer, on the tab strip it opens. **Read at HEAD only; not verified on the v0.23.x build the user will install.** Hand test 8 records what really happens | source reading only |
| WTE -> PhotoCraft, linked | "Edit in PhotoCraft", or the PSD handle | PhotoCraft works on WTE's PSD; Ctrl+S updates the sticker | v0.2.0: hand test 1; latest: hand test 6, unverified until then |
| **PhotoCraft -> WTE by drag** | **not possible**: PhotoCraft has no drag source in either version | use one of the next three | - |
| PhotoCraft -> WTE | Ctrl+S on a linked sticker or layer | updates by itself | hand test 1 |
| PhotoCraft -> WTE | Ctrl+C there, Ctrl+V in WTE | a sticker to place (or a layer in the Sticker view) | hand test 12 |
| PhotoCraft -> WTE | drag the saved file from Explorer | as any picture / PSD drop | hand test 7 |
| Any app <-> WTE | copy / paste of pictures and of text | - | hand tests 12, 13 |

---

## 7. Work packages

Order: P0, P1, P2, P3a, P3b, P4, P5 (core, headless), P6a, P6b, P7a, P7b, P8 (editor UI), P9 (link UI), P10a, P10b, P11 (FFI),
P12 (linked paint layers), P13 (docs, version), then S1, S2, then the gate. One builder per package, then one independent
checker. Every package ends with: `cargo test --workspace` (only WA20 red), clippy 0, the release build, its scenarios
`"ok": true`, a commit on `stickers`, and a note `docs/notes/sticker/build-P<n>.md` (what was built, deviations from this plan
with the reason, numbers measured). A builder who finds this plan wrong against the code follows the code, keeps the user's
decisions, and writes the deviation into the note. The ARCHITECTURE sections a package names are updated **in that package**
(the contract stays in step with the code).

"Kind": K = core logic (headless), U = app UI, F = Windows FFI. Revision 2 split P3, P6, P7 and P10 (review A15) and added
P12; no package should now need more than one builder.

| Package | One line |
|---|---|
| P0 | baseline numbers and outputs, LF notes, `.gitattributes`, PSD fixtures, licence file, the CLI-vs-GUI save check |
| P1 | styles and picture maths (`StyleStack`, `apply_styles`, EDT helpers, adjustments, resampler, shapes) |
| P2 | selection, cut-out and filter maths (wand, colour to alpha, remove background, defringe, blur, sharpen, gradient) |
| P3a | sticker types in `Document`, `StampSource::Sticker`, render, stamp styles pipeline, two placements, job freshness |
| P3b | pixel strokes, generic tile patches, `StickerPixels` undo, promotion inside an edit, limits, `convert_text_stamp`, Revert |
| P4 | project format 4 and `.wtes` |
| P5 | PSD reader / writer and the link logic (mapping, states) |
| P6a | Sticker view: mode, canvas, sticker layers in the Layers tab, Move and Hand, Sticker-mode shortcuts, companion rects |
| P6b | Sticker tab (five tabs), stamp styles UI, presets, tray, edit before placing, copy / paste style, shared-picture question |
| P7a | painting: brush, eraser, masks, fill, gradient, eyedropper, pen, the stroke-mode canvas |
| P7b | selections and cut-out: marquee, lasso, wand, colour range, selection commands, move selection, magic eraser, cut-out, blur / sharpen |
| P8 | layout (crop, trim, resize, rotate), transform handles, text and shape layers, clipping, adjustments UI, library, Export as PSD |
| P9 | PhotoCraft link in the app: watcher, launcher, dialogs, save / Save As / load / close rules |
| P10a | Windows drop target, `dnd.rs` on the OLE queue, `Step::Drop` |
| P10b | clipboard: subclass key chords, pictures, text in egui fields, `Step::Chord` |
| P11 | drag out (subclass message, `DoDragDrop`, prepared files, manifest cleanup, PSD handle) |
| P12 | linked texture paint layers, Paste as New Paint Layer |
| P13 | docs, hand-test sheet, licences in About and installer, version 0.4.0, the whole-workflow scenario |
| S1, S2 | stretch: GrabCut box cut-out; live PNG copy and watch folder |

### P0 - Baseline and housekeeping (K)

- Convert `docs/notes/sticker/{pc-features,pc-interop,win-dnd,wte-model}.md` to LF. Commit the five notes and this plan.
- Record the baseline in `docs/notes/sticker/baseline.md`: test counts, clippy, exe hash, and reference outputs in
  `target\sticker\baseline\` (git-ignored) made with the release exe of the untouched branch: the scenario set of 8.5, each
  with its shots, exports and saved `.wtep`.
- **Create** `.gitattributes` (none exists) with exactly `*.psd binary`, `*.psb binary`, `*.wtes binary`, `*.wtep binary`;
  no `text=` rule (the LF state of text files stays governed by `core.autocrlf=false`; the checker confirms
  `git ls-files --eol` is unchanged for every existing file).
- Make the PSD fixtures of 8.3 with the user's `photocraft-cli.exe` v0.2.0 (headless, scratch folder
  `G:\Claude_Resources\ref\scratch\stickerfix\`), copy them to `crates/wte_core/tests/fixtures/psd/` with `README.md` (exact
  command lines, PhotoCraft version, sizes).
- **CLI save = GUI save? (review B16)** Read in `pc020` whether `photocraft-cli convert in.psd out.psd` and the GUI's
  Ctrl+S (`saveDocument`) reach the same `psd_export` function with the same options (layer ids kept, trimming, merged
  image, compression), and whether the GUI path runs anything before it that the CLI does not (events, flatten, dirty
  flags). Write the call chains with file:line into `docs/notes/sticker/pc-save-paths.md`. If they differ in anything that
  touches `lyid`, layer pixels or the merged image, say so at the top of the note: fixture f1 then proves the CLI only, and
  **hand test 1 is the only proof** that WTE text layers stay text after a real Ctrl+S (the gate report repeats this).
- Add `licenses/photocraft-MIT.txt`.
- Checker: notes are LF (`git ls-files --eol`), fixtures total under 400 KB, each fixture's README line reproduces it; under
  `crates/` **only `crates/wte_core/tests/fixtures/psd/**` was added and no existing file changed** (review A21); reads the
  two save paths independently and compares with the note.

### P1 - Styles and picture maths (K)

- Files: new `sticker/{mod,style,fx,adjust,shape,resample}.rs`, `pixel.rs` (`AlphaBuf`), `text/edt.rs` (`nearest_ink`; the
  existing function untouched), `lib.rs` exports. No `Document` changes yet.
- Content: 2.3, 2.4, 4.2, 4.3, 4.6, 4.7, 4.8; `StyleStack::margin`, `sanitize`, presets.
- ARCHITECTURE: 2.1, 5.5, new 5.27 (types, part 1), new 6.11 (styles), 6.9 (pitfalls), 9.1; 1.12 skeleton.
- Unit tests (names start `k1_`): contour of a disc of radius 20 with width 10 is a disc of radius 30 within 0.6 px; contour
  fills the hole of a ring; smoothing closes a 3-px notch and never lowers alpha inside the artwork; stroke Outside / Inside /
  Center coverage sums on a square (areas within 1 %); a stroke around a contour sits outside it; drop shadow offset direction
  for 0 / 90 / 120 / 270 degrees; spread 1 gives a hard edge, spread 0 a tent profile (monotone, symmetric); margin is enough:
  for 200 random style stacks (fixed seed) no non-zero pixel touches the output border; `apply_styles` with all styles off
  returns the base unchanged (same bytes, margin 0); **on a fully opaque 64 x 64 picture (review A7): an Inside stroke of
  width 4 colours exactly the 4-px border ring (+-1 px of anti-aliasing), inner shadow, inner glow and bevel each change at
  least the border pixels, the output size is still 64 x 64, and no value is NaN or infinite**; colour overlay keeps alpha;
  gradient endpoints exact; bevel of a flat interior is neutral; every adjustment against hand-computed values (identity
  settings are byte-identical); affine identity and whole-pixel moves copy bytes; rotate 90 four times = identity; shapes:
  area of an ellipse / star within 0.5 %; tent blur preserves the sum away from borders; `nearest_ink` against brute force
  on 16x16; thread-count independence for `apply_styles` and the resampler with **local pools of 1 and 8 threads** (2.7);
  `apply_styles` at working scale 1/2 with halved lengths equals the box-reduced full-size result within 3/255 mean.
- Perf tests (ignored, `perf_k1_*`, serial): 8.4.
- Checker: reads every formula against section 4; fuzzes `apply_styles` with empty, 1x1, fully opaque and fully transparent
  inputs; checks the licence header on adapted files; confirms `distance_to_ink` is unchanged (the text tests pass untouched).

### P2 - Selection, cut-out and filter maths (K)

- Files: `sticker/{select,cutout,filter}.rs`.
- Content: 3.4 (core part), 4.4, 4.5 (not GrabCut), 4.9.
- ARCHITECTURE: 1.3 (the non-goals "colour-tolerance flood fill, selections" now apply to the texture Fill tool only), 5.27,
  new 6.12, 9.1.
- Unit tests (`k2_`): wand on a two-colour image selects exactly one side at tolerance 0; tolerance edge cases (diff equal to
  tolerance is in); contiguous vs not; anti-alias values on a known ramp; transparent pixels match each other; magic eraser on
  a white-background picture leaves the object's alpha 255 and the background 0; fill honours a selection; colour to alpha:
  pure `C` -> alpha 0, a 50 % mix of red over white with `C` = white -> red at alpha 128 +- 1, opaque black stays opaque;
  remove white / black matte invert a matte within 1 level for alpha >= 32; defringe replaces a white halo on a red disc
  (no pixel with alpha > 0 keeps `g > 40`); remove background on three synthetic pictures (flat white, two-colour border,
  vertical gradient with Follow gradients) keeps the object (IoU >= 0.98 against the generator's mask) and reports the
  "already transparent" case; grow then shrink of a disc returns it within 1 px; feather keeps the mask sum within 1 %;
  blur of a constant picture is the same picture, blur keeps the premultiplied sum away from borders, a transparent
  neighbour does not darken an edge (no colour bleed); sharpen with amount 0 is the identity; a gradient's endpoints are
  exact and a selection limits it; thread-count independence (local pools).
- Checker: reruns with its own pictures (a noisy white background generated in the test), looks for unbounded work in the
  flood fill (a 4096x4096 single-colour image completes; perf test), checks determinism.

### P3a - The sticker document: types, render, styles pipeline, jobs (K)

- Files: `ids.rs`, `stamp.rs`, `document.rs`, new `doc_stickers.rs` (queries, `add_sticker`, `remove_sticker`, `promote_stamp`,
  `detach_sticker`, `set_sticker` for `StickerRef::Id`, `set_stamp_styles`, `set_stamps_styles`, `bake_placement`,
  `bake_wrapped_projector`, `stamp_picture`, `sticker_picture`), `doc_stamps.rs` (the `source_mips` pipeline, the working
  size, the call sites of the two placements, `job_inputs_current`), `history.rs` (`StructureSnapshot::stickers` only),
  `stamp_job.rs` (`sticker`, `DrawnPicture`, styling in the job), `stamp_bake.rs` / `wrap_*` call sites, `sticker/render.rs`,
  `error.rs`, every `match` on `StampSource` in `wte_core` and the minimum in `wte_app` to compile (the app shows a sticker
  stamp like an image stamp for now: thumbnail from `stamp_picture`, the "Image: Choose..." row hidden).
- Content: 2.1, 2.2 (incl. picture versions), 2.5, 2.6, 2.7, 2.8 (the functions listed above), 2.9, 4.1 (incl. clipping).
- ARCHITECTURE: 4.4, 5.1, 5.2, 5.12, 5.14, 5.20, 5.27, 6.5 (two placements, margin, working size), new 6.10, 9.1.
- Unit tests (`k3a_`):
  1. **Identity:** a document with image and text stamps (projected, wrapped, UV; two sets) baked with default styles gives
     cache hashes equal to hashes recorded from the P0 code (stored in the test); `promote_stamp` changes no cache byte and
     starts no bake; `bake_placement` returns the stored placement bit for bit when the margin is 0.
  2. A promoted stamp with a contour: the artwork's texels are unchanged where the contour does not cover them; the linked
     layer grows; the stored placement is unchanged; undo restores caches byte for byte.
  3. **Two placements (review A9):** for a wrapped stamp with a shadow style, `stamp_wrap_outline` and the gizmo box are the
     same as without the style; the baked footprint is larger; `serving_wrap_param` says "does not serve" right after the
     style is added and "serves" after the re-parameterisation; an automatic depth is refitted when the margin changes
     (the far edge of the shadow is baked on a curved sample mesh) and a manual depth is left alone.
  4. Text stamp with a stroke style at 64, 128 and 256 px per em: stroke width on the model is the same within 1 texel.
  5. **Working size (review A5):** an 8192 x 8192 image stamp with ClassicSticker: `apply_styles` is never called on more
     than 4096 px a side (a test counter in `fx`), the Preview call has at most 524,288 pixels, the styled width on the
     model is the same within 1 texel for working scales 1, 1/2, 1/4; a Final edit does not call `apply_styles` on the
     calling thread (thread id recorded by the counter).
  6. `set_sticker` with a cropped artboard: a marked layer-space point stays at the same texel (+-1) for Projected, UV and
     rotated / flipped placements.
  7. Shared sticker: two stamps, one `set_sticker` re-bakes both; `detach_sticker` then edits one only.
  8. History: snapshots share Arcs (no pixel copy on a rename); `history_bytes` counts a replaced picture once; eviction under a
     small budget keeps the document consistent; pending Preview edits fold into one entry and `cancel_pending_edit` restores.
  9. **Job freshness (review A1):** make a Final job for a sticker stamp; edit the sticker (`set_sticker`) before applying;
     `apply_final_bake` returns `Stale`, the stamp is still in `unbaked`, the next job's result is applied and the cache
     equals the synchronous bake of the **new** sticker. The same for `apply_wrap_preview` / `same_inputs`. A job applied
     without an edit in between is `Applied`, and afterwards a Preview of the same stamp does not render (render counter 0:
     the job's `DrawnPicture` filled the caches, review A27). Undo while a bake is running never applies a stale result.
  10. **Caches (review A8):** after dropping a sticker value and creating another one, a render-cache lookup with the new
      Arc never returns the old picture (loop of 1,000 create / drop cycles with different content; any hit must be
      byte-correct); the per-layer key changes when the picture version changes and does not read pixel bytes (a counting
      wrapper around the hash input).
  11. Render: layer order, opacity, each blend mode against `composite::flatten_layers` on the same pixels; mask; clipping
      (a pattern clipped to a text shows only inside the letters; a hidden base hides it); shape crop; the trivial fast
      path returns the picture bytes; scale 2 doubles a shape's size exactly; `sticker_scale` is above 1 for a sticker
      whose only layer is a picture shrunk to half.
- Checker: greps every read of `stamp.placement` and every call of `wrapped_projector` in bake / wrap code and lists each
  with "bake" or "stored" and why (the table of 2.6); runs the existing wrap and stamp tests; runs scenarios `stamp`, `text`,
  `wrap_surface`, `workflow_path` and compares with the baseline (8.5).

### P3b - Pixel edits, undo, limits, conversion (K)

- Files: `history.rs` (`TileBuf`, generic `TilePatch` / `TileBackup`, `Edit::StickerPixels`), `pixel.rs`, `doc_stickers.rs`
  (strokes, `replace_sticker_pixels`, `StickerRef::OfStamp` with promotion inside the edit, `convert_text_stamp`,
  `sticker_checkpoint`, `restore_sticker_state`, `rebake_sticker_users`, limits), `document.rs` (`next_pixel_version`).
- Content: 2.7 (limits, history cost), 2.8 (the remaining functions), 2.10.
- ARCHITECTURE: 5.18, 5.20, 6.7, 9.1.
- Unit tests (`k3b_`):
  1. Strokes: begin / dab / end makes one entry; undo / redo restore bytes and give new versions each time; a stroke on
     `Picture(Blob)` turns it `Owned` and leaves the blob and other users of the blob alone; cancel restores; the selection
     mask limits dabs.
  2. **Masks (review A4):** a stroke on a mask is a 1-byte-per-pixel tile patch (patch bytes = changed tiles x 1), undo /
     redo restore it; a patch made for a mask is refused on a picture buffer; **the existing `Edit::Pixels` tests pass
     untouched** and a recorded patch of a texture stroke has the same bytes as on the P0 code.
  3. `replace_sticker_pixels` same size -> tile patch with only changed tiles; other size -> structure entry.
  4. **Promotion inside an edit (review A3):** on an image stamp, (a) Preview, Preview, Final of an opacity change through
     `StickerRef::OfStamp` = **one** undo entry, after undo the stamp is `StampSource::Image` again and no sticker is left
     in use; (b) `cancel_pending_edit` after the first Preview restores the image stamp; (c) a brush stroke = one entry
     (`Edit::Multi` of the promotion and the pixels), undo and redo both work; (d) `ShareScope::ThisStamp` leaves a second
     stamp of the same blob an image stamp.
  5. **First dab after a snapshot (review A25):** the buffer is copied exactly once, inside `sticker_stroke_begin` (an
     allocation counter on `PixelBuf` clones); the snapshot's Arc still holds the old bytes.
  6. Limits: 33rd layer, 4097-px artboard, over-budget picture are refused with their messages and change nothing.
  7. `convert_text_stamp` (review A24) at raster sizes 32, 128 and 512 px per em: same placement frame within 1 px; mean
     absolute difference of the baked cache <= 2/255 at each of the three; stamp style lengths scaled by `px / 128`.
  8. Revert: `restore_sticker_state` after five mixed edits (structure and pixels) is one entry; the sticker equals the
     checkpoint; undo of the revert brings the five edits' result back.
- Checker: reruns the full history test module; reads `TilePatch` for any place that still assumes 4 bytes; scenario
  `undo`, `paint2d`, `paint3d` against the baseline.

### P4 - Project format 4 and `.wtes` (K)

- Files: `project/{mod,manifest,save,load,migrate}.rs`, new `sticker/file.rs`, `tests/project_roundtrip.rs`.
- Content: 2.11, 2.12 (link entries are written here as plain data; their meaning comes in P5).
- ARCHITECTURE: 5.23, 7.1, 7.2, 7.3, new 7.8, 9.1.
- Tests (`k4_`): a v3 fixture project (made by the P0 exe, committed under `tests/fixtures/`) loads into a document equal to
  the one the P0 code built (stamps, layers, cache hashes) and saves with `min_reader_version` as before; `required_reader`
  is 4 for each new feature alone and unchanged otherwise (table test); full round trip of a sticker with every layer kind,
  mask, clip, adjustment, style, shared by two stamps, plus a kept tray sticker; unused, un-kept stickers are not written;
  missing picture file -> warning and empty layer; missing sticker -> raster layers; **save reuse (review A8):** an unchanged
  owned picture is copied (entry bytes equal, no encode call); after a stroke and its undo (bytes equal again, **new
  version**) the picture is encoded again and the entry decodes to the right bytes; two different pictures can never share
  a reuse record (property test over 200 random edit / save sequences comparing every saved PNG with the document);
  `.wtes` round trip and deterministic bytes; a manifest with unknown extra fields in the new entries still loads;
  non-finite floats in styles are refused at save with a named message (`stamp_floats` pattern).
- Checker: feeds a v4 manifest with a sticker to a copy of the P0 version check (unit test, no exe run) and confirms the
  "made by a newer version" path; byte-compares the layer PNG entries of a re-saved v3 fixture.

### P5 - PSD reader / writer and link logic (K)

- Files: new `psd/{mod,read,write}.rs`, new `link.rs`, `document.rs` (`links`), `doc_stickers.rs` (`apply_link_update`,
  `link_state`, `prepare_link_file`), `project/*` (archive entries, the save rule, `LoadReport::links`; **no disk write at
  load**).
- Content: 5.2-5.5 (core parts).
- ARCHITECTURE: new 5.28, new 6.13, 7.1, 7.7, new 7.9, 9.1. `export/psd.rs` untouched (its golden test proves it).
- Tests (`k5_`):
  - writer -> reader round trip (names incl. non-ASCII and > 31 bytes, hidden, opacity, all seven blends, a mask, a clipped
    layer, layer partly outside the canvas, empty layer, opaque vs transparent merged, scale 1 / 2 / 4); writer output opens
    in the old strict `read_psd`;
  - every fixture of 8.3: layer pixels equal the PhotoCraft-exported per-layer PNGs exactly, merged equals PhotoCraft's own
    `convert` PNG within 1 level for alpha >= 16 (the test states the bound per fixture), features flagged as listed in the
    fixture README;
  - **the comparison rule (review A11), on fixture f1** (which has a soft-edged layer, a text layer and a shape layer):
    after PhotoCraft's re-save every untouched layer is recognised as unchanged although its rect or its hidden RGB
    differs, `lyid`s are the ones WTE wrote, and importing gives back a sticker **equal** to the one exported (text still
    text, shape still shape); writing the same sticker twice gives the same `lyid`s; adding a layer in WTE and writing
    again keeps the old ids and uses `next_lyid` for the new one;
  - synthetic files: 1057 says no real merged data -> `merged` is `None` and `has_real_merged` false, and Layers mode still
    imports; ZIP and ZIP-prediction channels (8 and 16 bit) for layers **and for the merged image**; PSB with each of the
    13 long keys; default blending ranges in both forms (empty, `(cc + 1)` pairs) are not BLEND_IF, a changed range is;
    16-bit soft transparency un-matted at 16 bits is closer to the truth than un-matting after reduction (bound in the
    test); grayscale; CMYK / 32-bit -> the right `Unsupported` text;
  - **truncation sweep**: every prefix length of the smallest fixture and 2,000 evenly spaced prefixes of the largest return
    `Err` (never a panic, never `Ok` with a picture); random byte flips (fixed seed, 5,000) never panic;
  - **the mapping table of 5.4 row by row (review B3):** unchanged; moved; mask added to a plain picture layer; mask added
    to a text layer; pixels changed; a new PhotoCraft layer; a PhotoCraft TEXT layer with pixels (fixture f5) -> one new
    picture layer **and every WTE layer kept**, `pc_native` set; a clipped layer (fixture f11) -> `clip`; a deleted layer;
    a WTE layer deleted in PhotoCraft and a new layer that took its `lyid` (fixture f12, 5.4 "`lyid` reuse");
    a disabled mask; each forcing feature alone -> Merged with the right sentence; Merged replaces the layers and undo
    restores them;
  - scale (review B4): a text layer exported at scale 2 and re-saved untouched comes back as the same text layer; a
    changed layer comes back as a picture with transform scale 1/2 and the same on-model size; a canvas change maps to the
    artboard through the scale; a canvas over the artboard limit -> nothing applied, then "Fit it into the sticker";
  - the state table of 5.5 (all four rows) incl. "undo does not re-trigger", "nothing is written at save" (the save
    function gets no writer for link files at all: compile-time), and `stale_in_pc` (as a pure function of the inputs the
    app passes in); `.bak.psd` rotation; the save rule of 5.3 (CRC equal -> stored; different and readable -> stored;
    different and truncated -> the old archive entry is kept and a warning returned); `LoadReport::links` for present,
    missing and changed files with no file written by `load`; a link to a missing external file.
- Checker: reads the reader for unchecked arithmetic on lengths (32-bit multiplication overflow, `usize` casts), runs the
  sweep under `--release` as well, makes one more fixture with the real CLI that the builder did not make and reads it.

### P6a - Sticker view and canvas (U)

- Files: `state.rs` (`ViewMode::Sticker`, `StickerEdit`, `sticker_canvas_rect`, `PanelHit::StickerCanvas`), `action.rs`,
  `dispatch.rs`, `view/mod.rs`, new `wte_app` `sticker/{mod,canvas,layers_ui}.rs`, `panels/{layers,toolbar,menu_bar}.rs`,
  `shortcuts.rs` (`STICKER_SHORTCUTS`, the routing rule), `pen/mod.rs` + `app.rs` (`eraser_override` branch), `prefs.rs`,
  `companion/` (the canvas rect in `state.view_rects`), `selftest/mod.rs` (`Target`, `Step::WaitMs`, shots with expected
  content, 8.2), `selftest/scenarios/sticker.rs` (new), `frame_profile` spans.
- Content: 3.1 (incl. Revert), 3.2 (frame; Layers tab in Sticker mode with add / delete / reorder / show-hide / opacity /
  blend), 3.3 (the key table and routing; tools Move (move only) and Hand / zoom), 3.5 (structure edits), 3.10.
- ARCHITECTURE: 1 (new 1.12), 2.2, 8.2, 8.3, 8.5, 8.7, 8.10, 8.11, 8.13, new 8.15. HANDOFF 4.1.
- App tests: enter / leave keeps the previous mode; `ViewMode::Sticker` is never written to prefs / `ui_state`; the shortcut
  tests of 3.3 (one per colliding chord and mode; Delete never deletes the stamp; no duplicate chord); the pen's eraser end
  selects the sticker Eraser in Sticker mode and the paint Eraser outside; a layer add on an image stamp promotes and one
  undo undoes both; Revert; companion mapper with the canvas rect registered: a one-finger drag gives Secondary and the
  canvas pans, pinch zooms, `FingerMode::Mouse` gives Primary (driving the existing mapper in a unit test); `Target` keeps
  every existing scenario compiling (the scenario count test).
- Scenario: `sticker_view` (enter the view on the image stamp, add a second picture layer from the sample blob, move it with
  a `Step::Pointer` drag on `Target::StickerCanvas`, lower its opacity, reorder; assert the 3D cache hash changes after each
  step and one undo per step; Revert and its undo; leave with Esc; shots at 1280x800 and 900x600).
- Checker: runs the scenario and looks at every shot against its expected content; presses every key of the table in both
  modes through a copy of the scenario; compares the regression set (8.5).

### P6b - Sticker tab, styles UI, tray (U)

- Files: `panels/{side_groups,tool_options}.rs`, new `sticker/{panels,styles_ui,tray}.rs`, `tools/stamp_place.rs`,
  `stamp_bakes.rs`, `io_flows.rs`, `dialogs/` (`StickerShared`), scenarios.
- Content: 3.6, 3.7, 3.8 (tray, Export Picture; the library comes in P8), 3.9 (five tabs, two rows), the shared-picture
  question in the editor (E6). Stamp styles editable for image, text and sticker stamps (the Stroke / Outline note on text
  stamps, 2.3).
- ARCHITECTURE: 8.3, 8.7, 8.8, 8.9, 8.15. HANDOFF 5 (row + new 5.4), 6. `dist/HOW_TO_TEST.txt` new chapter "Stickers" (first
  part). README features.
- App tests: `control_sweep` for every new control (no panic, rects registered); G1-G4 with five tabs in one row (wide panel)
  and in two rows (300 pt) at the three window sizes; `UpperTab::ALL` indices `[0..=4]`; an unknown tab name in prefs falls
  back as on the P0 code; a style edit on an image stamp does not promote; E6 dialog answers; E5 paste onto three selected
  stamps is one entry; placing a `Blob` tool source with default styles creates `StampSource::Image`; a style slider drag
  on a stamp with an automatic depth ends with a refitted depth.
- Scenarios: `sticker_styles` (built-in sample: place an image stamp and a text stamp; apply ClassicSticker to both from the
  Sticker tab outside Sticker mode; assert the linked layers grew and the stored placements did not change; shots),
  `sticker_tray` (new sticker, edit before placing, place it twice, edit "all", then "only this one"; save, reopen, assert
  equality), `sticker_controls` (every new control clicked once through `ScreenClick`).
- Checker: runs the scenarios and looks at every shot; lists every feature with its non-hover, non-right-click way (3.10);
  `side_tabs` at the three sizes; compares the regression set (8.5).

### P7a - Painting (U)

- Files: `wte_app` `sticker/tools/{brush,eraser,fill,gradient,eyedropper}.rs`, `sticker/canvas.rs` (dirty rectangles, stroke
  mode, the restyle job), `sticker/commands.rs` (masks), panels, scenarios.
- Content: 3.3 rows Brush, Eraser, Fill, Gradient, Eyedropper; masks (Add from selection, Paint on mask, Invert, Apply,
  Delete); 3.5 (the stroke rules); pen pressure and the eraser end.
- ARCHITECTURE: 8.6, 8.7, 8.11, 8.15, 9.2. HOW_TO_TEST.
- App tests: **stroke mode (review A6):** on a layer with a contour, during a stroke the canvas pixels inside a dab's
  rectangle equal "layers below + unstyled active layer + layers above" and no style job result is shown before the
  stroke ends; at most one restyle job is running; after the end the canvas equals a fresh full render; on a layer without
  styles no full recomposite happens (counter).
- Scenarios: `sticker_edit` (brush stroke, eraser stroke, undo each; a stroke on a layer with a ClassicSticker layer style:
  the "styles update when you lift" note is on during the stroke and gone after, the model shows the contour around the
  new paint after the stroke; fill; a gradient inside a marquee made by a command; all through pointer steps; asserts on
  sticker pixels and on the 3D cache), `sticker_pen` (`Step::Pen` on the canvas: pressure changes the stroke width; the
  inverted end erases; a barrel drag pans).
- Checker: tries each tool on each layer kind (text / shape layers must offer Rasterize, never paint); strokes across the
  artboard edge; a selection that excludes the stroke; a tool switch in mid-drag; `Esc`; looks at the shots; reads the
  frame log of `sticker_edit` for the span limits of 8.4.

### P7b - Selections and cut-out (U)

- Files: `sticker/tools/{marquee,lasso,wand,magic_eraser,move_selection}.rs`, `sticker/commands.rs`, the "Sticker" menu,
  scenarios.
- Content: 3.3 rows Marquee, Lasso, Magic Wand, Magic Eraser, Move with a selection; 3.4; the Cut-out, Filter and Selection
  commands (incl. Color Range, Layer via Cut).
- ARCHITECTURE: 8.6, 8.7, 8.11, 8.15, 9.2. HOW_TO_TEST.
- Scenario: `sticker_cutout` (a generated picture: an object on white with a soft edge; marquee + Delete key (pixels go, the
  stamp stays); wand + fill; Magic Eraser, undo, Remove Background, Colour to Alpha on a second layer, Defringe, Blur
  inside a selection; move a selection; then the ClassicSticker style; asserts: corner alpha 0, object alpha 255, a halo
  measure below a bound; shot of the model with the die-cut sticker).
- Checker: as P7a; plus Delete / Backspace with and without a selection, with a stamp selected.

### P8 - Layout tools, text, shapes, clipping, adjustments, library (U)

- Files: `sticker/tools/{crop,text,shape,transform}.rs`, `sticker/{adjust_ui,library}.rs`, panels, jobs (library previews),
  scenarios.
- Content: 3.3 rows Move / Transform (handles, numbers), Crop, Text, Shape; the Sticker and Layer commands (Trim, Padding,
  Canvas Size, Resize, Rotate, Flip, Flatten, Merge Down, Rasterize, Clip to Layer Below); the style editor per sticker
  layer; the Adjustments UI; inner shadow, gradient overlay and bevel in the style editor; Library; Export as PSD; the
  `convert_text_stamp` button.
- ARCHITECTURE: 8.6, 8.7, 8.9, 8.11, 8.15, 9.2. HOW_TO_TEST. Help text.
- Scenarios: `sticker_text` (add a text layer and a star shape behind it, give the text layer a stroke, add a picture layer
  above the text and clip it to the text ("pattern inside text": assert a pixel between two letters is transparent and one
  inside a letter has the pattern's colour), change the text through the Text tab, warp it; convert an existing text
  stamp; shots), `sticker_layout` (crop to an ellipse, trim, padding, rotate 90, resize 200 %: the marked point stays on
  its texel and each step is one undo; Flatten), `sticker_library` (save to the library in the run folder, new project, add
  from the library, place).
- Checker: as P7a; plus every numeric field with 0, negative, huge and non-numeric input; a sticker at each limit.

### P9 - PhotoCraft link in the app (U)

- Files: new `wte_app` `link/{mod,watch,launch,ui}.rs`, `app.rs` (`logic` drains link events), `dialogs/` (save first,
  shared picture, conflict, "saved over", already open, send warning, locate, first-time note), `prefs.rs`, `io_flows.rs`
  (the archive at save, Save As with aliases, restore and "changed while closed" after load, the close prompt line), menus,
  Sticker tab, Help.
- Content: 5.1, 5.3 (app parts), 5.5 (dialogs, `stale_in_pc`), 5.6, 5.7, 5.8.
- ARCHITECTURE: 3.3 (`advapi32`), 4.6 (the watcher thread), 8.3, 8.8, 8.9, 8.13, new 8.16, 9.2. HOW_TO_TEST chapter "Working
  with PhotoCraft".
- Tests: watcher unit tests with temp files (a change is reported once; a half-written file waits; unreadable after the
  timeout; deleted file; **`repaints` stays 0 over 3 polls without a change**; a write whose CRC is in `own_writes` is not
  reported even when the watch list is still the old one; an alias path change is copied and reported once; the thread
  stops with the last link); launcher: the stub is what a self-test and the tests get; `command_for` with spaces,
  non-ASCII, UNC and `&` / `%` paths, spawning nothing; discovery from an injected registry reader: key 1, key 2, neither,
  a command line with and without quotes, a path that does not exist; the two-copies guard with a fake child; an unsaved
  project gets the "save first" dialog and no file is written; a shared sticker gets the question before any file exists;
  project save writes no link file (file times unchanged) and archives by the rule; the close prompt lists links made since
  the last save.
- Scenarios: `sticker_link`, all on a project saved into the run folder, the stub launcher, and files the scenario builds
  with `psd::write_sticker`:
  1. two image stamps of one picture; "Edit in PhotoCraft" -> the shared-picture dialog; "Only this one" -> the PSD exists in
     `<run>\<name>_links\`, reads back, the stub saw the path, the second stamp is still an image stamp;
  2. the scenario overwrites the file with a changed sticker and waits with `Step::Until` for the stamp's cache hash to
     change; one undo step "Update Linked Sticker";
  3. undo; `Step::WaitMs(3000)`; assert no second update arrived (history length, cache hash), the badge text is exactly
     "changed in WTE", and the watcher's `repaints` counter did not move during the wait (**the idle check**, review A14);
     "Update now" applies it again;
  4. save the project: the PSD's bytes and time on disk are unchanged, the archive entry exists;
  5. **the sticker open in the Sticker view (review B5):** select an area, start a pointer drag of the brush, overwrite the
     file with a version that lacks the active layer, wait: nothing changes while the button is down; after the release the
     update is applied, the selection is gone, the active layer is the topmost one, the toast "Updated from PhotoCraft";
  6. a file with a PhotoCraft TEXT layer added (fixture bytes): the WTE layers are still there, one new picture layer;
     then a file with an enabled layer effect -> the Merged toast, undo restores the layers;
  7. a change in WTE and on disk -> the conflict dialog, both answers;
  8. "Send to PhotoCraft" after a stub launch -> the warning; "Send anyway"; then overwrite the file -> the "saved over"
     conflict, never a silent update (review B1);
  9. save, reopen: the archive and the link state are there; delete the PSD and reopen: it is restored by the app;
     change it while closed: the "changed while closed" dialog.
  Also `sticker_link_path` (PATH = a PSD file: imported as a sticker and placed).
- Checker: greps the tree for any spawn of `photocraft` outside `RealLauncher` and shows that `RealLauncher` cannot be built in
  a self-test; greps `link/` for focus, flash and beep calls (none); runs `sticker_link` three times (timing robustness);
  deletes the links folder in mid-run in a copy of the scenario.

### P10a - Windows drop target (F)

- Files: new `wte_app` `ole/{mod,formats,com,drop_target,data_object}.rs`, `dnd.rs` (the OLE queue as the source of hover
  and drop when installed), `app.rs`, `selftest/mod.rs` (`Step::Drop { payload, target, at }` feeding the same queue the
  drop target feeds; never the OS).
- Content: 6.1 (without the subclass), 6.2.
- ARCHITECTURE: 2.2, 3.3 (libraries linked by `raw-dylib`), 8.1 (drop facts), 8.3, 8.9, 8.11, 8.13, new 8.17, 9.2. HANDOFF 4.2
  (`WTE_OLE`). HOW_TO_TEST chapter "Drag, drop, copy, paste".
- Tests: codecs (DIB 24 / 32, DIBV5 with masks, top-down, with and without alpha; DROPFILES with spaces and non-ASCII; file
  group descriptors; malformed inputs never panic); vtable layout asserts; the drop target driven in-process: `DragEnter` /
  `DragOver` / `Drop` / `DragLeave` with the fake data object for each format, reference counts return to their start,
  `QueryInterface` answers; **`Drop` has copied the bytes before it returns** (the fake object frees and poisons its memory
  on release; the queued payload is still right) and `ReleaseStgMedium` ran on every path; the caps (a 65-MiB item is
  skipped with `skipped: 1`, an endless `IStream` stops at the cap); a panic inside a callback is caught (test hook); the
  WTE-origin marker is ignored; the `plan_drop` table of 6.2 row by row; the overlay of `dnd.rs` from `Hover` / `Leave`
  events and from egui's `hovered_files` gives the same result.
- Scenario: `sticker_drop` (`Step::Drop` of a PNG payload on the 3D view at a model point: a stamp appears at that texel
  (+-2); on the 2D view; on the Sticker canvas; on a stamp row; a miss; a `.wtes`; three pictures at once; a `.wtep` hover
  shows the overlay), run once more with `WTE_OLE=1` (must give the same result).
- Checker: reads every `unsafe` block against the Win32 contracts (who frees which `HGLOBAL`, `ReleaseStgMedium` on every
  path, `AddRef` / `Release` balance); runs `smoke`, `default`, `pen`, `companion` with `WTE_OLE=1`.

### P10b - Clipboard (F)

- Files: `ole/{subclass,clipboard}.rs`, `app.rs` (`raw_input_hook`: Copy / Cut events, the paste chord, text paste; end of
  `App::ui`: `CopyText`), `panels/menu_bar.rs`, `tools/stamp_place.rs` (paste as the tool source), `selftest/mod.rs`
  (`Step::Chord`).
- Content: 6.3 and the key part of the subclass (6.1).
- ARCHITECTURE: 1.3 (the "no OS clipboard" paragraph is replaced), 8.1 (the verified egui-winit clipboard facts of 6.3), 8.3,
  8.10, 8.17, 9.2.
- Tests: the subclass routing on a fake previous procedure: `WM_KEYDOWN` V with Ctrl queues one `Chord(Paste)` and is
  forwarded; with Ctrl+Alt (AltGr) nothing is queued; Shift+Insert queues; every other message is forwarded unchanged;
  `raw_input_hook` on recorded inputs: (a) `Event::Copy` without text focus -> removed, `AppAction::Copy`; with text focus
  -> left alone; (b) chord + text focus + in-app `Event::Paste("old")` -> exactly one `Event::Paste(os text)`; (c) chord,
  no text focus -> `AppAction::Paste`, no paste event left; (d) chord with an empty in-memory clipboard and text focus ->
  egui-winit's event is kept; `CopyText` at the end of `ui` reaches the in-memory clipboard; **neither shortcut table
  binds Ctrl+C / X / V**; in-memory clipboard round trips of pictures (alpha kept) and `.wtes`; paste makes the Stamp
  tool's source and places nothing until a click; Paste at View Centre places one stamp.
- Scenario: `sticker_clip` (in-memory clipboard; **only `Step::Chord`, never `Step::Key`**: copy a stamp, paste -> the Stamp
  tool has it, a click places it; Paste at View Centre; in Sticker mode copy a selection and paste as a layer; put text on
  the in-memory clipboard and paste it into the text-sticker field with the chord; copy text out of the field).
- Checker: confirms no test opens the real clipboard (grep and the call sites of `ole/clipboard.rs`); reads the subclass
  (it never returns without calling the previous procedure); runs `pen` and `companion` with
  `WTE_PEN_SUBCLASS=1 WTE_OLE=1`.

### P11 - Drag out (F)

- Files: `ole/{subclass,drag_source}.rs`, `panels/layers.rs`, the Sticker tab thumbnails and the PSD handle, `app.rs`
  (`raw_input_hook` pointer-up injection), the drag-out job and `dragout\manifest.json`.
- Content: 6.4, 6.5.
- ARCHITECTURE: 8.3, 8.7, 8.13, 8.17, 9.2.
- Tests: the data object built for each drag kind offers the right formats and bytes (the PNG decodes to the expected picture
  **with the stamp styles**, the `DROPFILES` path exists on disk); the state machine: the job starts when the drag is
  recognised, leaving the window with the button held and the job done posts exactly one message, leaving before the job
  is done posts it when the job finishes, releasing first posts nothing and removes the file; a second request while
  `DRAG_ACTIVE` is dropped; `DragEnded` injects the pointer-up and cancels the row drag; in-window reordering still works
  (existing layer tests); **cleanup (review B7):** a manifest file older than a day is removed, a fresh one kept, a file
  not in the manifest is kept, a manifest file whose size or time changed is kept, the folder is never removed; the PSD
  handle on an unsaved project shows "save first", on a shared sticker asks first, and otherwise creates the link record
  and the PSD. `DoDragDrop` itself is never called by a test (a seam returns a canned effect).
- Scenario: `sticker_dragout` (pointer steps that start a row drag and leave the window; the seam records one drag with the
  expected formats; the next pointer move drags nothing).
- Checker: reads the subclass (every message but the registered drag message and the forwarded key-downs reaches the
  previous procedure untouched; the pen subclass still gets its messages: scenario `pen` with
  `WTE_PEN_SUBCLASS=1 WTE_OLE=1`).

### P12 - Linked texture paint layers (K + U)

- Files: `doc_layers.rs` or where layer commands live (`replace_layer_pixels`), `link.rs` (`PaintLink`, export / import of
  the one-layer PSD), `project/*` (entries, archive), `wte_app` `link/*` (the same watcher, launcher and dialogs keyed by
  `LinkId { Sticker(StickerId) | Layer(LayerId) }`), Layer menu, row context menu, Edit menu (Paste as New Paint Layer).
- Content: 5.9.
- ARCHITECTURE: 5.14, 5.28, 6.13, 7.1, 7.9, 8.16, 9.1, 9.2.
- Tests: `replace_layer_pixels` is one `Edit::Pixels` entry with only the changed tiles, undo / redo restore; the PSD of a
  layer reads back to the same pixels; one-layer and multi-layer PSDs choose the right source; a size mismatch is refused
  with the message and changes nothing; the state table for a layer link; **a project without layer links saves the same
  bytes as before P12** (identity); paste as a new layer at equal and at different sizes.
- Scenario: `layer_link` (paint a layer, "Edit Layer in PhotoCraft" with the stub, the scenario overwrites the PSD with a
  changed picture, the layer and the model update, one undo; conflict; save and reopen).
- Checker: runs the regression set (8.5) with special attention to `layers`, `export`, `roundtrip`; a locked and a
  stamp-linked layer do not offer the command.

### P13 - Docs, licences, version, the whole workflow (U)

- Files: README, `dist/HOW_TO_TEST.txt` (final, every hand test of 8.7 with steps and expected result), Help texts (what is
  lost, the guard's limits, "no drag from PhotoCraft"), `dialogs/about.rs` (**new** "Licences..." window),
  `installer/WrappedTextureEditor.iss` (**new** `[Files]` line for `licenses\*`), release notes, workspace version 0.4.0.
- ARCHITECTURE: 10 (work-package list). HANDOFF 1, 5 (5.4: state and the hand tests owed), 8.
- Tests: the About window lists the licence text (control sweep); the version string.
- Scenario: `sticker_flow_path` (PATH = a model; gate: `ABeautifulGame.glb`, `DamagedHelmet.glb`): the user's whole workflow -
  drop a picture on the model, cut out its background, die-cut + shadow, add text as a layer, wrap it, the stub "Edit in
  PhotoCraft" + a file update, copy / paste the style to a second sticker, link a paint layer and update it, save, reopen,
  export one layer, a selection and everything as a zip (the `export_flow` checks on the files).
- Checker: confirms every hand test of 8.7 is in HOW_TO_TEST with its expected result; the installer script lists the
  licence folder (no installer is built or run by an agent); greps for the forbidden name (8.6 item 7).

### S1 - GrabCut box cut-out (K + U, stretch)

`sticker/grabcut.rs` (4.5), tool "Cut Out Box" (drag a rectangle around the object), option "Into mask". Tests: three synthetic
photo-like pictures (textured background) reach IoU >= 0.9; determinism; 2048x2048 within the budget. Scenario
`sticker_grabcut`.

### S2 - More doors (K + U, stretch)

(a) **Live copy:** per sticker or layer "Keep a PNG copy up to date" in the links folder (re-written 500 ms after the last
change), for PhotoCraft's File > Place Linked. (b) **Watch folder:** File > Watch Folder for Stickers...: every PNG that
appears or changes there becomes / updates a tray sticker of that name (PhotoCraft's Generate > Image Assets folder
`<document>-assets`, critic D1). Tests and one scenario each. (Linked paint layers moved from here to P12.)

### Gate (after P13; again after S1 / S2 if built)

Section 8.6. Report `docs/reports/STICKER_GATE_REPORT.md`; then an independent completeness critic appends its section.

---

## 8. Test plan

### 8.1 Automated: unit and integration tests

Listed per package in section 7. Expected: about 220-280 new `wte_core` tests and 110-150 new `wte_app` tests. Naming: `k<n>_...`
in the core, plain descriptive names in the app. Wall-clock tests are ignored `perf_*` tests, run serially (HANDOFF 4.4).

### 8.2 Automated: scenarios (`selftest/scenarios.rs` `NAMES`, the count test updated)

New: `sticker_styles`, `sticker_view`, `sticker_tray`, `sticker_controls`, `sticker_edit`, `sticker_pen`, `sticker_cutout`,
`sticker_text`, `sticker_layout`, `sticker_library`, `sticker_link`, `sticker_link_path`, `sticker_drop`, `sticker_clip`,
`sticker_dragout`, `layer_link`, `sticker_flow_path` (+ `sticker_grabcut` and S2's). Every scenario: 1280x800, off-screen, no
focus, writes only inside its output folder, `"ok": true`, asserts state (hashes, counts, history labels). In each: one undo
per user action; after undoing everything the document hash equals the start; no Error toast.

Additions to the self-test runner (P6a, P10a, P10b), each with a unit test:

- `Target { View(ViewId), StickerCanvas }` for pointer, pen, click and drop steps (3.1).
- **`Step::WaitMs(n)`** (review A14): the runner keeps ticking and goes on when `n` milliseconds of wall-clock time have
  passed. Needed because the runner repaints every frame, so "nothing happened for 3 s" cannot be told from a frame count.
  What is asserted after the wait is **state** (history length, cache hash, the watcher's `repaints` counter), never a
  frame counter.
- **Shots carry their expected content** (review B15): `Step::Shot { name, expect: &'static str }`, for example
  `expect: "3D half: the helmet with a white-bordered photo on the visor; canvas: the photo on a checkerboard, two layers
  in the list"`. The runner writes `{ name, expect }` for every shot into `out.json`. The package checker and the gate
  write, per shot, one line: the name, `expect`, and "matches" or what differs. A shot without `expect` fails the
  scenario-count unit test for the new scenarios.
- `Step::Drop`, `Step::Chord` (6.3): they feed the same queues real input feeds.

### 8.3 PSD fixtures (made in P0 with the real v0.2.0 `photocraft-cli.exe`, headless; never `photocraft.exe`)

Small canvases (at most 64x48). For each: the `.psd`, PhotoCraft's own `convert` PNG (the expected merged picture), per-layer
PNGs from `file.export.layersToFiles` (the expected layer pixels), and the command line.

| Id | Content | Proves |
|---|---|---|
| f1 | a WTE-written three-layer sticker PSD (**a soft-edged feathered picture layer**, a text layer's raster, a shape's raster; `lyid` 1..3), opened and re-saved by PhotoCraft (`convert in.psd out.psd`) | PhotoCraft keeps `lyid`, names, order; its re-trim and its handling of RGB under alpha 0 do not make an untouched layer look changed (the comparison rule of 5.4) |
| f2 | soft-edged transparency (a feathered disc) | un-matting the merged picture from white |
| f3 | a fully opaque picture | 3-channel merged |
| f4 | a layer with a mask and a live Stroke effect (`select.subject`, `layer.layerMask.revealSelection`, `layer.layerStyle.stroke`); and the same file without the effect | EFFECTS -> Merged, the merged picture includes the effect; without the effect: the mask comes back as a WTE mask (Layers mode) |
| f5 | f1 plus a type layer added in PhotoCraft | TEXT with pixels -> one new picture layer, the three WTE layers kept |
| f6 | a layer group | GROUP -> Merged |
| f7 | a 16-bit document with soft transparency | `Lr16`, 16 -> 8, un-matting at 16 bits |
| f8 | hidden layer, 50 % opacity, Multiply; a second file with Color Dodge | hidden / opacity / blend mapping; BLEND_OTHER -> Merged |
| f9 | transparency plus a saved alpha channel | channel order (critic R1) |
| f10 | a PSB of f1 (if the CLI writes `.psb`) | PSB lengths |
| f11 | a layer clipped to the layer below (if the v0.2.0 command registry has a clipping command) | `clipped` -> `clip` |
| f12 | (added by the P0 check) f1 with the WTE layer `text` deleted and a new layer added in PhotoCraft | the new layer takes the freed `lyid` 2 (5.4 "`lyid` reuse") |

(f1 needs the P5 writer: P0 makes f1 from a PSD written by today's `export/psd.rs`, with a soft-edged layer; P5 replaces it
with one from the new writer and keeps both.) f1 is made with the CLI; whether that equals a GUI Ctrl+S is what P0's
`pc-save-paths.md` answers (if not: hand test 1 is the only proof, said in the gate report). If a command a fixture needs is
not in v0.2.0's registry (`G:\Claude_Resources\ref\scratch\pcfeat\commands.json`), the builder builds that file with the
test's byte builder instead and says so in the README. Built in tests (no latest-PhotoCraft binary exists here): resource
1057 with `hasRealMergedData = 0` and a white placeholder; ZIP and ZIP-prediction channels and a ZIP merged image; the 13 PSB
long keys; both default blending-range forms; CMYK and 32-bit headers; truncated and corrupted files.

### 8.4 Performance budgets (ignored `perf_*` tests, serial, release, this machine; the gate records the numbers)

| Operation | Budget |
|---|---|
| `apply_styles` ClassicSticker (contour 12, smooth 0.5, shadow), 1024x1024 base | 60 ms |
| the same, 2048x2048 / 4096x4096 (Final, in the job) | 250 ms / 1.1 s |
| the same at the Preview working size (<= 524,288 px, UI thread) | 33 ms |
| peak extra memory of `apply_styles` at 4096x4096 | 256 MB (counted allocations in the perf test) |
| EDT (`distance_to_ink`) 2048x2048 | 80 ms |
| `sticker::render`, 3 layers (picture, text, shape) with one layer style, 1024x1024: cold / one layer changed | 120 ms / 50 ms |
| Preview-quality render during a slider drag (any sticker within the limits) | 33 ms |
| one brush dab on a 2048x2048 picture layer, size 64: pixels + the dirty-rectangle composite (layer **without** styles, or in stroke mode) | 2 ms |
| `sticker_stroke_begin` on a 4096x4096 picture shared with a snapshot (the one copy) | 40 ms |
| entering stroke mode on a styled layer, 2048x2048 sticker (one full recomposite) | 50 ms |
| restyle job during a stroke (Preview scale) / at the stroke's end (full, 2048x2048) | 60 ms / 300 ms, both off the UI thread |
| wand / magic eraser / remove background, 2048x2048 | 60 / 60 / 150 ms |
| Gaussian blur radius 16, 2048x2048 | 150 ms |
| flood fill worst case, 4096x4096 one colour | 400 ms |
| `psd::read` 2048x2048 with 5 layers (RLE) / `psd::write_sticker` of the same | 150 ms / 150 ms |
| a settled file change on disk -> the sticker updated on the model (1024x1024) | 1.5 s |
| repaint requests from the watcher while a link is watched and nothing changes | 0 in 3 s (`LinkWatcher::repaints`) |
| drag-out PNG of a 4096x4096 texture layer (`PngLevel::Fast`) | on a job; the UI thread never waits for it (span check) |
| GrabCut (S1) 2048x2048 | 3 s, on a job with a progress note |

**Frame-time limit, measurable form (review B15).** All sticker, link and OLE code that runs inside a frame is inside
`frame_profile` spans whose names start with `sticker.`, `link.` or `ole.` (3.5). The frame log (`out.log` of a scenario,
the existing frame profile) then gives, per frame, the sum of these spans. Limit: in `sticker_view`, `sticker_edit`,
`sticker_styles` and `sticker_flow_path` on `ABeautifulGame`, **no frame has more than 50 ms in these spans**, except the
frames named in this table with their own budget (entering stroke mode). The existing preview-bake spans are reported, not
limited. `tools/sticker/frame_spans.py` (new) prints the worst frames; the gate records them.

A budget missed by less than 50 % is reported and decided at the gate; more is a fail.

### 8.5 Identity checks (rule 8 of 0.4)

Regression set, run with the P0 exe (baseline) and after every package: `smoke default paint3d paint2d stamp text preview
export export_run roundtrip undo layers workflow_path wrap_surface text_warp export_flow export_checks side_tabs objects pen
companion` (`workflow_path` on `ABeautifulGame.glb`). Compared: (1) every export file byte for byte; (2) the layer PNG entries
of every saved `.wtep` and the manifest's stamps and layers (the manifest may differ only in `format_version` and the app
version); (3) screenshots restricted to the central view rectangle below the mode strip, pixel-identical (the mode strip gains
a button and the upper tab strip a tab and possibly a row: those areas are excluded and listed in the report); (4) the core
hashes existing tests already assert. Tool: `tools/sticker/compare_baseline.py` (new; run with `python -E -P`), built on
`tools/round3/shot_diff.py` and `check_exports.py`.

### 8.6 Gate criteria

1. `cargo test --workspace`: only WA20 fails. The ignored GPU and `perf_` suites run as HANDOFF 4.4 describes; budgets of 8.4
   incl. the frame-span limit.
2. clippy 0 warnings; release build; exe hash recorded.
3. Every scenario of `NAMES` `"ok": true` (old and new); the new ones also with `WTE_OLE=1 WTE_PEN_SUBCLASS=1`. For every new
   shot the gate agent writes one line: name, the scenario's `expect` text, and "matches" or the difference (8.2). A shot
   that does not match its `expect` is a red.
4. The identity checks of 8.5 pass.
5. `Cargo.lock` differs from `main` only in the workspace crates' version; no new `[dependencies]` line anywhere.
6. Focus audit: nothing named `photocraft.exe` was started (the stub's log; a process list before and after the sweep), no
   scenario window took focus (the existing `raw.focused` check), no clipboard call in tests (grep), no focus / flash / beep
   call in `link/`.
7. LF audit (`git ls-files --eol`: no `w/crlf` among the branch's text files); the licence file ships; no "ArtCraft" outside
   `licenses/photocraft-MIT.txt` and the notes (grep over `crates/`, `dist/`, `installer/`, README).
8. The v3 fixture project opens and exports byte-identically; a v4 project with a sticker is refused by the P0 version check
   with the "newer version" message.
9. The hand-test sheet (8.7) is complete, and the report lists, for each hand test, "reported by the user: result" or "owed".
10. The report repeats what no automated test could prove: the real PhotoCraft GUI save (unless P0 showed CLI = GUI), every
    statement about the latest PhotoCraft (read at HEAD only), a real drag between two apps, the real clipboard.
11. Verdict PASS / PASS with listed reds / FAIL, with the numbers.

**Landing on `main` (review B15).** A gate PASS alone does not land the branch. The main session lands it only when the user
has reported hand tests **1, 3, 4, 7, 8, 12 and 15**, or when each of them that is still open is listed as owed in HANDOFF 5.4
**and the user has said to land anyway**. A failed hand test is a red like a failed scenario.

### 8.7 User hand tests (`dist/HOW_TO_TEST.txt`: "Stickers", "Working with PhotoCraft", "Drag, drop, copy, paste")

Each with steps and the expected result:

1. Real PhotoCraft v0.2.0 round trip: Edit in PhotoCraft on a sticker with a picture, a text and a shape layer (save the
   project when asked; locate the exe once), paint on the picture layer, Ctrl+S, the model updates **and the text is still an
   editable text layer in WTE**; once more. With a picture used by two stickers: the "Edit all / Only this one" question
   comes first, and "Only this one" leaves the other sticker unchanged after the update.
2. Add a layer effect in PhotoCraft, save: WTE shows the finished picture and the note that names the effect.
2b. **Add a text and a layer mask in PhotoCraft (no effects), save: the new text appears as a new layer in WTE, the mask
   is a mask in WTE, and WTE's own layers (text still text) are all still there.**
3. Click Edit in PhotoCraft twice: the "already open" question (v0.2.0 / portable copy only; with the installed build, note
   whether a second tab opens or the first is focused).
4. Change the sticker in WTE and in PhotoCraft, save: the conflict question, both answers. Then: Send to PhotoCraft while the
   file is open there (the warning), save in PhotoCraft: WTE asks instead of updating silently.
5. Close WTE, change the PSD, reopen the project: the "changed while the project was closed" note. Also: link a sticker,
   close WTE **without saving** (the prompt names the sticker), save in PhotoCraft: nothing breaks, the PSD is still in the
   links folder.
6. After updating PhotoCraft to the latest: 1, 2 and 2b again; with Maximize Compatibility off, the message of 5.4 (only when
   the file needs the merged picture). **Everything this plan says about the latest version was read in its source at HEAD,
   not tried; write down what differs.**
7. Drag a picture from Explorer onto the model; from a browser; onto the Sticker canvas.
8. Drag a sticker thumbnail and a layer row to the desktop (a PNG appears, with the border and shadow); onto PhotoCraft
   v0.2.0 (opens as a document); onto the latest PhotoCraft's picture and tab strip (**expected: becomes a layer / opens;
   unverified, record the real result**).
9. Drag the PSD handle onto PhotoCraft, Ctrl+S there, WTE updates.
10. After any drag out: the next mouse move in WTE drags nothing; reordering layers still works. A day later, files you saved
    yourself into the drag-out folder are still there.
11. The same drags with the pen (may not work: note what happens).
12. Ctrl+C in PhotoCraft, Ctrl+V in WTE (the sticker follows the pointer, a click places it), and the other way;
    transparency survives. Ctrl+V with a picture copied in Paint or a browser.
13. Paste text from Notepad into the text-sticker field with Ctrl+V and with Shift+Insert; copy text out with Ctrl+C.
14. Stay in PhotoCraft (or in a full-screen game) and press Ctrl+S: WTE does not come to the front, **its taskbar button does
    not flash, no sound plays**; a conflict or "changed while closed" question is waiting inside WTE's window when you go
    back, not on top of other programs.
15. Tablet companion: enter the Sticker view, paint with the S Pen with pressure, erase, drag one finger (the canvas pans),
    pinch-zoom the canvas, add a style, Revert, Done.
16. Open a project made with v0.3.0 and export: the files equal a v0.3.0 export.
17. Edit Layer in PhotoCraft on a texture paint layer: paint, Ctrl+S, the model updates; undo.
18. In the Sticker view press V, W, H, F, X, Delete (with and without a selection), Esc: each does what the key table of
    Help says; outside the view the same keys do what they did before.

---

## 9. Risks and open questions

Risks the builders handle (with the fallback):

| # | Risk | Handling |
|---|---|---|
| 1 | A bake or wrap call site uses the wrong one of the two placements | the table of 2.6, P3a tests 2 and 3, the checker's annotated grep; an app test that the gizmo box stays on the artwork |
| 2 | Five upper tabs do not fit the panel | decided in 3.9: a second row; G1-G4 re-proved in both layouts; the 640x480 numbers are reported |
| 3 | Re-registering the drop target leaves the window without one if `RegisterDragDrop` fails | log + toast; the failure path is tested through a seam; `dnd.rs` falls back to egui's events |
| 4 | OLE drag with a pen | hand test 11; documented fallback (mouse, Copy, Export Picture) |
| 5 | The latest PhotoCraft's PSDs are read only from constructed files (no binary here) | a strict reader with clear messages; hand test 6; `photocraft-cli` fallback on a click; every such statement is marked unverified (6.5, 8.6 item 10) |
| 6 | The merged picture loses colour precision at very low alpha after un-matting | Layers mode whenever nothing forces Merged; un-matting at 16 bits; bounds stated in the tests |
| 7 | Text stamp -> sticker conversion is not bit-identical | it is an explicit command; nothing existing changes without it; bound tested at 32, 128 and 512 px per em |
| 8 | Sticker pixel memory (several 4096 layers, undo, the one copy per stroke after a snapshot) | limits of 2.7, the history budget, refusal messages, the budget of 8.4 |
| 9 | Whole-window screenshot identity is no longer possible | 8.5 compares the central view and lists the excluded areas |
| 10 | An older build reads prefs with new keys | unknown keys and tab names are ignored by the prefs reader (tests against the P0 behaviour); `ViewMode::Sticker` is never written |
| 11 | A half-written PSD is applied or archived | the settle rule, the strict reader, the truncation sweep, the save rule of 5.3 |
| 12 | PhotoCraft's GUI save differs from the CLI save used for fixture f1 | P0's `pc-save-paths.md`; if they differ, hand test 1 is named as the only proof |
| 13 | PhotoCraft writes its old canvas over what WTE sent | no write at save; the warning; `stale_in_pc` turns the next change into a question (5.5) |
| 14 | The subclass misses the paste chord on some keyboard layout or IME | Edit > Paste always works; hand tests 12, 13; the subclass never eats a message |
| 15 | The generic tile patch changes texture-paint undo | the existing history tests untouched + a byte comparison of a recorded patch (P3b test 2) |
| 16 | A clipped or masked PhotoCraft layer is positioned by a rule this plan read only in v0.2.0 | fixtures f4, f11; otherwise the test's byte builder and a note |

Open questions the code cannot settle (section 11 lists those for the user):

- Whether the installed latest PhotoCraft focuses an already open tab or opens a second one when the same PSD is opened again
  (hand test 3 records it; the guard does not cover that build either way).
- Whether the latest PhotoCraft's own winit fork accepts picture data without files on a drop (WTE always offers a real file,
  so nothing depends on it).
- Whether key `4` or the PSD handle drag collides with something on the user's Windows (hand test).

---

## 10. ARCHITECTURE.md and HANDOFF.md edits (who changes what)

| Document section | Change | Package |
|---|---|---|
| ARCH 1 | new **1.12 Stickers and PhotoCraft** (the decisions of 0 and 0.1 here, traceability to the sections below); 1.1 row R4 mentions stickers; 1.3 non-goals updated (selections and tolerance fill exist for stickers only; the OS clipboard paragraph is replaced) | P6a (1.12 skeleton in P1), P2, P10b |
| ARCH 2.1 / 2.2 | new modules `sticker/`, `doc_stickers.rs`, `psd/`, `link.rs`; app `sticker/`, `link/`, `ole/` | P1, P3a, P5, P6a, P9, P10a |
| ARCH 3 | no new crate; the `raw-dylib` libraries (`ole32`, `shell32`, `advapi32` added) | P9, P10a |
| ARCH 4.2, 4.4, 4.6, 4.7 | sticker colour rules; new ids; the watcher thread; determinism of the sticker maths (local pools in tests) | P1, P3a, P9 |
| ARCH 5.1, 5.2, 5.5 | ids, errors, `AlphaBuf`, `TileBuf` | P1, P3a, P3b |
| ARCH 5.12, 5.14, 5.20 | `StampSource::Sticker`, `StampObject::styles`, the `Document` fields and API of 2.8, the `source_mips` pipeline and working size, the two placements, picture versions, job freshness (`job_inputs_current`), `DrawnPicture` | P3a |
| ARCH 5.18, 6.7 | generic tile patches, `Edit::StickerPixels`, snapshot contents, promotion inside an edit, the one copy per stroke | P3b |
| ARCH 5.14 (layers) | `replace_layer_pixels` | P12 |
| ARCH 5.23, 7.1, 7.2, 7.3 | format 4, entries, `required_reader`, load rules, save reuse by version | P4 (links: P5, P12) |
| ARCH new 5.27 | `sticker/` API | P1, P2, P3a, P3b |
| ARCH new 5.28 | `psd/` and `link.rs` API | P5, P12 |
| ARCH 6.5 | margin, bake placement vs stored placement (the table), auto depth, working size of styles | P3a |
| ARCH new 6.10, 6.11, 6.12, 6.13 | sticker render (incl. clipping); styles (incl. the inside pad); cut-out, selection and filter maths; the PSD <-> sticker mapping, the comparison rule, the state table, who writes when | P3a, P1, P2, P5 |
| ARCH 7.7 | one line: the sticker PSD writer is separate (`psd/write.rs`) | P5 |
| ARCH new 7.8, 7.9 | `.wtes`; the links folder and the PSD subset | P4, P5 |
| ARCH 8.1 | verified egui-winit clipboard facts (Copy / Cut events, no key event, paste only from the in-app string), drop facts, `ctx.output` at the end of `ui` | P10a, P10b |
| ARCH 8.2, 8.3, 8.5-8.11, 8.13 | state, actions, frame hooks, the view, tools, panels (five tabs, two rows), dialogs, prefs and flows, **the two shortcut tables and the routing rule**, `Target`, `Step::WaitMs` / `Drop` / `Chord`, shots with `expect`, pitfalls (never `DoDragDrop` inside `update`; never the real launcher or clipboard in a self-test; never a repaint from the watcher without a change; never a link write at project save; never `Step::Key` for clipboard chords) | P6a-P11 |
| ARCH new 8.15, 8.16, 8.17 | Sticker view (incl. stroke mode); PhotoCraft link (incl. layer links); OLE | P6a-P8, P9 + P12, P10a-P11 |
| ARCH 9.1, 9.2, 10 | tests; the work-package list "stickers" | every package; P13 |
| HANDOFF 1 | one paragraph: stickers, format 4 | P13 |
| HANDOFF 4.1, 4.2 | scenario list and count; `WTE_OLE` | P6a-P13 |
| HANDOFF 5 | table row "stickers" and new 5.4 (state, **hand tests owed, the landing rule of 8.6**) | P6b, P13 |
| HANDOFF 6 | the decisions of 0.1 here; "No image editor needed for PSD / ORA" stays for export validation, with the note that stickers have their own editor since 2026-11-25; "colour picker / tabs" note: five upper tabs | P6b |
| HANDOFF 8 | open items left by the gate | gate |
| `dist/HOW_TO_TEST.txt`, README, Help texts | 8.7 | P6b-P13 |
| `CLAUDE.md` | not edited by builders (the main session keeps it) | - |

---

## 11. Decisions for the user (defaults chosen so the work can proceed)

Things to know first (not choices):

- **You cannot drag from PhotoCraft into WTE.** PhotoCraft (both versions) cannot start a drag. What works instead: press
  Ctrl+S in PhotoCraft on a linked sticker or layer (WTE updates by itself); Ctrl+C in PhotoCraft and Ctrl+V in WTE; or drag
  the saved file from Explorer. Dragging **out of** WTE into PhotoCraft works.
- **What was read but not tried.** Everything about the latest PhotoCraft (v0.23.x) comes from reading its source at HEAD;
  there is no build of it here. After you update, hand tests 1, 2, 2b, 6 and 8 are the real check. The "already open" warning
  works only for v0.2.0 and portable copies.

Choices (each has a default; say so if you want the other):

1. **Styles stay in WTE.** PhotoCraft shows a linked sticker without WTE's border and shadow; WTE puts them back on whatever
   returns (5.4). Option: bake them into the PSD (they would freeze). Default: stay in WTE.
2. **What comes back from PhotoCraft.** Painting, new layers, PhotoCraft text, layer masks, clipping and moved layers come back
   layer by layer and WTE's own layers stay editable. Only layer effects, groups, adjustment layers and unusual blend modes
   make the sticker come back as one finished picture (still editable in PhotoCraft). Default: as planned.
3. **A link needs a saved project, and WTE never rewrites the linked file by itself.** "Edit in PhotoCraft" on a new project
   asks you to save first. Changes you make in WTE reach the file only when you click Send to PhotoCraft or Edit in
   PhotoCraft. Reason: PhotoCraft v0.2.0 does not reload a file that changed, so an automatic rewrite could make it save
   old content over your work. Option: rewrite at every project save (simpler, but can lose edits). Default: never by itself.
4. **A "PSD" drag handle hands PhotoCraft the linked file**; a plain drag of the thumbnail hands over a PNG copy (6.4).
   (Replaces the earlier Alt+drag idea.) Default: as planned.
5. **Sticker is a fifth tab** of the upper group; at the normal panel width the tab strip then has two rows (3.9). Options:
   two rows (default); or shorter labels / an icon for one tab (changes existing labels).
6. **Linked texture paint layers are in the plan (P12)**, because you asked to link "photos/layers": "Edit Layer in
   PhotoCraft" on a paint layer works like the sticker link, on the flat UV texture. Option: leave it out for now (saves one
   package). Default: built.
7. **Paste** makes the picture follow the pointer and a click places it; "Paste at View Centre" places it at once. Default:
   as planned.
8. **Keys in the Sticker view**: V Move, M Marquee, L Lasso, W Wand, C Crop, B Brush, E Eraser, G Fill, Shift+G Gradient,
   Shift+E Magic Eraser, I Eyedropper, X Text (as in the rest of WTE; PhotoCraft uses T), U Shape, H Hand, 4 opens the view.
   Delete clears selected pixels and never deletes the sticker. Outside the view every key does what it does today. Option:
   T for Text inside the view as well. Default: X only.
9. **Left for later** (say if one of these matters now): Curves and colour balance, perspective / warp of pictures, clone /
   heal, brush presets, a history panel, layer groups, pixelate / noise filters, quick-selection brush, several strokes per
   layer. Added in this revision because they are cheap and expected: clipping masks, Gaussian blur and sharpen, a gradient
   tool, moving selected pixels, Select > Color Range.
10. **Stretch packages**: S1 (cut a photo out with a box, GrabCut) and S2 (live PNG copy, watch folder for PhotoCraft's Image
    Assets) now, or after you have used the rest. Default: built only if P0-P13 are green.
11. **Version and release**: the branch becomes v0.4.0; a tag and an installer release only on your word. **Landing on
    `main`** happens after the gate **and** after you have tried hand tests 1, 3, 4, 7, 8, 12 and 15 (about 20 minutes with
    PhotoCraft v0.2.0 and the tablet), or earlier if you say "land, tests owed".

---

## 12. Review log (revision 2, 2026-11-25)

Two reviews of revision 1. Every finding was accepted unless marked otherwise. "Where" = the section that now holds the fix.

Reviewer A (feasibility):

| # | Finding (short) | Outcome | Where |
|---|---|---|---|
| A1 | Final-bake freshness cannot see a sticker edit | fixed: the job carries the `Arc<StickerDoc>`; `job_inputs_current` (`Arc::ptr_eq`) in `apply_final_bake`, `same_inputs`, `apply_wrap_preview` | 2.7, P3a test 9 |
| A2 | Ctrl+C / X / V never arrive as key events | fixed: Copy / Cut from `Event::Copy` / `Cut`; paste from `WM_KEYDOWN` in the subclass (`OleEvent::Chord`); text copy read from `ctx.output` at the end of `App::ui`; scenarios use `Step::Chord` only | 6.1, 6.3, P10b |
| A3 | `group_edits` flushes the pending edit: promotion would be a second undo step | fixed: promotion inside the command after `begin_pending`, no entry of its own; pixel edits push one `Edit::Multi` | 2.8 (`StickerRef`), 2.10, P3b test 4 |
| A4 | tile patches are RGBA only, masks are 1 byte | fixed: `TilePatch` / `TileBackup` generic over `TileBuf`; masks stay `AlphaBuf` | 2.10, P3b test 2 |
| A5 | styles on image stamps have no size cap | fixed: working size (Final <= 4096 a side and <= 2x texel density, Preview <= 524,288 px), lengths scaled, Final styled in the job | 2.6, 8.4, P3a test 5 |
| A6 | dirty-rectangle canvas is wrong for a styled layer | fixed: stroke mode (active layer unstyled, restyle job every 100 ms for the model, full restyle at the end); budgets redone | 3.5, 8.4, P7a |
| A7 | inside styles draw nothing on an opaque picture | fixed: internal pad of at least 1 px, cropped back; margin unchanged; k1 test | 2.3, 4.3, P1 |
| A8 | cache and save-reuse keys by address | fixed: picture versions (monotonic), render LRU holds the Arc and uses `ptr_eq`, `PngReuse` by version, no pixel hashing in keys | 2.2, 2.7, 2.11, P3a test 10, P4 |
| A9 | `wrapped_projector` has other readers; depth fit | fixed: new `bake_wrapped_projector`, the table of who uses which placement, `serving_wrap_param` on the bake placement, auto depth refit on margin change | 2.6, P3a test 3 |
| A10 | link file moves at first save / Save As while PhotoCraft has it open | fixed: a link needs a saved project (no session folder); Save As keeps an alias watch on the old path | 5.1, 5.3, 5.6 |
| A11 | CRC + rect comparison breaks on PhotoCraft's re-trim; `lyid` renumbered | fixed: normalised pixels (alpha-0 zeroed, trimmed) on write and on compare; origin compared separately (a move is a move); stable `lyid` with `next_lyid`; f1 has a soft-edged layer | 5.2, 5.3, 5.4, 8.3 |
| A12 | tool keys collide with the global table; pen eraser override | fixed: `STICKER_SHORTCUTS`, the routing rule, per-chord tests, `eraser_override` branch, `pen/mod.rs` in P6a | 3.3, P6a |
| A13 | five tabs: no overflow rule exists | decided: a second row when one row does not fit; `[Rect; 5]`; index test updated | 3.9, P6b |
| A14 | idle check by frame counter cannot work | fixed: `LinkWatcher::repaints` counter, `Step::WaitMs`, unit test | 5.6, 8.2, P9 |
| A15 | P3, P6, P10 too large | fixed: P3a / P3b, P6a / P6b, P10a / P10b; also P7a / P7b; docs split off as P13 | 7 |
| A16 | companion: one finger is a Secondary drag; no view list; no dialog hint | fixed: the canvas registers its rect in `state.view_rects`, Secondary / Middle drag pans, wheel zooms; no hint claimed; "(on the PC)" suffix | 3.10, P6a |
| A17 | `ViewId::Sticker` breaks exhaustive matches | fixed: no new `ViewId`; `sticker_canvas_rect` and self-test `Target` | 3.1, 8.2 |
| A18 | PSD table gaps (PSB keys, merged ZIP, blending ranges, 16-bit order, oversize canvas) | fixed, all five | 5.2, 5.4 |
| A19 | wrong install paths, and they contain the forbidden name | fixed: registry only (two keys), then ask | 5.7 |
| A20 | no licence list / installer entry exists | fixed: P13 creates both | 4.0, P13 |
| A21 | P0 checker rule contradicts the fixtures; no `.gitattributes` | fixed | P0 |
| A22 | `Drop` must copy synchronously; registered message; hover overlay | fixed, all three | 6.1 |
| A23 | PNG written at window exit stalls the UI | fixed: a job started when the drag is recognised, `PngLevel::Fast` | 6.4, P11 |
| A24 | text Outline vs stamp Stroke; conversion at 128 px/em | fixed: both allowed with a note; conversion at the current raster size, tested at 32 / 128 / 512 | 2.3, 2.8, P3b test 7 |
| A25 | `Arc::make_mut` copy on the first dab | stated and budgeted (in `begin`, 40 ms at 4096 x 4096); tile sharing **not** built (a larger redesign of the picture buffer than this delivery needs) | 2.10, 8.4 |
| A26 | archive of a half-written file; disk write at core load; own-write race | fixed, all three | 5.3, 5.6 |
| A27 | a job's render must be stored; determinism tests need local pools | fixed | 2.7 |

Reviewer B (user fit):

| # | Finding (short) | Outcome | Where |
|---|---|---|---|
| B1 | rewrite at project save lets PhotoCraft save old content silently | fixed: no write at save; explicit Send only; warning; `stale_in_pc` makes the next change a question | 5.3, 5.5, `sticker_link` step 8, hand test 4 |
| B2 | shared-picture question missing at link time | fixed: asked at Edit in PhotoCraft and at the PSD handle, before the file is written | 3.1, 5.1, `sticker_link` step 1, hand test 1 |
| B3 | Merged mode on the first text or mask | fixed: masks and clipping map to WTE masks and `clip`; PhotoCraft TEXT / SMART / FILL layers with pixels become picture layers; only real forcing features give Merged | 5.2, 5.4, hand test 2b |
| B4 | PSD at scale 1 comes back blurry | fixed: written at the render scale (1, 2, 4), stored in the link, mapped back | 2.6, 5.3, 5.4 |
| B5 | update while the sticker is open | fixed: held until the stroke / drag ends; selection dropped; active layer re-picked; toast | 5.6, `sticker_link` step 5 |
| B6 | key collisions; Delete must not delete the stamp; one Text key | fixed with A12; Text is X in both modes | 3.3, hand test 18 |
| B7 | drag-out cleanup could delete the user's files | fixed: manifest; only unchanged WTE-written files; never the folder | 6.4, P11 |
| B8 | unsaved-project links and session folders | fixed by removing them (A10); the close prompt names links made since the last save; WTE deletes nothing in the links folder | 5.3, hand test 5 |
| B9 | claims about the latest build stated as fact | fixed: marked "read at HEAD, unverified"; the guard's limit stated; hand tests record the result | 5.7, 6.5, 8.6 item 10, 11 |
| B10 | linked paint layers only a stretch; "no drag from PhotoCraft" hidden | fixed: P12 is a must with its own question; the statement is in 0.2 and at the top of 11 | 5.9, P12, 11 |
| B11 | cheap missing features | added: clipping masks, Gaussian blur, sharpen, gradient tool, move selected pixels, Color Range; the rest listed for the user | 2.2, 3.3, 4.1, 4.9, 11 item 9 |
| B12 | no Revert | fixed: Revert button, one entry | 3.1, P3b test 8 |
| B13 | Alt+drag is chord-only | fixed: a visible PSD handle; Alt+drag removed (so no Alt / menu-bar hand test is needed) | 6.4 |
| B14 | Ctrl+V placed at once; styles in copies unstated | fixed: paste = Stamp tool source with preview; Paste at View Centre; copy and drag-out are "with styles (what you see)". **Changed from the suggestion:** "Paste Here" in a view context menu is not built, because a right-drag in the views is navigation and the tablet rule forbids right-click-only features; Paste at View Centre is the menu equivalent | 6.3, 6.4 |
| B15 | testability gaps | fixed: shots with `expect`; frame spans with a limit; the landing rule; `command_for` test; one badge wording | 5.5, 5.7, 8.2, 8.4, 8.6 |
| B16 | f1 uses the CLI, the user uses the GUI | fixed: P0 compares the two save paths and records it; otherwise hand test 1 is named the only proof | P0, 8.3 |
| B17 | no check for flash / sound / dialogs coming to the front | fixed: rule in 5.6, grep in the gate, hand test 14 | 5.1, 5.6, 8.6, 8.7 |
| B18 | rule check: no violations | nothing to change | - |

Rejected: none. Two findings were taken with a change, both named above (A25: stated and budgeted instead of tile sharing;
B14: no context-menu "Paste Here").

P0 check (2026-11-25): the `lyid` claim of section 1 was wrong against the code (PhotoCraft gives a new layer the lowest free
id, not one above the largest); corrected in section 1, a "`lyid` reuse" rule added to 5.4 and a test to P5; fixture f12
shows it. Details: `docs/notes/sticker/check-P0.md`.

Defaults of revision 1 that changed, for the main session to tell the user: a link needs a saved project; no PSD write at
project save; the PSD handle replaces Alt+drag; linked paint layers are now in scope (P12); two rows of upper tabs; the
packages are P0-P13.
