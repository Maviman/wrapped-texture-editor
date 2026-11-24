// Owner: objects-tabs (WP3a). The object commands of ARCHITECTURE.md 8.3 as plain functions on `Session` /
// `AppState` (unit-testable without a GPU): selection (app state, never in the document), visibility (view state of
// the document: `Document::set_hidden_objects`, no history entry, no revision change, modified flag untouched),
// framing, and the hidden object names stored in `UiState`.

use std::collections::BTreeSet;

use wte_core::Document;

use crate::state::{AppState, Session, ViewMode};

/// Replaces the selection, bumping `object_selection_rev` when it changed.
fn set_selection(session: &mut Session, selection: BTreeSet<u32>) {
    if session.selected_objects != selection {
        session.selected_objects = selection;
        session.object_selection_rev += 1;
    }
}

fn valid(session: &Session, object: u32) -> bool {
    object < session.doc.mesh.object_count()
}

/// Plain click: this object only; it becomes the anchor of a later Shift-click.
pub(crate) fn select(session: &mut Session, object: u32) {
    if !valid(session, object) {
        return;
    }
    set_selection(session, BTreeSet::from([object]));
    session.object_anchor = Some(object);
}

/// Ctrl-click: adds the object to the selection or takes it out; it becomes the anchor.
pub(crate) fn toggle(session: &mut Session, object: u32) {
    if !valid(session, object) {
        return;
    }
    let mut selection = session.selected_objects.clone();
    if !selection.remove(&object) {
        selection.insert(object);
    }
    set_selection(session, selection);
    session.object_anchor = Some(object);
}

/// Shift-click: the range from the anchor to `to` (both included) becomes the selection; the anchor stays. Without
/// an anchor it is a plain select.
pub(crate) fn select_range(session: &mut Session, to: u32) {
    if !valid(session, to) {
        return;
    }
    let Some(anchor) = session.object_anchor.filter(|&a| valid(session, a)) else {
        select(session, to);
        return;
    };
    set_selection(session, (anchor.min(to)..=anchor.max(to)).collect());
}

/// Every object of the mesh.
pub(crate) fn select_all(session: &mut Session) {
    let all = (0..session.doc.mesh.object_count()).collect();
    set_selection(session, all);
}

/// Nothing selected, no anchor.
pub(crate) fn deselect(session: &mut Session) {
    set_selection(session, BTreeSet::new());
    session.object_anchor = None;
}

/// Applies new hidden flags through `Document::set_hidden_objects` (view state); an error (an open stroke) becomes a
/// warning toast. No-op without a document.
pub(crate) fn set_hidden(state: &mut AppState, hidden: Vec<bool>) {
    let Some(session) = state.session.as_mut() else { return };
    let start = std::time::Instant::now();
    match session.doc.set_hidden_objects(&hidden) {
        Ok(true) => log::info!(
            "objects: {} of {} hidden (mask in {:.1} ms)",
            session.doc.hidden_object_count(),
            session.doc.mesh.object_count(),
            start.elapsed().as_secs_f64() * 1000.0
        ),
        Ok(false) => {}
        Err(e) => state.toasts.warn(crate::tools::error_text(&e)),
    }
}

/// The eye of one row: shows or hides one object; the selection is not touched.
pub(crate) fn set_visible(state: &mut AppState, object: u32, visible: bool) {
    let Some(doc) = state.doc() else { return };
    if object >= doc.mesh.object_count() {
        return;
    }
    let mut hidden = doc.hidden_objects().to_vec();
    hidden[object as usize] = !visible;
    set_hidden(state, hidden);
}

/// Hides the selected objects (they stay selected, so the eye or Show All brings them back in place). No-op with an
/// empty selection.
pub(crate) fn hide_selected(state: &mut AppState) {
    let Some(session) = state.session.as_ref() else { return };
    if session.selected_objects.is_empty() {
        return;
    }
    let mut hidden = session.doc.hidden_objects().to_vec();
    for &o in &session.selected_objects {
        if let Some(h) = hidden.get_mut(o as usize) {
            *h = true;
        }
    }
    set_hidden(state, hidden);
}

