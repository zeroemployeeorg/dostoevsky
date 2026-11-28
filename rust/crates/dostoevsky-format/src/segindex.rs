//! The per-segment index of the segmented log. Mirrors `segindex.go`.
//!
//! A sparse map from a record's offset to its position in the segment, a table of the IDs published in the segment
//! (for the idempotency window), and a summary. It is built from the segment's own frames, held in memory for the
//! active segment, and written beside a segment when it is sealed as `seg-<id>.idx`. The file is a HINT: it is
//! verified against the segment before use and rebuilt by scanning that one segment when it is missing or wrong; it
//! is never trusted over the log.
//!
//! `startNextOf`, which reads the log state, is ported with the log state.

use crate::crc32c;
use crate::frame3::Record3;
use std::collections::BTreeMap;
use std::fmt;

/// A partition's records get an index entry at every offset divisible by this, and at the segment's first.
pub const SPARSE_STRIDE: u64 = 16;

// ----- the ID table -----

/// A stable 64-bit hash of a partition id and an idempotency ID (FNV-1a). It is persisted in idx files, so it must
/// never change; a collision is harmless because a hit is always confirmed against the log.
pub fn id_hash(pid: u32, id: &[u8]) -> u64 {
    const PRIME: u64 = 1_099_511_628_211;
    let mut h: u64 = 14_695_981_039_346_656_037;
    for byte in pid.to_le_bytes() {
        h = (h ^ byte as u64).wrapping_mul(PRIME);
    }
    for &c in id {
        h = (h ^ c as u64).wrapping_mul(PRIME);
    }
    h
}

/// Maps hashes to record offsets with open addressing. Slots hold (hash, offset + 1); an offset slot of zero is
/// empty. Equal hashes (distinct IDs that collide, or repeated entries) coexist.
#[derive(Debug, Clone, Default)]
pub struct IdTable {
    pub keys: Vec<u64>,
    vals: Vec<u64>,
    n: usize,
}

impl IdTable {
    fn slot(&self, h: u64) -> usize {
        ((h ^ (h >> 32)) & (self.keys.len() as u64 - 1)) as usize
    }

    pub fn add(&mut self, h: u64, off: u64) {
        if self.keys.is_empty() || (self.n + 1) * 4 > self.keys.len() * 3 {
            self.grow();
        }
        let mut i = self.slot(h);
        while self.vals[i] != 0 {
            i = (i + 1) & (self.keys.len() - 1);
        }
        self.keys[i] = h;
        self.vals[i] = off.wrapping_add(1);
        self.n += 1;
    }

    fn grow(&mut self) {
        let size = if self.keys.is_empty() { 16 } else { self.keys.len() * 2 };
        let old = std::mem::replace(
            self,
            IdTable {
                keys: vec![0; size],
                vals: vec![0; size],
                n: 0,
            },
        );
        for (i, &v) in old.vals.iter().enumerate() {
            if v != 0 {
                self.add(old.keys[i], v.wrapping_sub(1));
            }
        }
    }

    /// Calls `visit` with the offset of every entry whose hash is `h`, until `visit` returns true.
    pub fn lookup(&self, h: u64, mut visit: impl FnMut(u64) -> bool) {
        if self.keys.is_empty() {
            return;
        }
        let mut i = self.slot(h);
        while self.vals[i] != 0 {
            if self.keys[i] == h && visit(self.vals[i].wrapping_sub(1)) {
                return;
            }
            i = (i + 1) & (self.keys.len() - 1);
        }
    }

