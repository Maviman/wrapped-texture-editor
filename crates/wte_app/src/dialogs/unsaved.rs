// Owner: app-dialogs (A5). Unsaved-changes prompt of ARCHITECTURE.md 8.8: Save / Don't Save / Cancel before New,
// Open, Open Sample and Quit.

use eframe::egui;

use crate::action::AppAction;
use crate::state::Pending;
use crate::view::ViewCx;

/// What the user is about to do, as the end of the question "Save the changes ... before ...?".
pub(super) fn operation_text(then: &Pending) -> &'static str {
    match then {
        Pending::NewFromModel(_) => "creating a new document",
        Pending::OpenProject(_) => "opening another project",
        Pending::OpenSample(_) => "opening the sample",
        Pending::Quit => "quitting",
    }
}

/// Draws the Save / Don't Save / Cancel prompt shown before `then` runs.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, then: &Pending) {
    let name = cx.state.doc().map_or_else(|| "the document".to_owned(), |d| format!("\"{}\"", d.name));
    let response = super::modal("unsaved").show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.heading("Unsaved changes");
        ui.add_space(4.0);
        ui.add(egui::Label::new(format!("Save the changes to {name} before {}?", operation_text(then))).wrap());
        ui.weak("Your changes are lost if you don't save them.");
        // Stickers P9a (STICKER_PLAN 5.3, review B8): stickers linked since the last save are forgotten without one.
        if let Some(line) = crate::link::unsaved_line(cx.state) {
            ui.add(egui::Label::new(egui::RichText::new(line).color(super::warn_color(ui))).wrap());
        }
        ui.add_space(8.0);
        let mut answer = None;
        super::button_row(ui, |ui| {
            if ui.button("Cancel").clicked() {
                answer = Some(AppAction::CloseDialog);
            }
            if ui.button("Don't Save").clicked() {
                answer = Some(AppAction::ConfirmPending(then.clone()));
            }
            if ui.button("Save").clicked() {
                answer = Some(AppAction::SaveThen(then.clone()));
            }
        });
        answer
    });
    if let Some(action) = response.inner {
        cx.state.push(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wte_core::SampleKind;

    #[test]
    fn every_pending_operation_has_a_text() {
        assert_eq!(operation_text(&Pending::Quit), "quitting");
        assert_eq!(operation_text(&Pending::OpenSample(SampleKind::Cube)), "opening the sample");
        assert_eq!(operation_text(&Pending::NewFromModel(None)), "creating a new document");
        assert_eq!(operation_text(&Pending::OpenProject(None)), "opening another project");
    }
}
