# Feature 1 — Brush types (Circular, Sphere), a brush shape loaded from a model file, and a Brush tab

Round 4 design document for Wrapped Texture Editor. Written 2026-11-17. Design only: nothing here is implemented, no
source file was changed, nothing was built or run.

This document merges three proposals ("lean", "engine", "user") after three judges reviewed them. It starts from the
"engine" proposal, takes over the ideas the judges asked for, and corrects the errors they listed. Every judge
statement was checked against the source before it was used; appendix A lists what was done with each one and the
few places where a judge's statement did not hold as written.

**Revised 2026-11-17 after a review by three independent skeptics (code, data, user).** Every finding was checked
against the source. Section 10 lists each finding in one line with what was done. The larger changes: the stroke
pacing of 5.10 (stored view transforms, a per-frame time rule with a bounded trail, strokes that finish after the
release), the bake of 5.7 (the brush is sized from the closed solid that was found, not from the whole file), the
hardness fade of 5.2 (measured against the local thickness), a second way to use a shape, "Through", for flat and
thin shapes (5.6), and the life cycle of the loaded shape (4.2 to 4.4). The text as it was before the review is
kept in `target\round4_design\work\FEATURE_BRUSH.before_review.md`.


How to read it:

- All paths are below `E:\claude_programs\wrapped_texture_editor\`. `core\` means `crates\wte_core\src\`, `app\` means
  `crates\wte_app\src\`. `brush.rs:64` without a prefix means the core file.
- Every statement about today's code carries a `file:line` reference that I read myself in the main tree on
  2026-11-17. The main tree is the state before round 3 (it is identical to the round-3 base copy; round 3 works in
  copies). Round 3 will move line numbers in the files it owns, so each package must find its lines again.
  "(map, not re-read)" marks the few facts taken from the two code maps in `target\round4_design\work\brush\`.
- "Texel" is one pixel of a texture. "Set" is one texture set (one paintable texture). "Dab" is one print of the
  brush; a stroke is a row of dabs. "Tip" is the volume of the brush: a ball, or a shape. "Ring" is the brush cursor
  drawn on the model.
- "Estimate" marks a number that was calculated, not measured. "Not verified" marks something that could not be
  checked. Section 8.4 says how each estimate is measured and what the pass limit is.

The request, word for word:

> BRush type: circular vs sphere, where circular is what you have and sphere will glide along an object and paint
> whatever is in its sphere. i want to be able to upload and orient the shape and size of the "sphere" into any
> compatible model type. can be its own tab like in photoshop.

---

## 0. The decisions on one page

1. **Circular** is today's default brush, pixel for pixel: a ball test plus the existing side filter. Nothing the
   user has today changes without their say. One new checkbox, "Stay on connected surface" (default off), limits
   Circular to the piece of surface under the cursor.
2. **Sphere** paints every surface texel inside the ball: any side, any part, hidden or not. This is today's brush
   with "Same side only" switched off, plus a new Offset that lifts the ball off the surface or sinks it in.
3. The checkbox "Same side only" is removed. The Type choice replaces it.
4. The ball of the Sphere type can be replaced by a **shape**: a built-in Cube or Cylinder, or a model file (OBJ,
   glTF, GLB, FBX). A model is converted once, in a background job, into a grid of signed distances (at most
   128 x 128 x 128 values, 6 MiB with the thickness grid). "Inside" is everything that cannot be reached from
   outside without crossing the model's surface, so the winding of the file, duplicated faces and inner walls do
   not matter. When a file holds more than one closed solid (a floor, hair cards, open parts), only the closed
   solid is used and the user is told. A flat sheet that lies in one of the file's main planes is accepted as an
   outline. A model that encloses nothing otherwise (a cup, a tilted sheet) is refused with a message that says
   what to change.
5. **Size** keeps its meaning: the diameter of the ring. The solid found in the file is scaled to fit inside the
   ring (exactly when the whole file is that solid, within 0.8 % of Size otherwise).
6. **Orientation**: the shape stands on the surface (the file's +Y axis points away from the surface, towards the
   viewer; its +X axis points to the right on screen, as stamps do) and is turned with three angles. A second mode
   keeps it fixed in space.
7. **Hardness, Opacity, Flow, Spacing** work on a shape as on the ball. One formula, the existing `falloff`, serves
   both. For a shape the fade is measured against the thickness of the part a point lies in, so thin parts of a
   shape are painted as solidly as thick ones.
8. **Cursor**: the ring of today (now hidden on faces Circular will skip), plus, for a shape, a bold outline of its
   footprint and three thin wire cross-sections. No GPU pipeline changes; one shader condition is added.
9. **Brush tab**: the bottom area of the side panel gets two tabs, "Layers" and "Brush". That is the only place where
   the tab fits at 1280 x 800 with the colour picker fully visible (pixel budget in 6.1).
10. **Persistence**: brush settings and the path of the loaded shape go into `prefs.json` (a few hundred bytes).
    The project file `.wtep` does not change. Undo does not change.
11. **Speed**: a measurement comes first. Then one BVH query per dab instead of one per texture set. The pointer
    path is queued with the camera of each moment, so no part of it is skipped or moved. Stroke work takes 10 ms
    per frame while it keeps up; when a quarter second of work is waiting, frames may grow to 100 ms; a stroke
    that still trails is completed in the frames after the release, visibly, and no command throws it away by
    accident. No frame, the release frame included, does more than 100 ms plus one sample of stroke work.
12. **2D view**: every type paints the flat disc it paints today. The Brush tab says so at all times.
13. **Solid or Through**: a shape is used either as a volume ("Solid": paint lands where the surface passes through
    the shape) or as a cookie cutter ("Through": its outline, seen from above, is pressed straight through the
    surface inside the ring). Through is what makes a flat or thin shape, such as a star, print a star on a curved
    model; it is chosen automatically when such a shape is loaded.

Not in the first build (section 9.2 gives the reasons): turning the shape with the stroke direction, shape strokes
steered from the 2D view, stretching per axis, a shape library, an exact cut-line cursor on the GPU, closing openings
of non-closed models, outlines of flat sheets that are tilted in their file, F6 or any new key.

---

## 1. What the user gets

### 1.1 Two brush types

A **Type** choice with two values. The Brush and the Eraser each have their own.

- **Circular** (default). The brush as it is today: a round spot on the surface you point at, which also follows
  the surface over edges and around bends. It does not paint the far side of a thin part. A checkbox "Stay on
  connected surface" (default off) makes it stop at the part under the cursor; with the box off it also colours
  other parts inside the ring where they face roughly the same way, exactly as today.
- **Sphere**. A ball glides over the model with its centre on the surface under the cursor. Everything inside the
  ball is painted: the far side of a thin wall, both flanks of a deep fold, neighbouring parts, parts hidden inside
  other parts. Sphere differs from Circular only where the ball reaches a far side, a fold or another part. With
  the default Size of 32 px on a plain solid model the two paint the same pixels; the hover text of the Sphere
  button says so.

### 1.2 A shape instead of the ball

With Type = Sphere the ball can be replaced by a shape.

- **Shape** list: Ball, Cube, Cylinder, and the loaded model file (one at a time, shared by Brush and Eraser).
- **Load...** opens a file dialog for OBJ, glTF, GLB and FBX. The model is read in a background job with a progress
  window and a Cancel button. A cube of 12 triangles is ready in under 1 second, a 500,000-triangle model within
  8 seconds (limits in 8.4).
- The model is used as a solid: every surface texel inside the solid is painted. If the file holds more than the
  closed solid (a floor under it, open or very thin parts), the brush is made from the closed solid alone and a
  message says so.
- **Use: Solid / Through.** Solid: paint lands where the model's surface passes through the shape. On a curved
  surface a flat shape is then cut off where the surface bends away from it: a star plate that is 5 % as thick as
  it is wide paints only its middle on a ball. Through: the outline of the shape, seen from above, is pressed
  straight through the surface inside the ring, whatever the curvature: the star prints a star. Loading a shape
  whose height is under 25 % of its width switches to Through and says so.
- A flat sheet without thickness (a star cut from a plane) that lies in one of the file's main planes is accepted
  as an outline and used Through. A shape that lies flat in its file (thin along X or Z) is laid flat on the
  surface automatically.
- A model that encloses nothing else is refused: "This model has no closed inside that the brush can use. If it
  is flat or very thin, give it more thickness (2 % of its longest side or more), or lay it flat in one of the
  file's main planes so that it can be used as an outline. If it has openings, close them in a 3D program first."
- **Size** scales the whole shape: the solid always fits inside the ring.
- **Turn, Tilt, Roll** turn the shape about its own axes. **Align** chooses between "Follow surface" (the shape
  stands on the surface wherever you paint) and "Fixed in space" (it keeps the orientation of its file).
- **Offset** lifts the tip off the surface or sinks it in. It chooses which slice of a shape cuts the surface.
- The loaded shape, its angles and the type are remembered after a restart. "Remove file shape" in the Shape list
  forgets it.

### 1.3 A Brush tab

The bottom area of the side panel, which shows the Layers list today, gets a strip with two tabs: **Layers** and
**Brush**. The Brush tab holds everything about the tip. Size, Hardness, Opacity, Flow and Spacing stay in Tool
options, so they are visible together with the colour picker and the Brush tab.

### 1.4 Every control and its default

Tool options, section "Brush options" / "Eraser options" (existing controls, `app\panels\tool_options.rs:144-164`):

| Control | Range | Default | What it does after this change |
|---|---|---|---|
| Size | 1 to 2048 px (texels) | 32 px | Diameter of the ring, as today. A shape is scaled to fit inside the ring. Also Ctrl+wheel over a view (new). |
| Hardness | 0 to 100 % | 80 % | 100 % = hard edge. Lower values fade the paint from the middle of each part of the tip towards its skin: at 80 % the inner 80 % of every part, thick or thin, is solid. For the ball this is exactly today's fade from the centre to the rim. |
| Opacity | 1 to 100 % | 100 % | Unchanged: the most one stroke can cover. |
| Flow | 1 to 100 % | 100 % | Unchanged: build-up per dab. |
| Spacing | 2 to 100 % | 12 % | Unchanged: distance between dabs in % of Size. |
| "Same side only" | | | Removed. On = Circular, off = Sphere with the Ball. |

Brush tab (new). It edits the tip of the Eraser while the Eraser is the active tool, otherwise the tip of the Brush.

| Control | Shown for | Range | Default | What it does |
|---|---|---|---|---|
| Type: Circular / Sphere | always | two buttons | Circular | Section 1.1. Hover texts: "Circular: the round brush on the surface you point at. It does not paint the far side of thin parts." and "Sphere: a ball that glides over the model. Everything inside it is painted, also the far side and hidden parts. Differs from Circular where the ball reaches a far side, a fold or another part." |
| Stay on connected surface | Circular | checkbox | off | On: only the surface connected to the cursor point inside the ring. Off: every face in the ring that faces the same side, as today. |
| Shape | Sphere | Ball, Cube, Cylinder, the loaded file, "Remove file shape" | Ball | The volume that glides over the model. The file entry shows the file name; it is greyed out while no file is remembered. Choosing the entry of a remembered file that is not loaded yet loads it in the background. "Remove file shape" forgets the file. |
| Load... | Sphere | button | | File dialog "Load Brush Shape", then a background job with a progress window. On success the tool whose tab this is (the Eraser if it is active, otherwise the Brush) gets Type = Sphere and Shape = the file, and Use is set as the shape needs. A failed or cancelled load leaves the shape in use untouched. |
| Preview | Sphere, shape other than Ball | 72 x 72 px picture | | Three wire cross-sections of the shape as it sits on the surface, with a line for the surface. Dragging in the picture turns the shape: left/right changes Turn, up/down changes Tilt, 0.5 degrees per pixel. |
| Turn | Sphere, shape other than Ball | -180 to 180 degrees | 0 | Spin about the axis that points out of the surface. Positive = clockwise as seen from outside, like the stamp Rotation (`core\stamp_bake.rs:134-135`). Also Shift+wheel. |
| Tilt | same | -180 to 180 degrees | 0 | Leans the shape forward or back. |
| Roll | same | -180 to 180 degrees | 0 | Leans the shape sideways. |
| Offset | Sphere | -100 to +100 % of the radius | 0 % | Moves the tip along the surface normal towards the viewer. 0 = centre on the surface. +100 % = a ball only touches from outside. -100 % = it only touches from inside. |
| Align | Sphere, shape other than Ball | Follow surface / Fixed in space | Follow surface | Follow surface: the shape's up axis follows the surface under the brush. Fixed in space: the shape keeps the axes of its file wherever you paint. |
| Use | Sphere, shape other than Ball | Solid / Through | Solid (Through for a flat or thin loaded shape) | Solid: paint lands where the surface passes through the shape. Through: the outline of the shape, seen from above, is pressed straight through the surface inside the ring. Fixed on Through for a flat outline. |
| Reset | Sphere | button | | Sets Turn, Tilt, Roll and Offset to 0 and Align to Follow surface. |
| Note lines | always | text | | Line 1 says what the chosen type paints (texts in 6.3, at most 44 characters each). Line 2 is always: "The 2D view always paints a flat disc." |

### 1.5 Mouse and keys

| Input | Today | New |
|---|---|---|
| `[` `]`, `Shift+[` `Shift+]` | size, hardness | unchanged |
| Ctrl+wheel with Brush or Eraser | zooms the view (the brush has no wheel handler: `app\tools\brush.rs:214-301`; the default returns `false`, `app\tools\mod.rs:191-193`) | changes Size by a factor of 1.1 per notch, clamped to 1..2048 (the Stamp tool uses the same gesture for its size, `app\tools\stamp_place.rs:16-17, 305-309`) |
| Shift+wheel with Brush or Eraser | zooms the view | Type Sphere with a shape other than Ball: changes Turn by 5 degrees per notch (as the Stamp tool rotates, `stamp_place.rs:310-314`). Otherwise it still zooms. |

No new key is bound.

**This changes two gestures that work today, and the request did not ask for it.** With the Brush or the Eraser,
Ctrl+wheel and Shift+wheel zoom the view today; after this round Ctrl+wheel changes Size, and Shift+wheel turns a
shape (it still zooms when there is no shape to turn). The wheel without a key is not changed and still zooms.
The reason is that the Stamp and Text tools already use both gestures this way. Because it is a change of existing
behaviour, it is question Q9 (section 2), it is named in the gate report as a deliberate change, and it is built by
the brush tool package alone: removing it again is the deletion of one `scroll` function.

### 1.6 The 2D (texture) view

A stroke in the 2D view stays what it is today for every type: a flat disc in the texture that ignores the mesh
(`dab_2d` takes a scratch buffer, a centre and a radius in texels and no mesh, `brush.rs:838`; it is called by
`texture_dab`, `stroke.rs:433-448`). Type, shape and angles act on strokes made on the model in the 3D view.

### 1.7 What each type paints

Brush radius 0.2 units in all rows.

| Situation | Circular (= today's default) | Circular + "Stay on connected surface" | Sphere |
|---|---|---|---|
| Smooth connected surface, cube edge or corner, UV seam | round patch, as today | same pixels | same pixels, plus the far side where the ball reaches it |
| Thin wall: a plate 0.05 thick, cursor on the front | front face and the narrow rim faces; not the back face | same | front, rim and the back face |
| Concave fold, inner angle 90 degrees | both flanks | both flanks | both flanks |
| Concave fold, inner angle 60 degrees | only the flank under the cursor | same | both flanks |
| Two separate parts 0.01 apart (a piece standing on a board) | the part under the cursor, and on the other part every face that faces within 101.5 degrees of the cursor's normal, also where it is hidden | only the part under the cursor | every face of both parts inside the ball |
| A part sunk into another (sample "head" sunk 0.05 into the "body", `core\samples.rs:143-161`) | both, also their hidden portions | only the one under the cursor | both |
| Two fingers 0.1 apart, joined only outside the ring | both | only the finger under the cursor | both |
| One connected surface with an undercut ridge between the cursor and a plateau behind it (ridge wall turned more than 101.5 degrees away) | the plateau is painted, the wall is not | the plateau is not painted, because the connection runs through the wall | everything |

The fold limit follows from the existing side test: a triangle is kept when its normal is at most 101.5 degrees away
from the normal under the cursor (dot product at least -0.2, `brush.rs:64, 227-229`). The flanks of a fold with inner
angle `t` have normals `180 - t` degrees apart, so the other flank is painted when `t` is 78.5 degrees or wider.

No type checks whether a face is visible from the camera.

With the default settings (Size 32 px, a radius of 0.068 units on the built-in Cube, which is 1.5 units thick) the
two types paint the same pixels on every built-in sample: the ball is far too small to reach a far side. The
difference shows on thin parts, in folds, next to other parts, and with large sizes. Test C5 has a case at a
realistic size (a plate 0.05 thick, brush diameter 64 texels).

What a shape paints, by Use (shape radius 0.4 units, on a ball of radius 1.0 as the model):

| Shape | Solid | Through |
|---|---|---|
| Cube, Cylinder, a chunky model | the cut of the volume with the surface | the outline of its middle slice, a little larger than the Solid cut near the rim |
| A star plate, thickness 5 % of its width | only the middle, out to about half the radius: the surface drops 0.083 units at the ring, the plate is 0.02 units thick each way | the whole star |
| A flat star without thickness | not available | the whole star |

### 1.8 What does not change

Strokes still paint into every texture set they reach. One stroke is one undo step. Save, reopen and every export are
untouched, because the feature changes no document data. Settings of the Brush and of the Eraser stay separate. The
colour picker stays where round 3 pins it.

---

## 2. Unclear words: the reading chosen, the alternatives and their cost, the question

What the code does today matters for the first two rows (details in section 3): a dab is already a ball test. With
"Same side only" on (the default) it paints every UV-mapped texel within the radius whose triangle faces within 101.5
degrees of the cursor point's normal, on any part, hidden or not. With it off it paints everything in the ball.

| # | Words in the request | Reading chosen as the default | Alternatives and their extra cost | Exact question for the user |
|---|---|---|---|---|
| 1 | "circular is what you have" | Circular = today's default brush, bit for bit (ball test + side filter). The user's own definition. Every existing brush test and scenario keeps describing Circular. | (a) Circular stays on the connected piece of surface under the cursor. Built as the checkbox "Stay on connected surface"; making it the default is one changed default value. Cost of the checkbox: a neighbour table per document, built at the first such stroke (estimate 26 to 54 MiB and 0.1 to 0.3 s at 1.5 million triangles), about 400 lines. (b) A circle on the screen projected onto what the camera sees. A different tool (projection painting): needs a visibility test per texel and changes today's result everywhere. Not planned. | **Q1.** "Today the brush also colours other parts that lie inside its ring when they face roughly the same way, even hidden ones: painting the foot of a chess piece also colours the board under it. Should Circular keep doing exactly that (planned default; a checkbox 'Stay on connected surface' is there but off), should that checkbox be on from the start, or do you not need the checkbox at all?" |
| 2 | "sphere ... paint whatever is in its sphere" | Every UV-mapped surface texel whose 3D point is inside the ball: any side, any part, hidden or not. | Only surfaces the camera can see: one ray per texel, about 3.3 million rays for one dab of 2048 texels. Not affordable; not planned. | "Should Sphere also paint surfaces inside the ball that you cannot see, such as the back of a wall and hidden parts? Planned: yes." |
| 3 | "glide along an object" | The ball's centre rides on the first surface under the cursor, as today's dab centre does (`stroke.rs:186, 225`). Half the ball is inside the object. Offset moves it. | The ball always rests on top of the surface. A ball then touches a flat surface in one point and paints almost nothing. Available as Offset = +100 %. | "Should the centre of the ball ride on the surface (planned), with a slider to lift it off or sink it in?" |
| 4 | "upload" | Load a file from this computer through a file dialog. Nothing is sent anywhere. | Dropping a model file on the Brush tab: a dropped model means "new document" today (`app\dispatch.rs`, map, not re-read); about 40 lines and one more rule to explain. Not planned. | "'Upload' is read as 'load a model file from this computer'. Correct?" |
| 5 | "the shape ... of the 'sphere'" | The ball is replaced by the solid of one model file at a time, plus built-in Cube and Cylinder. The file is remembered by its path. | A shape library: shapes copied into a folder on E:, a list with names and thumbnails, Remove. A new file format, a third place the program writes to on its own, about 600 lines. | **Q4.** "The program remembers which shape file you loaded by its path and reads it again, in the background, the first time it is needed after each start. If you move or delete the file, the brush falls back to the ball and says so; the path stays remembered until you load another shape or choose 'Remove file shape'. Is that enough, or do you want shapes copied into a library folder on E:?" |
| 6 | "orient" | Three angles measured against the surface: the shape stands on the surface (file +Y away from the surface, +X to the right on screen at the press) and follows it along the stroke. A switch keeps it fixed in space instead. | (a) Turn with the stroke direction: the first dab of each stroke has to wait for a second sample; about 150 lines. (b) Handles in the 3D view: the stamp gizmo edits document stamps with undo and has one rotation axis (map, not re-read); a new interaction, about 400 lines. Neither is planned. | **Q7.** "A loaded shape stands on the surface with the model file's +Y axis pointing away from it; you turn it with three angles, Shift+wheel or by dragging the preview. Is +Y the axis you expect, and is that enough, or do you need the shape to turn with the stroke direction?" |
| 7 | "size" | The existing Size slider. The shape is scaled as a whole so that it fits exactly inside the ring. The ring stays the true outer limit; a cube's side is 58 % of Size. | (a) Size = the shape's longest side (a 100 px cube is 100 px wide). The shape then reaches up to 1.73 times beyond the ring; about 3 times the texels visited per dab and about 5 times the search volume. One changed line in the mapping of 4.3. (b) Separate width, height, depth: breaks the distance measure the hardness fade uses; about 120 lines. Not planned. | **Q2.** "Size is the diameter of the ring, and a loaded shape is scaled to fit inside the ring, so a cube's side is 58 % of Size. Is that right, or should Size be the shape's longest side? The second makes large shape brushes about 3 times slower." |
| 8 | "into any compatible model type" | The formats the importer reads: OBJ, glTF, GLB, FBX (`core\import\mod.rs:91-109`; the model dialog lists the same four, `app\io_flows.rs:540-549`). Second meaning, true by construction: the brush works on every model the app can open. "Compatible" for a brush shape: the file must hold a closed solid (other parts of the file are ignored, with a message), or be a flat sheet in one of the file's main planes (used as an outline). | Closing openings automatically (a cup becomes a plug shaped like the cup). The cheap rule (count walls in six directions) also fills deep hollows of open models; a better rule needs a second algorithm. About 150 lines and hard-to-predict results. | **Q3.** "A model with an opening (a cup, a head open at the neck) has no inside and is refused with a message; closed parts of the same file are still used. Should the program instead close openings by itself? It would then also fill deep hollows of such models." |
| 9 | "can be its own tab" | A tab "Brush" next to "Layers" in the bottom area of the side panel. | (a) Tabs "Tool / Brush" at the top: at 1280 x 800 only about 130 to 146 px remain there once the colour picker is pinned, so the tab would scroll (6.1). (b) A floating window: the app has no floating panels; it would cover the 3D view. | **Q5.** "The Brush tab shares its place with the Layers list, because that is the only part of the side panel where it fits at 1280 x 800 with the colour picker visible. While it is open you see the name of the active layer but not the list. Is that acceptable?" |
| 10 | "like in photoshop" | The tab holds the tip controls of Photoshop's Brush Settings panel: tip shape, angle, hardness (Size, Hardness, Spacing, Opacity and Flow stay in the tool options, as Photoshop's options bar also has them). | Photoshop's dynamics: jitter, scattering, texture, dual brush, wet edges. Each is its own feature. Not planned. | "Photoshop's panel also has jitter, scattering, texture and dual brushes. None of these is planned for this round. Correct?" |

One more question covers small extras that two judges wanted and one wanted confirmed:

**Q6.** "Four small extras are in the plan. Say no to any you do not want: built-in Cube and Cylinder shapes; the
Offset slider; the 'Fixed in space' alignment; turning the shape by dragging its preview picture."

**Q8.** "A flat or thin shape, such as a star, is used as a cookie cutter: its outline is pressed straight through
the surface ('Through'), so a star prints a star also on a curved model. A chunky shape is used as a volume
('Solid'). The program picks one when you load a shape and you can switch. Is that what you expect from a flat
shape, or should a flat shape be wrapped onto the surface like a stamp?" (Wrapping is what the Stamp tool does and
is not planned for the brush.)

**Q9.** "With the Brush or the Eraser, Ctrl+wheel will change the brush size and Shift+wheel will turn a loaded
shape, as both already do for stamps and texts. Today both zoom the view with these two tools; the wheel without a
key keeps zooming. Is that change welcome, or should Ctrl+wheel and Shift+wheel keep zooming?" (Planned: the
change. A "no" removes one function from the brush tool package.)

Part of Q1, after the review: with the default Size the two types paint the same pixels on a plain solid model
(1.7). If the user expects a difference at first try, the cheapest change is to switch "Stay on connected
surface" on by default, which shows on every model with more than one part.

None of the questions blocks the build: each has a working default, and each answer changes one default value or
removes one package (Q1: package C; Q6: single controls; Q8: the Use control; Q9: the wheel function).

---

## 3. What exists today

### 3.1 One 3D dab is already a ball test

- One dab is `dab_3d` (`brush.rs:724-765`; its comment calls it "One spherical dab on the surface", line 716),
  called once per texture set by `Document::surface_dab` (`stroke.rs:395-430`; the loop over all sets is line 398,
  the call is lines 415-427).
- Candidates: `bvh.query_sphere(mesh, center, radius + 0.5 * aa, tris)` (`brush.rs:365`), where `aa` is the set's
  mean world size of one texel (`brush.rs:209-212`). The query is repeated once with `radius + 0.5 * local_max` when
  a candidate has coarser texels, and only when `local_max > aa * 1.001` (`brush.rs:368-375`). A triangle's own texel
  size counts at most 4 times the mean (`brush.rs:66, 216-223`).
- A set the stroke has not reached yet first runs one more query into a newly allocated `Vec`
  (`dab_reaches_set`, `brush.rs:234-250`, called at `stroke.rs:406`). It uses the first radius, `radius + 0.5 * aa`
  (`brush.rs:248`). There is one BVH for the whole mesh (`core\document.rs:35`), so a dab on a model with N sets runs
  at least N sphere queries over the same ball.
- `query_sphere` accepts a triangle when the squared distance of its closest point is at most the squared radius
  and returns the ids sorted (`core\bvh.rs:439-481`; the test is line 467, the sort line 480).
- The only filter: the triangle belongs to the set and, with `same_side_only`, `geo_normal(tri).dot(normal) >= -0.2`
  (`brush.rs:64, 227-229`). -0.2 is the cosine of 101.5 degrees. `tri_set` is `None` for triangles without UVs
  (`core\mesh.rs:120-127`). There is no visibility test and no connectivity test: `brush.rs` calls no BVH function
  other than `query_sphere` (lines 248, 365, 374).
- `normal` is the stroke's reference normal: the unflipped shading normal of the hit (`stroke.rs:187`;
  `Hit::surface_normal`, `core\bvh.rs:43-47`). The viewer-facing normal is the separate field `Hit::normal`
  (`core\bvh.rs:32-34`). A hit on a back-facing triangle still paints with the default settings (test at
  `stroke.rs:734-745`).
- Coverage per texel: `falloff(|P - center|, radius, hardness, tri_aa)` with `P` the texel's 3D point from the
  barycentric weights (`brush.rs:328-332`); `falloff` is `brush.rs:84-109`. Within a dab a texel keeps the maximum
  (`brush.rs:516-522`).
- Each triangle's clip box in texture space is derived from the circle in which the ball cuts the triangle's plane
  (`brush.rs:394-413`). Texels outside the box are never visited, so the box is not only a speed-up.
- A brush with a radius under one texel is forced hard (`brush.rs:414-416`).
- Two fast paths for tiled UVs assume a ball of full strength around the centre: `core_radius`
  (`brush.rs:309-312`), `in_full_core` (`brush.rs:318-323`), `BlockFold::in_core` (`brush.rs:282-290`).
- A dab only raises a 16-bit mask per set (`commit_dab`, `brush.rs:670-714`).
- Test-build timing counters for candidates, raster, commit and re-apply exist (`brush.rs:767-796`); the one bench
  that prints them is ignored (`stroke.rs:936-937`).

### 3.2 The stroke

- `stroke_begin(&BrushParams)` stores a clamped copy of the parameters for the whole stroke (`stroke.rs:161-177`,
  `sanitize` at 88-97). `BrushParams` is `Copy` (`brush.rs:28`) and three places rely on it: `..*brush`
  (`stroke.rs:95`) and `let brush = session.brush;` (`stroke.rs:397`, `444`).
- One test helper lists every field of `BrushParams` without a default tail: `random_brush`
  (`crates\wte_core\tests\end_to_end.rs:755-766`). So does the app (`app\tools\brush.rs:61-70`). Adding a field to
  `BrushParams` would break both. This design adds none.
- A sample is a ray hit: `stroke_to_3d(&Hit, pressure)` uses `hit.position`, `hit.surface_normal()` and
  `hit.texture_set` (`stroke.rs:183-232`). The session remembers position, normal and pressure of the previous sample
  only (`stroke.rs:47-51`).
- Dabs between two samples lie on the straight 3D line between them (`stroke.rs:218`), one every
  `max(spacing * 2 * radius, half a texel)` (`stroke.rs:207`). Two samples are joined only when at most 6 radii apart
  and their normals do not oppose (`stroke.rs:33, 192`).
- After each dab the changed rectangles are re-applied from the pre-stroke tiles (`stroke.rs:452-482`).
  `stroke_end` pushes one `Edit::Multi` of `Edit::Pixels`, labelled "Brush Stroke" or "Erase" (`stroke.rs:288-312`).
  The entry holds pixels only.
- A stroke buffer (3 bytes per texel) is created when a stroke first reaches a set (`stroke.rs:377-392`;
  `brush.rs:150-168`).
- `texture_dab` (2D) calls `dab_2d` and never looks at the mesh (`stroke.rs:433-448`).

### 3.3 The brush tool in the app

- Settings: two `BrushSettings { size_px, hardness, opacity, flow, spacing, same_side_only }`, one for the brush and
  one for the eraser (`app\state.rs:154-172, 239-249`). They are saved nowhere: `Prefs` has eight other fields
  (`app\state.rs:458-475`) and the per-project view state has six other fields (`app\io_flows.rs:35-46`). The comment
  that the settings structs "are written into `Document::ui_state` and `Prefs`" (`app\state.rs:5-8`,
  `ARCHITECTURE.md:2753-2755`) is wrong for tool settings.
- The parameters of a stroke are built from the settings at the press (`app\tools\brush.rs:59-71`). A 3D stroke's
  radius is `size_px * 0.5 * doc.texel_world_size(set)` of the set under the cursor at the press
  (`app\tools\brush.rs:116`; `core\document.rs:147-150`) and is frozen for the stroke
  (`ActiveStroke::radius_world`, `app\tools\brush.rs:25-26, 125`).
- The drag is walked in screen space in steps of `max(1 point, a quarter of the ring radius on screen)`, one raycast
  per step, at most 64 per frame (`app\tools\brush.rs:13, 151-183`; step at line 160).
- **A defect found by the engine proposal and confirmed here:** `walk_3d` always heads for the one `target` it was
  given (`app\tools\brush.rs:155-163`), and `route_pointer` calls `drag` once per motion event with that event's
  position (`app\view\mod.rs:375-378`). When the 64 raycasts are used up in the middle of a segment, the next call
  continues from that middle point straight to the newest pointer position. The pointer positions in between are
  skipped, so the painted path depends on where the budget ran out.
- A frame without pointer motion still calls `drag` (`app\view\mod.rs:405-412`), but only if there is a frame:
  nothing in the tool or view code requests a repaint while a stroke is open (the call sites are listed in 5.10).
  With a still pointer the only source of frames is the thumbnail timer of the Layers and Texture sets lists, at
  200 ms (`app\panels\mod.rs:35, 140-141, 162-163`).
- On release up to 4,096 samples finish the last segment in one frame (`app\tools\brush.rs:15, 247-257`). Because
  earlier pointer positions were skipped, that is the rest of one segment only.
- `Tool::finish` (Save, Save As, Export while the button is held) gets no view transform
  (`app\tools\mod.rs:198, 442-448`); today's brush ends the stroke there without a trailing segment
  (`app\tools\brush.rs:259-264`).
- `ToolController::release` returns to Idle whatever the tool did (`app\tools\mod.rs:409-415`). A tool change, a
  view-mode change, a guarded command, a new document, an opening dialog and Esc cancel a running stroke
  (`app\dispatch.rs:33-43, 125-129, 417, 574, 632, 757-763`; `app\io_flows.rs:472-474`; `app\view\mod.rs:75-76`).
- The self-test runner requests a repaint in every frame (`app\selftest\mod.rs:255`) and feeds pointer positions
  per frame, not per second (`selftest\mod.rs:236-244, 566-568`), so a scenario cannot see a stroke that stalls
  for lack of frames, and its frames stretch instead of its backlog growing.
- `BrushTool` has no `scroll` handler; Ctrl+wheel and Shift+wheel go to the tool first and zoom the view when the
  tool does not take them (`app\view\mod.rs:420-430`).
- Pen pressure is the constant 1.0 (`app\view\mod.rs:472`, `app\tools\brush.rs:166`).
- An existing test reads the tool's private state after a budget-limited drag and must keep passing:
  `long_drag_respects_the_raycast_budget_and_finishes_on_release` (`app\tools\brush.rs:431-450`; the test view is
  800 x 600 points, `app\tools\mod.rs:537-542`).

### 3.4 The cursor

- 3D: `BrushTool::brush_ring` returns centre, radius and hardness, with the radius recomputed every frame from the
  live settings and the set under the cursor (`app\tools\brush.rs:295-300`), so during a stroke it can differ from the
  stroke's frozen radius. The renderer packs it into two uniform vectors: `brush` = centre and radius,
  `brush_aux.x` = hardness, `brush_aux.yzw` = 0 (`app\gpu\renderer.rs:41-44, 115-120`).
- The mesh fragment shader draws a white and black double line where `distance(world, centre)` equals the radius,
  on every visible fragment, and a thin inner line at `radius * hardness` (`app\gpu\shaders\mesh.wgsl:135-136,
  175-186`). It does not look at `same_side_only`. The unflipped vertex normal is available in the shader as
  `in.normal` (`mesh.wgsl:45-58`).
- `ViewUniforms` is 176 bytes; a test compares its size and member offsets with every shader
  (`app\gpu\renderer.rs:30-49, 753-785`). `params[3]` is written as 0 (`renderer.rs:129`).
- The mesh pipeline shares its layout (view uniforms plus set) with the 2D texture pipeline
  (`renderer.rs:380-381, 406-419`). The shader rule is "WebGL2-level features only: uniform buffers, 2D textures,
  instancing" (`ARCHITECTURE.md:3196`).
- 2D: two egui circles; the tool draws nothing itself in a 3D view (`app\tools\brush.rs:274-293`). The 3D view calls
  the tool's `overlay` with a painter after the rendered image, without a depth test
  (`app\view\view3d.rs:56-60`).

### 3.5 The side panel, with pixels

- `show_side_panel` shows a resizable bottom area for the Layers section (default 40 % of the panel height, limited
  to 160..420 points) and above it one scroll area with Tool options, Color and Texture sets
  (`app\panels\mod.rs:255-282`).
- "Layers" is an `egui::CollapsingHeader` inside the bottom area, and `layers::show` clears and refills
  `state.panel_rects` every frame (`app\panels\layers.rs:81-90`). One control test clicks that header
  (`app\control_sweep\panel_tests.rs:967-985`).
- Brush options are five slider rows and the checkbox (`app\panels\tool_options.rs:144-167`) inside a collapsing
  header with the id `side_tool_options` (`tool_options.rs:39-41`). Two control tests use the checkbox
  (`panel_tests.rs:358-386` and `1015-1036`); the first finds the five sliders by the start of their hover texts
  (line 365).
- There is no tab widget and no state for one. The nearest thing is a row of `egui::Button::selectable`
  (`app\view\mod.rs:141-155`). No row of the panel may be wider than 300 points (`ARCHITECTURE.md:3479-3483`); the
  label column is 72 points (`tool_options.rs:21`).
- Round 3 changes this layout: Tool options and Texture sets get their own scroll area, Color is pinned below them
  and must be complete at 1280 x 800 for every tool, and the plan says "A separate Brush tab is planned for round 4;
  keep the layout code easy to extend, but do not build tabs now" (`target\round3_audit\ROUND3_PLAN.md:303-311`).
  When I read the round-3 working copy, its `show_side_panel` was still the old one
  (`target\round3\work\pkg_c\crates\wte_app\src\panels\mod.rs:278-297`), so the final layout code is not verified.
- Pixels at 1280 x 800, read off two screenshots (accuracy about 5 px):
  - The side panel runs from y = 27 to y = 775: 748 px. The bottom area is 299 px (40 %). The area above it ends at
    about y = 474, so it is about 447 px high (`target\round3_audit\runs\paint3d_builtin\shot.png`).
  - "Brush options" takes 168 px: header 24, five slider rows of 24, checkbox row 24 (same screenshot, y 27..195).
  - The complete Color section takes about 315 px in `runs\default_builtin\shot.png`, where it is fully visible
    (its header starts at y = 83, the next header at y = 398). Two judges and one proposal read about 305 px from
    screenshots in which the second swatch row is cut off. So the true value is between 305 and 315 px.
  - With Color pinned and complete, **about 130 to 146 px remain** for Tool options and Texture sets at
    1280 x 800 (447 px minus 305 to 315 px, with the reading error). Today's Brush options (168 px) do not fit
    there. Without the checkbox row they need 144 px.

### 3.6 Loading files, jobs, preferences

- Panels never open dialogs. They push an action with `None`; dispatch resolves the path, except in self-test mode,
  where `None` is ignored (`app\dispatch.rs:175-195`). The model dialog has the title "New from Model"
  (`app\io_flows.rs:539-549`).
- Jobs run on their own thread and return a `JobOutput`, an enum with six variants (`app\jobs.rs:67-78, 106-134`).
  A job logs "job ... finished in ... s" (`jobs.rs:122`). The system-font scan is the existing non-modal job
  (`app\io_flows.rs:449-458`). While a modal job runs, no other job may start (`io_flows.rs:460-467`); a non-modal
  job does not block a new one, any number can run (`jobs.rs:96-98, 106-134`), and only modal jobs can be asked to
  stop (`jobs.rs:166-171`).
- Failed jobs report through `report_failure`: an error toast and a message window; a cancelled job is an info
  toast (`io_flows.rs:80-98`). An error toast fails a self-test scenario (`app\selftest\mod.rs:281-288`).
- `import_from_path` always decodes textures and keeps every file's bytes (`core\import\mod.rs:111-118, 133-163`; the
  `true` is at line 152). `import_core`, which can skip textures, is private (`import\mod.rs:218-227`); with the
  switch off no texture is read (`load_textures`, `import\mod.rs:341-357`). Import never checks whether a mesh is
  closed (`import_core`, `import\mod.rs:218-270`, contains no such check).
- The only tool setting that points at a loaded file today, the stamp image, is a blob of the open document and
  needs one (`app\dispatch.rs:713-724`).
- `prefs.json` lives under `%APPDATA%\WrappedTextureEditor` (`app\prefs.rs:14-21`). An invalid file is replaced by
  defaults as a whole (`prefs.rs:52-58`). Preferences are loaded at start (`app\app.rs:141`) and written at exit
  (`app.rs:320-321`) **and every time a project is opened or saved** (`remember_recent`,
  `app\io_flows.rs:239-245`), never in self-test mode. A setting that lives in `Prefs` is mirrored when it changes
  (the swatches, `app\panels\color.rs:19`). Loaded numbers are sanitized where it matters (the window size,
  `prefs.rs:80-88`). The core clamps hardness, opacity, flow and spacing of a stroke but the radius only to "not
  negative" (`stroke.rs:88-97`), so an absurd `size_px` from a file would reach a stroke
  (`app\tools\brush.rs:116`). Only `prefs.json` and `log.txt` may live under `%APPDATA%`
  (`ARCHITECTURE.md:3921-3923`).
- The app already paces work by the wall clock (`PreviewBakeClock`, `app\state.rs:548-580`). The contract forbids
  only the core from reading the clock (`ARCHITECTURE.md:419`) and says "The app bounds the work per frame (at most
  64 surface samples per frame, section 8.6); the core does not time-slice" (`ARCHITECTURE.md:2078`).

### 3.7 Geometry helpers that exist

- BVH: nearest ray hit, any-hit and the sphere query (`core\bvh.rs:427-481`), and the free function
  `closest_point_on_triangle` (`bvh.rs:519-520`). The ray test accepts `t` in a closed interval
  (`bvh.rs:376`), rejects triangles nearly parallel to the ray (`bvh.rs:262-266`), and is slightly inclusive at
  triangle edges (`bvh.rs:82-84`). Equal distances go to the smaller triangle id (`bvh.rs:379`). Nodes are private
  (`bvh.rs:62-65`); node boxes are padded by two units of rounding (`bvh.rs:328-329`).
- Positions are compared by bit pattern after adding 0.0, so -0.0 equals +0.0 (`bits3`, `core\mesh_build.rs:107-109`,
  used by `wire_edges` at line 426). `MeshData` has no neighbour data.
- A `Document` is built by one struct literal (`core\doc_build.rs:337`); the BVH and the texel maps are built in one
  `rayon::join` (`doc_build.rs:280-293`).
- The stamp tool builds its frame from the viewer-facing normal and the camera's up vector
  (`app\tools\stamp_place.rs:177-178`; `Projector::from_surface`, `core\stamp_bake.rs:70-84`; camera basis
  `app\view\camera.rs:35-44`).
- Test fixtures (`core\samples.rs`): `seam_cube` 117-141, `two_material_model` 147-162, `wrapped_quad` 180-185,
  `flat_quad` 190-194 (unit quad with identity UVs), `no_uv_tetra` 197, `occluded_quads` 222-227,
  `reverse_winding` 254, `to_obj` 386.
- The built-in Cube document is `seam_cube(1.5)` on a 1024 x 1024 texture (`core\doc_build.rs:374-378`). Each face
  island is 277.3 x 448 texels (`seam_cube_island`, `samples.rs:104-110`): 184.9 texels per world unit across and
  298.7 down. Its mean texel size is 0.0042555 world units (`core\texel_map.rs:474-477`). Footprints on that
  document are therefore not square in texels.

### 3.8 Measured speed today

Round-3 audit log `target\round3_audit\runs\workflow_path_glb_samples_ABeautifulGame.glb\shot.log`: the model has
1,498,080 triangles and 15 sets (line 44); "Creating document" took 0.97 s (line 42); its import took 2,353 ms
(line 31). During three 3D strokes with a 50-texel brush the frame segment `pointer/tools/hover` averaged 40.5, 34.6
and 59.4 ms per frame with single frames of 244.9, 594.3 and 440.2 ms, at 18.4, 19.7 and 13.0 frames per second
(lines 92, 103, 114). The three pointer steps took 3,315, 671 and 2,805 ms (lines 93, 104, 115).

On `tex_8192.obj` (one 8192 x 8192 set) the same brush averaged 10.1 ms with one frame of 683.2 ms in the first
stroke, then 3.9 ms (`...\workflow_path_tex_8192_tex_8192.obj\shot.log`, lines 75, 96).

So today's brush already misses an interactive frame rate on the 1.5-million-triangle model. How that time splits
between raycasts, queries, per-triangle setup, rasterizing, re-apply and first-use allocations is **not verified**:
no log separates them.

---

## 4. Data model, file format, versioning, undo

### 4.1 Core types

New file `core\brush_tip.rs`:

```rust
/// Which surface inside the tip receives paint.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SurfaceReach {
    /// Same side as the reference normal AND connected to the cursor point inside the tip's ball (5.5).
    Connected,
    /// Same side as the reference normal: today's `same_side_only == true`.
    SameSide,
    /// Everything: today's `same_side_only == false`.
    All,
}