    pub fn each(&self, mut f: impl FnMut(u64, u64)) {
        for (i, &v) in self.vals.iter().enumerate() {
            if v != 0 {
                f(self.keys[i], v.wrapping_sub(1));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

// ----- the index -----

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdxPart {
    /// The partition's first offset in this segment and how many records it holds here.
    pub first: u64,
    pub count: u64,
    /// Sparse entries: offset ...
    pub offs: Vec<u64>,
    /// ... and the position of that record's frame in the segment file.
    pub poss: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SegIndex {
    /// Identity verified on load or captured on seal.
    pub footer: IdxFooterInfo,
    /// Every partition's next offset when the segment began (its start checkpoint).
    pub start_next: BTreeMap<u32, u64>,
    pub parts: BTreeMap<u32, IdxPart>,
    pub ids: IdTable,
    pub records: u64,
    pub first_time: i64,
    pub last_time: i64,
}

impl SegIndex {
    pub fn new(start_next: BTreeMap<u32, u64>) -> SegIndex {
        SegIndex {
            start_next,
            ..Default::default()
        }
    }

    /// Adds one record frame at `pos`. Positions must fit 32 bits (a segment is far smaller).
    pub fn observe(&mut self, rec: &Record3, pos: i64) {
        let start_next = &self.start_next;
        let p = self.parts.entry(rec.partition).or_insert_with(|| {
            // A partition defined in this segment starts at its first record.
            IdxPart {
                first: start_next.get(&rec.partition).copied().unwrap_or(rec.offset),
                ..Default::default()
            }
        });
        p.count += 1;
        if p.offs.is_empty() || rec.offset.is_multiple_of(SPARSE_STRIDE) {
            p.offs.push(rec.offset);
            p.poss.push(pos as u32);
        }
        if !rec.id.is_empty() {
            self.ids.add(id_hash(rec.partition, rec.id), rec.offset);
        }
        if self.records == 0 {
            self.first_time = rec.time;
        }
        self.last_time = rec.time;
        self.records += 1;
    }

    /// The partition's first offset in this segment: its next offset at the segment's start, or 0 when it did not
    /// exist yet. It never decreases from one segment to the next, which is what lets a lookup binary-search the
    /// segments.
    pub fn first(&self, pid: u32) -> u64 {
        match self.parts.get(&pid) {
            Some(p) => p.first,
            None => self.start_next.get(&pid).copied().unwrap_or(0),
        }
    }

    pub fn count(&self, pid: u32) -> u64 {
        self.parts.get(&pid).map_or(0, |p| p.count)
    }

    /// The position of the sparse entry at or before `off`, from which a walk reaches it.
    pub fn find(&self, pid: u32, off: u64) -> Option<i64> {
        let p = self.parts.get(&pid)?;
        if off < p.first || off >= p.first.wrapping_add(p.count) {
            return None;
        }
        let i = p.offs.partition_point(|&o| o <= off);
        (i > 0).then(|| p.poss[i - 1] as i64)
    }

    /// Whether two indexes serialize identically (used by tests and by rebuild checks).
    pub fn same_as(&self, other: &SegIndex, f: IdxFooterInfo) -> bool {
        encode_seg_index(self, f) == encode_seg_index(other, f)
    }
}

// ----- the idx file -----

const IDX_SIG: &[u8; 8] = b"DOSTIDX1";
const IDX_END: &[u8; 8] = b"IDXEND01";
const IDX_VERSION: u32 = 1;
const IDX_HEAD: usize = 12;
const IDX_SUMMARY: usize = 36;
const IDX_PART_REC: usize = 28;
const IDX_FOOTER: usize = 40;

pub fn idx_name(id: u64) -> String {
    format!("seg-{id:016x}.idx")
}

pub fn idx_tmp_name(id: u64) -> String {
    format!("seg-{id:016x}.idx.tmp")
}

/// What an idx file says about the segment it was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdxFooterInfo {
    pub segment: u64,
    pub size: u64,
    pub last_pos: u64,
    pub last_crc: u32,
}

/// Go's `errIdxInvalid`, with what follows it in the Go message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdxInvalid(pub String);

impl fmt::Display for IdxInvalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "idx file invalid: {}", self.0)
    }
}

impl std::error::Error for IdxInvalid {}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
}

