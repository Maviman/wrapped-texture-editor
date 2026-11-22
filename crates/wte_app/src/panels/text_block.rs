// Owner: T6 text panels (round 4). Written by the scaffold S2, filled by T6.
//
// The Text block of the tool options (FEATURE_TEXT 6.1, 6.2): `target` decides which text the rows edit (the tool
// decides: the Text tool always edits the text to place, the Transform tool the selected text), `show` draws them.
// `tool_options.rs` calls only these two functions, with fixed signatures (ROUND4_PLAN 2.2 item 12).
//
// The block is four rows (TEXT 6.1): the content field; the font button (picker, `font_picker.rs`) and the style list;
// B, I, Text size, Angle and the colour button; the buttons Warp, Path and More that open the Text tab
// (`text_tab.rs`). Every other text control lives in the Text tab.
//
// Both targets are edited through one representation, a `TextSpec`: for the text to place it is built from
// `state.tools.text` (`draft_spec`) and written back (`write_draft`), no action is queued; for a placed text the new spec
// goes out as `UpdateStampSource` through `continuous_edit` (`Commit::Preview` while dragged or typed, one `Final`).
//
// Fonts (TEXT 5.9, 5.13): `candidates` lists the faces of a family from the catalog and from the open project,
// `resolve_face` picks the face for B / I and for a family change (the real face first; thicken or slant only what
// is missing), `resolve_font` turns a family and style into a `FontRef` (the project first, then the machine, then
// the bundled font with a toast).
//
// `GapBuffer` is the `egui::TextBuffer` of the content field of a placed text (TEXT 5.12): every edit moves the gap
// values with their characters, and a history of (text, gaps) states lets the field's own undo and redo bring gap
// values back. The scaffold's two egui proofs (`control_sweep::text_panel_tests`) show that egui reports every edit
// with exact character positions and the undo / redo as `replace_with` with a text the buffer held before.

use std::collections::HashMap;
use std::ops::Range;

use eframe::egui;
use egui::color_picker::{Alpha, color_edit_button_srgba};
use egui::text::CharIndex;
use egui::{DragValue, TextEdit, vec2};
use wte_core::text::{EmMap, face_traits, gaps};
use wte_core::{
    BlobId, Commit, Document, FAUX_BOLD_EM, FAUX_SLANT_DEG, FaceTraits, FontCatalog, FontRef, Gap, Outline, Placement,
    Projector, StampId, StampSource, TextSpec, WarpStyle,
};

use crate::action::AppAction;
use crate::panels::tool_options::{placement_rotation_deg, rotate_placement_to};
use crate::panels::{PanelState, continuous_edit, font_picker, from_color32, to_color32};
use crate::state::{AppState, PanelHit, SideTab, TextGapSel, TextPage, TextSettings, ToolKind};

/// Height of one control row of the block (TEXT 6.1).
const ROW_HEIGHT: f32 = 20.0;
/// Widths of row 3: B, I, Text size, Angle, colour (TEXT 6.1; 216 pt with the gaps).
const BI_WIDTH: f32 = 22.0;
const SIZE_WIDTH: f32 = 64.0;
const ANGLE_WIDTH: f32 = 56.0;
/// Widths of row 4: Warp, Path, More (264 pt with the gaps).
const WARP_WIDTH: f32 = 100.0;
const PATH_WIDTH: f32 = 104.0;
const MORE_WIDTH: f32 = 48.0;
/// Gap between the controls of a row.
const ROW_GAP: f32 = 6.0;
/// Limits of the Text size field, in % (TEXT 1.2).
const SIZE_MIN_PCT: f32 = 0.2;
const SIZE_MAX_PCT: f32 = 400.0;
/// The `BlobId` of the stand-in font of the text to place: never a blob of a document (the draft keeps names only).
const DRAFT_BLOB: BlobId = BlobId(u32::MAX);
/// Most states of the content field's history (TEXT 5.12; egui keeps at most 100 undo points).
const HISTORY_MAX: usize = 200;
/// Traits used when a face cannot be read (a regular upright face of normal width).
const PLAIN_TRAITS: FaceTraits = FaceTraits { weight: 400, italic: false, stretch: 5 };

/// Which text the Text block and the Text tab edit (FEATURE_TEXT 6.2, the target table).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TextTarget {
    /// The text to place (`state.tools.text`, the angle `state.tools.stamp.rotation_deg`); no action is queued.
    Draft,
    /// The selected text stamp: `UpdateStampSource` / `UpdateStampPlacement` through `continuous_edit`.
    Stamp(StampId),
}

/// The target of the Text block: the Text tool always edits the draft (also while a text is selected); the Transform
/// tool edits the selected stamp when it is a text; every other case has none (the block is not shown and the Text
/// tab is not enabled, ROUND4_PLAN 2.2 item 10).
pub fn target(state: &AppState) -> Option<TextTarget> {
    match state.tool {
        ToolKind::Text => Some(TextTarget::Draft),
        // By reference: called several times per frame (tool options, tab rule, shown tab).
        ToolKind::Transform => {
            let session = state.session.as_ref()?;
            let id = session.selected_stamp?;
            matches!(session.doc.stamp(id)?.source, StampSource::Text(_)).then_some(TextTarget::Stamp(id))
        }
        _ => None,
    }
}

/// Draws the Text block for `target` at the top of the tool options. Actions for a placed text go to `actions`.
pub fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, actions: &mut Vec<AppAction>) {
    // Round 5 R5-3: the block is a segment of the frame profile (the font picker's first open is measured inside it).
    let _p = crate::app::frame_profile::scope("text block");
    let Some(spec) = current_spec(state, target) else { return };
    if crate::panels::shown_tab(state) != SideTab::Text {
        crate::panels::text_tab::clear_gap_status(state);
    }
    match target {
        TextTarget::Draft => {
            rows(ui, state, ps, target, &spec, actions);
            // Round 6 (R6-2): Project | Wrap and the two Wrap options, next to "Project through" (shared with the
            // Stamp tool).
            crate::panels::tool_options::tool_surface_options(
                ui,
                &mut state.tools.stamp,
                &mut state.tool_probe.wrap_controls,
                "Also print on surfaces behind the first one (the far side of the model). Shared with the Stamp tool",
            );
            help(ui, "Click on the model or the texture to place the text. It becomes its own layer.");
        }
        TextTarget::Stamp(id) => {
            note_outside_changes(ui, state, ps, id, &spec);
            let locked = state.doc().is_some_and(|d| d.stamp_locked(id));
            ui.add_enabled_ui(!locked, |ui| rows(ui, state, ps, target, &spec, actions));
            ui.separator();
        }
    }
}

/// The four rows of TEXT 6.1.
fn rows(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    content_row(ui, state, ps, target, spec, actions);
    font_row(ui, state, ps, target, spec, actions);
    style_row(ui, state, ps, target, spec, actions);
    page_row(ui, state, spec);
}

// ---- the text both targets are edited through ----

/// The spec the controls of `target` show: the selected text stamp's, or one built from the draft settings.
pub(crate) fn current_spec(state: &AppState, target: TextTarget) -> Option<TextSpec> {
    match target {
        TextTarget::Draft => Some(draft_spec(&state.tools.text)),
        TextTarget::Stamp(id) => match &state.doc()?.stamp(id)?.source {
            StampSource::Text(spec) => Some(spec.clone()),
            StampSource::Image { .. } => None,
        },
    }
}