/// Shows the selected objects and hides every other one. No-op with an empty selection.
pub(crate) fn isolate_selected(state: &mut AppState) {
    let Some(session) = state.session.as_ref() else { return };
    if session.selected_objects.is_empty() {
        return;
    }
    let hidden = (0..session.doc.mesh.object_count()).map(|o| !session.selected_objects.contains(&o)).collect();
    set_hidden(state, hidden);
}

/// Shows every object.
pub(crate) fn show_all(state: &mut AppState) {
    let Some(doc) = state.doc() else { return };
    let hidden = vec![false; doc.hidden_objects().len()];
    set_hidden(state, hidden);
}

/// The box Frame Selection frames: the selected visible objects, else the visible model.
pub(crate) fn selection_bounds(session: &Session) -> wte_core::Aabb {
    let selected: Vec<u32> = session.selected_objects.iter().copied().collect();
    session.doc.objects_bounds(&selected).unwrap_or_else(|| session.doc.visible_bounds())
}

/// Frames the selected visible objects (the visible model when none is selected or visible): the editing camera in
/// 3D and Split, the preview camera in Preview, nothing in 2D.
pub(crate) fn frame_selection(state: &mut AppState) {
    let mode = state.view_mode;
    let Some(session) = state.session.as_mut() else { return };
    let bounds = selection_bounds(session);
    match mode {
        ViewMode::View3D | ViewMode::Split => session.camera.frame(bounds),
        ViewMode::Preview => session.preview_camera.frame(bounds),
        ViewMode::View2D => {}
    }
}

/// The names of the hidden objects, in object order (for `UiState::hidden_objects`).
pub(crate) fn hidden_names(doc: &Document) -> Vec<String> {
    doc.mesh
        .objects
        .iter()
        .enumerate()
        .filter(|(i, _)| doc.object_hidden(*i as u32))
        .map(|(_, o)| o.name.clone())
        .collect()
}

/// Hidden flags from stored names: an object is hidden when its name is listed; unknown and repeated names are
/// ignored.
pub(crate) fn hidden_from_names(doc: &Document, names: &[String]) -> Vec<bool> {
    let names: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    doc.mesh.objects.iter().map(|o| names.contains(o.name.as_str())).collect()
}

