// Owner: T6 text panels (round 4). Written by the scaffold S2 as a stub, filled by T6.
//
// The font picker (FEATURE_TEXT 6.3): a popup under the font button (300 pt wide, at most 360 pt high, closed by a
// click outside or Esc) with a search field, the families used in the open project ("In this project", most recently
// added first, at most 8) and every family of the catalog ("All fonts") with its sample (`AppState::font_samples`,
// loaded once per catalog through `AppAction::LoadFontSamples`, packed by `text_ops.rs`). Rows are 22 pt high and
// drawn with `ScrollArea::show_rows`, so only the visible rows are laid out. Up and Down move the highlight, Enter
// takes it, a click takes a row. A popup is not part of the panel layout, so it cannot widen the panel.
//
// The picker hands back a family name; `text_block::change_family` resolves the face (TEXT 5.9) and the font
// (`resolve_font`: the project first, so a font that is only embedded in the project can be given to another text).
//
// Round 5 (R5-3, the slow first open): the samples are asked for by the font button, once per catalog, so the sample
// job never starts in the frame that opens the popup; the per-family row data (the "N MB" label) is kept in
// `PickerState`, filled a few families per frame by the button and on demand by a row, once per catalog size, so a
// row does no catalog scan once its family is known. The picker's parts are timed as frame-profile segments
// ("picker: button" for the whole button, "picker: popup", "picker: setup" with "picker: search field" in it,
// "picker: project rows", "picker: rows", "picker: row data").

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use egui::{Align2, Color32, Key, Popup, PopupCloseBehavior, Rect, Sense, TextEdit, pos2, vec2};

use crate::action::AppAction;
use crate::app::frame_profile;
use crate::panels::PanelState;
use crate::panels::text_block::{pick_style, project_fonts};
use crate::state::AppState;

/// Width of the popup (TEXT 6.3).
const POPUP_WIDTH: f32 = 300.0;
/// Height of one row.
const ROW_HEIGHT: f32 = 22.0;
/// Height of the scrolling list of "All fonts" (the popup stays within 360 pt).
const LIST_HEIGHT: f32 = 240.0;
/// Most rows under "In this project".
const PROJECT_ROWS: usize = 8;
/// A font file of this size or more shows its size: the whole file goes into the project.
const BIG_FONT_BYTES: u64 = 5 * 1024 * 1024;
/// Height of a sample in the row.
const SAMPLE_HEIGHT: f32 = 16.0;
/// Time the font button spends per frame on filling the row data ahead of the first open.
const PREFILL_BUDGET: Duration = Duration::from_millis(2);

/// The picker's search text and highlighted row.
#[derive(Default)]
pub(crate) struct PickerState {
    pub query: String,
    /// Index into the rows the popup shows (project rows first, then the catalog rows).
    pub highlight: usize,
    /// The popup was opened in this frame: the search field takes the focus.
    pub just_opened: bool,
    /// The catalog size the samples were asked for (one `LoadFontSamples` per catalog).
    pub asked_for: Option<usize>,
    /// Per-family row data of the catalog (round 5 R5-3).
    pub rows: RowData,
    /// Number of catalog rows the popup laid out in the last frame (test counter).
    #[cfg(test)]
    pub rows_drawn: usize,
}

/// Per-family data of the picker's rows, for one catalog size (round 5 R5-3): the "N MB" label of a family whose face
/// (`pick_style` with "Regular") is a font file of `BIG_FONT_BYTES` or more, else none.
#[derive(Default)]
pub(crate) struct RowData {
    /// The catalog size the data was made for; another size starts it again.
    pub catalog: Option<usize>,
    /// Catalog families (in catalog order) the prefill has looked at.
    pub prefilled: usize,
    /// Family -> its size label (`None`: no label).
    pub labels: HashMap<String, Option<String>>,
}

impl RowData {
    /// Starts again when the catalog has another size (the system scan arrived, a font file was loaded).
    fn sync(&mut self, families: usize) {
        if self.catalog != Some(families) {
            *self = RowData { catalog: Some(families), ..RowData::default() };
        }
    }

    /// The size label of `family`, looked up in the catalog on its first use.
    fn label(&mut self, fonts: &wte_core::FontCatalog, family: &str) -> Option<&str> {
        if !self.labels.contains_key(family) {
            self.labels.insert(family.to_owned(), size_label(fonts, family));
        }
        self.labels.get(family).and_then(|l| l.as_deref())
    }