/// How a tip is turned as it moves. Ignored by a ball.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipAlign { Surface, World }

/// The volume of the brush.
#[derive(Clone, Debug)]
pub enum TipVolume { Ball, Field(std::sync::Arc<TipField>) }

/// Everything about a stroke's tip that is not in `BrushParams`.
#[derive(Clone, Debug)]
pub struct BrushTip {
    pub volume: TipVolume,
    pub reach: SurfaceReach,
    pub align: TipAlign,
    /// The user's rotation in the tip's own axes. Ignored by a ball.
    pub rotation: glam::Quat,
    /// Shift of the tip centre along the viewer-facing normal, in radii, -1..=1.
    pub offset: f32,
    /// World length of one tip unit, in brush radii. 1.0 = the shape fits exactly inside the ring. 0.25..=4.
    pub scale: f32,
    /// false = Solid (the volume), true = Through (the outline along tip Y, inside the ring's ball). Ignored by a
    /// ball; always treated as true for a flat outline.
    pub through: bool,
    /// World direction that counts as "up on screen" for this stroke (the camera's up at the press); +Y if unknown.
    pub up_hint: glam::Vec3,
}
impl BrushTip {
    /// The tip `stroke_begin` uses: a ball, `SameSide` or `All`, no offset, scale 1.
    pub fn legacy(same_side_only: bool) -> BrushTip;
}

/// World directions of the tip's X, Y, Z axes (columns) for a dab whose viewer-facing normal is `facing`, before the
/// user's rotation, and the tangent to carry to the next dab (5.3). `prev_up` = the tangent the previous dab returned.
pub fn tip_frame(facing: Vec3, up_hint: Vec3, prev_up: Option<Vec3>) -> (Mat3, Vec3);

/// Where the last dab of the open stroke put the tip (for the cursor).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TipPose { pub center: Vec3, pub world_from_tip: Mat3, pub unit: f32 /* world length of one tip unit */ }
```

New file `core\tip_field.rs` (the grid and its lookup; no mesh code):

```rust
/// A solid as a grid of signed distances in tip units (the shape fits inside the unit ball around the origin).
/// Negative inside. Immutable; shared through `Arc`; `Send + Sync`.
pub struct TipField { /* dims: [u32; 3], origin: Vec3, cell: f32, dist: Vec<i16>, thick: Vec<u8>,
                         slice: Slice2 { dims: [u32; 2], origin: Vec2, cell: f32, dist: Vec<i16>, thick: Vec<u8> },
                         inradius, bound, bounds: Aabb, sections: [Vec<[Vec2; 2]>; 3], report: TipReport, hash: u64 */ }
#[derive(Clone, Debug, PartialEq)]
pub struct TipReport {
    pub triangles: u32, pub grid: [u32; 3], pub inside_fraction: f32,
    pub thin: bool,          // some part is thinner than the grid can show
    pub partial: bool,       // the file held more than the closed solid; only the solid is used (5.7 step 6)
    pub flat: bool,          // a flat outline without a volume (5.7b); Through only
    pub up_axis: u8,         // the file axis that became tip +Y: 1 = Y (unchanged), 0 = X, 2 = Z (5.7 step 11)
    pub height_ratio: f32,   // extent along tip Y / largest extent along tip X or Z; 0 for a flat outline
}
impl TipField {
    pub fn cube() -> TipField;         // half side 1/sqrt(3) = 0.57735, written from the box formula
    pub fn cylinder() -> TipField;     // axis Y, radius 0.70711, half height 0.70711, from the formula
    /// Signed distance to the skin at a point in tip units; defined for every `q`. (`F` of 5.6.)
    pub fn sample(&self, q: Vec3) -> f32;
    /// Local thickness at `q` in tip units, never less than the depth of `q` (`T` of 5.2).
    pub fn thickness(&self, q: Vec3) -> f32;
    /// Signed distance to the outline of the middle slice y = 0, in the plane (`G` of 5.6), and its local thickness.
    pub fn outline(&self, x: f32, z: f32) -> f32;
    pub fn outline_thickness(&self, x: f32, z: f32) -> f32;
    pub fn outline_bounds(&self) -> [Vec2; 2];
    pub fn is_flat(&self) -> bool;
    pub fn inradius(&self) -> f32;     // largest depth inside, tip units
    pub fn bound(&self) -> f32;        // radius of a ball around the origin that holds the zero level:
                                       // largest distance of an inside node + sqrt(3) * cell
    pub fn bounds(&self) -> Aabb;      // box of the inside nodes, grown by one cell, tip units
    pub fn cell(&self) -> f32;
    pub fn report(&self) -> &TipReport;
    pub fn hash(&self) -> u64;         // fnv1a64 over dims and distances
    /// Outline segments of the cuts x = 0, y = 0, z = 0 (for the wire cursor and the preview).
    pub fn principal_sections(&self) -> &[Vec<[Vec2; 2]>; 3];
    /// Outline of the cut with the plane through `origin` spanned by unit vectors `u`, `v`: marching squares on an
    /// (n+1) x (n+1) lattice over [-bound, bound]^2, as segments in plane coordinates.
    pub fn section(&self, origin: Vec3, u: Vec3, v: Vec3, n: u32) -> Vec<[Vec2; 2]>;
    /// Test seam: a field from a formula evaluated at the nodes.
    pub(crate) fn from_fn(dims: [u32; 3], origin: Vec3, cell: f32, f: impl Fn(Vec3) -> f32 + Sync) -> TipField;
    /// Test seam: a flat outline from a two-dimensional formula evaluated at the nodes of the slice.
    pub(crate) fn outline_from_fn(dims: [u32; 2], origin: Vec2, cell: f32, f: impl Fn(Vec2) -> f32 + Sync) -> TipField;
}
```

New file `core\tip_bake.rs` (model to grid, section 5.7):

```rust
#[derive(Debug)]
pub enum TipError { Import(ImportError), NoInterior, TooManyTriangles { count: u32, limit: u32 }, Cancelled }
impl std::fmt::Display for TipError { /* plain sentences, 6.6 */ }
impl TipField {
    pub fn bake(mesh: &MeshData, progress: &dyn Progress) -> Result<TipField, TipError>;
    pub fn bake_from_path(path: &Path, progress: &dyn Progress) -> Result<TipField, TipError>;
}
```

New file `core\topology.rs` (only used by `SurfaceReach::Connected`):

```rust
pub struct MeshTopology { /* weld: Vec<u32> per vertex; vert_start: Vec<u32>; vert_tris: Vec<u32> */ }
impl MeshTopology {
    pub fn build(mesh: &MeshData) -> MeshTopology;
    pub fn welded_vertex_count(&self) -> u32;
    pub fn welded(&self, vertex: u32) -> u32;
    pub fn triangles_at(&self, welded: u32) -> &[u32];      // ascending triangle ids
}
```

Changes to existing core files:

```rust
// document.rs: one new private field, filled at first use, never saved
pub(crate) topology: std::sync::OnceLock<Arc<MeshTopology>>,
pub fn mesh_topology(&self) -> Arc<MeshTopology>;      // builds it on first call
pub fn topology_built(&self) -> bool;                  // for tests and the memory report

