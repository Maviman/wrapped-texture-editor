// Owner: core-import (C5). Model import of ARCHITECTURE.md 5.21 (importer notes: section 10): format sniffing, the
// two entry points (from disk, from a captured bundle), texture loading shared by the format loaders, and the
// warnings derived from the normalized mesh.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::{ImageIoError, ImportError, ImportWarning};
use crate::image_io::{DecodedImage, decode_image};
use crate::mesh::{MeshData, MeshReport, RawMesh};
use crate::mesh_build::{NormalizeOptions, normalize};
use crate::pixel::PixelBuf;
use crate::progress::Progress;

mod auto_uv;
mod fbx;
mod gltf;
pub mod mtl_ref;
mod obj;
pub mod resolver;
pub mod uri;

pub use mtl_ref::parse_texture_ref;
pub use resolver::{
    AssetResolver, DiskResolver, MemoryResolver, ModelFormat, RecordingResolver, SourceBundle, SourceFile, normalize_ref,
};
pub use uri::{base64_decode, parse_data_uri, percent_decode};

// `self::` because the local module `gltf` shadows the extern crate of the same name here (use `::gltf` for the crate).
pub(crate) use self::auto_uv::generate as generate_atlas_uvs;
pub(crate) use self::fbx::load_fbx;
pub(crate) use self::gltf::load_gltf;
pub(crate) use self::obj::load_obj;

use self::resolver::{file_name_of, file_stem_of};

/// Options of a model import; persisted in the project manifest.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ImportOptions {
    pub generate_fallback_uvs: bool,
    pub max_texture_dim: u32,
}

impl Default for ImportOptions {
    /// No fallback UVs, textures up to 8192 px.
    fn default() -> ImportOptions {
        ImportOptions { generate_fallback_uvs: false, max_texture_dim: 8192 }
    }
}

/// 2: the generated (fallback) layout is the box-projection atlas of `import/auto_uv.rs`; skinned glTF meshes are posed
/// by their skin; FBX import. Stored in the manifest; `project::load` compares it (ARCHITECTURE 7.1 step 3).
/// RULE: any change to the generated layout or to mesh normalisation that changes `uv_hash` for some input must bump this
/// constant, and from then on keep the previous path selectable by version (the manifest's `importer_version` passed
/// through `ImportOptions` / `NormalizeOptions`). `mesh_build` test `generated_uv_layout_of_importer_version_2_is_frozen`
/// pins the version-2 hashes of `no_uv_tetra` and a UV-less `uv_sphere(32, 16)`, so a silent change fails the tests.
/// A format loader that starts reading something it ignored before (FBX texture UV transforms, R3-12) is not such a
/// change: the files it affects were mapped wrongly before (ARCHITECTURE 5.21).
/// (No version-1 generator exists: no project from before version 2 with generated UVs was ever saved by a user.)
pub const IMPORTER_VERSION: u32 = 2;

/// One texture referenced by the model.
#[derive(Clone, Debug)]
pub struct ImportedTexture {
    /// File stem.
    pub name: String,
    pub reference: String,
    /// None: missing, undecodable, or `decode_textures == false`.
    pub image: Option<PixelBuf>,
}

/// Result of an import: the normalized mesh, its textures and the captured source files.
#[derive(Clone, Debug)]
pub struct ImportedModel {
    /// File stem.
    pub name: String,
    pub mesh: MeshData,
    /// Indexed by `SetInfo::texture`.
    pub textures: Vec<ImportedTexture>,
    pub report: MeshReport,
    pub warnings: Vec<ImportWarning>,
    pub bundle: SourceBundle,
    pub options: ImportOptions,
}

/// Largest model file or sidecar a project can store (`project::load` refuses larger entries): the importer refuses a
/// larger model file and treats a larger sidecar as unreadable, and `project::save` refuses to write one.
pub const MAX_SOURCE_FILE: u64 = 2 << 30;

/// b"glTF" magic -> Glb; b"Kaydara FBX Binary" magic or an ASCII file starting with the "; FBX" comment -> Fbx;
/// first non-whitespace byte '{' -> Gltf; else by extension (.obj/.gltf/.glb/.fbx).
///
/// A UTF-8 byte order mark in front of the JSON is skipped; the extension is compared without regard to case.
pub fn sniff_model_format(bytes: &[u8], file_name: &str) -> Option<ModelFormat> {
    if bytes.starts_with(b"glTF") {
        return Some(ModelFormat::Glb);
    }
    if bytes.starts_with(b"Kaydara FBX Binary") {
        return Some(ModelFormat::Fbx);
    }
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if text.starts_with(b"; FBX") {
        return Some(ModelFormat::Fbx);
    }
    if text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        return Some(ModelFormat::Gltf);
    }
    let (_, extension) = file_name_of(file_name).rsplit_once('.')?;
    [ModelFormat::Obj, ModelFormat::Gltf, ModelFormat::Glb, ModelFormat::Fbx]
        .into_iter()
        .find(|format| extension.eq_ignore_ascii_case(format.extension()))
}

/// Reads the file and its sidecars from disk (DiskResolver wrapped in RecordingResolver) and captures them in the bundle
/// (`extras` empty). Always decodes the referenced textures.
///
/// The model is named after the file stem; `bundle.main` has `key = normalize_ref(file name)` and the file name as
/// `original_name`; `bundle.original_path` is the absolute path. Sidecars appear in the order they were first read.
pub fn import_from_path(path: &Path, opts: &ImportOptions, progress: &dyn Progress) -> Result<ImportedModel, ImportError> {
    import_file(path, opts, false, progress)
}

/// `import_from_path`, except that with `auto_uvs` a model none of whose triangles has usable UVs gets the generated
/// texture layout right away (as if `generate_fallback_uvs` had been set; the returned `options` say so, so a saved
/// project re-imports the same way). Models with UVs, also partly mapped ones, are imported exactly as by
/// `import_from_path`. This is what *New from Model* uses: every model with geometry can be painted.
pub fn import_from_path_auto_uvs(
    path: &Path,
    opts: &ImportOptions,
    auto_uvs: bool,
    progress: &dyn Progress,
) -> Result<ImportedModel, ImportError> {
    import_file(path, opts, auto_uvs, progress)
}