/// The text to place as a `TextSpec` (its font carries the two names only; the blob is a stand-in).
pub(crate) fn draft_spec(t: &TextSettings) -> TextSpec {
    let font = FontRef { blob: DRAFT_BLOB, face_index: 0, family: t.family.clone(), style: t.style.clone() };
    let mut spec = TextSpec::new(&t.content, font);
    spec.color = t.color;
    spec.align = t.align;
    spec.line_spacing = t.line_spacing;
    spec.letter_spacing = t.letter_spacing;
    spec.outline = t.outline.then_some(Outline { width_em: t.outline_width_em, color: t.outline_color });
    spec.fx.bold_em = t.bold_em;
    spec.fx.slant_deg = t.slant_deg;
    spec.fx.warp = t.warp;
    spec.fx.path.clone_from(&t.path);
    spec
}

/// Writes a changed draft spec back into the settings of the Text tool (gap values do not exist before placing).
pub(crate) fn write_draft(t: &mut TextSettings, spec: &TextSpec) {
    t.content.clone_from(&spec.content);
    t.family.clone_from(&spec.font.family);
    t.style.clone_from(&spec.font.style);
    t.color = spec.color;
    t.align = spec.align;
    t.line_spacing = spec.line_spacing;
    t.letter_spacing = spec.letter_spacing;
    t.outline = spec.outline.is_some();
    if let Some(outline) = spec.outline {
        t.outline_width_em = outline.width_em;
        t.outline_color = outline.color;
    }
    t.bold_em = spec.fx.bold_em;
    t.slant_deg = spec.fx.slant_deg;
    t.warp = spec.fx.warp;
    t.path.clone_from(&spec.fx.path);
}

/// A continuous control (slider, drag value, colour popup) changed the text to `new`: the draft takes it at once; a
/// placed text gets `Preview`s while it is dragged and one `Final` (always called, so an ending drag is finished).
#[allow(clippy::too_many_arguments, reason = "the spec before and after, the target and the control's response")]
pub(crate) fn live_edit(
    state: &mut AppState,
    ps: &mut PanelState,
    ui: &egui::Ui,
    resp: &egui::Response,
    target: TextTarget,
    old: &TextSpec,
    new: &TextSpec,
    actions: &mut Vec<AppAction>,
) {
    match target {
        TextTarget::Draft => {
            if new != old {
                write_draft(&mut state.tools.text, new);
            }
        }
        TextTarget::Stamp(id) => {
            value_edit(ps, actions, ui, resp, new != old, |commit| AppAction::UpdateStampSource(id, StampSource::Text(new.clone()), commit))
        }
    }
}

/// `continuous_edit` with the change decided by the value itself (`changed`) instead of `Response::changed`: a drag
/// value that ends a typed edit with Enter reports `changed` in the frame it loses the focus and once more in the next,
/// which would send a second `Final` of the same value (one more undo entry).
pub(crate) fn value_edit(
    ps: &mut PanelState,
    out: &mut Vec<AppAction>,
    ui: &egui::Ui,
    resp: &egui::Response,
    changed: bool,
    make: impl Fn(Commit) -> AppAction,
) {
    let ending = resp.drag_stopped() || resp.lost_focus();
    if ending && changed {
        out.push(make(Commit::Final));
        ps.owed_final = None;
    } else if ending && let Some(owed) = ps.owed_final.take() {
        out.push(owed);
    } else if changed {
        let live = resp.dragged()
            || resp.has_focus()
            || resp.is_pointer_button_down_on()
            || ui.input(|i| i.pointer.any_down())
            || ui.ctx().text_edit_focused();
        if live {
            out.push(make(Commit::Preview));
            ps.owed_final = Some(make(Commit::Final));
            ps.preview_sent = true;
        } else {
            out.push(make(Commit::Final));
            ps.owed_final = None;
        }
    }
}

/// A one-shot change (a click on a button, a pick from a list): one `Final` for a placed text.
pub(crate) fn click_edit(state: &mut AppState, target: TextTarget, old: &TextSpec, new: &TextSpec, actions: &mut Vec<AppAction>) {
    if new == old {
        return;
    }
    match target {
        TextTarget::Draft => write_draft(&mut state.tools.text, new),
        TextTarget::Stamp(id) => actions.push(AppAction::UpdateStampSource(id, StampSource::Text(new.clone()), Commit::Final)),
    }
}

// ---- row 1: the content field ----

fn content_row(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    match target {
        TextTarget::Draft => {
            let focus = std::mem::take(&mut state.focus_text_content);
            let resp = ui.add(
                TextEdit::multiline(&mut state.tools.text.content)
                    .id_salt("text_tool_content")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type the text to place"),
            );
            if focus {
                resp.request_focus();
            }
        }
        TextTarget::Stamp(id) => stamp_content_field(ui, state, ps, id, spec, actions),
    }
}

/// The id of the content field of placed text `id`.
pub(crate) fn content_field_id(id: StampId) -> egui::Id {
    egui::Id::unique(("stamp_text", id.0))
}

/// The content field of a placed text: a `GapBuffer`, so the gap values move with their characters (TEXT 5.12). While
/// it has the focus and its cursor stands between two characters of a line, that gap is selected (TEXT 6.2 rule 2).
fn stamp_content_field(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, id: StampId, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    let field = content_field_id(id);
    let focused = ui.memory(|m| m.has_focus(field));
    let fresh = match &ps.text.buffer {
        Some((sid, buf)) if *sid == id => !focused && (buf.text != spec.content || buf.gaps != spec.fx.gaps),
        _ => true,
    };
    if fresh {
        ps.text.buffer = Some((id, GapBuffer::new(&spec.content, &spec.fx.gaps)));
    }
    let Some((_, mut buf)) = ps.text.buffer.take() else { return };
    let out = TextEdit::multiline(&mut buf).id(field).desired_rows(2).desired_width(f32::INFINITY).show(ui);
    let resp = out.response;
    state.panel_rects.insert(PanelHit::StampTextField, resp.rect);
    if resp.has_focus() || resp.lost_focus() {
        let mut new = spec.clone();
        new.content.clone_from(&buf.text);
        new.fx.gaps.clone_from(&buf.gaps);
        continuous_edit(ps, actions, ui, &resp, |commit| AppAction::UpdateStampSource(id, StampSource::Text(new.clone()), commit));
        ps.text.seen = Some((id, buf.text.clone()));
    }
    if resp.has_focus()
        && let Some(range) = out.cursor_range
    {
        let r = range.as_sorted_char_range();
        if r.start == r.end && r.start.0 > 0 {
            select_gap_at_cursor(state, ps, id, &buf.text, &buf.gaps, r.start.0);
        }
    }
    if resp.lost_focus() {
        buf.forget_history();
    }
    ps.text.buffer = Some((id, buf));
}

/// The cursor at character `cursor` of the field: the gap after `cursor - 1` is selected when it is a gap between two
/// characters of one line; the strip shows the line of the cursor.
fn select_gap_at_cursor(state: &mut AppState, ps: &mut PanelState, id: StampId, text: &str, gaps_list: &[Gap], cursor: usize) {
    let after = (cursor - 1) as u32;
    let lines = gaps::slots(text, gaps_list);
    let Some(line) = lines.iter().position(|l| l.iter().any(|s| s.after == after && s.right.is_some())) else { return };
    if let Some(session) = state.session.as_mut() {
        session.text_gap = Some(TextGapSel { stamp: id, after });
    }
    ps.text.strip_line = line;
    ps.text.scroll_to_gap = true;
}

