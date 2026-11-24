// Owner: core-import (C5). Autodesk FBX -> `RawMesh` (ARCHITECTURE.md 5.21, FBX notes: section 10).
//
// Two layers:
// 1. A node-tree reader for both encodings: binary FBX 6000..7999 (32-bit record headers below 7500, 64-bit from
//    7500; scalar properties Y C I F D L, strings S, raw bytes R, arrays f d l i b c with encoding 0 = plain or
//    1 = zlib via miniz_oxide) and ASCII FBX (6.x inline arrays with comma continuation lines, 7.x `*N { a: ... }`
//    arrays, base64 `Content`). Every length is bounds-checked, nesting is limited to `MAX_DEPTH`, decompressed
//    data is limited to a budget derived from the file size, so hostile input errors out instead of panicking,
//    hanging or exhausting memory.
// 2. The scene: `Objects` (Model, Geometry, Material, Texture, Video, Deformer) linked by `Connections`
//    (OO / OP; 6.x links by name, 7.x by id). Mesh geometry is baked into world space through the model hierarchy
//    (Lcl Translation / Rotation / Scaling with pre / post rotation, pivots, offsets, rotation order and the
//    geometric transform), converted to the app's Y-up axes and to metres (`GlobalSettings`). Polygons are
//    triangulated (ear clipping, fan as the fallback); normals, the first UV set and material indices come from the
//    layer elements with every mapping / reference mode; the UV transform of a material's texture (Translation /
//    Rotation / Scaling, or the older ModelUVTranslation / ModelUVScaling) is applied to its polygons' UVs. Skin
//    and blend-shape deformers are ignored (`SkinIgnored`): FBX stores the vertices in their bind pose.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use glam::{DMat3, DMat4, DVec2, DVec3, Vec2, Vec3};

use crate::error::{ImportError, ImportWarning};
use crate::import::resolver::{file_name_of, file_stem_of};
use crate::import::{
    AssetResolver, ImportOptions, RawImport, TextureRequest, TextureSource, base64_decode, clean_unit, decode_text,
    load_textures, normalize_ref,
};
use crate::mesh::{RawMaterial, RawMesh, RawObjects, RawTriangle};

/// Deepest node nesting accepted (real files stay below 10).
const MAX_DEPTH: usize = 64;
/// Polygons with more corners than this are fan-triangulated (ear clipping is quadratic).
const MAX_EAR_CLIP: usize = 512;
/// Base colour of materials without a diffuse colour and of geometry without a material.
const DEFAULT_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

fn invalid(message: impl std::fmt::Display) -> ImportError {
    ImportError::InvalidFbx(message.to_string())
}

// ---------------------------------------------------------------------------------------------------------------
// Node tree
// ---------------------------------------------------------------------------------------------------------------

/// One property value of a node.
#[derive(Clone, Debug, PartialEq)]
enum Prop {
    Int(i64),
    Float(f64),
    Str(String),
    Raw(Vec<u8>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

impl Prop {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Prop::Int(v) => Some(*v as f64),
            Prop::Float(v) => Some(*v),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Prop::Int(v) => Some(*v),
            Prop::Float(v) if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.0e18 => Some(*v as i64),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Node {
    name: String,
    props: Vec<Prop>,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// First child whose name matches without regard to case ("FileName" / "Filename").
    fn child_ci(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    fn prop_str(&self, i: usize) -> Option<&str> {
        self.props.get(i).and_then(Prop::as_str)
    }

    /// The numbers of the node: its array property, else all its numeric scalar properties (ASCII 6.x arrays).
    fn floats(&self) -> Cow<'_, [f64]> {
        match self.props.as_slice() {
            [Prop::Floats(v)] => Cow::Borrowed(v),
            [Prop::Ints(v)] => Cow::Owned(v.iter().map(|&i| i as f64).collect()),
            props => Cow::Owned(props.iter().filter_map(Prop::as_f64).collect()),
        }
    }

    /// The integers of the node, like `floats`.
    fn ints(&self) -> Cow<'_, [i64]> {
        match self.props.as_slice() {
            [Prop::Ints(v)] => Cow::Borrowed(v),
            [Prop::Floats(v)] => Cow::Owned(v.iter().map(|&f| f as i64).collect()),
            props => Cow::Owned(props.iter().filter_map(Prop::as_i64).collect()),
        }
    }

    /// The string of a child like `MappingInformationType: "ByPolygon"`.
    fn child_str(&self, name: &str) -> Option<&str> {
        self.child_ci(name).and_then(|c| c.prop_str(0))
    }
}

/// Splits "Name\0\x01Class" (binary) or "Class::Name" (ASCII) into the plain name.
fn object_name(raw: &str) -> &str {
    if let Some((name, _)) = raw.split_once("\0\u{1}") {
        return name;
    }
    match raw.split_once("::") {
        Some((_, name)) => name,
        None => raw,
    }
}

// ---- binary ----------------------------------------------------------------------------------------------------

const BINARY_MAGIC: &[u8] = b"Kaydara FBX Binary  \0";

struct BinaryReader<'a> {
    data: &'a [u8],
    /// 64-bit record headers (version >= 7500).
    wide: bool,
    /// Bytes array properties may still decompress to.
    budget: u64,
}

impl<'a> BinaryReader<'a> {
    fn slice(&self, at: usize, len: usize, end: usize) -> Result<&'a [u8], ImportError> {
        let data: &'a [u8] = self.data;
        match at.checked_add(len) {
            Some(stop) if stop <= end && stop <= data.len() => Ok(&data[at..stop]),
            _ => Err(invalid(format!("data at byte {at} runs past its record"))),
        }
    }

    /// `len` bytes at `*at`, advancing `*at`.
    fn take(&self, at: &mut usize, len: usize, end: usize) -> Result<&'a [u8], ImportError> {
        let bytes = self.slice(*at, len, end)?;
        *at += len;
        Ok(bytes)
    }

    fn u32_at(&self, at: usize, end: usize) -> Result<u32, ImportError> {
        let b = self.slice(at, 4, end)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64_at(&self, at: usize, end: usize) -> Result<u64, ImportError> {
        let b = self.slice(at, 8, end)?;
        Ok(u64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    fn header_len(&self) -> usize {
        if self.wide { 25 } else { 13 }
    }

    /// Reads records from `*pos` until a null record, `end`, or too little room for a header.
    fn list(&mut self, pos: &mut usize, end: usize, depth: usize) -> Result<Vec<Node>, ImportError> {
        if depth > MAX_DEPTH {
            return Err(invalid("the nodes are nested too deeply"));
        }
        let mut nodes = Vec::new();
        while end.saturating_sub(*pos) >= self.header_len() {
            let start = *pos;
            let (end_offset, prop_count, prop_len) = if self.wide {
                (self.u64_at(start, end)?, self.u64_at(start + 8, end)?, self.u64_at(start + 16, end)?)
            } else {
                (self.u32_at(start, end)? as u64, self.u32_at(start + 4, end)? as u64, self.u32_at(start + 8, end)? as u64)
            };
            let name_len = self.slice(start + self.header_len() - 1, 1, end)?[0] as usize;
            if end_offset == 0 && prop_count == 0 && prop_len == 0 && name_len == 0 {
                *pos = start + self.header_len();
                break;
            }
            let record_end = usize::try_from(end_offset).ok().filter(|&e| e > start && e <= end).ok_or_else(|| {
                invalid(format!("the record at byte {start} ends outside its parent"))
            })?;
            let name_at = start + self.header_len();
            let name = String::from_utf8_lossy(self.slice(name_at, name_len, record_end)?).into_owned();
            let props_at = name_at + name_len;
            let props_end = usize::try_from(prop_len)
                .ok()
                .and_then(|l| props_at.checked_add(l))
                .filter(|&e| e <= record_end)
                .ok_or_else(|| invalid(format!("the properties of \"{name}\" run past the record")))?;
            let mut props = Vec::with_capacity(prop_count.min(64) as usize);
            let mut at = props_at;
            for _ in 0..prop_count {
                props.push(self.prop(&mut at, props_end)?);
            }
            let mut child_at = props_end;
            let children = if child_at < record_end { self.list(&mut child_at, record_end, depth + 1)? } else { Vec::new() };
            nodes.push(Node { name, props, children });
            *pos = record_end;
        }
        Ok(nodes)
    }

    fn prop(&mut self, at: &mut usize, end: usize) -> Result<Prop, ImportError> {
        let code = self.take(at, 1, end)?[0];
        Ok(match code {
            b'Y' => Prop::Int(i16::from_le_bytes(self.take(at, 2, end)?.try_into().expect("2")) as i64),
            b'C' => Prop::Int(self.take(at, 1, end)?[0] as i64),
            b'I' => Prop::Int(i32::from_le_bytes(self.take(at, 4, end)?.try_into().expect("4")) as i64),
            b'L' => Prop::Int(i64::from_le_bytes(self.take(at, 8, end)?.try_into().expect("8"))),
            b'F' => Prop::Float(f32::from_le_bytes(self.take(at, 4, end)?.try_into().expect("4")) as f64),
            b'D' => Prop::Float(f64::from_le_bytes(self.take(at, 8, end)?.try_into().expect("8"))),
            b'S' | b'R' => {
                let len = self.u32_at(*at, end)? as usize;
                *at += 4;
                let bytes = self.take(at, len, end)?;
                if code == b'S' { Prop::Str(String::from_utf8_lossy(bytes).into_owned()) } else { Prop::Raw(bytes.to_vec()) }
            }
            b'f' | b'd' | b'l' | b'i' | b'b' | b'c' => {
                let count = self.u32_at(*at, end)? as u64;
                let encoding = self.u32_at(*at + 4, end)?;
                let stored = self.u32_at(*at + 8, end)? as usize;
                *at += 12;
                let elem: u64 = match code {
                    b'd' | b'l' => 8,
                    b'f' | b'i' => 4,
                    _ => 1,
                };
                let raw_len = count * elem;
                if raw_len > self.budget {
                    return Err(invalid(format!("an array of {count} elements is larger than the file can hold")));
                }
                self.budget -= raw_len;
                let stored_bytes = self.take(at, stored, end)?;
                let raw_len = raw_len as usize;
                let decoded: Cow<'_, [u8]> = match encoding {
                    0 if stored_bytes.len() >= raw_len => Cow::Borrowed(&stored_bytes[..raw_len]),
                    0 => return Err(invalid("an array is shorter than its element count")),
                    1 => {
                        let bytes = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(stored_bytes, raw_len)
                            .map_err(|e| invalid(format!("a compressed array cannot be inflated ({:?})", e.status)))?;
                        if bytes.len() != raw_len {
                            return Err(invalid("a compressed array has the wrong length"));
                        }
                        Cow::Owned(bytes)
                    }
                    other => return Err(invalid(format!("unknown array encoding {other}"))),
                };
                let data = &decoded[..];
                match code {
                    b'f' => Prop::Floats(data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c) as f64).collect()),
                    b'd' => Prop::Floats(data.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect()),
                    b'l' => Prop::Ints(data.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c)).collect()),
                    b'i' => Prop::Ints(data.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c) as i64).collect()),
                    _ => Prop::Ints(data.iter().map(|&b| b as i64).collect()),
                }
            }
            other => return Err(invalid(format!("unknown property type 0x{other:02x} at byte {}", *at - 1))),
        })
    }
}

/// Parses a binary FBX into its top-level nodes; also returns the version.
fn parse_binary(data: &[u8]) -> Result<(u32, Vec<Node>), ImportError> {
    if data.len() < 27 || !data.starts_with(BINARY_MAGIC) {
        return Err(invalid("the binary header is damaged"));
    }
    let version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);
    if !(6000..8000).contains(&version) {
        return Err(invalid(format!("unsupported binary FBX version {version}")));
    }
    // Decompressed arrays: at most ~64x the file size (zlib of real geometry compresses 2..5x) plus 64 MiB.
    let budget = (data.len() as u64).saturating_mul(64).saturating_add(64 << 20).min(8 << 30);
    let mut reader = BinaryReader { data, wide: version >= 7500, budget };
    let mut pos = 27;
    let nodes = reader.list(&mut pos, data.len(), 0)?;
    Ok((version, nodes))
}

// ---- ASCII -----------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Number(String),
    Colon,
    Comma,
    LBrace,
    RBrace,
    Star,
    Newline,
    Eof,
}

struct Lexer<'a> {
    text: &'a [u8],
    pos: usize,
    peeked: Option<Tok>,
}

