// objects-tabs (WP4, OBJECTS_TABS_PLAN section 6; ARCHITECTURE.md 8.7 "Side panel"): the right side panel as two tab
// groups with a draggable split between them.
//
// - Upper group: the strip Tool | Color | Texture sets | Objects and the page of the shown tab (`state.upper_tab`), each
//   page in its own vertical scroll area with thin visible bars. All four tabs are always available; a tool change
//   never writes `upper_tab` (two exceptions: the Text tool's focus request shows the Tool tab, a click on the
//   toolbar's primary colour chip shows the Color tab).
// - Lower group: the strip Layers | Brush | Text (the round-4 one tab rule of `shown_tab`) in a resizable bottom panel;
//   its top edge is the split. Until the user drags it, the group hugs its rows with the Layers tab and has the
//   computed height `tab_area_height` with the Brush or the Text tab. A press on the edge makes the size the user's
//   (kept for every lower tab, clamped to `layers_area_max`); a double-click on it returns to automatic.
//
// Guarantees (tests in control_sweep/panel_tests.rs and the scenarios' `assert_side_panel_layout`): at 1280x800 and
// above with the split automatic, G1 the whole Color page is on screen with a 140 pt square, G2 the five brush slider
// rows are inside the Tool page with the Layers and with the Brush tab below, G3 the Layers footer and Delete stay
// below the list inside the panel, G4 the four upper tab buttons fit in one row at 300 pt and the groups never overlap.

use eframe::egui;
use egui::{Rect, vec2};

use crate::panels::{PanelState, brush_tab, color, layers, objects, shown_tab, text_tab, texture_sets, tool_options};
use crate::panels::{brush_tab_enabled, text_tab_enabled};
use crate::state::{AppState, PanelHit, SideRects, SideTab, ToolKind, UpperTab};

/// Height of the tab strips of both groups (FEATURE_BRUSH 6.2).
pub(crate) const TAB_STRIP_HEIGHT: f32 = 24.0;
/// Height of the Tool page's heading and the five Brush slider rows (FEATURE_BRUSH 6.1, re-measured for the
/// header-less Tool page of objects-tabs WP4).
pub(crate) const BRUSH_OPTIONS_HEIGHT: f32 = 144.0;
/// Limits of the computed height of the lower group while the Brush or the Text tab is shown (FEATURE_BRUSH 6.1).
pub(crate) const TAB_AREA_MIN: f32 = 160.0;
pub(crate) const TAB_AREA_MAX: f32 = 420.0;
/// Smallest height of the lower group: tab strip, opacity row, two whole rows and the two-line footer (at 640x480 the
/// Layers tab gets this).
pub(crate) const LAYERS_MIN_HEIGHT: f32 = 200.0;
/// Height kept for the page of the upper group (about 96 pt of it stay visible once the spacing is taken off).
pub(crate) const SETTINGS_MIN_HEIGHT: f32 = 112.0;
/// Smallest height of the upper group: its strip and the page minimum. The split never takes more from it.
pub(crate) const UPPER_MIN_HEIGHT: f32 = TAB_STRIP_HEIGHT + SETTINGS_MIN_HEIGHT;
/// Room the Color page needs beyond its content (6 pt of spacing plus 2 pt of slack: a page exactly as tall as its
/// content could show the scroll bar on a rounding difference, which narrows the swatches, which shortens the content,
/// which hides the bar again — a layout that flips every frame).
pub(crate) const COLOR_AREA_MARGIN: f32 = 8.0;
/// Largest and smallest side of the colour picker's saturation/value square (= the width of its bars).
pub(crate) const COLOR_SQUARE_MAX: f32 = 140.0;
pub(crate) const COLOR_SQUARE_MIN: f32 = 72.0;
/// First-frame guesses of measured heights (corrected after one frame): the Color page without its square (measured
/// headless at 1280x800: 143.5 pt; `the_colour_page_guess_matches_its_measured_height`), the Layers footer.
pub(crate) const COLOR_FIXED_HEIGHT_GUESS: f32 = 144.0;
pub(crate) const LAYERS_FOOTER_HEIGHT_GUESS: f32 = 52.0;
/// Space between the upper strip and its page.
const UPPER_STRIP_GAP: f32 = 4.0;

