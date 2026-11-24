# objects-tabs WP3a / WP3b — builder notes

Branch `objects-tabs`, worktree `G:\software_projects\texture_wrapper_ap_ui`. Plan sections 4 (WP3a) and 5 (WP3b).

## WP3a — app state, actions, object_ops, menu, shortcuts, persistence

What is in place:

- `Session::{selected_objects, object_anchor, object_selection_rev, object_reveal}` (state.rs); `Session::validate` drops
  selected objects beyond `object_count` (bumping the revision) and a stale anchor / reveal.
- `AppAction::{SelectObject, ToggleObjectSelected, SelectObjectRange, SelectAllObjects, DeselectObjects, SetObjectVisible,
  HideSelectedObjects, IsolateSelectedObjects, ShowAllObjects, FrameSelection}`.
- New `crates/wte_app/src/object_ops.rs` with the plan's functions plus `selection_bounds` and `restored_hidden_text`.
  `set_hidden` logs `objects: N of M hidden (mask in X ms)` on a change.
- dispatch: the object actions; `FrameModel` and `ResetCamera` frame `doc.visible_bounds()`.
- Shortcuts (table 36 -> 40): Shift+F "Frame Selection" directly before F; Shift+H "Isolate Selected", Alt+H "Show All",
  H "Hide Selected" (a new "objects" block after "Frame Model"). All `with_doc`, `text_ok` false.
- Menu "Object" between Layer and Help (Hide Selected, Isolate Selected, Show All, Frame Selection, Select All, Deselect
  All); File > Open Sample > "Three Objects". The "Objects Panel" entry is WP4's.
- `UiState::hidden_objects: Vec<String>` (io_flows.rs): captured at save, restored in `install_document` with one Info
  toast.

Decisions where the plan was open or in tension (each chosen for the plan's stated intent):

1. **The visibility actions finish a held interaction instead of cancelling it.** Plan 4.3 says "a held stroke or gizmo
   drag is ended the usual way first", but its test (4.7) wants "HideSelectedObjects during a held stroke ends the stroke
   first and leaves one undo entry (the stroke)". The usual way for a guarded action is `cancel`, which discards a held
   stroke (`stroke_cancel`, no undo entry). So `SetObjectVisible`, `HideSelectedObjects`, `IsolateSelectedObjects` and
   `ShowAllObjects` are in `finishes_interaction` (like Save): hiding is view state and should never throw paint away.
2. **`UiState::hidden_objects` is not written when nothing is hidden** (`skip_serializing_if = "Vec::is_empty"`), so a
   project without hidden objects has the same `ui_state` as before. It is read leniently (`lenient_names`): any value
   that is not an array counts as nothing hidden *and the rest of the view state still loads* (the plan's "non-array
   value -> ignored with the warning path": the warning is logged, but I did not let one bad key drop the camera,
   colours and view mode); non-string entries are skipped.
3. **The Info toast on load** names the hidden objects when there are at most three ("1 object is hidden: Cube (Object >
   Show All)", "2 objects are hidden: Floor, Ball (Object > Show All)"), else "N objects are hidden (Object > Show All)".
   (Plan 4.3 gives the N form; scenario 8.1 step 11 wants the toast to name the Cube.)
4. `SelectObjectRange` replaces the selection with the range (like the Layers list's Shift-click), the anchor stays.
   `SelectAllObjects` keeps the anchor; `DeselectObjects` clears it.
5. The dispatch test with a held stroke needs `ToolController` and `dispatch`, i.e. a `WteApp`, which needs a GPU: it is
   `control_sweep::gpu_tests::hiding_during_a_held_stroke_commits_the_stroke_first` (ignored, run with `--ignored`; passes
   on this machine).
6. `SelectObjectRange` and `SetObjectVisible` carry `#[allow(dead_code)]` until the Objects tab (WP5) queues them.

Tests after WP3a (`cargo-gnu.cmd test --workspace --no-fail-fast`): app 391 / 0 / 20, core lib 821 / 1 (WA20, the known
red) / 24, brush_tips 15, end_to_end 9, objects 8, project_roundtrip 24 -> **1,268 passed, 1 failed (WA20), 44 ignored**.
Clippy `--workspace --all-targets`: 0 warnings. GPU: `hiding_during_a_held_stroke_commits_the_stroke_first` ok.