/// Drops the selected gap when the content of text `id` changed from outside the field (undo, redo) and remembers the
/// content the panel saw (TEXT 6.2 rule 2).
fn note_outside_changes(ui: &egui::Ui, state: &mut AppState, ps: &mut PanelState, id: StampId, spec: &TextSpec) {
    let changed = matches!(&ps.text.seen, Some((sid, content)) if *sid == id && *content != spec.content);
    let focused = ui.memory(|m| m.has_focus(content_field_id(id)));
    if changed
        && !focused
        && let Some(session) = state.session.as_mut()
        && session.text_gap.is_some_and(|g| g.stamp == id)
    {
        session.text_gap = None;
    }
    ps.text.seen = Some((id, spec.content.clone()));
}

// ---- row 2: font button and style list ----

fn font_row(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    let family = spec.font.family.clone();
    let style = spec.font.style.clone();
    let installed = state.fonts_loading || ps.text.font_installed(&state.fonts, &family, &style);
    let in_project = state.doc().is_some_and(|d| d.find_font(&family, &style).is_some());
    let mut picked: Option<String> = None;
    let mut style_pick: Option<usize> = None;
    let faces = candidates(&state.fonts, state.doc(), &mut ps.text.face_traits, &family);
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        let spacing = ui.spacing().item_spacing.x;
        let width = ui.available_width();
        let font_width = ((width - spacing) * 0.6).floor().max(60.0);
        let style_width = (width - spacing - font_width).max(50.0);
        let (warn, why) = match (installed, in_project, target) {
            (true, _, _) => (false, String::new()),
            (false, true, _) => (false, "Embedded in the project, not installed here".to_owned()),
            (false, false, TextTarget::Draft) => (true, "Font not installed: Ubuntu Light will be used".to_owned()),
            (false, false, TextTarget::Stamp(_)) => (true, "Font not installed and not in this project".to_owned()),
        };
        picked = font_picker::button(ui, state, ps, &family, font_width, warn, &why);
        // The style list: every face of the family, from the machine and from the project (TEXT 5.9 `candidates`).
        let combo = egui::ComboBox::from_id_salt(("text_font_style", target_salt(target)))
            .selected_text(style.as_str())
            // A combo box adds its arrow and padding to this width (as in round 3).
            .width(style_width - 22.0)
            .show_ui(ui, |ui| {
                if faces.is_empty() {
                    ui.weak(if state.fonts_loading { "Loading system fonts..." } else { "Not installed" });
                }
                for (i, face) in faces.iter().enumerate() {
                    let current = face.style == style;
                    if ui.selectable_label(current, face.style.as_str()).clicked() && !current {
                        style_pick = Some(i);
                    }
                }
            });
        let _ = combo.response.on_hover_text("Style: every face of the family on this machine and in this project");
    });
    if let Some(family) = picked {
        change_family(state, ps, target, spec, &family, actions);
    } else if let Some(i) = style_pick {
        let face = faces[i].clone();
        let mut new = spec.clone();
        // A pick in the style list is taken as it is; only an amount the face has itself is cleaned up (TEXT 5.9).
        if face.traits.weight >= 600 {
            new.fx.bold_em = 0.0;
        }
        if face.traits.italic {
            new.fx.slant_deg = 0.0;
        }
        apply_face(state, target, &mut new, &face);
        click_edit(state, target, spec, &new, actions);
    }
}

fn target_salt(target: TextTarget) -> u64 {
    match target {
        TextTarget::Draft => u64::MAX,
        TextTarget::Stamp(id) => id.0,
    }
}

/// A family taken in the font picker: the face is resolved again from what the text was (bold or not, italic or not),
/// so B and I keep their meaning and a real bold face replaces thickening where the new family has one (TEXT 5.9).
pub(crate) fn change_family(state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, family: &str, actions: &mut Vec<AppAction>) {
    let current = current_traits(state, ps, spec);
    let (bold, italic) = (is_bold(&current, spec.fx.bold_em), is_italic(&current, spec.fx.slant_deg));
    let faces = candidates(&state.fonts, state.doc(), &mut ps.text.face_traits, family);
    let mut new = spec.clone();
    match resolve_face(&faces, &current, bold, italic, spec.fx.bold_em, spec.fx.slant_deg) {
        Some((i, bold_em, slant_deg)) => {
            new.fx.bold_em = bold_em;
            new.fx.slant_deg = slant_deg;
            apply_face(state, target, &mut new, &faces[i]);
        }
        None => {
            // A family nobody has: the names are kept (the draft is placed with the bundled font and a warning).
            new.font.family = family.to_owned();
            new.font.style = pick_style(&state.fonts, family, &spec.font.style);
            if let TextTarget::Stamp(_) = target
                && let Some(font) = resolve_font(state, family, &new.font.style)
            {
                new.font = font;
            }
        }
    }
    click_edit(state, target, spec, &new, actions);
}

/// Puts the face `face` into `spec`: the names for the draft, the `FontRef` for a placed text.
fn apply_face(state: &mut AppState, target: TextTarget, spec: &mut TextSpec, face: &FaceChoice) {
    match target {
        TextTarget::Draft => {
            spec.font.family.clone_from(&face.family);
            spec.font.style.clone_from(&face.style);
        }
        TextTarget::Stamp(_) => {
            let font = match &face.source {
                FaceSource::Project(font) => Some(font.clone()),
                FaceSource::Catalog(_) => resolve_font(state, &face.family, &face.style),
            };
            if let Some(font) = font {
                spec.font = font;
            }
        }
    }
}

// ---- row 3: B, I, Text size, Angle, colour ----

fn style_row(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    let current = current_traits(state, ps, spec);
    let bold = is_bold(&current, spec.fx.bold_em);
    let italic = is_italic(&current, spec.fx.slant_deg);
    let faces = candidates(&state.fonts, state.doc(), &mut ps.text.face_traits, &spec.font.family);
    let family = spec.font.family.clone();
    let mut new_bold: Option<TextSpec> = None;
    let mut new_italic: Option<TextSpec> = None;
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.spacing_mut().item_spacing.x = ROW_GAP;
        let b = ui
            .add_sized([BI_WIDTH, ROW_HEIGHT], egui::Button::selectable(bold, egui::RichText::new("B").strong()))
            .on_hover_text(bold_hover(&faces, &current, spec, &family));
        if b.clicked() {
            new_bold = Some(toggle_style(state, target, spec, &faces, &current, !bold, italic, true));
        }
        let i = ui
            .add_sized([BI_WIDTH, ROW_HEIGHT], egui::Button::selectable(italic, egui::RichText::new("I").italics()))
            .on_hover_text(italic_hover(&faces, spec, &family));
        if i.clicked() {
            new_italic = Some(toggle_style(state, target, spec, &faces, &current, bold, !italic, false));
        }
        size_field(ui, state, ps, target, actions);
        angle_field(ui, state, ps, target, actions);
        let mut c = to_color32(spec.color);
        let resp = color_edit_button_srgba(ui, &mut c, Alpha::OnlyBlend).on_hover_text("Text colour");
        let mut new = spec.clone();
        new.color = from_color32(c);
        live_edit(state, ps, ui, &resp, target, spec, &new, actions);
    });
    if let Some(new) = new_bold.or(new_italic) {
        click_edit(state, target, spec, &new, actions);
    }
}

