// Owner: core-persist (C6). Export file naming of ARCHITECTURE.md 7.5: sanitizing user strings into Windows-safe
// file stems, the three name patterns, and the `_2`, `_3` de-duplication shared by the exporter and
// `project::extract_bundle`.

use std::collections::BTreeSet;

/// Longest stem `sanitize_file_stem` returns, in characters.
const MAX_STEM_CHARS: usize = 64;

/// True for characters that may not appear in a Windows file name.
fn is_forbidden(c: char) -> bool {
    c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// True when Windows treats a file with this stem as a device: the part before the first dot (trailing spaces
/// ignored) is CON, PRN, AUX, NUL, COM1-9 or LPT1-9 in any letter case.
fn is_reserved_device(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or(stem).trim_end_matches(' ');
    let upper = base.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    match (prefix.as_str(), chars.next(), chars.next()) {
        // Windows also reserves the superscript digits.
        ("COM" | "LPT", Some(digit), None) => matches!(digit, '1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'),
        _ => false,
    }
}

fn trim_end_dots_and_spaces(s: &str) -> &str {
    s.trim_end_matches(['.', ' '])
}

/// Makes a string safe as a Windows file stem (section 7.5): replaces `<>:"/\|?*` and control characters with `_`, trims
/// trailing dots and spaces, prefixes `_` to reserved device names, caps at 64 chars, empty => `untitled`.
pub fn sanitize_file_stem(s: &str) -> String {
    let replaced: String = s.chars().map(|c| if is_forbidden(c) { '_' } else { c }).take(MAX_STEM_CHARS).collect();
    let mut stem = trim_end_dots_and_spaces(&replaced).to_owned();
    if stem.is_empty() {
        return "untitled".to_owned();
    }
    if is_reserved_device(&stem) {
        stem.insert(0, '_');
        if stem.chars().count() > MAX_STEM_CHARS {
            stem = stem.chars().take(MAX_STEM_CHARS).collect();
            stem = trim_end_dots_and_spaces(&stem).to_owned();
        }
    }
    stem
}

/// Makes a file extension (without the dot) safe: forbidden characters, dots and spaces become `_`, at most 16
/// characters are kept. An empty result means "no extension".
pub(crate) fn sanitize_extension(ext: &str) -> String {
    ext.chars().map(|c| if is_forbidden(c) || c == '.' || c == ' ' { '_' } else { c }).take(16).collect()
}

/// True when `name` can be used unchanged as one directory level below an output folder: not empty, not `.` or `..`,
/// no forbidden character, no trailing dot or space and not a device name.
pub(crate) fn is_plain_dir_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.chars().any(is_forbidden)
        && !name.ends_with(['.', ' '])
        && !is_reserved_device(name)
}

/// `<base>` or `<base>_<set>`, both parts sanitized.
fn stem_for(base: &str, set_name: &str, multi: bool) -> String {
    let base = sanitize_file_stem(base);
    if multi { format!("{base}_{}", sanitize_file_stem(set_name)) } else { base }
}

/// Flattened texture file name for one set; `multi` = more than one set is exported. See section 7.5.
///
/// `<base>.<ext>` or `<base>_<set>.<ext>`; base and set name are sanitized here.
pub fn flat_name(base: &str, set_name: &str, multi: bool, ext: &str) -> String {
    format!("{}.{ext}", stem_for(base, set_name, multi))
}

/// Per-layer PNG file name for one set; `multi` = more than one set is exported. See section 7.5.
///
/// `<base>[_<set>]_L<index:02>_<kind>_<layer-name>.png`; index 0 is the bottom layer; base, set name and layer name
/// are sanitized here.
pub fn layer_name(base: &str, set_name: &str, multi: bool, index: usize, kind: &str, layer_name: &str) -> String {
    format!("{}_L{index:02}_{kind}_{}.png", stem_for(base, set_name, multi), sanitize_file_stem(layer_name))
}

