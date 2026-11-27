// Owner: app-shell (A1). Keyboard shortcuts of ARCHITECTURE.md 8.10: the table, `handle` (the only code that consumes
// key presses) and `shortcut_text` for menus and tooltips.
//
// Action labels (the `label` argument of `shortcut_text`): "New from Model...", "Open Project...", "Save", "Save As...",
// "Export...", "Undo", "Redo", "Delete", "Cancel", "3D", "2D", "Split", "Preview", "Texture on Model", "Wireframe",
// "Frame Model", the tool names "Navigate", "Brush", "Eraser", "Eyedropper", "Fill", "Stamp", "Text", "Transform",
// "Smaller Brush", "Larger Brush", "Softer Brush", "Harder Brush", "New Paint Layer", "Duplicate Layer", "Merge Down",
// "Move Layer Up", "Move Layer Down", "Reset Colors", "Swap Colors"; objects-tabs: "Frame Selection", "Isolate Selected",
// "Show All", "Hide Selected"; stickers P6a: "Sticker", "Edit Sticker".
//
// Stickers P6a (STICKER_PLAN 3.3, reviews A12 / B6): the second table `STICKER_SHORTCUTS` and one routing rule. In the
// Sticker view a chord is looked up in `STICKER_SHORTCUTS` first; when it is bound there that binding runs and the
// global one does not; otherwise the global binding runs unless its label is in `NOT_IN_STICKER`. Outside the Sticker
// view only the global table is used, exactly as before. Labels of the sticker table (`sticker_shortcut_text`): the
// sticker tool labels ("Move", "Marquee", ... `StickerTool::label`), "Fit Sticker", "Sticker Delete", "Sticker Esc",
// "Sticker Enter", "Duplicate Sticker Layer", "Merge Sticker Layer Down", "Move Sticker Layer Up", "Move Sticker Layer
// Down", "Nudge Left" ... "Nudge Down 10".

use eframe::egui;
use egui::{Key, KeyboardShortcut, Modifiers};

use crate::action::AppAction;
use crate::state::{AppState, StickerTool, ToolKind, ViewMode};
use crate::sticker::StickerAction;

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

/// Stickers P6a: "Edit Sticker" (Enter) needs a selected stamp.
fn with_stamp(state: &AppState, action: AppAction) -> Option<AppAction> {
    state.session.as_ref().filter(|s| s.selected_stamp.is_some_and(|id| s.doc.stamp(id).is_some())).map(|_| action)
}

/// A command of the Sticker view.
fn sticker(action: StickerAction) -> Option<AppAction> {
    Some(AppAction::Sticker(action))
}

