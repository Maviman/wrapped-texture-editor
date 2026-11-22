// Owner: app-shell (A1). Toasts of ARCHITECTURE.md 8.9: an `egui::Area` anchored to the bottom-right corner of the
// central view area (never over the side panel or the status bar), at most `MAX_WIDTH` wide, long words wrapped; info
// and warnings fade after 4 s (measured with `i.time`), errors stay until dismissed; everything is also logged.
// Repeats are coalesced (one toast per text, one toast for the Undo / Redo steps, with a count) and the stack is
// capped, so a burst of toasts never covers the views.

use std::time::Duration;

use eframe::egui;

use crate::action::ToastLevel;

/// Seconds an info or warning toast stays visible.
const FADE_SECONDS: f64 = 4.0;
/// Seconds of the fade-out at the end of that time.
const FADE_OUT_SECONDS: f64 = 0.5;
/// At most this many toasts are shown; the oldest info / warning toasts are dropped first, errors only when more
/// errors than this are waiting (they stay in the log and in `error_texts`).
const MAX_VISIBLE: usize = 4;
/// Text prefixes of the history toasts: a new one replaces the visible one of the same group instead of stacking.
const HISTORY_PREFIXES: [&str; 2] = ["Undo: ", "Redo: "];
/// Widest a toast is drawn (points, frame included); a longer text wraps, a word longer than a row (a path) breaks
/// anywhere.
pub const MAX_WIDTH: f32 = 380.0;
/// Gap between the toast stack and the corner of the central view area (points).
const MARGIN: f32 = 12.0;

struct Toast {
    level: ToastLevel,
    text: String,
    /// How many pushes this toast stands for (identical texts, or consecutive Undo / Redo steps of one kind).
    count: u32,
    /// `i.time` of the first frame the toast was shown (restarted by a coalesced push); `None` until then.
    shown_at: Option<f64>,
    /// Shown while the pointer is over the toast (the full path of a folder the text names only by its name).
    detail: Option<String>,
}

impl Toast {
    /// The text as drawn: with the count of the coalesced pushes.
    fn label(&self) -> String {
        if self.count > 1 { format!("{}  (x{})", self.text, self.count) } else { self.text.clone() }
    }
}

/// The history group prefix of a text ("Undo: " / "Redo: "), if it is one.
fn history_prefix(text: &str) -> Option<&'static str> {
    HISTORY_PREFIXES.into_iter().find(|p| text.starts_with(p))
}

/// The toast queue: visible toasts plus the error counter.
#[derive(Default)]
pub struct Toasts {
    visible: Vec<Toast>,
    errors: usize,
    /// Text of every Error-level push, oldest first (the self-test reports them).
    error_texts: Vec<String>,
    /// Screen rects (points) of the toasts drawn by the last `show`, oldest first (self-test asserts).
    drawn: Vec<egui::Rect>,
}

impl Toasts {
    /// Queues and logs a toast. Coalescing (8.9): a text identical to a visible toast does not stack, it counts that
    /// toast up and restarts its timer; an Undo / Redo toast replaces the visible Undo / Redo toast (the count goes up
    /// while the direction stays the same and restarts at 1 when it changes). The newest toast is always last.
    pub fn push(&mut self, level: ToastLevel, text: impl Into<String>) {
        self.push_toast(level, text.into(), None);
    }

    /// `push` with a hover text, e.g. the full path of a folder that the text names by its name only (R3-7). The
    /// detail is logged with the text.
    pub fn push_with_detail(&mut self, level: ToastLevel, text: impl Into<String>, detail: impl Into<String>) {
        self.push_toast(level, text.into(), Some(detail.into()));
    }

