// Owner: app-panels (A4); round 4: B4 brush panels. The Tool options section of the side panel (ARCHITECTURE.md
// 8.7): the settings of `state.tool` (changed directly in `AppState`) and, with the Transform tool, the editable
// properties of the selected stamp (as `UpdateStamp*` / `RenameLayer` / `ReplaceStampImage` / `DeleteStamp` actions;
// `Commit::Preview` while a control is dragged or typed in, `Commit::Final` when it is released or loses the focus).
//
// Round 4 (scaffold S2, ROUND4_PLAN 2.2 item 12): the text options live in `text_block.rs` (T6). This file calls
// `text_block::target(state)` and `text_block::show(ui, state, ps, target, actions)` only; the order is the Text block
// first, then the wrong-view hint, then the "Selected: ... Edit" row; under the Transform tool the Text block comes
// before Name, Tint and the placement fields. The "Same side only" checkbox is gone (the Brush tab's Type replaces it).

use eframe::egui;
use egui::color_picker::{Alpha, color_edit_button_srgba};
use egui::{DragValue, Sense, Slider, SliderClamping, TextEdit, vec2};
use wte_core::{FillScope, Placement, Projector, StampObject, StampSource, StylePreset, StyleStack, SurfaceMode, WrapOptions, WrapStats};

use crate::action::AppAction;
use crate::panels::text_block;
use crate::panels::toolbar::tool_name;
use crate::panels::{PanelState, SOURCE_THUMB_SIDE, continuous_edit, from_color32, paint_thumb, percent_slider, to_color32};
use crate::shortcuts::shortcut_text;
use crate::state::{AppState, BrushSettings, PanelHit, StampSettings, StampToolSource, ToolKind, ViewMode};

// Round 6 (R6-2, ROUND6_PLAN 5.1): the hover texts of the "On the model: Project | Wrap" selector and of the two Wrap
// options.
pub(crate) const PROJECT_HOVER: &str = "Like a projector from the front: sharp from this side, stretched at the sides";
pub(crate) const WRAP_HOVER: &str = "Lies on the surface like a sticker: goes round the object and over bumps";
pub(crate) const WRAP_ONLY_HOVER: &str = "Used in Wrap mode";
pub(crate) const JUMP_HOVER: &str =
    "Continue onto separate parts closer than 1 % of the model size; off: stop at the edge of a part";
pub(crate) const ENDS_MEET_HOVER: &str = "Ends meet: the middle part shows, the ends meet at the back";
pub(crate) const OVERLAP_HOVER: &str = "Overlap: keeps winding like tape, the end on top";
/// Hover text of "Project through" while it is greyed out (Wrap).
const THROUGH_WRAP_HOVER: &str = "Used in Project mode: a wrapped stamp lies on the surface itself";
/// objects-tabs (8.6): the Navigate options' line on a model with two or more objects.
pub(crate) const NAVIGATE_PICK_HINT: &str = "Click an object to select it; Ctrl- or Shift-click adds or removes it.";

/// The selector "On the model: Project | Wrap" (round 6). Returns the mode clicked when it differs from `mode`.
pub(crate) fn mode_selector(ui: &mut egui::Ui, mode: SurfaceMode) -> Option<SurfaceMode> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label("On the model:");
        for (m, text, hover) in [(SurfaceMode::Project, "Project", PROJECT_HOVER), (SurfaceMode::Wrap, "Wrap", WRAP_HOVER)] {
            if ui.add(egui::Button::selectable(mode == m, text)).on_hover_text(hover).clicked() && mode != m {
                picked = Some(m);
            }
        }
    });
    picked
}

/// The two options of Wrap (round 6): the checkbox "Jump small gaps" and the selector "Longer than the way round:
/// Ends meet | Overlap". Shown in Project mode too, greyed out ("Used in Wrap mode"). `probe` records whether they
/// could be used in this frame (`ToolProbe::wrap_controls`). Returns the options when a control changed them.
pub(crate) fn wrap_option_controls(
    ui: &mut egui::Ui,
    mode: SurfaceMode,
    opts: WrapOptions,
    probe: &mut Option<bool>,
) -> Option<WrapOptions> {
    let wrap = mode == SurfaceMode::Wrap;
    *probe = Some(wrap && ui.is_enabled());
    let mut out = opts;
    ui.add_enabled_ui(wrap, |ui| {
        ui.checkbox(&mut out.jump_gaps, "Jump small gaps").on_hover_text(JUMP_HOVER).on_disabled_hover_text(WRAP_ONLY_HOVER);
        // Check:B: the label on its own row; side by side the row was wider than the panel and "Overlap" wrapped alone
        // onto the next line.
        ui.label("Longer than the way round:").on_disabled_hover_text(WRAP_ONLY_HOVER);
        ui.horizontal(|ui| {
            let choices = [(false, "Ends meet", ENDS_MEET_HOVER), (true, "Overlap", OVERLAP_HOVER)];
            for (overlap, text, hover) in choices {
                let resp = ui.add(egui::Button::selectable(opts.overlap == overlap, text));
                if resp.on_hover_text(hover).on_disabled_hover_text(WRAP_ONLY_HOVER).clicked() {
                    out.overlap = overlap;
                }
            }
        });
    });
    (out != opts).then_some(out)
}

/// The surface options of the Stamp and the Text tool (shared, `StampSettings`, round 6): the mode selector, the two
/// Wrap options and "Project through" (greyed out in Wrap; `through_hover` is its hover text).
pub(crate) fn tool_surface_options(ui: &mut egui::Ui, st: &mut StampSettings, probe: &mut Option<bool>, through_hover: &str) {
    if let Some(mode) = mode_selector(ui, st.mode) {
        st.mode = mode;
    }
    if let Some(wrap) = wrap_option_controls(ui, st.mode, st.wrap, probe) {
        st.wrap = wrap;
    }
    let project = st.mode == SurfaceMode::Project;
    ui.add_enabled(project, egui::Checkbox::new(&mut st.project_through, "Project through"))
        .on_hover_text(through_hover)
        .on_disabled_hover_text(THROUGH_WRAP_HOVER);
}

/// The lines under the selector of a selected wrapped stamp (round 6): how much surface it covers and what happened
/// (from `Document::stamp_wrap_stats`; `None` = not computed yet: its bake or the parameterization is on its way).
fn wrap_stats_lines(ui: &mut egui::Ui, stats: Option<WrapStats>, opts: WrapOptions) {
    let Some(s) = stats else {
        help(ui, "Wrapping...");
        return;
    };
    help(ui, &format!("Wrapped over {} triangles", s.tris));
    for line in wrap_stats_texts(&s, opts) {
        match line {
            WrapLine::Help(text) => help(ui, &text),
            WrapLine::Warn(text) => {
                ui.add(egui::Label::new(egui::RichText::new(text).color(ui.visuals().warn_fg_color)).wrap());
            }
        }
    }
}

/// Round 9 (package B, T9): the panel line of a 3D stamp whose picture lies on shared texels on at least
/// `UV_SHARE_WARN` of its area (`Document::stamp_shared_texels`); `None` below that or while unknown.
const SHARED_TEXELS_LINE: &str =
    "This model uses the same texture area in several places (mirrored or repeated parts): the picture also shows there.";

fn shared_texels_text(share: Option<f32>) -> Option<&'static str> {
    share.filter(|s| *s >= wte_core::uv_share::UV_SHARE_WARN).map(|_| SHARED_TEXELS_LINE)
}

/// Least `WrapStats::stretch` (95th percentile of |area ratio - 1|) for which a relaxed wrap says how much it stretched.
const STRETCH_LINE_MIN: f32 = 0.05;

/// One line under "Wrapped over N triangles": plain help text or a warning.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WrapLine {
    Help(String),
    Warn(String),
}

/// Least `WrapStats::lost_cut` (share of the picture lost inside the laid outline to cuts) that gets a line.
const LOST_CUT_LINE_MIN: f32 = 0.02;
/// Stable (fix-wrap, item 2): least `WrapStats::gaps` / `WrapStats::at_part` (bare picture between separate parts / where
/// it stops at a neighbouring part) that gets a line; "runs off" then says only the rest, from the same share.
const GAP_LINE_MIN: f32 = 0.02;
/// Least `WrapStats::rejected` (share of the laid picture the owner test drops: it lies on surface under or behind
/// another part) that gets a line.
const HIDDEN_LINE_MIN: f32 = 0.05;
/// Stable (fix_wrap_1): least `WrapStats::beyond` (share of the picture's cells left bare beyond where the ends met)
/// for which the ends line gives the share of the picture's length shown. Below it the spine's closure cut nothing.
pub(crate) const ENDS_BEYOND_MIN: f32 = 0.02;

/// Stable (fix_wrap_1): whether the ends line gives a share (`ends_share` below 95 % and picture left bare beyond the
/// closure), for the panel and step 10's log.
pub(crate) fn ends_cut_shown(s: &WrapStats) -> bool {
    s.ends_share < 0.95 && s.beyond >= ENDS_BEYOND_MIN
}