impl Lexer<'_> {
    fn lex(&mut self) -> Tok {
        loop {
            let Some(&c) = self.text.get(self.pos) else { return Tok::Eof };
            match c {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b';' => {
                    while self.text.get(self.pos).is_some_and(|&c| c != b'\n') {
                        self.pos += 1;
                    }
                }
                b'\n' => {
                    self.pos += 1;
                    return Tok::Newline;
                }
                b':' => {
                    self.pos += 1;
                    return Tok::Colon;
                }
                b',' => {
                    self.pos += 1;
                    return Tok::Comma;
                }
                b'{' => {
                    self.pos += 1;
                    return Tok::LBrace;
                }
                b'}' => {
                    self.pos += 1;
                    return Tok::RBrace;
                }
                b'*' => {
                    self.pos += 1;
                    return Tok::Star;
                }
                b'"' => {
                    let start = self.pos + 1;
                    let mut end = start;
                    while self.text.get(end).is_some_and(|&c| c != b'"') {
                        end += 1;
                    }
                    self.pos = (end + 1).min(self.text.len());
                    let raw = String::from_utf8_lossy(&self.text[start..end.min(self.text.len())]).into_owned();
                    return Tok::Str(raw.replace("&quot;", "\"").replace("&amp;", "&"));
                }
                _ => {
                    let start = self.pos;
                    while self.text.get(self.pos).is_some_and(|&c| !b" \t\r\n;:,{}*\"".contains(&c)) {
                        self.pos += 1;
                    }
                    let token = String::from_utf8_lossy(&self.text[start..self.pos]).into_owned();
                    return if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' {
                        Tok::Number(token)
                    } else {
                        Tok::Word(token)
                    };
                }
            }
        }
    }

    fn next(&mut self) -> Tok {
        self.peeked.take().unwrap_or_else(|| self.lex())
    }

    fn peek(&mut self) -> &Tok {
        if self.peeked.is_none() {
            self.peeked = Some(self.lex());
        }
        self.peeked.as_ref().expect("just set")
    }
}

fn number_prop(token: &str) -> Prop {
    let plain = !token.contains(['.', 'e', 'E']);
    if plain && let Ok(v) = token.parse::<i64>() {
        return Prop::Int(v);
    }
    // "1.#INF", "-1.#IND", "nan" and the like: not a number.
    Prop::Float(token.parse::<f64>().unwrap_or(f64::NAN))
}

fn ascii_list(lexer: &mut Lexer<'_>, depth: usize) -> Result<Vec<Node>, ImportError> {
    if depth > MAX_DEPTH {
        return Err(invalid("the nodes are nested too deeply"));
    }
    let mut nodes = Vec::new();
    loop {
        match lexer.peek().clone() {
            Tok::Eof | Tok::RBrace => return Ok(nodes),
            Tok::Word(name) => {
                lexer.next();
                if *lexer.peek() != Tok::Colon {
                    continue; // a stray word: skipped
                }
                lexer.next();
                nodes.push(ascii_node(lexer, name, depth)?);
            }
            _ => {
                lexer.next(); // stray punctuation or values between nodes: skipped
            }
        }
    }
}

fn ascii_node(lexer: &mut Lexer<'_>, name: String, depth: usize) -> Result<Node, ImportError> {
    let mut node = Node { name, props: Vec::new(), children: Vec::new() };
    loop {
        match lexer.next() {
            Tok::Str(s) => node.props.push(Prop::Str(s)),
            Tok::Number(n) => node.props.push(number_prop(&n)),
            Tok::Word(w) => node.props.push(Prop::Str(w)),
            Tok::Star => {
                // `*N` announces an array's length; the values follow in the `a:` child.
                if matches!(lexer.peek(), Tok::Number(_)) {
                    lexer.next();
                }
            }
            Tok::Comma => {
                // A value list continues on the next line after a trailing comma.
                while *lexer.peek() == Tok::Newline {
                    lexer.next();
                }
            }
            Tok::Colon => {}
            Tok::Newline => {
                // 6.x writes long arrays as continuation lines that start with a comma.
                while *lexer.peek() == Tok::Newline {
                    lexer.next();
                }
                if *lexer.peek() != Tok::Comma {
                    break;
                }
            }
            Tok::LBrace => {
                node.children = ascii_list(lexer, depth + 1)?;
                if lexer.next() != Tok::RBrace {
                    return Err(invalid(format!("the block of \"{}\" is not closed", node.name)));
                }
                break;
            }
            Tok::RBrace => {
                lexer.peeked = Some(Tok::RBrace);
                break;
            }
            Tok::Eof => break,
        }
    }
    // 7.x arrays: `Vertices: *6 { a: 0,0,0,1,1,1 }`.
    if node.props.is_empty() && node.children.len() == 1 && node.children[0].name == "a" {
        let a = node.children.pop().expect("one child");
        node.props = a.props;
    }
    Ok(node)
}

fn parse_ascii(data: &[u8]) -> Result<Vec<Node>, ImportError> {
    let text = decode_text(data);
    let mut lexer = Lexer { text: text.as_bytes(), pos: 0, peeked: None };
    let nodes = ascii_list(&mut lexer, 0)?;
    if nodes.is_empty() {
        return Err(invalid("the file holds no FBX data"));
    }
    Ok(nodes)
}

/// The top-level nodes of a binary or ASCII FBX.
fn parse_document(data: &[u8]) -> Result<Vec<Node>, ImportError> {
    if data.starts_with(b"Kaydara FBX Binary") {
        return parse_binary(data).map(|(_, nodes)| nodes);
    }
    // A binary header with damaged bytes must not be read as text.
    if data.iter().take(1024).any(|&b| b == 0) {
        return Err(invalid("the file is neither binary nor ASCII FBX"));
    }
    parse_ascii(data)
}

// ---------------------------------------------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------------------------------------------

/// A Properties70 / Properties60 block: property name -> its values (after the type / label / flag strings).
struct Properties<'a> {
    values: BTreeMap<&'a str, &'a [Prop]>,
}

impl<'a> Properties<'a> {
    fn of(node: &'a Node) -> Properties<'a> {
        let mut values = BTreeMap::new();
        for block in node.children.iter().filter(|c| c.name == "Properties70" || c.name == "Properties60") {
            for p in &block.children {
                // P: name, type, label, flags, values...  Property: name, type, flags, values...
                let skip = match p.name.as_str() {
                    "P" => 4,
                    "Property" => 3,
                    _ => continue,
                };
                if let Some(name) = p.prop_str(0) {
                    values.insert(name, p.props.get(skip..).unwrap_or(&[]));
                }
            }
        }
        Properties { values }
    }

    fn f64s(&self, name: &str) -> Option<Vec<f64>> {
        self.values.get(name).map(|v| v.iter().filter_map(Prop::as_f64).collect())
    }

    fn vec3(&self, name: &str, default: DVec3) -> DVec3 {
        match self.f64s(name).as_deref() {
            Some([x, y, z, ..]) if x.is_finite() && y.is_finite() && z.is_finite() => DVec3::new(*x, *y, *z),
            _ => default,
        }
    }

    fn number(&self, name: &str) -> Option<f64> {
        self.f64s(name).and_then(|v| v.first().copied()).filter(|v| v.is_finite())
    }

    fn int(&self, name: &str) -> Option<i64> {
        self.values.get(name).and_then(|v| v.first()).and_then(Prop::as_i64)
    }

    fn string(&self, name: &str) -> Option<&'a str> {
        self.values.get(name).and_then(|v| v.iter().find_map(Prop::as_str))
    }
}

/// Euler rotation in degrees for an FBX rotation order (0 = XYZ: X is applied first).
fn euler(degrees: DVec3, order: i64) -> DMat4 {
    let r = degrees * (std::f64::consts::PI / 180.0);
    let (x, y, z) = (DMat4::from_rotation_x(r.x), DMat4::from_rotation_y(r.y), DMat4::from_rotation_z(r.z));
    match order {
        1 => y * z * x, // XZY
        2 => x * z * y, // YZX
        3 => z * x * y, // YXZ
        4 => y * x * z, // ZXY
        5 => x * y * z, // ZYX
        _ => z * y * x, // XYZ (also SphericXYZ)
    }
}

/// The local transform of a model node (FBX SDK formula):
/// `T * Roff * Rp * Rpre * R * Rpost^-1 * Rp^-1 * Soff * Sp * S * Sp^-1`.
fn local_transform(props: &Properties<'_>) -> DMat4 {
    let t = props.vec3("Lcl Translation", DVec3::ZERO);
    let r = props.vec3("Lcl Rotation", DVec3::ZERO);
    let s = props.vec3("Lcl Scaling", DVec3::ONE);
    let pre = props.vec3("PreRotation", DVec3::ZERO);
    let post = props.vec3("PostRotation", DVec3::ZERO);
    let roff = props.vec3("RotationOffset", DVec3::ZERO);
    let rp = props.vec3("RotationPivot", DVec3::ZERO);
    let soff = props.vec3("ScalingOffset", DVec3::ZERO);
    let sp = props.vec3("ScalingPivot", DVec3::ZERO);
    let order = props.int("RotationOrder").unwrap_or(0);
    let tr = DMat4::from_translation;
    tr(t) * tr(roff)
        * tr(rp)
        * euler(pre, 0)
        * euler(r, order)
        * euler(post, 0).inverse()
        * tr(-rp)
        * tr(soff)
        * tr(sp)
        * DMat4::from_scale(s)
        * tr(-sp)
}

/// The geometric transform of a model (applies to its own geometry only, not inherited).
fn geometric_transform(props: &Properties<'_>) -> DMat4 {
    DMat4::from_translation(props.vec3("GeometricTranslation", DVec3::ZERO))
        * euler(props.vec3("GeometricRotation", DVec3::ZERO), 0)
        * DMat4::from_scale(props.vec3("GeometricScaling", DVec3::ONE))
}

/// File axes and units -> the app's Y-up, metre space: rows = the file's right, up and front directions.
fn axis_conversion(settings: Option<&Properties<'_>>) -> DMat4 {
    let axis = |index: Option<i64>, sign: Option<i64>, default: usize| -> (usize, f64) {
        let i = index.filter(|i| (0..3).contains(i)).map_or(default, |i| i as usize);
        (i, if sign.unwrap_or(1) < 0 { -1.0 } else { 1.0 })
    };
    let (up, up_sign, right, right_sign, unit) = match settings {
        Some(p) => {
            let (up, up_sign) = axis(p.int("UpAxis"), p.int("UpAxisSign"), 1);
            let (right, right_sign) = axis(p.int("CoordAxis"), p.int("CoordAxisSign"), 0);
            let unit = p.number("UnitScaleFactor").filter(|u| *u > 0.0).unwrap_or(1.0);
            (up, up_sign, right, right_sign, unit)
        }
        None => (1, 1.0, 0, 1.0, 1.0),
    };
    let (up, up_sign, right, right_sign) = if up == right { (1, 1.0, 0, 1.0) } else { (up, up_sign, right, right_sign) };
    let mut u = DVec3::ZERO;
    u[up] = up_sign;
    let mut r = DVec3::ZERO;
    r[right] = right_sign;
    // Front completes a right-handed frame, so the conversion is a rotation (no mirroring).
    let f = r.cross(u);
    let rotation = DMat3::from_cols(r, u, f).transpose();
    // UnitScaleFactor is centimetres per file unit.
    DMat4::from_mat3(rotation * (unit * 0.01))
}

/// How a layer element maps its values.
#[derive(Copy, Clone, PartialEq, Debug)]
enum Mapping {
    PolygonVertex,
    Vertex,
    Polygon,
    AllSame,
    Unsupported,
}

/// One layer element (normals, UVs) with its values and optional index list.
struct LayerElement<'a> {
    mapping: Mapping,
    values: Cow<'a, [f64]>,
    index: Option<Cow<'a, [i64]>>,
    width: usize,
}

