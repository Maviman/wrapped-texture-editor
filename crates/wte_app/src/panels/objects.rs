// objects-tabs (WP5, OBJECTS_TABS_PLAN section 7; ARCHITECTURE.md 8.7 "Objects tab"): the Objects page of the side
// panel's upper group.
//
//   32 objects · 3 hidden · 1,523,104 triangles        <- one weak line
//   (eye) Board                              12,480    <- rows, 22 pt, virtualized (`ScrollArea::show_rows`)
//   ( - ) Pawn_White_2  (weak)               48,112    <- hidden: closed eye, weak name; selected rows: selection fill
//   [ Hide ] [ Isolate ] [ Show all ] [ Frame ]        <- footer, pinned below the list
//
// Only the rows in view are laid out, so a model with thousands of objects costs nothing per frame. The page never
// changes the document: every command is an `AppAction` (8.7); it only clears `Session::object_reveal` once it has
// scrolled the requested row into view.

use eframe::egui;
use egui::{Pos2, Rect, Sense, vec2};

use crate::action::AppAction;
use crate::panels::PanelState;
use crate::panels::layers::paint_eye;
use crate::shortcuts::shortcut_text;
use crate::state::{AppState, PanelHit};

/// Height of an object row in points.
pub(crate) const ROW_HEIGHT: f32 = 22.0;
/// Space between two rows.
const ROW_GAP: f32 = 1.0;
/// Width of the eye at the left of a row.
const EYE_WIDTH: f32 = 20.0;
/// First-frame guess of the footer's height (corrected after one frame).
const FOOTER_HEIGHT_GUESS: f32 = 24.0;

/// UI state of the Objects tab.
pub(crate) struct ObjectsTabState {
    /// Height of the footer in the last frame (points): the list scrolls in the room it leaves.
    footer_height: f32,
    /// The row of the last click: egui counts two quick clicks anywhere as a double-click, but only two on the same row
    /// frame it (a quick Ctrl- or Shift-click on another row is a click).
    last_click: Option<u32>,
}

impl Default for ObjectsTabState {
    fn default() -> ObjectsTabState {
        ObjectsTabState { footer_height: FOOTER_HEIGHT_GUESS, last_click: None }
    }
}

/// What a primary click on an object row does: Ctrl toggles the object in the selection, Shift selects the range from
/// `Session::object_anchor`, a plain click selects this object only.
pub(crate) fn row_click_action(mods: egui::Modifiers, object: u32) -> AppAction {
    if mods.command {
        AppAction::ToggleObjectSelected(object)
    } else if mods.shift {
        AppAction::SelectObjectRange(object)
    } else {
        AppAction::SelectObject(object)
    }
}

/// `1,523,104`.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The header line: "32 objects · 3 hidden · 1,523,104 triangles" ("1 object", no "hidden" part when none is).
pub(crate) fn header_text(objects: usize, hidden: usize, triangles: u64) -> String {
    let noun = if objects == 1 { "object" } else { "objects" };
    let tris = if triangles == 1 { "triangle" } else { "triangles" };
    let mut text = format!("{} {noun}", thousands(objects as u64));
    if hidden > 0 {
        text.push_str(&format!(" \u{b7} {} hidden", thousands(hidden as u64)));
    }
    text.push_str(&format!(" \u{b7} {} {tris}", thousands(triangles)));
    text
}

/// One row as data (taken from the document before the list is drawn).
struct RowData<'a> {
    object: u32,
    name: &'a str,
    triangles: u32,
    hidden: bool,
    selected: bool,
}

