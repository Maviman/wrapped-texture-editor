// Owner: app-shell (A1). Keyboard shortcuts of ARCHITECTURE.md 8.10: the table, `handle` (the only code that consumes
// key presses) and `shortcut_text` for menus and tooltips.
//
// Action labels (the `label` argument of `shortcut_text`): "New from Model...", "Open Project...", "Save", "Save As...",
// "Export...", "Undo", "Redo", "Delete", "Cancel", "3D", "2D", "Split", "Preview", "Texture on Model", "Wireframe",
// "Frame Model", the tool names "Navigate", "Brush", "Eraser", "Eyedropper", "Fill", "Stamp", "Text", "Transform",
// "Smaller Brush", "Larger Brush", "Softer Brush", "Harder Brush", "New Paint Layer", "Duplicate Layer", "Merge Down",
// "Move Layer Up", "Move Layer Down", "Reset Colors", "Swap Colors"; objects-tabs: "Frame Selection", "Isolate Selected",
// "Show All", "Hide Selected".

use eframe::egui;
use egui::{Key, KeyboardShortcut, Modifiers};

use crate::action::AppAction;
use crate::state::{AppState, ToolKind, ViewMode};

/// One keyboard shortcut.
pub struct ShortcutDef {
    pub action_label: &'static str,
    pub shortcut: egui::KeyboardShortcut,
    /// Menu text override, e.g. "Shift+[" for the `OpenCurlyBracket` binding.
    pub display: Option<&'static str>,
    /// Still fires while a text field has keyboard focus.
    pub text_ok: bool,
    /// `None` = not applicable right now.
    pub make: fn(&AppState) -> Option<AppAction>,
}

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

const fn def(
    action_label: &'static str,
    modifiers: Modifiers,
    key: Key,
    display: Option<&'static str>,
    text_ok: bool,
    make: fn(&AppState) -> Option<AppAction>,
) -> ShortcutDef {
    ShortcutDef { action_label, shortcut: KeyboardShortcut::new(modifiers, key), display, text_ok, make }
}

fn with_doc(state: &AppState, action: AppAction) -> Option<AppAction> {
    state.session.is_some().then_some(action)
}

