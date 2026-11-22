import sys
root = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_core\src"
styled = root + r"\text\styled.rs"
s = open(styled, encoding="utf-8", newline="").read()
probe = '''
    /// Gate stage 3 (round 4): the long fuzz text of `fuzz_path` seed 3 on ngons_tiling (action 151) on a Circle of
    /// diameter 15.29 em, the frame that took 4.9 s in the app. Times the pictures the document asks for (Preview
    /// and Final at 256 and at the 128 fallback) and `text_frame`, median of 5. Run with
    /// `cargo.cmd test -p wte_core --release --lib gate_fuzz_circle_text -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing; run in release on an idle machine"]
    fn gate_fuzz_circle_text() {
        assert_geometry_is_real();
        let path = TextPath { shape: PathShape::Circle { diameter: 15.288544, center: [-0.957_658, -1.643_390_2] }, start: 0.0, flip: false };
        for px in [256.0_f32, 128.0] {
            let fonts = Fonts::ubuntu(px);
            let mut s = at("A much longer text stamp line for wrapping", px);
            s.fx.path = Some(path);
            let frame = text_frame(&fonts.get(), &s);
            eprintln!("fuzz circle text at {px}: text_frame {:?}", frame.as_ref().map(|f| f.box_px).map_err(|e| e.to_string()));
            let ms = median_ms(|| {
                std::hint::black_box(text_frame(&fonts.get(), &s).ok());
            });
            eprintln!("fuzz circle text at {px}: text_frame {ms:.3} ms");
            if frame.is_ok() {
                row(&format!("fuzz circle text preview at {px}"), &fonts, &s, PREVIEW, true, 16.0);
                row(&format!("fuzz circle text final at {px}"), &fonts, &s, RasterQuality::Full, false, 1500.0);
            }
        }
    }
'''
marker = "\r\n}\r\n"
assert s.endswith(marker)
if "gate_fuzz_circle_text" not in s:
    s = s[: -len(marker)] + "\r\n" + probe.replace("\n", "\r\n").rstrip("\r\n") + "\r\n" + marker[2:]
    open(styled, "w", encoding="utf-8", newline="").write(s)
print("styled ok")
