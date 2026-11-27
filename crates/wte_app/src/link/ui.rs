//! The link's controls (STICKER_PLAN 3.2, 3.9, 5.1, 5.5, 13.1; stickers P9a): the Sticker tab's link block under the
//! selected stamp (the badge, "Editor: <name>" and the file, Edit in / Update now / Send to / Show file / Unlink,
//! Restore and the command-line conversion when they apply) and the canvas header's badge with "Edit in <name>".
//! Every control is a button with a `PanelHit` rect (tablet rule of 3.10); every text takes the editor's name.
//!
//! Stickers P9b (13.1): beside "Edit in <editor>" a small button **Edit in...** opens the editor list
//! ([`editor_list`]: Photoshop when known, PhotoCraft, the other program when chosen, "Other program...", and the
//! "Default: <name>" choices); the Layer menu and a stamp layer's row menu have the same list as a submenu "Edit in".
//! While two or more editors are known and none is the default, the main button reads "Edit in..." and opens the
//! chooser (the list button is then left out: the chooser offers every editor and "Other program...").

use eframe::egui;
use egui::Rect;
use wte_core::link::{LinkFile, LinkState, texts};

use super::editor::{self, ExternalEditor};
use super::{EditRequest, LinkAction, LinkTarget, SendRequest};
use crate::action::AppAction;
use crate::state::{AppState, PanelHit};

/// The sticker of a target (`None` for an image or text stamp: not linked yet).
fn sticker_of(state: &AppState, target: LinkTarget) -> Option<wte_core::StickerId> {
    match target {
        LinkTarget::Stamp(s) => super::sticker_of_stamp(state, s),
        LinkTarget::Sticker(id) => Some(id),
    }
}

/// The colour of a badge state.
fn badge_color(ui: &egui::Ui, st: &LinkState) -> egui::Color32 {
    match st {
        LinkState::Linked => crate::theme::ACCENT,
        LinkState::ChangedInWte | LinkState::ChangedInEditor => ui.visuals().warn_fg_color,
        _ => ui.visuals().error_fg_color,
    }
}

/// The Sticker tab's link block for the selected stamp (or a tray sticker).
pub(crate) fn tab_block(ui: &mut egui::Ui, state: &mut AppState, target: LinkTarget, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    editor::ensure_discovered(state, false);
    let sticker = sticker_of(state, target);
    let linked = sticker.and_then(|id| super::badge(state, id).map(|b| (id, b)));
    let editor = sticker.map_or_else(|| super::editor_name_new(state), |id| super::editor_name(state, id));
    let main_label = super::edit_in_label(state, sticker);
    let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
    if let Some((id, (st, text))) = &linked {
        ui.horizontal_wrapped(|ui| {
            ui.label("Link:");
            ui.colored_label(badge_color(ui, st), text);
        });
        ui.weak(format!("Editor: {editor}"));
        ui.weak(super::file_name(state, *id));
        if let Some(line) = state.link_ui.sessions.get(id).and_then(|s| s.unreadable.clone()) {
            ui.colored_label(ui.visuals().warn_fg_color, line);
        }
        if *st == LinkState::NeedsMerged {
            let key = state.doc().and_then(|d| d.link(*id)).map(|l| l.editor.clone()).unwrap_or_default();
            ui.add(egui::Label::new(texts::needs_merged(&key, &editor)).wrap());
        }
    }
    ui.horizontal_wrapped(|ui| {
        let resp = ui
            .button(format!("{main_label}{on_pc}"))
            .on_hover_text(format!("Open the sticker's layers in {editor}; Ctrl+S there updates the sticker on the model"));
        hits.push((PanelHit::LinkEditIn, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::EditIn(EditRequest::new(target))));
        }
        list_button(ui, state, target, &main_label, PanelHit::LinkEditInList, actions, hits);
        let Some((id, (st, _))) = &linked else { return };
        let id = *id;
        let resp = ui.button("Update now").on_hover_text(format!("Take the file as {editor} saved it"));
        hits.push((PanelHit::LinkUpdateNow, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::UpdateNow(id)));
        }
        let resp = ui.button(texts::send_to(&editor)).on_hover_text("Write WTE's sticker into the file (the old file is kept as .bak.psd)");
        hits.push((PanelHit::LinkSend, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::Send(SendRequest::new(id))));
        }
        let resp = ui.button(format!("Show file in Explorer{on_pc}"));
        hits.push((PanelHit::LinkShowFile, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::ShowFile(id)));
        }
        let resp = ui.button("Unlink").on_hover_text("Keep the picture and stop following the file (the file stays on disk)");
        hits.push((PanelHit::LinkUnlink, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::Unlink(id)));
        }
        let in_folder = state.doc().and_then(|d| d.link(id)).is_some_and(|l| matches!(l.file, LinkFile::InFolder(_)));
        if *st == LinkState::Missing && in_folder && state.session.as_ref().is_some_and(|s| s.path.is_some()) {
            let resp = ui.button("Restore from project").on_hover_text("Write the copy the project keeps back into the links folder");
            hits.push((PanelHit::LinkRestore, resp.rect));
            if resp.clicked() {
                actions.push(AppAction::Link(LinkAction::Restore(id)));
            }
        }
        let key = state.doc().and_then(|d| d.link(id)).map(|l| l.editor.clone()).unwrap_or_default();
        if *st == LinkState::NeedsMerged && texts::offers_cli_convert(&key) && super::flows::cli_exe(&state.prefs).is_some() {
            let resp = ui.button(format!("Convert with {editor}'s command-line tool"));
            hits.push((PanelHit::LinkConvert, resp.rect));
            if resp.clicked() {
                actions.push(AppAction::Link(LinkAction::Convert(id)));
            }
        }
    });
}

