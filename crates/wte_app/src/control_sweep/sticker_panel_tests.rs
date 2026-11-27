//! Stickers P6b (STICKER_PLAN 7 "P6b"): every new control of the Sticker tab, the Stamp tool options, the Tool tab's
//! Layer styles in the Sticker view and the Edit menu's style items, clicked through egui with real pointer events: no
//! panic, a rect in `state.panel_rects` for every control, and the action each one queues.

use super::*;
use crate::state::{PanelHit, StampToolSource, ToolKind, UpperTab, ViewMode};
use crate::sticker::{PictureOf, StickerAction, StyleTarget};
use wte_core::{Commit, StampSource, StylePreset};

const PRIMARY: egui::PointerButton = egui::PointerButton::Primary;

/// The panel app on the sphere with the sample logo as a selected image stamp, the Sticker tab shown.
fn sticker_tab_app() -> (PanelApp, wte_core::StampId, wte_core::BlobId) {
    let (mut state, stamp, blob) = crate::sticker::tests::image_stamp_state(ViewMode::View3D);
    state.tool = ToolKind::Transform;
    state.upper_tab = UpperTab::Sticker;
    let app = PanelApp::new(state);
    (app, stamp, blob)
}

impl PanelApp {
    /// The recorded rect of `hit`; panics with the recorded hits when it is missing.
    fn hit(&self, hit: PanelHit) -> Rect {
        self.state.panel_rects.get(&hit).copied().filter(|r| r.is_positive()).unwrap_or_else(|| {
            let mut keys: Vec<String> = self.state.panel_rects.keys().map(|k| format!("{k:?}")).collect();
            keys.sort();
            panic!("no rect for {hit:?}: {keys:?}")
        })
    }

    /// Scrolls the upper page until the control `hit` is inside it, then clicks it; returns the queued actions.
    fn click_hit(&mut self, hit: PanelHit) -> Vec<AppAction> {
        for _ in 0..60 {
            let rect = self.hit(hit);
            let page = self.state.side_rects.upper_page;
            if page.expand(1.0).contains_rect(rect) {
                break;
            }
            let delta = if rect.max.y > page.max.y { vec2(0.0, -1.0) } else { vec2(0.0, 1.0) };
            self.scroll_at(page.center(), delta);
            self.idle(2);
        }
        let rect = self.hit(hit);
        self.take();
        self.click_at(rect.center());
        self.idle(1);
        self.take()
    }

    /// Runs the sticker actions of `actions` on the state (the dispatch of the state-only commands).
    fn run_sticker(&mut self, actions: Vec<AppAction>) {
        for a in actions {
            if let AppAction::Sticker(s) = a {
                crate::sticker::run_in(&mut self.state, s);
            }
        }
        self.idle(3);
        self.take();
    }
}