/// The lines under "Wrapped over N triangles" of `wrap_stats_lines`, in order. Round 8 (R8-1, plan 4.1): a relaxed wrap
/// that stretched by 5 % or more says so (help text: it is the intended behaviour on a curved surface), and seams
/// inside the picture are named. Stable (w1, item 6): each loss has one line that names its own cause, and no two lines
/// contradict each other:
/// - the ends met (`met_itself` / `passed`, Ends meet): "The ends meet at the back" (with the share of the picture's
///   length shown when below 95 %); only a helix (`passed`) says "Longer than the way round"; stable (fix_wrap_1):
///   nothing when the ends share is below 95 % but no picture is left bare beyond the closure (`beyond` < 2 %: the
///   spine looped round a part standing on a surface that holds the picture: ABeautifulGame Jump on);
/// - stable (fix-wrap, item 2): less than 98 % on the surface and 2 % or more of the picture bare in the gaps between
///   separate parts it continued across (`gaps`): "falls into the gaps between separate parts" (help text: the
///   picture looks whole); 2 % or more bare where the picture stops at a neighbouring part (`at_part`): "stops where
///   it meets a neighbouring part";
/// - the picture reaches a boundary edge (`reached_edge`) and less than 98 % lies on the surface: "runs off the
///   surface: an edge, a hole or the end of the part"; stable (fix_wrap_1): "on the surface" is `shown` (each picture
///   cell once) where it was `covered` (laid areas summed: two places laid at one picture point counted twice, so
///   ABeautifulGame Jump on reported 100 % with 9 % of its cells bare past the board's front edge); stable (fix-wrap):
///   its share is the rest (`1 - shown` less `beyond`, `lost_cut`, `gaps` and `at_part`, whether or not their lines
///   show), and no line when the rest is below 2 %;
/// - picture lost inside the laid outline to cuts (`lost_cut` >= 2 %): a cut line;
/// - laid picture dropped because another place owns its picture points (`rejected` >= 5 %: surface under another part
///   with Jump small gaps, or a layout folded over itself, the Lantern's and nonmanifold's single parts): an overlap
///   line (stable fix_wrap_1: it says each spot is shown once, as it is: no loss);
/// - less than 98 % on the surface and none of the above explains it: the shown share; polish item 3: when no cause
///   line above passed its threshold, with the largest cause and its share (`largest_wrap_loss`);
/// - then the round-6 lines (turns, parts, limit).
pub(crate) fn wrap_stats_texts(s: &WrapStats, opts: WrapOptions) -> Vec<WrapLine> {
    let mut out = Vec::new();
    let pct = |x: f32| (x * 100.0).round().max(1.0);
    if s.relaxed && s.stretch >= STRETCH_LINE_MIN {
        out.push(WrapLine::Help(format!("Fitted to the curved surface: stretched up to {:.0} %", pct(s.stretch))));
    }
    if s.seams > 0 {
        out.push(WrapLine::Help("Cut where it cannot lie flat (a corner, a handle, or where its edges meet)".to_owned()));
    }
    // The ends met (Ends meet; Overlap winds the rest on: "Wound N times"). Stable (fix_wrap_1): the spine's ends
    // sum (`ends_share`) says how much of the picture's length the spine laid before it closed; it costs picture only
    // where the cells beyond the closure are bare (`beyond`). A spine that looped round a part standing on a plane
    // meets itself while the plane holds the picture (ABeautifulGame Jump on: ends 74 %, beyond 0): no ends line.
    let ends_cut = ends_cut_shown(s);
    let ends = !opts.overlap && (s.met_itself || s.passed) && (ends_cut || s.ends_share >= 0.95);
    let ends_cut = ends && ends_cut;
    if ends {
        let shown = (s.ends_share * 100.0).round();
        out.push(WrapLine::Warn(match (s.passed, ends_cut) {
            (true, true) => format!("Longer than the way round: {shown:.0} % shown, the ends meet at the back"),
            (false, true) => format!("The ends meet at the back: {shown:.0} % of the picture's length shown"),
            _ => "The ends meet at the back".to_owned(),
        }));
    }
    // Stable (fix-wrap, item 2): the bare picture between separate parts the picture continued across (`gaps`: the
    // picture looks whole at screen size, parts200's 28 %) and where it stops at a neighbouring part (`at_part`:
    // ABeautifulGame Jump on, the board slab's edge at the flush marble frame) have their own lines; "runs off" says
    // the rest.
    let gaps = s.shown < 0.98 && s.gaps >= GAP_LINE_MIN;
    if gaps {
        out.push(WrapLine::Help(format!("{:.0} % of the picture falls into the gaps between separate parts", pct(s.gaps))));
    }
    let at_part = s.shown < 0.98 && s.at_part >= GAP_LINE_MIN;
    if at_part {
        out.push(WrapLine::Warn(format!("{:.0} % of the picture stops where it meets a neighbouring part", pct(s.at_part))));
    }
    let explained = if gaps { s.gaps } else { 0.0 } + if at_part { s.at_part } else { 0.0 };
    // The bare picture no other share names: beyond an edge, a hole or the end of the part (DamagedHelmet Jump on: of
    // its 2.6 % bare, 2.0 % is lost to cuts inside the laid outline and 0.5 % in gaps, so no "3 % runs off" line).
    let off = 1.0 - s.shown - s.beyond - s.lost_cut - s.gaps - s.at_part;
    let edge = s.reached_edge && s.shown < 0.98 && off >= GAP_LINE_MIN;
    if edge {
        out.push(WrapLine::Warn(format!("{:.0} % of the picture runs off the surface: an edge, a hole or the end of the part", pct(off))));
    }
    let cut = s.lost_cut >= LOST_CUT_LINE_MIN;
    if cut {
        // Stable (w2): a picture covering a whole closed part is cut open at its corners (a box cannot lie flat).
        out.push(WrapLine::Warn(if s.wraps_closed {
            format!("{:.0} % of the picture is lost where it is cut open round the closed part's corners", pct(s.lost_cut))
        } else {
            format!("{:.0} % of the picture is lost where the surface had to be cut", pct(s.lost_cut))
        }));
    }
    if s.rejected >= HIDDEN_LINE_MIN {
        // Stable (fix_wrap_1): `rejected` counts the places the owner test drops in cells that have an owner, so each
        // dropped spot is shown at another place: no loss (Avocado Jump on: 35 % dropped, ink 0.996; round 9 / w1 said
        // "... and is not shown there", which read like a loss).
        out.push(WrapLine::Warn(format!(
            "{:.0} % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only",
            pct(s.rejected)
        )));
    }
    // Less on the surface than the ends and the cuts explain, and no edge reached: the shown share alone.
    // Stable (fix_wrap_1): with any ends line (also the plain one at 95 % and more) the ends explain `ends_share`.
    let expected = if ends { s.ends_share } else { 1.0 } - if cut { s.lost_cut } else { 0.0 } - explained;
    if !edge && s.shown < 0.98 && s.shown < expected - 0.02 {
        // Stable (w2): the part is narrower than the picture (`room`): that explains the share.
        out.push(WrapLine::Warn(if s.room < 0.95 {
            format!("The part is narrower than the picture: only {:.0} % of the picture lies on it", (s.shown * 100.0).round())
        } else {
            // Polish item 3: no cause line passed its threshold: name the largest cause with its share (DamagedHelmet
            // Jump on: "..., mostly lost to cuts (2 %)").
            let mut line = format!("Only {:.0} % of the picture lies on the surface", (s.shown * 100.0).round());
            if !(gaps || at_part || cut || ends_cut)
                && let Some((what, share)) = largest_wrap_loss(s, ends)
            {
                line.push_str(&format!(", mostly {what} ({:.0} %)", pct(share)));
            }
            line
        }));
    }
    if s.turns > 1 {
        out.push(WrapLine::Warn(format!("Wound {} times", s.turns)));
    }
    if s.jumps > 0 {
        out.push(WrapLine::Warn(format!("Continued onto {} separate part{}", s.jumps, if s.jumps == 1 { "" } else { "s" })));
    }
    if s.limited {
        out.push(WrapLine::Warn("Too large for this model: the wrap stops at 1 million vertices / 8 turns".to_owned()));
    }
    out
}

/// Below this share a loss is not named as the largest cause of the "Only N %" line.
const LARGEST_LOSS_MIN: f32 = 0.005;

