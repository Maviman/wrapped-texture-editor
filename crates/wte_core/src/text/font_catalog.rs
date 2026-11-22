// Owner: core-stamps-text (C4). Font enumeration (fontdb) and parsed-font cache (fontdue), ARCHITECTURE.md 5.13 / 6.8.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use crate::blob::{BlobKind, BlobStore};
use crate::error::TextError;
use crate::ids::BlobId;
use crate::text::FontRef;
use crate::text::{outline, scan};

/// One font face of the catalog.
#[derive(Clone, Debug)]
pub struct FontFaceInfo {
    pub id: fontdb::ID,
    /// `FaceInfo.families[0].0`.
    pub family: String,
    /// `style_name(weight, slant)`, e.g. "Bold Italic". A later face of the family with a style name that is already
    /// taken gets `"<style> (<PostScript name>)"` (FEATURE_TEXT 5.13), so every face stays reachable.
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub monospaced: bool,
    pub face_index: u32,
    /// `FaceInfo.post_script_name` (name id 6); may be empty.
    pub post_script_name: String,
    /// OS/2 width class (5 = normal).
    pub stretch: u16,
    /// Size of the whole font file (a collection counts once per face); 0 when it is unknown.
    pub file_bytes: u64,
}

/// What a face's bytes come from: one font file, or one allocation of loaded bytes.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum SourceKey {
    File(PathBuf),
    Memory(usize),
}

fn source_key(source: &fontdb::Source) -> SourceKey {
    match source {
        fontdb::Source::Binary(data) => SourceKey::Memory(Arc::as_ptr(data) as *const () as usize),
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => SourceKey::File(path.clone()),
    }
}

/// The font file extensions the catalog loads, compared without case.
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "ttc", "otf", "otc"];

/// The folders `fontdb::Database::load_system_fonts` scans on this platform (the same paths, so the same strings).
fn system_font_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut dirs = Vec::new();
        match std::env::var_os("SYSTEMROOT") {
            Some(root) => dirs.push(Path::new(&root).join("Fonts")),
            None => dirs.push(PathBuf::from("C:\\Windows\\Fonts\\")),
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            let home = Path::new(&home);
            dirs.push(home.join("AppData\\Local\\Microsoft\\Windows\\Fonts"));
            dirs.push(home.join("AppData\\Roaming\\Microsoft\\Windows\\Fonts"));
        }
        dirs
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

/// Family names sorted without regard to case, ties by the original string; names equal without case are one family
/// (`faces` matches without case), the first spelling in that order is kept.
fn sort_families(families: &mut Vec<String>) {
    families.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    families.dedup_by(|later, kept| later.to_lowercase() == kept.to_lowercase());
}

/// Width of a sample of the font picker, in pixels (FEATURE_TEXT 5.13).
pub const SAMPLE_MAX_WIDTH: u32 = 128;
/// Height of a sample of the font picker, in pixels.
pub const SAMPLE_MAX_HEIGHT: u32 = 20;
/// The text of a sample.
pub const SAMPLE_TEXT: &str = "Abc 123";
/// The size of a sample, pixels per em.
pub const SAMPLE_PX: f32 = 16.0;

/// The one definition of a style string (shown in the UI, persisted in FontRef.style, looked up by `find`).
/// fontdb exposes no subfamily name, so it is derived: weight name from `FaceInfo.weight.0` — 100 "Thin", 200 "ExtraLight",
/// 300 "Light", 400 "Regular", 500 "Medium", 600 "SemiBold", 700 "Bold", 800 "ExtraBold", 900 "Black", any other value "W<n>" —
/// followed by " Italic" / " Oblique" for `fontdb::Style::Italic` / `Oblique`; "Regular Italic" collapses to "Italic",
/// "Regular Oblique" to "Oblique".
pub fn style_name(weight: u16, style: fontdb::Style) -> String {
    let weight_name = match weight {
        100 => "Thin".to_owned(),
        200 => "ExtraLight".to_owned(),
        300 => "Light".to_owned(),
        400 => "Regular".to_owned(),
        500 => "Medium".to_owned(),
        600 => "SemiBold".to_owned(),
        700 => "Bold".to_owned(),
        800 => "ExtraBold".to_owned(),
        900 => "Black".to_owned(),
        other => format!("W{other}"),
    };
    let slant = match style {
        fontdb::Style::Normal => return weight_name,
        fontdb::Style::Italic => "Italic",
        fontdb::Style::Oblique => "Oblique",
    };
    if weight == 400 { slant.to_owned() } else { format!("{weight_name} {slant}") }
}

/// System + bundled font enumeration (fontdb). Send + Sync.
pub struct FontCatalog {
    db: fontdb::Database,
    /// First family name of every face: sorted without case, de-duplicated without case.
    families: Vec<String>,
    /// Sizes of the font files, read once when they are loaded (`faces()` touches no file).
    file_sizes: HashMap<PathBuf, u64>,
    /// The bytes `face_bytes` handed out, per font file, while someone holds them.
    shared: Mutex<HashMap<SourceKey, Weak<[u8]>>>,
}

/// A copy for a worker thread (round 4 T7: the font picker's sample job draws from it while the app keeps its own).
/// The face list and the file sizes are copied (fonts of files stay files; nothing is read); the shared font bytes
/// are not: the copy starts with none.
impl Clone for FontCatalog {
    fn clone(&self) -> FontCatalog {
        FontCatalog {
            db: self.db.clone(),
            families: self.families.clone(),
            file_sizes: self.file_sizes.clone(),
            shared: Mutex::new(HashMap::new()),
        }
    }
}

/// The first (English, when the font has one) family name of a face.
fn family_of(face: &fontdb::FaceInfo) -> Option<&str> {
    face.families.first().map(|(name, _)| name.as_str())
}

