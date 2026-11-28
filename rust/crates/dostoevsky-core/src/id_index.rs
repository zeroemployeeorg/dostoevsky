//! The in-memory index of producer IDs in a legacy stream. Mirrors the index in `id_index.go`; `storedID`, which
//! reads the journal, is ported with the stream.
//!
//! Fingerprints live only in this index. The journal holds the full ID. On a hit the stored ID is compared before a
//! receipt is returned, including when several IDs share a fingerprint. A miss costs one map lookup.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::OnceLock;

static PRODUCER_ID_SEED: OnceLock<RandomState> = OnceLock::new();

/// The fingerprint of a producer ID. Seeded once per process, as Go's `maphash.MakeSeed`: it is never stored.
pub fn producer_id_hash(id: &str) -> u64 {
    PRODUCER_ID_SEED.get_or_init(RandomState::new).hash_one(id)
}

#[derive(Debug, Default)]
pub struct IdIndex {
    latest: HashMap<u64, i64>,
    /// Filled only if two records have the same fingerprint.
    previous: HashMap<i64, i64>,
}

impl IdIndex {
    pub fn add(&mut self, hash: u64, offset: i64) {
        if let Some(old) = self.latest.insert(hash, offset) {
            self.previous.insert(offset, old);
        }
    }

    /// The offsets recorded under `hash`, newest first: the order in which `storedID` reads them.
    pub fn offsets(&self, hash: u64) -> impl Iterator<Item = i64> + '_ {
        let mut next = self.latest.get(&hash).copied();
        std::iter::from_fn(move || {
            let offset = next?;
            next = self.previous.get(&offset).copied();
            Some(offset)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_fingerprints_chain_newest_first() {
        let mut idx = IdIndex::default();
        assert_eq!(idx.offsets(7).count(), 0);
        idx.add(7, 3);
        idx.add(9, 4);
        idx.add(7, 10);
        idx.add(7, 12);
        assert_eq!(idx.offsets(7).collect::<Vec<_>>(), [12, 10, 3]);
        assert_eq!(idx.offsets(9).collect::<Vec<_>>(), [4]);
        assert_eq!(producer_id_hash("order-1"), producer_id_hash("order-1"));
        assert_ne!(producer_id_hash("order-1"), producer_id_hash("order-2"));
    }
}
