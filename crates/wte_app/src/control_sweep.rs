//! Control sweep (review round 2): every interactive control of the menu bar, toolbar, side panel, view-mode strip,
//! Preview option strip and dialogs is driven with real pointer / keyboard events through the real panel and dialog
//! functions, and the queued `AppAction`s or the changed state are checked against the control's label.
//!
//! Widgets are located through egui's AccessKit tree (label / value and bounds of every widget of the last pass),
//! so the tests click where a user would click: an overlay that swallows clicks, a popup that closes before its
//! item registers or a control that never receives input inside a scroll area or modal makes them fail.
//!
//! The panel tests run headless without a GPU. The dialog / Preview / end-to-end tests build a real `WteApp` on a
//! headless wgpu device and are `#[ignore = "needs a GPU"]` like the other GPU tests; run them with
//! `cargo test -p wte_app --offline -- --ignored control_sweep`.

#![allow(dead_code, reason = "helpers shared by the GPU and the non-GPU tests")]

use eframe::egui;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Pos2, Rect, Vec2, pos2, vec2};
use wte_core::{Document, SampleKind, TextureSetId};

use crate::action::AppAction;
use crate::panels::{self, PanelState};
use crate::state::{AppState, OrbitCamera, Prefs, Session, UvViewState};

/// One widget of the last pass, from the AccessKit tree.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub label: String,
    pub value: String,
    pub role: Role,
    pub rect: Rect,
    pub disabled: bool,
    pub toggled: Option<bool>,
    pub number: Option<f64>,
}

impl Node {
    pub fn text(&self) -> &str {
        if self.label.is_empty() { &self.value } else { &self.label }
    }
}

/// A headless egui context with a running clock and the widget list of the last pass.
pub(crate) struct Harness {
    pub ctx: egui::Context,
    pub time: f64,
    pub size: Vec2,
    pub mods: Modifiers,
    pub nodes: Vec<Node>,
    pub pointer: Pos2,
}

impl Harness {
    pub fn new(w: f32, h: f32) -> Harness {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Harness { ctx, time: 10.0, size: vec2(w, h), mods: Modifiers::NONE, nodes: Vec::new(), pointer: pos2(-10.0, -10.0) }
    }