/// Geometry only (FEATURE_BRUSH 4.1, brush shapes): reads the model file and the side files its geometry needs,
/// decodes no texture, generates no UVs, keeps no file bytes. `import_from_path` with the texture switch off,
/// `auto_uvs` off and the bundle dropped; a model without UVs, or whose MTL names a texture that does not exist, is
/// fine. Uses the default `ImportOptions` (glTF node transforms and FBX units are applied as for a model import).
pub fn import_geometry_from_path(path: &Path, progress: &dyn Progress) -> Result<MeshData, ImportError> {
    let file = read_model_file(path, progress)?;
    let name = file_stem_of(&file.file_name).to_owned();
    let resolver = DiskResolver { base_dir: file.base_dir };
    let parts = import_core(&file.bytes, file.format, &resolver, &ImportOptions::default(), false, false, &name, progress)?;
    Ok(parts.mesh)
}

/// A model file read from disk by `read_model_file`.
struct ModelFile {
    file_name: String,
    bytes: Vec<u8>,
    format: ModelFormat,
    /// The folder its side files are looked up in.
    base_dir: PathBuf,
}

/// The first steps of every import from disk: cancellation, the size limit, reading and sniffing the format.
fn read_model_file(path: &Path, progress: &dyn Progress) -> Result<ModelFile, ImportError> {
    if progress.cancelled() {
        return Err(ImportError::Cancelled);
    }
    progress.report(0.0, "Reading model file");
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let size = std::fs::metadata(path)?.len();
    if size > MAX_SOURCE_FILE {
        return Err(ImportError::FileTooLarge { name: file_name, size, limit: MAX_SOURCE_FILE });
    }
    let bytes = std::fs::read(path)?;
    let format = sniff_model_format(&bytes, &file_name).ok_or(ImportError::UnknownFormat)?;
    let base_dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => Path::new(".").to_path_buf(),
    };
    Ok(ModelFile { file_name, bytes, format, base_dir })
}

fn import_file(path: &Path, opts: &ImportOptions, auto_uvs: bool, progress: &dyn Progress) -> Result<ImportedModel, ImportError> {
    let ModelFile { file_name, bytes, format, base_dir } = read_model_file(path, progress)?;
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let name = file_stem_of(&file_name).to_owned();

    let recorder = RecordingResolver::new(DiskResolver { base_dir });
    let parts = import_core(&bytes, format, &recorder, opts, true, auto_uvs, &name, progress)?;
    let (sidecars, _missing) = recorder.into_captured();
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let bundle = SourceBundle {
        format,
        main: SourceFile { key: normalize_ref(&file_name), original_name: file_name, bytes },
        sidecars,
        extras: Vec::new(),
        original_path: Some(absolute.to_string_lossy().into_owned()),
    };
    Ok(parts.into_model(name, bundle, *opts))
}

/// Re-imports purely from memory (project load, samples, tests). Never touches the disk. With `decode_textures == false`
/// (project load: the pixels come from the layer PNGs) no texture is decoded, every `ImportedTexture.image` is None and no
/// MissingTexture / TextureDecodeFailed / TextureResized warning is emitted; mesh, sets and names are identical either way.
/// The returned model's bundle is a clone of `bundle` (extras included).
///
/// The format is `bundle.format`; the model is named after the stem of `bundle.main.original_name` (of its key when
/// that is empty). Sidecars are looked up through `MemoryResolver::from_bundle`.
pub fn import_from_bundle(
    bundle: &SourceBundle,
    opts: &ImportOptions,
    decode_textures: bool,
    progress: &dyn Progress,
) -> Result<ImportedModel, ImportError> {
    if progress.cancelled() {
        return Err(ImportError::Cancelled);
    }
    progress.report(0.0, "Reading model file");
    let resolver = MemoryResolver::from_bundle(bundle);
    let mut name = file_stem_of(&bundle.main.original_name);
    if name.is_empty() {
        name = file_stem_of(&bundle.main.key);
    }
    let parts = import_core(&bundle.main.bytes, bundle.format, &resolver, opts, decode_textures, false, name, progress)?;
    Ok(parts.into_model(name.to_owned(), bundle.clone(), *opts))
}

/// Everything an import produces apart from the bundle.
struct ImportParts {
    mesh: MeshData,
    textures: Vec<ImportedTexture>,
    report: MeshReport,
    warnings: Vec<ImportWarning>,
}

impl ImportParts {
    /// `options.generate_fallback_uvs` is set when the generated layout was applied (also automatically).
    fn into_model(self, name: String, bundle: SourceBundle, mut options: ImportOptions) -> ImportedModel {
        options.generate_fallback_uvs |= self.report.fallback_uvs_generated;
        ImportedModel {
            name,
            mesh: self.mesh,
            textures: self.textures,
            report: self.report,
            warnings: self.warnings,
            bundle,
            options,
        }
    }
}

/// Parses the main file with the loader of `format`, normalizes the mesh and collects the warnings. With
/// `auto_uvs` a model without any UVs gets the generated layout even when `opts` does not ask for it.
#[allow(clippy::too_many_arguments)]
fn import_core(
    main: &[u8],
    format: ModelFormat,
    resolver: &dyn AssetResolver,
    opts: &ImportOptions,
    decode_textures: bool,
    auto_uvs: bool,
    model_name: &str,
    progress: &dyn Progress,
) -> Result<ImportParts, ImportError> {
    let check = || if progress.cancelled() { Err(ImportError::Cancelled) } else { Ok(()) };
    check()?;
    progress.report(0.05, "Parsing model");
    let raw = match format {
        ModelFormat::Obj => load_obj(main, resolver, opts, decode_textures)?,
        ModelFormat::Gltf | ModelFormat::Glb => load_gltf(main, resolver, opts, decode_textures)?,
        ModelFormat::Fbx => load_fbx(main, resolver, opts, decode_textures)?,
    };
    check()?;
    // Why geometry was skipped: the only clue the user gets when nothing is left.
    let reasons = || -> Vec<String> {
        raw.warnings
            .iter()
            .filter_map(|w| match w {
                ImportWarning::SkippedPrimitive { reason } => Some(reason.clone()),
                _ => None,
            })
            .collect()
    };
    if raw.mesh.triangles.is_empty() {
        return Err(ImportError::Empty { reasons: reasons() });
    }
    check()?;
    progress.report(0.7, "Building mesh");
    // `normalize` only generates the layout when no triangle has usable UVs (missing, or all degenerate).
    let normalize_opts = NormalizeOptions {
        generate_fallback_uvs: opts.generate_fallback_uvs || auto_uvs,
        model_name: model_name.to_owned(),
        texture_names: raw.textures.iter().map(|t| t.name.clone()).collect(),
    };
    let empty_reasons = reasons();
    let (mesh, report) = normalize(raw.mesh, &normalize_opts);
    if mesh.tri_count() == 0 {
        let mut reasons = empty_reasons;
        if report.dropped_degenerate > 0 {
            reasons.push(format!("{} degenerate triangle(s) were dropped", report.dropped_degenerate));
        }
        return Err(ImportError::Empty { reasons });
    }
    check()?;
    let mut warnings = raw.warnings;
    mesh_warnings(&mesh, &report, &mut warnings);
    progress.report(1.0, "Model imported");
    Ok(ImportParts { mesh, textures: raw.textures, report, warnings })
}