impl LayerElement<'_> {
    fn read<'n>(node: &'n Node, values: &str, index: &[&str], width: usize) -> Option<LayerElement<'n>> {
        let mapping = match node.child_str("MappingInformationType").unwrap_or("ByPolygonVertex") {
            "ByPolygonVertex" => Mapping::PolygonVertex,
            "ByVertice" | "ByVertex" | "ByControlPoint" => Mapping::Vertex,
            "ByPolygon" => Mapping::Polygon,
            "AllSame" => Mapping::AllSame,
            _ => Mapping::Unsupported,
        };
        let indexed = matches!(node.child_str("ReferenceInformationType"), Some("IndexToDirect" | "Index"));
        let values = node.child(values)?.floats();
        let index = if indexed { index.iter().find_map(|name| node.child(name)).map(Node::ints) } else { None };
        Some(LayerElement { mapping, values, index, width })
    }

    /// The value index of a corner; `None` when the element does not cover it.
    fn slot(&self, polygon: usize, corner: usize, vertex: usize) -> Option<usize> {
        let i = match self.mapping {
            Mapping::PolygonVertex => corner,
            Mapping::Vertex => vertex,
            Mapping::Polygon => polygon,
            Mapping::AllSame => 0,
            Mapping::Unsupported => return None,
        };
        let d = match &self.index {
            Some(index) => usize::try_from(*index.get(i)?).ok()?,
            None => i,
        };
        (d.checked_add(1)?.checked_mul(self.width)? <= self.values.len()).then_some(d)
    }

    fn vec3(&self, polygon: usize, corner: usize, vertex: usize) -> Option<DVec3> {
        let d = self.slot(polygon, corner, vertex)? * 3;
        Some(DVec3::new(self.values[d], self.values[d + 1], self.values[d + 2]))
    }

    fn vec2(&self, polygon: usize, corner: usize, vertex: usize) -> Option<DVec2> {
        let d = self.slot(polygon, corner, vertex)? * 2;
        Some(DVec2::new(self.values[d], self.values[d + 1]))
    }
}

/// Triangulates a planar-ish polygon: ear clipping in the plane of its Newell normal, fan when that fails.
fn triangulate(points: &[DVec3]) -> Vec<[usize; 3]> {
    let n = points.len();
    let fan = || (1..n - 1).map(|i| [0, i, i + 1]).collect::<Vec<_>>();
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    if n > MAX_EAR_CLIP {
        return fan();
    }
    let mut normal = DVec3::ZERO;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        normal += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    let Some(normal) = normal.try_normalize() else { return fan() };
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let p: Vec<DVec2> = points.iter().map(|q| DVec2::new(q.dot(u), q.dot(v))).collect();
    let cross = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let mut ring: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    while ring.len() > 3 {
        let m = ring.len();
        // Starting at corner 1 makes a convex polygon come out exactly like the fan from corner 0.
        let ear = (1..=m).map(|i| i % m).find(|&i| {
            let (a, b, c) = (ring[(i + m - 1) % m], ring[i], ring[(i + 1) % m]);
            if cross(p[a], p[b], p[c]) <= 0.0 {
                return false; // reflex or degenerate corner
            }
            ring.iter().all(|&o| {
                if o == a || o == b || o == c {
                    return true;
                }
                let q = p[o];
                !(cross(p[a], p[b], q) >= 0.0 && cross(p[b], p[c], q) >= 0.0 && cross(p[c], p[a], q) >= 0.0)
            })
        });
        let Some(i) = ear else { return fan() };
        out.push([ring[(i + m - 1) % m], ring[i], ring[(i + 1) % m]]);
        ring.remove(i);
    }
    out.push([ring[0], ring[1], ring[2]]);
    out
}

/// The UV transform of a Texture object (FBX SDK `FbxTexture`: `Translation`, `Rotation` (W = the Z component, in
/// degrees, counter-clockwise), `Scaling`, `TextureRotationPivot`, `TextureScalingPivot`), in the file's UV space
/// (V up), composed like a node transform: `T * Rp * R * Rp^-1 * Sp * S * Sp^-1`. Without pivots this is
/// `uv' = T + R * (S * uv)`, the `KHR_texture_transform` form, and it is what 3ds Max (tiling / offset), Maya
/// (repeat UV / offset) and the FBX SDK write and what Assimp reads. Older files (and texture objects without a
/// video) carry `ModelUVTranslation` / `ModelUVScaling` child nodes instead; the properties win where both exist
/// (as in Assimp). A scale component of 0 counts as 1 (as in Blender), non-finite values are ignored.
#[derive(Copy, Clone, PartialEq, Debug)]
struct UvTransform {
    translation: DVec2,
    /// Radians, counter-clockwise in the V-up space of the file.
    rotation: f64,
    scaling: DVec2,
    rotation_pivot: DVec2,
    scaling_pivot: DVec2,
}

impl UvTransform {
    /// The transform of a texture object; `None` when it is the identity.
    fn of(texture: &Node) -> Option<UvTransform> {
        let props = Properties::of(texture);
        let pair = |names: &[&str], child: &str, default: DVec2| -> DVec2 {
            let finite = |v: &[f64]| match v {
                [x, y, ..] if x.is_finite() && y.is_finite() => Some(DVec2::new(*x, *y)),
                _ => None,
            };
            names
                .iter()
                .find_map(|name| props.f64s(name).and_then(|v| finite(&v)))
                .or_else(|| texture.child(child).and_then(|c| finite(&c.floats())))
                .unwrap_or(default)
        };
        let translation = pair(&["Translation", "UVTranslation"], "ModelUVTranslation", DVec2::ZERO);
        let scaling = pair(&["Scaling", "UVScaling"], "ModelUVScaling", DVec2::ONE);
        let scaling = DVec2::new(
            if scaling.x == 0.0 { 1.0 } else { scaling.x },
            if scaling.y == 0.0 { 1.0 } else { scaling.y },
        );
        let rotation = match props.f64s("Rotation").as_deref() {
            Some([_, _, w, ..]) if w.is_finite() => w.to_radians(),
            _ => 0.0,
        };
        let rotation_pivot = pair(&["TextureRotationPivot"], "", DVec2::ZERO);
        let scaling_pivot = pair(&["TextureScalingPivot"], "", DVec2::ZERO);
        let identity = translation == DVec2::ZERO && rotation == 0.0 && scaling == DVec2::ONE;
        (!identity).then_some(UvTransform { translation, rotation, scaling, rotation_pivot, scaling_pivot })
    }

    /// Transforms a UV of the file (V up).
    fn apply(&self, uv: DVec2) -> DVec2 {
        let p = (uv - self.scaling_pivot) * self.scaling + self.scaling_pivot - self.rotation_pivot;
        let p = if self.rotation == 0.0 {
            p
        } else {
            let (sin, cos) = self.rotation.sin_cos();
            DVec2::new(cos * p.x - sin * p.y, sin * p.x + cos * p.y)
        };
        p + self.rotation_pivot + self.translation
    }
}

/// What a texture object points at.
struct TextureFile {
    /// RelativeFilename, then FileName (texture first, then its video), without empty entries or duplicates.
    candidates: Vec<String>,
    /// Embedded image bytes of the video (`Content`).
    content: Option<Arc<[u8]>>,
}

/// Image bytes of a `Content` node: raw bytes (binary) or base64 strings (ASCII).
fn content_bytes(node: &Node) -> Option<Arc<[u8]>> {
    let content = node.child("Content")?;
    let bytes = match content.props.as_slice() {
        [Prop::Raw(bytes)] => bytes.clone(),
        props => {
            let text: String = props.iter().filter_map(Prop::as_str).flat_map(|s| s.chars()).filter(|c| !c.is_whitespace()).collect();
            if text.is_empty() {
                return None;
            }
            base64_decode(&text)?
        }
    };
    (!bytes.is_empty()).then(|| Arc::from(bytes))
}

/// The scene graph of an FBX document.
struct Scene<'a> {
    objects: HashMap<String, &'a Node>,
    /// Object order of the file (deterministic iteration).
    order: Vec<String>,
    /// (child, parent, property) in file order.
    connections: Vec<(String, String, Option<String>)>,
    settings: Option<Properties<'a>>,
}

fn object_id(node: &Node) -> Option<String> {
    match node.props.first()? {
        Prop::Int(id) => Some(id.to_string()),
        Prop::Str(name) => Some(name.clone()),
        _ => None,
    }
}

/// The display name of an object: the name property (after a 7.x id), without its class.
fn display_name(node: &Node) -> String {
    let raw = match node.props.first() {
        Some(Prop::Int(_)) => node.prop_str(1),
        _ => node.prop_str(0),
    };
    object_name(raw.unwrap_or("")).to_owned()
}

/// The subclass of an object ("Mesh", "LimbNode", "Skin", ...).
fn subclass(node: &Node) -> &str {
    match node.props.first() {
        Some(Prop::Int(_)) => node.prop_str(2),
        _ => node.prop_str(1),
    }
    .unwrap_or("")
}

impl<'a> Scene<'a> {
    fn new(roots: &'a [Node]) -> Scene<'a> {
        let mut objects = HashMap::new();
        let mut order = Vec::new();
        for objects_node in roots.iter().filter(|n| n.name == "Objects") {
            for object in &objects_node.children {
                if let Some(id) = object_id(object)
                    && !objects.contains_key(&id)
                {
                    order.push(id.clone());
                    objects.insert(id, object);
                }
            }
        }
        let mut connections = Vec::new();
        for block in roots.iter().filter(|n| n.name == "Connections") {
            for c in block.children.iter().filter(|c| c.name == "C" || c.name == "Connect") {
                let id = |i: usize| -> Option<String> {
                    match c.props.get(i)? {
                        Prop::Int(v) => Some(v.to_string()),
                        Prop::Str(s) => Some(s.clone()),
                        _ => None,
                    }
                };
                if let (Some(child), Some(parent)) = (id(1), id(2)) {
                    connections.push((child, parent, c.prop_str(3).map(str::to_owned)));
                }
            }
        }
        let settings = roots
            .iter()
            .find(|n| n.name == "GlobalSettings")
            .or_else(|| roots.iter().find(|n| n.name == "Settings").and_then(|s| s.child("GlobalSettings")))
            .map(Properties::of);
        Scene { objects, order, connections, settings }
    }

    fn class(&self, id: &str) -> Option<&str> {
        self.objects.get(id).map(|n| n.name.as_str())
    }

    /// Objects of `class` connected below `parent`, in connection order, with the connection property.
    fn children_of(&self, parent: &str, class: &str) -> Vec<(&'a Node, Option<&str>)> {
        self.connections
            .iter()
            .filter(|(c, p, _)| p == parent && self.class(c) == Some(class))
            .filter_map(|(c, _, prop)| self.objects.get(c).map(|n| (*n, prop.as_deref())))
            .collect()
    }

    /// The model a model is attached to (`None` for the scene root and for a missing parent).
    fn parent_model(&self, id: &str) -> Option<&str> {
        self.connections
            .iter()
            .find(|(c, p, _)| c == id && self.class(p) == Some("Model"))
            .map(|(_, p, _)| p.as_str())
    }

    /// World matrix of a model (cycles are cut after `MAX` levels).
    fn world(&self, id: &str, cache: &mut HashMap<String, DMat4>) -> DMat4 {
        if let Some(m) = cache.get(id) {
            return *m;
        }
        let mut chain: Vec<&str> = vec![id];
        let mut current = id;
        while let Some(parent) = self.parent_model(current) {
            if chain.len() > 1024 || chain.contains(&parent) || cache.contains_key(parent) {
                if cache.contains_key(parent) {
                    chain.push(parent);
                }
                break;
            }
            chain.push(parent);
            current = parent;
        }
        let mut acc = DMat4::IDENTITY;
        for &m in chain.iter().rev() {
            if let Some(known) = cache.get(m) {
                acc = *known;
                continue;
            }
            if let Some(node) = self.objects.get(m) {
                acc *= local_transform(&Properties::of(node));
            }
            cache.insert(m.to_owned(), acc);
        }
        acc
    }

    fn texture_file(&self, texture: &'a Node) -> TextureFile {
        let tid = object_id(texture).unwrap_or_default();
        let video = self.children_of(&tid, "Video").into_iter().next().map(|(v, _)| v);
        let mut candidates: Vec<String> = Vec::new();
        for node in std::iter::once(texture).chain(video) {
            let props = Properties::of(node);
            let names = [
                node.child_str("RelativeFilename").map(str::to_owned),
                props.string("RelPath").map(str::to_owned),
                node.child_str("FileName").map(str::to_owned),
                props.string("Path").map(str::to_owned),
            ];
            for name in names.into_iter().flatten() {
                let name = name.trim().to_owned();
                if !name.is_empty() && !candidates.iter().any(|c| normalize_ref(c) == normalize_ref(&name)) {
                    candidates.push(name);
                }
            }
        }
        let content = video.and_then(content_bytes).or_else(|| content_bytes(texture));
        TextureFile { candidates, content }
    }

    /// The texture of a material's base colour: the DiffuseColor / base-colour slot, else the first other texture
    /// that is not a normal / bump / specular-type map. A layered texture stands for its first layer.
    fn diffuse_texture(&self, material: &Node) -> Option<&'a Node> {
        let mid = object_id(material)?;
        let mut candidates: Vec<(&'a Node, Option<&str>)> = self.children_of(&mid, "Texture");
        for (layered, prop) in self.children_of(&mid, "LayeredTexture") {
            if let Some(id) = object_id(layered)
                && let Some((first, _)) = self.children_of(&id, "Texture").into_iter().next()
            {
                candidates.push((first, prop));
            }
        }
        let lower = |p: Option<&str>| p.unwrap_or("").to_ascii_lowercase();
        let rank = |p: Option<&str>| -> u8 {
            let p = lower(p);
            if p == "diffusecolor" || p.ends_with("base_color_map") || p.ends_with("basecolor") || p.ends_with("base_color") {
                0
            } else if p.contains("diffuse") || p.contains("albedo") || p.contains("base") {
                1
            } else if p.is_empty() {
                2
            } else if ["normal", "bump", "spec", "rough", "metal", "gloss", "emissive", "opacity", "transparen", "reflect", "displac", "height", "ao", "occlusion"]
                .iter()
                .any(|k| p.contains(k))
            {
                9
            } else {
                3
            }
        };
        candidates.iter().filter(|(_, p)| rank(*p) < 9).min_by_key(|(_, p)| rank(*p)).map(|(t, _)| *t)
    }
}

/// Collects the triangles, materials and textures while the models are walked.
struct Builder<'s, 'a> {
    scene: &'s Scene<'a>,
    resolver: &'s dyn AssetResolver,
    triangles: Vec<RawTriangle>,
    materials: Vec<RawMaterial>,
    /// The UV transform of each material's texture (parallel to `materials`).
    uv_transforms: Vec<Option<UvTransform>>,
    /// Material object id -> index into `materials`.
    material_index: HashMap<String, u32>,
    /// Index of the material of geometry without one.
    default_material: Option<u32>,
    requests: Vec<TextureRequest>,
    /// Texture identity (resolved reference or embedded video) -> index into `requests`.
    texture_index: HashMap<String, u32>,
    /// Embedded images of every video by normalized file name (an orphaned video can supply a texture's pixels).
    embedded_by_name: HashMap<String, Arc<[u8]>>,
    warnings: Vec<ImportWarning>,
}

impl Builder<'_, '_> {
    fn default_material(&mut self) -> u32 {
        if let Some(m) = self.default_material {
            return m;
        }
        let m = self.materials.len() as u32;
        self.materials.push(RawMaterial { name: String::new(), base_color: DEFAULT_COLOR, double_sided: false, texture: None });
        self.uv_transforms.push(None);
        self.default_material = Some(m);
        m
    }

