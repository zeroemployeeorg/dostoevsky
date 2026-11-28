//! Where each record of a legacy stream lies in the journal. Mirrors `location_index.go`.
//!
//! Small streams keep their direct locations. Longer streams encode physical gaps and frame lengths in blocks. Each
//! checkpoint bounds a lookup to 32 entries. The index is rebuilt from the unchanged journal; it has no on-disk
//! format.

const LOCATION_STRIDE: usize = 32;
const LOCATION_BLOCK_SIZE: usize = 256;

/// A frame's payload: where it starts in the journal and how long it is. Go's `location` (`journal.go`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Location {
    pub at: i64,
    pub size: i64,
}

#[derive(Debug, Default)]
struct LocationBlock {
    starts: [i64; LOCATION_BLOCK_SIZE / LOCATION_STRIDE],
    positions: [u16; LOCATION_BLOCK_SIZE / LOCATION_STRIDE],
    data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct LocationIndex {
    blocks: Vec<LocationBlock>,
    pub count: i64,
    /// The payload address assuming the next journal frame belongs to this stream.
    next: i64,
}

/// Go's `binary.AppendUvarint`.
fn append_uvarint(b: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        b.push(v as u8 | 0x80);
        v >>= 7;
    }
    b.push(v as u8);
}

/// Go's `binary.Uvarint`, for values this index wrote itself.
fn uvarint(b: &[u8]) -> (u64, usize) {
    let (mut v, mut shift) = (0u64, 0);
    for (i, &c) in b.iter().enumerate() {
        v |= ((c & 0x7f) as u64) << shift;
        if c < 0x80 {
            return (v, i + 1);
        }
        shift += 7;
    }
    (0, 0)
}

impl LocationIndex {
    pub fn append(&mut self, loc: Location) {
        let slot = (self.count % LOCATION_BLOCK_SIZE as i64) as usize;
        if slot == 0 {
            self.blocks.push(LocationBlock {
                data: Vec::with_capacity(LOCATION_BLOCK_SIZE * 2),
                ..Default::default()
            });
        }
        let block = self.blocks.last_mut().expect("a block was just made");
        let mut gap = loc.at.wrapping_sub(self.next);
        if slot.is_multiple_of(LOCATION_STRIDE) {
            let check = slot / LOCATION_STRIDE;
            block.starts[check] = loc.at;
            block.positions[check] = block.data.len() as u16;
            gap = 0;
        }
        let mut code = (loc.size as u64) << 1;
        if gap != 0 {
            code |= 1;
        }
        append_uvarint(&mut block.data, code);
        if gap != 0 {
            append_uvarint(&mut block.data, gap as u64);
        }
        self.next = loc.at.wrapping_add(loc.size).wrapping_add(20);
        self.count += 1;
    }

    pub fn location(&self, i: i64) -> Location {
        let block = &self.blocks[(i / LOCATION_BLOCK_SIZE as i64) as usize];
        let slot = (i % LOCATION_BLOCK_SIZE as i64) as usize;
        let check = slot / LOCATION_STRIDE;
        let mut at = block.starts[check];
        let mut b = &block.data[block.positions[check] as usize..];
        let mut j = check * LOCATION_STRIDE;
        loop {
            let (code, n) = uvarint(b);
            b = &b[n..];
            if code & 1 != 0 {
                let (gap, n) = uvarint(b);
                at = at.wrapping_add(gap as i64);
                b = &b[n..];
            }
            let size = (code >> 1) as i64;
            if j == slot {
                return Location { at, size };
            }
            at = at.wrapping_add(size).wrapping_add(20);
            j += 1;
        }
    }
}
