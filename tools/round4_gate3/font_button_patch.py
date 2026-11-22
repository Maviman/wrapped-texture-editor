import re
base = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_app\src"
def patch(path, old, new):
    s = open(path, encoding="utf-8", newline="").read()
    crlf = "\r\n" in s
    if crlf:
        old, new = old.replace("\n", "\r\n"), new.replace("\n", "\r\n")
    assert s.count(old) == 1, (path, s.count(old))
    s = s.replace(old, new)
    open(path, "w", encoding="utf-8", newline="").write(s)
patch(base + r"\state.rs", """    OpeningsOk,
    OpeningsCancel,
}""", """    OpeningsOk,
    OpeningsCancel,
    /// Round 4 (gate): the font button of the Text block (row 2), which opens the font picker (written by
    /// `font_picker::button`, for the self-test).
    FontButton,
}""")
patch(base + r"\panels\font_picker.rs", """    let resp = if why.is_empty() { resp.on_hover_text(format!("Font: {family}. Click to choose another font")) } else { resp.on_hover_text(why) };
""", """    let resp = if why.is_empty() { resp.on_hover_text(format!("Font: {family}. Click to choose another font")) } else { resp.on_hover_text(why) };
    state.panel_rects.insert(crate::state::PanelHit::FontButton, resp.rect);
""")
print("ok")