    fn material(&mut self, node: &Node) -> u32 {
        let id = object_id(node).unwrap_or_default();
        if let Some(&m) = self.material_index.get(&id) {
            return m;
        }
        let props = Properties::of(node);
        let color = props
            .f64s("DiffuseColor")
            .or_else(|| props.f64s("Diffuse"))
            .or_else(|| props.f64s("3dsMax|Parameters|base_color"))
            .filter(|c| c.len() >= 3);
        let opacity = props.number("Opacity").unwrap_or(1.0) as f32;
        let base_color = match color {
            Some(c) => [
                clean_unit(c[0] as f32, DEFAULT_COLOR[0]),
                clean_unit(c[1] as f32, DEFAULT_COLOR[1]),
                clean_unit(c[2] as f32, DEFAULT_COLOR[2]),
                clean_unit(opacity, 1.0),
            ],
            None => [DEFAULT_COLOR[0], DEFAULT_COLOR[1], DEFAULT_COLOR[2], clean_unit(opacity, 1.0)],
        };
        let name = display_name(node);
        let diffuse = self.scene.diffuse_texture(node);
        let texture = diffuse.map(|t| self.texture(t, &name));
        let m = self.materials.len() as u32;
        self.materials.push(RawMaterial { name, base_color, double_sided: false, texture });
        self.uv_transforms.push(diffuse.and_then(UvTransform::of));
        self.material_index.insert(id, m);
        m
    }

    /// The texture index of a texture object (one per distinct file or embedded image).
    fn texture(&mut self, texture: &Node, material: &str) -> u32 {
        let file = self.scene.texture_file(texture);
        let display = file.candidates.first().cloned().unwrap_or_else(|| display_name(texture));
        let hint = file.candidates.first().map(|c| file_name_of(c).to_owned());
        let (key, request) = if let Some(bytes) = file.content {
            let key = format!("embedded:{}", object_id(texture).unwrap_or_default());
            let request = TextureRequest {
                name: file_stem_of(&display).to_owned(),
                reference: display.clone(),
                material: material.to_owned(),
                name_hint: hint,
                source: TextureSource::Bytes(bytes),
            };
            (key, request)
        } else if let Some(found) = file.candidates.iter().find(|c| self.resolver.read(c).is_ok()) {
            let request = TextureRequest {
                name: file_stem_of(found).to_owned(),
                reference: found.clone(),
                material: material.to_owned(),
                name_hint: Some(found.clone()),
                source: TextureSource::Resolver,
            };
            (normalize_ref(found), request)
        } else if let Some(bytes) =
            file.candidates.iter().find_map(|c| self.embedded_by_name.get(file_name_of(&normalize_ref(c))).cloned())
        {
            // The file is missing but another (orphaned) video of the same name embeds it.
            let request = TextureRequest {
                name: file_stem_of(&display).to_owned(),
                reference: display.clone(),
                material: material.to_owned(),
                name_hint: hint,
                source: TextureSource::Bytes(bytes),
            };
            (format!("embedded-name:{}", normalize_ref(&display)), request)
        } else {
            let source = if file.candidates.is_empty() { TextureSource::Missing } else { TextureSource::Resolver };
            let request = TextureRequest {
                name: file_stem_of(&display).to_owned(),
                reference: display.clone(),
                material: material.to_owned(),
                name_hint: hint,
                source,
            };
            (normalize_ref(&display), request)
        };
        if let Some(&i) = self.texture_index.get(&key) {
            return i;
        }
        let i = self.requests.len() as u32;
        self.requests.push(request);
        self.texture_index.insert(key, i);
        i
    }