/// The shortcut table, ordered most-specific-first: of every pair that `Modifiers::matches_logically` could confuse
/// (Ctrl+Shift+S / Ctrl+S, Ctrl+Shift+N / Ctrl+N, Ctrl+Shift+Z / Ctrl+Z, Shift+X / X, Shift+F / F, Shift+H and Alt+H / H)
/// the more specific one comes first.
static TABLE: [ShortcutDef; 42] = [
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
    // Stickers P6a: the Sticker view (4) and "Edit Sticker" on the selected stamp (Enter).
    def("Sticker", NONE, Key::Num4, None, false, |s| with_stamp(s, AppAction::SetViewMode(ViewMode::Sticker))),
    def("Edit Sticker", NONE, Key::Enter, None, false, |s| with_stamp(s, AppAction::Sticker(StickerAction::Enter(None)))),
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

const fn tool(label: &'static str, modifiers: Modifiers, key: Key, make: fn(&AppState) -> Option<AppAction>) -> ShortcutDef {
    def(label, modifiers, key, None, false, make)
}

/// Stickers P6a (STICKER_PLAN 3.3): the keys of the Sticker view, ordered most-specific-first. They win over the global
/// table there (`sticker_order`). Stickers P7b: Ctrl+A / Ctrl+D / Ctrl+Shift+I (Select All, Select None, Invert
/// Selection).
static STICKER_SHORTCUTS: [ShortcutDef; 34] = [
    // selections (stickers P7b)
    def("Invert Selection", CMD_SHIFT, Key::I, None, false, |_| sticker(StickerAction::InvertSelection)),
    def("Select All", CMD, Key::A, None, false, |_| sticker(StickerAction::SelectAll)),
    def("Select None", CMD, Key::D, None, false, |_| sticker(StickerAction::SelectNone)),
    // tools (X is the Text key of both modes)
    tool("Move", NONE, Key::V, |_| Some(AppAction::SetStickerTool(StickerTool::Move))),
    tool("Marquee", NONE, Key::M, |_| Some(AppAction::SetStickerTool(StickerTool::Marquee))),
    tool("Lasso", NONE, Key::L, |_| Some(AppAction::SetStickerTool(StickerTool::Lasso))),
    tool("Magic Wand", NONE, Key::W, |_| Some(AppAction::SetStickerTool(StickerTool::Wand))),
    tool("Crop", NONE, Key::C, |_| Some(AppAction::SetStickerTool(StickerTool::Crop))),
    tool("Brush", NONE, Key::B, |_| Some(AppAction::SetStickerTool(StickerTool::Brush))),
    tool("Magic Eraser", SHIFT, Key::E, |_| Some(AppAction::SetStickerTool(StickerTool::MagicEraser))),
    tool("Eraser", NONE, Key::E, |_| Some(AppAction::SetStickerTool(StickerTool::Eraser))),
    tool("Gradient", SHIFT, Key::G, |_| Some(AppAction::SetStickerTool(StickerTool::Gradient))),
    tool("Fill", NONE, Key::G, |_| Some(AppAction::SetStickerTool(StickerTool::Fill))),
    tool("Eyedropper", NONE, Key::I, |_| Some(AppAction::SetStickerTool(StickerTool::Eyedropper))),
    tool("Text", NONE, Key::X, |_| Some(AppAction::SetStickerTool(StickerTool::Text))),
    tool("Shape", NONE, Key::U, |_| Some(AppAction::SetStickerTool(StickerTool::Shape))),
    tool("Hand", NONE, Key::H, |_| Some(AppAction::SetStickerTool(StickerTool::Hand))),
    // view and edit
    def("Fit Sticker", NONE, Key::F, None, false, |_| sticker(StickerAction::Fit)),
    def("Sticker Delete", NONE, Key::Delete, None, false, |_| sticker(StickerAction::DeleteKey)),
    def("Sticker Delete", NONE, Key::Backspace, None, false, |_| sticker(StickerAction::DeleteKey)),
    def("Sticker Esc", NONE, Key::Escape, Some("Esc"), false, |_| sticker(StickerAction::Escape)),
    def("Sticker Enter", NONE, Key::Enter, None, false, |_| sticker(StickerAction::EnterKey)),
    // the active sticker layer (never a texture layer)
    def("Duplicate Sticker Layer", CMD, Key::J, None, false, |_| sticker(StickerAction::DuplicateLayer(None))),
    def("Merge Sticker Layer Down", CMD, Key::M, None, false, |_| sticker(StickerAction::MergeDown(None))),
    def("Move Sticker Layer Up", CMD, Key::CloseBracket, Some("Ctrl+]"), false, |_| sticker(StickerAction::MoveLayer(None, 1))),
    def("Move Sticker Layer Down", CMD, Key::OpenBracket, Some("Ctrl+["), false, |_| sticker(StickerAction::MoveLayer(None, -1))),
    // the Move tool's nudge (Shift: 10 pixels)
    def("Nudge Left 10", SHIFT, Key::ArrowLeft, None, false, |_| sticker(StickerAction::Nudge(-10, 0))),
    def("Nudge Right 10", SHIFT, Key::ArrowRight, None, false, |_| sticker(StickerAction::Nudge(10, 0))),
    def("Nudge Up 10", SHIFT, Key::ArrowUp, None, false, |_| sticker(StickerAction::Nudge(0, -10))),
    def("Nudge Down 10", SHIFT, Key::ArrowDown, None, false, |_| sticker(StickerAction::Nudge(0, 10))),
    def("Nudge Left", NONE, Key::ArrowLeft, None, false, |_| sticker(StickerAction::Nudge(-1, 0))),
    def("Nudge Right", NONE, Key::ArrowRight, None, false, |_| sticker(StickerAction::Nudge(1, 0))),
    def("Nudge Up", NONE, Key::ArrowUp, None, false, |_| sticker(StickerAction::Nudge(0, -1))),
    def("Nudge Down", NONE, Key::ArrowDown, None, false, |_| sticker(StickerAction::Nudge(0, 1))),
];

/// Global entries that never run in the Sticker view (STICKER_PLAN 3.3): a paint tool cannot become active there.
const NOT_IN_STICKER: [&str; 2] = ["Stamp", "Transform"];

/// The sticker table (tests, the Shortcuts dialog).
pub fn sticker_table() -> &'static [ShortcutDef] {
    &STICKER_SHORTCUTS
}

