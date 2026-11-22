# Feature 2 — text stamps: fonts on the machine; warp, text along a line, angle, slant, bold, segments

Wrapped Texture Editor, round 4. Design only: nothing in this document is built. Written 2026-11-17.

This is the final design for Feature 2 of `target\round4_design\FEATURE_BRIEF.md`. It starts from the proposal the
three judges preferred (`work\text\proposal_user.md`), takes over the parts of the two other proposals the judges
asked for, and corrects the errors the judges found. Every judge finding was checked against the source; where a
finding led to a change, section 9.3 says which.

**Revised after a review** by three independent skeptics (code, data model, user tasks). Every finding was
checked against the source again; section 10 lists each one with what was changed. The larger changes: the Text
tool always edits the text to place and never a text placed before (6.2); a picked font is looked up in the
project before the machine (5.13); B and I read weight and slant from the font bytes of the text and are
resolved again when the family changes (5.9); the undo of the content field brings gap values back (5.12); a
change of spacing keeps the alignment side of the text in place (5.11); a new Circle is centred on its top
point for every alignment (5.5); and a styled picture has an area limit of 16 megapixels (5.10, 5.11).

**One thing is newer than the proposals and the verdicts.** The brief was extended on 2026-11-17 at 13:00 with a
decision of the user: "segmented means editing the space between each character". All three proposals had read
"segmented" as own formatting for parts of a text. This design follows the user: it builds a spacing value for
every single gap and does not design formatting for parts of a text (sections 2 and 9.2).

How to read the references:
- Paths are relative to `E:\claude_programs\wrapped_texture_editor\`. `core\` = `crates\wte_core\src\`,
  `app\` = `crates\wte_app\src\`, `ARCH:n` = line n of `ARCHITECTURE.md`,
  `REG\` = `E:\claude_downloads\rust-toolchain\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\`.
- Line numbers are those of the main tree on 2026-11-17. The main tree is still the round-2 result: round 3 is
  being built in copies under `target\round3\work\` and will shift lines in `doc_stamps.rs`, `stamp_bake.rs`,
  `project\load.rs`, `panels\*`, `tools\stamp_place.rs`, `tools\transform.rs` (section 7.4).
- Every `file:line` reference was read for this document. A fact taken from a proposal or a map without reading
  the source again is marked "(not re-read)".
- "pt" is an egui point; in the self-test at 1280x800 one point is one pixel (ARCH:3838-3839).
- "em" is the font size: the height of one text line box before line spacing. "Raster pixel" is a pixel of the
  picture the text is drawn into; "texel" is a pixel of a texture of the model.
- "Estimate" and "not verified" are meant literally. No build and no run of the app was allowed while this was
  written, so **no time in this document is measured**. Two things were run, both without compiling anything:
  an offline dependency resolution (section 5.1) and a Python check of the warp formulas (section 5.6).

## 0. The decisions on one page

1. **Where the controls live.** The Text options in the upper part of the side panel shrink from about 344 pt
   to four rows (content, font, one row with B / I / size / angle / colour, one row of three buttons). All other
   text controls live in a **Text tab** in the lower part of the side panel, next to Layers, with three pages:
   Character, Warp, Path. This is the tab mechanism the user asked for with the brush ("its own tab like in
   photoshop") and that the round-3 plan announces; one tab strip serves both features. The colour picker is not
   touched: it stays pinned between the two areas.
2. **Fonts on the machine** is already true in the code. What is missing is finding and judging a font: the two
   plain lists become a font picker with a search field, a sample of each font in its own face, every face of a
   family, and an order that ignores upper and lower case.
3. **Warped** = Photoshop's Warp Text: the same 15 style names, Bend, Horizontal and Vertical distortion, and the
   direction switch. The letter shapes bend. The formulas are our own; all 15 were checked numerically not to fold
   the text over itself.
4. **Curved along a line** = text on a line inside the flat picture of the stamp: an **Arc** (one Bend slider), a
   **Circle** of a fixed diameter, or a **Curve** through 2 to 16 points that the user drags in the view. Letters
   keep their shape and turn with the line.
5. **Angled** = the rotation of the whole text that exists today, now also shown as "Angle" in the Text options.
6. **Slanted** = a slant angle for the letters. **Bolded** = thicker letters. The B and I buttons take the real
   bold or italic face when the family has one and thicken or slant only when it has none. Both apply to the
   whole text.
7. **Segmented** = the space between any two neighbouring characters can be changed on its own (decided by the
   user). A text keeps a list of gap values, each a fraction of the text size, that add to Letter spacing and to
   the kerning of the font. The user picks a gap in a strip of the side panel and drags its value.
8. **Engine.** Two painters. A text without thickening, slant, warp or line is drawn by the `fontdue` code of
   today; gap spacing is one added term in its layout, and without gap values the result is today's, bit for
   bit. A text with one of the four is drawn from glyph outlines read with `ttf-parser` (the parser `fontdue`
   itself uses), positioned with the numbers of `fontdue`, bent by our formulas and filled by a small coverage
   rasterizer of our own. One new dependency line, no new package in `Cargo.lock`.
9. **The text stays where it is.** Every text picture reports an anchor point. One rule keeps the anchor and the
   size of one em fixed on the model whenever the picture changes: on an edit, on a resolution change and when the
   resolution has to drop because the picture got too large. A change of spacing alone (a gap value or Letter
   spacing) keeps the alignment side of the text fixed instead: the left edge of a left-aligned text, the centre
   of a centred one, the right edge of a right-aligned one.
10. **Bounded cost while typing and dragging.** A styled text is rasterized at no more than 1 megapixel while a
    control is live, at full resolution once at the end. The picture covers exactly the same rectangle in both
    cases, so nothing moves when the final picture replaces the preview. A styled picture has at most 16
    megapixels; a text that would need more is drawn with fewer pixels per em.
11. **File format.** A text entry gets one optional field `fx`. `FORMAT_VERSION` becomes 2 with an empty
    migration step. `min_reader_version` is 2 only when a text uses a new feature, so a project without one stays
    readable by the round-3 build.
12. **Before any package starts:** the open questions of section 2 go to the user; the plan is rebased on the
    finished round-3 tree; a real offline build with the new dependency line is proven; hashes and timings of
    the plain text of today are recorded (section 7.5).

## 1. What the user gets

### 1.1 The workflow

| Step | The user does | The app does |
|---|---|---|
| 1 | Presses X or clicks the T button. | The side panel shows "Text options": the text to place (default "Text"), font, style, B, I, Text size, Angle, Colour and the buttons Warp, Path, More. |
| 2 | Moves the pointer over the model or the texture. | A see-through picture of the text follows the pointer inside the yellow frame that exists today. Ctrl+wheel changes the size, Shift+wheel the angle (both exist today). |
| 3 | Clicks. | The text becomes its own layer (undo entry "Add Text"), is selected, and the tool switches to Transform, as today. Under the Transform tool the same four rows now show the placed text. |
| 3b | Presses X again to place a second text. | The four rows show the text to place again, not the text placed before. Below them the line "Selected: WTE 2026 [Edit]" of today leads back to the placed text. What is typed now changes only the text that the next click places. |
| 4 | Types in the content field. | The text on the model follows every key. Leaving the field is one undo entry "Edit Stamp". |
| 5 | Clicks the font button and types "ari". | The list shows the families whose name contains "ari", each with a sample in its own face. Enter or a click applies the font. |
| 6 | Clicks **B**. | The whole text becomes bold: the bold face of the family if it has one, else thickened letters. |
| 7 | Clicks "More". In the gap strip of "WTE 2026", clicks the gap between "E" and the space and drags the value to 30 %. | The lower part of the side panel switches to the Text tab, page Character. Only this one gap gets wider. The text is left-aligned, so "WTE" stays where it is on the model and the letters after the gap move to the right, live. (Right-aligned: the letters after the gap stay and "WTE" moves left. Centred: both halves move apart by half the value.) Releasing is one undo entry. |
| 8 | Clicks "Warp". | The Text tab shows the Warp page. A click on the "Arc" picture bends the text at 50 % at once. Dragging Bend changes it live; releasing the slider is one undo entry. |
| 9 | Clicks "Path", then "Curve". | Round points appear on the baseline of the text in the view. Dragging one bends the line; the letters follow live. Releasing is one undo entry. |
| 10 | Drags the gizmo, or types an Angle. | Moves, scales, rotates as today. |
| 11 | Saves, reopens, exports. | The text is still text with every setting changeable. Export writes the baked pixels as today. |

### 1.2 Controls and defaults

Every control acts on the whole text, except the gap controls, which act on one gap.

**Text options (upper part of the side panel).** The same four rows serve the text the Text tool will place and a
selected placed text. Which one they show follows the tool, as today: **with the Text tool they always show the
text to place; with the Transform tool they show the selected text** (section 6.2).

| Control | What it does | Range | Default |
|---|---|---|---|
| Content field (2 rows) | The text. Enter starts a new line. | free text | "Text" |
| Font button | Opens the font picker (section 6.3). Shows the family name. | all listed families | Ubuntu |
| Style list | Every face of the family (Regular, Bold, Italic, ...). | faces of the family | Light (the only bundled Ubuntu face) |
| **B** | Bold: the bold face of the family if it has one, else thickened letters (section 5.9). | on / off | off |
| **I** | Italic: the italic face of the family if it has one, else letters slanted by 12 degrees. | on / off | off |
| Text size | Height of one text line on the model, in % of the model size (2D: of the texture height). Ctrl+wheel in the view. | 0.2 to 400 % | 6 % |
| Angle | Turns the whole text. Shift+wheel in the view, or the round knob of the gizmo. This is the rotation of today. | -180 to 180 degrees | 0 |
| Colour button | Text colour, with transparency. | any | black, opaque |
| Warp button | Opens the Text tab on the Warp page. Shows the current style ("Warp: Arc"). | — | "Warp: None" |
| Path button | Opens the Text tab on the Path page. Shows the current kind ("Path: Circle"). | — | "Path: Straight" |
| More button | Opens the Text tab on the Character page. | — | — |
| Project through | Exists today. Shown below the four rows, only before placing. | on / off | off |

**Text tab, page Character.**

| Control | What it does | Range | Default |
|---|---|---|---|
| Align | Left / Center / Right for texts of several lines; on a line: where the text sits on it (Arc and Circle: Left starts at the left end of the arc or at 9 o'clock, Right ends at the right end or at 3 o'clock, Center is centred on the top point). Exists today. | 3 buttons | Left |
| Line spacing | Distance between lines. Exists today. | 0.5 to 3.0 x | 1.00 x |
| Letter spacing | Extra space after every letter of the text, in % of the text size. Exists today. | -20 to 100 % | 0 % |
| Thicken | How much the letters are thickened: every edge grows by this share of the text size. | 0 to 8 % | 0 (2 % when B has to thicken) |
| Slant | Leans every letter; the baseline stays where it is. | -45 to 45 degrees | 0 (12 when I has to slant) |
| Outline | On / off, width (0 to 25 % of the text size), colour. Exists today. | | off, 6 %, white |
| **Gap strip** | Shows the characters of one text line as small boxes with a slot between every two neighbours. A click on a slot selects that gap. A gap with its own value is marked. A value that is stored on the last character of a line (it has no effect there) is shown as a dashed slot after the last box, so it can be seen and reset. | one slot per gap | no gap selected |
| Line buttons `<` `>` | Choose which line of a text of several lines the strip shows ("line 1 of 3"). | lines of the text | line 1 |
| **Gap value** | The extra space in the selected gap, in % of the text size. It adds to Letter spacing and to the kerning of the font. Negative values pull the two characters together. | -50 to 400 % | 0 % |
| `-` and `+` | Change the selected gap by 1 % (5 % with Shift held). | | |
| Reset | Sets the selected gap back to 0. | button | — |
| Reset all | Removes the own value of every gap of the text. | button | — |

**Text tab, page Warp.**

| Control | What it does | Range | Default |
|---|---|---|---|
| Style | None, Arc, Arc Lower, Arc Upper, Arch, Bulge, Shell Lower, Shell Upper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist. Each is a small picture drawn with the real formula. | 16 choices | None |
| Bend | Strength and direction of the style. | -100 to 100 % | 50 % when a style is chosen first |
| Horizontal | One side of the text gets taller, the other shorter (1.4 times and 0.6 times at 100 %). | -100 to 100 % | 0 |
| Vertical | The top gets wider, the bottom narrower, or the reverse. | -100 to 100 % | 0 |
| Direction | Horizontal / Vertical: applies the style along the other axis. | 2 buttons | Horizontal |
| Reset | Style None, sliders back to the defaults. | button | — |

**Text tab, page Path.**

| Control | What it does | Range | Default |
|---|---|---|---|
| Kind | Straight / Arc / Circle / Curve. | 4 buttons | Straight |
| Bend (Arc) | Share of a full circle the text covers. Positive: the text arches upward; negative: it hangs like a smile. The circle follows the length of the text. | -100 to 100 % | 25 % (a quarter circle) |
| Diameter (Circle) | Diameter of the circle, in text sizes. The circle keeps this size whatever is typed. | 0.5 to 100 | 2 / pi = 0.64 times the text width, at least 1. A new Circle shows the text centred over the top point, covering the upper half, whatever Align says. |
| Fit (Circle) | Sets the diameter from the text as it is now, so that the widest line covers half the circle again. The top point stays. | button | — |
| Add point / Remove point (Curve) | Adds a point in the middle of the longest piece / removes the point dragged last. The points are dragged in the view. | 2 to 16 points | 3 points on the baseline (no letter moves by more than one raster pixel until a point is dragged) |
| Start | Slides the text along the line, in text sizes. On an Arc or a Curve the text can be slid by at most one line length past either end; on a Circle the value turns the text around the circle. | -100 to 100 | 0 (a new Circle of the smallest diameter gets the value that centres the text) |
| Other side | Puts the text on the other side of the line, reading the other way (the inside of a circle). | on / off | off |

Warp and Path can be combined: the letters are first put on the line, then the warp bends the result. Gap
values and Letter spacing are applied first of all, before line and warp.

A text that is longer than its Circle runs over its own start. The app does not prevent this (the circle keeps
its size on purpose); the Path page then shows the line "The text is longer than the circle. Raise Diameter or
press Fit." (section 6.4).

### 1.3 What does not change

- A text is one re-editable layer of kind "text". Undo, redo, save, reopen and every export work as today.
- Every committed change is one undo step called "Edit Stamp"; a slider drag or a point drag is one step.
- 3D (projected) and 2D (UV) texts stay separate kinds. Nothing converts one into the other.
- A text that uses none of the new controls is drawn by the code of today and looks exactly as before, pixel for
  pixel.
- The existing fields of a selected text (Name, Tint, Size, Rotation, Depth, flips) stay below the four rows.
- **One existing control behaves differently:** Letter spacing of a placed text. Today the text grows about its
  centre when the value changes; from now on the alignment side stays where it is (the left edge of a
  left-aligned text), the same rule as for a gap value (section 5.11). Typing still grows the text about its
  centre, as today (question Q8).

### 1.4 What the user does not get (said plainly)

- "Like in Photoshop" is met by names, controls and the order of work. Nobody on this project can compare pixels
  with Photoshop (user decision 3 of `target\round3_audit\ROUND3_PLAN.md`, lines 313-316: no image editor is
  available), and the warp formulas of Photoshop are not public. A given percentage will not match Photoshop
  exactly.
- The line of "curved along a line" lies in the flat picture of the stamp. It does not run around the model.
- Typing stays in the side panel. There is no text cursor on the model.
- Bold, slant, font and colour apply to the whole text. Parts of one text cannot have their own.
- The gap strip works on a placed text (it is selected right after the click), not before placing.
- Variable fonts show one style. On this machine these are Bahnschrift, Segoe UI Variable, Sitka and Ubuntu Mono
  (measured by the lean proposal, not re-measured).
- A font installed while the app is open appears after a restart. Fonts that Windows knows only through its
  registry (installed from another folder) are not listed.
- The font of a text is stored whole in the project file, as today; the largest font file on this machine is
  37 MB (`map_core.md` section 2.1, not re-measured).
- No ligatures, no right-to-left text, no kerning from the newer kerning table: the rule "one character, one
  glyph" of today stays, so the width of existing texts cannot change. An accent typed as its own character
  (a combining mark) and the parts of a joined emoji are single glyphs to the layout, as today; Letter spacing is
  added after them as today. The gap strip offers no slot inside such a pair, and thickening keeps a mark over
  its letter (sections 5.2 and 5.12).
- A text on a Circle is not shortened or shrunk when it gets longer than the circle; it overlaps its start and
  the Path page says so.

### 1.5 Photoshop next to this design

| Photoshop | This design |
|---|---|
| Font family menu with search and samples | Font picker with search and samples |
| Font style menu | Style list, now with every face of the family |
| Faux Bold, Faux Italic | B and I: the real face first, thicken or slant when there is none; amounts adjustable |
| Size, Leading, Tracking, Colour | Text size, Line spacing, Letter spacing, Colour |
| Kerning between two characters, next to tracking | Gap value for every gap on its own, next to Letter spacing |
| Warp Text dialog: 15 styles, Bend, Horizontal and Vertical Distortion, orientation | Warp page: same names and controls |
| Type on a path | Path page: Arc, Circle, Curve |
| Rotate with Free Transform | Angle, gizmo knob, Shift+wheel (exists) |
| Character panel as a tab in a panel group | Text tab next to Layers |
| Formatting a selected range of characters; typing on the canvas; baseline shift; caps; underline; justified paragraphs; vertical type | Not included |

## 2. Readings of unclear words

For every unclear word: the reading this design builds, the other readings with their extra cost, and the exact
question to put to the user. The open questions should be answered before the packages start; the design is cut
so that each answer changes one package only (last column).

**"Segmented" is decided and is not asked again.** The user was asked on 2026-11-17 at 13:00 and answered, word
for word: "segmented means editing the space between each character" (`FEATURE_BRIEF.md`, "USER DECISION"). This
design therefore builds a spacing value for every single gap between two neighbouring characters (Photoshop:
kerning between two characters, next to tracking for the whole text). The earlier working reading, own font,
size or colour for parts of a text, is withdrawn and not designed here (section 9.2).

| Word in the request | Reading built | Other readings and their extra cost | Question for the user | If the answer differs |
|---|---|---|---|---|
| "segmented" | **Decided by the user:** the space between any two neighbouring characters can be changed on its own. One value per gap, in % of the text size, added to Letter spacing and to the kerning of the font. | Not asked for, not designed: own font, size, colour, bold or slant for parts of a text; letters or words as movable pieces; a stencil look. | none (decided) | — |
| "Stamp type; text ... to put onto the stamp" | The Text tool of today is the text stamp type: a text is its own stamp and its own layer, placed and transformed like an image stamp. Everything in this design applies to it. | (a) Text as a choice inside the Stamp tool (one tool, a switch Image / Text): same engine, about half a day for the switch, and the key X would lose its meaning. (b) Text written onto an image stamp, both as one object: a stamp with two sources, new bake order, new file fields; about 1 week and its own design. | Q7: "Is a text that is its own stamp, as the Text tool makes today, what you mean by 'stamp type: text', or do you want to put text onto an image stamp so that both move as one?" | Reading (a): the tool bar and T6 only. Reading (b): a new design. |
| "choose from fonts on the machine" | Every font in the three Windows font folders is listed already. Added: search, a sample per font, all faces of a family, order without regard to case. | (a) Also fonts known only to the Windows registry, or loading a font file by hand: about 1 day (file dialog action, one more job). (b) Named styles of variable fonts: about 3 days and a second font parser feature that changes how `fontdue` is built (section 5.1); not recommended. | Q5: "Is a font you use missing from the font list of the app today?" | Only the fonts package (T3). |
| "warped" | The Warp Text of Photoshop: 15 named styles with Bend, Horizontal, Vertical. The letter shapes bend. | A free grid of points dragged by hand: about 5 days (grid editor in the view, grid stored per text). | none | — |
| "curved along a line" | The line lies in the flat picture of the stamp; the stamp is then projected (3D) or placed (2D) as one picture, as today. Kinds: Arc, Circle, Curve. | The line is drawn on the surface of the model and every letter is projected on its own: a new stamp kind with one projection per letter, new bake code, a gizmo that is no longer one rectangle. About 3 weeks, and it needs its own design. | Q2: "Is a curved text on one side of the model enough, or must a text be able to follow a line that runs around the model, for example once around a bottle?" | The Path page (parts of T1, T6, T7) is postponed; everything else stays. |
| "angled" | The rotation of the whole text. It exists today four ways (slider before placing, gizmo knob, numeric field, Shift+wheel); this design adds the field "Angle" to the Text options. | Every letter turned on the spot while the baseline stays straight: about half a day (one more number, one rotation per letter in section 5.5). | Q3: "By 'angled', do you mean the whole text turned (exists today), or each letter turned on the spot?" | One number in `TextFx`, one step in the styled painter (T2), one control (T6). |
| "slanted" | Each letter leans by an angle; the baseline stays. | Shearing the whole block including the baseline: for one line it looks the same as Slant plus Angle; not added. | none | — |
| "bolded" | B takes the real bold face of the family; if there is none the letters are thickened. 36 of the 76 installed families have a bold face and 18 an italic one (lean proposal, measured), so both ways are needed. | The strict split of Photoshop (style menu for real faces, button for thickening only): same engine, other button rule. | none | — |
| "like in Photoshop" | Same names, same kind of controls, same order of work where it fits this app. | Same pixels: cannot be checked by anyone on this project. | none | — |
| "its own tab" (said about the brush; decides where the extra text controls go) | A Text tab next to Layers and Brush in the lower part of the side panel, the place `FEATURE_BRUSH.md` chooses for the Brush tab. While it is open the layer list is hidden. | A floating panel over the view: nothing is hidden, but it covers part of the model and would be the first floating window of the app (about 1 day more for its tests). | Q4: "The extra text settings (and the brush settings) get a tab next to Layers; while such a tab is open you do not see the layer list. Is that acceptable, or do you prefer a small floating panel over the view?" | Only the container (section 6.1). |

Behaviour nobody asked for is **not** built. Each needs a yes from the user:

- Q6a: "Should a click with the Text tool on an existing text select that text for editing instead of placing a
  second text on top?" (Half a day. The hit area would be the rectangle of the text, because the hit test of
  today answers only on inked texels, `core\doc_stamps.rs:514-531`.)
- Q6b: "Should the app remember the last font and text settings between program runs?" (Half a day; today nothing
  about text is saved in the preferences, `app\state.rs:458-475`.)
- Q6c: "Do you want a button that reads the installed fonts again without a restart?" (2 hours.)
- Q8: "When you type more letters into a placed left-aligned text, should its start stay where it is, so that
  the text grows to the right, instead of growing to both sides as today?" (Half a day: the rule this design
  builds for spacing changes, section 5.11, would also be used for content changes. Not built without a yes,
  because it changes what typing does today, ARCH:2156-2160.)

## 3. What exists today

### 3.1 Fonts

- The system font list is loaded. `WteApp::new` starts one job (`app\app.rs:167`) that runs
  `FontCatalog::bundled()` and `load_system()` off the UI thread (`app\io_flows.rs:449-458`); the result replaces
  `state.fonts` (`app\io_flows.rs:231-235`). `load_system` is fontdb's `load_system_fonts()`
  (`core\text\font_catalog.rs:80-83`). The app logs 78 families on this machine
  (`target\round3_audit\runs\text_builtin\shot.log`, not re-read).
- fontdb loads only files whose extension is exactly `ttf`, `ttc`, `otf`, `otc` in all-lower or all-upper case
  (`REG\fontdb-0.24.0\src\lib.rs:375-378`); it reads no registry (`map_core.md` section 2.1, not re-read).
- The family list is sorted with `Vec<String>::sort()`, so capital letters come before small ones
  (`core\text\font_catalog.rs:85-90`).
- The style name of a face is built from weight and slant only (`core\text\font_catalog.rs:30-49`), and the
  style list drops every later face with a name that is already present (`:122-131`). Measured by the lean
  proposal on this machine: 173 faces in 150 files, 76 installed families; exactly 2 faces are hidden by this
  rule (Candara Light and Candara Light Italic, which report weight 400 like Candara Regular and Italic); two
  judges re-checked the Candara pair. Not re-measured for this document.
- The Text options show two plain `ComboBox`es, one `selectable_label` per name in the UI font, 320 pt high, no
  search, no sample (`app\panels\tool_options.rs:295-340`).
- The two bundled fonts are Ubuntu Light and Hack Regular (`core\text\font_catalog.rs:69-77`): no bold, no
  italic.
- `face_bytes` copies the whole font file into a new allocation on every call
  (`core\text\font_catalog.rs:143-145`); `BlobStore::insert` then hashes it and compares it with the stored blobs
  (`core\blob.rs:40-52`). The Text tool does this while hovering, because the hover measures the text
  (`app\tools\stamp_place.rs:48-60`, `:149-170`, `:222-224`). Blobs are never removed while the document is
  open (`core\blob.rs:28-30`).
- `fontdue` parses a font once per resolution bucket (`core\text\font_catalog.rs:159-167`, `:182-200`) and
  converts every glyph the character map names while parsing (`REG\fontdue-0.9.5\src\font.rs:258-312`). The parse
  runs inside document commands, on the UI thread.
- Nothing about the Text tool is saved between program runs: `Prefs` has no text or font field
  (`app\state.rs:458-475`).
- A picked family is looked up **only in the system catalog**: `font_ref_for` calls `state.fonts.find(family,
  style)` and on a miss returns the bundled font with the toast "Font {family} not found, using Ubuntu Light"
  (`app\panels\tool_options.rs:345-355`); the Text tool does the same (`app\tools\stamp_place.rs:48-60`). The
  catalog reads fontdb only (`core\text\font_catalog.rs:101-133`). So a font that is in the project file but not
  installed cannot be given to another text today.
- A `FontRef` holds the blob, the face index, the family and the style name, nothing else
  (`core\text\mod.rs:51-56`); weight and italic exist only in the catalog's `FontFaceInfo`
  (`core\text\font_catalog.rs:110-118`). fontdb takes them from the OS/2 table (weight, width, style) and the
  italic angle of the `post` table (`REG\fontdb-0.24.0\src\lib.rs:1039-1043`, `:1147-1167`); `ttf-parser` offers
  the same values on a parsed face (`REG\ttf-parser-0.25.1\src\lib.rs:1410-1413`, `:1469-1471`, `:1477-1479`).
- `faces()` builds its list anew on every call, and `find` and `pick_style` call it
  (`core\text\font_catalog.rs:101-138`, `app\panels\tool_options.rs:281-291`).
- `Document::register_font` always goes through `BlobStore::insert` with the bytes it is given
  (`core\doc_stamps.rs:182-188`); for bytes that are already stored, `insert` returns the old id and drops the
  `Arc` it was given (`core\blob.rs:40-47`).
- `fontdue` fills its character map from **every** cmap subtable, a later one overwriting an earlier one
  (`REG\fontdue-0.9.5\src\font.rs:263-275`); `ttf_parser::Face::glyph_index` looks only at Unicode subtables
  (`REG\ttf-parser-0.25.1\src\lib.rs:1839-1851`, `tables\cmap\mod.rs:61-83`). For symbol fonts the two differ.

### 3.2 Text settings and editing

- A text has one font, one colour, one alignment, line spacing, letter spacing and an optional outline:
  `TextSpec` (`core\text\mod.rs:60-75`). No field for bold, slant, warp, a line or ranges, and no `Default`.
  `TextSpec` is written as a full struct literal in 11 places (`app\control_sweep\panel_tests.rs:510`,
  `app\panels\layers.rs:585`, `app\panels\tool_options.rs:950`, `app\tools\stamp_place.rs:62`,
  `core\doc_stamps.rs:874`, `core\history.rs:1537`, `core\project\manifest.rs:272` and `:394`,
  `core\text\layout.rs:193`, `crates\wte_core\tests\end_to_end.rs:202`, `project_roundtrip.rs:142`), plus one
  struct-update expression that keeps compiling (`project_roundtrip.rs:487`).
- The settings of the Text tool are a second structure, `TextSettings` (`app\state.rs:198-237`), copied into a
  `TextSpec` by `text_source` (`app\tools\stamp_place.rs:48-72`).
- There are two separate sets of controls: `text_options` for the tool (`app\panels\tool_options.rs:203-266`)
  and `text_stamp_fields` for a selected text (`:508-599`). With the Text tool active the panel shows the
  settings of the tool even when a text is selected (`:85`). Round 3 changes this (item R3-10, section 7.4).
- Every change of a placed text is `AppAction::UpdateStampSource(id, StampSource::Text(spec), commit)`
  (`app\action.rs:102`) with the whole spec: `Preview` while a control is live, one `Final` at the end
  (`continuous_edit`, `app\panels\mod.rs:294-323`). The `Final` that is still owed is one slot
  (`app\panels\mod.rs:72-73`) and is sent by `end_frame` only when no pointer button is down and **no text field
  has the focus** (`app\panels\mod.rs:125-133`).
- `Document::set_stamp_source` keeps the raster resolution of the stamp (`core\doc_stamps.rs:331-334`),
  rasterizes the new text at once on the calling thread, also for a `Preview` (`:345-350`), multiplies the size
  of the placement by the ratio of the new and old picture sizes (`:367-371`; `rescale` at `:55-63` changes the
  size only, never the centre), and re-fits an automatic depth (`:374`). A picture that is too large is returned
  as an error; a lower resolution is not tried (`:348`).
- Keys: while an egui text field has the focus, single-key shortcuts, Delete and Ctrl+Z are left to the field
  (`app\shortcuts.rs:118-120`); a shortcut marked `text_ok` first gives up the focus so the field commits
  (`app\shortcuts.rs:130-136`). B alone selects the Brush, I the Eyedropper (`app\shortcuts.rs:79`, `:81`); Ctrl+B
  and Ctrl+I are not bound (table at `app\shortcuts.rs:50-94`). Delete removes the selected stamp
  (`app\shortcuts.rs:62`, `app\dispatch.rs:551-557`), and ARCH:2822-2824 allows no second owner of a key.
- After a click the new stamp is selected and the tool switches to Transform
  (`app\tools\stamp_place.rs:210-218`). Choosing another tool does not clear the selection
  (`app\dispatch.rs:631-635`); only Esc does (`app\dispatch.rs:787-789`). So when the user presses X for a second
  text, the first one is still selected; today the panel then shows the settings of the tool plus the row
  "Selected: ... Edit" (`app\panels\tool_options.rs:85`, `:91-101`). Before the click the user sees only a yellow
  frame (`app\tools\stamp_place.rs:320-339`).
- "Angled" exists: the Rotation slider before placing (`app\panels\tool_options.rs:258-260`), Shift+wheel
  (`app\tools\stamp_place.rs:310-314`), and after placing the numeric Rotation field (`:641-646`), the gizmo knob
  (`app\view\gizmo.rs:12`, `:117-119`; drag at `app\tools\transform.rs:261-270`) and Shift+wheel
  (`app\tools\transform.rs:358-384`). It turns the whole rectangle (`core\stamp_bake.rs:134-140`,
  `core\stamp.rs:49-50`). A stamp is a rectangle with four corners in the data, in the gizmo and in the hover
  frame (`core\stamp_bake.rs:113-117`, `:171-176`; `app\view\gizmo.rs:29-39`, `:81-95`).

### 3.3 From text to pixels

- Layout (`core\text\layout.rs:48-175`) places one `char` after the other with `fontdue` metrics and kerning
  (`:80-102`), rounds every glyph origin to a whole pixel (`:94`), rounds every baseline to a whole pixel
  (`:142`), rounds the alignment offset (`:136`), and makes the canvas the line box plus the same padding on all
  four sides, where the padding is `ceil(outline width) + 2` **plus the largest overhang of any glyph over the
  line box** (`:138-156`). So the centre of the picture is exactly the centre of the line box.
- Rasterization (`core\text\raster.rs:37-114`) takes one `fontdue` coverage bitmap per character, merges with a
  per-pixel maximum (`:68-74`), builds the outline from a distance transform (`:77-91`, `core\text\edt.rs:23`)
  and composes fill over outline (`:18-31`, `:83-113`).
- `fontdue` builds its glyphs with `face.outline_glyph(glyph_id, ..)` and `face.glyph_hor_advance(glyph_id)` of
  `ttf-parser` (`REG\fontdue-0.9.5\src\font.rs:296-304`). It depends on `ttf-parser` 0.25 with exactly the
  features `opentype-layout` and `no-std-float` (`REG\fontdue-0.9.5\Cargo.toml:69-75`). It gives out glyph
  indices, advances and kerning by index (`font.rs:434-440`, `:467-472`, `:637-640`) but no outlines. A glyph
  bitmap is placed so that the outline keeps its exact fractional position (`font.rs:475-495`).
- `ttf-parser` 0.25.1 offers `Face::parse(data, index)`, which its documentation calls "in general ... free"
  (`REG\ttf-parser-0.25.1\src\lib.rs:1100-1108`), the `OutlineBuilder` trait (`:577-596`) and
  `Face::outline_glyph` (`:2125-2153`). It is in `Cargo.lock` only through `fontdue` (`Cargo.lock:1255-1262`,
  `:3540-3546`).
- The picture is cached per stamp with the spec it was made from (`core\document.rs:69`,
  `core\doc_stamps.rs:666-675`). Undo drops a cached picture whose spec no longer matches
  (`core\history.rs:725-729`); a project load does not rasterize at all when the saved layer pixels are intact
  (`core\project\load.rs:397-410`). Every bake asks for the picture and rasterizes when it is not cached
  (`core\doc_stamps.rs:736-739`, `:678-683`).
- The core picks the resolution: 32, 64, 128, 256 or 512 px per em from the height of one em in texels
  (`core\doc_stamps.rs:43-44`, `:562-579`), on add and on every final bake (`:262`, `:546-556`, `:688-733`),
  never in a preview. `retune_text` rescales about the centre only (`:704-713`).
- The bake sees one flat picture. A picture pixel maps onto the placement rectangle with
  `u = 0.5 + l.x / width`, `v = 0.5 - l.y / height` and the two flips (`core\stamp_bake.rs:341-348`) in 3D, and
  with the rotated frame in 2D (`core\stamp_bake.rs:162-165`, `:693-701`).
- A canvas side above 8192 px is an error (`core\text\layout.rs:10`, `:157-160`). There is no limit on the
  area, so a plain text of many long lines can reach 8192 x 8192 px today. The distance transform of the outline
  allocates 8 bytes per canvas pixel (`core\text\edt.rs:35`, `:67`).
- Spacing today: one `letter_spacing` value for the whole text, a fraction of the em
  (`core\text\mod.rs:72-73`), clamped to -0.5..=4 in the layout (`core\text\layout.rs:55`), added to every
  advance (`:100`) and taken off the end of each line (`:118`); and the pair kerning of the font, applied
  automatically (`:82-88`). The slider shows -20 to 100 % (`app\panels\tool_options.rs:253-254`, `:588-590`).
  There is no value for a single gap.

### 3.4 Project file and undo

- A text is stored as a flat `TextEntry` with eight required fields (`core\project\manifest.rs:147-159`),
  converted by `From<&TextSpec>` and `to_spec` (`:249-288`). `Manifest.stamps` is a typed list (`:34-35`), so one
  entry that does not parse fails the whole manifest (`core\project\migrate.rs:73-74`). The rule of the schema is
  "new optional fields must be `#[serde(default)]`" (`core\project\manifest.rs:4`).