/// B or I clicked: the face and the two amounts for the wished bold / italic (TEXT 5.9 moment 1). Without any
/// candidate (a family nobody has) only the amounts change.
#[allow(clippy::too_many_arguments, reason = "the wishes and the button that was clicked")]
fn toggle_style(
    state: &mut AppState,
    target: TextTarget,
    spec: &TextSpec,
    faces: &[FaceChoice],
    current: &FaceTraits,
    want_bold: bool,
    want_italic: bool,
    bold_button: bool,
) -> TextSpec {
    let mut new = spec.clone();
    match resolve_face(faces, current, want_bold, want_italic, spec.fx.bold_em, spec.fx.slant_deg) {
        Some((i, bold_em, slant_deg)) => {
            new.fx.bold_em = bold_em;
            new.fx.slant_deg = slant_deg;
            let same = faces[i].style == spec.font.style && faces[i].family.eq_ignore_ascii_case(&spec.font.family);
            if !same {
                apply_face(state, target, &mut new, &faces[i]);
            }
        }
        None if bold_button => new.fx.bold_em = if want_bold { amount_or(spec.fx.bold_em, FAUX_BOLD_EM) } else { 0.0 },
        None => new.fx.slant_deg = if want_italic { amount_or(spec.fx.slant_deg, FAUX_SLANT_DEG) } else { 0.0 },
    }
    new
}

/// The spec a click on B (`bold`) or on I would make for `target`, without drawing the panel (the self-test scenario
/// `text_fonts` uses it, since scenarios send actions and do not click into the panel). `None` without a text.
pub(crate) fn style_click(state: &mut AppState, target: TextTarget, bold: bool) -> Option<TextSpec> {
    let spec = current_spec(state, target)?;
    let mut cache = HashMap::new();
    let current = match state.doc() {
        Some(doc) if spec.font.blob != DRAFT_BLOB => project_traits(&mut cache, doc, &spec.font),
        _ => None,
    };
    let faces = candidates(&state.fonts, state.doc(), &mut cache, &spec.font.family);
    let current = current.or_else(|| faces.iter().find(|f| f.style == spec.font.style).map(|f| f.traits)).unwrap_or(PLAIN_TRAITS);
    let (is_b, is_i) = (is_bold(&current, spec.fx.bold_em), is_italic(&current, spec.fx.slant_deg));
    let (want_bold, want_italic) = if bold { (!is_b, is_i) } else { (is_b, !is_i) };
    Some(toggle_style(state, target, &spec, &faces, &current, want_bold, want_italic, bold))
}

fn amount_or(old: f32, default: f32) -> f32 {
    if old != 0.0 { old } else { default }
}

fn bold_hover(faces: &[FaceChoice], current: &FaceTraits, spec: &TextSpec, family: &str) -> String {
    if current.weight >= 600 {
        if !faces.iter().any(|f| f.traits.weight <= 500) {
            return format!("No regular face of {family} is installed or in this project");
        }
        return format!("Bold ({family} {})", spec.font.style);
    }
    match faces.iter().find(|f| f.traits.weight >= 600) {
        Some(face) => format!("Bold ({} {})", face.family, face.style),
        None => format!("Bold (thickened: {family} has no bold face)"),
    }
}

fn italic_hover(faces: &[FaceChoice], spec: &TextSpec, family: &str) -> String {
    match faces.iter().find(|f| f.traits.italic) {
        Some(face) if face.style == spec.font.style => format!("Italic ({family} {})", face.style),
        Some(face) => format!("Italic ({} {})", face.family, face.style),
        None => format!("Italic (slanted: {family} has no italic face)"),
    }
}

/// Text size: the height of one em on the model (TEXT 6.2). For the draft it is `em_frac`; for a placed text
/// `em_px * picture height on the model / frame height`, in % of the model diagonal (2D: of the texture height), and a
/// change scales the placement about the anchor of the text.
fn size_field(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, actions: &mut Vec<AppAction>) {
    let hover = "Text size: height of one text line on the model, % of the model size (2D: of the texture height). Ctrl+wheel";
    match target {
        TextTarget::Draft => {
            let mut pct = state.tools.text.em_frac * 100.0;
            let resp = ui
                .add_sized([SIZE_WIDTH, ROW_HEIGHT], size_value(&mut pct))
                .on_hover_text(hover);
            if resp.changed() {
                state.tools.text.em_frac = pct.clamp(SIZE_MIN_PCT, SIZE_MAX_PCT) / 100.0;
            }
        }
        TextTarget::Stamp(id) => {
            let Some((placement, frame)) = stamp_frame(state, id) else {
                ui.add_enabled(false, egui::Button::new("-").min_size(vec2(SIZE_WIDTH, ROW_HEIGHT)));
                return;
            };
            let Some(base) = em_size_base(state, &placement) else { return };
            let em = placement_em_size(&placement, &frame);
            let pct = em / base * 100.0;
            let mut new_pct = pct;
            let resp = ui.add_sized([SIZE_WIDTH, ROW_HEIGHT], size_value(&mut new_pct)).on_hover_text(hover);
            // Only a value the user changed is clamped: a text scaled past the range elsewhere (gizmo, Size) is shown as
            // it is, and showing it must not resize it.
            let new_pct = if new_pct != pct { new_pct.clamp(SIZE_MIN_PCT, SIZE_MAX_PCT) } else { pct };
            let scaled = if pct > 0.0 && new_pct != pct { scale_about_anchor(&placement, &frame, new_pct / pct) } else { placement };
            let scaled = refit_auto_depth(state.doc(), &placement, scaled);
            value_edit(ps, actions, ui, &resp, scaled != placement, |commit| AppAction::UpdateStampPlacement(id, scaled, commit));
        }
    }
}

fn size_value(pct: &mut f32) -> DragValue<'_> {
    DragValue::new(pct).range(SIZE_MIN_PCT..=SIZE_MAX_PCT).clamp_existing_to_range(false).speed(0.1).max_decimals(1).suffix(" %")
}

/// The placement and the text frame of placed text `id`.
pub(crate) fn stamp_frame(state: &mut AppState, id: StampId) -> Option<(Placement, wte_core::TextFrame)> {
    let doc = state.doc_mut()?;
    let placement = doc.stamp(id)?.placement;
    let frame = doc.stamp_text_frame(id).ok()?;
    Some((placement, frame))
}

/// What Text size is a share of: the model diagonal (3D) or the height of the texture of a UV placement.
fn em_size_base(state: &AppState, placement: &Placement) -> Option<f32> {
    let doc = state.doc()?;
    let base = match placement {
        Placement::Projected(_) => doc.mesh.bounds.diagonal(),
        Placement::Uv(uv) => doc.set(uv.set).map_or(1.0, |s| s.height as f32),
    };
    (base > 0.0).then_some(base)
}

/// The height of one em of a placed text on the model (world units) or the texture (texels).
pub(crate) fn placement_em_size(placement: &Placement, frame: &wte_core::TextFrame) -> f32 {
    let height = match placement {
        Placement::Projected(p) => 2.0 * p.half_size.y,
        Placement::Uv(uv) => uv.size_px.y,
    };
    frame.em_px * height / (frame.size[1].max(1) as f32)
}

