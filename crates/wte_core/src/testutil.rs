//! Test-only helpers (ARCHITECTURE.md 4.8, 5.25). Always compiled (tiny) so that the integration tests and the
//! tests of `wte_app` can use them; not part of the supported API.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::blob::BlobStore;
use crate::doc_build::{DocumentParts, SetParts};
use crate::document::Document;
use crate::ids::LayerId;
use crate::import::{ImportOptions, ImportedModel, ImportedTexture, ModelFormat, SourceBundle, SourceFile};
use crate::layer::{Layer, LayerKind};
use crate::math::fnv1a64;
use crate::mesh::{MeshData, RawMesh, RawObjects};
use crate::mesh_build::{NormalizeOptions, normalize, normalize_with_objects};
use crate::pixel::PixelBuf;
use crate::progress::NoProgress;

/// Creates and returns a fresh directory `<workspace>/target/wte-test-tmp/<name>-<pid>-<counter>/` (on E:, never in
/// `%TEMP%`). The caller removes it at the end of the test.
pub fn temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let safe: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wte-test-tmp")
        .join(format!("{safe}-{}-{counter}", std::process::id()));
    // A leftover from a crashed run with a recycled pid must not leak into this test.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the test temp directory under target/");
    dir
}

/// `normalize(raw, &NormalizeOptions::default()).0`.
pub fn mesh_of(raw: RawMesh) -> MeshData {
    normalize(raw, &NormalizeOptions::default()).0
}

/// Peak signal-to-noise ratio in dB over all four channels. Identical images give `f64::INFINITY`; images of
/// different size (or empty ones) give 0.
pub fn psnr(a: &PixelBuf, b: &PixelBuf) -> f64 {
    if a.width != b.width || a.height != b.height || a.data.len() != b.data.len() || a.data.is_empty() {
        return 0.0;
    }
    let sum: f64 = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum();
    if sum == 0.0 {
        return f64::INFINITY;
    }
    let mse = sum / a.data.len() as f64;
    10.0 * (255.0 * 255.0 / mse).log10()
}

/// Deterministic hash of an image (size and bytes).
pub fn hash_pixels(buf: &PixelBuf) -> u64 {
    let mut h = fnv1a64(0xcbf2_9ce4_8422_2325, &buf.width.to_le_bytes());
    h = fnv1a64(h, &buf.height.to_le_bytes());
    fnv1a64(h, &buf.data)
}

/// The pixels of a layer as a dense image of its set's size (an unallocated paint layer is all zero, a stamp cache
/// is placed on the canvas). Panics for an unknown layer.
pub fn layer_dense(doc: &Document, id: LayerId) -> PixelBuf {
    let (set, _) = doc.find_layer(id).expect("unknown layer");
    let set = doc.set(set).expect("set");
    doc.layer(id).expect("layer").to_dense(set.width, set.height)
}

/// Deterministic xorshift64* random number generator for tests.
#[derive(Clone, Debug)]
pub struct XorShift(u64);

impl XorShift {
    /// Generator with the given seed (any value; 0 is remapped to a fixed non-zero state).
    pub fn new(seed: u64) -> XorShift {
        // Scramble the seed so that small consecutive seeds give unrelated streams.
        let mixed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        XorShift(if mixed == 0 { 0x2545_f491_4f6c_dd1d } else { mixed })
    }

    /// Next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as u32
    }

    /// Next float, uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Builds an `ImportedModel` without any importer: `normalize` with default options, a dummy in-memory `SourceBundle`
/// whose main file is the bytes `b"dummy"`, and `ImportedTexture`s named `tex0`, `tex1`, ... holding `textures`
/// (padded with missing textures up to the highest index a material references).
pub fn imported_from_raw(raw: RawMesh, textures: Vec<Option<PixelBuf>>) -> ImportedModel {
    imported_from_raw_objects(raw, &RawObjects::default(), textures)
}