/// Layered (ORA / PSD) file name for one set; `multi` = more than one set is exported. See section 7.5.
///
/// `<base>.<ext>` or `<base>_<set>.<ext>`; base and set name are sanitized here.
pub fn layered_name(base: &str, set_name: &str, multi: bool, ext: &str) -> String {
    format!("{}.{ext}", stem_for(base, set_name, multi))
}

/// File name of a single-package export (`ExportPackage::Zip`): `<base>.zip`, the base sanitized. See section 7.5.
pub fn package_name(base: &str) -> String {
    format!("{}.zip", sanitize_file_stem(base))
}

/// File name of a single-package export of a layer selection ("Export this layer", "Export selected layers"):
/// `<base>_selection.zip`, the base sanitized, so it never replaces the full package `<base>.zip`. See section 7.5.
pub fn selection_package_name(base: &str) -> String {
    format!("{}_selection.zip", sanitize_file_stem(base))
}

/// Splits a file name at its last dot into (stem, extension without the dot). A name without a dot, or with only a
/// leading one (".hidden"), has no extension.
pub(crate) fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot + 1..]),
        _ => (name, ""),
    }
}

/// Gives out file names that are unique without regard to letter case: a name that was already handed out (or that
/// the caller's `taken` test rejects) gets `_2`, `_3`, ... before its extension.
#[derive(Default)]
pub(crate) struct NameDeduper {
    used: BTreeSet<String>,
}

