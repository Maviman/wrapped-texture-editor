// Owner: core-import (C5). Wavefront OBJ + MTL -> `RawMesh` (ARCHITECTURE.md 5.21, tobj notes: section 10).
//
// tobj 4.0.5 does the number parsing, index building and triangulation, but it is not safe on arbitrary input:
// it panics on `Kd` with fewer than three values and on far-out relative indices (overflow checks), turns polylines
// (`l` with three points) into triangles, fails on trailing comments and substitutes the previous vertex's UV /
// normal for a missing one. So both files are first rewritten into a canonical subset tobj cannot trip over
// (`sanitize_obj`, `sanitize_mtl`), and the pre-pass remembers per triangle whether it really has UVs and normals
// and which material it uses.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Cursor;

use glam::{Vec2, Vec3};

use crate::error::{ImportError, ImportWarning};
use crate::import::resolver::file_stem_of;
use crate::import::{
    AssetResolver, ImportOptions, RawImport, TextureRequest, TextureSource, clean_unit, decode_text, load_textures,
    normalize_ref, parse_texture_ref,
};
use crate::mesh::{RawMaterial, RawMesh, RawObjects, RawTriangle};

/// `TriInfo::material` of a face in front of the first `usemtl`.
const NO_MATERIAL: u32 = u32::MAX;
/// `TriInfo::o` / `TriInfo::g` of a face in front of the first `o` / `g` line.
const NO_OBJECT: u32 = u32::MAX;
/// Colour of materials without `Kd` and of materials the MTL does not define.
const DEFAULT_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

/// What the pre-pass knows about one triangle of the triangulated output, in file order.
#[derive(Copy, Clone)]
struct TriInfo {
    /// Index into `Prepass::material_names`, or `NO_MATERIAL`.
    material: u32,
    /// Every corner of the face has a valid `vt` reference.
    has_uv: bool,
    /// Every corner of the face has a valid `vn` reference.
    has_normal: bool,
    /// Index into `Prepass::o_names` of the current `o`, or `NO_OBJECT`.
    o: u32,
    /// Index into `Prepass::g_names` of the current `g`, or `NO_OBJECT`.
    g: u32,
}

/// Result of `sanitize_obj`.
struct Prepass {
    /// Canonical OBJ text: only `v x y z`, `vt u v`, `vn x y z` and `f` lines with absolute indices.
    text: String,
    tris: Vec<TriInfo>,
    /// `usemtl` names in order of first use.
    material_names: Vec<String>,
    /// `mtllib` references in order of first appearance, without duplicates.
    mtllibs: Vec<String>,
    /// `o` names in order of first appearance (a name seen again continues the same entry).
    o_names: Vec<String>,
    /// `g` names in order of first appearance (`g` without a name is "default").
    g_names: Vec<String>,
    /// The `v` / `vt` / `vn` lines whose numbers could not be used.
    bad: BadNumbers,
}

/// `v`, `vt` and `vn` lines with a missing, malformed or non-finite number (`nan`, `inf`, `1e39`, `abc`), and `f` lines
/// with a position index that points nowhere. A bad `v` / `vt` / `vn` line keeps its place in the numbering as a
/// placeholder, so the indices of all later lines stay right.
#[derive(Default)]
struct BadNumbers {
    /// Physical line number and reason of the first bad line.
    first: Option<(usize, String)>,
    lines: u32,
    /// Faces dropped because a corner uses a bad `v` or a position index that points nowhere.
    dropped_faces: u32,
    /// Faces kept without UVs or file normals because a corner uses a bad `vt` or `vn`.
    degraded_faces: u32,
}

impl BadNumbers {
    fn record(&mut self, line: usize, reason: String) {
        self.lines += 1;
        self.first.get_or_insert((line, reason));
    }
}

/// The token if it is a finite `f32`; else what is wrong with it.
fn finite_number(token: Option<&str>) -> Result<&str, &'static str> {
    let token = token.ok_or("is missing")?;
    match token.parse::<f32>() {
        Ok(value) if value.is_finite() => Ok(token),
        Ok(_) => Err("is not a finite number"),
        Err(_) => Err("is not a number"),
    }
}

fn obj_error(line: usize, message: impl std::fmt::Display) -> ImportError {
    ImportError::Obj(format!("line {line}: {message}"))
}

/// The statement keyword and the rest of the line.
fn split_keyword(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(end) => (&line[..end], line[end..].trim()),
        None => (line, ""),
    }
}

/// Resolves a 1-based (or negative, relative) OBJ index against `count` elements; `None` when it points nowhere.
fn resolve_index(token: &str, count: usize) -> Result<Option<usize>, ()> {
    let value: i64 = token.parse().map_err(|_| ())?;
    let absolute = if value < 0 { count as i64 + 1 + value } else { value };
    Ok(if absolute >= 1 && absolute <= count as i64 { Some(absolute as usize) } else { None })
}