// bvh.rs
/// As `query_sphere`, and also returns each triangle's squared closest distance to `center` (the same value the
/// acceptance test of `query_sphere` computes). Sorted by triangle id. `out` is cleared first.
pub fn query_sphere_dist(&self, mesh: &MeshData, center: Vec3, radius: f32, out: &mut Vec<(TriId, f32)>);

// stroke.rs (impl Document)
/// As `stroke_begin`, with an explicit tip. `brush.same_side_only` is NOT read: `tip.reach` decides.
pub fn stroke_begin_with(&mut self, brush: &BrushParams, tip: &BrushTip) -> Result<(), EditError>;
/// Pose of the last dab of the open 3D stroke; `None` without one.
pub fn stroke_tip_pose(&self) -> Option<TipPose>;

// import\mod.rs
/// Geometry only: reads the model file and the side files its geometry needs, decodes no texture, generates no UVs,
/// keeps no file bytes.
pub fn import_geometry_from_path(path: &Path, progress: &dyn Progress) -> Result<MeshData, ImportError>;
```

Rules:

- `BrushParams` is not changed and stays `Copy`. No struct literal in the tests or in the app breaks
  (`end_to_end.rs:755-766`, `app\tools\brush.rs:61-70`), and `stroke.rs:95, 397, 444` compile unchanged.
- **One source of truth.** `stroke_begin(&brush)` becomes
  `stroke_begin_with(brush, &BrushTip::legacy(brush.same_side_only))`. `BrushParams::same_side_only` is read by this
  legacy entry point only. Test C3 proves that the legacy path equals reach `SameSide` / `All` bit for bit and that
  `stroke_begin_with` ignores the field.
- `dab_3d` (`brush.rs:724`) keeps its signature and its behaviour. It is the reference the new shared path is tested
  against (C2). Strokes no longer call it.
- The session clamps the tip like the brush: `offset` to -1..=1 (NaN becomes 0), `scale` to 0.25..=4 (NaN becomes
  1), a rotation that is not finite or has no length becomes the identity and is normalized, an `up_hint` that is
  not finite or has no length becomes +Y. `through` is forced to `true` for a flat outline.
- `from_fn` and `outline_from_fn` fill the thickness grids with the same transform as the bake (5.7 step 9), so
  fields made from a formula behave like baked ones. The thickness transform lives in `tip_field.rs` and the bake
  calls it.
- `stroke_begin_with` with `SurfaceReach::Connected` calls `mesh_topology()`, which builds the table at the first
  such stroke of a document. No document pays for it otherwise.
- `import_geometry_from_path` is `import_file` (`import\mod.rs:133-163`) with the texture switch off, `auto_uvs`
  off and the bundle dropped. A model without UVs is fine.

### 4.2 App types

```rust
// state.rs
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum BrushType { Circular, Sphere }
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ShapeChoice { Ball, Cube, Cylinder, File }