    /// Bakes one geometry into world space with the model's materials.
    fn add_geometry(&mut self, geometry: &Node, model_name: &str, transform: DMat4, materials: &[u32]) {
        let skip = |b: &mut Self, reason: String| {
            b.warnings.push(ImportWarning::SkippedPrimitive { reason: format!("model \"{model_name}\": {reason}") });
        };
        let (Some(vertices), Some(polygons)) = (geometry.child("Vertices"), geometry.child("PolygonVertexIndex")) else {
            skip(self, "the geometry has no polygons".to_owned());
            return;
        };
        let vertices = vertices.floats();
        let polygons = polygons.ints();
        let vertex_count = vertices.len() / 3;
        let normal_matrix = DMat3::from_mat4(transform).inverse().transpose();
        let normal_matrix = normal_matrix.is_finite().then_some(normal_matrix);
        let mirrored = transform.determinant() < 0.0;

        let first_layer = |name: &str| -> Option<&Node> {
            let layers: Vec<&Node> = geometry.children.iter().filter(|c| c.name == name).collect();
            layers.iter().find(|l| l.props.first().and_then(Prop::as_i64) == Some(0)).or(layers.first()).copied()
        };
        let normals = first_layer("LayerElementNormal").and_then(|n| LayerElement::read(n, "Normals", &["NormalsIndex", "NormalIndex"], 3));
        let uvs = first_layer("LayerElementUV").and_then(|n| LayerElement::read(n, "UV", &["UVIndex"], 2));
        let material_layer = first_layer("LayerElementMaterial");
        let material_mapping = material_layer.and_then(|n| n.child_str("MappingInformationType")).unwrap_or("AllSame");
        let material_list = material_layer.and_then(|n| n.child("Materials")).map(Node::ints);
        let material_of = |polygon: usize| -> Option<u32> {
            let list = material_list.as_ref()?;
            let slot = if material_mapping == "ByPolygon" { *list.get(polygon)? } else { *list.first()? };
            materials.get(usize::try_from(slot).ok()?).copied()
        };
        let fallback_material = materials.first().copied();
        let default_material = if fallback_material.is_none() { Some(self.default_material()) } else { None };

        let mut dangling = 0u64;
        let mut corners: Vec<(usize, usize)> = Vec::new();
        let mut polygon = 0usize;
        let mut emit = |corners: &[(usize, usize)], polygon: usize, b: &mut Self| {
            if corners.len() < 3 {
                return;
            }
            if corners.iter().any(|&(v, _)| v >= vertex_count) {
                dangling += 1;
                return;
            }
            let local: Vec<DVec3> =
                corners.iter().map(|&(v, _)| DVec3::new(vertices[3 * v], vertices[3 * v + 1], vertices[3 * v + 2])).collect();
            let world: Vec<DVec3> = local.iter().map(|p| transform.transform_point3(*p)).collect();
            let material = material_of(polygon).or(fallback_material).or(default_material).unwrap_or(0);
            let uv_transform = b.uv_transforms.get(material as usize).copied().flatten();
            for tri in triangulate(&local) {
                let tri = if mirrored { [tri[0], tri[2], tri[1]] } else { tri };
                let positions = tri.map(|k| world[k].as_vec3());
                let normal = |k: usize| -> Option<Vec3> {
                    let (v, c) = corners[k];
                    let n = normals.as_ref()?.vec3(polygon, c, v)?;
                    // A zero result is regenerated by the normalization.
                    Some((normal_matrix? * n).normalize_or_zero().as_vec3())
                };
                let normals = match (normal(tri[0]), normal(tri[1]), normal(tri[2])) {
                    (Some(a), Some(b), Some(c)) => Some([a, b, c]),
                    _ => None,
                };
                let uv = |k: usize| -> Option<Vec2> {
                    let (v, c) = corners[k];
                    let t = uvs.as_ref()?.vec2(polygon, c, v)?;
                    // The texture's UV transform works in the file's V-up space, before the flip.
                    let t = uv_transform.map_or(t, |x| x.apply(t));
                    // FBX has its UV origin bottom-left: flip V once, here.
                    Some(Vec2::new(t.x as f32, (1.0 - t.y) as f32))
                };
                let uvs = match (uv(tri[0]), uv(tri[1]), uv(tri[2])) {
                    (Some(a), Some(b), Some(c)) => Some([a, b, c]),
                    _ => None,
                };
                b.triangles.push(RawTriangle { positions, normals, uvs, material });
            }
        };
        for (corner, &raw) in polygons.iter().enumerate() {
            let (vertex, last) = if raw < 0 { (!raw, true) } else { (raw, false) };
            corners.push((usize::try_from(vertex).unwrap_or(usize::MAX), corner));
            if last {
                emit(&corners, polygon, self);
                corners.clear();
                polygon += 1;
            }
        }
        // A last polygon without its end marker is used as well.
        emit(&corners, polygon, self);
        if dangling > 0 {
            skip(self, format!("{dangling} polygon(s) refer to vertices that do not exist"));
        }
    }
}

/// FBX (binary or ASCII) -> `RawMesh` (geometry baked into world space, Y-up, metres; textures through `resolver`,
/// decoded only when `decode_textures`).
///
/// Every `Model` with mesh geometry is instanced (7.x: Geometry objects connected to the model; 6.x: the geometry
/// inside the model node). Materials: in order of first use; the base colour is `DiffuseColor` (else `Diffuse`) with
/// `Opacity` as alpha; the texture of the `DiffuseColor` / base-colour slot names the texture set and its UV transform
/// (`UvTransform`) is applied to the UVs of the material's polygons (materials showing the same image with different
/// transforms share one texture, as in the glTF loader). Texture files are
/// looked up by RelativeFilename, then FileName (through the resolver's fallbacks); a video's embedded `Content`
/// wins over files. With `decode_textures` every other texture file the file names is read too (and so captured by a
/// `RecordingResolver`). Skin and blend-shape deformers are ignored with `SkinIgnored`.
pub(crate) fn load_fbx(
    main: &[u8],
    resolver: &dyn AssetResolver,
    opts: &ImportOptions,
    decode_textures: bool,
) -> Result<RawImport, ImportError> {
    let roots = parse_document(main)?;
    let scene = Scene::new(&roots);
    let mut warnings = Vec::new();
    if scene.objects.is_empty() && !roots.iter().any(|n| n.name == "Objects") {
        return Err(invalid("the file has no Objects section"));
    }

    let mut embedded_by_name = HashMap::new();
    for id in &scene.order {
        let node = scene.objects[id];
        if node.name == "Video"
            && let Some(bytes) = content_bytes(node)
        {
            for name in [node.child_str("RelativeFilename"), node.child_str("FileName"), node.child_str("Filename")].into_iter().flatten() {
                embedded_by_name.entry(file_name_of(&normalize_ref(name)).to_owned()).or_insert_with(|| bytes.clone());
            }
        }
    }
    let mut builder = Builder {
        scene: &scene,
        resolver,
        triangles: Vec::new(),
        materials: Vec::new(),
        uv_transforms: Vec::new(),
        material_index: HashMap::new(),
        default_material: None,
        requests: Vec::new(),
        texture_index: HashMap::new(),
        embedded_by_name,
        warnings: Vec::new(),
    };

    let conversion = axis_conversion(scene.settings.as_ref());
    let mut world_cache: HashMap<String, DMat4> = HashMap::new();
    // One object per `Model` with geometry (5.21), named by its display name.
    let mut objects = RawObjects::default();
    let mut skinned = false;
    for id in &scene.order {
        let model = scene.objects[id];
        if model.name != "Model" {
            if model.name == "Deformer" && matches!(subclass(model), "Skin" | "BlendShape") {
                skinned = true;
            }
            continue;
        }
        let model_name = display_name(model);
        let props = Properties::of(model);
        let transform = conversion * scene.world(id, &mut world_cache) * geometric_transform(&props);
        let mut materials: Vec<u32> = Vec::new();
        for (material, _) in scene.children_of(id, "Material") {
            materials.push(builder.material(material));
        }
        if materials.is_empty() {
            // 6.x: a texture connected straight to the model.
            if let Some((texture, _)) = scene.children_of(id, "Texture").into_iter().next() {
                let t = builder.texture(texture, &model_name);
                let m = builder.default_material();
                if builder.materials[m as usize].texture.is_none() {
                    builder.materials[m as usize].texture = Some(t);
                    builder.uv_transforms[m as usize] = UvTransform::of(texture);
                }
                materials.push(m);
            }
        }
        // 7.x geometry objects, or the 6.x geometry inside the model.
        let mut geometries: Vec<&Node> = Vec::new();
        for (geometry, _) in scene.children_of(id, "Geometry") {
            match subclass(geometry) {
                "Mesh" | "" => geometries.push(geometry),
                "Shape" => skinned = true,
                other => builder.warnings.push(ImportWarning::SkippedPrimitive {
                    reason: format!("model \"{model_name}\": {other} geometry is not a polygon mesh"),
                }),
            }
        }
        if geometries.is_empty() && model.child("Vertices").is_some() {
            geometries.push(model);
        }
        if geometries.is_empty() {
            continue;
        }
        for geometry in geometries {
            let gid = object_id(geometry).unwrap_or_default();
            if scene.connections.iter().any(|(c, p, _)| *p == gid && scene.class(c) == Some("Deformer")) {
                skinned = true;
            }
            builder.add_geometry(geometry, &model_name, transform, &materials);
        }
        let object = objects.names.len() as u32;
        objects.names.push(model_name);
        objects.tri_object.resize(builder.triangles.len(), object);
    }
    if skinned {
        builder.warnings.push(ImportWarning::SkinIgnored);
    }

    let Builder { triangles, materials, requests, warnings: geometry_warnings, .. } = builder;
    warnings.extend(geometry_warnings);
    let textures = load_textures(requests, resolver, opts, decode_textures, &mut warnings);
    if decode_textures {
        // Every texture file the document names is part of the model (normal, specular maps, unused textures):
        // read them so that a recording resolver captures them in the project. Failures do not matter here.
        for id in &scene.order {
            let node = scene.objects[id];
            if node.name == "Texture" {
                let file = scene.texture_file(node);
                if file.content.is_none() {
                    let _ = file.candidates.iter().find(|c| resolver.read(c).is_ok());
                }
            }
        }
    }
    Ok(RawImport { mesh: RawMesh { triangles, materials }, objects, textures, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{
        ImportedModel, MemoryResolver, ModelFormat, SourceBundle, import_from_bundle, import_from_path_auto_uvs,
    };
    use crate::progress::NoProgress;

    // ---- a minimal binary FBX writer (test fixtures) ----

    #[derive(Clone)]
    enum W {
        I(i32),
        L(i64),
        D(f64),
        S(String),
        R(Vec<u8>),
        Ad(Vec<f64>, bool),
        Ai(Vec<i32>, bool),
    }

    struct N(&'static str, Vec<W>, Vec<N>);

    fn s(text: &str) -> W {
        W::S(text.to_owned())
    }

    fn prop_bytes(p: &W) -> Vec<u8> {
        let mut out = Vec::new();
        let array = |out: &mut Vec<u8>, code: u8, count: usize, raw: Vec<u8>, compress: bool| {
            out.push(code);
            out.extend((count as u32).to_le_bytes());
            let data = if compress { miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6) } else { raw };
            out.extend((compress as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        };
        match p {
            W::I(v) => {
                out.push(b'I');
                out.extend(v.to_le_bytes());
            }
            W::L(v) => {
                out.push(b'L');
                out.extend(v.to_le_bytes());
            }
            W::D(v) => {
                out.push(b'D');
                out.extend(v.to_le_bytes());
            }
            W::S(v) => {
                out.push(b'S');
                out.extend((v.len() as u32).to_le_bytes());
                out.extend(v.as_bytes());
            }
            W::R(v) => {
                out.push(b'R');
                out.extend((v.len() as u32).to_le_bytes());
                out.extend(v);
            }
            W::Ad(v, c) => array(&mut out, b'd', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect(), *c),
            W::Ai(v, c) => array(&mut out, b'i', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect(), *c),
        }
        out
    }

    fn encode(node: &N, start: usize, wide: bool) -> Vec<u8> {
        let header = if wide { 25 } else { 13 };
        let props: Vec<u8> = node.1.iter().flat_map(prop_bytes).collect();
        let mut body = node.0.as_bytes().to_vec();
        body.extend(&props);
        for child in &node.2 {
            let encoded = encode(child, start + header + body.len(), wide);
            body.extend(encoded);
        }
        if !node.2.is_empty() {
            body.extend(vec![0u8; header]);
        }
        let end = start + header + body.len();
        let mut out = Vec::new();
        if wide {
            out.extend((end as u64).to_le_bytes());
            out.extend((node.1.len() as u64).to_le_bytes());
            out.extend((props.len() as u64).to_le_bytes());
        } else {
            out.extend((end as u32).to_le_bytes());
            out.extend((node.1.len() as u32).to_le_bytes());
            out.extend((props.len() as u32).to_le_bytes());
        }
        out.push(node.0.len() as u8);
        out.extend(body);
        out
    }

    fn binary(version: u32, roots: &[N]) -> Vec<u8> {
        let wide = version >= 7500;
        let mut data = BINARY_MAGIC.to_vec();
        data.extend([0x1a, 0]);
        data.extend(version.to_le_bytes());
        for root in roots {
            let encoded = encode(root, data.len(), wide);
            data.extend(encoded);
        }
        data.extend(vec![0u8; if wide { 25 } else { 13 }]);
        data
    }

    fn p70(entries: Vec<N>) -> N {
        N("Properties70", vec![], entries)
    }

    fn p(name: &str, values: &[f64]) -> N {
        let mut props = vec![s(name), s("Vector3D"), s("Vector"), s("")];
        props.extend(values.iter().map(|&v| W::D(v)));
        N("P", props, vec![])
    }

    fn p_int(name: &str, value: i32) -> N {
        N("P", vec![s(name), s("int"), s("Integer"), s(""), W::I(value)], vec![])
    }

    fn conn(kind: &str, child: i64, parent: i64, prop: Option<&str>) -> N {
        let mut props = vec![s(kind), W::L(child), W::L(parent)];
        if let Some(prop) = prop {
            props.push(s(prop));
        }
        N("C", props, vec![])
    }

    /// A unit quad (one polygon) with normals by polygon vertex, indexed UVs and one material, in a model with a
    /// translation; texture "tex/quad.png" on the material's DiffuseColor, optionally embedded in a video.
    fn quad_document_with(version: u32, compress: bool, settings: Vec<N>, model_props: Vec<N>, embedded: Option<Vec<u8>>) -> Vec<u8> {
        let geometry = N(
            "Geometry",
            vec![W::L(10), s("Quad\0\u{1}Geometry"), s("Mesh")],
            vec![
                N("Vertices", vec![W::Ad(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0], compress)], vec![]),
                N("PolygonVertexIndex", vec![W::Ai(vec![0, 1, 2, -4], compress)], vec![]),
                N(
                    "LayerElementNormal",
                    vec![W::I(0)],
                    vec![
                        N("MappingInformationType", vec![s("ByPolygonVertex")], vec![]),
                        N("ReferenceInformationType", vec![s("Direct")], vec![]),
                        N("Normals", vec![W::Ad([0.0, 0.0, 1.0].repeat(4), compress)], vec![]),
                    ],
                ),
                N(
                    "LayerElementUV",
                    vec![W::I(0)],
                    vec![
                        N("MappingInformationType", vec![s("ByPolygonVertex")], vec![]),
                        N("ReferenceInformationType", vec![s("IndexToDirect")], vec![]),
                        N("UV", vec![W::Ad(vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0], compress)], vec![]),
                        N("UVIndex", vec![W::Ai(vec![0, 1, 2, 3], compress)], vec![]),
                    ],
                ),
                N(
                    "LayerElementMaterial",
                    vec![W::I(0)],
                    vec![
                        N("MappingInformationType", vec![s("AllSame")], vec![]),
                        N("ReferenceInformationType", vec![s("IndexToDirect")], vec![]),
                        N("Materials", vec![W::Ai(vec![0], compress)], vec![]),
                    ],
                ),
            ],
        );
        let model = N("Model", vec![W::L(20), s("Quad\0\u{1}Model"), s("Mesh")], vec![p70(model_props)]);
        let material = N(
            "Material",
            vec![W::L(30), s("Red\0\u{1}Material"), s("")],
            vec![p70(vec![N("P", vec![s("DiffuseColor"), s("Color"), s(""), s("A"), W::D(1.0), W::D(0.0), W::D(0.0)], vec![])])],
        );
        let texture = N(
            "Texture",
            vec![W::L(40), s("Tex\0\u{1}Texture"), s("")],
            vec![N("RelativeFilename", vec![s("tex\\quad.png")], vec![]), N("FileName", vec![s("C:/art/tex/quad.png")], vec![])],
        );
        let mut objects = vec![geometry, model, material, texture];
        let mut connections = vec![
            conn("OO", 10, 20, None),
            conn("OO", 20, 0, None),
            conn("OO", 30, 20, None),
            conn("OP", 40, 30, Some("DiffuseColor")),
        ];
        if let Some(content) = embedded {
            objects.push(N(
                "Video",
                vec![W::L(50), s("Clip\0\u{1}Video"), s("Clip")],
                vec![N("RelativeFilename", vec![s("tex\\quad.png")], vec![]), N("Content", vec![W::R(content)], vec![])],
            ));
            connections.push(conn("OO", 50, 40, None));
        }
        binary(
            version,
            &[
                N("GlobalSettings", vec![], vec![p70(settings)]),
                N("Objects", vec![], objects),
                N("Connections", vec![], connections),
            ],
        )
    }

    #[test]
    fn embedded_video_content_wins_over_the_file() {
        let embedded = crate::samples::checker_texture(4, 4, 1);
        let embedded_png =
            crate::image_io::encode_png(4, 4, &embedded.data, crate::image_io::PngLevel::Fast).unwrap();
        let data = quad_document_with(7400, false, vec![], vec![], Some(embedded_png));
        let raw = load(&data, &[("tex/quad.png", &png_bytes())]).unwrap();
        assert_eq!(raw.textures[0].image.as_ref(), Some(&embedded));
        assert_eq!(raw.textures[0].name, "quad");
    }

    fn quad_document(version: u32, compress: bool, settings: Vec<N>, model_props: Vec<N>) -> Vec<u8> {
        quad_document_with(version, compress, settings, model_props, None)
    }

    fn png_bytes() -> Vec<u8> {
        let image = crate::samples::checker_texture(8, 8, 2);
        crate::image_io::encode_png(image.width, image.height, &image.data, crate::image_io::PngLevel::Fast).unwrap()
    }

    fn load(data: &[u8], files: &[(&str, &[u8])]) -> Result<RawImport, ImportError> {
        let mut resolver = MemoryResolver::default();
        for (name, bytes) in files {
            resolver.insert(name, Arc::from(*bytes));
        }
        load_fbx(data, &resolver, &ImportOptions::default(), true)
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn binary_quad_in_every_encoding() {
        let png = png_bytes();
        for (version, compress) in [(7400, false), (7400, true), (7500, false), (7700, true), (6100, false)] {
            let data = quad_document(version, compress, vec![], vec![p("Lcl Translation", &[100.0, 0.0, 0.0])]);
            let raw = load(&data, &[("tex/quad.png", &png)]).unwrap();
            assert!(raw.warnings.is_empty(), "{version}: {:?}", raw.warnings);
            assert_eq!(raw.mesh.triangles.len(), 2, "{version}");
            // Centimetres -> metres: the translation of 100 units is 1 m.
            let t = &raw.mesh.triangles[0];
            assert!(close(t.positions[0], Vec3::new(1.0, 0.0, 0.0)), "{:?}", t.positions);
            assert!(close(t.positions[1], Vec3::new(1.01, 0.0, 0.0)), "{:?}", t.positions);
            assert_eq!(t.normals.unwrap()[0], Vec3::Z);
            // V flipped: the corner at v = 0 becomes v = 1.
            assert_eq!(t.uvs.unwrap()[0], Vec2::new(0.0, 1.0));
            assert_eq!(raw.mesh.materials.len(), 1);
            assert_eq!(raw.mesh.materials[0].name, "Red");
            assert_eq!(raw.mesh.materials[0].base_color, [1.0, 0.0, 0.0, 1.0]);
            assert_eq!(raw.mesh.materials[0].texture, Some(0));
            assert_eq!(raw.textures[0].reference, "tex\\quad.png");
            assert_eq!(raw.textures[0].name, "quad");
            assert!(raw.textures[0].image.is_some());
        }
    }

    #[test]
    fn missing_texture_is_a_warning_and_the_absolute_name_is_tried() {
        let data = quad_document(7400, false, vec![], vec![]);
        let raw = load(&data, &[]).unwrap();
        assert!(raw.textures[0].image.is_none());
        assert!(raw.warnings.iter().any(|w| matches!(w, ImportWarning::MissingTexture { material, .. } if material == "Red")), "{:?}", raw.warnings);
        // Only the absolute FileName exists (the resolver falls back on the bare file name).
        let raw = load(&data, &[("C:/art/tex/quad.png", &png_bytes())]).unwrap();
        assert!(raw.textures[0].image.is_some());
        assert_eq!(raw.textures[0].reference, "C:/art/tex/quad.png");
    }

    #[test]
    fn z_up_settings_and_rotation_orders() {
        // 3ds Max style: Z up, unit = 1 m (UnitScaleFactor 100).
        let settings = vec![p_int("UpAxis", 2), p_int("UpAxisSign", 1), p_int("CoordAxis", 0), p_int("CoordAxisSign", 1), p("UnitScaleFactor", &[100.0])];
        let data = quad_document(7400, false, settings, vec![]);
        let raw = load(&data, &[]).unwrap();
        // File (x, y, z) -> app (x, z, -y): the quad lying in the file's XY plane (normal file +Z = up) lies flat in
        // the app's XZ plane with its normal along app +Y.
        let t = &raw.mesh.triangles[0];
        assert!(close(t.positions[2], Vec3::new(1.0, 0.0, -1.0)), "{:?}", t.positions);
        assert!(close(t.normals.unwrap()[0], Vec3::Y), "{:?}", t.normals);

        // XYZ means X is applied first: R = Rz * Ry * Rx; ZYX the other way round.
        let (rx, ry) = (DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2), DMat4::from_rotation_y(std::f64::consts::FRAC_PI_2));
        let same = |a: DMat4, b: DMat4| a.to_cols_array().iter().zip(b.to_cols_array()).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(same(euler(DVec3::new(90.0, 90.0, 0.0), 0), ry * rx));
        assert!(same(euler(DVec3::new(90.0, 90.0, 0.0), 5), rx * ry));
        // The order reaches the geometry: the quad's normal (+Z) under 90 deg about X, then about Y.
        for (order, expected) in [(0, Vec3::new(0.0, -1.0, 0.0)), (5, Vec3::X)] {
            let props = vec![p("Lcl Rotation", &[90.0, 90.0, 0.0]), p_int("RotationOrder", order)];
            let raw = load(&quad_document(7400, false, vec![], props), &[]).unwrap();
            let n = raw.mesh.triangles[0].normals.unwrap()[0];
            assert!(close(n, expected), "order {order}: {n:?}");
        }
    }

    #[test]
    fn negative_scale_and_pivots_keep_front_faces() {
        let props = vec![p("Lcl Scaling", &[-1.0, 1.0, 1.0]), p("RotationPivot", &[0.5, 0.0, 0.0]), p("ScalingPivot", &[0.5, 0.0, 0.0])];
        let raw = load(&quad_document(7400, false, vec![], props), &[]).unwrap();
        for t in &raw.mesh.triangles {
            let face = (t.positions[1] - t.positions[0]).cross(t.positions[2] - t.positions[0]);
            // Mirrored in X about x = 0.5 (pivot): the quad still spans 0..1 cm and its winding still faces +Z.
            assert!(face.z > 0.0, "{:?}", t.positions);
            assert!(t.positions.iter().all(|p| p.x >= -1e-6 && p.x <= 0.01 + 1e-6), "{:?}", t.positions);
            assert!(close(t.normals.unwrap()[0], Vec3::Z));
        }
    }

    #[test]
    fn ascii_7_and_6_documents() {
        let ascii7 = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7400
}
GlobalSettings:  {
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
	}
}
Objects:  {
	Geometry: 1, "Geometry::Tri", "Mesh" {
		Vertices: *12 {
			a: 0,0,0,100,0,0,100,100,0,0,100,0
		}
		PolygonVertexIndex: *4 {
			a: 0,1,2,-4
		}
		LayerElementUV: 0 {
			MappingInformationType: "ByVertice"
			ReferenceInformationType: "Direct"
			UV: *8 {
				a: 0,0,1,0,1,1,0,1
			}
		}
	}
	Model: 2, "Model::Tri", "Mesh" {
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,0,-50
		}
	}
	Material: 3, "Material::Green &quot;x&quot;", "" {
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",0,1,0
		}
	}
	Texture: 4, "Texture::T", "" {
		RelativeFilename: "missing.png"
	}
	Video: 5, "Video::V", "Clip" {
		RelativeFilename: "missing.png"
		Content: ,
		 "PLACEHOLDER"
	}
}
Connections:  {
	;Geometry::Tri, Model::Tri
	C: "OO",1,2
	C: "OO",2,0
	C: "OO",3,2
	C: "OP",4,3, "DiffuseColor"
	C: "OO",5,4
}
"#;
        let png = png_bytes();
        let text = ascii7.replace("PLACEHOLDER", &base64_encode(&png));
        let raw = load(text.as_bytes(), &[]).unwrap();
        assert_eq!(raw.mesh.triangles.len(), 2);
        assert!(close(raw.mesh.triangles[0].positions[1], Vec3::new(1.0, 0.0, -0.5)), "{:?}", raw.mesh.triangles[0].positions);
        assert_eq!(raw.mesh.triangles[0].uvs.unwrap()[1], Vec2::new(1.0, 1.0));
        assert_eq!(raw.mesh.materials[0].name, "Green \"x\"");
        assert_eq!(raw.mesh.materials[0].base_color, [0.0, 1.0, 0.0, 1.0]);
        // The embedded video content is decoded (no file needed).
        assert_eq!(raw.textures.len(), 1);
        assert_eq!(raw.textures[0].image.as_ref(), Some(&crate::samples::checker_texture(8, 8, 2)));
        assert!(raw.warnings.is_empty(), "{:?}", raw.warnings);