fn same_family(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

impl FontCatalog {
    /// Catalog holding only the bundled fonts: Ubuntu-Light and Hack-Regular from epaint_default_fonts.
    pub fn bundled() -> FontCatalog {
        let mut db = fontdb::Database::new();
        // The bundled faces are loaded first, so they win over installed copies of the same style (`faces`).
        db.load_font_source(fontdb::Source::Binary(Arc::new(epaint_default_fonts::UBUNTU_LIGHT)));
        db.load_font_source(fontdb::Source::Binary(Arc::new(epaint_default_fonts::HACK_REGULAR)));
        let mut catalog =
            FontCatalog { db, families: Vec::new(), file_sizes: HashMap::new(), shared: Mutex::new(HashMap::new()) };
        catalog.refresh_families();
        catalog
    }

    /// Adds the system fonts (`Database::load_system_fonts()`) plus the files of the same folders whose extension
    /// fontdb skips (mixed case such as `.Ttf`), and reads every font file's size once; slow, call off the UI thread.
    pub fn load_system(&mut self) {
        self.db.load_system_fonts();
        for dir in system_font_dirs() {
            self.scan_dir(&dir, 0);
        }
        self.read_missing_sizes();
        self.refresh_families();
    }

    /// Records the size of every font file below `dir` (from the directory listing) and loads the files whose
    /// extension is a font extension in mixed case, which `load_fonts_dir` skips.
    fn scan_dir(&mut self, dir: &Path, depth: u32) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else { continue };
            let path = entry.path();
            if file_type.is_dir() {
                if depth < 8 {
                    self.scan_dir(&path, depth + 1);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
            let lower = ext.to_ascii_lowercase();
            if !FONT_EXTENSIONS.contains(&lower.as_str()) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                self.file_sizes.insert(path.clone(), meta.len());
            }
            let fontdb_loaded_it = ext == lower || ext == ext.to_ascii_uppercase();
            if !fontdb_loaded_it {
                // A malformed file is skipped, as fontdb does.
                let _ = self.db.load_font_file(&path);
            }
        }
    }

    /// One `fs::metadata` for every font file of the database whose size is not known yet.
    fn read_missing_sizes(&mut self) {
        let missing: HashSet<PathBuf> = self
            .db
            .faces()
            .filter_map(|face| match &face.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                    (!self.file_sizes.contains_key(path)).then(|| path.clone())
                }
                fontdb::Source::Binary(_) => None,
            })
            .collect();
        for path in missing {
            if let Ok(meta) = std::fs::metadata(&path) {
                self.file_sizes.insert(path, meta.len());
            }
        }
    }

    /// Loads one font file (all faces of a collection) and reads its size once.
    ///
    /// Once fontdb has loaded the faces the call succeeds: a size that cannot be read (the file vanished in between)
    /// is left unknown (0), as in `read_missing_sizes`, and the family list is refreshed either way.
    pub fn load_font_file(&mut self, path: &Path) -> std::io::Result<()> {
        self.db.load_font_file(path)?;
        self.file_loaded(path, std::fs::metadata(path).map(|m| m.len()));
        Ok(())
    }

    /// After fontdb loaded `path`: the family list and the file size (when it could be read).
    fn file_loaded(&mut self, path: &Path, size: std::io::Result<u64>) {
        self.refresh_families();
        if let Ok(len) = size {
            self.file_sizes.insert(path.to_path_buf(), len);
        }
    }

    fn refresh_families(&mut self) {
        let mut families: Vec<String> = self.db.faces().filter_map(family_of).map(str::to_owned).collect();
        sort_families(&mut families);
        self.families = families;
    }

    /// Family names, sorted without regard to case (ties by the original string), de-duplicated without case (the
    /// first spelling kept).
    pub fn families(&self) -> &[String] {
        &self.families
    }

    fn file_bytes(&self, source: &fontdb::Source) -> u64 {
        match source {
            fontdb::Source::Binary(data) => (**data).as_ref().len() as u64,
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                self.file_sizes.get(path).copied().unwrap_or(0)
            }
        }
    }

    /// Faces of a family, sorted by (weight, italic), faces with the same key in load order (bundled fonts before
    /// system fonts).
    ///
    /// The family is matched like in `find` (case-insensitive). No face is dropped: the first face with a style name
    /// keeps it, a later one is named `"<style> (<PostScript name>)"` (FEATURE_TEXT 5.13).
    pub fn faces(&self, family: &str) -> Vec<FontFaceInfo> {
        let mut faces: Vec<FontFaceInfo> = self
            .db
            .faces()
            .filter_map(|face| {
                let name = family_of(face)?;
                if !same_family(name, family) {
                    return None;
                }
                Some(FontFaceInfo {
                    id: face.id,
                    family: name.to_owned(),
                    style: style_name(face.weight.0, face.style),
                    weight: face.weight.0,
                    italic: face.style != fontdb::Style::Normal,
                    monospaced: face.monospaced,
                    face_index: face.index,
                    post_script_name: face.post_script_name.clone(),
                    stretch: face.stretch.to_number(),
                    file_bytes: self.file_bytes(&face.source),
                })
            })
            .collect();
        // Stable: faces with the same key keep their load order.
        faces.sort_by_key(|f| (f.weight, f.italic));
        let mut seen: HashSet<String> = HashSet::new();
        for face in &mut faces {
            if seen.contains(&face.style) {
                let base = if face.post_script_name.is_empty() {
                    format!("{} (face {})", face.style, face.face_index)
                } else {
                    format!("{} ({})", face.style, face.post_script_name)
                };
                let mut name = base.clone();
                let mut n = 2;
                while seen.contains(&name) {
                    name = format!("{base} {n}");
                    n += 1;
                }
                face.style = name;
            }
            seen.insert(face.style.clone());
        }
        faces
    }

    /// Exact family (case-insensitive) + exact style string.
    pub fn find(&self, family: &str, style: &str) -> Option<FontFaceInfo> {
        self.faces(family).into_iter().find(|f| f.style == style)
    }

    /// Whole font file + face index.
    ///
    /// The faces of one font file share one allocation while anyone holds it (FEATURE_TEXT 5.13): a second call for
    /// any face of the same file returns the same `Arc` without reading the file again, so `BlobStore::insert`
    /// recognises it before hashing. `None` for an unknown id or when the font file can no longer be read.
    pub fn face_bytes(&self, id: fontdb::ID) -> Option<(Arc<[u8]>, u32)> {
        let face = self.db.face(id)?;
        let key = source_key(&face.source);
        let lock = || self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(bytes) = lock().get(&key).and_then(Weak::upgrade) {
            return Some((bytes, face.index));
        }
        // The copy (up to tens of MB for a collection) is made without the lock, so other callers do not wait.
        let (bytes, face_index) = self.db.with_face_data(id, |data, face_index| (Arc::<[u8]>::from(data), face_index))?;
        let mut shared = lock();
        // Another thread may have copied the same file meanwhile: everyone keeps the allocation stored first.
        if let Some(existing) = shared.get(&key).and_then(Weak::upgrade) {
            return Some((existing, face_index));
        }
        shared.retain(|_, weak| weak.strong_count() > 0);
        shared.insert(key, Arc::downgrade(&bytes));
        Some((bytes, face_index))
    }

    /// A sample of face `id` for the font picker (FEATURE_TEXT 5.13): `text` (or, when the face maps none of its
    /// characters, the face's first six mapped characters) at `px` pixels per em as a coverage bitmap
    /// `(width, height, bytes)` of at most 128 x 20 px, row 0 on top. Outlines and coverage come from the styled
    /// painter (`outline`, `scan`), characters are mapped as the painter maps them (`outline::char_to_glyph`). The
    /// font is read in place (fontdb maps the file); nothing is copied and `fontdue` does not parse it. A face whose
    /// line is taller than 20 px is drawn smaller. `None` when the face cannot be read or draws nothing.
    pub fn sample(&self, id: fontdb::ID, text: &str, px: f32) -> Option<(u32, u32, Vec<u8>)> {
        if !(px.is_finite() && px > 0.0) {
            return None;
        }
        self.db.with_face_data(id, |data, face_index| draw_sample(data, face_index, text, px)).flatten()
    }
}