pub struct BrushSettings {            // #[serde(default)], as today
    pub size_px: f32, pub hardness: f32, pub opacity: f32, pub flow: f32, pub spacing: f32,   // unchanged
    pub kind: BrushType,              // Circular
    pub connected_only: bool,         // false
    pub shape: ShapeChoice,           // Ball
    pub through: bool,                // false = Solid, true = Through (5.6)
    pub turn_deg: f32, pub tilt_deg: f32, pub roll_deg: f32,     // 0, 0, 0
    pub offset: f32,                  // 0.0, -1..=1
    pub align: wte_core::TipAlign,    // Surface
}
// `same_side_only` is removed.
impl BrushSettings {
    /// Every value forced into the range of the table in 1.4; a value that is not finite becomes the default.
    /// size 1..=2048, hardness 0..=1, opacity and flow 0.01..=1, spacing 0.02..=1, angles wrapped into -180..=180,
    /// offset -1..=1.
    pub fn sanitized(self) -> BrushSettings;
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SideTab { Layers, Brush }

/// The one loaded model shape, shared by brush and eraser. Three independent parts, so that a new load or a failed
/// load never takes away the shape that strokes are using.
pub struct BrushShapeState {
    pub current: Option<ReadyShape>,              // what strokes use
    pub pending: Option<PendingShape>,            // a load that is running
    pub last_error: Option<(PathBuf, String)>,    // the last load that failed, for the "!" marker
    pub next_request: u64,                        // counts up with every load that is started
    pub warned_missing: bool,                     // the "no shape loaded" toast was shown in this program run
}
pub struct ReadyShape { pub path: PathBuf, pub name: String, pub field: Arc<wte_core::TipField> }
pub struct PendingShape { pub request: u64, pub path: PathBuf, pub by_user: bool, pub for_eraser: bool }

// AppState gains:
pub side_tab: SideTab,                               // Layers at start; not saved; survives a document change
pub brush_shape: BrushShapeState,                    // all empty at start
pub builtin_tips: [Option<Arc<wte_core::TipField>>; 2],   // Cube, Cylinder; made on first use
pub side_rects: SideRects,                           // written by the panels every frame; read by tests (6.1)
// pub struct SideRects { pub panel: Rect, pub tool_area: Rect, pub tool_options: Rect, pub color: Rect,
//                        pub bottom_area: Rect, pub brush_tab: Option<Rect> }   (all in points; Default = empty)
// `brush_tab` is `Some` exactly in the frames in which the Brush tab's content was drawn.

// Prefs gains (all #[serde(default)]):
pub brush: BrushSettings,
pub eraser: BrushSettings,
pub brush_shape_path: Option<PathBuf>,

// action.rs
LoadBrushShape(Option<PathBuf>),      // the user's "Load...": None = ask with the file dialog; modal job
ReloadBrushShape,                     // load `prefs.brush_shape_path` again, quietly, non-modal
RemoveBrushShape,                     // forget the loaded shape and its path

// jobs.rs
BrushShape { request: u64, path: PathBuf, result: Result<wte_core::TipField, String> },

// gpu\params.rs: BrushRing gains one field
pub side_normal: Option<glam::Vec3>,  // Some = draw the ring only on faces of that side
```

`AppState` derives nothing (`app\state.rs:503-504`), so the new fields need no derives. `View3dParams` keeps its
`PartialEq` (`app\gpu\params.rs:20-36`).

Struct literals that list every field and therefore break when a field is added (all fixed by the scaffold, 7.3):

- `BrushRing { center, radius, hardness }` at `app\gpu\mod.rs:248`, `app\gpu\renderer.rs:861` and
  `app\tools\brush.rs:299`: each gets `side_normal: None`.
- `Prefs { ... }` without a default tail in two tests: `app\prefs.rs:106-115` and `app\state.rs:805-814`: each gets
  `..Prefs::default()`.
- `BrushSettings { ... }` in its `Default` (`app\state.rs:169-171`).

### 4.3 Mapping from the settings to a stroke

At the press of a 3D stroke (today's `params`, `app\tools\brush.rs:59-71`, plus the tip):

| Settings | `BrushTip` |
|---|---|
| Circular, "Stay on connected surface" off (default) | Ball, `SameSide`, offset 0 |
| Circular, "Stay on connected surface" on | Ball, `Connected`, offset 0 |
| Sphere, Ball | Ball, `All`, offset |
| Sphere, Cube / Cylinder | Field, `All`, align, rotation, offset, scale 1.0, `through` |
| Sphere, File, `brush_shape.current` is `Some` (also while a newer load is running) | the same with the loaded field; a flat outline (5.7b) always has `through = true` |
| Sphere, File, no `current`, a load is `pending` | no stroke; info toast "The brush shape is still loading" |
| Sphere, File, no `current`, nothing pending (cannot be reached through the panel, see 4.4) | Ball, `All`, offset; the info toast "No brush shape is loaded; painting with the ball" once per program run (`warned_missing`) |

- `rotation = Quat::from_rotation_y(-turn) * Quat::from_rotation_x(tilt) * Quat::from_rotation_z(roll)` (radians).
- `up_hint` = the camera's up direction at the press (`OrbitCamera::basis`, `app\view\camera.rs:35-44`).
- `radius` as today. For a 2D stroke the tool calls `stroke_begin` as today; the tip is not used.
- The two user answers that can change a default are one line each: Q1 changes the default of `connected_only`;
  Q2 changes `scale` from `1.0` to `1.0 / (longest half side of the field's bounds)`.

**What a finished "Load..." changes.** The tab that issued the action decides the tool: the Eraser when the Eraser
was the active tool at that moment (`PendingShape::for_eraser`), otherwise the Brush. On success that tool gets
`kind = Sphere`, `shape = File`, and `through` = `true` when the report says the shape is flat or thin (5.7), `false`
otherwise. Its angles are not changed. The other tool is not changed. A quiet reload changes no setting.

### 4.4 Persistence and the life of the loaded shape

- Brush and eraser settings and the shape's path are written to `prefs.json`: about 300 bytes per tool plus one
  path. The file stays "a few KB" (`ARCHITECTURE.md:3921-3923`). This also ends the present state in which brush
  settings reset at every start (3.3).
- **When they are written.** `prefs.json` is written at exit (`app\app.rs:320-321`) and every time a project is
  opened or saved (`remember_recent`, `app\io_flows.rs:239-245`). One helper, `brush_flows::sync_prefs(&mut AppState)`,
  copies `tools.brush` and `tools.eraser` (through `sanitized()`) into `state.prefs`; both call sites call it
  directly before `prefs::save`. So every write of the file holds the brush settings of that moment, also after a
  crash later in the session. At start the two settings are copied from `Prefs` into `AppState::tools`.
- **`Prefs::brush_shape_path` is its own value.** Only two events change it: a successful "Load..." sets it, and
  "Remove file shape" clears it. A failed load, a failed reload, a missing file and whatever is loaded or loading at exit
  never change it. So a shape on a drive that is unplugged for one session is still remembered at the next start.
- **The shape is reloaded only when it is needed.** At start the app pushes `ReloadBrushShape` only if the path is
  set and the Brush or the Eraser has `kind == Sphere && shape == File`. Otherwise nothing is read at start; the
  Shape list then shows the remembered file name, and choosing that entry pushes `ReloadBrushShape`. The reload is a
  non-modal background job, like the font scan.
- **Request numbers.** Starting any load takes `next_request`, stores it in `pending` and counts up. A
  `JobOutput::BrushShape` is used only when its `request` equals `pending.request`; any other result is dropped
  without a toast. A "Load..." started while a quiet reload runs therefore wins whatever the order of arrival. The
  superseded reload cannot be stopped (only modal jobs can be cancelled, `app\jobs.rs:166-171`); it runs to its end
  and its result is dropped. `LoadBrushShape` is refused with the existing toast while a modal job runs
  (`app\io_flows.rs:460-467`).
- Nothing is cached on disk. No model data and no grid is written anywhere, in particular not under `%APPDATA%`.
- In self-test mode preferences are neither read nor written (`app\app.rs:141, 320`; `io_flows.rs:240`), so
  scenarios always start from the defaults.
- Limits of this choice, named for the user in Q4: the path breaks when the file is moved; the model is read and
  converted again the first time it is needed after each start (in the background; limit P10).

One table for every event (`current` / `pending` / `last_error` are the fields of `BrushShapeState`):

| Event | `current` | `pending` | `last_error` | `brush_shape_path` | Tool settings | Toast | Tab |
|---|---|---|---|---|---|---|---|
| "Load..." starts | kept | this load | kept | kept | none | none (progress window) | kept |
| "Load..." succeeds | the new shape | cleared | cleared | set to the path | issuing tool: Sphere, File, `through` as reported | info "Brush shape loaded: ..." and the warnings of 6.6 | Brush tab shown |
| "Load..." fails (import error, `NoInterior`, too many triangles) | kept | cleared | path and message | kept | none | error toast and message window | kept |
| "Load..." cancelled | kept | cleared | kept | kept | none | info "Loading cancelled" | kept |
| Reload starts | kept | this load | kept | kept | none | none | kept |
| Reload succeeds | the shape | cleared | cleared | kept | none | none | kept |
| Reload fails (file missing or not usable) | kept (`None` at start) | cleared | path and message | kept | every tool with `shape == File` and no `current` goes back to `Ball` | one warning toast "Brush shape not found: <path>. Using the ball." (or the bake message) | kept |
| "Remove file shape" | `None` | cleared; a running result is dropped by its number | cleared | cleared | every tool with `shape == File` goes back to `Ball` | none | kept |
| A result whose number is not the newest | kept | kept | kept | kept | none | none | kept |

Consequences: a stroke always has a usable tip while a new shape loads; the "no shape" toast cannot repeat at every
press, because a failed reload moves the tools back to the Ball; the "!" marker of 6.3 shows `last_error` until the
next successful load or Remove.

### 4.5 File formats and versioning

- `.wtep`: **no change.** `FORMAT_VERSION`, the manifest and the blob kinds stay as they are. Old projects load as
  before; new builds write files that old builds read. A stroke stores pixels only (3.2), so a project never needs
  the shape again.
- `prefs.json`: three new keys, each `#[serde(default)]`. `Prefs` has no version number; every field falls back to
  its default when missing (`app\state.rs:458-460`). An old file loads with default brush settings. A new file read
  by an older build loads too, because serde ignores unknown keys by default (not verified by a test in this round;
  test A10 adds one). So that a word written by a later version cannot make the whole file fall back to defaults
  (`prefs.rs:52-58` replaces an invalid file as a whole), the two keys `brush` and `eraser` are read leniently: each
  is first read as a JSON value and converted on its own, and a value that does not convert becomes the default
  brush.
- Values that convert but are out of range are forced into range: `prefs::from_json` passes both brushes through
  `BrushSettings::sanitized()` (4.2), next to the existing clamp of the window size (`prefs.rs:80-88`). A
  hand-edited `"size_px": 10000000` loads as 2048, a negative size as 1, a value that is not finite as the
  default. `sync_prefs` sanitizes on the way out too (4.4).

### 4.6 Undo and redo

Unchanged. A stroke with any tip goes through the same mask, re-apply and tile backup and ends as one entry "Brush
Stroke" or "Erase". Brush settings and the loaded shape are not undoable and do not mark the document as modified,
like every tool setting today. Changing a setting during a stroke does not affect that stroke, because the tip is
copied at `stroke_begin_with`, like the radius today.

One rule is new, for a stroke that is still being painted after the mouse button went up (5.10): Undo discards
that stroke, exactly as Undo discards a stroke that is still held today (`app\dispatch.rs:33-43`), and pushes
nothing; every other command first completes it, so its history entry exists before the command runs.

### 4.7 ARCHITECTURE.md sections this design changes

| Section | Change |
|---|---|
| 2.1, 2.2 | new files `brush_tip.rs`, `tip_field.rs`, `tip_bake.rs`, `topology.rs`, `brush_flows.rs`, `panels\brush_tab.rs` |
| 4.3 | tip units |
| 5.8, 6.2 | `query_sphere_dist` |
| 5.14 | `Document::mesh_topology`, built at first use |
| 5.19, 6.4 | tip, reach, `stroke_begin_with`, `stroke_tip_pose`, the shared query, field coverage, frames; the sentence "one spherical dab on the surface" is replaced by the exact description of 3.1; line 2078 gains the pointer queue and the time budget |
| new 5.26, new 6.10 | `tip_field.rs` / `tip_bake.rs` API; baking a model into a grid, the closed-solid rule, flat outlines, local thickness |
| 5.21 | `import_geometry_from_path` |
| 7 | one sentence: brush settings and shapes are not part of the project |
| 8.2 | new settings, `SideTab`, `BrushShapeState`, three actions, `Prefs` keys; the sentence that tool settings are written to `ui_state` and `Prefs` is corrected: only the brush and eraser settings are saved, in `Prefs`, at every write of the file |
| 8.3 | `LoadBrushShape` joins the interaction guard; Undo and Esc discard a stroke (`ToolController::discard`), every other cancel completes a brush stroke that is already released |
| 8.4 | `BrushRing::side_normal`, meaning of `brush_aux.yzw`. The shader rule at line 3196 does **not** change. |
| 8.6 | brush tool: tip mapping, the two queues with stored view transforms, the time rule, the repaint request through `end_frame`, finishing after the release, wheel, cursor; "at most 64 surface samples per frame" becomes "at most 64 raycasts per frame while the button is down" |
| 8.7 | bottom area with tabs; Brush tab; Brush options without the checkbox |
| 8.9 | `JobOutput::BrushShape` with its request number, the load flow, the quiet reload when the shape is needed, the event table of 4.4, `sync_prefs` before every write of `prefs.json` |
| 8.11, 9 | four scenarios, `Step::Hover`, "an input step ends when the tools are idle", and the tests of section 8 |

---

## 5. Algorithms and cost

### 5.1 Terms

- `c`: the anchor of a dab: the sample position, or the point on the straight line between two samples, as today.
- `n`: the stroke's reference normal at that dab (unflipped, as today). `n_v`: the viewer-facing normal at that dab
  (`Hit::normal`), blended between two samples exactly like `n` (`stroke.rs:215-216`).
- `r`: the stroke's radius in world units (fixed at the press).
- Tip space: the shape's own coordinates, moved and scaled so that the solid fits inside the unit ball around the
  origin (5.7 step 6 says how; for a file that is one closed solid this is: centre of the bounding box at the
  origin, farthest vertex at distance 1). +Y is "up". One tip unit is `k = r * scale` world units.
- `c' = c + offset * r * n_v`: the centre of the tip.
- `R`: world-from-tip rotation of the dab (5.3). A world point `P` is the tip point `q = R^T (P - c') / k`.
- `sd(P)`: signed distance from `P` to the tip's skin in world units, negative inside.
  Ball: `sd = |P - c'| - r`. Field, used as a solid: `sd = k * F(q)` with `F` the grid lookup of 5.6. Field, used
  "Through": `sd = k * G(q.x, q.z)` with `G` the two-dimensional lookup of 5.6.
- `thick(P)`: the local thickness at `P` in world units: the radius of the inscribed ball that holds `P` most
  deeply. Ball: `r` everywhere. Field: `k * T(q)` with `T` the second grid of 5.7 step 9 (Through: the
  two-dimensional `T2`). It is never smaller than the depth of `P`.
- `depth_max`: the largest depth inside the tip in world units. Ball: `r`. Field: `k * inradius`. `thick` never
  exceeds it.
- `R_b`: radius of a ball around `c'` that holds the whole tip. Ball: `r`. Field: `k * bound`, where `bound` is the
  largest distance of an inside node from the origin plus `sqrt(3) * cell` (at most 1.0284: one cell is at most 0.0164
  tip units, 5.7 steps 3 and 6).
- `aa`: world size of one texel of the triangle being rasterized (`tri_texel_world`, `brush.rs:216-223`).

### 5.2 One coverage formula for every tip

```
cover = falloff(thick + sd, thick, hardness, aa)          // the existing function, brush.rs:84-109
```

Written out, `falloff` computes

```
ramp    = clamp(0.5 - sd / aa, 0, 1)                    // anti-aliasing, one texel wide, centred on the skin
t       = clamp(1 + sd / thick, 0, 1)                   // 0 in the middle of the local part, 1 on the skin
profile = 1                                             if hardness >= 1 or t <= hardness
        = 1 - smoothstep(0, 1, (t - hardness) / (1 - hardness))   otherwise
cover   = profile * ramp
```

For the ball `thick + sd = |P - c'|` and `thick = r`, so this is today's call `falloff(dist, r, hardness, aa)`
exactly. The ball path keeps computing the distance as it does now (`brush.rs:328-332`), so its results stay
bit-identical.

**Why the local thickness and not the largest depth of the whole shape.** If `t` were measured against
`depth_max`, every part thinner than the thickest part would be faded as a whole at the default hardness of 80 %:
a fin whose half thickness is 10 % of the inradius would never exceed 50 % coverage (`t` = 0.90,
`1 - smoothstep(0.5)` = 0.50), one at 5 % never 16 %. With the local thickness every part of the shape, thick or
thin, is at full strength along its own middle and fades towards its own skin over the share `1 - hardness` of its
own half thickness. At hardness 80 % the inner 80 % of every part is solid. For a shape that is one ball `T` is the
inradius everywhere, so nothing changes there (test C12). Test C23 pins the thin-part case.

`T` is looked up only where it can matter: when `hardness < 1` and `-sd < (1 - hardness) * depth_max`. Everywhere
else the profile is 1 (because `thick <= depth_max`), and the lookup is skipped.

Opacity and flow are untouched: coverage goes into the same per-dab maximum buffer and the same 16-bit mask
(`brush.rs:516-522, 670-714`).

Gutter texels are evaluated at points that continue the triangle's plane beyond its edge, as today (map, not
re-read). `cover` is a continuous function of position for every tip (`F`, `G`, `T` and `T2` are blended lookups,
5.6), so a gutter texel gets the paint of the island edge beside it within one texel's change.

### 5.3 The tip's frame (rotation `R`)

`R = A * U`. `U` is the user's rotation (4.3). `A` aligns the tip:

- **Fixed in space**: `A` = identity.
- **Follow surface**: `A = tip_frame(n_v, up_hint, prev_up)`:

```
out   = n_v normalized (+Y when it has no direction)
seed  = prev_up if there is one, else up_hint
up_t  = seed minus its part along out; when that is shorter than 0.01: the same with up_hint; when that is still
        too short: any unit vector at right angles to out (Vec3::any_orthonormal_vector)
up_t  = up_t normalized
right = up_t x out
A     = columns (right, out, -up_t)      // tip X -> right on screen, tip Y -> out of the surface, tip Z -> down on screen
return (A, up_t)
```

`right`, `up_t`, `out` is the frame the stamp projector builds from a surface point: Z = normal, Y = up hint
projected, X = Y x Z (`core\stamp_bake.rs:70-84`), fed with the same two inputs the Stamp tool uses: the
viewer-facing normal and the camera's up (`app\tools\stamp_place.rs:177-178`). So a brush shape and a stamp at the
same spot are upright in the same way, also on a model with reversed winding, where the unflipped normal `n` points
into the model. `n` is used for the side filter only.

Along a stroke: the first dab of a segment has no `prev_up` and starts from `up_hint`. Every later dab passes the
`up_t` of the dab before it. The tangent is therefore carried along the surface and never jumps, also over the top
of a head where a fixed "up" rule has no answer. A new segment (after the pointer left the surface, or when two
samples are not joined) starts from `up_hint` again.

`tip_frame` is public, and the app calls it for the hover cursor with the hover hit's `normal` and the camera's
current up, so the cursor shows the frame the first dab will have. During a stroke the cursor reads
`stroke_tip_pose()`.

All of this is arithmetic on the sample sequence: the same samples give the same pixels (`ARCHITECTURE.md:415-421`).
A ball ignores `R`.

### 5.4 One dab, restructured: one query for all sets

Today: N sets, at least N sphere queries per dab (3.1). New `surface_dab`, for every tip:

1. For each set `s` the stroke has not skipped: `aa_s` = the set's mean texel size, `rho_s = R_b + 0.5 * aa_s`.
   `R1` = the largest `rho_s`. One `bvh.query_sphere_dist(mesh, c', R1, &mut q)`. `q` holds
   `(triangle, squared distance)` sorted by triangle id.
2. Per set, exactly today's first pass: `L1_s` = the triangles of `q` that belong to `s`, have
   `d2 <= rho_s * rho_s` and pass the side test (`SameSide` and `Connected`: `geo_normal . n >= -0.2`; `All`: none).
   `local_max_s` = the coarsest `tri_texel_world` among `L1_s`, at least `aa_s` (`brush.rs:368-372`).
   **A set is widened only when `local_max_s > aa_s * 1.001`** (`brush.rs:373`); then
   `rho_s = R_b + 0.5 * local_max_s`. A set that is not widened keeps its first radius.
3. If any widened `rho_s` exceeds `R1`: one second query with `R2` = the largest `rho_s`, replacing `q`.
4. The candidates of set `s` are the triangles of `q` that belong to `s`, have `d2 <= rho_s * rho_s` and pass the
   side test, in ascending triangle id. The radius is computed with the same `f32` expressions as today
   (`radius + 0.5 * x`, then squared, `brush.rs:365, 374`, `bvh.rs:444`), and `d2` is the value `query_sphere` itself
   compares (`bvh.rs:467`).
5. `Connected` only: the flood of 5.5 removes the candidates it does not reach.
6. "The stroke reaches set `s`": for a ball, its candidate list is not empty. For `SameSide` and `All` this equals
   today's `dab_reaches_set` (first radius, `brush.rs:248-249`): if the first-pass list is empty the set is not
   widened, and if it is not empty the final list contains it. **For a field tip, at least one candidate must
   survive the early reject of 5.6** (the test against the shape's own box). The first reach decides painted or
   skipped as today (`stroke_target`, `stroke.rs:377-392`) and creates the set's stroke buffer. A set that only the
   bounding ball of a shape touches is therefore not reached: it gets no buffer, and a locked or hidden layer in it
   does not produce the warning "Paint skipped on ..." (`app\tools\brush.rs:204-211`) for paint that would never
   have landed there (test C24). A cube fills only 37 % of its ball, so without this rule the false warning would
   be common next to set borders. What remains: a set whose triangles are inside the shape's box but outside the
   shape is still counted as reached.
7. Per painted set with candidates: today's per-triangle setup with `R_b` and `c'` in the place of `radius` and
   `center` (`brush.rs:380-436`), rasterization, commit and re-apply. A field tip changes the per-triangle steps as
   described in 5.6.

At most 2 tree walks per dab instead of N to 2N, and no allocation per unreached set.

**Identity with today.** For a ball with `SameSide` or `All` and offset 0, every set gets today's candidate list in
today's order, so masks are bit-identical. This holds as long as the BVH's own pruning is conservative, that is, as
long as no node box is farther from the centre than a triangle inside it. The build pads the boxes for that
(`bvh.rs:328-329`). If rounding ever broke it, today's narrower query would have missed a triangle it should have
found, and the new, wider query would find it. Test C2 compares the new path with the unchanged `dab_3d` on fixed
and random input and must report equal masks.

### 5.5 `Connected`: the piece of surface under the cursor

**Table, once per document, at the first connected stroke** (`MeshTopology::build`):

- `weld[v]`: vertices with identical position bits (after adding 0.0, the rule of `bits3`,
  `core\mesh_build.rs:107-109`) get the same number. Sort the vertex indices by position bits with a parallel sort,
  number the runs.
- `vert_tris`: for every welded vertex the triangles that use it, ascending, as one array with a start index per
  welded vertex (two counting passes over the index buffer).

No hash map is used, so the result does not depend on thread timing.

Cost for 1,498,080 triangles (estimate): 4 bytes per vertex + 4 bytes per welded vertex + 12 bytes per triangle:
26 MiB when the model has 1 million vertices, at most 54 MiB when no vertex is shared; 0.1 to 0.3 s on 12 threads,
once, in the frame of the first connected press. Limit P9 checks it; if it fails, the fallback is to build the table
in a background job when the checkbox is ticked (one more `JobOutput` variant, not built first).

**Per dab** (after step 4 of 5.4, with `Rq` = the radius of the last query made for this dab):

```
marked   = triangles of q (any set, with or without UVs) with geo_normal . n >= -0.2
seeds    = (a) the triangle of the ray hit of the sample the dab sits on, or, for a dab between two samples, the
               hit triangles of both end samples, where they are marked           (Hit::tri, bvh.rs:22-23)
           (b) the marked triangles that the previous dab of the same segment reached
           (c) for a dab at a sample only: marked triangles whose closest distance is at most 0.001 * r
           (d) only when (a) to (c) give nothing: with d0 = the smallest closest distance among marked
               (free: q carries the squared distances), the marked triangles within d0 + 0.001 * r
flood    : from the seeds. From a reached triangle T, for each of its three welded vertices W, for each marked,
           not yet reached triangle U in triangles_at(W): reach U when touches_inside(T, U, W).
touches_inside(T, U, W), with p = the position of W:
           true when |p - c'| <= Rq
           true when an edge of T leaving W towards a and an edge of U leaving W towards b point the same way
                (da = a - p, db = b - p, da . db > 0, |da x db|^2 <= 1e-8 * |da|^2 * |db|^2)
                and the part they share, the segment from p to p + da * min(1, |db| / |da|), comes within Rq of c'
           false otherwise
keep     : only reached triangles stay candidates, for every set.
```

- The seeds are "the surface the cursor was on". For a dab at a sample that is the hit triangle (a), and (c) also
  seeds a triangle that touches the centre across a crack. A dab between two samples lies on the straight line
  between them, which on a convex part runs inside the part; another part that touches or cuts it there can be
  nearer to that point than the part itself (on `two_material_model`, a stroke along the head 0.001 above the
  body: the chord point is 0.0017 inside the head but 0.001 above the body's top face). Such a dab is therefore
  never seeded by distance: it starts from the triangles the two samples actually hit and from what the dab
  before it reached, so the flood stays on the surface the stroke is on (test C25). The samples are a quarter
  of the ring radius apart on screen (`app\tools\brush.rs:160`), so their hit triangles are normally inside the
  dab's ball. Rule (d) is the fallback for the rare dab that holds none of them (samples up to 6 radii apart are
  still joined, `stroke.rs:33, 192`).
- The session stores the hit triangle with each surface sample, and "reached by the previous dab" is read from the
  stamp array of the flood (below) with the previous dab's number; no extra memory.
- Two triangles are neighbours when they share a welded vertex. The flood crosses only where the two triangles
  touch each other inside the ball: at the shared corner, or along the part of an edge they share. A full shared
  edge is the case where both edges have the same length. A long edge that meets a shorter one (a T-junction)
  shares its start with it and is crossed along the shorter edge. Two parts that are joined only outside the ball
  stay separate.
- "Marked" and "reached" are two arrays of `u32` per triangle, stamped with the dab number so they are never
  cleared, allocated at `stroke_begin_with` for a connected stroke: 8 bytes per triangle (12 MiB at 1.5 million
  triangles).
- The result is a set, so it does not depend on the order of the queue.

Cost per dab (estimate): about 18 neighbour checks per reached triangle; 50 to 100 microseconds for 2,000 candidate
triangles, 2 to 5 ms for 100,000.

Stated limits:

- `Connected` equals `SameSide` only where every same-side triangle in the ball is linked to the cursor point
  through same-side triangles inside the ball. A wall turned more than 101.5 degrees away breaks the link (last row
  of the table in 1.7). Test C6 pins both sides of this statement.
- At a concave fold between 78.5 and 90 degrees, when the dab centre is between 1.00 and 1.02 radii away from the
  crease, the other flank lies inside the ball but the crease does not. `SameSide` then paints a faint rim on the
  other flank and `Connected` does not. On convex shapes (a sphere, a cube corner) the two are equal.
- A crack whose two sides share no vertex position at all (parts that merely touch) is a border. A stroke along such
  a crack paints only the side it is on.
- On models made of many loose pieces (hair cards, a wall of separate bricks) a connected Circular brush paints one
  piece per dab. The checkbox is off by default.

### 5.6 A field tip inside the dab

The field is a grid of signed distances in tip units at the nodes `origin + (i, j, k) * cell`. Lookup `F(q)`:

```
g  = clamp((q - origin) / cell, 0, dims - 1) per axis
F  = trilinear blend of the 8 values around g  (i16 * cell / 256)
     + the distance from q to the clamped point                    // 0 inside the grid box
```

The grid has at least 2 cells of margin outside the shape on every side, so every value on the border of the grid is
positive and the two parts meet without a jump. (The one exception is a closed part that the second pass of 5.7
step 6 cuts off at the border of its grid: there the border value is negative, the lookup still has no jump, and
the cut part ends within its own depth outside the grid. `bound` is computed from the inside nodes, so it holds
there too.) Property the fast paths rely on, guaranteed by 5.7 and tested (F9):
neighbouring nodes differ by at most one cell, so `F` changes by at most `sqrt(3)` per unit of tip distance.

Changes to today's per-triangle steps for a field tip (the ball keeps today's code):

- **Query radius, plane test and clip box**: today's code with `R_b` and `c'` (5.4 step 7). `R_b` already contains
  one cell diagonal of margin, so the blended zero level can never lie outside the ball the clip box is derived
  from, and no paint is cut off (test C19).
- **Early reject**: a triangle whose three corners, taken to tip space, all lie beyond the same face of the field's
  `bounds()` box grown by `0.5 * aa / k` is dropped. For a rod-like shape this removes most of the ball's candidates.
  The oracle test C9 proves that no painted texel is lost. The same test decides whether a set is reached (5.4
  step 6). For Through the box is the outline's box in x and z and `-bound..bound` in y.
- **Tiny brushes**: when `R_b < max(aa_s, tri_aa)` (today's sub-texel rule, `brush.rs:414-416`), the triangle is
  rasterized with the ball of radius `R_b` around `c'` at full hardness. A shape under two texels across cannot show
  its form, and the texel under the cursor stays covered. Otherwise, when `depth_max < tri_aa`, hardness is forced
  to 1.
- **Coverage per fragment**: `q = R^T (P - c') / k` (9 multiplications), `F(q)` (8 loads, 7 blends), then 5.2. In
  the fade zone only (`hardness < 1` and `-sd < (1 - hardness) * depth_max`) one more lookup, `T(q)`, of the same
  cost.
- **Through** (`tip.through`, and always for a flat outline): the shape is the prism of its middle slice along tip
  Y, cut by the ring's ball:

  ```
  sd_o   = k * G(q.x, q.z)                                 // two-dimensional lookup: 4 loads, 3 blends
  cover  = falloff(k * T2 + sd_o, k * T2, hardness, aa)    // 5.2 with the outline's distance and thickness
           * clamp(0.5 - (|P - c'| - k) / aa, 0, 1)        // hard limit at the ring's ball
  ```

  `q.y` is not used, so the paint does not depend on how far the surface bends away from the tangent plane: with
  Follow surface the outline is pressed along the surface normal of each dab, and Tilt and Roll lean the pressing
  direction. Like every Sphere tip it paints all surfaces inside the ball, far sides included. The clip box and the
  query use `R_b` as for Solid (the ball of radius `k` lies inside it). `G` and `T2` have the neighbour property of
  `F`, and the distance to the ball's centre changes by at most the distance moved, so the fast paths below hold
  with the outline's distance and one more condition: the region lies inside the ball, `|g - c'| + rho <= k - aa / 2`.
- **The two fast paths** (whole triangle at full strength; 64 x 64 blocks at full strength) need "every point of
  this region has coverage exactly 1". Coverage is 1 where `-sd >= d1`, with
  `d1 = max((1 - hardness) * depth_max, aa / 2)` (`aa / 2` alone for hardness 100 %). Because `F` changes by at most
  `sqrt(3)` per unit, a region inside a ball of radius `rho` around a point `g` is at full strength when

  ```
  -k * F(q_g) - sqrt(3) * rho  >=  d1 * 1.001
  ```

  Triangle: `g` = centroid, `rho` = largest corner distance from `g`, widened by the barycentric slack as
  `in_full_core` does (`brush.rs:318-323`). Block: `g` = the mean of the four block corners mapped to the triangle's
  plane, `rho` = the largest distance from `g` to those four points (the mapping is `BlockFold`'s,
  `brush.rs:285-286`). One grid lookup per triangle or block. Folded and unfolded rasterization must give the same
  mask (test C16, on 20 random poses and on a baked shape).

### 5.7 Baking a model into a field (`TipField::bake`, on a job thread)

The solid is "everything that cannot be reached from outside without crossing the model's surface". A file can
hold more than that solid: a floor plane, hair cards, an open body next to closed buttons. The importer keeps every
non-degenerate triangle and never checks closedness (`core\import\mod.rs:218-271`). So the size of the brush and the
resolution of the grid are taken **from the solid that was found, not from all vertices of the file**.

1. **Import** with `import_geometry_from_path`. More than 2,000,000 triangles: `TooManyTriangles`. Build one BVH
   over all triangles (`Bvh::build`).
2. **Flat check.** `V` = the bounding box of all vertices. Non-finite or empty: `NoInterior`. If the thinnest side
   of `V` is under 0.5 % of its longest side (0.6 cells), the model is a flat sheet in one of the file's main
   planes: continue with 5.7b (a flat outline). If it is under 2 % (2.4 cells), the solid bake is tried first, and
   when step 4 finds no inside, 5.7b is used instead of the refusal. So a coin 150 mm wide and 1 mm thick is never
   refused for being thin. Otherwise:
3. **Grid over a box `B`** (first pass: `B = V`): cubic cells of `cell = (longest side of B) / 122`. Nodes per
   axis: `clamp(ceil(side / cell + 5.382), 8, 128)`; the first node is `2.382` cells below the box. So the longest
   axis has 128 nodes and every axis has at least 2 cells of margin on both sides. Memory: at most 128^3 nodes. The
   odd phase 0.382 keeps faces of typical models (at halves, thirds and quarters of the box) off the planes of
   nodes; step 4 is also safe when a face does lie on one.
4. **Links, outside, inside.** For each node and each of its three neighbours in +X, +Y, +Z: is the straight link
   between the two nodes blocked by a triangle? One `Bvh::occluded` call from `a - tau * d` over the length
   `cell + 2 * tau`, with `tau = cell * 1e-4`. The extension at both ends makes a triangle that lies exactly on a
   node block both links that end at this node, whatever the rounding (the BVH accepts a closed interval,
   `bvh.rs:376`). About 6.3 million short rays for a full grid, in parallel slabs on the rayon pool, stored as 3
   bits per node. Progress is reported per slab; the job can be cancelled between slabs.
   **Outside** = flood fill from the seed nodes through links that are not blocked (6 neighbours, one pass). In the
   first pass every node of the outermost layer is a seed. **Inside** = every node the flood did not reach. This
   is exact for a watertight model at grid resolution. It does not use the winding, and duplicated, non-manifold or
   inner geometry cannot break it, because only "is there a wall between these two points" is asked.
   Fewer inside nodes than `max(8, nodes / 10,000)`: `NoInterior`. A tilted flat sheet, a cup, a model with a hole
   wider than one cell, and a closed model that is thinner than about one cell everywhere (0.82 % of its longest
   side) end here. The message names all of these cases (6.6), so it is true for each of them.
5. **Skin points.** A link between an inside node and an outside node ("border link") is blocked by construction.
   For each border link one `Bvh::raycast` **from the outside node towards the inside node** (same extended
   interval) gives the first triangle the link meets coming from outside, and the hit point. A triangle buried in
   the solid (an inner wall, a face inside an overlapping part) is never the first one from outside, even where it
   meets the skin within one cell. The triangles are the skin set, the points are the skin points; both are
   collected per slab and merged sorted, so they are the same on any number of threads.
6. **Was the whole file the solid?** `H` = the bounding box of the skin points.
   - **Yes**, when `H` reaches within 2 cells of `V` on all six sides. Tip units are exact, as before: `p0` =
     centre of `V`, `s0` = the largest vertex distance from `p0`. Continue with step 7 on this grid.
   - **No**: part of the file is open, or closed but too thin for the first grid, and is not part of the solid.
     `report.partial = true`. The grid is laid out again over `B = H` grown by 3 cells of the first pass on every
     side, and steps 3 to 5 run a second time on it, with the same BVH (the unused triangles stay in it as walls).
     Seeds of the second pass: a node of its outermost layer is a seed when it lies outside the first grid, or
     when the straight segment from it to at least one of the 8 corners of the first-pass cell around it that
     were classified outside is not blocked (`Bvh::occluded`). A node inside a closed part can reach only inside
     corners, so it is never a seed, and a closed part that crosses the border of the second grid cannot flood the
     solid; it ends flat at the border. Then `p0` = centre of the box of the second pass's skin points and `s0` =
     the largest distance of a skin point from `p0` plus one cell, so the solid lies inside the unit ball and fills
     it to within one cell (0.8 % of Size).
   Tip coordinates are `(v - p0) / s0`; `cell` and all distances are expressed in them from here on.
7. **Border nodes** (a node with a neighbour of the other class): `f` = the smallest exact distance
   (`closest_point_on_triangle`) to the triangles that its own **border links** hit in step 5, and to the skin
   triangles within one cell (`query_sphere(node, cell)` filtered by the skin set). Links to a neighbour of the
   same class are not used, whatever blocks them. No search is wider than one cell. `f` is at most one cell, and
   for two neighbouring nodes on opposite sides the two values add up to at most one cell, because the triangle
   that blocks their link lies between them.
8. **All nodes**: `d(x) = sqrt(min over border nodes b of (|x - x_b|^2 + f_b^2))`, computed with the exact separable
   squared-distance transform (three passes of one-dimensional lower envelopes; every grid line is independent, so
   the passes run in parallel and the result does not depend on thread timing). The value is `-d` for inside nodes
   and `+d` for the others, stored as `i16 = round(d / cell * 256)`, **saturating at +-127.99 cells**. Saturation
   is needed: a node in the far corner of a grid whose solid fills another corner can be up to 221 cells away
   (128 * sqrt(3)). Two neighbouring saturated values are equal, so the neighbour property below survives.
9. **Local thickness** (the grid `T` of 5.2): for every inside node `x`, `T(x) = depth(y*)`, where `y*` is the
   inside node that minimises `|x - y|^2 - depth(y)^2` and `depth = -d`. That is the same separable lower-envelope
   transform as step 8, started from the inside nodes with the weights `-depth^2` and carrying the index of the
   winning node. `T(x) >= depth(x)`, because the node itself is a candidate. Outside nodes next to an inside node
   copy the largest `T` of their inside neighbours; all other outside nodes hold the inradius. Stored as one byte
   per node in half cells (depth is at most 64 cells). In a ball-like solid every node is won by the centre, so `T`
   is the inradius everywhere.
10. **Middle slice** (for "Through", 5.6): the sign of `F` on the plane y = 0, sampled on a square lattice with the
    grid's cell over the grid's x and z extent; the crossing point on every lattice link between two signs by
    linear interpolation; then the two-dimensional forms of steps 7 to 9 (distance to the crossings, the transform
    in two passes, local thickness). Result: `G` (i16) and `T2` (u8) on at most 128 x 128 nodes.
11. **Lying flat.** If the thinnest side of the solid's box is under 50 % of both other sides, that axis becomes the
    tip's +Y by a quarter turn (thin Z: `(x, y, z) -> (x, z, -y)`; thin X: `(x, y, z) -> (-y, x, z)`), applied as an
    exact re-indexing of the finished grids before step 10. A plate, a coin or an extruded star then lies flat on
    the surface with all angles at 0, whatever plane it was modelled in. `report.up_axis` records the file axis.
12. **Summary**: `inradius` = the largest `d` of an inside node; `bound`; `bounds`; the three principal sections
    (marching squares on the planes x = 0, y = 0, z = 0); `hash` (over all grids);
    `report.height_ratio` = (extent of the solid along tip Y) / (its largest extent along tip X or Z);
    `report.thin` when `inradius` is under 2 cells.

**5.7b A flat outline.** A sheet that step 2 sends here has no inside the grid can hold, but
seen along its thin axis it has an outline, and that is all "Through" needs. The thin axis becomes tip +Y (the
quarter turns of step 11). The triangles are projected onto the other two axes. On a lattice of 256 nodes on the
longer side (2 cells of margin) a node is inside when it lies in at least one projected triangle (edges included;
a bucket grid over the triangles keeps this at a few triangles per node). No covered node: `NoInterior`. The
crossing point on each link between a covered and an uncovered node is found by 10 bisection steps of the same
test. A border node's distance is its distance to the crossing; when links in both directions cross, at distances
`a` and `b`, it is `a * b / sqrt(a^2 + b^2)` (the distance to the straight line through both crossings). Then the
two-dimensional transform and local thickness as in step 10. The result is a `TipField` with `report.flat = true`,
no three-dimensional grid, `G` and `T2` only. It can only be used "Through"; `sample` is defined as
`G(q.x, q.z)`. Tip units: `p0` = centre of the box of the crossings, `s0` = the largest distance of a crossing from `p0` plus
one cell. A flat
sheet that is tilted against the file's main planes is not recognized and ends as `NoInterior`; the message says
how to fix it.

Why the distance is measured to the border of the inside region and never to every triangle of the file: buried
triangles and unused open parts then leave no trace in the paint (tests F5, F6, F16), and sign and distance come
from the same classification, so the field has no jump. Next to the skin the distance is exact, so a flat face of
the shape gives a straight, correctly placed edge at any angle. Deeper inside, and where a buried or unused
triangle meets the skin, the value can differ from the true distance by up to about one cell; only the hardness
fade reads it there.

Why neighbouring nodes differ by at most one cell (the property 5.6 needs): two nodes of the same class are two
points measured against the same set of (border node, value) pairs, which is a distance in four dimensions; two
neighbouring nodes of opposite class are both border nodes, and their values add up to at most one cell (step 7;
step 8 can only lower them). Saturation in step 8 only makes differences smaller.

What the grid cannot show: details thinner than about 1.5 cells (1.2 % of Size) fade out, and sharp edges and
corners are rounded by about one cell (0.8 % of Size: 4 texels at Size 512, 17 texels at Size 2048).

Worked example for step 6: a closed cube with a side of 1 standing on an open 10 x 10 floor quad. First pass:
`V` is 10 x 1 x 10, the cell is 10 / 122 = 0.082, the cube is 12 cells wide and has about 1,700 inside nodes, so
it is found. `H` is the cube's box, far from `V` on four sides: second pass over a box of 1.49, cell 0.0122, the
cube is 82 cells wide. Tip units come from the cube, so its side is 58 % of Size, as for the cube alone. Without
step 6 the floor would set the scale and the cube's side would be 7 % of Size at 12 cells. The warning of 6.6 tells
the user that the floor was left out.

Cost (estimates, 12 threads): step 4 is the bulk, 0.2 to 1.0 s for a full grid; a second pass doubles it for
files with unused parts only; steps 5 to 12 together under 0.5 s. Import of a 500,000-triangle OBJ is on top of
that. After the bake the mesh and its BVH are dropped; at most 6.1 MiB stays in memory (4 MiB distances, 2 MiB
local thickness, under 0.1 MiB for the slice).

### 5.8 Built-in Cube and Cylinder

`TipField::cube()` and `cylinder()` are written straight from the exact distance formulas of a box and of a capped
cylinder at the nodes of a grid laid out as in step 3 of 5.7: no mesh, no job. Exact distance formulas change by at
most the distance moved, so the property of 5.6 holds. Their thickness grids and middle slices come from the same
code as the bake's (5.7 steps 9 and 10). They are made at first use (estimate: under 50 ms each) and
kept in `AppState::builtin_tips`. They also serve as fixtures: the paint-engine tests use them and `from_fn`, so the
paint-engine package does not wait for the bake package.

### 5.9 The stroke session

- The session holds `tip: BrushTip`, the query buffer, one candidate list per set (no allocation per dab), the flood
  marks for a connected stroke, the carried tangent `prev_up`, and the pose of the last dab. A surface sample also
  stores the viewer-facing normal and the triangle the ray hit (`Hit::tri`), for the seeds of 5.5.
- `stroke_to_3d(&Hit, pressure)` keeps its signature; it now also reads `hit.normal`.
- The joining rule (6 radii, normals not opposing) and the spacing step are unchanged (`stroke.rs:192, 207`); they
  use `r`.
- Thin shapes and spacing: spacing is a share of Size, so a thin blade held across the stroke direction leaves gaps
  at 12 %, as a thin tip does in any paint program. The hover text of Spacing says so.
- `texture_dab` (2D) is not touched.

### 5.10 Keeping the window responsive: stroke pacing in the app

The core cannot time-slice (`ARCHITECTURE.md:2078`) and a dab of a large shape can cost tens of milliseconds (5.12).
Facts about today's code that this section has to live with:

- Stroke work happens only inside frames, and nothing in the tool or view code asks for a frame while a stroke is
  open: `request_repaint` is called only by the jobs (`app\jobs.rs:59, 125`), the thumbnail code
  (`app\panels\mod.rs:141, 150, 163, 172`; 200 ms, `mod.rs:35`), the toasts (`app\toasts.rs:140`), the Transform
  wheel gesture and deferred preview bakes (`app\view\mod.rs:48, 52`), the Preview view (`view\preview.rs:95`) and
  the self-test runner (`selftest\mod.rs:255`). The value `ToolController::end_frame` returns is already turned
  into `request_repaint_after` (`view\mod.rs:46-49`).
- `Tool::finish` gets no view transform (`app\tools\mod.rs:198`), and the wheel can zoom the view during a drag
  (`app\view\mod.rs:420-430` has no busy check for the zoom), so a screen position that is turned into a surface
  point later could meet another camera.
- `ToolController::release` always returns to Idle (`app\tools\mod.rs:409-415`), and every tool change, view-mode
  change, guarded command and Esc cancels a running stroke (`app\dispatch.rs:33-43, 125-129, 632, 761-763`;
  `app\io_flows.rs:472-474`; `app\view\mod.rs:75-76`).
- On release today only the rest of one segment is walked (`app\tools\brush.rs:247-257`), because pointer positions
  between frames were skipped (the defect of 3.3). A queue that keeps every position has more work left at the
  release than today's code.

The total work of a stroke is fixed by its path and the Spacing; pacing can only decide when it is done. The
design therefore makes three promises and says what gives way when the machine is too slow:

1. the painted pixels depend only on the pointer path, never on the clock or on a later camera move;
2. no frame spends more than 100 ms plus one sample on stroke work, also not the frame of the release;
3. paint trails the pointer by about 250 ms of work at most, as long as the stroke's work arrives slower than
   real time. When it arrives faster, the trail grows and the stroke finishes after the release, visibly.

**Two queues in `BrushTool`.**

- `unresolved: VecDeque<(egui::Pos2, ViewXform)>`: every pointer position of the drag in order, **each with the
  view transform of the frame it arrived in** (`ViewXform` is `Copy`, `app\tools\mod.rs:51-52`). A position within
  0.001 points of the last queued one is dropped.
- `pending: VecDeque<QueuedSample>`, `QueuedSample` = a ray `Hit` or a miss.

**Resolve** (cheap: raycasts only). The screen polyline through the unresolved positions is walked in today's
steps of `max(1 point, a quarter of the ring radius on screen)` (`app\tools\brush.rs:160`). Each step is raycast
with the transform stored with the position it leads to, and the hit or miss is appended to `pending`. At most 64
raycasts per frame while the button is down, as today (`brush.rs:13`); at most 4,096 per frame after the release
(today's `FINISH_SAMPLES`, `brush.rs:15`). What is left stays in `unresolved` with its own transform. So a zoom or
any other camera change in a later frame cannot move a position that is already queued, and the walked path no
longer depends on where a budget ran out.

**Paint** (the expensive part: the dabs). Samples are taken from `pending` in order and given to
`stroke_to_3d` / `stroke_break`. Per frame:

```
mean    = average wall time of the last 16 painted samples of this stroke (0 before the first)
backlog = pending.len() * mean
paint samples until pending is empty, or
      10 ms are spent in this frame and backlog <= 250 ms, or
      100 ms are spent in this frame
always at least one sample per frame
```

So a light stroke is done inside its frames as today. A heavy stroke first costs 10 ms per frame; once a quarter
second of work is waiting, frames grow up to 100 ms (10 frames per second) to hold the trail there; beyond that the
trail grows. The numbers are fields of the tool, `StrokePace { raycasts_per_frame: 64, soft: 10 ms, lag_target:
250 ms, frame_cap: 100 ms }`, and the clock is a replaceable function, so unit tests run with a clock that adds a
fixed time per sample (A5).

**Frames.** `drag` appends the position and then resolves and paints. `ToolController::end_frame`, which the
central area calls once per frame (`app\view\mod.rs:46-49`), resolves and paints when `drag` did not run in this
frame, and returns `Some(0.0)` as long as one of the queues is not empty; the existing code turns that into a
repaint request. So an open stroke with waiting work always gets its next frame, also with a still pointer, with
the pointer outside the window and with the Layers list hidden (A17). Nothing depends on the 200 ms thumbnail
timer.

**Release.** `release` appends the release position with its transform and marks the stroke as released; it does
not walk the rest in one go. Resolve and paint continue under the same rule in this frame and the next ones.
When both queues are empty the tool calls `stroke_end` (one history entry) and shows the skipped-sets warning. If
that happens in the frame of the release, everything is as today. Until then the stroke is **finishing**:

- The controller stays busy: `ToolController::release` keeps the interaction while `brush.finishing()` is true,
  and `end_frame` returns it to Idle when the tool has ended the stroke. `BrushTool::release` and `drag` do nothing
  for a stroke that is already released (the view calls `release` again in every frame in which the button is
  up, `app\view\mod.rs:413-417`).
- The ring stays at the point where paint is being applied, and the status bar shows "Finishing the stroke...
  (Esc discards it)". The same hint, "Painting... the stroke is catching up", is shown during a drag while the
  backlog is above 250 ms. The trail between the ring and the pointer is what the user sees of the lag.
- **A finishing stroke is the user's completed stroke, so nothing throws it away by accident.**
  `ToolController::cancel` completes a released brush stroke (it resolves and paints everything that waits,
  without a time limit, then `stroke_end`) and cancels everything else as today. So choosing another tool, changing
  the view mode, a dialog that opens, a new document and every guarded command first complete the stroke. Only two
  things discard it, through the new `ToolController::discard`: Undo (the guard at `dispatch.rs:39-43` already
  treats the interrupted stroke as the thing that was undone) and Esc (`dispatch.rs:757-763`).
- A press in a view while a stroke is finishing completes that stroke at once and then starts the new
  interaction, so a quick second click is not lost (`app\view\mod.rs:383` and `tools\mod.rs:352` return early for a
  busy controller today).
- `finish` (Save, Save As, Export, `dispatch.rs:98-100`) does the same completion for a held or a finishing stroke.
  It needs no transform: the samples in `pending` are hits, and the positions in `unresolved` carry their own. The
  saved file therefore holds the whole stroke up to the last pointer position the app has seen.
- `cancel` of a held stroke and `discard` drop both queues and call `stroke_cancel`.

A completion without a time limit can take as long as the backlog. Under promise 3 that is about a quarter of a
second; after a drag that outran the machine it can be seconds, and it happens only on an explicit action of the
user.

**What the user gets on the 1.5-million-triangle model** (figures of 3.8; one stroke of the round-3 audit did
about 1,540 ms of stroke work in 38 frames, 40.5 ms per frame): if the single query of 5.4 saves nothing, about
40 ms of work arrive per frame; the trail reaches 250 ms after about 8 frames, then the frames stretch to carry
what arrives, which is today's frame rate there (13 to 20 frames per second), with the paint a quarter second
behind the pointer, and the stroke is complete about 0.3 s after the release (three frames of up to 100 ms). Today
the same stroke runs at the same frame rate with the paint under the pointer, but with single frames of 245 to
594 ms and with a path that cuts corners. A user who draws much faster than the audit's stroke gets a longer
trail; the limits P8a to P8d measure this.

Because the path is the full polyline with stored transforms, the final pixels do not depend on the clock; only
how soon they appear does. A time budget without the queues would make pixels depend on the clock and is not
allowed. Reading the clock in the app is established practice (3.6).

The existing test `long_drag_respects_the_raycast_budget_and_finishes_on_release` (`app\tools\brush.rs:431-450`)
keeps its first assertion unchanged: `ActiveStroke` keeps its field `last` (the last resolved screen position) and
stays `Copy`, and one frame still resolves at most 64 steps of the 192-step drag. Its last line asserts that the
stroke is closed directly after `release`; that depends on the clock now, so the test gets one added line that
calls `end_frame` until the controller is idle (at most 10 times) before the assertion. This is the only existing
test that is edited for the pacing.

Considered and not done: thinning the queued path or coarsening the screen step when the trail grows. It does not
reduce the work (the number of dabs follows from the path length and the Spacing, `stroke.rs:207`, not from the
number of samples) and it would make the pixels depend on the clock.

### 5.11 The cursor in the 3D view

No pipeline, bind group or uniform layout changes. `ARCHITECTURE.md:3196` stays as it is.

- **Ring**: the ring is drawn around `c'` with the radius `r` for a ball and `k` for a shape (the ball the shape
  fits in). The free `brush_aux.yzw` carries the side normal
  (zero = no filter). The shader draws the ring only where `dot(normalize(in.normal), side) >= -0.2` when the side
  normal is set. Circular sets it to the hover hit's unflipped normal; Sphere leaves it empty. The uniform block
  keeps its 176 bytes, so the four shaders that mirror it and the layout test stay as they are. The thin inner
  hardness ring is drawn for the ball only.
- **Frozen radius**: while a 3D stroke is open the ring uses the stroke's radius (`ActiveStroke::radius_world`) and,
  for a shape, `stroke_tip_pose()`. This ends today's mismatch between ring and stroke (3.4).
- **Shape outline** (new code in `BrushTool::overlay`, which returns early for 3D views today,
  `app\tools\brush.rs:275-277`). With `h` the hover point, `A`, `up_t`, `right` from `tip_frame`, `R = A * U`:
  1. footprint: `field.section(origin = R^T (h - c') / k, u = R^T right, v = R^T up_t, n = 48)` gives segments in
     the tangent plane at the cursor; each end point `(x, y)` maps to the world point
     `h + k * (right * x + up_t * y)` and through `ViewXform::world_to_screen` to the screen. Drawn white, 1.5
     points, over a dark shadow, like the other overlay outlines. It is exact where the surface is flat under the
     brush and an approximation on curved surfaces.
  2. wires: the three stored principal sections, each point mapped by `c' + k * R * point`, drawn thin at 60 %
     white, so the orientation can be read above and below the surface.
  For a shape used Through the footprint is the stored outline (the principal section y = 0), each point mapped by
  `c' + k * R * (x, 0, z)` and clipped to the ring; with Tilt and Roll at 0 it shows exactly what is pressed
  through, on any surface.
  Cost per frame (estimate): 49 x 49 = 2,401 lookups and a few hundred line segments, well under 1 ms. The lines
  have no depth test (no overlay has one), which suits a volume that is half inside the model.
  The tool builds both as a list of screen segments in one function, `cursor_segments(state, xf) -> CursorLines
  { footprint: Vec<[Pos2; 2]>, wires: Vec<[Pos2; 2]> }`, and `overlay` only draws that list. Test A18 calls the
  function, so the outline is checked without a person looking at a screenshot.
- **While paint trails the pointer** (5.10): the ring and the outline are drawn at the last painted sample, not at
  the pointer, during the drag and while the stroke is finishing. The distance between the ring and the mouse
  pointer is the trail.
- **Sphere with the Ball**: today's ring on all visible fragments (it is exactly the border of the paint on visible
  surfaces), plus three wire circles of 48 segments, so the ball is visible where it floats off the surface.
- **2D view**: the two circles of today. When the pointer is in the 2D view of Split mode, the 3D view shows the
  plain ring at the matching surface point as today, without the shape outline and without the side filter.

### 5.12 Cost per dab against the budgets

`S` = sets not skipped, `F` = texels visited, `T` = texels touched.

| Step | Today | Proposed |
|---|---|---|
| BVH queries | S to 2S, each sorted; one `Vec` allocation per unreached set | 1 or 2 for all sets |
| Reach | side test per triangle per set | side test once per set list; `Connected`: about 18 checks per reached triangle |
| Per-triangle setup | per candidate | unchanged; field: plus 3 transformed corners and one lookup |
| Coverage | `F` x ball formula | ball: unchanged; field: `F` x (transform + grid lookup), estimated 3 times the ball |
| Commit, re-apply | `T` | unchanged |

Texels per dab on flat, undistorted UVs: `pi * (D / 2)^2` = 1,963 at D = 50, 205,887 at D = 512, 3,294,199 at
D = 2048. Estimates per dab on one 8192 x 8192 set (assumed 8 ns per ball fragment, 25 ns per field fragment, 5 ns
per committed texel, 10 ns per re-applied texel; large dabs are rasterized on the rayon pool, `brush.rs:72`):

| Size | Ball: rasterize + commit + re-apply | Field: rasterize + commit + re-apply | Dabs within the 10 ms of a frame that keeps up |
|---|---|---|---|
| 50 texels | 0.02 + 0.01 + 0.03 ms | 0.05 + 0.01 + 0.03 ms | over 100 (the 64 raycasts limit first) |
| 512 texels | 0.3 + 1.0 + 0.5 ms | 0.9 + 1.0 + 0.5 ms | 4 to 5 |
| 2048 texels | 4 + 16 + 7 ms | 12 + 16 + 7 ms | less than 1: one step per frame |

The field column is for hardness 100 %. Below that, fragments in the fade zone make one more lookup (estimate:
40 ns instead of 25 ns there). A Through dab reads the small two-dimensional slice and is cheaper per fragment
than Solid (estimate: 15 ns).

So at the largest size the stroke trails the pointer, but the window keeps answering. That is the meaning of
"interactive" this design commits to (5.10): no frame, the release frame included, spends more than 100 ms plus
one sample on stroke work; frames stay at 10 ms of stroke work while the stroke keeps up; the paint trails the
pointer by about a quarter second of work at most unless the stroke's work arrives faster than real time; and a
stroke that trails is completed after the release, in view of the user. A 2048-texel shape dab of 35 ms means
frames of about 35 to 100 ms while such a stroke is drawn.

On the 1.5-million-triangle, 15-set model with a 50-texel brush, rasterizing is small, and the queries are the part
this design can influence: 15 or more tree walks per dab become 1 or 2. How much of today's 35 to 59 ms per frame
that is has not been measured (3.8). The gate measures it first (8.4, step 0) and fixes the absolute limit for that
model from the measurement. What the pacing does there, with and without a gain from the single query, is worked
out in 5.10 from the audit's figures: today's frame rate, the paint about a quarter second behind, the stroke
complete about 0.3 s after the release. If the model stays at 13 to 20 frames per second, the gate report says so in plain
words for the user.

Not addressed: the single very long frames today (245 to 594 ms). They are probably first-use costs when a stroke
reaches a new set (a layer buffer and a stroke buffer per set, `stroke.rs:377-392`), but that is not verified, and
designing a fix without a measurement would be guessing. The gate records them separately (P12).

### 5.13 Memory added

| Item | Size |
|---|---|
| One baked field | at most 6.1 MiB (4 MiB distances, 2 MiB local thickness, the slice); at most 3 kept (loaded file, Cube, Cylinder). A flat outline: under 0.2 MiB. |
| Neighbour table per document | only after the first connected stroke: 26 to 54 MiB at 1.5 million triangles (estimate) |
| Flood marks | 12 MiB at 1.5 million triangles, per connected stroke |
| Per texel | nothing new (important at 8192 x 8192, where the stroke buffer is already 192 MiB per painted set) |
| GPU | nothing new |

A set that only the bounding ball of a shape reaches gets no stroke buffer (5.4 step 6). As today at the rim of
the ball, a set whose triangles are inside the shape's box but receive no paint still gets one (3 bytes per
texel). Whether the layer raster that `stroke_target` allocates at the first reach (`stroke.rs:379-383`) shows in a
saved file when no paint lands is not verified; the rule of 5.4 makes that case rarer than today, not more
frequent.

