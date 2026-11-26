// objects-tabs (WP4 / WP5; OBJECTS_TABS_PLAN 8.3, ARCHITECTURE.md 8.11): the scenario `side_tabs` — the side panel's
// two tab groups in the real window. The Three Objects sample (built in) so the Objects tab has rows: a shot of every
// upper tab (`tab_tool`, `tab_color`, `tab_sets`, `tab_objects`: the Cube selected, the Ball hidden) and every lower tab
// (`tab_layers`, `tab_brush`, `tab_text`), the guarantees G1 / G2 with several tools, a tool change keeps the upper tab,
// the Text tool's focus shows the Tool tab, the split dragged and reset with a double-click, and the layout at 900x600
// and 640x480 (`small_900`, `small_640`). All asserts are on state; the shots are for the reviewer.

use super::*;

/// The `side_tabs` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn side_tabs() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Objects)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("defaults: the Tool tab above, Layers below, both groups in the panel", |s, sc| {
            if s.upper_tab != UpperTab::Tool || crate::panels::shown_tab(s) != SideTab::Layers {
                return Err(format!("upper {:?}, lower {:?}", s.upper_tab, s.side_tab));
            }
            groups_ok(s, sc)
        }),
    ];
    // (2) Every upper tab by a click on its strip button; for the Objects tab the Cube is selected and the Ball hidden.
    steps.extend([
        Step::Do(AppAction::SelectObject(1)),
        Step::Do(AppAction::SetObjectVisible(2, false)),
        Step::WaitIdle,
    ]);
    for (tab, shot) in [
        (UpperTab::Color, "tab_color"),
        (UpperTab::TextureSets, "tab_sets"),
        (UpperTab::Objects, "tab_objects"),
        (UpperTab::Tool, "tab_tool"),
    ] {
        steps.push(upper_tab_click(tab));
        steps.push(Step::WaitFrames(3));
        steps.push(Step::Assert(shot, page_check(tab)));
        steps.push(Step::Shot { name: shot, expect: "" });
    }
    steps.extend([
        Step::Assert("the Objects rows: the Cube selected, the Ball hidden", |s, _| {
            let session = session_of(s)?;
            if session.selected_objects.iter().copied().collect::<Vec<_>>() != [1] || !session.doc.object_hidden(2) {
                return Err(format!("selection {:?}, hidden {:?}", session.selected_objects, session.doc.hidden_objects()));
            }
            Ok(())
        }),
        Step::Do(AppAction::ShowAllObjects),
        Step::Do(AppAction::DeselectObjects),
        Step::WaitIdle,
    ]);

    // (3) G1: the Color tab with the Text, Brush (Brush tab below), Stamp and Transform (a text selected) tools.
    steps.push(Step::Setup("the Color tab", |s, _| {
        s.upper_tab = UpperTab::Color;
        Ok(())
    }));
    for tool in [ToolKind::Text, ToolKind::Brush, ToolKind::Stamp] {
        steps.extend([Step::Do(AppAction::SetTool(tool)), Step::WaitIdle, Step::WaitFrames(3), Step::Assert("G1: the whole Color page is on screen", g1)]);
    }
    steps.extend([
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Setup("the Brush tab below", |s, _| {
            s.side_tab = SideTab::Brush;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("G1 with the Brush tab below", g1),
        Step::Shot { name: "tab_brush", expect: "" },
        // (5) A tool change keeps the upper tab; the Text tool asking for the focus in its content field (a click in
        // the view with no text typed) shows the Tool tab; placing a text keeps the tab.
        Step::Do(AppAction::SetTool(ToolKind::Eraser)),
        Step::WaitFrames(2),
        Step::Assert("a tool change keeps the Color tab", |s, _| upper_is(s, UpperTab::Color)),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::WaitFrames(2),
        Step::Assert("choosing the Text tool keeps the Color tab", |s, _| upper_is(s, UpperTab::Color)),
        Step::Setup("no text typed", |s, _| {
            s.tools.text.content.clear();
            Ok(())
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitFrames(3),
        Step::Assert("the focus request shows the Tool tab and is served", |s, _| {
            upper_is(s, UpperTab::Tool)?;
            if s.focus_text_content { Err("the focus request is still pending".to_owned()) } else { Ok(()) }
        }),
        Step::Setup("type a text, back to the Color tab", |s, _| {
            s.tools.text.content = "Tabs".to_owned();
            s.upper_tab = UpperTab::Color;
            Ok(())
        }),
        click(ViewId::Main3D, [0.5, 0.5]),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("placing a text keeps the Color tab and selects the text", |s, _| {
            upper_is(s, UpperTab::Color)?;
            let session = session_of(s)?;
            let selected = session.selected_stamp.and_then(|id| session.doc.stamp(id)).ok_or("no stamp is selected")?;
            if !matches!(selected.source, StampSource::Text(_)) {
                return Err("the selected stamp is not a text".to_owned());
            }
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::Setup("the Text tab below, the Color tab above", |s, _| {
            s.side_tab = SideTab::Text;
            s.upper_tab = UpperTab::Color;
            Ok(())
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("G1 with a selected text (Transform) and the Text tab below", g1),
        Step::Shot { name: "tab_text", expect: "" },
        // (4) G2: the Tool tab with the Brush, with the Layers and with the Brush tab below.
        Step::Do(AppAction::SelectStamp(None)),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Setup("the Tool tab, Layers below", |s, _| {
            s.upper_tab = UpperTab::Tool;
            s.side_tab = SideTab::Layers;
            Ok(())
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("G2 with the Layers tab below", g2),
        Step::Shot { name: "tab_layers", expect: "" },
        Step::Setup("the Brush tab below", |s, _| {
            s.side_tab = SideTab::Brush;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("G2 with the Brush tab below", g2),
        // (8) The lower one tab rule still holds.
        Step::Do(AppAction::SetTool(ToolKind::Fill)),
        Step::WaitFrames(3),
        Step::Assert("the Fill tool shows Layers below, the chosen tab kept", |s, _| {
            tabs_shown(s, false, false)?;
            if s.side_tab != SideTab::Brush { Err(format!("side_tab is {:?}", s.side_tab)) } else { Ok(()) }
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::WaitFrames(3),
        Step::Assert("the Brush tab comes back with the Brush", |s, _| tabs_shown(s, true, false)),
        Step::Dyn("click the Layers tab", |s, _| tab_click(s, SideTab::Layers)),
        Step::WaitFrames(3),
        Step::Assert("the Layers tab is shown", |s, _| tabs_shown(s, false, false)),
        // (6) The split: dragged 80 pt up, kept for another lower tab, back to automatic with a double-click.
        Step::Setup("remember the automatic lower height", |s, sc| {
            sc.values.insert("auto_lower", u64::from(s.side_rects.bottom_area.height().to_bits()));
            Ok(())
        }),
        Step::Dyn("drag the split 80 pt up", |s, _| {
            let edge = s.side_rects.bottom_area.center_top();
            Ok(vec![Step::ScreenPointer { path: vec![edge, edge - egui::vec2(0.0, 20.0), edge - egui::vec2(0.0, 50.0), edge - egui::vec2(0.0, 80.0)] }])
        }),
        Step::WaitFrames(3),
        Step::Assert("the lower group is 80 pt taller, the groups apart", |s, sc| {
            let auto = f32::from_bits(remembered(sc, "auto_lower")? as u32);
            let now = s.side_rects.bottom_area.height();
            // In a small window the split stops at its largest height.
            let want = (auto + 80.0).min(crate::panels::side_groups::layers_area_max(s.side_rects.panel.height()));
            if (now - want).abs() > 4.0 {
                return Err(format!("the lower group is {now:.1} pt, expected {want:.1}"));
            }
            groups_ok(s, sc)
        }),
        Step::Setup("the Brush tab below", |s, sc| {
            sc.values.insert("dragged_lower", u64::from(s.side_rects.bottom_area.height().to_bits()));
            s.side_tab = SideTab::Brush;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the Brush tab keeps the dragged height", |s, sc| {
            let dragged = f32::from_bits(remembered(sc, "dragged_lower")? as u32);
            let now = s.side_rects.bottom_area.height();
            if (now - dragged).abs() > 1.0 { Err(format!("{now:.1} pt, dragged {dragged:.1}")) } else { Ok(()) }
        }),
        Step::Setup("back to the Layers tab", |s, _| {
            s.side_tab = SideTab::Layers;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Dyn("double-click the split", |s, _| {
            let edge = s.side_rects.bottom_area.center_top();
            let at = |e| Step::ScreenClick { at: e, button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE };
            Ok(vec![at(edge), at(edge)])
        }),
        Step::WaitFrames(4),
        Step::Assert("a double-click returns to the automatic height", |s, sc| {
            let auto = f32::from_bits(remembered(sc, "auto_lower")? as u32);
            let now = s.side_rects.bottom_area.height();
            if (now - auto).abs() > 1.0 { Err(format!("{now:.1} pt, automatic {auto:.1}")) } else { Ok(()) }
        }),
    ]);
    // (7) Smaller windows: the Color page (it scrolls or shrinks), nothing overlaps.
    for (size, shot) in [([900, 600], "small_900"), ([640, 480], "small_640")] {
        steps.extend([
            Step::Setup("the Color tab", |s, _| {
                s.upper_tab = UpperTab::Color;
                Ok(())
            }),
            Step::Resize(size),
            Step::WaitFrames(3),
            Step::Assert("the groups stay inside the panel and apart", |s, sc| {
                if s.side_rects.upper_page.height() < 90.0 {
                    return Err(format!("the upper page is only {:.0} pt high: {:?}", s.side_rects.upper_page.height(), s.side_rects));
                }
                groups_ok(s, sc)
            }),
            Step::Shot { name: shot, expect: "" },
        ]);
    }
    steps.extend([
        Step::Resize([1280, 800]),
        Step::Setup("the Objects tab for the final screenshot", |s, _| {
            s.upper_tab = UpperTab::Objects;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the final layout", groups_ok),
    ]);
    steps
}

/// A click on the upper strip's button of `tab` (by its recorded rect).
fn upper_tab_click(tab: UpperTab) -> Step {
    let f: fn(&AppState, &Scratch) -> Result<Vec<Step>, String> = match tab {
        UpperTab::Tool => |s, _| button_click(s, UpperTab::Tool),
        UpperTab::Color => |s, _| button_click(s, UpperTab::Color),
        UpperTab::TextureSets => |s, _| button_click(s, UpperTab::TextureSets),
        UpperTab::Objects => |s, _| button_click(s, UpperTab::Objects),
    };
    Step::Dyn("click an upper tab button", f)
}

fn button_click(s: &AppState, tab: UpperTab) -> Result<Vec<Step>, String> {
    let rect = s.side_rects.upper_tab_buttons[tab.index()];
    if !rect.is_positive() {
        return Err(format!("the {tab:?} button was not drawn"));
    }
    Ok(vec![Step::ScreenPointer { path: vec![rect.center()] }])
}

/// After a click on the button of `tab`: exactly that page is drawn (and the layout holds).
fn page_check(tab: UpperTab) -> fn(&AppState, &Scratch) -> Result<(), String> {
    match tab {
        UpperTab::Tool => |s, sc| one_page(s, sc, UpperTab::Tool),
        UpperTab::Color => |s, sc| one_page(s, sc, UpperTab::Color),
        UpperTab::TextureSets => |s, sc| one_page(s, sc, UpperTab::TextureSets),
        UpperTab::Objects => |s, sc| one_page(s, sc, UpperTab::Objects),
    }
}

fn one_page(s: &AppState, sc: &Scratch, tab: UpperTab) -> Result<(), String> {
    upper_is(s, tab)?;
    let r = &s.side_rects;
    let drawn = [r.tool_area.is_positive(), r.color.is_positive(), r.sets_tab.is_some(), r.objects_tab.is_some()];
    let mut want = [false; 4];
    want[tab.index()] = true;
    if drawn != want {
        return Err(format!("{tab:?}: pages drawn {drawn:?}"));
    }
    // G4: the four buttons in one row inside the upper group, the side panel at most 300 pt wide.
    let b = r.upper_tab_buttons;
    if b.iter().any(|x| (x.top() - b[0].top()).abs() > 0.5 || !r.upper_area.expand(0.5).contains_rect(*x)) {
        return Err(format!("the upper tab buttons {b:?} are not one row inside {:?}", r.upper_area));
    }
    let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?;
    if side.width() > 300.5 {
        return Err(format!("the side panel is {} pt wide", side.width()));
    }
    if tab == UpperTab::Objects {
        for i in 0..3 {
            panel_rect(s, PanelHit::ObjectRow(i))?;
        }
        panel_rect(s, PanelHit::ObjectsFooter)?;
    }
    log::info!("side_tabs: {tab:?} page {:?}, buttons {b:?} ({:.1} pt)", r.upper_page, b[3].right() - b[0].left());
    groups_ok(s, sc)
}

fn upper_is(s: &AppState, tab: UpperTab) -> Result<(), String> {
    if s.upper_tab == tab { Ok(()) } else { Err(format!("the upper tab is {:?}, expected {tab:?}", s.upper_tab)) }
}

/// The guarantees G1 and G2 are defined for a window of 1280x800 or larger (side panel at least 740 pt high); the run
/// is also started at 900x600 and 640x480, where only the layout rules hold.
fn large(s: &AppState) -> bool {
    s.side_panel_rect.is_some_and(|r| r.height() >= 740.0)
}

/// G1: the Color page is shown and nothing of it is scrolled away (at 1280x800 and above).
fn g1(s: &AppState, sc: &Scratch) -> Result<(), String> {
    upper_is(s, UpperTab::Color)?;
    let color = panel_rect(s, PanelHit::ColorSection)?;
    let content = panel_rect(s, PanelHit::ColorContent)?;
    if large(s) && !color.expand(0.5).contains_rect(content) {
        return Err(format!("the Color page is cut off: content {content:?} in {color:?}"));
    }
    log::info!("side_tabs G1 ({:?}, lower {:?}): Color page {color:?}, content {:.1} pt", s.tool, s.side_tab, content.height());
    groups_ok(s, sc)
}

/// G2: the Tool page with the Brush shows its whole content (the heading and the five slider rows) without scrolling.
fn g2(s: &AppState, sc: &Scratch) -> Result<(), String> {
    upper_is(s, UpperTab::Tool)?;
    let r = &s.side_rects;
    if large(s) && r.tool_options.bottom() > r.tool_area.bottom() + 0.5 {
        return Err(format!("the Brush options {:?} end below the Tool page {:?}", r.tool_options, r.tool_area));
    }
    log::info!("side_tabs G2 (lower {:?}): Tool page {:?}, options {:?}", s.side_tab, r.tool_area, r.tool_options);
    groups_ok(s, sc)
}

/// The groups lie inside the side panel, apart, with every strip button in its group; with the Layers tab shown also
/// the Layers checks of `assert_side_panel_layout` (through `assert_side_areas_apart`).
fn groups_ok(s: &AppState, sc: &Scratch) -> Result<(), String> {
    if crate::panels::shown_tab(s) == SideTab::Layers {
        return assert_side_areas_apart(s, sc);
    }
    let r = &s.side_rects;
    let side = s.side_panel_rect.ok_or("the side panel rect was not recorded")?.expand(0.5);
    if !side.contains_rect(r.upper_area) || !side.contains_rect(r.bottom_area) || r.upper_area.bottom() > r.bottom_area.top() + 0.5 {
        return Err(format!("the groups are not apart inside the side panel {side:?}: {r:?}"));
    }
    if r.upper_tab_buttons.iter().any(|b| !r.upper_area.expand(0.5).contains_rect(*b)) || r.tab_buttons.iter().any(|b| !r.bottom_area.expand(0.5).contains_rect(*b)) {
        return Err(format!("a strip button lies outside its group: {r:?}"));
    }
    Ok(())
}
