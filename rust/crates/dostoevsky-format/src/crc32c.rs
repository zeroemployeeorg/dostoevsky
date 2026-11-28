//! CRC-32C (Castagnoli), as Go's `crc32.MakeTable(crc32.Castagnoli)` (`crcTable` in `journal.go`).
//!
//! Uses the processor's CRC instructions where it has them, else a slicing-by-8 table. Both give the same values;
//! the tests check each against the other and against the Go package.

const POLY: u32 = 0x82F6_3B78;

const fn make_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut i = 0;
    while i < 256 {
        let mut k = 1;
        while k < 8 {
            let p = t[k - 1][i];
            t[k][i] = (p >> 8) ^ t[0][(p & 0xFF) as usize];
            k += 1;
        }
        i += 1;
    }
    t
}

static TABLES: [[u32; 256]; 8] = make_tables();

/// The inner update, on the complemented value, by table.
fn update_table(mut crc: u32, b: &[u8]) -> u32 {
    let (words, rest) = b.as_chunks::<8>();
    for w in words {
        let lo = u32::from_le_bytes([w[0], w[1], w[2], w[3]]) ^ crc;
        let hi = u32::from_le_bytes([w[4], w[5], w[6], w[7]]);
        crc = TABLES[7][(lo & 0xFF) as usize]
            ^ TABLES[6][((lo >> 8) & 0xFF) as usize]
            ^ TABLES[5][((lo >> 16) & 0xFF) as usize]
            ^ TABLES[4][(lo >> 24) as usize]
            ^ TABLES[3][(hi & 0xFF) as usize]
            ^ TABLES[2][((hi >> 8) & 0xFF) as usize]
            ^ TABLES[1][((hi >> 16) & 0xFF) as usize]
            ^ TABLES[0][(hi >> 24) as usize];
    }
    for &x in rest {
        crc = TABLES[0][((crc ^ x as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
fn update_hardware(crc: u32, b: &[u8]) -> u32 {
    use std::arch::x86_64::{_mm_crc32_u64, _mm_crc32_u8};
    let (words, rest) = b.as_chunks::<8>();
    let mut c = crc as u64;
    for w in words {
        c = _mm_crc32_u64(c, u64::from_le_bytes(*w));
    }
    let mut c = c as u32;
    for &x in rest {
        c = _mm_crc32_u8(c, x);
    }
    c
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "crc")]
fn update_hardware(crc: u32, b: &[u8]) -> u32 {
    use std::arch::aarch64::{__crc32cb, __crc32cd};
    let (words, rest) = b.as_chunks::<8>();
    let mut c = crc;
    for w in words {
        c = __crc32cd(c, u64::from_le_bytes(*w));
    }
    for &x in rest {
        c = __crc32cb(c, x);
    }
    c
}

/// Continues a checksum over more bytes. Go's `crc32.Update(crc, crcTable, b)`.
pub fn update(crc: u32, b: &[u8]) -> u32 {
    let c = !crc;
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("sse4.2") {
        // SAFETY: the processor has the instructions this function is compiled for; that was just checked.
        return !unsafe { update_hardware(c, b) };
    }
    #[cfg(target_arch = "aarch64")]
    if std::arch::is_aarch64_feature_detected!("crc") {
        // SAFETY: the processor has the instructions this function is compiled for; that was just checked.
        return !unsafe { update_hardware(c, b) };
    }
    !update_table(c, b)
}

/// Go's `crc32.Checksum(b, crcTable)`.
pub fn checksum(b: &[u8]) -> u32 {
    update(0, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_and_the_processor_agree_and_chain() {
        assert_eq!(checksum(b"123456789"), 0xE306_9283);
        assert_eq!(checksum(b""), 0);
        let data: Vec<u8> = (0..2000u32).map(|i| (i * 7 + 3) as u8).collect();
        for n in [0, 1, 7, 8, 9, 15, 16, 17, 63, 64, 65, 1000, 2000] {
            assert_eq!(!update_table(!0, &data[..n]), checksum(&data[..n]), "length {n}");
            for cut in [0, n.min(1), n / 3, n] {
                assert_eq!(
                    update(checksum(&data[..cut]), &data[cut..n]),
                    checksum(&data[..n]),
                    "length {n} cut {cut}"
                );
            }
        }
    }
}