    /// Looks at the next families of the catalog until `budget` is spent; true when every family is known.
    fn prefill(&mut self, fonts: &wte_core::FontCatalog, budget: Duration) -> bool {
        let start = Instant::now();
        let families = fonts.families();
        while self.prefilled < families.len() {
            let family = &families[self.prefilled];
            self.prefilled += 1;
            if !self.labels.contains_key(family) {
                self.labels.insert(family.clone(), size_label(fonts, family));
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        self.prefilled >= families.len()
    }
}

/// The "N MB" label of `family`: its face of `pick_style` with "Regular" is a font file of `BIG_FONT_BYTES` or more
/// (the whole file goes into the project).
fn size_label(fonts: &wte_core::FontCatalog, family: &str) -> Option<String> {
    let style = pick_style(fonts, family, "Regular");
    let face = fonts.find(family, &style)?;
    (face.file_bytes >= BIG_FONT_BYTES).then(|| format!("{} MB", face.file_bytes / (1024 * 1024)))
}

/// Asks for the samples of the catalog once per catalog (round 5 R5-3: from the font button, which is drawn whenever
/// the Text block is, so the sample job never starts in the frame that opens the popup).
fn ask_for_samples(state: &mut AppState, picker: &mut PickerState) {
    let families = state.fonts.families().len();
    if !state.fonts_loading
        && !state.font_samples.loading
        && state.font_samples.requested_for != Some(families)
        && picker.asked_for != Some(families)
    {
        picker.asked_for = Some(families);
        state.push(AppAction::LoadFontSamples);
    }
}

/// Whether `family` matches the search text `query` (FEATURE_TEXT 5.13: a case-insensitive substring; the empty query
/// matches every family).
pub(crate) fn matches(family: &str, query: &str) -> bool {
    family.to_lowercase().contains(&query.trim().to_lowercase())
}

/// The families that match `query`: those that start with it first, then the others, each group in list order.
pub(crate) fn filtered<'a>(families: &'a [String], query: &str) -> Vec<&'a String> {
    let q = query.trim().to_lowercase();
    let (mut first, mut rest): (Vec<&String>, Vec<&String>) = (Vec::new(), Vec::new());
    for family in families {
        let lower = family.to_lowercase();
        if lower.starts_with(&q) {
            first.push(family);
        } else if lower.contains(&q) {
            rest.push(family);
        }
    }
    first.extend(rest);
    first
}

/// The font button of the Text block (row 2) and its picker. `warn` draws the name in the warning colour (a font that
/// is neither installed nor in the project), `why` is then its hover text. Returns the family the user took.
pub(crate) fn button(
    ui: &mut egui::Ui,
    state: &mut AppState,
    ps: &mut PanelState,
    family: &str,
    width: f32,
    warn: bool,
    why: &str,
) -> Option<String> {
    let mut text = egui::RichText::new(family);
    if warn {
        text = text.color(ui.visuals().warn_fg_color);
    }
    let _b = frame_profile::scope("picker: button");
    let resp = ui.add_sized([width, 20.0], egui::Button::new(text).truncate());
    let resp = if why.is_empty() { resp.on_hover_text(format!("Font: {family}. Click to choose another font")) } else { resp.on_hover_text(why) };
    state.panel_rects.insert(crate::state::PanelHit::FontButton, resp.rect);
    ask_for_samples(state, &mut ps.text.picker);
    {
        let _p = frame_profile::scope("picker: row data");
        let rows = &mut ps.text.picker.rows;
        rows.sync(state.fonts.families().len());
        if !state.fonts_loading && rows.prefilled < state.fonts.families().len() {
            rows.prefill(&state.fonts, PREFILL_BUDGET);
        }
    }
    let popup_id = Popup::default_response_id(&resp);
    if resp.clicked() && !Popup::is_id_open(ui.ctx(), popup_id) {
        let picker = &mut ps.text.picker;
        picker.query.clear();
        picker.highlight = 0;
        picker.just_opened = true;
    }
    let mut picked = None;
    let _p = frame_profile::scope("picker: popup");
    let _ = Popup::from_toggle_button_response(&resp)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(POPUP_WIDTH)
        .show(|ui| {
            ui.set_width(POPUP_WIDTH - 12.0);
            picked = list(ui, state, ps, family);
        });
    if picked.is_some() {
        Popup::close_id(ui.ctx(), popup_id);
    }
    picked
}

/// One row of the popup: what it shows and what a click takes.
struct Row {
    family: String,
    /// "In this project" and not installed: no sample, "(in project)" instead.
    project_only: bool,
}