/// Number of modifiers of a chord (for the most-specific-first order of `sticker_order`).
fn modifier_count(m: Modifiers) -> u8 {
    u8::from(m.alt) + u8::from(m.shift) + u8::from(m.ctrl || m.command || m.mac_cmd)
}

/// The routing rule of the Sticker view as one ordered list: every `STICKER_SHORTCUTS` entry, and every global entry
/// whose chord the sticker table does not bind (`true` = in `NOT_IN_STICKER`: its key is consumed and nothing runs),
/// sorted most-specific-first (stable), so Shift+F (Frame Selection) still comes before the sticker F (Fit) and
/// Shift+X (Swap Colors) before the sticker X (Text).
pub fn sticker_order() -> &'static [(&'static ShortcutDef, bool)] {
    static ORDER: std::sync::OnceLock<Vec<(&'static ShortcutDef, bool)>> = std::sync::OnceLock::new();
    ORDER.get_or_init(|| {
        let mut order: Vec<(&'static ShortcutDef, bool)> = STICKER_SHORTCUTS.iter().map(|d| (d, false)).collect();
        for d in table() {
            if STICKER_SHORTCUTS.iter().any(|s| s.shortcut == d.shortcut) {
                continue;
            }
            order.push((d, NOT_IN_STICKER.contains(&d.action_label)));
        }
        order.sort_by_key(|(d, _)| std::cmp::Reverse(modifier_count(d.shortcut.modifiers)));
        order
    })
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
        // Stickers P6a: the routing rule of the Sticker view (`sticker_order`).
        let in_sticker = state.view_mode == ViewMode::Sticker;
        let global: Vec<(&ShortcutDef, bool)>;
        let order: &[(&ShortcutDef, bool)] = if in_sticker {
            sticker_order()
        } else {
            global = table().iter().map(|d| (d, false)).collect();
            &global
        };
        for &(def, blocked) in order {
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
            // P6a check: Enter in a one-line field ends the typing and drops the focus in this very pass, so `typing`
            // is already false here; the key belongs to the field ("Edit Sticker" would open the Sticker view after a
            // stamp or layer was renamed with Enter).
            if def.shortcut.logical_key == Key::Enter && state.typing_prev {
                continue;
            }
            if popup_open && (is_esc || matches!(def.shortcut.logical_key, Key::Delete | Key::Backspace)) {
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
            if blocked {
                continue; // a paint tool cannot become active in the Sticker view
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
    text_in(table(), ctx, label)
}

/// Stickers P6a: the shortcut text of a label of `STICKER_SHORTCUTS` (the tool strip and the menus of the Sticker
/// view); "" if it has none.
pub fn sticker_shortcut_text(ctx: &egui::Context, label: &str) -> String {
    text_in(sticker_table(), ctx, label)
}

fn text_in(defs: &[ShortcutDef], ctx: &egui::Context, label: &str) -> String {
    defs.iter()
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

    /// Stickers P10b (STICKER_PLAN 6.3): egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V (and Ctrl+Insert, Shift+Insert,
    /// Shift+Delete) into `Event::Copy` / `Cut` / `Paste`, never into key events, so a binding of them would never fire:
    /// neither table binds one (the clipboard goes through `clip.rs`).
    #[test]
    fn neither_table_binds_the_clipboard_chords() {
        for d in table().iter().chain(sticker_table()) {
            let (m, k) = (d.shortcut.modifiers, d.shortcut.logical_key);
            let ctrl = m.command || m.ctrl;
            let only_ctrl = ctrl && !m.shift && !m.alt;
            assert!(!(only_ctrl && matches!(k, Key::C | Key::X | Key::V)), "{} binds a clipboard chord {:?}", d.action_label, d.shortcut);
            assert!(!(k == Key::Insert && (ctrl || m.shift)), "{} binds {:?}", d.action_label, d.shortcut);
            assert!(!(k == Key::Delete && m.shift), "{} binds {:?}", d.action_label, d.shortcut);
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
            (NONE, Key::Num4),
            (NONE, Key::Enter),
        ] {
            assert!(find(m, k).is_some(), "missing binding {m:?} {k:?}");
        }
        let text_ok: Vec<_> = table().iter().filter(|d| d.text_ok).map(|d| d.action_label).collect();
        assert_eq!(text_ok, ["Save As...", "Save", "New from Model...", "Open Project...", "Export...", "Preview"]);
        assert_eq!(table().len(), 42);
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

    /// Stickers P6a: the sticker table binds no chord twice, and in the routed order of the Sticker view no entry is
    /// shadowed by an earlier one (Shift+F before F, Shift+X before X, Shift+E / Shift+G before E / G).
    #[test]
    fn the_sticker_table_has_no_chord_twice_and_its_order_shadows_nothing() {
        for (i, a) in sticker_table().iter().enumerate() {
            for b in &sticker_table()[i + 1..] {
                assert!(a.shortcut != b.shortcut, "{} and {} share {:?}", a.action_label, b.action_label, a.shortcut);
            }
        }
        let order = sticker_order();
        for (j, (b, _)) in order.iter().enumerate() {
            for (a, _) in &order[..j] {
                let shadowed = a.shortcut.logical_key == b.shortcut.logical_key
                    && b.shortcut.modifiers.matches_logically(a.shortcut.modifiers);
                assert!(!shadowed, "{} ({:?}) is shadowed by the earlier {}", b.action_label, b.shortcut, a.action_label);
            }
        }
        // Every chord of either table is in the order once.
        assert_eq!(order.len(), sticker_table().len() + table().iter().filter(|d| !sticker_table().iter().any(|s| s.shortcut == d.shortcut)).count());
        let blocked: Vec<&str> = order.iter().filter(|(_, b)| *b).map(|(d, _)| d.action_label).collect();
        assert_eq!(blocked, ["Stamp", "Transform"]);
    }

    /// Stickers P6a (STICKER_PLAN 3.3): every colliding chord, in both modes: the action that runs.
    #[test]
    fn colliding_chords_run_the_action_of_their_mode() {
        use crate::sticker::StickerAction as S;
        let ctx = egui::Context::default();
        let (mut state, _, _) = crate::sticker::tests::image_stamp_state(ViewMode::View3D);
        let none: Vec<AppAction> = Vec::new();
        let one = |a: AppAction| vec![a];
        let cases: Vec<(Modifiers, Key, Vec<AppAction>, Vec<AppAction>)> = vec![
            (NONE, Key::V, one(AppAction::SetTool(ToolKind::Navigate)), one(AppAction::SetStickerTool(StickerTool::Move))),
            (NONE, Key::W, one(AppAction::ToggleWireframe), one(AppAction::SetStickerTool(StickerTool::Wand))),
            (NONE, Key::H, one(AppAction::HideSelectedObjects), one(AppAction::SetStickerTool(StickerTool::Hand))),
            (NONE, Key::F, one(AppAction::FrameModel), one(AppAction::Sticker(S::Fit))),
            (NONE, Key::X, one(AppAction::SetTool(ToolKind::Text)), one(AppAction::SetStickerTool(StickerTool::Text))),
            (NONE, Key::B, one(AppAction::SetTool(ToolKind::Brush)), one(AppAction::SetStickerTool(StickerTool::Brush))),
            (NONE, Key::E, one(AppAction::SetTool(ToolKind::Eraser)), one(AppAction::SetStickerTool(StickerTool::Eraser))),
            (NONE, Key::G, one(AppAction::SetTool(ToolKind::Fill)), one(AppAction::SetStickerTool(StickerTool::Fill))),
            (NONE, Key::I, one(AppAction::SetTool(ToolKind::Eyedropper)), one(AppAction::SetStickerTool(StickerTool::Eyedropper))),
            (NONE, Key::S, one(AppAction::SetTool(ToolKind::Stamp)), none.clone()),
            (NONE, Key::R, one(AppAction::SetTool(ToolKind::Transform)), none.clone()),
            (NONE, Key::Delete, one(AppAction::DeleteSelection), one(AppAction::Sticker(S::DeleteKey))),
            (NONE, Key::Backspace, none.clone(), one(AppAction::Sticker(S::DeleteKey))),
            (NONE, Key::Escape, one(AppAction::Cancel), one(AppAction::Sticker(S::Escape))),
            (NONE, Key::Enter, one(AppAction::Sticker(S::Enter(None))), one(AppAction::Sticker(S::EnterKey))),
            (CMD, Key::J, one(AppAction::DuplicateActiveLayer), one(AppAction::Sticker(S::DuplicateLayer(None)))),
            (CMD, Key::M, one(AppAction::MergeActiveDown), one(AppAction::Sticker(S::MergeDown(None)))),
            (CMD, Key::CloseBracket, one(AppAction::MoveActiveLayer(1)), one(AppAction::Sticker(S::MoveLayer(None, 1)))),
            (CMD, Key::OpenBracket, one(AppAction::MoveActiveLayer(-1)), one(AppAction::Sticker(S::MoveLayer(None, -1)))),
            (NONE, Key::M, none.clone(), one(AppAction::SetStickerTool(StickerTool::Marquee))),
            // Stickers P7b: the selection chords (unbound outside the Sticker view; Ctrl+D is not D's Reset Colors).
            (CMD, Key::A, none.clone(), one(AppAction::Sticker(S::SelectAll))),
            (CMD, Key::D, none.clone(), one(AppAction::Sticker(S::SelectNone))),
            (CMD_SHIFT, Key::I, none.clone(), one(AppAction::Sticker(S::InvertSelection))),
            // Outside the Sticker view Shift+E / Shift+G have no binding of their own; E / G match them logically (Shift
            // is ignored by a pattern without it), as before.
            (SHIFT, Key::E, one(AppAction::SetTool(ToolKind::Eraser)), one(AppAction::SetStickerTool(StickerTool::MagicEraser))),
            (SHIFT, Key::G, one(AppAction::SetTool(ToolKind::Fill)), one(AppAction::SetStickerTool(StickerTool::Gradient))),
            (NONE, Key::ArrowLeft, none.clone(), one(AppAction::Sticker(S::Nudge(-1, 0)))),
            (SHIFT, Key::ArrowDown, none.clone(), one(AppAction::Sticker(S::Nudge(0, 10)))),
            // Not bound in the sticker table: the global binding runs in both modes.
            (SHIFT, Key::F, one(AppAction::FrameSelection), one(AppAction::FrameSelection)),
            (SHIFT, Key::X, one(AppAction::SwapColors), one(AppAction::SwapColors)),
            (NONE, Key::T, one(AppAction::ToggleTexture), one(AppAction::ToggleTexture)),
            (NONE, Key::Num1, one(AppAction::SetViewMode(ViewMode::View3D)), one(AppAction::SetViewMode(ViewMode::View3D))),
            (NONE, Key::Num4, one(AppAction::SetViewMode(ViewMode::Sticker)), one(AppAction::SetViewMode(ViewMode::Sticker))),
            (CMD, Key::Z, one(AppAction::Undo), one(AppAction::Undo)),
            (NONE, Key::CloseBracket, one(AppAction::BrushSizeStep(1)), one(AppAction::BrushSizeStep(1))),
        ];
        for (mode, sticker) in [(ViewMode::View3D, false), (ViewMode::Sticker, true)] {
            state.view_mode = mode;
            for (mods, key, outside, inside) in &cases {
                let want = if sticker { inside } else { outside };
                let got = run(&ctx, &mut state, vec![key_event(*key, *mods)], *mods);
                assert_eq!(format!("{got:?}"), format!("{want:?}"), "{mods:?} {key:?} in {mode:?}");
            }
        }
        assert_eq!(sticker_shortcut_text(&ctx, "Move"), "V");
        assert_eq!(sticker_shortcut_text(&ctx, "Sticker Esc"), "Esc");
        assert_eq!(shortcut_text(&ctx, "Sticker"), "4");
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

    /// P6a check: Enter that ends the typing in a one-line field (the stamp's Name, a layer rename) stays the field's:
    /// with a stamp selected it does not open the Sticker view ("Edit Sticker"), in the order of the real frame (the
    /// panels with their fields first, `handle` after them); the next Enter, with nothing focused, does.
    #[test]
    fn enter_that_ends_typing_in_a_field_does_not_open_the_sticker_view() {
        let ctx = egui::Context::default();
        let (mut state, _, _) = crate::sticker::tests::image_stamp_state(ViewMode::View3D);
        let mut text = String::from("Logo");
        let field_id = egui::Id::unique("stamp_name_field");
        let frame = |events: Vec<egui::Event>, state: &mut AppState, text: &mut String| {
            let events: Vec<egui::Event> = std::iter::once(egui::Event::ModifiersChanged(NONE)).chain(events).collect();
            let input = egui::RawInput { events, ..Default::default() };
            let mut actions = Vec::new();
            crate::theme::run_test_pass(&ctx, input, |ui| {
                ui.add(egui::TextEdit::singleline(text).id(field_id));
                actions = handle(ui.ctx(), state);
            });
            actions
        };
        let _ = frame(vec![], &mut state, &mut text);
        ctx.memory_mut(|m| m.request_focus(field_id));
        let _ = frame(vec![], &mut state, &mut text);
        let a = frame(vec![key_event(Key::Num2, NONE), egui::Event::Text("2".into())], &mut state, &mut text);
        assert!(a.is_empty(), "{a:?}");
        assert_eq!(text, "Logo2");
        // Enter ends the typing: the field loses the focus in this pass, and no shortcut runs.
        let a = frame(vec![key_event(Key::Enter, NONE)], &mut state, &mut text);
        assert!(!ctx.memory(|m| m.has_focus(field_id)), "Enter ended the typing");
        assert!(a.is_empty(), "{a:?}");
        // The next Enter (nothing focused) is "Edit Sticker".
        let a = frame(vec![key_event(Key::Enter, NONE)], &mut state, &mut text);
        assert!(matches!(a[..], [AppAction::Sticker(crate::sticker::StickerAction::Enter(None))]), "{a:?}");
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
