p = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_app\src\selftest\scenarios.rs"
s = open(p, encoding="utf-8", newline="").read().replace("\r\n", "\n")
gate = open(r"E:\claude_programs\wrapped_texture_editor\target\round4\review\gate3\gate_scen.rs.txt", encoding="utf-8").read().replace("\r\n", "\n")
def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old[:90], s.count(old))
    s = s.replace(old, new)
rep('        "text_fonts" => text_fonts(),\n        _ => return None,',
    '        "text_fonts" => text_fonts(),\n        "text_preview_path" => text_preview_path(args.path.is_some()),\n        "brush_hover" => brush_hover(),\n        _ => return None,')
rep('        "workflow_path" | "fuzz_path" | "brush_types_path" => Some((Duration::from_secs(300), Duration::from_secs(3600))),',
    '        "workflow_path" | "fuzz_path" | "brush_types_path" | "text_preview_path" => {\n            Some((Duration::from_secs(300), Duration::from_secs(3600)))\n        }')
rep('        assert_eq!(scenario_names().len(), 35);', '        assert_eq!(scenario_names().len(), 37);')
rep('    "text_fonts",\n];', '    "text_fonts",\n    "text_preview_path",\n    "brush_hover",\n];')
marker = "\n#[cfg(test)]\nmod tests {"
assert s.count(marker) == 1
s = s.replace(marker, gate.rstrip("\n") + "\n" + marker)
open(p, "w", encoding="utf-8", newline="").write(s.replace("\n", "\r\n"))
print("ok part 3")
