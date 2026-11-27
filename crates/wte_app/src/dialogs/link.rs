// Owner: stickers P9a (STICKER_PLAN 5.1, 5.5, 5.7, 13.1; ARCHITECTURE 8.8, 8.16). The in-window questions of the
// editor link (`Dialog::Link`): save first, the shared picture, a text stamp to convert, send WTE's changes first, the
// warning before a send, already open, the conflict ("both changed" / "saved over"), changed while closed, a canvas
// too large, OK messages, and (stickers P9b) the chooser "Edit in which program?" with "Always use this one" and the
// question before a link opens in a second editor. They wait inside WTE's window; nothing raises, focuses or flashes it. A button closes
// the dialog and queues its actions; Esc and a click outside are Cancel (`dispatch::close_dialog`:
// "Decide later" for a conflict, "Ignore" for a change while closed).

use eframe::egui;

use crate::action::AppAction;
use crate::link::LinkQuestion;
use crate::state::{Dialog, PanelHit};
use crate::view::ViewCx;

/// Draws question `q`.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, q: &LinkQuestion) {
    let mut hits = Vec::new();
    let response = super::modal("link_question").show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading(&q.title);
        ui.add_space(4.0);
        for (i, line) in q.lines.iter().enumerate() {
            if i == 0 {
                ui.add(egui::Label::new(line).wrap());
            } else {
                ui.add(egui::Label::new(egui::RichText::new(line).weak()).wrap());
            }
        }
        // Stickers P9b: the chooser's box "Always use this one" (sets the default editor with the button pressed).
        if q.kind == crate::link::QuestionKind::ChooseEditor {
            ui.add_space(4.0);
            let resp = ui.checkbox(&mut cx.state.link_ui.chooser_always, "Always use this one");
            hits.push((PanelHit::LinkChooserAlways, resp.rect));
        }
        ui.add_space(8.0);
        let mut pressed = None;
        super::button_row(ui, |ui| {
            // Right to left: the last button (the default) is drawn first.
            for (i, b) in q.buttons.iter().enumerate().rev() {
                let resp = ui.button(&b.label);
                hits.push((PanelHit::LinkDialogButton(i as u8), resp.rect));
                if resp.clicked() {
                    pressed = Some(i);
                }
            }
        });
        pressed
    });
    cx.state.panel_rects.extend(hits);
    if let Some(actions) = response.inner.map(|i| crate::link::button_actions(cx.state, &q.buttons[i])) {
        // Closed without the Cancel meaning of `CloseDialog` (a conflict's "Decide later"), then the button's actions.
        cx.state.push(AppAction::ShowDialog(Dialog::None));
        cx.state.actions.extend(actions);
    } else if super::backdrop_clicked(&response) {
        cx.state.push(AppAction::CloseDialog);
    }
}