    fn push_toast(&mut self, level: ToastLevel, text: String, detail: Option<String>) {
        if let Some(d) = &detail {
            log::info!("toast detail: {d}");
        }
        match level {
            ToastLevel::Info => log::info!("toast: {text}"),
            ToastLevel::Warning => log::warn!("toast: {text}"),
            ToastLevel::Error => {
                log::error!("toast: {text}");
                self.errors += 1;
                self.error_texts.push(text.clone());
            }
        }
        let history = if level == ToastLevel::Error { None } else { history_prefix(&text) };
        let same = self.visible.iter().position(|t| {
            t.level == level && (t.text == text || (history.is_some() && t.level != ToastLevel::Error && history_prefix(&t.text).is_some()))
        });
        let count = match same.map(|i| self.visible.remove(i)) {
            Some(old) if old.text == text || history_prefix(&old.text) == history => old.count.saturating_add(1),
            _ => 1,
        };
        self.visible.push(Toast { level, text, count, shown_at: None, detail });
        while self.visible.len() > MAX_VISIBLE {
            let drop = self.visible.iter().position(|t| t.level != ToastLevel::Error).unwrap_or(0);
            self.visible.remove(drop);
        }
    }

    /// `push(ToastLevel::Info, ..)`.
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(ToastLevel::Info, text);
    }

    /// `push(ToastLevel::Warning, ..)`.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(ToastLevel::Warning, text);
    }

    /// `push(ToastLevel::Error, ..)`.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(ToastLevel::Error, text);
    }

    /// Number of Error-level pushes so far (monotonic; self-test exit code 2).
    pub fn error_count(&self) -> usize {
        self.errors
    }

    /// The texts of all Error-level pushes so far, oldest first (`error_texts()[n..]` = the errors after the n-th).
    pub fn error_texts(&self) -> &[String] {
        &self.error_texts
    }

    /// The texts of the toasts in the queue, oldest first (tests).
    #[cfg(test)]
    pub(crate) fn visible_texts(&self) -> Vec<&str> {
        self.visible.iter().map(|t| t.text.as_str()).collect()
    }

    /// The drawn labels (with counts) of the toasts in the queue, oldest first (tests and self-test asserts).
    pub(crate) fn visible_labels(&self) -> Vec<String> {
        self.visible.iter().map(Toast::label).collect()
    }

    /// The screen rects (points) of the toasts drawn by the last `show`, oldest first (self-test asserts).
    pub(crate) fn drawn_rects(&self) -> &[egui::Rect] {
        &self.drawn
    }

    /// Draws the toasts: a stack in an `egui::Area` whose bottom-right corner sits `MARGIN` inside the bottom-right
    /// corner of `within` (the central view area, so the toasts never cover the side panel, the tool bar or the status
    /// bar), each at most `MAX_WIDTH` wide with its text wrapped. Info and warnings fade after 4 s, errors stay until
    /// dismissed. The area takes pointer input only while an error (with its dismiss button) is shown, so the timed
    /// toasts never swallow a click meant for the view below them; the hover text of a toast with a detail is shown
    /// from the pointer position, which needs no input.
    pub fn show(&mut self, ctx: &egui::Context, within: egui::Rect) {
        self.drawn.clear();
        let now = ctx.input(|i| i.time);
        for toast in &mut self.visible {
            toast.shown_at.get_or_insert(now);
        }
        self.visible.retain(|t| t.level == ToastLevel::Error || t.shown_at.is_none_or(|t0| now - t0 < FADE_SECONDS));
        if self.visible.is_empty() {
            return;
        }

        // Expire the timed toasts without repainting continuously: wake up when the next fade step is due.
        let next_due = self
            .visible
            .iter()
            .filter(|t| t.level != ToastLevel::Error)
            .filter_map(|t| t.shown_at.map(|t0| t0 + FADE_SECONDS - now))
            .map(|left| if left > FADE_OUT_SECONDS { left - FADE_OUT_SECONDS } else { 1.0 / 30.0 })
            .fold(f64::INFINITY, f64::min);
        if next_due.is_finite() {
            ctx.request_repaint_after(Duration::from_secs_f64(next_due.max(0.0)));
        }

        let has_error = self.visible.iter().any(|t| t.level == ToastLevel::Error);
        let hover = ctx.pointer_hover_pos();
        let mut dismiss = None;
        let mut drawn = Vec::with_capacity(self.visible.len());
        let corner = within.right_bottom() - egui::vec2(MARGIN, MARGIN);
        let max_width = MAX_WIDTH.min(within.width() - 2.0 * MARGIN).max(120.0);
        egui::Area::new(egui::Id::unique("wte_toasts"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(corner)
            .constrain_to(within)
            .interactable(has_error)
            .show(ctx, |ui| {
                ui.set_max_width(max_width);
                // Right-aligned rows: a short toast hugs the right edge like the long ones.
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    for (index, toast) in self.visible.iter().enumerate() {
                        let left = toast.shown_at.map_or(FADE_SECONDS, |t0| FADE_SECONDS - (now - t0));
                        let opacity =
                            if toast.level == ToastLevel::Error { 1.0 } else { (left / FADE_OUT_SECONDS).clamp(0.0, 1.0) as f32 };
                        let (accent, label) = match toast.level {
                            ToastLevel::Info => (egui::Color32::from_rgb(90, 160, 230), "Info"),
                            ToastLevel::Warning => (egui::Color32::from_rgb(235, 175, 60), "Warning"),
                            ToastLevel::Error => (egui::Color32::from_rgb(230, 80, 70), "Error"),
                        };
                        let frame = egui::Frame::popup(ui.style())
                            .fill(egui::Color32::from_rgb(38, 40, 46))
                            .stroke(egui::Stroke::new(1.0, accent))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .multiply_with_opacity(opacity);
                        // Room for the dismiss button and the item gap before it, so the row stays inside
                        // `max_width` (the area would grow and leave the corner otherwise).
                        let gap = ui.spacing().item_spacing.x;
                        let text = toast.label();
                        // The row is as wide as its unwrapped text needs, at most `max_width`: a short toast stays
                        // small in the corner, a long one wraps (a word longer than the row breaks anywhere).
                        let font = egui::TextStyle::Body.resolve(ui.style());
                        let measure = |s: &str| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), accent).size().x;
                        let button = if toast.level == ToastLevel::Error {
                            measure("x") + 2.0 * ui.spacing().button_padding.x + gap + 4.0
                        } else {
                            0.0
                        };
                        let chrome = 2.0 * 10.0 + 2.0 + 2.0; // inner margin, stroke, rounding slack
                        let row_width =
                            (chrome + measure(label) + gap + measure(&text) + button).min(ui.available_width());
                        let row = ui.allocate_ui_with_layout(
                            egui::vec2(row_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                frame.show(ui, |ui| {
                                    ui.set_opacity(opacity);
                                    // The frame fills its row, so every toast ends on the corner's right edge.
                                    ui.set_min_width(ui.available_width());
                                    ui.horizontal_top(|ui| {
                                        ui.colored_label(accent, label);
                                        let text_width = (ui.available_width() - button).max(40.0);
                                        let text = egui::RichText::new(text).color(egui::Color32::from_gray(225));
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(text_width, 0.0),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Wrap)),
                                        );
                                        if toast.level == ToastLevel::Error
                                            && ui.small_button("x").on_hover_text("Dismiss").clicked()
                                        {
                                            dismiss = Some(index);
                                        }
                                    });
                                })
                            },
                        );
                        let response = row.inner.response;
                        if let Some(detail) = &toast.detail
                            && hover.is_some_and(|p| response.rect.contains(p))
                        {
                            response.show_tooltip_text(detail.as_str());
                        }
                        drawn.push(response.rect);
                        if index + 1 < self.visible.len() {
                            ui.add_space(4.0);
                        }
                    }
                });
            });
        self.drawn = drawn;
        if let Some(index) = dismiss {
            self.visible.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The central view area of the layout tests (points).
    const SCREEN: egui::Rect = egui::Rect { min: egui::pos2(44.0, 24.0), max: egui::pos2(960.0, 776.0) };

    #[test]
    fn push_counts_errors_and_coalesces_visible_duplicates() {
        let mut toasts = Toasts::default();
        assert_eq!(toasts.error_count(), 0);
        toasts.info("saved");
        toasts.warn("no UV mapping");
        toasts.warn("no UV mapping");
        assert_eq!(toasts.visible.len(), 2);
        assert_eq!(toasts.visible_labels(), ["saved", "no UV mapping  (x2)"]);
        assert_eq!(toasts.error_count(), 0);
        toasts.error("export failed");
        toasts.push(ToastLevel::Error, String::from("export failed"));
        assert_eq!(toasts.visible.len(), 3);
        assert_eq!(toasts.error_count(), 2);
        assert_eq!(toasts.error_texts(), ["export failed", "export failed"]);
        // A repeated toast moves to the newest place.
        toasts.info("saved");
        assert_eq!(toasts.visible_texts(), ["no UV mapping", "export failed", "saved"]);
    }

    #[test]
    fn undo_and_redo_steps_share_one_toast_with_a_count() {
        let mut toasts = Toasts::default();
        toasts.info("Opened model");
        for label in ["Brush Stroke", "Fill", "Add Stamp", "Brush Stroke"] {
            toasts.info(format!("Undo: {label}"));
        }
        assert_eq!(toasts.visible_labels(), ["Opened model", "Undo: Brush Stroke  (x4)"]);
        // The direction changes: the toast is replaced and the count restarts.
        toasts.info("Redo: Brush Stroke");
        toasts.info("Redo: Add Stamp");
        assert_eq!(toasts.visible_labels(), ["Opened model", "Redo: Add Stamp  (x2)"]);
        toasts.info("Undo: Add Stamp");
        assert_eq!(toasts.visible_labels(), ["Opened model", "Undo: Add Stamp"]);
        // A hundred undo steps are still one toast.
        for i in 0..100 {
            toasts.info(format!("Undo: step {i}"));
        }
        assert_eq!(toasts.visible.len(), 2);
        assert_eq!(toasts.visible_labels()[1], "Undo: step 99  (x101)");
        // An error that happens to start like a history toast is not merged into it.
        toasts.error("Undo: failed");
        assert_eq!(toasts.visible.len(), 3);
    }

    #[test]
    fn the_queue_is_capped_and_keeps_errors() {
        let mut toasts = Toasts::default();
        toasts.error("first error");
        for i in 0..20 {
            toasts.info(format!("info {i}"));
        }
        assert_eq!(toasts.visible.len(), MAX_VISIBLE);
        assert!(toasts.visible.iter().any(|t| t.text == "first error"));
        assert_eq!(toasts.visible.last().unwrap().text, "info 19");
        // Only errors beyond the cap push out older errors.
        for i in 0..MAX_VISIBLE {
            toasts.error(format!("error {i}"));
        }
        assert_eq!(toasts.visible.len(), MAX_VISIBLE);
        assert!(toasts.visible.iter().all(|t| t.level == ToastLevel::Error));
        assert_eq!(toasts.error_texts().len(), MAX_VISIBLE + 1);
    }

    #[test]
    fn timed_toasts_expire_and_errors_stay() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.info("hello");
        toasts.error("broken");
        let mut input = egui::RawInput { time: Some(0.0), ..Default::default() };
        crate::theme::run_test_pass(&ctx, input.clone(), |ui| toasts.show(ui.ctx(), SCREEN));
        assert_eq!(toasts.visible.len(), 2);
        input.time = Some(FADE_SECONDS + 0.1);
        crate::theme::run_test_pass(&ctx, input, |ui| toasts.show(ui.ctx(), SCREEN));
        assert_eq!(toasts.visible.len(), 1);
        assert_eq!(toasts.visible[0].text, "broken");
    }

    #[test]
    fn a_coalesced_push_restarts_the_timer() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.info("Undo: a");
        let at = |t: f64| egui::RawInput { time: Some(t), ..Default::default() };
        crate::theme::run_test_pass(&ctx, at(0.0), |ui| toasts.show(ui.ctx(), SCREEN));
        toasts.info("Undo: b");
        crate::theme::run_test_pass(&ctx, at(3.0), |ui| toasts.show(ui.ctx(), SCREEN));
        crate::theme::run_test_pass(&ctx, at(FADE_SECONDS + 1.0), |ui| toasts.show(ui.ctx(), SCREEN));
        assert_eq!(toasts.visible_labels(), ["Undo: b  (x2)"], "shown at 3 s, still visible at 5 s");
    }

    #[test]
    fn toasts_stay_in_the_view_corner_and_wrap_long_paths() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        let path = r"E:\claude_programs\wrapped_texture_editor\target\round3_audit\runs\workflow_path_uv_tiled_grid\shot_export";
        toasts.push_with_detail(ToastLevel::Info, "Exported 8 files to shot_export", path);
        toasts.error(format!(r"Cannot write {path}\very_long_file_name_without_any_break_points.png"));
        toasts.info("Redo: Edit Stamp");
        let input = |t: f64| egui::RawInput {
            time: Some(t),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            ..Default::default()
        };
        // The first pass of an area only measures it; the second draws it where it belongs.
        for t in [0.0, 0.1, 0.2] {
            crate::theme::run_test_pass(&ctx, input(t), |ui| toasts.show(ui.ctx(), SCREEN));
        }
        let rects = toasts.drawn_rects();
        assert_eq!(rects.len(), 3);
        for r in rects {
            assert!(r.width() <= MAX_WIDTH + 0.5, "{r:?} is wider than {MAX_WIDTH}");
            assert!(SCREEN.contains_rect(*r), "{r:?} leaves the view area {SCREEN:?}");
            assert!(r.right() > SCREEN.right() - MARGIN - 2.0, "{r:?} is not at the right edge: {rects:?}");
        }
        assert!(rects[1].height() > rects[2].height() * 1.5, "the long error wraps onto more rows: {rects:?}");
        assert!(rects[2].bottom() > SCREEN.bottom() - MARGIN - 2.0, "the newest toast is at the bottom: {rects:?}");
        assert!(rects[2].width() < 250.0, "a short toast stays small: {rects:?}");
        assert!(rects[0].top() < rects[1].top() && rects[1].top() < rects[2].top(), "oldest on top: {rects:?}");
        assert_eq!(toasts.visible[0].detail.as_deref(), Some(path));
    }

    /// True when a text shape of `out` contains `needle`.
    fn draws_text(out: &egui::FullOutput, needle: &str) -> bool {
        fn walk(shape: &egui::Shape, needle: &str) -> bool {
            match shape {
                egui::Shape::Text(t) => t.galley.text().contains(needle),
                egui::Shape::Vec(v) => v.iter().any(|s| walk(s, needle)),
                _ => false,
            }
        }
        out.shapes.iter().any(|c| walk(&c.shape, needle))
    }

    /// R3-7 review: hovering a toast with a detail shows the detail (the full path) as a tooltip, although the timed
    /// toasts take no pointer input; without the pointer over it, the path is not drawn.
    #[test]
    fn hovering_a_toast_shows_its_full_path() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        let path = "E:/some/long/folder/round3_audit_marker/shot_export";
        toasts.push_with_detail(ToastLevel::Info, "Exported 8 files to shot_export", path);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let input = |t: f64, events: Vec<egui::Event>| egui::RawInput {
            time: Some(t),
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let pass = |t: f64, events: Vec<egui::Event>, toasts: &mut Toasts| {
            let mut out = ctx.run_ui(input(t, events), |ui| toasts.show(ui.ctx(), SCREEN));
            out.textures_delta.clear();
            out
        };
        for t in [0.0, 0.1] {
            let out = pass(t, vec![], &mut toasts);
            assert!(!draws_text(&out, "round3_audit_marker"), "no tooltip without the pointer");
        }
        let rect = toasts.drawn_rects()[0];
        // Pointer elsewhere: still no tooltip.
        let out = pass(0.2, vec![egui::Event::PointerMoved(egui::pos2(300.0, 300.0))], &mut toasts);
        assert!(!draws_text(&out, "round3_audit_marker"));
        let mut shown = false;
        for (i, t) in [0.3, 0.4, 1.2, 1.4].into_iter().enumerate() {
            let events = if i == 0 { vec![egui::Event::PointerMoved(rect.center())] } else { vec![] };
            let out = pass(t, events, &mut toasts);
            shown |= draws_text(&out, "round3_audit_marker");
        }
        assert!(shown, "the full path is shown while the pointer is over the toast {rect:?}");
    }
}