#[test]
fn every_control_of_the_sticker_tab_is_laid_out_and_queues_its_action() {
    let (mut app, stamp, _) = sticker_tab_app();
    app.idle(4);
    let r = app.state.side_rects.clone();
    assert!(r.sticker_tab.is_some() && r.upper_page == r.sticker_tab.unwrap(), "the Sticker page is drawn: {r:?}");
    for hit in [PanelHit::StickerTabEdit, PanelHit::StickerTabExport, PanelHit::StyleCopy, PanelHit::StylePaste, PanelHit::StyleClear, PanelHit::StyleEditor, PanelHit::TrayNew, PanelHit::TrayNewFromPicture, PanelHit::TrayEdit, PanelHit::TrayDuplicate, PanelHit::TrayDelete, PanelHit::TrayExport] {
        app.hit(hit);
    }
    for i in 0..StylePreset::ALL.len() {
        app.hit(PanelHit::StylePresetButton(i as u8));
    }
    for i in 0..crate::sticker::styles_ui::BLOCKS.len() {
        app.hit(PanelHit::StyleBlockToggle(i as u8));
    }
    // The selected stamp's buttons.
    let a = app.click_hit(PanelHit::StickerTabEdit);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::Enter(Some(s)))] if *s == stamp), "{a:?}");
    let a = app.click_hit(PanelHit::StickerTabExport);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::ExportPicture(PictureOf::Stamp(s), None))] if *s == stamp), "{a:?}");
    // The presets.
    for (i, preset) in StylePreset::ALL.into_iter().enumerate() {
        let a = app.click_hit(PanelHit::StylePresetButton(i as u8));
        assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::ApplyPreset(p))] if *p == preset), "{preset:?}: {a:?}");
    }
    // Copy, Clear; Paste is disabled without a copy, enabled with one.
    let a = app.click_hit(PanelHit::StyleCopy);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::CopyStyle(StyleTarget::Stamps))]), "{a:?}");
    let a = app.click_hit(PanelHit::StylePaste);
    assert!(a.is_empty(), "Paste without a copy: {a:?}");
    app.state.style_clipboard = Some(StylePreset::Outline.styles([64, 64]));
    app.idle(2);
    let a = app.click_hit(PanelHit::StylePaste);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::PasteStyle(StyleTarget::Stamps))]), "{a:?}");
    let a = app.click_hit(PanelHit::StyleClear);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::ClearStyle(StyleTarget::Stamps))]), "{a:?}");
    // Each block's box: the stack with that block on, for the selected stamp, one Final.
    for i in 0..crate::sticker::styles_ui::BLOCKS.len() {
        let a = app.click_hit(PanelHit::StyleBlockToggle(i as u8));
        match a.as_slice() {
            [AppAction::Sticker(StickerAction::SetStampStyles(stamps, styles, Commit::Final))] => {
                assert_eq!(stamps, &vec![stamp]);
                assert_eq!(crate::sticker::styles_ui::block_on(styles, i), Some(true), "block {i}");
            }
            other => panic!("block {i}: {other:?}"),
        }
    }
    // The tray: New Sticker and From picture...; then a tray item, and its buttons.
    let a = app.click_hit(PanelHit::TrayNewFromPicture);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::NewStickerFromPicture(None))]), "{a:?}");
    let a = app.click_hit(PanelHit::TrayNew);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::NewSticker)]), "{a:?}");
    app.run_sticker(a);
    app.idle(4);
    let id = crate::sticker::tray::tray_stickers(&app.state)[0];
    let a = app.click_hit(PanelHit::TrayItem(id.0));
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::TrayUse(k))] if *k == id), "{a:?}");
    for (hit, want) in [
        (PanelHit::TrayEdit, StickerAction::EnterTray(id)),
        (PanelHit::TrayDuplicate, StickerAction::TrayDuplicate(id)),
        (PanelHit::TrayDelete, StickerAction::TrayDelete(id)),
        (PanelHit::TrayExport, StickerAction::ExportPicture(PictureOf::Sticker(id), None)),
    ] {
        let a = app.click_hit(hit);
        assert_eq!(format!("{a:?}"), format!("{:?}", [AppAction::Sticker(want)]), "{hit:?}");
    }
    // Idle: nothing is queued by itself.
    app.idle(5);
    assert!(app.take().is_empty());
}

#[test]
fn an_open_style_block_sends_previews_and_one_final() {
    let (mut app, stamp, _) = sticker_tab_app();
    app.state.style_clipboard = None;
    // The stamp with a drop shadow: its block exists.
    let styles = crate::sticker::styles_ui::toggled(&wte_core::StyleStack::default(), 2, true, 0.125);
    app.state.doc_mut().unwrap().set_stamp_styles(stamp, styles, Commit::Final).unwrap();
    app.idle(3);
    // Open the block with its arrow (left of the box).
    let toggle = app.hit(PanelHit::StyleBlockToggle(2));
    app.click_at(egui::pos2(toggle.left() - 8.0, toggle.center().y));
    app.idle(3);
    app.take();
    let slider = app
        .h
        .nodes
        .iter()
        .filter(|n| n.role == Role::Slider && app.state.side_rects.upper_page.contains_rect(n.rect))
        .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
        .cloned()
        .unwrap_or_else(|| panic!("no slider in the open block: {:?}", app.h.nodes.iter().map(|n| (n.role, n.text().to_owned())).collect::<Vec<_>>()));
    let from = slider.rect.left_center() + vec2(slider.rect.width() * 0.3, 0.0);
    let path: Vec<Pos2> = (0..6).map(|k| from + vec2(k as f32 * 8.0, 0.0)).collect();
    app.drag(&path);
    app.idle(2);
    let a = app.take();
    let previews = a.iter().filter(|x| matches!(x, AppAction::Sticker(StickerAction::SetStampStyles(_, _, Commit::Preview)))).count();
    let finals = a.iter().filter(|x| matches!(x, AppAction::Sticker(StickerAction::SetStampStyles(_, _, Commit::Final)))).count();
    assert!(previews >= 1 && finals == 1, "a drag is previews and one final: {a:?}");
}

