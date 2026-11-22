// Owner: core-import (C5). Asset resolvers and the captured source bundle of ARCHITECTURE.md 5.21: where the
// importers get MTL files, textures and glTF buffers from (disk on first import, memory on every later one).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::ResolveError;
use crate::import::uri::percent_decode;

/// Source of the files a model references (MTL, textures, glTF buffers).
pub trait AssetResolver {
    /// `reference` is the string exactly as written in the model file (mtllib, map_Kd, glTF uri).
    fn read(&self, reference: &str) -> Result<Arc<[u8]>, ResolveError>;
}

/// Resolves references relative to a directory on disk (lookup order: section 10, importer notes).
pub struct DiskResolver {
    pub base_dir: PathBuf,
}

/// Resolves references from memory; keys are `normalize_ref`ed.
#[derive(Default)]
pub struct MemoryResolver {
    files: BTreeMap<String, Arc<[u8]>>,
}

impl MemoryResolver {
    /// Adds a file under `normalize_ref(reference)`.
    pub fn insert(&mut self, reference: &str, bytes: Arc<[u8]>) {
        self.files.insert(normalize_ref(reference), bytes);
    }

    /// Resolver over the sidecars of a bundle (`extras` are ignored: the importer never reads them).
    ///
    /// Every sidecar is reachable under its `key` as stored and under `normalize_ref(key)`, so a bundle whose keys
    /// were not normalized by its author still resolves. The first sidecar wins when two share a key.
    pub fn from_bundle(bundle: &SourceBundle) -> MemoryResolver {
        let mut resolver = MemoryResolver::default();
        for file in &bundle.sidecars {
            resolver.files.entry(file.key.clone()).or_insert_with(|| file.bytes.clone());
            resolver.files.entry(normalize_ref(&file.key)).or_insert_with(|| file.bytes.clone());
        }
        resolver
    }
}

/// Wraps a resolver and records every file successfully read (first-import capture for R7).
pub struct RecordingResolver<R: AssetResolver> {
    inner: R,
    /// Files read so far, in first-read order.
    captured: RefCell<Vec<SourceFile>>,
    /// `SourceFile::key` -> index into `captured`.
    index: RefCell<BTreeMap<String, usize>>,
    /// References that could not be resolved, in first-failure order, without duplicates.
    missing: RefCell<Vec<String>>,
}

impl<R: AssetResolver> RecordingResolver<R> {
    /// Wraps `inner`.
    pub fn new(inner: R) -> Self {
        RecordingResolver {
            inner,
            captured: RefCell::new(Vec::new()),
            index: RefCell::new(BTreeMap::new()),
            missing: RefCell::new(Vec::new()),
        }
    }

    /// The files read so far (in first-read order) and the references that could not be resolved.
    ///
    /// A captured file has `key = normalize_ref(reference)` and `original_name` = the file-name part of the
    /// percent-decoded reference. A file read through several spellings of one key is captured once.
    pub fn into_captured(self) -> (Vec<SourceFile>, Vec<String>) {
        (self.captured.into_inner(), self.missing.into_inner())
    }
}