/// `imported_from_raw` with object data (`normalize_with_objects`): objects-tabs tests.
pub fn imported_from_raw_objects(raw: RawMesh, objects: &RawObjects, textures: Vec<Option<PixelBuf>>) -> ImportedModel {
    let referenced = raw.materials.iter().filter_map(|m| m.texture).max().map_or(0, |m| m as usize + 1);
    let mut textures = textures;
    if textures.len() < referenced {
        textures.resize(referenced, None);
    }
    let (mesh, report) = normalize_with_objects(raw, objects, &NormalizeOptions::default());
    let textures = textures
        .into_iter()
        .enumerate()
        .map(|(i, image)| ImportedTexture { name: format!("tex{i}"), reference: format!("tex{i}.png"), image })
        .collect();
    let bundle = SourceBundle {
        format: ModelFormat::Obj,
        main: SourceFile {
            key: "dummy.obj".to_owned(),
            original_name: "dummy.obj".to_owned(),
            bytes: Arc::from(&b"dummy"[..]),
        },
        sidecars: Vec::new(),
        extras: Vec::new(),
        original_path: None,
    };
    ImportedModel {
        name: "dummy".to_owned(),
        mesh,
        textures,
        report,
        warnings: Vec::new(),
        bundle,
        // Spelled out (the documented defaults) so this helper does not depend on the importer package.
        options: ImportOptions { generate_fallback_uvs: false, max_texture_dim: 8192 },
    }
}

/// Builds a document for algorithm tests without any importer: `imported_from_raw`, then **one** blank white,
/// unlocked raster layer of `size x size` per set (active), assembled with `Document::from_parts`.
/// Such a document cannot be re-imported; persistence tests use `Document::sample` or a real import.
pub fn doc_from_raw(raw: RawMesh, size: u32) -> Document {
    doc_from_raw_objects(raw, &RawObjects::default(), size)
}

/// `doc_from_raw` with object data (`normalize_with_objects`): objects-tabs tests.
pub fn doc_from_raw_objects(raw: RawMesh, objects: &RawObjects, size: u32) -> Document {
    let model = imported_from_raw_objects(raw, objects, Vec::new());
    let mut next_id = 1u64;
    let sets: Vec<SetParts> = model
        .mesh
        .sets
        .iter()
        .map(|info| {
            let id = LayerId(next_id);
            next_id += 1;
            let pixels = Arc::new(PixelBuf::filled(size, size, [255; 4]));
            SetParts {
                name: info.name.clone(),
                width: size,
                height: size,
                layers: vec![Layer::new(id, "Layer 1", LayerKind::Raster(pixels))],
                active: Some(id),
            }
        })
        .collect();
    let parts = DocumentParts {
        name: "test".to_owned(),
        model,
        sets,
        stamps: Vec::new(),
        blobs: BlobStore::default(),
        ui_state: serde_json::Value::Null,
        next_id,
    };
    Document::from_parts(parts, &NoProgress).expect("doc_from_raw: Document::from_parts failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dirs_are_unique_and_under_target() {
        let a = temp_dir("testutil self/test");
        let b = temp_dir("testutil self/test");
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        let text = a.to_string_lossy().replace('\\', "/");
        assert!(text.contains("/target/wte-test-tmp/testutil_self_test-"), "{text}");
        std::fs::write(a.join("probe.txt"), b"x").unwrap();
        std::fs::remove_dir_all(&a).unwrap();
        std::fs::remove_dir_all(&b).unwrap();
    }

    #[test]
    fn psnr_and_hash() {
        let a = PixelBuf::filled(4, 4, [10, 20, 30, 255]);
        let mut b = a.clone();
        assert_eq!(psnr(&a, &b), f64::INFINITY);
        assert_eq!(hash_pixels(&a), hash_pixels(&b));
        b.set(1, 1, [11, 20, 30, 255]);
        let p = psnr(&a, &b);
        // One channel of 64 differs by 1: mse = 1/64.
        assert!((p - 10.0 * (255.0f64 * 255.0 * 64.0).log10()).abs() < 1e-9, "{p}");
        assert_ne!(hash_pixels(&a), hash_pixels(&b));
        assert_eq!(psnr(&a, &PixelBuf::new(4, 5)), 0.0);
        // Same bytes, different shape: different hash.
        assert_ne!(hash_pixels(&PixelBuf::new(2, 8)), hash_pixels(&PixelBuf::new(8, 2)));
    }

    #[test]
    fn xorshift_is_deterministic_and_in_range() {
        let mut a = XorShift::new(42);
        let mut b = XorShift::new(42);
        let mut c = XorShift::new(43);
        let mut differs = false;
        let mut sum = 0.0f64;
        for _ in 0..10_000 {
            let x = a.next_u32();
            assert_eq!(x, b.next_u32());
            differs |= x != c.next_u32();
            let f = a.next_f32();
            b.next_f32();
            assert!((0.0..1.0).contains(&f));
            sum += f as f64;
        }
        assert!(differs);
        assert!((sum / 10_000.0 - 0.5).abs() < 0.02, "mean {}", sum / 10_000.0);
        let mut z = XorShift::new(0);
        assert_ne!(z.next_u32(), z.next_u32());
    }
}