#[test]
fn the_stamp_tool_options_and_the_edit_menu_style_items() {
    let (mut app, _, blob) = sticker_tab_app();
    app.state.upper_tab = UpperTab::Tool;
    app.state.tool = ToolKind::Stamp;
    app.state.tools.stamp.source = StampToolSource::Blob(blob);
    app.state.tools.stamp.source_name = "logo.png".to_owned();
    app.idle(4);
    let a = app.click_hit(PanelHit::StampEditBeforePlacing);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::EditBeforePlacing)]), "{a:?}");
    // The style for new stamps: a setting of the tool, chosen in its list.
    let combo = app.hit(PanelHit::StampToolStyle);
    let before = app.h.nodes.clone();
    app.click_at(egui::pos2(combo.right() - 20.0, combo.center().y));
    app.idle(2);
    let added = app.h.new_since(&before);
    let classic = added.iter().find(|n| n.text() == "Classic sticker").cloned().unwrap_or_else(|| panic!("no Classic sticker entry: {added:?}"));
    app.click_at(classic.rect.center());
    app.idle(2);
    assert_eq!(app.state.tools.stamp.styles, StylePreset::ClassicSticker.styles([64, 64]), "the preset scaled to the 64 px picture");
    assert!(app.take().is_empty(), "a setting queues no action");
    // Without a source "Edit before placing" is disabled.
    app.state.tools.stamp.source = StampToolSource::None;
    app.idle(2);
    let a = app.click_hit(PanelHit::StampEditBeforePlacing);
    assert!(a.is_empty(), "{a:?}");

    // The Edit menu: Copy / Paste / Clear Style on the selected stamps.
    for (label, target_ok) in [("Copy Style", true), ("Clear Style", true)] {
        app.click("Edit");
        app.idle(1);
        let item = app.h.menu_item(label);
        app.click_at(item.rect.center());
        app.idle(1);
        let a = app.take();
        assert!(target_ok && matches!(a.as_slice(), [AppAction::Sticker(StickerAction::CopyStyle(StyleTarget::Stamps) | StickerAction::ClearStyle(StyleTarget::Stamps))]), "{label}: {a:?}");
    }
}

#[test]
fn the_sticker_view_tool_tab_has_the_layer_styles_and_the_menu_targets_layers() {
    let (mut app, stamp, _) = sticker_tab_app();
    crate::sticker::run_in(&mut app.state, StickerAction::Enter(Some(stamp)));
    app.state.upper_tab = UpperTab::Tool;
    app.idle(4);
    app.hit(PanelHit::StickerLayerStyles);
    let a = app.click_hit(PanelHit::StylePresetButton(0));
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::ApplyLayerPreset(StylePreset::ClassicSticker))]), "{a:?}");
    let a = app.click_hit(PanelHit::StyleCopy);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::CopyStyle(StyleTarget::Layers))]), "{a:?}");
    let a = app.click_hit(PanelHit::StyleBlockToggle(1));
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::SetLayerStyles(_, _, Commit::Final))]), "{a:?}");
    // The style commands of the Edit menu act on the sticker layers in the view.
    app.click("Edit");
    app.idle(1);
    let item = app.h.menu_item("Copy Style");
    app.click_at(item.rect.center());
    app.idle(1);
    let a = app.take();
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::CopyStyle(StyleTarget::Layers))]), "{a:?}");
    // The Sticker tab in the view still styles the stamp.
    app.state.upper_tab = UpperTab::Sticker;
    app.idle(3);
    let a = app.click_hit(PanelHit::StyleCopy);
    assert!(matches!(a.as_slice(), [AppAction::Sticker(StickerAction::CopyStyle(StyleTarget::Stamps))]), "{a:?}");
    assert!(matches!(app.state.doc().unwrap().stamp(stamp).unwrap().source, StampSource::Image { .. }), "nothing was edited");
}

/// Stickers P6b: the split can be dragged from its edge while the upper page overflows (640x480 with the two-row strip:
/// the Tool page's scroll area used to take the press, `SPLIT_CLEARANCE`).
#[test]
fn the_split_drags_from_its_edge_while_the_upper_page_overflows() {
    for (w, h) in [(640.0, 480.0), (640.0, 520.0), (900.0, 600.0)] {
        let mut app = PanelApp::new(sample_state(wte_core::SampleKind::Objects));
        app.h.size = vec2(w, h);
        app.idle(4);
        let auto = app.state.side_rects.clone();
        let edge = auto.bottom_area.center_top();
        app.drag(&[edge, edge - vec2(0.0, 10.0), edge - vec2(0.0, 50.0)]);
        app.idle(3);
        let r = app.state.side_rects.clone();
        let max = crate::panels::side_groups::layers_area_max(r.panel.height(), r.upper_strip_height);
        let want = (auto.bottom_area.height() + 50.0).min(max);
        assert!((r.bottom_area.height() - want).abs() < 2.0, "{w}x{h}: {} -> {} (want {want})", auto.bottom_area.height(), r.bottom_area.height());
    }
}