/// Polish item 3: the largest of the losses that make up `1 - shown`, for the "Only N % of the picture lies on the
/// surface" line when none has a line of its own: lost to cuts (`lost_cut`), in the gaps between separate parts
/// (`gaps`), where it meets a neighbouring part (`at_part`), beyond where the picture closed on itself (`beyond`, only
/// when the spine met itself or passed beside itself and no ends line shows, `ends`), and the rest (as in the runs-off line): running off the surface when an edge was reached,
/// else hidden or folded. `None` when the largest is below `LARGEST_LOSS_MIN`.
fn largest_wrap_loss(s: &WrapStats, ends: bool) -> Option<(&'static str, f32)> {
    let rest = 1.0 - s.shown - s.beyond - s.lost_cut - s.gaps - s.at_part;
    let causes = [
        ("lost to cuts", s.lost_cut),
        ("in the gaps between separate parts", s.gaps),
        ("where it meets a neighbouring part", s.at_part),
        ("beyond where the picture closed on itself", if ends || !(s.met_itself || s.passed) { 0.0 } else { s.beyond }),
        (if s.reached_edge { "running off the surface" } else { "hidden or folded" }, rest),
    ];
    causes.into_iter().filter(|c| c.1 >= LARGEST_LOSS_MIN).max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Stable (w1): the texts of `wrap_stats_texts` (the self-test's log of the panel lines).
pub(crate) fn wrap_stats_line_texts(s: &WrapStats, opts: WrapOptions) -> Vec<String> {
    wrap_stats_texts(s, opts).into_iter().map(|l| match l {
        WrapLine::Help(t) | WrapLine::Warn(t) => t,
    }).collect()
}

/// Width of the label column of the option grids.
const LABEL_WIDTH: f32 = 72.0;
/// Upper end of the Depth front / back fields (% of the model size). Corner and wheel scaling of a projected decal
/// scale its depth too (transform.rs), so the value may also lie above it; the field shows it unchanged.
const MAX_DEPTH_PCT: f32 = 1000.0;

/// A `DragValue` bound to a document value: the range limits what the user can drag or type, but an existing value
/// outside it (a UV stamp turned past 180 degrees, a depth scaled past the limit) is shown as it is. egui's default
/// `clamp_existing_to_range(true)` would clamp it on every redraw and report that as a change, which
/// `continuous_edit` would turn into an edit of the document (and, after an Undo of it, into a new one).
fn doc_value(value: &mut f32, range: std::ops::RangeInclusive<f32>) -> DragValue<'_> {
    DragValue::new(value).range(range).clamp_existing_to_range(false)
}

/// A selected stamp that the current view cannot edit (3D stamp in the 2D view or the other way round) says so in the
/// status bar (and on the Tool page), instead of the gizmo silently not appearing. objects-tabs: called by the side
/// panel every frame, whichever upper tab is shown, so the status line never keeps a stale hint.
pub(super) fn sync_wrong_view_status(state: &mut AppState) {
    let stale = |h: &str| h.starts_with(WRONG_VIEW_PREFIX);
    match wrong_view_hint(state) {
        Some((text, _)) => state.status_hint = text,
        None if stale(&state.status_hint) => state.status_hint.clear(),
        None => {}
    }
}

/// Draws the Tool page: the heading "<Tool> options" (a plain label: objects-tabs WP4, the tab is the section) and the
/// options of the active tool, or the properties of the selected stamp. Returns the rect of the page's content (heading
/// and options as laid out) for `AppState::side_rects`.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) -> egui::Rect {
    if state.tool != ToolKind::Text {
        state.focus_text_content = false;
    }
    // Stickers P6a: in the Sticker view the page shows the sticker tool's options.
    if state.view_mode == ViewMode::Sticker {
        return crate::sticker::layers_ui::tool_page(ui, state, ps);
    }
    // Round 6: set again below by whichever options show the two Wrap controls in this frame.
    state.tool_probe.wrap_controls = None;
    let hint = wrong_view_hint(state);
    let text_target = text_block::target(state);
    let out = ui.vertical(|ui| {
        ui.label(egui::RichText::new(format!("{} options", tool_name(state.tool))).strong());
        // The options keep the indent they had under the round-3 collapsing header (every width budget of the Text
        // block and the stamp rows, TEXT 8.2, was set for it).
        ui.indent("side_tool_options", |ui| {
            let mut actions = Vec::new();
            // The Text block first (FEATURE_TEXT 6.1): with the Text tool the text to place, with the Transform tool the
            // selected text (above its Name, Tint and placement fields).
            if let Some(target) = text_target {
                text_block::show(ui, state, ps, target, &mut actions);
                if hint.is_some() {
                    ui.separator();
                }
            }
            if let Some((text, mode)) = &hint {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(ui.visuals().warn_fg_color, text.as_str());
                    let label = if *mode == ViewMode::View3D { "Show 3D view" } else { "Show 2D view" };
                    if ui.button(label).clicked() {
                        actions.push(AppAction::SetViewMode(*mode));
                    }
                });
                ui.separator();
            }
            match state.tool {
                ToolKind::Navigate => {
                    help(ui, "Drag: orbit (3D) or pan (2D). Right-drag: orbit. Middle-drag or Space+drag: pan. Wheel: zoom.");
                    // objects-tabs (8.6): the click that picks an object (models with two or more objects only).
                    if state.doc().is_some_and(|d| d.mesh.object_count() >= 2) {
                        help(ui, NAVIGATE_PICK_HINT);
                    }
                }
                ToolKind::Brush => brush_options(ui, "brush", &mut state.tools.brush, &state.pen.status_line()),
                ToolKind::Eraser => brush_options(ui, "eraser", &mut state.tools.eraser, &state.pen.status_line()),
                ToolKind::Eyedropper => {
                    help(ui, "Click the model or the texture to pick a colour. Holding Alt with the Brush, Eraser or Fill does the same.");
                }
                ToolKind::Fill => {
                    ui.horizontal(|ui| {
                        ui.label("Fill");
                        ui.radio_value(&mut state.tools.fill_scope, FillScope::Island, "UV island")
                            .on_hover_text("Fills the UV island under the cursor (and its seam padding)");
                        ui.radio_value(&mut state.tools.fill_scope, FillScope::Layer, "Whole layer");
                    });
                    help(ui, "Fills the active layer with the primary colour.");
                }
                ToolKind::Stamp => stamp_options(ui, state, ps, &mut actions),
                // The Text block above is the whole of the Text options.
                ToolKind::Text => {}
                ToolKind::Transform => match selected_stamp(state) {
                    Some(stamp) => stamp_properties(ui, state, ps, stamp, &mut actions),
                    None => help(ui, "Click a stamp or a text on the model to select it, or select its layer in the Layers list."),
                },
            }
            // Every tool but Transform, the Text tool included (FEATURE_TEXT 6.2): the selected stamp and the way to edit it.
            if state.tool != ToolKind::Transform
                && let Some(stamp) = selected_stamp(state)
            {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("Selected: {}", stamp.name));
                    let key = shortcut_text(ui.ctx(), "Transform");
                    if ui.button("Edit").on_hover_text(format!("Transform tool ({key})")).clicked() {
                        actions.push(AppAction::SetTool(ToolKind::Transform));
                    }
                });
            }
            // Stickers P6a (STICKER_PLAN 3.1): the selected stamp opens in the Sticker view.
            if let Some(stamp) = selected_stamp(state) {
                edit_sticker_button(ui, state, stamp.id, &mut actions);
            }
            state.actions.extend(actions);
        });
    });
    out.response.rect
}

/// Stickers P6a: "Edit Sticker" for the selected stamp (its rect is `PanelHit::StickerEditButton`).
fn edit_sticker_button(ui: &mut egui::Ui, state: &mut AppState, stamp: wte_core::StampId, actions: &mut Vec<AppAction>) {
    let key = shortcut_text(ui.ctx(), "Sticker");
    let resp = ui
        .button("Edit Sticker")
        .on_hover_text(format!("Open the stamp in the Sticker view: layers, picture and text, with the model beside it ({key})"));
    state.panel_rects.insert(PanelHit::StickerEditButton, resp.rect);
    if resp.clicked() {
        actions.push(AppAction::Sticker(crate::sticker::StickerAction::Enter(Some(stamp))));
    }
}

/// Start of every wrong-view hint (so a stale one in the status bar is recognised and cleared).
const WRONG_VIEW_PREFIX: &str = "This is a ";

/// The hint for a selected stamp that the current view mode cannot edit, and the view mode that can: a projected
/// (3D) stamp in the 2D view, a UV-placed (2D) stamp in the 3D view. Split shows both views, Preview edits nothing.
pub(crate) fn wrong_view_hint(state: &AppState) -> Option<(String, ViewMode)> {
    let stamp = selected_stamp(state)?;
    let text = matches!(stamp.source, StampSource::Text(_));
    let kind = |dim: &str| format!("{WRONG_VIEW_PREFIX}{dim} {}", if text { "text" } else { "stamp" });
    match (stamp.placement, state.view_mode) {
        (Placement::Projected(_), ViewMode::View2D) => Some((format!("{} - edit it in the 3D view", kind("3D")), ViewMode::View3D)),
        (Placement::Uv(_), ViewMode::View3D) => Some((format!("{} - edit it in the 2D view", kind("2D")), ViewMode::View2D)),
        _ => None,
    }
}

/// The selected stamp, if it still exists.
fn selected_stamp(state: &AppState) -> Option<StampObject> {
    let session = state.session.as_ref()?;
    session.selected_stamp.and_then(|id| session.doc.stamp(id)).cloned()
}

fn help(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

fn grid_label(ui: &mut egui::Ui, text: &str) {
    ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(text).truncate());
    });
}

fn brush_options(ui: &mut egui::Ui, salt: &str, b: &mut BrushSettings, pen_line: &str) {
    egui::Grid::new(("brush_grid", salt)).num_columns(2).show(ui, |ui| {
        grid_label(ui, "Size");
        let keys = format!("{} / {}", shortcut_text(ui.ctx(), "Smaller Brush"), shortcut_text(ui.ctx(), "Larger Brush"));
        ui.add(Slider::new(&mut b.size_px, 1.0..=2048.0).logarithmic(true).max_decimals(0).suffix(" px"))
            .on_hover_text(format!("Brush diameter in texels; a brush shape is scaled to fit inside this ring ({keys})"));
        ui.end_row();
        grid_label(ui, "Hardness");
        let keys = format!("{} / {}", shortcut_text(ui.ctx(), "Softer Brush"), shortcut_text(ui.ctx(), "Harder Brush"));
        ui.add(percent_slider(&mut b.hardness, 0.0, 100.0).max_decimals(0)).on_hover_text(keys);
        ui.end_row();
        grid_label(ui, "Opacity");
        ui.add(percent_slider(&mut b.opacity, 1.0, 100.0).max_decimals(0)).on_hover_text("Maximum coverage of one stroke");
        ui.end_row();
        grid_label(ui, "Flow");
        ui.add(percent_slider(&mut b.flow, 1.0, 100.0).max_decimals(0)).on_hover_text("Build-up per dab");
        ui.end_row();
        grid_label(ui, "Spacing");
        ui.add(percent_slider(&mut b.spacing, 2.0, 100.0).max_decimals(0))
            .on_hover_text("Distance between dabs, % of the diameter. A thin shape needs a lower value.");
        ui.end_row();
    });
    pen_pressure_options(ui, salt, b, pen_line);
}

/// Hover text of the "Size" pressure checkbox (branch `pen-input`, PEN_INPUT_PLAN 3.2).
pub(crate) const PEN_SIZE_HOVER: &str = "Press harder for a wider stroke. A mouse always paints at full size.";
/// Hover text of the "Flow" pressure checkbox.
pub(crate) const PEN_FLOW_HOVER: &str = "Press harder for more paint per dab.";

