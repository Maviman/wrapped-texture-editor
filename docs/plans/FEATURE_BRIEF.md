# Round 4 feature brief — Wrapped Texture Editor

Design only. Nothing here is implemented yet. Design documents go to
`E:\claude_programs\wrapped_texture_editor\target\round4_design\`. Do not change any source file.

## The user's request, word for word (2026-11-17)

> BRush type: circular vs sphere, where circular is what you have and sphere will glide along an object and paint
> whatever is in its sphere. i want to be able to upload and orient the shape and size of the "sphere" into any
> compatible model type. can be its own tab like in photoshop.
>
> Stamp type; text: being able to choose from fonts on the machine to put onto the stamp. the text can be warped,
> curved along a line, angled, slanted, bolded, or segmented like in photoshop.

## USER DECISION 2026-11-17 13:00 — what "segmented" means (binding; newer than the prompts of this design run)

The user was asked and answered, word for word: **"segmented means editing the space between each character"**.

So for the text feature:
- **Required:** the user can change the space between any two neighbouring characters of a text on its own — every gap
  separately (Photoshop: kerning between two characters, next to tracking for the whole text).
- **Exists today (verified):** one "Letter spacing" value for the whole text (`TextSpec::letter_spacing`, a fraction of
  the em, clamped to -0.5..=4; slider -20 %..100 % in `crates\wte_app\src\panels\tool_options.rs:254` and `:589`), and
  the font's own pair kerning, applied automatically (`crates\wte_core\src\text\layout.rs:82-88`, advance at `:100`).
  There is no per-gap value. The whole-text value stays; a per-gap value adds to it.
- **Not requested, so out of scope:** per-range character formatting (own font, size, colour, bold or slant for parts
  of one text), letters as separately movable pieces, a stencil look. List them under "deliberately left out" with one
  line each; do not design or plan them. Bold and slant apply to the whole text.
- **Do not ask the user about "segmented" again.** Where a prompt of this run says the word must be among the open
  questions, report it as decided and quote the answer above.

The text design must cover, for per-gap spacing:
- Data model: where the per-gap values live in `TextSpec`, their unit (a fraction of the em, like `letter_spacing`) and
  range; what a gap is keyed by, so that the values stay with their characters when the content is edited (insert,
  delete, replace, paste); what happens at line breaks, with tabs (laid out as four spaces today), with characters
  that are dropped, and with the fallback font.
- Layout: how the gap value combines with font kerning and the whole-text letter spacing; alignment and line width;
  how it carries over to text on a curve and to warped text (spacing is applied before the path / warp).
- User interface: how the user picks a gap and changes it — at least one way that works in the side panel without
  the 3D view (for example a strip that shows the characters with a clickable gap between each, a value field and
  nudge buttons), optionally handles in the view; reset one gap, reset all; what is shown for a multi-line text; how it
  fits the panel at 1280x800 with the colour picker visible.
- Undo (one entry per continuous edit), project format (a new optional field, empty by default, old projects load,
  version rule), export unchanged.
- Tests: unit tests in `text\layout.rs` (one gap moves exactly the glyphs after it on that line; values survive a
  content edit as specified; non-finite and out-of-range values are sanitized), a round-trip test, and a self-test
  scenario step that sets one gap and asserts that the ink bounds change.

## Feature 1 — brush types and a brush-shape volume ("brush")

What is asked:
- Two brush types: "circular" (today's brush) and "sphere" (a ball that glides over the model; everything inside the
  ball gets painted).
- The "sphere" can be replaced by a shape the user loads from a model file ("any compatible model type": the formats
  the app imports — OBJ, glTF, GLB, FBX). The user sets its orientation and size.
- The brush settings may live in their own tab, like Photoshop's Brush Settings panel.

Known facts to check, not to trust:
- The 3D dab is already described as "one spherical dab on the surface" with a world-space radius and a
  `same_side_only` switch (ARCHITECTURE.md 5.19 and 6.4; `crates\wte_core\src\brush.rs`, `stroke.rs`;
  `crates\wte_app\src\tools\brush.rs`). So the design must first say exactly what today's brush does to texels (which
  triangles, which side, connected or not, occluded or not) and then define "circular" and "sphere" so that the user
  sees a real difference. A likely reading: "circular" = a disc on the surface the cursor is on (today's result with
  "Same side only"); "sphere" = every surface texel whose world position is inside the ball, on any side, any part.
  Decide and justify; show what each paints on a thin wall, a concave fold and two nearby separate parts.
- A custom shape needs: import (reuse `wte_core\src\import\*`), a point-inside-shape test or a signed distance field,
  an orientation relative to the surface (follow the normal? fixed in world? rotate about the normal? align to stroke
  direction?), a size, fall-off (how do hardness / opacity / flow apply to a volume?), a cursor preview in the 3D
  view, behaviour in the 2D view, persistence (prefs or project? a brush library?), undo.
- Performance budgets: strokes must stay interactive on 4096 and 8192 textures and on models of 1.5 million triangles
  (`E:\claude_downloads\glb_samples\ABeautifulGame.glb`).

## Feature 2 — text stamps ("text")

What is asked:
- Choose from the fonts installed on the machine.
- Text can be: warped; curved along a line; angled; slanted; bolded; "segmented" — "like in Photoshop".

Known facts to check, not to trust:
- `crates\wte_core\src\text\font_catalog.rs` already has `FontCatalog::load_system()` (fontdb). Find out what the
  Text tool shows today: is the system font list loaded, complete, searchable, previewed? What is missing for
  "choose from fonts on the machine" to be true for the user?
- `TextSpec` (ARCHITECTURE.md 5.13, 6.8) has content, font, size, colour, align, line spacing, letter spacing,
  outline. Text is rasterized with fontdue, then baked as a stamp (3D projected or 2D UV; ARCHITECTURE 6.5). A stamp
  can already be rotated ("angled" may already exist: say so).
- Photoshop terms to map: Warp Text (Arc, Arc Lower/Upper, Arch, Bulge, Shell, Flag, Wave, Fish, Rise, Fisheye,
  Inflate, Squeeze, Twist, with Bend / Horizontal / Vertical distortion); type on a path; faux bold and faux italic
  next to real bold / italic faces; per-character formatting.
- "Segmented" is DECIDED: see "USER DECISION" at the top of this file — it means editing the space between each
  character (per-gap spacing). The earlier working reading (per-range character formatting) is withdrawn; proposals
  written before 13:00 used it and must be corrected in the synthesis.

## Constraints that bind both designs

- Rust only, `--offline`: no new crates unless they are already in the cargo cache
  (`E:\claude_downloads\rust-toolchain\cargo\registry\cache`). In the cache today, among others: fontdb, fontdue,
  ttf-parser, skrifa, read-fonts, harfrust, kurbo, peniko, vello_cpu, rayon, glam, image. A crate that is cached but
  not yet in `Cargo.lock` is a risk: say how to verify that it resolves offline. Anything else needs the user's OK
  and goes under open questions.
- No AI features. Windows only. The C: drive is nearly full.
- `ARCHITECTURE.md` is the binding contract; a design says which sections it changes.
- Project files (`.wtep`, ARCHITECTURE 7): new fields need a format version rule and must keep old projects loading.
- Undo / redo, save / reopen, export (PNG, per-layer PNG, ORA, PSD) must keep working; text stays its own re-editable
  layer.
- User decision: 3D (projected) and 2D (UV) stamps stay separate kinds, not cross-editable.
- User decision: the colour picker stays visible in the side panel at 1280x800 whatever tool is active.
- Round 3 is being implemented right now and touches the same areas: stamp bake performance (`stamp_bake.rs`,
  `texel_map.rs`, `doc_stamps.rs`), the side panel layout (`panels\*`), text layer naming, project load. Plan:
  `E:\claude_programs\wrapped_texture_editor\target\round3_audit\ROUND3_PLAN.md`. The round-4 build starts from the
  round-3 result, so designs must not depend on code that round 3 rewrites; name the overlap.
- Everything must be testable with the headless self-test runner (ARCHITECTURE 8.11) and unit tests; no OS-level
  input injection.
- The implementation will be done by parallel packages with disjoint file ownership and an integration gate, as in
  rounds 2 and 3.

## What a design document contains

1. What the user gets: each control and what it does, in the user's terms, with the default values.
2. Readings of unclear words, the one chosen, the alternatives, and the question to put to the user.
3. What exists today, with `file:line` references.
4. Data model and file format changes, versioning, undo.
5. Algorithms, with cost estimates against the budgets above.
6. User interface: where the controls live, how it fits 1280x800, what the cursor / preview shows.
7. Work breakdown: files touched, packages with disjoint ownership, order.
8. Acceptance tests: unit tests, self-test scenarios, performance numbers.
9. Risks, and what is deliberately left out.