The pointer queues of 5.10 hold one screen position with its transform (under 100 bytes) or one ray hit (under
100 bytes) per entry; a stroke that trails by 10,000 samples holds about 1 MiB.

---

## 6. User interface

### 6.1 The pixel budget at 1280 x 800

Numbers from 3.5: the side panel is 748 px high. The bottom area is 299 px today. The complete Color section is
305 to 315 px. Tool options and Texture sets get what is left: **about 130 to 146 px**. The usable width of a row
is about 283 px (content from x = 989 to x = 1272 in `target\round3_audit\runs\paint3d_builtin\shot.png`), and the
panel font is about 6.1 px per character there (the label "Same side only", 14 characters, is 85 px wide in the
same picture). So a text line of up to 44 characters fits on one line; longer ones wrap and cost 18 px more.

| What | Height | Fits where |
|---|---|---|
| Brush options after this change: header 24 + five slider rows of 24 | 144 px | upper tool area, once the bottom area is sized as below |
| The same plus a Type row | 168 px | does not fit: the Spacing slider would be hidden |
| Brush tab, tallest state (6.3) | 232 px when no note wraps; 250 px if one of the two notes wraps | bottom area |
| Bottom area, computed (below): 285 to 295 px, minus 4 px margin, minus the 24 px tab strip | 257 to 267 px for the tab's content | |

Decisions that follow:

- The Brush tab goes into the bottom area, next to Layers.
- The Type control lives in the Brush tab only. A Type row in Tool options would push the Spacing slider out of
  sight at 1280 x 800. The status bar names the type while the pointer is over the model (6.6), and the cursor
  differs per type, so the type is visible without opening the tab.
- **The default height of the bottom area is computed, not a fixed share.** The panel package measures the height
  of the complete Color section on the merged round-3 build at 1280 x 800 and sets the default to
  `panel height - Color height - 144 - 4`, limited to today's 160..420 points (`app\panels\mod.rs:255-282`). With a
  Color section of 305 to 315 px that is 285 to 295 px: the five slider rows fit above (144 px), and the Brush tab
  has 257 to 267 px for a content of at most 250 px. A fixed share cannot satisfy both sides: 40 % (299 px) is up
  to 14 px too much for the sliders, 37 % (277 px) leaves 249 px, less than the 250 px the tab can need.
- Both note lines of the tab are at most 44 characters long (6.3), so they should not wrap; the budget still
  allows for one that does.
- New state `AppState::side_rects` holds the rectangles of the side panel, the tool area, the Color section, the
  bottom area and the Brush tab's content, written every frame. Test A12 and scenario S3 assert at 1280 x 800: the
  Color section lies inside the side panel; the five slider rows end above the bottom of the tool area; every row
  of the Brush tab in its tallest state lies inside the bottom area. If round 3 already records some of these
  rectangles for its own Color assert (`ROUND3_PLAN.md:308-310`), the scaffold reuses them.

At the smallest window (900 x 600) the bottom area is about 219 px, so the last rows of the Brush tab scroll; the
area already has its own scroll area (`app\panels\mod.rs:264`), and the user can drag its border.

### 6.2 The tab strip

The strip takes the place of the "Layers" collapsing header (`app\panels\layers.rs:83`), which is about as high, so
the Layers tab loses no room. It is one row, 24 points high, with two `egui::Button::selectable` buttons, the widget
the view-mode strip already uses (`app\view\mod.rs:151`). No new crate.

- State: `AppState::side_tab`. It is not in `PanelState`, which is rebuilt when the document changes (map, not
  re-read). It starts on Layers at every program start and is not saved.
- A click on a tab switches it: it is a setting, written directly. A shape the user loads with "Load..." shows the
  Brush tab when it is ready; a quiet reload does not switch the tab.
- "Brush" is enabled while the tool is Brush or Eraser. Otherwise it is greyed out with the hover text "Select the
  Brush or the Eraser", and **the panel shows Layers whatever `side_tab` says**, so the layer list is never hidden
  while another tool is in use. `SetTool` does not change `side_tab` (`app\dispatch.rs:631-635` stays as it is): a
  user who tunes the tip, picks a colour with the Eyedropper and returns to the Brush finds the Brush tab open
  again without a click.
- While the Brush tab is shown the Layers list is not drawn; `state.panel_rects` is cleared in that case, so no
  stale row rectangle is left for the self-test or for clicks (`layers.rs:82`). `side_rects.brush_tab` is `Some`
  exactly in those frames; tests ask this field what is shown.
- The first line of the Brush tab names the tool and the layer that will be painted: "Brush tip - paints on:
  Paint 1" (or "Eraser tip - ..."; "no paint layer" when there is none).
- The tab works without an open document.

### 6.3 Brush tab

Circular (about 100 px):

```
[ Layers ][ Brush ]
Brush tip - paints on: Paint 1
Type    [ Circular ][ Sphere ]
[ ] Stay on connected surface
Paints the surface you point at.
The 2D view always paints a flat disc.
```

Sphere with a shape (the tallest state, 232 px):

```
[ Layers ][ Brush ]
Brush tip - paints on: Paint 1         [ Reset ]
Type    [ Circular ][ Sphere ]
Shape   [ star.obj          v ] [ Load... ]
+--------+  Turn  [    0 deg ]
| 72 x72 |  Tilt  [    0 deg ]
+--------+  Roll  [    0 deg ]
Offset  -----o------   0 %
Align   [ Follow surface v ] [ Solid v ]
Paints everything inside the shape.
The 2D view always paints a flat disc.
```

Sphere with the Ball shows Type, Shape, Offset and the notes (about 150 px).

- Row heights: first line 24, Type 24, Shape 24, preview block 76, Offset 24, Align and Use 24, two note lines 36.
- Row widths, all within the 283 px that are usable: label column 72 (`tool_options.rs:21`); Shape = 72 + 140 + 8 +
  56 = 276; preview block = 72 + 8 + 40 + 72 = 192; Align and Use = 72 + 120 + 8 + 76 = 276; the first line
  truncates the layer name with "...".
- First note line, by state (each at most 44 characters): Circular "Paints the surface you point at.";
  Sphere with the Ball "Also paints far sides and hidden parts."; a shape used Solid "Paints everything inside the
  shape."; a shape used Through "Presses the outline straight through.". Second note line, always: "The 2D view
  always paints a flat disc."
- The Shape list shows Ball, Cube, Cylinder, the file entry and, below a separator, "Remove file shape". The file
  entry reads the file's name when a model is loaded or remembered (hover text: full path and, when loaded,
  triangle count and "Up axis: Z" if the shape was laid flat), "File (none loaded)" and greyed out when no path is
  remembered, and "Loading..." in front of the name while a load runs. Choosing the entry of a remembered but not
  yet loaded file pushes `ReloadBrushShape`. "Remove file shape" pushes `RemoveBrushShape` and is greyed out when
  no path is remembered. A small "!" with a hover text follows the list when `last_error` is set (its message),
  when the report says `partial` ("Only the closed parts of this file are used") or `thin` ("Parts of this shape
  are thinner than the brush can show").
- **Use**: "Solid" / "Through" (5.6). Greyed out and fixed on Through for a flat outline. Hover texts: "Solid: the
  shape is a volume; paint lands where the surface passes through it." and "Through: the shape's outline, seen
  from above, is pressed straight through the surface inside the ring. Use it for flat or thin shapes."
- Turn, Tilt, Roll and Offset are drag values and a slider that change the settings directly, as all tool settings
  do. "Load...", the file entry of a shape that is not loaded and "Remove file shape" are actions.
- The preview is the three stored principal sections, turned by the angles, drawn with the egui painter in a fixed
  three-quarter view with a line for the surface; the part below the surface is drawn dimmer. Dragging it changes
  Turn and Tilt.
- Every control has a label or a hover text that the control tests can find, as the existing sliders have.

### 6.4 Tool options

- The "Same side only" checkbox is removed (`app\panels\tool_options.rs:165-166`).
- The Size hover text becomes "Brush diameter in texels; a brush shape is scaled to fit inside this ring (keys)".
  It still starts with "Brush diameter", which a control test uses to find the slider (`panel_tests.rs:365`).
- The Spacing hover text gains "A thin shape needs a lower value."

### 6.5 Cursor summary

| Brush | 3D view | 2D view |
|---|---|---|
| Circular | the ring of today, only on faces of the cursor's side | the two circles of today |
| Sphere, Ball | the ring on every visible face inside the ball, and three wire circles | the two circles of today |
| Sphere, a shape | the ring (outer limit), the bold footprint outline at the cursor (Solid: the cut with the tangent plane; Through: the outline that is pressed through), three thin wire sections | the two circles of today |
| Any type, while paint trails the pointer | ring and outline at the last painted point | the two circles of today |
| Any type, during a 3D stroke | as above, with the stroke's own radius and orientation | the two circles of today |

### 6.6 Status bar and messages

- Hover hint with the brush over the model: "Circular brush" / "Sphere brush: paints everything inside the ball,
  also hidden surfaces" / "Shape brush: star.obj".
- During a stroke whose paint trails by more than 250 ms of work: "Painting... the stroke is catching up". After
  the release, until the stroke is complete: "Finishing the stroke... (Esc discards it)" (5.10).
- Load success: info toast "Brush shape loaded: star.obj (1,280 triangles)".
- `report.partial`: warning toast "Only the closed parts of star.obj are used as the brush shape. Open or very
  thin parts are left out."
- `report.thin`: warning toast "Parts of star.obj are thinner than the brush can show."
- `report.flat`: info toast "star.obj is flat. Its outline is pressed straight through the surface (Use:
  Through)."
- `report.height_ratio` under 0.25 and not flat: info toast "star.obj is a thin shape, so Use was set to Through.
  With Solid it paints only where the surface passes through the shape."
- `NoInterior`: error toast and message window: "This model has no closed inside that the brush can use. If it is
  flat or very thin, give it more thickness (2 % of its longest side or more), or lay it flat in one of the file's
  main planes so that it can be used as an outline. If it has openings, close them in a 3D program first." The
  shape in use is kept (4.4).
- `TooManyTriangles`: "This model has 2,400,000 triangles; a brush shape may have at most 2,000,000."
- Import failure: the importer's message through the existing failure path (`report_failure`,
  `app\io_flows.rs:80-98`). Cancel: info toast "Loading cancelled".
- Press with File selected, no shape loaded yet and a load running: no stroke, info toast "The brush shape is still
  loading".
- Shape file missing at a quiet reload: warning toast "Brush shape not found: <path>. Using the ball." The tools go
  back to the Ball; the path stays remembered (4.4).

---

## 7. Work breakdown

No new crate. Everything uses `glam`, `rayon`, `serde` and `egui`, which are already dependencies
(`crates\wte_core\Cargo.toml:7-11`). `cargo build --offline` therefore needs nothing that is not already resolved.

### 7.1 Overlap with round 3 and with the text feature

- Round 4 starts from the round-3 result. Round 3 owns, until it is merged, `core\import\*`, `mesh.rs`,
  `mesh_build.rs`, `document.rs`, `texel_map.rs`, and every app file except `selftest\*`
  (`target\round3\ROUND3_BUILD_RULES.md:73-78`). `brush.rs`, `stroke.rs` and `bvh.rs`, where the engine work is,
  belong to no round-3 package (same file, line 80).
- In the files round 3 changes, this design only adds: one function in `import\mod.rs`, one field in `document.rs`
  and its initial value in `doc_build.rs`, a tab strip and a new panel file, one action, one job output. It replaces
  no code that round 3 writes, and it changes nothing in `texel_map.rs`, `stamp_bake.rs`, `history.rs` or
  `project\*`. Its one dependency on round 3 is the side panel layout (Color pinned, the bottom area resizable with
  its own scroll area).
- The text feature of round 4 also needs `app\state.rs`, `action.rs`, `dispatch.rs`, `io_flows.rs`,
  `panels\tool_options.rs`, `selftest\scenarios.rs` and `ARCHITECTURE.md`, and possibly `tools\mod.rs` and
  `view\mod.rs`, which the scaffold edits for the pacing (7.3). Proposal: one scaffold package and one
  gate package serve both features. This design keeps its own flow code in the new file `app\brush_flows.rs`, so it
  needs only one-line arms in `dispatch.rs` and `io_flows.rs`, written by the scaffold; and it needs only a
  two-line removal and two hover texts in `tool_options.rs`, also done by the scaffold.