    fn input(&mut self, events: Vec<Event>) -> egui::RawInput {
        self.time += 1.0 / 60.0;
        let events = std::iter::once(Event::ModifiersChanged(self.mods)).chain(events).collect();
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            events,
            ..Default::default()
        }
    }

    /// One frame of `f`; collects the widget list.
    pub fn pass(&mut self, events: Vec<Event>, mut f: impl FnMut(&mut egui::Ui)) {
        let input = self.input(events);
        let mut out = self.ctx.run_ui(input, |ui| f(ui));
        out.textures_delta.clear();
        self.collect(out.platform_output.accesskit_update);
    }

    pub fn collect(&mut self, update: Option<egui::accesskit::TreeUpdate>) {
        let Some(update) = update else { return };
        self.nodes = update
            .nodes
            .iter()
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                let rect = Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32));
                Some(Node {
                    label: n.label().unwrap_or_default().to_owned(),
                    value: n.value().unwrap_or_default().to_owned(),
                    role: n.role(),
                    rect,
                    disabled: n.is_disabled(),
                    toggled: n.toggled().map(|t| t == egui::accesskit::Toggled::True),
                    number: n.numeric_value(),
                })
            })
            .collect();
        self.nodes.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y).then(a.rect.min.x.total_cmp(&b.rect.min.x)));
    }

    /// All widgets whose label or value is exactly `text`, top to bottom.
    pub fn all(&self, text: &str) -> Vec<Node> {
        self.nodes.iter().filter(|n| n.label == text || n.value == text).cloned().collect()
    }

    /// The widget labelled `text` (the first one, top to bottom); panics with the list of labels when missing.
    pub fn get(&self, text: &str) -> Node {
        self.all(text).into_iter().find(|n| n.rect.is_positive()).unwrap_or_else(|| {
            let labels: Vec<String> = self.nodes.iter().map(|n| format!("{:?}:{:?}", n.role, n.text())).collect();
            panic!("no widget {text:?} in {labels:?}")
        })
    }

    pub fn has(&self, text: &str) -> bool {
        self.all(text).iter().any(|n| n.rect.is_positive())
    }

    fn labels(&self) -> Vec<String> {
        self.nodes.iter().map(|n| format!("{:?}:{:?}", n.role, n.text())).collect()
    }

    /// The first widget whose label starts with `prefix`.
    pub fn prefix(&self, prefix: &str) -> Node {
        self.nodes
            .iter()
            .find(|n| n.label.starts_with(prefix) && n.rect.is_positive())
            .cloned()
            .unwrap_or_else(|| panic!("no widget starting with {prefix:?} in {:?}", self.labels()))
    }

    /// A menu entry: its label is the text plus the shortcut text (menu entries are selectable buttons).
    pub fn menu_item(&self, text: &str) -> Node {
        self.find_menu_item(text).unwrap_or_else(|| panic!("no menu entry {text:?} in {:?}", self.labels()))
    }

    pub fn find_menu_item(&self, text: &str) -> Option<Node> {
        self.nodes
            .iter()
            .filter(|n| n.role == Role::Button && n.rect.is_positive())
            .find(|n| n.label.trim_end() == text || (n.toggled.is_some() && n.label.starts_with(&format!("{text} "))))
            .cloned()
    }

    /// The widget of `role` in the row of the label `row_label` (same line, right of the label); `nth` counts from 0.
    /// objects-tabs (WP4): of the widgets that overlap the label's centre line only those whose centres lie nearest to
    /// it (within 2 pt) count: AccessKit also lists the clipped widgets of a scrolled page, and those of the upper page
    /// can lie behind the rows of the lower group, a few points off their centre line.
    pub fn in_row(&self, row_label: &str, role: Role, nth: usize) -> Node {
        let label = self
            .nodes
            .iter()
            .find(|n| n.role == Role::Label && n.value == row_label)
            .unwrap_or_else(|| panic!("no row label {row_label:?} in {:?}", self.labels()))
            .clone();
        let y = label.rect.center().y;
        let mut found: Vec<Node> = self
            .nodes
            .iter()
            .filter(|n| n.role == role && n.rect.min.y <= y && n.rect.max.y >= y && n.rect.min.x > label.rect.min.x)
            .cloned()
            .collect();
        let off = |n: &Node| (n.rect.center().y - y).abs();
        let nearest = found.iter().map(off).fold(f32::INFINITY, f32::min);
        found.retain(|n| off(n) <= nearest + 2.0);
        found.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
        found.get(nth).cloned().unwrap_or_else(|| panic!("no {role:?} #{nth} in row {row_label:?}: {:?}", self.labels()))
    }

    /// Nodes that are not in `before` (same role, label and rect): what a popup added.
    pub fn new_since(&self, before: &[Node]) -> Vec<Node> {
        self.nodes
            .iter()
            .filter(|n| !before.iter().any(|b| b.role == n.role && b.label == n.label && b.rect == n.rect))
            .cloned()
            .collect()
    }

    /// The saturation/value square of a colour picker among `nodes` (the largest square unlabelled widget).
    pub fn picker_square(nodes: &[Node]) -> Node {
        nodes
            .iter()
            .filter(|n| n.role == Role::Unknown && n.label.is_empty() && n.rect.width() > 60.0)
            .filter(|n| (n.rect.width() - n.rect.height()).abs() < 2.0)
            .max_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
            .cloned()
            .unwrap_or_else(|| panic!("no colour square in {:?}", nodes.iter().map(|n| (n.role, n.text().to_owned(), n.rect)).collect::<Vec<_>>()))
    }

    pub fn button(pos: Pos2, button: egui::PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos, button, pressed, modifiers }
    }

    pub fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    pub fn key_up(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }
    }

    pub fn wheel(delta: Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers,
        }
    }
}

/// A scripted user: hover, click, drag, type, keys and wheel, one input step per frame (like the self-test runner).
pub(crate) trait Drive {
    fn h(&mut self) -> &mut Harness;
    /// One frame of the UI under test with these events.
    fn frame(&mut self, events: Vec<Event>);