/// Rewrites the OBJ into the canonical subset and collects per-triangle facts (see the file comment).
///
/// - `v`: the first three numbers (weights and vertex colours are dropped); `vt`: u and v (v = 0 when absent).
/// - A `v` / `vt` / `vn` line with a missing, malformed or non-finite number is written as a placeholder (so the
///   numbering does not shift) and recorded in `Prepass::bad`: faces that use a bad `v` are dropped, a bad `vt` or
///   `vn` counts as a missing reference (the face loses its UVs or its file normals).
/// - `f`: indices are made absolute; a `vt` / `vn` reference that is missing or out of range on any corner removes
///   that attribute from the whole face; a face with a position index out of range is dropped and recorded in
///   `Prepass::bad` like a bad number line; an index that is not an integer is an error; faces with fewer than three
///   corners are dropped. A face with n corners yields n - 2 `TriInfo`s (tobj triangulates it as a fan).
/// - `usemtl` / `mtllib` are recorded, not written; `o` and `g` are recorded as objects (the whole rest of the line is
///   the name; `g` without a name is "default"; a name seen again continues its entry); everything else (`s`, `l`, `p`,
///   comments) is dropped.
/// - A trailing `\` joins a line with the next one.
fn sanitize_obj(source: &str) -> Result<Prepass, ImportError> {
    let mut out = Prepass {
        text: String::with_capacity(source.len()),
        tris: Vec::new(),
        material_names: Vec::new(),
        mtllibs: Vec::new(),
        o_names: Vec::new(),
        g_names: Vec::new(),
        bad: BadNumbers::default(),
    };
    let mut material_index: BTreeMap<String, u32> = BTreeMap::new();
    let mut current_material = NO_MATERIAL;
    let (mut o_index, mut g_index): (BTreeMap<String, u32>, BTreeMap<String, u32>) = (BTreeMap::new(), BTreeMap::new());
    let (mut current_o, mut current_g) = (NO_OBJECT, NO_OBJECT);
    let (mut v_count, mut vt_count, mut vn_count) = (0usize, 0usize, 0usize);
    let mut corners: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
    let mut joined = String::new();
    // Per element (0-based): whether its line was bad.
    let (mut bad_v, mut bad_vt, mut bad_vn) = (Vec::<bool>::new(), Vec::<bool>::new(), Vec::<bool>::new());

    let mut lines = source.lines().enumerate().peekable();
    while let Some((index, physical)) = lines.next() {
        let line_no = index + 1;
        let physical = physical.trim_end();
        let (physical, continued) = match physical.strip_suffix('\\') {
            Some(head) => (head, true),
            None => (physical, false),
        };
        if continued && lines.peek().is_some() {
            joined.push_str(physical);
            joined.push(' ');
            continue;
        }
        let owned;
        let line = if joined.is_empty() {
            physical.trim_start()
        } else {
            joined.push_str(physical);
            owned = std::mem::take(&mut joined);
            owned.trim_start()
        };
        let (keyword, rest) = split_keyword(line);
        match keyword {
            "v" | "vn" => {
                let data = rest.split('#').next().unwrap_or("");
                let mut numbers = data.split_whitespace();
                let mut xyz = [""; 3];
                let mut problem = None;
                for (slot, which) in xyz.iter_mut().zip(["first", "second", "third"]) {
                    match finite_number(numbers.next()) {
                        Ok(token) => *slot = token,
                        Err(why) => {
                            problem = Some(format!("`{keyword}` needs three numbers; the {which} {why}"));
                            break;
                        }
                    }
                }
                let bad = problem.is_some();
                if let Some(reason) = problem {
                    out.bad.record(line_no, reason);
                    xyz = if keyword == "v" { ["0", "0", "0"] } else { ["0", "0", "1"] };
                }
                let _ = writeln!(out.text, "{keyword} {} {} {}", xyz[0], xyz[1], xyz[2]);
                if keyword == "v" {
                    v_count += 1;
                    bad_v.push(bad);
                } else {
                    vn_count += 1;
                    bad_vn.push(bad);
                }
            }
            "vt" => {
                let data = rest.split('#').next().unwrap_or("");
                let mut numbers = data.split_whitespace();
                let u = finite_number(numbers.next()).map_err(|why| format!("`vt` needs a number; the first {why}"));
                let v = match numbers.next() {
                    None => Ok("0"),
                    second => finite_number(second).map_err(|why| format!("`vt` has a second coordinate that {why}")),
                };
                let (u, v, bad) = match (u, v) {
                    (Ok(u), Ok(v)) => (u, v, false),
                    (Err(reason), _) | (_, Err(reason)) => {
                        out.bad.record(line_no, reason);
                        ("0", "0", true)
                    }
                };
                let _ = writeln!(out.text, "vt {u} {v}");
                vt_count += 1;
                bad_vt.push(bad);
            }
            "f" => {
                let data = rest.split('#').next().unwrap_or("");
                corners.clear();
                let mut missing = None;
                for token in data.split_whitespace() {
                    let bad = || obj_error(line_no, format_args!("invalid face element `{token}`"));
                    let mut parts = token.split('/');
                    let v = parts.next().unwrap_or("");
                    let vt = parts.next().unwrap_or("");
                    let vn = parts.next().unwrap_or("");
                    if parts.next().is_some() {
                        return Err(bad());
                    }
                    let v = resolve_index(v, v_count).map_err(|_| bad())?;
                    let vt = if vt.is_empty() { None } else { resolve_index(vt, vt_count).map_err(|_| bad())? };
                    let vn = if vn.is_empty() { None } else { resolve_index(vn, vn_count).map_err(|_| bad())? };
                    match v {
                        Some(v) => corners.push((v, vt, vn)),
                        None => {
                            missing.get_or_insert(token);
                        }
                    }
                }
                if let Some(token) = missing {
                    out.bad.record(line_no, format!("face element `{token}` refers to a vertex that does not exist"));
                    out.bad.dropped_faces += 1;
                    continue;
                }
                if corners.len() < 3 {
                    continue;
                }
                if corners.iter().any(|c| bad_v[c.0 - 1]) {
                    out.bad.dropped_faces += 1;
                    continue;
                }
                let uses_bad_vt = corners.iter().any(|c| c.1.is_some_and(|t| bad_vt[t - 1]));
                let uses_bad_vn = corners.iter().any(|c| c.2.is_some_and(|n| bad_vn[n - 1]));
                if uses_bad_vt || uses_bad_vn {
                    out.bad.degraded_faces += 1;
                    for corner in corners.iter_mut() {
                        corner.1 = corner.1.filter(|&t| !bad_vt[t - 1]);
                        corner.2 = corner.2.filter(|&n| !bad_vn[n - 1]);
                    }
                }
                let has_uv = corners.iter().all(|c| c.1.is_some());
                let has_normal = corners.iter().all(|c| c.2.is_some());
                out.text.push('f');
                for &(v, vt, vn) in &corners {
                    let _ = match (vt.filter(|_| has_uv), vn.filter(|_| has_normal)) {
                        (Some(vt), Some(vn)) => write!(out.text, " {v}/{vt}/{vn}"),
                        (Some(vt), None) => write!(out.text, " {v}/{vt}"),
                        (None, Some(vn)) => write!(out.text, " {v}//{vn}"),
                        (None, None) => write!(out.text, " {v}"),
                    };
                }
                out.text.push('\n');
                let info = TriInfo { material: current_material, has_uv, has_normal, o: current_o, g: current_g };
                out.tris.extend(std::iter::repeat_n(info, corners.len() - 2));
            }
            "usemtl" if !rest.is_empty() => {
                current_material = match material_index.get(rest) {
                    Some(&i) => i,
                    None => {
                        let i = out.material_names.len() as u32;
                        out.material_names.push(rest.to_owned());
                        material_index.insert(rest.to_owned(), i);
                        i
                    }
                };
            }
            "mtllib" if !rest.is_empty() && !out.mtllibs.iter().any(|m| m == rest) => {
                out.mtllibs.push(rest.to_owned());
            }
            "o" => current_o = intern_name(&mut o_index, &mut out.o_names, rest),
            "g" => current_g = intern_name(&mut g_index, &mut out.g_names, if rest.is_empty() { "default" } else { rest }),
            _ => {}
        }
    }
    Ok(out)
}

/// The index of `name` in `names`, appended on first use.
fn intern_name(index: &mut BTreeMap<String, u32>, names: &mut Vec<String>, name: &str) -> u32 {
    *index.entry(name.to_owned()).or_insert_with(|| {
        names.push(name.to_owned());
        names.len() as u32 - 1
    })
}

