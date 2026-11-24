// Owner: T6 text panels (round 4). Written by the scaffold S2 (the two egui proofs), filled by T6.
//
// Headless panel tests of the text feature (FEATURE_TEXT 8.2): the real side panel is driven with pointer and key
// events through `PanelApp` (a minimal dispatch applies the document actions), and the queued actions, the document and
// the recorded rectangles are checked.

use eframe::egui;
use egui::accesskit::Role;
use egui::text::{CCursor, CCursorRange};
use egui::{Event, Key, Modifiers, Pos2, pos2, vec2};
use glam::{Vec2 as GVec2, Vec3};
use wte_core::{
    Commit, FontCatalog, Gap, PathShape, Placement, Projector, SampleKind, StampId, StampSource, TextSpec, TextureSetId,
    Warp, WarpStyle,
};

use super::{Drive, Harness, Node, PanelApp, sample_state};
use crate::action::AppAction;
use crate::panels::text_block::{self, BufferCall, GapBuffer, content_field_id};
use crate::panels::text_tab::CIRCLE_TOO_SHORT;
use crate::panels::tool_options::placement_rotation_deg;
use crate::state::{AppState, PanelHit, SideTab, TextGapSel, TextPage, ToolKind, UpperTab};

/// Puts the cursor (or a selection `a..b`) into the field `id` the way a click or Shift+arrows would.
fn set_cursor(ctx: &egui::Context, id: egui::Id, a: usize, b: usize) {
    let mut st = egui::text_edit::TextEditState::load(ctx, id).expect("the field has a state");
    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    st.store(ctx, id);
}

/// A frame of one multiline field showing `buf` with the keyboard focus; returns the reported cursor range.
fn field_frame(h: &mut Harness, id: egui::Id, buf: &mut GapBuffer, events: Vec<Event>, focus: bool) -> Option<std::ops::Range<usize>> {
    let mut range = None;
    h.pass(events, |ui| {
        let out = egui::TextEdit::multiline(buf).id(id).desired_rows(2).show(ui);
        if focus {
            out.response.request_focus();
        }
        range = out.cursor_range.map(|r| {
            let s = r.as_sorted_char_range();
            s.start.0..s.end.0
        });
    });
    range
}

/// The calls that changed something: egui deletes the (empty) selection before every insert, which is a no-op.
fn edits(buf: &mut GapBuffer) -> Vec<BufferCall> {
    buf.calls.drain(..).filter(|c| !matches!(c, BufferCall::Delete { range } if range.is_empty())).collect()
}

/// Lets egui's undoer take a save point (it saves a state that stayed unchanged for one second).
fn settle(h: &mut Harness, id: egui::Id, buf: &mut GapBuffer) {
    h.time += 2.0;
    field_frame(h, id, buf, Vec::new(), true);
    field_frame(h, id, buf, Vec::new(), true);
}

fn key(k: Key, mods: Modifiers) -> Vec<Event> {
    vec![Event::ModifiersChanged(mods), Harness::key(k, mods), Harness::key_up(k, mods), Event::ModifiersChanged(Modifiers::NONE)]
}

/// TEXT 6.2 rule 6, first half: typing, Backspace, Delete and a replaced selection each arrive as `insert_text` /
/// `delete_char_range` with the exact character position; Ctrl+Z and Ctrl+Y arrive as `replace_with` with a text the
/// buffer held before (so a history of (text, gaps) states can be found by its text).
#[test]
fn the_gap_buffer_sees_every_edit_of_the_field() {
    let mut h = Harness::new(800.0, 600.0);
    let id = egui::Id::unique("gap_buffer_proof");
    let mut buf = GapBuffer::new("WTE 2026", &[]);
    field_frame(&mut h, id, &mut buf, Vec::new(), true);
    field_frame(&mut h, id, &mut buf, Vec::new(), true);
    assert!(buf.calls.is_empty(), "showing the field edits nothing: {:?}", buf.calls);
    let mut seen = vec![buf.text.clone()];

    // Typing at index 3 (between "E" and the space).
    set_cursor(&h.ctx, id, 3, 3);
    settle(&mut h, id, &mut buf);
    field_frame(&mut h, id, &mut buf, vec![Event::Text("X".into())], true);
    assert_eq!(edits(&mut buf), [BufferCall::Insert { text: "X".into(), at: 3 }]);
    assert_eq!(buf.text, "WTEX 2026");
    seen.push(buf.text.clone());
    settle(&mut h, id, &mut buf);

    // Backspace removes the character left of the cursor.
    field_frame(&mut h, id, &mut buf, key(Key::Backspace, Modifiers::NONE), true);
    assert_eq!(edits(&mut buf), [BufferCall::Delete { range: 3..4 }]);
    assert_eq!(buf.text, "WTE 2026");
    seen.push(buf.text.clone());
    settle(&mut h, id, &mut buf);

    // Delete removes the character right of the cursor.
    set_cursor(&h.ctx, id, 0, 0);
    field_frame(&mut h, id, &mut buf, key(Key::Delete, Modifiers::NONE), true);
    assert_eq!(edits(&mut buf), [BufferCall::Delete { range: 0..1 }]);
    assert_eq!(buf.text, "TE 2026");
    seen.push(buf.text.clone());
    settle(&mut h, id, &mut buf);

    // Typing over a selection ("E 2" = 1..4): one delete of the selection, one insert at its start.
    set_cursor(&h.ctx, id, 1, 4);
    field_frame(&mut h, id, &mut buf, vec![Event::Text("Z".into())], true);
    assert_eq!(edits(&mut buf), [BufferCall::Delete { range: 1..4 }, BufferCall::Insert { text: "Z".into(), at: 1 }]);
    assert_eq!(buf.text, "TZ026");
    seen.push(buf.text.clone());
    settle(&mut h, id, &mut buf);

    // Ctrl+Z: one `replace_with` whose text the buffer held before.
    field_frame(&mut h, id, &mut buf, key(Key::Z, Modifiers::COMMAND), true);
    let undo = edits(&mut buf);
    assert!(
        matches!(&undo[..], [BufferCall::Replace { text }] if seen.contains(text) && *text != "TZ026"),
        "undo arrives as replace_with with a text of the history: {undo:?}, history {seen:?}"
    );
    let undone = buf.text.clone();
    settle(&mut h, id, &mut buf);

    // Ctrl+Y: one `replace_with` back to the text before the undo.
    field_frame(&mut h, id, &mut buf, key(Key::Y, Modifiers::COMMAND), true);
    let redo = edits(&mut buf);
    assert!(
        matches!(&redo[..], [BufferCall::Replace { text }] if text == "TZ026"),
        "redo arrives as replace_with with the text before the undo ({undone:?}): {redo:?}"
    );
}

/// TEXT 6.2 rule 6, second half: `TextEdit::show` reports the cursor in every frame while the field has the focus (also
/// in frames without input), it follows the arrow keys, and it reports nothing once the focus is gone.
#[test]
fn the_field_reports_its_cursor_while_focused() {
    let mut h = Harness::new(800.0, 600.0);
    let id = egui::Id::unique("cursor_proof");
    let mut buf = GapBuffer::new("WTE 2026", &[]);
    field_frame(&mut h, id, &mut buf, Vec::new(), true);
    field_frame(&mut h, id, &mut buf, Vec::new(), true);
    set_cursor(&h.ctx, id, 2, 2);
    for _ in 0..3 {
        assert_eq!(field_frame(&mut h, id, &mut buf, Vec::new(), true), Some(2..2), "an idle focused frame reports the cursor");
    }
    assert_eq!(field_frame(&mut h, id, &mut buf, key(Key::ArrowLeft, Modifiers::NONE), true), Some(1..1));
    assert_eq!(field_frame(&mut h, id, &mut buf, Vec::new(), true), Some(1..1));
    assert_eq!(field_frame(&mut h, id, &mut buf, key(Key::ArrowRight, Modifiers::SHIFT), true), Some(1..2), "a selection");
    assert!(buf.calls.is_empty(), "moving the cursor edits nothing: {:?}", buf.calls);

    // The focus leaves: no cursor is reported.
    h.ctx.memory_mut(|m| m.surrender_focus(id));
    assert_eq!(field_frame(&mut h, id, &mut buf, Vec::new(), false), None);
    assert_eq!(field_frame(&mut h, id, &mut buf, Vec::new(), false), None);
}

// ---------------------------------------------------------------------------------------------------------------------
// Helpers of the panel tests
// ---------------------------------------------------------------------------------------------------------------------

/// An empty point of the central area: a click there takes the focus away from every field.
const OUTSIDE: Pos2 = pos2(600.0, 500.0);

/// Adds a projected text on the front of the sphere.
fn place_text(state: &mut AppState, content: &str, edit: impl FnOnce(&mut TextSpec)) -> StampId {
    let doc = state.doc_mut().unwrap();
    let d = doc.mesh.bounds.diagonal();
    let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
    let p = Projector::from_surface(hit, Vec3::Z, Vec3::Y, GVec2::new(d * 0.12, d * 0.03), d * 0.1);
    let mut spec = TextSpec::new(content, doc.default_font());
    edit(&mut spec);
    doc.add_stamp(content, StampSource::Text(spec), Placement::Projected(p), TextureSetId(0)).unwrap()
}

/// The sphere with a placed text, selected with the Transform tool; `page` opens the Text tab on that page. Actions are
/// applied.
fn text_app(content: &str, edit: impl FnOnce(&mut TextSpec), page: Option<TextPage>) -> (PanelApp, StampId) {
    let mut state = sample_state(SampleKind::Sphere);
    let id = place_text(&mut state, content, edit);
    state.session.as_mut().unwrap().selected_stamp = Some(id);
    state.tool = ToolKind::Transform;
    if let Some(page) = page {
        state.side_tab = SideTab::Text;
        state.text_page = page;
    }
    let mut app = PanelApp::new(state);
    app.apply = true;
    app.idle(2);
    app.take();
    (app, id)
}

