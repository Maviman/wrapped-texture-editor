// Owner: core-import (C5). glTF 2.0 / GLB -> `RawMesh` (ARCHITECTURE.md 5.21, gltf notes: section 10).
//
// The `gltf` crate parses and validates the JSON and reads the accessors, but its readers assume a well-formed
// file: an accessor of the wrong type, an empty accessor, a range outside its buffer, an image without a source
// or a GLB header with a tiny length make it panic. Everything handed to those readers is therefore checked here
// first (`check_accessor`, the raw JSON of images, the GLB header); a primitive that fails a check is skipped with
// a warning. The file system is never touched by the crate: buffers and images come from the `AssetResolver`.

use std::collections::BTreeMap;
use std::sync::Arc;

use ::gltf::accessor::{DataType, Dimensions, Item, Iter};
use ::gltf::mesh::{Mode, Semantic};
use glam::{Mat3, Mat4, Vec2, Vec3};

use crate::error::{ImportError, ImportWarning};
use crate::import::resolver::file_stem_of;
use crate::import::{
    AssetResolver, ImportOptions, RawImport, TextureRequest, TextureSource, clean_unit, load_textures, parse_data_uri,
    percent_decode,
};
use crate::math::linear_to_srgb;
use crate::mesh::{RawMaterial, RawMesh, RawTriangle};

/// Material key of primitives without a material.
const DEFAULT_MATERIAL: u32 = u32::MAX;

fn invalid(message: impl Into<String>) -> ImportError {
    ImportError::InvalidGltf(message.into())
}

fn is_data_uri(uri: &str) -> bool {
    uri.trim_start().get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
}

/// Turns a parse / validation failure of the `gltf` crate into an import error. A document that requires an
/// extension the crate was not built with is `UnsupportedGltfExtension` (whatever else is wrong with it: files
/// using such an extension usually fail other checks too, e.g. Draco accessors have no buffer view).
fn map_gltf_error(error: ::gltf::Error) -> ImportError {
    let ::gltf::Error::Validation(problems) = &error else {
        return invalid(error.to_string());
    };
    let unsupported: Vec<String> = problems
        .iter()
        .filter(|(_, problem)| *problem == ::gltf::json::validation::Error::Unsupported)
        .map(|(path, _)| {
            // The path reads `extensionsRequired[0] = "KHR_draco_mesh_compression"`.
            let text = path.as_str();
            match (text.find('"'), text.rfind('"')) {
                (Some(start), Some(end)) if end > start => text[start + 1..end].to_owned(),
                _ => text.to_owned(),
            }
        })
        .collect();
    if !unsupported.is_empty() {
        return ImportError::UnsupportedGltfExtension(unsupported);
    }
    const SHOWN: usize = 4;
    let mut message: Vec<String> =
        problems.iter().take(SHOWN).map(|(path, problem)| format!("{path}: {problem}")).collect();
    if problems.len() > SHOWN {
        message.push(format!("and {} more problem(s)", problems.len() - SHOWN));
    }
    invalid(message.join("; "))
}

/// The validation of `gltf::Gltf::from_slice`, made safe and a little more lenient.
///
/// The crate's own primitive check indexes the accessor list with the `POSITION` index before anything has checked
/// that index, so a file naming a missing accessor would panic: such indices are rejected here first. Then the full
/// validation runs (the crate's wrappers rely on every index being valid), except that a `POSITION` accessor without
/// `min` / `max` is accepted: the importer never reads those bounds and common viewers load such files. A required
/// `KHR_mesh_quantization` is accepted too (the crate refuses it by name); every other unknown required extension
/// (Draco, meshopt, ...) stays `UnsupportedGltfExtension`.
fn validate(document: &::gltf::Document) -> Result<(), ImportError> {
    use ::gltf::json::validation::{Checked, Error, Validate};
    let root = document.as_json();
    for (m, mesh) in root.meshes.iter().enumerate() {
        for (p, primitive) in mesh.primitives.iter().enumerate() {
            if let Some(index) = primitive.attributes.get(&Checked::Valid(Semantic::Positions))
                && index.value() >= root.accessors.len()
            {
                return Err(invalid(format!(
                    "meshes[{m}].primitives[{p}].attributes[\"POSITION\"]: accessor {} does not exist",
                    index.value()
                )));
            }
        }
    }
    let mut problems = Vec::new();
    root.validate(root, ::gltf::json::Path::new, &mut |path, error| problems.push((path(), error)));
    problems.retain(|(path, error)| {
        let path = path.as_str();
        let bound = path.ends_with("[\"POSITION\"].min") || path.ends_with("[\"POSITION\"].max");
        // `KHR_mesh_quantization` is read here (`read_f32`), though the crate does not know it.
        let quantization = path.starts_with("extensionsRequired[") && path.ends_with(" = \"KHR_mesh_quantization\"");
        !(*error == Error::Missing && bound) && !(*error == Error::Unsupported && quantization)
    });
    if problems.is_empty() { Ok(()) } else { Err(map_gltf_error(::gltf::Error::Validation(problems))) }
}

/// Loads every buffer of the document: the GLB chunk, a data URI or a file from the resolver. Only here is a
/// missing file fatal (`MissingSidecar`); a buffer shorter than its declared length is `BufferLength`.
fn load_buffers(
    document: &::gltf::Document,
    blob: Option<Vec<u8>>,
    resolver: &dyn AssetResolver,
) -> Result<Vec<Arc<[u8]>>, ImportError> {
    let mut blob: Option<Arc<[u8]>> = blob.map(Arc::from);
    let mut buffers = Vec::new();
    for buffer in document.buffers() {
        let index = buffer.index();
        let data: Arc<[u8]> = match buffer.source() {
            ::gltf::buffer::Source::Bin => blob
                .take()
                .ok_or_else(|| invalid(format!("buffer {index} has no URI and the file has no binary chunk for it")))?,
            ::gltf::buffer::Source::Uri(uri) if is_data_uri(uri) => match parse_data_uri(uri) {
                Some((_, bytes)) => Arc::from(bytes),
                None => return Err(invalid(format!("buffer {index} has a malformed data URI"))),
            },
            ::gltf::buffer::Source::Uri(uri) => {
                resolver.read(uri).map_err(|_| ImportError::MissingSidecar(uri.to_owned()))?
            }
        };
        if data.len() < buffer.length() {
            return Err(ImportError::BufferLength { index, expected: buffer.length(), actual: data.len() });
        }
        buffers.push(data);
    }
    Ok(buffers)
}

/// Checks that `count` elements of `elem` bytes, starting `offset` bytes into `view`, lie inside the view and
/// that the view lies inside its buffer. `count` must be at least 1.
fn check_range(
    view: &::gltf::buffer::View<'_>,
    offset: usize,
    elem: usize,
    count: usize,
    buffers: &[Arc<[u8]>],
) -> Result<(), String> {
    let id = view.index();
    let buffer_len = buffers.get(view.buffer().index()).map_or(0, |b| b.len());
    if view.offset().checked_add(view.length()).is_none_or(|end| end > buffer_len) {
        return Err(format!("buffer view {id} lies outside its buffer"));
    }
    let stride = view.stride().unwrap_or(elem);
    if stride < elem {
        return Err(format!("buffer view {id} has a stride smaller than its elements"));
    }
    let end = (count - 1).checked_mul(stride).and_then(|n| n.checked_add(offset)).and_then(|n| n.checked_add(elem));
    if end.is_none_or(|end| end > view.length()) {
        return Err(format!("accessor data lies outside buffer view {id}"));
    }
    Ok(())
}

/// Everything the accessor readers of the `gltf` crate take for granted: the element type, a non-zero count and
/// in-range data (dense and sparse parts).
fn check_accessor(
    accessor: &::gltf::Accessor<'_>,
    dimensions: Dimensions,
    types: &[DataType],
    buffers: &[Arc<[u8]>],
) -> Result<(), String> {
    let id = accessor.index();
    if accessor.dimensions() != dimensions || !types.contains(&accessor.data_type()) {
        return Err(format!(
            "accessor {id} has the unsupported type {:?} {:?}",
            accessor.data_type(),
            accessor.dimensions()
        ));
    }
    let count = accessor.count();
    if count == 0 {
        return Err(format!("accessor {id} is empty"));
    }
    let elem = accessor.size();
    let sparse = accessor.sparse();
    match accessor.view() {
        Some(view) => check_range(&view, accessor.offset(), elem, count, buffers)?,
        None if sparse.is_none() => return Err(format!("accessor {id} has no data")),
        None => {}
    }
    if let Some(sparse) = sparse {
        let changed = sparse.count();
        if changed == 0 || changed > count {
            return Err(format!("accessor {id} has an invalid sparse count"));
        }
        let indices = sparse.indices();
        check_range(&indices.view(), indices.offset(), indices.index_type().size(), changed, buffers)?;
        let values = sparse.values();
        check_range(&values.view(), values.offset(), elem, changed, buffers)?;
    }
    Ok(())
}

/// Component types of the vertex attributes the importer reads in a file that declares `KHR_mesh_quantization`
/// (in `extensionsUsed` or `extensionsRequired`): float and the integer types of the extension. The tangents the
/// extension also covers are never read.
const POSITION_TYPES: [DataType; 5] = [DataType::F32, DataType::I8, DataType::U8, DataType::I16, DataType::U16];
const NORMAL_TYPES: [DataType; 3] = [DataType::F32, DataType::I8, DataType::I16];
const TEXCOORD_TYPES: [DataType; 5] = [DataType::F32, DataType::I8, DataType::U8, DataType::I16, DataType::U16];
/// The same for a core glTF file (no `KHR_mesh_quantization`): float positions and normals, float or unsigned
/// integer UVs, which are always read as normalized (as before the extension was supported).
const CORE_POSITION_TYPES: [DataType; 1] = [DataType::F32];
const CORE_NORMAL_TYPES: [DataType; 1] = [DataType::F32];
const CORE_TEXCOORD_TYPES: [DataType; 3] = [DataType::F32, DataType::U8, DataType::U16];

/// True when the document declares `KHR_mesh_quantization`; only then are non-normalized integer attributes read
/// with their raw values.
fn declares_quantization(document: &::gltf::Document) -> bool {
    let root = document.as_json();
    root.extensions_used.iter().chain(&root.extensions_required).any(|name| name == "KHR_mesh_quantization")
}

/// Reads every element of `accessor` (dense and sparse parts) with the component type `T`, converting each component.
fn read_as<'a, 's, T: Item + Copy, const N: usize>(
    accessor: &::gltf::Accessor<'a>,
    buffers: &'s [Arc<[u8]>],
    convert: impl Fn(T) -> f32,
) -> Option<Vec<[f32; N]>>
where
    [T; N]: Item,
{
    let data = |buffer: ::gltf::Buffer<'a>| buffers.get(buffer.index()).map(|data| &data[..]);
    Some(Iter::<'s, [T; N]>::new(accessor.clone(), data)?.map(|value| value.map(&convert)).collect())
}

/// Reads a float or integer accessor as floats (`KHR_mesh_quantization`). A normalized integer becomes
/// `c / MAX` (unsigned, 0..1) or `max(c / MAX, -1)` (signed), as glTF 2.0 defines; any other integer keeps its value
/// (the node transform or `KHR_texture_transform` scales it back). The accessor must have passed `check_accessor`
/// with `N` components; `None` for a 32-bit integer accessor. Without `quantized` (a core glTF file) every integer
/// is read as normalized, whatever its flag says.
fn read_f32<const N: usize>(
    accessor: &::gltf::Accessor<'_>,
    buffers: &[Arc<[u8]>],
    quantized: bool,
) -> Option<Vec<[f32; N]>>
where
    [f32; N]: Item,
    [i8; N]: Item,
    [u8; N]: Item,
    [i16; N]: Item,
    [u16; N]: Item,
{
    let normalized = accessor.normalized() || !quantized;
    match accessor.data_type() {
        DataType::F32 => read_as::<f32, N>(accessor, buffers, |c| c),
        DataType::I8 if normalized => read_as::<i8, N>(accessor, buffers, |c| (f32::from(c) / 127.0).max(-1.0)),
        DataType::U8 if normalized => read_as::<u8, N>(accessor, buffers, |c| f32::from(c) / 255.0),
        DataType::I16 if normalized => read_as::<i16, N>(accessor, buffers, |c| (f32::from(c) / 32767.0).max(-1.0)),
        DataType::U16 if normalized => read_as::<u16, N>(accessor, buffers, |c| f32::from(c) / 65535.0),
        DataType::I8 => read_as::<i8, N>(accessor, buffers, f32::from),
        DataType::U8 => read_as::<u8, N>(accessor, buffers, f32::from),
        DataType::I16 => read_as::<i16, N>(accessor, buffers, f32::from),
        DataType::U16 => read_as::<u16, N>(accessor, buffers, f32::from),
        DataType::U32 => None,
    }
}

/// `KHR_texture_transform`: `uv' = offset + R(rotation) * (scale * uv)`, with the rotation matrix used by the
/// reference viewers (`u' = cos*u + sin*v`, `v' = -sin*u + cos*v`: the image turns clockwise on a v-down texture).
#[derive(Copy, Clone, PartialEq, Debug)]
struct UvTransform {
    offset: Vec2,
    rotation: f32,
    scale: Vec2,
}

impl UvTransform {
    fn apply(&self, uv: Vec2) -> Vec2 {
        let s = uv * self.scale;
        if self.rotation == 0.0 {
            return s + self.offset;
        }
        let (sin, cos) = self.rotation.sin_cos();
        Vec2::new(cos * s.x + sin * s.y, -sin * s.x + cos * s.y) + self.offset
    }
}

/// Identity of a texture for set sharing: materials showing the same image share one texture index.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum TextureKey {
    /// Index into the document's images.
    Image(usize),
    /// A texture without an image source the crate can see (e.g. only a Basis / WebP extension source).
    NoSource(usize),
}