/// Appends the warnings that follow from the normalization report: dropped triangles, generated normals, folded
/// UV tiles, fallback UVs and — unless fallback UVs replaced them — materials without (usable) UVs.
fn mesh_warnings(mesh: &MeshData, report: &MeshReport, warnings: &mut Vec<ImportWarning>) {
    if report.dropped_degenerate > 0 {
        warnings.push(ImportWarning::DroppedDegenerate { triangles: report.dropped_degenerate });
    }
    if report.normals_generated {
        warnings.push(ImportWarning::NormalsGenerated);
    }
    if report.wrapped_tris > 0 {
        warnings.push(ImportWarning::UvWrapped { triangles: report.wrapped_tris });
    }
    if report.fallback_uvs_generated {
        warnings.push(ImportWarning::FallbackUvsGenerated);
        return;
    }
    for (m, material) in mesh.materials.iter().enumerate() {
        let total = mesh.material_ranges[m].len() as u64;
        let unmapped = mesh.material_nouv_ranges[m].len() as u64;
        if total == 0 || unmapped == 0 {
            continue;
        }
        if unmapped == total {
            warnings.push(ImportWarning::NoUvs { material: material.name.clone() });
        } else {
            let percent = (unmapped * 100).div_ceil(total).clamp(1, 99) as u8;
            warnings.push(ImportWarning::DegenerateUvs { material: material.name.clone(), percent });
        }
    }
}

/// Output of the format-specific loaders before normalization. `import/mod.rs` then calls
/// `normalize(raw.mesh, &NormalizeOptions { generate_fallback_uvs: opts.generate_fallback_uvs, model_name: <file stem>,
/// texture_names: raw.textures.iter().map(|t| t.name.clone()).collect() })`.
pub(crate) struct RawImport {
    pub mesh: RawMesh,
    pub textures: Vec<ImportedTexture>,
    pub warnings: Vec<ImportWarning>,
}

/// Where the encoded bytes of a texture come from.
pub(crate) enum TextureSource {
    /// `TextureRequest::reference` is read through the resolver.
    Resolver,
    /// Bytes embedded in the model (data URI).
    Bytes(Arc<[u8]>),
    /// A range of a loaded buffer (glTF buffer view).
    Slice { buffer: Arc<[u8]>, range: Range<usize> },
    /// The model names no source for this texture.
    Missing,
    /// The model's description of the source is unusable; the string says why.
    Broken(String),
}

/// One texture a loader wants; `load_textures` turns the list into `ImportedTexture`s of the same order.
pub(crate) struct TextureRequest {
    /// `ImportedTexture::name`.
    pub name: String,
    /// `ImportedTexture::reference`.
    pub reference: String,
    /// Name of the first material using the texture (for `MissingTexture`).
    pub material: String,
    /// File name for format detection by extension (TGA), when known.
    pub name_hint: Option<String>,
    pub source: TextureSource,
}

/// Reads and decodes the requested textures. The reference and the name of every request are always recorded.
/// With `decode_textures == false` nothing is read or decoded and no warning is emitted. Otherwise the files are
/// fetched one after the other (the resolver need not be thread-safe) and decoded in parallel; warnings are
/// appended in request order.
pub(crate) fn load_textures(
    requests: Vec<TextureRequest>,
    resolver: &dyn AssetResolver,
    opts: &ImportOptions,
    decode_textures: bool,
    warnings: &mut Vec<ImportWarning>,
) -> Vec<ImportedTexture> {
    if !decode_textures {
        return requests
            .into_iter()
            .map(|r| ImportedTexture { name: r.name, reference: r.reference, image: None })
            .collect();
    }
    enum Fetched {
        Data(Arc<[u8]>, Range<usize>),
        Missing,
        Broken(String),
    }
    let fetched: Vec<Fetched> = requests
        .iter()
        .map(|request| match &request.source {
            TextureSource::Resolver => match resolver.read(&request.reference) {
                Ok(bytes) => {
                    let len = bytes.len();
                    Fetched::Data(bytes, 0..len)
                }
                Err(_) => Fetched::Missing,
            },
            TextureSource::Bytes(bytes) => Fetched::Data(bytes.clone(), 0..bytes.len()),
            TextureSource::Slice { buffer, range } => Fetched::Data(buffer.clone(), range.clone()),
            TextureSource::Missing => Fetched::Missing,
            TextureSource::Broken(reason) => Fetched::Broken(reason.clone()),
        })
        .collect();
    let max_dim = opts.max_texture_dim.max(1);
    let decoded: Vec<Option<Result<DecodedImage, ImageIoError>>> = fetched
        .par_iter()
        .zip(requests.par_iter())
        .map(|(fetched, request)| match fetched {
            Fetched::Data(bytes, range) => Some(match bytes.get(range.clone()) {
                Some(data) => decode_image(data, request.name_hint.as_deref(), max_dim),
                None => Err(ImageIoError::Decode("the image data lies outside its buffer".to_owned())),
            }),
            _ => None,
        })
        .collect();

    let mut textures = Vec::with_capacity(requests.len());
    for ((request, fetched), decoded) in requests.into_iter().zip(fetched).zip(decoded) {
        let image = match (fetched, decoded) {
            (_, Some(Ok(image))) => {
                if image.resized {
                    warnings.push(ImportWarning::TextureResized {
                        reference: request.reference.clone(),
                        from: image.original_size,
                        to: [image.pixels.width, image.pixels.height],
                    });
                }
                Some(image.pixels)
            }
            (_, Some(Err(error))) => {
                warnings.push(ImportWarning::TextureDecodeFailed {
                    reference: request.reference.clone(),
                    reason: error.to_string(),
                });
                None
            }
            (Fetched::Broken(reason), None) => {
                warnings.push(ImportWarning::TextureDecodeFailed { reference: request.reference.clone(), reason });
                None
            }
            (_, None) => {
                warnings.push(ImportWarning::MissingTexture {
                    material: request.material.clone(),
                    reference: request.reference.clone(),
                });
                None
            }
        };
        textures.push(ImportedTexture { name: request.name, reference: request.reference, image });
    }
    textures
}