fn spec_of(app: &PanelApp, id: StampId) -> TextSpec {
    match &app.state.doc().unwrap().stamp(id).unwrap().source {
        StampSource::Text(s) => s.clone(),
        StampSource::Image { .. } => panic!("not a text"),
    }
}

fn undo_len(app: &PanelApp) -> usize {
    app.state.doc().unwrap().history.undo_len()
}

/// The specs of the `UpdateStampSource` actions with commit `c`.
fn sources(actions: &[AppAction], c: Commit) -> Vec<TextSpec> {
    actions
        .iter()
        .filter_map(|a| match a {
            AppAction::UpdateStampSource(_, StampSource::Text(s), commit) if *commit == c => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// Asserts `Preview`s followed by exactly one `Final` among the stamp edits of `actions`.
fn assert_previews_then_one_final(actions: &[AppAction], what: &str) {
    let commits: Vec<Commit> = actions
        .iter()
        .filter_map(|a| match a {
            AppAction::UpdateStampSource(_, _, c) | AppAction::UpdateStampPlacement(_, _, c) => Some(*c),
            _ => None,
        })
        .collect();
    assert!(commits.len() >= 2 && commits[..commits.len() - 1].iter().all(|c| *c == Commit::Preview), "{what}: {actions:?}");
    assert_eq!(commits.last(), Some(&Commit::Final), "{what}: {actions:?}");
}

/// Asserts exactly one `Final` stamp edit and nothing else of that kind.
fn assert_one_final(actions: &[AppAction], what: &str) {
    let commits: Vec<Commit> = actions
        .iter()
        .filter_map(|a| match a {
            AppAction::UpdateStampSource(_, _, c) | AppAction::UpdateStampPlacement(_, _, c) => Some(*c),
            _ => None,
        })
        .collect();
    assert_eq!(commits, [Commit::Final], "{what}: {actions:?}");
}

/// A horizontal drag over three frames from the centre of `node`.
fn drag_node(app: &mut PanelApp, node: &Node, dx: f32) {
    let c = node.rect.center();
    app.drag(&[c, c + vec2(dx / 3.0, 0.0), c + vec2(2.0 * dx / 3.0, 0.0), c + vec2(dx, 0.0)]);
}

/// The widget labelled `text` inside the Text tab.
fn in_tab(app: &PanelApp, text: &str) -> Node {
    let tab = app.state.side_rects.text_tab.expect("the Text tab is shown");
    app.h
        .all(text)
        .into_iter()
        .find(|n| tab.expand(1.0).contains(n.rect.center()))
        .unwrap_or_else(|| panic!("no {text:?} in the Text tab: {:?}", app.h.nodes.iter().map(|n| n.text().to_owned()).collect::<Vec<_>>()))
}

/// The widget of `role` in the row of the widget `anchor` (same line, right of it), `nth` from the left.
fn right_of(app: &PanelApp, anchor: &Node, role: Role, nth: usize) -> Node {
    let y = anchor.rect.center().y;
    let mut found: Vec<Node> =
        app.h.nodes.iter().filter(|n| n.role == role && n.rect.min.y <= y && n.rect.max.y >= y && n.rect.min.x > anchor.rect.min.x).cloned().collect();
    found.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
    found.get(nth).cloned().unwrap_or_else(|| panic!("no {role:?} #{nth} right of {anchor:?}"))
}

/// The B button of the Text block.
fn bold_button(app: &PanelApp) -> Node {
    app.h.nodes.iter().find(|n| n.role == Role::Button && n.label == "B").cloned().expect("the B button")
}

/// Text size and Angle fields of row 3 (the drag values right of B).
fn size_and_angle(app: &PanelApp) -> (Node, Node) {
    let b = bold_button(app);
    (right_of(app, &b, Role::SpinButton, 0), right_of(app, &b, Role::SpinButton, 1))
}

/// The slot whose label starts with `prefix` ("Gap between E and (space)").
fn slot(app: &PanelApp, prefix: &str) -> Node {
    app.h.prefix(prefix)
}

/// Clicks into the content field of text `id` and gives it the selection `a..b`.
fn field_select(app: &mut PanelApp, id: StampId, a: usize, b: usize) {
    let field = *app.state.panel_rects.get(&PanelHit::StampTextField).expect("the content field is drawn");
    app.click_at(field.center());
    set_cursor(&app.h.ctx, content_field_id(id), a, b);
    app.idle(1);
}

/// Scrolls the settings area until the row labelled `label` is inside it; returns its widget of `role`.
fn reveal_row(app: &mut PanelApp, label: &str, role: Role) -> Node {
    for _ in 0..60 {
        let area = *app.state.panel_rects.get(&PanelHit::SettingsArea).expect("the settings area");
        let found = app.h.nodes.iter().find(|n| n.role == Role::Label && n.value == label).cloned();
        if let Some(l) = found
            && area.contains_rect(l.rect)
        {
            return app.h.in_row(label, role, 0);
        }
        app.scroll_at(area.center(), vec2(0.0, -1.0));
        app.idle(2);
    }
    panic!("the row {label:?} did not scroll into view");
}

/// Types into a drag value: a click turns it into a text field; select all, type, Enter.
fn type_into(app: &mut PanelApp, node: &Node, text: &str) {
    app.click_at(node.rect.center());
    app.press(Key::A, Modifiers::COMMAND);
    app.type_text(text);
    app.press(Key::Enter, Modifiers::NONE);
    app.idle(1);
}

// ---------------------------------------------------------------------------------------------------------------------
// The content field and its gap values
// ---------------------------------------------------------------------------------------------------------------------

/// TEXT 8.2 `field_undo_brings_the_gap_back`.
#[test]
fn field_undo_brings_the_gap_back() {
    let (mut app, id) = text_app("WTE 2026", |s| s.fx.gaps = vec![Gap { after: 2, em: 0.3 }], None);
    let undo0 = undo_len(&app);
    field_select(&mut app, id, 1, 3);
    app.take();
    app.press(Key::Delete, Modifiers::NONE);
    let previews = sources(&app.take(), Commit::Preview);
    let last = previews.last().expect("a preview after Delete");
    assert_eq!(last.content, "W 2026");
    assert!(last.fx.gaps.is_empty(), "the gap went with its character: {:?}", last.fx.gaps);
    app.press(Key::Z, Modifiers::COMMAND);
    let previews = sources(&app.take(), Commit::Preview);
    let last = previews.last().expect("a preview after Ctrl+Z");
    assert_eq!(last.content, "WTE 2026");
    assert_eq!(last.fx.gaps, [Gap { after: 2, em: 0.3 }], "the field's undo brought the gap back");
    app.click_at(OUTSIDE);
    app.idle(2);
    let finals = sources(&app.take(), Commit::Final);
    assert_eq!(finals.len(), 1, "one Final when the field loses the focus");
    assert_eq!(finals[0].fx.gaps, [Gap { after: 2, em: 0.3 }]);
    assert!(undo_len(&app) <= undo0 + 1, "at most one undo entry");
}

/// TEXT 8.2 `typing_keeps_the_gap_on_its_character`.
#[test]
fn typing_keeps_the_gap_on_its_character() {
    let (mut app, id) = text_app("WTE 2026", |s| s.fx.gaps = vec![Gap { after: 2, em: 0.3 }], None);
    field_select(&mut app, id, 0, 0);
    app.type_text("X");
    app.click_at(OUTSIDE);
    app.idle(2);
    assert_eq!(spec_of(&app, id).content, "XWTE 2026");
    assert_eq!(spec_of(&app, id).fx.gaps, [Gap { after: 3, em: 0.3 }]);

    let (mut app, id) = text_app("aaaa", |s| s.fx.gaps = vec![Gap { after: 1, em: 0.2 }], None);
    field_select(&mut app, id, 1, 1);
    app.type_text("a");
    app.click_at(OUTSIDE);
    app.idle(2);
    assert_eq!(spec_of(&app, id).content, "aaaaa");
    assert_eq!(spec_of(&app, id).fx.gaps, [Gap { after: 2, em: 0.2 }], "among equal letters the typed position decides");
}

/// TEXT 8.2 `the_cursor_in_the_field_selects_a_gap`.
#[test]
fn the_cursor_in_the_field_selects_a_gap() {
    let (mut app, id) = text_app("WTE 2026", |_| {}, Some(TextPage::Character));
    field_select(&mut app, id, 2, 2);
    app.idle(1);
    assert_eq!(app.state.session.as_ref().unwrap().text_gap, Some(TextGapSel { stamp: id, after: 1 }));
    assert!(app.take().iter().all(|a| !matches!(a, AppAction::UpdateStampSource(_, _, Commit::Final))), "moving the cursor edits nothing");
}

// ---------------------------------------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------------------------------------

/// TEXT 8.2 `text_rows_fit_the_panel`.
#[test]
fn text_rows_fit_the_panel() {
    let check = |app: &PanelApp, what: &str| {
        let r = &app.state.side_rects;
        assert!(r.panel.width() <= 300.5, "{what}: the side panel is {} wide", r.panel.width());
        assert!(r.panel.contains_rect(r.upper_area), "{what}: the upper group is inside the side panel: {r:?}");
        let top = r.tool_area.min.y;
        let b = bold_button(app);
        assert!(b.rect.max.y - top <= 116.0, "{what}: rows 1 to 3 end {} pt below the top", b.rect.max.y - top);
        // Every row of the block is at most 265 pt wide.
        let rows: Vec<Vec<Node>> = vec![
            vec![app.h.nodes.iter().find(|n| n.role == Role::MultilineTextInput).cloned().expect("the content field")],
            vec![b.clone(), right_of(app, &b, Role::SpinButton, 1), right_of(app, &b, Role::ColorWell, 0)],
            vec![app.h.prefix("Warp: "), app.h.prefix("Path: "), app.h.get("More")],
        ];
        for row in &rows {
            for n in row {
                assert!(n.rect.max.x <= r.tool_area.max.x + 0.5, "{what}: {n:?} reaches past the tool options {:?}", r.tool_area);
            }
        }
        for row in rows {
            let width = row.iter().map(|n| n.rect.max.x).fold(f32::MIN, f32::max) - row.iter().map(|n| n.rect.min.x).fold(f32::MAX, f32::min);
            assert!(width <= 265.5, "{what}: a row is {width} pt wide: {row:?}");
        }
    };
    let mut app = PanelApp::new(sample_state(SampleKind::Sphere));
    app.state.tool = ToolKind::Text;
    app.idle(3);
    check(&app, "Text tool");
    // The default labels fit their buttons, laid out with the button font.
    let pad = app.h.ctx.global_style().spacing.button_padding.x;
    let width_of = |text: &str| {
        let font = egui::TextStyle::Button.resolve(&app.h.ctx.global_style());
        app.h.ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE).size().x)
    };
    for (label, button) in [("Warp: None", 100.0), ("Path: Straight", 104.0), ("Path: Circle", 104.0)] {
        assert!(width_of(label) <= button - 2.0 * pad, "{label:?} is {} wide", width_of(label));
    }
    let (mut app, _) = text_app("WTE 2026", |s| s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::ShellLower) }), None);
    app.idle(2);
    check(&app, "a selected text with Warp: Shell Lower");
    assert!(app.h.has("Warp: Shell Lower"));
}