/// The content of the popup; returns the family taken by a click or Enter.
fn list(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, current: &str) -> Option<String> {
    let setup = frame_profile::scope("picker: setup");
    // Normally asked for by the button already (a test or a catalog that grew while the popup is open).
    ask_for_samples(state, &mut ps.text.picker);
    ps.text.picker.rows.sync(state.fonts.families().len());
    let picker = &mut ps.text.picker;
    let field = frame_profile::scope("picker: search field");
    let search = ui.add(TextEdit::singleline(&mut picker.query).hint_text("Search fonts").desired_width(f32::INFINITY));
    if std::mem::take(&mut picker.just_opened) {
        search.request_focus();
    }
    drop(field);
    if search.changed() {
        picker.highlight = 0;
    }
    let query = picker.query.clone();

    // "In this project": the families of the text stamps, most recently added first.
    let mut project: Vec<Row> = Vec::new();
    if let Some(doc) = state.doc() {
        for font in project_fonts(doc) {
            if project.len() == PROJECT_ROWS {
                break;
            }
            if !matches(&font.family, &query) || project.iter().any(|r| r.family.eq_ignore_ascii_case(&font.family)) {
                continue;
            }
            let project_only = state.fonts.faces(&font.family).is_empty();
            project.push(Row { family: font.family.clone(), project_only });
        }
    }
    let all = filtered(state.fonts.families(), &query);
    let total = project.len() + all.len();

    let (up, down, enter) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter)));
    let picker = &mut ps.text.picker;
    if total > 0 {
        if down {
            picker.highlight = (picker.highlight + 1).min(total - 1);
        }
        if up {
            picker.highlight = picker.highlight.saturating_sub(1);
        }
        picker.highlight = picker.highlight.min(total - 1);
    }
    let highlight = picker.highlight;
    let mut taken: Option<String> = None;
    if enter && total > 0 {
        taken = Some(if highlight < project.len() { project[highlight].family.clone() } else { all[highlight - project.len()].clone() });
    }
    let moved = up || down;
    drop(setup);

    if !project.is_empty() {
        let _p = frame_profile::scope("picker: project rows");
        ui.weak("In this project");
        for (i, row) in project.iter().enumerate() {
            if font_row(ui, state, &mut ps.text.picker.rows, row, current, i == highlight, moved && i == highlight) {
                taken = Some(row.family.clone());
            }
        }
    }
    let rows_scope = frame_profile::scope("picker: rows");
    ui.weak(format!("All fonts ({} of {})", all.len(), state.fonts.families().len()));
    if state.fonts_loading {
        ui.weak("Loading system fonts...");
    }
    let spacing = ui.spacing().item_spacing.y;
    let offset = project.len();
    let mut drawn = 0;
    let mut scroll = egui::ScrollArea::vertical().id_salt("font_picker_rows").max_height(LIST_HEIGHT).auto_shrink([false, true]);
    if moved && highlight >= offset {
        scroll = scroll.vertical_scroll_offset(((highlight - offset) as f32 * (ROW_HEIGHT + spacing) - LIST_HEIGHT / 2.0).max(0.0));
    }
    let row_data = &mut ps.text.picker.rows;
    let _ = scroll.show_rows(ui, ROW_HEIGHT, all.len(), |ui, range| {
        for i in range {
            drawn += 1;
            let row = Row { family: all[i].clone(), project_only: false };
            if font_row(ui, state, row_data, &row, current, offset + i == highlight, false) {
                taken = Some(row.family.clone());
            }
        }
    });
    drop(rows_scope);
    #[cfg(test)]
    {
        ps.text.picker.rows_drawn = drawn;
    }
    let _ = drawn;
    taken
}