/// Serializes `x` deterministically: partitions ascending, ID entries by (hash, offset).
pub fn encode_seg_index(x: &SegIndex, f: IdxFooterInfo) -> Vec<u8> {
    let mut pids: Vec<u32> = x.start_next.keys().chain(x.parts.keys()).copied().collect();
    pids.sort_unstable();
    pids.dedup();
    let sparse: usize = x.parts.values().map(|p| p.offs.len()).sum();
    let mut ids: Vec<(u64, u64)> = Vec::with_capacity(x.ids.len());
    x.ids.each(|h, off| ids.push((h, off)));
    ids.sort_unstable();
    let mut b = Vec::with_capacity(
        IDX_HEAD + IDX_SUMMARY + IDX_PART_REC * pids.len() + 12 * sparse + 16 * ids.len() + IDX_FOOTER,
    );
    b.extend_from_slice(IDX_SIG);
    b.extend_from_slice(&IDX_VERSION.to_le_bytes());
    b.extend_from_slice(&x.records.to_le_bytes());
    b.extend_from_slice(&(x.first_time as u64).to_le_bytes());
    b.extend_from_slice(&(x.last_time as u64).to_le_bytes());
    b.extend_from_slice(&(pids.len() as u32).to_le_bytes());
    b.extend_from_slice(&(sparse as u32).to_le_bytes());
    b.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    let mut start = 0u32;
    for pid in &pids {
        let (first, count, n) = match x.parts.get(pid) {
            Some(p) => (p.first, p.count, p.offs.len() as u32),
            None => (x.start_next.get(pid).copied().unwrap_or(0), 0, 0),
        };
        b.extend_from_slice(&pid.to_le_bytes());
        b.extend_from_slice(&first.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(&start.to_le_bytes());
        b.extend_from_slice(&n.to_le_bytes());
        start = start.wrapping_add(n);
    }
    for pid in &pids {
        if let Some(p) = x.parts.get(pid) {
            for (off, pos) in p.offs.iter().zip(&p.poss) {
                b.extend_from_slice(&off.to_le_bytes());
                b.extend_from_slice(&pos.to_le_bytes());
            }
        }
    }
    for (h, off) in &ids {
        b.extend_from_slice(&h.to_le_bytes());
        b.extend_from_slice(&off.to_le_bytes());
    }
    b.extend_from_slice(&f.segment.to_le_bytes());
    b.extend_from_slice(&f.size.to_le_bytes());
    b.extend_from_slice(&f.last_pos.to_le_bytes());
    b.extend_from_slice(&f.last_crc.to_le_bytes());
    let check = crc32c::checksum(&b); // over everything before this field
    b.extend_from_slice(&check.to_le_bytes());
    b.extend_from_slice(IDX_END);
    b
}

/// Validates an idx file as a whole (signature, version, check, exact length for the declared counts, ascending
/// partitions, contiguous and in-range sparse entries, sorted ID entries) and returns the index and what it says
/// about its segment. It does not look at the segment.
///
/// As in Go, the footer is returned with an error once it has been read: `Err` carries it.
pub fn decode_seg_index(b: &[u8]) -> Result<(SegIndex, IdxFooterInfo), (IdxFooterInfo, IdxInvalid)> {
    let mut f = IdxFooterInfo::default();
    macro_rules! bad {
        ($($arg:tt)*) => {
            return Err((f, IdxInvalid(format!($($arg)*))))
        };
    }
    if b.len() < IDX_HEAD + IDX_SUMMARY + IDX_FOOTER {
        bad!("{} bytes", b.len());
    }
    if &b[..8] != IDX_SIG || &b[b.len() - 8..] != IDX_END {
        bad!("signature");
    }
    let version = u32_at(b, 8);
    if version != IDX_VERSION {
        bad!("version {version}");
    }
    if u32_at(b, b.len() - 12) != crc32c::checksum(&b[..b.len() - 12]) {
        bad!("check failed");
    }
    let mut x = SegIndex::new(BTreeMap::new());
    x.records = u64_at(b, 12);
    x.first_time = u64_at(b, 20) as i64;
    x.last_time = u64_at(b, 28) as i64;
    let (parts, sparse, ids) = (u32_at(b, 36) as usize, u32_at(b, 40) as usize, u32_at(b, 44) as usize);
    if IDX_HEAD + IDX_SUMMARY + IDX_PART_REC * parts + 12 * sparse + 16 * ids + IDX_FOOTER != b.len() {
        bad!("counts do not match the length");
    }
    let foot = &b[b.len() - IDX_FOOTER..];
    f = IdxFooterInfo {
        segment: u64_at(foot, 0),
        size: u64_at(foot, 8),
        last_pos: u64_at(foot, 16),
        last_crc: u32_at(foot, 24),
    };
    let at = IDX_HEAD + IDX_SUMMARY;
    let sparse_at = at + IDX_PART_REC * parts;
    let mut next = 0u32;
    let mut prev: i64 = -1;
    let mut total_count = 0u64;
    for i in 0..parts {
        let r = &b[at + IDX_PART_REC * i..];
        let (pid, first, count) = (u32_at(r, 0), u64_at(r, 4), u64_at(r, 12));
        let (start, n) = (u32_at(r, 20), u32_at(r, 24));
        if pid as i64 <= prev {
            bad!("partition ids are not ascending");
        }
        prev = pid as i64;
        if start != next || start as usize + n as usize > sparse || (count == 0) != (n == 0) {
            bad!("partition {pid} has inconsistent sparse entries");
        }
        next = next.wrapping_add(n);
        total_count = total_count.wrapping_add(count);
        x.start_next.insert(pid, first);
        if count > 0 {
            let mut p = IdxPart {
                first,
                count,
                offs: Vec::with_capacity(n as usize),
                poss: Vec::with_capacity(n as usize),
            };
            let mut last = 0u64;
            for k in 0..n {
                let e = &b[sparse_at + 12 * (start + k) as usize..];
                let (off, pos) = (u64_at(e, 0), u32_at(e, 8));
                if off < first || off >= first.wrapping_add(count) || (k > 0 && off <= last) {
                    bad!("partition {pid} has a sparse entry out of order or range");
                }
                p.offs.push(off);
                p.poss.push(pos);
                last = off;
            }
            x.parts.insert(pid, p);
        }
    }
    if next as usize != sparse || total_count != x.records {
        bad!("sparse entries or record counts do not add up");
    }
    let id_at = sparse_at + 12 * sparse;
    let (mut ph, mut po) = (0u64, 0u64);
    for i in 0..ids {
        let e = &b[id_at + 16 * i..];
        let (h, off) = (u64_at(e, 0), u64_at(e, 8));
        if i > 0 && (h < ph || (h == ph && off < po)) {
            bad!("ID entries are not sorted");
        }
        (ph, po) = (h, off);
        x.ids.add(h, off);
    }
    Ok((x, f))
}