### 7.2 Files touched

Sizes are estimates and include tests.

| File | Change | Lines |
|---|---|---|
| `core\brush_tip.rs` (new) | types of 4.1, `tip_frame`, tests | 180 |
| `core\tip_field.rs` (new) | grids, lookups (`F`, `T`, `G`, `T2`), the thickness transform, `from_fn`, `outline_from_fn`, Cube, Cylinder, sections, hash, tests | 520 |
| `core\tip_bake.rs` (new) | 5.7 (two passes, skin points, lying flat, slice) and 5.7b (flat outline), errors, tests | 760 |
| `core\topology.rs` (new) | 5.5 table, tests | 260 |
| `core\bvh.rs` | `query_sphere_dist`, test | 70 |
| `core\import\mod.rs` | `import_geometry_from_path`, test | 50 |
| `core\brush.rs` | shared candidates, reach and flood with the new seeds, field coverage with local thickness, Through, field fast paths, tests | 760 |
| `core\stroke.rs` | `stroke_begin_with`, session, restructured `surface_dab`, `stroke_tip_pose`, bench, tests | 350 |
| `crates\wte_core\tests\brush_tips.rs` (new) | end-to-end tests of 8.1 | 330 |
| `core\samples.rs` | new generators (8.1) | 220 |
| `core\document.rs`, `doc_build.rs`, `lib.rs` | topology field and accessor, its initial value (`doc_build.rs:337`), re-exports | 30 |
| `app\state.rs`, `action.rs`, `jobs.rs`, `gpu\params.rs`, `gpu\mod.rs` | types of 4.2, `sanitized`; the `BrushRing` literal in the test at `gpu\mod.rs:248` | 140 |
| `app\brush_flows.rs` (new) | three actions, dialog, job, request numbers, the event table of 4.4, quiet reload, `sync_prefs`, tests | 300 |
| `app\prefs.rs`, `app.rs`, `dispatch.rs`, `io_flows.rs` | lenient and clamped reading and tests; call sites into `brush_flows.rs`; `discard` for Undo and Esc; guard entry | 150 |
| `app\tools\mod.rs`, `app\view\mod.rs` | controller side of 5.10: `end_frame` pump and repaint, busy while finishing, `cancel` completes a released stroke, `discard`, press during finishing | 60 |
| `app\tools\brush.rs` | tip mapping, the two queues, time rule, finishing, replaceable clock, wheel, ring data, `cursor_segments`, hints, the per-stroke log line, tests | 620 |
| `app\gpu\renderer.rs`, `gpu\shaders\mesh.wgsl` | side normal into `brush_aux.yzw`, shader condition, tests | 35 |
| `app\panels\mod.rs`, `layers.rs` | tab strip, switch, `side_rects`, bottom-area default | 70 |
| `app\panels\brush_tab.rs` (new) | the Brush tab and its preview | 370 |
| `app\panels\tool_options.rs` | checkbox removed, two hover texts | 4 |
| `app\control_sweep\panel_tests.rs` | three tests adapted, tab tests | 220 |
| `app\selftest\scenarios.rs`, `selftest\mod.rs` | four scenarios, helpers, name count; `Step::Hover`; input steps end when the tools are idle | 460 |
| `ARCHITECTURE.md` | sections of 4.7 | 300 |

Total: about 6,300 lines (5,100 before the review; the additions are the two-pass bake and flat outlines, local
thickness, Through, the pacing with finishing, the shape life cycle and their tests).

### 7.3 Packages with disjoint files

**Step 0 — measure (the gate agent, no feature code).** On the merged round-3 tree: the baseline of 8.4. Its numbers
go to `target\round4\perf\baseline.txt`. No package starts engine work before this file exists.

**Wave 0 — scaffold S (one package, lands first).** Writes the shared contracts completely; nothing else changes
them afterwards:

- `core\brush_tip.rs` complete (types, `legacy`, `tip_frame`, `TipPose`).
- `core\tip_field.rs` complete (it is small and everything else needs it): the grids, the lookups `F`, `G`, `T`,
  `T2`, `from_fn`, `outline_from_fn`, `cube`, `cylinder`, sections.
- `core\tip_bake.rs`: `TipError` and the two `bake` signatures returning `Err(NoInterior)`; no `todo!`.
- `core\topology.rs`: the public signatures and a correct serial `build`.
- `core\document.rs`, `doc_build.rs`, `lib.rs`: the topology field, accessor and re-exports.
- `core\stroke.rs`: `stroke_begin_with` and `stroke_tip_pose` exist; `stroke_begin_with` behaves as `stroke_begin`
  for `SameSide` and `All` with a ball and ignores the rest until package E lands.
- `core\samples.rs`: the generators of 8.1 and the three file writers of section 8 (`to_gltf`, `to_glb`,
  `to_fbx_ascii`), which the bake tests F21 to F24, the app test A16 and scenario S2 use.
- `app\state.rs`, `action.rs`, `jobs.rs`, `gpu\params.rs`: the types of 4.2 (with `BrushSettings::sanitized`), with
  every struct literal and every use of `same_side_only` adjusted so that the workspace compiles and all tests
  pass:
  - `same_side_only`: `app\tools\brush.rs:69` (derived from `kind`), `app\state.rs:164-170`,
    `app\panels\tool_options.rs:165-166`, `control_sweep\panel_tests.rs:372-382` and `1025-1033` (the checkbox parts
    are removed; package D1 adds the new tab tests);
  - `BrushRing`: `side_normal: None` at `app\gpu\mod.rs:248`, `app\gpu\renderer.rs:861`, `app\tools\brush.rs:299`;
  - `Prefs`: `..Prefs::default()` at `app\prefs.rs:106-115` and `app\state.rs:805-814` (listed cross-package edit
    in `prefs.rs`, which package D2 owns afterwards).
- `app\dispatch.rs`, `io_flows.rs`, `app.rs`: one-line calls into `app\brush_flows.rs`, whose functions exist with
  empty bodies: the three actions, the result of `JobOutput::BrushShape`, the start-up check, and
  `brush_flows::sync_prefs` before both `prefs::save` calls (`app.rs:321`, `io_flows.rs:244`). `LoadBrushShape`
  joins the interaction guard (`dispatch.rs:51-94`), because its modal job takes the pointer away from a held
  stroke; `ReloadBrushShape` and `RemoveBrushShape` do not (no modal job, no document data).
  In the guard (`dispatch.rs:33-43`) the Undo case calls `tools.discard`, the others keep `tools.cancel`; Esc
  (`dispatch.rs:757-763`) calls `tools.discard`. `SetTool` is not changed.
- `app\tools\mod.rs` and `app\view\mod.rs`, the controller side of 5.10: `ToolController::end_frame` also calls
  `self.brush.pump(state)` and returns `Some(0.0)` while the brush has queued work; `release` keeps the
  interaction while `self.brush.finishing()`; `cancel` completes a finishing brush stroke; new `discard` and
  `finishing`; `press` (`tools\mod.rs:352`) and the press branch of `route_pointer` (`view\mod.rs:383`) complete a
  finishing stroke before their busy check. In the scaffold `BrushTool::pump` does nothing and `finishing` returns
  `false`, so behaviour is unchanged until package F lands.
- `app\selftest\mod.rs`, one line: an input step (`Active::Input`, `selftest\mod.rs:338-341`) is complete when its
  injected frames are processed **and the tools are not busy**. Every pointer step of every scenario, old or new,
  then ends only when its stroke is complete, however long the frames were.

Files the scaffold writes completely have no later owner. A package that finds a defect in one fixes it as a listed
cross-package edit, as in round 3 (`target\round3\ROUND3_BUILD_RULES.md:80-85`). A package that needs one more test
fixture builds it inside its own test module.

After wave 0 the workspace builds, all old tests pass and painting behaves exactly as before.

**Wave 1 — six packages in parallel.**

| Package | Owns | Delivers |
|---|---|---|
| A core-bake | `core\tip_bake.rs`, `core\import\mod.rs` | 5.7 and 5.7b, `import_geometry_from_path`, tests F2 to F12, F14 to F19 and F21 to F25 (F1, F13 and F20 test `tip_field.rs` and come with the scaffold) |
| C core-topology (dropped if the answer to Q1 is "no checkbox") | `core\topology.rs` | parallel `build`, tests T1 to T4, the measurement for P9 |
| E core-paint | `core\brush.rs`, `core\stroke.rs`, `core\bvh.rs`, `crates\wte_core\tests\brush_tips.rs` | 5.2 to 5.6 and 5.9: `query_sphere_dist`, shared query, reach and flood, field coverage with local thickness, Through, fast paths, frames, the bench of 8.4; tests C1 to C26 |
| D1 app-panels | `app\panels\*`, `app\control_sweep\*` | tab strip, Brush tab, `side_rects`, the measurement and the computed bottom-area height of 6.1, tests A11 to A13 |
| D2 app-flows | `app\brush_flows.rs`, `app\prefs.rs` | the three actions, dialog, job, request numbers, the table of 4.4, `sync_prefs`, lenient and clamped reading, tests A10, A16, A20 |
| G app-gpu | `app\gpu\renderer.rs`, `app\gpu\shaders\mesh.wgsl` | ring side filter, tests A14, A15 |

E builds its volumes with `TipField::cube()`, `cylinder()`, `from_fn` and `outline_from_fn`, and uses the
scaffold's serial topology, so it waits for neither A nor C. D2 compiles against the scaffold's `bake` signature
and is merged after A.

**Wave 2 — one package.**

| Package | Owns | Delivers |
|---|---|---|
| F app-tools | `app\tools\brush.rs` | settings to tip (4.3), the two queues, pacing and finishing (5.10: `pump`, `finishing`, completion, discard), wheel, ring data, outline and wires as a segment list (5.11), hints; tests A1 to A9, A17 to A19 |

F may start in wave 1 against the scaffold; its tests that need the real engine (A2, A3) wait for E.

**Gate — one package, last.** `app\selftest\*` (the scenarios and the new step `Step::Hover { view, at }`: one
`PointerMoved` to the point and no button, about 15 lines next to `scroll_frames`, `selftest\mod.rs:581-596`),
`ARCHITECTURE.md` (4.7), the final performance runs, the release build, the report. The gate takes the screenshots
of S2 and S3 on the user's machine.

Order: step 0, then S, then A C E D1 D2 G together (F may start), then F, then the gate.

---

## 8. Acceptance tests

All unit tests are headless and use no file outside the workspace unless marked "ignored". Scenarios run in the
existing self-test runner with `Some(path)` actions; no OS input is injected. Test files and logs go to E:. The
runner gets two small additions (7.3): the step `Step::Hover { view, at }`, which moves the pointer onto a view
without pressing (today every pointer step presses, `selftest\mod.rs:553-574`, and every scroll step sends a wheel
event, `582-596`), and the rule that an input step ends only when the tools are idle.

New generators in `core\samples.rs`, each with documented UV rectangles: `thin_slab(thickness)` (a 1 x 1 plate, six
islands), `fold(inner_angle_deg)` (two unit quads sharing an edge, two islands), `two_blocks(gap)` (two unit cubes,
twelve islands), `fan_strips()` (two long coplanar strips 0.1 apart that share one corner 2 units away),
`undercut_ridge()` (a plate with a ridge whose walls lean 110 degrees away from the plate normal, and a plateau
behind it), `t_junction_quads()` (a quad x, y in 0..1 next to two quads x in 1..2 that split its edge at y = 0.5),
`cracked_quads(eps)`, `flat_quad_tilted(degrees)` (`flat_quad` turned about X, UVs kept),
`soup_sets(seed, triangles, sets)`, and for the bake, without UVs: `open_box(side)`,
`overlapping_cubes()` (two unit cubes, the second shifted by 0.5 along X), `cube_with_inner_wall()` (also with
the wall slanted, so that it meets the skin at an angle), `doubled_cube()`, `tilted_quad()` (a quad turned 30
degrees about X and 20 about Y), `cube_on_floor()` (a closed unit cube standing on an open 10 x 10 quad),
`cube_floor_rod()` (the same plus a closed rod 0.05 thick and 3 long that sticks out of the cube), `corner_cubes()`
(two closed unit cubes in opposite corners of a 20-unit box), `coin(diameter, thickness)` (a closed cylinder, axis
Y), `flat_star()` (a five-armed star polygon in the XY plane, outer radius 1, inner radius 0.4, no thickness),
`plate_xy()` (a closed box 1 x 1 x 0.2, thin along Z).

Three file writers next to `samples::to_obj` (`core\samples.rs:386`), so that every format the importer reads
(`core\import\mod.rs:91-109`) has a test file that is made by the test itself and needs nothing on disk:

- `to_gltf(raw, node) -> String`: a `.gltf` JSON text with one mesh, one node that carries the 4 x 4 matrix `node`,
  and the vertex data as a base64 `data:` buffer.
- `to_glb(parts) -> Vec<u8>`: a binary `.glb` (12-byte header, one JSON chunk, one BIN chunk) with one node per
  entry of `parts: &[(RawMesh, Mat4)]`, so that one file can hold several meshes with different node transforms.
- `to_fbx_ascii(raw, unit_scale_factor, lcl_scaling) -> String`: an ASCII FBX 7.4 text with one mesh model, the
  global setting `UnitScaleFactor` and the model property `Lcl Scaling`.

What was read in the importer for this: it recognises ASCII FBX by its first line (`core\import\mod.rs:98-100`),
reads the unit setting (`core\import\fbx.rs:608`, `:621`) and bakes glTF node transforms into the vertices
(`core\import\gltf.rs:687`); its own tests hold file literals of this kind (`core\import\fbx.rs:1574`, `:1643`).
How the FBX importer applies `Lcl Scaling` to the vertices was not verified for this document; test F23 finds out.
The three writers are about 120 lines together and are written by the scaffold with the other generators.

### 8.1 Core unit tests

Paint engine (`brush.rs`, `stroke.rs`, `bvh.rs`, `tests\brush_tips.rs`):

| # | Test | Passes when |
|---|---|---|
| C1 | Nothing old changes | Every existing test of `brush.rs`, `stroke.rs`, `bvh.rs` and `crates\wte_core\tests` passes without an edit. One exception, caused by the text feature of the same round and not by the brush: the scaffold adds one line (`fx: TextFx::default()`) to each `TextSpec` literal in `crates\wte_core\tests\end_to_end.rs` and `project_roundtrip.rs`. No assertion and no expected value of an existing test changes. |
| C2 | Shared query equals the per-set path | A test-only loop that calls the unchanged `dab_3d` once per set, as `surface_dab` does today, is the reference. For 200 seeded random dabs on `two_material_model` at 256 x 256 and on `soup_sets(1, 20000, 4)`, and for 30-sample strokes on `wrapped_quad`, `scattered_islands` and `mirrored_quads`, the masks of all sets and the composite hashes are equal, for `SameSide` and for `All`. |
| C3 | One source of truth | `stroke_begin` with `same_side_only` true / false gives the same composite hash as `stroke_begin_with` with reach `SameSide` / `All`; `stroke_begin_with` with `SameSide` gives the same hash whatever `brush.same_side_only` says. |
| C4 | `query_sphere_dist` | Agrees with brute force on the random soup of the existing BVH test (map, not re-read) for 1,000 centres; its ids equal `query_sphere`'s in the same order. |
| C5 | The rows of the table in 1.7 | At 256 x 256, radius 0.2, counting touched texels per UV island. `thin_slab(0.05)`: back island 0 for `SameSide` and `Connected`, within 5 % of the brute-force count for `All`. The same plate at 512 x 512 with a brush diameter of 64 texels (radius 0.0625, a realistic size): back island 0 for `SameSide`, over 20 for `All`. `fold(90)`: both flanks over 50 texels for all three. `fold(60)`: other flank 0 for `SameSide` and `Connected`, over 50 for `All`. `two_blocks(0.01)`, dab on the first block's top next to the gap: second block's top island 0 for `Connected`, over 50 for `SameSide` and `All`. `two_material_model`, dab on the head at (0, 0.8, 0.35), radius 0.5: body composite unchanged for `Connected`, changed for `SameSide`. `fan_strips()`, radius 0.3: second strip 0 for `Connected`, over 50 for `SameSide`. |
| C6 | `Connected` against `SameSide` | Equal mask hashes for a 30-sample stroke on `uv_sphere(48, 24, 1.0)` with radius 0.15 and for a corner dab on `seam_cube(1.0)`. On `undercut_ridge()`: the plateau behind the ridge has over 50 touched texels for `SameSide` and 0 for `Connected`. |
| C7 | T-junction | `t_junction_quads()`, dab at (0.9, 0.5), radius 0.3: all three quads have touched texels with `Connected`. |
| C8 | Crack under the centre | `cracked_quads(1e-6)`: a dab centred on the crack paints both quads; a dab centred 0.2 radii away paints one. |
| C9 | Brute-force oracle | For the ball with `All` and for the Cube field, on `uv_sphere` and `seam_cube` at 256 x 256: for every covered, non-gutter texel compute its point with `TexelMap::locate` (`core\texel_map.rs:587-607`) and the formula of 5.2 directly; the dab's coverage byte differs by at most 1, and no texel with oracle coverage over 1/255 is missed. |
| C10 | A cube tip prints a square | `flat_quad` at 512 x 512 (512 texels per unit, square), Cube, Follow surface, hardness 1, radius 100 texels: texels with coverage of 128 or more number 13,333 within 3 % and span 115.5 texels within 2 in both directions (the cube's side is 200 / sqrt(3)). With Turn 45 degrees: span 163.3 within 3, count within 3 %. With "Fixed in space" on `flat_quad_tilted(30)`: span 115.5 by 133.3 within 3 (the plane cuts the cube at an angle). |
| C11 | Cylinder | Same setup: a disc of radius 70.7 texels within 1.5 (15,708 texels within 3 %). With Tilt 90 degrees: 141.4 by 141.4 within 3 (20,000 texels within 3 %). |
| C12 | A ball-shaped field equals the ball | `from_fn` with `|q| - 1` on 129 nodes per axis, on `flat_quad` (512) and `uv_sphere` (256), diameters 16, 64 and 200 texels, hardness 0.5 and 1: coverage differs from the Ball tip by at most 4 of 255 at every texel, and by under 0.5 of 255 on average. |
| C13 | A baked ball matches the ball | Bake `uv_sphere(96, 48, 1.0)`; one dab of radius 60 texels on `flat_quad`: mean absolute difference under 1.0 % of full scale, largest under 12 %. |
| C14 | Offset | Ball, radius 80 texels, hardness 1, on `flat_quad`. Offset 0.5: the touched disc has a radius of 69.3 texels within 1.5. Offset 1.0: every touched texel lies within 9.5 texels of the contact point and none has more than half coverage. |
| C15 | Frames | `tip_frame` is right-handed with its Y column equal to the normal for 1,000 random normals and hints; normal +Z with hint +Y gives the columns (+X, +Z, -Y); a hint parallel to the normal still gives an orthonormal frame. A stroke over the pole of `uv_sphere` with a rod-shaped field: consecutive dab frames differ by under 10 degrees. On `reverse_winding(flat_quad)` hit from +Z: the frame's Y column is +Z, and a Cube tip with offset +0.5 touches the same number of texels (within 1 %) and the same bounding box (within 1 texel) as on `flat_quad`. |
| C16 | Folded equals unfolded for a field | `wrapped_quad` (tiled UVs) with the Cube, the Cylinder and the baked `overlapping_cubes()`, 20 seeded random poses each (rotation, offset, hardness 0.3 / 0.8 / 1.0), radius large enough for block folds: the mask equals the mask with folding switched off, texel for texel. |
| C17 | Parallel equals serial | A field dab of more than 65,536 texels gives the same mask on the pool and serially (pattern of the existing test, map, not re-read). |
| C18 | Tiny brush | A field dab with a radius of 0.4 texels covers the texel under the dab centre. |
| C19 | Never outside the bound | 200 seeded random poses: every texel with a non-zero mask has its 3D point within `R_b` plus one texel of `c'`. |
| C20 | History and opacity | A stroke with the Cube on `two_material_model`: one history entry; undo restores the document hash; redo gives the stroke's hash; `stroke_cancel` restores and pushes nothing. A self-crossing stroke at opacity 50 %: no texel above alpha 128. |
| C21 | 2D ignores the tip | A 2D stroke after `stroke_begin_with` with a field tip gives the same pixels as after `stroke_begin`. |
| C22 | Clamping | NaN rotation becomes identity; offset 5 becomes 1; scale NaN becomes 1; a zero `up_hint` becomes +Y; `through = false` with a flat outline behaves as `true`. |
| C23 | Thin parts are not faded as a whole | `from_fn` shape "ball with a fin": the union of a ball of radius 0.6 and a slab with half sizes 0.95 x 0.06 x 0.3 (thin along Y). On `flat_quad` at 512 x 512, radius 150 texels, hardness 0.8, Follow surface (the surface cuts the fin along its middle plane): every texel on the fin's centre line between 0.7 and 0.85 tip units from the centre has coverage of at least 242 of 255. With Offset 0.03 (half way to the fin's face): still at least 242. With Offset 0.054 (10 % of the half thickness left): between 100 and 155. The texel at the dab centre is 255 in all three. The same test with `T` replaced by the inradius (a test-only switch) gives under 140 on the centre line, which is the defect this guards against. |
| C24 | A set that only the bounding ball touches is not reached | `two_material_model` with the head's paint layer locked, Cube tip, a dab on the body's top face near a corner, placed so that the bounding ball holds head triangles and the cube's box, grown by one texel, holds none (the test asserts both conditions first): `stroke_skipped_sets()` is empty and the head's set has no stroke buffer. The same dab with the Ball tip: the head's set is reported as skipped (today's behaviour). |
| C25 | Connected dabs between samples stay on their part | `two_material_model`, `Connected`, radius 0.1, a stroke of samples 0.05 apart along the head at height y = 0.501 (0.001 above the body's top face; every chord point is nearer to the body than to the head's surface): the body's set hash is unchanged and the head has touched texels at every sample. With `SameSide` the body changes. |
| C26 | Through prints the outline on a curved surface | `outline_from_fn` with a five-armed star (outer radius 0.95, inner 0.4) on `uv_sphere(48, 24, 1.0)` at 512 x 512, radius 0.4 units, hardness 1, Follow surface, one dab: for each arm the texel whose point, projected into the tangent plane at the dab centre, lies at 0.85 of the arm's length on the arm's axis has coverage of at least 128; the texel at the same distance on the bisector between two arms has 0. A `from_fn` plate of the same star, 0.05 tip units thick each way, used Solid: all five arm-tip texels have 0 (the stated limit of Solid, 1.7); the same plate used Through: at least 128. Folded and unfolded rasterization agree for the Through dab on `wrapped_quad`. No texel with a non-zero mask is farther than `k` plus one texel from `c'`. |

Field and bake (`tip_field.rs`, `tip_bake.rs`):