/// Decodes the text of an OBJ / MTL file: UTF-8 when valid, otherwise Latin-1 (every byte is the code point of
/// the same value), so file names written by old exporters keep their letters. A UTF-8 byte order mark is dropped;
/// a UTF-16 byte order mark (LE or BE, as Notepad's "Unicode" saves) decodes the rest as UTF-16, invalid units
/// becoming U+FFFD. Line endings are normalized to `\n`: CRLF (Windows), a lone CR (classic Mac OS) and any mix of them, so every
/// line-based parser sees the same lines whatever wrote the file.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    for (bom, little) in [(b"\xff\xfe", true), (b"\xfe\xff", false)] {
        if let Some(rest) = bytes.strip_prefix(bom) {
            let units = rest.as_chunks::<2>().0.iter().map(|&c| if little { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
            let text: String = char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
            return normalize_line_endings(text);
        }
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    normalize_line_endings(text)
}

/// `\r\n` and lone `\r` become `\n`; text without a CR is returned unchanged (no copy).
pub(crate) fn normalize_line_endings(text: String) -> String {
    if !text.contains('\r') {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// An sRGB-encoded colour in 0..1 from possibly out-of-range components; non-finite values become `fallback`.
pub(crate) fn clean_unit(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { fallback }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::image_io::{PngLevel, encode_png};
    use crate::progress::NoProgress;
    use crate::samples;
    use crate::testutil::temp_dir;

    fn png(image: &PixelBuf) -> Arc<[u8]> {
        Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
    }

    fn source(name: &str, bytes: Arc<[u8]>) -> SourceFile {
        SourceFile { key: normalize_ref(name), original_name: name.to_owned(), bytes }
    }

    /// The two-material sample as an OBJ bundle; `with_head_texture == false` leaves `head.png` out.
    fn two_material_bundle(with_head_texture: bool) -> SourceBundle {
        let raw = samples::two_material_model();
        let (obj, mtl) = samples::to_obj(&raw, "Robot.mtl", &["Body.png".to_owned(), "tex/Head.png".to_owned()]);
        let mut sidecars = vec![
            source("Robot.mtl", Arc::from(mtl.as_bytes())),
            source("Body.png", png(&samples::checker_texture(32, 32, 4))),
        ];
        if with_head_texture {
            sidecars.push(source("tex/Head.png", png(&samples::checker_texture(16, 8, 2))));
        }
        SourceBundle {
            format: ModelFormat::Obj,
            main: source("Robot.obj", Arc::from(obj.as_bytes())),
            sidecars,
            extras: vec![source("textures/set00/chosen.png", Arc::from(&b"not read"[..]))],
            original_path: Some("X:\\models\\Robot.obj".to_owned()),
        }
    }

    fn texture_warning(w: &ImportWarning) -> bool {
        matches!(
            w,
            ImportWarning::MissingTexture { .. }
                | ImportWarning::TextureDecodeFailed { .. }
                | ImportWarning::TextureResized { .. }
        )
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff_model_format(b"glTF\x02\0\0\0", "model.bin"), Some(ModelFormat::Glb));
        assert_eq!(sniff_model_format(b"  \r\n\t{ \"asset\": {} }", "model.txt"), Some(ModelFormat::Gltf));
        assert_eq!(sniff_model_format(b"\xef\xbb\xbf{}", "model"), Some(ModelFormat::Gltf));
        assert_eq!(sniff_model_format(b"# comment\nv 0 0 0\n", "Model.OBJ"), Some(ModelFormat::Obj));
        assert_eq!(sniff_model_format(b"v 0 0 0\n", "dir.v2/model.obj"), Some(ModelFormat::Obj));
        assert_eq!(sniff_model_format(b"", "a.gltf"), Some(ModelFormat::Gltf));
        assert_eq!(sniff_model_format(b"", "a.GLB"), Some(ModelFormat::Glb));
        // The content wins over the extension.
        assert_eq!(sniff_model_format(b"glTF....", "wrong.obj"), Some(ModelFormat::Glb));
        assert_eq!(sniff_model_format(b"{}", "wrong.obj"), Some(ModelFormat::Gltf));
        assert_eq!(sniff_model_format(b"v 0 0 0\n", "model.FBX"), Some(ModelFormat::Fbx));
        assert_eq!(sniff_model_format(b"Kaydara FBX Binary  \0\x1a\0", "x.obj"), Some(ModelFormat::Fbx));
        assert_eq!(sniff_model_format(b"; FBX 7.4.0 project file\n", "x.txt"), Some(ModelFormat::Fbx));
        assert_eq!(sniff_model_format(b"v 0 0 0\n", "model.3ds"), None);
        assert_eq!(sniff_model_format(b"v 0 0 0\n", "model"), None);
        assert_eq!(sniff_model_format(b"v 0 0 0\n", "folder.obj/model"), None);
        assert_eq!(sniff_model_format(b"", ""), None);
    }

    #[test]
    fn import_options_defaults_and_serde() {
        let opts = ImportOptions::default();
        assert_eq!(opts, ImportOptions { generate_fallback_uvs: false, max_texture_dim: 8192 });
        let json = serde_json::to_string(&opts).unwrap();
        assert_eq!(serde_json::from_str::<ImportOptions>(&json).unwrap(), opts);
        assert_eq!(IMPORTER_VERSION, 2);
    }

    #[test]
    fn bundle_import_builds_the_two_material_model() {
        let bundle = two_material_bundle(true);
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        assert_eq!(model.name, "Robot");
        assert_eq!(model.options, ImportOptions::default());
        assert_eq!(model.mesh.sets.len(), 2);
        assert_eq!(model.mesh.sets[0].name, "Body");
        assert_eq!(model.mesh.sets[1].name, "Head");
        assert_eq!(model.mesh.sets[0].texture, Some(0));
        assert_eq!(model.mesh.sets[1].texture, Some(1));
        assert_eq!(model.textures.len(), 2);
        assert_eq!(model.textures[0].reference, "Body.png");
        assert_eq!(model.textures[1].reference, "tex/Head.png");
        assert_eq!(model.textures[0].image.as_ref().unwrap(), &samples::checker_texture(32, 32, 4));
        assert_eq!(model.textures[1].image.as_ref().unwrap(), &samples::checker_texture(16, 8, 2));
        assert!(model.warnings.is_empty(), "{:?}", model.warnings);
        assert_eq!(model.report.total_tris, samples::two_material_model().triangles.len() as u32);
        // The bundle is returned as given, extras included.
        assert_eq!(model.bundle.sidecars.len(), 3);
        assert_eq!(model.bundle.extras.len(), 1);
        assert_eq!(model.bundle.original_path.as_deref(), Some("X:\\models\\Robot.obj"));
        assert!(Arc::ptr_eq(&model.bundle.main.bytes, &bundle.main.bytes));
    }

    #[test]
    fn decode_textures_false_gives_the_same_mesh_without_images_or_texture_warnings() {
        for with_head_texture in [true, false] {
            let bundle = two_material_bundle(with_head_texture);
            // A tiny limit so that the decoding run also produces a TextureResized warning.
            let opts = ImportOptions { generate_fallback_uvs: false, max_texture_dim: 16 };
            let full = import_from_bundle(&bundle, &opts, true, &NoProgress).unwrap();
            let lean = import_from_bundle(&bundle, &opts, false, &NoProgress).unwrap();

            assert_eq!(lean.mesh.uv_hash, full.mesh.uv_hash);
            assert_eq!(lean.mesh.sets, full.mesh.sets);
            assert_eq!(lean.mesh.materials, full.mesh.materials);
            assert_eq!(lean.mesh.positions, full.mesh.positions);
            assert_eq!(lean.mesh.uvs, full.mesh.uvs);
            assert_eq!(lean.mesh.indices, full.mesh.indices);
            assert_eq!(lean.report, full.report);
            let names = |m: &ImportedModel| -> Vec<(String, String)> {
                m.textures.iter().map(|t| (t.name.clone(), t.reference.clone())).collect()
            };
            assert_eq!(names(&lean), names(&full));
            assert_eq!(lean.mesh.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Body", "Head"]);

            assert!(lean.textures.iter().all(|t| t.image.is_none()));
            assert!(!lean.warnings.iter().any(texture_warning), "{:?}", lean.warnings);
            // The decoding run reports the downscale and, when the file is absent, the missing texture.
            assert!(full.warnings.contains(&ImportWarning::TextureResized {
                reference: "Body.png".to_owned(),
                from: [32, 32],
                to: [16, 16],
            }));
            let missing = ImportWarning::MissingTexture { material: "Head".to_owned(), reference: "tex/Head.png".to_owned() };
            assert_eq!(full.warnings.contains(&missing), !with_head_texture);
            assert_eq!(full.textures[1].image.is_some(), with_head_texture);
            // Apart from the texture warnings both runs warn about the same things.
            let other = |m: &ImportedModel| -> Vec<ImportWarning> {
                m.warnings.iter().filter(|w| !texture_warning(w)).cloned().collect()
            };
            assert_eq!(other(&lean), other(&full));
        }
    }

    #[test]
    fn undecodable_texture_is_a_warning() {
        let mut bundle = two_material_bundle(true);
        bundle.sidecars[1].bytes = Arc::from(&b"this is not a png"[..]);
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        assert!(model.textures[0].image.is_none());
        assert_eq!(model.textures[0].name, "Body");
        assert!(model.textures[1].image.is_some());
        assert_eq!(model.mesh.sets.len(), 2);
        assert!(
            model.warnings.iter().any(|w| matches!(w,
                ImportWarning::TextureDecodeFailed { reference, .. } if reference == "Body.png")),
            "{:?}",
            model.warnings
        );
    }

    #[test]
    fn mesh_warnings_follow_the_report() {
        // No UVs at all: one NoUvs warning, normals generated.
        let (obj, _) = samples::to_obj(&samples::no_uv_tetra(), "", &[]);
        let bundle = SourceBundle {
            format: ModelFormat::Obj,
            main: source("tetra.obj", Arc::from(obj.as_bytes())),
            sidecars: Vec::new(),
            extras: Vec::new(),
            original_path: None,
        };
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        assert!(model.report.no_uvs);
        assert!(model.mesh.sets.is_empty());
        assert_eq!(
            model.warnings,
            [ImportWarning::NormalsGenerated, ImportWarning::NoUvs { material: "Tetra".to_owned() }]
        );
        // The same file with fallback UVs: one set named after the model, a different warning.
        let opts = ImportOptions { generate_fallback_uvs: true, max_texture_dim: 8192 };
        let model = import_from_bundle(&bundle, &opts, true, &NoProgress).unwrap();
        assert_eq!(model.mesh.sets.len(), 1);
        assert_eq!(model.mesh.sets[0].name, "tetra");
        assert_eq!(model.options, opts);
        assert_eq!(model.warnings, [ImportWarning::NormalsGenerated, ImportWarning::FallbackUvsGenerated]);

        // Wrapped UVs and partially missing UVs.
        let mut raw = samples::wrapped_quad();
        raw.triangles[3].uvs = None;
        let (obj, _) = samples::to_obj(&raw, "", &[]);
        let bundle = SourceBundle { main: source("w.obj", Arc::from(obj.as_bytes())), ..bundle };
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        assert!(model.warnings.contains(&ImportWarning::UvWrapped { triangles: 3 }), "{:?}", model.warnings);
        assert!(
            model.warnings.contains(&ImportWarning::DegenerateUvs { material: "Wrapped".to_owned(), percent: 25 }),
            "{:?}",
            model.warnings
        );
    }

    #[test]
    fn empty_and_unknown_models_are_errors() {
        let bundle = |format, text: &str| SourceBundle {
            format,
            main: source("m.obj", Arc::from(text.as_bytes())),
            sidecars: Vec::new(),
            extras: Vec::new(),
            original_path: None,
        };
        let import = |b: &SourceBundle| import_from_bundle(b, &ImportOptions::default(), true, &NoProgress);
        assert!(matches!(import(&bundle(ModelFormat::Obj, "")), Err(ImportError::Empty { .. })));
        assert!(matches!(import(&bundle(ModelFormat::Obj, "v 0 0 0\nv 1 0 0\nv 0 1 0\n")), Err(ImportError::Empty { .. })));
        // Only degenerate triangles: nothing is left after normalization.
        assert!(matches!(
            import(&bundle(ModelFormat::Obj, "v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n")),
            Err(ImportError::Empty { reasons }) if reasons == ["1 degenerate triangle(s) were dropped"]
        ));
        assert!(matches!(import(&bundle(ModelFormat::Gltf, "{ nonsense")), Err(ImportError::InvalidGltf(_))));
        // Every primitive skipped: the reasons travel with the error and show in its message.
        let gltf = r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"data:application/octet-stream;base64,AAAAAA=="}],
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":1000,"min":[0,0,0],"max":[1,1,1]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]}"#;
        match import(&bundle(ModelFormat::Gltf, gltf)) {
            Err(error @ ImportError::Empty { .. }) => {
                let ImportError::Empty { reasons } = &error else { unreachable!() };
                assert_eq!(reasons.len(), 1);
                assert!(reasons[0].contains("outside buffer view 0"), "{reasons:?}");
                let message = error.to_string();
                assert!(message.starts_with("model contains no triangles (") && message.contains("buffer view 0"), "{message}");
            }
            other => panic!("expected Empty, got {:?}", other.map(|m| m.name)),
        }
        assert_eq!(ImportError::Empty { reasons: Vec::new() }.to_string(), "model contains no triangles");
        let many = ImportError::Empty { reasons: (0..5).map(|i| format!("r{i}")).collect() };
        assert_eq!(many.to_string(), "model contains no triangles (r0; r1; r2; and 2 more)");
        assert!(matches!(import(&bundle(ModelFormat::Glb, "glTF")), Err(ImportError::InvalidGltf(_))));
    }

    #[test]
    fn import_from_path_captures_the_sidecars() {
        let dir = temp_dir("import-path");
        let raw = samples::two_material_model();
        // Backslash and wrong-case references, as written by exporters on Windows.
        let (obj, mtl) = samples::to_obj(&raw, "robot.mtl", &["BODY.png".to_owned(), "Tex\\head.png".to_owned()]);
        std::fs::create_dir_all(dir.join("tex")).unwrap();
        std::fs::write(dir.join("Robot Model.obj"), &obj).unwrap();
        std::fs::write(dir.join("robot.mtl"), &mtl).unwrap();
        std::fs::write(dir.join("Body.png"), png(&samples::checker_texture(32, 32, 4))).unwrap();
        std::fs::write(dir.join("tex").join("Head.png"), png(&samples::checker_texture(16, 8, 2))).unwrap();
        std::fs::write(dir.join("unrelated.png"), b"never referenced").unwrap();

        let path = dir.join("Robot Model.obj");
        let model = import_from_path(&path, &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(model.name, "Robot Model");
        assert_eq!(model.mesh.sets.len(), 2);
        assert!(model.warnings.is_empty(), "{:?}", model.warnings);
        assert_eq!(model.textures[0].image.as_ref().unwrap(), &samples::checker_texture(32, 32, 4));
        assert_eq!(model.textures[1].image.as_ref().unwrap(), &samples::checker_texture(16, 8, 2));

        assert_eq!(model.bundle.format, ModelFormat::Obj);
        assert_eq!(model.bundle.main.key, "robot model.obj");
        assert_eq!(model.bundle.main.original_name, "Robot Model.obj");
        assert_eq!(&*model.bundle.main.bytes, obj.as_bytes());
        assert!(model.bundle.extras.is_empty());
        let keys: Vec<&str> = model.bundle.sidecars.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["robot.mtl", "body.png", "tex/head.png"]);
        let original = model.bundle.original_path.clone().unwrap();
        assert!(Path::new(&original).is_absolute(), "{original}");
        assert!(original.ends_with("Robot Model.obj"));

        // Self-containment: once the folder is gone the bundle alone reproduces the import.
        std::fs::remove_dir_all(&dir).unwrap();
        let again = import_from_bundle(&model.bundle, &model.options, true, &NoProgress).unwrap();
        assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);
        assert_eq!(again.name, model.name);
        assert_eq!(again.textures[1].image, model.textures[1].image);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    }

    #[test]
    fn every_referenced_file_is_captured_and_mtl_textures_resolve_next_to_the_mtl() {
        let dir = temp_dir("import-path-all-maps");
        std::fs::create_dir_all(dir.join("mats")).unwrap();
        let (obj, _) = samples::to_obj(&samples::flat_quad(), "mats/model.mtl", &[]);
        let mtl = "newmtl Quad\nKd 1 1 1\nmap_Kd diffuse.png\nmap_Bump -bm 0.5 normal.png\nnorm n2.png\n\
                   map_Ks \"spec map.png\"\nmap_d gone.png\nnewmtl Unused\nmap_Kd unused.png\n";
        std::fs::write(dir.join("model.obj"), &obj).unwrap();
        std::fs::write(dir.join("mats").join("model.mtl"), mtl).unwrap();
        let diffuse = samples::checker_texture(16, 16, 2);
        std::fs::write(dir.join("mats").join("diffuse.png"), png(&diffuse)).unwrap();
        // A file of the same name next to the OBJ loses against the one next to the MTL.
        std::fs::write(dir.join("diffuse.png"), png(&samples::checker_texture(8, 8, 2))).unwrap();
        for name in ["normal.png", "n2.png", "spec map.png", "unused.png"] {
            std::fs::write(dir.join("mats").join(name), format!("bytes of {name}")).unwrap();
        }
        let model = import_from_path(&dir.join("model.obj"), &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(model.warnings, [ImportWarning::NormalsGenerated]);
        assert_eq!(model.textures[0].image.as_ref(), Some(&diffuse));
        let keys: Vec<&str> = model.bundle.sidecars.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            ["mats/model.mtl", "mats/diffuse.png", "mats/normal.png", "mats/n2.png", "mats/spec map.png", "mats/unused.png"]
        );
        let normal = model.bundle.sidecars.iter().find(|s| s.key == "mats/normal.png").unwrap();
        assert_eq!((normal.original_name.as_str(), &*normal.bytes), ("normal.png", &b"bytes of normal.png"[..]));
        // The bundle alone reproduces the import, the extra maps change nothing about it.
        std::fs::remove_dir_all(&dir).unwrap();
        let again = import_from_bundle(&model.bundle, &model.options, true, &NoProgress).unwrap();
        assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);
        assert_eq!(again.textures[0].image, model.textures[0].image);
        assert_eq!(again.textures[0].reference, model.textures[0].reference);
        assert_eq!(again.warnings, [ImportWarning::NormalsGenerated]);
        let again = import_from_bundle(&model.bundle, &model.options, false, &NoProgress).unwrap();
        assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);

        // glTF: every external image is captured, whichever material slot uses it.
        let dir = temp_dir("import-path-all-images");
        std::fs::create_dir_all(dir.join("tex")).unwrap();
        let mut bin = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            bin.extend(p.iter().flat_map(|v| v.to_le_bytes()));
        }
        for uv in [[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]] {
            bin.extend(uv.iter().flat_map(|v| v.to_le_bytes()));
        }
        bin.extend([0u16, 1, 2, 0, 2, 3].iter().flat_map(|v| v.to_le_bytes()));
        let gltf = r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":92,"uri":"model.bin"}],
            "bufferViews":[{"buffer":0,"byteLength":48},{"buffer":0,"byteOffset":48,"byteLength":32},
                           {"buffer":0,"byteOffset":80,"byteLength":12}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":4,"min":[0,0,0],"max":[1,1,0]},
                         {"bufferView":1,"componentType":5126,"type":"VEC2","count":4},
                         {"bufferView":2,"componentType":5123,"type":"SCALAR","count":6}],
            "images":[{"uri":"tex/base.png"},{"uri":"tex/normal.png"},{"uri":"tex/emit%20map.png"},{"uri":"tex/gone.png"}],
            "textures":[{"source":0},{"source":1},{"source":2}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}},"normalTexture":{"index":1},
                          "emissiveTexture":{"index":2}}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],
            "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"#;
        std::fs::write(dir.join("model.gltf"), gltf).unwrap();
        std::fs::write(dir.join("model.bin"), &bin).unwrap();
        std::fs::write(dir.join("tex").join("base.png"), png(&diffuse)).unwrap();
        std::fs::write(dir.join("tex").join("normal.png"), b"normal").unwrap();
        std::fs::write(dir.join("tex").join("emit map.png"), b"emissive").unwrap();
        let model = import_from_path(&dir.join("model.gltf"), &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(model.warnings, [ImportWarning::NormalsGenerated]);
        let keys: Vec<&str> = model.bundle.sidecars.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["model.bin", "tex/base.png", "tex/normal.png", "tex/emit map.png"]);
        std::fs::remove_dir_all(&dir).unwrap();
        let again = import_from_bundle(&model.bundle, &model.options, true, &NoProgress).unwrap();
        assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);
        assert_eq!(again.textures[0].image.as_ref(), Some(&diffuse));
        assert_eq!(again.warnings, [ImportWarning::NormalsGenerated]);
    }

    #[test]
    fn import_from_path_errors() {
        let dir = temp_dir("import-path-errors");
        assert!(matches!(
            import_from_path(&dir.join("missing.obj"), &ImportOptions::default(), &NoProgress),
            Err(ImportError::Io(_))
        ));
        std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
        assert!(matches!(
            import_from_path(&dir.join("notes.txt"), &ImportOptions::default(), &NoProgress),
            Err(ImportError::UnknownFormat)
        ));
        // A missing MTL and missing textures are warnings, never errors.
        let (obj, _) = samples::to_obj(&samples::seam_cube(1.0), "gone.mtl", &[]);
        std::fs::write(dir.join("cube.obj"), obj).unwrap();
        let model = import_from_path(&dir.join("cube.obj"), &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(model.warnings, [ImportWarning::MissingMtl("gone.mtl".to_owned())]);
        assert!(model.bundle.sidecars.is_empty());
        assert_eq!(model.mesh.sets.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct Recorder {
        cancel: AtomicBool,
        calls: AtomicU32,
        fractions: Mutex<Vec<f32>>,
    }

    impl Progress for Recorder {
        fn report(&self, fraction: f32, _stage: &str) {
            self.fractions.lock().unwrap().push(fraction);
        }

        fn cancelled(&self) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.cancel.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn progress_is_monotone_and_cancellation_is_honoured() {
        let bundle = two_material_bundle(true);
        let progress =
            Recorder { cancel: AtomicBool::new(false), calls: AtomicU32::new(0), fractions: Mutex::new(Vec::new()) };
        import_from_bundle(&bundle, &ImportOptions::default(), true, &progress).unwrap();
        let fractions = progress.fractions.lock().unwrap().clone();
        assert!(fractions.len() >= 3);
        assert!(fractions.windows(2).all(|w| w[0] <= w[1]), "{fractions:?}");
        assert!(fractions.iter().all(|f| (0.0..=1.0).contains(f)));
        assert_eq!(*fractions.last().unwrap(), 1.0);
        assert!(progress.calls.load(Ordering::Relaxed) >= 2);

        progress.cancel.store(true, Ordering::Relaxed);
        assert!(matches!(
            import_from_bundle(&bundle, &ImportOptions::default(), true, &progress),
            Err(ImportError::Cancelled)
        ));
    }

    /// The Khronos sample folder used by the compatibility sweep; tests reading it are skipped when it is missing.
    const GLB_SAMPLES: &str = r"E:\claude_downloads\glb_samples";

    fn sample_file(dir: &str, name: &str) -> Option<std::path::PathBuf> {
        let path = Path::new(dir).join(name);
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("skipped: {} is not available", path.display());
            None
        }
    }

    /// R6: models without UVs (BrainStem: skinned, OrientationTest) open with the generated layout when the import
    /// asks for automatic UVs, and the skinned mesh comes out posed by its joints, not exploded.
    #[test]
    fn sample_models_without_uvs_open_with_generated_uvs() {
        for name in ["BrainStem.glb", "OrientationTest.glb"] {
            let Some(path) = sample_file(GLB_SAMPLES, name) else { continue };
            let start = std::time::Instant::now();
            let plain = import_from_path(&path, &ImportOptions::default(), &NoProgress).unwrap();
            assert!(plain.report.no_uvs, "{name}: expected a model without UVs");
            assert!(plain.mesh.sets.is_empty());
            let model = import_from_path_auto_uvs(&path, &ImportOptions::default(), true, &NoProgress).unwrap();
            eprintln!(
                "{name}: {} triangles, {} set(s), {:?} warnings {:?} ({:?})",
                model.mesh.tri_count(),
                model.mesh.sets.len(),
                model.report,
                model.warnings,
                start.elapsed()
            );
            assert!(model.report.fallback_uvs_generated && model.options.generate_fallback_uvs);
            assert_eq!(model.mesh.sets.len(), 1, "{name}");
            assert!(model.warnings.contains(&ImportWarning::FallbackUvsGenerated));
            assert!(!model.warnings.iter().any(|w| matches!(w, ImportWarning::NoUvs { .. })), "{:?}", model.warnings);
            let map = crate::texel_map::TexelMap::build(&model.mesh, crate::ids::TextureSetId(0), 1024, 1024, 4);
            eprintln!("{name}: {:?} bounds {:?}", map.stats, model.mesh.bounds);
            assert_eq!(map.stats.overlapped_texels, 0, "{name}");
            assert!(map.stats.coverage > 0.2, "{name}: coverage {}", map.stats.coverage);
            // The project re-import from the bundle reproduces the generated layout exactly.
            let again = import_from_bundle(&model.bundle, &model.options, false, &NoProgress).unwrap();
            assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);
        }
    }

    /// Timing of the import phases of one model (`WTE_PROFILE_MODEL=<path>`; run with `--ignored --nocapture`).
    #[test]
    #[ignore]
    fn profile_import_phases() {
        let Ok(path) = std::env::var("WTE_PROFILE_MODEL") else { return };
        let path = Path::new(&path);
        let t = std::time::Instant::now();
        let bytes = std::fs::read(path).unwrap();
        eprintln!("read {} bytes: {:?}", bytes.len(), t.elapsed());
        let format = sniff_model_format(&bytes, &path.to_string_lossy()).unwrap();
        let resolver = DiskResolver { base_dir: path.parent().unwrap().to_path_buf() };
        let opts = ImportOptions::default();
        let load = |decode: bool| match format {
            ModelFormat::Obj => load_obj(&bytes, &resolver, &opts, decode),
            ModelFormat::Gltf | ModelFormat::Glb => load_gltf(&bytes, &resolver, &opts, decode),
            ModelFormat::Fbx => load_fbx(&bytes, &resolver, &opts, decode),
        };
        let t = std::time::Instant::now();
        let raw = load(false).unwrap();
        eprintln!("parse without textures: {:?} ({} triangles)", t.elapsed(), raw.mesh.triangles.len());
        let t = std::time::Instant::now();
        let raw = load(true).unwrap();
        eprintln!("parse with textures: {:?} ({} textures)", t.elapsed(), raw.textures.len());
        let t = std::time::Instant::now();
        let (mesh, _) = normalize(raw.mesh, &NormalizeOptions::default());
        eprintln!("normalize: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let model = import_from_path(path, &opts, &NoProgress).unwrap();
        eprintln!("import_from_path: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        for set in 0..mesh.sets.len() as u32 {
            let _ = crate::mesh_build::uv_layout_image(&model.mesh, crate::ids::TextureSetId(set), 256);
        }
        eprintln!("uv thumbnails: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        for set in 0..mesh.sets.len() as u32 {
            let _ = crate::texel_map::TexelMap::build(&model.mesh, crate::ids::TextureSetId(set), 256, 256, 0);
        }
        eprintln!("uv stats: {:?}", t.elapsed());
    }

    #[test]
    fn imported_model_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ImportedModel>();
        assert_send_sync::<SourceBundle>();
    }

    #[test]
    fn text_decoding() {
        assert_eq!(decode_text(b"\xef\xbb\xbfv 1 2 3"), "v 1 2 3");
        assert_eq!(decode_text("caf\u{e9}".as_bytes()), "caf\u{e9}");
        // Latin-1 bytes that are not valid UTF-8 keep their letters.
        assert_eq!(decode_text(b"K\xfcche.png"), "K\u{fc}che.png");
        assert_eq!(decode_text(b""), "");
        // Every line-ending convention (and a mix of them) becomes `\n`.
        assert_eq!(decode_text(b"a\r\nb\rc\nd\r\r\ne\r"), "a\nb\nc\nd\n\ne\n");
        assert_eq!(decode_text(b"K\xfcche\rx"), "K\u{fc}che\nx");
        // UTF-16 with a byte order mark (Notepad "Unicode" / "Unicode big endian"), CRLF included.
        let le: Vec<u8> =
            [0xFF, 0xFE].into_iter().chain("v 1 2 3\r\nf 1 2 3\u{e9}".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode_text(&le), "v 1 2 3\nf 1 2 3\u{e9}");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("v 1\r2".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&be), "v 1\n2");
        // A lone surrogate and an odd trailing byte do not panic.
        assert_eq!(decode_text(&[0xFF, 0xFE, 0x00, 0xD8, b'a', 0, 7]), "\u{fffd}a");
    }

    /// The same OBJ + MTL written with LF, CRLF, CR-only (classic Mac OS) and mixed line endings imports identically.
    #[test]
    fn obj_and_mtl_with_any_line_endings_import_identically() {
        let raw = samples::two_material_model();
        let (obj, mtl) = samples::to_obj(&raw, "Robot.mtl", &["Body.png".to_owned(), "tex/Head.png".to_owned()]);
        let convert = |text: &str, ending: &str| -> String {
            match ending {
                "mixed" => text.split('\n').enumerate().map(|(i, l)| format!("{l}{}", ["\r", "\r\n", "\n"][i % 3])).collect(),
                ending => text.replace('\n', ending),
            }
        };
        let import = |ending: &str| {
            let mut bundle = two_material_bundle(true);
            bundle.main.bytes = Arc::from(convert(&obj, ending).as_bytes());
            bundle.sidecars[0].bytes = Arc::from(convert(&mtl, ending).as_bytes());
            import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap()
        };
        let reference = import("\n");
        for ending in ["\r\n", "\r", "mixed"] {
            let model = import(ending);
            assert_eq!(model.mesh.tri_count(), reference.mesh.tri_count(), "{ending:?}");
            assert_eq!(model.mesh.uv_hash, reference.mesh.uv_hash, "{ending:?}");
            assert_eq!(model.mesh.positions, reference.mesh.positions, "{ending:?}");
            assert_eq!(model.mesh.sets, reference.mesh.sets, "{ending:?}");
            assert!(model.textures.iter().all(|t| t.image.is_some()), "{ending:?}: the MTL textures resolve");
            assert!(model.warnings.is_empty(), "{ending:?}: {:?}", model.warnings);
        }
    }

    /// F14 (FEATURE_BRUSH 8.1): the geometry import of a brush shape does not need the textures the MTL names, and
    /// gives the same triangles as the full import.
    #[test]
    fn geometry_import_ignores_missing_textures() {
        let dir = temp_dir("import-geometry");
        let raw = samples::two_material_model();
        let (obj, mtl) = samples::to_obj(&raw, "robot.mtl", &["missing_body.png".to_owned(), "missing_head.png".to_owned()]);
        std::fs::write(dir.join("robot.obj"), &obj).unwrap();
        std::fs::write(dir.join("robot.mtl"), &mtl).unwrap();
        let path = dir.join("robot.obj");
        let mesh = import_geometry_from_path(&path, &NoProgress).unwrap();
        let full = import_from_path(&path, &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(mesh.tri_count(), full.mesh.tri_count());
        assert_eq!(mesh.positions, full.mesh.positions);
        assert_eq!(mesh.indices, full.mesh.indices);
        // A cancelled progress stops before reading; a missing file is an I/O error.
        struct Stop;
        impl Progress for Stop {
            fn report(&self, _: f32, _: &str) {}
            fn cancelled(&self) -> bool {
                true
            }
        }
        assert!(matches!(import_geometry_from_path(&path, &Stop), Err(ImportError::Cancelled)));
        assert!(import_geometry_from_path(&dir.join("nothing.obj"), &NoProgress).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
