// Owner: app-dialogs (A5). Progress modal of ARCHITECTURE.md 8.8: title, stage, bar and Cancel (=> `CancelJob`) for
// modal jobs. It covers the whole window while a modal job runs (8.9).

use eframe::egui;

use crate::action::AppAction;
use crate::view::ViewCx;

/// Draws the progress modal (title, stage, bar, Cancel) for `state.busy`.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let Some(busy) = cx.state.busy.clone().filter(|b| b.modal) else { return };
    let modal = super::modal("progress");
    // Above any dialog that is open underneath (e.g. the Import dialog during "Generate automatic UVs").
    ctx.move_to_top(modal.area.layer());
    let response = modal.show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading(busy.title.as_str());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spinner();
            let stage = if busy.stage.is_empty() { "Working..." } else { busy.stage.as_str() };
            ui.add(egui::Label::new(stage).truncate());
        });
        ui.add(egui::ProgressBar::new(busy.fraction.clamp(0.0, 1.0)).show_percentage());
        ui.add_space(6.0);
        let mut cancel = false;
        super::button_row(ui, |ui| cancel = ui.button("Cancel").clicked());
        cancel
    });
    if response.inner {
        cx.state.push(AppAction::CancelJob);
    }
}