/// The Info toast after opening a project that restores hidden objects; `None` when nothing is hidden.
pub(crate) fn restored_hidden_text(doc: &Document) -> Option<String> {
    let names = hidden_names(doc);
    match names.len() {
        0 => None,
        1 => Some(format!("1 object is hidden: {} (Object > Show All)", names[0])),
        n @ 2..=3 => Some(format!("{n} objects are hidden: {} (Object > Show All)", names.join(", "))),
        n => Some(format!("{n} objects are hidden (Object > Show All)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::state_with;
    use wte_core::SampleKind;

    /// The Three Objects sample: Floor 0, Cube 1, Ball 2.
    fn trio() -> AppState {
        let state = state_with(SampleKind::Objects);
        let names: Vec<&str> = state.doc().unwrap().mesh.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["Floor", "Cube", "Ball"]);
        state
    }

    fn session(state: &mut AppState) -> &mut Session {
        state.session.as_mut().unwrap()
    }

    fn selected(state: &AppState) -> Vec<u32> {
        state.session.as_ref().unwrap().selected_objects.iter().copied().collect()
    }

    /// (revision, modified, undo depth) of the document.
    fn doc_marks(state: &AppState) -> (u64, bool, usize) {
        let doc = state.doc().unwrap();
        (doc.revision(), doc.is_modified(), doc.history.undo_len())
    }

    #[test]
    fn select_toggle_and_range_follow_the_anchor_rules() {
        let mut state = trio();
        let marks = doc_marks(&state);
        let s = session(&mut state);
        // A range without an anchor is a plain select.
        select_range(s, 1);
        assert_eq!(s.selected_objects, BTreeSet::from([1]));
        assert_eq!(s.object_anchor, Some(1));
        let rev = s.object_selection_rev;
        assert!(rev > 0);
        // Plain sets the anchor, Ctrl sets it, Shift keeps it.
        select(s, 0);
        assert_eq!(s.object_anchor, Some(0));
        select_range(s, 2);
        assert_eq!(s.selected_objects, BTreeSet::from([0, 1, 2]));
        assert_eq!(s.object_anchor, Some(0), "Shift keeps the anchor");
        toggle(s, 1);
        assert_eq!(s.selected_objects, BTreeSet::from([0, 2]));
        assert_eq!(s.object_anchor, Some(1), "Ctrl sets the anchor");
        toggle(s, 1);
        assert_eq!(s.selected_objects, BTreeSet::from([0, 1, 2]));
        select_range(s, 0);
        assert_eq!(s.selected_objects, BTreeSet::from([0, 1]), "the range runs from the anchor (1) to the row (0)");
        // Unknown objects are ignored; an unchanged selection does not bump the revision.
        let before = (s.selected_objects.clone(), s.object_selection_rev);
        select(s, 9);
        toggle(s, 9);
        select_range(s, 9);
        select(s, 0);
        select(s, 0);
        assert_eq!(s.selected_objects, BTreeSet::from([0]));
        assert_eq!(s.object_selection_rev, before.1 + 1, "one change, one revision");
        select_all(s);
        assert_eq!(s.selected_objects, BTreeSet::from([0, 1, 2]));
        deselect(s);
        assert!(s.selected_objects.is_empty() && s.object_anchor.is_none());
        assert_eq!(doc_marks(&state), marks, "selecting never touches the document");
    }

    #[test]
    fn hide_isolate_and_show_all_are_view_state_and_keep_the_selection() {
        let mut state = trio();
        let marks = doc_marks(&state);
        // No selection: Hide and Isolate do nothing.
        hide_selected(&mut state);
        isolate_selected(&mut state);
        assert_eq!(state.doc().unwrap().hidden_object_count(), 0);
        assert_eq!(state.doc().unwrap().visibility_generation(), 0);

        select(session(&mut state), 1);
        hide_selected(&mut state);
        assert_eq!(state.doc().unwrap().hidden_objects(), [false, true, false]);
        assert_eq!(selected(&state), [1], "the hidden row stays selected");

        select(session(&mut state), 2);
        isolate_selected(&mut state);
        assert_eq!(state.doc().unwrap().hidden_objects(), [true, true, false], "only the Ball is shown");

        set_visible(&mut state, 0, true);
        assert_eq!(state.doc().unwrap().hidden_objects(), [false, true, false]);
        assert_eq!(selected(&state), [2], "the eye does not change the selection");
        set_visible(&mut state, 7, false);

        show_all(&mut state);
        assert_eq!(state.doc().unwrap().hidden_object_count(), 0);
        let generation = state.doc().unwrap().visibility_generation();
        assert_eq!(generation, 4);
        show_all(&mut state);
        assert_eq!(state.doc().unwrap().visibility_generation(), generation, "nothing changed");
        assert_eq!(doc_marks(&state), marks, "hiding never touches history, revision or the modified flag");
        assert!(state.toasts.visible_labels().is_empty(), "no toast on a visibility change");

        // Without a document nothing happens.
        let mut empty = AppState::new(crate::state::Prefs::default(), true);
        hide_selected(&mut empty);
        show_all(&mut empty);
        set_visible(&mut empty, 0, false);
        frame_selection(&mut empty);
    }

    #[test]
    fn a_hide_during_an_open_stroke_is_refused_with_a_warning() {
        let mut state = trio();
        select(session(&mut state), 1);
        state.doc_mut().unwrap().stroke_begin(&wte_core::BrushParams::default()).unwrap();
        hide_selected(&mut state);
        assert_eq!(state.doc().unwrap().hidden_object_count(), 0);
        assert_eq!(state.toasts.visible_labels().len(), 1, "{:?}", state.toasts.visible_labels());
        state.doc_mut().unwrap().stroke_cancel();
    }

    #[test]
    fn frame_selection_targets_the_visible_selected_objects() {
        let mut state = trio();
        let doc = state.doc().unwrap();
        let (floor, cube, ball) = (doc.mesh.objects[0].bounds, doc.mesh.objects[1].bounds, doc.mesh.objects[2].bounds);
        let whole = doc.mesh.bounds;
        let framed = |state: &AppState| state.session.as_ref().unwrap().camera.target;
        let expect = |b: wte_core::Aabb| {
            let mut c = crate::state::OrbitCamera::default();
            c.frame(b);
            (c.target, c.distance)
        };

        // Nothing selected: the visible model.
        frame_selection(&mut state);
        assert!((framed(&state) - whole.center()).length() < 1e-5);

        select(session(&mut state), 1);
        toggle(session(&mut state), 2);
        frame_selection(&mut state);
        let s = state.session.as_ref().unwrap();
        let (target, distance) = expect(cube.union(&ball));
        assert!((s.camera.target - target).length() < 1e-5 && (s.camera.distance - distance).abs() < 1e-4);

        // A hidden selected object is not framed; the preview camera and 2D follow the mode.
        set_visible(&mut state, 2, false);
        frame_selection(&mut state);
        assert!((framed(&state) - cube.center()).length() < 1e-5);
        state.view_mode = ViewMode::Preview;
        frame_selection(&mut state);
        assert!((state.session.as_ref().unwrap().preview_camera.target - cube.center()).length() < 1e-5);
        state.view_mode = ViewMode::View2D;
        select(session(&mut state), 0);
        frame_selection(&mut state);
        assert!((framed(&state) - cube.center()).length() < 1e-5, "2D frames nothing");

        // Everything selected is hidden: the visible model (here: Floor and Cube).
        state.view_mode = ViewMode::View3D;
        select(session(&mut state), 2);
        frame_selection(&mut state);
        assert!((framed(&state) - floor.union(&cube).center()).length() < 1e-5);
    }

    #[test]
    fn hidden_names_round_trip_and_tolerate_unknown_and_repeated_names() {
        let mut state = trio();
        let doc = state.doc().unwrap();
        assert!(hidden_names(doc).is_empty());
        assert_eq!(restored_hidden_text(doc), None);
        state.doc_mut().unwrap().set_hidden_objects(&[true, false, true]).unwrap();
        let doc = state.doc().unwrap();
        let names = hidden_names(doc);
        assert_eq!(names, ["Floor", "Ball"]);
        assert_eq!(hidden_from_names(doc, &names), [true, false, true]);
        let odd = ["Ball".to_owned(), "Nope".to_owned(), "Ball".to_owned(), String::new()];
        assert_eq!(hidden_from_names(doc, &odd), [false, false, true]);
        assert_eq!(hidden_from_names(doc, &[]), [false, false, false]);
        assert_eq!(restored_hidden_text(doc).unwrap(), "2 objects are hidden: Floor, Ball (Object > Show All)");
        state.doc_mut().unwrap().set_hidden_objects(&[false, true]).unwrap();
        assert_eq!(restored_hidden_text(state.doc().unwrap()).unwrap(), "1 object is hidden: Cube (Object > Show All)");
    }

    #[test]
    fn validate_drops_objects_the_mesh_does_not_have() {
        let mut state = trio();
        let s = session(&mut state);
        s.selected_objects = BTreeSet::from([1, 5]);
        s.object_anchor = Some(5);
        s.object_reveal = Some(9);
        let rev = s.object_selection_rev;
        s.validate();
        assert_eq!(s.selected_objects, BTreeSet::from([1]));
        assert_eq!((s.object_anchor, s.object_reveal), (None, None));
        assert_eq!(s.object_selection_rev, rev + 1);
        s.validate();
        assert_eq!(s.object_selection_rev, rev + 1, "nothing stale: no change");
    }
}