/// The "Pen pressure" group under Flow and Spacing (branch `pen-input`, ARCHITECTURE 8.7): Size + its minimum, Flow +
/// its minimum, and the pen line (`PenState::status_line`). Brush and eraser each have their own.
fn pen_pressure_options(ui: &mut egui::Ui, salt: &str, b: &mut BrushSettings, pen_line: &str) {
    ui.label(egui::RichText::new("Pen pressure").strong());
    egui::Grid::new(("pen_grid", salt)).num_columns(2).show(ui, |ui| {
        ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.checkbox(&mut b.pressure_size, "Size").on_hover_text(PEN_SIZE_HOVER);
        });
        let min_size = percent_slider(&mut b.pressure_size_min, 0.0, 100.0).max_decimals(0).prefix("min ");
        ui.add_enabled(b.pressure_size, min_size).on_hover_text("Min size: the size at the lightest touch, % of the brush size");
        ui.end_row();
        ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.checkbox(&mut b.pressure_flow, "Flow").on_hover_text(PEN_FLOW_HOVER);
        });
        let min_flow = percent_slider(&mut b.pressure_flow_min, 0.0, 100.0).max_decimals(0).prefix("min ");
        ui.add_enabled(b.pressure_flow, min_flow).on_hover_text("Min flow: the flow at the lightest touch, % of the brush flow");
        ui.end_row();
    });
    help(ui, pen_line);
}

fn stamp_options(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>) {
    let has_doc = state.session.is_some();
    // Stickers P6b (STICKER_PLAN 3.6): the source is an image or a tray sticker.
    let generation = state.session.as_ref().map(|s| s.scene_generation);
    state.sticker_ui.pictures.begin(generation);
    if let StampToolSource::Sticker(id) = state.tools.stamp.source
        && state.doc().is_none_or(|d| d.sticker(id).is_none())
    {
        state.tools.stamp.source = StampToolSource::None;
    }
    let source = state.tools.stamp.source;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(SOURCE_THUMB_SIDE, SOURCE_THUMB_SIDE), Sense::hover());
        let thumb = match source {
            StampToolSource::Blob(blob) => ps.source_thumb(ui.ctx(), state, blob),
            StampToolSource::Sticker(id) => crate::sticker::tray::sticker_thumb(ui.ctx(), state, id),
            StampToolSource::None => None,
        };
        let aspect = thumb.map_or(1.0, |(_, [w, h])| w as f32 / h.max(1) as f32);
        paint_thumb(ui.painter(), rect, thumb.map(|(t, _)| t), aspect);
        ui.vertical(|ui| {
            match source {
                StampToolSource::Blob(_) => ui.label(&state.tools.stamp.source_name),
                StampToolSource::Sticker(id) => {
                    let name = state.doc().and_then(|d| d.sticker(id)).map_or_else(String::new, |d| d.name.clone());
                    ui.label(format!("Sticker: {name}"))
                }
                StampToolSource::None => ui.weak("No image chosen"),
            };
            ui.horizontal_wrapped(|ui| {
                let choose = ui.add_enabled(has_doc, egui::Button::new("Choose..."));
                if choose.on_disabled_hover_text("Open a model first").on_hover_text("Choose the image to stamp").clicked() {
                    actions.push(AppAction::ImportStampImage(None));
                }
                let edit = ui
                    .add_enabled(source.is_some(), egui::Button::new("Edit before placing"))
                    .on_hover_text("Open the picture in the Sticker view first: layers, border, shadow")
                    .on_disabled_hover_text("Choose an image or a sticker first");
                state.panel_rects.insert(PanelHit::StampEditBeforePlacing, edit.rect);
                if edit.clicked() {
                    actions.push(AppAction::Sticker(crate::sticker::StickerAction::EditBeforePlacing));
                }
            });
        });
    });
    stamp_tool_style(ui, state);
    let st = &mut state.tools.stamp;
    egui::Grid::new("stamp_grid").num_columns(2).show(ui, |ui| {
        grid_label(ui, "Size");
        // `SliderClamping::Edits`: a Ctrl+wheel size beyond the slider range (stamp_place.rs) is kept, not clamped back.
        ui.add(percent_slider(&mut st.size_frac, 1.0, 100.0).logarithmic(true).clamping(SliderClamping::Edits))
            .on_hover_text("Larger side of the stamp, % of the model size (2D: of the texture width). Ctrl+wheel");
        ui.end_row();
        grid_label(ui, "Rotation");
        ui.add(Slider::new(&mut st.rotation_deg, -180.0..=180.0).suffix(" °").max_decimals(0)).on_hover_text("Shift+wheel");
        ui.end_row();
    });
    // Round 6: Project | Wrap and the two Wrap options for new 3D stamps (shared with the Text tool).
    tool_surface_options(
        ui,
        st,
        &mut state.tool_probe.wrap_controls,
        "Also print on surfaces behind the first one (the far side of the model)",
    );
    help(ui, "Click on the model or the texture to place the stamp. It becomes its own layer.");
}

/// Stickers P6b (STICKER_PLAN 3.6): the style new stamps of the Stamp tool get (a setting: none by default, so a
/// placed image is exactly an image stamp of before). The presets are scaled to the chosen picture.
fn stamp_tool_style(ui: &mut egui::Ui, state: &mut AppState) {
    let source = state.tools.stamp.source;
    let size = match source.blob() {
        Some(blob) => state.doc_mut().and_then(|d| d.image_size(blob).ok()),
        None => match source {
            StampToolSource::Sticker(id) => state.doc().and_then(|d| d.sticker(id)).map(|d| d.size()),
            _ => None,
        },
    }
    .unwrap_or([512, 512]);
    let current = &state.tools.stamp.styles;
    let shown = if current.is_default() {
        "None".to_owned()
    } else {
        StylePreset::ALL
            .into_iter()
            .find(|p| *p != StylePreset::Clear && p.styles(size) == *current)
            .map_or_else(|| "Custom".to_owned(), |p| p.name().to_owned())
    };
    let mut picked = None;
    let resp = ui
        .horizontal(|ui| {
            grid_label(ui, "Style");
            egui::ComboBox::from_id_salt("stamp_tool_style")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_default(), "None").clicked() {
                        picked = Some(StyleStack::default());
                    }
                    for p in StylePreset::ALL.into_iter().filter(|p| *p != StylePreset::Clear) {
                        if ui.selectable_label(false, p.name()).clicked() {
                            picked = Some(p.styles(size));
                        }
                    }
                })
                .response
        })
        .inner
        .on_hover_text("The style new stamps get: a die-cut border, a shadow... (change it later on the Sticker tab)");
    state.panel_rects.insert(PanelHit::StampToolStyle, resp.rect);
    if let Some(styles) = picked {
        state.tools.stamp.styles = styles;
    }
}

// ---- the selected stamp ----

/// Larger side of a placement in percent of the model diagonal (projected) or of its set's texture width (UV).
pub(crate) fn placement_size_pct(p: &Placement, diag: f32, tex_width: f32) -> f32 {
    match p {
        Placement::Projected(pr) => 2.0 * pr.half_size.max_element() / diag.max(1e-9) * 100.0,
        Placement::Uv(uv) => uv.size_px.max_element() / tex_width.max(1.0) * 100.0,
    }
}

/// The placement scaled uniformly about its centre.
pub(crate) fn scale_placement(p: &Placement, factor: f32) -> Placement {
    let mut p = *p;
    match &mut p {
        Placement::Projected(pr) => pr.half_size *= factor,
        Placement::Uv(uv) => uv.size_px *= factor,
    }
    p
}

/// Rotation of a projector in radians, clockwise as seen by the viewer (+Z), the sense of `Projector::rotated`:
/// the angle of its up axis from the up axis of the unrotated frame `Projector::from_surface(.., +Y, ..)` at the
/// same point and normal.
pub(crate) fn projector_angle(p: &Projector) -> f32 {
    let (_, up, out) = p.axes();
    let reference = Projector::from_surface(p.origin, out, glam::Vec3::Y, p.half_size, 1.0);
    let (ref_right, ref_up, _) = reference.axes();
    up.dot(ref_right).atan2(up.dot(ref_up))
}

/// Rotation of a placement in degrees (clockwise positive, 4.1).
pub(crate) fn placement_rotation_deg(p: &Placement) -> f32 {
    match p {
        Placement::Projected(pr) => projector_angle(pr).to_degrees(),
        Placement::Uv(uv) => uv.rotation.to_degrees(),
    }
}

/// The placement turned to an absolute rotation in degrees.
pub(crate) fn rotate_placement_to(p: &Placement, deg: f32) -> Placement {
    match p {
        Placement::Projected(pr) => Placement::Projected(pr.rotated(deg.to_radians() - projector_angle(pr))),
        Placement::Uv(uv) => {
            let mut uv = *uv;
            uv.rotation = deg.to_radians();
            Placement::Uv(uv)
        }
    }
}