/// Draws the Objects page into the room below the upper strip; returns that room (the visible page).
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) -> Rect {
    let page = ui.available_rect_before_wrap();
    let Some(session) = state.session.as_ref() else {
        ui.weak("No document open");
        return page;
    };
    let doc = &session.doc;
    let objects = &doc.mesh.objects;
    let count = objects.len();
    ui.add(egui::Label::new(egui::RichText::new(header_text(count, doc.hidden_object_count(), u64::from(doc.mesh.tri_count()))).weak()).truncate());

    let mut actions = Vec::new();
    let mut hits: Vec<(PanelHit, Rect)> = Vec::new();
    let mods = ui.input(|i| i.modifiers);
    let pitch = ROW_HEIGHT + ROW_GAP;
    let list_height = (ui.available_height() - ps.objects.footer_height - ui.spacing().item_spacing.y).max(ROW_HEIGHT);
    let reveal = session.object_reveal.filter(|&i| (i as usize) < count);
    let mut scroll = egui::ScrollArea::vertical().id_salt("side_objects_scroll").auto_shrink([false, false]).max_height(list_height);
    if let Some(i) = reveal {
        // The requested row in the middle of the list (clamped by the scroll area at both ends).
        scroll = scroll.vertical_scroll_offset((i as f32 * pitch - (list_height - pitch) * 0.5).max(0.0));
    }
    let list = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            scroll.show_rows(ui, ROW_HEIGHT, count, |ui, range| {
                for i in range {
                    let info = &objects[i];
                    let object = i as u32;
                    let data = RowData {
                        object,
                        name: &info.name,
                        triangles: info.tri_count,
                        hidden: doc.object_hidden(object),
                        selected: session.selected_objects.contains(&object),
                    };
                    row(ui, &data, mods, &mut ps.objects.last_click, &mut actions, &mut hits);
                }
            })
        })
        .inner;
    let view = list.inner_rect;
    hits.push((PanelHit::ObjectsList, view));
    // A click on the empty list below the rows clears the selection.
    if list.content_size.y + 1.0 < view.height() {
        let empty = Rect::from_min_max(Pos2::new(view.left(), view.top() + list.content_size.y), view.max);
        let resp = ui.interact(empty, egui::Id::unique("wte_objects_list_empty"), Sense::click());
        if resp.clicked() && !session.selected_objects.is_empty() {
            actions.push(AppAction::DeselectObjects);
        }
    }

    let footer = footer(ui, state, &mut actions, &mut hits);
    ps.objects.footer_height = footer.height().max(0.0);
    hits.push((PanelHit::ObjectsFooter, footer));

    if reveal.is_some()
        && let Some(session) = state.session.as_mut()
    {
        session.object_reveal = None;
    }
    state.actions.extend(actions);
    state.panel_rects.extend(hits);
    page
}