/// Default height of the lower group for a side panel of `height` points: about 40 %, at least room for three rows.
pub(crate) fn layers_area_height(height: f32) -> f32 {
    (height * 0.4).clamp(LAYERS_MIN_HEIGHT.min(height * 0.5), 420.0)
}

/// Height of the lower group with the Layers tab before the user sizes it: what its rows and footer need (`natural`,
/// measured in the last frame), at least `LAYERS_MIN_HEIGHT` and at most the default `layers_area_height`; the default
/// until measured.
pub(crate) fn layers_area_fit(height: f32, natural: Option<f32>) -> f32 {
    let max = layers_area_height(height);
    natural.map_or(max, |n| n.clamp(LAYERS_MIN_HEIGHT.min(height * 0.5).min(max), max))
}

/// Largest height of the lower group: 85 % of the panel, and never so much that the upper group loses its minimum (a
/// split dragged in a large window keeps its size when the window shrinks, clamped to this).
pub(crate) fn layers_area_max(height: f32) -> f32 {
    (height * 0.85).min(height - UPPER_MIN_HEIGHT).max(LAYERS_MIN_HEIGHT)
}

/// What the upper group keeps while the lower group shows the Brush or the Text tab: its strip, and a page as tall as
/// the taller of the five brush slider rows and the whole Color page (`color_content`, measured; so G1 and G2 hold
/// whichever upper tab is shown, and switching upper tabs never moves the split).
pub(crate) fn upper_keep(color_content: f32) -> f32 {
    let color = if color_content.is_finite() { color_content + COLOR_AREA_MARGIN } else { 0.0 };
    TAB_STRIP_HEIGHT + UPPER_STRIP_GAP + BRUSH_OPTIONS_HEIGHT.max(color)
}

/// Height of the lower group while the Brush or the Text tab is shown and the split is automatic, for a side panel of
/// `height` points of which the upper group keeps `keep` (`upper_keep`): the rest, limited to `160..=420` and to
/// `layers_area_max`.
pub(crate) fn tab_area_height(height: f32, keep: f32) -> f32 {
    let wanted = if keep.is_finite() { height - keep } else { layers_area_height(height) };
    wanted.clamp(TAB_AREA_MIN.min(height * 0.5), TAB_AREA_MAX).min(layers_area_max(height))
}

/// Side of the colour picker's square: as large as the panel width allows (at most `COLOR_SQUARE_MAX`) and as the
/// page's `room` allows given the rest of the page (`fixed`), never below `COLOR_SQUARE_MIN` (then the page scrolls).
pub(crate) fn color_square_side(width: f32, room: f32, fixed: f32) -> f32 {
    let by_width = (width - 24.0).clamp(COLOR_SQUARE_MIN, COLOR_SQUARE_MAX);
    (room - fixed).floor().clamp(COLOR_SQUARE_MIN, by_width)
}

/// Thin scroll bars that stay visible while the content overflows (egui's default floating bars are invisible until
/// hovered, so a cut-off page gave no cue that it scrolls).
pub(crate) fn with_visible_scroll_bars(ui: &mut egui::Ui) {
    ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
}

/// The side panel (see the head of this file). Records `state.panel_rects` and `state.side_rects` for the self-test.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    ps.begin_frame(state);
    state.panel_rects.clear();
    // The Text tool asks for the keyboard focus in its content field: that field is on the Tool page (6.4 exception 1).
    // A request left over from another tool is dropped (the Tool page would drop it too, but may not be shown).
    if state.focus_text_content {
        if state.tool == ToolKind::Text {
            state.upper_tab = UpperTab::Tool;
        } else {
            state.focus_text_content = false;
        }
    }
    tool_options::sync_wrong_view_status(state);
    let mut rects = SideRects { panel: ui.max_rect(), ..SideRects::default() };
    let shown = shown_tab(state);
    if shown != SideTab::Text {
        text_tab::clear_gap_status(state); // T6 (round 4): the status line of a selected gap goes with the Text tab
    }
    let height = ui.available_height().max(1.0);
    split_input(ui, ps);
    lower_group(ui, state, ps, shown, height, &mut rects);
    upper_group(ui, state, ps, &mut rects);
    state.side_rects = rects;
    ps.end_frame(ui.ctx(), state);
}