fn stamp_properties(
    ui: &mut egui::Ui,
    state: &mut AppState,
    ps: &mut PanelState,
    stamp: StampObject,
    actions: &mut Vec<AppAction>,
) {
    let id = stamp.id;
    let Some(doc) = state.doc() else { return };
    let diag = doc.mesh.bounds.diagonal();
    let home_layer = doc.stamp_layers(id).into_iter().find(|(set, _)| *set == stamp.home_set).map(|(_, l)| l);
    let tex_width = match &stamp.placement {
        Placement::Uv(uv) => doc.set(uv.set).map_or(1.0, |s| s.width as f32),
        Placement::Projected(_) => 1.0,
    };
    // Stickers (P3a): a sticker stamp shows like an image stamp, without the "Image: Choose..." row (the sticker
    // editor of P6a / P6b replaces it).
    let source_name = match &stamp.source {
        StampSource::Image { blob } => doc.blobs.get(*blob).map(|b| b.name.clone()),
        StampSource::Text(_) | StampSource::Sticker { .. } => None,
    };
    // A locked linked layer protects the stamp (core `stamp_locked`): only the name stays editable.
    let locked = doc.stamp_locked(id);
    if locked {
        ui.colored_label(ui.visuals().warn_fg_color, crate::panels::layers::stamp_lock_message(doc, id));
        // The lock may sit in a set the Layers list does not show right now: offer to go there.
        let active_set = state.session.as_ref().map(|s| s.active_set);
        let locking: Vec<_> = doc
            .stamp_layers(id)
            .into_iter()
            .filter(|(set, l)| Some(*set) != active_set && doc.layer(*l).is_some_and(|l| l.locked))
            .collect();
        if let Some((set, _)) = locking.first() {
            let name = doc.set(*set).map_or_else(String::new, |s| s.name.clone());
            if ui.button(format!("Show set \"{name}\"")).on_hover_text("Show that set's layers to unlock the layer").clicked() {
                actions.push(AppAction::SetActiveSet(*set));
            }
        }
    }

    // Name (renames the stamp and its linked layers).
    egui::Grid::new(("stamp_props", id.0)).num_columns(2).show(ui, |ui| {
        grid_label(ui, "Name");
        let mut name = match &ps.stamp_name {
            Some((sid, text)) if *sid == id => text.clone(),
            _ => stamp.name.clone(),
        };
        let resp = ui.add(TextEdit::singleline(&mut name).id_salt(("stamp_name", id.0)).desired_width(f32::INFINITY));
        if resp.has_focus() {
            ps.stamp_name = Some((id, name));
        } else {
            let trimmed = name.trim();
            if resp.lost_focus()
                && !trimmed.is_empty()
                && trimmed != stamp.name
                && let Some(layer) = home_layer
            {
                actions.push(AppAction::RenameLayer(layer, trimmed.to_owned()));
            }
            ps.stamp_name = None;
        }
        ui.end_row();

        grid_label(ui, "Tint");
        let mut c = to_color32(stamp.tint);
        let resp = ui.add_enabled_ui(!locked, |ui| color_edit_button_srgba(ui, &mut c, Alpha::OnlyBlend)).inner;
        let tint = from_color32(c);
        continuous_edit(ps, actions, ui, &resp, |commit| AppAction::UpdateStampTint(id, tint, commit));
        ui.end_row();

        if let StampSource::Image { .. } = &stamp.source {
            grid_label(ui, "Image");
            ui.horizontal(|ui| {
                ui.label(source_name.as_deref().unwrap_or("(missing)"));
                if ui.add_enabled(!locked, egui::Button::new("Choose...")).on_hover_text("Replace the image of this stamp").clicked() {
                    actions.push(AppAction::ReplaceStampImage(id, None));
                }
                // Stickers P6b (STICKER_PLAN 3.9): the image opens in the Sticker view.
                if ui.button("Edit").on_hover_text("Edit the picture in the Sticker view").clicked() {
                    actions.push(AppAction::Sticker(crate::sticker::StickerAction::Enter(Some(id))));
                }
            });
            ui.end_row();
        }
    });

    // The text fields of a text stamp are the Text block above (`text_block::show`, round 4).
    ui.separator();
    let mut probe = None;
    ui.add_enabled_ui(!locked, |ui| placement_fields(ui, ps, state.doc(), &stamp, diag, tex_width, actions, &mut probe));
    if probe.is_some() {
        state.tool_probe.wrap_controls = probe;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!locked, egui::Button::new("Delete stamp")).on_disabled_hover_text("The layer is locked").clicked() {
            actions.push(AppAction::DeleteStamp(id));
        }
        if ui.button("Deselect").on_hover_text(shortcut_text(ui.ctx(), "Cancel")).clicked() {
            actions.push(AppAction::SelectStamp(None));
        }
    });
}

/// `new` (an edit of the projected placement `old`) with its depth range re-fitted to the surface when the depth of
/// `old` is the automatic one (6.5); unchanged otherwise, for UV placements and when nothing changed.
fn refit_auto_depth(doc: Option<&wte_core::Document>, old: &Placement, new: Placement) -> Placement {
    match (new, old, doc) {
        (Placement::Projected(p), Placement::Projected(o), Some(doc)) if new != *old && doc.stamp_depth_is_auto(o) => {
            Placement::Projected(doc.fit_stamp_depth(&p))
        }
        _ => new,
    }
}

