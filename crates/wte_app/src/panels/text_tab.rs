// Owner: T6 text panels (round 4). Written by the scaffold S2 as a stub, filled by T6.
//
// The Text tab of the lower side-panel area (FEATURE_TEXT 6.4): the page selector Character | Warp | Path
// (`state.text_page`) and the pages, with the gap controls of 6.2 on the Character page. It edits the target of
// `text_block::target(state)` (the tab is shown only when there is one) through the same spec helpers as the Text
// block: the text to place is changed directly and queues nothing; a placed text gets `UpdateStampSource` with
// `Commit::Preview` while a control is dragged and one `Final` (a click on a button is one `Final` at once). The
// container, the tab strip and the tab rule are in `panels/mod.rs`; this function draws into any `egui::Ui` of
// 265 pt width or more.
//
// The gap strip lays out only the visible cells (TEXT 6.2 rule 1): the scroll area gets the full content width as one
// allocation and only the cells inside the clip rectangle are drawn, so a line of 1,000 characters costs about a dozen
// widgets per frame.

use eframe::egui;
use egui::accesskit::Role;
use egui::{Align, Align2, Color32, DragValue, Pos2, Rect, Sense, Shape, Slider, SliderClamping, Stroke, pos2, vec2};
use wte_core::text::{WarpMap, gaps, path};
use wte_core::{MAX_PATH_POINTS, Outline, PathShape, StampId, StampSource, TextAlign, TextPath, TextSpec, Warp, WarpStyle};

use crate::action::AppAction;
use crate::panels::text_block::{self, TextTarget, click_edit, live_edit};
use crate::panels::{PanelState, from_color32, percent_slider, to_color32};
use crate::state::{AppState, TextGapSel, TextPage};

/// Width of the label column.
const LABEL_WIDTH: f32 = 84.0;
/// Width of the sliders of the pages.
const SLIDER_WIDTH: f32 = 100.0;
/// Width of the outline-width slider in the Outline row (checkbox + slider + value + colour button).
const OUTLINE_SLIDER_WIDTH: f32 = 56.0;
/// Gap strip: a box of 14 pt and a slot of 10 pt, so every cell is 24 pt wide (TEXT 6.2 rule 1).
const BOX_WIDTH: f32 = 14.0;
const SLOT_WIDTH: f32 = 10.0;
const CELL_WIDTH: f32 = BOX_WIDTH + SLOT_WIDTH;
const STRIP_HEIGHT: f32 = 24.0;
/// Style pictures of the Warp page: 30 x 26 pt, 3 pt apart, two rows of 8 (TEXT 6.4).
const STYLE_CELL: [f32; 2] = [30.0, 26.0];
const STYLE_GAP: f32 = 3.0;
/// Gap value limits in % of the text size (TEXT 1.2).
const GAP_MIN_PCT: f32 = -50.0;
const GAP_MAX_PCT: f32 = 400.0;
/// Limits of the Start field of a path, in em, and of a circle's diameter.
const START_MAX_EM: f32 = 100.0;
const DIAMETER_RANGE: std::ops::RangeInclusive<f32> = 0.5..=100.0;
/// Hover text of the gap controls before placing (TEXT 6.2 rule 7).
const GAPS_NEED_A_PLACED_TEXT: &str = "Place the text first; then the space between any two characters can be changed";
/// Hover text of the gap value controls without a selected gap.
const NO_GAP_SELECTED: &str = "Click a gap in the strip above";
/// The warning of a text longer than its circle (TEXT 6.4).
pub(crate) const CIRCLE_TOO_SHORT: &str = "The text is longer than the circle. Raise Diameter or press Fit.";

/// Draws the Text tab. Text controls compute the new spec and push `UpdateStampSource` (for a placed text) or change
/// `state.tools.text` (for the text to place).
pub fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    let Some(target) = text_block::target(state) else { return };
    let Some(spec) = text_block::current_spec(state, target) else { return };
    let mut actions = Vec::new();
    page_selector(ui, state);
    let locked = match target {
        TextTarget::Stamp(id) => state.doc().is_some_and(|d| d.stamp_locked(id)),
        TextTarget::Draft => false,
    };
    ui.add_enabled_ui(!locked, |ui| match state.text_page {
        TextPage::Character => character_page(ui, state, ps, target, &spec, &mut actions),
        TextPage::Warp => warp_page(ui, state, ps, target, &spec, &mut actions),
        TextPage::Path => path_page(ui, state, ps, target, &spec, &mut actions),
    });
    gap_status(state, target, &spec);
    state.actions.extend(actions);
}

/// The first row: Character | Warp | Path.
fn page_selector(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        for (page, label) in [(TextPage::Character, "Character"), (TextPage::Warp, "Warp"), (TextPage::Path, "Path")] {
            if ui.add(egui::Button::selectable(state.text_page == page, label).min_size(vec2(60.0, 20.0))).clicked() {
                state.text_page = page;
            }
        }
    });
}