/// The shortcut table, ordered most-specific-first: of every pair that `Modifiers::matches_logically` could confuse
/// (Ctrl+Shift+S / Ctrl+S, Ctrl+Shift+N / Ctrl+N, Ctrl+Shift+Z / Ctrl+Z, Shift+X / X, Shift+F / F, Shift+H and Alt+H / H)
/// the more specific one comes first.
static TABLE: [ShortcutDef; 40] = [
    // file (fire while typing: they release the focus first, rule 4)
    def("Save As...", CMD_SHIFT, Key::S, None, true, |s| with_doc(s, AppAction::SaveAs(None))),
    def("Save", CMD, Key::S, None, true, |s| with_doc(s, AppAction::Save)),
    def("New Paint Layer", CMD_SHIFT, Key::N, None, false, |s| with_doc(s, AppAction::AddPaintLayer)),
    def("New from Model...", CMD, Key::N, None, true, |_| Some(AppAction::NewFromModel(None))),
    def("Open Project...", CMD, Key::O, None, true, |_| Some(AppAction::OpenProject(None))),
    def("Export...", CMD, Key::E, None, true, |s| with_doc(s, AppAction::ShowExport)),
    // edit
    def("Redo", CMD, Key::Y, None, false, |s| with_doc(s, AppAction::Redo)),
    def("Redo", CMD_SHIFT, Key::Z, None, false, |s| with_doc(s, AppAction::Redo)),
    def("Undo", CMD, Key::Z, None, false, |s| with_doc(s, AppAction::Undo)),
    def("Delete", NONE, Key::Delete, None, false, |s| with_doc(s, AppAction::DeleteSelection)),
    def("Cancel", NONE, Key::Escape, Some("Esc"), false, |_| Some(AppAction::Cancel)),
    // layers
    def("Duplicate Layer", CMD, Key::J, None, false, |s| with_doc(s, AppAction::DuplicateActiveLayer)),
    def("Merge Down", CMD, Key::M, None, false, |s| with_doc(s, AppAction::MergeActiveDown)),
    def("Move Layer Up", CMD, Key::CloseBracket, Some("Ctrl+]"), false, |s| with_doc(s, AppAction::MoveActiveLayer(1))),
    def("Move Layer Down", CMD, Key::OpenBracket, Some("Ctrl+["), false, |s| with_doc(s, AppAction::MoveActiveLayer(-1))),
    // view
    def("3D", NONE, Key::Num1, None, false, |_| Some(AppAction::SetViewMode(ViewMode::View3D))),
    def("2D", NONE, Key::Num2, None, false, |_| Some(AppAction::SetViewMode(ViewMode::View2D))),
    def("Split", NONE, Key::Num3, None, false, |_| Some(AppAction::SetViewMode(ViewMode::Split))),
    def("Preview", NONE, Key::F5, None, true, |s| with_doc(s, AppAction::SetViewMode(ViewMode::Preview))),
    def("Texture on Model", NONE, Key::T, None, false, |_| Some(AppAction::ToggleTexture)),
    def("Wireframe", NONE, Key::W, None, false, |_| Some(AppAction::ToggleWireframe)),
    def("Frame Selection", SHIFT, Key::F, None, false, |s| with_doc(s, AppAction::FrameSelection)),
    def("Frame Model", NONE, Key::F, None, false, |s| with_doc(s, AppAction::FrameModel)),
    // objects (objects-tabs)
    def("Isolate Selected", SHIFT, Key::H, None, false, |s| with_doc(s, AppAction::IsolateSelectedObjects)),
    def("Show All", ALT, Key::H, None, false, |s| with_doc(s, AppAction::ShowAllObjects)),
    def("Hide Selected", NONE, Key::H, None, false, |s| with_doc(s, AppAction::HideSelectedObjects)),
    // tools
    def("Navigate", NONE, Key::V, None, false, |_| Some(AppAction::SetTool(ToolKind::Navigate))),
    def("Brush", NONE, Key::B, None, false, |_| Some(AppAction::SetTool(ToolKind::Brush))),
    def("Eraser", NONE, Key::E, None, false, |_| Some(AppAction::SetTool(ToolKind::Eraser))),
    def("Eyedropper", NONE, Key::I, None, false, |_| Some(AppAction::SetTool(ToolKind::Eyedropper))),
    def("Fill", NONE, Key::G, None, false, |_| Some(AppAction::SetTool(ToolKind::Fill))),
    def("Stamp", NONE, Key::S, None, false, |_| Some(AppAction::SetTool(ToolKind::Stamp))),
    def("Swap Colors", SHIFT, Key::X, None, false, |_| Some(AppAction::SwapColors)),
    def("Text", NONE, Key::X, None, false, |_| Some(AppAction::SetTool(ToolKind::Text))),
    def("Transform", NONE, Key::R, None, false, |_| Some(AppAction::SetTool(ToolKind::Transform))),
    // brush
    def("Smaller Brush", NONE, Key::OpenBracket, Some("["), false, |_| Some(AppAction::BrushSizeStep(-1))),
    def("Larger Brush", NONE, Key::CloseBracket, Some("]"), false, |_| Some(AppAction::BrushSizeStep(1))),
    def("Softer Brush", SHIFT, Key::OpenCurlyBracket, Some("Shift+["), false, |_| Some(AppAction::BrushHardnessStep(-1))),
    def("Harder Brush", SHIFT, Key::CloseCurlyBracket, Some("Shift+]"), false, |_| Some(AppAction::BrushHardnessStep(1))),
    // colours
    def("Reset Colors", NONE, Key::D, None, false, |_| Some(AppAction::ResetColors)),
];

/// All shortcuts, ordered most-specific-first (rule 5 of section 8.10).
pub fn table() -> &'static [ShortcutDef] {
    &TABLE
}

