//! Identifier newtypes shared by the whole crate (ARCHITECTURE.md 4.4, 5.1).

use serde::{Deserialize, Serialize};

/// Triangle index into `MeshData` (always below 2^31).
pub type TriId = u32;

/// Texture set id; equal to the index into `Document::sets` and `MeshData::sets`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextureSetId(pub u32);

/// Layer id, allocated by `Document::alloc_id` (monotonic, never reused).
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerId(pub u64);

/// Stamp id, allocated by `Document::alloc_id` (same counter as `LayerId`).
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StampId(pub u64);

/// Blob id, allocated by `BlobStore` (monotonic).
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(pub u32);

impl TextureSetId {
    /// The set's index into `Document::sets` / `MeshData::sets`.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}
