// Owner: stickers P6b (STICKER_PLAN 3.1 "Shared pictures", E6; ARCHITECTURE 8.8). The question before the first
// content edit, in the Sticker view, of a sticker (or an image) that several stamps show: Edit all / Only this one /
// Cancel. The answer is a `StickerAction::AnswerShared`; Esc and a click outside are Cancel (`dispatch::close_dialog`).

use eframe::egui;

use crate::action::AppAction;
use crate::state::PanelHit;
use crate::sticker::StickerAction;
use crate::view::ViewCx;

/// The question's text for `users` stamps.
pub(crate) fn question(users: usize) -> String {
    format!("This picture is used by {users} stickers.")
}

/// Draws the question.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, users: usize) {
    let mut hits = Vec::new();
    let response = super::modal("sticker_shared").show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.heading("Shared picture");
        ui.add_space(4.0);
        ui.add(egui::Label::new(question(users)).wrap());
        ui.weak("Edit all: every stamp that shows it changes. Only this one: this stamp gets its own copy first.");
        ui.add_space(8.0);
        let mut answer = None;
        super::button_row(ui, |ui| {
            let resp = ui.button("Cancel");
            hits.push((PanelHit::SharedCancel, resp.rect));
            if resp.clicked() {
                answer = Some(AppAction::CloseDialog);
            }
            let resp = ui.button("Only this one");
            hits.push((PanelHit::SharedOnlyThis, resp.rect));
            if resp.clicked() {
                answer = Some(AppAction::Sticker(StickerAction::AnswerShared(Some(wte_core::ShareScope::ThisStamp))));
            }
            let resp = ui.button("Edit all");
            hits.push((PanelHit::SharedEditAll, resp.rect));
            if resp.clicked() {
                answer = Some(AppAction::Sticker(StickerAction::AnswerShared(Some(wte_core::ShareScope::AllUsers))));
            }
        });
        answer
    });
    cx.state.panel_rects.extend(hits);
    if let Some(action) = response.inner {
        cx.state.push(action);
    } else if super::backdrop_clicked(&response) {
        cx.state.push(AppAction::CloseDialog);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_question_names_the_number_of_stamps() {
        assert_eq!(super::question(3), "This picture is used by 3 stickers.");
    }
}