/// Size, rotation and the projection options of the selected stamp (`UpdateStampPlacement`). Round 6: a 3D stamp
/// starts with the "On the model: Project | Wrap" selector, the two Wrap options (each change one Final edit) and, in
/// Wrap, the lines of `wrap_stats_lines`; in Wrap the depth, angle, back-face and occlusion rows are hidden (kept in
/// the placement for switching back). `probe` records the Wrap option controls (`ToolProbe::wrap_controls`).
#[expect(clippy::too_many_arguments, reason = "the fields' whole context")]
fn placement_fields(
    ui: &mut egui::Ui,
    ps: &mut PanelState,
    doc: Option<&wte_core::Document>,
    stamp: &StampObject,
    diag: f32,
    tex_width: f32,
    actions: &mut Vec<AppAction>,
    probe: &mut Option<bool>,
) {
    let id = stamp.id;
    let placement = stamp.placement;
    let place = |p: Placement| move |commit| AppAction::UpdateStampPlacement(id, p, commit);
    let wrapped = matches!(placement, Placement::Projected(pr) if pr.mode == SurfaceMode::Wrap);
    if let Placement::Projected(pr) = placement {
        let final_edit = |p: Projector| AppAction::UpdateStampPlacement(id, Placement::Projected(p), wte_core::Commit::Final);
        if let Some(mode) = mode_selector(ui, pr.mode) {
            actions.push(final_edit(Projector { mode, ..pr }));
        }
        if let Some(wrap) = wrap_option_controls(ui, pr.mode, pr.wrap, probe) {
            actions.push(final_edit(Projector { wrap, ..pr }));
        }
        if wrapped {
            wrap_stats_lines(ui, doc.and_then(|d| d.stamp_wrap_stats(id)), pr.wrap);
        }
        // Round 9 (package B, T9): the model itself reuses texture area under the picture.
        if let Some(text) = shared_texels_text(doc.and_then(|d| d.stamp_shared_texels(id))) {
            ui.add(egui::Label::new(egui::RichText::new(text).color(ui.visuals().warn_fg_color)).wrap());
        }
    }
    // Fix round 2, item 2: a new size or rotation of a projected stamp whose depth range is automatic re-fits it to the
    // surface under the new footprint, like the Transform tool does; a hand-set depth is kept. Only evaluated when the
    // field changed (the auto test itself is a fit).
    let refit = |p: Placement| refit_auto_depth(doc, &placement, p);
    egui::Grid::new(("stamp_placement", id.0)).num_columns(2).show(ui, |ui| {
        grid_label(ui, "Size");
        let size = placement_size_pct(&placement, diag, tex_width);
        let mut new_size = size;
        let what = if matches!(placement, Placement::Projected(_)) { "model size" } else { "texture width" };
        let resp = ui
            .add(doc_value(&mut new_size, 0.1..=500.0).speed(0.2).max_decimals(1).suffix(" %"))
            .on_hover_text(format!("Larger side of the stamp, % of the {what}"));
        let scaled = if size > 0.0 && new_size > 0.0 { scale_placement(&placement, new_size / size) } else { placement };
        continuous_edit(ps, actions, ui, &resp, place(refit(scaled)));
        ui.end_row();

        grid_label(ui, "Rotation");
        let mut deg = placement_rotation_deg(&placement);
        let resp = ui.add(doc_value(&mut deg, -180.0..=180.0).speed(0.5).max_decimals(1).suffix(" °"));
        let rotated = rotate_placement_to(&placement, deg);
        continuous_edit(ps, actions, ui, &resp, place(refit(rotated)));
        ui.end_row();

        if let Placement::Projected(pr) = placement
            && !wrapped
        {
            let pct = |v: f32| v / diag.max(1e-9) * 100.0;
            let world = |v: f32| v * diag.max(1e-9) / 100.0;

            grid_label(ui, "Depth front");
            let mut front = pct(pr.depth_front);
            let resp = ui
                .add(doc_value(&mut front, 0.05..=MAX_DEPTH_PCT).speed(0.2).max_decimals(1).suffix(" %"))
                .on_hover_text("Reach of the projection toward the viewer, % of the model size");
            let p = Projector { depth_front: world(front), ..pr };
            continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
            ui.end_row();

            grid_label(ui, "Depth back");
            let mut back = pct(pr.depth_back);
            let resp = ui
                .add(doc_value(&mut back, 0.05..=MAX_DEPTH_PCT).speed(0.2).max_decimals(1).suffix(" %"))
                .on_hover_text("Reach of the projection into the model, % of the model size");
            let p = Projector { depth_back: world(back), ..pr };
            continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
            ui.end_row();

            grid_label(ui, "");
            let fit = ui
                .add_enabled(doc.is_some(), egui::Button::new("Fit depth to surface"))
                .on_hover_text("Set the depth range to the surface under the stamp (as when it was placed). Moving or scaling it then keeps fitting it");
            if fit.clicked()
                && let Some(doc) = doc
            {
                actions.push(AppAction::UpdateStampPlacement(id, Placement::Projected(doc.fit_stamp_depth(&pr)), wte_core::Commit::Final));
            }
            ui.end_row();

            grid_label(ui, "Angle limits");
            ui.horizontal(|ui| {
                let mut start = pr.angle_start_deg;
                let resp = ui
                    .add(doc_value(&mut start, 0.0..=pr.angle_end_deg.min(90.0)).speed(0.5).max_decimals(0).suffix(" °"))
                    .on_hover_text("Full strength while the surface faces the projector within this angle");
                let p = Projector { angle_start_deg: start, ..pr };
                continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
                let mut end = pr.angle_end_deg;
                let resp = ui
                    .add(doc_value(&mut end, pr.angle_start_deg.max(0.0)..=90.0).speed(0.5).max_decimals(0).suffix(" °"))
                    .on_hover_text("No paint beyond this angle");
                let p = Projector { angle_end_deg: end, ..pr };
                continuous_edit(ps, actions, ui, &resp, place(Placement::Projected(p)));
            });
            ui.end_row();
        }
    });

    ui.horizontal_wrapped(|ui| {
        let (mut fx, mut fy) = match placement {
            Placement::Projected(pr) => (pr.flip_x, pr.flip_y),
            Placement::Uv(uv) => (uv.flip_x, uv.flip_y),
        };
        let fx_changed = ui.checkbox(&mut fx, "Flip X").changed();
        let fy_changed = ui.checkbox(&mut fy, "Flip Y").changed();
        if fx_changed || fy_changed {
            let mut p = placement;
            match &mut p {
                Placement::Projected(pr) => (pr.flip_x, pr.flip_y) = (fx, fy),
                Placement::Uv(uv) => (uv.flip_x, uv.flip_y) = (fx, fy),
            }
            actions.push(AppAction::UpdateStampPlacement(id, p, wte_core::Commit::Final));
        }
        match placement {
            // Wrap ignores the back-face and occlusion settings (kept for switching back).
            Placement::Projected(_) if wrapped => {}
            Placement::Projected(pr) => {
                let mut cull = pr.backface_cull;
                if ui.checkbox(&mut cull, "Backface cull").on_hover_text("Surfaces facing away receive nothing").changed() {
                    let p = Projector { backface_cull: cull, ..pr };
                    actions.push(AppAction::UpdateStampPlacement(id, Placement::Projected(p), wte_core::Commit::Final));
                }
                let mut occlusion = pr.occlusion;
                if ui.checkbox(&mut occlusion, "Occlusion").on_hover_text("Surfaces hidden behind others receive nothing").changed() {
                    let p = Projector { occlusion, ..pr };
                    actions.push(AppAction::UpdateStampPlacement(id, Placement::Projected(p), wte_core::Commit::Final));
                }
            }
            Placement::Uv(uv) => {
                let mut clip = uv.clip_to_islands;
                if ui.checkbox(&mut clip, "Clip to UV islands").changed() {
                    let mut uv = uv;
                    uv.clip_to_islands = clip;
                    actions.push(AppAction::UpdateStampPlacement(id, Placement::Uv(uv), wte_core::Commit::Final));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::tests::{run_panels, sample_state};
    use glam::{Vec2, Vec3};
    use wte_core::{Document, Outline, SampleKind, StampId, TextAlign, TextSpec, TextureSetId, UvPlacement};

    fn projector() -> Projector {
        Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1), 0.2)
    }

    /// Round 8 (plan 4.1 / 4.2): the two new lines under "Wrapped over N triangles".
    #[test]
    fn a_relaxed_or_cut_wrap_says_so_under_the_stats_line() {
        let base = wte_core::WrapStats {
            verts: 100,
            tris: 180,
            cut_tris: 0,
            covered: 1.0,
            met_itself: false,
            reached_edge: false,
            limited: false,
            jumps: 0,
            turns: 1,
            micros: 1000,
            relaxed: false,
            seams: 0,
            seams_by_rule: [0; 5],
            flipped: 0,
            stretch: 0.0,
            rejected: 0.0,
            cg_iters: 0,
            passed: false,
            ends_share: 1.0,
            lost_cut: 0.0,
            room: 1.0,
            wraps_closed: false,
            shown: 1.0,
            beyond: 0.0,
            gaps: 0.0,
            at_part: 0.0,
        };
        let opts = WrapOptions::default();
        assert!(wrap_stats_texts(&base, opts).is_empty(), "an exact (seed) wrap adds no line");
        let fitted = "Fitted to the curved surface: stretched up to 35 %";
        let cut = "Cut where it cannot lie flat (a corner, a handle, or where its edges meet)";
        let relaxed = wte_core::WrapStats { relaxed: true, stretch: 0.349, ..base };
        assert_eq!(wrap_stats_texts(&relaxed, opts), vec![WrapLine::Help(fitted.to_owned())]);
        // Below 5 % the stretch is not worth a line; a seed layout never says "fitted".
        assert!(wrap_stats_texts(&wte_core::WrapStats { stretch: 0.049, ..relaxed }, opts).is_empty());
        assert!(wrap_stats_texts(&wte_core::WrapStats { relaxed: false, ..relaxed }, opts).is_empty());
        let both = wte_core::WrapStats { seams: 3, seams_by_rule: [0, 3, 0, 0, 0], met_itself: true, ..relaxed };
        assert_eq!(
            wrap_stats_texts(&both, opts),
            vec![
                WrapLine::Help(fitted.to_owned()),
                WrapLine::Help(cut.to_owned()),
                WrapLine::Warn("The ends meet at the back".to_owned())
            ]
        );
        // The round-6 warnings stay as they were, after the new lines; stable (w1): "runs off" only at an edge.
        let off = wte_core::WrapStats { covered: 0.5, shown: 0.5, reached_edge: true, jumps: 2, ..base };
        assert_eq!(
            wrap_stats_texts(&off, opts),
            vec![
                WrapLine::Warn("50 % of the picture runs off the surface: an edge, a hole or the end of the part".to_owned()),
                WrapLine::Warn("Continued onto 2 separate parts".to_owned())
            ]
        );
    }

    /// Stable (w1, item 6): each loss names its own cause, one line each, never contradicting: the sweep's cases
    /// (numbers from their logs).
    #[test]
    fn the_wrap_lines_name_the_right_cause_and_never_contradict() {
        let base = wte_core::WrapStats {
            verts: 100,
            tris: 180,
            cut_tris: 0,
            covered: 1.0,
            met_itself: false,
            reached_edge: false,
            limited: false,
            jumps: 0,
            turns: 1,
            micros: 1000,
            relaxed: false,
            seams: 0,
            seams_by_rule: [0; 5],
            flipped: 0,
            stretch: 0.0,
            rejected: 0.0,
            cg_iters: 0,
            passed: false,
            ends_share: 1.0,
            lost_cut: 0.0,
            room: 1.0,
            wraps_closed: false,
            shown: 1.0,
            beyond: 0.0,
            gaps: 0.0,
            at_part: 0.0,
        };
        let opts = WrapOptions::default();
        let texts = |s: wte_core::WrapStats| wrap_stats_line_texts(&s, opts);
        // The Lantern: no edge, the loss is cut wedges (round 9 said "runs off the surface: an edge ..."); stable w2:
        // the picture covers the whole closed base block, cut open at its corners. Stable (fix_wrap_1, the sweep's
        // numbers): `shown` 0.731 where `covered` said 0.903 (the folded layout's areas summed): the rest of the
        // picture finds no surface on the block (`room` 0.91), which the covered line now says.
        let lantern = wte_core::WrapStats { covered: 0.9029, shown: 0.7311, lost_cut: 0.1211, rejected: 0.1837, room: 0.9091, wraps_closed: true, ..base };
        assert_eq!(
            texts(lantern),
            vec![
                "12 % of the picture is lost where it is cut open round the closed part's corners".to_owned(),
                "18 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned(),
                "The part is narrower than the picture: only 73 % of the picture lies on it".to_owned()
            ]
        );
        let open_cut = wte_core::WrapStats { covered: 0.8849, shown: 0.8849, lost_cut: 0.1123, ..base };
        assert_eq!(texts(open_cut), vec!["11 % of the picture is lost where the surface had to be cut".to_owned()]);
        // The torus knot: a helix, its ends met; nothing ran off an edge (sweep: beyond 0.558).
        let knot = wte_core::WrapStats { covered: 0.4441, shown: 0.4422, beyond: 0.5578, passed: true, ends_share: 0.43, ..base };
        assert_eq!(texts(knot), vec!["Longer than the way round: 43 % shown, the ends meet at the back".to_owned()]);
        // many_islands with Jump off: the spine met itself round an island and the picture past the S1 closure seams
        // is bare (beyond 0.99): the ends line with its share, and the islands' room.
        let islands_off = wte_core::WrapStats { covered: 0.0177, shown: 0.0125, beyond: 0.9875, met_itself: true, ends_share: 0.184, rejected: 0.2749, room: 0.091, ..base };
        assert_eq!(
            texts(islands_off),
            vec![
                "The ends meet at the back: 18 % of the picture's length shown".to_owned(),
                "27 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned(),
                "The part is narrower than the picture: only 1 % of the picture lies on it".to_owned()
            ]
        );
        // many_islands with Jump on: round 9 stacked "runs off", "The ends meet", "Longer than the way round" and the
        // parts line; stable w2: the islands are a tenth of the picture's height across (`room` 0.1029): the covered
        // line says so. Stable (fix_wrap_1): the spine met itself without a closure seam (S4 / S5 only), so no
        // picture is bare beyond a closure (`beyond` 0): the islands' room explains the share, no ends line (w1 / w2:
        // "The ends meet at the back: 37 % ...").
        let islands = wte_core::WrapStats { covered: 0.0338, shown: 0.0310, beyond: 0.0, met_itself: true, ends_share: 0.369, jumps: 2, rejected: 0.0642, lost_cut: 0.0050, room: 0.1029, ..base };
        let lines = texts(islands);
        assert_eq!(
            lines,
            vec![
                "6 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned(),
                "The part is narrower than the picture: only 3 % of the picture lies on it".to_owned(),
                "Continued onto 2 separate parts".to_owned()
            ]
        );
        assert!(!lines.iter().any(|l| l.contains("runs off") || l.contains("Longer than the way round")), "{lines:?}");
        // DamagedHelmet with Jump on: 18 % of the laid picture on overlapping plates (round 9: no line); stable
        // (fix_wrap_1): 2.6 % of the cells bare (`shown` 0.974, `covered` 1.0). Stable (fix-wrap, the sweep's numbers):
        // 1.96 % of them are lost to cuts inside the laid outline and 0.52 % in gaps between its plates, 0.1 % is
        // left: no "3 % runs off the surface" (fix_wrap_1's line), the shown share. Polish item 3: no cause passed its
        // threshold, so the line names the largest (the cuts' 1.96 %, gate report open item 4).
        let helmet = wte_core::WrapStats { covered: 1.0, shown: 0.9737, reached_edge: true, rejected: 0.1795, jumps: 9, lost_cut: 0.0196, gaps: 0.0052, at_part: 0.0005, ..base };
        assert_eq!(
            texts(helmet),
            vec![
                "18 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned(),
                "Only 97 % of the picture lies on the surface, mostly lost to cuts (2 %)".to_owned(),
                "Continued onto 9 separate parts".to_owned()
            ]
        );
        // Polish item 3: the largest cause below its own line's threshold, for each kind of loss; never one that has a
        // line, and nothing named while a cause line shows.
        let small = wte_core::WrapStats { covered: 1.0, shown: 0.97, ..base };
        let only = |s: wte_core::WrapStats| texts(s).into_iter().find(|l| l.starts_with("Only ")).unwrap_or_default();
        assert_eq!(
            only(wte_core::WrapStats { gaps: 0.016, at_part: 0.01, lost_cut: 0.004, reached_edge: true, ..small }),
            "Only 97 % of the picture lies on the surface, mostly in the gaps between separate parts (2 %)"
        );
        assert_eq!(
            only(wte_core::WrapStats { at_part: 0.018, gaps: 0.01, ..small }),
            "Only 97 % of the picture lies on the surface, mostly where it meets a neighbouring part (2 %)"
        );
        assert_eq!(
            only(wte_core::WrapStats { lost_cut: 0.012, reached_edge: true, ..small }),
            "Only 97 % of the picture lies on the surface, mostly running off the surface (2 %)",
            "an edge reached: the rest (1.8 %, below the runs-off line's 2 %) runs off"
        );
        assert_eq!(
            only(wte_core::WrapStats { shown: 0.95, lost_cut: 0.01, ..small }),
            "Only 95 % of the picture lies on the surface, mostly hidden or folded (4 %)",
            "no edge reached: the rest is hidden or folded"
        );
        assert_eq!(
            only(wte_core::WrapStats { shown: 0.972, met_itself: true, ends_share: 0.9, beyond: 0.018, ..small }),
            "Only 97 % of the picture lies on the surface, mostly beyond where the picture closed on itself (2 %)"
        );
        // A cause line shows (cut 3 %): the "Only" line names no cause of its own.
        let with_cut = texts(wte_core::WrapStats { shown: 0.90, lost_cut: 0.03, ..small });
        assert_eq!(
            with_cut,
            vec![
                "3 % of the picture is lost where the surface had to be cut".to_owned(),
                "Only 90 % of the picture lies on the surface".to_owned()
            ]
        );
        // nonmanifold: an edge and overlapping pages, both true; stable (fix_wrap_1): `shown` 0.178 (ink 0.188) where
        // `covered` summed the seven pages' areas (0.63).
        let book = wte_core::WrapStats { covered: 0.6303, shown: 0.1781, reached_edge: true, rejected: 0.7027, ..base };
        assert_eq!(
            texts(book),
            vec![
                "82 % of the picture runs off the surface: an edge, a hole or the end of the part".to_owned(),
                "70 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned()
            ]
        );
        // An open noisy field (no edge, no closure, little cut): the shown share alone, no cause invented; stable
        // (fix_wrap_1): noisy400's `shown` 0.473 (ink 0.472), where `covered` 0.862 made the line say 86 %.
        let noisy = wte_core::WrapStats { covered: 0.862, shown: 0.473, beyond: 0.5016, lost_cut: 0.0254, rejected: 0.4498, ..base };
        assert_eq!(
            texts(noisy),
            vec![
                "3 % of the picture is lost where the surface had to be cut".to_owned(),
                "45 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned(),
                "Only 47 % of the picture lies on the surface".to_owned()
            ]
        );
        // Overlap winds the rest on: no ends line.
        let wound = wte_core::WrapStats { met_itself: true, turns: 3, ..base };
        assert_eq!(wrap_stats_line_texts(&wound, WrapOptions { jump_gaps: false, overlap: true }), vec!["Wound 3 times".to_owned()]);
        // Stable (fix_wrap_1): ABeautifulGame with Jump on (step 10's log): the spine looped round a pawn and met its
        // own path (ends 73.8 %), but no picture cell is bare beyond that closure (`beyond` 0): the board holds the
        // picture, so no ends line (w1 / w2 said "The ends meet at the back: 74 % of the picture's length shown").
        // 8.7 % of the cells are bare past the board's front edge (the frame's corner): `shown` says so, where
        // `covered` (1.0: the board and the pawns laid over the same picture points, summed) hid it. Stable (fix-wrap,
        // item 2): they lie where the board slab meets the flush marble frame (`at_part` 0.086): the picture stops at a
        // neighbouring part (fix_wrap_1 said "9 % of the picture runs off the surface: an edge, ...").
        let chess = wte_core::WrapStats {
            covered: 1.0,
            shown: 0.9127,
            beyond: 0.0,
            met_itself: true,
            ends_share: 0.738,
            reached_edge: true,
            relaxed: true,
            stretch: 0.9246,
            seams: 2147,
            rejected: 0.5033,
            lost_cut: 0.0012,
            jumps: 30,
            at_part: 0.086,
            ..base
        };
        assert_eq!(
            texts(chess),
            vec![
                "Fitted to the curved surface: stretched up to 92 %".to_owned(),
                "Cut where it cannot lie flat (a corner, a handle, or where its edges meet)".to_owned(),
                "9 % of the picture stops where it meets a neighbouring part".to_owned(),
                "50 % of the laid picture fell where the surface overlaps another part or folds over itself: each spot of the picture is shown in one place only".to_owned(),
                "Continued onto 30 separate parts".to_owned()
            ]
        );
        // Stable (fix-wrap, item 2): parts200 (200 x 200 separate quads, Jump on): 28 % of the cells bare in the gaps
        // between the quads while the picture looks whole (fix_wrap_1: "28 % of the picture runs off the surface");
        // the 0.6 % bare past the field's outer quads says nothing.
        let parts = wte_core::WrapStats { covered: 0.7312, shown: 0.7231, reached_edge: true, rejected: 0.0037, jumps: 7774, gaps: 0.2763, ..base };
        assert_eq!(
            texts(parts),
            vec!["28 % of the picture falls into the gaps between separate parts".to_owned(), "Continued onto 7774 separate parts".to_owned()]
        );
        // Gaps, a neighbouring part and an edge together: each its own share.
        let mixed = wte_core::WrapStats { shown: 0.7, reached_edge: true, jumps: 3, gaps: 0.1, at_part: 0.15, ..base };
        assert_eq!(
            texts(mixed),
            vec![
                "10 % of the picture falls into the gaps between separate parts".to_owned(),
                "15 % of the picture stops where it meets a neighbouring part".to_owned(),
                "5 % of the picture runs off the surface: an edge, a hole or the end of the part".to_owned(),
                "Continued onto 3 separate parts".to_owned()
            ]
        );
        // Never "runs off" without an edge, never "Longer than the way round" without a pass; stable (fix_wrap_1): a
        // share of the picture's length only when picture is left bare beyond the closure.
        for covered in [0.3f32, 0.9, 0.99] {
            for beyond in [0.0f32, 1.0 - covered] {
                for (met, passed, edge) in [(false, false, false), (true, false, false), (false, true, false), (true, false, true), (false, false, true)] {
                    let s = wte_core::WrapStats { covered, shown: covered, beyond, met_itself: met, passed, reached_edge: edge, ends_share: 0.5, ..base };
                    let lines = texts(s);
                    let cut_ends = beyond >= ENDS_BEYOND_MIN;
                    // Stable (fix-wrap): picture bare beyond a closure does not run off an edge.
                    assert_eq!(lines.iter().any(|l| l.contains("runs off")), edge && covered < 0.98 && beyond == 0.0, "{s:?}: {lines:?}");
                    assert_eq!(lines.iter().any(|l| l.contains("Longer than the way round")), passed && cut_ends, "{s:?}: {lines:?}");
                    assert_eq!(lines.iter().any(|l| l.contains("ends meet")), (met || passed) && cut_ends, "{s:?}: {lines:?}");
                    assert!(lines.iter().filter(|l| l.contains("ends meet")).count() <= 1, "{lines:?}");
                }
            }
        }
    }

    /// Round 9 (package B, plan section 6): a picture longer than the way round says how much of it is shown.
    #[test]
    fn a_picture_longer_than_the_way_round_says_how_much_is_shown() {
        let base = wte_core::WrapStats {
            verts: 100,
            tris: 180,
            cut_tris: 0,
            covered: 1.0,
            met_itself: false,
            reached_edge: false,
            limited: false,
            jumps: 0,
            turns: 1,
            micros: 1000,
            relaxed: false,
            seams: 0,
            seams_by_rule: [0; 5],
            flipped: 0,
            stretch: 0.0,
            rejected: 0.0,
            cg_iters: 0,
            passed: true,
            ends_share: 0.43,
            lost_cut: 0.0,
            room: 1.0,
            wraps_closed: false,
            shown: 0.43,
            beyond: 0.57,
            gaps: 0.0,
            at_part: 0.0,
        };
        let opts = WrapOptions::default();
        assert_eq!(wrap_stats_texts(&base, opts), vec![WrapLine::Warn("Longer than the way round: 43 % shown, the ends meet at the back".to_owned())]);
        // Stable (fix_wrap_1): the plain line at 95 %, the 5 % past the meeting line bare beyond it, no other line.
        let fits = wte_core::WrapStats { ends_share: 0.95, shown: 0.95, beyond: 0.05, ..base };
        assert_eq!(wrap_stats_texts(&fits, opts), vec![WrapLine::Warn("The ends meet at the back".to_owned())]);
    }

    /// Round 9 (package B, T9): the shared-texel line shows for a stamp on mirrored halves (once the warm-up found the
    /// shared texels), not on the sphere; nothing is built on the calling thread before.
    #[test]
    fn the_shared_texel_line_shows_where_the_model_reuses_its_texture() {
        let img = wte_core::samples::sample_stamp_image(32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let line = |mut doc: Document, at: Vec3| {
            let blob = doc.add_image_blob("s.png", std::sync::Arc::from(png.clone())).unwrap();
            let p = Projector::from_surface(at, Vec3::Z, Vec3::Y, Vec2::new(0.3, 0.2), 0.3);
            let id = doc.add_stamp("s", wte_core::StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
            assert!(doc.stamp_shared_texels(id).is_none(), "nothing is known before the warm-up");
            doc.wrap_mesh_warmup(false).expect("a warm-up to run").run();
            assert!(doc.wrap_mesh_warmup(false).is_none(), "built once");
            let share = doc.stamp_shared_texels(id);
            (share, shared_texels_text(share))
        };
        let (share, text) = line(wte_core::testutil::doc_from_raw(wte_core::samples::mirrored_quads(), 256), Vec3::new(-0.2, 0.0, 0.0));
        assert!(share.is_some_and(|s| s > 0.9) && text == Some(SHARED_TEXELS_LINE), "mirrored halves: {share:?}");
        let (share, text) = line(Document::sample(SampleKind::Sphere), Vec3::new(0.0, 0.0, 1.0));
        assert!(share == Some(0.0) && text.is_none(), "sphere: {share:?}");
        assert!(shared_texels_text(Some(0.049)).is_none() && shared_texels_text(Some(0.05)).is_some() && shared_texels_text(None).is_none());
    }

    #[test]
    fn projector_angle_matches_rotated() {
        let p = projector();
        assert!(projector_angle(&p).abs() < 1e-5);
        for a in [0.3f32, -1.2, 2.5] {
            assert!((projector_angle(&p.rotated(a)) - a).abs() < 1e-4, "{a}");
        }
        // A tilted surface normal with a non-vertical up hint.
        let q = Projector::from_surface(Vec3::ZERO, Vec3::new(0.3, 0.5, 0.8).normalize(), Vec3::X, Vec2::splat(0.1), 0.1);
        let a0 = projector_angle(&q);
        let turned = rotate_placement_to(&Placement::Projected(q), 33.0);
        assert!((placement_rotation_deg(&turned) - 33.0).abs() < 1e-2);
        let back = rotate_placement_to(&turned, a0.to_degrees());
        let Placement::Projected(b) = back else { unreachable!() };
        assert!(b.rotation.angle_between(q.rotation) < 1e-3);
    }

    #[test]
    fn size_percent_and_scaling() {
        let p = Placement::Projected(projector());
        assert!((placement_size_pct(&p, 2.0, 1.0) - 20.0).abs() < 1e-4); // larger side 0.4 of diagonal 2
        let s = scale_placement(&p, 1.5);
        assert!((placement_size_pct(&s, 2.0, 1.0) - 30.0).abs() < 1e-4);
        let uv = Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(100.0, 100.0),
            size_px: Vec2::new(64.0, 32.0),
            rotation: 0.5,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        assert!((placement_size_pct(&uv, 9.0, 256.0) - 25.0).abs() < 1e-4);
        assert!((placement_rotation_deg(&uv) - 0.5f32.to_degrees()).abs() < 1e-4);
        let Placement::Uv(r) = rotate_placement_to(&uv, -90.0) else { unreachable!() };
        assert!((r.rotation + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(r.center_px, Vec2::new(100.0, 100.0));
    }

    /// Fix round 2, item 2: the Size / Rotation fields re-fit an automatic depth to the new footprint, keep a hand-set
    /// one, and leave UV placements and unchanged values alone.
    #[test]
    fn size_and_rotation_fields_refit_an_automatic_depth() {
        let state = sample_state(SampleKind::Sphere);
        let doc = state.doc().unwrap();
        let r = doc.mesh.bounds.diagonal() * 0.29;
        let c = doc.mesh.bounds.center();
        let p = Projector::from_surface(c + Vec3::Z * r, Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1) * r, 0.0);
        let auto = Placement::Projected(doc.fit_stamp_depth(&p));
        assert_eq!(refit_auto_depth(Some(doc), &auto, auto), auto, "nothing changed");
        let bigger = scale_placement(&auto, 3.0);
        let Placement::Projected(b) = refit_auto_depth(Some(doc), &auto, bigger) else { unreachable!() };
        assert!(doc.stamp_depth_is_auto(&b));
        assert_eq!(b, doc.fit_stamp_depth(&b));
        let Placement::Projected(plain) = bigger else { unreachable!() };
        assert!(b.depth_back > plain.depth_back, "the 3x footprint reaches deeper into the sphere: {b:?}");
        let turned = rotate_placement_to(&auto, 90.0);
        let Placement::Projected(t) = refit_auto_depth(Some(doc), &auto, turned) else { unreachable!() };
        assert_eq!(t, doc.fit_stamp_depth(&t));

        let Placement::Projected(a) = auto else { unreachable!() };
        let typed = Placement::Projected(Projector { depth_back: a.depth_back * 5.0, ..a });
        let typed_bigger = scale_placement(&typed, 3.0);
        assert_eq!(refit_auto_depth(Some(doc), &typed, typed_bigger), typed_bigger, "a hand-set depth is kept");
        assert_eq!(refit_auto_depth(None, &auto, bigger), bigger);
    }

    #[test]
    fn text_tool_focus_request_is_served_and_cleared() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        state.tool = ToolKind::Text;
        state.focus_text_content = true;
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        };
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(!state.focus_text_content);
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(ctx.text_edit_focused(), "the text content field has the keyboard focus");
        // With another tool a stale request is dropped.
        state.tool = ToolKind::Brush;
        state.focus_text_content = true;
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(!state.focus_text_content);
    }

    fn screen_1280() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        }
    }

    /// Sphere with a selected image stamp of `placement` (Transform tool).
    fn with_selected_stamp(placement: impl FnOnce(&Document) -> Placement) -> (AppState, StampId) {
        let mut state = sample_state(SampleKind::Sphere);
        let img = wte_core::samples::sample_stamp_image(32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("ring.png", std::sync::Arc::from(png)).unwrap();
        let p = placement(doc);
        let id = doc.add_stamp("ring", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        (state, id)
    }

    #[test]
    fn out_of_range_placements_are_shown_without_being_rewritten() {
        // A UV stamp turned past 180 degrees (the knob and Shift+wheel do that in the 2D view).
        let (mut state, _) = with_selected_stamp(|doc| {
            let w = doc.sets[0].width as f32;
            Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::splat(w * 0.5),
                size_px: Vec2::splat(w * 0.1),
                rotation: 190f32.to_radians(),
                flip_x: false,
                flip_y: false,
                clip_to_islands: false,
            })
        });
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen_1280());
        }
        assert!(state.actions.is_empty(), "display alone edits nothing: {:?}", state.actions);

        // A project-through decal scaled up: depth far beyond 200 % of the model size, size beyond 500 %.
        let (mut state, _) = with_selected_stamp(|doc| {
            let d = doc.mesh.bounds.diagonal();
            let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
            let mut p = Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::splat(d * 3.0), d * 3.0);
            p.depth_back = d * 12.0;
            Placement::Projected(p)
        });
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen_1280());
        }
        assert!(state.actions.is_empty(), "display alone edits nothing: {:?}", state.actions);
    }

    #[test]
    fn wheel_sizes_beyond_the_slider_range_survive_a_redraw() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        state.tool = ToolKind::Stamp;
        state.tools.stamp.size_frac = 1.1; // one Ctrl+wheel notch past 100 %
        run_panels(&ctx, &mut state, &mut ps, screen_1280());
        assert_eq!(state.tools.stamp.size_frac, 1.1);
        state.tools.stamp.size_frac = 0.004;
        run_panels(&ctx, &mut state, &mut ps, screen_1280());
        assert_eq!(state.tools.stamp.size_frac, 0.004);
        state.tool = ToolKind::Text;
        state.tools.text.em_frac = 0.8;
        run_panels(&ctx, &mut state, &mut ps, screen_1280());
        assert_eq!(state.tools.text.em_frac, 0.8);
    }

    #[test]
    fn a_locked_stamp_renders_quietly() {
        let (mut state, id) = with_selected_stamp(|doc| {
            let d = doc.mesh.bounds.diagonal();
            let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
            Placement::Projected(Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::splat(d * 0.1), d * 0.1))
        });
        let layer = state.doc().unwrap().stamp_layers(id)[0].1;
        state.doc_mut().unwrap().set_layer_locked(layer, true).unwrap();
        let ctx = egui::Context::default();
        let mut ps = PanelState::new();
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, screen_1280());
        }
        assert!(state.actions.is_empty(), "{:?}", state.actions);
    }

    #[test]
    fn selected_stamp_properties_render_and_stay_quiet() {
        let ctx = egui::Context::default();
        let mut state = sample_state(SampleKind::Sphere);
        let mut ps = PanelState::new();
        let doc = state.doc_mut().unwrap();
        let font = doc.default_font();
        let spec = TextSpec {
            content: "WTE".into(),
            font,
            size_px: wte_core::TEXT_PLACEMENT_PX,
            color: [0, 0, 0, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: Some(Outline { width_em: 0.05, color: [255; 4] }),
            fx: wte_core::TextFx::default(),
        };
        let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
        let p = Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::new(0.2, 0.1), 0.2);
        let id = doc.add_stamp("WTE", StampSource::Text(spec), Placement::Projected(p), TextureSetId(0)).unwrap();
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        state.tool = ToolKind::Transform;
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        };
        for _ in 0..3 {
            run_panels(&ctx, &mut state, &mut ps, input());
        }
        // (round 5 R5-3: the font button asks for the samples once per catalog.)
        state.actions.retain(|a| !matches!(a, AppAction::LoadFontSamples));
        assert!(state.actions.is_empty(), "{:?}", state.actions);
        // Another tool offers the jump back to Transform.
        state.tool = ToolKind::Brush;
        run_panels(&ctx, &mut state, &mut ps, input());
        assert!(state.actions.is_empty(), "{:?}", state.actions);
    }
}
