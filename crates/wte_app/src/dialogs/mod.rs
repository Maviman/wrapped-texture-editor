// Owner: app-dialogs (A5). Modal dialogs of ARCHITECTURE.md 8.8: import, export (with the embedded live preview),
// unsaved changes, progress, message, about / shortcuts. Dialogs change their own fields directly and close only
// through actions handled by dispatch; they never call `rfd` and never read keys (Esc arrives as `AppAction::Cancel`).

use eframe::egui;

use crate::action::AppAction;
use crate::state::Dialog;
use crate::view::ViewCx;

pub mod about;
pub mod companion;
pub mod export;
pub mod import;
pub mod message;
pub mod progress;
pub mod sticker_shared;
pub mod tablet;
pub mod unsaved;

/// UI-only state of the dialogs. Import: egui textures of the UV thumbnails; export: the cached plan and the options
/// it was built from.
pub struct DialogsState {
    /// The uploaded UV-layout thumbnails of the import dialog that is open.
    import_thumbs: Option<import::ThumbCache>,
    /// The file list of the export dialog that is open.
    export_plan: Option<export::PlanCache>,
}

impl Default for DialogsState {
    fn default() -> DialogsState {
        DialogsState::new()
    }
}

impl DialogsState {
    /// Empty caches.
    pub fn new() -> DialogsState {
        DialogsState { import_thumbs: None, export_plan: None }
    }
}

/// Draws the dialog in `state.dialog` (`egui::Modal`) and, when `state.busy` is modal, the progress modal on top.
///
/// The dialog is taken out of the state while it is drawn (so its own state and `cx` can be borrowed at the same
/// time) and is always put back; dialogs close only through actions handled by dispatch.
pub fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, ds: &mut DialogsState) {
    let mut dialog = std::mem::replace(&mut cx.state.dialog, Dialog::None);
    match &mut dialog {
        Dialog::None => {}
        Dialog::Import(st) => import::show(ctx, cx, ds, st),
        Dialog::Unsaved { then } => unsaved::show(ctx, cx, then),
        Dialog::Export(st) => export::show(ctx, cx, ds, st),
        Dialog::Message { title, lines } => message::show(ctx, cx, title, lines),
        Dialog::About => about::show_about(ctx, cx),
        Dialog::Shortcuts => about::show_shortcuts(ctx, cx),
        Dialog::TabletHelp => tablet::show(ctx, cx),
        Dialog::Companion => companion::show(ctx, cx),
        Dialog::CloseOpenings(st) => crate::brush_flows::show_openings_dialog(ctx, cx, st),
        Dialog::StickerShared { users } => sticker_shared::show(ctx, cx, *users),
    }
    // Always put back: dialogs never assign `state.dialog` themselves, they push actions.
    cx.state.dialog = dialog;

    // Caches of dialogs that are no longer open are dropped (the thumbnail textures are freed with their handles).
    if !matches!(cx.state.dialog, Dialog::Import(_)) {
        ds.import_thumbs = None;
    }
    if !matches!(cx.state.dialog, Dialog::Export(_)) {
        ds.export_plan = None;
    }

    if cx.state.busy.as_ref().is_some_and(|b| b.modal) {
        progress::show(ctx, cx);
    }
}

/// The modal every dialog uses (a stable id per dialog kind).
pub(crate) fn modal(name: &'static str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(("wte_dialog", name)))
}

/// True when the backdrop of a modal was clicked (the "click outside" close of Message / About / Shortcuts). Esc is
/// deliberately not looked at here: it reaches the dialogs only as `AppAction::Cancel` (8.8, 8.10).
pub(crate) fn backdrop_clicked<T>(response: &egui::ModalResponse<T>) -> bool {
    response.backdrop_response.clicked()
}

/// A right-aligned row of buttons at the bottom of a dialog; the closure adds them right to left.
pub(crate) fn button_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
}

/// Text colour of error lines inside dialogs.
pub(crate) fn error_color(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().error_fg_color
}

/// Text colour of warning lines inside dialogs.
pub(crate) fn warn_color(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().warn_fg_color
}

/// The largest dialog size that still fits the window with a margin (points).
pub(crate) fn max_dialog_size(ctx: &egui::Context) -> egui::Vec2 {
    let screen = ctx.content_rect().size();
    egui::vec2((screen.x - 48.0).max(320.0), (screen.y - 48.0).max(240.0))
}

/// Pushes `CloseDialog`.
pub(crate) fn close(cx: &mut ViewCx<'_>) {
    cx.state.push(AppAction::CloseDialog);
}