/// The placement scaled by `factor` about the anchor of the text (TEXT 6.2 "Text size").
pub(crate) fn scale_about_anchor(placement: &Placement, frame: &wte_core::TextFrame, factor: f32) -> Placement {
    let map = EmMap::new(placement, frame);
    match placement {
        Placement::Projected(p) => {
            let anchor = map.to_world([0.0, 0.0]);
            Placement::Projected(Projector { origin: anchor + (p.origin - anchor) * factor, half_size: p.half_size * factor, ..*p })
        }
        Placement::Uv(uv) => {
            let anchor = map.to_texel([0.0, 0.0]);
            let mut uv = *uv;
            uv.center_px = anchor + (uv.center_px - anchor) * factor;
            uv.size_px *= factor;
            Placement::Uv(uv)
        }
    }
}

/// `new` with its depth range re-fitted when the depth of `old` is the automatic one (as the placement fields do).
fn refit_auto_depth(doc: Option<&Document>, old: &Placement, new: Placement) -> Placement {
    match (new, old, doc) {
        (Placement::Projected(p), Placement::Projected(o), Some(doc)) if new != *old && doc.stamp_depth_is_auto(o) => {
            Placement::Projected(doc.fit_stamp_depth(&p))
        }
        _ => new,
    }
}

/// An angle in `-180..=180` degrees (wrapped, not clamped: 270 is -90).
pub(crate) fn wrap_deg(deg: f32) -> f32 {
    if !deg.is_finite() {
        return 0.0;
    }
    let w = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if w == -180.0 && deg > 0.0 { 180.0 } else { w }
}

/// Angle ("angled", TEXT 1.2): the rotation of the whole text. The draft's is the shared placement rotation of the
/// Text and the Stamp tool; a placed text's is its placement rotation (one `UpdateStampPlacement`).
fn angle_field(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, actions: &mut Vec<AppAction>) {
    let hover = "Angle: turns the whole text. Shift+wheel, or the round knob of the gizmo";
    match target {
        TextTarget::Draft => {
            let mut deg = state.tools.stamp.rotation_deg;
            let resp = ui.add_sized([ANGLE_WIDTH, ROW_HEIGHT], angle_value(&mut deg)).on_hover_text(hover);
            if resp.changed() {
                state.tools.stamp.rotation_deg = wrap_deg(deg);
            }
        }
        TextTarget::Stamp(id) => {
            let Some(placement) = state.doc().and_then(|d| d.stamp(id)).map(|s| s.placement) else { return };
            let deg = placement_rotation_deg(&placement);
            let mut new_deg = deg;
            let resp = ui.add_sized([ANGLE_WIDTH, ROW_HEIGHT], angle_value(&mut new_deg)).on_hover_text(hover);
            // Only a value the user changed turns the text, and only by a real turn: a stored angle outside the range
            // (a UV rotation is kept verbatim) or at the 180-degree seam is shown as it is and never re-written.
            let turned = if new_deg != deg && wrap_deg(new_deg - deg).abs() > 1e-4 {
                rotate_placement_to(&placement, wrap_deg(new_deg))
            } else {
                placement
            };
            let turned = refit_auto_depth(state.doc(), &placement, turned);
            value_edit(ps, actions, ui, &resp, turned != placement, |commit| AppAction::UpdateStampPlacement(id, turned, commit));
        }
    }
}

fn angle_value(deg: &mut f32) -> DragValue<'_> {
    DragValue::new(deg).speed(0.5).max_decimals(1).suffix(" °")
}

// ---- row 4: Warp, Path, More ----

/// The label of the Warp button: "Warp: Arc".
pub(crate) fn warp_label(spec: &TextSpec) -> String {
    format!("Warp: {}", spec.fx.warp.map_or("None", |w| w.style.label()))
}

/// The label of the Path button: "Path: Circle".
pub(crate) fn path_label(spec: &TextSpec) -> String {
    let kind = match spec.fx.path.as_ref().map(|p| &p.shape) {
        None => "Straight",
        Some(wte_core::PathShape::Arc { .. }) => "Arc",
        Some(wte_core::PathShape::Circle { .. }) => "Circle",
        Some(wte_core::PathShape::Curve { .. }) => "Curve",
    };
    format!("Path: {kind}")
}

fn page_row(ui: &mut egui::Ui, state: &mut AppState, spec: &TextSpec) {
    let showing = crate::panels::shown_tab(state) == SideTab::Text;
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.spacing_mut().item_spacing.x = ROW_GAP;
        // 264 pt in a column of 265 pt; narrower when the column is (the tool options indent and scroll bar).
        let scale = ((ui.available_width() - 2.0 * ROW_GAP) / (WARP_WIDTH + PATH_WIDTH + MORE_WIDTH)).min(1.0);
        let pages = [
            (TextPage::Warp, warp_label(spec), WARP_WIDTH, "Opens the Warp page of the Text tab"),
            (TextPage::Path, path_label(spec), PATH_WIDTH, "Opens the Path page of the Text tab"),
            (TextPage::Character, "More".to_owned(), MORE_WIDTH, "Opens the Character page of the Text tab: spacing, gaps, thicken, slant, outline"),
        ];
        for (page, label, width, hover) in pages {
            let open = showing && state.text_page == page;
            let resp = ui
                .add_sized([(width * scale).floor(), ROW_HEIGHT], egui::Button::selectable(open, label.as_str()).truncate())
                .on_hover_text(format!("{label}. {hover}"));
            if resp.clicked() {
                if open {
                    state.side_tab = SideTab::Layers;
                } else {
                    state.side_tab = SideTab::Text;
                    state.text_page = page;
                }
            }
        }
    });
}