/// The characters a sample draws: those of `text` the face maps, or, when it maps none of them (spaces aside), its
/// first six mapped characters above U+0020 in code point order.
fn sample_chars(face: &ttf_parser::Face<'_>, text: &str) -> Vec<char> {
    let mapped = |ch: char| outline::char_to_glyph(face, ch) != 0;
    if text.chars().any(|ch| !ch.is_whitespace() && mapped(ch)) {
        return text.chars().filter(|&ch| ch == ' ' || mapped(ch)).collect();
    }
    let mut smallest: BTreeSet<char> = BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for subtable in cmap.subtables {
            subtable.codepoints(|cp| keep_smallest_mapped(&mut smallest, cp, SAMPLE_FALLBACK_CHARS, mapped));
        }
    }
    smallest.into_iter().collect()
}

/// How many characters a sample draws when the face maps none of the text.
const SAMPLE_FALLBACK_CHARS: usize = 6;

/// Adds code point `cp` to `smallest` when it is a mapped printable character among the `n` smallest seen so far; the
/// set never holds more than `n`. A code point above the current `n` smallest costs no map lookup, so a cmap with a
/// million entries is enumerated without any allocation beyond the `n` characters.
fn keep_smallest_mapped(smallest: &mut BTreeSet<char>, cp: u32, n: usize, mapped: impl Fn(char) -> bool) {
    let Some(ch) = char::from_u32(cp) else { return };
    if ch <= ' ' || ch.is_control() || smallest.contains(&ch) {
        return;
    }
    if smallest.len() >= n && smallest.last().is_some_and(|&max| ch >= max) {
        return;
    }
    if !mapped(ch) {
        return;
    }
    smallest.insert(ch);
    while smallest.len() > n {
        smallest.pop_last();
    }
}

/// `FontCatalog::sample` on the bytes of a font file.
fn draw_sample(bytes: &[u8], face_index: u32, text: &str, px: f32) -> Option<(u32, u32, Vec<u8>)> {
    let face = outline::parse_face(bytes, face_index)?;
    let chars = sample_chars(&face, text);
    if chars.is_empty() {
        return None;
    }
    let upem = f32::from(face.units_per_em().max(1));
    let (mut ascender, mut descender) = (f32::from(face.ascender()), f32::from(face.descender()));
    if ascender - descender <= 0.0 {
        (ascender, descender) = (0.8 * upem, -0.2 * upem);
    }
    let max_h = SAMPLE_MAX_HEIGHT as f32;
    // The line (ascender to descender) fits the height; a taller line draws the face smaller.
    let px = px.min(max_h * upem / (ascender - descender));
    let scale = px / upem;
    let ascent = ascender * scale;
    let height = ((ascender - descender) * scale).ceil().clamp(1.0, max_h) as u32;

    let max_w = SAMPLE_MAX_WIDTH as f32;
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut pen = 1.0_f32;
    for ch in chars {
        if pen >= max_w {
            break;
        }
        let glyph = outline::char_to_glyph(&face, ch);
        let advance = match face.glyph_hor_advance(ttf_parser::GlyphId(glyph)) {
            Some(a) if glyph != 0 => f32::from(a) * scale,
            _ => 0.25 * px,
        };
        if glyph != 0 {
            let shape = outline::glyph_outline(&face, glyph, px);
            contours.extend(shape.contours.into_iter().map(|c| c.into_iter().map(|[x, y]| [x + pen, y]).collect()));
        }
        pen += advance;
    }
    let width = (pen + 1.0).ceil().clamp(1.0, max_w) as u32;
    // Glyph space has its origin on the baseline, y down: the baseline lies `ascent` pixels below the top row.
    let coverage = scan::fill_contours(&contours, [0.0, -ascent], width, height);
    if coverage.data.iter().all(|&c| c == 0) {
        return None;
    }
    Some((coverage.width, coverage.height, coverage.data))
}

/// Weight, italic flag and width of a face, read from its own bytes (FEATURE_TEXT 5.9).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FaceTraits {
    /// OS/2 weight class (400 regular, 700 bold).
    pub weight: u16,
    /// OS/2 style is not normal, or the `post` italic angle is not 0.
    pub italic: bool,
    /// OS/2 width class (5 = normal).
    pub stretch: u16,
}

/// The traits of face `face_index` of a font file, by the rule fontdb uses; `None` when the bytes are no font.
pub fn face_traits(bytes: &[u8], face_index: u32) -> Option<FaceTraits> {
    let face = ttf_parser::Face::parse(bytes, face_index).ok()?;
    Some(FaceTraits {
        weight: face.weight().to_number(),
        italic: face.style() != ttf_parser::Style::Normal || face.italic_angle() != 0.0,
        stretch: face.width().to_number(),
    })
}

/// Family of the bundled default font. Ubuntu-Light.ttf: typographic family (name id 16, preferred by fontdb) "Ubuntu".
pub const DEFAULT_FONT_FAMILY: &str = "Ubuntu";
/// Style of the bundled default font: OS/2 weight 300 => "Light".
pub const DEFAULT_FONT_STYLE: &str = "Light";

/// Bytes of the bundled default font (`epaint_default_fonts::UBUNTU_LIGHT`).
pub fn default_font_bytes() -> &'static [u8] {
    epaint_default_fonts::UBUNTU_LIGHT
}

/// Scale bucket of a raster size: the geometry of a parsed font is optimized for `bucket` pixels per em.
fn scale_bucket(px: f32) -> u32 {
    // The cast saturates and maps NaN to 0.
    (px as u32).next_power_of_two().clamp(32, 1024)
}