/// TEXT 8.2 `the_colour_section_stays_inside_the_panel_on_every_page` (1280 x 800, a text of 60 letters). objects-tabs
/// (WP4): the upper group stays inside the panel and above the lower one, with the Tool and with the Color tab shown;
/// the Color page is then on screen whole (G1).
#[test]
fn the_colour_section_stays_inside_the_panel_on_every_page() {
    let long: String = "ABCDEFGHIJ".repeat(6);
    let (mut app, _) = text_app(&long, |_| {}, Some(TextPage::Character));
    for (page, upper) in [TextPage::Character, TextPage::Warp, TextPage::Path].into_iter().flat_map(|p| [(p, UpperTab::Tool), (p, UpperTab::Color)]) {
        app.state.text_page = page;
        app.state.upper_tab = upper;
        app.idle(3);
        let r = app.state.side_rects.clone();
        let tab = r.text_tab.unwrap_or_else(|| panic!("{page:?}: the Text tab is shown"));
        assert!(r.panel.contains_rect(r.upper_area) && r.upper_area.bottom() <= r.bottom_area.top() + 0.5, "{page:?} {upper:?}: {r:?}");
        if upper == UpperTab::Color {
            let content = app.state.panel_rects[&PanelHit::ColorContent];
            assert!(r.panel.contains_rect(r.color) && r.color.expand(0.5).contains_rect(content), "{page:?}: the Color page is cut off: {content:?} in {r:?}");
        }
        assert!(r.bottom_area.contains_rect(tab), "{page:?}: {r:?}");
        assert!(r.panel.width() <= 300.5, "{page:?}: the side panel is {} wide", r.panel.width());
        assert!(app.take().is_empty(), "{page:?}: an idle page queues nothing");
    }
}