/// Consumes the key presses of this frame and returns the resulting actions; `App::ui` appends them to
/// `state.actions` after the panels and dialogs have run (8.3).
pub fn handle(ctx: &egui::Context, state: &mut AppState) -> Vec<AppAction> {
    let typing = ctx.text_edit_focused();
    // An open popup (combo box, colour popup, menu) owns Esc and Delete: Esc closes the innermost popup first (egui's
    // Popup reads `key_pressed(Escape)` later in this pass, so the key must stay unconsumed), and the next Esc cancels.
    // The state is the previous pass's, which is the popup the user is looking at.
    let popup_open = egui::Popup::is_any_open(ctx);
    let mut out = Vec::new();
    let modal_job = state.busy.as_ref().is_some_and(|b| b.modal);
    if !modal_job {
        let dialog_open = !matches!(state.dialog, crate::state::Dialog::None);
        for def in table() {
            let is_esc = def.shortcut.logical_key == Key::Escape;
            if dialog_open && !is_esc {
                continue;
            }
            if typing && !def.text_ok {
                continue; // not consumed: the text field gets the key
            }
            if is_esc && state.typing_prev {
                continue; // the first Esc only left the text field (egui cleared the focus before this pass)
            }
            if popup_open && (is_esc || def.shortcut.logical_key == Key::Delete) {
                continue; // not consumed: the popup closes on this Esc
            }
            if !ctx.input_mut(|i| i.consume_shortcut(&def.shortcut)) {
                continue;
            }
            if typing && def.text_ok {
                // Release the focus so the field reports `lost_focus()` in this pass and queues its final commit,
                // which is dispatched before this shortcut's action (appended last by App::ui).
                if let Some(id) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
            }
            if let Some(action) = (def.make)(state) {
                out.push(action);
            }
        }
    }
    state.typing_prev = ctx.text_edit_focused();
    out
}