fn help(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

// ---- UI state ----

/// UI-only state of the Text block, the Text tab and the font picker (a field of `PanelState`; rebuilt with it when
/// the document changes).
#[derive(Default)]
pub struct TextPanelState {
    /// Last "is this font installed" answer: (family, style, number of catalog families, installed).
    font_check: Option<(String, String, usize, bool)>,
    /// The content field's buffer of the placed text it shows (FEATURE_TEXT 5.12).
    pub(crate) buffer: Option<(StampId, GapBuffer)>,
    /// Face traits per project font face (FEATURE_TEXT 5.9: read once per face).
    pub(crate) face_traits: HashMap<(BlobId, u32), Option<FaceTraits>>,
    /// The content of the placed text the panel saw last (to notice an undo or redo of it).
    pub(crate) seen: Option<(StampId, String)>,
    /// The text line the gap strip shows (0-based).
    pub(crate) strip_line: usize,
    /// The selected slot is to be scrolled into view in the next frame.
    pub(crate) scroll_to_gap: bool,
    /// The font picker's search text and highlighted row.
    pub(crate) picker: font_picker::PickerState,
    /// The style picture under the pointer on the Warp page (its name is written under the grid).
    pub(crate) warp_hover: Option<Option<WarpStyle>>,
    /// Boxes and slots the gap strip laid out in the last frame (test counter).
    #[cfg(test)]
    pub(crate) strip_drawn: (usize, usize),
}

impl TextPanelState {
    /// Whether the catalog has `family` / `style` (cached: the lookup walks every face of the catalog).
    fn font_installed(&mut self, fonts: &FontCatalog, family: &str, style: &str) -> bool {
        let families = fonts.families().len();
        if let Some((f, s, n, ok)) = &self.font_check
            && f == family
            && s == style
            && *n == families
        {
            return *ok;
        }
        let ok = fonts.find(family, style).is_some();
        self.font_check = Some((family.to_owned(), style.to_owned(), families, ok));
        ok
    }

}

// ---- fonts: candidates, resolve_face, resolve_font (TEXT 5.9, 5.13) ----

/// Where a face of the style list comes from.
#[derive(Clone, PartialEq, Debug)]
pub enum FaceSource {
    /// A face of the machine's font catalog (its index in the font file; found again by family and style).
    Catalog(u32),
    /// A font the open project holds (the `FontRef` of a text stamp).
    Project(FontRef),
}

/// One face a text can use.
#[derive(Clone, PartialEq, Debug)]
pub struct FaceChoice {
    pub source: FaceSource,
    pub family: String,
    pub style: String,
    pub traits: FaceTraits,
}

/// The traits of a project font face, read once per `(blob, face)` from its bytes.
fn project_traits(cache: &mut HashMap<(BlobId, u32), Option<FaceTraits>>, doc: &Document, font: &FontRef) -> Option<FaceTraits> {
    *cache
        .entry((font.blob, font.face_index))
        .or_insert_with(|| doc.blobs.get(font.blob).and_then(|b| face_traits(&b.bytes, font.face_index)))
}

/// The faces of `family`: the catalog's and the fonts of that family inside the open project, without duplicates (same
/// family, style and face index: the project's copy wins). Catalog order first, project-only faces after.
pub fn candidates(
    fonts: &FontCatalog,
    doc: Option<&Document>,
    cache: &mut HashMap<(BlobId, u32), Option<FaceTraits>>,
    family: &str,
) -> Vec<FaceChoice> {
    let mut out: Vec<FaceChoice> = fonts
        .faces(family)
        .into_iter()
        .map(|f| FaceChoice {
            source: FaceSource::Catalog(f.face_index),
            family: f.family.clone(),
            style: f.style.clone(),
            traits: FaceTraits { weight: f.weight, italic: f.italic, stretch: f.stretch },
        })
        .collect();
    let mut face_index_of: Vec<Option<u32>> =
        out.iter().map(|c| match c.source { FaceSource::Catalog(i) => Some(i), FaceSource::Project(_) => None }).collect();
    if let Some(doc) = doc {
        for font in project_fonts(doc).into_iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            let traits = project_traits(cache, doc, &font).unwrap_or(PLAIN_TRAITS);
            let choice = FaceChoice { source: FaceSource::Project(font.clone()), family: font.family.clone(), style: font.style.clone(), traits };
            match out.iter().zip(&face_index_of).position(|(c, i)| c.style == font.style && *i == Some(font.face_index)) {
                Some(i) => out[i] = choice,
                None if out.iter().any(|c| matches!(&c.source, FaceSource::Project(f) if *f == font)) => {}
                None => {
                    out.push(choice);
                    face_index_of.push(Some(font.face_index));
                }
            }
        }
    }
    out
}

/// The fonts of the text stamps of `doc`, most recently added first, without duplicates.
pub(crate) fn project_fonts(doc: &Document) -> Vec<FontRef> {
    let mut out: Vec<FontRef> = Vec::new();
    for stamp in doc.stamps.iter().rev() {
        if let StampSource::Text(spec) = &stamp.source
            && doc.blobs.get(spec.font.blob).is_some()
            && !out.contains(&spec.font)
        {
            out.push(spec.font.clone());
        }
    }
    out
}

/// "Is bold" (TEXT 5.9): a face of weight 600 or more, or thickening.
pub fn is_bold(traits: &FaceTraits, bold_em: f32) -> bool {
    traits.weight >= 600 || bold_em > 0.0
}

/// "Is italic": an italic face, or a slant.
pub fn is_italic(traits: &FaceTraits, slant_deg: f32) -> bool {
    traits.italic || slant_deg != 0.0
}

/// The traits of the face the text uses: read from its own font bytes for a placed text (never from the list of
/// installed fonts), from the candidate list for the draft.
fn current_traits(state: &AppState, ps: &mut PanelState, spec: &TextSpec) -> FaceTraits {
    if spec.font.blob != DRAFT_BLOB
        && let Some(doc) = state.doc()
        && let Some(traits) = project_traits(&mut ps.text.face_traits, doc, &spec.font)
    {
        return traits;
    }
    candidates(&state.fonts, state.doc(), &mut ps.text.face_traits, &spec.font.family)
        .into_iter()
        .find(|f| f.style == spec.font.style)
        .map_or(PLAIN_TRAITS, |f| f.traits)
}

/// The face among `faces` for the wishes `want_bold` / `want_italic` and the amounts of thickening and slant that go
/// with it (TEXT 5.9): among the faces with the wished italic flag (else the other) and the current stretch (when
/// several exist), for bold the face of weight 600 or more nearest to 700, else the face of 500 or less nearest to
/// 400; without a face of the wished class it stays in the other. Thickening and slant only make up for what the face
/// lacks: `bold_em` is 0 when the face is bold or bold is not wished, else the old value if above 0, else
/// `FAUX_BOLD_EM`; the same for the slant with `FAUX_SLANT_DEG`. `None` without any face.
pub fn resolve_face(
    faces: &[FaceChoice],
    current: &FaceTraits,
    want_bold: bool,
    want_italic: bool,
    bold_em: f32,
    slant_deg: f32,
) -> Option<(usize, f32, f32)> {
    if faces.is_empty() {
        return None;
    }
    let all: Vec<usize> = (0..faces.len()).collect();
    let flagged: Vec<usize> = all.iter().copied().filter(|&i| faces[i].traits.italic == want_italic).collect();
    let pool = if flagged.is_empty() { all } else { flagged };
    let same_stretch: Vec<usize> = pool.iter().copied().filter(|&i| faces[i].traits.stretch == current.stretch).collect();
    let pool = if same_stretch.is_empty() { pool } else { same_stretch };
    let (target, in_class): (i32, fn(u16) -> bool) =
        if want_bold { (700, |w| w >= 600) } else { (400, |w| w <= 500) };
    let class: Vec<usize> = pool.iter().copied().filter(|&i| in_class(faces[i].traits.weight)).collect();
    let pick_from = if class.is_empty() { &pool } else { &class };
    let &best = pick_from.iter().min_by_key(|&&i| ((i32::from(faces[i].traits.weight) - target).abs(), i))?;
    let face = &faces[best].traits;
    let bold = if face.weight >= 600 || !want_bold { 0.0 } else { amount_or(bold_em.max(0.0), FAUX_BOLD_EM) };
    let slant = if face.italic || !want_italic { 0.0 } else { amount_or(slant_deg, FAUX_SLANT_DEG) };
    Some((best, bold, slant))
}

/// The `FontRef` for `family` / `style` (TEXT 5.13): a font the open project already holds under these names (no file
/// is read), else the catalog's face stored as a blob (once per font and document), else the bundled font with the
/// warning toast of today. `None` without a document.
pub(crate) fn resolve_font(state: &mut AppState, family: &str, style: &str) -> Option<FontRef> {
    if let Some(font) = state.doc().and_then(|d| d.find_font(family, style)) {
        return Some(font);
    }
    let face = state.fonts.find(family, style).and_then(|f| state.fonts.face_bytes(f.id).map(|(bytes, index)| (f, bytes, index)));
    let doc = state.doc_mut()?;
    match face {
        Some((f, bytes, index)) => Some(doc.register_font(&f.family, &f.style, index, bytes)),
        None => {
            let fallback = doc.default_font();
            state.toasts.warn(format!("Font {family} not found, using Ubuntu Light"));
            Some(fallback)
        }
    }
}

