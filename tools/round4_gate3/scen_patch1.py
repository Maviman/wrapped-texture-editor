p = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_app\src\selftest\scenarios.rs"
s = open(p, encoding="utf-8", newline="").read().replace("\r\n", "\n")
def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old[:80], s.count(old))
    s = s.replace(old, new)
# 1. closing dialog shot
rep("""        Step::Dyn("click \\"Flat caps\\" in the dialog", |s, _| {""", """        // Gate 3 (ROUND4_PLAN 3.3 item 7): the closing dialog of an open model, `<out>_dialog.png`.
        Step::Shot("dialog"),
        Step::Dyn("click \\"Flat caps\\" in the dialog", |s, _| {""")
# 2. text_path circle shot (3D only)
rep("""fn t7_path_steps() -> Vec<Step> {
    vec![""", """fn t7_path_steps() -> Vec<Step> {
    t7_path_steps_shot(None)
}

/// `t7_path_steps` with a named capture of the text on its Circle (gate 3, ROUND4_PLAN 3.3 item 7).
fn t7_path_steps_shot(circle_shot: Option<&'static str>) -> Vec<Step> {
    let mut steps = vec![""")
rep("""        Step::Setup("remember the Circle", t7_remember_rect),
        Step::Setup("Path: Curve", |s, _| {""", """        Step::Setup("remember the Circle", t7_remember_rect),
    ];
    if let Some(name) = circle_shot {
        steps.extend([Step::WaitFrames(3), Step::Shot(name)]);
    }
    steps.extend([
        Step::Setup("Path: Curve", |s, _| {""")
open(p, "w", encoding="utf-8", newline="").write(s.replace("\n", "\r\n"))
print("ok part 1")