/// The split edge (the lower group's top edge): a press on it makes the size the user's; a double-click on it returns
/// to automatic. Read from the raw pointer input: the panel's own resize handle takes the drag.
fn split_input(ui: &egui::Ui, ps: &mut PanelState) {
    let Some(area) = ps.layers_area_rect else { return };
    let grab = ui.style().interaction.resize_grab_radius_side + 3.0;
    let on_edge = |p: egui::Pos2| (p.y - area.top()).abs() <= grab && area.x_range().contains(p.x);
    let (pressed, double) = ui.input(|i| {
        let pressed = i.pointer.primary_pressed() && i.pointer.press_origin().is_some_and(on_edge);
        let double = i.pointer.button_double_clicked(egui::PointerButton::Primary) && i.pointer.interact_pos().is_some_and(on_edge);
        (pressed, double)
    });
    if double {
        ps.layers_user_sized = false;
        ps.split_start = None;
    } else if pressed {
        ps.layers_user_sized = true;
    }
}

/// The lower group: today's resizable bottom panel with the strip Layers | Brush | Text.
fn lower_group(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, shown: SideTab, height: f32, rects: &mut SideRects) {
    let min = LAYERS_MIN_HEIGHT.min(height * 0.5);
    let panel = egui::Panel::bottom("side_layers_area")
        .resizable(true)
        .frame(egui::Frame::new().inner_margin(egui::Margin { top: 4, ..egui::Margin::ZERO }));
    let panel = if ps.layers_user_sized {
        let start = ps.split_start.unwrap_or_else(|| layers_area_height(height));
        panel.default_size(start).size_range(min..=layers_area_max(height))
    } else if shown == SideTab::Layers {
        // Until the user sizes it, the group hugs its rows and footer, at most its default (40 %, R3-10 review): the
        // room a short list does not need goes to the upper page.
        panel.exact_size(layers_area_fit(height, ps.layers_natural_height))
    } else {
        // The Brush and the Text tab: the upper group keeps its strip and the taller of the five brush slider rows and
        // the whole Color page (G1, G2), the tab gets the rest.
        panel.exact_size(tab_area_height(height, upper_keep(ps.color_content_height)))
    };
    ps.layers_list_overflow = 0.0;
    ps.layers_content_bottom = None;
    let layers_area = panel.show(ui, |ui| {
        // A panel's size is the size of its content: the area keeps its full height even when the list (which
        // shrinks to its rows) and the footer need less, or it would shrink a little every frame.
        ui.set_min_height(ui.available_height());
        with_visible_scroll_bars(ui);
        rects.tab_buttons = lower_tab_strip(ui, state, shown);
        match shown {
            SideTab::Layers => layers::show(ui, state, ps),
            SideTab::Brush => {
                rects.brush_tab = Some(tab_content(ui, "side_brush_tab_scroll", |ui| brush_tab::show(ui, state, ps)));
                rects.brush_tab_content = ps.brush.content;
            }
            SideTab::Text => rects.text_tab = Some(tab_content(ui, "side_text_tab_scroll", |ui| text_tab::show(ui, state, ps))),
        }
    });
    let area = layers_area.response.rect;
    rects.bottom_area = area;
    state.panel_rects.insert(PanelHit::LayersArea, area);
    ps.layers_area_rect = Some(area);
    if let Some(bottom) = ps.layers_content_bottom {
        // 2 pt of slack: an area exactly as tall as its rows could flip the list's scroll bar on a rounding difference.
        let natural = (bottom - area.top() + ps.layers_list_overflow + 2.0).ceil();
        if ps.layers_natural_height.is_none_or(|n| (n - natural).abs() > 0.5) {
            ui.ctx().request_repaint();
        }
        ps.layers_natural_height = Some(natural);
    }
}

