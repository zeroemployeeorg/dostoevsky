//! A small count-min sketch of sealed-index requests. Mirrors the sketch in `admission.go`; the admission and
//! eviction that use it act on the segment layer's cache and are ported with it.
//!
//! Counters age in access time so a past burst cannot protect a cold index indefinitely.

#[derive(Debug, Clone)]
pub struct IndexFrequency {
    rows: [[u8; 256]; 4],
    accesses: u64,
}

impl Default for IndexFrequency {
    fn default() -> Self {
        IndexFrequency {
            rows: [[0; 256]; 4],
            accesses: 0,
        }
    }
}

fn frequency_hash(mut id: u64) -> u64 {
    id ^= id >> 30;
    id = id.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    id ^= id >> 27;
    id = id.wrapping_mul(0x94d0_49bb_1331_11eb);
    id ^ (id >> 31)
}

impl IndexFrequency {
    pub fn observe(&mut self, id: u64) {
        self.accesses += 1;
        if self.accesses.is_multiple_of(512) {
            for row in &mut self.rows {
                for c in row.iter_mut() {
                    *c >>= 1;
                }
            }
        }
        let mut h = frequency_hash(id);
        for row in &mut self.rows {
            let c = &mut row[h as u8 as usize];
            *c = c.saturating_add(1);
            h >>= 8;
        }
    }

    pub fn score(&self, id: u64) -> u8 {
        let mut h = frequency_hash(id);
        let mut v = 255u8;
        for row in &self.rows {
            v = v.min(row[h as u8 as usize]);
            h >>= 8;
        }
        v
    }
}

/// Includes the sketch and allocator rounding.
pub const FREQUENCY_CHARGE: usize = 1152;