/// TEXT 8.2 `the_text_tab_follows_the_tool_and_the_selection` (every line through `side_rects`, `side_tab = Text`).
#[test]
fn the_text_tab_follows_the_tool_and_the_selection() {
    let mut state = sample_state(SampleKind::Sphere);
    let text = place_text(&mut state, "One", |_| {});
    state.side_tab = SideTab::Text;
    state.tool = ToolKind::Stamp;
    let mut app = PanelApp::new(state);
    let shown = |app: &mut PanelApp| {
        app.idle(2);
        app.state.side_rects.text_tab.is_some()
    };
    assert!(!shown(&mut app), "Stamp tool, nothing selected: Layers");
    let text_button = app.state.side_rects.tab_buttons[SideTab::Text.index()];
    assert!(app.h.nodes.iter().any(|n| n.label == "Text" && n.disabled && n.rect == text_button), "the tab is greyed out");
    app.state.tool = ToolKind::Text;
    assert!(shown(&mut app), "Text tool: the Text tab");
    // The Text tool placed the text: it is selected and the tool is Transform.
    app.state.session.as_mut().unwrap().selected_stamp = Some(text);
    app.state.tool = ToolKind::Transform;
    assert!(shown(&mut app), "it stays shown after placing");
    app.state.tool = ToolKind::Brush;
    assert!(!shown(&mut app), "the Brush: Layers");
    assert_eq!(app.state.side_tab, SideTab::Text);
    assert_eq!(app.state.session.as_ref().unwrap().selected_stamp, Some(text), "the text is still selected");
    app.state.tool = ToolKind::Transform;
    assert!(shown(&mut app), "Transform again: the Text tab without a click");
    // An image stamp selected with the Transform tool: Layers.
    let img = wte_core::samples::sample_stamp_image(16);
    let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
    let doc = app.state.doc_mut().unwrap();
    let blob = doc.add_image_blob("ring.png", std::sync::Arc::from(png)).unwrap();
    let p = match doc.stamp(text).unwrap().placement {
        Placement::Projected(p) => p,
        Placement::Uv(_) => unreachable!(),
    };
    let image = doc.add_stamp("ring", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
    app.state.session.as_mut().unwrap().selected_stamp = Some(image);
    assert!(!shown(&mut app), "an image stamp: Layers");
    app.state.session.as_mut().unwrap().selected_stamp = None;
    assert!(!shown(&mut app), "nothing selected: Layers");
    // (round 5 R5-3: the font button asks for the samples once per catalog.)
    assert!(app.take().iter().all(|a| matches!(a, AppAction::LoadFontSamples)));
}

/// TEXT 8.2 `the_gap_selection_dies_with_the_session`.
#[test]
fn the_gap_selection_dies_with_the_session() {
    let (mut app, id) = text_app("WTE 2026", |_| {}, Some(TextPage::Character));
    app.state.session.as_mut().unwrap().text_gap = Some(TextGapSel { stamp: id, after: 2 });
    app.idle(2);
    assert!(!in_tab(&app, "+").disabled, "a selected gap enables +");
    // A new document with a text of the same id.
    let mut fresh = sample_state(SampleKind::Sphere);
    let same = place_text(&mut fresh, "WTE 2026", |_| {});
    assert_eq!(same, id);
    let mut session = fresh.session.take().unwrap();
    session.selected_stamp = Some(same);
    session.scene_generation = 8;
    app.state.session = Some(session);
    app.idle(3);
    assert_eq!(app.state.session.as_ref().unwrap().text_gap, None);
    assert!(in_tab(&app, "+").disabled, "+ is disabled");
    assert!(!app.h.nodes.iter().any(|n| n.label.starts_with("Gap between") && n.toggled == Some(true)), "no slot is framed");
}

// ---------------------------------------------------------------------------------------------------------------------
// Target: which text the rows edit
// ---------------------------------------------------------------------------------------------------------------------

/// TEXT 8.2 `the_text_tool_edits_the_draft_not_the_selected_text` (placing the second text is the tool's, T7: here
/// the draft the click would place is checked).
#[test]
fn the_text_tool_edits_the_draft_not_the_selected_text() {
    let (mut app, id) = text_app("One", |_| {}, None);
    app.state.tool = ToolKind::Text;
    app.idle(2);
    let field = app.h.nodes.iter().find(|n| n.role == Role::MultilineTextInput).cloned().unwrap();
    assert_eq!(field.value, "Text", "the field shows the draft, not \"One\"");
    app.click_at(field.rect.center());
    app.press(Key::A, Modifiers::COMMAND);
    app.type_text("Bob");
    app.click_at(OUTSIDE);
    app.idle(1);
    assert_eq!(app.state.tools.text.content, "Bob");
    assert_eq!(spec_of(&app, id).content, "One");
    assert!(app.h.has("Selected: One") && app.h.has("Edit"), "the Selected row with its Edit button");
    app.click_at(bold_button(&app).rect.center());
    assert!(app.state.tools.text.bold_em > 0.0, "B changes the draft");
    assert_eq!(spec_of(&app, id).fx.bold_em, 0.0, "and not the stamp");
    assert!(app.take().iter().all(|a| !matches!(a, AppAction::UpdateStampSource(..) | AppAction::UpdateStampPlacement(..))), "nothing queued");
}

/// TEXT 8.2 `the_transform_tool_edits_the_selected_text`.
#[test]
fn the_transform_tool_edits_the_selected_text() {
    let (mut app, id) = text_app("One", |_| {}, None);
    let draft = app.state.tools.text.clone();
    app.click_at(bold_button(&app).rect.center());
    let actions = app.take();
    assert!(matches!(&actions[..], [AppAction::UpdateStampSource(s, _, Commit::Final)] if *s == id), "{actions:?}");
    assert!(spec_of(&app, id).fx.bold_em > 0.0);
    assert_eq!(app.state.tools.text, draft, "the draft is left alone");
}

/// TEXT 8.2 `a_change_before_placing_changes_the_draft_and_queues_nothing`.
#[test]
fn a_change_before_placing_changes_the_draft_and_queues_nothing() {
    let mut state = sample_state(SampleKind::Sphere);
    state.tool = ToolKind::Text;
    state.side_tab = SideTab::Text;
    state.text_page = TextPage::Character;
    let mut app = PanelApp::new(state);
    let slider = app.h.in_row("Thicken", Role::Slider, 0);
    drag_node(&mut app, &slider, 40.0);
    assert!(app.state.tools.text.bold_em > 0.0, "Thicken changed the draft");
    let slider = app.h.in_row("Slant", Role::Slider, 0);
    drag_node(&mut app, &slider, 40.0);
    assert!(app.state.tools.text.slant_deg > 0.0);
    app.state.text_page = TextPage::Warp;
    app.idle(2);
    app.click_at(in_tab(&app, "Flag").rect.center());
    assert_eq!(app.state.tools.text.warp.map(|w| (w.style, w.bend)), Some((WarpStyle::Flag, 0.5)));
    app.state.text_page = TextPage::Path;
    app.idle(2);
    app.click_at(in_tab(&app, "Arc").rect.center());
    assert!(matches!(app.state.tools.text.path.as_ref().map(|p| &p.shape), Some(PathShape::Arc { .. })));
    assert!(in_tab(&app, "Curve").disabled, "Curve needs a placed text");
    assert!(app.take().is_empty(), "the draft queues nothing");
}

/// TEXT 8.2 `idle_text_controls_queue_no_action` (the four rows and every page, 10 idle frames, every feature set).
#[test]
fn idle_text_controls_queue_no_action() {
    let (mut app, _) = text_app(
        "WTE 2026\nline two",
        |s| {
            s.fx.gaps = vec![Gap { after: 2, em: 0.3 }, Gap { after: 7, em: 0.1 }];
            s.fx.bold_em = 0.02;
            s.fx.slant_deg = 12.0;
            s.fx.warp = Some(Warp { bend: 0.4, horizontal: 0.1, vertical: -0.2, ..Warp::new(WarpStyle::Wave) });
            s.fx.path = Some(wte_core::TextPath { shape: PathShape::Arc { bend: 0.3 }, start: 0.2, flip: false });
            s.outline = Some(wte_core::Outline { width_em: 0.05, color: [255; 4] });
        },
        Some(TextPage::Character),
    );
    for page in [TextPage::Character, TextPage::Warp, TextPage::Path] {
        app.state.text_page = page;
        app.idle(10);
        assert!(app.take().is_empty(), "{page:?}");
    }
    app.state.tool = ToolKind::Text;
    app.idle(10);
    assert!(app.take().is_empty(), "Text tool");
}

/// Check of T6: a placed text whose Text size lies outside the field's range (0.2 to 400 %; the gizmo and the Size field
/// can scale a text that far) is shown as it is: idle frames queue nothing and the placement is not changed.
#[test]
fn an_out_of_range_text_size_is_not_changed_by_showing_it() {
    for factor in [0.01f32, 100.0] {
        let mut state = sample_state(SampleKind::Sphere);
        let id = place_text(&mut state, "Tiny", |_| {});
        let doc = state.doc_mut().unwrap();
        let mut placement = doc.stamp(id).unwrap().placement;
        if let Placement::Projected(p) = &mut placement {
            p.half_size *= factor;
        }
        doc.set_stamp_placement(id, placement, Commit::Final).unwrap();
        let placement = doc.stamp(id).unwrap().placement;
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        let mut app = PanelApp::new(state);
        app.apply = true;
        app.idle(10);
        let actions = app.take();
        assert!(actions.is_empty(), "factor {factor}: {actions:?}");
        assert_eq!(app.state.doc().unwrap().stamp(id).unwrap().placement, placement, "factor {factor}");
    }
}

/// Check of T6: an angle outside -180..180 (a UV rotation is stored verbatim, here 4 rad = 229 degrees) or exactly at
/// the seam (180 degrees) is shown as it is: idle frames queue nothing and the placement is not turned.
#[test]
fn an_angle_outside_the_field_range_is_not_changed_by_showing_it() {
    for (uv, deg) in [(true, 4.0f32.to_degrees()), (false, 180.0), (false, -180.0)] {
        let mut state = sample_state(SampleKind::Sphere);
        let id = place_text(&mut state, "Turned", |_| {});
        let doc = state.doc_mut().unwrap();
        let placement = if uv {
            Placement::Uv(wte_core::UvPlacement {
                set: TextureSetId(0),
                center_px: GVec2::new(512.0, 512.0),
                size_px: GVec2::new(200.0, 50.0),
                rotation: deg.to_radians(),
                flip_x: false,
                flip_y: false,
                clip_to_islands: false,
            })
        } else {
            crate::panels::tool_options::rotate_placement_to(&doc.stamp(id).unwrap().placement, deg)
        };
        doc.set_stamp_placement(id, placement, Commit::Final).unwrap();
        let placement = doc.stamp(id).unwrap().placement;
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        let mut app = PanelApp::new(state);
        app.apply = true;
        app.idle(10);
        let actions = app.take();
        assert!(actions.is_empty(), "{deg} degrees (uv {uv}): {actions:?}");
        assert_eq!(app.state.doc().unwrap().stamp(id).unwrap().placement, placement, "{deg} degrees (uv {uv})");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Gap spacing
// ---------------------------------------------------------------------------------------------------------------------

/// TEXT 8.2 `a_slot_click_and_a_drag_change_one_gap`.
#[test]
fn a_slot_click_and_a_drag_change_one_gap() {
    let (mut app, id) = text_app("WTE 2026", |_| {}, Some(TextPage::Character));
    let undo0 = undo_len(&app);
    app.click_at(slot(&app, "Gap between E and (space)").rect.center());
    assert_eq!(app.state.session.as_ref().unwrap().text_gap, Some(TextGapSel { stamp: id, after: 2 }));
    app.idle(1);
    let minus = in_tab(&app, "-");
    let value = right_of(&app, &minus, Role::SpinButton, 0);
    assert!(!value.disabled);
    app.take();
    drag_node(&mut app, &value, 150.0);
    let actions = app.take();
    assert_previews_then_one_final(&actions, "the gap value drag");
    let spec = spec_of(&app, id);
    assert_eq!(spec.fx.gaps.len(), 1, "{:?}", spec.fx.gaps);
    assert_eq!(spec.fx.gaps[0].after, 2);
    assert!((0.25..=0.35).contains(&spec.fx.gaps[0].em), "{:?}", spec.fx.gaps);
    assert_eq!(undo_len(&app), undo0 + 1);
}

/// TEXT 8.2 `nudge_and_reset`.
#[test]
fn nudge_and_reset() {
    let (mut app, id) = text_app("WTE 2026", |s| s.fx.gaps = vec![Gap { after: 0, em: 0.1 }, Gap { after: 5, em: 0.2 }], Some(TextPage::Character));
    app.click_at(slot(&app, "Gap between E and (space)").rect.center());
    app.idle(1);
    for _ in 0..2 {
        let plus = in_tab(&app, "+");
        app.click_at(plus.rect.center());
        assert_one_final(&app.take(), "+");
    }
    assert!((wte_core::text::gaps::get(&spec_of(&app, id).fx.gaps, 2) - 0.02).abs() < 1e-6);
    let reset = in_tab(&app, "Reset");
    app.click_at(reset.rect.center());
    assert_one_final(&app.take(), "Reset");
    assert_eq!(wte_core::text::gaps::get(&spec_of(&app, id).fx.gaps, 2), 0.0, "the entry is gone");
    app.click_at(slot(&app, "Gap between E and (space)").rect.center());
    let plus = in_tab(&app, "+");
    app.click_at(plus.rect.center());
    app.take();
    assert_eq!(spec_of(&app, id).fx.gaps.len(), 3);
    let all = in_tab(&app, "Reset all");
    app.click_at(all.rect.center());
    assert_one_final(&app.take(), "Reset all");
    assert!(spec_of(&app, id).fx.gaps.is_empty());
    app.idle(1);
    assert!(in_tab(&app, "Reset all").disabled, "nothing left to reset");
}

/// TEXT 8.2 `the_strip_shows_one_line`.
#[test]
fn the_strip_shows_one_line() {
    let (mut app, _) = text_app("ab\ncd\nef", |_| {}, Some(TextPage::Character));
    assert!(app.h.has("line 1 of 3"));
    assert!(app.h.nodes.iter().any(|n| n.label.starts_with("Gap between a and b")));
    app.click_at(in_tab(&app, ">").rect.center());
    app.idle(1);
    assert!(app.h.has("line 2 of 3"));
    assert!(app.h.nodes.iter().any(|n| n.label.starts_with("Gap between c and d")));
    assert!(!app.h.nodes.iter().any(|n| n.label.starts_with("Gap between a and")), "only the shown line");
    assert!(!app.h.nodes.iter().any(|n| n.label.starts_with("Gap between d")), "the last character of a line has no slot");
}

/// TEXT 8.2 `the_end_slot_shows_a_stored_value`.
#[test]
fn the_end_slot_shows_a_stored_value() {
    let (mut app, id) = text_app("WTE 2026", |s| s.fx.gaps = vec![Gap { after: 4, em: 0.3 }], Some(TextPage::Character));
    field_select(&mut app, id, 5, 8);
    app.press(Key::Delete, Modifiers::NONE);
    app.click_at(OUTSIDE);
    app.idle(2);
    assert_eq!(spec_of(&app, id).content, "WTE 2");
    assert_eq!(spec_of(&app, id).fx.gaps, [Gap { after: 4, em: 0.3 }]);
    app.take();
    let end = slot(&app, "Stored gap after 2: 30 %");
    assert!(end.label.contains("no effect at the end of a line"), "{end:?}");
    app.click_at(end.rect.center());
    app.idle(1);
    app.click_at(in_tab(&app, "Reset").rect.center());
    assert_one_final(&app.take(), "Reset of the end slot");
    assert!(spec_of(&app, id).fx.gaps.is_empty());
    app.idle(1);
    assert!(!app.h.nodes.iter().any(|n| n.label.starts_with("Stored gap after")), "the end slot is gone");
}

/// TEXT 8.2 `the_strip_lays_out_only_visible_cells`.
#[test]
fn the_strip_lays_out_only_visible_cells() {
    let line: String = (0..1000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let (mut app, id) = text_app(&line, |s| s.size_px = 8.0, Some(TextPage::Character));
    app.idle(2);
    let (boxes, slots) = app.ps.text.strip_drawn;
    assert!(boxes <= 14 && slots <= 14 && boxes > 0, "{boxes} boxes, {slots} slots");
    assert!(app.state.side_rects.panel.width() <= 300.5);
    // The selected last gap is scrolled into view.
    app.state.session.as_mut().unwrap().text_gap = Some(TextGapSel { stamp: id, after: 998 });
    app.ps.text.scroll_to_gap = true;
    app.idle(3);
    let last = format!("Gap between {} and {}", char::from(b'a' + (998 % 26) as u8), char::from(b'a' + (999 % 26) as u8));
    let tab = app.state.side_rects.text_tab.unwrap();
    let node = app.h.nodes.iter().find(|n| n.label.starts_with(&last) && n.toggled == Some(true)).cloned().expect("the last slot is drawn");
    assert!(tab.contains(node.rect.center()), "the last slot is visible: {node:?}");
    let (boxes, slots) = app.ps.text.strip_drawn;
    assert!(boxes <= 14 && slots <= 14, "{boxes} boxes, {slots} slots at the end");
}

/// TEXT 8.2 `gap_texts_name_the_real_pair`.
#[test]
fn gap_texts_name_the_real_pair() {
    let (mut app, _) = text_app("WTE 2026", |_| {}, Some(TextPage::Character));
    let node = slot(&app, "Gap between E and (space)");
    assert_eq!(node.label, "Gap between E and (space): 0 %");
    assert!(app.h.nodes.iter().any(|n| n.label.starts_with("Gap between (space) and 2")), "the other gap of the pair");
    app.click_at(node.rect.center());
    app.idle(1);
    assert_eq!(app.state.status_hint, "Gap between E and (space): 0 % of the text size");
    app.state.tool = ToolKind::Brush;
    app.idle(2);
    assert!(app.state.status_hint.is_empty(), "the status line goes with the tab: {:?}", app.state.status_hint);
}

/// TEXT 8.2 `gap_controls_are_disabled_before_placing`.
#[test]
fn gap_controls_are_disabled_before_placing() {
    let mut state = sample_state(SampleKind::Sphere);
    state.tool = ToolKind::Text;
    state.side_tab = SideTab::Text;
    let app = PanelApp::new(state);
    for label in ["-", "+", "Reset", "Reset all"] {
        assert!(in_tab(&app, label).disabled, "{label}");
    }
    let minus = in_tab(&app, "-");
    assert!(right_of(&app, &minus, Role::SpinButton, 0).disabled);
}

// ---------------------------------------------------------------------------------------------------------------------
// Other controls
// ---------------------------------------------------------------------------------------------------------------------

/// TEXT 8.2 `bold_formats_the_whole_text` (Ubuntu has no bold face: thickening).
#[test]
fn bold_formats_the_whole_text() {
    let (mut app, id) = text_app("WTE", |_| {}, None);
    app.click_at(bold_button(&app).rect.center());
    let finals = sources(&app.take(), Commit::Final);
    assert_eq!(finals.len(), 1);
    assert_eq!(spec_of(&app, id).fx.bold_em, 0.02);
    assert_eq!(spec_of(&app, id).font.style, "Light");
    app.idle(1);
    assert_eq!(bold_button(&app).toggled, Some(true), "B is lit");
    app.click_at(bold_button(&app).rect.center());
    assert_one_final(&app.take(), "B off");
    assert_eq!(spec_of(&app, id).fx.bold_em, 0.0);
}

/// TEXT 8.2 `an_embedded_font_can_be_given_to_another_text`.
#[test]
fn an_embedded_font_can_be_given_to_another_text() {
    let mut state = sample_state(SampleKind::Sphere);
    let hack = state.fonts.find("Hack", "Regular").expect("the bundled Hack");
    let (bytes, index) = state.fonts.face_bytes(hack.id).unwrap();
    let doc = state.doc_mut().unwrap();
    let project_font = doc.register_font("Projectonly", "Regular", index, bytes);
    let first = place_text(&mut state, "First", |s| s.font = project_font.clone());
    let second = place_text(&mut state, "Second", |_| {});
    let _ = first;
    state.session.as_mut().unwrap().selected_stamp = Some(second);
    state.tool = ToolKind::Transform;
    let mut app = PanelApp::new(state);
    app.apply = true;
    let blobs = app.state.doc().unwrap().blobs.iter().count();
    app.click_at(app.h.get("Ubuntu").rect.center());
    app.idle(1);
    assert!(app.h.has("In this project"));
    let row = app.h.get("Projectonly");
    app.click_at(row.rect.center());
    app.idle(1);
    let finals = sources(&app.take(), Commit::Final);
    assert_eq!(finals.len(), 1, "one Final");
    assert_eq!(spec_of(&app, second).font, project_font, "the FontRef of the first text");
    assert_eq!(app.state.doc().unwrap().blobs.iter().count(), blobs, "no blob was added");
    assert!(app.state.toasts.visible_texts().is_empty(), "no toast: {:?}", app.state.toasts.visible_texts());
    assert!(!egui::Popup::is_any_open(&app.h.ctx), "the picker closed");

    // With the Text tool: the same click sets the draft's family, and the font of the project is the one placed.
    app.state.tool = ToolKind::Text;
    app.idle(2);
    app.click_at(app.h.get("Ubuntu").rect.center());
    app.idle(1);
    app.click_at(app.h.get("Projectonly").rect.center());
    app.idle(1);
    assert_eq!(app.state.tools.text.family, "Projectonly");
    assert!(app.take().is_empty());
    let (family, style) = (app.state.tools.text.family.clone(), app.state.tools.text.style.clone());
    assert_eq!(text_block::resolve_font(&mut app.state, &family, &style), Some(project_font));
}

/// The machine's Arial (Regular and Bold) in a catalog with the bundled fonts; `None` without it.
fn catalog_with_arial() -> Option<FontCatalog> {
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    let (regular, bold) = (dir.join("arial.ttf"), dir.join("arialbd.ttf"));
    if !regular.is_file() || !bold.is_file() {
        eprintln!("SKIP: Arial is not installed");
        return None;
    }
    let mut fonts = FontCatalog::bundled();
    fonts.load_font_file(&regular).ok()?;
    fonts.load_font_file(&bold).ok()?;
    Some(fonts)
}

/// TEXT 8.2 `a_font_change_resolves_bold_again` (with the machine's Arial Regular and Bold as the hand-made family;
/// reads `C:\Windows\Fonts`, skips without it).
#[test]
fn a_font_change_resolves_bold_again() {
    let Some(fonts) = catalog_with_arial() else { return };
    let (mut app, id) = text_app("WTE", |s| s.fx.bold_em = 0.02, None);
    app.state.fonts = fonts;
    app.idle(2);
    let pick = |app: &mut PanelApp, from: &str, family: &str| {
        let button = app.h.all(from).into_iter().find(|n| n.role == Role::Button).expect("the font button");
        app.click_at(button.rect.center());
        app.idle(1);
        let row = app.h.all(family).into_iter().rev().find(|n| n.role == Role::Button).expect("the picker row");
        app.click_at(row.rect.center());
        app.idle(2);
    };
    pick(&mut app, "Ubuntu", "Arial");
    let spec = spec_of(&app, id);
    assert_eq!((spec.font.family.as_str(), spec.font.style.as_str(), spec.fx.bold_em), ("Arial", "Bold", 0.0));
    pick(&mut app, "Arial", "Ubuntu");
    let spec = spec_of(&app, id);
    assert_eq!((spec.font.family.as_str(), spec.font.style.as_str(), spec.fx.bold_em), ("Ubuntu", "Light", 0.02));
}

/// TEXT 8.2 `the_font_picker_filters_and_takes_a_font`.
#[test]
fn the_font_picker_filters_and_takes_a_font() {
    let mut state = sample_state(SampleKind::Sphere);
    state.tool = ToolKind::Text;
    let mut app = PanelApp::new(state);
    let before = app.h.nodes.clone();
    app.click_at(app.h.get("Ubuntu").rect.center());
    app.idle(1);
    assert!(egui::Popup::is_any_open(&app.h.ctx), "the picker is open");
    assert!(app.take().iter().all(|a| matches!(a, AppAction::LoadFontSamples)), "opening asks for the samples only");
    app.type_text("ha");
    app.idle(1);
    let rows: Vec<String> = app
        .h
        .new_since(&before)
        .into_iter()
        .filter(|n| n.role == Role::Button && n.rect.width() > 200.0)
        .map(|n| n.label)
        .collect();
    assert_eq!(rows, ["Hack"], "only Hack matches");
    assert!(app.h.has("All fonts (1 of 2)") || app.h.nodes.iter().any(|n| n.value.starts_with("All fonts (1 of")));
    app.press(Key::Enter, Modifiers::NONE);
    app.idle(1);
    assert_eq!(app.state.tools.text.family, "Hack");
    assert!(!egui::Popup::is_any_open(&app.h.ctx), "the picker closed");
    assert!(app.state.side_rects.panel.width() <= 300.5);
}

/// TEXT 8.2 `the_warp_page_applies_a_style_and_one_drag_is_one_entry`.
#[test]
fn the_warp_page_applies_a_style_and_one_drag_is_one_entry() {
    let (mut app, id) = text_app("WTE 2026", |_| {}, None);
    let undo0 = undo_len(&app);
    app.click_at(app.h.get("Warp: None").rect.center());
    app.idle(1);
    assert_eq!((app.state.side_tab, app.state.text_page), (SideTab::Text, TextPage::Warp));
    app.click_at(in_tab(&app, "Arc").rect.center());
    assert_one_final(&app.take(), "Arc");
    assert_eq!(spec_of(&app, id).fx.warp.map(|w| (w.style, w.bend)), Some((WarpStyle::Arc, 0.5)));
    app.idle(1);
    let bend = app.h.in_row("Bend", Role::Slider, 0);
    drag_node(&mut app, &bend, 30.0);
    assert_previews_then_one_final(&app.take(), "Bend");
    assert_ne!(spec_of(&app, id).fx.warp.unwrap().bend, 0.5);
    assert_eq!(undo_len(&app), undo0 + 2);
    // A click on the button of the page that is showing switches back to Layers.
    app.click_at(app.h.get("Warp: Arc").rect.center());
    app.idle(1);
    assert_eq!(app.state.side_tab, SideTab::Layers);
}

/// The anchor of placed text `id` on the model.
fn anchor_of(app: &mut PanelApp, id: StampId) -> Vec3 {
    let (placement, frame) = text_block::stamp_frame(&mut app.state, id).unwrap();
    wte_core::text::EmMap::new(&placement, &frame).to_world([0.0, 0.0])
}

/// TEXT 8.2 `the_path_page_creates_shapes_that_move_nothing` (the anchor is compared on the model; the baked bounds
/// are the scenario `text_path`'s).
#[test]
fn the_path_page_creates_shapes_that_move_nothing() {
    let (mut app, id) = text_app("WTE 2026", |_| {}, Some(TextPage::Path));
    let anchor = anchor_of(&mut app, id);
    let texel = app.state.doc().unwrap().mesh.bounds.diagonal() / 2048.0;
    app.click_at(in_tab(&app, "Curve").rect.center());
    assert_one_final(&app.take(), "Curve");
    let points = |app: &PanelApp| match spec_of(app, id).fx.path.map(|p| p.shape) {
        Some(PathShape::Curve { points }) => points.len(),
        other => panic!("{other:?}"),
    };
    assert_eq!(points(&app), 3);
    assert!(anchor_of(&mut app, id).distance(anchor) < texel, "the anchor stays");
    app.idle(1);
    app.click_at(in_tab(&app, "Add point").rect.center());
    assert_eq!(points(&app), 4);
    app.idle(1);
    app.click_at(in_tab(&app, "Remove point").rect.center());
    app.idle(1);
    app.click_at(in_tab(&app, "Remove point").rect.center());
    assert_eq!(points(&app), 2);
    app.idle(1);
    assert!(in_tab(&app, "Remove point").disabled, "never below 2");
    for _ in 0..14 {
        app.click_at(in_tab(&app, "Add point").rect.center());
        app.idle(1);
    }
    assert_eq!(points(&app), 16);
    assert!(in_tab(&app, "Add point").disabled, "never above 16");
}

/// TEXT 8.2 `the_path_page_creates_a_centred_circle` (the circle is checked on its frame: the hint and Fit; the ink
/// centre is the scenario `text_path`'s).
#[test]
fn the_path_page_creates_a_centred_circle() {
    let (mut app, id) = text_app("WTE 2026", |_| {}, Some(TextPage::Path));
    app.click_at(in_tab(&app, "Circle").rect.center());
    assert_one_final(&app.take(), "Circle");
    let Some(PathShape::Circle { center, .. }) = spec_of(&app, id).fx.path.map(|p| p.shape) else { panic!("a circle") };
    assert!(center[0].abs() < 1e-6 && center[1] > 0.0, "centred under the top point: {center:?}");
    app.idle(1);
    assert!(!app.h.has(CIRCLE_TOO_SHORT));
    let mut longer = spec_of(&app, id);
    longer.content.push_str(" ABCDEFGHIJK");
    app.state.push(AppAction::UpdateStampSource(id, StampSource::Text(longer), Commit::Final));
    app.idle(3);
    assert!(app.h.has(CIRCLE_TOO_SHORT), "the hint");
    app.take();
    app.click_at(in_tab(&app, "Fit").rect.center());
    assert_one_final(&app.take(), "Fit");
    let (_, frame) = text_block::stamp_frame(&mut app.state, id).unwrap();
    assert!((frame.circle_fill - 0.5).abs() <= 0.01, "circle_fill {}", frame.circle_fill);
    app.idle(2);
    assert!(!app.h.has(CIRCLE_TOO_SHORT), "the hint is gone");
}

/// TEXT 8.2 `the_angle_field_turns_the_right_text`.
#[test]
fn the_angle_field_turns_the_right_text() {
    let (mut app, id) = text_app("One", |_| {}, None);
    let rotation = |app: &PanelApp| placement_rotation_deg(&app.state.doc().unwrap().stamp(id).unwrap().placement);
    let origin = |app: &PanelApp| match app.state.doc().unwrap().stamp(id).unwrap().placement {
        Placement::Projected(p) => p.origin,
        Placement::Uv(_) => unreachable!(),
    };
    let texel = app.state.doc().unwrap().mesh.bounds.diagonal() / 2048.0;
    // (a) The Text tool: the draft's angle, nothing queued, the stamp unchanged.
    app.state.tool = ToolKind::Text;
    app.idle(2);
    let undo0 = undo_len(&app);
    let (_, angle) = size_and_angle(&app);
    type_into(&mut app, &angle, "30");
    assert_eq!(app.state.tools.stamp.rotation_deg, 30.0);
    assert!(app.take().is_empty());
    assert_eq!(undo_len(&app), undo0);
    assert!(rotation(&app).abs() < 1e-3);
    // (b) The Transform tool: the field shows the stamp's rotation; 30 is one placement Final.
    app.state.tool = ToolKind::Transform;
    app.idle(2);
    let (_, angle) = size_and_angle(&app);
    assert!(angle.number.unwrap().abs() < 1e-3, "{angle:?}");
    let o0 = origin(&app);
    type_into(&mut app, &angle, "30");
    let actions = app.take();
    let finals = actions.iter().filter(|a| matches!(a, AppAction::UpdateStampPlacement(s, _, Commit::Final) if *s == id)).count();
    assert_eq!(finals, 1, "one placement Final: {actions:?}");
    assert!(actions.iter().all(|a| matches!(a, AppAction::UpdateStampPlacement(s, ..) if *s == id)), "only placement edits: {actions:?}");
    assert!((rotation(&app) - 30.0).abs() < 0.01, "{}", rotation(&app));
    assert!(origin(&app).distance(o0) < 0.5 * texel, "the centre stays");
    assert_eq!(undo_len(&app), undo0 + 1);
    assert_eq!(app.state.tools.stamp.rotation_deg, 30.0, "the draft was not written");
    // (c) The Rotation field of the placement fields reads the same.
    app.idle(1);
    let row = reveal_row(&mut app, "Rotation", Role::SpinButton);
    assert!((row.number.unwrap() - 30.0).abs() < 0.01, "{row:?}");
    let area = *app.state.panel_rects.get(&PanelHit::SettingsArea).unwrap();
    for _ in 0..20 {
        app.scroll_at(area.center(), vec2(0.0, 1.0));
    }
    app.idle(3);
    // (d) 270 wraps to -90.
    let (_, angle) = size_and_angle(&app);
    type_into(&mut app, &angle, "270");
    app.take();
    assert!((rotation(&app) + 90.0).abs() < 0.01, "{}", rotation(&app));
    app.idle(1);
    let (_, angle) = size_and_angle(&app);
    assert!((angle.number.unwrap() + 90.0).abs() < 0.01, "{angle:?}");
    // (e) A drag: previews, one Final, one undo entry.
    let undo1 = undo_len(&app);
    drag_node(&mut app, &angle, 30.0);
    assert_previews_then_one_final(&app.take(), "the angle drag");
    assert_eq!(undo_len(&app), undo1 + 1);
    // (f) Three undos: the drag, the 270, the 30.
    let after_drag = rotation(&app);
    assert!((after_drag + 90.0).abs() > 1.0);
    for expected in [-90.0, 30.0, 0.0] {
        app.state.doc_mut().unwrap().undo().expect("an undo");
        assert!((rotation(&app) - expected).abs() < 0.01, "after undo: {} (expected {expected})", rotation(&app));
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// One sweep over the controls (TEXT 8.2 `every_text_control_changes_exactly_its_own_field`)
// ---------------------------------------------------------------------------------------------------------------------

/// The names of the fields in which two specs differ.
fn changed_fields(a: &TextSpec, b: &TextSpec) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut check = |name: &'static str, differs: bool| {
        if differs {
            out.push(name);
        }
    };
    check("content", a.content != b.content);
    check("font", a.font != b.font);
    check("color", a.color != b.color);
    check("align", a.align != b.align);
    check("line_spacing", a.line_spacing != b.line_spacing);
    check("letter_spacing", a.letter_spacing != b.letter_spacing);
    check("outline", a.outline != b.outline);
    check("gaps", a.fx.gaps != b.fx.gaps);
    check("bold_em", a.fx.bold_em != b.fx.bold_em);
    check("slant_deg", a.fx.slant_deg != b.fx.slant_deg);
    let (wa, wb) = (a.fx.warp, b.fx.warp);
    check("warp.style", wa.map(|w| w.style) != wb.map(|w| w.style));
    check("warp.bend", wa.is_some() && wb.is_some() && wa.unwrap().bend != wb.unwrap().bend);
    check("warp.horizontal", wa.is_some() && wb.is_some() && wa.unwrap().horizontal != wb.unwrap().horizontal);
    check("warp.vertical", wa.is_some() && wb.is_some() && wa.unwrap().vertical != wb.unwrap().vertical);
    check("warp.axis", wa.is_some() && wb.is_some() && wa.unwrap().vertical_axis != wb.unwrap().vertical_axis);
    let (pa, pb) = (a.fx.path.as_ref(), b.fx.path.as_ref());
    check("path.kind", pa.map(|p| p.shape.key()) != pb.map(|p| p.shape.key()));
    if let (Some(pa), Some(pb)) = (pa, pb) {
        check("path.start", pa.start != pb.start);
        check("path.flip", pa.flip != pb.flip);
        match (&pa.shape, &pb.shape) {
            (PathShape::Arc { bend: x }, PathShape::Arc { bend: y }) => check("path.bend", x != y),
            (PathShape::Circle { diameter: d1, center: c1 }, PathShape::Circle { diameter: d2, center: c2 }) => {
                check("path.diameter", d1 != d2);
                check("path.center", c1 != c2);
            }
            (PathShape::Curve { points: x }, PathShape::Curve { points: y }) => check("path.points", x != y),
            _ => {}
        }
    }
    out
}

/// How the sweep finds a control in the last frame.
type Finder = fn(&PanelApp) -> Node;

/// How a control is operated.
#[derive(Copy, Clone, Debug)]
enum Op {
    Click,
    Drag(f32),
}

/// One row of the sweep: the page, how to find the control, how to operate it, the fields it owns.
struct Control {
    page: Option<TextPage>,
    name: &'static str,
    find: Finder,
    op: Op,
    owns: &'static [&'static str],
    /// The control edits the placement (Text size, Angle) instead of the spec.
    placement: bool,
}

fn controls() -> Vec<Control> {
    let mut list = vec![
        Control { page: None, name: "B", find: bold_button, op: Op::Click, owns: &["bold_em"], placement: false },
        Control { page: None, name: "I", find: |a| a.h.nodes.iter().find(|n| n.role == Role::Button && n.label == "I").cloned().unwrap(), op: Op::Click, owns: &["slant_deg"], placement: false },
        Control { page: None, name: "Text size", find: |a| size_and_angle(a).0, op: Op::Drag(30.0), owns: &[], placement: true },
        Control { page: None, name: "Angle", find: |a| size_and_angle(a).1, op: Op::Drag(30.0), owns: &[], placement: true },
        Control { page: Some(TextPage::Character), name: "Align", find: |a| in_tab(a, "Center"), op: Op::Click, owns: &["align"], placement: false },
        Control { page: Some(TextPage::Character), name: "Line spacing", find: |a| a.h.in_row("Line spacing", Role::Slider, 0), op: Op::Drag(30.0), owns: &["line_spacing"], placement: false },
        Control { page: Some(TextPage::Character), name: "Letter spacing", find: |a| a.h.in_row("Letter spacing", Role::Slider, 0), op: Op::Drag(30.0), owns: &["letter_spacing"], placement: false },
        Control { page: Some(TextPage::Character), name: "Thicken", find: |a| a.h.in_row("Thicken", Role::Slider, 0), op: Op::Drag(30.0), owns: &["bold_em"], placement: false },
        Control { page: Some(TextPage::Character), name: "Slant", find: |a| a.h.in_row("Slant", Role::Slider, 0), op: Op::Drag(30.0), owns: &["slant_deg"], placement: false },
        Control { page: Some(TextPage::Character), name: "Outline", find: |a| a.h.in_row("Outline", Role::CheckBox, 0), op: Op::Click, owns: &["outline"], placement: false },
    ];
    // Each of the 15 styles (from Twist, so the first click is a change).
    let style_names: [(&str, Finder); 15] = [
        ("Arc", |a| in_tab(a, "Arc")),
        ("Arc Lower", |a| in_tab(a, "Arc Lower")),
        ("Arc Upper", |a| in_tab(a, "Arc Upper")),
        ("Arch", |a| in_tab(a, "Arch")),
        ("Bulge", |a| in_tab(a, "Bulge")),
        ("Shell Lower", |a| in_tab(a, "Shell Lower")),
        ("Shell Upper", |a| in_tab(a, "Shell Upper")),
        ("Flag", |a| in_tab(a, "Flag")),
        ("Wave", |a| in_tab(a, "Wave")),
        ("Fish", |a| in_tab(a, "Fish")),
        ("Rise", |a| in_tab(a, "Rise")),
        ("Fisheye", |a| in_tab(a, "Fisheye")),
        ("Inflate", |a| in_tab(a, "Inflate")),
        ("Squeeze", |a| in_tab(a, "Squeeze")),
        ("Twist", |a| in_tab(a, "Twist")),
    ];
    for (name, find) in style_names {
        list.push(Control { page: Some(TextPage::Warp), name, find, op: Op::Click, owns: &["warp.style"], placement: false });
    }
    list.extend([
        Control { page: Some(TextPage::Warp), name: "Bend", find: |a| a.h.in_row("Bend", Role::Slider, 0), op: Op::Drag(30.0), owns: &["warp.bend"], placement: false },
        Control { page: Some(TextPage::Warp), name: "Horizontal", find: |a| a.h.in_row("Horizontal", Role::Slider, 0), op: Op::Drag(30.0), owns: &["warp.horizontal"], placement: false },
        Control { page: Some(TextPage::Warp), name: "Vertical", find: |a| a.h.in_row("Vertical", Role::Slider, 0), op: Op::Drag(30.0), owns: &["warp.vertical"], placement: false },
        Control { page: Some(TextPage::Warp), name: "Direction", find: |a| a.h.all("Vertical").into_iter().find(|n| n.role == Role::Button).unwrap(), op: Op::Click, owns: &["warp.axis"], placement: false },
        Control { page: Some(TextPage::Warp), name: "Reset", find: |a| in_tab(a, "Reset"), op: Op::Click, owns: &["warp.style"], placement: false },
        Control { page: Some(TextPage::Path), name: "Kind Arc", find: |a| in_tab(a, "Arc"), op: Op::Click, owns: &["path.kind"], placement: false },
        Control { page: Some(TextPage::Path), name: "Arc Bend", find: |a| a.h.in_row("Bend", Role::Slider, 0), op: Op::Drag(30.0), owns: &["path.bend"], placement: false },
        Control { page: Some(TextPage::Path), name: "Start", find: |a| a.h.in_row("Start", Role::SpinButton, 0), op: Op::Drag(30.0), owns: &["path.start"], placement: false },
        Control { page: Some(TextPage::Path), name: "Other side", find: |a| a.h.in_row("Other side", Role::CheckBox, 0), op: Op::Click, owns: &["path.flip"], placement: false },
        Control { page: Some(TextPage::Path), name: "Kind Circle", find: |a| in_tab(a, "Circle"), op: Op::Click, owns: &["path.kind", "path.start", "path.flip"], placement: false },
        Control { page: Some(TextPage::Path), name: "Diameter", find: |a| a.h.in_row("Diameter", Role::SpinButton, 0), op: Op::Drag(30.0), owns: &["path.diameter"], placement: false },
        Control { page: Some(TextPage::Path), name: "Fit", find: |a| in_tab(a, "Fit"), op: Op::Click, owns: &["path.diameter", "path.center", "path.start"], placement: false },
    ]);
    list
}

#[test]
fn every_text_control_changes_exactly_its_own_field() {
    let (mut app, id) = text_app("WTE 2026", |s| s.fx.warp = Some(Warp { bend: 0.3, ..Warp::new(WarpStyle::Twist) }), None);
    for control in controls() {
        if let Some(page) = control.page {
            app.state.side_tab = SideTab::Text;
            app.state.text_page = page;
        }
        app.idle(2);
        app.take();
        let before = spec_of(&app, id);
        let placement = app.state.doc().unwrap().stamp(id).unwrap().placement;
        let undo0 = undo_len(&app);
        let node = (control.find)(&app);
        match control.op {
            Op::Click => {
                app.click_at(node.rect.center());
                assert_one_final(&app.take(), control.name);
            }
            Op::Drag(dx) => {
                drag_node(&mut app, &node, dx);
                assert_previews_then_one_final(&app.take(), control.name);
            }
        }
        app.idle(1);
        let after = spec_of(&app, id);
        let new_placement = app.state.doc().unwrap().stamp(id).unwrap().placement;
        assert_eq!(undo_len(&app), undo0 + 1, "{}: one undo entry", control.name);
        let mut changed = changed_fields(&before, &after);
        if control.placement {
            assert_ne!(placement, new_placement, "{}: the placement changed", control.name);
            assert!(changed.is_empty(), "{}: the spec changed {changed:?}", control.name);
            continue;
        }
        // The core may re-fit the placement to keep the anchor (FEATURE_TEXT 5.11): only the spec is compared here.
        changed.retain(|f| !control.owns.contains(f));
        assert!(changed.is_empty(), "{}: other fields changed: {changed:?}", control.name);
        assert_ne!(before, after, "{}: its own field changed", control.name);
    }

    // The same controls of the draft under the Text tool: the draft's field changes, nothing is queued, and the gap
    // controls and Curve are disabled.
    // Check of T6: the whole table, not only three sliders.
    app.state.tool = ToolKind::Text;
    app.take();
    for control in controls() {
        if let Some(page) = control.page {
            app.state.side_tab = SideTab::Text;
            app.state.text_page = page;
        }
        app.idle(2);
        let before = text_block::draft_spec(&app.state.tools.text);
        let (size, angle) = (app.state.tools.text.em_frac, app.state.tools.stamp.rotation_deg);
        let node = (control.find)(&app);
        match control.op {
            Op::Click => app.click_at(node.rect.center()),
            Op::Drag(dx) => drag_node(&mut app, &node, dx),
        }
        app.idle(1);
        let actions = app.take();
        assert!(actions.iter().all(|a| matches!(a, AppAction::LoadFontSamples)), "{} (draft) queued {actions:?}", control.name);
        let after = text_block::draft_spec(&app.state.tools.text);
        let mut changed = changed_fields(&before, &after);
        if control.placement {
            assert!(changed.is_empty(), "{} (draft): the spec changed {changed:?}", control.name);
            assert!(app.state.tools.text.em_frac != size || app.state.tools.stamp.rotation_deg != angle, "{} (draft) changed nothing", control.name);
            continue;
        }
        assert_ne!(before, after, "{} (draft): its own field changed", control.name);
        changed.retain(|f| !control.owns.contains(f));
        assert!(changed.is_empty(), "{} (draft): other fields changed: {changed:?}", control.name);
    }
    app.state.side_tab = SideTab::Text;
    app.state.text_page = TextPage::Character;
    app.idle(2);
    assert!(in_tab(&app, "+").disabled && in_tab(&app, "Reset all").disabled, "the gap controls are disabled for the draft");
    app.state.text_page = TextPage::Path;
    app.idle(2);
    assert!(in_tab(&app, "Curve").disabled);
    assert!(app.take().is_empty(), "the draft queues nothing");
}

// ---------------------------------------------------------------------------------------------------------------------
// Gate stage 3 (round 4): the font button rect for the self-test, and the panel timing checks of FEATURE_TEXT 8.4
// ---------------------------------------------------------------------------------------------------------------------

/// The font button's rect is recorded every frame (`PanelHit::FontButton`), and a click there opens the picker: the
/// self-test opens the picker through it (`text_fonts`).
#[test]
fn the_font_button_rect_opens_the_picker() {
    let mut state = sample_state(SampleKind::Sphere);
    state.tool = ToolKind::Text;
    let mut app = PanelApp::new(state);
    let rect = app.state.panel_rects.get(&PanelHit::FontButton).copied().expect("the font button is recorded");
    assert!(rect.contains(app.h.get("Ubuntu").rect.center()), "the recorded rect is the button");
    app.click_at(rect.center());
    app.idle(1);
    assert!(egui::Popup::is_any_open(&app.h.ctx), "the picker is open");
    app.state.tool = ToolKind::Brush;
    app.press(Key::Escape, Modifiers::NONE);
    app.idle(2);
    assert!(!app.state.panel_rects.contains_key(&PanelHit::FontButton), "no font button without the Text block");
}

/// Milliseconds of one whole panel frame (menu, status, toolbar, side panel and mode strip of `PanelApp`, plus the
/// harness's accesskit tree): an upper bound of the app's "side panel" segment.
fn timed_frame(app: &mut PanelApp, events: Vec<Event>) -> f64 {
    let t = std::time::Instant::now();
    app.frame(events);
    t.elapsed().as_secs_f64() * 1000.0
}

fn frame_stats(name: &str, v: &[f64], limit: f64) {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let max = s[s.len() - 1];
    eprintln!(
        "{name}: {} frames, median {:.2} ms, max {max:.2} ms (limit {limit} ms per frame) {}",
        s.len(),
        s[s.len() / 2],
        if max <= limit { "ok" } else { "MISSED" }
    );
}

/// FEATURE_TEXT 8.4: the Character page with a text line of 1,000 characters selected, 20 idle frames and 20 frames of
/// scrolling the strip: no frame above 16 ms. Run in release:
/// `cargo.cmd test -p wte_app --release --lib gate_character_page -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run in release on an idle machine"]
fn gate_character_page_with_1000_characters_frame_times() {
    let line: String = (0..1000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let (mut app, _) = text_app(&line, |s| s.size_px = 8.0, Some(TextPage::Character));
    app.idle(5);
    let idle: Vec<f64> = (0..20).map(|_| timed_frame(&mut app, Vec::new())).collect();
    frame_stats("Character page, 1,000 characters, idle", &idle, 16.0);
    let gaps = |app: &PanelApp| -> Vec<String> {
        app.h.nodes.iter().filter(|n| n.label.starts_with("Gap between")).map(|n| n.label.clone()).collect()
    };
    let before = gaps(&app);
    let first = app.h.nodes.iter().find(|n| n.label.starts_with("Gap between")).cloned().expect("a gap slot is drawn");
    app.hover(first.rect.center());
    let scrolling: Vec<f64> =
        (0..20).map(|_| timed_frame(&mut app, vec![Harness::wheel(vec2(-3.0, 0.0), Modifiers::NONE)])).collect();
    app.idle(1);
    frame_stats("Character page, 1,000 characters, scrolling the strip", &scrolling, 16.0);
    assert_ne!(gaps(&app), before, "the strip scrolled");
    let (boxes, slots) = app.ps.text.strip_drawn;
    eprintln!("after scrolling: {boxes} boxes, {slots} slots drawn");
}

/// FEATURE_TEXT 8.4: opening the font picker with all families of this machine (no samples: the sample job runs in the
/// app, where `text_fonts` measures the real "side panel" segment): 20 frames open, 20 frames scrolling the list.
#[test]
#[ignore = "reads the installed fonts of this machine; timing"]
fn gate_font_picker_with_the_machine_fonts_frame_times() {
    let mut state = sample_state(SampleKind::Sphere);
    state.tool = ToolKind::Text;
    let mut catalog = FontCatalog::bundled();
    catalog.load_system();
    let families = catalog.families().len();
    state.fonts = catalog;
    let mut app = PanelApp::new(state);
    let rect = app.state.panel_rects[&PanelHit::FontButton];
    app.hover(rect.center());
    let open = timed_frame(&mut app, vec![Harness::button(rect.center(), egui::PointerButton::Primary, true, Modifiers::NONE)]);
    let open2 = timed_frame(&mut app, vec![Harness::button(rect.center(), egui::PointerButton::Primary, false, Modifiers::NONE)]);
    assert!(egui::Popup::is_any_open(&app.h.ctx), "the picker is open");
    let frames: Vec<f64> = (0..20).map(|_| timed_frame(&mut app, Vec::new())).collect();
    eprintln!("font picker: {families} families; press frame {open:.2} ms, opening frame {open2:.2} ms");
    frame_stats("font picker open, idle", &frames, 16.0);
    let row = app.h.nodes.iter().rfind(|n| n.role == Role::Button && n.rect.width() > 200.0).cloned().expect("a row");
    app.hover(row.rect.center());
    let scrolling: Vec<f64> =
        (0..20).map(|_| timed_frame(&mut app, vec![Harness::wheel(vec2(0.0, -3.0), Modifiers::NONE)])).collect();
    frame_stats("font picker open, scrolling the list", &scrolling, 16.0);
}

/// objects-tabs (WP4): the side panel stays 300 pt wide when the Tool page starts or stops scrolling. The lower tab,
/// the tool and the selection change the room and the length of the Tool page; egui reserves a scroll bar's room only
/// from the frame after it first shows it, and the option grids keep their column widths from the frame before, so a
/// page that started to overflow grew the panel by 6 pt for good (the `text` and `text_tab` scenarios: 306 and 312 pt).
/// The upper pages' bars therefore lie over the content's right edge and take none of its width.
#[test]
fn the_side_panel_keeps_its_width_when_the_tool_page_starts_to_scroll() {
    let mut state = sample_state(SampleKind::Sphere);
    let id = place_text(&mut state, "WTE", |_| {});
    state.session.as_mut().unwrap().selected_stamp = Some(id);
    state.tool = ToolKind::Transform;
    let mut app = PanelApp::new(state);
    let steps = [
        (ToolKind::Transform, SideTab::Layers),
        (ToolKind::Transform, SideTab::Text),
        (ToolKind::Text, SideTab::Layers),
        (ToolKind::Text, SideTab::Text),
        (ToolKind::Brush, SideTab::Brush),
        (ToolKind::Transform, SideTab::Layers),
        (ToolKind::Stamp, SideTab::Layers),
        (ToolKind::Transform, SideTab::Text),
    ];
    for (w, h) in [(1280.0, 800.0), (1280.0, 700.0), (1600.0, 1000.0)] {
        app.h.size = vec2(w, h);
        for (tool, tab) in steps {
            app.state.tool = tool;
            app.state.side_tab = tab;
            app.idle(4);
            let side = app.state.side_panel_rect.expect("the side panel is recorded");
            assert!(side.width() <= 300.5, "{w}x{h}, {tool:?}, {tab:?}: the side panel is {} pt wide", side.width());
        }
    }
    let actions: Vec<AppAction> = app.take().into_iter().filter(|a| !matches!(a, AppAction::LoadFontSamples)).collect();
    assert!(actions.is_empty(), "{actions:?}");
}