- `FORMAT_VERSION` is 1 (`core\project\mod.rs:19`); the migration chain is empty and a test ties its length to
  the version (`core\project\migrate.rs:15`, `:119`). A reader refuses a file whose `min_reader_version` is above
  its own version and opens a newer file with an acceptable `min_reader_version` with a warning and a forced
  "Save As" (`core\project\migrate.rs:68-72`, `core\project\load.rs:441`, `:568`). Save always writes
  `min_reader_version: FORMAT_VERSION` (`core\project\save.rs:375-376`).
- Three places assume one font per text: `referenced_blobs` (`core\project\save.rs:263-272`), the font fallback
  at load with `fallback_font` (`core\project\load.rs:191-206`, `:242-247`) and `text_fonts`
  (`core\doc_stamps.rs:647-656`).
- The floats that save checks for "not a number" are a hand-written list (`core\project\save.rs:112-139`).
- A text edit is undone by swapping a snapshot of all stamp objects and layer records
  (`core\history.rs:607-619`, `:636-648`, `:651-733`); everything inside `TextSpec` is covered as a value.

### 3.5 Space in the side panel

- The side panel is 300 pt wide (`app\app.rs:285-286`). Its rows are 265 pt wide inside the section indent: in
  `target\round3_audit\runs\text_builtin\shot.png` the content field runs from x = 1007 to x = 1272. A wider row
  makes egui widen the panel and moves the 3D view (ARCH:3479-3483; `app\panels\tool_options.rs:133-136`); the
  `text` scenario asserts that the view does not move (`app\selftest\scenarios.rs:419-433`).
- Today the side panel is one scroll area for tool options, colour and texture sets above a resizable Layers
  area of 40 % of the height (`app\panels\mod.rs:255-282`). The Text options are about 344 pt high and push the
  colour picker out of sight (same screenshot: the picker square is cut off at y = 478).
- Round 3 pins the whole Color section and lets tool options and texture sets scroll in their own area (user
  decision 2, `target\round3_audit\ROUND3_PLAN.md:303-311`). That code was not in any round-3 copy when this was
  written (section 7.4). The height that stays visible for tool options at 1280x800 is an estimate: about 140 pt
  (`map_app.md`), about 146 pt (measured on screenshots by `work\brush\proposal_user.md`, section 2.4).
- The row height of the theme is 20 pt and the item spacing 6 pt across, 4 pt down (`app\theme.rs:19-21`).
- The app has no floating window (`egui::Window` does not occur under `app\`). A view takes the pointer only
  when `response.contains_pointer()` (`app\view\mod.rs:368`).

### 3.6 What the tests can do

- Self-test scenarios cannot type: `Step` has `Key` but no text step, and `Setup`, `Assert` and `Dyn` take plain
  `fn` pointers (`app\selftest\mod.rs:48-72`); real text events are dropped (`:210-226`). A pointer step always
  ends with the button release (`app\selftest\mod.rs:552-574`), and a key is its own step, so a scenario cannot
  press Esc while a button is down.
- Stamp ids are counted per document (`core\doc_stamps.rs:254`), and a new session starts without a selection
  (`app\io_flows.rs:491-496`); `Session` holds what belongs to one open document (`app\state.rs:369-381`).
- The headless panel tests click, drag, press keys and type into the real panel code
  (`app\control_sweep.rs:207-285`, `type_text` at `:267-269`), with the frame built by `frame_inner` (`:374-389`),
  which calls `shortcuts::handle` and `panels::show_side_panel`.
- The overlay of a tool gets `&self`, `&AppState` and an `egui::Painter` (`app\tools\mod.rs:202`); `hover`,
  `press` and `drag` get `&mut AppState` (`:183-187`). The Transform tool calls the document directly during a
  drag (`app\tools\transform.rs:320-342`), and its drag state and the gizmo shape are `Copy` types
  (`app\tools\transform.rs:22-42`, `app\view\gizmo.rs:28-39`).

## 4. Data model, file format, versioning, undo

### 4.1 Core types (`core\text\mod.rs`)

All new settings sit in one new field of `TextSpec`. Each of the 11 literal sites of section 3.2 gains one line,
`fx: TextFx::default()`.

```rust
pub struct TextSpec { /* the eight fields of today, unchanged */ pub fx: TextFx }

#[derive(Clone, PartialEq, Debug, Default)]
pub struct TextFx {
    pub gaps: Vec<Gap>,         // own spacing of single gaps; sorted by `after`, one entry per character at most, no zeros
    pub bold_em: f32,           // 0..=0.08: every edge grows by this share of the em; 0 = off
    pub slant_deg: f32,         // -45..=45, positive leans to the right; 0 = off
    pub warp: Option<Warp>,
    pub path: Option<TextPath>,
}
impl TextFx { pub fn is_default(&self) -> bool;          // == TextFx::default(): the text uses no new feature
              pub fn needs_outlines(&self) -> bool }     // thickening, slant, warp or path: the styled painter draws it

/// The extra space after the stored character number `after` (counted in `char`s of `content`), as a fraction of
/// the em like `letter_spacing`. It takes effect when another glyph follows on the same text line.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Gap { pub after: u32, pub em: f32 }          // em: GAP_MIN_EM..=GAP_MAX_EM
pub const GAP_MIN_EM: f32 = -0.5;                       // the limits `letter_spacing` has in the layout today
pub const GAP_MAX_EM: f32 = 4.0;

#[derive(Copy, Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Warp { pub style: WarpStyle, pub bend: f32, pub horizontal: f32, pub vertical: f32,   // each -1..=1
                  pub vertical_axis: bool }
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarpStyle { Arc, ArcLower, ArcUpper, Arch, Bulge, ShellLower, ShellUpper, Flag, Wave, Fish, Rise,
                     Fisheye, Inflate, Squeeze, Twist }