/// The lower strip: one row of three selectable buttons. A tab that cannot be used is greyed out with a hover text that
/// says why. A click writes `state.side_tab` directly (a setting, not an action). Returns the buttons' rects (indexed
/// by `SideTab::index`).
fn lower_tab_strip(ui: &mut egui::Ui, state: &mut AppState, shown: SideTab) -> [Rect; 3] {
    let mut rects = [Rect::NOTHING; 3];
    ui.horizontal(|ui| {
        ui.set_min_height(TAB_STRIP_HEIGHT);
        for tab in SideTab::ALL {
            let (enabled, why) = match tab {
                SideTab::Layers => (true, ""),
                SideTab::Brush => (brush_tab_enabled(state), "Select the Brush or the Eraser"),
                SideTab::Text => (text_tab_enabled(state), "Choose the Text tool, or select a text with the Transform tool"),
            };
            let button = egui::Button::selectable(shown == tab, tab.label()).min_size(vec2(0.0, TAB_STRIP_HEIGHT - 4.0));
            let resp = ui.add_enabled(enabled, button).on_disabled_hover_text(why);
            if resp.clicked() {
                state.side_tab = tab;
            }
            rects[tab.index()] = resp.rect;
        }
    });
    rects
}

/// The content of the Brush or the Text tab in its own scroll area (the tab is taller than the area in a small
/// window); returns the visible rect of the content.
fn tab_content(ui: &mut egui::Ui, salt: &'static str, add: impl FnOnce(&mut egui::Ui)) -> Rect {
    let out = egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(ui, add);
    out.inner_rect
}

/// The upper strip: four selectable buttons, always enabled; a click writes `state.upper_tab` (a setting). Returns the
/// buttons' rects (indexed by `UpperTab::index`).
fn upper_tab_strip(ui: &mut egui::Ui, state: &mut AppState) -> [Rect; 4] {
    let mut rects = [Rect::NOTHING; 4];
    ui.horizontal(|ui| {
        ui.set_min_height(TAB_STRIP_HEIGHT);
        for tab in UpperTab::ALL {
            let button = egui::Button::selectable(state.upper_tab == tab, tab.label()).min_size(vec2(0.0, TAB_STRIP_HEIGHT - 4.0));
            let resp = ui.add(button);
            if resp.clicked() {
                state.upper_tab = tab;
            }
            rects[tab.index()] = resp.rect;
        }
    });
    rects
}

/// A page of the upper group in its own scroll area; returns the scroll output.
fn page<R>(ui: &mut egui::Ui, salt: &'static str, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
    // No minimum height (accept_fixes item B): in a short window the page takes only the room that is left.
    egui::ScrollArea::vertical().id_salt(salt).min_scrolled_height(0.0).auto_shrink([false, false]).show(ui, add)
}

/// The upper group: the strip and the page of `state.upper_tab` in the room the lower group left.
fn upper_group(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, rects: &mut SideRects) {
    rects.upper_area = ui.available_rect_before_wrap();
    with_visible_scroll_bars(ui);
    // The pages' bars lie over the content's right edge instead of taking 6 pt of its width. egui reserves a bar's room
    // only from the frame after it first shows it, and the option grids keep their column widths from the frame before,
    // so a page that started to overflow (a lower tab, a tool or a selection that leaves it less room than its content)
    // grew the side panel by 6 pt, for good (egui keeps a side panel's width): the `text` and `text_tab` scenarios had
    // a 306 / 312 pt panel. The thin bar stays visible while the page overflows.
    ui.spacing_mut().scroll.floating_allocated_width = 0.0;
    rects.upper_tab_buttons = upper_tab_strip(ui, state);
    for tab in UpperTab::ALL {
        state.panel_rects.insert(PanelHit::UpperTabButton(tab), rects.upper_tab_buttons[tab.index()]);
    }
    ui.add_space(UPPER_STRIP_GAP - ui.spacing().item_spacing.y);
    match state.upper_tab {
        UpperTab::Tool => {
            let out = page(ui, "side_settings_scroll", |ui| tool_options::show(ui, state, ps));
            state.panel_rects.insert(PanelHit::SettingsArea, out.inner_rect);
            rects.tool_area = out.inner_rect;
            rects.tool_options = out.inner;
            rects.upper_page = out.inner_rect;
        }
        UpperTab::Color => {
            // The square as large as the page allows given the rest of the page (measured in the last frame).
            let room = ui.available_height() - COLOR_AREA_MARGIN;
            let square = color_square_side(ui.available_width(), room, ps.color_fixed_height);
            let out = page(ui, "side_color_scroll", |ui| color::show(ui, state, ps, square));
            let content = Rect::from_min_size(out.inner_rect.min - out.state.clamped_offset(), out.content_size);
            if (out.content_size.y - ps.color_content_height).abs() > 0.5 {
                ui.ctx().request_repaint();
            }
            ps.color_content_height = out.content_size.y;
            ps.color_fixed_height = (out.content_size.y - out.inner).max(0.0);
            state.panel_rects.insert(PanelHit::ColorSection, out.inner_rect);
            state.panel_rects.insert(PanelHit::ColorContent, content);
            rects.color = out.inner_rect;
            rects.upper_page = out.inner_rect;
        }
        UpperTab::TextureSets => {
            let out = page(ui, "side_sets_scroll", |ui| texture_sets::show(ui, state, ps));
            rects.sets_tab = Some(out.inner_rect);
            rects.upper_page = out.inner_rect;
        }
        UpperTab::Objects => {
            let rect = objects::show(ui, state, ps);
            rects.objects_tab = Some(rect);
            rects.upper_page = rect;
        }
    }
}