/// The facts of one glTF material the importer uses.
struct MaterialInfo {
    name: String,
    /// sRGB-encoded.
    base_color: [f32; 4],
    double_sided: bool,
    texture: Option<TextureKey>,
    /// Name of the glTF texture object (fallback name of an embedded image).
    texture_name: String,
    uv_set: u32,
    transform: Option<UvTransform>,
}

fn material_info(material: &::gltf::Material<'_>) -> MaterialInfo {
    let pbr = material.pbr_metallic_roughness();
    let legacy = material.pbr_specular_glossiness();
    let info = pbr.base_color_texture().or_else(|| legacy.as_ref().and_then(|l| l.diffuse_texture()));
    // The base colour factor is linear; `MaterialSlot::base_color` is sRGB-encoded.
    let factor = match (&legacy, pbr.base_color_texture().is_some()) {
        (Some(legacy), false) => legacy.diffuse_factor(),
        _ => pbr.base_color_factor(),
    };
    let encode = |c: f32| linear_to_srgb(clean_unit(c, 0.8)) as f32 / 255.0;
    let mut out = MaterialInfo {
        name: material.name().unwrap_or("").to_owned(),
        base_color: [encode(factor[0]), encode(factor[1]), encode(factor[2]), clean_unit(factor[3], 1.0)],
        double_sided: material.double_sided(),
        texture: None,
        texture_name: String::new(),
        uv_set: 0,
        transform: None,
    };
    if let Some(info) = info {
        let texture = info.texture();
        out.texture = Some(match texture.source() {
            Some(image) => TextureKey::Image(image.index()),
            None => TextureKey::NoSource(texture.index()),
        });
        out.texture_name = texture.name().unwrap_or("").to_owned();
        out.uv_set = info.tex_coord();
        if let Some(transform) = info.texture_transform() {
            out.uv_set = transform.tex_coord().unwrap_or(out.uv_set);
            let t = UvTransform {
                offset: Vec2::from(transform.offset()),
                rotation: transform.rotation(),
                scale: Vec2::from(transform.scale()),
            };
            let identity = UvTransform { offset: Vec2::ZERO, rotation: 0.0, scale: Vec2::ONE };
            if t != identity && t.offset.is_finite() && t.rotation.is_finite() && t.scale.is_finite() {
                out.transform = Some(t);
            }
        }
    }
    out
}

/// `name` without a trailing image-file extension ("wood.png" -> "wood", "Material.001" stays).
fn strip_image_extension(name: &str) -> &str {
    const EXTENSIONS: [&str; 9] = ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "webp", "dds"];
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)) => stem,
        _ => name,
    }
}

fn mime_hint(mime: Option<&str>) -> Option<String> {
    let extension = match mime?.trim().to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tif",
        "image/x-tga" | "image/x-targa" | "image/tga" => "tga",
        "image/vnd-ms.dds" | "image/vnd.ms-dds" | "image/x-dds" => "dds",
        _ => return None,
    };
    Some(format!("image.{extension}"))
}

/// Describes where the pixels of `key` come from. Works on the raw JSON of the image, because
/// `gltf::Image::source()` panics on an image that has neither a URI nor a complete buffer-view description.
fn texture_request(
    document: &::gltf::Document,
    key: TextureKey,
    material: &MaterialInfo,
    buffers: &[Arc<[u8]>],
) -> TextureRequest {
    let embedded_name = |own: Option<&str>| -> String {
        let name = own.filter(|n| !n.trim().is_empty()).unwrap_or(&material.texture_name);
        strip_image_extension(name.trim()).to_owned()
    };
    let embedded = |own: Option<&str>, index: usize, hint: Option<String>, source: TextureSource| {
        let name = embedded_name(own);
        let reference = if name.is_empty() { format!("embedded image {index}") } else { name.clone() };
        TextureRequest { name, reference, material: material.name.clone(), name_hint: hint, source }
    };
    let index = match key {
        TextureKey::Image(index) => index,
        TextureKey::NoSource(texture) => return embedded(None, texture, None, TextureSource::Missing),
    };
    let Some(image) = document.as_json().images.get(index) else {
        return embedded(None, index, None, TextureSource::Missing);
    };
    let own_name = image.name.as_deref();
    let mime = image.mime_type.as_ref().map(|m| m.0.as_str());
    if let Some(view) = image.buffer_view.and_then(|v| document.views().nth(v.value())) {
        let source = match (buffers.get(view.buffer().index()), view.offset().checked_add(view.length())) {
            (Some(buffer), Some(end)) if end <= buffer.len() => {
                TextureSource::Slice { buffer: buffer.clone(), range: view.offset()..end }
            }
            _ => TextureSource::Broken("the image data lies outside its buffer".to_owned()),
        };
        return embedded(own_name, index, mime_hint(mime), source);
    }
    match image.uri.as_deref() {
        Some(uri) if is_data_uri(uri) => match parse_data_uri(uri) {
            Some((uri_mime, bytes)) => {
                embedded(own_name, index, mime_hint(mime.or(uri_mime)), TextureSource::Bytes(Arc::from(bytes)))
            }
            None => embedded(own_name, index, None, TextureSource::Broken("malformed data URI".to_owned())),
        },
        Some(uri) => {
            let decoded = percent_decode(uri);
            TextureRequest {
                name: file_stem_of(&decoded).to_owned(),
                reference: uri.to_owned(),
                material: material.name.clone(),
                name_hint: Some(decoded),
                source: TextureSource::Resolver,
            }
        }
        None => embedded(own_name, index, None, TextureSource::Missing),
    }
}

/// Positions into the index list of every triangle of a primitive with `count` indices, front faces
/// counter-clockwise (strips alternate, fans share the first vertex).
fn triangle_corners(mode: Mode, count: usize) -> Vec<[usize; 3]> {
    match mode {
        Mode::Triangles => (0..count / 3).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect(),
        Mode::TriangleStrip => (0..count.saturating_sub(2))
            .map(|i| if i % 2 == 0 { [i, i + 1, i + 2] } else { [i, i + 2, i + 1] })
            .collect(),
        Mode::TriangleFan => (0..count.saturating_sub(2)).map(|i| [0, i + 1, i + 2]).collect(),
        _ => Vec::new(),
    }
}

/// Working state of one import.
struct Loader<'a> {
    buffers: &'a [Arc<[u8]>],
    triangles: Vec<RawTriangle>,
    /// glTF material index (or `DEFAULT_MATERIAL`) -> its facts, for every material used by a triangle.
    materials: BTreeMap<u32, MaterialInfo>,
    warnings: Vec<ImportWarning>,
    skin_ignored: bool,
    /// The document declares `KHR_mesh_quantization` (`declares_quantization`).
    quantized: bool,
}

impl Loader<'_> {
    fn skip(&mut self, mesh: &::gltf::Mesh<'_>, primitive: usize, reason: impl std::fmt::Display) {
        let mesh_name = match mesh.name() {
            Some(name) if !name.is_empty() => format!("\"{name}\""),
            _ => format!("{}", mesh.index()),
        };
        self.warnings.push(ImportWarning::SkippedPrimitive {
            reason: format!("mesh {mesh_name}, primitive {primitive}: {reason}"),
        });
    }

    /// Reads the skin weights of a primitive (`JOINTS_0` / `WEIGHTS_0`) and blends `joint_matrices` into one matrix
    /// per vertex. `None` when the primitive has no usable weights (the caller then uses the node transform).
    fn skin_matrices(
        &self,
        primitive: &::gltf::Primitive<'_>,
        joint_matrices: &[Mat4],
        vertex_count: usize,
    ) -> Option<Vec<Mat4>> {
        let buffers = self.buffers;
        let joints = primitive.get(&Semantic::Joints(0))?;
        let weights = primitive.get(&Semantic::Weights(0))?;
        check_accessor(&joints, Dimensions::Vec4, &[DataType::U8, DataType::U16], buffers).ok()?;
        check_accessor(&weights, Dimensions::Vec4, &[DataType::F32, DataType::U8, DataType::U16], buffers).ok()?;
        let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| &data[..]));
        let joints: Vec<[u16; 4]> = reader.read_joints(0)?.into_u16().collect();
        let weights: Vec<[f32; 4]> = reader.read_weights(0)?.into_f32().collect();
        if joints.len() != vertex_count || weights.len() != vertex_count {
            return None;
        }
        Some(
            joints
                .iter()
                .zip(&weights)
                .map(|(j, w)| {
                    let mut blend = Mat4::ZERO;
                    let mut total = 0.0f32;
                    for k in 0..4 {
                        let weight = w[k];
                        if !(weight.is_finite() && weight > 0.0) {
                            continue;
                        }
                        // A joint index outside the skin contributes nothing (the weights are renormalized).
                        if let Some(m) = joint_matrices.get(j[k] as usize) {
                            blend += *m * weight;
                            total += weight;
                        }
                    }
                    if total > 1e-6 { blend * (1.0 / total) } else { Mat4::IDENTITY }
                })
                .collect(),
        )
    }

    /// Appends the triangles of one mesh instance, with `world` baked into positions and normals. With
    /// `joint_matrices` (a skinned node: `joint world * inverse bind matrix` per joint) every vertex is placed by
    /// its skin weights instead, and the node's own transform is ignored, as glTF requires for skinned meshes.
    fn add_mesh(&mut self, mesh: &::gltf::Mesh<'_>, world: Mat4, joint_matrices: Option<&[Mat4]>) {
        let linear = Mat3::from_mat4(world);
        let determinant = linear.determinant();
        let flip = world.determinant() < 0.0;
        // Normals transform with the inverse transpose; a singular transform leaves them to be regenerated.
        let normal_matrix = Some(linear.inverse().transpose())
            .filter(|m| determinant.is_finite() && determinant != 0.0 && m.is_finite());
        let buffers = self.buffers;

        for primitive in mesh.primitives() {
            let number = primitive.index();
            let mode = primitive.mode();
            if !matches!(mode, Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan) {
                self.skip(mesh, number, format_args!("{mode:?} primitives are not surfaces"));
                continue;
            }
            if primitive.morph_targets().next().is_some() {
                self.skin_ignored = true;
            }
            let Some(position_accessor) = primitive.get(&Semantic::Positions) else {
                self.skip(mesh, number, "it has no positions");
                continue;
            };
            let quantized = self.quantized;
            let (position_types, normal_types, texcoord_types): (&[DataType], &[DataType], &[DataType]) = if quantized {
                (&POSITION_TYPES, &NORMAL_TYPES, &TEXCOORD_TYPES)
            } else {
                (&CORE_POSITION_TYPES, &CORE_NORMAL_TYPES, &CORE_TEXCOORD_TYPES)
            };
            if let Err(reason) = check_accessor(&position_accessor, Dimensions::Vec3, position_types, buffers) {
                self.skip(mesh, number, reason);
                continue;
            }
            let index_accessor = primitive.indices();
            if let Some(accessor) = &index_accessor {
                let types = [DataType::U8, DataType::U16, DataType::U32];
                if let Err(reason) = check_accessor(accessor, Dimensions::Scalar, &types, buffers) {
                    self.skip(mesh, number, reason);
                    continue;
                }
            }

            let material = primitive.material();
            let key = material.index().map_or(DEFAULT_MATERIAL, |i| i as u32);
            let info = self.materials.entry(key).or_insert_with(|| material_info(&material));
            let (uv_set, transform) = (info.uv_set, info.transform);

            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| &data[..]));
            let local: Vec<Vec3> = read_f32::<3>(&position_accessor, buffers, quantized)
                .map(|values| values.into_iter().map(Vec3::from).collect())
                .unwrap_or_default();
            // Skinned: one blended matrix per vertex; a primitive without usable weights keeps the node transform.
            let skin = joint_matrices.and_then(|joints| self.skin_matrices(&primitive, joints, local.len()));
            if joint_matrices.is_some() && skin.is_none() {
                self.skin_ignored = true;
            }
            let positions: Vec<Vec3> = match &skin {
                Some(matrices) => local.iter().zip(matrices).map(|(p, m)| m.transform_point3(*p)).collect(),
                None => local.iter().map(|p| world.transform_point3(*p)).collect(),
            };
            // Quantized normals must be normalized integers; any other integer normal is regenerated.
            let raw_normals: Option<Vec<Vec3>> = primitive
                .get(&Semantic::Normals)
                .filter(|a| check_accessor(a, Dimensions::Vec3, normal_types, buffers).is_ok())
                .filter(|a| a.data_type() == DataType::F32 || a.normalized())
                .and_then(|a| read_f32::<3>(&a, buffers, quantized))
                .map(|values| values.into_iter().map(Vec3::from).collect::<Vec<Vec3>>())
                .filter(|normals| normals.len() == positions.len());
            let normals: Option<Vec<Vec3>> = match (&skin, raw_normals) {
                // A singular blend leaves a non-finite normal, which normalization regenerates.
                (Some(matrices), Some(raw)) => Some(
                    raw.iter().zip(matrices).map(|(n, m)| Mat3::from_mat4(*m).inverse().transpose() * *n).collect(),
                ),
                (None, Some(raw)) => normal_matrix.map(|matrix| raw.iter().map(|n| matrix * *n).collect()),
                (_, None) => None,
            };
            // Dequantized first, then `KHR_texture_transform` (which quantizing exporters use to rescale).
            let uvs: Option<Vec<Vec2>> = primitive
                .get(&Semantic::TexCoords(uv_set))
                .filter(|a| check_accessor(a, Dimensions::Vec2, texcoord_types, buffers).is_ok())
                .and_then(|a| read_f32::<2>(&a, buffers, quantized))
                .map(|values| {
                    values
                        .into_iter()
                        .map(|uv| {
                            let uv = Vec2::from(uv);
                            transform.map_or(uv, |t| t.apply(uv))
                        })
                        .collect::<Vec<Vec2>>()
                })
                .filter(|uvs| uvs.len() == positions.len());
            let indices: Option<Vec<u32>> = index_accessor
                .as_ref()
                .map(|_| reader.read_indices().map(|i| i.into_u32().collect()).unwrap_or_default());

            let index_count = indices.as_ref().map_or(positions.len(), |i| i.len());
            let vertex = |slot: usize| -> Option<usize> {
                let v = match &indices {
                    Some(list) => *list.get(slot)? as usize,
                    None => slot,
                };
                (v < positions.len()).then_some(v)
            };
            let mut dangling = 0u32;
            for corners in triangle_corners(mode, index_count) {
                let (Some(a), Some(b), Some(c)) = (vertex(corners[0]), vertex(corners[1]), vertex(corners[2])) else {
                    dangling += 1;
                    continue;
                };
                // A mirroring transform turns the triangle inside out: restore counter-clockwise front faces.
                let mirrored = match &skin {
                    Some(matrices) => matrices[a].determinant() < 0.0,
                    None => flip,
                };
                let v = if mirrored { [a, c, b] } else { [a, b, c] };
                self.triangles.push(RawTriangle {
                    positions: v.map(|i| positions[i]),
                    normals: normals.as_ref().map(|n| v.map(|i| n[i])),
                    uvs: uvs.as_ref().map(|t| v.map(|i| t[i])),
                    material: key,
                });
            }
            if dangling > 0 {
                self.skip(mesh, number, format_args!("{dangling} triangle(s) refer to vertices that do not exist"));
            }
        }
    }
}