fn grid_label(ui: &mut egui::Ui, text: &str) {
    ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 18.0), egui::Layout::left_to_right(Align::Center), |ui| {
        ui.add(egui::Label::new(text).truncate());
    });
}

fn grid(ui: &mut egui::Ui, salt: &str, target: TextTarget, add: impl FnOnce(&mut egui::Ui)) {
    let salt_id = match target {
        TextTarget::Draft => u64::MAX,
        TextTarget::Stamp(id) => id.0,
    };
    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    egui::Grid::new((salt, salt_id)).num_columns(2).spacing([6.0, 2.0]).show(ui, add);
}

// ---- Character page ----

fn character_page(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    grid(ui, "text_character_page", target, |ui| {
        grid_label(ui, "Align");
        let mut align = spec.align;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut align, TextAlign::Left, "Left");
            ui.selectable_value(&mut align, TextAlign::Center, "Center");
            ui.selectable_value(&mut align, TextAlign::Right, "Right");
        });
        if align != spec.align {
            let new = TextSpec { align, ..spec.clone() };
            click_edit(state, target, spec, &new, actions);
        }
        ui.end_row();

        grid_label(ui, "Line spacing");
        let mut v = spec.line_spacing;
        let resp = ui.add(Slider::new(&mut v, 0.5..=3.0).max_decimals(2).suffix(" x").clamping(SliderClamping::Edits));
        let new = TextSpec { line_spacing: v, ..spec.clone() };
        live_edit(state, ps, ui, &resp, target, spec, &new, actions);
        ui.end_row();

        grid_label(ui, "Letter spacing");
        let mut v = spec.letter_spacing;
        let resp = ui.add(percent_slider(&mut v, -20.0, 100.0)).on_hover_text("Extra space between letters, % of the text size");
        let new = TextSpec { letter_spacing: v, ..spec.clone() };
        live_edit(state, ps, ui, &resp, target, spec, &new, actions);
        ui.end_row();

        grid_label(ui, "Thicken");
        let mut v = spec.fx.bold_em;
        let resp = ui.add(percent_slider(&mut v, 0.0, 8.0)).on_hover_text("Every edge of the letters grows by this share of the text size");
        let mut new = spec.clone();
        new.fx.bold_em = v;
        live_edit(state, ps, ui, &resp, target, spec, &new, actions);
        ui.end_row();

        grid_label(ui, "Slant");
        let mut v = spec.fx.slant_deg;
        let resp = ui
            .add(Slider::new(&mut v, -45.0..=45.0).max_decimals(1).suffix(" °").clamping(SliderClamping::Edits))
            .on_hover_text("Leans every letter; the baseline stays where it is");
        let mut new = spec.clone();
        new.fx.slant_deg = v;
        live_edit(state, ps, ui, &resp, target, spec, &new, actions);
        ui.end_row();

        grid_label(ui, "Outline");
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = OUTLINE_SLIDER_WIDTH;
            let mut on = spec.outline.is_some();
            if ui.checkbox(&mut on, "").on_hover_text("Outline around the letters").changed() {
                let t = &state.tools.text;
                let outline = on.then_some(Outline { width_em: t.outline_width_em.clamp(0.0, 0.25), color: t.outline_color });
                let new = TextSpec { outline, ..spec.clone() };
                click_edit(state, target, spec, &new, actions);
            }
            let outline = spec.outline.unwrap_or(Outline { width_em: state.tools.text.outline_width_em, color: state.tools.text.outline_color });
            let mut width = outline.width_em;
            let resp = ui
                .add_enabled(spec.outline.is_some(), percent_slider(&mut width, 0.0, 25.0))
                .on_hover_text("Outline width, % of the text size");
            if spec.outline.is_some() {
                let new = TextSpec { outline: Some(Outline { width_em: width, ..outline }), ..spec.clone() };
                live_edit(state, ps, ui, &resp, target, spec, &new, actions);
            }
            let mut c = to_color32(outline.color);
            let resp = ui
                .add_enabled_ui(spec.outline.is_some(), |ui| egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend))
                .inner
                .on_hover_text("Outline colour");
            if spec.outline.is_some() {
                let new = TextSpec { outline: Some(Outline { color: from_color32(c), ..outline }), ..spec.clone() };
                live_edit(state, ps, ui, &resp, target, spec, &new, actions);
            }
        });
        ui.end_row();
    });
    gap_controls(ui, state, ps, target, spec, actions);
}

/// One box of the gap strip: the stored index of its last character (a joined mark belongs to the box of its base
/// letter) and what it shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StripBox {
    pub last: u32,
    pub text: String,
}

