//! Storage for the original encoded bytes of stamp images and font files (ARCHITECTURE.md 5.6).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ids::BlobId;

/// What a blob contains.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobKind {
    Image,
    Font,
}

/// One stored file: its original name, bytes and CRC-32.
#[derive(Clone, Debug)]
pub struct Blob {
    pub id: BlobId,
    pub kind: BlobKind,
    pub name: String,
    pub bytes: Arc<[u8]>,
    pub crc32: u32,
}

/// Original encoded bytes of stamp images and font files. De-duplicated by (len, crc32) then full comparison.
///
/// Blobs are never garbage-collected in memory (undo history may reference them).
#[derive(Clone, Debug, Default)]
pub struct BlobStore {
    blobs: BTreeMap<BlobId, Blob>,
    next: u32,
}

impl BlobStore {
    /// Stores `bytes` and returns its id; a blob of the same kind with identical bytes is reused (its id is returned and
    /// its name kept).
    ///
    /// Bytes in the very allocation of a stored blob of the same kind are recognised before hashing (FEATURE_TEXT
    /// 5.13: a font handed out again by `FontCatalog::face_bytes` while the document holds it). Either way the lowest
    /// id among the equal blobs of that kind is returned, as in round 3.
    pub fn insert(&mut self, kind: BlobKind, name: &str, bytes: Arc<[u8]>) -> BlobId {
        if let Some(shared) = self.blobs.values().find(|b| b.kind == kind && Arc::ptr_eq(&b.bytes, &bytes)) {
            // The stored CRC stands in for hashing. The lowest equal blob wins, as on the hashing path (two equal
            // blobs exist only after `insert_with_id`); it is normally `shared` itself, found without a byte compare.
            let lowest = self.blobs.values().find(|b| {
                b.kind == kind
                    && b.bytes.len() == shared.bytes.len()
                    && b.crc32 == shared.crc32
                    && (Arc::ptr_eq(&b.bytes, &shared.bytes) || b.bytes[..] == shared.bytes[..])
            });
            return lowest.map_or(shared.id, |b| b.id);
        }
        let crc32 = crc32fast::hash(&bytes);
        let duplicate = self.blobs.values().find(|b| {
            b.kind == kind && b.bytes.len() == bytes.len() && b.crc32 == crc32 && b.bytes[..] == bytes[..]
        });
        if let Some(existing) = duplicate {
            return existing.id;
        }
        let id = BlobId(self.next);
        self.next = self.next.saturating_add(1);
        self.blobs.insert(id, Blob { id, kind, name: name.to_owned(), bytes, crc32 });
        id
    }

    /// Stores `bytes` under a known id (project load), replacing any blob with that id. Later `insert` calls allocate
    /// ids above every id seen so far.
    pub fn insert_with_id(&mut self, id: BlobId, kind: BlobKind, name: &str, bytes: Arc<[u8]>) {
        let crc32 = crc32fast::hash(&bytes);
        self.next = self.next.max(id.0.saturating_add(1));
        self.blobs.insert(id, Blob { id, kind, name: name.to_owned(), bytes, crc32 });
    }

    /// The blob with this id.
    pub fn get(&self, id: BlobId) -> Option<&Blob> {
        self.blobs.get(&id)
    }