/// The objects of the file (5.21): the `o` entries when at least one face follows an `o` line, else the `g` entries
/// when at least one face follows a `g` line, else none (one object). Faces in front of the first `o` (or `g`) form
/// one unnamed object.
fn objects_of(prepass: &Prepass) -> RawObjects {
    let pick = |of: fn(&TriInfo) -> u32, names: &[String]| -> Option<RawObjects> {
        if !prepass.tris.iter().any(|t| of(t) != NO_OBJECT) {
            return None;
        }
        let mut names = names.to_vec();
        let unnamed = names.len() as u32;
        let tri_object: Vec<u32> = prepass.tris.iter().map(|t| if of(t) == NO_OBJECT { unnamed } else { of(t) }).collect();
        if tri_object.contains(&unnamed) {
            names.push(String::new());
        }
        Some(RawObjects { tri_object, names })
    };
    pick(|t| t.o, &prepass.o_names).or_else(|| pick(|t| t.g, &prepass.g_names)).unwrap_or_default()
}

/// Rewrites an MTL file into the statements the importer uses, in a form tobj parses without failing:
/// `newmtl <name>`, `Kd r g b` (a single value is used for all three channels; spectral and CIE forms are dropped),
/// `d <factor>` (the `-halo` flag is skipped) and `map_Kd` / `map_Ka` with the rest of their line. Statements in
/// front of the first `newmtl` and materials without a name are dropped.
fn sanitize_mtl(source: &str) -> String {
    let mut out = String::with_capacity(source.len() / 2);
    let mut in_material = false;
    for line in source.lines() {
        let (keyword, rest) = split_keyword(line.trim());
        if keyword.eq_ignore_ascii_case("newmtl") {
            in_material = !rest.is_empty();
            if in_material {
                let _ = writeln!(out, "newmtl {rest}");
            }
            continue;
        }
        if !in_material || rest.is_empty() {
            continue;
        }
        match keyword {
            "Kd" => {
                let values: Vec<f32> = rest.split_whitespace().map_while(|t| t.parse::<f32>().ok()).collect();
                match values[..] {
                    [r, g, b, ..] => {
                        let _ = writeln!(out, "Kd {r} {g} {b}");
                    }
                    [grey] => {
                        let _ = writeln!(out, "Kd {grey} {grey} {grey}");
                    }
                    _ => {}
                }
            }
            "d" => {
                if let Some(factor) = rest.split_whitespace().find_map(|t| t.parse::<f32>().ok()) {
                    let _ = writeln!(out, "d {factor}");
                }
            }
            _ if keyword.eq_ignore_ascii_case("map_Kd") => {
                let _ = writeln!(out, "map_Kd {rest}");
            }
            _ if keyword.eq_ignore_ascii_case("map_Ka") => {
                let _ = writeln!(out, "map_Ka {rest}");
            }
            _ => {}
        }
    }
    out
}

/// One material library that was read.
struct MtlLibrary {
    /// The folder part of the `mtllib` reference ("" when the library sits next to the OBJ).
    dir: String,
    /// The whole text (every statement, also the ones `sanitize_mtl` drops).
    text: String,
    materials: Vec<tobj::Material>,
}

/// Reads and parses one material library; `None` when it cannot be read or parsed.
fn read_mtl(resolver: &dyn AssetResolver, reference: &str) -> Option<MtlLibrary> {
    let bytes = resolver.read(reference).ok()?;
    let text = decode_text(&bytes);
    let (materials, _) = tobj::load_mtl_buf(&mut Cursor::new(sanitize_mtl(&text).as_bytes())).ok()?;
    let dir = match reference.trim().rfind(['/', '\\']) {
        Some(end) => reference.trim()[..end].to_owned(),
        None => String::new(),
    };
    Some(MtlLibrary { dir, text, materials })
}

/// Loads the libraries of all `mtllib` statements. A statement naming several files separated by spaces is
/// honoured when the line as a whole is not a file. Every library that cannot be read is a `MissingMtl` warning.
fn load_materials(
    resolver: &dyn AssetResolver,
    mtllibs: &[String],
    warnings: &mut Vec<ImportWarning>,
) -> Vec<MtlLibrary> {
    let mut libraries = Vec::new();
    for reference in mtllibs {
        if let Some(found) = read_mtl(resolver, reference) {
            libraries.push(found);
            continue;
        }
        let parts: Vec<&str> = reference.split_whitespace().collect();
        let found: Vec<Option<MtlLibrary>> =
            if parts.len() > 1 { parts.iter().map(|part| read_mtl(resolver, part)).collect() } else { Vec::new() };
        if found.iter().any(|f| f.is_some()) {
            for (part, found) in parts.iter().zip(found) {
                match found {
                    Some(found) => libraries.push(found),
                    None => warnings.push(ImportWarning::MissingMtl((*part).to_owned())),
                }
            }
        } else {
            warnings.push(ImportWarning::MissingMtl(reference.clone()));
        }
    }
    libraries
}

