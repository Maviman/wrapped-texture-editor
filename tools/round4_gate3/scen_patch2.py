p = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_app\src\selftest\scenarios.rs"
s = open(p, encoding="utf-8", newline="").read().replace("\r\n", "\n")
def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old[:90], s.count(old))
    s = s.replace(old, new)
rep("""            if first >= 1.5 || last >= 1.5 {
                return Err(format!("the first point moved {first} pt, the last {last} pt"));
            }
            Ok(())
        }),
    ]
}
""", """            if first >= 1.5 || last >= 1.5 {
                return Err(format!("the first point moved {first} pt, the last {last} pt"));
            }
            Ok(())
        }),
    ]);
    steps
}
""")
rep("""    steps.extend(t7_place_3d(has_path));
    steps.extend(t7_path_steps());
    steps.extend(t7_place_2d());
    steps.extend(t7_path_steps());""", """    steps.extend(t7_place_3d(has_path));
    // Gate 3 (ROUND4_PLAN 3.3 item 7): the text on its Circle on the model, `<out>_circle3d.png`.
    steps.extend(t7_path_steps_shot(Some("circle3d")));
    steps.extend(t7_place_2d());
    steps.extend(t7_path_steps());""")
rep("""    steps.extend(t7_warp_steps());
    steps.extend([Step::WaitFrames(3), Step::Assert("the 3D view rectangle did not move", same_view_rect)]);""",
"""    steps.extend(t7_warp_steps());
    steps.extend([Step::WaitFrames(3), Step::Assert("the 3D view rectangle did not move", same_view_rect)]);
    // Gate 3 (ROUND4_PLAN 3.3 item 7): the warped text (Arc 50 %) on the model, `<out>_warp3d.png`.
    steps.push(Step::Shot("warp3d"));""")
rep("""    steps.extend(t7_place_3d(false));
    steps.extend(t7_gap_steps());
    steps.extend(t7_place_2d());""", """    steps.extend(t7_place_3d(false));
    steps.extend(t7_gap_steps());
    // Gate 3 (ROUND4_PLAN 3.3 item 7): the text with one widened gap on the model, `<out>_gap3d.png`.
    steps.extend([Step::WaitFrames(3), Step::Shot("gap3d")]);
    steps.extend(t7_place_2d());""")
# text_tab pages: shots in the second call
rep("""    steps.extend(text_tab_pages());
    steps.extend([
        Step::Dyn("place the text on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.05])),""",
"""    steps.extend(text_tab_pages(false));
    steps.extend([
        Step::Dyn("place the text on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.05])),""")
rep("""    steps.extend(text_tab_pages());
    steps.extend([
        // The draft step""", """    // Gate 3 (ROUND4_PLAN 3.3 item 7): each page of the Text tab with the placed text, `<out>_page_<name>.png`.
    steps.extend(text_tab_pages(true));
    steps.extend([
        // The draft step""")
rep("""fn text_tab_pages() -> Vec<Step> {
    let mut steps = Vec::new();
    for (page, name) in [(TextPage::Character, "Character"), (TextPage::Warp, "Warp"), (TextPage::Path, "Path")] {""",
"""fn text_tab_pages(shots: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    for (page, name, shot) in [
        (TextPage::Character, "Character", "page_character"),
        (TextPage::Warp, "Warp", "page_warp"),
        (TextPage::Path, "Path", "page_path"),
    ] {""")
rep("""            text_page_layout(s, "page")?;
            same_view_rect(s, sc)
        }));
    }
    steps
}""", """            text_page_layout(s, "page")?;
            same_view_rect(s, sc)
        }));
        if shots {
            steps.push(Step::Shot(shot));
        }
    }
    steps
}""")
# text_fonts: the font picker with the machine fonts
rep("""        Step::Assert("the two pictures have ink (more than 200 pixels) and no fully inked row or column", |s, sc| {
            let _ = s;
            for key in ["otf_ink", "bahnschrift_ink"] {
                if let Some(&ink) = sc.values.get(key)
                    && ink <= 200
                {
                    return Err(format!("{key}: {ink} inked pixels"));
                }
            }
            Ok(())
        }),
    ]
}""", """        Step::Assert("the two pictures have ink (more than 200 pixels) and no fully inked row or column", |s, sc| {
            let _ = s;
            for key in ["otf_ink", "bahnschrift_ink"] {
                if let Some(&ink) = sc.values.get(key)
                    && ink <= 200
                {
                    return Err(format!("{key}: {ink} inked pixels"));
                }
            }
            Ok(())
        }),
        // Gate 3 (FEATURE_TEXT 8.4 "opening the font picker with all families"; ROUND4_PLAN 3.3 item 7): a click on
        // the font button opens the picker; the sample job runs; 30 frames with the picker open, then
        // `<out>_picker.png`. The log lines "gate: font picker open/closed" frame the frames (WTE_FRAME_LOG).
        Step::Setup("gate: font picker open", |_, _| {
            log::info!("gate: font picker open");
            Ok(())
        }),
        Step::Dyn("click the font button", |s, _| {
            Ok(vec![Step::ScreenPointer { path: vec![panel_rect(s, PanelHit::FontButton)?.center()] }])
        }),
        Step::WaitFrames(2),
        Step::Dyn("wait for the font samples", wait_for_samples),
        Step::WaitFrames(30),
        Step::Assert("the picker drew the samples of every family", |s, _| {
            let families = s.fonts.families().len();
            if s.font_samples.requested_for != Some(families) || s.font_samples.loading {
                return Err(format!("samples for {:?} of {families} families, loading {}", s.font_samples.requested_for, s.font_samples.loading));
            }
            log::info!("font picker: samples of {families} families");
            Ok(())
        }),
        Step::Shot("picker"),
        Step::Setup("gate: font picker closed", |_, _| {
            log::info!("gate: font picker closed");
            Ok(())
        }),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        Step::WaitIdle,
    ]
}

/// While the sample job of the font picker runs: wait and look again (at most about 30 s: 1,800 frames of 20).
fn wait_for_samples(s: &AppState, _: &Scratch) -> Result<Vec<Step>, String> {
    if s.font_samples.loading || s.font_samples.requested_for.is_none() {
        Ok(vec![Step::WaitFrames(10), Step::Dyn("wait for the font samples", wait_for_samples)])
    } else {
        Ok(Vec::new())
    }
}""")
open(p, "w", encoding="utf-8", newline="").write(s.replace("\n", "\r\n"))
print("ok part 2")
