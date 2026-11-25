# Sticker editing and working with PhotoCraft: options and decisions

Written 2026-11-25 for the request: "integrate sticker editing just like PhotoCraft ... drag and drop between the apps
and seamlessly link photos/layers together to edit in one and move to the other."

This is the decision document. It sums up four research notes in this folder:
`pc-interop.md` (how PhotoCraft talks to other programs), `pc-features.md` (PhotoCraft's editing tools),
`wte-model.md` (how WTE stores stickers and layers) and `win-dnd.md` (Windows drag and drop and clipboard).
Details and line numbers are in those notes. This file adds a few checks of its own (marked "this note").

Marks: **VERIFIED** = read in code or ran it. **INFERRED** = reasoning, not checked directly.

Words used below:
- **Sticker** = an image stamp in WTE (a picture you place on the model, projected, wrapped or flat on the texture).
- **Link** = a file on disk that both apps use. One app saves it, the other notices and updates.
- **Clipboard** = Windows copy and paste (Ctrl+C / Ctrl+V between programs).
- **Drag out** = dragging something from one program's window and dropping it into another program.
- **Crate** = a Rust library. The project rule since round 4: no new crates without your OK.

Sources: WTE = `G:\software_projects\texture_wrapper_ap_cmp` (branch `companion` @ 90ada7b). PhotoCraft source =
`G:\Claude_Resources\ref\photocraft` (latest main, b5c7620) and its v0.2.0 tree extracted to
`G:\Claude_Resources\ref\scratch\pc020\`. Your copy = `G:\Self-made_products\photocraft-0.2.0-windows-x86-portable\`.
No GUI was started. Only `photocraft-cli.exe` ran, headless, in scratch folders.

---

## 1. What PhotoCraft is, and what "work together" can mean

### 1.1 What it is

- PhotoCraft is a **third-party** open-source program, a Photoshop-like editor. It is written in Rust with egui and
  wgpu, the same building blocks as WTE. Licence `MIT OR Apache-2.0`. The names "ArtCraft" and "PhotoCraft" and their
  logos are trademarks. VERIFIED (`pc-features.md` section 11).
- **We can change WTE. We cannot change PhotoCraft** unless you decide to fork it (make our own copy of it).
- **Your copy is v0.2.0** (2026-10-05, portable zip: `photocraft.exe` and `photocraft-cli.exe`). The latest stable
  release is **v0.23.1** (2026-11-11). Main is near v0.24.0-rc.2. That is about 16,400 changes after your copy. VERIFIED.
- v0.2.0 already has almost every sticker tool: layer styles (stroke, drop shadow, glow, bevel, colour overlay...),
  Magic Wand, Magic Eraser, Select Subject, Select and Mask, Defringe, Trim, 23 adjustments, about 120 filters, type,
  shapes and masks. Latest adds a one-click **Remove Background** and **Color to Alpha**. VERIFIED
  (`pc-features.md` sections 0, 2-8; 748 commands in `G:\Claude_Resources\ref\scratch\pcfeat\commands.txt`).
- Its background removal is classic image maths, **not AI**. So it fits WTE's "no AI" rule. It works on a clear object
  in front of a plain background. VERIFIED (`pc-features.md` section 3).

### 1.2 What PhotoCraft can and cannot do with other programs (your v0.2.0)

| Feature | v0.2.0 (yours) | Latest (v0.23.x) | Source |
|---|---|---|---|
| Open a file given on the command line | yes, always a **new window** | MSI install: opens a **tab in the running window**. Portable zip: new window | VERIFIED `pc-interop.md` 1 |
| A file dropped on it | opens as a **new document** | dropped on the picture: **added as a layer**. On the tab strip: opened | VERIFIED `pc020\crates\ui-egui\src\file_open.rs:78-88`; latest `file_open.rs:25-56` |
| Images (not files) dropped on it | refused | refused (INFERRED) | VERIFIED `win-dnd.md` 2.2 |
| **Drag something out of PhotoCraft** | **no** | **no** | VERIFIED `pc-interop.md` 3 |
| Copy and paste images with other programs | yes (pixels only, no layer name or position) | yes, and also pastes files copied in Explorer | VERIFIED `pc-interop.md` 4 |
| Ctrl+S on an opened `.psd` | **saves back into the same file** | same | VERIFIED `pc020\crates\ui-egui\src\menus.rs:403-408` |
| Ctrl+S on an opened `.png` | asks for a new name (Save As) | saves in place only while it is one plain layer | VERIFIED same place; latest `pc-interop.md` 5 |
| "Place Linked" layer follows its file | only after the manual **Update Modified Content** command | automatic, checked every 2 s | VERIFIED `pc-interop.md` 5, `win-dnd.md` 4.2 |
| Remote control by another program | only if that program starts PhotoCraft with `--control`. No Place or Export commands. No "file saved" messages | also reachable after you opt in under Preferences. Still no Place, no messages | VERIFIED `pc-interop.md` 6 |
| Headless command-line tool (`photocraft-cli`) | yes, full engine, no window, ~0.1 s a run | yes | VERIFIED (ran) |

Two more facts that shape the design:
- PhotoCraft v0.2.0 saves a document by writing straight into the file, not via a temporary file. So another program
  can see a half-written file. VERIFIED `pc020\apps\photocraft\src\services.rs:125` (`std::fs::write(path, bytes)`).
- WTE's PSD export already opens correctly in v0.2.0: names, order, visibility kept, pixels identical after a
  PhotoCraft re-save. VERIFIED (ran, `pc-interop.md` 7).

### 1.3 So what can "work together" mean without changing PhotoCraft?

Realistic (INFERRED from the table above):
1. **WTE -> PhotoCraft by drag:** yes. You drag a sticker or layer out of WTE onto PhotoCraft. WTE hands over a real
   file. In v0.2.0 it opens as a new document. In latest, dropped on the picture, it becomes a layer.
2. **PhotoCraft -> WTE by drag:** **no.** PhotoCraft cannot drag anything out. Instead:
   - copy in PhotoCraft (Ctrl+C) and paste in WTE (Ctrl+V), or
   - the **linked sticker** (section 3.3): you save in PhotoCraft and WTE updates by itself, or
   - drag PhotoCraft's saved file from Explorer into WTE (works today).
3. **"Edit in PhotoCraft" button in WTE:** yes. It opens the sticker in PhotoCraft. Every Ctrl+S there updates the
   sticker on the model in WTE within about a second.
4. **A WTE layer used inside a PhotoCraft picture:** yes, with PhotoCraft's "Place Linked" on a PNG that WTE keeps up
   to date. v0.2.0 needs you to run "Update Modified Content". Latest updates by itself.

Not possible without a PhotoCraft fork: dragging out of PhotoCraft, PhotoCraft telling WTE "I saved",
PhotoCraft updating a linked file by itself (v0.2.0), PhotoCraft writing back into a linked file. VERIFIED that the
code for these does not exist (`pc-interop.md` 8).

---

## 2. Sticker editing inside WTE

### 2.1 What WTE has today

- An image sticker is a stored picture plus one colour setting (`tint`). No crop, eraser, outline, shadow or colour
  changes. VERIFIED `crates/wte_core/src/stamp.rs:10-14, 91-101`.
- **Good news:** a placed sticker's picture can already be swapped. WTE re-fits and re-bakes it on the model as one
  undo step. The Tool page has "Image: Choose...". VERIFIED `doc_stamps.rs:443-531`, `tool_options.rs:646-655`.
  So a sticker editor only has to produce a new picture and hand it to this existing step. VERIFIED (`wte-model.md` 1.4).
- WTE already has useful pieces: an exact distance map used for text outlines (the base of a die-cut border), resize,
  crop, crop-to-content, layer blending, the brush on a plain image, and image load/save. VERIFIED (`wte-model.md` 8).
- Text stickers already have outline, warp styles and text on a path. VERIFIED.
- Earlier scope notes to revisit: HANDOFF section 6 says "No image editor needed for PSD / ORA" (that was about
  checking exports). ARCHITECTURE 1.3 lists "colour-tolerance flood fill, selections" as non-goals (that was for the
  texture fill tool). A magic wand on sticker pictures touches both. **Needs your OK.** VERIFIED (`wte-model.md` 12.11,
  `pc-features.md` 3).

### 2.2 Proposed features, in three tiers

Tier 1 - the core sticker look (small pieces, high value):
1. Edits are **kept with the sticker** and can be changed later or undone (non-destructive).
2. **Die-cut border**: an outline outside, inside or centred on the edge, any colour and width.
3. **Drop shadow**, **outer glow**, **colour overlay** (turn the sticker into one-colour silhouette).
4. **Remove a plain background**: magic eraser / wand with tolerance, and **Color to Alpha** (for logos on white or
   black, keeps soft edges).
5. **Clean the edge**: defringe, remove white halo, remove black halo.
6. **Trim** empty borders, with **automatic padding** so shadow and border are not cut off. Flip, rotate 90.
7. **Colour**: hue/saturation (and colourise), brightness/contrast, invert, black-and-white.
8. **Brush and eraser** on the flat sticker picture, with pen pressure.

Tier 2 - very useful, more work:
9. **Remove background from a photo**: draw a box around the object (GrabCut, classic maths, no AI). Optionally a
   one-click "Select Subject".
10. Refine the edge (soften, shrink/grow, colour clean-up).
11. Gradient overlay, inner shadow, simple bevel (raised look).
12. Levels, curves, posterize, threshold.
13. A **mask** you can paint to hide or show parts (instead of erasing pixels for good).
14. **Backing shapes** behind the sticker (circle, rounded box, star badge); WTE text drawn onto an image sticker.
15. WTE's 15 text warp styles applied to image stickers; style presets ("Classic sticker", "Glow", "Badge").

Tier 3 - later, or leave to PhotoCraft:
16. Quick selection brush, lassos.
17. Filters: blur, sharpen, pixelate, noise, poster edges.
18. Pattern overlay, satin, several borders, more blend modes.
19. Puppet warp, liquify, healing and clone.

Two design points (INFERRED):
- Shadow and border make the picture bigger. The editor must pad the picture first, and keep the sticker's size on the
  model the same when padding is added. Today a new picture is re-fitted by its aspect only.
- Dragging a slider must not create a stored picture each time (WTE never frees stored pictures while running). The
  editor needs a "draft" preview that is stored only when you finish. VERIFIED that blobs are never freed
  (`blob.rs:28-30`).

How the editor looks (INFERRED, `wte-model.md` 9): a new **Sticker view** in the main window, next to 3D / 2D / Split /
Preview. The sticker fills the centre, the side panel shows its tools, and the 3D result can stay visible beside it and
update live. A pop-up dialog would be simpler but blocks the 3D view while open. A second window is not recommended
(it fights the "never take focus" rule and the self-test runner).

Saving (INFERRED, `wte-model.md` 4): keeping edits re-editable needs project format 4. Older WTE builds would refuse
such projects with "made by a newer version" (they already do this for format checks). Baking each edit into a new
picture needs no format change but you could not change the border width later.

### 2.3 Three ways to build it

| | A. Write it in WTE | B. Copy PhotoCraft v0.2.0 code in as crates | C. Hand heavy edits to PhotoCraft |
|---|---|---|---|
| What it is | Port only the chosen methods into `wte_core`, using PhotoCraft's code as the reference | Add PhotoCraft's engine libraries to the repo and call them | No editor in WTE (or only Tier 1); "Edit in PhotoCraft" for the rest |
| Code WTE owns | ~2-4k lines for Tier 1+2 (INFERRED) | 35-55k lines (VERIFIED counts) | small adapter |
| New crates | none | 5-8 copied crates. Needs your OK | none |
| Fit | native to WTE pictures, undo, project file | different pixel format (float vs 8-bit), effects tied to PhotoCraft's whole layer model | PhotoCraft must be installed; editing happens in another window |
| Updates | ours to fix | frozen at v0.2.0. Latest code **cannot** be used: it needs libraries we do not have and its own copies of egui and winit (VERIFIED) | follows your PhotoCraft version |
| Effort (INFERRED) | Tier 1: about 1-2 weeks with tests and UI. Tier 2: about 1-2 weeks more | 1-2 weeks to wire in, then a large surface to test and maintain | a few days (it is the link in section 3.3) |
| Risk | results differ slightly from Photoshop/PhotoCraft | stale fork, big review and test load | depends on a third-party program you might change or remove |
| Licence | add PhotoCraft's MIT notice once for ported code | same notice | none (separate program) |

A variant of C (INFERRED): WTE buttons that run `photocraft-cli` in the background (no window), for example "Remove
background" through PhotoCraft's Select Subject. It works today (VERIFIED: a full sticker - background removed, white
border, shadow, trimmed - came out of one headless run, `pc-features.md` 9). But WTE would then quietly depend on
PhotoCraft being installed, and on command names that change between PhotoCraft versions.

---

## 3. Linking and drag and drop

### 3.1 Each direction

| Direction | Today | How it can work | Effort (INFERRED) | What you feel |
|---|---|---|---|---|
| Explorer -> WTE | files only; an image becomes the Stamp tool's picture, then you click to place | WTE installs its own drop handler: knows **where** you dropped (place the sticker right there, or drop onto a sticker to replace its picture), accepts images dragged from browsers and Office (not just files) | 2-3 days | drop a picture on the model, it lands where you let go |
| WTE -> Explorer | no | drag a sticker or layer out of WTE; WTE saves a real PNG and hands it over | 3-5 days | drag a sticker to the desktop, get a PNG |
| WTE -> PhotoCraft | no | the same drag as above | +0.5 day | v0.2.0: it opens as a new document. Latest: on the picture it becomes a layer |
| PhotoCraft -> WTE | drag the saved file from Explorer | **no direct drag** (PhotoCraft has no drag-out). Use copy and paste, or the linked sticker | clipboard 1-2 days; link 2-4 days | Ctrl+C in PhotoCraft, Ctrl+V in WTE; or just Ctrl+S in PhotoCraft |
| Copy / paste both ways | WTE has no clipboard | WTE writes and reads Windows images (the "PNG" format plus a 32-bit bitmap with transparency) | 1-2 days | paste becomes a new sticker, or replaces the selected sticker's picture |

Technical notes (from `win-dnd.md`, `wte-model.md` 10):
- All of it can be done with direct Windows calls, the same way the pen code does it. **No new crate.** VERIFIED that
  the pattern exists (`pen/win.rs:97-109`). The `windows` crate is already built into WTE through other libraries;
  using it directly would save some hand-written code but counts as a dependency change. VERIFIED (`cargo tree`).
- Drag-out traps (INFERRED from winit's code, `win-dnd.md` 3.1): Windows' drag call blocks. Started from the wrong
  place it would crash WTE's event loop, so it must start from WTE's window message hook. After a drag, WTE must tell
  egui the mouse button is up, or the next move drags whatever is under the cursor.
- **Pen risk:** dragging out with a pen may not work, because WTE reads the pen through its own path. Needs a hand test
  on your tablet. The Android tablet companion cannot drag out at all (its input is fed into WTE directly).
- **Testing limit:** a real drag between two apps cannot be automated here (no fake mouse input, no windows that take
  focus). The data, the drop handler, the file watcher and the button-up fix can be tested in code. The real drag stays
  a hand test for you, like `dist/HOW_TO_TEST.txt` for the pen.
- Clipboard tests would overwrite your real clipboard, so only the picture conversion gets automatic tests.

### 3.2 Which file format for links

| Format | WTE -> PhotoCraft | PhotoCraft -> WTE | Keeps PhotoCraft layers and effects | Verdict |
|---|---|---|---|---|
| **PSD** | WTE already writes it; v0.2.0 opens it correctly (VERIFIED) | Ctrl+S writes back in place (VERIFIED). WTE reads the flattened picture that every PSD carries (see below) | yes, inside PhotoCraft (VERIFIED: a live stroke and a mask survived, `pc-features.md` 9) | **recommended** |
| PNG | yes | v0.2.0 asks for Save As each time (VERIFIED) | no | fine for one-off drags and paste |
| `.pcraft` | only via `photocraft-cli` | WTE would need a new crate (zstd) to read it, and its inside layout changes with PhotoCraft versions (VERIFIED) | yes, plus linked layers | not recommended |
| ORA | WTE writes it | v0.2.0 cannot open ORA (VERIFIED) | - | no |

Checked for this note (VERIFIED, ran a small reader on PhotoCraft v0.2.0 PSDs in
`G:\Claude_Resources\ref\scratch\options\psdcomp.py`):
- Every PhotoCraft PSD carries a **flattened copy of the whole picture** at the end of the file: 8-bit RGB, compressed
  (RLE), 4 channels when the picture has transparency. In `pcfeat\out_subject.psd` this copy already includes the mask
  and the live white stroke (1,572 white edge pixels, 20,587 transparent pixels). So WTE does not need to redo
  PhotoCraft's effects. It reads that one picture.
- That copy is **mixed against white** at soft edges: a source pixel (0,128,255) at half transparency is stored as
  (127,191,255) (`pcinterop\linked.psd` vs `sticker2.png`). WTE must undo that mix when reading, or soft edges get a
  white halo. A fully solid picture is stored with 3 channels (`pcinterop\wte_box_rt.psd`).
- WTE's own PSD reader today is a strict test helper for WTE's own files (VERIFIED `export/psd.rs:520`). Reading the
  flattened copy is a small new reader (INFERRED: a few hundred lines with tests). Fallback: `photocraft-cli convert
  x.psd x.png` (VERIFIED works) - but that ties WTE to PhotoCraft's tool.

### 3.3 The "linked sticker" design (edit in PhotoCraft, WTE updates on save)

Works with your v0.2.0 and with latest. INFERRED design, built on the VERIFIED facts above.

What you do:
1. Select an image sticker in WTE. Click **Edit in PhotoCraft** (Tool page, layer row menu, or Layer menu).
2. PhotoCraft opens the sticker. (v0.2.0: a new window. Latest installed version: a new tab.)
3. Edit as much as you like: layers, effects, masks, text. Press **Ctrl+S**.
4. WTE notices within about a second and the sticker on the model updates. This is one undo step in WTE
   ("Update linked sticker"). Save again, it updates again.
5. The sticker shows a small "linked" badge. Menu items: **Update now**, **Show file in Explorer**, **Unlink**
   (keep the current picture, stop following the file).

What WTE does:
1. Writes the sticker's picture to a PSD with one layer. If a PSD from an earlier round trip exists, WTE reuses it, so
   your PhotoCraft layers and effects are still there next time.
2. Starts `photocraft.exe <that file>` (path set once in WTE's preferences, or found automatically). Only when you
   click; never in tests.
3. Checks the file's size and time every second and when WTE becomes active. It waits until the file stops changing
   and reads cleanly (PhotoCraft writes in place, so a half-written file is possible).
4. Reads the flattened picture, undoes the white mix, and swaps it in through the existing "replace sticker picture"
   step. If the picture got bigger (for example you added a shadow), the sticker keeps its size on the model.
5. Remembers the link in the project: file name plus last size/time/checksum. On project save, the PSD is stored
   inside the `.wtep` too, so moving the project to another PC keeps your PhotoCraft layers. Needs project format 4.
6. WTE's own sticker edits (border, shadow from section 2) stay on top of the linked picture, so both work together.

Where the linked files live (decision 7): a folder next to the project (`<project>_links\`, easy to find and to drag
from Explorer), or a hidden WTE folder under `%LOCALAPPDATA%`. Unsaved projects would use the hidden folder until
first save.

Limits (INFERRED):
- One direction at a time: v0.2.0 does not reload a file that WTE changed while it is open. While a sticker is out
  for editing, WTE should warn before its own changes to that picture's source.
- Text stickers must first be turned into image stickers (a new command; text cannot be edited in PhotoCraft as WTE
  text).

The same idea for **paint layers** ("Edit layer in PhotoCraft", the full texture as a PSD) needs a new WTE command to
replace a paint layer's pixels (none exists, VERIFIED `wte-model.md` 2). It fits a later package.

The reverse link (a WTE layer inside a PhotoCraft picture): WTE keeps an exported PNG of a chosen layer or sticker up
to date; in PhotoCraft you use File > Place Linked. v0.2.0: run Update Modified Content to refresh. Latest: refreshes
by itself. Small effort on the WTE side (WTE already exports single layers).

---

## 4. Recommendation

### 4.1 Recommended package, in order (effort INFERRED)

1. **Sticker editor, Tier 1** (approach A): Sticker view, non-destructive edit list (project format 4), border, shadow,
   glow, colour overlay, magic eraser, Color to Alpha, defringe, trim with padding, colour adjustments, brush/eraser.
   Plus "Copy / Export sticker picture" and "Convert text sticker to image". About 2 weeks.
2. **Linked sticker: Edit in PhotoCraft** (section 3.3) with the PSD flattened-picture reader and the file watcher.
   About 3-5 days. This is the "work together" core and covers every tool WTE does not have.
3. **Clipboard** copy/paste of pictures and **own drop handler** (drop at a position, images from browsers, drop onto
   a sticker to replace it). About 3-5 days.
4. **Drag out of WTE** (sticker or layer as a PNG/PSD file) to Explorer and PhotoCraft. About 3-5 days, plus your
   hand test with mouse and pen.
5. Later, after you have used it: Tier 2 features, paint-layer round trip, live-exported layers for Place Linked.

Each package ends with the usual gate (tests, clippy 0, scenarios) and a hand-test sheet for what cannot be automated.

### 4.2 Decisions only you can make

1. **Scope:** a sticker editor inside WTE replaces the old "no image editor needed" note, and a magic wand / colour
   eraser is allowed on sticker pictures (not on the texture fill tool). Options: yes / editor without the wand /
   no editor, PhotoCraft only. Recommendation: **yes**.
2. **Build approach:** A write in WTE / B copy PhotoCraft v0.2.0 crates (needs new-crate OK) / C PhotoCraft does all
   editing. Recommendation: **A for Tier 1, plus C (Edit in PhotoCraft) for everything else**.
3. **First feature set:** Tier 1 only / Tier 1+2 / everything. Recommendation: **Tier 1 first**, Tier 2 after you try it.
4. **Re-editable sticker edits:** keep the edit list (project format 4; older WTE versions cannot open those projects)
   / bake every edit into a new picture (no format change, no later tweaking). Recommendation: **keep the edit list**.
5. **Where the editor lives:** Sticker view in the main window, 3D result live beside it / a pop-up dialog.
   Recommendation: **Sticker view**.
6. **Your PhotoCraft version:** stay on portable v0.2.0 / install latest (v0.23.1 installer). Latest gives one window
   with tabs, drop-onto-picture makes a layer, linked layers refresh by themselves, Remove Background. Everything above
   works with both. Recommendation: **install latest when convenient**; WTE supports both.
7. **Link file format and place:** PSD (recommended) / PNG / .pcraft; files next to the project (`<project>_links\`)
   or hidden in app data; PSD also stored inside the `.wtep`. Recommendation: **PSD, next to the project, also stored
   in the .wtep**.
8. **Windows code:** direct Windows calls written by hand (no dependency change) / use the `windows` crate already
   built into WTE (dependency change, needs your OK; less hand-written code). For the clipboard, the usual `arboard`
   crate is not available offline. Recommendation: **hand-written calls, no new crate**.
9. **Drag out from where:** layer row and sticker thumbnail only / also from the 3D view with a modifier key.
   Recommendation: **layer row and thumbnail** (no clash with painting and orbiting).
10. **photocraft-cli as a helper:** never / optional fallback (for example opening `.pcraft`) / hidden engine behind
    WTE buttons. Recommendation: **optional fallback only**; WTE reads PSD itself.
11. **Fork PhotoCraft** (to get drag-out of PhotoCraft, "saved" messages, auto-refresh in v0.2.0): no / later.
    Recommendation: **no**; latest PhotoCraft already covers most of it, and the link above covers the rest.
12. **Branch and release:** build on `companion` (pen PR #3 and the companion are not on main yet) and ship as part of
    v0.3.0 / land pen and companion first, then start from main. Recommendation: **land pen and companion first**.

---

## Critic

Completeness critic, 2026-11-25. I checked the key claims of this file and the four notes against
`G:\Claude_Resources\ref\photocraft` (v0.2.0, v0.23.1 tree entries, HEAD b5c7620), the v0.2.0 extract
`G:\Claude_Resources\ref\scratch\pc020\`, WTE `texture_wrapper_ap_cmp` @ 90ada7b and `REG\winit-0.30.13`. Nothing was
started or built. Git was run with `GIT_NO_LAZY_FETCH=1 -c gc.auto=0`.
Labels: **CONFIRMED wrong** = a note says something the code contradicts. **Gap** = missing from this decision
document. **Risk** = a threat to the plan or to WTE's rules. Each line is marked VERIFIED or INFERRED as above.

### C. Claims that are wrong

C1. **CONFIRMED wrong**: `pc-interop.md` 5 says "No automatic reload of linked files in either version ... latest has
no watcher (VERIFIED grep)". The same mistake is in 7 and 8 ("still missing in latest: automatic linked-file refresh").
Latest has `crates/engine-core/src/link_watch.rs` (lines 1-21). It compares size and modification time every 2 s and
when the window comes back to the front. It runs on a worker thread and makes one undo step per file. There is also a
`layer.smartObjects.updateChanged` command (`smart_cmds.rs:280`). The file is in the v0.23.1 and v0.10.0 trees as
`crates/engine/src/link_watch.rs` (VERIFIED, tree entries). Section 1.2 and `win-dnd.md` 4.2 are right. Limits from the
module doc (VERIFIED): it watches only the **active** document. A file seen for the first time is only remembered. A
file that changed while its document was closed waits for the manual Update Modified Content.

C2. **CONFIRMED wrong**: `win-dnd.md` 4.1 and 5.4 say that v0.2.0's `--control` channel can run "any engine command,
e.g. `file.placeLinked`, `layer.smartObjects.updateModifiedContent`". That is not true. With `--control`,
`apps/photocraft/src/main.rs:122-123` always passes an automation workspace, so `services.rs:93-97` installs
`authorize_desktop_engine_command`. `ui-egui/src/control.rs:80-85` checks it for `engine.execute` and `ui.menu.invoke`,
and `menus.rs:100-105` and `lib.rs:360-364` check it for synthetic input. `automation/src/workspace.rs:131-167` refuses
every `file.*` except a short safe list, and every `layer.smartObjects.*` except `convertToSmartObject` (VERIFIED).
`pc-interop.md` 6 and section 1.2 above are right. Correction: over control, WTE gets no Place, no Place Linked, no
Update Modified Content and no Export Layers.

C3. **CONFIRMED wrong**: section 1.1 says "The names 'ArtCraft' and 'PhotoCraft' and their logos are trademarks". Only
the **ArtCraft** name, wordmark and mark in `docs/brand/` are trademarks (`NOTICE:6-8`; `docs/brand/LICENSE-brand.txt`;
v0.2.0 `README.md:327-330`, VERIFIED). The PhotoCraft app icon is `MIT OR Apache-2.0` (`ATTRIBUTION.md:24`,
`assets/app-icon/`, VERIFIED). See R5 for what still applies.

C4. **CONFIRMED infeasible as written**: `wte-model.md` 10.1 suggests starting `DoDragDrop` "from `App::logic` of the next
frame". `logic` (`wte_app/src/app.rs:317`) runs inside winit's event handler, like `ui`. In
`winit-0.30.13/.../event_loop/runner.rs:208-238`, a `RedrawRequested` from the nested modal loop calls
`call_event_handler`, and its `take().expect("either event handler is re-entrant ...")` would fire (VERIFIED code;
INFERRED that the panic happens). Only the `win-dnd.md` 3.1 pattern works, and section 3.1 above uses it: post `WM_APP`,
then call `DoDragDrop` from WTE's window-procedure subclass. Add a re-entry guard, because egui frames keep running
during the drag and must not start a second drag (INFERRED).

C5. **Outdated "unknown" claims**: `pc-interop.md` says the v0.2.0 UI code is missing from the clone. It calls the drop
behaviour and the Save target "unknown" and Image Assets "after each save" INFERRED. All 828 v0.2.0 blobs are present
now (VERIFIED). The v0.2.0 code settles each question:
- drop = open as a new document: `pc020\crates\ui-egui\src\file_open.rs:76-88`;
- Save in place only for psd/psb/pcraft: `menus.rs:219-226, 402-408`;
- every GUI save runs the `saveDocument` script events and regenerates Image Assets: `ui-egui/src/lib.rs:525` ->
  `engine/src/automate_cmds.rs:268-271` -> `web_cmds.rs:1145-1153`.

### D. Doors this file leaves out

D1. **Gap, VERIFIED**: **Generate > Image Assets** on v0.2.0 is a working PhotoCraft -> WTE path for single layers, and
section 3 does not mention it. A layer named like `badge.png` is written to `<document>-assets\badge.png` on every
Ctrl+S (C5). The file is trimmed to the layer's bounds, so its position is lost (`pc-interop.md` 5, ran). WTE could
watch that folder and get one PNG per named layer, with no PSD reading. The on/off switch is stored in session state
(`Session.file_menu.image_assets: Vec<DocId>`, `automate_cmds.rs:33`), so the user probably has to turn it on again
after reopening the document (INFERRED). The Script Events `saveDocument` -> Layers to Files binding
(`pc-interop.md` 5) also runs on every GUI save (VERIFIED, same call). Both need setup in PhotoCraft by the user.

D2. **Checked, no use**: other PhotoCraft doors that the notes did not list.
- **Plug-ins**: `crates/plugins` (v0.2.0 and HEAD) are sandboxed WebAssembly filters. They have "no host imports: no
  WASI, file system, network, clock" (`plugins/src/lib.rs:1-20`, VERIFIED), so a plug-in cannot reach WTE.
- **URL scheme**: there is none. No `URL Protocol` or `photocraft://` in `packaging/` or `apps/` at HEAD (VERIFIED grep).
- **`links.rs`**: it only holds Help-menu web links (VERIFIED).
- **Tablet crate**: `crates/tablet` (v0.23.1 and HEAD, not in v0.2.0) uses Windows WH_GETMESSAGE and WH_CALLWNDPROC
  hooks plus WinTab to read barrel and eraser (`crates/tablet/src/lib.rs:1-30`, `Cargo.toml`, VERIFIED). It is a
  reference for WTE's pen code, not a link. v0.2.0 reads only touch force (`pc020\...\stylus.rs:1-20`).
- **External editor**: latest PhotoCraft's only external-editor round trip is for SVG and VectorCraft
  (`external_svg.rs`, `external_svg_portal.rs` = Linux xdg portal). There is none for raster layers (VERIFIED).
- **Recovery and autosave**: not an exchange path (`pc-interop.md`).

D3. **Gap, VERIFIED**: PhotoCraft has no "sticker" feature. The word appears only in a v0.2.0 test
(`pc020\crates\engine\src\vp_cmds.rs:223`) and a HEAD test, and the 748-command list has none. "Sticker editing just
like PhotoCraft" therefore means PhotoCraft's general editing applied to stickers. Decision 1 should also ask: does
the user want a **layered** sticker (several layers inside one sticker: picture + text + shape, like a small PhotoCraft
document)? The tiers above are a single-picture edit list.

### E. Sticker features the user will expect but the tiers leave out (INFERRED)

E1. **Die-cut contour**: a smooth outline with rounded joins and holes filled, plus a "contour smoothness" control. A
plain Photoshop-style Stroke follows every dent and hole (`pc-features.md` 2, chamfer metric), and that does not look
like a cut sticker. Small to add on WTE's EDT: dilate, fill holes, then blur the threshold.
E2. **Crop / resize / free rotate** of the sticker picture, and **shape crops** (circle, rounded box). Tier 1 has only
trim, flip and rotate 90.
E3. **The same effects on text stickers**: drop shadow, glow, border. Text stickers have only an outline today.
E4. **Edit before placing**: edit the Stamp tool's picture before placing it. `wte-model.md` 11 lists this hook
(`state.tools.stamp.source`); the tiers do not.
E5. **Copy / paste sticker style**, and apply one edit to all selected stickers (PhotoCraft has
`copyLayerStyle` / `pasteLayerStyle`, VERIFIED registry).
E6. **Shared pictures**: two stickers that use the same picture share one deduplicated blob (`blob.rs:44-67`, VERIFIED).
Editing or linking one must ask "this sticker only / all that use it".
E7. **Save the edited sticker for reuse** in other projects: a small sticker library, or at least "Export sticker
picture" (Tier 1 only).

### R. Risks

R1. **Risk, VERIFIED (latest only)**: "every PhotoCraft PSD carries a flattened copy" is true for v0.2.0, which always
writes the real merged image (`pc020\crates\io\src\psd_export.rs:602-607`, matted against white). Latest has
Preferences > File Handling > **Maximize PSD and PSB File Compatibility** (`engine-core/src/prefs.rs:633, 685`, values
Always / Never / Ask, default Always). When it is off, the file holds Photoshop's **white placeholder**
(`io/src/psd_export/merged.rs:186`, `psd_export/document.rs:364`), and resource 1057 says
`has_real_merged_data = false` (`pc020\crates\psd\src\builder.rs:1-5`). Decision 6 recommends latest, so WTE's
flattened-picture reader must check resource 1057 and refuse with a clear message (or use `photocraft-cli convert`).
It must also handle these cases:
- the 4th channel is transparency only when the layer count is negative; saved alpha channels and the Quick Mask
  come after it (`psd_export.rs:613-630`);
- 16-bit, 32-bit and CMYK documents;
- PSB.

R2. **Risk, INFERRED from tree entries**: the "Latest (v0.23.x)" column above is based on HEAD (~v0.24.0-rc.2) code.
`window_drop.rs` is not in the v0.23.1 tree, and the v0.23.1 `file_open.rs` blob is missing locally. So "drop on the
picture = new layer" is verified only for HEAD, not for the v0.23.1 installer decision 6 points to. `link_watch.rs` and
`single_instance.rs` are in v0.23.1 (VERIFIED tree). Write "HEAD" where HEAD is meant, or check the installed build by
hand.

R3. **Risk, INFERRED**: dropping back onto WTE. When WTE becomes a drag source while winit's own drop target is still
registered, a drag that ends over WTE gives WTE a `DroppedFile` of its own temp file, which `plan_drop` imports as a
new stamp source. winit's `QueryInterface` is `unimplemented!()` (`drop_handler.rs:53-60`, VERIFIED): a panic across
FFI if OLE ever calls it. Order the packages so WTE's own drop target (package 3) is in place before drag out
(package 4), and recognise WTE's own drags by the private format. Section 4.1 already has this order; make it a rule.
Also tell apart a drag that leaves the window from the in-window layer reorder (`panels/layers.rs:213-241`).

R4. **Risk, focus rule, INFERRED**:
- `photocraft-cli.exe` is a console program. Started from WTE (a GUI program) without `CREATE_NO_WINDOW`, it opens a
  console window that takes focus. Decision 10 must require that flag, as `companion/adb.rs` already does.
- Self-tests and scenarios must never start `photocraft.exe`: use a stub launcher and a gate, like `WTE_PEN_SUBCLASS`.
- A link update must not bring WTE to the front.
- Latest's single-instance hand-off brings PhotoCraft's window forward (`main.rs:247-290`). That is fine only because
  the user clicked.
- Polling "every second" should run on a worker thread that calls `request_repaint` only on a change. Do not keep
  WTE's reactive frame loop busy.

R5. **Risk, licence and brand, VERIFIED terms**:
- Ported code: keep "Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors" with the MIT notice
  (`pc020\NOTICE:1-4`).
- Do not use the ArtCraft name or marks in WTE in any way (`LICENSE-brand.txt`, Conditions).
- "Edit in PhotoCraft" as plain text is a factual reference and does not suggest endorsement (INFERRED as acceptable).
  PhotoCraft's app icon may be shown with its MIT notice.
- A fork (decision 11) must remove the ArtCraft marks and must not call itself ArtCraft (`LICENSE-brand.txt`, "Forks
  and modified versions").
- None of the ported algorithms comes from GPL code in `ATTRIBUTION.md` (VERIFIED grep: the Breeze theme is original,
  the face model is BSD-3 and not needed).

R6. **Risk, memory, INFERRED from `blob.rs:28-30`**: every PhotoCraft Ctrl+S on a linked sticker adds a new blob, and
blobs are never freed. If the PSD is also kept as a blob for the `.wtep`, each save adds a whole PSD. Keep only the
latest PSD outside the blob store, or allow freeing blobs that history no longer uses. Undoing "Update linked sticker"
must not make the watcher apply the file again: compare file stamps, not pictures.

R7. **Risk, two copies, INFERRED**: v0.2.0 has no single instance. Clicking "Edit in PhotoCraft" twice opens two
windows on the same PSD, and the last save wins. WTE should remember the files it has handed out and warn. A PSD
dragged out of WTE also opens with WTE's own file as its path (C5), so a later Ctrl+S writes into WTE's folder. Either
watch those files as links on purpose, or drag out PNG only.

R8. **Risk, finding the exe, VERIFIED**: the portable v0.2.0 writes nothing to the registry (`pc-interop.md` 1), so
"found automatically" (3.3, step 2) works only for the MSI install. For the user's copy, WTE must ask for the path once.

R9. **Gap, INFERRED**: the tablet companion mirrors only WTE's window. The PhotoCraft path cannot be used from the
Galaxy Tab, but the in-WTE Sticker view can. This is one more reason for approach A and the Sticker view.

R10. **Gap, INFERRED**: once WTE has an OS clipboard for pictures, users will expect Ctrl+V of **text** from other
apps into the text-sticker field. egui still uses its in-app string clipboard (`wte-model.md` 5.3). It is cheap to add
with the same user32 code.

R11. **Risk, project rules, VERIFIED**:
- All five notes in this folder were written with **CRLF** line endings (305/411/366/292/354 CR lines; the repo has
  `core.autocrlf=false`), which breaks the "LF" rule. This critic converted `OPTIONS.md` to LF when appending. The other
  four still need `sed -i 's/\r$//'`.
- The read-only reference clone was changed during research: `pc-features.md` and `win-dnd.md` note that git's
  automatic repack ran. The v0.2.0 blobs that `pc-interop.md` reported missing (587/828) are now all present, so a
  partial-clone lazy fetch (network) probably ran during `git archive v0.2.0` (INFERRED; 36 promisor packs dated
  2026-11-25 12:59). Future runs should use `GIT_NO_LAZY_FETCH=1 -c gc.auto=0` only.

### Corrections to apply to the sections above

- 1.1: the trademark is "ArtCraft" only (C3).
- 1.2 "Latest" column: label it HEAD where only HEAD was checked (R2). Add an Image Assets row (D1).
- 3.2 PSD row: "every PSD carries a flattened copy" holds for v0.2.0 always, and for latest only while Maximize
  Compatibility is on. The reader checks resource 1057 (R1).
- 3.3: no automatic exe discovery for the portable copy (R8). Guard against two open copies (R7). Do not keep a new PSD
  blob for every save (R6).
- 4.2: decision 1 also asks "layered sticker or edit list" (D3). Decision 10 requires `CREATE_NO_WINDOW` (R4).
  Add E1-E7 to the tier choice in decision 3.