/// Copies the side panel's tabs and split into the preferences; called directly before every `prefs::save` (at exit,
/// never in self-test mode).
pub fn sync_prefs(state: &mut AppState, ps: &PanelState) {
    state.prefs.side_upper_tab = state.upper_tab;
    state.prefs.side_lower_tab = state.side_tab.pref_name().to_owned();
    state.prefs.side_split = if ps.layers_user_sized { ps.layers_area_rect.map(|r| r.height()).or(ps.split_start) } else { None };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizing_functions_keep_their_limits() {
        // 1280x800: the side panel is about 750 pt high.
        let h = 750.0;
        assert_eq!(layers_area_height(h), 300.0);
        assert_eq!(layers_area_fit(h, None), 300.0);
        assert_eq!(layers_area_fit(h, Some(150.0)), 200.0, "never below the minimum");
        assert_eq!(layers_area_fit(h, Some(500.0)), 300.0, "never above the default");
        assert_eq!(layers_area_max(h), (h - UPPER_MIN_HEIGHT).min(h * 0.85));
        // 640x480: the minimum wins over 85 %.
        assert_eq!(layers_area_max(230.0), LAYERS_MIN_HEIGHT);
        // The upper group keeps the taller of the brush rows and the Color page.
        assert_eq!(upper_keep(100.0), TAB_STRIP_HEIGHT + UPPER_STRIP_GAP + BRUSH_OPTIONS_HEIGHT);
        assert_eq!(upper_keep(300.0), TAB_STRIP_HEIGHT + UPPER_STRIP_GAP + 300.0 + COLOR_AREA_MARGIN);
        assert_eq!(upper_keep(f32::NAN), TAB_STRIP_HEIGHT + UPPER_STRIP_GAP + BRUSH_OPTIONS_HEIGHT);
        let keep = upper_keep(300.0);
        assert_eq!(tab_area_height(h, keep), h - keep);
        assert_eq!(tab_area_height(2000.0, keep), TAB_AREA_MAX);
        assert_eq!(tab_area_height(430.0, keep), TAB_AREA_MIN, "at 640x480 the tab keeps its minimum");
        assert_eq!(tab_area_height(h, f32::INFINITY), layers_area_height(h));
        assert!(tab_area_height(300.0, keep) <= layers_area_max(300.0));
        // The square: full size when the room allows, never below the minimum.
        assert_eq!(color_square_side(284.0, 400.0, 150.0), COLOR_SQUARE_MAX);
        assert_eq!(color_square_side(284.0, 250.0, 150.0), 100.0);
        assert_eq!(color_square_side(284.0, 100.0, 150.0), COLOR_SQUARE_MIN);
        assert_eq!(color_square_side(120.0, 400.0, 150.0), COLOR_SQUARE_MIN.max(96.0));
    }

    #[test]
    fn tab_names_in_the_prefs() {
        for tab in SideTab::ALL {
            assert_eq!(SideTab::from_pref_name(tab.pref_name()), tab);
        }
        assert_eq!(SideTab::from_pref_name("objects"), SideTab::Layers);
        assert_eq!(UpperTab::ALL.map(UpperTab::index), [0, 1, 2, 3]);
        assert_eq!(UpperTab::default(), UpperTab::Tool);
        assert_eq!(serde_json::to_value(UpperTab::TextureSets).unwrap(), "texture_sets");
    }
}
