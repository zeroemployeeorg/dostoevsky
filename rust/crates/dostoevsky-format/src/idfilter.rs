//! Immutable Bloom filters of the IDs published in a sealed segment. Mirrors the filter itself in `idfilter.go`.
//!
//! A filter contains partition-qualified hashes. Only a negative answer is definitive. A positive always follows
//! the exact index and full-record comparison.
//!
//! The cache of filters, and reading and writing the `.bloom` side file, belong to the segment layer and are ported
//! with it in `dostoevsky-core`.

use crate::segindex::IdTable;

/// The bytes of one filter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdFilter(pub Vec<u8>);

pub const FILTER_PROBES: usize = 20;
pub const MAX_FILTER_BYTES: usize = 4 << 20;
pub const FILTER_CACHE_BYTES: usize = 64 << 20;

pub fn filter_name(id: u64) -> String {
    format!("seg-{id:016x}.bloom")
}

/// Avalanches the persistent FNV hash before double hashing.
pub fn mix_filter_hash(mut h: u64) -> u64 {
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    h
}

pub fn filter_positions(h: u64, bits: u32) -> (u32, u32) {
    let pos = ((h as u32 as u64 * bits as u64) >> 32) as u32;
    let step = (((h >> 32) * (bits.wrapping_sub(1)) as u64) >> 32) as u32 + 1;
    (pos, step)
}

impl IdFilter {
    /// Takes an already mixed hash so one lookup can visit several segments.
    pub fn contains(&self, h: u64) -> bool {
        let f = &self.0;
        if f.is_empty() {
            return false;
        }
        let bits = (f.len() * 8) as u32;
        let (mut p, step) = filter_positions(h, bits);
        for _ in 0..FILTER_PROBES {
            if f[(p >> 3) as usize] & (1 << (p & 7)) == 0 {
                return false;
            }
            p = p.wrapping_add(step);
            if p >= bits {
                p -= bits;
            }
        }
        true
    }
}

pub fn make_id_filter(t: &IdTable) -> IdFilter {
    let mut f = vec![0u8; MAX_FILTER_BYTES.min(t.len() * 4)];
    if f.is_empty() {
        return IdFilter(f);
    }
    let bits = (f.len() * 8) as u32;
    t.each(|h, _| {
        let (mut p, step) = filter_positions(mix_filter_hash(h), bits);
        for _ in 0..FILTER_PROBES {
            f[(p >> 3) as usize] |= 1 << (p & 7);
            p = p.wrapping_add(step);
            if p >= bits {
                p -= bits;
            }
        }
    });
    IdFilter(f)
}