/// The style to use after switching to `family`: the current one if that family has it, else "Regular", else the
/// family's first style.
pub(crate) fn pick_style(fonts: &FontCatalog, family: &str, current: &str) -> String {
    let faces = fonts.faces(family);
    if faces.iter().any(|f| f.style == current) {
        return current.to_owned();
    }
    faces
        .iter()
        .find(|f| f.style == "Regular")
        .or_else(|| faces.first())
        .map_or_else(|| current.to_owned(), |f| f.style.clone())
}

// ---- the content field's buffer (FEATURE_TEXT 5.12) ----

/// One call the content field made on its buffer (recorded in tests only).
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BufferCall {
    Insert { text: String, at: usize },
    Delete { range: Range<usize> },
    Replace { text: String },
}

/// The buffer of the content field of a placed text: the text and its gap values, kept together through every edit of
/// the field (FEATURE_TEXT 5.12). A gap value belongs to the character on its left and moves with it.
#[derive(Clone, Debug, Default)]
pub struct GapBuffer {
    pub text: String,
    pub gaps: Vec<Gap>,
    /// The (text, gaps) states of this focus session, oldest first; `at` is the current one.
    history: Vec<(String, Vec<Gap>)>,
    at: usize,
    /// Every call of the field, in order (the two egui proofs read it).
    #[cfg(test)]
    pub(crate) calls: Vec<BufferCall>,
}

impl GapBuffer {
    /// A buffer showing `text` with its gap list; the history starts with this state.
    pub fn new(text: &str, gaps: &[Gap]) -> GapBuffer {
        GapBuffer {
            text: text.to_owned(),
            gaps: gaps.to_vec(),
            history: vec![(text.to_owned(), gaps.to_vec())],
            at: 0,
            #[cfg(test)]
            calls: Vec::new(),
        }
    }

    /// Ends the focus session: the history holds the current state only.
    pub fn forget_history(&mut self) {
        self.history = vec![(self.text.clone(), self.gaps.clone())];
        self.at = 0;
    }

    /// Stores the current state after an edit: later states are dropped, at most `HISTORY_MAX` are kept.
    fn record(&mut self) {
        self.history.truncate(self.at + 1);
        self.history.push((self.text.clone(), self.gaps.clone()));
        if self.history.len() > HISTORY_MAX {
            let extra = self.history.len() - HISTORY_MAX;
            self.history.drain(..extra);
        }
        self.at = self.history.len() - 1;
    }

    /// Number of stored states (tests).
    #[cfg(test)]
    pub(crate) fn history_len(&self) -> usize {
        self.history.len()
    }
}

/// Byte offset of character `index` in `s` (the end for an index past it).
fn byte_at(s: &str, index: usize) -> usize {
    s.char_indices().nth(index).map_or(s.len(), |(b, _)| b)
}