fn parse_font(bytes: &[u8], face_index: u32, bucket: u32) -> Result<fontdue::Font, TextError> {
    let settings = fontdue::FontSettings { collection_index: face_index, scale: bucket as f32, load_substitutions: false };
    fontdue::Font::from_bytes(bytes, settings).map_err(|e| TextError::Font(e.to_owned()))
}

/// The bundled default font, parsed once per scale bucket for the whole process (documents and tests share it).
static FALLBACK_FONTS: Mutex<Vec<(u32, Arc<fontdue::Font>)>> = Mutex::new(Vec::new());

/// Parsed fontdue fonts keyed by (blob, face_index, scale bucket). Parsing is slow; never drop this cache per frame.
#[derive(Default)]
pub struct FontCache {
    fonts: HashMap<(BlobId, u32, u32), Arc<fontdue::Font>>,
}

impl FontCache {
    /// The parsed font for `font` at raster size `px` (parsed on first use).
    ///
    /// `Err(MissingFont)` when the blob does not exist, `Err(Font)` when it is not a font or cannot be parsed.
    pub fn get(&mut self, blobs: &BlobStore, font: &FontRef, px: f32) -> Result<Arc<fontdue::Font>, TextError> {
        let bucket = scale_bucket(px);
        let key = (font.blob, font.face_index, bucket);
        if let Some(parsed) = self.fonts.get(&key) {
            return Ok(Arc::clone(parsed));
        }
        let blob = blobs.get(font.blob).ok_or(TextError::MissingFont)?;
        if blob.kind != BlobKind::Font {
            return Err(TextError::Font("the referenced blob is not a font file".to_owned()));
        }
        // The bundled font registered as a blob (`Document::default_font`) shares the process-wide parse.
        let parsed = if font.face_index == 0 && blob.bytes[..] == *default_font_bytes() {
            self.fallback(px)
        } else {
            Arc::new(parse_font(&blob.bytes, font.face_index, bucket)?)
        };
        self.fonts.insert(key, Arc::clone(&parsed));
        Ok(parsed)
    }

