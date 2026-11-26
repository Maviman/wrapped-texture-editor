# Build note: P6a - Sticker view and canvas (STICKER_PLAN 7 "P6a", 3.1-3.3, 3.5, 3.10, 8.2)

Builder, 2026-11-26, branch `stickers` (from 9edab3c, the P5c check; plan revision 3). Kind U: the first package the
user can see. The Sticker view opens on a selected stamp (key 4, the mode strip, View > Sticker, Layer > Edit Sticker,
the Tool page's "Edit Sticker", the row menu or a double-click on the thumbnail of a stamp layer, Enter): the sticker
canvas on the left, the live 3D view on the right; the sticker's layers in the Layers tab; Move and Hand work, every
other sticker tool is in the strip with its key and says "not available yet" (P7a, P7b, P8 build them).

## What was built

| Item (plan P6a) | Where | Notes |
|---|---|---|
| `ViewMode::Sticker`, `StickerTool`, `StickerTarget`, `Canvas2D`, `StickerEdit`, `Session::sticker_edit`, `AppState::{sticker_canvas_rect, sticker_ui, remembered_view_mode}`, `Prefs::sticker_split`, `PanelHit` additions | `state.rs` | additive; `ViewMode::Sticker` is never written to `ui_state` (`remembered_view_mode`) and not restored from it; prefs hold no view mode at all; `sticker_split` read leniently (0.55, clamped to 0.15..=0.85) |
| `AppAction::SetStickerTool`, `AppAction::Sticker(StickerAction)` | `action.rs`, new `sticker/mod.rs` | 25 sticker commands; `run` (needs the app: file dialog, Esc) / `run_in` (state only, unit-tested) |
| Entering, leaving, Revert (3.1, review B12) | `sticker/mod.rs` | `enter` / `leave` / `validate`; return mode kept; opening changes nothing in the document; Revert = `restore_sticker_state(entry, "Revert Sticker")`, enabled by `differs_from_entry`; the view closes when its stamp is gone |
| What the canvas shows (3.1) | `sticker/mod.rs` `Shown` | sticker stamp: the sticker; image stamp: a virtual one-layer sticker (layer id 1), promoted inside the first content edit (`StickerRef::OfStamp`); text stamp: its picture with its styles, read-only, "Convert to layered sticker" |
| Structure edits (3.5) | `sticker/mod.rs` | add picture layer (blob centred / empty of artboard size), duplicate, delete (a sticker keeps one layer; locked stays), reorder (Up / Down / drop), show / hide, lock, opacity (Preview folds), blend, rename (layer and sticker), position (Move tool, X / Y), nudge; each one `set_sticker` = one entry |
| Frame and canvas (3.1, 3.2) | new `sticker/canvas.rs`, `view/mod.rs` | `[canvas | divider | 3D]` with `prefs.sticker_split`; header (name, Done, Revert, Convert, zoom, Fit, 100 %; narrow canvas drops the right-hand items); checkerboard, picture texture keyed by `CanvasKey` (by value: `LayerKey` per layer, no `Arc` kept), artboard outline, pixel grid from 800 %, active layer outline |
| Move (move only) and Hand / zoom (3.3) | `sticker/canvas.rs` `handle_input` | Move: Auto-select (`Document::sticker_layer_at`), Preview positions in whole pixels, Final at the release (one entry; a click makes none); Hand, Space, Secondary and Middle drags pan; the wheel zooms about the pointer; locked layer / text stamp / unbuilt tool explained in the status line |
| 3D half navigates only | `tools/mod.rs` `active_tool` | Navigate in the Sticker view for the `ToolController`, the hover cursor and the selected outline; `state.tool` untouched, so the paint tool comes back on leaving |
| Layers tab and Tool tab in the view (3.2) | new `sticker/layers_ui.rs`, `panels/side_groups.rs`, `panels/tool_options.rs` | rows (eye, thumbnail via `Document::sticker_layer_thumbnail`, name, kind, "clipped", opacity, blend, lock), click selects, drag reorders, Name / Opacity / Blend of the active layer, footer New / Add picture... ("(on the PC)" with a tablet) / Duplicate / Up / Down / Delete; Tool tab: "Sticker <tool> options" (Move: Auto-select layer) and the active layer's X / Y |
| Tool strip | `panels/toolbar.rs`, `theme.rs` `sticker_tool_icon` | the 14 sticker tools (sizes shrink to fit 600 pt), rects kept through the side panel's clear |
| Ways in | `panels/{menu_bar,layers,tool_options}.rs`, `view/mod.rs`, `shortcuts.rs` | listed above |
| `STICKER_SHORTCUTS` and the routing rule (3.3, reviews A12 / B6) | `shortcuts.rs` | 31 entries (tools, F fit, Delete / Backspace, Esc, Enter, Ctrl+J / M / ] / [, arrows and Shift+arrows); `sticker_order()` = sticker entries + global entries of unbound chords (S, R blocked), stably sorted most-specific-first; global table 42 (4 = Sticker, Enter = Edit Sticker) |
| Menu commands in the view | `dispatch.rs` `in_sticker_view` | Edit > Delete and Layer > Duplicate / Merge Down / Move act on the sticker (never a texture layer or the stamp) |
| Pen eraser end (3.3, 3.10) | `pen/mod.rs` `sticker_eraser_override`, `app.rs` | the same rule generic over the tool set; own memory `PenState::sticker_eraser_restore`; dispatched as `SetStickerTool` |
| Companion fingers (3.10, review A16) | `companion/mod.rs` `finger_views` | the mapper's view list = `view_rects` + `sticker_canvas_rect` |
| Self-test (8.2) | `selftest/mod.rs` | `Target { View(ViewId), StickerCanvas }` for Pointer / Click / Scroll / Hover / Pen; `Step::WaitMs(n)` (wall clock, `Active::WaitUntil`); `Step::Shot { name, expect }` and `"shots"` in `OUT.json` |
| Scenario `sticker_view` | new `selftest/scenarios/sticker.rs` | as plan 7 "P6a" (details in ARCHITECTURE 8.11) |
| `frame_profile` spans | `sticker/canvas.rs` | `sticker.canvas`, `sticker.render` |
| Core read-outs | `doc_stickers.rs`, `sticker/render.rs` | `Document::sticker_layer_at`, `Document::sticker_layer_thumbnail`, `render::layer_alpha_at`; read only, no history; test `k6a_layer_at_and_layer_thumbnail` |
| Docs | `ARCHITECTURE.md` 1.12 (completed: decisions S4-S10, traceability of every package), 2.1, 2.2, 5.20, 8.2, 8.3, 8.5, 8.7, 8.10 (rule 7, tables), 8.11, 8.13 (pitfall 23), new 8.15, 9.1; `HANDOFF.md` 4.1 (55 scenarios) | |

Tests added: core 1 (`k6a_*`); app 22: `sticker::tests` (13: enter / leave keep the mode and change nothing, the
split preference, promotion inside the add and one undo, every layer command one entry with its rules, Delete never
deletes the stamp, Revert, the view closing with its stamp, text stamp read-only and converted, Move drag one entry,
pans / zoom only the camera, locked layer and unbuilt tool, Auto-select, the guard list), `shortcuts` (2: the sticker
table and its routed order; every colliding chord V W H F X B E G I S R Delete Backspace Esc Enter Ctrl+J Ctrl+M
Ctrl+] Ctrl+[ M Shift+E Shift+G arrows Shift+F Shift+X T 1 4 Ctrl+Z ] in both modes), `pen` (1), `companion::input`
(1: one finger over the canvas pans, two zoom, Mouse mode gives Primary and starts a Move), `io_flows` (1),
`dispatch` (1), `selftest` (2: `Target`, `WaitMs` + shots), `toolbar` (1: every sticker tool has a key). The scenario
count test now also fails a `sticker*` scenario whose shot has no `expect`.