    fn idle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame(Vec::new());
        }
    }

    fn hover(&mut self, pos: Pos2) {
        self.h().pointer = pos;
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn click_at_with(&mut self, pos: Pos2, button: egui::PointerButton) {
        let mods = self.h().mods;
        self.hover(pos);
        self.frame(vec![Harness::button(pos, button, true, mods)]);
        self.frame(vec![Harness::button(pos, button, false, mods)]);
    }

    fn click_at(&mut self, pos: Pos2) {
        self.click_at_with(pos, egui::PointerButton::Primary);
    }

    fn double_click_at(&mut self, pos: Pos2) {
        let mods = self.h().mods;
        self.hover(pos);
        for _ in 0..2 {
            self.frame(vec![Harness::button(pos, egui::PointerButton::Primary, true, mods)]);
            self.frame(vec![Harness::button(pos, egui::PointerButton::Primary, false, mods)]);
        }
    }

    /// Clicks the widget labelled `text`.
    fn click(&mut self, text: &str) {
        let pos = self.h().get(text).rect.center();
        self.click_at(pos);
    }

    fn right_click(&mut self, text: &str) {
        let pos = self.h().get(text).rect.center();
        self.click_at_with(pos, egui::PointerButton::Secondary);
    }

    fn right_click_at(&mut self, pos: Pos2) {
        self.click_at_with(pos, egui::PointerButton::Secondary);
    }

    /// Press at the first point, move through the others (one per frame), release at the last one.
    fn drag(&mut self, points: &[Pos2]) {
        let mods = self.h().mods;
        self.hover(points[0]);
        self.frame(vec![Harness::button(points[0], egui::PointerButton::Primary, true, mods)]);
        for p in &points[1..] {
            self.h().pointer = *p;
            self.frame(vec![Event::PointerMoved(*p)]);
        }
        let last = *points.last().unwrap();
        self.frame(vec![Harness::button(last, egui::PointerButton::Primary, false, mods)]);
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    fn press(&mut self, key: Key, mods: Modifiers) {
        let old = self.h().mods;
        self.h().mods = mods;
        self.frame(vec![Event::ModifiersChanged(mods), Harness::key(key, mods)]);
        self.frame(vec![Harness::key_up(key, mods)]);
        self.h().mods = old;
        self.frame(vec![Event::ModifiersChanged(old)]);
    }

    fn scroll_at(&mut self, pos: Pos2, delta: Vec2) {
        self.hover(pos);
        let mods = self.h().mods;
        self.frame(vec![Harness::wheel(delta, mods)]);
    }
}

/// The sample document as `install_document` sets it up.
pub(crate) fn sample_state(kind: SampleKind) -> AppState {
    let mut state = AppState::new(Prefs::default(), true);
    let doc = Document::sample(kind);
    let mut camera = OrbitCamera::default();
    camera.frame(doc.mesh.bounds);
    state.session = Some(Session {
        doc,
        path: None,
        force_save_as: false,
        active_set: TextureSetId(0),
        selected_stamp: None,
        camera,
        preview_camera: camera,
        uv_view: UvViewState::default(),
        scene_generation: 7,
        text_gap: None,
        selected_layers: Vec::new(),
        selected_objects: Default::default(),
        object_anchor: None,
        object_selection_rev: 0,
        object_reveal: None,
        sticker_edit: None,
    });
    state
}

/// The menu bar, status bar, toolbar, side panel and the view-mode strip in the layout and order of `App::ui`
/// (shortcuts first, their actions last), without the GPU views and the dialogs.
pub(crate) struct PanelApp {
    pub h: Harness,
    pub state: AppState,
    pub ps: PanelState,
    /// Every action queued so far (moved out of `state.actions` after each frame).
    pub log: Vec<AppAction>,
    /// Apply the document-level actions after each frame (a minimal dispatch, so previews show up in the panels).
    pub apply: bool,
    /// Stickers P6b: the side panel's first width (300 pt as in `App::ui`; wider for the one-row upper strip).
    pub side_width: f32,
}

impl PanelApp {
    pub fn new(state: AppState) -> PanelApp {
        PanelApp::with_side_width(state, 300.0)
    }

    /// A panel app whose side panel starts `side_width` points wide (the user can widen the panel).
    pub fn with_side_width(state: AppState, side_width: f32) -> PanelApp {
        let mut app = PanelApp { h: Harness::new(1280.0, 800.0), state, ps: PanelState::new(), log: Vec::new(), apply: false, side_width };
        app.idle(3);
        app.log.clear();
        app
    }

    pub fn take(&mut self) -> Vec<AppAction> {
        std::mem::take(&mut self.log)
    }
}

/// The part of `dispatch` the panel tests need: document edits, selection, tool and colour changes.
pub(crate) fn apply_actions(state: &mut AppState, actions: &[AppAction]) {
    for a in actions {
        match a {
            AppAction::SetTool(t) => state.tool = *t,
            AppAction::SwapColors => std::mem::swap(&mut state.colors.primary, &mut state.colors.secondary),
            AppAction::SetActiveSet(s) => state.session.as_mut().unwrap().active_set = *s,
            AppAction::SelectStamp(id) => state.session.as_mut().unwrap().selected_stamp = *id,
            // objects-tabs (WP5): the object selection and visibility (view state, `object_ops`).
            AppAction::SelectObject(o) => crate::object_ops::select(state.session.as_mut().unwrap(), *o),
            AppAction::ToggleObjectSelected(o) => crate::object_ops::toggle(state.session.as_mut().unwrap(), *o),
            AppAction::SelectObjectRange(o) => crate::object_ops::select_range(state.session.as_mut().unwrap(), *o),
            AppAction::SelectAllObjects => crate::object_ops::select_all(state.session.as_mut().unwrap()),
            AppAction::DeselectObjects => crate::object_ops::deselect(state.session.as_mut().unwrap()),
            AppAction::SetObjectVisible(o, visible) => crate::object_ops::set_visible(state, *o, *visible),
            AppAction::HideSelectedObjects => crate::object_ops::hide_selected(state),
            AppAction::IsolateSelectedObjects => crate::object_ops::isolate_selected(state),
            AppAction::ShowAllObjects => crate::object_ops::show_all(state),
            _ => {}
        }
        let Some(doc) = state.doc_mut() else { continue };
        let r = match a {
            AppAction::SetLayerOpacity(l, v, c) => doc.set_layer_opacity(*l, *v, *c),
            AppAction::SetLayerBlend(l, b) => doc.set_layer_blend(*l, *b),
            AppAction::RenameLayer(l, n) => doc.rename_layer(*l, n),
            AppAction::UpdateStampPlacement(id, p, c) => doc.set_stamp_placement(*id, *p, *c),
            AppAction::UpdateStampTint(id, t, c) => doc.set_stamp_tint(*id, *t, *c),
            AppAction::UpdateStampSource(id, s, c) => doc.set_stamp_source(*id, s.clone(), *c),
            AppAction::SetLayerVisible(l, on) => doc.set_layer_visible(*l, *on),
            AppAction::SetLayerLocked(l, on) => doc.set_layer_locked(*l, *on),
            _ => Ok(()),
        };
        r.unwrap_or_else(|e| panic!("{a:?}: {e}"));
    }
}

impl Drive for PanelApp {
    fn h(&mut self) -> &mut Harness {
        &mut self.h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.frame_inner(events);
        let actions = std::mem::take(&mut self.state.actions);
        if self.apply {
            apply_actions(&mut self.state, &actions);
        }
        self.log.extend(actions);
    }
}

impl PanelApp {
    fn frame_inner(&mut self, events: Vec<Event>) {
        let PanelApp { h, state, ps, side_width, .. } = self;
        let side_width = *side_width;
        h.pass(events, |ui| {
            let late = crate::shortcuts::handle(ui.ctx(), state);
            egui::Panel::top("menu").show(ui, |ui| panels::show_menu_bar(ui, state));
            egui::Panel::bottom("status").show(ui, |ui| panels::show_status_bar(ui, state));
            egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| panels::show_toolbar(ui, state));
            let side = egui::Panel::right("side").default_size(side_width).show(ui, |ui| panels::show_side_panel(ui, state, ps));
            // As `App::ui` records it (round 4: the Brush tab tests check the side panel's width).
            state.side_panel_rect = Some(side.response.rect);
            egui::CentralPanel::no_frame().show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let strip = Rect::from_min_size(full.min, vec2(full.width(), 26.0));
                crate::view::mode_strip_for_tests(ui, state, strip);
            });
            state.actions.extend(late);
        });
    }
}

mod panel_tests;
mod sticker_panel_tests;
mod gpu_tests;
mod text_panel_tests;