/// The part of `path` after its last `/` or `\`.
pub(crate) fn file_name_of(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// File name of `path` without its extension ("textures/Wood.A.png" -> "Wood.A"; ".hidden" stays ".hidden").
pub(crate) fn file_stem_of(path: &str) -> &str {
    let name = file_name_of(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// Reads `path` if it is a file. `Ok(None)` when there is no such file; other failures are returned, also a file
/// larger than a project can store (`MAX_SOURCE_FILE`).
fn read_file(path: &Path) -> std::io::Result<Option<Arc<[u8]>>> {
    if !path.is_file() {
        return Ok(None);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > crate::import::MAX_SOURCE_FILE) {
        return Err(std::io::Error::other(format!(
            "{} is larger than the {} bytes a project can store",
            path.display(),
            crate::import::MAX_SOURCE_FILE
        )));
    }
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(Arc::from(bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Follows the `/`-separated relative path `relative` below `dir`, matching every component without regard to
/// case (exact matches are preferred, then the alphabetically first candidate, so the result is deterministic).
fn find_case_insensitive(dir: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = dir.to_path_buf();
    for component in relative.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                current.push("..");
                continue;
            }
            _ => {}
        }
        let exact = current.join(component);
        if exact.exists() {
            current = exact;
            continue;
        }
        let wanted = component.to_lowercase();
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(&current)
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().to_lowercase() == wanted)
            .map(|entry| entry.path())
            .collect();
        candidates.sort();
        current = candidates.into_iter().next()?;
    }
    Some(current)
}

/// True for references that name a location outside the model's folder: drive letters, UNC and rooted paths.
fn is_absolute_ref(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    reference.starts_with('/')
        || reference.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

impl AssetResolver for DiskResolver {
    /// Lookup order, for the reference as written and then for its percent-decoded form:
    /// 1. `base_dir.join(reference)` (an absolute reference is read from where it points);
    /// 2. the same with `\` and `/` swapped;
    /// 3. a case-insensitive match of every path component below `base_dir`;
    /// 4. the bare file name in `base_dir`, then in `base_dir/textures/` (absolute and dangling references).
    fn read(&self, reference: &str) -> Result<Arc<[u8]>, ResolveError> {
        let trimmed = reference.trim();
        if trimmed.is_empty() {
            return Err(ResolveError::NotFound(reference.to_owned()));
        }
        let mut spellings = vec![trimmed.to_owned()];
        let decoded = percent_decode(trimmed);
        if decoded != trimmed {
            spellings.push(decoded);
        }

        let mut first_error: Option<std::io::Error> = None;
        let mut attempt = |path: &Path| -> Option<Arc<[u8]>> {
            match read_file(path) {
                Ok(found) => found,
                Err(e) => {
                    first_error.get_or_insert(e);
                    None
                }
            }
        };

        for spelling in &spellings {
            let forward = spelling.replace('\\', "/");
            let backward = spelling.replace('/', "\\");
            for candidate in [spelling.as_str(), forward.as_str(), backward.as_str()] {
                if let Some(bytes) = attempt(&self.base_dir.join(candidate)) {
                    return Ok(bytes);
                }
            }
            if !is_absolute_ref(spelling)
                && let Some(bytes) = find_case_insensitive(&self.base_dir, &forward).and_then(|p| attempt(&p))
            {
                return Ok(bytes);
            }
        }
        for spelling in &spellings {
            let name = file_name_of(spelling);
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            for relative in [name.to_owned(), format!("textures/{name}")] {
                if let Some(bytes) = find_case_insensitive(&self.base_dir, &relative).and_then(|p| attempt(&p)) {
                    return Ok(bytes);
                }
            }
        }
        match first_error {
            Some(e) => Err(ResolveError::Io(reference.to_owned(), e)),
            None => Err(ResolveError::NotFound(reference.to_owned())),
        }
    }
}

impl AssetResolver for MemoryResolver {
    fn read(&self, reference: &str) -> Result<Arc<[u8]>, ResolveError> {
        self.files
            .get(&normalize_ref(reference))
            .cloned()
            .ok_or_else(|| ResolveError::NotFound(reference.to_owned()))
    }
}

impl<R: AssetResolver> AssetResolver for RecordingResolver<R> {
    fn read(&self, reference: &str) -> Result<Arc<[u8]>, ResolveError> {
        let key = normalize_ref(reference);
        if let Some(&i) = self.index.borrow().get(&key) {
            return Ok(self.captured.borrow()[i].bytes.clone());
        }
        match self.inner.read(reference) {
            Ok(bytes) => {
                let mut captured = self.captured.borrow_mut();
                self.index.borrow_mut().insert(key.clone(), captured.len());
                captured.push(SourceFile {
                    key,
                    original_name: file_name_of(&percent_decode(reference.trim())).to_owned(),
                    bytes: bytes.clone(),
                });
                Ok(bytes)
            }
            Err(e) => {
                let mut missing = self.missing.borrow_mut();
                if !missing.iter().any(|m| m == reference) {
                    missing.push(reference.to_owned());
                }
                Err(e)
            }
        }
    }
}

/// '\\' -> '/', percent-decode, strip leading "./", ASCII-lowercase.
pub fn normalize_ref(reference: &str) -> String {
    let decoded = percent_decode(&reference.replace('\\', "/"));
    let mut rest = decoded.as_str();
    while let Some(stripped) = rest.strip_prefix("./") {
        rest = stripped;
    }
    rest.to_ascii_lowercase()
}

/// Model file formats the importer understands.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFormat {
    Obj,
    Gltf,
    Glb,
    /// Autodesk FBX, binary or ASCII.
    Fbx,
}

impl ModelFormat {
    /// File extension without the dot: "obj", "gltf", "glb" or "fbx".
    pub fn extension(self) -> &'static str {
        match self {
            ModelFormat::Obj => "obj",
            ModelFormat::Gltf => "gltf",
            ModelFormat::Glb => "glb",
            ModelFormat::Fbx => "fbx",
        }
    }
}

/// One captured source file.
#[derive(Clone, Debug)]
pub struct SourceFile {
    /// `normalize_ref` of the reference.
    pub key: String,
    pub original_name: String,
    pub bytes: Arc<[u8]>,
}

/// The original model file plus every sidecar that was read, plus user-chosen texture files. Send + Sync.
#[derive(Clone, Debug)]
pub struct SourceBundle {
    pub format: ModelFormat,
    pub main: SourceFile,
    pub sidecars: Vec<SourceFile>,
    /// Original files of manually chosen textures (TextureChoice::Image), key
    /// "textures/set<NN>/<name>". Never read by the importer: `MemoryResolver::from_bundle` ignores them.
    pub extras: Vec<SourceFile>,
    /// Informational only; never read back.
    pub original_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_dir;

    fn bytes(text: &str) -> Arc<[u8]> {
        Arc::from(text.as_bytes())
    }

    fn file(key: &str, content: &str) -> SourceFile {
        SourceFile { key: key.to_owned(), original_name: key.to_owned(), bytes: bytes(content) }
    }

    #[test]
    fn normalize_ref_rules() {
        assert_eq!(normalize_ref("Textures\\Wood.PNG"), "textures/wood.png");
        assert_eq!(normalize_ref("./a/B.png"), "a/b.png");
        assert_eq!(normalize_ref(".\\.\\a.png"), "a.png");
        assert_eq!(normalize_ref("my%20Tex.png"), "my tex.png");
        assert_eq!(normalize_ref("%2E/x.bin"), "x.bin");
        assert_eq!(normalize_ref("../up/T.png"), "../up/t.png");
        assert_eq!(normalize_ref("C:\\Dir\\T.png"), "c:/dir/t.png");
        // Only ASCII is case-folded.
        assert_eq!(normalize_ref("\u{c9}T\u{c9}.png"), "\u{c9}t\u{c9}.png");
        assert_eq!(normalize_ref(""), "");
    }

    #[test]
    fn model_format_extensions_and_serde_names() {
        assert_eq!(ModelFormat::Obj.extension(), "obj");
        assert_eq!(ModelFormat::Gltf.extension(), "gltf");
        assert_eq!(ModelFormat::Glb.extension(), "glb");
        assert_eq!(ModelFormat::Fbx.extension(), "fbx");
        assert_eq!(serde_json::to_string(&ModelFormat::Fbx).unwrap(), "\"fbx\"");
        assert_eq!(serde_json::to_string(&ModelFormat::Glb).unwrap(), "\"glb\"");
        assert_eq!(serde_json::from_str::<ModelFormat>("\"obj\"").unwrap(), ModelFormat::Obj);
    }

    #[test]
    fn file_name_helpers() {
        assert_eq!(file_name_of("a/b\\c.png"), "c.png");
        assert_eq!(file_name_of("c.png"), "c.png");
        assert_eq!(file_name_of("dir/"), "");
        assert_eq!(file_stem_of("textures/Wood.A.png"), "Wood.A");
        assert_eq!(file_stem_of(".hidden"), ".hidden");
        assert_eq!(file_stem_of("noext"), "noext");
    }

    #[test]
    fn memory_resolver_normalizes_references() {
        let mut resolver = MemoryResolver::default();
        resolver.insert("Textures/Wood.png", bytes("wood"));
        assert_eq!(&*resolver.read("textures\\WOOD.PNG").unwrap(), b"wood");
        assert_eq!(&*resolver.read("./Textures/Wood.png").unwrap(), b"wood");
        assert_eq!(&*resolver.read("textures/wood%2Epng").unwrap(), b"wood");
        match resolver.read("textures/stone.png") {
            Err(ResolveError::NotFound(name)) => assert_eq!(name, "textures/stone.png"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn from_bundle_reads_sidecars_and_ignores_extras() {
        let bundle = SourceBundle {
            format: ModelFormat::Obj,
            main: file("model.obj", "main"),
            // The second key is deliberately not normalized.
            sidecars: vec![file("model.mtl", "mtl"), file("Tex/Albedo.PNG", "albedo")],
            extras: vec![file("textures/set00/chosen.png", "chosen")],
            original_path: None,
        };
        let resolver = MemoryResolver::from_bundle(&bundle);
        assert_eq!(&*resolver.read("MODEL.MTL").unwrap(), b"mtl");
        assert_eq!(&*resolver.read("tex\\albedo.png").unwrap(), b"albedo");
        assert!(resolver.read("textures/set00/chosen.png").is_err(), "extras are never read by the importer");
        assert!(resolver.read("chosen.png").is_err());
        assert!(resolver.read("model.obj").is_err(), "the main file is not a sidecar");
    }

    #[test]
    fn recording_resolver_captures_exactly_the_files_read() {
        let mut inner = MemoryResolver::default();
        inner.insert("a.mtl", bytes("A"));
        inner.insert("tex/b.png", bytes("B"));
        inner.insert("unused.png", bytes("U"));
        let recorder = RecordingResolver::new(inner);
        assert_eq!(&*recorder.read("Tex\\B.png").unwrap(), b"B");
        assert_eq!(&*recorder.read("a.mtl").unwrap(), b"A");
        assert!(recorder.read("missing.png").is_err());
        // Second reads (also through another spelling) neither duplicate the capture nor the miss.
        assert_eq!(&*recorder.read("tex/b.png").unwrap(), b"B");
        assert_eq!(&*recorder.read("./A.MTL").unwrap(), b"A");
        assert!(recorder.read("missing.png").is_err());
        assert!(recorder.read("my%20other.png").is_err());

        let (captured, missing) = recorder.into_captured();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].key, "tex/b.png");
        assert_eq!(captured[0].original_name, "B.png");
        assert_eq!(&*captured[0].bytes, b"B");
        assert_eq!(captured[1].key, "a.mtl");
        assert_eq!(captured[1].original_name, "a.mtl");
        assert_eq!(missing, vec!["missing.png".to_owned(), "my%20other.png".to_owned()]);
    }

    #[test]
    fn recorded_files_resolve_again_from_the_bundle() {
        let mut inner = MemoryResolver::default();
        inner.insert("Sub Dir/My Tex.png", bytes("T"));
        let recorder = RecordingResolver::new(inner);
        recorder.read("Sub%20Dir\\My%20Tex.png").unwrap();
        let (sidecars, _) = recorder.into_captured();
        assert_eq!(sidecars[0].original_name, "My Tex.png");
        let bundle = SourceBundle {
            format: ModelFormat::Gltf,
            main: file("m.gltf", "{}"),
            sidecars,
            extras: Vec::new(),
            original_path: None,
        };
        let memory = MemoryResolver::from_bundle(&bundle);
        assert_eq!(&*memory.read("Sub%20Dir\\My%20Tex.png").unwrap(), b"T");
    }

    #[test]
    fn disk_resolver_lookup_order() {
        let dir = temp_dir("resolver-disk");
        std::fs::create_dir_all(dir.join("Textures")).unwrap();
        std::fs::create_dir_all(dir.join("sub").join("deep")).unwrap();
        std::fs::write(dir.join("model.mtl"), b"mtl").unwrap();
        std::fs::write(dir.join("Textures").join("Wood.png"), b"wood").unwrap();
        std::fs::write(dir.join("sub").join("deep").join("Stone Wall.png"), b"stone").unwrap();
        std::fs::write(dir.join("loose.png"), b"loose").unwrap();
        let resolver = DiskResolver { base_dir: dir.clone() };

        // 1. As written.
        assert_eq!(&*resolver.read("model.mtl").unwrap(), b"mtl");
        assert_eq!(&*resolver.read("  model.mtl ").unwrap(), b"mtl");
        // 2. Backslash paths and mixed separators.
        assert_eq!(&*resolver.read("sub\\deep\\Stone Wall.png").unwrap(), b"stone");
        assert_eq!(&*resolver.read("sub/deep\\Stone Wall.png").unwrap(), b"stone");
        assert_eq!(&*resolver.read(".\\sub\\..\\loose.png").unwrap(), b"loose");
        // 3. Case-insensitive, component by component.
        assert_eq!(&*resolver.read("MODEL.MTL").unwrap(), b"mtl");
        assert_eq!(&*resolver.read("textures\\WOOD.PNG").unwrap(), b"wood");
        assert_eq!(&*resolver.read("SUB/Deep/stone wall.PNG").unwrap(), b"stone");
        // Percent-encoded (glTF) spelling.
        assert_eq!(&*resolver.read("sub/deep/Stone%20Wall.png").unwrap(), b"stone");
        // 4. Absolute and dangling paths fall back to the bare file name, in base_dir and then in textures/.
        assert_eq!(&*resolver.read("C:\\Users\\artist\\Desktop\\loose.png").unwrap(), b"loose");
        assert_eq!(&*resolver.read("/home/artist/project/wood.png").unwrap(), b"wood");
        assert_eq!(&*resolver.read("..\\..\\gone\\Wood.png").unwrap(), b"wood");
        // An absolute reference that exists is read from where it points.
        let absolute = dir.join("Textures").join("Wood.png");
        assert_eq!(&*resolver.read(&absolute.to_string_lossy()).unwrap(), b"wood");

        // Misses: nothing there, empty reference, a directory.
        assert!(matches!(resolver.read("nope.png"), Err(ResolveError::NotFound(name)) if name == "nope.png"));
        assert!(matches!(resolver.read(""), Err(ResolveError::NotFound(_))));
        assert!(matches!(resolver.read("Textures"), Err(ResolveError::NotFound(_))));
        assert!(matches!(resolver.read("sub/"), Err(ResolveError::NotFound(_))));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn disk_resolver_through_the_recorder() {
        let dir = temp_dir("resolver-record");
        std::fs::write(dir.join("Body.PNG"), b"body").unwrap();
        let recorder = RecordingResolver::new(DiskResolver { base_dir: dir.clone() });
        assert_eq!(&*recorder.read("D:\\export\\body.png").unwrap(), b"body");
        assert!(recorder.read("head.png").is_err());
        let (captured, missing) = recorder.into_captured();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].key, "d:/export/body.png");
        assert_eq!(captured[0].original_name, "body.png");
        assert_eq!(missing, vec!["head.png".to_owned()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