impl egui::TextBuffer for GapBuffer {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        &self.text
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let at = char_index.0;
        let n = text.chars().count();
        let byte = byte_at(&self.text, at);
        self.text.insert_str(byte, text);
        gaps::insert(&mut self.gaps, at as u32, n as u32);
        if n > 0 {
            self.record();
        }
        #[cfg(test)]
        self.calls.push(BufferCall::Insert { text: text.to_owned(), at });
        n
    }

    fn delete_char_range(&mut self, char_range: Range<CharIndex>) {
        let (a, b) = (char_range.start.0, char_range.end.0.max(char_range.start.0));
        let (ba, bb) = (byte_at(&self.text, a), byte_at(&self.text, b));
        self.text.replace_range(ba..bb, "");
        gaps::delete(&mut self.gaps, a as u32..b as u32);
        // egui deletes the empty selection before every insert: no state for that.
        if b > a {
            self.record();
        }
        #[cfg(test)]
        self.calls.push(BufferCall::Delete { range: a..b });
    }

    /// The field's undo and redo (and nothing else in egui 0.37) replace the whole text. The buffer looks for a stored
    /// state with exactly that text, first backward from the current one (an undo), then forward (a redo), and restores
    /// its gap list. Not found (a safety net): the gap list follows by a text comparison (`shift_for_edit`).
    fn replace_with(&mut self, text: &str) {
        let last = self.history.len().checked_sub(1);
        let back = last.and_then(|last| (0..=self.at.min(last)).rev().find(|&i| self.history[i].0 == text));
        let found = back.or_else(|| (self.at + 1..self.history.len()).find(|&i| self.history[i].0 == text));
        match found {
            Some(i) => {
                self.gaps.clone_from(&self.history[i].1);
                self.at = i;
            }
            None => {
                gaps::shift_for_edit(&mut self.gaps, &self.text, text);
                self.text.clear();
                self.text.push_str(text);
                self.record();
            }
        }
        self.text.clear();
        self.text.push_str(text);
        #[cfg(test)]
        self.calls.push(BufferCall::Replace { text: text.to_owned() });
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<GapBuffer>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::tests::sample_state;
    use wte_core::{Placement, Projector, SampleKind, TextureSetId};

    #[test]
    fn style_choice_after_a_family_switch() {
        let fonts = FontCatalog::bundled();
        // The bundled Hack has only "Regular"; Ubuntu (Light) keeps its style.
        assert_eq!(pick_style(&fonts, "Hack", "Light"), "Regular");
        assert_eq!(pick_style(&fonts, "Ubuntu", "Light"), "Light");
        assert_eq!(pick_style(&fonts, "No Such Family", "Bold"), "Bold");
    }

    /// The target table of FEATURE_TEXT 6.2: the tool decides, not the selection.
    #[test]
    fn the_tool_decides_the_target() {
        let mut state = sample_state(SampleKind::Sphere);
        let doc = state.doc_mut().unwrap();
        let spec = TextSpec::new("WTE", doc.default_font());
        let hit = glam::Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
        let p = Projector::from_surface(hit, glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::new(0.2, 0.1), 0.2);
        let id = doc.add_stamp("WTE", StampSource::Text(spec), Placement::Projected(p), TextureSetId(0)).unwrap();
        for tool in [ToolKind::Navigate, ToolKind::Brush, ToolKind::Eraser, ToolKind::Eyedropper, ToolKind::Fill, ToolKind::Stamp] {
            state.tool = tool;
            assert_eq!(target(&state), None, "{tool:?} without a selection");
            state.session.as_mut().unwrap().selected_stamp = Some(id);
            assert_eq!(target(&state), None, "{tool:?} with a selected text");
            state.session.as_mut().unwrap().selected_stamp = None;
        }
        state.tool = ToolKind::Text;
        assert_eq!(target(&state), Some(TextTarget::Draft));
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        assert_eq!(target(&state), Some(TextTarget::Draft), "the Text tool edits the draft, also with a selected text");
        state.tool = ToolKind::Transform;
        assert_eq!(target(&state), Some(TextTarget::Stamp(id)));
        state.session.as_mut().unwrap().selected_stamp = None;
        assert_eq!(target(&state), None);
    }

    fn face(style: &str, weight: u16, italic: bool, stretch: u16) -> FaceChoice {
        let font = FontRef { blob: BlobId(1), face_index: 0, family: "Fam".into(), style: style.into() };
        FaceChoice { source: FaceSource::Project(font), family: "Fam".into(), style: style.into(), traits: FaceTraits { weight, italic, stretch } }
    }

    fn traits(weight: u16, italic: bool) -> FaceTraits {
        FaceTraits { weight, italic, stretch: 5 }
    }

    /// `resolve_face` over hand-made candidate lists (TEXT 8.2 "`text_block.rs`"): B on / off, I on / off and the named
    /// cases.
    #[test]
    fn resolve_face_picks_the_real_face_first() {
        let style = |faces: &[FaceChoice], r: Option<(usize, f32, f32)>| r.map(|(i, b, s)| (faces[i].style.clone(), b, s));
        // Regular and Bold.
        let rb = [face("Regular", 400, false, 5), face("Bold", 700, false, 5)];
        assert_eq!(style(&rb, resolve_face(&rb, &traits(400, false), true, false, 0.0, 0.0)), Some(("Bold".into(), 0.0, 0.0)));
        assert_eq!(style(&rb, resolve_face(&rb, &traits(700, false), false, false, 0.0, 0.0)), Some(("Regular".into(), 0.0, 0.0)));
        // I on without an italic face: slanted by the default; I off again: no slant.
        assert_eq!(style(&rb, resolve_face(&rb, &traits(400, false), false, true, 0.0, 0.0)), Some(("Regular".into(), 0.0, FAUX_SLANT_DEG)));
        assert_eq!(style(&rb, resolve_face(&rb, &traits(400, false), false, false, 0.0, 12.0)), Some(("Regular".into(), 0.0, 0.0)));
        // Light only: B thickens (the default, or the old amount), B off removes it.
        let light = [face("Light", 300, false, 5)];
        assert_eq!(style(&light, resolve_face(&light, &traits(300, false), true, false, 0.0, 0.0)), Some(("Light".into(), FAUX_BOLD_EM, 0.0)));
        assert_eq!(style(&light, resolve_face(&light, &traits(300, false), true, false, 0.05, 0.0)), Some(("Light".into(), 0.05, 0.0)));
        assert_eq!(style(&light, resolve_face(&light, &traits(300, false), false, false, 0.05, 0.0)), Some(("Light".into(), 0.0, 0.0)));
        // Bold only (the family is not installed, the current face is a real Bold from the project): B off finds no
        // regular face, stays on the Bold face, and never thickens it.
        let bold_only = [face("Bold", 700, false, 5)];
        assert_eq!(style(&bold_only, resolve_face(&bold_only, &traits(700, false), false, false, 0.0, 0.0)), Some(("Bold".into(), 0.0, 0.0)));
        assert_eq!(style(&bold_only, resolve_face(&bold_only, &traits(700, false), true, false, 0.0, 0.0)), Some(("Bold".into(), 0.0, 0.0)));
        // Italic and Bold Italic.
        let it = [face("Italic", 400, true, 5), face("Bold Italic", 700, true, 5)];
        assert_eq!(style(&it, resolve_face(&it, &traits(400, true), true, true, 0.0, 0.0)), Some(("Bold Italic".into(), 0.0, 0.0)));
        assert_eq!(style(&it, resolve_face(&it, &traits(700, true), false, true, 0.0, 0.0)), Some(("Italic".into(), 0.0, 0.0)));
        // Two widths: the current stretch is kept.
        let widths = [face("Regular", 400, false, 5), face("Bold", 700, false, 5), face("Condensed Bold", 700, false, 3)];
        assert_eq!(style(&widths, resolve_face(&widths, &FaceTraits { weight: 400, italic: false, stretch: 3 }, true, false, 0.0, 0.0)).unwrap().0, "Condensed Bold");
        assert_eq!(style(&widths, resolve_face(&widths, &traits(400, false), true, false, 0.0, 0.0)).unwrap().0, "Bold");
        // A font change while thickened: to a family with a Bold face that face and no thickening; to one without it the
        // user's 5 % survive.
        assert_eq!(style(&rb, resolve_face(&rb, &traits(300, false), true, false, 0.05, 0.0)), Some(("Bold".into(), 0.0, 0.0)));
        assert_eq!(style(&light, resolve_face(&light, &traits(700, false), true, false, 0.05, 0.0)), Some(("Light".into(), 0.05, 0.0)));
        assert_eq!(resolve_face(&[], &traits(400, false), true, false, 0.0, 0.0), None);
    }

    /// `candidates` merges the project's and the catalog's faces without duplicates; the project's copy wins.
    #[test]
    fn candidates_merge_project_and_catalog_faces() {
        let mut state = sample_state(SampleKind::Sphere);
        let fonts = FontCatalog::bundled();
        let doc = state.doc_mut().unwrap();
        let font = doc.default_font();
        let hit = glam::Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
        let p = Projector::from_surface(hit, glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::new(0.2, 0.1), 0.2);
        doc.add_stamp("A", StampSource::Text(TextSpec::new("A", font.clone())), Placement::Projected(p), TextureSetId(0)).unwrap();
        doc.add_stamp("B", StampSource::Text(TextSpec::new("B", font.clone())), Placement::Projected(p), TextureSetId(0)).unwrap();
        let mut cache = HashMap::new();
        let list = candidates(&fonts, state.doc(), &mut cache, "ubuntu");
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].source, FaceSource::Project(font));
        assert_eq!(list[0].traits.weight, 300);
        let hack = candidates(&fonts, state.doc(), &mut cache, "Hack");
        assert!(matches!(hack[..], [FaceChoice { source: FaceSource::Catalog(_), .. }]), "{hack:?}");
        assert!(candidates(&fonts, state.doc(), &mut cache, "Nobody").is_empty());
    }

    #[test]
    fn angles_wrap_into_the_field_range() {
        assert_eq!(wrap_deg(270.0), -90.0);
        assert_eq!(wrap_deg(-190.0), 170.0);
        assert_eq!(wrap_deg(180.0), 180.0);
        assert_eq!(wrap_deg(30.0), 30.0);
        assert_eq!(wrap_deg(f32::NAN), 0.0);
    }

    /// The history of the content field (TEXT 5.12): an undo brings back the gap list of the state with that text, a
    /// redo the later one; a text that was never held falls back to the text comparison.
    #[test]
    fn the_history_finds_states_by_their_text() {
        use egui::TextBuffer;
        let mut buf = GapBuffer::new("WTE 2026", &[Gap { after: 2, em: 0.3 }]);
        buf.delete_char_range(CharIndex(1)..CharIndex(3)); // "TE" removed with the gap of "E"
        assert_eq!((buf.text.as_str(), buf.gaps.as_slice()), ("W 2026", &[][..]));
        buf.replace_with("WTE 2026");
        assert_eq!(buf.gaps, [Gap { after: 2, em: 0.3 }], "undo brings the gap back");
        buf.replace_with("W 2026");
        assert!(buf.gaps.is_empty(), "redo");
        buf.replace_with("XW 2026");
        assert_eq!(buf.text, "XW 2026");
        buf.delete_char_range(CharIndex(0)..CharIndex(0));
        assert_eq!(buf.history_len(), 3, "an empty delete stores nothing");
        buf.forget_history();
        assert_eq!(buf.history_len(), 1);
    }
}