/// The boxes of every text line: one per laid-out character; a character that joins the one before it
/// (`gaps::is_joined`, and every character after a zero-width joiner) is drawn in the box before it; skipped control
/// characters have no box.
pub(crate) fn strip_boxes(content: &str) -> Vec<Vec<StripBox>> {
    let mut lines = vec![Vec::new()];
    let mut after_zwj = false;
    for (i, ch) in content.chars().enumerate() {
        let i = i as u32;
        if ch == '\n' {
            lines.push(Vec::new());
            after_zwj = false;
            continue;
        }
        if ch.is_control() && ch != '\t' {
            continue;
        }
        let line: &mut Vec<StripBox> = lines.last_mut().expect("one line at least");
        let shown = match ch {
            ' ' => "·".to_owned(),
            '\t' => "→".to_owned(),
            c => c.to_string(),
        };
        match line.last_mut() {
            Some(last) if gaps::is_joined(ch) || after_zwj => {
                last.last = i;
                if !matches!(ch, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}') {
                    last.text.push(ch);
                }
            }
            _ => line.push(StripBox { last: i, text: shown }),
        }
        after_zwj = ch == '\u{200D}';
    }
    lines
}

/// The hover text of a slot: "Gap between E and (space): 30 %", or for an end slot "Stored gap after 6: 30 % (no effect
/// at the end of a line)".
pub(crate) fn slot_text(slot: &gaps::GapSlot) -> String {
    let pct = slot.em * 100.0;
    match slot.right {
        Some(right) => format!("Gap between {} and {}: {} %", gaps::slot_name(slot.left), gaps::slot_name(right), fmt_pct(pct)),
        None => format!("Stored gap after {}: {} % (no effect at the end of a line)", gaps::slot_name(slot.left), fmt_pct(pct)),
    }
}

fn fmt_pct(pct: f32) -> String {
    let r = (pct * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r:.1}") }
}

/// The gap controls of the Character page (TEXT 6.2 "The gap controls"): header with the line buttons, the strip, the
/// value row.
fn gap_controls(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    let stamp = match target {
        TextTarget::Stamp(id) => Some(id),
        TextTarget::Draft => None,
    };
    let slots = gaps::slots(&spec.content, &spec.fx.gaps);
    let boxes = strip_boxes(&spec.content);
    let line_count = boxes.len().max(1);
    // The selection lives in the session; it is dropped when its character no longer has a slot.
    let mut selected = stamp.and_then(|id| state.session.as_ref()?.text_gap.filter(|g| g.stamp == id));
    if let Some(sel) = selected
        && !slots.iter().flatten().any(|s| s.after == sel.after)
    {
        if let Some(session) = state.session.as_mut() {
            session.text_gap = None;
        }
        selected = None;
    }
    ps.text.strip_line = ps.text.strip_line.min(line_count - 1);

    ui.add_enabled_ui(stamp.is_some(), |ui| {
        ui.horizontal(|ui| {
            ui.label("Gaps");
            ui.weak(format!("line {} of {}", ps.text.strip_line + 1, line_count));
            let prev = ui.add_enabled(ps.text.strip_line > 0, egui::Button::new("<")).on_hover_text("Previous line");
            if prev.clicked() {
                ps.text.strip_line -= 1;
            }
            let next = ui.add_enabled(ps.text.strip_line + 1 < line_count, egui::Button::new(">")).on_hover_text("Next line");
            if next.clicked() {
                ps.text.strip_line += 1;
            }
        })
        .response
        .on_disabled_hover_text(GAPS_NEED_A_PLACED_TEXT);
        let line = ps.text.strip_line;
        let empty_boxes = Vec::new();
        let empty_slots = Vec::new();
        let line_boxes = boxes.get(line).unwrap_or(&empty_boxes);
        let line_slots = slots.get(line).unwrap_or(&empty_slots);
        if let Some(clicked) = strip(ui, ps, stamp, line, line_boxes, line_slots, selected)
            && let Some(session) = state.session.as_mut()
            && let Some(id) = stamp
        {
            session.text_gap = Some(TextGapSel { stamp: id, after: clicked });
            selected = session.text_gap;
        }
        value_row(ui, state, ps, target, spec, selected, actions);
    });
}