/// One row: the family name in the UI font on the left, the sample in its own face (or the file size, or "(in
/// project)") on the right. Returns true when it was clicked. The size label comes from `rows` (round 5 R5-3).
fn font_row(
    ui: &mut egui::Ui,
    state: &AppState,
    rows: &mut RowData,
    row: &Row,
    current: &str,
    highlighted: bool,
    scroll_to: bool,
) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let selected = row.family.eq_ignore_ascii_case(current);
    resp.widget_info(|| egui::WidgetInfo::selected(Role::Button, true, selected, row.family.as_str()));
    if scroll_to {
        resp.scroll_to_me(None);
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if highlighted || resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, visuals.widgets.hovered.weak_bg_fill);
        } else if selected {
            ui.painter().rect_filled(rect, 2.0, visuals.selection.bg_fill.gamma_multiply(0.5));
        }
        let color = visuals.text_color();
        let font = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui.painter().layout(row.family.clone(), font.clone(), color, rect.width() * 0.55);
        let name_pos = pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * 0.58, rect.max.y))).galley(name_pos, galley, color);
        let right = rect.right() - 4.0;
        if row.project_only {
            ui.painter().text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, "(in project)", font, visuals.weak_text_color());
        } else {
            let mut x = right;
            if let Some(cell) = state.font_samples.cells.get(&row.family)
                && let Some(atlas) = state.font_samples.atlases.get(cell.atlas)
            {
                let [aw, ah] = atlas.size();
                let scale = SAMPLE_HEIGHT / cell.rect.height().max(1.0);
                let w = (cell.rect.width() * scale).min(rect.width() * 0.4);
                let dest = Rect::from_min_size(pos2(x - w, rect.center().y - SAMPLE_HEIGHT / 2.0), vec2(w, SAMPLE_HEIGHT));
                let uv = Rect::from_min_max(
                    pos2(cell.rect.min.x / aw as f32, cell.rect.min.y / ah as f32),
                    pos2(cell.rect.min.x / aw as f32 + (w / scale) / aw as f32, cell.rect.max.y / ah as f32),
                );
                ui.painter().image(atlas.id(), dest, uv, color);
                x -= w + 6.0;
            }
            if let Some(mb) = rows.label(&state.fonts, &row.family) {
                ui.painter().text(pos2(x, rect.center().y), Align2::RIGHT_CENTER, mb, font, Color32::GRAY);
            }
        }
    }
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_puts_families_that_start_with_it_first() {
        let families: Vec<String> = ["Alpha", "Arial", "Bahnschrift", "Hack", "Shabnam", "Ubuntu"].map(String::from).to_vec();
        let found: Vec<&str> = filtered(&families, "ha").into_iter().map(String::as_str).collect();
        assert_eq!(found, ["Hack", "Alpha", "Shabnam"]);
        assert_eq!(filtered(&families, "").len(), 6);
        assert_eq!(filtered(&families, "  ARI ").into_iter().map(String::as_str).collect::<Vec<_>>(), ["Arial"]);
        assert!(matches("Arial", "RIA"));
        assert!(!matches("Arial", "x"));
    }

    /// Round 5 R5-3: the font button asks for the samples once per catalog, before the picker is ever opened; opening
    /// asks for nothing more; the row data is complete before the first open; a catalog of another size is asked for
    /// (and its row data made) once more.
    #[test]
    fn the_samples_are_asked_for_once_per_catalog_by_the_button() {
        use crate::control_sweep::{Drive, PanelApp, sample_state};
        use crate::state::{PanelHit, ToolKind};
        let asks = |actions: &[AppAction]| actions.iter().filter(|a| matches!(a, AppAction::LoadFontSamples)).count();
        let mut app = PanelApp::new(sample_state(wte_core::SampleKind::Sphere));
        assert_eq!(asks(&app.take()), 0, "no Text block, no font button");
        app.state.tool = ToolKind::Text;
        app.idle(3);
        assert_eq!(asks(&app.take()), 1, "the button asks once, before any open");
        assert!(!Popup::is_any_open(&app.h.ctx));
        let families = app.state.fonts.families().len();
        let rows = &app.ps.text.picker.rows;
        assert_eq!((rows.catalog, rows.prefilled, rows.labels.len()), (Some(families), families, families), "row data made");
        // Opening the picker asks for nothing more.
        let rect = app.state.panel_rects[&PanelHit::FontButton];
        app.click_at(rect.center());
        app.idle(2);
        assert!(Popup::is_any_open(&app.h.ctx), "the picker is open");
        assert_eq!(asks(&app.take()), 0, "opening asks for nothing");
        app.press(Key::Escape, egui::Modifiers::NONE);
        app.idle(2);
        assert_eq!(asks(&app.take()), 0);
        // The catalog grows (a font file is loaded; the machine's Arial, skipped without it): asked once more.
        let arial = std::path::Path::new(r"C:\Windows\Fonts\arial.ttf");
        if !arial.is_file() || app.state.fonts.load_font_file(arial).is_err() {
            eprintln!("SKIP the second catalog: no {}", arial.display());
            return;
        }
        let grown = app.state.fonts.families().len();
        assert!(grown > families);
        app.idle(3);
        assert_eq!(asks(&app.take()), 1, "a catalog of another size is asked for once");
        let rows = &app.ps.text.picker.rows;
        assert_eq!((rows.catalog, rows.prefilled), (Some(grown), grown));
    }

    /// The size label: none for the bundled fonts (small files); the prefill stops at its budget and goes on in the
    /// next call; a label looked up on demand is the same as a prefilled one.
    #[test]
    fn the_row_data_is_filled_within_its_budget() {
        let fonts = wte_core::FontCatalog::bundled();
        let n = fonts.families().len();
        let mut rows = RowData::default();
        rows.sync(n);
        assert!(!rows.prefill(&fonts, Duration::ZERO) || n <= 1, "a zero budget looks at one family per call");
        assert_eq!(rows.prefilled, 1.min(n));
        while !rows.prefill(&fonts, Duration::ZERO) {}
        assert_eq!(rows.labels.len(), n);
        assert!(rows.labels.values().all(Option::is_none), "the bundled fonts are small files");
        let mut lazy = RowData::default();
        lazy.sync(n);
        for family in fonts.families() {
            assert_eq!(lazy.label(&fonts, family), rows.labels[family].as_deref());
        }
        // Another catalog size starts again.
        rows.sync(n + 1);
        assert_eq!((rows.catalog, rows.prefilled, rows.labels.len()), (Some(n + 1), 0, 0));
    }
}