/// The world matrix of every node from the parent links (a node reached through several parents uses the first;
/// a cycle is cut where it closes, so a broken file cannot loop).
fn node_world_matrices(nodes: &[::gltf::Node<'_>]) -> Vec<Mat4> {
    let n = nodes.len();
    let mut parent = vec![usize::MAX; n];
    for node in nodes {
        for child in node.children() {
            if child.index() < n && parent[child.index()] == usize::MAX && child.index() != node.index() {
                parent[child.index()] = node.index();
            }
        }
    }
    let local: Vec<Mat4> = nodes.iter().map(|node| Mat4::from_cols_array_2d(&node.transform().matrix())).collect();
    let mut world: Vec<Option<Mat4>> = vec![None; n];
    let mut chain: Vec<usize> = Vec::new();
    for start in 0..n {
        chain.clear();
        let mut current = start;
        // Walk up until a known world matrix, a root or a cycle (chain longer than the node count).
        while world[current].is_none() && chain.len() <= n {
            chain.push(current);
            match parent[current] {
                p if p < n => current = p,
                _ => break,
            }
        }
        let mut acc = match world[current] {
            Some(m) if chain.last() != Some(&current) => m,
            _ => Mat4::IDENTITY,
        };
        for &i in chain.iter().rev() {
            if let Some(m) = world[i] {
                acc = m;
                continue;
            }
            acc *= local[i];
            world[i] = Some(acc);
        }
    }
    world.into_iter().map(|m| m.unwrap_or(Mat4::IDENTITY)).collect()
}

/// `joint world * inverse bind matrix` per joint of a skin (the pose the file's node transforms describe; for a
/// model saved at rest this is its bind pose). `None` when the skin is unusable (no joints, a joint index or the
/// inverse bind matrices out of range, non-finite matrices).
fn joint_matrices(skin: &::gltf::Skin<'_>, node_world: &[Mat4], buffers: &[Arc<[u8]>]) -> Option<Vec<Mat4>> {
    let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
    if joints.is_empty() || joints.iter().any(|&j| j >= node_world.len()) {
        return None;
    }
    let inverse_bind: Vec<Mat4> = match skin.inverse_bind_matrices() {
        Some(accessor) => {
            check_accessor(&accessor, Dimensions::Mat4, &[DataType::F32], buffers).ok()?;
            if accessor.count() < joints.len() {
                return None;
            }
            let reader = skin.reader(|buffer| buffers.get(buffer.index()).map(|data| &data[..]));
            reader.read_inverse_bind_matrices()?.map(|m| Mat4::from_cols_array_2d(&m)).collect()
        }
        None => vec![Mat4::IDENTITY; joints.len()],
    };
    let matrices: Vec<Mat4> = joints.iter().zip(&inverse_bind).map(|(&j, ibm)| node_world[j] * *ibm).collect();
    matrices.iter().all(|m| m.is_finite()).then_some(matrices)
}

/// glTF / GLB -> `RawMesh` (node transforms baked; buffers and images through `resolver`; textures decoded only when
/// `decode_textures`).
///
/// Scene: the default scene, else the first one, else every root node; each node is visited once. Materials: the
/// glTF materials used by at least one triangle, in document order, then one unnamed material for primitives
/// without a material. The base colour texture (else the specular-glossiness diffuse texture) names the texture,
/// its `texCoord` the UV set and its `KHR_texture_transform` is applied to the UVs; materials showing the same
/// image share one texture index. With `decode_textures` every other external image file is read too (and so
/// captured by a `RecordingResolver`). UVs are used as stored (no V flip). Skins and morph targets are ignored with
/// `SkinIgnored`; point and line primitives and primitives with unusable accessors are skipped with
/// `SkippedPrimitive`.
pub(crate) fn load_gltf(
    main: &[u8],
    resolver: &dyn AssetResolver,
    opts: &ImportOptions,
    decode_textures: bool,
) -> Result<RawImport, ImportError> {
    let main = main.strip_prefix(b"\xef\xbb\xbf").unwrap_or(main);
    if main.starts_with(b"glTF") {
        // `Glb::from_slice` subtracts the header size from this length without a check.
        let declared = main.get(8..12).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        match declared {
            Some(length) if length >= 12 => {}
            Some(_) => return Err(invalid("the GLB header declares an impossible length")),
            None => return Err(invalid("the GLB header is truncated")),
        }
    }
    let ::gltf::Gltf { document, blob } = ::gltf::Gltf::from_slice_without_validation(main).map_err(map_gltf_error)?;
    validate(&document)?;
    let buffers = load_buffers(&document, blob, resolver)?;

    // Scene graph: roots, then an explicit stack; every node is visited at most once (guards against cycles).
    let nodes: Vec<::gltf::Node<'_>> = document.nodes().collect();
    let roots: Vec<usize> = match document.default_scene().or_else(|| document.scenes().next()) {
        Some(scene) => scene.nodes().map(|n| n.index()).collect(),
        None => {
            let mut is_child = vec![false; nodes.len()];
            for node in &nodes {
                for child in node.children() {
                    if let Some(flag) = is_child.get_mut(child.index()) {
                        *flag = true;
                    }
                }
            }
            (0..nodes.len()).filter(|&i| !is_child[i]).collect()
        }
    };
    let mut loader = Loader {
        buffers: &buffers,
        triangles: Vec::new(),
        materials: BTreeMap::new(),
        warnings: Vec::new(),
        skin_ignored: false,
        quantized: declares_quantization(&document),
    };
    let node_world = node_world_matrices(&nodes);
    let mut visited = vec![false; nodes.len()];
    let mut stack: Vec<(usize, Mat4)> = roots.into_iter().rev().map(|i| (i, Mat4::IDENTITY)).collect();
    while let Some((index, parent)) = stack.pop() {
        let Some(node) = nodes.get(index) else { continue };
        if std::mem::replace(&mut visited[index], true) {
            continue;
        }
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            if mesh.weights().is_some_and(|w| !w.is_empty()) {
                loader.skin_ignored = true;
            }
            let joints = node.skin().and_then(|skin| joint_matrices(&skin, &node_world, &buffers));
            if node.skin().is_some() && joints.is_none() {
                // An unusable skin: the mesh is placed by its node transform, as before skinning was supported.
                loader.skin_ignored = true;
            }
            loader.add_mesh(&mesh, world, joints.as_deref());
        }
        let children: Vec<usize> = node.children().map(|c| c.index()).collect();
        stack.extend(children.into_iter().rev().map(|c| (c, world)));
    }
    let Loader { mut triangles, materials: used, mut warnings, skin_ignored, .. } = loader;
    if skin_ignored {
        warnings.push(ImportWarning::SkinIgnored);
    }

    // Materials in document order (the default material has the largest key and comes last), textures in order
    // of first appearance by material.
    let mut dense: BTreeMap<u32, u32> = BTreeMap::new();
    let mut materials: Vec<RawMaterial> = Vec::with_capacity(used.len());
    let mut requests: Vec<TextureRequest> = Vec::new();
    let mut texture_index: BTreeMap<TextureKey, u32> = BTreeMap::new();
    for (key, info) in &used {
        let texture = info.texture.map(|texture| {
            *texture_index.entry(texture).or_insert_with(|| {
                requests.push(texture_request(&document, texture, info, &buffers));
                requests.len() as u32 - 1
            })
        });
        dense.insert(*key, materials.len() as u32);
        materials.push(RawMaterial {
            name: info.name.clone(),
            base_color: info.base_color,
            double_sided: info.double_sided,
            texture,
        });
    }
    for triangle in &mut triangles {
        triangle.material = dense.get(&triangle.material).copied().unwrap_or(0);
    }
    let textures = load_textures(requests, resolver, opts, decode_textures, &mut warnings);
    if decode_textures {
        // Every external image is part of the model (normal, metallic-roughness, occlusion, emissive maps, unused
        // images): read them so that a recording resolver captures them in the project. Their bytes are not used and
        // a file that cannot be read does not affect the import.
        for image in &document.as_json().images {
            if let Some(uri) = image.uri.as_deref().filter(|uri| !is_data_uri(uri)) {
                let _ = resolver.read(uri);
            }
        }
    }

    Ok(RawImport { mesh: RawMesh { triangles, materials }, textures, warnings })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::image_io::{PngLevel, encode_png};
    use crate::import::{
        ImportedModel, MemoryResolver, ModelFormat, SourceBundle, SourceFile, import_from_bundle, normalize_ref,
    };
    use crate::pixel::PixelBuf;
    use crate::progress::NoProgress;
    use crate::samples;

    const F32: u32 = 5126;
    const U8: u32 = 5121;
    const U16: u32 = 5123;
    const U32: u32 = 5125;

    /// The unit quad of `samples::flat_quad`: x, y in 0..1, facing +Z, u = x, v = 1 - y.
    const QUAD_POSITIONS: [[f32; 3]; 4] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    const QUAD_UVS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    const QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

    /// Collects binary data and the buffer views / accessors describing it.
    #[derive(Default)]
    struct Builder {
        bin: Vec<u8>,
        views: Vec<Value>,
        accessors: Vec<Value>,
    }

    impl Builder {
        fn view(&mut self, data: &[u8]) -> usize {
            while !self.bin.len().is_multiple_of(4) {
                self.bin.push(0);
            }
            self.views.push(json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": data.len() }));
            self.bin.extend_from_slice(data);
            self.views.len() - 1
        }

        fn accessor(&mut self, data: &[u8], component: u32, kind: &str, count: usize, normalized: bool) -> usize {
            let view = self.view(data);
            self.accessors.push(json!({
                "bufferView": view, "componentType": component, "type": kind, "count": count, "normalized": normalized,
            }));
            self.accessors.len() - 1
        }

        fn vec3(&mut self, values: &[[f32; 3]]) -> usize {
            let data: Vec<u8> = values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
            let index = self.accessor(&data, F32, "VEC3", values.len(), false);
            // POSITION accessors must declare their bounds.
            let bound = |pick: fn(f32, f32) -> f32, start: f32| -> Vec<f32> {
                (0..3).map(|k| values.iter().fold(start, |acc, v| pick(acc, v[k]))).collect()
            };
            self.accessors[index]["min"] = json!(bound(f32::min, f32::INFINITY));
            self.accessors[index]["max"] = json!(bound(f32::max, f32::NEG_INFINITY));
            index
        }

        fn vec2(&mut self, values: &[[f32; 2]]) -> usize {
            let data: Vec<u8> = values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
            self.accessor(&data, F32, "VEC2", values.len(), false)
        }

        fn indices_u16(&mut self, values: &[u16]) -> usize {
            let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.accessor(&data, U16, "SCALAR", values.len(), false)
        }

        /// The standard quad primitive: positions, normals, UV set 0 and u16 indices.
        fn quad(&mut self, offset: [f32; 3]) -> Value {
            let positions: Vec<[f32; 3]> =
                QUAD_POSITIONS.iter().map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]).collect();
            let position = self.vec3(&positions);
            let normal = self.vec3(&[[0.0, 0.0, 1.0]; 4]);
            let uv = self.vec2(&QUAD_UVS);
            let indices = self.indices_u16(&QUAD_INDICES);
            json!({ "attributes": { "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv }, "indices": indices })
        }

        /// The complete document: `extra` (meshes, nodes, materials, ...) plus asset, buffer, views and accessors.
        fn document(&self, buffer_uri: Option<&str>, extra: Value) -> Value {
            let mut buffer = json!({ "byteLength": self.bin.len() });
            if let Some(uri) = buffer_uri {
                buffer["uri"] = json!(uri);
            }
            let mut doc = json!({
                "asset": { "version": "2.0" },
                "buffers": [buffer],
                "bufferViews": self.views,
                "accessors": self.accessors,
            });
            for (key, value) in extra.as_object().unwrap() {
                doc[key] = value.clone();
            }
            doc
        }

        fn glb(&self, extra: Value) -> Vec<u8> {
            let json = serde_json::to_vec(&self.document(None, extra)).unwrap();
            let glb = ::gltf::binary::Glb {
                header: ::gltf::binary::Header { magic: *b"glTF", version: 2, length: 0 },
                json: json.into(),
                bin: Some(self.bin.as_slice().into()),
            };
            glb.to_vec().unwrap()
        }
    }

    fn base64_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                out.push(if k <= chunk.len() { ALPHABET[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    fn png(image: &PixelBuf) -> Vec<u8> {
        encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap()
    }

    fn resolver(files: &[(&str, &[u8])]) -> MemoryResolver {
        let mut resolver = MemoryResolver::default();
        for (name, bytes) in files {
            resolver.insert(name, Arc::from(*bytes));
        }
        resolver
    }

    fn load(main: &[u8], files: &[(&str, &[u8])], decode: bool) -> Result<RawImport, ImportError> {
        load_gltf(main, &resolver(files), &ImportOptions::default(), decode)
    }

    fn load_json(doc: &Value, files: &[(&str, &[u8])]) -> RawImport {
        load(&serde_json::to_vec(doc).unwrap(), files, true).unwrap()
    }

    /// The whole import (loader + normalization) from an in-memory bundle.
    fn import(format: ModelFormat, main: Vec<u8>, files: &[(&str, &[u8])]) -> ImportedModel {
        let file = |name: &str, bytes: &[u8]| SourceFile {
            key: normalize_ref(name),
            original_name: name.to_owned(),
            bytes: Arc::from(bytes),
        };
        let bundle = SourceBundle {
            format,
            main: file(&format!("Scene.{}", format.extension()), &main),
            sidecars: files.iter().map(|(name, bytes)| file(name, bytes)).collect(),
            extras: Vec::new(),
            original_path: None,
        };
        import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap()
    }

    fn single_mesh(primitives: Value) -> Value {
        json!({ "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": primitives }] })
    }

    fn assert_is_flat_quad(mesh: &RawMesh) {
        let expected = samples::flat_quad();
        assert_eq!(mesh.triangles.len(), 2);
        for (a, b) in mesh.triangles.iter().zip(&expected.triangles) {
            assert_eq!(a.positions, b.positions);
            assert_eq!(a.uvs, b.uvs, "glTF UVs are used as stored (no V flip)");
            assert_eq!(a.normals, Some([Vec3::Z; 3]));
        }
    }

    #[test]
    fn glb_with_embedded_png() {
        let texture = samples::checker_texture(16, 16, 4);
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let image_view = b.view(&png(&texture));
        let mut extra = single_mesh(json!([{ "attributes": quad["attributes"], "indices": quad["indices"], "material": 0 }]));
        extra["materials"] = json!([{
            "name": "Painted", "doubleSided": true,
            "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 }, "baseColorFactor": [1.0, 0.5, 0.0, 0.25] },
        }]);
        extra["textures"] = json!([{ "source": 0, "name": "ignored when the image has a name" }]);
        extra["images"] = json!([{ "bufferView": image_view, "mimeType": "image/png", "name": "Checker.png" }]);
        let glb = b.glb(extra);
        assert!(glb.starts_with(b"glTF"));

        let raw = load(&glb, &[], true).unwrap();
        assert!(raw.warnings.is_empty(), "{:?}", raw.warnings);
        assert_is_flat_quad(&raw.mesh);
        assert_eq!(raw.mesh.materials.len(), 1);
        let material = &raw.mesh.materials[0];
        assert_eq!(material.name, "Painted");
        assert!(material.double_sided);
        assert_eq!(material.texture, Some(0));
        // Linear factor -> sRGB-encoded colour; alpha is passed through.
        assert_eq!(material.base_color, [1.0, 188.0 / 255.0, 0.0, 0.25]);
        assert_eq!(raw.textures.len(), 1);
        assert_eq!(raw.textures[0].name, "Checker");
        assert_eq!(raw.textures[0].reference, "Checker");
        assert_eq!(raw.textures[0].image.as_ref().unwrap(), &texture);

        let model = import(ModelFormat::Glb, glb, &[]);
        assert_eq!(model.name, "Scene");
        assert_eq!(model.mesh.sets.len(), 1);
        assert_eq!(model.mesh.sets[0].name, "Checker");
        assert_eq!(model.mesh.sets[0].texture, Some(0));
    }

    #[test]
    fn gltf_with_base64_buffer() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let uri = format!("data:application/octet-stream;base64,{}", base64_encode(&b.bin));
        let doc = b.document(Some(&uri), single_mesh(json!([quad])));
        let raw = load_json(&doc, &[]);
        assert!(raw.warnings.is_empty(), "{:?}", raw.warnings);
        assert_is_flat_quad(&raw.mesh);
        // No material: one unnamed default material, no texture.
        assert_eq!(raw.mesh.materials.len(), 1);
        assert_eq!(raw.mesh.materials[0].name, "");
        assert_eq!(raw.mesh.materials[0].base_color, [1.0; 4]);
        assert!(raw.textures.is_empty());
        // The set of a material-less model is named after the file.
        let model = import(ModelFormat::Gltf, serde_json::to_vec(&doc).unwrap(), &[]);
        assert_eq!(model.mesh.sets[0].name, "Scene");

        // A malformed data URI is a broken file, not a missing one.
        let doc = b.document(Some("data:application/octet-stream;base64,@@@"), single_mesh(json!([quad])));
        assert!(matches!(load(&serde_json::to_vec(&doc).unwrap(), &[], true), Err(ImportError::InvalidGltf(_))));
    }

    #[test]
    fn gltf_with_external_bin_and_image() {
        let texture = samples::checker_texture(8, 8, 2);
        let image = png(&texture);
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let mut extra = single_mesh(json!([{ "attributes": quad["attributes"], "indices": quad["indices"], "material": 0 }]));
        extra["materials"] = json!([{ "name": "Wall", "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }]);
        extra["textures"] = json!([{ "source": 0 }]);
        extra["images"] = json!([{ "uri": "Textures/Brick%20Wall.png" }]);
        let doc = b.document(Some("Mesh%20Data.bin"), extra);
        let main = serde_json::to_vec(&doc).unwrap();
        let files: [(&str, &[u8]); 2] = [("mesh data.bin", &b.bin), ("textures\\brick wall.PNG", &image)];

        let raw = load(&main, &files, true).unwrap();
        assert!(raw.warnings.is_empty(), "{:?}", raw.warnings);
        assert_is_flat_quad(&raw.mesh);
        assert_eq!(raw.textures[0].name, "Brick Wall");
        assert_eq!(raw.textures[0].reference, "Textures/Brick%20Wall.png");
        assert_eq!(raw.textures[0].image.as_ref().unwrap(), &texture);
        let model = import(ModelFormat::Gltf, main.clone(), &files);
        assert_eq!(model.mesh.sets[0].name, "Brick Wall");

        // Without decoding nothing but the buffer is read, and the texture is still recorded.
        let lean = load(&main, &files[..1], false).unwrap();
        assert!(lean.warnings.is_empty());
        assert_eq!(lean.textures[0].name, "Brick Wall");
        assert!(lean.textures[0].image.is_none());
        // A missing image is a warning ...
        let raw = load(&main, &files[..1], true).unwrap();
        assert_eq!(
            raw.warnings,
            [ImportWarning::MissingTexture {
                material: "Wall".to_owned(),
                reference: "Textures/Brick%20Wall.png".to_owned(),
            }]
        );
        assert_eq!(raw.mesh.materials[0].texture, Some(0));
        // ... a missing or short .bin is fatal.
        match load(&main, &files[1..], true) {
            Err(ImportError::MissingSidecar(name)) => assert_eq!(name, "Mesh%20Data.bin"),
            other => panic!("expected MissingSidecar, got {:?}", other.err()),
        }
        let short = &b.bin[..b.bin.len() - 3];
        match load(&main, &[("Mesh Data.bin", short)], true) {
            Err(ImportError::BufferLength { index: 0, expected, actual }) => {
                assert_eq!((expected, actual), (b.bin.len(), b.bin.len() - 3));
            }
            other => panic!("expected BufferLength, got {:?}", other.err()),
        }
    }

    #[test]
    fn node_transforms_are_baked() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        // Parent: translation. Child: rotation of +90 degrees about Z, then a uniform scale of 2.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let doc = b.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0] }],
                "nodes": [
                    { "translation": [10.0, 20.0, 30.0], "children": [1] },
                    { "rotation": [0.0, 0.0, half, half], "scale": [2.0, 2.0, 2.0], "mesh": 0 },
                ],
                "meshes": [{ "primitives": [quad] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        let close = |a: Vec3, b: Vec3| (a - b).abs().max_element() < 1e-4;
        // (x, y, 0) -> (-2y, 2x, 0) + (10, 20, 30).
        let expected = [Vec3::new(10.0, 20.0, 30.0), Vec3::new(10.0, 22.0, 30.0), Vec3::new(8.0, 22.0, 30.0)];
        for (p, e) in raw.mesh.triangles[0].positions.iter().zip(expected) {
            assert!(close(*p, e), "{p:?} vs {e:?}");
        }
        for n in raw.mesh.triangles[0].normals.unwrap() {
            assert!(close(n.normalize(), Vec3::Z), "{n:?}");
        }
        // A matrix transform (column-major, translation in the last column) does the same job.
        let doc = b.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0] }],
                "nodes": [{ "matrix": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0], "mesh": 0 }],
                "meshes": [{ "primitives": [quad] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert_eq!(raw.mesh.triangles[0].positions[1], Vec3::new(6.0, 6.0, 7.0));
        // A non-uniform scale tilts normals with the inverse transpose.
        let mut b2 = Builder::default();
        let position = b2.vec3(&QUAD_POSITIONS);
        let normal = b2.vec3(&[[1.0, 1.0, 0.0]; 4]);
        let doc = b2.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0] }],
                "nodes": [{ "scale": [4.0, 1.0, 1.0], "mesh": 0 }],
                "meshes": [{ "primitives": [{ "attributes": { "POSITION": position, "NORMAL": normal } }] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b2.bin)]);
        let n = raw.mesh.triangles[0].normals.unwrap()[0].normalize();
        assert!(close(n, Vec3::new(0.25, 1.0, 0.0).normalize()), "{n:?}");
    }

    #[test]
    fn negative_scale_flips_winding() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let doc = b.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0, 1] }],
                "nodes": [{ "mesh": 0 }, { "scale": [-1.0, 1.0, 1.0], "mesh": 0 }],
                "meshes": [{ "primitives": [quad] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert_eq!(raw.mesh.triangles.len(), 4);
        let face = |t: &RawTriangle| (t.positions[1] - t.positions[0]).cross(t.positions[2] - t.positions[0]);
        for t in &raw.mesh.triangles {
            // Both instances still face +Z with counter-clockwise winding, and the normals agree with it.
            assert!(face(t).z > 0.0, "{:?}", t.positions);
            assert_eq!(t.normals.unwrap(), [Vec3::Z; 3]);
        }
        let mirrored = &raw.mesh.triangles[2];
        assert_eq!(mirrored.positions, [Vec3::ZERO, Vec3::new(-1.0, 1.0, 0.0), Vec3::new(-1.0, 0.0, 0.0)]);
        // Attributes travel with their vertices.
        assert_eq!(mirrored.uvs.unwrap(), [Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)]);
    }

    #[test]
    fn primitives_without_indices_and_other_modes() {
        let mut b = Builder::default();
        let soup = b.vec3(&[
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0],
            [9.0, 9.0, 9.0], // one dangling vertex: not a full triangle
        ]);
        let strip = b.vec3(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 2.0, 0.0]]);
        let fan = b.vec3(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        let doc = b.document(
            Some("m.bin"),
            single_mesh(json!([
                { "attributes": { "POSITION": soup } },
                { "attributes": { "POSITION": strip }, "mode": 5 },
                { "attributes": { "POSITION": fan }, "mode": 6 },
                { "attributes": { "POSITION": fan }, "mode": 0 },
                { "attributes": { "POSITION": fan }, "mode": 1 },
            ])),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert_eq!(raw.mesh.triangles.len(), 2 + 3 + 2);
        assert_eq!(raw.mesh.triangles[1].positions[0], Vec3::new(2.0, 0.0, 0.0));
        for t in &raw.mesh.triangles {
            let normal = (t.positions[1] - t.positions[0]).cross(t.positions[2] - t.positions[0]);
            assert!(normal.z > 0.0, "strip and fan triangles keep a consistent winding: {:?}", t.positions);
            assert!(t.normals.is_none() && t.uvs.is_none());
        }
        // Points and lines are skipped with one warning each.
        assert_eq!(raw.warnings.len(), 2);
        assert!(raw.warnings.iter().all(|w| matches!(w, ImportWarning::SkippedPrimitive { .. })));
        assert_eq!(triangle_corners(Mode::TriangleStrip, 5), [[0, 1, 2], [1, 3, 2], [2, 3, 4]]);
        assert_eq!(triangle_corners(Mode::TriangleFan, 2), Vec::<[usize; 3]>::new());
    }

    #[test]
    fn normalized_integer_uvs_and_wide_indices() {
        let mut b = Builder::default();
        let position = b.vec3(&QUAD_POSITIONS);
        let uv16: Vec<u8> = [[0u16, 65535], [65535, 65535], [65535, 0], [0, 0]]
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let uv16 = b.accessor(&uv16, U16, "VEC2", 4, true);
        let uv8 = b.accessor(&[0, 255, 255, 255, 255, 0, 0, 0], U8, "VEC2", 4, true);
        let index32: Vec<u8> = [0u32, 1, 2, 0, 2, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
        let index32 = b.accessor(&index32, U32, "SCALAR", 6, false);
        let index8 = b.accessor(&[0, 1, 2, 0, 2, 3], U8, "SCALAR", 6, false);
        let doc = b.document(
            Some("m.bin"),
            single_mesh(json!([
                { "attributes": { "POSITION": position, "TEXCOORD_0": uv16 }, "indices": index32 },
                { "attributes": { "POSITION": position, "TEXCOORD_0": uv8 }, "indices": index8 },
            ])),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        let expected = samples::flat_quad();
        assert_eq!(raw.mesh.triangles.len(), 4);
        for (i, t) in raw.mesh.triangles.iter().enumerate() {
            assert_eq!(t.positions, expected.triangles[i % 2].positions);
            assert_eq!(t.uvs, expected.triangles[i % 2].uvs, "triangle {i}");
        }
    }

    #[test]
    fn integer_attributes_of_a_core_file_keep_the_core_reading() {
        // No KHR_mesh_quantization declared: a u16 UV flagged `normalized: false` is still read as normalized (as
        // before the extension was supported), and an integer position is not a core position (skipped).
        let mut b = Builder::default();
        let position = b.vec3(&QUAD_POSITIONS);
        let uv16: Vec<u8> = [[0u16, 65535], [65535, 65535], [65535, 0], [0, 0]]
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let uv16 = b.accessor(&uv16, U16, "VEC2", 4, false);
        let indices = b.indices_u16(&QUAD_INDICES);
        let primitive = json!({ "attributes": { "POSITION": position, "TEXCOORD_0": uv16 }, "indices": indices });
        let mut extra = single_mesh(json!([primitive]));
        let raw = load_json(&b.document(Some("m.bin"), extra.clone()), &[("m.bin", &b.bin)]);
        let expected = samples::flat_quad();
        assert_eq!(raw.mesh.triangles.len(), 2);
        for (t, e) in raw.mesh.triangles.iter().zip(&expected.triangles) {
            assert_eq!(t.uvs, e.uvs);
        }
        // Declared in extensionsUsed only: the same accessor keeps its raw values.
        extra["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
        let raw = load_json(&b.document(Some("m.bin"), extra.clone()), &[("m.bin", &b.bin)]);
        let largest = raw.mesh.triangles.iter().flat_map(|t| t.uvs.iter().flatten()).fold(0.0f32, |m, uv| m.max(uv.x));
        assert_eq!(largest, 65535.0);
        // An integer position in a core file is skipped with a warning, as before.
        extra.as_object_mut().unwrap().remove("extensionsUsed");
        let rows: Vec<Vec<i64>> = QUAD_POSITIONS.iter().map(|p| p.iter().map(|c| *c as i64).collect()).collect();
        let position8 = Encoding { component: U8, normalized: false }.accessor(&mut b, &rows, "VEC3");
        extra["meshes"][0]["primitives"] = json!([{ "attributes": { "POSITION": position8 }, "indices": indices }]);
        let raw = load_json(&b.document(Some("m.bin"), extra), &[("m.bin", &b.bin)]);
        assert!(raw.mesh.triangles.is_empty());
        assert!(matches!(&raw.warnings[..], [ImportWarning::SkippedPrimitive { .. }]), "{:?}", raw.warnings);
    }

    #[test]
    fn two_primitives_with_two_materials_give_two_sets() {
        let mut b = Builder::default();
        let first = b.quad([0.0; 3]);
        let second = b.quad([2.0, 0.0, 0.0]);
        let mut extra = single_mesh(json!([
            { "attributes": second["attributes"], "indices": second["indices"], "material": 1 },
            { "attributes": first["attributes"], "indices": first["indices"], "material": 0 },
        ]));
        extra["materials"] = json!([{ "name": "Body" }, { "name": "Head" }, { "name": "Unused" }]);
        let model = import(ModelFormat::Glb, b.glb(extra), &[]);
        // Materials in document order, whatever the primitive order; the unused one is left out.
        let names: Vec<&str> = model.mesh.materials.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Body", "Head"]);
        assert_eq!(model.mesh.sets.len(), 2);
        assert_eq!(model.mesh.sets[0].name, "Body");
        assert_eq!(model.mesh.sets[1].name, "Head");
        assert_eq!(model.mesh.sets[0].materials, [0]);
        assert_eq!(model.mesh.sets[1].materials, [1]);
        assert!(model.textures.is_empty());
        assert!(model.warnings.is_empty(), "{:?}", model.warnings);
    }

    #[test]
    fn two_materials_sharing_an_image_give_one_set() {
        let image = png(&samples::checker_texture(8, 8, 2));
        let mut b = Builder::default();
        let first = b.quad([0.0; 3]);
        let second = b.quad([2.0, 0.0, 0.0]);
        let third = b.quad([4.0, 0.0, 0.0]);
        let mut extra = single_mesh(json!([
            { "attributes": first["attributes"], "indices": first["indices"], "material": 0 },
            { "attributes": second["attributes"], "indices": second["indices"], "material": 1 },
            { "attributes": third["attributes"], "indices": third["indices"], "material": 2 },
        ]));
        // Two texture objects over the same image, and a third material with its own image.
        extra["materials"] = json!([
            { "name": "A", "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } },
            { "name": "B", "pbrMetallicRoughness": { "baseColorTexture": { "index": 1 } } },
            { "name": "C", "pbrMetallicRoughness": { "baseColorTexture": { "index": 2 } } },
        ]);
        extra["textures"] = json!([{ "source": 0 }, { "source": 0 }, { "source": 1 }]);
        extra["images"] = json!([{ "uri": "atlas.png" }, { "uri": "other.png" }]);
        let files: [(&str, &[u8]); 2] = [("atlas.png", &image), ("other.png", &image)];
        let model = import(ModelFormat::Glb, b.glb(extra), &files);
        assert_eq!(model.mesh.materials.len(), 3);
        assert_eq!(model.mesh.sets.len(), 2);
        assert_eq!(model.mesh.sets[0].name, "atlas");
        assert_eq!(model.mesh.sets[0].materials, [0, 1]);
        assert_eq!(model.mesh.sets[0].texture, Some(0));
        assert_eq!(model.mesh.sets[1].name, "other");
        assert_eq!(model.mesh.sets[1].materials, [2]);
        assert_eq!(model.textures.len(), 2);
        assert!(model.textures.iter().all(|t| t.image.is_some()));
    }

    #[test]
    fn texture_transform_offset_and_scale_are_applied() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let second_uv = b.vec2(&[[0.0, 0.0], [0.5, 0.0], [0.5, 0.5], [0.0, 0.5]]);
        let with_material = |material: u32| {
            let mut attributes = quad["attributes"].clone();
            attributes["TEXCOORD_1"] = json!(second_uv);
            json!({ "attributes": attributes, "indices": quad["indices"], "material": material })
        };
        let mut extra = single_mesh(json!([with_material(0), with_material(1), with_material(2)]));
        let transform = |body: Value| json!({ "index": 0, "extensions": { "KHR_texture_transform": body } });
        extra["extensionsUsed"] = json!(["KHR_texture_transform"]);
        extra["materials"] = json!([
            { "pbrMetallicRoughness": { "baseColorTexture": transform(json!({ "offset": [0.25, 0.5], "scale": [0.5, 0.25] })) } },
            { "pbrMetallicRoughness": { "baseColorTexture": transform(json!({ "rotation": std::f32::consts::FRAC_PI_2 })) } },
            { "pbrMetallicRoughness": { "baseColorTexture": transform(json!({ "texCoord": 1, "offset": [0.5, 0.5] })) } },
        ]);
        extra["textures"] = json!([{ "source": 0 }]);
        extra["images"] = json!([{ "uri": "t.png" }]);
        let raw = load(&b.glb(extra), &[], false).unwrap();
        assert_eq!(raw.mesh.triangles.len(), 6);
        let close = |a: Vec2, b: Vec2| (a - b).abs().max_element() < 1e-6;
        // Material 0: uv' = offset + scale * uv, for the corners (0,1), (1,1), (1,0).
        let uv = raw.mesh.triangles[0].uvs.unwrap();
        let expected = [Vec2::new(0.25, 0.75), Vec2::new(0.75, 0.75), Vec2::new(0.75, 0.5)];
        assert!(uv.iter().zip(expected).all(|(a, e)| close(*a, e)), "{uv:?}");
        // Material 1: a quarter turn, (u, v) -> (v, -u).
        let uv = raw.mesh.triangles[2].uvs.unwrap();
        let expected = [Vec2::new(1.0, 0.0), Vec2::new(1.0, -1.0), Vec2::new(0.0, -1.0)];
        assert!(uv.iter().zip(expected).all(|(a, e)| close(*a, e)), "{uv:?}");
        // Material 2: the transform selects UV set 1.
        let uv = raw.mesh.triangles[4].uvs.unwrap();
        let expected = [Vec2::new(0.5, 0.5), Vec2::new(1.0, 0.5), Vec2::new(1.0, 1.0)];
        assert!(uv.iter().zip(expected).all(|(a, e)| close(*a, e)), "{uv:?}");
        // All three materials show the same image: one texture.
        assert_eq!(raw.textures.len(), 1);
        assert!(raw.mesh.materials.iter().all(|m| m.texture == Some(0)));
    }

    #[test]
    fn texcoord_set_of_the_base_colour_texture_is_used() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let second_uv = b.vec2(&[[0.5, 0.5], [0.75, 0.5], [0.75, 0.25], [0.5, 0.25]]);
        let mut attributes = quad["attributes"].clone();
        attributes["TEXCOORD_1"] = json!(second_uv);
        let mut extra = single_mesh(json!([
            { "attributes": attributes, "indices": quad["indices"], "material": 0 },
            { "attributes": quad["attributes"], "indices": quad["indices"], "material": 0 },
        ]));
        extra["materials"] = json!([{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0, "texCoord": 1 } } }]);
        extra["textures"] = json!([{ "source": 0 }]);
        extra["images"] = json!([{ "uri": "t.png" }]);
        let raw = load(&b.glb(extra), &[], false).unwrap();
        assert_eq!(raw.mesh.triangles[0].uvs.unwrap()[0], Vec2::new(0.5, 0.5));
        // The second primitive has no TEXCOORD_1: it is unmapped rather than mapped with the wrong set.
        assert!(raw.mesh.triangles[2].uvs.is_none());
    }

    #[test]
    fn required_draco_extension_is_reported() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let mut extra = single_mesh(json!([quad]));
        extra["extensionsUsed"] = json!(["KHR_draco_mesh_compression"]);
        extra["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
        match load(&b.glb(extra.clone()), &[], true) {
            Err(ImportError::UnsupportedGltfExtension(names)) => assert_eq!(names, ["KHR_draco_mesh_compression"]),
            other => panic!("expected UnsupportedGltfExtension, got {:?}", other.err()),
        }
        // Several unsupported extensions are all named; supported ones (including KHR_mesh_quantization) are not.
        extra["extensionsRequired"] = json!([
            "KHR_texture_transform", "KHR_draco_mesh_compression", "KHR_mesh_quantization", "EXT_meshopt_compression",
        ]);
        match load(&b.glb(extra.clone()), &[], true) {
            Err(ImportError::UnsupportedGltfExtension(names)) => {
                assert_eq!(names, ["KHR_draco_mesh_compression", "EXT_meshopt_compression"]);
            }
            other => panic!("expected UnsupportedGltfExtension, got {:?}", other.err()),
        }
        extra["extensionsRequired"] = json!(["KHR_mesh_quantization", "EXT_meshopt_compression"]);
        match load(&b.glb(extra.clone()), &[], true) {
            Err(ImportError::UnsupportedGltfExtension(names)) => assert_eq!(names, ["EXT_meshopt_compression"]),
            other => panic!("expected UnsupportedGltfExtension, got {:?}", other.err()),
        }
        // Supported required extensions are fine.
        extra["extensionsRequired"] = json!(["KHR_texture_transform"]);
        assert!(load(&b.glb(extra.clone()), &[], true).is_ok());
        extra["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
        assert!(load(&b.glb(extra), &[], true).is_ok());
    }

    #[test]
    fn skins_and_morph_targets_are_ignored_with_a_warning() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let target = b.vec3(&[[0.0, 0.0, 1.0]; 4]);
        let matrices: Vec<u8> = Mat4::IDENTITY.to_cols_array().iter().flat_map(|v| v.to_le_bytes()).collect();
        let inverse_bind = b.accessor(&matrices, F32, "MAT4", 1, false);
        let doc = b.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0, 1] }],
                "nodes": [{ "mesh": 0, "skin": 0 }, { "name": "joint" }],
                "skins": [{ "joints": [1], "inverseBindMatrices": inverse_bind }],
                "meshes": [{ "primitives": [{
                    "attributes": quad["attributes"], "indices": quad["indices"], "targets": [{ "POSITION": target }],
                }] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert_is_flat_quad(&raw.mesh);
        assert_eq!(raw.warnings, [ImportWarning::SkinIgnored]);
    }

    #[test]
    fn skinned_meshes_are_posed_by_their_joints_and_ignore_their_node_transform() {
        // Two joints: joint A (node 1) moved by +5 x, joint B (node 2, child of A) moved by a further +1 y and
        // scaled by 2. Inverse bind matrices: identity for A, a -3 z translation for B. The skinned node itself has a
        // translation of 100 that glTF says to ignore.
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let mut ibm: Vec<u8> = Mat4::IDENTITY.to_cols_array().iter().flat_map(|v| v.to_le_bytes()).collect();
        ibm.extend(Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0)).to_cols_array().iter().flat_map(|v| v.to_le_bytes()));
        let inverse_bind = b.accessor(&ibm, F32, "MAT4", 2, false);
        // Vertices 0 and 1 follow joint A, vertex 2 joint B, vertex 3 half and half (u8 joints, f32 weights).
        let joints = b.accessor(&[0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0], U8, "VEC4", 4, false);
        let weights: Vec<u8> = [[1.0f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]]
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let weights = b.accessor(&weights, F32, "VEC4", 4, false);
        let mut attributes = quad["attributes"].clone();
        attributes["JOINTS_0"] = json!(joints);
        attributes["WEIGHTS_0"] = json!(weights);
        let doc = b.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0, 1] }],
                "nodes": [
                    { "mesh": 0, "skin": 0, "translation": [100.0, 0.0, 0.0] },
                    { "name": "A", "translation": [5.0, 0.0, 0.0], "children": [2] },
                    { "name": "B", "translation": [0.0, 1.0, 0.0], "scale": [2.0, 2.0, 2.0] },
                ],
                "skins": [{ "joints": [1, 2], "inverseBindMatrices": inverse_bind }],
                "meshes": [{ "primitives": [{ "attributes": attributes, "indices": quad["indices"] }] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert!(raw.warnings.is_empty(), "{:?}", raw.warnings);
        let joint_a = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        let joint_b = joint_a
            * Mat4::from_scale_rotation_translation(Vec3::splat(2.0), glam::Quat::IDENTITY, Vec3::new(0.0, 1.0, 0.0))
            * Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0));
        let expected = [
            joint_a.transform_point3(Vec3::from(QUAD_POSITIONS[0])),
            joint_a.transform_point3(Vec3::from(QUAD_POSITIONS[1])),
            joint_b.transform_point3(Vec3::from(QUAD_POSITIONS[2])),
            ((joint_a + joint_b) * 0.5).transform_point3(Vec3::from(QUAD_POSITIONS[3])),
        ];
        // QUAD_INDICES = [0, 1, 2, 0, 2, 3].
        let got = [raw.mesh.triangles[0].positions, raw.mesh.triangles[1].positions];
        let want = [[expected[0], expected[1], expected[2]], [expected[0], expected[2], expected[3]]];
        for (g, w) in got.iter().flatten().zip(want.iter().flatten()) {
            assert!((*g - *w).length() < 1e-5, "{got:?} vs {want:?}");
        }

        // A skinned mesh without weights keeps its node transform, with a warning.
        let mut attributes = quad["attributes"].clone();
        attributes["JOINTS_0"] = json!(joints);
        let doc = b.document(
            Some("m.bin"),
            json!({
                "scenes": [{ "nodes": [0, 1] }],
                "nodes": [{ "mesh": 0, "skin": 0 }, { "name": "A", "children": [2] }, { "name": "B" }],
                "skins": [{ "joints": [1, 2], "inverseBindMatrices": inverse_bind }],
                "meshes": [{ "primitives": [{ "attributes": attributes, "indices": quad["indices"] }] }],
            }),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert_is_flat_quad(&raw.mesh);
        assert_eq!(raw.warnings, [ImportWarning::SkinIgnored]);
    }

    #[test]
    fn scene_selection_and_node_cycles() {
        let mut b = Builder::default();
        let near = b.quad([0.0; 3]);
        let far = b.quad([100.0, 0.0, 0.0]);
        let meshes = json!([{ "primitives": [near] }, { "primitives": [far] }]);
        let first_x = |raw: &RawImport| -> Vec<f32> { raw.mesh.triangles.iter().step_by(2).map(|t| t.positions[0].x).collect() };
        // The default scene wins over the first one.
        let doc = b.document(
            Some("m.bin"),
            json!({ "scene": 1, "scenes": [{ "nodes": [0] }, { "nodes": [1] }], "nodes": [{ "mesh": 0 }, { "mesh": 1 }], "meshes": meshes }),
        );
        assert_eq!(first_x(&load_json(&doc, &[("m.bin", &b.bin)])), [100.0]);
        // No default scene: the first one.
        let doc = b.document(
            Some("m.bin"),
            json!({ "scenes": [{ "nodes": [0] }, { "nodes": [1] }], "nodes": [{ "mesh": 0 }, { "mesh": 1 }], "meshes": meshes }),
        );
        assert_eq!(first_x(&load_json(&doc, &[("m.bin", &b.bin)])), [0.0]);
        // No scenes at all: every root node, children through their parents only.
        let doc = b.document(
            Some("m.bin"),
            json!({ "nodes": [{ "mesh": 1, "translation": [1.0, 0.0, 0.0] }, { "children": [0], "translation": [10.0, 0.0, 0.0] }, { "mesh": 0 }], "meshes": meshes }),
        );
        assert_eq!(first_x(&load_json(&doc, &[("m.bin", &b.bin)])), [111.0, 0.0]);
        // A cycle and a node instanced twice terminate; every node contributes once.
        let doc = b.document(
            Some("m.bin"),
            json!({ "scenes": [{ "nodes": [0, 0] }], "nodes": [{ "mesh": 0, "children": [1] }, { "mesh": 1, "children": [0] }], "meshes": meshes }),
        );
        assert_eq!(first_x(&load_json(&doc, &[("m.bin", &b.bin)])), [0.0, 100.0]);
        // An empty scene has no triangles (the caller reports `ImportError::Empty`).
        let doc = b.document(Some("m.bin"), json!({ "scenes": [{ "nodes": [] }], "nodes": [{ "mesh": 0 }], "meshes": meshes }));
        assert!(load_json(&doc, &[("m.bin", &b.bin)]).mesh.triangles.is_empty());
    }

    #[test]
    fn embedded_and_broken_images() {
        let texture = samples::checker_texture(4, 4, 2);
        let data_uri = format!("data:image/png;base64,{}", base64_encode(&png(&texture)));
        let mut b = Builder::default();
        let quads: Vec<Value> = (0..5).map(|i| b.quad([2.0 * i as f32, 0.0, 0.0])).collect();
        let garbage_view = b.view(b"definitely not an image");
        let primitives: Vec<Value> = quads
            .iter()
            .enumerate()
            .map(|(i, q)| json!({ "attributes": q["attributes"], "indices": q["indices"], "material": i }))
            .collect();
        let mut extra = single_mesh(json!(primitives));
        let material = |texture: u32| json!({ "name": format!("M{texture}"), "pbrMetallicRoughness": { "baseColorTexture": { "index": texture } } });
        extra["materials"] = json!([material(0), material(1), material(2), material(3), material(4)]);
        extra["textures"] = json!([{ "source": 0, "name": "Decal.JPG" }, { "source": 1 }, { "source": 2 }, { "source": 3 }, {}]);
        extra["images"] = json!([
            { "uri": data_uri },
            { "bufferView": garbage_view, "mimeType": "image/png", "name": "Broken" },
            { "uri": "data:image/png;base64,***" },
            { "bufferView": garbage_view },
        ]);
        let raw = load(&b.glb(extra), &[], true).unwrap();
        assert_eq!(raw.textures.len(), 5);
        // A data-URI image is named after its texture object.
        assert_eq!(raw.textures[0].name, "Decal");
        assert_eq!(raw.textures[0].image.as_ref().unwrap(), &texture);
        assert_eq!((raw.textures[1].name.as_str(), raw.textures[1].image.is_none()), ("Broken", true));
        assert_eq!(raw.textures[2].reference, "embedded image 2");
        assert!(raw.textures[3].image.is_none(), "a buffer-view image without a mime type must not panic");
        // A texture without a source is recorded and reported as missing.
        assert_eq!(raw.textures[4].name, "");
        let kinds: Vec<&str> = raw
            .warnings
            .iter()
            .map(|w| match w {
                ImportWarning::TextureDecodeFailed { .. } => "decode",
                ImportWarning::MissingTexture { .. } => "missing",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["decode", "decode", "decode", "missing"]);
        assert!(raw.warnings.contains(&ImportWarning::MissingTexture {
            material: "M4".to_owned(),
            reference: "embedded image 4".to_owned(),
        }));
        // Set assignment does not depend on any of this.
        assert!(raw.mesh.materials.iter().enumerate().all(|(i, m)| m.texture == Some(i as u32)));
    }

    #[test]
    fn specular_glossiness_diffuse_texture_is_the_fallback() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let mut extra = single_mesh(json!([{ "attributes": quad["attributes"], "indices": quad["indices"], "material": 0 }]));
        extra["extensionsUsed"] = json!(["KHR_materials_pbrSpecularGlossiness"]);
        extra["materials"] = json!([{
            "name": "Legacy",
            "extensions": { "KHR_materials_pbrSpecularGlossiness": {
                "diffuseFactor": [0.0, 1.0, 0.0, 0.5], "diffuseTexture": { "index": 0 },
            } },
        }]);
        extra["textures"] = json!([{ "source": 0 }]);
        extra["images"] = json!([{ "uri": "old/Diffuse.TGA" }]);
        let raw = load(&b.glb(extra), &[], false).unwrap();
        assert_eq!(raw.mesh.materials[0].texture, Some(0));
        assert_eq!(raw.mesh.materials[0].base_color, [0.0, 1.0, 0.0, 0.5]);
        assert_eq!(raw.textures[0].name, "Diffuse");
    }

    #[test]
    fn malformed_files_are_errors_or_warnings_never_panics() {
        // Garbage, truncated and lying containers.
        for bytes in [&b""[..], b"{", b"[]", b"glTF", b"glTF\x02\0\0\0", b"glTF\x02\0\0\0\x04\0\0\0", b"glTF\x02\0\0\0\xff\0\0\0JSON"] {
            assert!(matches!(load(bytes, &[], true), Err(ImportError::InvalidGltf(_))), "{bytes:?}");
        }
        assert!(matches!(load(b"{}", &[], true), Err(ImportError::InvalidGltf(_))), "the asset object is required");
        // A GLB whose JSON needs the binary chunk that is not there.
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let json = serde_json::to_vec(&b.document(None, single_mesh(json!([quad])))).unwrap();
        let glb = ::gltf::binary::Glb {
            header: ::gltf::binary::Header { magic: *b"glTF", version: 2, length: 0 },
            json: json.into(),
            bin: None,
        };
        assert!(matches!(load(&glb.to_vec().unwrap(), &[], true), Err(ImportError::InvalidGltf(_))));
        // An index out of range is caught by validation.
        let doc = b.document(Some("m.bin"), json!({ "scenes": [{ "nodes": [7] }] }));
        assert!(matches!(load(&serde_json::to_vec(&doc).unwrap(), &[("m.bin", &b.bin)], true), Err(ImportError::InvalidGltf(_))));

        // Accessors the readers of the gltf crate would trip over: each costs its primitive (or attribute) only.
        let mut b = Builder::default();
        let good = b.quad([0.0; 3]);
        let position = good["attributes"]["POSITION"].as_u64().unwrap() as usize;
        let uv = good["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize;
        let index = good["indices"].as_u64().unwrap() as usize;
        let push = |b: &mut Builder, base: usize, patch: Value| -> usize {
            let mut accessor = b.accessors[base].clone();
            for (key, value) in patch.as_object().unwrap() {
                accessor[key] = value.clone();
            }
            b.accessors.push(accessor);
            b.accessors.len() - 1
        };
        let too_many = push(&mut b, position, json!({ "count": 1000 }));
        let empty = push(&mut b, position, json!({ "count": 0 }));
        let wrong_type = push(&mut b, position, json!({ "type": "VEC2" }));
        // (A 16-bit position is a valid KHR_mesh_quantization accessor; a 32-bit integer one is not.)
        let wrong_component = push(&mut b, position, json!({ "componentType": U32 }));
        let far_offset = push(&mut b, position, json!({ "byteOffset": 18446744073709551000u64 }));
        let float_index = push(&mut b, index, json!({ "componentType": F32, "count": 3 }));
        let big_index = b.indices_u16(&[0, 1, 2, 0, 2, 40000]);
        let wrong_uv = push(&mut b, uv, json!({ "type": "VEC3" }));
        let short_uv = push(&mut b, uv, json!({ "count": 2 }));
        b.views.push(json!({ "buffer": 0, "byteOffset": b.bin.len(), "byteLength": 4096 }));
        let outside_view = b.views.len() - 1;
        let outside = push(&mut b, position, json!({ "bufferView": outside_view }));
        let primitive = |position: usize, extra: Value| {
            let mut p = json!({ "attributes": { "POSITION": position } });
            for (key, value) in extra.as_object().unwrap() {
                p[key] = value.clone();
            }
            p
        };
        let doc = b.document(
            Some("m.bin"),
            single_mesh(json!([
                primitive(too_many, json!({})),
                primitive(empty, json!({})),
                primitive(wrong_type, json!({})),
                primitive(wrong_component, json!({})),
                primitive(far_offset, json!({})),
                primitive(outside, json!({})),
                primitive(position, json!({ "indices": float_index })),
                primitive(position, json!({ "indices": big_index })),
                { "attributes": { "POSITION": position, "TEXCOORD_0": wrong_uv, "NORMAL": wrong_type }, "indices": index },
                { "attributes": { "POSITION": position, "TEXCOORD_0": short_uv }, "indices": index },
                good,
            ])),
        );
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        // Seven primitives are skipped entirely, one loses a triangle, two lose an attribute, one is intact.
        let skipped = raw.warnings.iter().filter(|w| matches!(w, ImportWarning::SkippedPrimitive { .. })).count();
        assert_eq!(skipped, 8, "{:#?}", raw.warnings);
        assert_eq!(raw.warnings.len(), 8);
        assert_eq!(raw.mesh.triangles.len(), 1 + 2 + 2 + 2);
        assert!(raw.mesh.triangles[1].uvs.is_none() && raw.mesh.triangles[1].normals.is_none());
        assert!(raw.mesh.triangles[3].uvs.is_none());
        assert!(raw.mesh.triangles[5].uvs.is_some() && raw.mesh.triangles[5].normals.is_some());
    }

    #[test]
    fn position_naming_a_missing_accessor_is_an_error_not_a_panic() {
        // The 156-byte file of the review: the validation of the gltf crate indexed `accessors[3]` and panicked.
        let tiny = br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":3}}]}],"accessors":[]}"#;
        match load(tiny, &[], true) {
            Err(ImportError::InvalidGltf(message)) => assert!(message.contains("POSITION"), "{message}"),
            other => panic!("expected InvalidGltf, got {:?}", other.err()),
        }
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        for index in [4usize, 9, 255, 65535, 4294967295] {
            // An unused mesh with a bad index is caught too, in .gltf and in .glb.
            let mut bad = quad.clone();
            bad["attributes"]["POSITION"] = json!(index);
            let extra = json!({ "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0 }],
                                "meshes": [{ "primitives": [quad] }, { "primitives": [bad] }] });
            let gltf = serde_json::to_vec(&b.document(Some("m.bin"), extra.clone())).unwrap();
            assert!(matches!(load(&gltf, &[("m.bin", &b.bin)], true), Err(ImportError::InvalidGltf(_))), "{index}");
            assert!(matches!(load(&b.glb(extra.clone()), &[], true), Err(ImportError::InvalidGltf(_))), "{index}");
            // The same path as opening a project whose stored source has the defect.
            let file = |name: &str, bytes: &[u8]| SourceFile {
                key: normalize_ref(name),
                original_name: name.to_owned(),
                bytes: Arc::from(bytes),
            };
            let bundle = SourceBundle {
                format: ModelFormat::Gltf,
                main: file("main.gltf", &gltf),
                sidecars: vec![file("m.bin", &b.bin)],
                extras: Vec::new(),
                original_path: None,
            };
            let result = import_from_bundle(&bundle, &ImportOptions::default(), false, &NoProgress);
            assert!(matches!(result, Err(ImportError::InvalidGltf(_))), "{index}");
        }
    }

    #[test]
    fn position_without_bounds_is_accepted() {
        let mut b = Builder::default();
        let quad = b.quad([0.0; 3]);
        let position = quad["attributes"]["POSITION"].as_u64().unwrap() as usize;
        b.accessors[position].as_object_mut().unwrap().remove("min");
        b.accessors[position].as_object_mut().unwrap().remove("max");
        let raw = load_json(&b.document(Some("m.bin"), single_mesh(json!([quad]))), &[("m.bin", &b.bin)]);
        assert_is_flat_quad(&raw.mesh);
        // Bounds that are present but malformed are still an error, as is any other missing property.
        b.accessors[position]["min"] = json!("nonsense");
        let doc = b.document(Some("m.bin"), single_mesh(json!([quad])));
        assert!(matches!(load(&serde_json::to_vec(&doc).unwrap(), &[("m.bin", &b.bin)], true), Err(ImportError::InvalidGltf(_))));
        b.accessors[position].as_object_mut().unwrap().remove("min");
        b.accessors[position].as_object_mut().unwrap().remove("componentType");
        let doc = b.document(Some("m.bin"), single_mesh(json!([quad])));
        assert!(load(&serde_json::to_vec(&doc).unwrap(), &[("m.bin", &b.bin)], true).is_err());
    }

    #[test]
    fn sparse_accessors_are_read() {
        let mut b = Builder::default();
        let position = b.vec3(&QUAD_POSITIONS);
        let sparse_indices = b.view(&[3u8, 0, 0, 0]);
        let moved: Vec<u8> = [0.0f32, 5.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let sparse_values = b.view(&moved);
        b.accessors[position]["sparse"] = json!({
            "count": 1,
            "indices": { "bufferView": sparse_indices, "componentType": U8 },
            "values": { "bufferView": sparse_values },
        });
        let indices = b.indices_u16(&QUAD_INDICES);
        let doc = b.document(Some("m.bin"), single_mesh(json!([{ "attributes": { "POSITION": position }, "indices": indices }])));
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert_eq!(raw.mesh.triangles[1].positions[2], Vec3::new(0.0, 5.0, 0.0));
        // A sparse part pointing outside its view is rejected instead of read.
        let mut doc = doc;
        doc["accessors"][position]["sparse"]["count"] = json!(3);
        let raw = load_json(&doc, &[("m.bin", &b.bin)]);
        assert!(raw.mesh.triangles.is_empty());
        assert_eq!(raw.warnings.len(), 1);
    }

    const I8: u32 = 5120;
    const I16: u32 = 5122;

    /// One integer storage of `KHR_mesh_quantization`: component type and `normalized` flag.
    #[derive(Copy, Clone, Debug)]
    struct Encoding {
        component: u32,
        normalized: bool,
    }

    impl Encoding {
        const ALL: [Encoding; 8] = [
            Encoding { component: I8, normalized: false },
            Encoding { component: I8, normalized: true },
            Encoding { component: U8, normalized: false },
            Encoding { component: U8, normalized: true },
            Encoding { component: I16, normalized: false },
            Encoding { component: I16, normalized: true },
            Encoding { component: U16, normalized: false },
            Encoding { component: U16, normalized: true },
        ];

        fn signed(self) -> bool {
            matches!(self.component, I8 | I16)
        }

        fn bytes(self) -> usize {
            if matches!(self.component, I8 | U8) { 1 } else { 2 }
        }

        /// The largest value of the type (the divisor of normalized values).
        fn max(self) -> f64 {
            match self.component {
                I8 => 127.0,
                U8 => 255.0,
                I16 => 32767.0,
                _ => 65535.0,
            }
        }

        /// What a reader of the stored integer `q` gets (glTF 2.0, "Animations / normalized integers").
        fn decode(self, q: i64) -> f64 {
            if self.normalized { (q as f64 / self.max()).max(-1.0) } else { q as f64 }
        }

        /// Stores `rows` (one per vertex) as an accessor of this encoding, each row padded to a 4-byte stride as
        /// vertex attributes require.
        fn accessor(self, b: &mut Builder, rows: &[Vec<i64>], kind: &str) -> usize {
            let stride = (rows[0].len() * self.bytes()).next_multiple_of(4);
            let mut data = Vec::new();
            for row in rows {
                for &q in row {
                    match self.bytes() {
                        1 => data.push(q as u8),
                        _ => data.extend_from_slice(&(q as u16).to_le_bytes()),
                    }
                }
                data.resize(data.len().next_multiple_of(stride), 0);
            }
            let index = b.accessor(&data, self.component, kind, rows.len(), self.normalized);
            let view = b.accessors[index]["bufferView"].as_u64().unwrap() as usize;
            b.views[view]["byteStride"] = json!(stride);
            index
        }
    }

    /// A box with uneven extents, each face a fan of four triangles around its centre (so the centres fall between
    /// the quantization steps), with axis normals and one quarter cell of the texture per face.
    struct QuantBox {
        lo: [f64; 3],
        hi: [f64; 3],
        positions: Vec<[f64; 3]>,
        normals: Vec<[f64; 3]>,
        uvs: Vec<[f64; 2]>,
        indices: Vec<u16>,
    }

    fn quant_box() -> QuantBox {
        let (lo, hi) = ([-0.731, -0.42, 0.111], [1.237, 0.913, 2.05]);
        let mut out = QuantBox { lo, hi, positions: Vec::new(), normals: Vec::new(), uvs: Vec::new(), indices: Vec::new() };
        for face in 0..6usize {
            let (axis, positive) = (face / 2, face % 2 == 1);
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            // (a-, b-), (a+, b-), (a+, b+), (a-, b+) is counter-clockwise seen from +axis; a negative face mirrors
            // its positions along `a` (counter-clockwise seen from -axis) and keeps the same UV corners, so that
            // every face shows the texture the same way round.
            let corners = [(false, false), (true, false), (true, true), (false, true)];
            let cell = [(face % 4) as f64 * 0.25, (face / 4) as f64 * 0.25];
            let base = out.positions.len() as u16;
            let mut normal = [0.0; 3];
            normal[axis] = if positive { 1.0 } else { -1.0 };
            let mut centre = [0.0; 3];
            centre[axis] = if positive { hi[axis] } else { lo[axis] };
            centre[a] = (lo[a] + hi[a]) / 2.0;
            centre[b] = (lo[b] + hi[b]) / 2.0;
            out.positions.push(centre);
            out.uvs.push([cell[0] + 0.125, cell[1] + 0.125]);
            for (ua, ub) in corners {
                let mut p = centre;
                p[a] = if ua == positive { hi[a] } else { lo[a] };
                p[b] = if ub { hi[b] } else { lo[b] };
                out.positions.push(p);
                out.uvs.push([cell[0] + if ua { 0.25 } else { 0.0 }, cell[1] + if ub { 0.25 } else { 0.0 }]);
            }
            out.normals.extend([normal; 5]);
            for k in 0..4u16 {
                out.indices.extend([base, base + 1 + k, base + 1 + (k + 1) % 4]);
            }
        }
        out
    }

    /// The node holding the model: a rotation of 30 degrees about Y, a scale of 3 and a translation, so that the
    /// dequantization transform of the quantized twin is combined with an ordinary node transform.
    fn quant_parent() -> Value {
        let half = 15f64.to_radians();
        json!({ "translation": [5.0, -2.0, 1.0], "rotation": [0.0, half.sin(), 0.0, half.cos()], "scale": [3.0, 3.0, 3.0], "children": [1] })
    }

    fn quant_material(transform: Option<Value>) -> Value {
        let mut texture = json!({ "index": 0 });
        if let Some(transform) = transform {
            texture["extensions"] = json!({ "KHR_texture_transform": transform });
        }
        json!([{ "pbrMetallicRoughness": { "baseColorTexture": texture } }])
    }

    /// The float twin of `quant_box` (positions as given), as a GLB.
    fn float_twin(shape: &QuantBox, positions: &[[f64; 3]]) -> Vec<u8> {
        let to32 = |rows: &[[f64; 3]]| rows.iter().map(|p| p.map(|c| c as f32)).collect::<Vec<[f32; 3]>>();
        let mut b = Builder::default();
        let position = b.vec3(&to32(positions));
        let normal = b.vec3(&to32(&shape.normals));
        let uvs: Vec<[f32; 2]> = shape.uvs.iter().map(|uv| uv.map(|c| c as f32)).collect();
        let uv = b.vec2(&uvs);
        let indices = b.indices_u16(&shape.indices);
        b.glb(json!({
            "scene": 0, "scenes": [{ "nodes": [0] }],
            "nodes": [quant_parent(), { "mesh": 0 }],
            "meshes": [{ "primitives": [{
                "attributes": { "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv }, "indices": indices, "material": 0,
            }] }],
            "materials": quant_material(None),
            "textures": [{ "source": 0 }],
            "images": [{ "uri": "t.png" }],
        }))
    }

    /// The quantized twin: positions in `pos` (dequantized by the child node's scale / translation), normals in
    /// `normal` (normalized), tangents as normalized bytes (never read) and UVs in `uv` (rescaled by
    /// `KHR_texture_transform`). Returns the GLB and the positions a perfect reader gets.
    fn quantized_twin(shape: &QuantBox, pos: Encoding, normal: Encoding, uv: Encoding) -> (Vec<u8>, Vec<[f64; 3]>) {
        let mut b = Builder::default();
        // Positions: per axis, signed types centre the box on 0, unsigned ones start it at 0.
        let mut translation = [0.0; 3];
        let mut scale = [0.0; 3];
        let mut rows = vec![Vec::new(); shape.positions.len()];
        for k in 0..3 {
            let (lo, hi) = (shape.lo[k], shape.hi[k]);
            let (origin, range, steps) =
                if pos.signed() { ((lo + hi) / 2.0, (hi - lo) / 2.0, pos.max()) } else { (lo, hi - lo, pos.max()) };
            translation[k] = origin;
            scale[k] = if pos.normalized { range } else { range / steps };
            for (row, p) in rows.iter_mut().zip(&shape.positions) {
                row.push(((p[k] - origin) / range * steps).round() as i64);
            }
        }
        let decoded: Vec<[f64; 3]> = rows
            .iter()
            .map(|row| std::array::from_fn(|k| translation[k] + pos.decode(row[k]) * scale[k]))
            .collect();
        let position = pos.accessor(&mut b, &rows, "VEC3");
        let bound = |pick: fn(i64, i64) -> i64| -> Vec<i64> {
            (0..3).map(|k| rows.iter().map(|r| r[k]).reduce(pick).unwrap()).collect()
        };
        b.accessors[position]["min"] = json!(bound(i64::min));
        b.accessors[position]["max"] = json!(bound(i64::max));
        // Normals: unit axes, exact in both normalized types.
        let normal_rows: Vec<Vec<i64>> =
            shape.normals.iter().map(|n| n.iter().map(|c| (c * normal.max()) as i64).collect()).collect();
        let normal_index = normal.accessor(&mut b, &normal_rows, "VEC3");
        let tangent_rows = vec![vec![127i64, 0, 0, 127]; shape.positions.len()];
        let tangent = Encoding { component: I8, normalized: true }.accessor(&mut b, &tangent_rows, "VEC4");
        // UVs: on a 1/8 grid; stored as (uv - offset) * multiple, scaled back by the texture transform.
        let multiple = if uv.bytes() == 1 { 120.0 } else { 32760.0 };
        let offset = if uv.signed() { 0.5 } else { 0.0 };
        let uv_rows: Vec<Vec<i64>> =
            shape.uvs.iter().map(|t| t.iter().map(|c| ((c - offset) * multiple).round() as i64).collect()).collect();
        let uv_index = uv.accessor(&mut b, &uv_rows, "VEC2");
        let uv_scale = if uv.normalized { uv.max() / multiple } else { 1.0 / multiple };
        let indices = b.indices_u16(&shape.indices);
        let glb = b.glb(json!({
            "extensionsUsed": ["KHR_mesh_quantization", "KHR_texture_transform"],
            "extensionsRequired": ["KHR_mesh_quantization"],
            "scene": 0, "scenes": [{ "nodes": [0] }],
            "nodes": [quant_parent(), { "translation": translation, "scale": scale, "mesh": 0 }],
            "meshes": [{ "primitives": [{
                "attributes": { "POSITION": position, "NORMAL": normal_index, "TANGENT": tangent, "TEXCOORD_0": uv_index },
                "indices": indices, "material": 0,
            }] }],
            "materials": quant_material(Some(json!({ "offset": [offset, offset], "scale": [uv_scale, uv_scale] }))),
            "textures": [{ "source": 0 }],
            "images": [{ "uri": "t.png" }],
        }));
        (glb, decoded)
    }

    /// Asserts that two imports are the same mesh: same triangles and flags, positions within `1e-4` of the bounding
    /// box diagonal, normals and UVs within `1e-4`.
    fn assert_same_mesh(a: &ImportedModel, b: &ImportedModel, what: &str) {
        let (a, b) = (&a.mesh, &b.mesh);
        assert_eq!(a.indices, b.indices, "{what}");
        assert_eq!(a.tri_material, b.tri_material, "{what}");
        assert_eq!(a.positions.len(), b.positions.len(), "{what}");
        let diagonal = (b.bounds.max - b.bounds.min).length();
        let worst = |x: &[Vec3], y: &[Vec3]| x.iter().zip(y).map(|(p, q)| (*p - *q).length()).fold(0.0f32, f32::max);
        let position_error = worst(&a.positions, &b.positions) / diagonal;
        assert!(position_error <= 1e-4, "{what}: positions differ by {position_error} of the diagonal");
        let normal_error = worst(&a.normals, &b.normals);
        assert!(normal_error <= 1e-4, "{what}: normals differ by {normal_error}");
        let uv_error = a.uvs.iter().zip(&b.uvs).map(|(p, q)| (*p - *q).length()).fold(0.0f32, f32::max);
        assert!(uv_error <= 1e-4, "{what}: UVs differ by {uv_error}");
        assert_eq!(a.tri_flags, b.tri_flags, "{what}");
    }

    #[test]
    fn quantized_meshes_import_like_their_float_twin() {
        let image = png(&samples::checker_texture(8, 8, 2));
        let files: [(&str, &[u8]); 1] = [("t.png", &image)];
        let shape = quant_box();
        let original = import(ModelFormat::Glb, float_twin(&shape, &shape.positions), &files);
        assert_eq!(original.mesh.tri_count(), 24);
        assert!(original.warnings.is_empty(), "{:?}", original.warnings);
        let mirrored = original.mesh.tri_flags.iter().filter(|f| f.contains(crate::mesh::TriFlags::UV_MIRRORED)).count();
        assert!(mirrored == 0 || mirrored == 24, "the faces show the texture the same way round: {mirrored}");
        let normals = [Encoding { component: I8, normalized: true }, Encoding { component: I16, normalized: true }];
        for (i, pos) in Encoding::ALL.into_iter().enumerate() {
            // Every encoding is used for positions and for UVs, each normal type with half of them.
            let (normal, uv) = (normals[i % 2], Encoding::ALL[(i + 3) % 8]);
            let what = format!("POSITION {pos:?}, NORMAL {normal:?}, TEXCOORD_0 {uv:?}");
            let (glb, decoded) = quantized_twin(&shape, pos, normal, uv);
            let quantized = import(ModelFormat::Glb, glb, &files);
            assert!(quantized.warnings.is_empty(), "{what}: {:?}", quantized.warnings);
            assert!(!quantized.report.normals_generated, "{what}: the quantized normals are used");
            // Against the float file of the positions the quantized one stores (exact for the box corners).
            let twin = import(ModelFormat::Glb, float_twin(&shape, &decoded), &files);
            assert_same_mesh(&quantized, &twin, &what);
            // 16-bit positions also stay within 1e-4 of the diagonal of the unquantized box.
            if pos.bytes() == 2 {
                assert_same_mesh(&quantized, &original, &what);
            }
        }
    }

    #[test]
    fn quantized_attributes_are_read_with_the_gltf_normalization_rules() {
        let mut b = Builder::default();
        // Signed normalized: -128 clamps to -1, like -127.
        let rows = vec![vec![-128i64, -127, 127], vec![0, 64, -64]];
        let position = Encoding { component: I8, normalized: true }.accessor(&mut b, &rows, "VEC3");
        let rows16 = vec![vec![-32768i64, 32767, 1], vec![-1, 0, 0]];
        let position16 = Encoding { component: I16, normalized: true }.accessor(&mut b, &rows16, "VEC3");
        let raw_u16 = Encoding { component: U16, normalized: false }.accessor(&mut b, &[vec![65535, 7, 0]], "VEC3");
        let raw_u8 = Encoding { component: U8, normalized: true }.accessor(&mut b, &[vec![255, 0, 51]], "VEC3");
        let uv = Encoding { component: I16, normalized: false }.accessor(&mut b, &[vec![-3, 9]], "VEC2");
        let doc = b.document(Some("m.bin"), json!({}));
        let document = ::gltf::Gltf::from_slice_without_validation(&serde_json::to_vec(&doc).unwrap()).unwrap().document;
        let buffers: Vec<Arc<[u8]>> = vec![Arc::from(b.bin.as_slice())];
        let read3 = |index: usize| read_f32::<3>(&document.accessors().nth(index).unwrap(), &buffers, true).unwrap();
        assert_eq!(read3(position), [[-1.0, -1.0, 1.0], [0.0, 64.0 / 127.0, -64.0 / 127.0]]);
        assert_eq!(read3(position16), [[-1.0, 1.0, 1.0 / 32767.0], [-1.0 / 32767.0, 0.0, 0.0]]);
        assert_eq!(read3(raw_u16), [[65535.0, 7.0, 0.0]]);
        assert_eq!(read3(raw_u8), [[1.0, 0.0, 0.2]]);
        assert_eq!(read_f32::<2>(&document.accessors().nth(uv).unwrap(), &buffers, true).unwrap(), [[-3.0, 9.0]]);
        // A core file (no extension) reads every integer as normalized.
        assert_eq!(read3(raw_u16)[0][0], 65535.0);
        let core = read_f32::<3>(&document.accessors().nth(raw_u16).unwrap(), &buffers, false).unwrap();
        assert_eq!(core, [[1.0, 7.0 / 65535.0, 0.0]]);
    }

    #[test]
    fn quantized_edge_cases_are_skipped_or_regenerated_not_misread() {
        // A non-normalized integer normal is not a direction: the normals are regenerated, the mesh is kept.
        let mut b = Builder::default();
        let rows: Vec<Vec<i64>> = QUAD_POSITIONS.iter().map(|p| p.iter().map(|c| *c as i64).collect()).collect();
        let position = Encoding { component: U8, normalized: false }.accessor(&mut b, &rows, "VEC3");
        let normal = Encoding { component: I8, normalized: false }.accessor(&mut b, &vec![vec![0, 0, 1]; 4], "VEC3");
        let indices = b.indices_u16(&QUAD_INDICES);
        let primitive = json!({ "attributes": { "POSITION": position, "NORMAL": normal }, "indices": indices });
        let mut extra = single_mesh(json!([primitive]));
        extra["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
        let raw = load_json(&b.document(Some("m.bin"), extra.clone()), &[("m.bin", &b.bin)]);
        assert_eq!(raw.mesh.triangles.len(), 2);
        assert_eq!(raw.mesh.triangles[0].positions, samples::flat_quad().triangles[0].positions);
        assert!(raw.mesh.triangles.iter().all(|t| t.normals.is_none()));
        // A 32-bit integer position is not part of the extension: the primitive is skipped with a warning.
        let wide: Vec<u8> = [0u32, 0, 0, 1, 0, 0, 0, 1, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let wide = b.accessor(&wide, U32, "VEC3", 3, false);
        extra["meshes"][0]["primitives"] = json!([{ "attributes": { "POSITION": wide } }]);
        let raw = load_json(&b.document(Some("m.bin"), extra.clone()), &[("m.bin", &b.bin)]);
        assert!(raw.mesh.triangles.is_empty());
        assert!(matches!(&raw.warnings[..], [ImportWarning::SkippedPrimitive { .. }]), "{:?}", raw.warnings);
        // Quantized sparse positions are read too.
        let sparse_indices = b.view(&[2u8, 0, 0, 0]);
        let sparse_values = b.view(&[0u8, 5, 0, 0]);
        b.accessors[position]["sparse"] = json!({
            "count": 1,
            "indices": { "bufferView": sparse_indices, "componentType": U8 },
            "values": { "bufferView": sparse_values },
        });
        extra["meshes"][0]["primitives"] = json!([{ "attributes": { "POSITION": position }, "indices": indices }]);
        let raw = load_json(&b.document(Some("m.bin"), extra), &[("m.bin", &b.bin)]);
        assert_eq!(raw.mesh.triangles[0].positions[2], Vec3::new(0.0, 5.0, 0.0));
    }

    #[test]
    fn helper_functions() {
        assert_eq!(strip_image_extension("wood.PNG"), "wood");
        assert_eq!(strip_image_extension("Material.001"), "Material.001");
        assert_eq!(strip_image_extension(".png"), ".png");
        assert_eq!(mime_hint(Some("image/jpeg")).as_deref(), Some("image.jpg"));
        assert_eq!(mime_hint(Some("application/json")), None);
        assert!(is_data_uri(" DATA:,x") && !is_data_uri("data.bin") && !is_data_uri("d\u{e9}ta:"));
        let t = UvTransform { offset: Vec2::new(1.0, 2.0), rotation: 0.0, scale: Vec2::new(2.0, 3.0) };
        assert_eq!(t.apply(Vec2::new(0.5, 0.5)), Vec2::new(2.0, 3.5));
    }
}