| # | Test | Passes when |
|---|---|---|
| F1 | Built-in Cube | `sample(0,0,0)` = -0.577 within 0.005; `sample(0.57735,0,0)` = 0 within 0.005; `sample(0.9,0,0)` = +0.323 within 0.01; `inradius` = 0.577 within 0.01. |
| F2 | Baked cube against the formula | `seam_cube(1.0)`: at nodes within one cell of the skin the difference is at most 0.05 cells; at all nodes at most 1.2 cells; the sign agrees wherever the formula is more than one cell from zero. |
| F3 | Winding does not matter | `reverse_winding(seam_cube)` gives the same hash. |
| F4 | Duplicated faces do not matter | `doubled_cube()` gives the same hash as the cube. |
| F5 | An inner wall leaves no trace | `cube_with_inner_wall()`, with the wall axis-aligned and with the wall slanted so that it meets the skin within one cell of border nodes: the skin set of step 5 holds no wall triangle, and the hash equals the cube's. |
| F6 | Overlapping solids are one solid | `overlapping_cubes()`: `sample` is negative at both centres and in the overlap; at 200 points on the buried face x = 0.5 that are at least 3 cells from the union's skin the value is below -2 cells and equals minus the distance to the union's skin within 1.5 cells. |
| F7 | Shapes without an inside | `tilted_quad()` and `open_box(1.0)` give `Err(NoInterior)`, and the error's text contains "thickness" and "openings". `flat_quad` (flat in a main plane) gives `Ok` with `report.flat`: `outline(0, 0)` is negative, and the outline's zero level is the quad's edge within one cell. |
| F8 | A face exactly on a plane of nodes | A test-only entry point bakes the cube with phase 0, so that its faces lie exactly on planes of nodes: `Ok`; the number of inside nodes is within 2 % of the normal bake's; `sample` at the centre is within one cell of -half side. |
| F9 | Neighbouring nodes, and far nodes | For the baked cube, `overlapping_cubes()`, `uv_sphere(48, 24, 1.0)`, `no_uv_tetra` and `corner_cubes()`, and for the built-in Cube and Cylinder: every pair of neighbouring nodes differs by at most 1.001 cells, in `F` and in `G`. `corner_cubes()` bakes without a panic, and a node in an empty corner of its grid, more than 128 cells from both cubes, holds exactly the saturated value +127.99 cells. |
| F10 | A mesh without UVs | `no_uv_tetra` bakes; `sample` at its centre is negative. |
| F11 | Deterministic | Two bakes give the same hash; a bake inside a one-thread rayon pool gives the same hash. |
| F12 | Cancel and limit | A cancelled `Progress` gives `Err(Cancelled)`; a mesh over the triangle limit gives `TooManyTriangles` (tested through a private limit parameter). |
| F13 | Sections | The cut y = 0 of the Cube is a closed loop whose points satisfy `max(|x|, |z|)` = 0.577 within 0.015; `section` on a plane turned 45 degrees about Y gives a closed loop. |
| F14 | Geometry import | `import_geometry_from_path` on an OBJ written with `samples::to_obj` whose MTL names a texture file that does not exist: `Ok`, same triangle count as `import_from_path`. |
| F15 | Odd files, ignored (files on this machine) | Every OBJ in `target\compat\harness\shapes` (among them `klein_bottle`, `mobius`, `nonmanifold`, `flat_annulus`, `coincident_flipped_spheres`) and `target\compat\topologies\models\{open_shell, holes, inverted_normals, mixed_winding, nonmanifold_fins}` bakes to `Ok` or `NoInterior` without a panic, each within 10 s. |
| F16 | Unused parts of a file do not set the size | `cube_on_floor()`: `Ok`, `report.partial` is true, the longest grid axis has 128 nodes and the cube spans at least 75 cells; `inradius` is within one cell of the plain cube's 0.577; one dab on `flat_quad` as in C10 prints a square whose span is within 1.5 texels and whose texel count is within 3 % of the plain cube's. The plain cube has `report.partial == false` and its hash is the same as before this rule existed (one pass). |
| F17 | A thin closed model is not refused for being thin | `coin(150, 1)` (0.67 % thick): `Ok`; `outline(0, 0)` is negative and the outline is a circle of radius 1 within 2 cells; `report.flat` or `height_ratio < 0.25` holds. `coin(150, 6)` (4 % thick): `Ok` as a solid, `height_ratio` = 0.04 within 0.01. |
| F18 | Lying flat | `plate_xy()`: `report.up_axis == 2`; the solid's extent along tip Y is the thin one (0.2 of the others within one cell); `seam_cube` keeps `up_axis == 1` and its old hash. `flat_star()` (in the XY plane): `report.flat`, `up_axis == 2`, and `outline` is negative on an arm axis at 0.8 and positive on a bisector at 0.8. |
| F19 | A closed part cut by the second grid does not flood the solid | `cube_floor_rod()`: `Ok`; `sample` at the cube's centre is negative; `inside_fraction` is within 10 % of `cube_on_floor()`'s. |
| F20 | Local thickness | Baked `seam_cube`, built-in Cube and a baked `uv_sphere`: `thickness(q) >= -sample(q)` minus half a cell at 1,000 random inside points; for the sphere `thickness` is the inradius within 1.5 cells at all of them; for the fin shape of C23 it is 0.06 within 1.5 cells on the fin's middle plane beyond 0.75 from the centre and 0.6 within 1.5 cells at the centre. |
| F21 | A shape from a `.gltf` file | `seam_cube(1.0)` written with `to_gltf` (identity node) into `target\wte-test-tmp`: `import_geometry_from_path` gives `Ok` with 12 triangles; the bake gives `Ok`; `inside_fraction` is within 2 % and `inradius` within one cell of the bake of the same cube from the OBJ of F14; `report.partial` is false. |
| F22 | A shape from a `.glb` file | The same cube written with `to_glb`: the same four assertions. A second file whose name ends in `.bin` but whose bytes are this GLB also bakes (the format is found from the content first, `core\import\mod.rs:91-94`, called by the import at `:144`). |
| F23 | A shape from an `.fbx` file, and FBX units | The same cube written with `to_fbx_ascii(raw, 1.0, [1, 1, 1])`: the four assertions of F21. Written with `UnitScaleFactor` 100 (one file unit = 1 m): `Ok`, and `inside_fraction` and `inradius` equal the first FBX bake within one cell, because the bake scales the solid to the grid whatever its size in the file. Written with `Lcl Scaling` (1.5, 1, 1): the extents of the baked solid (the span of nodes with `sample < 0` along each grid axis) are in the ratio 1.5 : 1 : 1 within one cell. |
| F24 | Node transforms and several meshes in one file | (i) `to_glb` with one cube under a node scaled by (1.5, 1, 1): extents 1.5 : 1 : 1 within one cell, the long one along tip X, `up_axis == 1`. (ii) The same node with an added quarter turn about Z: extents 1 : 1.5 : 1, the long one along tip Y. (Both boxes are too chunky for the lying-flat rule of 5.7 step 11, so no axis is swapped.) (iii) Two unit cubes in one GLB, the second under a node translated by 0.5 along X: `sample` is negative at both centres and in the overlap, and `inside_fraction` equals that of `overlapping_cubes()` (F6) within 2 %. (iv) Two cubes 20 units apart in one GLB behave like `corner_cubes()` in F9: no panic, `Ok`. |
| F25 | Real files of every format (files on this machine, ignored test) | One `.glb`, one `.gltf` and one `.fbx` model from the round-2 sweep list (`target\round2\jobs_wf.txt`; the baseline step writes the three chosen paths into `target\round4\perf\baseline.txt`, they were not picked for this document): each bakes to `Ok` or `NoInterior` without a panic within 10 s, and the log names the result. |

Topology (`topology.rs`):

| # | Test | Passes when |
|---|---|---|
| T1 | Cube | `seam_cube(1.0)`: 8 welded vertices; the lists hold 36 entries in total; every list is ascending. |
| T2 | Quad | `flat_quad`: 4 welded vertices. |
| T3 | Same result | Built twice: identical. The parallel build equals the scaffold's serial build on a mesh of at least 65,536 triangles. |
| T4 | Lazy | `Document::sample(Cube)`: `topology_built()` is false after a `SameSide` stroke and true after a `Connected` stroke. |

### 8.2 App unit tests (headless)