/// One object row: eye, name (elided to the room before the count), triangle count right-aligned.
fn row(
    ui: &mut egui::Ui,
    data: &RowData<'_>,
    mods: egui::Modifiers,
    last_click: &mut Option<u32>,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let resp = ui.interact(rect, egui::Id::unique(("wte_object_row", data.object)), Sense::click());
    // The eye is registered after the row, so it lies above it and wins the click.
    let eye_rect = Rect::from_min_size(rect.min, vec2(EYE_WIDTH, ROW_HEIGHT));
    let eye = ui.interact(eye_rect, egui::Id::unique(("wte_object_eye", data.object)), Sense::click());
    hits.push((PanelHit::ObjectRow(data.object), rect));
    hits.push((PanelHit::ObjectEye(data.object), eye_rect));

    let visuals = ui.visuals();
    if data.selected {
        ui.painter().rect_filled(rect, 3.0, visuals.selection.bg_fill.linear_multiply(0.6));
    } else if resp.hovered() || eye.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let icon = if eye.hovered() { visuals.widgets.hovered.fg_stroke.color } else { visuals.widgets.inactive.fg_stroke.color };
    paint_eye(ui.painter(), eye_rect, !data.hidden, icon);

    let weak = visuals.weak_text_color();
    let count_galley = ui.painter().layout_no_wrap(thousands(u64::from(data.triangles)), egui::FontId::proportional(12.0), weak);
    let count_pos = Pos2::new(rect.right() - 4.0 - count_galley.size().x, rect.center().y - count_galley.size().y * 0.5);
    let name_left = eye_rect.right() + 4.0;
    let name_room = (count_pos.x - 8.0 - name_left).max(8.0);
    let name_color = if data.hidden { weak } else if data.selected { visuals.strong_text_color() } else { visuals.text_color() };
    let mut job = egui::text::LayoutJob::simple_singleline(data.name.to_owned(), egui::FontId::proportional(14.0), name_color);
    job.wrap = egui::text::TextWrapping { max_width: name_room, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
    let name_galley = ui.painter().layout_job(job);
    let name_pos = Pos2::new(name_left, rect.center().y - name_galley.size().y * 0.5);
    ui.painter().galley(name_pos, name_galley, name_color);
    ui.painter().galley(count_pos, count_galley, weak);

    let tip = if data.hidden { "Show object" } else { "Hide object" };
    let eye_clicked = eye.on_hover_text(tip).clicked();
    let mut hover = format!("{}\n{} triangles", data.name, thousands(u64::from(data.triangles)));
    if data.hidden {
        hover.push_str("\nHidden");
    }
    hover.push_str("\nClick: select  |  Ctrl / Shift-click: select more  |  Double-click: frame");
    let resp = resp.on_hover_text(hover);
    if eye_clicked {
        actions.push(AppAction::SetObjectVisible(data.object, data.hidden));
        *last_click = None;
    } else if resp.clicked() {
        let plain = !mods.command && !mods.shift;
        if resp.double_clicked() && plain && *last_click == Some(data.object) {
            actions.push(AppAction::SelectObject(data.object));
            actions.push(AppAction::FrameSelection);
        } else {
            actions.push(row_click_action(mods, data.object));
        }
        *last_click = Some(data.object);
    }
}

/// "Hide the selected objects (H)".
fn with_key(ctx: &egui::Context, text: &str, label: &str) -> String {
    let key = shortcut_text(ctx, label);
    if key.is_empty() { text.to_owned() } else { format!("{text} ({key})") }
}

/// The footer buttons: Hide / Isolate / Show all / Frame, enabled as their Object menu entries. Returns the footer's
/// rect.
fn footer(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) -> Rect {
    let Some(session) = state.session.as_ref() else { return Rect::NOTHING };
    let has_selection = !session.selected_objects.is_empty();
    let any_hidden = session.doc.hidden_object_count() > 0;
    let top = ui.cursor().min.y;
    let row = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let buttons: [(&str, &str, &str, bool, &str, PanelHit, AppAction); 4] = [
            ("Hide", "Hide the selected objects", "Hide Selected", has_selection, "Select objects first", PanelHit::ObjectsHide, AppAction::HideSelectedObjects),
            (
                "Isolate",
                "Show only the selected objects",
                "Isolate Selected",
                has_selection,
                "Select objects first",
                PanelHit::ObjectsIsolate,
                AppAction::IsolateSelectedObjects,
            ),
            ("Show all", "Show every object", "Show All", any_hidden, "No object is hidden", PanelHit::ObjectsShowAll, AppAction::ShowAllObjects),
            (
                "Frame",
                "Frame the selected objects in the 3D view (the visible model when none is selected)",
                "Frame Selection",
                true,
                "",
                PanelHit::ObjectsFrame,
                AppAction::FrameSelection,
            ),
        ];
        for (label, text, shortcut, enabled, why, hit, action) in buttons {
            let resp = ui.add_enabled(enabled, egui::Button::new(label));
            hits.push((hit, resp.rect));
            if resp.on_hover_text(with_key(ui.ctx(), text, shortcut)).on_disabled_hover_text(why).clicked() {
                actions.push(action);
            }
        }
    });
    Rect::from_min_max(Pos2::new(row.response.rect.left(), top), row.response.rect.max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_their_actions() {
        let ctrl = egui::Modifiers::COMMAND;
        assert!(matches!(row_click_action(egui::Modifiers::NONE, 3), AppAction::SelectObject(3)));
        assert!(matches!(row_click_action(ctrl, 3), AppAction::ToggleObjectSelected(3)));
        assert!(matches!(row_click_action(egui::Modifiers::SHIFT, 3), AppAction::SelectObjectRange(3)));
        assert!(matches!(row_click_action(ctrl | egui::Modifiers::SHIFT, 3), AppAction::ToggleObjectSelected(3)), "Ctrl wins");
    }

    /// Plan 5.3 with WP4: the selection tint is drawn with a selection, no open stroke, and the Navigate tool or the
    /// Objects tab shown.
    #[test]
    fn the_selection_tint_shows_with_navigate_or_the_objects_tab() {
        use crate::state::{ToolKind, UpperTab};
        use crate::view::view3d::highlight_shown;
        let mut state = crate::panels::tests::sample_state(wte_core::SampleKind::Objects);
        state.tool = ToolKind::Brush;
        assert!(!highlight_shown(&state), "no selection");
        state.session.as_mut().unwrap().selected_objects = [1].into();
        assert!(!highlight_shown(&state), "the Brush with the Tool tab: the model stays clean");
        state.upper_tab = UpperTab::Objects;
        assert!(highlight_shown(&state), "the Objects tab shows the selection");
        state.upper_tab = UpperTab::Color;
        state.tool = ToolKind::Navigate;
        assert!(highlight_shown(&state), "the Navigate tool shows the selection");
    }

    #[test]
    fn header_and_numbers() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_523_104), "1,523,104");
        assert_eq!(thousands(12_480), "12,480");
        assert_eq!(header_text(32, 3, 1_523_104), "32 objects \u{b7} 3 hidden \u{b7} 1,523,104 triangles");
        assert_eq!(header_text(1, 0, 12), "1 object \u{b7} 12 triangles");
        assert_eq!(header_text(2_000, 1, 1), "2,000 objects \u{b7} 1 hidden \u{b7} 1 triangle");
    }
}