## Deviations from the plan, with reasons

1. **Existing scenarios needed a one-word edit.** The plan says `Target` with `From<ViewId>` keeps every scenario
   compiling unchanged; struct-literal fields do not convert, so the 25 `view: ViewId::..` literals got `.into()`
   (mechanical, no behaviour change) and the two helpers (`drag`, `click`) convert. Likewise `Step::Shot(name)`
   became `Step::Shot { name, expect }` as the plan asks; the 40 old shots pass `expect: ""` (only `sticker*`
   scenarios must give one).
2. **The canvas rect is not put into `state.view_rects`** (it is keyed by `ViewId`, and review A17 forbids a new
   `ViewId`). It is `AppState::sticker_canvas_rect`; the companion's mapper gets it through `finger_views`, which
   gives it exactly the list the plan meant.
3. **Tools not built yet are in the strip and their keys work** (the plan's key tests need `SetStickerTool(Brush)`
   etc. now); their icons are dimmed, the tooltip and the status line say "not available yet". Ctrl+M (Merge Down of
   sticker layers, a P8 command) is routed and says the same; it never touches a texture layer.
4. **No row context menu and no multi-select in the sticker Layers list yet.** Its commands (Merge Down, Rasterize,
   Clip to Layer Below, Layer from file, Copy / Paste style) arrive in P6b / P8; everything P6a has is on the footer
   buttons and icons (tablet rule). The Text and Shape buttons come with P8.