impl WarpStyle { pub const ALL: [WarpStyle; 15]; pub fn key(self) -> &'static str;      // "arc_lower"
                 pub fn from_key(key: &str) -> Option<WarpStyle>; pub fn label(self) -> &'static str }   // "Arc Lower"

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct TextPath { pub shape: PathShape, pub start: f32 /* em along the line, -100..=100; see 5.5 */, pub flip: bool }
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum PathShape {
    Arc { bend: f32 },                             // -1..=1: share of a full circle the widest line covers
    Circle { diameter: f32, center: [f32; 2] },    // em; diameter 0.5..=100; centre relative to the anchor,
                                                   // each coordinate -PATH_COORD_MAX_EM..=PATH_COORD_MAX_EM
    Curve { points: Vec<[f32; 2]> },               // 2..=MAX_PATH_POINTS points, em, relative to the anchor, y down,
                                                   // each coordinate -PATH_COORD_MAX_EM..=PATH_COORD_MAX_EM
}
pub const MAX_PATH_POINTS: usize = 16;
pub const PATH_COORD_MAX_EM: f32 = 60.0;   // a curve across the whole range is 122 em wide and high with its letters:
                                           // 3,904 px at 32 px per em, 15.2 megapixels, inside both picture limits
pub const MAX_STYLED_PIXELS: u64 = 1 << 24;   // 16.8 megapixels: area limit of a styled picture (5.10)
pub const FAUX_BOLD_EM: f32 = 0.02;        // what B sets when the family has no bold face
pub const FAUX_SLANT_DEG: f32 = 12.0;      // what I sets when the family has no italic face

impl TextSpec {
    pub fn new(content: &str, font: FontRef) -> TextSpec;   // defaults of today: 128 px, black, left, 1.0, 0.0, no outline
    pub fn sanitize_fx(&mut self);                          // repairs `fx`, see the rules below
    pub fn floats(&self, f: &mut dyn FnMut(&'static str, f32));                 // every float, read only (save check)
    pub fn floats_mut(&mut self, f: &mut dyn FnMut(&'static str, &mut f32));    // the same floats, same order (repair)
    pub fn differs_only_in_spacing(&self, other: &TextSpec) -> bool;   // only `letter_spacing` and / or `fx.gaps` differ
}

/// Where the text sits in its full-resolution picture. All lengths in raster pixels of that picture.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct TextFrame {
    pub size: [u32; 2],        // picture size
    pub anchor_px: [f32; 2],   // the anchor (origin of text space) in the picture
    pub em_px: f32,            // raster pixels per em
    pub box_px: [f32; 2],      // width and height of the straight line box
    pub baseline0_px: f32,     // y of the first baseline relative to the anchor (negative = above it)
    pub align_px: [f32; 2],    // the point that a change of spacing keeps in place (5.11): on the line box at the
                               // alignment side (left edge, centre or right edge), at the height of the anchor;
                               // equal to `anchor_px` for a text with a warp or a path
    pub circle_fill: f32,      // Circle only, else 0: the largest share of its own circle that a text line covers;
                               // above 1 the text runs over its own start
}
pub enum RasterQuality { Full, Preview { max_pixels: u32 } }
pub struct TextRaster { pub image: PixelBuf, pub frame: TextFrame, pub full: bool }
```

**What a gap is.** The layout walks the stored characters of `content` with their index: a line feed ends a
line, a tab is laid out as four spaces, every other control character (also a carriage return) is skipped. This
gives the same lines as the code of today (`core\text\layout.rs:67-112`) and never moves an index. The gap
"after character `i`" is the space between the glyph of stored character `i` and the next glyph on the same
text line.

| Case | Rule |
|---|---|
| Key | The index of the character on the **left** of the gap. A gap value belongs to that character and moves with it when the text is edited (section 5.12). |
| Unit and range | A fraction of the em, like `letter_spacing` (`core\text\mod.rs:72-73`); -0.5 to 4.0, the limits the layout gives `letter_spacing` today (`core\text\layout.rs:55`). In the panel: -50 % to 400 % of the text size. |
| Last character of a line | Its gap value has no effect (nothing follows on the line) and does not widen the line. The value is kept, so that removing the line break brings the gap back. It is **not hidden**: the strip shows it as a dashed end slot that can be selected and reset (6.2), and because it is real, stored data it counts for `min_reader_version` like every other gap value (4.3). |
| Joined characters | No gap exists between a character and a combining mark, a variation selector or a zero-width joiner that follows it, nor after a zero-width joiner (`gaps::is_joined`, the fixed ranges of 5.12). A value keyed to the left character of such a pair is removed by `sanitize_fx`. |
| Line feed | A line feed is never the left side of a gap. A value keyed to it is removed by `sanitize_fx`. |
| Tab | The four spaces of a tab are one stored character. Its gap value is applied once, after the fourth space; a gap before the tab belongs to the character before it. |
| Skipped characters | A value keyed to a skipped control character is removed by `sanitize_fx`. A value on the visible character before it acts between that character and the next glyph of the line. |
| Fallback font | The value does not depend on which font supplies the two glyphs; it is always a fraction of the em of the text. |
| Kerning of the font | Unchanged and added as today, when both glyphs come from the same font (`core\text\layout.rs:82-88`). |

Other rules:
- All lengths are fractions of the em or pure numbers, so the look does not depend on the resolution the core
  picks (the rule of ARCH:2094-2095).
- **`sanitize_fx` repairs, it never rejects and never panics.** Non-finite numbers of thickening, slant, warp
  and path become the default and all are clamped to the ranges above; every coordinate of a Curve point and of
  a Circle centre is clamped to -60..=60 em (`PATH_COORD_MAX_EM`), `start` to -100..=100 em, the diameter to
  0.5..=100 em. With these ranges every path that can be stored has a picture at some resolution: no value from a
  file or from a far drag can make each later edit of the text fail (the length of the text itself can still be
  too large, as for a straight text). Gaps: an entry with a non-finite value, with an index beyond the text,
  keyed to a line feed or a skipped character, or keyed to the left character of a joined pair is removed; values
  are clamped to -0.5..=4.0; of two entries for one character the later wins; zeros are removed; the list is
  sorted. A curve with more than 16 points is cut and one with fewer than 2 is dropped; a warp with bend 0 and both distortions
  0, and an Arc with bend 0, stay as they are (they are valid and draw as plain shapes). It is called by
  `add_stamp`, `set_stamp_source`, `stamp_source_size`, `stamp_source_frame`, `text_preview` and by
  `TextEntry::to_spec`, so a spec from a file is repaired exactly like a spec from the app. It does not touch the
  eight old fields, which the layout sanitizes on use as today (`core\text\layout.rs:13-23`, `:55-56`).
- `TextSpec` equality stays the cache key and the change test (`core\doc_stamps.rs:337`, `:566-568`,
  `:667-668`). Because every stored spec went through `sanitize_fx`, no new field can hold a NaN.
- `floats` and `floats_mut` visit `size_px`, `line_spacing`, `letter_spacing`, the outline width, the value of
  every gap, `fx.bold_em`, `fx.slant_deg`, the three warp numbers and every path number. Two visitors are needed
  because the save check works on an immutable snapshot (`validate(snapshot: &DocumentSnapshot)`,
  `core\project\save.rs:84-88`) while `sanitize_fx` changes the values. Both are written by one private macro
  from one field list, so they cannot disagree; a test counts the floats of a full spec through each visitor and
  compares names and order, so a new field cannot be forgotten in one of the two.

### 4.2 App state (`app\state.rs`, `app\action.rs`: shared contract files, changed only by the scaffold)

- `TextSettings` (`app\state.rs:198-237`) gains `bold_em: f32`, `slant_deg: f32`, `warp: Option<Warp>`,
  `path: Option<TextPath>`, all with the defaults of section 1.2. It keeps its derives and `#[serde(default)]`,
  so it stays usable as the hover cache key (`app\tools\stamp_place.rs:31-36`). It gets no gaps: single gaps are
  edited on a placed text. It keeps the font as the two names `family` and `style`; the names are resolved
  against the open project first and the machine second (5.13), so a font that exists only inside the project
  can be used for the next text without a new field.
- `SideTab`, the tab of the lower area that `FEATURE_BRUSH.md` defines (`Layers`, `Brush`), gets a third value
  `Text`. New in `AppState`: `text_page: TextPage` (`Character`, `Warp`, `Path`).
- New in **`Session`** (`app\state.rs:369-381`), not in `AppState`:
  `text_gap: Option<TextGapSel { stamp: StampId, after: u32 }>`, the gap selected in the strip. Stamp ids are
  counted per document (section 3.6), so a selection kept across documents could point at another text; a
  `Session` is built anew for every document (`app\io_flows.rs:491-496`), which clears it.
- New action: `LoadFontSamples` (starts the sample job of 5.13). Every text control computes the new spec in the
  panel and sends the existing `UpdateStampSource`.
- `Prefs` is not changed (question Q6b). The shortcut table is not changed.

### 4.3 Project file

`TextEntry` (`core\project\manifest.rs:147-159`) gets one field. All entry structs keep
`Serialize, Deserialize, Clone, Debug, PartialEq`.

```rust
pub struct TextEntry { /* the eight fields of today */
    #[serde(default, skip_serializing_if = "TextFxEntry::is_default")] pub fx: TextFxEntry }

#[derive(Default)] pub struct TextFxEntry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]   pub gaps: Vec<GapEntry>,
    #[serde(default, skip_serializing_if = "is_zero")]         pub bold_em: f32,
    #[serde(default, skip_serializing_if = "is_zero")]         pub slant_deg: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub warp: Option<WarpEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub path: Option<PathEntry>,
}
pub struct GapEntry { pub after: u32, pub em: f32 }
pub struct WarpEntry { #[serde(default)] pub style: String,    // WarpStyle::key(); a string, so an unknown or missing name cannot fail the file
                       #[serde(default)] pub bend: f32, #[serde(default)] pub horizontal: f32,
                       #[serde(default)] pub vertical: f32, #[serde(default)] pub vertical_axis: bool }
pub struct PathEntry { #[serde(default)] pub kind: String,     // "arc" | "circle" | "curve"; missing = "" = unknown
                       #[serde(default, skip_serializing_if = "is_zero")]       pub bend: f32,      // written for an arc
                       #[serde(default, skip_serializing_if = "is_zero")]       pub diameter: f32,  // written for a circle
                       #[serde(default, skip_serializing_if = "is_zero2")]      pub center: [f32; 2],
                       #[serde(default, skip_serializing_if = "Vec::is_empty")] pub points: Vec<[f32; 2]>,   // a curve
                       #[serde(default)] pub start: f32, #[serde(default)] pub flip: bool }
impl TextEntry { pub fn to_spec(&self) -> TextSpec;            // unknown warp style or path kind: that feature is None; result sanitized
                 pub fn unknown_features(&self) -> Vec<String> }   // for example ["warp style \"ribbon\""]
```

A text source with everything set:

```json
{ "kind": "text", "content": "WTE 2026", "font": { "blob": 3, "face_index": 0, "family": "Ubuntu", "style": "Light" },
  "size_px": 128.0, "color": [0,0,0,255], "align": "center", "line_spacing": 1.0, "letter_spacing": 0.0, "outline": null,
  "fx": { "gaps": [ { "after": 2, "em": 0.3 }, { "after": 5, "em": -0.05 } ],
          "bold_em": 0.02, "slant_deg": 12.0,
          "warp": { "style": "arc", "bend": 0.5, "horizontal": 0.0, "vertical": 0.0, "vertical_axis": false },
          "path": { "kind": "curve", "points": [[-2.0, 0.3], [0.0, -0.2], [2.0, 0.3]], "start": 0.0, "flip": false } } }
```

The fields of a path that do not belong to its kind are zero or empty in the entry and are therefore not written:
the curve above has no `bend`, `diameter` or `center` key, as shown. An arc with bend 0 loads as an arc with
bend 0, because a missing key is the default.

A text without new features is written with exactly the eight keys of today plus `"kind"`. Export (PNG,
per-layer PNG, ORA, PSD) reads the baked layer and is not changed.

Versioning (changes ARCH 7.2 and 7.3). The bump is made once, by the scaffold; `FEATURE_BRUSH.md` 4.5 needs no
format change, so the text feature is its only cause.
1. `FORMAT_VERSION` becomes 2. `STEPS` gets one step, `v1_to_v2`, that returns the manifest unchanged (a
   version-1 text entry has no `fx`, serde fills in the default). The test `STEPS.len() == FORMAT_VERSION - 1`
   (`core\project\migrate.rs:119`) keeps holding.
2. Save no longer writes `min_reader_version: FORMAT_VERSION` blindly (`core\project\save.rs:375-376`). The rule
   becomes: **`min_reader_version` is the lowest format version whose reader shows everything in this file
   correctly.** A new function `required_reader(&DocumentSnapshot) -> u32` returns 2 when any text stamp has
   `!fx.is_default()` (a gap value counts, also one stored on the last character of a line: the strip shows it
   and it takes effect again when the line break is removed, so an older reader would lose real data) and 1
   otherwise.
3. Effects. A version-1 file loads unchanged. A project saved by round 4 without new text features opens in the
   round-3 build with its "newer format" warning and forced Save As. A project that uses a new text feature is
   refused by the round-3 build with "too new"; without this it would show the saved pixels, draw the text
   plain at the first edit and drop `fx` at the next save, silently (no struct uses `deny_unknown_fields`,
   `map_core.md` section 6, not re-read).
4. An unknown or missing **name** never fails a project: `style` and `kind` are strings with a default, and an
   empty name is an unknown feature. Out-of-range and non-finite **values** never fail a project either:
   `sanitize_fx` repairs them. What still fails the whole manifest, like any malformed field of a version-1 file,
   is a wrong JSON **type** inside `fx` (a text where a number belongs, a negative gap index, an object in place
   of a list): the manifest is parsed as one typed value (`core\project\migrate.rs:73-74`) before `sanitize_fx`
   can run. The app never writes such a file. When `unknown_features()` is not empty
   the stamp loads without that feature, a new `LoadWarning::TextFeatureUnknown { stamp, what }` is shown and
   `LoadReport.force_save_as` is set, like for a newer format (`core\project\load.rs:441`, `:568`).
5. Two frozen fixtures are added and must load forever. The first is added **before the schema changes** and
   must pass on the unchanged round-3 code (the scaffold writes it as the test
   `frozen_v1_text_fixture_loads` in `crates\wte_core\tests\project_roundtrip.rs` and runs it green as its first
   project task, before it touches `manifest.rs`; section 7.5 gate 5): a literal version-1 `manifest.json` string with one text stamp and
   one font blob, inside an archive the test builds (the blob bytes are the bundled Ubuntu Light). The frozen
   fixture of today has no stamp (`map_core.md` section 6, not re-read), so nothing pins the version-1 shape of
   a text entry. The second is a literal version-2 manifest with the full `fx` above.
6. `stamp_floats` (`core\project\save.rs:112-139`) is fed from `TextSpec::floats`, the read-only visitor, so
   nothing is cloned. A test puts a NaN into each
   float in turn (bypassing `sanitize_fx`) and expects save to refuse, naming the float.

A text still has exactly one font, so the font handling of save and load (`core\project\save.rs:263-272`,
`core\project\load.rs:191-206`, `:242-247`) is not changed.

### 4.4 Undo

Nothing new is needed. All new settings are inside `TextSpec`, inside `StampObject`, which the structure
snapshot stores and restores as a value (section 3.4). Every committed change is one "Edit Stamp" entry through
the existing `Preview` / `Final` protocol: dragging the gap value, a slider or a curve point sends `Preview`s and
one `Final`; a click on `-`, `+`, Reset, Reset all, a style or a kind is one `Final`. The shift of the rectangle
by the anchor rule (section 5.11) happens inside `set_stamp_source`, so it is in the same entry. Esc during a
point drag restores the state before the drag with `cancel_pending_edit`, as for the other gizmo handles
(`app\tools\transform.rs:398-405`).

Inside the focused content field Ctrl+Z stays the undo of the field. egui applies undo and redo with
`text.replace_with(..)` and a text only (`REG\egui-0.37.0\src\widgets\text_edit\builder.rs:1345-1350`,
`:1363-1368`); it knows nothing about gap values. The gap buffer of section 5.12 therefore keeps its own short
history of (text, gap list) pairs for the time the field has the focus and, in `replace_with`, puts back the gap
list that belonged to exactly that text. A letter deleted by mistake comes back **with** its gap value. This
matters because no `Final` exists while the field has the focus (`app\panels\mod.rs:125-133`), so the document
undo cannot help at that moment.

## 5. Algorithms and cost

### 5.1 Two painters, one layout rule, one new dependency line

```rust
// core\text\raster.rs
pub struct FaceFonts<'a> { pub metrics: &'a fontdue::Font, pub bytes: &'a [u8], pub face_index: u32 }
pub struct TextFonts<'a> { pub font: Option<FaceFonts<'a>>, pub fallback: FaceFonts<'a> }   // font: None when its blob is missing
pub fn text_frame(fonts: &TextFonts<'_>, spec: &TextSpec) -> Result<TextFrame, TextError>;      // geometry only, no pixels
pub fn render_text(fonts: &TextFonts<'_>, spec: &TextSpec, quality: RasterQuality) -> Result<TextRaster, TextError>;
```

- `!spec.fx.needs_outlines()` (no thickening, slant, warp or path): the **classic painter**, which is
  `layout_text` and `rasterize_text` of today (`core\text\layout.rs:48-175`, `core\text\raster.rs:37-114`) with
  **one** addition: the gap value in the pen advance (5.2). Without gap values it gives the bytes it gives
  today. Its frame is: `size` = the canvas, `anchor_px = (width / 2, height / 2)`, `em_px = raster_px(spec)`,
  `box_px` = canvas minus twice the padding, `baseline0_px = round(ascent) - box_h / 2`,
  `align_px = anchor_px + (-box_w / 2, 0)` for Left, `anchor_px` for Center, `anchor_px + (box_w / 2, 0)` for
  Right, `circle_fill = 0`. `quality` is ignored: a
  text without outlines is always rasterized in full, as today. `layout_text`, `measure_text` and
  `rasterize_text` keep their signatures and all their tests.
- Otherwise: the **styled painter** (sections 5.2 to 5.10).
- `text_frame` returns exactly the frame `render_text(.., Full)` would report (a test compares them for 12 specs,
  the successor of `measure_equals_the_raster_size`, `core\text\raster.rs:129-146`).

So a text whose only new feature is gap spacing stays on the `fontdue` painter: its letters have exactly the
shapes and edges they have today, only their positions change.

Both painters take positions from the same source: `fontdue` glyph indices (`lookup_glyph_index`), advances
(`metrics_indexed`) and kerning (`horizontal_kern_indexed`), and the same spacing rule. The styled painter takes
only the glyph **shapes** from somewhere else: `ttf_parser::Face::outline_glyph` with the glyph index `fontdue`
returned. `fontdue` builds its own glyphs with exactly this call (section 3.3), so shapes, character-to-glyph
mapping and advances are the same in both painters. Switching a feature on does not move a letter; only the grey
levels of the edges can differ slightly.

`FaceFonts.metrics` comes from the existing `FontCache`, at the bucket of the text size, as today; `bytes` are
the bytes of the font blob, on which `ttf_parser::Face::parse` only reads the table directory. A text has one
font plus the bundled fallback, as today (`core\doc_stamps.rs:647-656`).

**The dependency line**, in the workspace `Cargo.toml` and as `ttf-parser.workspace = true` in
`crates\wte_core\Cargo.toml`:

`ttf-parser = { version = "0.25.1", default-features = false, features = ["opentype-layout", "no-std-float"] }`

This is the exact feature set `fontdue` enables, so `fontdue` is compiled as today. Verified for this document
by resolution only, in a manifest-only copy at `target\round4_design\work\text\offline_probe_exact\`:
`cargo.cmd tree --offline -p wte_core --depth 1 -e normal,features` ended with exit code 0 and lists the two
features; `Cargo.lock` gained exactly one line (the name `ttf-parser` in the dependency list of `wte_core`) and
still has 456 packages; `cargo.cmd tree --offline -p wte_app -i ttf-parser -e normal,features` shows the same
feature set for the whole app as before. **Not verified: compilation.** The scaffold proves it with a real build
before any other package starts (section 7.5). Fallback if that build fails against expectation: read the
outlines with `skrifa` 0.44.0, which is compiled into the app today through egui and resolved offline in the
earlier probe (`work\text\offline_probe`); only `core\text\outline.rs` would change, because everything after it
works on plain polylines.

Why not the other crates that are in the binary through egui: `harfrust` (shaping) would change the advance
widths of every existing text; the `expand_path` of `kurbo` is documented as unreliable when the growth is large
compared with the curve radius, which is the normal case for bold (`proposal_user.md` 5.1, not re-read);
`vello_cpu` picks its SIMD path per machine and renders 4 bytes per pixel where 1 is needed
(`proposal_engine.md` 5.8, not re-read). The parts needed here are small: a curve flattener, a polygon coverage
rasterizer of about 150 lines, 15 warp formulas.

### 5.2 Spacing and layout: one rule for both painters

Input: the spec, `px = raster_px(spec)` (8 to 512, `core\text\layout.rs:13-15`),
`ls = letter_spacing * px` (clamped as today, `core\text\layout.rs:55`), and for the stored character `i` its gap
value `gap_i * px` (0 when the list has no entry for `i`).

1. Glyph: the first of (font of the text, bundled fallback) whose `lookup_glyph_index(ch)` is not 0; else glyph
   0 of the font of the text. The rule of `core\text\layout.rs:81`.
2. Kerning: added to the pen before a glyph is placed when the previous glyph of the line comes from the same
   font (`core\text\layout.rs:82-88`).
3. Glyph origin: `x = round(pen)` for a text without a path (as `core\text\layout.rs:94`); unrounded on a path.
4. **Advance: `pen += advance_width + 2 * bold_em * px + ls + gap_i * px`** (today: `advance_width + ls`,
   `core\text\layout.rs:100`). For the four spaces of a tab, `gap_i` is added after the fourth only. A glyph whose
   advance width is 0 (a combining mark) adds **no** bold term; see "Marks" below. `ls` is added after every
   glyph, also after such a mark, because the code of today does that (`core\text\layout.rs:100`) and a plain
   text must not change.
5. Line width = the pen position after the last glyph of the line **minus the `ls` and the gap value that
   followed it** (today: pen minus `ls`, `:118`), not below 0. So neither the letter spacing nor a gap value
   after the last character widens a line.
6. Line metrics: ascent, descent and line advance of the font of the text at `px`, with the fallback numbers of
   `core\text\layout.rs:63`. Baseline of line `l`: `y_l = ascent + l * new_line_size * line_spacing`; a text
   without a path uses `round(y_l)`, as `core\text\layout.rs:142` does.
7. Line box: `box_w = max(1, ceil(widest line))`, `box_h = max(1, ceil(y_last - descent))` (`:125-126`).
   Alignment offset per line `round((widest - width_l) * {0, 0.5, 1})` (`:129-137`).

Consequences, **in the picture**: a gap value moves every glyph after the gap on that line by the value and no
glyph before it, measured from the left edge of the line box; the other lines move only through the alignment
(not at all when the text is left-aligned and the widest line does not change). The distance between the
origins of two neighbours is `advance + kerning + letter spacing + gap value`. A negative sum lets letters
overlap; nothing is clipped, because the canvas follows the ink (classic: the overhang padding of section 3.3;
styled: 5.10).

Consequences, **on the model**: the picture gets wider, and where it is put is decided by the anchor rule, not by
the layout. Section 5.11 defines it: for a change of spacing the alignment side of the line box stays where it
is. So for a left-aligned text the letters before the gap stay on the model and the letters after it move by the
full value; for a right-aligned text it is the other way round; for a centred text the letters before the gap
move left by half the value and the letters after it right by half the value.

**Marks.** The layout is one character, one glyph (section 1.4). Two rules keep that from getting worse with the
new features. (1) Thickening: a glyph with an advance width of 0 adds no bold term to the pen and is moved by
`-bold_em * px` instead of `+bold_em * px` in 5.4. The base letter before it was moved right by the radius and
advanced by twice the radius; the mark, placed at the pen, therefore ends up moved right by exactly the radius
too, and stays over its letter. Without the rule "e" + U+0301 at a thickening of 8 % would show the accent
0.16 em to the right of its place. (2) Gaps: no gap exists inside a joined pair (section 4.1), so a gap value
cannot tear an accent from its letter.

**In the classic painter** this is the one change to `layout_text`: the character loop walks the stored
characters with their index (section 4.1, same lines as today), looks the gap value up in a sorted slice, and
uses steps 4 and 5. With an empty gap list every number is the number of today, bit for bit (a test with hashes
recorded before the change guards it, section 7.5).

**In the styled painter** the same seven steps run with unrounded origins where a path is set; for a text
without a path they are, step for step, the numbers of the classic layout, including the three roundings. This
is what makes the hand-over between the painters exact. Spacing is part of the pen positions, so it is applied
before the line (5.5) and before the warp (5.6): on a circle a wider gap is a larger angle between two letters;
under a warp the gap bends with the text.

**Text space.** Unit: raster pixel at the resolution of the stamp. x to the right, y down. The origin is the
**anchor**. For a text without a Circle or Curve the line box is centred on the anchor: its top-left corner is at
`(-box_w / 2, -box_h / 2)`. The baseline of line 0 is at `b0 = round(ascent) - box_h / 2`. For a Circle and a
Curve the stored geometry is relative to the anchor (section 5.5). The anchor is never warped and never the
centre of the canvas by definition; for the classic painter the two coincide because its padding is the same on
all four sides (section 3.3).

A **glyph cell** is the rectangle from the pen position to pen + advance, from the ascent above to the descent
below the baseline, sheared by the slant.

Cost: one hash lookup, two table reads and one binary search in the gap list per character; under 1 microsecond
per character (estimate).

### 5.3 Outlines (`core\text\outline.rs`)

Per glyph: `Face::outline_glyph(GlyphId(g), &mut builder)`. The builder scales by `px / units_per_em`, turns y
down and flattens curves into lines with a tolerance of `tol = 0.05` px: a quadratic piece into
`n = ceil(sqrt(M / (4 * tol)))` lines, a cubic piece into `n = ceil(sqrt(0.75 * M / tol))` lines, where `M` is
the largest second difference of its control points. The result is closed polylines in glyph space: origin on
the baseline at the pen position, y negative above the baseline. A glyph without an outline (a space, a
bitmap-only or colour glyph) has no ink, as in the classic painter.

### 5.4 Slant and bold shift

For every outline point `(x, y)` in glyph space: `x' = x - y * tan(slant_deg) + bold_em * px`, `y' = y`. The first
term leans the letter about its baseline. The second moves it right by the bold radius, so the thickened letter
keeps its left side bearing; the advance already grew by twice the radius in 5.2. For a glyph with an advance
width of 0 the second term is `- bold_em * px` (rule "Marks" of 5.2).

### 5.5 Placing glyphs: straight, Arc, Circle, Curve (`core\text\path.rs`)

**Straight.** Text-space point = `(-box_w / 2 + align_l + x_i + x', -box_h / 2 + round(y_l) + y')`.

**On a line.** A line gives, for an arc length `s` in pixels, a point `P(s)` and a unit tangent `T(s)`.
`D = (-T.y, T.x)` is the direction "down from the baseline" (for a tangent pointing right, `D` points down on
screen). A glyph is rigid: its middle is put on the line and the glyph is turned, never bent. With `pen_i` the
unrounded pen position in its text line and `s0_l` the start of text line `l` on the line:

`s = s0_l + pen_i + adv_i / 2`,   point = `P_l(s) + T_l(s) * (x' - adv_i / 2) + D_l(s) * y'`.

`start` (em) slides the text along the line. **Open lines (Arc, Curve):** `clamp(start * px, -L, L)` is added to
every `s0_l`, where `L` is the length of the line (Arc: the width `W` of the widest text line; Curve: the length
of the curve). The text can be slid by at most one line length past either end, so the rectangle of the text
stays within about one text length of its anchor and cannot leave the model through this number. **Circle:**
every text line is turned by the angle `start * px / r`; this is periodic, so every value is a valid position on
the circle. The Start control shows the stored value; the clamp is in the layout, because `sanitize_fx` has no
fonts and does not know `L`. Outside an open line the line continues straight along its end tangent, so a long
text is never cut off. `flip` = "Other side".

- **Arc { bend }.** `theta = bend * 2 * pi`. Below 0.001 in size the layout is straight. `W` = widest line,
  `r = W / |theta|`. The circle touches the straight baseline of line 0 in its middle `(0, b0)`. For `bend > 0`
  the centre is below it, `C = (0, b0 + r)`, and `P(phi) = C + r * (sin phi, -cos phi)`,
  `T = (cos phi, sin phi)`; for `bend < 0` the centre is above, `C = (0, b0 - r)`,
  `P(phi) = C + r * (sin phi, cos phi)`, `T = (cos phi, -sin phi)`; the letters stay upright. `phi = s / r_l`
  with `s = 0` at the top (or bottom) point. Text line `l` runs on the concentric circle of radius
  `r_l = r - y_l'` for `bend > 0` and `r + y_l'` for `bend < 0`, where `y_l' = y_l - y_0`, not below a quarter em.
  Alignment in angle: Left `s0_l = -(|theta| / 2) * r_l`, Center `s0_l = -width_l / 2`, Right
  `s0_l = (|theta| / 2) * r_l - width_l`. `flip`: `phi` is negated and `T` reversed, which puts the text on the
  other side of the same circle, reading the other way. Because `r` follows `W`, the text always covers the same
  share of a circle; at bend 0 nothing moves when Arc is switched on.
- **Circle { diameter, center }.** `r = diameter * px / 2`, `C = center * px`. Without `flip` the text stands
  outside the circle, runs clockwise on screen and its reference point is the top of the circle:
  `P(phi) = C + r * (sin phi, -cos phi)`, `T = (cos phi, sin phi)`. With `flip` it stands inside, runs the other
  way and its reference point is the bottom, so it reads upright there (the lower text of a badge):
  `P(phi) = C + r * (sin phi, cos phi)`, `T = (cos phi, -sin phi)`. `phi = s / r_l`; text line `l` runs on the
  circle of radius `r_l = r - y_l'` (with `flip`: `r + y_l'`), not below a quarter em.
  **Alignment is measured against the half circle that is centred on the reference point**, the way the Arc
  measures it against its own span: Left `s0_l = -(pi / 2) * r_l` (the line starts at 9 o'clock), Center
  `s0_l = -width_l / 2` (centred on the reference point), Right `s0_l = (pi / 2) * r_l - width_l` (the line ends
  at 3 o'clock). The 9 and 3 o'clock points belong to the circle, not to the text, so the circle still never
  depends on the text.
  **Creation.** The panel creates the Circle with `diameter = max(1, 2 * W / (pi * px))` em (`W` = the widest
  line; 2 / pi = 0.64), `center = (0, b0 / px + diameter / 2)`, so its top touches the middle of the baseline the
  text had, and `start = 0`. With this diameter the half circle is exactly as long as the widest line, so Left,
  Center and Right all show that line from 9 o'clock over the top to 3 o'clock: **a new Circle is centred on its
  top point and reads upright, whatever Align is.** (The first version of this design used `s0_l = 0` for Left;
  a default, left-aligned text then ran from 12 o'clock down the right side to 6 o'clock. Both reviewers who
  recomputed it were right.) Only when the text is so short that the smallest diameter of 1 em applies, the half
  circle is longer than the text; the panel then also sets `start` to `+((pi / 2) * r - W / 2) / px` for Left and
  to the negative of that for Right, which centres the text; Center needs 0. When the user changes the diameter
  the panel moves the centre by half the change, so the top (with `flip` the bottom) stays where it is.
  **A text longer than its circle.** Typing does not change the diameter, so a line can get longer than its
  circle (`width_l > 2 * pi * r_l`); it then runs over its own start. Nothing is cut and nothing fails. The frame
  reports `circle_fill = max over the lines of width_l / (2 * pi * r_l)`; above 1 the Path page shows its hint
  and the **Fit** button, which sets the diameter by the creation formula for the text as it is now and keeps
  the top point (6.4). Inner lines of a text of several lines reach this sooner, because their circles are
  smaller.
- **Curve { points }.** The points (em, times `px`) are joined by a centripetal Catmull-Rom spline, which has no
  loops and no overshoot between unevenly spaced points. For neighbours `P0..P3` with knots
  `t_i = t_(i-1) + sqrt(|P_i - P_(i-1)|)`, the piece from `P1` to `P2` is the cubic Bezier with control points
  `P1 + m1 / 3` and `P2 - m2 / 3`, where
  `m1 = (t2 - t1) * ((P1 - P0) / (t1 - t0) - (P2 - P0) / (t2 - t0) + (P2 - P1) / (t2 - t1))` and
  `m2 = (t2 - t1) * ((P2 - P1) / (t2 - t1) - (P3 - P1) / (t3 - t1) + (P3 - P2) / (t3 - t2))`. A missing
  neighbour at an end is the end point mirrored; two equal neighbours are 0.001 px apart for this purpose. Two
  points give a straight piece. Each piece is sampled at 32 parameter steps into a table of running chord
  lengths (at most 480 entries for 16 points); a look-up is a binary search, a linear step inside the interval,
  and the exact Bezier point and derivative at that parameter. Alignment: Left `s0 = 0`, Center
  `(L - width_l) / 2`, Right `L - width_l`. Text line `l > 0` uses the sampled polyline moved by `y_l'` along
  `D`, with its own length table. `flip` walks the line backwards (`s -> L - s`, `T -> -T`). The panel creates a
  Curve with three points on the baseline, `(-W / 2, b0)`, `(0, b0)`, `(W / 2, b0)` (each divided by `px`), so
  switching to Curve **moves no letter by more than one raster pixel** until a point is dragged. It is not "no
  pixel": a text on a line uses unrounded glyph origins and baselines and a canvas without the line-box border
  (5.2 steps 3 and 6, 5.10), so letters can shift by up to half a raster pixel, the picture size can change by a
  few pixels and the layer hash changes. The same holds for an Arc at bend 0.

Cost per glyph: one look-up (at most 9 search steps) and one rotation per outline point.

### 5.6 Warp (`core\text\warp.rs`)

```rust
pub struct WarpMap { /* frame, style, bend, distortion box */ }
impl WarpMap { pub fn new(warp: &Warp, frame_min: [f32; 2], frame_max: [f32; 2]) -> WarpMap;
               pub fn map(&self, p: [f32; 2]) -> [f32; 2] }
```

The **warp frame** is the bounding box of all glyph cells after 5.4 and 5.5 (for a straight, unslanted text it is
the line box). Centre `(cx, cy)`, half sizes `a`, `h`. If `a` or `h` is below 0.5 px the warp does nothing.
`u = (x - cx) / a`, `v = (y - cy) / h` (`v = -1` is the top). `b` = bend. `c(t) = 1 - t * t`. Every style maps
`(u, v)` to `(u', v')`; the warped point is `(cx + a * u', cy + h * v')`.

These formulas are ours. They give the shapes the style names describe. They are not the formulas of Photoshop,
which are not public. A reference implementation is `target\round4_design\work\text\warp_check.py`.

| Style | `(u', v')` | In words (positive bend) |
|---|---|---|
| Arc | `Rn = 2a / (abs(b) * pi)`, `n = min(h, 0.75 * Rn)`, `t = a * u / Rn`. For `b > 0`: `r = Rn - n + (1 - v) * h`, `u' = r * sin(t) / a`, `v' = (h - n + Rn - r * cos(t)) / h`. For `b < 0`: `r = Rn - n + (v + 1) * h`, `u' = r * sin(t) / a`, `v' = (-h + n - Rn + r * cos(t)) / h`. `abs(b) < 0.0001`: unchanged. | A fan between two circle arcs; a half circle at 100 %. The middle line keeps its length while the frame is at least 2.1 times the bend as wide as high (2.1 times at 100 %); in a shorter frame the line that keeps its length moves toward the inner edge, so the inner edge never shrinks below a quarter of its length. |
| Arc Lower | `u' = u`, `v' = v + b * c(u) * (1 + v) / 2` | The top edge stays, the bottom edge bows down in the middle. |
| Arc Upper | `u' = u`, `v' = v - b * c(u) * (1 - v) / 2` | The bottom edge stays, the top edge bows up in the middle. |
| Arch | `u' = u`, `v' = v - b * c(u)` | Both edges rise in the middle by half the frame height; letters stay upright. |
| Bulge | `u' = u`, `v' = v * (1 + 0.5 * b * c(u))` | Top up and bottom down in the middle. |
| Shell Lower | `u' = u`, `v' = v + b * u * u * (1 + v) / 2` | The top edge stays, the bottom edge drops at both ends. |
| Shell Upper | `u' = u`, `v' = v - b * u * u * (1 - v) / 2` | The bottom edge stays, the top edge rises at both ends. |
| Flag | `u' = u`, `v' = v - 0.5 * b * sin(pi * u)` | One full wave along the text; letters stay upright. |
| Wave | `u' = u + 0.1 * b * sin(pi * v)`, `v' = v - 0.5 * b * sin(pi * u)` | A flag that also sways sideways. |
| Fish | `u' = u`, `v' = v * (1 + 0.5 * b * sin(pi * u))` | Taller at three quarters of the width, flatter at one quarter. |
| Rise | `u' = u`, `v' = v - 0.25 * b * (3 * u - u^3)` | The left end sinks, the right end rises, along an S. |
| Fisheye | `q = u * u + v * v`; for `q < 1`: `g = 1 + 0.4 * b * (1 - q)`, `u' = u * g`, `v' = v * g`; else unchanged | The middle is magnified, the border stays. |
| Inflate | `u' = u * (1 + 0.25 * b * c(v))`, `v' = v * (1 + 0.5 * b * c(u))` | All four edges bow outward. |
| Squeeze | `u' = u * (1 - 0.5 * b * c(v))`, `v' = v * (1 + 0.5 * b * c(u))` | The sides are pulled in, top and bottom pushed out. |
| Twist | `r = min(1, sqrt(u * u + v * v))`, `al = b * (pi / 2) * (1 - r)^2`, `(u', v')` = `(u, v)` turned by `al` | The middle is turned by up to 90 degrees, the border stays. |

**Direction Vertical** (`vertical_axis`): the point is turned by -90 degrees about the frame centre, the style is
applied with the half sizes exchanged, and the result is turned back:
`(p, q) = style(y / h, -x / a; half sizes h, a)`, result `(-q * a, p * h)` (x, y relative to the frame centre).

**Distortion**, applied after the style. Let `B` be the bounding box of the warped frame border (each side
sampled at 65 points; the image of the frame lies inside the image of its border), with centre `(bx, by)` and
half sizes `ba`, `bh`. For the warped point `(xs, ys)`: `us = (xs - bx) / ba`, `vs = (ys - by) / bh`,

`x'' = bx + (xs - bx) * (1 + 0.4 * vertical * vs)`,   `y'' = by + (ys - by) * (1 + 0.4 * horizontal * us)`.

Its Jacobian determinant is `1 + 0.4 * (horizontal * us + vertical * vs)`, at least 0.2 inside the box.

**Checked for this document** with the reference script (pure Python, about 2 minutes): for all 15 styles, both
directions, bends -100, -75, -50, -25, 25, 50, 75, 100 %, frames from 1:4 to 16:1, a 64 x 64 grid of the frame
with the border included, every one of the 8,192 mapped triangles keeps a positive signed area: at least 0.23 of
its original area with bend alone (Fisheye; Arc 0.25), at least 0.10 with both distortions at -100, 0 or 100 %
(Shell Lower and Shell Upper 0.10, Fisheye 0.10, Inflate 0.11, Arc 0.14). No style folds the text over itself.
Bend 0 with both distortions 0 is the identity for every style. The same test is unit test
`no_style_folds_over` (section 8.1).

Three formulas of the proposals did **not** pass a strict test and are replaced above: Squeeze of the winning
proposal (its width slope is 0 at the left and right frame edge at bend -100 %), Bulge of the winning proposal
(collapses the middle column at bend -100 %), and Fisheye with a factor of 0.5 or more (the engine proposal;
slope 0 or below on the border at 100 %). A distortion factor of 0.9, or 0.5 on both axes, folds a corner; hence
0.4.

**Outline refinement for a warp.** A warp turns straight outline pieces into curves, so before mapping each
flattened edge is cut into equal pieces of at most 8 px, and then a piece `(A, B)` with middle `M` is halved
while `|W(M) - (W(A) + W(B)) / 2| > 0.1 px` or while the mapped piece `|W(B) - W(A)|` is longer than 16 px, at
most 6 times. The mapped outline then deviates from the true curve by about 0.1 px at most, also where the warp
magnifies. The glyph cell corners go through the same map, because the canvas of 5.10 needs them.

Cost: about three map evaluations per final edge, each 10 to 40 floating-point operations and at most two
sine or cosine calls.

### 5.7 Coverage rasterizer (`core\text\scan.rs`)

Signed-area accumulation, the method `fontdue` uses, on closed polylines only:

- One `f32` buffer of `(w + 2) * h` cells per glyph, sized to the bounding box of the glyph's mapped outline,
  grown by one pixel and by the bold radius.
- Every edge is clipped to the rows of the buffer. For each row it crosses, the signed height it covers in that
  row is spread over the cells it touches (the exact trapezoid area per cell), so that a running sum along the
  row gives the signed area covered up to each pixel.
- Coverage of a pixel = `min(|running sum|, 1)`, stored as `round(255 * coverage)`.

An area with winding number 1 or more is full; an area where opposite windings cancel is empty: the rule
`fontdue` applies to the same outlines. There is **no summing of a glyph with a stroked copy of itself**
anywhere; thickening is done on the finished coverage (5.9).

Glyphs are independent. They are taken in reading order into batches of at most 64 glyphs and at most 8
megapixels of glyph boxes, each batch is rasterized (and thickened) in parallel with rayon, and the results are
merged in glyph order on one thread. The picture does not depend on the number of threads (ARCH:415-417), and
the working memory of one batch stays below about 110 MB (8 megapixels times 4 bytes of sums, 1 byte of coverage
and 8 bytes inside the distance transform).

### 5.8 Preview quality: the same rectangle with fewer pixels

`render_text(.., RasterQuality::Preview { max_pixels })` for a styled text:

1. All geometry is computed at the resolution of the stamp, exactly as for `Full`, including the roundings of
   5.2 and the canvas `(w, h)` of 5.10.
2. `f` = the largest of 1, 1/2, 1/4, 1/8, 1/16 with `w * h * f * f <= max_pixels`. If `f = 1` the result is the
   full picture and `full = true`.
3. The preview picture is `wp = ceil(w * f)` by `hp = ceil(h * f)` pixels. Every outline point is multiplied by
   `(wp / w, hp / h)` just before the coverage rasterizer; bold radius and outline width are multiplied by `f`.
4. `TextRaster.frame` is the **full** frame (size `(w, h)`), `full = false`.

Because the picture is mapped onto the placement by its relative coordinates (section 3.3), the preview covers
exactly the rectangle the full picture covers: the placement does not depend on the preview resolution, and
nothing moves when the final picture replaces the preview. The difference between `wp / w` and `hp / h` is below
one pixel in the picture width.

Use: `set_stamp_source(.., Commit::Preview)` and every `Preview` bake of a styled text use
`Preview { max_pixels: 1 << 20 }` (1 megapixel); the hover picture uses `1 << 18`. A `Final`, a flush and a
`Full` bake use `Full`. The cache entry of a stamp records `full`; a bake at `BakeQuality::Full` that finds a
preview picture rasterizes again. The classic painter has no preview mode: a plain text, also one with gap
values, costs per key what it costs today.

### 5.9 Bold, and what the B and I buttons do

**Thickening.** After a glyph is rasterized, if `r = bold_em * px` (times `f` in a preview) is at least 0.25 px:
`d = distance_to_ink(coverage)` on the own buffer of the glyph (`core\text\edt.rs:23`, the function the outline
uses today) and `coverage = max(coverage, 255 * clamp(r + 0.5 - d, 0, 1))`. The shape grows by `r` on every side
with round corners. It cannot produce holes or spikes, works the same for TrueType, CFF and composite glyphs,
and runs after the warp, so the gain is even across a warped text.

**Buttons.** Bold and italic are not stored as flags. **The truth is the font of the text plus `fx`:** the weight
and the italic flag of the face in use are read from the font bytes of the text, which the document always has,
and never from the list of installed fonts.

```rust
// core\text\font_catalog.rs
pub struct FaceTraits { pub weight: u16, pub italic: bool, pub stretch: u16 }
pub fn face_traits(bytes: &[u8], face_index: u32) -> Option<FaceTraits>;   // OS/2 weight, width, style; `post` italic angle
// app\panels\text_block.rs (pure functions)
pub struct FaceChoice { pub source: FaceSource /* Catalog(fontdb::ID) | Project(FontRef) */, pub family: String,
                        pub style: String, pub traits: FaceTraits }
pub fn candidates(fonts: &FontCatalog, doc: &Document, family: &str) -> Vec<FaceChoice>;
pub fn resolve_face(faces: &[FaceChoice], current: &FaceTraits, want_bold: bool, want_italic: bool,
                    bold_em: f32, slant_deg: f32) -> Option<(usize /* face */, f32 /* bold_em */, f32 /* slant_deg */)>;
```

- `face_traits` reads the two tables fontdb reads, with the same rule (italic = OS/2 style is not normal, or the
  italic angle of `post` is not 0; section 3.1), so for an installed font it returns what the catalog says. The
  panel keeps the result per `(BlobId, face_index)` in `TextPanelState`; a parse reads only the table directory
  and two small tables.
- **Is bold** = `traits.weight >= 600` or `bold_em > 0`. **Is italic** = `traits.italic` or `slant_deg != 0`. B and
  I are lit accordingly. This is the same on every machine, because it does not ask whether the family is
  installed: a text that uses a real Bold face shows B lit also where that family is missing.
- `candidates` = the faces of the family in the catalog **and** the fonts of that family inside the open project
  (the fonts of its text stamps, 5.13), without duplicates (same family, style and face index: the project's copy
  wins). So a face that is only embedded can be chosen, and a family that is not installed still offers the faces
  the project has.
- `resolve_face` picks, among the candidates with the wanted italic flag (if there is none: with the other flag)
  and the stretch of the current face (if several exist), for `want_bold` the face with a weight of 600 or more
  nearest to 700, else the face with a weight of 500 or less nearest to 400; if no face of the wanted weight
  class exists, it stays in the other class. Then the two amounts follow from what the chosen face lacks:
  `bold_em` = 0 when the face is bold or bold is not wanted, else the old `bold_em` if it was above 0, else
  `FAUX_BOLD_EM`; `slant_deg` = 0 when the face is italic or italic is not wanted, else the old `slant_deg` if it
  was not 0, else `FAUX_SLANT_DEG`. So the rule "the real face first, thicken or slant only what is missing"
  holds after every step, and the two can never add up by accident.
- It runs at three moments, always with the wishes read **before** the change:
  1. a click on **B** (`want_bold` = not "is bold"; `want_italic` = "is italic") or on **I** (the mirror);
  2. **a change of the family** in the font picker (`want_bold` = "is bold", `want_italic` = "is italic" of the
     text as it was). Example: Ubuntu with B is `bold_em = 0.02` on the face "Light"; picking Arial gives the
     face "Arial Bold" and `bold_em = 0`, not a thickened Arial Regular. Picking Ubuntu again gives "Light" and
     `bold_em = 0.02`;
  3. never on a pick in the **style list**: the picked face is taken as it is. Only this is cleaned up: `bold_em`
     becomes 0 when the picked face is bold, `slant_deg` becomes 0 when it is italic.
- Switching B off when no face of 500 or less is among the candidates (the family has only bold faces, or the
  text uses an embedded Bold face and the regular face is neither installed nor in the project): if `bold_em > 0`
  it becomes 0; otherwise nothing changes, B stays lit, and the hover text says "No regular face of {family} is
  installed or in this project". A click never adds thickening to a face that is bold already.
- The hover text names what the button did: "Bold (Arial Bold)" or "Bold (thickened: Ubuntu has no bold face)".
- For the text to place (the draft) the same functions run on `tools.text.family`, `style`, `bold_em` and
  `slant_deg`; the traits of the draft's face come from the candidate list.

### 5.10 Merge, outline, compose, canvas and frame (`core\text\styled.rs`)

- Canvas: `R` = bounding box of all mapped outline points (grown by the bold radius), of all mapped glyph cell
  corners, and, for a text without a path, of the mapped border of the line box (16 points per side). Cells and
  line box mean that spaces count, an empty text is one space wide (as today, `core\text\layout.rs:69`), and the
  picture does not change size between "a" and "A". `pad = ceil(outline width in px) + 2`. The canvas runs from
  `floor(R.min) - pad` to `ceil(R.max) + pad`. A side above 8192 px is `TextError::TooLarge`. **So is an area
  above `MAX_STYLED_PIXELS` = 16,777,216 pixels** (a square of 4096 px). The side limit alone does not bound a
  styled picture: a straight line is a strip (16 em at 512 px per em is 8192 x about 600 px, 5 megapixels), but
  a text around a circle has a square picture, up to 8192 x 8192 = 67 megapixels, which would cost about 1 GB of
  working memory with an outline. The classic painter keeps the limit it has today (the side only), because a
  plain text must not start to fail or change resolution in a case that works today. `align_px` and
  `circle_fill` of the frame are set as defined in 4.1.
- `anchor_px` = minus the canvas origin (the anchor is the origin of text space). For a straight text without
  overhang the canvas is the line box plus `pad`, the same picture size as the classic painter; with overhang the
  classic canvas is larger by the overhang on all four sides (section 3.3) while the styled one is tight. The
  anchor rule of 5.11 absorbs the difference, so the ink does not move.
- Merge: one coverage byte per canvas pixel; a glyph pixel replaces the canvas pixel when its coverage is larger
  (the maximum rule of today, `core\text\raster.rs:71-73`).
- Outline: `distance_to_ink` on the merged coverage and `clamp(ow + 0.5 - d, 0, 1)`, as today
  (`core\text\raster.rs:77-91`). It is computed after warp and line, so it has the same width everywhere.
- Compose: fill `(color.rgb, color.a * coverage)` over the outline with `fill_over_outline`
  (`core\text\raster.rs:18-31`), rows in parallel from 65,536 pixels up as today.

### 5.11 The anchor rule and the document side (`core\text\frame.rs`, `core\doc_stamps.rs`)

**Frame of a placement** in its host space (world units in 3D, texels in 2D): centre `c`, full size `(W, H)`,
unit direction `ex` of the picture's x axis and `ey` of its y axis.
- Projected: `(right, up, _) = p.axes()`; `ex = right`, negated when `flip_x`; `ey = -up`, negated when
  `flip_y`; `W = 2 * half_size.x`, `H = 2 * half_size.y`, `c = origin` (this is the mapping of
  `core\stamp_bake.rs:341-348`).
- UV: `ex = (cos r, sin r)`, `ey = (-sin r, cos r)`, each negated by its flip; `(W, H) = size_px`,
  `c = center_px` (`core\stamp_bake.rs:162-165`, `:693-701`).

A picture of `w x h` pixels in this rectangle has the pixel size `k = (W / w, H / h)`, and its pixel point `q`
lies at `P(q) = c + ex * (q.x - w / 2) * k.x + ey * (q.y - h / 2) * k.y`.

**The rule.** When the full-resolution frame of a text stamp changes from `(w0, h0)`, kept point `a0`, `S0` px
per em to `(w1, h1)`, `a1`, `S1`. The **kept point** is the anchor (`anchor_px`), except for a change of spacing
alone, where it is `align_px` (see "Which point is kept" below):

```
k1 = k0 * S0 / S1                                  (per axis: a stretch made with Shift + corner drag is kept)
W1 = w1 * k1.x,   H1 = h1 * k1.y
c1 = c0 + ex * ((a0.x - w0 / 2) * k0.x - (a1.x - w1 / 2) * k1.x)
        + ey * ((a0.y - h0 / 2) * k0.y - (a1.y - h1 / 2) * k1.y)
```

Two things are preserved exactly: the host size of one em (`S * k`, per axis) and the host position of the kept
point (`P1(a1) = P0(a0)`). The size part is the "text rescale rule" of today (ARCH:2156-2160) and is computed
with the expressions of today (`core\doc_stamps.rs:370`, `:704-707`); when both kept points lie exactly in the
picture centres the centre is not touched at all. So a content edit of a plain text gets bit-identical
placements.

**Which point is kept.**
- **A change of spacing alone**: old and new spec differ only in `letter_spacing` and / or `fx.gaps`
  (`differs_only_in_spacing`), and neither has a warp or a path. The kept point is `align_px`: the left edge of
  the line box for Left, its centre for Center, its right edge for Right, at the height of the anchor. Glyph
  origins are laid out from the left edge of the line box, and the box is `ceil(widest line)` wide, so for a
  left-aligned text every letter before the changed gap keeps its place on the model exactly, and for a
  right-aligned text every letter after it within one raster pixel. This is what the workflow promises (1.1
  step 7) and what kerning does in Photoshop. Number: a gap of 30 % in a left-aligned text whose em is 100 texels
  moves the letters after the gap by 30 texels and the letters before it by 0. With the centre as the kept point
  (the first version of this design, and what Letter spacing does today through the bare `rescale`,
  `core\doc_stamps.rs:55-63`, `:367-371`) the letters before the gap would move 15 texels to the left, and a text
  lined up with a feature of the model would lose that on every spacing edit.
- **With a warp or a path** a spacing change keeps the anchor: there the wider text changes the bend of every
  letter (the warp frame follows the text, an Arc keeps its share of a circle), so no letter can stay exactly
  where it was, and the anchor is the stable choice.
- **Every other change** (content, font, size, effects) keeps the anchor. In particular typing into a plain text
  grows it about its centre as today; whether typing should keep the alignment side too is question Q8.

In 3D the kept point lies in the projector plane and the centre moves inside that plane. For a spacing edit of a
left-aligned text the centre moves by half the added width. In 3D the
centre moves inside the projector plane; the automatic depth is re-fitted afterwards as today
(`core\doc_stamps.rs:132-139`, `:374`, `:714`).

**Where it runs.** In all three places where the picture of a text changes:
1. `set_stamp_source` (`S0 = S1`), instead of the bare `rescale` at `core\doc_stamps.rs:367-371`.
2. The **too-large fallback**, new inside `set_stamp_source` and `add_stamp`: when the frame of the new spec at
   the resolution of the stamp exceeds 8192 px on a side **or, for a styled text, 16.8 megapixels in area**
   (5.10), the largest of `S / 2`, `S / 4`, ... down to 32 whose frame fits both limits is taken (each try is one
   `text_frame` call, no pixels), `spec.size_px` becomes that value and the rule runs with `S1 < S0`. Only when 32
   px per em does not fit either the error reaches the user ("The text is too long"). This is the one case in
   which a `Preview` changes `size_px`; ARCH:2094 gets this exception. Today such an edit simply fails (section
   3.2), and warps and lines reach the limits sooner than typing does. What fits at 32 px per em: a line of 256
   em, or a square of 128 x 128 em; every Circle the Diameter control allows (100 em plus two line heights is
   about 3,300 px, 10.6 megapixels) and every Curve inside the coordinate range of 4.1.
3. `retune_text` (`core\doc_stamps.rs:688-733`), where `S1 != S0`. Its trial rasterizations (`:701`,
   `:722-730`) are replaced by `text_frame` calls; pixels are made once, for the resolution that fits.

Applying the rule again and again cannot let the anchor drift: every application preserves `P(a)` up to float
rounding (test in 8.1: 20 resolution changes, drift below 0.01 texel).

**New and changed document functions.**

```rust
// core\text\frame.rs (pure)
pub enum Keep { Anchor, AlignEdge }
pub fn refit_placement(placement: &mut Placement, old: &TextFrame, new: &TextFrame, keep: Keep);   // the rule above
pub fn place_anchor_at_centre(placement: &mut Placement, frame: &TextFrame);  // moves the rectangle so that the anchor lies where its centre was
pub struct EmMap { .. }          // em of text space <-> host space of one placement and frame
impl EmMap { pub fn new(placement: &Placement, frame: &TextFrame) -> EmMap;
             pub fn to_world(&self, em: [f32; 2]) -> Vec3;  pub fn to_texel(&self, em: [f32; 2]) -> Vec2;
             pub fn plane(&self) -> (Vec3, Vec3);            // a point and the normal of the projector plane
             pub fn from_world(&self, p: Vec3) -> [f32; 2];  pub fn from_texel(&self, px: Vec2) -> [f32; 2] }

// core\doc_stamps.rs
pub fn stamp_text_frame(&mut self, id: StampId) -> Result<TextFrame, EditError>;   // geometry only; fills the frame cache
pub fn cached_text_frame(&self, id: StampId) -> Option<TextFrame>;                 // None when not cached for the current spec
pub fn stamp_source_frame(&mut self, source: &StampSource) -> Result<TextFrame, EditError>;   // for a text not placed yet
pub fn text_preview(&mut self, spec: &TextSpec, max_pixels: u32) -> Result<Arc<TextRaster>, EditError>;  // one-entry cache
```

- A new document field `text_frames: HashMap<StampId, (TextSpec, TextFrame)>` holds the frame of the current
  spec. `cached_text_frame` compares the stored spec with the stamp's spec, so a stale entry after undo is simply
  not returned; `history.rs` is not changed. Entries of stamps that no longer exist are removed when a frame is
  inserted.
- **After a project load and after an undo** the picture cache is empty (section 3.3). The frame is then
  rebuilt by `stamp_text_frame`, which the Text block of the panel calls for the selected text and the Transform
  tool calls in `hover` for a selected text with a Curve. It is a geometry pass without pixels, on the UI thread:
  layout only for a plain text; layout, outlines and warp for a styled one (estimate: under 2 ms for 200
  letters; limit in 8.4: 5 ms). No rasterization is needed for the path points or the Text size field.
- `set_stamp_source` takes the old frame from `stamp_text_frame` instead of rasterizing the old spec just to
  learn its size (`core\doc_stamps.rs:347`).
- `text_raster_px` reads the height from the frame cache instead of the mip size (`core\doc_stamps.rs:566-569`).
- Whether a cached picture is a preview is stored in a new document field
  `text_preview_ids: BTreeSet<StampId>` next to `text_mips`, so the type of `text_mips` (`core\document.rs:69`)
  and the line of `core\history.rs:727` that reads it stay as they are. If round 3 (R3-4) has changed how the
  picture is cached, the rule is only this: it must be known for every cached picture whether it is full.
- `text_fonts` builds `TextFonts` from the font of the text and the fallback, as today; `add_stamp`,
  `set_stamp_source`, `stamp_source_size`, `stamp_source_frame` and `text_preview` call `sanitize_fx` on their
  spec first.
- A change of gap values alone keeps the text on the classic painter. `set_stamp_source` sees that the specs
  differ only in spacing and calls `refit_placement(.., Keep::AlignEdge)`. For a centred text `align_px` is the
  picture centre and the result is the rescale of today, bit for bit.
- `Document::find_font(family, style) -> Option<FontRef>` (new, read only): a font of the open document with
  that family (compared without regard to case) and style: first among the fonts of the text stamps, then among
  the fonts `register_font` handed out earlier in this session (a small list `font_refs` in the document, not
  saved, not part of the history; an entry whose blob no longer exists is skipped). Section 5.13 uses it.
- The Text tool places the **anchor**, not the rectangle centre, at the click: it builds the placement as today
  from `stamp_source_frame` (`app\tools\stamp_place.rs:93-114`, `:173-203`) and then calls
  `place_anchor_at_centre`. For a plain text this changes nothing.

### 5.12 Keeping gap values with their characters (`core\text\gaps.rs`, pure functions)

```rust
pub fn get(gaps: &[Gap], after: u32) -> f32;                          // 0.0 without an entry
pub fn set(gaps: &mut Vec<Gap>, after: u32, em: f32);                 // clamps; 0.0 removes the entry
pub fn insert(gaps: &mut Vec<Gap>, at: u32, n: u32);                  // n characters were inserted before index `at`
pub fn delete(gaps: &mut Vec<Gap>, range: std::ops::Range<u32>);      // the characters of `range` were removed
pub fn shift_for_edit(gaps: &mut Vec<Gap>, old: &str, new: &str);     // one delete and one insert from a text comparison
pub fn is_joined(ch: char) -> bool;                                   // combining mark, variation selector, ZWNJ, ZWJ
pub struct GapSlot { pub after: u32, pub left: char, pub right: Option<char>, pub em: f32 }   // right None: end slot
pub fn slots(content: &str, gaps: &[Gap]) -> Vec<Vec<GapSlot>>;       // per text line: the gaps that exist, in reading order
pub fn slot_name(ch: char) -> String;                                 // "E", "(space)", "(tab)": how texts name a character
```

A gap value belongs to the character on its left. From this one rule:
- `insert(at, n)`: every entry with `after >= at` moves by `n`. An entry with `after = at - 1` stays: text typed
  into a gap appears to the right of the gap value, and the character on the left keeps it.
- `delete(a..b)`: entries with `a <= after < b` are removed (their character is gone); entries with
  `after >= b` move down by `b - a`. An entry with `after = a - 1` stays and now acts between that character and
  whatever follows.
- Replacing a selection and pasting are a `delete` followed by an `insert`, which is exactly what egui calls.
- `slots` lists, per text line, the pairs of neighbouring laid-out characters (a tab is one character) with the
  index of the left one. It leaves out a pair whose right character `is_joined` and a pair whose left character
  is U+200D, so no gap can be set inside an accented letter or a joined emoji. `is_joined` is a fixed list of
  ranges, because the standard library has no character classes and no new crate is wanted: U+0300-036F,
  U+1AB0-1AFF, U+1DC0-1DFF, U+20D0-20FF, U+FE20-FE2F (combining marks), U+FE00-FE0F and U+E0100-E01EF (variation
  selectors), U+200C and U+200D. Marks of other scripts are not covered; complex scripts are left out anyway
  (9.2).
- The last character of a line has no neighbour and normally no slot. **If it carries a stored value**, `slots`
  adds one end slot for it (`right: None`), so the value is visible and can be reset on its own (6.2 rule 1).
  The panel strip and the tests use `slots`.

The content field of the app edits a `GapBuffer`, which implements the `TextBuffer` trait of egui
(`REG\egui-0.37.0\src\widgets\text_edit\text_buffer.rs:26-70`, `:241`):

```rust
pub struct GapBuffer { pub text: String, pub gaps: Vec<Gap>,
                       history: Vec<(String, Vec<Gap>)>, at: usize }   // states of this focus session; `at` = the current one
```

- `insert_text` and `delete_char_range` receive exact character positions and call `insert` and `delete`. After
  each of them the new state `(text, gaps)` is stored: everything after `at` is dropped, the state is appended
  and `at` points to it. The list starts with the state the field had when it got the focus, holds at most 200
  states (the oldest is dropped; egui keeps at most 100 undo points,
  `REG\egui-0.37.0\src\util\undoer.rs:29`) and is emptied when the field loses the focus or shows another stamp.
- `replace_with(new)` is what egui calls for the undo and the redo of the field, with a text and nothing else
  (section 4.4). The buffer looks for a stored state whose text is exactly `new`: first backward from `at` (an
  undo), then forward (a redo). **Found:** text and gap list of that state are restored and `at` moves there.
  **Not found** (not expected; a safety net): `shift_for_edit`, which finds the longest common prefix and suffix
  of the old and new text and applies one `delete` and one `insert` for the middle.
- Example, the case a reviewer walked through: "WTE 2026" with a gap of 0.3 em after "E". Select "TE", press
  Delete: the entry is removed with its character (rule above). Ctrl+Z in the field: egui restores the text "WTE
  2026"; the buffer finds the first state with that text and restores its gap list: the gap after "E" is back.
  With `shift_for_edit` alone it would have been lost without a notice, and the `Final` sent when the field
  loses the focus would have committed the text without it.
- When the same text occurred more than once in the session with different gap lists, the nearest earlier state
  wins: the gap values the user saw the last time the field showed that text.
- `shift_for_edit` is used **only** as that safety net and as the fallback of 6.2: among equal neighbours it
  cannot tell where a key was typed ("a" typed into "aaaa"), and a gap value would slip by one letter.

Cost: linear in the number of gap entries per key, plus one copy of the text and the gap list per key for the
history (a text of 1,000 characters: 200 states are under 1 MB).

### 5.13 Font list and font memory (`core\text\font_catalog.rs`, `core\blob.rs`)

- `refresh_families` sorts by `to_lowercase()`, ties by the original string.
- `faces(family)` no longer drops a face. The first face with a style name keeps the plain name, so every
  `FontRef.style` in existing projects still resolves (`find` compares the same strings, `:136-138`). A later
  face with the same name gets `"<style> (<PostScript name>)"`, for example "Regular (Candara-Light)", from
  `FaceInfo.post_script_name` (`REG\fontdb-0.24.0\src\lib.rs:845`). `FontFaceInfo` gains `post_script_name`,
  `stretch` (`:854`) and `file_bytes: u64`. fontdb has no size field for a font file, and `faces()` is rebuilt on
  every call (section 3.1), so the sizes are **read once**: `load_system` and `bundled` fill a map
  `file_sizes: HashMap<PathBuf, u64>` in the catalog (one `fs::metadata` per font file, about 150 on this
  machine; a bundled font has its byte length), and `faces()` only looks the path of the face up in it.
- `load_system` adds one pass over the same three folders for files whose extension matches `ttf`, `ttc`, `otf`,
  `otc` in mixed case, which fontdb skips, and loads them with `load_font_file`.
- Search (app side): case-insensitive substring of the family name; families that start with the search text
  first, then the others, each group in list order.
- Samples: `FontCatalog::sample(id, text, px) -> Option<(u32, u32, Vec<u8>)>` draws "Abc 123" (or, when the face
  has none of these characters, its first six mapped characters) at 16 px per em with 5.3 and 5.7 into a coverage
  bitmap of at most 128 x 20 px, reading the font through fontdb's `with_face_data`
  (`REG\fontdb-0.24.0\src\lib.rs:723`) without copying the file and without a `fontdue` parse. **Characters are
  mapped the way the painter maps them**, with a helper `outline::char_to_glyph(face, ch)` that walks every cmap
  subtable in file order and keeps the last mapping that is not glyph 0: this is the table `fontdue` ends up
  with, because it reads every subtable and a later insert overwrites an earlier one (section 3.1).
  `Face::glyph_index` is **not** used: it skips subtables that are not Unicode, and Wingdings, Webdings, Symbol
  and Marlett on this machine have only such subtables (platform 1 encoding 0 and platform 3 encoding 0;
  measured by a reviewer from the cmap headers, not re-measured). With it "Abc 123" in Wingdings would draw
  Wingdings glyphs in the text but other characters in the sample. A second non-modal job, started the first
  time the picker opens, makes one sample per family (the face `pick_style` would choose,
  `app\panels\tool_options.rs:281-291`). The app packs them into atlas textures of 512 x 1024 px (204 samples
  each).
- **Where a picked font comes from: the project first, then the machine.** One function in the app,
  `text_block::resolve_font(state, family, style) -> FontRef`, replaces the look-up of today in `font_ref_for` and
  in `text_source` (section 3.1):
  1. `doc.find_font(family, style)` (5.11): a font the open project already holds under these names. Its
     `FontRef` is reused as it is. No file is read, nothing is copied, nothing is hashed.
  2. Else the catalog: `fonts.find(family, style)`, `face_bytes`, `doc.register_font` (one read of the file and
     one blob insert, once per font and document).
  3. Else the bundled font with the toast of today.
  Rows of "In this project" in the picker carry the `FontRef` of their font and hand it over directly; the style
  list of a family is the candidate list of 5.9 (project faces and installed faces). Consequences: a font that
  is embedded in a project but not installed on this machine can be given to a second text and to the text to
  place (the draft keeps the two names, and step 1 finds them); and the project's copy of a font wins over the
  installed file of the same name, so two texts of one project in "the same font" always use the same bytes.
- **Font memory.** The costly pattern of today is that hovering and font browsing copy the whole font file and
  hash it on every step (section 3.1). The first version of this design tried to cure that with a weak reference
  in `face_bytes` alone. That fails in the normal case: after a project was opened the document holds the font
  bytes in its own allocation, `BlobStore::insert` returns the old id and drops the `Arc` it was given
  (`core\blob.rs:40-47`), the weak reference dies at once and the next call copies and compares the file again.
  The cure is step 1 above: **the copy is avoided before it happens**, because a font the document knows by name
  is never asked from the catalog again. What remains is one copy and one hash per font and document, at the
  first use.
  Two small things are kept for that first use: `face_bytes` holds a weak reference **per font file** (keyed by
  the path of the source, not by the face), so the faces of one collection file share one allocation while the
  document holds it (the largest collection here is 37 MB, mingliub.ttc, measured by a reviewer), and
  `BlobStore::insert` returns at once when a blob with the same allocation exists (`Arc::ptr_eq`), before
  hashing. A font that was tried but is not used stays in memory until the document closes, as today: the rule
  of `core\blob.rs:28-30` (undo may still need it) is kept, and unused font blobs are **not** freed in this
  round.

### 5.14 The hover picture

The Text tool's `hover` (which has `&mut AppState`) asks `Document::text_preview(&spec, 1 << 18)` when its source
key changed (`app\tools\stamp_place.rs:149-170`) and keeps the `Arc<TextRaster>`. The tool's `overlay` has only
`&self` and a painter (section 3.6), so the egui texture lives in a
`Mutex<Option<(usize /* address of the raster */, egui::TextureHandle)>>` inside `StampPlaceTool` and is uploaded
from `overlay` through `painter.ctx()` when the raster changed. The `Tool` trait and `app\tools\mod.rs` are not
changed. The picture is drawn at 70 % opacity as a textured mesh: in the 2D view the rotated rectangle, in the 3D
view the placement rectangle on the plane that touches the surface under the pointer, cut into 8 x 8 cells whose
corners are projected to the screen. It shows font, size, angle, warp and line; it does not show how the text
will wrap around the surface (the bake after the click does that). `add_stamp` reuses the cached preview when it
is `full` and its spec equals the placed one. This item is first on the cut line (section 7.6).

For the tests the tool has one read-only accessor, `hover_preview() -> Option<(&TextSpec, &Arc<TextRaster>)>`: the
spec the cached picture was made for and the picture. The picture has at most `1 << 18` = 262,144 pixels unless
the full picture is smaller than that, in which case it is the full picture (`full` is true). The egui texture is
not part of what the unit tests check (they have no painter); the scenario `text_tab` takes a screenshot with the
picture on screen (section 8.3).

### 5.15 Cost against the budgets

All numbers are estimates from operation counts for this class of machine (12 logical cores). None was measured.
The first implementation step measures the plain text of today, and section 8.4 sets the limits.

Picture sizes. "WTE 2026" is taken as about 4.9 em wide (an estimate; not measured in a font); the warp frame is the box of the glyph cells, about 2,500 x 580
px at 512 px per em (half sizes a = 1,250, h = 290). For the Arc warp at 50 % section 5.6 gives `Rn = 1.273 a` =
1,591 px, a width of `1.414 * (Rn + h)` = 2,660 px and a height of `h + Rn - (Rn - h) * cos 45 deg` = 961 px:
**2.6 megapixels**, preview at 1/2: 0.64 megapixels. (The first version of this table used 2,480 x 680 px, which
a reviewer showed to be too small by a factor of 1.5 in area.) The per-pixel steps scale with these areas at
about 1.8 ms (merge and compose), 2.4 ms (mip chain) and 41 ms (outline) per megapixel.

| Step | A: Preview of "WTE 2026", Arc warp 50 %, stamp at 512 px per em (1,330 x 480 px) | B: Final of A (2,660 x 960 px, 2.6 megapixels) | C: Final, 200 letters in 4 lines, Flag, 128 px per em (about 3,200 x 620 px, 2.0 megapixels) | D: Final, "WTE 2026" on its default Circle, 512 px per em (the text covers the upper half: about 2,550 x 1,430 px, 3.6 megapixels) | E: Final, a line of 16 em around a full Circle, 512 px per em (about 3,560 x 3,560 px, 12.7 megapixels) |
|---|---|---|---|---|---|
| Layout and frame | under 0.05 ms | under 0.05 ms | 0.1 ms | under 0.05 ms | 0.05 ms |
| Outlines, slant, line, warp refinement | 0.3 ms | 0.3 ms | 2 ms | 0.2 ms | 0.6 ms |
| Coverage (parallel per glyph) | 0.5 ms | 2 ms | 2 ms | 2 ms | 6 ms |
| Thickening, if used (parallel per glyph) | 3 ms | 10 ms | 10 ms | 10 ms | 30 ms |
| Merge and compose | 1.1 ms | 4.5 ms | 3 ms | 6.5 ms | 23 ms |
| Outline, if used (as today, one pass over the canvas) | 26 ms | 105 ms | 80 ms | 150 ms | 520 ms |
| Mip chain (as today) | 1.5 ms | 6 ms | 5 ms | 9 ms | 30 ms |
| **Sum without thickening and outline** | **about 4 ms** | **about 13 ms** | **about 12 ms** | **about 18 ms** | **about 60 ms** |

Column D is not a square: a text over the upper half of a circle has a picture about as wide as the diameter
plus two line heights and about half as high. Column E is the square case, a text that goes all the way round.

- **The square pictures of a Circle are the expensive case**, and the area limit of 5.10 is what bounds them.
  The largest styled picture, 16.8 megapixels, costs about 30 ms merge and compose, 40 ms mip chain and, with an
  outline, about 0.7 s for the distance transform, once per `Final`, on the UI thread. Peak working memory at
  the limit: picture 67 MB (4 bytes per pixel), mip chain 22 MB, merged coverage 17 MB, and with an outline 134
  MB inside the distance transform (8 bytes per pixel, section 3.3): **about 240 MB with an outline, about 110 MB
  without**, plus the glyph batches of 5.7, which are freed before the outline pass. Without the area limit the
  same text at 8192 x 8192 px would need about 1 GB. Example of the limit at work: a line of 16 em on its default
  Circle (diameter 10.2 em, upper half) would be about 6,200 x 3,200 px = 20 megapixels at 512 px per em; the
  fallback of 5.11 draws it at 256 px per em, 5 megapixels.
- A `Preview` (typing, a slider, a point drag) costs a styled text about 4 to 7 ms on the UI thread, about 30 ms
  with the outline switched on, whatever the size of the text on the model and whatever its shape, because the
  preview picture has at most 1 megapixel. Today the same key rasterizes at the full resolution of the stamp.
- A change of a gap value in a text on the classic painter costs what one typed letter costs today.
- A font used for the first time is parsed by `fontdue` on the UI thread, once per bucket, as today (section
  3.1). This cost is not new; it is measured as baseline B4.
- The text engine does not look at the mesh or the texture, so its cost is the same on a model of 1.5 million
  triangles and an 8192 texture as on the sample sphere. Model and texture size enter only through the bake of
  the finished picture, which this design does not change. A warped or curved text has a larger rectangle than
  the same text straight (up to about twice the area for an Arc warp at 100 %; a text around a full circle has a
  square rectangle whose side is the circle diameter plus two line heights, for a line of 16 em about 7 em, that
  is about 3 times the area of the straight strip), so its bake visits that many more texels, most of them
  transparent; the round-3 limits for stamps apply unchanged and `text_path` is measured on the large model
  (8.4).

## 6. User interface

### 6.1 Where the controls live, and how it fits 1280x800

The side panel after round 3, top to bottom: the scrolling area with tool options and texture sets (about 130 to
146 pt visible at 1280x800, an estimate, section 3.5), the pinned Color section (about 305 to 315 pt), and the
lower area with the layer list (about 299 pt by default, resizable, with its own scroll area,
`app\panels\mod.rs:258-267`, `:277-282`).

**The lower area gets a tab strip: Layers | Brush | Text.** `FEATURE_BRUSH.md` (sections 6.1 and 6.2) defines the
strip for the Brush tab: one row of 24 pt in place of the "Layers" header, state `AppState::side_tab`, 271 pt for
the content of a tab. This design adds the third tab to the same strip. Reasons for the lower area: the user
asked for a tab "like in photoshop" for the brush settings; the round-3 plan announces a Brush tab and asks round
3 to keep the layout easy to extend (`target\round3_audit\ROUND3_PLAN.md:310-311`); the upper area has only about
140 pt because the colour picker must stay visible; and a floating window would be the first of the app (section
3.5), with its own risks for the pointer, the toasts and the gizmo. The price: while the Text tab is open the
layer list is hidden (question Q4).

- **The Text tab is enabled in exactly two cases: the tool is Text; or the tool is Transform and the selected
  stamp is a text.** These are the two cases in which the Text block has a target (table in 6.2). In every other
  case (any other tool, also while a text is still selected; Transform with an image stamp or with nothing
  selected) it is greyed out with the hover text "Choose the Text tool, or select a text with the Transform tool",
  and the area shows Layers. The selection alone does not enable the tab, because `SetTool` does not clear the
  selection (`app\dispatch.rs:631-635` only cancels the running tool and sets `state.tool`): a rule "enabled
  while a text is selected" would keep the Text tab open under the Brush, where none of its controls has a
  target. (An earlier version of this section had that rule and contradicted the test
  `the_text_tab_follows_the_selection`; the completeness check of 2026-11-17 found it.)
- **One rule for all tabs, the one `FEATURE_BRUSH.md` 6.2 states for the Brush tab:** a tool change never
  writes `side_tab` (`SetTool` stays as it is, `app\dispatch.rs:631-635`); a tab whose condition is not met is
  greyed out and **the area shows Layers whatever `side_tab` says**. So Text stays open when the Text tool places
  a text and switches to Transform (it does that by writing `state.tool` directly,
  `app\tools\stamp_place.rs:216`; the placed text is selected and the tool is Transform, so the tab is still
  enabled), the layer list comes back by itself under the Brush or any other tool, whether or not the text is
  still selected, and the Text tab is open again without a click when the user returns to the Text tool or
  presses "Edit" in the "Selected: ..." row (which switches to Transform).
- The buttons Warp, Path and More of the Text options set `side_tab = Text` and the page. A click on the button
  of the page that is already showing switches back to Layers.
- The whole content of the tab is one function, `text_tab::show(ui, state, ps)`, that draws into any `egui::Ui`
  of 265 pt width or more. If the user answers Q4 with "floating", only the container changes: the same function
  is then shown in an `egui::Window` (288 pt wide, not resizable, movable, kept inside the view area, default
  place 8 pt left of the side panel), and the tests marked [W] in section 8.2 apply.

**Text options: four rows, budgeted against 265 pt of width.**

```
v Text options
  +-----------------------------------------------+
  | WTE 2026                                      |   row 1: content field, 2 text rows, about 44 pt
  +-----------------------------------------------+
  [ Arial                    v ] [ Bold        v ]     row 2: font button (60 %), style list (40 %), 20 pt
  [B] [I]  [ 6.0 % ]  [ 0° ]  [##]                     row 3: B 22, I 22, Text size 64, Angle 56, Colour 28 pt
  [ Warp: Arc    ] [ Path: Circle   ] [ More ]         row 4: buttons of 100, 104 and 48 pt
```

- Width: row 3 is 22 + 22 + 64 + 56 + 28 pt plus four gaps of 6 pt = 216 pt. Row 4 is 100 + 104 + 48 pt plus
  two gaps = 264 pt. The widths are uneven on purpose: a button has 5 pt of padding on each side
  (`app\theme.rs:20`), so 90 and 94 pt remain for the two labels that tell the user the current state, and "More"
  needs little. The longest default label, "Path: Straight" (14 characters), is estimated at 85 to 90 pt in the
  button font of egui (not measured), which three equal buttons of 84 pt (74 pt for the label) could not show.
  The panel test measures the two default labels and "Path: Circle" with the font of the test context and
  asserts that each fits its button; if the measurement says otherwise, the labels lose their prefix ("None",
  "Straight") and the prefix moves into the hover text. A long style name ("Warp: Shell Lower") is cut off with
  "..." and shown in full in the hover text. No row is wider than 265 pt; a panel test asserts it (section 8.2).
- Height: header 20, then 44 + 20 + 20 + 20 pt with gaps of 4 pt: rows 1 to 3 end about 116 pt below the top of
  the area, row 4 at about 140 pt. **Rows 1 to 3 are the ones that must be visible without scrolling**; row 4 may
  scroll by a few points when round 3 leaves less than 140 pt, and loses nothing by it, because the Text tab can
  also be opened by its tab. The real visible height is measured on the finished round-3 build before the panel
  package starts (section 7.5); if it is below 116 pt the content field shrinks to one text row.
- The four rows are the first thing in the area. Everything else that can appear there comes below and scrolls:
  the wrong-view hint, the "Selected: ... Edit" row (`app\panels\tool_options.rs:91-101`), under the Transform
  tool the Name, Tint and placement fields, then the texture sets. The warning line "Font not installed"
  (`app\panels\tool_options.rs:220-222`) is replaced by a font button in the warning colour with the same text as
  hover text, so it costs no row.
- Who writes what, because `tool_options.rs` belongs to the brush panel package after the scaffold: **the
  scaffold makes every text-related edit that stays in `tool_options.rs`**: the call order (the Text block
  first, then the wrong-view hint, then the "Selected: ... Edit" row; under the Transform tool the Text block
  above Name, Tint and the placement fields), and nothing else, since the text functions move out. Everything the
  panel package T6 changes later is inside `text_block.rs`: the warning line moved there with `text_options`
  (it is at `tool_options.rs:220-222` today, inside that function), so replacing it by the coloured font button
  is an edit in T6's own file. `tool_options.rs` calls two functions with fixed signatures,
  `text_block::show(ui, state, ps, target, actions)` and `text_block::target(state)`; T6 changes their bodies, never
  the call sites.
- The round-3 asserts are kept and extended: the Color section rectangle lies inside the side panel, and the 3D
  view rectangle does not move, with the Text tool, with a selected text, and with each page of the Text tab.
  The rectangles the brush design records in `AppState::side_rects` are reused for this.

**Text tab: three pages, budgeted against the height.** Of the 271 pt of a tab a 24 pt page row leaves 247 pt
for a page. If the measurement of the brush design lowers the default share of the lower area to 37 %
(`FEATURE_BRUSH.md` 6.1), 225 pt remain. Every page fits both.

| Page | Rows | Height |
|---|---|---|
| Character | Align, Line spacing, Letter spacing, Thicken, Slant, Outline 6 x 20; gap header with the line buttons 20; gap strip 26; gap value row 20; gaps between rows 8 x 4 | 218 pt |
| Warp | style pictures 2 x 26 + 3; name line 16; Bend, Horizontal, Vertical, Direction with Reset 4 x 20; gaps 6 x 4 | 175 pt |
| Path | Kind 20; at most 2 parameter rows 40; Start 20; Other side 20; Add point / Remove point 20; hint 32; gaps 6 x 4 | 176 pt |

### 6.2 The Text block and the gap controls (`app\panels\text_block.rs`, `text_tab.rs`)

One piece of panel code, `text_block::show(ui, state, ps, target, actions)`, draws the four rows, and one,
`text_tab::show`, the tab. Both work on a `target`, and **the tool decides the target**:

| Tool | Target | The controls |
|---|---|---|
| Text | always `Draft`, also while a text is selected | change `state.tools.text` and `state.tools.stamp.rotation_deg`; no action is queued, as today (`app\panels\tool_options.rs:203-266`) |
| Transform, with a text selected | `Stamp(id)` | send `UpdateStampSource` or `UpdateStampPlacement` through `continuous_edit`, exactly as `text_stamp_fields` does today |
| Transform without a selected text; any other tool | none | the Text block is not shown; the Text tab is not enabled |

This is the rule of the code of today (`app\panels\tool_options.rs:85-101`), kept on purpose. The first version
of this design let the four rows edit the selected text whenever one was selected. That breaks placing a second
text: a placed text stays selected when the user presses X again (section 3.2), so the name typed for the second
text would have rewritten the first, while the hover picture and the next click still used the unseen draft.
With the rule above the walk-through is: X, click (text 1 is placed and selected, the tool is Transform, the rows
show text 1); X (the rows show the draft); type "Bob" (the draft changes, the hover picture shows "Bob", text 1
is untouched); click (text 2 is placed).

- Under the Text tool with a text selected, the row of today stays below the four rows: "Selected: {name}
  [Edit]"; Edit switches to the Transform tool (`app\panels\tool_options.rs:91-101`), where the rows show that
  text. The Text tab follows the same target: under the Text tool its pages edit the draft, and the gap controls
  are disabled (rule 7 below).
- Whatever round 3 builds for "Text options show the selected text ready to edit" (R3-10,
  `target\round3_audit\ROUND3_PLAN.md:231-233`) must end up as this table. If R3-10 made the Text tool show the
  selected text, the scaffold moves that to the Transform tool; the panel test
  `the_text_tool_edits_the_draft_not_the_selected_text` (8.2) decides.
- Under the Transform tool the block comes first; Name, Tint and the placement fields follow.

- **Content field**: `TextEdit::multiline` on the `GapBuffer` of 5.12 (for a draft: on the plain string, as
  today). Typing sends the new content **and** the moved gap list in one `UpdateStampSource`, `Preview` while the
  field has the focus and one `Final` when it loses it, as today (`app\panels\tool_options.rs:522-537`).
- **Text size** of a placed text is the height of one em on the model: `em_px * H / frame height`, as % of the
  model diagonal (2D: of the texture height), from `stamp_text_frame`. Changing it scales the placement about the
  anchor (centre `c1 = A + (c0 - A) * factor`, with `A = EmMap::to_world([0, 0])`), with the depth re-fit the
  placement fields use (`app\panels\tool_options.rs:625-638`). The existing field "Size" (larger side of the
  rectangle) stays in the placement fields.
- **Angle** of a placed text is the existing rotation (`placement_rotation_deg`, `rotate_placement_to`, used at
  `app\panels\tool_options.rs:641-646`); for a draft it is `tools.stamp.rotation_deg`.
- **B, I, font, style, colour** act on the whole text. A click on B or I, a font or a style pick is one `Final`;
  the colour popup sends `Preview`s and one `Final`, as today.
- A font that is embedded in the project but not installed on this machine is shown with its name and the hover
  text "Embedded in the project, not installed here". It is a full choice, not only a label: it is listed under
  "In this project" in the picker, its faces are in the style list, and picking it for another text or for the
  text to place reuses the font of the project (`resolve_font`, 5.13). Nothing is replaced by Ubuntu Light as
  long as the project holds the font.

**The gap controls** (Character page of the Text tab; they work in the side panel alone, without the view):

```
  Gaps   line 1 of 2   [<] [>]
  [W] [T] [E]|[·] [2] [0] [2] [6]            gap strip: one box per character, one slot between two boxes;
                                             here the slot between "E" and the space is selected
  [-] [  30.0 % ] [+]   [Reset] [Reset all]
```

1. **Strip.** One row, drawn from `gaps::slots`: one box per laid-out character of the shown text line (14 pt
   wide, the character in the UI font; a space is shown as a small dot, a tab as an arrow) and a slot of 10 pt
   between every two boxes, so every cell is exactly 24 pt wide. A joined pair (a letter with its combining
   mark) has no slot between its two boxes. The row scrolls sideways inside 265 pt and never widens the panel.
   **Only the visible cells are laid out:** the scroll area gets its full content width (number of cells times 24
   pt) as one empty allocation, and the cells whose index lies between `floor(left / 24)` and `ceil(right / 24)`
   of the visible clip rectangle are drawn, at most 12. A line can have more than 1,000 characters (256 em at 32
   px per em); without this every one of them would be two widgets in every frame. A slot whose gap has an own
   value is filled in the accent colour; its hover text names the two characters and the value: "Gap between E
   and (space): 30 %". Characters are named by `gaps::slot_name`: the character itself, "(space)" for a space,
   "(tab)" for a tab. In "WTE 2026" there are two gaps between "E" and "2": one between "E" and the space and one
   between the space and "2"; the texts always name the real pair. **End slot:** when the last character of the
   line carries a stored value, a slot with a dashed frame follows the last box, with the hover text "Stored gap
   after 6: 30 % (no effect at the end of a line)"; it can be selected and reset like any other. The selected
   slot has a frame and is scrolled into view.
2. **Selecting a gap.** A click on a slot sets `session.text_gap = (stamp, after)` (the field lives in `Session`,
   section 4.2, so it cannot survive into another document). In addition, while the content field has the focus
   and its cursor stands between two characters of one line without a selection, that gap is selected, so the
   arrow keys of the field walk through the gaps (this part needs the cursor position egui reports, rule 6). The
   selection is dropped when another stamp is selected, when the content changes from outside the field (undo,
   redo), or when its character no longer has a slot.
3. **Several lines.** The header shows "line n of m"; `<` and `>` change the line the strip shows. Selecting a
   gap through the field switches the strip to the line of the cursor.
4. **Value.** A drag field in % of the text size, -50 to 400, one decimal, 0.2 % per pixel dragged. Dragging
   sends `Preview`s and one `Final` through `continuous_edit`. `-` and `+` change the value by 1 % (5 % while
   Shift is held; panels may read modifiers, ARCH:2822-2824) and are one `Final` each. Without a selected gap the
   three controls are disabled with the hover text "Click a gap in the strip above". The row with the two
   reset buttons is 22 + 64 + 22 + 52 + 68 pt plus four gaps of 6 pt = 252 pt wide.
5. **Reset** sets the selected gap to 0 (its entry is removed); **Reset all** empties the gap list, including
   values that have no effect at a line end. Each is one `Final`; both are disabled when there is nothing to
   reset.
6. Two things were read in the egui source but **not tried in a running app**: that a custom buffer receives
   every edit of the field through `insert_text` and `delete_char_range` and the undo of the field through
   `replace_with` (the gap history of 5.12 depends on this); and that `TextEdit::show` reports the cursor position in every frame while the field has the
   focus (`REG\egui-0.37.0\src\widgets\text_edit\builder.rs:483`, `output.rs:23`). **The app half of the scaffold** writes
   the two panel tests that prove them, together with the part of `GapBuffer` they need (the `TextBuffer`
   implementation that records each call; about 60 lines), before `TextPanelState` and `text_gap` are fixed and
   before any package builds on them. If a proof fails, the scaffold switches the contract to the fallback below
   and says so in its note; the geometry package then builds `gaps.rs` without the field history. Fallbacks that need neither: the panel compares the old and new
   content after every change and calls `shift_for_edit` (exact except among equal neighbouring letters), and
   gaps are selected in the strip only.
7. For a draft (before placing) the gap controls are disabled with the hover text "Place the text first; then
   the space between any two characters can be changed".

While a gap is selected the status bar says, for example, "Gap between E and (space): 30 % of the text size".

### 6.3 The font picker (`app\panels\font_picker.rs`)

A popup under the font button (300 pt wide, at most 360 pt high, `PopupCloseBehavior::CloseOnClickOutside`). A
popup is not part of the panel layout, so it cannot widen the panel.

```
  [ ari|                                    ]   search field, has the focus when the popup opens
  In this project
    Hack                              Abc 123
  All fonts (3 of 78)
    Arial                             Abc 123
    Arial Rounded MT                  Abc 123
    Arial Unicode MS          23 MB   Abc 123
```

- Left: the family name in the UI font (always readable, also for symbol fonts). Right: the sample in the
  font's own face, once the sample job has delivered. For a font file of 5 MB or more its size is shown, because
  the whole file goes into the project when the font is used.
- "In this project": the families used by the text stamps of the open document, most recently added first, at
  most 8. It needs no saved preferences. Each row carries the `FontRef` of its font; a click hands that `FontRef`
  over, so the row works whether or not the family is installed on this machine. A family that is not installed
  has no sample from the sample job; its row shows "(in project)" in place of the sample.
- Rows are 22 pt high and drawn with `ScrollArea::show_rows`
  (`REG\egui-0.37.0\src\containers\scroll_area.rs:1087`), so only the visible rows are laid out.
- Typing filters (5.13). Up and Down move the highlight, Enter takes it, a click takes a row, Esc closes: an
  open popup owns Esc already (`app\shortcuts.rs:105-108`, `:124-126`), and typing in the search field fires no
  tool shortcut because it is an egui text field (`app\shortcuts.rs:104`, `:118-120`).
- Taking a font applies it to the whole text. The face of the new family is chosen by `resolve_face` (5.9) from
  what the text was before the change (bold or not, italic or not), so B and I keep their meaning across a font
  change and a real bold face replaces thickening where the new family has one. For a text that is neither bold
  nor italic this gives what `pick_style` gives today: the same style name if the new family has it, else
  "Regular", else the first face. When the font file has 5 MB or more the status bar says, for
  example, "Arial Unicode MS (23 MB) is stored in the project file".
- While the font job of the start still runs, the list shows the row "Loading system fonts..." as today.

### 6.4 The Text tab (`app\panels\text_tab.rs`)

- First row: the page selector Character | Warp | Path (`state.text_page`). Then the controls of section 1.2.
- **Character**: the six rows that exist today in other places (Align, Line spacing, Letter spacing, Outline)
  or are new (Thicken, Slant), then the gap controls of 6.2.
- **Style pictures** (Warp): 16 cells (None and the 15 styles) in two rows of 8, each 30 x 26 pt, 3 pt apart
  (261 pt). A picture is a grid of 4 x 2 squares sent through `WarpMap` at 50 % bend, so a picture can never
  disagree with the result. The name of the style under the pointer, else of the chosen style, is written under
  the grid. A click on a style is a `Final` at once with Bend 50 % if no warp was set, else with the bend kept.
- **Path**: choosing a kind is a `Final` at once. The panel creates the shape from `stamp_text_frame` (5.5): Arc
  with bend 25 %; Circle with the default diameter, its top on the old baseline and the text centred over the top
  for every alignment; Curve with three points on the baseline. "Straight" removes the path. The Diameter row
  of a Circle ends with the button **Fit** (one `Final`: diameter from the text as it is now, top point kept,
  Start 0). While `circle_fill` of the frame is above 1 the hint line of the page reads "The text is longer than
  the circle. Raise Diameter or press Fit." in the warning colour; otherwise it holds the usual hint of the
  kind. For a draft, Curve is disabled with the hover text "Place the text
  first, then bend its line with the round points". Add point inserts a point in the middle of the longest
  piece; Remove point removes the point dragged last, else the last one; both are a `Final`. Points are not
  bound to Delete or to a double-click: Delete removes the whole selected stamp today (section 3.2).
- Sliders and drag fields follow the `Preview` / `Final` protocol; a click on a button is a `Final` at once.
- An idle tab queues no action (the existing rule that display never edits).

### 6.5 What the view shows

- **Before placing:** the yellow frame of today plus the text itself at 70 % opacity (5.14).
- **Placed and selected:** the gizmo of today (four corners, rotate knob). The rectangle is the picture of the
  text; with a warp or a line it is the bounding box of the bent text. `GizmoShape`, `GizmoHandle` and the
  `Drag` struct of the Transform tool are not changed.
- **Curve points** (Transform tool, selected text with a Curve, in the view that can edit it): the baseline curve
  as a thin line (33 points per piece) and one round point of 5 pt radius per curve point, in a colour different
  from the gizmo. New file `app\tools\text_path.rs`:

  ```rust
  pub(crate) struct PathPoints { pub points: Vec<egui::Pos2>, pub curve: Vec<egui::Pos2> }      // screen positions
  pub(crate) fn path_points(placement: &Placement, frame: &TextFrame, path: &TextPath, xf: &ViewXform) -> Option<PathPoints>;
  pub(crate) fn hit(points: &PathPoints, pos: egui::Pos2) -> Option<u8>;                         // 8 pt tolerance
  pub(crate) fn screen_to_em(placement: &Placement, frame: &TextFrame, xf: &ViewXform, pos: egui::Pos2) -> Option<[f32; 2]>;
  pub(crate) struct PathDrag { pub id: StampId, pub view: ViewId, pub point: u8, pub grab: egui::Vec2, pub moved: bool }
  ```

  The Transform tool gets one new field, `path_drag: Option<PathDrag>`, next to its `drag` field. Positions come
  from `EmMap` (5.11): for a 3D text the points lie on the projector plane, where the corners of the gizmo lie
  too; the pointer is brought back by cutting the camera ray with that plane (`xf.camera.ray`, used the same way
  at `app\tools\transform.rs:231`), in 2D by `screen_to_px`. The frame comes from `doc.stamp_text_frame(id)` in
  `hover`, `press` and `drag` (which have `&mut AppState`) and from `doc.cached_text_frame(id)` in `overlay`.
- **Hit order** in `press`: the rotate knob, then the curve points, then the corners, then the body. In code: if
  `gizmo::hit_test` answers `Rotate`, the gizmo takes the press; else if `text_path::hit` answers, a point drag
  starts; else the answer of `gizmo::hit_test` is used. `app\view\gizmo.rs` is not changed.
- **A point drag** sends `doc.set_stamp_source(id, Text(spec with the moved point), Commit::Preview)` every
  frame and `Commit::Final` on release (as the tool calls `set_stamp_placement` for the other handles,
  `app\tools\transform.rs:331-353`); Esc cancels with `cancel_pending_edit`; `finish` commits. A locked text
  refuses with the existing warning. The position of one em and of the anchor on the model do not change during
  the drag (5.11), so the other points stay under their marks while the picture changes size. The pointer
  position is clamped to the coordinate range of 4.1 (60 em from the anchor) before it becomes a point, so a
  drag far out while zoomed out stops at the range. **When a `Preview` is refused** (the text is too long for
  any resolution, or the text is locked), the drag keeps the last point that was accepted: the spec, the picture
  and the mark of the point stay as they were, the status bar shows the reason once, and the drag goes on as
  soon as the pointer is back at a position that works. Release then commits the last accepted point; if none
  was accepted, the drag ends without a history entry.
- The cursor is "move" over a curve point. There is no text cursor on the model.

### 6.6 Keyboard

No new key. Single keys are no shortcuts while a field has the focus, the first Esc leaves the field, Ctrl+Z in
the field is the undo of the field, all as today. Ctrl+B and Ctrl+I are not bound: B and I are buttons.

### 6.7 Small windows

Below 1280x800 the colour picker may shrink and then scroll, as round 3 defines. The lower area has a minimum of
160 pt (`app\panels\mod.rs:277`); the Text tab then scrolls inside it. At 640x480 the four rows of the Text
options scroll like every tool option.

## 7. Work breakdown

### 7.1 Packages

The scaffold runs alone and first. It is **one package for both round-4 features**: `FEATURE_BRUSH.md` 7.3 calls
it "scaffold S" and lists what it writes for the brush; the row S0 below lists what it writes for the text. Every
file that both features need has the scaffold as its only owner. After it the workspace builds, all existing
tests pass, and nothing behaves differently. Sizes are estimated lines of new code plus lines of tests.

| Package | Owns | Content | Size | After |
|---|---|---|---|---|
| **S0 text part of the shared scaffold** | `Cargo.toml`, `crates\wte_core\Cargo.toml`, `Cargo.lock`; `core\text\mod.rs`, `core\lib.rs`, `core\error.rs`; `core\project\manifest.rs`, `mod.rs`, `migrate.rs`; `app\state.rs`, `app\action.rs`, `app\dispatch.rs`, `app\jobs.rs`, `app\panels\mod.rs`, `app\panels\tool_options.rs`, `app\control_sweep.rs`; one line at each of the 11 `TextSpec` literals | The gates of 7.5. Then: the types and functions of 4.1 with their tests; `fx` in `TextEntry` with conversions; version 2 with the empty step; `LoadWarning::TextFeatureUnknown`; the `ttf-parser` line; **the tab strip of the lower area with its three tabs** (`SideTab { Layers, Brush, Text }`, the enable conditions and the tab rule of 6.1, the container calls of `layers`, the Brush tab and `text_tab::show`, the rectangles in `side_rects`; see 7.3); `text_gap` in `Session`; the target table of 6.2 in the tool options (Text tool: draft; Transform: selected text); the action `LoadFontSamples` and the job output for samples, with one-line arms that call into `app\text_ops.rs`; the move, unchanged, of `text_options`, `font_combos`, `pick_style`, `font_ref_for`, `text_stamp_fields`, `align_buttons` and their tests from `tool_options.rs` into the new `app\panels\text_block.rs`, and of the text fields of `PanelState` (`stamp_text`, `font_check`) into a `TextPanelState` defined there; compiling stubs with the final signatures for every new file and document function below (`render_text` = classic painter with the centre as anchor, identity warp, straight path) | 600 + 350 | — |
| **T1 text geometry** | new `core\text\outline.rs`, `scan.rs`, `warp.rs`, `path.rs`, `gaps.rs` | 5.3, 5.5, 5.6, 5.7, 5.12: pure functions with unit tests | 900 + 650 | S0 |
| **T2 painters** | `core\text\layout.rs`, `core\text\raster.rs`, new `core\text\styled.rs`, `frame.rs` | the gap term in the classic layout (5.2) with the tests the brief asks for; the dispatcher of 5.1; the styled painter (5.2, 5.4, 5.8, 5.9 thickening, 5.10); the pure part of 5.11 | 750 + 650 | S0; needs T1 to produce styled pixels, starts on the stubs |
| **T3 fonts** | `core\text\font_catalog.rs`, `core\blob.rs` | 5.13 (list, file sizes, samples with the painter's character map, font memory) and `face_traits` of 5.9 | 300 + 250 | S0 |
| **T4 document** | `core\doc_stamps.rs`, `core\document.rs`, the initialisers of the two new fields in `core\doc_build.rs`, `crates\wte_core\tests\end_to_end.rs` | 5.11: anchor rule in the three places with the kept point, fallback with both picture limits, frame cache, preview quality, the four functions, `find_font` with the `font_refs` list, `sanitize_fx` calls | 450 + 500 | S0 |
| **T5 project** | `core\project\save.rs`, `core\project\load.rs`, `crates\wte_core\tests\project_roundtrip.rs` | 4.3 items 2 to 6: `required_reader`, the float list, the warning for unknown names, the fixtures and round trips | 120 + 350 | S0 |
| **T6 panels** | `app\panels\text_block.rs`, new `app\panels\text_tab.rs`, `font_picker.rs`, new `app\control_sweep\text_panel_tests.rs` | 6.1 (the four rows), 6.2 (with the gap controls, `GapBuffer` and its history), 6.3, 6.4, `resolve_font` of 5.13, `candidates` and `resolve_face` of 5.9 | 1,350 + 750 | S0 |
| **T7 tools and flows** | `app\tools\stamp_place.rs`, `app\tools\transform.rs`, new `app\tools\text_path.rs`, new `app\text_ops.rs` | 5.14, 6.5, anchor at the click, the sample job and its atlas | 650 + 350 | S0 |
| **G shared gate** | `app\selftest\*`, `ARCHITECTURE.md` (merge), `dist\*`, the release build | scenarios of 8.3, the numbers of 8.4, the round-3 scenario set, clippy, the model sweep | 450 | all |

The scaffold creates every new file as a compiling stub and adds the stub document functions to
`core\doc_stamps.rs`; from then on each of these files belongs to the T package named in the table.

Files that are deliberately **not** touched by the text feature: `core\stamp_bake.rs`, `texel_map.rs`,
`stamp.rs`, `core\history.rs` (one exception: the scaffold adds the line `fx: TextFx::default()` to the
`TextSpec` literal of the test `restore_drops_stale_text_rasters`, `core\history.rs:1537`; no code outside that
test changes), `core\text\edt.rs`, `core\export\*`, `app\view\gizmo.rs`, `app\tools\mod.rs`,
`app\shortcuts.rs`, `app\prefs.rs`, `app\selftest\mod.rs` (the text feature adds no step kind; scenario
`text_tab` uses `Step::Hover`, which the shared scaffold adds for the brush, 8.3), `app\io_flows.rs` beyond the
one-line arm for delivered samples and the `text_gap: None` of the new session (the font job of the start stays
as it is).

### 7.2 Order

1. The gates of 7.5, then the scaffold, alone.
2. T1, T3, T4, T5, T6, T7 in parallel; T2 with them, finishing after T1. T4 tests the anchor rule with
   hand-made frames and with the classic painter; T6 and T7 test specs and actions, not pixels. Each package
   builds into its own target folder on E:, as in round 3.
3. Merge, then G. The tests that need real styled pixels across packages (document tests with a real warp,
   scenarios) run in G.

The gap feature alone needs only S0, the gap part of T1 and T2, T5 and the gap controls of T6: if everything
else slipped, "segmented" as the user defined it would still ship.

### 7.3 Shared ground with Feature 1 (brush), to settle when the two plans are merged

`FEATURE_BRUSH.md` (written in the same run; read for this section) and this design agree on the tab in the
lower area, on one scaffold and on one gate. These points still need one decision each:

- **The tab strip: built once, by the shared scaffold.** Decided here, so that neither panel package depends on
  the other. The scaffold writes, in `app\panels\mod.rs`: the strip with the three tabs, the enable conditions
  (Brush: the tool is Brush or Eraser; Text: the tool is Text, or the tool is Transform and the selected stamp
  is a text), the one tab rule of
  6.1, the container that calls the Layers list, the Brush tab function and `text_tab::show`, and the rectangles
  in `side_rects`. The Brush tab function and `text_tab::show` are compiling stubs at that point (an empty tab
  with its name). The layout tests of both features (`text_rows_fit_the_panel`,
  `the_colour_section_stays_inside_the_panel_on_every_page`, `the_text_tab_follows_the_tool_and_the_selection`, the brush
  tests A11 to A13) can then run inside each package's own tree before the merge.
- **Ownership of `app\panels\*` and `app\control_sweep\*`, file by file.** After the scaffold: T6 owns
  `text_block.rs`, `text_tab.rs`, `font_picker.rs` and `control_sweep\text_panel_tests.rs` and touches no other
  file of the two folders. The brush panel package D1 owns everything else there (`mod.rs`, `tool_options.rs`,
  `layers.rs`, its Brush tab file, the existing test files) and fills its own tab function; if it has to change
  the strip (for example for the computed height of the lower area), it does so in `mod.rs`, which is its file
  from then on.
- **What this needs from the brush plan.** `FEATURE_BRUSH.md` (as read on 2026-11-17 for this revision) lists
  "tab strip" among the deliveries of D1 and gives D1 the two folders whole (its package table, row "D1
  app-panels"), and defines `SideTab { Layers, Brush }`. When the two plans are merged, that row must read:
  the strip moves to the shared scaffold, `SideTab` has three values, and the four text files are taken out of
  D1's folders. The tab rule itself needs no change: both documents now state the same one. This document cannot
  change the other; the merge step of the plan does.
- **Contract files.** `state.rs` and `action.rs` are written by the scaffold only (`app\state.rs:3`,
  `app\action.rs:4`); so are `dispatch.rs`, `jobs.rs` and the arms in `io_flows.rs`, with the flow code of each
  feature in its own file (`app\brush_flows.rs` there, `app\text_ops.rs` here).
- **Format version.** The brush design changes nothing in `.wtep`. The bump to 2 and `required_reader` of 4.3
  belong to the scaffold and are caused by the text feature alone.
- **Baselines.** The gate measures before any package starts; the text baseline of 7.5 goes next to the brush
  baseline in `target\round4\perf\`.

### 7.4 Overlap with round 3

Round 4 starts from the round-3 result. Every line number in this document is from the round-2 tree. State of
the round-3 copies when this was written (2026-11-17, about 13:00, compared with `target\round3\work\base`):
`pkg_c\...\panels\mod.rs` has the fix of R3-2 only (an edit that ends without a change sends the owed action
itself; an owed `Final` whose stamp is gone is dropped), which agrees with the way 6.2 ends an edit;
`pkg_a\...\doc_stamps.rs` has only a timing probe; `pkg_b\...\project\load.rs` has load-warning changes.
**R3-4, R3-9 and R3-10 were in no copy yet.** S0 re-reads them in the finished tree:

| Round-3 item | File | What this design does there, and what to check |
|---|---|---|
| R3-4: coverage cache per stamp for tints, preview bake throttle, no depth fit while the pointer is down | `core\doc_stamps.rs`, `app\tools\transform.rs`, `stamp_place.rs` | The frame cache and the preview set sit next to whatever holds the text picture. Check that a preview picture of 5.8 never ends up in a cache that a `Full` bake reads. The point drag follows whatever rule R3-4 gives the gizmo drag for the depth fit. |
| R3-9: a text stamp is renamed inside the source update when its name is still the automatic one | `core\doc_stamps.rs` (`set_stamp_source`) | The anchor rule and the fallback are added in the same function. The name rule reads `content`, which this design does not change. |
| R3-10 and user decision 2: pinned Color, tool options in their own scroll area, "Text options show the selected text" | `app\panels\mod.rs`, `tool_options.rs`, `layers.rs` | The tab strip goes into the lower area that R3-10 leaves; the Layers footer that R3-10 moves outside the scroll area belongs to the Layers tab. The `target` rule of 6.2 replaces whatever R3-10 built for the selected text. **Measure the visible height of the tool options at 1280x800.** |
| R3-1, R3-3: load warnings | `core\project\load.rs` | `TextFeatureUnknown` is added to the same warning list. |
| new asserts in `text` and `stamp` (Color section inside the panel) | `app\selftest\scenarios.rs` | Kept; extended to every page of the Text tab. |

`core\text\*` is owned by nobody in round 3 (`target\round3\ROUND3_BUILD_RULES.md`, section 5), and most new code
of this design is in new files.

### 7.5 Gates before any package starts

1. **The user has been asked Q2 to Q8** (section 2). Two answers are needed at fixed points, because the other
   answer throws work away: Q4 (tab or floating panel) before the app half of the scaffold starts, and Q2 (a
   curved text on one side, or around the model) before the first wave of packages starts. If the user was asked
   and has not answered when that point is reached, the default is built, and the plan states in writing that the
   Path page may then be set aside (Q2) or the container rebuilt (Q4). The other questions each change one
   package or one default and can be answered at any time. ("Segmented" is decided.)
2. **Rebase.** The references of this document are checked against the finished round-3 tree (table 7.4).
3. **The visible height** of the tool options at 1280x800 is measured on the round-3 exe (a screenshot of the
   `text` scenario) and written into the note for T6. Below 116 pt: the content field gets one text row.
4. **Offline build.** In a copy on E:, with the dependency line of 5.1:
   `.\cargo.cmd build --offline -p wte_core --target-dir E:\claude_programs\wrapped_texture_editor\target\r4_s0`
   and the full test suite of `wte_core`. `Cargo.lock` may differ from the round-3 one by exactly one line.
5. **Baseline, on the unchanged round-3 code** (release build, idle machine, median of 5 runs):
   - FNV-1a 64-bit hashes of the `rasterize_text` output for 8 plain specs with the bundled fonts: one line; three
     lines with an empty one; two centred lines; outline 8 %; letter spacing 20 % with line spacing 1.7; 32 px
     per em; 512 px per em; a character that only the fallback font has. They become the constants of the test
     `plain_text_is_bit_identical`.
   - The placement after one content edit of a projected and of a UV text (constants of
     `editing_a_plain_text_gives_the_round_3_placement`).
   - Times: B1 = `rasterize_text` plus `PremulMips::build` for "WTE 2026" at 256 px per em; B2 = 60 letters at
     128 px per em with a 6 % outline; B3 = one line 16 em wide at 512 px per em; B4 = the first
     `FontCache::get` (the `fontdue` parse) of Arial and of the largest installed font file at the 128 bucket.
   - The frozen version-1 text fixture of 4.3 item 5. It is a test in a source file, so it is not part of the
     measuring step: **the scaffold adds it as its first project task and runs it green on the still unchanged
     manifest code**, and only then writes the `fx` field and version 2. From then on the test belongs to the
     project package (T5) with the rest of `project_roundtrip.rs`.

### 7.6 Cut line

If the round must shrink, parts are dropped in this order; each cut removes work from the named packages only
and leaves the file format as designed:
1. the hover picture (T7; the yellow frame stays);
2. the samples in the font picker (T3, T7; search and names stay);
3. the Curve kind with its points in the view (T7, part of T1 and T6; Arc and Circle stay);
4. the direction switch of the warp (T1, T6).

The scaffold, T1 (gaps, outline, scan, warp), T2 and the anchor rule of T4 are the foundation and are never cut;
gap spacing, the one feature the user defined in their own words, is part of it. The three items of Q6 (click selects an existing text, remembered settings, rescan) are outside the plan until the
user asks for them.

### 7.7 ARCHITECTURE.md sections that change

2.1 and 2.2 (file lists), 3.1 and 3.2 (`ttf-parser` as a direct dependency), 5.2 (`TextFeatureUnknown`), 5.6
(`BlobStore::insert` fast path), 5.13 (types, `render_text`, `text_frame`, catalog), 5.14 (`text_frames`,
`text_preview_ids`), 5.20 (anchor rule, fallback, the four functions), 5.23 and 7.2 (`fx`), 6.5 (the text
rescale rule becomes the anchor rule, with the alignment side as the kept point for a change of spacing; the area limit of a styled picture; preview quality; the exception to "a preview never changes `size_px`"),
6.8 (gap spacing in the layout, styled layout, outlines, line, warp, rasterizer, thickening), 7.3 (version 2,
`required_reader`, the second frozen fixture), 8.2 (state, actions), 8.6 (hover picture, anchor at the click, curve points), 8.7 (tab strip,
Text block, gap controls, font picker, Text tab), 8.9 (sample job), 8.11 (new scenarios), 9 (tests).

## 8. Acceptance tests

No test injects input at the level of the operating system. Unit tests use only the two bundled fonts (Ubuntu
Light, Hack Regular), so their results do not depend on the machine; the one scenario that needs installed
fonts logs SKIP per missing family.

### 8.1 Unit tests in `wte_core`

Gap spacing in the layout (`text\layout.rs`; the tests the brief names). "ABCD\nEFGH", Ubuntu Light, 128 px per
em, left-aligned unless stated:
- `one_gap_moves_exactly_the_glyphs_after_it` (positions in the picture, measured from the left edge of the line
  box; where the picture lands on the model is tested under "Document"): a gap of 0.25 em after "B": the glyphs
  of "C" and "D" are 32 px further right; "A", "B" and all glyphs of the second line have the positions they have without the gap; the
  first line is 32 px longer.
- `a_negative_gap_pulls_letters_together` (-0.25 em: "C" and "D" are 32 px further left).
- `a_gap_adds_to_letter_spacing_and_kerning`: letter spacing 0.1 em and a gap of 0.2 em after "A" in "AV": the
  distance of the two glyph origins is advance + kerning + 0.3 em, within 1 px (the rounding of the origins).
- `a_gap_after_the_last_character_of_a_line_changes_nothing` (gaps after "D" and after "H": the picture is
  byte-identical to the one without them), `a_gap_after_a_tab_follows_its_fourth_space`.
- `a_gap_ignores_the_fallback_font` (a character only the fallback font has, with a gap after it: the next glyph
  moves by the gap value within 1 px).
- `centred_lines_stay_centred` (centre alignment, gap of 0.5 em in line 1: both lines stay centred on the box
  within 1 px).
- `an_empty_gap_list_is_today` (the 8 hashes of 7.5).

Gap values and edits (`text\gaps.rs`, `text\mod.rs`):
- `values_survive_content_edits`: "WTE 2026" with gaps after "E" (index 2) and after "0" (index 5). Insert "X"
  at 0: the entries are at 3 and 6. Insert "X" at 3 (into the first gap): 2 and 6. Delete "TE" (1..3): the first
  entry is gone, the second is at 3. Replace "20" by "1999": as delete then insert. Text with "ä" and "€":
  indices count characters, not bytes.
- `shift_for_edit_keeps_untouched_gaps` ("WTE 2026" to "WTE 2027": both entries unchanged).
- `edits_keep_the_invariants`: 10,000 pseudo-random `insert`, `delete`, `set` calls from a fixed seed; after each
  one the list is sorted, has one entry per character at most and no zero.
- `non_finite_and_out_of_range_gaps_are_sanitized`: NaN and infinity are removed; 9.0 becomes 4.0, -3.0 becomes
  -0.5; an index beyond the text, an entry on a line feed, one on a skipped control character and one on the
  "e" of "e\u{301}" (a joined pair) are removed; of two entries for one character the later wins; an unsorted
  list is sorted.
- `slots_follow_the_lines` ("AB\nC\tD": line 1 has one slot, line 2 has two, the tab is one character),
  `a_stored_end_value_gets_an_end_slot` ("AB\nCD" with a value after "B": line 1 has the slot A|B and one end
  slot with `right: None`; without the value there is no end slot),
  `joined_characters_have_no_slot` ("ae\u{301}b": slots a|e and mark|b only; a string with U+200D between two
  characters: no slot on either side of the joiner), `slot_names` ("E", "(space)", "(tab)").
- `field_history_restores_gaps` (the `GapBuffer` alone, no egui): "WTE 2026" with a gap after "E";
  `delete_char_range(1..3)`: no entry; `replace_with("WTE 2026")`: the entry `after = 2` with its value is back;
  `replace_with("W 2026")` (a redo): no entry again. The same text reached twice with different gap lists: the
  nearest earlier state wins. A `replace_with` to a text that was never in the field falls back to
  `shift_for_edit` without a panic. The history never holds more than 200 states.
- `sanitize_repairs_everything`: NaN and infinity in every float of `fx`, a curve with 1 and with 40 points, a
  curve point at (1e6, -1e6) em, a Circle centre at (0, 500) em, a Start of 1e9. Afterwards all floats are finite
  and in range (points and centre within 60 em, Start within 100), the curves are gone or have 16 points, **and
  `text_frame` of the repaired spec at 32 px per em is `Ok`** for a text of 8 letters. No panic for 1,000
  pseudo-random specs from a fixed seed.
- `float_list_is_complete`: a spec with an outline, 2 gaps, a warp and a curve of 3 points has exactly 18 floats
  in `floats` and in `floats_mut`, with the same names in the same order.
- `differs_only_in_spacing_is_strict` (true for a changed gap list and for a changed letter spacing; false when
  the content, the alignment, the font or any other field differs as well).

Warp (`text\warp.rs`):
- `every_style_is_the_identity_at_zero`: 15 styles, both directions, 81 points, error below 0.0001 px.
- `no_style_folds_over`: the check of 5.6 (64 x 64 grid with the border, bends -1 to 1 in steps of 0.25
  without 0, frames 1:4 to 16:1, both directions, both distortions at -1, 0, 1): every mapped triangle has more
  than 0.05 of its original signed area.
- `arc_at_full_bend_is_a_half_circle` (frame 8:1: the mapped left and right frame edges are horizontal within
  0.01 px), `arc_keeps_the_middle_line` (frame 4:1, bend 0.5: length within 0.5 % of the frame width),
  `arc_inner_edge_keeps_a_quarter` (frame 1:2, bend 1: inner edge at least 0.249 of the frame width).
- `arc_lower_keeps_the_top_edge`, `arc_upper_keeps_the_bottom_edge`, `fisheye_and_twist_keep_the_frame_border`
  (movement below 0.0001 px).
- `distortion_scales_the_sides` (bend 0, Horizontal 1: the right edge is 1.4 times, the left edge 0.6 times as
  tall, within 0.1 %), `vertical_axis_is_the_turned_style`.
- `refinement_follows_the_curve`: a straight edge of 200 px under Arc at 100 %: the refined, mapped polyline is
  nowhere further than 0.15 px from the true curve (1,000 samples) and has no piece longer than 16 px.

Line (`text\path.rs`):
- `arc_bend_is_the_share_of_a_circle` (bend 0.25: the tangents at the start and the end of the widest line are
  90 degrees apart within 0.1 degree), `arc_at_zero_is_the_straight_layout` (glyph positions within 0.01 px),
  `negative_bend_keeps_letters_upright`.
- `circle_does_not_depend_on_the_text` (centre and radius of the baseline equal for "A" and "AAAAAAAA" within
  0.01 px), `circle_flip_reads_upright_at_the_bottom`.
- `a_new_circle_is_centred_on_its_top`: the Circle the creation rule of 5.5 gives for a **left-aligned**
  "WTE 2026", and the same for Center and Right: the glyph middles are symmetric about the vertical line through
  the circle centre within 0.05 em, all of them lie above the centre, the first glyph is within 3 degrees of 9
  o'clock and the last within 3 degrees of 3 o'clock, and no glyph is turned by more than 95 degrees from
  upright. The same for the two-letter text "Hi", where the smallest diameter applies and `start` does the
  centring: the centroid of the glyph middles is within 0.1 em of that vertical line and above the centre.
- `circle_alignment_pins_its_side` (Left: the first glyph stays at 9 o'clock when letters are appended; Right:
  the last glyph stays at 3 o'clock; Center: the middle stays on top), `circle_start_turns_all_lines_alike`
  (two lines, Start 1 em: both are turned by the same angle), `circle_fill_reports_overflow` (a line 1.5 times
  the circumference: `circle_fill` is 1.5 within 1 %; half the circumference: 0.5; a text without a Circle: 0).
- `start_is_limited_on_open_lines` (Arc and Curve with Start 100 em and a line of 5 em: the text sits exactly one
  line length past the end, as with Start 5).
- `curve_passes_through_its_points` (within 0.01 em), `curve_does_not_overshoot` (points (0,0), (4,0),
  (4.2,0.2), (4.2,4) em: the curve stays inside the bounding box of the points grown by 0.15 em),
  `length_table_is_exact` (a straight piece within 0.001 px; a quarter circle of radius 100 px as one Bezier
  piece within 0.1 % of 157.08 px), `glyph_middles_are_spaced_by_their_advances` (within 0.5 px along the
  curve), `text_longer_than_the_line_continues_straight`, `flip_reverses_the_direction`.
- `a_gap_is_applied_before_the_line`: "AB" on a Circle of 4 em diameter; a gap of 0.5 em after "A" increases
  the angle between the two letters by 0.25 radian (0.5 em on a radius of 2 em), within 1 %.

Rasterizer and outlines (`text\scan.rs`, `text\outline.rs`):
- `a_square_has_exact_coverage` (a 10.5 x 10.5 px square at a fractional position: every pixel within 1/255 of
  its covered area), `a_ring_has_an_empty_middle`, `overlapping_contours_fill_once` (two overlapping squares
  wound the same way: no value above 255), `edges_outside_the_buffer_are_clipped`.
- `outline_bounds_match_fontdue_metrics` ("A", "g", "j", "W" of both fonts at 128 px: within 1 px).
- `coverage_matches_fontdue` (the same glyphs at 64 px: at least 99 % of the pixels within 8/255).

Painters (`text\raster.rs`, `text\styled.rs`):
- `plain_text_is_bit_identical` (the 8 hashes of 7.5); all existing tests of `layout.rs` and `raster.rs` pass
  unchanged.
- `frame_equals_the_raster_frame`: for 12 specs covering every feature, `text_frame` equals the frame of
  `render_text(.., Full)` and the picture has the size of the frame.
- `switching_the_painter_moves_no_ink`: for "Ag j", "WTE 2026" (with a gap of 0.3 em after "E") and two centred
  lines, at 64 and 512 px per em, each of these changes alone (slant 0.001 degrees; thickening 0.0001 em; Arc
  warp at bend 0; Flag at bend 0; Arc line at bend 0) leaves the ink bounding box, measured from the anchor,
  within 1 raster pixel of the classic one on every side.
- `both_painters_cover_alike`: with the anchors aligned, the alpha of the styled picture (slant 0.001 degrees)
  is within 8/255 of the classic alpha on at least 99 % of the pixels.
- `bold_grows_the_ink_by_its_radius` ("H" at 128 px per em, 0.02 em: the ink box grows by 2.56 px on every side
  within 1 px; "HH" gets wider by a further 5.12 px within 1 px).
- `bold_keeps_a_mark_over_its_letter` ("e\u{301}" in Ubuntu Light at 128 px per em, thickening 0 and 0.08 em:
  the horizontal distance between the ink centre of the accent and the ink centre of the "e" changes by less
  than 1 px; the line is wider by 2 x 0.08 em, not 4 x). Skipped with a log line if the bundled font has no
  U+0301.
- `slant_moves_the_top_to_the_right` ("l", 12 degrees: top row against bottom row = tan(12 degrees) times the
  ink height within 1 px; the lowest ink row within 1 px of the plain one).
- `a_gap_bends_with_the_warp` (Flag at 50 % on "ABCD" with a gap of 0.5 em after "B": the picture is 64 px
  wider than without the gap, within 2 px, at 128 px per em).
- `preview_covers_the_same_rectangle`: for 6 styled specs and `Preview { max_pixels: 65_536 }`: the frame equals
  the full frame; the picture has at most 65,536 pixels; scaled up to the full size it differs from the full
  picture by a mean alpha difference of at most 6/255; the ink centroid, as a fraction of the picture size, is
  within 0.002 of the full one.
- `the_raster_scales_with_size_px` (64 against 128 px per em: size and anchor times 2 within 2 px).
- `thread_count_does_not_matter` (rayon pools of 1 and 8 threads: identical bytes), `too_large_reports_its_size`.
- `a_styled_picture_has_an_area_limit`: `text_frame` of a line of 16 em around a full Circle is `Ok` at 512 px
  per em (about 12.7 megapixels); with a diameter of 9 em (about 5,560 px a side, 31 megapixels) it is
  `TooLarge` at 512 and `Ok` at 256 px per em; a plain text of the same size in pixels is not refused (the
  classic painter keeps the side limit only).

Frame (`text\frame.rs`):
- `refit_keeps_the_alignment_edge` (`Keep::AlignEdge`, projected and UV, rotation 30 degrees, each flip): two
  classic frames of the same left-aligned text, the second 64 px wider: the host position of the left edge of
  the line box changes by less than 0.0001 of the rectangle diagonal, the centre moves by half the added width
  along `ex`; right-aligned: the right edge stays; centred: the centre is bit-equal.
- `refit_keeps_anchor_and_em` (`Keep::Anchor`): projected and UV placements, rotation 30 degrees, each flip, both flips, and a
  rectangle stretched to twice its width; 100 pseudo-random frame pairs with off-centre anchors and different
  resolutions: the host position of the anchor changes by less than 0.0001 of the rectangle diagonal and the host
  size of one em by less than 0.000001 relative, per axis.
- `centred_frames_leave_the_centre_bits` (the centre is bit-equal), `repeated_refits_do_not_drift` (20 changes
  between 128 and 512 px per em: below 0.01 texel, or 0.00001 of the model diagonal),
  `em_map_round_trips` (within 0.001 em, 3D and 2D).

Catalog (`text\font_catalog.rs`, `blob.rs`):
- `families_sorted_without_case`; `equal_style_names_stay_reachable` (the bundled Ubuntu bytes loaded twice give
  the styles "Light" and "Light (Ubuntu-Light)", and `find` returns each);
  `face_bytes_shares_one_allocation_per_file` (two faces of one source get the same `Arc` while it is held);
  `blob_insert_knows_the_same_allocation`; `a_sample_has_ink` (both bundled fonts: at most 128 x 20 px, at least
  40 inked pixels); `file_sizes_are_read_once` (`faces()` called 100 times touches no file: the size comes from
  the map).
- `the_sample_maps_like_the_painter` (`text\outline.rs`): for both bundled fonts and 300 mapped characters,
  `char_to_glyph` returns the index `fontdue::Font::lookup_glyph_index` returns; for a hand-made font table with
  only a (1,0) and a (3,0) subtable it returns their mapping where `Face::glyph_index` returns `None`.
- `face_traits_match_the_catalog`: for both bundled fonts `face_traits` gives the weight and italic flag
  `FontFaceInfo` has; bytes that are no font give `None`.

Document (`doc_stamps.rs`, `tests\end_to_end.rs`):
- `a_spacing_edit_keeps_the_alignment_edge`: a projected and a UV text "WTE 2026", rotation 30 degrees; a gap of
  0.5 em after "E" is set. One em keeps its size on the model within 0.1 % and the rectangle is wider by 0.5 em
  within 1 % in every case. Left-aligned: the baked ink of "WTE" (the texels left of the gap) is unchanged
  within 1 texel and the ink right of the gap moved by 0.5 em within 1 texel; the centre of the rectangle moved
  by 0.25 em. Right-aligned: the ink right of the gap is unchanged, "WTE" moved by 0.5 em. Centred: the centre of
  the rectangle is bit-equal. The same three cases for a change of Letter spacing from 0 to 10 %. With an Arc
  warp the anchor is kept instead (within 0.05 texel).
- `find_font_prefers_the_project`: a document whose text uses font X registered from bytes A; `find_font("x",
  style)` returns that `FontRef` without regard to case; a font handed out by `register_font` but used by no
  stamp is found too; after its blob is gone it is not.
- `an_area_overflow_drops_the_resolution`: a stamp at 512 px per em with a line of 16 em gets its default Circle:
  `Ok`, `size_px` 256, one em on the model within 1 %, the anchor within 0.5 texel.
- `anchor_stays_when_an_effect_is_switched_on`: a projected text on the sample sphere and a UV text, with
  rotation 30 degrees and `flip_x`; after each of Arc 50 %, Flag -100 %, a Circle, thickening 2 %: the anchor
  moved by less than 0.01 % of the model diagonal (0.05 texel) and one em changed its size by less than 0.1 %.
- `editing_a_plain_text_gives_the_round_3_placement` (bit-equal to the constants of 7.5).
- `typing_on_a_circle_does_not_move_the_circle` (5 letters appended: the centre of the circle within 0.05
  texel), `moving_a_curve_point_keeps_the_others` (within 0.05 texel).
- `a_too_large_edit_drops_the_resolution` (a stamp at 512 px per em gets a line of 40 em: `Ok`, `size_px` 128,
  one em on the model within 1 %, the anchor within 0.5 texel).
- `previews_and_one_final_are_one_entry`, `undo_restores_fx_placement_and_pixels`, `a_locked_text_refuses_fx`.
- `the_frame_needs_no_pixels_after_undo_and_load`: `cached_text_frame` is `None`, `stamp_text_frame` is `Ok`,
  and the rasterization counter of the test build stays 0.
- `a_full_bake_never_uses_a_preview_picture`.

Project (`project\*`, `tests\project_roundtrip.rs`):
- `frozen_v1_text_fixture_loads`, `frozen_v2_fx_fixture_loads` (literal JSON strings).
- `gaps_round_trip` (three gap values, one of them negative: equal after save and load, bit for bit),
  `fx_round_trips` (thickening, slant, each of the 15 styles, each line kind),
  `plain_text_entry_has_the_v1_keys` (exactly the 8 keys and `"kind"`).
- `required_reader_follows_fx` (1 without; 2 with one gap value; 2 with a warp; `format_version` 2;
  `STEPS.len() == 1`).
- `an_unknown_warp_style_loads_without_warp` (style "ribbon": the stamp is there, `fx.warp` is `None`, one
  `TextFeatureUnknown`, `force_save_as`), `broken_fx_from_a_file_is_repaired` (a gap index beyond the text, 40
  curve points, bend 7.0, a curve point at 1e6 em; and a warp entry without a `style` key and a path entry
  without a `kind` key: the project loads, the feature is `None`, one `TextFeatureUnknown` each).
- `a_path_entry_writes_only_its_own_keys` (a curve has no `bend`, `diameter`, `center`; an arc has no `points`;
  each loads back equal), `a_wrong_type_in_fx_is_a_manifest_error` (a text where a number belongs:
  `ManifestInvalid`, the documented limit of 4.3 item 4).
- `an_end_of_line_gap_needs_reader_2` ("AB\nCD" with a value after "B" only: `required_reader` is 2).
- `a_nan_in_any_float_refuses_save` (18 cases, each naming its float).
- `styled_text_survives_save_reopen_and_export`: the exported flat PNG and the per-layer `_text_` PNG are
  byte-identical before saving and after reopening.

### 8.2 Headless panel tests (`app\control_sweep\text_panel_tests.rs`, typing and clicking the real panels)

Proofs of the egui behaviour, written by the app half of the scaffold before any package starts (6.2 rule 6); the
panel package T6 owns the file afterwards and keeps both tests:
- `the_gap_buffer_sees_every_edit_of_the_field` (typing, Backspace, a replaced selection: each arrives as
  `insert_text` or `delete_char_range` with the exact character position; Ctrl+Z and Ctrl+Y in the field arrive
  as `replace_with` with a text that is in the history of the buffer),
  `the_field_reports_its_cursor_while_focused`.
- `field_undo_brings_the_gap_back`: the text stamp "WTE 2026" with a gap of 0.3 em after "E"; click into the
  content field, select "TE", press Delete: the `Preview` spec has no gap entry; press Ctrl+Z: the `Preview` spec
  has the content "WTE 2026" and the entry `after = 2`, 0.3 em, again; leave the field: the one `Final` has it
  too, and the undo count grew by 1 at most.

Layout:
- `text_rows_fit_the_panel`: with the Text tool, with a selected text, and with "Warp: Shell Lower" chosen: the
  side panel is 300 pt wide, no row of the Text block is wider than 265 pt, rows 1 to 3 end at most 116 pt below
  the top of the tool options area, and the Color section rectangle is inside the side panel. In the default
  state the labels "Warp: None" and "Path: Straight", and "Path: Circle" after choosing it, are not cut off:
  the width of each label, laid out with the button font of the test context, is at most the inner width of its
  button.
- `the_colour_section_stays_inside_the_panel_on_every_page` (1280x800, the three pages, a text of 60 letters so
  the gap strip scrolls).
- `the_text_tab_follows_the_tool_and_the_selection`, formerly `the_text_tab_follows_the_selection` (every line
  is asserted through `side_rects`, with `side_tab = Text` throughout): with the Stamp tool and nothing selected
  the tab is greyed out and the area shows Layers; with the Text tool the Text tab is shown; it stays shown when
  the Text tool places a text and switches to Transform; `SetTool(Brush)` shows Layers and leaves `side_tab` on
  Text, **although the text is still selected** (the test asserts `selected_stamp` is unchanged);
  `SetTool(Transform)` shows the Text tab again without a click; selecting an image stamp with the Transform
  tool shows Layers; clearing the selection under the Transform tool shows Layers.
- `the_gap_selection_dies_with_the_session`: select the gap (stamp 7, after 2); replace the session by a new
  document that has a text with the same id: `text_gap` is `None`, no slot is framed, and `+` is disabled.

Target (which text the rows edit):
- `the_text_tool_edits_the_draft_not_the_selected_text`: place the text "One" (it is selected, the tool is
  Transform); `SetTool(Text)`; the content field shows the draft, not "One"; type "Bob": `tools.text.content`
  is "Bob", no action was queued, the stamp still reads "One", and the row "Selected: One" with its Edit button
  is there; a click on B changes `tools.text.bold_em` and not the stamp; click the model: the second stamp reads
  "Bob".
- `the_transform_tool_edits_the_selected_text` (the same rows under the Transform tool send
  `UpdateStampSource` for the selected stamp and leave the draft alone).

Gap spacing:
- `a_slot_click_and_a_drag_change_one_gap`: select the text stamp "WTE 2026", open the Character page, click the
  slot between "E" and the space, drag the value field over three frames to about 30 %: `Preview`s, then one
  `Final` whose spec has exactly one gap entry `after = 2` with a value between 0.25 and 0.35; the undo count
  grew by 1.
- `nudge_and_reset`: `+` twice: two `Final`s, the value is 0.02 more; Reset: the entry is gone; Reset all with
  three entries: an empty list, one `Final`.
- `typing_keeps_the_gap_on_its_character`: with the gap after "E", click into the content field, move the cursor
  to the start, type "X": the entry is `after = 3`; type "a" into "aaaa" at index 1 with a gap after index 1: the
  entry is `after = 2`.
- `the_cursor_in_the_field_selects_a_gap` (cursor between "T" and "E": `session.text_gap` has `after == 1`).
- `the_strip_shows_one_line` (text of 3 lines: "line 1 of 3"; `>` shows the slots of line 2; the last character
  of a line has no slot).
- `the_end_slot_shows_a_stored_value`: "WTE 2026" with a gap after the first "2" (index 4); delete "026" in the
  content field: the strip shows a dashed end slot after "2" with the value in its hover text; click it, click
  Reset: the gap list is empty, one `Final`, the end slot is gone.
- `the_strip_lays_out_only_visible_cells`: a line of 1,000 characters: the strip created at most 14 boxes and 14
  slots in a frame (counted by a test counter in the strip code), the panel is 300 pt wide, and scrolling to the
  end shows the last character.
- `gap_texts_name_the_real_pair` (the slot after "E" in "WTE 2026": hover text and status bar say "Gap between E
  and (space)").
- `gap_controls_are_disabled_before_placing`.

Other controls:
- `bold_formats_the_whole_text` (Ubuntu: `fx.bold_em = 0.02`, one `Final`; a second click: 0).
- `an_embedded_font_can_be_given_to_another_text`: a document with two texts; the first uses a font the catalog
  of the test does not know (the bundled Hack bytes registered under the family name "Projectonly"). Select the
  second text, open the picker: "Projectonly" is listed under "In this project"; click it: the `Final` spec has
  the `FontRef` of the first text (same blob), no toast was shown, and no blob was added. With the Text tool:
  the same click sets the draft's family, and the placed text uses that blob.
- `a_font_change_resolves_bold_again` (with a hand-made catalog that has "Fam" Regular and Bold): a text in
  Ubuntu with B (`bold_em = 0.02`); pick "Fam": the face is "Bold" and `bold_em` is 0; pick Ubuntu: the face is
  "Light" and `bold_em` is 0.02 again.
- `a_change_before_placing_changes_the_draft_and_queues_nothing`.
- `idle_text_controls_queue_no_action` (the four rows and every page, 10 idle frames, with a text that has
  every feature set).
- `the_font_picker_filters_and_takes_a_font`: click the font button, type "ha": the only row under "All fonts"
  is "Hack"; Enter: the family is "Hack", the popup is closed, the side panel is still 300 pt wide.
- `the_warp_page_applies_a_style_and_one_drag_is_one_entry`: click "Warp: None": `side_tab` is `Text`, the page
  is Warp; click "Arc": one `Final` with `bend = 0.5`; drag Bend over three frames: `Preview`s, then one
  `Final`; the undo count grew by 2.
- `the_path_page_creates_shapes_that_move_nothing`: choose Curve: one `Final` with 3 points; the baked bounds
  and the texel of the anchor are unchanged within 1 texel; Add point gives 4, Remove point 3; never below 2 or
  above 16.
- `the_path_page_creates_a_centred_circle`: a left-aligned "WTE 2026": choose Circle: one `Final`; the centre of
  the baked ink is within 2 texels of the vertical line through the old text centre (in the frame of the stamp)
  and the ink lies above the circle centre. Append 12 letters through `UpdateStampSource`: the hint "The text is
  longer than the circle" is shown; click Fit: one `Final`, `circle_fill` is 0.5 within 2 %, the hint is gone.

The Angle field ("angled"):
- `the_angle_field_turns_the_right_text`: (a) Text tool, a placed text "One" still selected: type 30 into Angle
  and press Enter: `tools.stamp.rotation_deg` is 30, no action was queued, the undo count is unchanged, and
  `placement_rotation_deg` of "One" is what it was. (b) `SetTool(Transform)`: the Angle field shows the rotation
  of "One" (0); type 30, Enter: exactly one `UpdateStampPlacement` `Final` for that stamp and no
  `UpdateStampSource`; `placement_rotation_deg` is 30 within 0.01 degrees; the centre of the placement moved by
  less than 0.5 texel; the undo count grew by 1; the draft rotation is still 30 from step (a) and was not
  written. (c) The existing Rotation field among the placement fields (`app\panels\tool_options.rs:641-646`)
  and the angle the gizmo knob is drawn at both read 30 in the same frame. (d) Type 270: the field shows -90 and
  the placement is at -90 (the range is -180 to 180 degrees, wrapped, not clamped). (e) A drag of the field over
  three frames: `Preview`s, then one `Final`; the undo count grew by 1. (f) Undo three times (the drag, the 270, the 30): after each undo the
  rotation is the value before that edit, and after the third it is 0 again.

One sweep over every control, like test A13 of the brush design:
- `every_text_control_changes_exactly_its_own_field`: a selected text "WTE 2026" under the Transform tool. A
  table lists every control of the four rows and of the three pages with the field of the spec it owns: B
  (`font` or `fx.bold_em`), I (`font` or `fx.slant_deg`), Text size (placement scale), Angle (placement rotation),
  Colour; Character page: Align, Line spacing, Letter spacing, Thicken (`fx.bold_em`), Slant (`fx.slant_deg`),
  Outline; Warp page: each of the 15 styles (`fx.warp.style`), Bend, Horizontal, Vertical, Direction, Reset;
  Path page: Kind, the Arc Bend, Diameter, Start, Other side, Fit. For each row the test operates the control
  with real pointer or key events (a click for a button or a switch, a drag over three frames for a slider or
  a value field), then compares the spec and the placement before and after field by field through
  `TextSpec::floats` and the non-float fields: **the listed field changed, no other field changed**, exactly one
  `Final` was sent for a click and `Preview`s followed by exactly one `Final` for a drag, and the undo count
  grew by exactly 1. The same table is then run under the Text tool: the same field of the draft changes and no
  action is queued (the gap controls and Curve are disabled there and are asserted to be disabled). A control
  that is added to a page later and is missing in the table fails the test: the table's length is compared with
  the number of interactive widgets the three pages and the four rows report.

[W] Only if the floating container is chosen (6.1): the window rectangle is inside the view area and does not
intersect the side panel at 1280x800; the 3D view rectangle equals the one without the window; no toast
rectangle intersects it; its close button is reachable at 640x480.

Unit tests next to the tools:
- `stamp_place.rs`: `text_source_copies_fx`, `a_styled_draft_is_placed_with_its_anchor_at_the_click` (within
  0.5 texel), `a_plain_draft_is_placed_as_in_round_3` (bit-equal placement).
- `stamp_place.rs`, the hover picture of 5.14 (tested through the tool's `hover` function and its accessor
  `hover_preview`, without a painter):
  `hover_with_the_text_tool_caches_a_preview_of_the_draft`: Sphere sample, Text tool, a draft "WTE 2026" with
  Arc 50 %; one `hover` over the model: `hover_preview()` is `Some`, its spec equals the draft's spec field by
  field, and the picture has at most `1 << 18` = 262,144 pixels and at least one inked pixel; a second `hover`
  with the same draft returns the same `Arc` (pointer-equal: nothing is drawn twice). With the image Stamp tool
  `hover_preview()` is `None`.
  `a_changed_draft_replaces_the_hover_preview`: after the first hover, change the draft content to "Bob" and
  hover again: the cached spec reads "Bob" and the `Arc` is a different one; change only the Angle: the `Arc` is
  the same (the rotation is not part of the picture).
  `add_stamp_reuses_only_a_full_preview`: with a small draft whose full picture has fewer than 262,144 pixels the
  cached picture is `full`, and the click that places the text makes no second picture (a counter of
  `text_preview` and `render_text` calls in the test build stays the same during `add_stamp`); with a draft at
  512 px per em whose full picture is larger, the cached picture is not `full` and the click makes exactly one
  full picture; in both cases the placed stamp's raster is `full`.
- `text_path.rs`: `points_map_to_the_screen_and_back` (within 0.001 em, 2D and 3D),
  `points_win_over_corners_and_lose_to_the_knob`, `dragging_a_point_sends_previews_and_one_final` (a drag of
  40 pt changes only that point; one history entry), `esc_restores_the_curve` (press on a point, move, Esc while
  the button is down, through the tool's `press`, `drag` and `cancel` functions: the spec equals the one before
  the drag and the history has no new entry; this is the only place the Esc case is tested, because a scenario
  cannot press a key during a pointer step, section 3.6), `a_locked_text_refuses_the_drag`,
  `a_drag_far_out_stops_at_the_range` (the point ends at 60 em from the anchor),
  `a_refused_preview_keeps_the_last_point` (a text so long that the Curve picture fits no resolution once the
  point is beyond 20 em: the drag stops updating there, the spec keeps the last accepted point, one status
  message, release gives one history entry).
- `text_block.rs`: `resolve_face` over hand-made candidate lists (Regular and Bold; Light only; Bold only; Italic
  and Bold Italic; two widths), each for B on, B off, I on, I off and a family change, and these cases by name:
  **the family is not installed and the current face is a real Bold from the project** (B is lit; B off with no
  regular face among the candidates: nothing changes, `bold_em` stays 0, the hover text is the "No regular
  face" one; B is never answered with thickening on a bold face); **a font change while thickened** (to a
  family with a Bold face: that face and `bold_em` 0; to a family without one: `bold_em` kept); **a style pick of
  a bold face while thickened** (`bold_em` becomes 0); a thickening the user set to 5 % survives a family change
  to another family without a bold face. `candidates` merges project and catalog faces without duplicates.

### 8.3 Self-test scenarios (new rows in ARCH 8.11; steps are actions and `fn` pointers, no typing)

| Scenario | Steps | Asserts |
|---|---|---|
| `text_gaps` | Sphere; place "WTE 2026" at 6 %; remember the baked bounds of the text layer; one `UpdateStampSource` `Final` that sets the gap after "E" to 0.5 em; undo; redo; the same once in the 2D view with a UV text; screenshot | 3D: the baked bounds are wider than before the edit; 2D (the text is left-aligned): the bounds are wider by 0.5 em in texels, within 2 texels, and **their left edge moved by less than 1 texel** (the start of the text stays, the letters after the gap move); the undo label is "Edit Stamp"; after undo the layer hash is the one from before, after redo the one from after; the layer is still of kind text |
| `text_styles` | Sphere; place "WTE 2026"; one `Final` with thickening 0.02 em and a slant of 12 degrees; undo; redo; screenshot | the layer has more inked texels than before; undo and redo give the two hashes; one history entry |
| `text_warp` | Sphere; place; Arc, then Flag, then Fisheye at 50 % (a table of `fn`s, as `fuzz_path` does); the same once in the 2D view with a UV text; screenshot with Arc | after each step the baked bounds differ from the step before; the anchor moved by less than 0.1 % of the model diagonal (0.5 texel); the 3D view rectangle did not move |
| `text_path` | Sphere; place; Arc 25 %; Circle; Curve; a pointer drag of the middle point by 40 pt upward, its screen position computed at run time in a `Dyn` step from `text_path::path_points` (a `Setup` step before it calls `stamp_text_frame`); screenshot. Only existing step kinds are used: a pointer step always ends with the release, so cancelling a drag with Esc is tested in the unit test `esc_restores_the_curve` (8.2) and not here | each kind changes the baked bounds; after choosing Circle the centre of the baked ink is within 2 texels of where the centre of the straight text was, across the reading direction; after the drag `points[1].y` is smaller and the history has exactly one more entry; the first and last point moved by less than 1.5 pt on screen |
| `text_tab` | Sphere; Text tool; `Step::Hover` onto the centre of the 3D view (no press; the step kind the shared scaffold adds for the brush scenarios), wait until idle, screenshot `hover` (the see-through text inside the yellow frame); each page of the Text tab in turn; place a text; each page again; `SetTool(Text)` with the text still selected, one action that sets the draft content, `SetTool(Transform)`; back to Layers; screenshot at 1280x800 with the Character page and a selected gap | after the hover step: the Text tool's `hover_preview()` is `Some` and its spec equals the draft (this is the assert for the picture before placing; the screenshot is for the report); on every page: the Color section rectangle is inside the side panel, the side panel is 300 pt wide, the 3D view rectangle equals the one with the Stamp tool; the content of the placed stamp is unchanged by the draft step; after switching back the Layers footer buttons are inside the window |
| `text_fonts` (this machine; logs SKIP per missing family, except for the two hard asserts) | Wait for the font job. Arial: B; Ubuntu: B; Candara: style list; the first installed `.otf` family and Bahnschrift: a thickened, Arc-warped "Ag" | **Hard asserts, never skipped:** the catalog holds at least 50 families (the round-3 run on this machine logged "font catalog: 78 families", `target\round3_audit\runs\text_builtin\shot.log:26`; the two bundled families alone would be 2), and the family "Arial" is among them and is not one of the bundled ones. The scenario logs the family count and the count of families that have more than one face. Then: Arial gets the face "Bold" and `bold_em` stays 0; Ubuntu gets `bold_em = 0.02`; Candara lists "Regular" and "Regular (Candara-Light)"; the two rendered pictures have more than 200 inked pixels and no fully inked row or column |
| `roundtrip` (extended) | save and reopen a project with a text that has two gap values and a warp | the reopened `TextSpec` equals the saved one; the layer hash is unchanged; one more `Final` changes the hash |
| `export_run` (extended) | export with that text | one `_text_` layer file as today; the round-3 Pillow check of PSD and ORA passes |
| `fuzz_path` (extended) | four new operations in its table: a random gap value, a random warp, a random line, clear `fx` | no panic, no "Invalid parameter" in the log, the structure check passes |

The existing scenarios `text`, `default`, `import_save` and `workflow_path` must pass unchanged, including "the
Text tool options leave the 3D view where it was" (`app\selftest\scenarios.rs:428`).

### 8.4 Performance limits (release build, idle machine, median of 5 runs, measured by the gate)

Ignored timing tests in `core\text\styled.rs` and `core\doc_stamps.rs`:

| Measurement | Limit |
|---|---|
| Classic painter, the three baseline cases of 7.5 | within 5 % of B1, B2, B3 |
| `text_frame`, 200 letters in 4 lines with Flag, 128 px per em | 5 ms |
| Preview picture (1 megapixel) of "WTE 2026", Arc 50 %, stamp at 512 px per em, with its mip chain | 8 ms |
| the same with thickening 2 % | 12 ms |
| the same with a 6 % outline | 45 ms (estimate 30 ms: the preview is 0.64 megapixels, not 0.42 as first assumed) |
| Preview picture of 40 letters, Flag, stamp at 256 px per em, no outline | 12 ms |
| Final picture of "WTE 2026", Arc 50 %, 512 px per em (2.6 megapixels) | 40 ms; with thickening 2 %: 60 ms; with a 6 % outline: 250 ms |
| Final picture of "WTE 2026" on its default Circle, 512 px per em (3.6 megapixels) | 50 ms; with a 6 % outline: 350 ms |
| Final picture of a line of 16 em around a full Circle, 512 px per em (12.7 megapixels, the square case) | 150 ms; with a 6 % outline: 1,200 ms |
| Preview picture (1 megapixel) of the same full-circle text, with a 6 % outline | 60 ms |
| Peak working memory of that Final with an outline (allocation counter of the test build) | 250 MB; without the outline: 120 MB |
| `text_frame` for the too-large fallback: a line of 16 em on its default Circle, tried at 512 and 256 px per em | 1 ms for both tries together |
| Final picture of 200 letters in 4 lines, Flag, 128 px per em | 40 ms |
| Classic painter with gap values: "WTE 2026" with 3 gap values at 256 px per em | within 5 % of B1 |
| `set_stamp_source(.., Preview)` with deferred bakes, styled text of 40 letters at 256 px per em, no outline, sample sphere | 16 ms (one frame) |
| Font sample job, all families of this machine, warm file cache | 0.5 s |
| B4, the first `fontdue` parse of a font | reported; not slower than in round 3 |

In the app:
- `text_warp` on `E:\claude_downloads\glb_samples\ABeautifulGame.glb` (1.5 million triangles): every `Final`
  step within the round-3 limit for an image stamp on that model (1500 ms,
  `target\round3_audit\ROUND3_PLAN.md:195`), and no frame above 100 ms outside the bake segment of the frame
  profile during the `Preview` steps.
- Twenty `Preview` edits of a selected styled text on an 8192 texture (sent as actions): the longest frame in the
  `dispatch` segment is at most 30 ms plus the round-3 preview-bake time.
- `workflow_path`, step "text stamp", on the same model: within 10 % of the round-3 result.
- Opening the font picker with all families: no frame above 16 ms in the "side panel" segment.
- The Character page with a text line of 1,000 characters selected, 20 idle frames and 20 frames of scrolling
  the strip: no frame above 16 ms in the "side panel" segment (panel timing check next to
  `the_strip_lays_out_only_visible_cells`).
- `text_path` on `E:\claude_downloads\glb_samples\ABeautifulGame.glb`: the `Final` of the Circle step within the
  same 1500 ms limit as `text_warp`.
- The font scan at start is not slower than in round 3 by more than 0.05 s.

## 9. Risks, what is left out, and what became of the judge findings

### 9.1 Risks

1. **Nothing was measured.** All times in 5.15 are estimates. The baseline of 7.5 and the limits of 8.4 decide.
   If thickening misses its limit the distance transform can run on a half-resolution copy of the glyph; if a
   plain text with an outline is felt to be slow while typing, that is the cost of today and outside this
   design.
2. **The round-3 result is not known yet.** The visible height of the tool options, the shape of the text
   picture cache and the layout code may differ from what is described. Gates 2 and 3 of 7.5 and the table of
   7.4 are the mitigation.
3. **The gap buffer rests on egui behaviour that was read, not tried** (6.2 rule 6). It is proven by the app
   half of the scaffold, before the contract types are fixed and before any package builds on them; the
   text comparison is the fallback and is exact except among equal neighbouring letters.
4. **A gap value belongs to the character on its left.** Text typed into a widened gap appears on the right of
   the gap. This is a rule the user may not expect; the strip shows where each value sits.
5. **Many gaps need many clicks.** The strip sets one gap at a time; spacing a whole word evenly is what Letter
   spacing is for. A long line scrolls in the strip.
6. **Shared ground with Feature 1**: the tab strip with three tabs, the tool-change rule, the ownership of the
   panel folder (7.3). If the two plans are merged without these decisions the same files get two owners.
7. **The Text tab hides the layer list** while it is open (Q4).
8. **Outlines against `fontdue`.** Same parser, same glyph index, same advances, so positions agree by
   construction; three tests guard it. Bitmap-only and colour fonts have no outlines and stay blank in both
   painters, as today. Real bold faces, CFF fonts and variable fonts cannot be tested with the two bundled fonts;
   pure functions with hand-made data and the `text_fonts` scenario cover them on this machine.
9. **The warp shapes are ours.** Someone who knows the Shell, Fish or Squeeze of Photoshop by heart may expect
   other shapes. Names, sliders and ranges match; the formulas are one table and one script and are easy to
   tune, as long as `no_style_folds_over` keeps passing.
10. **A preview is softer than the final picture**, by a factor of up to 4 in each direction for the largest
    text. The user sees the text sharpen when a drag ends.
11. **A preview can now lower the resolution** of a stamp (the fallback of 5.11). It happens only where the edit
    would fail today.
12. **Font files are large.** A font tried on a text stays in memory until the document closes and the font of a
    text is embedded whole, as today; the first use of a font parses it on the UI thread. The picker shows sizes
    and samples; nothing is freed (5.13).
13. **Second and later lines on a tight curve** bunch up on the inner side. The first line is always exact.
14. **Thin thickening at low resolution.** At 32 px per em a 2 % thickening is 0.64 px and barely visible; the
    resolution rule gives 32 px per em only to texts whose em is 32 texels or less.
15. **The anchor rule moves the origin of a projected text inside its plane.** After a strong warp the centre of
    the rectangle can lie slightly off the surface. An automatic depth is re-fitted as today; a depth typed in
    by hand may need a new value.
16. **Compiling against `ttf-parser` directly is not verified** (5.1); gate 4 of 7.5 does it, with a named
    fallback.
17. **A large styled picture with an outline is slow once per edit.** At the area limit of 16.8 megapixels the
    distance transform of the outline takes about 0.7 s and 134 MB on the UI thread at every `Final` (5.15).
    Previews stay at 1 megapixel. This is bounded now; a plain text has had the same kind of cost, unbounded,
    since round 2. If the measured number is felt, the outline of a styled text can be computed at half
    resolution (the same remedy as risk 1).
18. **A text longer than its Circle overlaps itself.** The circle keeps its size by design; the app only shows a
    hint and offers Fit (5.5, 6.4).
19. **Spacing and typing anchor a text differently.** A gap value or Letter spacing keeps the alignment side in
    place; typing still grows the text about its centre, as today, until the user answers Q8. Letter spacing of
    a left- or right-aligned placed text therefore behaves differently from round 3 (1.3).
20. **The project's copy of a font wins over the installed one** when both have the same family and style name
    (5.13). After a font update on the machine, texts of an old project keep the old shapes, also new texts in
    that font; this is what keeps a project stable, but it can surprise.
21. **B and I are derived, not stored.** They follow the weight and italic flag inside the font file. A font with
    wrong values in its OS/2 table (Candara Light reports 400, section 3.1) is treated by what it reports.
22. **The gap history of the content field matches states by their text.** If egui ever restores a text that
    was never in the field, the fallback comparison is used and a gap can slip among equal neighbouring
    letters (5.12). The first panel test of T6 proves the normal path.
23. **The brush plan must be edited at the merge** so that the scaffold builds the tab strip and D1 does not own
    the four text files (7.3). Until then the two documents disagree on that one row.

### 9.2 Deliberately left out

Not requested, by the decision of the user on "segmented" (one line each, not designed, not planned):
- Own font, size, colour, bold or slant for parts of one text. Bold, slant, font and colour apply to the whole
  text.
- Letters or words as separately movable pieces.
- A stencil look (gaps cut through the letters).

Left out for other reasons:
- Typing on the model with a text cursor; gap handles in the view (gaps are edited in the side panel).
- Keys for the gap value (the Alt+arrow of Photoshop) and Ctrl+B / Ctrl+I: every key needs a row in the one
  shortcut table (ARCH:2822-2824), and none was asked for.
- Gap values before the text is placed.
- A line drawn across the 3D surface with every letter projected on its own (Q2).
- OpenType shaping: ligatures, kerning from the newer table, right-to-left and complex scripts.
- Styles of variable fonts; fonts known only to the Windows registry; loading a font file by hand (Q5).
- Turning each letter on the spot (Q3).
- A click with the Text tool that selects an existing text; remembered text settings; a rescan button (Q6).
- Text written onto an image stamp, or text as a kind inside the Stamp tool (Q7).
- Typing that keeps the alignment side of a placed text in place (Q8); only spacing changes do.
- Editing a placed text while the Text tool is active: the Text tool always edits the text to place (6.2).
- Shrinking or cutting a text that is longer than its Circle; wrapping it onto a second turn.
- Correct spacing rules for combining marks beyond the fixed ranges of 5.12, and for Letter spacing after a
  mark (unchanged from today).
- A project reader that survives a wrong JSON type inside `fx` (4.3 item 4).
- Underline, strikethrough, caps, superscript, baseline shift.
- Paragraph boxes with word wrap and justification; vertical text.
- A free warp grid; handles for the centre of a Circle (the diameter is a number, the stamp is moved as a
  whole).
- Freeing unused font blobs; font subsetting; rasterizing on a worker thread; a preview mode for texts on the
  classic painter.
- Raising the 512 px per em limit; reading a project stamp by stamp (the string fields of 4.3 make unknown
  names harmless without it).
- Editable text layers in PSD or ORA export: export keeps writing the baked pixels.

### 9.3 What became of the judge findings

The judges scored three proposals that all read "segmented" as formatting for parts of a text. The user has
since decided otherwise (section 2), so every finding about parts ("spans") no longer applies; those rows say
so. Each remaining finding was checked in the source. All of them held, except where noted.

| Finding (judge) | What this design does |
|---|---|
| Anchor must be the centre of the unwarped line box, one rule in three places, with flips and a stretched rectangle (all three) | 5.2 defines the anchor; 5.11 gives one formula and the three places; tests `refit_keeps_anchor_and_em`, `switching_the_painter_moves_no_ink`, `repeated_refits_do_not_drift`. |
| The classic canvas pads by the overhang and rounds the baseline; the styled layout left the baseline unrounded (architect) | Checked at `core\text\layout.rs:138-156`, `:142`. 5.2 repeats the three roundings; 5.10 states the tight styled canvas and why the anchor rule absorbs the difference. |
| The frame is missing after load and undo (all three) | 5.11: `stamp_text_frame` rebuilds it from geometry without pixels; who calls it and what it costs is stated; test `the_frame_needs_no_pixels_after_undo_and_load`. |
| A bad spec from a file reaches the layout unchecked (architect) | Checked at `core\project\manifest.rs:269-288`, `core\project\load.rs:228`. `sanitize_fx` repairs, also in `to_spec` (4.1). |
| One hand-written float list (architect, implementer) | One field list, two visitors (`floats`, `floats_mut`), feeds both uses (4.1, 4.3 item 6); the count is tested. |
| Unknown enum values must not fail a project; per-entry stamp parsing is not asked for (architect against implementer) | Both met: `style` and `kind` are strings (4.3 item 4); the manifest parser is not changed. |
| Squeeze folds at the frame edges (user advocate); Fisheye of the engine proposal folds (user advocate) | Confirmed by calculation. Replaced; Bulge and the distortion factor were found to fail the same strict test and are replaced too (5.6). All 15 checked with a script. |
| Outline edges must be cut by their mapped length (implementer) | 5.6, outline refinement; test `refinement_follows_the_curve`. |
| Bold must not rely on summed windings (implementer) | Thickening by distance transform on the finished coverage (5.9); 5.7 says that no stroke is summed. |
| The preview cost is not bounded (all three) | 5.8: 1 megapixel previews that cover the same rectangle; limits in 8.4. |
| Positions from a text comparison go wrong among equal letters (architect, implementer, on the lean proposal) | The same problem exists for gap values. Exact positions come from the egui buffer; the comparison is used only for the undo of the field and as the fallback (5.12); test `typing_keeps_the_gap_on_its_character`. |
| `Tool::overlay` cannot upload a texture; `Drag` and `GizmoShape` are `Copy` (implementer) | Checked at `app\tools\mod.rs:202`, `app\tools\transform.rs:22-42`, `app\view\gizmo.rs:28`. 5.14 keeps the texture behind a mutex in the tool; 6.5 gives the point drag its own state; neither file of the two types changes. |
| 278 pt rows; the 140 pt are an estimate with no slack (architect, user advocate, implementer) | Rows budgeted against 265 pt; rows 1 to 3 need 116 pt; the height is measured first (6.1, 7.5). |
| One home for extra settings with Feature 1; the floating window is the first of the app (all three) | A tab next to Layers and Brush, as `FEATURE_BRUSH.md` chooses; one function that also fits a window; Q4 for the user (2, 6.1, 7.3). |
| Circle radius follows the text; uniform Catmull-Rom overshoots (architect, implementer) | Arc keeps the "share of a circle" meaning and says so; Circle has a fixed diameter; the Curve is centripetal (5.5). |
| "Bold 2" for equal faces; the real case is Candara Light (user advocate) | PostScript name in brackets (5.13). |
| Click on an existing text needs a hit area; Size changes meaning; saved settings; rescan (all three: not asked for) | Not built; Q6. The em size is shown as a new field "Text size", the existing "Size" stays (6.2). |
| Delete and double-click for points collide with existing keys (user advocate, on the engine proposal) | Buttons only (6.4). |
| Font copies on every call; unused blobs (architect, user advocate) | A font the project knows by name is not read from the machine again (`find_font`); one allocation per font file and a fast path in `insert` for the first use; nothing freed, said plainly (5.13, 1.4, 6.3). |
| Offline proof and baselines before the packages (all three) | Resolution run for the exact line (5.1); build, hashes and timings are gates (7.5). |
| Findings that concern parts of a text: the three one-font places, 256 spans against one index byte, the focus given back after a format click, Ctrl+B and Ctrl+I, mixed values, fonts per part multiplying memory (all three) | No longer apply: a text keeps one font, one colour and one style. The focus of the content field is handled exactly as today. |

One finding was only partly taken over: "tolerant project reading, stamp by stamp" (architect, graft from the
engine proposal). The need behind it, that an unknown name must not fail a file, is met by the string fields;
reading the stamp list entry by entry would change the loader that round 3 is editing, for a case
`min_reader_version` already prevents.

## 10. Review log

Three independent skeptics reviewed the first version of this document: one read it against the code, one
against the data model and the file format, one walked through user tasks. Each finding below was checked in
the source before it was acted on. "Confirmed" means the evidence was re-read and holds. No finding was rejected
as a whole; three were taken over in part, and the part not taken is named.

| # | Lens, weight | Finding | Checked | What was done |
|---|---|---|---|---|
| 1 | code, major | A Circle gives a square picture up to 67 megapixels; only the side limit bounds it; costs and limits cover strips only. | Confirmed (`core\text\layout.rs:10`, `:157-160`; `core\text\edt.rs:35`, `:67`). One number of the finding is too high: "WTE 2026" on its default Circle covers the upper half, so its picture is about half as high as wide (3.6, not 5.8 megapixels). The square case exists for a text that goes all the way round. | Area limit `MAX_STYLED_PIXELS` = 16.8 megapixels for styled pictures (4.1, 5.10); the fallback of 5.11 lowers the resolution for the side or the area; two Circle columns and the peak memory in 5.15; Circle rows in 8.4; tests `a_styled_picture_has_an_area_limit`, `an_area_overflow_drops_the_resolution`; risk 17. |
| 2 | code, major; user, major | A new Circle with the default alignment Left runs from 12 o'clock down the right side. | Confirmed by calculation (`app\state.rs:226`; the old formula `s0 = 0` for Left). | Circle alignment is measured against the half circle centred on the reference point (Left starts at 9 o'clock, Right ends at 3 o'clock); the creation rule centres every alignment; `start` centres a very short text (5.5, 1.2, 6.4). Tests `a_new_circle_is_centred_on_its_top`, `circle_alignment_pins_its_side`, `the_path_page_creates_a_centred_circle`; an assert in scenario `text_path`. |
| 3 | code, minor | `sanitize_fx` has no range for Curve points and the Circle centre; Start can move the text 100 em away. | Confirmed in the first version of 4.1. | Coordinates clamped to 60 em (every storable path then has a picture at 32 px per em); Start limited to one line length in the layout, an angle on a Circle (4.1, 5.5); a refused or far drag keeps the last valid point (6.5). Tests in 8.1 and 8.2. |
| 4 | code, minor; user, minor | A text longer than its Circle overlaps its start; nothing says so. | Confirmed. | Behaviour stated (1.2, 1.4, 5.5); `circle_fill` in the frame; hint and Fit button on the Path page (6.4); risk 18. |
| 5 | code, minor | The sample maps characters with `Face::glyph_index` (Unicode subtables only), the painter with fontdue (all subtables). | Confirmed (`REG\ttf-parser-0.25.1\src\lib.rs:1839-1851`, `tables\cmap\mod.rs:61-83`; `REG\fontdue-0.9.5\src\font.rs:263-275`). The subtables of the four symbol fonts were not re-measured. | `outline::char_to_glyph` walks every subtable, last mapping wins (5.13); test `the_sample_maps_like_the_painter`. |
| 6 | code, minor | `file_bytes` would need a file look-up per `faces()` call; the weak reference per face misses collection files. | Confirmed (`core\text\font_catalog.rs:101-138`). | File sizes read once into a map; weak reference per font file (5.13). |
| 7 | code, minor | Thickening pushes a combining mark off its letter; the strip offers gaps inside joined pairs. | Confirmed by calculation; `char::is_control` does not drop U+200D (`core\text\layout.rs:108`). | No bold term for a zero-advance glyph and a mirrored shift (5.2, 5.4); no slot inside a joined pair, `sanitize_fx` removes such a value (4.1, 5.12); tests. **Not taken:** dropping Letter spacing after a zero-advance glyph. The code of today adds it (`core\text\layout.rs:100`), and a plain text must stay bit-identical. |
| 8 | code, minor | "Changes no pixel" for a new Curve or an Arc at bend 0 is not true. | Confirmed from 5.2 and 5.10. | Reworded to "moves no letter by more than one raster pixel", with the reason (1.2, 5.5). |
| 9 | code, minor | The gap strip lays out every character of a long line in every frame. | Confirmed (no virtualization in the first version). | Fixed cells of 24 pt, only the visible ones are laid out (6.2); test and a timing check with 1,000 characters (8.2, 8.4). |
| 10 | code, minor | Three buttons of 84 pt cut off their default labels. | Padding confirmed (`app\theme.rs:20`); label widths are estimates. | Row 4 is 100 / 104 / 48 pt; the panel test measures the default labels; a named fallback (6.1, 8.2). |
| 11 | data, major | A font that is only embedded in the project cannot be given to a second text or to the draft. | Confirmed (`app\panels\tool_options.rs:345-355`, `app\tools\stamp_place.rs:48-60`, `core\text\font_catalog.rs:101-133`). | Resolution order project first, machine second (`resolve_font`, `Document::find_font`); picker rows carry the `FontRef`; the style list merges project and installed faces (5.9, 5.11, 5.13, 6.2, 6.3). Test `an_embedded_font_can_be_given_to_another_text`. |
| 12 | data, major | Bold and italic are derived from the system catalog; wrong on a machine without the family and after a font change. | Confirmed (`core\text\mod.rs:51-56`; fontdb reads OS/2 and `post`, `REG\fontdb-0.24.0\src\lib.rs:1039-1043`, `:1147-1167`). | The truth is the font bytes of the text plus `fx`: `face_traits` reads weight and italic from the blob; `resolve_face` runs on B, on I and on every family change; a style pick clears a doubled thickening; the case "no regular face anywhere" is defined (5.9). Tests in 8.1 and 8.2. No flag is stored, so the file format of 4.3 is unchanged. |
| 13 | data, major; user, minor | Ctrl+Z in the content field brings a deleted character back without its gap value. | Confirmed (`REG\egui-0.37.0\src\widgets\text_edit\builder.rs:1345-1350`, `:1363-1368`; `app\panels\mod.rs:125-133`). | `GapBuffer` keeps a history of (text, gaps) states for the focus session and restores by text in `replace_with` (4.4, 5.12). Tests `field_history_restores_gaps`, `field_undo_brings_the_gap_back`. |
| 14 | data, minor; user, major | The document promised that letters before a gap stay, but the block grew about its centre. | Confirmed (`core\doc_stamps.rs:55-63`, `:367-371`). | Design changed, not only the wording: a change of spacing alone keeps the alignment side of the line box (`align_px`, `Keep::AlignEdge`, 5.11); 1.1, 1.3, 5.2 and decision 9 say the same thing; the `text_gaps` assert is reversed; tests `refit_keeps_the_alignment_edge`, `a_spacing_edit_keeps_the_alignment_edge`. Typing is left as today and asked as Q8. |
| 15 | data, minor | A gap value on the last character of a line is invisible but raises `min_reader_version`. | Confirmed. | The strip shows it as a dashed end slot with its own Reset (4.1, 5.12, 6.2); it keeps counting for the reader version, with the reason (4.3). Tests. |
| 16 | data, minor | The weak reference in `face_bytes` dies at once when the document holds the same bytes in another allocation. | Confirmed (`core\blob.rs:40-47`). | The copy is avoided before it happens: a font the project knows by name is not asked from the catalog (5.13). Test `find_font_prefers_the_project`. |
| 17 | data, minor | The selected gap in `AppState` survives a change of document. | Confirmed (`core\doc_stamps.rs:254`, `app\io_flows.rs:491-496`). | `text_gap` moved into `Session` (4.2, 6.2). Test `the_gap_selection_dies_with_the_session`. |
| 18 | data, minor | `for_each_float(&mut self)` cannot serve the save check on an immutable snapshot. | Confirmed (`core\project\save.rs:84-88`). | Two visitors from one field list, `floats` and `floats_mut` (4.1, 4.3 item 6). |
| 19 | data, minor | The JSON example does not match `PathEntry`; a missing `style` or `kind` fails the project. | Confirmed (`core\project\migrate.rs:73-74`). | Skip attributes on the path fields; `#[serde(default)]` on both names; tests (4.3, 8.1). **Not taken:** surviving a wrong JSON type inside `fx`. It would need a stamp-by-stamp reader, which was already decided against (9.3); the sentence in 4.3 item 4 now says exactly what is and is not repaired. |
| 20 | user, major | With a text selected, the rows of the Text tool edit that text, so placing a second text rewrites the first. | Confirmed (`app\tools\stamp_place.rs:210-218`, `app\dispatch.rs:631-635`, `app\panels\tool_options.rs:85-101`). | The tool decides the target: the Text tool always edits the draft, the Transform tool the selected text (6.2, 1.1 step 3b, 1.2). Tests `the_text_tool_edits_the_draft_not_the_selected_text`, `the_transform_tool_edits_the_selected_text`; a step in scenario `text_tab`. |
| 21 | user, minor | The scenario step "a drag ended with Esc" cannot run in the self-test runner. | Confirmed (`app\selftest\mod.rs:552-574`). | Removed from scenario `text_path`; the unit test `esc_restores_the_curve` covers it; no new step kind (8.2, 8.3, 7.1). |
| 22 | user, minor | Sections 7.1 and 7.3 disagree on who builds the tab strip and owns the panel files. | Confirmed; `FEATURE_BRUSH.md` was re-read and has changed its tab rule since. | Settled: the shared scaffold builds the strip; file-by-file ownership; one tab rule shared with the brush design; the edit the brush plan needs is named (6.1, 7.1, 7.3; risk 23). |
| 23 | user, minor | The picture size behind the cost table was too small by about 1.5 times in area. | Confirmed by recomputing the Arc formula of 5.6. | Header and columns recomputed for 2,660 x 960 px; the preview limit with an outline is 45 ms (5.15, 8.4). |
| 24 | user, minor | "The gap between E and 2" does not exist in "WTE 2026". | Confirmed. | "Between E and the space" everywhere; `slot_name` names a space and a tab (1.1, 5.12, 6.2). |
| 25 | user, minor | "Stamp type; text ... onto the stamp" had no reading. | Confirmed (`FEATURE_BRIEF.md`, the user's words). | A row in section 2 with the reading built, two other readings with their cost, and question Q7. |

Changes that follow from the findings without having been asked for by one: `add_stamp` uses the too-large
fallback too (a styled draft can be too large at the resolution the core picks); the picker shows "(in project)"
for a family without a sample; the gap history is limited to 200 states.

### Completeness check of 2026-11-17 (after the review)

A check of this document against the request and against the plan found these gaps. What was done:

| Gap | What was done |
|---|---|
| The tab rule contradicted itself: "enabled while a text is selected" against "`SetTool(Brush)` shows Layers", while `SetTool` keeps the selection (`app\dispatch.rs:631-635`). | 6.1 and 7.3: the Text tab is enabled only with the Text tool, or with the Transform tool and a selected text. The test is renamed `the_text_tab_follows_the_tool_and_the_selection` and asserts each case without a condition. |
| The picture of the text before placing (5.14) had no test. | 5.14: the accessor `hover_preview`. 8.2: three tests in `stamp_place.rs`. 8.3: a `Step::Hover` with an assert and a screenshot in `text_tab`. |
| The new Angle field had no test. | 8.2: `the_angle_field_turns_the_right_text`. |
| I, Thicken, Slant, the warp sliders, the Direction switch and the Path controls were tested in the core only. | 8.2: the sweep `every_text_control_changes_exactly_its_own_field`. |
| The frozen version-1 fixture had no owner and no point in time. | 4.3 item 5 and 7.5 gate 5: the scaffold adds it and runs it green before it changes the manifest. |
| The egui proofs for the gap buffer were written after the contract they decide. | 6.2 rule 6, 8.2, risk 3: the app half of the scaffold writes them. |
| `tool_options.rs` belongs to the brush panel package, but 6.1 needs text edits there. | 6.1: the scaffold makes them; T6 works behind two fixed function signatures in `text_block.rs`. |
| "Not touched: `core\history.rs`" against a `TextSpec` literal in one of its tests. | 7.1 names the one-line exception. |
| `text_fonts` could pass without one machine font. | 8.3: two hard asserts (at least 50 families; Arial present). Q5 is asked. |
| 7.5 gate 1 asked for answers before any package; the plan said nothing blocks. | 7.5 gate 1: Q4 before the app scaffold, Q2 before the first wave, with the stated risk if there is no answer. |

---

## Appendix: decided and open questions for the user

**Decided, not to be asked again.** "Segmented": the user answered on 2026-11-17 at 13:00, word for word,
"segmented means editing the space between each character". This design builds exactly that (sections 1.2, 4.1,
5.2, 5.12, 6.2).

**Open.**
1. **Q2 "curved along a line".** Is a curved text on one side of the model enough, or must a text be able to
   follow a line that runs around the model, for example once around a bottle? Default: on one side.
2. **Q3 "angled".** Do you mean the whole text turned (exists today), or each letter turned on the spot?
   Default: the whole text.
3. **Q4 where the settings go.** The extra text settings (and the brush settings) get a tab next to Layers;
   while such a tab is open you do not see the layer list. Is that acceptable, or do you prefer a small floating
   panel over the view? Default: the tab.
4. **Q5 fonts.** Is a font you use missing from the font list of the app today? Two kinds of fonts are known
   not to appear: a font that Windows knows only through its registry (installed from a folder outside the three
   Windows font folders), and the named styles of variable fonts (Bahnschrift, Segoe UI Variable, Sitka show one
   face each instead of "Light", "SemiBold" and so on). Default: both stay as they are. This question is put to
   the user with the others; it is not held back.
5. **Q6 extras, each yes or no.** (a) A click with the Text tool on an existing text selects it instead of
   placing a second text. (b) The app remembers the last font and text settings between program runs. (c) A
   button that reads the installed fonts again without a restart. Default: none of the three.
6. **Q7 "stamp type: text".** Is a text that is its own stamp, as the Text tool makes today, what you mean, or
   do you want to put text onto an image stamp so that both move as one? Default: its own stamp.
7. **Q8 typing.** When you type more letters into a placed left-aligned text, should its start stay where it is
   (the text grows to the right), instead of growing to both sides as today? Default: as today. (A change of the
   space between letters already keeps the start in place in this design.)