/// The strip of one text line. Returns the `after` of a clicked slot.
#[allow(clippy::too_many_arguments, reason = "the line, its boxes and slots and the selection")]
fn strip(
    ui: &mut egui::Ui,
    ps: &mut PanelState,
    stamp: Option<StampId>,
    line: usize,
    boxes: &[StripBox],
    slots: &[gaps::GapSlot],
    selected: Option<TextGapSel>,
) -> Option<u32> {
    let mut clicked = None;
    let mut drawn = (0usize, 0usize);
    let scroll_to = std::mem::take(&mut ps.text.scroll_to_gap);
    let selected_cell = selected.and_then(|sel| boxes.iter().position(|b| b.last == sel.after));
    let mut scroll = egui::ScrollArea::horizontal()
        .id_salt(("text_gap_strip", stamp.map_or(u64::MAX, |s| s.0), line))
        .auto_shrink([false, true]);
    if scroll_to && let Some(cell) = selected_cell {
        let width = ui.available_width();
        scroll = scroll.horizontal_scroll_offset((cell as f32 * CELL_WIDTH - width / 2.0).max(0.0));
    }
    scroll.show_viewport(ui, |ui, viewport| {
        let total = boxes.len() as f32 * CELL_WIDTH + SLOT_WIDTH;
        ui.set_min_size(vec2(total, STRIP_HEIGHT));
        let base = ui.min_rect().min;
        let first = (viewport.min.x / CELL_WIDTH).floor().max(0.0) as usize;
        let last = ((viewport.max.x / CELL_WIDTH).ceil().max(0.0) as usize).min(boxes.len());
        let visuals = ui.visuals().clone();
        for (i, b) in boxes.iter().enumerate().take(last).skip(first) {
            let x = base.x + i as f32 * CELL_WIDTH;
            let box_rect = Rect::from_min_size(pos2(x, base.y + 2.0), vec2(BOX_WIDTH, STRIP_HEIGHT - 4.0));
            ui.painter().rect_stroke(box_rect, 2.0, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
            ui.painter().text(box_rect.center(), Align2::CENTER_CENTER, &b.text, egui::FontId::proportional(12.0), visuals.text_color());
            drawn.0 += 1;
            let Some(slot) = slots.iter().find(|s| s.after == b.last) else { continue };
            let slot_rect = Rect::from_min_size(pos2(x + BOX_WIDTH + 1.0, base.y + 2.0), vec2(SLOT_WIDTH - 2.0, STRIP_HEIGHT - 4.0));
            let id = ui.scope_id().with(("gap_slot", slot.after));
            let resp = ui.interact(slot_rect, id, Sense::click());
            let text = slot_text(slot);
            let is_selected = selected.is_some_and(|s| s.after == slot.after);
            resp.widget_info(|| egui::WidgetInfo::selected(Role::Button, ui.is_enabled(), is_selected, text.as_str()));
            drawn.1 += 1;
            if slot.em != 0.0 {
                ui.painter().rect_filled(slot_rect, 2.0, visuals.selection.bg_fill);
            } else if resp.hovered() {
                ui.painter().rect_filled(slot_rect, 2.0, visuals.widgets.hovered.weak_bg_fill);
            }
            if slot.right.is_none() {
                dashed_frame(ui, slot_rect, visuals.text_color());
            }
            if is_selected {
                ui.painter().rect_stroke(slot_rect.expand(1.0), 2.0, Stroke::new(2.0, visuals.strong_text_color()), egui::StrokeKind::Outside);
            }
            let resp = resp.on_hover_text(text);
            if resp.clicked() {
                clicked = Some(slot.after);
            }
        }
    });
    #[cfg(test)]
    {
        ps.text.strip_drawn = drawn;
    }
    let _ = drawn;
    clicked
}

fn dashed_frame(ui: &egui::Ui, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let corners: [Pos2; 5] = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    ui.painter().extend(Shape::dashed_line(&corners, stroke, 3.0, 2.0));
}

/// `-`, the value, `+`, Reset, Reset all (TEXT 6.2 rules 4, 5).
fn value_row(
    ui: &mut egui::Ui,
    state: &mut AppState,
    ps: &mut PanelState,
    target: TextTarget,
    spec: &TextSpec,
    selected: Option<TextGapSel>,
    actions: &mut Vec<AppAction>,
) {
    let after = selected.map(|s| s.after);
    let value = after.map_or(0.0, |a| gaps::get(&spec.fx.gaps, a));
    let with_gap = |em: f32| {
        let mut new = spec.clone();
        if let Some(a) = after {
            gaps::set(&mut new.fx.gaps, a, em);
        }
        new
    };
    let step = if ui.input(|i| i.modifiers.shift) { 0.05 } else { 0.01 };
    let why = if matches!(target, TextTarget::Draft) { GAPS_NEED_A_PLACED_TEXT } else { NO_GAP_SELECTED };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let on = after.is_some();
        let minus = ui.add_enabled(on, egui::Button::new("-").min_size(vec2(22.0, 20.0))).on_disabled_hover_text(why).on_hover_text("1 % less (5 % with Shift)");
        let mut pct = value * 100.0;
        let resp = ui
            .add_enabled(
                on,
                DragValue::new(&mut pct).range(GAP_MIN_PCT..=GAP_MAX_PCT).clamp_existing_to_range(false).speed(0.2).max_decimals(1).suffix(" %"),
            )
            .on_disabled_hover_text(why)
            .on_hover_text("Extra space in the selected gap, % of the text size. Adds to Letter spacing and to the kerning of the font");
        if on {
            let new = with_gap((pct / 100.0).clamp(GAP_MIN_PCT / 100.0, GAP_MAX_PCT / 100.0));
            live_edit(state, ps, ui, &resp, target, spec, &new, actions);
        }
        let plus = ui.add_enabled(on, egui::Button::new("+").min_size(vec2(22.0, 20.0))).on_disabled_hover_text(why).on_hover_text("1 % more (5 % with Shift)");
        if minus.clicked() {
            click_edit(state, target, spec, &with_gap(value - step), actions);
        }
        if plus.clicked() {
            click_edit(state, target, spec, &with_gap(value + step), actions);
        }
        let reset = ui
            .add_enabled(on && value != 0.0, egui::Button::new("Reset").min_size(vec2(52.0, 20.0)))
            .on_hover_text("Sets the selected gap back to 0");
        if reset.clicked() {
            click_edit(state, target, spec, &with_gap(0.0), actions);
        }
        let all = ui
            .add_enabled(!spec.fx.gaps.is_empty() && !matches!(target, TextTarget::Draft), egui::Button::new("Reset all").min_size(vec2(68.0, 20.0)))
            .on_hover_text("Removes the own value of every gap of the text");
        if all.clicked() {
            let mut new = spec.clone();
            new.fx.gaps.clear();
            click_edit(state, target, spec, &new, actions);
        }
    });
}

