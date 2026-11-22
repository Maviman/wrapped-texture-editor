// Owner: app-dialogs (A5). Message dialog of ARCHITECTURE.md 8.8: a title, scrollable lines (import / load warnings,
// error details) and Close.

use eframe::egui;

use crate::view::ViewCx;

/// Draws a title, scrollable lines and Close.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, title: &str, lines: &[String]) {
    let max = super::max_dialog_size(ctx);
    let response = super::modal("message").show(ctx, |ui| {
        let width = max.x.min(620.0);
        ui.set_min_width(width.min(380.0));
        ui.set_max_width(width);
        ui.heading(title);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("wte_message_lines")
            .max_height((max.y - 130.0).max(80.0))
            .auto_shrink([true, true])
            .show(ui, |ui| {
                if lines.is_empty() {
                    ui.weak("No details.");
                }
                for line in lines {
                    ui.add(egui::Label::new(line.as_str()).wrap());
                }
            });
        ui.separator();
        let mut close = false;
        super::button_row(ui, |ui| close = ui.button("Close").clicked());
        close
    });
    if response.inner || super::backdrop_clicked(&response) {
        super::close(cx);
    }
}