/// True for references that name a location outside the model's folder: drive letters, UNC and rooted paths.
fn is_absolute_ref(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    reference.starts_with(['/', '\\']) || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// The reference under which a texture named in a library is read: relative to the library's own folder when the
/// file is there (as other tools resolve it), else as written (relative to the OBJ, plus the resolver's fallbacks).
fn texture_reference(resolver: &dyn AssetResolver, library_dir: &str, reference: &str) -> String {
    if !library_dir.is_empty() && !is_absolute_ref(reference) {
        let joined = format!("{library_dir}/{reference}");
        if resolver.read(&joined).is_ok() {
            return joined;
        }
    }
    reference.to_owned()
}

/// True for the MTL statements that name a texture file (any `map_*`, `bump`, `disp`, `decal`, `refl`, `norm`).
fn is_texture_statement(keyword: &str) -> bool {
    let lower = keyword.to_ascii_lowercase();
    lower.starts_with("map_") || matches!(lower.as_str(), "bump" | "disp" | "decal" | "refl" | "norm")
}

/// Reads (and so captures in the project) every texture file the libraries name, used or not, whatever the slot:
/// normal, roughness and specular maps are part of the model even though only the base colour is edited. Files that
/// cannot be read are ignored: they do not affect the import.
fn capture_library_textures(resolver: &dyn AssetResolver, libraries: &[MtlLibrary]) {
    for library in libraries {
        for line in library.text.lines() {
            let (keyword, rest) = split_keyword(line.trim());
            if !is_texture_statement(keyword) {
                continue;
            }
            let reference = parse_texture_ref(rest);
            if !reference.is_empty() {
                let _ = resolver.read(&texture_reference(resolver, &library.dir, reference));
            }
        }
    }
}

fn vec3_at(data: &[f32], index: u32) -> Option<Vec3> {
    let i = index as usize * 3;
    Some(Vec3::new(*data.get(i)?, *data.get(i + 1)?, *data.get(i + 2)?))
}

fn vec2_at(data: &[f32], index: u32) -> Option<Vec2> {
    let i = index as usize * 2;
    Some(Vec2::new(*data.get(i)?, *data.get(i + 1)?))
}

/// The three values `data[indices[3 * tri + k]]`, `None` when anything is out of range.
fn corners_of<T: Copy + Default>(indices: &[u32], tri: usize, fetch: impl Fn(u32) -> Option<T>) -> Option<[T; 3]> {
    let mut out = [T::default(); 3];
    for (k, slot) in out.iter_mut().enumerate() {
        *slot = fetch(*indices.get(tri * 3 + k)?)?;
    }
    Some(out)
}

/// OBJ + MTL -> `RawMesh` (V flipped; MTL and textures through `resolver`; textures decoded only when `decode_textures`).
///
/// Text is read as UTF-8, else as Latin-1. Materials: the materials of the MTL files that at least one face uses,
/// in MTL order (a name defined twice means its last definition); then every `usemtl` name the MTL files do not
/// define, in order of first use (so materials stay separate when the MTL is missing); then one unnamed material
/// for faces in front of the first `usemtl`. `map_Kd` (else `map_Ka`) names the texture, read relative to the MTL's
/// own folder when the file is there, else as written; materials naming the same file (compared with
/// `normalize_ref`) share one texture index. Base colour = `Kd` with `d` as alpha. With `decode_textures` every
/// other texture file the libraries name is read too (and so captured by a `RecordingResolver`).
pub(crate) fn load_obj(
    main: &[u8],
    resolver: &dyn AssetResolver,
    opts: &ImportOptions,
    decode_textures: bool,
) -> Result<RawImport, ImportError> {
    let prepass = sanitize_obj(&decode_text(main))?;
    let mut warnings = Vec::new();
    if let Some((first_line, reason)) = &prepass.bad.first {
        let bad = &prepass.bad;
        if prepass.tris.is_empty() {
            // Nothing usable is left (a damaged or non-OBJ file): the error names the first bad line.
            return Err(obj_error(
                *first_line,
                format_args!("{reason} ({} line(s) with unusable numbers, no usable face)", bad.lines),
            ));
        }
        warnings.push(ImportWarning::BadObjNumbers {
            first_line: *first_line,
            lines: bad.lines,
            dropped_faces: bad.dropped_faces,
            degraded_faces: bad.degraded_faces,
        });
    }

    let load_options =
        tobj::LoadOptions { triangulate: true, ignore_points: true, ignore_lines: true, ..Default::default() };
    // The canonical text holds no `mtllib`, so the material loader is never called; libraries are loaded below.
    let (models, _) = tobj::load_obj_buf(&mut Cursor::new(prepass.text.as_bytes()), &load_options, |_| {
        Err(tobj::LoadError::OpenFileFailed)
    })
    .map_err(|e| ImportError::Obj(e.to_string()))?;

    // Materials: which MTL definition (if any) every `usemtl` name means, and which names are in use.
    let libraries = load_materials(resolver, &prepass.mtllibs, &mut warnings);
    // Every definition with the index of its library.
    let (library, library_of): (Vec<&tobj::Material>, Vec<usize>) =
        libraries.iter().enumerate().flat_map(|(l, lib)| lib.materials.iter().map(move |m| (m, l))).unzip();
    let mut defined: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, material) in library.iter().enumerate() {
        defined.insert(material.name.as_str(), i);
    }
    let mut name_used = vec![false; prepass.material_names.len()];
    let mut unnamed_used = false;
    for tri in &prepass.tris {
        match name_used.get_mut(tri.material as usize) {
            Some(used) => *used = true,
            None => unnamed_used = true,
        }
    }
    let definition: Vec<Option<usize>> =
        prepass.material_names.iter().map(|name| defined.get(name.as_str()).copied()).collect();

    let mut materials: Vec<RawMaterial> = Vec::new();
    let mut texture_refs: Vec<Option<String>> = Vec::new();
    let mut name_to_material = vec![NO_MATERIAL; prepass.material_names.len()];
    let mut library_slot: BTreeMap<usize, u32> = BTreeMap::new();
    let library_used: Vec<usize> = {
        let mut used: Vec<usize> =
            definition.iter().zip(&name_used).filter(|(_, used)| **used).filter_map(|(d, _)| *d).collect();
        used.sort_unstable();
        used.dedup();
        used
    };
    for &i in &library_used {
        let source = &library[i];
        let kd = source.diffuse.unwrap_or([DEFAULT_COLOR[0], DEFAULT_COLOR[1], DEFAULT_COLOR[2]]);
        let texture = source
            .diffuse_texture
            .as_deref()
            .map(parse_texture_ref)
            .filter(|r| !r.is_empty())
            .or_else(|| source.ambient_texture.as_deref().map(parse_texture_ref).filter(|r| !r.is_empty()))
            .map(|r| texture_reference(resolver, &libraries[library_of[i]].dir, r));
        library_slot.insert(i, materials.len() as u32);
        materials.push(RawMaterial {
            name: source.name.clone(),
            base_color: [
                clean_unit(kd[0], DEFAULT_COLOR[0]),
                clean_unit(kd[1], DEFAULT_COLOR[1]),
                clean_unit(kd[2], DEFAULT_COLOR[2]),
                clean_unit(source.dissolve.unwrap_or(1.0), 1.0),
            ],
            double_sided: false,
            texture: None,
        });
        texture_refs.push(texture);
    }
    for (n, name) in prepass.material_names.iter().enumerate() {
        match definition[n] {
            Some(i) => name_to_material[n] = library_slot.get(&i).copied().unwrap_or(NO_MATERIAL),
            None if name_used[n] => {
                name_to_material[n] = materials.len() as u32;
                materials.push(RawMaterial {
                    name: name.clone(),
                    base_color: DEFAULT_COLOR,
                    double_sided: false,
                    texture: None,
                });
                texture_refs.push(None);
            }
            None => {}
        }
    }
    let unnamed_material = materials.len() as u32;
    if unnamed_used {
        materials.push(RawMaterial {
            name: String::new(),
            base_color: DEFAULT_COLOR,
            double_sided: false,
            texture: None,
        });
        texture_refs.push(None);
    }

    // Textures: one per distinct file, in order of first appearance by material index.
    let mut requests: Vec<TextureRequest> = Vec::new();
    let mut texture_index: BTreeMap<String, u32> = BTreeMap::new();
    for (material, reference) in materials.iter_mut().zip(&texture_refs) {
        let Some(reference) = reference else { continue };
        let index = *texture_index.entry(normalize_ref(reference)).or_insert_with(|| {
            requests.push(TextureRequest {
                name: file_stem_of(reference).to_owned(),
                reference: reference.clone(),
                material: material.name.clone(),
                name_hint: Some(reference.clone()),
                source: TextureSource::Resolver,
            });
            requests.len() as u32 - 1
        });
        material.texture = Some(index);
    }
    let textures = load_textures(requests, resolver, opts, decode_textures, &mut warnings);
    if decode_textures {
        capture_library_textures(resolver, &libraries);
    }

    // Triangles: tobj's output in file order, lined up with the facts of the pre-pass.
    let produced: usize = models.iter().map(|m| m.mesh.indices.len() / 3).sum();
    if produced != prepass.tris.len() {
        return Err(ImportError::Obj(format!(
            "internal error: {produced} triangles were parsed where {} were expected",
            prepass.tris.len()
        )));
    }
    let inconsistent = || ImportError::Obj("internal error: inconsistent vertex data".to_owned());
    let mut triangles = Vec::with_capacity(produced);
    let mut infos = prepass.tris.iter();
    for model in &models {
        let mesh = &model.mesh;
        let uv_indexed = mesh.texcoord_indices.len() == mesh.indices.len();
        let normal_indexed = mesh.normal_indices.len() == mesh.indices.len();
        for (tri, info) in (0..mesh.indices.len() / 3).zip(&mut infos) {
            let positions = corners_of(&mesh.indices, tri, |i| vec3_at(&mesh.positions, i)).ok_or_else(inconsistent)?;
            let uvs = if info.has_uv && uv_indexed {
                let uv = corners_of(&mesh.texcoord_indices, tri, |i| vec2_at(&mesh.texcoords, i))
                    .ok_or_else(inconsistent)?;
                // OBJ has its UV origin bottom-left: flip V once, here (section 4.1).
                Some(uv.map(|t| Vec2::new(t.x, 1.0 - t.y)))
            } else {
                None
            };
            let normals = if info.has_normal && normal_indexed {
                Some(corners_of(&mesh.normal_indices, tri, |i| vec3_at(&mesh.normals, i)).ok_or_else(inconsistent)?)
            } else {
                None
            };
            let material = match name_to_material.get(info.material as usize) {
                Some(&m) if m != NO_MATERIAL => m,
                _ => unnamed_material,
            };
            triangles.push(RawTriangle { positions, normals, uvs, material });
        }
    }

    let objects = objects_of(&prepass);
    Ok(RawImport { mesh: RawMesh { triangles, materials }, objects, textures, warnings })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::import::MemoryResolver;
    use crate::samples;

    fn resolver(files: &[(&str, &str)]) -> MemoryResolver {
        let mut resolver = MemoryResolver::default();
        for (name, text) in files {
            resolver.insert(name, Arc::from(text.as_bytes()));
        }
        resolver
    }

    fn load(obj: &str, files: &[(&str, &str)]) -> RawImport {
        load_obj(obj.as_bytes(), &resolver(files), &ImportOptions::default(), false).unwrap()
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).abs().max_element() <= 1e-6
    }

    fn close2(a: Vec2, b: Vec2) -> bool {
        (a - b).abs().max_element() <= 1e-6
    }

    /// Compares an import with the raw mesh it was written from, triangle by triangle.
    fn assert_same_mesh(imported: &RawMesh, original: &RawMesh) {
        assert_eq!(imported.triangles.len(), original.triangles.len());
        for (i, (a, b)) in imported.triangles.iter().zip(&original.triangles).enumerate() {
            for k in 0..3 {
                assert!(close(a.positions[k], b.positions[k]), "triangle {i} position {k}");
            }
            assert_eq!(a.uvs.is_some(), b.uvs.is_some(), "triangle {i} uv presence");
            if let (Some(ua), Some(ub)) = (a.uvs, b.uvs) {
                for k in 0..3 {
                    assert!(close2(ua[k], ub[k]), "triangle {i} uv {k}: {:?} vs {:?}", ua[k], ub[k]);
                }
            }
            assert_eq!(a.normals.is_some(), b.normals.is_some(), "triangle {i} normal presence");
            if let (Some(na), Some(nb)) = (a.normals, b.normals) {
                for k in 0..3 {
                    assert!(close(na[k], nb[k]), "triangle {i} normal {k}");
                }
            }
            assert_eq!(a.material, b.material, "triangle {i} material");
        }
    }

    #[test]
    fn round_trip_through_to_obj_reproduces_positions_and_uvs() {
        let fixtures = [
            samples::uv_sphere(12, 6, 1.5),
            samples::seam_cube(2.0),
            samples::two_material_model(),
            samples::mirrored_quads(),
            samples::wrapped_quad(),
            samples::flat_quad(),
            samples::no_uv_tetra(),
            samples::tight_islands(),
            samples::occluded_quads(),
        ];
        for original in fixtures {
            let (obj, mtl) = samples::to_obj(&original, "m.mtl", &["a.png".to_owned(), "b.png".to_owned()]);
            let import = load(&obj, &[("m.mtl", &mtl)]);
            assert!(import.warnings.is_empty(), "{:?}", import.warnings);
            assert_same_mesh(&import.mesh, &original);
            assert_eq!(import.mesh.materials.len(), original.materials.len());
            for (a, b) in import.mesh.materials.iter().zip(&original.materials) {
                assert_eq!(a.name, b.name);
                assert_eq!(a.texture, b.texture);
                assert_eq!(a.base_color, b.base_color);
                assert!(!a.double_sided);
            }
        }
    }

    #[test]
    fn v_is_flipped_exactly_once() {
        // OBJ texture space has v up: the vertex at the top of the quad (y = 1) has vt v = 1 and must become v = 0.
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3\nf 1/1 3/3 4/4\n";
        let mesh = load(obj, &[]).mesh;
        let uv = mesh.triangles[0].uvs.unwrap();
        assert_eq!(uv, [Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0)]);
        assert_eq!(mesh.triangles[1].uvs.unwrap()[2], Vec2::new(0.0, 0.0));
        // Same layout as the built-in flat quad (u = x, v = 1 - y).
        assert_same_mesh(&mesh, &RawMesh { materials: Vec::new(), ..samples::flat_quad() });
    }

    #[test]
    fn quads_and_polygons_are_triangulated() {
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv -1 0.5 0\nf 1 2 3 4\nf 1 2 3 4 5\n";
        let mesh = load(obj, &[]).mesh;
        assert_eq!(mesh.triangles.len(), 2 + 3);
        let p = |i: usize| mesh.triangles[i].positions;
        let v = [
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(-1.0, 0.5, 0.0),
        ];
        // Fans around the first corner, winding preserved.
        assert_eq!(p(0), [v[0], v[1], v[2]]);
        assert_eq!(p(1), [v[0], v[2], v[3]]);
        assert_eq!(p(2), [v[0], v[1], v[2]]);
        assert_eq!(p(3), [v[0], v[2], v[3]]);
        assert_eq!(p(4), [v[0], v[3], v[4]]);
        for t in &mesh.triangles {
            assert!((t.positions[1] - t.positions[0]).cross(t.positions[2] - t.positions[0]).z > 0.0);
            assert!(t.uvs.is_none() && t.normals.is_none());
        }
    }

    #[test]
    fn mixed_faces_with_and_without_vt_do_not_misalign() {
        // Face 1 has UVs and normals, face 2 neither, face 3 only UVs (a quad), face 4 only normals.
        let obj = "\
v 0 0 0\nv 1 0 0\nv 0 1 0\nv 2 0 0\nv 3 0 0\nv 2 1 0\nv 4 0 0\nv 5 0 0\nv 5 1 0\nv 4 1 0\nv 6 0 0\nv 7 0 0\nv 6 1 0\n\
vt 0.125 0.25\nvt 0.375 0.25\nvt 0.125 0.625\nvt 0.5 0.5\nvt 0.75 0.5\nvt 0.75 0.75\nvt 0.5 0.75\n\
vn 0 0 1\nvn 0 1 0\n\
f 1/1/1 2/2/1 3/3/1\n\
f 4 5 6\n\
f 7/4 8/5 9/6 10/7\n\
f 11//2 12//2 13//2\n";
        let mesh = load(obj, &[]).mesh;
        assert_eq!(mesh.triangles.len(), 5);
        let t = &mesh.triangles;
        assert_eq!(t[0].uvs.unwrap(), [Vec2::new(0.125, 0.75), Vec2::new(0.375, 0.75), Vec2::new(0.125, 0.375)]);
        assert_eq!(t[0].normals.unwrap(), [Vec3::Z; 3]);
        assert!(t[1].uvs.is_none() && t[1].normals.is_none(), "no stale UVs or normals from the previous face");
        assert_eq!(t[1].positions[0], Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(t[2].uvs.unwrap(), [Vec2::new(0.5, 0.5), Vec2::new(0.75, 0.5), Vec2::new(0.75, 0.25)]);
        assert_eq!(t[3].uvs.unwrap(), [Vec2::new(0.5, 0.5), Vec2::new(0.75, 0.25), Vec2::new(0.5, 0.25)]);
        assert!(t[2].normals.is_none() && t[3].normals.is_none());
        assert!(t[4].uvs.is_none());
        assert_eq!(t[4].normals.unwrap(), [Vec3::Y; 3]);
        assert_eq!(t[4].positions[2], Vec3::new(6.0, 1.0, 0.0));

        // The very first face without vt / vn while the file has both.
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.25 0.25\nvt 0.5 0.25\nvt 0.25 0.5\nvn 1 0 0\nf 1 2 3\nf 1/1/1 2/2/1 3/3/1\n";
        let mesh = load(obj, &[]).mesh;
        assert!(mesh.triangles[0].uvs.is_none() && mesh.triangles[0].normals.is_none());
        assert_eq!(mesh.triangles[1].uvs.unwrap()[1], Vec2::new(0.5, 0.75));
        assert_eq!(mesh.triangles[1].normals.unwrap(), [Vec3::X; 3]);

        // A face with a UV on only some corners counts as unmapped.
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.25 0.25\nvt 0.5 0.25\nf 1/1 2/2 3\nf 1/1 2/2 3/9\n";
        let mesh = load(obj, &[]).mesh;
        assert!(mesh.triangles[0].uvs.is_none());
        assert!(mesh.triangles[1].uvs.is_none(), "an out-of-range vt is treated as missing");
    }

    #[test]
    fn usemtl_splits_materials() {
        let obj = "\
mtllib two.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\n\
f 1 2 3\nusemtl Red\nf 1 2 4\nusemtl Blue\nf 1 3 4\nusemtl Red\nf 2 3 4\ng another\nf 3 2 1\n";
        let mtl = "\
newmtl Unused\nKd 0 1 0\nmap_Kd unused.png\n\n\
newmtl Blue\nKd 0 0 1\nd 0.5\nmap_Kd shared.png\n\n\
newmtl Red\nKd 1 0 0\nmap_Ka -clamp on Shared.PNG\n";
        let import = load(obj, &[("two.mtl", mtl)]);
        let names: Vec<&str> = import.mesh.materials.iter().map(|m| m.name.as_str()).collect();
        // Used MTL materials in MTL order, then the unnamed material of the face in front of the first usemtl.
        assert_eq!(names, ["Blue", "Red", ""]);
        assert_eq!(import.mesh.materials[0].base_color, [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(import.mesh.materials[1].base_color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(import.mesh.materials[2].base_color, DEFAULT_COLOR);
        let per_triangle: Vec<u32> = import.mesh.triangles.iter().map(|t| t.material).collect();
        assert_eq!(per_triangle, [2, 1, 0, 1, 1], "`g` does not reset the material");
        // Both used materials name the same file (case and options aside): one texture, taken from map_Kd first.
        assert_eq!(import.textures.len(), 1);
        assert_eq!(import.textures[0].reference, "shared.png");
        assert_eq!(import.textures[0].name, "shared");
        assert!(import.textures[0].image.is_none());
        assert_eq!(import.mesh.materials[0].texture, Some(0));
        assert_eq!(import.mesh.materials[1].texture, Some(0));
        assert_eq!(import.mesh.materials[2].texture, None);
        assert!(import.warnings.is_empty());
    }

    #[test]
    fn map_kd_options_are_stripped() {
        let obj = "mtllib m.mtl\nusemtl A\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let mtl = "newmtl A\nmap_Kd -s 1 1 1 my tex.png\n";
        let import = load(obj, &[("m.mtl", mtl)]);
        assert_eq!(import.textures.len(), 1);
        assert_eq!(import.textures[0].reference, "my tex.png");
        assert_eq!(import.textures[0].name, "my tex");
        assert_eq!(import.mesh.materials[0].texture, Some(0));
        assert_eq!(import.mesh.materials[0].base_color, DEFAULT_COLOR);
    }

    #[test]
    fn missing_mtl_is_a_warning_and_materials_stay_separate() {
        let obj = "mtllib gone.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nusemtl Skin\nf 1 2 3\nusemtl Cloth\nf 1 2 4\n";
        let import = load(obj, &[]);
        assert_eq!(import.warnings, [ImportWarning::MissingMtl("gone.mtl".to_owned())]);
        let names: Vec<&str> = import.mesh.materials.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Skin", "Cloth"]);
        assert_eq!(import.mesh.triangles[1].material, 1);
        assert!(import.textures.is_empty());

        // An unparsable library is reported the same way; a library list on one line is split.
        let obj = "mtllib a.mtl b.mtl\nmtllib a.mtl b.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl FromA\nf 1 2 3\n";
        let import = load(obj, &[("a.mtl", "newmtl FromA\nKd 0.25\n")]);
        assert_eq!(import.warnings, [ImportWarning::MissingMtl("b.mtl".to_owned())]);
        assert_eq!(import.mesh.materials[0].name, "FromA");
        assert_eq!(import.mesh.materials[0].base_color, [0.25, 0.25, 0.25, 1.0]);
    }

    #[test]
    fn latin1_bytes_are_accepted() {
        // "Küche" in Latin-1 (0xFC), which is not valid UTF-8, in a comment, a material name and a texture name.
        let mut obj = b"# K\xfcche\nmtllib k.mtl\nusemtl K\xfcche\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec();
        obj.extend_from_slice(b"# fin\n");
        let mtl = b"newmtl K\xfcche\nmap_Kd K\xfcche.png\n";
        let mut files = MemoryResolver::default();
        files.insert("k.mtl", Arc::from(&mtl[..]));
        let import = load_obj(&obj, &files, &ImportOptions::default(), false).unwrap();
        assert_eq!(import.mesh.triangles.len(), 1);
        assert_eq!(import.mesh.materials[0].name, "K\u{fc}che");
        assert_eq!(import.textures[0].reference, "K\u{fc}che.png");
        assert_eq!(import.textures[0].name, "K\u{fc}che");
        // UTF-8 with a byte order mark works as well.
        let mut utf8 = b"\xef\xbb\xbf".to_vec();
        utf8.extend_from_slice("usemtl K\u{fc}che\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".as_bytes());
        let import = load_obj(&utf8, &files, &ImportOptions::default(), false).unwrap();
        assert_eq!(import.mesh.materials[0].name, "K\u{fc}che");
    }

    #[test]
    fn relative_indices_comments_and_odd_statements() {
        let obj = "\
# header\r\n\
o thing\r\n\
v 0 0 0 1.0   # weight\r\n\
v 1 0 0 0.5 0.5 0.5\r\n\
v 0 1 0 # trailing comment\r\n\
vt 0 0 0\r\nvt 1 0\r\nvt 0.5\r\n\
vn 0 0 1\r\n\
s off\r\n\
l 1 2 3\r\n\
p 1\r\n\
f 1 2\r\n\
f -3/-3/-1 -2/-2/-1 \\\r\n  -1/-1/-1   # continued\r\n\
usemtl\r\n\
unknown statement\r\n";
        let mesh = load(obj, &[]).mesh;
        assert_eq!(mesh.triangles.len(), 1, "lines, points and two-corner faces are not triangles");
        let t = &mesh.triangles[0];
        assert_eq!(t.positions, [Vec3::ZERO, Vec3::X, Vec3::Y], "weights and colours are ignored");
        assert_eq!(t.uvs.unwrap(), [Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(0.5, 1.0)]);
        assert_eq!(t.normals.unwrap(), [Vec3::Z; 3]);
        assert_eq!(mesh.materials.len(), 1);
        assert_eq!(mesh.materials[0].name, "");
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        let err = |obj: &str| match load_obj(obj.as_bytes(), &resolver(&[]), &ImportOptions::default(), false) {
            Err(ImportError::Obj(message)) => message,
            Err(other) => panic!("unexpected error {other:?}"),
            Ok(_) => panic!("expected an error for {obj:?}"),
        };
        assert!(err("v 0 0\n").contains("line 1"));
        assert!(err("v 0 0 0\nv 1 0 zero\n").contains("line 2"));
        assert!(err("vn 0 0\n").contains("three numbers"));
        assert!(err("vt\n").contains("vt"));
        assert!(err("vt 0 x\n").contains("line 1"));
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 x\n").contains("line 4"));
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/1/1/1 2 3\n").contains("invalid face element"));
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n").contains("does not exist"));
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n").contains("does not exist"));
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 -4\n").contains("does not exist"));
        // Indices far outside any range (these overflow inside tobj when passed through).
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 -9223372036854775807\n").contains("does not exist"));
        assert!(err("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99999999999999999999\n").contains("invalid face element"));
        // Out-of-range vt / vn only cost the face that attribute.
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvn 0 0 1\nf 1/-9223372036854775807/1 2/1/1 3/1/7\n";
        let mesh = load(obj, &[]).mesh;
        assert!(mesh.triangles[0].uvs.is_none() && mesh.triangles[0].normals.is_none());
        // An empty file is not an error at this level (the caller reports `ImportError::Empty`).
        assert!(load("", &[]).mesh.triangles.is_empty());
        assert!(load("f\nf \nusemtl x\n", &[]).mesh.triangles.is_empty());
    }

    #[test]
    fn bad_number_lines_keep_the_numbering() {
        let obj = "\
v 0 0 0\nv nan 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 abc\nv 1 1 0\n\
vt 0 0\nvt inf 0\nvt 1 0\nvt 0 1\n\
vn 0 0 1\nvn 0 0 1e39\n\
f 1 3 4\n\
f 1 2 3\n\
f 3/3/1 6/1/1 4/4/1\n\
f 1/1/1 3/2/1 4/4/2\n\
f 5 6 4\n\
f 1/1/1 3/3/2 4/4/1\n\
f 1/1 3/3 999999/3\n\
f 0 1 3\n";
        let import = load(obj, &[]);
        assert_eq!(
            import.warnings,
            [ImportWarning::BadObjNumbers { first_line: 2, lines: 6, dropped_faces: 4, degraded_faces: 2 }]
        );
        let text = import.warnings[0].to_string();
        assert!(text.contains("line 2") && text.contains("6 line(s)") && text.contains("4 face(s)"), "{text}");
        let t = &import.mesh.triangles;
        assert_eq!(t.len(), 4, "the faces using v 2, v 5 or a vertex that does not exist are dropped");
        // Indices after a bad line still point at the right vertices.
        assert_eq!(t[0].positions, [Vec3::ZERO, Vec3::X, Vec3::Y]);
        assert_eq!(t[1].positions, [Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y]);
        assert_eq!(t[1].uvs.unwrap(), [Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, 0.0)]);
        assert_eq!(t[1].normals.unwrap(), [Vec3::Z; 3]);
        // A bad vt makes the face unmapped, a bad vn leaves its normals to be generated.
        assert!(t[2].uvs.is_none() && t[2].normals.is_none());
        assert_eq!(t[3].uvs.unwrap()[1], Vec2::new(1.0, 1.0));
        assert!(t[3].normals.is_none());
        assert!(t.iter().all(|t| t.positions.iter().all(|p| p.is_finite())));
    }

    #[test]
    fn files_without_usable_numbers_still_fail() {
        let err = |obj: &str| match load_obj(obj.as_bytes(), &resolver(&[]), &ImportOptions::default(), false) {
            Err(ImportError::Obj(message)) => message,
            Err(other) => panic!("unexpected error {other:?}"),
            Ok(_) => panic!("expected an error for {obj:?}"),
        };
        let message = err("# junk\nv a b c\nv nan nan nan\nvt x\nf 1 2 1\n");
        assert!(message.starts_with("line 2: `v` needs three numbers; the first is not a number"), "{message}");
        assert!(message.contains("3 line(s)"), "{message}");
        assert!(err("v 0 0 inf\nv 0 1 0\nv 1 0 0\nf 1 2 3\n").contains("not a finite number"));
        assert!(err("vn 0 0 1\nvt 0 -inf\n").contains("line 2: `vt` has a second coordinate"));
        // Valid lines after the first bad one do not hide the problem when no face survives.
        assert!(err("v 1e39 0 0\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 3 4\n").contains("line 1"));
    }

    #[test]
    fn mtl_statements_that_break_tobj_are_tolerated() {
        let obj = "mtllib odd.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl A\nf 1 2 3\nusemtl B\nf 3 2 1\nusemtl C\nf 1 3 2\n";
        let mtl = "\
Kd 1 1 1\n\
newmtl\n\
Kd 0.1 0.1 0.1\n\
newmtl A\n\
Kd 0.5\n\
Ka spectral file.rfl 1.0\n\
Ks\n\
Ns shiny\n\
illum\n\
d -halo 0.25\n\
map_Kd\n\
map_bump\n\
newmtl B\n\
Kd spectral file.rfl\n\
Kd xyz 0.1 0.2 0.3\n\
d nothing\n\
map_kd -bm 2 b tex.tga\n\
newmtl C\n\
Kd 2 -1 nan\n\
Tr 0.3\n";
        let import = load(obj, &[("odd.mtl", mtl)]);
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);
        let m = &import.mesh.materials;
        assert_eq!(m.len(), 3);
        assert_eq!((m[0].name.as_str(), m[0].base_color, m[0].texture), ("A", [0.5, 0.5, 0.5, 0.25], None));
        assert_eq!((m[1].name.as_str(), m[1].base_color, m[1].texture), ("B", DEFAULT_COLOR, Some(0)));
        assert_eq!(m[2].base_color, [1.0, 0.0, 0.8, 1.0], "out-of-range and non-finite colours are cleaned");
        assert_eq!(import.textures[0].reference, "b tex.tga");
    }

    #[test]
    fn sanitized_text_is_canonical() {
        let pre = sanitize_obj("v 1 2 3 4\nvt 0.5\nvn 0 1 0\nf -1/-1/-1 1/1/1 1/1/1\nmtllib a b.mtl\ng x\n").unwrap();
        assert_eq!(pre.text, "v 1 2 3\nvt 0.5 0\nvn 0 1 0\nf 1/1/1 1/1/1 1/1/1\n");
        assert_eq!(pre.mtllibs, ["a b.mtl"]);
        assert_eq!(pre.tris.len(), 1);
        assert_eq!(pre.tris[0].material, NO_MATERIAL);
        assert_eq!(sanitize_mtl("newmtl  M 1 \n Kd 1 0.5 0 \nNs 10\nmap_Kd  a.png \n"), "newmtl M 1\nKd 1 0.5 0\nmap_Kd a.png\n");
    }

    // ----- objects-tabs WP1: `o` / `g` as objects -----

    /// Six vertices: a triangle at x = 0 and one at x = 5, then the faces of `body`.
    fn with_vertices(body: &str) -> String {
        format!("v 0 0 0\nv 1 0 0\nv 0 1 0\nv 5 0 0\nv 6 0 0\nv 5 1 0\nv 6 1 0\n{body}")
    }

    fn objects_of_text(body: &str) -> RawObjects {
        load(&with_vertices(body), &[]).objects
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn o_lines_are_objects() {
        // Faces in front of the first `o` form an unnamed object; a re-entered name continues its object; a quad is
        // two triangles of its object; the whole rest of the line is the name.
        let objects = objects_of_text("f 1 2 3\no Alpha one\nf 4 5 7 6\ng grp\no Beta\nf 1 2 3\no Alpha one\nf 4 5 6\n");
        assert_eq!(objects.names, names(&["Alpha one", "Beta", ""]));
        assert_eq!(objects.tri_object, [2, 0, 0, 1, 0]);
        let (mesh, _) = crate::mesh_build::normalize_with_objects(
            load(&with_vertices("f 1 2 3\no Alpha one\nf 4 5 7 6\no Beta\nf 1 2 3\n"), &[]).mesh,
            &objects_of_text("f 1 2 3\no Alpha one\nf 4 5 7 6\no Beta\nf 1 2 3\n"),
            &crate::mesh_build::NormalizeOptions::default(),
        );
        let got: Vec<&str> = mesh.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(got, ["Object 1", "Alpha one", "Beta"]);
    }

    #[test]
    fn g_groups_are_objects_when_there_is_no_o() {
        let objects = objects_of_text("g a\nf 1 2 3\ng b\nf 4 5 6\ng a\nf 4 5 6\ng\nf 1 2 3\n");
        assert_eq!(objects.names, names(&["a", "b", "default"]));
        assert_eq!(objects.tri_object, [0, 1, 0, 2]);
        // Faces in front of the first `g`.
        let objects = objects_of_text("f 1 2 3\ng a\nf 4 5 6\n");
        assert_eq!((objects.names, objects.tri_object), (names(&["a", ""]), vec![1, 0]));
        // An `o` with faces after it wins over the groups.
        let objects = objects_of_text("o X\ng a\nf 1 2 3\ng b\nf 4 5 6\n");
        assert_eq!((objects.names, objects.tri_object), (names(&["X"]), vec![0, 0]));
        // An `o` without faces after it does not: the groups are the objects.
        let objects = objects_of_text("g a\nf 1 2 3\ng b\nf 4 5 6\no Late\n");
        assert_eq!((objects.names, objects.tri_object), (names(&["a", "b"]), vec![0, 1]));
    }

    #[test]
    fn a_file_without_o_or_g_is_one_object_named_after_the_model() {
        assert_eq!(objects_of_text("f 1 2 3\nf 4 5 6\n"), RawObjects::default());
        let file = |name: &str, text: String| crate::import::SourceFile {
            key: crate::import::normalize_ref(name),
            original_name: name.to_owned(),
            bytes: Arc::from(text.into_bytes()),
        };
        let bundle = crate::import::SourceBundle {
            format: crate::import::ModelFormat::Obj,
            main: file("Thing.obj", with_vertices("f 1 2 3\nf 4 5 6\n")),
            sidecars: Vec::new(),
            extras: Vec::new(),
            original_path: None,
        };
        let model =
            crate::import::import_from_bundle(&bundle, &ImportOptions::default(), false, &crate::progress::NoProgress).unwrap();
        assert_eq!(model.mesh.objects.len(), 1);
        assert_eq!(model.mesh.objects[0].name, model.name);
        assert_eq!(model.mesh.objects[0].tri_count, 2);
        // The same file with objects: the triangle order and uv_hash do not change.
        let mut with = bundle.clone();
        with.main = file("Thing.obj", with_vertices("o A\nf 1 2 3\no B\nf 4 5 6\n"));
        let split =
            crate::import::import_from_bundle(&with, &ImportOptions::default(), false, &crate::progress::NoProgress).unwrap();
        assert_eq!(split.mesh.uv_hash, model.mesh.uv_hash);
        assert_eq!(split.mesh.indices, model.mesh.indices);
        let got: Vec<&str> = split.mesh.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(got, ["A", "B"]);
    }

    #[test]
    fn to_obj_objects_round_trips_the_object_trio() {
        let (raw, objects) = samples::object_trio();
        let (obj, mtl) = samples::to_obj_objects(&raw, &objects, "m.mtl", &["t.png".to_owned()]);
        let import = load(&obj, &[("m.mtl", &mtl)]);
        assert_same_mesh(&import.mesh, &raw);
        assert_eq!(import.objects, objects);
        // `to_obj` itself writes no `o` line.
        assert!(!samples::to_obj(&raw, "m.mtl", &[]).0.lines().any(|l| l.starts_with("o ")));
    }
}