/// The canvas header's link part: the badge (when linked) and "Edit in <editor>".
pub(crate) fn header(ui: &mut egui::Ui, state: &mut AppState, target: LinkTarget, narrow: bool, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    editor::ensure_discovered(state, false);
    let sticker = sticker_of(state, target);
    let editor = sticker.map_or_else(|| super::editor_name_new(state), |id| super::editor_name(state, id));
    let main_label = super::edit_in_label(state, sticker);
    if let Some((st, text)) = sticker.and_then(|id| super::badge(state, id)) {
        let short = if narrow && text.len() > 24 { "link".to_owned() } else { text.clone() };
        ui.colored_label(badge_color(ui, &st), short).on_hover_text(text);
    }
    let resp = ui.button(&main_label).on_hover_text(format!("Open the sticker's layers in {editor}"));
    hits.push((PanelHit::StickerHeaderEditIn, resp.rect));
    if resp.clicked() {
        actions.push(AppAction::Link(LinkAction::EditIn(EditRequest::new(target))));
    }
    list_button(ui, state, target, &main_label, PanelHit::StickerHeaderEditInList, actions, hits);
}

/// The small "Edit in..." button that opens the editor list (left out while the main button itself reads "Edit in...").
fn list_button(
    ui: &mut egui::Ui,
    state: &AppState,
    target: LinkTarget,
    main_label: &str,
    hit: PanelHit,
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, Rect)>,
) {
    if main_label == LIST_LABEL {
        return;
    }
    let resp = ui.menu_button(LIST_LABEL, |ui| editor_list(ui, state, target, actions, hits));
    hits.push((hit, resp.response.on_hover_text("Choose the program, or the default one").rect));
}

/// The label of the list button (and of the main button while the chooser has to ask).
pub(crate) const LIST_LABEL: &str = "Edit in...";

/// The editors the list offers (13.1): Photoshop when its program is known, PhotoCraft always (it can be located), the
/// other program when one was chosen.
pub(crate) fn listed_editors(state: &AppState) -> Vec<ExternalEditor> {
    let known = &state.link_ui.known;
    let mut out = Vec::new();
    if known.contains(&ExternalEditor::Photoshop) {
        out.push(ExternalEditor::Photoshop);
    }
    out.push(ExternalEditor::PhotoCraft);
    out.extend(known.iter().filter(|e| matches!(e, ExternalEditor::Other(_))).cloned());
    out
}

/// The editor list (13.1), drawn inside a menu or a submenu: one item per listed editor ("Edit in" that editor),
/// **Other program...**, a separator, then **Default: <name>** radio items. Each item closes the menu; the item rects go
/// to `hits` as `LinkEditorItem(i)`, top to bottom.
pub(crate) fn editor_list(ui: &mut egui::Ui, state: &AppState, target: LinkTarget, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    let mut index = 0u8;
    let mut hit = |rect: Rect| {
        hits.push((PanelHit::LinkEditorItem(index), rect));
        index = index.saturating_add(1);
    };
    let listed = listed_editors(state);
    for e in &listed {
        let resp = ui.button(e.name());
        hit(resp.rect);
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::EditIn(EditRequest::in_editor(target, e.key()))));
            ui.close();
        }
    }
    let resp = ui.button("Other program...");
    hit(resp.rect);
    if resp.clicked() {
        actions.push(AppAction::Link(LinkAction::PickOther(EditRequest::new(target))));
        ui.close();
    }
    ui.separator();
    let default = state.prefs.default_editor.as_deref();
    for e in &listed {
        let resp = ui.radio(default == Some(e.key()), format!("Default: {}", e.name()));
        hit(resp.rect);
        if resp.clicked() {
            actions.push(AppAction::Link(LinkAction::SetDefaultEditor(Some(e.key().to_owned()))));
            ui.close();
        }
    }
}