        // 6.x: geometry inside the model, inline arrays with continuation lines starting with a comma,
        // connections by name, Properties60.
        let ascii6 = "; FBX 6.1.0 project file\nObjects:  {\n\tModel: \"Model::Box\", \"Mesh\" {\n\t\tProperties60:  {\n\t\t\tProperty: \"Lcl Scaling\", \"Lcl Scaling\", \"A+\",2,2,2\n\t\t}\n\t\tVertices: 0,0,0,1,0,0\n,1,1,0,0,1,0\n\t\tPolygonVertexIndex: 0,1,2,-4\n\t}\n\tMaterial: \"Material::Blue\", \"\" {\n\t\tProperties60:  {\n\t\t\tProperty: \"DiffuseColor\", \"ColorRGB\", \"\",0,0,1\n\t\t}\n\t}\n}\nConnections:  {\n\tConnect: \"OO\", \"Model::Box\", \"Model::Scene\"\n\tConnect: \"OO\", \"Material::Blue\", \"Model::Box\"\n}\n";
        let raw = load(ascii6.as_bytes(), &[]).unwrap();
        assert_eq!(raw.mesh.triangles.len(), 2);
        assert!(close(raw.mesh.triangles[0].positions[2], Vec3::new(0.02, 0.02, 0.0)), "{:?}", raw.mesh.triangles[0].positions);
        assert_eq!(raw.mesh.materials[0].base_color, [0.0, 0.0, 1.0, 1.0]);
        assert!(raw.mesh.triangles[0].uvs.is_none());
    }

    // ---- texture UV transforms (R3-12) ----

    /// Two unit quads (x 0..1 and 2..3, facing +Z) with corner UVs (0,0), (1,0), (1,1), (0,1) (V up), polygon 0 on
    /// material A and polygon 1 on material B, each with a texture of `t.png` whose node body is `tex_a` / `tex_b`.
    fn uv_transform_document(tex_a: &str, tex_b: &str) -> String {
        format!(
            r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {{
	FBXVersion: 7400
}}
Objects:  {{
	Geometry: 1, "Geometry::Quads", "Mesh" {{
		Vertices: *24 {{
			a: 0,0,0,1,0,0,1,1,0,0,1,0,2,0,0,3,0,0,3,1,0,2,1,0
		}}
		PolygonVertexIndex: *8 {{
			a: 0,1,2,-4,4,5,6,-8
		}}
		LayerElementUV: 0 {{
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			UV: *16 {{
				a: 0,0,1,0,1,1,0,1,0,0,1,0,1,1,0,1
			}}
		}}
		LayerElementMaterial: 0 {{
			MappingInformationType: "ByPolygon"
			ReferenceInformationType: "IndexToDirect"
			Materials: *2 {{
				a: 0,1
			}}
		}}
	}}
	Model: 2, "Model::Quads", "Mesh" {{
	}}
	Material: 3, "Material::A", "" {{
	}}
	Material: 4, "Material::B", "" {{
	}}
	Texture: 5, "Texture::TA", "" {{
		RelativeFilename: "t.png"
{tex_a}
	}}
	Texture: 6, "Texture::TB", "" {{
		RelativeFilename: "t.png"
{tex_b}
	}}
}}
Connections:  {{
	C: "OO",1,2
	C: "OO",2,0
	C: "OO",3,2
	C: "OO",4,2
	C: "OP",5,3, "DiffuseColor"
	C: "OP",6,4, "DiffuseColor"
}}
"#
        )
    }

    fn props70(entries: &[(&str, [f64; 3])]) -> String {
        let lines: Vec<String> = entries
            .iter()
            .map(|(name, v)| format!("\t\t\tP: \"{name}\", \"Vector\", \"\", \"A\",{},{},{}", v[0], v[1], v[2]))
            .collect();
        format!("\t\tProperties70:  {{\n{}\n\t\t}}", lines.join("\n"))
    }

    fn uvs_of(raw: &RawImport, triangle: usize) -> [Vec2; 3] {
        raw.mesh.triangles[triangle].uvs.unwrap()
    }

    fn uvs_close(a: [Vec2; 3], b: [Vec2; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (*a - b).abs().max_element() < 1e-6)
    }

    /// Fan triangle 0 of a quad uses corners 0, 1, 2: UVs (0,0), (1,0), (1,1) of the file, V up.
    fn expected_first_triangle(f: impl Fn(f32, f32) -> (f32, f32)) -> [Vec2; 3] {
        [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)].map(|(u, v)| {
            let (u, v) = f(u, v);
            Vec2::new(u, 1.0 - v)
        })
    }

    #[test]
    fn texture_uv_transform_scale_and_offset_are_applied_before_the_v_flip() {
        let t = props70(&[("Translation", [0.25, 0.25, 0.0]), ("Scaling", [2.0, 2.0, 1.0])]);
        let raw = load(uv_transform_document(&t, &t).as_bytes(), &[]).unwrap();
        assert_eq!(raw.mesh.triangles.len(), 4);
        let expected = expected_first_triangle(|u, v| (2.0 * u + 0.25, 2.0 * v + 0.25));
        assert!(uvs_close(uvs_of(&raw, 0), expected), "{:?}", uvs_of(&raw, 0));
        assert!(uvs_close(uvs_of(&raw, 2), expected), "{:?}", uvs_of(&raw, 2));
        // Identity properties (as every exporter writes them) change nothing.
        let identity = props70(&[("Translation", [0.0; 3]), ("Rotation", [0.0; 3]), ("Scaling", [1.0; 3])]);
        let raw = load(uv_transform_document(&identity, "").as_bytes(), &[]).unwrap();
        assert_eq!(uvs_of(&raw, 0), expected_first_triangle(|u, v| (u, v)));
        assert_eq!(uvs_of(&raw, 2), expected_first_triangle(|u, v| (u, v)));
    }

    #[test]
    fn texture_uv_rotation_turns_around_its_pivot() {
        // A non-uniform scale around the tile centre, then a quarter turn counter-clockwise (V up) around it.
        let t = props70(&[
            ("Rotation", [0.0, 0.0, 90.0]),
            ("TextureRotationPivot", [0.5, 0.5, 0.0]),
            ("Scaling", [2.0, 1.0, 1.0]),
            ("TextureScalingPivot", [0.5, 0.5, 0.0]),
        ]);
        let raw = load(uv_transform_document(&t, "").as_bytes(), &[]).unwrap();
        let expected = expected_first_triangle(|u, v| {
            // Scale around (0.5, 0.5), then rotate by +90 degrees around (0.5, 0.5).
            let (x, y) = ((u - 0.5) * 2.0, v - 0.5);
            (0.5 - y, 0.5 + x)
        });
        assert!(uvs_close(uvs_of(&raw, 0), expected), "{:?} {expected:?}", uvs_of(&raw, 0));
        // (1, 0) -> scaled (1.5, 0) -> turned (1, 1.5) -> flipped (1, -0.5).
        assert!((uvs_of(&raw, 0)[1] - Vec2::new(1.0, -0.5)).abs().max_element() < 1e-6, "{:?}", uvs_of(&raw, 0));
        // Rotation X and Y are not UV rotations and are ignored.
        let t = props70(&[("Rotation", [30.0, 45.0, 0.0])]);
        let raw = load(uv_transform_document(&t, "").as_bytes(), &[]).unwrap();
        assert_eq!(uvs_of(&raw, 0), expected_first_triangle(|u, v| (u, v)));
    }

    #[test]
    fn older_model_uv_names_are_read_and_the_properties_win() {
        let old = "\t\tModelUVTranslation: 0.5,0\n\t\tModelUVScaling: 2,4";
        let raw = load(uv_transform_document(old, "").as_bytes(), &[]).unwrap();
        assert!(uvs_close(uvs_of(&raw, 0), expected_first_triangle(|u, v| (2.0 * u + 0.5, 4.0 * v))), "{:?}", uvs_of(&raw, 0));
        // Both present (as in Assimp's embedded_override_uv.fbx): the Properties70 values are used.
        let both = format!("{old}\n{}", props70(&[("Translation", [0.0, 0.25, 0.0]), ("Scaling", [3.0, 1.0, 1.0])]));
        let raw = load(uv_transform_document(&both, "").as_bytes(), &[]).unwrap();
        assert!(uvs_close(uvs_of(&raw, 0), expected_first_triangle(|u, v| (3.0 * u, v + 0.25))), "{:?}", uvs_of(&raw, 0));
        // A zero scale component counts as 1; non-numbers and non-finite values are ignored.
        let odd = "\t\tModelUVTranslation: \"bad\",\"x\"\n\t\tModelUVScaling: 0,2";
        let raw = load(uv_transform_document(odd, "").as_bytes(), &[]).unwrap();
        assert!(uvs_close(uvs_of(&raw, 0), expected_first_triangle(|u, v| (u, 2.0 * v))), "{:?}", uvs_of(&raw, 0));
        let huge = "		Properties70:  {
			P: \"Scaling\", \"Vector\", \"\", \"A\",1e999,2,1
		}";
        let raw = load(uv_transform_document(huge, "").as_bytes(), &[]).unwrap();
        assert_eq!(uvs_of(&raw, 0), expected_first_triangle(|u, v| (u, v)));
    }

    #[test]
    fn materials_sharing_an_image_with_different_transforms_keep_one_texture() {
        // As in the glTF loader: the image names one texture set; each material's transform moves only its own
        // triangles' UVs.
        let a = props70(&[("Scaling", [2.0, 2.0, 1.0])]);
        let b = props70(&[("Translation", [0.5, 0.0, 0.0])]);
        let raw = load(uv_transform_document(&a, &b).as_bytes(), &[]).unwrap();
        assert_eq!(raw.mesh.materials.len(), 2);
        assert_eq!(raw.textures.len(), 1);
        assert!(raw.mesh.materials.iter().all(|m| m.texture == Some(0)));
        assert!(uvs_close(uvs_of(&raw, 0), expected_first_triangle(|u, v| (2.0 * u, 2.0 * v))));
        assert!(uvs_close(uvs_of(&raw, 2), expected_first_triangle(|u, v| (u + 0.5, v))));
    }

    /// The glTF twin of `uv_transform_document`: the same two quads as two primitives with two materials showing
    /// one image through `KHR_texture_transform` `offset` / `scale`, UVs stored V down.
    fn uv_transform_gltf(offset: [f32; 2], scale: [f32; 2]) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        let positions: [[f32; 3]; 8] = [
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 1.0, 0.0], [2.0, 1.0, 0.0],
        ];
        let uvs: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for p in positions.iter().flatten() {
            bin.extend_from_slice(&p.to_le_bytes());
        }
        for uv in uvs.iter().flatten() {
            bin.extend_from_slice(&uv.to_le_bytes());
        }
        for i in [0u16, 1, 2, 0, 2, 3] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let primitive = |first: usize, material: usize| {
            serde_json::json!({ "attributes": { "POSITION": first, "TEXCOORD_0": 2 }, "indices": 3, "material": material })
        };
        let material = || {
            serde_json::json!({ "pbrMetallicRoughness": { "baseColorTexture": {
                "index": 0, "extensions": { "KHR_texture_transform": { "offset": offset, "scale": scale } } } } })
        };
        let doc = serde_json::json!({
            "asset": { "version": "2.0" },
            "extensionsUsed": ["KHR_texture_transform"],
            "buffers": [{ "byteLength": bin.len(), "uri": format!("data:application/octet-stream;base64,{}", base64_encode(&bin)) }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 48 },
                { "buffer": 0, "byteOffset": 48, "byteLength": 48 },
                { "buffer": 0, "byteOffset": 96, "byteLength": 32 },
                { "buffer": 0, "byteOffset": 128, "byteLength": 12 },
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "type": "VEC3", "count": 4, "min": [0, 0, 0], "max": [1, 1, 0] },
                { "bufferView": 1, "componentType": 5126, "type": "VEC3", "count": 4, "min": [2, 0, 0], "max": [3, 1, 0] },
                { "bufferView": 2, "componentType": 5126, "type": "VEC2", "count": 4 },
                { "bufferView": 3, "componentType": 5123, "type": "SCALAR", "count": 6 },
            ],
            "meshes": [{ "primitives": [primitive(0, 0), primitive(1, 1)] }],
            "nodes": [{ "mesh": 0 }],
            "scenes": [{ "nodes": [0] }],
            "scene": 0,
            "materials": [material(), material()],
            "textures": [{ "source": 0 }],
            "images": [{ "uri": "t.png" }],
        });
        serde_json::to_vec(&doc).unwrap()
    }

    fn import_memory(format: ModelFormat, main: Vec<u8>) -> ImportedModel {
        let png = png_bytes();
        let file = |name: &str, bytes: &[u8]| crate::import::SourceFile {
            key: normalize_ref(name),
            original_name: name.to_owned(),
            bytes: Arc::from(bytes),
        };
        let bundle = SourceBundle {
            format,
            main: file(&format!("Scene.{}", format.extension()), &main),
            sidecars: vec![file("t.png", &png)],
            extras: Vec::new(),
            original_path: None,
        };
        import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap()
    }

    #[test]
    fn texture_uv_transform_gives_the_uv_hash_of_the_equivalent_gltf() {
        // FBX: scale 2, offset 0.25 in V-up space. The glTF equivalent in V-down space keeps the U offset and moves
        // V to 1 - 2 - 0.25 = -1.25 (the conversion Blender's glTF exporter uses).
        let t = props70(&[("Translation", [0.25, 0.25, 0.0]), ("Scaling", [2.0, 2.0, 1.0])]);
        let fbx = import_memory(ModelFormat::Fbx, uv_transform_document(&t, &t).into_bytes());
        let gltf = import_memory(ModelFormat::Gltf, uv_transform_gltf([0.25, -1.25], [2.0, 2.0]));
        assert_eq!(fbx.mesh.tri_count(), 4);
        assert_eq!(gltf.mesh.tri_count(), 4);
        for t in 0..fbx.mesh.tri_count() {
            let (a, b) = (fbx.mesh.tri_uvs(t), gltf.mesh.tri_uvs(t));
            assert!(a.iter().zip(b).all(|(a, b)| (*a - b).abs().max_element() < 1e-6), "triangle {t}: {a:?} {b:?}");
        }
        assert_eq!(fbx.mesh.uv_hash, gltf.mesh.uv_hash);
        // Without the transform the hashes differ (the test can fail).
        let plain = import_memory(ModelFormat::Fbx, uv_transform_document("", "").into_bytes());
        assert_ne!(plain.mesh.uv_hash, fbx.mesh.uv_hash);
        let plain_gltf = import_memory(ModelFormat::Gltf, uv_transform_gltf([0.0, 0.0], [1.0, 1.0]));
        assert_eq!(plain.mesh.uv_hash, plain_gltf.mesh.uv_hash);
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

    #[test]
    fn ngons_are_ear_clipped() {
        // An L-shaped hexagon (concave): a fan from corner 0 would leave the polygon.
        let pts = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]].map(|[x, y]| DVec3::new(x, y, 0.0));
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 4);
        let area: f64 = tris.iter().map(|t| (pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]).z * 0.5).sum();
        assert!((area - 3.0).abs() < 1e-12, "{area}");
        assert!(tris.iter().all(|t| (pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]).z > 0.0), "same winding");
        // A degenerate polygon falls back to a fan without panicking.
        assert_eq!(triangulate(&[DVec3::ZERO; 5]).len(), 3);
    }

    #[test]
    fn hostile_input_errors_and_never_panics() {
        let good = quad_document(7400, true, vec![], vec![]);
        // Every truncation.
        for len in 0..good.len() {
            let _ = load(&good[..len], &[]);
        }
        // Every single-byte corruption of the header and record structure (a few values per byte).
        for at in 0..good.len().min(600) {
            for value in [0u8, 0xff, 0x7f, 0x01] {
                let mut bad = good.clone();
                bad[at] = value;
                let _ = load(&bad, &[]);
            }
        }
        // Huge counts and lengths.
        let mut bad = good.clone();
        let at = good.windows(8).position(|w| w == b"Vertices").unwrap() + 8 + 1;
        bad[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(load(&bad, &[]).is_err());
        // Deep nesting.
        let mut deep = N("Leaf", vec![W::I(1)], vec![]);
        for _ in 0..200 {
            deep = N("Deep", vec![], vec![deep]);
        }
        assert!(load(&binary(7400, &[deep]), &[]).is_err());
        let mut text = String::new();
        for _ in 0..200 {
            text.push_str("A: {\n");
        }
        assert!(load(text.as_bytes(), &[]).is_err());
        // Not FBX at all, a damaged binary header, an unknown version.
        assert!(load(b"", &[]).is_err());
        assert!(load(b"\0\0\0garbage", &[]).is_err());
        let mut version = good.clone();
        version[23..27].copy_from_slice(&99u32.to_le_bytes());
        assert!(load(&version, &[]).is_err());
        let mut spaces = good.clone();
        spaces.iter_mut().for_each(|b| if *b == 0 { *b = b' ' });
        // A damaged file says so in its own words ("invalid FBX file: ..."), not as an I/O error.
        let damaged = load(&spaces, &[]);
        assert!(matches!(damaged, Err(ImportError::InvalidFbx(_))), "{:?}", damaged.err());
        assert!(load(&spaces, &[]).err().unwrap().to_string().starts_with("invalid FBX file: "));
        // Unterminated ASCII constructs.
        for text in ["Objects: {", "Objects: { Geometry: 1, \"G\", \"Mesh\" { Vertices: *3 { a: 1,2", "\"open string", "Objects: { Model: 1 { P: ,,,, } }"] {
            let _ = load(text.as_bytes(), &[]);
        }
        // Index arrays pointing nowhere: skipped polygons, not a panic.
        let data = binary(
            7400,
            &[
                N("Objects", vec![], vec![
                    N("Geometry", vec![W::L(1), s("G"), s("Mesh")], vec![
                        N("Vertices", vec![W::Ad(vec![0.0; 9], false)], vec![]),
                        N("PolygonVertexIndex", vec![W::Ai(vec![0, 1, 99, -2_000_000_000, 5, -1], false)], vec![]),
                        N("LayerElementUV", vec![W::I(0)], vec![
                            N("MappingInformationType", vec![s("ByPolygonVertex")], vec![]),
                            N("ReferenceInformationType", vec![s("IndexToDirect")], vec![]),
                            N("UV", vec![W::Ad(vec![0.0; 2], false)], vec![]),
                            N("UVIndex", vec![W::Ai(vec![-5, 7, 0, 0, 0, 0], false)], vec![]),
                        ]),
                    ]),
                    N("Model", vec![W::L(2), s("M"), s("Mesh")], vec![]),
                ]),
                N("Connections", vec![], vec![conn("OO", 1, 2, None), conn("OO", 2, 2, None)]),
            ],
        );
        let raw = load(&data, &[]).unwrap();
        assert!(raw.mesh.triangles.is_empty());
        assert!(raw.warnings.iter().any(|w| matches!(w, ImportWarning::SkippedPrimitive { .. })));
    }

    /// Diagnostics: prints the node tree of `FBX_DUMP=<path>` (run with `--ignored --nocapture`).
    #[test]
    #[ignore]
    fn dump_tree() {
        fn dump(n: &Node, depth: usize) {
            let props: Vec<String> = n
                .props
                .iter()
                .map(|p| match p {
                    Prop::Ints(v) => format!("ints[{}]", v.len()),
                    Prop::Floats(v) => format!("floats[{}] {:?}", v.len(), &v[..v.len().min(12)]),
                    Prop::Raw(v) => format!("raw[{}]", v.len()),
                    other => format!("{other:?}"),
                })
                .collect();
            eprintln!("{}{} {}", "  ".repeat(depth), n.name, props.join(", "));
            for c in &n.children {
                dump(c, depth + 1);
            }
        }
        let Ok(path) = std::env::var("FBX_DUMP") else { return };
        for n in parse_document(&std::fs::read(path).unwrap()).unwrap() {
            if n.name == "Objects" || n.name == "GlobalSettings" || n.name == "Connections" {
                dump(&n, 0);
            }
        }
    }

    // ---- the Assimp sample files (not part of the repository) ----

    const FBX_SAMPLES: &str = r"E:\claude_downloads\fbx_samples";

    fn import_sample(name: &str) -> Option<Result<ImportedModel, ImportError>> {
        let path = std::path::Path::new(FBX_SAMPLES).join(name);
        if !path.is_file() {
            eprintln!("skipped: {} is not available", path.display());
            return None;
        }
        Some(import_from_path_auto_uvs(&path, &ImportOptions::default(), true, &NoProgress))
    }

    #[test]
    fn every_sample_file_imports_or_errors_cleanly() {
        let Ok(dir) = std::fs::read_dir(FBX_SAMPLES) else {
            eprintln!("skipped: {FBX_SAMPLES} is not available");
            return;
        };
        let mut names: Vec<String> = dir
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_ascii_lowercase().ends_with(".fbx"))
            .collect();
        names.sort();
        for name in names {
            let start = std::time::Instant::now();
            let result = import_sample(&name).expect("listed");
            match &result {
                Ok(model) => eprintln!(
                    "{name}: {} tris, {} materials, {} sets, textures {:?}, bounds {:?}, warnings {:?} ({:?})",
                    model.mesh.tri_count(),
                    model.mesh.materials.len(),
                    model.mesh.sets.len(),
                    model.textures.iter().map(|t| (t.reference.clone(), t.image.as_ref().map(|i| (i.width, i.height)))).collect::<Vec<_>>(),
                    model.mesh.bounds,
                    model.warnings,
                    start.elapsed()
                ),
                Err(e) => eprintln!("{name}: error {e} ({:?})", start.elapsed()),
            }
            match name.as_str() {
                // Every NUL byte of this copy was replaced by a space: damaged beyond repair.
                "transparentTest.fbx" => assert!(result.is_err()),
                // A skeleton and its animation without any mesh.
                "animation_with_skeleton.fbx" => {}
                _ => {
                    let model = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                    assert!(model.mesh.tri_count() > 0 && !model.mesh.sets.is_empty(), "{name}");
                    // The project bundle alone reproduces the import (R7).
                    let again = import_from_bundle(&model.bundle, &model.options, true, &NoProgress).unwrap();
                    assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash, "{name}");
                    assert_eq!(again.textures.len(), model.textures.len(), "{name}");
                    for (a, b) in again.textures.iter().zip(&model.textures) {
                        assert_eq!(a.image, b.image, "{name}: {}", a.reference);
                    }
                }
            }
        }
    }

    /// R3-12: reading texture UV transforms must not move the UVs of the 21 importable samples. Values recorded with
    /// the transform switched off (base behaviour) and with it on: identical. The `embedded_*_uv` files carry
    /// non-identity transforms but no UVs of their own (the generated layout is used, untransformed).
    #[test]
    fn sample_uv_hashes_are_unchanged_by_texture_transforms() {
        const HASHES: [(&str, u64); 21] = [
            ("animation_with_skeleton.fbx", 0x6264572a60bea784),
            ("box.fbx", 0x0a6dfb873fe4dfd5),
            ("boxWithCompressedCTypeArray.FBX", 0xb574732d06b41f7d),
            ("boxWithUncompressedCTypeArray.FBX", 0xb574732d06b41f7d),
            ("box_orphant_embedded_texture.fbx", 0xdd41528669c7b4e5),
            ("close_to_identity_transforms.fbx", 0x8b12e1661560b55f),
            ("cubes_nonames.fbx", 0x4ba11e20644cff1e),
            ("cubes_with_mirroring_and_pivot.fbx", 0xf9e027d8ad71409e),
            ("cubes_with_names.fbx", 0x4ba11e20644cff1e),
            ("cubes_with_outofrange_float.fbx", 0x4ba11e20644cff1e),
            ("embedded_double_uv.fbx", 0xb8a6902aa29bf68a),
            ("embedded_float_uv.fbx", 0xb8a6902aa29bf68a),
            ("embedded_integer_uv.fbx", 0xb8a6902aa29bf68a),
            ("embedded_invalid_uv.fbx", 0xb8a6902aa29bf68a),
            ("embedded_override_uv.fbx", 0xb8a6902aa29bf68a),
            ("global_settings.fbx", 0x3c504732efa89fde),
            ("huesitos.fbx", 0xd20c444440b9e0c9),
            ("maxPbrMaterial_metalRough.fbx", 0xdd41528669c7b4e5),
            ("maxPbrMaterial_specGloss.fbx", 0xdd41528669c7b4e5),
            ("phong_cube.fbx", 0xbfe2d06eb5684b85),
            ("spider.fbx", 0xe247d5f76f14b886),
        ];
        // The sample folder is not part of the repository: without it the test says so and checks nothing. With it,
        // every one of the 21 files must be there, so a partial copy cannot pass for full coverage.
        if !std::path::Path::new(FBX_SAMPLES).is_dir() {
            eprintln!("SKIPPED sample_uv_hashes_are_unchanged_by_texture_transforms: {FBX_SAMPLES} is not available");
            return;
        }
        for (name, hash) in HASHES {
            let result = import_sample(name).unwrap_or_else(|| panic!("{name} is missing from {FBX_SAMPLES}"));
            let model = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(model.mesh.uv_hash, hash, "{name}: {:016x}", model.mesh.uv_hash);
        }
        eprintln!("checked the uv_hash of {} FBX samples in {FBX_SAMPLES}", HASHES.len());
    }

    #[test]
    fn sample_boxes_face_outwards_and_the_orphaned_texture_is_found() {
        // Closed boxes: every triangle's winding agrees with its stored (outward) normal, also under the geometric
        // rotation / non-uniform scaling of box.fbx and the negative scales and pivots of the mirrored cubes.
        for name in ["box.fbx", "cubes_with_mirroring_and_pivot.fbx", "phong_cube.fbx"] {
            let Some(result) = import_sample(name) else { continue };
            let model = result.unwrap();
            let mesh = &model.mesh;
            if name == "box.fbx" {
                // 200 cm cube with a geometric scale of ~0.0016: about 0.3 m, rotated by the geometric rotation.
                assert_eq!(mesh.tri_count(), 12);
                let size = mesh.bounds.max - mesh.bounds.min;
                assert!(size.min_element() > 0.25 && size.max_element() < 0.6, "{size:?}");
            }
            let mut agree = 0;
            for t in 0..mesh.tri_count() {
                let n = mesh.tri_normals(t)[0] + mesh.tri_normals(t)[1] + mesh.tri_normals(t)[2];
                if mesh.geo_normal(t).dot(n) > 0.0 {
                    agree += 1;
                }
            }
            assert_eq!(agree, mesh.tri_count(), "{name}: triangles facing against their normals");
        }
        if let Some(result) = import_sample("box_orphant_embedded_texture.fbx") {
            let model = result.unwrap();
            // The used video has no content and its file is missing; the orphaned video of the same file name
            // embeds the image.
            assert!(model.textures.iter().any(|t| t.image.is_some()), "{:?}", model.warnings);
        }
    }

    #[test]
    fn fbx_bundle_round_trip_keeps_sidecar_textures() {
        let png = png_bytes();
        let data = quad_document(7400, false, vec![], vec![]);
        let dir = crate::testutil::temp_dir("fbx-bundle");
        std::fs::create_dir_all(dir.join("tex")).unwrap();
        std::fs::write(dir.join("quad.fbx"), &data).unwrap();
        std::fs::write(dir.join("tex").join("quad.png"), &png).unwrap();
        let model = import_from_path_auto_uvs(&dir.join("quad.fbx"), &ImportOptions::default(), true, &NoProgress).unwrap();
        assert_eq!(model.bundle.format, ModelFormat::Fbx);
        let keys: Vec<&str> = model.bundle.sidecars.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["tex/quad.png"]);
        std::fs::remove_dir_all(&dir).unwrap();
        let bundle = SourceBundle { extras: Vec::new(), ..model.bundle.clone() };
        let again = import_from_bundle(&bundle, &model.options, true, &NoProgress).unwrap();
        assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);
        assert!(again.textures[0].image.is_some());
    }

    /// Objects-tabs WP1: one object per `Model` with geometry, named by the model; a model without geometry is none.
    #[test]
    fn models_are_objects() {
        let ascii6 = "; FBX 6.1.0 project file\nObjects:  {\n\tModel: \"Model::Box\", \"Mesh\" {\n\t\tVertices: 0,0,0,1,0,0,1,1,0,0,1,0\n\t\tPolygonVertexIndex: 0,1,2,-4\n\t}\n\tModel: \"Model::Null\", \"Null\" {\n\t}\n\tModel: \"Model::Lid\", \"Mesh\" {\n\t\tVertices: 5,0,0,6,0,0,5,1,0\n\t\tPolygonVertexIndex: 0,1,-3\n\t}\n}\nConnections:  {\n\tConnect: \"OO\", \"Model::Box\", \"Model::Scene\"\n\tConnect: \"OO\", \"Model::Null\", \"Model::Scene\"\n\tConnect: \"OO\", \"Model::Lid\", \"Model::Scene\"\n}\n";
        let raw = load(ascii6.as_bytes(), &[]).unwrap();
        assert_eq!(raw.mesh.triangles.len(), 3);
        assert_eq!(raw.objects.names, ["Box", "Lid"]);
        assert_eq!(raw.objects.tri_object, [0, 0, 1]);
        let model = import_memory(ModelFormat::Fbx, ascii6.as_bytes().to_vec());
        let names: Vec<&str> = model.mesh.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["Box", "Lid"]);
        assert_eq!(model.mesh.objects.iter().map(|o| o.tri_count).collect::<Vec<_>>(), [2, 1]);
    }
}