| # | Test | Passes when |
|---|---|---|
| A1 | Settings to tip | The seven rows of the table in 4.3, with `through` passed on; a 2D stroke never gets a field. |
| A2 | Types through the tool | Cube sample, Size 1024, hardness 1 (radius 2.18 units; the farthest corner of the cube is 1.84 from a face centre, so the ball holds the whole cube), one press and release at the centre of the front face: with Circular exactly one of the six UV islands is unchanged; with Sphere none is. |
| A3 | Connected through the tool | TwoMaterial sample, Size set so that the radius is 0.5 units, click on the head: with the checkbox on the body's set hash is unchanged; with it off both sets changed. |
| A4 | Pacing and camera moves do not change pixels | A path with a right-angle corner, walked with a raycast budget of 1 per frame, gives the same composite hash as with an unlimited budget; a clock that makes every frame stop after one sample gives the same hash as no time rule. A wheel zoom (and, separately, an orbit of the camera) applied between two `drag` calls while positions are still unresolved under a 1-raycast budget gives the same hash as the same drags with an unlimited budget, where everything was resolved before the camera moved. The existing test `long_drag_respects_the_raycast_budget_and_finishes_on_release` passes with the one added line of 5.10. |
| A5 | The time rule, with the replaceable clock | (i) 1 ms per sample: the stroke is closed in the frame of the release, with one history entry. (ii) 5 ms per sample, 2 samples arriving per frame: no frame does more than 10 ms plus one sample, and the backlog estimate never exceeds 250 ms. (iii) 20 ms per sample, 4 samples arriving per frame for 40 frames (the drag outruns the machine): no frame, the release frame included, does more than 100 ms plus one sample; after the release the controller stays busy, `end_frame` returns `Some(0.0)` in every frame until the stroke is closed, the stroke is closed within (samples waiting at the release x 20 ms / 100 ms) + 2 frames, and the composite hash equals the hash of the same path painted without a time rule. (iv) at least one sample is painted in every frame with waiting work. |
| A6 | Wheel | Ctrl+wheel multiplies `size_px` by 1.1 per notch within 1..2048 and returns true; Shift+wheel with Sphere and the Cube adds 5 to `turn_deg`; Shift+wheel with Circular, or with Sphere and the Ball, returns false. |
| A7 | A stroke uses the tip it started with | Changing type and angles during a drag gives the same pixels as not changing them. |
| A8 | Ring | During a 3D stroke `brush_ring` returns the stroke's radius after `size_px` was changed; `side_normal` is `Some` for Circular and `None` for Sphere; with Offset 0.5 the centre is half a radius above the hit along the viewer-facing normal. |
| A9 | Missing and loading shape | Shape = File with a load pending and no current shape: `press` returns false, no stroke is open, one info toast. With a current shape and a second load pending: the stroke uses the current shape. Shape = File with nothing loaded and nothing pending (state set by hand): paints as the Ball; the info toast appears at the first press only, not at the second. |
| A10 | Preferences | `prefs::to_json` then `from_json` keeps type, checkbox, shape choice, Use, angles, offset, align and the path; a text without the new keys loads with default brushes and keeps its recent files; a text with `"kind": "spiral"` loads with the default brush and keeps its recent files; a text with `"size_px": 10000000`, `"hardness": -3`, `"turn_deg": 900` and `"offset": 7` loads with 2048, 0, 180 and 1; a text with an extra unknown key loads; `save_to` then `load_from` on a path under `target\wte-test-tmp` round-trips. `sync_prefs` after changing `tools.brush.size_px` makes `state.prefs.brush.size_px` equal to it, so the write in `remember_recent` holds the current value. |
| A11 | Panels idle and width | For every tool, both tabs and every state of 6.3, at 1280 x 800 and 900 x 600: three frames queue no action and the side panel is 300 points wide. |
| A12 | Heights at 1280 x 800 | With the Brush tool: the Color section lies inside the side panel; the bottom of the fifth slider row is above the bottom of the tool area; with the Brush tab in its tallest state every row lies inside the bottom area. |
| A13 | Controls (real pointer events on the widget tree) | A click on "Brush" shows the tab; "Sphere" changes `kind` of the brush and leaves the eraser alone, and the other way round with the Eraser; the checkbox, the Shape list, Use, Turn, Tilt, Roll, Offset and Align each change exactly their own setting; "Load..." queues exactly one `LoadBrushShape(None)`; the file entry of a remembered, not loaded shape queues exactly one `ReloadBrushShape`; "Remove file shape" queues exactly one `RemoveBrushShape`; Use is disabled for a flat outline; Reset zeroes the angles and the offset; a drag of 20 px to the right in the preview adds 10 degrees to Turn; choosing the Stamp tool shows the Layers list (`side_rects.brush_tab` is `None`) and leaves `side_tab` on Brush, and choosing the Brush again shows the Brush tab without a click; Space and Enter after a click on a Type button change nothing. The three existing tests are adapted: the checkbox parts of `panel_tests.rs:358-386` and `1015-1036` go, and the "Layers" header clicks of `967-985` become tab clicks. |
| A14 | Uniform packing | The side normal is written into `brush_aux.yzw`, zeros without one; the existing size and offset test passes unchanged. |
| A15 | GPU, ignored without a GPU | The ring image at a cube corner with a side normal differs from the one without. |
| A16 | Load flow: every row of the table in 4.4 | "Load..." of an OBJ of `seam_cube` with the Brush active: `current` set, path in prefs, brush is Sphere / File / Solid, eraser unchanged, tab Brush; the same with the Eraser active changes the eraser only. "Load..." of the same cube as a `.glb`, a `.gltf` and an ASCII `.fbx` file (the writers of section 8): each ends in the same state as the OBJ, with its own path in prefs. "Load..." of `flat_star()`: `through` is true. "Load..." of a missing file and of `tilted_quad()` while a shape is current: an error toast (the `NoInterior` sentence for the quad), `current` and the prefs path unchanged, `last_error` set. Cancel: `current` kept. Quiet reload of an existing path: `current` set, no toast, tab and settings unchanged. Quiet reload of a missing path with the brush on Sphere / File: one warning toast, no error toast, brush shape back to Ball, prefs path unchanged; a press afterwards shows no toast. Start-up check: with both tools on Circular and a path in prefs no job is started; with the brush on Sphere / File one `ReloadBrushShape` is pushed. Remove: `current`, `last_error` and the prefs path cleared, tools on File back to Ball. |
| A17 | A waiting stroke gets its frames | With queued work and no panel drawn, `ToolController::end_frame` returns `Some(0.0)`; it returns `None` (or the Transform tool's own wait) once both queues are empty. A drag with a still pointer and no `drag` call in a frame still advances the stroke through `end_frame`. |
| A18 | The shape cursor exists | Cube sample, Sphere with the Cube shape, hover at the centre of the front face: `cursor_segments` returns a non-empty footprint whose bounding box, mapped back to world units, is a square with a half side of 0.577 x `k` within 2 %, and non-empty wires. With Use = Through the footprint is the outline of the middle slice (the same square). With Circular both lists are empty. During a stroke the lists follow `stroke_tip_pose()`. |
| A19 | A stroke that finishes after the release is never lost by accident | With the clock of A5 (iii), after the release and while the controller is busy: (a) `SetTool(Eyedropper)` through dispatch: exactly one history entry, composite hash equal to the unpaced stroke; (b) `Undo` through dispatch: no history entry, the pre-stroke hash, nothing else undone; (c) Esc: the same as (b); (d) a press in the view: the first stroke is one history entry with the unpaced hash and a second stroke is open; (e) `Save` while the button is still held with unresolved positions queued: the saved document's hash equals a stroke released at the last pointer position; (f) repeated `release` calls and `drag` calls after the release add no sample. |
| A20 | Two shape jobs | A quiet reload is pending and a "Load..." is started: delivering the reload's result first and the user's second, and the other way round, both end with the user's shape as `current` and the user's path in prefs; the dropped result shows no toast. A result that arrives after "Remove file shape" is dropped. |

### 8.3 Self-test scenarios (four new names; the count test at `app\selftest\scenarios.rs:3063` becomes the round-3 count plus four)

The built-in Cube document has islands of 277.3 x 448 texels for a 1.5-unit face (3.7), so expectations there are
not square in texels.

| # | Scenario | Steps | Passes when |
|---|---|---|---|
| S1 | `brush_types` | Cube sample, camera preset Front. Setup: Size 1024, hardness 1.0. Click the centre of the view with Circular. Undo. Setup: Sphere. Click again. Then TwoMaterial sample: Setup: Circular with "Stay on connected surface", Size so that the radius is 0.5 units; click the screen point of the world point (0, 0.8, 0.35); undo; checkbox off; click again. Screenshot. | After the Circular click: island -Z has 0 changed texels and each of the other five has more than 100,000. After Undo: the composite hash is the one before the click. After the Sphere click: all six have more than 100,000. TwoMaterial with the checkbox: the body's set hash is unchanged and the head's changed; without it: both changed. One history entry per click. No error toast. |
| S2 | `brush_shape` | Cube sample, camera preset Front, `SetTool(Brush)`. Setup writes `seam_cube(1.0)` as `<out>_assets\cube_shape.obj` with `samples::to_obj`. `LoadBrushShape(Some(path))`, wait until idle. Setup: Size 200, hardness 1.0 (radius 0.4256 units; the shape's footprint on the face is a square of 0.4914 units). Click the centre of the view. Undo. Setup: Turn 45. Click again. Then a 40-point stroke. `Step::Hover` onto the model, Brush tab open, screenshot. Then Setup: Turn 0, and the same cube written as `<out>_assets\cube_shape.glb` (`to_glb`) and as `cube_shape.fbx` (`to_fbx_ascii`); for each of the two files: `LoadBrushShape(Some(path))`, wait until idle, one click at the centre of the view, Undo. | After the load: `brush_shape.current` holds 12 triangles and `report.partial` is false; by the rule of 4.3 the brush is Sphere with shape File and Use Solid, the eraser is unchanged; the Brush tab is shown (`side_rects.brush_tab` is `Some`). First click: only island +Z changed; changed texels = 13,333 within 5 % (a ball of the same Size would change about 31,400); bounding box 90.9 by 146.8 texels within 4. Undo: hash restored. Second click: count within 5 % of 13,333, bounding box 128.5 by 207.6 within 4. The stroke: exactly one more history entry. The GLB click and the FBX click each change only island +Z, with a texel count within 5 % of 13,333 and the bounding box of the first OBJ click within 4 texels; after each of the two loads the prefs path is the new file. No error toast. |
| S3 | `brush_tab` | 1280 x 800. Sphere sample, Brush tool, Setup: `side_tab = Brush`, Sphere, Cube, Turn 30. `Step::Hover` onto the model (no press, no wheel). Assert. Screenshot. `SetTool(Text)`. Assert. `SetTool(Brush)`. Assert. | With the Brush tab: the three assertions of A12 hold on the real window, and the side panel is 300 px wide; `cursor_segments` (A18) has a non-empty footprint and wires in this frame. After `SetTool(Text)`: `side_rects.brush_tab` is `None`, the Layers footer is on screen, and `side_tab` is still Brush. After `SetTool(Brush)`: `side_rects.brush_tab` is `Some` again. The screenshot shows the tab, the preview, the footprint outline and the wires; it is for the report, the assertions do not depend on a person looking at it. |
| S4 | `brush_types_path` (needs a model path, like `workflow_path`) | Open the model. Find a stroke path with the helpers `workflow_path` uses. One warm-up stroke, Undo. Then the same path with Circular, Sphere, and Sphere with the Cube, each for Size 50 and Size 512, with a log line naming type and size before each and Undo after each. | Each stroke changes pixels; each Undo restores the hash. The runner already logs the duration of every pointer step and the frame profile; these are the numbers of 8.4. |

Failure cases of loading are unit tests (A16, F7), not scenarios, because an error toast fails any scenario.

Every pointer step ends only when its stroke is complete (7.3, the runner rule), so an Undo that follows a click
always meets a finished stroke and undoes its history entry, in the old scenarios as in the new ones.

### 8.4 Performance: how it is measured and the limits

Machine: the one of the round-3 audit (12 logical processors), release build, one run at a time, 1280 x 800.

**Step 0, before any engine package starts.** On the merged round-3 tree:

1. `workflow_path` on `E:\claude_downloads\glb_samples\ABeautifulGame.glb`,
   `target\compat\topologies\models\tex_4096\tex_4096.obj`, `...\tex_8192\tex_8192.obj`, `...\mesh_1m\mesh_1m.glb`
   and `...\mesh_3m\mesh_3m.glb`: frame profiles (with the frame interval, which also shows whether frames wait for
   the display refresh) and pointer-step durations.
2. One instrumented throw-away copy of the tree (under `target\round4\perf\`, never merged) that splits
   `pointer/tools/hover` into raycasts, sphere queries, per-triangle setup, rasterizing, commit, re-apply and
   first-use allocation, and counts the walk steps per frame, run on the same five models.
3. A new ignored test `stroke_bench_tips` (`stroke.rs`, pattern of `stroke_bench_ngons_tiling`, `stroke.rs:936-937`,
   using the counters of `brush.rs:767-796`): 200 dabs along a fixed path on (a) `tex_8192.obj`, (b)
   `ABeautifulGame.glb` and (c) `tex_4096.obj`, diameters 50, 512 and 2048 texels, today's brush, after one warm-up stroke; milliseconds
   per dab per phase.

The output is saved as `target\round4\perf\baseline.txt`. "B" below is this baseline. The gate then writes the
absolute target for model (b) into its plan: if step 2 shows that sphere queries are at least 30 % of
`pointer/tools/hover`, P3 requires at least a 15 % reduction; otherwise P3 requires "not slower", and the report
says in plain words that this model stays at its present speed and why. From the measured cost per walk step the
gate also writes down the expected trail for model (b) in seconds for a stroke of the audit's speed (5.10 gives the
estimate made without that number: about 0.25 s).

After the feature, `stroke_bench_tips` also runs the tips Ball/`Connected`, Ball/`All`, Field(Cube) and
Field(Cube, Through). The brush tool writes one log line per stroke: samples, stroke work in ms, the longest frame
of stroke work, the longest single sample, the largest backlog estimate, and the time from the release to the end
of the stroke. P8 reads these lines.

| # | What | Limit |
|---|---|---|
| P1 | Ball/`SameSide` after the change, models (a) and (c), all three sizes: total per dab | at most 1.05 x B |
| P2 | Ball/`SameSide`, model (b), 50 texels: query phase per dab | at most 0.35 x B |
| P3 | Ball/`SameSide`, model (b), 50 texels: total per dab | at most 1.00 x B; at most 0.85 x B if step 0 found queries to be at least 30 % |
| P4 | Ball/`Connected` against Ball/`SameSide` in the same run, both models, all sizes: total per dab | at most 1.15 x |
| P5 | Field(Cube) against Ball/`All` in the same run, model (a): total per dab, hardness 1.0 and 0.8 | at most 1.5 x at 50 texels, at most 3.0 x at 512 and 2048 texels (3.5 x at hardness 0.8, which adds the thickness lookup in the fade zone) |
| P6 | Field(Cube), model (a): total per dab | at most 6 ms at 512 texels, at most 60 ms at 2048 texels |
| P7 | S4 on model (b), Size 50: stroke work per stroke (the tool's log line) | Circular at most 1.10 x the stroke work of the same path in B, counted per walk step, because the new walk follows the whole path and B's cuts corners (3.3). Reported, with the step counts of both. |
| P8a | S4 on models (a), (b) and (c), every stroke: the longest frame of stroke work, **the release frame and the frames after it included** | at most 100 ms plus the longest single sample of that stroke, in milliseconds, not as a ratio. A stroke whose longest single sample is itself over 100 ms is listed under P12. |
| P8b | S4, every stroke: time from the release to the end of the stroke | Size 50: at most 400 ms on (a), (b) and (c). Size 512: at most 1,000 ms. Size 2048: reported. |
| P8c | S4 on models (a), (b) and (c): duration of each pointer step (press to stroke complete) | Circular: at most 1.10 x the same step in B. Sphere: at most 1.3 x Circular of the same run. Cube shape: at most 1.6 x Circular of the same run at Size 50 and at most 3.0 x at Size 512. |
| P8d | Largest backlog estimate during the drags of S4 at Size 50 | at most 350 ms on (a), (b) and (c) (the 250 ms target plus one frame) |
| P9 | The first connected press on model (b): that frame minus an ordinary press frame; memory after it | at most 0.4 s; at most 70 MiB above B. "Creating document" for model (b) is unchanged: at most 1.05 x B (0.97 s today). |
| P10 | `LoadBrushShape` to ready (the job's own log line, `app\jobs.rs:122`): the cube of S2; `target\compat\harness\shapes\high_poly_sphere.obj` (500,000 faces: its `f` lines were counted for the completeness check on 2026-11-17); the cube-on-a-floor file of F16 (two passes) | at most 1.0 s; at most 8 s; at most 2.0 s. Cancel ends the job within 1 s. |
| P11 | Hover with a shape (S2, frame profile): "overlays" plus "side panel" | at most 1 ms above their values without a shape; "side panel" with the Brush tab idle at most 1.5 ms (0.9 to 1.1 ms today, log lines of 3.8) |
| P12 | Frames longer than 100 ms during the strokes of S4 on model (b) | each one is listed in the report with what the profile shows; no limit (5.12, last paragraph) |
| P13 | Absolute speed on the 1.5-million-triangle model, so that a miss shows as a number and not only as a ratio. S4 on model (b), Size 50, the drag frames of the Circular stroke and of the Sphere stroke (frame interval from the frame profile; the frames after the release are not counted) | Circular: the median frame is at most the baseline median of the same path with today's brush ("Same side only" on). Sphere: at most the baseline median of the same path with "Same side only" off. Step 0 writes both baseline medians into `baseline.txt` in milliseconds (the audit saw 35 to 59 ms per frame, not re-measured), and the report prints limit and measured value next to each other. The same two medians on models (a) and (c): at most 16.7 ms, or at most the baseline median where that is already higher. |

The self-test feeds pointer positions per frame, not per second (`selftest\mod.rs:236-244, 566-568`), so in S4 the
frames stretch and the backlog stays near its target. The case "the user draws faster than the machine paints" is
therefore tested with the replaceable clock in unit test A5, not in a scenario.

How the budget of the brief ("strokes must stay interactive on 4096 and 8192 textures and on models of 1.5
million triangles") is covered, said plainly: the window stays responsive through pacing (P8a, P8d: no frame does
more than 100 ms of stroke work), and the speed itself is held by P1 (4096 and 8192 textures, per dab), P3 and
P13 (the large model, per dab and per frame). On the large model the limits promise "not slower than today", not
"fast": if step 0 shows that the time is not spent in the queries, the single query cannot speed it up, and the
report says so with the numbers of P13 and P12.

A limit that is missed is reported with the measured figure; it is not waved through. For P5 and P6 the first
fallback is a grid of 96 instead of 128 nodes per axis (2.4 times fewer values); it changes one constant.

### 8.5 Regression

The full existing test suite, clippy, the built-in scenarios and the model sweep of round 3 pass. The existing
scenarios use the default type, Circular, which gives today's pixels for the same samples, so their results must
not change, with one stated exception: a drag that today runs out of its 64 raycasts in a frame now follows the
whole pointer path (3.3), so a scenario that asserts an exact hash after such a drag gets its expected value
re-recorded, and the gate report lists each such scenario.

The project format is checked by a unit test, not by comparing two app runs, because every manifest carries the
save time and the program version (`core\project\save.rs:373-380`; the app passes the wall clock and the package
version, `app\io_flows.rs:406-407`): the same document, painted with the same samples once through `stroke_begin`
and once through `stroke_begin_with(.., &BrushTip::legacy(true))`, saved with the same `SaveOptions` (`now_unix`
and `app_version` fixed, as the core tests do at `save.rs:687-689`), gives byte-identical files, and the manifest of both
has `min_reader_version` 1 (the document has no text, so the rule of the text design, `FEATURE_TEXT.md` 4.3
item 2, asks for no newer reader). The test does not assert the value of `FORMAT_VERSION`: the text feature of the
same round raises it from 1 to 2, while the brush adds nothing to the file. The test is written by package E
(core-paint) as `strokes_save_identically_through_both_entry_points` in `crates\wte_core\tests\brush_tips.rs`,
the file that package owns; it needs no file of the project packages.

---

## 9. Risks, and what is deliberately left out

### 9.1 Risks

| # | Risk | Effect | Answer in this design |
|---|---|---|---|
| 1 | Nobody has measured where today's 35 to 59 ms per frame on the 1.5-million-triangle model go. | The single query may not help there. | Step 0 measures before any engine work; P3 is set from it; the report tells the user plainly if the model stays at 13 to 20 frames per second. |
| 2 | The pixel budget of the side panel after round 3 is not known (about 130 to 146 px for tool options today). | The Spacing slider or rows of the Brush tab could be hidden at 1280 x 800. | The measure-then-decide rule of 6.1, test A12 and scenario S3. |
| 3 | The Brush tab hides the Layers list. | The user cannot see the layers while tuning the tip. | The tab names the active layer; the Layers list is shown whenever another tool is active; Q5. |
| 4 | Type is in the Brush tab only. | Two clicks more to switch the type. | Status bar hint and a different cursor per type; a Type row in Tool options does not fit (6.1). |
| 5 | Models with openings are refused; closed parts of a file with open parts are used alone. | A cup or a head open at the neck cannot be used as it is. A character with an open neck and closed buttons becomes a brush made of its buttons. | A message that names the cases and what to change; the warning "Only the closed parts ... are used" and the "!" marker; the preview and the cursor show the result before painting; Q3 offers automatic closing as later work. |
| 6 | A model with a hole of about one cell or smaller may count as closed in one bake and as open after a small change of the file. | Surprising refusal or acceptance. | The rule is "what cannot be reached at grid resolution"; the preview shows the result before painting. |
| 7 | Grid resolution: corners rounded by up to 0.8 % of Size, parts thinner than 1.2 % of Size vanish. | A cube's corners look slightly soft above about 1000 texels. | `report.thin` and its warning; a finer grid is one constant (8 times the memory per doubling). |
| 8 | The footprint outline is drawn on the tangent plane at the cursor. | On strongly curved surfaces it differs from the real cut. | The ring is always the true outer limit; the wires show the orientation; the exact cut line is a later option (9.2). |
| 9 | The ring's side filter uses the smooth vertex normal, the paint uses each triangle's flat normal. | Ring and paint can differ by one row of triangles at the cut-off. | Accepted for a cursor. |
| 10 | A drag that outruns the machine leaves work for after the release. | The stroke is completed in the following frames (each at most 100 ms plus one sample); until then the next click or command first completes it, which can take as long as the backlog. | 5.10: the trail is held near 250 ms while the work arrives slower than real time; the ring shows where the paint is; the status bar says the stroke is finishing; Undo and Esc discard it; P8a to P8d and A5 measure it. |
| 11 | The neighbour table is built in the frame of the first connected press. | One frame of 0.1 to 0.3 s on a 1.5-million-triangle model (estimate). | P9; fallback: a background job when the checkbox is ticked. |
| 12 | A connected Circular brush on models made of loose pieces paints one piece per dab. | Looks broken to someone who expects today's behaviour. | The checkbox is off by default; its hover text says what it does. |
| 13 | Thin shapes and Spacing. | A rod dragged sideways at 12 % leaves stripes. | Said in the Spacing hover text; the user lowers Spacing (down to 2 %). |
| 14 | The shape's centre is the centre of its bounding box. | For an L-shaped or ring-shaped solid the cursor point itself may stay unpainted. | The footprint outline shows where paint goes; Offset moves the slice. |
| 15 | A moved or deleted shape file. | The brush falls back to the ball at the next start. | Warning toast; Q4. |
| 16 | A set inside the shape's box but outside the shape counts as reached. | Up to 192 MiB for an 8192 x 8192 set that receives no paint, and a "Paint skipped" warning if its layer is locked. | Much rarer than with the bounding ball (5.4 step 6, C24); the same mechanism as today at the rim of the ball. |
| 17 | Round-3 merge: line numbers and the side panel's structure will differ from what is cited here. | The scaffold and the panel package must re-read the files they touch. | The design depends on one layout fact only (7.1). |
| 18 | Shared files with the text feature. | Merge conflicts. | One scaffold and one gate for both features; own flow code in `brush_flows.rs` (7.1). |
| 19 | A thin shape used Solid paints only where the surface passes through it. | A star plate 5 % thick prints a blob on a ball of 2.5 times its radius. | Through (5.6) is chosen at load for flat and thin shapes and says so; the Use control and its hover texts; table in 1.7; Q8. |
| 20 | Through paints all surfaces inside the ring's ball, far sides included, like every Sphere tip. | On a thin wall the outline also appears on the back. | Stated in the hover text; a smaller Size or the Circular type avoids it. A Through tip limited to the near side is left out (9.2). |
| 21 | A file with unused parts is baked twice, and a closed part thinner than one cell of the first pass (0.82 % of the file's longest side) can be left out. | Up to twice the bake time for such files; a thin fin may be missing. | P10 has a limit for the two-pass case; `report.partial` and its warning; F16, F19. |
| 22 | Local thickness changes quickly where a thin part meets a thick one. | With a soft brush the fade is not uniform across such a junction (it follows each part's own thickness). | The lookup is blended over one cell, so there is no step; C23 and F20 pin the values; hardness 100 % is unaffected. |
| 23 | The time rule reads the wall clock. | How many frames a heavy stroke takes differs between runs. | The pixels do not (A4, A5); scenarios wait for idle tools; unit tests use the replaceable clock. |
| 24 | A flat sheet that is tilted in its file is not recognized as an outline. | It is refused. | The message says to lay it flat in a main plane or give it thickness; recognizing tilted sheets is left out (9.2). |

### 9.2 Deliberately left out

- Turning the shape with the stroke direction; random rotation per dab; a ball that rolls.
- Sphere and shape strokes steered from the 2D view. The 2D brush stays a flat disc for every type.
- Stretching a shape along single axes, and with it an ellipsoid brush.
- A shape library with names, thumbnails and Remove; copying shape files anywhere; a recent-files list; storing a
  shape in the project file.
- Closing openings of non-closed models (Q3). Outlines from flat sheets that are tilted against the file's main
  planes. Wrapping a flat shape onto the surface like a stamp (Q8). A Through tip that paints only the near side.
- Stopping a quiet shape reload that a newer load has replaced (its result is dropped; only modal jobs can be
  cancelled today).
- Thinning or coarsening a stroke's path when the machine cannot keep up (5.10, last paragraph).
- The exact line where the shape cuts the visible surface, drawn by the mesh shader. If it is wanted later: the
  field as a 2D atlas of slices (R8, 16 x 8 slices of 128 x 128, read with `textureSampleLevel`), which stays inside
  `ARCHITECTURE.md:3196`; the mesh pipeline then needs its own layout, because it shares one with the 2D texture
  pipeline today (`app\gpu\renderer.rs:381, 406, 413`), every view that uses it must bind a dummy atlas, and the
  wire outline of this design stays as the fallback. No 3D texture.
- A Circular brush limited to what the camera sees (projection painting). No visibility test is added to the dab.
- Handles in the 3D view to turn the shape.
- F6 or any new keyboard shortcut; a View menu entry for the tab.
- Dropping a model file on the Brush tab (a dropped model still opens as a new document).
- Pen pressure (it is the constant 1.0 today, `app\view\mod.rs:472`) and every "dynamics" setting of Photoshop's
  panel: jitter, scattering, dual brushes, texture, wet edges.
- A fix for the long first frames when a stroke first reaches a set; it is measured and reported only.
- A change to how the world radius is derived from Size (the set under the press point, frozen for the stroke).
- Image-based (2D picture) brush tips.

---

## 10. Review log

Three skeptics reviewed the first version of this document (lenses: code, data, user). Each finding was checked
against the source on 2026-11-17; "confirmed" means the cited lines say what the finding says. No finding was
rejected. Two were taken over in a changed form (marked).

| # | Lens, severity | Finding in short | Check | What was done |
|---|---|---|---|---|
| 1 | code, major | A stroke with waiting work gets no frames with a still pointer. | Confirmed: the repaint call sites are as listed (`jobs.rs:59, 125`, `panels\mod.rs:141-172`, `toasts.rs:140`, `view\mod.rs:48, 52`, `preview.rs:95`, `selftest\mod.rs:255`). | 5.10 "Frames": `ToolController::end_frame` paints and returns `Some(0.0)` while work waits; the existing code at `view\mod.rs:46-49` turns it into a repaint. Scaffold edit in 7.3, ARCHITECTURE 8.6 in 4.7, test A17. |
| 2 | code, major | The 12 ms budget defers work, the backlog rule lifts every bound, the release frame can take hundreds of ms on the 1.5-million-triangle model. | Confirmed for the mechanism (`shot.log` lines 92, 103, 114; cap at `tools\brush.rs:13`). The cost per step is a lower bound from averages, not a measurement. | 5.10 rewritten: 10 ms per frame while the stroke keeps up, up to 100 ms when more than 250 ms of work wait, never more; the rest is done after the release. Expected trail for that model stated in 5.10; absolute limits P8a, P8b, P8d; test A5. **Changed form:** the path is not thinned or coarsened, because that does not reduce the number of dabs (`stroke.rs:207`) and would make pixels depend on the clock; reason in 5.10. |
| 3 | code, major | Tip units and grid come from all vertices; open or loose geometry shrinks the brush silently. | Confirmed (`import\mod.rs:218-271` keeps everything; the old steps 2, 3, 6). | 5.7 steps 5 and 6: skin points, a second pass over the solid that was found, units from the solid, `report.partial`, warning in 6.6, worked example; tests F16, F19, limit in P10. **Changed form:** the test compares size, resolution and the printed square instead of the hash, because a coplanar floor can shift the zero level by a fraction of a cell at the contact edge. |
| 4 | code, major | Hardness is normalised by the largest depth of the whole shape, so thin parts are faded as a whole at the default 80 %. | Confirmed by the formula (`brush.rs:84-109`; default at `state.rs:169-171`). | 5.2: the fade is measured against the local thickness `T` (5.7 step 9, one more grid of 1 byte per node); table 1.4; tests C23, F20; risk 22. |
| 5 | code, minor | Queued screen positions are raycast later with another camera. | Confirmed (`view\mod.rs:420-430` zooms without a busy check; `tools\brush.rs:164`). | 5.10: every queued position carries its `ViewXform`; test A4 has a zoom and an orbit case. |
| 6 | code, minor | `i16` distances overflow beyond 128 cells. | Confirmed by arithmetic (32767 / 256 = 127.996; grid diagonal 221 cells). | 5.7 step 8: saturating; test F9 with `corner_cubes()`. |
| 7 | code, minor | The border ray's direction and "its own links" are not specified; an inner wall can enter the skin set. | Confirmed (`Bvh::raycast` returns the nearest hit from the origin, `bvh.rs:427-431`). | 5.7 step 5: cast from the outside node; step 7: border links only; test F5 with a slanted wall. |
| 8 | code, minor | A closed thin model is refused with a message that is false for it. | Confirmed by arithmetic (cell = 0.82 % of the longest side). | 5.7 step 2 and 5.7b: a thin model in a main plane becomes an outline instead of a refusal; the `NoInterior` text (6.6) now names thin, flat and open; test F17. |
| 9 | code, minor | `SetTool` setting `side_tab = Layers` is redundant and costs a click. | Confirmed (`dispatch.rs:631-635` is one arm). | 6.2: `SetTool` is not changed, only the display rule falls back to Layers; A13 and S3 assert what is shown. |
| 10 | data, major | `finish` has no view transform; a camera change moves queued positions. | Confirmed (`tools\mod.rs:198, 442-448`; `tools\brush.rs:259-264`). | Same change as 5: stored transforms, hits in the second queue; 5.10 says what `finish`, `cancel` and `discard` do; tests A4, A19 (e). |
| 11 | data, major | The life cycle of the loaded shape is specified in three contradicting ways and the enum cannot keep the previous shape. | Confirmed in the old text of 4.2 to 4.4, 6.3, 6.6, A16; `state.rs:3` forbids later edits. | 4.2: `BrushShapeState` with `current`, `pending`, `last_error`; 4.4: the prefs path is its own value, one event table; "Remove file shape" (1.4, 6.3); A16 follows the table. |
| 12 | data, minor | Two shape jobs can finish in either order. | Confirmed (`jobs.rs:96-134`, `io_flows.rs:460-467`, `jobs.rs:166-171`). | Request numbers (4.2, 4.4); test A20. The superseded quiet job is not stopped, its result is dropped (9.2). |
| 13 | data, minor | The shape is imported and baked at every start, also when unused. | Confirmed in the old 4.4. | 4.4: reload only when a tool has Sphere and File at start, otherwise when the entry is chosen; A16. |
| 14 | data, minor | `prefs.json` is also written at every open and save; brush values there would be stale. | Confirmed (`io_flows.rs:239-245`, `app.rs:316-322`, `color.rs:19`). | 3.6 corrected; 4.4: `sync_prefs` before both writes; scaffold list; A10. |
| 15 | data, minor | A `Prefs` struct literal in `prefs.rs` breaks after the scaffold. | Confirmed (`prefs.rs:106-115`; second one at `state.rs:805-814`). | 4.2 and 7.3: both get `..Prefs::default()` in the scaffold. |
| 16 | data, minor | Out-of-range brush values from the file are taken as they are. | Confirmed (`stroke.rs:88-97` clamps the radius only at 0; `prefs.rs:80-88` shows the existing pattern). Whether the sliders clamp a stored value before the first stroke: not verified, and no longer needed. | 4.2 `BrushSettings::sanitized`, 4.5; test A10. |
| 17 | data, minor | "Byte-identical `.wtep`" cannot be tested as written. | Confirmed (`save.rs:373-380`, `io_flows.rs:406-407`). | 8.5: a unit test with fixed `SaveOptions`. |
| 18 | data, minor | A set that only the bounding ball reaches is marked skipped and warned about. | Confirmed (`stroke.rs:343-354, 377-392`; `tools\brush.rs:204-211`). | 5.4 step 6: a field tip reaches a set only when a candidate survives the shape's box test; 5.13; test C24; risk 16. Whether the early raster allocation shows in saved files: not verified, stated in 5.13. |
| 19 | user, major | The release frame freezes for about a second on the heavy model; P8 cannot pass; the comparison with today's release was wrong. | Confirmed (today's release walks one segment, `tools\brush.rs:247-257`; the self-test feeds points per frame, `selftest\mod.rs:236-244`). | Same change as 2: finishing after the release, with the controller busy and a completed-not-cancelled rule for every command except Undo and Esc (5.10, 4.6, 7.3); P8 restated in milliseconds; A5, A19; risk 10 rewritten. |
| 20 | user, major | A flat or thin shape does not print its outline on a curved model; a flat polygon is refused; hardness barely acts on a plate. | Confirmed by the reviewer's calculation on `uv_sphere` (drop of 0.083 units at a radius of 0.4). | New Use control, Solid / Through (0, 1.2, 1.4, 5.6), chosen at load for flat and thin shapes; flat sheets in a main plane accepted as outlines (5.7b); shapes are laid flat (5.7 step 11); the outline has its own distance and thickness, so hardness acts on it (5.6); table in 1.7; Q8; tests C26, F7, F17, F18; risks 19, 20, 24. |
| 21 | user, minor | With default settings Circular and Sphere paint the same pixels. | Confirmed (radius 0.068 units on a 1.5-unit cube; `state.rs:170`). | Said in 1.1, 1.7, the Sphere hover text and the note line; Q1 extended; C5 has a case at 64 texels. |
| 22 | user, minor | The tallest Brush tab does not fit the fallback height; notes wrap; the Shape row is too wide. | Confirmed on `paint3d_builtin\shot.png` (85 px for 14 characters; content about 283 px wide). | 6.1: computed bottom-area height instead of 37 %; notes of at most 44 characters with one wrap budgeted; 6.3: Shape list 140 px, all rows at most 276 px. |
| 23 | user, minor | S2 expects type and shape to change after a load, but no rule says so; S2 never selects the Brush. | Confirmed in the old text. | 4.3: the rule (the issuing tool gets Sphere, File and Use); S2 has `SetTool(Brush)`; A16. |
| 24 | user, minor | A dab between two samples is seeded by distance and can land on another part. | Confirmed by the geometry of `samples.rs:143-162` and `stroke.rs:192, 218`. | 5.5: seeds from the hit triangles of the end samples and from the previous dab; `Hit::tri` (`bvh.rs:22-23`) is stored with the sample (5.9); test C25. |
| 25 | user, minor | The runner cannot hover without pressing; no test checks the outline. | Confirmed (`selftest\mod.rs:48-73, 553-596`). | `Step::Hover` (7.3, gate); `cursor_segments` and test A18 (5.11); S3 asserts through it. |
| 26 | user, minor | Three `BrushRing` literals break; `gpu\mod.rs` is in no list. | Confirmed (`gpu\mod.rs:248`, `renderer.rs:861`, `tools\brush.rs:299`). | 4.2, 7.2, 7.3. |
| 27 | user, minor | The slot state `Failed` is never entered; a tool left on File toasts at every press. | Confirmed in the old text. | Same change as 11: `last_error` replaces `Failed`; a failed quiet reload moves the tools back to the Ball once; the fallback toast is shown once per run (4.3); A9, A16. |

Findings 5 and 10, 2 and 19, 11 and 27 describe the same defect from two sides and share one change each.

New facts the review brought in and that the design now relies on, each read in the source: `ViewXform` is `Copy`
(`app\tools\mod.rs:51-52`); `ToolController::end_frame` exists and its result already requests a repaint
(`tools\mod.rs:325-327`, `view\mod.rs:46-49`); `ToolController::release` returns to Idle unconditionally
(`tools\mod.rs:409-415`); the interaction guard and its Undo special case (`dispatch.rs:33-43`); the runner ends an
input step when the injected frames are processed (`selftest\mod.rs:338-341`) and waits for idle tools only in
`WaitIdle` (`selftest\mod.rs:540-545`).

---

### Completeness check of 2026-11-17 (after the review)

A check of this document against the request found gaps in the tests, not in the design. What was added:

| Gap | What was done |
|---|---|
| Every test that loads a shape file used OBJ, although OBJ, glTF, GLB and FBX are promised. | Three file writers (section 8), tests F21 to F25, the other formats in A16 and in scenario S2. Node transforms, several meshes in one file and FBX units are now tested. |
| The byte-identical save test had no author and asserted an unchanged `FORMAT_VERSION`, which the text feature raises. | 8.5: written by package E in `brush_tips.rs`; it asserts identical bytes and `min_reader_version` 1. |
| Ctrl+wheel and Shift+wheel change meaning for the Brush and the Eraser without the user having asked. | 1.5 says so; new question Q9; named in the gate report. |
| `tex_4096.obj` was only in the baseline; the large model had only relative limits. | 8.4: model (c) in the bench and in P1 and P8; the new absolute limit P13. |
| C1 said "without an edit" although the text scaffold adds one line to `TextSpec` literals in two test files. | C1 names the exception. |

---

## Appendix A. What was done with the judges' findings

Checked and taken over as stated:

| Finding | Check | Where |
|---|---|---|
| Circular must stay today's brush until the user says otherwise; today's behaviour must not be reachable only as "Sphere plus a checkbox" | `brush.rs:58, 227-229` | 0, 1.1, 4.3, Q1 |
| Do not charge every document for the connected mode | `doc_build.rs:280-293` | 4.1, 5.5 (built at first use), P9 |
| Signed distance to the border of the inside region, no jump | design argument | 5.7 steps 7 to 9, F5, F6, F9 |
| Orient with the viewer-facing normal and the camera's up, as stamps do | `stamp_place.rs:177-178`, `bvh.rs:32-47` | 5.3, C15 |
| Keep `ARCHITECTURE.md:3196`; CPU outline first | `ARCHITECTURE.md:3196` | 5.11, 9.2 |
| Correct Cube-sample numbers (277.3 x 448 texel islands) | `samples.rs:104-110`, `doc_build.rs:378` | 3.7, S1, S2; `flat_quad` where a square is wanted (C10) |
| Widen a set only above 1.001 times its mean; "set reached" as today | `brush.rs:248, 373` | 5.4 |
| A time budget only together with the pointer queue; keep the existing budget test | `tools\brush.rs:155-163, 431-450`, `view\mod.rs:375-378` | 5.10, A4 |
| A per-frame time limit is allowed in the app (the lean proposal said it was not) | `ARCHITECTURE.md:419, 2078`, `state.rs:548-580` | 3.6, 5.10 |
| Baseline before engine work; limit total stroke time, not only the per-frame segment | log lines of 3.8 | 8.4 step 0, P7, P8 |
| One source of truth for the side switch | `stroke.rs:161-177` | 4.1, C3 |
| The bake must not depend on a parallel package's topology | design | 5.7 needs no topology; 7.3 |
| `random_brush` and `BrushTool::params` list every field | `end_to_end.rs:755-766`, `tools\brush.rs:61-70` | `BrushParams` is unchanged (4.1) |
| Replacing the "Layers" header breaks a control test | `panel_tests.rs:967-985` | A13 |
| Every compile break of `same_side_only` | `state.rs:164-170`, `tools\brush.rs:69`, `tool_options.rs:165-166`, `panel_tests.rs:372-382, 1025-1033` | 7.3, scaffold |
| No shape data under `%APPDATA%`; settings survive a restart | `ARCHITECTURE.md:3921-3923`, `prefs.rs:14-21` | 4.4 |
| No unbounded nearest-triangle search in the bake; triangle limit; Cancel | design | 5.7 steps 1, 4, 8; F12; P10 |
| Refuse open sheets in any orientation; a face exactly on a plane of nodes | `bvh.rs:376` | 5.7 steps 4 to 6; F7, F8 |
| Fast-path bound valid for trilinear lookup, tested on several poses | design | 5.6, C16 |
| Query and clip radius enlarged by one cell diagonal | `brush.rs:394-413` | 5.1 (`R_b`), C19 |
| The 2D view stays a flat disc, said in the tab at all times | `stroke.rs:433-448` | 1.6, 6.3 |
| Honest sizes (4,000 to 5,000 lines) | | 7.2: about 5,100 |
| The claim "Connected equals today's brush on a connected surface" must be limited | `brush.rs:227-229` | 5.5, 1.7 last row, C6 |

Taken over in a changed form, or not taken over, with the reason:

1. **"About 146 px remain for tool options" (all three judges).** In `runs\default_builtin\shot.png`, where the whole
   Color section is visible, it measures about 315 px, not 305 px, and the area above the Layers list is about
   447 px, so about 130 to 146 px remain. The 144 px of the brush options after this change may not fit. 6.1 adds
   the measure-then-decide rule.
2. **"Keep a Type row in Tool options" (implementer).** It needs 168 px in an area of about 130 to 146 px and would hide
   the Spacing slider, which contradicts the same judge's demand that the five sliders stay visible. Type is in the
   Brush tab only (6.1, risk 4).
3. **"Edge-inside-the-ball test" (architect) and "neighbours by shared welded vertex so T-junctions do not stop the
   flood" (user advocate).** Taken literally the two contradict each other at a T-junction whose end points lie
   outside the ball. 5.5 uses one rule that serves both: neighbours by welded vertex, crossing only where the two
   triangles touch inside the ball, including along a partly shared edge.
4. **"`Bvh::closest_point` inside `bvh.rs` for exact border distances" (implementer).** Not needed: a border node
   has a skin triangle within one cell, so the existing `query_sphere` with a radius of one cell and
   `closest_point_on_triangle` are enough (5.7 step 8). This also meets the user advocate's demand that no search is
   unbounded.
5. **"Bound the work done in the release frame" (architect).** First bounded by a count only. After the review of
   section 10 it is bounded by time: the stroke finishes in the frames after the release, and the reason that
   spoke against it (a tool change cancels a running stroke, `app\dispatch.rs:631-635`) is removed by completing a
   released stroke instead of cancelling it; see 5.10 and risk 10.
6. **"A loaded model is never refused for being open" (a strength the user advocate named) against "keep the plain
   refusal" (implementer) and the architect's objection to the six-direction rule.** The first build refuses; Q3
   asks the user.
7. **"Built-in Cube and Cylinder, Offset, draggable preview: drop or ask" (user advocate) against "take them over"
   (architect, implementer).** Kept, each with its own test (C10, C11, C14, A13), and put to the user as Q6.
8. **"The shared query gives exactly today's candidate lists" (architect, implementer).** True as long as the BVH's
   pruning is conservative, which its padded boxes are built for (`bvh.rs:328-329`); 5.4 states the condition and C2
   tests it.