/// The shortcut text of an action label, for menus and tooltips; "" if the label has no binding.
pub fn shortcut_text(ctx: &egui::Context, label: &str) -> String {
    table()
        .iter()
        .find(|d| d.action_label == label)
        .map(|d| d.display.map_or_else(|| ctx.format_shortcut(&d.shortcut), str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Dialog, Prefs};

    #[test]
    fn no_duplicate_bindings() {
        for (i, a) in table().iter().enumerate() {
            for b in &table()[i + 1..] {
                assert!(a.shortcut != b.shortcut, "{} and {} share {:?}", a.action_label, b.action_label, a.shortcut);
            }
        }
    }

    #[test]
    fn no_entry_is_shadowed_by_an_earlier_one() {
        // Pressing exactly the keys of entry `b` must not be consumed by an earlier entry `a`.
        for (j, b) in table().iter().enumerate() {
            for a in &table()[..j] {
                let shadowed = a.shortcut.logical_key == b.shortcut.logical_key
                    && b.shortcut.modifiers.matches_logically(a.shortcut.modifiers);
                assert!(!shadowed, "{} ({:?}) is shadowed by the earlier {}", b.action_label, b.shortcut, a.action_label);
            }
        }
    }

    #[test]
    fn the_contract_bindings_exist() {
        let find = |m: Modifiers, k: Key| table().iter().find(|d| d.shortcut == KeyboardShortcut::new(m, k));
        for (m, k) in [
            (CMD, Key::N),
            (CMD, Key::O),
            (CMD, Key::S),
            (CMD_SHIFT, Key::S),
            (CMD, Key::E),
            (CMD, Key::Z),
            (CMD, Key::Y),
            (CMD_SHIFT, Key::Z),
            (NONE, Key::V),
            (NONE, Key::B),
            (NONE, Key::E),
            (NONE, Key::I),
            (NONE, Key::G),
            (NONE, Key::S),
            (NONE, Key::X),
            (NONE, Key::R),
            (SHIFT, Key::OpenCurlyBracket),
            (SHIFT, Key::CloseCurlyBracket),
            (CMD, Key::J),
            (CMD, Key::M),
            (CMD, Key::CloseBracket),
            (CMD, Key::OpenBracket),
            (NONE, Key::Escape),
            (NONE, Key::Num1),
            (NONE, Key::Num2),
            (NONE, Key::Num3),
            (NONE, Key::F5),
            (NONE, Key::T),
            (NONE, Key::W),
            (NONE, Key::F),
            (NONE, Key::OpenBracket),
            (NONE, Key::CloseBracket),
            (CMD_SHIFT, Key::N),
            (NONE, Key::Delete),
            (NONE, Key::D),
            (SHIFT, Key::X),
            (SHIFT, Key::F),
            (SHIFT, Key::H),
            (ALT, Key::H),
            (NONE, Key::H),
        ] {
            assert!(find(m, k).is_some(), "missing binding {m:?} {k:?}");
        }
        let text_ok: Vec<_> = table().iter().filter(|d| d.text_ok).map(|d| d.action_label).collect();
        assert_eq!(text_ok, ["Save As...", "Save", "New from Model...", "Open Project...", "Export...", "Preview"]);
        assert_eq!(table().len(), 40);
    }

    /// objects-tabs: H, Shift+H, Alt+H and Shift+F need a document and map to their actions; H in a focused text
    /// field types "h".
    #[test]
    fn object_keys_map_to_their_actions_and_stay_in_text_fields() {
        let ctx = egui::Context::default();
        let mut state = AppState::new(Prefs::default(), true);
        assert!(run(&ctx, &mut state, vec![key_event(Key::H, NONE)], NONE).is_empty(), "no document");
        state.session = crate::tools::test_support::state_with(wte_core::SampleKind::Objects).session;
        let a = run(&ctx, &mut state, vec![key_event(Key::H, NONE)], NONE);
        assert!(matches!(a[..], [AppAction::HideSelectedObjects]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::H, SHIFT)], SHIFT);
        assert!(matches!(a[..], [AppAction::IsolateSelectedObjects]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::H, ALT)], ALT);
        assert!(matches!(a[..], [AppAction::ShowAllObjects]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::F, SHIFT)], SHIFT);
        assert!(matches!(a[..], [AppAction::FrameSelection]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::F, NONE)], NONE);
        assert!(matches!(a[..], [AppAction::FrameModel]), "{a:?}");

        let mut text = String::new();
        let field_id = egui::Id::unique("object_field");
        let frame = |events: Vec<egui::Event>, state: &mut AppState, text: &mut String| {
            let events: Vec<egui::Event> = std::iter::once(egui::Event::ModifiersChanged(NONE)).chain(events).collect();
            let input = egui::RawInput { events, ..Default::default() };
            let mut actions = Vec::new();
            crate::theme::run_test_pass(&ctx, input, |ui| {
                actions = handle(ui.ctx(), state);
                ui.add(egui::TextEdit::singleline(text).id(field_id));
            });
            actions
        };
        let _ = frame(vec![], &mut state, &mut text);
        ctx.memory_mut(|m| m.request_focus(field_id));
        let _ = frame(vec![], &mut state, &mut text);
        let a = frame(vec![key_event(Key::H, NONE), egui::Event::Text("h".into())], &mut state, &mut text);
        assert!(a.is_empty(), "{a:?}");
        assert_eq!(text, "h");
    }

    fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// Runs one egui pass with `events` and returns the actions of `handle`.
    fn run(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>, mods: Modifiers) -> Vec<AppAction> {
        let events: Vec<egui::Event> = std::iter::once(egui::Event::ModifiersChanged(mods)).chain(events).collect();
        let input = egui::RawInput { events, ..Default::default() };
        let mut actions = Vec::new();
        crate::theme::run_test_pass(ctx, input, |ui| actions = handle(ui.ctx(), state));
        actions
    }

    #[test]
    fn handle_maps_keys_and_respects_order_and_dialogs() {
        let ctx = egui::Context::default();
        let mut state = AppState::new(Prefs::default(), true);
        // Ctrl+Shift+S is "Save As", not "Save" (and both need a document: none here).
        assert!(run(&ctx, &mut state, vec![key_event(Key::S, CMD_SHIFT)], CMD_SHIFT).is_empty());
        let a = run(&ctx, &mut state, vec![key_event(Key::X, SHIFT)], SHIFT);
        assert!(matches!(a[..], [AppAction::SwapColors]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::X, NONE)], NONE);
        assert!(matches!(a[..], [AppAction::SetTool(ToolKind::Text)]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::OpenCurlyBracket, SHIFT)], SHIFT);
        assert!(matches!(a[..], [AppAction::BrushHardnessStep(-1)]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::CloseBracket, NONE)], NONE);
        assert!(matches!(a[..], [AppAction::BrushSizeStep(1)]), "{a:?}");
        let a = run(&ctx, &mut state, vec![key_event(Key::N, CMD)], CMD);
        assert!(matches!(a[..], [AppAction::NewFromModel(None)]), "{a:?}");

        // A dialog lets only Esc through.
        state.dialog = Dialog::About;
        assert!(run(&ctx, &mut state, vec![key_event(Key::B, NONE)], NONE).is_empty());
        let a = run(&ctx, &mut state, vec![key_event(Key::Escape, NONE)], NONE);
        assert!(matches!(a[..], [AppAction::Cancel]), "{a:?}");
        state.dialog = Dialog::None;

        // A modal job blocks everything.
        state.busy = Some(crate::state::BusyInfo { title: "Saving".into(), stage: String::new(), fraction: 0.0, modal: true });
        assert!(run(&ctx, &mut state, vec![key_event(Key::Escape, NONE)], NONE).is_empty());
        state.busy = None;
    }

    #[test]
    fn typing_keeps_single_keys_in_the_text_field() {
        let ctx = egui::Context::default();
        let mut state = AppState::new(Prefs::default(), true);
        let mut text = String::new();
        let field_id = egui::Id::unique("field");
        let frame = |events: Vec<egui::Event>, mods: Modifiers, state: &mut AppState, text: &mut String| {
            let events: Vec<egui::Event> = std::iter::once(egui::Event::ModifiersChanged(mods)).chain(events).collect();
        let input = egui::RawInput { events, ..Default::default() };
            let mut actions = Vec::new();
            crate::theme::run_test_pass(&ctx, input, |ui| {
                actions = handle(ui.ctx(), state);
                ui.add(egui::TextEdit::singleline(text).id(field_id));
            });
            actions
        };
        // Give the field focus.
        let _ = frame(vec![], NONE, &mut state, &mut text);
        ctx.memory_mut(|m| m.request_focus(field_id));
        let _ = frame(vec![], NONE, &mut state, &mut text);
        assert!(ctx.text_edit_focused());
        // "b" goes to the field, not to the Brush tool.
        let a = frame(vec![key_event(Key::B, NONE), egui::Event::Text("b".into())], NONE, &mut state, &mut text);
        assert!(a.is_empty(), "{a:?}");
        assert!(state.typing_prev);
        // Ctrl+O still fires and releases the focus.
        let a = frame(vec![key_event(Key::O, CMD)], CMD, &mut state, &mut text);
        assert!(matches!(a[..], [AppAction::OpenProject(None)]), "{a:?}");
        assert!(!ctx.memory(|m| m.has_focus(field_id)));
    }

    #[test]
    fn shortcut_text_uses_display_overrides() {
        let ctx = egui::Context::default();
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |_| {});
        assert_eq!(shortcut_text(&ctx, "Softer Brush"), "Shift+[");
        assert_eq!(shortcut_text(&ctx, "Cancel"), "Esc");
        assert_eq!(shortcut_text(&ctx, "Save As..."), "Ctrl+Shift+S");
        assert_eq!(shortcut_text(&ctx, "Redo"), "Ctrl+Y");
        assert_eq!(shortcut_text(&ctx, "3D"), "1");
        assert_eq!(shortcut_text(&ctx, "no such label"), "");
    }
}