5. **E6 (the shared-picture question) is P6b's** (its file list has `Dialog::StickerShared`). Until then a content
   edit of an image stamp promotes it with `ShareScope::ThisStamp`, and an edit of a sticker shown by several stamps
   reaches all of them (the core's default).
6. **The canvas texture is re-uploaded whole when its key changes**; `TextureHandle::set_partial` for dirty rectangles
   belongs to painting (P7a), the first thing that changes pixels in small rectangles.
7. **Core additions in a U package**: `sticker_layer_at` / `sticker_layer_thumbnail` / `layer_alpha_at` (read-only).
   The Move tool's "click selects the top layer under the pointer" and the Layers tab's thumbnails need pixels the app
   cannot reach (`sticker_render_input` is crate-private). No existing behaviour changed (identity sweep below).
8. **Additions the plan implies but does not list**: the arrow-key nudge of the Move row (8 table entries); the Edit /
   Layer menu commands mapped to the sticker in the view (so the menus obey "never a texture layer" like the keys);
   Backspace beside Delete; Enter as the global "Edit Sticker" key (plan 3.1 lists "Enter with a stamp selected").
9. **`prefs.rs` is unchanged**: `sticker_split` is read leniently in `state.rs`'s `lenient_prefs_serde` like the other
   fields.
10. **`view/view3d.rs` (CRLF in the index) is not edited**: the Navigate-only 3D half goes through
    `tools::active_tool`. Its `highlight_shown` still reads `state.tool`, so the object selection tint appears in the
    Sticker view only with the Navigate tool or the Objects tab, as elsewhere.
11. **No `dist/HOW_TO_TEST.txt` edit**: plan 7 / 10 give P6a none; P6b writes the first part of the "Stickers" chapter.
12. **Shift+E / Shift+G outside the Sticker view** still select the paint Eraser / Fill (the plan's table says
    "unbound"; egui's logical matching lets E / G take them, as before this package). Unchanged and documented in the
    test.

## Numbers measured

- `cargo test --workspace --no-fail-fast`: **1,669 passed, 1 failed (WA20 only:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`), 62 ignored** (`wte_core` lib 1,066 / 1 /
  38, `wte_app` 546 / 0 / 24, integration 15 + 9 + 8 + 25). Against the P5c check (1,646 / 1 / 62): +1 core, +22 app.
  The app tests were run again on the final source (546 / 0 / 24).
- `cargo clippy --workspace --all-targets`: 0 warnings (final source).
- `cargo build --release -p wte_app`: OK, exe 29,671,424 bytes, SHA-256
  `ff9dc2e417c7d82d0d3f23295340ea467c8103382dd911ce25258aa64d35fe94`.
- Scenario `sticker_view` (that exe, 1280x800, off-screen, no focus taken): `"ok": true`, 5 shots with their `expect`
  in `target\sticker\p6a\final4\sticker_view.json`. Looked at every shot against its expectation: `sticker_enter`
  (logo on the checkerboard, Done, greyed Revert, zoom 184 %, Fit, 100 %; the sphere with the logo; Move highlighted;
  one picture row) matches; `sticker_moved` (the red disc below right with its outline, two rows, disc active, Revert
  enabled) matches; `sticker_reordered` (the half-transparent disc behind the logo, Logo above disc, "50 %") matches;
  `sticker_small` (900x600: header Done / Revert / Fit / 100 %, sphere right, all 14 tools and the colour chip)
  matches; `sticker_left` (3D view, the faded disc behind the logo inside the Transform gizmo, Transform options with
  Edit Sticker, Layers Logo / Paint 1 / Base) matches. A first run showed the header overlapping at 900x600 (fixed:
  the narrow canvas drops its right-hand items) and the editor's status line surviving the leave (fixed: cleared).
- Frame spans (`WTE_FRAME_LOG=1`, `sticker_view`, 194 frames): worst `sticker.canvas` 1.68 ms, worst `sticker.render`
  1.57 ms (nested inside it); limit 50 ms (8.4).
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with the exe of the code before the last sticker-only edits and
  `tools/sticker/jobs_identity.txt` into `target\sticker\p6a\sweep\`, off-screen, no focus taken): **21 / 21
  `"ok": true`**; `compare_baseline.py` against the P0 baseline: 214 of 216 files the same. The two others are
  `side_tabs/shot_small_640.png` and `shot_small_900.png` (compared whole because they are not 1280x800): every
  differing pixel lies in rows 25..50, the mode strip with its new "Sticker" tab (0 pixels differ elsewhere, checked
  with numpy); plan 8.5 excludes the mode strip. Exports, saved layer PNGs and manifests: all the same
  (`target\sticker\p6a\compare.txt`).
- Programs started: cargo, Python and the WTE release exe (only with `--screenshot ... --scenario ...`, off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new crate; `Cargo.toml` /
  `Cargo.lock` unchanged. New files LF; every edited file keeps its line endings.