impl NameDeduper {
    /// The first of `name`, `<stem>_2.<ext>`, `<stem>_3.<ext>`, ... that is neither used yet nor `taken`.
    /// `scope` keeps names of different folders apart (pass the folder, or "" for a single folder).
    pub(crate) fn unique(&mut self, scope: &str, name: &str, taken: impl Fn(&str) -> bool) -> String {
        let (stem, ext) = split_extension(name);
        let mut counter = 1u32;
        loop {
            let candidate = match (counter, ext.is_empty()) {
                (1, _) => name.to_owned(),
                (n, true) => format!("{stem}_{n}"),
                (n, false) => format!("{stem}_{n}.{ext}"),
            };
            let key = format!("{}/{}", scope.to_lowercase(), candidate.to_lowercase());
            if !self.used.contains(&key) && !taken(&candidate) {
                self.used.insert(key);
                return candidate;
            }
            counter += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_forbidden_characters() {
        assert_eq!(sanitize_file_stem("a<b>c:d\"e/f\\g|h?i*j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_file_stem("tab\there\nnew\u{7f}del"), "tab_here_new_del");
        assert_eq!(sanitize_file_stem("Body Paint (final)"), "Body Paint (final)");
        assert_eq!(sanitize_file_stem("K\u{f6}rper \u{6728}"), "K\u{f6}rper \u{6728}");
        assert_eq!(sanitize_file_stem("..\\..\\evil"), ".._.._evil");
    }

    #[test]
    fn sanitize_trims_caps_and_fills() {
        assert_eq!(sanitize_file_stem("name. . ."), "name");
        assert_eq!(sanitize_file_stem(""), "untitled");
        assert_eq!(sanitize_file_stem(" . "), "untitled");
        assert_eq!(sanitize_file_stem("..."), "untitled");
        let long = "x".repeat(200);
        assert_eq!(sanitize_file_stem(&long).chars().count(), 64);
        // The cap counts characters, not bytes, and a space exposed by the cut is trimmed.
        let wide = "\u{6728}".repeat(70);
        assert_eq!(sanitize_file_stem(&wide).chars().count(), 64);
        let cut = format!("{} tail", "y".repeat(63));
        assert_eq!(sanitize_file_stem(&cut), "y".repeat(63));
    }

    #[test]
    fn sanitize_prefixes_reserved_device_names() {
        for name in ["CON", "con", "Prn", "AUX", "nul", "COM1", "com9", "LPT1", "lpt9", "CON.txt", "nul.tar.gz", "COM\u{b2}"] {
            assert_eq!(sanitize_file_stem(name), format!("_{name}"), "{name}");
        }
        for name in ["CONSOLE", "COM0", "COM10", "LPT", "aux2", "my.con", "null"] {
            assert_eq!(sanitize_file_stem(name), name, "{name}");
        }
        // "CON " is the device too; the trailing space is trimmed first.
        assert_eq!(sanitize_file_stem("CON "), "_CON");
        let long_device = format!("con.{}", "z".repeat(80));
        let s = sanitize_file_stem(&long_device);
        assert!(s.starts_with("_con.") && s.chars().count() == 64, "{s}");
    }

    #[test]
    fn name_patterns() {
        assert_eq!(flat_name("robot", "Body", false, "png"), "robot.png");
        assert_eq!(flat_name("robot", "Body", true, "tga"), "robot_Body.tga");
        assert_eq!(layered_name("robot", "Body", false, "psd"), "robot.psd");
        assert_eq!(layered_name("robot", "He:ad", true, "ora"), "robot_He_ad.ora");
        assert_eq!(layer_name("robot", "Body", false, 0, "paint", "Base"), "robot_L00_paint_Base.png");
        assert_eq!(layer_name("robot", "Body", true, 12, "text", "Hello?"), "robot_Body_L12_text_Hello_.png");
        assert_eq!(layer_name("", "", true, 3, "stamp", ""), "untitled_untitled_L03_stamp_untitled.png");
        assert_eq!(flat_name("nul", "x", false, "png"), "_nul.png");
        assert_eq!(layer_name("a", "b", false, 100, "paint", "c"), "a_L100_paint_c.png");
    }

    #[test]
    fn package_names() {
        assert_eq!(package_name("robot"), "robot.zip");
        assert_eq!(package_name("my: robot?"), "my_ robot_.zip");
        assert_eq!(package_name("con"), "_con.zip");
        assert_eq!(package_name("lpt1"), "_lpt1.zip");
        assert_eq!(package_name(""), "untitled.zip");
        assert_eq!(package_name("trailing. "), "trailing.zip");
    }

    #[test]
    fn split_extension_cases() {
        assert_eq!(split_extension("a.png"), ("a", "png"));
        assert_eq!(split_extension("a.b.png"), ("a.b", "png"));
        assert_eq!(split_extension("noext"), ("noext", ""));
        assert_eq!(split_extension(".hidden"), (".hidden", ""));
        assert_eq!(split_extension("trailing."), ("trailing", ""));
        assert_eq!(sanitize_extension("PNG"), "PNG");
        assert_eq!(sanitize_extension("p?g x"), "p_g_x");
        assert_eq!(sanitize_extension(""), "");
    }

    #[test]
    fn deduper_appends_counters_without_regard_to_case() {
        let mut d = NameDeduper::default();
        assert_eq!(d.unique("", "a.png", |_| false), "a.png");
        assert_eq!(d.unique("", "A.PNG", |_| false), "A_2.PNG");
        assert_eq!(d.unique("", "a.png", |_| false), "a_3.png");
        assert_eq!(d.unique("", "a_2.png", |_| false), "a_2_2.png");
        assert_eq!(d.unique("", "noext", |_| false), "noext");
        assert_eq!(d.unique("", "noext", |_| false), "noext_2");
        // Another folder is another namespace.
        assert_eq!(d.unique("sub", "a.png", |_| false), "a.png");
        // Names rejected by the caller (files that already exist) are skipped.
        assert_eq!(d.unique("", "b.png", |c| c == "b.png" || c == "b_2.png"), "b_3.png");
    }

    #[test]
    fn plain_dir_names() {
        for good in ["textures", "set00", "My Folder", "a.b"] {
            assert!(is_plain_dir_name(good), "{good}");
        }
        for bad in ["", ".", "..", "a:b", "a*b", "dot.", "space ", "con", "c:"] {
            assert!(!is_plain_dir_name(bad), "{bad}");
        }
    }
}