    /// All blobs in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = &Blob> {
        self.blobs.values()
    }

    /// Number of stored blobs.
    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    /// True when no blob is stored.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(v: &[u8]) -> Arc<[u8]> {
        Arc::from(v)
    }

    #[test]
    fn insert_allocates_monotonic_ids() {
        let mut s = BlobStore::default();
        assert!(s.is_empty());
        let a = s.insert(BlobKind::Image, "a.png", bytes(b"aaaa"));
        let b = s.insert(BlobKind::Image, "b.png", bytes(b"bbbb"));
        let c = s.insert(BlobKind::Font, "c.ttf", bytes(b"cccc"));
        assert!(a < b && b < c);
        assert_eq!(s.len(), 3);
        assert_eq!(s.get(b).unwrap().name, "b.png");
        assert_eq!(s.get(c).unwrap().kind, BlobKind::Font);
        assert_eq!(&s.get(a).unwrap().bytes[..], b"aaaa");
        assert_eq!(s.get(a).unwrap().crc32, crc32fast::hash(b"aaaa"));
        assert!(s.get(BlobId(999)).is_none());
        let ids: Vec<BlobId> = s.iter().map(|b| b.id).collect();
        assert_eq!(ids, vec![a, b, c]);
    }

    #[test]
    fn duplicates_return_the_existing_id() {
        let mut s = BlobStore::default();
        let a = s.insert(BlobKind::Image, "first.png", bytes(b"same bytes"));
        let again = s.insert(BlobKind::Image, "second.png", bytes(b"same bytes"));
        assert_eq!(a, again);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(a).unwrap().name, "first.png");
        // Same length, different content.
        let other = s.insert(BlobKind::Image, "third.png", bytes(b"same bytez"));
        assert_ne!(a, other);
        // Same bytes but another kind is a different blob.
        let font = s.insert(BlobKind::Font, "font.ttf", bytes(b"same bytes"));
        assert_ne!(a, font);
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn insert_with_id_keeps_ids_and_advances_the_counter() {
        let mut s = BlobStore::default();
        s.insert_with_id(BlobId(7), BlobKind::Font, "f.ttf", bytes(b"font"));
        s.insert_with_id(BlobId(3), BlobKind::Image, "i.png", bytes(b"image"));
        assert_eq!(s.len(), 2);
        assert_eq!(s.get(BlobId(7)).unwrap().name, "f.ttf");
        assert_eq!(s.get(BlobId(3)).unwrap().crc32, crc32fast::hash(b"image"));
        let ids: Vec<u32> = s.iter().map(|b| b.id.0).collect();
        assert_eq!(ids, vec![3, 7]);
        let fresh = s.insert(BlobKind::Image, "n.png", bytes(b"new"));
        assert_eq!(fresh, BlobId(8));
        // Replacing an id keeps the count.
        s.insert_with_id(BlobId(3), BlobKind::Image, "j.png", bytes(b"other"));
        assert_eq!(s.len(), 3);
        assert_eq!(s.get(BlobId(3)).unwrap().name, "j.png");
    }

    /// TEXT 8.1 `blob_insert_knows_the_same_allocation`.
    #[test]
    fn blob_insert_knows_the_same_allocation() {
        let mut s = BlobStore::default();
        let shared = bytes(b"one font file");
        let a = s.insert(BlobKind::Font, "a.ttf", Arc::clone(&shared));
        // The same allocation again: the stored blob, its name kept, nothing added.
        let again = s.insert(BlobKind::Font, "b.ttf", Arc::clone(&shared));
        assert_eq!(a, again);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(a).unwrap().name, "a.ttf");
        assert!(Arc::ptr_eq(&s.get(a).unwrap().bytes, &shared));
        // Equal bytes in another allocation still find it (the hashing path).
        assert_eq!(s.insert(BlobKind::Font, "c.ttf", bytes(b"one font file")), a);
        // The same allocation under another kind is another blob.
        let image = s.insert(BlobKind::Image, "a.png", Arc::clone(&shared));
        assert_ne!(image, a);
        assert_eq!(s.len(), 2);
        // A blob stored under a known id is recognised by its allocation too.
        let loaded = bytes(b"loaded font");
        s.insert_with_id(BlobId(40), BlobKind::Font, "l.ttf", Arc::clone(&loaded));
        assert_eq!(s.insert(BlobKind::Font, "l2.ttf", Arc::clone(&loaded)), BlobId(40));
        assert_eq!(s.len(), 3);
        // The allocation is recognised before hashing: with a stored CRC that the hashing path can never match, the
        // same allocation still finds its blob, while equal bytes in another allocation do not.
        s.blobs.get_mut(&BlobId(40)).unwrap().crc32 ^= 0xFFFF_FFFF;
        assert_eq!(s.insert(BlobKind::Font, "l3.ttf", Arc::clone(&loaded)), BlobId(40));
        assert_eq!(s.len(), 3);
        assert_ne!(s.insert(BlobKind::Font, "l4.ttf", bytes(b"loaded font")), BlobId(40));
        assert_eq!(s.len(), 4);
    }

    /// Review of round 4: with two equal blobs of one kind (possible only through `insert_with_id` on load), `insert`
    /// returns the lowest id as in round 3, also when the bytes are the allocation of the higher one.
    #[test]
    fn the_same_allocation_still_returns_the_lowest_equal_blob() {
        let mut s = BlobStore::default();
        s.insert_with_id(BlobId(1), BlobKind::Font, "a", bytes(b"x"));
        let b = bytes(b"x");
        s.insert_with_id(BlobId(5), BlobKind::Font, "b", Arc::clone(&b));
        assert_eq!(s.insert(BlobKind::Font, "c", Arc::clone(&b)), BlobId(1));
        assert_eq!(s.insert(BlobKind::Font, "d", bytes(b"x")), BlobId(1));
        assert_eq!(s.len(), 2);
        // Another kind with equal bytes at a lower id does not count.
        let mut t = BlobStore::default();
        t.insert_with_id(BlobId(1), BlobKind::Image, "a", bytes(b"x"));
        t.insert_with_id(BlobId(5), BlobKind::Font, "b", Arc::clone(&b));
        assert_eq!(t.insert(BlobKind::Font, "c", Arc::clone(&b)), BlobId(5));
    }

    #[test]
    fn clone_shares_bytes() {
        let mut s = BlobStore::default();
        let a = s.insert(BlobKind::Image, "a.png", bytes(b"payload"));
        let t = s.clone();
        assert!(Arc::ptr_eq(&s.get(a).unwrap().bytes, &t.get(a).unwrap().bytes));
    }
}