/// The status bar line of the selected gap: "Gap between E and (space): 30 % of the text size".
fn gap_status(state: &mut AppState, target: TextTarget, spec: &TextSpec) {
    let selected = match target {
        TextTarget::Stamp(id) => state.session.as_ref().and_then(|s| s.text_gap).filter(|g| g.stamp == id),
        TextTarget::Draft => None,
    };
    let slot = selected.and_then(|sel| gaps::slots(&spec.content, &spec.fx.gaps).into_iter().flatten().find(|s| s.after == sel.after));
    match slot {
        Some(slot) if slot.right.is_some() => state.status_hint = format!("{} of the text size", slot_text(&slot)),
        Some(slot) => state.status_hint = slot_text(&slot),
        None => clear_gap_status(state),
    }
}

/// Clears a gap line of the status bar (when no gap is selected or the tab is not shown).
pub(crate) fn clear_gap_status(state: &mut AppState) {
    if state.status_hint.starts_with("Gap between ") || state.status_hint.starts_with("Stored gap after ") {
        state.status_hint.clear();
    }
}

// ---- Warp page ----

/// The style list of the Warp page: None and the 15 styles.
fn styles() -> impl Iterator<Item = Option<WarpStyle>> {
    std::iter::once(None).chain(WarpStyle::ALL.into_iter().map(Some))
}

fn style_label(style: Option<WarpStyle>) -> &'static str {
    style.map_or("None", WarpStyle::label)
}

fn warp_page(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    let chosen = spec.fx.warp.map(|w| w.style);
    let vertical_axis = spec.fx.warp.is_some_and(|w| w.vertical_axis);
    // The 16 style pictures, two rows of 8.
    let mut hovered = None;
    let mut pick: Option<Option<WarpStyle>> = None;
    let grid_size = vec2(8.0 * STYLE_CELL[0] + 7.0 * STYLE_GAP, 2.0 * STYLE_CELL[1] + STYLE_GAP);
    let (area, _) = ui.allocate_exact_size(grid_size, Sense::hover());
    for (i, style) in styles().enumerate() {
        let (col, row) = ((i % 8) as f32, (i / 8) as f32);
        let rect = Rect::from_min_size(
            area.min + vec2(col * (STYLE_CELL[0] + STYLE_GAP), row * (STYLE_CELL[1] + STYLE_GAP)),
            vec2(STYLE_CELL[0], STYLE_CELL[1]),
        );
        let resp = ui.interact(rect, ui.scope_id().with(("warp_style", i)), Sense::click());
        let selected = style == chosen;
        resp.widget_info(|| egui::WidgetInfo::selected(Role::Button, ui.is_enabled(), selected, style_label(style)));
        style_picture(ui, rect, style, vertical_axis, selected, resp.hovered());
        if resp.hovered() {
            hovered = Some(style);
        }
        if resp.on_hover_text(style_label(style)).clicked() {
            pick = Some(style);
        }
    }
    ps.text.warp_hover = hovered;
    ui.weak(style_label(hovered.unwrap_or(chosen)));
    if let Some(style) = pick
        && style != chosen
    {
        let mut new = spec.clone();
        new.fx.warp = style.map(|s| match spec.fx.warp {
            Some(w) => Warp { style: s, ..w },
            None => Warp { bend: 0.5, ..Warp::new(s) },
        });
        click_edit(state, target, spec, &new, actions);
    }

    let warp = spec.fx.warp;
    let on = warp.is_some();
    let w = warp.unwrap_or(Warp::new(WarpStyle::Arc));
    grid(ui, "text_warp_page", target, |ui| {
        let sliders = [
            ("Bend", 0, "Strength and direction of the style"),
            ("Horizontal", 1, "One side of the text gets taller, the other shorter"),
            ("Vertical", 2, "The top gets wider, the bottom narrower, or the reverse"),
        ];
        for (label, field, hover) in sliders {
            grid_label(ui, label);
            let mut v = *warp_amount(&mut w.clone(), field);
            let resp = ui.add_enabled(on, percent_slider(&mut v, -100.0, 100.0)).on_hover_text(hover).on_disabled_hover_text("Choose a style first");
            if on {
                let mut nw = w;
                *warp_amount(&mut nw, field) = v;
                let mut new = spec.clone();
                new.fx.warp = Some(nw);
                live_edit(state, ps, ui, &resp, target, spec, &new, actions);
            }
            ui.end_row();
        }
        grid_label(ui, "Direction");
        ui.horizontal(|ui| {
            let mut axis = w.vertical_axis;
            ui.add_enabled_ui(on, |ui| {
                ui.selectable_value(&mut axis, false, "Horizontal");
                ui.selectable_value(&mut axis, true, "Vertical");
            });
            if on && axis != w.vertical_axis {
                let mut new = spec.clone();
                new.fx.warp = Some(Warp { vertical_axis: axis, ..w });
                click_edit(state, target, spec, &new, actions);
            }
            if ui.add_enabled(on, egui::Button::new("Reset")).on_hover_text("Style None, sliders back to the defaults").clicked() {
                let mut new = spec.clone();
                new.fx.warp = None;
                click_edit(state, target, spec, &new, actions);
            }
        });
        ui.end_row();
    });
}