    /// The bundled default font at raster size `px`.
    pub fn fallback(&mut self, px: f32) -> Arc<fontdue::Font> {
        let bucket = scale_bucket(px);
        let mut shared = FALLBACK_FONTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, parsed)) = shared.iter().find(|(b, _)| *b == bucket) {
            return Arc::clone(parsed);
        }
        // Invariant: the bundled font is a compile-time constant that fontdue parses.
        let parsed = Arc::new(parse_font(default_font_bytes(), 0, bucket).expect("the bundled default font parses"));
        shared.push((bucket, Arc::clone(&parsed)));
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_traits_match_the_catalog() {
        let catalog = FontCatalog::bundled();
        for family in catalog.families() {
            for face in catalog.faces(family) {
                let (bytes, index) = catalog.face_bytes(face.id).unwrap();
                let traits = face_traits(&bytes, index).expect("a bundled font parses");
                assert_eq!(traits.weight, face.weight, "{family} {}", face.style);
                assert_eq!(traits.italic, face.italic, "{family} {}", face.style);
                assert_eq!(traits.stretch, face.stretch, "{family} {}", face.style);
            }
        }
        assert_eq!(face_traits(default_font_bytes(), 0).map(|t| (t.weight, t.italic)), Some((300, false)));
        assert_eq!(face_traits(b"not a font", 0), None);
    }

    #[test]
    fn style_name_table() {
        use fontdb::Style::{Italic, Normal, Oblique};
        let cases = [
            (100, Normal, "Thin"),
            (200, Normal, "ExtraLight"),
            (300, Normal, "Light"),
            (400, Normal, "Regular"),
            (500, Normal, "Medium"),
            (600, Normal, "SemiBold"),
            (700, Normal, "Bold"),
            (800, Normal, "ExtraBold"),
            (900, Normal, "Black"),
            (350, Normal, "W350"),
            (400, Italic, "Italic"),
            (400, Oblique, "Oblique"),
            (700, Italic, "Bold Italic"),
            (300, Oblique, "Light Oblique"),
            (350, Italic, "W350 Italic"),
            (0, Normal, "W0"),
        ];
        for (weight, style, want) in cases {
            assert_eq!(style_name(weight, style), want, "{weight} {style:?}");
        }
    }

    #[test]
    fn bundled_catalog_lists_ubuntu_and_hack() {
        let catalog = FontCatalog::bundled();
        assert_eq!(catalog.families(), ["Hack".to_owned(), "Ubuntu".to_owned()]);

        let light = catalog.find(DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE).expect("Ubuntu Light is bundled");
        assert_eq!((light.family.as_str(), light.style.as_str(), light.weight), ("Ubuntu", "Light", 300));
        assert!(!light.italic && !light.monospaced);
        assert_eq!(light.face_index, 0);
        // Family lookup is case-insensitive, the style string is exact.
        assert_eq!(catalog.find("ubuntu", "Light").map(|f| f.id), Some(light.id));
        assert!(catalog.find("Ubuntu", "light").is_none());
        assert!(catalog.find("Ubuntu", "Regular").is_none());
        assert!(catalog.find("No Such Family", "Regular").is_none());

        let hack = catalog.faces("Hack");
        assert_eq!(hack.len(), 1);
        assert_eq!((hack[0].style.as_str(), hack[0].weight, hack[0].monospaced), ("Regular", 400, true));
        assert!(catalog.faces("Nothing").is_empty());
    }

    /// TEXT 8.1 `equal_style_names_stay_reachable`. Round 4 changes the rule on purpose (TEXT 5.13): a later face with
    /// a style name that is taken is no longer dropped but named "<style> (<PostScript name>)". This replaces the
    /// round-3 test `faces_are_sorted_and_duplicate_styles_dropped`.
    #[test]
    fn equal_style_names_stay_reachable() {
        let mut catalog = FontCatalog::bundled();
        catalog.db.load_font_source(fontdb::Source::Binary(Arc::new(epaint_default_fonts::UBUNTU_LIGHT)));
        catalog.db.load_font_source(fontdb::Source::Binary(Arc::new(epaint_default_fonts::UBUNTU_LIGHT)));
        catalog.refresh_families();
        assert_eq!(catalog.families().len(), 2);
        let faces = catalog.faces("Ubuntu");
        let styles: Vec<&str> = faces.iter().map(|f| f.style.as_str()).collect();
        assert_eq!(styles, ["Light", "Light (Ubuntu-Light)", "Light (Ubuntu-Light) 2"]);
        assert!(faces.windows(2).all(|w| (w[0].weight, w[0].italic) <= (w[1].weight, w[1].italic)));
        let all: Vec<fontdb::ID> = catalog.db.faces().filter(|f| family_of(f) == Some("Ubuntu")).map(|f| f.id).collect();
        assert_eq!(faces.iter().map(|f| f.id).collect::<Vec<_>>(), all, "load order kept, nothing dropped");
        // `find` reaches each face; the first keeps the plain name, so FontRef.style of old projects resolves.
        for face in &faces {
            assert_eq!(catalog.find("ubuntu", &face.style).map(|f| f.id), Some(face.id), "{}", face.style);
        }
        assert_eq!(catalog.find("Ubuntu", "Light").map(|f| f.id), Some(all[0]));
        assert_eq!(faces[1].post_script_name, "Ubuntu-Light");
        // A sort of the faces is stable: the bundled catalog is unchanged.
        let bundled = FontCatalog::bundled().faces("Ubuntu");
        assert_eq!(bundled.len(), 1);
        assert_eq!(bundled[0].style, "Light");
    }

    /// TEXT 8.1 `families_sorted_without_case`.
    #[test]
    fn families_sorted_without_case() {
        let mut names: Vec<String> =
            ["b", "Hack", "A", "segoe UI", "a", "B", "hack", "Segoe Print", "Hack", "ubuntu", "Arial"].map(String::from).to_vec();
        sort_families(&mut names);
        // Names that differ only in case are one family (`faces` matches without case): the first spelling is kept.
        assert_eq!(names, ["A", "Arial", "B", "Hack", "Segoe Print", "segoe UI", "ubuntu"]);
        // Byte order would put every capital first.
        let mut mixed: Vec<String> = ["Zeta", "alpha", "Beta"].map(String::from).to_vec();
        let mut bytes = mixed.clone();
        bytes.sort();
        sort_families(&mut mixed);
        assert_eq!(mixed, ["alpha", "Beta", "Zeta"]);
        assert_ne!(bytes, mixed);
        assert_eq!(FontCatalog::bundled().families(), ["Hack".to_owned(), "Ubuntu".to_owned()]);
    }

    /// TEXT 8.1 `face_bytes_shares_one_allocation_per_file`.
    #[test]
    fn face_bytes_shares_one_allocation_per_file() {
        // Two faces of one source: the same allocation of loaded bytes behind two faces.
        let mut catalog = FontCatalog::bundled();
        let source: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(epaint_default_fonts::HACK_REGULAR);
        let a = catalog.db.load_font_source(fontdb::Source::Binary(Arc::clone(&source)))[0];
        let b = catalog.db.load_font_source(fontdb::Source::Binary(source))[0];
        assert_ne!(a, b);
        let (first, _) = catalog.face_bytes(a).unwrap();
        let (second, _) = catalog.face_bytes(b).unwrap();
        assert!(Arc::ptr_eq(&first, &second), "two faces of one source share the allocation while it is held");
        assert_eq!(&first[..], epaint_default_fonts::HACK_REGULAR);
        // Another source is another allocation.
        let ubuntu = catalog.find("Ubuntu", "Light").unwrap();
        let (other, _) = catalog.face_bytes(ubuntu.id).unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
        // While held, a document insert of the second copy is the fast path: same blob.
        let mut blobs = BlobStore::default();
        let id = blobs.insert(BlobKind::Font, "Hack.ttf", Arc::clone(&first));
        assert_eq!(blobs.insert(BlobKind::Font, "Hack.ttf", second), id);
        let released = Arc::downgrade(&first);
        drop(first);
        drop(blobs);
        // Nobody holds it: the allocation is freed (the catalog keeps only a weak entry), and the next call reads
        // the source again into a fresh allocation, which becomes the shared one.
        assert!(released.upgrade().is_none(), "the catalog does not keep the bytes alive");
        let (third, _) = catalog.face_bytes(a).unwrap();
        assert_eq!(&third[..], epaint_default_fonts::HACK_REGULAR);
        let key = source_key(&catalog.db.face(a).unwrap().source);
        let stored = catalog.shared.lock().unwrap().get(&key).and_then(Weak::upgrade).expect("the new copy is shared");
        assert!(Arc::ptr_eq(&stored, &third));
        assert_eq!(Arc::strong_count(&third), 2, "held by the test and the upgrade above only");
        drop(stored);
        let (fourth, _) = catalog.face_bytes(b).unwrap();
        assert!(Arc::ptr_eq(&third, &fourth));

        // A font file loaded from disk: every face of the file shares one allocation.
        let dir = test_dir("face_bytes_file");
        let path = dir.join("Ubuntu-Light.ttf");
        std::fs::write(&path, epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        let mut catalog = FontCatalog::bundled();
        catalog.load_font_file(&path).unwrap();
        let from_file: Vec<fontdb::ID> = catalog
            .db
            .faces()
            .filter(|f| matches!(&f.source, fontdb::Source::File(p) if *p == path))
            .map(|f| f.id)
            .collect();
        assert_eq!(from_file.len(), 1);
        let (x, _) = catalog.face_bytes(from_file[0]).unwrap();
        let (y, _) = catalog.face_bytes(from_file[0]).unwrap();
        assert!(Arc::ptr_eq(&x, &y));
        assert_eq!(&x[..], epaint_default_fonts::UBUNTU_LIGHT);

        // A collection file of this machine, when there is one: two of its faces, one allocation.
        let Some((ttc, ids)) = machine_collection() else {
            eprintln!("SKIP face_bytes_shares_one_allocation_per_file (collection part): no .ttc with two faces");
            return;
        };
        let (p, i0) = ttc.face_bytes(ids[0]).unwrap();
        let (q, i1) = ttc.face_bytes(ids[1]).unwrap();
        assert!(Arc::ptr_eq(&p, &q));
        assert_ne!(i0, i1);
    }

    /// TEXT 8.1 `file_sizes_are_read_once`: the size of a face comes from the map filled at load; `faces()` touches
    /// no file (the file is gone and the size is still known).
    #[test]
    fn file_sizes_are_read_once() {
        let dir = test_dir("file_sizes");
        let path = dir.join("Hack-Regular.ttf");
        std::fs::write(&path, epaint_default_fonts::HACK_REGULAR).unwrap();
        let mut catalog = FontCatalog::bundled();
        catalog.load_font_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let len = epaint_default_fonts::HACK_REGULAR.len() as u64;
        for _ in 0..100 {
            let faces = catalog.faces("Hack");
            // Bundled Hack (loaded bytes) and the file copy, both with their sizes.
            assert_eq!(faces.len(), 2);
            assert!(faces.iter().all(|f| f.file_bytes == len), "{faces:?}");
        }
        assert_eq!(catalog.faces("Ubuntu")[0].file_bytes, default_font_bytes().len() as u64);
        assert!(catalog.find("Hack", "Regular (Hack-Regular)").is_some());
    }

    /// Files whose extension fontdb skips (mixed case) are loaded by the extra pass; their sizes are recorded.
    #[test]
    fn mixed_case_extensions_are_loaded() {
        let dir = test_dir("mixed_case");
        std::fs::write(dir.join("Ubuntu-Light.Ttf"), epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        std::fs::write(dir.join("Hack-Regular.ttf"), epaint_default_fonts::HACK_REGULAR).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a font").unwrap();
        std::fs::write(dir.join("broken.oTf"), b"not a font either").unwrap();
        let mut fontdb_only = fontdb::Database::new();
        fontdb_only.load_fonts_dir(&dir);
        assert_eq!(fontdb_only.len(), 1, "fontdb skips the mixed-case file");

        let mut catalog = FontCatalog { db: fontdb::Database::new(), ..FontCatalog::bundled() };
        catalog.db.load_fonts_dir(&dir);
        catalog.scan_dir(&dir, 0);
        catalog.read_missing_sizes();
        catalog.refresh_families();
        assert_eq!(catalog.db.len(), 2);
        assert_eq!(catalog.families(), ["Hack".to_owned(), "Ubuntu".to_owned()]);
        assert_eq!(catalog.faces("Ubuntu")[0].file_bytes, default_font_bytes().len() as u64);
        assert_eq!(catalog.faces("Hack")[0].file_bytes, epaint_default_fonts::HACK_REGULAR.len() as u64);
    }

    /// TEXT 8.1 `a_sample_has_ink`: both bundled fonts at most 128 x 20 px with at least 40 inked pixels.
    #[test]
    fn a_sample_has_ink() {
        let catalog = FontCatalog::bundled();
        if !painter_ready() {
            eprintln!("SKIP a_sample_has_ink: outline.rs / scan.rs of T1 are still stubs");
            return;
        }
        for family in catalog.families() {
            let face = &catalog.faces(family)[0];
            let (w, h, bytes) = catalog.sample(face.id, SAMPLE_TEXT, SAMPLE_PX).expect("a bundled font draws");
            assert!(w <= SAMPLE_MAX_WIDTH && h <= SAMPLE_MAX_HEIGHT && w > 0 && h > 0, "{family}: {w}x{h}");
            assert_eq!(bytes.len(), (w * h) as usize);
            let inked = bytes.iter().filter(|&&c| c > 0).count();
            assert!(inked >= 40, "{family}: {inked} inked pixels");
            // The ink is the text: wider than tall, and it reaches below the middle (the baseline sits low).
            assert!(w >= 40, "{family}: {w} px wide");
            let row_ink = |y: u32| bytes[(y * w) as usize..((y + 1) * w) as usize].iter().filter(|&&c| c > 0).count();
            assert!((h / 2..h).any(|y| row_ink(y) > 0), "{family}: no ink below the middle");
            assert!((0..h / 2).any(|y| row_ink(y) > 0), "{family}: no ink above the middle");
            assert!(row_ink(0) < w as usize, "{family}: the top row is fully inked");
            eprintln!("a_sample_has_ink: {family}: {w}x{h}, {inked} inked pixels");
        }
        let ubuntu = catalog.find("Ubuntu", "Light").unwrap();
        // A larger size is drawn smaller to fit 20 px; a long text is cut at 128 px.
        let (w, h, _) = catalog.sample(ubuntu.id, "The quick brown fox jumps over the lazy dog", 40.0).unwrap();
        assert_eq!((w, h), (SAMPLE_MAX_WIDTH, SAMPLE_MAX_HEIGHT));
        // Characters the face does not map: its first six mapped characters.
        let (_, _, fallback) = catalog.sample(ubuntu.id, "\u{E000}\u{E001}", SAMPLE_PX).unwrap();
        assert!(fallback.iter().filter(|&&c| c > 0).count() >= 40);
        // Bad input.
        assert!(catalog.sample(ubuntu.id, SAMPLE_TEXT, f32::NAN).is_none());
        assert!(catalog.sample(ubuntu.id, SAMPLE_TEXT, 0.0).is_none());
        assert!(catalog.sample(fontdb::ID::dummy(), SAMPLE_TEXT, SAMPLE_PX).is_none());
        assert!(catalog.sample(ubuntu.id, "   ", SAMPLE_PX).is_some(), "only spaces: the first mapped characters");
    }

    /// The characters of a sample: mapped characters of the text, else the first six mapped. Needs T1's
    /// `outline::char_to_glyph` (skipped while it is the scaffold stub).
    #[test]
    fn sample_chars_follow_the_painters_map() {
        if !painter_ready() {
            eprintln!("SKIP sample_chars_follow_the_painters_map: char_to_glyph of T1 is still a stub");
            return;
        }
        let face = outline::parse_face(default_font_bytes(), 0).unwrap();
        assert_eq!(sample_chars(&face, SAMPLE_TEXT).into_iter().collect::<String>(), SAMPLE_TEXT);
        // An unmapped character is left out; one mapped character is enough to draw the text.
        assert_eq!(sample_chars(&face, "A\u{E000}b").into_iter().collect::<String>(), "Ab");
        let first = sample_chars(&face, "\u{E000}");
        assert_eq!(first.len(), 6);
        assert_eq!(first[0], '!', "Ubuntu maps U+0021 first above the space");
        assert!(first.windows(2).all(|w| w[0] < w[1]));
    }

    /// The fonts of this machine (Windows font folders; skipped when there are none): the hard asserts of the
    /// `text_fonts` scenario (at least 50 families, Arial among them) and the font side of R4-12: Arial has a real
    /// "Bold" face that `face_traits` reads as bold, the bundled Ubuntu has no bold face.
    #[test]
    fn machine_fonts_have_arial_with_a_real_bold_face() {
        if !machine_fonts_present() {
            eprintln!("SKIP machine_fonts_have_arial_with_a_real_bold_face: no Windows font folder");
            return;
        }
        let mut catalog = FontCatalog::bundled();
        catalog.load_system();
        let families = catalog.families();
        eprintln!("machine catalog: {} families", families.len());
        assert!(families.len() >= 50, "{} families", families.len());
        assert!(families.iter().any(|f| f == "Arial"), "Arial is installed");
        let arial = catalog.faces("Arial");
        let bold = catalog.find("Arial", "Bold").expect("Arial Bold");
        assert_eq!(bold.weight, 700);
        let (bytes, index) = catalog.face_bytes(bold.id).unwrap();
        let traits = face_traits(&bytes, index).unwrap();
        assert!(traits.weight == 700 && traits.weight == bold.weight && !traits.italic, "{traits:?}");
        assert!(arial.iter().any(|f| f.style == "Regular" && f.weight == 400));
        assert!(arial.iter().any(|f| f.italic));
        assert!(arial.iter().all(|f| f.file_bytes > 0), "every installed face has its file size");
        // The bundled Ubuntu (Light only) has no bold face, unless Ubuntu is installed on this machine.
        let ubuntu = catalog.faces("Ubuntu");
        if ubuntu.iter().all(|f| matches!(catalog.db.face(f.id).map(|i| &i.source), Some(fontdb::Source::Binary(_)))) {
            assert!(ubuntu.iter().all(|f| f.weight < 600));
        }
        // Families sorted without case.
        assert!(families.windows(2).all(|w| w[0].to_lowercase() <= w[1].to_lowercase()));
        // Samples of every family (the face the picker would show first).
        if painter_ready() {
            let mut drawn = 0;
            for family in families {
                if let Some(face) = catalog.faces(family).first()
                    && let Some((w, h, bytes)) = catalog.sample(face.id, SAMPLE_TEXT, SAMPLE_PX)
                {
                    assert!(w <= SAMPLE_MAX_WIDTH && h <= SAMPLE_MAX_HEIGHT && bytes.len() == (w * h) as usize);
                    drawn += 1;
                }
            }
            eprintln!("machine catalog: {drawn} of {} families drew a sample", families.len());
            assert!(drawn * 10 >= families.len() * 9, "{drawn} of {}", families.len());
        }
    }

    /// Times the font scan at start (round 3: fontdb's scan alone; round 4: `load_system`) and the sample job
    /// (one sample per family), twice each, back to back. Run in release:
    /// `cargo test -p wte_core --release --lib -- --ignored font_timings --nocapture`.
    #[test]
    #[ignore]
    fn font_timings() {
        use std::time::Instant;
        if !machine_fonts_present() {
            eprintln!("SKIP font_timings: no Windows font folder");
            return;
        }
        for round in 1..=2 {
            let t = Instant::now();
            let mut round3 = FontCatalog::bundled();
            round3.db.load_system_fonts();
            round3.refresh_families();
            let scan3 = t.elapsed();
            let t = Instant::now();
            let mut round4 = FontCatalog::bundled();
            round4.load_system();
            let scan4 = t.elapsed();
            let t = Instant::now();
            let mut drawn = 0;
            for family in round4.families() {
                if let Some(face) = round4.faces(family).first()
                    && round4.sample(face.id, SAMPLE_TEXT, SAMPLE_PX).is_some()
                {
                    drawn += 1;
                }
            }
            let job = t.elapsed();
            eprintln!(
                "font_timings run {round}: scan round-3 rule {:.1} ms ({} faces), load_system {:.1} ms ({} faces), \
                 difference {:+.1} ms; sample job {:.1} ms for {} families ({drawn} drawn)",
                scan3.as_secs_f64() * 1e3,
                round3.db.len(),
                scan4.as_secs_f64() * 1e3,
                round4.db.len(),
                (scan4.as_secs_f64() - scan3.as_secs_f64()) * 1e3,
                job.as_secs_f64() * 1e3,
                round4.families().len()
            );
        }
    }

    fn machine_fonts_present() -> bool {
        system_font_dirs().first().is_some_and(|d| d.is_dir())
    }

    /// A catalog of this machine and two faces of one collection file, when there is one.
    fn machine_collection() -> Option<(FontCatalog, Vec<fontdb::ID>)> {
        if !machine_fonts_present() {
            return None;
        }
        let mut catalog = FontCatalog::bundled();
        catalog.load_system();
        let mut by_file: HashMap<PathBuf, Vec<fontdb::ID>> = HashMap::new();
        for face in catalog.db.faces() {
            if let fontdb::Source::File(path) = &face.source {
                by_file.entry(path.clone()).or_default().push(face.id);
            }
        }
        // The smallest collection, so the test reads little.
        let (_, ids) = by_file
            .into_iter()
            .filter(|(_, ids)| ids.len() >= 2)
            .min_by_key(|(path, _)| catalog.file_sizes.get(path).copied().unwrap_or(u64::MAX))?;
        Some((catalog, ids))
    }

    /// True when T1's outlines and rasterizer are in this copy. False only while `outline.rs` or `scan.rs` still carry
    /// the scaffold's "stub until T1" marker; once T1's files are in, a painter that draws nothing is a failure (not
    /// a skip), so a regression of `char_to_glyph`, `glyph_outline` or `fill_contours` cannot pass as a SKIP.
    fn painter_ready() -> bool {
        const STUB: &str = "stub until T1";
        if include_str!("outline.rs").contains(STUB) || include_str!("scan.rs").contains(STUB) {
            return false;
        }
        let face = outline::parse_face(default_font_bytes(), 0).expect("outline::parse_face reads the bundled font");
        let glyph = outline::char_to_glyph(&face, 'A');
        assert_ne!(glyph, 0, "outline::char_to_glyph maps 'A' of the bundled font");
        assert!(!outline::glyph_outline(&face, glyph, 16.0).is_empty(), "outline::glyph_outline draws 'A'");
        let square = vec![vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]]];
        assert!(
            scan::fill_contours(&square, [0.0, 0.0], 4, 4).data.iter().any(|&c| c > 0),
            "scan::fill_contours covers a square"
        );
        true
    }

    /// A load whose size read fails (the file vanished after fontdb read it) still lists the family; the size is
    /// unknown (0). This is the path `load_font_file` takes after a successful fontdb load.
    #[test]
    fn a_failed_size_read_keeps_the_loaded_family() {
        let dir = test_dir("size_read_fails");
        let path = dir.join("Hack-Regular.ttf");
        std::fs::write(&path, epaint_default_fonts::HACK_REGULAR).unwrap();
        let mut catalog = FontCatalog { db: fontdb::Database::new(), ..FontCatalog::bundled() };
        catalog.refresh_families();
        assert!(catalog.families().is_empty());
        catalog.db.load_font_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let size = std::fs::metadata(&path).map(|m| m.len());
        assert!(size.is_err());
        catalog.file_loaded(&path, size);
        assert_eq!(catalog.families(), ["Hack".to_owned()]);
        assert_eq!(catalog.faces("Hack")[0].file_bytes, 0);
        // A real load of a file that exists records its size.
        std::fs::write(&path, epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        catalog.load_font_file(&path).unwrap();
        assert_eq!(catalog.families(), ["Hack".to_owned(), "Ubuntu".to_owned()]);
        assert_eq!(catalog.faces("Ubuntu")[0].file_bytes, default_font_bytes().len() as u64);
    }

    /// The fallback characters of a sample come from a bounded set: a cmap of a million code points costs no
    /// allocation beyond the six characters, and a map lookup only for code points below the current six.
    #[test]
    fn sample_fallback_keeps_only_the_smallest_mapped() {
        use std::cell::Cell;
        let lookups = Cell::new(0usize);
        let mapped = |ch: char| {
            lookups.set(lookups.get() + 1);
            !(ch as u32).is_multiple_of(3)
        };
        let mut smallest = BTreeSet::new();
        // Ascending (format 12 order) over a last-resort-sized range, then a few out of order and duplicates.
        for cp in 0..0x11_0000u32 {
            keep_smallest_mapped(&mut smallest, cp, 6, mapped);
            assert!(smallest.len() <= 6);
        }
        assert!(lookups.get() < 100, "{} map lookups", lookups.get());
        for cp in [0x22, 0x22, 0x10, 0x7F, 0xD800, 0x110000, u32::MAX] {
            keep_smallest_mapped(&mut smallest, cp, 6, mapped);
        }
        // Above U+0020, not U+0021 (33 % 3 == 0): '"' # % & ( )
        assert_eq!(smallest.into_iter().collect::<String>(), "\"#%&()");
        // Descending order gives the same set.
        let mut down = BTreeSet::new();
        for cp in (0..0x200u32).rev() {
            keep_smallest_mapped(&mut down, cp, 6, mapped);
        }
        assert_eq!(down.into_iter().collect::<String>(), "\"#%&()");
    }

    /// `face_bytes` copies a file without holding the catalog lock; callers racing for the same file still end up
    /// with one allocation.
    #[test]
    fn face_bytes_racing_threads_share_one_allocation() {
        let dir = test_dir("face_bytes_race");
        let path = dir.join("Ubuntu-Light.ttf");
        std::fs::write(&path, epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        let mut catalog = FontCatalog::bundled();
        catalog.load_font_file(&path).unwrap();
        let id = catalog
            .db
            .faces()
            .find(|f| matches!(&f.source, fontdb::Source::File(p) if *p == path))
            .map(|f| f.id)
            .unwrap();
        let barrier = std::sync::Barrier::new(8);
        let results: Vec<Arc<[u8]>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        barrier.wait();
                        catalog.face_bytes(id).unwrap().0
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(results.windows(2).all(|w| Arc::ptr_eq(&w[0], &w[1])), "one allocation for all callers");
        assert_eq!(&results[0][..], epaint_default_fonts::UBUNTU_LIGHT);
    }

    /// An empty folder for one test, below this copy's `target` folder (nothing is written to C:).
    fn test_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test_tmp/font_catalog").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn face_bytes_round_trip_into_fontdue() {
        let catalog = FontCatalog::bundled();
        let face = catalog.find("Ubuntu", "Light").unwrap();
        let (bytes, index) = catalog.face_bytes(face.id).unwrap();
        assert_eq!(index, 0);
        assert_eq!(&bytes[..], default_font_bytes());
        let font = parse_font(&bytes, index, 64).unwrap();
        assert!(font.has_glyph('A') && font.has_glyph('g'));
        assert!(font.metrics('A', 64.0).advance_width > 10.0);
        assert!(catalog.face_bytes(fontdb::ID::dummy()).is_none());
    }

    #[test]
    fn font_cache_keys_buckets_and_reports_errors() {
        assert_eq!(scale_bucket(8.0), 32);
        assert_eq!(scale_bucket(32.0), 32);
        assert_eq!(scale_bucket(33.0), 64);
        assert_eq!(scale_bucket(128.0), 128);
        assert_eq!(scale_bucket(129.0), 256);
        assert_eq!(scale_bucket(5000.0), 1024);
        assert_eq!(scale_bucket(f32::NAN), 32);
        assert_eq!(scale_bucket(-3.0), 32);

        let mut blobs = BlobStore::default();
        let hack = blobs.insert(BlobKind::Font, "Hack-Regular.ttf", Arc::from(epaint_default_fonts::HACK_REGULAR));
        let ubuntu = blobs.insert(BlobKind::Font, "Ubuntu-Light.ttf", Arc::from(default_font_bytes()));
        let garbage = blobs.insert(BlobKind::Font, "broken.ttf", Arc::from(&b"not a font at all"[..]));
        let image = blobs.insert(BlobKind::Image, "x.png", Arc::from(epaint_default_fonts::HACK_REGULAR));
        let font_ref = |blob| FontRef { blob, face_index: 0, family: "f".into(), style: "s".into() };

        let mut cache = FontCache::default();
        let a = cache.get(&blobs, &font_ref(hack), 100.0).unwrap();
        let b = cache.get(&blobs, &font_ref(hack), 128.0).unwrap();
        let c = cache.get(&blobs, &font_ref(hack), 129.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "same bucket, same parsed font");
        assert!(!Arc::ptr_eq(&a, &c), "another bucket is another parse");
        assert!(a.has_glyph('W'));

        // The bundled font as a blob is the process-wide fallback parse.
        let d = cache.get(&blobs, &font_ref(ubuntu), 64.0).unwrap();
        assert!(Arc::ptr_eq(&d, &cache.fallback(64.0)));
        assert!(Arc::ptr_eq(&cache.fallback(40.0), &FontCache::default().fallback(64.0)));

        assert!(matches!(cache.get(&blobs, &font_ref(BlobId(999)), 64.0), Err(TextError::MissingFont)));
        assert!(matches!(cache.get(&blobs, &font_ref(garbage), 64.0), Err(TextError::Font(_))));
        assert!(matches!(cache.get(&blobs, &font_ref(image), 64.0), Err(TextError::Font(_))));
        // A face index the file does not have.
        let bad_face = FontRef { blob: hack, face_index: 7, family: "f".into(), style: "s".into() };
        assert!(matches!(cache.get(&blobs, &bad_face, 64.0), Err(TextError::Font(_))));
    }

    #[test]
    fn catalog_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FontCatalog>();
        fn assert_send<T: Send>() {}
        assert_send::<FontCache>();
    }
}