/// Bend (0), Horizontal (1) or Vertical (2) of a warp.
fn warp_amount(w: &mut Warp, field: usize) -> &mut f32 {
    match field {
        0 => &mut w.bend,
        1 => &mut w.horizontal,
        _ => &mut w.vertical,
    }
}

/// A style picture: a grid of 4 x 2 squares sent through `WarpMap` at 50 % bend, so a picture can never disagree with
/// the result (TEXT 6.4).
fn style_picture(ui: &egui::Ui, rect: Rect, style: Option<WarpStyle>, vertical_axis: bool, selected: bool, hovered: bool) {
    let visuals = ui.visuals();
    let fill = if selected { visuals.selection.bg_fill } else if hovered { visuals.widgets.hovered.weak_bg_fill } else { visuals.extreme_bg_color };
    ui.painter().rect_filled(rect, 2.0, fill);
    let (w, h) = (40.0f32, 20.0f32);
    let map = style.map(|s| WarpMap::new(&Warp { bend: 0.5, vertical_axis, ..Warp::new(s) }, [0.0, 0.0], [w, h]));
    let warp = |p: [f32; 2]| map.as_ref().map_or(p, |m| m.map(p));
    let mut lines: Vec<Vec<[f32; 2]>> = Vec::new();
    const STEPS: usize = 8;
    for i in 0..=4 {
        let x = w * i as f32 / 4.0;
        lines.push((0..=STEPS).map(|k| warp([x, h * k as f32 / STEPS as f32])).collect());
    }
    for j in 0..=2 {
        let y = h * j as f32 / 2.0;
        lines.push((0..=STEPS).map(|k| warp([w * k as f32 / STEPS as f32, y])).collect());
    }
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for p in lines.iter().flatten() {
        for a in 0..2 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let inner = rect.shrink(3.0);
    let scale = (inner.width() / (hi[0] - lo[0]).max(1e-3)).min(inner.height() / (hi[1] - lo[1]).max(1e-3));
    if !scale.is_finite() {
        return;
    }
    let offset = inner.center() - vec2((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0) * scale;
    let color = if selected { visuals.selection.stroke.color } else { visuals.text_color() };
    for line in lines {
        let points: Vec<Pos2> = line.iter().map(|p| offset + vec2(p[0], p[1]) * scale).collect();
        ui.painter().add(Shape::line(points, Stroke::new(1.0, color)));
    }
}

// ---- Path page ----

/// The kinds of the Path page.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Kind {
    Straight,
    Arc,
    Circle,
    Curve,
}

fn kind_of(path: Option<&TextPath>) -> Kind {
    match path.map(|p| &p.shape) {
        None => Kind::Straight,
        Some(PathShape::Arc { .. }) => Kind::Arc,
        Some(PathShape::Circle { .. }) => Kind::Circle,
        Some(PathShape::Curve { .. }) => Kind::Curve,
    }
}

/// The straight frame of the text (no line): what a new Circle or Curve is created from (TEXT 5.5 "Creation"). For the
/// text to place its font is resolved like for the placement.
fn straight_frame(state: &mut AppState, target: TextTarget, spec: &TextSpec) -> Option<wte_core::TextFrame> {
    let mut straight = spec.clone();
    straight.fx.path = None;
    if matches!(target, TextTarget::Draft) {
        straight.font = text_block::resolve_font(state, &spec.font.family, &spec.font.style)?;
    }
    state.doc_mut()?.stamp_source_frame(&StampSource::Text(straight)).ok()
}

/// The frame of the text as it is now (for Fit and the circle hint).
fn current_frame(state: &mut AppState, target: TextTarget, spec: &TextSpec) -> Option<wte_core::TextFrame> {
    match target {
        TextTarget::Stamp(id) => text_block::stamp_frame(state, id).map(|(_, f)| f),
        TextTarget::Draft => {
            let mut s = spec.clone();
            s.font = state.doc().and_then(|d| d.find_font(&spec.font.family, &spec.font.style))?;
            state.doc_mut()?.stamp_source_frame(&StampSource::Text(s)).ok()
        }
    }
}

/// The line a newly chosen kind starts with (TEXT 6.4): Arc with 25 %; Circle with the default diameter, centred over
/// its top point; Curve with three points on the baseline.
fn new_path(state: &mut AppState, target: TextTarget, spec: &TextSpec, kind: Kind) -> Option<TextPath> {
    match kind {
        Kind::Straight => None,
        Kind::Arc => Some(TextPath { shape: PathShape::Arc { bend: 0.25 }, start: 0.0, flip: false }),
        Kind::Circle => Some(match straight_frame(state, target, spec) {
            Some(frame) => path::new_circle(&frame, spec.align),
            None => TextPath { shape: PathShape::Circle { diameter: 1.0, center: [0.0, 0.5] }, start: 0.0, flip: false },
        }),
        Kind::Curve => Some(match straight_frame(state, target, spec) {
            Some(frame) => path::new_curve(&frame),
            None => TextPath { shape: PathShape::Curve { points: vec![[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]] }, start: 0.0, flip: false },
        }),
    }
}

/// Add point: a point in the middle of the longest piece.
pub(crate) fn add_point(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = points.to_vec();
    if points.len() < 2 || points.len() >= MAX_PATH_POINTS {
        return out;
    }
    let len = |i: usize| ((points[i + 1][0] - points[i][0]).powi(2) + (points[i + 1][1] - points[i][1]).powi(2)).sqrt();
    let longest = (0..points.len() - 1).max_by(|&a, &b| len(a).total_cmp(&len(b))).unwrap_or(0);
    let (a, b) = (points[longest], points[longest + 1]);
    out.insert(longest + 1, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
    out
}

fn path_page(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, target: TextTarget, spec: &TextSpec, actions: &mut Vec<AppAction>) {
    let kind = kind_of(spec.fx.path.as_ref());
    let draft = matches!(target, TextTarget::Draft);
    let mut chosen = None;
    ui.horizontal(|ui| {
        for (k, label) in [(Kind::Straight, "Straight"), (Kind::Arc, "Arc"), (Kind::Circle, "Circle"), (Kind::Curve, "Curve")] {
            let enabled = !(draft && k == Kind::Curve);
            let resp = ui
                .add_enabled(enabled, egui::Button::selectable(kind == k, label))
                .on_disabled_hover_text("Place the text first, then bend its line with the round points");
            if resp.clicked() && kind != k {
                chosen = Some(k);
            }
        }
    });
    if let Some(k) = chosen {
        let mut new = spec.clone();
        new.fx.path = new_path(state, target, spec, k);
        click_edit(state, target, spec, &new, actions);
        return;
    }
    let Some(path_now) = spec.fx.path.clone() else {
        ui.weak("The text runs straight. Choose Arc, Circle or Curve to put it on a line.");
        return;
    };
    let with_path = |p: TextPath| {
        let mut new = spec.clone();
        new.fx.path = Some(p);
        new
    };
    let mut fill = 0.0;
    grid(ui, "text_path_page", target, |ui| {
        match &path_now.shape {
            PathShape::Arc { bend } => {
                grid_label(ui, "Bend");
                let mut v = *bend;
                let resp = ui
                    .add(percent_slider(&mut v, -100.0, 100.0))
                    .on_hover_text("Share of a full circle the text covers. Positive arches upward, negative hangs like a smile");
                let new = with_path(TextPath { shape: PathShape::Arc { bend: v }, ..path_now.clone() });
                live_edit(state, ps, ui, &resp, target, spec, &new, actions);
                ui.end_row();
            }
            PathShape::Circle { diameter, center } => {
                grid_label(ui, "Diameter");
                let mut d = *diameter;
                let mut fit = false;
                ui.horizontal(|ui| {
                    let resp = ui
                        .add(DragValue::new(&mut d).range(DIAMETER_RANGE).clamp_existing_to_range(false).speed(0.02).max_decimals(2).suffix(" em"))
                        .on_hover_text("Diameter of the circle, in text sizes. The circle keeps this size whatever is typed");
                    let new = with_path(TextPath { shape: PathShape::Circle { diameter: d, center: *center }, ..path_now.clone() });
                    live_edit(state, ps, ui, &resp, target, spec, &new, actions);
                    fit = ui.button("Fit").on_hover_text("Sets the diameter from the text as it is now; the top point stays").clicked();
                });
                if fit && let Some(frame) = current_frame(state, target, spec) {
                    let new = with_path(path::fit_circle(&frame, spec.align, &path_now));
                    click_edit(state, target, spec, &new, actions);
                }
                ui.end_row();
            }
            PathShape::Curve { .. } => {}
        }
        grid_label(ui, "Start");
        let mut s = path_now.start;
        let resp = ui
            .add(DragValue::new(&mut s).range(-START_MAX_EM..=START_MAX_EM).clamp_existing_to_range(false).speed(0.02).max_decimals(2).suffix(" em"))
            .on_hover_text("Slides the text along the line, in text sizes");
        let new = with_path(TextPath { start: s, ..path_now.clone() });
        live_edit(state, ps, ui, &resp, target, spec, &new, actions);
        ui.end_row();

        grid_label(ui, "Other side");
        let mut flip = path_now.flip;
        if ui.checkbox(&mut flip, "").on_hover_text("Puts the text on the other side of the line, reading the other way").changed() {
            let new = with_path(TextPath { flip, ..path_now.clone() });
            click_edit(state, target, spec, &new, actions);
        }
        ui.end_row();
    });
    if let PathShape::Curve { points } = &path_now.shape {
        ui.horizontal(|ui| {
            let add = ui.add_enabled(points.len() < MAX_PATH_POINTS, egui::Button::new("Add point")).on_hover_text("Adds a point in the middle of the longest piece");
            if add.clicked() {
                let new = with_path(TextPath { shape: PathShape::Curve { points: add_point(points) }, ..path_now.clone() });
                click_edit(state, target, spec, &new, actions);
            }
            let remove = ui.add_enabled(points.len() > 2, egui::Button::new("Remove point")).on_hover_text("Removes the last point");
            if remove.clicked() {
                let mut fewer = points.clone();
                fewer.pop();
                let new = with_path(TextPath { shape: PathShape::Curve { points: fewer }, ..path_now.clone() });
                click_edit(state, target, spec, &new, actions);
            }
        });
    }
    if kind == Kind::Circle
        && let TextTarget::Stamp(_) = target
    {
        fill = current_frame(state, target, spec).map_or(0.0, |f| f.circle_fill);
    }
    let hint = match kind {
        Kind::Circle if fill > 1.0 => None,
        Kind::Arc => Some("The text follows an arc whose length is the text's own."),
        Kind::Circle => Some("The circle keeps its size whatever is typed."),
        Kind::Curve => Some("Drag the round points in the view with the Transform tool."),
        Kind::Straight => None,
    };
    match hint {
        Some(h) => {
            ui.weak(h);
        }
        None if kind == Kind::Circle => {
            ui.colored_label(ui.visuals().warn_fg_color, CIRCLE_TOO_SHORT);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_follow_the_laid_out_characters() {
        let lines = strip_boxes("WTE 2026\nab\u{1}c\te\u{301}");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 8);
        assert_eq!(lines[0][3].text, "·");
        let second: Vec<(u32, &str)> = lines[1].iter().map(|b| (b.last, b.text.as_str())).collect();
        assert_eq!(second, [(9, "a"), (10, "b"), (12, "c"), (13, "→"), (15, "e\u{301}")]);
    }

    #[test]
    fn slot_texts_name_the_real_pair() {
        let slot = gaps::GapSlot { after: 2, left: 'E', right: Some(' '), em: 0.3 };
        assert_eq!(slot_text(&slot), "Gap between E and (space): 30 %");
        let end = gaps::GapSlot { after: 6, left: '6', right: None, em: 0.305 };
        assert_eq!(slot_text(&end), "Stored gap after 6: 30.5 % (no effect at the end of a line)");
    }

    #[test]
    fn add_point_splits_the_longest_piece() {
        let pts = [[0.0, 0.0], [1.0, 0.0], [4.0, 0.0]];
        assert_eq!(add_point(&pts), vec![[0.0, 0.0], [1.0, 0.0], [2.5, 0.0], [4.0, 0.0]]);
        let full: Vec<[f32; 2]> = (0..MAX_PATH_POINTS).map(|i| [i as f32, 0.0]).collect();
        assert_eq!(add_point(&full).len(), MAX_PATH_POINTS);
    }
}
